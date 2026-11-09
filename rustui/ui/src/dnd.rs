//! Do Not Disturb as the helper publishes it (the `dnd` topic, `helper/app/src/dnd.rs`): what the status bar, Quick Settings, the pop-ups'
//! Snooze and the toast feed read, and the requests that change it. Times are absolute; one that has passed counts as over at once, before
//! the helper's timer publishes it.

use serde_json::{Value, json};

pub const TOPIC: &str = "dnd";
/// A minus in a circle, not a moon: Quick Settings shows the Sleep timer's moon one tile away.
pub const GLYPH: &str = "circle-minus";
pub const SNOOZE_GLYPH: &str = "bell-off";
/// The kind of a camera's alert; a plugin's is `plugin:<id>`.
pub const CAMERA: &str = "camera";

/// The tile's durations, and the body of `POST /v1/dnd/on` for each: plain OK is the first.
pub const DURATIONS: [&str; 4] = ["Until I turn it off", "1 hour", "2 hours", "Until tomorrow morning"];
/// The snooze sheet's times.
pub const SNOOZE_TIMES: [&str; 4] = ["15 min", "1 hour", "4 hours", "Until 07:00"];
/// The time the sheet starts on: OK, OK snoozes this camera for an hour.
pub const SNOOZE_DEFAULT: usize = 1;
/// The snooze sheet's form id, what `menu.picked` names it by.
pub const SNOOZE_FORM: &str = "snooze";
/// The sheet's choices of scope; the second is also the label of a snooze of every camera.
pub const THIS_CAMERA: &str = "This camera";
pub const ALL_CAMERAS: &str = "All camera alerts";
/// The notification drawer's group of what was held.
pub const MISSED_GROUP: &str = "Do Not Disturb";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snooze {
    pub id: String,
    pub kind: String,
    pub source: Option<String>,
    pub label: String,
    pub until: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Missed {
    /// [`CAMERA`], `toast` or `plugin:<id>`.
    pub kind: String,
    pub title: String,
    pub body: String,
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dnd {
    pub on: bool,
    pub until: Option<u64>,
    pub let_doorbell: bool,
    pub snoozes: Vec<Snooze>,
    pub missed: Vec<Missed>,
}

impl Default for Dnd {
    fn default() -> Self {
        Self { on: false, until: None, let_doorbell: true, snoozes: Vec::new(), missed: Vec::new() }
    }
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

impl Dnd {
    pub fn from_json(v: &Value) -> Self {
        let list = |key: &str| v.get(key).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        Self {
            on: v.get("on").and_then(Value::as_bool).unwrap_or(false),
            until: v.get("until").and_then(Value::as_u64),
            let_doorbell: v.get("letDoorbell").and_then(Value::as_bool).unwrap_or(true),
            snoozes: list("snoozes")
                .iter()
                .map(|s| Snooze { id: text(s, "id"), kind: text(s, "kind"), source: s.get("source").and_then(Value::as_str).map(str::to_string), label: text(s, "label"), until: s.get("until").and_then(Value::as_u64).unwrap_or(0) })
                .collect(),
            missed: list("missed")
                .iter()
                .map(|m| Missed { kind: text(m, "kind"), title: text(m, "title"), body: text(m, "body"), at: m.get("at").and_then(Value::as_u64).unwrap_or(0) })
                .collect(),
        }
    }

    /// Whether it is on at `now`.
    pub fn active(&self, now: u64) -> bool {
        self.on && self.until.is_none_or(|u| u > now)
    }

    /// The snoozes still on at `now`.
    pub fn snoozes_at(&self, now: u64) -> impl Iterator<Item = &Snooze> {
        self.snoozes.iter().filter(move |s| s.until > now)
    }

    /// What the status bar shows at `now`: the moon with the time left when it is timed, or the snooze glyph alone (drawn smaller) while a
    /// snooze is on; none when neither is.
    pub fn status(&self, now: u64) -> Option<(&'static str, String)> {
        if self.active(now) {
            return Some((GLYPH, self.until.map(|u| left(u - now)).unwrap_or_default()));
        }
        self.snoozes_at(now).next().map(|_| (SNOOZE_GLYPH, String::new()))
    }

    /// When [`Self::status`] next changes after `now`: the minute its time left counts down to, or a snooze running out.
    pub fn changes_at(&self, now: u64) -> Option<u64> {
        let mut due = self.snoozes_at(now).map(|s| s.until).min();
        if let Some(u) = self.until.filter(|_| self.active(now)) {
            let tick = u - ((u - now).div_ceil(60_000) - 1) * 60_000;
            due = Some(due.map_or(tick, |d| d.min(tick)));
        }
        due
    }
}

/// The time left, rounded up to the minute: `1h 20m`, `45m`, `1d 3h`.
pub fn left(ms: u64) -> String {
    let m = ms.div_ceil(60_000).max(1);
    match (m / 1440, m % 1440 / 60, m % 60) {
        (0, 0, m) => format!("{m}m"),
        (0, h, 0) => format!("{h}h"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

/// The body of `POST /v1/dnd/on` for the duration at `index` of [`DURATIONS`].
pub fn on_body(index: usize) -> Value {
    match index {
        1 => json!({"minutes": 60}),
        2 => json!({"minutes": 120}),
        3 => json!({"until": "morning"}),
        _ => json!({}),
    }
}

/// The body of `POST /v1/dnd/snooze`: `source` none for the whole kind, `time` an index of [`SNOOZE_TIMES`].
pub fn snooze_body(kind: &str, source: Option<&str>, label: &str, time: usize) -> Value {
    let mut body = match time {
        0 => json!({"minutes": 15}),
        2 => json!({"minutes": 240}),
        3 => json!({"until": "morning"}),
        _ => json!({"minutes": 60}),
    };
    body["kind"] = json!(kind);
    body["label"] = json!(label);
    if let Some(s) = source {
        body["source"] = json!(s);
    }
    body
}

/// The confirmation toast after a snooze.
pub fn snoozed_words(label: &str, time: usize) -> String {
    match time {
        3 => format!("{label} snoozed until 07:00"),
        t => format!("{label} snoozed for {}", SNOOZE_TIMES.get(t).copied().unwrap_or("1 hour")),
    }
}

/// The sheet a held OK on a camera's pop-up opens (a form of `panels/menu.rs`), under the camera's name: whose alerts, then for how long.
pub fn snooze_sheet(name: &str) -> Value {
    json!({"form": SNOOZE_FORM, "title": "Snooze camera alerts", "sub": name, "icon": SNOOZE_GLYPH, "items": [
        {"id": "from", "label": "From", "options": [THIS_CAMERA, ALL_CAMERAS], "index": 0},
        {"id": "for", "label": "For", "options": SNOOZE_TIMES, "index": SNOOZE_DEFAULT},
    ]})
}

/// What the sheet's pick (`values`, each row's choice) snoozes for camera `camera` called `name`: the source (none for every camera), the
/// snooze's label and its time (an index of [`SNOOZE_TIMES`]).
pub fn snooze_picked(camera: &str, name: &str, values: &Value) -> (Option<String>, String, usize) {
    let at = |i: usize| values.get(i).and_then(Value::as_u64).map(|v| v as usize);
    let time = at(1).filter(|t| *t < SNOOZE_TIMES.len()).unwrap_or(SNOOZE_DEFAULT);
    if at(0) == Some(1) { (None, ALL_CAMERAS.into(), time) } else { (Some(camera.into()), name.into(), time) }
}

/// What a list of missed alerts is called on the tile's card.
pub fn missed_words(n: usize) -> String {
    format!("{n} missed while Do Not Disturb was on")
}

/// The Quick Settings tile's second line: how long it is on, else what is snoozed, then how many alerts it held. `time_of` writes a clock time.
pub fn detail(d: &Dnd, time_of: impl Fn(u64) -> String) -> String {
    let mut parts = Vec::new();
    if d.on {
        parts.push(d.until.map_or_else(|| "Until turned off".to_string(), |u| format!("Until {}", time_of(u))));
    } else {
        match d.snoozes.as_slice() {
            [] => {}
            [s] => parts.push(format!("{} snoozed until {}", s.label, time_of(s.until))),
            many => parts.push(format!("{} alerts snoozed", many.len())),
        }
    }
    if !d.missed.is_empty() {
        parts.push(format!("{} missed", d.missed.len()));
    }
    if parts.is_empty() { "Holds pop-ups and alerts".to_string() } else { parts.join(" \u{b7} ") }
}

/// The words of the tile's card: what it holds and lets through, and how many alerts it held.
pub fn card_text(d: &Dnd) -> String {
    let mut text = String::from("Holds pop-ups and alerts. Volume, input and receiver messages still show.");
    if d.let_doorbell {
        text.push_str(" The doorbell rings through.");
    }
    if !d.missed.is_empty() {
        text.push_str(&format!(" {}.", missed_words(d.missed.len())));
    }
    text
}

/// Whether a toast from LG's feed is an outside alert that Do Not Disturb holds: anything an app, a phone or Home Assistant sends (Home
/// Assistant's `notify` comes through LG's second-screen gateway), and nothing of LG's own system or of LumaTV's, which say what a key or a
/// switch just did.
pub fn holds_toast(source_id: &str) -> bool {
    const RELAYS: [&str; 2] = ["com.webos.service.secondscreen.gateway", "com.webos.service.apiadapter"];
    let own = ["com.webos.", "com.lge.", "lumatv"].iter().any(|p| source_id.starts_with(p)) || source_id == "lg";
    RELAYS.contains(&source_id) || !own
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_bar_shows_the_moon_with_the_time_left_or_the_snooze_glyph() {
        let now = 1_000_000_000;
        let on = Dnd::from_json(&json!({"on": true, "until": now + 80 * 60_000, "snoozes": [{"id": "s1", "kind": "camera", "until": now + 60_000}]}));
        assert_eq!(on.status(now), Some((GLYPH, "1h 20m".to_string())));
        assert_eq!(on.status(now + 80 * 60_000), None, "both ran out");
        let off = Dnd::from_json(&json!({"on": true, "until": null}));
        assert_eq!(off.status(now), Some((GLYPH, String::new())), "until it is turned off: no time");
        let snoozed = Dnd::from_json(&json!({"on": false, "snoozes": [{"id": "s1", "kind": "camera", "until": now + 60_000}]}));
        assert_eq!(snoozed.status(now), Some((SNOOZE_GLYPH, String::new())));
        assert_eq!([left(45 * 60_000 - 1), left(60 * 60_000), left(26 * 3_600_000)], ["45m", "1h", "1d 2h"]);
        // the text changes once a minute, not per frame: 80 min 30 s left shows 1h 21m until 80 min are left
        let timed = Dnd::from_json(&json!({"on": true, "until": now + 80 * 60_000 + 30_000}));
        assert_eq!(timed.changes_at(now), Some(now + 30_000));
        assert_eq!(on.changes_at(now), Some(now + 60_000), "the snooze runs out first");
        assert_eq!(off.changes_at(now), None);
    }

    #[test]
    fn outside_senders_toasts_are_held_and_lg_and_lumatv_feedback_is_not() {
        assert!(holds_toast("com.webos.service.secondscreen.gateway"), "Home Assistant's notify");
        assert!(holds_toast("com.netflix.app"));
        assert!(!holds_toast("com.webos.service.audio"));
        assert!(!holds_toast("com.webos.app.home"), "a Settings switch saying what it did");
        assert!(!holds_toast("lumatv"));
    }

    #[test]
    fn a_snooze_of_every_camera_has_no_source() {
        assert_eq!(snooze_picked("275", "Front door", &json!([1, 3])), (None, ALL_CAMERAS.to_string(), 3));
        assert_eq!(snooze_body(CAMERA, None, ALL_CAMERAS, 3), json!({"until": "morning", "kind": "camera", "label": "All camera alerts"}));
        assert_eq!(snoozed_words(ALL_CAMERAS, 3), "All camera alerts snoozed until 07:00");
    }
}
