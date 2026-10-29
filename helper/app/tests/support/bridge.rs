//! A fake LumaShow bridge, version 1.1, for the host tests: HTTPS on the loopback with a throwaway certificate (`tests/fixtures/scrypted`:
//! ECDSA P-256, `CA:FALSE`, `serverAuth`, valid to 2126, the shape the real bridge's has; the key protects nothing), the bearer check against
//! a set of verifiers (the SHA-256 of a token, as the bridge's device list holds them), kept-alive connections only for a client that asks,
//! JSON answers with a length and streams in chunks (as Node writes them), and the calls of design 3.3: health, cameras, live, stills, the push
//! channel, event thumbnails, recordings, events, clips, thumbnails and playback. The test steers it (`push_event`, `set_cameras`,
//! `fail_next`, `kill_streams`) and reads what it was asked (`requests`, `connections`, `stills_open`).

use parking_lot::{Condvar, Mutex};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const CERT: &[u8] = include_bytes!("../fixtures/scrypted/cert.der");
const KEY: &[u8] = include_bytes!("../fixtures/scrypted/key.der");
/// A 32x18 baseline JPEG.
pub const STILL: &[u8] = include_bytes!("../fixtures/scrypted/still.jpg");

/// The pin of the fake's certificate, as the bridge's settings would show it.
pub fn pin() -> String {
    lumatv_helper::scrypted::store::Pin::of_certificate(CERT).hex()
}

/// A token and its verifier, made the way the TV makes them.
pub fn verifier_of(token: &str) -> String {
    lumatv_helper::scrypted::store::Token::verifier_of(token)
}

pub struct Shared {
    verifiers: Mutex<HashSet<String>>,
    cameras: Mutex<Value>,
    features: Mutex<Value>,
    rtsp: Mutex<HashMap<String, String>>,
    docs: Mutex<HashMap<String, (u16, Value)>>,
    failures: Mutex<Vec<(String, u16, String)>>,
    requests: Mutex<Vec<String>>,
    unauthorized: AtomicUsize,
    connections: AtomicUsize,
    stills_open: AtomicUsize,
    stills_total: AtomicUsize,
    events_open: AtomicUsize,
    events_total: AtomicUsize,
    events: Mutex<Vec<(u64, String)>>,
    pushed: Condvar,
    last_event_ids: Mutex<Vec<Option<String>>>,
    stills_queries: Mutex<Vec<String>>,
    skew: AtomicI64,
    ping_ms: AtomicU64,
    kill: AtomicU64,
    stills_fps: AtomicU64,
    stop: AtomicBool,
    /// Connections are closed as they come (a bridge that is down).
    refuse: AtomicBool,
    next_seq: AtomicU64,
    playback_counter: AtomicU64,
    answer_delay_ms: AtomicU64,
    playback_delay_ms: AtomicU64,
}

pub struct FakeBridge {
    pub addr: SocketAddr,
    pub shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

fn camera_doc(id: &str, name: &str, kind: &str) -> Value {
    json!({
        "id": id, "name": name, "room": "Entry", "type": kind, "online": true,
        "features": ["live", "stills", "stream", "recordings", "thumbnails", "playback", "events", "clips"],
        "earliest": 1_792_500_000_000i64,
        "native": {"codec": "h264", "width": 640, "height": 360, "fps": 15},
        "streams": [{"quality": "native", "codec": "h264", "width": 640, "height": 360, "fps": 15, "prebuffered": true},
                    {"quality": "light", "codec": "h264", "width": 320, "height": 180, "fps": 10, "prebuffered": false}]
    })
}

impl FakeBridge {
    /// A bridge that knows the device `verifier`, shares cameras `273` (Front Door, a doorbell) and `274` (Garage), and has a push channel.
    pub fn start(verifier: &str) -> Self {
        let shared = Arc::new(Shared {
            verifiers: Mutex::new(HashSet::from([verifier.to_owned()])),
            cameras: Mutex::new(
                json!({"cameras": [camera_doc("273", "Front Door", "doorbell"), camera_doc("274", "Garage", "camera")]}),
            ),
            features: Mutex::new(
                json!({"cameras": 1, "recordings": 1, "live": 1, "stills": 1, "stream": 1, "event_thumbnails": 1, "devices": 1}),
            ),
            rtsp: Mutex::new(HashMap::new()),
            docs: Mutex::new(HashMap::new()),
            failures: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
            unauthorized: AtomicUsize::new(0),
            connections: AtomicUsize::new(0),
            stills_open: AtomicUsize::new(0),
            stills_total: AtomicUsize::new(0),
            events_open: AtomicUsize::new(0),
            events_total: AtomicUsize::new(0),
            events: Mutex::new(Vec::new()),
            pushed: Condvar::new(),
            last_event_ids: Mutex::new(Vec::new()),
            stills_queries: Mutex::new(Vec::new()),
            skew: AtomicI64::new(0),
            ping_ms: AtomicU64::new(300),
            kill: AtomicU64::new(0),
            stills_fps: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            refuse: AtomicBool::new(false),
            next_seq: AtomicU64::new(4000),
            playback_counter: AtomicU64::new(0),
            answer_delay_ms: AtomicU64::new(0),
            playback_delay_ms: AtomicU64::new(0),
        });
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(CERT.to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY.to_vec())),
            )
            .unwrap();
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let state = shared.clone();
        let thread = thread::Builder::new()
            .name("fake-bridge".into())
            .spawn(move || {
                while !state.stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((tcp, _)) => {
                            if state.refuse.load(Ordering::SeqCst) {
                                drop(tcp);
                                continue;
                            }
                            state.connections.fetch_add(1, Ordering::SeqCst);
                            // a stream that is open when the bridge "restarts" ends, however late its thread gets to run
                            let born = state.kill.load(Ordering::SeqCst);
                            let (state, config) = (state.clone(), config.clone());
                            thread::spawn(move || {
                                let _ = tcp.set_nonblocking(false);
                                let _ = serve(tcp, config, &state, born);
                            });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(_) => break,
                    }
                }
            })
            .unwrap();
        Self {
            addr,
            shared,
            thread: Some(thread),
        }
    }

    /// `127.0.0.1:port`, what `scrypted-link --address` takes.
    pub fn address(&self) -> String {
        self.addr.to_string()
    }

    pub fn add_device(&self, verifier: &str) {
        self.shared.verifiers.lock().insert(verifier.to_owned());
    }

    pub fn remove_device(&self, verifier: &str) {
        self.shared.verifiers.lock().remove(verifier);
    }

    pub fn set_cameras(&self, cameras: Vec<Value>) {
        *self.shared.cameras.lock() = json!({"cameras": cameras});
    }

    pub fn camera(id: &str, name: &str, kind: &str) -> Value {
        camera_doc(id, name, kind)
    }

    pub fn set_features(&self, features: Value) {
        *self.shared.features.lock() = features;
    }

    /// Where `live` and `playback` send the TV to (a fake camera's address).
    pub fn set_rtsp(&self, camera: &str, url: &str) {
        self.shared
            .rtsp
            .lock()
            .insert(camera.to_owned(), url.to_owned());
    }

    /// What a route answers (`recordings`, `events`, `clips`, `thumbnail`, `clip-thumbnail`, `event-thumbnail`), instead of its default.
    pub fn set_doc(&self, route: &str, status: u16, doc: Value) {
        self.shared
            .docs
            .lock()
            .insert(route.to_owned(), (status, doc));
    }

    /// The next call to `call` ("METHOD path", no query) is answered with this error.
    pub fn fail_next(&self, call: &str, status: u16, code: &str) {
        self.shared
            .failures
            .lock()
            .push((call.to_owned(), status, code.to_owned()));
    }

    /// A bridge that is down: connections are closed as they come, and the streams that are open end.
    pub fn refuse(&self, down: bool) {
        self.shared.refuse.store(down, Ordering::SeqCst);
        if down {
            self.kill_streams();
        }
    }

    pub fn set_skew_ms(&self, skew: i64) {
        self.shared.skew.store(skew, Ordering::SeqCst);
    }

    pub fn set_ping_ms(&self, ms: u64) {
        self.shared.ping_ms.store(ms, Ordering::SeqCst);
    }

    /// Every call waits this long before it is answered (a slow bridge).
    /// Only the calls that start a playback wait (a bridge that takes its time to open a recording; the stop calls are quick).
    pub fn set_playback_delay_ms(&self, ms: u64) {
        self.shared.playback_delay_ms.store(ms, Ordering::SeqCst);
    }

    pub fn set_answer_delay_ms(&self, ms: u64) {
        self.shared.answer_delay_ms.store(ms, Ordering::SeqCst);
    }

    /// Overrides the stills' rate (pictures a second) whatever the client asked.
    pub fn set_stills_fps(&self, fps: u64) {
        self.shared.stills_fps.store(fps, Ordering::SeqCst);
    }

    /// Pushes an event to every open push channel (and keeps it for a reconnect): `data` is the bridge's event object. Returns its sequence.
    pub fn push_event(&self, data: Value) -> u64 {
        let seq = self.shared.next_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.shared.events.lock().push((seq, data.to_string()));
        self.shared.pushed.notify_all();
        seq
    }

    /// An event as the detectors say it, for camera `camera`, with the bridge's clock as its time.
    pub fn event(&self, camera: &str, kind: &str, phase: &str) -> Value {
        let n = self.shared.next_seq.load(Ordering::SeqCst) + 1;
        json!({"id": format!("e{n}"), "camera": camera, "name": if camera == "273" { "Front Door" } else { "Garage" }, "kind": kind,
               "label": kind, "classes": [kind], "score": 0.92, "at": self.now() , "phase": phase, "thumbnail": true, "recording": true})
    }

    /// The bridge's own clock.
    pub fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            + self.shared.skew.load(Ordering::SeqCst)
    }

    /// Forgets every event the bridge kept (a bridge that restarted: a client that resumes is told to start again).
    pub fn clear_events(&self) {
        self.shared.events.lock().clear();
    }

    /// Ends every open push channel and stills stream (the bridge restarted).
    pub fn kill_streams(&self) {
        self.shared.kill.fetch_add(1, Ordering::SeqCst);
        self.shared.pushed.notify_all();
    }

    pub fn requests(&self) -> Vec<String> {
        self.shared.requests.lock().clone()
    }

    pub fn requests_matching(&self, prefix: &str) -> usize {
        self.shared
            .requests
            .lock()
            .iter()
            .filter(|r| r.starts_with(prefix))
            .count()
    }

    pub fn unauthorized(&self) -> usize {
        self.shared.unauthorized.load(Ordering::SeqCst)
    }

    pub fn connections(&self) -> usize {
        self.shared.connections.load(Ordering::SeqCst)
    }

    pub fn stills_open(&self) -> usize {
        self.shared.stills_open.load(Ordering::SeqCst)
    }

    pub fn stills_total(&self) -> usize {
        self.shared.stills_total.load(Ordering::SeqCst)
    }

    pub fn stills_queries(&self) -> Vec<String> {
        self.shared.stills_queries.lock().clone()
    }

    pub fn events_open(&self) -> usize {
        self.shared.events_open.load(Ordering::SeqCst)
    }

    pub fn events_total(&self) -> usize {
        self.shared.events_total.load(Ordering::SeqCst)
    }

    /// The `Last-Event-ID` of each push channel opened, in order.
    pub fn last_event_ids(&self) -> Vec<Option<String>> {
        self.shared.last_event_ids.lock().clone()
    }

    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.pushed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for FakeBridge {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---- the server ---------------------------------------------------------------------------------------------------------------------

type Conn = StreamOwned<ServerConnection, TcpStream>;

struct Req {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_request(reader: &mut BufReader<Conn>) -> std::io::Result<Option<Req>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.trim_end().splitn(3, ' ');
    let (method, target) = (
        parts.next().unwrap_or("").to_owned(),
        parts.next().unwrap_or("").to_owned(),
    );
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let query = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (k.to_owned(), decode(v))
        })
        .collect();
    let mut headers = HashMap::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let text = line.trim_end();
        if text.is_empty() {
            break;
        }
        if let Some((k, v)) = text.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    Ok(Some(Req {
        method,
        path: path.to_owned(),
        query,
        headers,
        body,
    }))
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        429 => "Too Many Requests",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        _ => "Error",
    }
}

fn write_answer(
    out: &mut Conn,
    status: u16,
    content_type: &str,
    body: &[u8],
    keep: bool,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: {}\r\n{}\r\n",
        status_text(status),
        body.len(),
        if keep { "keep-alive" } else { "close" },
        if keep {
            "Keep-Alive: timeout=30\r\n"
        } else {
            ""
        },
    );
    out.write_all(head.as_bytes())?;
    out.write_all(body)?;
    out.flush()
}

fn write_json(out: &mut Conn, status: u16, value: &Value, keep: bool) -> std::io::Result<()> {
    write_answer(
        out,
        status,
        "application/json",
        value.to_string().as_bytes(),
        keep,
    )
}

fn write_error(out: &mut Conn, status: u16, code: &str, keep: bool) -> std::io::Result<()> {
    write_json(
        out,
        status,
        &json!({"error": {"code": code, "message": "fake"}}),
        keep,
    )
}

/// A piece of a streaming answer: the bridge's streams have no length and are not chunked, the body runs until the connection closes.
fn chunk(out: &mut Conn, data: &[u8]) -> std::io::Result<()> {
    out.write_all(data)?;
    out.flush()
}

fn serve(
    tcp: TcpStream,
    config: Arc<ServerConfig>,
    state: &Arc<Shared>,
    born: u64,
) -> std::io::Result<()> {
    let _ = tcp.set_nodelay(true);
    let connection = ServerConnection::new(config).map_err(std::io::Error::other)?;
    let mut reader = BufReader::new(StreamOwned::new(connection, tcp));
    loop {
        let Some(req) = read_request(&mut reader)? else {
            return Ok(());
        };
        let keep = req
            .headers
            .get("connection")
            .is_some_and(|c| c.eq_ignore_ascii_case("keep-alive"));
        let out = reader.get_mut();
        let key = format!("{} {}", req.method, req.path);
        let full = match req.query.is_empty() {
            true => key.clone(),
            false => format!("{key}?{}", {
                let mut pairs: Vec<String> =
                    req.query.iter().map(|(k, v)| format!("{k}={v}")).collect();
                pairs.sort();
                pairs.join("&")
            }),
        };
        state.requests.lock().push(full);
        let delay = state.answer_delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            thread::sleep(Duration::from_millis(delay));
        }
        if key == "POST /v1/playback" {
            let delay = state.playback_delay_ms.load(Ordering::SeqCst);
            if delay > 0 {
                thread::sleep(Duration::from_millis(delay));
            }
        }
        let token = req
            .headers
            .get("authorization")
            .and_then(|a| a.strip_prefix("Bearer "))
            .map(str::to_owned);
        let good = token.is_some_and(|t| state.verifiers.lock().contains(&verifier_of(&t)));
        if !good {
            state.unauthorized.fetch_add(1, Ordering::SeqCst);
            write_error(out, 401, "unauthorized", false)?;
            return Ok(());
        }
        // an error the test asked for
        let failure = {
            let mut failures = state.failures.lock();
            failures
                .iter()
                .position(|(call, ..)| key == *call)
                .map(|at| failures.remove(at))
        };
        if let Some((_, status, code)) = failure {
            write_error(out, status, &code, keep)?;
            if !keep {
                return Ok(());
            }
            continue;
        }
        match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/v1/events/stream") => return stream_events(out, state, &req, born),
            ("GET", "/v1/stills") => return stream_stills(out, state, &req, born),
            _ => {}
        }
        answer(out, state, &req, keep)?;
        if !keep {
            return Ok(());
        }
    }
}

fn answer(out: &mut Conn, state: &Arc<Shared>, req: &Req, keep: bool) -> std::io::Result<()> {
    let route = req.path.trim_start_matches("/v1/");
    let canned = state.docs.lock().get(route).cloned();
    if let Some((status, doc)) = canned {
        return if route.contains("thumbnail") && status == 200 {
            write_answer(out, 200, "image/jpeg", STILL, keep)
        } else if status >= 400 {
            write_error(
                out,
                status,
                doc["code"].as_str().unwrap_or("not_found"),
                keep,
            )
        } else {
            write_json(out, status, &doc, keep)
        };
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        + state.skew.load(Ordering::SeqCst);
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/v1/health") => {
            let cameras = state.cameras.lock()["cameras"]
                .as_array()
                .map_or(0, Vec::len);
            write_json(
                out,
                200,
                &json!({"ok": true, "api": 1, "bridge": "1.1.0", "now": now, "cameras": cameras, "device": "LumaTV",
                        "features": state.features.lock().clone(), "limits": {"playback_sessions": 2, "playback_rates": [1, 2, 4]}}),
                keep,
            )
        }
        ("GET", "/v1/cameras") => {
            let mut doc = state.cameras.lock().clone();
            // without scope=all the bridge answers as v1 does: no live, stills or stream features
            if req.query.get("scope").map(String::as_str) != Some("all") {
                if let Some(list) = doc["cameras"].as_array_mut() {
                    for camera in list {
                        camera["features"] =
                            json!(["recordings", "thumbnails", "playback", "events", "clips"]);
                    }
                }
            }
            write_json(out, 200, &doc, keep)
        }
        ("POST", "/v1/live") => {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let camera = body["camera"].as_str().unwrap_or("").to_owned();
            let Some(url) = state.rtsp.lock().get(&camera).cloned() else {
                return write_error(out, 404, "camera_not_found", keep);
            };
            write_json(
                out,
                200,
                &json!({"rtsp": url, "camera": camera, "quality": body["quality"], "video": {"codec": "h264", "width": 640, "height": 360, "fps": 15},
                        "audio": null, "prebuffered": true, "expires_in_ms": 60000, "accept_seen": body["accept"]}),
                keep,
            )
        }
        ("POST", "/v1/playback") | ("POST", "/v1/clip-playback") => {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let camera = body["camera"].as_str().unwrap_or("").to_owned();
            let url = {
                let rtsp = state.rtsp.lock();
                rtsp.get(&format!("{camera}#playback"))
                    .or(rtsp.get(&camera))
                    .cloned()
            };
            let Some(url) = url else {
                return write_error(out, 404, "camera_not_found", keep);
            };
            let n = state.playback_counter.fetch_add(1, Ordering::SeqCst) + 1;
            write_json(
                out,
                200,
                &json!({"session": format!("s{n}"), "rtsp": url, "state": "ready", "camera": camera, "started_at": body["start"].as_i64().unwrap_or(1_792_605_903_000i64),
                        "rate": body["rate"].as_u64().unwrap_or(1), "mode": "copy", "video": {"codec": "h264", "width": 640, "height": 360}, "expires_in_ms": 1_800_000}),
                keep,
            )
        }
        ("POST", "/v1/playback/stop") => write_json(out, 200, &json!({"ok": true}), keep),
        ("GET", "/v1/still") | ("GET", "/v1/thumbnail") | ("GET", "/v1/clip-thumbnail") => {
            write_answer(out, 200, "image/jpeg", STILL, keep)
        }
        ("GET", "/v1/event-thumbnail") => {
            let id = req.query.get("id").cloned().unwrap_or_default();
            if id.starts_with("e") {
                write_answer(out, 200, "image/jpeg", STILL, keep)
            } else {
                write_error(out, 404, "not_found", keep)
            }
        }
        ("GET", "/v1/recordings") => {
            let (from, to) = (
                req.query
                    .get("from")
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(0),
                req.query
                    .get("to")
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(0),
            );
            write_json(
                out,
                200,
                &json!({"camera": req.query.get("camera"), "from": from, "to": to, "step_ms": 60000, "complete": true, "probed": 2,
                        "ranges": [{"from": from, "to": to}], "unknown": []}),
                keep,
            )
        }
        ("GET", "/v1/events") => write_json(
            out,
            200,
            &json!({"camera": req.query.get("camera"), "truncated": false, "events": [{"at": now - 1000, "kind": "person", "label": "person", "classes": ["person"]}]}),
            keep,
        ),
        ("GET", "/v1/clips") => write_json(
            out,
            200,
            &json!({"camera": req.query.get("camera"), "truncated": false,
                    "clips": [{"id": "c1.WyIx", "at": now - 60000, "duration": 30000, "kind": "person", "label": "person", "description": "Person", "has_thumbnail": true, "has_video": true}]}),
            keep,
        ),
        _ => write_error(out, 404, "not_found", keep),
    }
}

/// `GET /v1/events/stream`: hello, a replay when asked to resume, then what is pushed; a ping when nothing is.
fn stream_events(
    out: &mut Conn,
    state: &Arc<Shared>,
    req: &Req,
    killed: u64,
) -> std::io::Result<()> {
    state.events_open.fetch_add(1, Ordering::SeqCst);
    state.events_total.fetch_add(1, Ordering::SeqCst);
    state
        .last_event_ids
        .lock()
        .push(req.headers.get("last-event-id").cloned());
    let result = (|| {
        out.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n")?;
        let resume = req
            .headers
            .get("last-event-id")
            .and_then(|v| v.parse::<u64>().ok());
        // an id the bridge does not keep any more is said first: the client starts from now
        let (reset, mut sent) = {
            let events = state.events.lock();
            match resume {
                None => (false, events.last().map_or(0, |(s, _)| *s)),
                Some(id) if events.iter().any(|(s, _)| *s == id) => (false, id),
                Some(_) => (true, events.last().map_or(0, |(s, _)| *s)),
            }
        };
        if reset {
            chunk(out, b"event: reset\ndata: {\"seq\": 0}\n\n")?;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            + state.skew.load(Ordering::SeqCst);
        chunk(
            out,
            format!("event: hello\ndata: {{\"now\": {now}, \"seq\": {sent}}}\n\n").as_bytes(),
        )?;
        loop {
            if state.stop.load(Ordering::SeqCst) || state.kill.load(Ordering::SeqCst) != killed {
                return Ok(());
            }
            let fresh: Vec<(u64, String)> = {
                let mut events = state.events.lock();
                let pending: Vec<_> = events.iter().filter(|(s, _)| *s > sent).cloned().collect();
                if pending.is_empty() {
                    let wait = Duration::from_millis(state.ping_ms.load(Ordering::SeqCst));
                    if state.pushed.wait_for(&mut events, wait).timed_out() {
                        drop(events);
                        chunk(out, b": ping\n\n")?;
                    }
                    continue;
                }
                pending
            };
            for (seq, data) in fresh {
                chunk(
                    out,
                    format!("id: {seq}\nevent: event\ndata: {data}\n\n").as_bytes(),
                )?;
                sent = seq;
            }
        }
    })();
    state.events_open.fetch_sub(1, Ordering::SeqCst);
    result
}

/// `GET /v1/stills`: a multipart stream of the still, each part a little different so that a test can tell which came last.
fn stream_stills(
    out: &mut Conn,
    state: &Arc<Shared>,
    req: &Req,
    killed: u64,
) -> std::io::Result<()> {
    state.stills_open.fetch_add(1, Ordering::SeqCst);
    state.stills_total.fetch_add(1, Ordering::SeqCst);
    state.stills_queries.lock().push(format!(
        "camera={}&w={}&fps={}",
        req.query.get("camera").cloned().unwrap_or_default(),
        req.query.get("w").cloned().unwrap_or_default(),
        req.query.get("fps").cloned().unwrap_or_default()
    ));
    let result = (|| {
        out.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=lumashow\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n")?;
        let asked = req
            .query
            .get("fps")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(5)
            .clamp(1, 10);
        let fps = match state.stills_fps.load(Ordering::SeqCst) {
            0 => asked,
            n => n,
        };
        let mut n = 0u32;
        let started = Instant::now();
        loop {
            if state.stop.load(Ordering::SeqCst) || state.kill.load(Ordering::SeqCst) != killed {
                return Ok(());
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
                + state.skew.load(Ordering::SeqCst);
            // after the end-of-image marker, which a decoder does not read: the picture's number
            let mut body = STILL.to_vec();
            body.extend_from_slice(format!("<{n}>").as_bytes());
            let mut part = format!("--lumashow\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nX-Captured-At: {now}\r\n\r\n", body.len()).into_bytes();
            part.extend_from_slice(&body);
            part.extend_from_slice(b"\r\n");
            chunk(out, &part)?;
            n += 1;
            let due = started + Duration::from_millis(1000 * u64::from(n) / fps);
            thread::sleep(due.saturating_duration_since(Instant::now()));
        }
    })();
    state.stills_open.fetch_sub(1, Ordering::SeqCst);
    result
}
