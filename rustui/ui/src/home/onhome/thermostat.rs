//! The thermostat sheet, opened by OK on a thermostat: the room now (with what the thermostat is doing and the humidity, when it reads one), the set
//! point with a step down and a step up (or the two of a range, heat to and cool to), and the modes as chips. Up and Down go to another row, Left and
//! Right step the set point in front (or walk the chips, which OK chooses: a mode is not switched in passing); a set point rests for a moment and
//! is sent once, so that a held key is one call, and what the person set is shown until Home Assistant's own answer is.
//!
//! Pure: the sheet knows degrees and modes, not pixels. What it asks of Home Assistant are the helper's own verbs (`temperature`, `range`, `hvac_mode`),
//! the ones its planner takes (`helper/app/src/onhome/tile.rs`).

use super::{Act, mode_word, num_text};
use serde_json::{Value, json};

/// How long a changed set point rests before it is sent.
pub const REST_MS: u64 = 300;
/// How long what the person set is held against what the topic says (the topic answers a moment after the thermostat does).
pub const HOLD_MS: u64 = 2500;

/// What a row of the sheet changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// The one set point.
    Set,
    /// A range's low end (what it heats to) and high end (what it cools to).
    Low,
    High,
    /// The modes.
    Mode,
}

/// What a key on the sheet did.
#[derive(Clone, Debug, PartialEq)]
pub enum Done {
    Nothing,
    Close,
    /// Send this now.
    Send(Act),
}

#[derive(Clone, Debug)]
pub struct ThermostatSheet {
    pub id: String,
    pub rows: Vec<Row>,
    /// The row in front: an index into `rows`.
    pub at: usize,
    /// The room now, and what the thermostat is doing (`heating`, `cooling`, `idle`...).
    pub current: Option<f64>,
    pub action: String,
    /// The humidity it reads, and the humidity it is set to.
    pub humidity: Option<f64>,
    pub humidity_target: Option<f64>,
    /// The set point, or the two of a range.
    pub target: Option<f64>,
    pub low: Option<f64>,
    pub high: Option<f64>,
    /// The mode now, the modes there are, and the chip the cursor is on (an index into `modes`).
    pub mode: String,
    pub modes: Vec<String>,
    pub cursor: usize,
    /// What the set point may be: its ends and its step.
    pub min: f64,
    pub max: f64,
    pub step: f64,
    /// Whether Home Assistant lets the TV set a temperature, and a mode.
    pub may_set: bool,
    pub may_mode: bool,
    /// Why it may do neither, in the helper's words.
    pub why: String,
    pending: Option<(Act, u64)>,
    changed_at: u64,
}

fn number(ctl: &Value, key: &str) -> Option<f64> {
    ctl.get(key).and_then(Value::as_f64)
}

impl ThermostatSheet {
    /// The sheet of the thermostat `tile` (the helper's tile: its `ctl` says what it reads and is set to, its `verbs` what the TV may do).
    pub fn new(id: &str, tile: &Value) -> Self {
        let mut s = Self {
            id: id.to_owned(),
            rows: Vec::new(),
            at: 0,
            current: None,
            action: String::new(),
            humidity: None,
            humidity_target: None,
            target: None,
            low: None,
            high: None,
            mode: String::new(),
            modes: Vec::new(),
            cursor: 0,
            min: 50.0,
            max: 90.0,
            step: 1.0,
            may_set: false,
            may_mode: false,
            why: String::new(),
            pending: None,
            changed_at: 0,
        };
        s.read(tile);
        s
    }

    /// What the topic says of the thermostat now, taken as the sheet's own.
    fn read(&mut self, tile: &Value) {
        let ctl = tile.get("ctl").cloned().unwrap_or(Value::Null);
        let verbs: Vec<&str> = tile.get("verbs").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        self.may_set = verbs.contains(&"set_temperature");
        self.why = tile.get("why").and_then(Value::as_str).unwrap_or("").to_owned();
        self.current = number(&ctl, "current");
        self.action = ctl.get("action").and_then(Value::as_str).unwrap_or("").to_owned();
        self.humidity = number(&ctl, "humidity");
        self.humidity_target = number(&ctl, "humidity_target");
        self.target = number(&ctl, "target");
        self.low = number(&ctl, "low");
        self.high = number(&ctl, "high");
        self.min = number(&ctl, "min").unwrap_or(50.0);
        self.max = number(&ctl, "max").unwrap_or(90.0).max(self.min);
        self.step = number(&ctl, "step").filter(|s| *s > 0.0).unwrap_or(1.0);
        self.mode = ctl.get("mode").and_then(Value::as_str).unwrap_or("").to_owned();
        self.modes = ctl.get("modes").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
        self.may_mode = verbs.contains(&"set_hvac_mode") && self.modes.len() > 1;
        self.cursor = self.modes.iter().position(|m| *m == self.mode).unwrap_or(0);
        self.rows = self.rows_of();
        self.at = self.at.min(self.rows.len().saturating_sub(1));
    }

    /// The set points the sheet shows: the one, or the two of a range; none while the thermostat is off or has none.
    pub fn shown(&self) -> Vec<Row> {
        if self.mode == "off" {
            Vec::new()
        } else if self.target.is_some() {
            vec![Row::Set]
        } else if self.low.is_some() && self.high.is_some() {
            vec![Row::Low, Row::High]
        } else {
            Vec::new()
        }
    }

    /// The rows the keys go to: the set points shown, when Home Assistant lets the TV set a temperature, and the modes, when it lets it choose one.
    fn rows_of(&self) -> Vec<Row> {
        let mut rows = if self.may_set { self.shown() } else { Vec::new() };
        if self.may_mode {
            rows.push(Row::Mode);
        }
        rows
    }

    /// The topic changed: what it says becomes the sheet's own unless the person is in the middle of setting something.
    pub fn sync(&mut self, tile: &Value, now: u64) {
        if self.pending.is_none() && now >= self.changed_at + HOLD_MS {
            self.read(tile);
        }
    }

    pub fn row(&self) -> Option<Row> {
        self.rows.get(self.at).copied()
    }

    /// The value of a set point row.
    pub fn value(&self, row: Row) -> Option<f64> {
        match row {
            Row::Set => self.target,
            Row::Low => self.low,
            Row::High => self.high,
            Row::Mode => None,
        }
    }

    /// A temperature as the sheet says it.
    pub fn degrees(v: f64) -> String {
        format!("{}\u{b0}", num_text(v))
    }

    /// What the thermostat is doing, in a word, when it says (nothing for an action it is not doing).
    pub fn action_words(&self) -> &'static str {
        match self.action.as_str() {
            "heating" => "Heating",
            "cooling" => "Cooling",
            "idle" => "Idle",
            "drying" => "Drying",
            "fan" => "Fan running",
            _ => "",
        }
    }

    /// The humidity line: what it reads, and what it is set to.
    pub fn humidity_words(&self) -> Option<String> {
        let now = self.humidity?;
        Some(match self.humidity_target {
            Some(t) => format!("Humidity {}% \u{b7} set to {}%", num_text(now), num_text(t)),
            None => format!("Humidity {}%", num_text(now)),
        })
    }

    /// A mode as the chips say it.
    pub fn mode_word(mode: &str) -> String {
        mode_word(mode)
    }

    fn snap(&self, v: f64) -> f64 {
        (((v / self.step).round() * self.step).clamp(self.min, self.max) * 100.0).round() / 100.0
    }

    /// The set point `row` set to `to`, kept inside its ends and, for a range, a step apart; the call that follows is sent when it has rested.
    pub fn set(&mut self, row: Row, to: f64, now: u64) {
        let v = self.snap(to);
        let apart = self.step;
        match row {
            Row::Set => self.target = Some(v),
            Row::Low => self.low = Some(v.min(self.high.map_or(v, |h| h - apart)).max(self.min)),
            Row::High => self.high = Some(v.max(self.low.map_or(v, |l| l + apart)).min(self.max)),
            Row::Mode => return,
        }
        let id = self.id.clone();
        let act = match (row, self.target, self.low.zip(self.high)) {
            (Row::Set, Some(t), _) => Act { id, verb: "temperature", value: json!(t) },
            (_, _, Some((low, high))) => Act { id, verb: "range", value: json!({"low": low, "high": high}) },
            _ => return,
        };
        self.pending = Some((act, now + REST_MS));
        self.changed_at = now;
    }

    fn nudge(&mut self, row: Row, dir: i32, now: u64) {
        let Some(v) = self.value(row) else { return };
        let to = v + f64::from(dir) * self.step;
        self.set(row, to, now);
    }

    /// Chooses the mode `index` of the chips: the call is made at once, and the sheet shows the mode as chosen.
    pub fn choose(&mut self, index: usize, now: u64) -> Done {
        let Some(mode) = self.modes.get(index).cloned() else { return Done::Nothing };
        self.cursor = index;
        if mode == self.mode {
            return Done::Nothing;
        }
        self.mode = mode.clone();
        self.changed_at = now;
        // a set point that waited goes with the old mode's rows: it is not sent over the new one
        self.pending = None;
        self.rows = self.rows_of();
        self.at = self.at.min(self.rows.len().saturating_sub(1));
        Done::Send(Act { id: self.id.clone(), verb: "hvac_mode", value: json!(mode) })
    }

    /// A key: Up and Down go to another row, Left and Right step the set point or walk the chips, OK chooses the chip the cursor is on (and sends
    /// a set point that is resting at once), Back closes.
    pub fn key(&mut self, name: &str, now: u64) -> Done {
        match name {
            "back" => return Done::Close,
            "up" => self.at = self.at.saturating_sub(1),
            "down" => self.at = (self.at + 1).min(self.rows.len().saturating_sub(1)),
            "left" | "right" => {
                let dir = if name == "left" { -1 } else { 1 };
                match self.row() {
                    Some(Row::Mode) => self.cursor = (self.cursor as i32 + dir).clamp(0, self.modes.len() as i32 - 1) as usize,
                    Some(row) => self.nudge(row, dir, now),
                    None => {}
                }
            }
            "ok" => match self.row() {
                Some(Row::Mode) => return self.choose(self.cursor, now),
                Some(_) => {
                    if let Some((act, _)) = self.pending.take() {
                        return Done::Send(act);
                    }
                }
                None => return Done::Close,
            },
            _ => {}
        }
        Done::Nothing
    }

    /// The call of a set point that has rested long enough.
    pub fn due(&mut self, now: u64) -> Option<Act> {
        match &self.pending {
            Some((_, at)) if now >= *at => self.pending.take().map(|(a, _)| a),
            _ => None,
        }
    }

    /// When the call of a set point that rests is due.
    pub fn due_at(&self) -> Option<u64> {
        self.pending.as_ref().map(|(_, at)| *at)
    }
}

#[cfg(test)]
mod tests {
    use super::super::owner_tile;
    use super::*;

    fn hall() -> Value {
        owner_tile("climate.living_room_thermostat")
    }

    fn bedjet() -> Value {
        owner_tile("climate.master_bedroom_bedjet")
    }

    fn office() -> Value {
        owner_tile("climate.nitin_s_office_ac")
    }

    /// One thermostat of the owner's in a mode: its tile with the mode changed, as the topic would say it.
    fn in_mode(mut tile: Value, mode: &str, target: Option<f64>) -> Value {
        tile["ctl"]["mode"] = json!(mode);
        tile["ctl"]["target"] = json!(target);
        tile
    }

    #[test]
    fn the_rows_are_the_ones_the_thermostat_has_in_the_mode_it_is_in_and_start_from_what_it_reads() {
        // the living room's, heating and cooling between two set points: a low, a high and the modes
        let s = ThermostatSheet::new("climate.living_room_thermostat", &hall());
        assert_eq!(s.rows, [Row::Low, Row::High, Row::Mode]);
        assert_eq!((s.low, s.high, s.current, s.humidity, s.humidity_target), (Some(67.0), Some(76.0), Some(74.0), Some(77.0), Some(50.0)));
        assert_eq!((s.min, s.max, s.step, s.mode.as_str(), s.modes.len(), s.cursor), (64.0, 84.0, 1.0, "heat_cool", 4, 3));
        assert_eq!(s.humidity_words().as_deref(), Some("Humidity 77% \u{b7} set to 50%"));
        assert_eq!((s.action_words(), ThermostatSheet::degrees(74.0), ThermostatSheet::degrees(62.5)), ("Idle", "74\u{b0}".to_owned(), "62.5\u{b0}".to_owned()));
        // one set point in the modes that have one
        let heat = ThermostatSheet::new("climate.living_room_thermostat", &in_mode(hall(), "heat", Some(70.0)));
        assert_eq!((heat.rows.clone(), heat.target), (vec![Row::Set, Row::Mode], Some(70.0)));
        // off has no set point to move: the modes only
        let off = ThermostatSheet::new("climate.nitin_s_office_ac", &office());
        assert_eq!((off.rows.clone(), off.step, off.min, off.humidity_words()), (vec![Row::Mode], 0.5, 62.5, None));
        // a thermostat that reads no humidity says none, and one Home Assistant lets the TV do nothing to has no row at all
        let mut shown = bedjet();
        shown["verbs"] = json!([]);
        shown["why"] = json!("Home Assistant does not let the TV control this");
        let mut read_only = ThermostatSheet::new("climate.master_bedroom_bedjet", &shown);
        assert_eq!((read_only.rows.is_empty(), read_only.why.as_str(), read_only.humidity), (true, "Home Assistant does not let the TV control this", None));
        assert_eq!(read_only.key("ok", 0), Done::Close, "nothing to press: OK puts the sheet away");
    }

    #[test]
    fn right_and_left_step_the_set_point_by_the_thermostats_own_step_and_the_call_goes_when_it_has_rested() {
        let mut s = ThermostatSheet::new("climate.nitin_s_office_ac", &in_mode(office(), "cool", Some(74.5)));
        assert_eq!(s.row(), Some(Row::Set));
        assert_eq!(s.key("right", 1000), Done::Nothing);
        assert_eq!(s.target, Some(75.0), "the office unit steps by half a degree");
        for t in [1040, 1080, 1120] {
            s.key("right", t);
        }
        assert_eq!(s.target, Some(76.5));
        assert!(s.due(1300).is_none() && s.due_at() == Some(1120 + REST_MS), "a held key is one call");
        let act = s.due(1120 + REST_MS).expect("it has rested");
        assert_eq!((act.id.as_str(), act.verb, act.value.clone()), ("climate.nitin_s_office_ac", "temperature", json!(76.5)));
        assert!(s.due(5000).is_none() && s.due_at().is_none());
        // never past its own ends
        for t in 0..80 {
            s.key("right", 2000 + t);
        }
        assert_eq!(s.target, Some(86.0));
        for t in 0..80 {
            s.key("left", 3000 + t);
        }
        assert_eq!(s.target, Some(62.5));
    }

    #[test]
    fn a_range_moves_one_end_at_a_time_and_the_ends_never_cross() {
        let mut s = ThermostatSheet::new("climate.living_room_thermostat", &hall());
        assert_eq!(s.row(), Some(Row::Low));
        s.key("right", 100);
        assert_eq!((s.low, s.high), (Some(68.0), Some(76.0)));
        let act = s.due(100 + REST_MS).expect("a range goes whole");
        assert_eq!((act.verb, act.value), ("range", json!({"low": 68.0, "high": 76.0})));
        s.key("down", 500);
        assert_eq!(s.row(), Some(Row::High));
        s.key("left", 600);
        assert_eq!(s.high, Some(75.0));
        for t in 0..20 {
            s.key("left", 700 + t);
        }
        assert_eq!((s.low, s.high), (Some(68.0), Some(69.0)), "the high end stops a step above the low end");
        s.key("up", 900);
        for t in 0..20 {
            s.key("right", 1000 + t);
        }
        assert_eq!((s.low, s.high), (Some(68.0), Some(69.0)), "and the low end a step below the high");
        assert_eq!(s.key("back", 1100), Done::Close);
    }

    #[test]
    fn a_mode_is_walked_to_and_chosen_with_ok_and_is_not_switched_in_passing() {
        let mut s = ThermostatSheet::new("climate.living_room_thermostat", &in_mode(hall(), "heat", Some(70.0)));
        s.key("down", 0);
        assert_eq!(s.row(), Some(Row::Mode));
        assert_eq!(s.cursor, 1, "it starts on the mode it is in");
        assert_eq!(s.key("right", 10), Done::Nothing);
        assert_eq!((s.cursor, s.mode.as_str()), (2, "heat"), "walking changes nothing");
        assert!(s.due(10_000).is_none());
        assert_eq!(s.key("ok", 20), Done::Send(Act { id: "climate.living_room_thermostat".into(), verb: "hvac_mode", value: json!("cool") }));
        assert_eq!(s.mode, "cool", "shown as chosen at once");
        assert_eq!(s.key("ok", 30), Done::Nothing, "the mode it is in is not asked for again");
        // never past the ends
        for _ in 0..9 {
            s.key("right", 40);
        }
        assert_eq!(s.cursor, 3);
        for _ in 0..9 {
            s.key("left", 40);
        }
        assert_eq!(s.cursor, 0);
        // off takes its set point away; the modes stay
        assert!(matches!(s.key("ok", 50), Done::Send(a) if a.value == json!("off")));
        assert_eq!(s.rows, [Row::Mode]);
        assert_eq!(s.at, 0);
    }

    #[test]
    fn a_set_point_that_waits_is_not_sent_over_another_mode_and_ok_sends_it_at_once() {
        let mut s = ThermostatSheet::new("climate.living_room_thermostat", &in_mode(hall(), "heat", Some(70.0)));
        s.key("right", 100);
        let act = match s.key("ok", 150) {
            Done::Send(a) => a,
            other => panic!("{other:?}"),
        };
        assert_eq!((act.verb, act.value), ("temperature", json!(71.0)));
        assert!(s.due_at().is_none(), "OK sent it: it is not sent again");
        s.key("right", 200);
        s.key("down", 210);
        assert_eq!(s.key("ok", 220), Done::Nothing, "the mode it is in is not asked for again");
        assert!(s.due_at().is_some(), "and the set point that waits still waits");
        s.key("right", 230);
        assert!(matches!(s.key("ok", 240), Done::Send(a) if a.verb == "hvac_mode" && a.value == json!("cool")));
        assert!(s.due_at().is_none(), "the waiting set point went with the old mode");
    }

    #[test]
    fn the_topic_is_taken_when_the_person_has_let_go_and_not_before() {
        let mut s = ThermostatSheet::new("climate.living_room_thermostat", &in_mode(hall(), "heat", Some(70.0)));
        s.key("right", 1000);
        let mut other = in_mode(hall(), "heat", Some(66.0));
        other["ctl"]["current"] = json!(71.5);
        s.sync(&other, 1100);
        assert_eq!(s.target, Some(71.0), "mid-change: what was set stays");
        s.due(1300);
        s.sync(&other, 1500);
        assert_eq!(s.target, Some(71.0), "sent, but the topic has not caught up: held a little longer");
        s.sync(&other, 1000 + HOLD_MS);
        assert_eq!((s.target, s.current), (Some(66.0), Some(71.5)), "a change made elsewhere is the sheet's again");
        // a thermostat that is switched off elsewhere loses its set point row
        s.sync(&in_mode(hall(), "off", None), 10_000);
        assert_eq!(s.rows, [Row::Mode]);
    }
}
