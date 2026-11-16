//! Gaming, a page of System: the switch of the helper's Gaming plugin (`helper/app/src/gaming`), which LumaGame and Quick Settings'
//! Game stats tile need. It asks first, in the words of its entry under `firstParty` in the `plugins` topic, as the Beszel page does;
//! off is one press. The plugin has no settings of its own, so on, the page says where its control is.

use super::plugin_rows::text;
use crate::panels::hub;
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::Page;
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

const ID: &str = "gaming";
/// The name the switch's answer comes back under.
const SWITCH: &str = "gaming.switch";
const ICON: &str = "gamepad-2";

pub fn page() -> Page {
    Page::new("gaming", "Gaming", ICON).parent("system").back("System").rows(rows).summary(summary).chip(chip).answered(answered)
}

/// The plugin's entry in the `plugins` topic: none from a helper that is not there, or older than the plugin.
fn entry<'a>(c: &'a Ctx) -> Option<&'a Value> {
    hub::plugin_of(c.helper("plugins")?, ID)
}

fn on(c: &Ctx) -> bool {
    entry(c).is_some_and(|e| text(e, "state") != "off")
}

/// The value of the row that opens the page.
pub fn summary(c: &Ctx) -> String {
    match entry(c) {
        None => "Not available".into(),
        Some(_) if on(c) => "On".into(),
        Some(_) => "Off".into(),
    }
}

fn chip(c: &Ctx) -> Option<(String, &'static str)> {
    Some((summary(c), if on(c) { "positive" } else { "info" }))
}

fn answered(_: &Ctx, name: &str, data: &Value) -> Vec<Op> {
    match name {
        "topic:plugins" => vec![Op::Refresh],
        SWITCH if data.get("ok").and_then(Value::as_bool) != Some(true) => {
            let why = data.pointer("/error/message").and_then(Value::as_str).unwrap_or("the TV did not answer");
            vec![toast(&format!("Not changed: {why}"))]
        }
        _ => Vec::new(),
    }
}

fn switch(on: bool, code: &str) -> Op {
    let body = if on { json!({"on": true, "approve": code}) } else { json!({"on": false}) };
    Op::Send { name: Some(SWITCH.into()), method: "POST", path: format!("/v1/plugins/{ID}/switch"), body: Some(body) }
}

fn rows(c: &Ctx) -> Vec<Row> {
    let Some(e) = entry(c) else {
        return vec![Row::empty("Not available", "The TV's helper is not answering, or does not have the Gaming plugin.", ICON, 0)];
    };
    let (consent, code) = (text(e, "consent").to_string(), text(e, "approve").to_string());
    let on = on(c);
    let mut rows = vec![Row::toggle("use").icon(ICON).label("Gaming").help(&consent).on(on).detail(hub::failure(e).unwrap_or_default()).admin().act(move |a| match a {
        Arg::Toggle(true) => ask(ICON, "Turn on Gaming?", &consent, "Turn on", "Cancel", false, vec![switch(true, &code)]),
        Arg::Toggle(false) => vec![switch(false, "")],
        _ => Vec::new(),
    })];
    if on {
        rows.push(Row::note("Game stats is in Quick Settings.", 0));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::World;

    fn world(state: &str) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.helper_up = true;
        w.topics.set_helper(
            "plugins",
            json!({"plugins": [], "firstParty": [{"id": ID, "name": "Gaming", "state": state, "firstParty": true,
                   "consent": "Allows the gaming apps to use low-latency controls.", "approve": "c0de"}]}),
        );
        w
    }

    #[test]
    fn turning_it_on_asks_in_the_plugins_words_and_sends_its_approval() {
        let w = world("off");
        let c = w.ctx();
        assert_eq!(chip(&c), Some(("Off".into(), "info")));
        let r = rows(&c);
        assert_eq!(r.len(), 1, "the switch alone");
        let [Op::Ask(question)] = &(r[0].act.as_ref().unwrap())(Arg::Toggle(true))[..] else { panic!("it asks first") };
        assert_eq!(question.body, "Allows the gaming apps to use low-latency controls.");
        assert!(
            matches!(&question.answers[1].then[..], [Op::Send { path, body: Some(b), .. }] if path == "/v1/plugins/gaming/switch" && b == &json!({"on": true, "approve": "c0de"}))
        );
        assert_eq!(chip(&world("connected").ctx()), Some(("On".into(), "positive")));
    }
}
