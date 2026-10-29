//! Search and the on-screen keyboard (`panels/Search.qml`'s logic; `docs/mocks/09-search-keyboard`): a sheet over Home with the text
//! so far, up to four answers from what is on the set (apps, Settings pages, a web search) and a keyboard the D-pad types on. Nothing
//! leaves the TV: the answers are matched here. The keyboard is the plain QWERTY of the mock: the number row, three letter rows, shift
//! and backspace, `?123`, space and Done. Two uses: search (OK on an answer picks it) and entry (a name is typed; Done says it).
//!
//! The keyboard hook: a text typed anywhere else (the compositor's `ime` topic, the Panels' keyboard, voice) lands in the field through
//! [`Search::hook_text`], which the model takes like keys pressed here, so another keyboard can type into this sheet without owning it.

use crate::kit::focus::{Dir, Focus, Id, Moved, Region};
use serde_json::{Value, json};
use ui::geometry::Frame;

pub const KEY_ID: u32 = 600;
pub const ANSWER_ID: u32 = 700;
pub const KEYS: Region = Region(7);
pub const ANSWERS: Region = Region(11);
pub const SHEET: Frame = Frame::new(64.0, 1080.0 - 56.0 - 704.0, 1792.0, 704.0);
pub const KEYS_X: f32 = SHEET.x + ((1792.0 - 930.0) / 2.0);
pub const KEYS_Y: f32 = SHEET.y + 272.0;
pub const N_KEYS: usize = 43;

const LETTERS: [[&str; 10]; 3] = [
    ["q", "w", "e", "r", "t", "y", "u", "i", "o", "p"],
    ["a", "s", "d", "f", "g", "h", "j", "k", "l", "'"],
    ["\u{21e7}", "z", "x", "c", "v", "b", "n", "m", ".", "\u{232b}"],
];
const MARKS: [[&str; 10]; 3] = [
    ["@", "#", "$", "%", "&", "*", "-", "+", "(", ")"],
    ["~", "`", "^", "[", "]", "{", "}", "<", ">", "="],
    ["\u{21e7}", "!", "?", "/", ":", ";", "\"", "_", ",", "\u{232b}"],
];
const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
/// The bottom row: the key, its left edge and width in the keyboard.
const LAST_ROW: [(&str, f32, f32); 3] = [("?123", 0.0, 178.0), ("space", 188.0, 554.0), ("Done", 752.0, 178.0)];

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub kind: &'static str,
    pub id: String,
    pub title: String,
    pub sub: String,
    pub icon: String,
    pub icon_file: String,
    pub match_len: usize,
    pub match_at: Option<usize>,
}

pub enum SearchOut {
    Nothing,
    Close,
    Picked { kind: String, id: String, text: String },
    Entered(String),
}

#[derive(Default)]
pub struct Search {
    pub text: String,
    pub entry: bool,
    pub prompt: String,
    pub voice: bool,
    pub secret: bool,
    pub index: usize,
    pub shift: bool,
    pub symbols: bool,
    apps: Vec<Value>,
    pages: Vec<Value>,
    caret_until: u64,
}

fn norm(s: &str) -> String {
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where `text` starts inside `title`: at the title's start or at the start of one of its words.
pub fn at(title: &str, text: &str) -> Option<usize> {
    let (t, q) = (norm(title), norm(text));
    if q.is_empty() {
        return None;
    }
    let mut from = 0;
    while let Some(i) = t[from..].find(&q).map(|i| i + from) {
        if i == 0 || !t[..i].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric()) {
            return Some(i);
        }
        from = i + 1;
    }
    None
}

/// The answers to a typed text: up to two apps, then Settings pages, then the web, four at most. Empty text has none.
pub fn answers(text: &str, apps: &[Value], pages: &[Value]) -> Vec<Answer> {
    let q = norm(text);
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    for a in apps {
        if out.len() >= 2 {
            break;
        }
        if let Some(i) = at(&s(a, "title"), &q) {
            out.push(Answer { kind: "app", id: s(a, "id"), title: s(a, "title"), sub: "App \u{b7} open".into(), icon: String::new(), icon_file: s(a, "iconUrl"), match_len: q.len(), match_at: Some(i) });
        }
    }
    let mut p = 0;
    for pg in pages {
        if out.len() >= 3 || p >= 2 {
            break;
        }
        if let Some(i) = at(&s(pg, "title"), &q) {
            let sub = s(pg, "sub");
            let icon = s(pg, "icon");
            out.push(Answer {
                kind: "page",
                id: s(pg, "id"),
                title: s(pg, "title"),
                sub: if sub.is_empty() { "Settings".into() } else { sub },
                icon: if icon.is_empty() { "settings".into() } else { icon },
                icon_file: String::new(),
                match_len: q.len(),
                match_at: Some(i),
            });
            p += 1;
        }
    }
    out.push(Answer { kind: "web", id: "web".into(), title: format!("Web search \u{201c}{}\u{201d}", text.trim()), sub: "Opens the browser".into(), icon: "globe".into(), icon_file: String::new(), match_len: 0, match_at: None });
    out.truncate(4);
    out
}

impl Search {
    pub fn open(&mut self, apps: Vec<Value>, voice: bool, now: u64) {
        *self = Self { apps, voice, index: 14, caret_until: now + 500, ..Self::default() };
    }

    /// A name is typed: `prompt` says whose.
    pub fn open_entry(&mut self, prompt: &str, text: &str, now: u64) {
        *self = Self { entry: true, prompt: prompt.to_owned(), text: text.to_owned(), index: 14, caret_until: now + 500, ..Self::default() };
    }

    pub fn answers(&self) -> Vec<Answer> {
        if self.entry { Vec::new() } else { answers(&self.text, &self.apps, &self.pages) }
    }

    /// The field's text as shown: bullets for a password, the last character apart so a slip is seen.
    pub fn shown(&self) -> String {
        if !self.secret || self.text.is_empty() {
            return self.text.clone();
        }
        let n = self.text.chars().count();
        let mut s: String = "\u{2022}".repeat(n - 1);
        s.extend(self.text.chars().last());
        s
    }

    pub fn key_rows(&self) -> [[&'static str; 10]; 4] {
        let letters = if self.symbols { MARKS } else { LETTERS };
        [DIGITS, letters[0], letters[1], letters[2]]
    }

    /// The key at `i` of the 43, as its cap reads.
    pub fn key_at(&self, i: usize) -> &'static str {
        if i < 40 { self.key_rows()[i / 10][i % 10] } else { LAST_ROW[i - 40].0 }
    }

    /// Where key `i` sits on the screen.
    pub fn key_frame(i: usize) -> Frame {
        if i < 40 {
            Frame::new(KEYS_X + (i % 10) as f32 * 94.0, KEYS_Y + (i / 10) as f32 * 82.0, 84.0, 72.0)
        } else {
            let (_, x, w) = LAST_ROW[i - 40];
            Frame::new(KEYS_X + x, KEYS_Y + 4.0 * 82.0, w, 72.0)
        }
    }

    /// Where answer `i` sits: four across under the field.
    pub fn answer_frame(i: usize) -> Frame {
        let w = ((SHEET.w - 80.0 - 3.0 * 16.0) / 4.0).floor();
        Frame::new(SHEET.x + 40.0 + i as f32 * (w + 16.0), SHEET.y + 134.0, w, 108.0)
    }

    pub fn field_frame() -> Frame {
        Frame::new(SHEET.x + 40.0, SHEET.y + 28.0, SHEET.w - 80.0, 84.0)
    }

    fn type_char(&mut self, c: &str) {
        if self.shift {
            self.text.push_str(&c.to_uppercase());
            self.shift = false;
        } else {
            self.text.push_str(c);
        }
    }

    /// A key of the keyboard pressed, by its cap. Returns what it ended with.
    pub fn press(&mut self, k: &str) -> SearchOut {
        match k {
            "\u{232b}" => {
                self.text.pop();
            }
            "\u{21e7}" => self.shift = !self.shift,
            "?123" => self.symbols = !self.symbols,
            "space" => self.text.push(' '),
            "Done" => return if self.entry { SearchOut::Entered(self.text.clone()) } else { SearchOut::Close },
            other => self.type_char(other),
        }
        SearchOut::Nothing
    }

    /// Text from another keyboard (the hook): replaces the field's text; a trailing newline is Done.
    pub fn hook_text(&mut self, text: &str) -> SearchOut {
        if let Some(t) = text.strip_suffix('\n') {
            self.text = t.to_owned();
            return self.press("Done");
        }
        self.text = text.to_owned();
        SearchOut::Nothing
    }

    pub fn register(&self, focus: &mut Focus) {
        let scope = super::model::SCOPE_SEARCH;
        for i in 0..N_KEYS {
            focus.register(Id(KEY_ID + i as u32), Self::key_frame(i), KEYS, scope);
        }
        let n = self.answers().len();
        for i in 0..4u32 {
            let id = Id(ANSWER_ID + i);
            if (i as usize) < n {
                focus.register(id, Self::answer_frame(i as usize), ANSWERS, scope);
            } else {
                focus.unregister(id);
            }
        }
        focus.focus(if self.index < N_KEYS { Id(KEY_ID + self.index as u32) } else { Id(ANSWER_ID + (self.index - N_KEYS) as u32) });
    }

    pub fn unregister(&self, focus: &mut Focus) {
        for i in 0..N_KEYS as u32 {
            focus.unregister(Id(KEY_ID + i));
        }
        for i in 0..4 {
            focus.unregister(Id(ANSWER_ID + i));
        }
    }

    fn take(&mut self, id: Id) {
        if let Some(i) = id.0.checked_sub(KEY_ID).filter(|i| (*i as usize) < N_KEYS) {
            self.index = i as usize;
        } else if let Some(i) = id.0.checked_sub(ANSWER_ID).filter(|i| *i < 4) {
            self.index = N_KEYS + i as usize;
        }
    }

    pub fn key(&mut self, name: &str, dir: Option<Dir>, focus: &mut Focus, now: u64) -> SearchOut {
        self.caret_until = now + 500;
        if name == "back" {
            return SearchOut::Close;
        }
        if name == "ok" {
            return self.activate(focus);
        }
        if let Some(d) = dir
            && let Moved::To(_, to) = focus.step(d)
        {
            self.take(to);
        }
        SearchOut::Nothing
    }

    /// OK, or a click, on what is focused.
    pub fn activate(&mut self, focus: &mut Focus) -> SearchOut {
        if self.index < N_KEYS {
            let k = self.key_at(self.index);
            let out = self.press(k);
            self.register(focus);
            return out;
        }
        let i = self.index - N_KEYS;
        match self.answers().get(i) {
            Some(a) => SearchOut::Picked { kind: a.kind.to_owned(), id: a.id.clone(), text: self.text.clone() },
            None => SearchOut::Nothing,
        }
    }

    pub fn clicked(&mut self, id: Id, focus: &mut Focus) -> SearchOut {
        self.take(id);
        self.activate(focus)
    }

    /// The caret blinks: on for half a second after a key, then every half second.
    pub fn caret_on(&self, now: u64) -> bool {
        now < self.caret_until || (now / 500) % 2 == 0
    }

    pub fn tick(&mut self, _now: u64) -> bool {
        false
    }

    pub fn due_at(&self) -> Option<u64> {
        None
    }

    pub fn state(&self) -> Value {
        json!({"text": self.text, "entry": self.entry, "index": self.index, "shift": self.shift, "symbols": self.symbols, "answers": self.answers().iter().map(|a| json!({"kind": a.kind, "id": a.id, "title": a.title})).collect::<Vec<_>>()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answers_are_apps_then_pages_then_the_web_four_at_most() {
        let apps = vec![json!({"id": "netflix", "title": "Netflix"}), json!({"id": "net2", "title": "LG Network"}), json!({"id": "net3", "title": "Internet"})];
        let pages = vec![json!({"id": "network", "title": "Network", "sub": "Wi-Fi"})];
        let a = answers("net", &apps, &pages);
        assert_eq!(a.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["netflix", "net2", "network", "web"]);
        assert_eq!(a[0].match_at, Some(0));
        assert_eq!(a[1].match_at, Some(3), "a word's start");
        assert!(answers("", &apps, &pages).is_empty());
        assert_eq!(at("Internet", "net"), None, "inside a word is no match");
    }

    #[test]
    fn the_keyboard_types_shifts_and_done_ends() {
        let mut s = Search::default();
        s.open(vec![json!({"id": "netflix", "title": "Netflix"})], false, 0);
        let mut f = Focus::new();
        s.register(&mut f);
        assert_eq!(s.key_at(14), "t");
        s.press("\u{21e7}");
        s.press("n");
        s.press("e");
        assert_eq!(s.text, "Ne");
        s.press("?123");
        assert_eq!(s.key_at(11), "#");
        s.press("\u{232b}");
        assert_eq!(s.text, "N");
        assert_eq!(s.answers()[0].id, "netflix");
        s.index = N_KEYS;
        assert!(matches!(s.activate(&mut f), SearchOut::Picked { kind, id, .. } if kind == "app" && id == "netflix"));
        s.open_entry("Rename Netflix", "Netflix", 0);
        assert!(matches!(s.hook_text("Films\n"), SearchOut::Entered(t) if t == "Films"));
    }
}
