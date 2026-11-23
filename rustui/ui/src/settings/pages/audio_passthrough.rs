//! Dolby TrueHD & Atmos passthrough: the approval switch and the read-only status the helper reports, through the same plugin rows as every other
//! trusted-helper plugin, so colours, glass, radii and type come from the theme's tokens. No animation and no long-dwell element.
use super::plugin_rows::{Plugin, fact, field_rows, fields, text};
use crate::panels::hub;
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

const ID: &str = "audio-passthrough";
const PLUGIN: Plugin = Plugin { id: ID, sections: &["route"] };
const SWITCH: &str = "audio-passthrough.switch";
const ICON: &str = "audio-lines";

pub fn page() -> Page {
    Page::new(ID, "Dolby TrueHD & Atmos passthrough", ICON).parent("sound").back("Sound")
        .reads(|_| vec![Read::owner(ID)]).rows(rows).summary(summary).answered(answered)
}
fn entry<'a>(c: &'a Ctx) -> Option<&'a Value> { hub::plugin_of(c.helper("plugins")?, ID) }
fn on(c: &Ctx) -> bool { entry(c).is_some_and(|e| text(e, "state") != "off") }
/// Off, or the helper does not have it: the Sound page shows nothing more than "Off" beside the link then.
pub fn off(c: &Ctx) -> bool { !on(c) }
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
    let Some(e) = entry(c) else { return vec![Row::empty("Not available", "The helper does not have Dolby TrueHD & Atmos passthrough, or is not answering.", ICON, 0)] };
    let (consent, code) = (text(e, "consent").to_owned(), text(e, "approve").to_owned());
    let mut rows = vec![Row::toggle("use").icon(ICON).label("Dolby TrueHD & Atmos passthrough").help(&consent).on(on(c))
        .detail(hub::failure(e).unwrap_or_default()).admin().act(move |a| match a {
            Arg::Toggle(true) => ask(ICON, "Pass TrueHD to the receiver?", &consent, "Turn on", "Cancel", false, vec![switch(true, &code)]),
            Arg::Toggle(false) => vec![switch(false, "")],
            _ => Vec::new(),
        })];
    if !on(c) { return rows; }
    let Some(owner) = c.owner(ID).filter(|o| o.known) else { rows.push(Row::note("Status appears once the plugin is running.", 0)); return rows };
    for (_, field) in fields(PLUGIN, &owner.schema) {
        let icon = match text(field, "key") { "output" | "receiver" => "boom-box", "session" => ICON, _ => "activity" };
        rows.extend(field_rows(c, PLUGIN, 0, field, icon));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;

    fn world(state: &str) -> World {
        let mut world = World::new(crate::host::sio::Sio::new(0));
        world.topics.helper_up = true;
        world.topics.set_helper("plugins", json!({"plugins":[],"firstParty":[{"id":ID,"state":state,"firstParty":true,"consent":"Passes TrueHD to the receiver.","approve":"code"}]}));
        world
    }

    #[test]
    fn audio_passthrough_is_off_until_approved_and_off_needs_no_second_confirmation() {
        let world = world("off");
        let c = world.ctx();
        assert_eq!((summary(&c), off(&c)), ("Off".to_owned(), true));
        let rows = rows(&c);
        assert_eq!(rows.len(), 1);
        let act = rows[0].act.as_ref().unwrap();
        assert!(matches!(&act(Arg::Toggle(false))[..], [Op::Send { body: Some(b), .. }] if b == &json!({"on":false})));
        assert!(!matches!(&act(Arg::Toggle(true))[..], [Op::Send { .. }]));
    }

    #[test]
    fn a_running_plugin_without_a_status_yet_says_it_is_starting() {
        let world = world("running");
        let c = world.ctx();
        assert!(!off(&c));
        assert_eq!(summary(&c), "Starting");
    }
}
