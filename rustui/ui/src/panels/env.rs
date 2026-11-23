//! What the panels know of the TV: the services' models (the settings service, the inputs, the sleep timer, the network, the
//! receiver's gate), the helper's topics, the owner's theme, who is in front. One value the panels' models read and, to write a
//! setting or ask a service, change (each change answers with the effects it needs). The host feeds it every input, so what the
//! panels show is current whether they are open or not; the services that are costly (the inputs' subscription, the settings
//! reads) are only asked for by a panel that is coming up.

use super::hub::Topics;
use crate::host::inputs::{Inputs, Switch};
use crate::host::net::Net;
use crate::host::power::Power;
use crate::host::previews::{Facts, Previews};
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
    pub const DND: u64 = 0x5100_000C;
}

/// What the compositor says is in front.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Foreground {
    pub app: String,
    pub home: bool,
    pub known: bool,
    pub video: Option<(String, String)>,
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

/// A device chosen behind the receiver while its port was not in front. Launching the port lets the device that answers HDMI-CEC take
/// the receiver: on the set (2026-11-22) a set sent with the launch of HDMI 2 for the Blu-ray input ended on Apple TV's. So the receiver is
/// set once the port is in front, and again when it moves off that input within `FOLLOW_HOLD_MS`.
#[derive(Clone, Debug, PartialEq)]
pub struct Follow {
    pub app: String,
    pub code: String,
    pub asked_at: u64,
    pub front_at: Option<u64>,
    pub sets: u8,
    pub last_set: Option<u64>,
}

/// How long a chosen device waits for its port to come to the front.
pub const FOLLOW_WAIT_MS: u64 = 15_000;
/// How long after the port came to the front the receiver is put back on the chosen input, how often at most, and how far apart (the
/// receiver's report of a set comes within about a second).
pub const FOLLOW_HOLD_MS: u64 = 8_000;
pub const FOLLOW_SETS: u8 = 3;
pub const FOLLOW_GAP_MS: u64 = 1_500;

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
    /// Where the sound is going out now, as the audio service pushes it with each volume state (`volumeStatus.soundOutput`: the way out in
    /// effect, which the setting `sound.soundOutput` can lag behind or name otherwise); "" until the compositor has said.
    pub audio_output: String,
    /// The pictures of the inputs the picker shows (`host/previews.rs`): fed every input, asked for only while the picker is ours.
    pub previews: Previews,
    /// Nothing else of the shell is on the screen (the shell says so before a tick): the previews' quiet timers wait for it.
    pub quiet: bool,
    /// The receiver input to set once its port is in front (`follow_switch`).
    pub follow: Option<Follow>,
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
            audio_output: String::new(),
            previews: Previews::new(),
            quiet: true,
            follow: None,
        }
    }

    /// The receiver is to be set to `code` once `app` is in front (the picker launched it).
    pub fn follow_switch(&mut self, app: String, code: String, now: u64) {
        self.follow = Some(Follow { app, code, asked_at: now, front_at: None, sets: 0, last_set: None });
    }

    /// What the pending receiver switch asks for now: a set when the port is in front and the receiver is not on the chosen input.
    fn follow_effects(&mut self, now: u64) -> Vec<Effect> {
        let Some(f) = self.follow.as_ref() else { return Vec::new() };
        let front = self.fg.known && self.fg.app == f.app;
        let expired = match f.front_at {
            Some(at) => now.saturating_sub(at) > FOLLOW_HOLD_MS || f.sets >= FOLLOW_SETS || !front,
            None => now.saturating_sub(f.asked_at) > FOLLOW_WAIT_MS,
        };
        if expired {
            self.follow = None;
            return Vec::new();
        }
        if !front {
            return Vec::new();
        }
        // the picker has closed, so its gate is off: the helper's own word on the reading stands in for the gate's clock
        let switch = self.gate.rc().filter(|r| ["reachable", "fresh"].iter().all(|k| r.get(*k).and_then(Value::as_bool) == Some(true)));
        let Some(switch) = switch.filter(|_| self.denon_on()).and_then(Switch::from_doc).filter(|s| s.writable) else { return Vec::new() };
        let f = self.follow.as_mut().expect("checked above");
        f.front_at.get_or_insert(now);
        if switch.current.as_deref() == Some(f.code.as_str()) || f.last_set.is_some_and(|at| now.saturating_sub(at) < FOLLOW_GAP_MS) {
            return Vec::new();
        }
        f.sets += 1;
        f.last_set = Some(now);
        vec![Effect::Post { path: "/v1/receiver/set".into(), body: serde_json::json!({"setting": "input", "value": f.code, "session": switch.session}) }]
    }

    /// What goes out at the start: the network's subscription (the panel shows it from its first frame).
    pub fn start(&mut self) -> Vec<Effect> {
        Net::subscribe()
    }

    /// The owner's switch of a panel (`remote.quickPanel`, `remote.inputPicker`): on unless the theme says `false`.
    pub fn switch_on(&self, name: &str) -> bool {
        self.theme.pointer(&format!("/remote/{name}")).and_then(Value::as_bool) != Some(false)
    }

    /// A switch that is off until the owner turns it on (`remote.appSwitcher`, `remote.recents`: off until the TV check).
    pub fn switch_set(&self, name: &str) -> bool {
        self.theme.pointer(&format!("/remote/{name}")).and_then(Value::as_bool) == Some(true)
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

    /// The Denon receiver plugin is switched on (an entry of `plugins` that does not say `off`; none from a helper that is not there).
    pub fn denon_on(&self) -> bool {
        self.topics.get("plugins").and_then(|p| super::hub::plugin_of(p, "denon-receiver")).and_then(|e| e.get("state")).and_then(Value::as_str).is_some_and(|s| s != "off")
    }

    /// What the receiver says of its inputs, for the devices behind it: only while the plugin is on and the receiver's reading is in date.
    pub fn denon_switch(&self) -> Option<Switch> {
        if !self.denon_on() {
            return None;
        }
        Switch::from_doc(self.gate.doc()?)
    }

    /// Takes an input: the topics, the services' answers. Returns the effects the services ask for and whether anything a panel
    /// shows changed.
    pub fn update(&mut self, input: &Input, now: u64) -> (Vec<Effect>, bool) {
        let mut out = Vec::new();
        let mut changed = false;
        match input {
            Input::Topic { topic, data } => match topic.as_str() {
                "foreground" => {
                    let app = data.get("app").and_then(Value::as_str).unwrap_or("");
                    let home = data.get("home").and_then(Value::as_bool).unwrap_or(false);
                    let video = data.get("video").and_then(|v| Some((v.get("context")?.as_str()?, v.get("app")?.as_str()?)))
                        .filter(|(context, source_app)| !home && !context.is_empty() && *source_app == app && !app.is_empty())
                        .map(|(context, app)| (context.to_owned(), app.to_owned()));
                    let fg = Foreground { app: app.to_string(), home, known: true, video };
                    changed |= fg != self.fg;
                    self.fg = fg;
                }
                "screen" => {
                    let on = data.get("on").and_then(Value::as_bool).unwrap_or(true);
                    changed |= on != self.screen_on;
                    self.screen_on = on;
                }
                "audio" => {
                    if let Some(out) = data.pointer("/volumeStatus/soundOutput").and_then(Value::as_str).filter(|o| !o.is_empty() && *o != self.audio_output) {
                        self.audio_output = out.to_string();
                        changed = true;
                    }
                }
                "channel.up" if data.as_bool() == Some(false) => {
                    changed |= self.fg.video.take().is_some();
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
        let sio_revision = self.sio.revision;
        out.extend(self.sio.update(input, now));
        changed |= self.sio.revision != sio_revision;
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
        // the pictures of the inputs: what they ask for, and whether a tile's picture is another now
        let (e, c) = {
            let Self { previews, fg, theme, helper_up, screen_on, quiet, .. } = self;
            let on = theme.pointer("/inputs/previews").and_then(Value::as_bool) != Some(false);
            previews.update(input, &Facts { on, helper_up: *helper_up, screen_on: *screen_on, app: &fg.app, home: fg.home, known: fg.known }, *quiet, now)
        };
        out.extend(e);
        changed |= c;
        if let Input::Helper { topic, .. } = input
            && topic == "up"
        {
            self.gate.helper(self.helper_up, now);
        }
        out.extend(self.follow_effects(now));
        (out, changed)
    }

    /// The picker came up: the pictures of the inputs are asked for (the index, and one of the input in front at once).
    pub fn previews_opened(&mut self, now: u64) -> Vec<Effect> {
        let Self { previews, fg, theme, helper_up, screen_on, .. } = self;
        let on = theme.pointer("/inputs/previews").and_then(Value::as_bool) != Some(false);
        previews.opened(&Facts { on, helper_up: *helper_up, screen_on: *screen_on, app: &fg.app, home: fg.home, known: fg.known }, now)
    }

    /// The picker went away.
    pub fn previews_closed(&mut self, now: u64) {
        self.previews.closed(now);
    }

    pub fn due_at(&self) -> Option<u64> {
        // once the port is in front, a receiver report is what calls for another set; the timer only ends the hold
        let follow = self.follow.as_ref().map(|f| f.front_at.map_or(f.asked_at + FOLLOW_WAIT_MS + 1, |at| at + FOLLOW_HOLD_MS + 1));
        [self.sio.due_at(), self.inputs.due_at(), self.power.due_at(), self.gate.due_at(), self.previews.due_at(), follow].into_iter().flatten().min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_timed_out_picture_read_redraws_the_unknown_mode() {
        let mut env = Env::new(json!({}));
        env.sio.learn("picture", &json!({"settings":{"pictureMode":"eco"}, "dimension":{"pictureMode":"eco", "dynamicRange":"sdr", "input":"hdmi1"}}));
        let write = env.sio.write("picture.pictureMode", &json!("filmMaker"), None, 100);
        let Effect::Luna { tag, .. } = &write[0] else { panic!() };
        env.update(&Input::Luna {tag:*tag, payload:json!({"returnValue":true})}, 101);
        assert_eq!(env.sio.text("picture.pictureMode"), "filmMaker");
        let (_, changed) = env.update(&Input::Tick, 2101);
        assert!(changed);
        assert!(!env.sio.has("picture.pictureMode"));
    }

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

    #[test]
    fn a_device_chosen_with_its_port_away_is_set_once_the_port_is_in_front_and_again_when_cec_moves_the_receiver() {
        let at = 1_000_000u64;
        let rx = |t: u64, input: &str| {
            json!({"enabled": true, "fresh": true, "reachable": true, "updatedAt": t, "settingsSession": "s1", "input": input,
                   "settings": {"input": {"support": "supported", "writable": true, "value": input, "options": [{"value": "MPLAY", "label": "Apple TV"}, {"value": "BD", "label": "MAGNAVOX 4K BD"}]}}})
        };
        let posts = |fx: &[Effect]| fx.iter().filter(|f| matches!(f, Effect::Post { path, body } if path == "/v1/receiver/set" && body["value"] == "BD")).count();
        let mut e = Env::new(json!({}));
        e.update(&Input::Helper { topic: "up".into(), data: json!(true) }, at);
        e.update(&Input::Helper { topic: "plugins".into(), data: json!({"plugins": [], "firstParty": [{"id": "denon-receiver", "state": "on"}]}) }, at);
        e.gate.set_active(true, at);
        e.update(&Input::Helper { topic: "receiver".into(), data: rx(at, "TV") }, at);
        e.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true}) }, at);
        e.follow_switch("com.webos.app.hdmi2".into(), "BD".into(), at);
        e.gate.set_active(false, at); // the picker closed with the OK
        assert!(e.denon_switch().is_none(), "the gate is off once the picker has gone");
        assert_eq!(posts(&e.update(&Input::Tick, at + 100).0), 0, "not before the port is in front");
        assert_eq!(posts(&e.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.hdmi2", "home": false}) }, at + 900).0), 1);
        // HDMI-CEC hands the receiver to the device that answers it
        assert_eq!(posts(&e.update(&Input::Helper { topic: "receiver".into(), data: rx(at + 1200, "MPLAY") }, at + 1200).0), 0, "within the gap");
        assert_eq!(posts(&e.update(&Input::Helper { topic: "receiver".into(), data: rx(at + 2600, "MPLAY") }, at + 2600).0), 1, "set again");
        assert_eq!(posts(&e.update(&Input::Helper { topic: "receiver".into(), data: rx(at + 4500, "BD") }, at + 4500).0), 0);
        e.update(&Input::Tick, at + 900 + FOLLOW_HOLD_MS + 1);
        assert!(e.follow.is_none(), "the hold ends");
        e.follow_switch("com.webos.app.hdmi2".into(), "BD".into(), at + 20_000);
        e.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "netflix", "home": false}) }, at + 20_100);
        e.update(&Input::Tick, at + 20_000 + FOLLOW_WAIT_MS + 1);
        assert!(e.follow.is_none(), "a port that never came to the front is let go");
    }
}
