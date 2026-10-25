//! The blocking transport: one plain-HTTP GET of the status XML with a hard deadline.

use crate::status::{ParseError, Status, parse_status_lite};
use std::fmt;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The Lite status the receiver serves on its web port.
pub const STATUS_LITE_PATH: &str = "/goform/formMainZone_MainZoneXmlStatusLite.xml";

/// The path commands are sent to, with the command as the query (`?MV35`). The set's receiver answers `200` with an empty body.
const COMMAND_PATH: &str = "/goform/formiPhoneAppDirect.xml";

/// The longest reply that is read: the status is a few hundred bytes, so anything near this is not the status.
const MAX_REPLY: usize = 64 * 1024;
/// The most the status line and headers may take.
const MAX_HEAD: usize = 8 * 1024;

/// Why a status could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientError {
    /// No connection: refused, no route, or the name did not resolve. The text is short and safe to log.
    Unreachable(String),
    /// The connection or the reply did not finish in time.
    Timeout,
    /// The connection broke while it was in use.
    Io(String),
    /// The receiver answered with this HTTP status instead of the XML (its web server says `403` for pages it will not serve).
    Http(u16),
    /// What came back is not an HTTP reply that can be trusted to be whole.
    BadResponse(&'static str),
    /// The reply is longer than a status can be.
    TooLarge,
    /// The reply is HTTP but not the status XML.
    Parse(ParseError),
    /// The command is not 2 to 16 characters of `A-Z`, `0-9` and `?`; nothing was sent.
    InvalidCommand,
}

impl ClientError {
    /// A short stable word for the API and the log.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unreachable(_) => "unreachable",
            Self::Timeout => "timeout",
            Self::Io(_) => "io",
            Self::Http(_) => "http",
            Self::BadResponse(_) => "bad_response",
            Self::TooLarge => "too_large",
            Self::Parse(_) => "parse",
            Self::InvalidCommand => "invalid_command",
        }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(why) => write!(f, "the receiver is not reachable ({why})"),
            Self::Timeout => f.write_str("the receiver did not answer in time"),
            Self::Io(why) => write!(f, "the connection to the receiver broke ({why})"),
            Self::Http(code) => write!(f, "the receiver answered HTTP {code}"),
            Self::BadResponse(why) => write!(f, "the receiver's reply was not usable ({why})"),
            Self::TooLarge => f.write_str("the receiver's reply was too large"),
            Self::Parse(e) => write!(f, "{e}"),
            Self::InvalidCommand => f.write_str("the command is not a valid receiver command"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Reads the status of one receiver. Cheap to clone and to rebuild: it holds an address and two timeouts, no connection.
#[derive(Clone, Debug)]
pub struct Client {
    host: String,
    port: u16,
    connect_timeout: Duration,
    total_timeout: Duration,
    /// Where a host *name* resolved to: it is looked up once, not on every read (the resolver has no deadline of its own), and
    /// again after a connection fails. An address given as numbers is never looked up.
    resolved: Arc<Mutex<Option<SocketAddr>>>,
}

impl Client {
    /// A client for `host` (an address or a name) and `port`, with a 1.5 s connect timeout and 3 s for the whole request.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            connect_timeout: Duration::from_millis(1500),
            total_timeout: Duration::from_secs(3),
            resolved: Arc::new(Mutex::new(None)),
        }
    }

    /// The address to connect to: numbers as given, a name as resolved before, else resolved now (IPv4 first).
    fn address(&self) -> Result<SocketAddr, ClientError> {
        if let Ok(ip) = self.host.parse::<IpAddr>() {
            return Ok(SocketAddr::new(ip, self.port));
        }
        if let Some(addr) = self.resolved.lock().ok().and_then(|slot| *slot) {
            return Ok(addr);
        }
        let addr = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|e| ClientError::Unreachable(short(&e)))?
            .min_by_key(|a| !a.is_ipv4())
            .ok_or_else(|| ClientError::Unreachable("no address".to_owned()))?;
        if let Ok(mut slot) = self.resolved.lock() {
            *slot = Some(addr);
        }
        Ok(addr)
    }

    /// Sets the connect timeout and the deadline of the whole request (connect included).
    pub fn with_timeouts(mut self, connect: Duration, total: Duration) -> Self {
        self.connect_timeout = connect;
        self.total_timeout = total;
        self
    }

    /// The receiver's host as given.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The receiver's port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Reads the Lite status once.
    pub fn status(&self) -> Result<Status, ClientError> {
        let body = self.get(STATUS_LITE_PATH)?;
        let text = String::from_utf8_lossy(&body);
        parse_status_lite(&text).map_err(ClientError::Parse)
    }

    /// Sends one command (`MV35`, `MUON`, ...) and succeeds on any `2xx` reply, with or without a body: the receiver's reply
    /// to a command is empty and its effect shows in the status. A command that is not 2 to 16 characters of `A-Z`, `0-9`
    /// and `?` is refused without connecting, so nothing a caller passes can add to the request line.
    pub fn send_command(&self, command: &str) -> Result<(), ClientError> {
        if !(2..=16).contains(&command.len())
            || !command
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'?')
        {
            return Err(ClientError::InvalidCommand);
        }
        self.get(&format!("{COMMAND_PATH}?{command}")).map(|_| ())
    }

    /// The body of `GET path`, at most [`MAX_REPLY`] bytes, within the total deadline.
    fn get(&self, path: &str) -> Result<Vec<u8>, ClientError> {
        let deadline = Instant::now() + self.total_timeout;
        let addr = self.address()?;
        let connect = self
            .connect_timeout
            .min(deadline.saturating_duration_since(Instant::now()))
            .max(Duration::from_millis(1));
        let mut stream = TcpStream::connect_timeout(&addr, connect).map_err(|e| {
            // The name may now mean another address: look it up again next time.
            if let Ok(mut slot) = self.resolved.lock() {
                *slot = None;
            }
            match e.kind() {
                ErrorKind::TimedOut | ErrorKind::WouldBlock => ClientError::Timeout,
                _ => ClientError::Unreachable(short(&e)),
            }
        })?;
        let _ = stream.set_nodelay(true);
        // HTTP/1.0 with a close: the receiver's server is HTTP/1.0 and ends the body by closing, so a 1.0 request can never be
        // answered with chunked framing.
        let request = format!(
            "GET {path} HTTP/1.0\r\nHost: {}:{}\r\nUser-Agent: LumaTV\r\nAccept: text/xml\r\nConnection: close\r\n\r\n",
            self.host, self.port
        );
        stream
            .set_write_timeout(Some(
                deadline
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1)),
            ))
            .map_err(|e| ClientError::Io(short(&e)))?;
        stream
            .write_all(request.as_bytes())
            .map_err(|e| io_error(&e))?;

        let mut raw = Vec::new();
        let mut chunk = [0u8; 2048];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(ClientError::Timeout);
            }
            stream
                .set_read_timeout(Some(left))
                .map_err(|e| ClientError::Io(short(&e)))?;
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    raw.extend_from_slice(&chunk[..n]);
                    if raw.len() > MAX_REPLY {
                        return Err(ClientError::TooLarge);
                    }
                    if reply_complete(&raw) {
                        break;
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(io_error(&e)),
            }
        }
        parse_response(&raw).map(<[u8]>::to_vec)
    }
}

/// The command that sets the master volume to `level` on the receiver's 0 to 98 scale, or `None` for a level that is not
/// finite or outside `0.0..=98.0`. The receiver steps in halves: the level is rounded to the nearest 0.5 with ties going up
/// (`x.25` to `x.5`, `x.75` to the next whole). A whole level is two digits (`MV35`), a half is three (`MV355`, `MV055`).
pub fn volume_command(level: f64) -> Option<String> {
    if !level.is_finite() || !(0.0..=98.0).contains(&level) {
        return None;
    }
    let halves = (level * 2.0 + 0.5).floor() as u32;
    Some(if halves % 2 == 0 {
        format!("MV{:02}", halves / 2)
    } else {
        format!("MV{:02}5", halves / 2)
    })
}

/// The command that mutes or unmutes the master zone.
pub fn mute_command(muted: bool) -> &'static str {
    if muted { "MUON" } else { "MUOFF" }
}

fn io_error(e: &std::io::Error) -> ClientError {
    match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => ClientError::Timeout,
        _ => ClientError::Io(short(e)),
    }
}

/// An I/O error as a short lowercase phrase (its kind), never the platform's long text.
fn short(e: &std::io::Error) -> String {
    match e.kind() {
        ErrorKind::ConnectionRefused => "connection refused".to_owned(),
        ErrorKind::ConnectionReset => "connection reset".to_owned(),
        ErrorKind::ConnectionAborted => "connection aborted".to_owned(),
        ErrorKind::NotConnected => "not connected".to_owned(),
        ErrorKind::BrokenPipe => "broken pipe".to_owned(),
        ErrorKind::AddrNotAvailable => "address not available".to_owned(),
        ErrorKind::HostUnreachable => "no route to host".to_owned(),
        ErrorKind::NetworkUnreachable => "network unreachable".to_owned(),
        ErrorKind::PermissionDenied => "permission denied".to_owned(),
        _ => format!("{:?}", e.kind()).to_lowercase(),
    }
}

/// The position of the blank line that ends the headers, and how long the terminator is.
fn head_end(data: &[u8]) -> Option<(usize, usize)> {
    if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some((at, 4));
    }
    data.windows(2).position(|w| w == b"\n\n").map(|at| (at, 2))
}

/// The declared body length, if the headers say one.
fn content_length(head: &str) -> Option<usize> {
    head.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    })
}

/// Whether the reply is whole already, so the read need not wait for the receiver to close: only when it says how long it is.
fn reply_complete(raw: &[u8]) -> bool {
    let Some((end, sep)) = head_end(raw) else {
        return false;
    };
    let Ok(head) = std::str::from_utf8(&raw[..end]) else {
        return false;
    };
    content_length(head).is_some_and(|n| raw.len() >= end + sep + n)
}

/// The body of an HTTP reply that has a `2xx` status and, when it declares a length, all of it.
pub(crate) fn parse_response(raw: &[u8]) -> Result<&[u8], ClientError> {
    let Some((end, sep)) = head_end(raw) else {
        return Err(ClientError::BadResponse(if raw.len() > MAX_HEAD {
            "headers too long"
        } else {
            "no end of headers"
        }));
    };
    if end > MAX_HEAD {
        return Err(ClientError::BadResponse("headers too long"));
    }
    let head = std::str::from_utf8(&raw[..end])
        .map_err(|_| ClientError::BadResponse("headers not text"))?;
    let mut parts = head.lines().next().unwrap_or("").split(' ');
    let version = parts.next().unwrap_or("");
    let code = parts
        .next()
        .filter(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|c| c.parse::<u16>().ok());
    let (true, Some(code)) = (version.starts_with("HTTP/1."), code) else {
        return Err(ClientError::BadResponse("not HTTP"));
    };
    if !(200..300).contains(&code) {
        return Err(ClientError::Http(code));
    }
    let body = &raw[end + sep..];
    match content_length(head) {
        Some(n) if body.len() < n => Err(ClientError::BadResponse("body shorter than declared")),
        Some(n) => Ok(&body[..n]),
        None => Ok(body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_without_a_length_ends_where_the_data_ends() {
        let raw = b"HTTP/1.0 200 OK\r\nServer: mongoose2.5\r\nContent-Type: text/xml; charset=utf-8\r\n\r\n<item></item>\n";
        assert_eq!(parse_response(raw).unwrap(), b"<item></item>\n");
        assert!(!reply_complete(raw));
    }

    #[test]
    fn a_declared_length_cuts_the_body_and_ends_the_read() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhelloEXTRA";
        assert!(reply_complete(raw));
        assert_eq!(parse_response(raw).unwrap(), b"hello");
        let short = b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\nhello";
        assert!(!reply_complete(short));
        assert_eq!(
            parse_response(short),
            Err(ClientError::BadResponse("body shorter than declared"))
        );
    }

    #[test]
    fn other_statuses_are_errors_with_their_code() {
        let raw = b"HTTP/1.0 403 Forbidden\r\n\r\nError 403: Forbidden\r\nAccess Forbidden\r\n";
        assert_eq!(parse_response(raw), Err(ClientError::Http(403)));
    }

    #[test]
    fn what_is_not_http_is_refused() {
        assert_eq!(
            parse_response(b"SSH-2.0-banner\r\n\r\n"),
            Err(ClientError::BadResponse("not HTTP"))
        );
        assert_eq!(
            parse_response(b"HTTP/1.0 2x0 OK\r\n\r\n"),
            Err(ClientError::BadResponse("not HTTP"))
        );
        assert_eq!(
            parse_response(b"HTTP/1.0 200 OK\r\nServer: x"),
            Err(ClientError::BadResponse("no end of headers"))
        );
    }

    #[test]
    fn bare_line_feeds_end_the_headers_too() {
        let raw = b"HTTP/1.0 200 OK\nServer: x\n\nbody";
        assert_eq!(parse_response(raw).unwrap(), b"body");
    }

    #[test]
    fn error_codes_are_stable_words() {
        assert_eq!(ClientError::Timeout.code(), "timeout");
        assert_eq!(ClientError::Http(403).code(), "http");
        assert_eq!(ClientError::Parse(ParseError::NotStatus).code(), "parse");
    }
}
