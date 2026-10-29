//! A virtualised list of rows (design section 3.3, `Row`/`VirtualList`): a long list that keeps only the rows near its window
//! drawn, in a fixed ring of prepared tiles that are drawn again for another row and never made again.
//!
//! Rows are found by their [`RowId`], never by their place. The scroll position is not an index into the vector the caller last
//! gave: when the rows change, the list keeps the row at the top of its window where it was (an insertion above it moves nothing
//! the person is looking at), or the nearest row that is still there when that one has gone.
//!
//! A finger that goes down on a row holds the contact as a possible press; it becomes a scroll once it has moved past the slop
//! ([`super::swipe::SLOP`]), and the press is cancelled at that moment, so a row never activates at the end of a scroll. A scroll
//! follows the finger sample by sample, and on release the whole fling is one glide per tile, played by the compositor: nothing
//! is drawn or made while the list moves (the tiles of its window are all there is; a fling is cut to the rows already drawn).
//! The rows past the window's edge are drawn a few at a time when the list is at rest ([`VirtualList::tick`]).

use super::group::Pose;
use super::row::{Needs, Row, RowAction, RowId, RowModel};
use super::scroll::{Recycler, Room, Scroller, items_in, room_for};
use super::semantics::SemanticsSink;
use super::swipe::SLOP;
use super::{Component, Damage};
use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::{Event, TouchEvent, TouchPhase};
use crate::lucent::tokens::{motion, space};
use compositor_protocol::{Edges, Rect};
use std::collections::HashSet;
use std::io;

/// Rows drawn beyond each edge of the window, so that a short fling never reaches an empty tile.
pub const MARGIN: usize = 2;
/// Rows a tick at rest may draw: a burst of them never lands on one frame.
pub const DRAW_PER_TICK: usize = 2;
/// Between two rows.
pub const GAP: f32 = space::S2;

/// The rows of a list, in order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ListModel {
    pub rows: Vec<RowModel>,
}

/// What a touch on a list asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListAction {
    /// A row was pressed and released on itself: what the row says (`Activated`, or a toggle's proposed value).
    Row(RowId, RowAction),
}

#[derive(Clone, Copy, Debug)]
enum Contact {
    None,
    /// Down on the list, not yet past the slop: a press if the finger lifts here.
    Pending {
        id: u32,
        down: (u64, f32),
        tile: Option<usize>,
    },
    /// The scroll holds the contact.
    Scrolling {
        id: u32,
    },
}

/// A reason a model cannot be shown.
fn duplicate(rows: &[RowModel]) -> Option<RowId> {
    let mut seen = HashSet::with_capacity(rows.len());
    rows.iter().map(|r| r.id).find(|id| !seen.insert(*id))
}

pub struct VirtualList {
    view: Frame,
    z: i32,
    row_h: f32,
    needs: Needs,
    rows: Vec<RowModel>,
    tiles: Vec<Row>,
    recycler: Recycler,
    scroller: Scroller,
    /// Where the tiles are placed or going: the top of the window in content units.
    pos: f32,
    contact: Contact,
    /// The newest time the list has seen (a touch, a tick): what "now" means to its estimates.
    clock_ns: u64,
    /// When the compositor will next show a frame, as the host knows it; a fling starts then.
    frame_ns: u64,
    /// A model that needs layers or room the tiles do not have, held until the list is at rest.
    deferred: Option<Vec<RowModel>>,
    /// How a page holding the list poses it (slid sideways, faded); the rows scroll inside it.
    pose: Pose,
    /// How far the rows fade out towards the edges of the window; none is a hard edge.
    feather: Edges,
}

impl VirtualList {
    /// A list showing `rows` in the window `view`. The tiles are as tall as the tallest row needs at the person's text size
    /// ([`Row::required_height`]) and carry the layers of every row, so any tile can show any row.
    pub fn new(ui: &mut Ui, view: Frame, z: i32, rows: &[RowModel]) -> io::Result<Self> {
        if let Some(id) = duplicate(rows) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("row {} is in the list twice", id.0),
            ));
        }
        let (row_h, needs) = Self::measure(ui, view.w, rows);
        let mut list = Self {
            view,
            z,
            row_h,
            needs,
            rows: rows.to_vec(),
            tiles: Vec::new(),
            recycler: Recycler::new(0),
            scroller: Scroller::new(0.0, 0.0),
            pos: 0.0,
            contact: Contact::None,
            clock_ns: 0,
            frame_ns: 0,
            deferred: None,
            pose: Pose::REST,
            feather: Edges::NONE,
        };
        list.build_tiles(ui)?;
        list.scroller = Scroller::new(0.0, list.max_pos());
        list.refill(ui, list.visible_budget());
        Ok(list)
    }

    /// The row height every tile has, and the layers every tile has, for `rows` at `width`.
    fn measure(ui: &mut Ui, width: f32, rows: &[RowModel]) -> (f32, Needs) {
        let mut h = 0.0f32;
        let mut needs = Needs::default();
        for row in rows {
            h = h.max(Row::required_height(ui, row, width));
            needs = needs.merge(Row::needs_of(row));
        }
        (h.max(crate::lucent::tokens::smart::TARGET), needs)
    }

    fn pitch(&self) -> f32 {
        self.row_h + GAP
    }

    fn capacity(&self) -> usize {
        (self.view.h / self.pitch()).ceil() as usize + 1 + 2 * MARGIN
    }

    fn visible_budget(&self) -> usize {
        (self.view.h / self.pitch()).ceil() as usize + 1
    }

    fn clip(&self) -> Rect {
        Rect {
            x: self.view.x,
            y: self.view.y,
            w: self.view.w,
            h: self.view.h,
        }
    }

    fn build_tiles(&mut self, ui: &mut Ui) -> io::Result<()> {
        let frame = Frame::new(self.view.x, self.view.y, self.view.w, self.row_h);
        let capacity = self.capacity();
        let mut tiles = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            let mut tile = Row::spare(ui, frame, self.z, self.needs)?;
            tile.set_clip_feathered(Some(self.clip()), self.feather);
            tile.set_pose(self.pose);
            tiles.push(tile);
        }
        self.tiles = tiles;
        self.recycler = Recycler::new(capacity);
        Ok(())
    }

    /// How far the list scrolls.
    fn max_pos(&self) -> f32 {
        Self::max_for(self.rows.len(), self.pitch(), self.view.h)
    }

    fn max_for(count: usize, pitch: f32, view_h: f32) -> f32 {
        if count == 0 {
            0.0
        } else {
            (count as f32 * pitch - GAP - view_h).max(0.0)
        }
    }

    fn y_of(&self, index: usize, pos: f32) -> f32 {
        self.view.y + index as f32 * self.pitch() - pos
    }

    /// The rows the window reaches at `pos`, with the margin.
    fn want(&self, pos: f32) -> std::ops::Range<usize> {
        let visible = items_in(
            self.view.y,
            self.pitch(),
            self.rows.len(),
            self.view.y + pos,
            self.view.y + pos + self.view.h,
        );
        visible.start.saturating_sub(MARGIN)..(visible.end + MARGIN).min(self.rows.len())
    }

    /// The row at the middle of the window at `pos`: what a fling is cut around.
    fn focus(&self, pos: f32) -> usize {
        (((pos + self.view.h / 2.0) / self.pitch()).floor().max(0.0) as usize)
            .min(self.rows.len().saturating_sub(1))
    }

    /// Draws up to `budget` of the rows the window is missing, nearest the middle first.
    fn refill(&mut self, ui: &mut Ui, budget: usize) -> bool {
        let want = self.want(self.pos);
        let before: Vec<(usize, usize)> = self.recycler.resident().collect();
        let assigned = self.recycler.assign(&want, self.focus(self.pos), budget);
        let mut drawn = false;
        for &(slot, _) in &before {
            if self.recycler.row_of(slot).is_none() {
                self.tiles[slot].hide(ui);
            }
        }
        for (slot, row) in assigned {
            let y = self.y_of(row, self.pos);
            let tile = &mut self.tiles[slot];
            match tile.try_update(&self.rows[row], ui) {
                Ok(d) => drawn |= d >= Damage::Content,
                Err(_) => {
                    // The tile stays empty; the next tick tries again.
                    self.recycler.remap(|r| (r != row).then_some(r));
                    continue;
                }
            }
            tile.place(ui, y);
        }
        drawn
    }

    /// Puts every drawn tile where its row is at `pos`.
    fn layout(&mut self, ui: &Ui) {
        let resident: Vec<(usize, usize)> = self.recycler.resident().collect();
        for (slot, row) in resident {
            let y = self.y_of(row, self.pos);
            self.tiles[slot].place(ui, y);
        }
    }

    fn glide(&mut self, pos: f32, ms: u32, ease: compositor_protocol::Ease) {
        self.pos = pos;
        let resident: Vec<(usize, usize)> = self.recycler.resident().collect();
        for (slot, row) in resident {
            let y = self.y_of(row, pos);
            self.tiles[slot].glide(y, ms, ease);
        }
    }

    /// Poses the whole list at once (slid, faded): every drawn row moves with it and the rows to come are drawn in it. The window
    /// that clips the rows stays where it is.
    pub fn set_pose(&mut self, ui: &Ui, pose: Pose) {
        self.pose = pose;
        for tile in &mut self.tiles {
            tile.set_pose(pose);
        }
        self.layout(ui);
    }

    /// Poses the whole list on the compositor's clock: one `Animate` per layer of every drawn row.
    pub fn animate_pose(&mut self, pose: Pose, duration_ms: u32, ease: compositor_protocol::Ease) {
        self.pose = pose;
        let pos = self.pos;
        let resident: Vec<(usize, usize)> = self.recycler.resident().collect();
        for (slot, row) in resident {
            let y = self.y_of(row, pos);
            self.tiles[slot].set_pose(pose);
            self.tiles[slot].glide(y, duration_ms, ease);
        }
        for tile in &mut self.tiles {
            tile.set_pose(pose);
        }
    }

    /// The pose the list is in.
    pub fn pose(&self) -> Pose {
        self.pose
    }

    /// Tells the list when the compositor will next show a frame (a fling starts then, and the list's estimate of where a flight
    /// has got is measured from it). Without it a release's own time stands in, which is early by at most one refresh.
    pub fn set_frame_clock(&mut self, next_frame_ns: u64) {
        self.frame_ns = next_frame_ns;
    }

    /// The window of the list.
    pub fn view(&self) -> Frame {
        self.view
    }

    /// The rows fade out over `feather` towards the edges of the window (none: a hard edge). Applies to the tiles that exist, the
    /// drawn ones at once, and to every tile made later.
    pub fn set_feather(&mut self, feather: Edges) {
        self.feather = feather;
        let (clip, pose) = (self.clip(), self.pose);
        for tile in &mut self.tiles {
            tile.set_clip_feathered(Some(clip), feather);
            tile.set_pose(pose);
        }
    }

    /// How the rows fade towards the edges of the window.
    pub fn feather(&self) -> Edges {
        self.feather
    }

    /// How tall each row is, and how far apart two are (the height and the gap).
    pub fn pitch_of_rows(&self) -> (f32, f32) {
        (self.row_h, GAP)
    }

    pub fn rows(&self) -> &[RowModel] {
        &self.rows
    }

    /// How many tiles there are: the most rows ever drawn however long the list is.
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// How many layers the list holds, all the tiles'.
    pub fn layer_count(&self) -> usize {
        self.tiles.iter().map(|t| t.group().parts.len()).sum()
    }

    /// Where the window is, in content units from the top (the estimate while a flight plays).
    pub fn position(&self) -> f32 {
        self.scroller.position_at(self.clock_ns)
    }

    /// The row at the top of the window and how far its top is above the window's top: what survives a change of rows.
    pub fn anchor(&self) -> Option<(RowId, f32)> {
        let pos = self.position();
        let index =
            ((pos / self.pitch()).floor().max(0.0) as usize).min(self.rows.len().checked_sub(1)?);
        Some((self.rows[index].id, pos - index as f32 * self.pitch()))
    }

    /// Where on screen the top of row `id` is, if the list has the row.
    pub fn row_top(&self, id: RowId) -> Option<f32> {
        let index = self.rows.iter().position(|r| r.id == id)?;
        Some(self.y_of(index, self.position()))
    }

    /// The rows drawn right now, in list order.
    pub fn drawn(&self) -> Vec<RowId> {
        let mut shown: Vec<(usize, RowId)> = self
            .recycler
            .resident()
            .map(|(_, row)| (row, self.rows[row].id))
            .collect();
        shown.sort();
        shown.into_iter().map(|(_, id)| id).collect()
    }

    /// Whether a finger holds the list or a fling is still playing.
    pub fn is_moving(&self) -> bool {
        self.scroller.is_moving(self.clock_ns) || !matches!(self.contact, Contact::None)
    }

    /// Whether the window still has rows to draw (the host keeps ticking while this is true).
    pub fn filling(&self) -> bool {
        let want = self.want(self.pos);
        want.clone().any(|r| self.recycler.slot_of(r).is_none()) || self.deferred.is_some()
    }

    /// Whether row `id` is drawn pressed.
    pub fn is_pressed(&self, id: RowId) -> bool {
        self.recycler
            .resident()
            .any(|(slot, row)| self.rows[row].id == id && self.tiles[slot].is_pressed())
    }

    /// Ends a press and a scroll that something else has taken (a pager that began to turn, a sheet that began to follow the
    /// finger): nothing activates and a scroll stops where it is.
    pub fn cancel(&mut self, ui: &Ui) {
        if let Contact::Pending {
            tile: Some(slot), ..
        } = self.contact
        {
            self.tiles[slot].cancel(ui);
        }
        if matches!(self.contact, Contact::Scrolling { .. }) {
            let start = self.frame_ns.max(self.clock_ns);
            if let Some(plan) = self.scroller.stop(start) {
                self.glide(plan.to, plan.duration_ms, plan.ease);
            } else {
                self.pos = self.scroller.position_at(self.clock_ns);
            }
        }
        self.contact = Contact::None;
    }

    /// The list at rest draws what is missing of its margin, a few rows at a time, and lands a fling that has finished. Call it
    /// every tick; it never draws while a finger holds the list or a fling plays.
    pub fn tick(&mut self, ui: &mut Ui, now_ns: u64) -> Damage {
        self.clock_ns = self.clock_ns.max(now_ns);
        self.scroller.finish(self.clock_ns);
        if self.is_moving() {
            return Damage::None;
        }
        if let Some(rows) = self.deferred.take() {
            return match self.try_update_rows(ui, rows) {
                Ok(d) => d,
                Err(_) => Damage::None,
            };
        }
        if self.refill(ui, DRAW_PER_TICK) {
            Damage::Content
        } else {
            Damage::Properties
        }
    }

    /// Shows `rows`, keeping the row at the top of the window where it is. Rows are matched by id: a row that changed is drawn
    /// again in its tile, a row that went frees its tile, a new one is drawn when the window reaches it. A model that needs
    /// layers or room the tiles lack (a longer text at large size, a toggle where there was none) waits until the list is at rest.
    pub fn try_update(&mut self, rows: &[RowModel], ui: &mut Ui) -> io::Result<Damage> {
        self.try_update_rows(ui, rows.to_vec())
    }

    fn try_update_rows(&mut self, ui: &mut Ui, rows: Vec<RowModel>) -> io::Result<Damage> {
        if let Some(id) = duplicate(&rows) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("row {} is in the list twice", id.0),
            ));
        }
        if rows == self.rows && self.deferred.is_none() {
            return Ok(Damage::None);
        }
        self.deferred = None;
        let (row_h, needs) = Self::measure(ui, self.view.w, &rows);
        let rebuild = row_h > self.row_h || !self.needs.covers(&needs);
        if rebuild && self.is_moving() {
            self.deferred = Some(rows);
            return Ok(Damage::None);
        }
        let position = self.position();
        let anchor = self.anchor();
        let old: Vec<RowId> = self.rows.iter().map(|r| r.id).collect();
        let anchor_index = anchor.and_then(|(id, _)| old.iter().position(|o| *o == id));
        self.rows = rows;
        if rebuild {
            self.row_h = row_h.max(self.row_h);
            self.needs = self.needs.merge(needs);
            self.build_tiles(ui)?;
        } else {
            let new_index: std::collections::HashMap<RowId, usize> = self
                .rows
                .iter()
                .enumerate()
                .map(|(i, r)| (r.id, i))
                .collect();
            let freed_before: Vec<(usize, usize)> = self.recycler.resident().collect();
            self.recycler
                .remap(|row| old.get(row).and_then(|id| new_index.get(id)).copied());
            for (slot, _) in freed_before {
                if self.recycler.row_of(slot).is_none() {
                    self.tiles[slot].hide(ui);
                }
            }
        }
        // Where the window is now: the anchor row where it was, or the nearest row after it that is still here, else before.
        let pitch = self.pitch();
        let mut pos = position;
        if let (Some(index), Some((_, within))) = (anchor_index, anchor) {
            let survivor = (index..old.len()).chain((0..index).rev()).find_map(|i| {
                self.rows
                    .iter()
                    .position(|r| r.id == old[i])
                    .map(|j| (i, j))
            });
            if let Some((i, j)) = survivor {
                pos = j as f32 * pitch + if i == index { within } else { 0.0 };
            }
        }
        let max = self.max_pos();
        self.pos = pos.clamp(0.0, max);
        self.scroller.reframe(0.0, max, self.pos);
        if let Contact::Pending { tile, .. } = &mut self.contact {
            // The pressed row may have moved or gone; it is no longer a press.
            if let Some(slot) = tile.take() {
                self.tiles[slot].cancel(ui);
            }
        }
        // Rows a tile shows keep their layers; only what differs is drawn.
        let resident: Vec<(usize, usize)> = self.recycler.resident().collect();
        let mut damage = Damage::Properties;
        for (slot, row) in resident {
            if self.tiles[slot].try_update(&self.rows[row], ui)? >= Damage::Content {
                damage = Damage::Content;
            }
        }
        if self.refill(ui, self.visible_budget()) {
            damage = Damage::Content;
        }
        self.layout(ui);
        Ok(damage)
    }

    fn row_at(&self, y: f32) -> Option<usize> {
        let at = y - self.view.y + self.pos;
        let index = (at / self.pitch()).floor();
        (at >= 0.0
            && index >= 0.0
            && (index as usize) < self.rows.len()
            && at - index * self.pitch() < self.row_h)
            .then_some(index as usize)
    }

    fn touch(&mut self, t: &TouchEvent, ui: &mut Ui) -> Option<ListAction> {
        self.clock_ns = self.clock_ns.max(t.at_ns);
        match (t.phase, self.contact) {
            (TouchPhase::Down, Contact::None) => {
                if !self.view.contains(t.x, t.y) {
                    return None;
                }
                if self.scroller.flight().is_some_and(|p| !p.is_done(t.at_ns)) {
                    // A finger on a fling catches it where the list has got to: no press.
                    if let Some(at) = self.scroller.begin(t.at_ns, t.y) {
                        self.glide(at, motion::CATCH_MS, motion::SETTLE);
                    }
                    self.contact = Contact::Scrolling { id: t.id };
                    return None;
                }
                let tile = self
                    .row_at(t.y)
                    .and_then(|row| self.recycler.slot_of(row))
                    .filter(|&slot| self.tiles[slot].model().actionable);
                if let Some(slot) = tile {
                    self.tiles[slot].input(&Event::Touch(*t), ui);
                }
                self.contact = Contact::Pending {
                    id: t.id,
                    down: (t.at_ns, t.y),
                    tile,
                };
                None
            }
            (TouchPhase::Move, Contact::Pending { id, down, tile }) if id == t.id => {
                if (t.y - down.1).abs() <= SLOP {
                    if let Some(slot) = tile {
                        self.tiles[slot].input(&Event::Touch(*t), ui);
                    }
                    return None;
                }
                if let Some(slot) = tile {
                    self.tiles[slot].cancel(ui);
                }
                self.scroller.take(down);
                self.contact = Contact::Scrolling { id };
                self.follow(ui, t);
                None
            }
            (TouchPhase::Move, Contact::Scrolling { id }) if id == t.id => {
                self.follow(ui, t);
                None
            }
            (TouchPhase::Up, Contact::Pending { id, tile, .. }) if id == t.id => {
                self.contact = Contact::None;
                let slot = tile?;
                let row = self.recycler.row_of(slot)?;
                let action = self.tiles[slot].input(&Event::Touch(*t), ui)?;
                Some(ListAction::Row(self.rows[row].id, action))
            }
            (TouchPhase::Up, Contact::Scrolling { id }) if id == t.id => {
                self.contact = Contact::None;
                let focus = self.focus(self.pos);
                let resident = self.recycler.run_around(focus);
                let room = if self.rows.is_empty() {
                    Room::symmetric(0.0)
                } else {
                    room_for(self.view.y, self.pitch(), &resident, self.pos, self.view.h)
                };
                let start = self.frame_ns.max(t.at_ns);
                if let Some(plan) = self.scroller.release(t.at_ns, start, room) {
                    self.glide(plan.to, plan.duration_ms, plan.ease);
                }
                None
            }
            (TouchPhase::Cancel, Contact::Pending { id, tile, .. }) if id == t.id => {
                self.contact = Contact::None;
                if let Some(slot) = tile {
                    self.tiles[slot].input(&Event::Touch(*t), ui);
                    self.tiles[slot].cancel(ui);
                }
                None
            }
            (TouchPhase::Cancel, Contact::Scrolling { id }) if id == t.id => {
                self.cancel(ui);
                None
            }
            _ => None,
        }
    }

    /// The finger moved: the window follows it, drawing at most a few rows of what it is about to reach.
    fn follow(&mut self, ui: &mut Ui, t: &TouchEvent) {
        self.pos = self.scroller.drag(t.at_ns, t.y);
        self.refill(ui, DRAW_PER_TICK);
        self.layout(ui);
    }
}

impl Component for VirtualList {
    type Model = ListModel;
    type Action = ListAction;

    fn update(&mut self, model: &ListModel, ui: &mut Ui) -> Damage {
        // A model the list cannot show (a repeated id) leaves it as it was; `try_update` says why.
        self.try_update(&model.rows, ui).unwrap_or(Damage::None)
    }

    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<ListAction> {
        match event {
            Event::Touch(t) => self.touch(t, ui),
            Event::Gesture(_) => None,
        }
    }

    fn frame(&self) -> Frame {
        self.view
    }

    fn semantics(&self, out: &mut SemanticsSink) {
        let mut resident: Vec<(usize, usize)> = self
            .recycler
            .resident()
            .map(|(slot, row)| (row, slot))
            .collect();
        resident.sort();
        for (_, slot) in resident {
            let tile = &self.tiles[slot];
            let top = tile.visual().y;
            if top + self.row_h > self.view.y && top < self.view.bottom() {
                tile.semantics(out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::row::Trailing;
    use crate::testing::{Harness, harness};
    use compositor_protocol::Request;

    const MS: u64 = 1_000_000;
    const VIEW: Frame = Frame::new(40.0, 80.0, 600.0, 300.0);

    fn rows(ids: impl IntoIterator<Item = u64>) -> Vec<RowModel> {
        ids.into_iter()
            .map(|i| RowModel::new(RowId(i), format!("Row {i}")).actionable())
            .collect()
    }

    fn touch(id: u32, phase: TouchPhase, y: f32, at_ms: u64) -> Event {
        Event::Touch(TouchEvent {
            id,
            phase,
            x: 200.0,
            y,
            at_ns: at_ms * MS,
        })
    }

    fn list(h: &mut Harness, rows: &[RowModel]) -> VirtualList {
        let l = VirtualList::new(&mut h.ui, VIEW, 10, rows).unwrap();
        h.commit();
        l
    }

    fn creates(requests: &[Request]) -> usize {
        requests
            .iter()
            .filter(|r| matches!(r, Request::Create { .. }))
            .count()
    }

    fn animates(requests: &[Request]) -> usize {
        requests
            .iter()
            .filter(|r| matches!(r, Request::Animate { .. }))
            .count()
    }

    /// A finger from `y0` to `y1` in steps 16 ms apart, starting at `t0`; returns the time of the last sample.
    fn drag(l: &mut VirtualList, h: &mut Harness, y0: f32, y1: f32, t0: u64, up: bool) -> u64 {
        l.input(&touch(1, TouchPhase::Down, y0, t0), &mut h.ui);
        let steps = 10;
        let mut t = t0;
        for i in 1..=steps {
            t += 16;
            l.input(
                &touch(
                    1,
                    TouchPhase::Move,
                    y0 + (y1 - y0) * i as f32 / steps as f32,
                    t,
                ),
                &mut h.ui,
            );
        }
        if up {
            l.input(&touch(1, TouchPhase::Up, y1, t), &mut h.ui);
        }
        t
    }

    #[test]
    fn a_long_list_keeps_a_fixed_ring_of_tiles_and_scrolling_makes_no_layer() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..500));
        let tiles = l.tile_count();
        let layers = l.layer_count();
        assert!(tiles < 20, "{tiles}");
        for k in 0..6 {
            drag(&mut l, &mut h, 340.0, 100.0, 1000 + k * 600, true);
            for step in 0..60 {
                l.tick(&mut h.ui, (1500 + k * 600 + step * 10) * MS);
            }
        }
        assert_eq!((l.tile_count(), l.layer_count()), (tiles, layers));
        let sent = h.commit();
        assert_eq!(creates(&sent), 0, "no layer is made while the list scrolls");
        assert!(l.drawn().len() <= tiles);
        assert!(l.position() > 0.0);
    }

    #[test]
    fn a_press_released_on_a_row_activates_that_row_by_id() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..20));
        let y = l.row_top(RowId(2)).unwrap() + 20.0;
        assert_eq!(l.input(&touch(1, TouchPhase::Down, y, 10), &mut h.ui), None);
        assert!(l.is_pressed(RowId(2)));
        assert_eq!(
            l.input(&touch(1, TouchPhase::Move, y + 3.0, 30), &mut h.ui),
            None
        );
        assert_eq!(
            l.input(&touch(1, TouchPhase::Up, y + 3.0, 50), &mut h.ui),
            Some(ListAction::Row(RowId(2), RowAction::Activated))
        );
        assert!(!l.is_pressed(RowId(2)));
    }

    #[test]
    fn a_scroll_cancels_the_press_and_the_release_that_ends_it_activates_nothing() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..20));
        let y = l.row_top(RowId(1)).unwrap() + 20.0;
        l.input(&touch(1, TouchPhase::Down, y, 10), &mut h.ui);
        assert!(l.is_pressed(RowId(1)));
        // Past the slop the contact is the scroll's; the row is back at rest at once.
        l.input(&touch(1, TouchPhase::Move, y - SLOP - 10.0, 40), &mut h.ui);
        assert!(!l.is_pressed(RowId(1)));
        assert!(l.is_moving());
        l.input(&touch(1, TouchPhase::Move, y - SLOP - 50.0, 56), &mut h.ui);
        assert_eq!(
            l.input(&touch(1, TouchPhase::Up, y - SLOP - 50.0, 60), &mut h.ui),
            None
        );
        assert!(l.position() > 0.0, "the content followed the finger");
    }

    #[test]
    fn a_lost_contact_and_a_second_finger_activate_nothing() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..20));
        let y = l.row_top(RowId(0)).unwrap() + 20.0;
        l.input(&touch(1, TouchPhase::Down, y, 10), &mut h.ui);
        assert_eq!(l.input(&touch(2, TouchPhase::Down, y, 12), &mut h.ui), None);
        assert_eq!(l.input(&touch(2, TouchPhase::Up, y, 14), &mut h.ui), None);
        assert!(
            l.is_pressed(RowId(0)),
            "the first finger still holds the row"
        );
        assert_eq!(
            l.input(&touch(1, TouchPhase::Cancel, 0.0, 20), &mut h.ui),
            None
        );
        assert!(!l.is_pressed(RowId(0)));
        assert!(!l.is_moving());
    }

    #[test]
    fn a_contact_outside_the_window_or_in_the_gap_between_rows_does_nothing() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..20));
        assert_eq!(
            l.input(&touch(1, TouchPhase::Down, VIEW.y - 5.0, 10), &mut h.ui),
            None
        );
        assert!(!l.is_moving());
        let (row_h, _) = l.pitch_of_rows();
        let gap = l.row_top(RowId(0)).unwrap() + row_h + 2.0;
        l.input(&touch(1, TouchPhase::Down, gap, 20), &mut h.ui);
        assert_eq!(l.input(&touch(1, TouchPhase::Up, gap, 40), &mut h.ui), None);
    }

    #[test]
    fn rows_inserted_above_the_top_row_move_nothing_the_person_is_looking_at() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(100..160));
        drag(&mut l, &mut h, 340.0, 120.0, 1000, true);
        for step in 0..120 {
            l.tick(&mut h.ui, (1300 + step * 10) * MS);
        }
        let (anchor, within) = l.anchor().unwrap();
        let top = l.row_top(anchor).unwrap();
        // Ten rows arrive above everything, and one in the middle of what is shown.
        let mut next: Vec<RowModel> = rows(0..10);
        next.extend(l.rows().iter().cloned());
        l.try_update(&next, &mut h.ui).unwrap();
        assert_eq!(l.anchor(), Some((anchor, within)));
        assert!(
            (l.row_top(anchor).unwrap() - top).abs() < 0.01,
            "{top} -> {:?}",
            l.row_top(anchor)
        );
        h.commit();
    }

    #[test]
    fn when_the_top_row_goes_the_nearest_row_after_it_takes_its_place() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..60));
        drag(&mut l, &mut h, 340.0, 100.0, 1000, true);
        for step in 0..120 {
            l.tick(&mut h.ui, (1300 + step * 10) * MS);
        }
        let (anchor, _) = l.anchor().unwrap();
        let after = RowId(anchor.0 + 1);
        let next: Vec<RowModel> = l
            .rows()
            .iter()
            .filter(|r| r.id != anchor)
            .cloned()
            .collect();
        l.try_update(&next, &mut h.ui).unwrap();
        assert_eq!(l.anchor().unwrap().0, after);
        assert!(l.row_top(anchor).is_none());
        assert!(l.drawn().iter().all(|id| *id != anchor));
        h.commit();
    }

    #[test]
    fn a_tile_that_kept_its_row_across_an_update_is_not_drawn_again_and_a_changed_row_is_drawn_once()
     {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..30));
        while l.filling() {
            l.tick(&mut h.ui, 100 * MS);
        }
        h.commit();
        let mut next = l.rows().to_vec();
        assert_eq!(l.try_update(&next, &mut h.ui).unwrap(), Damage::None);
        next[2].title = "Renamed".into();
        let damage = l.try_update(&next, &mut h.ui).unwrap();
        assert_eq!(damage, Damage::Content);
        let sent = h.commit();
        let damages = sent
            .iter()
            .filter(|r| matches!(r, Request::Damage { .. }))
            .count();
        assert_eq!(
            damages, 1,
            "only the renamed row's text was drawn again: {sent:?}"
        );
        assert_eq!(creates(&sent), 0);
    }

    #[test]
    fn a_fling_is_one_glide_per_drawn_tile_and_stays_within_the_rows_that_are_drawn() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..500));
        let t = drag(&mut l, &mut h, 340.0, 60.0, 1000, false);
        let drawn_before = l.drawn();
        h.commit();
        l.input(&touch(1, TouchPhase::Up, 60.0, t), &mut h.ui);
        let sent = h.commit();
        let glides = animates(&sent);
        assert!(glides > 0, "the release plays a fling");
        assert!(
            glides <= l.tile_count() * 8,
            "one glide per layer of a drawn tile: {glides}"
        );
        let flung_to = l.position();
        let (row_h, gap) = l.pitch_of_rows();
        let last_drawn = drawn_before.last().unwrap().0 as f32;
        assert!(
            flung_to <= (last_drawn + 1.0) * (row_h + gap) - VIEW.h + 0.5,
            "a fling never goes past the rows that are drawn: {flung_to}"
        );
        assert!(l.is_moving());
        // The compositor plays it; the list lands it when it is over.
        l.tick(&mut h.ui, t * MS + 5_000 * MS);
        assert!(!l.is_moving());
    }

    #[test]
    fn a_finger_on_a_fling_catches_it_and_presses_nothing() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..500));
        let t = drag(&mut l, &mut h, 340.0, 60.0, 1000, true);
        assert!(l.is_moving());
        let y = l.row_top(l.anchor().unwrap().0).unwrap() + 30.0;
        assert_eq!(
            l.input(&touch(2, TouchPhase::Down, y, t + 20), &mut h.ui),
            None
        );
        assert!(l.drawn().iter().all(|id| !l.is_pressed(*id)));
        assert_eq!(
            l.input(&touch(2, TouchPhase::Up, y, t + 30), &mut h.ui),
            None
        );
        h.commit();
    }

    #[test]
    fn the_ends_resist_and_the_list_returns_from_past_them() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..40));
        drag(&mut l, &mut h, 100.0, 330.0, 1000, false);
        assert!(
            l.position() < 0.0,
            "pulled past the start: {}",
            l.position()
        );
        assert!(l.position() > -100.0, "the rubber band limits it");
        l.input(&touch(1, TouchPhase::Up, 330.0, 1200), &mut h.ui);
        l.tick(&mut h.ui, 3_000 * MS);
        assert_eq!(l.position(), 0.0);
        h.commit();
    }

    #[test]
    fn large_text_makes_the_rows_taller_and_a_short_list_does_not_scroll() {
        let mut h = harness();
        let model: Vec<RowModel> = (0..4)
            .map(|i| {
                RowModel::new(
                    RowId(i),
                    "A network with a name that is rather long to read",
                )
                .detail("Secured, strong signal")
                .trailing(Trailing::Chevron)
                .actionable()
            })
            .collect();
        let normal = list(&mut h, &model);
        let (h1, _) = normal.pitch_of_rows();
        h.ui.look.text_scale = 2.0;
        let large = list(&mut h, &model);
        let (h2, _) = large.pitch_of_rows();
        assert!(h2 > h1 * 1.5, "{h1} -> {h2}");
        assert!(large.tile_count() <= normal.tile_count());
    }

    #[test]
    fn a_row_id_listed_twice_is_refused_and_leaves_the_list_as_it_was() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..10));
        let mut twice = rows(0..10);
        twice[5].id = RowId(1);
        assert!(l.try_update(&twice, &mut h.ui).is_err());
        assert_eq!(l.rows(), &rows(0..10)[..]);
        assert_eq!(
            l.update(&ListModel { rows: twice }, &mut h.ui),
            Damage::None
        );
        assert!(VirtualList::new(&mut h.ui, VIEW, 10, &rows([1, 2, 1])).is_err());
    }

    #[test]
    fn a_row_that_needs_a_layer_the_tiles_lack_waits_for_the_list_to_come_to_rest() {
        let mut h = harness();
        let plain: Vec<RowModel> = (0..40)
            .map(|i| RowModel::new(RowId(i), format!("Row {i}")))
            .collect();
        let mut l = list(&mut h, &plain);
        drag(&mut l, &mut h, 340.0, 100.0, 1000, false);
        let mut with_toggle = plain.clone();
        with_toggle[0].trailing = Trailing::Toggle(true);
        assert_eq!(l.try_update(&with_toggle, &mut h.ui).unwrap(), Damage::None);
        assert!(l.filling(), "the change is waiting");
        let layers = l.layer_count();
        l.input(&touch(1, TouchPhase::Cancel, 100.0, 1200), &mut h.ui);
        l.tick(&mut h.ui, 5_000 * MS);
        assert!(
            l.layer_count() > layers,
            "the tiles were rebuilt with the toggle's layers at rest"
        );
        assert_eq!(l.rows()[0].trailing, Trailing::Toggle(true));
        h.commit();
    }

    #[test]
    fn what_is_drawn_is_described_for_semantics_and_hidden_rows_are_not() {
        let mut h = harness();
        let l = list(&mut h, &rows(0..100));
        let mut sink = SemanticsSink::new();
        l.semantics(&mut sink);
        assert!(sink.find("Row 0").is_some());
        assert!(sink.find("Row 99").is_none());
        assert!(sink.nodes().len() < 12);
    }

    #[test]
    fn a_posed_list_moves_every_drawn_row_and_rows_drawn_later_come_in_the_pose() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..40));
        let before: Vec<_> = h.layers().into_iter().map(|(_, _, p)| p).collect();
        l.set_pose(&h.ui, Pose::at(120.0, 0.0));
        h.commit();
        let after: Vec<_> = h.layers().into_iter().map(|(_, _, p)| p).collect();
        let moved = before
            .iter()
            .zip(&after)
            .filter(|(b, a)| b.visible && a.visible)
            .inspect(|(b, a)| {
                assert_eq!((a.x - b.x, a.y), (120.0, b.y), "only sideways");
                assert_eq!(
                    a.clip, b.clip,
                    "the window that clips the rows does not move"
                );
            })
            .count();
        assert!(moved >= l.drawn().len(), "every drawn row moved");
        // Scrolling brings rows into tiles: they are drawn in the pose, not at rest.
        let t = drag(&mut l, &mut h, 300.0, 100.0, 0, true);
        l.tick(&mut h.ui, t * MS + 2_000 * MS);
        h.commit();
        for (_, _, p) in h.layers() {
            if p.visible {
                assert!(
                    p.x >= 40.0 + 120.0 - 1.0,
                    "a row at rest in the page's pose: x {}",
                    p.x
                );
            }
        }
    }

    #[test]
    fn animating_a_list_pose_is_one_animate_per_layer_of_the_drawn_rows() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..40));
        let layers = l.layer_count();
        l.animate_pose(Pose::at(0.0, 16.0), 220, compositor_protocol::Ease::Out);
        let sent = h.commit();
        let n = animates(&sent);
        assert!(n > 0 && n <= layers, "{n} animates for {layers} layers");
        assert_eq!(l.pose(), Pose::at(0.0, 16.0));
    }

    fn assert_window_feather(h: &Harness, fade: Edges, what: &str) {
        let clip = Rect {
            x: VIEW.x,
            y: VIEW.y,
            w: VIEW.w,
            h: VIEW.h,
        };
        let drawn: Vec<_> = h
            .layers()
            .into_iter()
            .filter(|(_, _, p)| p.visible)
            .collect();
        assert!(!drawn.is_empty(), "{what}: nothing is drawn");
        for (id, _, p) in drawn {
            assert_eq!(p.clip, Some(clip), "{what}: layer {id:?}");
            assert_eq!(p.feather, fade, "{what}: layer {id:?}");
        }
    }

    #[test]
    fn a_feathered_list_fades_every_row_layer_towards_the_window_edges_while_it_scrolls_and_for_rows_made_later()
     {
        let fade = Edges::vertical(34.0);
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..60));
        l.set_feather(fade);
        assert_eq!(l.feather(), fade);
        h.commit();
        assert_window_feather(&h, fade, "after set_feather");
        let t = drag(&mut l, &mut h, 340.0, 100.0, 0, true);
        l.tick(&mut h.ui, t * MS + 2_000 * MS);
        h.commit();
        assert!(l.position() > 0.0);
        assert_window_feather(&h, fade, "after a scroll that recycled rows");
        // Tiles built later (a model that needs more layers rebuilds them) are feathered too.
        let mut with_toggle = rows(0..60);
        with_toggle[0].trailing = Trailing::Toggle(true);
        let layers = l.layer_count();
        l.try_update(&with_toggle, &mut h.ui).unwrap();
        l.tick(&mut h.ui, t * MS + 6_000 * MS);
        h.commit();
        assert!(l.layer_count() > layers, "the tiles were rebuilt");
        assert_window_feather(&h, fade, "after the tiles were rebuilt");
    }

    #[test]
    fn a_list_without_a_feather_keeps_a_hard_window_edge() {
        let mut h = harness();
        let mut l = list(&mut h, &rows(0..60));
        assert_eq!(l.feather(), Edges::NONE);
        assert_window_feather(&h, Edges::NONE, "at construction");
        let t = drag(&mut l, &mut h, 340.0, 100.0, 0, true);
        l.tick(&mut h.ui, t * MS + 2_000 * MS);
        h.commit();
        assert_window_feather(&h, Edges::NONE, "after a scroll");
        l.set_feather(Edges::vertical(34.0));
        l.set_feather(Edges::NONE);
        h.commit();
        assert_window_feather(&h, Edges::NONE, "after the feather is taken off");
    }
}
