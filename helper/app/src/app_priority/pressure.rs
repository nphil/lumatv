//! When memory is short enough to close a cached app: Android lmkd's decision (`system/memory/lmkd/lmkd.cpp`, `mp_event_psi`), fed by
//! one sample a second of the same counters it reads.

use std::time::{Duration, Instant};

/// lmkd's thresholds. The values are its low-RAM defaults (lmkd README: `ro.config.low_ram`), which Android applies to the 2 GB class
/// this set is: of its 1983 MB, 676 MB is LG's CMA reserve for the video decoders.
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// `ro.lmk.psi_partial_stall_ms`: some task stalled on memory this long in a second is medium pressure (high-end default 70).
    pub partial_stall_ms: u64,
    /// `ro.lmk.psi_complete_stall_ms`: every task stalled this long in a second is critical pressure.
    pub complete_stall_ms: u64,
    /// `ro.lmk.thrashing_limit`: file pages refaulted in a second, as a percentage of the file cache (high-end default 100).
    pub thrashing_limit: u64,
    /// `ro.lmk.thrashing_limit_decay`: how much lower the limit goes after each thrashing kill in the same window (high-end 10).
    pub thrashing_decay: u64,
    /// `ro.lmk.swap_free_low_percentage`: swap below this share of its size is low.
    pub swap_free_low_pct: u64,
    /// Seconds below half the partial threshold before an episode ends (not lmkd's: it gates the reclaim before a close).
    pub calm_windows: u32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            partial_stall_ms: 200,
            complete_stall_ms: 700,
            thrashing_limit: 30,
            thrashing_decay: 50,
            swap_free_low_pct: 10,
            calm_windows: 3,
        }
    }
}

/// lmkd's `THRASHING_RESET_INTERVAL_MS`.
const THRASHING_WINDOW: Duration = Duration::from_secs(1);

/// The counters of one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sample {
    /// `/proc/pressure/memory` `some` and `full` `total=`, in µs.
    pub some_us: u64,
    pub full_us: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
    /// Free pages less free CMA pages, which no ordinary allocation can use.
    pub free_pages: u64,
    /// lmkd's high watermark: every present zone's `high` plus its largest `protection`.
    pub high_wmark_pages: u64,
    /// `workingset_refault_file` (`workingset_refault` before 5.9).
    pub refault_file: u64,
    /// `nr_inactive_file` + `nr_active_file`.
    pub file_lru_pages: u64,
    pub pgscan_direct: u64,
}

/// Fills a sample from `/proc/pressure/memory`, `/proc/meminfo`, `/proc/vmstat` and `/proc/zoneinfo`.
pub fn sample(psi: &str, meminfo: &str, vmstat: &str, zoneinfo: &str) -> Option<Sample> {
    let total = |prefix: &str| {
        psi.lines()
            .find(|l| l.starts_with(prefix))?
            .split_whitespace()
            .find_map(|f| f.strip_prefix("total="))?
            .parse::<u64>()
            .ok()
    };
    let kb = |key: &str| {
        meminfo
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
            .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let vm = |key: &str| {
        vmstat.lines().find_map(|l| {
            let (k, v) = l.split_once(' ')?;
            (k == key).then(|| v.trim().parse::<u64>().ok()).flatten()
        })
    };
    let page_kb = 4;
    Some(Sample {
        some_us: total("some")?,
        full_us: total("full")?,
        swap_total_kb: kb("SwapTotal"),
        swap_free_kb: kb("SwapFree"),
        free_pages: kb("MemFree").saturating_sub(kb("CmaFree")) / page_kb,
        high_wmark_pages: high_watermark(zoneinfo),
        refault_file: vm("workingset_refault_file")
            .or_else(|| vm("workingset_refault"))
            .unwrap_or(0),
        file_lru_pages: vm("nr_inactive_file").unwrap_or(0) + vm("nr_active_file").unwrap_or(0),
        pgscan_direct: vm("pgscan_direct").unwrap_or(0),
    })
}

/// lmkd's `calc_zone_watermarks`: the sum over present zones of `high` and the zone's largest `protection`.
fn high_watermark(zoneinfo: &str) -> u64 {
    let (mut sum, mut present, mut high, mut protection) = (0, 0u64, 0u64, 0u64);
    let mut flush = |present: u64, high: u64, protection: u64| {
        if present > 0 {
            sum += high + protection;
        }
    };
    let mut in_zone = false;
    for line in zoneinfo.lines() {
        if line.starts_with("Node ") {
            if in_zone {
                flush(present, high, protection);
            }
            (in_zone, present, high, protection) = (true, 0, 0, 0);
            continue;
        }
        let line = line.trim();
        if let Some(list) = line.strip_prefix("protection:") {
            protection = list
                .trim_matches(|c: char| c == ' ' || c == '(' || c == ')')
                .split(',')
                .filter_map(|v| v.trim().parse::<u64>().ok())
                .max()
                .unwrap_or(0);
        } else if let Some((key, value)) = line.split_once(char::is_whitespace) {
            let value = value.trim().parse::<u64>().unwrap_or(0);
            match key {
                "present" => present = value,
                "high" => high = value,
                _ => {}
            }
        }
    }
    if in_zone {
        flush(present, high, protection);
    }
    sum
}

/// What a second of pressure asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// No pressure, or not enough of it to act.
    Calm,
    /// Medium pressure without a reason to close: reclaim a cached app's memory first.
    Trim,
    /// Close a cached app, for lmkd's reason.
    Kill(Reason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Complete stall over the critical threshold (lmkd `NOT_RESPONDING`).
    Stall,
    LowSwapAndThrashing,
    LowMemAndSwap,
    LowMemAndThrashing,
    DirectReclaimAndThrashing,
}

impl Reason {
    pub fn text(self) -> &'static str {
        match self {
            Reason::Stall => "memory stall",
            Reason::LowSwapAndThrashing => "low swap and thrashing",
            Reason::LowMemAndSwap => "low memory and swap",
            Reason::LowMemAndThrashing => "low memory and thrashing",
            Reason::DirectReclaimAndThrashing => "direct reclaim and thrashing",
        }
    }

    fn thrashing(self) -> bool {
        matches!(self, Reason::LowMemAndThrashing | Reason::DirectReclaimAndThrashing)
    }
}

/// The running state of the decision across samples.
pub struct Watch {
    tuning: Tuning,
    last: Option<(Instant, Sample)>,
    /// Medium or critical pressure seen, and not yet `calm_windows` quiet seconds since.
    pub episode: bool,
    calm: u32,
    /// Consecutive seconds at medium pressure or above.
    pub streak: u32,
    init_refault: u64,
    base_lru: u64,
    reset_at: Option<Instant>,
    prev_thrash: u64,
    limit: u64,
    /// The last second's stall and thrashing, for the topic.
    pub some_ms: u64,
    pub full_ms: u64,
    pub thrashing: u64,
}

impl Watch {
    pub fn new(tuning: Tuning) -> Self {
        Self {
            tuning,
            last: None,
            episode: false,
            calm: 0,
            streak: 0,
            init_refault: 0,
            base_lru: 0,
            reset_at: None,
            prev_thrash: 0,
            limit: tuning.thrashing_limit,
            some_ms: 0,
            full_ms: 0,
            thrashing: 0,
        }
    }

    /// Forgets the previous sample: sampling stopped, and the next gap is no measure of anything.
    pub fn pause(&mut self) {
        self.last = None;
        self.reset_at = None;
        self.streak = 0;
    }

    pub fn step(&mut self, now: Instant, s: Sample) -> Verdict {
        let Some((then, before)) = self.last.replace((now, s)) else {
            self.reset_thrashing(now, &s);
            return Verdict::Calm;
        };
        let elapsed_ms = now.saturating_duration_since(then).as_millis().max(1) as u64;
        let per_second = |us: u64| us / 1000 * 1000 / elapsed_ms;
        self.some_ms = per_second(s.some_us.saturating_sub(before.some_us));
        self.full_ms = per_second(s.full_us.saturating_sub(before.full_us));
        self.thrashing = self.thrashing_now(now, &s);

        let t = self.tuning;
        let critical = self.full_ms >= t.complete_stall_ms;
        let medium = critical || self.some_ms >= t.partial_stall_ms;
        if medium {
            (self.episode, self.calm) = (true, 0);
            self.streak += 1;
        } else {
            self.streak = 0;
            if self.episode && self.some_ms < t.partial_stall_ms / 2 {
                self.calm += 1;
                if self.calm >= t.calm_windows {
                    (self.episode, self.calm) = (false, 0);
                }
            } else {
                self.calm = 0;
            }
            return Verdict::Calm;
        }
        if critical {
            return Verdict::Kill(Reason::Stall);
        }
        let swap_low = s.swap_total_kb > 0 && s.swap_free_kb * 100 < s.swap_total_kb * t.swap_free_low_pct;
        let below_high = s.free_pages < s.high_wmark_pages;
        let thrashing = self.thrashing > self.limit;
        let direct = s.pgscan_direct > before.pgscan_direct;
        if swap_low && thrashing {
            Verdict::Kill(Reason::LowSwapAndThrashing)
        } else if swap_low && below_high {
            Verdict::Kill(Reason::LowMemAndSwap)
        } else if below_high && thrashing {
            Verdict::Kill(Reason::LowMemAndThrashing)
        } else if direct && thrashing {
            Verdict::Kill(Reason::DirectReclaimAndThrashing)
        } else {
            Verdict::Trim
        }
    }

    /// A close was made for `reason`: as lmkd, a thrashing close lowers the limit for the rest of its window.
    pub fn closed(&mut self, reason: Reason) {
        if reason.thrashing() {
            self.limit = self.limit * (100 - self.tuning.thrashing_decay) / 100;
        }
    }

    /// lmkd's thrashing: refaults since the window began over the file cache at its start, plus the previous window's, halved for
    /// every window passed unless that one was over the limit.
    fn thrashing_now(&mut self, now: Instant, s: &Sample) -> u64 {
        let Some(reset_at) = self.reset_at else {
            self.reset_thrashing(now, s);
            return 0;
        };
        let since = now.saturating_duration_since(reset_at);
        let growth = s.refault_file.saturating_sub(self.init_refault) * 100 / (self.base_lru + 1);
        if since >= THRASHING_WINDOW {
            let windows = (since.as_millis() / THRASHING_WINDOW.as_millis()) as u32;
            self.prev_thrash = growth;
            if windows > 1 || growth < self.tuning.thrashing_limit {
                self.prev_thrash = self.prev_thrash.checked_shr(windows).unwrap_or(0);
            }
            self.reset_thrashing(now, s);
            self.prev_thrash
        } else {
            growth + self.prev_thrash
        }
    }

    fn reset_thrashing(&mut self, now: Instant, s: &Sample) {
        self.base_lru = s.file_lru_pages;
        self.init_refault = s.refault_file;
        self.reset_at = Some(now);
        self.limit = self.tuning.thrashing_limit;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PSI: &str = "some avg10=0.00 avg60=0.00 avg300=0.03 total=3174510\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=311766\n";
    const MEMINFO: &str = "MemTotal:        2030392 kB\nMemFree:           53632 kB\nSwapTotal:        614396 kB\nSwapFree:         343088 kB\nCmaTotal:         692224 kB\nCmaFree:            6544 kB\n";
    const VMSTAT: &str = "nr_inactive_file 43489\nnr_active_file 51838\nworkingset_refault_anon 950\nworkingset_refault_file 22966\npgscan_direct 85104\npgscan_direct_throttle 0\n";
    const ZONEINFO: &str = "Node 0, zone    DMA32\n  pages free     11784\n        min      1349\n        low      1686\n        high     2023\n        present  355328\n        protection: (0, 0, 5408)\nNode 0, zone   Normal\n  pages free     0\n        high     0\n        present  0\n        protection: (0, 0, 5408)\nNode 0, zone  Movable\n  pages free     1624\n        high     1046\n        present  173056\n        protection: (0, 0, 0)\n";

    fn base() -> Sample {
        Sample {
            swap_total_kb: 600_000,
            swap_free_kb: 400_000,
            free_pages: 20_000,
            high_wmark_pages: 8_000,
            file_lru_pages: 100_000,
            ..Sample::default()
        }
    }

    /// Feeds one second with `some`/`full` ms of stall and `refaults` file refaults after a first quiet sample.
    fn second(w: &mut Watch, t0: Instant, n: u32, prev: &mut Sample, f: impl Fn(&mut Sample)) -> Verdict {
        let mut s = *prev;
        f(&mut s);
        *prev = s;
        w.step(t0 + Duration::from_secs(n as u64), s)
    }

    #[test]
    fn the_tvs_counters_read_as_lmkd_reads_them() {
        let s = sample(PSI, MEMINFO, VMSTAT, ZONEINFO).unwrap();
        assert_eq!((s.some_us, s.full_us), (3_174_510, 311_766));
        assert_eq!((s.swap_total_kb, s.swap_free_kb), (614_396, 343_088));
        assert_eq!(s.free_pages, (53_632 - 6_544) / 4);
        assert_eq!(s.high_wmark_pages, 2023 + 5408 + 1046);
        assert_eq!((s.refault_file, s.file_lru_pages, s.pgscan_direct), (22_966, 95_327, 85_104));
        assert_eq!(sample("", MEMINFO, VMSTAT, ZONEINFO), None);
    }

    #[test]
    fn stall_levels_follow_lmkds_thresholds() {
        let t0 = Instant::now();
        let mut w = Watch::new(Tuning::default());
        let mut s = base();
        assert_eq!(w.step(t0, s), Verdict::Calm);
        assert_eq!(second(&mut w, t0, 1, &mut s, |s| s.some_us += 150_000), Verdict::Calm);
        assert!(!w.episode);
        assert_eq!(second(&mut w, t0, 2, &mut s, |s| s.some_us += 250_000), Verdict::Trim);
        assert_eq!((w.some_ms, w.episode, w.streak), (250, true, 1));
        assert_eq!(
            second(&mut w, t0, 3, &mut s, |s| {
                s.some_us += 800_000;
                s.full_us += 700_000
            }),
            Verdict::Kill(Reason::Stall)
        );
        assert_eq!(w.streak, 2);
    }

    #[test]
    fn medium_pressure_closes_only_with_lmkds_second_reason() {
        let t0 = Instant::now();
        let mut w = Watch::new(Tuning::default());
        let mut s = base();
        w.step(t0, s);
        // Swap at 9 % of its size and the free pages under the high watermark.
        assert_eq!(
            second(&mut w, t0, 1, &mut s, |s| {
                s.some_us += 300_000;
                s.swap_free_kb = 54_000;
                s.free_pages = 7_000
            }),
            Verdict::Kill(Reason::LowMemAndSwap)
        );
        // Plenty of swap again; 32 % of the file cache refaulted in the second, with the free pages still low.
        assert_eq!(
            second(&mut w, t0, 2, &mut s, |s| {
                s.some_us += 300_000;
                s.swap_free_kb = 400_000;
                s.refault_file += 32_000
            }),
            Verdict::Kill(Reason::LowMemAndThrashing)
        );
        // The same refaults with free memory above the watermark and no direct reclaim: only a trim.
        assert_eq!(
            second(&mut w, t0, 3, &mut s, |s| {
                s.some_us += 300_000;
                s.free_pages = 20_000;
                s.refault_file += 32_000
            }),
            Verdict::Trim
        );
        assert_eq!(
            second(&mut w, t0, 4, &mut s, |s| {
                s.some_us += 300_000;
                s.refault_file += 32_000;
                s.pgscan_direct += 64
            }),
            Verdict::Kill(Reason::DirectReclaimAndThrashing)
        );
    }

    #[test]
    fn a_thrashing_close_lowers_the_limit_until_the_window_turns() {
        let t0 = Instant::now();
        let mut w = Watch::new(Tuning::default());
        let s = base();
        w.step(t0, s);
        w.closed(Reason::LowMemAndThrashing);
        assert_eq!(w.limit, 15);
        w.closed(Reason::Stall);
        assert_eq!(w.limit, 15);
        w.step(t0 + Duration::from_secs(1), s);
        assert_eq!(w.limit, 30);
    }

    #[test]
    fn an_episode_ends_after_three_quiet_seconds_and_not_on_a_dip() {
        let t0 = Instant::now();
        let mut w = Watch::new(Tuning::default());
        let mut s = base();
        w.step(t0, s);
        second(&mut w, t0, 1, &mut s, |s| s.some_us += 300_000);
        assert!(w.episode);
        // 150 ms is under the threshold but over half of it: no quiet second.
        second(&mut w, t0, 2, &mut s, |s| s.some_us += 150_000);
        for n in 3..5 {
            second(&mut w, t0, n, &mut s, |_| {});
            assert!(w.episode);
        }
        second(&mut w, t0, 5, &mut s, |_| {});
        assert!(!w.episode);
    }
}
