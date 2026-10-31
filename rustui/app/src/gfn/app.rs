//! The GeForce NOW app's rules, with no window, no GPU and no core process in them: what the remote's keys and the pad do on each
//! screen, which request goes to the core when, and what each answer changes. `gfn_ui.rs` draws the screens, `bin/gfn.rs` runs the
//! loop; the tests here drive the rules with the core's answers written by hand, in the shapes OpenNOW's core documents
//! (`docs/core-protocol.md`) and sends (`native/opennow-core/src/{gfn.rs,gfn/catalog.rs,cloudmatch.rs,streamer.rs}`).
//!
//! The launch sequence is the one OpenNOW's own shell runs (`opennow-qt/qml/state/ShellStore.qml`): `catalog.launch.inspect`, then
//! `session.create` (acknowledged with an `ack` as the protocol demands), `session.poll` until the seat is ready (status 2 or 3 with
//! endpoints), `streamer.start {session}`. Stopping is `streamer.stop` and `session.stop`. HDR is only resolved against the runtime
//! capabilities the shell passes, so `session.create` carries the engine's report plus `nativeHdrSupported` and the display's nits.
use crate::gfn::prefs::{Prefs, Row};
use crate::home::Key;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

/// The screens, in the order Back walks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// The core is starting, or an answer the next screen needs is on its way.
    Starting,
    SignIn,
    Library,
    /// The stream's tier, HDR, the bitrate cap, the stick dead zone and the stats line, from the library's header.
    Settings,
    /// The question before signing out: keep the account, or forget it on this TV.
    SignOut,
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

/// What the core has said about one picture of the library, by its source address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Art {
    /// Asked for; the core is fetching it (or has not answered yet).
    Pending,
    /// In the core's disk cache at this path.
    File(String),
    /// The core could not give it (no network, a refusal) at this time on the app's clock: the tile keeps its placeholder, and the
    /// picture is asked for again after [`ART_AGAIN_MS`] if its tile is on the screen.
    Missing(u64),
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

/// The press latency the engine measures from the pad's kernel time stamp to the packet leaving for the transport (microseconds), as
/// its side channel reports it (gfn/engine `pads.rs`, `lumatv-gfn-pad` `latency`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadLatency {
    pub p50_us: u32,
    pub p99_us: u32,
}

/// What comes into the rules.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// A remote key (evdev code), pressed or released.
    Key { code: u32, pressed: bool },
    /// The pad's side channel while the overlay is up: a direction, OK or Guide, decoded by the loop.
    Pad(PadKey),
    /// The engine's press latency figures, from the pad's side channel.
    PadLatency(PadLatency),
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
    /// The settings screen changed a value: the app writes its preferences file.
    SavePrefs(Prefs),
    Exit(String),
}

pub const OVERLAY_HIDE_MS: u64 = 4000;
pub const POLL_SESSION_MS: u64 = 2000;
pub const LIBRARY_COLUMNS: usize = 6;
/// Rows of tiles on the screen at once; the grid scrolls by whole rows.
pub const GRID_ROWS: usize = 2;
/// OK held this long on a settings row opens the row's explanation (the help mark's gesture on LumaTV's own settings).
pub const HOLD_MS: u64 = 600;
/// Box art is asked for again this long after the core said it was busy.
pub const ART_RETRY_MS: u64 = 1500;
/// Picture requests waiting for an answer at once: the core serves its library and picture calls on four workers between them and
/// refuses the rest as busy, and a library page must never be the call that is refused.
pub const ART_INFLIGHT: usize = 3;
/// A picture the core could not give is asked for again this long after, when its tile is on the screen.
pub const ART_AGAIN_MS: u64 = 60_000;
/// The account's tier is read again when the library's header or the settings are opened, but not more often than this.
pub const ACCOUNT_REFRESH_MS: u64 = 10_000;
/// What a launch the core keeps refusing for the account's scope says (it was retried once with fresh account state).
pub const STALE_ACCOUNT: &str = "This game could not be started for the signed-in account. Open Settings, sign out and sign in again.";

/// The first row shown so that the focused tile is on the screen.
pub fn first_row_for(focus: usize, current: usize) -> usize {
    let row = focus / LIBRARY_COLUMNS;
    if row < current {
        row
    } else if row >= current + GRID_ROWS {
        row + 1 - GRID_ROWS
    } else {
        current
    }
}

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
    pub pad_latency: Option<PadLatency>,
    pub overlay_row: usize,
    pub overlay_since_ms: u64,
    pub chrome_until_ms: u64,
    pub prefs: Prefs,
    /// The settings row with the focus, and whether its explanation is open.
    pub settings_row: usize,
    pub help_open: bool,
    /// The library's focus is on the header's settings button, above the first row of tiles.
    pub header_focus: bool,
    /// The first row of tiles on the screen.
    pub first_row: usize,
    /// The box art of the library, by source address.
    pub art: HashMap<String, Art>,
    pub error: String,
    /// The sign-out question's focus: 0 keeps the account, 1 signs out.
    pub confirm_row: usize,
    pub runtime_capabilities: Value,
    pub region: Option<(String, String)>,
    pub now_ms: u64,
    next_poll_ms: u64,
    pending: Vec<(String, String)>,
    /// `streamer.start` was asked for this launch: a second session update before the loop records the request must not ask again
    /// (the core answers the second streamer_busy and the failure tore down the first one's session).
    start_asked: bool,
    create_id: String,
    hello_done: bool,
    minimized: bool,
    /// The regions are asked for once, when an account is known (the core refuses the question to a signed-out app).
    regions_asked: bool,
    /// The core takes one setting a request, and a few requests at a time (it answers `busy` past that): the settings still to send, one
    /// at a time. `boot_settings` is the handshake's batch; the streamer is asked for once it is through, because the core finds the
    /// engine by the path in its settings.
    settings_queue: VecDeque<(String, Value)>,
    boot_settings: bool,
    engine_path: String,
    ok_down_ms: Option<u64>,
    ok_held: bool,
    ok_swallow: bool,
    /// Request id to source address for the picture requests in flight, and the core's normalised spelling of an address to ours.
    art_ids: HashMap<String, String>,
    art_alias: HashMap<String, String>,
    art_retry_ms: u64,
    /// Picture requests asked and not yet answered.
    art_asked: usize,
    /// The tier on show is the subscription's (read live); until it is read it is the one in the sign-in token, which only changes when
    /// the token is renewed.
    tier_live: bool,
    /// When the account was last re-read, and when a re-read the core refused as busy is asked again.
    account_at_ms: Option<u64>,
    account_due_ms: u64,
    /// A launch refused for a stale account scope is retried once with fresh state: 1 while the session is re-read, 2 while the game is
    /// inspected again. `launch_retried` is set from the first refusal until the player starts another launch.
    retry_stage: u8,
    launch_retried: bool,
    /// Requests asked before the account was dropped (a sign-out): their answers belong to the old account and are discarded.
    orphans: std::collections::HashSet<String>,
}

fn key_of(code: u32) -> Option<Key> {
    crate::card::key_of(code)
}

impl App {
    pub fn new() -> Self {
        Self::with_prefs(Prefs::default())
    }

    /// Where the streamer engine is: the core is told in the handshake, before it looks for it.
    pub fn with_engine(mut self, path: &str) -> Self {
        self.engine_path = path.to_owned();
        self
    }

    pub fn with_prefs(prefs: Prefs) -> Self {
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
            pad_latency: None,
            overlay_row: 0,
            overlay_since_ms: 0,
            chrome_until_ms: 0,
            prefs,
            settings_row: 0,
            help_open: false,
            header_focus: false,
            first_row: 0,
            art: HashMap::new(),
            error: String::new(),
            runtime_capabilities: Value::Null,
            region: None,
            now_ms: 0,
            next_poll_ms: 0,
            pending: Vec::new(),
            start_asked: false,
            create_id: String::new(),
            hello_done: false,
            minimized: false,
            regions_asked: false,
            settings_queue: VecDeque::new(),
            boot_settings: false,
            engine_path: String::new(),
            ok_down_ms: None,
            ok_held: false,
            ok_swallow: false,
            art_ids: HashMap::new(),
            art_alias: HashMap::new(),
            art_retry_ms: 0,
            art_asked: 0,
            confirm_row: 0,
            tier_live: false,
            account_at_ms: None,
            account_due_ms: 0,
            retry_stage: 0,
            launch_retried: false,
            orphans: std::collections::HashSet::new(),
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

    /// The library has asked for a page and has not heard back: an empty grid is a wait, not an empty library.
    pub fn loading_library(&self) -> bool {
        self.has_pending("catalog.library.list")
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
        if let Some(t) = self.ok_down_ms {
            if self.screen == Screen::Settings && !self.ok_held && !self.help_open {
                consider(t + HOLD_MS);
            }
        }
        if self.screen == Screen::Library && self.art_retry_ms > 0 {
            consider(self.art_retry_ms);
        }
        if self.signed_in {
            consider(self.account_due_ms);
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
                let Some(k) = key_of(code) else { return Vec::new() };
                if self.screen == Screen::Settings && k == Key::Ok {
                    return self.settings_ok(pressed);
                }
                if !pressed {
                    return Vec::new();
                }
                self.key(k)
            }
            Input::Pad(p) => self.pad(p),
            Input::PadLatency(l) => {
                self.pad_latency = Some(l);
                Vec::new()
            }
            Input::Core(m) => self.core(m),
            Input::State(s) => {
                let was_minimized = self.minimized;
                self.minimized = s == crate::card::MINIMIZED;
                // the app was away: an upgrade made on a phone meanwhile shows when it comes back
                if was_minimized && !self.minimized && matches!(self.screen, Screen::Library | Screen::Settings | Screen::Detail) {
                    return self.refresh_account(true);
                }
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
        if let Some(t) = self.ok_down_ms {
            if self.screen == Screen::Settings && !self.ok_held && !self.help_open && self.now_ms >= t + HOLD_MS {
                self.ok_held = true;
                self.help_open = true;
            }
        }
        if self.art_retry_ms > 0 && self.now_ms >= self.art_retry_ms && self.screen == Screen::Library {
            self.art_retry_ms = 0;
            out.extend(self.art_wanted());
        }
        if self.account_due_ms > 0 && self.now_ms >= self.account_due_ms {
            self.account_due_ms = 0;
            if self.signed_in && !self.has_pending("account.subscription.get") {
                out.push(Self::req("account.subscription.get", json!({})));
            }
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
            Screen::Settings => self.settings_key(k),
            Screen::SignOut => self.sign_out_key(k),
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
        if self.header_focus {
            return match k {
                Key::Back => self.leave("back on the library"),
                Key::Ok => self.open_settings(),
                Key::Down if n > 0 => {
                    self.header_focus = false;
                    Vec::new()
                }
                _ => Vec::new(),
            };
        }
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
                self.moved()
            }
            Key::Right if self.focus + 1 < n => {
                self.focus += 1;
                self.moved()
            }
            Key::Up if self.focus >= LIBRARY_COLUMNS => {
                self.focus -= LIBRARY_COLUMNS;
                self.moved()
            }
            Key::Up => {
                self.header_focus = true;
                self.refresh_account(false)
            }
            Key::Down if self.focus + LIBRARY_COLUMNS < n => {
                self.focus += LIBRARY_COLUMNS;
                self.moved()
            }
            Key::Down if n > 0 && self.focus < n - 1 => {
                self.focus = n - 1;
                self.moved()
            }
            _ => Vec::new(),
        }
    }

    /// The focus moved: the grid scrolls to keep it on the screen, the pictures near it are asked for, and the next page is fetched
    /// when the end is near.
    fn moved(&mut self) -> Vec<Action> {
        self.first_row = first_row_for(self.focus, self.first_row);
        let mut out = self.maybe_fetch_more();
        out.extend(self.art_wanted());
        out
    }

    /// The pictures of the tiles on the screen first, then of the row below and the row above, once each: `artwork.resolve` answers
    /// with a file in the core's disk cache, or with "pending" and an `artwork.ready` event when the download is done. At most
    /// [`ART_INFLIGHT`] requests wait at a time; each answer asks for the next.
    fn art_wanted(&mut self) -> Vec<Action> {
        if self.now_ms < self.art_retry_ms || !matches!(self.screen, Screen::Library | Screen::Settings | Screen::Detail) {
            return Vec::new();
        }
        let n = self.games.len();
        let row = |r: usize| (r * LIBRARY_COLUMNS).min(n)..((r + 1) * LIBRARY_COLUMNS).min(n);
        let mut order: Vec<usize> = (self.first_row..self.first_row + GRID_ROWS).flat_map(row).collect();
        order.extend(row(self.first_row + GRID_ROWS));
        if self.first_row > 0 {
            order.extend(row(self.first_row - 1));
        }
        let mut out = Vec::new();
        for i in order {
            if self.art_asked >= ART_INFLIGHT {
                break;
            }
            let url = self.games[i].image_url.clone();
            let due = match self.art.get(&url) {
                None => true,
                Some(Art::Missing(at)) => self.now_ms >= at + ART_AGAIN_MS,
                Some(_) => false,
            };
            if url.is_empty() || !due {
                continue;
            }
            self.art.insert(url.clone(), Art::Pending);
            self.art_asked += 1;
            out.push(Self::req("artwork.resolve", json!({"sourceUrl": url})));
        }
        out
    }

    fn open_settings(&mut self) -> Vec<Action> {
        self.screen = Screen::Settings;
        self.settings_row = 0;
        self.help_open = false;
        self.ok_down_ms = None;
        self.ok_held = false;
        self.refresh_account(true)
    }

    fn settings_key(&mut self, k: Key) -> Vec<Action> {
        if self.help_open {
            self.help_open = false;   /* any key closes the explanation, and is not also a command */
            return Vec::new();
        }
        match k {
            Key::Back => {
                self.screen = Screen::Library;
                self.ok_down_ms = None;
                self.ok_held = false;
                Vec::new()
            }
            Key::Up if self.settings_row > 0 => {
                self.settings_row -= 1;
                Vec::new()
            }
            Key::Down if self.settings_row + 1 < Row::ALL.len() => {
                self.settings_row += 1;
                Vec::new()
            }
            Key::Left => self.settings_step(-1, false),
            Key::Right => self.settings_step(1, false),
            Key::Ok => self.settings_step(1, true),
            _ => Vec::new(),
        }
    }

    /// A row's value one rung along (round to the other end on OK); a changed value is kept in the preferences file and, when the core
    /// needs it, sent to the core.
    fn settings_step(&mut self, step: i32, wrap: bool) -> Vec<Action> {
        let row = Row::ALL[self.settings_row];
        if row == Row::SignOut {
            return if wrap { self.ask_sign_out() } else { Vec::new() };
        }
        if !row.step(&mut self.prefs, step, wrap) {
            return Vec::new();
        }
        let mut out = vec![Action::SavePrefs(self.prefs)];
        let settings = self.prefs.settings();
        for key in row.keys() {
            self.settings_queue.push_back(((*key).to_owned(), settings[*key].clone()));
        }
        if !self.has_pending("settings.set") {
            out.extend(self.next_setting());
        }
        out
    }

    /// The next setting waiting to go to the core, as a request.
    fn next_setting(&mut self) -> Option<Action> {
        self.settings_queue.pop_front().map(|(key, value)| Self::req("settings.set", json!({"key": key, "value": value})))
    }

    /// A setting was answered (taken or refused): the next goes, and when the handshake's batch is through the streamer is asked for.
    fn after_setting(&mut self) -> Vec<Action> {
        if let Some(next) = self.next_setting() {
            return vec![next];
        }
        if self.boot_settings {
            self.boot_settings = false;
            return vec![Self::req("streamer.detect", json!({})), Self::req("auth.session.get", json!({}))];
        }
        Vec::new()
    }

    /// OK on the settings screen is told by how long it is held: pressed and let go it is the row's action, held for `HOLD_MS` it opens the
    /// row's explanation. While an explanation is open, OK closes it and the release that follows does nothing.
    fn settings_ok(&mut self, pressed: bool) -> Vec<Action> {
        if pressed {
            if self.help_open {
                self.help_open = false;
                self.ok_swallow = true;
                self.ok_down_ms = None;
                self.ok_held = false;
            } else if self.ok_down_ms.is_none() {
                self.ok_down_ms = Some(self.now_ms);
                self.ok_held = false;
            }
            return Vec::new();
        }
        let down = self.ok_down_ms.take();
        let swallowed = std::mem::take(&mut self.ok_swallow);
        let held = std::mem::take(&mut self.ok_held);
        if swallowed || held || down.is_none() {
            return Vec::new();
        }
        self.settings_step(1, true)
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
        if self.detail.is_none() || self.decision != "ready" {
            return Vec::new();
        }
        self.screen = Screen::Launching;
        self.session = None;
        self.stats = Stats::default();
        self.pad_latency = None;
        self.launch_retried = false;
        self.start_asked = false;
        self.retry_stage = 0;
        self.create_request().into_iter().collect()
    }

    /// `session.create` for the game on the detail card, in the account scope last heard of: the core refuses a launch whose scope is not
    /// the account's current one (`stale_account`), and the scope is the auth envelope's generation with the user and provider, which the
    /// envelope does not carry as a `scope` object.
    fn create_request(&self) -> Option<Action> {
        let g = self.detail.as_ref()?;
        let mut params = json!({
            "appId": g.variant_id,
            "catalogAppId": g.id,
            "variantId": g.variant_id,
            "scope": self.scope,
            "selectionIdentity": format!("{}:{}", g.id, g.variant_id),
            "title": g.title,
            "store": g.store,
            "accountLinked": g.in_library,
            // a TV played with a pad: GFN opens Steam (and launchers that have one) in its controller view, Big Picture
            "appLaunchMode": "gamepadFriendly",
            "runtimeCapabilities": self.runtime_capabilities,
            "maxEntitledFps": 120,
        });
        if let Some((zone, url)) = &self.region {
            params["zone"] = json!(zone);
            params["streamingBaseUrl"] = json!(url);
        }
        Some(Self::req("session.create", params))
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
                    self.prefs.show_stats = !self.prefs.show_stats;
                    let mut out = vec![Action::SavePrefs(self.prefs)];
                    out.extend(self.close_overlay());
                    out
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
        self.retry_stage = 0;
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
        self.retry_stage = 0;
        self.screen = Screen::Error;
        Vec::new()
    }

    /// Re-reads the account: the session (which renews an expiring token and gives the current scope) and the subscription (the tier
    /// as NVIDIA has it now, so an upgrade shows without signing out). Unless `force`, not within [`ACCOUNT_REFRESH_MS`] of the last read.
    fn refresh_account(&mut self, force: bool) -> Vec<Action> {
        if !self.signed_in || self.has_pending("auth.session.get") || self.has_pending("account.subscription.get") {
            return Vec::new();
        }
        if !force && self.account_at_ms.is_some_and(|t| self.now_ms < t + ACCOUNT_REFRESH_MS) {
            return Vec::new();
        }
        self.account_at_ms = Some(self.now_ms);
        vec![Self::req("auth.session.get", json!({})), Self::req("account.subscription.get", json!({}))]
    }

    /// The sign-out question.
    fn ask_sign_out(&mut self) -> Vec<Action> {
        self.screen = Screen::SignOut;
        self.confirm_row = 0;
        self.help_open = false;
        self.ok_down_ms = None;
        self.ok_held = false;
        Vec::new()
    }

    fn sign_out_key(&mut self, k: Key) -> Vec<Action> {
        match k {
            Key::Back => {
                self.screen = Screen::Settings;
                Vec::new()
            }
            Key::Up if self.confirm_row > 0 => {
                self.confirm_row -= 1;
                Vec::new()
            }
            Key::Down if self.confirm_row < 1 => {
                self.confirm_row += 1;
                Vec::new()
            }
            Key::Ok if self.confirm_row == 1 => self.sign_out(),
            Key::Ok => {
                self.screen = Screen::Settings;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Signs every saved account out of the core (the app has no account picker, so no other account may be left behind to come back
    /// as the active one), which drops the tokens and the core's per-account state; the app drops its own when the core has answered.
    pub fn signing_out(&self) -> bool {
        self.has_pending("auth.accounts.logoutAll")
    }

    fn sign_out(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        if self.session.is_some() {
            out.extend(self.stop_session());
        }
        self.screen = Screen::Starting;
        out.push(Self::req("auth.accounts.logoutAll", json!({})));
        out
    }

    /// Forgets everything that belongs to the account: who it is, its tier and scope, the library and the game on show, the seat, the
    /// pictures and the sign-in attempt. Answers still on their way for it are dropped when they arrive.
    fn reset_account(&mut self) {
        for (id, method) in &self.pending {
            if matches!(method.as_str(), "catalog.library.list" | "catalog.launch.inspect" | "account.subscription.get" | "network.regions.list" | "auth.session.get" | "auth.accounts.logoutAll") {
                self.orphans.insert(id.clone());
            }
        }
        self.signed_in = false;
        self.user_name.clear();
        self.tier.clear();
        self.tier_live = false;
        self.scope = Value::Null;
        self.games.clear();
        self.next_cursor.clear();
        self.catalog_revision = Value::Null;
        self.catalog_context = Value::Null;
        self.focus = 0;
        self.first_row = 0;
        self.header_focus = false;
        self.detail = None;
        self.decision.clear();
        self.decision_message.clear();
        self.session = None;
        self.stats = Stats::default();
        self.pad_latency = None;
        self.region = None;
        self.regions_asked = false;
        self.account_at_ms = None;
        self.account_due_ms = 0;
        self.retry_stage = 0;
        self.launch_retried = false;
        self.confirm_row = 0;
        self.error.clear();
        self.sign_in = SignIn::default();
        self.art.clear();
        self.art_retry_ms = 0;
    }

    /// The scope a launch carries, from an auth envelope: its generation with the user and the provider of its session.
    fn scope_of(envelope: &Value) -> Value {
        json!({"generation": envelope["generation"], "providerIdpId": envelope["session"]["provider"]["idpId"], "userId": envelope["session"]["user"]["userId"]})
    }

    /// Takes the scope a scoped answer (a library page, an inspection, the subscription) reports, when it is the same account's and not
    /// older: the generation moves on when the core renews its provider list or restores a session, and a launch must carry the latest.
    fn adopt_scope(&mut self, scope: &Value) {
        if !self.signed_in || !scope.is_object() {
            return;
        }
        let same_owner = scope["userId"] == self.scope["userId"] && scope["providerIdpId"] == self.scope["providerIdpId"];
        if self.scope.is_null() || (same_owner && scope["generation"].as_u64() >= self.scope["generation"].as_u64()) {
            self.scope = scope.clone();
        }
    }

    /// An answer for an account that is no longer the one on screen.
    fn foreign(&self, r: &Value) -> bool {
        r["scope"].is_object() && self.scope.is_object() && (r["scope"]["userId"] != self.scope["userId"] || r["scope"]["providerIdpId"] != self.scope["providerIdpId"])
    }

    /// Notes a request the loop sent, so answers can be matched against what was asked.
    pub fn sent(&mut self, id: &str, method: &str) {
        self.pending.push((id.to_owned(), method.to_owned()));
        if method == "session.create" {
            self.create_id = id.to_owned();
        }
    }

    /// [`App::sent`] for a request whose parameters matter to the answer: a picture request's address is what its answer is about.
    pub fn sent_params(&mut self, id: &str, method: &str, params: &Value) {
        self.sent(id, method);
        if method == "artwork.resolve" {
            if let Some(url) = params["sourceUrl"].as_str() {
                self.art_ids.insert(id.to_owned(), url.to_owned());
            }
        }
    }

    fn core(&mut self, m: crate::gfn::core::CoreMsg) -> Vec<Action> {
        use crate::gfn::core::CoreMsg;
        match m {
            CoreMsg::Gone => self.fail("The GeForce NOW core stopped. Open the app again.".to_owned()),
            CoreMsg::Event { name, payload } => self.event(&name, payload),
            CoreMsg::Response { id, method, ok, result, error_code, error_message } => {
                self.pending.retain(|(i, _)| *i != id);
                if method == "artwork.resolve" {
                    self.art_asked = self.art_asked.saturating_sub(1);
                    let mut out = if ok { self.response(&id, &method, result) } else { self.art_failed(&id, &error_code) };
                    out.extend(self.art_wanted());
                    return out;
                }
                if self.orphans.remove(&id) {
                    return Vec::new();
                }
                if !ok {
                    return self.error_response(&method, &error_code, &error_message);
                }
                self.adopt_scope(&result["scope"]);
                self.response(&id, &method, result)
            }
        }
    }

    /// A picture request failed: a picture is never worth an error screen. Busy is asked again soon; anything else leaves the placeholder.
    fn art_failed(&mut self, id: &str, code: &str) -> Vec<Action> {
        if let Some(url) = self.art_ids.remove(id) {
            if code == "busy" {
                self.art.remove(&url);
                self.art_retry_ms = self.now_ms + ART_RETRY_MS;
            } else {
                self.art.insert(url, Art::Missing(self.now_ms));
            }
        }
        Vec::new()
    }

    fn error_response(&mut self, method: &str, code: &str, message: &str) -> Vec<Action> {
        match method {
            "auth.device.poll" => {
                self.sign_in.poll_id.clear();
                self.next_poll_ms = self.now_ms + self.sign_in.interval_s.max(1) * 1000;
                Vec::new()
            }
            // a setting the core refuses is not the app's end: the rest goes on; the region is an optimisation, the core picks one without
            "settings.set" => self.after_setting(),
            "network.regions.list" => Vec::new(),
            // a re-read that failed (no network, the service busy) does not end the sign-in; the next one tries again
            "auth.session.get" if self.signed_in => {
                if self.retry_stage != 0 {
                    return self.fail(STALE_ACCOUNT.to_owned());
                }
                Vec::new()
            }
            "auth.session.get" => {
                self.signed_in = false;
                self.screen = Screen::SignIn;
                vec![Self::req("auth.device.start", json!({}))]
            }
            "account.subscription.get" => {
                if code == "busy" {
                    self.account_due_ms = self.now_ms + ART_RETRY_MS;
                }
                Vec::new()
            }
            // the launch carried an account scope the core no longer holds: once, the session is read again and the game inspected again
            // (the core hands out the current scope with both), and the launch goes out with that; a second refusal is the player's to act on
            "session.create" if code == "stale_account" && !self.launch_retried => {
                self.launch_retried = true;
                self.retry_stage = 1;
                vec![Self::req("auth.session.get", json!({}))]
            }
            "session.create" if code == "stale_account" => self.fail(STALE_ACCOUNT.to_owned()),
            "auth.accounts.logoutAll" => self.fail(if message.is_empty() { format!("Could not sign out ({code})") } else { format!("Could not sign out: {message} ({code})") }),
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
                let settings = self.prefs.settings();
                if let Some(map) = settings.as_object() {
                    self.settings_queue.extend(map.iter().map(|(k, v)| (k.clone(), v.clone())));
                }
                if !self.engine_path.is_empty() {
                    self.settings_queue.push_back(("nativeStreamerExecutablePath".to_owned(), json!(self.engine_path)));
                }
                self.boot_settings = true;
                self.after_setting()
            }
            "streamer.detect" => {
                let mut caps = r["capabilities"].clone();
                caps["nativeHdrSupported"] = json!(true);
                caps["nativeHdrDisplay"] = json!({"minimumNits": 0, "maximumNits": 1000, "maximumFullFrameNits": 400});
                self.runtime_capabilities = caps;
                Vec::new()
            }
            "settings.set" => self.after_setting(),
            "network.regions.list" => {
                if let Some(first) = r["regions"].as_array().and_then(|a| a.first()) {
                    if let (Some(n), Some(u)) = (first["name"].as_str(), first["url"].as_str()) {
                        self.region = Some((n.to_owned(), u.to_owned()));
                    }
                }
                Vec::new()
            }
            "auth.session.get" | "auth.device.complete" => {
                if !r["session"].is_object() {
                    self.reset_account();
                    self.screen = Screen::SignIn;
                    return vec![Self::req("auth.device.start", json!({}))];
                }
                let same_account = self.signed_in && method == "auth.session.get" && r["session"]["user"]["userId"] == self.scope["userId"];
                self.scope = Self::scope_of(&r);
                self.user_name = r["session"]["user"]["displayName"].as_str().unwrap_or("").to_owned();
                if !self.tier_live {
                    self.tier = r["session"]["user"]["membershipTier"].as_str().unwrap_or("").to_owned();
                }
                if same_account {
                    // a re-read, not a sign-in: the library, the card and the focus stay where they are
                    if self.retry_stage == 1 {
                        let Some(g) = self.detail.as_ref() else { return self.fail(STALE_ACCOUNT.to_owned()) };
                        self.retry_stage = 2;
                        return vec![Self::req("catalog.launch.inspect", json!({"appId": g.id, "variantId": g.variant_id}))];
                    }
                    return Vec::new();
                }
                self.signed_in = true;
                self.account_at_ms = Some(self.now_ms);
                self.screen = Screen::Library;
                self.games.clear();
                self.next_cursor.clear();
                self.focus = 0;
                let mut out = vec![self.library_request(), Self::req("account.subscription.get", json!({}))];
                if !self.regions_asked {
                    self.regions_asked = true;
                    out.push(Self::req("network.regions.list", json!({})));
                }
                out
            }
            "account.subscription.get" => {
                if self.foreign(&r) {
                    return Vec::new();
                }
                if let Some(tier) = r["subscription"]["membershipTier"].as_str().filter(|t| !t.is_empty()) {
                    self.tier = tier.to_owned();
                    self.tier_live = true;
                }
                Vec::new()
            }
            "auth.accounts.logoutAll" => {
                if self.signed_in {
                    self.reset_account();
                }
                self.screen = Screen::SignIn;
                if self.sign_in.attempt_id.is_empty() && !self.has_pending("auth.device.start") {
                    vec![Self::req("auth.device.start", json!({}))]
                } else {
                    Vec::new()
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
                if self.foreign(&r) {
                    return Vec::new();
                }
                for g in r["games"].as_array().into_iter().flatten() {
                    self.games.push(game_of(g));
                }
                self.next_cursor = r["nextCursor"].as_str().unwrap_or("").to_owned();
                self.catalog_revision = r["catalogRevision"].clone();
                self.catalog_context = r["catalogContext"].clone();
                self.art_wanted()
            }
            "catalog.launch.inspect" => {
                if self.foreign(&r) {
                    return Vec::new();
                }
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
                if self.retry_stage == 2 {
                    self.retry_stage = 0;
                    if self.screen == Screen::Launching {
                        if self.decision == "ready" {
                            return self.create_request().into_iter().collect();
                        }
                        self.screen = Screen::Detail;
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
            "artwork.resolve" => {
                let Some(url) = self.art_ids.remove(id) else { return Vec::new() };
                if let Some(norm) = r["sourceUrl"].as_str() {
                    if norm != url {
                        self.art_alias.insert(norm.to_owned(), url.clone());
                    }
                }
                let state = match r["status"].as_str() {
                    Some("ready") if r["filePath"].as_str().is_some_and(|p| !p.is_empty()) => Art::File(r["filePath"].as_str().unwrap_or("").to_owned()),
                    Some("pending") => Art::Pending,
                    _ => Art::Missing(self.now_ms),
                };
                self.art.insert(url, state);
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
            if ready && !self.start_asked && !self.has_pending("streamer.start") {
                self.next_poll_ms = 0;
                self.start_asked = true;
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
                if payload["session"].is_null() {
                    if !self.signed_in {
                        return Vec::new();
                    }
                    let mut out = Vec::new();
                    if self.session.is_some() {
                        out.extend(self.stop_session());
                    }
                    self.reset_account();
                    self.screen = Screen::SignIn;
                    out.push(Self::req("auth.device.start", json!({})));
                    return out;
                }
                // the core moved to a newer generation of the same account: launches carry that one
                self.adopt_scope(&Self::scope_of(&payload));
                Vec::new()
            }
            "artwork.ready" => {
                let reported = payload["sourceUrl"].as_str().unwrap_or("");
                let url = self.art_alias.get(reported).map(String::as_str).unwrap_or(reported).to_owned();
                if let Some(slot) = self.art.get_mut(&url) {
                    *slot = match payload["filePath"].as_str() {
                        Some(p) if payload["cached"] == true && !p.is_empty() => Art::File(p.to_owned()),
                        _ => Art::Missing(self.now_ms),
                    };
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
        image_url: g["imageUrl"].as_str().or_else(|| g["keyArtUrl"].as_str()).or_else(|| g["heroImageUrl"].as_str()).unwrap_or("").to_owned(),
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

    /// What the loop does with the requests the rules return: each is sent and noted with its id (`art<N>`, in order); the picture
    /// requests come back as (id, address).
    fn sent_art(app: &mut App, actions: &[Action], n: &mut u32) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for a in actions {
            if let Action::Request { method, params } = a {
                *n += 1;
                let id = format!("art{n}");
                app.sent_params(&id, method, params);
                if method == "artwork.resolve" {
                    out.push((id, params["sourceUrl"].as_str().unwrap_or("").to_owned()));
                }
            }
        }
        out
    }

    fn ready(url: &str, path: &str) -> Value {
        json!({"status": "ready", "sourceUrl": url, "artworkUrl": format!("file://{path}"), "filePath": path, "cached": true})
    }

    /// The settings the handshake sends one request at a time (the core takes one key and value, and a few requests at once): each is
    /// answered, and what comes after the last answer is returned with the settings as the core was told them.
    fn through_settings(app: &mut App, mut a: Vec<Action>) -> (Vec<Action>, serde_json::Map<String, Value>) {
        let mut told = serde_json::Map::new();
        let mut n = 0;
        while let [Action::Request { method, params }] = a.as_slice() {
            if method != "settings.set" {
                break;
            }
            assert_eq!(told.insert(params["key"].as_str().unwrap().to_owned(), params["value"].clone()), None, "a key is sent once");
            n += 1;
            let id = format!("set{n}");
            app.sent(&id, "settings.set");
            a = app.handle(ok(&id, "settings.set", json!({"key": "k", "value": null})));
        }
        (a, told)
    }

    /// The auth envelope the core answers `auth.session.get` and `auth.device.complete` with: the session's user and provider and the
    /// account generation; it carries no `scope` object (the app builds one from these).
    fn envelope(generation: u64, user: &str, tier: &str) -> Value {
        json!({"session": {"user": {"userId": user, "displayName": "Nitin", "membershipTier": tier}, "provider": {"idpId": "idp"}},
            "generation": generation, "persistence": "secure-store", "refresh": {"attempted": false}, "warnings": [], "deviceIdentity": "durable"})
    }

    fn scope(generation: u64, user: &str) -> Value {
        json!({"generation": generation, "providerIdpId": "idp", "userId": user})
    }

    /// The handshake through the library, with the sign-in in between: the requests in OpenNOW's order.
    fn signed_in_app() -> App {
        let mut app = App::new().with_engine("/app/bin64/opennow-streamer");
        assert_eq!(methods(&app.start()), ["core.hello"]);
        app.sent("1", "core.hello");
        let a = app.handle(ok("1", "core.hello", json!({"capabilities": ["nativeStreamer.v7"]})));
        let (a, told) = through_settings(&mut app, a);
        assert_eq!(told["resolution"], "3840x2160");
        assert_eq!(told["fps"], 120);
        assert_eq!(told["enableHdr"], true);
        assert_eq!(told["nativeStreamerExecutablePath"], "/app/bin64/opennow-streamer");
        assert_eq!(methods(&a), ["streamer.detect", "auth.session.get"], "the streamer is asked for once the core knows where it is");
        app.sent("2", "streamer.detect");
        app.sent("3", "auth.session.get");
        let a = app.handle(ok("2", "streamer.detect", json!({"available": true, "capabilities": {"protocolVersion": 7, "videoBackends": [{"backend": "ndl"}]}})));
        assert!(a.is_empty(), "signed out, the core refuses the regions: they are asked for after the sign-in");
        assert_eq!(app.runtime_capabilities["nativeHdrSupported"], true);
        assert_eq!(app.runtime_capabilities["nativeHdrDisplay"]["maximumNits"], 1000);
        let a = app.handle(ok("3", "auth.session.get", json!({"session": null, "generation": 1})));
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
        let a = app.handle(ok("7", "auth.device.complete", envelope(2, "u", "FREE")));
        assert_eq!(app.screen, Screen::Library);
        assert_eq!((app.user_name.as_str(), app.tier.as_str()), ("Nitin", "FREE"), "until the subscription is read, the tier in the sign-in token");
        assert_eq!(app.scope, scope(2, "u"), "the launch scope is built from the envelope, which has none");
        assert_eq!(methods(&a), ["catalog.library.list", "account.subscription.get", "network.regions.list"]);
        app.sent("7a", "account.subscription.get");
        assert!(app.handle(ok("7a", "account.subscription.get", json!({"subscription": {"membershipTier": "ULTIMATE"}, "scope": scope(2, "u")}))).is_empty());
        assert_eq!(app.tier, "ULTIMATE", "the subscription's tier wins over the token's");
        app.sent("7b", "network.regions.list");
        assert!(app.handle(err("7b", "network.regions.list", "authentication_required", "Sign in")).is_empty(), "a refused region question is not an error screen");
        assert_eq!(app.screen, Screen::Library);
        app.sent("8", "catalog.library.list");
        let a = app.handle(ok("8", "catalog.library.list", json!({"games": games(14), "nextCursor": "c2", "hasNextPage": true, "catalogRevision": 3, "catalogContext": "ctx"})));
        // the first pictures are in flight and stay so: the tests below that are not about pictures see no picture requests
        let mut n = 0;
        assert_eq!(sent_art(&mut app, &a, &mut n).len(), ART_INFLIGHT);
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
        // a second ready update before the loop records the request (the core's session.changed beside the poll's answer) asks nothing
        assert!(app.handle(ok("12b", "session.poll", json!({"session": ready}))).is_empty(), "one streamer.start a launch");
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

    #[test]
    fn the_library_asks_for_the_pictures_on_the_screen_three_at_a_time_and_each_answer_asks_for_the_next() {
        let mut app = App::new();
        app.screen = Screen::Library;
        let a = app.handle(ok("1", "catalog.library.list", json!({"games": games(30), "nextCursor": "", "catalogRevision": 1, "catalogContext": "c"})));
        let mut n = 0;
        let mut asked = sent_art(&mut app, &a, &mut n);
        let urls = |v: &[(String, String)]| v.iter().map(|(_, u)| u.clone()).collect::<Vec<_>>();
        assert_eq!(urls(&asked), ["https://x/0.jpg", "https://x/1.jpg", "https://x/2.jpg"], "the first row first, three at a time");
        // every answer lets exactly one more go, in the order of the screen: the two visible rows, then the row below, never more than three waiting
        let mut order = urls(&asked);
        let mut in_flight = asked.clone();
        while let Some((id, url)) = in_flight.first().cloned() {
            in_flight.remove(0);
            let a = app.handle(ok(&id, "artwork.resolve", ready(&url, &format!("/cache/{}", url.rsplit('/').next().unwrap()))));
            assert!(matches!(app.art.get(&url), Some(Art::File(_))));
            let more = sent_art(&mut app, &a, &mut n);
            assert!(more.len() <= 1, "one answer, at most one more request");
            order.extend(urls(&more));
            in_flight.extend(more);
            assert!(in_flight.len() <= ART_INFLIGHT);
            asked.clear();
        }
        let want: Vec<String> = (0..18).map(|i| format!("https://x/{i}.jpg")).collect();
        assert_eq!(order, want, "two visible rows (12) and the row below (6): the pictures of the row above and of the rest wait for the focus to move");
        // moving down two rows scrolls the grid by one: the row below that is asked for now (the rows already asked for are not)
        for _ in 0..2 {
            let a = app.handle(key(108));
            let more = sent_art(&mut app, &a, &mut n);
            order.extend(urls(&more));
        }
        assert_eq!(app.first_row, 1);
        assert_eq!(order.len(), 21, "the row below has six pictures: three go at once, the rest wait for answers");
        assert_eq!(order[18], "https://x/18.jpg");
    }

    #[test]
    fn a_picture_the_core_is_still_fetching_arrives_by_its_event_and_one_it_could_not_get_is_asked_for_again_after_a_minute() {
        let mut app = App::new();
        app.screen = Screen::Library;
        let a = app.handle(ok("1", "catalog.library.list", json!({"games": games(3), "nextCursor": "", "catalogRevision": 1, "catalogContext": "c"})));
        let mut n = 0;
        let asked = sent_art(&mut app, &a, &mut n);
        assert_eq!(asked.len(), 3);
        let (a0, u0) = asked[0].clone();
        let (a1, u1) = asked[1].clone();
        let (a2, u2) = asked[2].clone();
        // pending: the tile keeps its placeholder until the core says the file is there; the core spells the address its own way
        app.handle(ok(&a0, "artwork.resolve", json!({"status": "pending", "sourceUrl": "https://x/0.jpg?normalised"})));
        assert_eq!(app.art[&u0], Art::Pending);
        app.handle(Input::Core(CoreMsg::Event { name: "artwork.ready".into(), payload: json!({"sourceUrl": "https://x/0.jpg?normalised", "filePath": "/cache/a.jpg", "cached": true}) }));
        assert_eq!(app.art[&u0], Art::File("/cache/a.jpg".into()));
        // a download that failed: the placeholder stays, and nothing asks again for a minute
        app.handle(Input::Tick { now_ms: 5000 });
        app.handle(ok(&a1, "artwork.resolve", json!({"status": "pending", "sourceUrl": u1})));
        app.handle(Input::Core(CoreMsg::Event { name: "artwork.ready".into(), payload: json!({"sourceUrl": u1, "cached": false, "fallbackReason": "network-error"}) }));
        assert_eq!(app.art[&u1], Art::Missing(5000));
        app.handle(ok(&a2, "artwork.resolve", json!({"status": "fallback", "sourceUrl": u2, "fallbackReason": "queue-full"})));
        assert_eq!(app.art[&u2], Art::Missing(5000));
        app.handle(Input::Tick { now_ms: 5000 + ART_AGAIN_MS - 1 });
        assert!(methods(&app.handle(key(106))).is_empty(), "not yet");
        app.handle(Input::Tick { now_ms: 5000 + ART_AGAIN_MS });
        let a = app.handle(key(105));
        let again = sent_art(&mut app, &a, &mut n);
        assert_eq!(again.iter().map(|(_, u)| u.as_str()).collect::<Vec<_>>(), ["https://x/1.jpg", "https://x/2.jpg"]);
        assert_eq!(app.art[&u1], Art::Pending);
    }

    #[test]
    fn a_busy_core_gets_the_picture_request_again_after_a_moment_and_a_picture_never_makes_an_error_screen() {
        let mut app = App::new();
        app.screen = Screen::Library;
        app.handle(Input::Tick { now_ms: 100 });
        let a = app.handle(ok("1", "catalog.library.list", json!({"games": games(2), "nextCursor": "", "catalogRevision": 1, "catalogContext": "c"})));
        let mut n = 0;
        let asked = sent_art(&mut app, &a, &mut n);
        let a = app.handle(err(&asked[0].0, "artwork.resolve", "busy", "the core is busy"));
        assert!(a.is_empty(), "no new request while the core says busy");
        assert_eq!(app.screen, Screen::Library);
        assert!(!app.art.contains_key(&asked[0].1));
        assert_eq!(app.next_wake(), Some(Duration::from_millis(ART_RETRY_MS)));
        app.handle(err(&asked[1].0, "artwork.resolve", "invalid_params", "not a picture address"));
        assert_eq!(app.screen, Screen::Library, "a refused picture is not an error screen");
        assert_eq!(app.art[&asked[1].1], Art::Missing(100));
        let a = app.handle(Input::Tick { now_ms: 100 + ART_RETRY_MS });
        let again = sent_art(&mut app, &a, &mut n);
        assert_eq!(again.iter().map(|(_, u)| u.as_str()).collect::<Vec<_>>(), ["https://x/0.jpg"]);
        assert_eq!(app.next_wake(), None);
    }

    #[test]
    fn the_settings_screen_is_reached_from_the_header_and_each_change_is_kept_and_sent_to_the_core() {
        use crate::gfn::prefs::Tier;
        let mut app = signed_in_app();
        app.handle(key(103));
        assert!(app.header_focus, "Up from the first row reaches the header's button");
        assert!(app.handle(key(108)).is_empty());
        assert!(!app.header_focus);
        app.handle(key(103));
        let a = app.handle(key(28));
        assert_eq!(methods(&a), ["auth.session.get", "account.subscription.get"], "opening the profile re-reads the account");
        assert_eq!(app.screen, Screen::Settings);
        // a row's change goes to the core a key at a time, the next after the answer to the one before
        let set = |app: &mut App, a: Vec<Action>| -> Value {
            assert!(matches!(a[0], Action::SavePrefs(_)), "the change is kept first");
            let (rest, told) = through_settings(app, a[1..].to_vec());
            assert!(rest.is_empty());
            Value::Object(told)
        };
        let a = app.handle(key(106));
        assert_eq!(app.prefs.tier, Tier::Qhd120);
        assert!(matches!(a[0], Action::SavePrefs(p) if p.tier == Tier::Qhd120));
        let p = set(&mut app, a);
        assert_eq!((p["resolution"].as_str(), p["fps"].as_u64()), (Some("2560x1440"), Some(120)));
        assert!(!app.handle(key(105)).is_empty());
        assert_eq!(app.prefs.tier, Tier::Uhd120);
        assert!(app.handle(key(105)).is_empty(), "nothing before the first rung: nothing to keep or send");
        app.handle(key(108));
        let a = app.handle(key(106));
        let p = set(&mut app, a);
        assert_eq!((p["enableHdr"].as_bool(), p["colorQuality"].as_str()), (Some(false), Some("8bit_420")));
        app.handle(key(108));
        let a = app.handle(key(106));
        let p = set(&mut app, a);
        assert_eq!(p["maxBitrateMbps"].as_u64(), Some(100));
        assert!(app.handle(key(106)).is_empty(), "the ladder ends at 100");
        app.handle(key(108));
        let a = app.handle(key(106));
        let p = set(&mut app, a);
        assert_eq!((p["controllerLeftStickDeadzone"].as_u64(), p["controllerRightStickDeadzone"].as_u64()), (Some(10), Some(10)));
        app.handle(key(108));
        let a = app.handle(key(106));
        assert_eq!(a.len(), 1, "the stats line is the app's own: kept, not sent");
        assert!(matches!(a[0], Action::SavePrefs(p) if p.show_stats));
        assert!(app.handle(key(108)).is_empty());
        assert_eq!(app.settings_row, 5, "the sign-out row is last; Left and Right do nothing there");
        assert!(app.handle(key(106)).is_empty() && app.handle(key(108)).is_empty());
        assert_eq!(app.settings_row, 5);
        // a setting the core refuses does not end the app
        app.sent("s1", "settings.set");
        assert!(app.handle(err("s1", "settings.set", "invalid_setting", "no")).is_empty());
        assert_eq!(app.screen, Screen::Settings);
        app.handle(key(412));
        assert_eq!(app.screen, Screen::Library);
        assert!(app.header_focus, "back to the button the screen was opened from");
        assert_eq!(app.handle(key(412)), vec![Action::Exit("back on the library".into())]);
    }

    fn open_settings_row(app: &mut App, row: usize) {
        app.handle(key(103));
        app.handle(key(28));
        assert_eq!(app.screen, Screen::Settings);
        for _ in 0..row {
            app.handle(key(108));
        }
        assert_eq!(app.settings_row, row);
    }

    /// Marks each request of `a` as sent under ids `<prefix>1`, `<prefix>2`, ... and returns the ids by method.
    fn send_all(app: &mut App, a: &[Action], prefix: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (i, act) in a.iter().enumerate() {
            if let Action::Request { method, params } = act {
                let id = format!("{prefix}{}", i + 1);
                app.sent_params(&id, method, params);
                out.push((id, method.clone()));
            }
        }
        out
    }

    #[test]
    fn an_upgrade_shows_when_the_header_or_the_settings_are_opened_or_the_app_comes_back_and_the_library_stays() {
        let mut app = signed_in_app();
        // the account was a free one when the app started
        app.sent("f1", "account.subscription.get");
        app.handle(ok("f1", "account.subscription.get", json!({"subscription": {"membershipTier": "FREE"}, "scope": scope(2, "u")})));
        assert_eq!(crate::gfn::layout::account_text(&app), "Nitin · Free");
        // upgraded on a phone: the header's focus within ten seconds of the last read asks nothing, later it re-reads
        app.handle(Input::Tick { now_ms: 5_000 });
        app.handle(key(103));
        assert!(app.header_focus);
        assert!(app.handle(key(108)).is_empty());
        app.handle(Input::Tick { now_ms: 20_000 });
        let a = app.handle(key(103));
        assert_eq!(methods(&a), ["auth.session.get", "account.subscription.get"]);
        let ids = send_all(&mut app, &a, "r");
        assert!(app.handle(key(108)).is_empty());
        assert!(app.handle(key(103)).is_empty(), "no second read while the first is out, or within ten seconds");
        // the token still says free (it is only renewed with the token); the subscription is what NVIDIA has now
        assert!(app.handle(ok(&ids[0].0, "auth.session.get", envelope(3, "u", "FREE"))).is_empty());
        assert_eq!(app.scope, scope(3, "u"));
        assert_eq!(app.tier, "FREE");
        assert!(app.handle(ok(&ids[1].0, "account.subscription.get", json!({"subscription": {"membershipTier": "PERFORMANCE"}, "scope": scope(3, "u")}))).is_empty());
        assert_eq!(app.tier, "PERFORMANCE");
        assert_eq!(crate::gfn::layout::account_text(&app), "Nitin · Performance");
        assert_eq!((app.screen, app.games.len()), (Screen::Library, 14), "a re-read is not a sign-in: the library stays");
        // a later token renewal with the old claim does not take the tier back
        app.sent("r9", "auth.session.get");
        app.handle(ok("r9", "auth.session.get", envelope(3, "u", "FREE")));
        assert_eq!(app.tier, "PERFORMANCE");
        // coming back from being covered re-reads at once
        app.handle(Input::State(crate::card::MINIMIZED));
        let a = app.handle(Input::State(crate::card::FULLSCREEN));
        assert_eq!(methods(&a), ["auth.session.get", "account.subscription.get"]);
        // a re-read the core refused as busy is asked again after a moment; a failed one never shows an error
        let ids = send_all(&mut app, &a, "b");
        assert!(app.handle(err(&ids[0].0, "auth.session.get", "session_refresh_failed", "offline")).is_empty());
        assert_eq!(app.screen, Screen::Library, "a failed re-read leaves the account signed in");
        app.handle(Input::Tick { now_ms: 30_000 });
        assert!(app.handle(err(&ids[1].0, "account.subscription.get", "busy", "busy")).is_empty());
        assert!(app.handle(Input::Tick { now_ms: 30_500 }).is_empty());
        let a = app.handle(Input::Tick { now_ms: 31_600 });
        assert_eq!(methods(&a), ["account.subscription.get"]);
        // a subscription of another account is not shown
        app.sent("x1", "account.subscription.get");
        app.handle(ok("x1", "account.subscription.get", json!({"subscription": {"membershipTier": "ULTIMATE"}, "scope": scope(3, "someone-else")})));
        assert_eq!(app.tier, "PERFORMANCE");
        // a session that is gone (the token was revoked) is the sign-in screen
        app.sent("g1", "auth.session.get");
        let a = app.handle(ok("g1", "auth.session.get", json!({"session": null, "generation": 4})));
        assert_eq!((app.screen, methods(&a)), (Screen::SignIn, vec!["auth.device.start".to_owned()]));
        assert!(app.games.is_empty() && app.tier.is_empty());
    }

    #[test]
    fn signing_out_asks_first_then_drops_the_tokens_and_everything_of_the_account_and_a_new_sign_in_works() {
        let mut app = signed_in_app();
        open_settings_row(&mut app, 5);
        // a row that is only chosen does not change on Left and Right
        assert!(app.handle(key(106)).is_empty() && app.handle(key(105)).is_empty());
        assert_eq!(app.screen, Screen::Settings);
        app.handle(Input::Key { code: 28, pressed: true });
        assert!(app.handle(Input::Key { code: 28, pressed: false }).is_empty());
        assert_eq!((app.screen, app.confirm_row), (Screen::SignOut, 0), "the question comes first and keeps the account by default");
        assert!(app.handle(key(28)).is_empty());
        assert_eq!(app.screen, Screen::Settings, "OK on the first row keeps the account");
        assert!(app.signed_in);
        app.handle(Input::Key { code: 28, pressed: true });
        app.handle(Input::Key { code: 28, pressed: false });
        app.handle(key(412));
        assert_eq!(app.screen, Screen::Settings, "Back keeps the account");
        app.handle(Input::Key { code: 28, pressed: true });
        app.handle(Input::Key { code: 28, pressed: false });
        assert!(app.handle(key(108)).is_empty());
        assert!(app.handle(key(108)).is_empty(), "two rows only");
        assert_eq!(app.confirm_row, 1);
        // an answer for the library is still on its way when the player signs out
        app.sent("lib2", "catalog.library.list");
        app.detail = Some(app.games[1].clone());
        let a = app.handle(key(28));
        assert_eq!(methods(&a), ["auth.accounts.logoutAll"], "every saved account goes, so none comes back as the active one");
        let ids = send_all(&mut app, &a, "o");
        assert!(app.signing_out());
        assert_eq!(crate::gfn::layout::rows(&app), vec![("Status".to_owned(), "Signing out".to_owned())]);
        assert!(app.signed_in, "the account is dropped when the core says its tokens are gone");
        let a = app.handle(ok(&ids[0].0, "auth.accounts.logoutAll", json!({"session": null, "generation": 3, "ok": true})));
        assert_eq!((app.screen, methods(&a)), (Screen::SignIn, vec!["auth.device.start".to_owned()]));
        assert!(!app.signed_in && app.games.is_empty() && app.tier.is_empty() && app.user_name.is_empty());
        assert!(app.scope.is_null() && app.detail.is_none() && app.decision.is_empty() && app.region.is_none());
        assert!(app.art.is_empty() && app.session.is_none() && app.next_cursor.is_empty() && app.catalog_context.is_null());
        assert_eq!(app.sign_in, SignIn::default());
        assert!(!app.signing_out());
        // the old account's page arrives late: dropped
        assert!(app.handle(ok("lib2", "catalog.library.list", json!({"games": games(3), "nextCursor": "", "catalogRevision": 3, "catalogContext": "old", "scope": scope(2, "u")}))).is_empty());
        assert!(app.games.is_empty());
        // signing in again: the code, the account, and the same requests as the first time
        send_all(&mut app, &a, "n");
        app.handle(ok("n1", "auth.device.start", json!({"attemptId": "att2", "userCode": "WXYZ-1234", "verificationUri": "https://nvidia.com/activate", "qrRows": ["1"], "expiresAt": 999999, "intervalSeconds": 5})));
        assert_eq!(app.sign_in.user_code, "WXYZ-1234");
        app.sent("n2", "auth.device.complete");
        let a = app.handle(ok("n2", "auth.device.complete", envelope(4, "u2", "PERFORMANCE")));
        assert_eq!((app.screen, app.tier.as_str(), app.scope.clone()), (Screen::Library, "PERFORMANCE", scope(4, "u2")));
        assert_eq!(methods(&a), ["catalog.library.list", "account.subscription.get", "network.regions.list"]);
        // an answer for the other account is not taken for this one
        app.sent("n3", "catalog.library.list");
        app.handle(ok("n3", "catalog.library.list", json!({"games": games(2), "nextCursor": "", "catalogRevision": 1, "catalogContext": "c", "scope": scope(2, "u")})));
        assert!(app.games.is_empty());
    }

    #[test]
    fn a_sign_out_the_core_announces_first_is_the_same_as_one_it_answers_and_a_refused_one_is_an_error() {
        let mut app = signed_in_app();
        open_settings_row(&mut app, 5);
        app.handle(Input::Key { code: 28, pressed: true });
        app.handle(Input::Key { code: 28, pressed: false });
        app.handle(key(108));
        let a = app.handle(key(28));
        let ids = send_all(&mut app, &a, "o");
        // the event comes before the answer: one sign-in code is asked for, not two
        let a = app.handle(Input::Core(CoreMsg::Event { name: "auth.session.changed".into(), payload: json!({"session": null, "generation": 3}) }));
        assert_eq!((app.screen, methods(&a)), (Screen::SignIn, vec!["auth.device.start".to_owned()]));
        send_all(&mut app, &a, "s");
        assert!(app.handle(ok(&ids[0].0, "auth.accounts.logoutAll", json!({"session": null, "generation": 3, "ok": true}))).is_empty());
        assert_eq!(app.screen, Screen::SignIn);
        // a sign-out the core refuses says so, and the account stays
        let mut app = signed_in_app();
        open_settings_row(&mut app, 5);
        app.handle(Input::Key { code: 28, pressed: true });
        app.handle(Input::Key { code: 28, pressed: false });
        app.handle(key(108));
        let a = app.handle(key(28));
        let ids = send_all(&mut app, &a, "p");
        assert!(app.handle(err(&ids[0].0, "auth.accounts.logoutAll", "busy", "The core is busy")).is_empty());
        assert_eq!(app.screen, Screen::Error);
        assert!(app.error.starts_with("Could not sign out"));
        app.handle(key(28));
        assert_eq!((app.screen, app.signed_in), (Screen::Library, true));
    }

    #[test]
    fn a_launch_carries_the_scope_of_the_last_answer_and_a_stale_one_is_retried_once_with_fresh_state() {
        let mut app = signed_in_app();
        app.handle(key(106));
        app.handle(key(28));
        app.sent("10", "catalog.launch.inspect");
        // the core moved to generation 3 (its provider list was renewed) since the sign-in: the inspection says so
        let inspected = |scope: Value| json!({"decision": {"status": "ready", "message": ""}, "scope": scope, "game": {"id": "g1", "title": "Game One", "variants": [{"id": "v1", "store": "STEAM", "inLibrary": true}]}});
        app.handle(ok("10", "catalog.launch.inspect", inspected(scope(3, "u"))));
        let a = app.handle(key(28));
        let Action::Request { method, params } = &a[0] else { panic!("no request") };
        assert_eq!((method.as_str(), &params["scope"]), ("session.create", &scope(3, "u")));
        app.sent("11", "session.create");
        // the core refuses it all the same (the generation moved again): the session is read again, then the game inspected again
        let a = app.handle(err("11", "session.create", "stale_account", "This launch belongs to a different account context."));
        assert_eq!((app.screen, methods(&a)), (Screen::Launching, vec!["auth.session.get".to_owned()]));
        app.sent("12", "auth.session.get");
        let a = app.handle(ok("12", "auth.session.get", envelope(5, "u", "FREE")));
        assert_eq!(methods(&a), ["catalog.launch.inspect"]);
        assert_eq!((app.screen, app.games.len()), (Screen::Launching, 14), "the library and the card stay");
        app.sent("13", "catalog.launch.inspect");
        let a = app.handle(ok("13", "catalog.launch.inspect", inspected(scope(5, "u"))));
        let Action::Request { method, params } = &a[0] else { panic!("no request") };
        assert_eq!((method.as_str(), &params["scope"]), ("session.create", &scope(5, "u")));
        app.sent("14", "session.create");
        // refused again: a plain message, no third try
        let a = app.handle(err("14", "session.create", "stale_account", "The launch context changed. Try again."));
        assert!(a.is_empty());
        assert_eq!((app.screen, app.error.as_str()), (Screen::Error, STALE_ACCOUNT));
        // the next launch is its own: one retry again
        app.handle(key(28));
        assert_eq!(app.screen, Screen::Library);
        app.handle(key(28));
        app.sent("20", "catalog.launch.inspect");
        app.handle(ok("20", "catalog.launch.inspect", inspected(scope(5, "u"))));
        app.handle(key(28));
        app.sent("21", "session.create");
        let a = app.handle(err("21", "session.create", "stale_account", "x"));
        assert_eq!(methods(&a), ["auth.session.get"]);
        // Back while the retry is under way gives the launch up for good
        app.handle(key(412));
        assert_eq!(app.screen, Screen::Detail);
        app.sent("22", "auth.session.get");
        assert!(app.handle(ok("22", "auth.session.get", envelope(6, "u", "FREE"))).is_empty());
        assert_eq!(app.screen, Screen::Detail);
        // a launch refused for something else is not retried
        app.handle(key(28));
        app.sent("23", "session.create");
        let a = app.handle(err("23", "session.create", "launch_not_ready", "Not ready"));
        assert!(a.is_empty());
        assert_eq!(app.screen, Screen::Error);
    }

    #[test]
    fn ok_on_a_settings_row_steps_it_round_and_held_for_600_ms_opens_the_explanation_which_any_key_closes() {
        let mut app = signed_in_app();
        app.handle(key(103));
        app.handle(key(28));
        app.handle(Input::Tick { now_ms: 1000 });
        // a tap is a step with wrap
        assert!(app.handle(Input::Key { code: 28, pressed: true }).is_empty());
        assert_eq!(app.next_wake(), Some(Duration::from_millis(HOLD_MS)));
        app.handle(Input::Tick { now_ms: 1200 });
        assert!(!app.help_open);
        let a = app.handle(Input::Key { code: 28, pressed: false });
        assert_eq!(app.prefs.tier, crate::gfn::prefs::Tier::Qhd120);
        assert!(matches!(a[0], Action::SavePrefs(_)));
        // a hold opens the explanation of the focused row and the release that follows changes nothing
        app.handle(Input::Tick { now_ms: 2000 });
        app.handle(Input::Key { code: 28, pressed: true });
        app.handle(Input::Key { code: 28, pressed: true });
        assert_eq!(app.next_wake(), Some(Duration::from_millis(HOLD_MS)), "a repeat of the press does not restart the hold");
        app.handle(Input::Tick { now_ms: 2000 + HOLD_MS });
        assert!(app.help_open);
        assert_eq!(app.next_wake(), None);
        assert!(app.handle(Input::Key { code: 28, pressed: false }).is_empty());
        assert!(app.help_open && app.prefs.tier == crate::gfn::prefs::Tier::Qhd120);
        // a direction closes it and is not a move; OK closes it and its release is not a step
        assert!(app.handle(key(108)).is_empty());
        assert!(!app.help_open);
        assert_eq!(app.settings_row, 0);
        app.handle(Input::Tick { now_ms: 4000 });
        app.handle(Input::Key { code: 28, pressed: true });
        app.handle(Input::Tick { now_ms: 4000 + HOLD_MS });
        assert!(app.help_open);
        app.handle(Input::Key { code: 28, pressed: false });
        assert!(app.handle(Input::Key { code: 28, pressed: true }).is_empty());
        assert!(!app.help_open);
        assert!(app.handle(Input::Key { code: 28, pressed: false }).is_empty());
        assert_eq!(app.prefs.tier, crate::gfn::prefs::Tier::Qhd120, "the closing OK was not a step");
        // Back closes the explanation first, and only then leaves the screen
        app.handle(Input::Tick { now_ms: 6000 });
        app.handle(Input::Key { code: 28, pressed: true });
        app.handle(Input::Tick { now_ms: 6000 + HOLD_MS });
        app.handle(Input::Key { code: 28, pressed: false });
        assert!(app.help_open);
        app.handle(key(412));
        assert!(!app.help_open);
        assert_eq!(app.screen, Screen::Settings);
        app.handle(key(412));
        assert_eq!(app.screen, Screen::Library);
    }

    #[test]
    fn the_viewers_choices_are_what_the_handshake_sends_the_core_one_key_at_a_time_before_the_streamer_is_asked_for() {
        use crate::gfn::prefs::{Prefs, Tier};
        let mut app = App::with_prefs(Prefs { tier: Tier::Uhd60, hdr: false, bitrate_mbps: 50, deadzone: 12, show_stats: true });
        app.start();
        app.sent("1", "core.hello");
        let a = app.handle(ok("1", "core.hello", json!({})));
        assert_eq!(methods(&a), ["settings.set"], "one request, not the whole batch: the core answers busy past a few at a time");
        let (a, params) = through_settings(&mut app, a);
        assert_eq!((params["resolution"].as_str(), params["fps"].as_u64()), (Some("3840x2160"), Some(60)));
        assert_eq!((params["enableHdr"].as_bool(), params["colorQuality"].as_str(), params["maxBitrateMbps"].as_u64()), (Some(false), Some("8bit_420"), Some(50)));
        assert_eq!(params["controllerLeftStickDeadzone"].as_u64(), Some(12));
        assert!(!params.contains_key("nativeStreamerExecutablePath"), "no engine named, none sent");
        assert_eq!(methods(&a), ["streamer.detect", "auth.session.get"]);
        app.sent("2", "streamer.detect");
        let a = app.handle(ok("2", "streamer.detect", json!({"capabilities": {"protocolVersion": 7}})));
        assert!(a.is_empty());
        assert_eq!(app.prefs.summary(), "4K · 60 fps");
    }
}
