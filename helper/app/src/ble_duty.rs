//! How much of the time the radio may be asked to scan, as a state machine with no I/O (the caller says what time it is).
//!
//! The set's Wi-Fi and Bluetooth are one module with one antenna, so a scan costs Wi-Fi throughput and a stream of Bluetooth
//! audio, and LG's own remote link runs through the same controller. The limits, none of them a setting that can be turned past
//! them (`ble_store` checks the numbers):
//!
//! - a **window** of 5 to 30 seconds (10 by default), then a **rest** at least as long as the window (50 s by default): never
//!   more than half the time, a sixth by default;
//! - the rest counts from the end of the last window **even when the proxy's subscriber changes**: Home Assistant reconnecting in
//!   a loop gets one window per rest, not one per connection;
//! - no window while the panel is dark (unless the owner chose standby scanning), while a Bluetooth audio link is up, or while
//!   the adapter is off: the caller holds the machine for a while ([`Duty::hold`]) and looks again;
//! - a scan that fails or ends by itself is not retried at once: 1 minute, then 2, 4, 8, 16 (at most 30), and after five failures
//!   in a row the machine stops trying until [`Duty::reset`] (the owner linking again, or the helper starting), so a service that
//!   refuses the call costs five attempts, not a loop.

use std::time::{Duration, Instant};

/// The first wait after a failure, and the longest.
pub const BACKOFF_FIRST: Duration = Duration::from_secs(60);
pub const BACKOFF_MAX: Duration = Duration::from_secs(30 * 60);
/// Failures in a row after which the machine gives up.
pub const MAX_FAILURES: u32 = 5;
/// How long a hold lasts before the guards are looked at again.
pub const HOLD_SCREEN: Duration = Duration::from_secs(5);
pub const HOLD_RADIO: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// No window is wanted or none has happened yet.
    Idle,
    /// A scan is on until `until`.
    Window { until: Instant },
    /// The radio is left alone until `until`.
    Rest { until: Instant },
    /// A guard says no: looked at again at `until`.
    Held { until: Instant },
    /// A scan failed: not tried again before `until`.
    Backoff { until: Instant },
    /// Too many failures: no more attempts.
    GaveUp,
}

#[derive(Clone, Debug)]
pub struct Duty {
    scan: Duration,
    rest: Duration,
    phase: Phase,
    failures: u32,
    /// When the last window ended (or failed): the rest counts from here.
    last_end: Option<Instant>,
    windows: u64,
}

impl Duty {
    pub fn new(scan_seconds: u32, rest_seconds: u32) -> Self {
        Self {
            scan: Duration::from_secs(u64::from(scan_seconds)),
            rest: Duration::from_secs(u64::from(rest_seconds)),
            phase: Phase::Idle,
            failures: 0,
            last_end: None,
            windows: 0,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn windows(&self) -> u64 {
        self.windows
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    pub fn is_scanning(&self) -> bool {
        matches!(self.phase, Phase::Window { .. })
    }

    pub fn gave_up(&self) -> bool {
        self.phase == Phase::GaveUp
    }

    /// Forgets the failures (a new link, a new start).
    pub fn reset(&mut self) {
        self.failures = 0;
        if matches!(self.phase, Phase::GaveUp | Phase::Backoff { .. }) {
            self.phase = Phase::Idle;
        }
    }

    /// Whether a window may start now: the machine is not scanning and not waiting, and the rest since the last window is over.
    pub fn due(&self, now: Instant) -> bool {
        let waited = |until: Instant| now >= until;
        let phase_ok = match self.phase {
            Phase::Idle => true,
            Phase::Rest { until } | Phase::Held { until } | Phase::Backoff { until } => {
                waited(until)
            }
            Phase::Window { .. } | Phase::GaveUp => false,
        };
        phase_ok && self.last_end.is_none_or(|end| now >= end + self.rest)
    }

    /// The window starts now.
    pub fn begin(&mut self, now: Instant) {
        self.phase = Phase::Window {
            until: now + self.scan,
        };
        self.windows += 1;
    }

    /// Whether the window that is on has run its time.
    pub fn window_over(&self, now: Instant) -> bool {
        matches!(self.phase, Phase::Window { until } if now >= until)
    }

    /// The window ends (its time is up, or nobody wants it any more): the rest counts from now. Not a failure.
    pub fn end_window(&mut self, now: Instant) {
        if self.is_scanning() {
            self.failures = 0;
            self.phase = Phase::Rest {
                until: now + self.rest,
            };
            self.last_end = Some(now);
        }
    }

    /// A guard says no: the machine looks again after `retry`. A window that is on ends first (its rest is not owed: it ended
    /// because it was told to).
    pub fn hold(&mut self, now: Instant, retry: Duration) {
        if self.is_scanning() {
            self.last_end = Some(now);
        }
        self.phase = Phase::Held { until: now + retry };
    }

    /// The scan failed or ended by itself. Returns true when this was the last attempt.
    pub fn failed(&mut self, now: Instant) -> bool {
        self.failures += 1;
        self.last_end = Some(now);
        if self.failures >= MAX_FAILURES {
            self.phase = Phase::GaveUp;
            return true;
        }
        let wait = BACKOFF_FIRST
            .checked_mul(1 << (self.failures - 1))
            .unwrap_or(BACKOFF_MAX)
            .min(BACKOFF_MAX);
        self.phase = Phase::Backoff { until: now + wait };
        false
    }

    /// When the machine next needs the caller: the end of the window, or the end of whatever it waits for.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let rest_over = self.last_end.map(|end| end + self.rest);
        match self.phase {
            Phase::Window { until } => Some(until),
            Phase::Rest { until } | Phase::Held { until } | Phase::Backoff { until } => {
                Some(rest_over.map_or(until, |r| r.max(until)))
            }
            Phase::Idle => rest_over.filter(|r| *r > now),
            Phase::GaveUp => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn a_window_then_a_rest_and_never_more_than_the_share() {
        let t0 = Instant::now();
        let mut d = Duty::new(10, 50);
        assert!(d.due(t0));
        d.begin(t0);
        assert!(d.is_scanning() && !d.due(t0 + s(5)));
        assert!(!d.window_over(t0 + s(9)) && d.window_over(t0 + s(10)));
        d.end_window(t0 + s(10));
        assert!(!d.due(t0 + s(59)));
        assert!(d.due(t0 + s(60)));
        d.begin(t0 + s(60));
        assert_eq!(d.windows(), 2);
    }

    #[test]
    fn a_window_cut_short_still_owes_the_whole_rest() {
        // Home Assistant disconnects 2 s into a window and connects again at once: no new window before the rest is over.
        let t0 = Instant::now();
        let mut d = Duty::new(10, 50);
        d.begin(t0);
        d.end_window(t0 + s(2));
        assert!(!d.due(t0 + s(3)));
        assert!(!d.due(t0 + s(51)));
        assert!(d.due(t0 + s(52)));
    }

    #[test]
    fn a_hold_is_looked_at_again_and_costs_no_rest_when_nothing_ran() {
        let t0 = Instant::now();
        let mut d = Duty::new(10, 50);
        d.hold(t0, HOLD_SCREEN);
        assert!(!d.due(t0 + s(4)) && d.due(t0 + s(5)));
        // A window that is held to an end (the screen went dark) owes its rest from there.
        d.begin(t0 + s(5));
        d.hold(t0 + s(8), HOLD_SCREEN);
        assert!(!d.due(t0 + s(20)) && d.due(t0 + s(58)));
    }

    #[test]
    fn failures_back_off_and_five_end_the_trying() {
        let t0 = Instant::now();
        let mut d = Duty::new(10, 50);
        let mut now = t0;
        for (n, wait) in [(1, 60), (2, 120), (3, 240), (4, 480)] {
            d.begin(now);
            assert!(!d.failed(now), "failure {n}");
            assert_eq!(
                d.phase(),
                Phase::Backoff {
                    until: now + s(wait)
                }
            );
            assert!(!d.due(now + s(wait - 1)) && d.due(now + s(wait)));
            now += s(wait);
        }
        d.begin(now);
        assert!(d.failed(now));
        assert!(d.gave_up() && !d.due(now + s(100_000)));
        assert_eq!(d.next_deadline(now), None);
        d.reset();
        assert!(d.due(now + s(50)));
    }

    #[test]
    fn a_window_that_runs_its_time_clears_the_failures() {
        let t0 = Instant::now();
        let mut d = Duty::new(10, 50);
        d.begin(t0);
        d.failed(t0);
        assert_eq!(d.failures(), 1);
        d.begin(t0 + s(60));
        d.end_window(t0 + s(70));
        assert_eq!(d.failures(), 0);
    }

    #[test]
    fn the_next_deadline_is_what_the_machine_waits_for() {
        let t0 = Instant::now();
        let mut d = Duty::new(10, 50);
        assert_eq!(d.next_deadline(t0), None);
        d.begin(t0);
        assert_eq!(d.next_deadline(t0), Some(t0 + s(10)));
        d.end_window(t0 + s(10));
        assert_eq!(d.next_deadline(t0 + s(11)), Some(t0 + s(60)));
    }
}
