//! What this device is, in the words of the plugin API's `welcome`: the class of its screen and how a person drives it.
//!
//! LumaShow's plugin API (level 1) lets one plugin package run on every Luma device, and a plugin that has views of its own
//! must adapt to the device (a smart display is touched, a TV is driven with a remote). `welcome` carries the facts as one
//! additive field, `device` (the contract's change rule inside a level: a message may gain optional fields, and a receiver
//! skips one it does not know), and the TV's UI reads the same facts at `GET /v1/device`:
//!
//! ```json
//! {"screen": {"class": "tv", "width": 1920, "height": 1080}, "input": ["remote", "pointer"]}
//! ```
//!
//! - `screen.class` is a Lucent device profile (`tv`, `smart`, `phone`, `tablet`, `desktop`); `width` and `height` are the
//!   logical pixels the UI is drawn in (the TV draws 1920 by 1080 and LG's scaler fills the panel).
//! - `input` lists the ways a person drives the device, in the order `touch`, `remote`, `pointer`, `controller`: the D-pad
//!   remote, the Magic Remote's pointer, a game controller that is connected now.
//!
//! The screen and the remote are what the device is, so they are configuration (`device` in `helper.json`, the TV's defaults
//! when absent). A game controller comes and goes, so it is looked for in the kernel's input list at each question: a device
//! whose key bits include `BTN_GAMEPAD`.

use crate::config::DeviceConfig;
use serde_json::{Value, json};
use std::path::Path;

/// The key code the kernel gives a gamepad's first button (`BTN_SOUTH`, also `BTN_GAMEPAD`).
const BTN_GAMEPAD: usize = 0x130;

/// A way a person drives the device. The order here is the order of the list in the facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Input {
    Touch,
    Remote,
    Pointer,
    Controller,
}

impl Input {
    pub fn word(self) -> &'static str {
        match self {
            Self::Touch => "touch",
            Self::Remote => "remote",
            Self::Pointer => "pointer",
            Self::Controller => "controller",
        }
    }
}

/// The facts a plugin is told when it connects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    pub class: String,
    pub width: u32,
    pub height: u32,
    pub input: Vec<Input>,
}

impl Facts {
    pub fn to_json(&self) -> Value {
        json!({
            "screen": {"class": self.class, "width": self.width, "height": self.height},
            "input": self.input.iter().map(|i| i.word()).collect::<Vec<_>>(),
        })
    }
}

/// The facts now: the configured screen and inputs, plus a game controller if the kernel lists one under `sys_input_dir`
/// (`/sys/class/input`).
pub fn facts(config: &DeviceConfig, sys_input_dir: &Path) -> Facts {
    let mut input = Vec::new();
    if config.touch {
        input.push(Input::Touch);
    }
    if config.remote {
        input.push(Input::Remote);
    }
    if config.pointer {
        input.push(Input::Pointer);
    }
    if config.detect_controllers && has_gamepad(sys_input_dir) {
        input.push(Input::Controller);
    }
    Facts {
        class: config.screen_class.clone(),
        width: config.width,
        height: config.height,
        input,
    }
}

/// Whether any input device under `sys_dir` has the gamepad button in its key bits. The kernel prints
/// `capabilities/key` as hexadecimal words of the machine's `long`, most significant first, separated by spaces.
pub fn has_gamepad(sys_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(sys_dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with("input") && n[5..].bytes().all(|b| b.is_ascii_digit()))
            && std::fs::read_to_string(entry.path().join("capabilities").join("key"))
                .is_ok_and(|bits| key_bit(&bits, BTN_GAMEPAD))
    })
}

/// Whether bit `index` is set in a `capabilities/...` bitmap.
pub fn key_bit(bitmap: &str, index: usize) -> bool {
    let word_bits = usize::BITS as usize;
    let words: Vec<&str> = bitmap.split_whitespace().collect();
    // The last word holds bits 0 to word_bits - 1.
    let Some(from_right) = words.len().checked_sub(1 + index / word_bits) else {
        return false;
    };
    u64::from_str_radix(words[from_right], 16).is_ok_and(|word| word >> (index % word_bits) & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tv() -> DeviceConfig {
        DeviceConfig::default()
    }

    /// A fake `/sys/class/input` with one device per `(name, key bitmap)`.
    fn sysfs(devices: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (i, (name, key)) in devices.iter().enumerate() {
            let device = dir.path().join(format!("input{i}"));
            std::fs::create_dir_all(device.join("capabilities")).unwrap();
            std::fs::write(device.join("name"), format!("{name}\n")).unwrap();
            std::fs::write(device.join("capabilities").join("key"), format!("{key}\n")).unwrap();
        }
        // A node that is not a device (the kernel lists event nodes beside them) must not confuse the scan.
        std::fs::create_dir_all(dir.path().join("event0")).unwrap();
        dir
    }

    // A pad with BTN_SOUTH (0x130 = 304) and BTN_EAST set: bits 48 and 49 of the fifth word from the right.
    const PAD: &str = "3000000000000 0 0 0 0";
    // A remote's keys stop below 0x100: the first four words, nothing in the fifth.
    const REMOTE: &str = "0 0 ffffffffffffffff ffffffffffffffff ffffffffffffffff";

    #[test]
    fn the_tv_is_told_what_it_is_without_a_controller() {
        let dir = sysfs(&[("LGE RCU", REMOTE)]);
        let f = facts(&tv(), dir.path());
        assert_eq!(
            f.to_json(),
            json!({"screen": {"class": "tv", "width": 1920, "height": 1080}, "input": ["remote", "pointer"]})
        );
    }

    #[test]
    fn a_controller_is_listed_only_while_the_kernel_lists_one() {
        let dir = sysfs(&[("LGE RCU", REMOTE), ("Pad", PAD)]);
        assert!(has_gamepad(dir.path()));
        assert_eq!(
            facts(&tv(), dir.path()).input,
            [Input::Remote, Input::Pointer, Input::Controller]
        );
        // Pulled out: the next question says so.
        std::fs::remove_dir_all(dir.path().join("input1")).unwrap();
        assert!(!has_gamepad(dir.path()));
        assert_eq!(facts(&tv(), dir.path()).input, [Input::Remote, Input::Pointer]);
        // A missing list is no controller, not a failure.
        assert!(!has_gamepad(Path::new("/nonexistent/sys/class/input")));
    }

    #[test]
    fn bits_are_read_from_the_right_hand_word() {
        assert!(key_bit("1", 0));
        assert!(!key_bit("1", 1));
        assert!(key_bit("8000000000000000", 63));
        assert!(key_bit("1 0", 64));
        assert!(!key_bit("1 0", 0));
        assert!(key_bit(PAD, BTN_GAMEPAD));
        assert!(!key_bit(REMOTE, BTN_GAMEPAD), "a remote's keys stop below the gamepad's");
        assert!(!key_bit("3 0 0 0", BTN_GAMEPAD), "the same bits one word short are other keys");
        // Nothing to read, or not a number, is not a gamepad.
        assert!(!key_bit("", 0));
        assert!(!key_bit("zz", 0));
        assert!(!key_bit("1", 64));
    }

    #[test]
    fn the_configuration_decides_the_screen_and_the_fixed_inputs() {
        let dir = sysfs(&[]);
        let smart = DeviceConfig {
            screen_class: "smart".into(),
            width: 960,
            height: 480,
            touch: true,
            remote: false,
            pointer: false,
            detect_controllers: false,
        };
        let f = facts(&smart, dir.path());
        assert_eq!(
            f.to_json(),
            json!({"screen": {"class": "smart", "width": 960, "height": 480}, "input": ["touch"]})
        );
        // Detection switched off ignores a pad that is there.
        let pad = sysfs(&[("Pad", PAD)]);
        assert_eq!(facts(&smart, pad.path()).input, [Input::Touch]);
    }
}
