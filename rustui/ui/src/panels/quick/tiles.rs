//! The quick panel's tiles as descriptors, evaluated now (`QuickPanel.qml` `tiles()` and `QuickMore.qml` of the QML shell, where each
//! field was a binding): a row has a kind (`stepper`, `nudge`, `toggle`, `value`, `action`), its words, its value and what an act on it
//! does ([`Act`]). The built-in tiles come first, then the plugins' (between them and AI access), and "Add a tile" offers the rest.
//! Nothing here changes a setting: [`Quick::act`](super::Quick) carries an [`Act`] out.

use super::super::arrange::Cat;
use super::super::env::Env;
use super::super::hub;
use crate::host::power::SLEEP_LABELS;
use crate::settings::names;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Left and Right change a value from a list (it wraps), OK opens the list as a card.
    Stepper,
    /// Left and Right nudge a value that is read back; OK opens the page that has more.
    Nudge,
    Toggle,
    /// OK presses it; a chip or a chevron at its right says it opens something.
    Value,
    /// OK presses it; a button's words at its right say what it does.
    Action,
}

/// What an act on a tile does.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Picture(Vec<(String, bool)>),
    Receiver,
    Sleep,
    Settings(&'static str),
    Energy,
    Oled,
    ScreenOff,
    Pointer,
    Eye,
    Shot,
    Record,
    Plugin { plugin: String, action: String, target: Option<String> },
}

/// A value card with a track (OLED pixel brightness).
#[derive(Clone, Debug, PartialEq)]
pub struct Slider {
    pub title: &'static str,
    pub text: &'static str,
    pub from: f64,
    pub to: f64,
    pub step: f64,
    pub marks: [&'static str; 3],
    pub icons: [&'static str; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub kind: Kind,
    pub icon: String,
    pub label: String,
    pub detail: String,
    /// The lines the second line keeps, empty or not, for a row whose answer comes late: the rows below do not move.
    pub detail_lines: u8,
    pub enabled: bool,
    pub visible: bool,
    pub options: Vec<String>,
    pub index: Option<usize>,
    /// A nudge's reading, a value row's words at its right.
    pub value: String,
    /// The longest value its control takes, so a control beside the words does not change its mind as the value changes.
    pub widest: &'static str,
    pub on: bool,
    pub button: String,
    pub chip: bool,
    pub tone: &'static str,
    pub chevron: bool,
    pub act: Act,
    pub slider: Option<Slider>,
    /// A panel with no arrangement shows it.
    pub dflt: bool,
}

impl Row {
    fn base(id: &str, kind: Kind, icon: &str, label: &str, act: Act) -> Self {
        Self {
            id: id.to_string(),
            kind,
            icon: icon.to_string(),
            label: label.to_string(),
            detail: String::new(),
            detail_lines: 1,
            enabled: true,
            visible: true,
            options: Vec::new(),
            index: None,
            value: String::new(),
            widest: "Filmmaker",
            on: false,
            button: String::new(),
            chip: false,
            tone: "",
            chevron: true,
            act,
            slider: None,
            dflt: true,
        }
    }

    /// Whether the D-pad can land on it.
    pub fn focusable(&self) -> bool {
        self.visible && self.enabled
    }

    pub fn step_index(&self, from: usize, dir: i32) -> Option<usize> {
        let n = self.options.len();
        (1..=n).map(|step| (from as i32 + dir * step as i32).rem_euclid(n as i32) as usize).find(|&i| {
            !matches!(&self.act, Act::Picture(values) if !values.get(i).is_some_and(|(_, active)| *active))
        })
    }
}

pub const MODE_VALUES: [&str; 7] = ["filmMaker", "cinema", "normal", "vivid", "game", "sports", "eco"];
pub const ENERGY_NAMES: [&str; 5] = ["Auto", "Off", "Minimum", "Medium", "Maximum"];
pub const ENERGY_VALUES: [&str; 5] = ["auto", "off", "min", "med", "max"];
/// The en dash a reading shows where the TV has not answered.
pub const DASH: &str = "\u{2013}";

pub const OLED: Slider = Slider {
    title: "OLED pixel brightness",
    text: "How bright the screen is.",
    from: 0.0,
    to: 100.0,
    step: 5.0,
    marks: ["0", "50", "100"],
    icons: ["sun-dim", "sun"],
};

/// "Denon receiver": the first word of the receiver's model and "receiver", or just "Receiver".
fn brand(rc: Option<&Value>) -> String {
    let model = rc.and_then(|r| r.get("model")).and_then(Value::as_str).unwrap_or("");
    let word: String = model.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if word.is_empty() { "Receiver".into() } else { format!("{word} receiver") }
}

/// The built-in tiles, the plugins' between them and AI access.
pub fn tiles(env: &Env) -> Vec<Row> {
    let current = env.sio.text("picture.pictureMode");
    let choices = if let Some(list) = env.sio.list("picture.pictureMode") {
        list.into_iter().filter(|item| item.visible && !item.value.is_empty()).map(|item| (item.value, item.active)).collect::<Vec<_>>()
    } else if env.sio.desc("picture.pictureMode").is_none() && MODE_VALUES.contains(&current.as_str()) && env.sio.picture_dynamic_range().is_some_and(|range| range.eq_ignore_ascii_case("sdr")) {
        MODE_VALUES.iter().map(|value| ((*value).to_string(), true)).collect()
    } else {
        Vec::new()
    };
    let mut picture = Row::base("picture", Kind::Stepper, "image", "Picture mode", Act::Picture(Vec::new()));
    picture.detail = env.inputs.current().map(|e| e.port.clone()).unwrap_or_default();
    picture.options = choices.iter().map(|(value, _)| names::picture_mode(value).to_string()).collect();
    picture.index = choices.iter().position(|(value, _)| *value == current);
    picture.value = names::picture_mode(&current).to_string();
    picture.enabled = env.sio.usable("picture.pictureMode") && choices.iter().any(|(_, active)| *active);
    picture.visible = env.sio.shown("picture.pictureMode");
    if choices.is_empty() {
        picture.kind = Kind::Nudge;
    }
    picture.act = Act::Picture(choices);

    let rc = env.gate.rc();
    let answers = env.gate.fresh();
    let mut receiver = Row::base("receiver", Kind::Nudge, "boom-box", &brand(rc), Act::Receiver);
    receiver.detail_lines = 2;
    receiver.detail = if answers {
        let s = |k: &str| rc.and_then(|r| r.get(k)).and_then(Value::as_str).unwrap_or("").to_string();
        let input = if s("input").is_empty() { "Receiver".to_string() } else { s("input") };
        let mode = s("soundMode");
        if mode.is_empty() { input } else { format!("{input} \u{b7} {mode}") }
    } else {
        "Not answering".into()
    };
    receiver.value = env.gate.doc().and_then(|d| d.get("volumeText")).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(DASH).to_string();
    receiver.widest = "\u{2212}34.5 dB";
    receiver.enabled = !env.gate.disabled();

    let mut sleep = Row::base("sleep", Kind::Stepper, "moon", "Sleep timer", Act::Sleep);
    sleep.detail = env.power.sleep_left();
    sleep.options = SLEEP_LABELS.iter().map(|s| s.to_string()).collect();
    sleep.index = Some(env.power.sleep_index());

    let mut wifi = Row::base("wifi", Kind::Value, "wifi", "Wi-Fi", Act::Settings("network"));
    wifi.detail = env.net.summary();

    let ai_topic = env.topic("ai");
    let listener = ai_topic.and_then(|a| a.pointer("/listener/enabled")).and_then(Value::as_bool) == Some(true);
    let paired = ai_topic.and_then(|a| a.get("assistants")).and_then(Value::as_array).map_or(0, Vec::len);
    let mut ai = Row::base("ai", Kind::Value, "bot", "AI access", Act::Settings("ai"));
    ai.chip = true;
    ai.chevron = false;
    ai.detail = if ai_topic.is_none() {
        String::new()
    } else {
        format!("{}{}", if listener { "Home network only" } else { "" }, if paired > 0 { format!("{}{} paired", if listener { " \u{b7} " } else { "" }, paired) } else { String::new() })
    };
    ai.value = if listener { "Allowed" } else { "Blocked" }.into();
    ai.tone = if listener { "positive" } else { "" };

    let mut out = vec![picture, receiver, sleep, wifi];
    out.extend(plugin_rows(env));
    out.push(ai);
    out
}

/// The tiles plugins put (`PluginHub.tileRows`): a tile with an on/off state is a toggle, any other a button that runs its action.
/// An observed tile shows its state and cannot be flipped from here (the plugin owns it); one that is unavailable says why and takes
/// no focus.
pub fn plugin_rows(env: &Env) -> Vec<Row> {
    hub::tiles(&env.topics)
        .into_iter()
        .map(|t| {
            let stateful = matches!(t.state.as_str(), "on" | "off" | "observed");
            let act = Act::Plugin { plugin: t.plugin_id.clone(), action: t.action.clone(), target: t.target.clone() };
            let mut r = Row::base(&format!("plugin.{}.{}", t.plugin_id, t.id), if stateful { Kind::Toggle } else { Kind::Value }, &t.icon, if t.label.is_empty() { &t.id } else { &t.label }, act);
            r.detail = if t.state == "unavailable" { hub::unavailable_text(&t.reason).to_string() } else { t.hint.clone() };
            r.enabled = t.state != "unavailable" && t.state != "observed";
            r.on = t.on;
            r
        })
        .collect()
}

/// What "Add a tile" offers beyond the built-in ones (`QuickMore.rows`), each the settings page's row for the same TV setting.
pub fn extras(env: &Env) -> Vec<Row> {
    let mut screenoff = Row::base("screenoff", Kind::Action, "monitor", "Screen off, sound on", Act::ScreenOff);
    screenoff.button = "Screen off".into();
    screenoff.enabled = env.sio.usable("option.screenOff");

    let mut energy = Row::base("energy", Kind::Stepper, "leaf", "Energy saving", Act::Energy);
    energy.options = ENERGY_NAMES.iter().map(|s| s.to_string()).collect();
    energy.enabled = env.sio.has("picture.energySaving");
    energy.index = ENERGY_VALUES.iter().position(|m| *m == env.sio.text("picture.energySaving"));

    let mut oled = Row::base("oled", Kind::Nudge, "sun", "OLED pixel brightness", Act::Oled);
    oled.enabled = env.sio.usable("picture.backlight");
    oled.value = if env.sio.has("picture.backlight") { format!("{}", env.sio.num("picture.backlight", 0.0).round() as i64) } else { DASH.into() };
    oled.widest = "\u{2212}34.5 dB";
    oled.slider = Some(OLED);

    let mut pointer = Row::base("pointer", Kind::Toggle, "mouse-pointer-2", "Magic Remote pointer", Act::Pointer);
    pointer.enabled = env.sio.usable("general.remotePointer");
    pointer.on = env.sio.is("general.remotePointer", "on");

    let mut eye = Row::base("eye", Kind::Toggle, "eye", "Eye comfort", Act::Eye);
    eye.enabled = env.sio.usable("picture.eyeComfortMode");
    eye.on = env.sio.is("picture.eyeComfortMode", "on");

    let mut bluetooth = Row::base("bluetooth", Kind::Value, "headphones", "Bluetooth audio", Act::Settings("sound"));
    bluetooth.detail = "Speakers and headphones".into();

    let mut out = vec![screenoff, energy, oled, pointer, eye, bluetooth];
    if env.capture.enabled {
        let mut shot = Row::base("shot", Kind::Action, "camera", "Screenshot", Act::Shot);
        shot.button = "Capture".into();
        shot.enabled = env.capture.available;
        let mut record = Row::base("record", Kind::Toggle, "video", "Screen recording", Act::Record);
        record.enabled = (env.capture.available && env.capture.encoder) || env.capture.recording;
        record.on = env.capture.recording;
        out.push(shot);
        out.push(record);
    }
    for r in &mut out {
        r.dflt = false;
    }
    out
}

/// Every tile there is, the default ones first (with `dflt` set) in the panel's own order.
pub fn everything(env: &Env) -> Vec<Row> {
    let mut all = tiles(env);
    all.extend(extras(env));
    all
}

pub fn catalogue(rows: &[Row]) -> Vec<Cat> {
    rows.iter().map(|r| Cat { id: r.id.clone(), dflt: r.dflt }).collect()
}

/// The words a value card adds to a tile that has choices: what it is for, the icon of a row, and (the sleep timer) when the TV would
/// turn off for each choice, as a clock time.
pub struct Extras {
    pub text: &'static str,
    pub icon: Option<&'static str>,
}

pub fn pick_extras(id: &str) -> Extras {
    match id {
        "sleep" => Extras { text: "The TV turns off after this long. A minute before, a toast lets you cancel.", icon: Some("clock") },
        "picture" => Extras { text: "How the picture looks. Each input keeps its own.", icon: Some("image") },
        "energy" => Extras { text: "How far the TV lowers the picture's brightness to save power.", icon: Some("leaf") },
        _ => Extras { text: "", icon: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::screen::Input;
    use serde_json::json;

    fn env() -> Env {
        let mut e = Env::new(json!({}));
        e.update(&Input::Helper { topic: "up".into(), data: json!(true) }, 0);
        e
    }

    fn rx(at: u64) -> Value {
        json!({"enabled": true, "reachable": true, "fresh": true, "power": "on", "input": "TV Audio", "volumeText": "-34.5 dB", "model": "Denon AVR-S760H", "soundMode": "Dolby Surround", "updatedAt": at})
    }

    #[test]
    fn the_built_in_tiles_come_in_the_panels_order_with_the_plugins_before_ai_access() {
        let mut e = env();
        e.update(&Input::Helper { topic: "plugins".into(), data: json!({"plugins": [{"id": "ambient", "state": "connected"}]}) }, 0);
        e.update(&Input::Helper { topic: "plugin.ambient.tiles".into(), data: json!({"items": [{"id": "ambient", "label": "Ambient light", "icon": "base.icons/lamp", "state": {"type": "on"}, "action": "toggle", "hint": "Follows the picture"}]}) }, 0);
        let ids: Vec<String> = tiles(&e).into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["picture", "receiver", "sleep", "wifi", "plugin.ambient.ambient", "ai"]);
    }

    #[test]
    fn the_picture_tile_needs_the_settings_service_and_shows_the_mode_and_the_input() {
        let mut e = env();
        let r = &tiles(&e)[0];
        assert!(!r.enabled && r.index.is_none(), "not focusable until the settings service has answered");
        let reads = e.sio.read("picture", &["pictureMode"], None, 0);
        let Some(crate::kit::screen::Effect::Luna { tag, .. }) = reads.first() else { panic!() };
        e.update(&Input::Luna { tag: *tag, payload: json!({"settings": {"pictureMode": "cinema"}, "dimension": {"dynamicRange": "sdr"}}) }, 1);
        e.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.hdmi1"}) }, 2);
        e.inputs.parse(&json!({"devices": [{"appId": "com.webos.app.hdmi1", "label": "Apple TV", "connected": true}]}));
        let r = &tiles(&e)[0];
        assert!(r.enabled);
        assert_eq!((r.index, r.detail.as_str(), r.options[1].as_str()), (Some(1), "HDMI 1", "Cinema"));
    }

    #[test]
    fn the_receivers_tile_shows_a_number_only_while_it_is_in_date_and_keeps_its_two_lines() {
        let mut e = env();
        e.gate.set_active(true, 0);
        e.update(&Input::Helper { topic: "receiver".into(), data: rx(1000) }, 1000);
        let r = tiles(&e).remove(1);
        assert_eq!((r.label.as_str(), r.detail.as_str(), r.value.as_str(), r.detail_lines), ("Denon receiver", "TV Audio \u{b7} Dolby Surround", "-34.5 dB", 2));
        e.update(&Input::Tick, 7000);
        let r = tiles(&e).remove(1);
        assert_eq!((r.detail.as_str(), r.value.as_str()), ("Not answering", DASH), "six seconds after the last reading the number is gone");
        assert_eq!(r.label, "Denon receiver", "the name is still known");
        e.update(&Input::Helper { topic: "receiver".into(), data: json!({"enabled": false, "reachable": false, "fresh": false}) }, 7100);
        assert!(!tiles(&e).remove(1).enabled, "a receiver switched off in Settings is not reached");
    }

    /// The topic as the helper publishes it on the set: `updatedAt` moves without an event, so the copy the panel holds is old by the time it opens.
    #[test]
    fn the_receivers_tile_opened_on_an_old_copy_asks_the_helper_and_shows_the_answer_with_the_helper_up() {
        let doc = |at: u64| json!({"enabled": true, "error": null, "fresh": true, "host": "192.168.1.93", "input": "TV", "mute": true, "polling": true, "power": "on", "reachable": true, "signal": "PCM", "soundMode": "DOLBY AUDIO-DSUR", "updatedAt": at, "volumeAbs": 36.0, "volumeDb": -44.0, "volumeDisplay": "absolute", "volumeMax": 69.5, "volumeText": "36"});
        let mut e = Env::new(json!({}));
        e.update(&Input::Helper { topic: "up".into(), data: json!(true) }, 1_793_283_000_000);
        e.update(&Input::Helper { topic: "receiver".into(), data: doc(1_793_283_000_000) }, 1_793_283_000_000);
        let opened = 1_793_283_133_000;
        let ask = e.gate.set_active(true, opened);
        assert!(matches!(ask.as_slice(), [crate::kit::screen::Effect::Fetch { path, .. }] if path == "/v1/state/receiver"), "the copy is two minutes old: the helper is asked");
        assert_eq!(tiles(&e).remove(1).detail, "Not answering", "until it answers");
        e.update(&Input::Fetched { tag: crate::host::receiver::TAG, status: 200, data: json!({"api": 1, "ok": true, "topic": "receiver", "value": doc(1_793_283_132_900)}) }, opened + 40);
        let r = tiles(&e).remove(1);
        assert_eq!((r.label.as_str(), r.detail.as_str(), r.value.as_str()), ("Receiver", "TV \u{b7} DOLBY AUDIO-DSUR", "36"));
    }

    #[test]
    fn a_plugins_tiles_are_toggles_or_buttons_and_an_unavailable_one_takes_no_focus() {
        let mut e = env();
        e.update(&Input::Helper { topic: "plugins".into(), data: json!({"plugins": [{"id": "ambient", "state": "connected"}, {"id": "lock", "state": "connected"}]}) }, 0);
        e.update(&Input::Helper { topic: "plugin.ambient.tiles".into(), data: json!({"items": [{"id": "ambient", "label": "Ambient light", "icon": "base.icons/lamp", "state": {"type": "on"}, "action": "toggle", "hint": "Follows the picture"}]}) }, 0);
        e.update(
            &Input::Helper {
                topic: "plugin.lock.tiles".into(),
                data: json!({"items": [
                    {"id": "door", "label": "Front door", "icon": "base.icons/lock", "state": {"type": "unavailable", "reason": "offline"}, "action": "lock"},
                    {"id": "scene", "label": "Movie scene", "icon": "base.icons/home", "state": {"type": "unknown"}, "action": "run", "target": "movie"},
                    {"id": "obs", "label": "Observed", "state": {"type": "observed", "on": true}, "action": "x"}
                ]}),
            },
            0,
        );
        let rows = plugin_rows(&e);
        let by = |id: &str| rows.iter().find(|r| r.id == id).unwrap().clone();
        let ambient = by("plugin.ambient.ambient");
        assert_eq!((ambient.kind, ambient.on, ambient.detail.as_str(), ambient.enabled), (Kind::Toggle, true, "Follows the picture", true));
        let door = by("plugin.lock.door");
        assert_eq!((door.kind, door.detail.as_str(), door.enabled), (Kind::Value, "Offline", false));
        let scene = by("plugin.lock.scene");
        assert_eq!(scene.kind, Kind::Value);
        assert_eq!(scene.act, Act::Plugin { plugin: "lock".into(), action: "run".into(), target: Some("movie".into()) });
        let obs = by("plugin.lock.obs");
        assert!(!obs.enabled && obs.on, "an observed tile shows its state and cannot be flipped from here");
    }

    #[test]
    fn the_more_tiles_follow_their_settings_and_capture_comes_only_when_the_helper_has_it_on() {
        let mut e = env();
        let ids = |e: &Env| extras(e).into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids(&e), vec!["screenoff", "energy", "oled", "pointer", "eye", "bluetooth"]);
        e.update(&Input::Fetched { tag: crate::panels::env::tags::CAPTURE_READ, status: 200, data: json!({"capture": {"enabled": true, "available": true, "encoder": {"available": true}}}) }, 0);
        assert_eq!(ids(&e), vec!["screenoff", "energy", "oled", "pointer", "eye", "bluetooth", "shot", "record"]);
        assert!(extras(&e).iter().all(|r| !r.dflt), "none is on the panel until the owner adds it");
        let oled = &extras(&e)[2];
        assert_eq!((oled.value.as_str(), oled.enabled), (DASH, false));
        assert_eq!(oled.slider.as_ref().map(|s| (s.from, s.to, s.step)), Some((0.0, 100.0, 5.0)));
    }
}
