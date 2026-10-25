//! HTTPS to the Hue bridge (or to Bifrost): the few requests ambient light needs, and nothing else.
//!
//! The helper has no TLS library (a static musl build), so the connection is the image's `openssl s_client`, as it is for the weather
//! and for Home Assistant (`fetch::TlsFetch`). `fetch` itself only does `GET` and `HEAD` against the public authorities, and a
//! bridge's certificate is made by Signify's own authority (Bifrost's is its own), so this module runs the same child with a
//! different trust rule: the connection must present exactly the certificate that was saved when the bridge was linked
//! (`-CAfile <that certificate> -partial_chain`, a leaf as the only anchor), and nothing else is trusted. The name is not checked:
//! the certificate is the identity. A bridge that is replaced, or an impostor on the address, fails the handshake, so no key is
//! sent to it (the keys go on the child's standard input, after the handshake, never on a command line).
//!
//! What it speaks, all of it JSON over HTTP/1.1 with `Connection: close`:
//!
//! - `POST /api` (API v1): [`Bridge::register`], the link button's answer: an application key and a client key.
//! - `GET /auth/v1` (header `hue-application-key`): the application id, in the answer's header `hue-application-id`, which is the
//!   DTLS identity.
//! - `GET /clip/v2/resource/entertainment_configuration[/<id>]`: the entertainment areas, their channels and where each sits.
//! - `PUT /clip/v2/resource/entertainment_configuration/<id>` with `{"action":"start"}` or `{"action":"stop"}`
//!   ([`lumatv_ambient::huestream::start_request`]).
//!
//! A bridge's own words (`errors[].description`) are shown to the person; the keys are in no error, no log line and no `Debug`.

use crate::config::FetchConfig;
use crate::hue_store::Secret;
use lumatv_ambient::huestream::{self, MAX_CHANNELS};
use serde_json::{Value, json};
use std::fmt;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The whole of one request, and the most it may take in.
const TOTAL: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 1024 * 1024;
/// The most of the child's complaint that is kept.
const MAX_STDERR: usize = 4096;
/// What the bridge is told this program is, in the key it makes.
const DEVICE_TYPE: &str = "lumatv#tv";

/// Why a request gave no answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HueError {
    /// Nothing answered: refused, no route, no TLS.
    Unreachable(String),
    /// The certificate it presented is not the saved one (or the saved one cannot be read).
    Untrusted(String),
    /// The answer did not finish in time.
    Timeout,
    /// The bridge answered with this status, and these words (its own, when it gave any).
    Http(u16, String),
    /// The bridge answered 200 and said no, in its words.
    Rejected(String),
    /// Registration: the link button was not pressed.
    LinkButton,
    /// Not an answer that can be trusted to be whole and well formed.
    Malformed(String),
    /// The TLS program could not be run.
    Spawn(String),
}

impl fmt::Display for HueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(why) => write!(f, "the bridge did not answer ({why})"),
            Self::Untrusted(why) => write!(
                f,
                "the bridge did not present the certificate it was linked with ({why})"
            ),
            Self::Timeout => f.write_str("the bridge took too long to answer"),
            Self::Http(status, why) if why.is_empty() => {
                write!(f, "the bridge answered HTTP {status}")
            }
            Self::Http(status, why) => write!(f, "the bridge answered HTTP {status}: {why}"),
            Self::Rejected(why) => write!(f, "the bridge refused: {why}"),
            Self::LinkButton => f.write_str("the link button on the bridge was not pressed"),
            Self::Malformed(why) => write!(f, "the bridge's answer was not understood ({why})"),
            Self::Spawn(why) => write!(f, "cannot run the TLS program: {why}"),
        }
    }
}

impl std::error::Error for HueError {}

/// The `openssl` program that makes the connection, and who it runs as.
#[derive(Clone, Debug)]
pub struct Tls {
    program: PathBuf,
    drop_to: Option<(u32, u32)>,
}

impl Tls {
    pub fn new(program: impl Into<PathBuf>, drop_to: Option<(u32, u32)>) -> Self {
        Self {
            program: program.into(),
            drop_to,
        }
    }

    /// The program and user the weather's HTTPS is made with: the child that parses a certificate holds no privilege when the helper
    /// is root.
    pub fn from_config(config: &FetchConfig) -> Self {
        let program =
            crate::sys::which(&config.openssl).unwrap_or_else(|| config.openssl.clone().into());
        let drop_to = if crate::sys::is_root() {
            config.user.as_deref().and_then(crate::sys::lookup_user)
        } else {
            None
        };
        Self::new(program, drop_to)
    }

    fn command(&self, addr: SocketAddr, trust: Option<&Path>) -> Command {
        let mut command = Command::new(&self.program);
        command.args(["s_client", "-connect"]).arg(addr.to_string());
        match trust {
            Some(pinned) => {
                command
                    .args([
                        "-quiet",
                        "-verify_return_error",
                        "-verify",
                        "8",
                        "-min_protocol",
                        "TLSv1.2",
                        "-no-CApath",
                        "-no-CAstore",
                        "-CAfile",
                    ])
                    .arg(pinned)
                    .arg("-partial_chain");
            }
            None => {
                command.arg("-showcerts");
            }
        }
        if let Some((uid, gid)) = self.drop_to {
            command.uid(uid).gid(gid);
        }
        command
    }
}

/// A running child, killed and collected whenever it is dropped.
struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: Option<JoinHandle<()>>,
    said: Arc<Mutex<Vec<u8>>>,
}

impl Session {
    fn spawn(
        mut command: Command,
        stdin: Stdio,
    ) -> Result<(Self, mpsc::Receiver<Vec<u8>>), HueError> {
        command
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| HueError::Spawn(clean(&e.to_string())))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdin = child.stdin.take();
        let (Some(mut stdout), Some(mut stderr)) = (stdout, stderr) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(HueError::Spawn("the TLS program has no pipes".to_owned()));
        };
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = stdout.read(&mut chunk) {
                if n == 0 || tx.send(chunk[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let said = Arc::new(Mutex::new(Vec::new()));
        let kept = said.clone();
        let reader = thread::spawn(move || {
            let mut chunk = [0u8; 512];
            while let Ok(n) = stderr.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                // Reading on past the cap keeps the child from blocking on a full pipe.
                if let Ok(mut said) = kept.lock()
                    && said.len() < MAX_STDERR
                {
                    said.extend_from_slice(&chunk[..n]);
                }
            }
        });
        Ok((
            Self {
                child,
                stdin,
                stderr: Some(reader),
                said,
            },
            rx,
        ))
    }

    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        match self.stdin.as_mut() {
            Some(stdin) => stdin.write_all(bytes).and_then(|()| stdin.flush()),
            None => Err(std::io::ErrorKind::BrokenPipe.into()),
        }
    }

    fn end(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// What the child said when it ended, as the reason a connection gave nothing.
    fn why(&mut self) -> HueError {
        self.end();
        if let Some(reader) = self.stderr.take() {
            let _ = reader.join();
        }
        let said = self.said.lock().map(|s| s.clone()).unwrap_or_default();
        let text = String::from_utf8_lossy(&said);
        let line = text
            .lines()
            .find(|l| l.contains("verify error") || l.contains("Verification"))
            .or_else(|| text.lines().find(|l| !l.trim().is_empty()))
            .map(clean)
            .unwrap_or_default();
        let lower = line.to_ascii_lowercase();
        if lower.contains("verify") || lower.contains("certificate") && !lower.contains("loading") {
            HueError::Untrusted(line)
        } else if lower.contains("could not open file") || lower.contains("loading ca") {
            HueError::Untrusted("the saved certificate cannot be read".to_owned())
        } else if line.is_empty() {
            HueError::Unreachable("the connection closed".to_owned())
        } else {
            HueError::Unreachable(line)
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.end();
    }
}

fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(160).collect()
}

/// The certificate a bridge presents, as PEM: the first of its chain, taken without trusting it. Used once, when the bridge is
/// linked, to decide what to trust afterwards; the person is standing at the bridge pressing its button.
pub fn scan_certificate(tls: &Tls, addr: SocketAddr) -> Result<String, HueError> {
    let (mut session, rx) = Session::spawn(tls.command(addr, None), Stdio::null())?;
    let deadline = Instant::now() + TOTAL;
    let mut seen = Vec::new();
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(chunk) => {
                seen.extend_from_slice(&chunk);
                if seen.len() > MAX_BYTES {
                    return Err(HueError::Malformed("too much from the bridge".to_owned()));
                }
            }
            Err(RecvTimeoutError::Timeout) => return Err(HueError::Timeout),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let text = String::from_utf8_lossy(&seen);
    let begin = "-----BEGIN CERTIFICATE-----";
    let end = "-----END CERTIFICATE-----";
    let pem = text.find(begin).and_then(|from| {
        let to = text[from..].find(end)? + from + end.len();
        Some(format!("{}\n", &text[from..to]))
    });
    match pem {
        Some(pem)
            if pem.lines().skip(1).all(|l| {
                l.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+/=-_ ".contains(&b))
            }) =>
        {
            Ok(pem)
        }
        Some(_) => Err(HueError::Malformed("the certificate is not PEM".to_owned())),
        None => Err(session.why()),
    }
}

/// One answer.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == b"\r\n\r\n")
}

/// How far an answer has come: `Ok(None)` wants more. `eof` says nothing more will come.
fn parse_reply(buf: &[u8], eof: bool) -> Result<Option<Reply>, HueError> {
    let bad = |why: &str| HueError::Malformed(why.to_owned());
    let Some(end) = header_end(buf) else {
        return if eof {
            Err(bad("the answer ended in its headers"))
        } else if buf.len() > 16 * 1024 {
            Err(bad("the headers are too long"))
        } else {
            Ok(None)
        };
    };
    let head = std::str::from_utf8(&buf[..end]).map_err(|_| bad("the headers are not text"))?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.strip_prefix("HTTP/1."))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| bad("no status line"))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let find = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    };
    let rest = &buf[end + 4..];
    let body =
        if find("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
            match dechunk(rest) {
                Chunks::Whole(body) => Some(body),
                Chunks::More if eof => return Err(bad("the answer was cut short")),
                Chunks::More => None,
                Chunks::Bad => return Err(bad("bad chunking")),
            }
        } else if let Some(length) = find("content-length") {
            let length: usize = length.parse().map_err(|_| bad("bad length"))?;
            if length > MAX_BYTES {
                return Err(bad("the answer is too large"));
            }
            if rest.len() >= length {
                Some(rest[..length].to_vec())
            } else if eof {
                return Err(bad("the answer was cut short"));
            } else {
                None
            }
        } else if eof || status == 204 {
            Some(rest.to_vec())
        } else {
            None
        };
    Ok(body.map(|body| Reply {
        status,
        headers,
        body,
    }))
}

enum Chunks {
    Whole(Vec<u8>),
    More,
    Bad,
}

fn dechunk(mut rest: &[u8]) -> Chunks {
    let mut out = Vec::new();
    loop {
        let Some(line_end) = rest.windows(2).position(|w| w == b"\r\n") else {
            return Chunks::More;
        };
        let Ok(size) = std::str::from_utf8(&rest[..line_end])
            .ok()
            .and_then(|l| l.split(';').next())
            .and_then(|l| usize::from_str_radix(l.trim(), 16).ok())
            .ok_or(())
        else {
            return Chunks::Bad;
        };
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Chunks::Whole(out);
        }
        if size > MAX_BYTES || out.len() + size > MAX_BYTES {
            return Chunks::Bad;
        }
        if rest.len() < size + 2 {
            return Chunks::More;
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}

/// One entertainment channel: its number in the stream and where the area puts it (each from -1 to 1, see
/// [`huestream::channel_light`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Channel {
    pub id: u8,
    pub x: f32,
    pub z: f32,
}

/// An entertainment area.
#[derive(Clone, Debug, PartialEq)]
pub struct Area {
    pub id: String,
    pub name: String,
    /// `screen`, `monitor`, `music`, `3dspace` or `other`.
    pub kind: String,
    /// Streaming now, to whoever `streamer` names.
    pub active: bool,
    /// The application that is streaming to it, when the bridge says.
    pub streamer: Option<String>,
    pub channels: Vec<Channel>,
}

fn area_of(v: &Value) -> Option<Area> {
    let mut channels: Vec<Channel> = v
        .get("channels")?
        .as_array()?
        .iter()
        .filter_map(|c| {
            let id = u8::try_from(c.get("channel_id")?.as_u64()?).ok()?;
            let at = c.get("position")?;
            let number = |k: &str| at.get(k).and_then(Value::as_f64).map(|n| n as f32);
            (usize::from(id) < MAX_CHANNELS).then_some(Channel {
                id,
                x: number("x")?,
                z: number("z")?,
            })
        })
        .collect();
    channels.sort_by_key(|c| c.id);
    channels.dedup_by_key(|c| c.id);
    Some(Area {
        id: v.get("id")?.as_str()?.to_ascii_lowercase(),
        name: clean(
            v.pointer("/metadata/name")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
        kind: clean(
            v.get("configuration_type")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
        active: v.get("status").and_then(Value::as_str) == Some("active"),
        streamer: v
            .pointer("/active_streamer/rid")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase),
        channels,
    })
}

/// A bridge on the home network, reached through the certificate it was linked with.
pub struct Bridge {
    tls: Tls,
    addr: SocketAddr,
    pinned: PathBuf,
    /// The whole of one request; shortened by a caller that must not wait.
    pub total: Duration,
}

impl Bridge {
    pub fn new(tls: Tls, addr: SocketAddr, pinned: PathBuf) -> Self {
        Self {
            tls,
            addr,
            pinned,
            total: TOTAL,
        }
    }

    fn call(
        &self,
        method: &str,
        path: &str,
        key: Option<&Secret>,
        body: Option<&str>,
    ) -> Result<Reply, HueError> {
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nUser-Agent: LumaTV\r\nAccept: application/json\r\nConnection: close\r\n",
            self.addr
        );
        if let Some(key) = key {
            request.push_str("hue-application-key: ");
            request.push_str(key.expose());
            request.push_str("\r\n");
        }
        if let Some(body) = body {
            request.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            ));
        }
        request.push_str("\r\n");
        if let Some(body) = body {
            request.push_str(body);
        }
        let (mut session, rx) = Session::spawn(
            self.tls.command(self.addr, Some(&self.pinned)),
            Stdio::piped(),
        )?;
        if session.write(request.as_bytes()).is_err() {
            return Err(session.why());
        }
        // The request holds a key; it is not kept longer than the write.
        drop(request);
        let deadline = Instant::now() + self.total;
        let mut seen: Vec<u8> = Vec::new();
        loop {
            if let Some(reply) = parse_reply(&seen, false)? {
                return Ok(reply);
            }
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(chunk) => {
                    seen.extend_from_slice(&chunk);
                    if seen.len() > MAX_BYTES + 16 * 1024 {
                        return Err(HueError::Malformed("the answer is too large".to_owned()));
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Err(HueError::Timeout),
                Err(RecvTimeoutError::Disconnected) => {
                    return match parse_reply(&seen, true) {
                        Ok(Some(reply)) => Ok(reply),
                        Ok(None) => Err(session.why()),
                        Err(_) if seen.is_empty() => Err(session.why()),
                        Err(e) => Err(e),
                    };
                }
            }
        }
    }

    /// A request whose answer is JSON: `2xx` only, the bridge's own words for anything else.
    fn json(
        &self,
        method: &str,
        path: &str,
        key: Option<&Secret>,
        body: Option<&str>,
    ) -> Result<(Value, Reply), HueError> {
        let reply = self.call(method, path, key, body)?;
        let value: Option<Value> = serde_json::from_slice(&reply.body).ok();
        if !(200..300).contains(&reply.status) {
            let words = value
                .as_ref()
                .and_then(|v| v.pointer("/errors/0/description"))
                .and_then(Value::as_str)
                .map(clean)
                .unwrap_or_default();
            return Err(HueError::Http(reply.status, words));
        }
        let value =
            value.ok_or_else(|| HueError::Malformed("the answer is not JSON".to_owned()))?;
        Ok((value, reply))
    }

    /// The `data` of a v2 answer, after the bridge's own `errors`.
    fn data(
        &self,
        method: &str,
        path: &str,
        key: &Secret,
        body: Option<&str>,
    ) -> Result<Vec<Value>, HueError> {
        let (value, _) = self.json(method, path, Some(key), body)?;
        if let Some(words) = value
            .pointer("/errors/0/description")
            .and_then(Value::as_str)
        {
            return Err(HueError::Rejected(clean(words)));
        }
        value
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| HueError::Malformed("no data in the answer".to_owned()))
    }

    /// Makes an application key and a client key; the bridge answers only for a minute after its link button is pressed.
    pub fn register(&self) -> Result<(Secret, Secret), HueError> {
        let body = json!({"devicetype": DEVICE_TYPE, "generateclientkey": true}).to_string();
        let (value, _) = self.json("POST", "/api", None, Some(&body))?;
        let first = value
            .as_array()
            .and_then(|a| a.first())
            .ok_or_else(|| HueError::Malformed("not an answer to a registration".to_owned()))?;
        if let Some(error) = first.get("error") {
            return Err(if error.get("type").and_then(Value::as_u64) == Some(101) {
                HueError::LinkButton
            } else {
                HueError::Rejected(clean(
                    error
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("refused"),
                ))
            });
        }
        let success = first
            .get("success")
            .ok_or_else(|| HueError::Malformed("no key in the answer".to_owned()))?;
        let take = |k: &str| {
            success
                .get(k)
                .and_then(Value::as_str)
                .map(|s| Secret::new(s.to_owned()))
                .ok_or_else(|| HueError::Malformed(format!("no {k} in the answer")))
        };
        Ok((take("username")?, take("clientkey")?))
    }

    /// The application id of the key: the identity the DTLS handshake uses.
    pub fn application_id(&self, key: &Secret) -> Result<String, HueError> {
        let reply = self.call("GET", "/auth/v1", Some(key), None)?;
        if !(200..300).contains(&reply.status) {
            return Err(HueError::Http(
                reply.status,
                if reply.status == 403 {
                    "the application key is not accepted".to_owned()
                } else {
                    String::new()
                },
            ));
        }
        reply
            .header("hue-application-id")
            .map(str::trim)
            .filter(|id| huestream::is_uuid(id))
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| HueError::Malformed("no application id in the answer".to_owned()))
    }

    /// Every entertainment area.
    pub fn areas(&self, key: &Secret) -> Result<Vec<Area>, HueError> {
        let data = self.data(
            "GET",
            "/clip/v2/resource/entertainment_configuration",
            key,
            None,
        )?;
        Ok(data.iter().filter_map(area_of).collect())
    }

    /// One area.
    pub fn area(&self, key: &Secret, id: &str) -> Result<Area, HueError> {
        let path = format!("/clip/v2/resource/entertainment_configuration/{id}");
        self.data("GET", &path, key, None)?
            .first()
            .and_then(area_of)
            .ok_or_else(|| HueError::Malformed("no such entertainment area".to_owned()))
    }

    fn act(&self, key: &Secret, request: huestream::Request) -> Result<(), HueError> {
        self.data(request.method, &request.path, key, Some(request.body))
            .map(|_| ())
    }

    /// Asks the bridge to accept a stream for the area. Streaming starts only after this.
    pub fn start(&self, key: &Secret, id: &str) -> Result<(), HueError> {
        self.act(key, huestream::start_request(id))
    }

    /// Ends the stream; the lights go back to the bridge's own control.
    pub fn stop(&self, key: &Secret, id: &str) -> Result<(), HueError> {
        self.act(key, huestream::stop_request(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(text: &str, eof: bool) -> Result<Option<Reply>, HueError> {
        parse_reply(text.as_bytes(), eof)
    }

    #[test]
    fn a_length_framed_answer_is_whole_only_when_all_of_it_is_there() {
        let head = "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nhue-application-id: abc\r\n\r\n";
        assert!(reply(&format!("{head}12"), false).unwrap().is_none());
        assert!(reply(&format!("{head}12"), true).is_err());
        let whole = reply(&format!("{head}12345tail"), false).unwrap().unwrap();
        assert_eq!((whole.status, whole.body.as_slice()), (200, &b"12345"[..]));
        assert_eq!(whole.header("Hue-Application-Id"), Some("abc"));
    }

    #[test]
    fn a_chunked_answer_is_put_together_and_a_cut_one_is_refused() {
        let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        let whole = reply(&format!("{head}3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"), false)
            .unwrap()
            .unwrap();
        assert_eq!(whole.body, b"abcde");
        assert!(
            reply(&format!("{head}3\r\nabc\r\n2\r\nd"), false)
                .unwrap()
                .is_none()
        );
        assert!(reply(&format!("{head}3\r\nabc\r\n2\r\nd"), true).is_err());
        assert!(reply(&format!("{head}zz\r\n"), false).is_err());
    }

    #[test]
    fn an_unframed_answer_ends_with_the_connection_and_junk_is_refused() {
        let head = "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nbody";
        assert!(reply(head, false).unwrap().is_none());
        assert_eq!(reply(head, true).unwrap().unwrap().body, b"body");
        assert!(reply("garbage\r\n\r\n", false).is_err());
        assert!(reply("HTTP/1.1 200 OK\r\nContent-Length: 99999999\r\n\r\n", false).is_err());
    }

    #[test]
    fn an_area_is_read_with_its_channels_in_order_and_the_ones_past_the_stream_dropped() {
        let v = json!({
            "id": "7C6AB1F2-3E44-4A5E-9D1B-0C8F6A2D9E10",
            "metadata": {"name": "TV\u{7}"},
            "configuration_type": "screen",
            "status": "active",
            "active_streamer": {"rid": "0B9D5C1E-3F6A-4D2B-8A11-6E0C2F9B7A44", "rtype": "auth_v1"},
            "channels": [
                {"channel_id": 2, "position": {"x": 0.5, "y": 1.0, "z": -0.25}},
                {"channel_id": 0, "position": {"x": -0.5, "y": 1.0, "z": 0.75}},
                {"channel_id": 20, "position": {"x": 0.0, "y": 0.0, "z": 0.0}},
                {"channel_id": 1}
            ]
        });
        let a = area_of(&v).unwrap();
        assert_eq!(a.id, "7c6ab1f2-3e44-4a5e-9d1b-0c8f6a2d9e10");
        assert_eq!((a.name.as_str(), a.kind.as_str()), ("TV", "screen"));
        assert!(a.active);
        assert_eq!(
            a.streamer.as_deref(),
            Some("0b9d5c1e-3f6a-4d2b-8a11-6e0c2f9b7a44")
        );
        assert_eq!(
            a.channels,
            vec![
                Channel {
                    id: 0,
                    x: -0.5,
                    z: 0.75
                },
                Channel {
                    id: 2,
                    x: 0.5,
                    z: -0.25
                }
            ]
        );
        assert!(area_of(&json!({"name": "no id"})).is_none());
    }
}
