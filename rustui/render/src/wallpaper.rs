//! Wallpapers: how they keep time, and the taps on a `Liquid` one. The GPU side is in `wallpapers`.
//!
//! A wallpaper moves on the compositor's clock, at the rate its layer asks for and no faster. Its *phase* is its own
//! stopwatch of motion, in seconds of the pace it was designed at: every step adds the time since the last step times
//! the layer's `speed`, so changing the speed changes how fast it goes on from where it is, never where it is. It
//! steps every `N` refreshes, `N` being the panel's rate over the asked-for one rounded to a whole number, because a
//! step every so many milliseconds that is not a whole number of refreshes would beat against the panel and show as a
//! stutter every second or so. Between steps the picture is the same: a wallpaper at 30 fps on a 60 Hz panel is drawn
//! by every second frame, and costs no frame at all when nothing else moves and it is asked to be still.

/// How many taps on a `Liquid` wallpaper are alive at once; a new one replaces the oldest.
pub const MAX_RIPPLES: usize = 6;

/// How long a ripple lasts, in seconds of the wallpaper's own time.
pub const RIPPLE_LIFE_S: f64 = 2.5;

/// A step never accounts for more than this many intervals of time: after a stall the wallpaper carries on from about
/// where it was rather than jumping.
const MAX_STEPS_PER_STEP: u64 = 4;

/// The time between two steps of a wallpaper asked to move at `fps` frames a second on a panel that refreshes every
/// `refresh_period_ns`: a whole number of refreshes, at least one. `None` for a still wallpaper.
pub fn step_interval_ns(fps: u8, refresh_period_ns: u64) -> Option<u64> {
    if fps == 0 {
        return None;
    }
    let refreshes = (1.0e9 / (f64::from(fps) * refresh_period_ns as f64))
        .round()
        .max(1.0);
    Some(refreshes as u64 * refresh_period_ns)
}

/// A tap, as the renderer is told of it: where, and how old it is in seconds of the wallpaper's time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ripple {
    pub x: f32,
    pub y: f32,
    pub age_s: f32,
}

/// The live taps, as a small copyable array.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ripples {
    pub count: usize,
    pub items: [Ripple; MAX_RIPPLES],
}

#[cfg(test)]
impl Ripples {
    /// The taps that are alive, oldest first.
    pub fn live(&self) -> &[Ripple] {
        &self.items[..self.count]
    }
}

/// A wallpaper's time: its phase, and when it last stepped. `None` for the time it was last shown, so that a
/// wallpaper that was hidden (or still) carries on from where it stopped instead of from what the clock says.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Clock {
    pub phase: f64,
    stamp: Option<u64>,
}

impl Clock {
    /// Brings the clock to the frame time `slot_ns`. `interval_ns` is the time between steps, `None` when still;
    /// `speed` scales the phase (a mesh turning the other way has a negative one); `shown` is whether the layer is on
    /// screen. Returns whether it stepped.
    pub fn advance(
        &mut self,
        slot_ns: u64,
        shown: bool,
        interval_ns: Option<u64>,
        speed: f32,
        half_refresh_ns: u64,
    ) -> bool {
        let Some(interval) = interval_ns.filter(|_| shown && speed != 0.0) else {
            self.stamp = None;
            return false;
        };
        match self.stamp {
            None => {
                self.stamp = Some(slot_ns);
                false
            }
            Some(stamp) if slot_ns + half_refresh_ns >= stamp + interval => {
                let elapsed = slot_ns
                    .saturating_sub(stamp)
                    .min(MAX_STEPS_PER_STEP * interval);
                self.phase += elapsed as f64 / 1.0e9 * f64::from(speed);
                self.stamp = Some(slot_ns);
                true
            }
            Some(_) => false,
        }
    }

    /// The frame time at which the next step is due, if the clock is running.
    pub fn next_step(&self, interval_ns: Option<u64>) -> Option<u64> {
        Some(self.stamp? + interval_ns?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERIOD: u64 = 16_767_000; // 59.64 Hz
    const HALF: u64 = PERIOD / 2;

    #[test]
    fn a_wallpaper_steps_every_whole_number_of_refreshes_nearest_the_rate_asked_for() {
        let at = |fps| step_interval_ns(fps, PERIOD).map(|ns| ns / PERIOD);
        assert_eq!(at(60), Some(1));
        assert_eq!(at(50), Some(1));
        assert_eq!(at(45), Some(1));
        assert_eq!(at(30), Some(2));
        assert_eq!(at(25), Some(2));
        assert_eq!(at(20), Some(3));
        assert_eq!(at(15), Some(4));
        assert_eq!(at(1), Some(60));
        // Never faster than the panel, and a still wallpaper has no interval at all.
        assert_eq!(at(255), Some(1));
        assert_eq!(step_interval_ns(0, PERIOD), None);
        // Always a whole number of refreshes: no beating against the panel.
        for fps in 1..=60 {
            assert_eq!(step_interval_ns(fps, PERIOD).unwrap() % PERIOD, 0, "{fps}");
        }
    }

    #[test]
    fn the_faster_panel_steps_the_same_rates_on_its_own_whole_refreshes() {
        // The 62.26 Hz panel (16.062 ms): the rates that were one, two, three and four refreshes are still those.
        const BOE: u64 = 16_061_677;
        let at = |fps| step_interval_ns(fps, BOE).map(|ns| ns / BOE);
        assert_eq!(at(60), Some(1));
        assert_eq!(at(45), Some(1));
        assert_eq!(at(30), Some(2));
        assert_eq!(at(25), Some(2));
        assert_eq!(at(20), Some(3));
        assert_eq!(at(15), Some(4));
        for fps in 1..=62 {
            assert_eq!(step_interval_ns(fps, BOE).unwrap() % BOE, 0, "{fps}");
        }
    }

    #[test]
    fn the_first_frame_only_starts_the_clock_and_after_that_it_steps_on_time() {
        let interval = step_interval_ns(30, PERIOD);
        let mut clock = Clock::default();
        let mut slot = 1_000_000_000u64;
        assert!(!clock.advance(slot, true, interval, 1.0, HALF));
        assert_eq!(clock.phase, 0.0);
        // Frames every refresh: it steps every second one, by exactly the time that passed.
        let mut steps = Vec::new();
        for frame in 1..=6 {
            slot += PERIOD;
            steps.push((frame, clock.advance(slot, true, interval, 1.0, HALF)));
        }
        assert_eq!(
            steps,
            vec![
                (1, false),
                (2, true),
                (3, false),
                (4, true),
                (5, false),
                (6, true)
            ]
        );
        assert!((clock.phase - 6.0 * PERIOD as f64 / 1e9).abs() < 1e-9);
    }

    #[test]
    fn a_frame_a_little_early_still_steps_but_one_a_whole_refresh_early_does_not() {
        let interval = step_interval_ns(30, PERIOD);
        let mut clock = Clock::default();
        clock.advance(0, true, interval, 1.0, HALF);
        assert!(!clock.advance(PERIOD, true, interval, 1.0, HALF));
        assert!(clock.advance(2 * PERIOD - HALF + 1, true, interval, 1.0, HALF));
        let mut clock = Clock::default();
        clock.advance(0, true, interval, 1.0, HALF);
        assert!(!clock.advance(2 * PERIOD - HALF - 1, true, interval, 1.0, HALF));
    }

    #[test]
    fn a_still_wallpaper_has_no_next_step_and_never_moves() {
        let mut clock = Clock::default();
        for slot in 0..5 {
            assert!(!clock.advance(slot * PERIOD, true, None, 1.0, HALF));
        }
        assert_eq!((clock.phase, clock.next_step(None)), (0.0, None));
        // Nor does one with no speed, or one that is not shown.
        let interval = step_interval_ns(30, PERIOD);
        for (shown, speed) in [(true, 0.0), (false, 1.0)] {
            let mut clock = Clock::default();
            clock.advance(0, shown, interval, speed, HALF);
            assert!(!clock.advance(10 * PERIOD, shown, interval, speed, HALF));
            assert_eq!(clock.next_step(interval), None);
        }
    }

    #[test]
    fn the_next_step_is_one_interval_after_the_last() {
        let interval = step_interval_ns(30, PERIOD);
        let mut clock = Clock::default();
        assert_eq!(clock.next_step(interval), None, "not started");
        clock.advance(5_000, true, interval, 1.0, HALF);
        assert_eq!(clock.next_step(interval), Some(5_000 + 2 * PERIOD));
        clock.advance(5_000 + 2 * PERIOD, true, interval, 1.0, HALF);
        assert_eq!(clock.next_step(interval), Some(5_000 + 4 * PERIOD));
    }

    #[test]
    fn the_speed_scales_how_far_a_step_goes_and_a_change_of_it_never_jumps() {
        let interval = step_interval_ns(30, PERIOD);
        let mut clock = Clock::default();
        clock.advance(0, true, interval, 2.0, HALF);
        clock.advance(2 * PERIOD, true, interval, 2.0, HALF);
        let fast = clock.phase;
        assert!((fast - 2.0 * (2 * PERIOD) as f64 / 1e9).abs() < 1e-9);
        // Halving the speed does not move the phase back or forward on its own; the next step goes half as far.
        clock.advance(4 * PERIOD, true, interval, 1.0, HALF);
        assert!((clock.phase - fast - (2 * PERIOD) as f64 / 1e9).abs() < 1e-9);
    }

    #[test]
    fn hiding_a_wallpaper_stops_its_time_and_showing_it_carries_on_from_where_it_was() {
        let interval = step_interval_ns(30, PERIOD);
        let mut clock = Clock::default();
        clock.advance(0, true, interval, 1.0, HALF);
        clock.advance(2 * PERIOD, true, interval, 1.0, HALF);
        let before = clock.phase;
        // Hidden for a minute.
        clock.advance(3 * PERIOD, false, interval, 1.0, HALF);
        let minute = 60_000_000_000u64;
        // Shown again: the first frame restarts the clock and the phase has not moved.
        assert!(!clock.advance(minute, true, interval, 1.0, HALF));
        assert_eq!(clock.phase, before);
        assert!(clock.advance(minute + 2 * PERIOD, true, interval, 1.0, HALF));
        assert!((clock.phase - before - (2 * PERIOD) as f64 / 1e9).abs() < 1e-9);
    }

    #[test]
    fn a_stall_does_not_make_the_wallpaper_jump() {
        let interval = step_interval_ns(30, PERIOD);
        let mut clock = Clock::default();
        clock.advance(0, true, interval, 1.0, HALF);
        // Frames stopped for three seconds (the compositor was starved), then one came.
        assert!(clock.advance(3_000_000_000, true, interval, 1.0, HALF));
        let cap = (MAX_STEPS_PER_STEP * interval.unwrap()) as f64 / 1e9;
        assert!((clock.phase - cap).abs() < 1e-9, "{} vs {cap}", clock.phase);
    }
}
