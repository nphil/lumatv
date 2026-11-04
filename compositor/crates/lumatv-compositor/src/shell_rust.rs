//! Native shell client and its UI channel. The compositor owns rendering while
//! the supervised client supplies layers and receives input and service events.

use std::collections::BTreeMap;
use std::io::Write;
use std::net::TcpListener;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::State;
use crate::shell::{
    EVENTS_KEPT, Link, LunaTag, Msg, NICE, Stats, TAKEABLE, TOOK_STALE, TRACE_MARKER, allowed, drain,
    spawn_server, wake, write_token,
};

/// The kinds the native shell can provide.
pub const KINDS: &[&str] = &["home", "osd", "settings", "toast", "cameras", "quick", "picker", "drawer", "menu", "confirm", "switcher", "entry", "keyboard", "alert", "saver", "capture", "cards", "popup", "layout", "ai", "banner", "displaylab", "guide"];
/// Where the system layer's layers begin in the engine's z: everything the Rust shell draws over the windows is at this or more.
pub const SYSTEM_Z: i32 = 1000;
const TOKEN_FILE: &str = "lumatv-ui.token";
/// Start after SAM's Home maps, or after this delay.
const START_ALONE: Duration = Duration::from_secs(15);
// An idle long poll may last 60 seconds.
const POLL_TIMEOUT: Duration = Duration::from_secs(75);
const READY_TIMEOUT: Duration = Duration::from_secs(40);

/// Parse a diagnostic selection as a sorted list of known kinds.
pub fn parse_kinds(list: &str) -> Vec<String> {
    let mut out: Vec<String> = list.split(',').map(str::trim).filter(|k| KINDS.contains(k)).map(str::to_string).collect();
    out.sort();
    out.dedup();
    out
}


pub struct RustShell {
    /// The kinds this client answers; empty means it is not run.
    pub kinds: Vec<String>,
    launch: Option<Launch>,
    link: Option<Link>,
    port: u16,
    token_path: Option<PathBuf>,
    child: Option<Child>,
    /// The engine's client end, attached before any child and not handed to one yet.
    socket: Option<UnixStream>,
    /// This end of the key socket, while a child has the other.
    keys: Option<UnixStream>,
    keys_refused: bool,
    /// The unwritten end of a line the key socket took only part of: the next bytes on the socket must be these, or the client reads two lines as one.
    keys_tail: Vec<u8>,
    child_started: Option<Instant>,
    attach_started: Option<Instant>,
    frame_ready: bool,
    supported: Vec<String>,
    was_standby: bool,
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
    pub took: Vec<(u32, &'static str, Instant)>,
    /// The pointer is over the client's layers: its samples go over the key socket until it leaves (`State::shell_rust_pointer_sample`).
    pub pointer_in: bool,
    /// Where the last sample was.
    pointer_at: (f32, f32),
    /// The buttons of the last sample (bit 0: the left button).
    pub buttons: u32,
    pub live: bool,
    pub ui_state: Value,
    pub luna_tags: Vec<LunaTag>,
    cancelled_early: Vec<u64>,
    pushed: BTreeMap<String, Value>,
    subtitle_bounds: Option<(String, crate::surface::SurfaceId, Option<crate::Rect>)>,
    pub recorded: Vec<Value>,
    pub stats: Stats,
    pub error: Option<String>,
    /// The journal has said that the client waits for the engine's programs, since the last attach.
    shaders_told: bool,
    pub marker: PathBuf,
}

impl Default for RustShell {
    fn default() -> Self {
        RustShell {
            kinds: Vec::new(),
            launch: None,
            link: None,
            port: 0,
            token_path: None,
            child: None,
            socket: None,
            keys: None,
            keys_refused: false,
            keys_tail: Vec::new(),
            child_started: None,
            attach_started: None,
            frame_ready: false,
            supported: Vec::new(),
            was_standby: false,
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
            pointer_in: false,
            pointer_at: (0.0, 0.0),
            buttons: 0,
            live: false,
            ui_state: Value::Null,
            luna_tags: Vec::new(),
            cancelled_early: Vec::new(),
            pushed: BTreeMap::new(),
            subtitle_bounds: None,
            recorded: Vec::new(),
            stats: Stats::default(),
            error: None,
            shaders_told: false,
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

    /// Whether the native client is responsible for a kind.
    pub fn answers(&self, kind: &str) -> bool {
        self.enabled() && self.kinds.iter().any(|k| k == kind)
    }

    pub fn child_pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    fn ready(&self) -> bool {
        self.connected && self.engine_connected && self.frame_ready
            && self.kinds.iter().all(|kind| self.supported.contains(kind))
    }

    fn attach_timed_out(&mut self, now: Instant) -> bool {
        now.saturating_duration_since(*self.attach_started.get_or_insert(now)) > READY_TIMEOUT
    }

    /// The engine's client end, from the attach made before the loop (`main.rs`), kept for the first start.
    pub fn adopt_socket(&mut self, socket: UnixStream) {
        self.socket = Some(socket);
    }

    /// Supplies the key channel when the client is started externally.
    pub fn adopt_key_socket(&mut self, socket: UnixStream) {
        self.keys = Some(socket);
        self.keys_tail.clear();
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

    /// The bounded event history, including events queued for a connected client.
    pub fn events(&self) -> Vec<Value> {
        match &self.link {
            Some(link) => link.outbox.lock().events.iter().map(|(_, topic, data)| json!({"topic": topic, "data": data})).collect(),
            None => self.recorded.clone(),
        }
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

    /// One line to the client over the key socket. A client that does not read (its socket full) loses the line, and that is said once; a line the
    /// socket took only part of is finished before anything else goes (`write_line`), so the client never reads a spliced one.
    fn send_line(&mut self, line: &str) -> bool {
        let Some(k) = &mut self.keys else { return false };
        match write_line(k, &mut self.keys_tail, line) {
            Ok(()) => {
                self.keys_refused = false;
                true
            }
            Err(e) => {
                if !self.keys_refused {
                    self.keys_refused = true;
                    log::warn!("shell-rust: the key socket refused a line ({e}); keys are dropped until the client reads");
                }
                false
            }
        }
    }

    /// Finishes a line the key socket took only part of, once the client has read; called each tick so that it does not wait for the next key.
    fn flush_keys(&mut self) {
        if let Some(k) = &mut self.keys {
            let _ = flush(k, &mut self.keys_tail);
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


/// Writes what is left of a line; a socket that is full leaves the rest in `tail` and says so as `WouldBlock`, a socket that is gone empties it.
fn flush(w: &mut impl Write, tail: &mut Vec<u8>) -> std::io::Result<()> {
    while !tail.is_empty() {
        match w.write(tail) {
            Ok(0) => {
                tail.clear();
                return Err(std::io::ErrorKind::WriteZero.into());
            }
            Ok(n) => {
                tail.drain(..n);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                if e.kind() != std::io::ErrorKind::WouldBlock {
                    tail.clear();
                }
                return Err(e);
            }
        }
    }
    Ok(())
}

/// One line on a non-blocking socket. The end of an earlier line that the socket took only part of goes first, and while it cannot, the new line is
/// dropped; a new line the socket takes only part of keeps its end in `tail` and counts as sent, so that the next line starts on a line boundary.
fn write_line(w: &mut impl Write, tail: &mut Vec<u8>, line: &str) -> std::io::Result<()> {
    flush(w, tail)?;
    tail.extend_from_slice(line.as_bytes());
    tail.push(b'\n');
    let whole = tail.len();
    match flush(w, tail) {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && tail.len() == whole => {
            tail.clear();
            Err(e)
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
        other => other,
    }
}

/// How the client is run. On the set the supervisor starts the compositor as `<lib>/ld-linux-aarch64.so.1 --library-path <lib>
/// <bin>/lumatv-compositor`, because the 64-bit ELFs name `/lib/ld-linux-aarch64.so.1` as their interpreter and the set's 32-bit
/// userland has none. The client has the same interpreter, so it is run the same way; on the host it is exec'd directly.
#[derive(Clone, Debug, PartialEq)]
struct Launch {
    binary: PathBuf,
    loader: Option<Loader>,
}

#[derive(Clone, Debug, PartialEq)]
struct Loader {
    path: PathBuf,
    lib: PathBuf,
}

impl Launch {
    fn command(&self) -> Command {
        match &self.loader {
            Some(l) => {
                let mut cmd = Command::new(&l.path);
                cmd.arg("--library-path").arg(&l.lib).arg(&self.binary);
                cmd
            }
            None => Command::new(&self.binary),
        }
    }
}

/// The loader this process was started through and the program it was given, from `/proc/self/exe` and `/proc/self/cmdline`
/// (`current_exe` is the loader then, and the loader leaves argv as the supervisor wrote it). None when it runs directly.
fn through_loader(exe: &Path, cmdline: &[u8]) -> Option<(Loader, PathBuf)> {
    if !exe.file_name()?.to_str()?.starts_with("ld-linux") {
        return None;
    }
    let mut args = cmdline.split(|b| *b == 0).skip(1).map(|a| PathBuf::from(std::ffi::OsStr::from_bytes(a)));
    let mut lib = None;
    while let Some(a) = args.next() {
        match a.to_str() {
            Some("--library-path") => lib = args.next(),
            Some("--preload" | "--argv0" | "--audit" | "--glibc-hwcaps-prepend" | "--glibc-hwcaps-mask") => {
                args.next();
            }
            Some(o) if o.starts_with("--") => {}
            _ => {
                let program = std::fs::canonicalize(&a).unwrap_or(a);
                let lib = lib.unwrap_or_else(|| exe.parent().map(Path::to_path_buf).unwrap_or_default());
                return Some((Loader { path: exe.to_path_buf(), lib }, program));
            }
        }
    }
    None
}

/// Where the client is and how to run it. `LUMATV_UI_BIN` names the binary, `LUMATV_UI_LOADER` (and `LUMATV_UI_LIBDIR`, default the
/// loader's own directory) the loader to run it through; without them the client is `lumatv-ui` in the directory of the program this
/// process runs (`<bundle>/bin`), through the loader this process was started through, if any.
fn find_launch_with(env_bin: Option<PathBuf>, env_loader: Option<PathBuf>, env_lib: Option<PathBuf>, exe: &Path, cmdline: &[u8]) -> Option<Launch> {
    let ours = through_loader(exe, cmdline);
    let binary = match env_bin {
        Some(p) => p,
        None => {
            let own = ours.as_ref().map(|(_, program)| program.as_path()).unwrap_or(exe);
            own.parent()?.join("lumatv-ui")
        }
    };
    let loader = match env_loader {
        Some(path) => {
            let lib = env_lib.or_else(|| path.parent().map(Path::to_path_buf)).unwrap_or_default();
            Some(Loader { path, lib })
        }
        None => ours.map(|(l, _)| l),
    };
    let launch = Launch { binary, loader };
    (launch.binary.exists() && launch.loader.as_ref().is_none_or(|l| l.path.exists())).then_some(launch)
}

fn find_launch() -> Option<Launch> {
    let var = |k| std::env::var_os(k).map(PathBuf::from);
    let exe = std::env::current_exe().ok()?;
    let cmdline = std::fs::read("/proc/self/cmdline").unwrap_or_default();
    find_launch_with(var("LUMATV_UI_BIN"), var("LUMATV_UI_LOADER"), var("LUMATV_UI_LIBDIR"), &exe, &cmdline)
}

impl State {
    /// Open the native client's channel once the Wayland socket listens.
    pub fn shell_rust_start(&mut self) {
        let kinds = self.config.shell_rust.clone();
        if kinds.is_empty() {
            return;
        }
        self.shell_rust.kinds = kinds;
        let Some(launch) = find_launch() else {
            self.journal("shell-rust: no lumatv-ui beside the compositor's program (or LUMATV_UI_BIN), or its loader is gone; the Rust shell is not run".into());
            self.shell_rust.error = Some("no lumatv-ui".into());
            self.shell_rust.gave_up = true;
            return;
        };
        let port = self.config.shell_port + 1;
        let token_path = self.config.runtime_dir.join(TOKEN_FILE);
        let token = match write_token(&token_path) {
            Ok(t) => t,
            Err(e) => {
                self.journal(format!("shell-rust: token file {}: {e}; the Rust shell is not run", token_path.display()));
                self.shell_rust.error = Some(format!("token: {e}"));
                self.shell_rust.gave_up = true;
                return;
            }
        };
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => {
                self.journal(format!("shell-rust: port {port}: {e}; the Rust shell is not run"));
                self.shell_rust.error = Some(format!("port {port}: {e}"));
                self.shell_rust.gave_up = true;
                return;
            }
        };
        let link = match spawn_server(listener, token) {
            Ok(l) => l,
            Err(e) => {
                self.journal(format!("shell-rust: server: {e}; the Rust shell is not run"));
                self.shell_rust.error = Some(format!("server: {e}"));
                self.shell_rust.gave_up = true;
                return;
            }
        };
        self.shell_rust.link = Some(link);
        self.shell_rust.launch = Some(launch);
        self.shell_rust.port = port;
        self.shell_rust.token_path = Some(token_path);
        self.journal(format!("shell-rust: UI channel on 127.0.0.1:{port}, answers {}", self.shell_rust.kinds.join(", ")));
        self.shell_rust_sync_topics();
        self.shell_rust.next_start = Some(Instant::now() + self.ui_start_wait());
    }

    /// How long the client's first start waits for SAM's Home to map. Under boot direct there is no wait: SAM's Home is the stub, whose
    /// launch waits for WebAppMgr, which comes up with the compositor, and LumaTV's Home does not need it.
    fn ui_start_wait(&self) -> Duration {
        if self.boot_direct.on() { Duration::ZERO } else { START_ALONE }
    }

    /// Opens the channel on `port` with `token` for a client the caller runs itself (the tests, which answer for it).
    pub fn shell_rust_open(&mut self, port: u16, token: String) -> std::io::Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        self.shell_rust.link = Some(spawn_server(listener, token)?);
        self.shell_rust.port = port;
        Ok(())
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
        let Some(launch) = self.shell_rust.launch.clone() else { return };
        let binary = launch.binary.clone();
        let now = Instant::now();
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
                    Ok(s) => {
                        self.shell_rust.shaders_told = false;
                        self.shell_rust.attach_started = None;
                        // The client is forked at the next tick: the attach and the fork each take a pass of their own, so that neither adds
                        // to the other's.
                        self.shell_rust.socket = Some(s);
                        self.shell_rust.next_start = Some(now);
                        return;
                    }
                    // The engine's programs are being linked on a thread of its own (`Output::ui_attach`): the loop goes on, and asks
                    // again at the next tick.
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if self.shell_rust.attach_timed_out(now) {
                            self.shell_rust.error = Some("UI engine attachment timed out".into());
                            self.shell_rust.gave_up = true;
                            return;
                        }
                        if !std::mem::replace(&mut self.shell_rust.shaders_told, true) {
                            self.journal("shell-rust: the engine's programs are being linked on a thread of its own; the client starts when they are".into());
                        }
                        self.shell_rust.next_start = Some(now + Duration::from_millis(50));
                        return;
                    }
                    Err(e) => {
                        self.journal(format!("shell-rust: the engine could not be attached: {e}"));
                        self.shell_rust.error = Some(format!("engine: {e}"));
                        self.shell_rust.gave_up = true;
                        return;
                    }
                }
            }
        };
        let (ours, theirs) = match UnixStream::pair() {
            Ok(p) => p,
            Err(e) => {
                self.journal(format!("shell-rust: key socket: {e}"));
                self.shell_rust.error = Some(format!("key socket: {e}"));
                self.shell_rust.gave_up = true;
                self.shell_rust.socket = Some(socket);
                return;
            }
        };
        let _ = ours.set_nonblocking(true);
        let log = crate::childlog::pipe("lumatv-shell-rust", "");
        let mut cmd = launch.command();
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
        } else {
            self.shell_rust.error = Some("native UI assets missing".into());
            self.shell_rust.gave_up = true;
            self.output.ui_detach();
            return;
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
            let parent = std::process::id() as libc::pid_t;
            unsafe {
                cmd.pre_exec(move || {
                    if libc::dup2(engine_fd, 3) < 0 || libc::dup2(keys_fd, 4) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::getppid() != parent {
                        return Err(std::io::Error::other("compositor exited during UI launch"));
                    }
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
                self.shell_rust.frame_ready = false;
                self.shell_rust.supported.clear();
                self.shell_rust.next_start = None;
                self.shell_rust.keys = Some(ours);
                self.shell_rust.keys_tail.clear();
                self.shell_rust.keys_refused = false;
                self.shell_rust.engine_connected = false;
            }
            Err(e) => {
                self.journal(format!("shell-rust: {} did not start: {e}", binary.display()));
                self.shell_rust.error = Some(format!("start: {e}"));
                self.shell_rust.gave_up = true;
                self.output.ui_detach();
            }
        }
        // the child holds its ends now; ours of its two go
        drop(socket);
        drop(theirs);
    }

    /// Check the client and complete its engine handshake.
    pub fn shell_rust_tick(&mut self, now: Instant) -> bool {
        if self.shell_rust.link.is_none() {
            return false;
        }
        self.shell_rust.flush_keys();
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
                self.shell_rust.error = Some(format!("client exited: {status}"));
                self.shell_rust.gave_up = true;
            }
        }
        let first = self.shell_rust.stats.starts == 0;
        if self.shell_rust.child.is_none()
            && !self.shell_rust.gave_up
            && self.shell_rust.next_start.is_some_and(|t| now >= t || (first && self.sam_home_mapped()))
        {
            self.shell_rust_spawn();
        }
        if self.shell_rust.child.is_some() && !self.shell_rust.engine_connected {
            match self.output.ui_handshake() {
                Ok(true) => {
                    self.shell_rust_engine_connected();
                    new_socket = true;
                }
                Ok(false) => {}
                Err(e) => {
                    self.journal(format!("shell-rust: the engine's handshake failed: {e}"));
                    self.shell_rust.error = Some(format!("handshake: {e}"));
                    self.shell_rust.gave_up = true;
                }
            }
        }
        let standby = !self.luna_state.power.rendering;
        if standby || self.shell_rust.was_standby {
            if self.shell_rust.connected {
                self.shell_rust.last_seen = Some(now);
            }
            if self.shell_rust.child.is_some() && !self.shell_rust.ready() {
                self.shell_rust.child_started = Some(now);
            }
        }
        self.shell_rust.was_standby = standby;
        if !standby && self.shell_rust.connected && self.shell_rust.last_seen.is_some_and(|t| now.saturating_duration_since(t) > POLL_TIMEOUT) {
            self.shell_rust_client_gone("UI polling timed out");
            if self.shell_rust.launch.is_some() {
                self.shell_rust.error = Some("UI stopped polling".into());
                self.shell_rust.gave_up = true;
            }
        }
        if self.shell_rust.launch.is_some() && self.shell_rust.child.is_some()
            && !self.shell_rust.ready() && self.luna_state.power.rendering
            && self.shell_rust.child_started.is_some_and(|t| now.saturating_duration_since(t) > READY_TIMEOUT)
        {
            self.shell_rust.error = Some("UI did not become ready".into());
            self.shell_rust.gave_up = true;
        }
        new_socket
    }

    fn shell_rust_engine_connected(&mut self) {
        self.shell_rust.ui_state = Value::Null;
        self.shell_rust.engine_connected = true;
        self.journal("shell-rust: the client is on the engine; its layers are drawn from here".into());
        self.shell_rust_sync_home();
        self.refresh_policy_sync();
        // A report may have arrived before the engine handshake completed.
        self.shell_rust.push("refresh", json!({"hz": self.config.refresh_hz}));
    }

    /// Why the compositor ends because of the Rust shell: its client gave up (it did not start, exited, stopped answering or never became
    /// ready), and `default` when no reason was kept. The main loop returns it, which ends the compositor with an error: the supervisor
    /// falls back to LG's compositor and `recover-home.sh --stock` brings LG's Home back (and drops the Home stub of boot direct).
    pub fn shell_rust_failure(&self, default: &str) -> Option<String> {
        self.shell_rust.gave_up.then(|| self.shell_rust.error.clone().unwrap_or_else(|| default.into()))
    }

    fn shell_rust_client_gone(&mut self, why: &str) {
        if self.shell_rust.answers("keyboard") {
            crate::server::ime::own_gone(self);
        }
        if self.shell_rust.connected || !self.shell_rust.luna_tags.is_empty() {
            self.journal(format!(
                "shell-rust: client gone ({why}); {} luna subscription(s) cancelled",
                self.shell_rust.luna_tags.iter().filter(|t| t.subscribe).count()
            ));
        }
        if self.shell_rust.answers("switcher") {
            self.switcher_reset();
        }
        self.shell_rust.connected = false;
        self.shell_rust.engine_connected = false;
        self.shell_rust.frame_ready = false;
        self.shell_rust.supported.clear();
        self.shell_rust.attach_started = None;
        self.shell_rust.ui_state = Value::Null;
        self.shell_rust_sync_home();
        self.refresh_policy_sync();
        self.shell_rust.can.clear();
        self.shell_rust.takes.clear();
        // what the shell blanked of an app's window is the app's own picture again once the shell is gone; the next plan redraws it
        self.scene.mask_op(crate::mask::Op::Release);
        self.shell_rust.took.clear();
        self.shell_rust.cancelled_early.clear();
        self.shell_rust.keys = None;
        self.shell_rust.keys_tail.clear();
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
        // and the end of its socket that waited for the next child is the end of nothing
        self.shell_rust.socket = None;
    }

    /// Synchronize Home visibility with the native engine connection. LumaTV's Home is not drawn while LG's own is what the owner asked for
    /// (`boot_direct`): the client is up, but its Home gives the screen to LG's card.
    pub fn shell_rust_sync_home(&mut self) {
        let stepped_aside = self.boot_direct.hides_home();
        let on = self.shell_rust.answers("home") && self.shell_rust.engine_connected && !stepped_aside;
        if self.scene.set_home_rust(on) {
            let state = if on {
                "native"
            } else if stepped_aside {
                "stepped aside for LG's Home"
            } else {
                "not connected"
            };
            self.journal(format!("shell-rust: Home is {state}"));
            self.shell_sync_topics();
        }
    }

    /// `ctl shell restart` (and a change of the kinds): ends the client and starts it again.
    pub fn shell_rust_restart(&mut self) {
        if self.shell_rust.link.is_none() {
            return;
        }
        if let Some(c) = &mut self.shell_rust.child {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.shell_rust.child = None;
        self.shell_rust.gave_up = false;
        self.shell_rust_client_gone("restart asked for");
        self.shell_rust.next_start = Some(Instant::now());
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
                self.switcher.topics_sig = 0;
                self.shell_rust_sync_topics();
                self.screencap_publish();
            }
            Msg::Seen => self.shell_rust.note_seen(now),
            Msg::FrameReady => {
                self.shell_rust.note_seen(now);
                self.shell_rust.frame_ready = true;
            }
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
                let changed = if let Some(s) = state {
                    if let Some(kinds) = s.get("kinds").and_then(Value::as_array) {
                        self.shell_rust.supported = kinds.iter().filter_map(Value::as_str).map(str::to_owned).collect();
                    }
                    let changed = self.output.ui_state(&s);
                    self.shell_rust.ui_state = s;
                    changed
                } else {
                    if let Some(s) = self.shell_rust.ui_state.as_object_mut() {
                        s.remove("interactiveVisible");
                    }
                    false
                };
                if let Some(l) = live
                    && l != self.shell_rust.live
                {
                    self.shell_rust.live = l;
                    if l {
                        // where a toast's first frame goes: the report that it is live against the engine's next frame
                        self.journal("shell-rust: system layer live".into());
                    }
                }
                self.refresh_policy_sync();
                if changed && self.output.draws_on_damage(now) {
                    self.tick();
                    return true;
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
                if self.remote_service_refused(&uri, &payload) {
                    self.shell_rust.stats.luna_refused += 1;
                    self.journal("shell-rust: LG Remote Service is blocked (Settings > Privacy), launch dropped".into());
                    self.shell_rust.push("luna", json!({"tag": tag, "payload": {"returnValue": false, "errorCode": -1, "errorText": "LG Remote Service is blocked"}}));
                    return false;
                }
                // Where a tile is for the transitions is the compositor's own knowledge: answered here, not sent to the bus, so it does not depend on the
                // bridge (`lumatv-sm`) on the set knowing the method.
                if uri == "luna://com.webos.surfacemanager/setLaunchOrigin" {
                    self.shell_rust.stats.luna_calls += 1;
                    let answer = self.set_launch_origin(&payload);
                    self.shell_rust.push("luna", json!({"tag": tag, "payload": answer}));
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
            Msg::Ime(body) => {
                self.shell_rust.note_seen(now);
                // the keyboard of the apps' fields: the QML client's until the kind moves
                if self.shell_rust.answers("keyboard") {
                    crate::server::ime::own_op(self, &body);
                }
            }
            Msg::Switcher(body) => {
                self.shell_rust.note_seen(now);
                // the pictures move on the compositor's clock: a still screen shows the change at once, a running one at the tick
                if self.shell_rust.answers("switcher") && self.switcher_op(&body, now) && self.output.draws_on_damage(now) {
                    self.tick();
                    return true;
                }
            }
            Msg::Mask(body) => {
                self.shell_rust.note_seen(now);
                match crate::mask::Op::parse(&body) {
                    Ok(op) => {
                        if self.scene.mask_op(op) && self.output.draws_on_damage(now) {
                            self.tick();
                            return true;
                        }
                    }
                    Err(why) => self.journal(format!("shell-rust: mask refused ({why})")),
                }
            }
            Msg::Vwin(body) => {
                self.shell_rust.note_seen(now);
                self.vwin_command(&body);
                if self.output.draws_on_damage(now) {
                    self.tick();
                    return true;
                }
            }
            Msg::Front(app) => {
                self.shell_rust.note_seen(now);
                crate::server::webos_shell::front_app(self, &app);
                if self.output.draws_on_damage(now) {
                    self.tick();
                    return true;
                }
            }
            Msg::AppKey { app, key } => {
                self.shell_rust.note_seen(now);
                crate::server::webos_shell::app_key(self, &app, &key);
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

    fn shell_rust_foreground_surface(&self) -> Option<u32> {
        let window = self.scene.foreground(&self.surfaces)?;
        self.surfaces.get(window.surface).filter(|s| s.is_mapped()).map(|_| window.surface.0)
    }

    /// Foreign element updates need a topic push only when the visible identity changes.
    pub(crate) fn shell_rust_sync_video(&mut self) {
        if !self.shell_rust.enabled() {
            return;
        }
        let video = self.shell_rust.topic("foreground").and_then(|v| v.get("video"));
        let same = match (video, self.foreground_video()) {
            (Some(Value::Null), None) => true,
            (Some(v), Some((context, app))) => v.get("context").and_then(Value::as_str) == Some(context)
                && v.get("app").and_then(Value::as_str) == Some(app),
            _ => false,
        };
        let surface = self.shell_rust_foreground_surface();
        let old_surface = self.shell_rust.topic("foreground").and_then(|v| v.get("surface"));
        let same_surface = match (old_surface, surface) {
            (Some(Value::Null), None) => true,
            (Some(v), Some(id)) => v.as_u64() == Some(id as u64),
            _ => false,
        };
        if !same || !same_surface {
            let Some(mut foreground) = self.shell_rust.topic("foreground").cloned() else {
                self.shell_rust_sync_topics();
                return;
            };
            foreground["video"] = match self.foreground_video() {
                Some((context, app)) => json!({"context": context, "app": app}),
                None => Value::Null,
            };
            foreground["surface"] = json!(surface);
            self.shell_rust.push_if_changed("foreground", foreground);
        }
    }

    pub(crate) fn shell_rust_sync_subtitle_bounds(&mut self) {
        if !self.shell_rust.enabled() {
            return;
        }
        let bounds = self.scene.subtitle_bounds(&self.surfaces);
        let same = match (&self.shell_rust.subtitle_bounds, bounds) {
            (None, None) => true,
            (Some((old_app, old_surface, old_rect)), Some((app, surface, rect))) =>
                old_app == app && *old_surface == surface && *old_rect == rect,
            _ => false,
        };
        if same && self.shell_rust.topic("subtitleBounds").is_some() {
            return;
        }
        let value = match bounds {
            Some((app, surface, rect)) => json!({"app": app, "surface": surface.0, "rect": rect.map(|r| json!({"x": r.x, "y": r.y, "w": r.w, "h": r.h}))}),
            None => Value::Null,
        };
        self.shell_rust.subtitle_bounds = bounds.map(|(app, surface, rect)| (app.to_owned(), surface, rect));
        self.shell_rust.push("subtitleBounds", value);
    }


    /// The shared topics plus the Rust-only foreground video identity.
    pub fn shell_rust_sync_topics(&mut self) {
        if !self.shell_rust.enabled() {
            return;
        }
        for (topic, mut value) in self.shell_topic_values() {
            if topic == "foreground" {
                let video = self.foreground_video();
                let surface = self.shell_rust_foreground_surface();
                let unchanged = self.shell_rust.topic("foreground").and_then(Value::as_object).zip(value.as_object()).is_some_and(|(old, base)| {
                    old.len() == base.len() + 2 && base.iter().all(|(key, v)| old.get(key) == Some(v))
                        && match (old.get("surface"), surface) {
                            (Some(Value::Null), None) => true,
                            (Some(v), Some(id)) => v.as_u64() == Some(id as u64),
                            _ => false,
                        }
                        && match (old.get("video"), video) {
                        (Some(Value::Null), None) => true,
                        (Some(v), Some((context, app))) => v.get("context").and_then(Value::as_str) == Some(context) && v.get("app").and_then(Value::as_str) == Some(app),
                        _ => false,
                    }
                });
                if unchanged { continue; }
                value["video"] = match video {
                    Some((context, app)) => json!({"context": context, "app": app}),
                    None => Value::Null,
                };
                value["surface"] = json!(surface);
            }
            self.shell_rust.push_if_changed(&topic, value);
        }
        if self.shell_rust.answers("switcher") {
            self.switcher_sync_topics();
        }
        self.shell_rust_sync_subtitle_bounds();
    }

    /// Whether the Rust client's layer has the keys: the keys go over the key socket (`input.rs`).
    pub fn shell_rust_wants_keys(&self) -> bool {
        // while an app's card is growing over Home (Home is held as its underlay) the app has the keys: Home is only drawn, not the front
        self.shell_rust.connected && self.shell_rust.wants && self.shell_rust.keys.is_some() && !self.scene.transitions.holds_home()
    }

    /// Whether the Rust client's saver is up (its state says so): every key of the remote wakes it, the ones the key socket has no name for too.
    pub(crate) fn shell_rust_saver_up(&self) -> bool {
        self.shell_rust.answers("saver") && self.shell_rust.ui_state.get("saver").and_then(Value::as_bool) == Some(true)
    }

    /// One key of the remote for the Rust client, before the focus routing: its layer wants the keys (every key but the compositor's
    /// own goes to it), or its cards take OK and Back by name while an app has the focus (`takes`, as the QML client's). Returns
    /// whether the key went to it.
    pub fn shell_rust_takes_key(&mut self, seat: crate::server::seat::SeatId, code: u32, pressed: bool, swallowed: bool, now: Instant) -> bool {
        if let Some(i) = self.shell_rust.took.iter().position(|(c, _, _)| *c == code) {
            let (_, name, last) = self.shell_rust.took[i];
            if !pressed {
                self.shell_rust.took.remove(i);
                self.shell_rust.send_key(name, false, false, seat.0);
                self.keys.delivered += 1;
                return true;
            }
            if now.saturating_duration_since(last) < TOOK_STALE {
                self.shell_rust.took[i].2 = now;
                self.shell_rust.send_key(name, true, true, seat.0);
                self.keys.delivered += 1;
                return true;
            }
            self.shell_rust.took.remove(i);
        }
        // Taught keys normally go only to a client that lists them; an active saver takes every key.
        let taught = if crate::server::ime::own_key_name(code).is_none() { self.keys.router.table.shell_name(code) } else { None };
        let name = match crate::server::ime::own_key_name(code).or(taught) {
            Some(n) => n,
            None if self.shell_rust_saver_up() => "any",
            None => return false,
        };
        if !pressed || swallowed || !self.shell_rust.connected {
            return false;
        }
        let wants = self.shell_rust_wants_keys();
        let takes = self.shell_rust.takes.iter().any(|n| n == name);
        if (!wants && !takes) || (taught.is_some() && !takes && !(wants && self.shell_rust_saver_up())) {
            return false;
        }
        if takes && !wants {
            // Only an app's focus is the layer's to share.
            let focused = self.seat.focus.and_then(|s| self.scene.window_of(s)).and_then(|w| self.scene.window(w));
            if focused.is_none() || self.seat.pressed.contains(&(seat, code)) {
                return false;
            }
        }
        self.shell_rust.took.push((code, name, now));
        self.shell_rust.send_key(name, true, false, seat.0);
        self.keys.delivered += 1;
        true
    }

    /// A turn of the Magic Remote's wheel for the Rust client, as the key `wheel` pressed and released at once. It goes only while the client's layer
    /// wants the keys and lists `wheel` in its `takes` (the display lab switches between its two looks with it); a wheel nobody asked for is the
    /// pointer's alone. Returns whether the client got it.
    pub fn shell_rust_wheel(&mut self) -> bool {
        if !self.shell_rust_wants_keys() || !self.shell_rust.takes.iter().any(|n| n == "wheel") {
            return false;
        }
        let sent = self.shell_rust.send_key("wheel", true, false, 0);
        self.shell_rust.send_key("wheel", false, false, 0);
        if sent {
            self.keys.delivered += 1;
        }
        sent
    }

    /// A notch of the wheel with its direction for the Rust client, as the key `wheelup` or `wheeldown` pressed and released at once, under the
    /// same rules as [`State::shell_rust_wheel`]: only while the client's layer wants the keys and lists that name in its `takes` (Home's
    /// smart-home card scrolls with it, under the pointer). Returns whether the client got it.
    pub fn shell_rust_wheel_direction(&mut self, down: bool) -> bool {
        let name = if down { "wheeldown" } else { "wheelup" };
        if !self.shell_rust_wants_keys() || !self.shell_rust.takes.iter().any(|n| n == name) {
            return false;
        }
        let sent = self.shell_rust.send_key(name, true, false, 0);
        self.shell_rust.send_key(name, false, false, 0);
        if sent {
            self.keys.delivered += 1;
        }
        sent
    }

    /// A pointer sample for the Rust client (the caller decides that it has the pointer: [`State::shell_rust_pointer_sample`]).
    pub fn shell_rust_pointer(&mut self, x: f32, y: f32, buttons: u32, visible: bool) {
        self.shell_rust.send_pointer(x, y, buttons, visible);
    }

    /// Whether the Rust client has the pointer at (`x`, `y`): its layer wants the keys (Home in front, a panel up), or the point is on the sheet of
    /// its keyboard (the apps' fields: the app keeps the keys, the sheet is the client's).
    pub fn shell_rust_owns_pointer(&self, x: i32, y: i32) -> bool {
        self.shell_rust.connected
            && self.shell_rust.keys.is_some()
            && (self.shell_rust.wants || (self.shell_rust.answers("keyboard") && self.scene.shell_pointer.is_some_and(|r| r.contains(x, y))))
    }

    /// The pointer is at (`x`, `y`), shown or not: the client that has it gets the sample, and the client that had it is told once that it
    /// went. Returns whether the Rust client has it (the app under it hears nothing).
    pub fn shell_rust_pointer_sample(&mut self, x: i32, y: i32, visible: bool) -> bool {
        let owns = visible && self.shell_rust_owns_pointer(x, y);
        if owns {
            self.shell_rust.pointer_in = true;
            self.shell_rust.pointer_at = (x as f32, y as f32);
            let buttons = self.shell_rust.buttons;
            self.shell_rust.send_pointer(x as f32, y as f32, buttons, true);
        } else if self.shell_rust.pointer_in {
            self.shell_rust.pointer_in = false;
            self.shell_rust.buttons = 0;
            self.shell_rust.send_pointer(x as f32, y as f32, 0, false);
        }
        owns
    }

    /// A press was sent to the Rust client and its release is owed.
    pub fn shell_rust_button_down(&self) -> bool {
        self.shell_rust.buttons != 0
    }

    /// The remote's OK (or a button) while the pointer is over the Rust client's layers: its click. A release goes to it only for a press that did
    /// (the pointer left, or was hidden, in between: `buttons` is cleared with it), and a press only while the pointer is shown over it.
    pub fn shell_rust_pointer_button(&mut self, pressed: bool) -> bool {
        if !pressed && self.shell_rust.buttons == 0 {
            return false;
        }
        if !self.shell_rust.pointer_in || (pressed && !self.seat.cursor_visible) {
            return false;
        }
        self.shell_rust.buttons = u32::from(pressed);
        let (x, y) = self.shell_rust.pointer_at;
        self.shell_rust.send_pointer(x, y, self.shell_rust.buttons, true);
        true
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
            "frameReady": s.frame_ready,
            "ready": s.ready(),
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

    /// A native channel without starting a child.
    fn state_with(kinds: &str) -> (State, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lumatv-shell-rust-{}-{}", std::process::id(), kinds.len()));
        std::fs::create_dir_all(&dir).unwrap();
        let display: wayland_server::Display<State> = wayland_server::Display::new().unwrap();
        let rust_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = rust_listener.local_addr().unwrap().port();
        let config = crate::config::Config {
            data_dir: dir.clone(),
            runtime_dir: dir.clone(),
            shell_port: port,
            shell_rust: parse_kinds(kinds),
            ..crate::config::Config::default()
        };
        let mut state = State::new(display.handle(), config, Box::new(crate::render::software::Software::new(64, 64)), Box::new(crate::luna::Null::default()));
        state.shell_rust.link = Some(spawn_server(rust_listener, "rust".into()).unwrap());
        state.shell_rust.kinds = parse_kinds(kinds);
        state.shell_rust.marker = dir.join("marker");
        (state, dir)
    }

    #[test]
    fn engine_attach_clears_early_report_and_requests_a_fresh_one() {
        let (mut state, _dir) = state_with("toast");
        let report = || Msg::Ui { can: None, wants: None, live: None, state: Some(json!({"interactiveVisible": true})) };
        state.shell_rust_handle(report());
        assert_eq!(state.config.refresh_hz, 60);
        state.shell_rust_engine_connected();
        assert!(state.shell_rust.ui_state.is_null());
        assert_eq!(state.shell_rust.topic("refresh"), Some(&json!({"hz": 60})));
        state.shell_rust_handle(report());
        assert_eq!(state.config.refresh_hz, 120);
    }

    #[test]
    fn stale_and_restarted_clients_clear_the_interactive_refresh_vote() {
        let (mut state, _dir) = state_with("toast");
        let report = || Msg::Ui { can: None, wants: None, live: None, state: Some(json!({"interactiveVisible": true})) };
        state.shell_rust.engine_connected = true;
        state.shell_rust_handle(report());
        assert_eq!(state.config.refresh_hz, 120);
        state.shell_rust.last_seen = Some(Instant::now() - POLL_TIMEOUT - Duration::from_secs(1));
        state.shell_rust_tick(Instant::now());
        assert_eq!(state.config.refresh_hz, 60);
        assert!(state.shell_rust.ui_state.is_null());
        state.shell_rust.engine_connected = true;
        state.shell_rust_handle(Msg::Connected);
        state.refresh_policy_sync();
        assert_eq!(state.config.refresh_hz, 60);
        state.shell_rust_handle(report());
        assert_eq!(state.config.refresh_hz, 120);
        state.shell_rust_restart();
        assert_eq!(state.config.refresh_hz, 60);
        assert!(state.shell_rust.ui_state.is_null());
    }

    #[test]
    fn supervised_client_that_stops_polling_requests_stock_recovery() {
        let (mut state, _dir) = state_with("home,keyboard");
        state.shell_rust.launch = Some(Launch { binary: PathBuf::from("unused"), loader: None });
        state.shell_rust_handle(Msg::Connected);
        state.shell_rust.last_seen = Some(Instant::now() - POLL_TIMEOUT - Duration::from_secs(1));
        state.shell_rust_tick(Instant::now());
        assert!(state.shell_rust.gave_up);
        assert_eq!(state.shell_rust.error.as_deref(), Some("UI stopped polling"));
        assert_eq!(state.shell_rust_failure("x").as_deref(), Some("UI stopped polling"), "which ends the compositor, and the supervisor falls back");
    }

    #[test]
    fn a_client_that_exits_ends_the_compositor_and_one_that_runs_does_not() {
        let (mut state, _dir) = state_with("home,keyboard");
        state.shell_rust.launch = Some(Launch { binary: PathBuf::from("unused"), loader: None });
        assert_eq!(state.shell_rust_failure("x"), None, "nothing is wrong before the client has done anything");
        state.shell_rust.child = Some(Command::new("sleep").arg("120").spawn().unwrap());
        state.shell_rust_tick(Instant::now());
        assert_eq!(state.shell_rust_failure("x"), None, "a client that runs is not a failure");
        // the client dies (a crash, a kill)
        let mut child = state.shell_rust.child.take().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        state.shell_rust.child = Some(Command::new("true").spawn().unwrap());
        let mut at = Instant::now();
        for _ in 0..200 {
            at += Duration::from_millis(600);
            state.shell_rust_tick(at);
            if state.shell_rust.gave_up {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let error = state.shell_rust_failure("x").expect("the client exited");
        assert!(error.starts_with("client exited"), "{error}");
        assert!(state.shell_rust.child.is_none() && !state.shell_rust.connected);
        assert_eq!(state.shell_rust_failure("x").as_deref(), Some(error.as_str()), "and it stays so: nothing restarts the client by itself");
        // the same with no reason kept
        state.shell_rust.error = None;
        assert_eq!(state.shell_rust_failure("native UI failed").as_deref(), Some("native UI failed"));
    }

    #[test]
    fn readiness_requires_every_requested_kind_and_a_presented_commit() {
        let (mut state, _) = state_with("home,keyboard");
        state.shell_rust_handle(Msg::Connected);
        state.shell_rust_engine_connected();
        state.shell_rust_handle(Msg::Ui { can: Some(vec!["home".into(), "keyboard".into()]), wants: None, live: None, state: Some(json!({"kinds": ["home"]})) });
        assert!(!state.shell_rust.ready());
        state.shell_rust_handle(Msg::FrameReady);
        assert!(!state.shell_rust.ready());
        state.shell_rust_handle(Msg::Ui { can: Some(vec![]), wants: None, live: None, state: Some(json!({"kinds": ["home", "keyboard"]})) });
        assert!(state.shell_rust.ready(), "temporary availability does not define support");
        state.shell_rust_client_gone("test");
        assert!(!state.shell_rust.ready());
        assert!(!state.shell_rust.frame_ready);
        assert!(state.shell_rust.supported.is_empty());
    }

    #[test]
    fn quiet_long_poll_is_not_mistaken_for_a_dead_client() {
        let (mut state, _) = state_with("home");
        state.shell_rust.launch = Some(Launch { binary: PathBuf::from("unused"), loader: None });
        state.shell_rust_handle(Msg::Connected);
        state.shell_rust.last_seen = Some(Instant::now() - Duration::from_secs(65));
        state.shell_rust_tick(Instant::now());
        assert!(!state.shell_rust.gave_up);
        assert!(state.shell_rust.connected);
    }

    #[test]
    fn startup_deadline_waits_through_standby_then_expires_on_wake() {
        let (mut state, _) = state_with("home");
        state.shell_rust.launch = Some(Launch { binary: PathBuf::from("unused"), loader: None });
        state.shell_rust.child = Some(Command::new("sleep").arg("120").spawn().unwrap());
        state.shell_rust.engine_connected = true;
        let now = Instant::now();
        state.shell_rust.child_started = Some(now - READY_TIMEOUT - Duration::from_secs(1));
        state.luna_state.power.rendering = false;
        state.shell_rust_tick(now);
        assert!(!state.shell_rust.gave_up);
        state.luna_state.power.rendering = true;
        let wake = now + Duration::from_secs(180);
        state.shell_rust_tick(wake);
        assert!(!state.shell_rust.gave_up);
        state.shell_rust_tick(wake + READY_TIMEOUT + Duration::from_secs(1));
        assert_eq!(state.shell_rust.error.as_deref(), Some("UI did not become ready"));
    }

    #[test]
    fn polling_deadline_restarts_on_wake() {
        let (mut state, _) = state_with("home");
        state.shell_rust.launch = Some(Launch { binary: PathBuf::from("unused"), loader: None });
        state.shell_rust_handle(Msg::Connected);
        let now = Instant::now();
        state.luna_state.power.rendering = false;
        state.shell_rust_tick(now);
        state.luna_state.power.rendering = true;
        let wake = now + Duration::from_secs(180);
        state.shell_rust_tick(wake);
        assert!(!state.shell_rust.gave_up);
        state.shell_rust_tick(wake + POLL_TIMEOUT + Duration::from_secs(1));
        assert_eq!(state.shell_rust.error.as_deref(), Some("UI stopped polling"));
    }

    #[test]
    fn repeated_would_block_is_bounded() {
        let mut shell = RustShell::default();
        let now = Instant::now();
        assert!(!shell.attach_timed_out(now));
        assert!(!shell.attach_timed_out(now + READY_TIMEOUT));
        assert!(shell.attach_timed_out(now + READY_TIMEOUT + Duration::from_millis(1)));
    }


    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumatv-shell-rust-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: &Path, text: &str, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// The set's layout: the compositor and the client in `bin/`, the loader in `lib/`.
    fn bundle(name: &str) -> PathBuf {
        let root = scratch(name);
        write_file(&root.join("bin/lumatv-compositor"), "", 0o755);
        write_file(&root.join("bin/lumatv-ui"), "", 0o755);
        write_file(&root.join("lib/ld-linux-aarch64.so.1"), "", 0o755);
        root
    }

    fn cmdline(words: &[&dyn AsRef<std::ffi::OsStr>]) -> Vec<u8> {
        words.iter().flat_map(|w| w.as_ref().as_bytes().iter().copied().chain([0])).collect()
    }

    #[test]
    fn started_through_the_loader_the_client_is_found_in_bin_and_run_through_the_same_loader() {
        let root = bundle("indirect");
        let loader = root.join("lib/ld-linux-aarch64.so.1");
        let argv = cmdline(&[&loader, &"--library-path", &root.join("lib"), &root.join("bin/lumatv-compositor"), &"--runtime-dir", &"/tmp/xdg", &"--input"]);
        // /proc/self/exe is the loader, as under lumatv-sm: lib/ has no lumatv-ui
        let launch = find_launch_with(None, None, None, &loader, &argv).expect("found");
        let want = Launch {
            binary: std::fs::canonicalize(root.join("bin/lumatv-ui")).unwrap(),
            loader: Some(Loader { path: loader.clone(), lib: root.join("lib") }),
        };
        assert_eq!(launch, want);
        let cmd = launch.command();
        assert_eq!(cmd.get_program(), loader.as_os_str());
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, [std::ffi::OsStr::new("--library-path"), root.join("lib").as_os_str(), want.binary.as_os_str()]);

        // the loader's own options before the program are skipped
        let argv = cmdline(&[&loader, &"--preload", &"heapshim.so", &"--library-path", &root.join("lib"), &root.join("bin/lumatv-compositor")]);
        assert_eq!(find_launch_with(None, None, None, &loader, &argv), Some(want));
        // no library path: the loader's own directory
        let argv = cmdline(&[&loader, &root.join("bin/lumatv-compositor")]);
        assert_eq!(through_loader(&loader, &argv).unwrap().0.lib, root.join("lib"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_directly_the_client_is_beside_the_executable_and_exec_d_without_a_loader() {
        let root = bundle("direct");
        let exe = root.join("bin/lumatv-compositor");
        let launch = find_launch_with(None, None, None, &exe, &cmdline(&[&exe, &"--input"])).expect("found");
        assert_eq!(launch, Launch { binary: root.join("bin/lumatv-ui"), loader: None });
        assert_eq!(launch.command().get_program(), root.join("bin/lumatv-ui").as_os_str());
        assert_eq!(launch.command().get_args().count(), 0);
        // the overrides win; a client or a loader that is not there is no launch
        let other = root.join("lib/other-ui");
        write_file(&other, "", 0o755);
        assert_eq!(find_launch_with(Some(other.clone()), None, None, &exe, &[]).unwrap().binary, other);
        std::fs::remove_file(root.join("bin/lumatv-ui")).unwrap();
        assert_eq!(find_launch_with(None, None, None, &exe, &[]), None);
        assert_eq!(find_launch_with(Some(other.clone()), Some(root.join("lib/gone")), None, &exe, &[]), None);
        let _ = std::fs::remove_dir_all(&root);
    }


    #[test]
    fn the_audio_topic_goes_to_the_shell_that_answers_osd_and_the_toast_feed_likewise() {
        let (mut state, _dir) = state_with("osd,toast");
        state.shell_rust_handle(Msg::Connected);
        state.shell_audio(&json!({"volumeStatus": {"volume": 3}}));
        assert_eq!(state.shell_rust.topic("audio").unwrap()["volumeStatus"]["volume"], 3);
        state.shell_rust_handle(Msg::Luna {
            tag: 9,
            uri: "luna://com.webos.notification/getToastNotification".into(),
            payload: json!({"subscribe": true}),
            subscribe: true,
        });
        let rust_token = state.shell_rust.luna_tags[0].token;
        assert!(state.shell_luna_event(rust_token, &json!({"message": "Remote battery low"})));
        assert_eq!(state.shell_rust.topic("luna").unwrap()["tag"], 9);
        assert!(state.journal.iter().any(|l| l.contains("shell-rust: toast delivered")));
    }

    #[test]
    fn native_alert_receives_all_three_notification_feeds() {
        let (mut state, _dir) = state_with("alert");
        state.shell_rust_handle(Msg::Connected);
        let feeds =
            [("getAlertNotification", "alertAction"), ("getInputAlertNotification", "alertAction"), ("getPincodePromptNotification", "pincodePromptAction")];
        for (i, (feed, _)) in feeds.iter().enumerate() {
            let uri = format!("luna://com.webos.notification/{feed}");
            state.shell_rust_handle(Msg::Luna { tag: 20 + i as u64, uri, payload: json!({"subscribe": true}), subscribe: true });
        }
        for (i, (_, action)) in feeds.iter().enumerate() {
            let rust = state.shell_rust.luna_tags.iter().find(|t| t.tag == 20 + i as u64).unwrap().token;
            assert!(state.shell_luna_event(rust, &json!({*action: "open"})));
            assert_eq!(state.shell_rust.topic("luna").unwrap()["tag"], 20 + i as u64, "and reaches the Rust client");
        }
    }

    #[test]
    fn native_saver_vote_and_care_follow_the_client() {
        let (mut state, _dir) = state_with("saver");
        state.shell_rust_handle(Msg::Connected);
        let vote = "luna://com.webos.service.tvpower/power2/registerScreenSaverRequest";
        state.shell_rust_handle(Msg::Luna { tag: 32, uri: vote.into(), payload: json!({"clientName": "rust", "subscribe": true}), subscribe: true });
        assert_eq!(state.shell_rust.luna_tags.len(), 1, "the Rust client's vote goes through");
        // the OLED care stands aside for the saver the Rust client says is up
        state.shell_rust_handle(Msg::Ui { can: Some(vec![]), wants: Some(true), live: Some(true), state: Some(json!({"saver": true})) });
        state.tick();
        assert!(state.scene.care.saver, "the Rust shell's saver is the saver");
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(false), live: Some(false), state: Some(json!({"saver": false})) });
        state.tick();
        assert!(!state.scene.care.saver);
    }

    #[test]
    fn native_client_is_told_whether_the_panel_carries_saver_dim() {
        let (mut state, _dir) = state_with("saver");
        state.shell_rust_handle(Msg::Connected);
        // no luna bridge on the set (the null one): the panel cannot be asked, the saver dims its own picture
        state.tick();
        assert_eq!(state.shell_rust.topic("paneldim").unwrap()["carry"], false);
        // a bridge that is up: the panel carries, and the client is told
        state.panel.set_bridge(true);
        state.panel_dim_tick(crate::output::monotonic_ns());
        assert_eq!(state.shell_rust.topic("paneldim").unwrap()["carry"], true);
        // the switch off: it is the saver's again
        state.panel.settings.enabled = false;
        state.panel_dim_tick(crate::output::monotonic_ns());
        assert_eq!(state.shell_rust.topic("paneldim").unwrap()["carry"], false);
        // the saver's level counts only when it says it does not draw the dim itself
        state.shell_rust.ui_state = json!({"saver": true, "saverState": {"level": 0.5, "carry": false}});
        state.scene.care.saver = true;
        assert_eq!(state.panel_want(), 1.0);
        state.shell_rust.ui_state = json!({"saver": true, "saverState": {"level": 0.5, "carry": true}});
        assert_eq!(state.panel_want(), 0.5);
    }

    #[test]
    fn native_capture_topic_and_dot_follow_the_client() {
        let (mut state, _dir) = state_with("capture");
        state.shell_rust_handle(Msg::Connected);
        state.screencap_publish();
        assert!(state.shell_rust.topic("capture").is_some(), "the Rust client is told");
        assert!(!state.screencap_dot_up());
        state.shell_rust_handle(Msg::Ui { can: None, wants: None, live: None, state: Some(json!({"capture": {"dot": true}})) });
        assert!(state.screencap_dot_up());
    }


    #[test]
    fn subtitle_unknown_is_published_once_and_replayed_on_reconnect() {
        let (mut state, _dir) = state_with("popup");
        state.shell_rust_handle(Msg::Connected);
        assert_eq!(state.shell_rust.topic("subtitleBounds"), Some(&Value::Null));
        assert_eq!(state.shell_rust.topic("foreground").unwrap()["surface"], Value::Null);
        let before = state.shell_rust.events().len();
        state.shell_rust_sync_subtitle_bounds();
        assert_eq!(state.shell_rust.events().len(), before);
        state.shell_rust.pushed.clear();
        state.shell_rust_sync_subtitle_bounds();
        assert_eq!(state.shell_rust.topic("subtitleBounds"), Some(&Value::Null));
        assert_eq!(state.shell_rust.events().len(), before + 1);
    }


    #[test]
    fn only_native_kinds_can_open_and_receive_topics() {
        let (mut state, _dir) = state_with("osd");
        state.shell_rust_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Ui { can: Some(vec![]), wants: Some(false), live: Some(false), state: Some(json!({"osd": false})) });
        assert!(!state.shell_can("quick"));
        assert!(!state.shell_show("quick", false));
        assert!(!state.shell_can("osd"));
        state.shell_sync_topics();
        for topic in ["foreground", "screen", "cursor", "refresh", "wall", "transitions", "keyboard", "subtitleBounds"] {
            assert!(state.shell_rust.topic(topic).is_some(), "{topic} for native UI");
        }
        let report = state.shell_rust_report();
        assert_eq!(report["kinds"], json!(["osd"]));
        assert_eq!(report["connected"], true);
        assert_eq!(report["ui"]["osd"], false);
    }

    #[test]
    fn native_layout_editor_opens_only_when_available() {
        let (mut state, _dir) = state_with("cards,popup,layout");
        state.shell_rust_handle(Msg::Connected);
        assert!(!state.shell_show_page("layout", false, Some("popup")));
        state.shell_rust_handle(Msg::Ui { can: Some(vec!["layout".into()]), wants: Some(false), live: Some(false), state: Some(json!({"layout": {"open": false}})) });
        assert!(state.shell_show_page("layout", false, Some("popup")));
        let show = state.shell_rust.topic("show").unwrap();
        assert_eq!((show["kind"].as_str(), show["page"].as_str()), (Some("layout"), Some("popup")));
    }

    #[test]
    fn the_switcher_is_drawn_by_the_client_that_answers_it_and_only_that_one_hears_of_it() {
        let (mut state, _dir) = state_with("switcher");
        state.shell_rust_handle(Msg::Connected);
        assert!(!state.shell_can("switcher"));
        state.shell_rust_handle(Msg::Ui { can: Some(vec!["switcher".into()]), wants: Some(false), live: Some(false), state: None });
        assert!(state.shell_can("switcher"));
        state.shell_sync_topics();
        assert_eq!(state.shell_rust.topic("switcher").unwrap()["open"], false);
        assert_eq!(state.shell_rust.topic("apps"), Some(&json!([])));
        // its request to open one is the compositor's to judge: with no app running nothing opens
        assert!(!state.shell_rust_handle(Msg::Switcher(json!({"op": "open"}))));
        assert_eq!(state.switcher_report()["open"], false);
        // and a client that does not answer the kind cannot drive it
        let (mut qml, _dir2) = state_with("osd");
        qml.shell_rust_handle(Msg::Connected);
        qml.shell_sync_topics();
        assert!(qml.shell_rust.topic("switcher").is_none(), "a kind that is not its own gets no topics");
    }

    #[test]
    fn the_home_key_reaches_the_rust_client_while_its_home_is_drawn_with_nothing_focused() {
        let (mut state, _dir) = state_with("home,switcher");
        state.shell_rust_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Ui { can: Some(vec!["switcher".into()]), wants: Some(false), live: Some(false), state: None });
        let seat = crate::server::seat::SeatId(0);
        let tap = |state: &mut State, at: Instant| {
            state.key_event_at(seat, crate::input::code::KEY_HOME, true, false, at);
            state.key_event_at(seat, crate::input::code::KEY_HOME, false, false, at + Duration::from_millis(60));
        };
        assert!(!state.shell_hears_home_key(), "Home is not drawn: nothing to tell");
        let now = Instant::now();
        tap(&mut state, now);
        assert!(state.shell_rust.topic("homekey").is_none(), "an app has the screen: the Home key is the compositor's");
        state.scene.set_home_rust(true);
        assert!(state.shell_hears_home_key(), "Home is drawn by the Rust shell, which has no window to focus");
        tap(&mut state, now + Duration::from_secs(5));
        assert!(state.shell_rust.topic("homekey").is_some(), "the Rust client is told of the Home key, which may open the switcher");
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
    fn a_turn_of_the_wheel_is_the_key_wheel_for_the_client_only_while_its_layer_wants_the_keys_and_lists_it() {
        let (mut state, _dir) = state_with("displaylab");
        state.shell_rust_handle(Msg::Connected);
        let (ours, theirs) = UnixStream::pair().unwrap();
        ours.set_nonblocking(true).unwrap();
        state.shell_rust.keys = Some(ours);
        // the lab is open and wants the keys, but has not asked for the wheel: the turn is the pointer's alone
        state.shell_rust_handle(Msg::Ui { can: Some(vec!["displaylab".into()]), wants: Some(true), live: Some(true), state: None });
        assert!(!state.shell_rust_wheel(), "not listed");
        crate::server::seat::wheel_turn(&mut state);
        state.shell_rust_handle(Msg::Takes(vec!["wheel".into(), "menu".into()]));
        assert_eq!(state.shell_rust.takes, vec!["wheel"], "the wheel is takeable, a name nobody taught is not");
        assert!(state.shell_rust_wheel(), "listed, and the layer has the keys: the turn is the lab's");
        // a turn of the wheel the control socket makes is the same turn
        crate::server::seat::wheel_turn(&mut state);
        // the layer lets go of the keys (the lab closed): listed or not, the turn is not sent
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(false), live: None, state: None });
        assert!(!state.shell_rust_wheel());
        let mut reader = theirs;
        reader.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut text = String::new();
        let mut buf = [0u8; 1024];
        let keys = |text: &str| -> Vec<Value> { text.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).filter(|l| l.get("key").is_some()).collect() };
        while keys(&text).len() < 4 {
            let n = reader.read(&mut buf).unwrap();
            text.push_str(std::str::from_utf8(&buf[..n]).unwrap());
        }
        // the pointer the first turn showed is on the socket too, and is not what is looked at
        let lines = keys(&text);
        assert_eq!(lines.len(), 4, "two turns, each pressed and released: {text}");
        assert_eq!(lines[0], json!({"key": {"k": "wheel", "d": true, "r": false, "seat": 0}}));
        assert_eq!(lines[1], json!({"key": {"k": "wheel", "d": false, "r": false, "seat": 0}}));
        assert_eq!(lines[2..], lines[..2]);
    }

    #[test]
    fn a_turn_of_the_wheel_is_wheelup_or_wheeldown_for_the_client_that_lists_that_name_and_only_while_its_layer_wants_the_keys() {
        let (mut state, _dir) = state_with("home");
        state.shell_rust_handle(Msg::Connected);
        let (ours, theirs) = UnixStream::pair().unwrap();
        ours.set_nonblocking(true).unwrap();
        state.shell_rust.keys = Some(ours);
        // Home is in front and wants the keys, but has not asked for the turns: they are the pointer's alone
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: None });
        assert!(!state.shell_rust_wheel_direction(true) && !state.shell_rust_wheel_direction(false), "not listed");
        state.shell_rust_handle(Msg::Takes(vec!["wheelup".into(), "wheeldown".into(), "wheelsideways".into()]));
        assert_eq!(state.shell_rust.takes, vec!["wheelup", "wheeldown"], "both ways are takeable, a name nobody taught is not");
        assert!(state.shell_rust_wheel_direction(true), "a turn towards the person");
        assert!(state.shell_rust_wheel_direction(false), "a turn away");
        // the plain `wheel` is not what a client that lists only the directions hears
        assert!(!state.shell_rust_wheel());
        // only one way listed: the other is the pointer's alone
        state.shell_rust_handle(Msg::Takes(vec!["wheeldown".into()]));
        assert!(!state.shell_rust_wheel_direction(false));
        // the layer lets go of the keys: listed or not, the turn is not sent
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(false), live: None, state: None });
        assert!(!state.shell_rust_wheel_direction(true));
        let mut reader = theirs;
        reader.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut text = String::new();
        let mut buf = [0u8; 1024];
        let keys = |text: &str| -> Vec<Value> { text.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).filter(|l| l.get("key").is_some()).collect() };
        while keys(&text).len() < 4 {
            let n = reader.read(&mut buf).unwrap();
            text.push_str(std::str::from_utf8(&buf[..n]).unwrap());
        }
        let lines = keys(&text);
        assert_eq!(lines.len(), 4, "two turns, each pressed and released: {text}");
        assert_eq!(lines[0], json!({"key": {"k": "wheeldown", "d": true, "r": false, "seat": 0}}));
        assert_eq!(lines[1], json!({"key": {"k": "wheeldown", "d": false, "r": false, "seat": 0}}));
        assert_eq!(lines[2], json!({"key": {"k": "wheelup", "d": true, "r": false, "seat": 0}}));
        assert_eq!(lines[3], json!({"key": {"k": "wheelup", "d": false, "r": false, "seat": 0}}));
    }

    #[test]
    fn a_key_keys_conf_taught_the_shell_by_name_is_sent_only_while_the_client_lists_it_and_its_release_with_it() {
        let (mut state, _dir) = state_with("osd");
        state.shell_rust_handle(Msg::Connected);
        let (ours, theirs) = UnixStream::pair().unwrap();
        ours.set_nonblocking(true).unwrap();
        state.shell_rust.keys = Some(ours);
        state.keys.router.table = crate::keys::KeyTable::parse("voice 1050\nsearch 217\n").0;
        let seat = crate::server::seat::SeatId(0);
        let now = Instant::now();
        // Home is in front: the layer wants every key. The mic key is not listed (the helper cannot listen): it stays LG's, and so does a code nobody named
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: None });
        assert!(!state.shell_rust_takes_key(seat, 1050, true, false, now), "not listed");
        assert!(!state.shell_rust_takes_key(seat, 1051, true, false, now), "no name at all");
        state.shell_rust_handle(Msg::Takes(vec!["voice".into(), "search".into(), "menu".into()]));
        assert_eq!(state.shell_rust.takes, vec!["voice", "search"], "the names are takeable, a name nobody taught is not");
        assert!(state.shell_rust_takes_key(seat, 1050, true, false, now), "listed: the press is the shell's");
        assert!(state.shell_rust_takes_key(seat, 1050, false, false, now + Duration::from_millis(900)), "and the release, which is the end of the question");
        assert!(state.shell_rust_takes_key(seat, 217, true, false, now), "Search too");
        // it stops listing the mic key (the helper went away) between a press and its release: the release still reaches it
        assert!(state.shell_rust_takes_key(seat, 1050, true, false, now + Duration::from_secs(2)));
        state.shell_rust_handle(Msg::Takes(vec!["search".into()]));
        assert!(state.shell_rust_takes_key(seat, 1050, false, false, now + Duration::from_secs(3)));
        assert!(!state.shell_rust_takes_key(seat, 1050, true, false, now + Duration::from_secs(4)), "and the next press is LG's again");
        let mut reader = theirs;
        reader.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut buf = [0u8; 2048];
        let n = reader.read(&mut buf).unwrap();
        let names: Vec<String> = std::str::from_utf8(&buf[..n]).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["key"]["k"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, vec!["voice", "voice", "search", "voice", "voice"]);
    }

    #[test]
    fn the_channel_rocker_and_info_are_the_guides_only_while_it_lists_them_and_the_release_goes_with_the_press() {
        let (mut state, _dir) = state_with("guide");
        state.shell_rust_handle(Msg::Connected);
        let (ours, theirs) = UnixStream::pair().unwrap();
        ours.set_nonblocking(true).unwrap();
        state.shell_rust.keys = Some(ours);
        let seat = crate::server::seat::SeatId(0);
        let now = Instant::now();
        let (up, down, info) = (crate::input::code::KEY_CHANNELUP, crate::input::code::KEY_CHANNELDOWN, crate::input::code::KEY_INFO);
        // the guide is up and wants the keys, but has not listed these: they are Live TV's
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: None });
        assert!(!state.shell_rust_takes_key(seat, up, true, false, now));
        state.shell_rust_handle(Msg::Takes(vec!["chup".into(), "chdown".into(), "info".into(), "list".into()]));
        assert_eq!(state.shell_rust.takes, vec!["chup", "chdown", "info"], "the names are takeable, a name nobody gave is not");
        for (code, at) in [(up, 10), (down, 20), (info, 30)] {
            assert!(state.shell_rust_takes_key(seat, code, true, false, now + Duration::from_millis(at)));
            assert!(state.shell_rust_takes_key(seat, code, false, false, now + Duration::from_millis(at + 5)), "the release goes with the press");
        }
        // the guide closes: the layer lets go of the keys and the channel rocker is Live TV's again
        state.shell_rust_handle(Msg::Takes(Vec::new()));
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(false), live: None, state: None });
        assert!(!state.shell_rust_takes_key(seat, up, true, false, now + Duration::from_secs(1)));
        let mut reader = theirs;
        reader.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut buf = [0u8; 2048];
        let n = reader.read(&mut buf).unwrap();
        let names: Vec<String> = std::str::from_utf8(&buf[..n]).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["key"]["k"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, vec!["chup", "chup", "chdown", "chdown", "info", "info"]);
    }

    #[test]
    fn taught_keys_wake_the_saver_without_changing_other_layers_key_choices() {
        let (mut state, _dir) = state_with("saver");
        state.shell_rust_handle(Msg::Connected);
        let mut reader = with_key_socket(&mut state);
        state.keys.router.table = crate::keys::KeyTable::parse("voice 1050\nsearch 1051\nscreenremote 1052\n").0;
        let seat = crate::server::seat::SeatId(0);
        let now = Instant::now();
        let keys = [(1050, "voice"), (1051, "search"), (1052, "screenremote")];
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: Some(json!({"saver": false})) });
        for (code, _) in keys {
            assert!(!state.shell_rust_takes_key(seat, code, true, false, now), "ordinary layer did not list {code}");
        }
        state.shell_rust_handle(Msg::Takes(keys.iter().map(|(_, name)| (*name).into()).collect()));
        for (code, _) in keys {
            assert!(state.shell_rust_takes_key(seat, code, true, false, now));
            assert!(state.shell_rust_takes_key(seat, code, false, false, now));
        }
        state.shell_rust_handle(Msg::Takes(Vec::new()));
        for (code, _) in keys {
            state.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: Some(json!({"saver": true})) });
            assert!(state.shell_rust_takes_key(seat, code, true, false, now), "saver takes {code}");
            state.shell_rust_handle(Msg::Ui { can: None, wants: Some(false), live: Some(false), state: Some(json!({"saver": false})) });
            assert!(state.shell_rust_takes_key(seat, code, false, false, now), "release follows the waking press");
            assert!(!state.shell_rust_takes_key(seat, code, true, false, now), "next press is not captured");
        }
        reader.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut text = String::new();
        let mut buf = [0u8; 2048];
        while text.matches('\n').count() < 12 {
            let n = reader.read(&mut buf).unwrap();
            assert_ne!(n, 0);
            text.push_str(std::str::from_utf8(&buf[..n]).unwrap());
        }
        let events: Vec<Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        for (i, (_, name)) in keys.iter().cycle().take(6).enumerate() {
            assert_eq!(events[2 * i], json!({"key": {"k": name, "d": true, "r": false, "seat": 0}}));
            assert_eq!(events[2 * i + 1], json!({"key": {"k": name, "d": false, "r": false, "seat": 0}}));
        }
    }

    #[test]
    fn any_key_wakes_a_saver_that_is_up_as_the_name_any_and_the_home_key_reaches_the_rust_client() {
        let (mut state, _dir) = state_with("saver");
        state.shell_rust_handle(Msg::Connected);
        let (ours, theirs) = UnixStream::pair().unwrap();
        ours.set_nonblocking(true).unwrap();
        state.shell_rust.keys = Some(ours);
        let seat = crate::server::seat::SeatId(0);
        let now = Instant::now();
        let red = crate::input::code::KEY_RED;
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: Some(json!({"saver": false})) });
        assert!(!state.shell_rust_takes_key(seat, red, true, false, now), "a key with no name is the app's while the saver is not up");
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: Some(json!({"saver": true})) });
        assert!(state.shell_rust_takes_key(seat, red, true, false, now), "up: it wakes the saver and does nothing else");
        assert!(state.shell_rust_takes_key(seat, red, false, false, now + Duration::from_millis(60)), "and its release is the saver's too");
        assert!(!state.shell_rust_takes_key(seat, red, true, true, now), "a key the router swallowed (power, the gear) keeps its own way");
        let mut reader = theirs;
        reader.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut buf = [0u8; 1024];
        let n = reader.read(&mut buf).unwrap();
        let first: Value = serde_json::from_str(std::str::from_utf8(&buf[..n]).unwrap().lines().next().unwrap()).unwrap();
        assert_eq!(first, json!({"key": {"k": "any", "d": true, "r": false, "seat": 0}}));
        state.shell_home_key_offer(false);
        assert_eq!(state.shell_rust.topic("homekey").unwrap()["switcher"], false, "the Home key is told to the Rust client too");
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
    fn the_kinds_are_the_known_ones_sorted() {
        assert_eq!(parse_kinds("toast,osd,osd, nothing,settings"), vec!["osd", "settings", "toast"]);
        assert!(parse_kinds("").is_empty());
        assert_eq!(parse_kinds("osd,ai"), vec!["ai", "osd"], "the AI prompt is a kind of its own");
    }

    /// The state of `keys_go_over_the_key_socket...`: a connected Rust client with a key socket whose other end is returned.
    fn with_key_socket(state: &mut State) -> UnixStream {
        let (ours, theirs) = UnixStream::pair().unwrap();
        ours.set_nonblocking(true).unwrap();
        state.shell_rust.keys = Some(ours);
        theirs
    }

    fn lines(mut reader: UnixStream) -> Vec<Value> {
        reader.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        let mut text = String::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            text.push_str(std::str::from_utf8(&buf[..n]).unwrap());
            if n < buf.len() {
                break;
            }
        }
        text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn the_pointer_goes_to_the_rust_client_where_its_layer_wants_the_keys_or_on_the_sheet_of_its_keyboard_and_nowhere_else() {
        let (mut state, _dir) = state_with("keyboard");
        state.shell_rust_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Ui { can: Some(vec!["keyboard".into()]), wants: Some(false), live: None, state: None });
        let theirs = with_key_socket(&mut state);
        // the layer does not want the keys and no keyboard is up: the pointer is the apps'
        assert!(!state.shell_rust_pointer_sample(100, 900, true));
        assert!(!state.shell_rust_pointer_button(true), "no click is the client's either");
        // a keyboard sheet at the foot: a sample over it is the client's, one beside it is the app's, and the client is told once that it left
        state.scene.shell_pointer = Some(crate::Rect::new(0, 700, 1920, 380));
        assert!(state.shell_rust_pointer_sample(100, 900, true));
        state.seat.cursor_visible = true;
        assert!(state.shell_rust_pointer_button(true));
        assert!(state.shell_rust_pointer_button(false));
        assert!(state.shell_rust_pointer_sample(110, 900, true));
        assert!(!state.shell_rust_pointer_sample(110, 400, true));
        assert!(!state.shell_rust_pointer_sample(120, 400, true), "nothing more once it is gone");
        assert_eq!(
            lines(theirs),
            vec![
                json!({"pointer": {"x": 100.0, "y": 900.0, "buttons": 0, "visible": true}}),
                json!({"pointer": {"x": 100.0, "y": 900.0, "buttons": 1, "visible": true}}),
                json!({"pointer": {"x": 100.0, "y": 900.0, "buttons": 0, "visible": true}}),
                json!({"pointer": {"x": 110.0, "y": 900.0, "buttons": 0, "visible": true}}),
                json!({"pointer": {"x": 110.0, "y": 400.0, "buttons": 0, "visible": false}}),
            ]
        );
        // a QML keyboard is not the Rust client's sheet
        let (mut qml, _dir2) = state_with("osd");
        qml.shell_rust_handle(Msg::Connected);
        let _theirs = with_key_socket(&mut qml);
        qml.scene.shell_pointer = Some(crate::Rect::new(0, 700, 1920, 380));
        assert!(!qml.shell_rust_pointer_sample(100, 900, true));
        // while its layer wants the keys the pointer is the client's everywhere, and the cursor going ends it
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: None, state: None });
        assert!(state.shell_rust_pointer_sample(5, 5, true));
        assert!(!state.shell_rust_pointer_sample(5, 5, false), "a cursor that is hidden is nobody's");
    }

    #[test]
    fn native_saver_captures_real_app_input_and_preserves_an_earlier_app_press() {
        use crate::harness::{Client, Server};
        use crate::input::code::{KEY_ENTER, KEY_LEFT};
        use crate::server::seat::{SeatId, pointer_motion};

        let mut server = Server::start();
        let mut app = Client::connect(&mut server);
        let _keyboard = app.seat.get_keyboard(&app.qh, ());
        let _pointer = app.seat.get_pointer(&app.qh, ());
        let window = app.card("_WEBOS_WINDOW_TYPE_CARD", "Player");
        let buffer = app.shm_buffer(64, 64, 0xff00_0000);
        app.present(&window.surface, &buffer);
        assert!(app.wait_until(Duration::from_secs(3), |s| s.keyboard_enters >= 1 && s.frames_done >= 1));
        let _keys = server.eval(|s| {
            s.shell_rust_open(0, "test".into()).unwrap();
            s.shell_rust.kinds = vec!["saver".into()];
            s.shell_rust_handle(Msg::Connected);
            with_key_socket(s)
        });
        server.eval(|s| pointer_motion(s, 20, 25));
        assert!(app.wait_until(Duration::from_secs(3), |s| !s.pointer_enters.is_empty()));
        let t = Instant::now();

        // An app already holding a press receives its release, even if the saver comes up in between.
        server.eval(move |s| s.key_event_at(SeatId(0), KEY_ENTER, true, false, t));
        assert!(app.wait_until(Duration::from_secs(3), |s| s.buttons.len() == 1));
        server.eval(move |s| {
            s.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: Some(json!({"saver": true})) });
            pointer_motion(s, 25, 30);
            s.key_event_at(SeatId(0), KEY_ENTER, true, false, t + Duration::from_millis(25));
            s.key_event_at(SeatId(0), KEY_ENTER, false, false, t + Duration::from_millis(50));
        });
        assert!(app.wait_until(Duration::from_secs(3), |s| s.buttons.len() == 3));
        assert_eq!(app.state.buttons, vec![(0x110, 1), (0x110, 1), (0x110, 0)]);

        // A stationary cursor stays over the app, but a new click belongs to the saver.
        server.eval(move |s| {
            s.key_event_at(SeatId(0), KEY_ENTER, true, false, t + Duration::from_millis(200));
            s.shell_rust_handle(Msg::Ui { can: None, wants: Some(false), live: Some(false), state: Some(json!({"saver": false})) });
            s.key_event_at(SeatId(0), KEY_ENTER, false, false, t + Duration::from_millis(250));
        });
        app.roundtrip();
        assert_eq!(app.state.buttons.len(), 3, "the app did not receive the waking click");
        server.eval(move |s| {
            s.key_event_at(SeatId(0), KEY_ENTER, true, false, t + Duration::from_millis(400));
            s.key_event_at(SeatId(0), KEY_ENTER, false, false, t + Duration::from_millis(450));
        });
        assert!(app.wait_until(Duration::from_secs(3), |s| s.buttons.len() == 5));
        assert_eq!(&app.state.buttons[3..], &[(0x110, 1), (0x110, 0)]);

        // The waking key and its release are the saver's; the next key goes to the app.
        server.eval(move |s| {
            s.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: Some(json!({"saver": true})) });
            s.key_event_at(SeatId(0), KEY_LEFT, true, false, t + Duration::from_millis(600));
            s.shell_rust_handle(Msg::Ui { can: None, wants: Some(false), live: Some(false), state: Some(json!({"saver": false})) });
            s.key_event_at(SeatId(0), KEY_LEFT, false, false, t + Duration::from_millis(650));
        });
        app.roundtrip();
        assert!(app.state.keys.is_empty(), "the app did not receive the waking key");
        server.eval(move |s| {
            s.key_event_at(SeatId(0), KEY_LEFT, true, false, t + Duration::from_millis(800));
            s.key_event_at(SeatId(0), KEY_LEFT, false, false, t + Duration::from_millis(850));
        });
        assert!(app.wait_until(Duration::from_secs(3), |s| s.keys.len() == 2));
        assert_eq!(app.state.keys, vec![(KEY_LEFT, 1), (KEY_LEFT, 0)]);
    }

    #[test]
    fn unknown_wake_key_keeps_its_original_name_until_release_for_real_app() {
        use crate::harness::{Client, Server};
        use crate::input::code::KEY_RED;
        use crate::server::seat::SeatId;

        let mut server = Server::start();
        let mut app = Client::connect(&mut server);
        let _keyboard = app.seat.get_keyboard(&app.qh, ());
        let window = app.card("_WEBOS_WINDOW_TYPE_CARD", "Player");
        let buffer = app.shm_buffer(64, 64, 0xff00_0000);
        app.present(&window.surface, &buffer);
        assert!(app.wait_until(Duration::from_secs(3), |s| s.keyboard_enters >= 1 && s.frames_done >= 1));
        let reader = server.eval(|s| {
            s.shell_rust_open(0, "test".into()).unwrap();
            s.shell_rust.kinds = vec!["saver".into()];
            s.shell_rust_handle(Msg::Connected);
            with_key_socket(s)
        });
        let t = Instant::now();
        server.eval(move |s| {
            s.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: Some(json!({"saver": true})) });
            s.key_event_at(SeatId(0), KEY_RED, true, false, t);
            s.shell_rust_handle(Msg::Ui { can: None, wants: Some(false), live: Some(false), state: Some(json!({"saver": false})) });
            s.key_event_at(SeatId(0), KEY_RED, true, false, t + Duration::from_millis(50));
            s.key_event_at(SeatId(0), KEY_RED, false, false, t + Duration::from_millis(100));
            assert!(s.shell_rust.took.is_empty(), "the wake key capture is released");
        });
        app.roundtrip();
        assert!(app.state.keys.is_empty(), "no repeat or orphan release reaches the app");
        let events = lines(reader);
        assert_eq!(events, vec![
            json!({"key": {"k": "any", "d": true, "r": false, "seat": 0}}),
            json!({"key": {"k": "any", "d": true, "r": true, "seat": 0}}),
            json!({"key": {"k": "any", "d": false, "r": false, "seat": 0}}),
        ]);
        server.eval(move |s| {
            s.key_event_at(SeatId(0), KEY_RED, true, false, t + Duration::from_millis(200));
            s.key_event_at(SeatId(0), KEY_RED, false, false, t + Duration::from_millis(250));
        });
        assert!(app.wait_until(Duration::from_secs(3), |s| s.keys.len() == 2));
        assert_eq!(app.state.keys, vec![(KEY_RED, 1), (KEY_RED, 0)]);
    }

    #[test]
    fn stationary_app_pointer_is_claimed_by_native_saver_without_motion() {
        let (mut state, _dir) = state_with("saver");
        state.shell_rust_handle(Msg::Connected);
        let theirs = with_key_socket(&mut state);
        state.seat.cursor_visible = true;
        state.seat.pointer_position = (300, 200);
        state.seat.pointer_focus = Some(crate::surface::SurfaceId(999));
        let motions = state.seat.motions;
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: Some(json!({"saver": true})) });
        assert_eq!(state.seat.motions, motions, "handoff is not remote motion");
        assert!(!state.shell_rust.pointer_in, "metadata alone sends no pointer sample");
        state.seat.button_down = true;
        assert!(!crate::server::seat::pointer_button(&mut state, false), "an app-held release is not captured by the saver");
        assert!(!state.shell_rust.pointer_in);
        state.seat.button_down = false;
        assert!(crate::server::seat::pointer_button(&mut state, true));
        assert!(state.shell_rust.pointer_in, "the real press claims the stationary pointer");
        assert!(state.shell_rust_button_down());
        assert!(!state.seat.button_down, "the app did not receive the press");
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(false), live: Some(false), state: Some(json!({"saver": false})) });
        assert!(crate::server::seat::pointer_button(&mut state, false), "release follows the waking press");
        assert!(!state.seat.button_down);
        assert!(!crate::server::seat::pointer_button(&mut state, true), "the next press goes back toward the app");
        assert!(!state.shell_rust.pointer_in);
        let sent: Vec<(u64, bool)> = lines(theirs).iter().map(|l| (l["pointer"]["buttons"].as_u64().unwrap(), l["pointer"]["visible"].as_bool().unwrap())).collect();
        assert_eq!(sent, vec![(0, true), (1, true), (0, true), (0, false)]);
    }

    #[test]
    fn a_release_with_no_press_is_nothing_a_press_needs_the_cursor_shown_and_a_hidden_cursor_ends_a_press_without_a_click() {
        let (mut state, _dir) = state_with("home");
        state.shell_rust_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: None, state: None });
        let theirs = with_key_socket(&mut state);
        state.seat.cursor_visible = true;
        assert!(state.shell_rust_pointer_sample(300, 300, true));
        // the remote's sleep: a release with no press seen: swallowed by the seat, nothing sent to anyone
        assert!(crate::server::seat::pointer_button(&mut state, false), "swallowed");
        assert!(!state.shell_rust_pointer_button(false), "the client is owed no release");
        // a press with the cursor hidden is not a click
        state.seat.cursor_visible = false;
        assert!(!state.shell_rust_pointer_button(true));
        state.seat.cursor_visible = true;
        // a real click, then a press that the cursor's hiding ends: its release is nothing
        assert!(state.shell_rust_pointer_button(true));
        assert!(state.shell_rust_button_down());
        assert!(state.shell_rust_pointer_button(false));
        assert!(state.shell_rust_pointer_button(true));
        assert!(!state.shell_rust_pointer_sample(300, 300, false));
        assert!(!state.shell_rust_button_down());
        assert!(crate::server::seat::pointer_button(&mut state, false), "the release after the hiding is swallowed too");
        let sent: Vec<(u64, bool)> = lines(theirs)
            .iter()
            .map(|l| (l["pointer"]["buttons"].as_u64().unwrap(), l["pointer"]["visible"].as_bool().unwrap()))
            .collect();
        assert_eq!(sent, vec![(0, true), (1, true), (0, true), (1, true), (0, false)], "no release, no stray press");
    }

    #[test]
    fn the_tile_a_launch_grows_from_is_taken_in_process_and_answered_not_sent_to_the_bus() {
        let (mut state, _dir) = state_with("home");
        state.shell_rust_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Luna {
            tag: 41,
            uri: "luna://com.webos.surfacemanager/setLaunchOrigin".into(),
            payload: json!({"appId": "youtube.leanback.v4", "from": null, "tile": {"x": 300.0, "y": 700.0, "width": 134.4, "height": 134.4, "radius": 26.0}}),
            subscribe: false,
        });
        assert!(state.scene.transitions.origin_of("youtube.leanback.v4").is_some(), "the compositor knows the tile");
        assert!(state.shell_rust.luna_tags.is_empty(), "nothing was sent to the bus to wait for");
        assert_eq!(state.shell_rust.topic("luna").map(|t| (t["tag"].clone(), t["payload"]["returnValue"].clone())), Some((json!(41), json!(true))));
    }

    #[test]
    fn while_an_apps_card_grows_over_home_the_app_has_the_keys_and_home_has_them_back_when_the_card_is_whole() {
        use crate::transition::{FRect, Kind, Party};
        let (mut state, _dir) = state_with("home");
        state.shell_rust_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: None, state: None });
        let _theirs = with_key_socket(&mut state);
        assert!(state.shell_rust_wants_keys(), "Home in front has the keys");
        state.scene.transitions.settings.enabled = true;
        let full = FRect::new(0.0, 0.0, 1920.0, 1080.0);
        let app = Party { id: crate::window::WindowId(7), app_id: "youtube.leanback.v4", rect: full, video: false };
        assert!(state.scene.transitions.begin(Kind::Launch, 1_000_000, Some(app), None, &[(crate::scene::HOME_RUST, full)]));
        assert!(state.scene.transitions.holds_home());
        assert!(!state.shell_rust_wants_keys(), "Home is drawn under the card, it is not the front");
        state.scene.transitions.clear();
        assert!(state.shell_rust_wants_keys());
    }

    #[test]
    fn keyboard_routes_only_to_the_client_that_supports_it() {
        let (mut state, _dir) = state_with("keyboard");
        state.shell_rust_handle(Msg::Connected);
        state.shell_sync_topics();
        assert_eq!(state.shell_rust.topic("ime"), Some(&json!({"active": false, "show": false})));
        state.shell_rust_handle(Msg::Ui { can: Some(vec!["keyboard".into()]), wants: None, live: None, state: None });
        assert!(state.shell_can("keyboard"));
        let (mut absent, _dir2) = state_with("osd");
        absent.shell_rust_handle(Msg::Connected);
        absent.shell_rust_handle(Msg::Ui { can: Some(vec!["keyboard".into()]), wants: None, live: None, state: None });
        assert!(!absent.shell_can("keyboard"));
    }

    /// A socket that takes `room` more bytes and then reports it is full.
    struct Narrow {
        room: usize,
        got: Vec<u8>,
    }

    impl Write for Narrow {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.room == 0 {
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            let n = buf.len().min(self.room);
            self.room -= n;
            self.got.extend_from_slice(&buf[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_line_the_key_socket_took_part_of_is_finished_before_the_next_goes() {
        let mut sock = Narrow { room: 5, got: Vec::new() };
        let mut tail = Vec::new();
        write_line(&mut sock, &mut tail, "{\"key\":1}").expect("part of a line is as good as sent");
        assert_eq!(sock.got, b"{\"key");
        // the client has not read: the next line is lost whole, not spliced on
        assert!(write_line(&mut sock, &mut tail, "{\"key\":2}").is_err());
        assert_eq!(sock.got, b"{\"key");
        // it reads: the rest of the first line goes, then lines go whole
        sock.room = 100;
        write_line(&mut sock, &mut tail, "{\"key\":3}").unwrap();
        assert_eq!(String::from_utf8(sock.got).unwrap(), "{\"key\":1}\n{\"key\":3}\n");
        assert!(tail.is_empty());
    }

    #[test]
    fn a_line_the_key_socket_took_none_of_is_dropped_and_the_next_one_goes_whole() {
        let mut sock = Narrow { room: 0, got: Vec::new() };
        let mut tail = Vec::new();
        assert!(write_line(&mut sock, &mut tail, "{\"key\":1}").is_err());
        assert!(tail.is_empty());
        sock.room = 100;
        write_line(&mut sock, &mut tail, "{\"key\":2}").unwrap();
        assert_eq!(sock.got, b"{\"key\":2}\n");
    }

    #[test]
    fn the_first_start_waits_for_homes_map_unless_boot_direct_is_on() {
        let (mut state, _dir) = state_with("home");
        assert_eq!(state.ui_start_wait(), START_ALONE);
        // a directory of its own: `state_with`'s is shared by the tests whose list is as long
        let flag_dir = std::env::temp_dir().join(format!("lumatv-boot-direct-start-{}", std::process::id()));
        std::fs::create_dir_all(&flag_dir).unwrap();
        std::fs::write(flag_dir.join(crate::boot_direct::FLAG_FILE), "on\n").unwrap();
        state.boot_direct = crate::boot_direct::BootDirect::new(&flag_dir);
        assert_eq!(state.ui_start_wait(), Duration::ZERO, "SAM's Home is the stub, whose launch waits for WebAppMgr: the client does not");
        std::fs::write(flag_dir.join(crate::boot_direct::FLAG_FILE), "off\n").unwrap();
        state.boot_direct = crate::boot_direct::BootDirect::new(&flag_dir);
        assert_eq!(state.ui_start_wait(), START_ALONE);
        let _ = std::fs::remove_dir_all(flag_dir);
    }
}
