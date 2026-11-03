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
//! (picture mode, dynamic range, input): writes carry the last answer's dimension except for a picture mode selector write, which
//! selects the current native context without a dimension. Each read asks twice, for the values and for the description of them (`ui` says whether LG's own Settings dims or hides a row,
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
    backlight_dimension: Option<Value>,
    backlight_context_required: bool,
    queue: VecDeque<Read>,
    picture_reads: Vec<Read>,
    picture_writes: BTreeMap<u64, u64>,
    picture_refresh: bool,
    picture_mode: Option<String>,
    picture_range: Option<String>,
    // LG omits dimension fields irrelevant to a key. Keep identity separate from the exact native write dimension.
    picture_dimension_mode: String,
    picture_input: String,
    /// Changes that require a redraw without a service answer.
    pub revision: u64,
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
            backlight_dimension: None,
            backlight_context_required: false,
            queue: VecDeque::new(),
            picture_reads: Vec::new(),
            picture_writes: BTreeMap::new(),
            picture_refresh: false,
            picture_mode: None,
            picture_range: None,
            picture_dimension_mode: String::new(),
            picture_input: String::new(),
            revision: 0,
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

    /// A setting is usable when its value and any required current-context description have arrived.
    pub fn usable(&self, ds: &str) -> bool {
        self.has(ds)
            && !(ds == "picture.backlight" && self.backlight_context_required && self.backlight_dimension.is_none())
            && !(self.picture_refresh && ds.starts_with("picture.") && ds != "picture.pictureMode"
                && (!self.picture_writes.is_empty() || !self.picture_mode_matches_dimension() || !self.descs.contains_key(ds)))
            && !self.descs.get(ds).is_some_and(|d| d.active == Some(false) || d.visible == Some(false))
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

    /// The last dynamic range explicitly reported by the TV, including while a selector is pending.
    pub fn picture_dynamic_range(&self) -> Option<&str> {
        self.picture_range.as_deref()
    }

    fn picture_mode_matches_dimension(&self) -> bool {
        self.picture_mode.as_deref().filter(|mode| !mode.is_empty()).is_some_and(|mode|
            !self.picture_dimension_mode.is_empty() && self.picture_dimension_mode == mode)
    }

    /// A confirmed mode and matching dimension with an input identify the native context.
    pub fn picture_context_ready(&self) -> bool {
        self.picture_writes.is_empty() && self.picture_mode_matches_dimension()
            && self.dimension("picture").and_then(|d| d.get("input")).and_then(Value::as_str).is_some_and(|input| !input.is_empty())
    }

    /// Asks for keys of one category (and, for a per-app one, that app's values), for their values and their descriptions. Nothing is
    /// queued twice while it is still waiting. The read that goes now, when none is in flight.
    pub fn read(&mut self, category: &str, keys: &[&str], app: Option<&str>, now: u64) -> Vec<Effect> {
        for method in [Method::Values, Method::Desc] {
            let r = Read { category: category.to_string(), keys: keys.iter().map(|k| k.to_string()).collect(), app: app.map(str::to_string), method };
            if category == "picture" && !self.picture_reads.contains(&r) {
                self.picture_reads.push(r.clone());
            }
            if !self.queue.contains(&r) {
                self.queue.push_back(r);
            }
        }
        if category == "picture" && app.is_none() && keys.contains(&"backlight") {
            self.backlight_context_required = true;
            if keys != ["backlight"] {
                let r = Read { category: category.to_string(), keys: vec!["backlight".into()], app: None, method: Method::Values };
                if !self.picture_reads.contains(&r) { self.picture_reads.push(r.clone()); }
                if !self.queue.contains(&r) { self.queue.push_back(r); }
            }
        }
        self.pump(now)
    }

    fn pump(&mut self, now: u64) -> Vec<Effect> {
        if self.current.is_some() {
            return Vec::new();
        }
        let Some(index) = self.queue.iter().position(|r| r.category != "picture" || self.picture_writes.is_empty()) else { return Vec::new() };
        let read = self.queue.remove(index).unwrap();
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
                if self.picture_writes.remove(tag).is_some() {
                    return self.pump(now);
                }
                let Some((read, t, _)) = self.current.clone() else { return Vec::new() };
                if t != *tag {
                    return Vec::new();
                }
                self.current = None;
                let category = if read.category.is_empty() { payload.get("category").and_then(Value::as_str).unwrap_or("").to_string() } else { read.category.clone() };
                if self.picture_refresh && category == "picture" && read.method == Method::Values
                    && read.keys.iter().any(|key| key == "pictureMode")
                    && payload.pointer("/settings/pictureMode").is_none()
                {
                    self.values.remove("picture.pictureMode");
                    self.seen.remove("picture.pictureMode");
                    self.picture_mode = None;
                }
                match read.method {
                    Method::Desc => self.learn_desc(&category, payload),
                    Method::Values => {
                        let backlight_read = category == "picture" && read.app.is_none() && read.keys == ["backlight"];
                        if self.learn_values(&category, payload, !backlight_read) {
                            self.queue_picture_reads();
                        }
                        if backlight_read {
                            self.backlight_dimension = payload.get("settings").and_then(|s| s.get("backlight"))
                                .and_then(|_| payload.get("dimension")).filter(|d| d.is_object()).cloned();
                        }
                    }
                }
                self.answers += 1;
                self.pump(now)
            }
            Input::Tick => {
                self.picture_writes.retain(|_, deadline| now < *deadline);
                if self.current.as_ref().is_some_and(|(_, _, deadline)| now >= *deadline) {
                    if self.picture_refresh && self.current.as_ref().is_some_and(|(read, _, _)|
                        read.category == "picture" && read.method == Method::Values && read.keys.iter().any(|key| key == "pictureMode"))
                    {
                        self.values.remove("picture.pictureMode");
                        self.seen.remove("picture.pictureMode");
                        self.picture_mode = None;
                        self.revision = self.revision.wrapping_add(1);
                    }
                    self.current = None;
                }
                self.pump(now)
            }
            _ => Vec::new(),
        }
    }

    pub fn due_at(&self) -> Option<u64> {
        self.current.as_ref().map(|(_, _, deadline)| *deadline).into_iter().chain(self.picture_writes.values().copied()).min()
    }

    /// Nothing in flight and nothing waiting.
    pub fn idle(&self) -> bool {
        self.current.is_none() && self.queue.is_empty() && self.picture_writes.is_empty()
    }

    /// Takes the answer to a read of values: the keys it has, the dimension it answered for, and the keys it said it lacks.
    pub fn learn(&mut self, category: &str, payload: &Value) {
        self.learn_values(category, payload, true);
    }

    fn learn_values(&mut self, category: &str, payload: &Value, record_dimension: bool) -> bool {
        let mut picture_changed = false;
        if let Some(settings) = payload.get("settings").and_then(Value::as_object) {
            if category == "picture" {
                let dimension = payload.get("dimension");
                let changed_dimension = dimension.is_some_and(|d| [
                    ("pictureMode", self.picture_dimension_mode.as_str()),
                    ("dynamicRange", self.picture_range.as_deref().unwrap_or("")),
                    ("input", self.picture_input.as_str()),
                ].iter().any(|(key, old)| !old.is_empty() && d.get(*key).and_then(Value::as_str)
                    .is_some_and(|new| !new.is_empty() && new != *old)));
                let changed_mode = settings.get("pictureMode").and_then(Value::as_str)
                    .is_some_and(|mode| self.picture_mode.as_deref().is_some_and(|old| old != mode));
                if changed_dimension || changed_mode {
                    picture_changed = true;
                    self.picture_refresh = true;
                    self.values.retain(|key, _| !key.starts_with("picture."));
                    self.seen.retain(|key| !key.starts_with("picture."));
                    self.absent.retain(|key| !key.starts_with("picture."));
                    self.descs.retain(|key, _| !key.starts_with("picture."));
                    self.backlight_dimension = None;
                    self.picture_mode = None;
                    self.picture_dimension_mode.clear();
                    self.picture_input.clear();
                    self.picture_range = None;
                    if dimension.is_none() || !record_dimension {
                        self.dimensions.remove("picture");
                    }
                }
                if let Some(d) = dimension {
                    if let Some(range) = d.get("dynamicRange").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                        self.picture_range = Some(range.to_owned());
                    }
                    if let Some(mode) = d.get("pictureMode").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                        mode.clone_into(&mut self.picture_dimension_mode);
                    }
                    if let Some(input) = d.get("input").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                        input.clone_into(&mut self.picture_input);
                    }
                }
                if let Some(mode) = settings.get("pictureMode") {
                    self.picture_mode = mode.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
                }
            }
            for (key, v) in settings {
                let ds = ds_name(category, key);
                self.values.insert(ds.clone(), v.clone());
                self.seen.insert(ds);
            }
            if record_dimension && let Some(d) = payload.get("dimension") {
                self.dimensions.insert(category.to_string(), d.clone());
            }
        }
        if let Some(keys) = payload.get("errorKey").and_then(Value::as_array) {
            for k in keys.iter().filter_map(Value::as_str) {
                self.absent.insert(ds_name(category, k));
            }
        }
        picture_changed
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

    /// Changes one setting: the value shows at once. Writes carry the last dimension except for picture mode selectors, and the app's
    /// id for a per-app key. The service wants strings.
    pub fn write(&mut self, ds: &str, value: &Value, app: Option<&str>, now: u64) -> Vec<Effect> {
        let (cat, key) = split(ds);
        if cat == "picture" && key != "pictureMode" && !self.picture_write_ready(key) {
            return Vec::new();
        }
        let mut settings = Map::new();
        settings.insert(key.to_string(), json!(js::string(value)));
        if cat == "picture" && key == "pictureMode" {
            self.invalidate_picture();
        }
        self.values.insert(ds.to_string(), value.clone());
        vec![self.send(cat, Value::Object(settings), app, now)]
    }

    /// Several keys of one category in one call, each value as given (a key may hold an object: `option.adjustments`).
    pub fn write_many(&mut self, category: &str, settings: &Map<String, Value>, app: Option<&str>, now: u64) -> Vec<Effect> {
        if category == "picture" && settings.keys().any(|key| key != "pictureMode" && !self.picture_write_ready(key)) {
            return Vec::new();
        }
        if category == "picture" && settings.contains_key("pictureMode") {
            self.invalidate_picture();
        }
        for (key, v) in settings {
            if category != "picture" || !settings.contains_key("pictureMode") || key == "pictureMode" {
                self.values.insert(ds_name(category, key), v.clone());
            }
        }
        vec![self.send(category, Value::Object(settings.clone()), app, now)]
    }

    fn picture_write_ready(&self, key: &str) -> bool {
        self.picture_writes.is_empty() && !(key == "backlight" && self.backlight_context_required && self.backlight_dimension.is_none())
            && (!self.picture_refresh || self.usable(&ds_name("picture", key)))
    }

    fn invalidate_picture(&mut self) {
        self.picture_refresh = true;
        self.values.retain(|key, _| !key.starts_with("picture.") || key == "picture.pictureMode");
        self.seen.retain(|key| !key.starts_with("picture.") || key == "picture.pictureMode");
        self.absent.retain(|key| !key.starts_with("picture.") || key == "picture.pictureMode");
        self.descs.retain(|key, _| !key.starts_with("picture.") || key == "picture.pictureMode");
        self.dimensions.remove("picture");
        self.backlight_dimension = None;
        self.picture_mode = None;
        self.picture_dimension_mode.clear();
        self.picture_input.clear();
        if self.current.as_ref().is_some_and(|(read, _, _)| read.category == "picture") {
            self.current = None;
        }
        self.queue_picture_reads();
    }

    fn queue_picture_reads(&mut self) {
        self.queue.retain(|read| read.category != "picture");
        for method in [Method::Values, Method::Desc] {
            if !self.picture_reads.iter().any(|read| read.method == method && read.keys.iter().any(|key| key == "pictureMode")) {
                self.picture_reads.push(Read { category: "picture".into(), keys: vec!["pictureMode".into()], app: None, method });
            }
        }
        self.queue.extend(self.picture_reads.iter().cloned());
    }

    fn send(&mut self, category: &str, settings: Value, app: Option<&str>, now: u64) -> Effect {
        let mut body = json!({"settings": settings});
        if !category.is_empty() {
            body["category"] = json!(category);
        }
        // r19w: an old pictureMode dimension kept the TV on eco; an unpinned filmMaker write switched it at once.
        let backlight_only = category == "picture" && body["settings"].as_object().is_some_and(|s| s.len() == 1 && s.contains_key("backlight"));
        let dimension = if backlight_only && self.backlight_context_required { self.backlight_dimension.as_ref() } else { self.dimensions.get(category) };
        if !(category == "picture" && body["settings"].get("pictureMode").is_some())
            && let Some(d) = dimension
        {
            body["dimension"] = d.clone();
        }
        if let Some(app) = app {
            body["app_id"] = json!(app);
        }
        let tag = self.tag();
        if category == "picture" && body["settings"].get("pictureMode").is_some() {
            self.picture_writes.insert(tag, now.saturating_add(READ_MS));
        }
        Effect::Luna { tag, uri: format!("{SERVICE}/setSystemSettings"), payload: body, subscribe: false }
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
    fn backlight_writes_use_its_native_context_and_follow_independent_readback() {
        let mut sio = Sio::new(0x5300_0000);
        let batch = sio.read("picture", &["pictureMode", "backlight"], None, 0);
        let desc = answer(&mut sio, &batch, json!({"settings":{"pictureMode":"eco","backlight":"77"}, "dimension":{"pictureMode":"eco","dynamicRange":"sdr","input":"default"}}));
        assert!(sio.write("picture.backlight", &json!(78.0), None, 11).is_empty());
        let native = answer(&mut sio, &desc, json!({"results":[{"key":"backlight","ui":{"active":true}}]}));
        let reset_dimension = sio.dimension("picture").cloned();
        answer(&mut sio, &native, json!({"settings":{"backlight":"77"}, "dimension":{"pictureMode":"eco","input":"default"}}));
        assert_eq!(sio.dimension("picture").cloned(), reset_dimension);
        sio.learn("picture", &json!({"settings":{"contrast":"85"}, "dimension":{"dynamicRange":"sdr","input":"default"}}));
        for value in [78.0, 79.0] {
            let write = sio.write("picture.backlight", &json!(value), None, 12);
            let Effect::Luna { uri, payload, .. } = &write[0] else { panic!() };
            assert!(uri.ends_with("/setSystemSettings"));
            assert_eq!(payload["dimension"], json!({"pictureMode":"eco","input":"default"}));
            assert_eq!(payload["settings"]["backlight"], js::string(&json!(value)));
        }
        let readback = sio.read("picture", &["backlight"], None, 20);
        answer(&mut sio, &readback, json!({"settings":{"backlight":"76"}, "dimension":{"pictureMode":"eco","input":"default"}}));
        assert_eq!(sio.num("picture.backlight", -1.0), 76.0, "the service's independent answer wins");
        sio.write("picture.pictureMode", &json!("cinema"), None, 30);
        assert!(sio.write("picture.backlight", &json!(80), None, 31).is_empty(), "old context is never reused");
    }


    #[test]
    fn partial_picture_dimensions_keep_current_controls_but_real_context_changes_clear_them() {
        let mut sio = Sio::new(0x5300_0000);
        let full = json!({"pictureMode":"filmMaker","dynamicRange":"sdr","input":"default"});
        sio.learn("picture", &json!({"settings":{"pictureMode":"filmMaker","backlight":"10"},"dimension":full}));
        sio.learn_desc("picture", &json!({"key":"backlight","ui":{"active":true}}));
        for dimension in [json!({"pictureMode":"filmMaker","input":"default"}), json!({"dynamicRange":"sdr","input":"default"}), json!({"pictureMode":"filmMaker","dynamicRange":"sdr"}), full] {
            sio.learn("picture", &json!({"settings":{"contrast":"85"},"dimension":dimension}));
            assert_eq!(sio.text("picture.pictureMode"), "filmMaker");
            assert_eq!(sio.num("picture.backlight", -1.0), 10.0);
            assert!(sio.usable("picture.backlight"));
            assert_eq!(sio.picture_context_ready(), dimension.get("input").is_some(), "reset must have a native input dimension");
            assert_eq!(sio.picture_dynamic_range(), Some("sdr"));
            assert_eq!(sio.dimension("picture"), Some(&dimension), "outbound dimensions remain exactly native");
        }
        sio.learn("picture", &json!({"settings":{"contrast":"90"},"dimension":{"input":"hdmi2"}}));
        assert!(!sio.has("picture.backlight"));
        assert!(!sio.usable("picture.backlight"));
        assert!(!sio.picture_context_ready());
        assert_eq!(sio.picture_dynamic_range(), None);
        assert!(sio.write("picture.backlight", &json!(20), None, 100).is_empty());
    }

    #[test]
    fn late_partial_picture_context_change_refreshes_once_and_restores_controls() {
        let mut sio = Sio::new(0x5300_0000);
        let old_dimension = json!({"pictureMode":"eco", "dynamicRange":"sdr", "input":"hdmi1"});
        let new_dimension = json!({"pictureMode":"dolbyHdrCinema", "dynamicRange":"dolbyHdr", "input":"hdmi2"});
        let first = sio.read("picture", &["pictureMode"], None, 0);
        let desc = answer(&mut sio, &first, json!({"settings":{"pictureMode":"eco"}, "dimension":old_dimension}));
        answer(&mut sio, &desc, json!({"key":"pictureMode"}));
        let partial = sio.read("picture", &["backlight"], None, 20);
        let mode = answer(&mut sio, &partial, json!({"settings":{"backlight":"70"}, "dimension":new_dimension}));
        assert!(!sio.usable("picture.backlight"));
        let mode_desc = answer(&mut sio, &mode, json!({"settings":{"pictureMode":"dolbyHdrCinema"}, "dimension":new_dimension}));
        let brightness = answer(&mut sio, &mode_desc, json!({"key":"pictureMode"}));
        let brightness_desc = answer(&mut sio, &brightness, json!({"settings":{"backlight":"70"}, "dimension":new_dimension}));
        let done = answer(&mut sio, &brightness_desc, json!({"key":"backlight", "ui":{"active":true}}));
        assert!(done.is_empty() && sio.idle(), "the same context must not start another refresh");
        assert!(sio.picture_context_ready());
        assert!(sio.usable("picture.backlight"));
        assert!(!sio.write("picture.backlight", &json!(80), None, 30).is_empty());
    }

    #[test]
    fn picture_switch_discards_stale_reads_and_waits_for_fresh_context() {
        let mut sio = Sio::new(0x5300_0000);
        let old = sio.read("picture", &["pictureMode", "backlight"], None, 0);
        let old_desc = answer(&mut sio, &old, json!({"settings": {"pictureMode": "eco", "backlight": "30"}, "dimension": {"pictureMode": "eco", "dynamicRange": "sdr"}}));
        let first = sio.write("picture.pictureMode", &json!("cinema"), None, 100);
        let second = sio.write("picture.pictureMode", &json!("filmMaker"), None, 110);
        assert!(!sio.usable("picture.backlight"));
        assert!(sio.write("picture.backlight", &json!(80), None, 111).is_empty());
        assert!(sio.write_many("picture", json!({"pictureMode":"cinema", "backlight":"80"}).as_object().unwrap(), None, 111).is_empty());
        assert!(sio.update(&Input::Luna { tag: tag_of(&old_desc), payload: json!({"key":"backlight", "ui":{"active":true}}) }, 112).is_empty());
        assert!(sio.update(&Input::Luna { tag: tag_of(&second), payload: json!({"returnValue":false}) }, 113).is_empty());
        let fresh = sio.update(&Input::Luna { tag: tag_of(&first), payload: json!({"returnValue":true}) }, 114);
        let desc = sio.update(&Input::Luna { tag: tag_of(&fresh), payload: json!({"settings":{"pictureMode":"cinema", "backlight":"50"}, "dimension":{"pictureMode":"cinema", "dynamicRange":"sdr"}}) }, 115);
        assert_eq!(sio.text("picture.pictureMode"), "cinema");
        assert!(!sio.usable("picture.backlight"));
        assert!(sio.write("picture.backlight", &json!(80), None, 116).is_empty());
        let native = sio.update(&Input::Luna { tag: tag_of(&desc), payload: json!({"results":[{"key":"backlight", "ui":{"active":true}}]}) }, 117);
        assert!(!sio.usable("picture.backlight"));
        answer(&mut sio, &native, json!({"settings":{"backlight":"50"}, "dimension":{"pictureMode":"cinema","input":"default"}}));
        assert!(sio.usable("picture.backlight"));
        let adjusted = sio.write("picture.backlight", &json!(80), None, 118);
        let Effect::Luna { payload, .. } = &adjusted[0] else { panic!() };
        assert_eq!(payload["dimension"]["pictureMode"], "cinema");
    }

    #[test]
    fn picture_switch_timeout_rereads_and_failed_read_keeps_adjustments_unavailable() {
        let mut sio = Sio::new(0x5300_0000);
        let old = sio.read("picture", &["pictureMode", "backlight"], None, 0);
        let desc = answer(&mut sio, &old, json!({"settings":{"pictureMode":"eco", "backlight":"30"}, "dimension":{"pictureMode":"eco"}}));
        answer(&mut sio, &desc, json!({"results":[{"key":"backlight"}]}));
        let write = sio.write("picture.pictureMode", &json!("filmMaker"), None, 10_000);
        assert_eq!(sio.due_at(), Some(12_000));
        assert!(sio.update(&Input::Tick, 11_999).is_empty());
        let fresh = sio.update(&Input::Tick, 12_000);
        assert!(sio.update(&Input::Luna { tag: tag_of(&write), payload: json!({"returnValue":true}) }, 12_001).is_empty());
        let desc = sio.update(&Input::Luna { tag: tag_of(&fresh), payload: json!({"returnValue":false}) }, 12_002);
        sio.update(&Input::Luna { tag: tag_of(&desc), payload: json!({"returnValue":false}) }, 12_003);
        assert!(!sio.has("picture.pictureMode"));
        assert!(!sio.usable("picture.backlight"));
        assert!(sio.write("picture.backlight", &json!(80), None, 12_004).is_empty());
        sio.learn("picture", &json!({"settings":{"backlight":"70"}, "dimension":{"pictureMode":"filmMaker", "input":"hdmi1"}}));
        sio.learn_desc("picture", &json!({"key":"backlight", "ui":{"active":true}}));
        assert!(!sio.usable("picture.backlight"));
        assert!(!sio.picture_context_ready());
        assert!(sio.write("picture.backlight", &json!(80), None, 12_005).is_empty());
    }

    #[test]
    fn native_context_changes_do_not_reuse_previous_picture_values_or_descriptions() {
        let mut sio = Sio::new(0x5300_0000);
        sio.learn("picture", &json!({"settings":{"pictureMode":"eco", "backlight":"30"}, "dimension":{"pictureMode":"eco", "dynamicRange":"sdr", "input":"hdmi1"}}));
        sio.learn_desc("picture", &json!({"key":"backlight", "ui":{"active":true}}));
        assert!(sio.picture_context_ready());
        let write = sio.write("picture.pictureMode", &json!("filmMaker"), None, 100);
        assert_eq!(sio.picture_dynamic_range(), Some("sdr"));
        assert!(!sio.picture_context_ready());
        let fresh = sio.update(&Input::Luna {tag:tag_of(&write), payload:json!({"returnValue":true})}, 101);
        let desc = sio.update(&Input::Luna {tag:tag_of(&fresh), payload:json!({"settings":{"pictureMode":"filmMaker", "backlight":"60"}, "dimension":{"pictureMode":"filmMaker", "dynamicRange":"sdr", "input":"hdmi1"}})}, 102);
        sio.update(&Input::Luna {tag:tag_of(&desc), payload:json!({"key":"backlight"})}, 103);
        assert!(sio.usable("picture.backlight"));
        sio.learn("picture", &json!({"settings":{"pictureMode":"dolbyHdrCinema"}, "dimension":{"pictureMode":"dolbyHdrCinema", "dynamicRange":"dolbyHdr", "input":"hdmi2"}}));
        assert_eq!(sio.picture_dynamic_range(), Some("dolbyHdr"));
        assert!(sio.picture_context_ready());
        assert!(!sio.has("picture.backlight"));
        assert!(!sio.usable("picture.backlight"));
        assert!(sio.write("picture.backlight", &json!(80), None, 104).is_empty());
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
        assert_eq!(sio.queue.len(), 5, "the native backlight context also waits");
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
        assert!(!sio.usable("picture.backlight"), "the backlight context has not arrived yet");
        let native = answer(
            &mut sio,
            &fx,
            json!({"results": [{"category": "picture", "key": "backlight", "ui": {"active": false}, "values": {"range": {"min": 0, "max": 100, "interval": 1}}}]}),
        );
        assert!(!sio.usable("picture.backlight"), "inactive: dimmed");
        assert!(sio.shown("picture.backlight"));
        assert_eq!(sio.range("picture.backlight"), Some(Range { min: 0.0, max: 100.0, interval: Some(1.0) }));
        answer(&mut sio, &native, json!({"settings":{"backlight":"60"}, "dimension":{"pictureMode":"cinema","input":"hdmi1"}}));
        assert_eq!(sio.answers, 3);
        assert!(sio.idle());
    }

    #[test]
    fn a_write_shows_at_once_and_goes_out_as_strings_with_the_dimension_and_the_app() {
        let mut sio = Sio::new(0x5300_0000);
        let fx = sio.read("picture", &["backlight"], None, 0);
        answer(&mut sio, &fx, json!({"settings": {"backlight": "60"}, "dimension": {"pictureMode": "cinema"}}));
        let e = sio.write("picture.backlight", &json!(70), None, 10);
        assert_eq!(sio.num("picture.backlight", 0.0), 70.0, "shown before the service says anything");
        let Effect::Luna { uri, payload, subscribe, .. } = &e[0] else { panic!() };
        assert!(uri.ends_with("setSystemSettings") && !*subscribe);
        assert_eq!(*payload, json!({"settings": {"backlight": "70"}, "category": "picture", "dimension": {"pictureMode": "cinema"}}));
        let e = sio.write("aspectRatio.justScan", &json!("on"), Some("hdmi2"), 10);
        let Effect::Luna { payload, .. } = &e[0] else { panic!() };
        assert_eq!(payload["app_id"], "hdmi2");
        let e = sio.write("localeInfo", &json!({"x": 1}), None, 10);
        let Effect::Luna { payload, .. } = &e[0] else { panic!() };
        assert!(payload.get("category").is_none(), "a key outside every category is sent with none");
        let mut m = Map::new();
        m.insert("adjustments".into(), json!({"blackAndWhite": "off"}));
        let e = sio.write_many("option", &m, None, 10);
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
