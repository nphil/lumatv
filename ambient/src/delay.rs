//! The output delay: lights change `delay_ms` after the picture was captured, to line up with what the viewer sees
//! and hears (the picture reaches the panel after the capture tap, and sound through the receiver arrives later still).

use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct DelayLine<T> {
    items: VecDeque<(u64, T)>,
    cap: usize,
}

impl<T> DelayLine<T> {
    /// `cap` bounds memory: a stalled consumer drops the oldest entries rather than growing.
    pub fn new(cap: usize) -> DelayLine<T> {
        DelayLine { items: VecDeque::with_capacity(cap.min(1024)), cap: cap.max(1) }
    }

    pub fn push(&mut self, captured_ms: u64, item: T) {
        if self.items.len() == self.cap {
            self.items.pop_front();
        }
        self.items.push_back((captured_ms, item));
    }

    /// The newest item captured at or before `now_ms - delay_ms`, with everything older discarded; `None` when
    /// nothing is due yet or nothing new is due since the last call.
    pub fn pop_due(&mut self, now_ms: u64, delay_ms: u64) -> Option<T> {
        let mut due = None;
        while let Some((t, _)) = self.items.front() {
            if t.saturating_add(delay_ms) <= now_ms {
                due = self.items.pop_front().map(|(_, v)| v);
            } else {
                break;
            }
        }
        due
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_due_before_the_delay() {
        let mut d = DelayLine::new(16);
        d.push(1000, 'a');
        assert_eq!(d.pop_due(1099, 100), None);
        assert_eq!(d.pop_due(1100, 100), Some('a'));
        assert_eq!(d.pop_due(1200, 100), None, "already taken");
    }

    #[test]
    fn the_newest_due_item_wins_and_older_ones_are_dropped() {
        let mut d = DelayLine::new(16);
        for (t, c) in [(0, 'a'), (20, 'b'), (40, 'c'), (60, 'd')] {
            d.push(t, c);
        }
        assert_eq!(d.pop_due(100, 50), Some('c'), "a, b and c are due (captured at or before 50 ms), c is newest");
        assert_eq!(d.len(), 1);
        assert_eq!(d.pop_due(110, 50), Some('d'), "d is due at 110");
        assert!(d.is_empty());
    }

    #[test]
    fn zero_delay_passes_straight_through() {
        let mut d = DelayLine::new(4);
        d.push(5, 1);
        assert_eq!(d.pop_due(5, 0), Some(1));
    }

    #[test]
    fn capacity_drops_the_oldest() {
        let mut d = DelayLine::new(3);
        for i in 0..5u64 {
            d.push(i, i);
        }
        assert_eq!(d.len(), 3);
        assert_eq!(d.pop_due(1000, 0), Some(4));
    }

    #[test]
    fn a_long_delay_holds_several_items() {
        let mut d = DelayLine::new(64);
        for i in 0..30u64 {
            d.push(i * 33, i);
        }
        assert_eq!(d.pop_due(500, 300), Some(6), "captured at or before 200 ms: item 6 at 198 ms");
    }
}
