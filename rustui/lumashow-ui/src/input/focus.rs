//! Who gets a touch, and where keyboard-style focus goes: a registry of the places that can take input, in scopes, with the two
//! rules of the design (section 3.3): **content that is hidden or inert is absent** from hit testing and from focus navigation,
//! and **a modal scope owns input** until it is closed, then focus returns to where it was.
//!
//! The registry is small plain data (no layers): a component registers the frame it hit-tests with, a screen opens a scope for a
//! sheet or a dialog and closes it again, and the host asks [`Focus::hit`] what a contact is on. Frames are half-open, like
//! [`Frame::contains`], so two that meet share no point.

use crate::geometry::Frame;

/// Names one place that can take input. Chosen by the component's owner; unique within a [`Focus`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FocusId(pub u32);

/// Names a scope: the base scope of a screen is [`Scope::BASE`]; a sheet or a dialog opens another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Scope(pub u32);

impl Scope {
    pub const BASE: Scope = Scope(0);
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    id: FocusId,
    frame: Frame,
    scope: Scope,
    enabled: bool,
}

/// An open modal scope and where focus was when it opened.
#[derive(Clone, Copy, Debug)]
struct Modal {
    scope: Scope,
    return_to: Option<FocusId>,
}

#[derive(Debug, Default)]
pub struct Focus {
    entries: Vec<Entry>,
    modals: Vec<Modal>,
    /// Scopes switched off (a hidden page, a pager page that is not showing).
    inert: Vec<Scope>,
    current: Option<FocusId>,
}

impl Focus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a place that takes input, in `scope`. A later registration is drawn above an earlier one, so it wins a hit where
    /// they overlap. Registering an id again replaces its frame and scope.
    pub fn register(&mut self, id: FocusId, frame: Frame, scope: Scope) {
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(e) => {
                e.frame = frame;
                e.scope = scope;
            }
            None => self.entries.push(Entry {
                id,
                frame,
                scope,
                enabled: true,
            }),
        }
    }

    pub fn unregister(&mut self, id: FocusId) {
        self.entries.retain(|e| e.id != id);
        if self.current == Some(id) {
            self.current = None;
        }
        for m in &mut self.modals {
            if m.return_to == Some(id) {
                m.return_to = None;
            }
        }
    }

    /// Moves a place (a list row following a finger) without changing its order.
    pub fn place(&mut self, id: FocusId, frame: Frame) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.frame = frame;
        }
    }

    /// A disabled place keeps its frame (a finger on it still hits it, so the press is swallowed rather than falling through to
    /// what is behind) but takes no focus and no activation.
    pub fn set_enabled(&mut self, id: FocusId, enabled: bool) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.enabled = enabled;
        }
        if !enabled && self.current == Some(id) {
            self.current = None;
        }
    }

    /// Switches a scope off or on. An inert scope is absent: nothing in it is hit or focused.
    pub fn set_inert(&mut self, scope: Scope, inert: bool) {
        self.inert.retain(|s| *s != scope);
        if inert {
            self.inert.push(scope);
            if self
                .current
                .is_some_and(|c| self.scope_of(c) == Some(scope))
            {
                self.current = None;
            }
        }
    }

    /// Opens a modal scope: from now on only it takes input, and what was focused is remembered for [`Focus::close_modal`].
    pub fn open_modal(&mut self, scope: Scope) {
        self.modals.push(Modal {
            scope,
            return_to: self.current,
        });
        self.current = None;
    }

    /// Closes the newest modal scope and returns focus to where it was, if that place is still there and live.
    pub fn close_modal(&mut self) -> Option<Scope> {
        let modal = self.modals.pop()?;
        self.current = modal.return_to.filter(|id| {
            self.entries
                .iter()
                .any(|e| e.id == *id && e.enabled && self.live(e.scope))
        });
        Some(modal.scope)
    }

    /// The scope that owns input now: the newest modal, else everything that is not inert.
    pub fn modal(&self) -> Option<Scope> {
        self.modals.last().map(|m| m.scope)
    }

    fn scope_of(&self, id: FocusId) -> Option<Scope> {
        self.entries.iter().find(|e| e.id == id).map(|e| e.scope)
    }

    /// Whether anything in `scope` can take input right now.
    fn live(&self, scope: Scope) -> bool {
        !self.inert.contains(&scope) && self.modal().is_none_or(|m| m == scope)
    }

    /// What a contact at (`x`, `y`) is on: the newest registered place there that is live, whether or not it is enabled.
    pub fn hit(&self, x: f32, y: f32) -> Option<FocusId> {
        self.entries
            .iter()
            .rev()
            .find(|e| self.live(e.scope) && e.frame.contains(x, y))
            .map(|e| e.id)
    }

    /// Whether the place at (`x`, `y`) takes activation: it is live and enabled.
    pub fn hit_enabled(&self, x: f32, y: f32) -> Option<FocusId> {
        self.hit(x, y)
            .filter(|id| self.entries.iter().any(|e| e.id == *id && e.enabled))
    }

    pub fn current(&self) -> Option<FocusId> {
        self.current
    }

    /// Focuses `id` if it is live and enabled.
    pub fn focus(&mut self, id: FocusId) -> bool {
        let ok = self
            .entries
            .iter()
            .any(|e| e.id == id && e.enabled && self.live(e.scope));
        if ok {
            self.current = Some(id);
        }
        ok
    }

    pub fn clear(&mut self) {
        self.current = None;
    }

    /// The live, enabled places in reading order: by row of their top edge (to within half the shorter height), then by left.
    pub fn order(&self) -> Vec<FocusId> {
        let mut live: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| e.enabled && self.live(e.scope))
            .collect();
        live.sort_by(|a, b| {
            let same_row = (a.frame.y - b.frame.y).abs() < a.frame.h.min(b.frame.h) / 2.0;
            if same_row {
                a.frame.x.total_cmp(&b.frame.x)
            } else {
                a.frame.y.total_cmp(&b.frame.y)
            }
        });
        live.into_iter().map(|e| e.id).collect()
    }

    /// Moves focus to the next place in reading order, wrapping; the first one when nothing is focused.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<FocusId> {
        self.step(1)
    }

    pub fn previous(&mut self) -> Option<FocusId> {
        self.step(-1)
    }

    fn step(&mut self, by: isize) -> Option<FocusId> {
        let order = self.order();
        if order.is_empty() {
            self.current = None;
            return None;
        }
        let at = self
            .current
            .and_then(|c| order.iter().position(|id| *id == c));
        let next = match at {
            Some(i) => (i as isize + by).rem_euclid(order.len() as isize) as usize,
            None if by > 0 => 0,
            None => order.len() - 1,
        };
        self.current = Some(order[next]);
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> FocusId {
        FocusId(n)
    }

    fn row() -> Focus {
        let mut f = Focus::new();
        f.register(id(1), Frame::new(0.0, 0.0, 100.0, 64.0), Scope::BASE);
        f.register(id(2), Frame::new(100.0, 0.0, 100.0, 64.0), Scope::BASE);
        f.register(id(3), Frame::new(0.0, 64.0, 100.0, 64.0), Scope::BASE);
        f
    }

    #[test]
    fn two_places_that_meet_share_no_point_and_a_finger_on_the_seam_hits_one() {
        let f = row();
        assert_eq!(f.hit(99.9, 10.0), Some(id(1)));
        assert_eq!(f.hit(100.0, 10.0), Some(id(2)));
        assert_eq!(f.hit(10.0, 64.0), Some(id(3)));
        assert_eq!(f.hit(250.0, 10.0), None);
    }

    #[test]
    fn the_newer_place_wins_where_two_overlap() {
        let mut f = row();
        f.register(id(9), Frame::new(50.0, 0.0, 100.0, 40.0), Scope::BASE);
        assert_eq!(f.hit(60.0, 10.0), Some(id(9)));
        assert_eq!(f.hit(60.0, 50.0), Some(id(1)));
    }

    #[test]
    fn an_inert_scope_is_absent_from_hits_and_from_navigation_and_returns_when_switched_on() {
        let mut f = row();
        let page = Scope(5);
        f.register(id(7), Frame::new(300.0, 0.0, 100.0, 64.0), page);
        assert_eq!(f.hit(310.0, 10.0), Some(id(7)));
        f.set_inert(page, true);
        assert_eq!(f.hit(310.0, 10.0), None);
        assert!(!f.order().contains(&id(7)));
        assert!(!f.focus(id(7)));
        f.set_inert(page, false);
        assert_eq!(f.hit(310.0, 10.0), Some(id(7)));
        assert!(f.order().contains(&id(7)));
    }

    #[test]
    fn making_the_focused_scope_inert_drops_focus_instead_of_leaving_it_on_something_invisible() {
        let mut f = row();
        let page = Scope(5);
        f.register(id(7), Frame::new(300.0, 0.0, 100.0, 64.0), page);
        assert!(f.focus(id(7)));
        f.set_inert(page, true);
        assert_eq!(f.current(), None);
    }

    #[test]
    fn a_modal_owns_input_and_focus_returns_to_where_it_was_when_it_closes() {
        let mut f = row();
        assert!(f.focus(id(2)));
        let dialog = Scope(2);
        f.register(id(20), Frame::new(40.0, 0.0, 100.0, 64.0), dialog);
        f.open_modal(dialog);
        // Behind the dialog nothing is reachable, even where the dialog has no place of its own.
        assert_eq!(f.hit(10.0, 10.0), None);
        assert_eq!(f.hit(60.0, 10.0), Some(id(20)));
        assert_eq!(f.order(), [id(20)]);
        assert!(!f.focus(id(1)));
        assert!(f.focus(id(20)));
        assert_eq!(f.close_modal(), Some(dialog));
        assert_eq!(f.current(), Some(id(2)));
        assert_eq!(f.modal(), None);
    }

    #[test]
    fn focus_does_not_return_to_a_place_that_went_away_while_the_modal_was_open() {
        let mut f = row();
        assert!(f.focus(id(2)));
        f.open_modal(Scope(2));
        f.unregister(id(2));
        f.close_modal();
        assert_eq!(f.current(), None);
    }

    #[test]
    fn a_disabled_place_swallows_a_touch_but_takes_no_focus_and_is_skipped_in_order() {
        let mut f = row();
        f.set_enabled(id(2), false);
        assert_eq!(f.hit(150.0, 10.0), Some(id(2)));
        assert_eq!(f.hit_enabled(150.0, 10.0), None);
        assert_eq!(f.order(), [id(1), id(3)]);
        assert!(!f.focus(id(2)));
    }

    #[test]
    fn navigation_follows_reading_order_wraps_both_ways_and_starts_at_the_ends() {
        let mut f = row();
        assert_eq!(f.next(), Some(id(1)));
        assert_eq!(f.next(), Some(id(2)));
        assert_eq!(f.next(), Some(id(3)));
        assert_eq!(f.next(), Some(id(1)));
        assert_eq!(f.previous(), Some(id(3)));
        f.clear();
        assert_eq!(f.previous(), Some(id(3)));
        let mut empty = Focus::new();
        assert_eq!(empty.next(), None);
    }

    #[test]
    fn a_moved_place_keeps_its_order_slot_and_is_hit_at_its_new_frame() {
        let mut f = row();
        f.place(id(1), Frame::new(500.0, 500.0, 100.0, 64.0));
        assert_eq!(f.hit(10.0, 10.0), None);
        assert_eq!(f.hit(510.0, 510.0), Some(id(1)));
    }
}
