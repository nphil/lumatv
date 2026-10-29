//! Receiver (`settings/PageReceiver.qml`, mocks `24-settings-receiver`): the Denon's own controls, one page below Sound, drawn only from what
//! the helper's `receiver` topic says: the receiver's own units and names, values read back after every change, and any value older than
//! 6 s without an answer is hidden, not shown stale (`host/receiver.rs` is the same gate for the quick panel). The page holds no state of
//! the receiver's: a row reads the topic and sends `POST /v1/receiver/set {what, value}`; the next reading is what the row then shows.
//!
//! The helper's receiver module says `controls: false` until it can send the Denon's commands: then the page shows what it reads and dims
//! every control, with the reason in the first row. When it says `controls: true` it also gives these fields, which the rows read:
//! `model, soundMode, soundModeId, modes [{id, title, icon}], inputs [name], input, quick [{n, label}], quickSelect, audyssey {multEq,
//! dynEq, refOffset, dynVol}, levels {dialog, subwoofer, bass, treble}, cinemaEq, eco, zone2 {name, power, volumeDb, volumeText, input}`.
//!
//! The helper moves a reading's `updatedAt` quietly (a poll that changes nothing raises no event), so while the page is up it asks for the
//! topic's newest copy (`GET /v1/state/receiver`) every two seconds, as the gate does when a copy ages out.

use crate::settings::ctx::Ctx;
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Card, Op, Row};
use serde_json::{Value, json};

pub const STALE_MS: f64 = 6000.0;
const ASK_MS: u64 = 2000;

pub fn page() -> Page {
    Page::new("receiver", "Receiver", "boom-box")
        .parent("sound")
        .back("Sound")
        .tabs(&["Sound mode", "Levels", "Audyssey", "Zones & power"])
        .reads(reads)
        .rows(rows)
        .chip(chip)
        .subtitle(subtitle)
        .answered(answered)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![Read::Get { name: "receiver", path: "/v1/state/receiver" }]
}

fn answered(_: &Ctx, name: &str, _: &Value) -> Vec<Op> {
    if name == "receiver" { vec![Op::RefreshIn(ASK_MS)] } else { Vec::new() }
}

fn stamp(v: &Value) -> f64 {
    v.get("updatedAt").and_then(Value::as_f64).unwrap_or(0.0)
}

/// The newest copy the page has, in date or not: the topic, or the helper's answer to the last ask, whichever carries the later stamp.
fn newest<'a>(c: &'a Ctx<'_>) -> Option<&'a Value> {
    let topic = c.helper("receiver");
    let answer = c.docs.get("receiver").get("value").filter(|v| !v.is_null());
    match (topic, answer) {
        (Some(t), Some(a)) => Some(if stamp(a) > stamp(t) { a } else { t }),
        (t, a) => t.or(a),
    }
}

/// The reading while it may be shown (`ReceiverGate.doc`): the helper is up, the copy says `reachable` and `fresh`, and its stamp is no
/// older than six seconds.
pub fn doc<'a>(c: &'a Ctx<'_>) -> Option<&'a Value> {
    let r = newest(c)?;
    let age = c.now as f64 - stamp(r);
    let ok = c.topics.helper_up
        && r.get("reachable").and_then(Value::as_bool) == Some(true)
        && r.get("fresh").and_then(Value::as_bool) == Some(true)
        && stamp(r) > 0.0
        && age < STALE_MS
        && age > -1000.0;
    ok.then_some(r)
}

fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    Some(if doc(c).is_some() { ("Answering".into(), "positive") } else { ("Not answering".into(), "danger") })
}

fn subtitle(c: &Ctx) -> String {
    doc(c).and_then(|r| r.get("model")).and_then(Value::as_str).filter(|m| !m.is_empty()).unwrap_or("Denon receiver").to_string()
}

/// A level in dB the way the receiver's display writes it: a sign and one decimal where it moves by halves.
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

/// JavaScript's `String(n)`: 3, not 3.0.
fn plain(v: f64) -> String {
    if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") }
}

fn set(what: &'static str, value: Value) -> Vec<Op> {
    vec![Op::Send { name: None, method: "POST", path: "/v1/receiver/set".into(), body: Some(json!({"what": what, "value": value})) }]
}

fn words(r: &Value, key: &str) -> Vec<String> {
    r.get(key).and_then(Value::as_array).map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// A level slider: its value from `from`, dimmed while the receiver does not give it.
fn level(
    id: &str,
    tab: usize,
    icon: &str,
    label: &str,
    help: &str,
    (min, max, step): (f64, f64, f64),
    ctl: bool,
    v: Option<f64>,
    what: &'static str,
    half: bool,
) -> Row {
    let mut r = Row::slider(id, min, max, step).tab(tab).icon(icon).label(label).help(help).admin();
    r.enabled = ctl && v.is_some();
    r.num = v.unwrap_or(0.0);
    r.value_text = Some(db(v, half));
    r.act(move |a| match a {
        Arg::Value(x) => set(what, json!(x)),
        _ => Vec::new(),
    })
}

/// A switch of the receiver's: on from a boolean the receiver gave.
fn switch(id: &str, tab: usize, icon: &str, label: &str, help: &str, ctl: bool, v: Option<bool>, what: &'static str) -> Row {
    let mut r = Row::toggle(id).tab(tab).icon(icon).label(label).help(help).on(v == Some(true)).admin();
    r.enabled = ctl && v.is_some();
    r.act(move |a| match a {
        Arg::Toggle(x) => set(what, json!(x)),
        _ => Vec::new(),
    })
}

/// A few options on one row, written as the value at the same place of `values`.
fn pick(
    id: &str,
    tab: usize,
    icon: &str,
    label: &str,
    help: &str,
    ctl: bool,
    options: &[&str],
    values: Vec<Value>,
    now: Option<&Value>,
    what: &'static str,
) -> Row {
    let index = now.and_then(|n| values.iter().position(|v| v == n));
    let mut r = Row::seg(id, options).tab(tab).icon(icon).label(label).help(help).index(index).admin();
    r.enabled = ctl && now.is_some();
    r.act(move |a| match a {
        Arg::Index(i) => values.get(i).map(|v| set(what, v.clone())).unwrap_or_default(),
        _ => Vec::new(),
    })
}

fn rows(c: &Ctx) -> Vec<Row> {
    let live = doc(c);
    let empty = json!({});
    let rc = live.unwrap_or(&empty);
    let answers = live.is_some();
    let ctl = answers && rc.get("controls").and_then(Value::as_bool) == Some(true);
    let why = if !answers {
        "Receiver not answering"
    } else if ctl {
        ""
    } else {
        "Read only for now"
    };
    let aud = rc.get("audyssey").unwrap_or(&empty);
    let lev = rc.get("levels").unwrap_or(&empty);
    let z2 = rc.get("zone2").unwrap_or(&empty);
    let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64);
    let absolute = |v: &Value| v.get("volumeDisplay").and_then(Value::as_str) == Some("absolute");
    let mut rows = Vec::new();

    // ---- Sound mode
    rows.push(Row::note(why, 0).visible(!why.is_empty()));
    let mut power = Row::toggle("power")
        .tab(0)
        .icon("power")
        .label("Receiver power")
        .help("Turns on with the TV over HDMI-CEC. Off here keeps the TV on.")
        .on(rc.get("power").and_then(Value::as_bool) != Some(false))
        .admin();
    power.enabled = ctl;
    rows.push(power.act(|a| match a {
        Arg::Toggle(v) => set("power", json!(v)),
        _ => Vec::new(),
    }));
    let inputs = words(rc, "inputs");
    let at = |list: &[String], v: Option<&str>| v.and_then(|v| list.iter().position(|x| x == v));
    let mut input = Row::stepper("input", &inputs)
        .tab(0)
        .icon("cable")
        .label("Input")
        .help("What the receiver plays. TV Audio is this TV over eARC.")
        .index(at(&inputs, rc.get("input").and_then(Value::as_str)))
        .admin();
    input.enabled = ctl && !inputs.is_empty();
    let list = inputs.clone();
    rows.push(input.act(move |a| match a {
        Arg::Index(i) => list.get(i).map(|v| set("input", json!(v))).unwrap_or_default(),
        _ => Vec::new(),
    }));
    rows.push(Row::head(
        &format!("Sound mode{}", rc.get("soundMode").and_then(Value::as_str).filter(|s| !s.is_empty()).map(|s| format!(" \u{b7} now {s}")).unwrap_or_default()),
        0,
    ));
    let modes: Vec<(String, String, String)> = rc
        .get("modes")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .map(|m| {
                    let text = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    (text("id"), text("title"), if text("icon").is_empty() { "audio-waveform".to_string() } else { text("icon") })
                })
                .collect()
        })
        .unwrap_or_default();
    let current = rc.get("soundModeId").and_then(Value::as_str);
    let mut cards = Row::cards("modes", modes.iter().map(|(_, t, i)| Card { title: t.clone(), icon: i.clone(), sub: String::new(), face: None }).collect())
        .tab(0)
        .index(modes.iter().position(|(id, _, _)| Some(id.as_str()) == current))
        .admin();
    cards.cols = 4;
    cards.enabled = ctl && !modes.is_empty();
    rows.push(cards.act(move |a| match a {
        Arg::Index(i) => modes.get(i).map(|(id, _, _)| set("soundMode", json!(id))).unwrap_or_default(),
        _ => Vec::new(),
    }));
    let quick: Vec<(i64, String)> = rc
        .get("quick")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter().map(|q| (q.get("n").and_then(Value::as_i64).unwrap_or(0), q.get("label").and_then(Value::as_str).unwrap_or("").to_string())).collect()
        })
        .unwrap_or_default();
    let labels: Vec<String> = quick.iter().map(|(n, l)| format!("{n} \u{b7} {l}")).collect();
    let labels_ref: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut qs = Row::seg("quick", &labels_ref)
        .tab(0)
        .icon("zap")
        .label("Quick Select")
        .help("Four memories: input, volume and mode in one press.")
        .index(rc.get("quickSelect").and_then(Value::as_i64).map(|n| (n - 1).max(0) as usize))
        .admin();
    qs.enabled = ctl && !quick.is_empty();
    rows.push(qs.act(|a| match a {
        Arg::Index(i) => set("quickSelect", json!(i + 1)),
        _ => Vec::new(),
    }));

    // ---- Levels
    let limits = |v: &Value| if absolute(v) { (0.0, 98.0, 0.5) } else { (-80.0, 18.0, 0.5) };
    let mut volume = Row::slider("volume", limits(rc).0, limits(rc).1, 0.5).tab(1).icon("volume-2").label("Volume");
    volume.info = true;
    volume.num = if answers { if absolute(rc) { num(rc, "volumeAbs") } else { num(rc, "volumeDb") }.unwrap_or(0.0) } else { 0.0 };
    volume.value_text = Some(if answers {
        rc.get("volumeText").map(crate::host::sio::js::string).filter(|t| !t.is_empty()).unwrap_or_else(|| "\u{2013}".into())
    } else {
        "\u{2013}".into()
    });
    rows.push(volume);
    rows.push(level(
        "dialog",
        1,
        "mic",
        "Dialog level",
        "Louder voices without louder everything.",
        (-12.0, 12.0, 0.5),
        ctl,
        num(lev, "dialog"),
        "dialog",
        true,
    ));
    rows.push(level(
        "subwoofer",
        1,
        "speaker",
        "Subwoofer level",
        "More or less bass from the sub only.",
        (-12.0, 12.0, 0.5),
        ctl,
        num(lev, "subwoofer"),
        "subwoofer",
        true,
    ));
    rows.push(level(
        "bass",
        1,
        "sliders-vertical",
        "Bass",
        "Tone control. Off in Direct and Pure Direct.",
        (-6.0, 6.0, 1.0),
        ctl,
        num(lev, "bass"),
        "bass",
        false,
    ));
    rows.push(level(
        "treble",
        1,
        "sliders-vertical",
        "Treble",
        "Tone control. Off in Direct and Pure Direct.",
        (-6.0, 6.0, 1.0),
        ctl,
        num(lev, "treble"),
        "treble",
        false,
    ));
    rows.push(switch(
        "cinemaeq",
        1,
        "theater",
        "Cinema EQ",
        "Softens bright film soundtracks mixed for a big room.",
        ctl,
        rc.get("cinemaEq").map(|v| v.as_bool().unwrap_or(!v.is_null() && v != &json!(0))),
        "cinemaEq",
    ));

    // ---- Audyssey
    let text = |v: &Value, k: &str| v.get(k).cloned();
    rows.push(pick(
        "multeq",
        2,
        "audio-lines",
        "MultEQ",
        "Audyssey is the room correction the receiver measured. These switch its parts on and off; re-measuring is done on the receiver.",
        ctl,
        &["Reference", "L/R bypass", "Flat", "Off"],
        vec![json!("reference"), json!("bypass"), json!("flat"), json!("off")],
        text(aud, "multEq").as_ref(),
        "multEq",
    ));
    rows.push(switch(
        "dyneq",
        2,
        "activity",
        "Dynamic EQ",
        "Keeps bass and surround full at low volume.",
        ctl,
        aud.get("dynEq").map(|v| v.as_bool().unwrap_or(!v.is_null() && v != &json!(0))),
        "dynEq",
    ));
    rows.push(pick(
        "refoffset",
        2,
        "gauge",
        "Reference level offset",
        "For Dynamic EQ: 0 for films, 5\u{2013}15 for TV and music mixed quieter.",
        ctl,
        &["0 dB", "5", "10", "15"],
        vec![json!(0), json!(5), json!(10), json!(15)],
        text(aud, "refOffset").as_ref(),
        "refOffset",
    ));
    rows.push(pick(
        "dynvol",
        2,
        "volume-1",
        "Dynamic Volume",
        "Evens out loud adverts and quiet dialogue.",
        ctl,
        &["Off", "Light", "Medium", "Heavy"],
        vec![json!("off"), json!("light"), json!("medium"), json!("heavy")],
        text(aud, "dynVol").as_ref(),
        "dynVol",
    ));

    // ---- Zones & power
    rows.push(Row::head(
        &format!("Zone 2{}", z2.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).map(|s| format!(" \u{b7} {s}")).unwrap_or_default()),
        3,
    ));
    let z2_on = z2.get("power").map(|v| v.as_bool().unwrap_or(!v.is_null() && v != &json!(0)));
    rows.push(switch("zone2power", 3, "power", "Zone 2 power", "The second pair of speakers, on their own volume and input.", ctl, z2_on, "zone2Power"));
    let (lo, hi, step) = limits(z2);
    let mut zv = level("zone2volume", 3, "volume-2", "Zone 2 volume", "", (lo, hi, step), ctl && z2_on == Some(true), num(z2, "volumeDb"), "zone2Volume", true);
    zv.help.clear();
    zv.detail = if z2_on == Some(true) { String::new() } else { "Zone 2 is off".into() };
    zv.value_text = Some(z2.get("volumeText").map(crate::host::sio::js::string).filter(|t| !t.is_empty()).unwrap_or_else(|| "\u{2013}".into()));
    rows.push(zv);
    let mut z2in = Row::stepper("zone2input", &inputs)
        .tab(3)
        .icon("cable")
        .label("Zone 2 input")
        .help("Follows the main zone unless set here.")
        .index(at(&inputs, z2.get("input").and_then(Value::as_str)))
        .admin();
    z2in.enabled = ctl && z2_on == Some(true) && !inputs.is_empty();
    rows.push(z2in.act(move |a| match a {
        Arg::Index(i) => inputs.get(i).map(|v| set("zone2Input", json!(v))).unwrap_or_default(),
        _ => Vec::new(),
    }));
    rows.push(Row::head("Power", 3));
    rows.push(pick(
        "eco",
        3,
        "leaf",
        "ECO mode",
        "Less power at low volume. Auto decides by level; On always.",
        ctl,
        &["On", "Auto", "Off"],
        vec![json!("on"), json!("auto"), json!("off")],
        text(rc, "eco").as_ref(),
        "eco",
    ));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id} in {:?}", rows.iter().map(|r| &r.id).collect::<Vec<_>>()))
    }

    const NOW: u64 = 1_700_000_100_000;

    fn reading(controls: bool) -> Value {
        json!({"enabled": true, "reachable": true, "fresh": true, "updatedAt": NOW - 1000, "controls": controls, "model": "Denon AVR-S760H", "power": true,
               "input": "TV Audio", "inputs": ["TV Audio", "Game"], "soundMode": "Dolby Surround", "soundModeId": "dolby",
               "modes": [{"id": "movie", "title": "Movie", "icon": "film"}, {"id": "dolby", "title": "Dolby Surround"}],
               "quick": [{"n": 1, "label": "TV"}, {"n": 2, "label": "Game"}], "quickSelect": 2,
               "volumeDisplay": "db", "volumeDb": -34.5, "volumeText": "-34.5 dB",
               "levels": {"dialog": 1.5, "subwoofer": -3, "bass": 2, "treble": 0}, "cinemaEq": true,
               "audyssey": {"multEq": "flat", "dynEq": false, "refOffset": 5, "dynVol": "light"},
               "eco": "auto", "zone2": {"name": "Patio", "power": false, "volumeDb": -40, "volumeText": "-40 dB", "input": "Game"}})
    }

    fn world(controls: bool) -> World {
        let mut w = World::new(lg_with(&[], &[]));
        w.now = NOW;
        w.topics.helper_up = true;
        w.topics.set_helper("receiver", reading(controls));
        w
    }

    #[test]
    fn a_reading_older_than_six_seconds_or_not_fresh_shows_nothing() {
        let mut w = world(true);
        assert_eq!(chip(&w.ctx()), Some(("Answering".into(), "positive")));
        assert_eq!(subtitle(&w.ctx()), "Denon AVR-S760H");
        w.now = NOW + 5100;
        assert!(doc(&w.ctx()).is_none(), "{} ms old", 6100);
        assert_eq!(chip(&w.ctx()), Some(("Not answering".into(), "danger")));
        assert_eq!(subtitle(&w.ctx()), "Denon receiver");
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "note:Receiver not answering").text, "Receiver not answering");
        assert!(r.iter().filter(|x| x.kind != crate::settings::row::Kind::Note && x.kind != crate::settings::row::Kind::Head).all(|x| !x.enabled || x.info));
        w.now = NOW;
        w.topics.helper_up = false;
        assert!(doc(&w.ctx()).is_none(), "the helper is down");
    }

    #[test]
    fn the_helpers_newer_copy_in_answer_to_an_ask_brings_the_reading_back() {
        let mut w = world(true);
        w.now = NOW + 9000;
        assert!(doc(&w.ctx()).is_none());
        let mut again = reading(true);
        again["updatedAt"] = json!(NOW + 8000);
        w.docs.set("receiver", json!({"ok": true, "value": again}));
        assert!(doc(&w.ctx()).is_some());
        let ops = answered(&w.ctx(), "receiver", w.docs.get("receiver"));
        assert!(matches!(&ops[..], [Op::RefreshIn(2000)]), "it keeps asking while the page is up");
    }

    #[test]
    fn read_only_dims_every_control_and_says_why_in_the_first_row() {
        let w = world(false);
        let r = rows(&w.ctx());
        assert_eq!(r[0].text, "Read only for now");
        for id in ["power", "input", "modes", "quick", "dialog", "multeq", "zone2power", "eco"] {
            assert!(!by_id(&r, id).enabled, "{id}");
        }
        assert_eq!(by_id(&r, "volume").value_text.as_deref(), Some("-34.5 dB"), "what it reads is still shown");
        assert_eq!(by_id(&r, "input").index, Some(0));
        assert_eq!(by_id(&r, "modes").index, Some(1));
    }

    #[test]
    fn a_row_sends_what_and_value_in_the_receivers_own_units() {
        let w = world(true);
        let r = rows(&w.ctx());
        assert!(!r[0].visible, "nothing to explain while it answers and has controls");
        let sent = |row: &str, a: Arg| -> (String, Value) {
            let ops = (by_id(&r, row).act.as_ref().unwrap())(a);
            let [Op::Send { method: "POST", path, body: Some(b), .. }] = &ops[..] else { panic!("{ops:?}") };
            assert_eq!(path, "/v1/receiver/set");
            (b["what"].as_str().unwrap().to_string(), b["value"].clone())
        };
        assert_eq!(sent("power", Arg::Toggle(false)), ("power".into(), json!(false)));
        assert_eq!(sent("input", Arg::Index(1)), ("input".into(), json!("Game")));
        assert_eq!(sent("modes", Arg::Index(0)), ("soundMode".into(), json!("movie")));
        assert_eq!(sent("quick", Arg::Index(0)), ("quickSelect".into(), json!(1)));
        assert_eq!(sent("dialog", Arg::Value(2.5)), ("dialog".into(), json!(2.5)));
        assert_eq!(sent("multeq", Arg::Index(3)), ("multEq".into(), json!("off")));
        assert_eq!(sent("refoffset", Arg::Index(3)), ("refOffset".into(), json!(15)));
        assert_eq!(sent("eco", Arg::Index(0)), ("eco".into(), json!("on")));
        assert_eq!(by_id(&r, "multeq").index, Some(2));
        assert_eq!(by_id(&r, "refoffset").index, Some(1));
        assert_eq!(by_id(&r, "eco").index, Some(1));
        assert_eq!(by_id(&r, "quick").options, vec!["1 \u{b7} TV", "2 \u{b7} Game"]);
        assert_eq!(by_id(&r, "quick").index, Some(1));
        assert_eq!(r.iter().find(|x| x.id == "head:Sound mode \u{b7} now Dolby Surround").map(|x| x.tab), Some(0));
    }

    #[test]
    fn levels_read_in_decibels_with_a_sign_and_halves_only_where_they_move_by_halves() {
        assert_eq!(db(Some(1.5), true), "+1.5 dB");
        assert_eq!(db(Some(-3.0), true), "\u{2212}3.0 dB");
        assert_eq!(db(Some(2.0), false), "+2 dB");
        assert_eq!(db(Some(0.0), false), "0 dB");
        assert_eq!(db(None, true), "\u{2013}");
        let w = world(true);
        let r = rows(&w.ctx());
        assert_eq!(by_id(&r, "subwoofer").value_text.as_deref(), Some("\u{2212}3.0 dB"));
        assert_eq!(by_id(&r, "bass").value_text.as_deref(), Some("+2 dB"));
        assert_eq!((by_id(&r, "volume").min, by_id(&r, "volume").max), (-80.0, 18.0));
        assert!(by_id(&r, "volume").info);
    }

    #[test]
    fn zone_two_waits_for_its_power_and_a_missing_field_dims_its_row() {
        let w = world(true);
        let r = rows(&w.ctx());
        assert!(by_id(&r, "zone2power").enabled && !by_id(&r, "zone2power").on);
        assert!(!by_id(&r, "zone2volume").enabled && by_id(&r, "zone2volume").detail == "Zone 2 is off");
        assert!(!by_id(&r, "zone2input").enabled);
        assert_eq!(by_id(&r, "zone2volume").value_text.as_deref(), Some("-40 dB"));
        let mut w = world(true);
        let mut doc = reading(true);
        doc["zone2"]["power"] = json!(true);
        doc["levels"].as_object_mut().unwrap().remove("bass");
        doc["volumeDisplay"] = json!("absolute");
        w.topics.set_helper("receiver", doc);
        let r = rows(&w.ctx());
        assert!(by_id(&r, "zone2volume").enabled && by_id(&r, "zone2input").enabled);
        assert!(!by_id(&r, "bass").enabled && by_id(&r, "bass").value_text.as_deref() == Some("\u{2013}"));
        assert_eq!((by_id(&r, "volume").min, by_id(&r, "volume").max), (0.0, 98.0));
    }
}
