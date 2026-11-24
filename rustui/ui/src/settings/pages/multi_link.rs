//! Multi Link uses the same approval and schema rows as every trusted-helper plugin.
use super::plugin_rows::{Plugin, fact, field_rows, fields, text};
use crate::panels::hub;
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

const ID: &str = "multi-link";
const PLUGIN: Plugin = Plugin { id: ID, sections: &["links"] };
const SWITCH: &str = "multi-link.switch";

pub fn page() -> Page {
    Page::new(ID, "Multi Link", "ethernet-port").parent("network").back("Network")
        .reads(|_| vec![Read::owner(ID)]).rows(rows).summary(summary).answered(answered)
}
fn entry<'a>(c: &'a Ctx) -> Option<&'a Value> { hub::plugin_of(c.helper("plugins")?, ID) }
fn on(c: &Ctx) -> bool { entry(c).is_some_and(|e| text(e, "state") != "off") }
pub fn summary(c: &Ctx) -> String {
    match entry(c) {
        None => "Not available".into(),
        Some(_) if !on(c) => "Off".into(),
        Some(_) => fact(c, PLUGIN, "status").filter(|s| !s.is_empty()).unwrap_or_else(|| "Starting".into()),
    }
}
fn switch(enabled: bool, code: &str) -> Op {
    Op::Send { name: Some(SWITCH.into()), method: "POST", path: format!("/v1/plugins/{ID}/switch"),
        body: Some(if enabled { json!({"on":true,"approve":code}) } else { json!({"on":false}) }) }
}
fn answered(_: &Ctx, name: &str, value: &Value) -> Vec<Op> {
    match name {
        "topic:plugins" => vec![Op::Refresh],
        SWITCH if value.get("ok").and_then(Value::as_bool) != Some(true) => vec![toast(&format!("Not changed: {}", value.pointer("/error/message").and_then(Value::as_str).unwrap_or("the TV did not answer")))],
        _ => Vec::new(),
    }
}
fn rows(c: &Ctx) -> Vec<Row> {
    let Some(e) = entry(c) else { return vec![Row::empty("Not available", "The helper does not have Multi Link, or is not answering.", "ethernet-port", 0)] };
    let (consent, code) = (text(e, "consent").to_owned(), text(e, "approve").to_owned());
    let mut rows = vec![Row::toggle("use").icon("ethernet-port").label("Multi Link").help(&consent).on(on(c))
        .detail(hub::failure(e).unwrap_or_default()).admin().act(move |a| match a {
            Arg::Toggle(true) => ask("ethernet-port", "Use more than one connection?", &consent, "Turn on", "Cancel", false, vec![switch(true, &code)]),
            Arg::Toggle(false) => vec![switch(false, "")],
            _ => Vec::new(),
        })];
    if !on(c) { return rows; }
    let Some(owner) = c.owner(ID).filter(|o| o.known) else { rows.push(Row::note("Connection details appear once the plugin is running.", 0)); return rows };
    for (_, field) in fields(PLUGIN, &owner.schema) {
        let icon = match text(field, "key") { "ethernet" | "usb" => "ethernet-port", "wifi" => "wifi", "controls" => "radio", "bulk" => "download", _ => "activity" };
        rows.extend(field_rows(c, PLUGIN, 0, field, icon));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;

    #[test]
    fn multi_link_is_off_until_approved_and_off_needs_no_second_confirmation() {
        let mut world = World::new(crate::host::sio::Sio::new(0));
        world.topics.helper_up = true;
        world.topics.set_helper("plugins", json!({"plugins":[],"firstParty":[{"id":ID,"state":"off","firstParty":true,"consent":"Uses both links.","approve":"code"}]}));
        let c = world.ctx();
        assert_eq!(summary(&c), "Off");
        let rows = rows(&c);
        assert_eq!(rows.len(), 1);
        let act = rows[0].act.as_ref().unwrap();
        assert!(matches!(&act(Arg::Toggle(false))[..], [Op::Send { body: Some(b), .. }] if b == &json!({"on":false})));
        assert!(!matches!(&act(Arg::Toggle(true))[..], [Op::Send { .. }]));
    }

    #[test]
    fn the_connection_rows_stay_when_a_later_read_of_the_plugin_fails() {
        use crate::settings::src::Owner;
        let mut world = World::new(crate::host::sio::Sio::new(0));
        world.topics.helper_up = true;
        world.topics.set_helper("plugins", json!({"plugins":[],"firstParty":[{"id":ID,"state":"on","firstParty":true,"consent":"Uses both links.","approve":"code"}]}));
        world.topics.set_helper("plugin.multi-link.facts", json!({"facts": {"status": "Ready for supported apps", "ethernet": "100 Mbps"}}));
        let field = |key: &str| json!({"key": key, "label": key, "kind": {"type": "read-only", "value": {"capability": format!("plugin.{ID}.{key}")}}});
        let mut owner = Owner::new(ID);
        owner.take(&json!({"ok": true, "version": 1, "effective": {}, "schema": {"sections": [{"id": "links", "fields": [field("status"), field("ethernet"), field("usb")]}]}}));
        world.owners.insert(ID.into(), owner);
        let before: Vec<String> = rows(&world.ctx()).iter().map(|r| r.id.clone()).collect();
        assert_eq!(before.len(), 4, "the switch and one row per line: {before:?}");
        for (status, body) in [(0, Value::Null), (404, json!({"ok": false, "error": {"code": "no_such_owner"}})), (503, json!({}))] {
            world.owners.get_mut(ID).unwrap().fetched(status, &body);
            let after: Vec<String> = rows(&world.ctx()).iter().map(|r| r.id.clone()).collect();
            assert_eq!(after, before, "a failed read ({status}) leaves the page as it was");
        }
    }
}
