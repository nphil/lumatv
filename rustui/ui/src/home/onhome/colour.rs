//! The colour sheet of a lamp, opened by a held OK on it: a ring of hues, how much of the colour there is, how bright the lamp is, how warm a
//! white lamp's white is, a row of colours worth having and the effects the lamp has. Left and Right change the row in front, Up and Down go
//! to another; what is changed is shown at once and sent as the lamp is changed, at most four times a second (a held key is one call a
//! quarter of a second, not forty).
//!
//! Pure: the sheet knows hues and levels, not pixels; what it asks of Home Assistant are the helper's own verbs (`colour`, `brightness`,
//! `kelvin`, `effect`), the ones its planner takes.

use super::Act;
use super::paint;
use serde_json::{Value, json};

/// The least time between two calls to one lamp while its sheet is being turned.
pub const SEND_EVERY_MS: u64 = 250;
/// The integration holds a Show to 10 calls in 10 seconds, whatever they are (`lumashow-ha` protocol §11). The sheet's calls are paced as a bucket of
/// three that fills one in [`CALL_EVERY_MS`]: at most nine in any ten seconds, so that a key held down on a row, or the pointer dragged round the ring,
/// is never answered with "too many actions" and leaves room for a switch pressed meanwhile; the lamp follows at a call every 1.5 s, and the latest value
/// is the one sent.
pub const CALL_EVERY_MS: u64 = 1500;
/// How far ahead of that pace the sheet may run: the first three calls of a turn go a quarter of a second apart.
pub const BURST_MS: u64 = 2 * CALL_EVERY_MS;
/// How long what the person set is held against what the topic says (the topic answers a moment after the lamp does).
pub const HOLD_MS: u64 = 2500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Hue,
    Saturation,
    Brightness,
    Temperature,
    Preset,
    Effect,
}

/// A colour worth having: a colour of the lamp's, or a white at a temperature.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Preset {
    Colour([u8; 3]),
    White(u32),
}

impl Preset {
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Preset::Colour(c) => c,
            Preset::White(k) => paint::kelvin_rgb(k as f32),
        }
    }
}

/// What the lamp is showing: a colour of its own, or a white.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Colour,
    White,
}

/// What a key on the sheet did.
#[derive(Clone, Debug, PartialEq)]
pub enum Done {
    Nothing,
    Close,
}

const WHITES: [u32; 3] = [2700, 4000, 6500];
const COLOURS: [[u8; 3]; 5] = [[255, 60, 48], [255, 159, 10], [48, 209, 88], [10, 132, 255], [191, 90, 242]];

#[derive(Clone, Debug)]
pub struct ColourSheet {
    pub id: String,
    pub rows: Vec<Row>,
    /// The row in front: an index into `rows`.
    pub at: usize,
    /// The hue in degrees and how much colour (0 white, 1 all), in the model colour screens use: an exact round trip with the lamp's red, green and blue.
    pub hue: f32,
    pub sat: f32,
    /// How bright, 0.01 to 1.
    pub level: f32,
    /// The warmth of a white lamp's white: now, and the ends of its slider.
    pub kelvin: f32,
    pub kelvin_range: (f32, f32),
    pub mode: Mode,
    pub presets: Vec<Preset>,
    pub preset: usize,
    pub effects: Vec<String>,
    /// The effect that runs, an index into `effects`.
    pub effect: Option<usize>,
    pending: Pending,
    last_sent: Option<u64>,
    /// When the calls made so far are paid for: the pace's own clock, which a call moves on by [`CALL_EVERY_MS`].
    paid_until: u64,
    changed_at: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Pending {
    colour: Option<[u8; 3]>,
    level: Option<u8>,
    kelvin: Option<u32>,
    effect: Option<String>,
}

impl Pending {
    fn is_empty(&self) -> bool {
        *self == Pending::default()
    }
}

pub fn rgb_to_hsv(rgb: [u8; 3]) -> (f32, f32, f32) {
    let [r, g, b] = rgb.map(|c| f32::from(c) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let h = if d <= 1e-6 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if max <= 1e-6 { 0.0 } else { d / max }, max)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let (h, s, v) = (h.rem_euclid(360.0), s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + v - c, g + v - c, b + v - c].map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8)
}

impl ColourSheet {
    /// The sheet of the lamp `tile` (the helper's tile: its `ctl` says what the lamp can do and what it is doing).
    pub fn new(id: &str, tile: &Value) -> Self {
        let ctl = tile.get("ctl").cloned().unwrap_or(Value::Null);
        let colour = ctl.get("colour").and_then(Value::as_bool) == Some(true);
        let dimmable = ctl.get("dimmable").and_then(Value::as_bool) == Some(true);
        let kelvin = ctl.get("kelvin").filter(|k| !k.is_null());
        let effects: Vec<String> = ctl.get("effects").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
        let mut rows = Vec::new();
        if colour {
            rows.extend([Row::Hue, Row::Saturation]);
        }
        if dimmable || colour || kelvin.is_some() {
            rows.push(Row::Brightness);
        }
        if kelvin.is_some() {
            rows.push(Row::Temperature);
        }
        if colour || kelvin.is_some() {
            rows.push(Row::Preset);
        }
        if !effects.is_empty() {
            rows.push(Row::Effect);
        }
        let range = kelvin.map_or((2000.0, 6500.0), |k| (k["min"].as_f64().unwrap_or(2000.0) as f32, k["max"].as_f64().unwrap_or(6500.0) as f32));
        let mut presets: Vec<Preset> = WHITES.iter().map(|k| Preset::White(*k)).collect();
        if colour {
            presets.extend(COLOURS.iter().map(|c| Preset::Colour(*c)));
        }
        let mut s = Self {
            id: id.to_owned(),
            rows,
            at: 0,
            hue: 30.0,
            sat: 0.0,
            level: 1.0,
            kelvin: 3000.0,
            kelvin_range: range,
            mode: if colour { Mode::Colour } else { Mode::White },
            presets,
            preset: 0,
            effects,
            effect: None,
            pending: Pending::default(),
            last_sent: None,
            paid_until: 0,
            changed_at: 0,
        };
        s.read(tile);
        s
    }

    /// What the topic says of the lamp now, taken as the sheet's own.
    fn read(&mut self, tile: &Value) {
        let ctl = tile.get("ctl").cloned().unwrap_or(Value::Null);
        if let Some(b) = ctl.get("brightness").and_then(Value::as_f64) {
            self.level = (b as f32 / 100.0).clamp(0.01, 1.0);
        }
        if let Some(k) = ctl.pointer("/kelvin/now").and_then(Value::as_f64) {
            self.kelvin = (k as f32).clamp(self.kelvin_range.0, self.kelvin_range.1);
        }
        // where the ring's marker stands: the hue and the saturation (in percent) the lamp says, else those of the colour it shows
        let said = ctl.get("hs").and_then(Value::as_array).and_then(|a| Some((a.first()?.as_f64()? as f32, (a.get(1)?.as_f64()? as f32 / 100.0).clamp(0.0, 1.0))));
        let shown = ctl.get("rgb").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).map(|v| v.clamp(0.0, 255.0).round() as u8).collect::<Vec<u8>>()).filter(|c| c.len() == 3).map(|c| {
            let (h, s, _) = rgb_to_hsv([c[0], c[1], c[2]]);
            (h, s)
        });
        if self.rows.contains(&Row::Hue)
            && let Some((h, s)) = said.or(shown)
        {
            // the ring keeps its hue for a white (no colour has no hue)
            if s > 0.02 {
                self.hue = h.rem_euclid(360.0);
            }
            self.sat = s;
            self.mode = Mode::Colour;
        }
        // a lamp that shows a white says so: the ring has no colour to show then, however warm the white is
        if self.rows.contains(&Row::Hue) {
            match ctl.get("mode").and_then(Value::as_str) {
                Some("white") => (self.mode, self.sat) = (Mode::White, 0.0),
                Some("colour") => self.mode = Mode::Colour,
                _ => {}
            }
        }
        self.effect = ctl.get("effect").and_then(Value::as_str).and_then(|e| self.effects.iter().position(|x| x == e));
    }

    /// The topic changed: what it says becomes the sheet's own unless the person is in the middle of setting something.
    pub fn sync(&mut self, tile: &Value, now: u64) {
        if self.pending.is_empty() && now >= self.changed_at + HOLD_MS {
            self.read(tile);
        }
    }

    pub fn row(&self) -> Option<Row> {
        self.rows.get(self.at).copied()
    }

    /// The colour the lamp is shown with now, at full brightness: its hue and saturation, or the white it is set to.
    pub fn rgb(&self) -> [u8; 3] {
        match self.mode {
            Mode::Colour => hsv_to_rgb(self.hue, self.sat, 1.0),
            Mode::White => paint::kelvin_rgb(self.kelvin),
        }
    }

    fn colour_changed(&mut self, now: u64) {
        self.mode = Mode::Colour;
        self.pending.colour = Some(self.rgb());
        self.changed_at = now;
    }

    fn step_row(&mut self, dir: i32, now: u64) {
        let d = dir as f32;
        match self.row() {
            Some(Row::Hue) => {
                self.hue = (self.hue + 10.0 * d).rem_euclid(360.0);
                // a hue of a white is a colour: the ring gives it some
                if self.sat < 0.15 {
                    self.sat = 0.6;
                }
                self.colour_changed(now);
            }
            Some(Row::Saturation) => {
                self.sat = (self.sat + 0.08 * d).clamp(0.0, 1.0);
                self.colour_changed(now);
            }
            Some(Row::Brightness) => {
                self.level = (self.level + 0.05 * d).clamp(0.01, 1.0);
                self.pending.level = Some((self.level * 100.0).round().clamp(1.0, 100.0) as u8);
                self.changed_at = now;
            }
            Some(Row::Temperature) => {
                let (lo, hi) = self.kelvin_range;
                self.kelvin = (self.kelvin + 250.0 * d).clamp(lo, hi);
                self.mode = Mode::White;
                self.pending.kelvin = Some(self.kelvin.round() as u32);
                self.changed_at = now;
            }
            Some(Row::Preset) => {
                let n = self.presets.len() as i32;
                self.preset = (self.preset as i32 + dir).clamp(0, n - 1) as usize;
                match self.presets[self.preset] {
                    Preset::Colour(c) => {
                        let (h, s, _) = rgb_to_hsv(c);
                        (self.hue, self.sat) = (h, s);
                        self.colour_changed(now);
                    }
                    Preset::White(k) => {
                        self.kelvin = (k as f32).clamp(self.kelvin_range.0, self.kelvin_range.1);
                        self.mode = Mode::White;
                        self.pending.kelvin = Some(self.kelvin.round() as u32);
                        self.changed_at = now;
                    }
                }
            }
            Some(Row::Effect) => {
                let n = self.effects.len() as i32;
                let now_at = self.effect.map_or(-1, |e| e as i32);
                let to = (now_at + dir).clamp(0, n - 1);
                self.effect = Some(to as usize);
                self.pending.effect = self.effects.get(to as usize).cloned();
                self.changed_at = now;
            }
            None => {}
        }
    }

    /// A key: Up and Down go to another row, Left and Right change the row in front, Back closes.
    pub fn key(&mut self, name: &str, now: u64) -> Done {
        match name {
            "back" => return Done::Close,
            "up" => self.at = self.at.saturating_sub(1),
            "down" => self.at = (self.at + 1).min(self.rows.len().saturating_sub(1)),
            "left" => self.step_row(-1, now),
            "right" => self.step_row(1, now),
            _ => {}
        }
        Done::Nothing
    }

    /// The pointer set the row `at` to the share `t` (0 to 1) of its track or ring: a slider, the ring's angle, a swatch or a chip by its place.
    pub fn set_row(&mut self, row: Row, t: f32, now: u64) {
        let t = t.clamp(0.0, 1.0);
        if let Some(i) = self.rows.iter().position(|r| *r == row) {
            self.at = i;
        }
        match row {
            Row::Hue => {
                self.hue = (t * 360.0).rem_euclid(360.0);
                if self.sat < 0.15 {
                    self.sat = 0.6;
                }
                self.colour_changed(now);
            }
            Row::Saturation => {
                self.sat = t;
                self.colour_changed(now);
            }
            Row::Brightness => {
                self.level = t.max(0.01);
                self.pending.level = Some((self.level * 100.0).round().clamp(1.0, 100.0) as u8);
                self.changed_at = now;
            }
            Row::Temperature => {
                let (lo, hi) = self.kelvin_range;
                self.kelvin = lo + t * (hi - lo);
                self.mode = Mode::White;
                self.pending.kelvin = Some(self.kelvin.round() as u32);
                self.changed_at = now;
            }
            Row::Preset | Row::Effect => {}
        }
    }

    /// A swatch or a chip chosen by the pointer.
    pub fn pick(&mut self, row: Row, index: usize, now: u64) {
        if let Some(i) = self.rows.iter().position(|r| *r == row) {
            self.at = i;
        }
        let from = match row {
            Row::Preset => self.preset as i32,
            Row::Effect => self.effect.map_or(-1, |e| e as i32),
            _ => return,
        };
        self.step_row(index as i32 - from, now);
    }

    /// When the next call may be made: a quarter of a second after the last, and no sooner than the pace allows.
    fn ready_at(&self) -> u64 {
        let spaced = self.last_sent.map_or(0, |at| at + SEND_EVERY_MS);
        spaced.max(self.paid_until.saturating_sub(BURST_MS))
    }

    /// The calls that are due: what was changed since the last, one for each thing that changed, once a quarter of a second at most and at the pace of
    /// [`CALL_EVERY_MS`] beyond the first three (what waits is sent when the pace allows, and is the latest value then).
    pub fn tick(&mut self, now: u64) -> Vec<Act> {
        if self.pending.is_empty() || now < self.ready_at() {
            return Vec::new();
        }
        self.last_sent = Some(now);
        let id = self.id.clone();
        let act = |verb: &'static str, value: Value| Act { id: id.clone(), verb, value };
        let p = std::mem::take(&mut self.pending);
        let mut out = Vec::new();
        if let Some(c) = p.colour {
            out.push(act("colour", json!({"rgb": c})));
        }
        if let Some(k) = p.kelvin {
            out.push(act("kelvin", json!(k)));
        }
        if let Some(l) = p.level {
            out.push(act("brightness", json!(l)));
        }
        if let Some(e) = p.effect {
            out.push(act("effect", json!(e)));
        }
        self.paid_until = self.paid_until.max(now) + CALL_EVERY_MS * out.len() as u64;
        out
    }

    /// When the next call is due, if one waits.
    pub fn due_at(&self) -> Option<u64> {
        (!self.pending.is_empty()).then(|| self.ready_at())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lamp() -> Value {
        json!({"id": "light.strip", "name": "Strip", "group": "lights", "words": "On \u{b7} 60%", "available": true, "can": ["toggle"], "verbs": ["turn_on", "turn_off", "toggle"],
               "ctl": {"on": true, "dimmable": true, "brightness": 60, "colour": true, "rgb": [255, 0, 128], "kelvin": null, "effect": "Rainbow", "effects": ["None", "Rainbow", "Pulse"]}})
    }

    fn white() -> Value {
        json!({"id": "light.desk", "name": "Desk", "group": "lights", "available": true, "can": ["toggle"], "verbs": ["turn_on"],
               "ctl": {"on": true, "dimmable": true, "brightness": 40, "colour": false, "rgb": [255, 177, 110], "kelvin": {"now": 3000, "min": 2000, "max": 6500}, "effects": []}})
    }

    #[test]
    fn the_rows_are_the_ones_the_lamp_has_and_start_from_what_it_is_doing() {
        let s = ColourSheet::new("light.strip", &lamp());
        assert_eq!(s.rows, [Row::Hue, Row::Saturation, Row::Brightness, Row::Preset, Row::Effect]);
        assert_eq!((s.level, s.effect, s.mode), (0.6, Some(1), Mode::Colour));
        let (h, sat, _) = rgb_to_hsv([255, 0, 128]);
        assert!((s.hue - h).abs() < 0.5 && (s.sat - sat).abs() < 0.01);
        let w = ColourSheet::new("light.desk", &white());
        assert_eq!(w.rows, [Row::Brightness, Row::Temperature, Row::Preset]);
        assert_eq!((w.mode, w.kelvin, w.kelvin_range), (Mode::White, 3000.0, (2000.0, 6500.0)));
        assert!(w.presets.iter().all(|p| matches!(p, Preset::White(_))), "a white lamp has whites, not colours");
        // a lamp that only dims has one row
        let mut plain = white();
        plain["ctl"] = json!({"on": true, "dimmable": true, "brightness": 50, "colour": false, "kelvin": null, "effects": []});
        assert_eq!(ColourSheet::new("light.x", &plain).rows, [Row::Brightness]);
    }

    #[test]
    fn hues_and_levels_round_trip_with_the_lamps_red_green_and_blue() {
        for c in [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255], [10, 200, 140], [255, 177, 110], [128, 128, 128]] {
            let (h, s, v) = rgb_to_hsv(c);
            let back = hsv_to_rgb(h, s, v);
            assert!(c.iter().zip(back).all(|(a, b)| (i32::from(*a) - i32::from(b)).abs() <= 1), "{c:?} {back:?}");
        }
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), [255, 0, 0]);
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), [0, 255, 0]);
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), [0, 0, 255]);
        assert_eq!(hsv_to_rgb(77.0, 0.0, 1.0), [255, 255, 255]);
    }

    #[test]
    fn left_and_right_change_the_row_in_front_at_once_and_up_and_down_go_to_another() {
        let mut s = ColourSheet::new("light.strip", &lamp());
        let hue = s.hue;
        s.key("right", 1000);
        assert!((s.hue - (hue + 10.0).rem_euclid(360.0)).abs() < 0.01);
        s.key("down", 1001);
        assert_eq!(s.row(), Some(Row::Saturation));
        let sat = s.sat;
        s.key("left", 1002);
        assert!((s.sat - (sat - 0.08)).abs() < 1e-5);
        s.key("down", 1003);
        assert_eq!(s.row(), Some(Row::Brightness));
        s.key("right", 1004);
        assert!((s.level - 0.65).abs() < 1e-5);
        // never beyond its ends, never wrapping from one row to another
        for _ in 0..30 {
            s.key("right", 1005);
        }
        assert_eq!(s.level, 1.0);
        for _ in 0..10 {
            s.key("up", 1006);
        }
        assert_eq!(s.row(), Some(Row::Hue));
        assert_eq!(s.key("back", 1007), Done::Close);
    }

    #[test]
    fn what_is_changed_is_sent_as_the_lamp_is_changed_a_quarter_of_a_second_apart_and_the_last_value_is_not_lost() {
        let mut s = ColourSheet::new("light.strip", &lamp());
        s.key("right", 1000);
        let first = s.tick(1000);
        assert_eq!(first.len(), 1);
        assert_eq!((first[0].verb, first[0].id.as_str()), ("colour", "light.strip"));
        assert_eq!(first[0].value["rgb"].as_array().map(Vec::len), Some(3));
        // a held key: a change every 40 ms, a call every quarter of a second, and the one that rests is sent
        for t in 1040..1240 {
            if t % 40 == 0 {
                s.key("right", t);
            }
            assert!(s.tick(t).is_empty(), "{t}");
        }
        assert_eq!(s.due_at(), Some(1250));
        assert!(s.tick(1249).is_empty());
        let next = s.tick(1250);
        assert_eq!(next.len(), 1);
        let last = hsv_to_rgb(s.hue, s.sat, 1.0);
        assert_eq!(next[0].value["rgb"], json!(last), "the latest hue, not the one the last call had");
        assert!(s.due_at().is_none() && s.tick(2000).is_empty(), "nothing is sent that did not change");
    }

    #[test]
    fn brightness_temperature_presets_and_effects_each_have_their_verb() {
        let mut s = ColourSheet::new("light.strip", &lamp());
        s.key("down", 0);
        s.key("down", 0);
        s.key("left", 10);
        let a = s.tick(10);
        assert_eq!((a[0].verb, a[0].value.clone()), ("brightness", json!(55)));
        s.key("down", 20);
        assert_eq!(s.row(), Some(Row::Preset));
        s.key("right", 400);
        assert_eq!(s.preset, 1);
        let a = s.tick(400);
        assert_eq!((a[0].verb, a[0].value.clone()), ("kelvin", json!(4000)), "a white preset sets the white");
        s.key("down", 410);
        s.key("right", 700);
        assert_eq!(s.effect, Some(2));
        let a = s.tick(700);
        assert_eq!((a[0].verb, a[0].value.clone()), ("effect", json!("Pulse")));
        let mut w = ColourSheet::new("light.desk", &white());
        w.key("down", 0);
        w.key("right", 1000);
        assert_eq!(w.kelvin, 3250.0);
        assert_eq!(w.tick(1000)[0].value, json!(3250));
        for _ in 0..40 {
            w.key("right", 1001);
        }
        assert_eq!(w.kelvin, 6500.0);
    }

    #[test]
    fn the_topic_is_taken_when_the_person_has_let_go_and_not_before() {
        let mut s = ColourSheet::new("light.strip", &lamp());
        s.key("down", 0);
        s.key("down", 0);
        s.key("right", 1000);
        let mut other = lamp();
        other["ctl"]["brightness"] = json!(20);
        s.sync(&other, 1100);
        assert!((s.level - 0.65).abs() < 1e-5, "mid-change: what was set stays");
        s.tick(1300);
        s.sync(&other, 1500);
        assert!((s.level - 0.65).abs() < 1e-5, "sent, but the topic has not caught up: held a little longer");
        s.sync(&other, 1000 + HOLD_MS);
        assert!((s.level - 0.2).abs() < 1e-5, "a change made elsewhere is the sheet's again");
    }

    #[test]
    fn the_pointer_sets_a_row_by_where_it_is_and_picks_a_swatch_or_a_chip() {
        let mut s = ColourSheet::new("light.strip", &lamp());
        s.set_row(Row::Hue, 0.5, 100);
        assert!((s.hue - 180.0).abs() < 0.01 && s.row() == Some(Row::Hue));
        s.set_row(Row::Brightness, 0.0, 100);
        assert_eq!(s.level, 0.01, "off is the switch's, the least is 1 %");
        s.pick(Row::Preset, 6, 200);
        assert_eq!(s.preset, 6);
        s.pick(Row::Effect, 0, 200);
        assert_eq!(s.effect, Some(0));
    }

    #[test]
    fn the_sheet_of_the_owners_own_bulbs_shows_the_wheel_the_white_range_and_the_effects_they_say() {
        use super::super::owner_tile;
        // a Hue bulb showing a white of 2000 K: the ring has no colour, the warmth track runs over the bulb's own 2000 to 6535 K
        let tile = owner_tile("light.master_bedroom_corner_light");
        let hue = ColourSheet::new("light.master_bedroom_corner_light", &tile);
        assert_eq!(hue.rows, [Row::Hue, Row::Saturation, Row::Brightness, Row::Temperature, Row::Preset, Row::Effect]);
        assert_eq!((hue.mode, hue.kelvin, hue.kelvin_range, hue.sat), (Mode::White, 2000.0, (2000.0, 6535.0), 0.0));
        assert!((hue.hue - 30.601).abs() < 0.01, "the ring keeps the hue the bulb says");
        assert_eq!((hue.effects.len(), hue.effect.map(|e| hue.effects[e].as_str()), hue.level), (20, Some("opal"), 1.0));
        // a Govee floor lamp that is off: its own white range, all its hundred and eleven effects, the ring and nothing running
        let floor = ColourSheet::new("light.living_room_floor_lamp", &owner_tile("light.living_room_floor_lamp"));
        assert_eq!((floor.rows.clone(), floor.kelvin_range, floor.effects.len(), floor.effect), (vec![Row::Hue, Row::Saturation, Row::Brightness, Row::Temperature, Row::Preset, Row::Effect], (2700.0, 6500.0), 111, None));
        // the aquarium light in blue: its marker is at the hue it says, and the effect "off" is no effect
        let aquarium = ColourSheet::new("light.plant_room_aquarium_light", &owner_tile("light.plant_room_aquarium_light"));
        assert_eq!((aquarium.mode, aquarium.effect, aquarium.rows.contains(&Row::Temperature)), (Mode::Colour, None, false));
        assert!((aquarium.hue - 236.941).abs() < 0.01 && aquarium.sat == 1.0 && (aquarium.level - 0.05).abs() < 1e-5, "{} {} {}", aquarium.hue, aquarium.sat, aquarium.level);
        // a lamp that only dims has the one row
        assert_eq!(ColourSheet::new("light.kitchen_lights", &owner_tile("light.kitchen_lights")).rows, [Row::Brightness]);
        // a bulb that goes from a white to a colour on the topic takes the colour the next time the person lets go
        let mut s = ColourSheet::new("light.master_bedroom_corner_light", &tile);
        let mut now = tile.clone();
        now["ctl"]["mode"] = json!("colour");
        now["ctl"]["hs"] = json!([120.0, 100.0]);
        s.sync(&now, HOLD_MS);
        assert_eq!((s.mode, s.hue, s.sat), (Mode::Colour, 120.0, 1.0));
    }

    #[test]
    fn a_key_held_for_half_a_minute_stays_inside_the_integrations_pace_and_the_latest_value_is_sent_as_it_allows() {
        let mut s = ColourSheet::new("light.strip", &lamp());
        s.key("down", 0);
        s.key("down", 0);
        assert_eq!(s.row(), Some(Row::Brightness));
        let mut calls: Vec<u64> = Vec::new();
        for t in (1000..31_000u64).step_by(10) {
            if t % 40 == 0 {
                s.key(if (t / 40) % 2 == 0 { "left" } else { "right" }, t);
            }
            if !s.tick(t).is_empty() {
                calls.push(t);
            }
        }
        // the integration refuses an eleventh call in ten seconds: the sheet never makes a tenth, from any call on
        for t in &calls {
            let n = calls.iter().filter(|u| **u >= *t && **u < *t + 10_000).count();
            assert!(n <= 9, "{n} calls in the 10 s from {t}");
        }
        assert_eq!(calls[..5], [1000, 1250, 1500, 2500, 4000], "three at once, a quarter of a second apart, then one in 1.5 s");
        assert!(calls.len() >= 19, "it did not stop sending: {}", calls.len());
        // what the last key set is on its way, and goes out when the pace allows
        s.key("right", 31_000);
        let waits = s.due_at().expect("a value waits");
        assert_eq!(waits, 31_000, "a second and a half after the call before it");
        assert!(s.tick(waits - 1).is_empty(), "not before the pace allows");
        assert_eq!(s.tick(waits).len(), 1);
    }
}
