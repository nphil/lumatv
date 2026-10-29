//! LG's settings service as the Rust shell reads and writes it (`kit/Sio.qml` of the QML shell, rule for rule). A screen names what it
//! shows as `category.key` (the names LG's own Settings app and the stock services use: `picture.backlight`, `general.remotePointer`)
//! and this reads them with `getSystemSettings` and `getSystemSettingDesc`, keeps the answers, and writes one back with
//! `setSystemSettings`. A write shows at once: a screen is never waiting for the service to say what the person just chose, and a later
//! answer replaces it. A key without a category (`localeInfo`) is read with none and named by the key alone.
//!
//! The set answers a read with the keys it has in `settings` and the rest in `errorKey`, with `returnValue: false` as soon as one key is
//! missing (OLED65C4, webOS 24: asked for eleven picture keys it answered nine and refused two); LG's own app takes the settings
//! whatever the flag says, and so does this. What the service does not have stays absent: [`Sio::has`] is false for it, [`Sio::missing`]
//! true once it said so, and a screen leaves such a row out rather than show a made-up value. Picture settings belong to a "dimension"
//! (picture mode, dynamic range, input): the service names the dimension it answered for, and a write carries the one the last answer
//! had. Each read asks twice, for the values and for the description of them (`ui` says whether LG's own Settings dims or hides a row,
//! `values` the list or the range it offers): a setting whose description says it cannot be changed now is [`Sio::usable`] false, a
//! description that never comes leaves the row usable. A few settings are per app (the aspect ratio, TruMotion): the service answers
//! them only when asked with the app's id. Reads go one at a time (the service answers by method name, not by call) and each gets two
//! seconds.
//!
//! Pure data: a read or a write is an [`Effect::Luna`] the host carries out, the answer comes back as [`Input::Luna`] with the tag, and
//! the guard is a [`Input::Tick`] at [`Sio::due_at`].

use crate::kit::screen::{Effect, Input};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// The settings service, as the compositor's luna allowlist names it.
pub const SERVICE: &str = "luna://com.webos.service.settings";
/// How long a read of the service may take before the next goes.
pub const READ_MS: u64 = 2000;
/// The name the quick panel's copy of this port gave it.
pub const GUARD_MS: u64 = READ_MS;
/// How many tags one instance uses before it starts again from the first.
pub const TAGS: u64 = 0x1_0000;

/// What JavaScript's `String(x)` and `parseFloat(x)` make of a value: the pages compare the text of what the service answers.
pub mod js {
    use serde_json::Value;

    /// `String(v)`: a string as it is, a number in its shortest decimal, `true`/`false`, `null`, a list joined by commas, an object as
    /// `[object Object]`.
    pub fn string(v: &Value) -> String {
        match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => match n.as_f64() {
                Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
                Some(f) => format!("{f}"),
                None => n.to_string(),
            },
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".into(),
            Value::Array(a) => a.iter().map(string).collect::<Vec<_>>().join(","),
            Value::Object(_) => "[object Object]".into(),
        }
    }

    /// `parseFloat(v)`: the number a value starts with, `None` where JavaScript says NaN.
    pub fn float(v: &Value) -> Option<f64> {
        match v {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => {
                let t = s.trim_start();
                let mut end = 0;
                let mut seen_dot = false;
                let mut seen_digit = false;
                for (i, c) in t.char_indices() {
                    match c {
                        '+' | '-' if i == 0 => {}
                        '0'..='9' => seen_digit = true,
                        '.' if !seen_dot => seen_dot = true,
                        _ => break,
                    }
                    end = i + c.len_utf8();
                }
                if !seen_digit { None } else { t[..end].parse().ok() }
            }
            _ => None,
        }
    }
}

/// `category.key`, or the key alone for one the service keeps outside every category (`localeInfo`).
pub fn ds_name(category: &str, key: &str) -> String {
    if category.is_empty() { key.to_string() } else { format!("{category}.{key}") }
}

fn split(ds: &str) -> (&str, &str) {
    match ds.split_once('.') {
        Some((c, k)) => (c, k),
        None => ("", ds),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    Values,
    Desc,
}

impl Method {
    fn uri(self) -> String {
        format!("{SERVICE}/{}", if self == Method::Values { "getSystemSettings" } else { "getSystemSettingDesc" })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Read {
    category: String,
    keys: Vec<String>,
    app: Option<String>,
    method: Method,
}

/// One key's description: what LG's own Settings reads to hide or dim a row, and the values or the range it offers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Desc {
    pub visible: Option<bool>,
    pub active: Option<bool>,
    pub values: Value,
}

/// The limits of a number the description gives (`values.range`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    pub interval: Option<f64>,
}

/// One value of the TV's own list for a key (`values.arrayExt`).
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub value: String,
    pub visible: bool,
    pub active: bool,
}

#[derive(Debug)]
pub struct Sio {
    base: u64,
    next: u64,
    values: BTreeMap<String, Value>,
    seen: BTreeSet<String>,
    absent: BTreeSet<String>,
    descs: BTreeMap<String, Desc>,
    dimensions: BTreeMap<String, Value>,
    queue: VecDeque<Read>,
    /// The read in flight, the tag it went with and when it is let go.
    current: Option<(Read, u64, u64)>,
    /// How many answers have come: a screen that wants to know it has heard.
    pub answers: u64,
}

impl Sio {
    /// A port whose calls take their tags from `tag_base` up (`TAGS` of them), so the host can tell them from every other model's.
    pub fn new(tag_base: u64) -> Self {
        Sio {
            base: tag_base,
            next: tag_base,
            values: BTreeMap::new(),
            seen: BTreeSet::new(),
            absent: BTreeSet::new(),
            descs: BTreeMap::new(),
            dimensions: BTreeMap::new(),
            queue: VecDeque::new(),
            current: None,
            answers: 0,
        }
    }

    fn tag(&mut self) -> u64 {
        let t = self.next;
        self.next = if self.next + 1 >= self.base + TAGS { self.base } else { self.next + 1 };
        t
    }

    /// Whether `tag` is one this port made.
    pub fn owns(&self, tag: u64) -> bool {
        (self.base..self.base + TAGS).contains(&tag)
    }

    pub fn get(&self, ds: &str) -> Option<&Value> {
        self.values.get(ds)
    }

    pub fn has(&self, ds: &str) -> bool {
        self.seen.contains(ds)
    }

    /// The service said it has no such key (a row for it is left out, as LG's own app leaves it out).
    pub fn missing(&self, ds: &str) -> bool {
        self.absent.contains(ds)
    }

    pub fn num(&self, ds: &str, fallback: f64) -> f64 {
        self.values.get(ds).and_then(js::float).unwrap_or(fallback)
    }

    /// `String(value)`: what the pages compare; `None` for a key with no value.
    pub fn opt_text(&self, ds: &str) -> Option<String> {
        self.values.get(ds).map(js::string)
    }

    /// `String(value)`, empty for a key with no value (the quick panel compares it as it is).
    pub fn text(&self, ds: &str) -> String {
        self.opt_text(ds).unwrap_or_default()
    }

    /// Whether the value reads as `want` (`String(Sio.get(ds)) === want`).
    pub fn is(&self, ds: &str, want: &str) -> bool {
        self.opt_text(ds).as_deref() == Some(want)
    }

    pub fn on(&self, ds: &str) -> bool {
        self.is(ds, "on")
    }

    pub fn desc(&self, ds: &str) -> Option<&Desc> {
        self.descs.get(ds)
    }

    /// The service says the setting may be shown and changed: answered, and no description says otherwise.
    pub fn usable(&self, ds: &str) -> bool {
        self.has(ds) && !self.descs.get(ds).is_some_and(|d| d.active == Some(false) || d.visible == Some(false))
    }

    /// LG leaves a row out when the key is missing or its description says it is not visible.
    pub fn shown(&self, ds: &str) -> bool {
        !self.missing(ds) && !self.descs.get(ds).is_some_and(|d| d.visible == Some(false))
    }

    pub fn range(&self, ds: &str) -> Option<Range> {
        let r = self.descs.get(ds)?.values.get("range")?;
        Some(Range { min: r.get("min").and_then(js::float).unwrap_or(0.0), max: r.get("max").and_then(js::float)?, interval: r.get("interval").and_then(js::float) })
    }

    /// The TV's own list of values for a key, in its order.
    pub fn list(&self, ds: &str) -> Option<Vec<Listed>> {
        let arr = self.descs.get(ds)?.values.get("arrayExt")?.as_array()?;
        if arr.is_empty() {
            return None;
        }
        Some(
            arr.iter()
                .map(|e| Listed {
                    value: e.get("value").map(js::string).unwrap_or_default(),
                    visible: e.get("visible").and_then(Value::as_bool) != Some(false),
                    active: e.get("active").and_then(Value::as_bool) != Some(false),
                })
                .collect(),
        )
    }

    pub fn dimension(&self, category: &str) -> Option<&Value> {
        self.dimensions.get(category)
    }

    /// Asks for keys of one category (and, for a per-app one, that app's values), for their values and their descriptions. Nothing is
    /// queued twice while it is still waiting. The read that goes now, when none is in flight.
    pub fn read(&mut self, category: &str, keys: &[&str], app: Option<&str>, now: u64) -> Vec<Effect> {
        for method in [Method::Values, Method::Desc] {
            let r = Read { category: category.to_string(), keys: keys.iter().map(|k| k.to_string()).collect(), app: app.map(str::to_string), method };
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
        let mut payload = json!({"keys": read.keys});
        if !read.category.is_empty() {
            payload["category"] = json!(read.category);
        }
        if let Some(app) = &read.app {
            payload["app_id"] = json!(app);
        }
        let tag = self.tag();
        let effect = Effect::Luna { tag, uri: read.method.uri(), payload, subscribe: false };
        self.current = Some((read, tag, now + READ_MS));
        vec![effect]
    }

    /// The answer to the read in flight (`Input::Luna` with its tag), or the guard (`Input::Tick`): a read that did not answer in time
    /// is let go and the next goes. What goes out next.
    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        match input {
            Input::Luna { tag, payload } => {
                let Some((read, t, _)) = self.current.clone() else { return Vec::new() };
                if t != *tag {
                    return Vec::new();
                }
                self.current = None;
                let category = if read.category.is_empty() { payload.get("category").and_then(Value::as_str).unwrap_or("").to_string() } else { read.category.clone() };
                match read.method {
                    Method::Desc => self.learn_desc(&category, payload),
                    Method::Values => self.learn(&category, payload),
                }
                self.answers += 1;
                self.pump(now)
            }
            Input::Tick if self.current.as_ref().is_some_and(|(_, _, deadline)| now >= *deadline) => {
                self.current = None;
                self.pump(now)
            }
            _ => Vec::new(),
        }
    }

    pub fn due_at(&self) -> Option<u64> {
        self.current.as_ref().map(|(_, _, deadline)| *deadline)
    }

    /// Nothing in flight and nothing waiting.
    pub fn idle(&self) -> bool {
        self.current.is_none() && self.queue.is_empty()
    }

    /// Takes the answer to a read of values: the keys it has, the dimension it answered for, and the keys it said it lacks.
    pub fn learn(&mut self, category: &str, payload: &Value) {
        if let Some(settings) = payload.get("settings").and_then(Value::as_object) {
            for (key, v) in settings {
                let ds = ds_name(category, key);
                self.values.insert(ds.clone(), v.clone());
                self.seen.insert(ds);
            }
            if let Some(d) = payload.get("dimension") {
                self.dimensions.insert(category.to_string(), d.clone());
            }
        }
        if let Some(keys) = payload.get("errorKey").and_then(Value::as_array) {
            for k in keys.iter().filter_map(Value::as_str) {
                self.absent.insert(ds_name(category, k));
            }
        }
    }

    /// Takes the answer to a read of descriptions: one entry for each key, with its `ui` and its `values`.
    pub fn learn_desc(&mut self, category: &str, payload: &Value) {
        let list: Vec<&Value> = match payload.get("results").and_then(Value::as_array) {
            Some(r) => r.iter().collect(),
            None if payload.get("key").is_some() => vec![payload],
            None => Vec::new(),
        };
        for e in list {
            let Some(key) = e.get("key").and_then(Value::as_str) else { continue };
            let cat = e.get("category").and_then(Value::as_str).unwrap_or(category);
            let ui = e.get("ui");
            self.descs.insert(
                ds_name(cat, key),
                Desc {
                    visible: ui.and_then(|u| u.get("visible")).and_then(Value::as_bool),
                    active: ui.and_then(|u| u.get("active")).and_then(Value::as_bool),
                    values: e.get("values").cloned().unwrap_or(Value::Null),
                },
            );
        }
    }

    /// Changes one setting: the value shows at once, the call goes out with the dimension of the last answer and the app's id for a
    /// per-app key. The service wants strings.
    pub fn write(&mut self, ds: &str, value: &Value, app: Option<&str>) -> Vec<Effect> {
        let (cat, key) = split(ds);
        self.values.insert(ds.to_string(), value.clone());
        let mut settings = Map::new();
        settings.insert(key.to_string(), json!(js::string(value)));
        vec![self.send(cat, Value::Object(settings), app)]
    }

    /// Several keys of one category in one call, each value as given (a key may hold an object: `option.adjustments`).
    pub fn write_many(&mut self, category: &str, settings: &Map<String, Value>, app: Option<&str>) -> Vec<Effect> {
        for (key, v) in settings {
            self.values.insert(ds_name(category, key), v.clone());
        }
        vec![self.send(category, Value::Object(settings.clone()), app)]
    }

    fn send(&mut self, category: &str, settings: Value, app: Option<&str>) -> Effect {
        let mut body = json!({"settings": settings});
        if !category.is_empty() {
            body["category"] = json!(category);
        }
        if let Some(d) = self.dimensions.get(category) {
            body["dimension"] = d.clone();
        }
        if let Some(app) = app {
            body["app_id"] = json!(app);
        }
        Effect::Luna { tag: self.tag(), uri: format!("{SERVICE}/setSystemSettings"), payload: body, subscribe: false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_of(effects: &[Effect]) -> u64 {
        match effects.first() {
            Some(Effect::Luna { tag, .. }) => *tag,
            other => panic!("a luna call goes: {other:?}"),
        }
    }

    fn answer(sio: &mut Sio, effects: &[Effect], payload: Value) -> Vec<Effect> {
        sio.update(&Input::Luna { tag: tag_of(effects), payload }, 10)
    }

    #[test]
    fn strings_and_numbers_read_as_javascript_reads_them() {
        assert_eq!(js::string(&json!("on")), "on");
        assert_eq!(js::string(&json!(3)), "3");
        assert_eq!(js::string(&json!(3.0)), "3");
        assert_eq!(js::string(&json!(2.5)), "2.5");
        assert_eq!(js::string(&json!(true)), "true");
        assert_eq!(js::string(&json!([1, "a"])), "1,a");
        assert_eq!(js::string(&json!({"a": 1})), "[object Object]");
        assert_eq!(js::float(&json!("12.5ms")), Some(12.5));
        assert_eq!(js::float(&json!("-3")), Some(-3.0));
        assert_eq!(js::float(&json!("off")), None);
        assert_eq!(js::float(&json!(true)), None);
        assert_eq!(js::float(&json!(7)), Some(7.0));
    }

    #[test]
    fn reads_go_one_at_a_time_with_their_descriptions_and_a_stuck_one_is_let_go_after_two_seconds() {
        let mut sio = Sio::new(0x5300_0000);
        let first = sio.read("picture", &["backlight", "contrast"], None, 0);
        assert_eq!(first.len(), 1, "one read goes, the rest wait");
        let Effect::Luna { uri, payload, .. } = &first[0] else { panic!() };
        assert_eq!(uri, "luna://com.webos.service.settings/getSystemSettings");
        assert_eq!(payload["category"], "picture");
        assert_eq!(payload["keys"], json!(["backlight", "contrast"]));
        assert!(sio.read("picture", &["backlight", "contrast"], None, 1).is_empty(), "nothing goes while one is in flight");
        sio.read("aspectRatio", &["justScan"], Some("hdmi1"), 1);
        assert_eq!(sio.queue.len(), 4, "a read already waiting is not queued twice (the first one's description waits still), one in flight comes again: the values again and the two of the per-app read");
        // the service did not answer: after two seconds the next goes
        assert!(sio.update(&Input::Tick, 1999).is_empty());
        let next = sio.update(&Input::Tick, 2000);
        assert_eq!(next.len(), 1);
        let Effect::Luna { uri, .. } = &next[0] else { panic!() };
        assert!(uri.ends_with("getSystemSettingDesc"));
        assert_eq!(sio.due_at(), Some(4000));
        assert!(sio.owns(tag_of(&next)) && !sio.owns(0x7041));
    }

    #[test]
    fn a_key_the_service_lacks_is_missing_and_one_it_answered_is_usable_unless_its_description_says_not_now() {
        let mut sio = Sio::new(0x5300_0000);
        let fx = sio.read("picture", &["backlight", "hgig"], None, 0);
        let fx = answer(&mut sio, &fx, json!({"returnValue": false, "settings": {"backlight": "60"}, "errorKey": ["hgig"], "dimension": {"pictureMode": "cinema", "input": "hdmi1"}}));
        assert!(sio.has("picture.backlight") && !sio.has("picture.hgig"));
        assert!(sio.missing("picture.hgig") && !sio.shown("picture.hgig"));
        assert_eq!(sio.num("picture.backlight", 0.0), 60.0);
        assert_eq!(sio.dimension("picture"), Some(&json!({"pictureMode": "cinema", "input": "hdmi1"})));
        assert!(sio.usable("picture.backlight"), "no description yet: usable");
        answer(
            &mut sio,
            &fx,
            json!({"results": [{"category": "picture", "key": "backlight", "ui": {"active": false}, "values": {"range": {"min": 0, "max": 100, "interval": 1}}}]}),
        );
        assert!(!sio.usable("picture.backlight"), "inactive: dimmed");
        assert!(sio.shown("picture.backlight"));
        assert_eq!(sio.range("picture.backlight"), Some(Range { min: 0.0, max: 100.0, interval: Some(1.0) }));
        assert_eq!(sio.answers, 2);
        assert!(sio.idle());
    }

    #[test]
    fn a_write_shows_at_once_and_goes_out_as_strings_with_the_dimension_and_the_app() {
        let mut sio = Sio::new(0x5300_0000);
        let fx = sio.read("picture", &["backlight"], None, 0);
        answer(&mut sio, &fx, json!({"settings": {"backlight": "60"}, "dimension": {"pictureMode": "cinema"}}));
        let e = sio.write("picture.backlight", &json!(70), None);
        assert_eq!(sio.num("picture.backlight", 0.0), 70.0, "shown before the service says anything");
        let Effect::Luna { uri, payload, subscribe, .. } = &e[0] else { panic!() };
        assert!(uri.ends_with("setSystemSettings") && !*subscribe);
        assert_eq!(*payload, json!({"settings": {"backlight": "70"}, "category": "picture", "dimension": {"pictureMode": "cinema"}}));
        let e = sio.write("aspectRatio.justScan", &json!("on"), Some("hdmi2"));
        let Effect::Luna { payload, .. } = &e[0] else { panic!() };
        assert_eq!(payload["app_id"], "hdmi2");
        let e = sio.write("localeInfo", &json!({"x": 1}), None);
        let Effect::Luna { payload, .. } = &e[0] else { panic!() };
        assert!(payload.get("category").is_none(), "a key outside every category is sent with none");
        let mut m = Map::new();
        m.insert("adjustments".into(), json!({"blackAndWhite": "off"}));
        let e = sio.write_many("option", &m, None);
        let Effect::Luna { payload, .. } = &e[0] else { panic!() };
        assert_eq!(payload["settings"]["adjustments"], json!({"blackAndWhite": "off"}), "an object goes as it is");
        assert_eq!(sio.get("option.adjustments"), Some(&json!({"blackAndWhite": "off"})));
    }

    #[test]
    fn a_list_in_the_description_keeps_the_tvs_order_and_what_it_hides() {
        let mut sio = Sio::new(0x5300_0000);
        let fx = sio.read("picture", &["gamma"], None, 0);
        let fx = answer(&mut sio, &fx, json!({"settings": {"gamma": "medium"}}));
        answer(
            &mut sio,
            &fx,
            json!({"results": [{"category": "picture", "key": "gamma", "values": {"arrayExt": [{"value": "low"}, {"value": "medium", "active": false}, {"value": "high1", "visible": false}]}}]}),
        );
        let l = sio.list("picture.gamma").unwrap();
        assert_eq!(l.iter().map(|e| (e.value.as_str(), e.visible, e.active)).collect::<Vec<_>>(), vec![("low", true, true), ("medium", true, false), ("high1", false, true)]);
    }

    #[test]
    fn tags_stay_in_the_ports_range_and_wrap() {
        let mut sio = Sio::new(0x5300_0000);
        assert_eq!(sio.tag(), 0x5300_0000);
        sio.next = 0x5300_0000 + TAGS - 1;
        assert_eq!(sio.tag(), 0x5300_0000 + TAGS - 1);
        assert_eq!(sio.tag(), 0x5300_0000);
    }
}
