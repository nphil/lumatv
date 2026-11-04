//! Native shell state, events and shared UI channel.

use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::State;
use crate::window::WindowType;

pub(crate) const NICE: i32 = 10;
/// Events kept for a client that is behind; one that is further behind is told to read the state again.
pub(crate) const EVENTS_KEPT: usize = 512;
/// The kernel's trace buffer. The frame gate's captures (`docs/research/perf/capture.sh`) read their markers from it; the compositor
/// leaves two kinds of its own there (`Q:<panel>-show` when it asks the shell for a panel on a key, `Q:<feed>-delivered` when a
/// notification reaches it from the service), so the gate can tell what the shell is asked, and when, from what the shell does with it.
pub const TRACE_MARKER: &str = "/sys/kernel/debug/tracing/trace_marker";
/// The keys the system layer may ask to take while an app has the focus (`Msg::Takes`): the overlay cards' OK and Back. The arrows stay the
/// app's, so a doorbell card never costs the person the film's controls. The Rust shell's Home also names the keys `keys.conf` teaches by name
/// (`keys::SHELL_KEYS`): the microphone key while the helper can listen, the on-screen remote's and Search's while Home is in front, and the
/// channel rocker and Info while the channel guide is up. `wheel`, `wheelup` and `wheeldown` are no keys: they name a turn of the Magic
/// Remote's wheel (`State::shell_rust_wheel`, with its direction `State::shell_rust_wheel_direction`), which the Rust shell's display lab and
/// Home's smart-home card ask for.
pub const TAKEABLE: &[&str] = &["ok", "back", "voice", "screenremote", "search", "chup", "chdown", "info", "wheel", "wheelup", "wheeldown"];
/// A press the layer took whose release has not come for this long was a release lost: the next press is a new one, not a repeat.
pub(crate) const TOOK_STALE: Duration = Duration::from_secs(3);

/// Services permitted through the native UI bridge. The compositor handles its own names in `luna_methods.rs`.
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
    // whether the app in front plays its video (`getMediaSessionId`, `getMediaPlayStatus`): the Cameras full view pauses only a video that plays
    "com.webos.service.mediacontroller",
    "lg.thinqai.adapter",
    // LG's voice service, for the keyboard's microphone key (maliit's own dictation card calls it the same way)
    "com.webos.service.voiceconductor",
];

/// Single methods of a service that is not open as a whole. The Homebrew Channel's service also runs commands as root (`exec`, `spawn`), so
/// only its two settings calls are open, for Settings > System > Support > Update's block switch (`blockUpdates`): without them the page
/// reads "Homebrew Channel: not allowed by the compositor" and cannot change the flag.
///
/// Also the two services behind LG's hidden support menu's items, which Settings > System > About and Reset have now: the factory data's
/// read (model, serial number) and the AV reset's video call (the rest of the AV reset is the audio, input and picture services' own).
pub const ALLOWED_METHODS: &[&str] = &[
    "org.webosbrew.hbchannel.service/getConfiguration",
    "org.webosbrew.hbchannel.service/setConfiguration",
    "com.webos.service.factorymanager/getFactoryOpt",
    "com.webos.service.videooutput/getStatus",
    "com.webos.service.videooutput/reset/resetVideoSettings",
    // The channel guide's and Live TV banner's reads: the channels and programmes LG's own guide shows (`iepg`) and the channel the tuner was on.
    // The rest of those services (favourites, reservations, tuning) is not open.
    "com.webos.service.iepg/getChannelList",
    "com.webos.service.iepg/getChannelInfo",
    "com.webos.service.iepg/getEventList",
    "com.webos.service.iepg/getCurrentEventList",
    "com.webos.service.iepg/getCurrentNextEventList",
    "com.webos.service.utp.channel/getLastChannelId",
    "com.webos.service.utp.broadcast/getCurrentChannel",
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
    /// The native UI has committed its first frame.
    FrameReady,
    /// `POST /v1/ui`
    Ui { can: Option<Vec<String>>, wants: Option<bool>, live: Option<bool>, state: Option<Value> },
    /// `POST /v1/luna`.
    Luna { tag: u64, uri: String, payload: Value, subscribe: bool },
    /// `POST /v1/cancel`.
    Cancel { tag: u64 },
    /// `POST /v1/ime`: the shell's keyboard (`ime::own_op`).
    Ime(Value),
    /// `POST /v1/switcher`: the app switcher's requests (`switcher.rs`): open, layout, close, activate, quit, quitall.
    Switcher(Value),
    /// `POST /v1/vwin`: a claim for one of the display's hardware video windows, its move or its release (`vwin::State::vwin_command`); the answer is
    /// the topic `vwin`.
    Vwin(Value),
    /// `POST /v1/front {app}`: the app that is running comes back to the front without a launch (`server::webos_shell::front_app`).
    Front(String),
    /// `POST /v1/appkey {app, key}`: a transport key for the app in front (`server::webos_shell::app_key`).
    AppKey { app: String, key: String },
    /// `POST /v1/ui` with a `takes` list: the keys, by name (`ok`, `back`), the system layer wants as `syskey` events while an app keeps
    /// the keyboard focus (the overlay cards' OK and Back). A list that is empty is the layer letting them go.
    Takes(Vec<String>),
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

/// Topics that are things happening, not state: a client that (re)connects must not be told of them again (a replayed `homekey`
/// opens the switcher, a replayed `menu.open` or `confirm.ask` raises a dialog nobody asked for), so they are events only and
/// never in the snapshot `GET /v1/state` answers.
const ONE_SHOT_TOPICS: &[&str] = &["show", "homekey", "imekey", "luna", "menu.open", "confirm.ask"];

impl Outbox {
    pub(crate) fn push(&mut self, topic: &str, data: Value) -> u64 {
        self.seq += 1;
        let state = !ONE_SHOT_TOPICS.contains(&topic);
        self.events.push_back((self.seq, topic.to_string(), data.clone()));
        while self.events.len() > EVENTS_KEPT {
            self.events.pop_front();
        }
        if state {
            self.topics.insert(topic.to_string(), data);
        }
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

pub(crate) struct Link {
    pub(crate) outbox: Arc<Mutex<Outbox>>,
    /// Written when the outbox grew: the server thread answers its pending long polls.
    pub(crate) wake_out: OwnedFd,
    pub(crate) inbox: Receiver<Msg>,
    /// Written by the server thread when the inbox grew; the loop polls it ([`State::shell_fd`]).
    pub(crate) wake_in: OwnedFd,
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
    pub luna_cancelled_early: u64,
    pub shows: u64,
}

/// The main loop's side.
impl State {
    /// Whether SAM's Home (the Home stub) has a mapped window.
    pub(crate) fn sam_home_mapped(&self) -> bool {
        self.scene.windows().any(|w| w.app_id() == crate::scene::HOME_APP && w.map_serial > 0)
    }

    /// Deliver an answer to the native UI client.
    pub fn shell_luna_event(&mut self, token: u64, payload: &Value) -> bool {
        self.shell_rust_luna_event(token, payload)
    }

    /// Forward audio updates to the native UI.
    pub fn shell_audio(&mut self, payload: &Value) {
        if self.shell_rust.enabled() {
            self.shell_rust.push("audio", payload.clone());
        }
    }

    /// The topics both clients follow, computed once: what is in front, the transitions, the screen, the keyboard, the cursor, the
    /// wallpaper's hole, the refresh.
    pub fn shell_topic_values(&mut self) -> Vec<(String, Value)> {
        let home_shown = self.shell_home_drawn();
        // Under an app card that is still growing (a transition holds Home as its underlay) Home is still on the screen: it is told so, and the
        // video wallpaper's player is not stopped, until the card is whole.
        let covered = !home_shown || (!self.scene.transitions.holds_home() && self.scene.shell_home_covered(&self.surfaces));
        let motion = self.scene.transitions.settings;
        vec![
            ("foreground".into(), json!({"app": self.foreground_app(), "home": home_shown, "covered": covered})),
            ("transitions".into(), json!({"on": motion.enabled, "reduced": motion.reduced})),
            ("screen".into(), json!({"on": self.luna_state.power.rendering})),
            ("keyboard".into(), json!({"up": self.keyboard_up()})),
            ("cursor".into(), json!({"visible": self.seat.cursor_visible})),
            // The moving wallpaper's hole is under Home: the shell takes its own wallpaper away only while this holds, and puts it back in
            // the frame after it stops (an app taking the front stops it at once: the wallpaper's window is left out of the stack with Home).
            ("wall".into(), json!({"hole": home_shown && self.scene.wall_hole()})),
            // The hardware video windows (`vwin`): the mode, how many windows are free, and each claim's state, window, limits and reason. A client that
            // asked reads its grant here and shows stills when it was denied or the claim was taken back.
            ("vwin".into(), self.vwin_topic()),
            // The refresh the output runs at (`refresh`): the shell draws its frames per refresh and trims what a frame costs at 120.
            ("refresh".into(), json!({"hz": self.config.refresh_hz})),
            // The kinds the Rust shell answers: the QML client leaves out what the Rust client draws (its cards and its camera pop-up read the
            // helper's topics, which cannot be withheld, so this is how it learns not to).
            ("rust".into(), json!({"kinds": if self.shell_rust.enabled() { self.shell_rust.kinds.clone() } else { Vec::new() }})),
        ]
    }

    /// Publish the native UI topics and capture state.
    pub fn shell_sync_topics(&mut self) {
        self.shell_rust_sync_topics();
        crate::server::ime::own_publish(self);
        self.screencap_publish();
    }

    /// Whether native Home is drawn under the current foreground or its transition.
    pub fn shell_home_drawn(&self) -> bool {
        self.scene.home_rust() && (self.scene.foreground(&self.surfaces).is_none_or(|w| w.app_id() == crate::scene::HOME_APP) || self.scene.transitions.holds_home())
    }

    /// Whether native Home hears the Home key.
    pub fn shell_hears_home_key(&self) -> bool {
        self.shell_rust.connected && self.shell_home_drawn()
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

    /// Whether the native UI can show this kind now.
    pub fn shell_can(&self, kind: &str) -> bool {
        self.shell_rust.answers(kind) && self.shell_rust.connected && self.shell_rust.can.iter().any(|c| c == kind)
    }

    /// The gear's tap, the Input key, a request through the luna methods: LumaTV's own panel when the UI says it can show it
    /// (`kind`: "quick", "picker", "drawer"). `warm` is the gear's press: the panel joins the scene off the screen, so the slide
    /// of the opening starts in a scene that has it. Returns whether the UI takes it.
    pub fn shell_show(&mut self, kind: &str, warm: bool) -> bool {
        self.shell_show_page(kind, warm, None)
    }

    /// [`State::shell_show`] for a kind that opens at a page of its own (`settings`: `privacy`, `picture`, ...).
    pub fn shell_show_page(&mut self, kind: &str, warm: bool, page: Option<&str>) -> bool {
        if !self.shell_can(kind) {
            return false;
        }
        self.shell_rust.stats.shows += 1;
        if !warm {
            self.shell_rust.mark(&format!("Q:{kind}-show"));
        }
        let mut event = json!({"kind": kind, "warm": warm, "n": self.shell_rust.stats.shows});
        if let Some(page) = page {
            event["page"] = json!(page);
        }
        self.shell_rust.push("show", event);
        true
    }

    /// The Home key was pressed while LumaTV's Home has the screen: the UI closes what it has open.
    pub fn shell_home_key(&mut self) {
        self.shell_home_key_offer(false);
    }

    /// [`State::shell_home_key`], saying whether the switcher is on offer: Home was pressed at rest with apps open, and the UI, which knows what
    /// Home has open, may open the switcher instead of closing something.
    pub fn shell_home_key_offer(&mut self, switcher: bool) {
        // the Rust client closes what it has open on the Home key as well (the saver wakes, the Cameras app closes)
        if self.shell_rust.connected {
            let n = self.shell_rust.stats.events;
            self.shell_rust.push("homekey", json!({"n": n, "switcher": switcher}));
        }
    }

    /// Write a trace marker for UI events.
    pub fn shell_mark(&self, text: &str) {
        self.shell_rust.mark(text);
    }

    /// UI state for diagnostics.
    pub fn shell_report(&self) -> Value {
        let mut report = self.shell_rust_report();
        report["ok"] = json!(true);
        report["homeDrawn"] = json!(self.shell_home_drawn());
        report["bootDirect"] = self.boot_direct_report();
        report
    }
}

/// The feed of LG's notification service a push belongs to, when the push is a notification the shell shows: a toast with a message,
/// an alert that opens. The compositor marks the trace when one reaches it (`Shell::mark`): the frame gate times the shell's own part
/// of a toast from there, since the service hands it over at its own pace (2 to 4 s after `luna-send` returned under LG's compositor).
pub(crate) fn notification_feed(uri: &str, payload: &Value) -> Option<&'static str> {
    if uri.ends_with("/getToastNotification") && payload.get("message").and_then(Value::as_str).is_some_and(|m| !m.is_empty()) {
        return Some("toast");
    }
    if uri.ends_with("/getAlertNotification") && payload.get("alertAction").and_then(Value::as_str) == Some("open") {
        return Some("alert");
    }
    None
}

/// Directory containing the native UI assets.
pub(crate) fn module_dir(data_dir: &Path) -> Option<PathBuf> {
    let dir = data_dir.join("compositor/assets/ui");
    dir.is_dir().then_some(dir)
}

/// Writes a fresh token (32 hex digits from the kernel's random source) readable by the owner only.
pub(crate) fn write_token(path: &Path) -> std::io::Result<String> {
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

pub(crate) fn eventfd() -> std::io::Result<OwnedFd> {
    // SAFETY: eventfd returns a new descriptor or -1.
    let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the descriptor is ours and not shared.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

pub(crate) fn wake(fd: &OwnedFd) {
    let one = 1u64.to_ne_bytes();
    // SAFETY: writing 8 bytes to an eventfd.
    unsafe { libc::write(fd.as_raw_fd(), one.as_ptr() as *const libc::c_void, 8) };
}

pub(crate) fn drain(fd: &OwnedFd) {
    let mut buf = [0u8; 8];
    // SAFETY: reading the eventfd's 8-byte counter; EAGAIN when there is none.
    unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, 8) };
}

/// Starts the server thread on `listener`; the handles the loop keeps.
pub(crate) fn spawn_server(listener: TcpListener, token: String) -> std::io::Result<Link> {
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
        ("POST", "/v1/frame-ready") => {
            inbox.send(Msg::FrameReady);
            Handled::Answer(200, json!({"ok": true}))
        }
        ("POST", "/v1/ui") => {
            let can = body.get("can").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
            let wants = body.get("wants").and_then(Value::as_bool);
            let live = body.get("live").and_then(Value::as_bool);
            let state = body.get("state").cloned();
            inbox.send(Msg::Ui { can, wants, live, state });
            if let Some(list) = body.get("takes").and_then(Value::as_array) {
                inbox.send(Msg::Takes(list.iter().filter_map(Value::as_str).map(str::to_string).collect()));
            }
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
        ("POST", "/v1/switcher") => {
            inbox.send(Msg::Switcher(body));
            Handled::Answer(200, json!({"ok": true}))
        }
        ("POST", "/v1/vwin") => {
            inbox.send(Msg::Vwin(body));
            Handled::Answer(200, json!({"ok": true}))
        }
        ("POST", "/v1/front") => {
            let Some(app) = body.get("app").and_then(Value::as_str).filter(|a| !a.is_empty()) else {
                return Handled::Answer(400, json!({"ok": false, "error": "app"}));
            };
            inbox.send(Msg::Front(app.to_string()));
            Handled::Answer(200, json!({"ok": true}))
        }
        ("POST", "/v1/appkey") => {
            let app = body.get("app").and_then(Value::as_str).filter(|a| !a.is_empty());
            let key = body.get("key").and_then(Value::as_str).filter(|k| ["pause", "play"].contains(k));
            let (Some(app), Some(key)) = (app, key) else {
                return Handled::Answer(400, json!({"ok": false, "error": "app and key (pause or play)"}));
            };
            inbox.send(Msg::AppKey { app: app.to_string(), key: key.to_string() });
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
        assert!(allowed("luna://com.webos.service.factorymanager/getFactoryOpt") && allowed("luna://com.webos.service.videooutput/reset/resetVideoSettings"));
        assert!(!allowed("luna://com.webos.service.videooutput/setDisplayWindow") && !allowed("luna://com.webos.service.factorymanager/setFactoryOpt"));
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
    fn the_guides_reads_are_open_and_the_rest_of_its_services_is_not() {
        for read in ["getChannelList", "getChannelInfo", "getEventList", "getCurrentEventList", "getCurrentNextEventList"] {
            assert!(allowed(&format!("luna://com.webos.service.iepg/{read}")), "{read}");
        }
        assert!(allowed("luna://com.webos.service.utp.channel/getLastChannelId") && allowed("luna://com.webos.service.utp.broadcast/getCurrentChannel"));
        assert!(!allowed("luna://com.webos.service.iepg/setFavoriteChannels") && !allowed("luna://com.webos.service.utp.channel/setCurrentChannel"));
        assert!(!allowed("luna://com.webos.service.iepg") && !allowed("luna://com.webos.service.utp.broadcast/setBroadcastChannel"));
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
    fn one_shot_topics_are_events_but_not_state() {
        let mut o = Outbox { seq: 0, events: VecDeque::new(), topics: BTreeMap::new(), boot: 1 };
        for topic in ["show", "homekey", "imekey", "luna", "menu.open", "confirm.ask"] {
            o.push(topic, json!({"n": 1}));
        }
        o.push("foreground", json!({"app": "x"}));
        assert_eq!(o.since(0).unwrap().len(), 7, "all of them reach a client that is following");
        assert_eq!(o.topics.keys().collect::<Vec<_>>(), ["foreground"], "a client that connects later is given state only");
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
    fn frame_ready_requires_authentication_and_post() {
        let outbox = Arc::new(Mutex::new(Outbox { seq: 0, events: VecDeque::new(), topics: BTreeMap::new(), boot: 1 }));
        let (tx, rx) = channel();
        let inbox = Inbox { tx, wake: eventfd().unwrap() };
        let req = Request { method: "POST".into(), path: "/v1/frame-ready".into(), query: BTreeMap::new(), token: "wrong".into(), body: Vec::new(), close: false };
        assert!(matches!(handle_request(&req, "right", &outbox, &inbox), Handled::Answer(403, _)));
        assert!(rx.try_recv().is_err());
        let req = Request { token: "right".into(), method: "GET".into(), ..req };
        assert!(matches!(handle_request(&req, "right", &outbox, &inbox), Handled::Answer(404, _)));
        assert!(rx.try_recv().is_err());
        let req = Request { method: "POST".into(), ..req };
        assert!(matches!(handle_request(&req, "right", &outbox, &inbox), Handled::Answer(200, _)));
        assert_eq!(rx.try_recv().unwrap(), Msg::FrameReady);
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

}
