//! Wake-up latency of the frame loop (LPC 2026, "timerlat vs cyclictest": one wake-up latency test of the input, render and
//! present threads, tracked per release in the gate). Three things are measured on the loop's own wake-ups, per frame and for
//! free, as a latency from the moment the loop could first have run for something to the moment it did:
//!
//! - `tick`: the frame timer. Due at the time it was armed for; the loop is late when the CPU is given to another task, when
//!   the scheduler wakes it on a busy core, or when the kernel's timer interrupt is held off.
//! - `flip`: the page flip's event (the present path). Due at the kernel's own stamp of the vertical blank.
//! - `input`: a key or pointer event (the input path). Due at the stamp the kernel gave the event (`EVIOCSCLOCKID`, monotonic).
//!
//! What the loop spent on an earlier pass is not a wake-up latency: a source that fell due while the loop was still busy counts
//! from the end of that pass, and the wait for it goes in `busy` (how long the loop's own work held the tick back).
//!
//! A cyclictest-style probe ([`probe_start`]) measures what the scheduler would give a thread of a class, with no event of the
//! compositor's in it: one thread per class sleeps to an absolute time every period and records how late it ran, which is what
//! the loop (SCHED_FIFO 20), the Mali blob's submission threads (FIFO 19) and the clients (time-shared) each get under the
//! load of the moment. The percentiles come from a log-linear histogram (about 6 % wide at worst); the maximum is exact.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::output::monotonic_ns;

/// Buckets: exact microseconds below 64, then eight per power of two up to 2^27 us.
const BUCKETS: usize = 64 + 21 * 8;

fn bucket(us: u64) -> usize {
    if us < 64 {
        return us as usize;
    }
    let exp = 63 - us.leading_zeros() as usize;
    if exp >= 27 {
        return BUCKETS - 1;
    }
    64 + (exp - 6) * 8 + ((us >> (exp - 3)) & 7) as usize
}

/// The largest microsecond count a bucket holds.
fn bucket_top(idx: usize) -> u64 {
    if idx < 64 {
        return idx as u64;
    }
    let k = idx - 64;
    let exp = 6 + k / 8;
    let sub = (k % 8) as u64;
    ((8 + sub + 1) << (exp - 3)) - 1
}

/// One latency distribution. Written by one thread, read by any: relaxed counters, no lock.
pub struct Hist {
    counts: [AtomicU64; BUCKETS],
    n: AtomicU64,
    sum_us: AtomicU64,
    max_us: AtomicU64,
}

/// A distribution's numbers, in microseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub n: u64,
    pub p50_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
    pub mean_us: u64,
}

impl Hist {
    pub const fn new() -> Hist {
        Hist { counts: [const { AtomicU64::new(0) }; BUCKETS], n: AtomicU64::new(0), sum_us: AtomicU64::new(0), max_us: AtomicU64::new(0) }
    }

    pub fn record_ns(&self, ns: u64) {
        let us = ns / 1000;
        self.counts[bucket(us)].fetch_add(1, Ordering::Relaxed);
        self.n.fetch_add(1, Ordering::Relaxed);
        self.sum_us.fetch_add(us, Ordering::Relaxed);
        self.max_us.fetch_max(us, Ordering::Relaxed);
    }

    pub fn reset(&self) {
        for c in &self.counts {
            c.store(0, Ordering::Relaxed);
        }
        self.n.store(0, Ordering::Relaxed);
        self.sum_us.store(0, Ordering::Relaxed);
        self.max_us.store(0, Ordering::Relaxed);
    }

    /// The value no more than `q` (0 to 1) of the samples exceed, rounded up to its bucket and never past the maximum.
    fn quantile(&self, q: f64, n: u64, max_us: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        let target = ((q * n as f64).ceil() as u64).clamp(1, n);
        let mut seen = 0;
        for (i, c) in self.counts.iter().enumerate() {
            seen += c.load(Ordering::Relaxed);
            if seen >= target {
                return bucket_top(i).min(max_us);
            }
        }
        max_us
    }

    pub fn summary(&self) -> Summary {
        let n = self.n.load(Ordering::Relaxed);
        let max_us = self.max_us.load(Ordering::Relaxed);
        Summary {
            n,
            p50_us: self.quantile(0.50, n, max_us),
            p99_us: self.quantile(0.99, n, max_us),
            max_us,
            mean_us: self.sum_us.load(Ordering::Relaxed).checked_div(n).unwrap_or(0),
        }
    }
}

impl Default for Hist {
    fn default() -> Self {
        Hist::new()
    }
}

/// The loop's own sources.
pub static TICK: Hist = Hist::new();
pub static FLIP: Hist = Hist::new();
pub static INPUT: Hist = Hist::new();
/// How long a tick that fell due while the loop was busy waited for that pass to end.
pub static BUSY: Hist = Hist::new();

/// When the tick timer is next due (CLOCK_MONOTONIC ns), as it was armed; 0 while it has not been armed.
static TICK_DUE_NS: AtomicU64 = AtomicU64::new(0);

/// The tick timer was armed to fire at `due_ns`.
pub fn tick_armed(due_ns: u64) {
    TICK_DUE_NS.store(due_ns, Ordering::Relaxed);
}

/// The tick timer fired and the loop ran at `woke_ns`, after its last pass ended at `idle_since_ns`; `expirations` and `interval_ns`
/// say how far a periodic timer has moved on (0 for a one-shot). Returns the latency, when there is one to record.
pub fn tick_fired(woke_ns: u64, idle_since_ns: u64, expirations: u64, interval_ns: u64) -> Option<u64> {
    let due = TICK_DUE_NS.load(Ordering::Relaxed);
    if due == 0 {
        return None;
    }
    TICK_DUE_NS.store(if interval_ns > 0 { due + interval_ns * expirations.max(1) } else { 0 }, Ordering::Relaxed);
    Some(record(&TICK, due, woke_ns, idle_since_ns))
}

/// Records one wake-up of the loop at `woke_ns` for something due at `due_ns`, the loop having been free since `idle_since_ns`:
/// the latency counts from the later of the two, and a source that fell due while the loop was busy adds its wait to [`BUSY`].
/// Returns the latency in ns.
pub fn record(into: &Hist, due_ns: u64, woke_ns: u64, idle_since_ns: u64) -> u64 {
    let from = due_ns.max(idle_since_ns);
    if idle_since_ns > due_ns {
        BUSY.record_ns(idle_since_ns - due_ns);
    }
    let late = woke_ns.saturating_sub(from);
    into.record_ns(late);
    late
}

/// What a probe thread runs as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Fifo(i32),
    Other,
}

impl Class {
    pub fn parse(word: &str) -> Option<Class> {
        match word {
            "other" => Some(Class::Other),
            w => w.strip_prefix("fifo")?.parse::<i32>().ok().filter(|p| (1..=99).contains(p)).map(Class::Fifo),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Class::Fifo(p) => format!("fifo{p}"),
            Class::Other => "other".into(),
        }
    }
}

/// One probe: a class, and the CPU its thread is kept on (`fifo20@1`). Without one the scheduler places it as it places the loop;
/// with one the same class is measured on each CPU in turn, which is what shows whether a CPU that serves a noisy interrupt wakes
/// a real-time thread later than the others (the question of the IRQ placement in `docs/research/lpc-tv-result.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    pub class: Class,
    pub cpu: Option<usize>,
}

impl Spec {
    /// `other`, `fifo20`, `fifo20@1`: the class, and after an `@` the CPU.
    pub fn parse(word: &str) -> Option<Spec> {
        let (class, cpu) = match word.split_once('@') {
            Some((class, cpu)) => (class, Some(cpu.parse::<usize>().ok().filter(|&n| n < libc::CPU_SETSIZE as usize)?)),
            None => (word, None),
        };
        Some(Spec { class: Class::parse(class)?, cpu })
    }

    pub fn name(&self) -> String {
        match self.cpu {
            Some(cpu) => format!("{}@{cpu}", self.class.name()),
            None => self.class.name(),
        }
    }
}

/// Keeps the calling thread on one CPU.
fn pin(cpu: usize) -> std::io::Result<()> {
    // SAFETY: a zeroed cpu_set_t is the empty set, `cpu` is below CPU_SETSIZE (`Spec::parse` says so) and the call only reads the set.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(cpu, &mut set);
        if libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
    }
}

struct Probes {
    stop: std::sync::Arc<AtomicBool>,
    threads: Vec<(Spec, std::sync::Arc<Hist>, JoinHandle<()>)>,
}

static PROBES: Mutex<Option<Probes>> = Mutex::new(None);

/// Sleeps to an absolute time every `period` and records how late each wake-up ran, until `stop`.
fn probe_thread(spec: Spec, period: Duration, hist: std::sync::Arc<Hist>, stop: std::sync::Arc<AtomicBool>) {
    // A probe that cannot get its CPU or its class would measure another one: it measures nothing (`no samples`) rather than lie.
    if let Some(cpu) = spec.cpu
        && let Err(e) = pin(cpu)
    {
        log::warn!("wake probe {}: cannot be kept on CPU {cpu} ({e}); not measuring", spec.name());
        return;
    }
    if let Class::Fifo(p) = spec.class
        && crate::sched::fifo(p).is_err()
    {
        log::warn!("wake probe {}: SCHED_FIFO refused; not measuring", spec.name());
        return;
    }
    let period_ns = period.as_nanos() as u64;
    let mut next = monotonic_ns() + period_ns;
    while !stop.load(Ordering::Relaxed) {
        let ts = libc::timespec { tv_sec: (next / 1_000_000_000) as libc::time_t, tv_nsec: (next % 1_000_000_000) as libc::c_long };
        // SAFETY: an absolute sleep on a stack timespec; an interrupted sleep returns early and the loop measures that too.
        unsafe { libc::clock_nanosleep(libc::CLOCK_MONOTONIC, libc::TIMER_ABSTIME, &ts, std::ptr::null_mut()) };
        let now = monotonic_ns();
        hist.record_ns(now.saturating_sub(next));
        next += period_ns;
        if next <= now {
            // Slept through whole periods (a stall): the periods are lost, not owed, as in cyclictest.
            next = now + period_ns - (now - next) % period_ns;
        }
    }
}

/// Starts one probe thread per spec, each waking every `period`. Replaces probes that run. Returns the names started.
pub fn probe_start(specs: &[Spec], period: Duration) -> Vec<String> {
    probe_stop();
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let mut threads = Vec::new();
    for &spec in specs {
        let hist = std::sync::Arc::new(Hist::new());
        let (h, s) = (hist.clone(), stop.clone());
        match std::thread::Builder::new().name(format!("wake-{}", spec.name())).spawn(move || probe_thread(spec, period, h, s)) {
            Ok(t) => threads.push((spec, hist, t)),
            Err(e) => log::warn!("wake probe {}: {e}", spec.name()),
        }
    }
    let names = threads.iter().map(|(s, _, _)| s.name()).collect();
    *PROBES.lock().unwrap_or_else(PoisonError::into_inner) = Some(Probes { stop, threads });
    names
}

/// Stops the probes and returns what each measured, as `(class, summary)`.
pub fn probe_stop() -> Vec<(String, Summary)> {
    let Some(p) = PROBES.lock().unwrap_or_else(PoisonError::into_inner).take() else { return Vec::new() };
    p.stop.store(true, Ordering::Relaxed);
    p.threads
        .into_iter()
        .map(|(spec, hist, t)| {
            let _ = t.join();
            (spec.name(), hist.summary())
        })
        .collect()
}

/// What the probes have measured so far, without stopping them.
pub fn probe_read() -> Vec<(String, Summary)> {
    match PROBES.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
        Some(p) => p.threads.iter().map(|(c, h, _)| (c.name(), h.summary())).collect(),
        None => Vec::new(),
    }
}

/// Zeroes the loop's distributions (the gate does it at the start of a run).
pub fn reset() {
    for h in [&TICK, &FLIP, &INPUT, &BUSY] {
        h.reset();
    }
}

fn ms(us: u64) -> String {
    format!("{:.2}", us as f64 / 1000.0)
}

/// `p50/p99/max ms (n)` of one distribution, for the gate's line.
pub fn describe(s: &Summary) -> String {
    if s.n == 0 { "no samples".into() } else { format!("p50 {} p99 {} max {} ms (n {})", ms(s.p50_us), ms(s.p99_us), ms(s.max_us), s.n) }
}

/// A distribution as the control socket's JSON.
pub fn summary_json(s: &Summary) -> serde_json::Value {
    serde_json::json!({"n": s.n, "p50_us": s.p50_us, "p99_us": s.p99_us, "max_us": s.max_us, "mean_us": s.mean_us})
}

/// `ctl wake`: every distribution as numbers and as the one line the gate prints.
pub fn report() -> serde_json::Value {
    let probes = probe_read();
    let mut line = format!(
        "tick {}; flip {}; input {}; tick waited on a busy loop {}",
        describe(&TICK.summary()),
        describe(&FLIP.summary()),
        describe(&INPUT.summary()),
        describe(&BUSY.summary())
    );
    for (class, s) in &probes {
        line.push_str(&format!("; probe {class} {}", describe(s)));
    }
    serde_json::json!({
        "ok": true,
        "line": line,
        "tick": summary_json(&TICK.summary()),
        "flip": summary_json(&FLIP.summary()),
        "input": summary_json(&INPUT.summary()),
        "busy": summary_json(&BUSY.summary()),
        "probes": probes.iter().map(|(c, s)| (c.clone(), summary_json(s))).collect::<serde_json::Map<_, _>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The distributions and the tick's due time are process-wide: the tests that touch them take turns.
    static GLOBALS: Mutex<()> = Mutex::new(());

    fn globals() -> std::sync::MutexGuard<'static, ()> {
        GLOBALS.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[test]
    fn buckets_are_monotonic_and_their_tops_hold_their_values() {
        let mut last = 0;
        for us in (0..200_000u64).chain([1 << 20, 3 << 22, (1 << 27) - 1]) {
            let b = bucket(us);
            assert!(b >= last || us > 200_000, "bucket {b} for {us} fell back from {last}");
            assert!(b < BUCKETS);
            assert!(bucket_top(b) >= us || b == BUCKETS - 1, "{us} us is past the top {} of its bucket {b}", bucket_top(b));
            if us > 0 && b > 0 && us < 200_000 {
                assert!(bucket_top(b - 1) < us, "{us} us sits in a bucket above its own");
            }
            last = b.max(last);
        }
        assert_eq!(bucket(u64::MAX), BUCKETS - 1, "an hour is a bucket, not a panic");
    }

    #[test]
    fn percentiles_of_a_known_set() {
        let h = Hist::new();
        for us in 1..=1000u64 {
            h.record_ns(us * 1000);
        }
        h.record_ns(40_000_000);
        let s = h.summary();
        assert_eq!(s.n, 1001);
        assert_eq!(s.max_us, 40_000, "the maximum is exact");
        assert!((s.p50_us as f64 - 500.0).abs() / 500.0 < 0.07, "p50 {} us of 1..1000", s.p50_us);
        assert!((s.p99_us as f64 - 990.0).abs() / 990.0 < 0.07, "p99 {} us of 1..1000 plus an outlier", s.p99_us);
        assert!(s.p99_us < s.max_us, "one outlier in 1001 is not the 99th percentile");
        assert_eq!(s.mean_us, (500_500 + 40_000) / 1001);
        h.reset();
        assert_eq!(h.summary(), Summary::default());
    }

    #[test]
    fn small_latencies_are_exact_microseconds() {
        let h = Hist::new();
        for us in [3u64, 3, 3, 9] {
            h.record_ns(us * 1000 + 400);
        }
        let s = h.summary();
        assert_eq!((s.p50_us, s.max_us), (3, 9));
    }

    #[test]
    fn a_wake_counts_from_when_the_loop_could_have_run() {
        let _g = globals();
        let into = Hist::new();
        BUSY.reset();
        // Due at 1000 us, the loop free since 0: woke at 1300 us, 300 us late.
        assert_eq!(record(&into, 1_000_000, 1_300_000, 0), 300_000);
        assert_eq!(BUSY.summary().n, 0, "nothing held it back");
        // Due at 1000 us while the loop was busy until 5000 us, woke at 5100 us: 100 us of wake-up, 4000 us of busy loop.
        assert_eq!(record(&into, 1_000_000, 5_100_000, 5_000_000), 100_000);
        let b = BUSY.summary();
        assert_eq!((b.n, b.max_us), (1, 4000));
        assert_eq!(into.summary().max_us, 300);
        BUSY.reset();
    }

    #[test]
    fn the_tick_is_judged_against_when_it_was_armed_for_and_a_periodic_one_moves_on() {
        let _g = globals();
        TICK.reset();
        BUSY.reset();
        tick_armed(0);
        assert_eq!(tick_fired(5_000_000, 0, 1, 0), None, "a timer never armed has no due time");
        tick_armed(10_000_000);
        assert_eq!(tick_fired(10_250_000, 9_000_000, 1, 0), Some(250_000));
        assert_eq!(tick_fired(20_000_000, 0, 1, 0), None, "a one-shot is spent");
        // Periodic, 16.7 ms: two periods read at once move the due time on by both, so the next one is judged against the third.
        tick_armed(100_000_000);
        assert_eq!(tick_fired(100_100_000, 90_000_000, 2, 16_666_667), Some(100_000));
        // Due at 133.33 ms; the loop was busy until 140 ms and woke at 150 ms: 10 ms of wake-up, 6.67 ms of busy loop before it.
        assert_eq!(tick_fired(150_000_000, 140_000_000, 1, 16_666_667), Some(10_000_000));
        assert_eq!(BUSY.summary().max_us, 6_666);
        assert_eq!(TICK.summary().n, 3);
        TICK.reset();
        BUSY.reset();
    }

    #[test]
    fn classes_are_spelled_as_the_gate_spells_them() {
        assert_eq!(Class::parse("fifo20"), Some(Class::Fifo(20)));
        assert_eq!(Class::parse("other"), Some(Class::Other));
        for bad in ["fifo0", "fifo100", "fifo", "rr5", ""] {
            assert_eq!(Class::parse(bad), None, "{bad}");
        }
        assert_eq!(Class::Fifo(19).name(), "fifo19");
    }

    #[test]
    fn a_probe_names_its_cpu_after_an_at_sign_and_only_a_cpu_that_can_exist() {
        assert_eq!(Spec::parse("fifo20@1"), Some(Spec { class: Class::Fifo(20), cpu: Some(1) }));
        assert_eq!(Spec::parse("other@0"), Some(Spec { class: Class::Other, cpu: Some(0) }));
        assert_eq!(Spec::parse("fifo20"), Some(Spec { class: Class::Fifo(20), cpu: None }));
        for bad in ["fifo20@", "fifo20@x", "fifo20@-1", "fifo20@1024", "@1", "rr5@1", "other@1@2"] {
            assert_eq!(Spec::parse(bad), None, "{bad}");
        }
        assert_eq!(Spec::parse("fifo19@3").unwrap().name(), "fifo19@3");
        assert_eq!(Spec::parse("other").unwrap().name(), "other");
    }

    /// A CPU the test process may run on.
    fn allowed_cpu() -> usize {
        // SAFETY: a zeroed cpu_set_t is the empty set; sched_getaffinity fills the set it is given.
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            assert_eq!(libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut set), 0);
            (0..libc::CPU_SETSIZE as usize).find(|&cpu| libc::CPU_ISSET(cpu, &set)).expect("the test runs on some CPU")
        }
    }

    /// The probes' summaries once probe `i` has `n` samples (a 60 s cap ends a probe that never wakes).
    fn wait_for_samples(i: usize, n: u64) -> Vec<(String, Summary)> {
        let end = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let live = probe_read();
            if live.get(i).is_some_and(|l| l.1.n >= n) || std::time::Instant::now() >= end {
                return live;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_time_shared_probe_measures_a_sleeping_thread_and_stops() {
        let _g = globals();
        let started = probe_start(&[Spec { class: Class::Other, cpu: None }], Duration::from_millis(2));
        assert_eq!(started, vec!["other".to_string()]);
        // 20 wake-ups at 2 ms is 40 ms of an idle machine; a loaded one is given as long as it takes (the count is the test, not the time).
        let live = wait_for_samples(0, 20);
        assert_eq!(live.len(), 1);
        assert!(live[0].1.n >= 20, "{} wake-ups at 2 ms", live[0].1.n);
        let done = probe_stop();
        assert_eq!(done[0].0, "other");
        assert!(done[0].1.n >= live[0].1.n);
        assert!(done[0].1.p50_us < 50_000, "a thread that sleeps to an absolute time wakes within 50 ms (p50 {} us)", done[0].1.p50_us);
        assert!(probe_read().is_empty() && probe_stop().is_empty());
    }

    #[test]
    fn a_probe_kept_on_a_cpu_measures_there_and_one_that_cannot_be_says_so_by_having_no_samples() {
        let _g = globals();
        let cpu = allowed_cpu();
        let here = Spec::parse(&format!("other@{cpu}")).unwrap();
        let nowhere = Spec::parse("other@1023").unwrap();
        let started = probe_start(&[here, nowhere], Duration::from_millis(2));
        assert_eq!(started, vec![format!("other@{cpu}"), "other@1023".to_string()]);
        wait_for_samples(0, 20);
        let done = probe_stop();
        assert!(done[0].1.n >= 20, "{} wake-ups on CPU {cpu} at 2 ms", done[0].1.n);
        assert_eq!(done[1].1.n, 0, "CPU 1023 does not exist: nothing was measured and nothing was made up");
        assert!(describe(&done[1].1).contains("no samples"));
    }

    #[test]
    fn the_report_carries_the_line_the_gate_prints() {
        let _g = globals();
        reset();
        TICK.record_ns(120_000);
        let r = report();
        assert_eq!(r["ok"], true);
        assert!(r["line"].as_str().unwrap().starts_with("tick p50 0.12 p99 0.12 max 0.12 ms (n 1); flip no samples"), "{}", r["line"]);
        assert_eq!(r["tick"]["max_us"], 120);
        reset();
    }
}
