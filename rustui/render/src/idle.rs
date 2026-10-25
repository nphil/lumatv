//! The compositor's quiet moments: what it does when it has had nothing to draw for a while.
//!
//! * **The trim** (always): after [`TRIM_AFTER_NS`] without a commit or a frame it gives back what it does not need while it stands
//!   still, once for each quiet period: the GPU memory that only makes the next frame cheaper (`Renderer::trim`: the pool's free
//!   targets, cached backdrops and plates of glass nobody sees) and the heap pages the C library has freed but not returned
//!   ([`Host::purge_heap`]). The resident size of the compositor counts both.
//! * **The census** (`LUMASHOW_CENSUS=1` in the compositor's environment, off otherwise: nothing is read, nothing is logged, no wake
//!   is asked for): one line [`CENSUS_AFTER_NS`] into a quiet period, and not within [`CENSUS_EVERY_NS`] of the one before, of what
//!   the process holds (`/proc/self/status`, `mallinfo`) and what the renderer holds on the GPU by kind (`Renderer::census`); and
//!   one line after each trim with what it let go of and the sizes before and after.
//! * **The catch-up** (always): [`CATCH_UP_AFTER_NS`] into a quiet period, once, the picture the next frame is drawn into is brought up
//!   to date with the screen when it owes enough to be worth it (`engine::FrameLoop::catch_up`). A frame is drawn into the target the
//!   frame before did not go into, which holds the screen as it was two frames ago, so it first draws what the frame before changed;
//!   after a frame that changed the whole panel and nothing that shows (the commits that make hidden pages draw no frame) the next
//!   frame, a finger's, draws the whole panel for a change of a few hundred pixels. The catch-up draws what is owed while nothing is
//!   asked and shows nothing, so that frame draws what it changes. This module only says when.
//!
//! Everything here takes the time as an argument and talks to the system through [`Host`], so the policy is tested without a clock
//! or a process.

use crate::census::Census;
use crate::log::log;
use crate::renderer::Renderer;
use crate::renderer::trim::Trimmed;
use std::fmt;

/// The environment variable that turns the census on (`1`).
pub const ENV: &str = "LUMASHOW_CENSUS";

/// How long without a frame or a commit before the census line.
pub const CENSUS_AFTER_NS: u64 = 3_000_000_000;
/// The least time between two census lines.
pub const CENSUS_EVERY_NS: u64 = 10_000_000_000;
/// How long without a frame or a commit before the trim.
pub const TRIM_AFTER_NS: u64 = 5_000_000_000;
/// How long a quiet period has lasted before the pictures the next frames are drawn into are brought up to date: two refreshes (a
/// commit that follows the frame before within them is looked at first, whatever it asks).
pub const CATCH_UP_AFTER_NS: u64 = 34_000_000;

/// The C library's heap, as `mallinfo` counts it, in kilobytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Heap {
    pub arena_kb: u64,
    pub in_use_kb: u64,
    pub free_kb: u64,
}

/// What the process holds, in kilobytes: its resident size by kind, and the heap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub rss_kb: u64,
    pub anon_kb: u64,
    pub file_kb: u64,
    pub shmem_kb: u64,
    pub heap: Heap,
}

impl fmt::Display for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rss {} kB (anon {}, file {}, shmem {}); heap arena {} kB, in use {} kB, free {} kB",
            self.rss_kb,
            self.anon_kb,
            self.file_kb,
            self.shmem_kb,
            self.heap.arena_kb,
            self.heap.in_use_kb,
            self.heap.free_kb
        )
    }
}

/// The parts of `/proc/PID/status` that make a [`Memory`] (the heap is left at zero).
pub fn parse_status(status: &str) -> Memory {
    let mut memory = Memory::default();
    for line in status.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let slot = match name {
            "VmRSS" => &mut memory.rss_kb,
            "RssAnon" => &mut memory.anon_kb,
            "RssFile" => &mut memory.file_kb,
            "RssShmem" => &mut memory.shmem_kb,
            _ => continue,
        };
        *slot = rest
            .split_whitespace()
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
    }
    memory
}

/// What the quiet moment asks of the system.
pub trait Host {
    /// One line of the log.
    fn log(&mut self, line: &str);
    /// What the process holds now.
    fn memory(&mut self) -> Memory;
    /// Asks the C library to return the heap pages it has freed; what it used, or why it could not.
    fn purge_heap(&mut self) -> Result<&'static str, String>;
}

/// The running process.
pub struct Process;

impl Host for Process {
    fn log(&mut self, line: &str) {
        log(line);
    }

    fn memory(&mut self) -> Memory {
        let mut memory = std::fs::read_to_string("/proc/self/status")
            .map(|status| parse_status(&status))
            .unwrap_or_default();
        // SAFETY: `mallinfo` reads the allocator's counters.
        #[allow(deprecated)]
        let info = unsafe { libc::mallinfo() };
        let kb = |bytes| u64::try_from(bytes).unwrap_or(0) / 1024;
        memory.heap = Heap {
            arena_kb: kb(info.arena),
            in_use_kb: kb(info.uordblks),
            free_kb: kb(info.fordblks),
        };
        memory
    }

    fn purge_heap(&mut self) -> Result<&'static str, String> {
        /// `mallopt`'s option that makes bionic return the free pages of its heap.
        const M_PURGE: libc::c_int = -101;
        // The symbols are looked up and not linked: the C library of the Echo's Android may not have either of them, and the
        // compositor must start on it all the same.
        // SAFETY: `malloc_trim` and `mallopt` are called as their C declarations have them, with what they take.
        unsafe {
            let trim = libc::dlsym(libc::RTLD_DEFAULT, c"malloc_trim".as_ptr());
            if !trim.is_null() {
                let trim: extern "C" fn(libc::size_t) -> libc::c_int = std::mem::transmute(trim);
                trim(0);
                return Ok("malloc_trim");
            }
            let mallopt = libc::dlsym(libc::RTLD_DEFAULT, c"mallopt".as_ptr());
            if mallopt.is_null() {
                return Err("the C library has neither malloc_trim nor mallopt".into());
            }
            let mallopt: extern "C" fn(libc::c_int, libc::c_int) -> libc::c_int =
                std::mem::transmute(mallopt);
            if mallopt(M_PURGE, 0) != 0 {
                Ok("mallopt(M_PURGE)")
            } else {
                Err("mallopt(M_PURGE) is refused by this C library".into())
            }
        }
    }
}

/// What the renderer gives the quiet moment.
pub trait Gpu {
    /// What it holds on the GPU, by kind.
    fn census(&self) -> Census;
    /// Gives back what only makes the next frame cheaper.
    fn trim(&mut self) -> Trimmed;
}

impl Gpu for Renderer {
    fn census(&self) -> Census {
        Renderer::census(self)
    }

    fn trim(&mut self) -> Trimmed {
        Renderer::trim(self)
    }
}

/// Where the compositor is in a quiet period.
pub struct Idle {
    census: bool,
    /// The last time something happened (a commit, a frame, a scene that wants frames).
    since: u64,
    /// Whether this quiet period has had its census line and its trim.
    censused: bool,
    trimmed: bool,
    /// Whether this quiet period has had its catch-up (`Idle::catch_up_at`).
    caught_up: bool,
    last_census: Option<u64>,
    /// Whether the heap's not being given back has been logged: once.
    purge_told: bool,
}

impl Idle {
    /// `census`: whether the census lines are logged. `now`: the time the compositor started, from which its first quiet period runs.
    pub fn new(census: bool, now: u64) -> Self {
        Self {
            census,
            since: now,
            censused: false,
            trimmed: false,
            caught_up: false,
            last_census: None,
            purge_told: false,
        }
    }

    /// As the environment says ([`ENV`]).
    pub fn from_env(now: u64) -> Self {
        Self::new(std::env::var(ENV).as_deref() == Ok("1"), now)
    }

    /// Something happened at `now`: a new quiet period begins when it is over.
    pub fn active(&mut self, now: u64) {
        self.since = now;
        self.censused = false;
        self.trimmed = false;
        self.caught_up = false;
    }

    /// When the loop has to wake to bring the pictures its frames are drawn into up to date with the screen (`engine::FrameLoop::
    /// catch_up`): a moment after the quiet began, once. `owes`: the loop is ready and a picture owes enough to be worth it. `None`
    /// when it does not, and when this quiet period has had its catch-up ([`Idle::catch_up_done`]).
    pub fn catch_up_at(&self, owes: bool) -> Option<u64> {
        (owes && !self.caught_up).then(|| self.since.saturating_add(CATCH_UP_AFTER_NS))
    }

    /// The catch-up was made (or could not be): not again until something happens. It is not activity: the trim's clock runs on.
    pub fn catch_up_done(&mut self) {
        self.caught_up = true;
    }

    /// When the census line is due in this quiet period.
    fn census_at(&self) -> u64 {
        let after = self.since.saturating_add(CENSUS_AFTER_NS);
        self.last_census.map_or(after, |last| {
            after.max(last.saturating_add(CENSUS_EVERY_NS))
        })
    }

    /// When the loop has to wake to do what is due, if anything is: `None` when something is going on (`ready` false: a frame
    /// is wanted, a commit waits, the scene animates) or when the quiet period has had what it is due.
    pub fn wake_at(&self, ready: bool) -> Option<u64> {
        if !ready {
            return None;
        }
        let census = (self.census && !self.censused).then(|| self.census_at());
        let trim = (!self.trimmed).then(|| self.since.saturating_add(TRIM_AFTER_NS));
        match (census, trim) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Does what is due at `now`. `ready`: nothing is going on (no frame in flight, none wanted, no commit waiting, the scene idle);
    /// without it nothing is done, whatever the time is. What is done is done once for a quiet period, so a caller that wakes at
    /// [`Idle::wake_at`] and calls this makes progress each time.
    pub fn moment(&mut self, gpu: &mut impl Gpu, host: &mut impl Host, now: u64, ready: bool) {
        if !ready {
            return;
        }
        if self.census && !self.censused && now >= self.census_at() {
            self.censused = true;
            self.last_census = Some(now);
            let memory = host.memory();
            host.log(&format!("census: {memory}; {}", census_text(&gpu.census())));
        }
        if !self.trimmed && now >= self.since.saturating_add(TRIM_AFTER_NS) {
            self.trimmed = true;
            self.trim(gpu, host);
        }
    }

    fn trim(&mut self, gpu: &mut impl Gpu, host: &mut impl Host) {
        let before = self.census.then(|| host.memory());
        let freed = gpu.trim();
        let purged = host.purge_heap();
        if let Err(e) = &purged {
            if !std::mem::replace(&mut self.purge_told, true) {
                host.log(&format!("idle trim: the heap is not given back: {e}"));
            }
        }
        if let Some(before) = before {
            let after = host.memory();
            let how = purged.map_or_else(|e| format!("no ({e})"), |how| how.to_string());
            host.log(&format!(
                "trim: let go of {freed} ({} KiB of GPU memory); heap purge {how}; rss {} -> {} kB, heap free {} -> {} kB, heap arena {} -> {} kB",
                freed.bytes().div_ceil(1024),
                before.rss_kb,
                after.rss_kb,
                before.heap.free_kb,
                after.heap.free_kb,
                before.heap.arena_kb,
                after.heap.arena_kb
            ));
        }
    }
}

/// The renderer's side of a census line: what it holds by kind, and the sizes in the pool.
fn census_text(census: &Census) -> String {
    let sizes: Vec<String> = census
        .pool_sizes
        .iter()
        .map(|((w, h), n)| format!("{w}x{h}x{n}"))
        .collect();
    format!(
        "{census}; pool sizes [{}]; atlas pages {}",
        sizes.join(" "),
        census.atlases.count
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A system that records what it is asked.
    #[derive(Default)]
    struct Fake {
        lines: Vec<String>,
        memories: u32,
        purges: u32,
        purge: Option<String>,
        rss_kb: u64,
    }

    impl Host for Fake {
        fn log(&mut self, line: &str) {
            self.lines.push(line.to_string());
        }

        fn memory(&mut self) -> Memory {
            self.memories += 1;
            // Each read is smaller than the one before: what a trim is for.
            self.rss_kb = self.rss_kb.saturating_sub(1000);
            Memory {
                rss_kb: self.rss_kb,
                ..Memory::default()
            }
        }

        fn purge_heap(&mut self) -> Result<&'static str, String> {
            self.purges += 1;
            match &self.purge {
                Some(e) => Err(e.clone()),
                None => Ok("fake"),
            }
        }
    }

    /// A renderer that holds a pool and counts the calls.
    #[derive(Default)]
    struct Pool {
        censuses: std::cell::Cell<u32>,
        trims: u32,
    }

    impl Gpu for Pool {
        fn census(&self) -> Census {
            self.censuses.set(self.censuses.get() + 1);
            let mut census = Census::default();
            census.pool_free.texture(240, 120, 4);
            census.pool_sizes.insert((240, 120), 1);
            census
        }

        fn trim(&mut self) -> Trimmed {
            self.trims += 1;
            let mut freed = Trimmed::default();
            freed.pool.texture(240, 120, 4);
            freed
        }
    }

    const SECOND: u64 = 1_000_000_000;

    #[test]
    fn the_status_of_a_process_gives_its_resident_sizes() {
        let status = "Name:\tcompositor\nVmRSS:\t   49100 kB\nRssAnon:\t   13200 kB\nRssFile:\t   35900 kB\nRssShmem:\t       0 kB\nThreads:\t10\n";
        let memory = parse_status(status);
        assert_eq!(
            memory,
            Memory {
                rss_kb: 49100,
                anon_kb: 13200,
                file_kb: 35900,
                ..Memory::default()
            }
        );
        assert_eq!(parse_status("garbage"), Memory::default());
    }

    #[test]
    fn the_running_process_can_be_read_and_asked_to_give_back_its_heap() {
        let mut process = Process;
        let memory = process.memory();
        assert!(memory.rss_kb > 0, "{memory}");
        assert!(memory.heap.arena_kb > 0, "{memory}");
        // The host's C library has `malloc_trim`; the Echo's may not, and says so.
        if let Err(e) = process.purge_heap() {
            assert!(e.contains("malloc"), "{e}");
        }
    }

    #[test]
    fn nothing_is_read_logged_or_woken_for_without_the_census_and_the_trim_is_silent() {
        let (mut gpu, mut host) = (Pool::default(), Fake::default());
        let mut idle = Idle::new(false, 0);
        assert_eq!(idle.wake_at(true), Some(TRIM_AFTER_NS), "the trim only");
        for second in 0..60 {
            idle.moment(&mut gpu, &mut host, second * SECOND, true);
        }
        assert!(host.lines.is_empty(), "{:?}", host.lines);
        assert_eq!(host.memories, 0, "no /proc read, no mallinfo");
        assert_eq!(gpu.censuses.get(), 0);
        assert_eq!((gpu.trims, host.purges), (1, 1), "one trim, once");
        assert_eq!(idle.wake_at(true), None, "nothing more is due");
    }

    #[test]
    fn a_heap_that_cannot_be_given_back_is_said_once_and_the_trim_goes_on() {
        let (mut gpu, mut host) = (Pool::default(), Fake::default());
        host.purge = Some("no such thing".into());
        let mut idle = Idle::new(false, 0);
        for round in 0..3 {
            idle.active(round * 100 * SECOND);
            idle.moment(&mut gpu, &mut host, round * 100 * SECOND + 6 * SECOND, true);
        }
        assert_eq!(
            (gpu.trims, host.purges),
            (3, 3),
            "tried at each quiet period"
        );
        assert_eq!(host.lines.len(), 1, "told once: {:?}", host.lines);
        assert!(host.lines[0].contains("no such thing"));
    }

    #[test]
    fn the_census_line_comes_three_seconds_in_the_trim_at_five_and_each_once_for_a_quiet_period() {
        let (mut gpu, mut host) = (Pool::default(), Fake::default());
        host.rss_kb = 50_000;
        let mut idle = Idle::new(true, 10 * SECOND);
        assert_eq!(idle.wake_at(true), Some(13 * SECOND));
        idle.moment(&mut gpu, &mut host, 12 * SECOND, true);
        assert!(host.lines.is_empty(), "not yet");
        idle.moment(&mut gpu, &mut host, 13 * SECOND, true);
        assert_eq!(host.lines.len(), 1);
        let line = &host.lines[0];
        assert!(line.starts_with("census: rss 49000 kB"), "{line}");
        assert!(line.contains("gpu 113 KiB"), "{line}");
        assert!(line.contains("pool free 1 (113 KiB)"), "{line}");
        assert!(line.contains("pool sizes [240x120x1]"), "{line}");
        assert!(line.contains("graveyard 0"), "{line}");
        assert_eq!(idle.wake_at(true), Some(15 * SECOND), "then the trim");
        idle.moment(&mut gpu, &mut host, 15 * SECOND, true);
        assert_eq!(host.lines.len(), 2);
        let line = &host.lines[1];
        assert!(
            line.starts_with("trim: let go of pool 1 (113 KiB)"),
            "{line}"
        );
        assert!(line.contains("rss 48000 -> 47000 kB"), "{line}");
        assert_eq!((gpu.trims, host.purges), (1, 1));
        assert_eq!(idle.wake_at(true), None);
        idle.moment(&mut gpu, &mut host, 500 * SECOND, true);
        assert_eq!(host.lines.len(), 2, "once for a quiet period");
        assert_eq!((gpu.trims, host.purges), (1, 1));
    }

    #[test]
    fn census_lines_are_ten_seconds_apart_at_the_least() {
        let (mut gpu, mut host) = (Pool::default(), Fake::default());
        let mut idle = Idle::new(true, 0);
        let censuses = |host: &Fake| {
            host.lines
                .iter()
                .filter(|l| l.starts_with("census:"))
                .count()
        };
        idle.moment(&mut gpu, &mut host, 3 * SECOND, true);
        assert_eq!(censuses(&host), 1);
        // A frame at 4 s, quiet again from then: the line is due at 7 s, too soon after the one at 3 s; the trim, at 9 s, is not
        // held up by it.
        idle.active(4 * SECOND);
        assert_eq!(idle.wake_at(true), Some(9 * SECOND));
        idle.moment(&mut gpu, &mut host, 8 * SECOND, true);
        assert_eq!(censuses(&host), 1);
        idle.moment(&mut gpu, &mut host, 9 * SECOND, true);
        assert_eq!(gpu.trims, 1);
        assert_eq!(idle.wake_at(true), Some(13 * SECOND), "ten after the first");
        idle.moment(&mut gpu, &mut host, 13 * SECOND, true);
        assert_eq!(censuses(&host), 2);
    }

    #[test]
    fn nothing_is_done_while_frames_flow_or_a_commit_waits() {
        let (mut gpu, mut host) = (Pool::default(), Fake::default());
        let mut idle = Idle::new(true, 0);
        // Frames every 16 ms for a minute: the quiet period starts over each time.
        for frame in 0..3750u64 {
            let now = frame * 16_000_000;
            idle.active(now);
            idle.moment(&mut gpu, &mut host, now, true);
        }
        assert!(host.lines.is_empty());
        assert_eq!((gpu.trims, host.purges, host.memories), (0, 0, 0));
        // A quiet screen with a commit waiting, a frame wanted or a scene that animates is not ready, whatever the time is, and
        // asks for no wake.
        let mut idle = Idle::new(true, 0);
        assert_eq!(idle.wake_at(false), None);
        idle.moment(&mut gpu, &mut host, 100 * SECOND, false);
        assert!(host.lines.is_empty());
        assert_eq!((gpu.trims, host.purges, host.memories), (0, 0, 0));
        // When it is ready, all that is due is done.
        idle.moment(&mut gpu, &mut host, 100 * SECOND, true);
        assert_eq!((gpu.trims, host.purges), (1, 1));
    }

    #[test]
    fn the_catch_up_is_due_two_refreshes_into_a_quiet_period_once_and_when_something_is_owed() {
        let mut idle = Idle::new(false, 100);
        assert_eq!(idle.catch_up_at(false), None, "nothing owed, or not ready");
        assert_eq!(idle.catch_up_at(true), Some(100 + CATCH_UP_AFTER_NS));
        idle.catch_up_done();
        assert_eq!(idle.catch_up_at(true), None, "once for a quiet period");
        // A frame, a commit: a new quiet period, with a catch-up of its own.
        idle.active(5 * SECOND);
        assert_eq!(idle.catch_up_at(true), Some(5 * SECOND + CATCH_UP_AFTER_NS));
        // A commit that follows the frame before within the two refreshes starts the wait over: it is looked at first.
        idle.active(5 * SECOND + CATCH_UP_AFTER_NS - 1);
        assert_eq!(
            idle.catch_up_at(true),
            Some(5 * SECOND + 2 * CATCH_UP_AFTER_NS - 1)
        );
    }

    #[test]
    fn the_catch_up_is_no_activity_so_the_trim_keeps_its_own_time() {
        let (mut gpu, mut host) = (Pool::default(), Fake::default());
        let mut idle = Idle::new(false, 0);
        idle.active(SECOND);
        idle.catch_up_done();
        assert_eq!(
            idle.wake_at(true),
            Some(SECOND + TRIM_AFTER_NS),
            "the trim is due where it was, not after the catch-up"
        );
        idle.moment(&mut gpu, &mut host, SECOND + TRIM_AFTER_NS, true);
        assert_eq!(gpu.trims, 1);
    }
}
