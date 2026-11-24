//! The game recorder's settings: one section each engine with the `recorder` feature adds to its page, and what the engine reads
//! back. The recorder itself (the RAM ring and the muxer) is `lumagame-recorder`; the app only draws these fields and sends
//! `save-clip`.

use crate::settings::{Field, FieldKind, Section, Value, Values};

pub const ENABLED: &str = "recorder";
pub const URL: &str = "recorder-url";
pub const FOLDER: &str = "recorder-folder";
pub const SECONDS: &str = "recorder-seconds";

pub const DEFAULT_FOLDER: &str = "lumagame";
pub const DEFAULT_SECONDS: i64 = 15;

/// The settings section. The recorder is off until the viewer turns it on.
pub fn section() -> Section {
    let field = |key: &str, label: &str, help: Option<&str>, kind| Field { key: key.into(), label: label.into(), help: help.map(Into::into), kind };
    Section {
        title: "Game recorder".into(),
        fields: vec![
            field(
                ENABLED,
                "Save clips",
                Some("Keeps the stream's last seconds in the TV's memory (at most 128 MB, nothing is written on the TV) and adds Save clip to the in-stream menu. A clip is sent to the receiver below."),
                FieldKind::Toggle { default: false },
            ),
            field(
                URL,
                "Receiver address",
                Some("Where clips go, as http://host:port, on BeastNAS or another machine on the home network. The address is plain HTTP, so keep it on the LAN."),
                FieldKind::Text { max_chars: 128, default: String::new() },
            ),
            field(
                FOLDER,
                "Clip folder",
                Some("A folder under the receiver's own; letters, digits and - _ . only."),
                FieldKind::Text { max_chars: 64, default: DEFAULT_FOLDER.into() },
            ),
            field(
                SECONDS,
                "Clip length",
                Some("How far back a clip starts, from the keyframe at or before that point. The memory holds about 14 seconds at 65 Mbit/s, so a clip is shorter on a higher bitrate."),
                FieldKind::Number { min: 5, max: 60, step: 5, unit: Some("s".into()), default: DEFAULT_SECONDS },
            ),
        ],
    }
}

/// The recorder settings as an engine reads them from its resolved values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub url: String,
    pub folder: String,
    pub seconds: u32,
}

impl Settings {
    pub fn from_values(values: &Values) -> Self {
        let text = |key: &str, default: &str| match values.get(key) {
            Some(Value::Text(t)) => t.trim().to_owned(),
            _ => default.to_owned(),
        };
        Self {
            enabled: matches!(values.get(ENABLED), Some(Value::Bool(true))),
            url: text(URL, ""),
            folder: text(FOLDER, DEFAULT_FOLDER),
            seconds: match values.get(SECONDS) {
                Some(Value::Int(n)) => u32::try_from(*n).unwrap_or(DEFAULT_SECONDS as u32),
                _ => DEFAULT_SECONDS as u32,
            },
        }
    }

    /// Whether a recorder should run: switched on and told where clips go.
    pub fn runs(&self) -> bool {
        self.enabled && !self.url.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SettingsSchema;

    #[test]
    fn the_recorder_is_off_until_switched_on_and_given_an_address() {
        let schema = SettingsSchema { sections: vec![section()] };
        schema.validate().unwrap();
        let mut values = schema.defaults();
        let off = Settings::from_values(&values);
        assert_eq!((off.enabled, off.folder.as_str(), off.seconds, off.runs()), (false, "lumagame", 15, false));
        values.insert(ENABLED.into(), Value::Bool(true));
        assert!(!Settings::from_values(&values).runs(), "no address, no recorder");
        values.insert(URL.into(), Value::Text(" http://nas:8765 ".into()));
        let on = Settings::from_values(&values);
        assert!(on.runs());
        assert_eq!(on.url, "http://nas:8765");
    }
}
