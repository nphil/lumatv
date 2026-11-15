//! Vendor setting builders. Profile-domain writes retain the complete freshly read object.

use crate::{Change, Command, Error, Profile, Request, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ControllerMode {
    Xinput = 0,
    Switch = 1,
    Ps4 = 2,
    Ps5 = 3,
    MobileTouch = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TriggerMode {
    Adaptive = 0,
    RapidFire = 1,
    Linear = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MacroDevice {
    Gamepad = 0,
    Keyboard = 1,
    Mouse = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacroStep {
    pub device: MacroDevice,
    /// Wire key id, not the PC frontend's private Rust enum discriminant.
    pub key: u8,
    pub interval: u32,
    pub duration: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ButtonAction {
    #[serde(rename = "t")]
    pub target: u8,
    #[serde(rename = "v")]
    pub keys: [u8; 4],
    #[serde(rename = "n")]
    pub macro_name: String,
    #[serde(rename = "r")]
    pub repeat: u8,
    #[serde(rename = "sp")]
    pub turbo_rate: u8,
    #[serde(rename = "mr")]
    pub macro_mode: u8,
    #[serde(rename = "ms")]
    pub macro_steps: u16,
    #[serde(rename = "ri")]
    pub macro_interval: u16,
}

impl ButtonAction {
    pub fn original() -> Self {
        Self { target: 0, keys: [0; 4], macro_name: String::new(), repeat: 0, turbo_rate: 0, macro_mode: 0, macro_steps: 0, macro_interval: 0 }
    }

    pub fn mapping(target: u8, keys: [u8; 4], turbo_rate: u8) -> Result<Self> {
        if !matches!(target, 0 | 2..=4) || !matches!(turbo_rate, 0 | 5 | 10 | 20 | 30) {
            return Err(Error::Invalid("mapping target or turbo rate is not vendor-supported"));
        }
        Ok(Self { target, keys, turbo_rate, repeat: if turbo_rate == 0 { 0 } else { 2 }, ..Self::original() })
    }

    pub fn macro_action(name: String, mode: u8, steps: u16, interval: u16) -> Result<Self> {
        check_macro_name(&name)?;
        if mode > 4 {
            return Err(Error::Invalid("macro execution mode must be 0..=4"));
        }
        Ok(Self { target: 1, repeat: 1, macro_name: name, macro_mode: mode, macro_steps: steps, macro_interval: interval, ..Self::original() })
    }
}

impl Request {
    pub fn macro_detail(name: &str) -> Result<Self> {
        Self::argument(Command::MacroDetail, "n", json!(name))
    }

    pub fn fps_detail(name: &str) -> Result<Self> {
        Self::argument(Command::FpsDetail, "n", json!(name))
    }

    pub fn fps(profile: Profile, slots: &[u8]) -> Result<Self> {
        if slots.iter().any(|s| *s > 2) {
            return Err(Error::Invalid("FPS slot must be 0..=2"));
        }
        let mut args = profile.arguments();
        args["s"] = json!(slots);
        Self::argument(Command::Fps, "g", args)
    }

    pub fn motion_device_config(profile: Profile, devices: &[u8]) -> Result<Self> {
        if devices.iter().any(|d| !(1..=4).contains(d)) {
            return Err(Error::Invalid("motion device index must be 1..=4"));
        }
        let mut args = profile.arguments();
        args["mkn"] = json!(devices);
        Self::argument(Command::MotionDeviceConfig, "g", args)
    }

    /// Calibration is an explicit operation; this constructor neither starts it nor sends I/O.
    pub fn calibration(kind: u8, end: bool) -> Result<Self> {
        if kind > 1 {
            return Err(Error::Invalid("calibration type must be 0 or 1"));
        }
        Self::argument(Command::Calibration, "s", json!({"t":kind,"a":u8::from(end)}))
    }

    pub fn macro_recording(start: bool) -> Result<Self> {
        Self::argument(Command::MacroRecording, "s", json!(u8::from(start)))
    }

    pub fn reset_config(slot: u8, kind: u8, sub_type: u8) -> Result<Self> {
        Profile::new(slot, 0)?;
        Self::argument(Command::ResetConfig, "s", json!({"b":slot,"t":kind,"st":sub_type}))
    }
}

impl Change {
    pub fn dual_output(enabled: bool) -> Result<Self> {
        scalar(Command::SetDualOutput, Command::DualOutput, u8::from(enabled).into())
    }

    pub fn polling_rate(rate: u16) -> Result<Self> {
        if !matches!(rate, 125 | 250 | 500 | 1000) {
            return Err(Error::Invalid("unsupported polling rate"));
        }
        scalar(Command::SetPollingRate, Command::PollingRate, rate.into())
    }

    /// Zero is the vendor's adaptive setting, not a zero-step stick.
    pub fn step_size(size: u16) -> Result<Self> {
        if !matches!(size, 0 | 32 | 64 | 128 | 256) {
            return Err(Error::Invalid("unsupported step size"));
        }
        scalar(Command::SetStepSize, Command::StepSize, size.into())
    }

    /// Switching modes can re-enumerate the USB interface, so `Error::Unverified` after this write
    /// means reconnect and read c42 again. Its `l` (connection type) is reported, not compared.
    pub fn controller_mode(mode: ControllerMode) -> Result<Self> {
        scalar(Command::SetControllerMode, Command::ControllerMode, json!({"m":mode as u8}))
    }

    /// The complete c48 object: `std` (stick/D-pad swap) and `ddl` (diagonal lock) are 0/1, and
    /// `ljod`/`rjod` are default 0, four-way 1, eight-way 2. Other returned fields are kept.
    pub fn joystick_dpad(settings: Value) -> Result<Self> {
        let within = |key: &str, max: u64| settings.get(key).and_then(Value::as_u64).is_some_and(|v| v <= max);
        if !(within("std", 1) && within("ddl", 1) && within("ljod", 2) && within("rjod", 2)) {
            return Err(Error::Invalid("D-pad settings need std/ddl 0..=1 and ljod/rjod 0..=2"));
        }
        scalar(Command::SetJoystickDpad, Command::JoystickDpad, settings)
    }

    pub fn profile_name(slot: u8, name: &str, summary: &Value) -> Result<Self> {
        let version = summary_version(slot, summary)?;
        Ok(Self {
            write: Request::argument(Command::SetProfileName, "s", json!({"b":slot,"n":name}))?,
            read: Request::profile_summary(slot)?,
            expected: json!({"b":slot, version:{"n":name}}),
        })
    }

    pub fn activate_profile(slot: u8, summary: &Value) -> Result<Self> {
        let version = summary_version(slot, summary)?;
        Ok(Self {
            write: Request::argument(Command::ActivateProfile, "s", slot.into())?,
            read: Request::profile_summary(slot)?,
            expected: json!({"b":slot, version:{"a":1}}),
        })
    }

    /// `settings` is the complete c54 `s` object; the device summary supplies the other sub-config.
    /// Only fields the c51 summary reports are compared: V54 omits `kao`, so a key-logic change is
    /// written and acknowledged there but cannot be read back.
    pub fn sub_config(settings: Value, summary: &Value) -> Result<Self> {
        let slot =
            settings.get("b").and_then(Value::as_u64).and_then(|v| u8::try_from(v).ok()).ok_or(Error::Invalid("sub-config requires its profile slot"))?;
        summary_version(slot, summary)?;
        let sub =
            settings.get("c").and_then(Value::as_u64).filter(|v| (1..=2).contains(v)).ok_or(Error::Invalid("only Sub1 and Sub2 have activation triggers"))?;
        if ["ko", "at", "kao"].iter().any(|k| settings.get(k).and_then(Value::as_u64).is_none_or(|v| v > 255))
            || !settings.get("k").is_some_and(Value::is_array)
        {
            return Err(Error::Invalid("complete sub-config activation fields required"));
        }
        let mut cfg = summary
            .get("d2")
            .and_then(|v| v.get("cfg"))
            .and_then(Value::as_array)
            .ok_or(Error::Invalid("legacy summary cannot verify sub-config activation fields"))?
            .clone();
        let entry = cfg.iter_mut().find(|v| v.get("c").and_then(Value::as_u64) == Some(sub)).ok_or(Error::Invalid("sub-config absent from device summary"))?;
        let target = entry.as_object_mut().ok_or(Error::Invalid("sub-config summary must be an object"))?;
        if ["ko", "at", "k"].iter().any(|key| !target.contains_key(*key)) {
            return Err(Error::Invalid("summary does not report the sub-config activation fields"));
        }
        for (key, value) in settings.as_object().ok_or(Error::Invalid("sub-config must be an object"))? {
            if let Some(reported) = target.get_mut(key) {
                *reported = value.clone();
            }
        }
        Ok(Self {
            write: Request::argument(Command::SetSubConfig, "s", settings)?,
            read: Request::profile_summary(slot)?,
            expected: json!({"b":slot,"d2":{"cfg":cfg}}),
        })
    }

    /// Appends a macro. An existing name must be deleted and its absence verified before replacement.
    pub fn classic_macro(name: &str, steps: &[MacroStep]) -> Result<Self> {
        check_macro_name(name)?;
        if steps.is_empty() {
            return Err(Error::Invalid("a macro needs at least one step"));
        }
        if steps.iter().any(|step| step.interval > u16::MAX.into() || step.duration > u16::MAX.into()) {
            return Err(Error::Invalid("macro step timing must fit u16 milliseconds"));
        }
        let count = u16::try_from(steps.len()).map_err(|_| Error::Invalid("macro exceeds u16 step count"))?;
        let data: Vec<u32> = steps.iter().flat_map(|s| [s.device as u32, s.key as u32, s.interval, s.duration, 1]).collect();
        let expected = json!({"n":name,"s":count,"t":0,"d":data});
        Ok(Self { write: Request::argument(Command::WriteMacro, "w", expected.clone())?, read: Request::macro_detail(name)?, expected })
    }

    /// `summary` is a fresh c170 result; the read-back expects it without `name`.
    pub fn delete_macro(name: &str, summary: &Value) -> Result<Self> {
        let mut remaining = summary.as_array().ok_or(Error::Invalid("macro summary must be an array"))?.clone();
        let before = remaining.len();
        remaining.retain(|entry| entry.get("n").and_then(Value::as_str) != Some(name));
        if remaining.len() == before {
            return Err(Error::Invalid("macro is not in the device summary"));
        }
        Ok(Self {
            write: Request::argument(Command::DeleteMacro, "n", name.into())?,
            read: Request::query(Command::MacroSummary),
            expected: Value::Array(remaining),
        })
    }
}

fn check_macro_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 32 || name.as_bytes().contains(&0) {
        return Err(Error::Invalid("macro name must contain 1..=32 UTF-8 bytes without NUL"));
    }
    Ok(())
}

fn scalar(write: Command, read: Command, expected: Value) -> Result<Change> {
    Ok(Change { write: Request::argument(write, "s", expected.clone())?, read: Request::query(read), expected })
}

fn summary_version(slot: u8, summary: &Value) -> Result<&'static str> {
    Profile::new(slot, 0)?;
    if summary.get("b").and_then(Value::as_u64) != Some(slot.into()) {
        return Err(Error::Invalid("profile summary belongs to a different slot"));
    }
    ["d2", "d1"].into_iter().find(|key| summary.get(key).is_some_and(Value::is_object)).ok_or(Error::Invalid("profile summary has neither d1 nor d2"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode, dry_run, frame::Decoder};

    fn wire(request: &Request) -> Value {
        let mut decoder = Decoder::default();
        let mut payload = None;
        for report in request.reports().unwrap() {
            payload = decoder.push(&report).unwrap();
        }
        decode(&payload.unwrap()).unwrap()
    }

    #[test]
    fn global_setting_pairs_match_the_vendor_maps() {
        for (change, write, read, value) in [
            (Change::dual_output(true).unwrap(), 31, 30, json!(1)),
            (Change::polling_rate(1000).unwrap(), 45, 44, json!(1000)),
            (Change::step_size(0).unwrap(), 47, 46, json!(0)),
            (Change::controller_mode(ControllerMode::Ps4).unwrap(), 43, 42, json!({"m":2})),
            (Change::joystick_dpad(json!({"std":1,"ljod":0,"rjod":1,"ddl":0})).unwrap(), 49, 48, json!({"std":1,"ljod":0,"rjod":1,"ddl":0})),
        ] {
            assert_eq!(wire(&change.write), json!({"c":write,"s":value}));
            assert_eq!(wire(&change.read), json!({"c":read}));
            assert_eq!(change.expected, value);
            dry_run(&change).unwrap();
        }
        assert!(Change::polling_rate(8000).is_err());
        assert!(Change::step_size(1).is_err());
        for dpad in [json!({"std":2,"ljod":0,"rjod":0,"ddl":0}), json!({"std":0,"ljod":3,"rjod":0,"ddl":0}), json!({"std":0,"ljod":0,"rjod":0})] {
            assert!(Change::joystick_dpad(dpad).is_err());
        }
    }

    #[test]
    fn every_profile_domain_preserves_complete_objects_and_selectors() {
        let profile = Profile::new(1, 2).unwrap();
        for (setter, read) in [
            (Command::SetButton, Request::buttons(profile, &[1, 24]).unwrap()),
            (Command::SetJoystick, Request::joystick(profile, 1).unwrap()),
            (Command::SetTrigger, Request::profile_settings(Command::Trigger, profile).unwrap()),
            (Command::SetMotion, Request::profile_settings(Command::Motion, profile).unwrap()),
            (Command::SetVibration, Request::profile_settings(Command::Vibration, profile).unwrap()),
            (Command::SetFps, Request::fps(profile, &[0, 1, 2]).unwrap()),
            (Command::SetMotionDeviceConfig, Request::motion_device_config(profile, &[1, 4]).unwrap()),
        ] {
            let mut settings = json!({"d1":{"untouched":71,"cv":[0,0,100,100]}});
            for key in ["b", "c", "d"] {
                if let Some(selector) = read.value()["g"].get(key) {
                    settings[key] = selector.clone();
                }
            }
            let change = Change::profile_settings(setter, read.clone(), settings.clone()).unwrap();
            assert_eq!(wire(&change.write)["w"]["d1"], settings["d1"]);
            dry_run(&change).unwrap();
            settings["b"] = json!(2);
            assert!(Change::profile_settings(setter, read, settings).is_err());
        }
        let trigger = Request::profile_settings(Command::Trigger, profile).unwrap();
        assert!(Change::profile_settings(Command::SetMotion, trigger, json!({"b":1,"c":2,"d1":{}})).is_err());
    }

    #[test]
    fn sub_config_reads_back_only_the_fields_the_summary_reports() {
        let settings = json!({"b":1,"c":1,"ko":1,"at":2,"kao":1,"k":[24,25]});
        let v54 = json!({"b":1,"d2":{"a":1,"n":"P1","cfg":[{"c":1,"ko":0,"at":0,"k":[24]},{"c":2,"ko":0,"at":0,"k":[]}]}});
        let change = Change::sub_config(settings.clone(), &v54).unwrap();
        assert_eq!(wire(&change.write), json!({"c":54,"s":settings}));
        assert_eq!(wire(&change.read), json!({"c":51,"b":1}));
        assert_eq!(change.expected, json!({"b":1,"d2":{"cfg":[{"c":1,"ko":1,"at":2,"k":[24,25]},{"c":2,"ko":0,"at":0,"k":[]}]}}));
        let mut reports_logic = v54.clone();
        reports_logic["d2"]["cfg"][0]["kao"] = json!(0);
        assert_eq!(Change::sub_config(settings.clone(), &reports_logic).unwrap().expected["d2"]["cfg"][0]["kao"], 1);
        let mut main = settings.clone();
        main["c"] = json!(0);
        assert!(Change::sub_config(main, &v54).is_err());
        assert!(Change::sub_config(settings, &json!({"b":1,"d1":{"a":1,"n":"P1","k1":[],"k2":[]}})).is_err());
    }

    #[test]
    fn profile_names_and_activation_follow_the_reported_summary_version() {
        for version in ["d1", "d2"] {
            let summary = json!({"b":1,version:{"a":0,"n":"Old"}});
            let renamed = Change::profile_name(1, "New", &summary).unwrap();
            assert_eq!(wire(&renamed.write), json!({"c":53,"s":{"b":1,"n":"New"}}));
            assert_eq!(renamed.expected[version]["n"], "New");
            assert_eq!(Change::activate_profile(1, &summary).unwrap().expected[version]["a"], 1);
            assert!(Change::profile_name(2, "New", &summary).is_err());
        }
    }

    #[test]
    fn button_actions_and_macros_use_the_vendor_flat_step_format() {
        let action = ButtonAction::mapping(2, [1, 2, 0, 0], 20).unwrap();
        assert_eq!(json!(action), json!({"t":2,"v":[1,2,0,0],"n":"","r":2,"sp":20,"mr":0,"ms":0,"ri":0}));
        assert!(ButtonAction::mapping(1, [0; 4], 0).is_err());
        assert_eq!(
            json!(ButtonAction::macro_action("Jump".into(), 4, 1, 300).unwrap()),
            json!({"t":1,"v":[0,0,0,0],"n":"Jump","r":1,"sp":0,"mr":4,"ms":1,"ri":300})
        );
        assert!(ButtonAction::macro_action("Jump".into(), 5, 1, 300).is_err());
        let step = MacroStep { device: MacroDevice::Gamepad, key: 1, interval: 40, duration: 80 };
        let change = Change::classic_macro("Jump", &[step]).unwrap();
        assert_eq!(wire(&change.write), json!({"c":172,"w":{"n":"Jump","s":1,"t":0,"d":[0,1,40,80,1]}}));
        assert_eq!(wire(&change.read), json!({"c":171,"n":"Jump"}));
        assert!(Change::classic_macro("Jump", &[]).is_err());
        let summary = json!([{"n":"Jump","s":1,"k":[0,0]}]);
        let deleted = Change::delete_macro("Jump", &summary).unwrap();
        assert_eq!(wire(&deleted.write), json!({"c":173,"n":"Jump"}));
        assert_eq!(deleted.expected, json!([]));
        dry_run(&deleted).unwrap();
        assert!(Change::delete_macro("Missing", &summary).is_err());
    }

    #[test]
    fn macro_names_and_timing_match_firmware_storage_limits() {
        let step = MacroStep { device: MacroDevice::Gamepad, key: 1, interval: 65535, duration: 65535 };
        let name = "é".repeat(16);
        assert!(Change::classic_macro(&name, &[step]).is_ok());
        assert!(ButtonAction::macro_action(name, 3, 1, 65535).is_ok());
        for name in [String::new(), "a".repeat(33), "é".repeat(17), "A\0B".into()] {
            assert!(Change::classic_macro(&name, &[step]).is_err());
            assert!(ButtonAction::macro_action(name, 1, 1, 0).is_err());
        }
        assert!(Change::classic_macro("Jump", &[MacroStep { interval: 65536, ..step }]).is_err());
        assert!(Change::classic_macro("Jump", &[MacroStep { duration: 65536, ..step }]).is_err());
    }
}
