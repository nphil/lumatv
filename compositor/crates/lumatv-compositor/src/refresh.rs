//! The refresh the output runs at (`compositor.refresh`, `--refresh`, `ctl refresh`): 60, as LG leaves the set, or 120, the test mode
//! of `docs/research/ui120-result.md`.
//!
//! The panel and the display engine run at 120 Hz at Home whatever the UI does (`DE2VSYNC` and `osd_irq` 120 a second,
//! `hardware-census.md`); the OSD plane latches a new framebuffer at every one of those vsyncs. What runs at 60 is the page-flip
//! completion the pacer keys to: LG's `lg1k` driver hands the DRM core every second OSD vsync, a software divider
//! (`fbdev_vsync_cnt_on_flip`, SRC `core/fbdev/fbdev_drv.c:2939`, "value 2 makes 60Hz handler (120Hz/2=60Hz)"), a module parameter
//! root may write at run time (`framerate-result.md`). At 1 the flips land at every refresh, 8.33 ms apart, and the pacer follows them
//! (`kms/timing.rs` snaps to 120 Hz after four agreeing intervals). The KMS mode list is cosmetic and never touched: the CRTC does not
//! set panel timing (`gfn-444-vrr-result.md`). Under 50-family content the engine runs 100 Hz and the divider at 1 gives 100 flips a
//! second; the pacer follows that too.
//!
//! What the mode decides is measured here per flip since the last switch ([`Measure`]): how many refreshes each flip was after the
//! last, the frames late and lost against the pacer's slots, the refreshes a frame was due at and missed, and the frame pipeline
//! (draw to the GPU's finish) as p50, p99 and max. The budget at 120 is 8.33 ms less the flip's lead; the shell client's frames cost
//! 15-16 ms of CPU and GPU on the picker's slide at 60 (`compositor-surfaces-result.md`), so the report says what the set does, not what
//! it should.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The refresh without the mode: LG's divider of 2 over the 120 Hz OSD vsync.
pub const DEFAULT_HZ: u32 = 60;
/// The OSD's vsync, which the divider divides: 120 a second on this set.
const OSD_VSYNC_HZ: u32 = 120;
/// The module parameter that divides the OSD's vsync into page-flip completions.
pub const DIVIDER_PATH: &str = "/sys/module/lg1k/parameters/fbdev_vsync_cnt_on_flip";

/// The refreshes the mode knows.
pub fn supported(hz: u32) -> bool {
    hz == 60 || hz == 120
}

/// The refresh period, ns, rounded (16,666,667 at 60: the default tick's own figure).
pub fn period_ns(hz: u32) -> u64 {
    (1e9 / hz.max(1) as f64).round() as u64
}

/// The divider that gives `hz` page flips a second from the OSD's vsync.
pub fn divider_for(hz: u32) -> u32 {
    (OSD_VSYNC_HZ / hz.max(1)).max(1)
}

/// The OSD vsync divider as a file: what it held before this compositor's first write is put back by [`Divider::restore`] and when
/// the divider is dropped (the compositor's exit), so LG's compositor finds its 60 Hz flips after a fallback.
pub struct Divider {
    path: PathBuf,
    /// The value found before the first write, once written.
    found: Option<u32>,
    /// The last value written, if any.
    written: Option<u32>,
}

impl Divider {
    pub fn new() -> Divider {
        Divider::at(DIVIDER_PATH)
    }

    pub fn at(path: impl AsRef<Path>) -> Divider {
        Divider { path: path.as_ref().to_path_buf(), found: None, written: None }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What was found before the first write.
    pub fn found(&self) -> Option<u32> {
        self.found
    }

    /// The divider in force, read from the file.
    pub fn read(&self) -> io::Result<u32> {
        let text = std::fs::read_to_string(&self.path)?;
        text.trim().parse().map_err(|_| io::Error::other(format!("{}: not a number: {:?}", self.path.display(), text.trim())))
    }

    /// Writes the divider for `hz` and reads it back; what the file holds afterwards.
    pub fn set(&mut self, hz: u32) -> io::Result<u32> {
        let want = divider_for(hz);
        let now = self.read()?;
        if self.found.is_none() {
            self.found = Some(now);
        }
        if now != want {
            std::fs::write(&self.path, format!("{want}\n"))?;
            self.written = Some(want);
        }
        let got = self.read()?;
        if got != want {
            return Err(io::Error::other(format!("{} reads {got} after writing {want}", self.path.display())));
        }
        Ok(got)
    }

    /// Puts back what was found before the first write; `None` when nothing was ever written.
    pub fn restore(&mut self) -> io::Result<Option<u32>> {
        let Some(found) = self.found else { return Ok(None) };
        if self.written.take().is_some() {
            std::fs::write(&self.path, format!("{found}\n"))?;
        }
        Ok(Some(found))
    }

    pub fn report(&self) -> Value {
        json!({"path": self.path, "value": self.read().ok(), "found": self.found, "written": self.written})
    }
}

impl Default for Divider {
    fn default() -> Self {
        Divider::new()
    }
}

impl Drop for Divider {
    fn drop(&mut self) {
        if let Err(e) = self.restore() {
            log::warn!("refresh: {}: {e}; LG's compositor would find page flips at the OSD's rate", self.path.display());
        }
    }
}

/// Buckets of [`BUCKET_NS`] up to [`HIST_MAX_NS`]; the last bucket takes everything over it.
const BUCKETS: usize = 200;
const BUCKET_NS: u64 = 250_000;
const HIST_MAX_NS: u64 = BUCKETS as u64 * BUCKET_NS;

/// A fixed histogram of durations: no allocation per sample, percentiles to a quarter of a millisecond.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Histogram {
    buckets: [u32; BUCKETS],
    n: u64,
    max_ns: u64,
}

impl Default for Histogram {
    fn default() -> Self {
        Histogram { buckets: [0; BUCKETS], n: 0, max_ns: 0 }
    }
}

impl Histogram {
    pub fn add(&mut self, ns: u64) {
        let i = ((ns / BUCKET_NS) as usize).min(BUCKETS - 1);
        self.buckets[i] += 1;
        self.n += 1;
        self.max_ns = self.max_ns.max(ns);
    }

    pub fn n(&self) -> u64 {
        self.n
    }

    pub fn max_ns(&self) -> u64 {
        self.max_ns
    }

    /// The upper edge of the bucket the `p`-th percentile falls in (`p` in 0..=100), ns; `None` with no sample. A value over the
    /// histogram's reach reads as the reach.
    pub fn percentile_ns(&self, p: u32) -> Option<u64> {
        if self.n == 0 {
            return None;
        }
        let rank = (self.n * p.min(100) as u64).div_ceil(100).max(1);
        let mut seen = 0u64;
        for (i, &c) in self.buckets.iter().enumerate() {
            seen += c as u64;
            if seen >= rank {
                return Some(((i as u64 + 1) * BUCKET_NS).min(self.max_ns.max(BUCKET_NS)).min(HIST_MAX_NS));
            }
        }
        Some(HIST_MAX_NS)
    }
}

/// What the flips said since the last switch of the mode (or the start): the measure `ctl refresh` reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Measure {
    /// When the measure started (CLOCK_MONOTONIC ns), 0 before the first flip.
    pub since_ns: u64,
    pub last_ns: u64,
    pub flips: u64,
    /// Flips one, two, three and more refreshes after the last, at the period in force when they landed.
    pub one: u64,
    pub two: u64,
    pub three: u64,
    pub more: u64,
    pub worst_interval_ns: u64,
    /// Frames of ours the flips were of, and what the pacer made of their landing.
    pub frames: u64,
    pub late: u64,
    pub lost: u64,
    pub missed: u64,
    pub pipeline: Histogram,
    /// Runs of flips (each at most [`RUN_GAP_NS`] after the last) that ended with [`RUN_MIN`] flips or more, the idle gaps between runs, and the
    /// intervals over [`STALL_NS`] inside those runs: a motion that missed a refresh. The run still going is counted by [`Measure::runs`].
    runs_done: u64,
    gaps: u64,
    stalls_done: u64,
    stall_worst_done_ns: u64,
    run_flips: u32,
    run_stalls: u32,
    run_worst_ns: u64,
}

/// An interval over this ends a run of frames: the gaps between a video app's own commits (about half a second apart) and a toast's hold are over
/// it; the frames of a motion, and a stall inside one, are under it.
pub const RUN_GAP_NS: u64 = 250_000_000;
/// The frame rule of the PLAN (`GAP_LIMIT_MS` of the gate): an interval over this inside a run is a stall.
pub const STALL_NS: u64 = 20_000_000;
/// Flips a run needs to count as a motion: two are a pair (an app's commit and the frame after it), which no stall belongs to.
pub const RUN_MIN: u32 = 3;

impl Measure {
    /// A flip `interval_ns` after the last (0 for the first) at a `period_ns` refresh.
    pub fn flip(&mut self, now_ns: u64, interval_ns: u64, period_ns: u64) {
        if self.since_ns == 0 {
            self.since_ns = now_ns;
        }
        self.last_ns = now_ns;
        self.flips += 1;
        if interval_ns == 0 {
            self.run_flips = 1;
            return;
        }
        let refreshes = (interval_ns + period_ns / 2) / period_ns.max(1);
        match refreshes {
            0 | 1 => self.one += 1,
            2 => self.two += 1,
            3 => self.three += 1,
            _ => self.more += 1,
        }
        self.worst_interval_ns = self.worst_interval_ns.max(interval_ns);
        if interval_ns > RUN_GAP_NS {
            self.close_run();
            self.gaps += 1;
            self.run_flips = 1;
        } else {
            self.run_flips += 1;
            if interval_ns > STALL_NS {
                self.run_stalls += 1;
                self.run_worst_ns = self.run_worst_ns.max(interval_ns);
            }
        }
    }

    /// The run that ended is counted when it was a motion (a pair or a lone flip is not one).
    fn close_run(&mut self) {
        if self.run_flips >= RUN_MIN {
            self.runs_done += 1;
            self.stalls_done += self.run_stalls as u64;
            self.stall_worst_done_ns = self.stall_worst_done_ns.max(self.run_worst_ns);
        }
        self.run_flips = 0;
        self.run_stalls = 0;
        self.run_worst_ns = 0;
    }

    /// The motions the flips made: `(runs, stalls, the worst stall in ns)`, the run that is still going included. A stall is an interval over
    /// [`STALL_NS`] between two flips of one run of [`RUN_MIN`] flips or more; the gaps between runs (the idle cadence of a video app's own
    /// commits, a hold) are none, so a frame rule that is read here is not failed by a screen that was still.
    pub fn runs(&self) -> (u64, u64, u64) {
        let open = self.run_flips >= RUN_MIN;
        let (stalls, worst) = if open { (self.run_stalls as u64, self.run_worst_ns) } else { (0, 0) };
        (self.runs_done + open as u64, self.stalls_done + stalls, self.stall_worst_done_ns.max(worst))
    }

    /// A frame of ours landed: its pipeline (draw to the GPU's finish) and what the pacer said.
    pub fn frame(&mut self, pipeline_ns: u64, late: i32, lost: u32, missed: bool) {
        self.frames += 1;
        self.pipeline.add(pipeline_ns);
        if late > 0 {
            self.late += 1;
        }
        self.lost += lost as u64;
        if missed {
            self.missed += 1;
        }
    }

    /// Flips a second over the measure, from its first to its last flip.
    pub fn rate_hz(&self) -> Option<f64> {
        (self.flips >= 2 && self.last_ns > self.since_ns).then(|| (self.flips - 1) as f64 * 1e9 / (self.last_ns - self.since_ns) as f64)
    }

    pub fn report(&self, period_ns: u64) -> Value {
        let ms = |ns: u64| ns as f64 / 1e6;
        let pct = |p: u32| self.pipeline.percentile_ns(p).map(ms);
        let (runs, stalls, stall_worst) = self.runs();
        json!({
            "period_ms": ms(period_ns),
            "seconds": (self.last_ns.saturating_sub(self.since_ns)) as f64 / 1e9,
            "flips": self.flips,
            "flips_per_second": self.rate_hz(),
            "intervals": {"one": self.one, "two": self.two, "three": self.three, "more": self.more, "worst_ms": ms(self.worst_interval_ns)},
            "runs": {"n": runs, "stalls": stalls, "stall_worst_ms": ms(stall_worst), "gaps": self.gaps},
            "frames": {"n": self.frames, "late": self.late, "lost": self.lost, "missed": self.missed},
            "pipeline_ms": {"p50": pct(50), "p99": pct(99), "max": (self.pipeline.n() > 0).then(|| ms(self.pipeline.max_ns()))},
        })
    }
}

#[cfg(test)]
mod run_tests {
    use super::*;

    const P60: u64 = 16_666_667;
    const MS: u64 = 1_000_000;

    /// Flips at the given intervals (ms) after a first flip at 1 s, as `FlipTiming::landed` feeds them.
    fn feed(m: &mut Measure, intervals_ms: &[u64]) {
        let mut t = 1_000 * MS;
        for &i in intervals_ms {
            t += i * MS;
            m.flip(t, i * MS, P60);
        }
    }

    #[test]
    fn a_video_apps_own_cadence_is_gaps_and_no_stall() {
        let mut m = Measure::default();
        feed(&mut m, &[550, 520, 540, 560, 500, 530]);
        assert_eq!(m.runs(), (0, 0, 0));
        assert_eq!(m.report(P60)["runs"]["gaps"], 6);
        // the old buckets still call every one of them an interval over 20 ms
        assert_eq!(m.more, 6);
    }

    #[test]
    fn an_idle_pair_is_not_a_motion() {
        let mut m = Measure::default();
        feed(&mut m, &[600, 33, 600, 20, 600]);
        assert_eq!(m.runs(), (0, 0, 0));
    }

    #[test]
    fn a_motion_with_a_missed_refresh_has_one_stall_and_the_gaps_around_it_are_not_stalls() {
        let mut m = Measure::default();
        // idle gap, 20 frames a refresh apart with one 33 ms interval in the middle, idle gap, a lone flip
        let mut v = vec![540];
        v.extend([17; 9]);
        v.push(33);
        v.extend([17; 10]);
        v.extend([540, 540]);
        feed(&mut m, &v);
        let r = m.report(P60);
        assert_eq!((r["runs"]["n"].as_u64(), r["runs"]["stalls"].as_u64(), r["runs"]["gaps"].as_u64()), (Some(1), Some(1), Some(3)));
        assert_eq!(r["runs"]["stall_worst_ms"].as_f64().unwrap().round(), 33.0);
    }

    #[test]
    fn a_motion_still_going_is_counted_and_a_thirty_frames_a_second_cadence_is_all_stalls() {
        let mut m = Measure::default();
        feed(&mut m, &[540, 33, 33, 33, 33]);
        assert_eq!(m.runs(), (1, 4, 33 * MS));
    }

    #[test]
    fn a_hold_between_two_motions_ends_the_first_and_starts_the_second() {
        let mut m = Measure::default();
        let mut v = vec![540];
        v.extend([17; 6]);
        v.push(1200);
        v.extend([17; 6]);
        feed(&mut m, &v);
        let (runs, stalls, _) = m.runs();
        assert_eq!((runs, stalls), (2, 0));
    }

    #[test]
    fn at_120_hz_two_refreshes_are_not_a_stall_and_three_are() {
        let mut m = Measure::default();
        let p = 8_333_333;
        let mut t = 1_000 * MS;
        for i in [540 * MS, 8_333_333, 8_333_333, 16_666_667, 8_333_333, 25_000_000, 8_333_333] {
            t += i;
            m.flip(t, i, p);
        }
        assert_eq!(m.runs(), (1, 1, 25 * MS));
    }

    #[test]
    fn a_measure_that_starts_over_forgets_the_run() {
        let mut m = Measure::default();
        feed(&mut m, &[540, 17, 17, 33]);
        assert_eq!(m.runs().1, 1);
        m = Measure::default();
        assert_eq!(m.runs(), (0, 0, 0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("lumatv-refresh-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn periods_and_dividers() {
        assert_eq!(period_ns(60), 16_666_667);
        assert_eq!(period_ns(120), 8_333_333);
        assert_eq!(divider_for(60), 2);
        assert_eq!(divider_for(120), 1);
        assert!(supported(60) && supported(120) && !supported(90) && !supported(0));
    }

    #[test]
    fn the_divider_is_written_read_back_and_put_back() {
        let p = temp("divider");
        std::fs::write(&p, "2\n").unwrap();
        let mut d = Divider::at(&p);
        assert_eq!(d.set(120).unwrap(), 1);
        assert_eq!(std::fs::read_to_string(&p).unwrap().trim(), "1");
        assert_eq!(d.found(), Some(2));
        // 60 writes LG's value back; a second set at the same value writes nothing new
        assert_eq!(d.set(60).unwrap(), 2);
        assert_eq!(d.set(60).unwrap(), 2);
        assert_eq!(d.set(120).unwrap(), 1);
        assert_eq!(d.restore().unwrap(), Some(2));
        assert_eq!(std::fs::read_to_string(&p).unwrap().trim(), "2");
        // dropping it after a write puts the found value back too
        d.set(120).unwrap();
        drop(d);
        assert_eq!(std::fs::read_to_string(&p).unwrap().trim(), "2");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_missing_divider_is_an_error_and_restores_nothing() {
        let mut d = Divider::at(temp("missing"));
        assert!(d.set(120).is_err());
        assert_eq!(d.restore().unwrap(), None);
    }

    #[test]
    fn the_histogram_gives_percentiles_to_a_quarter_millisecond() {
        let mut h = Histogram::default();
        assert_eq!(h.percentile_ns(50), None);
        for i in 1..=100u64 {
            h.add(i * 100_000); // 0.1 .. 10 ms
        }
        assert_eq!(h.n(), 100);
        assert_eq!(h.max_ns(), 10_000_000);
        assert_eq!(h.percentile_ns(50), Some(5_250_000), "the upper edge of the bucket the 50th sample is in");
        assert_eq!(h.percentile_ns(99), Some(10_000_000));
        assert_eq!(h.percentile_ns(100), Some(10_000_000));
        h.add(400_000_000);
        assert_eq!(h.percentile_ns(100), Some(HIST_MAX_NS), "a sample past the reach reads as the reach");
    }

    #[test]
    fn the_measure_counts_refreshes_between_flips_at_the_period_in_force() {
        let mut m = Measure::default();
        let p = period_ns(120);
        m.flip(1_000_000_000, 0, p);
        m.flip(1_000_000_000 + p, p, p);
        m.flip(1_000_000_000 + 3 * p, 2 * p + 40_000, p);
        m.flip(1_000_000_000 + 6 * p, 3 * p - 40_000, p);
        m.flip(1_000_000_000 + 12 * p, 6 * p, p);
        assert_eq!((m.flips, m.one, m.two, m.three, m.more), (5, 1, 1, 1, 1));
        assert_eq!(m.worst_interval_ns, 6 * p);
        m.frame(4_700_000, 0, 0, false);
        m.frame(9_100_000, 1, 1, true);
        assert_eq!((m.frames, m.late, m.lost, m.missed), (2, 1, 1, 1));
        let r = m.report(p);
        assert_eq!(r["flips"], 5);
        assert_eq!(r["intervals"]["two"], 1);
        assert_eq!(r["frames"]["missed"], 1);
        assert_eq!(r["pipeline_ms"]["p50"], 4.75);
        assert_eq!(r["pipeline_ms"]["max"], 9.1);
        let hz = r["flips_per_second"].as_f64().unwrap();
        assert!((hz - 40.0).abs() < 0.5, "4 intervals over 12 periods at 120 Hz: {hz}");
    }
}
