//! The camera pop-up's model (the pop-up of `panels/Cameras.qml`, `docs/research/cameras-ui-spec.md` "The pop-up"): a detector of a shared camera
//! saw something (`cameras.popup`: the camera, the kind, when) and the card says so for a while, with the camera's stills in it. Stills
//! only: nothing of an app's playback is touched, and a live picture on a hardware video window is for later.
//!
//! - **Due** when its `id` differs from the one shown and `untilMs` is ahead. It waits (looking again every 500 ms) while the host is busy
//!   (a PIN, an alert, a panel, the keyboard) and, with `overApps` false, while an app is in front; it shows when free if still in date.
//!   One that is up when the host turns busy, or an app comes to the front with `overApps` false, steps aside, is not dismissed and
//!   returns with the time it has left. An `update` of the same story changes the words of the card up and re-reads `untilMs`; it is never
//!   a second pop-up. It ends at `untilMs` and tells the helper (`/v1/cameras/popup/clear`).
//! - **Keys:** only OK and Back, and only 600 ms after it arrived (a press made as it came was the app's). OK clears it (the helper is told)
//!   and asks for the full view of its camera; Back clears it. While the viewer is up there is no card.
//! - **Picture:** the event's own (`/v1/cameras/events/<event>/thumbnail`, once) at once, then the camera's stills (`.../frame`), the next ask 250 ms after
//!   the last arrived. `Live` once a still is on screen; `Event picture` while only the event's is; `Connecting` before any; `No picture`
//!   after two refused still asks with none shown.
//! - A doorbell stays half as long again and tells the overlay queue it showed, which folds the plugin's record of the same ring into the card.

use crate::cameras::pics::{Planner, Want};
use crate::cameras::sheet::encode;
use crate::kit::picture::{self, Picture};
use crate::kit::screen::{Effect, Input};
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
/// The tags of the pictures' asks.
const FIRST: u64 = 0x5359_1000;
const LAST: u64 = 0x5359_1FFF;
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
            camera,
        })
    }

    fn blocked_over_app(&self) -> bool {
        self.over_apps == Some(false)
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
    /// Whether the keys are the card's yet.
    pub armed: bool,
}

/// What the card asks of the shell besides effects.
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// OK: the full view of this camera.
    Expand(String),
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
    planner: Planner,
    size: (u32, u32),
    wants: Vec<Want>,
    pictures: HashMap<&'static str, Picture>,
    /// Bumped each time a picture changed, for the surface.
    pub revision: u64,
    intents: Vec<Intent>,
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
            planner: Planner::with_tags(FIRST, LAST),
            size: PICTURE,
            wants: Vec::new(),
            pictures: HashMap::new(),
            revision: 0,
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
            self.hold(viewer_up);
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

    pub fn view(&self) -> View {
        let (camera, name, what, age, icon) = match &self.current {
            Some(r) => {
                let age = ago(((self.clock as i64 + self.skew_ms - r.at) / 1000).max(0));
                (r.camera.clone(), if r.name.is_empty() { r.camera.clone() } else { r.name.clone() }, words_of(r), age, icon_of(&r.kind))
            }
            None => (String::new(), String::new(), String::new(), String::new(), "camera"),
        };
        let face = if self.pictures.contains_key("still") {
            Face::Live
        } else if self.pictures.contains_key("event") {
            Face::EventPicture
        } else if self.planner.pic("still").refused > 1 {
            Face::NoPicture
        } else {
            Face::Connecting
        };
        View { up: self.up, camera, name, icon, what, age, face, can_expand: self.can_expand, armed: self.armed(self.clock) }
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
        self.current = Some(t);
        self.up = true;
        self.pictures.clear();
        self.revision += 1;
        self.wants.clear();
        out.push(Effect::Redraw);
        self.plan(now, out);
    }

    /// The card steps aside: not dismissed, due again when the host is free if its time is not over. `viewer`: the full view took the camera, so
    /// it is not due again.
    fn hold(&mut self, viewer: bool) {
        self.up = false;
        self.ends_at = None;
        self.ago_at = None;
        if !viewer {
            self.shown_id = None;
        }
        self.current = None;
        self.wants.clear();
        self.planner.quiesce();
    }

    fn hide(&mut self, tell: bool, out: &mut Vec<Effect>) {
        if !self.up {
            return;
        }
        self.up = false;
        self.ends_at = None;
        self.ago_at = None;
        self.current = None;
        self.wants.clear();
        self.planner.quiesce();
        if tell {
            out.push(Effect::Post { path: "/v1/cameras/popup/clear".into(), body: json!({}) });
        }
        out.push(Effect::Redraw);
    }

    fn plan(&mut self, now: u64, out: &mut Vec<Effect>) {
        if let Some(c) = &self.current
            && self.up
        {
            let cam = encode(&c.camera);
            let mut wants = vec![Want::repeating("still", &format!("/v1/cameras/{cam}/frame?w={}&h={}", self.size.0, self.size.1), STILL_MS)];
            if c.thumbnail && !c.event.is_empty() {
                wants.push(Want::once("event", &format!("/v1/cameras/events/{}/thumbnail?w={}&h={}", encode(&c.event), self.size.0, self.size.1)));
            }
            self.wants = wants;
            out.extend(self.planner.plan(&self.wants, now, 2));
        }
    }

    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        self.clock = now;
        let mut out = Vec::new();
        match input {
            Input::Helper { topic, data } if topic == "cameras" => {
                self.skew_ms = data.get("now_skew_ms").and_then(Value::as_i64).unwrap_or(0);
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
                        _ => self.hide(false, &mut out),
                    }
                }
                self.consider(now, &mut out);
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
            Input::Fetched { tag, status, .. } if (FIRST..=LAST).contains(tag) => {
                // a refusal answered as a document (404 no_picture, 503, 429)
                if self.planner.arrived(*tag, *status, false, now).is_some() {
                    out.push(Effect::Redraw);
                }
            }
            Input::Tick => {
                if self.up && self.ends_at.is_some_and(|t| now >= t) {
                    self.hide(true, &mut out);
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
                "ok" => {
                    let camera = self.current.as_ref().map(|c| c.camera.clone()).unwrap_or_default();
                    self.hide(true, &mut out);
                    if self.can_expand {
                        self.intents.push(Intent::Expand(camera));
                    }
                }
                "back" => self.hide(true, &mut out),
                _ => {}
            },
            _ => {}
        }
        if self.up {
            self.plan(now, &mut out);
        }
        out
    }

    pub fn due_at(&self) -> Option<u64> {
        let pics = if self.up { self.planner.due_at(&self.wants) } else { None };
        let arm = if self.up && !self.armed(self.clock) { Some(self.arrived_at + ARM_MS) } else { None };
        [self.ends_at, self.waiting_at, self.ago_at, pics, arm].into_iter().flatten().min()
    }

    pub fn state(&self) -> Value {
        match &self.current {
            Some(c) if self.up => json!({"up": true, "camera": c.camera, "kind": c.kind, "id": c.id, "armed": self.armed(self.clock)}),
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
}
