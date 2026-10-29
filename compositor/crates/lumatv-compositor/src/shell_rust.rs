//! The Rust shell client (`docs/research/rust-shell-plan.md`, section 4.1): `lumatv-ui`, LumaTV's shell logic as a 64-bit process
//! without GL, whose layers the engine inside this compositor draws (feature `ui`, `src/ui`). It runs beside the QML client
//! (`shell.rs`) during the migration and answers the kinds `--shell-rust LIST` names (`ctl shell rust +kind -kind` changes the
//! list and restarts it): `osd`, the volume and mute display from the `audio` topic, `toast`, LG's toast feed, and `settings`, LumaTV's
//! Settings (opened by the `show` event, the gear held, `ctl shell show settings [PAGE]`; it takes every key but Home's while it is open). For a kind on
//! the list the compositor withholds the feed from the QML client (the `audio` topic, the toast feed's pushes), so one shell
//! draws each surface; for the rest the QML client goes on as before, untouched. The QML shell stays the fallback for a kind until
//! its Rust surface has passed the gate on the set twice.
//!
//! What the client gets at its start: the engine's socket (`LUMATV_UI_FD`, the client's end of the pair `Output::ui_attach`
//! made; the engine's handshake finishes once the client has said hello), the key socket (`LUMATV_UI_KEYS_FD`: JSON lines,
//! `{"key": {k, d, r, seat}}` while its layer has the keys and `{"pointer": {x, y, buttons, visible}}` samples), and the UI channel
//! on a port of its own (`--shell-port` plus one, a token of its own) with the same messages as the QML client's: the topics, the
//! `show` events, `luna` answers, and its reports (`can`, `wants`, `live`, `state`). Supervision is the QML client's: nice 10,
//! PR_SET_PDEATHSIG, a restart after a pause, given up after five starts in ten minutes, `ctl shell restart`. A client that is
//! gone takes its layers with it in one frame (the engine is dropped with its socket) and the compositor keeps composing.

use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::net::TcpListener;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::State;
use crate::shell::{
    EVENTS_KEPT, Link, LunaTag, Msg, NICE, POLL_TIMEOUT, RESTART_LIMIT, RESTART_WINDOW, Stats, TAKEABLE, TOOK_STALE, TRACE_MARKER, allowed, drain,
    spawn_server, wake, write_token,
};

/// The kinds the Rust shell may answer, in the order `ctl shell` lists them.
pub const KINDS: &[&str] = &["home", "osd", "settings", "toast", "cameras"];
/// Where the system layer's layers begin in the engine's z: everything the Rust shell draws over the windows is at this or more.
pub const SYSTEM_Z: i32 = 1000;
const TOKEN_FILE: &str = "lumatv-ui.token";
/// The client starts once SAM's Home has mapped, or this long after the compositor started (the QML client's rule).
const START_ALONE: Duration = Duration::from_secs(15);

/// `osd,toast` (or `+osd -toast` changes) as a sorted list of the known kinds; unknown names are dropped.
pub fn parse_kinds(list: &str) -> Vec<String> {
    let mut out: Vec<String> = list.split(',').map(str::trim).filter(|k| KINDS.contains(k)).map(str::to_string).collect();
    out.sort();
    out.dedup();
    out
}

/// `kinds` with `+kind` added and `-kind` removed, as `ctl shell rust` takes them; `Err` names an unknown word.
pub fn change_kinds(kinds: &[String], words: &[&str]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = kinds.to_vec();
    for w in words {
        let (add, name) = match w.as_bytes().first() {
            Some(b'+') => (true, &w[1..]),
            Some(b'-') => (false, &w[1..]),
            _ => return Err(format!("{w}: +kind or -kind")),
        };
        if !KINDS.contains(&name) {
            return Err(format!("{name}: not a kind ({})", KINDS.join(", ")));
        }
        out.retain(|k| k != name);
        if add {
            out.push(name.to_string());
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

pub struct RustShell {
    /// The kinds this client answers; empty means it is not run.
    pub kinds: Vec<String>,
    binary: Option<PathBuf>,
    link: Option<Link>,
    port: u16,
    token_path: Option<PathBuf>,
    log: Option<PathBuf>,
    child: Option<Child>,
    /// The engine's client end, attached before any child and not handed to one yet.
    socket: Option<UnixStream>,
    /// This end of the key socket, while a child has the other.
    keys: Option<UnixStream>,
    keys_refused: bool,
    child_started: Option<Instant>,
    starts: VecDeque<Instant>,
    next_start: Option<Instant>,
    last_wait: Option<Instant>,
    pub gave_up: bool,
    pub connected: bool,
    /// The engine's handshake with this child is done: its layers are drawn from here.
    pub engine_connected: bool,
    last_seen: Option<Instant>,
    pub can: Vec<String>,
    /// The client's system layer wants the keys: they go over the key socket (`State::shell_rust_takes_key`).
    pub wants: bool,
    pub takes: Vec<String>,
    pub took: Vec<(u32, Instant)>,
    pub live: bool,
    pub ui_state: Value,
    pub luna_tags: Vec<LunaTag>,
    cancelled_early: Vec<u64>,
    pushed: BTreeMap<String, Value>,
    pub recorded: Vec<Value>,
    pub stats: Stats,
    pub error: Option<String>,
    pub marker: PathBuf,
}

impl Default for RustShell {
    fn default() -> Self {
        RustShell {
            kinds: Vec::new(),
            binary: None,
            link: None,
            port: 0,
            token_path: None,
            log: None,
            child: None,
            socket: None,
            keys: None,
            keys_refused: false,
            child_started: None,
            starts: VecDeque::new(),
            next_start: None,
            last_wait: None,
            gave_up: false,
            connected: false,
            engine_connected: false,
            last_seen: None,
            can: Vec::new(),
            wants: false,
            takes: Vec::new(),
            took: Vec::new(),
            live: false,
            ui_state: Value::Null,
            luna_tags: Vec::new(),
            cancelled_early: Vec::new(),
            pushed: BTreeMap::new(),
            recorded: Vec::new(),
            stats: Stats::default(),
            error: None,
            marker: PathBuf::from(TRACE_MARKER),
        }
    }
}

impl Drop for RustShell {
    fn drop(&mut self) {
        if let Some(c) = &mut self.child {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl RustShell {
    /// Whether the client is meant to run (it has kinds) and its channel exists.
    pub fn enabled(&self) -> bool {
        self.link.is_some()
    }

    /// Whether `kind` is the Rust shell's: on the list, with the client meant to run. The feeds of such a kind are withheld from
    /// the QML client whether or not the Rust client is up yet, so a surface is never drawn twice.
    pub fn answers(&self, kind: &str) -> bool {
        self.enabled() && self.kinds.iter().any(|k| k == kind)
    }

    pub fn child_pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    /// The engine's client end, from the attach made before the loop (`main.rs`), kept for the first start.
    pub fn adopt_socket(&mut self, socket: UnixStream) {
        self.socket = Some(socket);
    }

    pub fn push(&mut self, topic: &str, data: Value) -> u64 {
        self.stats.events += 1;
        self.pushed.insert(topic.to_string(), data.clone());
        match &self.link {
            Some(l) => {
                let seq = l.outbox.lock().push(topic, data);
                wake(&l.wake_out);
                seq
            }
            None => {
                self.recorded.push(json!({"topic": topic, "data": data}));
                if self.recorded.len() > EVENTS_KEPT {
                    self.recorded.remove(0);
                }
                self.recorded.len() as u64
            }
        }
    }

    pub fn push_if_changed(&mut self, topic: &str, data: Value) -> bool {
        if self.pushed.get(topic) == Some(&data) {
            return false;
        }
        self.push(topic, data);
        true
    }

    pub fn topic(&self, name: &str) -> Option<&Value> {
        self.pushed.get(name)
    }

    fn note_seen(&mut self, now: Instant) {
        self.last_seen = Some(now);
        self.connected = true;
    }

    pub(crate) fn mark(&self, text: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&self.marker) {
            let _ = f.write_all(format!("{text}\n").as_bytes());
        }
    }

    /// One line to the client over the key socket; a client that does not read (its socket full) loses it, and that is said once.
    fn send_line(&mut self, line: &str) -> bool {
        let Some(k) = &mut self.keys else { return false };
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\n');
        match k.write_all(&bytes) {
            Ok(()) => true,
            Err(e) => {
                if !self.keys_refused {
                    self.keys_refused = true;
                    log::warn!("shell-rust: the key socket refused a line ({e}); keys are dropped until the client reads");
                }
                false
            }
        }
    }

    /// `{"key": {...}}` over the key socket.
    pub fn send_key(&mut self, name: &str, down: bool, repeat: bool, seat: u32) -> bool {
        let line = json!({"key": {"k": name, "d": down, "r": repeat, "seat": seat}}).to_string();
        self.send_line(&line)
    }

    /// `{"pointer": {...}}` over the key socket.
    pub fn send_pointer(&mut self, x: f32, y: f32, buttons: u32, visible: bool) -> bool {
        let line = json!({"pointer": {"x": x, "y": y, "buttons": buttons, "visible": visible}}).to_string();
        self.send_line(&line)
    }
}

/// Where the kinds chosen with `ctl shell rust` are kept between starts, under the data directory: one line, `osd,toast`. A
/// compositor started without `--shell-rust` reads it, so a flip made from the workstation survives a restart and a redeploy.
pub const KINDS_FILE: &str = "compositor/shell-rust.conf";

/// The kinds kept in `data_dir` (`KINDS_FILE`), none when there is no file.
pub fn read_kinds(data_dir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(data_dir.join(KINDS_FILE)).map(|t| parse_kinds(t.trim())).unwrap_or_default()
}

fn write_kinds(data_dir: &std::path::Path, kinds: &[String]) -> std::io::Result<()> {
    let path = data_dir.join(KINDS_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", kinds.join(",")))
}

/// `<bundle>/bin/lumatv-ui` beside this executable, or `LUMATV_UI_BIN`.
fn find_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LUMATV_UI_BIN") {
        let p = PathBuf::from(p);
        return p.exists().then_some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let beside = exe.parent()?.join("lumatv-ui");
    beside.exists().then_some(beside)
}

impl State {
    /// Opens the Rust client's channel and plans its first start (`--shell-rust`). Called with `shell_start`, once the Wayland
    /// socket listens. The engine was attached before the loop (`main.rs`, `Output::ui_attach`) and its client end adopted.
    pub fn shell_rust_start(&mut self) {
        let kinds = self.config.shell_rust.clone();
        if kinds.is_empty() {
            return;
        }
        self.shell_rust.kinds = kinds;
        let Some(binary) = find_binary() else {
            self.journal("shell-rust: no lumatv-ui beside this executable (or LUMATV_UI_BIN); the Rust shell is not run".into());
            self.shell_rust.error = Some("no lumatv-ui".into());
            return;
        };
        let port = self.config.shell_port + 1;
        let token_path = self.config.runtime_dir.join(TOKEN_FILE);
        let token = match write_token(&token_path) {
            Ok(t) => t,
            Err(e) => {
                self.journal(format!("shell-rust: token file {}: {e}; the Rust shell is not run", token_path.display()));
                self.shell_rust.error = Some(format!("token: {e}"));
                return;
            }
        };
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => {
                self.journal(format!("shell-rust: port {port}: {e}; the Rust shell is not run"));
                self.shell_rust.error = Some(format!("port {port}: {e}"));
                return;
            }
        };
        let link = match spawn_server(listener, token) {
            Ok(l) => l,
            Err(e) => {
                self.journal(format!("shell-rust: server: {e}; the Rust shell is not run"));
                self.shell_rust.error = Some(format!("server: {e}"));
                return;
            }
        };
        self.shell_rust.link = Some(link);
        self.shell_rust.binary = Some(binary);
        self.shell_rust.port = port;
        self.shell_rust.token_path = Some(token_path);
        self.shell_rust.log = Some(self.config.data_dir.join("compositor/shell-rust.log"));
        self.journal(format!("shell-rust: UI channel on 127.0.0.1:{port}, answers {}", self.shell_rust.kinds.join(", ")));
        self.shell_rust_sync_topics();
        self.shell_rust.next_start = Some(Instant::now() + START_ALONE);
    }

    /// Starts the client at the next tick instead of waiting for SAM's Home (the tests, `ctl shell restart`).
    pub fn shell_rust_start_now(&mut self) {
        if self.shell_rust.link.is_some() && self.shell_rust.child.is_none() {
            self.shell_rust.next_start = Some(Instant::now());
        }
    }

    /// The eventfd the server thread writes when a message waits; the loop polls it.
    pub fn shell_rust_fd(&self) -> Option<RawFd> {
        self.shell_rust.link.as_ref().map(|l| l.wake_in.as_raw_fd())
    }

    /// Starts the client: the engine attached (or the end kept from before the loop), the key socket pair, the fork and exec with
    /// the two sockets as descriptors 3 and 4.
    fn shell_rust_spawn(&mut self) {
        let Some(binary) = self.shell_rust.binary.clone() else { return };
        let now = Instant::now();
        self.shell_rust.starts.retain(|t| now.duration_since(*t) < RESTART_WINDOW);
        if self.shell_rust.starts.len() >= RESTART_LIMIT {
            self.shell_rust.gave_up = true;
            self.journal(format!("shell-rust: {RESTART_LIMIT} starts in {} min, giving up (ctl shell restart tries again)", RESTART_WINDOW.as_secs() / 60));
            return;
        }
        let shm = self.config.runtime_dir.join("ui-shm");
        let socket = match self.shell_rust.socket.take() {
            Some(s) => s,
            None => {
                if self.output.ui_attached() {
                    // the engine of the client before is still there: its socket closes with the child, and the next tick drops it
                    self.output.ui_detach();
                }
                let Some(driver) = self.config.driver.clone() else {
                    self.shell_rust.error = Some("no driver".into());
                    self.shell_rust.gave_up = true;
                    return;
                };
                if let Err(e) = std::fs::create_dir_all(&shm) {
                    self.journal(format!("shell-rust: {}: {e}", shm.display()));
                }
                match self.output.ui_attach(&driver, SYSTEM_Z, shm.clone()) {
                    Ok(s) => s,
                    Err(e) => {
                        self.journal(format!("shell-rust: the engine could not be attached: {e}"));
                        self.shell_rust.error = Some(format!("engine: {e}"));
                        self.shell_rust.next_start = Some(now + Duration::from_secs(10));
                        return;
                    }
                }
            }
        };
        let (ours, theirs) = match UnixStream::pair() {
            Ok(p) => p,
            Err(e) => {
                self.journal(format!("shell-rust: key socket: {e}"));
                self.shell_rust.next_start = Some(now + Duration::from_secs(10));
                self.shell_rust.socket = Some(socket);
                return;
            }
        };
        let _ = ours.set_nonblocking(true);
        self.shell_rust.starts.push_back(now);
        let log = self.shell_rust.log.clone().and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
        let mut cmd = Command::new(&binary);
        cmd.env_remove("NOTIFY_SOCKET")
            .env_remove("LUMATV_LUNA_BRIDGE")
            .env("LUMATV_UI_FD", "3")
            .env("LUMATV_UI_KEYS_FD", "4")
            .env("LUMATV_UI_KINDS", self.shell_rust.kinds.join(","))
            .env("LUMATV_UI_SYSTEM_Z", SYSTEM_Z.to_string())
            .env("LUMATV_UI_SHM", &shm)
            .env("LUMATV_SHELL_PORT", self.shell_rust.port.to_string())
            .env("LUMATV_DATA_DIR", &self.config.data_dir)
            .env("XDG_RUNTIME_DIR", &self.config.runtime_dir)
            .stdin(Stdio::null());
        if let Some(t) = &self.shell_rust.token_path {
            cmd.env("LUMATV_SHELL_TOKEN_FILE", t);
        }
        if let Some(module) = crate::shell::module_dir(&self.config.data_dir) {
            cmd.env("LUMATV_MODULE_DIR", module);
        }
        match log {
            Some(f) => {
                let err = f.try_clone().map(Stdio::from).unwrap_or_else(|_| Stdio::null());
                cmd.stdout(Stdio::from(f)).stderr(err);
            }
            None => {
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
            }
        }
        let (engine_fd, keys_fd) = (socket.as_raw_fd(), theirs.as_raw_fd());
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: dup2, prctl and setpriority in the child between fork and exec touch nothing shared; the descriptors are the
            // parent's own, open until the spawn returns.
            unsafe {
                cmd.pre_exec(move || {
                    if libc::dup2(engine_fd, 3) < 0 || libc::dup2(keys_fd, 4) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                    libc::setpriority(libc::PRIO_PROCESS, 0, NICE);
                    Ok(())
                });
            }
        }
        match cmd.spawn() {
            Ok(child) => {
                self.shell_rust.stats.starts += 1;
                self.journal(format!("shell-rust: client started, pid {} ({})", child.id(), binary.display()));
                self.shell_rust.child = Some(child);
                self.shell_rust.child_started = Some(now);
                self.shell_rust.next_start = None;
                self.shell_rust.keys = Some(ours);
                self.shell_rust.keys_refused = false;
                self.shell_rust.engine_connected = false;
            }
            Err(e) => {
                self.journal(format!("shell-rust: {} did not start: {e}", binary.display()));
                self.shell_rust.error = Some(format!("start: {e}"));
                self.shell_rust.next_start = Some(now + Duration::from_secs(10));
                self.output.ui_detach();
            }
        }
        // the child holds its ends now; ours of its two go
        drop(socket);
        drop(theirs);
    }

    /// Supervision, every tick: a client that exited is started again after a pause, one that stopped polling is taken as gone, the
    /// engine's handshake is finished once the client has said hello. Returns whether the engine's socket is new (the loop adds it
    /// to its poll set).
    pub fn shell_rust_tick(&mut self, now: Instant) -> bool {
        if self.shell_rust.link.is_none() {
            return false;
        }
        let mut new_socket = false;
        if self.shell_rust.last_wait.is_none_or(|t| now.duration_since(t) >= Duration::from_millis(500)) {
            self.shell_rust.last_wait = Some(now);
            let exited = self.shell_rust.child.as_mut().and_then(|c| c.try_wait().ok().flatten());
            if let Some(status) = exited {
                self.shell_rust.child = None;
                self.shell_rust.stats.exits += 1;
                let alive = self.shell_rust.child_started.map(|t| now.duration_since(t)).unwrap_or_default();
                self.journal(format!("shell-rust: client exited ({status}) after {:.0} s", alive.as_secs_f64()));
                self.shell_rust_client_gone("the client exited");
                let pause = if alive > Duration::from_secs(60) { 2 } else { 2u64 << self.shell_rust.starts.len().min(4) };
                self.shell_rust.next_start = Some(now + Duration::from_secs(pause));
            }
        }
        let first = self.shell_rust.starts.is_empty();
        if self.shell_rust.child.is_none()
            && !self.shell_rust.gave_up
            && self.shell_rust.next_start.is_some_and(|t| now >= t || (first && self.sam_home_mapped()))
        {
            self.shell_rust_spawn();
        }
        if self.shell_rust.child.is_some() && !self.shell_rust.engine_connected {
            match self.output.ui_handshake() {
                Ok(true) => {
                    self.shell_rust.engine_connected = true;
                    self.journal("shell-rust: the client is on the engine; its layers are drawn from here".into());
                    new_socket = true;
                    self.shell_rust_sync_home();
                }
                Ok(false) => {}
                Err(e) => {
                    self.journal(format!("shell-rust: the engine's handshake failed: {e}; the client is restarted"));
                    self.shell_rust_restart();
                }
            }
        }
        if self.shell_rust.connected && self.shell_rust.last_seen.is_some_and(|t| now.duration_since(t) > POLL_TIMEOUT) {
            self.shell_rust_client_gone("no poll for 40 s");
        }
        new_socket
    }

    fn shell_rust_client_gone(&mut self, why: &str) {
        if self.shell_rust.connected || !self.shell_rust.luna_tags.is_empty() {
            self.journal(format!(
                "shell-rust: client gone ({why}); {} luna subscription(s) cancelled",
                self.shell_rust.luna_tags.iter().filter(|t| t.subscribe).count()
            ));
        }
        self.shell_rust.connected = false;
        self.shell_rust.engine_connected = false;
        self.shell_rust_sync_home();
        self.shell_rust.can.clear();
        self.shell_rust.takes.clear();
        self.shell_rust.took.clear();
        self.shell_rust.cancelled_early.clear();
        self.shell_rust.keys = None;
        self.shell_rust.live = false;
        for t in std::mem::take(&mut self.shell_rust.luna_tags) {
            if t.subscribe {
                self.luna.send(crate::luna::ToBridge::Cancel { token: t.token });
            }
        }
        if self.shell_rust.wants {
            self.shell_rust.wants = false;
            crate::server::seat::refocus(self);
        }
        // its layers go with its socket: the engine is dropped and the next frame draws everything without them
        self.output.ui_detach();
    }

    /// Whether the Rust shell draws Home now: the `home` kind is its and the client is on the engine. The scene then leaves the QML
    /// shell's Home window out of the stack and counts Home as drawn while no app's card is in front, which is what the `foreground`
    /// topic tells both clients (the Rust one answers `wants` for the keys while Home is in front).
    pub fn shell_rust_sync_home(&mut self) {
        let on = self.shell_rust.answers("home") && self.shell_rust.engine_connected;
        if self.scene.set_home_rust(on) {
            self.journal(format!("shell-rust: Home is {}", if on { "the Rust shell's: the QML Home window is left out of the stack" } else { "the QML shell's again" }));
            self.shell_sync_topics();
        }
    }

    /// `ctl shell restart` (and a change of the kinds): ends the client and starts it again, whatever the count.
    pub fn shell_rust_restart(&mut self) {
        if self.shell_rust.link.is_none() {
            return;
        }
        if let Some(c) = &mut self.shell_rust.child {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.shell_rust.child = None;
        self.shell_rust.starts.clear();
        self.shell_rust.gave_up = false;
        self.shell_rust_client_gone("restart asked for");
        self.shell_rust.next_start = Some(Instant::now());
    }

    /// `ctl shell rust +kind -kind`: the new list, kept in the data directory for the next start (`KINDS_FILE`), the client restarted
    /// with it (or started, when none ran yet). The QML client gets the kinds it lost back through the feeds, and loses the ones the
    /// Rust shell gained, at the next change of each.
    pub fn shell_rust_set_kinds(&mut self, words: &[&str]) -> Result<(), String> {
        let kinds = change_kinds(&self.config.shell_rust, words)?;
        if kinds == self.config.shell_rust {
            return Ok(());
        }
        self.journal(format!("shell-rust: answers {}", if kinds.is_empty() { "nothing".to_string() } else { kinds.join(", ") }));
        self.config.shell_rust = kinds.clone();
        if let Err(e) = write_kinds(&self.config.data_dir, &kinds) {
            self.journal(format!("shell-rust: the kinds were not kept for the next start: {e}"));
        }
        if self.shell_rust.link.is_none() {
            self.shell_rust_start();
            self.shell_rust_start_now();
            return Ok(());
        }
        self.shell_rust.kinds = kinds;
        self.shell_rust_restart();
        self.shell_rust_sync_home();
        if self.shell_rust.kinds.is_empty() {
            self.shell_rust.next_start = None;
        }
        // what the QML client was denied is pushed again when it changes next; the audio state it has not seen goes now
        if let Some(audio) = self.shell_rust.topic("audio").cloned()
            && !self.shell_rust.answers("osd")
        {
            self.shell.push("audio", audio);
        }
        Ok(())
    }

    pub fn shell_rust_poll(&mut self) -> bool {
        let Some(l) = &self.shell_rust.link else { return false };
        drain(&l.wake_in);
        let mut msgs = Vec::new();
        while let Ok(m) = l.inbox.try_recv() {
            msgs.push(m);
        }
        let mut drew = false;
        for m in msgs {
            drew |= self.shell_rust_handle(m);
        }
        drew
    }

    /// One message from the Rust client. Returns whether a frame was drawn for it.
    pub fn shell_rust_handle(&mut self, msg: Msg) -> bool {
        let now = Instant::now();
        match msg {
            Msg::Connected => {
                let was = self.shell_rust.connected;
                self.shell_rust.note_seen(now);
                if !was {
                    self.journal("shell-rust: client connected".into());
                }
                self.shell_rust.pushed.clear();
                self.shell_rust_sync_topics();
            }
            Msg::Seen => self.shell_rust.note_seen(now),
            Msg::Ui { can, wants, live, state } => {
                self.shell_rust.note_seen(now);
                if let Some(can) = can
                    && can != self.shell_rust.can
                {
                    self.journal(format!("shell-rust: can show {}", if can.is_empty() { "nothing".to_string() } else { can.join(", ") }));
                    self.shell_rust.can = can;
                }
                if let Some(w) = wants
                    && w != self.shell_rust.wants
                {
                    self.shell_rust.wants = w;
                    self.journal(format!("shell-rust: the system layer {} the keys", if w { "wants" } else { "lets go of" }));
                    crate::server::seat::refocus(self);
                }
                if let Some(s) = state {
                    self.shell_rust.ui_state = s;
                }
                if let Some(l) = live
                    && l != self.shell_rust.live
                {
                    self.shell_rust.live = l;
                    if l {
                        // where a toast's first frame goes: the report that it is live against the engine's next frame
                        self.journal("shell-rust: system layer live".into());
                    }
                }
            }
            Msg::Luna { tag, uri, payload, subscribe } => {
                self.shell_rust.note_seen(now);
                if let Some(i) = self.shell_rust.cancelled_early.iter().position(|t| *t == tag) {
                    self.shell_rust.cancelled_early.remove(i);
                    self.shell_rust.stats.luna_cancelled_early += 1;
                    return false;
                }
                if !allowed(&uri) {
                    self.shell_rust.stats.luna_refused += 1;
                    self.journal(format!("shell-rust: luna {uri} refused (not on the allowlist)"));
                    self.shell_rust
                        .push("luna", json!({"tag": tag, "payload": {"returnValue": false, "errorCode": -1, "errorText": "not allowed by the compositor"}}));
                    return false;
                }
                self.shell_rust.stats.luna_calls += 1;
                let token = self.luna_token();
                self.shell_rust.luna_tags.push(LunaTag { tag, token, subscribe, uri: uri.clone() });
                if !subscribe && self.shell_rust.luna_tags.len() > 256 {
                    let n = self.shell_rust.luna_tags.len() - 256;
                    let mut dropped = 0;
                    self.shell_rust.luna_tags.retain(|t| {
                        let keep = t.subscribe || dropped >= n;
                        if !keep {
                            dropped += 1;
                        }
                        keep
                    });
                }
                self.luna.send(crate::luna::ToBridge::Outbound(crate::luna::Outbound { token, uri, payload, subscribe }));
            }
            Msg::Cancel { tag } => {
                self.shell_rust.note_seen(now);
                if let Some(i) = self.shell_rust.luna_tags.iter().position(|t| t.tag == tag) {
                    let t = self.shell_rust.luna_tags.remove(i);
                    if t.subscribe {
                        self.luna.send(crate::luna::ToBridge::Cancel { token: t.token });
                    }
                } else {
                    if self.shell_rust.cancelled_early.len() >= 64 {
                        self.shell_rust.cancelled_early.remove(0);
                    }
                    self.shell_rust.cancelled_early.push(tag);
                }
            }
            Msg::Ime(_) | Msg::Switcher(_) => {
                // the keyboard and the switcher are the QML client's until their kinds move
                self.shell_rust.note_seen(now);
            }
            Msg::Takes(names) => {
                self.shell_rust.note_seen(now);
                let mut takes: Vec<String> = Vec::new();
                for n in names {
                    if !takes.contains(&n) && TAKEABLE.contains(&n.as_str()) {
                        takes.push(n);
                    }
                }
                if takes != self.shell_rust.takes {
                    self.journal(format!(
                        "shell-rust: the system layer {}",
                        if takes.is_empty() { "lets go of OK and Back".to_string() } else { format!("takes {} for its cards", takes.join(" and ")) }
                    ));
                    self.shell_rust.takes = takes;
                }
            }
        }
        false
    }

    /// An answer to one of the compositor's calls: the Rust client's, if a tag of its owns the token.
    pub fn shell_rust_luna_event(&mut self, token: u64, payload: &Value) -> bool {
        let Some(i) = self.shell_rust.luna_tags.iter().position(|t| t.token == token) else { return false };
        let tag = self.shell_rust.luna_tags[i].tag;
        if let Some(feed) = crate::shell::notification_feed(&self.shell_rust.luna_tags[i].uri, payload) {
            self.shell_rust.mark(&format!("Q:{feed}-delivered"));
            self.journal(format!("shell-rust: {feed} delivered to the client"));
        }
        if !self.shell_rust.luna_tags[i].subscribe {
            self.shell_rust.luna_tags.remove(i);
        }
        self.shell_rust.push("luna", json!({"tag": tag, "payload": payload}));
        true
    }

    /// The topics the Rust client follows: the same as the QML client's (`shell_sync_topics` computes them and hands them here).
    pub fn shell_rust_sync_topics(&mut self) {
        if !self.shell_rust.enabled() {
            return;
        }
        for (topic, value) in self.shell_topic_values() {
            self.shell_rust.push_if_changed(&topic, value);
        }
    }

    /// Whether the Rust client's layer has the keys: the keys go over the key socket (`input.rs`).
    pub fn shell_rust_wants_keys(&self) -> bool {
        self.shell_rust.connected && self.shell_rust.wants && self.shell_rust.keys.is_some()
    }

    /// One key of the remote for the Rust client, before the focus routing: its layer wants the keys (every key but the compositor's
    /// own goes to it), or its cards take OK and Back by name while an app has the focus (`takes`, as the QML client's). Returns
    /// whether the key went to it.
    pub fn shell_rust_takes_key(&mut self, seat: crate::server::seat::SeatId, code: u32, pressed: bool, swallowed: bool, now: Instant) -> bool {
        let Some(name) = crate::server::ime::own_key_name(code) else { return false };
        let mut repeat = false;
        if let Some(i) = self.shell_rust.took.iter().position(|(c, _)| *c == code) {
            if !pressed {
                self.shell_rust.took.remove(i);
                self.shell_rust.send_key(name, false, false, seat.0);
                self.keys.delivered += 1;
                return true;
            }
            if now.saturating_duration_since(self.shell_rust.took[i].1) < TOOK_STALE {
                self.shell_rust.took[i].1 = now;
                repeat = true;
            } else {
                self.shell_rust.took.remove(i);
            }
        }
        if !repeat {
            if !pressed || swallowed || !self.shell_rust.connected {
                return false;
            }
            let wants = self.shell_rust_wants_keys();
            let takes = self.shell_rust.takes.iter().any(|n| n == name);
            if !wants && !takes {
                return false;
            }
            if takes && !wants {
                // only an app's focus is the layer's to share
                let focused = self.seat.focus.and_then(|s| self.scene.window_of(s)).and_then(|w| self.scene.window(w));
                if !focused.is_some_and(|w| !w.kind.is_shell()) || self.seat.pressed.contains(&(seat, code)) {
                    return false;
                }
            }
            self.shell_rust.took.push((code, now));
        }
        self.shell_rust.send_key(name, true, repeat, seat.0);
        self.keys.delivered += 1;
        true
    }

    /// A pointer sample for the Rust client, while its layer wants the keys (the sample's rate is the pacer's).
    pub fn shell_rust_pointer(&mut self, x: f32, y: f32, buttons: u32, visible: bool) {
        if self.shell_rust_wants_keys() {
            self.shell_rust.send_pointer(x, y, buttons, visible);
        }
    }

    /// `ctl shell`'s `rust` part.
    pub fn shell_rust_report(&self) -> Value {
        let s = &self.shell_rust;
        json!({
            "kinds": s.kinds,
            "enabled": s.enabled(),
            "gaveUp": s.gave_up,
            "error": s.error,
            "pid": s.child_pid(),
            "connected": s.connected,
            "engine": s.engine_connected,
            "can": s.can,
            "wants": s.wants,
            "takes": s.takes,
            "systemLive": s.live,
            "ui": s.ui_state,
            "topics": s.pushed,
            "lunaOpen": s.luna_tags.iter().map(|t| json!({"tag": t.tag, "uri": t.uri, "subscribe": t.subscribe})).collect::<Vec<_>>(),
            "stats": {"starts": s.stats.starts, "exits": s.stats.exits, "events": s.stats.events, "lunaCalls": s.stats.luna_calls, "lunaRefused": s.stats.luna_refused, "lunaCancelledEarly": s.stats.luna_cancelled_early, "shows": s.stats.shows},
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// A state with both clients' channels open on free ports (no child is started: no binary), the Rust shell answering `kinds`.
    fn state_with(kinds: &str) -> (State, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lumatv-shell-rust-{}-{}", std::process::id(), kinds.len()));
        std::fs::create_dir_all(&dir).unwrap();
        let display: wayland_server::Display<State> = wayland_server::Display::new().unwrap();
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let config = crate::config::Config {
            data_dir: dir.clone(),
            runtime_dir: dir.clone(),
            shell_port: port,
            shell_rust: parse_kinds(kinds),
            ..crate::config::Config::default()
        };
        let mut state = State::new(display.handle(), config, Box::new(crate::render::software::Software::new(64, 64)), Box::new(crate::luna::Null::default()));
        // the QML client's side as `shell_start` would leave it, without its runner
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        state.shell.link = Some(spawn_server(listener, "qml".into()).unwrap());
        let rust_listener = TcpListener::bind(("127.0.0.1", port + 1)).unwrap();
        state.shell_rust.link = Some(spawn_server(rust_listener, "rust".into()).unwrap());
        state.shell_rust.kinds = parse_kinds(kinds);
        state.shell.marker = dir.join("marker");
        state.shell_rust.marker = dir.join("marker");
        (state, dir)
    }

    #[test]
    fn the_audio_topic_goes_to_the_shell_that_answers_osd_and_the_toast_feed_likewise() {
        let (mut state, _dir) = state_with("osd,toast");
        state.shell_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Connected);
        state.shell_audio(&json!({"volumeStatus": {"volume": 3}}));
        assert!(state.shell.topic("audio").is_none(), "the QML client is not told the audio state while osd is the Rust shell's");
        assert_eq!(state.shell_rust.topic("audio").unwrap()["volumeStatus"]["volume"], 3);
        // both subscribe to the toast feed; the push reaches the Rust client alone
        state.shell_handle(Msg::Luna {
            tag: 7,
            uri: "luna://com.webos.notification/getToastNotification".into(),
            payload: json!({"subscribe": true}),
            subscribe: true,
        });
        state.shell_rust_handle(Msg::Luna {
            tag: 9,
            uri: "luna://com.webos.notification/getToastNotification".into(),
            payload: json!({"subscribe": true}),
            subscribe: true,
        });
        let qml_token = state.shell.luna_tags[0].token;
        let rust_token = state.shell_rust.luna_tags[0].token;
        assert!(state.shell_luna_event(qml_token, &json!({"message": "Remote battery low"})));
        assert!(state.shell.topic("luna").is_none(), "withheld from the QML client");
        assert!(state.shell_luna_event(rust_token, &json!({"message": "Remote battery low"})));
        assert_eq!(state.shell_rust.topic("luna").unwrap()["tag"], 9);
        assert!(state.journal.iter().any(|l| l.contains("shell-rust: toast delivered")));
        // a feed that is not a kind of the Rust shell goes to the QML client as before
        state.shell_handle(Msg::Luna {
            tag: 8,
            uri: "luna://com.webos.notification/getAlertNotification".into(),
            payload: json!({"subscribe": true}),
            subscribe: true,
        });
        let alert_token = state.shell.luna_tags.iter().find(|t| t.tag == 8).unwrap().token;
        assert!(state.shell_luna_event(alert_token, &json!({"alertAction": "open", "alertInfo": {"message": "hot"}})));
        assert_eq!(state.shell.topic("luna").unwrap()["tag"], 8);
    }

    #[test]
    fn with_osd_off_the_list_the_qml_client_keeps_the_audio_topic_and_the_switch_moves_it() {
        let (mut state, _dir) = state_with("toast");
        state.shell_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Connected);
        state.shell_audio(&json!({"volumeStatus": {"volume": 5}}));
        assert_eq!(state.shell.topic("audio").unwrap()["volumeStatus"]["volume"], 5);
        assert_eq!(state.shell_rust.topic("audio").unwrap()["volumeStatus"]["volume"], 5, "the Rust client follows the topic too");
        assert!(state.shell_rust_set_kinds(&["+osd"]).is_ok());
        assert_eq!(state.config.shell_rust, vec!["osd", "toast"]);
        assert!(state.shell_rust.answers("osd"));
        state.shell_audio(&json!({"volumeStatus": {"volume": 6}}));
        assert_eq!(state.shell.topic("audio").unwrap()["volumeStatus"]["volume"], 5, "the QML client hears no more of it");
        assert!(state.shell_rust_set_kinds(&["-osd"]).is_ok());
        assert_eq!(state.shell.topic("audio").unwrap()["volumeStatus"]["volume"], 6, "given back: the state it missed is pushed");
        assert!(state.shell_rust_set_kinds(&["quick"]).is_err());
        assert!(state.shell_rust_set_kinds(&["+quick"]).is_err());
    }

    #[test]
    fn the_shows_and_the_topics_reach_both_clients_and_a_kind_is_answered_by_its_shell() {
        let (mut state, _dir) = state_with("osd");
        state.shell_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Connected);
        state.shell_handle(Msg::Ui { can: Some(vec!["quick".into(), "picker".into()]), wants: None, live: None, state: None });
        state.shell_rust_handle(Msg::Ui { can: Some(vec![]), wants: Some(false), live: Some(false), state: Some(json!({"osd": false})) });
        assert!(state.shell_can("quick"), "a kind the Rust shell does not answer is the QML client's");
        assert!(state.shell_show("quick", false));
        assert_eq!(state.shell.topic("show").unwrap()["kind"], "quick");
        assert!(state.shell_rust.topic("show").is_none());
        assert!(!state.shell_can("osd"), "osd is the Rust shell's and it shows no panel");
        state.shell_sync_topics();
        for topic in ["foreground", "screen", "cursor", "refresh", "wall", "transitions", "keyboard"] {
            assert!(state.shell.topic(topic).is_some(), "{topic} for the QML client");
            assert_eq!(state.shell.topic(topic), state.shell_rust.topic(topic), "{topic} the same for the Rust client");
        }
        let report = state.shell_report();
        assert_eq!(report["rust"]["kinds"], json!(["osd"]));
        assert_eq!(report["rust"]["connected"], true);
        assert_eq!(report["rust"]["ui"]["osd"], false);
    }

    #[test]
    fn keys_go_over_the_key_socket_while_the_rust_layer_wants_them_and_its_cards_take_ok_and_back() {
        let (mut state, _dir) = state_with("osd");
        state.shell_rust_handle(Msg::Connected);
        let (ours, theirs) = UnixStream::pair().unwrap();
        ours.set_nonblocking(true).unwrap();
        state.shell_rust.keys = Some(ours);
        let seat = crate::server::seat::SeatId(0);
        let now = Instant::now();
        // nothing is wanted or taken: the key is not the Rust shell's
        assert!(!state.shell_rust_takes_key(seat, crate::input::code::KEY_UP, true, false, now));
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: None });
        assert!(state.shell_rust_takes_key(seat, crate::input::code::KEY_UP, true, false, now));
        assert!(state.shell_rust_takes_key(seat, crate::input::code::KEY_UP, true, false, now + Duration::from_millis(40)), "a repeat");
        assert!(state.shell_rust_takes_key(seat, crate::input::code::KEY_UP, false, false, now + Duration::from_millis(80)));
        assert!(!state.shell_rust_takes_key(seat, crate::input::code::KEY_UP, true, true, now), "a key the router swallowed stays swallowed");
        state.shell_rust_pointer(10.0, 20.0, 0, true);
        let mut reader = theirs;
        reader.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut text = String::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            text.push_str(std::str::from_utf8(&buf[..n]).unwrap());
            if text.matches('\n').count() >= 4 {
                break;
            }
        }
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert_eq!(lines[0], json!({"key": {"k": "up", "d": true, "r": false, "seat": 0}}));
        assert_eq!(lines[1], json!({"key": {"k": "up", "d": true, "r": true, "seat": 0}}));
        assert_eq!(lines[2], json!({"key": {"k": "up", "d": false, "r": false, "seat": 0}}));
        assert_eq!(lines[3], json!({"pointer": {"x": 10.0, "y": 20.0, "buttons": 0, "visible": true}}));
        // the layer lets go: the keys are the app's again, but its cards' OK is taken by name
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(false), live: None, state: None });
        assert!(!state.shell_rust_takes_key(seat, crate::input::code::KEY_UP, true, false, now));
        state.shell_rust_handle(Msg::Takes(vec!["ok".into(), "menu".into()]));
        assert_eq!(state.shell_rust.takes, vec!["ok"]);
    }

    #[test]
    fn settings_is_shown_by_the_rust_client_with_the_page_asked_for_and_only_when_it_can() {
        let (mut state, _dir) = state_with("settings");
        state.shell_rust_handle(Msg::Connected);
        assert!(!state.shell_show_page("settings", false, Some("privacy")), "the client has not said it can show it");
        state.shell_rust_handle(Msg::Ui { can: Some(vec!["settings".into()]), wants: Some(false), live: Some(false), state: None });
        assert!(state.shell_can("settings"));
        assert!(state.shell_show_page("settings", false, Some("privacy")));
        let show = state.shell_rust.topic("show").unwrap();
        assert_eq!((show["kind"].as_str(), show["page"].as_str()), (Some("settings"), Some("privacy")));
        assert!(state.shell_show("settings", false));
        assert!(state.shell_rust.topic("show").unwrap().get("page").is_none(), "no page: Settings opens where it was");
    }

    #[test]
    fn the_kinds_are_the_known_ones_sorted_and_changed_by_sign() {
        assert_eq!(parse_kinds("toast,osd,osd, nothing,settings"), vec!["osd", "settings", "toast"]);
        assert!(parse_kinds("").is_empty());
        let kinds = parse_kinds("osd");
        assert_eq!(change_kinds(&kinds, &["+toast"]).unwrap(), vec!["osd", "toast"]);
        assert_eq!(change_kinds(&kinds, &["-osd", "+toast"]).unwrap(), vec!["toast"]);
        assert!(change_kinds(&kinds, &["osd"]).is_err());
        assert!(change_kinds(&kinds, &["+quick"]).is_err());
    }
}
