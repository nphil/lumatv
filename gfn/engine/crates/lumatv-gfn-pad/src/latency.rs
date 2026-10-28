//! Phase 4's measurement: for each button press, how long from the moment the kernel stamped the event to the moment the engine
//! handed the packet on (docs/research/gfn-input-result.md).
//!
//! One press passes five instants, all on `CLOCK_MONOTONIC` so they compare without conversion:
//!
//! | Instant | Taken by |
//! |---|---|
//! | `kernel` | the evdev `SYN_REPORT`'s own time stamp, set when `hid-sony` delivered the report (the pad's evdev fd is switched to the monotonic clock with `EVIOCSCLOCKID`) |
//! | `read` | the reader thread, right after `read()` returned |
//! | `pushed` | the reader thread, after the sink took the report (for the engine: the queue holds the packet) |
//! | `handoff` | the engine, when it took the sample out of the queue to give the transport: [`Tap::handoff`] |
//! | `send` | the first send that returned after the handoff, from the `lumatv-gfn-sendtap` preload when the engine runs with it |
//!
//! The press is followed by the sample's `captured_at` time (a key the sink makes up and the queue gives back), so the tap needs
//! no change to the queue's types. Finished presses go to a ring for the summary and out through [`Tap::take_finished`] to the
//! stats thread, which writes `latency.tsv` (one line per press, absolute stamps, for the shell tool) and `latency.json`.
use std::collections::VecDeque;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

/// How many finished presses the summary looks at.
pub const WINDOW: usize = 512;
const PENDING_MAX: usize = 256;
/// A press the engine never took (the UI had the input) is forgotten after this long.
const UNHANDED_AFTER_NS: u64 = 2_000_000_000;
/// The send is looked for once the handoff is this old; a send that returned later than [`SEND_WITHIN_NS`] is not the press's.
const SEND_AFTER_NS: u64 = 30_000_000;
const SEND_WITHIN_NS: u64 = 20_000_000;

pub fn mono_ns() -> u64 {
    clock_ns(libc::CLOCK_MONOTONIC)
}

fn clock_ns(id: libc::clockid_t) -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a valid out-parameter.
    unsafe { libc::clock_gettime(id, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// Which clock an evdev time stamp is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clock {
    Mono,
    /// The kernel's default for a node nobody called `EVIOCSCLOCKID` on.
    Real,
}

/// An event's time stamp on the monotonic clock, from its `timeval`. A realtime stamp is moved by how far the two clocks are apart now.
pub fn stamp_ns(sec: i64, usec: i64, clock: Clock) -> u64 {
    let ns = (sec as u64).wrapping_mul(1_000_000_000).wrapping_add((usec as u64).wrapping_mul(1000));
    match clock {
        Clock::Mono => ns,
        Clock::Real => ns.saturating_sub(clock_ns(libc::CLOCK_REALTIME).saturating_sub(mono_ns())),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    id: u64,
    slot: u8,
    buttons: u16,
    kernel_ns: u64,
    read_ns: u64,
    pushed_ns: Option<u64>,
    link: Option<Instant>,
    handoff_ns: Option<u64>,
}

/// A finished press, as microseconds after the kernel's stamp (`send_us` is absent without the preload).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Done {
    pub id: u64,
    pub slot: u8,
    pub buttons: u16,
    pub kernel_ns: u64,
    pub wake_us: u32,
    pub pushed_us: u32,
    pub handoff_us: u32,
    pub send_us: Option<u32>,
}

impl Done {
    /// The line `latency.tsv` holds: `id slot buttons kernel_ns wake_us pushed_us handoff_us send_us` (`-` for no send).
    pub fn tsv(&self) -> String {
        let send = self.send_us.map_or("-".to_owned(), |s| s.to_string());
        format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", self.id, self.slot, self.buttons, self.kernel_ns, self.wake_us, self.pushed_us, self.handoff_us, send)
    }

    pub const TSV_HEADER: &'static str = "id\tslot\tbuttons\tkernel_ns\twake_us\tpushed_us\thandoff_us\tsend_us";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stat {
    pub p50: u32,
    pub p90: u32,
    pub p99: u32,
    pub max: u32,
}

impl Stat {
    /// Percentiles by the nearest-rank rule over `values` (any order); all zero for none.
    pub fn of(values: &[u32]) -> Stat {
        if values.is_empty() {
            return Stat::default();
        }
        let mut v = values.to_vec();
        v.sort_unstable();
        let rank = |p: usize| v[((p * v.len()).div_ceil(100)).clamp(1, v.len()) - 1];
        Stat { p50: rank(50), p90: rank(90), p99: rank(99), max: v[v.len() - 1] }
    }

    fn json(&self) -> String {
        format!("{{\"p50\":{},\"p90\":{},\"p99\":{},\"max\":{}}}", self.p50, self.p90, self.p99, self.max)
    }
}

/// Bucket edges of the histogram, in microseconds; the last bucket is everything above.
pub const BUCKETS_US: [u32; 6] = [250, 500, 1000, 2000, 4000, 8000];

pub fn histogram(values: &[u32]) -> [usize; 7] {
    let mut h = [0usize; 7];
    for &v in values {
        h[BUCKETS_US.iter().position(|&edge| v < edge).unwrap_or(6)] += 1;
    }
    h
}

/// The stages over the last [`WINDOW`] finished presses.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    /// Presses finished since the start, and how many the summary covers.
    pub total: u64,
    pub samples: usize,
    /// Kernel stamp to the reader's `read()` returning.
    pub wake: Stat,
    /// Kernel stamp to the report being in the engine's queue.
    pub pushed: Stat,
    /// Kernel stamp to the engine taking the packet for the transport: the number that matters.
    pub handoff: Stat,
    /// Kernel stamp to the send returning; `None` without the preload.
    pub send: Option<Stat>,
}

impl Summary {
    pub fn json(&self) -> String {
        let send = self.send.map_or("null".to_owned(), |s| s.json());
        format!(
            "{{\"total\":{},\"samples\":{},\"wake_us\":{},\"pushed_us\":{},\"handoff_us\":{},\"send_us\":{}}}",
            self.total,
            self.samples,
            self.wake.json(),
            self.pushed.json(),
            self.handoff.json(),
            send
        )
    }

    /// The short form for the stats line: `pad 0.4 ms (p99 0.9)`; empty before the first press.
    pub fn line(&self) -> String {
        if self.samples == 0 {
            return String::new();
        }
        let ms = |us: u32| format!("{:.1}", f64::from(us) / 1000.0);
        let s = self.send.unwrap_or(self.handoff);
        format!("pad {} ms (p99 {})", ms(s.p50), ms(s.p99))
    }
}

type Probe = Box<dyn Fn(u64) -> u64 + Send + Sync>;

struct State {
    next_id: u64,
    pending: VecDeque<Pending>,
    done: VecDeque<Done>,
    total: u64,
}

impl std::fmt::Debug for Tap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tap").field("preload", &self.probe.is_some()).finish()
    }
}

pub struct Tap {
    state: Mutex<State>,
    /// `first send return at or after t`, 0 for none (the preload's `lumatv_sendtap_first_after`).
    probe: Option<Probe>,
}

impl Tap {
    pub fn new(probe: Option<Probe>) -> Self {
        Self { state: Mutex::new(State { next_id: 1, pending: VecDeque::new(), done: VecDeque::new(), total: 0 }), probe }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A press was read: its stamps so far. Returns the press's id.
    pub fn begin(&self, slot: u8, buttons: u16, kernel_ns: u64, read_ns: u64) -> u64 {
        let mut s = self.lock();
        let id = s.next_id;
        s.next_id += 1;
        if s.pending.len() >= PENDING_MAX {
            s.pending.pop_front();
        }
        s.pending.push_back(Pending { id, slot, buttons, kernel_ns, read_ns, pushed_ns: None, link: None, handoff_ns: None });
        id
    }

    /// The sink is about to queue the packet and will tag the queued sample with `at`; [`Tap::handoff`] gives the tag back.
    pub fn link(&self, id: u64, at: Instant) {
        if let Some(p) = self.lock().pending.iter_mut().rev().find(|p| p.id == id) {
            p.link = Some(at);
        }
    }

    pub fn pushed(&self, id: u64, ns: u64) {
        if let Some(p) = self.lock().pending.iter_mut().rev().find(|p| p.id == id) {
            p.pushed_ns = Some(ns);
        }
    }

    /// The engine took the sample tagged `at` to hand to the transport. Cheap for the samples that are not presses.
    pub fn handoff(&self, at: Instant, ns: u64) {
        let mut s = self.lock();
        if s.pending.is_empty() {
            return;
        }
        if let Some(p) = s.pending.iter_mut().rev().find(|p| p.link == Some(at)) {
            p.handoff_ns.get_or_insert(ns);
        }
    }

    /// Move the presses that are complete (or hopeless) to the finished ring; returns the newly finished, oldest first.
    pub fn take_finished(&self, now_ns: u64) -> Vec<Done> {
        let mut s = self.lock();
        let mut fresh = Vec::new();
        let mut keep = VecDeque::with_capacity(s.pending.len());
        let pending = std::mem::take(&mut s.pending);
        for p in pending {
            match p.handoff_ns {
                Some(h) if now_ns.saturating_sub(h) >= SEND_AFTER_NS || self.probe.is_none() => {
                    let sent = self.probe.as_ref().map(|probe| probe(h)).filter(|&t| t != 0 && t - h <= SEND_WITHIN_NS);
                    let us = |t: u64| u32::try_from(t.saturating_sub(p.kernel_ns) / 1000).unwrap_or(u32::MAX);
                    fresh.push(Done {
                        id: p.id,
                        slot: p.slot,
                        buttons: p.buttons,
                        kernel_ns: p.kernel_ns,
                        wake_us: us(p.read_ns),
                        pushed_us: us(p.pushed_ns.unwrap_or(h)),
                        handoff_us: us(h),
                        send_us: sent.map(us),
                    });
                }
                Some(_) => keep.push_back(p),
                None if now_ns.saturating_sub(p.kernel_ns) > UNHANDED_AFTER_NS => {}
                None => keep.push_back(p),
            }
        }
        s.pending = keep;
        for d in &fresh {
            if s.done.len() >= WINDOW {
                s.done.pop_front();
            }
            s.done.push_back(*d);
            s.total += 1;
        }
        fresh
    }

    pub fn summary(&self) -> Summary {
        let s = self.lock();
        let col = |f: fn(&Done) -> u32| s.done.iter().map(f).collect::<Vec<u32>>();
        let sends: Vec<u32> = s.done.iter().filter_map(|d| d.send_us).collect();
        Summary {
            total: s.total,
            samples: s.done.len(),
            wake: Stat::of(&col(|d| d.wake_us)),
            pushed: Stat::of(&col(|d| d.pushed_us)),
            handoff: Stat::of(&col(|d| d.handoff_us)),
            send: (!sends.is_empty()).then(|| Stat::of(&sends)),
        }
    }
}

fn preload_probe() -> Option<Probe> {
    // SAFETY: dlsym with a NUL-terminated name; the symbol, if present, is `extern "C" fn(u64) -> u64` (lumatv-gfn-sendtap).
    let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"lumatv_sendtap_first_after".as_ptr()) };
    if p.is_null() {
        return None;
    }
    let f: extern "C" fn(u64) -> u64 = unsafe { std::mem::transmute::<*mut libc::c_void, extern "C" fn(u64) -> u64>(p) };
    Some(Box::new(move |t| f(t)))
}

/// The engine's tap: the reader, the sink and the queue all find it here (they are made in different places and in a different order).
pub fn global() -> &'static Arc<Tap> {
    static TAP: LazyLock<Arc<Tap>> = LazyLock::new(|| Arc::new(Tap::new(preload_probe())));
    &TAP
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn percentiles_are_nearest_rank() {
        assert_eq!(Stat::of(&[]), Stat::default());
        assert_eq!(Stat::of(&[7]), Stat { p50: 7, p90: 7, p99: 7, max: 7 });
        let v: Vec<u32> = (1..=100).rev().collect();
        assert_eq!(Stat::of(&v), Stat { p50: 50, p90: 90, p99: 99, max: 100 });
        assert_eq!(Stat::of(&[10, 20, 30, 40]).p50, 20);
        assert_eq!(histogram(&[0, 249, 250, 999, 1000, 7999, 8000, 90_000]), [2, 1, 1, 1, 0, 1, 2]);
    }

    #[test]
    fn a_realtime_stamp_lands_on_the_monotonic_clock() {
        let real = clock_ns(libc::CLOCK_REALTIME);
        let mono = mono_ns();
        let got = stamp_ns((real / 1_000_000_000) as i64, ((real % 1_000_000_000) / 1000) as i64, Clock::Real);
        let diff = got.abs_diff(mono);
        assert!(diff < 5_000_000, "off by {diff} ns");
        assert_eq!(stamp_ns(5, 250, Clock::Mono), 5_000_250_000);
    }

    #[test]
    fn a_press_is_followed_from_the_kernel_stamp_to_the_send() {
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        // the preload's answer: the packet leaves 120 us after the handoff
        let tap = Tap::new(Some(Box::new(move |t| {
            log.lock().unwrap().push(t);
            t + 120_000
        })));
        let at = Instant::now();
        let id = tap.begin(0, 0x1000, 1_000_000_000, 1_000_150_000);
        tap.link(id, at);
        tap.pushed(id, 1_000_190_000);
        assert!(tap.take_finished(1_001_000_000).is_empty(), "nothing handed over yet");
        tap.handoff(at + Duration::from_nanos(1), 1_000_700_000);
        tap.handoff(at, 1_000_400_000);
        tap.handoff(at, 1_000_900_000);
        assert!(tap.take_finished(1_010_000_000).is_empty(), "the send is looked for once the handoff has aged");
        let done = tap.take_finished(1_100_000_000);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0], Done { id, slot: 0, buttons: 0x1000, kernel_ns: 1_000_000_000, wake_us: 150, pushed_us: 190, handoff_us: 400, send_us: Some(520) });
        assert_eq!(seen.lock().unwrap().as_slice(), [1_000_400_000], "the first handoff counts");
        assert_eq!(done[0].tsv(), format!("{id}\t0\t4096\t1000000000\t150\t190\t400\t520"));
        let s = tap.summary();
        assert_eq!((s.total, s.samples, s.handoff.p50, s.send.unwrap().p50), (1, 1, 400, 520));
        assert_eq!(s.line(), "pad 0.5 ms (p99 0.5)");
        assert!(s.json().contains("\"send_us\":{\"p50\":520"));
        assert!(tap.take_finished(2_000_000_000).is_empty(), "a press finishes once");
    }

    #[test]
    fn without_the_preload_a_press_finishes_at_its_handoff_and_a_late_send_is_not_its_own() {
        let tap = Tap::new(None);
        let at = Instant::now();
        let id = tap.begin(1, 0x2000, 5_000_000, 5_100_000);
        tap.link(id, at);
        tap.handoff(at, 5_300_000);
        let d = tap.take_finished(5_300_001);
        assert_eq!((d[0].handoff_us, d[0].pushed_us, d[0].send_us), (300, 300, None), "no push stamp: the handoff stands in");
        assert_eq!(tap.summary().send, None);
        assert_eq!(tap.summary().line(), "pad 0.3 ms (p99 0.3)");
        let slow = Tap::new(Some(Box::new(|t| t + 50_000_000)));
        let id = slow.begin(0, 1, 0, 0);
        slow.link(id, at);
        slow.handoff(at, 1_000_000);
        assert_eq!(slow.take_finished(100_000_000)[0].send_us, None, "50 ms later is another packet");
    }

    #[test]
    fn presses_the_engine_never_takes_are_forgotten_and_the_rings_are_bounded() {
        let tap = Tap::new(None);
        tap.begin(0, 1, 1_000, 2_000);
        assert!(tap.take_finished(1_000_000_000).is_empty());
        assert!(tap.take_finished(UNHANDED_AFTER_NS + 10_000).is_empty());
        assert_eq!(tap.summary().total, 0);
        for i in 0..(WINDOW as u64 + 50) {
            let at = Instant::now();
            let id = tap.begin(0, 1, i, i);
            tap.link(id, at);
            tap.handoff(at, i + 10_000);
            tap.take_finished(i + 10_000);
        }
        let s = tap.summary();
        assert_eq!((s.total, s.samples), (WINDOW as u64 + 50, WINDOW));
    }
}
