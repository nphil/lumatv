//! Opt-in, bounded diagnostics. The sender child isolates DNS and can be killed on reconfiguration.
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, io::{Read, Write}, process::{Command, Stdio}, sync::{Arc, Condvar, Mutex, atomic::{AtomicBool, Ordering}}, thread, time::{Duration, Instant}};
use std::os::unix::process::CommandExt;
use std::os::fd::AsRawFd;

const QUEUE_LIMIT: usize = 128;
const BATCH_LIMIT: usize = 3072;
const FLUSH: Duration = Duration::from_secs(5);

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config { pub enabled: bool, pub server: String, pub topic: String, pub token: String }
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config").field("enabled", &self.enabled).field("configured", &self.endpoint().is_some()).finish()
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
    Stream { ms: u64, fps: Option<f32>, bitrate_kbps: Option<u32>, rtt_ms: Option<f32>, jitter_ms: Option<f32>, loss_pct: Option<f32>, dropped: Option<u64>, pad_reports: Option<u32>, pad_rate_hz: Option<f32>, pad_p99_us: Option<u32> },
    Frames { ms: u64, fed: u64, accepted: u64, dropped: [u64; 5], omitted: u64 },
    Error { ms: u64, kind: &'static str },
    Test,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status { #[default] Off, NotConfigured, Waiting, Sending, Sent, Error }
struct State { config: Config, generation: u64, queue: VecDeque<String>, dropped: u64, status: Status, running: bool, flush_now: bool }
struct Shared { state: Mutex<State>, wake: Condvar, enabled: AtomicBool }
#[derive(Clone)]
pub struct Diagnostics { shared: Arc<Shared> }
impl Default for Diagnostics {
    fn default() -> Self { Self { shared: Arc::new(Shared { state: Mutex::new(State { config: Config::default(), generation: 0, queue: VecDeque::new(), dropped: 0, status: Status::Off, running: false, flush_now: false }), wake: Condvar::new(), enabled: AtomicBool::new(false) }) } }
}
impl Diagnostics {
    pub fn configure(&self, config: Config) {
        let mut s = self.shared.state.lock().unwrap();
        if s.config == config { return; }
        s.generation = s.generation.wrapping_add(1);
        s.queue.clear(); s.dropped = 0;
        s.flush_now = false;
        s.status = if !config.enabled { Status::Off } else if config.endpoint().is_none() { Status::NotConfigured } else { Status::Waiting };
        self.shared.enabled.store(s.status == Status::Waiting, Ordering::Release);
        s.config = config;
        if s.status == Status::Waiting && !s.running {
            s.running = true;
            let shared = self.shared.clone();
            thread::spawn(move || worker(shared));
        }
        self.shared.wake.notify_all();
    }
    pub fn enabled(&self) -> bool { self.shared.enabled.load(Ordering::Acquire) }
    pub fn status(&self) -> Status { self.shared.state.lock().unwrap().status }
    pub fn emit(&self, event: Event) {
        let mut s = self.shared.state.lock().unwrap();
        if !self.enabled() { return; }
        if s.queue.len() == QUEUE_LIMIT { s.dropped = s.dropped.saturating_add(1); return; }
        if let Ok(line) = serde_json::to_string(&event) { s.queue.push_back(line); self.shared.wake.notify_one(); }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config { Config { enabled: true, server: "https://example.invalid".into(), topic: "test_topic".into(), token: "secret-fixture".into() } }

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
            d.shared.enabled.store(true, Ordering::Release);
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
            d.shared.enabled.store(true, Ordering::Release);
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
            d.shared.enabled.store(true, Ordering::Release);
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
        d.shared.enabled.store(true, Ordering::Release);
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
}
