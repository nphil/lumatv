//! Four-Hz snapshots for the retained HUD; no per-frame painting and no invented latency.
use crate::lifecycle::Frames;
use lumagame_core::ipc::Stats;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;

pub const INTERVAL_MS: u64 = 250;
pub const STALE_MS: u64 = 2_000;
const WINDOW_MS: u64 = 10_000;

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

#[derive(Clone, Debug, Default)]
pub struct Telemetry {
    pub stream: Stats,
    pub display: Value,
    pub frames: FrameView,
    pub live: [bool; 4],
    history: FrameHistory,
    stream_at: Option<u64>,
    display_at: Option<u64>,
    pad_at: Option<u64>,
    pending_stream: Option<Stats>,
    pending_display: Option<Value>,
    due: u64,
    dirty: bool,
}
impl Telemetry {
    pub fn stream(&mut self, stats: Stats, now: u64) {
        self.stream_at = Some(now);
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
    pub fn tick(&mut self, now: u64) -> bool {
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
        true
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
        assert!(t.tick(1000));
        assert_eq!(t.frames.gaps, Some([9000, 20000, 20000, 20000]));
        assert_eq!(t.frames.feed, Some([400, 800, 800, 800]));
        assert_eq!(t.frames.graph, [20000]);
        assert_eq!(t.frames.queue, Some(1));
        assert_eq!(t.frames.dropped, [1, 0, 0, 0, 0]);
        assert!(t.live[1]);
        assert!(!t.tick(1001));
        assert!(t.tick(3001));
        assert!(!t.live[1]);
        assert!(!t.tick(3251));
    }
    #[test]
    fn old_toggle_migrates_and_repeated_stream_stats_do_not_revive_idle_pads() {
        assert_eq!(Overlay::read(&json!(true)).mode, Mode::Compact);
        assert_eq!(Overlay::read(&json!(false)).mode, Mode::Off);
        let mut t = Telemetry::default();
        t.stream(Stats { pad: Some(lumagame_core::ipc::PadStats { reports: 5, age_ms: 2100, ..Default::default() }), ..Default::default() }, 5000);
        assert!(t.tick(5000));
        assert!(t.live[0]);
        assert!(!t.live[3]);
        assert_eq!(Mode::Full.next(), Mode::Off);
    }
}
