//! What a controller test shows of a pad: how evenly its reports arrive, from the kernel's own stamps, and where a stick rests once
//! it is let go. Plain arithmetic on stamps, so the app and the tests feed it alike.

use std::collections::VecDeque;

/// Reports older than this leave the figures.
pub const WINDOW_NS: u64 = 3_000_000_000;
/// The kernel passes on only changed reports, so a pad nobody touches sends nothing: a gap this long is a pause, not a slow report.
pub const IDLE_NS: u64 = 250_000_000;
/// Interval histogram edges in µs: bucket `i` counts intervals below `EDGES_US[i]`, the last one everything longer.
pub const EDGES_US: [u32; 8] = [500, 1000, 2000, 4000, 8000, 16000, 32000, 64000];
/// A burst is a gap of at least this, then [`BURST_MIN`] or more reports each within [`BURST_SPACING_NS`] of the one before: a
/// Bluetooth link in sniff mode holds reports between its wake-ups and hands them over together.
pub const BURST_GAP_NS: u64 = 20_000_000;
pub const BURST_MIN: u32 = 3;
pub const BURST_SPACING_NS: u64 = 1_000_000;
const CAP: usize = 4096;

/// A pad's report stamps over the last [`WINDOW_NS`].
#[derive(Debug, Clone, Default)]
pub struct Timing {
    stamps: VecDeque<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Summary {
    /// Reports in the window.
    pub reports: u32,
    /// Reports per second of use, pauses left out.
    pub rate_hz: f32,
    pub p50_us: u32,
    pub p95_us: u32,
    pub p99_us: u32,
    pub max_us: u32,
    /// The mean change from one interval to the next.
    pub jitter_us: u32,
    pub histogram: [u32; EDGES_US.len() + 1],
    pub bursts: Bursts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bursts {
    pub count: u32,
    /// The median gap before a burst, µs.
    pub gap_us: u32,
    /// The median reports in one.
    pub size: u32,
}

impl Timing {
    pub fn push(&mut self, kernel_ns: u64) {
        if self.stamps.back().is_some_and(|&last| kernel_ns < last) {
            return;
        }
        let from = kernel_ns.saturating_sub(WINDOW_NS);
        while self.stamps.front().is_some_and(|&s| s < from) || self.stamps.len() >= CAP {
            self.stamps.pop_front();
        }
        self.stamps.push_back(kernel_ns);
    }

    pub fn clear(&mut self) {
        self.stamps.clear();
    }

    /// The figures for the window that ends at `now_ns`.
    pub fn summary(&self, now_ns: u64) -> Summary {
        let from = now_ns.saturating_sub(WINDOW_NS);
        let stamps: Vec<u64> = self.stamps.iter().copied().filter(|&s| s >= from).collect();
        let gaps: Vec<u64> = stamps.windows(2).map(|w| w[1] - w[0]).collect();
        let active: Vec<u64> = gaps.iter().copied().filter(|&d| d < IDLE_NS).collect();
        let mut s = Summary { reports: stamps.len() as u32, bursts: bursts(&gaps), ..Summary::default() };
        if active.is_empty() {
            return s;
        }
        let busy_ns: u64 = active.iter().sum();
        s.rate_hz = (active.len() as f64 * 1e9 / busy_ns.max(1) as f64) as f32;
        let mut sorted: Vec<u32> = active.iter().map(|&d| us(d)).collect();
        for &v in &sorted {
            s.histogram[EDGES_US.iter().position(|&edge| v < edge).unwrap_or(EDGES_US.len())] += 1;
        }
        sorted.sort_unstable();
        (s.p50_us, s.p95_us, s.p99_us, s.max_us) = (rank(&sorted, 50), rank(&sorted, 95), rank(&sorted, 99), sorted[sorted.len() - 1]);
        let steps: Vec<u64> = gaps.windows(2).filter(|w| w[0] < IDLE_NS && w[1] < IDLE_NS).map(|w| w[0].abs_diff(w[1])).collect();
        if !steps.is_empty() {
            s.jitter_us = us(steps.iter().sum::<u64>() / steps.len() as u64);
        }
        s
    }
}

fn us(ns: u64) -> u32 {
    (ns / 1000).min(u64::from(u32::MAX)) as u32
}

/// Nearest rank, the rule [`crate::latency::Stat::of`] uses.
fn rank(sorted: &[u32], p: usize) -> u32 {
    sorted[(p * sorted.len()).div_ceil(100).clamp(1, sorted.len()) - 1]
}

fn bursts(gaps: &[u64]) -> Bursts {
    let (mut before, mut sizes) = (Vec::new(), Vec::new());
    for (i, &gap) in gaps.iter().enumerate() {
        if gap < BURST_GAP_NS {
            continue;
        }
        let size = 1 + gaps[i + 1..].iter().take_while(|&&d| d < BURST_SPACING_NS).count() as u32;
        if size >= BURST_MIN {
            before.push(us(gap));
            sizes.push(size);
        }
    }
    if before.is_empty() {
        return Bursts::default();
    }
    before.sort_unstable();
    sizes.sort_unstable();
    Bursts { count: before.len() as u32, gap_us: rank(&before, 50), size: rank(&sizes, 50) }
}

/// Within this of where it stopped (of full travel), a stick is still: a 0..255 axis's last bit is 0.8 %.
pub const STILL: f32 = 0.01;
/// Still this long, a stick is at rest. A resting stick sends no reports, so time is passed in by [`Drift::tick`].
pub const SETTLE_NS: u64 = 300_000_000;
/// Only a rest this close to the centre is the stick's own; one further out is a thumb holding it.
pub const NEAR: f32 = 0.30;

/// Where a stick comes to rest after it is let go: how far off the centre it sits, now and at worst, which is what the dead zone
/// has to cover.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Drift {
    at: (f32, f32),
    since_ns: Option<u64>,
    settled: bool,
    /// The last rest's distance from the centre, of full travel.
    pub rest: Option<f32>,
    pub worst: f32,
}

impl Drift {
    pub fn report(&mut self, pos: (f32, f32), kernel_ns: u64) {
        if self.since_ns.is_none() || (pos.0 - self.at.0).hypot(pos.1 - self.at.1) >= STILL {
            self.at = pos;
            self.since_ns = Some(kernel_ns);
            self.settled = false;
        }
    }

    /// Settles the stick once it has been still for [`SETTLE_NS`] by `now_ns`; true when that changed the readout.
    pub fn tick(&mut self, now_ns: u64) -> bool {
        let Some(since) = self.since_ns else { return false };
        let off = self.at.0.hypot(self.at.1);
        if self.settled || now_ns.saturating_sub(since) < SETTLE_NS || off > NEAR {
            return false;
        }
        self.settled = true;
        self.rest = Some(off);
        self.worst = self.worst.max(off);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    #[test]
    fn a_steady_pad_reads_its_rate_and_no_jitter_and_pauses_do_not_count() {
        let mut t = Timing::default();
        let mut at = 10_000 * MS;
        for _ in 0..100 {
            t.push(at);
            at += 4 * MS;
        }
        at += 900 * MS;
        for _ in 0..100 {
            t.push(at);
            at += 4 * MS;
        }
        let s = t.summary(at);
        assert_eq!(s.reports, 200);
        assert!((s.rate_hz - 250.0).abs() < 0.01, "{s:?}");
        assert_eq!((s.p50_us, s.p95_us, s.p99_us, s.max_us, s.jitter_us), (4000, 4000, 4000, 4000, 0));
        assert_eq!(s.histogram[4], 198, "4 ms is in the 4-8 ms bucket");
        assert_eq!(s.bursts, Bursts::default());
        assert_eq!(t.summary(at + WINDOW_NS).reports, 0, "the window moves on");
    }

    #[test]
    fn sniff_mode_bunching_is_counted_as_bursts() {
        let mut t = Timing::default();
        for k in 0..10 {
            for j in 0..4 {
                t.push(5_000 * MS + k * 77 * MS + j * 400_000);
            }
        }
        let s = t.summary(6_000 * MS);
        assert_eq!(s.bursts, Bursts { count: 9, gap_us: 75_800, size: 4 });
        assert_eq!(s.max_us, 75_800);
        assert_eq!((s.histogram[0], s.histogram[8]), (30, 9), "bunched below 0.5 ms, gaps above 64 ms");
        assert!(s.jitter_us > 30_000, "{s:?}");
    }

    #[test]
    fn a_let_go_stick_settles_near_the_centre_and_keeps_its_worst() {
        let mut d = Drift::default();
        assert!(!d.tick(SETTLE_NS));
        d.report((0.8, 0.0), 0);
        assert!(!d.tick(SETTLE_NS * 2), "held far out by a thumb");
        d.report((0.03, -0.04), SETTLE_NS * 2);
        d.report((0.035, -0.04), SETTLE_NS * 2 + 10 * MS);
        assert!(!d.tick(SETTLE_NS * 3 - MS));
        assert!(d.tick(SETTLE_NS * 3));
        assert!((d.rest.unwrap() - 0.05).abs() < 1e-6);
        assert!(!d.tick(SETTLE_NS * 4), "settles once");
        d.report((0.0, 0.01), SETTLE_NS * 5);
        assert!(d.tick(SETTLE_NS * 6));
        assert_eq!(d.rest, Some(0.01));
        assert!((d.worst - 0.05).abs() < 1e-6);
    }
}
