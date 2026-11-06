//! The Denon receiver, a page of Sound: the Denon receiver plugin's switch and address, then the receiver's own settings, shown only from
//! independent, fresh receiver readbacks.
//!
//! The plugin asks first, as the cameras do: off, the page is its switch in the words of its entry in the `plugins` topic, nothing is
//! read from the receiver and the volume keys change the TV's own volume; off is one press. The address and web port are the plugin's
//! settings (`denon-receiver`), which exist only while it is on.

use super::plugin_rows::text;
use crate::panels::hub;
use crate::panels::ime::{PURPOSE_DIGITS, PURPOSE_NORMAL};
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Card, Op, Row};
use serde_json::{Value, json};
use std::rc::Rc;

pub const STALE_MS: f64 = 6000.0;
const ASK_MS: u64 = 2000;
const ID: &str = "denon-receiver";
const TABS: [&str; 5] = ["Receiver", "Sound mode", "Levels", "Audyssey", "Zones & power"];
/// The name the switch's answer comes back under.
const SWITCH: &str = "receiver.switch";

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

fn on(c: &Ctx) -> bool {
    entry(c).is_some_and(|e| text(e, "state") != "off")
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
        return vec![Op::RefreshIn(ASK_MS)];
    }
    if let Some((setting, token)) = name.strip_prefix("receiver.set:").and_then(|n| n.rsplit_once(':')) {
        let local = c.local.get("receiver.write");
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
        let accepted = status == 202
            && response.get("accepted").and_then(Value::as_bool) == Some(true)
            && response.get("session").and_then(Value::as_str) == Some(session);
        let state = if accepted && id.is_some() {
            json!({"setting": setting, "requestId": id, "session": session, "token": token, "at": c.now})
        } else {
            let error = response
                .pointer("/error/message")
                .and_then(Value::as_str)
                .or_else(|| response.get("error").and_then(Value::as_str))
                .unwrap_or("Receiver did not accept the change");
            json!({"setting": setting, "error": error, "session": session, "token": token, "at": c.now})
        };
        return vec![Op::Set("receiver.write".into(), state)];
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
        && age < STALE_MS
        && age > -1000.0)
        .then_some(r)
}

fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    if !on(c) {
        return Some(("Off".into(), "info"));
    }
    Some(if doc(c).is_some() { ("Answering".into(), "positive") } else { ("Not answering".into(), "danger") })
}
fn subtitle(c: &Ctx) -> String {
    doc(c).and_then(|r| r.get("model")).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("Denon receiver").into()
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

fn set(setting: &'static str, value: Value, now: u64, session: &str) -> Vec<Op> {
    vec![
        Op::Set("receiver.write".into(), json!({"setting": setting, "session": session, "at": now})),
        Op::Send {
            name: Some(format!("receiver.set:{setting}")),
            method: "POST",
            path: "/v1/receiver/set".into(),
            body: Some(json!({"setting": setting, "value": value, "session": session})),
        },
    ]
}

#[derive(Clone, Copy)]
struct H(&'static str, usize, &'static str, &'static str, &'static str);

struct Setting<'a> {
    d: Option<&'a Value>,
    local: &'a Value,
    now: u64,
    busy: bool,
    has_session: bool,
    key: &'static str,
}
impl<'a> Setting<'a> {
    fn new(c: &'a Ctx<'_>, r: Option<&'a Value>, key: &'static str) -> Self {
        let settings = r.and_then(|r| r.get("settings"));
        let d = settings.and_then(|s| s.get(key)).filter(|d| d.is_object());
        let session = r.and_then(|r| r.get("settingsSession")).and_then(Value::as_str).filter(|s| !s.is_empty());
        let local = if session.is_some() && c.local.get("receiver.write").get("session").and_then(Value::as_str) == session {
            c.local.get("receiver.write")
        } else {
            &Value::Null
        };
        let local_age = c.now.saturating_sub(local.get("at").and_then(Value::as_u64).unwrap_or(0));
        let local_wait = local.get("setting").and_then(Value::as_str).is_some()
            && local.get("error").is_none()
            && local_age < STALE_MS as u64
            && !local.get("requestId").and_then(Value::as_str).is_some_and(|id| {
                settings.and_then(Value::as_object).is_some_and(|all| all.values().any(|s| s.pointer("/result/requestId").and_then(Value::as_str) == Some(id)))
            });
        let busy = local_wait || settings.and_then(Value::as_object).is_some_and(|all| all.values().any(|s| s.get("pending").is_some_and(Value::is_object)));
        Self { d, local, now: c.now, busy, key, has_session: session.is_some() }
    }
    fn fresh(&self) -> bool {
        let t = self.d.map(stamp).unwrap_or(0.0);
        let age = self.now as f64 - t;
        t > 0.0 && age < STALE_MS && age > -1000.0
    }
    fn value(&self) -> Option<&Value> {
        self.d
            .filter(|d| d.get("support").and_then(Value::as_str) == Some("supported"))
            .filter(|_| self.fresh())
            .and_then(|d| d.get("value"))
            .filter(|v| !v.is_null())
    }
    fn enabled(&self) -> bool {
        self.has_session && self.value().is_some() && !self.busy && self.d.and_then(|d| d.get("writable")).and_then(Value::as_bool) == Some(true)
    }
    fn detail(&self) -> String {
        if self.local.get("setting").and_then(Value::as_str) == Some(self.key) {
            let age = self.now.saturating_sub(self.local.get("at").and_then(Value::as_u64).unwrap_or(0));
            if age < STALE_MS as u64 {
                if let Some(e) = self.local.get("error").and_then(Value::as_str) {
                    return e.into();
                }
            }
            if let Some(id) = self.local.get("requestId").and_then(Value::as_str) {
                if let Some(result) = self.d.and_then(|d| d.get("result"))
                    && result.get("requestId").and_then(Value::as_str) == Some(id)
                    && result.get("status").and_then(Value::as_str) == Some("failed")
                {
                    return result.get("error").and_then(Value::as_str).unwrap_or("Change failed").into();
                }
                let applied = self
                    .d
                    .and_then(|d| d.get("result"))
                    .is_some_and(|r| r.get("requestId").and_then(Value::as_str) == Some(id) && r.get("status").and_then(Value::as_str) == Some("applied"));
                if !applied && age >= STALE_MS as u64 && age < 2 * STALE_MS as u64 && self.d.and_then(|d| d.get("pending")).is_none_or(Value::is_null) {
                    return "Change not confirmed".into();
                }
            }
        }
        if self.d.and_then(|d| d.get("pending")).is_some_and(Value::is_object)
            || (self.busy && self.local.get("setting").and_then(Value::as_str) == Some(self.key))
        {
            return "Applying…".into();
        }
        if self.busy {
            return "Another change is pending".into();
        }
        if let Some(reason) = self.d.and_then(|d| d.get("reason")).and_then(Value::as_str).filter(|s| !s.is_empty()) {
            return reason.into();
        }
        if self.d.and_then(|d| d.get("support")).and_then(Value::as_str) == Some("unsupported") {
            return "Not supported".into();
        }
        if self.value().is_none() {
            return "Unavailable".into();
        }
        if !self.has_session {
            return "Receiver unavailable".into();
        }
        if !self.enabled() {
            return "Read only".into();
        }
        String::new()
    }
    fn options(&self, numeric: bool) -> Vec<(Value, String)> {
        self.d
            .and_then(|d| d.get("options"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
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

fn switch(c: &Ctx, live: Option<&Value>, h: H, key: &'static str) -> Row {
    let s = Setting::new(c, live, key);
    let v = s.value().and_then(Value::as_bool);
    let enabled = s.enabled() && v.is_some();
    let mut r = Row::toggle(h.0).tab(h.1).icon(h.2).label(h.3).help(h.4).on(v == Some(true)).admin();
    r.enabled = enabled;
    r.detail = s.detail();
    if v.is_none() && r.detail.is_empty() {
        r.detail = "Unavailable".into();
    }
    let now = c.now;
    let session = live.and_then(|d| d.get("settingsSession")).and_then(Value::as_str).unwrap_or("").to_string();
    r.act(move |a| match a {
        Arg::Toggle(x) if enabled && !session.is_empty() => set(key, json!(x), now, &session),
        _ => Vec::new(),
    })
}

fn choice(c: &Ctx, live: Option<&Value>, h: H, key: &'static str, numeric: bool, kind: u8) -> Row {
    let s = Setting::new(c, live, key);
    let opts: Vec<_> = s.options(numeric).into_iter().filter(|(v, _)| key != "quickSelect" || v.as_i64().is_some()).collect();
    let index = s.value().and_then(|v| opts.iter().position(|(x, _)| x == v));
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
    .admin();
    let observed_mode = key == "soundMode"
        && s.value().and_then(Value::as_str).is_some_and(|v| {
            let value = v.trim();
            !value.is_empty() && !value.eq_ignore_ascii_case("unknown") && !value.eq_ignore_ascii_case("unavailable")
        });
    let enabled = s.enabled() && (if key == "soundMode" { observed_mode } else { index.is_some() }) && !opts.is_empty();
    r.enabled = enabled;
    r.detail = s.detail();
    if index.is_none() && !observed_mode && r.detail.is_empty() {
        r.detail = "Unavailable".into();
    }
    let now = c.now;
    let session = live.and_then(|d| d.get("settingsSession")).and_then(Value::as_str).unwrap_or("").to_string();
    r.act(move |a| match a {
        Arg::Index(i) if enabled && !session.is_empty() => opts.get(i).map(|(v, _)| set(key, v.clone(), now, &session)).unwrap_or_default(),
        _ => Vec::new(),
    })
}

fn level(c: &Ctx, live: Option<&Value>, h: H, key: &'static str) -> Row {
    let s = Setting::new(c, live, key);
    let number = |k| s.d.and_then(|d| d.get(k)).and_then(Value::as_f64).filter(|n| n.is_finite());
    let range = number("min").zip(number("max")).zip(number("step")).map(|((a, b), step)| (a, b, step)).filter(|(a, b, step)| a < b && *step > 0.0);
    let v = s.value().and_then(Value::as_f64).filter(|n| n.is_finite());
    let (min, max, step) = range.unwrap_or((0.0, 1.0, 1.0));
    let valid = v.is_some_and(|x| x >= min && x <= max && ((x - min) / step - ((x - min) / step).round()).abs() < 0.00001);
    let enabled = s.enabled() && range.is_some() && valid;
    let unit = s.d.and_then(|d| d.get("unit")).and_then(Value::as_str).unwrap_or("");
    let mut r = Row::slider(h.0, min, max, step).tab(h.1).icon(h.2).label(h.3).help(h.4).admin();
    r.enabled = enabled;
    r.detail = s.detail();
    r.num = v.filter(|_| valid).unwrap_or(min);
    r.value_text = Some(match v.filter(|_| valid) {
        Some(x) if unit == "dB" => db(Some(x), step.fract() != 0.0),
        Some(x) if unit.is_empty() => plain(x),
        Some(x) => format!("{} {unit}", plain(x)),
        None => "\u{2013}".into(),
    });
    if !valid && r.detail.is_empty() {
        r.detail = "Unavailable".into();
    }
    let now = c.now;
    let session = live.and_then(|d| d.get("settingsSession")).and_then(Value::as_str).unwrap_or("").to_string();
    r.act(move |a| match a {
        Arg::Value(x)
            if enabled && !session.is_empty() && x.is_finite() && x >= min && x <= max && ((x - min) / step - ((x - min) / step).round()).abs() < 0.00001 =>
        {
            set(key, json!(x), now, &session)
        }
        _ => Vec::new(),
    })
}

fn rows(c: &Ctx) -> Vec<Row> {
    let Some(e) = entry(c) else {
        return (0..TABS.len()).map(|t| Row::empty("Not available", "The TV's helper is not answering.", "boom-box", t)).collect();
    };
    let (consent, code) = (text(e, "consent").to_string(), text(e, "approve").to_string());
    let on = on(c);
    let mut rows = vec![Row::toggle("use").tab(0).icon("boom-box").label("Denon receiver").help(&consent).on(on).admin().act(move |a| match a {
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

/// The receiver's own settings, on the tabs after the plugin's.
fn controls(c: &Ctx) -> Vec<Row> {
    let live = doc(c);
    let mut rows = Vec::new();
    let why = if live.is_none() {
        "Receiver not answering"
    } else if live.and_then(|r| r.get("settings")).and_then(Value::as_object).is_none() {
        "Receiver settings unavailable"
    } else {
        ""
    };
    rows.push(Row::note(why, 1).visible(!why.is_empty()));
    let local = c.local.get("receiver.write");
    let local_error = local.get("error").and_then(Value::as_str);
    let failure = if live.is_none() { local_error } else { None }.or_else(|| {
        newest(c).filter(|r| live.is_none() || r.get("settingsSession").and_then(Value::as_str) != local.get("session").and_then(Value::as_str)).and_then(|r| {
            if let Some(error) = local_error {
                return Some(error);
            }
            let setting = local.get("setting")?.as_str()?;
            let request = local.get("requestId")?.as_str()?;
            let result = r.get("settings")?.get(setting)?.get("result")?;
            (result.get("requestId")?.as_str()? == request && result.get("status")?.as_str()? == "failed")
                .then(|| result.get("error").and_then(Value::as_str).unwrap_or("Change failed"))
        })
    });
    if let Some(error) = failure {
        for tab in 1..TABS.len() {
            let mut note = Row::note(&format!("Change failed: {error}"), tab);
            note.id = format!("receiver.write.failure:{tab}");
            rows.push(note);
        }
    }
    rows.push(switch(c, live, H("power", 1, "power", "Receiver power", "Turns on with the TV over HDMI-CEC. Off here keeps the TV on."), "power"));
    rows.push(choice(c, live, H("input", 1, "cable", "Input", "What the receiver plays."), "input", false, 1));
    let mode_setting = Setting::new(c, live, "soundMode");
    let mode = mode_setting
        .value()
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("unknown") && !v.eq_ignore_ascii_case("unavailable"));
    rows.push(Row::head(&mode.map(|v| format!("Sound mode · now {v}")).unwrap_or_else(|| "Sound mode".into()), 1));
    rows.push(choice(c, live, H("modes", 1, "audio-waveform", "Sound mode", ""), "soundMode", false, 2));
    rows.push(choice(c, live, H("quick", 1, "zap", "Quick Select", "Receiver memories."), "quickSelect", true, 0));
    let absolute = live.and_then(|r| r.get("volumeDisplay")).and_then(Value::as_str) == Some("absolute");
    let (min, max) = if absolute { (0.0, 98.0) } else { (-80.0, 18.0) };
    let mut volume = Row::slider("volume", min, max, 0.5).tab(2).icon("volume-2").label("Volume");
    volume.info = true;
    volume.num = live.and_then(|r| r.get(if absolute { "volumeAbs" } else { "volumeDb" })).and_then(Value::as_f64).unwrap_or(0.0);
    volume.value_text = Some(live.and_then(|r| r.get("volumeText")).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("\u{2013}").into());
    rows.push(volume);
    rows.push(choice(c, live, H("dialog", 2, "mic", "Dialog Enhancer", "Louder voices without louder everything."), "dialog", false, 0));
    rows.push(switch(c, live, H("subwooferEnabled", 2, "speaker", "Subwoofer adjustment", ""), "subwooferEnabled"));
    rows.push(level(c, live, H("subwoofer", 2, "speaker", "Subwoofer adjustment level", ""), "subwoofer"));
    rows.push(switch(c, live, H("toneEnabled", 2, "sliders-vertical", "Tone control", ""), "toneEnabled"));
    for (h, key) in
        [(H("bass", 2, "sliders-vertical", "Bass", "Tone control."), "bass"), (H("treble", 2, "sliders-vertical", "Treble", "Tone control."), "treble")]
    {
        rows.push(level(c, live, h, key));
    }
    rows.push(switch(c, live, H("cinemaeq", 2, "theater", "Cinema EQ", "Softens bright film soundtracks."), "cinemaEq"));
    rows.push(choice(c, live, H("multeq", 3, "audio-lines", "MultEQ", "Audyssey room correction."), "multEq", false, 0));
    rows.push(switch(c, live, H("dyneq", 3, "activity", "Dynamic EQ", "Keeps bass and surround full at low volume."), "dynEq"));
    rows.push(choice(c, live, H("refoffset", 3, "gauge", "Reference level offset", "For Dynamic EQ."), "refOffset", true, 0));
    rows.push(choice(c, live, H("dynvol", 3, "volume-1", "Dynamic Volume", "Evens out loud adverts and quiet dialogue."), "dynVol", false, 0));
    rows.push(Row::head("Zone 2", 4));
    rows.push(switch(c, live, H("zone2power", 4, "power", "Zone 2 power", "The second pair of speakers."), "zone2Power"));
    rows.push(level(c, live, H("zone2volume", 4, "volume-2", "Zone 2 volume", ""), "zone2Volume"));
    rows.push(choice(c, live, H("zone2input", 4, "cable", "Zone 2 input", ""), "zone2Input", false, 1));
    rows.push(Row::head("Power", 4));
    rows.push(choice(c, live, H("eco", 4, "leaf", "ECO mode", "Less power at low volume."), "eco", false, 0));
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
        let [Op::Set(_, _), Op::Send { name: Some(name), method: "POST", path, body: Some(body) }] = &ops[..] else { panic!("{ops:?}") };
        assert_eq!(name, &format!("receiver.set:{}", body["setting"].as_str().unwrap()));
        assert_eq!(path, "/v1/receiver/set");
        body.clone()
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
        let mut mode = descriptor(json!("Dolby Surround"));
        mode["options"] = json!([{"value":"STEREO","label":"Stereo"},{"value":"DIRECT","label":"Direct"}]);
        put(&mut w, "soundMode", mode.clone());
        let r = rows(&w.ctx());
        assert!(r.iter().any(|r| r.text == "Sound mode · now Dolby Surround"));
        assert_eq!(row(&r, "modes").index, None);
        assert!(row(&r, "modes").enabled);
        assert_eq!(send(row(&r, "modes"), Arg::Index(1)), json!({"setting":"soundMode","value":"DIRECT","session":"session-1"}));
        mode["value"] = json!("unknown");
        put(&mut w, "soundMode", mode);
        let r = rows(&w.ctx());
        assert!(!row(&r, "modes").enabled);
        assert!(r.iter().any(|r| r.text == "Sound mode"));
    }

    #[test]
    fn named_response_and_readback_control_pending_and_errors() {
        let mut w = world();
        put(&mut w, "power", descriptor(json!(true)));
        w.local.set("receiver.write", json!({"setting":"power","session":"session-1","token":1,"at":NOW}));
        w.docs.set("receiver.set:power:1.status", json!(202));
        let ops = answered(&w.ctx(), "receiver.set:power:1", &json!({"accepted":true,"requestId":"r1","session":"session-1"}));
        let Op::Set(key, state) = &ops[0] else { panic!() };
        w.local.set(key, state.clone());
        let r = rows(&w.ctx());
        assert!(row(&r, "power").on && !row(&r, "power").enabled);
        assert_eq!(row(&r, "power").detail, "Applying…");
        let mut d = descriptor(json!(false));
        d["result"] = json!({"requestId":"r1","status":"applied","error":null});
        put(&mut w, "power", d.clone());
        let r = rows(&w.ctx());
        assert!(!row(&r, "power").on && row(&r, "power").enabled);
        d["result"] = json!({"requestId":"r1","status":"failed","error":"No readback"});
        put(&mut w, "power", d);
        assert_eq!(row(&rows(&w.ctx()), "power").detail, "No readback");
        w.docs.set("receiver.set:power:1.status", json!(409));
        let ops = answered(&w.ctx(), "receiver.set:power:1", &json!({"error":{"message":"Busy"}}));
        assert!(matches!(&ops[0],Op::Set(_, v) if v["error"] == "Busy"));
    }
    #[test]
    fn changed_session_discards_pending_and_old_response() {
        let mut w = world();
        put(&mut w, "power", descriptor(json!(true)));
        w.local.set("receiver.write", json!({"setting":"power","session":"session-1","token":2,"at":NOW}));
        let mut changed = w.topics.helper("receiver").unwrap().clone();
        changed["settingsSession"] = json!("session-2");
        w.topics.set_helper("receiver", changed);
        assert!(row(&rows(&w.ctx()), "power").enabled);
        assert!(answered(&w.ctx(), "receiver.set:power:1", &json!({"accepted":true,"session":"session-1","requestId":"old"})).is_empty());
        let mut no_session = w.topics.helper("receiver").unwrap().clone();
        no_session["settingsSession"] = Value::Null;
        w.topics.set_helper("receiver", no_session);
        assert!(!row(&rows(&w.ctx()), "power").enabled);
    }

    #[test]
    fn disconnect_shows_the_matching_failed_write() {
        let mut w = world();
        let mut d = descriptor(Value::Null);
        d["result"] = json!({"requestId":"r1","status":"failed","error":"Connection lost"});
        put(&mut w, "power", d);
        w.local.set("receiver.write", json!({"setting":"power","requestId":"r1","session":"session-1","at":NOW}));
        let mut disconnected = w.topics.helper("receiver").unwrap().clone();
        disconnected["reachable"] = json!(false);
        w.topics.set_helper("receiver", disconnected.clone());
        assert_eq!(rows(&w.ctx()).iter().filter(|r| r.text == "Change failed: Connection lost").count(), 4);
        w.local.set("receiver.write", json!({"setting":"power","error":"Busy","session":"session-1","at":NOW}));
        assert_eq!(rows(&w.ctx()).iter().filter(|r| r.text == "Change failed: Busy").count(), 4);
        w.local.set("receiver.write", json!({"setting":"power","requestId":"r1","session":"session-1","at":NOW}));
        disconnected["settingsSession"] = Value::Null;
        w.topics.set_helper("receiver", disconnected);
        let r = rows(&w.ctx());
        assert_eq!(r.iter().filter(|r| r.text == "Change failed: Connection lost").count(), 4);
        assert!(!row(&r, "power").enabled);
    }

    #[test]
    fn stale_and_old_backend_values_do_not_enable_controls() {
        let mut w = world();
        let mut d = descriptor(json!(true));
        d["updatedAt"] = json!(NOW - 7000);
        put(&mut w, "power", d);
        let r = rows(&w.ctx());
        assert!(!row(&r, "power").enabled && !row(&r, "power").on);
        let mut old = w.topics.helper("receiver").unwrap().clone();
        old.as_object_mut().unwrap().remove("settings");
        w.topics.set_helper("receiver", old);
        assert_eq!(controls(&w.ctx())[0].text, "Receiver settings unavailable");
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
