//! Home Assistant gateway, a page of System: the helper's plugin that answers Home Assistant's LG webOS TV integration on port 3001 with
//! LumaTV's own gateway in place of LG's (`helper/app/src/ssgw.rs`). It asks first, in the words of its entry in the `plugins` topic, as
//! the Beszel page does; off is one press and gives the port back to LG's gateway. While it is on, the page draws what the plugin names
//! (`plugin_rows`): how the gateway stands, its last crash until it is marked read, Try again once the plugin has stopped trying, the
//! notify service told of crashes, Forget clients, and how LG's service host and extras stand.

use super::plugin_rows::{Plugin, fact, field_rows, fields, text};
use crate::panels::hub;
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

const ID: &str = "ssgw";
const PLUGIN: Plugin = Plugin { id: ID, sections: &["gateway"] };
/// The name the switch's answer comes back under.
const SWITCH: &str = "ssgw.switch";
const ICON: &str = "router";

pub fn page() -> Page {
    Page::new("ssgw", "Home Assistant gateway", ICON)
        .parent("system")
        .back("System")
        .reads(reads)
        .rows(rows)
        .summary(summary)
        .chip(chip)
        .answered(answered)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![Read::owner(ID)]
}

/// The plugin's entry in the `plugins` topic: none from a helper that is not there, or older than the plugin.
fn entry<'a>(c: &'a Ctx) -> Option<&'a Value> {
    hub::plugin_of(c.helper("plugins")?, ID)
}

fn on(c: &Ctx) -> bool {
    entry(c).is_some_and(|e| text(e, "state") != "off")
}

/// The value of the row that opens the page: off, or how the gateway stands in the plugin's words.
pub fn summary(c: &Ctx) -> String {
    match entry(c) {
        None => "Not available".into(),
        Some(_) if !on(c) => "Off".into(),
        Some(_) => fact(c, PLUGIN, "status").filter(|s| !s.is_empty()).unwrap_or_else(|| "On".into()),
    }
}

/// Serving is good news and starting is news; an unread crash, or a gateway that is not serving, wants the owner.
fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    let words = summary(c);
    let tone = match () {
        _ if !on(c) || words == "On" => "info",
        _ if fact(c, PLUGIN, "crash").is_some() => "warning",
        _ if words.starts_with("Serving") => "positive",
        _ if words.starts_with("Starting") || words.starts_with("Waiting") => "info",
        _ => "warning",
    };
    Some((words, tone))
}

fn answered(_: &Ctx, name: &str, data: &Value) -> Vec<Op> {
    let ok = data.get("ok").and_then(Value::as_bool) == Some(true);
    let why = || data.pointer("/error/message").and_then(Value::as_str).unwrap_or("the TV did not answer").to_string();
    match name {
        // switched on or off: the plugin's page came or went
        "topic:plugins" => vec![Op::Refresh],
        SWITCH if !ok => vec![toast(&format!("Not changed: {}", why()))],
        _ if !ok && name.starts_with(&format!("action:{ID}:")) => vec![toast(&format!("Not done: {}", why()))],
        _ => Vec::new(),
    }
}

fn switch(on: bool, code: &str) -> Op {
    let body = if on { json!({"on": true, "approve": code}) } else { json!({"on": false}) };
    Op::Send { name: Some(SWITCH.into()), method: "POST", path: format!("/v1/plugins/{ID}/switch"), body: Some(body) }
}

/// A glyph for the fields the plugin names.
fn icon(key: &str) -> &'static str {
    match key {
        "status" => "activity",
        "crash" | "crash-log" => "triangle-alert",
        "crash-where" => "file-text",
        "ack" => "check",
        "retry" => "rotate-ccw",
        "notify.service" | "notified" => "bell",
        "forget" => "key-round",
        "sysbus" | "lgextras" => "cpu",
        _ => "",
    }
}

fn rows(c: &Ctx) -> Vec<Row> {
    let Some(e) = entry(c) else {
        return vec![Row::empty("Not available", "The TV's helper is not answering, or does not have the gateway plugin.", ICON, 0)];
    };
    let (consent, code) = (text(e, "consent").to_string(), text(e, "approve").to_string());
    let serving = on(c);
    let mut rows = vec![Row::toggle("use").icon(ICON).label("LumaTV's gateway").help(&consent).on(serving).detail(hub::failure(e).unwrap_or_default()).admin().act(move |a| match a {
        Arg::Toggle(true) => ask(ICON, "Serve Home Assistant with LumaTV's gateway?", &consent, "Turn on", "Cancel", false, vec![switch(true, &code)]),
        Arg::Toggle(false) => vec![switch(false, "")],
        _ => Vec::new(),
    })];
    if !serving {
        return rows;
    }
    let Some(o) = c.owner(ID).filter(|o| o.known) else {
        rows.push(Row::note("The gateway's page comes once the plugin is running.", 0));
        return rows;
    };
    for (_, f) in fields(PLUGIN, &o.schema) {
        rows.extend(field_rows(c, PLUGIN, 0, f, icon(text(f, "key"))));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;

    fn schema() -> Value {
        let fact = |key: &str| json!({"key": key, "label": key, "kind": {"type": "read-only", "value": {"capability": format!("plugin.{ID}.{key}")}}});
        let with = |mut f: Value, key: &str| {
            f["visible_when"] = json!({"type": "capability-present", "capability": format!("plugin.{ID}.{key}")});
            f
        };
        let action = |key: &str| json!({"key": key, "label": key, "kind": {"type": "action", "action": key, "confirmation": "none", "verb": key}});
        json!({"sections": [{"id": "gateway", "title": "Gateway", "fields": [
            fact("status"), with(fact("crash"), "crash"), with(action("ack"), "crash"), with(action("retry"), "gaveup"),
            {"key": "notify.service", "label": "Tell crashes to", "kind": {"type": "text", "max_chars": 100, "secret": false}},
            action("forget")]}]})
    }

    fn world(state: &str, facts: Value) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.helper_up = true;
        w.topics.set_helper(
            "plugins",
            json!({"plugins": [], "firstParty": [{"id": ID, "name": "Home Assistant gateway", "state": state, "firstParty": true,
                   "consent": "Answers Home Assistant with LumaTV's gateway.", "approve": "c0de"}]}),
        );
        if state != "off" {
            let mut o = crate::settings::src::Owner::new(ID);
            o.take(&json!({"ok": true, "version": 1, "schema": schema(), "effective": {"notify.service": {"value": "notify.mobile_app_phone"}}}));
            w.owners.insert(ID.into(), o);
            w.topics.set_helper(&format!("plugin.{ID}.facts"), json!({"facts": facts}));
        }
        w
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    #[test]
    fn off_it_is_one_switch_that_asks_in_the_plugins_words() {
        let w = world("off", Value::Null);
        let c = w.ctx();
        assert_eq!(chip(&c), Some(("Off".into(), "info")));
        let r = rows(&c);
        assert_eq!(r.len(), 1, "the switch alone");
        let [Op::Ask(question)] = &(row(&r, "use").act.as_ref().unwrap())(Arg::Toggle(true))[..] else { panic!("it asks first") };
        assert_eq!(question.body, "Answers Home Assistant with LumaTV's gateway.");
        assert!(
            matches!(&question.answers[1].then[..], [Op::Send { path, body: Some(b), .. }] if path.ends_with("/ssgw/switch") && b == &json!({"on": true, "approve": "c0de"}))
        );
    }

    #[test]
    fn on_a_crash_shows_with_its_mark_as_read_only_until_it_is_read() {
        let w = world("running", json!({"status": "Serving Home Assistant: 1 session, up 3 h"}));
        let c = w.ctx();
        assert_eq!(chip(&c), Some(("Serving Home Assistant: 1 session, up 3 h".into(), "positive")));
        let r = rows(&c);
        assert!(!row(&r, "crash").visible && !row(&r, "ack").visible && !row(&r, "retry").visible);
        assert_eq!(row(&r, "notify.service").value, "notify.mobile_app_phone");
        assert!(matches!(&(row(&r, "forget").act.as_ref().unwrap())(Arg::Press)[..], [Op::Send { path, .. }] if path == "/v1/plugins/ssgw/actions/forget"));

        let w = world("running", json!({"status": "Restarting after a crash", "crash": "2026-11-13 04:22 UTC: panicked after 3 h"}));
        let c = w.ctx();
        assert_eq!(chip(&c).unwrap().1, "warning");
        let r = rows(&c);
        assert!(row(&r, "crash").visible && row(&r, "ack").visible && !row(&r, "retry").visible);
        assert_eq!(row(&r, "crash").value, "2026-11-13 04:22 UTC: panicked after 3 h");
    }
}
