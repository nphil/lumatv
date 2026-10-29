//! What shows while the screensaver is up, by *source class* (`docs/research/saver-complications.md`, "What shows during the screensaver").
//!
//! Every thing that wants the screen has a class: the camera's video pop-up, what a camera saw, a doorbell, a timer, a Home Assistant or ntfy
//! notice, a system notice (a toast), the items a script put on the saver's board, and one class per installed plugin (`plugin:<id>`, so a
//! plugin that is installed tomorrow has its row tomorrow, with no code here). The owner picks one of four answers for a class:
//!
//! | Choice | While the saver is up |
//! |---|---|
//! | `over` | it shows over the saver, as it would over Home |
//! | `inside` | it is one dim line in the saver's own notification area; the saver stays |
//! | `wake` | the screen is woken (the saver ends) and it shows as usual |
//! | `hide` | it does not show; it stays in the notification drawer |
//!
//! Only the answer is stored (the helper keeps the owner's overrides in a file; a class never set has its default). A class lists the answers
//! that make sense for it (a live video cannot be a dim line; a timer ringing in the dark should not be hidden by an accident), and a stored
//! answer outside the list is ignored. The functions here are pure: the helper holds the state and the saver draws.

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// What a class does while the saver is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Over,
    Inside,
    Wake,
    Hide,
}

impl Choice {
    pub const ALL: [Choice; 4] = [Choice::Over, Choice::Inside, Choice::Wake, Choice::Hide];

    pub const fn id(self) -> &'static str {
        match self {
            Choice::Over => "over",
            Choice::Inside => "inside",
            Choice::Wake => "wake",
            Choice::Hide => "hide",
        }
    }

    pub fn parse(s: &str) -> Option<Choice> {
        Choice::ALL.into_iter().find(|c| c.id() == s)
    }

    /// The word of the answer on a row's value.
    pub const fn title(self) -> &'static str {
        match self {
            Choice::Over => "Over",
            Choice::Inside => "Inside",
            Choice::Wake => "Wake",
            Choice::Hide => "Hide",
        }
    }

    /// `[{id, glyph, title}]` for the four answers: what a settings page offers, so that it holds none of these words.
    pub fn answers() -> Value {
        Value::Array(Choice::ALL.iter().map(|c| json!({"id": c.id(), "glyph": c.glyph(), "title": c.title()})).collect())
    }

    /// The Lucide mark of the answer: the picture-in-picture frame, the moon, the sun, the closed eye.
    pub const fn glyph(self) -> &'static str {
        match self {
            Choice::Over => "picture-in-picture",
            Choice::Inside => "moon",
            Choice::Wake => "sun",
            Choice::Hide => "eye-off",
        }
    }
}

const ALL: &[Choice] = &Choice::ALL;
const NO_INSIDE: &[Choice] = &[Choice::Over, Choice::Wake, Choice::Hide];
const NO_OVER: &[Choice] = &[Choice::Inside, Choice::Wake, Choice::Hide];
const QUIET: &[Choice] = &[Choice::Over, Choice::Inside, Choice::Hide];
const ONLY_INSIDE_OR_HIDE: &[Choice] = &[Choice::Inside, Choice::Hide];

/// One class of the shipped table.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub id: &'static str,
    pub title: &'static str,
    pub glyph: &'static str,
    pub default: Choice,
    pub allowed: &'static [Choice],
}

/// The shipped classes, in the order the settings list them.
pub const CLASSES: &[Spec] = &[
    Spec {
        id: "doorbell",
        title: "Doorbell",
        glyph: "bell-ring",
        default: Choice::Wake,
        allowed: ALL,
    },
    Spec {
        id: "timer",
        title: "Timers and alarms",
        glyph: "timer",
        default: Choice::Wake,
        allowed: ALL,
    },
    Spec {
        id: "camera.popup",
        title: "Camera video",
        glyph: "video",
        default: Choice::Hide,
        allowed: NO_INSIDE,
    },
    Spec {
        id: "camera.still",
        title: "Camera events",
        glyph: "camera",
        default: Choice::Inside,
        allowed: NO_OVER,
    },
    Spec {
        id: "ha",
        title: "Home Assistant",
        glyph: "house-wifi",
        default: Choice::Inside,
        allowed: ALL,
    },
    Spec {
        id: "ntfy",
        title: "ntfy",
        glyph: "bell",
        default: Choice::Inside,
        allowed: ALL,
    },
    Spec {
        id: "toast",
        title: "System notices",
        glyph: "message-square-warning",
        default: Choice::Hide,
        allowed: QUIET,
    },
    Spec {
        id: "complication",
        title: "Saver items",
        glyph: "layout-grid",
        default: Choice::Inside,
        allowed: ONLY_INSIDE_OR_HIDE,
    },
];

/// The plugin ids that have a shipped class of their own: Home Assistant's and ntfy's.
const NAMED_PLUGINS: &[(&str, &str)] = &[("home-assistant", "ha"), ("ntfy", "ntfy")];

/// The prefix of a plugin's own class.
pub const PLUGIN_PREFIX: &str = "plugin:";
/// The answer of a plugin's class that was never set: its notices are one dim line in the saver.
pub const PLUGIN_DEFAULT: Choice = Choice::Inside;
/// The most overrides kept: a file that lists more is cut.
pub const MAX_OVERRIDES: usize = 64;

/// The class of a plugin's own.
pub fn plugin_class(plugin_id: &str) -> String {
    format!("{PLUGIN_PREFIX}{plugin_id}")
}

fn plugin_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_' || b == b'.')
}

/// An installed plugin, as the helper knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plugin {
    pub id: String,
    pub title: String,
    pub glyph: String,
}

// ---- classes of what arrives -----------------------------------------------------------------------------------------------------------------

/// The classes a card of the overlay queue is one of, most urgent first (`Overlays.js`).
pub const CARD_CLASSES: [&str; 5] = ["doorbell", "alarm", "timer", "alert", "score"];

fn base(name: &str) -> String {
    name.rsplit('/').next().unwrap_or("").to_lowercase()
}

fn is_one_of(s: &str, names: &[&str]) -> bool {
    names.contains(&s)
}

/// The class of a plugin's note as a card of the queue: an explicit `kind` when it has one, else by what the plugin called its channel and its
/// icon, else an alert.
pub fn card_class(kind: &str, channel: &str, channel_icon: &str) -> &'static str {
    if let Some(c) = CARD_CLASSES.iter().find(|c| **c == kind) {
        return c;
    }
    let (ch, icon) = (base(channel), base(channel_icon));
    if is_one_of(&ch, &["doorbell", "door-bell", "ring", "rings", "visitor"]) || is_one_of(&icon, &["doorbell", "bell-ring"]) {
        return "doorbell";
    }
    if is_one_of(&ch, &["alarm", "alarms", "wake", "wakeup"]) || is_one_of(&icon, &["alarm", "alarm-clock"]) {
        return "alarm";
    }
    if is_one_of(&ch, &["timer", "timers", "countdown"]) || is_one_of(&icon, &["timer", "hourglass"]) {
        return "timer";
    }
    if is_one_of(&ch, &["score", "scores", "sport", "sports", "game", "games", "match", "matches"]) || is_one_of(&icon, &["trophy", "score", "sports"]) {
        return "score";
    }
    "alert"
}

/// The source class of a plugin's note: a doorbell and a timer (an alarm is a timer here) are their own whoever sent them, else the plugin's
/// shipped class (Home Assistant, ntfy), else its own.
pub fn note_class(plugin_id: &str, card_class: &str) -> String {
    match card_class {
        "doorbell" => "doorbell".to_owned(),
        "timer" | "alarm" => "timer".to_owned(),
        _ => NAMED_PLUGINS.iter().find(|(p, _)| *p == plugin_id).map_or_else(|| plugin_class(plugin_id), |(_, c)| (*c).to_owned()),
    }
}

/// The class of a plugin's cards and complications: the plugin's shipped class, else its own.
pub fn plugin_source_class(plugin_id: &str) -> String {
    note_class(plugin_id, "alert")
}

/// The class of a camera's pop-up: a doorbell is the doorbell's, every other kind is the camera's.
pub fn camera_class(kind: &str) -> &'static str {
    if kind == "doorbell" { "doorbell" } else { "camera.popup" }
}

/// The class of what a camera saw once its video is withheld (a line for the saver's notification area, or a wake): the camera's.
pub const CAMERA_EVENT: &str = "camera.still";

// ---- the answers -----------------------------------------------------------------------------------------------------------------------------

fn spec(class: &str) -> Option<&'static Spec> {
    CLASSES.iter().find(|s| s.id == class)
}

/// The class's answers: the shipped list, or every one for a plugin's own.
pub fn allowed(class: &str) -> &'static [Choice] {
    spec(class).map_or(ALL, |s| s.allowed)
}

/// The class's default answer.
pub fn default_choice(class: &str) -> Choice {
    spec(class).map_or(PLUGIN_DEFAULT, |s| s.default)
}

/// Why a rule was not taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The class is not one of the table's and no plugin's.
    UnknownClass,
    /// The class does not offer that answer.
    NotOffered,
    UnknownChoice,
}

impl Refused {
    pub fn code(&self) -> &'static str {
        match self {
            Refused::UnknownClass => "unknown_class",
            Refused::NotOffered => "not_offered",
            Refused::UnknownChoice => "unknown_choice",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Refused::UnknownClass => "no such source class",
            Refused::NotOffered => "that class does not offer that answer",
            Refused::UnknownChoice => "the answer is over, inside, wake or hide",
        }
    }
}

/// The owner's overrides: only what differs from the defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    set: BTreeMap<String, Choice>,
}

impl Rules {
    /// What the class does: the owner's answer when it is one the class offers, else the default.
    pub fn choice(&self, class: &str) -> Choice {
        self.set.get(class).copied().filter(|c| allowed(class).contains(c)).unwrap_or_else(|| default_choice(class))
    }

    /// Takes an answer. The class must be the table's or a well formed `plugin:<id>`; the answer one the class offers. The default is not stored.
    pub fn set(&mut self, class: &str, choice: &str) -> Result<Choice, Refused> {
        let known = spec(class).is_some() || class.strip_prefix(PLUGIN_PREFIX).is_some_and(plugin_id_ok);
        if !known {
            return Err(Refused::UnknownClass);
        }
        let choice = Choice::parse(choice).ok_or(Refused::UnknownChoice)?;
        if !allowed(class).contains(&choice) {
            return Err(Refused::NotOffered);
        }
        if choice == default_choice(class) {
            self.set.remove(class);
        } else if self.set.contains_key(class) || self.set.len() < MAX_OVERRIDES {
            self.set.insert(class.to_owned(), choice);
        }
        Ok(choice)
    }

    /// Back to the default: one class, or all of them.
    pub fn reset(&mut self, class: Option<&str>) {
        match class {
            Some(c) => {
                self.set.remove(c);
            }
            None => self.set.clear(),
        }
    }

    pub fn is_default(&self) -> bool {
        self.set.is_empty()
    }

    /// What is stored: `{class: answer}` for the classes that differ from their default.
    pub fn to_json(&self) -> Value {
        Value::Object(self.set.iter().map(|(k, v)| (k.clone(), json!(v.id()))).collect::<Map<_, _>>())
    }

    /// What `to_json` wrote; anything that is not a class and one of its answers is left out.
    pub fn from_json(v: &Value) -> Rules {
        let mut rules = Rules::default();
        if let Some(map) = v.as_object() {
            for (class, choice) in map.iter().take(MAX_OVERRIDES * 2) {
                if let Some(choice) = choice.as_str() {
                    let _ = rules.set(class, choice);
                }
            }
        }
        rules
    }

    /// The rows of the settings: the shipped classes, then one per plugin that is not one of them; each `{id, title, glyph, choice, default,
    /// choices}`, `choice` and `default` an answer's id and `choices` the ids the class offers.
    pub fn rows(&self, plugins: &[Plugin]) -> Vec<Value> {
        let row = |id: &str, title: &str, glyph: &str| {
            json!({"id": id, "title": title, "glyph": glyph, "choice": self.choice(id).id(), "default": default_choice(id).id(),
                   "choices": allowed(id).iter().map(|c| c.id()).collect::<Vec<_>>()})
        };
        let mut rows: Vec<Value> = CLASSES.iter().map(|s| row(s.id, s.title, s.glyph)).collect();
        for p in plugins {
            if NAMED_PLUGINS.iter().any(|(id, _)| *id == p.id) || !plugin_id_ok(&p.id) {
                continue;
            }
            rows.push(row(&plugin_class(&p.id), &p.title, if p.glyph.is_empty() { "plug" } else { &p.glyph }));
        }
        rows
    }

    /// `{class: answer}` for every class the table and the plugins have: what the saver reads from the topic.
    pub fn effective(&self, plugins: &[Plugin]) -> Value {
        Value::Object(self.rows(plugins).into_iter().filter_map(|r| Some((r["id"].as_str()?.to_owned(), r["choice"].clone()))).collect())
    }
}

/// What the helper does with a thing of a class, given whether the saver is up. `choice` is the class's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// The saver is not up (or the answer is `over`): the thing goes out as it always did.
    Normal,
    /// The thing goes out, marked as allowed over the saver.
    OverSaver,
    /// A dim line for the saver's notification area; the thing itself does not go out.
    Inside,
    /// The screen is woken and the thing goes out as usual.
    Wake,
    /// The thing does not go out.
    Withheld,
}

/// The delivery of a thing whose class answers `choice` while the saver is `up`.
pub fn delivery(choice: Choice, up: bool) -> Delivery {
    if !up {
        return Delivery::Normal;
    }
    match choice {
        Choice::Over => Delivery::OverSaver,
        Choice::Inside => Delivery::Inside,
        Choice::Wake => Delivery::Wake,
        Choice::Hide => Delivery::Withheld,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plug(id: &str) -> Plugin {
        Plugin { id: id.into(), title: id.to_uppercase(), glyph: String::new() }
    }

    #[test]
    fn the_defaults_are_the_ones_nitin_asked_for() {
        let r = Rules::default();
        assert_eq!(r.choice("camera.popup"), Choice::Hide, "a video pop-up never covers the saver");
        assert_eq!(r.choice("doorbell"), Choice::Wake);
        assert_eq!(r.choice("ha"), Choice::Inside);
        assert_eq!(r.choice("ntfy"), Choice::Inside);
        assert_eq!(r.choice("toast"), Choice::Hide);
        assert_eq!(r.choice("timer"), Choice::Wake);
        assert_eq!(r.choice("camera.still"), Choice::Inside);
        assert_eq!(r.choice("complication"), Choice::Inside);
        assert_eq!(r.choice("plugin:anything"), Choice::Inside, "a plugin that was never heard of is one calm line");
        for s in CLASSES {
            assert!(s.allowed.contains(&s.default), "{} offers its own default", s.id);
        }
    }

    #[test]
    fn an_answer_is_taken_when_the_class_offers_it_and_a_default_is_not_stored() {
        let mut r = Rules::default();
        assert_eq!(r.set("ha", "wake"), Ok(Choice::Wake));
        assert_eq!(r.choice("ha"), Choice::Wake);
        assert_eq!(r.to_json(), json!({"ha": "wake"}));
        assert_eq!(r.set("ha", "inside"), Ok(Choice::Inside));
        assert!(r.is_default(), "the shipped default needs no entry");
        assert_eq!(r.set("camera.popup", "inside"), Err(Refused::NotOffered), "a video is not a dim line");
        assert_eq!(r.set("complication", "wake"), Err(Refused::NotOffered));
        assert_eq!(r.set("toast", "wake"), Err(Refused::NotOffered));
        assert_eq!(r.set("nonsense", "hide"), Err(Refused::UnknownClass));
        assert_eq!(r.set("ha", "loud"), Err(Refused::UnknownChoice));
        assert_eq!(r.set("plugin:", "hide"), Err(Refused::UnknownClass));
        assert_eq!(r.set("plugin:Bad Id", "hide"), Err(Refused::UnknownClass));
        assert_eq!(r.set("plugin:photos", "hide"), Ok(Choice::Hide));
        assert_eq!(r.choice("plugin:photos"), Choice::Hide);
        r.reset(Some("plugin:photos"));
        assert!(r.is_default());
        r.set("ha", "hide").unwrap();
        r.set("ntfy", "over").unwrap();
        r.reset(None);
        assert!(r.is_default());
    }

    #[test]
    fn what_is_stored_comes_back_and_what_is_wrong_in_the_file_is_left_out() {
        let mut r = Rules::default();
        r.set("ha", "wake").unwrap();
        r.set("plugin:photos", "hide").unwrap();
        assert_eq!(Rules::from_json(&r.to_json()), r);
        let f = Rules::from_json(&json!({"ha": "wake", "camera.popup": "inside", "nonsense": "hide", "ntfy": 4, "toast": "over"}));
        assert_eq!(f.choice("ha"), Choice::Wake);
        assert_eq!(f.choice("camera.popup"), Choice::Hide, "an answer the class does not offer is the default");
        assert_eq!(f.choice("toast"), Choice::Over);
        assert_eq!(f.to_json(), json!({"ha": "wake", "toast": "over"}));
        assert!(Rules::from_json(&json!("x")).is_default());
        assert!(Rules::from_json(&Value::Null).is_default());
    }

    #[test]
    fn a_plugin_appears_as_a_row_by_itself_and_the_two_named_ones_do_not_twice() {
        let rows = Rules::default().rows(&[plug("photos"), plug("home-assistant"), plug("ntfy"), plug("Bad Id")]);
        let ids: Vec<&str> = rows.iter().filter_map(|r| r["id"].as_str()).collect();
        assert_eq!(ids.len(), CLASSES.len() + 1);
        assert_eq!(ids.last(), Some(&"plugin:photos"));
        let ha = rows.iter().find(|r| r["id"] == "ha").unwrap();
        assert_eq!(ha["title"], "Home Assistant");
        assert_eq!(ha["choice"], "inside");
        assert_eq!(ha["choices"], json!(["over", "inside", "wake", "hide"]));
        let last = rows.last().unwrap();
        assert_eq!(last["glyph"], "plug", "a plugin with no mark gets the plug");
        assert_eq!(last["title"], "PHOTOS");
        let video = rows.iter().find(|r| r["id"] == "camera.popup").unwrap();
        assert_eq!(video["choices"], json!(["over", "wake", "hide"]));
        assert_eq!(video["default"], "hide");
    }

    #[test]
    fn the_topic_carries_every_class_and_its_answer() {
        let mut r = Rules::default();
        r.set("timer", "over").unwrap();
        let e = r.effective(&[plug("photos")]);
        assert_eq!(e["timer"], "over");
        assert_eq!(e["doorbell"], "wake");
        assert_eq!(e["plugin:photos"], "inside");
        assert_eq!(e.as_object().unwrap().len(), CLASSES.len() + 1);
    }

    #[test]
    fn a_note_is_told_from_its_plugin_and_what_it_is() {
        assert_eq!(card_class("", "doorbell", "base.icons/doorbell"), "doorbell");
        assert_eq!(card_class("", "ha", "base.icons/home"), "alert");
        assert_eq!(card_class("", "x", "base.icons/timer"), "timer");
        assert_eq!(card_class("", "alarms", ""), "alarm");
        assert_eq!(card_class("", "nba", "base.icons/trophy"), "score");
        assert_eq!(card_class("alert", "doorbell", ""), "alert");
        assert_eq!(card_class("timer", "ha", ""), "timer");
        assert_eq!(card_class("nonsense", "ha", ""), "alert");
        assert_eq!(note_class("home-assistant", "alert"), "ha");
        assert_eq!(note_class("ntfy", "alert"), "ntfy");
        assert_eq!(note_class("home-assistant", "doorbell"), "doorbell", "a doorbell is a doorbell whoever rang it");
        assert_eq!(note_class("ntfy", "alarm"), "timer");
        assert_eq!(note_class("photos", "score"), "plugin:photos");
        assert_eq!(plugin_source_class("home-assistant"), "ha");
        assert_eq!(plugin_source_class("hello"), "plugin:hello");
        assert_eq!(camera_class("doorbell"), "doorbell");
        assert_eq!(camera_class("person"), "camera.popup");
    }

    #[test]
    fn the_delivery_is_normal_unless_the_saver_is_up() {
        for c in Choice::ALL {
            assert_eq!(delivery(c, false), Delivery::Normal);
        }
        assert_eq!(delivery(Choice::Over, true), Delivery::OverSaver);
        assert_eq!(delivery(Choice::Inside, true), Delivery::Inside);
        assert_eq!(delivery(Choice::Wake, true), Delivery::Wake);
        assert_eq!(delivery(Choice::Hide, true), Delivery::Withheld);
    }

    #[test]
    fn the_overrides_are_bounded() {
        let mut r = Rules::default();
        for i in 0..(MAX_OVERRIDES + 10) {
            let _ = r.set(&format!("plugin:p{i}"), "hide");
        }
        assert_eq!(r.set.len(), MAX_OVERRIDES);
        assert_eq!(r.set("plugin:p0", "wake"), Ok(Choice::Wake), "one that is held can still change");
    }
}
