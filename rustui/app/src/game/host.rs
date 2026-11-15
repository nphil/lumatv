//! Nonblocking engine pipes share the window's poll loop; only enabled engines get a process.
use super::{native::{self, Native}, store::private_dir};
use crate::lifecycle::{FromApp, ToApp};
use lumagame_core::engine::{Engine, Manifest};
use lumagame_core::ids::EngineId;
use lumagame_core::ipc::{self, Display, FromEngine, Hello, LineBuffer, Request, ToEngine};
use lumagame_core::state::State;
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

const STOP_GRACE_MS: u64 = 2_000;
const STABLE_MS: u64 = 30_000;

pub struct Installed {
    pub manifest: Manifest,
    pub directory: PathBuf,
}

pub fn discover(root: &Path) -> io::Result<(Vec<Installed>, Vec<String>)> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => Some(entries),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let mut installed = BTreeMap::new();
    let mut notices = Vec::new();
    for entry in entries.into_iter().flatten() {
        let entry = entry?;
        if !entry.file_type()?.is_dir() { continue; }
        let result = (|| {
            let text = fs::read_to_string(entry.path().join("engine.json"))?;
            let manifest = Manifest::parse(&entry.file_name().to_string_lossy(), &text).map_err(io::Error::other)?;
            if manifest.exec.is_none() && manifest.id != native::id() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "only This TV is a built-in engine"));
            }
            Ok(manifest)
        })();
        match result {
            Ok(manifest) => { installed.insert(manifest.id.clone(), Installed { manifest, directory: entry.path().canonicalize()? }); }
            Err(e) => notices.push(format!("Engine {} is unavailable: {e}", entry.file_name().to_string_lossy())),
        }
    }
    installed.entry(native::id()).or_insert_with(|| Installed { manifest: native::manifest(), directory: root.join("native") });
    Ok((installed.into_values().collect(), notices))
}

pub struct Config {
    pub data: PathBuf,
    pub feeder: PathBuf,
    /// The package's private-loader wrapper, absent on the host.
    pub runner: Option<PathBuf>,
    pub display: Display,
    pub locale: String,
    pub lifecycle: bool,
}

pub enum HostEvent {
    Engine { engine: EngineId, message: FromEngine },
    Gone(EngineId),
    Notice(String),
}

enum Running { Process(Process), Native(Native) }

struct Slot {
    installed: Installed,
    running: Option<Running>,
    next_seq: u64,
    started: u64,
    retry_at: u64,
    failures: u32,
}

impl Slot {
    fn backoff(&mut self, now_ms: u64) {
        if now_ms.saturating_sub(self.started) >= STABLE_MS { self.failures = 0; }
        self.retry_at = now_ms.saturating_add(retry_delay(self.failures));
        self.failures = self.failures.saturating_add(1);
    }
    fn message(&mut self, request: Request) -> ToEngine {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        ToEngine { seq, request }
    }
}

fn retry_delay(failures: u32) -> u64 { (500u64 << failures.min(6)).min(30_000) }

pub struct Supervisor {
    config: Config,
    slots: BTreeMap<EngineId, Slot>,
    events: VecDeque<HostEvent>,
    readiness: OwnedFd,
}

impl Supervisor {
    pub fn new(installed: Vec<Installed>, config: Config) -> io::Result<Self> {
        // The window has three extra poll slots: controllers, lifecycle and all engine outputs.
        let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if fd < 0 { return Err(io::Error::last_os_error()); }
        let readiness = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self { config, readiness, events: VecDeque::new(), slots: installed.into_iter().map(|installed| {
            (installed.manifest.id.clone(), Slot { installed, running: None, next_seq: 1, started: 0, retry_at: 0, failures: 0 })
        }).collect() })
    }

    pub fn sync(&mut self, state: &State, foreground: bool, session_active: bool, now_ms: u64) {
        for (id, slot) in &mut self.slots {
            let wanted = (foreground || session_active) && state.engines.get(id).is_some_and(|e| e.enabled);
            if !wanted {
                match &mut slot.running {
                    Some(Running::Process(process)) => process.stop(now_ms),
                    Some(Running::Native(_)) => slot.running = None,
                    None => {}
                }
                slot.retry_at = 0;
                slot.failures = 0;
                continue;
            }
            if slot.running.is_some() || now_ms < slot.retry_at { continue; }
            let data = self.config.data.join("engines").join(id.as_str());
            let cache = self.config.data.join("cache").join(id.as_str());
            let start = (|| -> io::Result<Running> {
                private_dir(&data)?;
                private_dir(&cache)?;
                match &slot.installed.manifest.exec {
                    Some(exec) => {
                        let process = Process::spawn(&slot.installed.directory.join(exec), &slot.installed.manifest.args, &slot.installed.directory, &data, &cache, self.config.runner.as_deref())?;
                        watch_readable(self.readiness.as_raw_fd(), process.output.as_raw_fd())?;
                        Ok(Running::Process(process))
                    }
                    None => Ok(Running::Native(Native::new(self.config.lifecycle))),
                }
            })();
            match start {
                Ok(mut engine) => {
                    slot.started = now_ms;
                    let hello = slot.message(Request::Hello(Hello {
                        protocol: ipc::PROTOCOL, app_version: env!("CARGO_PKG_VERSION").into(),
                        data_dir: data.to_string_lossy().into_owned(), cache_dir: cache.to_string_lossy().into_owned(),
                        feeder: Some(self.config.feeder.to_string_lossy().into_owned()), locale: self.config.locale.clone(), display: self.config.display.clone(),
                        settings: state.engines.get(id).map(|e| e.settings.clone()).unwrap_or_default(),
                    }));
                    match &mut engine { Running::Process(p) => p.send(hello), Running::Native(n) => n.send(hello) }
                    slot.running = Some(engine);
                }
                Err(e) => {
                    slot.started = now_ms;
                    slot.backoff(now_ms);
                    self.events.push_back(HostEvent::Gone(id.clone()));
                    self.events.push_back(HostEvent::Notice(format!("{} could not start: {e}", slot.installed.manifest.name)));
                }
            }
        }
    }

    pub fn send(&mut self, id: &EngineId, request: Request) -> io::Result<()> {
        let slot = self.slots.get_mut(id).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "engine is not installed"))?;
        let message = slot.message(request);
        match &mut slot.running {
            Some(Running::Process(p)) if p.stopping.is_none() => { p.send(message); Ok(()) }
            Some(Running::Native(n)) => { n.send(message); Ok(()) }
            _ => Err(io::Error::new(io::ErrorKind::NotConnected, "engine is not running")),
        }
    }

    pub fn kill(&mut self, id: &EngineId, now_ms: u64) {
        if let Some(slot) = self.slots.get_mut(id) {
            // Drop kills the process group and reaps its leader before the model releases the picture lease.
            slot.running = None;
            slot.backoff(now_ms);
        }
    }

    pub fn poll(&mut self, now_ms: u64) -> Vec<HostEvent> {
        for (id, slot) in &mut self.slots {
            let mut ended = false;
            let mut unexpected = false;
            match &mut slot.running {
                Some(Running::Process(p)) => {
                    p.pump();
                    while let Some(message) = p.poll() {
                        if p.stopping.is_none() { self.events.push_back(HostEvent::Engine { engine: id.clone(), message }); }
                    }
                    if p.stopping.is_some_and(|at| now_ms.saturating_sub(at) >= STOP_GRACE_MS) { p.kill_group(); }
                    ended = p.eof || p.failure.is_some() || !matches!(p.child.try_wait(), Ok(None));
                    unexpected = ended && p.stopping.is_none();
                    if let Some(why) = p.failure.take() { self.events.push_back(HostEvent::Notice(format!("{}: {why}", slot.installed.manifest.name))); }
                }
                Some(Running::Native(n)) => while let Some(message) = n.poll() { self.events.push_back(HostEvent::Engine { engine: id.clone(), message }); },
                None => {}
            }
            if ended {
                slot.running = None;
                if unexpected {
                    slot.backoff(now_ms);
                    self.events.push_back(HostEvent::Gone(id.clone()));
                }
            }
        }
        self.events.drain(..).collect()
    }

    pub fn lifecycle(&mut self, message: &ToApp) {
        for slot in self.slots.values_mut() {
            if let Some(Running::Native(n)) = &mut slot.running { n.lifecycle(message); }
        }
    }

    pub fn lifecycle_commands(&mut self) -> Vec<FromApp> {
        let mut out = Vec::new();
        for slot in self.slots.values_mut() {
            if let Some(Running::Native(n)) = &mut slot.running { while let Some(command) = n.command() { out.push(command); } }
        }
        out
    }

    pub fn channel_failed(&mut self, why: String) {
        self.config.lifecycle = false;
        for slot in self.slots.values_mut() { if let Some(Running::Native(n)) = &mut slot.running { n.channel_failed(why.clone()); } }
    }

    pub fn fds(&self, out: &mut Vec<RawFd>) {
        out.push(self.readiness.as_raw_fd());
    }

    pub fn wait_ms(&self, now_ms: u64) -> u64 {
        self.slots.values().fold(1000, |wait, slot| match &slot.running {
            Some(Running::Process(p)) if !p.outgoing.is_empty() => wait.min(16),
            Some(Running::Process(p)) if p.stopping.is_some() => wait.min(p.stopping.unwrap().saturating_add(STOP_GRACE_MS).saturating_sub(now_ms)),
            None if slot.retry_at > now_ms => wait.min(slot.retry_at - now_ms),
            _ => wait,
        })
    }
}

fn watch_readable(watcher: RawFd, fd: RawFd) -> io::Result<()> {
    let mut event = libc::epoll_event { events: libc::EPOLLIN as u32, u64: 0 };
    if unsafe { libc::epoll_ctl(watcher, libc::EPOLL_CTL_ADD, fd, &mut event) } < 0 { return Err(io::Error::last_os_error()); }
    Ok(())
}

struct Process {
    child: Child,
    input: Option<ChildStdin>,
    output: ChildStdout,
    buffer: LineBuffer,
    outgoing: VecDeque<Vec<u8>>,
    offset: usize,
    queued: usize,
    events: VecDeque<FromEngine>,
    eof: bool,
    failure: Option<String>,
    stopping: Option<u64>,
}

impl Process {
    fn spawn(program: &Path, args: &[String], cwd: &Path, data: &Path, cache: &Path, runner: Option<&Path>) -> io::Result<Self> {
        let mut command = Command::new(runner.unwrap_or(program));
        if runner.is_some() { command.arg(program); }
        command.args(args).current_dir(cwd).env("HOME", data).env("XDG_CONFIG_HOME", data.join("config"))
            .env("XDG_DATA_HOME", data.join("data")).env("XDG_CACHE_HOME", cache)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        let parent = std::process::id() as libc::pid_t;
        // Only async-signal-safe system calls are allowed between fork and exec.
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, 0) < 0 || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) < 0 { return Err(io::Error::last_os_error()); }
                if libc::getppid() != parent { libc::_exit(1); }
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let input = child.stdin.take().expect("piped stdin");
        let output = child.stdout.take().expect("piped stdout");
        for fd in [input.as_raw_fd(), output.as_raw_fd()] {
            if let Err(e) = nonblocking(fd) { let _ = child.kill(); let _ = child.wait(); return Err(e); }
        }
        Ok(Self { child, input: Some(input), output, buffer: LineBuffer::default(), outgoing: VecDeque::new(), offset: 0, queued: 0,
            events: VecDeque::new(), eof: false, failure: None, stopping: None })
    }

    fn flush(&mut self) -> io::Result<()> {
        let Some(input) = &mut self.input else { return Ok(()) };
        while let Some(line) = self.outgoing.front() {
            match input.write(&line[self.offset..]) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "engine closed its input")),
                Ok(n) => {
                    self.offset += n; self.queued -= n;
                    if self.offset == line.len() { self.outgoing.pop_front(); self.offset = 0; }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn pump(&mut self) {
        if let Err(e) = self.flush() { self.failure = Some(e.to_string()); }
        let mut chunk = [0u8; 4096];
        for _ in 0..256 {
            match self.output.read(&mut chunk) {
                Ok(0) => { self.eof = true; break; }
                Ok(n) => self.buffer.push(&chunk[..n], |line| { if let Ok(message) = ipc::decode(line) { self.events.push_back(message); } }),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => { self.failure = Some(e.to_string()); break; }
            }
        }
    }

    fn stop(&mut self, now_ms: u64) {
        if self.stopping.is_none() {
            self.input = None;
            self.outgoing.clear();
            self.queued = 0;
            self.offset = 0;
            self.stopping = Some(now_ms);
        }
    }

    fn kill_group(&mut self) {
        // The child created its own group before exec; descendants cannot retain the feeder connection after a forced stop.
        unsafe { libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL); }
    }
}

impl Engine for Process {
    fn send(&mut self, message: ToEngine) {
        let line = ipc::encode(&message).into_bytes();
        if self.queued + line.len() > ipc::MAX_LINE_BYTES * 2 {
            self.failure = Some("engine stopped reading its requests".into());
            return;
        }
        self.queued += line.len();
        self.outgoing.push_back(line);
        if let Err(e) = self.flush() { self.failure = Some(e.to_string()); }
    }
    fn poll(&mut self) -> Option<FromEngine> { self.events.pop_front() }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.input = None;
        self.kill_group();
        let _ = self.child.wait();
    }
}

fn nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 { return Err(io::Error::last_os_error()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumagame_core::{engine::{Codec, Presentation}, ids::{GameId, SessionId}, ipc::{Event, SessionState}, session::{Input, Phase, Session}};

    #[test]
    fn restart_delay_is_bounded_and_resets_after_a_stable_run() {
        assert_eq!([0, 1, 6, 99].map(retry_delay), [500, 1000, 30000, 30000]);
        let mut slot = Slot { installed: Installed { manifest: native::manifest(), directory: PathBuf::new() }, running: None, next_seq: 1, started: 0, retry_at: 0, failures: 9 };
        slot.backoff(STABLE_MS);
        assert_eq!(slot.retry_at, STABLE_MS + 500);
    }

    #[test]
    fn only_enabled_engines_run_and_background_without_a_session_stops_them() {
        let root = std::env::var_os("CODY_DATA_SCRATCH").or_else(|| std::env::var_os("TMPDIR")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let temp = tempfile::tempdir_in(root).unwrap();
        let installed = vec![Installed { manifest: native::manifest(), directory: temp.path().into() }];
        let mut host = Supervisor::new(installed, Config { data: temp.path().into(), feeder: temp.path().join("feed"), runner: None,
            display: Display { width: 1920, height: 1080, refresh_hz: 60, hdr10: false, codecs: vec![Codec::H264] }, locale: "en-US".into(), lifecycle: false }).unwrap();
        let mut state = State::default();
        host.sync(&state, true, false, 0);
        assert!(host.slots[&native::id()].running.is_none());
        state.engines.entry(native::id()).or_default().enabled = true;
        host.sync(&state, true, false, 1);
        assert!(host.slots[&native::id()].running.is_some());
        host.sync(&state, false, true, 2);
        assert!(host.slots[&native::id()].running.is_some(), "a TV game owns the foreground");
        host.sync(&state, false, false, 3);
        assert!(host.slots[&native::id()].running.is_none());
    }

    #[test]
    fn one_window_poll_slot_wakes_for_every_engine_output() {
        let host = Supervisor::new(vec![], Config { data: PathBuf::new(), feeder: PathBuf::new(), runner: None,
            display: Display { width: 1920, height: 1080, refresh_hz: 60, hdr10: false, codecs: vec![Codec::H264] }, locale: "en-US".into(), lifecycle: false }).unwrap();
        let mut pipes: Vec<_> = (0..4).map(|_| std::os::unix::net::UnixStream::pair().unwrap()).collect();
        for (read, _) in &pipes { watch_readable(host.readiness.as_raw_fd(), read.as_raw_fd()).unwrap(); }
        let mut fds = Vec::new();
        host.fds(&mut fds);
        assert_eq!(fds.len(), 1);
        let mut poll = libc::pollfd { fd: fds[0], events: libc::POLLIN, revents: 0 };
        for (read, write) in &mut pipes {
            write.write_all(&[1]).unwrap();
            assert_eq!(unsafe { libc::poll(&mut poll, 1, 0) }, 1);
            read.read_exact(&mut [0]).unwrap();
            assert_eq!(unsafe { libc::poll(&mut poll, 1, 0) }, 0, "a drained engine must not spin the window loop");
        }
    }

    #[test]
    fn a_child_process_drives_checking_running_and_ended_without_a_window() {
        let root = std::env::var_os("CODY_DATA_SCRATCH").or_else(|| std::env::var_os("TMPDIR")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let temp = tempfile::tempdir_in(root).unwrap();
        let script = "while IFS= read -r line; do case \"$line\" in *'\"type\":\"hello\"'*) printf '%s\\n' '{\"type\":\"targets\",\"targets\":[]}' ;; *'\"type\":\"launch\"'*) printf '%s\\n' '{\"type\":\"session\",\"session\":1,\"state\":\"checking\"}' '{\"type\":\"session\",\"session\":1,\"state\":\"running\"}' ;; *'\"type\":\"stop\"'*) printf '%s\\n' '{\"type\":\"session\",\"session\":1,\"state\":\"ended\",\"reason\":\"stopped\"}' ;; esac; done";
        let mut process = Process::spawn(Path::new("/bin/sh"), &["-c".into(), script.into()], temp.path(), temp.path(), temp.path(), None).unwrap();
        let source = "fake|pc|game".parse().unwrap();
        let (mut session, _) = Session::start(SessionId(1), GameId(1), source, Presentation::Feeder, false, 0);
        process.send(ToEngine { seq: 1, request: Request::Hello(Hello { protocol: 1, app_version: "test".into(), data_dir: String::new(), cache_dir: String::new(), feeder: None, locale: "en-US".into(), display: Display { width: 1920, height: 1080, refresh_hz: 60, hdr10: false, codecs: vec![Codec::H264] }, settings: Default::default() }) });
        process.send(ToEngine { seq: 2, request: Request::Launch { session: SessionId(1), target: session.source.target.clone(), game: session.source.id.clone() } });
        let mut phases = Vec::new();
        for _ in 0..8 {
            let mut fd = libc::pollfd { fd: process.output.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            assert!(unsafe { libc::poll(&mut fd, 1, 100) } > 0, "fake engine did not answer");
            process.pump();
            while let Some(message) = process.poll() {
                if let Event::Session { state, .. } = message.event {
                    let running = matches!(state, SessionState::Running { .. });
                    session.step(1_000, Input::Engine(state));
                    phases.push(session.phase().clone());
                    if running { process.send(ToEngine { seq: 3, request: Request::Stop { session: SessionId(1), end_game: false } }); }
                }
            }
            if session.is_done() { break; }
        }
        assert!(matches!(phases.as_slice(), [Phase::Checking, Phase::Running, Phase::Done(_)]));
        process.stop(1_000);
        assert!(process.input.is_none());
    }
}
