//! The PIN pad of a profile (`docs/research/profiles-design.md`, Screens): four dots; the remote's digits fill them and the fourth
//! submits; a 3 by 4 pad for the D-pad and the pointer; a wrong PIN shakes and clears (`403 wrong_pin`, with the tries left), and
//! `429 pin_locked` shows the wait (`retryAfterS`). A pure model: the quick panel's strip asks it in place, and "Who's watching?"
//! asks it over the picker. The words are state, never an instruction: the name of whose PIN it is and, under it, what happened.
//!
//! The pad's cells in reading order: 1 2 3 / 4 5 6 / 7 8 9 / delete 0 cancel.

use super::K;
use crate::kit::screen::Effect;
use serde_json::{Value, json};

pub const LENGTH: usize = 4;
pub const COLUMNS: usize = 3;
pub const CELLS: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Digit(u8),
    Delete,
    Cancel,
}

pub fn cell(i: usize) -> Cell {
    match i {
        0..=8 => Cell::Digit(i as u8 + 1),
        9 => Cell::Delete,
        10 => Cell::Digit(0),
        _ => Cell::Cancel,
    }
}

/// What a step of the pad asks of its host.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Nothing for the host: the view changed or not.
    None,
    /// The fourth digit is in: the request to switch with it (the answer comes back through [`Pin::answer`]).
    Submit(Vec<Effect>),
    /// The person backed out.
    Cancel,
}

/// What an answer of the helper means for the pad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The PIN was right: the switch is done.
    Done,
    /// Wrong: the pad shook and cleared.
    Wrong,
    /// Too many wrong ones: the pad waits.
    Locked,
    /// The helper could not say.
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub name: String,
    pub filled: usize,
    /// How many times it has shaken: a new number is a new shake.
    pub shake: u32,
    /// What happened, in words: "Wrong PIN", "2 tries left", "Try again in 27 s".
    pub note: String,
    pub focus: usize,
    pub locked: bool,
}

#[derive(Clone, Debug)]
pub struct Pin {
    pub id: String,
    pub name: String,
    tag: u64,
    digits: String,
    wrong: bool,
    tries_left: Option<u32>,
    failed: bool,
    locked_until: Option<u64>,
    pending: bool,
    focus: usize,
    shake: u32,
}

impl Pin {
    /// A pad for profile `id` (shown as `name`), whose switch request comes back with `tag`.
    pub fn new(id: &str, name: &str, tag: u64) -> Self {
        Self { id: id.to_string(), name: name.to_string(), tag, digits: String::new(), wrong: false, tries_left: None, failed: false, locked_until: None, pending: false, focus: 4, shake: 0 }
    }

    pub fn tag(&self) -> u64 {
        self.tag
    }

    pub fn filled(&self) -> usize {
        self.digits.len()
    }

    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn pending(&self) -> bool {
        self.pending
    }

    fn waiting(&self, now: u64) -> Option<u64> {
        self.locked_until.filter(|u| *u > now).map(|u| (u - now).div_ceil(1000))
    }

    pub fn view(&self, now: u64) -> View {
        let wait = self.waiting(now);
        let note = if let Some(s) = wait {
            format!("Try again in {s} s")
        } else if self.failed {
            "Could not switch".to_string()
        } else if self.wrong {
            match self.tries_left {
                Some(1) => "Wrong PIN, 1 try left".to_string(),
                Some(n) if n > 1 => format!("Wrong PIN, {n} tries left"),
                _ => "Wrong PIN".to_string(),
            }
        } else {
            String::new()
        };
        View { name: self.name.clone(), filled: self.digits.len(), shake: self.shake, note, focus: self.focus, locked: wait.is_some() }
    }

    fn type_digit(&mut self, d: u8, now: u64) -> Step {
        if self.pending || self.waiting(now).is_some() || self.digits.len() >= LENGTH {
            return Step::None;
        }
        self.wrong = false;
        self.failed = false;
        self.digits.push(char::from(b'0' + d));
        if self.digits.len() == LENGTH {
            self.pending = true;
            let body = json!({"id": self.id, "pin": self.digits});
            return Step::Submit(vec![Effect::Request { tag: self.tag, method: "POST".into(), path: "/v1/profiles/switch".into(), body: Some(body) }]);
        }
        Step::None
    }

    fn press(&mut self, c: Cell, now: u64) -> Step {
        match c {
            Cell::Digit(d) => self.type_digit(d, now),
            Cell::Delete => {
                if !self.pending {
                    self.digits.pop();
                }
                Step::None
            }
            Cell::Cancel => Step::Cancel,
        }
    }

    /// A key of the remote: digits type, the arrows move over the pad, OK presses the cell, Back is the way out.
    pub fn key(&mut self, k: K, now: u64) -> Step {
        match k {
            K::Digit(d) => self.type_digit(d, now),
            K::Back => Step::Cancel,
            K::Ok => self.press(cell(self.focus), now),
            K::Left => {
                if self.focus % COLUMNS > 0 {
                    self.focus -= 1;
                }
                Step::None
            }
            K::Right => {
                if self.focus % COLUMNS < COLUMNS - 1 {
                    self.focus += 1;
                }
                Step::None
            }
            K::Up => {
                if self.focus >= COLUMNS {
                    self.focus -= COLUMNS;
                }
                Step::None
            }
            K::Down => {
                if self.focus + COLUMNS < CELLS {
                    self.focus += COLUMNS;
                }
                Step::None
            }
        }
    }

    /// A click of the pointer on cell `i`.
    pub fn click(&mut self, i: usize, now: u64) -> Step {
        if i >= CELLS {
            return Step::None;
        }
        self.focus = i;
        self.press(cell(i), now)
    }

    /// The helper's answer to the switch the fourth digit asked for.
    pub fn answer(&mut self, status: u16, data: &Value, now: u64) -> Verdict {
        self.pending = false;
        self.digits.clear();
        if (200..300).contains(&status) {
            return Verdict::Done;
        }
        let err = data.get("error");
        match err.and_then(|e| e.get("code")).and_then(Value::as_str) {
            Some("wrong_pin") => {
                self.wrong = true;
                self.failed = false;
                self.tries_left = err.and_then(|e| e.get("triesLeft")).and_then(Value::as_u64).map(|n| n as u32);
                self.shake += 1;
                Verdict::Wrong
            }
            Some("pin_locked") => {
                let secs = err.and_then(|e| e.get("retryAfterS")).and_then(Value::as_u64).unwrap_or(30);
                self.locked_until = Some(now + secs * 1000);
                self.wrong = false;
                self.failed = false;
                self.shake += 1;
                Verdict::Locked
            }
            _ => {
                self.failed = true;
                Verdict::Failed
            }
        }
    }

    /// When the wait's words next change: every second while it runs.
    pub fn due_at(&self, now: u64) -> Option<u64> {
        self.waiting(now).map(|_| now + 1000 - now % 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_all(p: &mut Pin, digits: &[u8], now: u64) -> Step {
        let mut last = Step::None;
        for d in digits {
            last = p.key(K::Digit(*d), now);
        }
        last
    }

    #[test]
    fn the_digits_fill_the_dots_and_the_fourth_submits_the_switch() {
        let mut p = Pin::new("isabel", "Isabel", 77);
        assert_eq!(type_all(&mut p, &[1, 2, 3], 0), Step::None);
        assert_eq!(p.filled(), 3);
        let Step::Submit(e) = p.key(K::Digit(4), 0) else { panic!("the fourth submits") };
        assert_eq!(e, vec![Effect::Request { tag: 77, method: "POST".into(), path: "/v1/profiles/switch".into(), body: Some(json!({"id": "isabel", "pin": "1234"})) }]);
        assert!(p.pending());
        assert_eq!(p.key(K::Digit(5), 0), Step::None, "nothing is typed while the answer is awaited");
        assert_eq!(p.filled(), 4);
    }

    #[test]
    fn a_wrong_pin_shakes_and_clears_and_says_how_many_tries_are_left() {
        let mut p = Pin::new("isabel", "Isabel", 77);
        type_all(&mut p, &[1, 2, 3, 4], 0);
        let v = p.answer(403, &json!({"ok": false, "error": {"code": "wrong_pin", "triesLeft": 2}}), 100);
        assert_eq!(v, Verdict::Wrong);
        let view = p.view(100);
        assert_eq!((view.filled, view.shake, view.note.as_str()), (0, 1, "Wrong PIN, 2 tries left"));
        // typing again clears the words
        p.key(K::Digit(9), 200);
        assert_eq!(p.view(200).note, "");
        type_all(&mut p, &[1, 2, 3], 200);
        p.answer(403, &json!({"error": {"code": "wrong_pin", "triesLeft": 1}}), 300);
        assert_eq!(p.view(300).note, "Wrong PIN, 1 try left");
        assert_eq!(p.view(300).shake, 2, "each wrong one shakes anew");
    }

    #[test]
    fn too_many_wrong_ones_wait_and_the_wait_counts_down_by_the_second() {
        let mut p = Pin::new("isabel", "Isabel", 77);
        type_all(&mut p, &[1, 1, 1, 1], 0);
        assert_eq!(p.answer(429, &json!({"error": {"code": "pin_locked", "retryAfterS": 30}}), 1000), Verdict::Locked);
        let v = p.view(1000);
        assert!(v.locked);
        assert_eq!(v.note, "Try again in 30 s");
        assert_eq!(p.view(4_500).note, "Try again in 27 s");
        assert_eq!(p.due_at(4_500), Some(5_000), "the words change at the next whole second");
        assert_eq!(p.key(K::Digit(1), 5_000), Step::None, "digits are refused during the wait");
        assert_eq!(p.filled(), 0);
        assert!(!p.view(31_000).locked);
        assert_eq!(p.due_at(31_000), None);
        assert_eq!(p.key(K::Digit(1), 31_000), Step::None);
        assert_eq!(p.filled(), 1, "typing works again after the wait");
    }

    #[test]
    fn the_right_pin_is_done_and_an_answer_nobody_understands_is_a_failure() {
        let mut p = Pin::new("isabel", "Isabel", 77);
        type_all(&mut p, &[1, 2, 3, 4], 0);
        assert_eq!(p.answer(200, &json!({"ok": true}), 10), Verdict::Done);
        let mut p = Pin::new("isabel", "Isabel", 77);
        type_all(&mut p, &[1, 2, 3, 4], 0);
        assert_eq!(p.answer(500, &Value::Null, 10), Verdict::Failed);
        assert_eq!(p.view(10).note, "Could not switch");
    }

    #[test]
    fn the_pad_is_three_by_four_for_the_dpad_and_ok_presses_the_cell() {
        let mut p = Pin::new("isabel", "Isabel", 77);
        assert_eq!(cell(p.focus()), Cell::Digit(5), "it opens on the middle");
        assert_eq!(p.key(K::Up, 0), Step::None);
        assert_eq!(cell(p.focus()), Cell::Digit(2));
        assert_eq!(p.key(K::Up, 0), Step::None);
        assert_eq!(cell(p.focus()), Cell::Digit(2), "the top row holds");
        p.key(K::Right, 0);
        p.key(K::Right, 0);
        assert_eq!(cell(p.focus()), Cell::Digit(3), "the right edge holds");
        p.key(K::Ok, 0);
        assert_eq!(p.filled(), 1);
        for _ in 0..5 {
            p.key(K::Down, 0);
        }
        assert_eq!(cell(p.focus()), Cell::Cancel, "the bottom right is the way out");
        p.key(K::Left, 0);
        p.key(K::Left, 0);
        assert_eq!(cell(p.focus()), Cell::Delete);
        p.key(K::Ok, 0);
        assert_eq!(p.filled(), 0, "delete takes the last digit back");
        p.key(K::Right, 0);
        p.key(K::Ok, 0);
        assert_eq!(p.filled(), 1, "the zero");
        p.key(K::Right, 0);
        assert_eq!(p.key(K::Ok, 0), Step::Cancel);
        assert_eq!(p.key(K::Back, 0), Step::Cancel);
    }

    #[test]
    fn a_click_presses_the_cell_it_is_on() {
        let mut p = Pin::new("isabel", "Isabel", 77);
        p.click(0, 0);
        p.click(10, 0);
        assert_eq!(p.filled(), 2);
        assert_eq!(p.click(9, 0), Step::None);
        assert_eq!(p.filled(), 1);
        assert_eq!(p.click(11, 0), Step::Cancel);
        assert_eq!(p.click(99, 0), Step::None);
    }
}
