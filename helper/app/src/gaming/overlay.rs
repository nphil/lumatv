//! LumaGame's stats overlay setting (Off, Compact or Full), mirrored here so the TV's Quick Settings can show and change it.
//! LumaGame keeps its own copy for when this plugin is off; on either side the newer change wins.

use super::lease::save;
use crate::hub::Hub;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

/// The shared setting mirrored for LumaGame: `{"mode": "off" | "compact" | "full", "changed": unix ms}`.
pub const TOPIC: &str = "gaming.overlay";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Off,
    Compact,
    Full,
}

impl Mode {
    const CHOICES: [(Self, &'static str, &'static str); 3] = [
        (Self::Off, "Off", "off"),
        (Self::Compact, "Compact", "compact"),
        (Self::Full, "Full", "full"),
    ];

    pub fn choice(value: &str) -> Option<Self> {
        Self::CHOICES.iter().find(|(_, _, key)| *key == value).map(|(mode, _, _)| *mode)
    }
}

pub fn tile(mode: Mode) -> serde_json::Value {
    let index = Mode::CHOICES.iter().position(|(choice, _, _)| *choice == mode);
    let options: Vec<_> = Mode::CHOICES.iter()
        .map(|(_, label, value)| json!({"label":label,"value":value})).collect();
    json!({"id":"stats","label":"Game stats","icon":"base.icons/gauge",
        "state":{"type":"stepper","index":index},"action":"stats-mode","options":options})
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Overlay {
    pub mode: Mode,
    /// When it was chosen (Unix ms); 0 for never.
    pub changed: u64,
}

pub struct Store {
    path: PathBuf,
    value: Mutex<Overlay>,
}

impl Store {
    pub fn load(path: PathBuf) -> Self {
        let value = fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { path, value: Mutex::new(value) }
    }

    pub fn get(&self) -> Overlay {
        *self.value.lock()
    }

    pub fn publish(&self, hub: &Hub) {
        hub.set(TOPIC, json!(self.get()));
    }

    /// A change made at `changed` (now, when the caller does not say): kept unless the one held is newer. Returns what is held and
    /// whether this change was taken.
    pub fn set(&self, mode: Mode, changed: Option<u64>, now: u64, hub: &Hub) -> std::io::Result<(Overlay, bool)> {
        let mut value = self.value.lock();
        let next = Overlay { mode, changed: changed.unwrap_or(now) };
        if next.changed < value.changed {
            return Ok((*value, false));
        }
        save(&self.path, &next)?;
        *value = next;
        hub.set(TOPIC, json!(next));
        Ok((next, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newer_change_wins_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gaming/overlay.json");
        let hub = Hub::new(0);
        let store = Store::load(path.clone());
        assert_eq!(store.get(), Overlay::default());
        assert_eq!(store.set(Mode::Compact, None, 2_000, &hub).unwrap(), (Overlay { mode: Mode::Compact, changed: 2_000 }, true));
        assert_eq!(hub.get(TOPIC), Some(json!({"mode": "compact", "changed": 2_000})));
        let (held, taken) = store.set(Mode::Full, Some(1_000), 3_000, &hub).unwrap();
        assert_eq!((held.mode, taken), (Mode::Compact, false), "an older change from LumaGame loses to Quick Settings");
        assert_eq!(Store::load(path).get().mode, Mode::Compact);
    }
}
