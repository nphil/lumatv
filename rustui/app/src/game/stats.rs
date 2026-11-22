//! Four-Hz snapshots for the retained HUD; no per-frame painting and no invented latency.
use super::diagnosis::{Cause, Diagnoser, Evidence, Report, report, scores};
use crate::lifecycle::Frames;
use lumagame_core::ipc::Stats;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;

pub const INTERVAL_MS: u64 = 250;
pub const STALE_MS: u64 = 2_000;
const WINDOW_MS: u64 = 10_000;
/// One reading of the stream's own measurements a second, and how many the HUD's graphs can show.
const TREND_EVERY_MS: u64 = 900;
const TREND_LEN: usize = 120;
/// Drops are judged over this long, so one old drop stops counting.
const DROP_WINDOW_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode { #[default] Off, Compact, Full }
impl Mode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Compact, Self::Full];
    pub fn index(self) -> usize { match self { Self::Off => 0, Self::Compact => 1, Self::Full => 2 } }
    pub fn name(self) -> &'static str { ["Off", "Compact", "Full"][self.index()] }
    pub fn on(self) -> bool { self != Self::Off }
    pub fn next(self) -> Self { Self::ALL[(self.index() + 1) % 3] }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overlay { pub mode: Mode, pub changed: u64 }
impl Overlay {
    pub fn read(value: &Value) -> Self {
        if value == &Value::Bool(true) { Self { mode: Mode::Compact, changed: 1 } }
        else { serde_json::from_value(value.clone()).unwrap_or_default() }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameView {
    pub fed: u64,
    pub accepted: u64,
    pub dropped: [u64; 5],
    pub omitted: u64,
    pub gaps: Option<[u64; 4]>,
    pub feed: Option<[u64; 4]>,
    pub queue: Option<u32>,
    pub graph: Vec<u64>,
}
#[derive(Clone, Debug, Default)]
pub struct FrameHistory {
    samples: VecDeque<(u64, [i64; 4])>,
    last: Frames,
    pub at: Option<u64>,
}
impl FrameHistory {
    pub fn push(&mut self, mut batch: Frames, now: u64) {
        if batch.fed < self.last.fed { self.samples.clear(); }
        if !batch.samples.is_empty() { self.at = Some(now); }
        self.samples.extend(batch.samples.drain(..).map(|s| (now, s)));
        while self.samples.len() > 2048 { self.samples.pop_front(); }
        self.last = batch;
    }
    pub fn view(&mut self, now: u64) -> FrameView {
        while self.samples.front().is_some_and(|(at, _)| now.saturating_sub(*at) > WINDOW_MS) { self.samples.pop_front(); }
        let gaps: Vec<u64> = self.samples.iter().filter_map(|(_, s)| u64::try_from(s[0]).ok()).collect();
        let feed = self.samples.iter().filter_map(|(_, s)| (s[3] == 0).then(|| u64::try_from(s[1]).ok()).flatten()).collect::<Vec<_>>();
        let graph = gaps.chunks(4).map(|s| *s.iter().max().unwrap()).collect();
        FrameView { fed: self.last.fed, accepted: self.last.accepted, dropped: self.last.dropped, omitted: self.last.omitted,
            gaps: percentiles(gaps), feed: percentiles(feed), graph,
            queue: self.samples.back().and_then(|(_, s)| u32::try_from(s[2]).ok()) }
    }
}
pub(crate) fn percentiles(mut values: Vec<u64>) -> Option<[u64; 4]> {
    if values.is_empty() { return None; }
    values.sort_unstable();
    Some([50, 95, 99, 100].map(|p| values[(p * values.len()).div_ceil(100) - 1]))
}
pub fn fresh(at: Option<u64>, now: u64) -> bool { at.is_some_and(|at| now.saturating_sub(at) <= STALE_MS) }

/// What the diagnosis needs from outside the telemetry: what the stream was asked for.
#[derive(Clone, Debug, Default)]
pub struct Asked { pub fps: u32, pub cap_mbps: u32, pub sdr_reason: Option<String> }

/// The stream's own readings a second apart, for the HUD's jitter, loss and bitrate graphs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trend { pub jitter_ms: Option<f32>, pub loss_pct: Option<f32>, pub bitrate_mbps: Option<f32> }

/// Panel care for the HUD (the TV's OLED must not wear under a fixed overlay). The HUD is drawn at full strength for
/// `DIM_AFTER_MS` after it appears, then at `DIM_TO` of that strength for as long as it stays, and every `SHIFT_EVERY_MS` it
/// moves to the next of `SHIFTS`: logical pixels left and down from its corner of the safe area, so it never leaves it. It is
/// never brighter than the strength it has without this.
pub const DIM_AFTER_MS: u64 = 20_000;
pub const DIM_TO: f32 = 0.6;
pub const SHIFT_EVERY_MS: u64 = 60_000;
pub const SHIFTS: [(f32, f32); 6] = [(0.0, 0.0), (10.0, 6.0), (4.0, 12.0), (12.0, 2.0), (2.0, 8.0), (8.0, 0.0)];
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rest { since: Option<u64>, dimmed: bool, step: usize }
impl Rest {
    /// Moves on to `now`; whether the HUD has to be drawn again.
    pub fn tick(&mut self, now: u64) -> bool {
        let since = *self.since.get_or_insert(now);
        let elapsed = now.saturating_sub(since);
        let (dimmed, step) = (elapsed >= DIM_AFTER_MS, (elapsed / SHIFT_EVERY_MS) as usize % SHIFTS.len());
        let changed = (dimmed, step) != (self.dimmed, self.step);
        (self.dimmed, self.step) = (dimmed, step);
        changed
    }
    /// How much of its usual strength the HUD is drawn with.
    pub fn strength(&self) -> f32 { if self.dimmed { DIM_TO } else { 1.0 } }
    /// How far left and down of its corner the HUD sits.
    pub fn shift(&self) -> (f32, f32) { SHIFTS[self.step] }
}

/// Frames dropped by reason, and packets the socket discarded, as counted so far.
type Drops = ([u64; 5], u64);
#[derive(Clone, Debug, Default)]
pub struct Telemetry {
    pub stream: Stats,
    pub display: Value,
    pub frames: FrameView,
    pub live: [bool; 4],
    pub trend: VecDeque<Trend>,
    pub evidence: Evidence,
    pub report: Report,
    history: FrameHistory,
    stream_at: Option<u64>,
    display_at: Option<u64>,
    pad_at: Option<u64>,
    pending_stream: Option<Stats>,
    pending_display: Option<Value>,
    pub rest: Rest,
    due: u64,
    dirty: bool,
    trend_at: Option<u64>,
    drops: VecDeque<(u64, [u64; 5], u64)>,
    diagnoser: Diagnoser,
}
impl Telemetry {
    pub fn stream(&mut self, stats: Stats, now: u64) {
        self.stream_at = Some(now);
        if self.trend_at.is_none_or(|at| now.saturating_sub(at) >= TREND_EVERY_MS) {
            self.trend_at = Some(now);
            self.trend.push_back(Trend { jitter_ms: stats.jitter_ms, loss_pct: stats.loss_pct, bitrate_mbps: stats.bitrate_kbps.map(|k| k as f32 / 1000.0) });
            while self.trend.len() > TREND_LEN { self.trend.pop_front(); }
        }
        self.pad_at = stats.pad.as_ref().and_then(|p| (p.reports > 1).then(|| now.checked_sub(p.age_ms)).flatten());
        self.pending_stream = Some(stats);
        self.dirty = true;
    }
    pub fn display(&mut self, value: Value, now: u64) {
        self.display_at = value.get("window").filter(|v| !v.is_null()).map(|_| now);
        self.pending_display = Some(value);
        self.dirty = true;
    }
    pub fn frames(&mut self, batch: Frames, now: u64) {
        self.dirty |= !batch.samples.is_empty();
        self.history.push(batch, now);
    }
    pub fn tick(&mut self, now: u64, asked: &Asked) -> bool {
        if now < self.due { return false; }
        self.due = now.saturating_add(INTERVAL_MS);
        let live = [fresh(self.stream_at, now), fresh(self.history.at, now), fresh(self.display_at, now), fresh(self.pad_at, now)];
        let changed = self.dirty || live != self.live;
        self.live = live;
        if !changed { return false; }
        self.dirty = false;
        if let Some(s) = self.pending_stream.take() { self.stream = s; }
        if let Some(d) = self.pending_display.take() { self.display = d; }
        self.frames = self.history.view(now);
        if self.drops.back().is_some_and(|(_, d, _)| self.frames.dropped.iter().sum::<u64>() < d.iter().sum()) { self.drops.clear(); }
        self.drops.push_back((now, self.frames.dropped, self.stream.socket_drops.unwrap_or(0)));
        while self.drops.front().is_some_and(|(at, ..)| now.saturating_sub(*at) > DROP_WINDOW_MS) { self.drops.pop_front(); }
        self.evidence = self.evidence(asked);
        let cause = self.diagnoser.update(scores(&self.evidence));
        self.report = report(&self.evidence, cause);
        true
    }
    pub fn cause(&self) -> Option<Cause> { self.report.cause }
    /// The readings that are live now, as the diagnosis takes them: one that went stale is absent, not zero.
    fn evidence(&self, asked: &Asked) -> Evidence {
        let [stream_live, frames_live, display_live, pad_live] = self.live;
        let (s, f) = (&self.stream, &self.frames);
        let st = |v: Option<f32>| v.filter(|_| stream_live);
        let window = |read: &dyn Fn(&Drops) -> u64| match (self.drops.front(), self.drops.back()) {
            (Some((_, a, x)), Some((_, b, y))) => read(&(*b, *y)).saturating_sub(read(&(*a, *x))),
            _ => 0,
        };
        let pad = s.pad.as_ref().filter(|_| pad_live);
        let firmware = &self.display["firmware"]["sysVeri10"]["videoDelayUs"];
        Evidence {
            target_us: 1_000_000.0 / asked.fps.max(1) as f32,
            fps: st(s.fps),
            rtt_ms: st(s.rtt_ms),
            jitter_ms: st(s.jitter_ms),
            loss_pct: st(s.loss_pct),
            bitrate_mbps: st(s.bitrate_kbps.map(|k| k as f32 / 1000.0)),
            cap_mbps: (asked.cap_mbps > 0).then_some(asked.cap_mbps as f32),
            host_ms: st(s.host_ms),
            client_ms: st(s.client_ms),
            pacing_ms: st(s.pacing_ms),
            host_gap_p95_ms: st(s.host_gap_ms.map(|g| g[1])),
            arrival_gap_p95_ms: st(s.arrival_gap_ms.map(|g| g[1])),
            gap_p99_us: f.gaps.filter(|_| frames_live).map(|g| g[2]),
            gap_max_us: f.gaps.filter(|_| frames_live).map(|g| g[3]),
            feed_p99_us: f.feed.filter(|_| frames_live).map(|g| g[2]),
            tv_drops: window(&|(d, _)| d[0] + d[1] + d[2] + d[4]),
            loss_drops: window(&|(d, _)| d[3]),
            socket_drops: window(&|(_, x)| *x),
            refreshes: Evidence::refreshes_of(&self.display).filter(|_| display_live),
            panel_delay_ms: firmware.as_f64().filter(|_| display_live).map(|v| v as f32 / 1000.0),
            pad_rate_hz: pad.map(|p| p.rate_hz),
            pad_gap_p99_ms: pad.map(|p| p.p99_us as f32 / 1000.0),
            pad_bursts: pad.map_or(0, |p| p.bursts),
            sdr_reason: asked.sdr_reason.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn timings_count_reasons_use_nearest_rank_and_expire() {
        let mut t = Telemetry::default();
        t.frames(Frames { fed: 5, accepted: 4, dropped: [1, 0, 0, 0, 0], omitted: 0,
            samples: vec![[-1, 200, 1, 0], [8000, 400, 2, 0], [9000, 600, 3, 0], [10000, 800, 1, 0], [20000, 900, 1, 1]] }, 1000);
        assert!(t.tick(1000, &Asked::default()));
        assert_eq!(t.frames.gaps, Some([9000, 20000, 20000, 20000]));
        assert_eq!(t.frames.feed, Some([400, 800, 800, 800]));
        assert_eq!(t.frames.graph, [20000]);
        assert_eq!(t.frames.queue, Some(1));
        assert_eq!(t.frames.dropped, [1, 0, 0, 0, 0]);
        assert!(t.live[1]);
        assert!(!t.tick(1001, &Asked::default()));
        assert!(t.tick(3001, &Asked::default()));
        assert!(!t.live[1]);
        assert!(!t.tick(3251, &Asked::default()));
    }
    #[test]
    fn the_hud_dims_after_twenty_seconds_and_steps_within_twelve_pixels_each_minute() {
        let mut rest = Rest::default();
        assert!(!rest.tick(5_000), "the first reading starts the clock");
        assert_eq!((rest.strength(), rest.shift()), (1.0, (0.0, 0.0)));
        assert!(!rest.tick(5_000 + DIM_AFTER_MS - 1));
        assert_eq!(rest.strength(), 1.0);
        assert!(rest.tick(5_000 + DIM_AFTER_MS), "the dim needs a repaint");
        assert_eq!((rest.strength(), rest.shift()), (DIM_TO, (0.0, 0.0)));
        assert!(!rest.tick(5_000 + SHIFT_EVERY_MS - 1));
        assert!(rest.tick(5_000 + SHIFT_EVERY_MS), "the move needs a repaint");
        assert_eq!((rest.strength(), rest.shift()), (DIM_TO, SHIFTS[1]), "a move never brightens it again");
        let mut seen = vec![rest.shift()];
        for minute in 2..=SHIFTS.len() as u64 {
            rest.tick(5_000 + minute * SHIFT_EVERY_MS);
            assert!(rest.strength() <= 1.0 && rest.strength() == DIM_TO);
            seen.push(rest.shift());
        }
        assert_eq!(seen.last(), Some(&SHIFTS[0]), "the cycle comes back to the corner");
        assert!(SHIFTS.iter().all(|(dx, dy)| (0.0..=12.0).contains(dx) && (0.0..=12.0).contains(dy)), "left and down only, a few pixels");
        assert!(SHIFTS.windows(2).all(|w| w[0] != w[1]));
    }
    #[test]
    fn old_toggle_migrates_and_repeated_stream_stats_do_not_revive_idle_pads() {
        assert_eq!(Overlay::read(&json!(true)).mode, Mode::Compact);
        assert_eq!(Overlay::read(&json!(false)).mode, Mode::Off);
        let mut t = Telemetry::default();
        t.stream(Stats { pad: Some(lumagame_core::ipc::PadStats { reports: 5, age_ms: 2100, ..Default::default() }), ..Default::default() }, 5000);
        assert!(t.tick(5000, &Asked::default()));
        assert!(t.live[0]);
        assert!(!t.live[3]);
        assert_eq!(Mode::Full.next(), Mode::Off);
    }
}
