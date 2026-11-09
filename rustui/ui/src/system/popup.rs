//! The camera pop-up's model (the pop-up of `panels/Cameras.qml`, `docs/research/cameras-ui-spec.md` "The pop-up"): a detector of a shared camera
//! saw something (`cameras.popup`: the camera, the kind, when) and the card says so for a while, with the camera's picture in it: live video on a
//! hardware video window where the compositor gives one, and the camera's stills until its first picture is on the plane and where it gives none.
//!
//! - **Due** when its `id` differs from the one shown and `untilMs` is ahead. It waits (looking again every 500 ms) while the host is busy
//!   (a PIN, an alert, a panel, the keyboard) and, with `overApps` false, while an app is in front; it shows when free if still in date.
//!   One that is up when the host turns busy, or an app comes to the front with `overApps` false, steps aside, is not dismissed and
//!   returns with the time it has left. An `update` of the same story changes the words of the card up and re-reads `untilMs`; it is never
//!   a second pop-up. It ends at `untilMs` and tells the helper (`/v1/cameras/popup/clear`).
//! - **Keys:** only OK and Back, and only 600 ms after it arrived (a press made as it came was the app's). OK clears it (the helper is told)
//!   and asks for the full view of its camera; Back clears it. Where the snooze sheet is this process's, OK is decided on its release and OK
//!   held 700 ms asks for the sheet ([`Intent::Snooze`]); the card steps aside under it and a snooze picked there ends it ([`Popup::snoozed`]).
//!   While the viewer is up there is no card.
//! - **Picture:** the event's own (`/v1/cameras/events/<event>/thumbnail`, once) at once, then the camera's stills (`.../frame`), the next ask 250 ms after
//!   the last arrived. `Live` once a still is on screen; `Event picture` while only the event's is; `Connecting` before any; `No picture`
//!   after two refused still asks with none shown.
//! - **Live video** (the record says `live`): the card asks the compositor for a video window for its picture box (`Popup::claim`, kind `camera-popup`, which the
//!   shell posts) and asks the helper for the stream only once that window is granted (`popup_live.rs`): a decoder is never started beside an app's video the
//!   compositor did not allow. The picture box is a hole over the plane once the stream plays, the claim is carried and videooutputd lists the video on a
//!   sink, `popup_live::HANDOFF_MS` after the helper says so; the stills stop while it is. Refused, taken back or failed twice: the stills go on, quicker
//!   (8 a second) than while they wait for the video.
//! - A doorbell stays half as long again and tells the overlay queue it showed, which folds the plugin's record of the same ring into the card.

use super::popup_live::Live;
use crate::cameras::app::Window;
use crate::cameras::pics::{Planner, Want};
use crate::cameras::sheet::encode;
use crate::kit::picture::{self, Picture};
use crate::kit::screen::{Effect, Input};
use crate::panels::hold::{Hold, Released};
use crate::panels::layer::HOLD_MS;
use serde_json::{Value, json};
use std::collections::HashMap;

/// A card meant to be seen is not taken for less than this.
const MIN_MS: i64 = 500;
/// The keys are the card's only after this.
pub const ARM_MS: u64 = 600;
/// The card looks again for its turn this often while it waits.
const WAIT_MS: u64 = 500;
/// The age's words are made again this often.
const AGO_MS: u64 = 30_000;
/// The stills' pace: the next ask after the last arrived plus this.
const STILL_MS: u64 = 250;
/// ... and where they are the only picture there is (the video was refused, taken back or failed, or the record says there is none).
const QUICK_MS: u64 = 125;
/// ... and while an app holds both scalers (a game at 120 Hz, 4K video) quieter still, so as not to take frames from it.
const QUIET_MS: u64 = 250;
/// A card that can play live video has this long to be answered about its window before its stills quicken (a compositor with the switch off answers
/// "off", which asks for no more than the stills).
const QUICK_AFTER_MS: u64 = 1_500;
/// The tags of the pictures' asks; the live stream's are `popup_live::TAG_FIRST` and up.
const FIRST: u64 = 0x5359_1000;
const LAST: u64 = 0x5359_1EFF;
/// The default picture box, before the surface has measured one (the spec's 520 by 292).
pub const PICTURE: (u32, u32) = (520, 292);

/// The record of `cameras.popup` as the card reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub id: String,
    pub event: String,
    pub camera: String,
    pub name: String,
    pub kind: String,
    pub label: String,
    /// When it was seen, on the bridge's clock (milliseconds).
    pub at: i64,
    pub until_ms: i64,
    /// The owner's "Over playing apps" switch: `Some(false)` keeps it off an app.
    pub over_apps: Option<bool>,
    /// A picture of the moment is there.
    pub thumbnail: bool,
    /// The helper says live video can play for this card (`cameras.popupLive` is on and the live path is there); `None` from a helper that does not say.
    pub live: Option<bool>,
    /// The helper says the camera has no stream a video window can play without the panel following its rate (a 1080p stream at 25 a second takes the panel
    /// to 100 Hz): the card claims a texture, never a window. `false` from a helper that does not say.
    pub texture_only: bool,
}

fn text(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(n) if n.is_number() => n.to_string(),
        _ => String::new(),
    }
}

impl Record {
    pub fn parse(v: &Value) -> Option<Self> {
        if !v.is_object() {
            return None;
        }
        let camera = text(v, "camera");
        if camera.is_empty() {
            return None;
        }
        let int = |k: &str| v.get(k).and_then(|n| n.as_i64().or_else(|| n.as_f64().map(|f| f as i64))).unwrap_or(0);
        Some(Self {
            id: text(v, "id"),
            event: text(v, "event"),
            name: text(v, "name"),
            kind: text(v, "kind"),
            label: text(v, "label"),
            at: int("at"),
            until_ms: int("untilMs"),
            over_apps: v.get("overApps").and_then(Value::as_bool),
            thumbnail: v.get("thumbnail").and_then(Value::as_bool) == Some(true),
            live: v.get("live").and_then(Value::as_bool),
            texture_only: v.get("textureOnly").and_then(Value::as_bool) == Some(true),
            camera,
        })
    }

    fn blocked_over_app(&self) -> bool {
        self.over_apps == Some(false)
    }

    /// The card's number for the helper's log.
    fn card_id(&self) -> Option<u64> {
        self.id.parse().ok()
    }

    /// The camera's name, its id where it has none.
    fn title(&self) -> String {
        if self.name.is_empty() { self.camera.clone() } else { self.name.clone() }
    }
}

/// What the picture box says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Connecting,
    EventPicture,
    Live,
    NoPicture,
}

impl Face {
    pub fn words(self) -> &'static str {
        match self {
            Face::Connecting => "Connecting",
            Face::EventPicture => "Event picture",
            Face::Live => "Live",
            Face::NoPicture => "No picture",
        }
    }
}

/// What the surface draws.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub up: bool,
    pub camera: String,
    pub name: String,
    pub icon: &'static str,
    /// What was seen ("Person", "Animal  ·  Domestic Cat", "Doorbell rang") and how long ago ("Just now").
    pub what: String,
    pub age: String,
    pub face: Face,
    /// The full view can be opened from here (the Cameras app is the Rust shell's).
    pub can_expand: bool,
    /// OK held asks for the snooze sheet.
    pub can_snooze: bool,
    /// Whether the keys are the card's yet.
    pub armed: bool,
    /// The picture box is a hole over the video plane (the live video is what shows there), not a picture of the shell's.
    pub video: bool,
    /// The camera's picture size (width, height), which the picture box takes the shape of: the stream as decoded once the helper says it, before that
    /// the camera's main stream (`cameras` topic, `native`), so that the card does not change shape when the video comes. `None`: 16:9.
    pub shape: Option<(u32, u32)>,
}

/// What the card asks of the shell besides effects.
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// OK: the full view of this camera.
    Expand(String),
    /// OK held: the snooze sheet for this camera (`camera` its id, `name` what the card calls it).
    Snooze { camera: String, name: String },
    /// A pop-up of this kind came up: the overlay queue takes in a doorbell record of the same ring.
    Shown(String),
}

pub struct Popup {
    topic: Option<Record>,
    current: Option<Record>,
    shown_id: Option<String>,
    up: bool,
    arrived_at: u64,
    ends_at: Option<u64>,
    waiting_at: Option<u64>,
    ago_at: Option<u64>,
    /// The bridge's clock less this TV's (`cameras.now_skew_ms`).
    skew_ms: i64,
    busy: bool,
    app_front: bool,
    viewer_up: bool,
    can_expand: bool,
    can_snooze: bool,
    /// OK while it is down, where a hold snoozes.
    ok: Hold,
    planner: Planner,
    size: (u32, u32),
    /// Each camera's main stream size, from the `cameras` topic: the card's shape before its stream says one.
    natives: HashMap<String, (u32, u32)>,
    wants: Vec<Want>,
    pictures: HashMap<&'static str, Picture>,
    /// The card's picture when OK opened the full view of its camera: the view's first one ([`Popup::take_expanded`]).
    expanded: Option<Picture>,
    /// The window was refused because an app holds both scalers: the stills keep quiet whatever the helper's rate.
    heavy: bool,
    /// Bumped each time a picture changed, for the surface.
    pub revision: u64,
    intents: Vec<Intent>,
    /// The live video's claim and stream (`popup_live.rs`).
    live: Live,
    /// Whether the picture box was a hole at the last look: a change is drawn.
    was_video: bool,
    /// The time of the last input: what the words' age is read at.
    clock: u64,
}

impl Default for Popup {
    fn default() -> Self {
        Self::new()
    }
}

fn icon_of(kind: &str) -> &'static str {
    match kind {
        "person" => "user",
        "vehicle" => "car",
        "animal" => "paw-print",
        "package" => "package",
        "doorbell" => "bell-ring",
        "motion" => "activity",
        _ => "camera",
    }
}

fn kind_word(kind: &str) -> String {
    let mut c = kind.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// "Person", "Animal  ·  Domestic Cat" (a label that only repeats the kind is left out), "Doorbell rang".
pub fn words_of(r: &Record) -> String {
    if r.kind == "doorbell" {
        return "Doorbell rang".into();
    }
    let w = kind_word(&r.kind);
    if !r.label.is_empty() && r.label.to_lowercase() != r.kind { format!("{w}  \u{b7}  {}", r.label) } else { w }
}

fn ago(s: i64) -> String {
    if s < 60 { "Just now".into() } else { format!("{} min ago", (s as f64 / 60.0).round() as i64) }
}

impl Popup {
    pub fn new() -> Self {
        Self {
            topic: None,
            current: None,
            shown_id: None,
            up: false,
            arrived_at: 0,
            ends_at: None,
            waiting_at: None,
            ago_at: None,
            skew_ms: 0,
            busy: false,
            app_front: false,
            viewer_up: false,
            can_expand: false,
            can_snooze: false,
            ok: Hold::default(),
            planner: Planner::with_tags(FIRST, LAST),
            size: PICTURE,
            natives: HashMap::new(),
            wants: Vec::new(),
            pictures: HashMap::new(),
            expanded: None,
            heavy: false,
            revision: 0,
            live: Live::new(),
            was_video: false,
            intents: Vec::new(),
            clock: 0,
        }
    }

    pub fn up(&self) -> bool {
        self.up
    }

    /// The Cameras app is the Rust shell's, so OK can open the full view.
    pub fn set_can_expand(&mut self, on: bool) {
        self.can_expand = on;
    }

    /// The snooze sheet is this process's, so OK held can open it.
    pub fn set_can_snooze(&mut self, on: bool) {
        self.can_snooze = on;
    }

    /// What the host has up that the card must not cover or be covered by: the card steps aside (it is not dismissed) or waits.
    pub fn set_context(&mut self, busy: bool, app_front: bool, viewer_up: bool, now: u64) -> Vec<Effect> {
        let changed = (self.busy, self.app_front, self.viewer_up) != (busy, app_front, viewer_up);
        self.busy = busy;
        self.app_front = app_front;
        self.viewer_up = viewer_up;
        let mut out = Vec::new();
        if !changed {
            return out;
        }
        if self.up && (busy || viewer_up || self.current.as_ref().is_some_and(|c| app_front && c.blocked_over_app())) {
            self.hold(viewer_up, &mut out);
            out.push(Effect::Redraw);
        }
        self.consider(now, &mut out);
        out
    }

    /// The box the picture is drawn in (the surface's measure): the stills are asked for at that size.
    pub fn set_picture_size(&mut self, w: u32, h: u32) {
        self.size = (w, h);
    }

    pub fn take_intents(&mut self) -> Vec<Intent> {
        std::mem::take(&mut self.intents)
    }

    /// The picture to draw: the still when there is one, else the event's.
    pub fn best(&self) -> Option<&Picture> {
        self.pictures.get("still").or_else(|| self.pictures.get("event"))
    }

    /// The picture the card had when OK opened the full view of its camera, once: the view opens on it, and the video replaces it when it plays.
    pub fn take_expanded(&mut self) -> Option<Picture> {
        self.expanded.take()
    }

    /// The claim on a video window for the card's picture box, while a card the record says can play live video is up: the shell posts it. A camera whose
    /// streams would all re-time the panel claims a texture (`texture_only`); any other claims a window, where the helper plays a calm stream whose declared
    /// rate keeps the panel at 120 Hz (`live::rate`). With another app in front the compositor answers a texture or stills (`app-front`), never a window.
    pub fn claim(&self) -> Option<Value> {
        let capable = self.up && !self.viewer_up && self.current.as_ref().is_some_and(|c| c.live == Some(true));
        let texture_only = self.current.as_ref().is_some_and(|c| c.texture_only);
        self.live.claim(capable, texture_only)
    }

    /// The picture box on the screen, in output pixels (the surface measured it at its last draw).
    pub fn set_picture_rect(&mut self, rect: Option<(i32, i32, i32, i32)>) {
        self.live.set_rect(rect);
    }

    /// What the compositor made of the claim (`slot`: the window it named, or why it refused), and whether videooutputd lists the video on a sink.
    pub fn set_window(&mut self, window: Window, slot: &str, sink: bool, now: u64) -> Vec<Effect> {
        self.clock = now;
        self.heavy = window == Window::Refused && slot == "both-scalers";
        let mut out = Vec::new();
        if self.up {
            out.extend(self.live.set_window(window, slot, sink, now));
            self.settle(now, &mut out);
        }
        out
    }

    pub fn view(&self) -> View {
        let (camera, name, what, age, icon) = match &self.current {
            Some(r) => {
                let age = ago(((self.clock as i64 + self.skew_ms - r.at) / 1000).max(0));
                (r.camera.clone(), r.title(), words_of(r), age, icon_of(&r.kind))
            }
            None => (String::new(), String::new(), String::new(), String::new(), "camera"),
        };
        let video = self.up && self.live.video(self.clock);
        let face = if video || self.pictures.contains_key("still") {
            Face::Live
        } else if self.pictures.contains_key("event") {
            Face::EventPicture
        } else if self.planner.pic("still").refused > 1 {
            Face::NoPicture
        } else {
            Face::Connecting
        };
        View { up: self.up, camera, name, icon, what, age, face, can_expand: self.can_expand, can_snooze: self.can_snooze, armed: self.armed(self.clock), video, shape: self.shape() }
    }

    /// The picture's size the card is shaped by (see [`View::shape`]).
    fn shape(&self) -> Option<(u32, u32)> {
        let camera = self.current.as_ref()?.camera.as_str();
        self.live.size().or_else(|| self.natives.get(camera).copied())
    }

    /// The keys OK and Back are the card's: it is up and has been for 600 ms.
    pub fn armed(&self, now: u64) -> bool {
        self.up && now.saturating_sub(self.arrived_at) >= ARM_MS
    }

    /// [`Popup::armed`] at the time of the last input.
    pub fn armed_now(&self) -> bool {
        self.armed(self.clock)
    }

    fn due(&self, wall: u64) -> Option<&Record> {
        self.topic.as_ref().filter(|t| Some(&t.id) != self.shown_id.as_ref() && t.until_ms > wall as i64)
    }

    fn consider(&mut self, now: u64, out: &mut Vec<Effect>) {
        let Some(t) = self.due(now).cloned() else {
            self.waiting_at = None;
            return;
        };
        if self.viewer_up {
            // already watching: the full view turns to the camera something was seen on, with no card over it
            self.shown_id = Some(t.id);
            return;
        }
        if self.busy || (self.app_front && t.blocked_over_app()) {
            self.waiting_at = Some(now + WAIT_MS);
            return;
        }
        self.waiting_at = None;
        self.shown_id = Some(t.id.clone());
        self.arrived_at = now;
        let span = (t.until_ms - now as i64).max(MIN_MS) as u64;
        // a doorbell stays half as long again
        let span = if t.kind == "doorbell" { span + span / 2 } else { span };
        self.ends_at = Some(now + span);
        self.ago_at = Some(now + AGO_MS);
        self.intents.push(Intent::Shown(t.kind.clone()));
        self.live.begin(&t.camera, t.card_id());
        self.was_video = false;
        self.current = Some(t);
        self.up = true;
        self.pictures.clear();
        self.revision += 1;
        self.wants.clear();
        out.push(Effect::Redraw);
        self.plan(now, out);
    }

    /// The card steps aside: not dismissed, due again when the host is free if its time is not over. `viewer`: the full view took the camera, so
    /// it is not due again. What it asked of the helper is stopped.
    fn hold(&mut self, viewer: bool, out: &mut Vec<Effect>) {
        self.up = false;
        self.ok.clear();
        self.ends_at = None;
        self.ago_at = None;
        if !viewer {
            self.shown_id = None;
        }
        self.current = None;
        self.wants.clear();
        self.planner.quiesce();
        self.was_video = false;
        out.extend(self.live.end(true));
    }

    /// The card goes: the helper is told (`tell`) and the stream it asked for is stopped, unless the full view of the same camera takes it over (`keep_live`).
    fn hide(&mut self, tell: bool, keep_live: bool, out: &mut Vec<Effect>) {
        if !self.up {
            return;
        }
        self.up = false;
        self.ok.clear();
        self.ends_at = None;
        self.ago_at = None;
        self.current = None;
        self.wants.clear();
        self.planner.quiesce();
        self.was_video = false;
        out.extend(self.live.end(!keep_live));
        if tell {
            out.push(Effect::Post { path: "/v1/cameras/popup/clear".into(), body: json!({}) });
        }
        out.push(Effect::Redraw);
    }

    /// OK (a tap, where a hold snoozes): the card clears and asks for the full view of its camera where there is one.
    fn ok(&mut self, out: &mut Vec<Effect>) {
        let camera = self.current.as_ref().map(|c| c.camera.clone()).unwrap_or_default();
        let picture = if self.can_expand { self.best().cloned() } else { None };
        // the full view of the same camera takes the stream over: it asks for it, and the helper finds it there
        self.hide(true, self.can_expand, out);
        if self.can_expand {
            // ... and opens on the card's picture, which the video replaces when it plays
            self.expanded = picture;
            self.intents.push(Intent::Expand(camera));
        }
    }

    /// A snooze was picked on the sheet the card asked for: the record of `camera` (of any camera when `None`, a snooze of them all) is done
    /// with. It is not shown again and the helper is told.
    pub fn snoozed(&mut self, camera: Option<&str>) -> Vec<Effect> {
        let mut out = Vec::new();
        let Some(t) = self.topic.as_ref().filter(|t| camera.is_none_or(|c| c == t.camera)) else { return out };
        self.shown_id = Some(t.id.clone());
        self.waiting_at = None;
        if self.up {
            self.hide(true, false, &mut out);
        } else {
            out.push(Effect::Post { path: "/v1/cameras/popup/clear".into(), body: json!({}) });
        }
        out
    }

    /// Whether the stills are the only picture there is: the helper says there is no live video, or it was refused, taken back or failed.
    fn stills_quick(&self, now: u64) -> bool {
        match self.current.as_ref().and_then(|c| c.live) {
            Some(false) => true,
            Some(true) => {
                self.live.given() || self.live.window() == Window::Refused || (self.live.window() == Window::Unclaimed && now >= self.arrived_at + QUICK_AFTER_MS)
            }
            None => false,
        }
    }

    fn plan(&mut self, now: u64, out: &mut Vec<Effect>) {
        if let Some(c) = &self.current
            && self.up
        {
            let cam = encode(&c.camera);
            let mut wants = Vec::new();
            // the stills stop while the video is the picture
            if !self.live.video(now) {
                let (every, pace) = match (self.stills_quick(now), self.heavy) {
                    (true, true) => (QUIET_MS, "&fps=4"),
                    (true, false) => (QUICK_MS, "&fps=8"),
                    _ => (STILL_MS, ""),
                };
                wants.push(Want::repeating("still", &format!("/v1/cameras/{cam}/frame?w={}&h={}{pace}", self.size.0, self.size.1), every));
            }
            if c.thumbnail && !c.event.is_empty() {
                wants.push(Want::once("event", &format!("/v1/cameras/events/{}/thumbnail?w={}&h={}", encode(&c.event), self.size.0, self.size.1)));
            }
            self.wants = wants;
            out.extend(self.planner.plan(&self.wants, now, 2));
        }
    }

    /// After a change of what the live video is: the picture box turns into a hole or back (a draw), and the stills follow.
    fn settle(&mut self, now: u64, out: &mut Vec<Effect>) {
        let video = self.up && self.live.video(now);
        if video != self.was_video {
            self.was_video = video;
            self.revision += 1;
            out.push(Effect::Redraw);
        }
        self.plan(now, out);
    }

    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        self.clock = now;
        let mut out = Vec::new();
        match input {
            Input::Helper { topic, data } if topic == "cameras" => {
                self.skew_ms = data.get("now_skew_ms").and_then(Value::as_i64).unwrap_or(0);
                if let Some(list) = data.get("cameras").and_then(Value::as_array) {
                    let before = self.shape();
                    self.natives = list
                        .iter()
                        .filter_map(|c| {
                            let n = c.get("native")?;
                            let side = |k: &str| n.get(k).and_then(Value::as_u64).filter(|v| *v > 0).map(|v| v as u32);
                            Some((text(c, "id"), (side("width")?, side("height")?)))
                        })
                        .collect();
                    if self.up && self.shape() != before {
                        out.push(Effect::Redraw);
                    }
                }
            }
            Input::Helper { topic, data } if topic == "cameras.popup" => {
                self.topic = Record::parse(data);
                if self.up {
                    match (&self.topic, &self.shown_id) {
                        (Some(t), Some(id)) if &t.id == id => {
                            // an update of the story on the card: its words and its end change, it is not a new pop-up
                            let span = (t.until_ms - now as i64).max(MIN_MS) as u64;
                            self.ends_at = Some(now + span);
                            self.current = Some(t.clone());
                            out.push(Effect::Redraw);
                        }
                        // cleared elsewhere, or replaced
                        _ => self.hide(false, false, &mut out),
                    }
                }
                self.consider(now, &mut out);
            }
            Input::Helper { topic, data } if topic == "cameras.live" => {
                if self.up {
                    let before = self.shape();
                    out.extend(self.live.topic(data, now));
                    if self.shape() != before {
                        out.push(Effect::Redraw);
                    }
                }
            }
            Input::Bytes { tag, status, bytes } if (FIRST..=LAST).contains(tag) => {
                if let Some(key) = self.planner.key_of(*tag).map(str::to_owned) {
                    let decoded = (*status == 200).then(|| picture::decode(bytes).ok()).flatten();
                    let ok = decoded.is_some();
                    if let Some(p) = decoded
                        && self.up
                    {
                        self.pictures.insert(if key == "event" { "event" } else { "still" }, p);
                        self.revision += 1;
                    }
                    self.planner.arrived(*tag, *status, ok, now);
                    out.push(Effect::Redraw);
                }
            }
            Input::Fetched { tag, status, data } if Live::owns(*tag) => {
                if self.up {
                    out.extend(self.live.answered(*tag, *status, data));
                }
            }
            Input::Fetched { tag, status, .. } if (FIRST..=LAST).contains(tag) => {
                // a refusal answered as a document (404 no_picture, 503, 429)
                if self.planner.arrived(*tag, *status, false, now).is_some() {
                    out.push(Effect::Redraw);
                }
            }
            Input::Tick => {
                if self.up && self.ends_at.is_some_and(|t| now >= t) {
                    self.hide(true, false, &mut out);
                }
                if self.up {
                    out.extend(self.live.tick(now));
                }
                if self.ok.tick(now)
                    && let Some(c) = self.current.as_ref().filter(|_| self.up)
                {
                    self.intents.push(Intent::Snooze { camera: c.camera.clone(), name: c.title() });
                }
                if self.up && self.ago_at.is_some_and(|t| now >= t) {
                    self.ago_at = Some(now + AGO_MS);
                    out.push(Effect::Redraw);
                }
                if self.waiting_at.is_some_and(|t| now >= t) {
                    self.waiting_at = None;
                    self.consider(now, &mut out);
                    // still waiting: look again
                    if !self.up && self.waiting_at.is_none() && self.due(now).is_some() {
                        self.waiting_at = Some(now + WAIT_MS);
                    }
                }
            }
            Input::Key { name, down: true, repeat: false } if self.armed(now) => match name.as_str() {
                // where a hold snoozes, OK is decided on its release
                "ok" if self.can_snooze => self.ok.press(now, HOLD_MS),
                "ok" => self.ok(&mut out),
                "back" => self.hide(true, false, &mut out),
                _ => {}
            },
            Input::Key { name, down: false, .. } if name == "ok" && self.ok.release() == Released::Tap && self.up => self.ok(&mut out),
            _ => {}
        }
        if self.up {
            self.settle(now, &mut out);
        }
        out
    }

    pub fn due_at(&self) -> Option<u64> {
        let pics = if self.up { self.planner.due_at(&self.wants) } else { None };
        let arm = if self.up && !self.armed(self.clock) { Some(self.arrived_at + ARM_MS) } else { None };
        let live = if self.up { self.live.due_at(self.clock) } else { None };
        // the stills turn quick when no window was answered in time
        let quick = (self.up && self.current.as_ref().is_some_and(|c| c.live == Some(true)) && self.live.window() == Window::Unclaimed && !self.stills_quick(self.clock))
            .then_some(self.arrived_at + QUICK_AFTER_MS);
        [self.ends_at, self.waiting_at, self.ago_at, pics, arm, live, quick, self.ok.due_at()].into_iter().flatten().min()
    }

    pub fn state(&self) -> Value {
        match &self.current {
            Some(c) if self.up => json!({"up": true, "camera": c.camera, "kind": c.kind, "id": c.id, "armed": self.armed(self.clock), "live": self.live.state(self.clock)}),
            _ => json!({"up": false}),
        }
    }

    /// When it ends, for the tests.
    pub fn ends_at(&self) -> Option<u64> {
        self.ends_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_800_000_000_000;

    fn record(id: i64, kind: &str, until: u64) -> Input {
        Input::Helper {
            topic: "cameras.popup".into(),
            data: json!({"id": id, "event": format!("e{id}"), "camera": "275", "name": "Front door", "kind": kind, "at": T0, "untilMs": until, "thumbnail": true}),
        }
    }

    fn key(name: &str) -> Input {
        Input::Key { name: name.into(), down: true, repeat: false }
    }

    fn posted(fx: &[Effect]) -> Vec<String> {
        fx.iter().filter_map(|e| if let Effect::Post { path, .. } = e { Some(path.clone()) } else { None }).collect()
    }

    fn fetches(fx: &[Effect]) -> Vec<(u64, String)> {
        fx.iter().filter_map(|e| if let Effect::Fetch { tag, path } = e { Some((*tag, path.clone())) } else { None }).collect()
    }

    fn bmp() -> Vec<u8> {
        let (w, h) = (2u32, 2u32);
        let stride = (w * 3).div_ceil(4) * 4;
        let mut b = vec![0u8; 54];
        b[0..2].copy_from_slice(b"BM");
        b[2..6].copy_from_slice(&(54 + stride * h).to_le_bytes());
        b[10..14].copy_from_slice(&54u32.to_le_bytes());
        b[14..18].copy_from_slice(&40u32.to_le_bytes());
        b[18..22].copy_from_slice(&w.to_le_bytes());
        b[22..26].copy_from_slice(&h.to_le_bytes());
        b[26..28].copy_from_slice(&1u16.to_le_bytes());
        b[28..30].copy_from_slice(&24u16.to_le_bytes());
        for _ in 0..h {
            for _ in 0..w {
                b.extend_from_slice(&[50, 100, 200]);
            }
            b.extend(std::iter::repeat_n(0u8, (stride - w * 3) as usize));
        }
        b
    }

    #[test]
    fn a_due_record_shows_a_card_that_ends_at_its_time_and_tells_the_helper() {
        let mut p = Popup::new();
        p.update(&record(41, "person", T0 + 10_000), T0);
        assert!(p.up());
        assert_eq!(p.take_intents(), vec![Intent::Shown("person".into())]);
        assert_eq!(p.ends_at(), Some(T0 + 10_000));
        let fx = p.update(&Input::Tick, T0 + 9_999);
        assert!(p.up() && posted(&fx).is_empty());
        let fx = p.update(&Input::Tick, T0 + 10_000);
        assert!(!p.up());
        assert_eq!(posted(&fx), vec!["/v1/cameras/popup/clear".to_string()]);
        // the same record read again is not a new pop-up
        p.update(&record(41, "person", T0 + 10_000), T0 + 10_001);
        assert!(!p.up());
    }

    #[test]
    fn the_card_is_shaped_by_the_cameras_size_and_then_by_its_stream_as_decoded() {
        let mut p = Popup::new();
        let cams = json!({"cameras": [{"id": "275", "native": {"codec": "h264", "width": 1536, "height": 2048, "fps": 15}}, {"id": "276"}]});
        p.update(&Input::Helper { topic: "cameras".into(), data: cams }, T0);
        p.update(&record(41, "person", T0 + 10_000), T0);
        assert_eq!(p.view().shape, Some((1536, 2048)), "the camera's known size before the stream says one");
        let live = json!({"camera": "275", "mode": "live", "state": "playing", "width": 480, "height": 640, "fps": 15});
        let fx = p.update(&Input::Helper { topic: "cameras.live".into(), data: live }, T0 + 100);
        assert_eq!(p.view().shape, Some((480, 640)), "the stream as decoded");
        assert!(fx.contains(&Effect::Redraw));
        // another camera's stream says nothing of this card's shape
        let other = json!({"camera": "276", "mode": "live", "state": "playing", "width": 1920, "height": 1080, "fps": 15});
        p.update(&Input::Helper { topic: "cameras.live".into(), data: other }, T0 + 200);
        assert_eq!(p.view().shape, Some((1536, 2048)));
    }

    #[test]
    fn a_record_out_of_date_is_not_shown_and_a_doorbell_stays_half_as_long_again() {
        let mut p = Popup::new();
        p.update(&record(1, "person", T0 - 1), T0);
        assert!(!p.up());
        p.update(&record(2, "doorbell", T0 + 10_000), T0);
        assert_eq!(p.ends_at(), Some(T0 + 15_000));
    }

    #[test]
    fn it_waits_while_the_host_is_busy_and_shows_when_free_if_still_in_date() {
        let mut p = Popup::new();
        p.set_context(true, false, false, T0);
        p.update(&record(3, "person", T0 + 10_000), T0);
        assert!(!p.up());
        assert_eq!(p.due_at(), Some(T0 + 500), "looks again every 500 ms");
        p.update(&Input::Tick, T0 + 500);
        assert!(!p.up(), "still busy");
        assert_eq!(p.due_at(), Some(T0 + 1_000));
        p.set_context(false, false, false, T0 + 2_000);
        assert!(p.up(), "free again");
        assert_eq!(p.ends_at(), Some(T0 + 10_000), "with the time it had");
        // it is not shown at all once its time is over
        let mut q = Popup::new();
        q.set_context(true, false, false, T0);
        q.update(&record(4, "person", T0 + 2_000), T0);
        q.set_context(false, false, false, T0 + 3_000);
        assert!(!q.up());
    }

    #[test]
    fn one_that_is_up_when_the_host_turns_busy_steps_aside_and_comes_back_with_its_time() {
        let mut p = Popup::new();
        p.update(&record(5, "person", T0 + 20_000), T0);
        assert!(p.up());
        let fx = p.set_context(true, false, false, T0 + 1_000);
        assert!(!p.up() && posted(&fx).is_empty(), "stepped aside, the helper is not told");
        p.set_context(false, false, false, T0 + 4_000);
        assert!(p.up());
        assert_eq!(p.ends_at(), Some(T0 + 20_000));
    }

    #[test]
    fn with_over_apps_off_it_stays_off_an_app_and_shows_on_home() {
        let mut p = Popup::new();
        p.set_context(false, true, false, T0);
        p.update(
            &Input::Helper { topic: "cameras.popup".into(), data: json!({"id": 6, "event": "e6", "camera": "275", "kind": "person", "at": T0, "untilMs": T0 + 30_000, "overApps": false}) },
            T0,
        );
        assert!(!p.up(), "an app is in front");
        p.set_context(false, false, false, T0 + 5_000);
        assert!(p.up(), "Home: it shows, still in date");
        p.set_context(false, true, false, T0 + 6_000);
        assert!(!p.up(), "an app comes to the front: it steps aside");
    }

    #[test]
    fn an_update_of_the_story_changes_the_words_and_the_end_and_is_never_a_second_card() {
        let mut p = Popup::new();
        p.update(&record(41, "person", T0 + 10_000), T0);
        p.take_intents();
        p.update(
            &Input::Helper {
                topic: "cameras.popup".into(),
                data: json!({"id": 41, "event": "e41", "camera": "275", "name": "Front door", "kind": "animal", "label": "Domestic Cat", "at": T0, "untilMs": T0 + 14_000}),
            },
            T0 + 2_000,
        );
        assert!(p.up());
        assert!(p.take_intents().is_empty(), "not shown again");
        assert_eq!(p.ends_at(), Some(T0 + 14_000));
        assert_eq!(p.view().what, "Animal  \u{b7}  Domestic Cat");
    }

    #[test]
    fn a_record_cleared_elsewhere_takes_the_card_away_without_telling_the_helper() {
        let mut p = Popup::new();
        p.update(&record(7, "person", T0 + 10_000), T0);
        let fx = p.update(&Input::Helper { topic: "cameras.popup".into(), data: Value::Null }, T0 + 1_000);
        assert!(!p.up() && posted(&fx).is_empty());
    }

    #[test]
    fn the_keys_are_the_cards_only_after_600_ms_ok_clears_and_asks_for_the_full_view_back_clears() {
        let mut p = Popup::new();
        p.set_can_expand(true);
        p.update(&record(8, "person", T0 + 30_000), T0);
        p.take_intents();
        assert!(!p.armed(T0 + 599));
        p.update(&key("ok"), T0 + 599);
        assert!(p.up(), "a press made as it came was the app's");
        assert!(p.armed(T0 + 600));
        let fx = p.update(&key("ok"), T0 + 700);
        assert!(!p.up());
        assert_eq!(posted(&fx), vec!["/v1/cameras/popup/clear".to_string()]);
        assert_eq!(p.take_intents(), vec![Intent::Expand("275".into())]);
        p.update(&record(9, "person", T0 + 30_000), T0 + 800);
        p.take_intents();
        let fx = p.update(&key("back"), T0 + 1_500);
        assert!(!p.up() && posted(&fx).len() == 1 && p.take_intents().is_empty());
        // another key is the app's: the arrows never reach it
        p.update(&record(10, "person", T0 + 30_000), T0 + 2_000);
        p.update(&key("left"), T0 + 3_000);
        assert!(p.up());
    }

    #[test]
    fn ok_with_no_viewer_to_open_only_clears() {
        let mut p = Popup::new();
        p.update(&record(11, "person", T0 + 30_000), T0);
        p.take_intents();
        p.update(&key("ok"), T0 + 700);
        assert!(!p.up());
        assert!(p.take_intents().is_empty());
        assert!(!p.view().can_expand);
    }

    #[test]
    fn the_viewer_being_up_means_no_card_and_the_record_is_not_due_again() {
        let mut p = Popup::new();
        p.set_context(false, false, true, T0);
        p.update(&record(12, "person", T0 + 30_000), T0);
        assert!(!p.up());
        p.set_context(false, false, false, T0 + 1_000);
        assert!(!p.up(), "the full view turned to it: it was seen");
    }

    #[test]
    fn the_picture_box_says_connecting_then_the_event_picture_then_live_and_no_picture_after_two_refusals() {
        let mut p = Popup::new();
        let fx = p.update(&record(13, "person", T0 + 30_000), T0);
        let asked = fetches(&fx);
        assert_eq!(asked.len(), 2, "the still and the event's own picture");
        let still = asked.iter().find(|f| f.1 == "/v1/cameras/275/frame?w=520&h=292").expect("the still at the spec's size").0;
        let event = asked.iter().find(|f| f.1 == "/v1/cameras/events/e13/thumbnail?w=520&h=292").expect("the event's picture").0;
        assert_eq!(p.view().face, Face::Connecting);
        p.update(&Input::Bytes { tag: event, status: 200, bytes: bmp() }, T0 + 100);
        assert_eq!(p.view().face, Face::EventPicture);
        p.update(&Input::Bytes { tag: still, status: 200, bytes: bmp() }, T0 + 200);
        assert_eq!(p.view().face, Face::Live);
        assert!(p.best().is_some());
        // the next still is asked 250 ms after this one arrived
        assert_eq!(p.due_at(), Some(T0 + 450));
        let fx = p.update(&Input::Tick, T0 + 450);
        assert!(fetches(&fx).iter().any(|(_, path)| path.contains("frame")));
        // a refusal with none shown
        let mut q = Popup::new();
        let fx = q.update(&record(14, "person", T0 + 30_000), T0);
        let tag = fetches(&fx).into_iter().find(|f| f.1.contains("frame")).unwrap().0;
        q.update(&Input::Fetched { tag, status: 404, data: json!({"error": "no_picture"}) }, T0 + 100);
        assert_eq!(q.view().face, Face::Connecting, "one refusal is not yet an answer");
        let fx = q.update(&Input::Tick, T0 + 1_000);
        let tag = fetches(&fx).into_iter().find(|f| f.1.contains("frame")).unwrap().0;
        q.update(&Input::Fetched { tag, status: 404, data: Value::Null }, T0 + 1_100);
        assert_eq!(q.view().face, Face::NoPicture);
    }

    #[test]
    fn the_age_follows_the_bridges_clock() {
        let mut p = Popup::new();
        p.update(&Input::Helper { topic: "cameras".into(), data: json!({"now_skew_ms": 5_000}) }, T0);
        p.update(&record(15, "person", T0 + 600_000), T0);
        assert_eq!(p.view().age, "Just now");
        p.update(&Input::Tick, T0 + 125_000);
        assert_eq!(p.view().age, "2 min ago");
    }

    const BOX: (i32, i32, i32, i32) = (1316, 200, 520, 292);

    fn live_record(id: i64, live: bool) -> Input {
        Input::Helper {
            topic: "cameras.popup".into(),
            data: json!({"id": id, "event": format!("e{id}"), "camera": "275", "name": "Cat Feeder", "kind": "animal", "at": T0, "untilMs": T0 + 60_000, "thumbnail": true, "live": live}),
        }
    }

    fn live_asks(fx: &[Effect]) -> Vec<Value> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Request { path, body, .. } if path == "/v1/cameras/live" => body.clone(),
                _ => None,
            })
            .collect()
    }

    fn stopped(fx: &[Effect]) -> usize {
        posted(fx).iter().filter(|p| *p == "/v1/cameras/live/stop").count()
    }

    fn stream(state: &str) -> Input {
        Input::Helper { topic: "cameras.live".into(), data: json!({"camera": "275", "mode": "live", "state": state, "fps": 15, "fedFps": 14.9}) }
    }

    fn still_asks(fx: &[Effect]) -> Vec<String> {
        fetches(fx).into_iter().map(|f| f.1).filter(|p| p.contains("/frame?")).collect()
    }

    #[test]
    fn a_card_that_can_play_live_asks_the_compositor_for_a_window_and_only_then_the_helper_for_the_stream() {
        let mut p = Popup::new();
        p.set_context(false, true, false, T0);
        p.set_picture_rect(Some(BOX));
        let fx = p.update(&live_record(41, true), T0);
        assert!(p.up() && live_asks(&fx).is_empty(), "no stream is asked for before a window is granted: {fx:?}");
        let claim = p.claim().expect("a card the record says can play live claims a window");
        assert_eq!((claim["id"].as_str(), claim["kind"].as_str()), (Some("camera-popup"), Some("camera-popup")));
        assert_eq!(claim["rect"], json!({"x": 1316, "y": 200, "w": 520, "h": 292}));
        // the stills are the picture meanwhile, at the pace of a card that waits
        assert_eq!(still_asks(&fx), vec!["/v1/cameras/275/frame?w=520&h=292".to_string()]);

        let fx = p.set_window(Window::Waiting, "main", false, T0 + 60);
        assert_eq!(live_asks(&fx), vec![json!({"camera": "275", "for": "popup", "window": "main", "popup": 41})]);
        let fx = p.update(&stream("starting"), T0 + 300);
        assert!(live_asks(&fx).is_empty() && !p.view().video);
        p.update(&stream("playing"), T0 + 700);
        p.set_window(Window::Carried, "main", true, T0 + 710);
        assert!(!p.view().video, "the stills stay under the hole for the handoff");
        assert_eq!(p.due_at().map(|t| t <= T0 + 700 + super::super::popup_live::HANDOFF_MS), Some(true), "the handoff is due");
        let fx = p.update(&Input::Tick, T0 + 700 + super::super::popup_live::HANDOFF_MS);
        assert!(p.view().video, "the picture box is a hole now");
        assert_eq!(p.view().face, Face::Live);
        assert!(fx.iter().any(|e| matches!(e, Effect::Redraw)));
        // the stills are asked for no more while the video is the picture
        let fx = p.update(&Input::Tick, T0 + 5_000);
        assert!(still_asks(&fx).is_empty(), "{fx:?}");
        assert_eq!(p.state()["live"]["video"], true);
    }

    #[test]
    fn a_record_that_says_texture_only_claims_a_texture_and_the_texture_is_asked_for_the_low_stream() {
        let mut p = Popup::new();
        p.set_context(false, true, false, T0);
        p.set_picture_rect(Some(BOX));
        let Input::Helper { topic, mut data } = live_record(45, true) else { unreachable!() };
        data["textureOnly"] = json!(true);
        p.update(&Input::Helper { topic, data }, T0);
        assert_eq!(p.claim().expect("a card that can play live claims")["texture_only"], true);
        let fx = p.set_window(Window::Waiting, "texture", false, T0 + 60);
        assert_eq!(live_asks(&fx), vec![json!({"camera": "275", "for": "popup", "window": "texture", "popup": 45})]);
        // a record that does not say it is a window claim, as before
        let mut q = Popup::new();
        q.set_context(false, true, false, T0);
        q.set_picture_rect(Some(BOX));
        q.update(&live_record(46, true), T0);
        assert!(q.claim().unwrap().get("texture_only").is_none());
        // with Home in front it is a window claim too
        let mut h = Popup::new();
        h.set_picture_rect(Some(BOX));
        h.update(&live_record(47, true), T0);
        assert!(h.claim().unwrap().get("texture_only").is_none());
    }

    #[test]
    fn a_window_that_is_refused_leaves_the_stills_and_makes_them_quicker() {
        let mut p = Popup::new();
        p.set_picture_rect(Some(BOX));
        p.update(&live_record(42, true), T0);
        let fx = p.set_window(Window::Refused, "", false, T0 + 50);
        assert!(live_asks(&fx).is_empty() && stopped(&fx) == 0, "nothing was started, nothing is stopped");
        // the stills are asked at the quicker pace, of the helper's quicker stream
        assert!(still_asks(&fx).iter().any(|path| path == "/v1/cameras/275/frame?w=520&h=292&fps=8"), "{fx:?}");
        let paths: Vec<String> = p.wants.iter().map(|w| w.path.clone()).collect();
        assert!(paths.iter().any(|path| path == "/v1/cameras/275/frame?w=520&h=292&fps=8"), "{paths:?}");
        assert_eq!(p.wants.iter().find(|w| w.key == "still").map(|w| w.every_ms), Some(QUICK_MS));
        // and a window the compositor grants later (the app's video ended) starts the stream
        let fx = p.set_window(Window::Waiting, "sub", false, T0 + 4_000);
        assert_eq!(live_asks(&fx).len(), 1);
    }

    #[test]
    fn a_card_the_helper_says_has_no_live_video_claims_nothing_and_has_quick_stills() {
        let mut p = Popup::new();
        p.set_picture_rect(Some(BOX));
        let fx = p.update(&live_record(43, false), T0);
        assert!(p.claim().is_none());
        assert_eq!(still_asks(&fx), vec!["/v1/cameras/275/frame?w=520&h=292&fps=8".to_string()]);
        // an older helper says nothing: the stills are as they were, and nothing is claimed
        let mut q = Popup::new();
        q.set_picture_rect(Some(BOX));
        let fx = q.update(&record(44, "person", T0 + 60_000), T0);
        assert!(q.claim().is_none());
        assert_eq!(still_asks(&fx), vec!["/v1/cameras/275/frame?w=520&h=292".to_string()]);
    }

    #[test]
    fn the_stream_is_stopped_when_the_card_goes_unless_the_full_view_takes_it_over() {
        let go = |keys: &[&str]| {
            let mut p = Popup::new();
            p.set_can_expand(true);
            p.set_picture_rect(Some(BOX));
            p.update(&live_record(45, true), T0);
            p.take_intents();
            p.set_window(Window::Carried, "main", true, T0 + 50);
            p.update(&stream("playing"), T0 + 400);
            let mut fx = Vec::new();
            for k in keys {
                fx.extend(p.update(&key(k), T0 + 2_000));
            }
            fx
        };
        assert_eq!(stopped(&go(&["back"])), 1, "Back dismisses the card and ends its stream");
        assert_eq!(stopped(&go(&["ok"])), 0, "OK opens the full view of the same camera, which takes the stream over");
        // the time being up, and the card stepping aside, end it too
        let mut p = Popup::new();
        p.set_picture_rect(Some(BOX));
        p.update(&live_record(46, true), T0);
        p.set_window(Window::Carried, "main", true, T0 + 50);
        let fx = p.update(&Input::Tick, T0 + 60_000);
        assert!(!p.up() && stopped(&fx) == 1, "{fx:?}");
        let mut q = Popup::new();
        q.set_picture_rect(Some(BOX));
        q.update(&live_record(47, true), T0);
        q.set_window(Window::Carried, "main", true, T0 + 50);
        let fx = q.set_context(true, false, false, T0 + 1_000);
        assert!(!q.up() && stopped(&fx) == 1, "an alert came up over it");
    }

    #[test]
    fn a_window_taken_back_while_the_stream_plays_ends_it_and_brings_the_stills_back() {
        let mut p = Popup::new();
        p.set_picture_rect(Some(BOX));
        p.update(&live_record(48, true), T0);
        p.set_window(Window::Carried, "main", true, T0 + 50);
        p.update(&stream("playing"), T0 + 300);
        p.update(&Input::Tick, T0 + 600);
        assert!(p.view().video);
        let fx = p.set_window(Window::Refused, "main", false, T0 + 5_000);
        assert_eq!(stopped(&fx), 1);
        assert!(!p.view().video, "the hole closes with the claim: the picture box shows the shell's own picture again");
        let fx = p.update(&Input::Tick, T0 + 5_100);
        assert!(!still_asks(&fx).is_empty() || p.wants.iter().any(|w| w.key == "still"), "the stills are asked for again");
    }

    #[test]
    fn ok_hands_the_cards_picture_to_the_full_view_once_and_back_hands_nothing() {
        let go = |name: &str| {
            let mut p = Popup::new();
            p.set_can_expand(true);
            let fx = p.update(&record(60, "person", T0 + 30_000), T0);
            let still = fetches(&fx).into_iter().find(|f| f.1.contains("frame")).unwrap().0;
            p.update(&Input::Bytes { tag: still, status: 200, bytes: bmp() }, T0 + 200);
            p.take_intents();
            p.update(&key(name), T0 + 900);
            p
        };
        let mut p = go("ok");
        assert_eq!(p.take_intents(), vec![Intent::Expand("275".into())]);
        let pic = p.take_expanded().expect("the picture the card had");
        assert!(pic.width > 0 && pic.height > 0);
        assert!(p.take_expanded().is_none(), "once");
        let mut q = go("back");
        assert!(q.take_expanded().is_none() && q.take_intents().is_empty());
    }

    #[test]
    fn stills_over_an_app_that_holds_both_scalers_keep_quiet_and_over_any_other_refusal_are_quick() {
        let heavy = |why: &str| {
            let mut p = Popup::new();
            p.set_picture_rect(Some(BOX));
            p.update(&live_record(61, true), T0);
            p.set_window(Window::Refused, why, false, T0 + 50);
            p.wants.iter().find(|w| w.key == "still").map(|w| (w.every_ms, w.path.clone())).unwrap()
        };
        assert_eq!(heavy("both-scalers"), (QUIET_MS, "/v1/cameras/275/frame?w=520&h=292&fps=4".to_string()), "a game at 120 Hz or 4K video: a quarter of the frames of a second, not more");
        assert_eq!(heavy("app-video"), (QUICK_MS, "/v1/cameras/275/frame?w=520&h=292&fps=8".to_string()));
    }

    #[test]
    fn where_a_hold_snoozes_ok_acts_on_its_release_and_a_held_ok_asks_for_the_sheet() {
        let up = |name: &str| Input::Key { name: name.into(), down: false, repeat: false };
        let mut p = Popup::new();
        p.set_can_expand(true);
        p.set_can_snooze(true);
        p.update(&record(70, "person", T0 + 30_000), T0);
        p.take_intents();
        p.update(&key("ok"), T0 + 700);
        assert!(p.up() && p.due_at() <= Some(T0 + 700 + HOLD_MS), "the press only starts the hold");
        p.update(&Input::Tick, T0 + 700 + HOLD_MS - 1);
        p.update(&up("ok"), T0 + 800);
        assert!(!p.up());
        assert_eq!(p.take_intents(), vec![Intent::Expand("275".into())], "a tap is OK");
        p.update(&record(71, "person", T0 + 30_000), T0 + 1_000);
        p.take_intents();
        p.update(&key("ok"), T0 + 2_000);
        p.update(&Input::Tick, T0 + 2_000 + HOLD_MS);
        assert_eq!(p.take_intents(), vec![Intent::Snooze { camera: "275".into(), name: "Front door".into() }]);
        let fx = p.update(&up("ok"), T0 + 2_900);
        assert!(p.up() && posted(&fx).is_empty() && p.take_intents().is_empty(), "the release of the hold is nothing");
        // the sheet is up: the card steps aside, and a snooze of another camera leaves it due
        p.set_context(true, false, false, T0 + 3_000);
        assert!(!p.up());
        assert!(p.snoozed(Some("276")).is_empty());
        assert_eq!(posted(&p.snoozed(Some("275"))), vec!["/v1/cameras/popup/clear".to_string()]);
        p.set_context(false, false, false, T0 + 3_500);
        assert!(!p.up(), "a snoozed card does not come back");
    }
}
