//! A small HTTP/1.1 server for the loopback API.
//!
//! It exists because QML in LG's compositor has `XMLHttpRequest` and nothing else that can talk to a process: no WebSocket
//! module (`QtWebSockets` is not in the set's QML tree) and no plugins. So: plain request/response over `127.0.0.1`, one
//! connection per request (`Connection: close`; loopback makes that cheap and it is the simplest thing every client agrees on),
//! one thread per connection with a cap, because a long poll holds its connection for up to 30 s.
//!
//! What it refuses, before any handler runs:
//! - a listener address that is not loopback, and any peer that is not (Principle 0: the LAN door is the owner's to open, on the TV);
//! - a `Host` that is not this listener (DNS rebinding: a page on the TV's browser must not reach the API through a name it controls);
//! - an `Origin` that is not in the allow list (web pages always send one; QML's `XMLHttpRequest` and `curl` never do);
//! - a write (`POST`, `PUT`, `DELETE`) without the `X-Luma-Client` header, which no cross-site form or simple request can set.
//!
//! Limits: 16 KiB of headers, 64 KiB of body, 64 headers, 10 s to receive a request, request bodies with a `Content-Length` only.

use crate::API_VERSION;
use serde_json::{Value, json};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 64 * 1024;
const MAX_HEADERS: usize = 64;
const RECEIVE_DEADLINE: Duration = Duration::from_secs(10);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
/// The header every write must carry. A browser cannot add it to a cross-site request without a preflight that this server refuses.
pub const CLIENT_HEADER: &str = "x-luma-client";
const CONNECTION_STACK: usize = 256 * 1024;

/// A parsed request.
#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    /// The path, percent-decoded, without the query.
    pub path: String,
    pub query: Vec<(String, String)>,
    /// Header names lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// The first header called `name` (lower case).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// The first query parameter called `name`.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// What a handler answers.
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Response {
    /// A JSON body with `status`.
    pub fn json(status: u16, value: &Value) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec()),
        }
    }

    pub fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

/// The error envelope every failure uses: `{"ok":false,"api":1,"error":{"code","message"}}`.
pub fn error_response(status: u16, code: &str, message: &str) -> Response {
    Response::json(
        status,
        &json!({"ok": false, "api": API_VERSION, "error": {"code": code, "message": message}}),
    )
}

/// Answers requests. Called on the connection's own thread, so it may block (a long poll does).
pub trait Handler: Send + Sync + 'static {
    fn handle(&self, request: &Request) -> Response;
}

/// How the server behaves.
#[derive(Clone, Debug)]
pub struct Options {
    /// Connections served at once; more are refused with `503`.
    pub max_connections: usize,
    /// Browser origins that may call (`Origin` header values); empty allows only callers that send none.
    pub allowed_origins: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_connections: 32,
            allowed_origins: Vec::new(),
        }
    }
}

/// A running server.
pub struct Server {
    addr: SocketAddr,
    stopping: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
    accept: Option<JoinHandle<()>>,
}

impl Server {
    /// Binds `addr` (loopback only) and starts accepting.
    pub fn start(addr: SocketAddr, handler: Arc<dyn Handler>, options: Options) -> std::io::Result<Self> {
        if !addr.ip().is_loopback() {
            return Err(std::io::Error::new(
                ErrorKind::InvalidInput,
                "the API listens on loopback only",
            ));
        }
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let stopping = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));
        let accept = {
            let stopping = stopping.clone();
            let active = active.clone();
            let options = Arc::new(options);
            thread::Builder::new()
                .name("http-accept".to_owned())
                .stack_size(CONNECTION_STACK)
                .spawn(move || accept_loop(listener, addr, handler, options, stopping, active))?
        };
        Ok(Self {
            addr,
            stopping,
            active,
            accept: Some(accept),
        })
    }

    /// The address that was bound (the real port when 0 was asked for).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stops accepting and waits up to `grace` for connections in flight (a long poll ends when the hub is closed first).
    pub fn stop(mut self, grace: Duration) {
        self.stopping.store(true, Ordering::SeqCst);
        // `accept` blocks: one connection to ourselves wakes it so it sees the flag.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(500));
        if let Some(handle) = self.accept.take() {
            let _ = handle.join();
        }
        let deadline = Instant::now() + grace;
        while self.active.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
    }
}

struct ActiveGuard(Arc<AtomicUsize>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept_loop(
    listener: TcpListener,
    addr: SocketAddr,
    handler: Arc<dyn Handler>,
    options: Arc<Options>,
    stopping: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
) {
    for incoming in listener.incoming() {
        if stopping.load(Ordering::SeqCst) {
            break;
        }
        let stream = match incoming {
            Ok(stream) => stream,
            Err(e) => {
                crate::log_warn!("http: accept failed: {e}");
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        if !stream.peer_addr().is_ok_and(|p| p.ip().is_loopback()) {
            continue;
        }
        if active.fetch_add(1, Ordering::SeqCst) >= options.max_connections {
            active.fetch_sub(1, Ordering::SeqCst);
            let mut stream = stream;
            let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
            let _ = write_response(
                &mut stream,
                &error_response(503, "busy", "too many connections"),
                None,
            );
            continue;
        }
        let guard = ActiveGuard(active.clone());
        let handler = handler.clone();
        let options = options.clone();
        let spawned = thread::Builder::new()
            .name("http-conn".to_owned())
            .stack_size(CONNECTION_STACK)
            .spawn(move || {
                let _guard = guard;
                serve_connection(stream, addr, handler.as_ref(), &options);
            });
        if let Err(e) = spawned {
            // The guard moved into the closure that was never run; it is dropped with the failed spawn.
            crate::log_warn!("http: could not start a connection thread: {e}");
        }
    }
}

/// Why a request could not be read.
#[derive(Debug, PartialEq, Eq)]
enum ReadError {
    /// The peer closed before sending anything: nothing to answer.
    Closed,
    Timeout,
    Bad(&'static str),
    HeadTooLarge,
    BodyTooLarge,
    NotImplemented(&'static str),
}

fn serve_connection(mut stream: TcpStream, local: SocketAddr, handler: &dyn Handler, options: &Options) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(SEND_TIMEOUT));
    let request = match read_request(&mut stream, Instant::now() + RECEIVE_DEADLINE) {
        Ok(request) => request,
        Err(ReadError::Closed) => return,
        Err(e) => {
            let response = match e {
                ReadError::Timeout => error_response(408, "timeout", "the request did not arrive in time"),
                ReadError::Bad(why) => error_response(400, "bad_request", why),
                ReadError::HeadTooLarge => error_response(431, "too_large", "the request headers are too large"),
                ReadError::BodyTooLarge => error_response(413, "too_large", "the request body is too large"),
                ReadError::NotImplemented(why) => error_response(501, "not_implemented", why),
                ReadError::Closed => return,
            };
            let _ = write_response(&mut stream, &response, None);
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
    };
    let origin = request.header("origin").map(str::to_owned);
    let response = match guard(&request, local, options) {
        Some(refusal) => refusal,
        None if request.method == "OPTIONS" => Response {
            status: 204,
            headers: Vec::new(),
            body: Vec::new(),
        },
        None => catch_unwind(AssertUnwindSafe(|| handler.handle(&request))).unwrap_or_else(|_| {
            crate::log_error!("http: a handler panicked on {} {}", request.method, request.path);
            error_response(500, "internal", "the helper hit a bug handling this request")
        }),
    };
    let _ = write_response(&mut stream, &response, origin.as_deref().filter(|o| options.allowed_origins.iter().any(|a| a == o)));
    let _ = stream.shutdown(Shutdown::Both);
}

/// The transport-level refusals; `None` lets the request through.
fn guard(request: &Request, local: SocketAddr, options: &Options) -> Option<Response> {
    if let Some(host) = request.header("host")
        && !host_is_local(host, local.port())
    {
        return Some(error_response(403, "host_not_allowed", "this API answers only to its own loopback address"));
    }
    if let Some(origin) = request.header("origin")
        && !options.allowed_origins.iter().any(|a| a == origin)
    {
        return Some(error_response(403, "origin_not_allowed", "web pages may not call this API"));
    }
    match request.method.as_str() {
        "GET" | "OPTIONS" => None,
        "POST" | "PUT" | "DELETE" => {
            if request.header(CLIENT_HEADER).is_none_or(str::is_empty) {
                return Some(error_response(
                    403,
                    "client_header_required",
                    "writes must send the X-Luma-Client header",
                ));
            }
            if !request.body.is_empty()
                && !request
                    .header("content-type")
                    .is_some_and(|t| t.to_ascii_lowercase().starts_with("application/json"))
            {
                return Some(error_response(415, "unsupported_media_type", "send the body as application/json"));
            }
            None
        }
        _ => Some(
            error_response(405, "method_not_allowed", "use GET, POST, PUT or DELETE").with_header("Allow", "GET, POST, PUT, DELETE"),
        ),
    }
}

/// Whether a `Host` header names this listener: loopback by number or as `localhost`, with its port.
fn host_is_local(host: &str, port: u16) -> bool {
    let (name, given) = match host.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((name, after)) => (format!("[{name}]"), after.strip_prefix(':')),
            None => return false,
        },
        None => match host.rsplit_once(':') {
            Some((name, p)) => (name.to_owned(), Some(p)),
            None => (host.to_owned(), None),
        },
    };
    let name = name.to_ascii_lowercase();
    let loopback = name == "localhost"
        || name == "[::1]"
        || name
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback());
    loopback && given.is_some_and(|p| p.parse::<u16>().is_ok_and(|p| p == port))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn write_response(stream: &mut TcpStream, response: &Response, cors_origin: Option<&str>) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\n", response.status, reason(response.status));
    if response.status != 204 {
        head.push_str("Content-Type: application/json; charset=utf-8\r\n");
    }
    head.push_str(&format!("Content-Length: {}\r\n", response.body.len()));
    head.push_str("Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n");
    for (name, value) in &response.headers {
        // Header values come from our own code; keep a stray line break from ever splitting a response.
        let value: String = value.chars().filter(|c| !c.is_control()).collect();
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(origin) = cors_origin {
        let origin: String = origin.chars().filter(|c| !c.is_control()).collect();
        head.push_str(&format!(
            "Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Headers: content-type, x-luma-client\r\nAccess-Control-Allow-Methods: GET, POST, PUT, DELETE\r\nVary: Origin\r\n"
        ));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

fn read_request(stream: &mut TcpStream, deadline: Instant) -> Result<Request, ReadError> {
    let mut data: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if data.len() > MAX_HEAD {
            return Err(ReadError::HeadTooLarge);
        }
        let n = read_some(stream, &mut chunk, deadline)?;
        if n == 0 {
            return Err(if data.is_empty() {
                ReadError::Closed
            } else {
                ReadError::Bad("the request ended inside its headers")
            });
        }
        data.extend_from_slice(&chunk[..n]);
    };
    if head_end > MAX_HEAD {
        return Err(ReadError::HeadTooLarge);
    }
    let head = std::str::from_utf8(&data[..head_end]).map_err(|_| ReadError::Bad("the headers are not text"))?;
    let mut request = parse_head(head)?;
    let expects_continue = request.header("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue"));
    let length = body_length(&request)?;
    let mut body = data[head_end + 4..].to_vec();
    if body.len() > length {
        body.truncate(length);
    }
    if body.len() < length && expects_continue {
        let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
    }
    while body.len() < length {
        let n = read_some(stream, &mut chunk, deadline)?;
        if n == 0 {
            return Err(ReadError::Bad("the request ended inside its body"));
        }
        body.extend_from_slice(&chunk[..n.min(length - body.len())]);
    }
    request.body = body;
    Ok(request)
}

fn read_some(stream: &mut TcpStream, buf: &mut [u8], deadline: Instant) -> Result<usize, ReadError> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ReadError::Timeout);
        }
        let _ = stream.set_read_timeout(Some(left));
        match stream.read(buf) {
            Ok(n) => return Ok(n),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => return Err(ReadError::Timeout),
            Err(_) => return Err(ReadError::Closed),
        }
    }
}

fn body_length(request: &Request) -> Result<usize, ReadError> {
    let length = request.header("content-length");
    if request.header("transfer-encoding").is_some() {
        return Err(if length.is_some() {
            ReadError::Bad("both Content-Length and Transfer-Encoding")
        } else {
            ReadError::NotImplemented("chunked request bodies are not accepted; send Content-Length")
        });
    }
    let Some(length) = length else {
        return Ok(0);
    };
    if length.is_empty() || !length.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ReadError::Bad("Content-Length is not a number"));
    }
    match length.parse::<usize>() {
        Ok(n) if n <= MAX_BODY => Ok(n),
        _ => Err(ReadError::BodyTooLarge),
    }
}

fn parse_head(head: &str) -> Result<Request, ReadError> {
    let mut lines = head.split("\r\n");
    let line = lines.next().unwrap_or("");
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(ReadError::Bad("malformed request line"));
    };
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(ReadError::Bad("malformed method"));
    }
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(ReadError::Bad("only HTTP/1.0 and HTTP/1.1 are spoken"));
    }
    if !target.starts_with('/') {
        return Err(ReadError::Bad("the target must be a path"));
    }
    let (raw_path, raw_query) = target.split_once('?').unwrap_or((target, ""));
    let path = percent_decode(raw_path, false).ok_or(ReadError::Bad("bad escape in the path"))?;
    if path.chars().any(char::is_control) {
        return Err(ReadError::Bad("control characters in the path"));
    }
    let mut query = Vec::new();
    for pair in raw_query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let (Some(k), Some(v)) = (percent_decode(k, true), percent_decode(v, true)) else {
            return Err(ReadError::Bad("bad escape in the query"));
        };
        query.push((k, v));
    }
    let mut headers = Vec::new();
    for line in lines {
        if headers.len() >= MAX_HEADERS {
            return Err(ReadError::HeadTooLarge);
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(ReadError::Bad("malformed header"));
        };
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_graphic() && b != b':') {
            return Err(ReadError::Bad("malformed header name"));
        }
        let value = value.trim();
        if value.chars().any(|c| c.is_control() && c != '\t') {
            return Err(ReadError::Bad("control characters in a header"));
        }
        headers.push((name.to_ascii_lowercase(), value.to_owned()));
    }
    Ok(Request {
        method: method.to_owned(),
        path,
        query,
        headers,
        body: Vec::new(),
    })
}

/// Decodes `%XX` (and `+` as a space in a query); `None` for a broken escape or bytes that are not UTF-8.
fn percent_decode(text: &str, plus_is_space: bool) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = text.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_must_name_this_listener() {
        for ok in ["127.0.0.1:18731", "localhost:18731", "LOCALHOST:18731", "[::1]:18731", "127.0.0.2:18731"] {
            assert!(host_is_local(ok, 18731), "{ok}");
        }
        for bad in [
            "127.0.0.1",
            "127.0.0.1:80",
            "evil.example:18731",
            "127.0.0.1.evil.example:18731",
            "192.168.1.5:18731",
            "[::1",
            "",
            "127.0.0.1:",
            "127.0.0.1:abc",
        ] {
            assert!(!host_is_local(bad, 18731), "{bad}");
        }
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%20b", false).as_deref(), Some("a b"));
        assert_eq!(percent_decode("a+b", false).as_deref(), Some("a+b"));
        assert_eq!(percent_decode("a+b", true).as_deref(), Some("a b"));
        assert_eq!(percent_decode("%e2%82%ac", false).as_deref(), Some("\u{20ac}"));
        assert_eq!(percent_decode("%zz", false), None);
        assert_eq!(percent_decode("%2", false), None);
        assert_eq!(percent_decode("%ff", false), None);
    }

    #[test]
    fn a_request_head_is_parsed() {
        let head = "GET /v1/events?after=7&topics=screen%2Creceiver&x HTTP/1.1\r\nHost: 127.0.0.1:1\r\nX-Luma-Client: home\r\nAccept:  */* ";
        let r = parse_head(head).unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/v1/events");
        assert_eq!(r.param("after"), Some("7"));
        assert_eq!(r.param("topics"), Some("screen,receiver"));
        assert_eq!(r.param("x"), Some(""));
        assert_eq!(r.header("x-luma-client"), Some("home"));
        assert_eq!(r.header("accept"), Some("*/*"));
    }

    #[test]
    fn malformed_heads_are_refused() {
        for bad in [
            "GET",
            "GET / HTTP/1.1 extra",
            "get / HTTP/1.1",
            "GET / HTTP/2",
            "GET http://example.com/ HTTP/1.1",
            "GET /%zz HTTP/1.1",
            "GET / HTTP/1.1\r\nno colon here",
            "GET / HTTP/1.1\r\nbad name: x",
            "GET / HTTP/1.1\r\n: empty",
        ] {
            assert!(matches!(parse_head(bad), Err(ReadError::Bad(_))), "{bad}");
        }
    }

    #[test]
    fn body_lengths_are_strict() {
        let with = |value: &str| Request {
            method: "POST".into(),
            path: "/".into(),
            query: vec![],
            headers: vec![("content-length".into(), value.into())],
            body: vec![],
        };
        assert_eq!(body_length(&with("12")), Ok(12));
        assert_eq!(body_length(&with("65536")), Ok(65536));
        assert_eq!(body_length(&with("65537")), Err(ReadError::BodyTooLarge));
        assert!(matches!(body_length(&with("-1")), Err(ReadError::Bad(_))));
        assert!(matches!(body_length(&with("1e3")), Err(ReadError::Bad(_))));
        assert!(matches!(body_length(&with("")), Err(ReadError::Bad(_))));
        assert_eq!(body_length(&with("99999999999999999999999")), Err(ReadError::BodyTooLarge));
    }
}
