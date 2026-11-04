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

/// Runtime-only refresh policy; startup selection remains the fallback.
pub struct Policy {
    pub default_hz: u32,
    pub manual: bool,
    marquee: bool,
    interactive: bool,
    pub retime: Retime,
    /// LG's output-rate request (`DE_IOW_SET_FR_RATE`, the call the kernel itself makes at a video's first frame).
    pub fr_rate: PathBuf,
    /// The scaler's status for the main window (`connect_src=NONE` when no video feeds it).
    pub scaler: PathBuf,
}

impl Policy {
    pub fn new(default_hz: u32) -> Self {
        Self {
            default_hz,
            manual: false,
            marquee: false,
            interactive: false,
            retime: Retime::default(),
            fr_rate: PathBuf::from(FR_RATE_PATH),
            scaler: PathBuf::from(SCALER_PATH),
        }
    }

    fn target(&self) -> u32 {
        if self.marquee || self.interactive { 120 } else { self.default_hz }
    }
}

/// Free run at the 60 family: 120 Hz on this panel (`framerate-result.md`).
const FR_RATE_PATH: &str = "/proc/lg/de/fr_rate";
const SCALER_PATH: &str = "/proc/lgtv-driver/scaler/main.p0";
const SET_120: &[u8] = b"1 60\n";
/// How long nothing may feed the main window before the rate is asked back: the kernel re-times at a window's first frame and a
/// closing video's window takes a moment to go.
const CLEAR_FOR_NS: u64 = 1_500_000_000;

/// The flip period the 25/50 fps family leaves behind: 10.00 ms for 8.33 and 20.00 ms for 16.67, so a fifth to a third over what
/// the mode asks. A longer one is a film rate, which only a video window causes.
pub fn retimed(expected_ns: u64, measured_ns: u64) -> bool {
    measured_ns * 10 > expected_ns * 11 && measured_ns * 10 < expected_ns * 13
}

/// The one write per episode that asks the panel back to 120 Hz. An episode is a stretch with the panel re-timed and no video
/// feeding the main window; a video that comes back ends it, so a stream that is still playing is never written over, and a
/// write the firmware ignores (measured with no source and with a 25 fps one, `vwin-refresh-result.md`) is not repeated.
#[derive(Default)]
pub struct Retime {
    clear_since: Option<u64>,
    written: bool,
}

impl Retime {
    /// `stuck`: the panel is held at a re-timed rate. `clear`: Home is in front and no video window or source feeds the main window.
    /// `true` when the request is due now.
    pub fn step(&mut self, now_ns: u64, stuck: bool, clear: bool) -> bool {
        if !(stuck && clear) {
            *self = Retime::default();
            return false;
        }
        let since = *self.clear_since.get_or_insert(now_ns);
        if self.written || now_ns.saturating_sub(since) < CLEAR_FOR_NS {
            return false;
        }
        self.written = true;
        true
    }

    /// Whether the request went out in this episode.
    pub fn written(&self) -> bool {
        self.written
    }
}

/// Whether the scaler status says a source feeds the main window; `None` when the text has no such line.
fn source_connected(status: &str) -> Option<bool> {
    status.lines().find_map(|l| l.trim().strip_prefix("connect_src=")).map(|s| s.trim() != "NONE")
}

/// Asks the output for 120 Hz (the 60 family, free run).
fn write_120(path: &Path) -> io::Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new().write(true).open(path)?.write_all(SET_120)
}

impl crate::server::state::State {
    /// Changes the output first, so refusal leaves the effective state unchanged.
    pub fn refresh_apply(&mut self, hz: u32) -> io::Result<u32> {
        let hz = self.output.set_refresh(hz)?;
        self.config.refresh_hz = hz;
        self.config.tick_ns = period_ns(hz);
        Ok(hz)
    }

    /// Puts the panel back to 120 Hz after a 25/50 fps video left it at 100: with the refresh policy automatic, Home in front and
    /// nothing feeding the main window, the request LG's kernel makes at a video's first frame is made once (`Retime`).
    pub fn panel_rate_sync(&mut self, now_ns: u64) {
        let flips = self.output.flip_period_ns().filter(|&p| retimed(period_ns(self.config.refresh_hz), p));
        let stuck = !self.refresh_policy.manual && flips.is_some();
        let clear = stuck && self.panel_rate_clear(!self.refresh_policy.retime.written());
        if !self.refresh_policy.retime.step(now_ns, stuck, clear) {
            return;
        }
        // the flips run at the panel's rate over the OSD divider
        let panel_hz = flips.map_or(0.0, |p| 1e9 / p as f64 * 120.0 / f64::from(self.config.refresh_hz.max(1)));
        match write_120(&self.refresh_policy.fr_rate) {
            Ok(()) => self.journal(format!("refresh: the panel was left at {panel_hz:.0} Hz by a video; 120 Hz asked for, no video window left")),
            Err(e) => log::warn!("refresh: {}: {e}", self.refresh_policy.fr_rate.display()),
        }
    }

    /// Screen on, Home in front (the screen at rest also has no card), no video window of ours and, when `read_source`, no source on the
    /// main window (once the request is made only the cheap checks run, to see a video that ends the episode).
    fn panel_rate_clear(&self, read_source: bool) -> bool {
        let home = self.scene.foreground(&self.surfaces).is_none_or(|w| w.app_id() == crate::scene::HOME_APP);
        let video = self.scene.windows().any(|w| w.kind == crate::window::WindowType::VideoWindow)
            || self.vwin.alloc.granted().any(|c| !c.texture);
        if !self.luna_state.power.rendering || !home || video {
            return false;
        }
        let source = read_source.then(|| std::fs::read_to_string(&self.refresh_policy.scaler).ok().and_then(|s| source_connected(&s))).flatten();
        source != Some(true)
    }

    fn refresh_votes(&self) -> (bool, bool) {
        let marquee = self.scene.foreground(&self.surfaces).is_some_and(|w| w.app_id() == "com.beb.plxnative");
        let interactive = self.shell_rust.connected && self.shell_rust.engine_connected && self.luna_state.power.rendering
            && self.shell_rust.ui_state.get("interactiveVisible").and_then(Value::as_bool) == Some(true);
        (marquee, interactive)
    }

    /// Re-evaluates the combined foreground and interactive UI votes.
    pub fn refresh_policy_sync(&mut self) {
        let (marquee, interactive) = self.refresh_votes();
        self.refresh_policy.marquee = marquee;
        self.refresh_policy.interactive = interactive;
        let hz = self.refresh_policy.target();
        if !self.refresh_policy.manual && hz != self.config.refresh_hz {
            match self.refresh_apply(hz) {
                Ok(hz) => {
                    self.journal(format!("refresh: {hz} Hz (automatic policy)"));
                    self.shell_sync_topics();
                },
                Err(e) => log::warn!("refresh: automatic {hz} Hz refused: {e}"),
            }
        }
    }

    pub fn refresh_auto(&mut self) -> io::Result<u32> {
        let (marquee, interactive) = self.refresh_votes();
        let hz = if marquee || interactive { 120 } else { self.refresh_policy.default_hz };
        let hz = if hz == self.config.refresh_hz { hz } else { self.refresh_apply(hz)? };
        self.refresh_policy.marquee = marquee;
        self.refresh_policy.interactive = interactive;
        self.refresh_policy.manual = false;
        Ok(hz)
    }
}
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
    /// The flips that followed a frame with an animation still running after it ([`Measure::motion`]), the ones of them over [`STALL_NS`]
    /// (refreshes the motion missed) and the worst of those.
    motion_intervals: u64,
    motion_dropped: u64,
    motion_worst_ns: u64,
}

/// The frame rule of the PLAN (`GAP_LIMIT_MS` of the gate): a flip later than this after the one of a frame that had an animation running after
/// it is a refresh the motion missed.
pub const STALL_NS: u64 = 20_000_000;

impl Measure {
    /// A flip `interval_ns` after the last (0 for the first) at a `period_ns` refresh.
    pub fn flip(&mut self, now_ns: u64, interval_ns: u64, period_ns: u64) {
        if self.since_ns == 0 {
            self.since_ns = now_ns;
        }
        self.last_ns = now_ns;
        self.flips += 1;
        if interval_ns == 0 {
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
    }

    /// A flip `interval_ns` after the last, whose frame was drawn `was_animating` (an animation of the scene or the engine ran after it, so
    /// that the frame after it was due at the next refresh). Only the flips that follow such a frame are judged: a gap after a still screen
    /// (a video app's own commits, a toast's hold) or between the phases of a motion (the glass's rest before it settles) is no frame missed,
    /// and an interval over [`STALL_NS`] after a frame of a running animation is.
    pub fn motion(&mut self, interval_ns: u64, was_animating: bool) {
        if !was_animating || interval_ns == 0 {
            return;
        }
        self.motion_intervals += 1;
        if interval_ns > STALL_NS {
            self.motion_dropped += 1;
            self.motion_worst_ns = self.motion_worst_ns.max(interval_ns);
        }
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
        json!({
            "period_ms": ms(period_ns),
            "seconds": (self.last_ns.saturating_sub(self.since_ns)) as f64 / 1e9,
            "flips": self.flips,
            "flips_per_second": self.rate_hz(),
            "intervals": {"one": self.one, "two": self.two, "three": self.three, "more": self.more, "worst_ms": ms(self.worst_interval_ns)},
            "motion": {"intervals": self.motion_intervals, "dropped": self.motion_dropped, "dropped_worst_ms": ms(self.motion_worst_ns)},
            "frames": {"n": self.frames, "late": self.late, "lost": self.lost, "missed": self.missed},
            "pipeline_ms": {"p50": pct(50), "p99": pct(99), "max": (self.pipeline.n() > 0).then(|| ms(self.pipeline.max_ns()))},
        })
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;

    const MS: u64 = 1_000_000;

    fn report(m: &Measure) -> (u64, u64, f64) {
        let r = m.report(16_666_667);
        (r["motion"]["intervals"].as_u64().unwrap(), r["motion"]["dropped"].as_u64().unwrap(), r["motion"]["dropped_worst_ms"].as_f64().unwrap())
    }

    #[test]
    fn a_video_apps_own_gaps_and_a_holds_are_judged_by_no_animation_and_drop_nothing() {
        let mut m = Measure::default();
        // the flips of a still screen: each one's frame had nothing running after it
        for gap in [550, 520, 540, 33, 560, 1000] {
            m.motion(gap * MS, false);
        }
        assert_eq!(report(&m), (0, 0, 0.0));
    }

    #[test]
    fn the_pause_between_the_phases_of_a_motion_is_no_drop_because_the_slide_ended_before_it() {
        let mut m = Measure::default();
        // 8 frames of a slide a refresh apart, the last drawn with the animation over; 100 ms of nothing; the settle's frames a refresh apart
        for _ in 0..7 {
            m.motion(17 * MS, true);
        }
        m.motion(100 * MS, false);
        for _ in 0..30 {
            m.motion(17 * MS, true);
        }
        assert_eq!(report(&m), (37, 0, 0.0));
    }

    #[test]
    fn a_flip_over_20_ms_after_a_frame_of_a_running_animation_is_a_drop_and_the_worst_is_kept() {
        let mut m = Measure::default();
        for gap in [17, 17, 34, 17, 67, 17] {
            m.motion(gap * MS, true);
        }
        assert_eq!(report(&m), (6, 2, 67.0));
        // exactly 20 ms is the rule's bar, not over it; a first flip (no interval) is nothing
        m.motion(20 * MS, true);
        m.motion(0, true);
        assert_eq!(report(&m).1, 2);
    }

    #[test]
    fn at_120_hz_two_refreshes_are_under_the_rule_and_three_are_over_it() {
        let mut m = Measure::default();
        m.motion(8_333_333, true);
        m.motion(16_666_667, true);
        m.motion(25_000_000, true);
        assert_eq!(report(&m).1, 1);
    }

    #[test]
    fn a_measure_that_starts_over_forgets_the_drops() {
        let mut m = Measure::default();
        m.motion(50 * MS, true);
        assert_eq!(report(&m).1, 1);
        m = Measure::default();
        assert_eq!(report(&m), (0, 0, 0.0));
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
    fn a_panel_left_at_100_hz_is_asked_back_once_no_video_is_left() {
        let (p120, p100, p60, p50) = (period_ns(120), 10_000_000, period_ns(60), 20_000_000);
        assert!(retimed(p120, p100) && retimed(p60, p50));
        assert!(!retimed(p120, p120) && !retimed(p60, p60) && !retimed(p120, p60) && !retimed(p120, 41_700_000));

        let s = 1_000_000_000;
        let mut r = Retime::default();
        // the Ring's view is still up: nothing is written, however long the panel is at 100
        assert!(!r.step(0, true, false) && !r.step(10 * s, true, false));
        // it closes: the request waits for the window to be gone, then goes out once and is not repeated
        assert!(!r.step(11 * s, true, true) && !r.step(11 * s + CLEAR_FOR_NS - 1, true, true));
        assert!(r.step(11 * s + CLEAR_FOR_NS, true, true));
        assert!(r.written() && !r.step(12 * s + CLEAR_FOR_NS, true, true) && !r.step(60 * s, true, true));
        // a video that comes back ends the episode and is left alone; its end opens the next
        assert!(!r.step(61 * s, true, false) && !r.written());
        assert!(!r.step(62 * s, true, true) && r.step(62 * s + CLEAR_FOR_NS, true, true));
        // the panel back at 120 forgets the episode: a later re-time starts the wait again
        assert!(!r.step(70 * s, false, true) && !r.written());
        assert!(!r.step(71 * s, true, true) && r.step(71 * s + CLEAR_FOR_NS, true, true));
    }

    #[test]
    fn the_request_is_free_run_at_the_60_family_and_the_source_is_read_from_the_scaler() {
        let path = temp("fr_rate");
        assert!(write_120(&path).is_err(), "the file is not created: a host has none");
        std::fs::write(&path, "").unwrap();
        write_120(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "1 60\n");
        std::fs::remove_file(&path).ok();
        assert_eq!(source_connected("win0\n connect_src=NONE\n"), Some(false));
        assert_eq!(source_connected("connect_src=VDEC"), Some(true));
        assert_eq!(source_connected("nothing"), None);
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
