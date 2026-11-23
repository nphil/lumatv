//! The numbers the task manager shows: per-core CPU load, memory, the GPU's load, the temperature and the clock.
//!
//! They are read only while the task manager is open (the [`Sampler`] is started by its opening and stopped by its closing, and nothing
//! else calls it), once at the start for a baseline, again after [`FIRST`] and then every [`EVERY`]. Every source is a small text file of
//! the kernel or of LG's power driver; all of them together took [`Sampler::micros`] to read and parse, which `ctl switcher` reports so a
//! cost can be read off the set, and which does not grow while the task manager is closed.

use std::fs;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The first reading after the baseline, so the task manager has numbers within half a second of opening.
pub const FIRST: Duration = Duration::from_millis(250);
/// The reading rate after that.
pub const EVERY: Duration = Duration::from_secs(1);

const CPU: &str = "/proc/stat";
const MEM: &str = "/proc/meminfo";
/// The Mali driver's cumulative busy and idle time (debugfs; root reads it).
const GPU: &str = "/sys/kernel/debug/mali0/dvfs_utilization";
/// LG's power driver: the SoC temperature in degrees Celsius and the CPU clock in kHz.
const TEMP: &str = "/proc/lg/pm/temperature";
const FREQ: &str = "/proc/lg/pm/frequency";
/// eplmanager's state file: its first word is `2` while the panel's pixel refresher runs (the rule of `tv-cutover.sh` and the helper's
/// `GET /v1/panel/refresher`).
pub const PNWASH: &str = "/mnt/lg/cmn_data/pnwash/state";

/// One reading of everything.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    /// Each core's load since the reading before, 0 to 100, in core order.
    pub cpu: Vec<u8>,
    pub total_kb: u64,
    pub avail_kb: u64,
    pub cached_kb: u64,
    /// The GPU's load since the reading before; `None` where the driver does not say.
    pub gpu: Option<u8>,
    pub temp_c: Option<i32>,
    pub mhz: Option<u32>,
    /// The panel's pixel refresher is running.
    pub refresher: bool,
}

impl Stats {
    /// The `stats` topic's body (without its sequence number).
    pub fn json(&self, n: u64) -> Value {
        json!({
            "n": n,
            "cpu": self.cpu,
            "mem": {"totalKb": self.total_kb, "availKb": self.avail_kb, "cachedKb": self.cached_kb},
            "gpu": self.gpu,
            "tempC": self.temp_c,
            "mhz": self.mhz,
            "refresher": self.refresher,
        })
    }
}

/// Each core's busy and total jiffies, in core order, from the text of `/proc/stat` (the `cpuN` lines; the aggregate `cpu` line is not one).
pub fn parse_cpu(text: &str) -> Vec<(u64, u64)> {
    text.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("cpu")?;
            rest.chars().next().filter(char::is_ascii_digit)?;
            let mut f = rest.split_whitespace().skip(1).map(|v| v.parse::<u64>().unwrap_or(0));
            let (user, nice, system, idle, iowait, irq, softirq, steal) =
                (f.next()?, f.next()?, f.next()?, f.next()?, f.next().unwrap_or(0), f.next().unwrap_or(0), f.next().unwrap_or(0), f.next().unwrap_or(0));
            let busy = user + nice + system + irq + softirq + steal;
            Some((busy, busy + idle + iowait))
        })
        .collect()
}

/// `MemTotal`, `MemAvailable` and `Cached` in kB, from the text of `/proc/meminfo`.
pub fn parse_meminfo(text: &str) -> (u64, u64, u64) {
    let mut found = (0, 0, 0);
    for l in text.lines() {
        let Some((key, rest)) = l.split_once(':') else { continue };
        let slot = match key {
            "MemTotal" => &mut found.0,
            "MemAvailable" => &mut found.1,
            "Cached" => &mut found.2,
            _ => continue,
        };
        *slot = rest.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0);
    }
    found
}

/// The Mali driver's `busy_time: N idle_time: M` (cumulative, in one unit).
pub fn parse_mali(text: &str) -> Option<(u64, u64)> {
    let mut words = text.split_whitespace();
    let (mut busy, mut idle) = (None, None);
    while let Some(w) = words.next() {
        match w {
            "busy_time:" => busy = words.next().and_then(|v| v.parse().ok()),
            "idle_time:" => idle = words.next().and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    Some((busy?, idle?))
}

/// The first word of the pnwash state is `2` while a run is in progress.
pub fn parse_pnwash(text: &str) -> bool {
    text.split_whitespace().next() == Some("2")
}

/// Whether the pixel refresher is running now: the one read the System menu's Restart TV makes before it goes ahead (the bridge makes it
/// again before it runs `reboot`).
pub fn refresher_running() -> bool {
    fs::read_to_string(PNWASH).is_ok_and(|t| parse_pnwash(&t))
}

fn percent(busy: u64, total: u64) -> u8 {
    (busy.min(total) * 100).checked_div(total).map_or(0, |p| p as u8)
}

/// A source's whole text into `buf`; `false` when it cannot be read (a missing file is a number the screen leaves out).
fn read_into(buf: &mut String, path: &str) -> bool {
    use std::io::Read;
    buf.clear();
    fs::File::open(path).and_then(|mut f| f.read_to_string(buf)).is_ok()
}

/// Reads the sources, remembers the last counters, answers the loads since them.
#[derive(Default)]
pub struct Sampler {
    cpu: Vec<(u64, u64)>,
    gpu: Option<(u64, u64)>,
    /// Readings of this run (since [`Sampler::start`]); the first is the baseline.
    run: u32,
    started: bool,
    last: Option<Instant>,
    text: String,
    /// How many readings were made since the process began (the baselines count), and the time they took: zero growth while the task
    /// manager is closed is the proof it costs nothing then.
    pub samples: u64,
    pub micros: u64,
    pub stats: Stats,
}

impl Sampler {
    /// The task manager opened: the baseline is read now, and the first reading is due after [`FIRST`].
    pub fn start(&mut self, now: Instant) {
        self.started = true;
        self.run = 0;
        self.stats = Stats::default();
        self.sample(now);
    }

    /// The task manager closed: nothing is read until it opens again, and no counter from this opening is kept.
    pub fn stop(&mut self) {
        self.started = false;
        self.last = None;
        self.cpu.clear();
        self.gpu = None;
    }

    /// When the next reading is due; `None` while stopped.
    pub fn deadline(&self) -> Option<Instant> {
        self.started.then_some(self.last? + if self.run >= 2 { EVERY } else { FIRST })
    }

    /// Whether a reading is due at `now`.
    pub fn due(&self, now: Instant) -> bool {
        self.deadline().is_some_and(|t| now >= t)
    }

    /// The readings are numbers worth showing: the baseline alone is not.
    pub fn ready(&self) -> bool {
        self.run >= 2
    }

    /// Reads everything. The baseline's loads are unknown, so `stats.cpu` and `stats.gpu` stay empty until the second reading.
    pub fn sample(&mut self, now: Instant) {
        let t0 = Instant::now();
        let mut text = std::mem::take(&mut self.text);
        self.last = Some(now);
        let cpu = if read_into(&mut text, CPU) { parse_cpu(&text) } else { Vec::new() };
        let loads: Vec<u8> = if self.run == 0 { Vec::new() } else { cpu.iter().zip(&self.cpu).map(|(n, o)| percent(n.0.saturating_sub(o.0), n.1.saturating_sub(o.1))).collect() };
        self.cpu = cpu;
        let (total_kb, avail_kb, cached_kb) = if read_into(&mut text, MEM) { parse_meminfo(&text) } else { (0, 0, 0) };
        let gpu_now = if read_into(&mut text, GPU) { parse_mali(&text) } else { None };
        let gpu = match (gpu_now, self.gpu) {
            (Some(n), Some(o)) => {
                let busy = n.0.saturating_sub(o.0);
                Some(percent(busy, busy + n.1.saturating_sub(o.1)))
            }
            _ => None,
        };
        self.gpu = gpu_now;
        let temp_c = if read_into(&mut text, TEMP) { text.trim().parse().ok() } else { None };
        let mhz = if read_into(&mut text, FREQ) { text.trim().parse::<u32>().ok().map(|k| k / 1000) } else { None };
        let refresher = read_into(&mut text, PNWASH) && parse_pnwash(&text);
        self.text = text;
        self.stats = Stats { cpu: loads, total_kb, avail_kb, cached_kb, gpu, temp_c, mhz, refresher };
        self.run += 1;
        self.samples += 1;
        self.micros += t0.elapsed().as_micros() as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  694396 81424 585188 7818768 2950 0 25901 0 0 0\n\
cpu0 216195 22834 187479 2394753 815 0 23039 0 0 0\n\
cpu1 202088 21121 185960 2470249 636 0 1347 0 0 0\n\
cpu2 208558 28807 166310 2268138 764 0 1117 0 0 0\n\
intr 161593105 0 17088795\n";

    #[test]
    fn cpu_lines_are_per_core_and_the_aggregate_is_not_one() {
        let cores = parse_cpu(STAT);
        assert_eq!(cores.len(), 3);
        // busy = user + nice + system + irq + softirq + steal; total adds idle and iowait
        assert_eq!(cores[0], (216195 + 22834 + 187479 + 23039, 216195 + 22834 + 187479 + 23039 + 2394753 + 815));
    }

    #[test]
    fn a_core_that_did_no_work_reads_zero_and_one_that_did_all_of_it_reads_a_hundred() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(0, 400), 0);
        assert_eq!(percent(400, 400), 100);
        // a counter that went backwards (a core came back) saturates to the total instead of passing 100
        assert_eq!(percent(500, 400), 100);
        assert_eq!(percent(1, 3), 33);
    }

    #[test]
    fn meminfo_gives_total_available_and_cached() {
        let text = "MemTotal:        2030392 kB\nMemFree:          355456 kB\nMemAvailable:     564304 kB\nBuffers:            6916 kB\nCached:           268104 kB\nSwapCached:          844 kB\n";
        assert_eq!(parse_meminfo(text), (2030392, 564304, 268104));
        assert_eq!(parse_meminfo(""), (0, 0, 0));
    }

    #[test]
    fn the_gpu_counters_parse_and_a_missing_one_is_none() {
        assert_eq!(parse_mali("busy_time: 2924761704 idle_time: 498632464"), Some((2924761704, 498632464)));
        assert_eq!(parse_mali("busy_time: 5"), None);
        assert_eq!(parse_mali(""), None);
    }

    #[test]
    fn the_refresher_runs_only_when_the_first_word_is_two() {
        assert!(parse_pnwash("2 1"));
        assert!(!parse_pnwash("0 8"));
        assert!(!parse_pnwash("1 0"));
        assert!(!parse_pnwash("8 2"));
        assert!(!parse_pnwash(""));
    }

    #[test]
    fn the_topic_names_what_the_ui_reads() {
        let s = Stats { cpu: vec![10, 20, 30], total_kb: 1000, avail_kb: 400, cached_kb: 100, gpu: Some(7), temp_c: Some(53), mhz: Some(1400), refresher: false };
        let j = s.json(5);
        assert_eq!(j["n"], 5);
        assert_eq!(j["cpu"], json!([10, 20, 30]));
        assert_eq!(j["mem"], json!({"totalKb": 1000, "availKb": 400, "cachedKb": 100}));
        assert_eq!(j["gpu"], 7);
        assert_eq!(j["tempC"], 53);
        assert_eq!(j["mhz"], 1400);
        assert_eq!(j["refresher"], false);
        assert_eq!(Stats::default().json(1)["gpu"], Value::Null);
    }

    #[test]
    fn a_stopped_sampler_reads_nothing_and_has_no_deadline() {
        let mut s = Sampler::default();
        let now = Instant::now();
        assert_eq!(s.deadline(), None);
        assert!(!s.due(now + Duration::from_secs(60)));
        assert_eq!(s.samples, 0);
        s.start(now);
        assert_eq!(s.samples, 1, "the baseline is a reading");
        assert!(!s.ready(), "the baseline alone has no loads");
        assert_eq!(s.deadline(), Some(now + FIRST));
        s.stop();
        let seen = s.samples;
        assert_eq!(s.deadline(), None);
        assert!(!s.due(now + Duration::from_secs(60)));
        assert_eq!(s.samples, seen);
    }
}
