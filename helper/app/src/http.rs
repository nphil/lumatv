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
//!
//! `Mode::Lan` is the other door: a listener on the TV's own private IPv4 address for AI assistants (MCP clients), which are
//! not browsers. It serves only peers inside the configured private ranges, refuses every request that carries `Origin` or
//! `Sec-Fetch-Site` (a browser's mark), allows `GET` and `POST` only, and is tighter on resources: 5 s to receive, a small
//! connection cap and at most 4 concurrent connections per peer address, so one host cannot hold every slot with slow requests.

use crate::API_VERSION;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 64 * 1024;
const MAX_HEADERS: usize = 64;
const RECEIVE_DEADLINE: Duration = Duration::from_secs(10);
/// The receive deadline in `Mode::Lan`: a slow peer on the LAN gets less time to hold a slot than a local QML client.
const LAN_RECEIVE_DEADLINE: Duration = Duration::from_secs(5);
/// Concurrent connections one peer address may hold in `Mode::Lan`.
const MAX_PER_PEER: usize = 4;
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
    /// The address the request came from (the peer of the connection; a test handler may use any).
    pub peer: std::net::IpAddr,
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

/// Which door the server is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// The loopback API: loopback address and peers only, browser guards.
    #[default]
    Loopback,
    /// A private-network listener for assistants; only peers inside `peers` (and loopback, when bound to loopback) are served.
    Lan { peers: Vec<Cidr> },
}

/// An IPv4 network, `192.168.0.0/16`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cidr {
    pub net: Ipv4Addr,
    pub prefix: u8,
}

impl Cidr {
    /// Parses `a.b.c.d/n` (`n` 0..=32); host bits of the address are masked off.
    pub fn parse(text: &str) -> Option<Cidr> {
        let (addr, prefix) = text.split_once('/')?;
        if prefix.is_empty() || prefix.len() > 2 || !prefix.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let prefix: u8 = prefix.parse().ok()?;
        if prefix > 32 {
            return None;
        }
        let addr: Ipv4Addr = addr.parse().ok()?;
        Some(Cidr {
            net: Ipv4Addr::from(u32::from(addr) & Self::mask(prefix)),
            prefix,
        })
    }

    fn mask(prefix: u8) -> u32 {
        if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - u32::from(prefix.min(32)))
        }
    }

    /// Whether `ip` is inside the network. IPv4 only; an IPv4-mapped IPv6 address counts as its IPv4 address.
    pub fn contains(&self, ip: IpAddr) -> bool {
        let v4 = match ip.to_canonical() {
            IpAddr::V4(v4) => v4,
            IpAddr::V6(_) => return false,
        };
        if self.prefix > 32 {
            return false;
        }
        let mask = Self::mask(self.prefix);
        u32::from(v4) & mask == u32::from(self.net) & mask
    }
}

/// The private ranges of RFC 1918: where a home LAN's assistants live.
pub fn default_lan_peers() -> Vec<Cidr> {
    ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"]
        .iter()
        .filter_map(|c| Cidr::parse(c))
        .collect()
}

/// Whether a LAN listener may be bound to `ip`: loopback, RFC 1918 private or link-local, never a public or unspecified address.
pub(crate) fn lan_address_ok(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

/// Whether a connection from `peer` is served in LAN mode: inside one of `peers`, or loopback while the listener is on loopback.
pub(crate) fn peer_allowed(peer: IpAddr, bound: IpAddr, peers: &[Cidr]) -> bool {
    peers.iter().any(|c| c.contains(peer))
        || (peer.to_canonical().is_loopback() && bound.is_loopback())
}

/// How the server behaves.
#[derive(Clone, Debug)]
pub struct Options {
    /// Connections served at once; more are refused with `503`.
    pub max_connections: usize,
    /// Browser origins that may call (`Origin` header values); empty allows only callers that send none.
    pub allowed_origins: Vec<String>,
    /// Loopback API (default) or LAN listener.
    pub mode: Mode,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_connections: 32,
            allowed_origins: Vec::new(),
            mode: Mode::Loopback,
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
    /// Binds `addr` and starts accepting. `Mode::Loopback` needs a loopback address; `Mode::Lan` a loopback, private or
    /// link-local IPv4 address (never public, never `0.0.0.0`).
    pub fn start(
        addr: SocketAddr,
        handler: Arc<dyn Handler>,
        options: Options,
    ) -> std::io::Result<Self> {
        match options.mode {
            Mode::Loopback => {
                if !addr.ip().is_loopback() {
                    return Err(std::io::Error::new(
                        ErrorKind::InvalidInput,
                        "the API listens on loopback only",
                    ));
                }
            }
            Mode::Lan { .. } => {
                if !matches!(addr.ip(), IpAddr::V4(v4) if lan_address_ok(v4)) {
                    return Err(std::io::Error::new(
                        ErrorKind::InvalidInput,
                        "the LAN listener needs a loopback, private or link-local IPv4 address",
                    ));
                }
            }
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
        // `accept` blocks: one connection to ourselves wakes it so it sees the flag. If that connection cannot be made the thread
        // is left to the process exit rather than joined, so a stop never hangs.
        let woke = TcpStream::connect_timeout(&self.addr, Duration::from_millis(500)).is_ok();
        if let Some(handle) = self.accept.take()
            && woke
        {
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
    let per_peer: Arc<Mutex<HashMap<IpAddr, usize>>> = Arc::new(Mutex::new(HashMap::new()));
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
        let Ok(peer) = stream.peer_addr().map(|p| p.ip().to_canonical()) else {
            continue;
        };
        let lan = match &options.mode {
            Mode::Loopback => {
                if !peer.is_loopback() {
                    continue;
                }
                false
            }
            Mode::Lan { peers } => {
                if !peer_allowed(peer, addr.ip(), peers) {
                    continue;
                }
                true
            }
        };
        if active.fetch_add(1, Ordering::SeqCst) >= options.max_connections {
            active.fetch_sub(1, Ordering::SeqCst);
            refuse_busy(stream, "too many connections");
            continue;
        }
        let guard = ActiveGuard(active.clone());
        let peer_guard = if lan {
            match PeerGuard::acquire(&per_peer, peer) {
                Some(g) => Some(g),
                None => {
                    // `guard` goes out of scope here and gives the global slot back.
                    refuse_busy(stream, "too many connections from this address");
                    continue;
                }
            }
        } else {
            None
        };
        let handler = handler.clone();
        let options = options.clone();
        let spawned = thread::Builder::new()
            .name("http-conn".to_owned())
            .stack_size(CONNECTION_STACK)
            .spawn(move || {
                let _guard = guard;
                let _peer_guard = peer_guard;
                serve_connection(stream, addr, handler.as_ref(), &options);
            });
        if let Err(e) = spawned {
            // The guards moved into the closure that was never run; they are dropped with the failed spawn.
            crate::log_warn!("http: could not start a connection thread: {e}");
        }
    }
}

fn refuse_busy(mut stream: TcpStream, message: &str) {
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let _ = write_response(&mut stream, &error_response(503, "busy", message), None);
}

/// One of a peer address's concurrent connections; released on drop.
struct PeerGuard {
    counts: Arc<Mutex<HashMap<IpAddr, usize>>>,
    peer: IpAddr,
}

impl PeerGuard {
    fn acquire(counts: &Arc<Mutex<HashMap<IpAddr, usize>>>, peer: IpAddr) -> Option<Self> {
        let mut map = counts.lock();
        let n = map.entry(peer).or_insert(0);
        if *n >= MAX_PER_PEER {
            return None;
        }
        *n += 1;
        Some(Self {
            counts: counts.clone(),
            peer,
        })
    }
}

impl Drop for PeerGuard {
    fn drop(&mut self) {
        let mut map = self.counts.lock();
        if let Some(n) = map.get_mut(&self.peer) {
            *n -= 1;
            if *n == 0 {
                map.remove(&self.peer);
            }
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

fn serve_connection(
    mut stream: TcpStream,
    local: SocketAddr,
    handler: &dyn Handler,
    options: &Options,
) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(SEND_TIMEOUT));
    let peer = stream
        .peer_addr()
        .map_or(std::net::Ipv4Addr::UNSPECIFIED.into(), |a| a.ip());
    let deadline = match options.mode {
        Mode::Loopback => RECEIVE_DEADLINE,
        Mode::Lan { .. } => LAN_RECEIVE_DEADLINE,
    };
    let request = match read_request(&mut stream, Instant::now() + deadline) {
        Ok(mut request) => {
            request.peer = peer;
            request
        }
        Err(ReadError::Closed) => return,
        Err(e) => {
            let response = match e {
                ReadError::Timeout => {
                    error_response(408, "timeout", "the request did not arrive in time")
                }
                ReadError::Bad(why) => error_response(400, "bad_request", why),
                ReadError::HeadTooLarge => {
                    error_response(431, "too_large", "the request headers are too large")
                }
                ReadError::BodyTooLarge => {
                    error_response(413, "too_large", "the request body is too large")
                }
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
        Some((refusal, code)) => {
            note_refusal(code, &request);
            refusal
        }
        None if request.method == "OPTIONS" => Response {
            status: 204,
            headers: Vec::new(),
            body: Vec::new(),
        },
        None => catch_unwind(AssertUnwindSafe(|| handler.handle(&request))).unwrap_or_else(|_| {
            crate::log_error!("http: a handler panicked");
            error_response(
                500,
                "internal",
                "the helper hit a bug handling this request",
            )
        }),
    };
    let _ = write_response(
        &mut stream,
        &response,
        origin.as_deref().filter(|o| {
            matches!(options.mode, Mode::Loopback) && options.allowed_origins.iter().any(|a| a == o)
        }),
    );
    let _ = stream.shutdown(Shutdown::Both);
}

/// The transport-level refusals, with the code that names them; `None` lets the request through.
fn guard(
    request: &Request,
    local: SocketAddr,
    options: &Options,
) -> Option<(Response, &'static str)> {
    if matches!(options.mode, Mode::Lan { .. }) {
        return guard_lan(request);
    }
    let refuse = |status: u16, code: &'static str, message: &str| {
        Some((error_response(status, code, message), code))
    };
    if let Some(host) = request.header("host")
        && !host_is_local(host, local)
    {
        return refuse(
            403,
            "host_not_allowed",
            "this API answers only to its own loopback address",
        );
    }
    match request.header("origin") {
        Some(origin) if options.allowed_origins.iter().any(|a| a == origin) => {}
        Some(_) => return refuse(403, "origin_not_allowed", "web pages may not call this API"),
        // A web page's subresource requests (an image, a frame) carry no Origin but do carry Fetch Metadata saying where they come
        // from. QML's XMLHttpRequest and curl send none; typing the address into a browser is `none`. Anything else is a page
        // reaching for the API, which could at best occupy its connections (a long poll holds one for 30 s).
        None if request
            .header("sec-fetch-site")
            .is_some_and(|v| v != "none") =>
        {
            return refuse(
                403,
                "cross_site_not_allowed",
                "web pages may not call this API",
            );
        }
        None => {}
    }
    match request.method.as_str() {
        "GET" | "OPTIONS" => None,
        "POST" | "PUT" | "DELETE" => {
            if request.header(CLIENT_HEADER).is_none_or(str::is_empty) {
                return refuse(
                    403,
                    "client_header_required",
                    "writes must send the X-Luma-Client header",
                );
            }
            if !request.body.is_empty()
                && !request
                    .header("content-type")
                    .is_some_and(|t| t.to_ascii_lowercase().starts_with("application/json"))
            {
                return refuse(
                    415,
                    "unsupported_media_type",
                    "send the body as application/json",
                );
            }
            None
        }
        _ => Some((
            error_response(405, "method_not_allowed", "use GET, POST, PUT or DELETE")
                .with_header("Allow", "GET, POST, PUT, DELETE"),
            "method_not_allowed",
        )),
    }
}

/// The refusals of the LAN door. Assistants are not browsers: any `Origin` or `Sec-Fetch-Site` is a page reaching for the API.
/// No `Host` check (the peer addresses the TV by IP or by any name) and no `X-Luma-Client` (bearer tokens do that job).
fn guard_lan(request: &Request) -> Option<(Response, &'static str)> {
    let refuse = |status: u16, code: &'static str, message: &str| {
        Some((error_response(status, code, message), code))
    };
    if request.header("origin").is_some() {
        return refuse(403, "origin_not_allowed", "web pages may not call this API");
    }
    if request.header("sec-fetch-site").is_some() {
        return refuse(
            403,
            "cross_site_not_allowed",
            "web pages may not call this API",
        );
    }
    if !matches!(request.method.as_str(), "GET" | "POST") {
        return Some((
            error_response(405, "method_not_allowed", "use GET or POST")
                .with_header("Allow", "GET, POST"),
            "method_not_allowed",
        ));
    }
    if !request.body.is_empty()
        && !request
            .header("content-type")
            .is_some_and(|t| t.to_ascii_lowercase().starts_with("application/json"))
    {
        return refuse(
            415,
            "unsupported_media_type",
            "send the body as application/json",
        );
    }
    None
}

/// Refusal categories only: request headers and paths can contain credentials.
static SEEN_REFUSALS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn note_refusal(code: &str, _request: &Request) {
    let mut seen = SEEN_REFUSALS.lock();
    if seen.len() >= 32 { seen.clear(); }
    if seen.insert(code.to_owned()) { crate::log_warn!("http: refused a request ({code})"); }
}

/// Whether a `Host` header names this listener: `localhost` or the address it is bound to, with its port. (A request for any
/// other loopback alias cannot reach a socket bound to one address, and an alias would only give a page one more connection pool.)
fn host_is_local(host: &str, local: SocketAddr) -> bool {
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
    let bound = match local.ip() {
        std::net::IpAddr::V4(v4) => v4.to_string(),
        std::net::IpAddr::V6(v6) => format!("[{v6}]"),
    };
    (name == "localhost" || name == bound)
        && given.is_some_and(|p| p.parse::<u16>().is_ok_and(|p| p == local.port()))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn write_response(
    stream: &mut TcpStream,
    response: &Response,
    cors_origin: Option<&str>,
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        reason(response.status)
    );
    // A 204 never has a body. A 202 without one (an MCP notification's acknowledgement) has no content type either; a 202 with
    // one (a loopback API answer that says "waiting for the remote") is sent as it is.
    let empty = response.status == 204 || (response.status == 202 && response.body.is_empty());
    // A picture names its own type and may name its own caching; everything else is JSON, never cached.
    let named = |name: &str| {
        response
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case(name))
    };
    if !empty && !named("Content-Type") {
        head.push_str("Content-Type: application/json; charset=utf-8\r\n");
    }
    let body: &[u8] = if empty { &[] } else { &response.body };
    head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    if !named("Cache-Control") {
        head.push_str("Cache-Control: no-store\r\n");
    }
    head.push_str("X-Content-Type-Options: nosniff\r\nConnection: close\r\n");
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
    stream.write_all(body)?;
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
    let head = std::str::from_utf8(&data[..head_end])
        .map_err(|_| ReadError::Bad("the headers are not text"))?;
    let mut request = parse_head(head)?;
    let expects_continue = request
        .header("expect")
        .is_some_and(|v| v.eq_ignore_ascii_case("100-continue"));
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

fn read_some(
    stream: &mut TcpStream,
    buf: &mut [u8],
    deadline: Instant,
) -> Result<usize, ReadError> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ReadError::Timeout);
        }
        let _ = stream.set_read_timeout(Some(left));
        match stream.read(buf) {
            Ok(n) => return Ok(n),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Err(ReadError::Timeout);
            }
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
            ReadError::NotImplemented(
                "chunked request bodies are not accepted; send Content-Length",
            )
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
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
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
        peer: std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
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
                // `from_str_radix` would take a sign, and `%+a` is not an escape.
                if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
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
        let v4: SocketAddr = "127.0.0.1:18731".parse().unwrap();
        for ok in ["127.0.0.1:18731", "localhost:18731", "LOCALHOST:18731"] {
            assert!(host_is_local(ok, v4), "{ok}");
        }
        for bad in [
            "127.0.0.1",
            "127.0.0.1:80",
            "127.0.0.2:18731",
            "[::1]:18731",
            "evil.example:18731",
            "127.0.0.1.evil.example:18731",
            "localhost.:18731",
            "192.168.1.5:18731",
            "0.0.0.0:18731",
            "[::1",
            "",
            "127.0.0.1:",
            "127.0.0.1:abc",
        ] {
            assert!(!host_is_local(bad, v4), "{bad}");
        }
        let v6: SocketAddr = "[::1]:18731".parse().unwrap();
        assert!(host_is_local("[::1]:18731", v6));
        assert!(host_is_local("localhost:18731", v6));
        assert!(!host_is_local("127.0.0.1:18731", v6));
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%20b", false).as_deref(), Some("a b"));
        assert_eq!(percent_decode("a+b", false).as_deref(), Some("a+b"));
        assert_eq!(percent_decode("a+b", true).as_deref(), Some("a b"));
        assert_eq!(
            percent_decode("%e2%82%ac", false).as_deref(),
            Some("\u{20ac}")
        );
        assert_eq!(percent_decode("%zz", false), None);
        assert_eq!(percent_decode("%2", false), None);
        assert_eq!(percent_decode("%ff", false), None);
        assert_eq!(percent_decode("%+a", false), None);
        assert_eq!(percent_decode("%-1", false), None);
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
            peer: std::net::Ipv4Addr::LOCALHOST.into(),
        };
        assert_eq!(body_length(&with("12")), Ok(12));
        assert_eq!(body_length(&with("65536")), Ok(65536));
        assert_eq!(body_length(&with("65537")), Err(ReadError::BodyTooLarge));
        assert!(matches!(body_length(&with("-1")), Err(ReadError::Bad(_))));
        assert!(matches!(body_length(&with("1e3")), Err(ReadError::Bad(_))));
        assert!(matches!(body_length(&with("")), Err(ReadError::Bad(_))));
        assert_eq!(
            body_length(&with("99999999999999999999999")),
            Err(ReadError::BodyTooLarge)
        );
    }

    // ------------------------------------------------------------------------------------------------ LAN mode

    fn lan_request(method: &str, headers: &[(&str, &str)], body: &str) -> Request {
        Request {
            method: method.into(),
            path: "/mcp".into(),
            query: vec![],
            headers: headers
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
            body: body.as_bytes().to_vec(),
            peer: Ipv4Addr::new(192, 168, 1, 20).into(),
        }
    }

    fn refusal(request: &Request) -> Option<(u16, &'static str)> {
        guard_lan(request).map(|(r, code)| (r.status, code))
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn cidr_parsing() {
        let ok = |text: &str| Cidr::parse(text).map(|c| (c.net.to_string(), c.prefix));
        assert_eq!(ok("192.168.0.0/16"), Some(("192.168.0.0".into(), 16)));
        assert_eq!(ok("10.0.0.0/8"), Some(("10.0.0.0".into(), 8)));
        assert_eq!(ok("192.168.1.77/24"), Some(("192.168.1.0".into(), 24)));
        assert_eq!(ok("0.0.0.0/0"), Some(("0.0.0.0".into(), 0)));
        assert_eq!(ok("1.2.3.4/32"), Some(("1.2.3.4".into(), 32)));
        for bad in [
            "",
            "192.168.0.0",
            "192.168.0.0/",
            "192.168.0.0/33",
            "192.168.0.0/-1",
            "192.168.0.0/+8",
            "192.168.0.0/008",
            "192.168.0/24",
            "192.168.0.256/24",
            "::1/128",
            "/8",
            "a.b.c.d/8",
            "192.168.0.0/8/8",
            "192.168.0.0/ 8",
        ] {
            assert_eq!(Cidr::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn cidr_membership() {
        let lan = Cidr::parse("192.168.0.0/16").unwrap();
        for yes in ["192.168.0.0", "192.168.1.5", "192.168.255.255"] {
            assert!(lan.contains(ip(yes)), "{yes}");
        }
        for no in [
            "192.167.255.255",
            "192.169.0.0",
            "10.0.0.1",
            "8.8.8.8",
            "::1",
        ] {
            assert!(!lan.contains(ip(no)), "{no}");
        }
        assert!(lan.contains(ip("::ffff:192.168.1.5")));
        assert!(!lan.contains(ip("::ffff:8.8.8.8")));
        assert!(!lan.contains(ip("fe80::1")));
        let all = Cidr::parse("0.0.0.0/0").unwrap();
        assert!(all.contains(ip("8.8.8.8")));
        let one = Cidr::parse("10.1.2.3/32").unwrap();
        assert!(one.contains(ip("10.1.2.3")) && !one.contains(ip("10.1.2.4")));
        let defaults = default_lan_peers();
        assert_eq!(defaults.len(), 3);
        for yes in ["10.9.9.9", "172.16.0.1", "172.31.255.255", "192.168.1.1"] {
            assert!(defaults.iter().any(|c| c.contains(ip(yes))), "{yes}");
        }
        for no in [
            "172.15.0.1",
            "172.32.0.1",
            "11.0.0.1",
            "169.254.1.1",
            "8.8.8.8",
        ] {
            assert!(!defaults.iter().any(|c| c.contains(ip(no))), "{no}");
        }
    }

    #[test]
    fn the_lan_guard_refuses_what_a_browser_sends() {
        let json = ("content-type", "application/json");
        assert_eq!(refusal(&lan_request("GET", &[], "")), None);
        assert_eq!(refusal(&lan_request("POST", &[json], "{}")), None);
        assert_eq!(refusal(&lan_request("POST", &[], "")), None);
        // Host is not checked and X-Luma-Client is not required.
        assert_eq!(
            refusal(&lan_request("POST", &[("host", "tv.local:9"), json], "{}")),
            None
        );
        assert_eq!(
            refusal(&lan_request("POST", &[("origin", "http://x"), json], "{}")),
            Some((403, "origin_not_allowed"))
        );
        assert_eq!(
            refusal(&lan_request("GET", &[("origin", "")], "")),
            Some((403, "origin_not_allowed"))
        );
        for value in ["cross-site", "same-origin", "none", ""] {
            assert_eq!(
                refusal(&lan_request("GET", &[("sec-fetch-site", value)], "")),
                Some((403, "cross_site_not_allowed")),
                "{value:?}"
            );
        }
        assert_eq!(
            refusal(&lan_request("POST", &[("content-type", "text/plain")], "x")),
            Some((415, "unsupported_media_type"))
        );
        assert_eq!(
            refusal(&lan_request("POST", &[], "{}")),
            Some((415, "unsupported_media_type"))
        );
        assert_eq!(
            refusal(&lan_request(
                "POST",
                &[("content-type", "Application/JSON; charset=utf-8")],
                "{}"
            )),
            None
        );
        for method in ["OPTIONS", "PUT", "DELETE", "PATCH", "HEAD"] {
            let (response, code) = guard_lan(&lan_request(method, &[], "")).unwrap();
            assert_eq!(
                (response.status, code),
                (405, "method_not_allowed"),
                "{method}"
            );
            assert!(
                response
                    .headers
                    .iter()
                    .any(|(n, v)| *n == "Allow" && v == "GET, POST"),
                "{method}"
            );
        }
    }

    #[test]
    fn lan_addresses_are_private_only() {
        for ok in [
            "127.0.0.1",
            "10.0.0.5",
            "172.16.3.4",
            "172.31.255.255",
            "192.168.1.71",
            "169.254.10.10",
        ] {
            assert!(lan_address_ok(ok.parse().unwrap()), "{ok}");
        }
        for bad in [
            "8.8.8.8",
            "0.0.0.0",
            "172.32.0.1",
            "100.64.0.1",
            "255.255.255.255",
            "224.0.0.1",
        ] {
            assert!(!lan_address_ok(bad.parse().unwrap()), "{bad}");
        }
        let lan = || Options {
            mode: Mode::Lan {
                peers: default_lan_peers(),
            },
            ..Options::default()
        };
        for bad in ["8.8.8.8:0", "0.0.0.0:0", "[::1]:0", "[::]:0"] {
            let err = Server::start(bad.parse().unwrap(), Arc::new(Fixed(200)), lan())
                .err()
                .unwrap_or_else(|| panic!("{bad} was bound"));
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "{bad}");
        }
        // Loopback mode still refuses everything but loopback.
        let err = Server::start(
            "192.168.1.71:0".parse().unwrap(),
            Arc::new(Fixed(200)),
            Options::default(),
        )
        .err()
        .unwrap();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        Server::start("127.0.0.1:0".parse().unwrap(), Arc::new(Fixed(200)), lan())
            .unwrap()
            .stop(Duration::from_millis(100));
    }

    #[test]
    fn peers_are_filtered_by_range() {
        let lan = default_lan_peers();
        let tv = ip("192.168.1.71");
        assert!(peer_allowed(ip("192.168.1.20"), tv, &lan));
        assert!(peer_allowed(ip("10.0.0.2"), tv, &lan));
        assert!(!peer_allowed(ip("8.8.8.8"), tv, &lan));
        assert!(!peer_allowed(ip("192.168.1.20"), tv, &[]));
        // Loopback peers are served only while the listener itself is on loopback.
        assert!(!peer_allowed(ip("127.0.0.1"), tv, &lan));
        assert!(peer_allowed(ip("127.0.0.1"), ip("127.0.0.1"), &[]));
        assert!(peer_allowed(ip("::ffff:127.0.0.1"), ip("127.0.0.1"), &[]));
        assert!(!peer_allowed(ip("192.168.1.20"), ip("127.0.0.1"), &[]));
        assert!(peer_allowed(ip("::ffff:192.168.1.20"), tv, &lan));
    }

    struct Fixed(u16);

    impl Handler for Fixed {
        fn handle(&self, _request: &Request) -> Response {
            Response {
                status: self.0,
                headers: Vec::new(),
                body: b"{\"ignored\":true}".to_vec(),
            }
        }
    }

    struct Bodyless(u16);

    impl Handler for Bodyless {
        fn handle(&self, _request: &Request) -> Response {
            Response {
                status: self.0,
                headers: Vec::new(),
                body: Vec::new(),
            }
        }
    }

    fn lan_server_with(handler: Arc<dyn Handler>) -> Server {
        Server::start(
            "127.0.0.1:0".parse().unwrap(),
            handler,
            Options {
                mode: Mode::Lan { peers: Vec::new() },
                ..Options::default()
            },
        )
        .unwrap()
    }

    fn lan_server(status: u16) -> Server {
        lan_server_with(Arc::new(Fixed(status)))
    }

    fn read_all(mut stream: TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut data = Vec::new();
        let _ = stream.read_to_end(&mut data);
        String::from_utf8_lossy(&data).into_owned()
    }

    fn exchange(addr: SocketAddr, request: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        read_all(stream)
    }

    #[test]
    fn a_202_without_a_body_has_no_content_type() {
        let server = lan_server_with(Arc::new(Bodyless(202)));
        let reply = exchange(
            server.local_addr(),
            "POST /x HTTP/1.1\r\nContent-Length: 2\r\nContent-Type: application/json\r\n\r\n{}",
        );
        server.stop(Duration::from_millis(200));
        assert!(reply.starts_with("HTTP/1.1 202 Accepted\r\n"), "{reply}");
        assert!(reply.contains("Content-Length: 0\r\n"), "{reply}");
        assert!(
            !reply.to_ascii_lowercase().contains("\r\ncontent-type:"),
            "{reply}"
        );
        assert!(reply.ends_with("\r\n\r\n"), "{reply}");
    }

    #[test]
    fn a_202_with_a_body_and_a_204_are_each_sent_as_they_are() {
        let server = lan_server(202);
        let reply = exchange(
            server.local_addr(),
            "POST /x HTTP/1.1\r\nContent-Length: 2\r\nContent-Type: application/json\r\n\r\n{}",
        );
        server.stop(Duration::from_millis(200));
        assert!(reply.starts_with("HTTP/1.1 202 Accepted\r\n"), "{reply}");
        assert!(reply.contains("Content-Type: application/json"), "{reply}");
        assert!(reply.ends_with("{\"ignored\":true}"), "{reply}");
        let server = lan_server(204);
        let reply = exchange(
            server.local_addr(),
            "POST /x HTTP/1.1\r\nContent-Length: 2\r\nContent-Type: application/json\r\n\r\n{}",
        );
        server.stop(Duration::from_millis(200));
        assert!(reply.contains("Content-Length: 0\r\n"), "{reply}");
        assert!(
            !reply.to_ascii_lowercase().contains("\r\ncontent-type:"),
            "{reply}"
        );
        assert!(reply.ends_with("\r\n\r\n"), "{reply}");
    }

    #[test]
    fn the_lan_door_refuses_pages_and_odd_methods_over_tcp() {
        let server = lan_server(200);
        let addr = server.local_addr();
        let page = exchange(
            addr,
            "POST /x HTTP/1.1\r\nOrigin: http://evil.example\r\nContent-Length: 0\r\n\r\n",
        );
        assert!(page.starts_with("HTTP/1.1 403 "), "{page}");
        assert!(page.contains("origin_not_allowed"), "{page}");
        assert!(
            !page.to_ascii_lowercase().contains("access-control"),
            "{page}"
        );
        let options = exchange(addr, "OPTIONS /x HTTP/1.1\r\n\r\n");
        server.stop(Duration::from_millis(200));
        assert!(options.starts_with("HTTP/1.1 405 "), "{options}");
        assert!(options.contains("Allow: GET, POST\r\n"), "{options}");
    }

    #[test]
    fn one_peer_cannot_hold_every_connection() {
        let server = lan_server(200);
        let addr = server.local_addr();
        // Idle connections: each holds a slot until its 5 s deadline. The accept loop is sequential, so by the time the fifth
        // is accepted the first four are counted.
        let held: Vec<TcpStream> = (0..MAX_PER_PEER)
            .map(|_| TcpStream::connect(addr).unwrap())
            .collect();
        let fifth = TcpStream::connect(addr).unwrap();
        let reply = read_all(fifth);
        assert!(reply.starts_with("HTTP/1.1 503 "), "{reply}");
        assert!(reply.contains("\"busy\""), "{reply}");
        drop(held);
        server.stop(Duration::from_millis(200));
    }
}
