//! Voice assistant, a page of Sound: the helper's plugin that makes the TV's microphone a Home Assistant voice satellite (`helper/voice`).
//! It asks first, in the words of its entry in the `plugins` topic; off is one press. While it is on, the page draws what the plugin names
//! (`plugin_rows`): how it stands, the microphone, Home Assistant, the last turn's time, the few choices and the two actions. Nothing that
//! was said or answered is here: that is on the screen only while it happens (`system/voice_assistant`).
use super::plugin_rows::{Plugin, fact, field_rows, fields, text};
use crate::panels::hub;
use crate::settings::ctx::{Ctx, ask, toast};
use crate::settings::page::{Page, Read};
use crate::settings::row::{Arg, Op, Row};
use serde_json::{Value, json};

const ID: &str = "voice-assistant";
const PLUGIN: Plugin = Plugin { id: ID, sections: &["assistant"] };
/// The name the switch's answer comes back under.
const SWITCH: &str = "voice-assistant.switch";
const ICON: &str = "mic";

pub fn page() -> Page {
    Page::new(ID, "Voice assistant", ICON).parent("sound").back("Sound").reads(|_| vec![Read::owner(ID)]).rows(rows).summary(summary).answered(answered)
}

/// The plugin's entry in the `plugins` topic: none from a helper that is not there, or older than the plugin.
fn entry<'a>(c: &'a Ctx) -> Option<&'a Value> {
    hub::plugin_of(c.helper("plugins")?, ID)
}

fn on(c: &Ctx) -> bool {
    entry(c).is_some_and(|e| text(e, "state") != "off")
}

/// The value of the row that opens the page: off, or how the assistant stands in the plugin's own words (its `status` line).
pub fn summary(c: &Ctx) -> String {
    match entry(c) {
        None => "Not available".into(),
        Some(_) if !on(c) => "Off".into(),
        Some(_) => fact(c, PLUGIN, "status").filter(|s| !s.is_empty()).unwrap_or_else(|| "Starting".into()),
    }
}

fn answered(_: &Ctx, name: &str, data: &Value) -> Vec<Op> {
    let ok = data.get("ok").and_then(Value::as_bool) == Some(true);
    let why = || data.pointer("/error/message").and_then(Value::as_str).unwrap_or("the TV did not answer").to_owned();
    match name {
        // switched on or off: the plugin's page came or went
        "topic:plugins" => vec![Op::Refresh],
        SWITCH if !ok => vec![toast(&format!("Not changed: {}", why()))],
        _ if !ok && name.starts_with("action:voice-assistant:") => vec![toast(&format!("Not done: {}", why()))],
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
        "microphone" => "mic",
        "homeassistant" => "house",
        "last" => "history",
        "others" => "radio",
        "room" => "door-open",
        "wake-word" => "audio-waveform",
        "sensitivity" => "sliders-horizontal",
        "speak" => "speaker",
        "volume" => "volume-2",
        "quiet-hours" => "moon",
        "quiet-from" | "quiet-until" => "clock",
        "pair" => "plug",
        "forget" => "unplug",
        _ => "",
    }
}

fn rows(c: &Ctx) -> Vec<Row> {
    let Some(e) = entry(c) else {
        return vec![Row::empty("Not available", "The TV's helper is not answering, or does not have the Voice assistant plugin.", ICON, 0)];
    };
    let (consent, code) = (text(e, "consent").to_owned(), text(e, "approve").to_owned());
    let mut rows = vec![Row::toggle("use").icon(ICON).label("Voice assistant").help(&consent).on(on(c)).detail(hub::failure(e).unwrap_or_default()).admin().act(move |a| match a {
        Arg::Toggle(true) => ask(ICON, "Let the TV listen for Home Assistant?", &consent, "Turn on", "Cancel", false, vec![switch(true, &code)]),
        Arg::Toggle(false) => vec![switch(false, "")],
        _ => Vec::new(),
    })];
    if !on(c) {
        return rows;
    }
    let Some(o) = c.owner(ID).filter(|o| o.known) else {
        rows.push(Row::note("The page comes once the plugin is running.", 0));
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
        let read = |key: &str, label: &str| json!({"key": key, "label": label, "kind": {"type": "read-only", "value": {"capability": format!("plugin.{ID}.{key}")}}});
        let choice = |key: &str, label: &str, options: &[(&str, &str)]| {
            let inline: Vec<Value> = options.iter().map(|(id, label)| json!({"id": id, "label": label})).collect();
            json!({"key": key, "label": label, "kind": {"type": "choice", "options": {"inline": inline}}})
        };
        let action = |key: &str, label: &str| json!({"key": key, "label": label, "kind": {"type": "action", "action": key, "verb": label, "confirmation": "none"}});
        json!({"sections": [{"id": "assistant", "title": "Voice assistant", "fields": [
            read("status", "Status"), read("microphone", "Microphone"), read("homeassistant", "Home Assistant"), read("last", "Last heard"),
            choice("wake-word", "Wake word", &[("okay_nabu", "Okay Nabu"), ("hey_jarvis", "Hey Jarvis")]),
            choice("sensitivity", "Sensitivity", &[("low", "Low"), ("normal", "Normal"), ("high", "High")]),
            {"key": "speak", "label": "Speak answers", "kind": {"type": "bool"}},
            {"key": "volume", "label": "Answer volume", "kind": {"type": "integer", "min": 1, "max": 100, "step": 5, "unit": "%"}},
            {"key": "quiet-hours", "label": "Quiet at night", "kind": {"type": "bool"}},
            action("pair", "Pair with Home Assistant"), action("forget", "Forget Home Assistant")]}]})
    }

    /// `None`: the plugin is off. `Some(facts)`: it is running, its page is served and it has published `facts`.
    fn world(facts: Option<Value>) -> World {
        let mut w = World::new(crate::host::sio::Sio::new(0));
        w.topics.helper_up = true;
        let state = if facts.is_some() { "running" } else { "off" };
        w.topics.set_helper(
            "plugins",
            json!({"plugins": [], "firstParty": [{"id": ID, "name": "Voice assistant", "state": state, "firstParty": true,
                   "consent": "Listens for the wake word and sends what is said to Home Assistant.", "approve": "c0de"}]}),
        );
        if let Some(facts) = facts {
            let mut o = crate::settings::src::Owner::new(ID);
            o.take(&json!({"ok": true, "version": 1, "schema": schema(), "effective": {"speak": {"value": true}, "volume": {"value": 60}}}));
            w.owners.insert(ID.into(), o);
            w.topics.set_helper(&format!("plugin.{ID}.facts"), json!({ "facts": facts }));
        }
        w
    }

    fn row<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    #[test]
    fn off_it_is_one_switch_that_asks_in_the_plugins_words() {
        let w = world(None);
        let c = w.ctx();
        assert_eq!(summary(&c), "Off");
        let r = rows(&c);
        assert_eq!(r.len(), 1, "the switch alone");
        let act = row(&r, "use").act.as_ref().unwrap();
        let [Op::Ask(question)] = &act(Arg::Toggle(true))[..] else { panic!("it asks first") };
        assert_eq!(question.body, "Listens for the wake word and sends what is said to Home Assistant.");
        assert!(matches!(&question.answers[1].then[..], [Op::Send { path, body: Some(b), .. }] if path.ends_with("/voice-assistant/switch") && b == &json!({"on": true, "approve": "c0de"})));
        assert!(matches!(&act(Arg::Toggle(false))[..], [Op::Send { body: Some(b), .. }] if b == &json!({"on": false})));
    }

    #[test]
    fn on_it_draws_the_schemas_rows_and_the_link_says_how_it_stands() {
        let w = world(Some(json!({"status": "Not paired", "homeassistant": "Not paired", "microphone": "Ready"})));
        let c = w.ctx();
        assert_eq!(summary(&c), "Not paired");
        let r = rows(&c);
        assert_eq!(r.len(), 12, "the switch and the schema's eleven fields");
        assert_eq!((row(&r, "microphone").value.as_str(), row(&r, "homeassistant").value.as_str()), ("Ready", "Not paired"));

        let paired = world(Some(json!({"status": "Listening for Okay Nabu", "homeassistant": "Paired with Home Assistant"})));
        assert_eq!(summary(&paired.ctx()), "Listening for Okay Nabu");
    }
}
