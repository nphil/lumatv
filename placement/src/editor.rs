//! The layout editor's controller: which item is chosen, what is in the hand, what a key or a pointer does, what is saved. A toolkit draws
//! [`View`] and hands the keys and the pointer to the controller, so the rules (the steps and the stops, the refusals, the corner a volume bar
//! turns, what Reset does) are the same in every toolkit and tested once.
//!
//! The controller never saves anything itself: a drop or a Reset asks the caller, with [`Effect::Save`], to write the whole `layout.overlays` map
//! to the theme (the caller routes the key to the profile or the shared file as it does for any theme key), and the controller goes on with
//! the map it asked for. Time is the caller's too (`now_ms`): a held key is told from a tap by the gap between presses, and a notice goes by
//! itself.

use crate::model::{
    Frame, H, Layout, Placement, Point, Rect, Registry, SUBS, Size, Sizes, User, W,
};
use crate::motion::{Dir, Press};
use serde::Serialize;
use serde_json::Value;

/// A press of the same arrow closer than this to the last is a held key (the remote repeats every 40 ms after a pause).
const HELD_GAP_MS: u64 = 220;
/// A notice stays this long.
const NOTICE_MS: u64 = 2600;
/// A pointer moves this far (the two axes added) before it takes the item it is on: a click is not a drag.
const SLOP: f64 = 12.0;

pub const HEADING: &str = "Layout of pop-ups";
const HINT_MOVE: &str = "Arrows move it  \u{b7}  OK Put down  \u{b7}  Back Put back";
const HINT_BAR: &str = "\u{2190} \u{2192} Choose  \u{b7}  OK Do it  \u{b7}  Up Items";
const HINT_ITEMS: &str =
    "\u{2190} \u{2192} Choose  \u{b7}  OK Pick up  \u{b7}  Down Buttons  \u{b7}  Back Done";
const BUTTONS: [&str; 3] = ["Reset this", "Reset all", "Done"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Ok,
    Back,
}

impl Key {
    pub fn parse(name: &str) -> Option<Key> {
        match name {
            "ok" => Some(Key::Ok),
            "back" => Some(Key::Back),
            other => Dir::parse(other).map(|d| match d {
                Dir::Left => Key::Left,
                Dir::Right => Key::Right,
                Dir::Up => Key::Up,
                Dir::Down => Key::Down,
            }),
        }
    }

    fn dir(self) -> Option<Dir> {
        match self {
            Key::Left => Some(Dir::Left),
            Key::Right => Some(Dir::Right),
            Key::Up => Some(Dir::Up),
            Key::Down => Some(Dir::Down),
            _ => None,
        }
    }
}

/// What the caller has to do after a key or a pointer.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Write this as `layout.overlays` in the theme: the whole map.
    Save(Value),
    /// Leave the editor.
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Browse,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Items,
    Bar,
}

pub struct Editor {
    reg: Registry,
    user: User,
    frame: Frame,
    sizes: Sizes,
    sel: &'static str,
    mode: Mode,
    focus: Focus,
    bar: usize,
    /// The placement of the item in the hand, and where it was.
    work: Option<Placement>,
    held: Option<Placement>,
    /// What the item covered where it was picked up: a move may leave an overlap it started with, never make one.
    held_foot: Option<Rect>,
    notice: String,
    notice_at: u64,
    blocker: Option<&'static str>,
    run: u32,
    run_dir: Option<Dir>,
    run_at: u64,
    grab: (f64, f64),
    down: (f64, f64),
    dragging: bool,
}

impl Editor {
    /// An editor on what the owner has placed, the frame Home gave and the sizes the toolkit measured for the stand-ins it draws (the registry's
    /// samples where it measured none). The editor shows an app's picture behind: the volume bar on the bottom edge is at its lifted height.
    pub fn new(reg: Registry, user: User, frame: Frame, sizes: Sizes) -> Editor {
        let sel = reg.items().first().map_or("", |it| it.id);
        Editor {
            reg,
            user,
            frame,
            sizes,
            sel,
            mode: Mode::Browse,
            focus: Focus::Items,
            bar: 2,
            work: None,
            held: None,
            held_foot: None,
            notice: String::new(),
            notice_at: 0,
            blocker: None,
            run: 0,
            run_dir: None,
            run_at: 0,
            grab: (0.0, 0.0),
            down: (0.0, 0.0),
            dragging: false,
        }
    }

    /// Opens on `id` (the first item when it is not one), with nothing in the hand.
    pub fn start(&mut self, id: Option<&str>) {
        self.sel = id
            .and_then(|id| self.reg.item(id))
            .or_else(|| self.reg.items().first())
            .map_or("", |it| it.id);
        self.mode = Mode::Browse;
        self.focus = Focus::Items;
        self.bar = 2;
        self.work = None;
        self.held = None;
        self.held_foot = None;
        self.dragging = false;
        self.clear_notice();
    }

    /// The map changed under the editor (another profile took over); a move in the hand is put back.
    pub fn set_user(&mut self, user: User) {
        self.user = user;
        if self.mode == Mode::Move {
            self.put_back();
        }
    }

    pub fn set_frame(&mut self, frame: Frame) {
        self.frame = frame;
    }

    pub fn set_sizes(&mut self, sizes: Sizes) {
        self.sizes = sizes;
    }

    pub fn user(&self) -> &User {
        &self.user
    }

    pub fn selected(&self) -> &'static str {
        self.sel
    }

    pub fn in_hand(&self) -> bool {
        self.mode == Mode::Move
    }

    fn layout(&self) -> Layout {
        self.reg.layout_all(
            &self.user,
            &self.frame,
            &self.sizes,
            self.work.map(|p| (self.sel, p)),
            true,
        )
    }

    fn title(&self, id: &str) -> &'static str {
        self.reg.item(id).map_or("", |it| it.title)
    }

    fn tell(&mut self, text: String, who: Option<&'static str>, now: u64) {
        self.notice = text;
        self.notice_at = now;
        self.blocker = who;
    }

    fn clear_notice(&mut self) {
        self.notice.clear();
        self.blocker = None;
    }

    /// What an item covers: the mark also covers the places it visits.
    fn footprint(&self, id: &str, p: Placement, r: Rect) -> Rect {
        match self.reg.item(id) {
            Some(it) if it.drift > 0 => {
                let u = self.reg.with_placement(&self.user, id, p);
                self.reg
                    .drift_box(id, &u, &self.frame, Size::new(r.w, r.h))
                    .unwrap_or(r)
            }
            _ => r,
        }
    }

    fn pick_up(&mut self) {
        let Some(held) = self
            .reg
            .placement_of(self.sel, &self.user)
            .map(|p| p.placement)
        else {
            return;
        };
        let foot = self
            .layout()
            .rect(self.sel)
            .map(|r| self.footprint(self.sel, held, r));
        self.held = Some(held);
        self.held_foot = foot;
        self.work = Some(held);
        self.mode = Mode::Move;
        self.clear_notice();
    }

    fn put_back(&mut self) {
        self.work = None;
        self.held = None;
        self.held_foot = None;
        self.dragging = false;
        self.mode = Mode::Browse;
        self.clear_notice();
    }

    /// The rule (nothing on another item) was kept at every step, so what is in the hand is put down where it is; where it did not move
    /// nothing is written.
    fn put_down(&mut self) -> Vec<Effect> {
        let (next, from) = (self.work.take(), self.held.take());
        self.held_foot = None;
        self.mode = Mode::Browse;
        match (next, from) {
            (Some(n), Some(f)) if !n.same(&f) => {
                self.user = self.reg.with_placement(&self.user, self.sel, n);
                vec![Effect::Save(self.user.to_json())]
            }
            _ => Vec::new(),
        }
    }

    /// The item in the hand at `next`, unless that would put it on another item.
    fn try_work(&mut self, next: Placement, now: u64) {
        let l = self.reg.layout_all(
            &self.user,
            &self.frame,
            &self.sizes,
            Some((self.sel, next)),
            true,
        );
        let Some(r) = l.rect(self.sel) else { return };
        let foot = self.footprint(self.sel, next, r);
        if let Some(who) = self
            .reg
            .blocker(&l, self.sel, &foot, self.held_foot.as_ref())
        {
            let text = format!("{} is in the way", self.title(who));
            self.tell(text, Some(who), now);
            return;
        }
        self.work = Some(next);
        self.clear_notice();
    }

    fn move_by(&mut self, dir: Dir, now: u64) {
        self.run = if self.run_dir == Some(dir) && now.saturating_sub(self.run_at) < HELD_GAP_MS {
            self.run + 1
        } else {
            0
        };
        self.run_dir = Some(dir);
        self.run_at = now;
        let Some(work) = self.work else { return };
        let press = Press {
            repeat: self.run > 0,
            run: self.run,
        };
        if let Some(step) = self
            .reg
            .step(self.sel, &work, dir, &self.sizes, &self.frame, press)
        {
            if step.moved {
                self.try_work(step.placement, now);
            }
        }
    }

    fn choose(&mut self, delta: i32) {
        let items = self.reg.items();
        if items.is_empty() {
            return;
        }
        let i = items.iter().position(|it| it.id == self.sel).unwrap_or(0) as i32;
        let j = (i + delta).clamp(0, items.len() as i32 - 1) as usize;
        self.sel = items[j].id;
        self.clear_notice();
    }

    fn reset_this(&mut self, now: u64) -> Vec<Effect> {
        if !self.user.has(self.sel) {
            return Vec::new();
        }
        self.user = self.user.without(self.sel);
        self.tell("Back to where it was".into(), None, now);
        vec![Effect::Save(self.user.to_json())]
    }

    fn reset_all(&mut self, now: u64) -> Vec<Effect> {
        if self.user.is_empty() {
            return Vec::new();
        }
        self.user = User::default();
        self.tell("Everything back to where it was".into(), None, now);
        vec![Effect::Save(self.user.to_json())]
    }

    /// One of the three buttons (0 Reset this, 1 Reset all, 2 Done): what OK does on it, and what a pointer that presses it does.
    pub fn press_button(&mut self, index: usize, now: u64) -> Vec<Effect> {
        if self.mode == Mode::Move {
            return Vec::new();
        }
        self.focus = Focus::Bar;
        self.bar = index.min(2);
        match self.bar {
            0 => self.reset_this(now),
            1 => self.reset_all(now),
            _ => vec![Effect::Close],
        }
    }

    /// A key of the remote. Always taken: the editor is the whole screen.
    pub fn key(&mut self, key: Key, now: u64) -> Vec<Effect> {
        if self.mode == Mode::Move {
            return match key {
                Key::Ok => self.put_down(),
                Key::Back => {
                    self.put_back();
                    Vec::new()
                }
                other => {
                    if let Some(d) = other.dir() {
                        self.move_by(d, now);
                    }
                    Vec::new()
                }
            };
        }
        if self.focus == Focus::Bar {
            match key {
                Key::Left => self.bar = self.bar.saturating_sub(1),
                Key::Right => self.bar = (self.bar + 1).min(2),
                Key::Up | Key::Back => self.focus = Focus::Items,
                Key::Ok => return self.press_button(self.bar, now),
                Key::Down => {}
            }
            return Vec::new();
        }
        match key {
            Key::Left | Key::Up => self.choose(-1),
            Key::Right => self.choose(1),
            Key::Down => self.focus = Focus::Bar,
            Key::Ok => self.pick_up(),
            Key::Back => return vec![Effect::Close],
        }
        Vec::new()
    }

    // ---- a pointer: an item is taken where it is pressed, follows it and is put down where it lets go. Coordinates are the screen's (1920 by
    // 1080), so the toolkit does not translate them into an item's own.

    pub fn pointer_down(&mut self, id: &str, x: f64, y: f64) {
        if self.mode == Mode::Move && !self.dragging {
            return;
        }
        let Some(it) = self.reg.item(id) else { return };
        let Some(r) = self.layout().rect(it.id) else {
            return;
        };
        self.focus = Focus::Items;
        self.sel = it.id;
        self.grab = (x - f64::from(r.x), y - f64::from(r.y));
        self.down = (x, y);
        self.dragging = false;
    }

    pub fn pointer_move(&mut self, x: f64, y: f64, pressed: bool, now: u64) {
        if !pressed || self.sel.is_empty() {
            return;
        }
        let Some(r) = self.layout().rect(self.sel) else {
            return;
        };
        if !self.dragging {
            if (x - self.down.0).abs() + (y - self.down.1).abs() < SLOP {
                return;
            }
            self.dragging = true;
            if self.mode != Mode::Move {
                self.pick_up();
            }
        }
        let Some(work) = self.work else { return };
        let (cx, cy) = (
            x - self.grab.0 + f64::from(r.w) / 2.0,
            y - self.grab.1 + f64::from(r.h) / 2.0,
        );
        if let Some(next) = self
            .reg
            .at_point(self.sel, cx, cy, &self.sizes, &self.frame, &work)
        {
            self.try_work(next, now);
        }
    }

    pub fn pointer_up(&mut self) -> Vec<Effect> {
        if self.dragging {
            self.dragging = false;
            if self.mode == Mode::Move {
                return self.put_down();
            }
        }
        Vec::new()
    }

    /// Where a panel of `w` by `h` (the toolkit's heads-up sheet with the item, the buttons and the keys) goes: the first place, nearest the middle
    /// of the screen, that no item is in or beside, the one in the hand before any other; where every place has one, the one that covers the
    /// fewest.
    pub fn panel_spot(&self, w: i32, h: i32) -> Point {
        let l = self.layout();
        let f = &self.frame;
        let mut xs: Vec<i32> = Vec::new();
        let mut x = f.safe_x + 40;
        while x <= W - f.safe_x - 40 - w {
            xs.push(x);
            x += 40;
        }
        let mid = |x: i32| (f64::from(x) + f64::from(w) / 2.0 - 960.0).abs();
        xs.sort_by(|a, b| mid(*a).total_cmp(&mid(*b)));
        let ys = [
            crate::model::round(f64::from(H - h) / 2.0),
            f.top + 28,
            f.limit - h - 28,
        ];
        let mut best = Point {
            x: xs.first().copied().unwrap_or(f.safe_x),
            y: ys[0],
        };
        let mut best_score = i32::MAX;
        for y in ys {
            for x in &xs {
                let r = Rect::new(*x, y, w, h);
                let mut score = 0;
                for it in &l.items {
                    if it.waits {
                        continue;
                    }
                    if r.meets(&it.rect, 16) {
                        score += if it.id == self.sel { 100 } else { 1 };
                    }
                }
                if score < best_score {
                    best_score = score;
                    best = Point { x: *x, y };
                }
                if score == 0 {
                    return best;
                }
            }
        }
        best
    }

    /// What to draw. A notice older than a few seconds is no longer in it.
    pub fn view(&self, now: u64) -> View {
        let l = self.layout();
        let effective = match self.work {
            Some(p) => self.reg.with_placement(&self.user, self.sel, p),
            None => self.user.clone(),
        };
        let items = l
            .items
            .iter()
            .map(|laid| {
                let it = self.reg.item(laid.id);
                ItemView {
                    id: laid.id,
                    title: it.map_or("", |i| i.title),
                    icon: it.map_or("", |i| i.icon),
                    rect: laid.rect,
                    placement: laid.placement,
                    custom: effective.has(laid.id),
                    waits: laid.waits,
                    words: self.reg.words(laid.id, &effective),
                    selected: laid.id == self.sel,
                    in_hand: self.mode == Mode::Move && laid.id == self.sel,
                    flagged: self.blocker == Some(laid.id) && self.notice_live(now),
                    reach: it
                        .filter(|i| i.drift > 0 && !self.reg.is_auto(laid.id, &effective))
                        .and_then(|_| {
                            self.reg.drift_box(
                                laid.id,
                                &effective,
                                &self.frame,
                                Size::new(laid.rect.w, laid.rect.h),
                            )
                        }),
                }
            })
            .collect();
        let held_rect = match (self.mode, self.held) {
            (Mode::Move, Some(h)) => self
                .reg
                .layout_all(
                    &self.user,
                    &self.frame,
                    &self.sizes,
                    Some((self.sel, h)),
                    true,
                )
                .rect(self.sel),
            _ => None,
        };
        let f = &self.frame;
        let browse = self.mode == Mode::Browse;
        let buttons = if browse {
            vec![
                Button {
                    label: BUTTONS[0],
                    enabled: self.user.has(self.sel),
                    focused: self.focus == Focus::Bar && self.bar == 0,
                    primary: false,
                },
                Button {
                    label: BUTTONS[1],
                    enabled: !self.user.is_empty(),
                    focused: self.focus == Focus::Bar && self.bar == 1,
                    primary: false,
                },
                Button {
                    label: BUTTONS[2],
                    enabled: true,
                    focused: self.focus == Focus::Bar && self.bar == 2,
                    primary: true,
                },
            ]
        } else {
            Vec::new()
        };
        View {
            selected: self.sel,
            mode: if browse { "browse" } else { "move" },
            focus: if self.focus == Focus::Bar {
                "buttons"
            } else {
                "items"
            },
            items,
            held_rect,
            heading: if browse { Some(HEADING) } else { None },
            subject: format!(
                "{}  \u{b7}  {}",
                self.title(self.sel),
                self.reg.words(self.sel, &effective)
            ),
            notice: if self.notice_live(now) {
                self.notice.clone()
            } else {
                String::new()
            },
            hint: if !browse {
                HINT_MOVE
            } else if self.focus == Focus::Bar {
                HINT_BAR
            } else {
                HINT_ITEMS
            },
            buttons,
            customised: self.user.customised(),
            dragging: self.dragging,
            guides: Guides {
                safe: Rect::from_edges(f.safe_x, f.safe_y, W - f.safe_x, H - f.safe_y),
                status_bar: Rect::new(f.safe_x, f.safe_y, W - 2 * f.safe_x, f.bar),
                bottom_quarter: Rect::from_edges(0, f.limit, W, H),
                subtitles: Rect::from_edges(0, SUBS.top, W, SUBS.bottom),
            },
        }
    }

    fn notice_live(&self, now: u64) -> bool {
        !self.notice.is_empty() && now.saturating_sub(self.notice_at) < NOTICE_MS
    }
}

/// What a toolkit draws: the frame's guides, every item in its place with the words for it, the one in the hand and where it was, the
/// panel's copy and buttons.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub selected: &'static str,
    /// `browse` or `move` (an item is in the hand).
    pub mode: &'static str,
    /// `items` or `buttons`.
    pub focus: &'static str,
    pub items: Vec<ItemView>,
    /// Where the item in the hand was: its place stays outlined.
    pub held_rect: Option<Rect>,
    /// The panel's title, in browse mode only.
    pub heading: Option<&'static str>,
    /// The item and where it is: "Camera pop-up  ·  Top right, moved".
    pub subject: String,
    pub notice: String,
    pub hint: &'static str,
    pub buttons: Vec<Button>,
    /// How many items the owner has placed.
    pub customised: usize,
    pub dragging: bool,
    pub guides: Guides,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemView {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub rect: Rect,
    pub placement: Placement,
    /// The owner placed it (with the move in the hand, as it would be put down).
    pub custom: bool,
    /// Beside the window it would leave its zone: draw it faint, it waits for the window to go.
    pub waits: bool,
    pub words: String,
    pub selected: bool,
    pub in_hand: bool,
    /// It is the item that was in the way of the last refused move.
    pub flagged: bool,
    /// The box an item that drifts visits, for one the owner placed.
    pub reach: Option<Rect>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Button {
    pub label: &'static str,
    pub enabled: bool,
    pub focused: bool,
    pub primary: bool,
}

/// What the frame keeps clear: the safe area, Home's status bar's row, the bottom quarter (the subtitles are in it).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Guides {
    pub safe: Rect,
    pub status_bar: Rect,
    pub bottom_quarter: Rect,
    pub subtitles: Rect,
}
