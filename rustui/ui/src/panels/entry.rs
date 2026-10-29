//! Text typed on the TV for one of the shell's own screens (a Wi-Fi password, a name, an address): a sheet along the bottom with the field and
//! the keyboard of Home's search (`home/search.rs` holds the keys, the case and the symbols; the sheet in Home types a name or a search, this
//! types for a screen that asked). The owner of the field asks with [`Entry::open`] and is told one of two things: the text, when Done is
//! pressed (or Enter on a keyboard), or that the person backed out. Nothing else leaves the sheet, and the text is never in an event or a
//! log: it goes back to the one that asked, once.
//!
//! Keys: the arrows move over the keys, OK types the key, the number keys type their digit, Back cancels. A password shows a bullet for each
//! character but the last, so a slip can be seen.

use super::life::Life;
use super::{K, Out, Typed};
use crate::home::search::{Search, SearchOut};

/// The most characters a field takes: the longest thing typed here is a Wi-Fi passphrase (63), and a field is one line.
pub const MAX_CHARS: usize = 128;
/// The key the focus starts on: `t`, as in Home's search.
const START: usize = 14;
/// Keys in the grid: ten across for four rows, then `?123`, space and Done.
pub const KEYS: usize = 43;

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub prompt: String,
    /// The text as it is shown (bullets for a password).
    pub shown: String,
    pub secret: bool,
    pub shift: bool,
    pub symbols: bool,
    pub index: usize,
    /// How many characters are in the field.
    pub chars: usize,
    /// What each of the 43 keys says now (the letters or the marks).
    pub caps: Vec<&'static str>,
    pub over_video: bool,
}

#[derive(Default)]
pub struct Entry {
    pub life: Life,
    search: Search,
    index: usize,
    up: bool,
}

/// The key under `index` that is nearest to a move: left and right along the row, up and down to the key whose span holds the column.
fn step(index: usize, k: K) -> usize {
    let row = if index < 40 { index / 10 } else { 4 };
    let frame = |i: usize| Search::key_frame(i);
    match k {
        K::Left | K::Right => {
            let d: i32 = if k == K::Left { -1 } else { 1 };
            let (lo, hi) = if row < 4 { (row * 10, row * 10 + 9) } else { (40, 42) };
            (index as i32 + d).clamp(lo as i32, hi as i32) as usize
        }
        K::Up | K::Down => {
            let to = if k == K::Up { row as i32 - 1 } else { row as i32 + 1 };
            if !(0..=4).contains(&to) {
                return index;
            }
            let x = frame(index).x + frame(index).w / 2.0;
            let range = if to < 4 { to as usize * 10..to as usize * 10 + 10 } else { 40..43 };
            // the key whose span holds the column; else the nearest centre
            range
                .min_by(|a, b| {
                    let dist = |i: &usize| {
                        let f = frame(*i);
                        if x >= f.x && x < f.x + f.w { 0.0 } else { (f.x + f.w / 2.0 - x).abs() }
                    };
                    dist(a).total_cmp(&dist(b))
                })
                .unwrap_or(index)
        }
        _ => index,
    }
}

impl Entry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_up(&self) -> bool {
        self.up
    }

    /// The field is asked for. `secret` shows the text as bullets.
    pub fn open(&mut self, prompt: &str, text: &str, secret: bool, now: u64) {
        let text: String = text.chars().take(MAX_CHARS).collect();
        self.search.open_entry(prompt, &text, now);
        self.search.secret = secret;
        self.index = START;
        self.up = true;
    }

    pub fn close(&mut self) {
        self.up = false;
    }

    fn typed(&self) -> Out {
        Out { typed: Some(Typed::Entered(self.search.text.clone())), close: true, ..Out::default() }
    }

    /// OK, or a click, on key `i`.
    fn press(&mut self, i: usize, now: u64) -> Out {
        let cap = self.search.key_at(i);
        let chars = self.search.text.chars().count();
        // a character key does nothing at the end of the field; the keys that are not characters always act
        if chars >= MAX_CHARS && !matches!(cap, "\u{232b}" | "\u{21e7}" | "?123" | "Done") {
            return Out::default();
        }
        let _ = now;
        match self.search.press(cap) {
            SearchOut::Entered(_) => self.typed(),
            _ => Out::default(),
        }
    }

    pub fn key(&mut self, k: K, now: u64) -> Out {
        match k {
            K::Back => Out { typed: Some(Typed::Cancelled), close: true, ..Out::default() },
            K::Ok => self.press(self.index, now),
            K::Digit(d) => {
                let cap = char::from(b'0' + d.min(9));
                if self.search.text.chars().count() < MAX_CHARS {
                    self.search.press(&cap.to_string());
                }
                Out::default()
            }
            k => {
                self.index = step(self.index, k);
                Out::default()
            }
        }
    }

    /// A click on key `i`: it takes the focus and is typed.
    pub fn click(&mut self, i: usize, now: u64) -> Out {
        if i >= KEYS {
            return Out::default();
        }
        self.index = i;
        self.press(i, now)
    }

    /// A click outside the sheet is Back.
    pub fn click_outside(&mut self) -> Out {
        self.key(K::Back, 0)
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn cap(&self, i: usize) -> &'static str {
        self.search.key_at(i)
    }

    /// Whether the caret is drawn (it blinks after a rest).
    pub fn caret_on(&self, now: u64) -> bool {
        self.search.caret_on(now)
    }

    pub fn view(&self, over_video: bool) -> View {
        View {
            prompt: self.search.prompt.clone(),
            shown: self.search.shown(),
            secret: self.search.secret,
            shift: self.search.shift,
            symbols: self.search.symbols,
            index: self.index,
            chars: self.search.text.chars().count(),
            caps: (0..KEYS).map(|i| self.search.key_at(i)).collect(),
            over_video,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_keys(e: &mut Entry, caps: &[&str]) {
        for cap in caps {
            let i = (0..KEYS).find(|i| e.cap(*i) == *cap).unwrap_or_else(|| panic!("no key {cap}"));
            e.click(i, 0);
        }
    }

    #[test]
    fn the_arrows_move_over_the_keys_the_wide_keys_hold_the_column_and_the_ends_hold() {
        let mut e = Entry::new();
        e.open("Wi-Fi password", "", true, 0);
        assert_eq!(e.cap(e.index()), "t");
        e.key(K::Left, 0);
        assert_eq!(e.cap(e.index()), "r");
        for _ in 0..12 {
            e.key(K::Left, 0);
        }
        assert_eq!(e.index(), 10, "the row's end holds");
        e.key(K::Up, 0);
        assert_eq!(e.index(), 0, "the digits are the top row");
        e.key(K::Up, 0);
        assert_eq!(e.index(), 0);
        e.key(K::Down, 0);
        e.key(K::Down, 0);
        e.key(K::Down, 0);
        e.key(K::Down, 0);
        assert_eq!(e.cap(e.index()), "?123", "the first column's key of the bottom row is the one that spans it");
        e.key(K::Right, 0);
        assert_eq!(e.cap(e.index()), "space");
        e.key(K::Up, 0);
        assert!((30..40).contains(&e.index()), "Up from the middle of space lands on a key of the row above, near the column it left");
        let mut f = Entry::new();
        f.open("", "", false, 0);
        f.index = 9;
        f.key(K::Down, 0);
        f.key(K::Down, 0);
        f.key(K::Down, 0);
        f.key(K::Down, 0);
        assert_eq!(f.cap(f.index()), "Done");
    }

    #[test]
    fn typing_shifts_and_deleting_end_with_done_giving_the_text_once_and_back_gives_nothing() {
        let mut e = Entry::new();
        e.open("TV name", "Den", false, 0);
        type_keys(&mut e, &["\u{232b}", "\u{21e7}", "m", "o"]);
        e.key(K::Digit(5), 0);
        let out = e.click(42, 0);
        assert!(out.close);
        assert_eq!(out.typed, Some(Typed::Entered("DeMo5".into())), "the delete took the n, the shift made one capital and the number key typed its digit");
        let mut b = Entry::new();
        b.open("TV name", "Den", false, 0);
        type_keys(&mut b, &["x"]);
        let out = b.key(K::Back, 0);
        assert!(out.close && out.typed == Some(Typed::Cancelled));
    }

    #[test]
    fn a_password_shows_bullets_but_the_last_character_and_the_text_is_not_in_what_is_printed() {
        let mut e = Entry::new();
        e.open("Wi-Fi password", "", true, 0);
        type_keys(&mut e, &["a", "b", "c"]);
        assert_eq!(e.view(false).shown, "\u{2022}\u{2022}c");
        let out = e.click(42, 0);
        assert!(!format!("{out:?}").contains("abc"), "{out:?}");
    }

    #[test]
    fn a_field_takes_at_most_128_characters_and_the_control_keys_still_act() {
        let mut e = Entry::new();
        e.open("Name", &"x".repeat(200), false, 0);
        assert_eq!(e.view(false).chars, MAX_CHARS, "what was given is cut to the limit");
        type_keys(&mut e, &["a"]);
        assert_eq!(e.view(false).chars, MAX_CHARS, "a character at the end of the field is not typed");
        type_keys(&mut e, &["\u{232b}"]);
        assert_eq!(e.view(false).chars, MAX_CHARS - 1);
        type_keys(&mut e, &["a"]);
        assert_eq!(e.view(false).chars, MAX_CHARS);
    }
}
