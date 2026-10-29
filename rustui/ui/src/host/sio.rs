//! The TV's settings service as the screens see it (`kit/Sio.qml` of the QML shell, rule for rule): a screen names what it shows as
//! `category.key` (the names LG's Settings app and the stock services use: `picture.backlight`, `general.remotePointer`), this reads
//! them with `getSystemSettings` and keeps the answers, and writes one back with `setSystemSettings`. A write shows at once (the
//! screen never waits for the service to say what the person just chose) and a later answer replaces it.
//!
//! The set answers a read with the keys it has in `settings` and the rest in `errorKey`, with `returnValue: false` as soon as one key
//! is missing (OLED65C4, webOS 24): the settings are taken whatever the flag says. What the service does not have stays absent
//! (`has` is false, `missing` true once it said so) and a screen leaves such a row out rather than show a made-up value. Picture
//! settings belong to a "dimension" (picture mode, dynamic range, input): the service names the dimension it answered for and a write
//! carries the one the last answer had. Each read asks twice, for the values and for the service's description of them
//! (`getSystemSettingDesc`, `ui.active`/`ui.visible`): a setting whose description says it cannot be changed now stays on the screen
//! dimmed (`usable` is false), and a description that never comes leaves it usable. Reads go one at a time (the service answers by
//! method name, not by call) and each gets two seconds; one that is late is not waited for, and its answer is still taken when it comes.

use crate::kit::screen::{Effect, Input};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const SERVICE: &str = "luna://com.webos.service.settings";
/// How long one read is waited for.
pub const GUARD_MS: u64 = 2000;
/// The tags of this service's reads are `base + n`; its writes use `WRITE_TAG`.
pub const WRITE_TAG: u64 = 0x53FF_FFFF;
#[cfg(test)]
const TAG_BASE: u64 = 0x5300_0000;

#[derive(Clone, Debug, PartialEq)]
struct Read {
    category: String,
    keys: Vec<String>,
    app_id: Option<String>,
    desc: bool,
}

impl Read {
    fn method(&self) -> &'static str {
        if self.desc { "getSystemSettingDesc" } else { "getSystemSettings" }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
struct Desc {
    active: Option<bool>,
    visible: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct Sio {
    values: BTreeMap<String, Value>,
    seen: BTreeSet<String>,
    absent: BTreeSet<String>,
    descs: BTreeMap<String, Desc>,
    dimensions: BTreeMap<String, Value>,
    queue: VecDeque<Read>,
    /// The read in flight: its tag, what it asked and when it is given up.
    current: Option<(u64, Read, u64)>,
    /// Reads given up on, kept for their late answers.
    late: Vec<(u64, Read)>,
    next: u64,
    base: u64,
    /// Counts answers, for a screen that wants to know it has heard.
    pub answers: u64,
}

pub fn ds_name(category: &str, key: &str) -> String {
    if category.is_empty() { key.to_string() } else { format!("{category}.{key}") }
}

impl Sio {
    pub fn new(tag_base: u64) -> Self {
        Self { base: tag_base, ..Self::default() }
    }

    pub fn get(&self, ds: &str) -> Option<&Value> {
        self.values.get(ds)
    }

    /// The value as text, the way QML's `String(Sio.get(ds))` reads it (`""` for none).
    pub fn text(&self, ds: &str) -> String {
        match self.values.get(ds) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(v) => v.to_string(),
        }
    }

    pub fn has(&self, ds: &str) -> bool {
        self.seen.contains(ds)
    }

    pub fn missing(&self, ds: &str) -> bool {
        self.absent.contains(ds)
    }

    pub fn num(&self, ds: &str, fallback: f64) -> f64 {
        match self.values.get(ds) {
            Some(Value::Number(n)) => n.as_f64().unwrap_or(fallback),
            Some(Value::String(s)) => s.trim().parse::<f64>().unwrap_or(fallback),
            _ => fallback,
        }
    }

    pub fn is(&self, ds: &str, on: &str) -> bool {
        self.text(ds) == on
    }

    /// The service says the setting may be shown and changed (true until a description says otherwise); false until it has answered.
    pub fn usable(&self, ds: &str) -> bool {
        let blocked = self.descs.get(ds).is_some_and(|d| d.active == Some(false) || d.visible == Some(false));
        self.seen.contains(ds) && !blocked
    }

    /// Asks for keys of one category (and, for a per-app one, that app's values): the values and their description, one after the
    /// other. Nothing is asked twice while it is still queued.
    pub fn read(&mut self, category: &str, keys: &[&str], app_id: Option<&str>, now: u64) -> Vec<Effect> {
        for desc in [false, true] {
            let r = Read { category: category.to_string(), keys: keys.iter().map(|k| k.to_string()).collect(), app_id: app_id.map(str::to_string), desc };
            if !self.queue.contains(&r) {
                self.queue.push_back(r);
            }
        }
        self.pump(now)
    }

    fn pump(&mut self, now: u64) -> Vec<Effect> {
        if self.current.is_some() {
            return Vec::new();
        }
        let Some(read) = self.queue.pop_front() else { return Vec::new() };
        self.next += 1;
        let tag = self.base + (self.next & 0xFFFF);
        let mut params = Map::new();
        params.insert("keys".into(), json!(read.keys));
        if !read.category.is_empty() {
            params.insert("category".into(), json!(read.category));
        }
        if let Some(app) = &read.app_id {
            params.insert("app_id".into(), json!(app));
        }
        let effect = Effect::Luna { tag, uri: format!("{SERVICE}/{}", read.method()), payload: Value::Object(params), subscribe: false };
        self.current = Some((tag, read, now + GUARD_MS));
        vec![effect]
    }

    /// Takes the answer to a read: the values, the errors, the description.
    fn answer(&mut self, read: &Read, payload: &Value) {
        let cat = read.category.clone();
        if read.desc {
            let list: Vec<&Value> = match (payload.get("results").and_then(Value::as_array), payload.get("key")) {
                (Some(r), _) => r.iter().collect(),
                (None, Some(_)) => vec![payload],
                _ => Vec::new(),
            };
            for e in list {
                if let Some(key) = e.get("key").and_then(Value::as_str) {
                    let category = e.get("category").and_then(Value::as_str).unwrap_or(&cat);
                    let ui = e.get("ui");
                    self.descs.insert(
                        ds_name(category, key),
                        Desc { active: ui.and_then(|u| u.get("active")).and_then(Value::as_bool), visible: ui.and_then(|u| u.get("visible")).and_then(Value::as_bool) },
                    );
                }
            }
            self.answers += 1;
            return;
        }
        if let Some(settings) = payload.get("settings").and_then(Value::as_object) {
            for (key, value) in settings {
                let ds = ds_name(&cat, key);
                self.values.insert(ds.clone(), value.clone());
                self.seen.insert(ds);
            }
            if let Some(d) = payload.get("dimension") {
                self.dimensions.insert(cat.clone(), d.clone());
            }
        }
        if let Some(errors) = payload.get("errorKey").and_then(Value::as_array) {
            for key in errors.iter().filter_map(Value::as_str) {
                self.absent.insert(ds_name(&cat, key));
            }
        }
        self.answers += 1;
    }

    /// An answer of the service or the guard's time.
    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        match input {
            Input::Luna { tag, payload } if (self.base..self.base + 0x1_0000).contains(tag) => {
                if self.current.as_ref().is_some_and(|(t, _, _)| t == tag) {
                    let (_, read, _) = self.current.take().expect("checked");
                    self.answer(&read, payload);
                    return self.pump(now);
                }
                if let Some(i) = self.late.iter().position(|(t, _)| t == tag) {
                    let (_, read) = self.late.remove(i);
                    self.answer(&read, payload);
                }
                Vec::new()
            }
            Input::Tick => {
                if let Some((tag, read, due)) = self.current.clone()
                    && now >= due
                {
                    self.current = None;
                    self.late.push((tag, read));
                    if self.late.len() > 8 {
                        self.late.remove(0);
                    }
                    return self.pump(now);
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn due_at(&self) -> Option<u64> {
        self.current.as_ref().map(|(_, _, due)| *due)
    }

    /// Changes one setting. `ds` is `category.key` (or a bare key); the value goes out as the service wants it (text). It shows at
    /// once; a later answer replaces it.
    pub fn write(&mut self, ds: &str, value: &Value, app_id: Option<&str>) -> Vec<Effect> {
        let (cat, key) = ds.split_once('.').unwrap_or(("", ds));
        self.values.insert(ds.to_string(), value.clone());
        let text = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let mut body = Map::new();
        if !cat.is_empty() {
            body.insert("category".into(), json!(cat));
        }
        body.insert("settings".into(), json!({ key: text }));
        if let Some(d) = self.dimensions.get(cat) {
            body.insert("dimension".into(), d.clone());
        }
        if let Some(app) = app_id {
            body.insert("app_id".into(), json!(app));
        }
        vec![Effect::Luna { tag: WRITE_TAG, uri: format!("{SERVICE}/setSystemSettings"), payload: Value::Object(body), subscribe: false }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(sio: &mut Sio, effects: &[Effect], payload: Value, now: u64) -> Vec<Effect> {
        let Some(Effect::Luna { tag, .. }) = effects.first() else { panic!("no call: {effects:?}") };
        sio.update(&Input::Luna { tag: *tag, payload }, now)
    }

    #[test]
    fn a_read_asks_for_the_values_then_their_description_one_at_a_time() {
        let mut sio = Sio::new(TAG_BASE);
        let e = sio.read("picture", &["pictureMode", "backlight"], None, 0);
        assert_eq!(e.len(), 1, "one call in flight");
        let Effect::Luna { uri, payload, .. } = &e[0] else { panic!() };
        assert_eq!(uri, "luna://com.webos.service.settings/getSystemSettings");
        assert_eq!(payload, &json!({"keys": ["pictureMode", "backlight"], "category": "picture"}));
        // asking again while it is queued adds nothing
        assert!(sio.read("picture", &["pictureMode", "backlight"], None, 1).is_empty());
        let next = answer(&mut sio, &e, json!({"returnValue": true, "category": "picture", "settings": {"pictureMode": "cinema", "backlight": "40"}, "dimension": {"pictureMode": "cinema"}}), 5);
        assert_eq!(next.len(), 1);
        let Effect::Luna { uri, .. } = &next[0] else { panic!() };
        assert!(uri.ends_with("/getSystemSettingDesc"));
        assert!(sio.has("picture.pictureMode") && sio.is("picture.pictureMode", "cinema"));
        assert_eq!(sio.num("picture.backlight", 0.0), 40.0);
        assert_eq!(sio.answers, 1);
        answer(&mut sio, &next, json!({"results": [{"key": "backlight", "category": "picture", "ui": {"active": false, "visible": true}}]}), 9);
        assert!(!sio.usable("picture.backlight"), "a setting the description says is not active is dimmed");
        assert!(sio.usable("picture.pictureMode"), "a description that says nothing leaves it usable");
        assert!(!sio.usable("picture.eyeComfortMode"), "nothing is usable before it has answered");
    }

    #[test]
    fn a_key_the_service_does_not_have_stays_absent_and_the_others_are_taken_whatever_the_flag_says() {
        let mut sio = Sio::new(TAG_BASE);
        let e = sio.read("picture", &["pictureMode", "truMotionMode"], None, 0);
        answer(&mut sio, &e, json!({"returnValue": false, "settings": {"pictureMode": "vivid"}, "errorKey": ["truMotionMode"]}), 1);
        assert!(sio.has("picture.pictureMode"));
        assert!(!sio.has("picture.truMotionMode") && sio.missing("picture.truMotionMode"));
    }

    #[test]
    fn a_write_shows_at_once_and_carries_the_dimension_of_the_last_answer() {
        let mut sio = Sio::new(TAG_BASE);
        let e = sio.read("picture", &["pictureMode"], None, 0);
        answer(&mut sio, &e, json!({"settings": {"pictureMode": "cinema"}, "dimension": {"pictureMode": "cinema", "input": "hdmi1"}}), 1);
        let w = sio.write("picture.pictureMode", &json!("vivid"), None);
        assert!(sio.is("picture.pictureMode", "vivid"));
        let Effect::Luna { uri, payload, .. } = &w[0] else { panic!() };
        assert!(uri.ends_with("/setSystemSettings"));
        assert_eq!(payload, &json!({"category": "picture", "settings": {"pictureMode": "vivid"}, "dimension": {"pictureMode": "cinema", "input": "hdmi1"}}));
        // a number goes out as text; a key with no category carries none
        let w = sio.write("localeInfo", &json!(5), None);
        let Effect::Luna { payload, .. } = &w[0] else { panic!() };
        assert_eq!(payload, &json!({"settings": {"localeInfo": "5"}}));
    }

    #[test]
    fn a_read_that_is_late_is_given_up_after_two_seconds_and_its_answer_is_still_taken() {
        let mut sio = Sio::new(TAG_BASE);
        let e = sio.read("general", &["remotePointer"], None, 100);
        sio.read("sound", &["soundOutput"], None, 100);
        assert_eq!(sio.due_at(), Some(2100));
        let moved = sio.update(&Input::Tick, 2100);
        assert_eq!(moved.len(), 1, "the next read goes out");
        let Effect::Luna { payload, .. } = &moved[0] else { panic!() };
        assert_eq!(payload["category"], "general", "the description of the first is asked next");
        answer(&mut sio, &e, json!({"settings": {"remotePointer": "on"}}), 2500);
        assert!(sio.is("general.remotePointer", "on"), "the late answer is applied to what was asked");
    }
}
