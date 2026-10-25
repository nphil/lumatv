//! Shared by the integration tests: a raw HTTP client, a fake receiver, a fake `luna-send`, a scripted fetcher and a rig that
//! starts the whole service in a temporary data dir. Nothing here touches the network beyond the loopback.
#![allow(dead_code)]

use fetch::{Fetch, FetchError, FetchLimits};
use lumatv_helper::config::{Config, Paths};
use lumatv_helper::service::{Service, StartOptions};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const OPEN_METEO: &str = include_str!("../../../feeds/tests/fixtures/open-meteo.json");
pub const ZIPPOPOTAM: &str = include_str!("../../../feeds/tests/fixtures/zippopotam-30311.json");
pub const BBC_WORLD: &str = include_str!("../../../feeds/tests/fixtures/bbc-world.xml");
pub const BBC_US: &str = include_str!("../../../feeds/tests/fixtures/bbc-us.xml");
pub const NYT_WORLD: &str = include_str!("../../../feeds/tests/fixtures/nyt-world.xml");
pub const NYT_US: &str = include_str!("../../../feeds/tests/fixtures/nyt-us.xml");

pub const FORECAST_ATLANTA: &str = "https://api.open-meteo.com/v1/forecast?latitude=33.723&longitude=-84.4702&current=temperature_2m,apparent_temperature,weather_code,is_day,relative_humidity_2m,wind_speed_10m&daily=weather_code,temperature_2m_max,temperature_2m_min&temperature_unit=fahrenheit&wind_speed_unit=mph&timezone=auto&forecast_days=7";
pub const BBC_WORLD_URL: &str = "https://feeds.bbci.co.uk/news/world/rss.xml";
pub const BBC_US_URL: &str = "https://feeds.bbci.co.uk/news/world/us_and_canada/rss.xml";
pub const NYT_WORLD_URL: &str = "https://rss.nytimes.com/services/xml/rss/nyt/World.xml";
pub const NYT_US_URL: &str = "https://rss.nytimes.com/services/xml/rss/nyt/US.xml";

/// Runs a script that was just written until the system lets it run: another test thread's fork can hold the script's write
/// descriptor for a moment, and exec of a file that is open for writing fails with "text file busy". Once one exec has worked
/// no later one can fail that way.
pub fn settle(program: &Path) {
    for _ in 0..300 {
        let result = std::process::Command::new(program)
            .arg("probe")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match result {
            Err(e) if e.raw_os_error() == Some(26) => thread::sleep(Duration::from_millis(10)),
            _ => return,
        }
    }
}

/// Polls `condition` every few milliseconds until it holds or `timeout` passes.
pub fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

// ----------------------------------------------------------------------------------------------- raw HTTP client

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Sends raw bytes and reads the whole answer (the server closes the connection).
pub fn raw(addr: SocketAddr, request: &[u8]) -> Reply {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(40)))
        .unwrap();
    stream.write_all(request).unwrap();
    let mut data = Vec::new();
    let _ = stream.read_to_end(&mut data);
    parse_reply(&data)
}

pub fn parse_reply(data: &[u8]) -> Reply {
    let end = data
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no end of headers in {:?}", String::from_utf8_lossy(data)));
    let head = String::from_utf8_lossy(&data[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|s| s.parse().ok())
        .expect("status line");
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
        .collect();
    Reply {
        status,
        headers,
        body: data[end + 4..].to_vec(),
    }
}

/// One request with the headers a QML client sends.
pub fn http(
    addr: SocketAddr,
    method: &str,
    target: &str,
    extra: &[(&str, &str)],
    body: Option<&str>,
) -> Reply {
    let mut request = format!("{method} {target} HTTP/1.1\r\nHost: {addr}\r\n");
    for (name, value) in extra {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(body) = body {
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    if let Some(body) = body {
        request.push_str(body);
    }
    raw(addr, request.as_bytes())
}

pub fn get(addr: SocketAddr, target: &str) -> Reply {
    http(addr, "GET", target, &[], None)
}

pub fn post(addr: SocketAddr, target: &str, body: &str) -> Reply {
    http(
        addr,
        "POST",
        target,
        &[
            ("X-Luma-Client", "test"),
            ("Content-Type", "application/json"),
        ],
        Some(body),
    )
}

// ----------------------------------------------------------------------------------------------- fake receiver

/// What the fake receiver says.
#[derive(Clone)]
pub struct AvrState {
    pub power: String,
    pub input: String,
    pub volume: String,
    pub mute: String,
    /// Reply `403` instead of the XML.
    pub forbidden: bool,
}

impl Default for AvrState {
    fn default() -> Self {
        Self {
            power: "ON".into(),
            input: "TV".into(),
            volume: "-44.5".into(),
            mute: "off".into(),
            forbidden: false,
        }
    }
}

/// A receiver on the loopback: answers every connection with the Lite status as the set's receiver writes it (HTTP/1.0, no
/// length, closes), counts the requests, and can be switched to `403` or shut down.
pub struct FakeAvr {
    pub port: u16,
    state: Arc<Mutex<AvrState>>,
    hits: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeAvr {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(AvrState::default()));
        let hits = Arc::new(AtomicUsize::new(0));
        let stopping = Arc::new(AtomicBool::new(false));
        let thread = {
            let (state, hits, stopping) = (state.clone(), hits.clone(), stopping.clone());
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stopping.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(mut stream) = stream else { continue };
                    let mut buf = [0u8; 1024];
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut request = Vec::new();
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    if !String::from_utf8_lossy(&request)
                        .starts_with("GET /goform/formMainZone_MainZoneXmlStatusLite.xml ")
                    {
                        continue;
                    }
                    hits.fetch_add(1, Ordering::SeqCst);
                    let s = state.lock().clone();
                    let reply = if s.forbidden {
                        "HTTP/1.0 403 Forbidden\r\n\r\nError 403: Forbidden\r\nAccess Forbidden\r\n"
                            .to_owned()
                    } else {
                        format!(
                            "HTTP/1.0 200 OK\r\nServer: mongoose2.5\r\nPragma: no-cache\r\nCache-control: no-cache\r\nContent-Type: text/xml; charset=utf-8\r\n\r\n<?xml version=\"1.0\" encoding=\"utf-8\" ?>\n<item>\n<Power><value>{}</value></Power>\n<InputFuncSelect><value>{}</value></InputFuncSelect>\n<VolumeDisplay><value>Absolute</value></VolumeDisplay>\n<MasterVolume><value>{}</value></MasterVolume>\n<Mute><value>{}</value></Mute>\n</item>\n",
                            s.power, s.input, s.volume, s.mute
                        )
                    };
                    let _ = stream.write_all(reply.as_bytes());
                }
            })
        };
        Self {
            port,
            state,
            hits,
            stopping,
            thread: Some(thread),
        }
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub fn set(&self, change: impl FnOnce(&mut AvrState)) {
        change(&mut self.state.lock());
    }

    /// Closes the listener: later connections are refused.
    pub fn shut_down(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for FakeAvr {
    fn drop(&mut self) {
        self.shut_down();
    }
}

// ----------------------------------------------------------------------------------------------- fake luna-send

/// A `luna-send` that answers the power-state call from a file, as a one-shot read and (optionally) as a subscription.
pub struct FakeLuna {
    pub program: PathBuf,
    state_file: PathBuf,
    pub pid_file: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LunaMode {
    /// `-n 1` answers; a subscription (`-n 1000000`) exits at once without a word (a luna-send that cannot subscribe).
    PollOnly,
    /// `-n 1` answers; a subscription prints the state now and again whenever the file changes.
    Subscribes,
    /// `-n 1` answers; a subscription stays silent forever (a luna-send that buffers its output on a pipe).
    SilentSubscription,
}

impl FakeLuna {
    pub fn new(dir: &Path, mode: LunaMode, initial: &str) -> Self {
        let state_file = dir.join("luna-state");
        let pid_file = dir.join("luna-pid");
        std::fs::write(&state_file, initial).unwrap();
        let program = dir.join("luna-send");
        let subscribe = match mode {
            LunaMode::PollOnly => "exit 0".to_owned(),
            LunaMode::SilentSubscription => "echo $$ > PIDFILE; while :; do sleep 1; done".to_owned(),
            LunaMode::Subscribes => "echo $$ > PIDFILE; last=''; while :; do now=$(cat STATEFILE); if [ \"$now\" != \"$last\" ]; then printf '{\"returnValue\":true,\"state\":\"%s\",\"subscribed\":true}\\n' \"$now\"; last=$now; fi; sleep 0.02; done".to_owned(),
        };
        let script = format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n  \"-n 1\") printf '{{\"returnValue\":true,\"state\":\"%s\"}}\\n' \"$(cat STATEFILE)\" ;;\n  \"-n \"*) {subscribe} ;;\n  *) exit 1 ;;\nesac\n"
        )
        .replace("STATEFILE", &state_file.display().to_string())
        .replace("PIDFILE", &pid_file.display().to_string());
        std::fs::write(&program, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        settle(&program);
        Self {
            program,
            state_file,
            pid_file,
        }
    }

    /// Sets what the TV says its power state is.
    pub fn set_state(&self, state: &str) {
        std::fs::write(&self.state_file, state).unwrap();
    }
}

// ----------------------------------------------------------------------------------------------- scripted fetcher

type Answer = (String, Result<Vec<u8>, FetchError>);

/// A `Fetch` that answers by exact URL and records every URL asked for.
pub struct Script {
    answers: Mutex<Vec<Answer>>,
    log: Mutex<Vec<String>>,
}

impl Script {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(Vec::new()),
            log: Mutex::new(Vec::new()),
        })
    }

    /// The standard reachable internet: Atlanta's geocoding and forecast and the four feeds.
    pub fn internet() -> Arc<Self> {
        let script = Self::new();
        script.answer(
            "https://api.zippopotam.us/us/30311",
            Ok(ZIPPOPOTAM.as_bytes().to_vec()),
        );
        script.answer(FORECAST_ATLANTA, Ok(OPEN_METEO.as_bytes().to_vec()));
        script.answer(BBC_WORLD_URL, Ok(BBC_WORLD.as_bytes().to_vec()));
        script.answer(BBC_US_URL, Ok(BBC_US.as_bytes().to_vec()));
        script.answer(NYT_WORLD_URL, Ok(NYT_WORLD.as_bytes().to_vec()));
        script.answer(NYT_US_URL, Ok(NYT_US.as_bytes().to_vec()));
        script
    }

    /// Replaces any earlier answer for `url`.
    pub fn answer(&self, url: &str, answer: Result<Vec<u8>, FetchError>) {
        let mut answers = self.answers.lock();
        answers.retain(|(u, _)| u != url);
        answers.push((url.to_owned(), answer));
    }

    /// Every URL answered by `error` from now on, whatever it is.
    pub fn everything_fails(&self, error: FetchError) {
        let mut answers = self.answers.lock();
        for (_, answer) in answers.iter_mut() {
            *answer = Err(error.clone());
        }
    }

    pub fn requests(&self) -> Vec<String> {
        self.log.lock().clone()
    }
}

impl Fetch for Script {
    fn get(&self, url: &str, _limits: FetchLimits) -> Result<Vec<u8>, FetchError> {
        self.log.lock().push(url.to_owned());
        self.answers
            .lock()
            .iter()
            .find(|(u, _)| u == url)
            .map_or_else(
                || Err(FetchError::Failed(format!("no script for {url}"))),
                |(_, a)| a.clone(),
            )
    }
}

// ----------------------------------------------------------------------------------------------- the rig

/// The whole service in a temporary data dir, on a loopback port the system chose.
pub struct Rig {
    pub dir: tempfile::TempDir,
    pub paths: Paths,
    pub service: Option<Service>,
    pub addr: SocketAddr,
}

/// A config with the rhythm of a test: polls in tens of milliseconds, the screen always on.
pub fn test_config(overrides: Value) -> Config {
    let mut base = json!({
        "listen": {"port": 0},
        "screen": {"source": "on", "pollMs": 40},
        "receiver": {"pollMs": 30, "fastPollMs": 10, "burstMs": 300, "maxBackoffMs": 60, "hideAfterMs": 300,
                     "connectTimeoutMs": 300, "requestTimeoutMs": 500},
        "nice": 0
    });
    merge(&mut base, &overrides);
    Config::parse(&base.to_string()).unwrap()
}

fn merge(into: &mut Value, from: &Value) {
    match (into, from) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                merge(a.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (slot, value) => *slot = value.clone(),
    }
}

/// Writes a settings owner's stored values before the service starts.
pub fn seed_settings(paths: &Paths, owner: &str, values: Value) {
    let dir = paths.settings_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let doc = json!({"owner": owner, "schemaRevision": 1, "version": 1, "values": values});
    std::fs::write(dir.join(format!("{owner}.json")), doc.to_string()).unwrap();
}

/// Stored values that switch the receiver off, so a test that is not about it never polls anything.
pub fn receiver_off(paths: &Paths) {
    seed_settings(
        paths,
        "base.receiver",
        json!({"enabled": {"type": "bool", "value": false}}),
    );
}

/// Stored values that point the receiver client at a loopback port.
pub fn receiver_at(paths: &Paths, port: u16) {
    seed_settings(
        paths,
        "base.receiver",
        json!({
            "enabled": {"type": "bool", "value": true},
            "host": {"type": "text", "value": "127.0.0.1"},
            "port": {"type": "integer", "value": port}
        }),
    );
}

impl Rig {
    /// Prepares a data dir. Call `start` after seeding what the test needs on disk.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        Self {
            dir,
            paths,
            service: None,
            addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        }
    }

    pub fn start(&mut self, config: Config, fetcher: Arc<dyn Fetch>) {
        let service = Service::start(StartOptions {
            paths: self.paths.clone(),
            config,
            fetcher: Some(fetcher),
        })
        .unwrap();
        self.addr = service.addr();
        self.service = Some(service);
    }

    pub fn service(&self) -> &Service {
        self.service.as_ref().expect("started")
    }

    /// A topic's current value through the API.
    pub fn topic(&self, name: &str) -> Option<Value> {
        let reply = get(self.addr, &format!("/v1/state/{name}"));
        (reply.status == 200).then(|| reply.json()["value"].clone())
    }

    /// Waits until the topic satisfies `check`; returns its value then (or the last one seen).
    pub fn wait_topic(
        &self,
        name: &str,
        timeout: Duration,
        check: impl Fn(&Value) -> bool,
    ) -> Option<Value> {
        let mut last = None;
        let ok = wait_until(timeout, || {
            last = self.topic(name);
            last.as_ref().is_some_and(&check)
        });
        if ok { last } else { None }
    }

    pub fn stop(&mut self) {
        if let Some(service) = self.service.take() {
            service.stop();
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.stop();
    }
}
