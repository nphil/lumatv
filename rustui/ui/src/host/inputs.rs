//! What the TV knows about its inputs, for the input picker and the quick panel (`kit/Inputs.qml` of the QML shell): each HDMI port's
//! name (the device names itself over HDMI-CEC) and whether anything is connected, from the external input manager
//! (`com.webos.service.eim/getAllInputStatus`), and which app is in front, which is the current input when its id is an input's: the
//! compositor's `foreground` topic says it, so no second subscription asks LG's app manager. The picker asks for a refresh when it
//! opens; the service is also asked every 30 s while a surface that shows inputs is up (`watch`).
//!
//! The device icon follows the name the device gave itself, then the port's own label; the owner's choice per input (Auto: only while
//! something is connected, Always, Hidden: theme `inputs.<port>`) decides which tiles are shown.

use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

pub const EIM: &str = "luna://com.webos.service.eim/getAllInputStatus";
/// The tags of the subscription: a refresh cancels the last and takes the next.
pub const TAG_BASE: u64 = 0x4900_0000;
pub const KEEP_MS: u64 = 30_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub app_id: String,
    pub port: String,
    pub label: String,
    pub name: String,
    pub connected: bool,
    pub icon: String,
}

/// A tile of the picker and of Home's Inputs row, one shape for both: Live TV first, then each port the owner has not hidden.
#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub app_id: String,
    pub key: String,
    pub port: String,
    pub icon: String,
    pub name: String,
    pub state: String,
    pub on: bool,
    pub connected: bool,
    pub front: bool,
    pub sound: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Inputs {
    list: Vec<Entry>,
    pub foreground: String,
    generation: u64,
    open_tag: Option<u64>,
    watchers: u32,
    keep_due: Option<u64>,
}

fn has_any(n: &str, words: &[&str]) -> bool {
    words.iter().any(|w| n.contains(w))
}

/// `set.?top`: "set", at most one character, "top".
fn set_top(n: &str) -> bool {
    n.match_indices("set").any(|(i, _)| {
        let rest = &n[i + 3..];
        rest.starts_with("top") || rest.chars().next().is_some_and(|c| rest[c.len_utf8()..].starts_with("top"))
    })
}

/// The icon for what is on the port: the device's own name first.
pub fn icon_for(name: &str, _port: &str) -> &'static str {
    let n = name.to_lowercase();
    if has_any(&n, &["playstation", "play station", "ps3", "ps4", "ps5", "xbox", "switch", "nintendo", "console", "steam deck"]) {
        "gamepad-2"
    } else if has_any(&n, &["apple tv", "chromecast", "firetv", "fire tv", "roku", "shield", "streamer", "nvidia", "google tv"]) {
        "tv-minimal-play"
    } else if has_any(&n, &["denon", "marantz", "yamaha", "onkyo", "receiver", "avr", "soundbar", "sonos", "bose", "speaker", "arc"]) {
        "speaker"
    } else if has_any(&n, &["bluray", "blu-ray", "dvd", "player", "disc", "magnavox"]) {
        "disc-3"
    } else if has_any(&n, &["pc", "computer", "laptop", "mac", "desktop", "monitor"]) {
        "monitor"
    } else if has_any(&n, &["antenna", "tuner", "cable box", "settop", "set top", "set-top"]) || set_top(&n) {
        "antenna"
    } else if n.is_empty() {
        "unplug"
    } else {
        "cable"
    }
}

/// What a tile is for: the receiver (the sound output when the sound goes out over eARC) is told from its name.
pub fn is_receiver(name: &str) -> bool {
    has_any(&name.to_lowercase(), &["denon", "marantz", "yamaha", "onkyo", "receiver", "avr"])
}

/// The owner's choice for an input (theme `inputs.<port>`: auto, always, hidden): Auto shows it only while something is connected.
pub fn visible_for(choices: &Value, key: &str, connected: bool) -> bool {
    let c = choices.get(key).and_then(Value::as_str).unwrap_or("auto");
    c == "always" || (c == "auto" && connected)
}

/// `HDMI 2` for `com.webos.app.hdmi2`, `Live TV` for the tuner, the id otherwise.
pub fn port_name(app_id: &str) -> String {
    if let Some(rest) = app_id.split("hdmi").nth(1)
        && let Some(d) = rest.chars().next().filter(char::is_ascii_digit)
    {
        return format!("HDMI {d}");
    }
    if app_id == "com.webos.app.livetv" { "Live TV".into() } else { app_id.to_string() }
}

impl Inputs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list(&self) -> &[Entry] {
        &self.list
    }

    pub fn connected(&self) -> Vec<&Entry> {
        self.list.iter().filter(|e| e.connected).collect()
    }

    /// The entry of the input in front, if the app in front is one.
    pub fn current(&self) -> Option<&Entry> {
        self.list.iter().find(|e| e.app_id == self.foreground)
    }

    /// The tiles of the picker and of Home's Inputs row; `choices` is the theme's `inputs`.
    pub fn tiles(&self, choices: &Value) -> Vec<Tile> {
        let mut out = Vec::new();
        if visible_for(choices, "livetv", true) {
            out.push(Tile {
                app_id: "com.webos.app.livetv".into(),
                key: "livetv".into(),
                port: "LIVE TV".into(),
                icon: "antenna".into(),
                name: "Antenna".into(),
                state: "Live TV".into(),
                on: true,
                connected: true,
                front: self.foreground == "com.webos.app.livetv",
                sound: false,
            });
        }
        for (i, e) in self.list.iter().enumerate() {
            let digit = e.app_id.split("hdmi").nth(1).and_then(|r| r.chars().next()).filter(char::is_ascii_digit).map_or_else(|| (i + 1).to_string(), |d| d.to_string());
            let key = format!("hdmi{digit}");
            if !visible_for(choices, &key, e.connected) {
                continue;
            }
            let front = e.app_id == self.foreground;
            let sound = e.connected && is_receiver(&e.label);
            out.push(Tile {
                app_id: e.app_id.clone(),
                key,
                port: format!("{}{}", e.port.to_uppercase(), if sound { " \u{b7} eARC" } else { "" }),
                icon: e.icon.clone(),
                name: if e.connected { if e.label.is_empty() { e.port.clone() } else { e.label.clone() } } else { "Nothing connected".into() },
                state: if !e.connected { "Plug in a device to use it" } else if front { "In front" } else { "Connected" }.into(),
                on: e.connected,
                connected: e.connected,
                front,
                sound,
            });
        }
        out
    }

    /// The inputs as the service lists them. A refresh usually repeats the subscription's last answer; replacing the list then would
    /// rebuild every tile, so an equal one is kept. Returns whether the list changed.
    pub fn parse(&mut self, doc: &Value) -> bool {
        if doc.get("returnValue").and_then(Value::as_bool) == Some(false) {
            return false;
        }
        let src = ["devices", "inputs", "deviceList"].iter().find_map(|k| doc.get(*k).and_then(Value::as_array)).cloned().unwrap_or_default();
        let mut out = Vec::new();
        for d in &src {
            let text = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let app_id = [text("appId"), text("id")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
            let label = [text("label"), text("deviceLabel"), text("name")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
            let port = port_name(&app_id);
            let up = d.get("connected").is_some_and(|v| v == &json!(true) || v == &json!("true")) || d.get("activate") == Some(&json!(true));
            let id = if text("id").is_empty() { app_id.clone() } else { text("id") };
            out.push(Entry { id, app_id, port: port.clone(), label: label.clone(), name: if up { label.clone() } else { String::new() }, connected: up, icon: icon_for(if up { &label } else { "" }, &port).to_string() });
        }
        if out == self.list {
            return false;
        }
        self.list = out;
        true
    }

    /// Asks for the list again: each ask is a new subscription, and the one before is let go first (LG's call stays open until it is
    /// cancelled, so every opening would otherwise leave one more call answering every input change).
    pub fn refresh(&mut self) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Some(tag) = self.open_tag.take() {
            out.push(Effect::Cancel { tag });
        }
        self.generation += 1;
        let tag = TAG_BASE + self.generation;
        self.open_tag = Some(tag);
        out.push(Effect::Luna { tag, uri: EIM.into(), payload: json!({"subscribe": true}), subscribe: true });
        out
    }

    /// A surface that shows inputs is up (`true`) or gone: while one is, the list is asked again every 30 s.
    pub fn watch(&mut self, on: bool, now: u64) {
        self.watchers = if on { self.watchers + 1 } else { self.watchers.saturating_sub(1) };
        self.keep_due = if self.watchers > 0 { Some(now + KEEP_MS) } else { None };
    }

    pub fn update(&mut self, input: &Input, now: u64) -> (Vec<Effect>, bool) {
        match input {
            Input::Luna { tag, payload } if self.open_tag == Some(*tag) => (Vec::new(), self.parse(payload)),
            Input::Topic { topic, data } if topic == "foreground" => {
                let app = data.get("app").and_then(Value::as_str).unwrap_or("").to_string();
                let changed = app != self.foreground;
                self.foreground = app;
                (Vec::new(), changed)
            }
            Input::Tick if self.keep_due.is_some_and(|d| now >= d) => {
                self.keep_due = if self.watchers > 0 { Some(now + KEEP_MS) } else { None };
                (self.refresh(), false)
            }
            _ => (Vec::new(), false),
        }
    }

    pub fn due_at(&self) -> Option<u64> {
        self.keep_due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tv() -> Inputs {
        let mut i = Inputs::new();
        i.parse(&json!({"devices": [
            {"appId": "com.webos.app.hdmi1", "label": "Apple TV 4K", "connected": true},
            {"appId": "com.webos.app.hdmi2", "label": "Denon AVR-S760H", "connected": true},
            {"appId": "com.webos.app.hdmi3", "label": "PlayStation 5", "connected": true},
            {"appId": "com.webos.app.hdmi4", "label": "", "connected": false},
        ]}));
        i.foreground = "com.webos.app.hdmi2".into();
        i
    }

    #[test]
    fn the_icon_is_the_devices_own_name_first() {
        for (name, icon) in [
            ("PlayStation 5", "gamepad-2"),
            ("Apple TV 4K", "tv-minimal-play"),
            ("Denon AVR-S760H", "speaker"),
            ("Panasonic Blu-ray", "disc-3"),
            ("Gaming PC", "monitor"),
            ("Cable Box", "antenna"),
            ("Set-top", "antenna"),
            ("Settop", "antenna"),
            ("Something", "cable"),
            ("", "unplug"),
        ] {
            assert_eq!(icon_for(name, "HDMI 1"), icon, "{name}");
        }
    }

    #[test]
    fn auto_shows_live_tv_and_the_connected_ports_and_names_the_receiver_and_its_sound() {
        let tiles = tv().tiles(&json!({}));
        let row = |t: &Tile| (t.port.clone(), t.icon.clone(), t.name.clone(), t.connected, t.front, t.sound);
        assert_eq!(
            tiles.iter().map(row).collect::<Vec<_>>(),
            vec![
                ("LIVE TV".into(), "antenna".into(), "Antenna".into(), true, false, false),
                ("HDMI 1".into(), "tv-minimal-play".into(), "Apple TV 4K".into(), true, false, false),
                ("HDMI 2 \u{b7} eARC".into(), "speaker".into(), "Denon AVR-S760H".into(), true, true, true),
                ("HDMI 3".into(), "gamepad-2".into(), "PlayStation 5".into(), true, false, false),
            ],
            "the empty HDMI 4 is not there"
        );
        assert_eq!(tiles[2].state, "In front");
        assert_eq!(tiles[1].state, "Connected");
    }

    #[test]
    fn always_shows_an_empty_port_and_hidden_removes_one() {
        let tiles = tv().tiles(&json!({"hdmi4": "always", "hdmi3": "hidden", "livetv": "hidden"}));
        assert_eq!(tiles.iter().map(|t| t.port.as_str()).collect::<Vec<_>>(), vec!["HDMI 1", "HDMI 2 \u{b7} eARC", "HDMI 4"]);
        let empty = &tiles[2];
        assert_eq!((empty.icon.as_str(), empty.name.as_str(), empty.connected, empty.state.as_str()), ("unplug", "Nothing connected", false, "Plug in a device to use it"));
    }

    #[test]
    fn a_repeated_answer_keeps_the_list_and_a_real_change_replaces_it() {
        let mut i = Inputs::new();
        let doc = json!({"devices": [{"appId": "com.webos.app.hdmi1", "label": "Apple TV", "connected": true}]});
        assert!(i.parse(&doc));
        assert!(!i.parse(&doc), "the same answer changes nothing");
        let renamed = json!({"devices": [{"appId": "com.webos.app.hdmi1", "label": "PlayStation", "connected": true}]});
        assert!(i.parse(&renamed));
        assert_eq!(i.list()[0].icon, "gamepad-2");
        let gone = json!({"devices": [{"appId": "com.webos.app.hdmi1", "label": "PlayStation", "connected": false}]});
        assert!(i.parse(&gone));
        assert_eq!(i.list()[0].icon, "unplug");
    }

    #[test]
    fn each_refresh_lets_the_last_subscription_go() {
        let mut i = Inputs::new();
        let first = i.refresh();
        assert!(matches!(first.as_slice(), [Effect::Luna { subscribe: true, .. }]));
        let second = i.refresh();
        let [Effect::Cancel { tag: old }, Effect::Luna { tag: new, .. }] = second.as_slice() else { panic!("{second:?}") };
        assert_ne!(old, new);
        // an answer of the old subscription is not taken
        let stale = Input::Luna { tag: *old, payload: json!({"devices": [{"appId": "com.webos.app.hdmi1", "label": "X", "connected": true}]}) };
        assert_eq!(i.update(&stale, 0), (Vec::new(), false));
        assert!(i.list().is_empty());
        let fresh = Input::Luna { tag: *new, payload: json!({"devices": [{"appId": "com.webos.app.hdmi1", "label": "X", "connected": true}]}) };
        assert_eq!(i.update(&fresh, 0), (Vec::new(), true));
    }

    #[test]
    fn the_front_app_is_the_compositors_and_the_list_is_asked_again_every_thirty_seconds_while_watched() {
        let mut i = tv();
        i.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.hdmi3", "home": false}) }, 0);
        assert_eq!(i.current().map(|e| e.port.as_str()), Some("HDMI 3"));
        assert!(i.tiles(&json!({}))[3].front);
        i.watch(true, 1000);
        assert_eq!(i.due_at(), Some(31_000));
        assert!(i.update(&Input::Tick, 30_999).0.is_empty());
        assert_eq!(i.update(&Input::Tick, 31_000).0.len(), 1);
        i.watch(false, 40_000);
        assert_eq!(i.due_at(), None);
    }
}
