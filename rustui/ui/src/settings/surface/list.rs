//! The list of rows of a page (`PageList.qml`'s viewport): the rows of the tab in force in a column under a scroll group, shown by whole
//! rows (the clip of the viewport is cut at the last whole row, so a row the viewport would cut is not drawn at rest), scrolled on the
//! flowing focus's spring (the same track as the lift, so the scroll and the lift are one gesture), and built only where they can be
//! seen: the rows that are in view or a row's reach from it. A long list (the apps, the networks) costs the rows in view and a little
//! beside them, not the whole of it, so the scene's budget holds at 200 % text too.

use super::layout::{Column, RowGeom, Sizes, follow, measure_row, whole_rows};
use super::paint::Paint;
use super::rows::{Env, Moment, Placed, RowArt, other, signature};
use crate::kit::focus::{Id, Region, Scope};
use crate::kit::viewport::Reach;
use crate::kit::widgets::Lift;
use crate::kit::{Engine, Kit};
use crate::settings::row::{Kind, Row};
use compositor_protocol::{Edges, GroupPose, Rect};
use ui::geometry::Frame;
use ui::group::Group;

/// The id of the first row of a list; each row has room for its cards after it.
pub const ROW_ID: u32 = 10_000;
pub const ROW_STRIDE: u32 = 128;

pub fn id_at(base: u32, i: usize) -> Id {
    Id(base + i as u32 * ROW_STRIDE)
}

/// Which row (and which card of it) a focus id is, for the list whose first id is `base` and which has `rows` rows.
pub fn row_of(base: u32, rows: usize, id: Id) -> Option<(usize, Option<usize>)> {
    let n = id.0.checked_sub(base)?;
    let (row, card) = ((n / ROW_STRIDE) as usize, n % ROW_STRIDE);
    (row < rows).then_some((row, if card == 0 { None } else { Some(card as usize - 1) }))
}

struct Slot {
    id: String,
    sig: String,
    geom: RowGeom,
    art: Option<RowArt>,
    live: bool,
    top: f32,
    w: f32,
}

pub struct ListArt {
    pub region: Region,
    scope: Scope,
    z: i32,
    /// The id of the first row (the page's list and the rail's have ranges of their own).
    id_base: u32,
    /// The rail's rows keep a little less air than a page row, so ten fit.
    tight: bool,
    viewport: Group,
    scroll: Group,
    /// The viewport's top left on the screen.
    pub origin: (f32, f32),
    pub size: (f32, f32),
    zone: f32,
    slots: Vec<Slot>,
    col: Column,
    /// The offset the list is scrolled to (the target; the spring takes the picture there).
    pub offset: f32,
    extent: f32,
    /// What the list was made for: a different page, tab or width makes it again.
    key: String,
    /// The layers the next `apply` may add while the list is first filled (`None`: as many as the rows in reach need; a row is made whole, and at
    /// least one is made in each). A page made new puts every row of it in one frame, 60 to 120 layers on the set, which a refresh at 120 Hz does not
    /// hold: the rest are made by the `apply` of the frames after.
    pub budget: Option<usize>,
    /// The list has not had every row in reach made yet, and whether rows are waiting for the next `apply`.
    filling: bool,
    pending: bool,
    /// The layers the last `apply` made.
    made: usize,
}

/// The clip of the viewport over the rows from `y` for `h`: the window and what a lifted row (or one of its cards) stands out by, so the
/// focused plate is not squared off at its ends.
fn rect_clip(reach: Reach, w: f32, y: f32, h: f32) -> Option<Rect> {
    Some(reach.clip(Frame::new(0.0, y, w, h)))
}

impl ListArt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(kit: &mut Kit, engine: &mut Engine, parent: Option<&Group>, key: String, origin: (f32, f32), size: (f32, f32), region: Region, scope: Scope, z: i32, id_base: u32, tight: bool) -> std::io::Result<Self> {
        let scene = kit.ui.scene.clone();
        let viewport = scene.group(parent.map(Group::id), GroupPose { x: origin.0, y: origin.1, clip: rect_clip(Reach::lift(kit, Lift::Row, (size.0, 0.0)), size.0, 0.0, size.1), feather: Edges::NONE, ..GroupPose::IDENTITY }).map_err(other)?;
        let rest = GroupPose::IDENTITY;
        let scroll = scene.group(Some(viewport.id()), rest).map_err(other)?;
        let mut me = ListArt { region, scope, z, id_base, tight, viewport, scroll, origin, size, zone: 0.0, slots: Vec::new(), col: Column::default(), offset: 0.0, extent: 0.0, key, budget: None, filling: true, pending: false, made: 0 };
        me.rescroll(kit, engine, 0.0)?;
        Ok(me)
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    /// Whether rows in reach are still to be made: the next `apply` makes them.
    pub fn pending(&self) -> bool {
        self.pending
    }

    /// The layers the last `apply` made.
    pub fn made(&self) -> usize {
        self.made
    }

    pub fn offset(&self) -> f32 { self.offset }

    pub fn restore_offset(&mut self, offset: f32) { self.offset = offset.max(0.0); }

    /// Keep the scroll group and its track while replacing palette-dependent row art.
    pub fn reparent_restyle(&mut self, parent: Option<&Group>, engine: &mut Engine) -> std::io::Result<()> {
        self.viewport.set_parent(parent.map(Group::id), false).map_err(other)?;
        for slot in &mut self.slots {
            if let Some(art) = slot.art.take() { art.release(engine, false); }
            slot.live = false;
        }
        Ok(())
    }

    /// The scroller's track covers 0 to `extent`; a list whose height changed has a new one.
    fn rescroll(&mut self, kit: &mut Kit, engine: &mut Engine, extent: f32) -> std::io::Result<()> {
        let rest = GroupPose { y: self.zone, ..GroupPose::IDENTITY };
        self.scroll.set_pose(rest);
        let scene = kit.ui.scene.clone();
        engine.flow.scroller(&scene, self.region, &self.scroll, rest, false, extent.max(1.0)).map_err(other)?;
        self.extent = extent.max(1.0);
        Ok(())
    }

    pub fn usable(&self) -> f32 {
        self.size.1 - 2.0 * self.zone
    }

    pub fn more_above(&self) -> bool {
        self.offset > 0.5
    }

    pub fn more_below(&self) -> bool {
        self.offset + self.usable() < self.col.total - 0.5
    }

    pub fn id_base(&self) -> u32 {
        self.id_base
    }

    /// Where the row `i` is on the screen now (by the target offset), or `None` for a row the list does not have.
    pub fn frame_of(&self, i: usize) -> Option<ui::geometry::Frame> {
        let s = self.slots.get(i)?;
        Some(ui::geometry::Frame::new(self.origin.0, self.origin.1 + self.zone + s.top - self.offset, s.w, s.geom.h))
    }

    /// Describes the rows in view to the layout lint (a row the viewport cuts is not drawn at rest, so only whole rows are fed).
    pub fn feed_lint(&self, kit: &mut Kit, paint: &Paint, rows: &[Row], lint: &mut crate::kit::lint::Lint, panel: usize) {
        let sz = Sizes::of(kit);
        let view = ui::geometry::Frame::new(self.origin.0, self.origin.1, self.size.0, self.size.1);
        for (i, s) in self.slots.iter().enumerate() {
            let (Some(art), Some(at), Some(row)) = (&s.art, self.frame_of(i), rows.get(i)) else { continue };
            if at.y >= view.y - 1.0 && at.bottom() <= view.bottom() + 1.0 {
                art.feed_lint(kit, paint, &sz, row, Placed { at, panel }, lint);
            }
        }
    }

    /// The rows' ids, in order.
    pub fn ids(&self) -> Vec<&str> {
        self.slots.iter().map(|s| s.id.as_str()).collect()
    }

    pub fn art(&self, i: usize) -> Option<&RowArt> {
        self.slots.get(i).and_then(|s| s.art.as_ref())
    }

    /// The layers the built rows hold now (the scene's budget).
    pub fn built(&self) -> usize {
        self.slots.iter().filter(|s| s.art.is_some()).count()
    }

    pub fn total(&self) -> f32 {
        self.col.total
    }

    /// Lays the rows out and brings them up to date: measured when new, built when they come into reach, dropped when they leave it,
    /// set in place for what moves. `stop` is the row the focus is on (the list scrolls to show it), `hovered` the row the pointer's
    /// cue is on, `seg` the inner focus of a segmented row or a row of cards.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(&mut self, kit: &mut Kit, engine: &mut Engine, paint: Paint, rows: &[Row], stop: Option<usize>, hovered: Option<usize>, seg: usize, focused: bool, now_ns: u64, at_once: bool) -> std::io::Result<()> {
        let sz = Sizes::of(kit);
        let w = self.size.0;
        let new_list = self.slots.is_empty();
        // the slots follow the rows: a row that kept its place and its words keeps its layers
        let mut slots: Vec<Slot> = Vec::with_capacity(rows.len());
        let mut old: Vec<Option<Slot>> = std::mem::take(&mut self.slots).into_iter().map(Some).collect();
        let mut tops = Vec::new();
        let mut y = 0.0;
        for (i, r) in rows.iter().enumerate() {
            let sig = signature(r);
            let previous = old.get_mut(i).and_then(Option::take);
            let (geom, art, live) = match previous {
                Some(s) if s.id == r.id && s.sig == sig && s.w == w => (s.geom, s.art, s.live),
                previous => {
                    if let Some(gone) = previous
                        && let Some(a) = &gone.art
                    {
                        let same_row = gone.id == r.id;
                        a.release(engine, same_row);
                        if same_row {
                            for (id, _) in a.cards.iter().skip(r.cards.len()) {
                                engine.flow.drop_lift(*id);
                            }
                        }
                    }
                    {
                        let mut g = measure_row(kit, r, w);
                        if self.tight {
                            g.h = (g.h - sz.s2).max(sz.row);
                        }
                        (g, None, false)
                    }
                }
            };
            tops.push(y);
            y += geom.h;
            slots.push(Slot { id: r.id.clone(), sig, geom, art, live, top: tops[i], w });
        }
        for gone in old.into_iter().flatten() {
            if let Some(a) = &gone.art {
                a.release(engine, false);
            }
        }
        // a row that moved (a taller row above it) is made again where it is now
        for (i, s) in slots.iter_mut().enumerate() {
            if s.art.as_ref().is_some_and(|a| (a.rect.y - tops[i]).abs() > 0.01) {
                if let Some(a) = &s.art {
                    a.release(engine, true);
                }
                s.art = None;
                s.live = false;
            }
            s.top = tops[i];
        }
        let heights: Vec<f32> = slots.iter().map(|s| s.geom.h).collect();
        let col = Column::of(&heights);
        let zone = if col.total > self.size.1 { sz.s6 } else { 0.0 };
        let layout_changed = (zone - self.zone).abs() > 0.01 || (col.total - self.col.total).abs() > 0.5;
        self.zone = zone;
        self.col = col;
        self.slots = slots;
        if layout_changed {
            self.rescroll(kit, engine, self.col.total)?;
            // the spring starts from rest at the offset the list is at
            engine.flow.scroll_snap(self.region, self.offset).map_err(other)?;
        }
        // scroll to show the focus
        let usable = self.usable();
        let want = match stop {
            Some(i) if i < self.slots.len() => follow(&self.col, i, self.offset, usable),
            _ => self.offset,
        };
        let want = want.clamp(0.0, self.col.total.max(0.0));
        if (want - self.offset).abs() > 0.01 {
            self.offset = want;
            if new_list {
                engine.flow.scroll_snap(self.region, want).map_err(other)?;
            } else {
                engine.flow.scroll_to(self.region, want).map_err(other)?;
            }
        }
        // the clip is cut at the last whole row
        let whole = whole_rows(&self.col, self.offset, usable);
        let clip_h = if whole.is_empty() { usable } else { self.col.tops[whole.end - 1] + self.col.heights[whole.end - 1] - self.offset };
        let clip_h = if self.col.total <= self.size.1 { self.size.1 } else { clip_h.max(0.0) };
        let clip_y = if self.col.total <= self.size.1 { 0.0 } else { self.zone };
        let tallest = self.col.heights.iter().copied().fold(0.0, f32::max);
        let focus = &kit.tokens.focus;
        let reach = self.slots.iter().filter_map(|s| s.geom.cards.as_ref()).fold(Reach::lift(kit, Lift::Row, (w, tallest)), |r, c| r.and(Reach::of(focus.button_scale, focus.card_lift, (c.card_w, c.card_h))));
        self.viewport.set_clip_feathered(rect_clip(reach, self.size.0, clip_y, clip_h), Edges::NONE);
        // the rows that left reach are given back; the rows in reach are made, those the list is first being filled with in the order they are
        // wanted in (the one the focus is on, the rest of those in view, then the margin), as many as its budget allows
        let margin = 1.5 * sz.row + tallest.min(usable);
        let (lo, hi) = (self.offset - margin, self.offset + usable + margin);
        let origin = (self.origin.0, self.origin.1 + self.zone - self.offset);
        let mut wanted = Vec::new();
        for i in 0..self.slots.len() {
            let (top, h) = (self.slots[i].top, self.slots[i].geom.h);
            let near = top + h > lo && top < hi;
            if near && self.slots[i].art.is_none() {
                wanted.push(i);
            } else if !near && self.slots[i].art.is_some() {
                if let Some(a) = self.slots[i].art.take() {
                    a.release(engine, false);
                }
                self.slots[i].live = false;
            }
        }
        let budget = self.budget.filter(|_| self.filling);
        if budget.is_some() {
            let (v0, v1) = (self.offset, self.offset + usable);
            wanted.sort_by_key(|&i| (stop != Some(i), !(self.slots[i].top + self.slots[i].geom.h > v0 && self.slots[i].top < v1), i));
        }
        let mut left = budget.unwrap_or(usize::MAX);
        let (mut made, mut waiting, mut rows_made) = (0usize, false, 0usize);
        for i in wanted {
            if left == 0 && rows_made > 0 {
                waiting = true;
                continue;
            }
            let (top, row) = (self.slots[i].top, &rows[i]);
            let geom = self.slots[i].geom.clone();
            let room = kit.ui.scene.layer_room();
            let mut env = Env { kit, engine, paint, sz, scroll: &self.scroll, z: self.z, region: self.region, scope: self.scope, origin };
            let art = RowArt::build(&mut env, row, geom, top, w, id_at(self.id_base, i), row.focusable())?;
            let used = room.saturating_sub(kit.ui.scene.layer_room());
            (made, left, rows_made) = (made + used, left.saturating_sub(used), rows_made + 1);
            self.slots[i].art = Some(art);
        }
        self.made = made;
        self.pending = waiting;
        // a list made before it has rows (the rail, before the model has named them) is not filled yet
        self.filling &= waiting || self.slots.is_empty();
        // the focus plate (a lift group and a lit group) is made for the row the focus is on and given back once its lift is at rest: a page
        // holds two or three, not one for every row it shows
        for i in 0..self.slots.len() {
            let Some(art) = self.slots[i].art.as_mut() else { continue };
            art.follow_card((focused && stop == Some(i)).then_some(seg));
            if stop == Some(i) || !art.at_rest(engine, now_ns) {
                let mut env = Env { kit, engine, paint, sz, scroll: &self.scroll, z: self.z, region: self.region, scope: self.scope, origin };
                art.ensure_plates(&mut env, (focused && stop == Some(i)).then_some(seg), now_ns)?;
            } else {
                art.drop_plates(engine);
            }
        }
        // a row that was to be made again and is out of reach now gives its lift back too
        for i in 0..self.slots.len() {
            if self.slots[i].art.is_none() {
                let id = id_at(self.id_base, i);
                engine.flow.drop_lift(id);
                for c in 0..rows[i].cards.len() {
                    engine.flow.drop_lift(Id(id.0 + 1 + c as u32));
                }
            }
        }
        // the focus frames follow the scroll, and a row that is whole in view takes the pointer, one that is not does not
        let whole_set = |i: usize| whole.contains(&i) || self.col.total <= self.size.1;
        for i in 0..self.slots.len() {
            let Some(art) = self.slots[i].art.as_ref() else { continue };
            let frame = ui::geometry::Frame::new(origin.0, origin.1 + self.slots[i].top, w, self.slots[i].geom.h);
            let want_live = rows[i].focusable() && whole_set(i);
            art.sync_focus(engine, frame, self.region, self.scope, want_live);
            self.slots[i].live = want_live;
        }
        // what moves in place
        for i in 0..self.slots.len() {
            let is_stop = stop == Some(i);
            let moment = Moment { focused: focused && is_stop, hovered: hovered == Some(i), seg: if focused && is_stop { Some(seg) } else { None } };
            let Some(art) = self.slots[i].art.as_mut() else { continue };
            let mut env = Env { kit, engine, paint, sz, scroll: &self.scroll, z: self.z, region: self.region, scope: self.scope, origin };
            art.update(&mut env, &rows[i], moment, at_once);
        }
        let _ = Kind::Head;
        Ok(())
    }

    /// Gives every row and the scroller back to the engine (the list is going).
    pub fn release(&mut self, engine: &mut Engine) {
        for s in &self.slots {
            if let Some(a) = &s.art {
                a.release(engine, false);
            }
        }
        self.slots.clear();
        engine.flow.drop_scroller(self.region);
    }

    /// The cue of the pointer on one card of the row `row`.
    pub fn set_card_hover(&mut self, row: usize, card: Id, on: bool) {
        if let Some(a) = self.slots.get_mut(row).and_then(|s| s.art.as_mut()) {
            a.set_card_hover(card, on);
        }
    }

    /// The parts of row `i`: where its control and its info mark are on the screen, for the pointer.
    pub fn control_of(&self, i: usize) -> Option<(ui::geometry::Frame, Option<ui::geometry::Frame>, Option<ui::geometry::Frame>)> {
        let f = self.frame_of(i)?;
        let a = self.art(i)?;
        let abs = |r: ui::geometry::Frame| ui::geometry::Frame::new(f.x + r.x, f.y + r.y, r.w, r.h);
        Some((f, a.control.map(abs), a.info.map(abs)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::settings::row::Card;

    #[test]
    fn replacing_rows_releases_obsolete_focus_groups() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = Engine::new(&kit).unwrap();
        let mut list = ListArt::new(&mut kit, &mut engine, None, "test".into(), (0.0, 0.0), (900.0, 600.0), Region(42), Scope::BASE, 0, ROW_ID, false).unwrap();
        let paint = Paint::of(&kit);
        let cards = || (0..3).map(|i| Card { title: format!("Card {i}"), ..Card::default() }).collect();
        let mut row = Row::cards("gallery", cards());
        list.apply(&mut kit, &mut engine, paint, &[row.clone()], Some(0), None, 2, true, 1_000_000, true).unwrap();
        let removed = Id(ROW_ID + 3);
        assert!(engine.flow.has(removed));
        row.cards.truncate(1);
        list.apply(&mut kit, &mut engine, paint, &[row.clone()], Some(0), None, 0, true, 2_000_000, true).unwrap();
        assert!(!engine.flow.has(removed));
        row.id = "replacement".into();
        list.apply(&mut kit, &mut engine, paint, &[row], None, None, 0, false, 3_000_000, true).unwrap();
        assert!(!engine.flow.has(Id(ROW_ID + 1)));
        list.release(&mut engine);
        drop(list);
    }

    #[test]
    fn a_list_being_filled_makes_the_rows_its_budget_allows_the_focus_first_and_the_rest_at_the_next_apply() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut engine = Engine::new(&kit).unwrap();
        let paint = Paint::of(&kit);
        let rows: Vec<Row> = (0..12).map(|i| Row::toggle(&format!("t{i}")).label(&format!("Switch {i}"))).collect();
        let new = |kit: &mut Kit, engine: &mut Engine| ListArt::new(kit, engine, None, "test".into(), (0.0, 0.0), (900.0, 640.0), Region(42), Scope::BASE, 0, ROW_ID, false).unwrap();
        let apply = |list: &mut ListArt, kit: &mut Kit, engine: &mut Engine, stop: usize| list.apply(kit, engine, paint, &rows, Some(stop), None, 0, true, 1_000_000, false).unwrap();

        // with no budget every row in reach is made at once
        let mut whole = new(&mut kit, &mut engine);
        apply(&mut whole, &mut kit, &mut engine, 0);
        let in_reach = whole.built();
        assert!(in_reach > 3 && !whole.pending());
        let per_row = whole.made() / in_reach;
        assert!(per_row >= 2, "a switch is more than one layer ({per_row})");
        whole.release(&mut engine);

        // a budget of two rows' layers makes two rows a draw, until every row in reach is made
        let mut list = new(&mut kit, &mut engine);
        list.budget = Some(2 * per_row);
        apply(&mut list, &mut kit, &mut engine, 0);
        assert_eq!((list.built(), list.made()), (2, 2 * per_row), "two rows of the twelve");
        assert!(list.pending());
        let mut draws = 1;
        while list.pending() {
            let before = list.built();
            apply(&mut list, &mut kit, &mut engine, 0);
            assert!(list.built() > before, "every draw makes a row");
            draws += 1;
            assert!(draws < 20);
        }
        assert_eq!(list.built(), in_reach, "the rows in reach, as with no budget");
        // and a list that is filled is made no more by a budget: rows that come into reach as it scrolls are made at once
        list.budget = Some(1);
        apply(&mut list, &mut kit, &mut engine, 0);
        assert!(!list.pending());
        list.release(&mut engine);

        // the row the focus is on is made first, and the rest of the rows in view after it
        let mut list = new(&mut kit, &mut engine);
        list.budget = Some(per_row);
        apply(&mut list, &mut kit, &mut engine, 3);
        assert!(list.art(3).is_some(), "the focused row");
        assert_eq!(list.built(), 1);
        apply(&mut list, &mut kit, &mut engine, 3);
        assert!(list.art(0).is_some(), "then the top of the view");
        list.release(&mut engine);
    }

    /// Every built switch of `list`: its stored value and where its thumb is at 0, 50 ms and 2 s after the frame.
    fn switches(h: &mut ui::testing::Harness, list: &ListArt, n: usize) -> Vec<(bool, [f32; 3])> {
        h.commit();
        let at = [0, 50_000_000, 2_000_000_000].map(|t| ui::reference::flatten(&h.all(), 0, t));
        (0..n)
            .filter_map(|i| list.art(i)?.switch_probe())
            .map(|(id, on)| (on, at.each_ref().map(|layers| layers.iter().find(|l| l.id == id).expect("the thumb is in the scene").props.x)))
            .collect()
    }

    fn assert_still(seen: &[(bool, [f32; 3])], what: &str) {
        let (on_x, off_x) = (seen.iter().find(|s| s.0).map(|s| s.1[0]), seen.iter().find(|s| !s.0).map(|s| s.1[0]));
        for (on, x) in seen {
            assert!(x[0] == x[1] && x[1] == x[2], "{what}: a switch is moving ({x:?})");
            assert_eq!(Some(x[0]), if *on { on_x } else { off_x }, "{what}: a switch is not where its value says");
        }
        if let (Some(on), Some(off)) = (on_x, off_x) {
            assert!(on > off, "{what}: on sits to the right of off");
        }
    }

    #[test]
    fn a_switch_shows_its_value_at_once_when_scrolled_into_view_made_again_or_a_page_opens_and_glides_only_on_a_change() {
        let (mut h, mut kit) = testkit::kit(1.0);
        let mut engine = Engine::new(&kit).unwrap();
        let paint = Paint::of(&kit);
        let mut rows: Vec<Row> = (0..40).map(|i| Row::toggle(&format!("t{i}")).label(&format!("Switch {i}")).on(i % 3 != 1)).collect();
        let new = |kit: &mut Kit, engine: &mut Engine| ListArt::new(kit, engine, None, "test".into(), (0.0, 0.0), (900.0, 400.0), Region(42), Scope::BASE, 0, ROW_ID, false).unwrap();
        let mut now = 1_000_000;
        let mut list = new(&mut kit, &mut engine);
        // the page opens: the first frame already shows the stored values
        list.apply(&mut kit, &mut engine, paint, &rows, Some(0), None, 0, true, now, false).unwrap();
        assert_still(&switches(&mut h, &list, rows.len()), "on entering the page");
        // the focus walks the page down and back, so rows come into reach and leave it
        for i in (0..rows.len()).chain((0..rows.len()).rev()) {
            now += 16_000_000;
            list.apply(&mut kit, &mut engine, paint, &rows, Some(i), None, 0, true, now, false).unwrap();
            let seen = switches(&mut h, &list, rows.len());
            assert!(!seen.is_empty());
            assert_still(&seen, &format!("scrolled to row {i}"));
        }
        // a row made again (its words changed) keeps its value
        for r in &mut rows {
            r.detail = "unavailable".into();
        }
        list.apply(&mut kit, &mut engine, paint, &rows, Some(0), None, 0, true, now, false).unwrap();
        assert_still(&switches(&mut h, &list, rows.len()), "made again");
        // another page and back: a new list per page
        list.release(&mut engine);
        let mut list = new(&mut kit, &mut engine);
        list.apply(&mut kit, &mut engine, paint, &rows, Some(0), None, 0, true, now, false).unwrap();
        assert_still(&switches(&mut h, &list, rows.len()), "on entering the page again");
        // a real change still glides
        rows[0].on = !rows[0].on;
        list.apply(&mut kit, &mut engine, paint, &rows, Some(0), None, 0, true, now + 16_000_000, false).unwrap();
        let moved = switches(&mut h, &list, rows.len());
        assert!(moved[0].1[0] != moved[0].1[2], "a changed switch slides");
        list.release(&mut engine);
    }
}
