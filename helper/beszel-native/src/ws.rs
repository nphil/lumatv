//! A WebSocket client for the hub's agent endpoint: an HTTP/1.1 upgrade over TCP or TLS, then binary
//! messages, pings and close. No extensions and no proxy.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ring::rand::{SecureRandom, SystemRandom};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, ServerName};

const ACCEPT_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const DIAL_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a connection may go without a message or ping from the hub, as in the Go agent.
const DEADLINE: Duration = Duration::from_secs(120);
const MAX_HEAD: usize = 16 << 10;
const MAX_REFUSAL_BODY: usize = 4 << 10;
const MAX_MESSAGE: usize = 16 << 20;

/// The hub's agent endpoint, derived from `HUB_URL` as the Go agent derives it.
#[derive(Debug, PartialEq)]
pub struct Endpoint {
    pub secure: bool,
    /// Host name or address for the lookup and TLS, without IPv6 brackets.
    pub host: String,
    pub port: u16,
    /// The URL's host and port as written: the `Host` header and the name in the logs.
    pub authority: String,
    /// Request path and query.
    pub target: String,
}

impl Endpoint {
    /// `None` when the URL has no scheme or host.
    pub fn parse(hub_url: &str) -> Option<Endpoint> {
        let (scheme, rest) = hub_url.split_once("://")?;
        let mut chars = scheme.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return None;
        }
        let secure = scheme.eq_ignore_ascii_case("https");
        let rest = rest.split_once('#').map_or(rest, |(before, _)| before);
        let split = rest.find(['/', '?']).unwrap_or(rest.len());
        let (authority, path_query) = rest.split_at(split);
        let authority = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
        let (host, port) = split_host_port(authority)?;
        if host.is_empty() {
            return None;
        }
        let port = match port {
            Some(port) => port,
            None if secure => 443,
            None => 80,
        };
        let (path, query) = path_query.split_once('?').unwrap_or((path_query, ""));
        let mut target = join_path(path, "api/beszel/agent-connect");
        if !query.is_empty() {
            target.push('?');
            target.push_str(query);
        }
        Some(Endpoint { secure, host: host.to_owned(), port, authority: authority.to_owned(), target })
    }
}

fn split_host_port(authority: &str) -> Option<(&str, Option<u16>)> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        match after {
            "" => (host, None),
            _ => (host, Some(after.strip_prefix(':')?)),
        }
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    match port {
        None | Some("") => Some((host, None)),
        Some(port) => Some((host, Some(port.parse().ok()?))),
    }
}

/// Go's `path.Join(base, tail)` with the leading slash a URL path always has.
fn join_path(base: &str, tail: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in base.split('/').chain(tail.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}

/// The roots a `wss` connection trusts: Mozilla's set, plus the certificates in `CA_CERT_FILE`.
pub fn tls_config(ca_cert_file: Option<&str>) -> Result<Arc<rustls::ClientConfig>, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(path) = ca_cert_file {
        let pem = std::fs::read(path).map_err(|err| format!("read CA_CERT_FILE {path:?}: {err}"))?;
        let added = CertificateDer::pem_slice_iter(&pem)
            .filter_map(Result::ok)
            .filter(|cert| roots.add(cert.clone()).is_ok())
            .count();
        if added == 0 {
            return Err(format!("CA_CERT_FILE {path:?} does not contain any valid PEM certificates"));
        }
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| err.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

#[derive(Debug)]
pub enum ConnectError {
    /// The hub answered the upgrade with this status and body instead of switching protocols.
    Refused { status: u16, body: String },
    Failed(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::Refused { status, body } if body.is_empty() => write!(f, "hub refused the connection: {status}"),
            ConnectError::Refused { status, body } => write!(f, "hub refused the connection: {status} {body}"),
            ConnectError::Failed(err) => f.write_str(err),
        }
    }
}

enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(tcp) => tcp,
            Stream::Tls(tls) => tls.get_ref(),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(tcp) => tcp.read(buf),
            Stream::Tls(tls) => tls.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(tcp) => tcp.write(buf),
            Stream::Tls(tls) => tls.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(tcp) => tcp.flush(),
            Stream::Tls(tls) => tls.flush(),
        }
    }
}

pub enum Message {
    Binary(Vec<u8>),
    /// The hub closed the connection; the text describes its close frame.
    Closed(String),
}

pub struct Conn {
    stream: Stream,
    buf: Vec<u8>,
    start: usize,
    rng: SystemRandom,
}

/// Opens the connection; `headers` are sent with the upgrade request as given.
pub fn connect(
    endpoint: &Endpoint,
    headers: &[(String, String)],
    tls: Option<&Arc<rustls::ClientConfig>>,
) -> Result<Conn, ConnectError> {
    let failed = |err: String| ConnectError::Failed(err);
    let addrs = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|err| failed(format!("dial tcp: lookup {}: {err}", endpoint.host)))?;
    let mut last = None;
    let mut tcp = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, DIAL_TIMEOUT) {
            Ok(stream) => {
                tcp = Some(stream);
                break;
            }
            Err(err) => last = Some(format!("dial tcp {addr}: {err}")),
        }
    }
    let tcp = tcp.ok_or_else(|| failed(last.unwrap_or_else(|| format!("dial tcp: lookup {}: no address", endpoint.host))))?;
    let io = |err: io::Error| failed(describe(&err));
    tcp.set_nodelay(true).map_err(io)?;
    tcp.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).map_err(io)?;
    tcp.set_write_timeout(Some(HANDSHAKE_TIMEOUT)).map_err(io)?;
    let stream = match (endpoint.secure, tls) {
        (true, Some(config)) => {
            let name = ServerName::try_from(endpoint.host.clone()).map_err(|err| failed(err.to_string()))?;
            let session = rustls::ClientConnection::new(config.clone(), name).map_err(|err| failed(err.to_string()))?;
            Stream::Tls(Box::new(rustls::StreamOwned::new(session, tcp)))
        }
        (true, None) => return Err(failed("no TLS configuration".into())),
        (false, _) => Stream::Plain(tcp),
    };
    let rng = SystemRandom::new();
    let mut nonce = [0u8; 16];
    rng.fill(&mut nonce).map_err(|_| failed("no randomness for the handshake".into()))?;
    let key = STANDARD.encode(nonce);
    let mut conn = Conn { stream, buf: Vec::new(), start: 0, rng };
    let mut request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n",
        endpoint.target, endpoint.authority
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    conn.stream.write_all(request.as_bytes()).map_err(io)?;
    conn.stream.flush().map_err(io)?;

    let head_len = conn.read_head().map_err(io)?;
    let head = String::from_utf8_lossy(&conn.buf[..head_len]).into_owned();
    conn.start = head_len;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| failed("malformed HTTP response".into()))?;
    let header = |name: &str| {
        head.split("\r\n").skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_owned())
        })
    };
    if status != 101 {
        let length = header("content-length").and_then(|len| len.parse::<usize>().ok());
        let body = conn.read_refusal_body(length);
        return Err(ConnectError::Refused { status, body });
    }
    let accept = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, format!("{key}{ACCEPT_GUID}").as_bytes());
    let upgraded = header("upgrade").is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    if !upgraded || header("sec-websocket-accept").as_deref() != Some(STANDARD.encode(accept.as_ref()).as_str()) {
        return Err(failed("websocket handshake failed: bad upgrade response".into()));
    }
    let tcp = conn.stream.tcp();
    tcp.set_read_timeout(Some(DEADLINE)).map_err(io)?;
    tcp.set_write_timeout(Some(DEADLINE)).map_err(io)?;
    Ok(conn)
}

/// Error text for the logs; a read past the deadline reads as Go's.
pub fn describe(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => "i/o timeout".into(),
        io::ErrorKind::UnexpectedEof => "EOF".into(),
        _ => err.to_string(),
    }
}

impl Conn {
    /// Reads until the end of the response head and returns its length.
    fn read_head(&mut self) -> io::Result<usize> {
        let mut searched = 0;
        loop {
            if let Some(at) = self.buf[searched..].windows(4).position(|w| w == b"\r\n\r\n") {
                return Ok(searched + at + 4);
            }
            searched = self.buf.len().saturating_sub(3);
            if self.buf.len() > MAX_HEAD {
                return Err(io::Error::other("HTTP response head too large"));
            }
            self.read_more()?;
        }
    }

    fn read_refusal_body(&mut self, length: Option<usize>) -> String {
        let want = length.unwrap_or(MAX_REFUSAL_BODY).min(MAX_REFUSAL_BODY);
        while self.buf.len() - self.start < want {
            if self.read_more().is_err() {
                break;
            }
        }
        let end = (self.start + want).min(self.buf.len());
        String::from_utf8_lossy(&self.buf[self.start..end]).trim().to_owned()
    }

    fn read_more(&mut self) -> io::Result<()> {
        let mut chunk = [0u8; 4096];
        let n = self.stream.read(&mut chunk)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(())
    }

    fn fill(&mut self, n: usize) -> io::Result<()> {
        if self.start > 0 && self.buf.len() - self.start < n {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        while self.buf.len() - self.start < n {
            self.read_more()?;
        }
        Ok(())
    }

    /// Reads one frame: whether it ends a message, its opcode and payload.
    fn frame(&mut self) -> io::Result<(bool, u8, Vec<u8>)> {
        self.fill(2)?;
        let (b0, b1) = (self.buf[self.start], self.buf[self.start + 1]);
        if b0 & 0x70 != 0 {
            return Err(protocol("reserved bits set"));
        }
        let (fin, opcode, masked) = (b0 & 0x80 != 0, b0 & 0x0f, b1 & 0x80 != 0);
        let (len, mut at) = match b1 & 0x7f {
            126 => {
                self.fill(4)?;
                (u64::from(u16::from_be_bytes([self.buf[self.start + 2], self.buf[self.start + 3]])), 4)
            }
            127 => {
                self.fill(10)?;
                let bytes = self.buf[self.start + 2..self.start + 10].try_into().unwrap();
                (u64::from_be_bytes(bytes), 10)
            }
            len => (u64::from(len), 2),
        };
        if opcode >= 8 && (len > 125 || !fin) {
            return Err(protocol("malformed control frame"));
        }
        let len = usize::try_from(len).ok().filter(|&len| len <= MAX_MESSAGE).ok_or_else(|| protocol("message too large"))?;
        let mask_at = at;
        if masked {
            at += 4;
        }
        self.fill(at + len)?;
        let begin = self.start + at;
        let mut payload = self.buf[begin..begin + len].to_vec();
        if masked {
            let mask: [u8; 4] = self.buf[self.start + mask_at..self.start + mask_at + 4].try_into().unwrap();
            payload.iter_mut().enumerate().for_each(|(i, byte)| *byte ^= mask[i % 4]);
        }
        self.start += at + len;
        Ok((fin, opcode, payload))
    }

    /// Waits for the next binary message, answering pings and skipping text.
    pub fn read(&mut self) -> io::Result<Message> {
        let mut partial: Option<(u8, Vec<u8>)> = None;
        loop {
            let (fin, opcode, payload) = self.frame()?;
            let (kind, data) = match opcode {
                0x0 => {
                    let (kind, mut data) = partial.take().ok_or_else(|| protocol("unexpected continuation frame"))?;
                    if data.len() + payload.len() > MAX_MESSAGE {
                        return Err(protocol("message too large"));
                    }
                    data.extend_from_slice(&payload);
                    (kind, data)
                }
                0x1 | 0x2 if partial.is_none() => (opcode, payload),
                0x8 => {
                    let text = match payload.as_slice() {
                        [hi, lo, reason @ ..] => {
                            let code = u16::from_be_bytes([*hi, *lo]);
                            let _ = self.send_frame(0x8, &code.to_be_bytes());
                            format!("code={code}, reason={}", String::from_utf8_lossy(reason))
                        }
                        _ => {
                            let _ = self.send_frame(0x8, &[]);
                            "code=1005".to_owned()
                        }
                    };
                    return Ok(Message::Closed(text));
                }
                0x9 => {
                    self.send_frame(0xa, &payload)?;
                    continue;
                }
                0xa => continue,
                _ => return Err(protocol("unexpected frame")),
            };
            if !fin {
                partial = Some((kind, data));
            } else if kind == 0x2 {
                return Ok(Message::Binary(data));
            }
        }
    }

    pub fn send(&mut self, payload: &[u8]) -> io::Result<()> {
        self.send_frame(0x2, payload)
    }

    /// Sends a normal close; the connection is dropped right after, so failures do not matter.
    pub fn close(&mut self) {
        let _ = self.send_frame(0x8, &1000u16.to_be_bytes());
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> io::Result<()> {
        let len = payload.len();
        let mut frame = Vec::with_capacity(len + 14);
        frame.push(0x80 | opcode);
        if len < 126 {
            frame.push(0x80 | len as u8);
        } else if let Ok(len) = u16::try_from(len) {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&len.to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        let mut mask = [0u8; 4];
        self.rng.fill(&mut mask).map_err(|_| io::Error::other("no randomness for the frame mask"))?;
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, byte)| byte ^ mask[i % 4]));
        self.stream.write_all(&frame)?;
        self.stream.flush()
    }
}

fn protocol(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn derives_the_endpoint_like_the_go_agent() {
        let ep = Endpoint::parse("https://Hub.example.com/beszel/?a=1").unwrap();
        assert_eq!(
            ep,
            Endpoint {
                secure: true,
                host: "Hub.example.com".into(),
                port: 443,
                authority: "Hub.example.com".into(),
                target: "/beszel/api/beszel/agent-connect?a=1".into(),
            }
        );
        let ep = Endpoint::parse("http://[fd00::2]:8090").unwrap();
        assert_eq!((ep.secure, ep.host.as_str(), ep.port, ep.authority.as_str()), (false, "fd00::2", 8090, "[fd00::2]:8090"));
        assert_eq!(ep.target, "/api/beszel/agent-connect");
        assert!(Endpoint::parse("hub.example.com:8090").is_none());
        assert!(Endpoint::parse("http://").is_none());
    }

    /// Answers one upgrade request with `reply`, then hands the socket to `then`.
    fn hub(
        reply: impl FnOnce(&str) -> String + Send + 'static,
        then: impl FnOnce(TcpStream) + Send + 'static,
    ) -> (Endpoint, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8];
            while !request.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            socket.write_all(reply(&request).as_bytes()).unwrap();
            then(socket);
        });
        (Endpoint::parse(&format!("http://127.0.0.1:{port}")).unwrap(), server)
    }

    fn upgrade(request: &str) -> String {
        let key = request.lines().find_map(|line| line.strip_prefix("Sec-WebSocket-Key: ")).unwrap();
        let accept = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, format!("{key}{ACCEPT_GUID}").as_bytes());
        format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            STANDARD.encode(accept.as_ref())
        )
    }

    fn read_client_frame(socket: &mut TcpStream) -> (u8, Vec<u8>) {
        let mut head = [0u8; 2];
        socket.read_exact(&mut head).unwrap();
        assert_eq!(head[1] & 0x80, 0x80, "client frames are masked");
        let mut mask = [0u8; 4];
        socket.read_exact(&mut mask).unwrap();
        let mut payload = vec![0u8; usize::from(head[1] & 0x7f)];
        socket.read_exact(&mut payload).unwrap();
        payload.iter_mut().enumerate().for_each(|(i, b)| *b ^= mask[i % 4]);
        (head[0] & 0x0f, payload)
    }

    #[test]
    fn exchanges_messages_answers_pings_and_follows_close() {
        let (endpoint, server) = hub(upgrade, |mut socket| {
            // A ping inside a fragmented binary message, a text message, a binary message, a close.
            socket.write_all(&[0x02, 2, b'h', b'e', 0x89, 1, b'p', 0x80, 1, b'y']).unwrap();
            socket.write_all(&[0x81, 1, b't', 0x82, 1, b'z', 0x88, 4, 0x03, 0xe8, b'o', b'k']).unwrap();
            assert_eq!(read_client_frame(&mut socket), (0xa, b"p".to_vec()));
            assert_eq!(read_client_frame(&mut socket), (0x2, b"reply".to_vec()));
            assert_eq!(read_client_frame(&mut socket), (0x8, vec![0x03, 0xe8]));
        });
        let headers = [("X-Token".to_owned(), "t".to_owned())];
        let mut conn = connect(&endpoint, &headers, None).unwrap();
        assert!(matches!(conn.read().unwrap(), Message::Binary(m) if m == b"hey"));
        assert!(matches!(conn.read().unwrap(), Message::Binary(m) if m == b"z"));
        conn.send(b"reply").unwrap();
        assert!(matches!(conn.read().unwrap(), Message::Closed(text) if text == "code=1000, reason=ok"));
        server.join().unwrap();
    }

    #[test]
    fn reports_a_refused_upgrade_with_its_body() {
        let (endpoint, server) = hub(
            |request| {
                assert!(request.starts_with("GET /api/beszel/agent-connect HTTP/1.1\r\n"));
                assert!(request.contains("\r\nX-Beszel: 0.21.1\r\n"));
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 21\r\n\r\nInvalid agent version".into()
            },
            drop,
        );
        let headers = [("X-Beszel".to_owned(), "0.21.1".to_owned())];
        match connect(&endpoint, &headers, None) {
            Err(ConnectError::Refused { status: 401, body }) => assert_eq!(body, "Invalid agent version"),
            other => panic!("{:?}", other.err()),
        }
        server.join().unwrap();
    }
}
