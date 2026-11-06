//! Home Assistant device, a page of Inputs & devices: the TV as an ESPHome device of Home Assistant's, served by two of the helper's own
//! plugins (`helper/app/src/ble.rs`, `tvsensors.rs`): the Bluetooth proxy, and the TV's sensors. Each asks first, in the words of its
//! entry in the `plugins` topic, as the tuner's page does; off is one press. While one is on, its line of status is shown. The device's
//! key and port are the owner's, given from a shell (`lumatv-helper ble-link`): shown here only as the plugins word them.

use super::plugin_rows::{Plugin, fact, text};
use crate::panels::hub;
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::Page;
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

const PROXY: Plugin = Plugin { id: "ble-proxy", sections: &[] };
const SENSORS: Plugin = Plugin { id: "tv-sensors", sections: &[] };
/// The name the answer to a switch comes back under.
const SWITCH: &str = "hadevice.switch";

pub fn page() -> Page {
    Page::new("hadevice", "Home Assistant device", "radio")
        .parent("inputs")
        .back("Inputs & devices")
        .rows(rows)
        .summary(summary)
        .answered(answered)
}

/// A plugin's entry in the `plugins` topic: none from a helper that is not there, or older than the plugin.
fn entry<'a>(c: &'a Ctx, id: &str) -> Option<&'a Value> {
    hub::plugin_of(c.helper("plugins")?, id)
}

fn on(c: &Ctx, id: &str) -> bool {
    entry(c, id).is_some_and(|e| text(e, "state") != "off")
}

/// The value of the row that opens the page: which of the two is on.
pub fn summary(c: &Ctx) -> String {
    if entry(c, PROXY.id).is_none() {
        return "Not available".into();
    }
    match (on(c, PROXY.id), on(c, SENSORS.id)) {
        (true, true) => "Bluetooth proxy and sensors".into(),
        (true, false) => "Bluetooth proxy".into(),
        (false, true) => "Sensors".into(),
        (false, false) => "Off".into(),
    }
}

fn answered(_: &Ctx, name: &str, data: &Value) -> Vec<Op> {
    let ok = data.get("ok").and_then(Value::as_bool) == Some(true);
    match name {
        // switched on or off: the plugin's status came or went
        "topic:plugins" => vec![Op::Refresh],
        SWITCH if !ok => {
            let why = data.pointer("/error/message").and_then(Value::as_str).unwrap_or("the TV did not answer");
            vec![toast(&format!("Not changed: {why}"))]
        }
        _ => Vec::new(),
    }
}

fn switch(id: &str, on: bool, code: &str) -> Op {
    let body = if on { json!({"on": true, "approve": code}) } else { json!({"on": false}) };
    Op::Send { name: Some(SWITCH.into()), method: "POST", path: format!("/v1/plugins/{id}/switch"), body: Some(body) }
}

/// One plugin: its switch, which asks in its words, and while it is on its status and the facts it names.
fn plugin_rows(c: &Ctx, plugin: Plugin, icon: &'static str, label: &str, question: &'static str) -> Vec<Row> {
    let Some(e) = entry(c, plugin.id) else {
        return Vec::new();
    };
    let (consent, code, id) = (text(e, "consent").to_string(), text(e, "approve").to_string(), plugin.id);
    let lit = on(c, id);
    let mut rows = vec![Row::toggle(id).icon(icon).label(label).help(&consent).on(lit).admin().act(move |a| match a {
        Arg::Toggle(true) => ask(icon, question, &consent, "Turn on", "Cancel", false, vec![switch(id, true, &code)]),
        Arg::Toggle(false) => vec![switch(id, false, "")],
        _ => Vec::new(),
    })];
    if lit {
        let status = fact(c, plugin, "status").filter(|s| !s.is_empty()).unwrap_or_else(|| "Starting".into());
        rows.push(Row::info(&format!("{id}:status")).icon("activity").label("Status").value(&status));
        if let Some(address) = fact(c, plugin, "address").filter(|s| !s.is_empty()) {
            rows.push(Row::info(&format!("{id}:address")).icon("network").label("Address").value(&address));
        }
    }
    rows
}

fn rows(c: &Ctx) -> Vec<Row> {
    if entry(c, PROXY.id).is_none() {
        return vec![Row::empty("Not available", "The TV's helper is not answering, or does not have these plugins.", "radio", 0)];
    }
    let mut rows = plugin_rows(c, PROXY, "bluetooth", "Bluetooth proxy", "Use this TV as a Bluetooth proxy?");
    rows.extend(plugin_rows(c, SENSORS, "activity", "TV sensors", "Send this TV's state to Home Assistant?"));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;

    fn world(proxy: &str, sensors: &str) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.helper_up = true;
        let entry = |id: &str, name: &str, state: &str| {
            json!({"id": id, "name": name, "state": state, "firstParty": true, "consent": format!("{name} words."), "approve": format!("{id}-code")})
        };
        w.topics.set_helper(
            "plugins",
            json!({"plugins": [], "firstParty": [entry("ble-proxy", "Bluetooth proxy", proxy), entry("tv-sensors", "TV sensors", sensors)]}),
        );
        w.topics.set_helper("plugin.ble-proxy.facts", json!({"facts": {"status": "Waiting for Home Assistant", "address": "192.168.1.242:6053"}}));
        w
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    #[test]
    fn each_plugin_is_one_switch_that_asks_in_its_words_and_an_on_one_says_where_it_stands() {
        let w = world("connected", "off");
        let c = w.ctx();
        assert_eq!(summary(&c), "Bluetooth proxy");
        let r = rows(&c);
        assert_eq!(row(&r, "ble-proxy:status").value, "Waiting for Home Assistant");
        assert_eq!(row(&r, "ble-proxy:address").value, "192.168.1.242:6053");
        assert!(r.iter().all(|x| x.id != "tv-sensors:status"), "an off plugin shows its switch alone");
        let [Op::Ask(question)] = &(row(&r, "tv-sensors").act.as_ref().unwrap())(Arg::Toggle(true))[..] else { panic!("it asks first") };
        assert_eq!(question.body, "TV sensors words.");
        assert!(
            matches!(&question.answers[1].then[..], [Op::Send { path, body: Some(b), .. }] if path == "/v1/plugins/tv-sensors/switch" && b == &json!({"on": true, "approve": "tv-sensors-code"}))
        );
        assert!(matches!(&(row(&r, "ble-proxy").act.as_ref().unwrap())(Arg::Toggle(false))[..], [Op::Send { body: Some(b), .. }] if b == &json!({"on": false})));
    }
}
