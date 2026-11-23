//! Where a job's output waits: a bounded delivery queue for an owner that pulls, and a ring of whole groups of pictures for clips. Pure data
//! structures, driven by the engine's reader thread under a lock.

use crate::types::{Clip, ClipError, Out};
use std::collections::VecDeque;
use std::sync::Arc;

/// Units for the owner to pull. It never grows past its limits: when it is full a unit is dropped and so is everything after it up to the next key
/// unit, because a decoder can use nothing between a hole and the next key unit.
pub(crate) struct Queue {
    items: VecDeque<Out>,
    bytes: usize,
    max_units: usize,
    max_bytes: usize,
    skipping: bool,
    /// The next unit accepted follows a gap in the stream.
    mark: bool,
    dropped: u32,
}

impl Queue {
    pub(crate) fn new(max_units: usize, max_bytes: usize) -> Self {
        Self { items: VecDeque::new(), bytes: 0, max_units, max_bytes, skipping: false, mark: false, dropped: 0 }
    }

    pub(crate) fn push(&mut self, mut out: Out) {
        let room = self.items.len() < self.max_units && self.bytes.saturating_add(out.data.len()) <= self.max_bytes;
        if self.skipping {
            if out.key && room {
                self.skipping = false;
                out.discontinuity = true;
            } else {
                self.dropped = self.dropped.saturating_add(1);
                return;
            }
        } else if !room {
            self.skipping = true;
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        if std::mem::take(&mut self.mark) {
            out.discontinuity = true;
        }
        self.bytes += out.data.len();
        self.items.push_back(out);
    }

    pub(crate) fn pop(&mut self) -> Option<Out> {
        let out = self.items.pop_front()?;
        self.bytes = self.bytes.saturating_sub(out.data.len());
        Some(out)
    }

    /// The job paused or restarted: the next unit is not a continuation.
    pub(crate) fn mark_gap(&mut self) {
        self.mark = true;
    }

    /// Units dropped since the last call.
    pub(crate) fn take_dropped(&mut self) -> u32 {
        std::mem::take(&mut self.dropped)
    }
}

/// The newest units, in whole groups of pictures: it starts on a key unit and the oldest group goes first, so a clip cut from it always decodes.
pub(crate) struct Ring {
    units: VecDeque<Arc<Out>>,
    /// The sequence number of `units[0]`. Sequence numbers count every unit the ring ever took, so the ones in `keys` stay valid as the head is evicted.
    first: u64,
    /// Sequence numbers of the key units held, oldest first.
    keys: VecDeque<u64>,
    /// The newest held unit that follows a gap.
    gap: Option<u64>,
    bytes: usize,
    max_bytes: usize,
    /// `max_seconds` in pts ticks; 0 keeps only to the byte limit.
    max_ticks: u64,
    hz: u32,
    mark: bool,
}

impl Ring {
    pub(crate) fn new(max_bytes: usize, max_seconds: u32, pts_hz: u32) -> Self {
        Self {
            units: VecDeque::new(),
            first: 0,
            keys: VecDeque::new(),
            gap: None,
            bytes: 0,
            max_bytes,
            max_ticks: u64::from(max_seconds) * u64::from(pts_hz),
            hz: pts_hz,
            mark: false,
        }
    }

    pub(crate) fn push(&mut self, mut out: Out) {
        if self.units.is_empty() && !out.key {
            return;
        }
        if self.mark {
            // what follows a gap is only usable from a key unit; the child's first unit after a restart is one
            if !out.key {
                return;
            }
            self.mark = false;
            out.discontinuity = true;
        }
        let seq = self.first + self.units.len() as u64;
        if out.discontinuity {
            self.gap = Some(seq);
        }
        if out.key {
            self.keys.push_back(seq);
        }
        self.bytes += out.data.len();
        self.units.push_back(Arc::new(out));
        self.evict();
    }

    pub(crate) fn mark_gap(&mut self) {
        self.mark = true;
    }

    /// Payload bytes held.
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    fn pts_at(&self, seq: u64) -> Option<u64> {
        let index = usize::try_from(seq.checked_sub(self.first)?).ok()?;
        self.units.get(index).map(|u| u.pts)
    }

    /// Whether the groups after the oldest still span the time limit, so the oldest one is not needed for it.
    fn oldest_is_surplus_in_time(&self) -> bool {
        let (Some(newest), Some(&next)) = (self.units.back(), self.keys.get(1)) else {
            return false;
        };
        self.max_ticks > 0 && self.pts_at(next).is_some_and(|start| newest.pts.saturating_sub(start) >= self.max_ticks)
    }

    fn evict(&mut self) {
        while self.keys.len() >= 2 && (self.bytes > self.max_bytes || self.oldest_is_surplus_in_time()) {
            self.drop_oldest_group();
        }
        if self.bytes > self.max_bytes {
            // a single group larger than the ring: nothing to keep that would decode
            self.clear();
        }
    }

    fn drop_oldest_group(&mut self) {
        let Some(&next) = self.keys.get(1) else {
            return;
        };
        for _ in self.first..next {
            if let Some(unit) = self.units.pop_front() {
                self.bytes = self.bytes.saturating_sub(unit.data.len());
            }
        }
        self.first = next;
        self.keys.pop_front();
        if self.gap.is_some_and(|g| g < self.first) {
            self.gap = None;
        }
    }

    fn clear(&mut self) {
        self.first += self.units.len() as u64;
        self.units.clear();
        self.keys.clear();
        self.gap = None;
        self.bytes = 0;
    }

    /// The newest `seconds` of units, starting on a key unit and never across a gap: from the newest key unit at least `seconds` before the newest
    /// unit, or from the oldest one held when the ring holds less than that.
    pub(crate) fn clip(&self, seconds: u32) -> Result<Clip, ClipError> {
        let newest = self.units.back().ok_or(ClipError::Empty)?;
        let target = newest.pts.saturating_sub(u64::from(seconds) * u64::from(self.hz));
        let mut start = *self.keys.front().ok_or(ClipError::Empty)?;
        if let Some(&key) = self.keys.iter().rev().find(|&&k| self.pts_at(k).is_some_and(|pts| pts <= target)) {
            start = key;
        }
        if let Some(gap) = self.gap
            && gap > start
        {
            start = gap;
        }
        let skip = usize::try_from(start.saturating_sub(self.first)).unwrap_or(usize::MAX);
        let units: Vec<Arc<Out>> = self.units.iter().skip(skip).cloned().collect();
        let (Some(head), Some(tail)) = (units.first(), units.last()) else {
            return Err(ClipError::Empty);
        };
        let seconds = tail.pts.saturating_sub(head.pts) as f64 / f64::from(self.hz.max(1));
        Ok(Clip { pts_hz: self.hz, units, seconds: seconds as f32 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(pts: u64, key: bool, len: usize) -> Out {
        Out { pts, key, discontinuity: false, encode_tenths_ms: 0, data: vec![0; len] }
    }

    fn pts_of(ring: &Ring) -> Vec<u64> {
        ring.units.iter().map(|u| u.pts).collect()
    }

    #[test]
    fn a_full_queue_drops_up_to_the_next_key_unit_and_marks_it() {
        let mut q = Queue::new(3, 1 << 20);
        for pts in 0..3 {
            q.push(unit(pts, pts == 0, 10));
        }
        q.push(unit(3, false, 10));
        q.push(unit(4, true, 10));
        assert_eq!(q.take_dropped(), 2);
        // room again, but the stream is still in a hole: a delta unit is dropped, the key unit is taken and marked
        assert_eq!(q.pop().map(|u| u.pts), Some(0));
        assert_eq!(q.pop().map(|u| u.pts), Some(1));
        q.push(unit(5, false, 10));
        q.push(unit(6, true, 10));
        q.push(unit(7, false, 10));
        assert_eq!(q.take_dropped(), 1);
        let rest: Vec<(u64, bool)> = std::iter::from_fn(|| q.pop()).map(|u| (u.pts, u.discontinuity)).collect();
        assert_eq!(rest, [(2, false), (6, true), (7, false)]);
        assert_eq!(q.bytes, 0);
    }

    #[test]
    fn a_key_unit_that_does_not_fit_does_not_end_the_hole() {
        let mut q = Queue::new(2, 1 << 20);
        q.push(unit(0, true, 1));
        q.push(unit(1, false, 1));
        q.push(unit(2, false, 1));
        q.push(unit(3, true, 1));
        assert_eq!((q.take_dropped(), q.items.len(), q.skipping), (2, 2, true));
    }

    #[test]
    fn the_byte_limit_counts_like_the_unit_limit() {
        let mut q = Queue::new(100, 25);
        q.push(unit(0, true, 10));
        q.push(unit(1, false, 10));
        q.push(unit(2, false, 10));
        assert_eq!((q.items.len(), q.bytes, q.take_dropped()), (2, 20, 1));
    }

    #[test]
    fn a_gap_marks_the_next_unit_once() {
        let mut q = Queue::new(8, 1 << 20);
        q.push(unit(0, true, 1));
        q.mark_gap();
        q.push(unit(1, true, 1));
        q.push(unit(2, false, 1));
        let flags: Vec<bool> = std::iter::from_fn(|| q.pop()).map(|u| u.discontinuity).collect();
        assert_eq!(flags, [false, true, false]);
    }

    #[test]
    fn a_ring_starts_on_a_key_unit() {
        let mut ring = Ring::new(1000, 0, 1000);
        ring.push(unit(0, false, 10));
        ring.push(unit(1, false, 10));
        assert!(ring.units.is_empty());
        ring.push(unit(2, true, 10));
        ring.push(unit(3, false, 10));
        assert_eq!(pts_of(&ring), [2, 3]);
    }

    #[test]
    fn a_ring_over_its_bytes_drops_the_oldest_whole_group() {
        // groups of 5 units of 10 bytes, a key unit first
        let mut ring = Ring::new(120, 0, 1000);
        for pts in 0..14 {
            ring.push(unit(pts, pts % 5 == 0, 10));
            assert!(ring.bytes() <= 120);
            assert!(ring.units.front().is_none_or(|u| u.key), "starts with a delta unit at pts {pts}");
        }
        // 14 units pushed, 120 byte limit: the first group went at pts 12, the unit that took the ring to 130 bytes
        assert_eq!(pts_of(&ring), (5..14).collect::<Vec<_>>());
        assert_eq!(ring.bytes(), 90);
    }

    #[test]
    fn a_ring_over_its_time_keeps_at_least_the_time_and_at_most_a_group_more() {
        // one group a second at 1000 ticks a second, a unit every 250 ticks; keep 2 s
        let mut ring = Ring::new(1 << 20, 2, 1000);
        for n in 0..40u64 {
            ring.push(unit(n * 250, n % 4 == 0, 10));
            let (Some(first), Some(last)) = (ring.units.front(), ring.units.back()) else { panic!("empty") };
            let span = last.pts - first.pts;
            if n >= 8 {
                assert!((2000..=3000).contains(&span), "span {span} at unit {n}");
            }
            assert!(first.key);
        }
    }

    #[test]
    fn a_group_bigger_than_the_ring_leaves_it_empty_instead_of_growing() {
        let mut ring = Ring::new(100, 0, 1000);
        ring.push(unit(0, true, 60));
        ring.push(unit(1, false, 60));
        assert_eq!((ring.units.len(), ring.bytes()), (0, 0));
        ring.push(unit(2, false, 10));
        assert!(ring.units.is_empty());
        ring.push(unit(3, true, 10));
        assert_eq!(pts_of(&ring), [3]);
    }

    #[test]
    fn a_clip_starts_on_the_newest_key_unit_at_least_that_long_ago() {
        let mut ring = Ring::new(1 << 20, 0, 1000);
        // keys at 0, 1000, 2000, 3000; units every 500
        for n in 0..8u64 {
            ring.push(unit(n * 500, n % 2 == 0, 10));
        }
        let clip = ring.clip(1).unwrap();
        // the newest unit is 3500: the key unit at or before 2500 is 2000
        assert_eq!(clip.units.iter().map(|u| u.pts).collect::<Vec<_>>(), [2000, 2500, 3000, 3500]);
        assert!((clip.seconds - 1.5).abs() < 1e-6);
        assert_eq!(clip.pts_hz, 1000);
        // more than held: all of it, from the oldest key unit
        let all = ring.clip(60).unwrap();
        assert_eq!(all.units.len(), 8);
        // the clip shares the ring's units
        assert!(Arc::ptr_eq(&all.units[3], &ring.units[3]));
        // asking for nothing gives the last group
        assert_eq!(ring.clip(0).unwrap().units.first().map(|u| u.pts), Some(3000));
    }

    #[test]
    fn a_clip_never_spans_a_gap() {
        let mut ring = Ring::new(1 << 20, 0, 1000);
        ring.push(unit(0, true, 10));
        ring.push(unit(500, false, 10));
        ring.mark_gap();
        // a delta unit after the gap is not taken: only a key unit can follow it
        ring.push(unit(1000, false, 10));
        ring.push(unit(1500, true, 10));
        ring.push(unit(2000, false, 10));
        let clip = ring.clip(60).unwrap();
        assert_eq!(clip.units.iter().map(|u| (u.pts, u.discontinuity)).collect::<Vec<_>>(), [(1500, true), (2000, false)]);
        // the gap leaves with the group that holds it
        let mut small = Ring::new(35, 0, 1000);
        small.push(unit(0, true, 10));
        small.mark_gap();
        small.push(unit(1, true, 10));
        small.push(unit(2, false, 10));
        small.push(unit(3, true, 10));
        assert_eq!((pts_of(&small), small.gap), (vec![1, 2, 3], Some(1)));
        small.push(unit(4, false, 10));
        assert_eq!((pts_of(&small), small.gap), (vec![3, 4], None));
        assert!(small.clip(60).unwrap().units.iter().all(|u| !u.discontinuity));
    }

    #[test]
    fn an_empty_ring_has_no_clip() {
        let ring = Ring::new(100, 0, 1000);
        assert_eq!(ring.clip(5).unwrap_err(), ClipError::Empty);
    }
}
