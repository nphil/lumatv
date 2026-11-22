//! Opt-in, bounded diagnostics. The sender child isolates DNS and can be killed on reconfiguration.
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, io::{Read, Write}, process::{Command, Stdio}, sync::{Arc, Condvar, Mutex, atomic::{AtomicU8, Ordering}}, thread, time::{Duration, Instant}};
use std::os::unix::process::CommandExt;
use std::os::fd::AsRawFd;

pub(crate) const QUEUE_LIMIT: usize = 128;
pub(crate) const BATCH_LIMIT: usize = 3072;
const FLUSH: Duration = Duration::from_secs(5);

/// What is sent. Cumulative: each level also sends everything at the levels before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level { Error, Warn, #[default] Info, Debug, Trace }
impl Level {
    pub const ALL: [Level; 5] = [Level::Error, Level::Warn, Level::Info, Level::Debug, Level::Trace];
    pub fn as_str(self) -> &'static str { match self { Level::Error => "error", Level::Warn => "warn", Level::Info => "info", Level::Debug => "debug", Level::Trace => "trace" } }
    pub fn index(self) -> usize { self as usize }
}
impl Serialize for Level {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> { serializer.serialize_str(self.as_str()) }
}
impl<'de> Deserialize<'de> for Level {
    /// Any other word, or another type, reads as the default so a file written by a newer build still loads.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value.as_str().and_then(|word| Self::ALL.into_iter().find(|level| level.as_str() == word)).unwrap_or_default())
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config { pub enabled: bool, pub level: Level, pub server: String, pub topic: String, pub token: String }
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config").field("enabled", &self.enabled).field("level", &self.level).field("configured", &self.endpoint().is_some()).finish()
    }
}

impl Config {
    pub fn active(&self) -> bool { self.enabled && self.endpoint().is_some() }
    fn endpoint(&self) -> Option<String> {
        if self.server.len() > 1024 { return None; }
        let url = url::Url::parse(self.server.trim()).ok()?;
        if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() { return None; }
        let topic = self.topic.trim();
        if topic.is_empty() || topic.len() > 64 || !topic.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) { return None; }
        if self.token.len() > 1024 || self.token.bytes().any(|b| b.is_ascii_control()) { return None; }
        Some(format!("{}/{}", url.as_str().trim_end_matches('/'), topic))
    }
}

/// Event fields are fixed by the application, never raw errors, URLs, names or key reports.
#[derive(Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Lifecycle { ms: u64, page: &'static str, foreground: bool, phase: &'static str, visible: bool, sequence: u64 },
    Draw { ms: u64, sequence: u64, frame: u64 },
    Swap { ms: u64, sequence: u64, frame: u64, success: bool },
    Render { ms: u64, frames: u32, max_cpu_us: u32, max_draw_us: u32 },
    Stream { ms: u64, fps: Option<f32>, bitrate_kbps: Option<u32>, rtt_ms: Option<f32>, jitter_ms: Option<f32>, loss_pct: Option<f32>, dropped: Option<u64>, #[serde(skip_serializing_if = "Option::is_none")] socket_drops: Option<u64>, pad_reports: Option<u32>, pad_rate_hz: Option<f32>, pad_p99_us: Option<u32> },
    Frames { ms: u64, fed: u64, accepted: u64, dropped: [u64; 5], omitted: u64 },
    Error { ms: u64, kind: &'static str },
    Test,
    /// First line after the level changes or the app starts, so a reader knows what a missing event means.
    #[serde(rename = "level")]
    Setting { ms: u64, level: Level },
    Session { ms: u64, session: u64, engine: &'static str, phase: &'static str, after_ms: u64, #[serde(skip_serializing_if = "Option::is_none")] queue: Option<u32>, #[serde(skip_serializing_if = "Option::is_none")] eta_s: Option<u32> },
    StreamFormat { ms: u64, session: u64, codec: &'static str, width: u32, height: u32, fps: u32, bit_depth: u8, hdr: bool, want_hdr: bool, sdr: &'static str, audio: u8, want_fps: u32, want_height: u32 },
    SessionEnd { ms: u64, session: u64, engine: &'static str, outcome: &'static str, reason: &'static str, by: &'static str, #[serde(skip_serializing_if = "Option::is_none")] ran_s: Option<u32>, #[serde(skip_serializing_if = "Option::is_none")] code: Option<String>, #[serde(skip)] level: Level },
    Display { ms: u64, at: &'static str, width: u32, height: u32, hdr_type: u8, #[serde(skip_serializing_if = "Option::is_none")] muted: Option<bool>, #[serde(skip_serializing_if = "Option::is_none")] hz: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] src_hz: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] vrr: Option<bool>, #[serde(skip_serializing_if = "Option::is_none")] low_delay: Option<bool> },
    /// The first reading with a picture: the video window sized and not muted, after the feeder took a frame. Times run from the session running and from that frame.
    FirstPicture { ms: u64, session: u64, after_running_ms: u64, after_frame_ms: u64 },
    /// No such reading five seconds after the session ran: the video window as it then stood, and the rates the panel and the engine report.
    NoPicture { ms: u64, session: u64, #[serde(skip_serializing_if = "Option::is_none")] muted: Option<bool>, width: u32, height: u32, #[serde(skip_serializing_if = "Option::is_none")] src_hz: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] fps: Option<f32> },
    HdrMissing { ms: u64, session: u64, engine: &'static str, why: &'static str },
    /// Dropped frames and stalls since the last report. `socket_drops` is the packets the TV's network stack discarded in the same time, left out while zero.
    FrameTrouble { ms: u64, dropped: [u64; 5], stalls: u32, max_gap_us: u64, #[serde(skip_serializing_if = "is_zero")] socket_drops: u32 },
    NetLoss { ms: u64, loss_pct: f32, #[serde(skip_serializing_if = "Option::is_none")] rtt_ms: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] jitter_ms: Option<f32> },
    EngineError { ms: u64, engine: &'static str, code: &'static str },
    Controller { ms: u64, slot: u8, connected: bool },
    /// Pacing over the last two seconds; `late` and `stalls` are left out while zero. `pacing_ms` and the host and arrival gaps
    /// (p50, p95) are GFN's playout pacer's latest second.
    Detail { ms: u64, fps: f32, gap_us: [u64; 4], feed_us: [u64; 4], queue: u32, #[serde(skip_serializing_if = "is_zero")] late: u32, #[serde(skip_serializing_if = "is_zero")] stalls: u32, #[serde(skip_serializing_if = "Option::is_none")] client_ms: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] host_ms: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] pacing_ms: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] host_gap_ms: Option<[f32; 2]>, #[serde(skip_serializing_if = "Option::is_none")] arrival_gap_ms: Option<[f32; 2]>, #[serde(skip_serializing_if = "Option::is_none")] pad_bursts: Option<u32>, #[serde(skip_serializing_if = "Option::is_none")] pad_gap_us: Option<u32>, #[serde(skip_serializing_if = "Option::is_none")] hz: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] min_hz: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] fewest: Option<u32> },
    Hist { ms: u64, span_ms: u32, gap: [u32; 7], feed: [u32; 8], queue: [u32; 5] },
    Trip { ms: u64, #[serde(skip_serializing_if = "Option::is_none")] presses: Option<u32>, #[serde(skip_serializing_if = "Option::is_none")] press_p50_ms: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] press_p99_ms: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] network_ms: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] host_ms: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] arrival_p50_ms: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] arrival_p95_ms: Option<f32>, #[serde(skip_serializing_if = "Option::is_none")] client_ms: Option<f32> },
}
fn is_zero(n: &u32) -> bool { *n == 0 }
impl Event {
    /// The lowest level that sends the event.
    pub fn level(&self) -> Level {
        match self {
            Event::Error { .. } | Event::Test | Event::Setting { .. } => Level::Error,
            Event::HdrMissing { .. } | Event::FrameTrouble { .. } | Event::NetLoss { .. } | Event::EngineError { .. } | Event::NoPicture { .. } => Level::Warn,
            Event::Controller { connected, .. } => if *connected { Level::Info } else { Level::Warn },
            Event::SessionEnd { level, .. } => *level,
            Event::Lifecycle { .. } | Event::Draw { .. } | Event::Swap { .. } | Event::Render { .. } | Event::Stream { .. } | Event::Frames { .. } | Event::Session { .. } | Event::StreamFormat { .. } | Event::Display { .. } | Event::FirstPicture { .. } => Level::Info,
            Event::Detail { .. } => Level::Debug,
            Event::Hist { .. } | Event::Trip { .. } => Level::Trace,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status { #[default] Off, NotConfigured, Waiting, Sending, Sent, Error }
struct State { config: Config, generation: u64, queue: VecDeque<String>, dropped: u64, status: Status, running: bool, flush_now: bool }
/// `gate` is 0 while off, else the level's index + 1, so hot paths can skip building an event without taking the lock.
struct Shared { state: Mutex<State>, wake: Condvar, gate: AtomicU8 }
#[derive(Clone)]
pub struct Diagnostics { shared: Arc<Shared> }
impl Default for Diagnostics {
    fn default() -> Self { Self { shared: Arc::new(Shared { state: Mutex::new(State { config: Config::default(), generation: 0, queue: VecDeque::new(), dropped: 0, status: Status::Off, running: false, flush_now: false }), wake: Condvar::new(), gate: AtomicU8::new(0) }) } }
}
impl Diagnostics {
    pub fn configure(&self, config: Config) {
        let mut s = self.shared.state.lock().unwrap();
        if s.config == config { return; }
        s.generation = s.generation.wrapping_add(1);
        s.queue.clear(); s.dropped = 0;
        s.flush_now = false;
        s.status = if !config.enabled { Status::Off } else if config.endpoint().is_none() { Status::NotConfigured } else { Status::Waiting };
        self.shared.gate.store(if s.status == Status::Waiting { config.level as u8 + 1 } else { 0 }, Ordering::Release);
        s.config = config;
        if s.status == Status::Waiting && !s.running {
            s.running = true;
            let shared = self.shared.clone();
            thread::spawn(move || worker(shared));
        }
        self.shared.wake.notify_all();
    }
    pub fn enabled(&self) -> bool { self.shared.gate.load(Ordering::Acquire) != 0 }
    /// The level being sent, `None` while diagnostics are off.
    pub fn level(&self) -> Option<Level> { Level::ALL.get(usize::from(self.shared.gate.load(Ordering::Acquire)).checked_sub(1)?).copied() }
    /// Whether an event at `level` would be queued, so callers skip the work of building it.
    pub fn wants(&self, level: Level) -> bool { self.level().is_some_and(|on| level <= on) }
    pub fn status(&self) -> Status { self.shared.state.lock().unwrap().status }
    #[cfg(test)]
    pub(crate) fn open_for_test(level: Level) -> Self {
        let d = Self::default();
        {
            let mut s = d.shared.state.lock().unwrap();
            s.config = Config { enabled: true, level, ..Config::default() };
            s.running = true;
        }
        d.shared.gate.store(level as u8 + 1, Ordering::Release);
        d
    }
    #[cfg(test)]
    pub(crate) fn queued(&self) -> (Vec<String>, u64) {
        let s = self.shared.state.lock().unwrap();
        (s.queue.iter().cloned().collect(), s.dropped)
    }
    pub fn emit(&self, event: Event) {
        let level = event.level();
        let mut s = self.shared.state.lock().unwrap();
        if !self.wants(level) { return; }
        if s.queue.len() == QUEUE_LIMIT { s.dropped = s.dropped.saturating_add(1); return; }
        let Ok(line) = serde_json::to_string(&event) else { return };
        // Per-second detail yields once a batch's worth is waiting, so it never delays the failures and phases queued behind it. Trace yields first,
        // at three quarters, so the finest lines never take the room Debug's per-second lines need.
        let room = if level >= Level::Trace { BATCH_LIMIT / 4 * 3 } else { BATCH_LIMIT };
        if level >= Level::Debug && s.queue.iter().map(|queued| queued.len() + 1).sum::<usize>() + line.len() + 1 > room { s.dropped = s.dropped.saturating_add(1); return; }
        s.queue.push_back(line);
        self.shared.wake.notify_one();
    }
    /// Best effort at a fatal exit only; false includes timeout, failure and reconfiguration.
    pub fn flush_fatal(&self, ms: u64, timeout: Duration) -> bool {
        let mut s = self.shared.state.lock().unwrap();
        if !self.enabled() { return false; }
        let fatal = serde_json::to_string(&Event::Error { ms, kind: "runtime_failed" }).unwrap();
        let mut bytes = s.queue.iter().map(|line| line.len() + 1).sum::<usize>() + fatal.len() + 1;
        while s.queue.len() >= QUEUE_LIMIT || bytes > BATCH_LIMIT - 64 {
            let Some(oldest) = s.queue.pop_front() else { break };
            bytes -= oldest.len() + 1;
            s.dropped = s.dropped.saturating_add(1);
        }
        s.queue.push_back(fatal);
        s.flush_now = true;
        let generation = s.generation;
        let deadline = Instant::now() + timeout;
        self.shared.wake.notify_all();
        while s.generation == generation && (s.flush_now || s.status == Status::Sending) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { break; }
            s = self.shared.wake.wait_timeout(s, remaining).unwrap().0;
        }
        let delivered = s.generation == generation && !s.flush_now && s.status == Status::Sent;
        s.flush_now = false;
        delivered
    }
}
#[derive(Serialize, Deserialize)]
struct SendRequest { endpoint: String, token: String, body: String }
fn worker(shared: Arc<Shared>) {
    worker_with(shared, FLUSH, send_child);
}
fn worker_with(shared: Arc<Shared>, flush: Duration, send: impl Fn(&Shared, u64, &SendRequest) -> u8) {
    let mut s = shared.state.lock().unwrap();
    loop {
        if !s.config.enabled || s.config.endpoint().is_none() { s.running = false; return; }
        if s.queue.is_empty() { s = shared.wake.wait(s).unwrap(); continue; }
        let generation = s.generation;
        let deadline = Instant::now() + flush;
        while s.generation == generation && !s.flush_now && Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            s = shared.wake.wait_timeout(s, remaining).unwrap().0;
        }
        if s.generation != generation { continue; }
        let mut body = String::new();
        if s.dropped > 0 { body.push_str(&format!("{{\"event\":\"dropped\",\"count\":{}}}\n", s.dropped)); s.dropped = 0; }
        while let Some(line) = s.queue.front() {
            if body.len() + line.len() + 1 > BATCH_LIMIT { break; }
            body.push_str(line); body.push('\n'); s.queue.pop_front();
        }
        s.flush_now = false;
        let request = SendRequest { endpoint: s.config.endpoint().unwrap(), token: s.config.token.clone(), body };
        s.status = Status::Sending;
        drop(s);
        let result = send(&shared, generation, &request);
        s = shared.state.lock().unwrap();
        if s.generation == generation {
            s.status = if result == 0 { Status::Sent } else { Status::Error };
            shared.wake.notify_all();
            if result != 0 {
                let backoff = Duration::from_secs(if result >= 5 { u64::from(result) } else { 30 });
                let until = Instant::now() + backoff;
                while s.generation == generation && Instant::now() < until {
                    s = shared.wake.wait_timeout(s, until.saturating_duration_since(Instant::now())).unwrap().0;
                }
            }
        }
    }
}
fn send_child(shared: &Shared, generation: u64, request: &SendRequest) -> u8 {
    let Ok(payload) = serde_json::to_vec(request) else { return 1 };
    if payload.len() >= 8192 { return 1; }
    let Ok(exe) = std::env::current_exe() else { return 1 };
    let mut command = Command::new(&exe);
    if exe.file_name().is_some_and(|name| name.to_string_lossy().starts_with("ld-")) {
        let (Some(lib), Some(app)) = (exe.parent(), std::env::args_os().next()) else { return 1 };
        command.arg("--library-path").arg(lib).arg(app);
    }
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 { return Err(std::io::Error::last_os_error()); }
            if libc::getppid() == 1 { return Err(std::io::Error::other("parent exited")); }
            Ok(())
        });
    }
    let Ok(mut child) = command.arg("--diagnostic-send").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return 1 };
    let Some(mut input) = child.stdin.take() else { let _ = child.kill(); let _ = child.wait(); return 1 };
    let flags = unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(input.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        let _ = child.kill(); let _ = child.wait(); return 1;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut written = 0;
    let wrote = loop {
        let guard = shared.state.lock().unwrap();
        if guard.generation != generation || Instant::now() >= deadline { break false; }
        if written == payload.len() { drop(input); break true; }
        match input.write(&payload[written..]) {
            Ok(0) => break false,
            Ok(n) => written += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => { drop(shared.wake.wait_timeout(guard, Duration::from_millis(20)).unwrap()); continue; }
            Err(_) => break false,
        }
    };
    wait_child(shared, generation, child, wrote, Duration::from_secs(10))
}
fn wait_child(shared: &Shared, generation: u64, mut child: std::process::Child, wrote: bool, timeout: Duration) -> u8 {
    let started = Instant::now();
    loop {
        let s = shared.state.lock().unwrap();
        if s.generation != generation || !wrote || started.elapsed() >= timeout {
            drop(s); let _ = child.kill(); let _ = child.wait(); return 1;
        }
        drop(s);
        match child.try_wait() { Ok(Some(status)) => return status.code().unwrap_or(1).clamp(0, 255) as u8, Err(_) => { let _ = child.kill(); let _ = child.wait(); return 1; }, Ok(None) => {} }
        let s = shared.state.lock().unwrap();
        drop(shared.wake.wait_timeout(s, Duration::from_millis(50)).unwrap());
    }
}
fn outgoing(agent: &ureq::Agent, request: &SendRequest) -> ureq::Request {
    let mut outgoing = agent.post(&request.endpoint).set("Content-Type", "text/plain; charset=utf-8");
    if !request.token.is_empty() { outgoing = outgoing.set("Authorization", &format!("Bearer {}", request.token)); }
    outgoing
}
fn post(agent: &ureq::Agent, request: &SendRequest) -> i32 {
    match outgoing(agent, request).send_string(&request.body) {
        Ok(response) if (200..300).contains(&response.status()) => 0,
        Err(ureq::Error::Status(429, response)) => response.header("Retry-After").and_then(|v| v.parse::<i32>().ok()).unwrap_or(60).clamp(5, 250),
        _ => 1,
    }
}
/// Invoked before application startup in the isolated child; input and errors never reach logs.
pub fn sender_main() -> i32 {
    // The parent may own inherited compositor descriptors without close-on-exec.
    let closed = unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) } == 0;
    if !closed {
        if let Ok(entries) = std::fs::read_dir("/proc/self/fd") {
            let descriptors: Vec<i32> = entries.filter_map(Result::ok).filter_map(|entry| entry.file_name().to_str()?.parse().ok()).filter(|fd| *fd >= 3).collect();
            for fd in descriptors { unsafe { libc::close(fd); } }
        } else {
            let max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) }.max(3);
            for fd in 3..max.min(i32::MAX as libc::c_long) { unsafe { libc::close(fd as i32); } }
        }
    }
    let mut bytes = Vec::new();
    if std::io::stdin().take(8192).read_to_end(&mut bytes).is_err() { return 1; }
    let Ok(request) = serde_json::from_slice::<SendRequest>(&bytes) else { return 1 };
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(8)).redirects(0).user_agent("LumaGame/0.1").build();
    post(&agent, &request)
}

/// The keys an event may carry, and the shape of any word: a short lower-case token, so no URL, name, token or error text can appear.
#[cfg(test)]
pub(crate) fn assert_allowlisted(line: &str) {
    const KEYS: &[&str] = &[
        "event", "ms", "page", "foreground", "phase", "visible", "sequence", "frame", "success", "frames", "max_cpu_us", "max_draw_us",
        "fps", "bitrate_kbps", "rtt_ms", "jitter_ms", "loss_pct", "dropped", "pad_reports", "pad_rate_hz", "pad_p99_us", "fed", "accepted", "omitted",
        "kind", "level", "session", "engine", "after_ms", "queue", "eta_s", "codec", "width", "height", "bit_depth", "hdr", "want_hdr", "sdr", "audio",
        "want_fps", "want_height", "outcome", "reason", "by", "ran_s", "at", "hdr_type", "hz", "src_hz", "vrr", "low_delay", "why", "stalls", "max_gap_us",
        "code", "slot", "connected", "gap_us", "feed_us", "late", "client_ms", "host_ms", "pacing_ms", "host_gap_ms", "arrival_gap_ms", "pad_bursts", "pad_gap_us",
        "min_hz", "fewest", "span_ms",
        "gap", "feed", "presses", "press_p50_ms", "press_p99_ms", "network_ms", "arrival_p50_ms", "arrival_p95_ms", "muted", "after_running_ms", "after_frame_ms", "socket_drops",
    ];
    let value: serde_json::Value = serde_json::from_str(line).unwrap();
    for (key, field) in value.as_object().unwrap() {
        assert!(KEYS.contains(&key.as_str()), "{key} in {line}");
        match field {
            serde_json::Value::String(word) => assert!(word.len() <= 24 && word.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-')), "{word} in {line}"),
            serde_json::Value::Array(items) => assert!(items.iter().all(serde_json::Value::is_number), "{key} in {line}"),
            other => assert!(other.is_number() || other.is_boolean() || other.is_null(), "{key} in {line}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config { Config { enabled: true, level: Level::Info, server: "https://example.invalid".into(), topic: "test_topic".into(), token: "secret-fixture".into() } }
    fn open(d: &Diagnostics, level: Level) { d.shared.gate.store(level as u8 + 1, Ordering::Release); }

    /// One of every event, with the level it is documented at.
    fn samples() -> Vec<(Event, Level)> {
        vec![
            (Event::Lifecycle { ms: 1, page: "running", foreground: true, phase: "running", visible: false, sequence: 4 }, Level::Info),
            (Event::Draw { ms: 1, sequence: 4, frame: 9 }, Level::Info),
            (Event::Swap { ms: 1, sequence: 4, frame: 9, success: true }, Level::Info),
            (Event::Render { ms: 1, frames: 60, max_cpu_us: 900, max_draw_us: 400 }, Level::Info),
            (Event::Stream { ms: 1, fps: Some(59.9), bitrate_kbps: Some(40_000), rtt_ms: Some(12.5), jitter_ms: Some(1.5), loss_pct: Some(0.0), dropped: Some(0), socket_drops: Some(0), pad_reports: Some(120), pad_rate_hz: Some(125.0), pad_p99_us: Some(9_000) }, Level::Info),
            (Event::Frames { ms: 1, fed: 60, accepted: 60, dropped: [0; 5], omitted: 0 }, Level::Info),
            (Event::Error { ms: 1, kind: "swap_failed" }, Level::Error),
            (Event::Test, Level::Error),
            (Event::Setting { ms: 1, level: Level::Debug }, Level::Error),
            (Event::Session { ms: 1, session: 1, engine: "gfn", phase: "queue", after_ms: 900, queue: Some(3), eta_s: Some(40) }, Level::Info),
            (Event::StreamFormat { ms: 1, session: 1, codec: "h265", width: 3840, height: 2160, fps: 60, bit_depth: 10, hdr: true, want_hdr: true, sdr: "none", audio: 6, want_fps: 60, want_height: 2160 }, Level::Info),
            (Event::SessionEnd { ms: 1, session: 1, engine: "gfn", outcome: "failed", reason: "video", by: "engine", ran_s: Some(3), code: None, level: Level::Error }, Level::Error),
            (Event::SessionEnd { ms: 1, session: 1, engine: "gfn", outcome: "ended", reason: "remote", by: "engine", ran_s: Some(300), code: None, level: Level::Warn }, Level::Warn),
            (Event::SessionEnd { ms: 1, session: 1, engine: "gfn", outcome: "ended", reason: "stopped", by: "owner", ran_s: Some(300), code: None, level: Level::Info }, Level::Info),
            (Event::Display { ms: 1, at: "start", width: 3840, height: 2160, hdr_type: 2, muted: Some(false), hz: Some(59.9), src_hz: Some(60.0), vrr: Some(false), low_delay: Some(true) }, Level::Info),
            (Event::HdrMissing { ms: 1, session: 1, engine: "gfn", why: "game_unsupported" }, Level::Warn),
            (Event::FrameTrouble { ms: 1, dropped: [0, 0, 0, 7, 0], stalls: 1, max_gap_us: 140_000, socket_drops: 12 }, Level::Warn),
            (Event::NetLoss { ms: 1, loss_pct: 2.5, rtt_ms: Some(30.0), jitter_ms: Some(4.0) }, Level::Warn),
            (Event::EngineError { ms: 1, engine: "gfn", code: "protocol" }, Level::Warn),
            (Event::Controller { ms: 1, slot: 0, connected: false }, Level::Warn),
            (Event::Controller { ms: 1, slot: 0, connected: true }, Level::Info),
            (Event::Detail { ms: 1, fps: 59.9, gap_us: [16_700, 17_000, 18_000, 33_000], feed_us: [300, 600, 900, 1_500], queue: 2, late: 1, stalls: 0, client_ms: Some(1.2), host_ms: Some(3.4), pacing_ms: Some(2.4), host_gap_ms: Some([16.6, 17.1]), arrival_gap_ms: Some([8.2, 25.0]), pad_bursts: Some(0), pad_gap_us: Some(0), hz: Some(59.9), min_hz: Some(59.8), fewest: Some(5) }, Level::Debug),
            (Event::Hist { ms: 1, span_ms: 6000, gap: [0, 0, 118, 2, 0, 0, 0], feed: [100, 16, 2, 1, 0, 0, 0, 0], queue: [100, 16, 2, 1, 0] }, Level::Trace),
            (Event::Trip { ms: 1, presses: Some(14), press_p50_ms: Some(0.4), press_p99_ms: Some(1.9), network_ms: Some(21.5), host_ms: None, arrival_p50_ms: None, arrival_p95_ms: None, client_ms: None }, Level::Trace),
            (Event::FirstPicture { ms: 1, session: 1, after_running_ms: 1_800, after_frame_ms: 1_300 }, Level::Info),
            (Event::NoPicture { ms: 1, session: 1, muted: Some(true), width: 3840, height: 2160, src_hz: Some(60.0), fps: Some(59.9) }, Level::Warn),
        ]
    }

    #[test]
    fn configuration_and_request_keep_auth_private() {
        let c = config();
        let request = SendRequest { endpoint: c.endpoint().unwrap(), token: c.token.clone(), body: serde_json::to_string(&Event::Test).unwrap() };
        let prepared = outgoing(&ureq::Agent::new(), &request);
        assert_eq!(prepared.method(), "POST");
        assert_eq!(prepared.header("Authorization"), Some("Bearer secret-fixture"));
        assert!(!request.body.contains(&c.token));
        assert!(!request.body.contains(&c.topic));
        assert!(!format!("{c:?}").contains(&c.token));
        let no_auth = outgoing(&ureq::Agent::new(), &SendRequest { token: String::new(), ..request });
        assert!(no_auth.header("Authorization").is_none());
        let mut invalid = c;
        invalid.server = "http://example.invalid".into();
        assert!(invalid.endpoint().is_none());
        invalid.server = "https://user:password@example.invalid".into();
        assert!(invalid.endpoint().is_none());
    }

    #[test]
    fn post_sends_only_the_body_to_local_test_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 1024];
            while !bytes.ends_with(b"{\"event\":\"test\"}") {
                let n = socket.read(&mut chunk).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let request = SendRequest { endpoint: format!("http://{address}/private"), token: "fixture-token".into(), body: "{\"event\":\"test\"}".into() };
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(2)).redirects(0).build();
        outgoing(&agent, &request).send_string(&request.body).unwrap();
        let received = server.join().unwrap();
        let (headers, body) = received.split_once("\r\n\r\n").unwrap();
        assert!(headers.to_ascii_lowercase().contains("authorization: bearer fixture-token"));
        assert_eq!(body, request.body);
    }

    #[test]
    fn redirect_is_a_send_error_not_success() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0; 1024];
            let _ = socket.read(&mut buf).unwrap();
            socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Length: 0\r\n\r\n").unwrap();
        });
        let d = Diagnostics::default();
        {
            let mut state = d.shared.state.lock().unwrap();
            state.config = config();
            state.running = true;
            open(&d, Level::Info);
        }
        let shared = d.shared.clone();
        let waiter = thread::spawn(move || worker_with(shared, Duration::ZERO, |_, _, request| {
            let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(2)).redirects(0).build();
            post(&agent, &SendRequest { endpoint: format!("http://{address}/topic"), token: String::new(), body: request.body.clone() }) as u8
        }));
        d.emit(Event::Test);
        let mut state = d.shared.state.lock().unwrap();
        while state.status != Status::Error { state = d.shared.wake.wait(state).unwrap(); }
        drop(state);
        let mut off = config();
        off.enabled = false;
        d.configure(off);
        waiter.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn fatal_flush_sends_latest_error_without_waiting_for_batch_deadline() {
        let d = Diagnostics::default();
        {
            let mut state = d.shared.state.lock().unwrap();
            state.config = config();
            state.running = true;
            open(&d, Level::Info);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = d.shared.clone();
        let waiter = thread::spawn(move || worker_with(shared, Duration::from_secs(60), |_, _, request| {
            tx.send(request.body.clone()).unwrap();
            0
        }));
        d.emit(Event::Error { ms: 4, kind: "swap_failed" });
        assert!(d.flush_fatal(5, Duration::from_secs(2)));
        let body = rx.recv().unwrap();
        assert!(body.contains("swap_failed"));
        assert!(body.contains("runtime_failed"));
        let mut off = config();
        off.enabled = false;
        d.configure(off);
        waiter.join().unwrap();
        assert!(!d.flush_fatal(6, Duration::ZERO));
    }

    #[test]
    fn queue_is_bounded_and_reconfiguration_discards_it() {
        let d = Diagnostics::default();
        {
            let mut state = d.shared.state.lock().unwrap();
            state.config = config();
            state.running = true;
            open(&d, Level::Info);
        }
        for _ in 0..QUEUE_LIMIT + 3 { d.emit(Event::Test); }
        {
            let state = d.shared.state.lock().unwrap();
            assert_eq!(state.queue.len(), QUEUE_LIMIT);
            assert_eq!(state.dropped, 3);
        }
        let mut replacement = config();
        replacement.topic = "different".into();
        d.configure(replacement.clone());
        assert!(d.shared.state.lock().unwrap().queue.is_empty());
        d.emit(Event::Test);
        replacement.enabled = false;
        d.configure(replacement);
        let state = d.shared.state.lock().unwrap();
        assert!(state.queue.is_empty());
        assert_eq!(state.status, Status::Off);
        assert!(!d.enabled());
    }

    #[test]
    fn disabling_kills_and_reaps_a_blocked_child() {
        let d = Diagnostics::default();
        {
            let mut state = d.shared.state.lock().unwrap();
            state.config = config();
            state.running = true;
        }
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let shared = d.shared.clone();
        let waiter = thread::spawn(move || worker_with(shared, Duration::ZERO, |shared, generation, _| {
            let mut child = Command::new("sh").args(["-c", "printf r; exec cat >/dev/null"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
            let mut ready = [0];
            child.stdout.take().unwrap().read_exact(&mut ready).unwrap();
            assert_eq!(ready, *b"r");
            let stdin = child.stdin.take().unwrap();
            ready_tx.send(()).unwrap();
            let result = wait_child(shared, generation, child, true, Duration::from_secs(10));
            drop(stdin);
            result
        }));
        open(&d, Level::Info);
        d.emit(Event::Test);
        ready_rx.recv().unwrap();
        let mut off = config();
        off.enabled = false;
        d.configure(off);
        waiter.join().unwrap();
        let state = d.shared.state.lock().unwrap();
        assert!(!state.running);
        assert!(state.queue.is_empty());
        assert_eq!(state.status, Status::Off);
    }

    #[test]
    fn sender_timeout_kills_a_blocked_child() {
        let d = Diagnostics::default();
        let mut child = Command::new("cat").stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        let stdin = child.stdin.take().unwrap();
        assert_eq!(wait_child(&d.shared, 0, child, true, Duration::ZERO), 1);
        drop(stdin);
    }

    #[test]
    fn the_level_defaults_to_info_and_older_files_still_load() {
        assert_eq!(Config::default().level, Level::Info);
        let old: Config = serde_json::from_str(r#"{"enabled":true,"server":"https://example.invalid","topic":"t","token":"k"}"#).unwrap();
        assert_eq!((old.enabled, old.level, old.token.as_str()), (true, Level::Info, "k"));
        for level in Level::ALL {
            let saved = serde_json::to_string(&Config { level, ..config() }).unwrap();
            assert_eq!(serde_json::from_str::<Config>(&saved).unwrap().level, level);
        }
        for odd in [r#""verbose""#, "3", "null", r#"["trace"]"#, "{}"] {
            let c: Config = serde_json::from_str(&format!(r#"{{"enabled":true,"level":{odd},"token":"k"}}"#)).unwrap();
            assert_eq!((c.level, c.token.as_str()), (Level::Info, "k"), "{odd}");
        }
    }

    #[test]
    fn events_carry_their_documented_level_and_stay_within_the_allowlist() {
        for (event, level) in samples() {
            let line = serde_json::to_string(&event).unwrap();
            assert_eq!(event.level(), level, "{line}");
            assert_allowlisted(&line);
        }
    }

    #[test]
    fn a_level_queues_its_own_events_and_those_below() {
        let ladder: Vec<Event> = Level::ALL.iter().map(|level| samples().into_iter().find(|(_, at)| at == level).unwrap().0).collect();
        for (n, level) in Level::ALL.into_iter().enumerate() {
            let d = Diagnostics::open_for_test(level);
            for event in &ladder { d.emit(event.clone()); }
            let (lines, dropped) = d.queued();
            let wanted: Vec<String> = ladder[..=n].iter().map(|event| serde_json::to_string(event).unwrap()).collect();
            assert_eq!(lines, wanted, "{level:?}");
            assert_eq!(dropped, 0);
        }
    }

    #[test]
    fn changing_the_level_discards_the_queue_and_applies_at_once() {
        let d = Diagnostics::open_for_test(Level::Info);
        d.emit(Event::Test);
        d.configure(Config { level: Level::Trace, ..config() });
        assert!(d.queued().0.is_empty());
        assert_eq!(d.level(), Some(Level::Trace));
        d.configure(Config { level: Level::Warn, ..config() });
        assert!(d.wants(Level::Warn) && !d.wants(Level::Info));
        d.configure(Config { enabled: false, ..config() });
        assert_eq!(d.level(), None);
        assert!(!d.wants(Level::Error));
    }

    #[test]
    fn per_second_detail_yields_to_a_full_batch_but_failures_do_not() {
        let d = Diagnostics::open_for_test(Level::Trace);
        let frames = Event::Frames { ms: 1, fed: 60, accepted: 60, dropped: [0; 5], omitted: 0 };
        while d.queued().0.iter().map(|line| line.len() + 1).sum::<usize>() <= BATCH_LIMIT { d.emit(frames.clone()); }
        let (before, dropped) = d.queued();
        let fine: Vec<Event> = samples().into_iter().filter(|(_, level)| *level >= Level::Debug).map(|(event, _)| event).collect();
        for event in &fine { d.emit(event.clone()); }
        let (after, shed) = d.queued();
        assert_eq!(after, before);
        assert_eq!(shed, dropped + fine.len() as u64);
        d.emit(Event::Error { ms: 2, kind: "swap_failed" });
        assert_eq!(d.queued().0.len(), before.len() + 1);
    }

    #[test]
    fn trace_yields_to_debug_before_the_batch_is_full() {
        let d = Diagnostics::open_for_test(Level::Trace);
        let frames = Event::Frames { ms: 1, fed: 60, accepted: 60, dropped: [0; 5], omitted: 0 };
        let waiting_bytes = |d: &Diagnostics| d.queued().0.iter().map(|line| line.len() + 1).sum::<usize>();
        while waiting_bytes(&d) < BATCH_LIMIT / 4 * 3 { d.emit(frames.clone()); }
        let waiting = d.queued().0.len();
        let of = |level: Level| samples().into_iter().find(|(_, at)| *at == level).unwrap().0;
        d.emit(of(Level::Trace));
        assert_eq!(d.queued().0.len(), waiting);
        d.emit(of(Level::Debug));
        assert_eq!(d.queued().0.len(), waiting + 1);
    }

    #[test]
    fn trace_never_grows_the_queue_past_a_batch() {
        let d = Diagnostics::open_for_test(Level::Trace);
        let fine: Vec<Event> = samples().into_iter().filter(|(_, level)| *level >= Level::Debug).map(|(event, _)| event).collect();
        for _ in 0..1000 { for event in &fine { d.emit(event.clone()); } }
        let (lines, dropped) = d.queued();
        assert!(lines.iter().map(|line| line.len() + 1).sum::<usize>() <= BATCH_LIMIT);
        assert!(lines.len() <= QUEUE_LIMIT);
        assert_eq!(lines.len() as u64 + dropped, 1000 * fine.len() as u64);
    }
}
