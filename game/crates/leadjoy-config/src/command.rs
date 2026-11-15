//! MessagePack command maps. Numeric ids are from the PC DLL's command-name table.

use serde::{Deserialize, Serialize, ser::SerializeMap};
use serde_json::{Map, Value, json};

use crate::{Error, Result, frame::Report};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Command {
    DeviceReset = 10,
    DeviceShutdown = 11,
    DeviceVersion = 12,
    DeviceId = 13,
    Battery = 14,
    SaveSettings = 15,
    EnterEditMode = 16,
    ProductName = 17,
    DualOutput = 30,
    SetDualOutput = 31,
    ButtonReporting = 40,
    Calibration = 41,
    ControllerMode = 42,
    SetControllerMode = 43,
    PollingRate = 44,
    SetPollingRate = 45,
    StepSize = 46,
    SetStepSize = 47,
    JoystickDpad = 48,
    SetJoystickDpad = 49,
    ProfileCount = 50,
    ProfileSummary = 51,
    ActivateProfile = 52,
    SetProfileName = 53,
    SetSubConfig = 54,
    ResetConfig = 55,
    ReloadConfig = 56,
    Button = 60,
    SetButton = 61,
    Joystick = 70,
    SetJoystick = 71,
    Trigger = 90,
    SetTrigger = 91,
    Motion = 100,
    SetMotion = 101,
    Vibration = 120,
    SetVibration = 121,
    Fps = 130,
    SetFps = 131,
    FpsList = 132,
    DeleteFps = 133,
    FpsDetail = 134,
    WriteFps = 135,
    MacroSummary = 170,
    MacroDetail = 171,
    WriteMacro = 172,
    DeleteMacro = 173,
    MacroRecording = 174,
    MacroStep = 175,
    Lighting = 190,
    JoystickLighting = 191,
    SetJoystickLighting = 192,
    MotionDevices = 220,
    MotionDeviceConfig = 221,
    SetMotionDeviceConfig = 222,
    PairMotionDevice = 224,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    XenoPlus,
    XenoPro,
    XenoSe,
    Saber,
}

impl Model {
    /// PC 1.4.0 enables RGB-list effects only for these two models, not Xeno Plus/Pro.
    pub fn supports_led_frames(self) -> bool {
        matches!(self, Self::XenoSe | Self::Saber)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    slot: u8,
    sub_config: u8,
}

impl Profile {
    pub fn new(slot: u8, sub_config: u8) -> Result<Self> {
        if !(1..=4).contains(&slot) || sub_config > 2 {
            return Err(Error::Invalid("profile slot must be 1..=4 and sub-config 0..=2"));
        }
        Ok(Self { slot, sub_config })
    }
    pub fn arguments(self) -> Value {
        json!({"b": self.slot, "c": self.sub_config})
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    command: Command,
    fields: Map<String, Value>,
}

impl Serialize for Request {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.fields.len() + 1))?;
        map.serialize_entry("c", &(self.command as u8))?;
        for (key, value) in &self.fields {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl Request {
    /// Encodes known commands with their documented short-key fields. The argument schema remains
    /// visible rather than inventing defaults for firmware settings not read from the controller.
    pub fn new(command: Command, fields: Map<String, Value>) -> Result<Self> {
        if fields.contains_key("c") {
            return Err(Error::Invalid("command id cannot be overridden"));
        }
        Ok(Self { command, fields })
    }

    pub fn query(command: Command) -> Self {
        Self { command, fields: Map::new() }
    }

    pub fn argument(command: Command, key: &str, value: Value) -> Result<Self> {
        let mut fields = Map::new();
        fields.insert(key.into(), value);
        Self::new(command, fields)
    }

    pub fn command(&self) -> Command {
        self.command
    }

    pub fn value(&self) -> Value {
        let mut fields = self.fields.clone();
        fields.insert("c".into(), json!(self.command as u8));
        Value::Object(fields)
    }

    pub fn reports(&self) -> Result<Vec<Report>> {
        crate::frame::encode(&rmp_serde::to_vec_named(self).map_err(Error::Encode)?)
    }

    pub fn profile_summary(slot: u8) -> Result<Self> {
        Profile::new(slot, 0)?;
        Self::argument(Command::ProfileSummary, "b", json!(slot))
    }

    pub fn profile_settings(command: Command, profile: Profile) -> Result<Self> {
        if !matches!(command, Command::Trigger | Command::Motion | Command::Vibration) {
            return Err(Error::Invalid("this query requires additional selectors"));
        }
        Self::argument(command, "g", profile.arguments())
    }

    pub fn buttons(profile: Profile, keys: &[u8]) -> Result<Self> {
        let mut args = profile.arguments();
        args["k"] = json!(keys);
        Self::argument(Command::Button, "g", args)
    }

    /// Raw left/right direction discriminator (0 or 1), not a bitmask or a "both" selector.
    pub fn joystick(profile: Profile, side: u8) -> Result<Self> {
        check_side(side)?;
        let mut args = profile.arguments();
        args["d"] = json!(side);
        Self::argument(Command::Joystick, "g", args)
    }

    pub fn lighting() -> Self {
        let mut fields = Map::new();
        fields.insert("a".into(), json!(0));
        Self { command: Command::Lighting, fields }
    }

    pub fn joystick_lighting(model: Model, side: u8) -> Result<Self> {
        check_led_support(model, side)?;
        Self::argument(Command::JoystickLighting, "g", json!({"d": side}))
    }

    /// Registered in Xeno Plus V54, but the physical LEDs it addresses remain unverified.
    /// This opt-in is for a supervised investigation, not the normal settings UI.
    pub fn unverified_xeno_plus_joystick_lighting(side: u8) -> Result<Self> {
        check_side(side)?;
        Self::argument(Command::JoystickLighting, "g", json!({"d": side}))
    }

    /// True only for non-mutating queries; used to guard a change's read-back request.
    pub fn is_read(&self) -> bool {
        match self.command {
            Command::Lighting => self.fields.get("a") == Some(&json!(0)) && !self.fields.contains_key("s"),
            Command::DeviceVersion
            | Command::DeviceId
            | Command::Battery
            | Command::ProductName
            | Command::DualOutput
            | Command::ControllerMode
            | Command::PollingRate
            | Command::StepSize
            | Command::JoystickDpad
            | Command::ProfileCount
            | Command::ProfileSummary
            | Command::Button
            | Command::Joystick
            | Command::Trigger
            | Command::Motion
            | Command::Vibration
            | Command::Fps
            | Command::FpsList
            | Command::FpsDetail
            | Command::MacroSummary
            | Command::MacroDetail
            | Command::JoystickLighting
            | Command::MotionDevices
            | Command::MotionDeviceConfig => true,
            _ => false,
        }
    }
}

fn check_side(side: u8) -> Result<()> {
    if side > 1 {
        return Err(Error::Invalid("side must be a zero-based direction (0 or 1)"));
    }
    Ok(())
}

fn check_led_support(model: Model, side: u8) -> Result<()> {
    check_side(side)?;
    if !model.supports_led_frames() {
        return Err(Error::Invalid("RGB-list commands are not verified on Xeno Plus/Pro"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lighting {
    #[serde(rename = "brt")]
    pub brightness: u8,
    pub rgb: [u8; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LightingMode {
    Off = 0,
    Steady = 2,
    Gradient = 3,
    Breathe = 4,
    Trailing = 5,
}

/// The V54 firmware stores ten RGB triplets per side, not an arbitrary-size image.
pub const MAX_LED_COLORS: usize = 10;

/// Ordered RGB colours for the joystick effect engine, not a proven pixel-addressed logo bitmap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedFrame {
    pub brightness: u8,
    pub mode: LightingMode,
    pub speed: u16,
    pub colors: Vec<[u8; 3]>,
}

impl LedFrame {
    pub fn value(&self) -> Value {
        let rgb = Value::Array(self.colors.iter().flatten().map(|v| Value::from(*v)).collect());
        json!({"brt": self.brightness, "m": self.mode as u8, "s": self.speed, "rgb": rgb})
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        let brightness = byte(value.get("brt"))?;
        let mode = match byte(value.get("m"))? {
            0 => LightingMode::Off,
            2 => LightingMode::Steady,
            3 => LightingMode::Gradient,
            4 => LightingMode::Breathe,
            5 => LightingMode::Trailing,
            _ => return Err(Error::Invalid("unknown lighting effect mode")),
        };
        let speed = value.get("s").and_then(Value::as_u64).and_then(|v| u16::try_from(v).ok()).ok_or(Error::Invalid("lighting speed must fit u16"))?;
        let rgb = value.get("rgb").and_then(Value::as_array).ok_or(Error::Invalid("RGB array missing"))?;
        let mut colors = Vec::new();
        if rgb.first().is_some_and(Value::is_array) {
            if rgb.len() > MAX_LED_COLORS {
                return Err(Error::Invalid("LED palette exceeds ten colours"));
            }
            colors.reserve(rgb.len());
            for color in rgb {
                let a = color.as_array().filter(|a| a.len() == 3).ok_or(Error::Invalid("RGB triplet length"))?;
                colors.push([byte(a.first())?, byte(a.get(1))?, byte(a.get(2))?]);
            }
        } else {
            if rgb.len() % 3 != 0 {
                return Err(Error::Invalid("flat RGB array length must be a multiple of three"));
            }
            if rgb.len() / 3 > MAX_LED_COLORS {
                return Err(Error::Invalid("LED palette exceeds ten colours"));
            }
            colors.reserve(rgb.len() / 3);
            for color in rgb.chunks_exact(3) {
                colors.push([byte(color.first())?, byte(color.get(1))?, byte(color.get(2))?]);
            }
        }
        Ok(Self { brightness, mode, speed, colors })
    }
}

fn byte(value: Option<&Value>) -> Result<u8> {
    value.and_then(Value::as_u64).and_then(|v| u8::try_from(v).ok()).ok_or(Error::Invalid("expected an unsigned byte"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub write: Request,
    pub read: Request,
    pub expected: Value,
}

impl Change {
    pub fn lighting(value: Lighting) -> Self {
        let expected = json!(value);
        let mut fields = Map::new();
        fields.insert("a".into(), json!(1));
        fields.insert("s".into(), expected.clone());
        Self { write: Request { command: Command::Lighting, fields }, read: Request::lighting(), expected }
    }

    pub fn led_frame(model: Model, side: u8, frame: &LedFrame) -> Result<Self> {
        Self::palette(Request::joystick_lighting(model, side)?, side, frame)
    }

    /// API-only: V54 contains c191/c192, but their effect on a real Xeno Plus is unverified.
    /// Save both sides first and restore them afterwards; these are persistable settings.
    pub fn unverified_xeno_plus_led_frame(side: u8, frame: &LedFrame) -> Result<Self> {
        Self::palette(Request::unverified_xeno_plus_joystick_lighting(side)?, side, frame)
    }

    fn palette(read: Request, side: u8, frame: &LedFrame) -> Result<Self> {
        if frame.colors.len() > MAX_LED_COLORS {
            return Err(Error::Invalid("LED palette exceeds ten colours"));
        }
        let expected = json!({"d": side, "d1": frame.value()});
        let write = Request::argument(Command::SetJoystickLighting, "s", expected.clone())?;
        Ok(Self { write, read, expected })
    }

    /// These setters replace complete returned configuration objects. The caller must preserve
    /// untouched fields and selectors from a fresh read, rather than constructing partial defaults.
    pub fn profile_settings(command: Command, read: Request, settings: Value) -> Result<Self> {
        if !matches!(
            (command, read.command),
            (Command::SetButton, Command::Button)
                | (Command::SetJoystick, Command::Joystick)
                | (Command::SetTrigger, Command::Trigger)
                | (Command::SetMotion, Command::Motion)
                | (Command::SetVibration, Command::Vibration)
                | (Command::SetFps, Command::Fps)
                | (Command::SetMotionDeviceConfig, Command::MotionDeviceConfig)
        ) || !settings.is_object()
        {
            return Err(Error::Invalid("setting write/read-back command mismatch"));
        }
        let selectors = read.fields.get("g").and_then(Value::as_object).ok_or(Error::Invalid("profile read-back requires selectors"))?;
        for key in ["b", "c", "d"] {
            if let Some(value) = selectors.get(key) {
                if settings.get(key) != Some(value) {
                    return Err(Error::Invalid("write and read-back profile selectors differ"));
                }
            }
        }
        Ok(Self { write: Request::argument(command, "w", settings.clone())?, read, expected: settings })
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Response {
    #[serde(rename = "c")]
    pub command: u8,
    #[serde(rename = "e")]
    pub error: u8,
    #[serde(rename = "r", default)]
    pub result: Value,
}

impl Response {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        serde_json::from_value(crate::decode(payload)?).map_err(|_| Error::Invalid("response requires c and e"))
    }
    pub fn ensure_success(&self) -> Result<()> {
        if self.error != 0 {
            return Err(Error::Device(self.error));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Decoder;

    fn wire(request: &Request) -> Value {
        let mut decoder = Decoder::default();
        let mut payload = None;
        for report in request.reports().unwrap() {
            payload = decoder.push(&report).unwrap();
        }
        crate::decode(&payload.unwrap()).unwrap()
    }

    #[test]
    fn documented_profile_requests_and_selectors() {
        let profile = Profile::new(2, 1).unwrap();
        assert_eq!(wire(&Request::buttons(profile, &[1, 24]).unwrap()), json!({"c":60,"g":{"b":2,"c":1,"k":[1,24]}}));
        assert_eq!(wire(&Request::joystick(profile, 0).unwrap()), json!({"c":70,"g":{"b":2,"c":1,"d":0}}));
        assert_eq!(wire(&Request::profile_summary(4).unwrap()), json!({"c":51,"b":4}));
        assert!(Profile::new(0, 0).is_err());
        assert!(Profile::new(5, 0).is_err());
        assert!(Profile::new(1, 3).is_err());
        assert!(Request::joystick(profile, 3).is_err());
        assert!(Request::argument(Command::Battery, "c", json!(255)).is_err());
    }

    #[test]
    fn logo_is_one_rgb_triplet_and_has_read_back() {
        let change = Change::lighting(Lighting { brightness: 80, rgb: [255, 0, 32] });
        assert_eq!(wire(&change.write), json!({"c":190,"a":1,"s":{"brt":80,"rgb":[255,0,32]}}));
        assert_eq!(wire(&change.read), json!({"c":190,"a":0}));
        assert!(!change.write.is_read());
        assert!(change.read.is_read());
    }

    #[test]
    fn led_lists_flatten_rgb_and_refuse_unproven_models() {
        let frame = LedFrame { brightness: 255, mode: LightingMode::Trailing, speed: 500, colors: vec![[1, 2, 3], [4, 5, 6]] };
        let change = Change::led_frame(Model::Saber, 1, &frame).unwrap();
        assert_eq!(wire(&change.write), json!({"c":192,"s":{"d":1,"d1":{"brt":255,"m":5,"s":500,"rgb":[1,2,3,4,5,6]}}}));
        assert_eq!(wire(&change.read), json!({"c":191,"g":{"d":1}}));
        assert_eq!(LedFrame::from_value(&frame.value()).unwrap(), frame);
        let mut nested = frame.value();
        nested["rgb"] = json!([[1, 2, 3], [4, 5, 6]]);
        assert_eq!(LedFrame::from_value(&nested).unwrap(), frame);
        nested["rgb"] = json!([1, 2]);
        assert!(LedFrame::from_value(&nested).is_err());
        nested["rgb"] = json!([1, 2, 256]);
        assert!(LedFrame::from_value(&nested).is_err());
        assert!(Change::led_frame(Model::XenoPlus, 0, &frame).is_err());
        assert!(Change::led_frame(Model::XenoPro, 0, &frame).is_err());
        let mut full = frame.clone();
        full.colors = vec![[1, 2, 3]; MAX_LED_COLORS];
        let experimental = Change::unverified_xeno_plus_led_frame(0, &full).unwrap();
        assert_eq!(wire(&experimental.write)["s"]["d1"]["rgb"].as_array().unwrap().len(), 30);
        assert_eq!(wire(&experimental.read), json!({"c":191,"g":{"d":0}}));
        full.colors.push([4, 5, 6]);
        assert!(Change::unverified_xeno_plus_led_frame(0, &full).is_err());
        assert!(LedFrame::from_value(&full.value()).is_err());
        assert!(Request::unverified_xeno_plus_joystick_lighting(2).is_err());
    }

    #[test]
    fn reply_validates_error_and_trailing_bytes() {
        let payload = [0x83, 0xa1, b'c', 12, 0xa1, b'e', 0, 0xa1, b'r', 1];
        let reply = Response::decode(&payload).unwrap();
        assert_eq!(reply.command, 12);
        assert_eq!(reply.result, json!(1));
        reply.ensure_success().unwrap();
        let mut invalid = payload.to_vec();
        invalid.push(0);
        assert!(Response::decode(&invalid).is_err());
        invalid = payload.to_vec();
        invalid[6] = 2;
        assert!(matches!(Response::decode(&invalid).unwrap().ensure_success(), Err(Error::Device(2))));
        let acknowledgement = Response::decode(&[0x82, 0xa1, b'c', 61, 0xa1, b'e', 0]).unwrap();
        assert!(acknowledgement.result.is_null());
        acknowledgement.ensure_success().unwrap();
    }
}
