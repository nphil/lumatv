//! Shared by the integration tests: a raw HTTP client, a fake receiver, a fake `luna-send`, a scripted fetcher and a rig that
//! starts the whole service in a temporary data dir. Nothing here touches the network beyond the loopback.
#![allow(dead_code)]

pub mod bridge;

use fetch::{Fetch, FetchError, FetchLimits};
use lumatv_helper::ai::types::{
    ActionError, AppInfo, ForegroundApp, InputInfo, KeyEvent, KeySession, KeySource, MediaKey,
    ReceiverControl, TvControl,
};
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
/// length, closes), counts the requests, and can be switched to `403` or shut down. It also takes the web port's commands
/// (`MV355`, recorded in `commands` and applied) and, like the set's Denon, takes the CEC volume steps LG's audio service passes on
/// only one per `cec_gap`: [`FakeAvr::cec_step`] is such a step, and one that comes sooner is lost.
pub struct FakeAvr {
    pub port: u16,
    state: Arc<Mutex<AvrState>>,
    hits: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
    commands: Arc<Mutex<Vec<(Instant, String)>>>,
    last_cec: Arc<Mutex<Option<Instant>>>,
    cec_gap: Duration,
    thread: Option<JoinHandle<()>>,
}

/// The volume as the fake writes it in the status, from the panel's 0 to 98 scale.
fn db_text(abs: f64) -> String {
    format!("{:.1}", abs - 80.0)
}

/// The panel level a state's volume text says.
fn abs_of(state: &AvrState) -> f64 {
    state.volume.parse::<f64>().unwrap() + 80.0
}

impl FakeAvr {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(AvrState::default()));
        let hits = Arc::new(AtomicUsize::new(0));
        let stopping = Arc::new(AtomicBool::new(false));
        let commands = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let (state, hits, stopping, commands) = (
                state.clone(),
                hits.clone(),
                stopping.clone(),
                commands.clone(),
            );
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
                    let text = String::from_utf8_lossy(&request).into_owned();
                    if let Some(command) = text
                        .strip_prefix("GET /goform/formiPhoneAppDirect.xml?")
                        .and_then(|rest| rest.split(' ').next())
                    {
                        commands.lock().push((Instant::now(), command.to_owned()));
                        if let Some(digits) = command.strip_prefix("MV") {
                            let abs = match digits.len() {
                                2 => digits.parse::<f64>().ok(),
                                3 => digits.parse::<f64>().ok().map(|v| v / 10.0),
                                _ => None,
                            };
                            if let Some(abs) = abs {
                                state.lock().volume = db_text(abs);
                            }
                        }
                        let _ = stream.write_all(b"HTTP/1.0 200 OK\r\n\r\n");
                        continue;
                    }
                    if !text.starts_with("GET /goform/formMainZone_MainZoneXmlStatusLite.xml ") {
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
            commands,
            last_cec: Arc::new(Mutex::new(None)),
            cec_gap: Duration::from_millis(150),
            thread: Some(thread),
        }
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    /// Every command the receiver was sent over its web port, in order, with when.
    pub fn commands(&self) -> Vec<(Instant, String)> {
        self.commands.lock().clone()
    }

    /// The volume on the panel's 0 to 98 scale.
    pub fn level(&self) -> f64 {
        abs_of(&self.state.lock())
    }

    /// One CEC volume step from the TV (half a step on the panel): taken if the last one was `cec_gap` ago or more, lost otherwise,
    /// as the set's Denon loses the steps of a held key that come faster than it can take them. Returns whether it was taken.
    pub fn cec_step(&self, up: bool) -> bool {
        let mut last = self.last_cec.lock();
        let now = Instant::now();
        if last.is_some_and(|at| now.duration_since(at) < self.cec_gap) {
            return false;
        }
        *last = Some(now);
        let mut state = self.state.lock();
        let abs = (abs_of(&state) + if up { 0.5 } else { -0.5 }).clamp(0.0, 98.0);
        state.volume = db_text(abs);
        true
    }

    /// A CEC step that reaches the receiver `after` from now, whatever came in between: one still in flight when the key was let go.
    pub fn cec_step_after(&self, up: bool, after: Duration) {
        let (state, last) = (self.state.clone(), self.last_cec.clone());
        thread::spawn(move || {
            thread::sleep(after);
            *last.lock() = Some(Instant::now());
            let mut state = state.lock();
            let abs = (abs_of(&state) + if up { 0.5 } else { -0.5 }).clamp(0.0, 98.0);
            state.volume = db_text(abs);
        });
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
            "#!/bin/sh\ncase \"$1 $2\" in\n  \"-n 1\") printf '{{\"returnValue\":true,\"state\":\"%s\"}}\\n' \"$(cat STATEFILE)\" ;;\n  \"-n \"*) case \"$3\" in *getSystemSettings*) if [ -e SOUNDFILE ]; then printf '{{\"returnValue\":true,\"subscribed\":true,\"settings\":{{\"soundOutput\":\"tv_speaker\"}}}}\\n'; while :; do sleep 1; done; fi; {subscribe} ;; *) {subscribe} ;; esac ;;\n  *) exit 1 ;;\nesac\n"
        )
        .replace("STATEFILE", &state_file.display().to_string())
        .replace("PIDFILE", &pid_file.display().to_string())
        .replace("SOUNDFILE", &dir.join("luna-sound-output").display().to_string());
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

    /// From now on the sound-output subscription (`getSystemSettings`) is answered at once as valid, as LG's service does: without it the
    /// receiver waits four seconds for it each time the screen comes on.
    pub fn answer_sound_output(&self) {
        std::fs::write(self.state_file.with_file_name("luna-sound-output"), "").unwrap();
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

// ----------------------------------------------------------------------------------------------- the AI module's fakes

/// A remote whose key presses a test makes by hand: `open()` makes a fresh channel and keeps its sender for `tap`.
#[derive(Default)]
pub struct ScriptedKeys {
    sender: Mutex<Option<std::sync::mpsc::Sender<KeyEvent>>>,
    opens: AtomicUsize,
}

struct ScriptedSession(std::sync::mpsc::Receiver<KeyEvent>);

impl KeySession for ScriptedSession {
    fn next(&mut self, timeout: Duration) -> Option<KeyEvent> {
        self.0.recv_timeout(timeout).ok()
    }
}

impl KeySource for ScriptedKeys {
    fn open(&self) -> Box<dyn KeySession> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.sender.lock() = Some(tx);
        self.opens.fetch_add(1, Ordering::SeqCst);
        Box::new(ScriptedSession(rx))
    }
}

impl ScriptedKeys {
    pub fn opens(&self) -> usize {
        self.opens.load(Ordering::SeqCst)
    }

    fn send(&self, device: &str, code: u16, value: i32) {
        if let Some(tx) = self.sender.lock().as_ref() {
            let _ = tx.send(KeyEvent {
                device: device.to_owned(),
                code,
                value,
                at: Instant::now(),
            });
        }
    }

    /// A person's tap: press, a moment, release.
    pub fn tap(&self, device: &str, code: u16) {
        self.send(device, code, 1);
        thread::sleep(Duration::from_millis(10));
        self.send(device, code, 0);
    }
}

pub const IR_REMOTE: &str = "LGE RCU";
pub const MAGIC_REMOTE: &str = "LGE M-RCU - Builtin [2]";
pub const NETWORK_KEYS: &str = "Smart Remote RCU Input";
pub const KEY_OK: u16 = 28;
pub const KEY_BACK: u16 = 412;

/// A TV that records what it was told to do.
#[derive(Default)]
pub struct FakeTv {
    pub acted: Mutex<Vec<String>>,
}

impl TvControl for FakeTv {
    fn foreground_app(&self) -> Result<ForegroundApp, ActionError> {
        Ok(ForegroundApp {
            app_id: Some("netflix".into()),
            title: Some("Netflix".into()),
        })
    }

    fn launchable_apps(&self) -> Result<Vec<AppInfo>, ActionError> {
        Ok(vec![
            AppInfo {
                id: "netflix".into(),
                title: "Netflix".into(),
            },
            AppInfo {
                id: "youtube.leanback.v4".into(),
                title: "YouTube".into(),
            },
        ])
    }

    fn inputs(&self) -> Result<Vec<InputInfo>, ActionError> {
        Ok(vec![InputInfo {
            id: "HDMI_2".into(),
            label: "AVR-S760H".into(),
            connected: true,
            app_id: Some("com.webos.app.hdmi2".into()),
        }])
    }

    fn launch_app(&self, app_id: &str) -> Result<AppInfo, ActionError> {
        self.acted.lock().push(format!("launch {app_id}"));
        Ok(AppInfo {
            id: app_id.into(),
            title: "Netflix".into(),
        })
    }

    fn switch_input(&self, input_id: &str) -> Result<InputInfo, ActionError> {
        self.acted.lock().push(format!("switch {input_id}"));
        Ok(InputInfo {
            id: input_id.into(),
            label: "AVR-S760H".into(),
            connected: true,
            app_id: None,
        })
    }

    fn standby(&self) -> Result<(), ActionError> {
        self.acted.lock().push("standby".into());
        Ok(())
    }

    fn media_key(&self, key: MediaKey) -> Result<(), ActionError> {
        self.acted.lock().push(format!("key {}", key.id()));
        Ok(())
    }
}

/// A receiver that records what it was told to do.
#[derive(Default)]
pub struct FakeReceiver {
    pub acted: Mutex<Vec<String>>,
}

impl ReceiverControl for FakeReceiver {
    fn set_volume(&self, level: f64) -> Result<(), ActionError> {
        self.acted.lock().push(format!("volume {level}"));
        Ok(())
    }

    fn set_mute(&self, muted: bool) -> Result<(), ActionError> {
        self.acted.lock().push(format!("mute {muted}"));
        Ok(())
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

/// A config with the rhythm of a test: polls in tens of milliseconds, the screen always on, and no telnet link to the receiver
/// (a test that is about it sets `receiver.telnetPort`).
pub fn test_config(overrides: Value) -> Config {
    let mut base = json!({
        "listen": {"port": 0},
        "screen": {"source": "on", "pollMs": 40},
        "receiver": {"pollMs": 30, "fastPollMs": 10, "burstMs": 300, "maxBackoffMs": 60, "hideAfterMs": 300,
                     "connectTimeoutMs": 300, "requestTimeoutMs": 500, "telnetPort": 0},
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

/// The Denon receiver plugin switched on as the owner switches it, pointed at a loopback port.
pub fn receiver_at(paths: &Paths, port: u16) {
    use lumatv_helper::plugins::roster;
    use lumatv_helper::receiver_plugin::{ID, OPT_IN};
    seed_settings(
        paths,
        ID,
        json!({
            "host": {"type": "text", "value": "127.0.0.1"},
            "port": {"type": "integer", "value": port}
        }),
    );
    let dir = paths.plugins_dir();
    let mut file = roster::read_file(&dir).unwrap();
    file.first_party.push(roster::Approved {
        id: ID.to_owned(),
        approve: OPT_IN.approval(),
    });
    roster::write_file(&dir, &file).unwrap();
}

/// Switches the Denon receiver plugin as the TV's Settings does.
pub fn switch_receiver(addr: SocketAddr, on: bool) -> Reply {
    use lumatv_helper::receiver_plugin::{ID, OPT_IN};
    let body = if on {
        json!({"on": true, "approve": OPT_IN.approval()})
    } else {
        json!({"on": false})
    };
    http(
        addr,
        "POST",
        &format!("/v1/plugins/{ID}/switch"),
        &[("X-Luma-Client", "lumatv-ui"), ("Content-Type", "application/json")],
        Some(&body.to_string()),
    )
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
        self.start_with(config, fetcher, None);
    }

    /// Starts with the AI module acting through the given fakes.
    pub fn start_with(
        &mut self,
        config: Config,
        fetcher: Arc<dyn Fetch>,
        ai: Option<lumatv_helper::ai::Deps>,
    ) {
        self.start_full(config, fetcher, ai, None, None);
    }

    /// Starts with a fake Home Assistant on the loopback: voice search may dial it.
    pub fn start_voice(&mut self, config: Config, fetcher: Arc<dyn Fetch>) {
        self.start_full(
            config,
            fetcher,
            None,
            Some(ha_link::AddressPolicy::TESTING),
            None,
        );
    }

    /// Starts with the cameras acting through the given deps (the loopback, a feeder and dial of the test's, quick timing), the Cameras
    /// plugin switched on as the owner switches it.
    pub fn start_cameras(
        &mut self,
        config: Config,
        fetcher: Arc<dyn Fetch>,
        cameras: lumatv_helper::scrypted::Deps,
    ) {
        use lumatv_helper::plugins::roster;
        use lumatv_helper::scrypted::options::OPT_IN;
        let dir = self.paths.plugins_dir();
        let mut file = roster::read_file(&dir).unwrap();
        file.first_party.push(roster::Approved {
            id: OPT_IN.spec.id.to_owned(),
            approve: OPT_IN.approval(),
        });
        roster::write_file(&dir, &file).unwrap();
        self.start_full(config, fetcher, None, None, Some(cameras));
    }

    fn start_full(
        &mut self,
        config: Config,
        fetcher: Arc<dyn Fetch>,
        ai: Option<lumatv_helper::ai::Deps>,
        voice_policy: Option<ha_link::AddressPolicy>,
        cameras: Option<lumatv_helper::scrypted::Deps>,
    ) {
        let service = Service::start(StartOptions {
            paths: self.paths.clone(),
            config,
            fetcher: Some(fetcher),
            ai,
            voice_policy,
            cameras,
            onhome: None,
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
