//! Who is using the antenna tuner, as the helper publishes it (the `tuner` topic, `helper/app/src/tuner.rs`), and the glyphs the status bar
//! shows for it: an antenna while a client's stream has the tuner, a chip beside it while the H.264 converter is making a stream. Nothing
//! shows while neither is so, and no words.

use serde_json::Value;

pub const TOPIC: &str = "tuner";
pub const ANTENNA: &str = "antenna";
pub const CONVERTER: &str = "cpu";

/// The theme key (a profile's own, under `layout.statusBar`) that places the glyphs on the bar.
pub const PLACE_KEY: &str = "tuner";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TunerUse {
    pub clients: u32,
    pub converting: bool,
}

impl TunerUse {
    pub fn from_json(v: &Value) -> Self {
        let clients = v.get("clients").and_then(Value::as_u64).unwrap_or(0) as u32;
        let streaming = v.get("streaming").and_then(Value::as_bool).unwrap_or(clients > 0);
        Self { clients: if streaming { clients.max(1) } else { 0 }, converting: streaming && v.get("converting").and_then(Value::as_bool).unwrap_or(false) }
    }

    /// The glyphs to show now, antenna first.
    pub fn glyphs(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.clients > 0 {
            out.push(ANTENNA);
        }
        if self.converting {
            out.push(CONVERTER);
        }
        out
    }
}

/// Where the glyphs sit on the status bar: after its buttons (the default, the right side), before them, or nowhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Off,
    Left,
    Right,
}

impl Place {
    pub const CHOICES: [Place; 3] = [Place::Right, Place::Left, Place::Off];
    pub const WORDS: [&'static str; 3] = ["Right", "Left", "Off"];

    pub fn of(key: Option<&str>) -> Self {
        match key {
            Some("off") => Self::Off,
            Some("left") => Self::Left,
            _ => Self::Right,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_antenna_shows_while_streaming_and_the_chip_only_while_converting_a_stream() {
        assert!(TunerUse::from_json(&json!({"streaming": false, "clients": 0, "converting": false})).glyphs().is_empty());
        assert_eq!(TunerUse::from_json(&json!({"streaming": true, "clients": 1, "converting": false})).glyphs(), [ANTENNA]);
        assert_eq!(TunerUse::from_json(&json!({"streaming": true, "clients": 1, "converting": true})).glyphs(), [ANTENNA, CONVERTER]);
        assert!(TunerUse::from_json(&json!({"streaming": false, "converting": true})).glyphs().is_empty());
        assert!(TunerUse::from_json(&Value::Null).glyphs().is_empty());
    }

    #[test]
    fn the_place_defaults_to_the_right_and_round_trips() {
        assert_eq!(Place::of(None), Place::Right);
        for p in Place::CHOICES {
            assert_eq!(Place::of(Some(p.key())), p);
        }
    }
}
