//! Live TV Tuner, a page of Inputs & devices: the helper's plugin that shares the antenna tuner on the home network as an HDHomeRun, with
//! a channel playlist and the TV's guide (`helper/app/src/tuner.rs`). It asks first: its entry in the `plugins` topic says whether the owner
//! switched it on and in which words it asks, and the switch asks the owner in exactly those words before it turns it on (`POST
//! /v1/plugins/live-tv-tuner/switch` with their code); off is one press. While it is on, the page draws what the plugin says of itself
//! (`plugin_rows`): where the tuner stands (idle, streaming a channel, busy while the TV's own Live TV has it), the addresses a player is
//! given, the networks it answers and the name players show, then every channel under its RF channel with how it came in last and its
//! switch in the lineup, and the button that checks every channel. Opening the page asks the plugin to look at the TV (`check`), so a busy
//! tuner reads as busy. The channel list is asked for once the page's schema is in, again when the plugin's facts change (a check moves on
//! a frequency, a stream starts or ends) and after a channel's switch. The TV answers only while it is on or in Always Ready (LG's
//! `general.lifeOnScreenMode`, read here and never written): with Always Ready off, a TV switched off sleeps, and the page says so.
//!
//! The antenna meter, under the tuner's own rows, reads the demodulator about once a second while the page is open (`/v1/tuner/meter`):
//! of an RF channel picked, which the helper tunes when the tuner is free and lets go of when anything else wants it, or of the one the
//! tuner is on. Its bar is the signal to noise in the verdict's colour, with a tick at the best reading since the channel was picked, so
//! the antenna can be moved and compared; its line says the numbers. Leaving the page stops it, and so does the helper when the page
//! stops asking.

use super::plugin_rows::{Plugin, ask_all, field_rows, fields, text};
use super::util::go;
use crate::panels::hub;
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row, Tone};
use serde_json::{Value, json};

const ID: &str = "live-tv-tuner";
const PLUGIN: Plugin = Plugin { id: ID, sections: &["tuner", "addresses", "channels"] };
/// The page's own state: the channel list was asked for since the page opened.
const ASKED: &str = "asked";
/// The answers of the check button and of a channel's switch.
const CHECK_ALL: &str = "action:live-tv-tuner:reception";
const LINEUP: &str = "item:live-tv-tuner:channels";
/// The name the switch's answer comes back under.
const SWITCH: &str = "tuner.switch";
/// The meter's answers, and the page's own state: whether it reads, and the RF channel picked (none: the one the tuner is on).
const METER: &str = "tuner.meter";
const METER_ON: &str = "meter.on";
const METER_RF: &str = "meter.rf";
const METER_HELP: &str = "Move the antenna and watch the bar: longer is better, and the tick is the best so far. Good is a clean picture with room to spare, Fair a picture with little margin, Poor one that breaks up. MSE is the noise the TV measures (lower is better), packet errors are what it could not repair.";
/// LG's Always Ready: `off`, or how much of it is on.
const ALWAYS_READY: &str = "general.lifeOnScreenMode";
const WHEN: &str = "Players reach the tuner while the TV is on, or off in Always Ready. With Always Ready off, a TV switched off sleeps and nothing answers.";

pub fn page() -> Page {
    Page::new("tuner", "Live TV Tuner", "antenna")
        .parent("inputs")
        .back("Inputs & devices")
        .reads(reads)
        .rows(rows)
        .summary(summary)
        .chip(chip)
        .opened(opened)
        .closed(closed)
        .answered(answered)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![Read::owner(ID), Read::lg("general", &["lifeOnScreenMode"])]
}

/// The plugin's entry in the `plugins` topic: none from a helper that is not there, or older than the plugin.
fn entry<'a>(c: &'a Ctx) -> Option<&'a Value> {
    hub::plugin_of(c.helper("plugins")?, ID)
}

fn on(c: &Ctx) -> bool {
    entry(c).is_some_and(|e| text(e, "state") != "off")
}

/// The value of the row that opens the page: off, or where the tuner stands in the plugin's words.
pub fn summary(c: &Ctx) -> String {
    match entry(c) {
        None => "Not available".into(),
        Some(_) if !on(c) => "Off".into(),
        Some(_) => super::plugin_rows::fact(c, PLUGIN, "status").filter(|s| !s.is_empty()).unwrap_or_else(|| "On".into()),
    }
}

fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    let words = summary(c);
    let tone = match words.split([':', ' ']).next().unwrap_or("") {
        _ if !on(c) => "info",
        "Busy" | "Not" => "warning",
        _ => "positive",
    };
    Some((words, tone))
}

fn opened(c: &Ctx) -> Vec<Op> {
    let mut ops = vec![Op::Set(ASKED.into(), json!(false))];
    if on(c) {
        ops.push(Op::Send { name: None, method: "POST", path: format!("/v1/plugins/{ID}/actions/check"), body: Some(json!({})) });
        ops.push(Op::Set(METER_ON.into(), json!(true)));
        ops.push(aim(c.local.get(METER_RF).as_u64()));
    }
    ops
}

/// Leaving the page stops the meter: no reading is asked for again, and the helper lets go of the tuner now.
fn closed(_: &Ctx) -> Vec<Op> {
    vec![
        Op::Set(METER_ON.into(), json!(false)),
        Op::Later { key: "meter".into(), ms: 0, then: Vec::new() },
        Op::Send { name: None, method: "POST", path: "/v1/tuner/meter/stop".into(), body: None },
    ]
}

/// The meter on RF channel `rf`, or on the one the tuner is on.
fn aim(rf: Option<u64>) -> Op {
    Op::Send { name: Some(METER.into()), method: "POST", path: "/v1/tuner/meter".into(), body: Some(json!({"rf": rf})) }
}

/// The channel list asked for again after a moment of quiet: a check that moves on several frequencies asks once.
fn ask_again(c: &Ctx) -> Vec<Op> {
    let Some(o) = c.owner(ID).filter(|o| o.known) else { return Vec::new() };
    vec![Op::Later { key: "channels".into(), ms: 400, then: ask_all(PLUGIN, &o.schema) }]
}

fn answered(c: &Ctx, name: &str, data: &Value) -> Vec<Op> {
    let ok = data.get("ok").and_then(Value::as_bool) == Some(true);
    let why = || data.pointer("/error/message").and_then(Value::as_str).unwrap_or("the TV did not answer").to_string();
    match name {
        // a reading came: the next one in a second, while the page is open and the tuner shared
        METER if c.local.flag(METER_ON) && on(c) => {
            vec![Op::Later { key: "meter".into(), ms: 1000, then: vec![Op::Send { name: Some(METER.into()), method: "GET", path: "/v1/tuner/meter".into(), body: None }] }]
        }
        // switched on or off: the plugin's page came or went, and the meter starts with it
        "topic:plugins" if on(c) && !c.local.flag(METER_ON) => vec![Op::Refresh, Op::Set(METER_ON.into(), json!(true)), aim(c.local.get(METER_RF).as_u64())],
        "topic:plugins" => vec![Op::Refresh],
        SWITCH if !ok => vec![toast(&format!("Not changed: {}", why()))],
        "owner:live-tv-tuner" if on(c) && !c.local.flag(ASKED) => match c.owner(ID).filter(|o| o.known) {
            Some(o) => {
                let mut ops = vec![Op::Set(ASKED.into(), json!(true))];
                ops.extend(ask_all(PLUGIN, &o.schema));
                ops
            }
            None => Vec::new(),
        },
        "topic:plugin.live-tv-tuner.facts" => ask_again(c),
        LINEUP if ok => ask_again(c),
        LINEUP => vec![toast(&format!("Not changed: {}", why()))],
        CHECK_ALL if ok => vec![toast("Checking every channel")],
        CHECK_ALL => vec![toast(&format!("Not checked: {}", why()))],
        _ => Vec::new(),
    }
}

fn switch(on: bool, code: &str) -> Op {
    let body = if on { json!({"on": true, "approve": code}) } else { json!({"on": false}) };
    Op::Send { name: Some(SWITCH.into()), method: "POST", path: format!("/v1/plugins/{ID}/switch"), body: Some(body) }
}

/// A glyph for the facts the plugin names.
fn icon(key: &str) -> &'static str {
    match key {
        "status" => "activity",
        "name" => "tv",
        "networks" => "shield",
        "discover" => "server",
        "playlist" => "list-video",
        "guide" => "tv-minimal",
        "reception" => "activity",
        "channels" => "antenna",
        _ => "",
    }
}

fn rows(c: &Ctx) -> Vec<Row> {
    let Some(e) = entry(c) else {
        return vec![Row::empty("Not available", "The TV's helper is not answering, or does not have the tuner plugin.", "antenna", 0)];
    };
    let (consent, code) = (text(e, "consent").to_string(), text(e, "approve").to_string());
    let shared = on(c);
    let help = format!("{consent} {WHEN}");
    let mut rows = vec![Row::toggle("share").icon("antenna").label("Share the antenna tuner").help(&help).on(shared).admin().act(move |a| match a {
        Arg::Toggle(true) => ask("antenna", "Share the antenna tuner?", &consent, "Share", "Cancel", false, vec![switch(true, &code)]),
        Arg::Toggle(false) => vec![switch(false, "")],
        _ => Vec::new(),
    })];
    if c.lg.has(ALWAYS_READY) && c.lg.is(ALWAYS_READY, "off") {
        let mut r = Row::link("alwaysready").icon("moon-star").label("Always Ready is off").help(WHEN).does(go("look"));
        r.detail = "A TV switched off sleeps: players reach the tuner only while it is on".into();
        rows.push(r);
    }
    if !shared {
        return rows;
    }
    let Some(o) = c.owner(ID).filter(|o| o.known) else {
        rows.push(Row::note("The tuner's page comes once the plugin is running.", 0));
        return rows;
    };
    let mut section = None;
    for (s, f) in fields(PLUGIN, &o.schema) {
        if section.is_some_and(|was| was != s)
            && let Some(title) = o.schema["sections"].as_array().into_iter().flatten().find(|x| text(x, "id") == PLUGIN.sections[s]).map(|x| text(x, "title"))
        {
            if section == Some(0) {
                rows.extend(meter_rows(c));
            }
            rows.push(Row::head(title, 0));
        }
        section = Some(s);
        rows.extend(field_rows(c, PLUGIN, 0, f, icon(text(f, "key"))));
    }
    rows
}

/// The antenna meter: the channel it reads, its bar with the verdict and the best so far, and the best's numbers with a way to start
/// over.
fn meter_rows(c: &Ctx) -> Vec<Row> {
    let doc = c.docs.get(METER);
    let rfs: Vec<(u64, Vec<&str>)> = doc["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| Some((x["rf"].as_u64()?, x["guides"].as_array()?.iter().filter_map(Value::as_str).collect())))
        .collect();
    let picked = c.local.get(METER_RF).as_u64();
    let mut options = vec!["Current".to_owned()];
    options.extend(rfs.iter().map(|(rf, _)| format!("RF {rf}")));
    let index = picked.and_then(|p| rfs.iter().position(|(rf, _)| *rf == p)).map_or(0, |i| i + 1);
    let choices: Vec<u64> = rfs.iter().map(|(rf, _)| *rf).collect();
    let mut pick = Row::stepper("meter.rf", &options).icon("antenna").label("Channel to measure").index(Some(index)).act(move |a| match a {
        Arg::Index(i) => {
            let rf = i.checked_sub(1).and_then(|i| choices.get(i).copied());
            vec![Op::Set(METER_RF.into(), json!(rf)), aim(rf)]
        }
        _ => Vec::new(),
    });
    pick.detail = match rfs.iter().find(|(rf, _)| Some(*rf) == picked) {
        Some((_, guides)) => format!("Channels {}", guides.join(", ")),
        None => "The frequency the tuner is on now".into(),
    };
    let m = &doc["meter"];
    let s = &m["signal"];
    let (word, tone) = verdict(s);
    let mut meter = Row::slider("meter.signal", 0.0, 100.0, 1.0)
        .icon("gauge")
        .label(&match m["rf"].as_u64() {
            Some(rf) => format!("Signal on RF {rf}"),
            None => "Signal".into(),
        })
        .help(METER_HELP)
        .tone(tone)
        .mark(m["best"]["level"].as_f64().map(|l| l * 100.0))
        .num(s["level"].as_f64().unwrap_or(0.0) * 100.0)
        .value_text(if m.is_null() { "\u{2026}" } else { word });
    meter.info = true;
    let numbers = if s.is_null() { "The TV gives no reading".to_owned() } else { reading_words(s) };
    meter.detail = match (&m["note"], m.is_null()) {
        (_, true) => doc.pointer("/error/message").and_then(Value::as_str).map_or("Measuring\u{2026}".into(), |why| format!("Not measuring: {why}")),
        // why the reading is not of the channel picked: the label names the one it is of
        (Value::String(note), _) => format!("{note} · {numbers}"),
        _ => numbers,
    };
    let mut best = Row::action("meter.best", "Start over").icon("rotate-ccw").label("Best so far").does(vec![Op::Send {
        name: Some(METER.into()),
        method: "POST",
        path: "/v1/tuner/meter/best".into(),
        body: None,
    }]);
    best.detail = match &m["best"] {
        Value::Null => "Nothing yet".into(),
        b => format!("{}: {}", verdict(b).0, reading_words(b)),
    };
    vec![Row::head("Antenna signal", 0), pick, meter, best]
}

/// A reading's verdict in a word and its colour.
fn verdict(s: &Value) -> (&'static str, Tone) {
    match s["grade"].as_str() {
        Some("good") => ("Good", Tone::Positive),
        Some("fair") => ("Fair", Tone::Warning),
        Some("poor") => ("Poor", Tone::Danger),
        _ => ("No signal", Tone::Danger),
    }
}

/// A reading's numbers: its signal to noise and the demodulator's noise, the packets it could not repair and its bit errors.
fn reading_words(s: &Value) -> String {
    if s["locked"] != true {
        return "No lock: nothing comes in on this frequency".into();
    }
    let mut words = Vec::new();
    if let Some(snr) = s["snr"].as_f64() {
        words.push(format!("{snr:.1} dB"));
    }
    if let Some(mse) = s["mse"].as_u64() {
        words.push(format!("MSE {mse}"));
    }
    if let Some(e) = s["errors"].as_u64() {
        words.push(if e == 1 { "1 packet error".into() } else { format!("{e} packet errors") });
    }
    if let Some(ber) = s["ber"].as_u64() {
        words.push(format!("BER {ber}"));
    }
    words.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};
    use crate::settings::row::Kind;

    fn schema() -> Value {
        let fact =
            |key: &str, label: &str| json!({"key": key, "label": label, "kind": {"type": "read-only", "value": {"capability": format!("plugin.{ID}.{key}")}}});
        json!({"sections": [
            {"id": "tuner", "title": "Tuner", "fields": [fact("status", "Status"),
                {"key": "name", "label": "Name shown in players", "kind": {"type": "text", "max_chars": 40, "secret": false}}, fact("networks", "Answers")]},
            {"id": "addresses", "title": "Addresses", "fields": [fact("discover", "HDHomeRun"), fact("playlist", "Playlist"), fact("guide", "Guide")]},
            {"id": "channels", "title": "Channels", "fields": [
                {"key": "channels.note", "label": "How each channel came in.", "kind": {"type": "note"}},
                {"key": "reception", "label": "Check every channel", "kind": {"type": "action", "action": "reception", "confirmation": "none", "verb": "Check"}},
                {"key": "channels", "label": "Channels", "kind": {"type": "items", "provider": "channels"}}]}]})
    }

    fn world(state: &str, always_ready: &str) -> World {
        let mut w = World::new(lg_with(&[("general", json!({"settings": {"lifeOnScreenMode": always_ready}}))], &[]));
        w.topics.helper_up = true;
        w.topics.set_helper(
            "plugins",
            json!({"plugins": [], "firstParty": [{"id": ID, "name": "Live TV Tuner", "state": state, "firstParty": true,
                   "consent": "Shares this TV's antenna tuner on the local network.", "approve": "c0de"}]}),
        );
        if state == "connected" {
            let mut o = crate::settings::src::Owner::new(ID);
            o.take(&json!({"ok": true, "version": 1, "schema": schema(), "effective": {"name": {"value": "Den"}}}));
            w.owners.insert(ID.into(), o);
            w.topics.set_helper(
                &format!("plugin.{ID}.facts"),
                json!({"facts": {"status": "Streaming 2.4 WEST", "discover": "http://192.168.1.242:5004/discover.json",
                "playlist": "http://192.168.1.242:5004/lineup.m3u", "guide": "http://192.168.1.242:5004/xmltv.xml", "networks": "192.168.0.0/16"}}),
            );
        }
        w
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    #[test]
    fn off_it_is_one_switch_that_asks_in_the_plugins_words_before_it_shares_the_tuner() {
        let w = world("off", "allEnabled");
        let c = w.ctx();
        assert_eq!((summary(&c), chip(&c)), ("Off".into(), Some(("Off".into(), "info"))));
        let r = rows(&c);
        assert_eq!(r.len(), 1, "the switch alone");
        let share = row(&r, "share");
        assert!(!share.on && share.admin && share.help.starts_with("Shares this TV's antenna tuner on the local network."));
        let [Op::Ask(question)] = &(share.act.as_ref().unwrap())(Arg::Toggle(true))[..] else { panic!("it asks first") };
        assert_eq!(question.body, "Shares this TV's antenna tuner on the local network.");
        assert!(
            matches!(&question.answers[1].then[..], [Op::Send { path, body: Some(b), .. }] if path.ends_with("/live-tv-tuner/switch") && b == &json!({"on": true, "approve": "c0de"}))
        );
        assert!(!opened(&c).iter().any(|o| matches!(o, Op::Send { .. })), "nothing to check while it is off");
    }

    #[test]
    fn on_the_page_shows_where_the_tuner_stands_and_what_a_player_is_given_and_off_asks_nothing() {
        let w = world("connected", "allEnabled");
        let c = w.ctx();
        assert_eq!(chip(&c), Some(("Streaming 2.4 WEST".into(), "positive")));
        let r = rows(&c);
        assert!(row(&r, "share").on);
        assert_eq!(row(&r, "status").value, "Streaming 2.4 WEST");
        assert_eq!(row(&r, "playlist").value, "http://192.168.1.242:5004/lineup.m3u");
        assert_eq!(row(&r, "name").value, "Den");
        assert!(r.iter().any(|x| x.kind == Kind::Head && x.text == "Addresses"));
        assert!(matches!(&(row(&r, "share").act.as_ref().unwrap())(Arg::Toggle(false))[..], [Op::Send { body: Some(b), .. }] if b == &json!({"on": false})));
        assert!(matches!(&opened(&c)[..], [Op::Set(..), Op::Send { path, .. }, ..] if path.ends_with("/actions/check")), "opening looks at the TV");
        assert!(
            matches!(&answered(&c, SWITCH, &json!({"ok": false, "error": {"message": "the words changed"}}))[..], [Op::Act { payload, .. }] if payload["message"] == "Not changed: the words changed")
        );
    }

    #[test]
    fn with_always_ready_off_the_page_says_a_tv_switched_off_does_not_answer() {
        let r = rows(&world("connected", "off").ctx());
        assert!(row(&r, "alwaysready").detail.contains("only while it is on"));
        assert!(!rows(&world("connected", "allEnabled").ctx()).iter().any(|x| x.id == "alwaysready"));
    }

    #[test]
    fn the_channels_are_listed_under_their_rf_channels_with_how_they_came_in_and_a_switch_and_a_check_of_them_all() {
        let mut w = world("connected", "allEnabled");
        assert!(rows(&w.ctx()).iter().any(|x| x.text == "Loading\u{2026}"), "the list until it is in");
        let asked = answered(&w.ctx(), "owner:live-tv-tuner", &Value::Null);
        assert!(
            matches!(&asked[..], [Op::Set(k, v), Op::Send { name: Some(n), path, .. }] if k == ASKED && v == &json!(true) && n == "choices:live-tv-tuner:channels" && path.starts_with("/v1/plugins/live-tv-tuner/choices/channels"))
        );
        w.local.set(ASKED, json!(true));
        assert!(answered(&w.ctx(), "owner:live-tv-tuner", &Value::Null).is_empty(), "asked once a visit");
        let item = |id: &str, rf: u8, text: &str, on: bool| {
            json!({"id": id, "label": format!("{id} W{rf}"), "group": format!("RF {rf}"),
                   "item": {"tile": "base.icons/antenna", "status": {"text": text, "tone": "warn"}, "switch": {"on": on, "action": "lineup"}}})
        };
        w.docs.set(
            "choices:live-tv-tuner:channels",
            json!({"ok": true, "page": {"state": "ready", "more": false,
            "rows": [item("16.7", 29, "Weak · 18 dB · 32% lost", true), item("16.9", 29, "No signal", false), item("2.1", 32, "Good · 28 dB", true)]}}),
        );
        let r = rows(&w.ctx());
        let heads: Vec<&str> = r.iter().filter(|x| x.kind == Kind::Head).map(|x| x.text.as_str()).collect();
        assert_eq!(heads, ["Antenna signal", "Addresses", "Channels", "RF 29", "RF 32"]);
        let weak = row(&r, "channels:16.7");
        assert!(weak.kind == Kind::Toggle && weak.on && weak.detail == "Weak · 18 dB · 32% lost" && weak.label == "16.7 W29" && weak.icon == "antenna");
        assert!(!row(&r, "channels:16.9").on);
        assert!(
            matches!(&(weak.act.as_ref().unwrap())(Arg::Toggle(false))[..], [Op::Send { name: Some(n), path, body: Some(b), .. }] if n == LINEUP && path.ends_with("/actions/lineup") && b == &json!({"target": "16.7"}))
        );
        assert!(
            matches!(&answered(&w.ctx(), LINEUP, &json!({"ok": true}))[..], [Op::Later { then, .. }] if matches!(&then[..], [Op::Send { .. }])),
            "the list again after a switch"
        );
        assert!(matches!(&answered(&w.ctx(), "topic:plugin.live-tv-tuner.facts", &Value::Null)[..], [Op::Later { .. }]), "and as a check moves on");
        let check = row(&r, "reception");
        assert!(check.kind == Kind::Action && check.button == "Check");
        assert!(
            matches!(&(check.act.as_ref().unwrap())(Arg::Press)[..], [Op::Send { name: Some(n), path, .. }] if n == CHECK_ALL && path.ends_with("/actions/reception"))
        );
        assert!(r.iter().any(|x| x.kind == Kind::Note && x.text == "How each channel came in."));
    }

    #[test]
    fn the_meter_reads_once_a_second_while_the_page_is_open_and_draws_the_verdict_and_the_best_so_far() {
        let mut w = world("connected", "allEnabled");
        let start = opened(&w.ctx());
        assert!(start.iter().any(|o| matches!(o, Op::Set(k, v) if k == METER_ON && v == &json!(true))));
        assert!(
            start.iter().any(|o| matches!(o, Op::Send { name: Some(n), method: "POST", path, body: Some(b) } if n == METER && path == "/v1/tuner/meter" && b == &json!({"rf": null}))),
            "on the channel the tuner is on until one is picked"
        );
        w.local.set(METER_ON, json!(true));
        let signal = |grade: &str, level: f64| json!({"locked": true, "mse": 355, "snr": 17.8, "errors": 81, "ber": 584211, "grade": grade, "level": level});
        w.docs.set(
            METER,
            json!({"ok": true, "channels": [{"rf": 29, "guides": ["16.1", "16.3"]}, {"rf": 32, "guides": ["2.1"]}],
                   "meter": {"aim": 29, "rf": 29, "tuned": true, "note": null, "signal": signal("poor", 0.13), "best": signal("fair", 0.4)}}),
        );
        assert!(
            matches!(&answered(&w.ctx(), METER, &Value::Null)[..], [Op::Later { ms: 1000, then, .. }] if matches!(&then[..], [Op::Send { method: "GET", .. }])),
            "the next reading in a second"
        );
        w.local.set(METER_RF, json!(29));
        let r = rows(&w.ctx());
        let pick = row(&r, "meter.rf");
        assert_eq!((pick.options.clone(), pick.index, pick.detail.as_str()), (vec!["Current".into(), "RF 29".into(), "RF 32".into()], Some(1), "Channels 16.1, 16.3"));
        assert!(matches!(&(pick.act.as_ref().unwrap())(Arg::Index(2))[..], [Op::Set(_, v), Op::Send { body: Some(b), .. }] if v == &json!(32) && b == &json!({"rf": 32})));
        let meter = row(&r, "meter.signal");
        assert_eq!((meter.label.as_str(), meter.value_text.as_deref(), meter.tone), ("Signal on RF 29", Some("Poor"), Tone::Danger));
        assert_eq!((meter.num.round(), meter.mark.map(f64::round)), (13.0, Some(40.0)), "the bar and the best so far");
        assert_eq!(meter.detail, "17.8 dB · MSE 355 · 81 packet errors · BER 584211");
        assert_eq!(row(&r, "meter.best").detail, "Fair: 17.8 dB · MSE 355 · 81 packet errors · BER 584211");
        // leaving the page stops it: no reading asked again, and the tuner let go of
        let stop = closed(&w.ctx());
        assert!(stop.iter().any(|o| matches!(o, Op::Later { key, then, .. } if key == "meter" && then.is_empty())));
        assert!(stop.iter().any(|o| matches!(o, Op::Send { path, .. } if path == "/v1/tuner/meter/stop")));
        w.local.set(METER_ON, json!(false));
        assert!(answered(&w.ctx(), METER, &Value::Null).is_empty(), "an answer after the page closed asks nothing more");
    }
}
