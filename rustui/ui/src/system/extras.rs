//! What the QML Home did besides its own screens, as models (`panels/Splash.qml`, `panels/QuickAccess.qml`, `panels/ScreenRemote.qml`,
//! `panels/VoiceBar.qml`, `MiniFrame.qml` and the parts of `LumaHome.qml` that drove them). They belong to the Rust `home` kind: with it on the QML
//! Home is not in the stack, so these are Home's too, and `ctl shell rust -home` gives all of them back to the QML together.
//!
//! The surfaces that draw them are in [`super::extras_surface`], the system layer feeds them ([`super::layer::Layer`]) and Home tells them what it did
//! ([`Note`]: `home/model.rs` pushes them, the shell hands them over after every input). The compositor's key names for the mic, on-screen remote and
//! Search keys are taught in `keys.conf` (`voice N`, `screenremote N`, `search N`) and the shell asks for them in its report's `takes`.
//!
//! - the **launch screen**: the icon, the app's name and a ring while an app opens. With the compositor's transitions on, the card grows out of the
//!   tile, the screen waits 400 ms (an app that has a window by then never shows it) and tells the compositor where its icon is, which is where
//!   the card grows from then; without them it is the cut it always was;
//! - **Quick Access**: a toast for the digit that opened something, and the question for a digit with nothing behind it (four apps no digit opens
//!   yet, More, Not now). The choice is Home's to keep, so the question only says what was picked ([`Intent`]);
//! - the **on-screen remote**: the keys the micom service presses. It has no OK, no Back and no arrow, so nothing on it can answer a prompt that
//!   belongs to the physical remote (Principle 0: [`REMOTE_KEYS`] is the whole list and a test holds it to that);
//! - the **voice bar**: hold the mic key and speak, the helper listens (`POST /v1/voice/listen` and `/stop`) and Home Assistant's Assist turns
//!   the speech into text. The words are typed into Search and nothing acts on them;
//! - the **mini frame**: a news item opens in a pane that grows out of its card, with the web half (`<app>.frame`) painted into its interior.

use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

pub const ORIGIN_TAG: u64 = 0x5359_0400;
pub const MICOM_TAG: u64 = 0x5359_0401;
pub const FRAME_TAG: u64 = 0x5359_0402;
pub const RUNNING_TAG: u64 = 0x5359_0403;
/// The voice requests' tags: the base plus the number of the press, so a late answer to an old press is told apart.
pub const VOICE_TAG: u64 = 0x5359_0500;

/// The web half of the mini apps (`frame/appinfo.json`: `{{appId}}.frame`).
pub const FRAME_APP: &str = "org.nphil.lumatv.frame";
const APP_MGR: &str = "luna://com.webos.applicationManager";
const SURFACE_MGR: &str = "luna://com.webos.surfacemanager";
const MICOM: &str = "luna://com.webos.service.micomservice";

const SPLASH_WAIT_MS: u64 = 400;
const SPLASH_END_MS: u64 = 8000;
const SPLASH_NOTE_MS: u64 = 50;
const TOAST_MS: u64 = 2400;
const VOICE_CLEAR_MS: u64 = 2600;
const FRAME_LAND_MS: u64 = 260;
const FRAME_GRACE_MS: u64 = 350;
/// The ring and the loading mark turn a step this often.
pub const SPIN_MS: u64 = 75;

/// A rectangle on the screen with a corner radius: where a tile is, where a launch screen's icon is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
}

/// An app a digit may be given.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub id: String,
    pub title: String,
    pub icon: String,
}

/// What a mini frame shows: a page of the web.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub subtitle: String,
    pub source: String,
    pub ago: String,
}

/// What Home tells the extras (the shell hands it over after every input): an app is opening, a digit did or asked for something, the remote key
/// was pressed, a card opened a page.
#[derive(Clone, Debug, PartialEq)]
pub enum Note {
    Launch { id: String, title: String, icon: String, tile: Option<Tile> },
    Toast { digit: String, title: String, line: String },
    Ask { digit: String, picks: Vec<Pick> },
    Remote,
    Frame { page: Page, from: [f32; 4], to: [f32; 4] },
}

/// What a click of the pointer landed on: a card of the question (the apps, then More), its Not now button, a key of the on-screen remote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    Pick(usize),
    Later,
    Key(usize),
}

/// What the extras ask Home for: it keeps the choice and opens the app, or takes the person to Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    Assign { digit: String, app: String },
    More(String),
}

// ---- the launch screen
#[derive(Default)]
struct Splash {
    shown: Option<(String, String)>,
    pending: Option<(String, String, u64)>,
    id: String,
    end_at: Option<u64>,
    note_at: Option<u64>,
    rect: Option<Tile>,
}

impl Splash {
    fn origin(id: &str, tile: Option<Tile>, from: Option<Tile>) -> Effect {
        let rect = |t: Tile| json!({"x": t.x, "y": t.y, "width": t.w, "height": t.h, "radius": t.radius});
        let mut payload = json!({"appId": id, "from": from.map(rect)});
        if let Some(t) = tile {
            payload["tile"] = rect(t);
        }
        Effect::Luna { tag: ORIGIN_TAG, uri: format!("{SURFACE_MGR}/setLaunchOrigin"), payload, subscribe: false }
    }

    /// `app` is the id, the name and the icon's file of what opens.
    fn launch(&mut self, app: (&str, &str, &str), tile: Option<Tile>, smooth: bool, now: u64, out: &mut Vec<Effect>) {
        let (id, title, icon) = app;
        self.done();
        self.id = id.to_owned();
        self.end_at = Some(now + SPLASH_END_MS);
        if smooth {
            if let Some(t) = tile {
                out.push(Self::origin(id, Some(t), None));
            }
            self.pending = Some((title.to_owned(), icon.to_owned(), now + SPLASH_WAIT_MS));
        } else {
            self.shown = Some((title.to_owned(), icon.to_owned()));
        }
    }

    fn done(&mut self) {
        *self = Self::default();
    }

    fn tick(&mut self, now: u64, smooth: bool, out: &mut Vec<Effect>) -> bool {
        let mut changed = false;
        if let Some((title, icon, at)) = self.pending.clone()
            && now >= at
        {
            self.pending = None;
            self.shown = Some((title, icon));
            self.note_at = smooth.then_some(now + SPLASH_NOTE_MS);
            changed = true;
        }
        if let Some(at) = self.note_at
            && now >= at
            && self.shown.is_some()
        {
            // one frame after the screen shows, its icon has its place; the surface tells where
            match self.rect {
                Some(r) => {
                    out.push(Self::origin(&self.id, None, Some(r)));
                    self.note_at = None;
                }
                None => self.note_at = Some(now + 30),
            }
        }
        if self.end_at.is_some_and(|t| now >= t) {
            self.done();
            changed = true;
        }
        changed
    }

    fn due_at(&self) -> Option<u64> {
        [self.pending.as_ref().map(|p| p.2), self.note_at, self.end_at].into_iter().flatten().min()
    }
}

/// What the launch screen shows.
#[derive(Clone, Debug, PartialEq)]
pub struct SplashView {
    pub name: String,
    pub icon: String,
    /// The screen fades in (it came after a wait the transitions made) rather than being there at once.
    pub fade: bool,
}

// ---- Quick Access
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub digit: String,
    pub title: String,
    pub line: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub digit: String,
    pub picks: Vec<Pick>,
    pub index: usize,
}

impl Ask {
    /// 0..picks.len() the apps, then More, then Not now.
    pub fn more(&self) -> usize {
        self.picks.len()
    }

    pub fn later(&self) -> usize {
        self.picks.len() + 1
    }
}

#[derive(Default)]
struct Access {
    toast: Option<(Toast, u64)>,
    ask: Option<Ask>,
}

// ---- the on-screen remote
/// What a key of the remote shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Icon(&'static str),
    Rewind,
    Text(&'static str),
    Colour(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteKey {
    pub name: &'static str,
    pub face: Face,
    /// 0 the transport keys, 1 the number pad, 2 the colours.
    pub group: u8,
}

const fn key(name: &'static str, face: Face, group: u8) -> RemoteKey {
    RemoteKey { name, face, group }
}

/// The keys of the on-screen remote, in the order the focus walks them. Its keycodes are LG's published IR command numbers for its remotes
/// (`docs/research/home-launcher-replacements.md`) [INFERENCE: not yet run on this set]; a key this table lacks is not sent.
pub const REMOTE_KEYS: [RemoteKey; 21] = [
    key("rewind", Face::Rewind, 0),
    key("play", Face::Icon("play"), 0),
    key("pause", Face::Icon("pause"), 0),
    key("stop", Face::Icon("square"), 0),
    key("forward", Face::Icon("fast-forward"), 0),
    key("1", Face::Text("1"), 1),
    key("2", Face::Text("2"), 1),
    key("3", Face::Text("3"), 1),
    key("4", Face::Text("4"), 1),
    key("5", Face::Text("5"), 1),
    key("6", Face::Text("6"), 1),
    key("7", Face::Text("7"), 1),
    key("8", Face::Text("8"), 1),
    key("9", Face::Text("9"), 1),
    key("info", Face::Text("Info"), 1),
    key("0", Face::Text("0"), 1),
    key("back", Face::Icon("undo-2"), 1),
    key("red", Face::Colour("#EB6F92"), 2),
    key("green", Face::Colour("#9CCFD8"), 2),
    key("yellow", Face::Colour("#F6C177"), 2),
    key("blue", Face::Colour("#7AA2F7"), 2),
];

/// The micom service's keycode for a key of the remote; `back` closes the panel and has none.
pub fn remote_code(name: &str) -> Option<u32> {
    Some(match name {
        "0" => 16,
        "1" => 17,
        "2" => 18,
        "3" => 19,
        "4" => 20,
        "5" => 21,
        "6" => 22,
        "7" => 23,
        "8" => 24,
        "9" => 25,
        "red" => 114,
        "green" => 113,
        "yellow" => 99,
        "blue" => 97,
        "play" => 176,
        "pause" => 186,
        "stop" => 177,
        "rewind" => 143,
        "forward" => 142,
        "info" => 170,
        _ => return None,
    })
}

/// Where key `i` is in the body of the sheet: the transport keys and the colours are pills, the number pad's are smaller.
pub fn remote_box(i: usize) -> [f32; 4] {
    let k = REMOTE_KEYS[i];
    match k.group {
        0 => [i as f32 * 128.0, 40.0, 112.0, 88.0],
        1 => {
            let slot = i - 5;
            [720.0 + (slot % 3) as f32 * 128.0, (slot / 3) as f32 * 62.0, 112.0, 54.0]
        }
        _ => [1200.0 + (i - 17) as f32 * 130.0, 40.0, 120.0, 88.0],
    }
}

/// The key a step of the D-pad goes to: the nearest in that direction, one that is level with this one before one that is not.
pub fn remote_next(from: usize, dir: &str) -> Option<usize> {
    let a = remote_box(from);
    let (ax, ay) = (a[0] + a[2] / 2.0, a[1] + a[3] / 2.0);
    let vertical = dir == "up" || dir == "down";
    let (mut best, mut best_score, mut aligned, mut aligned_score) = (None, f32::MAX, None, f32::MAX);
    for i in (0..REMOTE_KEYS.len()).filter(|i| *i != from) {
        let b = remote_box(i);
        let (dx, dy) = (b[0] + b[2] / 2.0 - ax, b[1] + b[3] / 2.0 - ay);
        let ahead = match dir {
            "up" => dy < -4.0,
            "down" => dy > 4.0,
            "left" => dx < -4.0,
            "right" => dx > 4.0,
            _ => false,
        };
        if !ahead {
            continue;
        }
        let gap = if vertical { (dx.abs() - (a[2] + b[2]) / 2.0).max(0.0) } else { (dy.abs() - (a[3] + b[3]) / 2.0).max(0.0) };
        let primary = if vertical { dy.abs() } else { dx.abs() };
        if primary + 3.0 * gap < best_score {
            best = Some(i);
            best_score = primary + 3.0 * gap;
        }
        if gap == 0.0 && primary < aligned_score {
            aligned = Some(i);
            aligned_score = primary;
        }
    }
    aligned.or(best)
}

// ---- voice
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceState {
    Idle,
    Listening,
    Working,
    Failed(String),
}

struct Voice {
    state: VoiceState,
    mic: bool,
    busy: bool,
    released: bool,
    session: u64,
    request: u64,
    clear_at: Option<u64>,
    topic: Value,
    helper_up: bool,
}

impl Default for Voice {
    fn default() -> Self {
        Self { state: VoiceState::Idle, mic: false, busy: false, released: false, session: 0, request: 0, clear_at: None, topic: Value::Null, helper_up: false }
    }
}

impl Voice {
    fn ready(&self) -> bool {
        self.topic.get("ready").and_then(Value::as_bool) == Some(true)
    }

    /// Why the helper would not listen: its own words when it gave some, else what its `voice` topic says.
    fn why(&self, doc: &Value) -> String {
        if let Some(m) = doc.pointer("/error/message").and_then(Value::as_str) {
            return m.to_owned();
        }
        if !self.helper_up {
            return "The TV\u{2019}s helper is not running".into();
        }
        match self.topic.get("reason").and_then(Value::as_str).unwrap_or("") {
            "voice_off" => "Voice search is switched off",
            "voice_unlinked" => "Voice search is not linked to Home Assistant",
            "no_microphone" => "No microphone is chosen for voice search",
            _ => "Voice search is busy or not available",
        }
        .into()
    }

    fn down(&mut self, out: &mut Vec<Effect>) {
        if self.busy {
            return;
        }
        self.request += 1;
        self.busy = true;
        self.released = false;
        self.session = 0;
        self.clear_at = None;
        self.state = VoiceState::Listening;
        self.mic = true;
        out.push(Effect::Request { tag: VOICE_TAG + self.request, method: "POST".into(), path: "/v1/voice/listen".into(), body: Some(json!({})) });
    }

    fn up(&mut self, out: &mut Vec<Effect>) {
        if !self.busy {
            return;
        }
        self.released = true;
        if self.state == VoiceState::Listening {
            self.state = VoiceState::Working;
        }
        if self.session != 0 {
            out.push(Effect::Post { path: "/v1/voice/stop".into(), body: json!({}) });
        }
    }

    /// Back, or anything that takes Home away: the microphone is closed and nothing is reported.
    fn abort(&mut self, out: &mut Vec<Effect>) {
        if !self.busy && self.state == VoiceState::Idle {
            return;
        }
        self.busy = false;
        self.session = 0;
        self.mic = false;
        self.state = VoiceState::Idle;
        self.clear_at = None;
        out.push(Effect::Post { path: "/v1/voice/cancel".into(), body: json!({}) });
    }

    fn fail(&mut self, words: String, now: u64) {
        self.busy = false;
        self.mic = false;
        self.state = VoiceState::Failed(words);
        self.clear_at = Some(now + VOICE_CLEAR_MS);
    }

    fn end(&mut self) {
        self.busy = false;
        self.mic = false;
        self.state = VoiceState::Idle;
        self.clear_at = None;
    }

    fn answered(&mut self, tag: u64, status: u16, doc: &Value, now: u64, out: &mut Vec<Effect>) {
        let ok = status == 200 && doc.get("ok").and_then(Value::as_bool) == Some(true);
        // Back (or the app launched) came first: whatever started is ended again
        if tag != VOICE_TAG + self.request || !self.busy {
            if ok {
                out.push(Effect::Post { path: "/v1/voice/cancel".into(), body: json!({}) });
            }
            return;
        }
        if ok {
            self.session = doc.get("session").and_then(Value::as_u64).unwrap_or(0);
            if self.released && self.session != 0 {
                out.push(Effect::Post { path: "/v1/voice/stop".into(), body: json!({}) });
            }
            self.topic_changed(now);
        } else {
            let why = self.why(doc);
            self.fail(why, now);
        }
    }

    fn topic_changed(&mut self, now: u64) {
        let v = &self.topic;
        if !self.busy || self.session == 0 || v.get("session").and_then(Value::as_u64) != Some(self.session) {
            return;
        }
        let state = v.get("state").and_then(Value::as_str).unwrap_or("");
        if state == "listening" {
            return;
        }
        self.mic = false;
        match state {
            "thinking" => self.state = VoiceState::Working,
            // the words go to Search, which Home opens with them; the bar has done its part
            "heard" => self.end(),
            "error" => {
                let why = v.pointer("/error/message").and_then(Value::as_str).unwrap_or("Voice search failed").to_owned();
                self.fail(why, now);
            }
            _ => self.end(),
        }
    }
}

/// What the voice bar shows.
#[derive(Clone, Debug, PartialEq)]
pub struct VoiceView {
    pub state: VoiceState,
}

// ---- the mini frame
#[derive(Default)]
struct Frame {
    page: Option<Page>,
    from: [f32; 4],
    to: [f32; 4],
    opened_at: u64,
    landed: bool,
    loading: bool,
    wanted: bool,
    seen: bool,
    grace_at: Option<u64>,
}

/// What the mini frame shows.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameView {
    pub title: String,
    pub subtitle: String,
    pub from: [f32; 4],
    pub to: [f32; 4],
    pub landed: bool,
    pub loading: bool,
}

/// Everything the surface draws.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct View {
    pub splash: Option<SplashView>,
    pub toast: Option<Toast>,
    pub ask: Option<Ask>,
    pub remote: Option<usize>,
    pub voice: Option<VoiceView>,
    pub mic: bool,
    pub frame: Option<FrameView>,
}

pub struct Extras {
    splash: Splash,
    access: Access,
    remote: Option<usize>,
    voice: Voice,
    frame: Frame,
    intents: Vec<Intent>,
    smooth: bool,
    reduced: bool,
    light: bool,
    home_front: bool,
}

impl Default for Extras {
    fn default() -> Self {
        Self::new()
    }
}

impl Extras {
    pub fn new() -> Self {
        Self { splash: Splash::default(), access: Access::default(), remote: None, voice: Voice::default(), frame: Frame::default(), intents: Vec::new(), smooth: false, reduced: false, light: false, home_front: false }
    }

    /// The subscription that tells when the frame's web half is up and when it has gone.
    pub fn start(&self) -> Vec<Effect> {
        vec![Effect::Luna { tag: RUNNING_TAG, uri: format!("{APP_MGR}/running"), payload: json!({"subscribe": true}), subscribe: true }]
    }

    /// The material the frame's web half is painted in follows the shell's.
    pub fn set_light(&mut self, light: bool) {
        self.light = light;
    }

    /// Where the launch screen's icon is (the surface knows once it has drawn it): where the compositor's card grows from.
    pub fn set_splash_icon(&mut self, rect: Tile) {
        self.splash.rect = Some(rect);
    }

    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    /// Whether the voice key is the extras' to take: the helper says voice search is ready, or a request is under way.
    pub fn takes_voice(&self) -> bool {
        self.home_front && (self.voice.ready() || self.voice.busy)
    }

    /// Home has told something.
    pub fn note(&mut self, note: Note, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        match note {
            Note::Launch { id, title, icon, tile } => self.splash.launch((&id, &title, &icon), tile, self.smooth, now, &mut out),
            Note::Toast { digit, title, line } => self.access.toast = Some((Toast { digit, title, line }, now + TOAST_MS)),
            Note::Ask { digit, picks } => {
                self.access.toast = None;
                self.access.ask = Some(Ask { digit, picks, index: 0 });
            }
            Note::Remote => self.remote = if self.remote.is_some() { None } else { Some(0) },
            Note::Frame { page, from, to } => {
                self.frame = Frame { page: Some(page), from, to, opened_at: now, loading: true, ..Frame::default() };
            }
        }
        out
    }

    /// The mic key went down or up.
    pub fn voice_key(&mut self, down: bool, _now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if down {
            self.voice.down(&mut out);
        } else {
            self.voice.up(&mut out);
        }
        out
    }

    fn close_all(&mut self, out: &mut Vec<Effect>) {
        self.access.ask = None;
        self.remote = None;
        self.voice.abort(out);
        self.close_frame(out);
    }

    fn close_frame(&mut self, out: &mut Vec<Effect>) {
        if self.frame.page.is_none() {
            return;
        }
        if self.frame.wanted {
            out.push(Effect::Luna { tag: FRAME_TAG, uri: format!("{APP_MGR}/closeByAppId"), payload: json!({"id": FRAME_APP}), subscribe: false });
        }
        self.frame = Frame::default();
    }

    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        match input {
            Input::Topic { topic, data } => match topic.as_str() {
                "transitions" => {
                    self.smooth = data.get("on").and_then(Value::as_bool) == Some(true);
                    self.reduced = data.get("reduced").and_then(Value::as_bool) == Some(true);
                }
                "foreground" => {
                    let home = data.get("home").and_then(Value::as_bool).unwrap_or(false);
                    let covered = data.get("covered").and_then(Value::as_bool).unwrap_or(!home);
                    let front = home && !covered;
                    // Home going away is the app arriving; Home coming back is the launch having failed or the app having gone
                    if front != self.home_front {
                        self.home_front = front;
                        self.splash.done();
                        if !front {
                            self.close_all(&mut out);
                        }
                    }
                }
                _ => {}
            },
            Input::Helper { topic, data } => match topic.as_str() {
                "up" => self.voice.helper_up = data.as_bool().unwrap_or(false),
                "voice" => {
                    self.voice.topic = data.clone();
                    self.voice.topic_changed(now);
                }
                _ => {}
            },
            Input::Fetched { tag, status, data } if (VOICE_TAG..VOICE_TAG + 0x1_0000).contains(tag) => self.voice.answered(*tag, *status, data, now, &mut out),
            Input::Luna { tag: RUNNING_TAG, payload } => self.running(payload, now, &mut out),
            Input::Key { name, down, repeat } if self.wants() => self.key(name, *down, *repeat, now, &mut out),
            _ => {}
        }
        self.tick(now, &mut out);
        out
    }

    fn running(&mut self, payload: &Value, now: u64, out: &mut Vec<Effect>) {
        let Some(list) = payload.get("running").and_then(Value::as_array) else { return };
        let seen = list.iter().any(|a| a.get("id").and_then(Value::as_str) == Some(FRAME_APP));
        if seen && !self.frame.seen {
            // the app paints its own ring once it is up; ours covers the launch latency
            self.frame.seen = true;
            self.frame.grace_at = Some(now + FRAME_GRACE_MS);
        } else if !seen && self.frame.seen {
            self.frame.seen = false;
            if self.frame.wanted {
                self.frame.wanted = false;
                self.close_frame(out);
            }
        }
    }

    fn key(&mut self, name: &str, down: bool, repeat: bool, _now: u64, out: &mut Vec<Effect>) {
        if !down {
            return;
        }
        if let Some(ask) = &mut self.access.ask {
            let (more, later) = (ask.more(), ask.later());
            match name {
                "left" if ask.index <= more => ask.index = ask.index.saturating_sub(1),
                "right" if ask.index < more => ask.index += 1,
                "down" if ask.index <= more => ask.index = later,
                "up" if ask.index == later => ask.index = more,
                "ok" if !repeat => self.answer(),
                "back" if !repeat => self.access.ask = None,
                _ => {}
            }
        } else if let Some(i) = self.remote {
            match name {
                "back" | "screenremote" if !repeat => self.remote = None,
                "ok" => self.press(i, out),
                d @ ("up" | "down" | "left" | "right") => self.remote = remote_next(i, d).or(Some(i)),
                _ => {}
            }
        } else if self.frame.page.is_some() {
            if name == "back" && !repeat {
                self.close_frame(out);
            }
        } else if self.voice.state != VoiceState::Idle && name == "back" && !repeat {
            self.voice.abort(out);
        }
    }

    /// Picks the highlighted answer of the question.
    pub fn answer(&mut self) {
        let Some(ask) = self.access.ask.take() else { return };
        if ask.index < ask.picks.len() {
            self.intents.push(Intent::Assign { digit: ask.digit, app: ask.picks[ask.index].id.clone() });
        } else if ask.index == ask.more() {
            self.intents.push(Intent::More(ask.digit));
        }
    }

    /// A key of the remote pressed by OK or a click.
    pub fn press(&mut self, i: usize, out: &mut Vec<Effect>) {
        let Some(k) = REMOTE_KEYS.get(i) else { return };
        self.remote = Some(i);
        // Back on the panel is leaving it
        if k.name == "back" {
            self.remote = None;
            return;
        }
        if let Some(code) = remote_code(k.name) {
            out.push(Effect::Luna { tag: MICOM_TAG, uri: format!("{MICOM}/sendKeycode"), payload: json!({"keycode": code}), subscribe: false });
        }
    }

    /// A click: a card or Not now answers the question, a key of the remote is pressed.
    pub fn hit(&mut self, click: Click, out: &mut Vec<Effect>) {
        match click {
            Click::Pick(i) => self.pick(i),
            Click::Later => self.pick(usize::MAX),
            Click::Key(i) => self.press(i, out),
        }
    }

    /// A click on a pick of the question.
    pub fn pick(&mut self, i: usize) {
        if let Some(ask) = &mut self.access.ask {
            ask.index = i.min(ask.later());
        }
        self.answer();
    }

    pub fn focus_remote(&mut self, i: usize) {
        if self.remote.is_some() && i < REMOTE_KEYS.len() {
            self.remote = Some(i);
        }
    }

    pub fn focus_ask(&mut self, i: usize) {
        if let Some(ask) = &mut self.access.ask {
            ask.index = i.min(ask.later());
        }
    }

    /// Back on the voice bar (a click outside it is not one).
    pub fn voice_back(&mut self) -> Vec<Effect> {
        let mut out = Vec::new();
        self.voice.abort(&mut out);
        out
    }

    pub fn close_frame_now(&mut self) -> Vec<Effect> {
        let mut out = Vec::new();
        self.close_frame(&mut out);
        out
    }

    fn tick(&mut self, now: u64, out: &mut Vec<Effect>) {
        self.splash.tick(now, self.smooth, out);
        if self.access.toast.as_ref().is_some_and(|t| now >= t.1) {
            self.access.toast = None;
        }
        if self.voice.clear_at.is_some_and(|t| now >= t) && !self.voice.busy {
            self.voice.end();
        }
        if self.frame.page.is_some() {
            if !self.frame.landed && now >= self.frame.opened_at + FRAME_LAND_MS {
                self.frame.landed = true;
                if let Some(p) = self.frame.page.clone() {
                    let t = self.frame.to;
                    let (x, y, w, h) = (t[0] + 32.0, t[1] + 64.0, t[2] - 64.0, t[3] - 64.0 - 32.0);
                    self.frame.wanted = true;
                    out.push(Effect::Luna {
                        tag: FRAME_TAG,
                        uri: format!("{APP_MGR}/launch"),
                        payload: json!({"id": FRAME_APP, "params": {
                            "url": p.url, "title": p.title, "source": p.source, "ago": p.ago, "mode": "auto", "light": self.light,
                            "rect": {"x": x, "y": y, "w": w, "h": h},
                        }}),
                        subscribe: false,
                    });
                }
            }
            if self.frame.grace_at.is_some_and(|t| now >= t) {
                self.frame.grace_at = None;
                self.frame.loading = false;
            }
        }
    }

    /// Whether the extras have the keys: the question, the remote, the frame or the voice bar is up.
    pub fn wants(&self) -> bool {
        self.access.ask.is_some() || self.remote.is_some() || self.frame.page.is_some() || self.voice.state != VoiceState::Idle
    }

    /// Whether something that turns (the ring of the launch screen, the loading mark) is on the screen.
    pub fn spinning(&self) -> bool {
        self.splash.shown.is_some() || self.frame.loading || self.voice.state == VoiceState::Working
    }

    pub fn due_at(&self, now: u64) -> Option<u64> {
        let frame = self.frame.page.as_ref().map(|_| if self.frame.landed { u64::MAX } else { self.frame.opened_at + FRAME_LAND_MS }).filter(|t| *t != u64::MAX);
        let spin = self.spinning().then(|| (now / SPIN_MS + 1) * SPIN_MS);
        [self.splash.due_at(), self.access.toast.as_ref().map(|t| t.1), self.voice.clear_at, frame, self.frame.grace_at, spin].into_iter().flatten().min()
    }

    pub fn view(&self) -> View {
        View {
            splash: self.splash.shown.as_ref().map(|(name, icon)| SplashView { name: name.clone(), icon: icon.clone(), fade: self.smooth && !self.reduced }),
            toast: self.access.toast.as_ref().map(|t| t.0.clone()),
            ask: self.access.ask.clone(),
            remote: self.remote,
            voice: (self.voice.state != VoiceState::Idle).then(|| VoiceView { state: self.voice.state.clone() }),
            mic: self.voice.mic,
            frame: self.frame.page.as_ref().map(|p| FrameView {
                title: p.title.clone(),
                subtitle: p.subtitle.clone(),
                from: self.frame.from,
                to: self.frame.to,
                landed: self.frame.landed,
                loading: self.frame.loading,
            }),
        }
    }

    pub fn state(&self) -> Value {
        json!({
            "splash": self.splash.shown.as_ref().map(|s| json!({"name": s.0, "icon": s.1})),
            "splashWaiting": self.splash.pending.is_some(),
            "toast": self.access.toast.as_ref().map(|t| json!({"digit": t.0.digit, "title": t.0.title, "line": t.0.line})),
            "ask": self.access.ask.as_ref().map(|a| json!({"digit": a.digit, "index": a.index, "picks": a.picks.iter().map(|p| p.id.clone()).collect::<Vec<_>>()})),
            "remote": self.remote.map(|i| REMOTE_KEYS[i].name),
            "voice": match &self.voice.state { VoiceState::Idle => "", VoiceState::Listening => "listening", VoiceState::Working => "working", VoiceState::Failed(_) => "failed" },
            "voiceWords": if let VoiceState::Failed(w) = &self.voice.state { w.clone() } else { String::new() },
            "mic": self.voice.mic,
            "frame": self.frame.page.as_ref().map(|p| json!({"title": p.title, "landed": self.frame.landed, "loading": self.frame.loading, "wanted": self.frame.wanted})),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(t: &str, data: Value) -> Input {
        Input::Topic { topic: t.into(), data }
    }

    fn key_in(name: &str) -> Input {
        Input::Key { name: name.into(), down: true, repeat: false }
    }

    fn front(e: &mut Extras, now: u64) {
        e.update(&topic("foreground", json!({"home": true, "covered": false})), now);
    }

    fn launch(e: &mut Extras, now: u64) -> Vec<Effect> {
        e.note(Note::Launch { id: "app.x".into(), title: "App X".into(), icon: "/icon.png".into(), tile: Some(Tile { x: 10.0, y: 20.0, w: 120.0, h: 120.0, radius: 26.0 }) }, now)
    }

    fn out_of(e: &mut Extras, now: u64) -> Vec<Effect> {
        e.update(&Input::Tick, now)
    }

    fn origin_calls(out: &[Effect]) -> Vec<Value> {
        out.iter().filter_map(|e| match e { Effect::Luna { tag: ORIGIN_TAG, payload, .. } => Some(payload.clone()), _ => None }).collect()
    }

    #[test]
    fn a_launch_with_the_transitions_on_tells_the_tile_waits_then_shows_and_tells_where_its_icon_is() {
        let mut e = Extras::new();
        front(&mut e, 0);
        e.update(&topic("transitions", json!({"on": true, "reduced": false})), 0);
        let out = launch(&mut e, 1000);
        let tile = origin_calls(&out);
        assert_eq!(tile.len(), 1, "the compositor is told where the card grows from");
        assert_eq!(tile[0]["tile"]["width"], 120.0);
        assert!(e.view().splash.is_none(), "the screen waits: an app that has a window by then never shows it");
        assert_eq!(e.due_at(1000), Some(1400));
        e.update(&Input::Tick, 1400);
        let v = e.view().splash.expect("the screen shows after 400 ms");
        assert_eq!((v.name.as_str(), v.icon.as_str(), v.fade), ("App X", "/icon.png", true));
        e.set_splash_icon(Tile { x: 870.0, y: 300.0, w: 180.0, h: 180.0, radius: 38.0 });
        let out = e.update(&Input::Tick, 1450);
        let from = origin_calls(&out);
        assert_eq!(from.len(), 1, "one frame after it shows, its icon has its place");
        assert_eq!(from[0]["from"]["x"], 870.0);
        assert!(from[0].get("tile").is_none());
    }

    #[test]
    fn an_app_that_comes_to_the_front_in_time_never_shows_the_launch_screen() {
        let mut e = Extras::new();
        front(&mut e, 0);
        e.update(&topic("transitions", json!({"on": true})), 0);
        launch(&mut e, 1000);
        e.update(&topic("foreground", json!({"home": false, "covered": true})), 1200);
        e.update(&Input::Tick, 1500);
        assert!(e.view().splash.is_none());
        assert_eq!(e.due_at(1500), None, "nothing is waiting any more");
    }

    #[test]
    fn without_the_transitions_the_screen_is_the_cut_it_was_and_it_gives_up_after_eight_seconds() {
        let mut e = Extras::new();
        front(&mut e, 0);
        let out = launch(&mut e, 1000);
        assert!(origin_calls(&out).is_empty(), "nothing to tell: no card grows");
        let v = e.view().splash.expect("up at once");
        assert!(!v.fade);
        e.update(&Input::Tick, 8999);
        assert!(e.view().splash.is_some());
        e.update(&Input::Tick, 9000);
        assert!(e.view().splash.is_none(), "Home is still in front: the app never came, and the screen does not cover Home for ever");
    }

    #[test]
    fn a_toast_stays_two_and_a_half_seconds_and_the_question_takes_the_keys() {
        let mut e = Extras::new();
        front(&mut e, 0);
        e.note(Note::Toast { digit: "4".into(), title: "4 \u{b7} Plex".into(), line: "Opening.".into() }, 100);
        assert!(e.view().toast.is_some() && !e.wants(), "a toast takes no key");
        e.update(&Input::Tick, 2499);
        assert!(e.view().toast.is_some());
        e.update(&Input::Tick, 2500);
        assert!(e.view().toast.is_none());
        let picks = vec![Pick { id: "a".into(), title: "A".into(), icon: String::new() }, Pick { id: "b".into(), title: "B".into(), icon: String::new() }];
        e.note(Note::Ask { digit: "7".into(), picks }, 3000);
        assert!(e.wants());
        for k in ["right", "right"] {
            e.update(&key_in(k), 3100);
        }
        assert_eq!(e.view().ask.unwrap().index, 2, "More is after the apps");
        e.update(&key_in("right"), 3100);
        assert_eq!(e.view().ask.unwrap().index, 2, "and the last of the row");
        e.update(&key_in("down"), 3100);
        assert_eq!(e.view().ask.unwrap().index, 3, "Not now is below");
        e.update(&key_in("up"), 3100);
        e.update(&key_in("left"), 3100);
        e.update(&key_in("ok"), 3100);
        assert_eq!(e.take_intents(), vec![Intent::Assign { digit: "7".into(), app: "b".into() }]);
        assert!(!e.wants());
    }

    #[test]
    fn more_goes_to_settings_not_now_and_back_leave_it_as_it_was() {
        let mut e = Extras::new();
        front(&mut e, 0);
        let ask = |e: &mut Extras| e.note(Note::Ask { digit: "3".into(), picks: vec![Pick { id: "a".into(), title: "A".into(), icon: String::new() }] }, 0);
        ask(&mut e);
        e.update(&key_in("right"), 10);
        e.update(&key_in("ok"), 20);
        assert_eq!(e.take_intents(), vec![Intent::More("3".into())]);
        ask(&mut e);
        e.update(&key_in("down"), 10);
        e.update(&key_in("ok"), 20);
        assert!(e.take_intents().is_empty(), "Not now asks nothing");
        ask(&mut e);
        e.update(&key_in("back"), 10);
        assert!(e.take_intents().is_empty() && !e.wants());
        ask(&mut e);
        e.pick(0);
        assert_eq!(e.take_intents(), vec![Intent::Assign { digit: "3".into(), app: "a".into() }], "a click on an app is the answer");
    }

    #[test]
    fn the_remote_walks_its_keys_presses_the_micom_codes_and_back_closes_it() {
        let mut e = Extras::new();
        front(&mut e, 0);
        e.note(Note::Remote, 0);
        assert_eq!(e.view().remote, Some(0));
        let out = e.update(&key_in("right"), 10);
        assert!(out.is_empty());
        assert_eq!(e.view().remote, Some(1), "play is right of rewind");
        let out = e.update(&key_in("ok"), 20);
        assert!(matches!(&out[0], Effect::Luna { uri, payload, .. } if uri.ends_with("/sendKeycode") && payload["keycode"] == 176));
        e.update(&key_in("down"), 30);
        assert_eq!(REMOTE_KEYS[e.view().remote.unwrap()].group, 1, "down from the transport keys lands on the number pad");
        e.update(&key_in("back"), 40);
        assert_eq!(e.view().remote, None);
        e.note(Note::Remote, 50);
        e.note(Note::Remote, 60);
        assert_eq!(e.view().remote, None, "the remote key a second time closes it");
    }

    #[test]
    fn nothing_on_the_remote_can_answer_a_prompt_that_belongs_to_the_physical_remote() {
        // The helper reads OK and Back off the remote's own input device; the micom service's keys arrive as the remote's. So the panel must
        // have no OK, no Back, no arrow, and no key that closes or confirms anything.
        let names: Vec<&str> = REMOTE_KEYS.iter().map(|k| k.name).collect();
        for banned in ["ok", "enter", "select", "left", "right", "up", "down", "exit", "home", "menu"] {
            assert!(!names.contains(&banned), "{banned} is on the on-screen remote");
        }
        for k in &names {
            if let Some(code) = remote_code(k) {
                // LG's IR command numbers of OK (68), Back (40), Up (64), Down (65), Left (7), Right (6), Exit (91) and Home (124)
                assert!(![68, 40, 64, 65, 7, 6, 91, 124].contains(&code), "{k} sends {code}");
            }
        }
        assert_eq!(remote_code("back"), None, "Back on the panel is leaving it, and sends nothing");
    }

    fn voice_ready(e: &mut Extras, now: u64) {
        e.update(&Input::Helper { topic: "up".into(), data: json!(true) }, now);
        e.update(&Input::Helper { topic: "voice".into(), data: json!({"ready": true, "state": "idle"}) }, now);
    }

    #[test]
    fn holding_the_mic_key_listens_and_letting_go_stops_and_the_words_go_to_search() {
        let mut e = Extras::new();
        front(&mut e, 0);
        assert!(!e.takes_voice(), "the key stays LG's while the helper does not say ready");
        voice_ready(&mut e, 5);
        assert!(e.takes_voice());
        let out = e.voice_key(true, 10);
        let tag = match &out[0] {
            Effect::Request { tag, path, .. } if path == "/v1/voice/listen" => *tag,
            other => panic!("{other:?}"),
        };
        assert_eq!(e.view().voice.unwrap().state, VoiceState::Listening);
        assert!(e.view().mic);
        e.voice_key(false, 20);
        assert_eq!(e.view().voice.unwrap().state, VoiceState::Working, "released before the helper answered");
        let out = e.update(&Input::Fetched { tag, status: 200, data: json!({"ok": true, "session": 7}) }, 30);
        assert!(out.iter().any(|f| matches!(f, Effect::Post { path, .. } if path == "/v1/voice/stop")), "the stop that waited for the session goes now");
        e.update(&Input::Helper { topic: "voice".into(), data: json!({"ready": true, "session": 7, "state": "thinking"}) }, 40);
        assert_eq!(e.view().voice.unwrap().state, VoiceState::Working);
        assert!(!e.view().mic, "the helper says the microphone is closed");
        e.update(&Input::Helper { topic: "voice".into(), data: json!({"ready": true, "session": 7, "state": "heard", "text": "the news"}) }, 50);
        assert!(e.view().voice.is_none(), "the bar's part is done: Home opens Search with the words");
    }

    #[test]
    fn a_voice_request_that_failed_says_why_for_a_moment_and_back_ends_one_that_started() {
        let mut e = Extras::new();
        front(&mut e, 0);
        e.update(&Input::Helper { topic: "up".into(), data: json!(true) }, 0);
        e.update(&Input::Helper { topic: "voice".into(), data: json!({"ready": true, "reason": "voice_off"}) }, 0);
        let out = e.voice_key(true, 100);
        let Effect::Request { tag, .. } = out[0].clone() else { panic!() };
        e.update(&Input::Fetched { tag, status: 409, data: Value::Null }, 110);
        assert_eq!(e.view().voice.unwrap().state, VoiceState::Failed("Voice search is switched off".into()));
        assert!(e.wants(), "the bar is up and Back is its");
        e.update(&Input::Tick, 110 + VOICE_CLEAR_MS);
        assert!(e.view().voice.is_none(), "a reason stays long enough to read, then the bar goes");
        // Back while listening: whatever started is ended, and a late answer to that press ends again
        let out = e.voice_key(true, 5000);
        let Effect::Request { tag, .. } = out[0].clone() else { panic!() };
        let out = e.update(&key_in("back"), 5010);
        assert!(out.iter().any(|f| matches!(f, Effect::Post { path, .. } if path == "/v1/voice/cancel")));
        let out = e.update(&Input::Fetched { tag, status: 200, data: json!({"ok": true, "session": 9}) }, 5020);
        assert!(out.iter().any(|f| matches!(f, Effect::Post { path, .. } if path == "/v1/voice/cancel")), "the press is old: what it started is ended again");
        assert!(e.view().voice.is_none());
    }

    fn page() -> Page {
        Page { url: "https://example.org/a".into(), title: "A story".into(), subtitle: "Wire \u{b7} 2 h".into(), source: "Wire".into(), ago: "2 h".into() }
    }

    #[test]
    fn a_page_grows_out_of_its_card_then_the_web_half_is_launched_into_its_interior_and_back_closes_both() {
        let mut e = Extras::new();
        front(&mut e, 0);
        e.set_light(true);
        e.update(&Input::Helper { topic: "up".into(), data: json!(true) }, 0);
        e.note(Note::Frame { page: page(), from: [100.0, 200.0, 400.0, 300.0], to: [200.0, 150.0, 1520.0, 860.0] }, 1000);
        assert!(e.wants());
        assert!(e.view().frame.unwrap().loading);
        assert!(out_of(&mut e, 1259).is_empty(), "the web half waits for the frame to land");
        let out = e.update(&Input::Tick, 1260);
        let Effect::Luna { uri, payload, .. } = &out[0] else { panic!("{out:?}") };
        assert!(uri.ends_with("/launch"));
        assert_eq!(payload["id"], FRAME_APP);
        assert_eq!(payload["params"]["url"], "https://example.org/a");
        assert_eq!(payload["params"]["light"], true);
        assert_eq!(payload["params"]["rect"]["x"], 232.0, "the interior: the frame's x and a 32 px pad");
        assert_eq!(payload["params"]["rect"]["y"], 214.0, "under the 64 px bar");
        assert_eq!(payload["params"]["rect"]["w"], 1456.0);
        assert_eq!(payload["params"]["rect"]["h"], 764.0);
        // the app shows up in the running list: the ring stays a moment more, then goes
        e.update(&Input::Luna { tag: RUNNING_TAG, payload: json!({"running": [{"id": FRAME_APP}]}) }, 1500);
        assert!(e.view().frame.unwrap().loading);
        e.update(&Input::Tick, 1850);
        assert!(!e.view().frame.unwrap().loading);
        // Back in the web half closes the app; its going closes the frame
        e.update(&Input::Luna { tag: RUNNING_TAG, payload: json!({"running": []}) }, 5000);
        assert!(e.view().frame.is_none());
        // Back on the frame itself closes the app too
        e.note(Note::Frame { page: page(), from: [0.0; 4], to: [200.0, 150.0, 1520.0, 860.0] }, 6000);
        e.update(&Input::Tick, 6300);
        let out = e.update(&key_in("back"), 6400);
        assert!(matches!(&out[0], Effect::Luna { uri, payload, .. } if uri.ends_with("/closeByAppId") && payload["id"] == FRAME_APP));
        assert!(e.view().frame.is_none());
    }

    #[test]
    fn home_going_away_closes_every_panel_and_ends_the_microphone() {
        let mut e = Extras::new();
        front(&mut e, 0);
        voice_ready(&mut e, 1);
        e.voice_key(true, 10);
        e.note(Note::Remote, 20);
        let out = e.update(&topic("foreground", json!({"home": false, "covered": true})), 30);
        assert!(out.iter().any(|f| matches!(f, Effect::Post { path, .. } if path == "/v1/voice/cancel")));
        assert!(!e.wants() && !e.view().mic);
    }

    #[test]
    fn the_spinner_asks_for_a_tick_while_something_turns() {
        let mut e = Extras::new();
        front(&mut e, 0);
        assert_eq!(e.due_at(1000), None);
        launch(&mut e, 1000);
        assert!(e.spinning());
        assert_eq!(e.due_at(1010), Some(1050), "the next step of the ring");
    }
}
