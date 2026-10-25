//! What one assistant has been given: a [`Level`] for each [`Group`], kept in the settings layer.
//!
//! Each paired assistant is a settings owner `ai.<assistant id>` with one choice field per group, so the TV's Settings can draw
//! its page from the schema like any other page. The owner is *protected*: the generic settings API refuses it, and every change
//! goes through this module's callers (`store`), where a change that gives an assistant more needs a physical remote press and
//! a change that gives it less does not.

use super::types::{Group, Level};
use serde_json::{Value, json};
use settings_schema::{SettingsSchema, Value as SettingValue, Values};
use std::collections::BTreeMap;

/// The prefix of every AI settings owner. The settings store refuses a change to any owner with this prefix from outside the AI module.
pub const OWNER_PREFIX: &str = "ai.";

/// The settings owner of an assistant.
pub fn owner_of(assistant_id: &str) -> String {
    format!("{OWNER_PREFIX}{assistant_id}")
}

/// A level for every group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Levels {
    pub state: Level,
    pub playback: Level,
    pub launch: Level,
    pub power: Level,
}

impl Levels {
    /// The plan's table: reads and playback always, opening apps and turning the TV off ask.
    pub fn defaults() -> Self {
        Self {
            state: Group::State.default_level(),
            playback: Group::Playback.default_level(),
            launch: Group::Launch.default_level(),
            power: Group::Power.default_level(),
        }
    }

    pub fn get(&self, group: Group) -> Level {
        match group {
            Group::State => self.state,
            Group::Playback => self.playback,
            Group::Launch => self.launch,
            Group::Power => self.power,
        }
    }

    pub fn set(&mut self, group: Group, level: Level) {
        match group {
            Group::State => self.state = level,
            Group::Playback => self.playback = level,
            Group::Launch => self.launch = level,
            Group::Power => self.power = level,
        }
    }

    /// Whether going from `self` to `new` gives the assistant more in any group.
    pub fn widened_by(&self, new: &Levels) -> bool {
        Group::ALL.into_iter().any(|g| new.get(g) > self.get(g))
    }

    /// The groups whose level differs, as `(group, old, new)`.
    pub fn changes_to(&self, new: &Levels) -> Vec<(Group, Level, Level)> {
        Group::ALL
            .into_iter()
            .filter(|g| self.get(*g) != new.get(*g))
            .map(|g| (g, self.get(g), new.get(g)))
            .collect()
    }

    /// `{"state": "always", ...}`
    pub fn to_json(&self) -> Value {
        let map: BTreeMap<&str, &str> = Group::ALL
            .into_iter()
            .map(|g| (g.id(), self.get(g).id()))
            .collect();
        json!(map)
    }

    /// Reads a levels object as an API client sends it: every group must be present with a known level, nothing else.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let map = value.as_object().ok_or("levels must be an object")?;
        let mut levels = Self::defaults();
        for group in Group::ALL {
            let text = map.get(group.id()).and_then(Value::as_str).ok_or_else(|| {
                format!("levels.{} must be one of never, ask, always", group.id())
            })?;
            let level = Level::parse(text).ok_or_else(|| {
                format!("levels.{} must be one of never, ask, always", group.id())
            })?;
            levels.set(group, level);
        }
        if let Some(extra) = map.keys().find(|k| Group::parse(k).is_none()) {
            return Err(format!("levels.{extra} is not a permission group"));
        }
        Ok(levels)
    }

    /// The levels in force from a settings owner's effective values (a missing or unreadable value is the group's default,
    /// never more).
    pub fn from_values(values: &Values) -> Self {
        let mut levels = Self::defaults();
        for group in Group::ALL {
            let key = group.id();
            let stored = values
                .iter()
                .find(|(k, _)| k.as_str() == key)
                .map(|(_, v)| v);
            if let Some(SettingValue::Choice(choice)) = stored
                && let Some(level) = Level::parse(&choice.to_string())
            {
                levels.set(group, level);
            }
        }
        levels
    }
}

/// The settings schema of one assistant's page. `name` is the assistant's own word for itself, cleaned: it is shown, never trusted.
pub fn schema_for(assistant_id: &str, name: &str, revision: u32) -> Result<SettingsSchema, String> {
    let title = format!("{name} (AI assistant)");
    let fields: Vec<Value> = Group::ALL
        .into_iter()
        .map(|group| {
            json!({
                "key": group.id(),
                "label": group.title(),
                "kind": {"type": "choice", "options": {"inline": [
                    {"id": "never", "label": Level::Never.title()},
                    {"id": "ask", "label": Level::Ask.title()},
                    {"id": "always", "label": Level::Always.title()},
                ]}},
                "default": {"type": "choice", "value": group.default_level().id()},
                "apply": "on-commit",
            })
        })
        .collect();
    let document = json!({
        "format": 1,
        "owner": owner_of(assistant_id),
        "revision": revision,
        "title": title,
        "sections": [{"id": "permissions", "title": "What it may do", "fields": fields}],
    });
    let bytes = serde_json::to_vec(&document).map_err(|e| e.to_string())?;
    SettingsSchema::parse(&bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings_schema::{FieldKey, Value as SettingValue};

    fn all(level: Level) -> Levels {
        Levels {
            state: level,
            playback: level,
            launch: level,
            power: level,
        }
    }

    #[test]
    fn the_defaults_are_the_plans_table() {
        let d = Levels::defaults();
        assert_eq!(
            (d.state, d.playback, d.launch, d.power),
            (Level::Always, Level::Always, Level::Ask, Level::Ask)
        );
        assert_eq!(
            d.to_json(),
            json!({"state": "always", "playback": "always", "launch": "ask", "power": "ask"})
        );
    }

    #[test]
    fn levels_are_ordered_by_how_much_they_allow() {
        assert!(Level::Never < Level::Ask && Level::Ask < Level::Always);
    }

    #[test]
    fn widening_means_more_in_any_group_and_nothing_else() {
        let ask = all(Level::Ask);
        assert!(ask.widened_by(&all(Level::Always)));
        assert!(!ask.widened_by(&all(Level::Never)));
        assert!(!ask.widened_by(&ask));
        let mut mixed = all(Level::Never);
        mixed.set(Group::Launch, Level::Ask);
        assert!(all(Level::Never).widened_by(&mixed));
        // One group down and another up is a widening: it needs the press.
        let mut swapped = ask;
        swapped.set(Group::State, Level::Never);
        swapped.set(Group::Power, Level::Always);
        assert!(ask.widened_by(&swapped));
        assert_eq!(
            ask.changes_to(&swapped),
            vec![
                (Group::State, Level::Ask, Level::Never),
                (Group::Power, Level::Ask, Level::Always)
            ]
        );
    }

    #[test]
    fn json_levels_must_be_complete_and_known() {
        let ok = json!({"state": "never", "playback": "ask", "launch": "always", "power": "never"});
        let levels = Levels::from_json(&ok).unwrap();
        assert_eq!(
            (levels.state, levels.playback, levels.launch, levels.power),
            (Level::Never, Level::Ask, Level::Always, Level::Never)
        );
        for bad in [
            json!({"state": "never"}),
            json!({"state": "sometimes", "playback": "ask", "launch": "ask", "power": "ask"}),
            json!({"state": "ask", "playback": "ask", "launch": "ask", "power": "ask", "root": "always"}),
            json!({"state": 1, "playback": "ask", "launch": "ask", "power": "ask"}),
            json!(["state"]),
            json!(null),
        ] {
            assert!(Levels::from_json(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_assistants_page_is_a_valid_schema_with_one_choice_per_group() {
        let schema = schema_for("a1b2c3d4e5f6", "Cody", 1).unwrap();
        assert_eq!(schema.owner.as_str(), "ai.a1b2c3d4e5f6");
        assert_eq!(schema.title, "Cody (AI assistant)");
        let fields: Vec<_> = schema.fields().map(|f| f.key.as_str().to_owned()).collect();
        assert_eq!(fields, ["state", "playback", "launch", "power"]);
        let defaults = schema.defaults();
        assert_eq!(Levels::from_values(&defaults), Levels::defaults());
    }

    #[test]
    fn a_hostile_name_cannot_break_the_schema_or_widen_anything() {
        // A name is only ever a title: control characters are refused by the schema, so the caller cleans names first.
        assert!(schema_for("a1", "Cody\u{7}", 1).is_err());
        assert!(schema_for("a1", "x", 1).is_ok());
    }

    #[test]
    fn stored_values_are_read_back_and_unknown_ones_are_the_default() {
        let mut values = Values::new();
        values.insert(
            "launch".parse::<FieldKey>().unwrap(),
            SettingValue::Choice("always".parse().unwrap()),
        );
        values.insert(
            "power".parse::<FieldKey>().unwrap(),
            SettingValue::Text("always".into()),
        );
        let levels = Levels::from_values(&values);
        assert_eq!(levels.launch, Level::Always);
        assert_eq!(
            levels.power,
            Level::Ask,
            "a value of the wrong type is the default, never more"
        );
        assert_eq!(levels.state, Level::Always);
    }

    #[test]
    fn groups_and_levels_round_trip_by_id() {
        for g in Group::ALL {
            assert_eq!(Group::parse(g.id()), Some(g));
        }
        for l in [Level::Never, Level::Ask, Level::Always] {
            assert_eq!(Level::parse(l.id()), Some(l));
        }
        assert_eq!(Group::parse("capture"), None);
        assert_eq!(Level::parse("Always"), None);
    }
}
