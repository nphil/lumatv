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

use super::plugin_rows::{Plugin, ask_all, field_rows, fields, text};
use super::util::go;
use crate::panels::hub;
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row};
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
    }
    ops
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
        // switched on or off: the plugin's page came or went
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
            rows.push(Row::head(title, 0));
        }
        section = Some(s);
        rows.extend(field_rows(c, PLUGIN, 0, f, icon(text(f, "key"))));
    }
    rows
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
        assert!(matches!(&opened(&c)[..], [Op::Set(..), Op::Send { path, .. }] if path.ends_with("/actions/check")), "opening looks at the TV");
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
        assert_eq!(heads, ["Addresses", "Channels", "RF 29", "RF 32"]);
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
}
