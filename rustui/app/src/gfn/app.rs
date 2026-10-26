//! The GeForce NOW app's rules, with no window, no GPU and no core process in them: what the remote's keys and the pad do on each
//! screen, which request goes to the core when, and what each answer changes. `gfn_ui.rs` draws the screens, `bin/gfn.rs` runs the
//! loop; the tests here drive the rules with the core's answers written by hand, in the shapes OpenNOW's core documents
//! (`docs/core-protocol.md`) and sends (`native/opennow-core/src/{gfn.rs,gfn/catalog.rs,cloudmatch.rs,streamer.rs}`).
//!
//! The launch sequence is the one OpenNOW's own shell runs (`opennow-qt/qml/state/ShellStore.qml`): `catalog.launch.inspect`, then
//! `session.create` (acknowledged with an `ack` as the protocol demands), `session.poll` until the seat is ready (status 2 or 3 with
//! endpoints), `streamer.start {session}`. Stopping is `streamer.stop` and `session.stop`. HDR is only resolved against the runtime
//! capabilities the shell passes, so `session.create` carries the engine's report plus `nativeHdrSupported` and the display's nits.
use crate::home::Key;
use serde_json::{Value, json};
use std::time::Duration;

/// The screens, in the order Back walks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// The core is starting, or an answer the next screen needs is on its way.
    Starting,
    SignIn,
    Library,
    Detail,
    Launching,
    Stream,
    /// The overlay over the stream (Guide, Back, or the exit chord).
    Overlay,
    Error,
}

/// What the overlay offers, top to bottom.
pub const OVERLAY_ROWS: [&str; 3] = ["Continue", "Stats", "Quit game"];

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Game {
    pub id: String,
    pub variant_id: String,
    pub title: String,
    pub image_url: String,
    pub store: String,
    pub in_library: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SignIn {
    pub attempt_id: String,
    pub user_code: String,
    pub url: String,
    pub qr_rows: Vec<String>,
    pub expires_at_ms: u64,
    pub interval_s: u64,
    pub poll_id: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Session {
    pub id: String,
    pub status: i64,
    pub phase: String,
    pub queue_position: i64,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Stats {
    pub status: String,
    pub message: String,
    pub fps: f64,
    pub bitrate_mbps: f64,
    pub backend: String,
    pub first_frame_ms: i64,
    pub input_ready: bool,
    pub queue_drops: i64,
}

/// What comes into the rules.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// A remote key (evdev code), pressed or released.
    Key { code: u32, pressed: bool },
    /// The pad's side channel while the overlay is up: a direction, OK or Guide, decoded by the loop.
    Pad(PadKey),
    /// A response or event from the core.
    Core(crate::gfn::core::CoreMsg),
    /// The monotonic clock, for the overlay's inactivity and the sign-in's poll timer.
    Tick { now_ms: u64 },
    /// The compositor's word (minimized, fullscreen) and SAM's close.
    State(u32),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadKey {
    Up,
    Down,
    Left,
    Right,
    Ok,
    Back,
    Guide,
    Exit,
}

/// What the rules want done.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Request { method: String, params: Value },
    Ack { id: String },
    /// The engine's input is paused (the overlay has the pad) or resumed.
    PauseInput(bool),
    Exit(String),
}

pub const OVERLAY_HIDE_MS: u64 = 4000;
pub const POLL_SESSION_MS: u64 = 2000;
pub const LIBRARY_COLUMNS: usize = 6;

pub struct App {
    pub screen: Screen,
    pub previous: Screen,
    pub signed_in: bool,
    pub user_name: String,
    pub tier: String,
    pub scope: Value,
    pub sign_in: SignIn,
    pub games: Vec<Game>,
    pub next_cursor: String,
    pub catalog_revision: Value,
    pub catalog_context: Value,
    pub focus: usize,
    pub detail: Option<Game>,
    pub decision: String,
    pub decision_message: String,
    pub session: Option<Session>,
    pub stats: Stats,
    pub overlay_row: usize,
    pub overlay_since_ms: u64,
    pub chrome_until_ms: u64,
    pub show_stats: bool,
    pub error: String,
    pub runtime_capabilities: Value,
    pub region: Option<(String, String)>,
    pub now_ms: u64,
    next_poll_ms: u64,
    pending: Vec<(String, String)>,
    create_id: String,
    hello_done: bool,
    minimized: bool,
}

fn key_of(code: u32) -> Option<Key> {
    crate::card::key_of(code)
}

impl App {
    pub fn new() -> Self {
        Self {
            screen: Screen::Starting,
            previous: Screen::Starting,
            signed_in: false,
            user_name: String::new(),
            tier: String::new(),
            scope: Value::Null,
            sign_in: SignIn::default(),
            games: Vec::new(),
            next_cursor: String::new(),
            catalog_revision: Value::Null,
            catalog_context: Value::Null,
            focus: 0,
            detail: None,
            decision: String::new(),
            decision_message: String::new(),
            session: None,
            stats: Stats::default(),
            overlay_row: 0,
            overlay_since_ms: 0,
            chrome_until_ms: 0,
            show_stats: false,
            error: String::new(),
            runtime_capabilities: Value::Null,
            region: None,
            now_ms: 0,
            next_poll_ms: 0,
            pending: Vec::new(),
            create_id: String::new(),
            hello_done: false,
            minimized: false,
        }
    }

    /// The first requests, once the core is up: the handshake, then the engine's report and the account.
    pub fn start(&mut self) -> Vec<Action> {
        vec![Self::req("core.hello", json!({"protocolVersion": crate::gfn::core::PROTOCOL_VERSION, "shell": "lumatv", "shellVersion": env!("CARGO_PKG_VERSION")}))]
    }

    fn req(method: &str, params: Value) -> Action {
        Action::Request { method: method.to_owned(), params }
    }

    /// The stream is up and the window should let the picture through: the UI draws nothing but its chrome.
    pub fn streaming(&self) -> bool {
        matches!(self.screen, Screen::Stream | Screen::Overlay)
    }

    /// Whether chrome is on the screen over the stream (the overlay, or the status line after a key).
    pub fn chrome_visible(&self) -> bool {
        self.screen == Screen::Overlay || (self.screen == Screen::Stream && self.now_ms < self.chrome_until_ms)
    }

    pub fn wants_frames(&self) -> bool {
        !self.minimized
    }

    /// When the loop should wake the rules next, if nothing else happens.
    pub fn next_wake(&self) -> Option<Duration> {
        let mut at: Option<u64> = None;
        let mut consider = |t: u64| {
            if t > 0 {
                at = Some(at.map_or(t, |a| a.min(t)));
            }
        };
        if self.screen == Screen::Launching || (self.screen == Screen::SignIn && self.sign_in.poll_id.is_empty() && !self.sign_in.attempt_id.is_empty()) {
            consider(self.next_poll_ms);
        }
        if self.screen == Screen::Stream && self.now_ms < self.chrome_until_ms {
            consider(self.chrome_until_ms);
        }
        at.map(|t| Duration::from_millis(t.saturating_sub(self.now_ms)))
    }

    pub fn handle(&mut self, input: Input) -> Vec<Action> {
        match input {
            Input::Tick { now_ms } => {
                self.now_ms = now_ms;
                self.tick()
            }
            Input::Key { code, pressed } => {
                if !pressed {
                    return Vec::new();
                }
                match key_of(code) {
                    Some(k) => self.key(k),
                    None => Vec::new(),
                }
            }
            Input::Pad(p) => self.pad(p),
            Input::Core(m) => self.core(m),
            Input::State(s) => {
                self.minimized = s == crate::card::MINIMIZED;
                Vec::new()
            }
            Input::Close => self.leave("closed"),
        }
    }

    fn tick(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        if self.screen == Screen::Launching && self.session.is_some() && self.now_ms >= self.next_poll_ms && self.next_poll_ms > 0 && !self.has_pending("session.poll") {
            self.next_poll_ms = self.now_ms + POLL_SESSION_MS;
            let id = self.session.as_ref().map(|s| s.id.clone()).unwrap_or_default();
            out.push(Self::req("session.poll", json!({"sessionId": id})));
        }
        if self.screen == Screen::SignIn && !self.sign_in.attempt_id.is_empty() && self.sign_in.poll_id.is_empty() && self.now_ms >= self.next_poll_ms && self.next_poll_ms > 0 {
            self.sign_in.poll_id = "pending".to_owned();
            out.push(Self::req("auth.device.poll", json!({"attemptId": self.sign_in.attempt_id})));
        }
        out
    }

    fn has_pending(&self, method: &str) -> bool {
        self.pending.iter().any(|(_, m)| m == method)
    }

    fn key(&mut self, k: Key) -> Vec<Action> {
        match self.screen {
            Screen::Starting => Vec::new(),
            Screen::SignIn => match k {
                Key::Back => self.leave("back on sign-in"),
                _ => Vec::new(),
            },
            Screen::Library => self.library_key(k),
            Screen::Detail => match k {
                Key::Back => {
                    self.screen = Screen::Library;
                    Vec::new()
                }
                Key::Ok => self.launch(),
                _ => Vec::new(),
            },
            Screen::Launching => match k {
                Key::Back => self.stop_session(),
                _ => Vec::new(),
            },
            Screen::Stream => match k {
                Key::Back => self.open_overlay(),
                _ => {
                    self.chrome_until_ms = self.now_ms + OVERLAY_HIDE_MS;
                    Vec::new()
                }
            },
            Screen::Overlay => self.overlay_key(k),
            Screen::Error => match k {
                Key::Back | Key::Ok => {
                    self.error.clear();
                    self.screen = if self.signed_in { Screen::Library } else { Screen::SignIn };
                    Vec::new()
                }
                _ => Vec::new(),
            },
        }
    }

    fn pad(&mut self, p: PadKey) -> Vec<Action> {
        match p {
            PadKey::Guide => {
                if self.screen == Screen::Stream {
                    self.open_overlay()
                } else if self.screen == Screen::Overlay {
                    self.close_overlay()
                } else {
                    Vec::new()
                }
            }
            PadKey::Exit => {
                if self.streaming() {
                    self.stop_session()
                } else {
                    Vec::new()
                }
            }
            PadKey::Up => self.key(Key::Up),
            PadKey::Down => self.key(Key::Down),
            PadKey::Left => self.key(Key::Left),
            PadKey::Right => self.key(Key::Right),
            PadKey::Ok => self.key(Key::Ok),
            PadKey::Back => self.key(Key::Back),
        }
    }

    fn library_key(&mut self, k: Key) -> Vec<Action> {
        let n = self.games.len();
        match k {
            Key::Back => self.leave("back on the library"),
            Key::Ok => {
                let Some(g) = self.games.get(self.focus).cloned() else { return Vec::new() };
                self.detail = Some(g.clone());
                self.decision.clear();
                self.decision_message.clear();
                self.screen = Screen::Detail;
                vec![Self::req("catalog.launch.inspect", json!({"appId": g.id, "variantId": g.variant_id}))]
            }
            Key::Left if self.focus > 0 => {
                self.focus -= 1;
                Vec::new()
            }
            Key::Right if self.focus + 1 < n => {
                self.focus += 1;
                Vec::new()
            }
            Key::Up if self.focus >= LIBRARY_COLUMNS => {
                self.focus -= LIBRARY_COLUMNS;
                Vec::new()
            }
            Key::Down if self.focus + LIBRARY_COLUMNS < n => {
                self.focus += LIBRARY_COLUMNS;
                self.maybe_fetch_more()
            }
            Key::Down if n > 0 && self.focus < n - 1 => {
                self.focus = n - 1;
                self.maybe_fetch_more()
            }
            _ => Vec::new(),
        }
    }

    fn maybe_fetch_more(&mut self) -> Vec<Action> {
        if !self.next_cursor.is_empty() && self.focus + 2 * LIBRARY_COLUMNS >= self.games.len() && !self.has_pending("catalog.library.list") {
            return vec![self.library_request()];
        }
        Vec::new()
    }

    fn library_request(&self) -> Action {
        let mut p = json!({"limit": 60, "cursor": self.next_cursor});
        if !self.next_cursor.is_empty() {
            p["catalogRevision"] = self.catalog_revision.clone();
            p["catalogContext"] = self.catalog_context.clone();
        }
        Self::req("catalog.library.list", p)
    }

    fn launch(&mut self) -> Vec<Action> {
        let Some(g) = self.detail.clone() else { return Vec::new() };
        if self.decision != "ready" {
            return Vec::new();
        }
        self.screen = Screen::Launching;
        self.session = None;
        self.stats = Stats::default();
        let mut params = json!({
            "appId": g.variant_id,
            "catalogAppId": g.id,
            "variantId": g.variant_id,
            "scope": self.scope,
            "selectionIdentity": format!("{}:{}", g.id, g.variant_id),
            "title": g.title,
            "store": g.store,
            "accountLinked": g.in_library,
            "appLaunchMode": "default",
            "runtimeCapabilities": self.runtime_capabilities,
            "maxEntitledFps": 120,
        });
        if let Some((zone, url)) = &self.region {
            params["zone"] = json!(zone);
            params["streamingBaseUrl"] = json!(url);
        }
        vec![Self::req("session.create", params)]
    }

    fn open_overlay(&mut self) -> Vec<Action> {
        self.previous = self.screen;
        self.screen = Screen::Overlay;
        self.overlay_row = 0;
        self.overlay_since_ms = self.now_ms;
        vec![Action::PauseInput(true), Self::req("streamer.input.pause", json!({"paused": true}))]
    }

    fn close_overlay(&mut self) -> Vec<Action> {
        self.screen = Screen::Stream;
        self.chrome_until_ms = self.now_ms + OVERLAY_HIDE_MS;
        vec![Action::PauseInput(false), Self::req("streamer.input.pause", json!({"paused": false}))]
    }

    fn overlay_key(&mut self, k: Key) -> Vec<Action> {
        match k {
            Key::Back => self.close_overlay(),
            Key::Up if self.overlay_row > 0 => {
                self.overlay_row -= 1;
                Vec::new()
            }
            Key::Down if self.overlay_row + 1 < OVERLAY_ROWS.len() => {
                self.overlay_row += 1;
                Vec::new()
            }
            Key::Ok => match self.overlay_row {
                0 => self.close_overlay(),
                1 => {
                    self.show_stats = !self.show_stats;
                    self.close_overlay()
                }
                _ => self.stop_session(),
            },
            _ => Vec::new(),
        }
    }

    fn stop_session(&mut self) -> Vec<Action> {
        let mut out = vec![Action::PauseInput(false), Self::req("streamer.stop", json!({"reason": "the viewer left the stream"}))];
        if let Some(s) = &self.session {
            out.push(Self::req("session.stop", json!({"sessionId": s.id, "streamingBaseUrl": s.value["streamingBaseUrl"], "serverIp": s.value["serverIp"]})));
        }
        self.session = None;
        self.screen = if self.detail.is_some() { Screen::Detail } else { Screen::Library };
        out
    }

    fn leave(&mut self, why: &str) -> Vec<Action> {
        let mut out = Vec::new();
        if self.session.is_some() {
            out.extend(self.stop_session());
        }
        out.push(Action::Exit(why.to_owned()));
        out
    }

    fn fail(&mut self, message: String) -> Vec<Action> {
        self.error = message;
        self.screen = Screen::Error;
        Vec::new()
    }

    /// Notes a request the loop sent, so answers can be matched against what was asked.
    pub fn sent(&mut self, id: &str, method: &str) {
        self.pending.push((id.to_owned(), method.to_owned()));
        if method == "session.create" {
            self.create_id = id.to_owned();
        }
    }

    fn core(&mut self, m: crate::gfn::core::CoreMsg) -> Vec<Action> {
        use crate::gfn::core::CoreMsg;
        match m {
            CoreMsg::Gone => self.fail("The GeForce NOW core stopped. Open the app again.".to_owned()),
            CoreMsg::Event { name, payload } => self.event(&name, payload),
            CoreMsg::Response { id, method, ok, result, error_code, error_message } => {
                self.pending.retain(|(i, _)| *i != id);
                if !ok {
                    return self.error_response(&method, &error_code, &error_message);
                }
                self.response(&id, &method, result)
            }
        }
    }

    fn error_response(&mut self, method: &str, code: &str, message: &str) -> Vec<Action> {
        match method {
            "auth.device.poll" => {
                self.sign_in.poll_id.clear();
                self.next_poll_ms = self.now_ms + self.sign_in.interval_s.max(1) * 1000;
                Vec::new()
            }
            "auth.session.get" => {
                self.signed_in = false;
                self.screen = Screen::SignIn;
                vec![Self::req("auth.device.start", json!({}))]
            }
            "session.poll" if code == "busy" || code == "session_update_busy" => {
                self.next_poll_ms = self.now_ms + POLL_SESSION_MS;
                Vec::new()
            }
            "streamer.stop" | "session.stop" | "streamer.input.pause" | "catalog.library.list" if self.screen == Screen::Stream || self.screen == Screen::Overlay => Vec::new(),
            _ => {
                if self.session.is_some() {
                    let mut out = self.stop_session();
                    out.extend(self.fail(format!("{message} ({code})")));
                    return out;
                }
                self.fail(if message.is_empty() { format!("{method} failed ({code})") } else { format!("{message} ({code})") })
            }
        }
    }

    fn response(&mut self, id: &str, method: &str, r: Value) -> Vec<Action> {
        match method {
            "core.hello" => {
                self.hello_done = true;
                vec![Self::req("streamer.detect", json!({})), Self::req("auth.session.get", json!({}))]
            }
            "streamer.detect" => {
                let mut caps = r["capabilities"].clone();
                caps["nativeHdrSupported"] = json!(true);
                caps["nativeHdrDisplay"] = json!({"minimumNits": 0, "maximumNits": 1000, "maximumFullFrameNits": 400});
                self.runtime_capabilities = caps;
                vec![Self::req("settings.set", json!({"resolution": "3840x2160", "fps": 120, "codec": "auto", "colorQuality": "10bit_420", "enableHdr": true, "maxBitrateMbps": 80, "nativeVideoBackend": "ndl"}))]
            }
            "settings.set" => vec![Self::req("network.regions.list", json!({}))],
            "network.regions.list" => {
                if let Some(first) = r["regions"].as_array().and_then(|a| a.first()) {
                    if let (Some(n), Some(u)) = (first["name"].as_str(), first["url"].as_str()) {
                        self.region = Some((n.to_owned(), u.to_owned()));
                    }
                }
                Vec::new()
            }
            "auth.session.get" | "auth.device.complete" => {
                self.scope = r["scope"].clone();
                if r["session"].is_object() {
                    self.signed_in = true;
                    self.user_name = r["session"]["user"]["displayName"].as_str().unwrap_or("").to_owned();
                    self.tier = r["session"]["user"]["membershipTier"].as_str().unwrap_or("").to_owned();
                    self.screen = Screen::Library;
                    self.games.clear();
                    self.next_cursor.clear();
                    self.focus = 0;
                    vec![self.library_request()]
                } else {
                    self.signed_in = false;
                    self.screen = Screen::SignIn;
                    vec![Self::req("auth.device.start", json!({}))]
                }
            }
            "auth.device.start" => {
                self.sign_in = SignIn {
                    attempt_id: r["attemptId"].as_str().unwrap_or("").to_owned(),
                    user_code: r["userCode"].as_str().unwrap_or("").to_owned(),
                    url: r["verificationUri"].as_str().unwrap_or("").to_owned(),
                    qr_rows: r["qrRows"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
                    expires_at_ms: r["expiresAt"].as_u64().unwrap_or(0),
                    interval_s: r["intervalSeconds"].as_u64().unwrap_or(5),
                    poll_id: String::new(),
                    status: "pending".to_owned(),
                };
                self.next_poll_ms = self.now_ms + self.sign_in.interval_s * 1000;
                Vec::new()
            }
            "auth.device.poll" => {
                self.sign_in.poll_id.clear();
                let status = r["status"].as_str().unwrap_or("").to_owned();
                self.sign_in.status = status.clone();
                match status.as_str() {
                    "authorized" => vec![Self::req("auth.device.complete", json!({"attemptId": self.sign_in.attempt_id}))],
                    "expired" => {
                        self.sign_in.attempt_id.clear();
                        vec![Self::req("auth.device.start", json!({}))]
                    }
                    _ => {
                        let retry = r["retryAfterMs"].as_u64().unwrap_or(self.sign_in.interval_s * 1000);
                        self.next_poll_ms = self.now_ms + retry.max(1000);
                        Vec::new()
                    }
                }
            }
            "catalog.library.list" => {
                for g in r["games"].as_array().into_iter().flatten() {
                    self.games.push(game_of(g));
                }
                self.next_cursor = r["nextCursor"].as_str().unwrap_or("").to_owned();
                self.catalog_revision = r["catalogRevision"].clone();
                self.catalog_context = r["catalogContext"].clone();
                Vec::new()
            }
            "catalog.launch.inspect" => {
                self.decision = r["decision"]["status"].as_str().unwrap_or("metadata_unconfirmed").to_owned();
                self.decision_message = r["decision"]["message"].as_str().unwrap_or("").to_owned();
                if let Some(d) = self.detail.as_mut() {
                    if r["game"].is_object() {
                        let g = game_of(&r["game"]);
                        if g.id == d.id {
                            d.title = g.title;
                            d.store = g.store;
                            d.in_library = g.in_library;
                            d.image_url = if g.image_url.is_empty() { d.image_url.clone() } else { g.image_url };
                        }
                    }
                }
                Vec::new()
            }
            "session.create" => {
                let mut out = vec![Action::Ack { id: id.to_owned() }];
                out.extend(self.session_update(r));
                out
            }
            "session.poll" | "session.claim" => self.session_update(r),
            "streamer.start" => {
                self.screen = Screen::Stream;
                self.chrome_until_ms = self.now_ms + OVERLAY_HIDE_MS;
                self.stats_from(&r);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn session_update(&mut self, r: Value) -> Vec<Action> {
        let value = if r["session"].is_object() { r["session"].clone() } else { r.clone() };
        if value["sessionId"].as_str().is_none() {
            return Vec::new();
        }
        let s = Session {
            id: value["sessionId"].as_str().unwrap_or("").to_owned(),
            status: value["status"].as_i64().unwrap_or(0),
            phase: value["phase"].as_str().unwrap_or("").to_owned(),
            queue_position: value["queuePosition"].as_i64().unwrap_or(0),
            value: value.clone(),
        };
        let ready = matches!(s.status, 2 | 3) && value["rtspsEndpoints"].as_array().is_some_and(|e| !e.is_empty()) && value["resumePending"] != true;
        let finished = s.status == 7 || s.phase == "finished" || s.phase == "failed";
        self.session = Some(s.clone());
        if finished {
            let why = value["termination"]["reason"].as_str().unwrap_or("The session ended.").to_owned();
            let mut out = Vec::new();
            if self.streaming() {
                out.push(Action::PauseInput(false));
                out.push(Self::req("streamer.stop", json!({"reason": "the session ended"})));
            }
            self.session = None;
            out.extend(self.fail(why));
            return out;
        }
        if self.screen == Screen::Launching {
            if ready && !self.has_pending("streamer.start") {
                self.next_poll_ms = 0;
                return vec![Self::req("streamer.start", json!({"session": value, "surface": {"visible": true, "rect": {"x": 0, "y": 0, "width": 1920, "height": 1080}}}))];
            }
            self.next_poll_ms = self.now_ms + POLL_SESSION_MS;
        }
        Vec::new()
    }

    fn stats_from(&mut self, r: &Value) {
        self.stats = Stats {
            status: r["status"].as_str().unwrap_or("").to_owned(),
            message: r["message"].as_str().unwrap_or("").to_owned(),
            fps: r["framesPerSecond"].as_f64().unwrap_or(0.0),
            bitrate_mbps: r["bitrateMbps"].as_f64().unwrap_or(0.0),
            backend: r["mediaBackend"].as_str().unwrap_or("").to_owned(),
            first_frame_ms: r["firstFrameLatencyMs"].as_i64().unwrap_or(-1),
            input_ready: r["inputReady"].as_bool().unwrap_or(false),
            queue_drops: r["queueDropCount"].as_i64().unwrap_or(0),
        };
    }

    fn event(&mut self, name: &str, payload: Value) -> Vec<Action> {
        match name {
            "streamer.changed" => {
                self.stats_from(&payload);
                if self.streaming() && matches!(self.stats.status.as_str(), "error" | "stopped" | "exited") {
                    let message = if self.stats.message.is_empty() { "The stream ended.".to_owned() } else { self.stats.message.clone() };
                    let mut out = vec![Action::PauseInput(false)];
                    if let Some(s) = &self.session {
                        out.push(Self::req("session.stop", json!({"sessionId": s.id, "streamingBaseUrl": s.value["streamingBaseUrl"], "serverIp": s.value["serverIp"]})));
                    }
                    self.session = None;
                    out.extend(self.fail(message));
                    return out;
                }
                Vec::new()
            }
            "session.changed" => {
                if self.screen == Screen::Launching || self.streaming() {
                    return self.session_update(payload);
                }
                Vec::new()
            }
            "auth.session.changed" => {
                if payload["session"].is_null() && self.signed_in {
                    self.signed_in = false;
                    let mut out = Vec::new();
                    if self.session.is_some() {
                        out.extend(self.stop_session());
                    }
                    self.screen = Screen::SignIn;
                    out.push(Self::req("auth.device.start", json!({})));
                    return out;
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

fn game_of(g: &Value) -> Game {
    let variants = g["variants"].as_array().cloned().unwrap_or_default();
    let selected = variants.iter().find(|v| v["inLibrary"] == true).or_else(|| variants.first()).cloned().unwrap_or(Value::Null);
    Game {
        id: g["id"].as_str().map(str::to_owned).unwrap_or_else(|| g["id"].to_string()),
        variant_id: selected["id"].as_str().map(str::to_owned).unwrap_or_else(|| if selected["id"].is_null() { String::new() } else { selected["id"].to_string() }),
        title: g["title"].as_str().unwrap_or("").to_owned(),
        image_url: g["imageUrl"].as_str().or_else(|| g["keyArtUrl"].as_str()).unwrap_or("").to_owned(),
        store: selected["store"].as_str().unwrap_or("").to_owned(),
        in_library: selected["inLibrary"] == true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfn::core::CoreMsg;

    fn ok(id: &str, method: &str, result: Value) -> Input {
        Input::Core(CoreMsg::Response { id: id.into(), method: method.into(), ok: true, result, error_code: String::new(), error_message: String::new() })
    }

    fn err(id: &str, method: &str, code: &str, message: &str) -> Input {
        Input::Core(CoreMsg::Response { id: id.into(), method: method.into(), ok: false, result: Value::Null, error_code: code.into(), error_message: message.into() })
    }

    fn methods(actions: &[Action]) -> Vec<String> {
        actions.iter().filter_map(|a| if let Action::Request { method, .. } = a { Some(method.clone()) } else { None }).collect()
    }

    fn key(code: u32) -> Input {
        Input::Key { code, pressed: true }
    }

    fn games(n: usize) -> Value {
        json!((0..n).map(|i| json!({"id": format!("g{i}"), "title": format!("Game {i}"), "imageUrl": format!("https://x/{i}.jpg"),
            "variants": [{"id": format!("v{i}"), "store": "STEAM", "inLibrary": true}]})).collect::<Vec<_>>())
    }

    /// The handshake through the library, with the sign-in in between: the requests in OpenNOW's order.
    fn signed_in_app() -> App {
        let mut app = App::new();
        assert_eq!(methods(&app.start()), ["core.hello"]);
        app.sent("1", "core.hello");
        let a = app.handle(ok("1", "core.hello", json!({"capabilities": ["nativeStreamer.v7"]})));
        assert_eq!(methods(&a), ["streamer.detect", "auth.session.get"]);
        app.sent("2", "streamer.detect");
        app.sent("3", "auth.session.get");
        let a = app.handle(ok("2", "streamer.detect", json!({"available": true, "capabilities": {"protocolVersion": 7, "videoBackends": [{"backend": "ndl"}]}})));
        assert_eq!(methods(&a), ["settings.set"]);
        assert_eq!(app.runtime_capabilities["nativeHdrSupported"], true);
        assert_eq!(app.runtime_capabilities["nativeHdrDisplay"]["maximumNits"], 1000);
        if let Action::Request { params, .. } = &a[0] {
            assert_eq!(params["resolution"], "3840x2160");
            assert_eq!(params["fps"], 120);
            assert_eq!(params["enableHdr"], true);
        }
        let a = app.handle(ok("3", "auth.session.get", json!({"session": null, "scope": {"generation": 1}})));
        assert_eq!(app.screen, Screen::SignIn);
        assert_eq!(methods(&a), ["auth.device.start"]);
        app.sent("4", "auth.device.start");
        app.handle(ok("4", "auth.device.start", json!({"attemptId": "att", "userCode": "ABCD-EFGH", "verificationUri": "https://nvidia.com/activate", "qrRows": ["101", "010", "101"], "expiresAt": 999999, "intervalSeconds": 5})));
        assert_eq!(app.sign_in.user_code, "ABCD-EFGH");
        assert_eq!(app.sign_in.qr_rows.len(), 3);
        assert!(app.handle(Input::Tick { now_ms: 1000 }).is_empty());
        let a = app.handle(Input::Tick { now_ms: 5001 });
        assert_eq!(methods(&a), ["auth.device.poll"]);
        app.sent("5", "auth.device.poll");
        let a = app.handle(ok("5", "auth.device.poll", json!({"status": "pending", "retryAfterMs": 3000})));
        assert!(a.is_empty());
        assert!(app.handle(Input::Tick { now_ms: 7000 }).is_empty());
        let a = app.handle(Input::Tick { now_ms: 8100 });
        assert_eq!(methods(&a), ["auth.device.poll"]);
        app.sent("6", "auth.device.poll");
        let a = app.handle(ok("6", "auth.device.poll", json!({"status": "authorized"})));
        assert_eq!(methods(&a), ["auth.device.complete"]);
        app.sent("7", "auth.device.complete");
        let a = app.handle(ok("7", "auth.device.complete", json!({"session": {"user": {"displayName": "Nitin", "membershipTier": "ULTIMATE"}}, "scope": {"generation": 2, "userId": "u"}})));
        assert_eq!(app.screen, Screen::Library);
        assert_eq!((app.user_name.as_str(), app.tier.as_str()), ("Nitin", "ULTIMATE"));
        assert_eq!(methods(&a), ["catalog.library.list"]);
        app.sent("8", "catalog.library.list");
        app.handle(ok("8", "catalog.library.list", json!({"games": games(14), "nextCursor": "c2", "hasNextPage": true, "catalogRevision": 3, "catalogContext": "ctx"})));
        assert_eq!(app.games.len(), 14);
        assert_eq!(app.games[0].variant_id, "v0");
        app
    }

    #[test]
    fn handshake_sign_in_and_library() {
        let app = signed_in_app();
        assert_eq!(app.next_cursor, "c2");
    }

    #[test]
    fn the_library_moves_in_a_grid_and_fetches_the_next_page_near_the_end() {
        let mut app = signed_in_app();
        app.handle(key(106));
        app.handle(key(106));
        assert_eq!(app.focus, 2);
        app.handle(key(108));
        assert_eq!(app.focus, 8);
        app.handle(key(103));
        assert_eq!(app.focus, 2);
        app.handle(key(105));
        app.handle(key(105));
        app.handle(key(105));
        assert_eq!(app.focus, 0);
        let a = app.handle(key(108));
        assert_eq!(app.focus, 6);
        // a page of 14 with a cursor: moving into the last two rows asks for more, with the revision and context the core gave
        assert_eq!(methods(&a), ["catalog.library.list"]);
        if let Action::Request { params, .. } = &a[0] {
            assert_eq!(params["cursor"], "c2");
            assert_eq!(params["catalogRevision"], 3);
            assert_eq!(params["catalogContext"], "ctx");
        }
        app.sent("9", "catalog.library.list");
        assert!(app.handle(key(108)).is_empty(), "no second request while one is pending");
        app.handle(ok("9", "catalog.library.list", json!({"games": games(3), "nextCursor": "", "catalogRevision": 3, "catalogContext": "ctx"})));
        assert_eq!(app.games.len(), 17);
    }

    #[test]
    fn launch_runs_inspect_create_ack_poll_start_and_the_overlay_pauses_input() {
        let mut app = signed_in_app();
        app.handle(key(106));
        let a = app.handle(key(28));
        assert_eq!(app.screen, Screen::Detail);
        assert_eq!(methods(&a), ["catalog.launch.inspect"]);
        if let Action::Request { params, .. } = &a[0] {
            assert_eq!(params["appId"], "g1");
            assert_eq!(params["variantId"], "v1");
        }
        assert!(app.handle(key(28)).is_empty(), "no launch before the decision");
        app.sent("10", "catalog.launch.inspect");
        app.handle(ok("10", "catalog.launch.inspect", json!({"decision": {"status": "ready", "message": ""}, "game": {"id": "g1", "title": "Game One", "variants": [{"id": "v1", "store": "STEAM", "inLibrary": true}]}})));
        assert_eq!(app.decision, "ready");
        assert_eq!(app.detail.as_ref().unwrap().title, "Game One");
        let a = app.handle(key(28));
        assert_eq!(app.screen, Screen::Launching);
        assert_eq!(methods(&a), ["session.create"]);
        if let Action::Request { params, .. } = &a[0] {
            assert_eq!(params["appId"], "v1");
            assert_eq!(params["catalogAppId"], "g1");
            assert_eq!(params["scope"]["userId"], "u");
            assert_eq!(params["runtimeCapabilities"]["nativeHdrSupported"], true);
            assert_eq!(params["maxEntitledFps"], 120);
        }
        app.sent("11", "session.create");
        let a = app.handle(ok("11", "session.create", json!({"session": {"sessionId": "s1", "status": 1, "phase": "queued", "queuePosition": 4, "rtspsEndpoints": []}})));
        assert_eq!(a[0], Action::Ack { id: "11".into() });
        assert_eq!(app.session.as_ref().unwrap().queue_position, 4);
        assert!(app.handle(Input::Tick { now_ms: 8500 }).is_empty());
        let a = app.handle(Input::Tick { now_ms: 10200 });
        assert_eq!(methods(&a), ["session.poll"]);
        app.sent("12", "session.poll");
        let ready = json!({"sessionId": "s1", "status": 2, "phase": "ready", "rtspsEndpoints": [{"uri": "rtsps://x"}], "streamingBaseUrl": "https://b", "serverIp": "1.2.3.4"});
        let a = app.handle(ok("12", "session.poll", json!({"session": ready})));
        assert_eq!(methods(&a), ["streamer.start"]);
        if let Action::Request { params, .. } = &a[0] {
            assert_eq!(params["session"]["sessionId"], "s1");
            assert_eq!(params["surface"]["visible"], true);
        }
        app.sent("13", "streamer.start");
        app.handle(ok("13", "streamer.start", json!({"status": "starting", "message": "Launching"})));
        assert_eq!(app.screen, Screen::Stream);
        assert!(app.chrome_visible());
        app.handle(Input::Tick { now_ms: 10200 + OVERLAY_HIDE_MS + 1 });
        assert!(!app.chrome_visible());
        app.handle(Input::Core(CoreMsg::Event { name: "streamer.changed".into(), payload: json!({"status": "streaming", "framesPerSecond": 119.8, "bitrateMbps": 78.2, "mediaBackend": "ndl", "inputReady": true}) }));
        assert_eq!(app.stats.backend, "ndl");
        assert!(app.stats.input_ready);
        // Guide opens the overlay, which pauses the game's input; Down then OK on "Quit game" stops both halves
        let a = app.handle(Input::Pad(PadKey::Guide));
        assert_eq!(app.screen, Screen::Overlay);
        assert_eq!(a[0], Action::PauseInput(true));
        assert_eq!(methods(&a), ["streamer.input.pause"]);
        app.handle(key(108));
        app.handle(key(108));
        assert_eq!(app.overlay_row, 2);
        let a = app.handle(key(28));
        assert_eq!(a[0], Action::PauseInput(false));
        assert_eq!(methods(&a), ["streamer.stop", "session.stop"]);
        if let Action::Request { params, .. } = &a[2] {
            assert_eq!(params["sessionId"], "s1");
            assert_eq!(params["streamingBaseUrl"], "https://b");
        }
        assert_eq!(app.screen, Screen::Detail);
        assert!(app.session.is_none());
    }

    #[test]
    fn the_overlay_closes_on_back_and_a_key_shows_chrome_briefly() {
        let mut app = signed_in_app();
        app.screen = Screen::Stream;
        app.handle(Input::Tick { now_ms: 50_000 });
        let a = app.handle(key(412));
        assert_eq!(app.screen, Screen::Overlay);
        assert_eq!(a[0], Action::PauseInput(true));
        let a = app.handle(key(412));
        assert_eq!(app.screen, Screen::Stream);
        assert_eq!(a[0], Action::PauseInput(false));
        assert!(app.chrome_visible());
        assert_eq!(app.next_wake(), Some(Duration::from_millis(OVERLAY_HIDE_MS)));
        app.handle(Input::Tick { now_ms: 50_000 + OVERLAY_HIDE_MS });
        assert!(!app.chrome_visible());
        app.handle(key(103));
        assert!(app.chrome_visible());
        let a = app.handle(Input::Pad(PadKey::Exit));
        assert_eq!(methods(&a), ["streamer.stop"]);
        assert_eq!(app.screen, Screen::Library);
    }

    #[test]
    fn a_session_that_ends_goes_to_the_error_screen_and_back_returns() {
        let mut app = signed_in_app();
        app.screen = Screen::Stream;
        app.session = Some(Session { id: "s1".into(), status: 2, phase: "ready".into(), queue_position: 0, value: json!({"sessionId": "s1"}) });
        let a = app.handle(Input::Core(CoreMsg::Event { name: "session.changed".into(), payload: json!({"sessionId": "s1", "status": 7, "phase": "finished", "termination": {"source": "cloudmatch-session-status", "status": 7}}) }));
        assert_eq!(app.screen, Screen::Error);
        assert_eq!(a[0], Action::PauseInput(false));
        assert_eq!(methods(&a), ["streamer.stop"]);
        app.handle(key(412));
        assert_eq!(app.screen, Screen::Library);
        let a = app.handle(err("20", "catalog.launch.inspect", "upstream", "GFN is down"));
        assert_eq!(app.screen, Screen::Error);
        assert_eq!(app.error, "GFN is down (upstream)");
        assert!(a.is_empty());
        let a = app.handle(Input::Core(CoreMsg::Gone));
        assert!(a.is_empty());
        assert!(app.error.contains("core stopped"));
    }

    #[test]
    fn back_leaves_from_the_library_and_the_sign_in_and_stops_a_session_first() {
        let mut app = signed_in_app();
        assert_eq!(app.handle(key(412)), vec![Action::Exit("back on the library".into())]);
        let mut app = App::new();
        app.screen = Screen::SignIn;
        assert_eq!(app.handle(key(412)), vec![Action::Exit("back on sign-in".into())]);
        let mut app = signed_in_app();
        app.screen = Screen::Launching;
        app.session = Some(Session { id: "s9".into(), status: 1, phase: "queued".into(), queue_position: 1, value: json!({"sessionId": "s9"}) });
        let a = app.handle(Input::Close);
        assert_eq!(methods(&a), ["streamer.stop", "session.stop"]);
        assert_eq!(a.last(), Some(&Action::Exit("closed".into())));
    }

    #[test]
    fn signing_out_returns_to_sign_in_and_a_minimized_card_draws_nothing() {
        let mut app = signed_in_app();
        let a = app.handle(Input::Core(CoreMsg::Event { name: "auth.session.changed".into(), payload: json!({"session": null}) }));
        assert_eq!(app.screen, Screen::SignIn);
        assert_eq!(methods(&a), ["auth.device.start"]);
        app.handle(Input::State(crate::card::MINIMIZED));
        assert!(!app.wants_frames());
        app.handle(Input::State(crate::card::FULLSCREEN));
        assert!(app.wants_frames());
    }
}
