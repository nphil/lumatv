//! What the TV says of one Home Assistant entity, and what it may ask of it. Pure: no socket, no clock.
//!
//! **Words, not attributes.** Home draws a tile from what this module returns (a name, a line of status, a glyph, a tone, the verbs
//! the OK key has and the numbers a detail sheet moves), never from Home Assistant's own attributes: the TV's UI has no table of
//! device classes, and an entity it does not know is still drawn honestly (its state, unavailable when it is).
//!
//! **The only way to a service call is [`plan`].** It is a table of (domain, verb) to one service and the few plain values that
//! service is given. Locks, alarms, sirens, cameras and valves have no row at all, a cover has one only when it is not a garage door
//! or a gate, and a switch, a script or a scene whose name says it unlocks, disarms or switches a camera off is refused whatever
//! its domain (Principle 0: doors, alarms and cameras are the owner's to operate). The table is the whole of what the TV can do to
//! the house; nothing else in the module builds a call.

use serde_json::{Map, Value, json};

/// The most entities Home carries (Home Assistant's own list of states is far longer; a card has room for a screenful).
pub const MAX_PICKS: usize = 48;

/// The domains a person can put on Home.
pub const PICKABLE: [&str; 12] = [
    "light",
    "switch",
    "input_boolean",
    "fan",
    "scene",
    "script",
    "sensor",
    "binary_sensor",
    "climate",
    "lock",
    "cover",
    "media_player",
];

/// The kinds Home groups its tiles by, in the order it lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Lights,
    Switches,
    Scenes,
    Climate,
    Covers,
    Sensors,
    Locks,
    Media,
}

pub const GROUPS: [Group; 8] = [
    Group::Lights,
    Group::Switches,
    Group::Scenes,
    Group::Climate,
    Group::Covers,
    Group::Sensors,
    Group::Locks,
    Group::Media,
];

impl Group {
    pub fn of_domain(domain: &str) -> Option<Self> {
        Some(match domain {
            "light" => Self::Lights,
            "switch" | "input_boolean" | "fan" => Self::Switches,
            "scene" | "script" => Self::Scenes,
            "climate" => Self::Climate,
            "cover" => Self::Covers,
            "sensor" | "binary_sensor" => Self::Sensors,
            "lock" => Self::Locks,
            "media_player" => Self::Media,
            _ => return None,
        })
    }

    pub fn of_id(id: &str) -> Option<Self> {
        GROUPS.iter().copied().find(|g| g.id() == id)
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Lights => "lights",
            Self::Switches => "switches",
            Self::Scenes => "scenes",
            Self::Climate => "climate",
            Self::Covers => "covers",
            Self::Sensors => "sensors",
            Self::Locks => "locks",
            Self::Media => "media",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Lights => "Lights",
            Self::Switches => "Switches & fans",
            Self::Scenes => "Scenes & scripts",
            Self::Climate => "Climate",
            Self::Covers => "Covers",
            Self::Sensors => "Sensors",
            Self::Locks => "Locks",
            Self::Media => "Media players",
        }
    }
}

/// The domain of an entity id (`light.lamp` is `light`), when it is one of [`PICKABLE`] and the id is plain.
pub fn domain_of(id: &str) -> Option<&str> {
    let (domain, name) = id.split_once('.')?;
    let plain = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    (id.len() <= 128 && plain(domain) && plain(name) && PICKABLE.contains(&domain)).then_some(domain)
}

// -- what is kept of a state ---------------------------------------------------------------------------------------------------

/// One entity as it is kept: the state and the few attributes the tiles are made from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ent {
    pub state: String,
    pub attrs: Map<String, Value>,
}

const KEPT: [&str; 25] = [
    "friendly_name",
    "device_class",
    "unit_of_measurement",
    "brightness",
    "color_temp_kelvin",
    "min_color_temp_kelvin",
    "max_color_temp_kelvin",
    "supported_color_modes",
    "percentage",
    "percentage_step",
    "current_temperature",
    "temperature",
    "target_temp_low",
    "target_temp_high",
    "target_temp_step",
    "min_temp",
    "max_temp",
    "hvac_modes",
    "hvac_action",
    "current_humidity",
    "current_position",
    "media_title",
    "media_artist",
    "app_name",
    "source",
];

fn short(text: &str) -> bool {
    text.chars().count() <= 128
}

/// Whether an attribute is one the tiles use, and plain enough to keep (a short text, a number, a flag, a short list of short words).
pub fn keeps(name: &str, value: &Value) -> bool {
    KEPT.contains(&name)
        && match value {
            Value::String(s) => short(s),
            Value::Number(_) | Value::Bool(_) => true,
            Value::Array(list) => list.len() <= 16 && list.iter().all(|v| v.as_str().is_some_and(|s| s.chars().count() <= 32)),
            _ => false,
        }
}

pub fn kept(all: &Map<String, Value>) -> Map<String, Value> {
    all.iter().filter(|(k, v)| keeps(k, v)).map(|(k, v)| (k.clone(), v.clone())).collect()
}

impl Ent {
    fn text(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
    }

    fn num(&self, key: &str) -> Option<f64> {
        self.attrs.get(key).and_then(Value::as_f64)
    }

    fn list(&self, key: &str) -> Vec<&str> {
        self.attrs.get(key).and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
    }

    pub fn name(&self) -> Option<&str> {
        self.text("friendly_name")
    }

    /// Home Assistant says it cannot reach the thing, or does not know its state.
    pub fn gone(&self) -> bool {
        matches!(self.state.as_str(), "unavailable" | "unknown" | "")
    }
}

// -- words ---------------------------------------------------------------------------------------------------------------------------

/// A number as a person writes it: at most one decimal, none when it is whole.
pub fn number(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else { format!("{r:.1}") }
}

/// A number and its unit: a degree sign or a percent sign sits against the number, any other unit a space after it.
pub fn with_unit(value: &str, unit: &str) -> String {
    match unit {
        "" => value.to_owned(),
        u if u.starts_with('\u{b0}') || u == "%" => format!("{value}{u}"),
        u => format!("{value} {u}"),
    }
}

fn pct(brightness: f64) -> i64 {
    ((brightness * 100.0 / 255.0).round() as i64).clamp(1, 100)
}

fn title_case(text: &str) -> String {
    let t = text.replace('_', " ");
    let mut c = t.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.truncate(out.trim_end().len());
    out.push('\u{2026}');
    out
}

/// The warm and the cool end of a white lamp's slider.
const KELVIN: (i64, i64) = (2000, 6500);

/// What the tile of a thing says: the words, a tone, the glyph, what OK does and the numbers a sheet moves.
struct Said {
    words: String,
    tone: &'static str,
    icon: &'static str,
    can: Vec<&'static str>,
    ctl: Value,
}

fn said(words: impl Into<String>, tone: &'static str, icon: &'static str, can: Vec<&'static str>, ctl: Value) -> Said {
    Said { words: words.into(), tone, icon, can, ctl }
}

fn on_off(state: &str) -> (bool, &'static str) {
    if state == "on" { (true, "on") } else { (false, "off") }
}

fn say(domain: &str, e: &Ent, unit: &str) -> Said {
    match domain {
        "light" => light(e),
        "switch" | "input_boolean" => {
            let (on, tone) = on_off(&e.state);
            let icon = match (domain, e.text("device_class")) {
                ("input_boolean", _) => "toggle-right",
                (_, Some("outlet")) => "plug",
                _ => "power",
            };
            said(if on { "On" } else { "Off" }, tone, icon, vec!["toggle"], json!({"on": on}))
        }
        "fan" => {
            let (on, tone) = on_off(&e.state);
            let level = e.num("percentage").map(|p| p.round() as i64);
            let words = match (on, level) {
                (true, Some(p)) if p > 0 => format!("On \u{b7} {p}%"),
                (true, _) => "On".into(),
                _ => "Off".into(),
            };
            let step = e.num("percentage_step").filter(|s| *s > 0.0);
            said(words, tone, "fan", vec!["toggle"], json!({"on": on, "percentage": level, "step": step}))
        }
        "scene" => said("Scene", "muted", "sparkles", vec!["run"], json!({})),
        "script" => {
            let running = e.state == "on";
            said(if running { "Running" } else { "Script" }, if running { "on" } else { "muted" }, "play", vec!["run"], json!({"running": running}))
        }
        "sensor" => sensor(e),
        "binary_sensor" => binary(e),
        "climate" => climate(e, unit),
        "cover" => cover(e),
        "lock" => {
            let (words, tone, icon) = match e.state.as_str() {
                "locked" => ("Locked", "ok", "lock"),
                "unlocked" => ("Unlocked", "warn", "lock-open"),
                "open" => ("Open", "warn", "lock-open"),
                "locking" => ("Locking", "info", "lock"),
                "unlocking" => ("Unlocking", "info", "lock-open"),
                "jammed" => ("Jammed", "alert", "lock"),
                _ => ("Unknown", "muted", "lock"),
            };
            said(words, tone, icon, vec![], json!({}))
        }
        "media_player" => media(e),
        _ => said(title_case(&e.state), "muted", "activity", vec![], json!({})),
    }
}

fn light(e: &Ent) -> Said {
    let (on, tone) = on_off(&e.state);
    let modes = e.list("supported_color_modes");
    let dimmable = if modes.is_empty() { e.attrs.contains_key("brightness") } else { modes.iter().any(|m| *m != "onoff") };
    let level = e.num("brightness").map(pct);
    // the integration sends no range: the usual one for a white lamp, inside what its `turn_on` takes (1000 to 12000)
    let kelvin = modes.contains(&"color_temp").then(|| json!({"now": e.num("color_temp_kelvin").map(|k| k.round() as i64), "min": KELVIN.0, "max": KELVIN.1}));
    let words = match (on, dimmable, level) {
        (true, true, Some(p)) => format!("On \u{b7} {p}%"),
        (true, _, _) => "On".into(),
        _ => "Off".into(),
    };
    said(words, tone, "lightbulb", vec!["toggle"], json!({"on": on, "dimmable": dimmable, "brightness": if on { level } else { None }, "kelvin": kelvin}))
}

fn sensor(e: &Ent) -> Said {
    let class = e.text("device_class").unwrap_or("");
    let icon = match class {
        "temperature" => "thermometer",
        "humidity" | "moisture" => "droplets",
        "power" | "energy" | "current" | "voltage" | "apparent_power" | "power_factor" => "zap",
        "battery" => "battery",
        "pm1" | "pm25" | "pm10" | "aqi" | "co2" | "carbon_dioxide" | "carbon_monoxide" | "volatile_organic_compounds" | "volatile_organic_compounds_parts" | "nitrogen_dioxide" | "ozone" => "wind",
        "illuminance" => "sun",
        "pressure" | "atmospheric_pressure" => "gauge",
        _ => "activity",
    };
    if e.gone() {
        return said("Unavailable", "muted", icon, vec![], json!({}));
    }
    let unit = e.text("unit_of_measurement").unwrap_or("");
    let words = match e.state.parse::<f64>() {
        Ok(v) if v.is_finite() => with_unit(&number(v), unit),
        _ if matches!(class, "timestamp" | "date") => "\u{2014}".into(),
        _ => clip(&title_case(&e.state), 24),
    };
    said(words, "info", icon, vec![], json!({}))
}

fn binary(e: &Ent) -> Said {
    let class = e.text("device_class").unwrap_or("");
    let on = e.state == "on";
    // (words when on, words when off, glyph when on, glyph when off, whether on is something to look at)
    let (yes, no, icon_on, icon_off, alert): (&str, &str, &str, &str, &str) = match class {
        "door" | "garage_door" | "opening" | "window" => ("Open", "Closed", "door-open", "door-closed", ""),
        "motion" | "occupancy" | "presence" | "moving" => (
            match class {
                "motion" => "Motion",
                "occupancy" => "Occupied",
                "presence" => "Home",
                _ => "Moving",
            },
            match class {
                "presence" => "Away",
                _ => "Clear",
            },
            "footprints",
            "footprints",
            "",
        ),
        "moisture" => ("Wet", "Dry", "droplets", "droplets", "alert"),
        "smoke" => ("Smoke", "Clear", "triangle-alert", "triangle-alert", "alert"),
        "gas" | "carbon_monoxide" => ("Detected", "Clear", "triangle-alert", "triangle-alert", "alert"),
        "problem" | "safety" | "tamper" => ("Problem", "OK", "triangle-alert", "triangle-alert", "warn"),
        "connectivity" => ("Connected", "Disconnected", "wifi", "wifi", ""),
        "battery" => ("Low", "OK", "battery", "battery", "warn"),
        "lock" => ("Unlocked", "Locked", "lock-open", "lock", "warn"),
        "plug" | "power" => ("Plugged in", "Unplugged", "plug", "plug", ""),
        "light" => ("Light", "Dark", "sun", "sun", ""),
        "sound" | "vibration" => ("Detected", "Clear", "activity", "activity", ""),
        "running" => ("Running", "Stopped", "activity", "activity", ""),
        _ => ("On", "Off", "activity", "activity", ""),
    };
    if e.gone() {
        return said("Unavailable", "muted", icon_off, vec![], json!({}));
    }
    let tone = match (on, alert) {
        (true, "alert") => "alert",
        (true, "warn") => "warn",
        (true, _) => "on",
        (false, _) => "off",
    };
    said(if on { yes } else { no }, tone, if on { icon_on } else { icon_off }, vec![], json!({"on": on}))
}

fn mode_word(mode: &str) -> String {
    match mode {
        "heat_cool" => "Heat/Cool".into(),
        "fan_only" => "Fan".into(),
        other => title_case(other),
    }
}

/// The unit of a thermostat's numbers: the one the house's temperature sensors use, else Fahrenheit for a number that no room is in
/// Celsius at (the integration sends no unit for a thermostat).
fn temp_unit<'a>(unit: &'a str, e: &Ent) -> &'a str {
    if !unit.is_empty() {
        return unit;
    }
    let seen = [e.num("current_temperature"), e.num("temperature"), e.num("target_temp_low"), e.num("target_temp_high")];
    if seen.iter().flatten().any(|v| *v >= 45.0) { "\u{b0}F" } else { "\u{b0}C" }
}

fn climate(e: &Ent, unit: &str) -> Said {
    let deg = temp_unit(unit, e);
    let fahrenheit = deg.ends_with('F');
    let degrees = |v: f64| format!("{}\u{b0}", number(v));
    let current = e.num("current_temperature");
    let target = e.num("temperature");
    let (low, high) = (e.num("target_temp_low"), e.num("target_temp_high"));
    let action = e.text("hvac_action").unwrap_or("");
    let modes: Vec<&str> = e.list("hvac_modes");
    let tone = match action {
        "heating" => "warn",
        "cooling" => "info",
        _ if e.state == "off" => "off",
        _ => "on",
    };
    let ctl = json!({
        "mode": e.state, "modes": modes, "current": current, "target": target, "low": low, "high": high,
        "min": e.num("min_temp").unwrap_or(if fahrenheit { 50.0 } else { 10.0 }), "max": e.num("max_temp").unwrap_or(if fahrenheit { 90.0 } else { 32.0 }),
        "step": e.num("target_temp_step").unwrap_or(if fahrenheit { 1.0 } else { 0.5 }), "unit": deg, "action": action,
    });
    if e.gone() {
        return said("Unavailable", "muted", "thermometer", vec![], json!({}));
    }
    let now = current.map(degrees);
    let words = match (e.state.as_str(), now, target, low.zip(high)) {
        ("off", Some(now), ..) => format!("Off \u{b7} {now}"),
        ("off", None, ..) => "Off".into(),
        (_, now, _, Some((l, h))) => format!("{}{} to {}", now.map(|n| format!("{n} \u{b7} ")).unwrap_or_default(), degrees(l), degrees(h)),
        (mode, now, Some(t), None) => {
            let verb = match action {
                "heating" => "Heating to",
                "cooling" => "Cooling to",
                _ => "Set to",
            };
            let _ = mode;
            format!("{}{verb} {}", now.map(|n| format!("{n} \u{b7} ")).unwrap_or_default(), degrees(t))
        }
        (mode, Some(now), _, None) => format!("{now} \u{b7} {}", mode_word(mode)),
        (mode, None, _, None) => mode_word(mode),
    };
    said(words, tone, "thermometer", vec!["sheet"], ctl)
}

fn cover(e: &Ent) -> Said {
    let position = e.num("current_position").map(|p| p.round() as i64);
    let words = match (e.state.as_str(), position) {
        ("opening", _) => "Opening".to_owned(),
        ("closing", _) => "Closing".to_owned(),
        ("closed", _) => "Closed".to_owned(),
        ("open", Some(p)) if p < 100 => format!("Open \u{b7} {p}%"),
        ("open", _) => "Open".to_owned(),
        (other, _) => title_case(other),
    };
    if e.gone() {
        return said("Unavailable", "muted", "blinds", vec![], json!({}));
    }
    let tone = if e.state == "closed" { "off" } else { "on" };
    said(words, tone, "blinds", vec!["sheet"], json!({"state": e.state, "position": position}))
}

fn media(e: &Ent) -> Said {
    let icon = match e.text("device_class") {
        Some("tv") => "tv",
        Some("speaker" | "receiver") => "speaker",
        _ => "music",
    };
    let title = e.text("media_title").map(|t| clip(t, 40));
    let words = match (e.state.as_str(), title) {
        ("playing", Some(t)) => format!("Playing \u{b7} {t}"),
        ("playing", None) => "Playing".into(),
        ("paused", Some(t)) => format!("Paused \u{b7} {t}"),
        (other, _) => title_case(other),
    };
    let tone = if e.state == "playing" { "on" } else { "off" };
    said(words, tone, icon, vec![], json!({"artist": e.text("media_artist"), "app": e.text("app_name"), "source": e.text("source")}))
}

// -- the tile ----------------------------------------------------------------------------------------------------------------------

/// How far Home Assistant can be heard now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    Live,
    /// Connecting, or not reachable: whatever was known is not said as if it were current.
    Waiting(&'static str),
}

/// Whether what the tile offers the OK key is something Home Assistant lets the TV do now.
fn offered(verb: &str, ctl: &[String]) -> bool {
    let has = |v: &str| ctl.iter().any(|c| c == v);
    match verb {
        "toggle" => has("toggle") || (has("turn_on") && has("turn_off")),
        "run" => has("turn_on"),
        "sheet" => !ctl.is_empty(),
        _ => false,
    }
}

/// The tile of one shared entity: `ent` is what is known of it (`None`: Home Assistant no longer shares it), `ctl` the services Home
/// Assistant lets the TV call on it now (empty: shown, not controlled).
pub fn tile(id: &str, name: Option<&str>, area: Option<&str>, ent: Option<&Ent>, ctl: &[String], unit: &str, reach: Reach) -> Value {
    let domain = domain_of(id).unwrap_or("");
    let group = Group::of_domain(domain);
    let name = name.map(str::to_owned).or_else(|| ent.and_then(|e| e.name().map(str::to_owned))).unwrap_or_else(|| id.to_owned());
    let mut tile = json!({"id": id, "name": name, "area": area, "group": group.map(Group::id)});
    let (words, tone, icon, can, ctl_doc, available) = match (reach, ent) {
        (Reach::Waiting(why), _) => (why.to_owned(), "muted", say(domain, &Ent::default(), unit).icon, Vec::new(), json!({}), false),
        (Reach::Live, None) => ("Not shared any more".to_owned(), "muted", say(domain, &Ent::default(), unit).icon, Vec::new(), json!({}), false),
        (Reach::Live, Some(e)) => {
            let mut s = say(domain, e, unit);
            let available = !e.gone();
            if !available {
                s.words = "Unavailable".into();
                s.tone = "muted";
            }
            let can: Vec<&'static str> = if available && !barred(domain, id, e) { s.can.into_iter().filter(|v| offered(v, ctl)).collect() } else { Vec::new() };
            (s.words, s.tone, s.icon, can, s.ctl, available)
        }
    };
    let map = tile.as_object_mut().expect("an object");
    map.insert("words".into(), json!(words));
    map.insert("tone".into(), json!(tone));
    map.insert("icon".into(), json!(icon));
    map.insert("available".into(), json!(available));
    map.insert("can".into(), json!(can));
    map.insert("verbs".into(), json!(if can.is_empty() { &[][..] } else { ctl }));
    map.insert("ctl".into(), ctl_doc);
    tile
}

/// A short word for a candidate in the picker: what it is doing now, in the same words the tile uses.
pub fn glance(id: &str, ent: &Ent, unit: &str) -> String {
    say(domain_of(id).unwrap_or(""), ent, unit).words
}

pub fn icon_of(id: &str, ent: &Ent, unit: &str) -> &'static str {
    say(domain_of(id).unwrap_or(""), ent, unit).icon
}

// -- what may be asked ---------------------------------------------------------------------------------------------------------------

/// A service call, whole: the service, and the plain values it is given. The target is always the one picked entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub domain: &'static str,
    pub service: &'static str,
    pub data: Map<String, Value>,
}

/// Why a request is not made, in words for the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The thing is not on Home, or its domain is not one Home carries.
    NotShown,
    /// Home Assistant cannot reach it now.
    Unavailable,
    /// The TV never does this (a door, an alarm, a camera) or this thing offers no such control.
    NotAllowed(&'static str),
    /// The value is not one this control takes.
    Invalid(&'static str),
}

impl Refused {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotShown => "not_shown",
            Self::Unavailable => "unavailable",
            Self::NotAllowed(_) => "not_allowed",
            Self::Invalid(_) => "invalid",
        }
    }

    pub fn words(&self) -> String {
        match self {
            Self::NotShown => "That is not on Home".into(),
            Self::Unavailable => "Home Assistant cannot reach it right now".into(),
            Self::NotAllowed(why) | Self::Invalid(why) => (*why).into(),
        }
    }
}

const NOT_FROM_THE_TV: &str = "Doors, alarms and cameras are not operated from the TV";

/// Words that make a switch, a script or a scene one the TV does not run: it unlocks, disarms or switches off a camera or an alarm.
const HUMAN_ONLY: [&str; 11] = ["lock", "unlock", "locks", "disarm", "alarm", "siren", "camera", "cameras", "garage", "gate", "deadbolt"];

fn human_only(id: &str, name: &str) -> bool {
    let hay = format!("{} {}", id.replace('.', " "), name).to_lowercase();
    hay.split(|c: char| !c.is_alphanumeric()).any(|w| HUMAN_ONLY.contains(&w) || w.contains("unlock") || w.contains("disarm"))
}

fn cover_ok(id: &str, e: &Ent) -> bool {
    matches!(e.text("device_class"), None | Some("blind" | "curtain" | "shade" | "shutter" | "awning" | "damper")) && !human_only(id, e.name().unwrap_or(""))
}

/// Things the TV does not operate: a switch, script or scene that unlocks or disarms, and a cover that is a garage door, a gate or a door.
fn barred(domain: &str, id: &str, e: &Ent) -> bool {
    match domain {
        "switch" | "input_boolean" | "script" | "scene" | "fan" => human_only(id, e.name().unwrap_or("")),
        "cover" => !cover_ok(id, e),
        _ => false,
    }
}

fn call(domain: &'static str, service: &'static str, data: &[(&str, Value)]) -> Result<Call, Refused> {
    Ok(Call { domain, service, data: data.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect() })
}

fn whole(value: &Value, lo: f64, hi: f64, why: &'static str) -> Result<i64, Refused> {
    match value.as_f64() {
        Some(v) if v.is_finite() && v >= lo && v <= hi => Ok(v.round() as i64),
        _ => Err(Refused::Invalid(why)),
    }
}

fn degrees(value: &Value, e: &Ent, unit: &str) -> Result<f64, Refused> {
    let fahrenheit = temp_unit(unit, e).ends_with('F');
    let lo = e.num("min_temp").unwrap_or(if fahrenheit { 50.0 } else { 10.0 });
    let hi = e.num("max_temp").unwrap_or(if fahrenheit { 90.0 } else { 32.0 });
    match value.as_f64() {
        Some(v) if v.is_finite() && v >= lo && v <= hi => Ok((v * 10.0).round() / 10.0),
        _ => Err(Refused::Invalid("That temperature is out of range")),
    }
}

/// The call that `verb` (with `value`) comes to for the thing `id`, in the state `e`; or why not. `ctl` is what Home Assistant said the TV
/// may do to it now (its services): a call outside it is not made, however the table reads.
pub fn plan(id: &str, e: &Ent, ctl: &[String], unit: &str, verb: &str, value: &Value) -> Result<Call, Refused> {
    let call = planned(id, e, unit, verb, value)?;
    if ctl.iter().any(|v| v == call.service) {
        Ok(call)
    } else {
        Err(Refused::NotAllowed("Home Assistant does not let the TV do that"))
    }
}

fn planned(id: &str, e: &Ent, unit: &str, verb: &str, value: &Value) -> Result<Call, Refused> {
    let domain = domain_of(id).ok_or(Refused::NotShown)?;
    if matches!(domain, "lock" | "sensor" | "binary_sensor" | "media_player") {
        return Err(Refused::NotAllowed(if domain == "lock" { NOT_FROM_THE_TV } else { "That is shown, not controlled" }));
    }
    if e.gone() {
        return Err(Refused::Unavailable);
    }
    if barred(domain, id, e) {
        return Err(Refused::NotAllowed(NOT_FROM_THE_TV));
    }
    let no = Refused::NotAllowed("That thing has no such control");
    match (domain, verb) {
        ("light", "toggle") => call("light", "toggle", &[]),
        ("light", "on") => call("light", "turn_on", &[]),
        ("light", "off") => call("light", "turn_off", &[]),
        ("light", "brightness") => match whole(value, 0.0, 100.0, "Brightness is 0 to 100")? {
            0 => call("light", "turn_off", &[]),
            p => call("light", "turn_on", &[("brightness_pct", json!(p))]),
        },
        ("light", "kelvin") => {
            if !e.list("supported_color_modes").contains(&"color_temp") {
                return Err(no);
            }
            call("light", "turn_on", &[("color_temp_kelvin", json!(whole(value, KELVIN.0 as f64, KELVIN.1 as f64, "That colour temperature is out of range")?))])
        }
        ("switch", "toggle") => call("switch", "toggle", &[]),
        ("switch", "on") => call("switch", "turn_on", &[]),
        ("switch", "off") => call("switch", "turn_off", &[]),
        ("input_boolean", "toggle") => call("input_boolean", "toggle", &[]),
        ("input_boolean", "on") => call("input_boolean", "turn_on", &[]),
        ("input_boolean", "off") => call("input_boolean", "turn_off", &[]),
        ("fan", "toggle") => call("fan", "toggle", &[]),
        ("fan", "on") => call("fan", "turn_on", &[]),
        ("fan", "off") => call("fan", "turn_off", &[]),
        ("fan", "percentage") => call("fan", "set_percentage", &[("percentage", json!(whole(value, 0.0, 100.0, "Speed is 0 to 100")?))]),
        ("scene", "run") => call("scene", "turn_on", &[]),
        ("script", "run") => call("script", "turn_on", &[]),
        ("climate", "hvac_mode") => {
            let mode = value.as_str().ok_or(Refused::Invalid("That mode is not one this thermostat has"))?;
            if !e.list("hvac_modes").contains(&mode) {
                return Err(Refused::Invalid("That mode is not one this thermostat has"));
            }
            call("climate", "set_hvac_mode", &[("hvac_mode", json!(mode))])
        }
        ("climate", "temperature") => call("climate", "set_temperature", &[("temperature", json!(degrees(value, e, unit)?))]),
        ("climate", "range") => {
            let (low, high) = (degrees(&value["low"], e, unit)?, degrees(&value["high"], e, unit)?);
            if low > high {
                return Err(Refused::Invalid("The low is above the high"));
            }
            call("climate", "set_temperature", &[("target_temp_low", json!(low)), ("target_temp_high", json!(high))])
        }
        ("cover", verb) => {
            match verb {
                "open" => call("cover", "open_cover", &[]),
                "close" => call("cover", "close_cover", &[]),
                "stop" => call("cover", "stop_cover", &[]),
                "position" => call("cover", "set_cover_position", &[("position", json!(whole(value, 0.0, 100.0, "Position is 0 to 100")?))]),
                _ => Err(no),
            }
        }
        _ => Err(no),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(state: &str, attrs: Value) -> Ent {
        Ent { state: state.into(), attrs: attrs.as_object().cloned().unwrap_or_default() }
    }

    /// Every service a thing of the domain has: what a Home Assistant that shares and controls it says.
    fn all_ctl() -> Vec<String> {
        ["turn_on", "turn_off", "toggle", "set_percentage", "set_temperature", "set_hvac_mode", "open_cover", "close_cover", "stop_cover", "set_cover_position"].map(String::from).to_vec()
    }

    fn live(id: &str, e: &Ent) -> Value {
        tile(id, None, Some("Kitchen"), Some(e), &all_ctl(), "\u{b0}F", Reach::Live)
    }

    #[test]
    fn ids_are_plain_and_of_a_domain_home_carries() {
        assert_eq!(domain_of("light.lamp"), Some("light"));
        assert_eq!(domain_of("camera.door"), None);
        assert_eq!(domain_of("alarm_control_panel.house"), None);
        assert_eq!(domain_of("light.Lamp"), None);
        assert_eq!(domain_of("light."), None);
        assert_eq!(domain_of("lamp"), None);
        assert_eq!(Group::of_domain("fan"), Some(Group::Switches));
        assert_eq!(Group::of_id("scenes"), Some(Group::Scenes));
    }

    #[test]
    fn numbers_are_written_as_a_person_writes_them() {
        assert_eq!((number(21.0), number(21.54), number(0.04), number(-3.26)), ("21".into(), "21.5".into(), "0".into(), "-3.3".into()));
        assert_eq!((with_unit("21", "\u{b0}C"), with_unit("45", "%"), with_unit("120", "W")), ("21\u{b0}C".into(), "45%".into(), "120 W".into()));
    }

    #[test]
    fn a_light_says_on_off_and_its_level_and_offers_the_controls_it_has() {
        let t = live("light.lamp", &ent("on", json!({"brightness": 128, "supported_color_modes": ["color_temp", "xy"], "color_temp_kelvin": 3000})));
        assert_eq!((t["words"].as_str(), t["tone"].as_str(), t["icon"].as_str()), (Some("On \u{b7} 50%"), Some("on"), Some("lightbulb")));
        assert_eq!(t["can"], json!(["toggle"]));
        assert_eq!(t["ctl"]["brightness"], json!(50));
        assert_eq!(t["ctl"]["kelvin"], json!({"now": 3000, "min": 2000, "max": 6500}));
        let off = live("light.lamp", &ent("off", json!({"supported_color_modes": ["onoff"]})));
        assert_eq!((off["words"].as_str(), off["ctl"]["dimmable"].as_bool()), (Some("Off"), Some(false)));
        // a lamp that is only a switch has no level, whatever its attributes say
        let plain = live("light.relay", &ent("on", json!({"supported_color_modes": ["onoff"]})));
        assert_eq!(plain["words"], json!("On"));
    }

    #[test]
    fn a_thing_that_is_unavailable_says_so_and_offers_nothing() {
        let t = live("switch.pump", &ent("unavailable", json!({})));
        assert_eq!((t["words"].as_str(), t["available"].as_bool(), t["can"].clone()), (Some("Unavailable"), Some(false), json!([])));
        let s = live("sensor.t", &ent("unknown", json!({"device_class": "temperature"})));
        assert_eq!((s["words"].as_str(), s["icon"].as_str()), (Some("Unavailable"), Some("thermometer")));
        let gone = tile("light.old", None, None, None, &[], "", Reach::Live);
        assert_eq!((gone["words"].as_str(), gone["available"].as_bool()), (Some("Not shared any more"), Some(false)));
        let off = tile("light.lamp", Some("Lamp"), None, Some(&ent("on", json!({}))), &all_ctl(), "", Reach::Waiting("Not reachable"));
        assert_eq!((off["words"].as_str(), off["can"].clone(), off["name"].as_str()), (Some("Not reachable"), json!([]), Some("Lamp")));
    }

    #[test]
    fn sensors_say_their_value_and_binary_sensors_a_word() {
        let t = live("sensor.temp", &ent("21.54", json!({"device_class": "temperature", "unit_of_measurement": "\u{b0}C"})));
        assert_eq!(t["words"], json!("21.5\u{b0}C"));
        let p = live("sensor.power", &ent("1200", json!({"device_class": "power", "unit_of_measurement": "W"})));
        assert_eq!((p["words"].as_str(), p["icon"].as_str()), (Some("1200 W"), Some("zap")));
        let d = live("binary_sensor.door", &ent("on", json!({"device_class": "door"})));
        assert_eq!((d["words"].as_str(), d["icon"].as_str(), d["can"].clone()), (Some("Open"), Some("door-open"), json!([])));
        let c = live("binary_sensor.door", &ent("off", json!({"device_class": "door"})));
        assert_eq!((c["words"].as_str(), c["icon"].as_str(), c["tone"].as_str()), (Some("Closed"), Some("door-closed"), Some("off")));
        let m = live("binary_sensor.hall", &ent("on", json!({"device_class": "motion"})));
        assert_eq!(m["words"], json!("Motion"));
        let w = live("binary_sensor.leak", &ent("on", json!({"device_class": "moisture"})));
        assert_eq!((w["words"].as_str(), w["tone"].as_str()), (Some("Wet"), Some("alert")));
        let text = live("sensor.mode", &ent("away_mode", json!({})));
        assert_eq!(text["words"], json!("Away mode"));
    }

    #[test]
    fn climate_says_the_room_and_the_set_point() {
        let t = live("climate.hall", &ent("heat", json!({"current_temperature": 68.0, "temperature": 70.0, "hvac_action": "heating", "hvac_modes": ["off", "heat", "cool"], "min_temp": 50, "max_temp": 90})));
        assert_eq!((t["words"].as_str(), t["tone"].as_str(), t["can"].clone()), (Some("68\u{b0} \u{b7} Heating to 70\u{b0}"), Some("warn"), json!(["sheet"])));
        assert_eq!(t["ctl"]["modes"], json!(["off", "heat", "cool"]));
        let range = live("climate.hall", &ent("heat_cool", json!({"current_temperature": 71.5, "target_temp_low": 68.0, "target_temp_high": 75.0, "hvac_action": "idle"})));
        assert_eq!(range["words"], json!("71.5\u{b0} \u{b7} 68\u{b0} to 75\u{b0}"));
        let off = live("climate.hall", &ent("off", json!({"current_temperature": 70.0})));
        assert_eq!((off["words"].as_str(), off["tone"].as_str()), (Some("Off \u{b7} 70\u{b0}"), Some("off")));
    }

    #[test]
    fn covers_locks_and_players_are_said_in_words() {
        let c = live("cover.blind", &ent("open", json!({"current_position": 60})));
        assert_eq!((c["words"].as_str(), c["can"].clone()), (Some("Open \u{b7} 60%"), json!(["sheet"])));
        assert_eq!(live("cover.blind", &ent("closed", json!({"current_position": 0})))["words"], json!("Closed"));
        let l = live("lock.front", &ent("unlocked", json!({})));
        assert_eq!((l["words"].as_str(), l["tone"].as_str(), l["can"].clone()), (Some("Unlocked"), Some("warn"), json!([])));
        assert_eq!(live("lock.front", &ent("jammed", json!({})))["tone"], json!("alert"));
        let m = live("media_player.tv", &ent("playing", json!({"media_title": "The Expanse", "device_class": "tv"})));
        assert_eq!((m["words"].as_str(), m["icon"].as_str(), m["can"].clone()), (Some("Playing \u{b7} The Expanse"), Some("tv"), json!([])));
        assert_eq!(live("media_player.tv", &ent("off", json!({})))["words"], json!("Off"));
        assert_eq!(live("scene.movie", &ent("2026-10-30T01:00:00+00:00", json!({})))["can"], json!(["run"]));
    }

    #[test]
    fn what_is_kept_of_a_state_is_short_and_plain() {
        let all = json!({"friendly_name": "Lamp", "brightness": 10, "entity_picture": "/x", "supported_color_modes": ["xy"], "effect_list": ["a"], "hvac_modes": ["off", {"no": 1}], "media_title": "t".repeat(200)});
        let k = kept(all.as_object().unwrap());
        assert_eq!(k.keys().map(String::as_str).collect::<Vec<_>>(), ["brightness", "friendly_name", "supported_color_modes"]);
    }

    fn plan_of(id: &str, state: &str, attrs: Value, verb: &str, value: Value) -> Result<Call, Refused> {
        plan(id, &ent(state, attrs), &all_ctl(), "\u{b0}F", verb, &value)
    }

    #[test]
    fn a_light_is_asked_for_its_brightness_and_colour_temperature_in_range() {
        let attrs = json!({"supported_color_modes": ["color_temp", "xy"]});
        let c = plan_of("light.l", "on", attrs.clone(), "brightness", json!(40)).unwrap();
        assert_eq!((c.domain, c.service, c.data["brightness_pct"].clone()), ("light", "turn_on", json!(40)));
        assert_eq!(plan_of("light.l", "on", attrs.clone(), "brightness", json!(0)).unwrap().service, "turn_off");
        assert_eq!(plan_of("light.l", "on", attrs.clone(), "brightness", json!(101)), Err(Refused::Invalid("Brightness is 0 to 100")));
        assert_eq!(plan_of("light.l", "on", attrs.clone(), "kelvin", json!(2700)).unwrap().data["color_temp_kelvin"], json!(2700));
        assert!(plan_of("light.l", "on", attrs, "kelvin", json!(9000)).is_err());
        assert!(plan_of("light.l", "on", json!({"supported_color_modes": ["xy"]}), "kelvin", json!(2700)).is_err(), "a light with no colour temperature has no such control");
        assert_eq!(plan_of("light.l", "unavailable", json!({}), "toggle", json!(null)), Err(Refused::Unavailable));
    }

    #[test]
    fn switches_fans_scenes_and_scripts_have_their_services() {
        assert_eq!(plan_of("switch.s", "off", json!({}), "toggle", json!(null)).unwrap().service, "toggle");
        assert_eq!(plan_of("input_boolean.b", "off", json!({}), "on", json!(null)).unwrap().service, "turn_on");
        assert_eq!(plan_of("fan.f", "on", json!({}), "percentage", json!(66)).unwrap().data["percentage"], json!(66));
        assert_eq!(plan_of("scene.movie", "scening", json!({}), "run", json!(null)).unwrap().domain, "scene");
        assert_eq!(plan_of("script.goodnight", "off", json!({}), "run", json!(null)).unwrap().service, "turn_on");
        assert!(plan_of("scene.movie", "x", json!({}), "toggle", json!(null)).is_err());
    }

    #[test]
    fn climate_takes_a_mode_it_has_and_a_temperature_in_range() {
        let attrs = json!({"hvac_modes": ["off", "heat"]});
        assert_eq!(plan_of("climate.c", "off", attrs.clone(), "hvac_mode", json!("heat")).unwrap().data["hvac_mode"], json!("heat"));
        assert!(plan_of("climate.c", "off", attrs.clone(), "hvac_mode", json!("cool")).is_err());
        assert_eq!(plan_of("climate.c", "heat", attrs.clone(), "temperature", json!(71.26)).unwrap().data["temperature"], json!(71.3));
        assert!(plan_of("climate.c", "heat", attrs.clone(), "temperature", json!(120)).is_err());
        let r = plan_of("climate.c", "heat_cool", attrs.clone(), "range", json!({"low": 66, "high": 74})).unwrap();
        assert_eq!((r.data["target_temp_low"].clone(), r.data["target_temp_high"].clone()), (json!(66.0), json!(74.0)));
        assert!(plan_of("climate.c", "heat_cool", attrs, "range", json!({"low": 80, "high": 70})).is_err());
    }

    #[test]
    fn a_blind_opens_and_closes_but_a_garage_door_and_a_gate_do_not() {
        assert_eq!(plan_of("cover.living_room_curtain", "open", json!({"device_class": "curtain"}), "close", json!(null)).unwrap().service, "close_cover");
        assert_eq!(plan_of("cover.blinds", "open", json!({"friendly_name": "Main Blinds"}), "position", json!(30)).unwrap().data["position"], json!(30));
        assert_eq!(plan_of("cover.blinds", "open", json!({}), "stop", json!(null)).unwrap().service, "stop_cover");
        for (id, attrs) in [("cover.garage", json!({"device_class": "garage"})), ("cover.x", json!({"device_class": "gate"})), ("cover.x", json!({"device_class": "door"})), ("cover.x", json!({"device_class": "window"})), ("cover.opener", json!({"friendly_name": "Garage Opener"}))] {
            assert_eq!(plan_of(id, "closed", attrs, "open", json!(null)), Err(Refused::NotAllowed(NOT_FROM_THE_TV)), "{id}");
        }
    }

    #[test]
    fn nothing_is_ever_asked_of_a_lock_an_alarm_a_siren_a_camera_or_a_valve() {
        for verb in ["lock", "unlock", "open", "toggle", "run", "on", "off"] {
            assert_eq!(plan_of("lock.front_door", "locked", json!({}), verb, json!(null)), Err(Refused::NotAllowed(NOT_FROM_THE_TV)));
        }
        for id in ["alarm_control_panel.house", "siren.hall", "camera.porch", "valve.water", "button.press", "automation.x", "vacuum.v"] {
            assert_eq!(plan_of(id, "on", json!({}), "toggle", json!(null)), Err(Refused::NotShown), "{id}");
        }
        // sensors and players are shown, not controlled
        assert!(plan_of("sensor.t", "1", json!({}), "toggle", json!(null)).is_err());
        assert!(plan_of("media_player.tv", "on", json!({}), "on", json!(null)).is_err());
    }

    #[test]
    fn a_switch_a_script_or_a_scene_that_unlocks_disarms_or_silences_a_camera_is_not_run_from_the_tv() {
        for (id, name) in [
            ("script.unlock_front_door", "Front door"),
            ("script.x", "Unlock the back door"),
            ("scene.x", "Disarm the house"),
            ("switch.porch_camera_power", "Porch camera"),
            ("switch.x", "Siren"),
            ("input_boolean.x", "Alarm armed"),
            ("script.garage_open", "Open garage"),
            ("script.lock_all", "Lock all doors"),
        ] {
            let e = ent("off", json!({"friendly_name": name}));
            let verb = if id.starts_with("script") || id.starts_with("scene") { "run" } else { "toggle" };
            assert_eq!(plan(id, &e, &all_ctl(), "", verb, &json!(null)), Err(Refused::NotAllowed(NOT_FROM_THE_TV)), "{id} {name}");
        }
        // lookalikes are fine
        for (id, name) in [("switch.clock_radio", "Clock radio"), ("script.movie_night", "Movie night"), ("switch.block_heater", "Block heater")] {
            assert!(plan(id, &ent("off", json!({"friendly_name": name})), &all_ctl(), "", if id.starts_with("script") { "run" } else { "toggle" }, &json!(null)).is_ok(), "{id}");
        }
    }

    #[test]
    fn a_call_only_carries_the_plain_values_of_its_table() {
        // the values every row can carry, and the keys the lease's own list of call values has (so a call is one the Show would make)
        let allowed = ["brightness_pct", "color_temp_kelvin", "percentage", "temperature", "target_temp_low", "target_temp_high", "hvac_mode", "position"];
        let all = [
            plan_of("light.l", "on", json!({}), "brightness", json!(10)),
            plan_of("light.l", "on", json!({"supported_color_modes": ["color_temp"]}), "kelvin", json!(3000)),
            plan_of("fan.f", "on", json!({}), "percentage", json!(10)),
            plan_of("climate.c", "heat", json!({"hvac_modes": ["heat"]}), "hvac_mode", json!("heat")),
            plan_of("climate.c", "heat", json!({}), "range", json!({"low": 60, "high": 70})),
            plan_of("cover.c", "open", json!({}), "position", json!(10)),
        ];
        for c in all {
            assert!(c.unwrap().data.keys().all(|k| allowed.contains(&k.as_str())));
        }
    }

    #[test]
    fn what_home_assistant_does_not_let_the_tv_do_is_not_done_or_offered() {
        let e = ent("on", json!({"supported_color_modes": ["brightness"]}));
        // shared to look at: nothing is offered and nothing is planned
        let t = tile("light.l", None, None, Some(&e), &[], "", Reach::Live);
        assert_eq!((t["can"].clone(), t["verbs"].clone()), (json!([]), json!([])));
        assert_eq!(plan("light.l", &e, &[], "", "toggle", &json!(null)), Err(Refused::NotAllowed("Home Assistant does not let the TV do that")));
        // a service it did not list is not called: turn_on only, so a toggle is not made
        let only_on: Vec<String> = vec!["turn_on".into()];
        assert!(plan("light.l", &e, &only_on, "", "toggle", &json!(null)).is_err());
        assert_eq!(plan("light.l", &e, &only_on, "", "on", &json!(null)).unwrap().service, "turn_on");
        assert_eq!(tile("light.l", None, None, Some(&e), &only_on, "", Reach::Live)["can"], json!([]), "a light that can only be turned on has no toggle");
        let t = tile("scene.s", None, None, Some(&ent("x", json!({}))), &only_on, "", Reach::Live);
        assert_eq!(t["can"], json!(["run"]));
    }

    #[test]
    fn a_thermostat_without_a_unit_is_read_in_the_one_its_numbers_are_in() {
        let f = ent("heat", json!({"current_temperature": 68.0, "temperature": 70.0}));
        let c = ent("heat", json!({"current_temperature": 21.0, "temperature": 22.5}));
        assert_eq!(tile("climate.t", None, None, Some(&f), &all_ctl(), "", Reach::Live)["ctl"]["unit"], json!("\u{b0}F"));
        let t = tile("climate.t", None, None, Some(&c), &all_ctl(), "", Reach::Live);
        assert_eq!((t["ctl"]["unit"].clone(), t["ctl"]["step"].clone()), (json!("\u{b0}C"), json!(0.5)));
        assert_eq!(plan("climate.t", &c, &all_ctl(), "", "temperature", &json!(22)).unwrap().data["temperature"], json!(22.0));
        assert!(plan("climate.t", &c, &all_ctl(), "", "temperature", &json!(70)).is_err(), "70 is not a room in Celsius");
    }
}
