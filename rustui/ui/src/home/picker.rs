//! "Who's watching?" (`docs/research/profiles-design.md`, screens): the profiles as tiles of 220 with the name under, the current one
//! ticked, over the blurred wallpaper; OK switches (`POST /v1/profiles/switch`), a profile with a PIN asks it in place: four dots, the
//! remote's digits fill them and the fourth submits, a 3 by 4 pad for the D-pad and the pointer, a wrong PIN shakes and clears, `429
//! pin_locked` shows the wait. Back keeps the current person. The switch veil: opaque 90 ms in, held while the new Home is applied, 130 ms
//! out (reduced motion: one 120 ms fade), so a late frame is never seen. The picker is resident: shown under 150 ms after wake when
//! `pickerOnWake` and Home is in front.

use crate::kit::focus::{Dir, Focus, Id, Moved, Region};
use serde_json::{Value, json};
use ui::geometry::Frame;

pub const TILE_ID: u32 = 1000;
pub const PAD_ID: u32 = 1100;
pub const TILES: Region = Region(12);
pub const PAD: Region = Region(13);
pub const TILE: f32 = 220.0;
pub const TILE_GAP: f32 = 48.0;
pub const PAD_KEY: f32 = 96.0;
pub const PAD_GAP: f32 = 16.0;
pub const VEIL_IN_MS: u64 = 90;
pub const VEIL_OUT_MS: u64 = 130;
pub const VEIL_REDUCED_MS: u64 = 120;
/// A switch is given this long to be applied before the veil lifts on its own.
pub const VEIL_HOLD_MAX_MS: u64 = 1500;

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub initial: String,
    pub color: String,
    pub avatar_file: Option<String>,
    pub active: bool,
    pub locked: bool,
    pub owner: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Veil {
    /// Fading in since `since`.
    In { since: u64, id: String },
    /// Opaque: the new Home is being applied.
    Held { since: u64, id: String },
    Out { since: u64 },
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Pin {
    pub id: String,
    pub digits: String,
    pub shake_until: u64,
    pub locked_until: u64,
    pub pad_index: usize,
}

pub enum PickerOut {
    Nothing,
    Close,
    Switch { id: String, pin: Option<String> },
}

#[derive(Default)]
pub struct Picker {
    pub profiles: Vec<Profile>,
    pub can_admin: bool,
    pub picker_on_wake: bool,
    pub index: usize,
    pub pin: Option<Pin>,
    pub veil: Option<Veil>,
    pub reduced: bool,
    opened_at: u64,
    pub up: bool,
}

/// The pad: 1 to 9 in three rows, then a blank, 0 and backspace.
pub const PAD_KEYS: [&str; 12] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "", "0", "\u{232b}"];

impl Picker {
    pub fn set_profiles(&mut self, topic: &Value) {
        self.can_admin = topic.get("canAdmin").and_then(Value::as_bool).unwrap_or(false);
        self.picker_on_wake = topic.get("pickerOnWake").and_then(Value::as_bool).unwrap_or(false);
        self.profiles = topic
            .get("profiles")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|p| Profile {
                        id: p.get("id").and_then(Value::as_str).unwrap_or("").to_owned(),
                        name: p.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
                        initial: p.get("initial").and_then(Value::as_str).unwrap_or("").to_owned(),
                        color: p.get("color").and_then(Value::as_str).unwrap_or("#7360E7").to_owned(),
                        avatar_file: p.get("avatarFile").and_then(Value::as_str).map(str::to_owned),
                        active: p.get("active").and_then(Value::as_bool).unwrap_or(false),
                        locked: p.get("locked").and_then(Value::as_bool).unwrap_or(false),
                        owner: p.get("owner").and_then(Value::as_bool).unwrap_or(false),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if self.index >= self.profiles.len() {
            self.index = 0;
        }
    }

    pub fn open(&mut self, now: u64) {
        self.up = true;
        self.opened_at = now;
        self.pin = None;
        self.index = self.profiles.iter().position(|p| p.active).unwrap_or(0);
    }

    pub fn close(&mut self) {
        self.up = false;
        self.pin = None;
    }

    /// The row of tiles, centred, with the names under.
    pub fn tile_frame(&self, i: usize) -> Frame {
        let n = self.profiles.len().max(1) as f32;
        let total = n * TILE + (n - 1.0) * TILE_GAP;
        let x0 = ((1920.0 - total) / 2.0).round();
        Frame::new(x0 + i as f32 * (TILE + TILE_GAP), 400.0, TILE, TILE)
    }

    /// The pad under the dots, centred under the asked tile's column of the screen.
    pub fn pad_frame(&self, i: usize) -> Frame {
        let (col, row) = (i % 3, i / 3);
        let w = 3.0 * PAD_KEY + 2.0 * PAD_GAP;
        let x0 = ((1920.0 - w) / 2.0).round();
        Frame::new(x0 + col as f32 * (PAD_KEY + PAD_GAP), 470.0 + row as f32 * (PAD_KEY + PAD_GAP), PAD_KEY, PAD_KEY)
    }

    pub fn register(&self, focus: &mut Focus) {
        let scope = super::model::SCOPE_PICKER;
        for i in 0..8u32 {
            let id = Id(TILE_ID + i);
            if (i as usize) < self.profiles.len() && self.pin.is_none() {
                focus.register(id, self.tile_frame(i as usize), TILES, scope);
            } else {
                focus.unregister(id);
            }
        }
        for i in 0..12u32 {
            let id = Id(PAD_ID + i);
            if self.pin.is_some() && !PAD_KEYS[i as usize].is_empty() {
                focus.register(id, self.pad_frame(i as usize), PAD, scope);
            } else {
                focus.unregister(id);
            }
        }
        match &self.pin {
            Some(p) => {
                focus.focus(Id(PAD_ID + p.pad_index as u32));
            }
            None => {
                focus.focus(Id(TILE_ID + self.index as u32));
            }
        }
    }

    pub fn unregister(&self, focus: &mut Focus) {
        for i in 0..8 {
            focus.unregister(Id(TILE_ID + i));
        }
        for i in 0..12 {
            focus.unregister(Id(PAD_ID + i));
        }
    }

    fn choose(&mut self, i: usize, now: u64) -> PickerOut {
        let Some(p) = self.profiles.get(i).cloned() else { return PickerOut::Nothing };
        if p.active {
            return PickerOut::Close;
        }
        if p.locked && !self.can_admin {
            self.pin = Some(Pin { id: p.id, pad_index: 0, ..Pin::default() });
            return PickerOut::Nothing;
        }
        self.start_switch(&p.id, None, now)
    }

    fn start_switch(&mut self, id: &str, pin: Option<String>, now: u64) -> PickerOut {
        self.veil = Some(Veil::In { since: now, id: id.to_owned() });
        PickerOut::Switch { id: id.to_owned(), pin }
    }

    /// A digit of the remote or the pad: the fourth submits.
    fn digit(&mut self, d: char, now: u64) -> PickerOut {
        let Some(p) = &mut self.pin else { return PickerOut::Nothing };
        if now < p.locked_until || p.digits.len() >= 4 {
            return PickerOut::Nothing;
        }
        p.digits.push(d);
        if p.digits.len() == 4 {
            let (id, pin) = (p.id.clone(), p.digits.clone());
            return self.start_switch(&id, Some(pin), now);
        }
        PickerOut::Nothing
    }

    pub fn key(&mut self, name: &str, dir: Option<Dir>, focus: &mut Focus, now: u64) -> PickerOut {
        if let Some(d) = name.chars().next().filter(|c| c.is_ascii_digit() && name.len() == 1) {
            if self.pin.is_some() {
                return self.digit(d, now);
            }
        }
        if name == "back" {
            if self.pin.is_some() {
                self.pin = None;
                self.register(focus);
                return PickerOut::Nothing;
            }
            return PickerOut::Close;
        }
        if name == "ok" {
            return self.activate(focus, now);
        }
        if let Some(d) = dir
            && let Moved::To(_, to) = focus.step(d)
        {
            self.take(to);
        }
        PickerOut::Nothing
    }

    fn take(&mut self, id: Id) {
        if let Some(i) = id.0.checked_sub(TILE_ID).filter(|i| *i < 8) {
            self.index = i as usize;
        } else if let Some(i) = id.0.checked_sub(PAD_ID).filter(|i| *i < 12)
            && let Some(p) = &mut self.pin
        {
            p.pad_index = i as usize;
        }
    }

    pub fn activate(&mut self, focus: &mut Focus, now: u64) -> PickerOut {
        let out = match &self.pin {
            Some(p) => match PAD_KEYS[p.pad_index] {
                "\u{232b}" => {
                    if let Some(p) = &mut self.pin {
                        p.digits.pop();
                    }
                    PickerOut::Nothing
                }
                k => match k.chars().next() {
                    Some(c) => self.digit(c, now),
                    None => PickerOut::Nothing,
                },
            },
            None => self.choose(self.index, now),
        };
        self.register(focus);
        out
    }

    pub fn clicked(&mut self, id: Id, focus: &mut Focus, now: u64) -> PickerOut {
        self.take(id);
        self.activate(focus, now)
    }

    /// The switch's answer: `200` keeps the veil until the new Home is in; a wrong PIN shakes and clears; `429` locks for `retryAfterS`.
    pub fn answered(&mut self, status: u16, data: &Value, now: u64) -> bool {
        match status {
            200 => {
                if let Some(Veil::In { id, .. }) = &self.veil {
                    let id = id.clone();
                    self.veil = Some(Veil::Held { since: now, id });
                }
                self.pin = None;
                true
            }
            _ => {
                self.veil = None;
                if let Some(p) = &mut self.pin {
                    p.digits.clear();
                    p.shake_until = now + 400;
                    if status == 429 {
                        let s = data.pointer("/error/retryAfterS").and_then(Value::as_u64).unwrap_or(30);
                        p.locked_until = now + s * 1000;
                    }
                }
                true
            }
        }
    }

    /// The new person's Home is applied (the `profiles` topic changed): the veil lifts and the picker goes.
    pub fn switched(&mut self, now: u64) {
        self.veil = Some(Veil::Out { since: now });
        self.up = false;
        self.pin = None;
    }

    /// The veil's opacity now (0 is none), and whether it is over everything.
    pub fn veil_alpha(&self, now: u64) -> f32 {
        let (in_ms, out_ms) = if self.reduced { (VEIL_REDUCED_MS, VEIL_REDUCED_MS) } else { (VEIL_IN_MS, VEIL_OUT_MS) };
        match &self.veil {
            None => 0.0,
            Some(Veil::In { since, .. }) => ((now.saturating_sub(*since)) as f32 / in_ms as f32).min(1.0),
            Some(Veil::Held { .. }) => 1.0,
            Some(Veil::Out { since }) => 1.0 - ((now.saturating_sub(*since)) as f32 / out_ms as f32).min(1.0),
        }
    }

    pub fn tick(&mut self, now: u64) -> bool {
        let mut changed = false;
        match &self.veil {
            // a switch the helper never answered, or whose Home never came: the veil lifts rather than hiding the TV
            Some(Veil::Held { since, .. }) | Some(Veil::In { since, .. }) if now >= since + VEIL_HOLD_MAX_MS => {
                self.switched(now);
                changed = true;
            }
            Some(Veil::Out { since }) if now >= since + VEIL_OUT_MS.max(VEIL_REDUCED_MS) => {
                self.veil = None;
                changed = true;
            }
            Some(Veil::In { .. }) | Some(Veil::Out { .. }) => changed = true,
            _ => {}
        }
        if let Some(p) = &mut self.pin {
            if p.shake_until > 0 && now >= p.shake_until {
                p.shake_until = 0;
                changed = true;
            }
            if p.locked_until > 0 && now >= p.locked_until {
                p.locked_until = 0;
                changed = true;
            }
        }
        changed
    }

    pub fn due_at(&self) -> Option<u64> {
        let mut due: Option<u64> = None;
        let mut take = |t: u64| due = Some(due.map_or(t, |d| d.min(t)));
        match &self.veil {
            Some(Veil::In { since, .. }) => take((since + 16).min(since + VEIL_HOLD_MAX_MS)),
            Some(Veil::Held { since, .. }) => take(since + VEIL_HOLD_MAX_MS),
            Some(Veil::Out { since }) => take(since + 16),
            None => {}
        }
        if let Some(p) = &self.pin {
            if p.shake_until > 0 {
                take(p.shake_until);
            }
            if p.locked_until > 0 {
                take(p.locked_until.min(((self.opened_at / 1000) + 1) * 1000));
            }
        }
        due
    }

    /// The words under the dots: the wait while locked, nothing otherwise.
    pub fn pin_words(&self, now: u64) -> String {
        match &self.pin {
            Some(p) if now < p.locked_until => format!("Wait {} s", (p.locked_until - now).div_ceil(1000)),
            _ => String::new(),
        }
    }

    pub fn state(&self) -> Value {
        json!({"up": self.up, "index": self.index, "pin": self.pin.as_ref().map(|p| json!({"id": p.id, "digits": p.digits.len(), "locked": p.locked_until > 0, "pad": p.pad_index})),
               "veil": self.veil.as_ref().map(|v| match v { Veil::In { .. } => "in", Veil::Held { .. } => "held", Veil::Out { .. } => "out" }), "profiles": self.profiles.len()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(can_admin: bool) -> Value {
        json!({"ready": true, "active": "nitin", "owner": "nitin", "canAdmin": can_admin, "pickerOnWake": false, "profiles": [
            {"id": "nitin", "name": "Nitin", "initial": "N", "color": "#A79BF5", "active": true, "locked": false, "owner": true},
            {"id": "isabel", "name": "Isabel", "initial": "I", "color": "#EB6F92", "active": false, "locked": true, "owner": false},
            {"id": "mia", "name": "Mia", "initial": "M", "color": "#F6C177", "active": false, "locked": false, "owner": false}]})
    }

    #[test]
    fn ok_on_a_profile_switches_and_the_veil_holds_until_the_new_home_is_in() {
        let mut p = Picker::default();
        p.set_profiles(&topic(true));
        p.open(1000);
        let mut f = Focus::new();
        p.register(&mut f);
        assert_eq!(p.index, 0);
        p.key("right", Some(Dir::Right), &mut f, 1100);
        p.key("right", Some(Dir::Right), &mut f, 1200);
        assert!(matches!(p.key("ok", None, &mut f, 1300), PickerOut::Switch { id, pin: None } if id == "mia"));
        assert!(p.veil_alpha(1345) > 0.4 && p.veil_alpha(1345) < 0.6);
        assert!(p.answered(200, &json!({}), 1400));
        assert_eq!(p.veil_alpha(1500), 1.0);
        p.switched(1600);
        assert!(!p.up);
        assert!(p.veil_alpha(1665) < 0.6);
        assert!(p.tick(1800));
        assert_eq!(p.veil, None);
    }

    #[test]
    fn a_locked_profile_asks_a_pin_the_fourth_digit_submits_and_a_wrong_one_shakes() {
        let mut p = Picker::default();
        p.set_profiles(&topic(false));
        p.open(0);
        let mut f = Focus::new();
        p.register(&mut f);
        p.key("right", Some(Dir::Right), &mut f, 10);
        assert!(matches!(p.key("ok", None, &mut f, 20), PickerOut::Nothing));
        assert!(p.pin.is_some());
        for d in ["1", "2", "3"] {
            assert!(matches!(p.key(d, None, &mut f, 30), PickerOut::Nothing));
        }
        assert!(matches!(p.key("4", None, &mut f, 40), PickerOut::Switch { id, pin: Some(pin) } if id == "isabel" && pin == "1234"));
        assert!(p.answered(403, &json!({"error": {"code": "wrong_pin"}}), 50));
        let pin = p.pin.as_ref().unwrap();
        assert_eq!((pin.digits.as_str(), pin.shake_until), ("", 450));
        assert!(p.answered(429, &json!({"error": {"code": "pin_locked", "retryAfterS": 30}}), 60));
        assert_eq!(p.pin_words(1060), "Wait 29 s");
        assert!(matches!(p.key("5", None, &mut f, 70), PickerOut::Nothing));
        assert_eq!(p.pin.as_ref().unwrap().digits, "");
        p.key("back", None, &mut f, 80);
        assert!(p.pin.is_none());
        assert!(matches!(p.key("back", None, &mut f, 90), PickerOut::Close));
    }

    #[test]
    fn the_owner_opens_a_locked_profile_without_a_pin() {
        let mut p = Picker::default();
        p.set_profiles(&topic(true));
        p.open(0);
        let mut f = Focus::new();
        p.register(&mut f);
        p.key("right", Some(Dir::Right), &mut f, 10);
        assert!(matches!(p.key("ok", None, &mut f, 20), PickerOut::Switch { id, pin: None } if id == "isabel"));
    }
}
