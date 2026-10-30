//! The Cameras app's model (`docs/research/cameras-ui-spec.md`): the grid of stills, the full view of one camera (live on the video plane,
//! or stills where live is not offered), the timeline strip over it, the events and clips sheet, the per-camera pop-up options page. Pure:
//! `update(Input, now)` returns the host's effects (helper asks, pictures to fetch), `view()` is what the surfaces draw, `due_at()` when
//! the next tick is wanted. The behaviour is the spec's; the cases the QML pop-up and viewer tests state (`tests/test_camera_popup.py`) are
//! this model's host tests.
//!
//! One ask of the helper is told from another by its tag (`pics::TAG_FIRST..=TAG_LAST`); a picture's answer is `Input::Fetched` with
//! `{"picture": {...}}` once the host has decoded it (`super::host`), or a refusal as the helper sent it.

use super::data::{self, Cameras, Live, LiveMode, LiveState, Popup, Pushed, Recordings, Settings};
use super::grid::{self, Dir, Focus as GFocus, Layout};
use super::options::{self, Options};
use super::pics::{self, Planner, Want};
use super::sheet::{self, Action as CardAction, Sheet};
use super::strip::{self, Ctx, Item as SItem, Out as SOut, Strip};
use super::view::*;
use super::words::{self, Clock};
use crate::kit::hint::{Action as HAct, Key as HKey};
use crate::kit::screen::{Effect, Input, Model};
use serde_json::{Value, json};

/// The chrome over the picture goes this long after the last key.
pub const CHROME_MS: u64 = 2500;
/// A refusal's words are shown this long.
pub const TOAST_MS: u64 = 3000;
/// The full view's stills, where live is not offered or has not started.
pub const FULL_W: u32 = 1280;
pub const FULL_H: u32 = 720;
pub const FULL_EVERY_MS: u64 = 125;
/// At most this many asks for pictures are in flight.
pub const FLIGHT: usize = 8;
/// A still shown under chrome is "live" while it is this fresh.
pub const FRESH_STILL_MS: u64 = 3000;
/// The grid's tiles over an app (its video plays behind the grid): no more often than this whatever the owner chose, so that the previews take no frames from it.
pub const OVER_APP_TILE_MS: u64 = 500;

#[derive(Clone, Debug)]
pub struct Config {
    pub text_scale: f32,
    pub clock: Clock,
    /// `theme.cameras.liveOverApps`: live video over a playing app (check C12).
    pub live_over_apps: bool,
    /// How long OK is held for the options page (the theme's `motion.holdMs`).
    pub hold_ms: u64,
}

impl Config {
    /// The configuration the theme and the kit give: the text size, the 12 or 24 hour clock (`clock.twelveHour`, on unless the owner turned
    /// it off), live video over a playing app (`cameras.liveOverApps`, off until the feeder's window over an app is measured) and how long
    /// OK is held for its second meaning (`motion.holdMs`).
    pub fn from_theme(kit: &crate::kit::Kit, theme: &Value) -> Self {
        Self {
            text_scale: kit.text_scale,
            clock: Clock { twelve_hour: theme.pointer("/clock/twelveHour").and_then(Value::as_bool).unwrap_or(true), zone: super::words::Zone::system() },
            live_over_apps: theme.pointer("/cameras/liveOverApps").and_then(Value::as_bool).unwrap_or(false),
            hold_ms: u64::from(kit.tokens.motion.hold_ms),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self { text_scale: 1.0, clock: Clock::default(), live_over_apps: false, hold_ms: 700 }
    }
}

/// The id of the full view's claim on a spare video window (the compositor's `vwin`).
pub const CLAIM_ID: &str = "cameras-full";

/// What the compositor made of the full view's claim, as the picture needs to know it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Window {
    /// No claim is in force or the switch is off: the stream plays on the video plane as it always did.
    #[default]
    Unclaimed,
    /// A window is granted and not drawing yet: the layer is no hole until it is (a hole over nothing would be black).
    Waiting,
    /// The window carries the picture: the layer is a hole over it.
    Carried,
    /// Refused or taken back (no window, too big, both scalers, an app's video, standby): the plane path goes on and a glyph says so.
    Refused,
}

impl Window {
    /// The grant of the claim as the picture reads it. A switch that is off refuses nothing the person should hear of.
    pub fn of(grant: &crate::host::vwin::Grant) -> Window {
        use crate::host::vwin::Grant;
        match grant {
            Grant::None | Grant::Asked => Window::Unclaimed,
            Grant::Granted { placed: true, .. } => Window::Carried,
            Grant::Granted { placed: false, .. } => Window::Waiting,
            Grant::Denied { reason, .. } if reason == "off" => Window::Unclaimed,
            Grant::Denied { .. } | Grant::Preempted { .. } => Window::Refused,
        }
    }
}

/// The live video the full view wants a window for (see [`App::live_claim`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveClaim {
    pub camera: String,
    /// Where the picture goes in the 1920 by 1080 of the screen and the output: the stream fitted and centred.
    pub rect: (i32, i32, i32, i32),
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl LiveClaim {
    /// The claim as the compositor's `POST /v1/vwin` takes it (`vwin.rs` `Request::from_json`).
    pub fn request(&self) -> Value {
        let (x, y, w, h) = self.rect;
        json!({"id": CLAIM_ID, "kind": "camera-full", "rect": {"x": x, "y": y, "w": w, "h": h}, "need": {"codec": self.codec, "w": self.width, "h": self.height, "fps": self.fps}})
    }
}

/// A `width` by `height` picture fitted into the screen and centred; the whole screen when the size is not known.
fn fit(width: u32, height: u32) -> (i32, i32, i32, i32) {
    if width == 0 || height == 0 {
        return (0, 0, 1920, 1080);
    }
    let scale = (1920.0 / width as f32).min(1080.0 / height as f32);
    let (w, h) = (((width as f32 * scale).round() as i32).min(1920), ((height as f32 * scale).round() as i32).min(1080));
    ((1920 - w) / 2, (1080 - h) / 2, w, h)
}

/// What the app asks the shell for besides effects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// The grid's `Settings` button: Settings, Home & look, Cameras.
    OpenSettings,
    /// The full view was opened over an app and its picture is a video window's: Home takes the front, as an app of its own does (the app goes to the
    /// background). The shell remembers the app and launches Home.
    TakeForeground,
    /// Back closed a view that had taken the front: the shell brings back the app it took it from.
    ReturnToApp,
}

/// Something that can be clicked or focused: the surfaces name what a pointer hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Tile(usize),
    HeaderEvents,
    HeaderSettings,
    EmptySettings,
    Retry,
    /// The picture of the full view: a click on it brings the chrome back and does nothing else.
    Backdrop,
    Strip(SItem),
    /// The bar at this fraction of its width, in thousandths.
    Bar(u32),
    Tab(Tab),
    Card(usize),
    Option(usize),
    /// The way back of the options page: what Back does.
    Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    Live,
    Recording,
}

#[derive(Clone, Debug)]
struct GridState {
    focus: GFocus,
    top: usize,
    back_to: usize,
}

#[derive(Clone, Debug)]
struct FullState {
    /// The camera shown, by id (the list may change under it).
    camera: String,
    chrome: bool,
    chrome_until: u64,
}

#[derive(Clone, Debug)]
enum Screen {
    Closed,
    Grid(GridState),
    Full(FullState),
}

/// What an ask of the helper was for.
#[derive(Clone, Debug)]
enum Ask {
    Settings,
    Live { restore: Expect },
    Playback { restore: Expect },
    Pause,
    Recordings { camera: String },
    Ticks { camera: String },
    SheetEvents { camera: String },
    SheetClips { camera: String },
    Notify { camera: String },
}

pub struct App {
    pub cfg: Config,
    cams: Cameras,
    live: Live,
    recent: Vec<Pushed>,
    popup_seen: Option<String>,
    settings: Settings,
    app_front: bool,
    /// The compositor has video windows to give (its `vwin` topic is up and the mode is not `off`): the full view's stream is for one of them.
    vwin_on: bool,
    /// The full view took the front from an app: Back brings the app back.
    switched: bool,
    /// When the full view was opened (this camera's), and how long it took to show live video.
    opened_at: u64,
    first_live_ms: Option<u64>,
    /// How long after the opening Home was in front (the app switch) and the stream was asked for.
    home_ms: Option<u64>,
    asked_ms: Option<u64>,
    /// What the compositor made of the full view's claim on a video window (the shell sets it from the `vwin` topic).
    window: Window,
    screen: Screen,
    strip: Option<Strip>,
    sheet: Option<Sheet>,
    options: Option<Options>,
    toast: Option<(String, u64)>,
    planner: Planner,
    pending: Vec<(u64, Ask)>,
    /// The camera the helper was asked to play, and what the person is looking at.
    asked: Option<String>,
    expect: Expect,
    /// Where a recording was asked to start, until the helper says where its picture is.
    commit_hint: Option<i64>,
    /// A recording started from the live edge to hold it: paused as soon as it plays.
    pause_when_playing: bool,
    /// A playback is asked for and not playing yet: the picture area shows the cursor's thumbnail.
    thumb_pending: bool,
    ok_down: Option<u64>,
    ok_fired: bool,
    intents: Vec<Intent>,
    dropped: Vec<String>,
    now: u64,
    dirty: bool,
}

impl App {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            cams: Cameras::default(),
            live: Live { rate: 1.0, ..Live::default() },
            recent: Vec::new(),
            popup_seen: None,
            settings: Settings::default(),
            app_front: false,
            vwin_on: false,
            switched: false,
            opened_at: 0,
            first_live_ms: None,
            home_ms: None,
            asked_ms: None,
            window: Window::Unclaimed,
            screen: Screen::Closed,
            strip: None,
            sheet: None,
            options: None,
            toast: None,
            planner: Planner::new(),
            pending: Vec::new(),
            asked: None,
            expect: Expect::Live,
            commit_hint: None,
            pause_when_playing: false,
            thumb_pending: false,
            ok_down: None,
            ok_fired: false,
            intents: Vec::new(),
            dropped: Vec::new(),
            now: 0,
            dirty: false,
        }
    }

    // ------------------------------------------------------------------ what is known

    /// The viewer (grid, full view, strip, sheet, options) is up: the pop-up shows no card, and the layer wants every key.
    pub fn viewer_up(&self) -> bool {
        !matches!(self.screen, Screen::Closed)
    }

    pub fn cameras(&self) -> &Cameras {
        &self.cams
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn is_full(&self) -> bool {
        matches!(self.screen, Screen::Full(_))
    }

    /// The camera the full view shows.
    pub fn current(&self) -> Option<&data::Camera> {
        match &self.screen {
            Screen::Full(f) => self.cams.get(&f.camera),
            _ => None,
        }
    }

    fn current_index(&self) -> Option<usize> {
        match &self.screen {
            Screen::Full(f) => self.cams.index_of(&f.camera),
            _ => None,
        }
    }

    /// The bridge's clock now: this TV's plus the skew.
    fn bridge_now(&self) -> i64 {
        self.now as i64 + self.cams.skew_ms
    }

    /// Live video is offered: the link is ready, and over a playing app only where the theme says so.
    fn live_on(&self) -> bool {
        self.cams.state == data::LinkState::Ready && (!self.app_front || self.cfg.live_over_apps)
    }

    fn can_live(&self, camera: &data::Camera) -> bool {
        self.live_on() && camera.live
    }

    /// The live session plays the camera shown, in the mode the person asked for: the layer is a hole over the video plane.
    pub fn hole(&self) -> bool {
        let Some(c) = self.current() else { return false };
        let mode_ok = match self.expect {
            Expect::Live => self.live.mode == LiveMode::Live,
            Expect::Recording => self.live.is_recording(),
        };
        self.live.camera == c.id && self.live.state == LiveState::Playing && mode_ok && self.asked.as_deref() == Some(&c.id) && self.window != Window::Waiting
    }

    /// What the compositor made of the claim: the picture follows (a window not drawing yet is no hole; a refusal is a glyph). Returns whether it
    /// changed.
    pub fn set_window(&mut self, window: Window) -> bool {
        let changed = self.window != window;
        self.window = window;
        self.dirty |= changed && self.viewer_up();
        changed
    }

    /// Whether the compositor has video windows to give.
    pub fn set_vwin_on(&mut self, on: bool) {
        self.vwin_on = on;
    }

    /// Opening the full view over an app: its picture is a video window's, and the decoder is never started beside an app's video, so the view takes the
    /// front as an app of its own does (Home comes forward, the app goes to the background, Back brings it back). Only for a camera that plays live, where the
    /// compositor has a window to give and the owner has not asked for live over apps.
    fn takes_foreground(&self, camera: &str) -> bool {
        self.app_front && self.vwin_on && !self.cfg.live_over_apps && self.cams.state == data::LinkState::Ready && self.cams.get(camera).is_some_and(|c| c.live)
    }

    /// The window the full view's pictures are asked for in when the compositor's video windows carry them: the display's main one, with Home in front (the app
    /// LG's video path knows the video for). None where nothing is claimed (no compositor windows, an app in front without live over apps, a camera that does not
    /// play live): the stream is then as it was before there were any.
    fn window_for(&self, camera: &str) -> Option<&'static str> {
        (self.vwin_on && self.cams.get(camera).is_some_and(|c| self.can_live(c))).then_some("main")
    }

    /// The call for the full view's live stream: the camera's own at the owner's quality (native by default), up to 4K at its own rate.
    fn live_body(&self, camera: &str) -> Value {
        match self.window_for(camera) {
            Some(window) => json!({"camera": camera, "for": "full", "window": window}),
            None => json!({"camera": camera}),
        }
    }

    /// What the full view's live video asks of the display while it is wanted: the stream the helper was asked for and the rectangle it is
    /// fitted to on the screen. Whoever owns the spare hardware video windows (`compositor/.../vwin.rs`, kind `camera-full`) makes its claim
    /// from this, and lets it go when this is `None` (the view closed, another camera, an app in front without live over apps).
    pub fn live_claim(&self) -> Option<LiveClaim> {
        let Screen::Full(f) = &self.screen else { return None };
        let c = self.cams.get(&f.camera)?;
        if self.asked.as_deref() != Some(&c.id) || !self.can_live(c) {
            return None;
        }
        let (codec, width, height, fps) = if self.live.camera == c.id && self.live.width > 0 {
            (self.live.codec.clone(), self.live.width, self.live.height, self.live.fps)
        } else {
            let n = c.native.as_ref()?;
            (n.codec.clone(), n.width, n.height, n.fps)
        };
        Some(LiveClaim { camera: c.id.clone(), rect: fit(width, height), codec, width, height, fps })
    }

    fn layout(&self) -> Layout {
        let banner = !self.cams.list.is_empty() && self.cams.state != data::LinkState::Ready;
        grid::layout(self.cams.list.len(), grid::room(self.cfg.text_scale, banner))
    }

    fn tag(&mut self) -> u64 {
        self.planner.next_tag()
    }

    fn ask(&mut self, ask: Ask, method: &str, path: String, body: Option<Value>, fx: &mut Vec<Effect>) {
        let tag = self.tag();
        self.pending.push((tag, ask));
        if self.pending.len() > 64 {
            self.pending.remove(0);
        }
        fx.push(Effect::Request { tag, method: method.to_string(), path, body });
    }

    fn get(&mut self, ask: Ask, path: String, fx: &mut Vec<Effect>) {
        let tag = self.tag();
        self.pending.push((tag, ask));
        if self.pending.len() > 64 {
            self.pending.remove(0);
        }
        fx.push(Effect::Fetch { tag, path });
    }

    fn say(&mut self, words: Option<String>) {
        if let Some(w) = words {
            self.toast = Some((w, self.now + TOAST_MS));
            self.dirty = true;
        }
    }

    // ------------------------------------------------------------------ opening and closing

    fn reset_transient(&mut self) {
        self.toast = None;
        self.strip = None;
        self.sheet = None;
        self.options = None;
        self.commit_hint = None;
        self.pause_when_playing = false;
        self.thumb_pending = false;
        self.ok_down = None;
        self.ok_fired = false;
        self.expect = Expect::Live;
        self.dirty = true;
    }

    /// The grid of every camera (the quick panel's tile, the full view's OK, `ctl shell show cameras`).
    pub fn open_grid(&mut self, now: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.now = now;
        let focus_on = self.current_index().unwrap_or(0);
        self.stop_live(&mut fx);
        let opening = !self.viewer_up();
        self.reset_transient();
        let layout = self.layout();
        let focus = if self.cams.list.is_empty() { GFocus::Settings } else { GFocus::Tile(focus_on.min(self.cams.list.len() - 1)) };
        let top = match focus {
            GFocus::Tile(i) => layout.top_for(0, i),
            _ => 0,
        };
        self.screen = Screen::Grid(GridState { focus, top, back_to: focus_on.min(self.cams.list.len().saturating_sub(1)) });
        if opening {
            self.request_settings(&mut fx);
        }
        self.finish(now, &mut fx);
        fx
    }

    /// The full view of one camera (the pop-up's OK, a tile's OK); the grid when there is no such camera.
    pub fn open_full(&mut self, camera: &str, now: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.now = now;
        let opening = !self.viewer_up();
        self.enter_full(camera, true, &mut fx);
        if opening {
            self.request_settings(&mut fx);
        }
        self.finish(now, &mut fx);
        fx
    }

    /// `Events` of the grid: the recent events of every camera as a sheet over the grid.
    pub fn open_events(&mut self, now: u64) -> Vec<Effect> {
        let mut fx = self.open_grid(now);
        self.sheet = Some(Sheet::recent(&self.recent));
        self.dirty = true;
        self.finish(now, &mut fx);
        fx
    }

    fn enter_full(&mut self, camera: &str, ask_live: bool, fx: &mut Vec<Effect>) {
        let id = match self.cams.get(camera) {
            Some(c) => c.id.clone(),
            None => match self.cams.list.first() {
                Some(c) => c.id.clone(),
                None => {
                    // no camera at all: the grid says why
                    self.reset_transient();
                    self.screen = Screen::Grid(GridState { focus: GFocus::Settings, top: 0, back_to: 0 });
                    return;
                }
            },
        };
        self.reset_transient();
        let takes = self.takes_foreground(&id);
        self.screen = Screen::Full(FullState { camera: id, chrome: true, chrome_until: self.now + CHROME_MS });
        if takes {
            self.switched = true;
            self.intents.push(Intent::TakeForeground);
        }
        self.opened_at = self.now;
        self.first_live_ms = None;
        self.home_ms = None;
        self.asked_ms = None;
        if ask_live {
            self.ensure_live(true, fx);
        } else {
            self.asked = self.current().map(|c| c.id.clone());
        }
    }

    /// Closes the viewer: back to where it was, in one frame; the live session is stopped off the UI path.
    pub fn close(&mut self, now: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.now = now;
        self.close_inner(&mut fx);
        self.finish(now, &mut fx);
        fx
    }

    fn close_inner(&mut self, fx: &mut Vec<Effect>) {
        if !self.viewer_up() {
            return;
        }
        self.stop_live(fx);
        self.screen = Screen::Closed;
        self.switched = false;
        self.reset_transient();
        self.planner.quiesce();
        self.dropped.push("bubble:".into());
        self.pending.clear();
    }

    fn request_settings(&mut self, fx: &mut Vec<Effect>) {
        self.get(Ask::Settings, options::settings_path(), fx);
    }

    // ------------------------------------------------------------------ the live session

    /// Asks the helper for the camera shown when live is offered for it and it is not the one asked for (or `force`); stops what was asked
    /// when live is no longer offered.
    fn ensure_live(&mut self, force: bool, fx: &mut Vec<Effect>) {
        let want = self.current().filter(|c| self.can_live(c)).map(|c| c.id.clone());
        match (want, self.asked.clone()) {
            (Some(id), asked) if force || asked.as_deref() != Some(&id) => {
                self.asked = Some(id.clone());
                self.asked_ms.get_or_insert(self.now.saturating_sub(self.opened_at));
                self.expect = Expect::Live;
                self.commit_hint = None;
                self.thumb_pending = false;
                let restore = Expect::Live;
                self.ask(Ask::Live { restore }, "POST", "/v1/cameras/live".into(), Some(self.live_body(&id)), fx);
            }
            (None, Some(_)) => self.stop_live(fx),
            _ => {}
        }
    }

    /// Tears the session down off the UI path (`live/stop`).
    fn stop_live(&mut self, fx: &mut Vec<Effect>) {
        if self.asked.take().is_some() {
            fx.push(Effect::Post { path: "/v1/cameras/live/stop".into(), body: json!({}) });
        }
        self.expect = Expect::Live;
        self.commit_hint = None;
        self.thumb_pending = false;
        self.pause_when_playing = false;
    }

    fn go_live(&mut self, fx: &mut Vec<Effect>) {
        if let Some(s) = &mut self.strip {
            s.live();
        }
        self.dropped.push("bubble:".into());
        self.ensure_live(true, fx);
    }

    /// The playhead of a recording: the helper's position, else where it was asked to start.
    fn picture_at(&self) -> Option<i64> {
        if self.expect != Expect::Recording {
            return None;
        }
        self.live.position_ms.filter(|_| self.live.is_recording()).or(self.commit_hint)
    }

    fn strip_ctx(&self) -> Ctx {
        let recording = self.expect == Expect::Recording;
        Ctx { now: self.bridge_now(), picture_at: self.picture_at(), recording, paused: recording && self.live.paused }
    }

    fn commit_playback(&mut self, at: i64, rate: Option<f32>, fx: &mut Vec<Effect>) {
        let Some(camera) = self.current().map(|c| c.id.clone()) else { return };
        let rate = rate.unwrap_or_else(|| self.strip.as_ref().map_or(1.0, Strip::rate_value));
        let mut body = json!({"camera": camera, "at": at});
        if let Some(window) = self.window_for(&camera) {
            // a recording plays in the claim's window too: a seek reuses the pipeline and the picture on the plane
            body["window"] = json!(window);
        }
        if (rate - 1.0).abs() > 0.01 {
            body["rate"] = json!(rate);
        }
        let restore = self.expect;
        self.ask(Ask::Playback { restore }, "POST", "/v1/cameras/playback".into(), Some(body), fx);
        self.expect = Expect::Recording;
        self.asked = Some(camera);
        self.commit_hint = Some(at);
        self.thumb_pending = true;
        if let Some(s) = &mut self.strip {
            s.committed();
        }
    }

    fn play_clip(&mut self, camera: &str, clip: &str, fx: &mut Vec<Effect>) {
        let restore = self.expect;
        let mut body = json!({"camera": camera, "clip": clip});
        if let Some(window) = self.window_for(camera) {
            body["window"] = json!(window);
        }
        self.ask(Ask::Playback { restore }, "POST", "/v1/cameras/playback".into(), Some(body), fx);
        self.expect = Expect::Recording;
        self.asked = Some(camera.to_string());
        self.commit_hint = None;
        self.thumb_pending = false;
    }

    fn toggle_pause(&mut self, fx: &mut Vec<Effect>) {
        if self.expect == Expect::Recording {
            let paused = !self.live.paused;
            self.ask(Ask::Pause, "POST", "/v1/cameras/live/pause".into(), Some(json!({"paused": paused})), fx);
        } else {
            // live cannot be held: a recording from the live edge, held at its first picture
            let at = self.bridge_now() - 2000;
            self.pause_when_playing = true;
            self.commit_playback(at, None, fx);
        }
    }

    // ------------------------------------------------------------------ topics and answers

    fn topic(&mut self, topic: &str, data: &Value, now: u64, fx: &mut Vec<Effect>) {
        match topic {
            "foreground" => {
                // the feeder's window (`<brand app id>.cameras`, helper/app/src/scrypted/mod.rs) comes to the front while a stream plays: it is
                // the picture, not an app in front of it, and the app the stream plays over is still the one that was
                if data.get("app").and_then(Value::as_str).is_some_and(|a| a.ends_with(".cameras")) {
                    return;
                }
                let front = data.get("home").and_then(Value::as_bool).is_some_and(|h| !h);
                if front != self.app_front {
                    self.app_front = front;
                    if !front && self.switched {
                        self.home_ms.get_or_insert(now.saturating_sub(self.opened_at));
                    }
                    if self.is_full() {
                        self.ensure_live(false, fx);
                    }
                    self.dirty = true;
                }
            }
            "screen" => {
                if data.get("on").and_then(Value::as_bool) == Some(false) {
                    self.close_inner(fx);
                }
            }
            "homekey" => self.close_inner(fx),
            "show" if data.get("kind").and_then(Value::as_str) == Some("cameras") => {
                let opened = self.open_grid(now);
                fx.extend(opened);
            }
            _ => {}
        }
    }

    fn helper(&mut self, topic: &str, data: &Value, now: u64, fx: &mut Vec<Effect>) {
        match topic {
            "cameras" => {
                let was_ready = self.cams.state == data::LinkState::Ready;
                let was_n = self.cams.list.len();
                self.cams = Cameras::parse(data);
                self.cameras_changed(was_ready, was_n, fx);
            }
            "cameras.live" => {
                let before = self.live.clone();
                self.live = Live::parse(data);
                self.live_changed(&before, fx);
            }
            "cameras.events" => {
                self.recent = data::recent_cards(data);
                if let Some(s) = &mut self.sheet {
                    if s.source == sheet::Source::Recent {
                        s.set_recent(&self.recent);
                    }
                }
                self.dirty = self.viewer_up();
            }
            "cameras.popup" => self.popup_record(Popup::parse(data), now, fx),
            _ => {}
        }
    }

    fn cameras_changed(&mut self, was_ready: bool, was_n: usize, fx: &mut Vec<Effect>) {
        self.dirty = self.viewer_up();
        let n = self.cams.list.len();
        match self.screen.clone() {
            Screen::Grid(mut g) => {
                if n == 0 {
                    g.focus = GFocus::Settings;
                    g.top = 0;
                } else {
                    let layout = self.layout();
                    if let GFocus::Tile(i) = g.focus {
                        let i = i.min(n - 1);
                        g.focus = GFocus::Tile(i);
                        g.top = layout.top_for(g.top, i);
                    } else if g.focus == GFocus::Settings && was_n == 0 {
                        g.focus = GFocus::Tile(0);
                    }
                    g.back_to = g.back_to.min(n - 1);
                    g.top = g.top.min(layout.rows_total.saturating_sub(layout.rows_shown));
                }
                self.screen = Screen::Grid(g);
            }
            Screen::Full(f) => {
                if self.cams.get(&f.camera).is_none() {
                    // the camera is no longer shared: the first one, or the grid's words when there is none
                    let next = self.cams.list.first().map(|c| c.id.clone());
                    self.stop_live(fx);
                    match next {
                        Some(id) => self.enter_full(&id, true, fx),
                        None => {
                            self.reset_transient();
                            self.screen = Screen::Grid(GridState { focus: GFocus::Settings, top: 0, back_to: 0 });
                        }
                    }
                } else {
                    let ready = self.cams.state == data::LinkState::Ready;
                    if ready != was_ready || self.asked.is_none() {
                        self.ensure_live(false, fx);
                    }
                }
            }
            Screen::Closed => {}
        }
    }

    fn live_changed(&mut self, before: &Live, fx: &mut Vec<Effect>) {
        self.dirty = self.viewer_up();
        if !self.is_full() {
            return;
        }
        let Some(cur) = self.current().map(|c| c.id.clone()) else { return };
        if self.live.camera != cur {
            return;
        }
        // a recording that ends asks live again by itself
        if self.live.is_recording() && self.live.state == LiveState::Stopped && before.state != LiveState::Stopped && self.expect == Expect::Recording {
            self.go_live(fx);
            return;
        }
        // a refusal while a recording plays leaves nothing playing: the words, and live again by itself
        if self.expect == Expect::Recording && self.live.state == LiveState::Failed && before.state != LiveState::Failed {
            let reason = self.live.reason.clone();
            self.say(Some(reason).filter(|r| !r.trim().is_empty()));
            self.go_live(fx);
            return;
        }
        if self.pause_when_playing && self.live.is_recording() && self.live.state == LiveState::Playing {
            self.pause_when_playing = false;
            if !self.live.paused {
                self.ask(Ask::Pause, "POST", "/v1/cameras/live/pause".into(), Some(json!({"paused": true})), fx);
            }
        }
        if self.live.is_recording() && self.live.state == LiveState::Playing && self.expect == Expect::Recording {
            self.thumb_pending = false;
        }
    }

    /// A record of the pop-up topic: while the viewer is up there is no card, and the full view turns to the event's camera.
    fn popup_record(&mut self, record: Option<Popup>, now: u64, fx: &mut Vec<Effect>) {
        let Some(p) = record else { return };
        if self.popup_seen.as_deref() == Some(p.id.as_str()) {
            return;
        }
        self.popup_seen = Some(p.id.clone());
        if !self.viewer_up() || p.until_ms <= now as i64 {
            return;
        }
        if let Screen::Full(f) = &self.screen {
            if f.camera != p.camera && self.cams.get(&p.camera).is_some() {
                self.enter_full(&p.camera, true, fx);
            }
        }
    }

    fn fetched(&mut self, tag: u64, status: u16, data: &Value, now: u64, fx: &mut Vec<Effect>) {
        let Some(i) = self.pending.iter().position(|(t, _)| *t == tag) else {
            // a picture's answer (decoded by the host: `{"picture": {...}}`) or a refusal as the helper sent it
            if pics::owns(tag) {
                let ok = status == 200 && data.get("picture").is_some();
                if self.planner.arrived(tag, status, ok, now).is_some() {
                    self.dirty = self.viewer_up();
                }
            }
            return;
        };
        let (_, ask) = self.pending.remove(i);
        let refusal = |data: &Value| -> Option<String> {
            let e = data.get("error")?;
            words::refusal(e.get("code").and_then(Value::as_str).unwrap_or(""), e.get("message").and_then(Value::as_str).unwrap_or(""))
        };
        match ask {
            Ask::Settings => {
                if status == 200 {
                    self.settings = Settings::parse(data);
                }
            }
            Ask::Live { restore } | Ask::Playback { restore } => {
                if status != 200 && status != 202 {
                    // refused: the picture goes on as it was
                    self.expect = restore;
                    self.thumb_pending = false;
                    self.commit_hint = None;
                    self.pause_when_playing = false;
                    let words = refusal(data).or_else(|| (status == 0).then(|| words::link_words("offline").to_string()));
                    self.say(words);
                }
            }
            Ask::Pause => {
                if status == 409 {
                    self.say(refusal(data));
                }
            }
            Ask::Recordings { camera } => {
                if self.strip_camera().as_deref() == Some(&camera) {
                    if let Some(s) = &mut self.strip {
                        if status == 200 {
                            s.set_data(Some(Recordings::parse(data)), None);
                        } else {
                            s.set_failed();
                        }
                    }
                }
            }
            Ask::Ticks { camera } => {
                if self.strip_camera().as_deref() == Some(&camera) {
                    if let Some(s) = &mut self.strip {
                        if status == 200 {
                            s.set_data(None, Some(data::ticks(data)));
                        }
                    }
                }
            }
            Ask::SheetEvents { camera } => {
                if let Some(s) = &mut self.sheet {
                    if s.camera_id() == Some(camera.as_str()) {
                        s.set_events(&if status == 200 { data::ticks(data) } else { Vec::new() });
                    }
                }
            }
            Ask::SheetClips { camera } => {
                if let Some(s) = &mut self.sheet {
                    if s.camera_id() == Some(camera.as_str()) {
                        s.set_clips(&if status == 200 { data::clips(data) } else { Vec::new() });
                    }
                }
            }
            Ask::Notify { camera } => {
                if status == 200 {
                    let settings = Settings::parse(data.get("settings").unwrap_or(data));
                    if let Some(o) = self.options.as_mut().filter(|o| o.camera == camera) {
                        o.answered(&settings);
                    }
                    self.settings = settings;
                } else {
                    // the write was refused (the settings changed under it): read the page again
                    self.request_settings(fx);
                    if let Some(o) = self.options.as_mut().filter(|o| o.camera == camera) {
                        o.kinds = self.settings.kinds_for(&camera);
                    }
                }
            }
        }
        self.dirty = true;
    }

    fn strip_camera(&self) -> Option<String> {
        self.current().map(|c| c.id.clone())
    }

    // ------------------------------------------------------------------ keys

    fn key(&mut self, name: &str, down: bool, repeat: bool, now: u64, fx: &mut Vec<Effect>) {
        if !self.viewer_up() {
            return;
        }
        let dir = match name {
            "up" => Some(Dir::Up),
            "down" => Some(Dir::Down),
            "left" => Some(Dir::Left),
            "right" => Some(Dir::Right),
            _ => None,
        };
        match (name, dir) {
            (_, Some(d)) => {
                if down {
                    self.dpad(d, repeat, now, fx);
                } else {
                    self.dpad_release(d, fx);
                }
            }
            ("ok", _) => {
                if down {
                    if !repeat && self.ok_down.is_none() {
                        self.ok_down = Some(now);
                        self.ok_fired = false;
                    }
                } else if self.ok_down.take().is_some() && !self.ok_fired {
                    self.reveal_or(now, fx, |app, fx| app.ok(now, fx));
                }
            }
            ("back", _) if down && !repeat => self.back(now, fx),
            _ => {}
        }
    }

    /// The chrome of the full view comes with every key and goes 2.5 s later; the first key that finds it gone only reveals it.
    fn chrome_up(&self) -> bool {
        match &self.screen {
            Screen::Full(f) => f.chrome,
            _ => true,
        }
    }

    fn show_chrome(&mut self, now: u64) {
        if let Screen::Full(f) = &mut self.screen {
            f.chrome = true;
            f.chrome_until = now + CHROME_MS;
        }
        self.dirty = true;
    }

    /// Runs `act` when the chrome is up; when it is gone the key only brings it back.
    fn reveal_or(&mut self, now: u64, fx: &mut Vec<Effect>, act: impl FnOnce(&mut Self, &mut Vec<Effect>)) {
        if !self.chrome_up() {
            self.show_chrome(now);
            return;
        }
        self.show_chrome(now);
        act(self, fx);
    }

    fn dpad(&mut self, dir: Dir, repeat: bool, now: u64, fx: &mut Vec<Effect>) {
        self.reveal_or(now, fx, |app, fx| app.dpad_act(dir, repeat, now, fx));
    }

    fn dpad_act(&mut self, dir: Dir, repeat: bool, now: u64, fx: &mut Vec<Effect>) {
        if self.options.is_some() {
            if let Some(o) = &mut self.options {
                match dir {
                    Dir::Up => o.step(false),
                    Dir::Down => o.step(true),
                    _ => {}
                }
            }
            self.dirty = true;
            return;
        }
        if self.sheet.is_some() {
            let key = match dir {
                Dir::Up => sheet::Key::Up,
                Dir::Down => sheet::Key::Down,
                Dir::Left => sheet::Key::Left,
                Dir::Right => sheet::Key::Right,
            };
            let out = self.sheet.as_mut().map_or(sheet::Out::Nothing, |s| s.key(key));
            self.sheet_out(out, now, fx);
            return;
        }
        if self.strip.is_some() {
            let key = match dir {
                Dir::Up => strip::Key::Up,
                Dir::Down => strip::Key::Down,
                Dir::Left => strip::Key::Left,
                Dir::Right => strip::Key::Right,
            };
            let ctx = self.strip_ctx();
            let out = self.strip.as_mut().map_or(SOut::Nothing, |s| s.press(key, repeat, &ctx, now));
            self.strip_out(out, now, fx);
            return;
        }
        match self.screen.clone() {
            Screen::Grid(g) => self.grid_dpad(g, dir, fx),
            Screen::Full(_) => self.full_dpad(dir, now, fx),
            Screen::Closed => {}
        }
    }

    fn dpad_release(&mut self, dir: Dir, fx: &mut Vec<Effect>) {
        if !matches!(dir, Dir::Left | Dir::Right) || self.strip.is_none() {
            return;
        }
        let key = if dir == Dir::Left { strip::Key::Left } else { strip::Key::Right };
        let ctx = self.strip_ctx();
        let now = self.now;
        let out = self.strip.as_mut().map_or(SOut::Nothing, |s| s.release(key, &ctx));
        self.strip_out(out, now, fx);
    }

    fn grid_dpad(&mut self, mut g: GridState, dir: Dir, _fx: &mut Vec<Effect>) {
        let layout = self.layout();
        if self.cams.list.is_empty() {
            return;
        }
        let next = grid::step(&layout, g.focus, dir, g.back_to);
        if let GFocus::Tile(i) = next {
            g.back_to = i;
            g.top = layout.top_for(g.top, i);
        } else if let GFocus::Tile(i) = g.focus {
            g.back_to = i;
        }
        g.focus = next;
        self.screen = Screen::Grid(g);
        self.dirty = true;
    }

    fn full_dpad(&mut self, dir: Dir, _now: u64, fx: &mut Vec<Effect>) {
        let n = self.cams.list.len();
        let Some(i) = self.current_index() else { return };
        match dir {
            Dir::Left | Dir::Right if n > 1 => {
                let j = if dir == Dir::Left { (i + n - 1) % n } else { (i + 1) % n };
                let id = self.cams.list[j].id.clone();
                self.switch_to(&id, fx);
            }
            Dir::Up => {
                if let Some(c) = self.current().cloned().filter(|c| c.recordings && self.live_on()) {
                    self.strip = Some(Strip::new(c.since));
                    self.dirty = true;
                }
            }
            Dir::Down => {
                if let Some(c) = self.current().cloned().filter(|c| (c.events || c.clips) && self.live_on()) {
                    self.open_sheet_for(&c, fx);
                }
            }
            _ => {}
        }
    }

    /// Another camera in the full view: the session of the old one is replaced by the new one's.
    fn switch_to(&mut self, id: &str, fx: &mut Vec<Effect>) {
        self.enter_full(id, true, fx);
    }

    fn open_sheet_for(&mut self, c: &data::Camera, fx: &mut Vec<Effect>) {
        let now = self.bridge_now();
        self.sheet = Some(Sheet::camera(&c.id, &c.name));
        if c.events {
            self.get(Ask::SheetEvents { camera: c.id.clone() }, format!("/v1/cameras/{}/events?from={}&to={}&count={}", sheet::encode(&c.id), now - 24 * 3_600_000, now, sheet::MAX_EVENTS), fx);
        } else if let Some(s) = &mut self.sheet {
            s.set_events(&[]);
        }
        self.dirty = true;
    }

    fn ok(&mut self, now: u64, fx: &mut Vec<Effect>) {
        if self.options.is_some() {
            let row = self.options.as_ref().map_or(0, |o| o.focus);
            self.toggle_option(row, fx);
            return;
        }
        if self.sheet.is_some() {
            let out = self.sheet.as_mut().map_or(sheet::Out::Nothing, |s| s.key(sheet::Key::Ok));
            self.sheet_out(out, now, fx);
            return;
        }
        if self.strip.is_some() {
            let ctx = self.strip_ctx();
            let out = self.strip.as_mut().map_or(SOut::Nothing, |s| s.press(strip::Key::Ok, false, &ctx, now));
            self.strip_out(out, now, fx);
            return;
        }
        match self.screen.clone() {
            Screen::Grid(g) => match g.focus {
                GFocus::Tile(i) => {
                    if let Some(c) = self.cams.list.get(i).map(|c| c.id.clone()) {
                        self.enter_full(&c, true, fx);
                    }
                }
                GFocus::Events => {
                    self.sheet = Some(Sheet::recent(&self.recent));
                    self.dirty = true;
                }
                GFocus::Settings => self.open_settings(fx),
            },
            Screen::Full(_) => {
                if self.note_up() {
                    self.ensure_live(true, fx);
                } else if self.cams.list.len() > 1 {
                    let cur = self.current_index().unwrap_or(0);
                    let mut fxx = Vec::new();
                    self.stop_live(&mut fxx);
                    fx.extend(fxx);
                    let layout = self.layout();
                    self.screen = Screen::Grid(GridState { focus: GFocus::Tile(cur), top: layout.top_for(0, cur), back_to: cur });
                    self.reset_transient();
                }
            }
            Screen::Closed => {}
        }
    }

    fn open_settings(&mut self, fx: &mut Vec<Effect>) {
        self.intents.push(Intent::OpenSettings);
        self.close_inner(fx);
    }

    /// OK held on a tile of the grid: its pop-up options page.
    fn ok_hold(&mut self, now: u64, fx: &mut Vec<Effect>) {
        if let Screen::Grid(g) = self.screen.clone() {
            if self.sheet.is_none() && self.options.is_none() {
                if let GFocus::Tile(i) = g.focus {
                    if let Some(c) = self.cams.list.get(i) {
                        self.options = Options::new(&c.id, &c.name, &self.settings);
                        self.dirty = true;
                        if self.options.is_some() {
                            return;
                        }
                    }
                }
            }
        }
        self.reveal_or(now, fx, |app, fx| app.ok(now, fx));
    }

    fn toggle_option(&mut self, row: usize, fx: &mut Vec<Effect>) {
        let Some(o) = self.options.as_mut() else { return };
        let camera = o.camera.clone();
        if let Some(body) = o.toggle(row) {
            self.ask(Ask::Notify { camera }, "POST", options::settings_path(), Some(body), fx);
        }
        self.dirty = true;
    }

    fn back(&mut self, now: u64, fx: &mut Vec<Effect>) {
        // Back works while the chrome is hidden
        self.show_chrome(now);
        if self.options.is_some() {
            self.options = None;
            self.dirty = true;
            return;
        }
        if self.sheet.is_some() {
            self.sheet = None;
            self.dropped.push("card:".into());
            self.dirty = true;
            return;
        }
        if self.strip.is_some() {
            if self.expect == Expect::Recording {
                self.go_live(fx);
            } else {
                self.strip = None;
                self.dropped.push("bubble:".into());
            }
            self.dirty = true;
            return;
        }
        if self.is_full() && self.expect == Expect::Recording {
            self.go_live(fx);
            self.dirty = true;
            return;
        }
        let returns = self.switched;
        self.close_inner(fx);
        if returns {
            self.intents.push(Intent::ReturnToApp);
        }
    }

    fn note_up(&self) -> bool {
        self.is_full() && self.expect == Expect::Live && self.live.state == LiveState::Failed && self.current().is_some_and(|c| self.live.camera == c.id)
    }

    fn strip_out(&mut self, out: SOut, now: u64, fx: &mut Vec<Effect>) {
        self.dirty = true;
        match out {
            SOut::Nothing | SOut::Redraw => {}
            SOut::Close => {
                self.strip = None;
                self.dropped.push("bubble:".into());
            }
            SOut::OpenSheet => {
                self.strip = None;
                self.dropped.push("bubble:".into());
                if let Some(c) = self.current().cloned() {
                    self.open_sheet_for(&c, fx);
                }
            }
            SOut::Commit { at } => {
                let ctx = self.strip_ctx();
                if self.strip.as_ref().is_some_and(|s| s.in_gap(at, &ctx)) {
                    self.say(Some(words::NO_RECORDING.to_string()));
                    return;
                }
                self.commit_playback(at, None, fx);
            }
            SOut::GoLive => self.go_live(fx),
            SOut::TogglePause => self.toggle_pause(fx),
            SOut::Redial { rate } => {
                let at = self.picture_at().unwrap_or(self.bridge_now());
                self.commit_playback(at, Some(rate), fx);
            }
        }
        let _ = now;
    }

    fn sheet_out(&mut self, out: sheet::Out, now: u64, fx: &mut Vec<Effect>) {
        self.dirty = true;
        match out {
            sheet::Out::Nothing | sheet::Out::Redraw | sheet::Out::Close => {}
            sheet::Out::Tab(tab) => {
                let Some((camera, read)) = self.sheet.as_ref().and_then(|s| s.camera_id().map(|c| (c.to_string(), s.read()))) else { return };
                if tab == Tab::Clips && !read {
                    let now_b = self.bridge_now();
                    let has_clips = self.cams.get(&camera).is_some_and(|c| c.clips);
                    if has_clips {
                        self.get(Ask::SheetClips { camera: camera.clone() }, format!("/v1/cameras/{}/clips?from={}&to={}&count={}", sheet::encode(&camera), now_b - 7 * 24 * 3_600_000, now_b, sheet::MAX_EVENTS), fx);
                    } else if let Some(s) = &mut self.sheet {
                        s.set_clips(&[]);
                    }
                }
            }
            sheet::Out::Do(action) => {
                let from_grid = matches!(self.screen, Screen::Grid(_));
                self.sheet = None;
                self.dropped.push("card:".into());
                match action {
                    CardAction::Turn { camera } => self.enter_full(&camera, true, fx),
                    CardAction::Play { camera, at } => {
                        let same = self.current().is_some_and(|c| c.id == camera);
                        if from_grid || !same {
                            self.enter_full(&camera, false, fx);
                        }
                        self.commit_playback(at, Some(1.0), fx);
                    }
                    CardAction::Clip { camera, clip } => {
                        let same = self.current().is_some_and(|c| c.id == camera);
                        if from_grid || !same {
                            self.enter_full(&camera, false, fx);
                        }
                        self.play_clip(&camera, &clip, fx);
                    }
                }
                self.show_chrome(now);
            }
        }
    }

    /// A click (or a tap of OK on a pointer) on `target`: focuses it and does what OK does there.
    pub fn click(&mut self, target: Target, now: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.now = now;
        if !self.chrome_up() {
            self.show_chrome(now);
            self.finish(now, &mut fx);
            return fx;
        }
        self.show_chrome(now);
        match target {
            Target::Option(row) => {
                if let Some(o) = &mut self.options {
                    o.focus_row(row);
                }
                self.toggle_option(row, &mut fx);
            }
            Target::Tab(tab) => {
                let out = self.sheet.as_mut().map_or(sheet::Out::Nothing, |s| s.select(tab));
                self.sheet_out(out, now, &mut fx);
            }
            Target::Card(i) => {
                if let Some(s) = &mut self.sheet {
                    s.focus_card(i);
                }
                let out = self.sheet.as_mut().map_or(sheet::Out::Nothing, |s| s.key(sheet::Key::Ok));
                self.sheet_out(out, now, &mut fx);
            }
            Target::Strip(item) => {
                if let Some(s) = &mut self.strip {
                    s.focus = item;
                }
                let ctx = self.strip_ctx();
                let out = self.strip.as_mut().map_or(SOut::Nothing, |s| s.press(strip::Key::Ok, false, &ctx, now));
                self.strip_out(out, now, &mut fx);
            }
            Target::Bar(permille) => {
                let ctx = self.strip_ctx();
                if let Some(s) = &mut self.strip {
                    s.focus = SItem::Bar;
                    let w = s.window(ctx.now);
                    let at = strip::moment_at(permille as f32, w, 1000.0);
                    s.point(at, &ctx, now);
                    let out = s.press(strip::Key::Ok, false, &ctx, now);
                    self.strip_out(out, now, &mut fx);
                }
            }
            Target::Retry => self.ensure_live(true, &mut fx),
            Target::Backdrop => {}
            Target::Back => self.back(now, &mut fx),
            Target::Tile(i) => {
                if let Screen::Grid(g) = &mut self.screen {
                    g.focus = GFocus::Tile(i);
                    g.back_to = i;
                }
                self.ok(now, &mut fx);
            }
            Target::HeaderEvents => {
                if let Screen::Grid(g) = &mut self.screen {
                    g.focus = GFocus::Events;
                }
                self.ok(now, &mut fx);
            }
            Target::HeaderSettings | Target::EmptySettings => {
                if let Screen::Grid(g) = &mut self.screen {
                    g.focus = GFocus::Settings;
                }
                self.ok(now, &mut fx);
            }
        }
        self.finish(now, &mut fx);
        fx
    }

    // ------------------------------------------------------------------ time

    fn wants(&self) -> Vec<Want> {
        let mut out = Vec::new();
        match &self.screen {
            Screen::Closed => {}
            Screen::Grid(g) => {
                let layout = self.layout();
                let every = data::grid_interval_ms(&self.settings.video_grid);
                let every = if self.app_front { every.max(OVER_APP_TILE_MS) } else { every };
                if self.sheet.is_none() && self.options.is_none() {
                    for i in layout.shown(g.top) {
                        let c = &self.cams.list[i];
                        out.push(Want::repeating(&tile_key(&c.id), &format!("/v1/cameras/{}/frame?w={}&h={}", sheet::encode(&c.id), layout.tile_w, layout.tile_h), every));
                    }
                }
            }
            Screen::Full(f) => {
                if let Some(c) = self.cams.get(&f.camera) {
                    if self.wants_still(c) {
                        out.push(Want::repeating(&full_key(&c.id), &format!("/v1/cameras/{}/frame?w={FULL_W}&h={FULL_H}", sheet::encode(&c.id)), FULL_EVERY_MS));
                    }
                    if let Some(s) = &self.strip {
                        if s.bubble_ready(self.now) {
                            let ctx = self.strip_ctx();
                            let at = s.cursor(&ctx);
                            out.push(Want::repeating(&bubble_key(&c.id), &format!("/v1/cameras/{}/thumbnail?at={at}&w=256&h=144", sheet::encode(&c.id)), 0));
                        }
                    }
                    if self.thumb_pending {
                        if let Some(at) = self.commit_hint {
                            out.push(Want::repeating(&bubble_key(&c.id), &format!("/v1/cameras/{}/thumbnail?at={at}&w=640&h=360", sheet::encode(&c.id)), 0));
                        }
                    }
                }
            }
        }
        if let Some(s) = &self.sheet {
            for (i, path) in s.wanted() {
                out.push(Want::once(&s.pic_key(i), path));
            }
        }
        out
    }

    /// The full view draws the camera's stills while live is not playing it: not offered, starting from a stream that is not
    /// prebuffered, failing; a prebuffered start is black until it plays.
    fn wants_still(&self, c: &data::Camera) -> bool {
        if self.hole() {
            return false;
        }
        if !self.can_live(c) {
            return true;
        }
        if self.note_up() {
            return true;
        }
        !c.prebuffered
    }

    fn chrome_holds(&self) -> bool {
        if self.sheet.is_some() || self.options.is_some() || self.toast.is_some() || self.note_up() {
            return true;
        }
        let Some(c) = self.current() else { return true };
        if self.can_live(c) {
            // video plays: the chrome may go; a recording that is held, or nothing playing yet, keeps it
            !self.hole() || (self.expect == Expect::Recording && self.live.paused)
        } else {
            // stills: live while the newest is fresh
            !self.planner.pic(&full_key(&c.id)).last_at.is_some_and(|t| self.now.saturating_sub(t) < FRESH_STILL_MS)
        }
    }

    fn housekeeping(&mut self, now: u64, fx: &mut Vec<Effect>) {
        if !self.viewer_up() {
            return;
        }
        if self.toast.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.toast = None;
            self.dirty = true;
        }
        if let Some(down) = self.ok_down {
            if !self.ok_fired && now >= down + self.cfg.hold_ms {
                self.ok_fired = true;
                self.ok_hold(now, fx);
            }
        }
        // the chrome goes 2.5 s after the last key, where something plays
        let holds = self.chrome_holds();
        let mut hide = false;
        if let Screen::Full(f) = &mut self.screen {
            if f.chrome && holds {
                f.chrome_until = now + CHROME_MS;
            } else if f.chrome && now >= f.chrome_until {
                f.chrome = false;
                hide = true;
            } else if !f.chrome && holds {
                // where nothing plays (a failure, a held recording, a refusal's words) the chrome comes back by itself
                f.chrome = true;
                f.chrome_until = now + CHROME_MS;
                self.dirty = true;
            }
        }
        if hide {
            self.dirty = true;
            // the strip fades with the chrome in a live view and closes with it; in a playback it stays, hidden, and comes back with the key
            if self.expect == Expect::Live && self.strip.is_some() {
                self.strip = None;
                self.dropped.push("bubble:".into());
            }
        }
        // the strip's own time: a commit waiting for a release, and the window's data
        if self.strip.is_some() {
            let ctx = self.strip_ctx();
            let out = self.strip.as_mut().map_or(SOut::Nothing, |s| s.tick(&ctx, now));
            if out != SOut::Nothing {
                self.strip_out(out, now, fx);
            }
            let camera = self.strip_camera();
            if let (Some(camera), Some(window)) = (camera, self.strip.as_ref().and_then(|s| s.load_due(&ctx, now))) {
                if let Some(s) = &mut self.strip {
                    s.mark_loading(now);
                }
                let id = sheet::encode(&camera);
                self.get(Ask::Recordings { camera: camera.clone() }, format!("/v1/cameras/{id}/recordings?from={}&to={}", window.from, window.to), fx);
                self.get(Ask::Ticks { camera }, format!("/v1/cameras/{id}/events?from={}&to={}&count=200", window.from, window.to), fx);
            }
        }
        if self.is_full() {
            self.ensure_live(false, fx);
        }
        let wants = self.wants();
        fx.extend(self.planner.plan(&wants, now, FLIGHT));
    }

    fn finish(&mut self, now: u64, fx: &mut Vec<Effect>) {
        if self.first_live_ms.is_none() && self.is_full() && self.hole() {
            self.first_live_ms = Some(now.saturating_sub(self.opened_at));
        }
        self.housekeeping(now, fx);
        if self.dirty {
            self.dirty = false;
            fx.push(Effect::Redraw);
        }
    }

    // ------------------------------------------------------------------ the view

    fn hints(&self) -> Vec<HintView> {
        match &self.screen {
            Screen::Grid(g) => {
                let mut v = vec![HintView::new(&[HKey::Ok], Do::Kit(HAct::Expand), "OK, expand")];
                if matches!(g.focus, GFocus::Tile(i) if self.cams.list.get(i).is_some_and(|c| options::field_key(&c.id).is_some())) {
                    v.push(HintView { keys: vec![HKey::Ok], held: true, does: Do::Kit(HAct::Options), name: "Hold OK, options".into() });
                }
                v.push(HintView::new(&[HKey::Back], Do::Kit(HAct::Close), "Back, close"));
                v
            }
            Screen::Full(_) => {
                let Some(c) = self.current() else { return Vec::new() };
                let mut v = Vec::new();
                if self.cams.list.len() > 1 {
                    v.push(HintView::new(&[HKey::Left, HKey::Right], Do::Icon("video"), "Left and Right, next camera"));
                }
                if c.recordings && self.live_on() {
                    v.push(HintView::new(&[HKey::Up], Do::Icon("history"), "Up, timeline"));
                }
                if (c.events || c.clips) && self.live_on() {
                    v.push(HintView::new(&[HKey::Down], Do::Icon("list-video"), "Down, events and clips"));
                }
                if self.cams.list.len() > 1 {
                    v.push(HintView::new(&[HKey::Ok], Do::Icon("layout-grid"), "OK, all cameras"));
                }
                v.push(HintView::new(&[HKey::Back], Do::Kit(HAct::Close), "Back, close"));
                v
            }
            Screen::Closed => Vec::new(),
        }
    }

    fn grid_view(&self, g: &GridState) -> GridView {
        let layout = self.layout();
        let n = self.cams.list.len();
        let link = self.cams.state;
        let tiles: Vec<TileView> = self
            .cams
            .list
            .iter()
            .map(|c| {
                let w = grid::tile_words(c.online, link, self.planner.pic(&tile_key(&c.id)), self.now);
                TileView { id: c.id.clone(), name: c.name.clone(), pic: tile_key(&c.id), words: w.lines, dim: w.dim }
            })
            .collect();
        let empty = (n == 0).then(|| match link {
            data::LinkState::Ready => Empty { icon: "video", title: words::NO_CAMERAS.to_string() },
            data::LinkState::Refused => Empty { icon: "shield-alert", title: words::link_words("refused").to_string() },
            data::LinkState::Untrusted => Empty { icon: "shield-alert", title: words::link_words("untrusted").to_string() },
            data::LinkState::Unlinked => Empty { icon: "unplug", title: words::link_words("unlinked").to_string() },
            data::LinkState::Connecting => Empty { icon: "unplug", title: words::link_words("connecting").to_string() },
            data::LinkState::Offline => Empty { icon: "unplug", title: words::link_words("offline").to_string() },
        });
        GridView {
            count_words: if n == 0 { String::new() } else { words::count(n) },
            banner: (n > 0 && link != data::LinkState::Ready).then(|| words::link_words(link.name()).to_string()).filter(|w| !w.is_empty()),
            empty,
            tiles,
            top: g.top,
            below: layout.below(g.top).map(|(a, b)| words::range_below(a, b)),
            above: layout.above(g.top).map(|(a, b)| words::range_below(a, b)),
            layout,
            focus: g.focus,
            hints: self.hints(),
        }
    }

    fn state_word(&self, c: &data::Camera) -> StateWord {
        if self.note_up() {
            return StateWord { word: words::CANT_PLAY, dot: false };
        }
        if self.hole() {
            return if self.expect == Expect::Live { StateWord { word: words::LIVE, dot: true } } else { StateWord { word: words::RECORDED, dot: false } };
        }
        if self.can_live(c) {
            return StateWord { word: words::STARTING, dot: false };
        }
        let fresh = self.planner.pic(&full_key(&c.id)).last_at.is_some_and(|t| self.now.saturating_sub(t) < FRESH_STILL_MS * 2);
        if fresh { StateWord { word: words::LIVE, dot: true } } else { StateWord { word: words::STARTING, dot: false } }
    }

    fn full_view(&self, f: &FullState) -> Option<FullView> {
        let c = self.cams.get(&f.camera)?;
        let i = self.cams.index_of(&f.camera).unwrap_or(0);
        let n = self.cams.list.len();
        let hole = self.hole();
        let note = self.note_up().then(|| Note {
            title: words::FAILED_TITLE,
            detail: if self.live.reason.trim().is_empty() { "The camera did not start.".to_string() } else { self.live.reason.trim().to_string() },
            button: words::TRY_AGAIN,
        });
        let starting = self.can_live(c) && !hole && note.is_none();
        let info = if self.live.camera == c.id && self.live.state == LiveState::Playing && self.asked.as_deref() == Some(&c.id) {
            let s = words::stream_words(self.live.width, self.live.height, &self.live.codec, self.live.fps, self.live.rate);
            (!s.is_empty()).then_some(s)
        } else if self.can_live(c) && self.asked.as_deref() == Some(&c.id) {
            c.native.as_ref().and_then(|nat| {
                let s = words::stream_words(nat.width, nat.height, &nat.codec, nat.fps, 1.0);
                (!s.is_empty()).then_some(s)
            })
        } else {
            None
        };
        let thumb = self.thumb_pending && self.commit_hint.is_some() && self.planner.pic(&bubble_key(&c.id)).last_at.is_some();
        let backdrop = if hole {
            Backdrop::None
        } else if thumb {
            Backdrop::Thumb(bubble_key(&c.id))
        } else if self.wants_still(c) {
            Backdrop::Still(full_key(&c.id))
        } else {
            Backdrop::None
        };
        let veil = if note.is_some() {
            0.62
        } else if starting && matches!(backdrop, Backdrop::Still(_)) {
            0.35
        } else {
            0.0
        };
        let overlays = self.strip.is_some() || self.sheet.is_some() || self.options.is_some();
        let note_up = note.is_some();
        Some(FullView {
            name: c.name.clone(),
            position: (n > 1).then(|| words::of(i + 1, n)),
            state: self.state_word(c),
            info,
            line: (starting && !c.prebuffered).then(|| words::OPENS_SOON.to_string()),
            note,
            hole,
            backdrop,
            veil,
            chrome: f.chrome,
            button_focused: note_up,
            window_cue: self.window == Window::Refused && self.live_claim().is_some(),
            hints: if overlays || note_up { Vec::new() } else { self.hints() },
        })
    }

    fn strip_view(&self, s: &Strip) -> StripView {
        let ctx = self.strip_ctx();
        let clock = &self.cfg.clock;
        let cursor = s.cursor(&ctx);
        let live = self.expect == Expect::Live && !s.scrubbing();
        let on_picture = !s.scrubbing() && ctx.picture_at.is_some();
        let when = if live {
            words::LIVE.to_string()
        } else if on_picture {
            clock.clock_seconds(cursor)
        } else {
            clock.clock(cursor)
        };
        let window = s.window(ctx.now);
        let bubble = s.scrubbing().then(|| {
            let step = (i64::from(s.span_minutes()) * 60_000 / 60).max(5000);
            let near = s.ticks.iter().filter(|t| (t.at - cursor).abs() <= step).min_by_key(|t| (t.at - cursor).abs());
            let camera = self.strip_camera().unwrap_or_default();
            BubbleView {
                at: cursor,
                time: clock.clock(cursor),
                kind_word: near.map(|t| words::what(&t.kind, &t.label)),
                pic: self.planner.pic(&bubble_key(&camera)).last_at.map(|_| bubble_key(&camera)),
            }
        });
        StripView {
            when,
            rel: words::behind(ctx.now - cursor),
            span_word: words::span_word(s.span_minutes()),
            rate_word: words::rate_word(s.rate_value()),
            window,
            bar: s.bar(&ctx),
            axis: strip::axis(window, s.span_minutes(), clock),
            cursor,
            live,
            play: ctx.recording && ctx.paused,
            focus: s.focus,
            bubble,
            can_prev: strip::event_from(&s.ticks, cursor, false).is_some(),
            can_next: strip::event_from(&s.ticks, cursor, true).is_some(),
        }
    }

    /// Whether a picture of this key is in the host's store is the surface's to know; the sheet asks through `has_pic`.
    pub fn view_with(&self, has_pic: &dyn Fn(&str) -> bool) -> View {
        let base = match &self.screen {
            Screen::Closed => Base::Closed,
            Screen::Grid(g) => Base::Grid(self.grid_view(g)),
            Screen::Full(f) => self.full_view(f).map_or(Base::Closed, Base::Full),
        };
        let chrome = self.chrome_up();
        View {
            strip: if chrome { self.strip.as_ref().map(|s| self.strip_view(s)) } else { None },
            sheet: self.sheet.as_ref().map(|s| s.view(&self.cfg.clock, self.bridge_now(), has_pic)),
            options: self.options.as_ref().map(Options::view),
            toast: self.toast.as_ref().map(|(w, _)| w.clone()),
            over_app: self.app_front,
            base,
        }
    }

    /// The intents the app raised for the shell since the last call.
    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    /// The key the picture ask with `tag` is for (the host stores the decoded picture under it).
    pub fn picture_key(&self, tag: u64) -> Option<String> {
        self.planner.key_of(tag).map(str::to_string)
    }

    /// The picture keys (or key prefixes ending in `:`) the host's store may let go of.
    pub fn take_dropped(&mut self) -> Vec<String> {
        let mut d = std::mem::take(&mut self.dropped);
        d.sort();
        d.dedup();
        for prefix in d.iter().filter(|k| k.ends_with(':')) {
            self.planner.forget_prefix(prefix);
        }
        d
    }

    /// The model's own account, for the gate and the host harness (`--eval cameras`).
    pub fn debug(&self) -> Value {
        let screen = match &self.screen {
            Screen::Closed => json!({"mode": ""}),
            Screen::Grid(g) => json!({"mode": "grid", "focus": format!("{:?}", g.focus), "top": g.top}),
            Screen::Full(f) => json!({"mode": "full", "camera": f.camera, "current": self.current_index(), "chrome": f.chrome}),
        };
        json!({
            "screen": screen,
            "link": self.cams.state.name(),
            "cameras": self.cams.list.len(),
            "asked": self.asked,
            "expect": format!("{:?}", self.expect),
            "hole": self.hole(),
            "liveClaim": self.live_claim().map(|c| c.request()),
            "window": format!("{:?}", self.window),
            "strip": self.strip.is_some(),
            "sheet": self.sheet.is_some(),
            "options": self.options.is_some(),
            "toast": self.toast.as_ref().map(|(w, _)| w.clone()),
            "inFlight": self.planner.in_flight(),
            "appFront": self.app_front,
            "switched": self.switched,
            "vwinOn": self.vwin_on,
            "openedAt": self.opened_at,
            "firstLiveMs": self.first_live_ms,
            "homeMs": self.home_ms,
            "askedMs": self.asked_ms,
        })
    }
}


pub fn tile_key(camera: &str) -> String {
    format!("tile:{camera}")
}

pub fn full_key(camera: &str) -> String {
    format!("full:{camera}")
}

pub fn bubble_key(camera: &str) -> String {
    format!("bubble:{camera}")
}

impl Model for App {
    type View = View;

    fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.now = now;
        match input {
            Input::Topic { topic, data } => self.topic(topic, data, now, &mut fx),
            Input::Helper { topic, data } => self.helper(topic, data, now, &mut fx),
            Input::Fetched { tag, status, data } => self.fetched(*tag, *status, data, now, &mut fx),
            Input::Key { name, down, repeat } => self.key(name, *down, *repeat, now, &mut fx),
            Input::Tick => {}
            _ => {}
        }
        self.finish(now, &mut fx);
        fx
    }

    fn view(&self) -> View {
        self.view_with(&|_| false)
    }

    fn due_at(&self) -> Option<u64> {
        if !self.viewer_up() {
            return None;
        }
        let mut due: Vec<u64> = Vec::new();
        if let Some((_, until)) = &self.toast {
            due.push(*until);
        }
        if let Some(down) = self.ok_down.filter(|_| !self.ok_fired) {
            due.push(down + self.cfg.hold_ms);
        }
        if let Screen::Full(f) = &self.screen {
            if f.chrome {
                due.push(f.chrome_until);
            }
        }
        if let Some(s) = &self.strip {
            if let Some(d) = s.due_at(self.now) {
                due.push(d);
            }
        }
        if let Some(d) = self.planner.due_at(&self.wants()) {
            due.push(d);
        }
        due.into_iter().min().map(|d| d.max(self.now + 1))
    }
}
