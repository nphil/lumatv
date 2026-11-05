//! The way out of LG's screensaver, which has to work with nobody else's help.
//!
//! A key or the pointer starts an episode; while it lasts the loop's own tick goes on asking (`tick`), so one refused call (tvpower
//! answered `power is busy` in the receipts of the first TV checks) does not need a second key.
//!
//! LG's saver is a window of LG's own app above the retained app; the compositor swallows the keys while it is up and asks tvpower to end
//! it (`power2/turnOnScreen`). Both halves can fail without a sound: tvpower's answer never comes, tvpower refuses, or the saver's window
//! stays mapped after the state is `Active` (its app hung). A swallowed key with no way out leaves the set on the saver until the power key
//! is used. This is the whole state of that wake, time passed in, so a test drives it with a fake clock:
//!
//! - a wake call with no answer in [`ANSWER_WAIT`] is lost and is made again; while the saver stays up it is repeated every [`RETRY`];
//! - the saver still up [`CLOSE_AFTER`] after the first key is asked to close (its app, through SAM and its window);
//! - still up [`ABANDON_AFTER`] after the first key it is given up on: the keys reach the app under it and its window is hidden.
//!
//! The episode ends when neither the saver's window nor the power state `Screen Saver` is there any more (`step`'s `engaged` false).

use std::time::{Duration, Instant};

/// A wake call this old without an answer is taken as lost.
pub const ANSWER_WAIT: Duration = Duration::from_secs(2);
/// The least time between two wake calls while the saver stays up.
pub const RETRY: Duration = Duration::from_secs(1);
/// The saver is asked to close this long after the first key that found it up.
pub const CLOSE_AFTER: Duration = Duration::from_secs(3);
/// The saver is given up on this long after the first key that found it up.
pub const ABANDON_AFTER: Duration = Duration::from_secs(6);

#[derive(Debug, Default)]
pub struct SaverGuard {
    pending: Option<(u64, Instant)>,
    last_wake: Option<Instant>,
    since: Option<Instant>,
    closed: bool,
    abandoned: bool,
    pub wakes: u64,
    pub lost: u64,
    pub refused: u64,
    pub closes: u64,
    pub abandons: u64,
}

/// What one key or pointer event asks of the compositor.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Step {
    pub wake: bool,
    pub close: bool,
    pub abandon: bool,
}

impl SaverGuard {
    /// Whether the saver still has the keys: it is there (`engaged`) and has not been given up on.
    pub fn holds_keys(&self, engaged: bool) -> bool {
        engaged && !self.abandoned
    }

    pub fn abandoned(&self) -> bool {
        self.abandoned
    }

    /// The saver's state at a look of the loop: an episode that is over is forgotten, so the next saver starts afresh.
    pub fn observe(&mut self, engaged: bool) {
        if !engaged && (self.since.is_some() || self.abandoned || self.closed) {
            self.since = None;
            self.closed = false;
            self.abandoned = false;
            self.last_wake = None;
            self.pending = None;
        }
    }

    /// A look of the loop's tick: an episode that a key began goes on (asking again, closing, giving up) without another key; with no
    /// episode it only notices that the saver is gone.
    pub fn tick(&mut self, engaged: bool, now: Instant) -> Step {
        if self.since.is_none() {
            self.observe(engaged);
            return Step::default();
        }
        self.input(engaged, now)
    }

    /// A key or the pointer while the saver may be up.
    pub fn input(&mut self, engaged: bool, now: Instant) -> Step {
        self.observe(engaged);
        if !engaged || self.abandoned {
            return Step::default();
        }
        let since = *self.since.get_or_insert(now);
        let mut step = Step::default();
        let up_for = now.saturating_duration_since(since);
        if up_for >= ABANDON_AFTER {
            self.abandoned = true;
            self.abandons += 1;
            step.abandon = true;
            return step;
        }
        if up_for >= CLOSE_AFTER && !self.closed {
            self.closed = true;
            self.closes += 1;
            step.close = true;
        }
        if self.pending.is_some_and(|(_, at)| now.saturating_duration_since(at) >= ANSWER_WAIT) {
            self.pending = None;
            self.lost += 1;
        }
        if self.pending.is_none() && self.last_wake.is_none_or(|at| now.saturating_duration_since(at) >= RETRY) {
            step.wake = true;
        }
        step
    }

    /// The wake call went out under `token`.
    pub fn sent(&mut self, token: u64, now: Instant) {
        self.pending = Some((token, now));
        self.last_wake = Some(now);
        self.wakes += 1;
    }

    /// An answer: true when `token` was the wake call's. A refusal is counted and the call may be made again at the next key.
    pub fn answered(&mut self, token: u64, ok: bool) -> bool {
        if self.pending.map(|(t, _)| t) != Some(token) {
            return false;
        }
        self.pending = None;
        if !ok {
            self.refused += 1;
        }
        true
    }

    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({
            "pending": self.pending.is_some(), "abandoned": self.abandoned, "wakes": self.wakes, "lost": self.lost,
            "refused": self.refused, "closes": self.closes, "abandons": self.abandons,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn a_key_on_a_saver_makes_one_wake_call_and_waits_for_its_answer() {
        let t0 = Instant::now();
        let mut g = SaverGuard::default();
        assert!(g.input(true, t0).wake);
        g.sent(7, t0);
        assert_eq!(g.input(true, at(t0, 300)), Step::default(), "the call is in flight");
        assert!(g.answered(7, true));
        assert_eq!(g.input(true, at(t0, 600)), Step::default(), "answered, but not asked again inside the retry spacing");
        assert!(g.input(true, at(t0, 1100)).wake, "still up a second on: asked again");
    }

    #[test]
    fn a_wake_call_that_is_never_answered_is_made_again() {
        let t0 = Instant::now();
        let mut g = SaverGuard::default();
        assert!(g.input(true, t0).wake);
        g.sent(7, t0);
        assert!(!g.input(true, at(t0, 1999)).wake);
        assert!(g.input(true, at(t0, 2000)).wake, "two seconds without an answer: lost");
        assert_eq!(g.lost, 1);
        g.sent(8, at(t0, 2000));
        assert!(!g.answered(7, true), "the lost call's late answer is nobody's");
    }

    #[test]
    fn a_refused_wake_is_counted_and_tried_again() {
        let t0 = Instant::now();
        let mut g = SaverGuard::default();
        g.input(true, t0);
        g.sent(1, t0);
        assert!(g.answered(1, false));
        assert_eq!(g.refused, 1);
        assert!(g.input(true, at(t0, 1000)).wake);
    }

    #[test]
    fn a_saver_that_stays_up_is_closed_and_then_given_up_on() {
        let t0 = Instant::now();
        let mut g = SaverGuard::default();
        g.input(true, t0);
        g.sent(1, t0);
        g.answered(1, true);
        let mut closes = 0;
        for ms in (1000..6000).step_by(500) {
            let s = g.input(true, at(t0, ms));
            closes += s.close as u32;
            assert!(!s.abandon, "not before six seconds ({ms})");
            if s.wake {
                g.sent(ms, at(t0, ms));
                g.answered(ms, true);
            }
        }
        assert_eq!(closes, 1, "asked to close once");
        assert!(g.holds_keys(true));
        let s = g.input(true, at(t0, 6000));
        assert!(s.abandon && !s.wake);
        assert!(!g.holds_keys(true), "the keys reach the app now");
        assert_eq!(g.input(true, at(t0, 7000)), Step::default(), "nothing more is done for a saver given up on");
        assert_eq!((g.closes, g.abandons), (1, 1));
    }

    #[test]
    fn the_episode_ends_with_the_saver_and_the_next_one_starts_afresh() {
        let t0 = Instant::now();
        let mut g = SaverGuard::default();
        g.input(true, t0);
        g.input(true, at(t0, 6000));
        assert!(g.abandoned());
        g.observe(false);
        assert!(!g.abandoned());
        let s = g.input(true, at(t0, 600_000));
        assert!(s.wake && !s.close && !s.abandon, "a saver minutes later is a new one");
    }

    #[test]
    fn a_refused_wake_is_asked_again_by_the_tick_without_another_key() {
        let t0 = Instant::now();
        let mut g = SaverGuard::default();
        assert_eq!(g.tick(true, t0), Step::default(), "a saver nobody has pressed a key on is left alone");
        assert!(g.input(true, at(t0, 10)).wake);
        g.sent(1, at(t0, 10));
        assert!(g.answered(1, false), "power is busy");
        assert_eq!(g.tick(true, at(t0, 500)), Step::default());
        assert!(g.tick(true, at(t0, 1100)).wake, "a second on, the tick asks again");
        g.sent(2, at(t0, 1100));
        assert!(g.answered(2, true));
        g.tick(true, at(t0, 3100));
        assert!(g.tick(true, at(t0, 6100)).abandon, "and the tick gives up on it at six seconds");
        g.tick(false, at(t0, 7000));
        assert!(!g.abandoned(), "the end of the saver ends the episode");
    }

    #[test]
    fn nothing_is_asked_when_no_saver_is_up() {
        let mut g = SaverGuard::default();
        assert_eq!(g.input(false, Instant::now()), Step::default());
        assert!(!g.holds_keys(false));
    }
}
