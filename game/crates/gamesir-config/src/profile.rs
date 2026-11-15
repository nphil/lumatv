//! The two settings blocks, at the offsets GameSir Connect 1.14.5 reads and writes (`$a.fromBytes`/`fromModel` for profiles, `qo`
//! for lights). Setters accept only what Connect's pages offer and encode it the way Connect does; bytes they do not cover are kept.

use crate::Error;
use crate::frame::{LIGHT_LEN, PROFILE_LEN};

const NAME_LEN: usize = 32;
const MOTORS: usize = 32;
const REPORT_RATE: usize = 46;
const BUTTONS: usize = 64;
const BUTTON_LEN: usize = 7;
const L4: usize = 176;
const R4: usize = 335;
const TRIGGERS: usize = 494;
const TRIGGER_LEN: usize = 28;
const STICKS: usize = 550;
const STICK_LEN: usize = 32;
const HAIR_ON: u8 = 0x80;

fn check(setting: &'static str, value: u8, max: u8) -> Result<(), Error> {
    if value <= max { Ok(()) } else { Err(Error::Range { setting, value: value.into() }) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportRate {
    Hz250,
    Hz500,
    Hz1000,
}

impl ReportRate {
    pub fn hz(self) -> u32 {
        match self {
            ReportRate::Hz250 => 250,
            ReportRate::Hz500 => 500,
            ReportRate::Hz1000 => 1000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    /// Lets the d-pad report only the four straight directions.
    DpadDiagonalLock = 43,
    /// Swaps A/B and X/Y in X-input mode.
    XinputAbxySwap = 44,
    /// Swaps A/B and X/Y in Switch mode.
    SwitchAbxySwap = 45,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// The buttons with a remap and turbo entry, in the order the profile stores them. Face buttons go by their Cyclone 2 labels; Connect
/// names them after the PlayStation symbols (A is cross, B circle, X square, Y triangle).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    Lb,
    Rb,
    Ls,
    Rs,
    A,
    B,
    X,
    Y,
    Home,
    View,
    Menu,
    Share,
    L4,
    R4,
}

impl Button {
    pub const ALL: [Button; 18] = [
        Button::Up,
        Button::Down,
        Button::Left,
        Button::Right,
        Button::Lb,
        Button::Rb,
        Button::Ls,
        Button::Rs,
        Button::A,
        Button::B,
        Button::X,
        Button::Y,
        Button::Home,
        Button::View,
        Button::Menu,
        Button::Share,
        Button::L4,
        Button::R4,
    ];

    pub fn name(self) -> &'static str {
        OUTPUTS[self as usize].0
    }

    pub fn from_name(name: &str) -> Option<Button> {
        Self::ALL.into_iter().find(|b| b.name() == name)
    }

    fn offset(self) -> usize {
        match self {
            Button::L4 => L4,
            Button::R4 => R4,
            b => BUTTONS + BUTTON_LEN * b as usize,
        }
    }
}

/// Gamepad outputs a button can send, with Connect's codes. Codes 50-155 are keyboard keys and 200-206 mouse buttons and wheel.
const OUTPUTS: [(&str, u8); 29] = [
    ("up", 1),
    ("down", 2),
    ("left", 3),
    ("right", 4),
    ("lb", 5),
    ("rb", 6),
    ("ls", 7),
    ("rs", 8),
    ("a", 9),
    ("b", 10),
    ("x", 11),
    ("y", 12),
    ("home", 13),
    ("view", 14),
    ("menu", 15),
    ("share", 16),
    ("l4", 17),
    ("r4", 18),
    ("lt", 19),
    ("rt", 20),
    ("ls-up", 21),
    ("ls-down", 22),
    ("ls-left", 23),
    ("ls-right", 24),
    ("rs-up", 25),
    ("rs-down", 26),
    ("rs-left", 27),
    ("rs-right", 28),
    ("disabled", 255),
];

pub fn output_code(name: &str) -> Option<u8> {
    OUTPUTS.iter().find(|(n, _)| *n == name).map(|&(_, code)| code)
}

pub fn output_name(code: u8) -> Option<&'static str> {
    OUTPUTS.iter().find(|&&(_, c)| c == code).map(|&(name, _)| name)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonConfig {
    /// Presses per second while held, 0 for off. Connect offers up to 30.
    pub turbo_hz: u8,
    /// Output codes the button sends, up to three at once; all zero keeps the button's own function.
    pub outputs: [u8; 3],
    /// One press holds the output, the next releases it.
    pub toggle: bool,
}

/// Percent of travel, 0-100. Input below `inner` reads as rest and above `outer` as full; output is scaled into `anti_inner..anti_outer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadzone {
    pub inner: u8,
    pub outer: u8,
    pub anti_inner: u8,
    pub anti_outer: u8,
}

/// Connect's response curve: `kind` 0 linear, 1 concave/convex, 2 S-shaped, 3 custom; `curvature` 0-100. Points are (input, output)
/// in percent of travel and must lie strictly inside the deadzone on both axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Curve {
    pub kind: u8,
    pub curvature: u8,
    pub points: [(u8, u8); 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisResponse {
    pub deadzone: Deadzone,
    pub curve: Curve,
}

impl AxisResponse {
    /// What Connect shows for an axis whose stored deadzone or curve is switched off.
    pub const UNSET: AxisResponse = AxisResponse {
        deadzone: Deadzone { inner: 5, outer: 100, anti_inner: 0, anti_outer: 100 },
        curve: Curve { kind: 0, curvature: 100, points: [(25, 25), (50, 50), (75, 75)] },
    };

    /// Sets the deadzone and pulls curve points that fall outside it to its edges, which keeps the curve valid.
    pub fn with_deadzone(mut self, deadzone: Deadzone) -> Self {
        fn inside(p: u8, low: u8, high: u8) -> u8 {
            if low.saturating_add(1) < high { p.clamp(low + 1, high - 1) } else { p }
        }
        self.deadzone = deadzone;
        for (x, y) in &mut self.curve.points {
            *x = inside(*x, deadzone.inner, deadzone.outer);
            *y = inside(*y, deadzone.anti_inner, deadzone.anti_outer);
        }
        self
    }

    fn check(&self) -> Result<(), Error> {
        let d = self.deadzone;
        check("deadzone", d.outer, 100)?;
        check("anti-deadzone", d.anti_outer, 100)?;
        check("curve type", self.curve.kind, 3)?;
        check("curvature", self.curve.curvature, 100)?;
        for (x, y) in self.curve.points {
            if !(d.inner < x && x < d.outer) {
                return Err(Error::Range { setting: "curve point input (must lie inside the deadzone)", value: x.into() });
            }
            if !(d.anti_inner < y && y < d.anti_outer) {
                return Err(Error::Range { setting: "curve point output (must lie inside the anti-deadzone)", value: y.into() });
            }
        }
        Ok(())
    }

    fn read(b: &[u8], dead: usize, linear: usize) -> Self {
        let raw = Deadzone { inner: b[dead + 1], outer: b[dead + 2], anti_inner: b[dead + 3], anti_outer: b[dead + 4] };
        let deadzone = if b[dead] != 0 { raw } else { Self::UNSET.deadzone };
        // Points are decoded against the stored deadzone bytes even when the deadzone is off, as Connect does.
        let point = |k: usize| (percent(b[linear + 5 + 2 * k], raw.inner, raw.outer), percent(b[linear + 6 + 2 * k], raw.anti_inner, raw.anti_outer));
        let curve =
            if b[linear] != 0 { Curve { kind: b[linear + 1], curvature: b[linear + 2], points: [point(0), point(1), point(2)] } } else { Self::UNSET.curve };
        AxisResponse { deadzone, curve }
    }

    fn write(&self, b: &mut [u8], dead: usize, linear: usize) {
        let d = self.deadzone;
        b[dead..dead + 5].copy_from_slice(&[1, d.inner, d.outer, d.anti_inner, d.anti_outer]);
        b[linear..linear + 3].copy_from_slice(&[1, self.curve.kind, self.curve.curvature]);
        // Five (input, output) pairs scaled to the deadzone: the ends are fixed at (0, 0) and (255, 255).
        b[linear + 3..linear + 5].fill(0);
        for (k, (x, y)) in self.curve.points.into_iter().enumerate() {
            b[linear + 5 + 2 * k] = scaled(x, d.inner, d.outer);
            b[linear + 6 + 2 * k] = scaled(y, d.anti_inner, d.anti_outer);
        }
        b[linear + 11..linear + 13].fill(255);
    }
}

/// JavaScript's `Math.round`: halves go up.
fn js_round(v: f64) -> f64 {
    let floor = v.floor();
    if v - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// Only called with `low < p < high`, so the result stays inside 1..=254.
fn scaled(p: u8, low: u8, high: u8) -> u8 {
    js_round(f64::from(p - low) / f64::from(high - low) * 255.0) as u8
}

fn percent(v: u8, low: u8, high: u8) -> u8 {
    js_round(f64::from(v) / 255.0 * (f64::from(high) - f64::from(low)) + f64::from(low)) as u8
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StickConfig {
    /// Connect's "raw" stick trace; off is its "radial" trace.
    pub raw: bool,
    pub response: AxisResponse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HairMode {
    /// Re-arms as soon as the trigger starts travelling back.
    Rapid = 1,
    /// Fires at a fixed point of travel.
    Fixed = 2,
}

/// The trigger fires within `start..end` percent of travel instead of on the full pull.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HairTrigger {
    pub mode: HairMode,
    pub start: u8,
    pub end: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriggerConfig {
    pub response: AxisResponse,
    pub hair: Option<HairTrigger>,
}

/// One of the four profiles or the shift layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile([u8; PROFILE_LEN]);

impl Profile {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        bytes.try_into().map(Self).map_err(|_| Error::Length { expected: PROFILE_LEN, actual: bytes.len() })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn name(&self) -> String {
        String::from_utf8_lossy(&self.0[..NAME_LEN]).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_owned()
    }

    pub fn report_rate(&self) -> Option<ReportRate> {
        match self.0[REPORT_RATE] {
            0 => Some(ReportRate::Hz250),
            1 => Some(ReportRate::Hz500),
            2 => Some(ReportRate::Hz1000),
            _ => None,
        }
    }

    pub fn set_report_rate(&mut self, rate: ReportRate) {
        self.0[REPORT_RATE] = rate as u8;
    }

    /// Left and right motor strength as Connect's levels 0-4; a stored strength between steps rounds down, as in Connect.
    pub fn rumble(&self) -> (u8, u8) {
        let level = |v: u8| v.min(100) / 25;
        (level(self.0[MOTORS]), level(self.0[MOTORS + 1]))
    }

    /// Levels 0-4, stored as 0, 25, 50, 75 or 100 percent. The trigger motors that follow are kept.
    pub fn set_rumble(&mut self, left: u8, right: u8) -> Result<(), Error> {
        check("rumble level", left, 4)?;
        check("rumble level", right, 4)?;
        self.0[MOTORS] = left * 25;
        self.0[MOTORS + 1] = right * 25;
        Ok(())
    }

    pub fn flag(&self, flag: Flag) -> bool {
        self.0[flag as usize] & 1 != 0
    }

    pub fn set_flag(&mut self, flag: Flag, on: bool) {
        self.0[flag as usize] = u8::from(on);
    }

    pub fn button(&self, button: Button) -> ButtonConfig {
        let b = &self.0[button.offset()..][..BUTTON_LEN];
        ButtonConfig { turbo_hz: if b[0] & 1 != 0 { b[1] } else { 0 }, outputs: if b[2] & 1 != 0 { [b[3], b[4], b[5]] } else { [0; 3] }, toggle: b[6] & 1 != 0 }
    }

    /// The paddles' macro bytes after their first seven are kept.
    pub fn set_button(&mut self, button: Button, config: &ButtonConfig) -> Result<(), Error> {
        check("turbo rate", config.turbo_hz, 30)?;
        let mut outputs = [0; 3];
        for (slot, code) in outputs.iter_mut().zip(config.outputs.into_iter().filter(|&c| c != 0)) {
            *slot = code;
        }
        let mapped = outputs[0] != 0;
        let [a, b, c] = outputs;
        self.0[button.offset()..][..BUTTON_LEN].copy_from_slice(&[
            u8::from(config.turbo_hz > 0),
            config.turbo_hz,
            u8::from(mapped),
            a,
            b,
            c,
            u8::from(config.toggle),
        ]);
        Ok(())
    }

    pub fn stick(&self, side: Side) -> StickConfig {
        let at = STICKS + STICK_LEN * side as usize;
        // A stick block that was never enabled reads as raw in Connect.
        let raw = self.0[at] == 0 || self.0[at + 1] != 0;
        StickConfig { raw, response: AxisResponse::read(&self.0, at + 2, at + 7) }
    }

    /// The stick's remap bytes after the curve are kept.
    pub fn set_stick(&mut self, side: Side, config: &StickConfig) -> Result<(), Error> {
        config.response.check()?;
        let at = STICKS + STICK_LEN * side as usize;
        self.0[at] = 1;
        self.0[at + 1] = u8::from(config.raw);
        config.response.write(&mut self.0, at + 2, at + 7);
        Ok(())
    }

    pub fn trigger(&self, side: Side) -> TriggerConfig {
        let at = TRIGGERS + TRIGGER_LEN * side as usize;
        let status = self.0[at + 12];
        let hair = (status & HAIR_ON != 0).then(|| HairTrigger {
            mode: if status & 1 == 0 && status & 2 != 0 { HairMode::Fixed } else { HairMode::Rapid },
            start: self.0[at + 13],
            end: self.0[at + 14],
        });
        TriggerConfig { response: AxisResponse::read(&self.0, at + 2, at + 15), hair }
    }

    /// Turbo and remap bytes of the trigger are kept; switching the hair trigger off keeps its last range, as Connect does.
    pub fn set_trigger(&mut self, side: Side, config: &TriggerConfig) -> Result<(), Error> {
        config.response.check()?;
        if let Some(hair) = config.hair {
            check("hair trigger end", hair.end, 100)?;
            if hair.start >= hair.end {
                return Err(Error::Range { setting: "hair trigger start (must be below its end)", value: hair.start.into() });
            }
        }
        let at = TRIGGERS + TRIGGER_LEN * side as usize;
        config.response.write(&mut self.0, at + 2, at + 15);
        match config.hair {
            Some(hair) => self.0[at + 12..at + 15].copy_from_slice(&[HAIR_ON | hair.mode as u8, hair.start, hair.end]),
            None => self.0[at + 12] = 0,
        }
        Ok(())
    }
}

const ANIMATIONS: usize = 1;
const ANIMATION_LEN: usize = 124;
const FRAMES: usize = 8;
const FRAME_LEN: usize = 15;
const ZONES: usize = 5;
const MAX_SPEED: u8 = 20;
const STANDBY: usize = 627;

/// Animation slots the light page can make active. The block stores a fifth that Connect never selects.
pub const LIGHT_SLOTS: u8 = 4;
/// Minutes before the lights go idle, 0 for never: the choices Connect offers.
pub const STANDBY_MINUTES: [u8; 5] = [0, 1, 5, 10, 20];

/// The lighting block, shared by all profiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LightProfile([u8; LIGHT_LEN]);

impl LightProfile {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        bytes.try_into().map(Self).map_err(|_| Error::Length { expected: LIGHT_LEN, actual: bytes.len() })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Connect reads an out-of-range slot as slot 0.
    pub fn active_slot(&self) -> u8 {
        if self.0[0] < LIGHT_SLOTS { self.0[0] } else { 0 }
    }

    pub fn set_active_slot(&mut self, slot: u8) -> Result<(), Error> {
        check("light slot", slot, LIGHT_SLOTS - 1)?;
        self.0[0] = slot;
        Ok(())
    }

    fn animation(slot: u8) -> Result<usize, Error> {
        check("light slot", slot, LIGHT_SLOTS - 1)?;
        Ok(ANIMATIONS + ANIMATION_LEN * usize::from(slot))
    }

    pub fn brightness(&self, slot: u8) -> Result<u8, Error> {
        Ok(self.0[Self::animation(slot)? + 3].min(100))
    }

    pub fn set_brightness(&mut self, slot: u8, percent: u8) -> Result<(), Error> {
        check("brightness", percent, 100)?;
        self.0[Self::animation(slot)? + 3] = percent;
        Ok(())
    }

    /// Animation speed 0-20; the block stores 20 minus the speed.
    pub fn speed(&self, slot: u8) -> Result<u8, Error> {
        Ok(MAX_SPEED - self.0[Self::animation(slot)? + 2].min(MAX_SPEED))
    }

    pub fn set_speed(&mut self, slot: u8, speed: u8) -> Result<(), Error> {
        check("light speed", speed, MAX_SPEED)?;
        self.0[Self::animation(slot)? + 2] = MAX_SPEED - speed;
        Ok(())
    }

    /// Replaces the slot's animation with one steady colour on all five zones; unused frames are cleared to black, as Connect does.
    pub fn set_color(&mut self, slot: u8, rgb: [u8; 3]) -> Result<(), Error> {
        let at = Self::animation(slot)?;
        self.0[at] = 1;
        for frame in 0..FRAMES {
            for zone in 0..ZONES {
                let led = at + 4 + FRAME_LEN * frame + 3 * zone;
                self.0[led..led + 3].copy_from_slice(if frame == 0 { &rgb } else { &[0; 3] });
            }
        }
        Ok(())
    }

    pub fn standby_minutes(&self) -> u8 {
        self.0[STANDBY]
    }

    pub fn set_standby_minutes(&mut self, minutes: u8) -> Result<(), Error> {
        if !STANDBY_MINUTES.contains(&minutes) {
            return Err(Error::Range { setting: "light standby minutes", value: minutes.into() });
        }
        self.0[STANDBY] = minutes;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Profile {
        Profile::from_bytes(&[0; PROFILE_LEN]).unwrap()
    }

    const TEN_TO_HUNDRED: Deadzone = Deadzone { inner: 10, outer: 100, anti_inner: 10, anti_outer: 100 };

    #[test]
    fn simple_settings_sit_at_connect_offsets() {
        assert!(matches!(Profile::from_bytes(&[0; 10]), Err(Error::Length { expected: PROFILE_LEN, actual: 10 })));
        let mut p = blank();
        p.set_report_rate(ReportRate::Hz1000);
        assert_eq!(p.as_bytes()[46], 2);
        assert_eq!(p.report_rate(), Some(ReportRate::Hz1000));

        p.set_rumble(4, 1).unwrap();
        assert_eq!(p.as_bytes()[32..34], [100, 25]);
        assert!(p.set_rumble(5, 0).is_err());
        let mut odd = p.as_bytes().to_vec();
        odd[32] = 60;
        assert_eq!(Profile::from_bytes(&odd).unwrap().rumble(), (2, 1), "Connect rounds stored strengths down to a level");

        p.set_flag(Flag::DpadDiagonalLock, true);
        p.set_flag(Flag::SwitchAbxySwap, true);
        assert_eq!(p.as_bytes()[43..46], [1, 0, 1]);
        assert!(p.flag(Flag::DpadDiagonalLock) && !p.flag(Flag::XinputAbxySwap));
    }

    #[test]
    fn name_stops_at_padding() {
        let mut bytes = [0; PROFILE_LEN];
        bytes[..5].copy_from_slice(b"Race ");
        assert_eq!(Profile::from_bytes(&bytes).unwrap().name(), "Race");
    }

    #[test]
    fn stick_curve_encodes_like_connect() {
        let mut p = blank();
        let stick = StickConfig { raw: false, response: AxisResponse { deadzone: TEN_TO_HUNDRED, ..AxisResponse::UNSET } };
        p.set_stick(Side::Left, &stick).unwrap();
        assert_eq!(p.as_bytes()[550..570], [1, 0, 1, 10, 100, 10, 100, 1, 0, 100, 0, 0, 43, 43, 113, 113, 184, 184, 255, 255]);
        assert!(p.as_bytes()[582..614].iter().all(|&b| b == 0), "the right stick is untouched");
        assert_eq!(p.stick(Side::Left), stick, "the points decode back to the same percentages");
        assert!(p.stick(Side::Right).raw, "a never-enabled stick reads as raw");
    }

    #[test]
    fn deadzones_keep_the_curve_inside() {
        let mut p = blank();
        let narrow = Deadzone { inner: 30, outer: 70, anti_inner: 0, anti_outer: 100 };
        let unfitted = StickConfig { raw: true, response: AxisResponse { deadzone: narrow, ..AxisResponse::UNSET } };
        assert!(p.set_stick(Side::Right, &unfitted).is_err(), "point 25 lies inside the 30% inner deadzone");

        let fitted = AxisResponse::UNSET.with_deadzone(narrow);
        assert_eq!(fitted.curve.points, [(31, 25), (50, 50), (69, 75)]);
        p.set_stick(Side::Right, &StickConfig { raw: true, response: fitted }).unwrap();
        assert_eq!(p.as_bytes()[582 + 9..582 + 20], [100, 0, 0, 6, 64, 128, 128, 249, 191, 255, 255]);
    }

    #[test]
    fn hair_trigger_modes() {
        let mut p = blank();
        let response = AxisResponse::UNSET;
        let rapid = TriggerConfig { response, hair: Some(HairTrigger { mode: HairMode::Rapid, start: 10, end: 90 }) };
        p.set_trigger(Side::Right, &rapid).unwrap();
        assert_eq!(p.as_bytes()[522 + 12..522 + 15], [0x81, 10, 90]);
        assert_eq!(p.trigger(Side::Right).hair, rapid.hair);

        let fixed = TriggerConfig { response, hair: Some(HairTrigger { mode: HairMode::Fixed, start: 0, end: 40 }) };
        p.set_trigger(Side::Left, &fixed).unwrap();
        assert_eq!(p.as_bytes()[494 + 12..494 + 15], [0x82, 0, 40]);
        assert_eq!(p.as_bytes()[494 + 2..494 + 7], [1, 5, 100, 0, 100]);

        p.set_trigger(Side::Left, &TriggerConfig { response, hair: None }).unwrap();
        assert_eq!(p.as_bytes()[494 + 12..494 + 15], [0, 0, 40]);
        assert_eq!(p.trigger(Side::Left).hair, None);

        let backwards = TriggerConfig { response, hair: Some(HairTrigger { mode: HairMode::Rapid, start: 50, end: 50 }) };
        assert!(p.set_trigger(Side::Left, &backwards).is_err());
    }

    #[test]
    fn button_entries() {
        let mut p = blank();
        p.set_button(Button::L4, &ButtonConfig { turbo_hz: 20, outputs: [0, 9, 0], toggle: false }).unwrap();
        assert_eq!(p.as_bytes()[176..183], [1, 20, 1, 9, 0, 0, 0], "outputs are packed to the front");
        p.set_button(Button::Share, &ButtonConfig { toggle: true, ..ButtonConfig::default() }).unwrap();
        assert_eq!(p.as_bytes()[169..176], [0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(p.button(Button::L4), ButtonConfig { turbo_hz: 20, outputs: [9, 0, 0], toggle: false });
        assert!(p.set_button(Button::A, &ButtonConfig { turbo_hz: 31, ..ButtonConfig::default() }).is_err());

        assert_eq!(Button::from_name("r4"), Some(Button::R4));
        assert_eq!(output_code("rs-left"), Some(27));
        assert_eq!(output_name(255), Some("disabled"));
    }

    #[test]
    fn light_settings() {
        let mut l = LightProfile::from_bytes(&[0; LIGHT_LEN]).unwrap();
        l.set_active_slot(2).unwrap();
        assert!(l.set_active_slot(4).is_err());
        l.set_brightness(2, 80).unwrap();
        l.set_speed(2, 15).unwrap();
        assert_eq!(l.as_bytes()[249..253], [0, 0, 5, 80], "slot 2 starts at 1 + 2 * 124; speed is stored as 20 - speed");
        assert_eq!((l.speed(2).unwrap(), l.brightness(2).unwrap()), (15, 80));

        l.set_color(1, [255, 0, 0]).unwrap();
        assert_eq!(l.as_bytes()[125], 1, "one key frame");
        assert_eq!(l.as_bytes()[129..144], [255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0]);
        assert!(l.as_bytes()[144..249].iter().all(|&b| b == 0));

        l.set_standby_minutes(5).unwrap();
        assert_eq!(l.as_bytes()[627], 5);
        assert!(l.set_standby_minutes(7).is_err());
    }
}
