//! The Denon receiver, a page of Sound: the Denon receiver plugin's switch and address, then the receiver's own settings, shown from its
//! readbacks. While it does not answer, the last values it gave stay on screen, unchangeable.
//!
//! The plugin asks first, as the cameras do: off, the page is its switch in the words of its entry in the `plugins` topic, nothing is
//! read from the receiver and the volume keys change the TV's own volume; off is one press. The address and web port are the plugin's
//! settings (`denon-receiver`), which exist only while it is on.

use super::plugin_rows::text;
use crate::host::receiver_words::words;
use crate::panels::hub;
use crate::panels::ime::{PURPOSE_DIGITS, PURPOSE_NORMAL};
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Card, Op, Row};
use serde_json::{Value, json};
use std::rc::Rc;

/// The helper's report is live while its stamp is this recent.
const LIVE_MS: f64 = 6000.0;
const ASK_MS: u64 = 2000;
/// How soon the page reads again while a change is on its way, so the next queued one follows quickly.
const FAST_MS: u64 = 400;
/// A write with no result after this long is reported as not confirmed.
const WAIT_MS: u64 = 12_000;
/// How long a row says what became of its change.
const SAY_MS: u64 = 60_000;
/// The last live settings, shown while the receiver does not answer.
const LAST: &str = "receiver.last";
/// The write on its way: `{setting, value, session, at, token, requestId | error}`.
const WRITE: &str = "receiver.write";
/// Changes made while another is on its way, only the latest per setting: `{setting: {value, session, at}}`.
const QUEUE: &str = "receiver.queue";
const ID: &str = "denon-receiver";
const TABS: [&str; 5] = ["Receiver", "Sound mode", "Levels", "Audyssey", "Zones & power"];
/// The name the switch's answer comes back under.
const SWITCH: &str = "receiver.switch";
/// The helper's channel level keys; it offers only the speakers the receiver reports as configured.
const CHANNELS: [(&str, &str); 20] = [
    ("channelFL", "Front left"),
    ("channelFR", "Front right"),
    ("channelC", "Centre"),
    ("channelSW", "Subwoofer"),
    ("channelSW2", "Subwoofer 2"),
    ("channelSL", "Surround left"),
    ("channelSR", "Surround right"),
    ("channelSBL", "Surround back left"),
    ("channelSBR", "Surround back right"),
    ("channelSB", "Surround back"),
    ("channelFHL", "Front height left"),
    ("channelFHR", "Front height right"),
    ("channelTFL", "Top front left"),
    ("channelTFR", "Top front right"),
    ("channelTML", "Top middle left"),
    ("channelTMR", "Top middle right"),
    ("channelFDL", "Front Dolby left"),
    ("channelFDR", "Front Dolby right"),
    ("channelSDL", "Surround Dolby left"),
    ("channelSDR", "Surround Dolby right"),
];

pub fn page() -> Page {
    Page::new("receiver", "Receiver", "boom-box")
        .parent("sound")
        .back("Sound")
        .tabs(&TABS)
        .reads(reads)
        .rows(rows)
        .chip(chip)
        .subtitle(subtitle)
        .answered(answered)
}

/// The plugin's entry in the `plugins` topic: none from a helper that is not there.
fn entry<'a>(c: &'a Ctx) -> Option<&'a Value> {
    hub::plugin_of(c.helper("plugins")?, ID)
}

/// The plugin is on, or not known to be off.
pub fn on(c: &Ctx) -> bool {
    entry(c).is_some_and(|e| text(e, "state") != "off")
}

/// The plugin is switched off: known to be, not merely unknown as with no helper.
pub fn off(c: &Ctx) -> bool {
    entry(c).is_some_and(|e| text(e, "state") == "off")
}

fn turn(on: bool, code: &str) -> Op {
    let body = if on { json!({"on": true, "approve": code}) } else { json!({"on": false}) };
    Op::Send { name: Some(SWITCH.into()), method: "POST", path: format!("/v1/plugins/{ID}/switch"), body: Some(body) }
}

fn reads(c: &Ctx) -> Vec<Read> {
    if on(c) { vec![Read::owner(ID), Read::Get { name: "receiver", path: "/v1/receiver/settings" }] } else { Vec::new() }
}

fn answered(c: &Ctx, name: &str, response: &Value) -> Vec<Op> {
    match name {
        // switched on or off: the plugin's settings came or went
        "topic:plugins" => return vec![Op::Refresh],
        SWITCH if response.get("ok").and_then(Value::as_bool) != Some(true) => {
            return vec![toast(&format!("Not changed: {}", response.pointer("/error/message").and_then(Value::as_str).unwrap_or("the TV did not answer")))];
        }
        _ => {}
    }
    if name == "receiver" {
        let mut ops = Vec::new();
        if let (Some(live), true) = shown(c) {
            ops.push(Op::Set(LAST.into(), live.clone()));
        }
        ops.extend(next(c));
        let hurry = in_flight(c) || c.local.get(QUEUE).as_object().is_some_and(|q| !q.is_empty());
        ops.push(Op::RefreshIn(if hurry { FAST_MS } else { ASK_MS }));
        return ops;
    }
    if let Some((setting, token)) = name.strip_prefix("receiver.set:").and_then(|n| n.rsplit_once(':')) {
        let local = c.local.get(WRITE);
        let Some(session) = local.get("session").and_then(Value::as_str).filter(|s| !s.is_empty()) else { return Vec::new() };
        let token = token.parse::<u64>().ok();
        if token.is_none() || local.get("token").and_then(Value::as_u64) != token || local.get("setting").and_then(Value::as_str) != Some(setting) {
            return Vec::new();
        }
        let status = c.docs.get(&format!("{name}.status")).as_u64().unwrap_or(0);
        let id = response.get("requestId").and_then(Value::as_str).filter(|s| !s.is_empty());
        if response.get("session").and_then(Value::as_str).is_some_and(|s| s != session) {
            return Vec::new();
        }
        let value = local.get("value").cloned().unwrap_or(Value::Null);
        if status == 409 {
            // The helper takes one change at a time; this one goes back in line unless a newer one for it waits.
            let mut queue = c.local.get(QUEUE).clone();
            if queue.get(setting).is_none() {
                queue[setting] = json!({"value": value, "session": session, "at": c.now});
            }
            return vec![Op::Set(WRITE.into(), Value::Null), Op::Set(QUEUE.into(), queue), Op::RefreshIn(FAST_MS)];
        }
        let accepted = status == 202
            && response.get("accepted").and_then(Value::as_bool) == Some(true)
            && response.get("session").and_then(Value::as_str) == Some(session);
        let state = if accepted && id.is_some() {
            json!({"setting": setting, "value": value, "requestId": id, "session": session, "token": token, "at": local.get("at")})
        } else {
            let error = response
                .pointer("/error/message")
                .and_then(Value::as_str)
                .or_else(|| response.get("error").and_then(Value::as_str))
                .unwrap_or("Receiver did not accept the change");
            json!({"setting": setting, "value": value, "error": error, "session": session, "token": token, "at": c.now})
        };
        return vec![Op::Set(WRITE.into(), state), Op::RefreshIn(FAST_MS)];
    }
    Vec::new()
}

fn stamp(v: &Value) -> f64 {
    v.get("updatedAt").and_then(Value::as_f64).unwrap_or(0.0)
}

fn newest<'a>(c: &'a Ctx<'_>) -> Option<&'a Value> {
    let topic = c.helper("receiver");
    let envelope = topic.and_then(|t| t.get("_hub"));
    let answer = c.docs.get("receiver");
    let ordered = envelope
        .zip(answer.get("boot"))
        .and_then(|(e, boot)| {
            let same_boot = !boot.is_null() && e.get("boot") == Some(boot);
            let topic_seq = e.get("seq").and_then(Value::as_u64)?;
            let answer_seq = answer.get("seq").and_then(Value::as_u64)?;
            (same_boot && answer_seq >= topic_seq).then_some(())
        })
        .is_some();
    if ordered { answer.get("value").filter(|v| v.is_object()).or(topic) } else { topic }
}

pub fn doc<'a>(c: &'a Ctx<'_>) -> Option<&'a Value> {
    let r = newest(c)?;
    let age = c.now as f64 - stamp(r);
    (c.topics.helper_up
        && r.get("reachable").and_then(Value::as_bool) == Some(true)
        && r.get("fresh").and_then(Value::as_bool) == Some(true)
        && stamp(r) > 0.0
        && age < LIVE_MS
        && age > -1000.0)
        .then_some(r)
}

fn session(r: &Value) -> Option<&str> {
    r.get("settingsSession").and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// The settings to show, and whether they are live: the receiver's current ones, or the last it gave while it does not answer.
fn shown<'a>(c: &'a Ctx<'_>) -> (Option<&'a Value>, bool) {
    if let Some(live) = doc(c).filter(|r| session(r).is_some() && r.get("settings").is_some_and(Value::is_object)) {
        return (Some(live), true);
    }
    let last = c.local.get(LAST);
    (last.is_object().then_some(last), false)
}

fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    if !on(c) {
        return Some(("Off".into(), "info"));
    }
    Some(if doc(c).is_some() {
        ("Answering".into(), "positive")
    } else if shown(c).0.is_some() {
        // the last values stay on the page; the chip alone says they are not current
        ("Not answering".into(), "info")
    } else {
        ("Not answering".into(), "danger")
    })
}
fn subtitle(c: &Ctx) -> String {
    shown(c).0.and_then(|r| r.get("model")).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("Denon receiver").into()
}

pub fn db(v: Option<f64>, half: bool) -> String {
    let Some(v) = v else { return "\u{2013}".into() };
    let sign = if v > 0.0 {
        "+"
    } else if v < 0.0 {
        "\u{2212}"
    } else {
        ""
    };
    if half { format!("{sign}{:.1} dB", v.abs()) } else { format!("{sign}{} dB", plain(v.abs())) }
}
fn plain(v: f64) -> String {
    if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") }
}

fn set(setting: &str, value: Value, now: u64, session: &str) -> Vec<Op> {
    vec![
        Op::Set(WRITE.into(), json!({"setting": setting, "value": value, "session": session, "at": now})),
        Op::Send {
            name: Some(format!("receiver.set:{setting}")),
            method: "POST",
            path: "/v1/receiver/set".into(),
            body: Some(json!({"setting": setting, "value": value, "session": session})),
        },
    ]
}

enum Fate<'a> {
    Waiting,
    Applied,
    Failed(&'a str),
    Unconfirmed,
}

/// The latest write and what became of it.
struct Write<'a> {
    setting: &'a str,
    value: &'a Value,
    age: u64,
    fate: Fate<'a>,
}

fn write<'a>(c: &'a Ctx<'_>) -> Option<Write<'a>> {
    let w = c.local.get(WRITE);
    let setting = w.get("setting")?.as_str()?;
    let age = c.now.saturating_sub(w.get("at").and_then(Value::as_u64).unwrap_or(0));
    let value = w.get("value").unwrap_or(&Value::Null);
    if let Some(e) = w.get("error").and_then(Value::as_str) {
        return Some(Write { setting, value, age, fate: Fate::Failed(e) });
    }
    let result = w.get("requestId").and_then(Value::as_str).and_then(|id| {
        newest(c)?.get("settings")?.get(setting)?.get("result").filter(|r| r.get("requestId").and_then(Value::as_str) == Some(id))
    });
    // a new connection drops what the old one had on its way
    let moved = doc(c).and_then(session).is_some_and(|s| Some(s) != w.get("session").and_then(Value::as_str));
    let fate = match result.and_then(|r| r.get("status")).and_then(Value::as_str) {
        Some("applied") => Fate::Applied,
        Some(_) => Fate::Failed(result.and_then(|r| r.get("error")).and_then(Value::as_str).unwrap_or("the receiver did not take it")),
        None if age < WAIT_MS && !moved => Fate::Waiting,
        None => Fate::Unconfirmed,
    };
    Some(Write { setting, value, age, fate })
}

fn in_flight(c: &Ctx) -> bool {
    write(c).is_some_and(|w| matches!(w.fate, Fate::Waiting))
}

/// Sends the oldest queued change once nothing is on its way and the receiver answers.
fn next(c: &Ctx) -> Vec<Op> {
    let Some(queue) = c.local.get(QUEUE).as_object().filter(|q| !q.is_empty()) else { return Vec::new() };
    let (Some(live), true) = shown(c) else { return Vec::new() };
    if in_flight(c) {
        return Vec::new();
    }
    let Some(session) = session(live) else { return Vec::new() };
    let at = |item: &Value| item.get("at").and_then(Value::as_u64).unwrap_or(0);
    let Some((key, item)) = queue.iter().min_by_key(|(_, item)| at(item)) else { return Vec::new() };
    if item.get("session").and_then(Value::as_str) != Some(session) {
        // made for an earlier connection: the receiver may be in another state now
        return vec![Op::Set(QUEUE.into(), Value::Null)];
    }
    let mut rest = queue.clone();
    rest.remove(key);
    let mut ops = vec![Op::Set(QUEUE.into(), Value::Object(rest))];
    ops.extend(set(key, item.get("value").cloned().unwrap_or(Value::Null), c.now, session));
    ops
}

/// How a row's change leaves: now, or in line behind the one on its way, where a newer value for the same setting replaces
/// the older one (a slider dragged through ten steps sends the first and the last).
#[derive(Clone)]
struct Tx {
    session: String,
    now: u64,
    busy: bool,
    queue: Value,
}
impl Tx {
    fn send(&self, key: &str, value: Value) -> Vec<Op> {
        if self.session.is_empty() {
            return Vec::new();
        }
        let mut queue = self.queue.clone();
        if self.busy {
            queue[key] = json!({"value": value, "session": self.session, "at": self.now});
            return vec![Op::Set(QUEUE.into(), queue)];
        }
        if let Some(q) = queue.as_object_mut() {
            q.remove(key);
        }
        let mut ops = vec![Op::Set(QUEUE.into(), queue)];
        ops.extend(set(key, value, self.now, &self.session));
        ops
    }
}

/// What the page shows and how its rows send.
struct View<'a> {
    r: Option<&'a Value>,
    live: bool,
    tx: Tx,
}
impl<'a> View<'a> {
    fn of(c: &'a Ctx<'_>) -> Self {
        let (r, live) = shown(c);
        let session = r.filter(|_| live).and_then(session).unwrap_or_default().to_string();
        View { r, live, tx: Tx { session, now: c.now, busy: in_flight(c), queue: c.local.get(QUEUE).clone() } }
    }
}

#[derive(Clone, Copy)]
struct H(&'static str, usize, &'static str, &'static str, &'static str);

struct Setting<'a> {
    d: Option<&'a Value>,
    live: bool,
    queued: Option<&'a Value>,
    write: Option<Write<'a>>,
    key: &'static str,
}
impl<'a> Setting<'a> {
    fn new(c: &'a Ctx<'_>, v: &View<'a>, key: &'static str) -> Self {
        let d = v.r.and_then(|r| r.get("settings")).and_then(|s| s.get(key)).filter(|d| d.is_object());
        let queued = c.local.get(QUEUE).get(key).and_then(|q| q.get("value")).filter(|x| !x.is_null());
        Self { d, live: v.live, queued, write: write(c).filter(|w| w.setting == key), key }
    }
    /// What the receiver last reported.
    fn read(&self) -> Option<&'a Value> {
        self.d.filter(|d| d.get("support").and_then(Value::as_str) == Some("supported")).and_then(|d| d.get("value")).filter(|v| !v.is_null())
    }
    /// What the row shows: the change on its way or in line, until the receiver's readback settles it.
    fn value(&self) -> Option<&'a Value> {
        let sent = self.write.as_ref().filter(|w| matches!(w.fate, Fate::Waiting)).map(|w| w.value).filter(|v| !v.is_null());
        self.queued.or(sent).or_else(|| self.read())
    }
    /// Settings the receiver model does not offer stay off the page.
    fn offered(&self) -> bool {
        self.d.and_then(|d| d.get("support")).and_then(Value::as_str) != Some("unsupported")
    }
    fn enabled(&self) -> bool {
        self.live && self.read().is_some() && self.d.and_then(|d| d.get("writable")).and_then(Value::as_bool) == Some(true)
    }
    fn detail(&self) -> String {
        if self.queued.is_some() {
            return "Queued".into();
        }
        if let Some(w) = self.write.as_ref().filter(|w| w.age < SAY_MS) {
            match w.fate {
                Fate::Waiting => return "Applying…".into(),
                Fate::Failed(e) => return format!("Not changed: {e}"),
                Fate::Unconfirmed => return "Change not confirmed".into(),
                Fate::Applied => {}
            }
        }
        if self.d.and_then(|d| d.get("pending")).is_some_and(Value::is_object) {
            return "Applying…".into();
        }
        // not answering: the chip says so once, rather than every row
        if !self.live {
            return String::new();
        }
        if let Some(reason) = self.d.and_then(|d| d.get("reason")).and_then(Value::as_str).filter(|s| !s.is_empty()) {
            return reason.into();
        }
        if self.d.and_then(|d| d.get("support")).and_then(Value::as_str) == Some("unsupported") {
            return "Not supported".into();
        }
        if self.read().is_none() {
            return "Unavailable".into();
        }
        String::new()
    }
    fn options(&self, numeric: bool, kind: Option<&str>) -> Vec<(Value, String)> {
        self.d
            .and_then(|d| d.get("options"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|o| kind.is_none() || o.get("kind").and_then(Value::as_str) == kind)
                    .filter_map(|o| {
                        let v = o.get("value")?;
                        if if numeric { !v.is_number() } else { !v.is_string() } {
                            return None;
                        }
                        let label = o.get("label")?.as_str()?.trim();
                        if label.is_empty() {
                            return None;
                        }
                        Some((v.clone(), label.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn switch(c: &Ctx, v: &View, h: H, key: &'static str) -> Row {
    let s = Setting::new(c, v, key);
    let on = s.value().and_then(Value::as_bool);
    let enabled = s.enabled() && on.is_some();
    let mut r = Row::toggle(h.0).tab(h.1).icon(h.2).label(h.3).help(h.4).on(on == Some(true)).admin().visible(s.offered());
    r.enabled = enabled;
    r.steady = true;
    r.detail = s.detail();
    if on.is_none() && r.detail.is_empty() && v.live {
        r.detail = "Unavailable".into();
    }
    let tx = v.tx.clone();
    r.act(move |a| match a {
        Arg::Toggle(x) if enabled => tx.send(key, json!(x)),
        _ => Vec::new(),
    })
}

fn choice(c: &Ctx, v: &View, h: H, key: &'static str, numeric: bool, kind: u8) -> Row {
    let s = Setting::new(c, v, key);
    let opts: Vec<_> = s.options(numeric, None).into_iter().filter(|(v, _)| key != "quickSelect" || v.as_i64().is_some()).collect();
    let index = s.value().and_then(|v| opts.iter().position(|(x, _)| x == v));
    // Quick Select reports 0 until a memory is recalled; any memory can still be chosen.
    pick(v, h, &s, opts, index, key == "quickSelect", kind)
}

/// A row of choices. `free` keeps it usable while the current value is none of them.
fn pick(v: &View, h: H, s: &Setting, opts: Vec<(Value, String)>, index: Option<usize>, free: bool, kind: u8) -> Row {
    let key = s.key;
    let labels: Vec<String> = opts.iter().map(|(_, l)| l.clone()).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut r = match kind {
        1 => Row::stepper(h.0, &labels),
        2 => {
            let mut r =
                Row::cards(h.0, labels.iter().map(|l| Card { title: l.clone(), icon: "audio-waveform".into(), sub: String::new(), face: None }).collect());
            r.cols = 4;
            r
        }
        _ => Row::seg(h.0, &refs),
    }
    .tab(h.1)
    .icon(h.2)
    .label(h.3)
    .help(h.4)
    .index(index)
    .admin()
    .visible(s.offered());
    let enabled = s.enabled() && (free || index.is_some()) && !opts.is_empty();
    r.enabled = enabled;
    r.steady = true;
    r.detail = s.detail();
    if index.is_none() && !free && r.detail.is_empty() && v.live {
        r.detail = "Unavailable".into();
    }
    let tx = v.tx.clone();
    r.act(move |a| match a {
        Arg::Index(i) if enabled => opts.get(i).map(|(v, _)| tx.send(key, v.clone())).unwrap_or_default(),
        _ => Vec::new(),
    })
}

/// The MOVIE/MUSIC/GAME/PURE buttons and the modes themselves, both written through `soundMode`. The receiver reports
/// only the mode, so a button is highlighted only when its mode belongs to that button alone.
fn sound_modes(c: &Ctx, v: &View, observed: bool) -> [Row; 2] {
    let s = Setting::new(c, v, "soundMode");
    let row = |h: H, kind: &str, field: &str| {
        let opts = s.options(false, Some(kind));
        // a mode on its way is shown as its own button; the readback names its category and style
        let current = s.queued.or(s.write.as_ref().filter(|w| matches!(w.fate, Fate::Waiting)).map(|w| w.value)).and_then(Value::as_str).filter(|x| opts.iter().any(|(o, _)| o.as_str() == Some(x)));
        let current = current.or_else(|| s.read().and(s.d).and_then(|d| d.get(field)).and_then(Value::as_str));
        let index = current.and_then(|v| opts.iter().position(|(x, _)| x.as_str() == Some(v)));
        pick(v, h, &s, opts, index, observed, 2)
    };
    let categories = row(H("modes", 1, "audio-waveform", "Category", "Movie, Music and Game recall that button's last mode; choosing it again steps to the next."), "category", "category");
    let shown = categories.visible && !categories.cards.is_empty();
    [categories.visible(shown), row(H("styles", 1, "audio-waveform", "Mode", ""), "style", "style")]
}

fn level(c: &Ctx, v: &View, h: H, key: &'static str) -> Row {
    let s = Setting::new(c, v, key);
    let number = |k| s.d.and_then(|d| d.get(k)).and_then(Value::as_f64).filter(|n| n.is_finite());
    let range = number("min").zip(number("max")).zip(number("step")).map(|((a, b), step)| (a, b, step)).filter(|(a, b, step)| a < b && *step > 0.0);
    let x = s.value().and_then(Value::as_f64).filter(|n| n.is_finite());
    let (min, max, step) = range.unwrap_or((0.0, 1.0, 1.0));
    let on_step = move |x: f64| x >= min && x <= max && ((x - min) / step - ((x - min) / step).round()).abs() < 0.00001;
    let valid = x.is_some_and(on_step);
    let enabled = s.enabled() && range.is_some() && valid;
    let unit = s.d.and_then(|d| d.get("unit")).and_then(Value::as_str).unwrap_or("");
    let mut r = Row::slider(h.0, min, max, step).tab(h.1).icon(h.2).label(h.3).help(h.4).admin().visible(s.offered());
    r.enabled = enabled;
    r.steady = true;
    r.detail = s.detail();
    r.num = x.filter(|_| valid).unwrap_or(min);
    r.value_text = Some(match x.filter(|_| valid) {
        Some(x) if unit == "dB" => db(Some(x), step.fract() != 0.0),
        Some(x) if unit.is_empty() => plain(x),
        Some(x) => format!("{} {unit}", plain(x)),
        None => "\u{2013}".into(),
    });
    if !valid && r.detail.is_empty() && v.live {
        r.detail = "Unavailable".into();
    }
    let tx = v.tx.clone();
    r.act(move |a| match a {
        Arg::Value(x) if enabled && x.is_finite() && on_step(x) => tx.send(key, json!(x)),
        _ => Vec::new(),
    })
}

fn rows(c: &Ctx) -> Vec<Row> {
    let Some(e) = entry(c) else {
        return (0..TABS.len()).map(|t| Row::empty("Not available", "The TV's helper is not answering.", "boom-box", t)).collect();
    };
    let (consent, code) = (text(e, "consent").to_string(), text(e, "approve").to_string());
    let on = on(c);
    let mut rows = vec![Row::toggle("use").tab(0).icon("boom-box").label("Denon receiver").help(&consent).on(on).detail(hub::failure(e).unwrap_or_default()).admin().act(move |a| match a {
        Arg::Toggle(true) => ask("boom-box", "Use the Denon receiver?", &consent, "Turn on", "Cancel", false, vec![turn(true, &code)]),
        Arg::Toggle(false) => vec![turn(false, "")],
        _ => Vec::new(),
    })];
    if !on {
        rows.extend((1..TABS.len()).map(|t| Row::empty("The Denon receiver is off", "Switch it on under Receiver.", "power", t)));
        return rows;
    }
    rows.extend(address(c));
    rows.extend(controls(c));
    rows
}

/// The plugin's settings: where the receiver is on the home network.
fn address(c: &Ctx) -> Vec<Row> {
    let o = c.owner(ID).filter(|o| o.known);
    let host = o.and_then(|o| o.text("host")).unwrap_or_default();
    let port = o.map(|o| (o.num("port", 0.0) as u32).to_string()).unwrap_or_default();
    let shown = |v: &str| if v.is_empty() || v == "0" { "Not set".to_string() } else { v.to_string() };
    let write = |key: &'static str, value: Value| vec![Op::Owner { owner: ID.into(), key: key.into(), value }];
    let mut host_row = Row::link("host").tab(0).icon("network").label("Address").help("The receiver's IP address or name on the home network.").value(&shown(&host)).admin().does(vec![Op::Entry {
        prompt: "Receiver address".into(),
        text: host,
        secret: false,
        purpose: PURPOSE_NORMAL,
        then: Rc::new(move |t| write("host", json!(t.trim()))),
    }]);
    if let Some(o) = o.filter(|o| !o.problem.is_empty()) {
        host_row.detail = o.problem.clone();
    }
    let port_row = Row::link("port").tab(0).icon("hash").label("Web port").help("Where the receiver answers on the network, 8080 on most Denon models.").value(&shown(&port)).admin().does(vec![Op::Entry {
        prompt: "Web port".into(),
        text: port,
        secret: false,
        purpose: PURPOSE_DIGITS,
        then: Rc::new(move |t| t.trim().parse::<u16>().ok().filter(|p| *p > 0).map(|p| write("port", json!(p))).unwrap_or_default()),
    }]);
    vec![host_row, port_row]
}

/// The receiver's own settings, on the tabs after the plugin's. Every row the receiver model offers keeps its place: what
/// cannot be changed now is disabled with the reason under it, never removed.
fn controls(c: &Ctx) -> Vec<Row> {
    let v = View::of(c);
    let mut rows = Vec::new();
    let old_helper = doc(c).is_some_and(|r| !r.get("settings").is_some_and(Value::is_object));
    rows.push(Row::note("Receiver settings unavailable", 1).visible(old_helper));
    rows.push(switch(c, &v, H("power", 1, "power", "Receiver power", "Turns on with the TV over HDMI-CEC. Off here keeps the TV on."), "power"));
    rows.push(choice(c, &v, H("input", 1, "cable", "Input", "What the receiver plays."), "input", false, 1));
    let mode_setting = Setting::new(c, &v, "soundMode");
    let mode = mode_setting
        .read()
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("unknown") && !v.eq_ignore_ascii_case("unavailable"));
    let signal = v.r.and_then(|r| r.get("signal")).and_then(Value::as_str).unwrap_or_default();
    rows.push(Row::head(&mode.map(|v| format!("Sound mode · now {}", words(v, signal))).unwrap_or_else(|| "Sound mode".into()), 1));
    rows.extend(sound_modes(c, &v, mode.is_some()));
    rows.push(choice(c, &v, H("quick", 1, "zap", "Quick Select", "Receiver memories."), "quickSelect", true, 0));
    let r = v.r;
    let absolute = r.and_then(|r| r.get("volumeDisplay")).and_then(Value::as_str) == Some("absolute");
    let (min, max) = if absolute { (0.0, 98.0) } else { (-80.0, 18.0) };
    let mut volume = Row::slider("volume", min, max, 0.5).tab(2).icon("volume-2").label("Volume");
    volume.info = true;
    volume.num = r.and_then(|r| r.get(if absolute { "volumeAbs" } else { "volumeDb" })).and_then(Value::as_f64).unwrap_or(0.0);
    volume.value_text = Some(r.and_then(|r| r.get("volumeText")).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("\u{2013}").into());
    rows.push(volume);
    rows.push(choice(c, &v, H("dialog", 2, "mic", "Dialog Enhancer", "Louder voices without louder everything."), "dialog", false, 0));
    rows.push(level(c, &v, H("dialogControl", 2, "mic", "Dialog Control", "DTS:X dialogue level."), "dialogControl"));
    rows.push(level(c, &v, H("subwoofer", 2, "speaker", "Subwoofer level", ""), "subwoofer"));
    rows.push(level(c, &v, H("lfe", 2, "speaker", "LFE level", "The low-frequency effects channel of Dolby and DTS sound."), "lfe"));
    rows.push(switch(c, &v, H("toneEnabled", 2, "sliders-vertical", "Tone control", ""), "toneEnabled"));
    for (h, key) in
        [(H("bass", 2, "sliders-vertical", "Bass", "Tone control."), "bass"), (H("treble", 2, "sliders-vertical", "Treble", "Tone control."), "treble")]
    {
        rows.push(level(c, &v, h, key));
    }
    rows.push(switch(c, &v, H("cinemaeq", 2, "theater", "Cinema EQ", "Softens bright film soundtracks."), "cinemaEq"));
    rows.push(choice(c, &v, H("drc", 2, "activity", "Dynamic compression", "Narrows the gap between loud and quiet scenes."), "drc", false, 0));
    rows.push(switch(c, &v, H("loudness", 2, "volume-1", "Loudness management", "For Dolby sound."), "loudness"));
    rows.push(switch(c, &v, H("subwooferDirect", 2, "speaker", "Subwoofer in Direct and Stereo", ""), "subwooferDirect"));
    let channels: Vec<Row> = CHANNELS
        .iter()
        .map(|&(key, label)| {
            let row = level(c, &v, H(key, 2, "speaker", label, ""), key);
            let shown = row.visible && Setting::new(c, &v, key).d.is_some();
            row.visible(shown)
        })
        .collect();
    rows.push(Row::head("Channel levels", 2).visible(channels.iter().any(|r| r.visible)));
    rows.extend(channels);
    rows.push(choice(c, &v, H("multeq", 3, "audio-lines", "MultEQ", "Audyssey room correction."), "multEq", false, 0));
    rows.push(switch(c, &v, H("dyneq", 3, "activity", "Dynamic EQ", "Keeps bass and surround full at low volume."), "dynEq"));
    rows.push(choice(c, &v, H("refoffset", 3, "gauge", "Reference level offset", "For Dynamic EQ."), "refOffset", true, 0));
    rows.push(choice(c, &v, H("dynvol", 3, "volume-1", "Dynamic Volume", "Evens out loud adverts and quiet dialogue."), "dynVol", false, 0));
    let zone2 = Setting::new(c, &v, "zone2Power").offered();
    rows.push(Row::head("Zone 2", 4).visible(zone2));
    rows.push(switch(c, &v, H("zone2power", 4, "power", "Zone 2 power", "The second pair of speakers."), "zone2Power"));
    rows.push(level(c, &v, H("zone2volume", 4, "volume-2", "Zone 2 volume", ""), "zone2Volume"));
    rows.push(choice(c, &v, H("zone2input", 4, "cable", "Zone 2 input", ""), "zone2Input", false, 1));
    rows.push(switch(c, &v, H("zone2mute", 4, "volume-1", "Zone 2 mute", ""), "zone2Mute"));
    let eco = choice(c, &v, H("eco", 4, "leaf", "ECO mode", "Less power at low volume."), "eco", false, 0);
    rows.push(Row::head("Power", 4).visible(eco.visible));
    rows.push(eco);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};

    const NOW: u64 = 1_700_000_100_000;
    fn descriptor(value: Value) -> Value {
        json!({"support":"supported","value":value,"updatedAt":NOW-1000,"writable":true,"options":[],"pending":null,"result":null})
    }
    fn world() -> World {
        let mut w = World::new(lg_with(&[], &[]));
        w.now = NOW;
        w.topics.helper_up = true;
        w.topics.set_helper(
            "receiver",
            json!({"host":"one","settingsSession":"session-1","reachable":true,"fresh":true,"updatedAt":NOW-1000,"volumeDb":-34.5,"volumeText":"-34.5 dB","settings":{}}),
        );
        plugin(&mut w, "connected");
        w
    }
    fn plugin(w: &mut World, state: &str) {
        w.topics.set_helper("plugins", json!({"firstParty": [{"id": ID, "state": state, "consent": "Connects to it.", "approve": "c0de"}]}));
    }
    fn put(w: &mut World, key: &str, d: Value) {
        let mut doc = w.topics.helper("receiver").unwrap().clone();
        doc["settings"][key] = d;
        w.topics.set_helper("receiver", doc);
    }
    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap()
    }
    fn send(r: &Row, arg: Arg) -> Value {
        let ops = (r.act.as_ref().unwrap())(arg);
        let [Op::Set(_, _), Op::Set(_, _), Op::Send { name: Some(name), method: "POST", path, body: Some(body) }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(name, &format!("receiver.set:{}", body["setting"].as_str().unwrap()));
        assert_eq!(path, "/v1/receiver/set");
        body.clone()
    }
    fn apply(w: &mut World, ops: &[Op]) {
        for op in ops {
            if let Op::Set(key, v) = op {
                w.local.set(key, v.clone());
            }
        }
    }
    #[test]
    fn capabilities_are_per_setting_and_types_are_strict() {
        let mut w = world();
        put(&mut w, "power", descriptor(json!(false)));
        put(&mut w, "cinemaEq", descriptor(json!("off")));
        let mut bass = descriptor(json!(1));
        bass["writable"] = json!(false);
        bass["reason"] = json!("Direct mode");
        bass["min"] = json!(-6);
        bass["max"] = json!(6);
        bass["step"] = json!(1);
        put(&mut w, "bass", bass);
        let r = rows(&w.ctx());
        assert!(row(&r, "power").enabled && !row(&r, "power").on);
        assert!(!row(&r, "cinemaeq").enabled && !row(&r, "cinemaeq").on);
        assert_eq!(row(&r, "bass").detail, "Direct mode");
        assert!(!row(&r, "bass").enabled);
        assert!(!row(&r, "eco").enabled);
        assert_eq!(row(&r, "volume").value_text.as_deref(), Some("-34.5 dB"));
    }
    #[test]
    fn advertised_values_and_zone_scale_are_preserved() {
        let mut w = world();
        let mut quick = descriptor(json!(4));
        quick["options"] = json!([{"value":2,"label":"TV"},{"value":4,"label":"Game"}]);
        put(&mut w, "quickSelect", quick);
        let mut z = descriptor(json!(41.5));
        z["min"] = json!(0);
        z["max"] = json!(98);
        z["step"] = json!(0.5);
        z["unit"] = json!("absolute");
        put(&mut w, "zone2Volume", z);
        let mut input = descriptor(json!("SOURCE"));
        input["options"] = json!([{"value":"SOURCE","label":"Follow main"},{"value":"CD","label":"CD"}]);
        put(&mut w, "zone2Input", input);
        let r = rows(&w.ctx());
        assert_eq!(row(&r, "quick").index, Some(1));
        assert_eq!(send(row(&r, "quick"), Arg::Index(0)), json!({"setting":"quickSelect","value":2,"session":"session-1"}));
        assert_eq!((row(&r, "zone2volume").min, row(&r, "zone2volume").max, row(&r, "zone2volume").step), (0.0, 98.0, 0.5));
        assert_eq!(send(row(&r, "zone2volume"), Arg::Value(42.5)), json!({"setting":"zone2Volume","value":42.5,"session":"session-1"}));
        assert!((row(&r, "zone2volume").act.as_ref().unwrap())(Arg::Value(42.25)).is_empty());
        assert_eq!(send(row(&r, "zone2input"), Arg::Index(1)), json!({"setting":"zone2Input","value":"CD","session":"session-1"}));
    }
    #[test]
    fn actual_sound_mode_is_shown_separately_from_writable_targets() {
        let mut w = world();
        let mut mode = descriptor(json!("DOLBY AUDIO-DD+DSUR"));
        mode["style"] = json!("DOLBY DIGITAL");
        mode["category"] = Value::Null;
        mode["options"] = json!([
            {"value":"MOVIE","label":"Movie","kind":"category"},{"value":"MUSIC","label":"Music","kind":"category"},
            {"value":"STEREO","label":"Stereo","kind":"style"},{"value":"DOLBY DIGITAL","label":"Dolby","kind":"style"}
        ]);
        put(&mut w, "soundMode", mode.clone());
        let r = rows(&w.ctx());
        assert!(r.iter().any(|r| r.text == "Sound mode · now Dolby Digital Plus \u{b7} Dolby Surround"));
        assert_eq!(row(&r, "modes").index, None);
        assert!(row(&r, "modes").enabled);
        assert_eq!(send(row(&r, "modes"), Arg::Index(1)), json!({"setting":"soundMode","value":"MUSIC","session":"session-1"}));
        assert_eq!(row(&r, "styles").index, Some(1));
        assert_eq!(send(row(&r, "styles"), Arg::Index(0)), json!({"setting":"soundMode","value":"STEREO","session":"session-1"}));
        mode["category"] = json!("MOVIE");
        put(&mut w, "soundMode", mode.clone());
        assert_eq!(row(&rows(&w.ctx()), "modes").index, Some(0));
        mode["options"] = json!([{"value":"STEREO","label":"Stereo","kind":"style"}]);
        mode["value"] = json!("unknown");
        put(&mut w, "soundMode", mode);
        let r = rows(&w.ctx());
        assert!(!row(&r, "modes").visible);
        assert!(!row(&r, "styles").enabled);
        assert!(r.iter().any(|r| r.text == "Sound mode"));
    }

    #[test]
    fn rows_follow_what_the_receiver_model_offers() {
        let mut w = world();
        let mut eco = descriptor(json!("AUTO"));
        eco["support"] = json!("unsupported");
        put(&mut w, "eco", eco);
        let mut front = descriptor(json!(-1.5));
        front["min"] = json!(-12);
        front["max"] = json!(12);
        front["step"] = json!(0.5);
        front["unit"] = json!("dB");
        put(&mut w, "channelFL", front);
        let mut quick = descriptor(json!(0));
        quick["options"] = json!([{"value":1,"label":"1"},{"value":2,"label":"2"}]);
        put(&mut w, "quickSelect", quick);
        let r = rows(&w.ctx());
        assert!(!row(&r, "eco").visible);
        assert!(!r.iter().any(|r| r.text == "Power" && r.visible));
        assert!(row(&r, "channelFL").visible && !row(&r, "channelC").visible);
        assert_eq!(send(row(&r, "channelFL"), Arg::Value(-2.0)), json!({"setting":"channelFL","value":-2.0,"session":"session-1"}));
        assert!((row(&r, "channelFL").act.as_ref().unwrap())(Arg::Value(12.5)).is_empty());
        assert_eq!(send(row(&r, "quick"), Arg::Index(1)), json!({"setting":"quickSelect","value":2,"session":"session-1"}));
    }

    #[test]
    fn a_write_shows_on_its_row_until_the_readback_settles_it() {
        let mut w = world();
        put(&mut w, "power", descriptor(json!(true)));
        put(&mut w, "dynEq", descriptor(json!(true)));
        w.local.set(WRITE, json!({"setting":"power","value":false,"session":"session-1","token":1,"at":NOW}));
        w.docs.set("receiver.set:power:1.status", json!(202));
        let ops = answered(&w.ctx(), "receiver.set:power:1", &json!({"accepted":true,"requestId":"r1","session":"session-1"}));
        apply(&mut w, &ops);
        let r = rows(&w.ctx());
        assert!(!row(&r, "power").on && row(&r, "power").enabled, "the change shows at once and the row stays usable");
        assert_eq!(row(&r, "power").detail, "Applying…");
        assert!(row(&r, "dyneq").enabled && row(&r, "dyneq").detail.is_empty(), "other rows are not held up");
        let mut d = descriptor(json!(true));
        d["result"] = json!({"requestId":"r1","status":"failed","error":"No readback"});
        put(&mut w, "power", d.clone());
        let r = rows(&w.ctx());
        assert!(row(&r, "power").on, "a failed change falls back to the readback");
        assert_eq!(row(&r, "power").detail, "Not changed: No readback");
        d["result"] = json!({"requestId":"r1","status":"applied","error":null});
        d["value"] = json!(false);
        put(&mut w, "power", d);
        let r = rows(&w.ctx());
        assert!(!row(&r, "power").on && row(&r, "power").detail.is_empty());
    }

    #[test]
    fn changes_queue_behind_the_one_on_its_way_latest_value_wins() {
        let mut w = world();
        let mut bass = descriptor(json!(0));
        (bass["min"], bass["max"], bass["step"]) = (json!(-6), json!(6), json!(1));
        put(&mut w, "bass", bass);
        put(&mut w, "power", descriptor(json!(true)));
        w.local.set(WRITE, json!({"setting":"power","value":false,"requestId":"r1","session":"session-1","token":1,"at":NOW}));
        for x in [1.0, 2.0, 3.0] {
            let ops = (row(&rows(&w.ctx()), "bass").act.as_ref().unwrap())(Arg::Value(x));
            assert!(!ops.iter().any(|o| matches!(o, Op::Send { .. })), "nothing is sent while a change is on its way");
            apply(&mut w, &ops);
        }
        let r = rows(&w.ctx());
        assert_eq!((row(&r, "bass").num, row(&r, "bass").detail.as_str()), (3.0, "Queued"));
        let mut d = descriptor(json!(false));
        d["result"] = json!({"requestId":"r1","status":"applied","error":null});
        put(&mut w, "power", d);
        let ops = answered(&w.ctx(), "receiver", &Value::Null);
        let sent: Vec<_> = ops.iter().filter_map(|o| if let Op::Send { body: Some(b), .. } = o { Some(b.clone()) } else { None }).collect();
        assert_eq!(sent, [json!({"setting":"bass","value":3.0,"session":"session-1"})], "only the latest value is sent");
        apply(&mut w, &ops);
        assert_eq!(w.local.get(QUEUE), &json!({}));
        w.local.set(WRITE, json!({"setting":"bass","value":3.0,"session":"session-1","token":2,"at":NOW}));
        w.docs.set("receiver.set:bass:2.status", json!(409));
        let ops = answered(&w.ctx(), "receiver.set:bass:2", &json!({"error":{"message":"busy"}}));
        apply(&mut w, &ops);
        assert_eq!((w.local.get(WRITE), &w.local.get(QUEUE)["bass"]["value"]), (&Value::Null, &json!(3.0)), "a busy answer puts the change back in line");
    }

    #[test]
    fn changed_session_discards_pending_and_old_response() {
        let mut w = world();
        put(&mut w, "power", descriptor(json!(true)));
        w.local.set(WRITE, json!({"setting":"power","session":"session-1","token":2,"at":NOW}));
        let mut changed = w.topics.helper("receiver").unwrap().clone();
        changed["settingsSession"] = json!("session-2");
        w.topics.set_helper("receiver", changed);
        assert!(row(&rows(&w.ctx()), "power").enabled);
        assert!(!in_flight(&w.ctx()));
        assert!(answered(&w.ctx(), "receiver.set:power:1", &json!({"accepted":true,"session":"session-1","requestId":"old"})).is_empty());
        let mut no_session = w.topics.helper("receiver").unwrap().clone();
        no_session["settingsSession"] = Value::Null;
        w.topics.set_helper("receiver", no_session);
        assert!(!row(&rows(&w.ctx()), "power").enabled);
    }

    #[test]
    fn last_values_stay_in_place_while_the_receiver_does_not_answer() {
        let mut w = world();
        put(&mut w, "power", descriptor(json!(true)));
        put(&mut w, "dynEq", descriptor(json!(false)));
        let ops = answered(&w.ctx(), "receiver", &Value::Null);
        apply(&mut w, &ops);
        let layout = |r: &[Row]| r.iter().map(|r| (r.id.clone(), r.text.clone(), r.visible, r.steady)).collect::<Vec<_>>();
        let live = rows(&w.ctx());
        w.local.set(WRITE, json!({"setting":"dynEq","value":true,"requestId":"r1","session":"session-1","at":NOW}));
        let mut disconnected = w.topics.helper("receiver").unwrap().clone();
        disconnected["reachable"] = json!(false);
        disconnected["settingsSession"] = Value::Null;
        disconnected["settings"]["dynEq"]["result"] = json!({"requestId":"r1","status":"failed","error":"Connection lost"});
        w.topics.set_helper("receiver", disconnected);
        let r = rows(&w.ctx());
        assert_eq!(layout(&r), layout(&live), "no row comes, goes or changes height");
        assert!(row(&r, "power").on && !row(&r, "power").enabled && row(&r, "power").detail.is_empty());
        assert_eq!(row(&r, "dyneq").detail, "Not changed: Connection lost");
        assert_eq!(chip(&w.ctx()), Some(("Not answering".into(), "info")));
        let mut old = w.topics.helper("receiver").unwrap().clone();
        old["reachable"] = json!(true);
        old.as_object_mut().unwrap().remove("settings");
        w.topics.set_helper("receiver", old);
        let r = controls(&w.ctx());
        assert!(r[0].visible && r[0].text == "Receiver settings unavailable");
    }

    #[test]
    fn off_the_page_is_the_plugins_switch_and_nothing_is_read() {
        let mut w = world();
        plugin(&mut w, "off");
        assert!(reads(&w.ctx()).is_empty());
        let r = rows(&w.ctx());
        assert!(!row(&r, "use").on);
        assert_eq!(r.iter().map(|r| r.tab).collect::<Vec<_>>(), [0, 1, 2, 3, 4], "the switch, and a line on each of the receiver's tabs");
        let [Op::Ask(question)] = &(row(&r, "use").act.as_ref().unwrap())(Arg::Toggle(true))[..] else { panic!("it asks first") };
        assert_eq!(question.body, "Connects to it.");
        assert!(matches!(&question.answers[1].then[..], [Op::Send { path, body: Some(b), .. }] if path == "/v1/plugins/denon-receiver/switch" && b == &json!({"on": true, "approve": "c0de"})));
    }

    #[test]
    fn a_plugin_stopped_by_an_error_keeps_its_switch_on_and_says_why_under_it() {
        let mut w = world();
        w.topics.set_helper(
            "plugins",
            json!({"firstParty": [{"id": ID, "state": "failed", "reason": "Stopped by an internal error: boom", "consent": "Connects to it.", "approve": "c0de"}]}),
        );
        let r = rows(&w.ctx());
        assert!(row(&r, "use").on, "turning it off and on again is how to restart it");
        assert_eq!(row(&r, "use").detail, "Stopped by an internal error: boom");
        plugin(&mut w, "connected");
        assert!(row(&rows(&w.ctx()), "use").detail.is_empty());
    }

    #[test]
    fn on_the_address_is_the_plugins_and_a_port_must_be_one() {
        let w = world();
        assert_eq!(reads(&w.ctx()).len(), 2);
        let r = rows(&w.ctx());
        assert!(matches!(&(row(&r, "use").act.as_ref().unwrap())(Arg::Toggle(false))[..], [Op::Send { body: Some(b), .. }] if b == &json!({"on": false})));
        assert_eq!(row(&r, "power").tab, 1);
        let typed = (row(&r, "port").act.as_ref().unwrap())(Arg::Press);
        let [Op::Entry { then, .. }] = &typed[..] else { panic!("the port is typed") };
        assert!(matches!(&then(" 8080 ".into())[..], [Op::Owner { owner, key, value }] if owner == ID && key == "port" && value == &json!(8080)));
        assert!(then("0".into()).is_empty() && then("70000".into()).is_empty());
    }
}

#[cfg(test)]
mod ordering_tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};

    #[test]
    fn sequence_and_boot_prevent_old_target_resurrection() {
        let mut w = World::new(lg_with(&[], &[]));
        let first = json!({"settingsSession":"old","updatedAt":500});
        let second = json!({"settingsSession":"new","updatedAt":400,"_hub":{"boot":7,"seq":12}});
        w.topics.set_helper("receiver", second);
        w.docs.set("receiver", json!({"boot":7,"seq":11,"value":first}));
        assert_eq!(newest(&w.ctx()).unwrap()["settingsSession"], "new");
        w.docs.set("receiver", json!({"boot":6,"seq":99,"value":first}));
        assert_eq!(newest(&w.ctx()).unwrap()["settingsSession"], "new");
        w.docs.set("receiver", json!({"boot":7,"seq":12,"value":first}));
        assert_eq!(newest(&w.ctx()).unwrap()["settingsSession"], "old");
    }
}
