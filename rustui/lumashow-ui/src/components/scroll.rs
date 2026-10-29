//! One-axis scrolling as a pure model, and the rules that keep a virtualised list honest.
//!
//! The finger drags the content 1:1 from the point it went down on (`take`, then `drag`, applied to the layers with `Set` on every
//! touch sample, resisted past the ends by the one rubber band: [`motion::rubber`]). On release the model returns the whole fling *up
//! front* as a [`Plan`]: where the content will come to rest and for how long, on the compositor's Lucent settle curve (the return
//! from past an end on the spring's release curve), so the screen hands every layer one `Animate` and the compositor moves the content
//! on its own clock. Nothing is drawn or computed per frame during a fling, so its motion is as smooth as the compositor's animations
//! are (position error against the curve well under a pixel), and the fling can only cross rows that are already rastered: the
//! caller passes the room it has ([`Room`]) and the plan never goes past it, so a caller that wants a long fling makes the rows ahead
//! first (see [`Scroller::reach`]).
//!
//! `position_at` is the app's estimate of where a running plan has taken the content, for virtualisation (which rows
//! are still worth keeping) and for catching a fling with a finger; it is good to about a frame.
//!
//! # When a plan starts (the one integration point with the engine)
//!
//! A plan is played by the compositor, and the compositor starts an animation on the first frame after the commit that
//! carries it. The model does not guess when that is: the caller hands [`Scroller::release`] the `start_ns` it knows from
//! the frame clock (for example the presentation time of the next frame) and every estimate afterwards is measured from
//! it. There is deliberately no built-in "about one refresh" constant. When the engine's own scroll motion lands and
//! reports the real start of a scroll, that value is what the caller passes here; nothing else in this file changes.
//! A caller that has no frame clock yet passes the release time itself, which makes the estimate early by at most one
//! refresh.
//!
//! # The pieces
//!
//! * [`Scroller`]: the position of one axis, from the finger (with a rubber band past the ends) and from flights.
//! * [`Plan`], [`Room`]: a flight, and how far it may go before it would run out of drawn rows.
//! * [`items_in`], [`room_for`]: which items of a uniform list a window reaches, and how much room the drawn ones give.
//! * [`Resident`]: items drawn, by index (for strips that keep a window of cells).
//! * [`Recycler`]: a fixed number of slots that show rows by index, reused as the window moves (for lists).

use crate::lucent::tokens::motion;
use compositor_protocol::Ease;
use compositor_protocol::motion::{VELOCITY_WINDOW_NS, ease, slope_per_s, start_slope};
use std::collections::BTreeMap;
use std::ops::Range;

const SAMPLES: usize = 8;

/// A scroll the compositor is playing: from where the content was to where it lands, beginning at `start_ns` on the frame
/// clock (see the module docs), along `ease`: a fling on [`motion::SETTLE`], the return from past an end on the spring's release
/// ([`motion::RELEASE`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub from: f32,
    pub to: f32,
    pub duration_ms: u32,
    pub ease: Ease,
    started_ns: u64,
}

impl Plan {
    /// A plan from `from` to `to` over `duration_ms` on the settle curve (a fling) that the compositor starts at `start_ns`.
    pub fn new(from: f32, to: f32, duration_ms: u32, start_ns: u64) -> Self {
        Self::along(from, to, duration_ms, motion::SETTLE, start_ns)
    }

    /// The same along `ease`.
    pub fn along(from: f32, to: f32, duration_ms: u32, ease: Ease, start_ns: u64) -> Self {
        Self {
            from,
            to,
            duration_ms,
            ease,
            started_ns: start_ns,
        }
    }

    /// When the compositor starts the plan, on the frame clock.
    pub fn start_ns(&self) -> u64 {
        self.started_ns
    }

    /// Where the plan has taken the content at `now_ns`.
    pub fn position_at(&self, now_ns: u64) -> f32 {
        let elapsed_ms = now_ns.saturating_sub(self.started_ns) as f32 / 1e6;
        let t = if self.duration_ms == 0 {
            1.0
        } else {
            elapsed_ms / self.duration_ms as f32
        };
        self.from + (self.to - self.from) * ease(self.ease, t)
    }

    pub fn is_done(&self, now_ns: u64) -> bool {
        now_ns >= self.started_ns + u64::from(self.duration_ms) * 1_000_000
    }

    /// Milliseconds still to run at `now_ns` (what a layer created mid-fling animates for).
    pub fn remaining_ms(&self, now_ns: u64) -> u32 {
        let end = self.started_ns + u64::from(self.duration_ms) * 1_000_000;
        (end.saturating_sub(now_ns) / 1_000_000) as u32
    }
}

/// How far the content may still travel before it runs out of rastered rows, in the direction of a smaller
/// position (`back`) and of a larger one (`forward`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    pub back: f32,
    pub forward: f32,
}

impl Room {
    pub const fn symmetric(cap: f32) -> Self {
        Room {
            back: cap,
            forward: cap,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Rest,
    Drag { anchor_touch: f32, anchor_pos: f32 },
    Flight(Plan),
}

/// The position of one axis of scrolling content, from the finger and from the flights it starts.
#[derive(Debug)]
pub struct Scroller {
    min: f32,
    max: f32,
    pos: f32,
    mode: Mode,
    /// The last touch samples (time, coordinate along the axis), newest last.
    samples: [(u64, f32); SAMPLES],
    count: usize,
}

impl Scroller {
    /// Content that can be scrolled from `min` to `max` (a `max` below `min` means nothing to scroll), at `min`.
    pub fn new(min: f32, max: f32) -> Self {
        Self {
            min,
            max: max.max(min),
            pos: min,
            mode: Mode::Rest,
            samples: [(0, 0.0); SAMPLES],
            count: 0,
        }
    }

    /// Where the content is now: the finger's position while dragging, the plan's estimate while flying.
    pub fn position_at(&self, now_ns: u64) -> f32 {
        match self.mode {
            Mode::Rest | Mode::Drag { .. } => self.pos,
            Mode::Flight(plan) => plan.position_at(now_ns).clamp(self.min, self.max),
        }
    }

    /// The plan being played, if any.
    pub fn flight(&self) -> Option<Plan> {
        match self.mode {
            Mode::Flight(plan) => Some(plan),
            _ => None,
        }
    }

    pub fn is_dragging(&self) -> bool {
        matches!(self.mode, Mode::Drag { .. })
    }

    /// True while a finger holds the content or a plan is still playing.
    pub fn is_moving(&self, now_ns: u64) -> bool {
        match self.mode {
            Mode::Rest => false,
            Mode::Drag { .. } => true,
            Mode::Flight(plan) => !plan.is_done(now_ns),
        }
    }

    /// A finger went down at `touch` (coordinate along the axis). Catching a fling in flight stops it where the
    /// app estimates it is and returns that position: the caller animates its layers there over a few
    /// milliseconds (`Animate`) so the catch does not jump.
    pub fn begin(&mut self, now_ns: u64, touch: f32) -> Option<f32> {
        let caught = self
            .flight()
            .map(|plan| plan.position_at(now_ns).clamp(self.min, self.max));
        if let Some(at) = caught {
            self.pos = at;
        }
        self.mode = Mode::Drag {
            anchor_touch: touch,
            anchor_pos: self.pos,
        };
        self.count = 0;
        self.note(now_ns, touch);
        caught
    }

    /// A finger that went down at `down` (time, coordinate) and has since moved past the slop has just been taken by the scroll:
    /// a list that held the contact as a possible tap until then. The content follows the finger from where it landed, so that the
    /// next `drag` (the sample that crossed the slop) puts it exactly as far from where it was as the finger has travelled: the
    /// slop is not lost, and the content never trails the finger by it. The speed estimate counts from the landing too, so that a
    /// quick flick that crosses the slop in one sample still flings.
    pub fn take(&mut self, down: (u64, f32)) {
        self.mode = Mode::Drag {
            anchor_touch: down.1,
            anchor_pos: self.pos,
        };
        self.count = 0;
        self.note(down.0, down.1);
    }

    /// The finger moved to `touch`. Returns the position to show the content at (rubber-banded past the ends).
    pub fn drag(&mut self, now_ns: u64, touch: f32) -> f32 {
        if let Mode::Drag {
            anchor_touch,
            anchor_pos,
        } = self.mode
        {
            self.pos = rubber(anchor_pos - (touch - anchor_touch), self.min, self.max);
            self.note(now_ns, touch);
        }
        self.pos
    }

    /// How far a release at `now_ns` would fling the content, in the sign of the position (positive towards larger positions), when
    /// nothing cuts the fling but the ends of the content: what a caller needs ready ahead of the finger, so that the rows are made
    /// before the fling reaches them and not after. Zero when the finger is not dragging or would not fling.
    pub fn reach(&self, now_ns: u64) -> f32 {
        if !self.is_dragging() || self.pos < self.min || self.pos > self.max {
            return 0.0;
        }
        plan_fling(
            self.pos,
            self.speed(now_ns),
            (self.min, self.max),
            Room::symmetric(f32::INFINITY),
        )
        .map_or(0.0, |(to, _)| to - self.pos)
    }

    /// The finger lifted at `now_ns`. Returns what the compositor should play, starting at `start_ns` on the frame clock
    /// (see the module docs): the fling the release speed asks for, cut to the content's ends and to `room`; or the
    /// return from past an end. `None` when the content just stays.
    pub fn release(&mut self, now_ns: u64, start_ns: u64, room: Room) -> Option<Plan> {
        if !self.is_dragging() {
            return None;
        }
        let speed = self.speed(now_ns);
        self.mode = Mode::Rest;
        if self.pos < self.min || self.pos > self.max {
            let to = self.pos.clamp(self.min, self.max);
            return Some(self.fly(start_ns, to, motion::RELEASE_MS, motion::RELEASE));
        }
        let (to, ms) = plan_fling(self.pos, speed, (self.min, self.max), room)?;
        Some(self.fly(start_ns, to, ms, motion::SETTLE))
    }

    /// The contact was lost (not lifted): the content stays where it is, with no fling, except that content held past an end
    /// returns to it. `None` when it just stays.
    pub fn stop(&mut self, start_ns: u64) -> Option<Plan> {
        if !self.is_dragging() {
            return None;
        }
        self.mode = Mode::Rest;
        if self.pos < self.min || self.pos > self.max {
            let to = self.pos.clamp(self.min, self.max);
            return Some(self.fly(start_ns, to, motion::RELEASE_MS, motion::RELEASE));
        }
        None
    }

    /// Lands a plan that has finished: the content rests where it went. Call it from the tick.
    pub fn finish(&mut self, now_ns: u64) {
        if let Mode::Flight(plan) = self.mode {
            if plan.is_done(now_ns) {
                self.pos = plan.to.clamp(self.min, self.max);
                self.mode = Mode::Rest;
            }
        }
    }

    /// The content changed under the scroll (rows were added or removed): it now runs from `min` to `max` and is at `pos`.
    /// A flight is over (the caller puts its layers at `pos` with `Set`, which stops the compositor's animation); a drag
    /// goes on from the finger where it is, from the new position.
    pub fn reframe(&mut self, min: f32, max: f32, pos: f32) {
        self.min = min;
        self.max = max.max(min);
        self.pos = pos.clamp(self.min, self.max);
        match self.mode {
            Mode::Rest => {}
            Mode::Flight(_) => self.mode = Mode::Rest,
            Mode::Drag { .. } => {
                let touch = self.samples[..self.count].last().map_or(0.0, |s| s.1);
                self.mode = Mode::Drag {
                    anchor_touch: touch,
                    anchor_pos: self.pos,
                };
            }
        }
    }

    /// The end of the scroll range.
    pub fn max(&self) -> f32 {
        self.max
    }

    fn fly(&mut self, start_ns: u64, to: f32, ms: u32, ease: Ease) -> Plan {
        let plan = Plan::along(self.pos, to, ms, ease, start_ns);
        self.mode = Mode::Flight(plan);
        plan
    }

    fn note(&mut self, now_ns: u64, touch: f32) {
        if self.count == SAMPLES {
            self.samples.copy_within(1.., 0);
            self.count -= 1;
        }
        self.samples[self.count] = (now_ns, touch);
        self.count += 1;
    }

    /// The content's speed along the axis at release, px per ms: positive scrolls towards a larger position
    /// (the finger was moving to a smaller coordinate). It is the slope of the samples of the last [`VELOCITY_WINDOW_NS`] before the
    /// newest ([`slope_per_s`], the one every release uses): the last one given, which may be dated before the one ahead of it
    /// (the resampler hands on a position a little past the finger's newest report, and a finger that lifts after it has its last
    /// position handed on at a moment that is no later).
    fn speed(&self, now_ns: u64) -> f32 {
        let samples = &self.samples[..self.count];
        let Some(&(last_ns, _)) = samples.last() else {
            return 0.0;
        };
        if now_ns.saturating_sub(last_ns) > motion::FLING_STILL_NS {
            return 0.0;
        }
        -slope_per_s(samples.iter().copied(), VELOCITY_WINDOW_NS) / 1000.0
    }
}

/// How the content follows a finger past an end: the one rubber band ([`motion::rubber`]), capped at [`motion::OVERSCROLL_PX`].
fn rubber(raw: f32, min: f32, max: f32) -> f32 {
    if raw < min {
        min - motion::rubber(min - raw, motion::OVERSCROLL_PX)
    } else if raw > max {
        max + motion::rubber(raw - max, motion::OVERSCROLL_PX)
    } else {
        raw
    }
}

/// How long a fling released at `speed` (px per ms) glides when nothing cuts it short: the time the shared inertia of design 8.4
/// takes to bring that speed down to rest (an exponential decay with time constant `SCROLL_TAU_MS`), kept to the tokens' bounds.
/// A faster finger glides longer, and only logarithmically so.
fn glide_ms(speed: f32) -> f32 {
    let rest = motion::SCROLL_REST_PX_PER_S / 1000.0;
    (motion::SCROLL_TAU_MS * (speed.abs().max(rest) / rest).ln())
        .clamp(motion::FLING_MIN_MS as f32, motion::FLING_MAX_MS as f32)
}

/// The fling a release at `speed` (px per ms, signed) from `from` asks for, as (where it lands, how long), cut to
/// the content's `bounds` and to `room`. A glide along [`motion::SETTLE`] starts at [`start_slope`] times its average speed, so a
/// fling of distance `d` over `t` ms starts at `d * start_slope / t`: the distance is the one that starts at the speed the finger let
/// go at, over [`glide_ms`]; when a cut shortens the distance the duration shortens with it, keeping that speed.
fn plan_fling(from: f32, speed: f32, bounds: (f32, f32), room: Room) -> Option<(f32, u32)> {
    if speed.abs() < motion::FLING_MIN_SPEED {
        return None;
    }
    let start_slope = start_slope(motion::SETTLE);
    let free_ms = glide_ms(speed);
    let free = speed * free_ms / start_slope;
    let available = if speed > 0.0 {
        (bounds.1 - from).min(room.forward)
    } else {
        (from - bounds.0).min(room.back)
    }
    .max(0.0);
    let distance = free.abs().min(available);
    if distance < 2.0 {
        return None;
    }
    let ms = if distance < free.abs() {
        (start_slope * distance / speed.abs()).clamp(motion::FLING_CUT_MIN_MS as f32, free_ms)
    } else {
        free_ms
    };
    Some((from + distance.copysign(speed), ms.round() as u32))
}

/// The items of a list of `count` items, `pitch` px apart from `origin`, that a window `from..to` (px along the same
/// axis) reaches: their first and one past their last index.
pub fn items_in(origin: f32, pitch: f32, count: usize, from: f32, to: f32) -> Range<usize> {
    if count == 0 || to <= from {
        return 0..0;
    }
    let first = ((from - origin) / pitch).floor().max(0.0) as usize;
    let end = ((to - origin) / pitch).ceil().max(0.0) as usize;
    first.min(count)..end.min(count)
}

/// How far a fling from `position` may carry a view `view` px long before it would run past the rows that are
/// already rastered (`resident`, indices of items `pitch` px apart from `origin`).
pub fn room_for(
    origin: f32,
    pitch: f32,
    resident: &Range<usize>,
    position: f32,
    view: f32,
) -> Room {
    if resident.is_empty() {
        return Room::symmetric(0.0);
    }
    let start = origin + resident.start as f32 * pitch;
    let end = origin + resident.end as f32 * pitch;
    Room {
        back: (position - start).max(0.0),
        forward: (end - (position + view)).max(0.0),
    }
}

/// The items of a virtualised list that are rastered right now, by index. Only the window a screen asks for is
/// ever held, and new items are made a few at a time so a burst of them never lands on one frame.
pub struct Resident<V> {
    items: BTreeMap<usize, V>,
}

impl<V> Default for Resident<V> {
    fn default() -> Self {
        Self {
            items: BTreeMap::new(),
        }
    }
}

impl<V> Resident<V> {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, index: usize) -> bool {
        self.items.contains_key(&index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut V> {
        self.items.get_mut(&index)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (usize, &mut V)> {
        self.items.iter_mut().map(|(&i, v)| (i, v))
    }

    pub fn insert(&mut self, index: usize, item: V) {
        self.items.insert(index, item);
    }

    /// Drops every item outside `keep` (their layers go with them) and returns how many went.
    pub fn retain_in(&mut self, keep: &Range<usize>) -> usize {
        let before = self.items.len();
        self.items.retain(|i, _| keep.contains(i));
        before - self.items.len()
    }

    /// Up to `budget` indices of `wanted` that are not rastered yet, nearest to `focus` first.
    pub fn missing(&self, wanted: &Range<usize>, focus: usize, budget: usize) -> Vec<usize> {
        let mut missing: Vec<usize> = wanted
            .clone()
            .filter(|i| !self.items.contains_key(i))
            .collect();
        missing.sort_by_key(|&i| i.abs_diff(focus));
        missing.truncate(budget);
        missing
    }

    /// The longest run of consecutive indices held that includes `near` (the rows a fling can cross), or empty.
    pub fn run_around(&self, near: usize) -> Range<usize> {
        if !self.items.contains_key(&near) {
            return near..near;
        }
        let mut start = near;
        while start > 0 && self.items.contains_key(&(start - 1)) {
            start -= 1;
        }
        let mut end = near + 1;
        while self.items.contains_key(&end) {
            end += 1;
        }
        start..end
    }
}

/// Which row each of a fixed number of slots shows: the pure part of a recycled list. A slot is one prepared tile (a set of
/// layers) that is drawn again for another row, never created again; the recycler says which slot shows which row, which rows of
/// the window are still missing, and which slots go free when rows leave it.
#[derive(Debug)]
pub struct Recycler {
    slots: Vec<Option<usize>>,
}

impl Recycler {
    pub fn new(slots: usize) -> Self {
        Self {
            slots: vec![None; slots],
        }
    }

    /// How many slots there are.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// The slot showing `row`.
    pub fn slot_of(&self, row: usize) -> Option<usize> {
        self.slots.iter().position(|&r| r == Some(row))
    }

    /// The row `slot` shows, if it shows one.
    pub fn row_of(&self, slot: usize) -> Option<usize> {
        self.slots.get(slot).copied().flatten()
    }

    /// Every (slot, row) that shows something.
    pub fn resident(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(slot, row)| row.map(|r| (slot, r)))
    }

    /// How many slots show nothing.
    pub fn free(&self) -> usize {
        self.slots.iter().filter(|r| r.is_none()).count()
    }

    /// Frees every slot.
    pub fn clear(&mut self) {
        self.slots.fill(None);
    }

    /// Frees the slots of rows outside `keep`, and returns how many.
    pub fn retain_in(&mut self, keep: &Range<usize>) -> usize {
        let mut freed = 0;
        for slot in &mut self.slots {
            if slot.is_some_and(|row| !keep.contains(&row)) {
                *slot = None;
                freed += 1;
            }
        }
        freed
    }

    /// The rows changed places (rows were inserted, removed or reordered): each shown row is now the one `to` says, and a
    /// row `to` gives no place (it is gone) frees its slot. Returns how many slots were freed. Two slots never come to
    /// show the same row: if `to` maps them alike, the later slot is freed.
    pub fn remap(&mut self, mut to: impl FnMut(usize) -> Option<usize>) -> usize {
        let mut freed = 0;
        for i in 0..self.slots.len() {
            let Some(row) = self.slots[i] else { continue };
            let new = to(row);
            let taken = new.is_some_and(|n| self.slots[..i].contains(&Some(n)));
            self.slots[i] = if taken { None } else { new };
            if self.slots[i].is_none() {
                freed += 1;
            }
        }
        freed
    }

    /// Frees the slots of rows outside `want`, then gives free slots to the rows of `want` that are not shown, at most
    /// `budget` of them and the ones nearest `focus` first, and returns the (slot, row) pairs that now need drawing.
    /// Rows that stay in the window keep their slot and are not drawn again.
    pub fn assign(
        &mut self,
        want: &Range<usize>,
        focus: usize,
        budget: usize,
    ) -> Vec<(usize, usize)> {
        self.retain_in(want);
        let mut missing: Vec<usize> = want
            .clone()
            .filter(|&r| self.slot_of(r).is_none())
            .collect();
        missing.sort_by_key(|&r| r.abs_diff(focus));
        let mut drawn = Vec::new();
        for row in missing.into_iter().take(budget) {
            let Some(slot) = self.slots.iter().position(Option::is_none) else {
                break;
            };
            self.slots[slot] = Some(row);
            drawn.push((slot, row));
        }
        drawn
    }

    /// The longest run of consecutive rows around `focus` that are shown: what a fling may cross without reaching a row
    /// that is not drawn.
    pub fn run_around(&self, focus: usize) -> Range<usize> {
        if self.slot_of(focus).is_none() {
            return focus..focus;
        }
        let (mut start, mut end) = (focus, focus + 1);
        while start > 0 && self.slot_of(start - 1).is_some() {
            start -= 1;
        }
        while self.slot_of(end).is_some() {
            end += 1;
        }
        start..end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    /// A finger going from `y0` to `y1` in `steps` samples, 16 ms apart, then lifting `lift_after_ms` after the
    /// last one. Returns the time of the lift.
    fn swipe(s: &mut Scroller, y0: f32, y1: f32, steps: u32, lift_after_ms: u64) -> u64 {
        let mut t = 1_000 * MS;
        s.begin(t, y0);
        for i in 1..=steps {
            t += 16 * MS;
            s.drag(t, y0 + (y1 - y0) * i as f32 / steps as f32);
        }
        t + lift_after_ms * MS
    }

    /// When the compositor starts a plan released at `lift`, as a host that knows its frame clock would say it: the next
    /// refresh.
    const NEXT_FRAME_NS: u64 = 17 * MS;

    fn release(s: &mut Scroller, lift: u64, room: Room) -> Option<Plan> {
        s.release(lift, lift + NEXT_FRAME_NS, room)
    }

    /// The speed (px per ms) a plan starts at: its mean speed times the start slope of its curve.
    fn start_speed(plan: &Plan) -> f32 {
        start_slope(plan.ease) * (plan.to - plan.from) / plan.duration_ms as f32
    }

    #[test]
    fn dragging_moves_the_content_opposite_to_the_finger_one_to_one_inside_the_bounds() {
        let mut s = Scroller::new(0.0, 1000.0);
        s.begin(0, 300.0);
        assert_eq!(s.drag(16 * MS, 250.0), 50.0);
        assert_eq!(s.drag(32 * MS, 280.0), 20.0);
    }

    #[test]
    fn past_the_ends_the_content_resists_the_finger_and_never_leaves_by_more_than_the_limit() {
        let (min, max) = (0.0, 500.0);
        // Continuous at the end, then strictly less than the finger's own travel, bounded.
        assert_eq!(rubber(0.0, min, max), 0.0);
        let mut last = 0.0;
        for over in [1.0, 10.0, 50.0, 200.0, 5000.0] {
            let shown = min - rubber(-over, min, max);
            assert!(shown > last, "content stopped following at {over}");
            assert!(
                shown <= over,
                "content moved more than the finger at {over}"
            );
            assert!(shown <= motion::OVERSCROLL_PX);
            last = shown;
        }
        assert!(rubber(max + 40.0, min, max) - max < 40.0);
    }

    #[test]
    fn a_fast_flick_becomes_a_fling_that_starts_at_the_speed_the_finger_let_go() {
        let mut s = Scroller::new(0.0, 100_000.0);
        // 20 samples of 32 px per 16 ms: the finger moves up at 2 px/ms, so the content scrolls forward at 2.
        let lift = swipe(&mut s, 900.0, 900.0 - 32.0 * 20.0, 20, 4);
        let plan = release(&mut s, lift, Room::symmetric(10_000.0)).expect("a fling");
        assert!(plan.to > plan.from);
        let start_speed = start_speed(&plan);
        assert!(
            (start_speed - 2.0).abs() < 0.02,
            "starts at {start_speed} px/ms, the finger let go at 2"
        );
        assert!((motion::FLING_MIN_MS..=motion::FLING_MAX_MS).contains(&plan.duration_ms));
    }

    #[test]
    fn a_fling_is_cut_to_the_rows_that_exist_and_keeps_its_starting_speed() {
        let mut s = Scroller::new(0.0, 100_000.0);
        let lift = swipe(&mut s, 900.0, 900.0 - 32.0 * 20.0, 20, 4);
        let start = s.position_at(lift);
        let plan = release(
            &mut s,
            lift,
            Room {
                back: 0.0,
                forward: 150.0,
            },
        )
        .expect("a shorter fling");
        assert!((plan.to - plan.from - 150.0).abs() < 1e-3);
        let start_speed = start_speed(&plan);
        assert!(
            (start_speed - 2.0).abs() < 0.05,
            "a cut fling starts at {start_speed} px/ms, not slower"
        );
        assert_eq!(plan.from, start);
        assert!(plan.duration_ms < motion::FLING_MAX_MS);
    }

    #[test]
    fn a_fling_never_runs_past_the_ends_of_the_content() {
        let plan = plan_fling(250.0, 3.0, (0.0, 300.0), Room::symmetric(10_000.0)).unwrap();
        assert_eq!(plan.0, 300.0);
        let back = plan_fling(50.0, -3.0, (0.0, 300.0), Room::symmetric(10_000.0)).unwrap();
        assert_eq!(back.0, 0.0);
        // Already at the end and pushing on: nothing to play.
        assert_eq!(
            plan_fling(300.0, 3.0, (0.0, 300.0), Room::symmetric(10_000.0)),
            None
        );
    }

    #[test]
    fn a_swipe_that_has_just_got_up_to_speed_flings_at_that_speed_not_at_its_average_from_the_landing()
     {
        // The shape of the swipes made on the Show: the finger rests 20 ms, speeds up to 3 px/ms over 30 ms and goes on at that, and is
        // lifted 92 ms after it landed. Its speed averaged from where it landed is 1.9 px/ms (62 % of what it lifts at).
        let (y0, up_to_speed) = (9_000.0f32, 3.0f32);
        let travelled = |t_ms: f32| {
            let ramp = (t_ms - 20.0).clamp(0.0, 30.0);
            let plateau = (t_ms - 50.0).max(0.0);
            0.5 * (up_to_speed / 30.0) * ramp * ramp + up_to_speed * plateau
        };
        let mut s = Scroller::new(0.0, 100_000.0);
        s.take((0, y0));
        let mut last = 0;
        // Handed on every 16.77 ms (the resampler's deliveries), the last a moment before the lift.
        for k in 0..6 {
            let t_ms = 8.0 + 16.77 * k as f32;
            last = (t_ms * 1e6) as u64;
            s.drag(last, y0 - travelled(t_ms));
        }
        let plan = s
            .release(last, last, Room::symmetric(100_000.0))
            .expect("a fling");
        let start = start_speed(&plan);
        assert!(
            (start / up_to_speed - 1.0).abs() < 0.12,
            "starts at {start} px/ms; the finger lifted at {up_to_speed}"
        );
    }

    #[test]
    fn a_finger_that_stopped_before_lifting_does_not_fling() {
        let mut s = Scroller::new(0.0, 100_000.0);
        let lift = swipe(&mut s, 900.0, 300.0, 20, 300);
        assert_eq!(release(&mut s, lift, Room::symmetric(10_000.0)), None);
        assert!(!s.is_moving(lift + 1));
    }

    #[test]
    fn only_the_last_fifty_milliseconds_of_the_drag_set_the_speed() {
        let mut s = Scroller::new(0.0, 100_000.0);
        let mut t = 1_000 * MS;
        s.begin(t, 900.0);
        // A quick start (4 px/ms for 320 ms) and then a slow finish (0.2 px/ms for 160 ms): a gentle release.
        let mut y = 900.0;
        for _ in 0..20 {
            t += 16 * MS;
            y -= 64.0;
            s.drag(t, y);
        }
        for _ in 0..10 {
            t += 16 * MS;
            y -= 3.2;
            s.drag(t, y);
        }
        let plan = release(&mut s, t + 4 * MS, Room::symmetric(10_000.0));
        let speed = plan.map_or(0.0, |p| start_speed(&p));
        assert!(speed < 0.3, "released at {speed} px/ms after a slow finish");
    }

    #[test]
    fn letting_go_past_an_end_brings_the_content_back_in_the_scroll_time() {
        let mut s = Scroller::new(0.0, 500.0);
        s.begin(0, 100.0);
        let shown = s.drag(16 * MS, 260.0);
        assert!(shown < 0.0 && shown > -160.0);
        let plan = release(&mut s, 20 * MS, Room::symmetric(1000.0)).expect("a return");
        assert_eq!(
            (plan.to, plan.duration_ms, plan.ease),
            (0.0, motion::RELEASE_MS, motion::RELEASE)
        );
        assert_eq!(plan.from, shown);
    }

    #[test]
    fn a_release_at_the_fling_speed_flings_and_one_just_under_it_does_not() {
        let at =
            |speed: f32| plan_fling(5_000.0, speed, (0.0, 100_000.0), Room::symmetric(100_000.0));
        assert_eq!(at(motion::FLING_MIN_SPEED * 0.99), None);
        assert!(at(motion::FLING_MIN_SPEED).is_some());
        assert!(at(-motion::FLING_MIN_SPEED).is_some(), "either way");
    }

    #[test]
    fn a_finger_that_rested_the_still_time_before_lifting_still_flings_and_a_millisecond_more_stops_it()
     {
        let after = |rest_ms: u64| {
            let mut s = Scroller::new(0.0, 100_000.0);
            let lift = swipe(&mut s, 900.0, 260.0, 20, rest_ms);
            release(&mut s, lift, Room::symmetric(10_000.0))
        };
        let still_ms = motion::FLING_STILL_NS / MS;
        assert!(after(still_ms).is_some(), "{still_ms} ms is still going");
        assert_eq!(after(still_ms + 1), None, "a millisecond more is stopped");
    }

    #[test]
    fn a_fling_cut_to_the_content_is_cut_in_time_to_no_less_than_the_least_and_not_planned_under_two_pixels()
     {
        let room = Room::symmetric(10_000.0);
        // 10 px left at 3 px/ms is a few milliseconds of glide, and the least it glides is the cut minimum.
        assert_eq!(
            plan_fling(290.0, 3.0, (0.0, 300.0), room),
            Some((300.0, motion::FLING_CUT_MIN_MS))
        );
        assert_eq!(plan_fling(298.5, 3.0, (0.0, 300.0), room), None);
        assert!(plan_fling(298.0, 3.0, (0.0, 300.0), room).is_some());
    }

    #[test]
    fn at_an_end_the_content_is_where_the_finger_puts_it_and_past_it_follows_at_the_rubber_bands_share()
     {
        let mut s = Scroller::new(0.0, 500.0);
        s.begin(0, 100.0);
        assert_eq!(s.drag(16 * MS, -400.0), 500.0, "exactly at the end");
        let past = s.drag(32 * MS, -401.0) - 500.0;
        assert!(
            (past - motion::RUBBER_FOLLOW).abs() < 0.01,
            "one unit past the end shows {past}"
        );
    }

    #[test]
    fn a_plan_runs_from_where_it_started_to_where_it_lands_and_never_backwards() {
        let plan = Plan::new(100.0, 700.0, 800, 5_000 * MS + NEXT_FRAME_NS);
        let start = plan.start_ns();
        assert_eq!(plan.position_at(start - 5 * MS), 100.0);
        assert_eq!(plan.position_at(start + 800 * MS), 700.0);
        let mut last = 100.0;
        for ms in (0..=800).step_by(8) {
            let p = plan.position_at(start + ms * MS);
            assert!(p >= last && p <= 700.0);
            last = p;
        }
        assert!(
            plan.position_at(start + 400 * MS) > 400.0,
            "out-cubic is ahead of linear"
        );
        assert!(!plan.is_done(start + 799 * MS));
        assert!(plan.is_done(start + 800 * MS));
        assert_eq!(plan.remaining_ms(start + 300 * MS), 500);
    }

    #[test]
    fn the_rest_of_a_plan_is_nearly_the_same_curve_again_so_a_late_layer_can_join_it() {
        // From time t1 the rest of the settle curve is the same curve again over the remaining time and distance to within 0.25 % of
        // the plan's distance (the CSS curve is a cubic with its time bent a little, not the exact cubic it resembles, whose rest is
        // itself): a layer created at t1 and animated to the landing with Out for the remaining time is on the plan's curve at every
        // later time, to a couple of pixels of a 900 px fling.
        let plan = Plan::new(0.0, 900.0, 1000, NEXT_FRAME_NS);
        let start = plan.start_ns();
        let t1 = start + 300 * MS;
        let (p1, remaining) = (plan.position_at(t1), plan.remaining_ms(t1) as f32);
        for ms in [100u64, 250, 400, 600, 699] {
            let late = p1 + (900.0 - p1) * ease(Ease::Out, ms as f32 / remaining);
            let exact = plan.position_at(t1 + ms * MS);
            assert!(
                (late - exact).abs() < 900.0 * 0.0025,
                "{late} vs {exact} at +{ms}"
            );
        }
    }

    #[test]
    fn catching_a_fling_stops_the_content_where_the_plan_has_taken_it() {
        let mut s = Scroller::new(0.0, 100_000.0);
        let lift = swipe(&mut s, 900.0, 900.0 - 32.0 * 20.0, 20, 4);
        let plan = release(&mut s, lift, Room::symmetric(10_000.0)).unwrap();
        let later = lift + 400 * MS;
        let expected = plan.position_at(later);
        assert!(s.is_moving(later));
        let caught = s.begin(later, 300.0).expect("a fling was in flight");
        assert!((caught - expected).abs() < 1e-3);
        assert!(s.is_dragging());
        // The next drag continues from the caught position.
        assert_eq!(s.drag(later + 16 * MS, 290.0), caught + 10.0);
    }

    #[test]
    fn a_finished_fling_leaves_the_content_at_rest_where_it_landed() {
        let mut s = Scroller::new(0.0, 100_000.0);
        let lift = swipe(&mut s, 900.0, 900.0 - 32.0 * 20.0, 20, 4);
        let plan = release(&mut s, lift, Room::symmetric(10_000.0)).unwrap();
        s.finish(lift + 10 * MS);
        assert!(s.flight().is_some(), "still flying");
        let done = lift + (u64::from(plan.duration_ms) + 40) * MS;
        s.finish(done);
        assert!(s.flight().is_none() && !s.is_moving(done));
        assert_eq!(s.position_at(done), plan.to);
    }

    #[test]
    fn the_window_of_items_a_range_reaches_includes_partly_visible_ones_and_stays_inside_the_list()
    {
        // Items 100 px apart from 40: item i covers 40+100i.
        assert_eq!(items_in(40.0, 100.0, 10, 0.0, 480.0), 0..5);
        assert_eq!(items_in(40.0, 100.0, 10, 150.0, 350.0), 1..4);
        assert_eq!(items_in(40.0, 100.0, 10, -500.0, 60.0), 0..1);
        assert_eq!(items_in(40.0, 100.0, 10, 900.0, 5000.0), 8..10);
        assert_eq!(items_in(40.0, 100.0, 10, 2000.0, 3000.0), 10..10);
        assert!(items_in(40.0, 100.0, 0, 0.0, 480.0).is_empty());
    }

    #[test]
    fn the_room_a_fling_has_is_the_rastered_rows_beyond_the_view_on_each_side() {
        // Rows 3..9 are rastered, 100 px apart from 0; the view is 300 px tall at position 450.
        let room = room_for(0.0, 100.0, &(3..9), 450.0, 300.0);
        assert_eq!(room.back, 150.0);
        assert_eq!(room.forward, 900.0 - 750.0);
        // A view that already reaches past the rastered rows has no room that way.
        assert_eq!(room_for(0.0, 100.0, &(3..9), 300.0, 700.0).forward, 0.0);
        assert_eq!(
            room_for(0.0, 100.0, &(0..0), 0.0, 300.0),
            Room::symmetric(0.0)
        );
    }

    #[test]
    fn only_the_window_is_kept_and_new_items_come_in_small_batches_nearest_first() {
        let mut r: Resident<u32> = Resident::default();
        for i in 0..10 {
            r.insert(i, i as u32);
        }
        assert_eq!(r.retain_in(&(4..8)), 6);
        assert_eq!(r.len(), 4);
        assert!(!r.contains(3) && r.contains(4) && r.contains(7) && !r.contains(8));
        // Wanting 2..12 with rows 4..8 held: the missing ones are asked for nearest to the focus first, at most
        // `budget` at a time.
        let missing = r.missing(&(2..12), 9, 3);
        assert_eq!(missing, vec![9, 8, 10]);
        for i in &missing {
            r.insert(*i, 0);
        }
        assert!(r.len() <= 12 - 2, "never more than the window");
        assert_eq!(r.missing(&(4..8), 4, 10), Vec::<usize>::new());
    }

    #[test]
    fn a_fling_may_cross_only_the_unbroken_run_of_rows_around_the_view() {
        let mut r: Resident<()> = Resident::default();
        for i in [2, 3, 4, 5, 6, 9, 10] {
            r.insert(i, ());
        }
        assert_eq!(r.run_around(4), 2..7);
        assert_eq!(r.run_around(9), 9..11);
        assert_eq!(r.run_around(0), 0..0);
    }

    #[test]
    fn a_plan_starts_exactly_when_the_caller_says_and_not_a_moment_sooner() {
        let mut s = Scroller::new(0.0, 100_000.0);
        let lift = swipe(&mut s, 900.0, 900.0 - 32.0 * 20.0, 20, 4);
        // The frame clock says the compositor starts this one 40 ms after the lift, and another released the same way 5 ms.
        let late = s
            .release(lift, lift + 40 * MS, Room::symmetric(10_000.0))
            .unwrap();
        assert_eq!(late.start_ns(), lift + 40 * MS);
        assert_eq!(
            late.position_at(lift + 39 * MS),
            late.from,
            "nothing has moved before the start"
        );
        assert!(late.position_at(lift + 60 * MS) > late.from);

        let mut t = Scroller::new(0.0, 100_000.0);
        let lift = swipe(&mut t, 900.0, 900.0 - 32.0 * 20.0, 20, 4);
        let early = t
            .release(lift, lift + 5 * MS, Room::symmetric(10_000.0))
            .unwrap();
        assert_eq!(early.start_ns(), lift + 5 * MS);
        assert!(
            early.position_at(lift + 40 * MS) > late.position_at(lift + 40 * MS),
            "the same release, started earlier, is further along at the same moment"
        );
    }

    #[test]
    fn a_finger_taken_after_the_slop_moves_the_content_from_where_it_landed_and_still_flings_if_it_was_quick()
     {
        let mut s = Scroller::new(0.0, 100_000.0);
        // Down at 900 at t=0 and, 16 ms later, already 60 px up: one sample crossed the slop.
        s.take((0, 900.0));
        assert_eq!(
            s.drag(16 * MS, 840.0),
            60.0,
            "the sample that crossed the slop puts the content as far as the finger has travelled: no lag, no later jump"
        );
        assert_eq!(
            s.drag(32 * MS, 800.0),
            100.0,
            "then the content follows the finger one to one"
        );
        let plan =
            release(&mut s, 34 * MS, Room::symmetric(10_000.0)).expect("a quick flick flings");
        let start_speed = start_speed(&plan);
        assert!((start_speed - 100.0 / 32.0).abs() < 0.1, "{start_speed}");
    }

    #[test]
    fn a_finger_far_past_an_end_never_shows_the_content_more_than_32_px_past_it() {
        let mut s = Scroller::new(0.0, 500.0);
        s.begin(0, 100.0);
        let mut last = 0.0;
        for (i, travel) in [5.0, 40.0, 150.0, 600.0, 20_000.0].into_iter().enumerate() {
            let past = -s.drag((i as u64 + 1) * 16 * MS, 100.0 + travel);
            assert!(past > last, "the content stopped following at {travel}");
            assert!(
                past <= 32.0,
                "{past} px past the end for a finger {travel} px on"
            );
            last = past;
        }
        assert!(last > 31.0, "and the curve saturates towards 32: {last}");
    }

    #[test]
    fn a_fling_glides_longer_the_faster_the_finger_let_go_within_the_token_bounds() {
        let mut lasts = Vec::new();
        for speed in [0.2f32, 0.5, 1.0, 2.0, 3.0] {
            // Samples 16 ms apart, `speed` px/ms; lifted a moment after the last.
            let mut s = Scroller::new(0.0, 100_000.0);
            let step = speed * 16.0;
            let lift = swipe(&mut s, 9_000.0, 9_000.0 - step * 10.0, 10, 2);
            let plan = release(&mut s, lift, Room::symmetric(100_000.0)).expect("a fling");
            assert!(
                (motion::FLING_MIN_MS..=motion::FLING_MAX_MS).contains(&plan.duration_ms),
                "{speed} px/ms glided {} ms",
                plan.duration_ms
            );
            lasts.push(plan.duration_ms);
        }
        assert!(
            lasts
                .windows(2)
                .all(|w| w[0] < w[1] || w[1] == motion::FLING_MAX_MS),
            "a faster finger never glides shorter, until the cap: {lasts:?}"
        );
        assert!(
            lasts[0] < 800 && *lasts.last().unwrap() == motion::FLING_MAX_MS,
            "{lasts:?}"
        );
        // The design's decay: from 1 px/ms down to 2.5 px/s takes its time constant * ln(400).
        assert_eq!(
            glide_ms(1.0).round(),
            (motion::SCROLL_TAU_MS * 400f32.ln()).round()
        );
    }

    #[test]
    fn what_a_release_would_fling_is_known_while_the_finger_is_still_down() {
        let mut s = Scroller::new(0.0, 100_000.0);
        let mut t = 1_000 * MS;
        s.begin(t, 900.0);
        assert_eq!(
            s.reach(t),
            0.0,
            "a finger that has not moved flings nowhere"
        );
        for i in 1..=10 {
            t += 16 * MS;
            s.drag(t, 900.0 - 32.0 * i as f32);
        }
        let reach = s.reach(t);
        let plan = release(&mut s, t, Room::symmetric(100_000.0)).expect("a fling");
        assert!(
            (reach - (plan.to - plan.from)).abs() < 0.5,
            "{reach} against the plan's {}",
            plan.to - plan.from
        );
        assert_eq!(s.reach(t), 0.0, "and nothing once it has let go");
    }

    #[test]
    fn reframing_ends_a_flight_and_keeps_a_drag_going_from_the_new_position() {
        let mut s = Scroller::new(0.0, 100_000.0);
        let lift = swipe(&mut s, 900.0, 900.0 - 32.0 * 20.0, 20, 4);
        release(&mut s, lift, Room::symmetric(10_000.0)).unwrap();
        s.reframe(0.0, 500.0, 120.0);
        assert!(s.flight().is_none() && !s.is_moving(lift + 10 * MS));
        assert_eq!((s.position_at(lift), s.max()), (120.0, 500.0));

        s.begin(lift, 300.0);
        s.drag(lift + 16 * MS, 280.0);
        s.reframe(0.0, 500.0, 200.0);
        assert!(s.is_dragging());
        assert_eq!(
            s.drag(lift + 32 * MS, 270.0),
            210.0,
            "the finger carries on from the new position"
        );
        s.reframe(0.0, 100.0, 400.0);
        assert_eq!(
            s.position_at(0),
            100.0,
            "a position past the new end is brought in"
        );
    }

    /// Scrolling a 60-row list a row at a time through a window of 12 keeps at most 12 rows drawn, draws each row exactly once as
    /// it enters, and reuses the slots of the rows that left.
    #[test]
    fn scrolling_draws_each_row_once_and_recycles_the_slots_of_rows_that_left() {
        let mut r = Recycler::new(12);
        let mut drawn = Vec::new();
        for first in 0..=48 {
            let want = first..first + 12;
            drawn.extend(r.assign(&want, first, usize::MAX));
            assert!(r.resident().count() <= 12);
            for row in want.clone() {
                assert!(
                    r.slot_of(row).is_some(),
                    "row {row} of the window has no slot"
                );
            }
            let mut slots: Vec<_> = r.resident().map(|(s, _)| s).collect();
            slots.sort_unstable();
            slots.dedup();
            assert_eq!(slots.len(), r.resident().count(), "two rows share a slot");
        }
        // 12 rows at the start and one more for each step: every row of the list, once.
        let mut rows: Vec<_> = drawn.iter().map(|&(_, row)| row).collect();
        rows.sort_unstable();
        assert_eq!(rows, (0..60).collect::<Vec<_>>());
        assert!(
            drawn.iter().all(|&(slot, _)| slot < 12),
            "never more than the 12 slots"
        );
        assert!(
            r.assign(&(48..60), 48, usize::MAX).is_empty(),
            "a window that has not moved draws nothing"
        );
    }

    #[test]
    fn rows_that_leave_the_window_are_forgotten_and_the_nearest_missing_rows_come_first_within_a_budget()
     {
        let mut r = Recycler::new(8);
        r.assign(&(0..8), 0, usize::MAX);
        // The window jumps to 20..28: everything leaves, and only 3 rows may be drawn now, nearest 24 first.
        let rows: Vec<_> = r
            .assign(&(20..28), 24, 3)
            .iter()
            .map(|&(_, row)| row)
            .collect();
        assert_eq!(rows, vec![24, 23, 25]);
        assert_eq!(r.slot_of(0), None, "row 0 left");
        assert_eq!(r.free(), 5);
        assert_eq!(
            r.assign(&(20..28), 24, 3).len(),
            3,
            "the rest arrive on later calls"
        );
    }

    #[test]
    fn a_fling_may_cross_only_the_consecutive_rows_that_are_drawn() {
        let mut r = Recycler::new(10);
        // Two of the four rows of the window are drawn so far, the ones nearest the focus.
        r.assign(&(10..14), 12, 2);
        assert_eq!(r.run_around(12), 11..13);
        r.assign(&(10..14), 12, usize::MAX);
        assert_eq!(r.run_around(12), 10..14);
        assert_eq!(r.run_around(20), 20..20, "an undrawn row has no run");
    }

    #[test]
    fn rows_that_changed_places_keep_their_slots_and_rows_that_are_gone_free_them() {
        let mut r = Recycler::new(6);
        r.assign(&(10..14), 10, usize::MAX);
        let before: Vec<_> = (10..14).map(|row| r.slot_of(row).unwrap()).collect();
        // Two rows were inserted above: every row moves down by two, and the row that was 12 is gone.
        let freed = r.remap(|row| (row != 12).then_some(row + 2));
        assert_eq!(freed, 1);
        assert_eq!(r.slot_of(12), Some(before[0]));
        assert_eq!(r.slot_of(13), Some(before[1]));
        assert_eq!(r.slot_of(15), Some(before[3]));
        assert_eq!(
            r.row_of(before[2]),
            None,
            "the removed row's slot is free again"
        );
        assert_eq!(r.free(), 3);
    }

    #[test]
    fn two_slots_never_come_to_show_the_same_row() {
        let mut r = Recycler::new(3);
        r.assign(&(0..3), 0, usize::MAX);
        r.remap(|_| Some(7));
        assert_eq!(r.resident().count(), 1);
        assert_eq!(r.resident().next().map(|(_, row)| row), Some(7));
    }
}
