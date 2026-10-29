//! Home Assistant, a page of Home & look (`settings/PageHomeAssistant.qml`, `PluginRows.js`): the settings page of LumaShow's Home Assistant
//! plugin, drawn from the schema the helper serves for it (`GET /v1/settings/home-assistant`) and written through the same route, so a choice
//! made here is the plugin's setting, as it is on the Show. The pickers (the room, its lights and scenes, the doorbell, the presence
//! sensors) are lists the plugin makes from Home Assistant (`plugin_rows`).
//!
//! What is not here, on purpose: the address and the access token. They are the owner's alone, given from a shell on the TV
//! (`lumatv-helper ha-link`, the token on standard input) so that no key press, no route of the helper and no plugin can set either. This
//! page shows the address and whether a token is kept, and can forget them: that only ever takes access away.

use crate::panels::hub;
use super::plugin_rows::{Plugin, ask_all, field_rows, fields};
use crate::settings::ctx::{Ctx, ask};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Op, Row};
use serde_json::{Value, json};

const ID: &str = "home-assistant";
/// The sections the plugin declares, in the order the tabs are (a section it does not declare leaves its tab with a line saying so).
const PLUGIN: Plugin = Plugin { id: ID, sections: &["connection", "room", "doorbell", "presence", "notifications"] };

pub fn page() -> Page {
    Page::new("homeassistant", "Home Assistant", "house-wifi")
        .parent("look")
        .back("Home & look")
        .tabs(&["Connection", "Room", "Doorbell", "Presence", "Alerts"])
        .reads(reads)
        .rows(rows)
        .summary(words)
        .chip(chip)
        .opened(opened)
        .answered(answered)
}

fn reads(_: &Ctx) -> Vec<Read> {
    vec![Read::owner(ID)]
}

fn opened(_: &Ctx) -> Vec<Op> {
    vec![Op::Set("asked".into(), json!(""))]
}

/// Where it stands, in one word (`PluginHub.haState`; see [`hub::ha_state`]).
pub fn state(c: &Ctx) -> String {
    hub::ha_state(c.helper("plugins").unwrap_or(&Value::Null), c.helper("ha.link").unwrap_or(&Value::Null))
}

/// A short phrase for a row that opens the page (`PluginHub.haWords`).
pub fn words(c: &Ctx) -> String {
    hub::ha_words(c.helper("plugins").unwrap_or(&Value::Null), c.helper("ha.link").unwrap_or(&Value::Null)).into()
}

fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    let tone = match state(c).as_str() {
        "connected" => "positive",
        "refused" | "inert" => "danger",
        "off" | "unlinked" | "connecting" => "info",
        _ => "warning",
    };
    Some((words(c), tone))
}

fn answered(c: &Ctx, name: &str, _: &Value) -> Vec<Op> {
    match name {
        // the link or the plugin came or went: the page's schema may have appeared or gone
        "topic:plugins" | "topic:ha.link" => vec![Op::Refresh],
        // The lists are asked again when the page is opened, when the link comes up and when the room changes (the lights and the
        // sensors are the room's); the last answer stays until the next.
        "owner:home-assistant" => {
            let Some(o) = c.owner(ID).filter(|o| o.known) else { return Vec::new() };
            let room = o.text("room").unwrap_or_default();
            let key = format!("{room}|{}", state(c));
            if c.local.text("asked") == key {
                return Vec::new();
            }
            let mut ops = vec![Op::Set("asked".into(), json!(key))];
            ops.extend(ask_all(PLUGIN, &o.schema));
            ops
        }
        _ => Vec::new(),
    }
}

fn link<'a>(c: &'a Ctx, key: &str) -> Option<&'a Value> {
    c.helper("ha.link").and_then(|l| l.get(key)).filter(|v| !v.is_null())
}

fn rows(c: &Ctx) -> Vec<Row> {
    let token = link(c, "token").is_some_and(|t| t.as_bool() != Some(false));
    let mut own = vec![
        {
            let mut r = Row::info("address").tab(0).icon("globe").label("Address").help("Set with lumatv-helper ha-link, in a shell on the TV");
            r.value = link(c, "address").map(crate::host::sio::js::string).filter(|a| !a.is_empty()).unwrap_or_else(|| "Not set".into());
            r
        },
        {
            let mut r = Row::info("token").tab(0).icon("key-round").label("Access token").help("Kept on this TV and never shown. Set with lumatv-helper ha-link, in a shell on the TV");
            r.value = if token { "Saved" } else { "Not saved" }.into();
            r
        },
        Row::action("forget", "Forget")
            .tab(0)
            .icon("unplug")
            .label("Forget link")
            .help("Deletes the saved address and token")
            .danger()
            .enabled(token && state(c) != "off")
            .admin()
            .does(ask("unplug", "Forget the link?", "The saved address and token are deleted. Linking again is done from a shell on the TV.", "Forget", "Cancel", true, vec![Op::Send { name: None, method: "POST", path: format!("/v1/plugins/{ID}/actions/lease.ha.forget"), body: Some(json!({})) }])),
    ];
    let Some(o) = c.owner(ID).filter(|o| o.known) else {
        // no page from the plugin yet: the first tab only, the others say why
        own.extend((1..PLUGIN.sections.len()).map(|t| Row::empty("Not available yet", &format!("{} is set once Home Assistant is linked and the plugin is running.", ["", "The room", "The doorbell", "Presence", "Alerts"][t]), "house-wifi", t)));
        return own;
    };
    let mut first = Vec::new();
    let mut rest = Vec::new();
    for (tab, f) in fields(PLUGIN, &o.schema) {
        let made = field_rows(c, PLUGIN, tab, f, icon(f.get("key").and_then(Value::as_str).unwrap_or("")));
        if tab == 0 { first.extend(made) } else { rest.extend(made) }
    }
    for t in 1..PLUGIN.sections.len() {
        if !rest.iter().any(|r| r.tab == t) {
            rest.push(Row::empty("Nothing to set", "The plugin has no settings for this.", "house-wifi", t));
        }
    }
    first.extend(own);
    first.extend(rest);
    first
}

/// A glyph for the fields the plugin names; the rest have none (the QML rows have none either).
fn icon(key: &str) -> &'static str {
    match key {
        "state" => "plug",
        "summary" => "eye",
        "notify_doorbell" | "notify_ha" => "bell",
        "hold" => "timer",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;
    use crate::settings::row::Arg;

    fn schema() -> Value {
        json!({"sections": [
            {"id": "connection", "title": "Connection", "fields": [{"key": "state", "label": "Connection", "kind": {"type": "read-only", "value": {"capability": "plugin.home-assistant.link"}}}]},
            {"id": "room", "title": "This room", "fields": [
                {"key": "room", "label": "Room", "kind": {"type": "choice", "options": {"provider": "rooms"}}},
                {"key": "controls", "label": "Lights and scenes", "kind": {"type": "ordered-set", "max_items": 2, "options": {"provider": "controls"}}},
                {"key": "summary", "label": "Watching", "kind": {"type": "read-only", "value": {"capability": "plugin.home-assistant.summary"}}, "visible_when": {"type": "capability-present", "capability": "plugin.home-assistant.summary"}}]},
            {"id": "presence", "title": "Who is in the room", "fields": [{"key": "hold", "label": "Still here for", "kind": {"type": "integer", "min": 10, "max": 600, "step": 10, "unit": "s"}}]},
            {"id": "notifications", "title": "Notifications", "fields": [{"key": "notify_ha", "label": "Home Assistant's notifications", "kind": {"type": "bool"}}]}
        ]})
    }

    fn world(link: &str) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.helper_up = true;
        w.topics.set_helper("plugins", json!({"plugins": [{"id": "home-assistant", "state": "connected"}], "firstParty": []}));
        w.topics.set_helper("ha.link", json!({"state": link, "address": "http://ha.local:8123", "token": true}));
        let mut o = crate::settings::src::Owner::new(ID);
        o.take(&json!({"ok": true, "version": 1, "schema": schema(), "effective": {
            "room": {"value": "kitchen"}, "controls": {"value": ["light.lamp"]}, "hold": {"value": 60}, "notify_ha": {"value": true}}}));
        w.owners.insert(ID.into(), o);
        w
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    #[test]
    fn the_page_says_where_the_link_stands_in_the_words_of_the_plugin_hub() {
        let mut w = world("connected");
        assert_eq!((words(&w.ctx()), chip(&w.ctx())), ("Connected".into(), Some(("Connected".into(), "positive"))));
        w.topics.set_helper("ha.link", json!({"state": "auth_failed"}));
        assert_eq!(chip(&w.ctx()), Some(("Token refused".into(), "danger")));
        w.topics.set_helper("ha.link", json!({"state": "off"}));
        assert_eq!(words(&w.ctx()), "Not linked");
        w.topics.set_helper("plugins", json!({"plugins": [], "firstParty": []}));
        assert_eq!(words(&w.ctx()), "Not switched on");
        w.topics.set_helper("plugins", json!({"plugins": [{"id": "home-assistant", "state": "inert"}]}));
        assert_eq!(words(&w.ctx()), "Cannot run");
    }

    #[test]
    fn a_picker_lists_what_the_plugin_answered_marks_the_choice_and_writes_it_to_the_plugins_page() {
        let mut w = world("connected");
        w.docs.set("choices:home-assistant:rooms", json!({"ok": true, "page": {"state": "ready", "rows": [{"id": "kitchen", "label": "Kitchen"}, {"id": "living_room", "label": "Living room"}], "more": false}}));
        w.docs.set("choices:home-assistant:controls", json!({"ok": true, "page": {"state": "ready", "rows": [{"id": "light.lamp", "label": "Lamp", "detail": "Living room"}, {"id": "scene.movie", "label": "Movie"}, {"id": "light.hall", "label": "Hall"}], "more": true}}));
        let r = rows(&w.ctx());
        assert!(row(&r, "room:kitchen").selected && !row(&r, "room:living_room").selected);
        assert!(matches!(&(row(&r, "room:living_room").act.as_ref().unwrap())(Arg::Press)[..], [Op::Owner { owner, key, value }] if owner == ID && key == "room" && value == &json!("living_room")));
        // an ordered set: the order is shown, a pick is added after the others, a second pick removes, the limit holds
        assert_eq!((row(&r, "controls:light.lamp").value.as_str(), row(&r, "controls:light.lamp").detail.as_str()), ("1", "Living room"));
        let add = (row(&r, "controls:scene.movie").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&add[..], [Op::Owner { value, .. }] if value == &json!(["light.lamp", "scene.movie"])));
        let drop = (row(&r, "controls:light.lamp").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&drop[..], [Op::Owner { value, .. }] if value == &json!([])));
        assert!(rows(&w.ctx()).iter().any(|r| r.text == "First 3 shown"));
        // the other kinds of field
        assert_eq!(row(&r, "hold").value_text.as_deref(), Some("60 s"));
        assert!(row(&r, "notify_ha").on);
        assert!(!row(&r, "summary").visible, "a read-only line the plugin has not published is left out");
        w.topics.set_helper("plugin.home-assistant.facts", json!({"facts": {"summary": "3 lights", "link": "Connected"}}));
        let r = rows(&w.ctx());
        assert_eq!((row(&r, "summary").visible, row(&r, "summary").value.as_str(), row(&r, "state").value.as_str()), (true, "3 lights", "Connected"));
    }

    #[test]
    fn a_list_not_yet_asked_says_loading_and_one_the_plugin_cannot_make_says_why() {
        let mut w = world("offline");
        assert!(rows(&w.ctx()).iter().any(|r| r.text == "Loading\u{2026}"));
        w.docs.set("choices:home-assistant:rooms", json!({"ok": true, "page": {"state": "offline", "rows": [], "more": false}}));
        assert!(rows(&w.ctx()).iter().any(|r| r.text == "Offline"));
    }

    #[test]
    fn the_address_is_shown_not_typed_the_token_never_and_forgetting_asks_first() {
        let w = world("connected");
        let r = rows(&w.ctx());
        assert_eq!((row(&r, "address").value.as_str(), row(&r, "token").value.as_str()), ("http://ha.local:8123", "Saved"));
        assert!(r.iter().all(|r| !r.value.contains("secret")) && row(&r, "address").act.is_none());
        let forget = row(&r, "forget");
        assert!(forget.enabled && forget.danger);
        assert!(matches!(&(forget.act.as_ref().unwrap())(Arg::Press)[..], [Op::Ask(a)] if matches!(&a.answers[1].then[..], [Op::Send { path, .. }] if path.ends_with("/actions/lease.ha.forget"))));
        let mut w = world("unconfigured");
        w.topics.set_helper("ha.link", json!({"state": "unconfigured"}));
        assert!(!row(&rows(&w.ctx()), "forget").enabled, "nothing to forget");
    }

    #[test]
    fn without_the_plugins_page_only_the_connection_tab_has_rows_and_the_others_say_so() {
        let mut w = world("connected");
        w.owners.clear();
        let r = rows(&w.ctx());
        assert!(r.iter().filter(|r| r.tab == 0).all(|r| matches!(r.id.as_str(), "address" | "token" | "forget")));
        assert_eq!(r.iter().filter(|r| r.kind == crate::settings::row::Kind::Empty).count(), 4);
    }

    #[test]
    fn the_lists_are_asked_once_per_room_and_link_state_and_again_when_either_changes() {
        let mut w = world("connected");
        let first = answered(&w.ctx(), "owner:home-assistant", &Value::Null);
        let asked: Vec<&str> = first.iter().filter_map(|o| if let Op::Send { path, .. } = o { path.rsplit('/').next() } else { None }).collect();
        assert_eq!(asked, ["rooms?limit=64", "controls?limit=64"]);
        w.local.set("asked", json!("kitchen|connected"));
        assert!(answered(&w.ctx(), "owner:home-assistant", &Value::Null).is_empty());
        w.topics.set_helper("ha.link", json!({"state": "offline"}));
        assert_eq!(answered(&w.ctx(), "owner:home-assistant", &Value::Null).len(), 3, "the key and two lists");
        assert!(matches!(&answered(&w.ctx(), "topic:ha.link", &Value::Null)[..], [Op::Refresh]));
    }
}
