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
    /// The name the audio system on this port gave itself over HDMI-CEC (`subList` of the input's status, `cecpDevType` 5: a receiver or
    /// a soundbar), with its maker in front when the name does not have it; "" when there is none.
    pub receiver: String,
    /// The names of the devices behind that audio system (`subList` entries at `depth` 2: what is plugged into the receiver), in the order of
    /// their HDMI-CEC physical addresses; the receiver itself and entries with no name are not in it.
    pub devices: Vec<String>,
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
    /// The devices behind the receiver on this port, with the receiver's input each one is on.
    pub children: Vec<Child>,
}

/// A device behind the receiver, under its port's tile (HDMI 2, AVR-S760H, Apple TV).
#[derive(Clone, Debug, PartialEq)]
pub struct Child {
    pub name: String,
    pub icon: String,
    /// The receiver input (`SI` code) this device is on, when the Denon receiver plugin is on and names one like the device's own name; a
    /// device with none has no switch.
    pub source: Option<String>,
    /// This is the device the receiver is playing now (its input is the receiver's current one, and this port is in front).
    pub active: bool,
    /// OK on it switches the receiver: it has an input and the helper takes a change of the receiver's input now.
    pub can_switch: bool,
}

/// What the Denon receiver plugin says of the receiver's inputs (the `receiver` topic's `settings.input`), for the devices behind it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Switch {
    /// `(code, name)` of each input, the name as the owner has renamed it on the receiver.
    pub sources: Vec<(String, String)>,
    /// The receiver's current input code.
    pub current: Option<String>,
    /// The helper takes an input change now, and the session to ask it with.
    pub writable: bool,
    pub session: String,
}

impl Switch {
    /// From the helper's `receiver` document; none while it offers no input choice.
    pub fn from_doc(rx: &Value) -> Option<Self> {
        if rx.get("enabled").and_then(Value::as_bool) == Some(false) {
            return None;
        }
        let input = rx.pointer("/settings/input")?;
        if input.get("support").and_then(Value::as_str) != Some("supported") {
            return None;
        }
        let sources: Vec<(String, String)> = input
            .get("options")?
            .as_array()?
            .iter()
            .filter_map(|o| Some((o.get("value")?.as_str()?.to_string(), o.get("label").and_then(Value::as_str).unwrap_or_default().to_string())))
            .collect();
        if sources.is_empty() {
            return None;
        }
        let session = rx.get("settingsSession").and_then(Value::as_str).unwrap_or_default().to_string();
        Some(Self {
            sources,
            current: input.get("value").and_then(Value::as_str).map(str::to_string),
            writable: input.get("writable").and_then(Value::as_bool) == Some(true) && !session.is_empty(),
            session,
        })
    }
}

/// A name as it is compared: its letters and digits in lower case, so "Apple TV" and "apple-tv" are one.
fn plain(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A name may match another by being part of it only from this many letters: the receiver's "TV" (and "CD") is in "Apple TV".
const MIN_PART: usize = 3;

/// The receiver input a CEC device is on, by its name: an input named the same wins, else the one input whose name has the device's, or
/// the device's name has its (spaces and case aside). Two inputs that fit equally are none: a wrong switch is worse than none.
pub fn source_for(device: &str, sources: &[(String, String)]) -> Option<String> {
    let d = plain(device);
    if d.is_empty() {
        return None;
    }
    let named: Vec<(&String, String)> = sources.iter().map(|(code, name)| (code, plain(name))).filter(|(_, n)| !n.is_empty()).collect();
    let only = |hits: Vec<&String>| if let [one] = hits[..] { Some(one.clone()) } else { None };
    let exact: Vec<&String> = named.iter().filter(|(_, n)| *n == d).map(|(c, _)| *c).collect();
    if !exact.is_empty() {
        return only(exact);
    }
    let part = |a: &str, b: &str| a.len().min(b.len()) >= MIN_PART && (a.contains(b) || b.contains(a));
    only(named.iter().filter(|(_, n)| part(n, &d)).map(|(c, _)| *c).collect())
}

/// The one device the receiver's current input is (the codes compared without case); none when no device is on it or two are.
pub fn active_index(sources: &[Option<String>], current: &str) -> Option<usize> {
    let mut on = sources.iter().enumerate().filter(|(_, s)| s.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(current)));
    let first = on.next()?.0;
    on.next().is_none().then_some(first)
}

/// The devices behind a receiver as children of its tile. Without the plugin's `switch` they only have names.
fn children(devices: &[String], switch: Option<&Switch>, front: bool) -> Vec<Child> {
    let sources: Vec<Option<String>> = devices.iter().map(|d| switch.and_then(|s| source_for(d, &s.sources))).collect();
    let active = switch.and_then(|s| s.current.as_deref()).filter(|_| front).and_then(|c| active_index(&sources, c));
    devices
        .iter()
        .zip(sources)
        .enumerate()
        .map(|(i, (name, source))| Child {
            name: name.clone(),
            icon: icon_for(name, "").to_string(),
            can_switch: source.is_some() && switch.is_some_and(|s| s.writable),
            source,
            active: active == Some(i),
        })
        .collect()
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

/// The CEC vendor ids of the makers whose receivers name themselves without the maker (the IEEE number of the maker, as the status gives it).
const VENDORS: &[(u64, &str)] = &[(1485, "Denon")];

/// The name of the audio system among the CEC devices behind an input (`subList`: `osdName`, `cecpDevType` 5 for an audio system), the maker
/// put in front when the status gives its id and the name does not already have it.
fn cec_audio_system(input: &Value) -> String {
    let Some(sub) = input.get("subList").and_then(Value::as_array) else { return String::new() };
    let number = |v: &Value, k: &str| v.get(k).and_then(|n| n.as_u64().or_else(|| n.as_str().and_then(|s| s.parse().ok())));
    let Some(dev) = sub.iter().find(|d| number(d, "cecpDevType") == Some(5) && d.get("osdName").and_then(Value::as_str).is_some_and(|n| !n.trim().is_empty())) else {
        return String::new();
    };
    let name = dev.get("osdName").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let maker = number(dev, "vendor").or_else(|| number(dev, "vendorId")).and_then(|id| VENDORS.iter().find(|(v, _)| *v == id)).map(|(_, n)| *n);
    match maker {
        Some(m) if !name.to_lowercase().contains(&m.to_lowercase()) => format!("{m} {name}"),
        _ => name,
    }
}

/// The names of the devices plugged in behind the first one on an input (`subList` entries at `depth` 2 or more, named, not themselves an
/// audio system), by CEC physical address. An entry that does not say its depth is not taken for one.
fn cec_devices_behind(input: &Value) -> Vec<String> {
    let Some(sub) = input.get("subList").and_then(Value::as_array) else { return Vec::new() };
    let number = |v: &Value, k: &str| v.get(k).and_then(|n| n.as_u64().or_else(|| n.as_str().and_then(|s| s.parse().ok())));
    let mut found: Vec<(u64, String)> = sub
        .iter()
        .filter(|d| number(d, "depth").is_some_and(|n| n >= 2) && number(d, "cecpDevType") != Some(5))
        .filter_map(|d| {
            let name = d.get("osdName").and_then(Value::as_str)?.trim();
            (!name.is_empty()).then(|| (number(d, "physicalAddress").unwrap_or(u64::MAX), name.to_string()))
        })
        .collect();
    found.sort_by_key(|(address, _)| *address);
    found.dedup_by(|a, b| a.1 == b.1);
    found.into_iter().map(|(_, name)| name).collect()
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

    /// The audio system on a connected port, by the name it gave itself over HDMI-CEC; failing that, a device whose own name is a receiver's.
    pub fn receiver_name(&self) -> Option<String> {
        let up = self.list.iter().filter(|e| e.connected);
        up.clone().find(|e| !e.receiver.is_empty()).map(|e| e.receiver.clone()).or_else(|| up.clone().find(|e| is_receiver(&e.label)).map(|e| e.label.clone()))
    }

    /// The tiles of the picker; `choices` is the theme's `inputs`. `arc`: the sound goes out over HDMI to the audio system, so its port is the
    /// one the sound goes out of. `switch`: the Denon receiver plugin's word on the receiver's inputs, which only a plugin that is on gives;
    /// without it the devices behind the receiver are named and nothing more.
    pub fn tiles(&self, choices: &Value, arc: bool, switch: Option<&Switch>) -> Vec<Tile> {
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
                children: Vec::new(),
            });
        }
        for (i, e) in self.list.iter().enumerate() {
            let digit = e
                .app_id
                .split("hdmi")
                .nth(1)
                .and_then(|r| r.chars().next())
                .filter(char::is_ascii_digit)
                .map_or_else(|| (i + 1).to_string(), |d| d.to_string());
            let key = format!("hdmi{digit}");
            if !visible_for(choices, &key, e.connected) {
                continue;
            }
            let front = e.app_id == self.foreground;
            let sound = arc && e.connected && (!e.receiver.is_empty() || is_receiver(&e.label));
            out.push(Tile {
                app_id: e.app_id.clone(),
                key,
                port: format!("{}{}", e.port.to_uppercase(), if sound { " \u{b7} eARC" } else { "" }),
                icon: e.icon.clone(),
                name: if e.connected { if e.label.is_empty() { e.port.clone() } else { e.label.clone() } } else { "Nothing connected".into() },
                state: if !e.connected {
                    "Plug in a device to use it"
                } else if front {
                    "In front"
                } else {
                    "Connected"
                }
                .into(),
                on: e.connected,
                connected: e.connected,
                front,
                sound,
                children: if e.connected { children(&e.devices, switch, front) } else { Vec::new() },
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
            let receiver = if up { cec_audio_system(d) } else { String::new() };
            let devices = if up { cec_devices_behind(d) } else { Vec::new() };
            out.push(Entry {
                id,
                app_id,
                port: port.clone(),
                label: label.clone(),
                name: if up { label.clone() } else { String::new() },
                connected: up,
                icon: icon_for(if up { &label } else { "" }, &port).to_string(),
                receiver,
                devices,
            });
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
        let tiles = tv().tiles(&json!({}), true, None);
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
        let tiles = tv().tiles(&json!({"hdmi4": "always", "hdmi3": "hidden", "livetv": "hidden"}), true, None);
        assert_eq!(tiles.iter().map(|t| t.port.as_str()).collect::<Vec<_>>(), vec!["HDMI 1", "HDMI 2 \u{b7} eARC", "HDMI 4"]);
        let empty = &tiles[2];
        assert_eq!(
            (empty.icon.as_str(), empty.name.as_str(), empty.connected, empty.state.as_str()),
            ("unplug", "Nothing connected", false, "Plug in a device to use it")
        );
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
        assert!(i.tiles(&json!({}), true, None)[3].front);
        i.watch(true, 1000);
        assert_eq!(i.due_at(), Some(31_000));
        assert!(i.update(&Input::Tick, 30_999).0.is_empty());
        assert_eq!(i.update(&Input::Tick, 31_000).0.len(), 1);
        i.watch(false, 40_000);
        assert_eq!(i.due_at(), None);
    }
    #[test]
    fn the_audio_system_behind_a_port_is_named_by_what_it_says_over_cec_with_its_maker_in_front() {
        let mut i = Inputs::new();
        // the status of this TV: the receiver and the player behind it are CEC devices of the one port
        i.parse(&json!({"devices": [
            {"appId": "com.webos.app.hdmi2", "label": "HDMI 2", "connected": true, "subList": [
                {"id": "SIMPLINK", "portId": 2, "osdName": "Apple TV", "cecpDevType": 4, "vendor": 4346},
                {"id": "SIMPLINK", "portId": 2, "osdName": "AVR-S760H", "cecpDevType": 5, "vendor": 1485},
            ]},
            {"appId": "com.webos.app.hdmi1", "label": "HDMI 1", "connected": false, "subList": [{"osdName": "Ghost", "cecpDevType": 5}]},
        ]}));
        assert_eq!(i.receiver_name().as_deref(), Some("Denon AVR-S760H"));
        i.foreground = "com.webos.app.hdmi2".into();
        let on = i.tiles(&json!({}), true, None);
        let hdmi2 = on.iter().find(|t| t.app_id.ends_with("hdmi2")).unwrap();
        assert!(hdmi2.sound && hdmi2.port == "HDMI 2 \u{b7} eARC", "its port is the one the sound goes out of: {hdmi2:?}");
        let off = i.tiles(&json!({}), false, None);
        assert!(off.iter().all(|t| !t.sound && !t.port.contains("eARC")), "the sound does not go there while the output is elsewhere");
        // a name that already has the maker, and a number given as text
        i.parse(&json!({"devices": [{"appId": "com.webos.app.hdmi3", "label": "x", "connected": true, "subList": [{"osdName": "Denon AVR", "cecpDevType": "5", "vendorId": "1485"}]}]}));
        assert_eq!(i.receiver_name().as_deref(), Some("Denon AVR"));
        // no CEC audio system: a device whose own name is a receiver's still is one
        i.parse(&json!({"devices": [{"appId": "com.webos.app.hdmi1", "label": "Marantz NR1200", "connected": true}]}));
        assert_eq!(i.receiver_name().as_deref(), Some("Marantz NR1200"));
    }

    /// This TV's status of HDMI 2 (`docs/research/arch/x4/all.jsonl`): the player is at depth 2 on the receiver's input 2, the receiver at depth 1.
    fn behind_the_receiver() -> Value {
        json!({"devices": [{"appId": "com.webos.app.hdmi2", "label": "HDMI 2", "connected": true, "subList": [
            {"cecpDevType": 4, "depth": 2, "vendorId": 4346, "physicalAddress": 8704, "osdName": "Apple TV"},
            {"cecpDevType": 4, "depth": 2, "physicalAddress": 8448, "osdName": "  "},
            {"cecpDevType": 4, "depth": 2, "physicalAddress": 8960, "osdName": "PlayStation 5"},
            {"cecpDevType": 4, "physicalAddress": 12288, "osdName": "No depth"},
            {"cecpDevType": 5, "depth": 1, "vendorId": 1485, "physicalAddress": 8192, "osdName": "AVR-S760H"},
        ]}]})
    }

    #[test]
    fn the_devices_behind_the_receiver_are_its_children_by_address_without_the_receiver_or_a_nameless_one() {
        let mut i = Inputs::new();
        i.parse(&behind_the_receiver());
        assert_eq!(i.list()[0].devices, vec!["Apple TV".to_string(), "PlayStation 5".to_string()]);
        assert_eq!(i.list()[0].receiver, "Denon AVR-S760H", "the receiver is still named by its own entry");
        let tiles = i.tiles(&json!({}), true, None);
        let hdmi2 = tiles.iter().find(|t| t.key == "hdmi2").unwrap();
        let named: Vec<_> = hdmi2.children.iter().map(|c| (c.name.as_str(), c.source.is_some(), c.active)).collect();
        assert_eq!(named, vec![("Apple TV", false, false), ("PlayStation 5", false, false)], "no plugin: names only, no switch and no mark");
        assert!(tiles.iter().filter(|t| t.key != "hdmi2").all(|t| t.children.is_empty()));
    }

    fn sources(names: &[(&str, &str)]) -> Vec<(String, String)> {
        names.iter().map(|(c, n)| ((*c).to_string(), (*n).to_string())).collect()
    }

    #[test]
    fn a_device_is_on_the_receiver_input_that_has_its_name_and_never_a_guessed_one() {
        let s = sources(&[("TV", "TV Audio"), ("MPLAY", "Apple TV"), ("GAME", "PS5 Game"), ("BD", "Blu-ray"), ("CD", "CD")]);
        for (device, code) in [
            ("Apple TV", Some("MPLAY")),
            ("apple-tv", Some("MPLAY")),
            ("Apple TV 4K", Some("MPLAY")),
            ("Blu-ray Player", Some("BD")),
            ("PlayStation 5", None),
            ("", None),
        ] {
            assert_eq!(source_for(device, &s).as_deref(), code, "{device}");
        }
        // the receiver's short names are in too many others to count as a part
        assert_eq!(source_for("Apple TV", &sources(&[("TV", "TV")])), None);
        assert_eq!(source_for("CD Changer", &sources(&[("CD", "CD")])), None);
        // an exact name wins over one that only has it; two that fit equally are none
        assert_eq!(source_for("Apple TV", &sources(&[("GAME", "Apple TV 4K"), ("MPLAY", "Apple TV")])).as_deref(), Some("MPLAY"));
        assert_eq!(source_for("Apple TV", &sources(&[("GAME", "Apple TV 4K"), ("AUX1", "Apple TV HD")])), None);
    }

    #[test]
    fn the_active_device_is_the_one_on_the_receivers_current_input_only_while_its_port_is_in_front() {
        let known = [Some("MPLAY".to_string()), None, Some("GAME".to_string())];
        assert_eq!(active_index(&known, "mplay"), Some(0));
        assert_eq!(active_index(&known, "GAME"), Some(2));
        assert_eq!(active_index(&known, "TV"), None, "the receiver is on something that is not behind it");
        assert_eq!(active_index(&[Some("GAME".into()), Some("GAME".into())], "GAME"), None, "two on one input: none is marked");

        let mut i = Inputs::new();
        i.parse(&behind_the_receiver());
        let switch = Switch { sources: sources(&[("MPLAY", "Apple TV"), ("GAME", "PlayStation 5")]), current: Some("GAME".into()), writable: true, session: "s".into() };
        let marked = |i: &Inputs| {
            let tiles = i.tiles(&json!({}), true, Some(&switch));
            let hdmi2 = tiles.into_iter().find(|t| t.key == "hdmi2").unwrap();
            hdmi2.children.iter().map(|c| (c.name.clone(), c.source.clone(), c.active)).collect::<Vec<_>>()
        };
        assert_eq!(marked(&i), vec![("Apple TV".into(), Some("MPLAY".into()), false), ("PlayStation 5".into(), Some("GAME".into()), false)], "HDMI 2 is not in front");
        i.foreground = "com.webos.app.hdmi2".into();
        assert_eq!(marked(&i), vec![("Apple TV".into(), Some("MPLAY".into()), false), ("PlayStation 5".into(), Some("GAME".into()), true)]);
    }

    #[test]
    fn the_switch_is_read_from_the_receiver_document_and_there_is_none_without_a_choice_of_inputs() {
        let doc = json!({"enabled": true, "settingsSession": "s1", "settings": {"input": {"support": "supported", "value": "TV", "writable": true,
            "options": [{"value": "TV", "label": "TV Audio"}, {"value": "MPLAY", "label": "Apple TV"}]}}});
        let s = Switch::from_doc(&doc).unwrap();
        assert_eq!((s.current.as_deref(), s.writable, s.session.as_str(), s.sources.len()), (Some("TV"), true, "s1", 2));
        let mut busy = doc.clone();
        busy["settings"]["input"]["writable"] = json!(false);
        assert!(!Switch::from_doc(&busy).unwrap().writable, "an input the helper will not take now: names and the mark, no switch");
        let mut no_session = doc.clone();
        no_session["settingsSession"] = json!("");
        assert!(!Switch::from_doc(&no_session).unwrap().writable);
        let mut unread = doc.clone();
        unread["settings"]["input"] = json!({"support": "unknown", "options": []});
        assert_eq!(Switch::from_doc(&unread), None);
        let mut off = doc;
        off["enabled"] = json!(false);
        assert_eq!(Switch::from_doc(&off), None);
    }
}
