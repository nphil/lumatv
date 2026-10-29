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
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::State;
use crate::shell::{
    EVENTS_KEPT, Link, LunaTag, Msg, NICE, POLL_TIMEOUT, RESTART_LIMIT, RESTART_WINDOW, Stats, TAKEABLE, TOOK_STALE, TRACE_MARKER, allowed, drain,
    spawn_server, wake, write_token,
};

/// The kinds the Rust shell may answer, in the order `ctl shell` lists them. `quick`, `picker` and `drawer` are the panels LumaTV opens
/// itself (`State::shell_show`): while the Rust shell answers one, its `show` goes to it and the QML client's panel stays shut.
pub const KINDS: &[&str] = &["home", "osd", "settings", "toast", "cameras", "quick", "picker", "drawer", "menu", "confirm", "switcher", "entry", "keyboard", "alert", "saver", "capture", "cards", "popup", "layout", "ai", "banner"];
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
    launch: Option<Launch>,
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
            pointer_in: false,
            pointer_at: (0.0, 0.0),
            buttons: 0,
            live: false,
            ui_state: Value::Null,
            luna_tags: Vec::new(),
            cancelled_early: Vec::new(),
            pushed: BTreeMap::new(),
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
    /// Opens the Rust client's channel and plans its first start (`--shell-rust`). Called with `shell_start`, once the Wayland
    /// socket listens. The engine was attached before the loop (`main.rs`, `Output::ui_attach`) and its client end adopted.
    pub fn shell_rust_start(&mut self) {
        let kinds = self.config.shell_rust.clone();
        if kinds.is_empty() {
            return;
        }
        self.shell_rust.kinds = kinds;
        let Some(launch) = find_launch() else {
            self.journal("shell-rust: no lumatv-ui beside the compositor's program (or LUMATV_UI_BIN), or its loader is gone; the Rust shell is not run".into());
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
        self.shell_rust.launch = Some(launch);
        self.shell_rust.port = port;
        self.shell_rust.token_path = Some(token_path);
        self.shell_rust.log = Some(self.config.data_dir.join("compositor/shell-rust.log"));
        self.journal(format!("shell-rust: UI channel on 127.0.0.1:{port}, answers {}", self.shell_rust.kinds.join(", ")));
        self.shell_rust_sync_topics();
        self.shell_rust.next_start = Some(Instant::now() + START_ALONE);
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
                    Ok(s) => {
                        self.shell_rust.shaders_told = false;
                        // The client is forked at the next tick: the attach and the fork each take a pass of their own, so that neither adds
                        // to the other's.
                        self.shell_rust.socket = Some(s);
                        self.shell_rust.next_start = Some(now);
                        return;
                    }
                    // The engine's programs are being linked on a thread of its own (`Output::ui_attach`): the loop goes on, and asks
                    // again at the next tick.
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if !std::mem::replace(&mut self.shell_rust.shaders_told, true) {
                            self.journal("shell-rust: the engine's programs are being linked on a thread of its own; the client starts when they are".into());
                        }
                        self.shell_rust.next_start = Some(now + Duration::from_millis(50));
                        return;
                    }
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
        // and the end of its socket that waited for the next child is the end of nothing
        self.shell_rust.socket = None;
    }

    /// Whether the Rust shell draws Home now: the `home` kind is its and the client is on the engine. The scene then leaves the QML
    /// shell's Home window out of the stack and counts Home as drawn while no app's card is in front, which is what the `foreground`
    /// topic tells both clients (the Rust one answers `wants` for the keys while Home is in front).
    pub fn shell_rust_sync_home(&mut self) {
        let on = self.shell_rust.answers("home") && self.shell_rust.engine_connected;
        if self.scene.set_home_rust(on) {
            self.journal(format!(
                "shell-rust: Home is {}",
                if on { "the Rust shell's: the QML Home window is left out of the stack" } else { "the QML shell's again" }
            ));
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
        // the switcher's topics go to whichever client answers it now
        self.switcher.topics_sig = 0;
        self.shell_sync_topics();
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
                self.switcher.topics_sig = 0;
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
            Msg::Vwin(body) => {
                self.shell_rust.note_seen(now);
                self.vwin_command(&body);
                if self.output.draws_on_damage(now) {
                    self.tick();
                    return true;
                }
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
        if self.shell_rust.answers("switcher") {
            self.switcher_sync_topics();
        }
    }

    /// Whether the Rust client's layer has the keys: the keys go over the key socket (`input.rs`).
    pub fn shell_rust_wants_keys(&self) -> bool {
        self.shell_rust.connected && self.shell_rust.wants && self.shell_rust.keys.is_some()
    }

    /// Whether the Rust client's saver is up (its state says so): every key of the remote wakes it, the ones the key socket has no name for too.
    fn shell_rust_saver_up(&self) -> bool {
        self.shell_rust.answers("saver") && self.shell_rust.ui_state.get("saver").and_then(Value::as_bool) == Some(true)
    }

    /// One key of the remote for the Rust client, before the focus routing: its layer wants the keys (every key but the compositor's
    /// own goes to it), or its cards take OK and Back by name while an app has the focus (`takes`, as the QML client's). Returns
    /// whether the key went to it.
    pub fn shell_rust_takes_key(&mut self, seat: crate::server::seat::SeatId, code: u32, pressed: bool, swallowed: bool, now: Instant) -> bool {
        // a key `keys.conf` taught the Rust shell by name (the mic, the on-screen remote, Search): sent only while the client lists it in its `takes`
        let taught = if crate::server::ime::own_key_name(code).is_none() { self.keys.router.table.shell_name(code) } else { None };
        let name = match crate::server::ime::own_key_name(code).or(taught) {
            Some(n) => n,
            // the saver is up: any key wakes it, whatever it is (the colour keys, play, menu), and the key does nothing else
            None if self.shell_rust_saver_up() => "any",
            None => return false,
        };
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
            if (!wants && !takes) || (taught.is_some() && !takes) {
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

    /// The remote's OK (or a button) while the pointer is over the Rust client's layers: its click.
    pub fn shell_rust_pointer_button(&mut self, pressed: bool) -> bool {
        if !self.shell_rust.pointer_in {
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
    use crate::shell::is_saver_vote;
    use std::io::Read;

    /// A state with both clients' channels open on free ports (no child is started: no binary), the Rust shell answering `kinds`.
    fn state_with(kinds: &str) -> (State, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("lumatv-shell-rust-{}-{}", std::process::id(), kinds.len()));
        std::fs::create_dir_all(&dir).unwrap();
        let display: wayland_server::Display<State> = wayland_server::Display::new().unwrap();
        // two ports in a row (the QML client's and the Rust client's), free at this moment: another test may take one in between, so each is
        // bound at once and the pair is tried again when the second is not there
        let (port, listener, rust_listener) = loop {
            let first = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = first.local_addr().unwrap().port();
            if let Ok(second) = TcpListener::bind(("127.0.0.1", port + 1)) {
                break (port, first, second);
            }
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
        state.shell.link = Some(spawn_server(listener, "qml".into()).unwrap());
        state.shell_rust.link = Some(spawn_server(rust_listener, "rust".into()).unwrap());
        state.shell_rust.kinds = parse_kinds(kinds);
        state.shell.marker = dir.join("marker");
        state.shell_rust.marker = dir.join("marker");
        (state, dir)
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

    /// The client is started through the loader with its two descriptors and its environment. The stand-in loader drops the loader's
    /// options and runs its program with `sh`; the stand-in client is not executable, so a direct exec of it (the bug) fails.
    #[test]
    fn the_client_is_spawned_through_the_loader_when_there_is_one() {
        let root = scratch("spawn");
        let out = root.join("out");
        write_file(&root.join("lib/ld-linux-aarch64.so.1"), &format!("#!/bin/sh\necho \"loader $*\" >> {}\nshift 2\nexec sh \"$@\"\n", out.display()), 0o755);
        write_file(&root.join("bin/lumatv-ui"), &format!("echo \"client fd3=$LUMATV_UI_FD kinds=$LUMATV_UI_KINDS\" >> {}\n", out.display()), 0o644);
        let (mut state, _dir) = state_with("osd,toast");
        let (engine, _theirs) = UnixStream::pair().unwrap();
        state.shell_rust.adopt_socket(engine);
        state.shell_rust.launch = Some(Launch {
            binary: root.join("bin/lumatv-ui"),
            loader: Some(Loader { path: root.join("lib/ld-linux-aarch64.so.1"), lib: root.join("lib") }),
        });
        state.shell_rust_spawn();
        assert!(state.shell_rust.child.is_some(), "{:?}", state.journal);
        let child = state.shell_rust.child.as_mut().unwrap();
        assert!(child.wait().unwrap().success());
        let text = std::fs::read_to_string(&out).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines, [format!("loader --library-path {} {}", root.join("lib").display(), root.join("bin/lumatv-ui").display()), "client fd3=3 kinds=osd,toast".into()]);

        // the same client exec'd directly, as before: not executable, so the spawn fails and is journalled
        let (mut state, _dir) = state_with("osd,toast");
        let (engine, _theirs) = UnixStream::pair().unwrap();
        state.shell_rust.adopt_socket(engine);
        state.shell_rust.launch = Some(Launch { binary: root.join("bin/lumatv-ui"), loader: None });
        state.shell_rust_spawn();
        assert!(state.shell_rust.child.is_none());
        assert!(state.journal.iter().any(|l| l.contains("did not start")), "{:?}", state.journal);
        let _ = std::fs::remove_dir_all(&root);
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
    fn the_alert_kind_takes_the_three_notification_feeds_from_the_qml_client_and_only_what_opens_a_surface() {
        let (mut state, _dir) = state_with("alert");
        state.shell_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Connected);
        let feeds =
            [("getAlertNotification", "alertAction"), ("getInputAlertNotification", "alertAction"), ("getPincodePromptNotification", "pincodePromptAction")];
        for (i, (feed, _)) in feeds.iter().enumerate() {
            let uri = format!("luna://com.webos.notification/{feed}");
            state.shell_handle(Msg::Luna { tag: 10 + i as u64, uri: uri.clone(), payload: json!({"subscribe": true}), subscribe: true });
            state.shell_rust_handle(Msg::Luna { tag: 20 + i as u64, uri, payload: json!({"subscribe": true}), subscribe: true });
        }
        for (i, (_, action)) in feeds.iter().enumerate() {
            let qml = state.shell.luna_tags.iter().find(|t| t.tag == 10 + i as u64).unwrap().token;
            let rust = state.shell_rust.luna_tags.iter().find(|t| t.tag == 20 + i as u64).unwrap().token;
            assert!(state.shell_luna_event(qml, &json!({*action: "open"})));
            assert!(
                state.shell.topic("luna").is_none_or(|l| l["tag"] != 10 + i as u64),
                "{action} of feed {i}: what opens a surface is withheld from the QML client"
            );
            assert!(state.shell_luna_event(rust, &json!({*action: "open"})));
            assert_eq!(state.shell_rust.topic("luna").unwrap()["tag"], 20 + i as u64, "and reaches the Rust client");
        }
        let qml = state.shell.luna_tags.iter().find(|t| t.tag == 10).unwrap().token;
        assert!(state.shell_luna_event(qml, &json!({"alertAction": "closeAll"})));
        assert_eq!(state.shell.topic("luna").unwrap()["tag"], 10, "a close still goes to it: nothing of it is open, and nothing is lost");
    }

    #[test]
    fn with_the_saver_the_rust_shells_the_qml_one_is_refused_its_vote_and_the_care_reads_the_rust_clients_word() {
        let (mut state, _dir) = state_with("saver");
        state.shell_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Connected);
        let vote = "luna://com.webos.service.tvpower/power2/registerScreenSaverRequest";
        state.shell_handle(Msg::Luna { tag: 31, uri: vote.into(), payload: json!({"clientName": "qml", "subscribe": true}), subscribe: true });
        assert!(state.shell.luna_tags.is_empty(), "the QML client's vote is not passed on");
        assert_eq!(state.shell.topic("luna").unwrap()["payload"]["returnValue"], false, "it is told no");
        state.shell_rust_handle(Msg::Luna { tag: 32, uri: vote.into(), payload: json!({"clientName": "rust", "subscribe": true}), subscribe: true });
        assert_eq!(state.shell_rust.luna_tags.len(), 1, "the Rust client's vote goes through");
        // the OLED care stands aside for the saver the Rust client says is up
        state.shell_rust_handle(Msg::Ui { can: Some(vec![]), wants: Some(true), live: Some(true), state: Some(json!({"saver": true})) });
        state.tick();
        assert!(state.scene.care.saver, "the Rust shell's saver is the saver");
        state.shell_rust_handle(Msg::Ui { can: None, wants: Some(false), live: Some(false), state: Some(json!({"saver": false})) });
        state.tick();
        assert!(!state.scene.care.saver);
        // without the kind the vote is the QML client's as before
        let (mut qml_only, _d2) = state_with("toast");
        qml_only.shell_handle(Msg::Connected);
        qml_only.shell_handle(Msg::Luna { tag: 31, uri: vote.into(), payload: json!({"subscribe": true}), subscribe: true });
        assert_eq!(qml_only.shell.luna_tags.len(), 1);
        assert!(
            is_saver_vote("luna://com.webos.service.tvpower/power/registerScreenSaverRequest")
                && !is_saver_vote("luna://com.webos.service.tvpower/power/getPowerState")
        );
    }

    #[test]
    fn with_the_capture_mark_the_rust_shells_the_topic_and_the_dot_are_its_own_and_the_qml_client_is_not_told() {
        let (mut state, _dir) = state_with("capture");
        state.shell_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Connected);
        state.screencap_publish();
        assert!(state.shell_rust.topic("capture").is_some(), "the Rust client is told");
        assert!(state.shell.topic("capture").is_none(), "the QML client is not");
        assert!(!state.screencap_dot_up());
        // the QML client's dot is not the dot: nothing of the Rust client's is on the screen
        state.shell_handle(Msg::Ui { can: None, wants: None, live: None, state: Some(json!({"capture": {"dot": true}})) });
        assert!(!state.screencap_dot_up(), "a dot the QML client draws does not count while the Rust shell answers capture");
        state.shell_rust_handle(Msg::Ui { can: None, wants: None, live: None, state: Some(json!({"capture": {"dot": true}})) });
        assert!(state.screencap_dot_up());
        // without the kind it is the QML client's as before
        let (mut qml, _d2) = state_with("toast");
        qml.shell_handle(Msg::Connected);
        qml.screencap_publish();
        assert!(qml.shell.topic("capture").is_some());
        qml.shell_handle(Msg::Ui { can: None, wants: None, live: None, state: Some(json!({"capture": {"dot": true}})) });
        assert!(qml.screencap_dot_up());
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
        assert!(state.shell_rust_set_kinds(&["+bogus"]).is_err());
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
    fn the_qml_client_is_told_which_kinds_the_rust_shell_answers_so_it_leaves_out_its_cards_and_its_pop_up_and_the_editor_is_shown_by_the_rust_client() {
        let (mut state, _dir) = state_with("cards,popup,layout");
        state.shell_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Connected);
        state.shell_sync_topics();
        assert_eq!(state.shell.topic("rust").unwrap()["kinds"], json!(["cards", "layout", "popup"]), "the QML client reads the list and draws neither");
        // the editor: only when the Rust client says it can, and with the item as the page
        assert!(!state.shell_show_page("layout", false, Some("popup")), "not before the client says it can");
        state.shell_rust_handle(Msg::Ui { can: Some(vec!["layout".into()]), wants: Some(false), live: Some(false), state: Some(json!({"layout": {"open": false}})) });
        assert!(state.shell_show_page("layout", false, Some("popup")));
        let show = state.shell_rust.topic("show").unwrap();
        assert_eq!((show["kind"].as_str(), show["page"].as_str()), (Some("layout"), Some("popup")));
        assert!(state.shell.topic("show").is_none(), "the QML client has no editor to open");
        // without the kinds the QML client is told the list is empty and goes on as before
        let (mut qml, _d2) = state_with("toast");
        qml.shell_handle(Msg::Connected);
        qml.shell_sync_topics();
        assert_eq!(qml.shell.topic("rust").unwrap()["kinds"], json!(["toast"]));
    }

    #[test]
    fn the_switcher_is_drawn_by_the_client_that_answers_it_and_only_that_one_hears_of_it() {
        let (mut state, _dir) = state_with("switcher");
        state.shell_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Connected);
        state.shell_handle(Msg::Ui { can: Some(vec!["switcher".into()]), wants: None, live: None, state: None });
        assert!(!state.shell_can("switcher"), "the Rust shell answers it: the QML client's word does not count");
        state.shell_rust_handle(Msg::Ui { can: Some(vec!["switcher".into()]), wants: Some(false), live: Some(false), state: None });
        assert!(state.shell_can("switcher"));
        state.shell_sync_topics();
        assert_eq!(state.shell_rust.topic("switcher").unwrap()["open"], false);
        assert_eq!(state.shell_rust.topic("apps"), Some(&json!([])));
        assert!(state.shell.topic("switcher").is_none() && state.shell.topic("apps").is_none(), "the QML client is not told");
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
        assert!(!state.shell_hears_home_key(false), "Home is not drawn: nothing to tell");
        let now = Instant::now();
        tap(&mut state, now);
        assert!(state.shell_rust.topic("homekey").is_none(), "an app has the screen: the Home key is the compositor's");
        state.scene.set_home_rust(true);
        assert!(state.shell_hears_home_key(false), "Home is drawn by the Rust shell, which has no window to focus");
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
    fn the_kinds_are_the_known_ones_sorted_and_changed_by_sign() {
        assert_eq!(parse_kinds("toast,osd,osd, nothing,settings"), vec!["osd", "settings", "toast"]);
        assert!(parse_kinds("").is_empty());
        assert_eq!(parse_kinds("osd,ai"), vec!["ai", "osd"], "the AI prompt is a kind of its own");
        let kinds = parse_kinds("osd");
        assert_eq!(change_kinds(&kinds, &["+toast"]).unwrap(), vec!["osd", "toast"]);
        assert_eq!(change_kinds(&kinds, &["-osd", "+toast"]).unwrap(), vec!["toast"]);
        assert!(change_kinds(&kinds, &["osd"]).is_err());
        assert!(change_kinds(&kinds, &["+bogus"]).is_err());
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
    fn the_field_and_the_keys_of_the_keyboard_go_to_the_client_that_answers_it_and_the_other_is_told_there_is_none() {
        let (mut state, _dir) = state_with("keyboard");
        state.shell_handle(Msg::Connected);
        state.shell_rust_handle(Msg::Connected);
        state.shell_sync_topics();
        assert_eq!(state.shell_rust.topic("ime"), Some(&json!({"active": false, "show": false})));
        assert_eq!(state.shell.topic("ime"), Some(&json!({"active": false, "show": false})));
        state.shell_handle(Msg::Ui { can: Some(vec!["keyboard".into()]), wants: None, live: None, state: None });
        assert!(!state.shell_can("keyboard"), "the QML client's word does not count while the Rust client answers the kind");
        state.shell_rust_handle(Msg::Ui { can: Some(vec!["keyboard".into()]), wants: None, live: None, state: None });
        assert!(state.shell_can("keyboard"));
        // a kind that is not the Rust client's stays the QML client's
        let (mut qml, _dir2) = state_with("osd");
        qml.shell_handle(Msg::Connected);
        qml.shell_handle(Msg::Ui { can: Some(vec!["keyboard".into()]), wants: None, live: None, state: None });
        assert!(qml.shell_can("keyboard"));
    }
}
