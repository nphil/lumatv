//! The game recorder's settings: one section each engine with the `recorder` feature adds to its page, and what the engine reads
//! back. The recorder itself (the RAM ring, the muxer and the feed to the NAS) is `lumagame-recorder`; the app only draws these
//! fields and sends `recorder` requests from the in-stream menu.

use crate::settings::{ChoiceOption, Field, FieldKind, Section, Value, Values};

pub const ENABLED: &str = "recorder";
pub const URL: &str = "recorder-url";
pub const SECONDS: &str = "recorder-seconds";
pub const REPLAY: &str = "recorder-replay";
pub const REACH: &str = "recorder-reach";
pub const RECORD_MAX: &str = "recorder-record-max";

pub const DEFAULT_SECONDS: i64 = 15;
/// The reach of long replay in minutes; the choices on the page.
pub const REACH_MINUTES: [u32; 4] = [1, 5, 15, 30];
pub const DEFAULT_REACH_MINUTES: u32 = 5;
pub const DEFAULT_RECORD_MAX_MINUTES: i64 = 120;

/// The settings section. The recorder is off until the viewer turns it on, and long replay is off until turned on as well.
pub fn section() -> Section {
    let field = |key: &str, label: &str, help: &str, kind| Field { key: key.into(), label: label.into(), help: Some(help.into()), kind };
    Section {
        title: "Game recorder".into(),
        fields: vec![
            field(
                ENABLED,
                "Game recorder",
                "Keeps the stream's last seconds in the TV's memory (at most 128 MB, nothing is written on the TV) and adds Save clip and Record to the in-stream menu. Clips and recordings go to the receiver below, which files them by game.",
                FieldKind::Toggle { default: false },
            ),
            field(
                URL,
                "Receiver address",
                "The receiver on BeastNAS, as http://host:port. The stream goes there over plain HTTP, so keep it on the home network.",
                FieldKind::Text { max_chars: 128, default: String::new() },
            ),
            field(
                REPLAY,
                "Long replay",
                "Sends every stream to the receiver while it plays; the receiver keeps the last minutes in a buffer of a fixed size, and Save clip then saves the length below. Off, or while the NAS is not answering, Save clip keeps the TV's own last seconds. The in-stream menu turns it on or off for the game that is running.",
                FieldKind::Toggle { default: false },
            ),
            field(
                REACH,
                "Replay length",
                "How far back Save clip reaches with long replay on. The receiver's buffer has a fixed size (6 GB as set up on BeastNAS), so what it holds falls as the bitrate rises: about 13 min at 65 Mbit/s and 5 min 40 s at 150 Mbit/s. A length that does not fit is refused: LumaGame says so when the stream starts, and Save clip saves what the buffer holds.",
                FieldKind::Choice {
                    options: REACH_MINUTES.iter().map(|m| ChoiceOption { id: m.to_string(), label: format!("{m} min") }).collect(),
                    default: DEFAULT_REACH_MINUTES.to_string(),
                },
            ),
            field(
                SECONDS,
                "Instant clip length",
                "How far back a clip from the TV's own memory starts, from the keyframe at or before that point. The memory holds about 13 seconds at 65 Mbit/s, so such a clip is shorter on a higher bitrate.",
                FieldKind::Number { min: 5, max: 60, step: 5, unit: Some("s".into()), default: DEFAULT_SECONDS },
            ),
            field(
                RECORD_MAX,
                "Longest recording",
                "Record in the in-stream menu writes the stream straight to the recordings share from the next keyframe on. A recording ends when stopped, with the stream, at this length, or when the share is nearly full; the receiver has its own limit as well.",
                FieldKind::Number { min: 10, max: 240, step: 10, unit: Some("min".into()), default: DEFAULT_RECORD_MAX_MINUTES },
            ),
        ],
    }
}

/// The recorder settings as an engine reads them from its resolved values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub url: String,
    /// The length of a clip from the TV's own memory.
    pub seconds: u32,
    /// Long replay is on when a stream starts.
    pub replay: bool,
    pub reach_s: u32,
    pub record_max_s: u32,
}

impl Settings {
    pub fn from_values(values: &Values) -> Self {
        let number = |key: &str, default: i64| match values.get(key) {
            Some(Value::Int(n)) => u32::try_from(*n).unwrap_or(default as u32),
            _ => default as u32,
        };
        let reach = match values.get(REACH) {
            Some(Value::Text(t)) => t.parse().ok().filter(|m| REACH_MINUTES.contains(m)),
            _ => None,
        };
        Self {
            enabled: matches!(values.get(ENABLED), Some(Value::Bool(true))),
            url: match values.get(URL) {
                Some(Value::Text(t)) => t.trim().to_owned(),
                _ => String::new(),
            },
            seconds: number(SECONDS, DEFAULT_SECONDS),
            replay: matches!(values.get(REPLAY), Some(Value::Bool(true))),
            reach_s: reach.unwrap_or(DEFAULT_REACH_MINUTES) * 60,
            record_max_s: number(RECORD_MAX, DEFAULT_RECORD_MAX_MINUTES) * 60,
        }
    }

    /// Whether a recorder should run: switched on and told where the receiver is.
    pub fn runs(&self) -> bool {
        self.enabled && !self.url.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SettingsSchema;

    #[test]
    fn the_recorder_is_off_until_switched_on_and_given_an_address_and_long_replay_is_its_own_switch() {
        let schema = SettingsSchema { sections: vec![section()] };
        schema.validate().unwrap();
        let mut values = schema.defaults();
        let off = Settings::from_values(&values);
        assert_eq!((off.enabled, off.seconds, off.replay, off.reach_s, off.record_max_s, off.runs()), (false, 15, false, 300, 7200, false));
        values.insert(ENABLED.into(), Value::Bool(true));
        assert!(!Settings::from_values(&values).runs(), "no address, no recorder");
        values.insert(URL.into(), Value::Text(" http://nas:8765 ".into()));
        values.insert(REPLAY.into(), Value::Bool(true));
        values.insert(REACH.into(), Value::Text("30".into()));
        let on = Settings::from_values(&values);
        assert!(on.runs());
        assert_eq!((on.url.as_str(), on.replay, on.reach_s), ("http://nas:8765", true, 1800));
    }
}
