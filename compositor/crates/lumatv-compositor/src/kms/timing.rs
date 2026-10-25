//! The flip-timing bookkeeping of the KMS output, apart from the device: the pacer fed with the page-flip stamps, the odd
//! stamps, and the refresh period followed from the stamps themselves.
//!
//! The panel's rate follows the content: a browser page put the output at 50 Hz for as long as it was in front
//! (`perf-tweaks.md`), and 24 Hz film is the other case. The mode's `vrefresh` says 60 whatever is showing, so a pacer keyed
//! to it would plan a slot every 16.7 ms under a 41.7 ms refresh: every frame "late", frames drawn for refreshes that do
//! not exist and replaced before they go out. Here the period is what consecutive flips measure, and the pacer is keyed
//! again when it changes, so the UI draws one frame per refresh that exists. Its animations are evaluated at wall time,
//! so a 300 ms slide takes 300 ms at 24 Hz as at 60: the UI's clock is its own; only its frame rate is the panel's.

use crate::pacer::{Pacer, Shown};

/// Rates the panel is known or likely to run at, as periods in ns: a measured interval is snapped to the nearest.
const RATES_HZ: [f64; 10] = [23.976, 24.0, 25.0, 29.97, 30.0, 48.0, 50.0, 59.94, 60.0, 120.0];
/// Consecutive flips that must agree on a new period before the pacer is keyed to it.
const AGREE: usize = 4;
/// How far an interval may sit from a period (or a multiple of it) and still be taken as that period: the stamps on the
/// set are within 0.05 ms of a refresh, so this is for the clocks, not the jitter.
const TOLERANCE: f64 = 0.05;

/// When a frame was planned for, started and ready: kept with the frame from its draw to its flip, since the next frame
/// is drawn while this one's flip is pending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub slot_ns: u64,
    pub started_ns: u64,
    /// When the draw was submitted; the flip replaces it with the GPU's finish (the in-fence's signal time).
    pub ready_ns: u64,
}

/// What one flip taught.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Landed {
    /// The stamp was not one refresh after the last (a pair, or a refresh skipped): the interval in ns.
    pub odd: Option<u64>,
    /// The refresh period changed: `(old, new)` in ns, and the pacer was keyed again.
    pub rate_change: Option<(u64, u64)>,
    /// The frame's landing, when the flip was of one the output was waiting for.
    pub late: i32,
    pub lost: u32,
}

pub struct FlipTiming {
    pub pacer: Pacer,
    period_ns: u64,
    /// When the last flip landed, 0 before the first.
    last_flip_ns: u64,
    /// The last few intervals between consecutive flips.
    recent: [u64; AGREE],
    filled: usize,
    pub flips: u64,
    pub odd_stamps: u64,
    pub late_frames: u64,
    pub lost_refreshes: u64,
    pub rate_changes: u64,
}

impl FlipTiming {
    pub fn new(period_ns: u64) -> FlipTiming {
        FlipTiming {
            pacer: Pacer::new(period_ns),
            period_ns,
            last_flip_ns: 0,
            recent: [0; AGREE],
            filled: 0,
            flips: 0,
            odd_stamps: 0,
            late_frames: 0,
            lost_refreshes: 0,
            rate_changes: 0,
        }
    }

    /// The refresh period the pacer is keyed to, in ns.
    pub fn period_ns(&self) -> u64 {
        self.period_ns
    }

    /// A flip landed at `presented_ns` (CLOCK_MONOTONIC). `frame` is the timing of the frame it was of, with the GPU's
    /// finish when the in-fence said; `None` for a flip nothing was waiting for.
    pub fn landed(&mut self, presented_ns: u64, frame: Option<(Timing, Option<u64>)>) -> Landed {
        self.flips += 1;
        let mut out = Landed::default();
        if self.last_flip_ns != 0 {
            let since = presented_ns.saturating_sub(self.last_flip_ns);
            self.recent[self.filled % AGREE] = since;
            self.filled += 1;
            if let Some(new) = self.rate_from_recent() {
                out.rate_change = Some((self.period_ns, new));
                self.period_ns = new;
                self.pacer = Pacer::new(new);
                self.rate_changes += 1;
                self.filled = 0;
            } else if since < self.period_ns * 3 / 4 || (since > self.period_ns * 5 / 4 && since < self.period_ns * 11 / 4) {
                self.odd_stamps += 1;
                out.odd = Some(since);
            }
        }
        self.last_flip_ns = presented_ns;
        if let Some((mut timing, signalled)) = frame {
            // The pipeline the pacer learns ends when the GPU finished the page, not when the draw was submitted: the
            // kernel waits on the in-fence before the flip, so a page the GPU finishes after the driver's cutoff slips a
            // refresh that a CPU-only pipeline never shows (16 gaps of 20-33 ms in the second gate's x1).
            if let Some(s) = signalled
                && s > timing.ready_ns
                && s <= presented_ns
            {
                timing.ready_ns = s;
            }
            let shown = Shown { slot_ns: timing.slot_ns, started_ns: timing.started_ns, ready_ns: timing.ready_ns, presented_ns, content_ns: None };
            let landing = self.pacer.presented(&shown);
            if landing.late > 0 {
                self.late_frames += 1;
            }
            self.lost_refreshes += landing.lost as u64;
            out.late = landing.late;
            out.lost = landing.lost;
        }
        out
    }

    /// A new period when the last `AGREE` intervals all name the same known rate, one that is not the current period
    /// or a whole number of it (a skipped refresh is not a slower panel).
    fn rate_from_recent(&self) -> Option<u64> {
        if self.filled < AGREE {
            return None;
        }
        let first = snap(self.recent[0])?;
        if self.recent.iter().any(|&i| snap(i) != Some(first)) {
            return None;
        }
        if (1..=4).any(|k| near_multiple(first, self.period_ns, k)) {
            return None;
        }
        Some(first)
    }
}

/// Whether `interval` is within the tolerance of `k` periods.
fn near_multiple(interval: u64, period_ns: u64, k: u64) -> bool {
    let target = (period_ns * k) as f64;
    ((interval as f64) - target).abs() <= target * TOLERANCE
}

/// The known rate whose period is nearest `interval`, if it is within the tolerance.
fn snap(interval: u64) -> Option<u64> {
    RATES_HZ.iter().map(|hz| (1e9 / hz) as u64).min_by_key(|&p| p.abs_diff(interval)).filter(|&p| near_multiple(interval, p, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const P60: u64 = (1e9 / 60.0) as u64;
    const P50: u64 = (1e9 / 50.0) as u64;
    const P24: u64 = (1e9 / 24.0) as u64;

    fn timing(slot: u64) -> Option<(Timing, Option<u64>)> {
        Some((Timing { slot_ns: slot, started_ns: slot.saturating_sub(8_000_000), ready_ns: slot.saturating_sub(4_000_000) }, None))
    }

    /// Flips every `period` from `from`, `n` of them, each of a frame planned for its own refresh.
    fn run(t: &mut FlipTiming, from: u64, period: u64, n: usize) -> Vec<Landed> {
        (0..n).map(|i| t.landed(from + i as u64 * period, timing(from + i as u64 * period))).collect()
    }

    #[test]
    fn steady_sixty_hz_is_neither_odd_nor_a_rate_change() {
        let mut t = FlipTiming::new(P60);
        let out = run(&mut t, 1_000_000_000, P60, 20);
        assert!(out.iter().all(|l| l.odd.is_none() && l.rate_change.is_none()), "{out:?}");
        assert_eq!(t.period_ns(), P60);
        assert_eq!(t.flips, 20);
    }

    #[test]
    fn a_skipped_refresh_is_odd_and_not_a_slower_panel() {
        let mut t = FlipTiming::new(P60);
        let base = 1_000_000_000;
        run(&mut t, base, P60, 6);
        let next = base + 5 * P60;
        // four frames landing every other refresh: 33.3 ms apart, as a 30 Hz panel would be
        let out = run(&mut t, next + 2 * P60, 2 * P60, 4);
        assert!(out.iter().all(|l| l.odd.is_some()), "{out:?}");
        assert!(out.iter().all(|l| l.rate_change.is_none()), "{out:?}");
        assert_eq!(t.period_ns(), P60);
        assert_eq!(t.odd_stamps, 4);
    }

    #[test]
    fn four_flips_at_fifty_hz_key_the_pacer_to_fifty() {
        let mut t = FlipTiming::new(P60);
        let base = 1_000_000_000;
        run(&mut t, base, P60, 6);
        let start = base + 5 * P60 + P50;
        let out = run(&mut t, start, P50, 6);
        let change = out.iter().position(|l| l.rate_change.is_some()).expect("a rate change");
        assert_eq!(out[change].rate_change, Some((P60, P50)));
        assert_eq!(change, 3, "the fourth interval at 50 Hz settles it: {out:?}");
        assert!(out[..change].iter().all(|l| l.odd.is_none()), "20 ms at 60 Hz is not a pair or a skip: {out:?}");
        assert!(out[change + 1..].iter().all(|l| l.odd.is_none() && l.rate_change.is_none()), "{out:?}");
        assert_eq!(t.period_ns(), P50);
        assert_eq!(t.rate_changes, 1);
    }

    #[test]
    fn twenty_four_hz_film_and_back_to_sixty() {
        let mut t = FlipTiming::new(P60);
        let base = 1_000_000_000;
        run(&mut t, base, P60, 4);
        let start = base + 3 * P60 + P24;
        let out = run(&mut t, start, P24, 8);
        assert_eq!(out.iter().filter_map(|l| l.rate_change).collect::<Vec<_>>(), [(P60, P24)]);
        assert_eq!(t.period_ns(), P24);
        // The pacer keyed to 24 Hz: frames planned for consecutive 24 Hz slots land on time.
        let late_before = t.late_frames;
        let from = start + 8 * P24;
        let out = run(&mut t, from, P24, 8);
        assert!(out.iter().all(|l| l.odd.is_none() && l.rate_change.is_none()), "{out:?}");
        assert_eq!(t.late_frames, late_before, "nothing late once the period is the panel's");
        // Film ends: 60 Hz again.
        let from = from + 8 * P24;
        let out = run(&mut t, from, P60, 8);
        assert_eq!(out.iter().filter_map(|l| l.rate_change).collect::<Vec<_>>(), [(P24, P60)]);
        assert_eq!(t.period_ns(), P60);
    }

    #[test]
    fn a_flip_nothing_waited_for_teaches_the_interval_only() {
        let mut t = FlipTiming::new(P60);
        t.landed(5_000_000_000, None);
        let l = t.landed(5_000_000_000 + P60, None);
        assert_eq!(l, Landed::default());
        assert_eq!(t.flips, 2);
        assert_eq!(t.late_frames, 0);
    }

    #[test]
    fn intervals_snap_to_known_rates_within_tolerance() {
        assert_eq!(snap(20_300_000), Some(P50));
        assert_eq!(snap(41_000_000), Some((1e9 / 24.0) as u64));
        assert_eq!(snap(27_000_000), None);
        assert!(near_multiple(2 * P60 + 400_000, P60, 2));
        assert!(!near_multiple(P50, P60, 1));
    }
}
