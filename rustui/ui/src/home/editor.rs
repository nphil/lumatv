//! The layout editor (`LayoutEdit.qml`'s logic): Home's cards moved, sized, added and removed on the grid with the D-pad or the
//! pointer, Home drawing the layout being edited as it changes (`EditorOut::Preview`), so what is seen is what would be saved. On the
//! grid the arrows choose a card and OK picks it up; picked up, the arrows move it a cell and OK turns to its size (the arrows then move
//! its bottom right corner) and OK again puts it down; Back puts it back. Up from the top card, or the Menu key, goes to the bar; Down
//! from the bar returns. Back with nothing picked up leaves, asking first if anything changed. Nothing here writes a file: it asks
//! (`Save`, `Leave`) and Home does it.

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
/// The bar's height, the add list's width and the row at 100 % text; each grows with the text size.
pub const BAR_H: f32 = 72.0;
pub const ADD_W: f32 = 380.0;

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
    /// "Save changes?": Keep editing, Save, Discard.
    Ask,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditorOut {
    Preview(Value),
    Save(Value),
    Leave(Value),
}

pub struct Button {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    pub enabled: bool,
    pub primary: bool,
    pub frame: Frame,
}

pub const ASK_ANSWERS: [(&str, &str, bool); 3] = [("edit-keep", "Keep editing", false), ("edit-save", "Save", false), ("edit-discard", "Discard", true)];

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
            drag: None,
            ghost: None,
            scale: scale.max(1.0),
        }
    }

    pub fn bar_h(&self) -> f32 {
        (BAR_H * self.scale).round()
    }

    pub fn dirty(&self) -> bool {
        self.work != self.base
    }

    pub fn card(&self) -> Option<Entry> {
        grid::find(&self.work, &self.sel)
    }

    pub fn sel_rect(&self) -> Option<Rect> {
        self.card().map(|c| grid::rect_of(&self.work, &c))
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
            Zone::Ask => "ask",
        }
    }

    /// The bar's buttons, right-aligned in the bar, with their frames on the screen.
    pub fn buttons(&self) -> Vec<Button> {
        let specs: [(&'static str, &'static str, &'static str, bool, bool, f32); 5] = [
            ("add", "Add", "plus", true, false, 150.0),
            ("remove", "Remove", "trash-2", self.can_remove(), false, 196.0),
            ("undo", "Undo", "undo-2", !self.history.is_empty(), false, 164.0),
            ("cancel", "Cancel", "x", true, false, 184.0),
            ("save", "Save", "check", true, true, 150.0),
        ];
        let k = self.scale;
        let gap = 12.0;
        let total: f32 = specs.iter().map(|s| s.5 * k).sum::<f32>() + gap * 4.0;
        let mut x = 64.0 + 1792.0 - 16.0 - total;
        let bh = (56.0 * k).round();
        let y = BAR_Y + ((self.bar_h() - bh) / 2.0).round();
        specs
            .iter()
            .map(|s| {
                let b = Button { id: s.0, label: s.1, icon: s.2, enabled: s.3, primary: s.4, frame: Frame::new(x.round(), y, (s.5 * k).round(), bh) };
                x += s.5 * k + gap;
                b
            })
            .collect()
    }

    fn add_row_h(&self) -> f32 {
        (80.0 * self.scale).round()
    }

    /// The add list's frame under the Add button.
    pub fn add_frame(&self) -> Frame {
        let b = self.buttons();
        let n = grid::ADDABLE.len() as f32;
        Frame::new(b[0].frame.x, BAR_Y + self.bar_h() + 8.0, (ADD_W * self.scale).round(), n * self.add_row_h() + 24.0)
    }

    pub fn add_rows(&self) -> Vec<(&'static str, &'static str, &'static str, bool, Frame)> {
        let f = self.add_frame();
        let rh = self.add_row_h();
        grid::ADDABLE
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let t = grid::type_of(name).expect("every addable type is a type");
                (t.id, t.title, t.icon, grid::hidden_of(&self.work, name), Frame::new(f.x + 8.0, f.y + 12.0 + i as f32 * rh, f.w - 16.0, rh))
            })
            .collect()
    }

    /// The question's frame and its answers' frames.
    pub fn ask_frames(&self) -> (Frame, Vec<Frame>) {
        let k = self.scale;
        let (w, h) = ((760.0 * k).min(1792.0).round(), (236.0 * k).round());
        let f = Frame::new(((1920.0 - w) / 2.0).round(), ((1080.0 - h) / 2.0).round(), w, h);
        let bh = (64.0 * k).round();
        let bw = ((w - 32.0 * 2.0 - 12.0 * 2.0) / 3.0).floor();
        let answers = (0..3).map(|i| Frame::new(f.x + 32.0 + i as f32 * (bw + 12.0), f.y + h - 32.0 - bh, bw, bh)).collect();
        (f, answers)
    }

    /// The bar, the add list and the question as focusables of `scope` (for the pointer's cue and click); the cards are chosen by
    /// their own rule.
    pub fn register(&self, focus: &mut Focus, scope: Scope) {
        for (i, b) in self.buttons().iter().enumerate() {
            let id = Id(BAR_ID + i as u32);
            focus.register(id, b.frame, BAR, scope);
            focus.set_enabled(id, b.enabled);
        }
        for i in 0..grid::ADDABLE.len() {
            let id = Id(ADD_ID + i as u32);
            if self.zone == Zone::Add {
                focus.register(id, self.add_rows()[i].4, ADD, scope);
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
            Zone::Add => Some(Id(ADD_ID + self.add_index as u32)),
            Zone::Ask => Some(Id(ASK_ID + self.ask_index as u32)),
            Zone::Grid => None,
        };
        match current {
            Some(id) => {
                focus.focus(id);
            }
            None => focus.clear(),
        }
    }

    pub fn unregister(&self, focus: &mut Focus) {
        for i in 0..5 {
            focus.unregister(Id(BAR_ID + i));
        }
        for i in 0..grid::ADDABLE.len() as u32 {
            focus.unregister(Id(ADD_ID + i));
        }
        for i in 0..3 {
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
    }

    fn put_back(&mut self, outs: &mut Vec<EditorOut>) {
        if let Some(h) = self.held.take() {
            self.work = h;
            outs.push(EditorOut::Preview(self.work.clone()));
        }
        self.mode = Mode::Select;
    }

    fn undo(&mut self, outs: &mut Vec<EditorOut>) {
        let Some(prev) = self.history.pop() else { return };
        self.work = prev;
        if grid::find(&self.work, &self.sel).is_none() {
            self.sel = grid::order(&self.work).first().cloned().or_else(|| grid::dock_id(&self.work)).unwrap_or_default();
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

    fn add_card(&mut self, kind: &str, now: u64, outs: &mut Vec<EditorOut>) {
        self.zone = Zone::Grid;
        match grid::add(&self.work, kind) {
            Ok((layout, id)) => {
                // placed in the first free place and picked up, so it can be put where it is wanted; Back takes it away again
                self.held = Some(self.work.clone());
                self.work = layout;
                self.sel = id;
                self.mode = Mode::Move;
                outs.push(EditorOut::Preview(self.work.clone()));
            }
            Err(e) => self.turned_down(&e, now),
        }
    }

    fn leave_now(&self, outs: &mut Vec<EditorOut>) {
        outs.push(EditorOut::Leave(self.orig.clone()));
    }

    fn save_now(&self, outs: &mut Vec<EditorOut>) {
        if self.dirty() {
            outs.push(EditorOut::Save(self.work.clone()));
        } else {
            self.leave_now(outs);
        }
    }

    /// Back with nothing picked up: the question when something changed.
    fn back(&mut self, outs: &mut Vec<EditorOut>) {
        if self.zone == Zone::Add {
            self.zone = Zone::Bar;
            return;
        }
        if self.dirty() {
            self.zone = Zone::Ask;
            self.ask_index = 0;
        } else {
            self.leave_now(outs);
        }
    }

    fn answer(&mut self, i: usize, outs: &mut Vec<EditorOut>) {
        match ASK_ANSWERS.get(i).map(|a| a.0) {
            Some("edit-save") => self.save_now(outs),
            Some("edit-discard") => self.leave_now(outs),
            _ => self.zone = Zone::Grid,
        }
    }

    fn press(&mut self, id: &str, now: u64, outs: &mut Vec<EditorOut>) {
        match id {
            "add" => {
                self.zone = Zone::Add;
                self.add_index = 0;
            }
            "remove" => self.remove_card(now, outs),
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

    /// The button over the selected card.
    fn nearest_button(&self) -> usize {
        let b = self.buttons();
        let Some(r) = self.sel_rect() else { return b.len() - 1 };
        let cx = r.x as f32 + r.w as f32 / 2.0;
        b.iter().enumerate().min_by(|a, c| (a.1.frame.centre().0 - cx).abs().total_cmp(&(c.1.frame.centre().0 - cx).abs())).map_or(0, |(i, _)| i)
    }

    // ---- the keys
    pub fn key(&mut self, name: &str, dir: Option<Dir>, _focus: &mut Focus, now: u64) -> Vec<EditorOut> {
        let mut outs = Vec::new();
        match self.zone {
            Zone::Ask => match (name, dir) {
                (_, Some(Dir::Left)) => self.ask_index = self.ask_index.saturating_sub(1),
                (_, Some(Dir::Right)) => self.ask_index = (self.ask_index + 1).min(2),
                ("ok", _) => self.answer(self.ask_index, &mut outs),
                ("back", _) => self.answer(0, &mut outs),
                _ => {}
            },
            Zone::Add => match (name, dir) {
                (_, Some(Dir::Up)) => self.add_index = self.add_index.saturating_sub(1),
                (_, Some(Dir::Down)) => self.add_index = (self.add_index + 1).min(grid::ADDABLE.len() - 1),
                ("ok", _) => self.add_card(grid::ADDABLE[self.add_index], now, &mut outs),
                ("back", _) | (_, Some(Dir::Left)) => self.zone = Zone::Bar,
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
                    let id = self.buttons()[self.bar_index].id;
                    self.press(id, now, &mut outs);
                }
                ("back", _) => self.back(&mut outs),
                _ => {}
            },
            Zone::Grid => self.grid_key(name, dir, now, &mut outs),
        }
        outs
    }

    /// The Menu key: the add list.
    pub fn menu(&mut self) {
        if self.mode == Mode::Select {
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
                ("ok", _) => self.pick_up(),
                ("back", _) => self.back(outs),
                (_, Some(Dir::Up)) if !grid::has_beside(&self.work, &self.sel, -1) => {
                    self.zone = Zone::Bar;
                    self.bar_index = self.nearest_button();
                    if !self.buttons()[self.bar_index].enabled
                        && let Some(n) = self.next_enabled(self.bar_index, 1)
                    {
                        self.bar_index = n;
                    }
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
                ("ok", _) => self.mode = Mode::Resize,
                (_, Some(_)) => {
                    let r = grid::shift(&self.work, &self.sel, dx, dy);
                    self.change(r, now, outs);
                }
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
            let r = grid::rect_of(&self.work, e);
            x >= r.x as f32 && x < (r.x + r.w) as f32 && y >= r.y as f32 && y < (r.y + r.h) as f32
        }).map(|e| e.id)
    }

    /// The corner handle of the chosen card.
    fn corner_at(&self, x: f32, y: f32) -> bool {
        self.sel_rect().is_some_and(|r| {
            let (hx, hy) = ((r.x + r.w) as f32 - 30.0, (r.y + r.h) as f32 - 30.0);
            x >= hx && x < hx + 44.0 && y >= hy && y < hy + 44.0
        })
    }

    /// A press of the pointer: on a bar button, an add row or an answer it is that; on a card it chooses it and starts a drag.
    pub fn press_at(&mut self, x: f32, y: f32, focus: &mut Focus, now: u64) -> Vec<EditorOut> {
        let mut outs = Vec::new();
        if let Some(id) = focus.click(x, y) {
            if let Some(i) = id.0.checked_sub(BAR_ID).filter(|i| *i < 5) {
                self.zone = Zone::Bar;
                self.bar_index = i as usize;
                let bid = self.buttons()[i as usize].id;
                self.press(bid, now, &mut outs);
                return outs;
            }
            if let Some(i) = id.0.checked_sub(ADD_ID).filter(|i| (*i as usize) < grid::ADDABLE.len()) {
                self.add_index = i as usize;
                self.add_card(grid::ADDABLE[i as usize], now, &mut outs);
                return outs;
            }
            if let Some(i) = id.0.checked_sub(ASK_ID).filter(|i| *i < 3) {
                self.answer(i as usize, &mut outs);
                return outs;
            }
        }
        if self.zone == Zone::Ask {
            return outs;
        }
        if self.zone == Zone::Add {
            self.zone = Zone::Bar;
            return outs;
        }
        if self.card().is_some() && self.corner_at(x, y) {
            self.drag = Some(Drag { x, y, size: true, on: false, to: None });
            return outs;
        }
        let Some(id) = self.card_at(x, y) else { return outs };
        if self.mode != Mode::Select {
            self.put_down();
        }
        self.sel = id;
        self.zone = Zone::Grid;
        self.drag = Some(Drag { x, y, size: false, on: false, to: None });
        outs
    }

    /// The pointer moved: a drag snaps the card (or its corner) to whole cells; otherwise the card under it is chosen.
    pub fn drag(&mut self, x: f32, y: f32) -> bool {
        if let Some(d) = &mut self.drag {
            let (dx, dy) = (x - d.x, y - d.y);
            if !d.on && dx.abs() + dy.abs() < 10.0 {
                return false;
            }
            d.on = true;
            let to = if d.size {
                grid::snap_resize(&self.work, &self.sel, dx, dy).map(|(s, r)| (None, None, Some(s), Some(r)))
            } else {
                grid::snap_move(&self.work, &self.sel, dx, dy).map(|(c, r)| (Some(c), Some(r), None, None))
            };
            d.to = to;
            self.ghost = to.and_then(|t| {
                let c = self.card()?;
                let next = grid::change(&self.work, &self.sel, t.0, t.1, t.2, t.3);
                let ok = matches!(next, Ok(_) | Err(Refusal::Edge));
                let e = c.with_numbers(t.0.unwrap_or(c.col), t.1.unwrap_or(c.row), t.2.unwrap_or(c.span), t.3.unwrap_or(c.rows));
                Some((grid::rect_of(&self.work, &e), ok))
            });
            return true;
        }
        if self.mode == Mode::Select
            && let Some(id) = self.card_at(x, y)
            && id != self.sel
        {
            self.sel = id;
            self.zone = Zone::Grid;
            return true;
        }
        false
    }

    pub fn release(&mut self, _x: f32, _y: f32, _focus: &mut Focus) -> Option<EditorOut> {
        let d = self.drag.take()?;
        self.ghost = None;
        let to = d.to.filter(|_| d.on)?;
        match grid::change(&self.work, &self.sel, to.0, to.1, to.2, to.3) {
            Ok((layout, _)) => {
                let mut outs = Vec::new();
                self.step(layout, None, &mut outs);
                outs.pop()
            }
            Err(e) => {
                self.turned_down(&e, 0);
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

    /// The words beside the heading: what OK does now.
    pub fn hint_words(&self) -> String {
        match (self.mode, self.card()) {
            (Mode::Select, _) => "OK moves".into(),
            (Mode::Move, Some(c)) => format!("{} \u{d7} {} \u{b7} OK sizes \u{b7} Back undoes", c.span, c.rows),
            (Mode::Resize, Some(c)) => format!("{} \u{d7} {} \u{b7} OK done \u{b7} Back undoes", c.span, c.rows),
            (Mode::Move, None) => "OK sizes \u{b7} Back undoes".into(),
            (Mode::Resize, None) => "OK done \u{b7} Back undoes".into(),
        }
    }

    pub fn state(&self) -> Value {
        json!({"mode": self.mode_name(), "zone": self.zone_name(), "sel": self.sel, "dirty": self.dirty(), "toast": self.toast, "barIndex": self.bar_index, "addIndex": self.add_index,
               "history": self.history.len(), "askIndex": self.ask_index})
    }
}

impl Entry {
    pub(crate) fn with_numbers(&self, col: i64, row: i64, span: i64, rows: i64) -> Entry {
        Entry { col, row, span, rows, ..self.clone() }
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
        let mut f = Focus::new();
        let mut outs = Vec::new();
        for n in names {
            let dir = match *n {
                "up" => Some(Dir::Up),
                "down" => Some(Dir::Down),
                "left" => Some(Dir::Left),
                "right" => Some(Dir::Right),
                _ => None,
            };
            outs.extend(ed.key(n, dir, &mut f, now));
        }
        outs
    }

    fn numbers(l: &Value, id: &str) -> (i64, i64, i64, i64) {
        let e = grid::find(l, id).unwrap();
        (e.col, e.row, e.span, e.rows)
    }

    #[test]
    fn the_editor_picks_up_a_card_moves_it_by_the_cell_turns_a_move_down_and_saves() {
        let mut ed = Editor::new(&free(), "clock", 0, 1.0);
        assert_eq!((ed.mode, ed.zone, ed.sel.as_str()), (Mode::Select, Zone::Grid, "clock"));
        keys(&mut ed, &["ok", "right", "down"], 100);
        assert_eq!(numbers(&ed.work, "clock"), (1, 0, 4, 4));
        assert_eq!(ed.toast, "At home is in the way");
        assert!(ed.refused());
        keys(&mut ed, &["ok", "right", "ok"], 200);
        assert_eq!(numbers(&ed.work, "clock"), (1, 0, 5, 4));
        assert_eq!(ed.mode, Mode::Select);
        let outs = keys(&mut ed, &["up", "right", "right", "right", "right", "ok"], 300);
        assert_eq!(ed.zone, Zone::Bar);
        assert!(matches!(outs.last(), Some(EditorOut::Save(l)) if numbers(l, "clock") == (1, 0, 5, 4)));
    }

    #[test]
    fn back_asks_when_something_changed_and_the_bar_removes_undoes_and_adds() {
        let mut ed = Editor::new(&free(), "weather", 0, 1.0);
        keys(&mut ed, &["ok", "left", "ok", "ok", "back"], 100);
        assert_eq!(ed.zone, Zone::Ask);
        assert_eq!(numbers(&ed.work, "weather").0, 8);
        keys(&mut ed, &["back"], 200);
        assert_eq!(ed.zone, Zone::Grid, "Keep editing is what Back is");
        keys(&mut ed, &["back"], 300);
        let outs = keys(&mut ed, &["right", "right", "ok"], 400);
        assert!(matches!(outs.last(), Some(EditorOut::Leave(_))));
        let mut ed = Editor::new(&free(), "clock", 0, 1.0);
        keys(&mut ed, &["up", "right", "ok"], 500);
        assert!(grid::find(&ed.work, "clock").is_none(), "Remove took the clock away");
        keys(&mut ed, &["right", "ok"], 600);
        assert!(grid::find(&ed.work, "clock").is_some(), "Undo brought it back");
        keys(&mut ed, &["left", "left", "ok"], 700);
        assert_eq!(ed.zone, Zone::Add);
        let outs = keys(&mut ed, &["back", "right", "right", "ok"], 800);
        assert!(matches!(outs.last(), Some(EditorOut::Leave(_))), "Cancel with nothing changed asked or stayed");
    }

    #[test]
    fn a_card_added_from_the_list_is_picked_up_where_it_fits() {
        let mut ed = Editor::new(&free(), "clock", 0, 1.0);
        keys(&mut ed, &["up", "ok", "down", "down", "down", "ok"], 100);
        assert_eq!((ed.zone, ed.mode, ed.sel.as_str()), (Zone::Grid, Mode::Move, "news"));
        assert!(!grid::find(&ed.work, "news").unwrap().hidden);
        keys(&mut ed, &["back"], 200);
        assert!(grid::find(&ed.work, "news").unwrap().hidden, "Back takes it away again");
    }

    #[test]
    fn a_drag_moves_a_card_by_whole_cells() {
        let mut ed = Editor::new(&free(), "clock", 0, 1.0);
        let mut f = Focus::new();
        let r = ed.sel_rect().unwrap();
        let (x, y) = (r.x as f32 + 40.0, r.y as f32 + 40.0);
        ed.press_at(x, y, &mut f, 0);
        let (cw, _) = grid::cell_size(&ed.work);
        assert!(ed.drag(x + cw as f32 + 20.0, y));
        assert!(ed.ghost.is_some_and(|g| g.1));
        let out = ed.release(x + cw as f32 + 20.0, y, &mut f);
        assert!(matches!(out, Some(EditorOut::Preview(_))));
        assert_eq!(numbers(&ed.work, "clock").0, 1);
    }
}
