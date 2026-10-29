//! The sleep timer the quick panel shows and sets (`kit/Power.qml`): LG's own service. `timer/sleep/getRemainTime` answers
//! `{sleepTimeMinute, sleepTimeSecond}` (what is left; an idle timer answers none) and `timer/sleep/add {sleepTimeMinute: "30"}` sets
//! it (`"0"` clears it). LG's choices are Off, 10, 20, 30, 60, 90, 120, 180 and 240 minutes, and its row shows the first choice that is
//! not shorter than what is left. Until the service has answered the timer reads Off, never a made-up number.

use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

pub const REMAIN: &str = "luna://com.webos.service.tvReservationAgent/timer/sleep/getRemainTime";
pub const ADD: &str = "luna://com.webos.service.tvReservationAgent/timer/sleep/add";
pub const READ_TAG: u64 = 0x5000_0001;
pub const SET_TAG: u64 = 0x5000_0002;
/// The service takes a moment to start the countdown: the timer is read back once after a change.
pub const AGAIN_MS: u64 = 700;

pub const SLEEP_MINUTES: [u32; 9] = [0, 10, 20, 30, 60, 90, 120, 180, 240];
pub const SLEEP_LABELS: [&str; 9] = ["Off", "10 min", "20 min", "30 min", "1 h", "90 min", "2 h", "3 h", "4 h"];

#[derive(Clone, Debug, Default)]
pub struct Power {
    /// What is left, in seconds; 0 when none runs or the service has not said.
    sleep_seconds: f64,
    again_due: Option<u64>,
}

impl Power {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sleep_seconds(&self) -> f64 {
        self.sleep_seconds
    }

    /// The choice that shows: the first not shorter than the whole minutes left.
    pub fn sleep_index(&self) -> usize {
        if self.sleep_seconds <= 0.0 {
            return 0;
        }
        let minutes = (self.sleep_seconds / 60.0).floor() as u32;
        (1..SLEEP_MINUTES.len()).find(|i| minutes <= SLEEP_MINUTES[*i]).unwrap_or(SLEEP_MINUTES.len() - 1)
    }

    /// "Off in 23 min" while one runs; nothing otherwise.
    pub fn sleep_left(&self) -> String {
        if self.sleep_seconds <= 0.0 {
            String::new()
        } else if self.sleep_seconds < 60.0 {
            "Off in under a minute".into()
        } else {
            format!("Off in {} min", (self.sleep_seconds / 60.0).round() as u64)
        }
    }

    pub fn read(&self) -> Vec<Effect> {
        vec![Effect::Luna { tag: READ_TAG, uri: REMAIN.into(), payload: json!({}), subscribe: false }]
    }

    /// Sets choice `i`: it shows at once and the service's answer replaces it.
    pub fn set(&mut self, i: usize, now: u64) -> Vec<Effect> {
        let minutes = SLEEP_MINUTES[i.min(SLEEP_MINUTES.len() - 1)];
        self.sleep_seconds = f64::from(minutes) * 60.0;
        self.again_due = Some(now + AGAIN_MS);
        vec![Effect::Luna { tag: SET_TAG, uri: ADD.into(), payload: json!({"sleepTimeMinute": minutes.to_string()}), subscribe: false }]
    }

    /// When the timer would turn the TV off if choice `i` were set now, as a clock time: the picker's grey words.
    pub fn minutes_of(i: usize) -> u32 {
        SLEEP_MINUTES[i.min(SLEEP_MINUTES.len() - 1)]
    }

    pub fn update(&mut self, input: &Input, now: u64) -> (Vec<Effect>, bool) {
        match input {
            Input::Luna { tag, payload } if *tag == READ_TAG => {
                if payload.get("returnValue").and_then(Value::as_bool) == Some(false) {
                    return (Vec::new(), false);
                }
                let n = |k: &str| payload.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0.0);
                let seconds = n("sleepTimeMinute") * 60.0 + n("sleepTimeSecond");
                let changed = seconds != self.sleep_seconds;
                self.sleep_seconds = seconds;
                (Vec::new(), changed)
            }
            Input::Tick if self.again_due.is_some_and(|d| now >= d) => {
                self.again_due = None;
                (self.read(), false)
            }
            _ => (Vec::new(), false),
        }
    }

    pub fn due_at(&self) -> Option<u64> {
        self.again_due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn left(p: &mut Power, minute: u32, second: u32) {
        p.update(&Input::Luna { tag: READ_TAG, payload: json!({"returnValue": true, "sleepTimeMinute": minute, "sleepTimeSecond": second}) }, 0);
    }

    #[test]
    fn the_choice_shown_is_the_first_not_shorter_than_what_is_left() {
        let mut p = Power::new();
        assert_eq!((p.sleep_index(), p.sleep_left().as_str()), (0, ""));
        left(&mut p, 23, 10);
        assert_eq!(p.sleep_index(), 3, "23 minutes left shows 30 min");
        assert_eq!(p.sleep_left(), "Off in 23 min");
        left(&mut p, 0, 40);
        assert_eq!(p.sleep_index(), 1);
        assert_eq!(p.sleep_left(), "Off in under a minute");
        left(&mut p, 240, 0);
        assert_eq!(p.sleep_index(), 8);
        left(&mut p, 300, 0);
        assert_eq!(p.sleep_index(), 8, "longer than the longest shows the longest");
    }

    #[test]
    fn a_choice_shows_at_once_is_sent_as_text_and_is_read_back_once() {
        let mut p = Power::new();
        let e = p.set(3, 1000);
        assert_eq!(p.sleep_index(), 3);
        let [Effect::Luna { uri, payload, .. }] = e.as_slice() else { panic!() };
        assert!(uri.ends_with("/timer/sleep/add"));
        assert_eq!(payload, &json!({"sleepTimeMinute": "30"}));
        assert_eq!(p.due_at(), Some(1700));
        assert!(p.update(&Input::Tick, 1699).0.is_empty());
        let (again, _) = p.update(&Input::Tick, 1700);
        assert!(matches!(again.as_slice(), [Effect::Luna { tag, .. }] if *tag == READ_TAG));
        assert_eq!(p.due_at(), None);
        let off = p.set(0, 2000);
        let [Effect::Luna { payload, .. }] = off.as_slice() else { panic!() };
        assert_eq!(payload, &json!({"sleepTimeMinute": "0"}));
        assert_eq!(p.sleep_left(), "");
    }

    #[test]
    fn a_refused_read_changes_nothing() {
        let mut p = Power::new();
        left(&mut p, 10, 0);
        p.update(&Input::Luna { tag: READ_TAG, payload: json!({"returnValue": false}) }, 0);
        assert_eq!(p.sleep_index(), 1);
    }
}
