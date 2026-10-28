//! The frame path's memory discipline (LPC 2026, "Kernel Lock Contention Hotspots Causing Frame Drops on Android": the lock waits
//! behind a late frame are the GPU driver's, `mmap_lock` and swap-in). On the set's 5.4 kernel every page fault and every
//! `mmap`/`munmap`/`mprotect`/`brk` of the process takes the one `mmap_sem`, so a loop that is reading a page back from flash
//! (or from the compressed swap, `memory-result.md`: 1,428 and 4,454 compositor major faults after a hog) holds up a loop that
//! is only trimming its heap. Two things keep the loop out of it:
//!
//! - glibc's `malloc` is told not to ask the kernel for anything in steady state ([`tune_malloc`]): no `mmap` for large blocks
//!   (they come from the heap, which is grown in 8 MiB steps), no trim of the heap's top, and the heap is grown once at start
//!   ([`pregrow`]) so the first frames find their room already mapped. Everything the compositor allocates on the loop thread
//!   is in the main arena (the loop is the process's first thread); other threads' arenas grow by `mprotect`, which only
//!   threads that are not in a frame's way do.
//! - what the compositor has touched stays in RAM ([`lock`]): `mlockall(MCL_CURRENT | MCL_FUTURE | MCL_ONFAULT)`. The flag
//!   makes the lock follow the pages in as they are touched instead of faulting every mapping in at once (a plain `mlockall`
//!   reads every page of the Mali blob and gives each thread's 8 MiB stack its 2,000 pages), so the resident set does not grow
//!   by the lock itself: the pages the frame path has used are the ones pinned, and nothing else's memory pressure can take them.
//!
//! The GPU driver's own mappings (the blob's buffers through kbase) are special mappings the lock skips and the compositor does
//! not make; the audit of what it asks of the driver per frame is in `docs/research/lpc-tv-result.md`.

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How far the main arena's break moves past what a request needs: one `brk` an 8 MiB step instead of one per frame's worth.
pub const TOP_PAD: libc::c_int = 8 << 20;
/// A heap top this big is never returned to the kernel (glibc trims at 128 KiB by default, then at twice the largest block freed).
pub const TRIM_THRESHOLD: libc::c_int = 256 << 20;
/// How much heap [`pregrow`] touches at start.
pub const PREGROW: usize = 16 << 20;
/// The lock is not taken when less than this is available: a pinned page cannot be reclaimed for the apps in front of Home.
pub const MIN_AVAILABLE_KIB: u64 = 256 * 1024;
/// Below this `RLIMIT_MEMLOCK` (bytes) a process without `CAP_IPC_LOCK` cannot lock what the frame path needs.
pub const MIN_RLIMIT_BYTES: u64 = 128 << 20;

const MCL_ONFAULT: libc::c_int = 4;
const CAP_IPC_LOCK: u32 = 14;
const PAGE: usize = 4096;

/// `--mlock`: whether the lock is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mlock {
    Off,
    /// Lock what is touched, if the set's limit and headroom allow ([`decide`]).
    Auto,
}

/// What the set says about locking, read at start ([`facts`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facts {
    pub mem_available_kib: u64,
    /// `RLIMIT_MEMLOCK` in bytes, `None` for unlimited.
    pub rlimit_bytes: Option<u64>,
    pub ipc_lock: bool,
    pub rss_kib: u64,
}

/// The flags for `mlockall`, or why the lock is not taken. Pure: the set's numbers come in `facts`.
pub fn decide(mode: Mlock, facts: &Facts) -> Result<libc::c_int, String> {
    if mode == Mlock::Off {
        return Err("off (--mlock off)".into());
    }
    if !facts.ipc_lock && facts.rlimit_bytes.is_some_and(|r| r < MIN_RLIMIT_BYTES) {
        return Err(format!(
            "RLIMIT_MEMLOCK is {} KiB and the process has no CAP_IPC_LOCK (needs {} MiB)",
            facts.rlimit_bytes.unwrap_or(0) / 1024,
            MIN_RLIMIT_BYTES >> 20
        ));
    }
    if facts.mem_available_kib < MIN_AVAILABLE_KIB {
        return Err(format!("MemAvailable is {} MiB, under the {} MiB kept for reclaim", facts.mem_available_kib / 1024, MIN_AVAILABLE_KIB / 1024));
    }
    Ok(libc::MCL_CURRENT | libc::MCL_FUTURE | MCL_ONFAULT)
}

/// The `Key:   N kB` lines of `/proc/self/status` or `/proc/meminfo` as a map of the numbers.
pub fn parse_kib(text: &str) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else { continue };
        let mut words = rest.split_whitespace();
        if let (Some(n), Some("kB")) = (words.next().and_then(|n| n.parse::<u64>().ok()), words.next()) {
            out.insert(key.to_string(), n);
        }
    }
    out
}

/// Whether bit `cap` is set in a `CapEff:` hex mask.
pub fn has_cap(mask_hex: &str, cap: u32) -> bool {
    u64::from_str_radix(mask_hex.trim(), 16).is_ok_and(|m| m & (1 << cap) != 0)
}

fn rlimit_memlock() -> Option<u64> {
    let mut r = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: a stack rlimit for getrlimit to fill.
    if unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &mut r) } != 0 || r.rlim_cur == libc::RLIM_INFINITY { None } else { Some(r.rlim_cur) }
}

/// The set's numbers as they are now.
pub fn facts() -> Facts {
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let cap = status.lines().find_map(|l| l.strip_prefix("CapEff:")).unwrap_or("0");
    Facts {
        mem_available_kib: parse_kib(&meminfo).get("MemAvailable").copied().unwrap_or(0),
        rlimit_bytes: rlimit_memlock(),
        ipc_lock: has_cap(cap, CAP_IPC_LOCK),
        rss_kib: parse_kib(&status).get("VmRSS").copied().unwrap_or(0),
    }
}

/// Tells glibc's `malloc` not to ask the kernel for anything in steady state: no `mmap` for large blocks, no trim, and room asked
/// for in 8 MiB steps. Setting any of these also ends glibc's dynamic raising of its thresholds. Call it before the first thread
/// is made. Returns the parameters glibc refused (none on glibc).
#[cfg(target_env = "gnu")]
pub fn tune_malloc() -> Vec<&'static str> {
    let mut refused = Vec::new();
    // SAFETY: mallopt only sets the allocator's parameters.
    unsafe {
        if libc::mallopt(libc::M_MMAP_MAX, 0) != 1 {
            refused.push("M_MMAP_MAX");
        }
        if libc::mallopt(libc::M_TRIM_THRESHOLD, TRIM_THRESHOLD) != 1 {
            refused.push("M_TRIM_THRESHOLD");
        }
        if libc::mallopt(libc::M_TOP_PAD, TOP_PAD) != 1 {
            refused.push("M_TOP_PAD");
        }
    }
    refused
}

/// A libc without `mallopt` has nothing to tune.
#[cfg(not(target_env = "gnu"))]
pub fn tune_malloc() -> Vec<&'static str> {
    Vec::new()
}

/// Grows the heap by `bytes` and touches every page, then gives it back to the allocator (which keeps it, [`tune_malloc`]). On
/// the loop's thread: the main arena is the one that is grown by `brk`.
pub fn pregrow(bytes: usize) -> Duration {
    let t0 = Instant::now();
    let mut v: Vec<u8> = Vec::with_capacity(bytes);
    let p = v.as_mut_ptr();
    let mut at = 0;
    while at < bytes {
        // SAFETY: `at` is inside the capacity just reserved.
        unsafe { p.add(at).write_volatile(0) };
        at += PAGE;
    }
    drop(v);
    t0.elapsed()
}

static LOCK_LINE: OnceLock<String> = OnceLock::new();

/// Takes the lock ([`decide`]) and returns what was done, as one line for the log; `ctl mem` repeats it.
pub fn lock(mode: Mlock) -> String {
    let f = facts();
    let line = match decide(mode, &f) {
        Err(why) => format!("not locked: {why}"),
        Ok(flags) => {
            let t0 = Instant::now();
            // SAFETY: mlockall takes no pointers.
            if unsafe { libc::mlockall(flags) } == 0 {
                format!(
                    "locked what is touched (MCL_CURRENT|MCL_FUTURE|MCL_ONFAULT) in {:.1} ms; VmRSS {} MiB, MemAvailable {} MiB, RLIMIT_MEMLOCK {}, CAP_IPC_LOCK {}",
                    t0.elapsed().as_secs_f64() * 1e3,
                    f.rss_kib / 1024,
                    f.mem_available_kib / 1024,
                    f.rlimit_bytes.map_or("unlimited".to_string(), |b| format!("{} KiB", b / 1024)),
                    if f.ipc_lock { "yes" } else { "no" }
                )
            } else {
                format!("not locked: mlockall refused ({})", std::io::Error::last_os_error())
            }
        }
    };
    let _ = LOCK_LINE.set(line.clone());
    line
}

/// `ctl mem`: the process's memory as the frame path sees it.
#[cfg(target_env = "gnu")]
pub fn report() -> serde_json::Value {
    let status = parse_kib(&std::fs::read_to_string("/proc/self/status").unwrap_or_default());
    let f = facts();
    // SAFETY: mallinfo2 only reads the allocator's counters.
    let m = unsafe { libc::mallinfo2() };
    serde_json::json!({
        "ok": true,
        "rss_kib": status.get("VmRSS"),
        "hwm_kib": status.get("VmHWM"),
        "locked_kib": status.get("VmLck"),
        "data_kib": status.get("VmData"),
        "swap_kib": status.get("VmSwap"),
        "mem_available_kib": f.mem_available_kib,
        "rlimit_memlock_bytes": f.rlimit_bytes,
        "cap_ipc_lock": f.ipc_lock,
        "lock": LOCK_LINE.get(),
        "malloc": {
            "arena_bytes": m.arena as u64,
            "mmapped_blocks": m.hblks as u64,
            "mmapped_bytes": m.hblkhd as u64,
            "in_use_bytes": m.uordblks as u64,
            "free_bytes": m.fordblks as u64,
            "top_reusable_bytes": m.keepcost as u64,
        },
    })
}

#[cfg(not(target_env = "gnu"))]
pub fn report() -> serde_json::Value {
    serde_json::json!({"ok": true, "lock": LOCK_LINE.get()})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(available_mib: u64, rlimit: Option<u64>, ipc_lock: bool) -> Facts {
        Facts { mem_available_kib: available_mib * 1024, rlimit_bytes: rlimit, ipc_lock, rss_kib: 80 * 1024 }
    }

    #[test]
    fn the_lock_is_taken_where_the_set_has_room_for_it() {
        let flags = decide(Mlock::Auto, &facts(700, None, true)).unwrap();
        assert_eq!(flags, libc::MCL_CURRENT | libc::MCL_FUTURE | MCL_ONFAULT, "on fault: the lock must not fault every mapping in");
        assert!(decide(Mlock::Auto, &facts(700, Some(64 << 10), true)).is_ok(), "CAP_IPC_LOCK ignores the limit");
        assert!(decide(Mlock::Auto, &facts(700, Some(MIN_RLIMIT_BYTES), false)).is_ok());
    }

    #[test]
    fn it_says_why_it_is_not_taken() {
        assert!(decide(Mlock::Off, &facts(700, None, true)).unwrap_err().contains("--mlock off"));
        let e = decide(Mlock::Auto, &facts(700, Some(64 << 10), false)).unwrap_err();
        assert!(e.contains("RLIMIT_MEMLOCK is 64 KiB") && e.contains("CAP_IPC_LOCK"), "{e}");
        let e = decide(Mlock::Auto, &facts(200, None, true)).unwrap_err();
        assert!(e.contains("MemAvailable is 200 MiB") && e.contains("256 MiB"), "{e}");
    }

    #[test]
    fn proc_files_are_read_by_their_kib_lines() {
        let m = parse_kib("MemTotal:  3000000 kB\nMemAvailable:\t 684000 kB\nHugePages_Total:       0\nVmLck:\t 0 kB\n");
        assert_eq!(m.get("MemAvailable"), Some(&684_000));
        assert_eq!(m.get("VmLck"), Some(&0));
        assert!(!m.contains_key("HugePages_Total"), "a line with no unit is not a size");
    }

    #[test]
    fn a_capability_mask_is_read_bit_by_bit() {
        assert!(has_cap("000001ffffffffff", CAP_IPC_LOCK), "root's full set");
        assert!(!has_cap("00000000a80425fb", CAP_IPC_LOCK), "a container's default set has no IPC_LOCK");
        assert!(!has_cap("garbage", CAP_IPC_LOCK));
    }

    #[test]
    #[cfg(target_env = "gnu")]
    fn glibc_takes_the_parameters_and_the_heap_grows_in_place() {
        assert!(tune_malloc().is_empty());
        let took = pregrow(1 << 20);
        assert!(took < Duration::from_secs(1));
        let r = report();
        assert_eq!(r["ok"], true);
        assert!(r["malloc"]["arena_bytes"].as_u64().unwrap() > 0);
    }
}
