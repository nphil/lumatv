//! Shell-owned text entry uses the same Lucent keys as app text fields. Drafts stay here until the owner receives Save or Cancel.
//! Passwords are fully masked; neither views used for diagnostics nor Debug output contain an entered value.

use super::ime::{self, Gate, Kind, Layout, LayoutId, Move};
use super::life::Life;
use super::{K, Out, Typed};
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

/// New input is bounded, but an existing longer value is never silently truncated on opening it.
pub const MAX_CHARS: usize = 4096;

pub struct View<'a> {
    pub prompt: &'a str,
    pub shown: &'a str,
    pub secret: bool,
    /// Byte positions in the displayed (possibly masked) string, always on grapheme boundaries.
    pub cursor: usize,
    pub selection: Option<(usize, usize)>,
    pub chars: usize,
    pub revision: u64,
    pub keyboard: ime::View,
}

pub struct Entry {
    pub life: Life,
    prompt: String,
    text: String,
    mask: String,
    boundaries: Vec<usize>,
    cursor: usize,
    anchor: Option<usize>,
    secret: bool,
    base: Rc<Layout>,
    symbols: Rc<Layout>,
    symbol_mode: bool,
    shift: u8,
    index: usize,
    column: f32,
    gate: Gate,
    revision: u64,
    up: bool,
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            life: Life::default(), prompt: String::new(), text: String::new(), mask: String::new(), boundaries: vec![0], cursor: 0,
            anchor: None, secret: false, base: Self::layout(LayoutId::Letters), symbols: Self::layout(LayoutId::Symbols),
            symbol_mode: false, shift: 0, index: 0, column: 0.0, gate: Gate::default(), revision: 0, up: false,
        }
    }
}

impl Entry {
    fn layout(id: LayoutId) -> Rc<Layout> {
        let mut layout = Layout::build(id, ime::ENTER_DONE, false);
        if let Some(i) = layout.index_of("enter") { layout.keys[i].label = "Save".into(); }
        if let Some(i) = layout.index_of("hide") { layout.keys[i].icon = Some("x"); }
        if layout.pad {
            let full = Layout::build(LayoutId::Letters, ime::ENTER_DONE, false);
            for (id, row, col) in [("left", 4, 0), ("right", 4, 3), ("hide", 3, 3)] {
                let mut key = full.keys[full.index_of(id).unwrap()].clone();
                key.row = row; key.col = col; key.span = 1;
                if id == "hide" { key.icon = Some("x"); }
                layout.keys.push(key);
            }
            layout.rows = 5;
        }
        Rc::new(layout)
    }

    pub fn new() -> Self { Self::default() }
    pub fn is_up(&self) -> bool { self.up }
    pub(crate) fn visual_state(&self) -> (u64, usize, u8, bool, bool) { (self.revision, self.index, self.shift, self.symbol_mode, self.up) }

    pub(crate) fn set_enter_label(&mut self, label: &str) {
        for layout in [&mut self.base, &mut self.symbols] {
            let layout = Rc::make_mut(layout);
            if let Some(i) = layout.index_of("enter") { layout.keys[i].label.clear(); layout.keys[i].label.push_str(label); }
        }
    }

    pub fn open(&mut self, prompt: &str, text: &str, secret: bool, now: u64) {
        self.open_purpose(prompt, text, secret, ime::PURPOSE_NORMAL, now);
    }

    pub fn open_purpose(&mut self, prompt: &str, text: &str, secret: bool, purpose: u32, _now: u64) {
        self.prompt.clear(); self.prompt.push_str(prompt);
        self.text.clear(); self.text.push_str(text);
        self.secret = secret;
        let id = LayoutId::for_purpose(purpose);
        if self.base.id != id { self.base = Self::layout(id); }
        self.symbol_mode = false;
        self.shift = ime::initial_shift(purpose, 0);
        self.index = self.base.home();
        self.column = self.base.keys[self.index].centre();
        self.anchor = None;
        self.refresh_text();
        self.cursor = self.count();
        self.gate = Gate::default();
        self.up = true;
    }

    /// External field replacement keeps keyboard focus and mode, like input from an app or voice.
    pub fn replace_text(&mut self, text: &str) {
        if self.text == text { return; }
        self.text.clear(); self.text.push_str(text);
        self.anchor = None; self.refresh_text(); self.cursor = self.count();
    }

    fn count(&self) -> usize { self.boundaries.len() - 1 }
    fn current(&self) -> &Rc<Layout> { if self.symbol_mode { &self.symbols } else { &self.base } }

    fn refresh_text(&mut self) {
        self.boundaries.clear();
        self.boundaries.extend(self.text.grapheme_indices(true).map(|(i, _)| i));
        self.boundaries.push(self.text.len());
        self.mask.clear();
        if self.secret { self.mask.extend(std::iter::repeat_n('•', self.count())); }
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn close(&mut self) {
        self.up = false;
        self.text.clear(); self.mask.clear();
        self.boundaries.clear(); self.boundaries.push(0);
        self.cursor = 0; self.anchor = None;
        self.revision = self.revision.wrapping_add(1);
    }

    fn finish(&mut self, save: bool) -> Out {
        if !self.up { return Out::default(); }
        let typed = if save { Typed::Entered(std::mem::take(&mut self.text)) } else { Typed::Cancelled };
        self.close();
        Out { typed: Some(typed), close: true, ..Out::default() }
    }

    fn selected(&self) -> Option<(usize, usize)> {
        self.anchor.filter(|a| *a != self.cursor).map(|a| (a.min(self.cursor), a.max(self.cursor)))
    }

    fn insert(&mut self, value: &str) {
        let (lo, hi) = self.selected().unwrap_or((self.cursor, self.cursor));
        let added = value.graphemes(true).count();
        if self.count() - (hi - lo) + added > MAX_CHARS { return; }
        let byte = self.boundaries[lo];
        self.text.replace_range(byte..self.boundaries[hi], value);
        let end = byte + value.len();
        self.refresh_text();
        self.cursor = self.boundaries.partition_point(|b| *b < end).min(self.count());
        self.anchor = None;
    }

    fn erase(&mut self, forward: bool) {
        let (lo, hi) = self.selected().unwrap_or_else(|| {
            if forward { (self.cursor, (self.cursor + 1).min(self.count())) } else { (self.cursor.saturating_sub(1), self.cursor) }
        });
        if lo != hi {
            self.text.replace_range(self.boundaries[lo]..self.boundaries[hi], "");
            self.refresh_text();
        }
        self.cursor = lo; self.anchor = None;
    }

    fn move_cursor(&mut self, right: bool, selecting: bool) {
        let was_selection = self.selected();
        if selecting { self.anchor.get_or_insert(self.cursor); }
        let next = if !selecting && let Some((lo, hi)) = self.selected() { if right { hi } else { lo } }
            else if right { (self.cursor + 1).min(self.count()) } else { self.cursor.saturating_sub(1) };
        if !selecting { self.anchor = None; }
        let moved = self.cursor != next;
        self.cursor = next;
        if moved || self.selected() != was_selection { self.revision = self.revision.wrapping_add(1); }
    }

    fn press(&mut self, i: usize) -> Out {
        let layout = self.current().clone();
        let Some(key) = layout.keys.get(i) else { return Out::default() };
        match key.kind {
            Kind::Char | Kind::Text => {
                self.insert(if self.shift > 0 && layout.shiftable { &key.shifted } else { &key.text });
                if key.letter && self.shift == 1 { self.shift = 0; }
            }
            Kind::Space => self.insert(" "),
            Kind::Shift => self.shift = (self.shift + 1) % 3,
            Kind::Mode(to) => {
                self.symbol_mode = to == LayoutId::Symbols;
                self.index = self.current().at(key.row, key.col).unwrap_or_else(|| self.current().home());
                self.column = self.current().keys[self.index].centre();
            }
            Kind::Backspace => self.erase(false),
            Kind::Clear => { self.anchor = Some(0); self.cursor = self.count(); self.erase(false); }
            Kind::Cursor(ime::Dir::Left) => self.move_cursor(false, false),
            Kind::Cursor(ime::Dir::Right) => self.move_cursor(true, false),
            Kind::Enter => return self.finish(true),
            Kind::Hide => return self.finish(false),
            Kind::Voice => {}
        }
        Out::default()
    }

    /// Complete press/release input: only navigation, deletion and cursor keys repeat; Save and Cancel never do.
    pub fn input(&mut self, name: &str, down: bool, repeat: bool, now: u64) -> Out {
        if !self.up || !down { return Out::default(); }
        let repeats = matches!(name, "up" | "down" | "left" | "right" | "backspace" | "delete")
            || name == "ok" && self.current().keys.get(self.index).is_some_and(|k| k.repeat);
        if repeat && !repeats { return Out::default(); }
        if repeats && !self.gate.pass(name, repeat, now) { return Out::default(); }
        match name {
            "backspace" => { self.erase(false); Out::default() }
            "delete" => { self.erase(true); Out::default() }
            "enter" => self.finish(true),
            "home" | "end" => { self.cursor = if name == "home" { 0 } else { self.count() }; self.anchor = None; self.revision = self.revision.wrapping_add(1); Out::default() }
            "selectall" => { self.anchor = Some(0); self.cursor = self.count(); self.revision = self.revision.wrapping_add(1); Out::default() }
            "selectleft" | "selectright" => { self.move_cursor(name == "selectright", true); Out::default() }
            _ => K::parse(name).map_or_else(Out::default, |k| self.key(k, now)),
        }
    }

    pub fn key(&mut self, k: K, _now: u64) -> Out {
        if !self.up { return Out::default(); }
        let dir = match k {
            K::Back => return self.finish(false),
            K::Ok => return self.press(self.index),
            K::Digit(d) => { let digit = [b'0' + d.min(9)]; self.insert(std::str::from_utf8(&digit).unwrap()); return Out::default(); }
            K::Up => Move::Up, K::Down => Move::Down, K::Left => Move::Left, K::Right => Move::Right,
        };
        (self.index, self.column) = self.current().step(self.index, dir, self.column);
        Out::default()
    }

    pub fn click(&mut self, i: usize, _now: u64) -> Out {
        if !self.up || i >= self.current().keys.len() { return Out::default(); }
        self.index = i; self.column = self.current().keys[i].centre(); self.press(i)
    }
    pub fn click_outside(&mut self) -> Out { self.finish(false) }
    pub fn index(&self) -> usize { self.index }
    pub fn cap(&self, i: usize) -> &str { self.current().label(i, self.shift) }

    pub fn view(&self, _over_video: bool) -> View<'_> {
        let at = |i: usize| if self.secret { i * '•'.len_utf8() } else { self.boundaries[i] };
        View {
            prompt: &self.prompt, shown: if self.secret { &self.mask } else { &self.text }, secret: self.secret,
            cursor: at(self.cursor), selection: self.selected().map(|(lo, hi)| (at(lo), at(hi))), chars: self.count(), revision: self.revision,
            keyboard: ime::View { layout: self.current().clone(), shift: self.shift, focus: self.index, has_text: self.count() > 0, note: String::new(), listening: false },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn press(e: &mut Entry, id: &str) -> Out { let i = e.current().index_of(id).unwrap(); e.click(i, 0) }
    fn character(e: &mut Entry, c: &str) { let i = e.current().keys.iter().position(|k| k.text == c).unwrap(); e.click(i, 0); }

    #[test]
    fn editing_unicode_uses_whole_graphemes_and_selection_replaces_in_place() {
        let mut e = Entry::new(); e.open("Name", "e\u{301}👩🏽‍💻東京", false, 0);
        press(&mut e, "left"); e.input("backspace", true, false, 0);
        assert_eq!(e.view(false).shown, "e\u{301}👩🏽‍💻京");
        e.input("home", true, false, 0); e.input("delete", true, false, 0);
        assert_eq!(e.view(false).shown, "👩🏽‍💻京");
        e.input("selectright", true, false, 0); character(&mut e, "a");
        assert_eq!(e.view(false).shown, "a京");
        e.input("selectall", true, false, 0); e.key(K::Digit(7), 0);
        assert_eq!(press(&mut e, "enter").typed, Some(Typed::Entered("7".into())));
        assert!(press(&mut e, "enter").typed.is_none(), "an answer is delivered once");
    }

    #[test]
    fn passwords_never_reveal_the_last_character_or_return_a_draft_on_cancel() {
        let mut e = Entry::new(); e.open("Password", "A👩🏽‍💻e\u{301}", true, 0);
        assert_eq!(e.view(false).shown, "•••");
        character(&mut e, "x"); assert_eq!(e.view(false).shown, "••••");
        assert_eq!(e.key(K::Back, 0).typed, Some(Typed::Cancelled));
        assert!(e.view(false).shown.is_empty());
    }

    #[test]
    fn long_prefills_and_hostname_punctuation_survive_edit_and_save() {
        let host = format!("{}.{}.{}.{}", "a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(61));
        let mut e = Entry::new(); e.open_purpose("Server address", &host, false, ime::PURPOSE_URL, 0);
        assert_eq!(e.count(), 253); assert_eq!(press(&mut e, "enter").typed, Some(Typed::Entered(host)));
        e.open_purpose("Server address", "", false, ime::PURPOSE_URL, 0);
        for c in ["n", "a", "s", "-", "1", ".", "l", "a", "n", ":", "5", "1", "4"] { character(&mut e, c); }
        assert_eq!(press(&mut e, "enter").typed, Some(Typed::Entered("nas-1.lan:514".into())));
        e.open("Existing", &"x".repeat(MAX_CHARS + 1), false, 0);
        assert_eq!(e.count(), MAX_CHARS + 1, "opening never truncates existing data");
    }

    #[test]
    fn held_delete_repeats_but_save_and_back_do_not_and_new_presses_are_never_lost() {
        let mut e = Entry::new(); e.open("Name", "abcdef", false, 0);
        let delete = e.current().index_of("backspace").unwrap(); e.click(delete, 0);
        e.input("ok", true, false, 10); e.input("ok", true, true, 100); e.input("ok", true, true, 310);
        assert_eq!(e.view(false).shown, "abc");
        e.input("ok", true, false, 320); e.input("ok", true, false, 321);
        assert_eq!(e.view(false).shown, "a");
        e.index = e.current().index_of("enter").unwrap();
        assert!(e.input("ok", true, true, 1000).typed.is_none());
        assert_eq!(e.input("back", true, false, 1001).typed, Some(Typed::Cancelled));
    }
}
