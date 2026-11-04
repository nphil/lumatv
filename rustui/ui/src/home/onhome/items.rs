//! The items of the card: what each tile of the helper's `onhome` topic, and each camera, is as something to look at and press.
//!
//! The helper says everything in words (a name, a line of status, a glyph, a tone, what OK does: `helper/app/src/onhome/tile.rs`); an item is
//! that, read once into what the card draws and the keys decide on. Nothing here knows Home Assistant's attributes.

use super::layout::Size;
use serde_json::Value;

/// What an item is: it decides how it is drawn and what a press does with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Light,
    /// A switch, a plug, a fan: a thing that is on or off.
    Switch,
    /// A scene or a script.
    Scene,
    /// A sensor that reads a number or words.
    Sensor,
    /// A sensor that is on or off (a door, a motion sensor).
    Binary,
    Climate,
    Cover,
    Lock,
    Media,
    Camera,
}

/// What OK does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// Switches it, at once: a light, a switch, a fan.
    Toggle,
    /// Runs it: a scene, a script.
    Run,
    /// Locks it, at once.
    Lock,
    /// Asks whether to unlock it.
    Unlock,
    /// Opens its controls: a thermostat, a cover; and the words of a thing that is only shown.
    Controls,
    /// Opens the history of its readings.
    History,
    /// Opens the camera at full size.
    Camera,
}

/// What a held OK does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    Nothing,
    /// The colour sheet of a lamp.
    Colour,
    /// The controls of a thing that has finer ones than its press (a fan's speed).
    Controls,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The entity's id, or `camera:<id>` for a camera.
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub area: String,
    /// The status in words ("On · 60%", "21.4°C", "Open").
    pub words: String,
    pub icon: String,
    pub tone: String,
    /// Switched on, lit, open: what the glyph and the switch show.
    pub on: bool,
    /// Home Assistant cannot reach it, or no longer shares it: it looks unavailable and offers nothing.
    pub gone: bool,
    /// The size the owner gave it, or the kind's own.
    pub size: Size,
    pub press: Press,
    pub hold: Hold,
    /// The colour a lit lamp shows.
    pub rgb: Option<[u8; 3]>,
    /// How bright it is, 0 to 1: the strength of a lamp's glow.
    pub level: f32,
}

impl Item {
    /// Whether the status words are what the item is for (a reading, a state) and the name is the line under it.
    pub fn headline(&self) -> bool {
        matches!(self.kind, Kind::Sensor | Kind::Binary | Kind::Climate | Kind::Cover | Kind::Lock | Kind::Media)
    }

    /// Whether the item is a switch that is drawn as one: a control with a track and a thumb.
    pub fn has_switch(&self) -> bool {
        self.kind == Kind::Switch && self.press == Press::Toggle
    }

    /// Whether it is a thing of the house that can be lit (a lamp).
    pub fn is_lamp(&self) -> bool {
        self.kind == Kind::Light
    }
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_owned()
}

fn strings(v: &Value, k: &str) -> Vec<String> {
    v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
}

/// The size an item has when the owner has not chosen: wide for what has more to say (a camera, a thermostat, a player), small for the rest.
pub fn default_size(kind: Kind) -> Size {
    match kind {
        Kind::Camera | Kind::Climate | Kind::Media => Size::Wide,
        _ => Size::Small,
    }
}

/// The item of one tile of the topic.
pub fn of_tile(tile: &Value) -> Option<Item> {
    let id = text(tile, "id");
    if id.is_empty() {
        return None;
    }
    let group = text(tile, "group");
    let domain = id.split('.').next().unwrap_or("");
    let ctl = tile.get("ctl").cloned().unwrap_or(Value::Null);
    let can = strings(tile, "can");
    let verbs = strings(tile, "verbs");
    let gone = tile.get("available").and_then(Value::as_bool) == Some(false);
    let has = |v: &str| can.iter().any(|c| c == v);
    let kind = match (group.as_str(), domain) {
        ("lights", _) => Kind::Light,
        ("switches", _) => Kind::Switch,
        ("scenes", _) => Kind::Scene,
        ("climate", _) => Kind::Climate,
        ("covers", _) => Kind::Cover,
        ("locks", _) => Kind::Lock,
        ("media", _) => Kind::Media,
        ("sensors", "binary_sensor") => Kind::Binary,
        ("sensors", _) => Kind::Sensor,
        _ => Kind::Sensor,
    };
    let on = match kind {
        Kind::Light | Kind::Switch | Kind::Binary => ctl.get("on").and_then(Value::as_bool).unwrap_or(false),
        Kind::Media => text(tile, "tone") == "on",
        _ => false,
    };
    let press = if gone {
        Press::Controls
    } else if has("toggle") {
        Press::Toggle
    } else if has("run") {
        Press::Run
    } else if has("lock") {
        Press::Lock
    } else if has("unlock") {
        Press::Unlock
    } else if matches!(kind, Kind::Sensor | Kind::Binary) {
        Press::History
    } else {
        Press::Controls
    };
    // a lamp with a level, a colour, a white or an effect to set has a sheet for them; a fan has its speed; the rest has nothing under a hold
    let dimmable = ctl.get("dimmable").and_then(Value::as_bool) == Some(true);
    let colour = ctl.get("colour").and_then(Value::as_bool) == Some(true);
    let kelvin = ctl.get("kelvin").is_some_and(|k| !k.is_null());
    let effects = ctl.get("effects").and_then(Value::as_array).is_some_and(|e| !e.is_empty());
    let may = verbs.iter().any(|v| v == "turn_on");
    let hold = match kind {
        Kind::Light if !gone && may && (dimmable || colour || kelvin || effects) => Hold::Colour,
        Kind::Switch if !gone && domain == "fan" && verbs.iter().any(|v| v == "set_percentage") => Hold::Controls,
        _ => Hold::Nothing,
    };
    let rgb = ctl.get("rgb").and_then(Value::as_array).and_then(|a| {
        let c: Vec<u8> = a.iter().filter_map(Value::as_f64).map(|v| v.clamp(0.0, 255.0).round() as u8).collect();
        (c.len() == 3).then(|| [c[0], c[1], c[2]])
    });
    let level = ctl.get("brightness").and_then(Value::as_f64).map_or(1.0, |b| (b / 100.0).clamp(0.05, 1.0)) as f32;
    let size = tile.get("size").and_then(Value::as_str).and_then(Size::from_id).unwrap_or_else(|| default_size(kind));
    Some(Item {
        id,
        kind,
        name: text(tile, "name"),
        area: text(tile, "area"),
        words: text(tile, "words"),
        icon: text(tile, "icon"),
        tone: text(tile, "tone"),
        on: on && !gone,
        gone,
        size,
        press,
        hold,
        rgb: rgb.filter(|_| on && !gone),
        level,
    })
}

/// The camera `cam` of the cameras topic as an item (a still, a name; OK opens the viewer).
pub fn of_camera(cam: &Value) -> Option<Item> {
    let id = cam.get("id").and_then(Value::as_str).filter(|i| !i.is_empty())?;
    let online = cam.get("online").and_then(Value::as_bool) == Some(true);
    let name = cam.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(id).to_owned();
    Some(Item {
        id: format!("camera:{id}"),
        kind: Kind::Camera,
        name,
        area: String::new(),
        words: if online { "Live".into() } else { "Offline".into() },
        icon: "camera".into(),
        tone: if online { "info".into() } else { "muted".into() },
        on: false,
        gone: !online,
        size: default_size(Kind::Camera),
        press: Press::Camera,
        hold: Hold::Nothing,
        rgb: None,
        level: 1.0,
    })
}

/// How many cameras the card carries: a card is for the house's things; the cameras card is for the cameras.
pub const MAX_CAMERAS: usize = 2;

/// The items of the card, in order: the cameras first, then the tiles as the helper lists them (the owner's order).
pub fn of_topics(onhome: &Value, cameras: &Value) -> Vec<Item> {
    let cams = cameras.get("cameras").and_then(Value::as_array).into_iter().flatten().filter_map(of_camera).take(MAX_CAMERAS);
    let tiles = onhome.get("tiles").and_then(Value::as_array).into_iter().flatten().filter_map(of_tile);
    cams.chain(tiles).collect()
}

/// The id of the camera of an item, if it is one.
pub fn camera_of(id: &str) -> Option<&str> {
    id.strip_prefix("camera:")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lamp() -> Value {
        json!({"id": "light.lamp", "name": "Lamp", "group": "lights", "words": "On \u{b7} 60%", "tone": "on", "icon": "lightbulb", "available": true, "can": ["toggle"],
               "verbs": ["turn_on", "turn_off", "toggle"], "ctl": {"on": true, "dimmable": true, "brightness": 60, "colour": true, "rgb": [255, 0, 128], "effects": ["Rainbow"], "kelvin": null}})
    }

    #[test]
    fn a_lamp_is_lit_in_its_own_colour_at_its_own_level_and_a_hold_opens_its_sheet() {
        let i = of_tile(&lamp()).unwrap();
        assert_eq!((i.kind, i.on, i.rgb, i.press, i.hold, i.size), (Kind::Light, true, Some([255, 0, 128]), Press::Toggle, Hold::Colour, Size::Small));
        assert!((i.level - 0.6).abs() < 1e-6);
        let mut off = lamp();
        off["ctl"]["on"] = json!(false);
        off["ctl"]["rgb"] = Value::Null;
        let i = of_tile(&off).unwrap();
        assert_eq!((i.on, i.rgb), (false, None), "off is not lit");
        // a plain on/off lamp has nothing to hold for; one the integration does not let the TV switch is only shown
        let mut plain = lamp();
        plain["ctl"] = json!({"on": true, "dimmable": false, "colour": false, "kelvin": null, "effects": []});
        assert_eq!(of_tile(&plain).unwrap().hold, Hold::Nothing);
        let mut shown = lamp();
        shown["can"] = json!([]);
        shown["verbs"] = json!([]);
        let i = of_tile(&shown).unwrap();
        assert_eq!((i.press, i.hold), (Press::Controls, Hold::Nothing));
    }

    #[test]
    fn what_a_press_does_follows_what_the_tile_can_do_and_a_sensor_opens_its_history() {
        let sensor = json!({"id": "sensor.t", "name": "Hall", "group": "sensors", "words": "21.4\u{b0}C", "tone": "info", "icon": "thermometer", "available": true, "can": [], "verbs": [], "ctl": {"kind": "number", "value": 21.4, "unit": "\u{b0}C", "class": "temperature"}});
        let i = of_tile(&sensor).unwrap();
        assert_eq!((i.kind, i.press, i.headline()), (Kind::Sensor, Press::History, true));
        let door = json!({"id": "binary_sensor.d", "name": "Door", "group": "sensors", "words": "Open", "tone": "on", "icon": "door-open", "available": true, "can": [], "verbs": [], "ctl": {"kind": "binary", "on": true}});
        let i = of_tile(&door).unwrap();
        assert_eq!((i.kind, i.press, i.on), (Kind::Binary, Press::History, true));
        let scene = json!({"id": "scene.movie", "name": "Movie", "group": "scenes", "words": "Scene", "tone": "muted", "icon": "sparkles", "available": true, "can": ["run"], "verbs": ["turn_on"], "ctl": {}});
        assert_eq!(of_tile(&scene).unwrap().press, Press::Run);
        let lock = json!({"id": "lock.f", "name": "Front", "group": "locks", "words": "Locked", "tone": "ok", "icon": "lock", "available": true, "can": ["unlock"], "verbs": ["lock"], "ctl": {"state": "locked"}});
        assert_eq!(of_tile(&lock).unwrap().press, Press::Unlock);
        let climate = json!({"id": "climate.t", "name": "Thermostat", "group": "climate", "words": "68\u{b0}", "tone": "off", "icon": "thermometer", "available": true, "can": ["sheet"], "verbs": ["set_temperature"], "ctl": {}});
        let i = of_tile(&climate).unwrap();
        assert_eq!((i.press, i.size), (Press::Controls, Size::Wide), "a thermostat has more to say: wide unless the owner says otherwise");
        // a thing that is gone offers nothing, whatever it could do
        let mut gone = lamp();
        gone["available"] = json!(false);
        let i = of_tile(&gone).unwrap();
        assert_eq!((i.gone, i.on, i.press, i.hold, i.rgb), (true, false, Press::Controls, Hold::Nothing, None));
    }

    #[test]
    fn the_owners_size_is_kept_and_a_size_that_is_not_one_is_the_kinds_own() {
        let mut t = lamp();
        t["size"] = json!("large");
        assert_eq!(of_tile(&t).unwrap().size, Size::Large);
        t["size"] = json!("huge");
        assert_eq!(of_tile(&t).unwrap().size, Size::Small);
        t["size"] = Value::Null;
        assert_eq!(of_tile(&t).unwrap().size, Size::Small);
    }

    #[test]
    fn at_most_two_cameras_come_first_and_open_the_viewer() {
        let cams = json!({"cameras": [{"id": "front", "name": "Front door", "online": true}, {"id": "back", "online": false}, {"id": "side", "online": true}]});
        let items = of_topics(&json!({"tiles": [lamp()]}), &cams);
        assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["camera:front", "camera:back", "light.lamp"]);
        assert_eq!((items[0].press, items[0].gone, items[0].size, camera_of(&items[0].id)), (Press::Camera, false, Size::Wide, Some("front")));
        assert_eq!((items[1].name.as_str(), items[1].gone, items[1].words.as_str()), ("back", true, "Offline"));
        assert_eq!(camera_of("light.lamp"), None);
    }
}
