//! Guide data, a page of Inputs & devices: the helper's plugin that fills the Live TV guide and the channel banner with online listings
//! for a zip code, laid over the broadcast's own (`helper/app/src/guidedata.rs`). It asks first, in the words of its entry in the `plugins`
//! topic, as the tuner's page does; off is one press. While it is on, the page draws what the plugin names (`plugin_rows`): the zip code,
//! the sources in the order they are trusted, Plex's server and token, the Schedules Direct account, and how each source stands. Refresh
//! fetches the listings again now (`POST /v1/plugins/guide-data/actions/refresh`).

use super::plugin_rows::{Plugin, fact, field_rows, fields, text};
use crate::panels::hub;
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

const ID: &str = "guide-data";
const PLUGIN: Plugin = Plugin { id: ID, sections: &["listings", "plex", "schedules-direct", "gracenote"] };
/// The names the answers come back under.
const SWITCH: &str = "guide.switch";
const REFRESH: &str = "guide.refresh";

pub fn page() -> Page {
    Page::new("guidedata", "Guide data", "tv-minimal")
        .parent("inputs")
        .back("Inputs & devices")
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

/// The value of the row that opens the page: off, or how the listings stand in the plugin's words.
pub fn summary(c: &Ctx) -> String {
    match entry(c) {
        None => "Not available".into(),
        Some(_) if !on(c) => "Off".into(),
        Some(_) => fact(c, PLUGIN, "status").filter(|s| !s.is_empty()).unwrap_or_else(|| "On".into()),
    }
}

/// Listings in hand are good news; fetching is news; anything else (no zip code, no source, a source's problem) wants the owner.
fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    let words = summary(c);
    let tone = match words.chars().next() {
        _ if !on(c) || words == "On" || words.starts_with("Fetching") => "info",
        Some(d) if d.is_ascii_digit() => "positive",
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
        REFRESH if ok => vec![toast("Fetching the listings")],
        REFRESH => vec![toast(&format!("Not refreshed: {}", why()))],
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
        "zip" => "hash",
        "plex" | "schedules-direct" | "gracenote" => "tv-minimal",
        "plex-server" | "plex.address" => "server",
        "plex.token" | "sd.password" => "key-round",
        "sd.username" => "user",
        _ => "",
    }
}

fn rows(c: &Ctx) -> Vec<Row> {
    let Some(e) = entry(c) else {
        return vec![Row::empty("Not available", "The TV's helper is not answering, or does not have the guide data plugin.", "tv-minimal", 0)];
    };
    let (consent, code) = (text(e, "consent").to_string(), text(e, "approve").to_string());
    let shared = on(c);
    let mut rows = vec![Row::toggle("use").icon("tv-minimal").label("Online listings").help(&consent).on(shared).admin().act(move |a| match a {
        Arg::Toggle(true) => ask("tv-minimal", "Fill the guide from online listings?", &consent, "Turn on", "Cancel", false, vec![switch(true, &code)]),
        Arg::Toggle(false) => vec![switch(false, "")],
        _ => Vec::new(),
    })];
    if !shared {
        return rows;
    }
    let Some(o) = c.owner(ID).filter(|o| o.known) else {
        rows.push(Row::note("The guide data page comes once the plugin is running.", 0));
        return rows;
    };
    let refresh = Op::Send { name: Some(REFRESH.into()), method: "POST", path: format!("/v1/plugins/{ID}/actions/refresh"), body: Some(json!({})) };
    let mut section = None;
    for (s, f) in fields(PLUGIN, &o.schema) {
        if section.is_some_and(|was| was != s)
            && let Some(title) = o.schema["sections"].as_array().into_iter().flatten().find(|x| text(x, "id") == PLUGIN.sections[s]).map(|x| text(x, "title"))
        {
            rows.push(Row::head(title, 0));
        }
        if section.is_none() {
            rows.push(Row::action("refresh", "Refresh now").icon("refresh-cw").label("Listings").admin().does(vec![refresh.clone()]));
        }
        section = Some(s);
        rows.extend(field_rows(c, PLUGIN, 0, f, icon(text(f, "key"))));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;

    fn schema() -> Value {
        let fact =
            |key: &str, label: &str| json!({"key": key, "label": label, "kind": {"type": "read-only", "value": {"capability": format!("plugin.{ID}.{key}")}}});
        let text = |key: &str, secret: bool| json!({"key": key, "label": key, "kind": {"type": "text", "max_chars": 64, "secret": secret}});
        json!({"sections": [
            {"id": "listings", "title": "Listings", "fields": [fact("status", "Status"), text("zip", false),
                {"key": "sources", "label": "Sources", "kind": {"type": "ordered-set", "max_items": 3,
                 "options": {"inline": [{"id": "plex", "label": "Plex"}, {"id": "schedules-direct", "label": "Schedules Direct"}, {"id": "gracenote", "label": "Free listings"}]}}}]},
            {"id": "plex", "title": "Plex", "fields": [fact("plex", "Listings"), text("plex.address", false), text("plex.token", true)]},
            {"id": "schedules-direct", "title": "Schedules Direct", "fields": [text("sd.username", false), text("sd.password", true)]}]})
    }

    fn world(state: &str) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.helper_up = true;
        w.topics.set_helper(
            "plugins",
            json!({"plugins": [], "firstParty": [{"id": ID, "name": "Guide data", "state": state, "firstParty": true,
                   "consent": "Fetches TV listings for a zip code from the services picked.", "approve": "c0de"}]}),
        );
        if state == "connected" {
            let mut o = crate::settings::src::Owner::new(ID);
            o.take(&json!({"ok": true, "version": 3, "schema": schema(),
                "effective": {"zip": {"value": "30311"}, "sources": {"value": ["gracenote", "plex"]}, "plex.address": {"value": ""}}}));
            w.owners.insert(ID.into(), o);
            w.topics.set_helper(
                &format!("plugin.{ID}.facts"),
                json!({"facts": {"status": "53 of 57 channels have online listings", "plex": "Updated 2:05 PM: 53 of 57 channels", "plex-token": "Kept on this TV"}}),
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
        assert_eq!(question.body, "Fetches TV listings for a zip code from the services picked.");
        assert!(
            matches!(&question.answers[1].then[..], [Op::Send { path, body: Some(b), .. }] if path.ends_with("/guide-data/switch") && b == &json!({"on": true, "approve": "c0de"}))
        );
    }

    #[test]
    fn on_the_zip_and_the_sources_are_picked_here_a_secret_goes_to_the_plugin_alone_and_refresh_asks_again() {
        let w = world("connected");
        let c = w.ctx();
        assert_eq!(chip(&c), Some(("53 of 57 channels have online listings".into(), "positive")));
        let r = rows(&c);
        assert_eq!(row(&r, "plex").value, "Updated 2:05 PM: 53 of 57 channels");
        // a text is typed and written as the owner's
        let zip = row(&r, "zip");
        assert_eq!(zip.value, "30311");
        let [Op::Entry { then, secret: false, .. }] = &(zip.act.as_ref().unwrap())(Arg::Press)[..] else { panic!("typed") };
        assert!(matches!(&then(" 10001 ".into())[..], [Op::Owner { owner, key, value }] if owner == ID && key == "zip" && value == &json!("10001")));
        assert_eq!(row(&r, "plex.address").value, "Not set");
        // a secret goes to the plugin, and the row says only what the plugin says of it
        assert_eq!((row(&r, "plex.token").value.as_str(), row(&r, "sd.password").value.as_str()), ("Kept on this TV", "Not set"));
        let [Op::Entry { then, secret: true, text, .. }] = &(row(&r, "plex.token").act.as_ref().unwrap())(Arg::Press)[..] else { panic!("typed") };
        assert!(text.is_empty(), "a secret is never shown back");
        assert!(
            matches!(&then("tk".into())[..], [Op::Send { path, body: Some(b), .. }] if path == "/v1/plugins/guide-data/secret" && b == &json!({"field": "plex.token", "secret": "tk"}))
        );
        // the order the sources are trusted in
        assert_eq!((row(&r, "sources:gracenote").value.as_str(), row(&r, "sources:plex").value.as_str()), ("1", "2"));
        assert!(matches!(&(row(&r, "refresh").act.as_ref().unwrap())(Arg::Press)[..], [Op::Send { path, .. }] if path.ends_with("/actions/refresh")));
        assert!(
            matches!(&answered(&c, "secret:guide-data:plex.token", &json!({"ok": false, "error": {"message": "too long"}}))[..], [Op::Act { payload, .. }] if payload["message"] == "Not kept: too long")
        );
    }
}
