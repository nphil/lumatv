//! Auto brightness as the helper publishes it (the `autobright` topic, `helper/app/src/autobright.rs`): what Quick Settings' tiles and
//! Settings › Picture show beside OLED pixel brightness, and where a slider sends the owner's value while it is on.
//!
//! While it is on, a slider's value goes to the helper rather than to LG's service, as Android's and iOS's auto brightness take it: the
//! value chosen is the level wanted in the room's light at that moment, and the helper's curve follows it from then on. The helper
//! writes it at once and publishes it, so the slider shows the topic's `level`, the value on the panel, and never jumps back.

use serde_json::{Value, json};

pub const TOPIC: &str = "autobright";
/// The owner's switch in theme.json, as Settings names it and as a pointer into the document.
pub const SWITCH: &str = "system.autoBrightness";
pub const POINTER: &str = "/system/autoBrightness";
pub const LEVEL: &str = "/v1/autobright/level";
pub const LABEL: &str = "Auto brightness";
pub const ICON: &str = "sun-medium";
pub const HELP: &str = "Follows the room's light from the TV's own sensor, never brighter than the level you set. Moving OLED pixel brightness while it is on sets the level wanted in that light. LG's Energy saving is held off while it is on.";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// The owner's switch.
    pub on: bool,
    /// The helper drives the value of the picture in force.
    pub active: bool,
    /// The value on the panel, as the helper last read or wrote it.
    pub level: Option<f64>,
    /// What a row says under its name.
    pub detail: &'static str,
}

/// The owner's switch as the theme holds it (at [`SWITCH`]): off unless it says `true`.
pub fn switched_on(switch: Option<&Value>) -> bool {
    switch.and_then(Value::as_bool) == Some(true)
}

/// The state from the owner's switch and the helper's topic.
pub fn state(on: bool, topic: Option<&Value>) -> State {
    let t = topic.unwrap_or(&Value::Null);
    let active = on && t.get("active").and_then(Value::as_bool) == Some(true);
    let detail = if !on {
        ""
    } else if active {
        "Following the room"
    } else {
        match t.get("hold").and_then(Value::as_str) {
            Some("hdr") => "Paused for HDR",
            Some("game") => "Paused for a game",
            Some("panel dim") => "Paused while Home dims the screen",
            Some("error") => "The light sensor is not answering",
            _ => "Starting",
        }
    };
    State { on, active, level: active.then(|| t.get("level").and_then(Value::as_f64)).flatten(), detail }
}

/// The theme patch of the switch.
pub fn patch(on: bool) -> Value {
    json!({"system": {"autoBrightness": on}})
}

/// The body of `POST /v1/autobright/level`.
pub fn level_body(v: f64) -> Value {
    json!({"value": v.round().clamp(0.0, 100.0)})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_is_the_helpers_only_while_it_drives_the_picture() {
        let s = state(switched_on(patch(true).pointer(POINTER)), Some(&json!({"active": true, "level": 42, "hold": null})));
        assert_eq!((s.on, s.active, s.level, s.detail), (true, true, Some(42.0), "Following the room"));
        let s = state(true, Some(&json!({"active": false, "level": 100, "hold": "hdr"})));
        assert_eq!((s.active, s.level, s.detail), (false, None, "Paused for HDR"));
        assert!(!switched_on(None) && !switched_on(Some(&json!("on"))), "off unless the switch says true");
        assert_eq!(state(false, Some(&json!({"active": true, "level": 42}))), State::default());
    }
}
