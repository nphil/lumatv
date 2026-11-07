//! Volume+ and Volume- while the volume display sets the receiver's level, and the way back to the audio service.
//!
//! While the display says the receiver takes the volume keys (`volumeKeys`), a press or key repeat goes to it alone, numbered; it
//! answers with the newest number it counted (`volumeAck`) and sends its level through the helper. The audio service is not told: its
//! CEC step reached the Denon 110 to 270 ms after the display's level and left it one step past the bar on 12 of 12 taps (2026-11-06).
//! A key the display has not answered within [`ANSWER_WAIT`] (it hangs, or the helper or the receiver plugin went away under it) is the
//! service's after all, so the volume never goes dead; the service's report of that press names the key, so a display that was only
//! late counts it once. The release goes to the service only while a press of its own is open.
//!
//! The whole state, time passed in, so a test drives it with a fake clock.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// A key the display has not answered in this long is the audio service's.
pub const ANSWER_WAIT: Duration = Duration::from_millis(250);
/// The audio service reports a press within this long or not at all: a key given back longer ago names no report.
pub const REPORT_WAIT: Duration = Duration::from_secs(1);

/// What the audio service is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    /// A press or repeat: `master/volumeUp` (`true`) or `master/volumeDown`.
    Press(bool),
    /// The key was let go: `volumeKeeping`.
    Release,
}

#[derive(Debug, Default)]
pub struct VolumeKeys {
    last: u64,
    /// Keys given to the display and not answered, oldest first: number, Volume+, when.
    waiting: VecDeque<(u64, bool, Instant)>,
    /// Keys given back to the service whose report has not come, oldest first: number, when.
    given_back: VecDeque<(u64, Instant)>,
    /// The service has a press it has not been told the end of.
    open: bool,
    /// The key was let go while keys still waited for the display.
    released: bool,
}

impl VolumeKeys {
    /// A press or repeat given to the display; its number goes with it.
    pub fn to_display(&mut self, up: bool, now: Instant) -> u64 {
        self.last += 1;
        self.released = false;
        self.waiting.push_back((self.last, up, now));
        self.last
    }

    /// A press or repeat that went to the service.
    pub fn to_service(&mut self) {
        self.released = false;
        self.open = true;
    }

    /// The display counted the key numbered `ack`, and every key before it. A number that names no waiting key is old news: a display
    /// still reporting what it counted for a compositor that has since restarted.
    pub fn answered(&mut self, ack: u64) -> Option<Call> {
        if self.waiting.iter().any(|&(n, ..)| n == ack) {
            while self.waiting.front().is_some_and(|&(n, ..)| n <= ack) {
                self.waiting.pop_front();
            }
        }
        self.settle()
    }

    /// The key was let go: the service's release now, once the keys still waiting are settled, or never (it has no press open).
    pub fn release(&mut self) -> Option<Call> {
        self.released = true;
        self.settle()
    }

    /// The keys the display has left unanswered for [`ANSWER_WAIT`], as the service's presses, and the release after them.
    pub fn due(&mut self, now: Instant) -> Vec<Call> {
        let mut calls = Vec::new();
        while let Some(&(n, up, at)) = self.waiting.front()
            && now >= at + ANSWER_WAIT
        {
            self.waiting.pop_front();
            self.given_back.push_back((n, now));
            self.open = true;
            calls.push(Call::Press(up));
        }
        calls.extend(self.settle());
        calls
    }

    /// The key given back that this report of a press answers, if any.
    pub fn report(&mut self, now: Instant) -> Option<u64> {
        while self.given_back.front().is_some_and(|&(_, at)| now > at + REPORT_WAIT) {
            self.given_back.pop_front();
        }
        self.given_back.pop_front().map(|(n, _)| n)
    }

    /// When the oldest waiting key becomes the service's.
    pub fn deadline(&self) -> Option<Instant> {
        self.waiting.front().map(|&(_, _, at)| at + ANSWER_WAIT)
    }

    fn settle(&mut self) -> Option<Call> {
        if !self.released || !self.waiting.is_empty() {
            return None;
        }
        self.released = false;
        std::mem::take(&mut self.open).then_some(Call::Release)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_held_key_the_display_answers_never_reaches_the_service() {
        let (mut k, t0) = (VolumeKeys::default(), Instant::now());
        for i in 0..10u32 {
            let n = k.to_display(true, t0 + i * 100 * MS);
            // each answer comes after the next repeat: the service waits on none of them
            assert_eq!(k.answered(n.saturating_sub(1)), None);
            assert!(k.due(t0 + i * 100 * MS).is_empty());
        }
        assert_eq!(k.release(), None);
        assert_eq!(k.answered(10), None, "the service had no press to end");
        assert_eq!((k.deadline(), k.due(t0 + 5000 * MS)), (None, vec![]));
    }

    #[test]
    fn keys_the_display_stops_answering_go_to_the_service_in_order_and_the_release_after_them() {
        let (mut k, t0) = (VolumeKeys::default(), Instant::now());
        k.to_display(true, t0);
        k.to_display(true, t0 + 100 * MS);
        k.to_display(false, t0 + 200 * MS);
        assert_eq!(k.answered(57), None);
        assert_eq!(k.deadline(), Some(t0 + ANSWER_WAIT), "an answer naming no key here is an earlier compositor's");
        assert_eq!(k.answered(1), None);
        assert_eq!(k.release(), None, "the release waits for the keys before it");
        assert_eq!(k.deadline(), Some(t0 + 100 * MS + ANSWER_WAIT));
        assert!(k.due(t0 + 349 * MS).is_empty());
        assert_eq!(k.due(t0 + 350 * MS), [Call::Press(true)]);
        assert_eq!(k.due(t0 + 450 * MS), [Call::Press(false), Call::Release]);
        assert_eq!(k.answered(3), None, "a late answer changes nothing");
        // the service's reports name the keys given back, oldest first, while they can still be the answer
        assert_eq!((k.report(t0 + 400 * MS), k.report(t0 + 1500 * MS)), (Some(2), None));
    }

    #[test]
    fn a_press_the_service_already_had_is_ended_once_the_display_has_answered_the_keys_after_it() {
        let (mut k, t0) = (VolumeKeys::default(), Instant::now());
        k.to_service();
        let n = k.to_display(true, t0);
        assert_eq!(k.release(), None);
        assert_eq!(k.answered(n), Some(Call::Release));
        assert_eq!(k.release(), None);
    }
}
