//! Beszel, a page of System: the helper's plugin that runs Beszel's agent so a Beszel hub can watch the TV
//! (`helper/app/src/beszel.rs`). It asks first, in the words of its entry in the `plugins` topic, as the guide data page does; off is
//! one press. While it is on, the page draws what the plugin names (`plugin_rows`): how the agent stands, and the hub's address, key
//! and token from the hub's Add System.

use super::plugin_rows::{Plugin, fact, field_rows, fields, text};
use crate::panels::hub;
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

const ID: &str = "beszel";
const PLUGIN: Plugin = Plugin { id: ID, sections: &["hub"] };
/// The name the switch's answer comes back under.
const SWITCH: &str = "beszel.switch";

pub fn page() -> Page {
    Page::new("beszel", "Beszel", "gauge")
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

/// The value of the row that opens the page: off, or how the agent stands in the plugin's words.
pub fn summary(c: &Ctx) -> String {
    match entry(c) {
        None => "Not available".into(),
        Some(_) if !on(c) => "Off".into(),
        Some(_) => fact(c, PLUGIN, "status").filter(|s| !s.is_empty()).unwrap_or_else(|| "On".into()),
    }
}

/// Connected is good news; connecting is news; anything else (no hub yet, a refusal, an agent that will not start) wants the owner.
fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    let words = summary(c);
    let tone = match () {
        _ if !on(c) || words == "On" || words.starts_with("Connecting") => "info",
        _ if words.starts_with("Connected") => "positive",
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
        _ if name.starts_with(&format!("secret:{ID}:")) => vec![toast(&if ok { "Kept on this TV".to_string() } else { format!("Not kept: {}", why()) })],
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
        "hub.url" => "server",
        "hub.key" | "hub.token" => "key-round",
        _ => "",
    }
}

fn rows(c: &Ctx) -> Vec<Row> {
    let Some(e) = entry(c) else {
        return vec![Row::empty("Not available", "The TV's helper is not answering, or does not have the Beszel plugin.", "gauge", 0)];
    };
    let (consent, code) = (text(e, "consent").to_string(), text(e, "approve").to_string());
    let shared = on(c);
    let mut rows = vec![Row::toggle("use").icon("gauge").label("Beszel").help(&consent).on(shared).detail(hub::failure(e).unwrap_or_default()).admin().act(move |a| match a {
        Arg::Toggle(true) => ask("gauge", "Let a Beszel hub watch this TV?", &consent, "Turn on", "Cancel", false, vec![switch(true, &code)]),
        Arg::Toggle(false) => vec![switch(false, "")],
        _ => Vec::new(),
    })];
    if !shared {
        return rows;
    }
    let Some(o) = c.owner(ID).filter(|o| o.known) else {
        rows.push(Row::note("The Beszel page comes once the plugin is running.", 0));
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
        let text = |key: &str, secret: bool| json!({"key": key, "label": key, "kind": {"type": "text", "max_chars": 200, "secret": secret}});
        json!({"sections": [{"id": "hub", "title": "Hub", "fields": [
            {"key": "status", "label": "Status", "kind": {"type": "read-only", "value": {"capability": format!("plugin.{ID}.status")}}},
            text("hub.url", false), text("hub.key", false), text("hub.token", true)]}]})
    }

    fn world(state: &str) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.helper_up = true;
        w.topics.set_helper(
            "plugins",
            json!({"plugins": [], "firstParty": [{"id": ID, "name": "Beszel", "state": state, "firstParty": true,
                   "consent": "Lets a Beszel hub watch this TV.", "approve": "c0de"}]}),
        );
        if state == "connected" {
            let mut o = crate::settings::src::Owner::new(ID);
            o.take(&json!({"ok": true, "version": 1, "schema": schema(), "effective": {"hub.url": {"value": "https://hub.test"}}}));
            w.owners.insert(ID.into(), o);
            w.topics.set_helper(
                &format!("plugin.{ID}.facts"),
                json!({"facts": {"status": "Connected to hub.test", "hub-token": "Kept on this TV"}}),
            );
        }
        w
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    #[test]
    fn off_it_is_one_switch_that_asks_in_the_plugins_words() {
        let w = world("off");
        let c = w.ctx();
        assert_eq!(chip(&c), Some(("Off".into(), "info")));
        let r = rows(&c);
        assert_eq!(r.len(), 1, "the switch alone");
        let [Op::Ask(question)] = &(row(&r, "use").act.as_ref().unwrap())(Arg::Toggle(true))[..] else { panic!("it asks first") };
        assert_eq!(question.body, "Lets a Beszel hub watch this TV.");
        assert!(
            matches!(&question.answers[1].then[..], [Op::Send { path, body: Some(b), .. }] if path.ends_with("/beszel/switch") && b == &json!({"on": true, "approve": "c0de"}))
        );
    }

    #[test]
    fn on_it_says_how_the_agent_stands_and_the_token_goes_to_the_plugin_alone() {
        let w = world("connected");
        let c = w.ctx();
        assert_eq!(chip(&c), Some(("Connected to hub.test".into(), "positive")));
        let r = rows(&c);
        assert_eq!(row(&r, "hub.url").value, "https://hub.test");
        assert_eq!(row(&r, "hub.token").value, "Kept on this TV");
        let [Op::Entry { then, secret: true, text, .. }] = &(row(&r, "hub.token").act.as_ref().unwrap())(Arg::Press)[..] else { panic!("typed") };
        assert!(text.is_empty(), "a secret is never shown back");
        assert!(
            matches!(&then("tk".into())[..], [Op::Send { path, body: Some(b), .. }] if path == "/v1/plugins/beszel/secret" && b == &json!({"field": "hub.token", "secret": "tk"}))
        );
    }
}
