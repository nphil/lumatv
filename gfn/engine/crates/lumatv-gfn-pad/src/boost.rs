//! The reader threads' real-time slot, asked of `lumatv-helper` (helper/app/src/gfn_boost.rs): the app's jail cannot raise its own
//! threads' priority, the helper is root and does it for the one kind of thread it is told about, with FIFO 10 and an affinity off
//! CPU 0, as a lease that the helper ends by itself after 90 s. So while the stream is in front this asks every 30 s, and when it
//! leaves (or the runtime stops) it says so with `pad-unboost`.
//!
//! The request is a tiny HTTP/1.1 `POST` over loopback with the `X-Luma-Client` header every write to the helper carries. A refusal
//! or an unreachable helper costs nothing but a line in the log, once per thread and reason: the pad works unboosted.
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

pub const BOOST_PATH: &str = "/v1/gfn/pad-boost";
pub const UNBOOST_PATH: &str = "/v1/gfn/pad-unboost";
const TIMEOUT: Duration = Duration::from_millis(800);

/// The helper's loopback address: `LUMATV_GFN_BOOST_ADDR` if set, nothing for `LUMATV_GFN_BOOST=0`, else its default port.
pub fn addr_from_env() -> Option<SocketAddr> {
    if std::env::var("LUMATV_GFN_BOOST").is_ok_and(|v| v == "0") {
        return None;
    }
    match std::env::var("LUMATV_GFN_BOOST_ADDR") {
        Ok(a) => a.parse().ok(),
        Err(_) => Some(SocketAddr::from(([127, 0, 0, 1], 18731))),
    }
}

/// The answer's status and body.
pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    /// The helper's refusal in words (`error.message`), or the status when the body is not its envelope.
    pub fn why(&self) -> String {
        let key = "\"message\":\"";
        self.body.find(key).and_then(|i| self.body[i + key.len()..].split('"').next().map(str::to_owned)).unwrap_or_else(|| format!("status {}", self.status))
    }
}

pub fn post(addr: SocketAddr, path: &str, body: &str) -> io::Result<Reply> {
    let mut s = TcpStream::connect_timeout(&addr, TIMEOUT)?;
    s.set_read_timeout(Some(TIMEOUT))?;
    s.set_write_timeout(Some(TIMEOUT))?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nX-Luma-Client: gfn-pad\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(request.as_bytes())?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) && !raw.is_empty() => break,
            Err(e) => return Err(e),
        }
        if raw.len() > 64 * 1024 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no HTTP status"))?;
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default();
    Ok(Reply { status, body })
}

fn ids(tid: i32) -> String {
    format!("{{\"pid\":{},\"tid\":{tid}}}", std::process::id())
}

/// Ask for (or renew) the boost of one reader thread. `Err` is the reason in words, for the log.
pub fn boost(addr: SocketAddr, tid: i32) -> Result<(), String> {
    match post(addr, BOOST_PATH, &ids(tid)) {
        Ok(r) if r.status == 200 => Ok(()),
        Ok(r) => Err(r.why()),
        Err(e) => Err(format!("the helper at {addr} did not answer: {e}")),
    }
}

pub fn unboost(addr: SocketAddr, tid: i32) {
    let _ = post(addr, UNBOOST_PATH, &ids(tid));
}
