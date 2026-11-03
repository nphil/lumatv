//! The on-screen keyboard of the apps' text fields (`panels/Keyboard.qml` and `KeyboardLayouts.js` of the QML shell, the kind `keyboard`).
//! The compositor holds the text protocol (`server/ime.rs`): it tells the shell which field is asking (the `ime` topic: `{active, show, model,
//! hint, purpose, enter, max, text}`, where `text` says only whether the field has any), hands over the remote's keys as `imekey` events
//! (`{k, d, r}`) while the keyboard is on the screen, and takes what is typed back as `POST /v1/ime` (`text`, `key`, `clear`, `hide`, `shown`),
//! which leaves as the field's `commit_string` and `keysym`. Nothing typed is ever drawn here and nothing of it is kept or logged: a key's
//! text goes out and is gone, the same for a password and a search.
//!
//! What a field gets is [`Layout`]'s: the letters, the e-mail and URL letters, the symbols or a number pad, with the field's enter key as the
//! one accent key. A field says what it is with three numbers of the text protocol (compositor/protocol/webos/text.xml): a purpose, hint
//! bits and an enter key type. The purpose chooses the layout; the hint bits and the enter type change the keys of it.
//!
//! The remote: the arrows move over the keys, OK presses one, a digit types itself, Back puts the keyboard away (or ends the microphone,
//! which Back ends first). The press acts: a keyboard's key is its press, and the release was taken by the compositor too, so none reaches
//! the app. The remote's repeats go through only for the keys that repeat and at the keyboard's own pace ([`Gate`]).

use super::life::Life;
use crate::kit::screen::Effect;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

// ---- what a field says

pub const PURPOSE_NORMAL: u32 = 0;
pub const PURPOSE_DIGITS: u32 = 2;
pub const PURPOSE_NUMBER: u32 = 3;
pub const PURPOSE_PHONE: u32 = 4;
pub const PURPOSE_URL: u32 = 5;
pub const PURPOSE_EMAIL: u32 = 6;
pub const PURPOSE_NAME: u32 = 7;
pub const PURPOSE_DATE: u32 = 9;
pub const PURPOSE_TIME: u32 = 10;
pub const PURPOSE_DATETIME: u32 = 11;

pub const HINT_LOWERCASE: u32 = 0x8;
pub const HINT_UPPERCASE: u32 = 0x10;
pub const HINT_TITLECASE: u32 = 0x20;

pub const ENTER_DEFAULT: u32 = 0;
pub const ENTER_DONE: u32 = 2;
pub const ENTER_GO: u32 = 3;
pub const ENTER_SEND: u32 = 4;
pub const ENTER_SEARCH: u32 = 5;
pub const ENTER_NEXT: u32 = 6;
pub const ENTER_PREVIOUS: u32 = 7;

/// The tags of the keyboard's own luna calls (the voice probe, the recognition, its cancel).
pub mod tags {
    pub const PROBE: u64 = 0x5100_0300;
    pub const VOICE: u64 = 0x5100_0301;
    pub const CANCEL: u64 = 0x5100_0302;
}

const FULL_COLS: usize = 12;
const FULL_ROWS: usize = 5;
const PAD_COLS: usize = 4;
const PAD_ROWS: usize = 4;

/// How long the microphone listens before the keyboard gives up on it.
pub const VOICE_LIMIT_MS: u64 = 20_000;
/// How long a word on the space bar ("Nothing heard", "Voice isn't available") stays.
pub const NOTE_MS: u64 = 3_000;
/// How long the focus stands still before it counts as rested (the lift of the key waits for it).
pub const REST_MS: u64 = 150;

/// The field the compositor says is asking, as the `ime` topic gives it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Field {
    pub active: bool,
    pub show: bool,
    pub model: u64,
    pub purpose: u32,
    pub hint: u32,
    pub enter: u32,
    pub has_text: bool,
}

impl Field {
    pub fn from_json(v: &Value) -> Self {
        let num = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        Self {
            active: v.get("active").and_then(Value::as_bool) == Some(true),
            show: v.get("show").and_then(Value::as_bool) == Some(true),
            model: num("model"),
            purpose: num("purpose") as u32,
            hint: num("hint") as u32,
            enter: num("enter") as u32,
            has_text: v.get("text").and_then(Value::as_bool) == Some(true),
        }
    }

    /// The keyboard is wanted on the screen: a field is active and asks for it.
    pub fn wanted(&self) -> bool {
        self.active && self.show
    }
}

// ---- the layouts

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutId {
    Letters,
    Email,
    Url,
    Symbols,
    PadDigits,
    PadNumber,
    PadPhone,
    PadDate,
}

impl LayoutId {
    pub fn is_pad(self) -> bool {
        matches!(self, LayoutId::PadDigits | LayoutId::PadNumber | LayoutId::PadPhone | LayoutId::PadDate)
    }

    pub fn name(self) -> &'static str {
        match self {
            LayoutId::Letters => "letters",
            LayoutId::Email => "email",
            LayoutId::Url => "url",
            LayoutId::Symbols => "symbols",
            LayoutId::PadDigits => "pad-digits",
            LayoutId::PadNumber => "pad-number",
            LayoutId::PadPhone => "pad-phone",
            LayoutId::PadDate => "pad-date",
        }
    }

    /// The layout a field's purpose asks for: a number pad for digits, numbers, phone numbers, dates and times, the e-mail and URL letters
    /// for those, the plain letters for everything else (a password too: the keys are the same, only nothing is ever shown of what is typed).
    pub fn for_purpose(purpose: u32) -> LayoutId {
        match purpose {
            PURPOSE_DIGITS => LayoutId::PadDigits,
            PURPOSE_NUMBER => LayoutId::PadNumber,
            PURPOSE_PHONE => LayoutId::PadPhone,
            PURPOSE_DATE | PURPOSE_TIME | PURPOSE_DATETIME => LayoutId::PadDate,
            PURPOSE_EMAIL => LayoutId::Email,
            PURPOSE_URL => LayoutId::Url,
            _ => LayoutId::Letters,
        }
    }
}

/// Shift when the field opens: caps lock for a field that asks for uppercase, one capital for a name or a title.
pub fn initial_shift(purpose: u32, hint: u32) -> u8 {
    if hint & HINT_UPPERCASE != 0 {
        2
    } else if hint & HINT_LOWERCASE != 0 {
        0
    } else if hint & HINT_TITLECASE != 0 || purpose == PURPOSE_NAME {
        1
    } else {
        0
    }
}

/// What a key is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Types `text` (`shifted` with Shift).
    Char,
    /// Types `text` as it is (".com").
    Text,
    Space,
    /// Cycles off, one capital, caps lock.
    Shift,
    /// Shows another layout.
    Mode(LayoutId),
    Backspace,
    /// The field's action; its words are the enter type's.
    Enter,
    /// Deletes the field's text.
    Clear,
    Voice,
    /// Moves the field's caret.
    Cursor(Dir),
    /// Puts the keyboard away (where there is no voice key, so the grid has no hole).
    Hide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub kind: Kind,
    pub id: &'static str,
    pub label: String,
    pub text: String,
    pub shifted: String,
    /// Drawn recessed: everything but the characters.
    pub fn_key: bool,
    /// The one primary key.
    pub accent: bool,
    /// A picture instead of words (a Lucide name).
    pub icon: Option<&'static str>,
    /// Repeats while OK is held.
    pub repeat: bool,
    /// Does nothing while the field is empty.
    pub needs_text: bool,
    /// The keyboard goes away after it (a field that is finished).
    pub hides: bool,
    /// A letter: one capital turns back to small letters after it.
    pub letter: bool,
    pub row: usize,
    pub col: usize,
    pub span: usize,
}

impl Key {
    fn at(mut self, row: usize, col: usize, span: usize) -> Key {
        self.row = row;
        self.col = col;
        self.span = span;
        self
    }

    fn plain(kind: Kind, id: &'static str, label: &str) -> Key {
        Key {
            kind,
            id,
            label: label.into(),
            text: String::new(),
            shifted: String::new(),
            fn_key: true,
            accent: false,
            icon: None,
            repeat: false,
            needs_text: false,
            hides: false,
            letter: false,
            row: 0,
            col: 0,
            span: 1,
        }
    }

    fn with_icon(mut self, icon: &'static str) -> Key {
        self.icon = Some(icon);
        self
    }

    /// A character key: its capital is `shifted`, and `letter` when the capital is a letter's.
    fn ch(text: &str, letter: bool) -> Key {
        let up = text.to_uppercase();
        Key {
            kind: Kind::Char,
            id: "char",
            label: text.into(),
            text: text.into(),
            letter: letter && up != text,
            shifted: up,
            fn_key: false,
            ..Key::plain(Kind::Char, "char", "")
        }
    }

    fn dotcom() -> Key {
        Key { kind: Kind::Text, id: "dotcom", label: ".com".into(), text: ".com".into(), shifted: ".com".into(), ..Key::plain(Kind::Text, "dotcom", "") }
    }

    /// The centre column of the key.
    pub fn centre(&self) -> f32 {
        self.col as f32 + self.span as f32 / 2.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub id: LayoutId,
    pub pad: bool,
    pub cols: usize,
    pub rows: usize,
    /// Shift acts on it (the three letter layouts).
    pub shiftable: bool,
    pub keys: Vec<Key>,
    pub home_row: usize,
    pub home_col: usize,
}

/// The enter key: its picture or words, and whether the keyboard goes away after it (a field that is finished: Search, Go, Send, Done).
fn enter_look(enter: u32) -> (Option<&'static str>, &'static str, bool) {
    match enter {
        ENTER_DONE => (None, "Done", true),
        ENTER_GO => (None, "Go", true),
        ENTER_SEND => (None, "Send", true),
        ENTER_SEARCH => (None, "Search", true),
        ENTER_NEXT => (None, "Next", false),
        ENTER_PREVIOUS => (None, "Prev", false),
        _ => (Some("corner-down-left"), "", false),
    }
}

fn enter_key(enter: u32) -> Key {
    let (icon, text, hides) = enter_look(enter);
    let mut k = Key::plain(Kind::Enter, "enter", text);
    k.icon = icon;
    k.accent = true;
    k.hides = hides;
    k
}

fn backspace_key() -> Key {
    let mut k = Key::plain(Kind::Backspace, "backspace", "").with_icon("delete");
    k.repeat = true;
    k
}

fn clear_key() -> Key {
    let mut k = Key::plain(Kind::Clear, "clear", "Clear");
    k.needs_text = true;
    k
}

/// One run of characters on a row, from column 1.
fn run(keys: &mut Vec<Key>, row: usize, list: &[&str], letter: bool) {
    for (i, c) in list.iter().enumerate() {
        keys.push(Key::ch(c, letter).at(row, 1 + i, 1));
    }
}

const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
const Q_ROW: [&str; 10] = ["q", "w", "e", "r", "t", "y", "u", "i", "o", "p"];
const A_ROW: [&str; 9] = ["a", "s", "d", "f", "g", "h", "j", "k", "l"];
const Z_ROW: [&str; 7] = ["z", "x", "c", "v", "b", "n", "m"];
const SYM_1: [&str; 10] = ["!", "@", "#", "$", "%", "^", "&", "*", "(", ")"];
const SYM_2: [&str; 10] = ["-", "_", "=", "+", "[", "]", "{", "}", "|", "\\"];
const SYM_3: [&str; 10] = [";", ":", "'", "\"", ",", ".", "<", ">", "/", "?"];

fn full(id: LayoutId, enter: u32, voice: bool) -> Layout {
    let letters = id != LayoutId::Symbols;
    let mut keys = Vec::with_capacity(51);
    // the left column: the way to the other characters, Shift (or the first symbol), and two keys of the layout (the three letter layouts
    // differ in a few places; the grid is the same, so a hand that knows one knows the others)
    let mode = if letters { Key::plain(Kind::Mode(LayoutId::Symbols), "mode", "?123") } else { Key::plain(Kind::Mode(LayoutId::Letters), "mode", "ABC") };
    keys.push(mode.at(0, 0, 1));
    run(&mut keys, 0, &DIGITS, letters);
    keys.push(backspace_key().at(0, 11, 1));

    keys.push(if letters { Key::plain(Kind::Shift, "shift", "").with_icon("arrow-big-up") } else { Key::ch("`", false) }.at(1, 0, 1));
    if letters {
        run(&mut keys, 1, &Q_ROW, true);
    } else {
        run(&mut keys, 1, &SYM_1, false);
    }
    keys.push(enter_key(enter).at(1, 11, 1));

    let l2 = match id {
        LayoutId::Email => Key::ch("@", false),
        LayoutId::Url => Key::ch("/", false),
        LayoutId::Symbols => Key::ch("~", false),
        _ => Key::ch("'", false),
    };
    keys.push(l2.at(2, 0, 1));
    match id {
        LayoutId::Symbols => run(&mut keys, 2, &SYM_2, false),
        _ => {
            let last = match id {
                LayoutId::Email => "_",
                LayoutId::Url => ":",
                _ => "?",
            };
            let row: Vec<&str> = A_ROW.iter().copied().chain([last]).collect();
            run(&mut keys, 2, &row, true);
        }
    }
    keys.push(
        if voice { Key::plain(Kind::Voice, "voice", "").with_icon("mic") } else { Key::plain(Kind::Hide, "hide", "").with_icon("chevron-down") }.at(2, 11, 1),
    );

    let l3 = match id {
        LayoutId::Email | LayoutId::Url => Key::dotcom(),
        LayoutId::Symbols => Key::ch("\u{20ac}", false),
        _ => Key::ch("@", false),
    };
    keys.push(l3.at(3, 0, 1));
    match id {
        LayoutId::Symbols => run(&mut keys, 3, &SYM_3, false),
        _ => {
            let tail: [&str; 3] = match id {
                LayoutId::Email => [".", "-", "+"],
                LayoutId::Url => [".", "-", "_"],
                _ => [",", ".", "-"],
            };
            let row: Vec<&str> = Z_ROW.iter().copied().chain(tail).collect();
            run(&mut keys, 3, &row, true);
        }
    }
    keys.push(clear_key().at(3, 11, 1));

    let mut left = Key::plain(Kind::Cursor(Dir::Left), "left", "").with_icon("chevron-left");
    left.repeat = true;
    keys.push(left.at(4, 0, 1));
    let mut space = Key::plain(Kind::Space, "space", "");
    space.text = " ".into();
    keys.push(space.at(4, 1, 10));
    let mut right = Key::plain(Kind::Cursor(Dir::Right), "right", "").with_icon("chevron-right");
    right.repeat = true;
    keys.push(right.at(4, 11, 1));
    Layout { id, pad: false, cols: FULL_COLS, rows: FULL_ROWS, shiftable: letters, keys, home_row: 2, home_col: 5 }
}

fn pad(id: LayoutId, enter: u32) -> Layout {
    let mut keys = Vec::with_capacity(16);
    for i in 0..9 {
        keys.push(Key::ch(&(i + 1).to_string(), false).at(i / 3, i % 3, 1));
    }
    let (a, b) = match id {
        LayoutId::PadNumber => (Some("."), Some("-")),
        LayoutId::PadPhone => (Some("+"), Some("-")),
        LayoutId::PadDate => (Some("/"), Some(":")),
        _ => (None, None),
    };
    if let Some(a) = a {
        keys.push(Key::ch(a, false).at(3, 0, 1));
    }
    keys.push(Key::ch("0", false).at(3, 1, 1));
    if let Some(b) = b {
        keys.push(Key::ch(b, false).at(3, 2, 1));
    }
    keys.push(backspace_key().at(0, 3, 1));
    keys.push(enter_key(enter).at(1, 3, 1));
    keys.push(clear_key().at(2, 3, 1));
    Layout { id, pad: true, cols: PAD_COLS, rows: PAD_ROWS, shiftable: false, keys, home_row: 1, home_col: 1 }
}

impl Layout {
    /// `enter` is the field's enter key type; `voice` whether LG's voice can be asked (the voice key replaces the hide key).
    pub fn build(id: LayoutId, enter: u32, voice: bool) -> Layout {
        if id.is_pad() { pad(id, enter) } else { full(id, enter, voice) }
    }

    pub fn at(&self, row: usize, col: usize) -> Option<usize> {
        self.keys.iter().position(|k| k.row == row && col >= k.col && col < k.col + k.span)
    }

    /// The key the focus starts on when a field opens: the middle of the letters, the 5 of a pad.
    pub fn home(&self) -> usize {
        self.at(self.home_row, self.home_col).unwrap_or(0)
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.keys.iter().position(|k| k.id == id)
    }


    /// One step of the D-pad from key `from`: the key it lands on and the column the walk wants. Left and Right walk the row and wrap to its
    /// other end; Up and Down go to the key above or below at the column the walk started from (`want`, kept while only moving vertically,
    /// so a wide key such as the space bar hands the column back), wrapping top to bottom, skipping rows with no key and landing on the
    /// nearest key where the row has a hole at that column.
    pub fn step(&self, from: usize, dir: Move, want: f32) -> (usize, f32) {
        let Some(k) = self.keys.get(from) else {
            let home = self.home();
            return (home, self.keys[home].centre());
        };
        match dir {
            Move::Left | Move::Right => {
                let row = || self.keys.iter().enumerate().filter(|(_, c)| c.row == k.row);
                let n = if dir == Move::Right {
                    row().filter(|(_, c)| c.col > k.col).min_by_key(|(_, c)| c.col).or_else(|| row().min_by_key(|(_, c)| c.col))
                } else {
                    row().filter(|(_, c)| c.col < k.col).max_by_key(|(_, c)| c.col).or_else(|| row().max_by_key(|(_, c)| c.col))
                }.map_or(from, |(i, _)| i);
                (n, self.keys[n].centre())
            }
            Move::Up | Move::Down => {
                let stride = if dir == Move::Down { 1 } else { self.rows - 1 };
                for i in 1..=self.rows {
                    let r = (k.row + stride * i) % self.rows;
                    let dist = |c: &Key| {
                        let (lo, hi) = (c.col as f32, (c.col + c.span) as f32);
                        if want >= lo && want < hi { 0.0 } else { (want - lo).abs().min((want - hi).abs()) }
                    };
                    if let Some((best, _)) = self.keys.iter().enumerate().filter(|(_, c)| c.row == r).min_by(|(_, a), (_, b)| dist(a).total_cmp(&dist(b))) {
                        return (best, want);
                    }
                }
                (from, want)
            }
        }
    }

    /// What pressing key `index` does, from the keyboard's shift (0 off, 1 one capital, 2 caps lock).
    pub fn press(&self, index: usize, shift: u8) -> Press {
        let Some(k) = self.keys.get(index) else { return Press::default() };
        let mut out = Press::default();
        match k.kind {
            Kind::Char => {
                let up = self.shiftable && shift > 0;
                out.text = Some(if up { k.shifted.clone() } else { k.text.clone() });
                // one capital, then small letters again
                if up && k.letter && shift == 1 {
                    out.shift = Some(0);
                }
            }
            Kind::Text => out.text = Some(k.text.clone()),
            Kind::Space => out.text = Some(" ".into()),
            Kind::Shift => out.shift = Some((shift + 1) % 3),
            Kind::Mode(to) => out.mode = Some(to),
            Kind::Backspace => out.action = Some(Action::Backspace),
            Kind::Clear => out.action = Some(Action::Clear),
            Kind::Cursor(Dir::Left) => out.action = Some(Action::Left),
            Kind::Cursor(Dir::Right) => out.action = Some(Action::Right),
            Kind::Voice => out.voice = true,
            Kind::Hide => out.hide = true,
            Kind::Enter => {
                out.action = Some(Action::Enter);
                out.hide = k.hides;
            }
        }
        out
    }

    /// The label a key shows: the capital with Shift on, the words of the enter key.
    pub fn label(&self, index: usize, shift: u8) -> &str {
        let Some(k) = self.keys.get(index) else { return "" };
        if k.kind == Kind::Char && self.shiftable && shift > 0 { &k.shifted } else { &k.label }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Up,
    Down,
    Left,
    Right,
}

impl Move {
    fn parse(name: &str) -> Option<Move> {
        Some(match name {
            "up" => Move::Up,
            "down" => Move::Down,
            "left" => Move::Left,
            "right" => Move::Right,
            _ => return None,
        })
    }
}

/// What the field is asked to do besides taking text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Backspace,
    Enter,
    Left,
    Right,
    Clear,
}

impl Action {
    /// The `POST /v1/ime` body of it (`clear` is an op of its own, the rest are keys).
    fn body(self) -> Value {
        match self {
            Action::Clear => json!({"op": "clear"}),
            Action::Backspace => json!({"op": "key", "key": "backspace"}),
            Action::Enter => json!({"op": "key", "key": "enter"}),
            Action::Left => json!({"op": "key", "key": "left"}),
            Action::Right => json!({"op": "key", "key": "right"}),
        }
    }
}

/// What pressing a key does: text typed, an action for the field, another layout, a new shift, the microphone, the keyboard going away.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Press {
    pub text: Option<String>,
    pub action: Option<Action>,
    pub mode: Option<LayoutId>,
    pub shift: Option<u8>,
    pub voice: bool,
    pub hide: bool,
}

// ---- the remote's repeats

/// One held key of the remote arrives as a press and then a press for every repeat. The first press moves; the repeats are let through after
/// `delay` ms and then at most one in `interval` ms, one in `fast` ms after `fast_after` steps: a held arrow crosses the keyboard quickly
/// without the pace being the remote's.
#[derive(Clone, Debug, Default)]
pub struct Gate {
    key: String,
    first: u64,
    last: u64,
    steps: u32,
}

const DELAY_MS: u64 = 300;
const INTERVAL_MS: u64 = 100;
const FAST_MS: u64 = 60;
const FAST_AFTER: u32 = 8;

impl Gate {
    /// Whether to act on this press of `key` at `now`.
    pub fn pass(&mut self, key: &str, repeat: bool, now: u64) -> bool {
        if !repeat || self.key != key {
            self.key.clear();
            self.key.push_str(key);
            self.first = now;
            self.last = now;
            self.steps = 0;
            return true;
        }
        if now.saturating_sub(self.first) < DELAY_MS {
            return false;
        }
        if now.saturating_sub(self.last) < if self.steps >= FAST_AFTER { FAST_MS } else { INTERVAL_MS } {
            return false;
        }
        self.last = now;
        self.steps += 1;
        true
    }
}

// ---- the model

/// What the surface draws of the keyboard.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub layout: Rc<Layout>,
    pub shift: u8,
    pub focus: usize,
    /// The field has text: the keys that need it are not dimmed.
    pub has_text: bool,
    /// Words on the space bar: what the microphone is doing.
    pub note: String,
    pub listening: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Probe {
    Idle,
    Asked,
    Done,
}

pub struct Ime {
    pub life: Life,
    layout_cache: RefCell<Option<(LayoutId, u32, bool, Rc<Layout>)>>,
    field: Field,
    symbols: bool,
    shift: u8,
    focus: usize,
    column: f32,
    gate: Gate,
    voice_ok: bool,
    probe: Probe,
    listening: bool,
    ticket: String,
    note: String,
    voice_due: Option<u64>,
    note_due: Option<u64>,
    /// Where the sheet was last announced to the compositor (it posts again when the place changes).
    announced: Option<[i32; 4]>,
    /// A key of the remote went down since the surface last asked: the pointer's cue goes.
    key_taken: bool,
    redraw: bool,
}

impl Default for Ime {
    fn default() -> Self {
        Self {
            life: Life::new(super::life::ENTER_MS, super::life::EXIT_MS, 0),
            layout_cache: RefCell::new(None),
            field: Field::default(),
            symbols: false,
            shift: 0,
            focus: 0,
            column: 0.0,
            gate: Gate::default(),
            voice_ok: false,
            probe: Probe::Idle,
            listening: false,
            ticket: String::new(),
            note: String::new(),
            voice_due: None,
            note_due: None,
            announced: None,
            key_taken: false,
            redraw: false,
        }
    }
}

impl Ime {
    pub fn new() -> Self {
        Self::default()
    }

    /// The keyboard is wanted on the screen now.
    pub fn wanted(&self) -> bool {
        self.field.wanted()
    }

    pub fn field(&self) -> &Field {
        &self.field
    }

    fn base(&self) -> LayoutId {
        LayoutId::for_purpose(self.field.purpose)
    }

    pub fn layout_id(&self) -> LayoutId {
        if self.symbols && !self.base().is_pad() { LayoutId::Symbols } else { self.base() }
    }

    pub fn layout(&self) -> Rc<Layout> {
        let id = self.layout_id();
        let mut cache = self.layout_cache.borrow_mut();
        if cache.as_ref().is_none_or(|(was, enter, voice, _)| (*was, *enter, *voice) != (id, self.field.enter, self.voice_ok)) {
            *cache = Some((id, self.field.enter, self.voice_ok, Rc::new(Layout::build(id, self.field.enter, self.voice_ok))));
        }
        cache.as_ref().unwrap().3.clone()
    }

    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn shift(&self) -> u8 {
        self.shift
    }

    pub fn listening(&self) -> bool {
        self.listening
    }

    pub fn voice_ok(&self) -> bool {
        self.voice_ok
    }

    pub fn note(&self) -> &str {
        &self.note
    }

    pub fn view(&self) -> View {
        View { layout: self.layout(), shift: self.shift, focus: self.focus, has_text: self.field.has_text, note: self.note.clone(), listening: self.listening }
    }

    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }

    /// A key of the remote went down: the pointer's cue is to go (taken once by the surface).
    pub fn take_key_taken(&mut self) -> bool {
        std::mem::take(&mut self.key_taken)
    }

    fn enabled(&self, layout: &Layout, i: usize) -> bool {
        layout.keys.get(i).is_none_or(|k| !k.needs_text || self.field.has_text)
    }

    /// Whether key `i` of the layout takes a press now (the Clear key does not while the field is empty).
    pub fn key_enabled(&self, i: usize) -> bool {
        self.enabled(&self.layout(), i)
    }

    fn post(&self, mut body: Value) -> Effect {
        body["model"] = json!(self.field.model);
        Effect::Channel { path: "/v1/ime".into(), body }
    }

    /// A field opens: the first layout, the first focus.
    fn begin(&mut self) {
        self.symbols = false;
        self.shift = initial_shift(self.field.purpose, self.field.hint);
        let layout = self.layout();
        self.focus = layout.home();
        self.column = layout.keys[self.focus].centre();
        self.gate = Gate::default();
        self.announced = None;
    }

    /// The subscription of the voice probe: asked once at the start.
    pub fn start(&mut self) -> Vec<Effect> {
        self.ask_probe()
    }

    fn ask_probe(&mut self) -> Vec<Effect> {
        if self.probe != Probe::Idle {
            return Vec::new();
        }
        self.probe = Probe::Asked;
        vec![Effect::Luna {
            tag: tags::PROBE,
            uri: "luna://com.webos.service.config/getConfigs".into(),
            payload: json!({"configNames": ["system.supportVoiceRecognition"]}),
            subscribe: false,
        }]
    }

    /// The `ime` topic: the field that is asking, and whether it wants the keyboard.
    pub fn field_is(&mut self, data: &Value, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        let was = std::mem::replace(&mut self.field, Field::from_json(data));
        let now_wanted = self.field.wanted();
        if now_wanted && (!was.wanted() || was.model != self.field.model || LayoutId::for_purpose(was.purpose) != self.base()) {
            self.begin();
            // a probe that got no answer is asked again when a field wants the keyboard
            out.extend(self.ask_probe());
        }
        if now_wanted {
            if !self.life.presented() {
                self.life.show(now);
            }
        } else {
            if self.life.presented() {
                self.life.hide(now);
            }
            self.announced = None;
            // the keyboard going away ends a listening that is still open
            if self.listening {
                out.extend(self.voice_stop(None, now));
            }
        }
        self.redraw = true;
        out
    }

    // ---- what a key does

    fn set_focus(&mut self, i: usize) {
        let layout = self.layout();
        if i >= layout.keys.len() {
            return;
        }
        self.focus = i;
        self.column = layout.keys[i].centre();
    }

    fn walk(&mut self, dir: Move) {
        let layout = self.layout();
        let (i, want) = layout.step(self.focus, dir, self.column);
        self.focus = i;
        self.column = want;
    }

    /// `i` pressed: the layout says what happens, this carries it out.
    fn activate(&mut self, i: usize, now: u64) -> Vec<Effect> {
        let layout = self.layout();
        let mut out = Vec::new();
        if i >= layout.keys.len() || !self.enabled(&layout, i) {
            return out;
        }
        let e = layout.press(i, self.shift);
        if let Some(to) = e.mode {
            let was = &layout.keys[i];
            self.symbols = to == LayoutId::Symbols;
            let next = self.layout();
            let j = next.at(was.row, was.col).unwrap_or_else(|| next.home());
            self.set_focus(j);
            self.redraw = true;
            return out;
        }
        if let Some(s) = e.shift {
            self.shift = s;
        }
        if let Some(t) = e.text {
            out.push(self.post(json!({"op": "text", "text": t})));
        }
        if let Some(a) = e.action {
            out.push(self.post(a.body()));
        }
        if e.voice {
            if self.listening {
                out.extend(self.voice_stop(None, now));
            } else {
                out.extend(self.voice_start(now));
            }
        }
        if e.hide {
            out.push(self.post(json!({"op": "hide"})));
        }
        self.redraw = true;
        out
    }

    /// Back: the microphone first, then the keyboard.
    fn back(&mut self, now: u64) -> Vec<Effect> {
        if self.listening { self.voice_stop(None, now) } else { vec![self.post(json!({"op": "hide"}))] }
    }

    /// One key of the compositor's `imekey` events: the arrows move, OK presses the key, a digit types itself, Back puts the keyboard away.
    pub fn imekey(&mut self, name: &str, down: bool, repeat: bool, now: u64) -> Vec<Effect> {
        if down {
            self.key_taken = true;
        }
        if !self.wanted() {
            return Vec::new();
        }
        self.redraw = true;
        if name.len() == 1 && name.as_bytes()[0].is_ascii_digit() {
            if down && !repeat {
                return vec![self.post(json!({"op": "text", "text": name}))];
            }
            return Vec::new();
        }
        if !down {
            return Vec::new();
        }
        if name == "back" {
            return if repeat { Vec::new() } else { self.back(now) };
        }
        if repeat && !self.repeats(name) {
            return Vec::new();
        }
        if !self.gate.pass(name, repeat, now) {
            return Vec::new();
        }
        if name == "ok" {
            self.activate(self.focus, now)
        } else {
            if let Some(m) = Move::parse(name) {
                self.walk(m);
            }
            Vec::new()
        }
    }

    /// Which keys of the remote repeat at all: the arrows, and OK on a key that is held to be repeated (Backspace, the cursor keys).
    fn repeats(&self, name: &str) -> bool {
        match name {
            "up" | "down" | "left" | "right" => true,
            "ok" => self.layout().keys.get(self.focus).is_some_and(|k| k.repeat),
            _ => false,
        }
    }

    // ---- the pointer (Magic Remote)

    /// A press on key `i` takes the focus.
    pub fn pointer_down(&mut self, i: usize) {
        self.set_focus(i);
        self.redraw = true;
    }

    /// A press let go on the key it began on types it.
    pub fn click(&mut self, i: usize, now: u64) -> Vec<Effect> {
        self.set_focus(i);
        self.activate(i, now)
    }

    // ---- where the sheet is

    /// The surface says where the sheet is on the screen (it is on its way in): the compositor is told once for each place.
    pub fn announce(&mut self, x: i32, y: i32, w: i32, h: i32) -> Option<Effect> {
        if !self.wanted() || self.announced == Some([x, y, w, h]) {
            return None;
        }
        self.announced = Some([x, y, w, h]);
        Some(self.post(json!({"op": "shown", "x": x, "y": y, "w": w, "h": h})))
    }

    // ---- the microphone: LG's voice service does the listening (the call maliit's own dictation card makes, `recognizeVoiceWithDetails` with
    // `runVoiceUi: false`), the keyboard says what it is doing on its space bar and the first answer is typed into the field. The key is there
    // only when the TV says it has voice recognition; a TV whose voice services are stopped answers with an error and the space bar says so.

    fn voice_start(&mut self, now: u64) -> Vec<Effect> {
        if self.listening {
            return Vec::new();
        }
        self.listening = true;
        self.ticket.clear();
        self.note = "Listening".into();
        self.note_due = None;
        self.voice_due = Some(now + VOICE_LIMIT_MS);
        vec![Effect::Luna {
            tag: tags::VOICE,
            uri: "luna://com.webos.service.voiceconductor/system/recognizeVoiceWithDetails".into(),
            payload: json!({"subscribe": true, "language": "en-US", "runVoiceUi": false}),
            subscribe: true,
        }]
    }

    fn voice_stop(&mut self, note: Option<&str>, now: u64) -> Vec<Effect> {
        let mut out = vec![Effect::Cancel { tag: tags::VOICE }];
        if !self.ticket.is_empty() {
            out.push(Effect::Luna {
                tag: tags::CANCEL,
                uri: "luna://com.webos.service.voiceconductor/cancelRecognition".into(),
                payload: json!({"voiceTicket": self.ticket}),
                subscribe: false,
            });
        }
        self.voice_due = None;
        self.ticket.clear();
        self.listening = false;
        self.note = note.unwrap_or("").to_string();
        self.note_due = note.map(|_| now + NOTE_MS);
        self.redraw = true;
        out
    }

    /// The answer of a luna call of the keyboard's: the probe, and the recognition's events.
    pub fn luna(&mut self, tag: u64, p: &Value, now: u64) -> Vec<Effect> {
        match tag {
            tags::PROBE => {
                let ok = p.pointer("/configs/system.supportVoiceRecognition").and_then(Value::as_bool);
                match ok {
                    Some(v) => {
                        self.probe = Probe::Done;
                        if v != self.voice_ok {
                            self.voice_ok = v;
                            self.redraw = true;
                        }
                    }
                    // no answer worth having (the service was not up): asked again at the next field
                    None => self.probe = Probe::Idle,
                }
                Vec::new()
            }
            tags::VOICE if self.listening => {
                if p.get("returnValue").and_then(Value::as_bool) == Some(false) || p.get("event").and_then(Value::as_str) == Some("sttError") {
                    return self.voice_stop(Some("Voice isn't available"), now);
                }
                match p.get("event").and_then(Value::as_str) {
                    Some("sttStart") => {
                        self.ticket = p.get("voiceTicket").map(|t| t.as_str().map_or_else(|| t.to_string(), str::to_string)).unwrap_or_default();
                        Vec::new()
                    }
                    Some("sttConnecting") => {
                        self.note = "Working".into();
                        self.redraw = true;
                        Vec::new()
                    }
                    Some("sttResult") => {
                        let words = p.pointer("/text/0").and_then(Value::as_str).unwrap_or("").to_string();
                        let mut out = self.voice_stop(if words.is_empty() { Some("Nothing heard") } else { None }, now);
                        if !words.is_empty() {
                            out.push(self.post(json!({"op": "text", "text": words})));
                        }
                        out
                    }
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    // ---- time

    pub fn tick(&mut self, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Some(super::life::Change::Opened | super::life::Change::Parked | super::life::Change::Drop) = self.life.tick(now) {
            self.redraw = true;
        }
        if self.voice_due.is_some_and(|d| now >= d) {
            out.extend(self.voice_stop(Some("Voice isn't available"), now));
        }
        if self.note_due.is_some_and(|d| now >= d) {
            self.note_due = None;
            if !self.listening {
                self.note.clear();
                self.redraw = true;
            }
        }
        out
    }

    pub fn due_at(&self) -> Option<u64> {
        [self.life.due_at(), self.voice_due, self.note_due].into_iter().flatten().min()
    }

    /// For the harness and the gate: what the keyboard is doing.
    pub fn state(&self) -> Value {
        let layout = self.layout();
        json!({
            "up": self.wanted(), "live": self.life.visible(), "layout": layout.id.name(), "focus": self.focus,
            "key": layout.keys.get(self.focus).map_or("", |k| k.id), "shift": self.shift, "listening": self.listening, "voice": self.voice_ok,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(purpose: u32, hint: u32, enter: u32, text: bool) -> Value {
        json!({"active": true, "show": true, "model": 7, "purpose": purpose, "hint": hint, "enter": enter, "max": 0, "text": text})
    }

    fn up(purpose: u32, hint: u32, enter: u32) -> Ime {
        let mut k = Ime::new();
        k.field_is(&field(purpose, hint, enter, true), 0);
        k
    }

    fn posts(fx: &[Effect]) -> Vec<Value> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Channel { path, body } if path == "/v1/ime" => Some(body.clone()),
                _ => None,
            })
            .collect()
    }

    fn index(k: &Ime, id: &str) -> usize {
        k.layout().index_of(id).unwrap_or_else(|| panic!("no key {id}"))
    }

    fn char_at(k: &Ime, text: &str) -> usize {
        k.layout().keys.iter().position(|c| c.kind == Kind::Char && c.text == text).unwrap_or_else(|| panic!("no key {text}"))
    }

    #[test]
    fn a_fields_purpose_chooses_the_layout_and_its_hints_the_first_shift() {
        assert_eq!(LayoutId::for_purpose(PURPOSE_NORMAL), LayoutId::Letters);
        assert_eq!(LayoutId::for_purpose(1), LayoutId::Letters, "alpha");
        assert_eq!(LayoutId::for_purpose(8), LayoutId::Letters, "a password too");
        assert_eq!(LayoutId::for_purpose(PURPOSE_DIGITS), LayoutId::PadDigits);
        assert_eq!(LayoutId::for_purpose(PURPOSE_NUMBER), LayoutId::PadNumber);
        assert_eq!(LayoutId::for_purpose(PURPOSE_PHONE), LayoutId::PadPhone);
        for p in [PURPOSE_DATE, PURPOSE_TIME, PURPOSE_DATETIME] {
            assert_eq!(LayoutId::for_purpose(p), LayoutId::PadDate);
        }
        assert_eq!(LayoutId::for_purpose(PURPOSE_EMAIL), LayoutId::Email);
        assert_eq!(LayoutId::for_purpose(PURPOSE_URL), LayoutId::Url);
        assert_eq!(initial_shift(PURPOSE_NORMAL, 0), 0);
        assert_eq!(initial_shift(PURPOSE_NAME, 0), 1, "a name starts with a capital");
        assert_eq!(initial_shift(PURPOSE_NORMAL, HINT_TITLECASE), 1);
        assert_eq!(initial_shift(PURPOSE_NORMAL, HINT_UPPERCASE), 2, "caps lock");
        assert_eq!(initial_shift(PURPOSE_NAME, HINT_LOWERCASE), 0, "a field that asks for lowercase wins over a name");
    }

    #[test]
    fn the_letters_are_a_grid_of_twelve_columns_and_five_rows_with_51_keys() {
        let l = Layout::build(LayoutId::Letters, ENTER_DEFAULT, false);
        assert_eq!((l.cols, l.rows, l.keys.len(), l.pad, l.shiftable), (12, 5, 51, false, true));
        let row = |r: usize| -> String {
            l.keys.iter().filter(|k| k.row == r).map(|k| if k.kind == Kind::Char { k.text.clone() } else { format!("[{}]", k.id) }).collect::<Vec<_>>().join("")
        };
        assert_eq!(row(0), "[mode]1234567890[backspace]");
        assert_eq!(row(1), "[shift]qwertyuiop[enter]");
        assert_eq!(row(2), "'asdfghjkl?[hide]");
        assert_eq!(row(3), "@zxcvbnm,.-[clear]");
        assert_eq!(row(4), "[left][space][right]");
        let space = &l.keys[index_in(&l, "space")];
        assert_eq!((space.col, space.span), (1, 10));
        assert_eq!(l.keys[l.home()].text, "g", "the focus starts in the middle of the letters");
    }

    fn index_in(l: &Layout, id: &str) -> usize {
        l.index_of(id).unwrap()
    }

    #[test]
    fn e_mail_and_url_letters_change_a_few_keys_and_the_symbols_replace_the_letters() {
        let row = |id: LayoutId, r: usize| -> String {
            Layout::build(id, 0, false)
                .keys
                .iter()
                .filter(|k| k.row == r)
                .map(|k| if matches!(k.kind, Kind::Char | Kind::Text) { k.text.clone() } else { format!("[{}]", k.id) })
                .collect::<Vec<_>>()
                .join("")
        };
        assert_eq!(row(LayoutId::Email, 2), "@asdfghjkl_[hide]");
        assert_eq!(row(LayoutId::Email, 3), ".comzxcvbnm.-+[clear]");
        assert_eq!(row(LayoutId::Url, 2), "/asdfghjkl:[hide]");
        assert_eq!(row(LayoutId::Url, 3), ".comzxcvbnm.-_[clear]");
        assert_eq!(row(LayoutId::Symbols, 0), "[mode]1234567890[backspace]");
        assert_eq!(row(LayoutId::Symbols, 1), "`!@#$%^&*()[enter]");
        assert_eq!(row(LayoutId::Symbols, 2), "~-_=+[]{}|\\[hide]");
        assert_eq!(row(LayoutId::Symbols, 3), "\u{20ac};:'\",.<>/?[clear]");
        let sym = Layout::build(LayoutId::Symbols, 0, false);
        assert!(!sym.shiftable && sym.index_of("shift").is_none());
        assert_eq!(sym.keys[sym.index_of("mode").unwrap()].label, "ABC");
        assert_eq!(Layout::build(LayoutId::Letters, 0, false).keys[0].label, "?123");
    }

    #[test]
    fn a_number_pad_is_four_columns_with_the_corners_of_its_purpose() {
        let l = Layout::build(LayoutId::PadDigits, ENTER_DEFAULT, false);
        assert_eq!((l.cols, l.rows, l.pad, l.shiftable), (4, 4, true, false));
        assert_eq!(l.keys.len(), 13, "digits: 1 to 9, 0, delete, enter, clear");
        let text =
            |id: LayoutId| -> Vec<String> { Layout::build(id, 0, false).keys.iter().filter(|k| k.row == 3 && k.col < 3).map(|k| k.text.clone()).collect() };
        assert_eq!(text(LayoutId::PadDigits), vec!["0"]);
        assert_eq!(text(LayoutId::PadNumber), vec![".", "0", "-"]);
        assert_eq!(text(LayoutId::PadPhone), vec!["+", "0", "-"]);
        assert_eq!(text(LayoutId::PadDate), vec!["/", "0", ":"]);
        assert_eq!(l.keys[l.home()].text, "5");
        assert_eq!(Layout::build(LayoutId::PadDate, 0, true).keys.iter().filter(|k| k.kind == Kind::Voice).count(), 0, "a pad has no voice key");
    }

    #[test]
    fn the_enter_key_is_the_one_accent_key_with_the_enter_types_words_and_hides_the_keyboard_when_the_field_is_finished() {
        let look = |enter: u32| -> (Option<&'static str>, String, bool) {
            let l = Layout::build(LayoutId::Letters, enter, false);
            let k = &l.keys[l.index_of("enter").unwrap()];
            assert!(k.accent && l.keys.iter().filter(|k| k.accent).count() == 1);
            (k.icon, k.label.clone(), k.hides)
        };
        assert_eq!(look(ENTER_DEFAULT), (Some("corner-down-left"), String::new(), false));
        assert_eq!(look(1), (Some("corner-down-left"), String::new(), false), "return");
        assert_eq!(look(ENTER_DONE), (None, "Done".into(), true));
        assert_eq!(look(ENTER_GO), (None, "Go".into(), true));
        assert_eq!(look(ENTER_SEND), (None, "Send".into(), true));
        assert_eq!(look(ENTER_SEARCH), (None, "Search".into(), true));
        assert_eq!(look(ENTER_NEXT), (None, "Next".into(), false));
        assert_eq!(look(ENTER_PREVIOUS), (None, "Prev".into(), false));
    }

    #[test]
    fn the_voice_key_stands_where_the_hide_key_does_only_when_the_tv_can_listen() {
        let at = |voice: bool| Layout::build(LayoutId::Letters, 0, voice).keys.iter().find(|k| k.row == 2 && k.col == 11).map(|k| k.id).unwrap();
        assert_eq!(at(false), "hide");
        assert_eq!(at(true), "voice");
    }

    #[test]
    fn left_and_right_walk_the_row_and_wrap_up_and_down_keep_the_column_and_the_space_bar_hands_it_back() {
        let l = Layout::build(LayoutId::Letters, 0, false);
        let from = l.index_of("mode").unwrap();
        let (to, _) = l.step(from, Move::Left, 0.5);
        assert_eq!(l.keys[to].id, "backspace", "the row wraps");
        let (back, _) = l.step(to, Move::Right, 11.5);
        assert_eq!(back, from);
        // down from `g` (column 5.5) to the b under it, the space bar that holds the column, and round the bottom to the 5 of the top row
        let h = l.home();
        let (a, want) = l.step(h, Move::Down, l.keys[h].centre());
        assert_eq!(l.keys[a].text, "b");
        let (space, want) = l.step(a, Move::Down, want);
        assert_eq!(l.keys[space].id, "space");
        let (wrapped, want) = l.step(space, Move::Down, want);
        assert_eq!(l.keys[wrapped].text, "5", "the bottom wraps to the top at the same column");
        assert_eq!(want, 5.5);
        let (up, _) = l.step(space, Move::Up, 5.5);
        assert_eq!(l.keys[up].text, "b");
    }

    #[test]
    fn a_field_asking_for_the_keyboard_opens_it_on_its_first_key_and_announces_the_sheet_once_per_place() {
        let mut k = Ime::new();
        assert!(!k.wanted() && !k.life.visible());
        let fx = k.field_is(&field(PURPOSE_NORMAL, 0, 0, false), 1000);
        assert!(k.wanted() && k.life.presented());
        assert!(posts(&fx).is_empty());
        assert_eq!(k.layout().keys[k.focus()].text, "g");
        let e = k.announce(0, 700, 1920, 380).expect("the sheet is told");
        assert_eq!(e, Effect::Channel { path: "/v1/ime".into(), body: json!({"op": "shown", "x": 0, "y": 700, "w": 1920, "h": 380, "model": 7}) });
        assert!(k.announce(0, 700, 1920, 380).is_none(), "the same place is not told again");
        assert!(k.announce(0, 690, 1920, 390).is_some(), "a new place is");
        // the same field saying the same thing again changes nothing
        k.imekey("right", true, false, 1100);
        let at = k.focus();
        k.field_is(&field(PURPOSE_NORMAL, 0, 0, false), 1200);
        assert_eq!(k.focus(), at, "a topic that only repeats does not begin the field again");
        // another field begins again
        let mut other = field(PURPOSE_NORMAL, 0, 0, false);
        other["model"] = json!(8);
        k.field_is(&other, 1300);
        assert_eq!(k.layout().keys[k.focus()].text, "g");
        let e = k.announce(0, 700, 1920, 380).unwrap();
        assert_eq!(e, Effect::Channel { path: "/v1/ime".into(), body: json!({"op": "shown", "x": 0, "y": 700, "w": 1920, "h": 380, "model": 8}) });
    }

    #[test]
    fn a_field_that_hides_it_or_goes_away_slides_the_keyboard_out_and_a_hidden_one_takes_no_keys() {
        let mut k = up(PURPOSE_NORMAL, 0, 0);
        k.tick(200);
        k.field_is(&json!({"active": true, "show": false, "model": 7}), 300);
        assert!(!k.wanted() && !k.life.presented() && k.life.visible(), "sliding out");
        assert!(k.imekey("ok", true, false, 310).is_empty(), "no key is the keyboard's once it is hidden");
        assert!(k.announce(0, 0, 1, 1).is_none());
        k.field_is(&json!({"active": false, "show": false}), 320);
        assert!(!k.wanted());
    }

    #[test]
    fn ok_types_the_key_a_digit_types_itself_and_a_capital_lasts_one_letter() {
        let mut k = up(PURPOSE_NAME, 0, 0);
        assert_eq!(k.shift(), 1);
        k.set_focus(char_at(&k, "t"));
        let fx = k.imekey("ok", true, false, 10);
        assert_eq!(posts(&fx), vec![json!({"op": "text", "text": "T", "model": 7})]);
        assert_eq!(k.shift(), 0, "back to small letters");
        let fx = k.imekey("ok", true, false, 20);
        assert_eq!(posts(&fx), vec![json!({"op": "text", "text": "t", "model": 7})]);
        assert_eq!(posts(&k.imekey("7", true, false, 30)), vec![json!({"op": "text", "text": "7", "model": 7})]);
        assert!(k.imekey("7", true, true, 40).is_empty(), "a digit does not repeat");
        assert!(k.imekey("7", false, false, 50).is_empty(), "and its release does nothing");
    }

    #[test]
    fn shift_cycles_off_one_capital_caps_lock_and_caps_lock_stays() {
        let mut k = up(PURPOSE_NORMAL, 0, 0);
        let shift = index(&k, "shift");
        let key = k.layout().keys[shift].clone();
        assert_eq!(key.icon, Some("arrow-big-up"));
        for want in [1, 2, 0] {
            k.click(shift, 0);
            assert_eq!(k.shift(), want);
        }
        k.click(shift, 0);
        k.click(shift, 0);
        assert_eq!(k.shift(), 2);
        k.set_focus(char_at(&k, "a"));
        let fx = k.imekey("ok", true, false, 0);
        assert_eq!(posts(&fx)[0]["text"], "A");
        assert_eq!(k.shift(), 2, "caps lock stays");
        // a mark takes no capital and does not end one capital
        let mut k = up(PURPOSE_NORMAL, 0, 0);
        k.click(index(&k, "shift"), 0);
        let fx = k.click(char_at(&k, "?"), 0);
        assert_eq!(posts(&fx)[0]["text"], "?");
        assert_eq!(k.shift(), 1, "the capital waits for a letter");
    }

    #[test]
    fn the_symbols_come_with_the_mode_key_keep_the_focus_on_the_same_place_and_a_pad_has_no_symbols() {
        let mut k = up(PURPOSE_NORMAL, 0, 0);
        let mode = index(&k, "mode");
        k.click(mode, 0);
        assert_eq!(k.layout_id(), LayoutId::Symbols);
        assert_eq!(k.focus(), mode, "the mode key is still where the finger was");
        assert_eq!(k.layout().keys[mode].label, "ABC");
        let fx = k.click(char_at(&k, "!"), 0);
        assert_eq!(posts(&fx)[0]["text"], "!");
        k.click(mode, 0);
        assert_eq!(k.layout_id(), LayoutId::Letters);
        // a new field starts at the letters again
        k.click(mode, 0);
        k.field_is(&field(PURPOSE_NORMAL, 0, 0, false), 10);
        assert_eq!(k.layout_id(), LayoutId::Symbols, "the same field stays where it was");
        let mut other = field(PURPOSE_NORMAL, 0, 0, false);
        other["model"] = json!(9);
        k.field_is(&other, 20);
        assert_eq!(k.layout_id(), LayoutId::Letters);
        let mut p = up(PURPOSE_DIGITS, 0, 0);
        p.symbols = true;
        assert_eq!(p.layout_id(), LayoutId::PadDigits, "a pad has no other layout");
    }

    #[test]
    fn backspace_enter_and_the_cursor_keys_are_keys_of_the_field_and_clear_is_its_own_op_and_needs_text() {
        let mut k = up(PURPOSE_NORMAL, 0, ENTER_SEARCH);
        let key = |k: &mut Ime, id: &str| {
            posts(&{
                let i = index(k, id);
                k.click(i, 0)
            })
        };
        assert_eq!(key(&mut k, "backspace"), vec![json!({"op": "key", "key": "backspace", "model": 7})]);
        assert_eq!(key(&mut k, "left"), vec![json!({"op": "key", "key": "left", "model": 7})]);
        assert_eq!(key(&mut k, "right"), vec![json!({"op": "key", "key": "right", "model": 7})]);
        assert_eq!(key(&mut k, "clear"), vec![json!({"op": "clear", "model": 7})]);
        assert_eq!(key(&mut k, "space"), vec![json!({"op": "text", "text": " ", "model": 7})]);
        assert_eq!(
            key(&mut k, "enter"),
            vec![json!({"op": "key", "key": "enter", "model": 7}), json!({"op": "hide", "model": 7})],
            "Search finishes the field: the keyboard goes"
        );
        assert_eq!(key(&mut k, "hide"), vec![json!({"op": "hide", "model": 7})]);
        let mut e = up(PURPOSE_NORMAL, 0, ENTER_NEXT);
        assert_eq!(key(&mut e, "enter"), vec![json!({"op": "key", "key": "enter", "model": 7})], "Next goes on to the next field");
        // a field with nothing in it has nothing to clear
        let mut empty = Ime::new();
        empty.field_is(&field(PURPOSE_NORMAL, 0, 0, false), 0);
        assert!(!empty.key_enabled(index(&empty, "clear")));
        assert!(key(&mut empty, "clear").is_empty());
        let mut dotcom = up(PURPOSE_EMAIL, 0, 0);
        assert_eq!(key(&mut dotcom, "dotcom"), vec![json!({"op": "text", "text": ".com", "model": 7})], ".com types as it is");
    }

    #[test]
    fn back_puts_the_keyboard_away_and_a_release_or_a_repeat_of_it_does_nothing() {
        let mut k = up(PURPOSE_NORMAL, 0, 0);
        assert_eq!(posts(&k.imekey("back", true, false, 0)), vec![json!({"op": "hide", "model": 7})]);
        assert!(k.imekey("back", true, true, 10).is_empty());
        assert!(k.imekey("back", false, false, 20).is_empty());
    }

    #[test]
    fn a_held_arrow_crosses_the_keyboard_at_the_keyboards_pace_and_ok_repeats_only_on_the_keys_that_repeat() {
        let mut k = up(PURPOSE_NORMAL, 0, 0);
        let start = k.focus();
        k.imekey("right", true, false, 1000);
        assert_ne!(k.focus(), start);
        let after_first = k.focus();
        k.imekey("right", true, true, 1100);
        assert_eq!(k.focus(), after_first, "before the delay no repeat is let through");
        k.imekey("right", true, true, 1310);
        assert_ne!(k.focus(), after_first, "after 300 ms the first is");
        let f = k.focus();
        k.imekey("right", true, true, 1350);
        assert_eq!(k.focus(), f, "then one in 100 ms");
        // OK on a letter does not repeat; on Backspace it does
        let mut k = up(PURPOSE_NORMAL, 0, 0);
        assert!(k.imekey("ok", true, true, 0).is_empty());
        let b = index(&k, "backspace");
        k.set_focus(b);
        k.imekey("ok", true, false, 5000);
        assert_eq!(posts(&k.imekey("ok", true, true, 5400)).len(), 1, "a held Backspace deletes on");
        // after eight steps the pace is the fast one
        let mut g = Gate::default();
        assert!(g.pass("left", false, 0));
        let mut t = 300;
        let mut passed = 0;
        while passed < 8 {
            if g.pass("left", true, t) {
                passed += 1;
            }
            t += 10;
        }
        let step = |g: &mut Gate, from: u64| (from..from + 200).find(|t| g.pass("left", true, *t)).map(|t| t - from).unwrap();
        assert_eq!(step(&mut g, t + 59 - 10), 1, "60 ms after the eighth");
    }

    #[test]
    fn the_pointer_moves_the_focus_on_the_press_and_types_on_the_release_and_a_key_that_needs_text_ignores_it() {
        let mut k = up(PURPOSE_NORMAL, 0, 0);
        let a = char_at(&k, "a");
        k.pointer_down(a);
        assert_eq!(k.focus(), a);
        assert_eq!(posts(&k.click(a, 0)), vec![json!({"op": "text", "text": "a", "model": 7})]);
        let mut empty = Ime::new();
        empty.field_is(&field(PURPOSE_NORMAL, 0, 0, false), 0);
        let clear = index(&empty, "clear");
        assert!(empty.click(clear, 0).is_empty());
        assert_eq!(empty.focus(), clear, "the focus still goes there");
    }

    #[test]
    fn the_hint_text_flag_changes_what_is_enabled_without_beginning_the_field_again() {
        let mut k = Ime::new();
        k.field_is(&field(PURPOSE_NORMAL, 0, 0, false), 0);
        k.imekey("right", true, false, 0);
        let at = k.focus();
        k.field_is(&field(PURPOSE_NORMAL, 0, 0, true), 10);
        assert_eq!(k.focus(), at);
        assert!(k.key_enabled(index(&k, "clear")));
    }

    fn voice_ready() -> Ime {
        let mut k = Ime::new();
        let fx = k.start();
        assert!(matches!(&fx[0], Effect::Luna { tag, uri, subscribe: false, .. } if *tag == tags::PROBE && uri.ends_with("/getConfigs")));
        k.luna(tags::PROBE, &json!({"returnValue": true, "configs": {"system.supportVoiceRecognition": true}}), 0);
        assert!(k.voice_ok());
        k.field_is(&field(PURPOSE_NORMAL, 0, 0, true), 0);
        k
    }

    #[test]
    fn the_microphone_listens_types_the_first_answer_and_says_what_it_is_doing_on_the_space_bar() {
        let mut k = voice_ready();
        assert_eq!(k.layout().keys.iter().filter(|c| c.kind == Kind::Voice).count(), 1);
        let fx = k.click(index(&k, "voice"), 1000);
        assert!(
            matches!(&fx[0], Effect::Luna { tag, subscribe: true, payload, .. } if *tag == tags::VOICE && payload["runVoiceUi"] == false && payload["language"] == "en-US")
        );
        assert!(k.listening());
        assert_eq!(k.note(), "Listening");
        k.luna(tags::VOICE, &json!({"event": "sttStart", "voiceTicket": "t-1"}), 1100);
        k.luna(tags::VOICE, &json!({"event": "sttConnecting"}), 1200);
        assert_eq!(k.note(), "Working");
        let fx = k.luna(tags::VOICE, &json!({"event": "sttResult", "text": ["hello there"]}), 1500);
        assert_eq!(posts(&fx), vec![json!({"op": "text", "text": "hello there", "model": 7})]);
        assert!(!k.listening() && k.note().is_empty());
        assert!(fx.iter().any(|e| matches!(e, Effect::Cancel { tag } if *tag == tags::VOICE)));
        assert!(
            fx.iter().any(|e| matches!(e, Effect::Luna { tag, payload, .. } if *tag == tags::CANCEL && payload["voiceTicket"] == "t-1")),
            "the ticket is given back"
        );
        // nothing heard says so for three seconds
        k.click(index(&k, "voice"), 2000);
        let fx = k.luna(tags::VOICE, &json!({"event": "sttResult", "text": []}), 2100);
        assert!(posts(&fx).is_empty());
        assert_eq!(k.note(), "Nothing heard");
        k.tick(5100);
        assert_eq!(k.note(), "");
    }

    #[test]
    fn back_ends_the_microphone_before_the_keyboard_and_the_keyboard_going_away_ends_it_too() {
        let mut k = voice_ready();
        k.click(index(&k, "voice"), 0);
        let fx = k.imekey("back", true, false, 10);
        assert!(posts(&fx).is_empty(), "the first Back is the microphone's");
        assert!(!k.listening());
        assert_eq!(posts(&k.imekey("back", true, false, 20)).len(), 1, "the second puts the keyboard away");
        k.click(index(&k, "voice"), 30);
        let fx = k.field_is(&json!({"active": true, "show": false, "model": 7}), 40);
        assert!(!k.listening());
        assert!(fx.iter().any(|e| matches!(e, Effect::Cancel { .. })));
        // pressing the key while it listens ends it
        let mut k = voice_ready();
        k.click(index(&k, "voice"), 0);
        k.click(index(&k, "voice"), 5);
        assert!(!k.listening());
    }

    #[test]
    fn a_voice_service_that_errs_or_does_not_answer_says_it_is_not_available_and_a_tv_without_voice_has_no_key() {
        let mut k = voice_ready();
        k.click(index(&k, "voice"), 0);
        k.luna(tags::VOICE, &json!({"returnValue": false, "errorText": "no"}), 100);
        assert!(!k.listening());
        assert_eq!(k.note(), "Voice isn't available");
        k.click(index(&k, "voice"), 1000);
        k.tick(1000 + VOICE_LIMIT_MS);
        assert!(!k.listening());
        assert_eq!(k.note(), "Voice isn't available");
        // a probe that was not answered is asked again at the next field; a TV that says no is not asked twice
        let mut n = Ime::new();
        n.start();
        n.luna(tags::PROBE, &json!({"returnValue": false}), 0);
        assert!(!n.voice_ok());
        let model = |m: u64| json!({"active": true, "show": true, "model": m});
        assert_eq!(n.field_is(&model(11), 10).len(), 1);
        n.luna(tags::PROBE, &json!({"returnValue": true, "configs": {"system.supportVoiceRecognition": false}}), 20);
        assert!(n.field_is(&model(12), 30).is_empty());
        assert!(!n.voice_ok());
        assert_eq!(Layout::build(n.layout_id(), 0, n.voice_ok()).keys.iter().filter(|c| c.kind == Kind::Voice).count(), 0);
    }

    #[test]
    fn a_key_of_the_remote_takes_the_pointers_cue_even_while_the_keyboard_is_not_shown() {
        let mut k = Ime::new();
        k.imekey("up", true, false, 0);
        assert!(k.take_key_taken());
        assert!(!k.take_key_taken());
        k.imekey("up", false, false, 0);
        assert!(!k.take_key_taken(), "a release is not a key");
    }

    #[test]
    fn the_state_names_the_layout_and_the_focus() {
        let mut k = up(PURPOSE_NORMAL, 0, 0);
        while k.state()["key"] != "hide" {
            k.imekey("right", true, false, 0);
        }
        k.imekey("up", true, false, 0);
        assert_eq!(k.state()["key"], "enter");
        assert_eq!(k.state()["layout"], "letters");
        assert_eq!(k.state()["up"], true);
    }
}
