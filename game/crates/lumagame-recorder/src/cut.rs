//! When the recorder asks the game server for a keyframe. A stream from a low-latency encoder may carry one keyframe for the whole
//! session, and a clip can only start on a keyframe, so the recorder makes its own cut points. Two kinds of ask:
//!
//! * Steady: one keyframe every `every` while the recorder runs. Any keyframe that arrives, asked for or not, restarts the count, so a
//!   stream that carries its own keyframes costs nothing extra.
//! * Urgent: something is waiting for a keyframe now (Record, Save clip, a ring that holds none). Asked at once, again every
//!   [`RETRY_MS`] up to [`ATTEMPTS`] times, then given up on, so the viewer is told rather than left waiting.
//!
//! Neither kind goes out while the stream is in trouble (the decoder is recovering or the link is dropping packets): the decoder asks
//! for its own recovery keyframe then, and a second ask on top would only add to the load. Time is a plain millisecond count, so the
//! rules are testable with a fake clock.

pub const RETRY_MS: u64 = 2_000;
pub const ATTEMPTS: u8 = 3;
/// An urgent ask is given up on at the latest this long after it began, whatever held it back.
pub const GIVE_UP_MS: u64 = 15_000;
/// Quiet after the last sign of trouble.
pub const HOLD_MS: u64 = 3_000;
/// A ring with no keyframe to start from, while video flows, is an urgent need after this long: the stream's own first keyframe has
/// had its chance by then.
pub const STARVED_MS: u64 = 2_000;

#[derive(Debug)]
struct Urgent {
    since: u64,
    sent: u8,
    due: u64,
}

/// What to do now.
#[derive(Debug, PartialEq, Eq)]
pub enum Move {
    Idle,
    Ask,
    /// An urgent need went unmet: tell whoever waits.
    GaveUp,
}

#[derive(Debug)]
pub struct Cutter {
    /// Milliseconds between steady asks; 0 asks only when something is waiting.
    every_ms: u64,
    /// The newest keyframe's arrival or the newest ask, whichever is later.
    mark: u64,
    urgent: Option<Urgent>,
    starved_since: Option<u64>,
}

impl Cutter {
    pub fn new(every_s: u32, now: u64) -> Self {
        Self { every_ms: u64::from(every_s) * 1000, mark: now, urgent: None, starved_since: None }
    }

    /// A video keyframe reached the recorder.
    pub fn key_arrived(&mut self, now: u64) {
        self.mark = now;
        self.urgent = None;
    }

    /// Something waits for a keyframe.
    pub fn urgent(&mut self, now: u64) {
        self.urgent.get_or_insert(Urgent { since: now, sent: 0, due: now });
    }

    /// `quiet_until`: no ask goes out before this time. `ready`: the ring holds a keyframe to start from. `video`: video has arrived.
    pub fn poll(&mut self, now: u64, quiet_until: u64, ready: bool, video: bool) -> Move {
        if ready || !video {
            self.starved_since = None;
        } else if let Some(since) = self.starved_since {
            if now.saturating_sub(since) >= STARVED_MS {
                self.urgent(now);
            }
        } else {
            self.starved_since = Some(now);
        }
        if let Some(u) = &self.urgent {
            if now.saturating_sub(u.since) >= GIVE_UP_MS || (u.sent >= ATTEMPTS && now >= u.due) {
                self.urgent = None;
                self.mark = now;
                self.starved_since = None;
                return Move::GaveUp;
            }
        }
        if now < quiet_until {
            return Move::Idle;
        }
        if let Some(u) = &mut self.urgent {
            if now < u.due {
                return Move::Idle;
            }
            u.sent += 1;
            u.due = now + RETRY_MS;
            self.mark = now;
            return Move::Ask;
        }
        if self.every_ms > 0 && now.saturating_sub(self.mark) >= self.every_ms {
            self.mark = now;
            return Move::Ask;
        }
        Move::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asks(c: &mut Cutter, from: u64, to: u64, step: u64, quiet: u64) -> Vec<u64> {
        (from..=to).step_by(step as usize).filter(|&t| c.poll(t, quiet, true, true) == Move::Ask).collect()
    }

    #[test]
    fn steady_asks_come_once_an_interval_and_any_keyframe_restarts_the_count() {
        let mut c = Cutter::new(5, 0);
        assert_eq!(asks(&mut c, 0, 21_000, 100, 0), [5_000, 10_000, 15_000, 20_000]);
        c.key_arrived(22_000);
        assert_eq!(asks(&mut c, 22_100, 26_900, 100, 0), Vec::<u64>::new(), "a keyframe of the stream's own counts");
        assert_eq!(asks(&mut c, 27_000, 27_000, 100, 0), [27_000]);
    }

    #[test]
    fn only_when_i_save_asks_for_nothing_by_itself() {
        let mut c = Cutter::new(0, 0);
        assert!(asks(&mut c, 0, 120_000, 100, 0).is_empty());
        c.urgent(120_000);
        assert_eq!(c.poll(120_000, 0, true, true), Move::Ask);
    }

    #[test]
    fn trouble_holds_asks_back_without_using_up_an_urgent_ones_attempts() {
        let mut c = Cutter::new(5, 0);
        assert!(asks(&mut c, 0, 9_000, 100, 20_000).is_empty(), "the steady ask waits out the trouble");
        c.urgent(9_000);
        assert!(asks(&mut c, 9_000, 19_900, 100, 20_000).is_empty());
        assert_eq!(asks(&mut c, 20_000, 20_000, 100, 20_000), [20_000], "the first attempt goes out when the quiet ends");
    }

    #[test]
    fn an_urgent_ask_is_retried_a_few_times_and_then_given_up() {
        let mut c = Cutter::new(0, 0);
        c.urgent(1_000);
        let mut told = Vec::new();
        for t in (1_000..=9_000).step_by(100) {
            match c.poll(t, 0, false, true) {
                Move::Idle => {}
                m => told.push((t, m)),
            }
        }
        assert_eq!(told, [(1_000, Move::Ask), (3_000, Move::Ask), (5_000, Move::Ask), (7_000, Move::GaveUp)]);
    }

    #[test]
    fn an_urgent_ask_waiting_out_trouble_is_still_given_up_at_its_deadline() {
        let mut c = Cutter::new(0, 0);
        c.urgent(0);
        assert_eq!(c.poll(15_000, u64::MAX, false, true), Move::GaveUp);
    }

    #[test]
    fn a_keyframe_settles_an_urgent_ask() {
        let mut c = Cutter::new(0, 0);
        c.urgent(0);
        assert_eq!(c.poll(0, 0, false, true), Move::Ask);
        c.key_arrived(500);
        assert!(asks(&mut c, 500, 20_000, 100, 0).is_empty());
    }

    #[test]
    fn a_ring_with_no_keyframe_to_start_from_is_an_urgent_need_after_a_grace_and_not_before_video() {
        let mut c = Cutter::new(10, 0);
        assert!((0..=5_000).step_by(100).all(|t| c.poll(t, 0, false, false) == Move::Idle), "no video, nothing to want");
        assert_eq!(c.poll(5_100, 0, false, true), Move::Idle, "the stream's own first keyframe gets its chance");
        assert_eq!(c.poll(7_000, 0, false, true), Move::Idle);
        assert_eq!(c.poll(7_100, 0, false, true), Move::Ask);
    }
}
