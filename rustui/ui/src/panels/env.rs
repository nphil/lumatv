//! What the panels know of the TV: the services' models (the settings service, the inputs, the sleep timer, the network, the
//! receiver's gate), the helper's topics, the owner's theme, who is in front. One value the panels' models read and, to write a
//! setting or ask a service, change (each change answers with the effects it needs). The host feeds it every input, so what the
//! panels show is current whether they are open or not; the services that are costly (the inputs' subscription, the settings
//! reads) are only asked for by a panel that is coming up.

use super::hub::Topics;
use crate::host::inputs::Inputs;
use crate::host::net::Net;
use crate::host::power::Power;
use crate::host::receiver::Gate;
use crate::host::sio::Sio;
use crate::kit::screen::{Effect, Input};
use serde_json::Value;

/// The tags of the panels' own calls to the helper and the channel (`0x5100_0000..`).
pub mod tags {
    pub const CAPTURE_READ: u64 = 0x5100_0001;
    pub const CAPTURE_POST: u64 = 0x5100_0002;
    pub const AUDIT: u64 = 0x5100_0003;
    pub const PROFILE_SWITCH: u64 = 0x5100_0004;
    pub const THEME: u64 = 0x5100_0005;
    pub const LAUNCH: u64 = 0x5100_0006;
    pub const VOLUME_NUDGE: u64 = 0x5100_0007;
    pub const RESUME: u64 = 0x5100_0008;
    pub const PLUGIN: u64 = 0x5100_0009;
    pub const CLOSE_PIN: u64 = 0x5100_000A;
    pub const SETTINGS: u64 = 0x5100_000B;
}

/// What the compositor says is in front.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Foreground {
    pub app: String,
    pub home: bool,
    pub known: bool,
}

/// What the helper's `/v1/capture` said, reduced to what a tile shows (`settings/Capture.js`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Capture {
    pub answered: bool,
    pub enabled: bool,
    pub available: bool,
    pub recording: bool,
    pub encoder: bool,
}

impl Capture {
    pub fn from_doc(status: u16, doc: &Value) -> Self {
        let Some(c) = (status == 200).then(|| doc.get("capture")).flatten().filter(|c| !c.is_null()) else { return Self::default() };
        if c.get("enabled").and_then(Value::as_bool) != Some(true) {
            return Self { answered: true, ..Self::default() };
        }
        Self {
            answered: true,
            enabled: true,
            available: c.get("available").and_then(Value::as_bool) == Some(true),
            recording: c.get("recording").and_then(Value::as_bool) == Some(true),
            encoder: c.pointer("/encoder/available").and_then(Value::as_bool) == Some(true),
        }
    }
}

pub struct Env {
    pub sio: Sio,
    pub inputs: Inputs,
    pub power: Power,
    pub net: Net,
    pub gate: Gate,
    pub topics: Topics,
    pub helper_up: bool,
    /// The merged theme document (the owner's edits over the shipped file), as the shell last read or wrote it.
    pub theme: Value,
    pub fg: Foreground,
    pub capture: Capture,
    /// Whether the screen is on (a panel is not drawn for a screen that is off).
    pub screen_on: bool,
}

impl Env {
    pub fn new(theme: Value) -> Self {
        Self {
            sio: Sio::new(0x5300_0000),
            inputs: Inputs::new(),
            power: Power::new(),
            net: Net::new(),
            gate: Gate::new(),
            topics: Topics::new(),
            helper_up: false,
            theme,
            fg: Foreground::default(),
            capture: Capture::default(),
            screen_on: true,
        }
    }

    /// What goes out at the start: the network's subscription (the panel shows it from its first frame).
    pub fn start(&mut self) -> Vec<Effect> {
        Net::subscribe()
    }

    /// The owner's switch of a panel (`remote.quickPanel`, `remote.inputPicker`): on unless the theme says `false`.
    pub fn switch_on(&self, name: &str) -> bool {
        self.theme.pointer(&format!("/remote/{name}")).and_then(Value::as_bool) != Some(false)
    }

    /// The tile editor and the value cards are the owner's to switch on (`remote.quickPanelEdit`, off until the TV check).
    pub fn popups_on(&self) -> bool {
        self.theme.pointer("/remote/quickPanelEdit").and_then(Value::as_bool) == Some(true)
    }

    pub fn topic(&self, name: &str) -> Option<&Value> {
        self.topics.get(name)
    }

    /// Whether a surface that owns the remote is up that a panel must not open over: the AI prompt waiting for an answer.
    pub fn ai_prompt_up(&self) -> bool {
        self.topics.get("ai.prompt").is_some_and(|p| !p.is_null() && p.as_object().is_none_or(|o| !o.is_empty()))
    }

    /// Takes an input: the topics, the services' answers. Returns the effects the services ask for and whether anything a panel
    /// shows changed.
    pub fn update(&mut self, input: &Input, now: u64) -> (Vec<Effect>, bool) {
        let mut out = Vec::new();
        let mut changed = false;
        match input {
            Input::Topic { topic, data } => match topic.as_str() {
                "foreground" => {
                    let fg = Foreground { app: data.get("app").and_then(Value::as_str).unwrap_or("").to_string(), home: data.get("home").and_then(Value::as_bool).unwrap_or(false), known: true };
                    changed |= fg != self.fg;
                    self.fg = fg;
                }
                "screen" => {
                    let on = data.get("on").and_then(Value::as_bool).unwrap_or(true);
                    changed |= on != self.screen_on;
                    self.screen_on = on;
                }
                _ => {}
            },
            Input::Helper { topic, data } if topic == "up" => {
                let up = data.as_bool().unwrap_or(false);
                changed |= up != self.helper_up;
                self.helper_up = up;
            }
            Input::Helper { topic, data } => {
                if self.topics.get(topic) != Some(data) {
                    self.topics.insert(topic.clone(), data.clone());
                    changed = true;
                }
            }
            Input::Fetched { tag, status, data } if *tag == tags::CAPTURE_READ => {
                let cap = Capture::from_doc(*status, data);
                changed |= cap != self.capture;
                self.capture = cap;
            }
            Input::Fetched { tag, status, data } if *tag == tags::THEME && *status == 200 && self.theme != *data => {
                self.theme = data.clone();
                changed = true;
            }
            _ => {}
        }
        out.extend(self.sio.update(input, now));
        let (e, c) = self.inputs.update(input, now);
        out.extend(e);
        changed |= c;
        let (e, c) = self.power.update(input, now);
        out.extend(e);
        changed |= c;
        changed |= self.net.update(input);
        let (e, c) = self.gate.update(input, now);
        out.extend(e);
        changed |= c;
        // what a service answered that a panel shows
        if let Input::Luna { tag, .. } = input
            && (0x5300_0000..0x5400_0000).contains(tag)
        {
            changed = true;
        }
        if let Input::Helper { topic, .. } = input
            && topic == "up"
        {
            self.gate.helper(self.helper_up, now);
        }
        (out, changed)
    }

    pub fn due_at(&self) -> Option<u64> {
        [self.sio.due_at(), self.inputs.due_at(), self.power.due_at(), self.gate.due_at()].into_iter().flatten().min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_owners_switches_are_on_unless_the_theme_says_false_and_the_editor_is_off_until_it_says_true() {
        let mut e = Env::new(json!({}));
        assert!(e.switch_on("quickPanel") && e.switch_on("inputPicker"));
        assert!(!e.popups_on());
        e.theme = json!({"remote": {"quickPanel": false, "quickPanelEdit": true}});
        assert!(!e.switch_on("quickPanel") && e.switch_on("inputPicker"));
        assert!(e.popups_on());
    }

    #[test]
    fn the_compositors_topics_and_the_helpers_are_kept_and_a_repeat_changes_nothing() {
        let mut e = Env::new(json!({}));
        let (_, c) = e.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "netflix", "home": false}) }, 0);
        assert!(c && e.fg.app == "netflix" && !e.fg.home);
        let (_, c) = e.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "netflix", "home": false}) }, 1);
        assert!(!c);
        let (_, c) = e.update(&Input::Helper { topic: "ai".into(), data: json!({"listener": {"enabled": true}}) }, 2);
        assert!(c && e.topic("ai").is_some());
        let (_, c) = e.update(&Input::Helper { topic: "ai".into(), data: json!({"listener": {"enabled": true}}) }, 3);
        assert!(!c, "the same topic again is not news");
        e.update(&Input::Helper { topic: "ai.prompt".into(), data: Value::Null }, 4);
        assert!(!e.ai_prompt_up());
        e.update(&Input::Helper { topic: "ai.prompt".into(), data: json!({"id": "x"}) }, 5);
        assert!(e.ai_prompt_up());
    }

    #[test]
    fn capture_is_offered_only_while_the_helper_says_it_is_on() {
        assert_eq!(Capture::from_doc(500, &json!({})), Capture::default());
        assert!(!Capture::from_doc(200, &json!({"capture": {"enabled": false}})).enabled);
        let c = Capture::from_doc(200, &json!({"capture": {"enabled": true, "available": true, "recording": false, "encoder": {"available": true}}}));
        assert!(c.enabled && c.available && c.encoder && !c.recording);
    }

    #[test]
    fn the_helper_going_down_reaches_the_receivers_gate() {
        let mut e = Env::new(json!({}));
        e.update(&Input::Helper { topic: "up".into(), data: json!(true) }, 0);
        e.gate.set_active(true, 0);
        e.update(&Input::Helper { topic: "receiver".into(), data: json!({"reachable": true, "fresh": true, "updatedAt": 100, "volumeText": "-30 dB"}) }, 100);
        assert!(e.gate.doc().is_some());
        e.update(&Input::Helper { topic: "up".into(), data: json!(false) }, 200);
        assert!(e.gate.doc().is_none());
    }
}
