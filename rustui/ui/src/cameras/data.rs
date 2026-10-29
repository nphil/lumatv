//! What the Cameras screens read, as typed values: the helper's topics (`cameras`, `cameras.popup`, `cameras.live`, `cameras.events`)
//! and the answers of the timeline routes and the settings (`docs/research/cameras-ui-spec.md`, "Data the screens read"; the
//! contracts of `scrypted-cameras-design.md` 3.4). Parsing is forgiving of what is absent and strict about nothing it does not use:
//! a member the helper leaves out is its default, a record that is not an object is nothing.

use serde_json::Value;
use std::collections::BTreeMap;

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn int(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)))
}

fn flag(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

/// The state of the link to Scrypted (`cameras.state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LinkState {
    #[default]
    Unlinked,
    Connecting,
    Ready,
    Offline,
    Refused,
    Untrusted,
}

impl LinkState {
    pub fn parse(s: &str) -> Self {
        match s {
            "connecting" => Self::Connecting,
            "ready" => Self::Ready,
            "offline" => Self::Offline,
            "refused" => Self::Refused,
            "untrusted" => Self::Untrusted,
            _ => Self::Unlinked,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Unlinked => "unlinked",
            Self::Connecting => "connecting",
            Self::Ready => "ready",
            Self::Offline => "offline",
            Self::Refused => "refused",
            Self::Untrusted => "untrusted",
        }
    }
}

/// What a camera's main stream is, until a stream says what it plays (`native`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Native {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// One camera of the `cameras` topic, already ordered and filtered by what Settings shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Camera {
    pub id: String,
    pub name: String,
    pub room: String,
    pub online: bool,
    pub live: bool,
    pub recordings: bool,
    pub events: bool,
    pub clips: bool,
    /// The first moment of footage (epoch ms), when the bridge knows it.
    pub since: Option<i64>,
    pub native: Option<Native>,
    pub prebuffered: bool,
}

impl Camera {
    pub fn parse(v: &Value) -> Option<Self> {
        let id = match v.get("id") {
            Some(Value::String(s)) => s.clone(),
            Some(n) if n.is_number() => n.to_string(),
            _ => return None,
        };
        let native = v.get("native").filter(|n| n.is_object()).map(|n| Native {
            codec: text(n, "codec"),
            width: int(n, "width").unwrap_or(0) as u32,
            height: int(n, "height").unwrap_or(0) as u32,
            fps: int(n, "fps").unwrap_or(0) as u32,
        });
        let name = text(v, "name");
        Some(Self {
            name: if name.is_empty() { id.clone() } else { name },
            id,
            room: text(v, "room"),
            online: flag(v, "online", true),
            live: flag(v, "live", true),
            recordings: flag(v, "recordings", false),
            events: flag(v, "events", false),
            clips: flag(v, "clips", false),
            since: int(v, "since").filter(|s| *s > 0),
            native,
            prebuffered: flag(v, "prebuffered", false),
        })
    }
}

/// The `cameras` topic.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cameras {
    pub state: LinkState,
    pub reason: String,
    /// The bridge's clock minus this TV's, in milliseconds.
    pub skew_ms: i64,
    pub rev: i64,
    pub list: Vec<Camera>,
}

impl Cameras {
    pub fn parse(v: &Value) -> Self {
        Self {
            state: LinkState::parse(v.get("state").and_then(Value::as_str).unwrap_or("")),
            reason: text(v, "reason"),
            skew_ms: int(v, "now_skew_ms").unwrap_or(0),
            rev: int(v, "rev").unwrap_or(0),
            list: v.get("cameras").and_then(Value::as_array).map(|a| a.iter().filter_map(Camera::parse).collect()).unwrap_or_default(),
        }
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.list.iter().position(|c| c.id == id)
    }

    pub fn get(&self, id: &str) -> Option<&Camera> {
        self.list.iter().find(|c| c.id == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LiveMode {
    #[default]
    Live,
    Playback,
    Clip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LiveState {
    #[default]
    Idle,
    Asking,
    Starting,
    Playing,
    Failed,
    Stopped,
}

/// The `cameras.live` topic: the one session on the video plane.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub camera: String,
    pub mode: LiveMode,
    pub state: LiveState,
    pub reason: String,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// The wall-clock moment of the picture on the plane in a playback (4 updates a second); `None` in live.
    pub position_ms: Option<i64>,
    pub started_at: Option<i64>,
    pub rate: f32,
    pub paused: bool,
}

impl Live {
    pub fn parse(v: &Value) -> Self {
        if !v.is_object() {
            return Self { rate: 1.0, ..Self::default() };
        }
        Self {
            camera: match v.get("camera") {
                Some(Value::String(s)) => s.clone(),
                Some(n) if n.is_number() => n.to_string(),
                _ => String::new(),
            },
            mode: match v.get("mode").and_then(Value::as_str) {
                Some("playback") => LiveMode::Playback,
                Some("clip") => LiveMode::Clip,
                _ => LiveMode::Live,
            },
            state: match v.get("state").and_then(Value::as_str) {
                Some("asking") => LiveState::Asking,
                Some("starting") => LiveState::Starting,
                Some("playing") => LiveState::Playing,
                Some("failed") => LiveState::Failed,
                Some("stopped") => LiveState::Stopped,
                _ => LiveState::Idle,
            },
            reason: text(v, "reason"),
            codec: text(v, "codec"),
            width: int(v, "width").unwrap_or(0) as u32,
            height: int(v, "height").unwrap_or(0) as u32,
            fps: int(v, "fps").unwrap_or(0) as u32,
            position_ms: int(v, "positionMs"),
            started_at: int(v, "startedAt"),
            rate: v.get("rate").and_then(Value::as_f64).map_or(1.0, |r| r as f32).max(0.25),
            paused: flag(v, "paused", false),
        }
    }

    /// The session plays `camera` in a recording (a playback or a clip).
    pub fn is_recording(&self) -> bool {
        matches!(self.mode, LiveMode::Playback | LiveMode::Clip)
    }
}

/// The phase of an event's story: `new` the first time a tracked object is seen, `update` when it gains a label or a better score,
/// `end` when it leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    New,
    Update,
    End,
}

/// One pushed event, as `cameras.events` lists it (newest first).
#[derive(Clone, Debug, PartialEq)]
pub struct Pushed {
    /// The story's id, which is also its picture's.
    pub id: String,
    pub camera: String,
    pub name: String,
    pub kind: String,
    pub label: String,
    pub at: i64,
    pub phase: Phase,
    /// A picture of the moment exists (`thumbnail`); absent is taken as yes, the helper asks the bridge for it.
    pub thumbnail: bool,
    pub recording: bool,
}

impl Pushed {
    pub fn parse(v: &Value) -> Option<Self> {
        let id = match v.get("id") {
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            Some(n) if n.is_number() => n.to_string(),
            _ => return None,
        };
        Some(Self {
            id,
            camera: match v.get("camera") {
                Some(Value::String(s)) => s.clone(),
                Some(n) if n.is_number() => n.to_string(),
                _ => String::new(),
            },
            name: text(v, "name"),
            kind: text(v, "kind"),
            label: text(v, "label"),
            at: int(v, "at").unwrap_or(0),
            phase: match v.get("phase").and_then(Value::as_str) {
                Some("update") => Phase::Update,
                Some("end") => Phase::End,
                _ => Phase::New,
            },
            thumbnail: flag(v, "thumbnail", true),
            recording: flag(v, "recording", false),
        })
    }
}

/// The recent events of every camera: one card per event `id`, from its newest `new` or `update` entry; `end` entries make no card.
pub fn recent_cards(topic: &Value) -> Vec<Pushed> {
    let entries: Vec<Pushed> = topic.get("events").and_then(Value::as_array).map(|a| a.iter().filter_map(Pushed::parse).collect()).unwrap_or_default();
    let mut out: Vec<Pushed> = Vec::new();
    for e in entries {
        if e.phase == Phase::End || out.iter().any(|c| c.id == e.id) {
            continue;
        }
        out.push(e);
    }
    out
}

/// The `cameras.popup` record the viewer reads (the card is the overlay system's): which story, on which camera.
#[derive(Clone, Debug, PartialEq)]
pub struct Popup {
    pub id: String,
    pub event: String,
    pub camera: String,
    pub kind: String,
    pub until_ms: i64,
}

impl Popup {
    pub fn parse(v: &Value) -> Option<Self> {
        if !v.is_object() {
            return None;
        }
        let key = |k: &str| match v.get(k) {
            Some(Value::String(s)) => s.clone(),
            Some(n) if n.is_number() => n.to_string(),
            _ => String::new(),
        };
        let camera = key("camera");
        if camera.is_empty() {
            return None;
        }
        Some(Self { id: key("id"), event: key("event"), camera, kind: text(v, "kind"), until_ms: int(v, "untilMs").unwrap_or(0) })
    }
}

/// A span of time, epoch milliseconds, `from` to `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Span {
    pub from: i64,
    pub to: i64,
}

fn spans(v: &Value, key: &str) -> Vec<Span> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|r| Some(Span { from: int(r, "from")?, to: int(r, "to")? })).filter(|s| s.to > s.from).collect())
        .unwrap_or_default()
}

/// `GET /v1/cameras/{id}/recordings`: where something was recorded, and the cells the bridge could not tell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recordings {
    pub from: i64,
    pub to: i64,
    pub ranges: Vec<Span>,
    pub unknown: Vec<Span>,
}

impl Recordings {
    pub fn parse(v: &Value) -> Self {
        Self { from: int(v, "from").unwrap_or(0), to: int(v, "to").unwrap_or(0), ranges: spans(v, "ranges"), unknown: spans(v, "unknown") }
    }
}

/// A clip an event pointed at.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipRef {
    pub id: String,
    pub from: i64,
    pub to: i64,
}

/// One tick of the timeline: an event the recorder has (`GET /v1/cameras/{id}/events`).
#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    pub at: i64,
    pub kind: String,
    pub label: String,
    pub clip: Option<ClipRef>,
}

pub fn ticks(v: &Value) -> Vec<Tick> {
    v.get("events")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    Some(Tick {
                        at: int(e, "at")?,
                        kind: text(e, "kind"),
                        label: text(e, "label"),
                        clip: e.get("clip").filter(|c| c.is_object()).map(|c| ClipRef { id: text(c, "id"), from: int(c, "from").unwrap_or(0), to: int(c, "to").unwrap_or(0) }),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One clip (`GET /v1/cameras/{id}/clips`), newest first.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub id: String,
    pub at: i64,
    pub duration: i64,
    pub kind: String,
    pub label: String,
    pub description: String,
    pub has_thumbnail: bool,
    pub has_video: bool,
}

pub fn clips(v: &Value) -> Vec<Clip> {
    v.get("clips")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    let id = text(c, "id");
                    if id.is_empty() {
                        return None;
                    }
                    Some(Clip {
                        id,
                        at: int(c, "at")?,
                        duration: int(c, "duration").unwrap_or(0),
                        kind: text(c, "kind"),
                        label: text(c, "label"),
                        description: text(c, "description"),
                        has_thumbnail: flag(c, "has_thumbnail", true),
                        has_video: flag(c, "has_video", true),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// How often the grid's tiles ask for a picture (the `video.grid` choice): Light 333 ms, Balanced 200 ms, Smooth 125 ms.
pub fn grid_interval_ms(choice: &str) -> u64 {
    match choice {
        "light" => 333,
        "smooth" => 125,
        _ => 200,
    }
}

/// The values of the page the screens read: the two video choices and each camera's kinds.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub video_live: String,
    pub video_grid: String,
    /// Each camera's kinds that pop up, in the order the owner picked them.
    pub notify: BTreeMap<String, Vec<String>>,
    pub version: u64,
    pub revision: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { video_live: "native".into(), video_grid: "balanced".into(), notify: BTreeMap::new(), version: 0, revision: 0 }
    }
}

/// The defaults of a camera that was never picked for (design 3.5): a person and the doorbell.
pub fn default_kinds() -> Vec<String> {
    vec!["person".to_string(), "doorbell".to_string()]
}

impl Settings {
    /// The answer of `GET /v1/settings/scrypted-cameras`: the effective values (the stored ones over the defaults).
    pub fn parse(v: &Value) -> Self {
        let mut s = Self { version: v.get("version").and_then(Value::as_u64).unwrap_or(0), revision: v.get("schemaRevision").and_then(Value::as_u64).unwrap_or(0), ..Self::default() };
        let values = v.get("effective").filter(|e| e.is_object()).or_else(|| v.get("values"));
        let Some(values) = values.and_then(Value::as_object) else { return s };
        for (key, val) in values {
            let inner = val.get("value");
            match key.as_str() {
                "video.live" => {
                    if let Some(c) = inner.and_then(Value::as_str) {
                        s.video_live = c.to_string();
                    }
                }
                "video.grid" => {
                    if let Some(c) = inner.and_then(Value::as_str) {
                        s.video_grid = c.to_string();
                    }
                }
                k => {
                    if let Some(camera) = k.strip_prefix("notify.") {
                        let kinds = inner.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
                        s.notify.insert(camera.to_string(), kinds);
                    }
                }
            }
        }
        s
    }

    /// The kinds that pop up for `camera`: the owner's pick, else the defaults.
    pub fn kinds_for(&self, camera: &str) -> Vec<String> {
        self.notify.get(camera).cloned().unwrap_or_else(default_kinds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_cameras_topic_parses_with_defaults_for_what_is_left_out() {
        let c = Cameras::parse(&json!({
            "state": "ready", "reason": "", "now_skew_ms": 12, "rev": 7,
            "cameras": [
                {"id": "273", "name": "Front Door", "room": "Entry", "online": true, "live": true, "recordings": true, "events": true, "clips": false,
                 "since": 1792500000000i64, "native": {"codec": "h265", "width": 3840, "height": 2160, "fps": 15}, "prebuffered": true},
                {"id": 274},
                "junk"
            ]
        }));
        assert_eq!((c.state, c.skew_ms, c.rev, c.list.len()), (LinkState::Ready, 12, 7, 2));
        assert_eq!(c.list[0].name, "Front Door");
        assert_eq!(c.list[0].native, Some(Native { codec: "h265".into(), width: 3840, height: 2160, fps: 15 }));
        assert_eq!(c.list[0].since, Some(1792500000000));
        assert_eq!(c.list[1].name, "274", "a camera without a name is its id");
        assert!(c.list[1].online && c.list[1].live, "a camera is online and live until the helper says otherwise");
        assert!(!c.list[1].recordings && c.list[1].native.is_none());
        assert_eq!(c.index_of("274"), Some(1));
        assert_eq!(Cameras::parse(&json!(null)).state, LinkState::Unlinked);
    }

    #[test]
    fn every_link_state_round_trips_and_an_unknown_one_is_unlinked() {
        for s in ["unlinked", "connecting", "ready", "offline", "refused", "untrusted"] {
            assert_eq!(LinkState::parse(s).name(), s);
        }
        assert_eq!(LinkState::parse("weird"), LinkState::Unlinked);
    }

    #[test]
    fn the_live_topic_parses_the_session_and_a_null_position_in_live() {
        let l = Live::parse(&json!({"camera": "273", "mode": "playback", "state": "playing", "reason": "", "codec": "h265", "width": 3840, "height": 2160, "fps": 15,
                                    "startedAt": 1792605903000i64, "positionMs": 1792605910250i64, "rate": 2, "paused": false}));
        assert_eq!((l.camera.as_str(), l.mode, l.state), ("273", LiveMode::Playback, LiveState::Playing));
        assert_eq!((l.position_ms, l.started_at, l.rate), (Some(1792605910250), Some(1792605903000), 2.0));
        assert!(l.is_recording());
        let live = Live::parse(&json!({"camera": 273, "mode": "live", "state": "starting", "startedAt": null, "positionMs": null}));
        assert_eq!((live.camera.as_str(), live.state, live.position_ms, live.rate), ("273", LiveState::Starting, None, 1.0));
        assert!(!live.is_recording());
        assert_eq!(Live::parse(&Value::Null).state, LiveState::Idle);
    }

    #[test]
    fn the_recent_list_has_one_card_per_event_from_its_newest_entry_and_no_card_for_an_end() {
        let topic = json!({"events": [
            {"id": "e3", "camera": "1", "name": "A", "kind": "animal", "label": "Domestic Cat", "at": 300, "phase": "update"},
            {"id": "e3", "camera": "1", "name": "A", "kind": "animal", "label": "animal", "at": 300, "phase": "new"},
            {"id": "e2", "camera": "2", "name": "B", "kind": "person", "at": 200, "phase": "end"},
            {"id": "e2", "camera": "2", "name": "B", "kind": "person", "at": 200, "phase": "new"},
            {"id": "e1", "camera": "1", "kind": "doorbell", "at": 100}
        ]});
        let cards = recent_cards(&topic);
        assert_eq!(cards.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["e3", "e2", "e1"]);
        assert_eq!(cards[0].label, "Domestic Cat", "the newest entry of the story");
        assert_eq!(cards[1].phase, Phase::New, "the end made none and the new one of the story is the card");
    }

    #[test]
    fn the_popup_record_names_the_story_and_the_camera() {
        let p = Popup::parse(&json!({"id": 41, "event": "e4021", "camera": "273", "kind": "person", "untilMs": 99})).unwrap();
        assert_eq!((p.id.as_str(), p.event.as_str(), p.camera.as_str(), p.until_ms), ("41", "e4021", "273", 99));
        assert!(Popup::parse(&Value::Null).is_none());
        assert!(Popup::parse(&json!({"id": 1})).is_none(), "a card for no camera is nothing");
    }

    #[test]
    fn recordings_events_and_clips_parse_what_the_bridge_sent() {
        let r = Recordings::parse(&json!({"from": 0, "to": 100, "ranges": [{"from": 0, "to": 40}, {"from": 50, "to": 50}], "unknown": [{"from": 40, "to": 50}]}));
        assert_eq!(r.ranges, vec![Span { from: 0, to: 40 }], "an empty range is not one");
        assert_eq!(r.unknown, vec![Span { from: 40, to: 50 }]);
        let t = ticks(&json!({"events": [{"at": 5, "kind": "person", "label": "person", "clip": {"id": "c1", "from": 1, "to": 9}}, {"at": 6, "kind": "motion"}, {"kind": "x"}]}));
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].clip, Some(ClipRef { id: "c1".into(), from: 1, to: 9 }));
        assert_eq!(t[1].clip, None);
        let c = clips(&json!({"clips": [{"id": "c1.Wy", "at": 9, "duration": 30000, "kind": "person", "label": "person", "description": "Person at the front door",
                                          "has_thumbnail": true, "has_video": false}, {"at": 1}]}));
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].duration, c[0].has_video, c[0].description.as_str()), (30000, false, "Person at the front door"));
    }

    #[test]
    fn the_grid_interval_follows_the_choice() {
        assert_eq!(grid_interval_ms("light"), 333);
        assert_eq!(grid_interval_ms("balanced"), 200);
        assert_eq!(grid_interval_ms("smooth"), 125);
        assert_eq!(grid_interval_ms("whatever"), 200);
    }

    #[test]
    fn the_settings_read_the_video_choices_and_each_cameras_kinds_over_the_defaults() {
        let s = Settings::parse(&json!({"owner": "scrypted-cameras", "version": 4, "schemaRevision": 2, "effective": {
            "video.live": {"type": "choice", "value": "balanced"},
            "video.grid": {"type": "choice", "value": "smooth"},
            "notify.273": {"type": "ordered", "value": ["animal", "motion"]},
            "notify.274": {"type": "ordered", "value": []},
            "popups.on": {"type": "bool", "value": true}
        }}));
        assert_eq!((s.video_live.as_str(), s.video_grid.as_str(), s.version, s.revision), ("balanced", "smooth", 4, 2));
        assert_eq!(s.kinds_for("273"), ["animal", "motion"]);
        assert!(s.kinds_for("274").is_empty(), "off is a pick");
        assert_eq!(s.kinds_for("999"), ["person", "doorbell"], "never picked: the defaults");
        let d = Settings::parse(&json!({}));
        assert_eq!((d.video_live.as_str(), d.video_grid.as_str()), ("native", "balanced"));
    }
}
