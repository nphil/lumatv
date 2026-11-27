//! The Home editor updates a private preview. The same tools drive pointer and
//! remote input; the shell alone saves the layout.

use super::dashboard::Start;
use super::dashboard_sizes::{self, Fit, Preset};
use super::onhome::layout::Metrics;
use super::grid::{self, Entry, Rect, Refusal};
use crate::kit::focus::{Dir, Focus, Id, Region, Scope};
use serde_json::{Value, json};
use ui::geometry::Frame;

pub const BAR_ID: u32 = 800;
pub const ADD_ID: u32 = 900;
pub const ASK_ID: u32 = 950;
pub const BAR: Region = Region(8);
pub const ADD: Region = Region(9);
pub const ASK: Region = Region(10);
pub const BAR_Y: f32 = 50.0;
const OPTION_MAX: usize = 9;
const MOVE_BUTTON: usize = 1;
const SIZE_BUTTON: usize = 2;
pub const ASK_ANSWERS: [(&str, bool); 2] = [("Keep editing", false), ("Discard", true)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Select,
    Move,
    Resize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Grid,
    Bar,
    Add,
    Size,
    /// Keep editing or discard the unsaved work.
    Ask,
    /// The items of the chosen smart-home card: the arrows choose one, OK opens its menu (size, place, hide), Back is done.
    Items,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditorOut {
    Preview(Value),
    Save(Value),
    Leave,
    /// The items of the smart-home card with this id are to be arranged: the card takes the keys.
    Items(String),
}

pub struct Button {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    pub enabled: bool,
    pub frame: Frame,
}
/// A change a drag asks for: the column and row (a move) or the span and rows (a resize).
type Snap = (Option<i64>, Option<i64>, Option<i64>, Option<i64>);

#[derive(Clone, Debug)]
struct Drag {
    x: f32,
    y: f32,
    size: bool,
    on: bool,
    to: Option<Snap>,
}

pub struct Editor {
    pub orig: Value,
    base: Value,
    pub work: Value,
    history: Vec<Value>,
    pub sel: String,
    pub mode: Mode,
    held: Option<Value>,
    pub zone: Zone,
    pub bar_index: usize,
    pub add_index: usize,
    pub ask_index: usize,
    pub toast: String,
    toast_until: u64,
    pub refused_until: u64,
    pub start: Start,
    pub saving: bool,
    pub locked: bool,
    pub issue: &'static str,
    size_kind: Option<&'static str>,
    sizes: Vec<Preset>,
    pub size_rev: u64,
    ask_from: Zone,
    drag: Option<Drag>,
    pub ghost: Option<(Rect, bool)>,
    /// The person's text size: the bar, its buttons, the add list and the question grow with it.
    pub scale: f32,
}

impl Editor {
    pub fn new(layout: &Value, id: &str, _now: u64, scale: f32) -> Self {
        let base = grid::tidy_layout(&grid::to_uniform(layout));
        let order = grid::order(&base);
        let sel = if grid::find(&base, id).is_some_and(|e| !e.hidden) {
            id.to_owned()
        } else {
            order.first().cloned().or_else(|| grid::dock_id(&base)).unwrap_or_default()
        };
        Self {
            orig: layout.clone(),
            work: base.clone(),
            base,
            history: Vec::new(),
            sel,
            mode: Mode::Select,
            held: None,
            zone: Zone::Grid,
            bar_index: 0,
            add_index: 0,
            ask_index: 0,
            toast: String::new(),
            toast_until: 0,
            refused_until: 0,
            start: Start::Edit,
            saving: false,
            locked: false,
            issue: "",
            size_kind: None,
            sizes: Vec::new(),
            size_rev: 0,
            ask_from: Zone::Grid,
            drag: None,
            ghost: None,
            scale: scale.max(1.0),
        }
    }

    /// A separate layout is previewed for Create and both types of guide.
    pub(super) fn draft(original: &Value, blank: &Value, start: Start, now: u64, scale: f32) -> Self {
        let mut ed = Self::new(blank, "", now, scale);
        ed.orig = original.clone();
        ed.start = start;
        ed.zone = Zone::Bar;
        ed
    }

    pub fn header_h(&self) -> f32 {
        (44.0 * self.scale).round()
    }

    pub fn bar_h(&self) -> f32 {
        self.header_h() + (68.0 * self.scale).round() + 8.0
    }

    pub(super) fn status_height(&self) -> f32 {
        let height = self.work.pointer("/statusBar/height").and_then(Value::as_f64).unwrap_or(60.0) as f32;
        let normal = (height * self.scale / 2.0).round() * 2.0;
        let area = grid::area(&self.work);
        normal.max(BAR_Y + self.bar_h() + 12.0 - area.y as f32 + height.round())
    }

    pub fn area(&self) -> Rect {
        let mut area = grid::area(&self.work);
        let height = self.work.pointer("/statusBar/height").and_then(Value::as_f64).unwrap_or(60.0).round() as i32;
        let shift = self.status_height().round() as i32 - height;
        area.y += shift;
        area.h = (area.h - shift).max(1);
        area
    }

    pub fn rect(&self, card: &Entry) -> Rect {
        grid::uniform_rect(&self.work, self.area(), card)
    }

    pub fn dirty(&self) -> bool {
        self.work != self.base
    }

    pub fn card(&self) -> Option<Entry> {
        grid::find(&self.work, &self.sel)
    }

    pub fn sel_rect(&self) -> Option<Rect> {
        self.card().map(|c| self.rect(&c))
    }

    pub fn can_remove(&self) -> bool {
        self.card().is_some_and(|c| !c.is_dock())
    }

    pub fn mode_name(&self) -> &'static str {
        match self.mode {
            Mode::Select => "select",
            Mode::Move => "move",
            Mode::Resize => "resize",
        }
    }

    pub fn zone_name(&self) -> &'static str {
        match self.zone {
            Zone::Grid => "grid",
            Zone::Bar => "bar",
            Zone::Add => "add",
            Zone::Size => "size",
            Zone::Ask => "ask",
            Zone::Items => "items",
        }
    }

    /// Whether the chosen card is one whose items can be arranged: a smart-home card.
    pub fn arrangeable(&self) -> bool {
        !self.start.is_new() && self.card().is_some_and(|c| c.kind == "onhome")
    }

    pub fn has_widget(&self) -> bool {
        grid::widgets(&self.work).iter().any(|w| w.get("type").and_then(Value::as_str).is_some_and(|s| s != "dock") && w.get("hidden").and_then(Value::as_bool) != Some(true))
    }

    pub fn can_save(&self) -> bool {
        !self.saving && !self.locked && self.mode == Mode::Select && (!self.start.guided() || self.has_widget())
    }

    /// A separate prompt line keeps the tools usable even at 200% text size.
    pub fn buttons(&self) -> [Button; 8] {
        let editing = self.mode == Mode::Select && !self.saving;
        let card = self.card();
        let specs = [
            ("add", "Add", "plus", editing, 176.0),
            ("move", "Move", "move", editing && card.is_some(), 178.0),
            ("size", "Size", "maximize-2", editing && card.as_ref().is_some_and(|c| grid::type_of(&c.kind).is_some()), 162.0),
            ("remove", "Remove", "trash-2", editing && card.as_ref().is_some_and(|c| !c.is_dock()), 218.0),
            ("items", "Items", "layout-grid", editing && !self.start.is_new() && card.as_ref().is_some_and(|c| c.kind == "onhome"), 160.0),
            ("undo", "Undo", "undo-2", editing && !self.history.is_empty(), 166.0),
            ("cancel", if self.start.guided() { "Skip" } else { "Cancel" }, "x", !self.saving, 178.0),
            ("save", if self.start.is_new() { "Finish" } else { "Save" }, "check", self.can_save(), 184.0),
        ];
        let kw = self.scale.min(1.17);
        let gap = 10.0;
        let mut x = 82.0;
        let bh = (58.0 * self.scale).round();
        let y = BAR_Y + self.header_h() + 4.0;
        specs.map(|(id, label, icon, enabled, width)| {
            let w = (width * kw).round();
            let b = Button { id, label, icon, enabled, frame: Frame::new(x, y, w, bh) };
            x += w + gap;
            b
        })
    }

    pub fn options_len(&self) -> usize {
        match self.zone {
            Zone::Add => grid::ADDABLE.len(),
            Zone::Size => self.sizes.len() + usize::from(self.size_kind.is_none()),
            _ => 0,
        }
    }

    pub fn options_frame(&self) -> Frame {
        let y = BAR_Y + self.bar_h() + 8.0;
        let n = self.options_len().max(1) as f32;
        let row = (68.0 * self.scale).min((1026.0 - y - 24.0) / n).floor();
        let x = if self.zone == Zone::Size && self.size_kind.is_none() { self.buttons()[SIZE_BUTTON].frame.x } else { 82.0 };
        let width = if self.zone == Zone::Size { 492.0 } else { 440.0 };
        Frame::new(x, y, (width * self.scale).round(), n * row + 24.0)
    }

    pub fn option_frame(&self, i: usize) -> Frame {
        let f = self.options_frame();
        let row = (f.h - 24.0) / self.options_len().max(1) as f32;
        Frame::new(f.x + 8.0, f.y + 12.0 + i as f32 * row, f.w - 16.0, row)
    }

    pub fn option(&self, i: usize) -> (String, &'static str) {
        if self.zone == Zone::Add {
            return grid::type_of(grid::ADDABLE[i]).map_or((String::new(), ""), |t| (t.title.to_owned(), t.icon));
        }
        let Some(preset) = self.sizes.get(i) else { return ("Adjust by cell".into(), "move") };
        let detail = match preset.fit {
            Fit::NoRoom => " · No room",
            _ if preset.current => " · In use",
            _ if self.size_kind.is_some() => " · Fits",
            Fit::Here => " · Fits here",
            Fit::Elsewhere { .. } => " · Fits elsewhere",
        };
        (format!("{}{detail}", preset.label), "maximize-2")
    }

    /// The question's frame and its answers' frames.
    pub fn ask_frames(&self) -> (Frame, [Frame; 2]) {
        let k = self.scale;
        let (w, h) = ((760.0 * k).min(1792.0).round(), (236.0 * k).round());
        let f = Frame::new(((1920.0 - w) / 2.0).round(), ((1080.0 - h) / 2.0).round(), w, h);
        let bh = (64.0 * k).round();
        let bw = ((w - 64.0 - 12.0) / 2.0).floor();
        let answers = [0, 1].map(|i| Frame::new(f.x + 32.0 + i as f32 * (bw + 12.0), f.y + h - 32.0 - bh, bw, bh));
        (f, answers)
    }

    /// The bar, the add list and the question as focusables of `scope` (for the pointer's cue and click); the cards are chosen by
    /// their own rule.
    pub fn register(&self, focus: &mut Focus, scope: Scope) {
        for (i, b) in self.buttons().iter().enumerate() {
            let id = Id(BAR_ID + i as u32);
            focus.register(id, b.frame, BAR, scope);
            focus.set_enabled(id, b.enabled && !matches!(self.zone, Zone::Add | Zone::Size | Zone::Ask | Zone::Items));
        }
        let count = self.options_len();
        for i in 0..OPTION_MAX {
            let id = Id(ADD_ID + i as u32);
            if i < count {
                focus.register(id, self.option_frame(i), ADD, scope);
            } else {
                focus.unregister(id);
            }
        }
        let (_, answers) = self.ask_frames();
        for (i, f) in answers.iter().enumerate() {
            let id = Id(ASK_ID + i as u32);
            if self.zone == Zone::Ask {
                focus.register(id, *f, ASK, scope);
            } else {
                focus.unregister(id);
            }
        }
        let current = match self.zone {
            Zone::Bar => Some(Id(BAR_ID + self.bar_index as u32)),
            Zone::Add | Zone::Size => Some(Id(ADD_ID + self.add_index as u32)),
            Zone::Ask => Some(Id(ASK_ID + self.ask_index as u32)),
            Zone::Grid | Zone::Items => None,
        };
        match current {
            Some(id) => {
                focus.focus(id);
            }
            None => focus.clear(),
        }
    }

    pub fn unregister(&self, focus: &mut Focus) {
        for i in 0..self.buttons().len() as u32 {
            focus.unregister(Id(BAR_ID + i));
        }
        for i in 0..OPTION_MAX as u32 {
            focus.unregister(Id(ADD_ID + i));
        }
        for i in 0..ASK_ANSWERS.len() as u32 {
            focus.unregister(Id(ASK_ID + i));
        }
    }

    // ---- changes
    fn say(&mut self, text: &str, now: u64) {
        self.toast = text.to_owned();
        self.toast_until = now + 1800;
    }

    fn turned_down(&mut self, r: &Refusal, now: u64) {
        match r {
            Refusal::Edge | Refusal::Missing => return,
            _ => {}
        }
        self.refused_until = now + 450;
        let words = match r {
            Refusal::Overlap(b) => format!("{} is in the way", b.title()),
            Refusal::Small(_) => "Smallest size".into(),
            Refusal::Full => "No room".into(),
            _ => "Not here".into(),
        };
        self.say(&words, now);
    }

    /// A change that is one step of its own (add, remove, undo): the layout before it is kept to go back to.
    fn step(&mut self, next: Value, id: Option<String>, outs: &mut Vec<EditorOut>) {
        self.history.push(self.work.clone());
        self.work = next;
        if let Some(id) = id {
            self.sel = id;
        }
        outs.push(EditorOut::Preview(self.work.clone()));
    }

    fn change(&mut self, r: grid::Changed, now: u64, outs: &mut Vec<EditorOut>) -> bool {
        match r {
            Ok((layout, _)) => {
                self.work = layout;
                outs.push(EditorOut::Preview(self.work.clone()));
                true
            }
            Err(e) => {
                self.turned_down(&e, now);
                false
            }
        }
    }

    fn pick_up(&mut self) {
        if self.card().is_some() {
            self.held = Some(self.work.clone());
            self.mode = Mode::Move;
        }
    }

    fn put_down(&mut self) {
        if let Some(h) = self.held.take()
            && h != self.work
        {
            self.history.push(h);
        }
        self.mode = Mode::Select;
        self.ghost = None;
    }

    fn put_back(&mut self, outs: &mut Vec<EditorOut>) {
        if let Some(h) = self.held.take() {
            self.work = h;
            if grid::find(&self.work, &self.sel).is_none() {
                self.sel = grid::order(&self.work).into_iter().next().or_else(|| grid::dock_id(&self.work)).unwrap_or_default();
            }
            outs.push(EditorOut::Preview(self.work.clone()));
        }
        self.mode = Mode::Select;
        self.ghost = None;
    }

    fn undo(&mut self, outs: &mut Vec<EditorOut>) {
        let Some(prev) = self.history.pop() else { return };
        self.work = prev;
        if grid::find(&self.work, &self.sel).is_none() {
            self.sel = grid::order(&self.work).into_iter().next().or_else(|| grid::dock_id(&self.work)).unwrap_or_default();
        }
        outs.push(EditorOut::Preview(self.work.clone()));
    }

    fn remove_card(&mut self, now: u64, outs: &mut Vec<EditorOut>) {
        if !self.can_remove() {
            self.say("The dock stays", now);
            return;
        }
        let order = grid::order(&self.work);
        let i = order.iter().position(|x| *x == self.sel).unwrap_or(0);
        match grid::remove(&self.work, &self.sel) {
            Ok(layout) => {
                let rest: Vec<String> = order.into_iter().filter(|x| *x != self.sel).collect();
                let next = if rest.is_empty() { grid::dock_id(&layout).unwrap_or_default() } else { rest[i.min(rest.len() - 1)].clone() };
                self.step(layout, Some(next), outs);
            }
            Err(e) => self.turned_down(&e, now),
        }
    }

    fn open_sizes(&mut self, new: Option<&'static str>, theme: &Value, metrics: &Metrics) {
        let current = if new.is_none() { self.card() } else { None };
        let kind = match (new, &current) {
            (Some(kind), _) => kind,
            (None, Some(card)) => card.kind.as_str(),
            _ => return,
        };
        self.sizes = dashboard_sizes::choices(&self.work, kind, current.as_ref(), theme, self.area(), self.scale, metrics);
        self.size_rev = self.size_rev.wrapping_add(1);
        self.size_kind = new;
        self.zone = Zone::Size;
        self.add_index = self.sizes.iter().position(|p| p.current).unwrap_or(0);
    }

    fn hold_preview(&mut self, next: Value, id: Option<String>, outs: &mut Vec<EditorOut>) {
        self.held = Some(std::mem::replace(&mut self.work, next));
        if let Some(id) = id { self.sel = id; }
        self.mode = Mode::Move;
        self.zone = Zone::Grid;
        self.size_kind = None;
        outs.push(EditorOut::Preview(self.work.clone()));
    }

    fn choose_option(&mut self, now: u64, theme: &Value, metrics: &Metrics, outs: &mut Vec<EditorOut>) {
        if self.zone == Zone::Add {
            if let Some(kind) = grid::ADDABLE.get(self.add_index).copied() {
                self.open_sizes(Some(kind), theme, metrics);
            }
            return;
        }
        let preset = self.sizes.get(self.add_index).copied();
        if preset.is_some_and(|p| p.fit == Fit::NoRoom) {
            self.say("No room for this size", now);
            return;
        }
        if let Some(kind) = self.size_kind {
            if let Some(preset) = preset {
                match dashboard_sizes::add(&self.work, kind, &preset) {
                    Ok((layout, id)) => self.hold_preview(layout, Some(id), outs),
                    Err(e) => self.turned_down(&e, now),
                }
            }
            return;
        }
        match preset {
            Some(Preset { current: true, .. }) => self.zone = Zone::Bar,
            Some(Preset { fit, size: (w, h), .. }) => {
                let (col, row) = if let Fit::Elsewhere { col, row } = fit { (Some(col), Some(row)) } else { (None, None) };
                match grid::change(&self.work, &self.sel, col, row, Some(w), Some(h)) {
                    Ok((next, _)) if col.is_some() => self.hold_preview(next, None, outs),
                    Ok((next, _)) => {
                        self.step(next, None, outs);
                        self.zone = Zone::Bar;
                    }
                    Err(e) => self.turned_down(&e, now),
                }
            }
            None if self.card().is_some() => {
                self.held = Some(self.work.clone());
                self.mode = Mode::Resize;
                self.zone = Zone::Grid;
            }
            None => {}
        }
    }

    fn leave_now(outs: &mut Vec<EditorOut>) {
        outs.push(EditorOut::Leave);
    }

    fn save_now(&self, outs: &mut Vec<EditorOut>) {
        if !self.can_save() { return; }
        if self.dirty() || self.start.is_new() {
            outs.push(EditorOut::Save(self.work.clone()));
        } else {
            Self::leave_now(outs);
        }
    }

    /// Back with nothing picked up: the question when something changed.
    fn back(&mut self, outs: &mut Vec<EditorOut>) {
        if self.close_options() { return; }
        if self.dirty() {
            self.ask_from = if self.mode == Mode::Select { self.zone } else { Zone::Grid };
            self.ghost = None;
            self.zone = Zone::Ask;
            self.ask_index = 0;
        } else {
            Self::leave_now(outs);
        }
    }

    fn answer(&mut self, i: usize, outs: &mut Vec<EditorOut>) {
        if i == 1 {
            Self::leave_now(outs);
        } else {
            self.zone = self.ask_from;
        }
    }

    fn close_options(&mut self) -> bool {
        match self.zone {
            Zone::Size => {
                if let Some(kind) = self.size_kind.take() {
                    self.zone = Zone::Add;
                    self.add_index = grid::ADDABLE.iter().position(|&k| k == kind).unwrap_or_default();
                } else {
                    self.zone = Zone::Bar;
                }
            }
            Zone::Add => self.zone = Zone::Bar,
            _ => return false,
        }
        true
    }

    fn press(&mut self, id: &str, now: u64, theme: &Value, metrics: &Metrics, outs: &mut Vec<EditorOut>) {
        match id {
            "add" => {
                self.zone = Zone::Add;
                self.add_index = 0;
            }
            "move" => {
                self.pick_up();
                self.zone = Zone::Grid;
            }
            "size" => self.open_sizes(None, theme, metrics),
            "remove" => self.remove_card(now, outs),
            "items" if self.arrangeable() => {
                self.zone = Zone::Items;
                outs.push(EditorOut::Items(self.sel.clone()));
            }
            "undo" => self.undo(outs),
            "cancel" => self.back(outs),
            "save" => self.save_now(outs),
            _ => {}
        }
    }

    fn next_enabled(&self, from: usize, d: i64) -> Option<usize> {
        let b = self.buttons();
        let mut i = from as i64 + d;
        while i >= 0 && (i as usize) < b.len() {
            if b[i as usize].enabled {
                return Some(i as usize);
            }
            i += d;
        }
        None
    }

    /// The enabled tool nearest to the selected card.
    fn nearest_button(&self) -> usize {
        let b = self.buttons();
        let Some(r) = self.sel_rect() else { return 0 };
        let cx = r.x as f32 + r.w as f32 / 2.0;
        b.iter().enumerate().filter(|(_, b)| b.enabled).min_by(|a, c| (a.1.frame.centre().0 - cx).abs().total_cmp(&(c.1.frame.centre().0 - cx).abs())).map_or(0, |(i, _)| i)
    }

    // ---- the keys
    pub fn key(&mut self, name: &str, dir: Option<Dir>, now: u64, theme: &Value, metrics: &Metrics) -> Vec<EditorOut> {
        let mut outs = Vec::new();
        if self.saving { return outs; }
        match self.zone {
            Zone::Ask => match (name, dir) {
                (_, Some(Dir::Left)) => self.ask_index = self.ask_index.saturating_sub(1),
                (_, Some(Dir::Right)) => self.ask_index = (self.ask_index + 1).min(ASK_ANSWERS.len() - 1),
                ("ok", _) => self.answer(self.ask_index, &mut outs),
                ("back", _) => self.answer(0, &mut outs),
                _ => {}
            },
            Zone::Add | Zone::Size => match (name, dir) {
                (_, Some(Dir::Up)) => self.add_index = self.add_index.saturating_sub(1),
                (_, Some(Dir::Down)) => self.add_index = (self.add_index + 1).min(self.options_len().saturating_sub(1)),
                ("ok", _) => self.choose_option(now, theme, metrics, &mut outs),
                ("back", _) | (_, Some(Dir::Left)) => { self.close_options(); }
                _ => {}
            },
            Zone::Bar => match (name, dir) {
                (_, Some(Dir::Left)) => {
                    if let Some(l) = self.next_enabled(self.bar_index, -1) {
                        self.bar_index = l;
                    }
                }
                (_, Some(Dir::Right)) => {
                    if let Some(r) = self.next_enabled(self.bar_index, 1) {
                        self.bar_index = r;
                    }
                }
                (_, Some(Dir::Down)) => self.zone = Zone::Grid,
                ("ok", _) => {
                    let b = &self.buttons()[self.bar_index];
                    if b.enabled { self.press(b.id, now, theme, metrics, &mut outs); }
                }
                ("back", _) => self.back(&mut outs),
                _ => {}
            },
            Zone::Grid => self.grid_key(name, dir, now, &mut outs),
            // the card has the keys (the model sends them there): a key that gets here is not one of its
            Zone::Items => {
                if name == "back" {
                    self.zone = Zone::Bar;
                }
            }
        }
        outs
    }

    /// The Menu key: the add list.
    pub fn menu(&mut self) {
        if !self.saving && self.mode == Mode::Select && matches!(self.zone, Zone::Grid | Zone::Bar) {
            self.bar_index = 0;
            self.zone = Zone::Add;
            self.add_index = 0;
        }
    }

    fn grid_key(&mut self, name: &str, dir: Option<Dir>, now: u64, outs: &mut Vec<EditorOut>) {
        if name == "menu" {
            self.menu();
            return;
        }
        let (dx, dy) = match dir {
            Some(Dir::Left) => (-1, 0),
            Some(Dir::Right) => (1, 0),
            Some(Dir::Up) => (0, -1),
            Some(Dir::Down) => (0, 1),
            None => (0, 0),
        };
        match self.mode {
            Mode::Select => match (name, dir) {
                ("ok", _) => {
                    self.zone = Zone::Bar;
                    self.bar_index = if self.card().is_some() { MOVE_BUTTON } else { 0 };
                }
                ("back", _) => self.back(outs),
                (_, Some(Dir::Up)) if !grid::has_beside(&self.work, &self.sel, -1) => {
                    self.zone = Zone::Bar;
                    self.bar_index = self.nearest_button();
                }
                (_, Some(_)) => {
                    if let Some(n) = grid::neighbour(&self.work, &self.sel, dx, dy) {
                        self.sel = n;
                    }
                }
                _ => {}
            },
            Mode::Move => match (name, dir) {
                ("back", _) => self.put_back(outs),
                (_, Some(_)) => {
                    self.ghost = None;
                    let r = grid::shift(&self.work, &self.sel, dx, dy);
                    self.change(r, now, outs);
                }
                ("ok", _) => self.put_down(),
                _ => {}
            },
            Mode::Resize => match (name, dir) {
                ("back", _) => self.put_back(outs),
                ("ok", _) => self.put_down(),
                (_, Some(_)) => {
                    let r = grid::resize(&self.work, &self.sel, dx, dy);
                    self.change(r, now, outs);
                }
                _ => {}
            },
        }
    }

    // ---- the pointer
    fn card_at(&self, x: f32, y: f32) -> Option<String> {
        grid::entries(&self.work).into_iter().rev().filter(|e| !e.hidden).find(|e| {
            let r = self.rect(e);
            x >= r.x as f32 && x < (r.x + r.w) as f32 && y >= r.y as f32 && y < (r.y + r.h) as f32
        }).map(|e| e.id)
    }

    /// The corner handle of the chosen card.
    fn corner_at(&self, x: f32, y: f32) -> bool {
        self.sel_rect().is_some_and(|r| {
            let (hx, hy) = ((r.x + r.w) as f32 - 52.0, (r.y + r.h) as f32 - 52.0);
            x >= hx && x < hx + 44.0 && y >= hy && y < hy + 44.0
        })
    }

    fn pointer_cell(&self, x: f32, y: f32) -> Option<Snap> {
        let a = self.area();
        if x < a.x as f32 || x >= (a.x + a.w) as f32 || y < a.y as f32 || y >= (a.y + a.h) as f32 {
            return None;
        }
        let (cw, ch) = grid::cell_size_in(&self.work, a);
        let gap = grid::grid(&self.work).1 as f64;
        let col = ((x as f64 - a.x as f64) / (cw + gap)).floor() as i64;
        let row = ((y as f64 - a.y as f64) / (ch + gap)).floor() as i64;
        if self.mode == Mode::Resize {
            let card = self.card()?;
            return Some((None, None, Some(col - card.col + 1), Some(row - card.row + 1)));
        }
        Some((Some(col), Some(row), None, None))
    }

    /// Click an open cell to place a held widget, or drag an existing widget.
    /// Hover never changes selection on the way to the toolbar.
    pub fn press_at(&mut self, x: f32, y: f32, focus: &mut Focus, now: u64, theme: &Value, metrics: &Metrics) -> Vec<EditorOut> {
        let mut outs = Vec::new();
        if self.saving { return outs; }
        let hit = focus.click(x, y);
        match self.zone {
            Zone::Ask => {
                if let Some(i) = hit.and_then(|id| id.0.checked_sub(ASK_ID)).filter(|&i| (i as usize) < ASK_ANSWERS.len()) {
                    self.answer(i as usize, &mut outs);
                }
                return outs;
            }
            Zone::Add | Zone::Size => {
                if let Some(i) = hit.and_then(|id| id.0.checked_sub(ADD_ID)).filter(|&i| (i as usize) < self.options_len()) {
                    self.add_index = i as usize;
                    self.choose_option(now, theme, metrics, &mut outs);
                } else {
                    self.close_options();
                }
                return outs;
            }
            Zone::Items => return outs,
            _ => {}
        }
        if let Some(i) = hit.and_then(|id| id.0.checked_sub(BAR_ID))
            && let Some(b) = self.buttons().get(i as usize).filter(|b| b.enabled)
        {
            self.zone = Zone::Bar;
            self.bar_index = i as usize;
            self.press(b.id, now, theme, metrics, &mut outs);
            return outs;
        }
        if y >= BAR_Y && y < BAR_Y + self.bar_h() { return outs; }
        if self.mode != Mode::Select {
            if let Some((col, row, span, rows)) = self.pointer_cell(x, y) {
                let same = self.card().is_some_and(|e| col.is_none_or(|c| e.col == c) && row.is_none_or(|r| e.row == r)
                    && span.is_none_or(|w| e.span == w) && rows.is_none_or(|h| e.rows == h));
                if same || self.change(grid::change(&self.work, &self.sel, col, row, span, rows), now, &mut outs) {
                    self.put_down();
                } else if self.mode == Mode::Move {
                    self.say("Choose an open spot", now);
                }
            }
            return outs;
        }
        self.zone = Zone::Grid;
        if self.card().is_some() && self.corner_at(x, y) {
            self.drag = Some(Drag { x, y, size: true, on: false, to: None });
        } else if let Some(id) = self.card_at(x, y) {
            self.sel = id;
            self.drag = Some(Drag { x, y, size: false, on: false, to: None });
        }
        outs
    }

    fn footprint(&self, to: Snap) -> Option<(Rect, bool)> {
        let mut e = self.card()?;
        e.col = to.0.unwrap_or(e.col);
        e.row = to.1.unwrap_or(e.row);
        e.span = to.2.unwrap_or(e.span);
        e.rows = to.3.unwrap_or(e.rows);
        if e.span < 1 || e.rows < 1 { return None; }
        Some((self.rect(&e), grid::fits(&self.work, &e) && !grid::overlaps(&self.work, &e)))
    }

    /// Show the pointer's proposed footprint without changing the layout.
    pub fn drag(&mut self, x: f32, y: f32) -> bool {
        let area = self.area();
        let to = if let Some(d) = &mut self.drag {
            let (dx, dy) = (x - d.x, y - d.y);
            if !d.on && dx.abs() + dy.abs() < 10.0 {
                return false;
            }
            d.on = true;
            let to = if d.size {
                grid::snap_resize(&self.work, area, &self.sel, dx, dy).map(|(s, r)| (None, None, Some(s), Some(r)))
            } else {
                grid::snap_move(&self.work, area, &self.sel, dx, dy).map(|(c, r)| (Some(c), Some(r), None, None))
            };
            d.to = to;
            to
        } else if self.mode != Mode::Select && self.zone == Zone::Grid {
            self.pointer_cell(x, y)
        } else {
            None
        };
        let ghost = to.and_then(|t| self.footprint(t));
        if self.ghost == ghost { return false; }
        self.ghost = ghost;
        true
    }

    pub fn cancel_pointer(&mut self) -> bool {
        self.drag = None;
        self.ghost.take().is_some()
    }

    pub fn release(&mut self, now: u64) -> Option<EditorOut> {
        let d = self.drag.take()?;
        self.ghost = None;
        let to = d.to.filter(|_| d.on)?;
        match grid::change(&self.work, &self.sel, to.0, to.1, to.2, to.3) {
            Ok((layout, _)) => {
                self.history.push(std::mem::replace(&mut self.work, layout));
                Some(EditorOut::Preview(self.work.clone()))
            }
            Err(e) => {
                self.turned_down(&e, now);
                None
            }
        }
    }

    /// Time passed: the toast and the refusal flash end.
    pub fn tick(&mut self, now: u64) -> bool {
        let mut changed = false;
        if !self.toast.is_empty() && now >= self.toast_until {
            self.toast.clear();
            changed = true;
        }
        if self.refused_until > 0 && now >= self.refused_until {
            self.refused_until = 0;
            changed = true;
        }
        changed
    }

    pub fn due_at(&self) -> Option<u64> {
        [if self.toast.is_empty() { None } else { Some(self.toast_until) }, if self.refused_until > 0 { Some(self.refused_until) } else { None }].into_iter().flatten().min()
    }

    pub fn refused(&self) -> bool {
        self.refused_until > 0
    }

    pub fn hint_words(&self) -> &'static str {
        if self.saving { return "Saving dashboard…"; }
        if !self.issue.is_empty() { return self.issue; }
        match (self.zone, self.mode) {
            (Zone::Add, _) => "Choose a widget",
            (Zone::Size, _) => "Choose a size",
            (Zone::Items, _) => "Arrange the items in this widget",
            (_, Mode::Move) => "Point and click, or use arrows then OK",
            (_, Mode::Resize) => "Adjust the highlighted corner",
            _ if self.start.is_new() && !self.has_widget() => "Choose Add to place your first widget",
            _ if self.start.guided() => "Move, Size or Remove. Finish when ready.",
            _ => "Select a widget to move or resize it",
        }
    }

    pub fn state(&self) -> Value {
        json!({"mode": self.mode_name(), "zone": self.zone_name(), "sel": self.sel, "dirty": self.dirty(), "toast": self.toast, "barIndex": self.bar_index, "addIndex": self.add_index,
               "history": self.history.len(), "askIndex": self.ask_index})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free() -> Value {
        json!({"safe": {"x": 64, "y": 54}, "grid": {"cols": 12, "gutter": 20, "rows": 16}, "statusBar": {"height": 60, "gap": 20, "left": [], "right": []},
            "widgets": [{"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 4, "rows": 4}, {"id": "home", "type": "homeassistant", "col": 0, "row": 4, "span": 4, "rows": 5, "title": "At home"},
                        {"id": "news", "type": "news", "col": 4, "row": 0, "span": 5, "rows": 9, "hidden": true}, {"id": "weather", "type": "weather", "col": 9, "row": 0, "span": 3, "rows": 9},
                        {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]})
    }

    fn keys(ed: &mut Editor, names: &[&str], now: u64) -> Vec<EditorOut> {
        let theme = lumatv_tokens::Tokens::shipped();
        let metrics = Metrics::nominal(ed.scale);
        let mut outs = Vec::new();
        for n in names {
            let dir = match *n {
                "up" => Some(Dir::Up),
                "down" => Some(Dir::Down),
                "left" => Some(Dir::Left),
                "right" => Some(Dir::Right),
                _ => None,
            };
            outs.extend(ed.key(n, dir, now, theme.json(), &metrics));
        }
        outs
    }

    fn numbers(l: &Value, id: &str) -> (i64, i64, i64, i64) {
        let e = grid::find(l, id).unwrap();
        (e.col, e.row, e.span, e.rows)
    }

    #[test]
    fn the_editor_moves_by_the_cell_and_relates_overlap_to_the_card_in_the_way() {
        let mut ed = Editor::new(&free(), "clock", 0, 1.0);
        keys(&mut ed, &["ok", "ok", "right", "down", "ok"], 100);
        assert_eq!(numbers(&ed.work, "clock"), (1, 0, 4, 4));
        assert_eq!(ed.toast, "At home is in the way");
        assert!(ed.refused());
        keys(&mut ed, &["ok", "right", "ok"], 200);
        assert_eq!(ed.zone, Zone::Size);
        let count = ed.options_len();
        keys(&mut ed, &vec!["down"; count], 300);
        assert_eq!(ed.option(ed.add_index).0, "Adjust by cell");
        keys(&mut ed, &["ok", "right", "ok"], 400);
        assert_eq!(numbers(&ed.work, "clock"), (1, 0, 5, 4));
        keys(&mut ed, &["ok"], 500);
        keys(&mut ed, &["right"; 8], 510);
        let outs = keys(&mut ed, &["ok"], 520);
        assert!(matches!(outs.last(), Some(EditorOut::Save(l)) if numbers(l, "clock") == (1, 0, 5, 4)));
    }

    #[test]
    fn dashboard_guide_remote_uses_real_add_size_move_remove_and_undo() {
        let original = free();
        let mut ed = Editor::draft(&original, &grid::dock_only(&original), Start::Replay, 0, 1.0);
        assert_eq!((ed.zone, ed.bar_index), (Zone::Bar, 0));
        assert!(!ed.has_widget() && !ed.can_save());
        assert!(keys(&mut ed, &["ok", "ok"], 100).is_empty());
        assert_eq!(ed.zone, Zone::Size);
        assert!(ed.option(0).0.contains("Clock"));
        let chosen = ed.sizes[0].size;
        keys(&mut ed, &["ok", "right", "ok"], 200);
        assert_eq!(ed.mode, Mode::Select);
        assert_eq!(numbers(&ed.work, "clock"), (1, 0, chosen.0, chosen.1));
        keys(&mut ed, &["ok", "right", "ok", "back", "right", "ok"], 300);
        assert!(!ed.has_widget());
        keys(&mut ed, &["right", "ok"], 400);
        assert!(ed.has_widget());
        assert!(keys(&mut ed, &["back", "ok"], 500).is_empty(), "Keep editing is the safe default");
        let outs = keys(&mut ed, &["back", "right", "ok"], 600);
        assert!(matches!(outs.as_slice(), [EditorOut::Leave]));
        assert_eq!(ed.orig, original);
    }

    #[test]
    fn dashboard_guide_pointer_selects_size_then_places_at_the_clicked_cell() {
        let original = free();
        let mut ed = Editor::draft(&original, &grid::dock_only(&original), Start::FirstRun, 0, 2.0);
        let mut f = Focus::new();
        let scope = Scope(83);
        f.open_modal(scope);
        let theme = lumatv_tokens::Tokens::shipped();
        let metrics = Metrics::nominal(ed.scale);
        let click = |ed: &mut Editor, f: &mut Focus, (x, y): (f32, f32)| {
            ed.register(f, scope);
            ed.press_at(x, y, f, 100, theme.json(), &metrics)
        };
        let at = ed.buttons()[0].frame.centre();
        click(&mut ed, &mut f, at);
        let at = ed.option_frame(0).centre();
        click(&mut ed, &mut f, at);
        assert_eq!(ed.zone, Zone::Size);
        assert!(!ed.has_widget());
        let at = ed.option_frame(0).centre();
        let chosen = ed.sizes[0].size;
        click(&mut ed, &mut f, at);
        let a = ed.area();
        let stride = grid::cell_size_in(&ed.work, a).0 as f32 + grid::grid(&ed.work).1 as f32;
        assert!(ed.drag(a.x as f32 + stride * 2.0 + 1.0, a.y as f32 + 1.0));
        let at = (a.x as f32 + stride * 2.0 + 1.0, a.y as f32 + 1.0);
        click(&mut ed, &mut f, at);
        assert_eq!(numbers(&ed.work, "clock"), (2, 0, chosen.0, chosen.1));
        assert_eq!(ed.mode, Mode::Select);
        assert!(ed.ghost.is_none());
    }

    #[test]
    fn a_drag_moves_a_card_by_whole_cells() {
        let mut ed = Editor::new(&free(), "clock", 0, 1.0);
        let mut f = Focus::new();
        let r = ed.sel_rect().unwrap();
        let (x, y) = (r.x as f32 + 40.0, r.y as f32 + 40.0);
        let theme = lumatv_tokens::Tokens::shipped();
        ed.press_at(x, y, &mut f, 0, theme.json(), &Metrics::nominal(1.0));
        let (cw, _) = grid::cell_size(&ed.work);
        assert!(ed.drag(x + cw as f32 + 20.0, y));
        assert!(ed.ghost.is_some_and(|g| g.1));
        assert!(matches!(ed.release(10), Some(EditorOut::Preview(_))));
        assert_eq!(numbers(&ed.work, "clock").0, 1);
    }
}
