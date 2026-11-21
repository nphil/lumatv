//! Even hand-over for video frames that reach the TV in bunches.
//!
//! The NDL player shows each unit as it is handed over (it has no clock, gf_ndl.c), so the panel's fixed 120 Hz grid shows frames in
//! the rhythm they arrive. With frame generation the seat sends frames bunched: in a Cyberpunk 2077 session at 95-111 fps the arrival
//! gaps ran p50 4.5-9 ms and p95 16-24 ms while the network's own jitter stayed at 0.1-0.3 ms, and the panel turned that into
//! alternating one- and two-refresh frames. Smooth pacing hands frames over no closer together than the stream's mean period and never
//! more than 1.5 frame times after they arrived: frames that come evenly go at once, and only those a bunch brought early wait.

use std::collections::VecDeque;

/// Gaps that measure the period: an even count, so the short and long gaps of bunched pairs cancel exactly.
const GAPS: usize = 16;
/// A gap this many periods long is a stall, not a bunch: the measurement starts again.
const STALL_PERIODS: i64 = 4;
const SECOND_NS: i64 = 1_000_000_000;

pub(crate) struct Pacer {
    smooth: bool,
    nominal_ns: i64,
    cap_ns: i64,
    clock_hz: u32,
    /// Arrival time and RTP stamp of the latest frames, oldest first.
    recent: VecDeque<(i64, u32)>,
    last_release: i64,
    second: Second,
    report: Option<Report>,
}

impl Pacer {
    pub(crate) fn new(fps: u32, smooth: bool) -> Self {
        let nominal_ns = SECOND_NS / i64::from(fps.max(1));
        Self {
            smooth,
            nominal_ns,
            cap_ns: nominal_ns * 3 / 2,
            clock_hz: 0,
            recent: VecDeque::with_capacity(GAPS + 1),
            last_release: 0,
            second: Second::default(),
            report: None,
        }
    }

    /// When the frame that arrived at `now` goes to the decoder. A keyframe or the first frame after a gap in the sequence
    /// (`resync`) goes at once and starts a new grid, as does the first frame after a stall.
    pub(crate) fn schedule(&mut self, now: i64, stamp: u64, clock_hz: u32, resync: bool) -> i64 {
        let stamp = stamp as u32; /* RTP stamps wrap at 32 bits */
        self.clock_hz = clock_hz;
        if let Some(&(at, previous)) = self.recent.back() {
            self.second.gap(now - at, stamp.wrapping_sub(previous), clock_hz);
            if resync || now - at > STALL_PERIODS * self.period().max(self.nominal_ns) {
                self.recent.clear();
            }
        }
        let release = if self.smooth && !self.recent.is_empty() {
            // 2% under the period, so the delay a bunch leaves behind drains instead of staying.
            (self.last_release + self.period() * 49 / 50).clamp(now, now + self.cap_ns)
        } else {
            now
        };
        if self.recent.len() > GAPS {
            self.recent.pop_front();
        }
        self.recent.push_back((now, stamp));
        self.last_release = release;
        if let Some(report) = self.second.add(now, release - now) {
            self.report = Some(report);
        }
        release
    }

    pub(crate) const fn smooth(&self) -> bool {
        self.smooth
    }

    /// The last full second's cadence, once.
    pub(crate) fn take_report(&mut self) -> Option<Report> {
        self.report.take()
    }

    /// The mean frame period over the newest even number of gaps. It is taken on the seat's clock when the RTP stamps keep time
    /// with the arrivals (within 10%), since network jitter does not move them, and on the arrivals otherwise.
    fn period(&self) -> i64 {
        let gaps = self.recent.len().saturating_sub(1) & !1;
        if gaps < 2 {
            return self.nominal_ns;
        }
        let (first, first_stamp) = self.recent[self.recent.len() - 1 - gaps];
        let Some(&(last, last_stamp)) = self.recent.back() else { return self.nominal_ns };
        let arrived = last - first;
        let stamped = match self.clock_hz {
            0 => 0,
            hz => i64::from(last_stamp.wrapping_sub(first_stamp)) * SECOND_NS / i64::from(hz),
        };
        let span = if (stamped - arrived).abs() * 10 <= arrived { stamped } else { arrived };
        span / gaps as i64
    }
}

/// One second of cadence, for the stats: the pacing delay and the frame spacing as stamped and as arrived.
#[derive(Default)]
struct Second {
    started: Option<i64>,
    arrival_us: Vec<u32>,
    host_us: Vec<u32>,
    /// A frame carried the stamp of the one before, so the stamps do not tell frames apart and show no spacing.
    shared_stamps: bool,
    delay_ns: i64,
    frames: u32,
}

impl Second {
    fn gap(&mut self, arrived_ns: i64, ticks: u32, clock_hz: u32) {
        self.arrival_us.push(u32::try_from(arrived_ns / 1000).unwrap_or(0));
        if ticks == 0 || clock_hz == 0 {
            self.shared_stamps = true;
        } else {
            self.host_us.push(u32::try_from(u64::from(ticks) * 1_000_000 / u64::from(clock_hz)).unwrap_or(u32::MAX));
        }
    }

    fn add(&mut self, now: i64, delay_ns: i64) -> Option<Report> {
        let started = *self.started.get_or_insert(now);
        self.delay_ns += delay_ns;
        self.frames += 1;
        if now - started < SECOND_NS {
            return None;
        }
        let report = Report {
            pacing_ms: self.delay_ns as f32 / self.frames as f32 / 1e6,
            host_gap_ms: if self.shared_stamps { None } else { spread(&mut self.host_us) },
            arrival_gap_ms: spread(&mut self.arrival_us),
        };
        self.started = Some(now);
        self.arrival_us.clear();
        self.host_us.clear();
        self.shared_stamps = false;
        self.delay_ns = 0;
        self.frames = 0;
        Some(report)
    }
}

/// Median and 95th percentile, in milliseconds.
fn spread(us: &mut [u32]) -> Option<[f32; 2]> {
    if us.is_empty() {
        return None;
    }
    us.sort_unstable();
    let at = |q: usize| us[(us.len() - 1) * q / 100] as f32 / 1000.0;
    Some([at(50), at(95)])
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Report {
    pacing_ms: f32,
    host_gap_ms: Option<[f32; 2]>,
    arrival_gap_ms: Option<[f32; 2]>,
}

impl Report {
    /// Adds the second to the streamer's telemetry event, under the event's own camelCase names.
    pub(crate) fn add_to(&self, telemetry: &mut serde_json::Value) {
        if let Some(t) = telemetry.as_object_mut() {
            t.insert("pacingMs".to_owned(), serde_json::json!(self.pacing_ms));
            t.insert("hostGapMs".to_owned(), serde_json::json!(self.host_gap_ms));
            t.insert("arrivalGapMs".to_owned(), serde_json::json!(self.arrival_gap_ms));
        }
    }
}

/// The viewer's Frame pacing choice, from the runtime prefs (`LUMATV_GFN_PREFS`) as each stream starts: smooth unless "latency".
pub(crate) fn smooth_from_prefs() -> bool {
    let Some(path) = std::env::var_os("LUMATV_GFN_PREFS") else { return true };
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_none_or(|v| v["framePacing"] != "latency")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: i64 = 1_000_000;

    /// Pairs 1 ms apart at `fps` frames a second, as frame generation arrives.
    fn pairs(fps: i64, frames: i64) -> Vec<i64> {
        (0..frames).map(|i| i / 2 * 2 * (SECOND_NS / fps) + i % 2 * MS).collect()
    }

    /// Releases for `arrivals`, stamped on a 90 kHz clock at their send times.
    fn feed(pacer: &mut Pacer, arrivals: &[i64]) -> Vec<i64> {
        arrivals.iter().map(|&at| pacer.schedule(at, (at * 9 / 100_000) as u64, 90_000, false)).collect()
    }

    #[test]
    fn bunched_pairs_come_out_even() {
        let period = SECOND_NS / 105;
        let arrivals = pairs(105, 400);
        let mut pacer = Pacer::new(120, true);
        let released = feed(&mut pacer, &arrivals);
        for (k, w) in released.windows(2).enumerate().skip(GAPS) {
            assert!((w[1] - w[0] - period).abs() < period / 20, "gap {k}: {} ns", w[1] - w[0]);
        }
        assert!(released.iter().zip(&arrivals).all(|(r, a)| (0..=pacer.cap_ns).contains(&(r - a))));
    }

    #[test]
    fn bursts_wait_no_longer_than_the_cap() {
        let period = SECOND_NS / 120;
        let arrivals: Vec<i64> = (0..400).map(|i| i / 4 * 4 * period).collect();
        let mut pacer = Pacer::new(120, true);
        let released = feed(&mut pacer, &arrivals);
        let delays: Vec<i64> = released.iter().zip(&arrivals).map(|(r, a)| r - a).collect();
        assert_eq!(delays.iter().max(), Some(&pacer.cap_ns));
        assert!(delays.iter().all(|d| *d >= 0));
        assert!(released.windows(2).all(|w| w[0] <= w[1]), "frames stay in order");
    }

    #[test]
    fn keyframes_gaps_and_stalls_start_a_new_grid() {
        let mut pacer = Pacer::new(120, true);
        let arrivals = pairs(105, 40);
        feed(&mut pacer, &arrivals);
        let early = arrivals[39] + MS;
        assert_eq!(pacer.schedule(early, 0, 90_000, true), early, "a keyframe or a gap goes at once");
        assert_eq!(pacer.schedule(early + MS, 0, 90_000, false), early + pacer.nominal_ns * 49 / 50, "the next is on the new grid");
        let stalled = early + 100 * MS;
        assert_eq!(pacer.schedule(stalled, 0, 90_000, false), stalled);
        assert_eq!(pacer.recent.len(), 1, "the measurement restarts after a stall");
    }

    #[test]
    fn ten_minutes_of_bunched_frames_build_no_delay() {
        // 105 fps in pairs with up to 0.5 ms of jitter, on a seat clock 100 ppm fast: 63,000 frames.
        let mut seed = 1u32;
        let mut pacer = Pacer::new(120, true);
        let mut delays = Vec::new();
        for sent in pairs(105, 105 * 600) {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let at = sent + sent / 10_000 + i64::from(seed >> 23) * MS / 1024;
            delays.push(pacer.schedule(at, (sent * 9 / 100_000) as u64, 90_000, false) - at);
        }
        let mean = |d: &[i64]| d.iter().sum::<i64>() / d.len() as i64;
        let (first, last) = (mean(&delays[GAPS..1050]), mean(&delays[delays.len() - 1050..]));
        assert!(delays.iter().all(|d| (0..=pacer.cap_ns).contains(d)));
        assert!((first - last).abs() < MS / 2, "first 10 s {first} ns, last 10 s {last} ns");
        assert!(last < 5 * MS);
    }

    #[test]
    fn the_report_tells_host_spacing_from_arrival_spacing_and_lowest_latency_waits_for_nothing() {
        let mut pacer = Pacer::new(120, false);
        for (i, at) in pairs(60, 130).into_iter().enumerate() {
            assert_eq!(pacer.schedule(at, i as u64 * 1500, 90_000, false), at);
        }
        let report = pacer.take_report().unwrap();
        assert_eq!(report.pacing_ms, 0.0);
        assert_eq!(report.host_gap_ms, Some([16.666, 16.666]), "evenly stamped");
        assert_eq!(report.arrival_gap_ms.unwrap()[1], 32.333, "bunched on arrival");
    }
}
