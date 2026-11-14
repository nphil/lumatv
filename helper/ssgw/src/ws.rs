//! WebSocket (RFC 6455) frames, message assembly and the HTTP/1.1 heads around the upgrade.

use crate::util::{b64_encode, random_bytes};
use ring::digest;

pub const OP_CONT: u8 = 0x0;
pub const OP_TEXT: u8 = 0x1;
pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xA;

/// The longest message accepted at all; LG's own limit (answered with 413, `ssap::MSG_MAX`) is below it.
pub const HARD_MAX: usize = 4 * 1024 * 1024;
/// The longest request head read before the connection is dropped.
pub const HEAD_MAX: usize = 16 * 1024;

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub struct Frame {
    pub fin: bool,
    pub op: u8,
    pub payload: Vec<u8>,
}

pub enum Parsed {
    Frame(Frame, usize),
    Incomplete,
    /// A protocol violation; the close code to answer with.
    Fail(u16),
}

/// Reads one frame from the front of `buf`, unmasking it when it is masked.
pub fn parse_frame(buf: &[u8]) -> Parsed {
    if buf.len() < 2 {
        return Parsed::Incomplete;
    }
    let (b0, b1) = (buf[0], buf[1]);
    let fin = b0 & 0x80 != 0;
    let op = b0 & 0x0F;
    if b0 & 0x70 != 0 || !matches!(op, OP_CONT | OP_TEXT | OP_BINARY | OP_CLOSE | OP_PING | OP_PONG) {
        return Parsed::Fail(1002);
    }
    let masked = b1 & 0x80 != 0;
    let mut pos = 2;
    let len = match b1 & 0x7F {
        126 => {
            if buf.len() < 4 {
                return Parsed::Incomplete;
            }
            pos = 4;
            u64::from(u16::from_be_bytes([buf[2], buf[3]]))
        }
        127 => {
            if buf.len() < 10 {
                return Parsed::Incomplete;
            }
            pos = 10;
            u64::from_be_bytes(buf[2..10].try_into().expect("8 bytes"))
        }
        n => u64::from(n),
    };
    if op >= OP_CLOSE && (len > 125 || !fin) {
        return Parsed::Fail(1002);
    }
    if len > HARD_MAX as u64 {
        return Parsed::Fail(1009);
    }
    let len = len as usize;
    let mask = if masked {
        if buf.len() < pos + 4 {
            return Parsed::Incomplete;
        }
        let m = [buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]];
        pos += 4;
        Some(m)
    } else {
        None
    };
    if buf.len() < pos + len {
        return Parsed::Incomplete;
    }
    let mut payload = buf[pos..pos + len].to_vec();
    if let Some(m) = mask {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= m[i & 3];
        }
    }
    Parsed::Frame(Frame { fin, op, payload }, pos + len)
}

/// Appends one unfragmented frame; `mask` is for frames a client sends.
pub fn encode(out: &mut Vec<u8>, op: u8, payload: &[u8], mask: bool) {
    out.push(0x80 | op);
    let bit = if mask { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => out.push(bit | n as u8),
        n if n <= 0xFFFF => {
            out.push(bit | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(bit | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    if mask {
        let m: [u8; 4] = random_bytes();
        out.extend_from_slice(&m);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ m[i & 3]));
    } else {
        out.extend_from_slice(payload);
    }
}

/// A close frame's body: no code at all when `code` is `None` (what `ws` sends for a plain `close()`).
pub fn close_body(code: Option<u16>, reason: &str) -> Vec<u8> {
    let mut body = Vec::new();
    if let Some(code) = code {
        body.extend_from_slice(&code.to_be_bytes());
        body.extend_from_slice(reason.as_bytes());
    }
    body
}

pub enum Msg {
    Text(Vec<u8>),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong,
    Close(Vec<u8>),
}

/// Joins fragmented data frames into messages; control frames pass straight through.
#[derive(Default)]
pub struct Assembler {
    op: Option<u8>,
    buf: Vec<u8>,
}

impl Assembler {
    pub fn push(&mut self, f: Frame) -> Result<Option<Msg>, u16> {
        match f.op {
            OP_PING => return Ok(Some(Msg::Ping(f.payload))),
            OP_PONG => return Ok(Some(Msg::Pong)),
            OP_CLOSE => return Ok(Some(Msg::Close(f.payload))),
            OP_CONT => {
                if self.op.is_none() {
                    return Err(1002);
                }
            }
            _ => {
                if self.op.is_some() {
                    return Err(1002);
                }
                if f.fin {
                    return Ok(Some(data_msg(f.op, f.payload)));
                }
                self.op = Some(f.op);
            }
        }
        if self.buf.len() + f.payload.len() > HARD_MAX {
            return Err(1009);
        }
        self.buf.extend_from_slice(&f.payload);
        if !f.fin {
            return Ok(None);
        }
        let op = self.op.take().unwrap_or(OP_TEXT);
        Ok(Some(data_msg(op, std::mem::take(&mut self.buf))))
    }
}

fn data_msg(op: u8, payload: Vec<u8>) -> Msg {
    if op == OP_BINARY { Msg::Binary(payload) } else { Msg::Text(payload) }
}

pub struct Head {
    pub method: String,
    pub target: String,
    /// The status code, for a response head.
    pub status: u16,
    headers: Vec<(String, String)>,
    pub len: usize,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Whether this request asks for a WebSocket.
    pub fn is_upgrade(&self) -> bool {
        self.method == "GET" && self.header("upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
    }
}

/// Parses a request or response head at the front of `buf`: `Ok(None)` until it is complete, `Err` when it is malformed or too long.
pub fn parse_head(buf: &[u8]) -> Result<Option<Head>, ()> {
    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return if buf.len() > HEAD_MAX { Err(()) } else { Ok(None) };
    };
    if end > HEAD_MAX {
        return Err(());
    }
    let text = std::str::from_utf8(&buf[..end]).map_err(|_| ())?;
    let mut lines = text.split("\r\n");
    let first = lines.next().ok_or(())?;
    let mut parts = first.splitn(3, ' ');
    let (a, b) = (parts.next().ok_or(())?, parts.next().ok_or(())?);
    let (method, target, status) = if a.starts_with("HTTP/") {
        (String::new(), String::new(), b.parse().map_err(|_| ())?)
    } else {
        let version = parts.next().ok_or(())?;
        if !version.starts_with("HTTP/1.") {
            return Err(());
        }
        (a.to_owned(), b.to_owned(), 0)
    };
    let mut headers = Vec::new();
    for line in lines {
        let (k, v) = line.split_once(':').ok_or(())?;
        headers.push((k.trim().to_owned(), v.trim().to_owned()));
    }
    Ok(Some(Head { method, target, status, headers, len: end + 4 }))
}

pub fn accept_key(key: &str) -> String {
    let mut ctx = digest::Context::new(&digest::SHA1_FOR_LEGACY_USE_ONLY);
    ctx.update(key.as_bytes());
    ctx.update(GUID.as_bytes());
    b64_encode(ctx.finish().as_ref())
}

pub fn server_handshake(key: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        accept_key(key)
    )
    .into_bytes()
}

/// The upgrade request this gateway sends LG's; returns the request and the key whose accept value the answer must carry.
pub fn client_handshake(host: &str, path: &str) -> (Vec<u8>, String) {
    let key = b64_encode(&random_bytes::<16>());
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    (req.into_bytes(), key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_key_matches_rfc_example() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn masked_frames_round_trip_and_fragments_join() {
        let mut wire = Vec::new();
        encode(&mut wire, OP_TEXT, &[b'x'; 300], true);
        let Parsed::Frame(f, used) = parse_frame(&wire) else { panic!("frame") };
        assert_eq!((used, f.op, f.payload.len()), (wire.len(), OP_TEXT, 300));
        assert!(f.payload.iter().all(|b| *b == b'x'));
        assert!(matches!(parse_frame(&wire[..wire.len() - 1]), Parsed::Incomplete));

        let mut asm = Assembler::default();
        let first = Frame { fin: false, op: OP_TEXT, payload: b"ab".to_vec() };
        assert!(matches!(asm.push(first), Ok(None)));
        assert!(matches!(asm.push(Frame { fin: true, op: OP_PING, payload: vec![] }), Ok(Some(Msg::Ping(_)))));
        let Ok(Some(Msg::Text(t))) = asm.push(Frame { fin: true, op: OP_CONT, payload: b"c".to_vec() }) else { panic!("text") };
        assert_eq!(t, b"abc");
        assert!(matches!(asm.push(Frame { fin: true, op: OP_CONT, payload: vec![] }), Err(1002)));
    }

    #[test]
    fn heads_parse_requests_and_responses() {
        let req = b"GET /resources/x HTTP/1.1\r\nHost: tv:3001\r\nUpgrade: WebSocket\r\nConnection: Upgrade\r\n\r\nrest";
        let head = parse_head(req).unwrap().unwrap();
        assert_eq!((head.method.as_str(), head.target.as_str(), head.len), ("GET", "/resources/x", req.len() - 4));
        assert!(head.is_upgrade());
        assert_eq!(head.header("host"), Some("tv:3001"));
        assert!(parse_head(b"GET / HTTP/1.1\r\nHost: a").unwrap().is_none());
        let resp = parse_head(b"HTTP/1.1 101 Switching Protocols\r\nSec-WebSocket-Accept: k\r\n\r\n").unwrap().unwrap();
        assert_eq!(resp.status, 101);
        assert!(parse_head(&vec![b'a'; HEAD_MAX + 1]).is_err());
    }
}
