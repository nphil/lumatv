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
use crate::refresh::Measure;

/// Rates the panel is known or likely to run at, as periods in ns: a measured interval is snapped to the nearest. 100 and 120 are
/// the engine's own rates with the OSD's vblank divider at 1 (`refresh`): 100 under 50-family content, 120 otherwise.
const RATES_HZ: [f64; 11] = [23.976, 24.0, 25.0, 29.97, 30.0, 48.0, 50.0, 59.94, 60.0, 100.0, 120.0];
/// Consecutive flips that must agree on a new period before the pacer is keyed to it.
const AGREE: usize = 4;
/// How far an interval may sit from a period (or a multiple of it) and still be taken as that period: the stamps on the
/// set are within 0.05 ms of a refresh, so this is for the clocks, not the jitter.
const TOLERANCE: f64 = 0.05;
/// What a flip needs before its refresh to land on it (the pacer's own lead).
const FLIP_LEAD_NS: u64 = 500_000;

/// When a frame was planned for, started and ready: kept with the frame from its draw to its flip, since the next frame
/// is drawn while this one's flip is pending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub slot_ns: u64,
    pub started_ns: u64,
    /// When the draw was submitted; the flip replaces it with the GPU's finish (the in-fence's signal time).
    pub ready_ns: u64,
    /// When the oldest commit the frame shows first arrived (`Plan::content_ns`).
    pub content_ns: Option<u64>,
}

impl Timing {
    /// Moves the slot by `shift_ns` (a [`Landed::shift_ns`]): a frame planned while an older one was on its way is
    /// planned a refresh after it, and when that one lands a refresh late the timeline moves on by one, this frame's
    /// expected refresh with it. Judged against its old slot it would be late once more when it lands a refresh after
    /// the late one, and the timeline would move on twice: on the tenth gate's x1 runs the frame after that was aimed a
    /// refresh too far (a `page-flip stamp 33.37 ms` at X1's mapping, the second of two).
    pub fn shift(&mut self, shift_ns: i64) {
        self.slot_ns = (self.slot_ns as i64 + shift_ns).max(0) as u64;
    }
}

/// What one flip taught.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Landed {
    /// The stamp was not one refresh after the last (a pair, or a refresh skipped): the interval in ns.
    pub odd: Option<u64>,
    /// The refresh the odd stamp skipped was one this frame was due at: its content was in early enough for a frame
    /// that costs what this one did, started at once, to make it. A skip with nothing due is a screen with nothing new
    /// for one refresh (a window made and redrawn, then the first frame of its client), or a frame on a GPU asleep
    /// (10-11 ms against 4.7 in a run) whose commit came too late for it: not a frame the panel missed.
    pub missed: bool,
    /// The refresh period changed: `(old, new)` in ns, and the pacer was keyed again.
    pub rate_change: Option<(u64, u64)>,
    /// The frame's landing, when the flip was of one the output was waiting for.
    pub late: i32,
    pub lost: u32,
    /// How far the pacer's timeline moved with this landing (`late` refreshes), for the frames planned before it landed
    /// ([`Timing::shift`]).
    pub shift_ns: i64,
}

pub struct FlipTiming {
    pub pacer: Pacer,
    period_ns: u64,
    /// When the last flip landed, 0 before the first.
    last_flip_ns: u64,
    /// When the first flip landed, 0 before it: with the count, the refresh period as the panel keeps it.
    first_flip_ns: u64,
    /// The last few intervals between consecutive flips.
    recent: [u64; AGREE],
    filled: usize,
    pub flips: u64,
    pub odd_stamps: u64,
    /// Odd stamps whose skipped refresh had a frame due (`Landed::missed`).
    pub missed_stamps: u64,
    pub late_frames: u64,
    pub lost_refreshes: u64,
    pub rate_changes: u64,
    /// What the flips said since the last `rekey` (`ctl refresh`), or the start.
    pub measure: Measure,
}

impl FlipTiming {
    pub fn new(period_ns: u64) -> FlipTiming {
        FlipTiming {
            pacer: Pacer::new(period_ns),
            period_ns,
            last_flip_ns: 0,
            first_flip_ns: 0,
            recent: [0; AGREE],
            filled: 0,
            flips: 0,
            odd_stamps: 0,
            missed_stamps: 0,
            late_frames: 0,
            lost_refreshes: 0,
            rate_changes: 0,
            measure: Measure::default(),
        }
    }

    /// The refresh period the pacer is keyed to, in ns.
    pub fn period_ns(&self) -> u64 {
        self.period_ns
    }

    /// Keys the pacer to `period_ns` by order (`ctl refresh`), forgets the intervals seen and starts the measure over. The measured
    /// path cannot do this for 120 to 60: a period that is a whole number of the current one is a skipped refresh to it, not a slower
    /// panel (`rate_from_recent`).
    pub fn rekey(&mut self, period_ns: u64) {
        if period_ns != self.period_ns {
            self.period_ns = period_ns;
            self.pacer = Pacer::new(period_ns);
            self.rate_changes += 1;
        }
        self.filled = 0;
        self.measure = Measure::default();
    }

    /// When the last flip landed (CLOCK_MONOTONIC ns), 0 before the first.
    pub fn last_flip_ns(&self) -> u64 {
        self.last_flip_ns
    }

    /// The latch of the next refresh on the panel's grid: the first refresh, counted from the last flip's stamp in
    /// whole periods, whose latch (the refresh less `budget_ns`) is after `now_ns`. `None` before any flip, or once the
    /// last one is older than `memory_ns` (the grid drifts against our clock).
    pub fn grid_latch_ns(&self, now_ns: u64, budget_ns: u64, memory_ns: u64) -> Option<u64> {
        let last = self.last_flip_ns;
        if last == 0 || now_ns < last || now_ns - last > memory_ns {
            return None;
        }
        let period = self.period_ns.max(1);
        let refreshes = (now_ns + budget_ns - last) / period + 1;
        Some(last + refreshes * period - budget_ns)
    }

    /// The mean interval between the first and the last flip, in ns: the panel's refresh period on CLOCK_MONOTONIC,
    /// against the mode's nominal one. `None` before the second flip.
    pub fn mean_period_ns(&self) -> Option<f64> {
        (self.flips >= 2 && self.first_flip_ns != 0).then(|| (self.last_flip_ns - self.first_flip_ns) as f64 / (self.flips - 1) as f64)
    }

    /// A flip landed at `presented_ns` (CLOCK_MONOTONIC). `frame` is the timing of the frame it was of, with the GPU's
    /// finish when the in-fence said; `None` for a flip nothing was waiting for.
    pub fn landed(&mut self, presented_ns: u64, frame: Option<(Timing, Option<u64>)>) -> Landed {
        self.flips += 1;
        let mut out = Landed::default();
        let last = self.last_flip_ns;
        let mut skipped = false;
        let mut interval = 0;
        if last != 0 {
            let since = presented_ns.saturating_sub(last);
            interval = since;
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
                skipped = since > self.period_ns;
            }
        }
        self.measure.flip(presented_ns, interval, self.period_ns);
        self.last_flip_ns = presented_ns;
        if self.first_flip_ns == 0 {
            self.first_flip_ns = presented_ns;
        }
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
            // The refresh after `last` could have shown this frame if its content was in its own pipeline's time before
            // it (what a frame cost at that moment, the GPU's wake-up included: the pacer's `lost` reads it the same
            // way); a frame of no new commit (a layout request) is taken as due since `last`.
            if skipped {
                let earliest = timing.content_ns.map_or(last, |c| c.max(last));
                let pipeline = timing.ready_ns.saturating_sub(timing.started_ns);
                out.missed = earliest + pipeline + FLIP_LEAD_NS <= last + self.period_ns;
                if out.missed {
                    self.missed_stamps += 1;
                }
            }
            let shown =
                Shown { slot_ns: timing.slot_ns, started_ns: timing.started_ns, ready_ns: timing.ready_ns, presented_ns, content_ns: timing.content_ns };
            let landing = self.pacer.presented(&shown);
            if landing.late > 0 {
                self.late_frames += 1;
            }
            self.lost_refreshes += landing.lost as u64;
            out.late = landing.late;
            out.lost = landing.lost;
            out.shift_ns = landing.late as i64 * self.period_ns as i64;
            self.measure.frame(timing.ready_ns.saturating_sub(timing.started_ns), landing.late, landing.lost, out.missed);
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
        with_content(slot, None)
    }

    /// A frame planned for the refresh at `slot`, 4 ms from start to ready, showing a commit that arrived at `content`.
    fn with_content(slot: u64, content: Option<u64>) -> Option<(Timing, Option<u64>)> {
        Some((Timing { slot_ns: slot, started_ns: slot.saturating_sub(8_000_000), ready_ns: slot.saturating_sub(4_000_000), content_ns: content }, None))
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

    const P120: u64 = (1e9 / 120.0) as u64;

    /// Flips every `period` from `from`, `n` of them, each of a frame with a `pipeline` from start to ready.
    fn run_costing(t: &mut FlipTiming, from: u64, period: u64, n: usize, pipeline: u64) -> Vec<Landed> {
        (0..n)
            .map(|i| {
                let slot = from + i as u64 * period;
                t.landed(slot, Some((Timing { slot_ns: slot, started_ns: slot - pipeline - 500_000, ready_ns: slot - 500_000, content_ns: None }, None)))
            })
            .collect()
    }

    #[test]
    fn flips_at_one_hundred_and_twenty_hz_key_the_pacer_to_it_by_themselves() {
        // The divider written to 1 with the pacer still keyed to 60: the first four 8.33 ms intervals look like pairs, the fourth settles it.
        let mut t = FlipTiming::new(P60);
        let base = 1_000_000_000;
        run(&mut t, base, P60, 4);
        let start = base + 3 * P60 + P120;
        let out = run_costing(&mut t, start, P120, 12, 4_000_000);
        let change = out.iter().position(|l| l.rate_change.is_some()).expect("a rate change");
        assert_eq!(out[change].rate_change, Some((P60, P120)));
        assert_eq!(change, 3);
        assert_eq!(t.period_ns(), P120);
        let late_before = t.late_frames;
        let out = run_costing(&mut t, start + 12 * P120, P120, 12, 4_000_000);
        assert!(out.iter().all(|l| l.odd.is_none() && l.rate_change.is_none()), "{out:?}");
        assert_eq!(t.late_frames, late_before, "4 ms frames land on every 8.33 ms refresh");
    }

    #[test]
    fn fifty_family_content_at_one_hundred_hz_is_followed_and_left_again() {
        // The divider at 1 and a 50 fps video in front: the engine runs 100 Hz and so do the flips; the pacer follows, and back.
        const P100: u64 = 10_000_000;
        let mut t = FlipTiming::new(P120);
        let base = 1_000_000_000;
        run_costing(&mut t, base, P120, 6, 4_000_000);
        let start = base + 5 * P120 + P100;
        let out = run_costing(&mut t, start, P100, 8, 4_000_000);
        assert_eq!(out.iter().filter_map(|l| l.rate_change).collect::<Vec<_>>(), [(P120, P100)]);
        assert_eq!(t.period_ns(), P100);
        let from = start + 8 * P100;
        let out = run_costing(&mut t, from, P120, 8, 4_000_000);
        assert_eq!(out.iter().filter_map(|l| l.rate_change).collect::<Vec<_>>(), [(P100, P120)]);
    }

    #[test]
    fn rekey_takes_the_pacer_back_to_sixty_where_the_measured_path_cannot() {
        let mut t = FlipTiming::new(P120);
        let base = 1_000_000_000;
        run_costing(&mut t, base, P120, 8, 4_000_000);
        // The divider back at 2: 16.67 ms intervals are two 120 Hz refreshes to the measured path, never a 60 Hz panel.
        let out = run_costing(&mut t, base + 7 * P120 + P60, P60, 8, 4_000_000);
        assert!(out.iter().all(|l| l.rate_change.is_none()), "{out:?}");
        assert_eq!(t.period_ns(), P120);
        assert!(out.iter().all(|l| l.odd.is_some()), "every one a skipped refresh at 120: {out:?}");
        t.rekey(P60);
        assert_eq!(t.period_ns(), P60);
        assert_eq!(t.measure.flips, 0, "the measure starts over");
        let from = base + 7 * P120 + 9 * P60;
        let out = run_costing(&mut t, from, P60, 8, 4_000_000);
        assert!(out.iter().all(|l| l.odd.is_none() && l.rate_change.is_none()), "{out:?}");
        assert_eq!(t.measure.flips, 8);
        assert_eq!(t.measure.one, 8, "the first flip of the measure is one refresh after the last of the old one");
        // rekey to the period in force changes nothing but the measure
        let changes = t.rate_changes;
        t.rekey(P60);
        assert_eq!((t.rate_changes, t.measure.flips), (changes, 0));
    }

    #[test]
    fn the_measure_at_one_hundred_and_twenty_reads_a_frame_that_costs_a_sixty_hz_budget() {
        let mut t = FlipTiming::new(P120);
        let base = 1_000_000_000;
        run_costing(&mut t, base, P120, 4, 4_000_000);
        // The shell's 15 ms frames: each planned for the refresh after the last landing, each landing two refreshes after it, so the
        // pacer finds every one a refresh late and the flips two refreshes apart.
        let mut last = base + 3 * P120;
        let mut out = Vec::new();
        for _ in 0..6 {
            let slot = last + P120;
            let presented = last + 2 * P120;
            out.push(t.landed(presented, Some((Timing { slot_ns: slot, started_ns: last, ready_ns: last + 15_000_000, content_ns: None }, None))));
            last = presented;
        }
        assert!(out.iter().all(|l| l.odd.is_some() && l.late == 1 && !l.missed), "{out:?}");
        let m = &t.measure;
        assert_eq!(m.flips, 10);
        assert_eq!((m.one, m.two, m.three, m.more), (3, 6, 0, 0));
        assert_eq!((m.frames, m.late, m.lost, m.missed), (10, 6, 0, 0));
        let r = m.report(t.period_ns());
        assert_eq!(r["period_ms"].as_f64().unwrap().round(), 8.0);
        assert_eq!(r["pipeline_ms"]["max"], 15.0);
        assert_eq!(r["pipeline_ms"]["p50"], 15.0);
        assert_eq!(r["intervals"]["worst_ms"].as_f64().unwrap().round(), 17.0);
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

    #[test]
    fn a_skip_with_the_content_in_time_is_a_missed_refresh() {
        let mut t = FlipTiming::new(P60);
        let base = 1_000_000_000;
        run(&mut t, base, P60, 6);
        let last = base + 5 * P60;
        // In 1 ms after the last flip, on screen two refreshes later: a typical 4 ms frame could have made the one between.
        let l = t.landed(last + 2 * P60, with_content(last + 2 * P60, Some(last + 1_000_000)));
        assert!(l.odd.is_some() && l.missed, "{l:?}");
        // No new commit (a layout request): judged due since the last flip.
        let last = last + 2 * P60;
        let l = t.landed(last + 2 * P60, with_content(last + 2 * P60, None));
        assert!(l.odd.is_some() && l.missed, "{l:?}");
        assert_eq!((t.odd_stamps, t.missed_stamps), (2, 2));
    }

    #[test]
    fn a_skip_whose_content_came_too_late_for_the_refresh_between_is_not_missed() {
        // X1's first frame on every gate: a redraw of the same picture flipped, X1's first commit came 15 ms later, 1.7 ms
        // before the next refresh, and its frame landed on the one after: 33.37 ms between the two flips.
        let mut t = FlipTiming::new(P60);
        let base = 1_000_000_000;
        run(&mut t, base, P60, 6);
        let last = base + 5 * P60;
        let l = t.landed(last + 2 * P60, with_content(last + 2 * P60, Some(last + 15_000_000)));
        assert!(l.odd.is_some() && !l.missed, "{l:?}");
        assert_eq!((t.odd_stamps, t.missed_stamps), (1, 0));
        // Two flips stamped alike are odd, never a skip.
        let last = last + 2 * P60;
        let l = t.landed(last + 1_000_000, with_content(last + 1_000_000, None));
        assert!(l.odd.is_some() && !l.missed, "{l:?}");
    }

    #[test]
    fn a_skip_after_a_frame_that_paid_the_gpus_wake_up_is_not_missed() {
        // The tenth gate's trial, 60085.717: a commit 10 ms after the last flip, drawn at once, the GPU done 10.8 ms after
        // (4.7 in a run), so the refresh 7 ms away was out of reach for the frame that was there to make it.
        let mut t = FlipTiming::new(P60);
        let base = 1_000_000_000;
        run(&mut t, base, P60, 6);
        let last = base + 5 * P60;
        let content = last + 10_000_000;
        let frame = Timing { slot_ns: last + P60, started_ns: content, ready_ns: content + 300_000, content_ns: Some(content) };
        let l = t.landed(last + 2 * P60, Some((frame, Some(content + 10_800_000))));
        assert!(l.odd.is_some() && !l.missed, "{l:?}");
        // The same commit, a frame of a run's 4.7 ms: the refresh between was there to make.
        let last = last + 2 * P60;
        let content = last + 10_000_000;
        let frame = Timing { slot_ns: last + P60, started_ns: content, ready_ns: content + 300_000, content_ns: Some(content) };
        let l = t.landed(last + 2 * P60, Some((frame, Some(content + 4_700_000))));
        assert!(l.odd.is_some() && l.missed, "{l:?}");
        assert_eq!((t.odd_stamps, t.missed_stamps), (2, 1));
    }

    /// A frame planned by the pacer at `started` (as `Kms::render` does), ready `pipeline` later.
    fn planned(t: &mut FlipTiming, started: u64, pipeline: u64) -> Timing {
        let slot = t.pacer.plan(started).slot_ns;
        Timing { slot_ns: slot, started_ns: started, ready_ns: started + pipeline, content_ns: None }
    }

    #[test]
    fn a_frame_planned_behind_a_late_one_moves_with_the_timeline_and_is_not_late_twice() {
        // x1's mapping on the tenth gate (run 2 of round 5): the first frame lands a refresh late; the second, planned
        // for the slot after it while it was on its way, lands one refresh after it, the soonest it could.
        let mut t = FlipTiming::new(P60);
        let base = 1_000_000_000;
        let refresh = |i: u64| base + i * P60;
        for i in 0..6 {
            let f = planned(&mut t, refresh(i) - 12_000_000, 4_000_000);
            let l = t.landed(refresh(i), Some((f, None)));
            assert_eq!(l.late, 0, "frame {i}: {l:?}");
        }
        let first = planned(&mut t, refresh(6) - 12_000_000, 4_000_000);
        let mut second = planned(&mut t, refresh(7) - 12_000_000, 4_000_000);
        let l = t.landed(refresh(7), Some((first, None)));
        assert_eq!((l.late, l.shift_ns), (1, P60 as i64), "{l:?}");
        second.shift(l.shift_ns);
        let l = t.landed(refresh(8), Some((second, None)));
        assert_eq!((l.late, l.shift_ns), (0, 0), "{l:?}");
        // The next frame is aimed at the refresh right after the second's, not one further.
        let next = planned(&mut t, refresh(8) + 1_000_000, 4_000_000);
        let l = t.landed(refresh(9), Some((next, None)));
        assert_eq!((l.late, l.odd), (0, None), "{l:?}");
    }

    #[test]
    fn the_idle_tick_follows_the_grid_from_the_last_flip_for_a_while() {
        let mut t = FlipTiming::new(16_666_667);
        assert_eq!(t.grid_latch_ns(1_000_000_000, 14_000_000, 180_000_000_000), None, "no flip yet");
        t.landed(1_000_000_000, None);
        // Right after the flip: the next refresh is one period on, its latch 14 ms before it.
        assert_eq!(t.grid_latch_ns(1_000_000_100, 14_000_000, 180_000_000_000), Some(1_016_666_667 - 14_000_000));
        // 2.5 ms before the next refresh's latch has passed: the one after.
        assert_eq!(t.grid_latch_ns(1_003_000_000, 14_000_000, 180_000_000_000), Some(1_033_333_334 - 14_000_000));
        // A minute on, still on the grid; beyond the memory, the fixed tick.
        let later = 1_000_000_000 + 3600 * 16_666_667;
        assert_eq!(t.grid_latch_ns(later + 1, 14_000_000, 180_000_000_000), Some(later + 16_666_667 - 14_000_000));
        assert_eq!(t.grid_latch_ns(1_000_000_000 + 181_000_000_000, 14_000_000, 180_000_000_000), None);
    }
}
