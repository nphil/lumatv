//! The frame tick's timer, and the one thing it does when nothing is happening: wake rarely (LPC 2026, "batching wakeups": the Echo
//! woke 474 times a second at idle). On the tenth gate's rounds the loop had a periodic timer at the panel's rate, and within
//! three minutes of the last flip a one-shot on the panel's grid at every refresh: 60 wake-ups a second on a Home nobody was
//! touching, each to find nothing to draw, so a covered Home (an app's film in front) and a still Home cost the CPU the same
//! 60 a second as one scrolling. What a tick that draws nothing still does is housekeeping: the hold of Home and of the
//! remote's keys, the shell client's supervision, the cursor's timeout, the stand-in a warm owes. None of it is due faster than
//! 100 ms, and anything that needs a frame sooner arrives as an event (a client's commit, a key, the UI app, the shell's
//! channel, a flip) which draws at once on a still screen and puts the tick back to the panel's rate.
//!
//! So the tick is slowed to [`IDLE_TICK_NS`] only when all of these hold: no event for [`CALM_AFTER_NS`] (a key held without
//! repeats needs the hold measured, so input counts for [`INPUT_CALM_AFTER_NS`], longer than the longest hold, 2 s), the output
//! says nothing is in flight, owed or drawn in the last second (`Output::idle_ok`), and no commit waits to be drawn. The next
//! event ends it: the pass that handled it re-arms the tick at the panel's rate before it sleeps.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

use crate::State;
use crate::output::monotonic_ns;

/// The slowed tick: ten a second, on the panel's grid where the output keeps it.
pub const IDLE_TICK_NS: u64 = 100_000_000;
/// No event for this long before the tick may slow.
pub const CALM_AFTER_NS: u64 = 250_000_000;
/// No input event for this long: the longest hold the routing measures is Mute's 2 s (`keys::HOLD_MUTE`).
pub const INPUT_CALM_AFTER_NS: u64 = 3_000_000_000;
/// The shortest one-shot: a latch already in the past fires at once, but not in a spin.
const MIN_ONCE: Duration = Duration::from_micros(500);

/// When the loop last had something to do other than the tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Calm {
    last_event_ns: u64,
    last_input_ns: u64,
}

impl Calm {
    /// A wake-up for something other than the tick, at `now_ns`; `input` for an input device.
    pub fn event(&mut self, now_ns: u64, input: bool) {
        self.last_event_ns = now_ns;
        if input {
            self.last_input_ns = now_ns;
        }
    }

    pub fn is_calm(&self, now_ns: u64) -> bool {
        now_ns.saturating_sub(self.last_event_ns) >= CALM_AFTER_NS && now_ns.saturating_sub(self.last_input_ns) >= INPUT_CALM_AFTER_NS
    }
}

/// What to do with the timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// A one-shot this far ahead.
    Once(Duration),
    /// A periodic timer of this many ns (started one period from now).
    Periodic(u64),
    /// It is a periodic timer of the right period: left alone, so it keeps its phase.
    Keep,
}

/// The timer's next arming. `latch` is how far ahead the output wants the tick (its pacer's latch, or the panel's grid), `slow_ns` the
/// slowed period while the slowdown is in force, `tick_ns` the panel's rate; `paced` and `period_ns` say how the timer is armed now.
pub fn plan(paced: bool, period_ns: u64, tick_ns: u64, slow_ns: Option<u64>, latch: Option<Duration>) -> Plan {
    match latch {
        Some(d) => Plan::Once(d.max(MIN_ONCE)),
        None => {
            let want = slow_ns.unwrap_or(tick_ns);
            if paced || period_ns != want { Plan::Periodic(want) } else { Plan::Keep }
        }
    }
}

/// The tick's timerfd and how it is armed.
pub struct Tick {
    fd: OwnedFd,
    /// A one-shot at the output's latch (true) or periodic (false).
    paced: bool,
    /// The period of the periodic timer while it is one, ns.
    period_ns: u64,
    /// Whether the last arming was the slowed one.
    slowed: bool,
    pub calm: Calm,
}

impl Tick {
    /// A timer armed periodically at `interval_ns`.
    pub fn new(interval_ns: u64) -> io::Result<Tick> {
        // SAFETY: creating a timerfd; the fd is owned from here on.
        let fd = unsafe { libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_NONBLOCK | libc::TFD_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd is a fresh, valid timerfd.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut tick = Tick { fd, paced: false, period_ns: 0, slowed: false, calm: Calm::default() };
        tick.arm_periodic(interval_ns)?;
        Ok(tick)
    }

    pub fn raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    /// The period the timer repeats at, 0 while it is a one-shot (for the wake-up latency's due time, `wake::tick_fired`).
    pub fn period_ns(&self) -> u64 {
        if self.paced { 0 } else { self.period_ns }
    }

    /// Whether the timer is armed slowed: an event must put it back (`State` has nothing to say about that).
    pub fn is_slowed(&self) -> bool {
        self.slowed
    }

    /// Reads the number of expirations the timerfd owes (1 at least when it fired).
    pub fn read_expirations(&self) -> u64 {
        let mut buf = [0u8; 8];
        // SAFETY: reading the 8-byte expiration count the timerfd owes into a stack buffer.
        let n = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, 8) };
        if n == 8 { u64::from_ne_bytes(buf) } else { 0 }
    }

    /// Re-arms the tick as a one-shot `delay` from now.
    pub fn arm_once(&mut self, delay: Duration) -> io::Result<()> {
        let value = libc::timespec { tv_sec: delay.as_secs() as libc::time_t, tv_nsec: delay.subsec_nanos() as libc::c_long };
        let spec = libc::itimerspec { it_interval: libc::timespec { tv_sec: 0, tv_nsec: 0 }, it_value: value };
        self.settime(&spec)?;
        crate::wake::tick_armed(monotonic_ns() + delay.as_nanos() as u64);
        self.paced = true;
        Ok(())
    }

    /// Arms the tick as a repeating interval starting one interval from now.
    pub fn arm_periodic(&mut self, interval_ns: u64) -> io::Result<()> {
        let ts = libc::timespec { tv_sec: (interval_ns / 1_000_000_000) as libc::time_t, tv_nsec: (interval_ns % 1_000_000_000) as libc::c_long };
        let spec = libc::itimerspec { it_interval: ts, it_value: ts };
        self.settime(&spec)?;
        crate::wake::tick_armed(monotonic_ns() + interval_ns);
        self.paced = false;
        self.period_ns = interval_ns;
        Ok(())
    }

    fn settime(&self, spec: &libc::itimerspec) -> io::Result<()> {
        // SAFETY: arming a timerfd with a stack itimerspec.
        if unsafe { libc::timerfd_settime(self.fd.as_raw_fd(), 0, spec, std::ptr::null_mut()) } < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    /// After a frame, a flip or an event: the output's latch as a one-shot while it paces itself, else the fixed interval on the panel's grid
    /// where the output remembers it (a one-shot too), else periodic (re-armed only when the timer was last set as a one-shot or the period
    /// changes, so the periodic timer keeps its phase). In the calm described above the grid latch is one at least `idle_tick_ns` away and
    /// the periodic fallback has that period.
    pub fn rearm(&mut self, state: &State, tick_ns: u64) -> io::Result<()> {
        let now = Instant::now();
        let slow = Some(state.config.idle_tick_ns)
            .filter(|&ns| ns > 0 && self.calm.is_calm(monotonic_ns()) && !state.surfaces.any_undrawn() && state.output.idle_ok());
        let latch = state.output.next_frame_at(now).or_else(|| match slow {
            Some(ns) => state.output.idle_tick_after(now, Duration::from_nanos(ns)),
            None => state.output.idle_tick_at(now),
        });
        let latch = latch.map(|at| at.saturating_duration_since(Instant::now()));
        self.slowed = slow.is_some();
        match plan(self.paced, self.period_ns, tick_ns, slow, latch) {
            Plan::Once(d) => self.arm_once(d),
            Plan::Periodic(ns) => self.arm_periodic(ns),
            Plan::Keep => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFRESH: u64 = 16_666_667;

    #[test]
    fn a_calm_loop_has_had_no_event_for_a_quarter_second_and_no_key_for_three() {
        let mut c = Calm::default();
        let t0 = 10_000_000_000;
        c.event(t0, false);
        assert!(!c.is_calm(t0 + 100_000_000), "a client committed 100 ms ago");
        assert!(c.is_calm(t0 + CALM_AFTER_NS + 1), "nothing since");
        c.event(t0 + 1_000_000_000, true);
        assert!(!c.is_calm(t0 + 1_000_000_000 + CALM_AFTER_NS + 1), "a key was pressed: a hold is being measured");
        assert!(!c.is_calm(t0 + 1_000_000_000 + INPUT_CALM_AFTER_NS - 1));
        assert!(c.is_calm(t0 + 1_000_000_000 + INPUT_CALM_AFTER_NS));
        c.event(t0 + 5_000_000_000, false);
        assert!(!c.is_calm(t0 + 5_000_000_000 + 10), "any event ends it at once");
    }

    #[test]
    fn a_loop_that_has_seen_nothing_yet_is_not_held_back_by_the_clock_it_never_read() {
        assert!(Calm::default().is_calm(INPUT_CALM_AFTER_NS), "the epoch's first seconds are not a key's hold");
    }

    #[test]
    fn the_output_wanting_a_tick_is_a_one_shot_at_its_latch_never_shorter_than_half_a_millisecond() {
        let p = plan(false, REFRESH, REFRESH, None, Some(Duration::from_millis(9)));
        assert_eq!(p, Plan::Once(Duration::from_millis(9)));
        assert_eq!(plan(true, 0, REFRESH, None, Some(Duration::ZERO)), Plan::Once(MIN_ONCE), "a latch in the past does not spin the loop");
    }

    #[test]
    fn with_no_latch_the_fixed_tick_is_periodic_and_keeps_its_phase_once_it_is() {
        assert_eq!(plan(true, 0, REFRESH, None, None), Plan::Periodic(REFRESH), "back from a one-shot");
        assert_eq!(plan(false, REFRESH, REFRESH, None, None), Plan::Keep, "already periodic at the right rate");
    }

    #[test]
    fn the_slowed_period_replaces_the_panels_rate_and_the_event_that_ends_it_brings_the_rate_back() {
        assert_eq!(plan(false, REFRESH, REFRESH, Some(IDLE_TICK_NS), None), Plan::Periodic(IDLE_TICK_NS), "calm: ten a second");
        assert_eq!(plan(false, IDLE_TICK_NS, REFRESH, Some(IDLE_TICK_NS), None), Plan::Keep, "and left alone while it holds");
        assert_eq!(plan(false, IDLE_TICK_NS, REFRESH, None, None), Plan::Periodic(REFRESH), "an event: the panel's rate again at once");
    }

    #[test]
    fn a_real_timer_fires_once_for_a_one_shot_and_repeats_for_a_periodic() {
        let mut t = Tick::new(50_000_000).unwrap();
        assert_eq!(t.period_ns(), 50_000_000);
        t.arm_once(Duration::from_millis(5)).unwrap();
        assert_eq!(t.period_ns(), 0, "a one-shot has no period");
        let mut p = libc::pollfd { fd: t.raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd on the timer's own descriptor.
        assert_eq!(unsafe { libc::poll(&mut p, 1, 500) }, 1);
        assert_eq!(t.read_expirations(), 1);
        // SAFETY: as above.
        assert_eq!(unsafe { libc::poll(&mut p, 1, 30) }, 0, "a one-shot is spent");
        t.arm_periodic(10_000_000).unwrap();
        std::thread::sleep(Duration::from_millis(55));
        assert!(t.read_expirations() >= 3, "a 10 ms timer that was not read for 55 ms owes several");
        assert_eq!(t.read_expirations(), 0, "and nothing once read");
    }
}
