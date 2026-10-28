//! The shell client and the UI channel: build-order items 2 to 4 of "Surfaces under LumaTV's compositor"
//! (`docs/research/compositor-c2-result.md`), the interim route; the result is `compositor-surfaces-result.md`.
//!
//! The shell is LumaTV's existing QML (`payload/compositor/<module>`: LumaHome with its Settings layer, the volume display,
//! toasts, alerts and the PIN prompt, the quick panel and the input picker, the screensaver) in a 32-bit Qt process on LG's
//! own Qt 5.12, started here through `/usr/bin/qml-runner` (SAM's QML runner) as a client of this compositor with two
//! windows (`compositor/shell/Shell.qml`): `ShellHome`, drawn in the card slot while the Home stub's card is the foreground
//! one, and `ShellSystem`, above every window (`window.rs`, `scene.rs`). Keys reach them as any window's do, on
//! `wl_keyboard` with the focus the scene gives (the Home window while it is drawn, the system layer while it says it
//! wants them), after the compositor's own routing (`input.rs`, `keys.rs`), so the hold-Home escape, Volume and Mute and
//! the gear's and Input's decisions stay the compositor's.
//!
//! Everything else goes over the UI channel: an HTTP server on the loopback (`--shell-port`) that the client long-polls the
//! way it polls LumaTV's helper (`kit/Helper.qml`), since QML has no socket of its own and the set has no QtWebSockets.
//! To the UI, as topics: `foreground` (the app in front and whether Home is drawn), `screen`, `cursor`, `keyboard`, `wall` (whether the
//! moving wallpaper's hole is under Home), `refresh` (the Hz the output runs at, `refresh.rs`),
//! `audio` (the audio service's volume state the compositor subscribes to), `homekey`; and `show` (a panel the gear or
//! Input asked for) and `luna` (the answers to its calls). From the UI: what it can show (`can`), whether its system layer
//! wants the keys (`wants`), its state for the gate (`state`), and luna calls from an allowlist, which the bridge makes in
//! the compositor's name: qml-runner's own bus role reaches the app manager and nothing else. Every request carries the
//! token of `<runtime dir>/lumatv-shell.token` (root-only), so another local process cannot drive the channel.
//!
//! LumaTV's own on-screen keyboard (`server/ime.rs`, `lumatv keyboard luma`) uses three more things of the channel: the topic `ime` (the
//! text field that holds the activation and whether it wants the keyboard), the event `imekey` (the remote's keys while the keyboard is up)
//! and `POST /v1/ime` (what it typed, the keys it pressed, where it is on the screen), and `keyboard` in `can`.
//!
//! The server runs on a thread of its own and never touches the compositor's state: requests come to the loop as
//! [`Msg`]s through a channel and an eventfd ([`State::shell_fd`]), events go out through the shared [`Outbox`].

use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::State;
use crate::config::ShellMode;
use crate::luna::{Outbound, ToBridge};
use crate::window::WindowType;

/// SAM's QML runner: LG's Qt, QML and Wayland client stack in one 32-bit process.
pub const RUNNER: &str = "/usr/bin/qml-runner";
/// The app id the shell gives its windows (`appId` property); journal lines and `ctl windows` show it.
pub const APP_ID: &str = "org.nphil.lumatv.shell";
const TOKEN_FILE: &str = "lumatv-shell.token";
/// A client that has not polled for this long is gone (its own long poll is 25 s).
const POLL_TIMEOUT: Duration = Duration::from_secs(40);
/// Restarts: after this many within [`RESTART_WINDOW`] the shell is given up until `ctl shell restart`.
const RESTART_LIMIT: usize = 5;
const RESTART_WINDOW: Duration = Duration::from_secs(600);
/// Events kept for a client that is behind; one that is further behind is told to read the state again.
const EVENTS_KEPT: usize = 512;
/// The client starts once SAM's Home has mapped, or this long after the compositor started.
const START_ALONE: Duration = Duration::from_secs(15);
/// The client's nice level: below the apps (nice 0), so an app's frames win the CPU over the shell's.
const NICE: i32 = 10;
/// The kernel's trace buffer. The frame gate's captures (`docs/research/perf/capture.sh`) read their markers from it; the compositor
/// leaves two kinds of its own there (`Q:<panel>-show` when it asks the shell for a panel on a key, `Q:<feed>-delivered` when a
/// notification reaches it from the service), so the gate can tell what the shell is asked, and when, from what the shell does with it.
pub const TRACE_MARKER: &str = "/sys/kernel/debug/tracing/trace_marker";

/// The bus names the shell may call through the bridge: the services LumaTV's QML calls under LG's compositor (read off
/// `payload/compositor/<module>`), and the compositor's own names, which the bridge routes back to `luna_methods.rs`.
pub const ALLOWED_SERVICES: &[&str] = &[
    "com.webos.applicationManager",
    "com.webos.service.applicationmanager",
    "com.webos.appInstallService",
    "com.webos.service.homelaunchpoints",
    "com.webos.service.audio",
    "com.webos.service.tvpower",
    "com.webos.notification",
    "com.webos.settingsservice",
    "com.lge.settingsservice",
    "com.webos.service.settings",
    "com.palm.systemservice",
    "com.webos.service.config",
    "com.webos.service.connectionmanager",
    "com.webos.service.wifi",
    "com.webos.service.eim",
    "com.webos.service.tvReservationAgent",
    "com.webos.service.micomservice",
    "com.webos.service.bthidmanager",
    "com.webos.service.btaudiosrc",
    "com.webos.service.btaudiosnk",
    "com.webos.service.sm",
    "com.webos.service.sdx",
    "com.webos.service.update",
    "com.webos.service.pqcontroller",
    "com.webos.service.panelcontroller",
    "com.webos.service.ppm",
    "com.webos.service.attachedstoragemanager",
    "com.webos.service.accountmanager",
    "com.webos.service.userintentmanager",
    "com.webos.service.usercontextmanager",
    "com.webos.service.admanager",
    "com.webos.service.miracast",
    "com.webos.service.ics",
    "com.webos.service.ftms",
    "com.webos.service.asc",
    "com.webos.service.devicereset",
    "com.webos.service.recent",
    "com.webos.service.systemUIManager",
    "com.webos.surfacemanager",
    "lg.thinqai.adapter",
    // LG's voice service, for the keyboard's microphone key (maliit's own dictation card calls it the same way)
    "com.webos.service.voiceconductor",
];

/// Single methods of a service that is not open as a whole. The Homebrew Channel's service also runs commands as root (`exec`, `spawn`), so
/// only its two settings calls are open, for Settings > System > Support > Update's block switch (`blockUpdates`): without them the page
/// reads "Homebrew Channel: not allowed by the compositor" and cannot change the flag.
pub const ALLOWED_METHODS: &[&str] = &[
    "org.webosbrew.hbchannel.service/getConfiguration",
    "org.webosbrew.hbchannel.service/setConfiguration",
];

/// Whether the shell may make this call: `luna://<service>/...` with the service on the allowlist (a sub-name such as
/// `com.webos.service.audio/tv` counts as its service), or one of the single methods in `ALLOWED_METHODS`.
pub fn allowed(uri: &str) -> bool {
    let Some(rest) = uri.strip_prefix("luna://") else { return false };
    if ALLOWED_METHODS.contains(&rest) {
        return true;
    }
    let service = rest.split('/').next().unwrap_or("");
    ALLOWED_SERVICES.contains(&service) || ALLOWED_SERVICES.iter().any(|s| service.starts_with(s) && service[s.len()..].starts_with('.'))
}

/// What the server thread hands the loop.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// A `GET /v1/state`: the client is (re)starting its session.
    Connected,
    /// Any request: the client is alive.
    Seen,
    /// `POST /v1/ui`: any subset of the four.
    Ui { can: Option<Vec<String>>, wants: Option<bool>, live: Option<bool>, state: Option<Value> },
    /// `POST /v1/luna`.
    Luna { tag: u64, uri: String, payload: Value, subscribe: bool },
    /// `POST /v1/cancel`.
    Cancel { tag: u64 },
    /// `POST /v1/ime`: the shell's keyboard (`ime::own_op`).
    Ime(Value),
}

/// Events on their way to the client, shared with the server thread.
pub struct Outbox {
    pub seq: u64,
    /// (seq, topic, data), oldest first.
    pub events: VecDeque<(u64, String, Value)>,
    /// The latest value of each topic, what `GET /v1/state` answers.
    pub topics: BTreeMap<String, Value>,
    /// This compositor's start, so a client tells a restart from a reset.
    pub boot: u64,
}

impl Outbox {
    fn push(&mut self, topic: &str, data: Value) -> u64 {
        self.seq += 1;
        self.events.push_back((self.seq, topic.to_string(), data.clone()));
        while self.events.len() > EVENTS_KEPT {
            self.events.pop_front();
        }
        self.topics.insert(topic.to_string(), data);
        self.seq
    }

    /// The events after `after`, or `None` when the client is further behind than what is kept.
    fn since(&self, after: u64) -> Option<Vec<Value>> {
        if after < self.seq && self.events.front().is_some_and(|(s, _, _)| *s > after + 1) {
            return None;
        }
        Some(self.events.iter().filter(|(s, _, _)| *s > after).map(|(_, t, d)| json!({"topic": t, "data": d})).collect())
    }
}

struct Link {
    outbox: Arc<Mutex<Outbox>>,
    /// Written when the outbox grew: the server thread answers its pending long polls.
    wake_out: OwnedFd,
    inbox: Receiver<Msg>,
    /// Written by the server thread when the inbox grew; the loop polls it ([`State::shell_fd`]).
    wake_in: OwnedFd,
}

/// How the client is started.
struct Launch {
    runner: PathBuf,
    main_qml: PathBuf,
    port: u16,
    token_path: PathBuf,
    log: PathBuf,
}

/// Why a start on the spawn thread did not give a child.
enum SpawnFailure {
    /// No LumaHome.qml under the data directory: the shell is given up until `ctl shell restart`.
    NoModule(PathBuf),
    /// The runner did not start; tried again after a pause.
    Start(std::io::Error),
}

/// A luna call made for the shell: its tag, the bridge token it got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LunaTag {
    pub tag: u64,
    pub token: u64,
    pub subscribe: bool,
    pub uri: String,
}

#[derive(Default, Debug, Clone, Copy)]
pub struct Stats {
    pub starts: u32,
    pub exits: u32,
    pub events: u64,
    pub luna_calls: u64,
    pub luna_refused: u64,
    pub shows: u64,
}

pub struct Shell {
    launch: Option<Launch>,
    link: Option<Link>,
    child: Option<Child>,
    /// A start on its thread (`spawn_client`) whose child has not arrived yet.
    spawning: Option<Receiver<Result<Child, SpawnFailure>>>,
    /// The thread that forks the client (`spawn_thread`), made at the first start and kept for the life of the compositor: the
    /// client's PR_SET_PDEATHSIG fires when the thread that forked it ends, so a thread per start killed the client at once.
    spawner: Option<Sender<SpawnRequest>>,
    child_started: Option<Instant>,
    starts: VecDeque<Instant>,
    next_start: Option<Instant>,
    last_wait: Option<Instant>,
    pub gave_up: bool,
    pub connected: bool,
    last_seen: Option<Instant>,
    /// What the UI says it can show now: "quick", "picker", "drawer", "keyboard".
    pub can: Vec<String>,
    /// The UI's system layer wants the keys.
    pub wants: bool,
    /// The UI's system layer has something to show (a banner, a panel, a toast, the saver). Its window stays mapped and warm
    /// meanwhile; the scene draws it only while this is true (`Scene::set_shell_system_live`). True until the UI says.
    pub live: bool,
    /// What the UI last reported about itself (`ctl shell` shows it; the gate reads it).
    pub ui_state: Value,
    pub luna_tags: Vec<LunaTag>,
    /// The last value pushed per topic, so an unchanged state is not pushed again.
    pushed: BTreeMap<String, Value>,
    /// Without a server thread (tests, `--shell off`) the events pushed, newest last.
    pub recorded: Vec<Value>,
    pub stats: Stats,
    pub error: Option<String>,
    /// Where the trace markers are written ([`TRACE_MARKER`]; a file of its own in the tests).
    pub marker: PathBuf,
}

impl Default for Shell {
    fn default() -> Self {
        Shell {
            launch: None,
            link: None,
            child: None,
            spawning: None,
            spawner: None,
            child_started: None,
            starts: VecDeque::new(),
            next_start: None,
            last_wait: None,
            gave_up: false,
            connected: false,
            last_seen: None,
            can: Vec::new(),
            wants: false,
            live: true,
            ui_state: Value::Null,
            luna_tags: Vec::new(),
            pushed: BTreeMap::new(),
            recorded: Vec::new(),
            stats: Stats::default(),
            error: None,
            marker: PathBuf::from(TRACE_MARKER),
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        if let Some(c) = &mut self.child {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Shell {
    /// Whether a server thread or a child exists.
    pub fn enabled(&self) -> bool {
        self.launch.is_some()
    }

    pub fn child_pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    /// Queues one event for the client (and keeps it when there is no client thread). Returns its sequence number.
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

    /// Pushes `topic` when its value differs from the last push.
    pub fn push_if_changed(&mut self, topic: &str, data: Value) -> bool {
        if self.pushed.get(topic) == Some(&data) {
            return false;
        }
        self.push(topic, data);
        true
    }

    /// The topics pushed so far, for `ctl shell` and the tests.
    pub fn topic(&self, name: &str) -> Option<&Value> {
        self.pushed.get(name)
    }

    /// Handles one message from the server thread without a `State` (the parts that need one are in `State::shell_handle`).
    fn note_seen(&mut self, now: Instant) {
        self.last_seen = Some(now);
        if !self.connected {
            self.connected = true;
        }
    }

    /// Writes a line into the trace buffer ([`TRACE_MARKER`]); where there is none (the host, a kernel without tracefs) nothing happens.
    fn mark(&self, text: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&self.marker) {
            let _ = f.write_all(format!("{text}\n").as_bytes());
        }
    }
}

/// The main loop's side.
impl State {
    /// Decides whether the shell runs (`--shell`), opens the channel and starts the client. Called once the Wayland socket
    /// listens, so the client finds it.
    pub fn shell_start(&mut self) {
        let main_qml = match &self.config.shell {
            ShellMode::Off => return,
            ShellMode::At(p) => p.clone(),
            ShellMode::Auto => {
                if !self.config.input {
                    return;
                }
                match bundle_shell() {
                    Some(p) => p,
                    None => {
                        log::info!("shell: no shell/Shell.qml beside this executable; the shell client is not run");
                        return;
                    }
                }
            }
        };
        let runner = PathBuf::from(RUNNER);
        if !runner.exists() {
            self.journal(format!("shell: {} is not on this set; the shell client is not run", runner.display()));
            self.shell.error = Some("no qml-runner".into());
            return;
        }
        let token_path = self.config.runtime_dir.join(TOKEN_FILE);
        let token = match write_token(&token_path) {
            Ok(t) => t,
            Err(e) => {
                self.journal(format!("shell: token file {}: {e}; the shell client is not run", token_path.display()));
                self.shell.error = Some(format!("token: {e}"));
                return;
            }
        };
        let listener = match TcpListener::bind(("127.0.0.1", self.config.shell_port)) {
            Ok(l) => l,
            Err(e) => {
                self.journal(format!("shell: port {}: {e}; the shell client is not run", self.config.shell_port));
                self.shell.error = Some(format!("port {}: {e}", self.config.shell_port));
                return;
            }
        };
        let link = match spawn_server(listener, token) {
            Ok(l) => l,
            Err(e) => {
                self.journal(format!("shell: server: {e}; the shell client is not run"));
                self.shell.error = Some(format!("server: {e}"));
                return;
            }
        };
        let log = self.config.data_dir.join("compositor/shell.log");
        self.shell.link = Some(link);
        self.shell.launch = Some(Launch { runner, main_qml, port: self.config.shell_port, token_path, log });
        self.journal(format!("shell: UI channel on 127.0.0.1:{}", self.config.shell_port));
        self.shell_sync_topics();
        // The client's first frames and WebAppMgr's Home arriving together cost a missed refresh (CompFrames' trial, 2026-10-27): the
        // client starts once SAM's Home has mapped, or START_ALONE after this compositor started.
        self.shell.next_start = Some(Instant::now() + START_ALONE);
    }

    /// Whether SAM's Home (the Home stub) has a mapped window.
    fn sam_home_mapped(&self) -> bool {
        self.scene.windows().any(|w| w.app_id() == crate::scene::HOME_APP && w.map_serial > 0)
    }

    /// The eventfd the server thread writes when a message waits; the loop polls it.
    pub fn shell_fd(&self) -> Option<RawFd> {
        self.shell.link.as_ref().map(|l| l.wake_in.as_raw_fd())
    }

    /// Starts the client on a thread of its own: the fork and the exec of a 32-bit Qt process took the loop 23 ms on the
    /// tenth gate's trial (`loop: 24.5 ms in one pass` at the shell's start), a refresh and a half. The child arrives at a
    /// later tick (`shell_tick`); meanwhile `spawning` says one is on its way.
    fn shell_spawn(&mut self) {
        let Some(l) = &self.shell.launch else { return };
        if self.shell.spawning.is_some() {
            return;
        }
        let now = Instant::now();
        self.shell.starts.retain(|t| now.duration_since(*t) < RESTART_WINDOW);
        if self.shell.starts.len() >= RESTART_LIMIT {
            self.shell.gave_up = true;
            self.journal(format!("shell: {RESTART_LIMIT} starts in {} min, giving up (ctl shell restart tries again)", RESTART_WINDOW.as_secs() / 60));
            return;
        }
        self.shell.starts.push_back(now);
        let job = SpawnJob {
            runner: l.runner.clone(),
            main_qml: l.main_qml.clone(),
            port: l.port,
            token_path: l.token_path.clone(),
            log: l.log.clone(),
            data_dir: self.config.data_dir.clone(),
            runtime_dir: self.config.runtime_dir.clone(),
            socket: self.config.socket.clone(),
        };
        let (tx, rx) = channel();
        let spawner = match &self.shell.spawner {
            Some(s) => Ok(s.clone()),
            None => spawn_thread().inspect(|s| self.shell.spawner = Some(s.clone())),
        };
        match spawner.and_then(|s| s.send((job, tx)).map_err(|_| std::io::Error::other("the spawn thread ended"))) {
            Ok(()) => self.shell.spawning = Some(rx),
            Err(e) => {
                self.shell.spawner = None;
                self.journal(format!("shell: no thread to start the client: {e}"));
                self.shell.error = Some(format!("start: {e}"));
                self.shell.next_start = Some(now + Duration::from_secs(10));
            }
        }
    }

    /// Takes the child (or the failure) a start left on its thread, once it is there.
    fn shell_take_spawned(&mut self, now: Instant) {
        let Some(rx) = &self.shell.spawning else { return };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(SpawnFailure::Start(std::io::Error::other("the spawn thread ended without a result"))),
        };
        self.shell.spawning = None;
        let Some(l) = &self.shell.launch else { return };
        match result {
            Ok(child) => {
                self.shell.stats.starts += 1;
                self.journal(format!("shell: client started, pid {} ({} {})", child.id(), l.runner.display(), l.main_qml.display()));
                self.shell.child = Some(child);
                self.shell.child_started = Some(now);
                self.shell.next_start = None;
            }
            Err(SpawnFailure::NoModule(root)) => {
                self.journal(format!("shell: no LumaHome.qml under {}; the shell client is not run", root.display()));
                self.shell.error = Some("no module".into());
                self.shell.gave_up = true;
            }
            Err(SpawnFailure::Start(e)) => {
                self.journal(format!("shell: {} did not start: {e}", l.runner.display()));
                self.shell.error = Some(format!("start: {e}"));
                self.shell.next_start = Some(now + Duration::from_secs(10));
            }
        }
    }

    /// Supervision, called every tick: a start on its thread is taken in, a client that exited is started again after a
    /// pause, a client that stopped polling is taken as gone (its luna subscriptions cancelled).
    pub fn shell_tick(&mut self, now: Instant) {
        if self.shell.launch.is_none() {
            return;
        }
        self.shell_take_spawned(now);
        if self.shell.last_wait.is_none_or(|t| now.duration_since(t) >= Duration::from_millis(500)) {
            self.shell.last_wait = Some(now);
            let exited = self.shell.child.as_mut().and_then(|c| c.try_wait().ok().flatten());
            if let Some(status) = exited {
                self.shell.child = None;
                self.shell.stats.exits += 1;
                let alive = self.shell.child_started.map(|t| now.duration_since(t)).unwrap_or_default();
                self.journal(format!("shell: client exited ({status}) after {:.0} s", alive.as_secs_f64()));
                self.shell_client_gone("the client exited");
                // A client that ran a while gets a quick restart; one that died at once waits longer each time.
                let pause = if alive > Duration::from_secs(60) { 2 } else { 2u64 << self.shell.starts.len().min(4) };
                self.shell.next_start = Some(now + Duration::from_secs(pause));
            }
        }
        let first = self.shell.starts.is_empty();
        if self.shell.child.is_none()
            && self.shell.spawning.is_none()
            && !self.shell.gave_up
            && self.shell.next_start.is_some_and(|t| now >= t || (first && self.sam_home_mapped()))
        {
            if first {
                self.journal(format!(
                    "shell: starting the client ({})",
                    if self.sam_home_mapped() { "SAM's Home has mapped" } else { "no Home yet, 15 s are up" }
                ));
            }
            self.shell_spawn();
        }
        if self.shell.connected && self.shell.last_seen.is_some_and(|t| now.duration_since(t) > POLL_TIMEOUT) {
            self.shell_client_gone("no poll for 40 s");
        }
    }

    fn shell_client_gone(&mut self, why: &str) {
        crate::server::ime::own_gone(self);
        if self.shell.connected || !self.shell.luna_tags.is_empty() {
            self.journal(format!("shell: client gone ({why}); {} luna subscription(s) cancelled", self.shell.luna_tags.iter().filter(|t| t.subscribe).count()));
        }
        self.shell.connected = false;
        self.shell.can.clear();
        for t in std::mem::take(&mut self.shell.luna_tags) {
            if t.subscribe {
                self.luna.send(ToBridge::Cancel { token: t.token });
            }
        }
        if self.shell.wants {
            self.shell.wants = false;
            if self.scene.set_shell_takes_keys(false) {
                crate::server::seat::refocus(self);
            }
        }
    }

    /// `ctl shell restart`: ends the client and starts it again, whatever the count.
    pub fn shell_restart(&mut self) {
        if let Some(c) = &mut self.shell.child {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.shell.child = None;
        self.shell.starts.clear();
        self.shell.gave_up = false;
        self.shell_client_gone("restart asked for");
        self.shell_spawn();
    }

    /// Takes in what the server thread received. Returns whether a frame was drawn for it (the main loop re-arms its tick).
    pub fn shell_poll(&mut self) -> bool {
        let Some(l) = &self.shell.link else { return false };
        drain(&l.wake_in);
        let mut msgs = Vec::new();
        while let Ok(m) = l.inbox.try_recv() {
            msgs.push(m);
        }
        let mut drew = false;
        for m in msgs {
            drew |= self.shell_handle(m);
        }
        drew
    }

    /// One message from the client (the tests call this directly). Returns whether a frame was drawn for it: the system
    /// layer came live with its frame already in, on a still screen.
    pub fn shell_handle(&mut self, msg: Msg) -> bool {
        let now = Instant::now();
        match msg {
            Msg::Connected => {
                let was = self.shell.connected;
                self.shell.note_seen(now);
                if !was {
                    self.journal("shell: client connected".into());
                }
                // Its session starts from the state: every topic is pushed again so the first poll has them all.
                self.shell.pushed.clear();
                self.shell_sync_topics();
            }
            Msg::Seen => self.shell.note_seen(now),
            Msg::Ui { can, wants, live, state } => {
                self.shell.note_seen(now);
                if let Some(can) = can
                    && can != self.shell.can
                {
                    self.journal(format!("shell: can show {}", if can.is_empty() { "nothing".to_string() } else { can.join(", ") }));
                    self.shell.can = can;
                }
                if let Some(w) = wants
                    && w != self.shell.wants
                {
                    self.shell.wants = w;
                    self.journal(format!("shell: the system layer {} the keys", if w { "wants" } else { "lets go of" }));
                    if self.scene.set_shell_takes_keys(w) {
                        crate::server::seat::refocus(self);
                    }
                }
                // The state goes in before the live word can return: the report that brings the layer live carries the state that
                // says why (the volume display's `osd`), and an identical report is never sent again.
                if let Some(s) = state {
                    self.shell.ui_state = s;
                }
                if let Some(l) = live
                    && l != self.shell.live
                {
                    self.shell.live = l;
                    let now_ns = crate::output::monotonic_ns();
                    let age = self.scene.shell_system_commit_age_ms(now_ns);
                    let fresh = self.scene.set_shell_system_live(l, now_ns);
                    // The focus is looked for among the windows in the stack, which has no idle layer: a layer that wanted the keys
                    // while it was idle (the word `wants` came before the word `live`) has them only once it is back in the stack.
                    crate::server::seat::refocus(self);
                    if l {
                        // Where a toast's first frame goes: the layer's last commit against the word that it is live.
                        self.journal(format!(
                            "shell: system layer live; its last commit {} ms ago, join {}",
                            age.map_or("-".to_string(), |a| a.to_string()),
                            if fresh { "draws it" } else { "quiet" }
                        ));
                    }
                    if fresh && self.output.draws_on_damage(now) {
                        self.tick();
                        return true;
                    }
                }
            }
            Msg::Luna { tag, uri, payload, subscribe } => {
                self.shell.note_seen(now);
                if !allowed(&uri) {
                    self.shell.stats.luna_refused += 1;
                    self.journal(format!("shell: luna {uri} refused (not on the allowlist)"));
                    self.shell
                        .push("luna", json!({"tag": tag, "payload": {"returnValue": false, "errorCode": -1, "errorText": "not allowed by the compositor"}}));
                    return false;
                }
                self.shell.stats.luna_calls += 1;
                let token = self.luna_token();
                self.shell.luna_tags.push(LunaTag { tag, token, subscribe, uri: uri.clone() });
                if !subscribe && self.shell.luna_tags.len() > 256 {
                    // One-shot calls whose answer never came (a service that is down) must not pile up.
                    let n = self.shell.luna_tags.len() - 256;
                    let mut dropped = 0;
                    self.shell.luna_tags.retain(|t| {
                        let keep = t.subscribe || dropped >= n;
                        if !keep {
                            dropped += 1;
                        }
                        keep
                    });
                }
                self.luna.send(ToBridge::Outbound(Outbound { token, uri, payload, subscribe }));
            }
            Msg::Cancel { tag } => {
                self.shell.note_seen(now);
                if let Some(i) = self.shell.luna_tags.iter().position(|t| t.tag == tag) {
                    let t = self.shell.luna_tags.remove(i);
                    if t.subscribe {
                        self.luna.send(ToBridge::Cancel { token: t.token });
                    }
                }
            }
            Msg::Ime(body) => {
                self.shell.note_seen(now);
                crate::server::ime::own_op(self, &body);
            }
        }
        false
    }

    /// An answer to one of the compositor's calls: the shell's, if a tag owns the token. Returns whether it was.
    pub fn shell_luna_event(&mut self, token: u64, payload: &Value) -> bool {
        let Some(i) = self.shell.luna_tags.iter().position(|t| t.token == token) else { return false };
        let tag = self.shell.luna_tags[i].tag;
        if let Some(feed) = notification_feed(&self.shell.luna_tags[i].uri, payload) {
            self.shell.mark(&format!("Q:{feed}-delivered"));
            // The journal's clock against `shell: system layer live; its last commit N ms ago`: the time the client took from the
            // delivery to its first frame of the toast or alert.
            self.journal(format!("shell: {feed} delivered to the client"));
        }
        if !self.shell.luna_tags[i].subscribe {
            self.shell.luna_tags.remove(i);
        }
        self.shell.push("luna", json!({"tag": tag, "payload": payload}));
        true
    }

    /// The audio service's state (the compositor's own subscription): the UI's `LS.audioService` reads it.
    pub fn shell_audio(&mut self, payload: &Value) {
        if self.shell.enabled() || self.shell.connected {
            self.shell.push("audio", payload.clone());
        }
    }

    /// Pushes the topics the UI follows when they changed: what is in front, the screen, the keyboard, the cursor. Called
    /// wherever the scene or the input state changed (`luna_notify` and the shell's own mappings).
    pub fn shell_sync_topics(&mut self) {
        if !self.shell.enabled() && !self.shell.connected {
            return;
        }
        let home_shown = self.shell_home_drawn();
        let covered = !home_shown || self.scene.shell_home_covered(&self.surfaces);
        let fg = json!({"app": self.foreground_app(), "home": home_shown, "covered": covered});
        self.shell.push_if_changed("foreground", fg);
        let screen = json!({"on": self.luna_state.power.rendering});
        self.shell.push_if_changed("screen", screen);
        let keyboard = json!({"up": self.keyboard_up()});
        self.shell.push_if_changed("keyboard", keyboard);
        let cursor = json!({"visible": self.seat.cursor_visible});
        self.shell.push_if_changed("cursor", cursor);
        // The moving wallpaper's hole is under Home: the shell takes its own wallpaper away only while this holds, and puts it back in the
        // frame after it stops (an app taking the front stops it at once: the wallpaper's window is left out of the stack with Home).
        let wall = json!({"hole": home_shown && self.scene.wall_hole()});
        self.shell.push_if_changed("wall", wall);
        // The refresh the output runs at (`refresh`): the shell draws its frames per refresh and trims what a frame costs at 120.
        let refresh = json!({"hz": self.config.refresh_hz});
        self.shell.push_if_changed("refresh", refresh);
        crate::server::ime::own_publish(self);
    }

    /// Whether LumaTV's Home would be drawn now: the shell's Home window is mapped and the foreground card is SAM's Home or there is
    /// none (the scene's rule, `scene.rs` `build_stack`).
    pub fn shell_home_drawn(&self) -> bool {
        let home_mapped = self.scene.shell_window(WindowType::ShellHome).is_some_and(|w| w.map_serial > 0);
        home_mapped && self.scene.foreground(&self.surfaces).is_none_or(|w| w.app_id() == crate::scene::HOME_APP)
    }

    /// The moving wallpaper's window is closed the moment LumaTV's Home is not drawn (an app, an input, LG's Home or Settings took the front):
    /// the player takes its window away and unloads the decoder, so what comes to the front finds the pipeline free. This does not wait for
    /// the shell's word or the player's ttl; the shell asks again for the wallpaper when Home is back. Called after each drawn frame, which is
    /// when the stack that says so changes.
    pub fn wall_handover(&mut self) {
        if self.shell_home_drawn() {
            return;
        }
        let ids: Vec<_> = self.scene.windows().filter(|w| w.kind == WindowType::Wallpaper && w.map_serial > 0).map(|w| w.id).collect();
        for id in ids {
            if let Some(res) = self.webos_shell_resource(id) {
                res.close();
                self.journal(format!("wall: Home is not drawn, close sent to window {}", id.0));
            }
        }
    }

    /// One of the shell's windows got its first buffer.
    pub fn shell_window_mapped(&mut self, kind: WindowType) {
        self.journal(format!("shell: {} window mapped", if kind == WindowType::ShellHome { "Home" } else { "system layer" }));
        self.shell_sync_topics();
    }

    /// Whether the client is connected and said it can show `kind` ("quick", "picker", "drawer") now.
    pub fn shell_can(&self, kind: &str) -> bool {
        self.shell.connected && self.shell.can.iter().any(|c| c == kind)
    }

    /// The gear's tap, the Input key, a request through the luna methods: LumaTV's own panel when the UI says it can show it
    /// (`kind`: "quick", "picker", "drawer"). `warm` is the gear's press: the panel joins the scene off the screen, so the slide
    /// of the opening starts in a scene that has it. Returns whether the UI takes it.
    pub fn shell_show(&mut self, kind: &str, warm: bool) -> bool {
        if !self.shell_can(kind) {
            return false;
        }
        self.shell.stats.shows += 1;
        if !warm {
            self.shell.mark(&format!("Q:{kind}-show"));
        }
        self.shell.push("show", json!({"kind": kind, "warm": warm, "n": self.shell.stats.shows}));
        true
    }

    /// The Home key was pressed while LumaTV's Home has the screen: the UI closes what it has open.
    pub fn shell_home_key(&mut self) {
        if self.shell.connected {
            let n = self.shell.stats.events;
            self.shell.push("homekey", json!({"n": n}));
        }
    }

    /// A line in the kernel's trace buffer for the frame gate (`Q:<name>-show`, `-shown`): the keyboard writes its own, since it is asked for by
    /// a text field and not by a key of the remote.
    pub fn shell_mark(&self, text: &str) {
        self.shell.mark(text);
    }

    /// `ctl shell`.
    pub fn shell_report(&self) -> Value {
        let s = &self.shell;
        json!({
            "ok": true,
            "enabled": s.enabled(),
            "gaveUp": s.gave_up,
            "error": s.error,
            "pid": s.child_pid(),
            "connected": s.connected,
            "can": s.can,
            "wants": s.wants,
            "systemLive": s.live,
            "homeDrawn": self.shell_home_drawn(),
            "homeMapped": self.scene.shell_window(WindowType::ShellHome).is_some_and(|w| w.map_serial > 0),
            "systemMapped": self.scene.shell_window(WindowType::ShellSystem).is_some_and(|w| w.map_serial > 0),
            "homeCommits": self.scene.shell_window(WindowType::ShellHome).map(|w| w.commits).unwrap_or(0),
            "systemCommits": self.scene.shell_window(WindowType::ShellSystem).map(|w| w.commits).unwrap_or(0),
            "ui": s.ui_state,
            "topics": s.pushed,
            "lunaOpen": s.luna_tags.iter().map(|t| json!({"tag": t.tag, "uri": t.uri, "subscribe": t.subscribe})).collect::<Vec<_>>(),
            "stats": {"starts": s.stats.starts, "exits": s.stats.exits, "events": s.stats.events, "lunaCalls": s.stats.luna_calls, "lunaRefused": s.stats.luna_refused, "shows": s.stats.shows},
        })
    }
}

/// The feed of LG's notification service a push belongs to, when the push is a notification the shell shows: a toast with a message,
/// an alert that opens. The compositor marks the trace when one reaches it (`Shell::mark`): the frame gate times the shell's own part
/// of a toast from there, since the service hands it over at its own pace (2 to 4 s after `luna-send` returned under LG's compositor).
fn notification_feed(uri: &str, payload: &Value) -> Option<&'static str> {
    if uri.ends_with("/getToastNotification") && payload.get("message").and_then(Value::as_str).is_some_and(|m| !m.is_empty()) {
        return Some("toast");
    }
    if uri.ends_with("/getAlertNotification") && payload.get("alertAction").and_then(Value::as_str) == Some("open") {
        return Some("alert");
    }
    None
}

/// `<bundle>/shell/Shell.qml` beside this executable (`<bundle>/bin/lumatv-compositor`), when the bundle has one.
fn bundle_shell() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bundle = exe.parent()?.parent()?;
    let p = bundle.join("shell/Shell.qml");
    p.is_file().then_some(p)
}

/// LumaTV's QML module as `tools/lumatv` stages it: the directory under `<data dir>/qml/WebOSCompositor` that holds
/// `LumaHome.qml` (named by the brand's `qmlModule`).
fn module_dir(data_dir: &Path) -> Option<PathBuf> {
    let root = data_dir.join("qml/WebOSCompositor");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root).ok()?.flatten().map(|e| e.path()).filter(|p| p.join("LumaHome.qml").is_file()).collect();
    dirs.sort();
    dirs.into_iter().next()
}

/// What a start needs off the loop's state: the launch and the paths the client is told.
struct SpawnJob {
    runner: PathBuf,
    main_qml: PathBuf,
    port: u16,
    token_path: PathBuf,
    log: PathBuf,
    data_dir: PathBuf,
    runtime_dir: PathBuf,
    socket: String,
}

/// A start asked of the spawn thread, and where its child goes.
type SpawnRequest = (SpawnJob, Sender<Result<Child, SpawnFailure>>);

/// The thread that forks every client start (`Shell::spawner`). It lives until the compositor drops its sender, which is the
/// end of the process: the client's PR_SET_PDEATHSIG is tied to this thread, so the thread must outlive the client.
fn spawn_thread() -> std::io::Result<Sender<SpawnRequest>> {
    let (tx, rx) = channel::<SpawnRequest>();
    std::thread::Builder::new().name("shell-spawn".into()).spawn(move || {
        for (job, reply) in rx {
            let _ = reply.send(spawn_client(job));
        }
    })?;
    Ok(tx)
}

/// Starts the client: the module lookup, the log file and the fork and exec of the runner, on the spawn thread.
fn spawn_client(job: SpawnJob) -> Result<Child, SpawnFailure> {
    let imports = job.main_qml.parent().map(|d| d.join("imports")).unwrap_or_default();
    let Some(module_dir) = module_dir(&job.data_dir) else {
        return Err(SpawnFailure::NoModule(job.data_dir.join("qml/WebOSCompositor")));
    };
    let params = json!({
        "port": job.port,
        "tokenFile": job.token_path,
        "dataDir": job.data_dir,
        "moduleDir": module_dir,
        "runtimeDir": job.runtime_dir,
        "socket": job.socket,
    });
    let arg = json!({"event": "launch", "appId": APP_ID, "main": job.main_qml, "params": params}).to_string();
    let log = std::fs::OpenOptions::new().create(true).append(true).open(&job.log);
    let mut cmd = Command::new(&job.runner);
    cmd.arg(arg)
        .env_remove("NOTIFY_SOCKET")
        .env_remove("LUMATV_LUNA_BRIDGE")
        .env_remove("LUMATV_LOG")
        .env("XDG_RUNTIME_DIR", &job.runtime_dir)
        .env("WAYLAND_DISPLAY", &job.socket)
        // LG's Qt apps' environment, read off the set (`compositor-surfaces-result.md`).
        .env("QT_QPA_PLATFORM", "wayland")
        .env("QT_WAYLAND_SHELL_INTEGRATION", "webos")
        .env("QT_WAYLAND_HARDWARE_INTEGRATION", "wayland-egl")
        .env("QT_WAYLAND_DISABLE_WINDOWDECORATION", "1")
        .env("QT_WAYLAND_XKB_RULE_NAMES", "evdev:lgwebostv:us::")
        .env("QT_IM_MODULE", "wayland")
        .env("QT_QPA_FONTDIR", "/usr/share/fonts")
        .env("QT_HARFBUZZ", "new")
        .env("WEBOS_GL_DISABLE_THREADED_RENDERING", "1")
        .env("QML2_IMPORT_PATH", format!("{}:/usr/lib/qt5/qml", imports.display()))
        .env("LUMATV_SHELL_PORT", job.port.to_string())
        .stdin(Stdio::null());
    match log {
        Ok(f) => {
            let err = f.try_clone().map(Stdio::from).unwrap_or_else(|_| Stdio::null());
            cmd.stdout(Stdio::from(f)).stderr(err);
        }
        Err(_) => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    // The client dies with the compositor: a Qt client left over would hold LG's compositor's socket name after the fallback.
    // Its class is time-shared by the loop's reset-on-fork flag (`sched::fifo`); the nice level is set here.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: prctl and setpriority in the child between fork and exec touch nothing shared.
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                libc::setpriority(libc::PRIO_PROCESS, 0, NICE);
                Ok(())
            });
        }
    }
    cmd.spawn().map_err(SpawnFailure::Start)
}

/// Writes a fresh token (32 hex digits from the kernel's random source) readable by the owner only.
fn write_token(path: &Path) -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let _ = std::fs::remove_file(path);
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        f.write_all(token.as_bytes())?;
    }
    Ok(token)
}

fn eventfd() -> std::io::Result<OwnedFd> {
    // SAFETY: eventfd returns a new descriptor or -1.
    let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the descriptor is ours and not shared.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn wake(fd: &OwnedFd) {
    let one = 1u64.to_ne_bytes();
    // SAFETY: writing 8 bytes to an eventfd.
    unsafe { libc::write(fd.as_raw_fd(), one.as_ptr() as *const libc::c_void, 8) };
}

fn drain(fd: &OwnedFd) {
    let mut buf = [0u8; 8];
    // SAFETY: reading the eventfd's 8-byte counter; EAGAIN when there is none.
    unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, 8) };
}

/// Starts the server thread on `listener`; the handles the loop keeps.
fn spawn_server(listener: TcpListener, token: String) -> std::io::Result<Link> {
    let wake_out = eventfd()?;
    let wake_in = eventfd()?;
    let outbox = Arc::new(Mutex::new(Outbox { seq: 0, events: VecDeque::new(), topics: BTreeMap::new(), boot: boot_ms() }));
    let (tx, rx) = channel();
    let out2 = outbox.clone();
    let wake_out_fd = wake_out.try_clone()?;
    let wake_in_fd = wake_in.try_clone()?;
    listener.set_nonblocking(true)?;
    std::thread::Builder::new().name("lumatv-shell-http".into()).spawn(move || serve(listener, token, out2, wake_out_fd, tx, wake_in_fd))?;
    Ok(Link { outbox, wake_out, inbox: rx, wake_in })
}

fn boot_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// One parsed HTTP request.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub token: String,
    pub body: Vec<u8>,
    pub close: bool,
}

/// Parses one request at the start of `buf`; `None` while it is incomplete, the request and the bytes it took otherwise.
pub fn parse_request(buf: &[u8]) -> Option<(Request, usize)> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&buf[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?;
    let (path, query_text) = target.split_once('?').unwrap_or((target, ""));
    let mut query = BTreeMap::new();
    for pair in query_text.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        query.insert(k.to_string(), v.to_string());
    }
    let mut length = 0usize;
    let mut token = String::new();
    let mut close = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().ok()?,
            "x-luma-token" => token = value.to_string(),
            "connection" => close = value.eq_ignore_ascii_case("close"),
            _ => {}
        }
    }
    let body_start = end + 4;
    if buf.len() < body_start + length {
        return None;
    }
    let body = buf[body_start..body_start + length].to_vec();
    Some((Request { method, path: path.to_string(), query, token, body, close }, body_start + length))
}

fn response(status: u16, body: &Value, close: bool) -> Vec<u8> {
    let text = body.to_string();
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "OK",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{}\r\n{text}",
        text.len(),
        if close { "Connection: close\r\n" } else { "" }
    )
    .into_bytes()
}

/// A connection of the client: what it sent so far, and the long poll it waits on.
struct Conn {
    stream: TcpStream,
    buf: Vec<u8>,
    pending: Option<(u64, Instant)>,
    close_after: bool,
}

/// What a request does, decided on the server thread: an answer now, or a long poll to hold.
enum Handled {
    Answer(u16, Value),
    Poll { after: u64, timeout: Duration },
}

/// The server thread's way to the loop: a message on the channel and a write on the loop's eventfd.
struct Inbox {
    tx: Sender<Msg>,
    wake: OwnedFd,
}

impl Inbox {
    fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        wake(&self.wake);
    }
}

fn handle_request(req: &Request, token: &str, outbox: &Arc<Mutex<Outbox>>, inbox: &Inbox) -> Handled {
    if req.token != token {
        return Handled::Answer(403, json!({"ok": false, "error": "token"}));
    }
    let body: Value = if req.body.is_empty() { Value::Null } else { serde_json::from_slice(&req.body).unwrap_or(Value::Null) };
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/v1/state") => {
            inbox.send(Msg::Connected);
            let o = outbox.lock();
            Handled::Answer(200, json!({"ok": true, "boot": o.boot, "seq": o.seq, "topics": o.topics}))
        }
        ("GET", "/v1/events") => {
            inbox.send(Msg::Seen);
            let after = req.query.get("after").and_then(|v| v.parse().ok()).unwrap_or(0);
            let timeout = req.query.get("timeout").and_then(|v| v.parse().ok()).unwrap_or(25u64).min(60);
            Handled::Poll { after, timeout: Duration::from_secs(timeout) }
        }
        ("POST", "/v1/ui") => {
            let can = body.get("can").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            let wants = body.get("wants").and_then(Value::as_bool);
            let live = body.get("live").and_then(Value::as_bool);
            let state = body.get("state").cloned();
            inbox.send(Msg::Ui { can, wants, live, state });
            Handled::Answer(200, json!({"ok": true}))
        }
        ("POST", "/v1/luna") => {
            let (Some(tag), Some(uri)) = (body.get("tag").and_then(Value::as_u64), body.get("uri").and_then(Value::as_str)) else {
                return Handled::Answer(400, json!({"ok": false, "error": "tag and uri"}));
            };
            let payload = body.get("payload").cloned().unwrap_or(json!({}));
            let subscribe = body.get("subscribe").and_then(Value::as_bool).unwrap_or(false);
            inbox.send(Msg::Luna { tag, uri: uri.to_string(), payload, subscribe });
            Handled::Answer(200, json!({"ok": true, "tag": tag}))
        }
        ("POST", "/v1/cancel") => {
            let Some(tag) = body.get("tag").and_then(Value::as_u64) else {
                return Handled::Answer(400, json!({"ok": false, "error": "tag"}));
            };
            inbox.send(Msg::Cancel { tag });
            Handled::Answer(200, json!({"ok": true}))
        }
        ("POST", "/v1/ime") => {
            inbox.send(Msg::Ime(body));
            Handled::Answer(200, json!({"ok": true}))
        }
        _ => Handled::Answer(404, json!({"ok": false, "error": "no such route"})),
    }
}

/// The answer to a long poll when there is one: events after `after`, or a reset when the client is too far behind.
fn poll_answer(outbox: &Arc<Mutex<Outbox>>, after: u64, now_due: bool) -> Option<Value> {
    let o = outbox.lock();
    match o.since(after) {
        None => Some(json!({"ok": true, "reset": true, "boot": o.boot, "seq": o.seq, "events": []})),
        Some(events) if !events.is_empty() || now_due => Some(json!({"ok": true, "boot": o.boot, "seq": o.seq, "events": events})),
        Some(_) => None,
    }
}

/// The server thread: every connection and the wake-up in one `poll`, requests parsed and answered here, long polls held
/// until the outbox grows or their timeout.
fn serve(listener: TcpListener, token: String, outbox: Arc<Mutex<Outbox>>, wake_out: OwnedFd, tx: Sender<Msg>, wake_in: OwnedFd) {
    let inbox = Inbox { tx, wake: wake_in };
    let mut conns: Vec<Conn> = Vec::new();
    loop {
        let mut fds: Vec<libc::pollfd> = Vec::with_capacity(conns.len() + 2);
        fds.push(libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        fds.push(libc::pollfd { fd: wake_out.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        for c in &conns {
            fds.push(libc::pollfd { fd: c.stream.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        }
        let now = Instant::now();
        let timeout_ms =
            conns.iter().filter_map(|c| c.pending.map(|(_, d)| d.saturating_duration_since(now).as_millis() as i32)).min().unwrap_or(1000).clamp(1, 1000);
        // SAFETY: fds is a valid array of pollfd for the call's duration.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if n < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return;
        }
        if fds[0].revents & libc::POLLIN != 0 {
            while let Ok((s, _)) = listener.accept() {
                let _ = s.set_nodelay(true);
                conns.push(Conn { stream: s, buf: Vec::new(), pending: None, close_after: false });
            }
        }
        if fds[1].revents & libc::POLLIN != 0 {
            drain(&wake_out);
        }
        let mut closed = Vec::new();
        // Connections accepted in this pass have no entry in `fds` yet; they are polled from the next one.
        for i in 0..fds.len() - 2 {
            let c = &mut conns[i];
            let revents = fds[i + 2].revents;
            if revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                let mut chunk = [0u8; 4096];
                match c.stream.read(&mut chunk) {
                    Ok(0) | Err(_) => {
                        closed.push(i);
                        continue;
                    }
                    Ok(k) => c.buf.extend_from_slice(&chunk[..k]),
                }
                if c.buf.len() > 1 << 20 {
                    closed.push(i);
                    continue;
                }
                while let Some((req, used)) = parse_request(&c.buf) {
                    c.buf.drain(..used);
                    c.close_after |= req.close;
                    match handle_request(&req, &token, &outbox, &inbox) {
                        Handled::Answer(status, body) => {
                            if c.stream.write_all(&response(status, &body, c.close_after)).is_err() {
                                closed.push(i);
                                break;
                            }
                        }
                        Handled::Poll { after, timeout } => {
                            // A second poll on the same connection replaces the first: the client abandoned it.
                            c.pending = Some((after, Instant::now() + timeout));
                        }
                    }
                }
            }
            if let Some((after, deadline)) = c.pending {
                let due = Instant::now() >= deadline;
                if let Some(body) = poll_answer(&outbox, after, due) {
                    c.pending = None;
                    inbox.send(Msg::Seen);
                    if c.stream.write_all(&response(200, &body, c.close_after)).is_err() {
                        closed.push(i);
                    }
                }
            }
            if c.close_after && c.pending.is_none() {
                closed.push(i);
            }
        }
        closed.sort_unstable();
        closed.dedup();
        for i in closed.into_iter().rev() {
            conns.remove(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_names_services_not_methods() {
        assert!(allowed("luna://com.webos.applicationManager/launch"));
        assert!(allowed("luna://com.webos.service.voiceconductor/system/recognizeVoiceWithDetails"));
        assert!(allowed("luna://com.webos.service.audio/tv/getSoundCardsState"));
        assert!(allowed("luna://com.webos.service.sm/deviceid/getIDs"));
        assert!(allowed("luna://com.palm.systemservice/time/getEffectiveBroadcastTime"));
        assert!(!allowed("luna://com.webos.service.secret/anything"));
        assert!(!allowed("com.webos.applicationManager/launch"));
        assert!(!allowed("luna://com.webos.applicationManagerX/launch"));
    }

    #[test]
    fn the_homebrew_channels_service_is_open_for_its_settings_only() {
        assert!(allowed("luna://org.webosbrew.hbchannel.service/getConfiguration"));
        assert!(allowed("luna://org.webosbrew.hbchannel.service/setConfiguration"));
        assert!(!allowed("luna://org.webosbrew.hbchannel.service/exec"));
        assert!(!allowed("luna://org.webosbrew.hbchannel.service/spawn"));
        assert!(!allowed("luna://org.webosbrew.hbchannel.service"));
    }

    #[test]
    fn parses_a_request_with_a_body_and_a_query() {
        let text = b"POST /v1/luna?x=1&y HTTP/1.1\r\nHost: a\r\nX-Luma-Token: abc\r\nContent-Length: 7\r\nConnection: close\r\n\r\n{\"a\":1}GET /v1/state HTTP/1.1\r\n\r\n";
        let (req, used) = parse_request(text).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/v1/luna");
        assert_eq!(req.query.get("x").map(String::as_str), Some("1"));
        assert_eq!(req.query.get("y").map(String::as_str), Some(""));
        assert_eq!(req.token, "abc");
        assert_eq!(req.body, b"{\"a\":1}");
        assert!(req.close);
        let (second, used2) = parse_request(&text[used..]).unwrap();
        assert_eq!(second.method, "GET");
        assert_eq!(used + used2, text.len());
        assert!(parse_request(b"GET /v1/state HTTP/1.1\r\nContent-Length: 5\r\n\r\nab").is_none());
        assert!(parse_request(b"GET /v1/state HTTP/1.1\r\nHost").is_none());
    }

    #[test]
    fn outbox_keeps_a_window_and_resets_a_client_too_far_behind() {
        let mut o = Outbox { seq: 0, events: VecDeque::new(), topics: BTreeMap::new(), boot: 1 };
        for i in 0..(EVENTS_KEPT as u64 + 10) {
            o.push("t", json!(i));
        }
        assert_eq!(o.seq, EVENTS_KEPT as u64 + 10);
        assert_eq!(o.since(o.seq).unwrap().len(), 0);
        assert_eq!(o.since(o.seq - 3).unwrap().len(), 3);
        assert!(o.since(0).is_none(), "the first events are gone: a reset");
        assert_eq!(o.since(10).unwrap().len(), EVENTS_KEPT, "the oldest kept is seq 11");
        assert_eq!(o.topics.get("t"), Some(&json!(EVENTS_KEPT as u64 + 9)));
    }

    #[test]
    fn a_token_mismatch_is_refused_before_anything_is_read() {
        let outbox = Arc::new(Mutex::new(Outbox { seq: 0, events: VecDeque::new(), topics: BTreeMap::new(), boot: 1 }));
        let (tx, rx) = channel();
        let inbox = Inbox { tx, wake: eventfd().unwrap() };
        let req = Request { method: "GET".into(), path: "/v1/state".into(), query: BTreeMap::new(), token: "wrong".into(), body: Vec::new(), close: false };
        match handle_request(&req, "right", &outbox, &inbox) {
            Handled::Answer(403, _) => {}
            _ => panic!("refused"),
        }
        assert!(rx.try_recv().is_err());
        let req = Request { token: "right".into(), ..req };
        match handle_request(&req, "right", &outbox, &inbox) {
            Handled::Answer(200, body) => assert_eq!(body["seq"], 0),
            _ => panic!("answered"),
        }
        assert_eq!(rx.try_recv().unwrap(), Msg::Connected);
    }

    #[test]
    fn the_server_answers_over_tcp_and_holds_a_long_poll() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let link = spawn_server(listener, "tok".into()).unwrap();
        fn get(port: u16, path: &str) -> (u16, Value) {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(format!("GET {path} HTTP/1.1\r\nX-Luma-Token: tok\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
            let mut text = String::new();
            s.read_to_string(&mut text).unwrap();
            let status: u16 = text.split(' ').nth(1).unwrap().parse().unwrap();
            let body = text.split("\r\n\r\n").nth(1).unwrap();
            (status, serde_json::from_str(body).unwrap())
        }
        let (status, body) = get(port, "/v1/state");
        assert_eq!(status, 200);
        assert_eq!(body["seq"], 0);
        assert_eq!(link.inbox.recv_timeout(Duration::from_secs(2)).unwrap(), Msg::Connected);
        // A poll with nothing to say waits for the outbox.
        let t0 = Instant::now();
        let poller = std::thread::spawn(move || get(port, "/v1/events?after=0&timeout=5"));
        std::thread::sleep(Duration::from_millis(150));
        link.outbox.lock().push("foreground", json!({"app": "x"}));
        wake(&link.wake_out);
        let (status, body) = poller.join().unwrap();
        assert_eq!(status, 200);
        assert_eq!(body["events"][0]["topic"], "foreground");
        assert!(t0.elapsed() >= Duration::from_millis(100) && t0.elapsed() < Duration::from_secs(4), "held until the push, not until the timeout");
        // The wake-in eventfd was written for the messages.
        let mut fds = [libc::pollfd { fd: link.wake_in.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
        // SAFETY: one pollfd.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 1, 1000) };
        assert_eq!(n, 1);
    }

    #[test]
    fn the_client_is_started_off_the_loop_and_arrives_at_a_later_tick() {
        // The runner is `sh`, handed the launch JSON as its script: it exits at once, which is all the loop's side needs.
        let dir = std::env::temp_dir().join(format!("lumatv-shell-spawn-{}", std::process::id()));
        let module = dir.join("qml/WebOSCompositor/lumatv");
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(module.join("LumaHome.qml"), "").unwrap();
        let display: wayland_server::Display<State> = wayland_server::Display::new().unwrap();
        let config = crate::config::Config { data_dir: dir.clone(), runtime_dir: dir.clone(), ..crate::config::Config::default() };
        let mut state = State::new(display.handle(), config, Box::new(crate::render::software::Software::new(64, 64)), Box::new(crate::luna::Null::default()));
        state.shell.launch = Some(Launch {
            runner: PathBuf::from("/bin/sh"),
            main_qml: dir.join("Shell.qml"),
            port: 1,
            token_path: dir.join("token"),
            log: dir.join("shell.log"),
        });
        state.shell.next_start = Some(Instant::now());
        let t0 = Instant::now();
        state.shell_tick(Instant::now());
        let pass = t0.elapsed();
        assert!(state.shell.child.is_none() && state.shell.spawning.is_some(), "the start is on its thread, the tick returned without a child");
        assert!(state.journal.iter().any(|l| l.contains("shell: starting the client")));
        let mut arrived = false;
        for _ in 0..200 {
            state.shell_tick(Instant::now());
            if state.shell.child.is_some() {
                arrived = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(arrived, "the child came through the channel");
        assert!(state.shell.spawning.is_none());
        assert_eq!(state.shell.stats.starts, 1);
        assert!(state.journal.iter().any(|l| l.contains("shell: client started, pid")), "{:?}", state.journal);
        // No second start while one was on its way, and the pass that asked for it was not held for the exec.
        assert_eq!(state.shell.starts.len(), 1);
        assert!(pass < Duration::from_millis(50), "the tick took {pass:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_client_outlives_the_start_that_forked_it() {
        // PR_SET_PDEATHSIG fires when the thread that forked the client ends: on the set the client was killed 0 s after
        // its start (round 6) while the spawn thread of each start ended. The runner here is a script that sleeps.
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("lumatv-shell-outlive-{}", std::process::id()));
        let module = dir.join("qml/WebOSCompositor/lumatv");
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(module.join("LumaHome.qml"), "").unwrap();
        let runner = dir.join("runner.sh");
        std::fs::write(&runner, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&runner, std::fs::Permissions::from_mode(0o755)).unwrap();
        let display: wayland_server::Display<State> = wayland_server::Display::new().unwrap();
        let config = crate::config::Config { data_dir: dir.clone(), runtime_dir: dir.clone(), ..crate::config::Config::default() };
        let mut state = State::new(display.handle(), config, Box::new(crate::render::software::Software::new(64, 64)), Box::new(crate::luna::Null::default()));
        state.shell.launch = Some(Launch { runner, main_qml: dir.join("Shell.qml"), port: 1, token_path: dir.join("token"), log: dir.join("shell.log") });
        state.shell.next_start = Some(Instant::now());
        for _ in 0..200 {
            state.shell_tick(Instant::now());
            if state.shell.child.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(300));
        let child = state.shell.child.as_mut().expect("the client started");
        assert!(child.try_wait().unwrap().is_none(), "the client is still running 300 ms after its start");
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
