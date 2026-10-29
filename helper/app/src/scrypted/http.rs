//! A small HTTP/1.1 client for the bridge: requests on a few kept-alive connections, and dedicated connections for the two streams (the push
//! channel and a camera's stills) that end by closing.
//!
//! The bridge keeps a connection only when asked (`Connection: keep-alive`, then `Keep-Alive: timeout=30`); a stream is never reused. A
//! request that finds its pooled connection dead before the first byte of the answer is sent again on a fresh one (the server closed it while
//! it idled), once; any other failure is the caller's. Bodies are framed by `Content-Length`, by chunks (Node's way of streaming) or by the
//! connection's end, and are read through [`BodyReader`] so that a stream's parser sees only the body.
//!
//! Nothing here logs, and no error carries the token, the address or a stream address.

use super::store::Token;
use super::tls::{ConnectError, Dialer, Stream};
use parking_lot::{Condvar, Mutex};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// The longest a header block may be.
const MAX_HEAD: usize = 16 * 1024;
/// How long to reach the bridge and finish the handshake.
const CONNECT_WITHIN: Duration = Duration::from_secs(4);
/// How long one read or write may wait, and (for a plain request) all of the call.
const IO_WITHIN: Duration = Duration::from_secs(5);
/// How long the head of a stream may take.
const HEAD_WITHIN: Duration = Duration::from_secs(8);
const CALL_WITHIN: Duration = Duration::from_secs(8);
/// A kept connection is dropped after this long idle: the bridge closes at 30 s, and one used at 25 s may be closed under the request.
const KEEP_IDLE: Duration = Duration::from_secs(20);
/// Connections kept, and calls in flight at once (the bridge allows four calls to the recorder at a time).
const KEEP_MAX: usize = 4;
const IN_FLIGHT: usize = 4;
/// The stack a thread that makes a TLS handshake is given.
pub const TLS_STACK: usize = 256 * 1024;

/// What went wrong, in the few ways the module above acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    Connect(ConnectError),
    /// A read or a write timed out.
    Timeout,
    /// The connection failed or ended; nothing of an answer was read when `before_answer`.
    Io {
        before_answer: bool,
        why: String,
    },
    /// What came back is not HTTP, or is too big.
    Protocol(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(e) => write!(f, "{e}"),
            Self::Timeout => f.write_str("the bridge was too slow"),
            Self::Io { why, .. } => write!(f, "the connection to the bridge failed ({why})"),
            Self::Protocol(why) => write!(f, "the bridge's answer was not understood ({why})"),
        }
    }
}

fn from_io(e: &io::Error, before_answer: bool) -> HttpError {
    match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => HttpError::Timeout,
        _ => HttpError::Io {
            before_answer,
            why: e.kind().to_string(),
        },
    }
}

/// One request.
#[derive(Clone, Debug)]
pub struct Request {
    pub method: &'static str,
    /// Path and query, already encoded.
    pub path: String,
    pub json: Option<String>,
    /// What the caller accepts (`application/json`, `image/jpeg`, `text/event-stream`).
    pub accept: &'static str,
    /// The push channel's resume point (`Last-Event-ID`).
    pub last_event_id: Option<String>,
}

impl Request {
    pub fn get(path: impl Into<String>) -> Self {
        Self {
            method: "GET",
            path: path.into(),
            json: None,
            accept: "application/json",
            last_event_id: None,
        }
    }

    pub fn post(path: impl Into<String>, json: String) -> Self {
        Self {
            method: "POST",
            path: path.into(),
            json: Some(json),
            accept: "application/json",
            last_event_id: None,
        }
    }

    pub fn accepting(mut self, accept: &'static str) -> Self {
        self.accept = accept;
        self
    }

    fn write(
        &self,
        out: &mut impl Write,
        host: &str,
        token: &Token,
        keep_alive: bool,
    ) -> io::Result<()> {
        let mut head = format!(
            "{} {} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {}\r\nAccept: {}\r\nUser-Agent: LumaTV\r\nConnection: {}\r\n",
            self.method,
            self.path,
            token.expose(),
            self.accept,
            if keep_alive { "keep-alive" } else { "close" },
        );
        if let Some(id) = &self.last_event_id {
            head.push_str(&format!("Last-Event-ID: {id}\r\n"));
        }
        match &self.json {
            Some(body) => head.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )),
            None => head.push_str("\r\n"),
        }
        let mut bytes = head.into_bytes();
        if let Some(body) = &self.json {
            bytes.extend_from_slice(body.as_bytes());
        }
        out.write_all(&bytes)?;
        out.flush()
    }
}

/// The status and headers of an answer.
#[derive(Clone, Debug, Default)]
pub struct Head {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn keeps_alive(&self) -> bool {
        self.header("connection")
            .is_some_and(|v| v.to_ascii_lowercase().contains("keep-alive"))
    }

    fn framing(&self) -> Framing {
        if self
            .header("transfer-encoding")
            .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
        {
            return Framing::Chunked;
        }
        match self
            .header("content-length")
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            Some(n) => Framing::Length(n),
            None => Framing::Close,
        }
    }

    /// Reads the status line and the headers from `r`.
    pub fn read(r: &mut impl BufRead) -> Result<Self, HttpError> {
        let mut total = 0usize;
        let mut line = String::new();
        let mut first = true;
        let mut head = Head::default();
        loop {
            line.clear();
            let n = r.read_line(&mut line).map_err(|e| from_io(&e, first))?;
            if n == 0 {
                return Err(HttpError::Io {
                    before_answer: first,
                    why: "closed".into(),
                });
            }
            total += n;
            if total > MAX_HEAD {
                return Err(HttpError::Protocol("headers too long".into()));
            }
            let text = line.trim_end_matches(['\r', '\n']);
            if first {
                let mut parts = text.splitn(3, ' ');
                let (version, code) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                head.status = match (version.starts_with("HTTP/1."), code.parse::<u16>()) {
                    (true, Ok(code)) => code,
                    _ => return Err(HttpError::Protocol("not an HTTP status line".into())),
                };
                first = false;
            } else if text.is_empty() {
                return Ok(head);
            } else if let Some((name, value)) = text.split_once(':') {
                head.headers
                    .push((name.trim().to_owned(), value.trim().to_owned()));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Framing {
    Length(usize),
    Chunked,
    Close,
}

/// How far a body has been read.
#[derive(Debug)]
pub struct BodyState {
    framing: Framing,
    /// Bytes left of the current length or chunk.
    left: usize,
    /// In a chunked body: a chunk's size line is due.
    want_size: bool,
    done: bool,
}

impl BodyState {
    pub fn of(head: &Head) -> Self {
        let framing = head.framing();
        Self {
            framing,
            left: if let Framing::Length(n) = framing {
                n
            } else {
                0
            },
            want_size: true,
            done: matches!(framing, Framing::Length(0)),
        }
    }

    pub fn finished(&self) -> bool {
        self.done
    }

    /// Whether the connection can carry another request once the body is read (its end is known, not the connection's).
    fn reusable(&self) -> bool {
        self.done && self.framing != Framing::Close
    }

    /// Reads decoded body bytes into `out`; 0 is the end of the body.
    pub fn read(&mut self, r: &mut impl BufRead, out: &mut [u8]) -> io::Result<usize> {
        if self.done || out.is_empty() {
            return Ok(0);
        }
        match self.framing {
            Framing::Close => {
                let n = r.read(out)?;
                if n == 0 {
                    self.done = true;
                }
                Ok(n)
            }
            Framing::Length(_) => {
                let want = out.len().min(self.left);
                let n = r.read(&mut out[..want])?;
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                self.left -= n;
                if self.left == 0 {
                    self.done = true;
                }
                Ok(n)
            }
            Framing::Chunked => {
                if self.left == 0 {
                    if !self.want_size {
                        // the CRLF that ends a chunk
                        let mut crlf = [0u8; 2];
                        r.read_exact(&mut crlf)?;
                        if &crlf != b"\r\n" {
                            return Err(io::Error::new(io::ErrorKind::InvalidData, "chunk end"));
                        }
                        self.want_size = true;
                    }
                    let mut line = String::new();
                    if r.read_line(&mut line)? == 0 {
                        return Err(io::ErrorKind::UnexpectedEof.into());
                    }
                    let size = line.trim().split(';').next().unwrap_or("");
                    let size = usize::from_str_radix(size, 16)
                        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "chunk size"))?;
                    if size == 0 {
                        // the trailer: lines up to a blank one
                        loop {
                            line.clear();
                            if r.read_line(&mut line)? == 0 || line.trim().is_empty() {
                                break;
                            }
                        }
                        self.done = true;
                        return Ok(0);
                    }
                    self.left = size;
                    self.want_size = false;
                }
                let want = out.len().min(self.left);
                let n = r.read(&mut out[..want])?;
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                self.left -= n;
                Ok(n)
            }
        }
    }
}

/// The body of an answer as a reader: it ends where the body does, whatever the framing.
pub struct BodyReader {
    conn: BufReader<Stream>,
    state: BodyState,
}

impl Read for BodyReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.state.read(&mut self.conn, out)
    }
}

impl BodyReader {
    /// The socket under the connection, to set what a read may wait.
    pub fn socket(&self) -> &std::net::TcpStream {
        &self.conn.get_ref().sock
    }
}

/// A connection that may be kept.
struct Conn {
    reader: BufReader<Stream>,
    idle_since: Instant,
}

/// An answer read whole.
#[derive(Debug)]
pub struct Reply {
    pub head: Head,
    pub body: Vec<u8>,
}

/// Counts the calls in flight and holds one more back until there is room.
struct Gate {
    count: Mutex<usize>,
    room: Condvar,
}

struct Ticket<'a>(&'a Gate);

impl Gate {
    fn enter(&self, within: Duration) -> Option<Ticket<'_>> {
        let mut count = self.count.lock();
        let deadline = Instant::now() + within;
        while *count >= IN_FLIGHT {
            if self.room.wait_until(&mut count, deadline).timed_out() && *count >= IN_FLIGHT {
                return None;
            }
        }
        *count += 1;
        Some(Ticket(self))
    }
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        *self.0.count.lock() -= 1;
        self.0.room.notify_one();
    }
}

/// The calls to one bridge with one token.
pub struct Client {
    dialer: Dialer,
    token: Token,
    host: String,
    kept: Mutex<Vec<Conn>>,
    gate: Gate,
}

impl Client {
    pub fn new(dialer: Dialer, token: Token) -> Arc<Self> {
        let host = dialer.addr().to_string();
        Arc::new(Self {
            dialer,
            token,
            host,
            kept: Mutex::new(Vec::new()),
            gate: Gate {
                count: Mutex::new(0),
                room: Condvar::new(),
            },
        })
    }

    pub fn addr(&self) -> std::net::SocketAddr {
        self.dialer.addr()
    }

    /// Drops every kept connection (the TV is going to sleep, or the link is).
    pub fn close_kept(&self) {
        self.kept.lock().clear();
    }

    fn dial(&self) -> Result<BufReader<Stream>, HttpError> {
        self.dialer
            .connect(CONNECT_WITHIN, IO_WITHIN)
            .map(BufReader::new)
            .map_err(HttpError::Connect)
    }

    fn take_kept(&self) -> Option<Conn> {
        let mut kept = self.kept.lock();
        while let Some(conn) = kept.pop() {
            if conn.idle_since.elapsed() < KEEP_IDLE {
                return Some(conn);
            }
        }
        None
    }

    /// One call, its answer read whole (at most `max_body` bytes). The call runs on a thread of its own with a stack made for TLS: the caller
    /// may be one of the API's small connection threads, and the handshake's frames (rustls and ring's) are far larger than a handler's.
    pub fn call(&self, request: &Request, max_body: usize) -> Result<Reply, HttpError> {
        thread::scope(|scope| {
            let spawned = thread::Builder::new()
                .name("scrypted-call".to_owned())
                .stack_size(TLS_STACK)
                .spawn_scoped(scope, || self.call_here(request, max_body));
            match spawned {
                Ok(handle) => handle
                    .join()
                    .unwrap_or_else(|_| Err(HttpError::Protocol("the call failed".into()))),
                Err(e) => Err(HttpError::Io {
                    before_answer: true,
                    why: e.kind().to_string(),
                }),
            }
        })
    }

    fn call_here(&self, request: &Request, max_body: usize) -> Result<Reply, HttpError> {
        let _ticket = self.gate.enter(CALL_WITHIN).ok_or(HttpError::Timeout)?;
        let deadline = Instant::now() + CALL_WITHIN;
        let mut retried = false;
        loop {
            let (mut conn, reused) = match self.take_kept() {
                Some(conn) => (conn.reader, true),
                None => (self.dial()?, false),
            };
            match self.once(&mut conn, request, max_body, deadline) {
                Ok((reply, reusable)) => {
                    if reusable && self.kept.lock().len() < KEEP_MAX {
                        self.kept.lock().push(Conn {
                            reader: conn,
                            idle_since: Instant::now(),
                        });
                    }
                    return Ok(reply);
                }
                // a kept connection the server had closed: nothing of ours was seen, so the call is made again on a fresh one
                Err(HttpError::Io {
                    before_answer: true,
                    ..
                }) if reused && !retried => {
                    retried = true;
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn once(
        &self,
        conn: &mut BufReader<Stream>,
        request: &Request,
        max_body: usize,
        deadline: Instant,
    ) -> Result<(Reply, bool), HttpError> {
        request
            .write(conn.get_mut(), &self.host, &self.token, true)
            .map_err(|e| from_io(&e, true))?;
        let head = Head::read(conn)?;
        let mut state = BodyState::of(&head);
        let mut body = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(HttpError::Timeout);
            }
            let _ = conn
                .get_ref()
                .sock
                .set_read_timeout(Some(left.min(IO_WITHIN)));
            let n = state
                .read(conn, &mut chunk)
                .map_err(|e| from_io(&e, false))?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
            if body.len() > max_body {
                return Err(HttpError::Protocol("an answer over the limit".into()));
            }
        }
        let reusable = head.keeps_alive() && state.reusable() && conn.buffer().is_empty();
        Ok((Reply { head, body }, reusable))
    }

    /// A dedicated connection for a stream: the request is sent, the head read, and the body handed over as a reader. The connection is
    /// never kept; ending the reader closes it. `read_within` is what one read may wait (a stream that goes quiet is the caller's to
    /// notice, so this is the longest silence it will sit through).
    pub fn open(
        &self,
        request: &Request,
        read_within: Duration,
    ) -> Result<(Head, BodyReader), HttpError> {
        let mut conn = self.dial()?;
        request
            .write(conn.get_mut(), &self.host, &self.token, false)
            .map_err(|e| from_io(&e, true))?;
        // the head of a stream may wait for its first picture (the camera's next key picture): it is given as long as a picture is
        let _ = conn
            .get_ref()
            .sock
            .set_read_timeout(Some(read_within.max(HEAD_WITHIN)));
        let head = Head::read(&mut conn)?;
        let _ = conn.get_ref().sock.set_read_timeout(Some(read_within));
        let state = BodyState::of(&head);
        Ok((head, BodyReader { conn, state }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn read_all(head: &Head, wire: &[u8]) -> io::Result<Vec<u8>> {
        let mut cursor = Cursor::new(wire.to_vec());
        let mut state = BodyState::of(head);
        let mut out = Vec::new();
        let mut buf = [0u8; 7];
        loop {
            let n = state.read(&mut cursor, &mut buf)?;
            if n == 0 {
                return Ok(out);
            }
            out.extend_from_slice(&buf[..n]);
        }
    }

    #[test]
    fn a_head_is_read_with_its_headers_and_a_bad_one_is_refused() {
        let wire = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: keep-alive\r\nKeep-Alive: timeout=30\r\n\r\n{}";
        let mut cursor = Cursor::new(wire.to_vec());
        let head = Head::read(&mut cursor).unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(head.header("content-type"), Some("application/json"));
        assert!(head.keeps_alive());
        assert_eq!(head.framing(), Framing::Length(2));
        let mut rest = Vec::new();
        cursor.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"{}");
        for bad in [
            &b"GARBAGE\r\n\r\n"[..],
            b"HTTP/2 200\r\n\r\n",
            b"HTTP/1.1 abc\r\n\r\n",
        ] {
            assert!(matches!(
                Head::read(&mut Cursor::new(bad.to_vec())),
                Err(HttpError::Protocol(_))
            ));
        }
        // a connection that closes before any answer is the retryable kind
        assert!(matches!(
            Head::read(&mut Cursor::new(Vec::new())),
            Err(HttpError::Io {
                before_answer: true,
                ..
            })
        ));
        assert!(matches!(
            Head::read(&mut Cursor::new(b"HTTP/1.1 200 OK\r\nA: b\r\n".to_vec())),
            Err(HttpError::Io {
                before_answer: false,
                ..
            })
        ));
        let long = format!("HTTP/1.1 200 OK\r\nX: {}\r\n\r\n", "a".repeat(20_000));
        assert!(matches!(
            Head::read(&mut Cursor::new(long.into_bytes())),
            Err(HttpError::Protocol(_))
        ));
    }

    #[test]
    fn a_body_is_framed_by_its_length_by_chunks_or_by_the_connections_end() {
        let by_length = Head {
            status: 200,
            headers: vec![("Content-Length".into(), "11".into())],
        };
        assert_eq!(
            read_all(&by_length, b"hello worldEXTRA").unwrap(),
            b"hello world"
        );
        assert!(
            read_all(&by_length, b"short").is_err(),
            "a body that ends early is an error"
        );
        let chunked = Head {
            status: 200,
            headers: vec![("Transfer-Encoding".into(), "chunked".into())],
        };
        let wire = b"5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\nTrailer: x\r\n\r\n";
        assert_eq!(read_all(&chunked, wire).unwrap(), b"hello world");
        assert!(read_all(&chunked, b"5\r\nhel").is_err());
        assert!(read_all(&chunked, b"zz\r\n").is_err());
        assert!(
            read_all(&chunked, b"5\r\nhelloXX").is_err(),
            "a chunk must end with CRLF"
        );
        let by_close = Head::default();
        assert_eq!(
            read_all(&by_close, b"until the end").unwrap(),
            b"until the end"
        );
        // an empty body is over before it starts
        let empty = Head {
            status: 204,
            headers: vec![("Content-Length".into(), "0".into())],
        };
        assert!(BodyState::of(&empty).finished());
    }

    #[test]
    fn a_connection_is_reusable_only_when_the_end_of_the_body_was_known() {
        let mut state = BodyState::of(&Head {
            status: 200,
            headers: vec![("Content-Length".into(), "3".into())],
        });
        assert!(!state.reusable());
        let mut cursor = Cursor::new(b"abc".to_vec());
        let mut buf = [0u8; 8];
        assert_eq!(state.read(&mut cursor, &mut buf).unwrap(), 3);
        assert!(state.reusable());
        let mut closing = BodyState::of(&Head::default());
        let mut cursor = Cursor::new(b"abc".to_vec());
        while closing.read(&mut cursor, &mut buf).unwrap() > 0 {}
        assert!(
            !closing.reusable(),
            "the connection's end is not a message boundary"
        );
    }
}
