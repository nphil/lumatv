//! An engine's settings page as data. The engine names its fields; the app draws them with the theme's own rows and keeps the values,
//! so an engine never draws and a new engine needs no new screen. The vocabulary is the one LumaTV's plugin pages use (a toggle, a
//! choice, a number, a text, an action, a note) without depending on it.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const MAX_SECTIONS: usize = 16;
pub const MAX_FIELDS: usize = 128;
pub const MAX_OPTIONS: usize = 32;
pub const MAX_KEY_BYTES: usize = 64;
pub const MAX_LABEL_CHARS: usize = 128;
pub const MAX_HELP_CHARS: usize = 1024;

/// A setting's value. Nothing else is stored for an engine: its credentials stay in its own directory, out of the app's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Text(String),
}

/// One engine's values by field key.
pub type Values = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum FieldKind {
    Toggle {
        default: bool,
    },
    Choice {
        options: Vec<ChoiceOption>,
        default: String,
    },
    Number {
        min: i64,
        max: i64,
        step: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        default: i64,
    },
    Text {
        max_chars: u16,
        #[serde(default)]
        default: String,
    },
    /// A button. Pressing it sends the field's key to the engine as an action; it holds no value.
    Action,
    /// Words only.
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(flatten)]
    pub kind: FieldKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub title: String,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsSchema {
    #[serde(default)]
    pub sections: Vec<Section>,
}

/// Why a schema is refused. Each names the field's key (or the section's title) at fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// More sections, fields or options than a page shows.
    TooMany(&'static str),
    /// Empty, too long, not lowercase words, or used twice.
    Key(String),
    /// A label or title that is empty or too long, or help that is too long.
    Words(String),
    /// A default the field itself would refuse, a choice with no or repeated options, an empty range or a step under one.
    Shape(String),
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooMany(what) => write!(f, "too many {what}"),
            Self::Key(key) => write!(f, "bad or repeated key {key:?}"),
            Self::Words(key) => write!(f, "bad label or help at {key:?}"),
            Self::Shape(key) => write!(f, "field {key:?} contradicts itself"),
        }
    }
}

impl std::error::Error for SchemaError {}

/// Why a value is refused for a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueError {
    /// The schema has no such field.
    NoField,
    /// The field is an action or a note.
    NoValue,
    /// A value of another kind than the field's.
    Kind,
    /// Outside the range, off the step, not one of the options, or too long.
    Range,
}

fn is_key(key: &str) -> bool {
    let b = key.as_bytes();
    !b.is_empty() && b.len() <= MAX_KEY_BYTES && b[0].is_ascii_lowercase() && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

fn words_ok(text: &str, max_chars: usize) -> bool {
    !text.trim().is_empty() && text.chars().count() <= max_chars && !text.chars().any(char::is_control)
}

impl FieldKind {
    /// The value a field has before anyone changes it; `None` for the kinds that hold none.
    pub fn default(&self) -> Option<Value> {
        match self {
            Self::Toggle { default } => Some(Value::Bool(*default)),
            Self::Choice { default, .. } => Some(Value::Text(default.clone())),
            Self::Number { default, .. } => Some(Value::Int(*default)),
            Self::Text { default, .. } => Some(Value::Text(default.clone())),
            Self::Action | Self::Note => None,
        }
    }

    pub fn check(&self, value: &Value) -> Result<(), ValueError> {
        let in_range = |ok: bool| if ok { Ok(()) } else { Err(ValueError::Range) };
        match (self, value) {
            (Self::Action | Self::Note, _) => Err(ValueError::NoValue),
            (Self::Toggle { .. }, Value::Bool(_)) => Ok(()),
            (Self::Choice { options, .. }, Value::Text(id)) => in_range(options.iter().any(|o| &o.id == id)),
            (Self::Number { min, max, step, .. }, Value::Int(v)) => in_range((*min..=*max).contains(v) && *step > 0 && (v - min) % step == 0),
            (Self::Text { max_chars, .. }, Value::Text(text)) => {
                in_range(text.chars().count() <= usize::from(*max_chars) && !text.chars().any(char::is_control))
            }
            _ => Err(ValueError::Kind),
        }
    }
}

impl SettingsSchema {
    pub fn fields(&self) -> impl Iterator<Item = &Field> {
        self.sections.iter().flat_map(|s| s.fields.iter())
    }

    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields().find(|f| f.key == key)
    }

    /// Whether the page can be drawn and every default is a value its own field accepts. An engine's schema is checked once, when
    /// the engine says it is ready; a schema that fails leaves the engine without a settings page rather than with a broken one.
    pub fn validate(&self) -> Result<(), SchemaError> {
        if self.sections.len() > MAX_SECTIONS {
            return Err(SchemaError::TooMany("sections"));
        }
        if self.fields().count() > MAX_FIELDS {
            return Err(SchemaError::TooMany("fields"));
        }
        let mut keys = BTreeSet::new();
        for section in &self.sections {
            if !words_ok(&section.title, MAX_LABEL_CHARS) {
                return Err(SchemaError::Words(section.title.clone()));
            }
            for field in &section.fields {
                let key = || field.key.clone();
                if !is_key(&field.key) || !keys.insert(field.key.as_str()) {
                    return Err(SchemaError::Key(key()));
                }
                if !words_ok(&field.label, MAX_LABEL_CHARS) || field.help.as_deref().is_some_and(|h| !words_ok(h, MAX_HELP_CHARS)) {
                    return Err(SchemaError::Words(key()));
                }
                match &field.kind {
                    FieldKind::Choice { options, .. } => {
                        if options.len() > MAX_OPTIONS {
                            return Err(SchemaError::TooMany("options"));
                        }
                        let ids: BTreeSet<&str> = options.iter().map(|o| o.id.as_str()).collect();
                        if options.is_empty() || ids.len() != options.len() || options.iter().any(|o| !words_ok(&o.label, MAX_LABEL_CHARS)) {
                            return Err(SchemaError::Shape(key()));
                        }
                    }
                    FieldKind::Number { min, max, step, .. } if min > max || *step < 1 => return Err(SchemaError::Shape(key())),
                    _ => {}
                }
                if field.kind.default().is_some_and(|d| field.kind.check(&d).is_err()) {
                    return Err(SchemaError::Shape(key()));
                }
            }
        }
        Ok(())
    }

    pub fn check(&self, key: &str, value: &Value) -> Result<(), ValueError> {
        self.field(key).ok_or(ValueError::NoField)?.kind.check(value)
    }

    pub fn defaults(&self) -> Values {
        self.fields().filter_map(|f| Some((f.key.clone(), f.kind.default()?))).collect()
    }

    /// The values an engine is given: every field's default, overlaid with each stored value its field still accepts. A value kept
    /// from an older build of the engine that no longer fits (a removed option, a narrower range) falls back to the default instead
    /// of reaching the engine.
    pub fn resolve(&self, stored: &Values) -> Values {
        let mut values = self.defaults();
        for (key, value) in stored {
            if self.check(key, value).is_ok() {
                values.insert(key.clone(), value.clone());
            }
        }
        values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> SettingsSchema {
        serde_json::from_str(
            r#"{"sections":[{"title":"Stream","fields":[
                {"key":"tier","label":"Quality","type":"choice","options":[{"id":"uhd120","label":"4K 120"},{"id":"fhd120","label":"1080p 120"}],"default":"uhd120"},
                {"key":"hdr","label":"HDR","help":"Needs an Ultimate plan.","type":"toggle","default":true},
                {"key":"bitrate-mbps","label":"Bitrate","type":"number","min":10,"max":100,"step":5,"unit":"Mb/s","default":75},
                {"key":"host-name","label":"Name","type":"text","max_chars":8},
                {"key":"sign-out","label":"Sign out","type":"action"},
                {"key":"about","label":"Plays from NVIDIA's servers.","type":"note"}
            ]}]}"#,
        )
        .unwrap()
    }

    fn with(edit: impl FnOnce(&mut Vec<Field>)) -> Result<(), SchemaError> {
        let mut s = schema();
        edit(&mut s.sections[0].fields);
        s.validate()
    }

    #[test]
    fn a_schema_reads_from_its_wire_form_and_writes_it_back() {
        let s = schema();
        assert_eq!(s.validate(), Ok(()));
        assert!(matches!(s.field("bitrate-mbps").unwrap().kind, FieldKind::Number { min: 10, max: 100, step: 5, default: 75, .. }));
        assert_eq!(serde_json::from_str::<SettingsSchema>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
        assert_eq!(serde_json::from_str::<SettingsSchema>("{}").unwrap(), SettingsSchema::default());
        assert!(serde_json::from_str::<Field>(r#"{"key":"x","label":"X","type":"slider"}"#).is_err(), "a kind the app cannot draw is refused");
    }

    #[test]
    fn a_schema_that_contradicts_itself_is_refused() {
        assert_eq!(with(|f| f[1].key = "tier".into()), Err(SchemaError::Key("tier".into())));
        assert_eq!(with(|f| f[1].key = "Has Space".into()), Err(SchemaError::Key("Has Space".into())));
        assert_eq!(with(|f| f[1].label = " ".into()), Err(SchemaError::Words("hdr".into())));
        assert_eq!(with(|f| f[0].kind = FieldKind::Choice { options: vec![], default: String::new() }), Err(SchemaError::Shape("tier".into())));
        let other_default = |f: &mut Vec<Field>| {
            if let FieldKind::Choice { default, .. } = &mut f[0].kind {
                *default = "qhd120".into();
            }
        };
        assert_eq!(with(other_default), Err(SchemaError::Shape("tier".into())));
        assert_eq!(
            with(|f| f[2].kind = FieldKind::Number { min: 10, max: 100, step: 5, unit: None, default: 77 }),
            Err(SchemaError::Shape("bitrate-mbps".into()))
        );
        assert_eq!(
            with(|f| f[2].kind = FieldKind::Number { min: 10, max: 5, step: 1, unit: None, default: 10 }),
            Err(SchemaError::Shape("bitrate-mbps".into()))
        );
        assert_eq!(with(|f| f[2].kind = FieldKind::Number { min: 0, max: 5, step: 0, unit: None, default: 0 }), Err(SchemaError::Shape("bitrate-mbps".into())));
        let many = |f: &mut Vec<Field>| {
            let note = f[5].clone();
            f.extend((0..MAX_FIELDS).map(|i| Field { key: format!("n{i}"), ..note.clone() }));
        };
        assert_eq!(with(many), Err(SchemaError::TooMany("fields")));
    }

    #[test]
    fn a_value_is_taken_only_by_a_field_it_fits_and_the_rest_fall_back_to_defaults() {
        let s = schema();
        assert_eq!(s.check("hdr", &Value::Bool(false)), Ok(()));
        assert_eq!(s.check("hdr", &Value::Int(1)), Err(ValueError::Kind));
        assert_eq!(s.check("tier", &Value::Text("qhd120".into())), Err(ValueError::Range));
        assert_eq!(s.check("bitrate-mbps", &Value::Int(80)), Ok(()));
        assert_eq!(s.check("bitrate-mbps", &Value::Int(82)), Err(ValueError::Range), "off the step");
        assert_eq!(s.check("bitrate-mbps", &Value::Int(105)), Err(ValueError::Range));
        assert_eq!(s.check("host-name", &Value::Text("Living r\u{f6}".into())), Err(ValueError::Range), "nine characters");
        assert_eq!(s.check("host-name", &Value::Text("Den r\u{f6}\u{f6}m".into())), Ok(()), "characters are counted, not bytes");
        assert_eq!(s.check("sign-out", &Value::Bool(true)), Err(ValueError::NoValue));
        assert_eq!(s.check("gone", &Value::Bool(true)), Err(ValueError::NoField));

        let stored = Values::from([
            ("hdr".into(), Value::Bool(false)),
            ("tier".into(), Value::Text("qhd120".into())),
            ("bitrate-mbps".into(), Value::Int(50)),
            ("gone".into(), Value::Int(1)),
        ]);
        let expected = Values::from([
            ("tier".into(), Value::Text("uhd120".into())),
            ("hdr".into(), Value::Bool(false)),
            ("bitrate-mbps".into(), Value::Int(50)),
            ("host-name".into(), Value::Text(String::new())),
        ]);
        assert_eq!(s.resolve(&stored), expected);
        assert_eq!(serde_json::to_string(&s.resolve(&stored)).unwrap(), r#"{"bitrate-mbps":50,"hdr":false,"host-name":"","tier":"uhd120"}"#);
    }
}
