//! The TV focus engine: where the D-pad's focus is and where it goes, and what the Magic Remote's pointer does. One registry for
//! every screen (`docs/research/rust-shell-kit.md`), pure data with no layers in it, so a screen's focus rules are tested without a
//! compositor; the motion that shows a move is the kit's [`super::flow`], which every screen inherits through [`super::Engine`].
//!
//! The rules (design language section 6 and `ten-foot-input.md`, PointerHover's rule of 2026-10-29):
//! - **Spatial, nearest in the direction pressed**, between the focusables of the scope that owns input: a candidate lies beyond
//!   the current one's centre in that direction; the nearest by the gap along the axis wins, with twice the weight on how far it
//!   sits off the axis, so a neighbour in line beats a nearer one diagonally. No wrap: at an edge the focus stays.
//! - **Memory per region**: a move that enters another region (the dock from the widgets, a row from the status bar) lands on
//!   what was focused there last, when it still lies in the direction; the first visit takes the nearest.
//! - **Guides** where the geometry misleads: an explicit next focusable for one direction of one focusable wins over the search.
//! - **Hidden or inert content is absent** from hits and from the search; **a modal scope owns input** until it closes, and the
//!   focus returns to where it was.
//! - **Pointer: hover is a cue, click is the act.** The pointer over a control only shows the hover state; it never moves the focus,
//!   opens anything or picks a row. A click focuses the control under it and activates it. The next key hides the pointer and the
//!   cue with it, and the D-pad's focus shows where it was. A hover synthesised when content moves under a still pointer does not
//!   bring the cue back. Hit testing is half-open, newest registration on top.

use ui::geometry::Frame;

/// Names one focusable; chosen by the screen, unique within a [`Focus`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(pub u32);

/// A region with focus memory of its own: the status bar, the widgets, the dock, a row of a grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Region(pub u32);

/// A scope: the base scope of a screen is [`Scope::BASE`]; a sheet or a dialog opens another, modal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Scope(pub u32);

impl Scope {
    pub const BASE: Scope = Scope(0);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

    fn index(self) -> usize {
        match self {
            Dir::Up => 0,
            Dir::Down => 1,
            Dir::Left => 2,
            Dir::Right => 3,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    id: Id,
    frame: Frame,
    region: Region,
    scope: Scope,
    enabled: bool,
    guides: [Option<Id>; 4],
}

#[derive(Clone, Copy, Debug)]
struct Modal {
    scope: Scope,
    return_to: Option<Id>,
}

/// What a key did to the focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moved {
    /// The focus went from the first to the second.
    To(Id, Id),
    /// Nothing lies that way: the focus stayed where it was.
    Stayed(Id),
    /// Nothing is focused (an empty screen, or one whose focus was cleared).
    Nothing,
}

/// What a pointer movement changed of the hover cue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hovered {
    pub from: Option<Id>,
    pub to: Option<Id>,
}

#[derive(Debug, Default)]
pub struct Focus {
    entries: Vec<Entry>,
    modals: Vec<Modal>,
    inert: Vec<Scope>,
    current: Option<Id>,
    memory: Vec<(Region, Id)>,
    /// The control under the pointer while the cue is live; never the focus.
    hover: Option<Id>,
}

impl Focus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a focusable in `region` and `scope`. A later registration is on top of an earlier one for hits. Registering an id
    /// again replaces its frame, region and scope and keeps its guides.
    pub fn register(&mut self, id: Id, frame: Frame, region: Region, scope: Scope) {
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(e) => {
                e.frame = frame;
                e.region = region;
                e.scope = scope;
            }
            None => self.entries.push(Entry { id, frame, region, scope, enabled: true, guides: [None; 4] }),
        }
    }

    pub fn unregister(&mut self, id: Id) {
        self.entries.retain(|e| e.id != id);
        self.memory.retain(|(_, m)| *m != id);
        for e in &mut self.entries {
            for g in &mut e.guides {
                if *g == Some(id) {
                    *g = None;
                }
            }
        }
        if self.current == Some(id) {
            self.current = None;
        }
        if self.hover == Some(id) {
            self.hover = None;
        }
        for m in &mut self.modals {
            if m.return_to == Some(id) {
                m.return_to = None;
            }
        }
    }

    /// Moves a focusable (a row that scrolled) without changing its order.
    pub fn place(&mut self, id: Id, frame: Frame) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.frame = frame;
        }
    }

    pub fn frame(&self, id: Id) -> Option<Frame> {
        self.entries.iter().find(|e| e.id == id).map(|e| e.frame)
    }

    pub fn region(&self, id: Id) -> Option<Region> {
        self.entries.iter().find(|e| e.id == id).map(|e| e.region)
    }

    /// Where the search is wrong, says where `dir` from `from` goes.
    pub fn guide(&mut self, from: Id, dir: Dir, to: Id) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == from) {
            e.guides[dir.index()] = Some(to);
        }
    }

    /// A disabled focusable keeps its frame (a click on it is swallowed rather than falling through) but takes no focus.
    pub fn set_enabled(&mut self, id: Id, enabled: bool) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.enabled = enabled;
        }
        if !enabled && self.current == Some(id) {
            self.current = None;
        }
    }

    /// Switches a scope off or on. An inert scope is absent: nothing in it is hit, focused or found.
    pub fn set_inert(&mut self, scope: Scope, inert: bool) {
        self.inert.retain(|s| *s != scope);
        if inert {
            self.inert.push(scope);
            if self.current.is_some_and(|c| self.scope_of(c) == Some(scope)) {
                self.current = None;
            }
            if self.hover.is_some_and(|h| self.scope_of(h) == Some(scope)) {
                self.hover = None;
            }
        }
    }

    /// Opens a modal scope: only it takes input from now on; what was focused is remembered for [`Focus::close_modal`].
    pub fn open_modal(&mut self, scope: Scope) {
        self.modals.push(Modal { scope, return_to: self.current });
        self.current = None;
        self.hover = None;
    }

    /// Closes the newest modal scope and returns the focus to where it was, if that place is still there and live.
    pub fn close_modal(&mut self) -> Option<Scope> {
        let modal = self.modals.pop()?;
        self.current = modal.return_to.filter(|id| self.is_live(*id));
        self.hover = None;
        Some(modal.scope)
    }

    pub fn modal(&self) -> Option<Scope> {
        self.modals.last().map(|m| m.scope)
    }

    fn scope_of(&self, id: Id) -> Option<Scope> {
        self.entries.iter().find(|e| e.id == id).map(|e| e.scope)
    }

    fn live_scope(&self, scope: Scope) -> bool {
        !self.inert.contains(&scope) && self.modal().is_none_or(|m| m == scope)
    }

    fn entry(&self, id: Id) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Whether `id` can take the focus now: registered, enabled, in a live scope.
    pub fn is_live(&self, id: Id) -> bool {
        self.entry(id).is_some_and(|e| e.enabled && self.live_scope(e.scope))
    }

    pub fn current(&self) -> Option<Id> {
        self.current
    }

    /// Focuses `id` if it is live, and remembers it for its region.
    pub fn focus(&mut self, id: Id) -> bool {
        if !self.is_live(id) {
            return false;
        }
        self.current = Some(id);
        self.remember(id);
        true
    }

    pub fn clear(&mut self) {
        self.current = None;
    }

    fn remember(&mut self, id: Id) {
        let Some(region) = self.region(id) else { return };
        match self.memory.iter_mut().find(|(r, _)| *r == region) {
            Some(slot) => slot.1 = id,
            None => self.memory.push((region, id)),
        }
    }

    /// What a region remembers: the focusable last focused in it.
    pub fn remembered(&self, region: Region) -> Option<Id> {
        self.memory.iter().find(|(r, _)| *r == region).map(|(_, id)| *id)
    }

    /// The live focusables in reading order: by row of their top edge (to within half the shorter height), then by left.
    pub fn order(&self) -> Vec<Id> {
        let mut live: Vec<&Entry> = self.entries.iter().filter(|e| e.enabled && self.live_scope(e.scope)).collect();
        live.sort_by(|a, b| {
            let same_row = (a.frame.y - b.frame.y).abs() < a.frame.h.min(b.frame.h) / 2.0;
            if same_row { a.frame.x.total_cmp(&b.frame.x) } else { a.frame.y.total_cmp(&b.frame.y) }
        });
        live.into_iter().map(|e| e.id).collect()
    }

    /// Focuses the first live focusable in reading order when nothing is focused; what is focused otherwise.
    pub fn ensure(&mut self) -> Option<Id> {
        if self.current.is_some_and(|c| self.is_live(c)) {
            return self.current;
        }
        let first = self.order().into_iter().next()?;
        self.focus(first);
        self.current
    }

    /// Whether `to` lies beyond `from` in `dir`: its centre past the current's centre and its near edge past the current's middle.
    fn beyond(from: &Frame, to: &Frame, dir: Dir) -> bool {
        let (fc, tc) = (from.centre(), to.centre());
        match dir {
            Dir::Right => tc.0 > fc.0 && to.x >= fc.0,
            Dir::Left => tc.0 < fc.0 && to.right() <= fc.0,
            Dir::Down => tc.1 > fc.1 && to.y >= fc.1,
            Dir::Up => tc.1 < fc.1 && to.bottom() <= fc.1,
        }
    }

    /// Whether `to` overlaps `from` across the axis of `dir` (it is in the beam the move casts).
    fn in_beam(from: &Frame, to: &Frame, dir: Dir) -> bool {
        match dir {
            Dir::Right | Dir::Left => to.y < from.bottom() && to.bottom() > from.y,
            Dir::Down | Dir::Up => to.x < from.right() && to.right() > from.x,
        }
    }

    /// The cost of going from `from` to `to` in `dir`: the gap along the axis, the gap across it weighed twice, the centres' offset
    /// across the axis as the tie-breaker.
    fn cost(from: &Frame, to: &Frame, dir: Dir) -> f32 {
        let (fc, tc) = (from.centre(), to.centre());
        let gap = |a0: f32, a1: f32, b0: f32, b1: f32| -> f32 {
            if b0 > a1 {
                b0 - a1
            } else if a0 > b1 {
                a0 - b1
            } else {
                0.0
            }
        };
        let (primary, secondary, offset) = match dir {
            Dir::Right | Dir::Left => (gap(from.x, from.right(), to.x, to.right()), gap(from.y, from.bottom(), to.y, to.bottom()), (tc.1 - fc.1).abs()),
            Dir::Down | Dir::Up => (gap(from.y, from.bottom(), to.y, to.bottom()), gap(from.x, from.right(), to.x, to.right()), (tc.0 - fc.0).abs()),
        };
        primary + 2.0 * secondary + 0.1 * offset
    }

    /// Where `dir` from `from` goes: the guide, the region's memory, else the nearest in that direction; `None` at an edge. A
    /// candidate must lie in the beam the move casts (overlap the current one across the axis): a row never jumps diagonally.
    /// Up and Down may leave the beam into another region (the status bar above a row of widgets, the dock below), where the
    /// nearest wins; Left and Right never do, so the end of a row holds.
    pub fn next(&self, from: Id, dir: Dir) -> Option<Id> {
        let e = self.entry(from)?;
        if let Some(g) = e.guides[dir.index()].filter(|g| self.is_live(*g)) {
            return Some(g);
        }
        let frame = e.frame;
        let region = e.region;
        let vertical = matches!(dir, Dir::Up | Dir::Down);
        let candidates = || self.entries.iter().filter(|c| c.id != from && c.enabled && self.live_scope(c.scope) && Self::beyond(&frame, &c.frame, dir));
        let nearest = |allowed: &dyn Fn(&Entry) -> bool| {
            candidates().filter(|c| allowed(c)).map(|c| (Self::cost(&frame, &c.frame, dir), c)).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, c)| c)
        };
        let best = match nearest(&|c| Self::in_beam(&frame, &c.frame, dir)) {
            Some(c) => c,
            None if vertical => nearest(&|c| c.region != region)?,
            None => return None,
        };
        if best.region != region
            && let Some(m) = self.remembered(best.region).filter(|m| *m != from && self.is_live(*m))
            && let Some(me) = self.entry(m)
            && Self::beyond(&frame, &me.frame, dir)
        {
            return Some(m);
        }
        Some(best.id)
    }

    /// A D-pad key: moves the focus, hides the pointer's cue (the key hides the pointer).
    pub fn step(&mut self, dir: Dir) -> Moved {
        self.hover = None;
        let Some(from) = self.current.filter(|c| self.is_live(*c)) else {
            return match self.ensure() {
                Some(id) => Moved::Stayed(id),
                None => Moved::Nothing,
            };
        };
        match self.next(from, dir) {
            Some(to) => {
                self.current = Some(to);
                self.remember(to);
                Moved::To(from, to)
            }
            None => Moved::Stayed(from),
        }
    }

    /// Any key that is not a move (OK, Back, a digit): the pointer and its cue go.
    pub fn key(&mut self) {
        self.hover = None;
    }

    /// What is under (`x`, `y`): the newest live registration there, enabled or not.
    pub fn hit(&self, x: f32, y: f32) -> Option<Id> {
        self.entries.iter().rev().find(|e| self.live_scope(e.scope) && e.frame.contains(x, y)).map(|e| e.id)
    }

    /// The pointer moved to (`x`, `y`): the hover cue follows it onto enabled controls. `moved` is false for a hover the host
    /// synthesises when content moves under a still pointer: that never brings the cue back.
    pub fn hover(&mut self, x: f32, y: f32, moved: bool) -> Hovered {
        let from = self.hover;
        if !moved && from.is_none() {
            return Hovered { from, to: from };
        }
        let to = self.hit(x, y).filter(|id| self.is_live(*id));
        self.hover = to;
        Hovered { from, to }
    }

    /// The cue's control, while the cue is live.
    pub fn hovered(&self) -> Option<Id> {
        self.hover
    }

    /// The pointer went: the cue goes with it.
    pub fn pointer_gone(&mut self) -> Hovered {
        let from = self.hover.take();
        Hovered { from, to: None }
    }

    /// A click at (`x`, `y`): focuses the enabled control there and returns it for activation; a click on nothing, or on a disabled
    /// control, does nothing and returns `None`.
    pub fn click(&mut self, x: f32, y: f32) -> Option<Id> {
        let id = self.hit(x, y).filter(|id| self.is_live(*id))?;
        self.focus(id);
        self.hover = None;
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(x: f32, y: f32, w: f32, h: f32) -> Frame {
        Frame::new(x, y, w, h)
    }

    const WIDGETS: Region = Region(1);
    const DOCK: Region = Region(2);
    const BAR: Region = Region(3);

    /// Home's shape: a status bar of three buttons, two rows of widgets of different sizes, a dock of four tiles.
    fn home() -> Focus {
        let mut fo = Focus::new();
        for (i, x) in [1400.0, 1560.0, 1720.0].into_iter().enumerate() {
            fo.register(Id(10 + i as u32), f(x, 54.0, 120.0, 60.0), BAR, Scope::BASE);
        }
        fo.register(Id(20), f(64.0, 134.0, 600.0, 222.0), WIDGETS, Scope::BASE);
        fo.register(Id(21), f(684.0, 134.0, 760.0, 480.0), WIDGETS, Scope::BASE);
        fo.register(Id(22), f(1464.0, 134.0, 392.0, 480.0), WIDGETS, Scope::BASE);
        fo.register(Id(23), f(64.0, 376.0, 600.0, 238.0), WIDGETS, Scope::BASE);
        for i in 0..4u32 {
            fo.register(Id(30 + i), f(64.0 + 180.0 * i as f32, 700.0, 160.0, 160.0), DOCK, Scope::BASE);
        }
        fo
    }

    #[test]
    fn the_nearest_in_the_direction_wins_and_an_edge_holds() {
        let mut fo = home();
        fo.focus(Id(20));
        assert_eq!(fo.step(Dir::Right), Moved::To(Id(20), Id(21)));
        assert_eq!(fo.step(Dir::Right), Moved::To(Id(21), Id(22)));
        assert_eq!(fo.step(Dir::Right), Moved::Stayed(Id(22)), "no wrap at the edge");
        assert_eq!(fo.step(Dir::Left), Moved::To(Id(22), Id(21)));
        assert_eq!(fo.step(Dir::Left), Moved::To(Id(21), Id(23)), "of the two to the left, the one nearer the tall widget's centre");
        assert_eq!(fo.step(Dir::Up), Moved::To(Id(23), Id(20)));
        assert_eq!(fo.step(Dir::Down), Moved::To(Id(20), Id(23)));
        assert_eq!(fo.step(Dir::Down), Moved::To(Id(23), Id(31)), "into the dock, the tile nearest in line");
        assert_eq!(fo.step(Dir::Down), Moved::Stayed(Id(31)));
    }

    #[test]
    fn a_region_remembers_where_its_focus_was() {
        let mut fo = home();
        fo.focus(Id(32));
        assert_eq!(fo.step(Dir::Up), Moved::To(Id(32), Id(23)), "the dock's third tile goes up into the widget over it");
        assert_eq!(fo.step(Dir::Down), Moved::To(Id(23), Id(32)), "and back down to the tile it came from, not the nearest");
        fo.focus(Id(21));
        assert_eq!(fo.step(Dir::Up), Moved::To(Id(21), Id(10)), "the first visit of the status bar takes the nearest");
        assert_eq!(fo.step(Dir::Right), Moved::To(Id(10), Id(11)));
        assert_eq!(fo.step(Dir::Down), Moved::To(Id(11), Id(21)), "the widgets remember 21 and it lies below");
        assert_eq!(fo.step(Dir::Up), Moved::To(Id(21), Id(11)), "the bar remembers its second button");
    }

    #[test]
    fn a_guide_wins_over_the_search_and_a_disabled_or_inert_target_is_absent() {
        let mut fo = home();
        fo.guide(Id(22), Dir::Down, Id(33));
        fo.focus(Id(22));
        assert_eq!(fo.step(Dir::Down), Moved::To(Id(22), Id(33)));
        fo.set_enabled(Id(33), false);
        fo.focus(Id(22));
        assert_eq!(fo.step(Dir::Down), Moved::To(Id(22), Id(32)), "a disabled guide target is skipped and the search runs");
        fo.set_inert(Scope::BASE, true);
        assert_eq!(fo.current(), None);
        assert_eq!(fo.step(Dir::Down), Moved::Nothing);
        fo.set_inert(Scope::BASE, false);
        assert_eq!(fo.step(Dir::Down), Moved::Stayed(Id(10)), "the first in reading order is taken when nothing is focused");
    }

    #[test]
    fn a_modal_scope_owns_input_and_gives_the_focus_back() {
        let mut fo = home();
        fo.focus(Id(21));
        let sheet = Scope(5);
        fo.register(Id(50), f(600.0, 300.0, 300.0, 80.0), Region(9), sheet);
        fo.register(Id(51), f(600.0, 400.0, 300.0, 80.0), Region(9), sheet);
        fo.open_modal(sheet);
        assert_eq!(fo.current(), None);
        assert_eq!(fo.step(Dir::Down), Moved::Stayed(Id(50)), "the first of the sheet");
        assert_eq!(fo.step(Dir::Down), Moved::To(Id(50), Id(51)));
        assert_eq!(fo.hit(100.0, 200.0), None, "the base scope is absent under a modal");
        assert_eq!(fo.close_modal(), Some(sheet));
        assert_eq!(fo.current(), Some(Id(21)));
    }

    #[test]
    fn hover_is_a_cue_and_click_is_the_act() {
        let mut fo = home();
        fo.focus(Id(20));
        let h = fo.hover(700.0, 200.0, true);
        assert_eq!(h, Hovered { from: None, to: Some(Id(21)) });
        assert_eq!(fo.current(), Some(Id(20)), "hover never moves the focus");
        assert_eq!(fo.hover(700.0, 1000.0, true), Hovered { from: Some(Id(21)), to: None });
        assert_eq!(fo.hover(700.0, 200.0, false), Hovered { from: None, to: None }, "a synthesised hover brings no cue back");
        fo.hover(700.0, 200.0, true);
        assert_eq!(fo.step(Dir::Right), Moved::To(Id(20), Id(21)));
        assert_eq!(fo.hovered(), None, "the key hid the pointer and its cue");
        assert_eq!(fo.click(1500.0, 300.0), Some(Id(22)));
        assert_eq!(fo.current(), Some(Id(22)), "a click focuses what is under it");
        fo.set_enabled(Id(22), false);
        assert_eq!(fo.click(1500.0, 300.0), None, "a disabled control swallows the click");
        assert_eq!(fo.click(5.0, 5.0), None);
    }

    #[test]
    fn hits_are_half_open_and_the_newest_registration_is_on_top() {
        let mut fo = Focus::new();
        fo.register(Id(1), f(0.0, 0.0, 100.0, 100.0), Region(0), Scope::BASE);
        fo.register(Id(2), f(100.0, 0.0, 100.0, 100.0), Region(0), Scope::BASE);
        fo.register(Id(3), f(50.0, 50.0, 100.0, 100.0), Region(0), Scope::BASE);
        assert_eq!(fo.hit(99.9, 10.0), Some(Id(1)));
        assert_eq!(fo.hit(100.0, 10.0), Some(Id(2)));
        assert_eq!(fo.hit(120.0, 60.0), Some(Id(3)));
        fo.unregister(Id(3));
        assert_eq!(fo.hit(120.0, 60.0), Some(Id(2)));
    }

    #[test]
    fn widgets_of_different_shapes_hand_over_along_the_axis() {
        // a short wide card over a tall narrow one beside a square: down from the wide card goes to the tall one under it,
        // not to the square that is nearer by centre
        let mut fo = Focus::new();
        fo.register(Id(1), f(0.0, 0.0, 800.0, 100.0), Region(0), Scope::BASE);
        fo.register(Id(2), f(0.0, 120.0, 200.0, 600.0), Region(0), Scope::BASE);
        fo.register(Id(3), f(300.0, 120.0, 300.0, 300.0), Region(0), Scope::BASE);
        fo.focus(Id(1));
        assert_eq!(fo.step(Dir::Down), Moved::To(Id(1), Id(3)), "the square is in line with the wide card's centre");
        fo.focus(Id(2));
        assert_eq!(fo.step(Dir::Right), Moved::To(Id(2), Id(3)));
        assert_eq!(fo.step(Dir::Up), Moved::To(Id(3), Id(1)));
    }
}
