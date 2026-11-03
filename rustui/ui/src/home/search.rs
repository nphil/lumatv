//! Local Home answers above the shared Lucent keyboard. Entry owns the draft and editing; Search owns matching and choosing an answer.

use crate::kit::focus::{Focus, Id, Region};
use crate::panels::entry::{Entry, View};
use crate::panels::ime_surface::{Geometry, geometry_for};
use crate::panels::{Out, Typed};
use lumatv_tokens::{Tokens, TypeRole};
use serde_json::{Value, json};
use ui::geometry::Frame;

pub const KEY_ID: u32 = 600;
pub const ANSWER_ID: u32 = 700;
pub const KEYS: Region = Region(7);
pub const ANSWERS: Region = Region(11);

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

pub struct Search {
    draft: Entry,
    pub entry: bool,
    answer: Option<usize>,
    apps: Vec<Value>,
    pages: Vec<Value>,
    answers: Vec<Answer>,
    query: String,
    tokens: Tokens,
    scale: f32,
    geo: Geometry,
    last_now: u64,
    caret_visible: bool,
}

impl Default for Search {
    fn default() -> Self {
        let draft = Entry::new();
        let tokens = Tokens::shipped();
        let geo = geometry_for(&tokens, 1.0, &draft.view(false).keyboard.layout, header_height(&tokens, 1.0));
        Self { draft, entry: false, answer: None, apps: Vec::new(), pages: Vec::new(), answers: Vec::new(), query: String::new(), tokens, scale: 1.0, geo, last_now: 0, caret_visible: true }
    }
}

fn line(tokens: &Tokens, scale: f32, role: TypeRole) -> f32 {
    ui::components::line_height(ui::text::Type { px: (tokens.ty(role).px as f32 * scale.clamp(1.0, 2.0)).round() as u32, ..tokens.ty(role) })
}

pub fn field_height(tokens: &Tokens, scale: f32) -> f32 {
    84.0f32.max(line(tokens, scale, TypeRole::Title) + 2.0 * tokens.space(3)).ceil()
}

pub fn answer_height(tokens: &Tokens, scale: f32) -> f32 {
    108.0f32.max(line(tokens, scale, TypeRole::Label) + line(tokens, scale, TypeRole::Caption) + 2.0 * tokens.space(3)).ceil()
}

pub fn header_height(tokens: &Tokens, scale: f32) -> f32 {
    field_height(tokens, scale) + answer_height(tokens, scale) + 2.0 * tokens.space(5)
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
    pub fn configure(&mut self, theme: &Value, scale: f32) {
        self.tokens = Tokens::from_json(theme);
        self.scale = scale;
        self.relayout();
    }

    fn relayout(&mut self) {
        self.geo = geometry_for(&self.tokens, self.scale, &self.draft.view(false).keyboard.layout, header_height(&self.tokens, self.scale));
    }

    pub fn open(&mut self, apps: Vec<Value>, now: u64) {
        self.apps = apps;
        self.entry = false;
        self.answer = None;
        self.draft.open("Search", "", false, now);
        self.draft.set_enter_label("Done");
        self.refresh_answers();
        self.relayout();
        self.last_now = now;
        self.caret_visible = true;
    }

    pub fn set_pages(&mut self, pages: Vec<Value>) {
        self.pages = pages;
        self.answers = if self.entry { Vec::new() } else { answers(self.text(), &self.apps, &self.pages) };
        self.clamp_answer();
    }

    pub fn open_entry(&mut self, prompt: &str, text: &str, now: u64) {
        self.entry = true;
        self.answer = None;
        self.draft.open(prompt, text, false, now);
        self.draft.set_enter_label("Save");
        self.refresh_answers();
        self.relayout();
        self.last_now = now;
        self.caret_visible = true;
    }

    pub fn view(&self) -> View<'_> { self.draft.view(false) }
    pub fn text(&self) -> &str { self.draft.view(false).shown }
    pub fn answers(&self) -> &[Answer] { &self.answers }
    pub fn answer_index(&self) -> Option<usize> { self.answer }
    pub fn key_frames(&self) -> &[Frame] { &self.geo.keys }
    pub fn visual_state(&self) -> ((u64, usize, u8, bool, bool), Option<usize>) { (self.draft.visual_state(), self.answer) }

    pub fn field_frame(&self) -> Frame {
        Frame::new(self.geo.sheet.x + self.tokens.space(8), self.geo.sheet.y + self.tokens.space(5), self.geo.sheet.w - 2.0 * self.tokens.space(8), field_height(&self.tokens, self.scale))
    }

    pub fn answer_frame(&self, i: usize) -> Frame {
        let f = self.field_frame();
        let gap = self.tokens.space(4);
        let w = ((f.w - 3.0 * gap) / 4.0).floor();
        Frame::new(f.x + i as f32 * (w + gap), f.y + f.h + self.tokens.space(5), w, answer_height(&self.tokens, self.scale))
    }

    fn clamp_answer(&mut self) {
        self.answer = self.answer.and_then(|i| (!self.answers.is_empty()).then(|| i.min(self.answers.len() - 1)));
    }

    fn refresh_answers(&mut self) {
        let text = self.draft.view(false).shown;
        if self.query != text {
            self.query.clear(); self.query.push_str(text);
            self.answers = if self.entry { Vec::new() } else { answers(text, &self.apps, &self.pages) };
        } else if self.entry || text.is_empty() {
            self.answers.clear();
        }
        self.clamp_answer();
    }

    fn result(&mut self, out: Out) -> SearchOut {
        match out.typed {
            Some(Typed::Entered(text)) => if self.entry { SearchOut::Entered(text) } else { SearchOut::Close },
            Some(Typed::Cancelled) => SearchOut::Close,
            _ => { self.refresh_answers(); SearchOut::Nothing }
        }
    }

    /// Phone and voice replace the draft; a trailing newline submits without launching an answer.
    pub fn hook_text(&mut self, text: &str) -> SearchOut {
        let done = text.ends_with('\n');
        self.draft.replace_text(text.strip_suffix('\n').unwrap_or(text));
        if done { let out = self.draft.input("enter", true, false, self.last_now); self.result(out) }
        else { self.refresh_answers(); SearchOut::Nothing }
    }

    pub fn register(&self, focus: &mut Focus) {
        let scope = super::model::SCOPE_SEARCH;
        for (i, f) in self.geo.keys.iter().enumerate() { focus.register(Id(KEY_ID + i as u32), *f, KEYS, scope); }
        for i in 0..4 {
            let id = Id(ANSWER_ID + i as u32);
            if i < self.answers.len() { focus.register(id, self.answer_frame(i), ANSWERS, scope); }
            else { focus.unregister(id); }
        }
        self.refocus(focus);
    }

    fn refocus(&self, focus: &mut Focus) {
        focus.focus(self.answer.map_or(Id(KEY_ID + self.draft.index() as u32), |i| Id(ANSWER_ID + i as u32)));
    }

    pub fn unregister(&self, focus: &mut Focus) {
        for i in 0..self.geo.keys.len() as u32 { focus.unregister(Id(KEY_ID + i)); }
        for i in 0..4 { focus.unregister(Id(ANSWER_ID + i)); }
    }

    fn pick(&self, i: usize) -> SearchOut {
        self.answers.get(i).map_or(SearchOut::Nothing, |a| SearchOut::Picked { kind: a.kind.to_owned(), id: a.id.clone(), text: self.text().to_owned() })
    }

    pub fn input(&mut self, name: &str, down: bool, repeat: bool, focus: &mut Focus, now: u64) -> SearchOut {
        self.last_now = now;
        if !down { return SearchOut::Nothing; }
        if let Some(i) = self.answer {
            match name {
                "ok" if !repeat => return self.pick(i),
                "left" => self.answer = Some(i.saturating_sub(1)),
                "right" => self.answer = Some((i + 1).min(self.answers.len() - 1)),
                "down" => self.answer = None,
                "up" | "ok" => {},
                _ => self.answer = None,
            }
            if matches!(name, "left" | "right" | "down" | "up" | "ok") { self.refocus(focus); return SearchOut::Nothing; }
        }
        let before = self.draft.view(false);
        if name == "up" && before.keyboard.layout.keys[before.keyboard.focus].row == 0 && !self.answers.is_empty() {
            let key = self.geo.keys[self.draft.index()];
            let x = key.x + key.w / 2.0;
            self.answer = (0..self.answers.len()).min_by(|a, b| {
                let distance = |i| { let f = self.answer_frame(i); (f.x + f.w / 2.0 - x).abs() };
                distance(*a).total_cmp(&distance(*b))
            });
            self.refocus(focus);
            return SearchOut::Nothing;
        }
        let layout = before.keyboard.layout.id;
        let revision = before.revision;
        let out = self.draft.input(name, down, repeat, now);
        let result = self.result(out);
        if self.draft.view(false).keyboard.layout.id != layout { self.relayout(); self.register(focus); }
        else if self.draft.view(false).revision != revision { self.register(focus); }
        else { self.refocus(focus); }
        result
    }

    pub fn clicked(&mut self, id: Id, focus: &mut Focus, now: u64) -> SearchOut {
        self.last_now = now;
        if let Some(i) = id.0.checked_sub(ANSWER_ID).filter(|i| (*i as usize) < self.answers.len()) { return self.pick(i as usize); }
        let Some(i) = id.0.checked_sub(KEY_ID).filter(|i| (*i as usize) < self.geo.keys.len()) else { return SearchOut::Nothing };
        self.answer = None;
        let layout = self.draft.view(false).keyboard.layout.id;
        let out = self.draft.click(i as usize, now);
        let result = self.result(out);
        if self.draft.view(false).keyboard.layout.id != layout { self.relayout(); }
        self.register(focus);
        result
    }

    pub fn caret_on(&self, now: u64) -> bool { (now / 500) % 2 == 0 }
    pub fn tick(&mut self, now: u64) -> bool {
        self.last_now = now;
        if !self.draft.is_up() { return false; }
        let on = self.caret_on(now);
        let changed = on != self.caret_visible;
        self.caret_visible = on;
        changed
    }
    pub fn due_at(&self) -> Option<u64> { self.draft.is_up().then_some((self.last_now / 500 + 1) * 500) }

    pub fn state(&self) -> Value {
        let v = self.view();
        json!({"text": v.shown, "entry": self.entry, "index": self.answer.map_or(v.keyboard.focus, |i| v.keyboard.layout.keys.len() + i), "answer": self.answer,
            "shift": v.keyboard.shift, "symbols": v.keyboard.layout.id == crate::panels::ime::LayoutId::Symbols,
            "cursor": v.cursor, "selection": v.selection,
            "answers": self.answers.iter().map(|a| json!({"kind": a.kind, "id": a.id, "title": a.title})).collect::<Vec<_>>()})
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
        assert_eq!(a[1].match_at, Some(3));
        assert!(answers("", &apps, &pages).is_empty());
        assert_eq!(at("Internet", "net"), None);
    }

    #[test]
    fn replacement_and_cursor_edits_refresh_answers_without_resetting_the_keyboard() {
        let mut s = Search::default();
        let mut f = Focus::new();
        s.open(vec![json!({"id": "netflix", "title": "Netflix"})], 0);
        s.register(&mut f);
        let index = s.view().keyboard.focus;
        s.hook_text("net👨‍👩‍👧‍👦");
        s.input("backspace", true, false, &mut f, 100);
        assert_eq!(s.text(), "net");
        assert_eq!(s.answers()[0].id, "netflix");
        assert_eq!(s.view().keyboard.focus, index);
        s.input("selectleft", true, false, &mut f, 200);
        s.input("delete", true, false, &mut f, 300);
        assert_eq!(s.text(), "ne");
        let answers_at = s.answers().as_ptr();
        s.input("right", true, false, &mut f, 400);
        assert_eq!(s.answers().as_ptr(), answers_at, "focus travel retains the matched answers");
        s.open_entry("Rename Netflix", "Netflix", 500);
        assert!(matches!(s.hook_text("Films\n"), SearchOut::Entered(t) if t == "Films"));
        s.open(vec![], 600);
        assert!(matches!(s.hook_text("films\n"), SearchOut::Close));
    }

    #[test]
    fn removing_answers_while_one_is_selected_returns_to_the_keyboard() {
        let mut s = Search::default();
        let mut f = Focus::new();
        s.open(vec![], 0);
        s.hook_text("hello");
        for n in 0..3 { s.input("up", true, false, &mut f, 100 + n * 100); }
        assert_eq!(s.answer_index(), Some(0));
        s.hook_text("");
        s.register(&mut f);
        assert_eq!(s.answer_index(), None);
        assert_eq!(f.current(), Some(Id(KEY_ID + s.view().keyboard.focus as u32)));
        assert!(matches!(s.clicked(Id(ANSWER_ID), &mut f, 500), SearchOut::Nothing));
    }
}

