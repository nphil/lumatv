//! Host-only heap audit (feature `heap-audit`; no release build has it): a global allocator that records, for every live block, the call
//! stack it was made from, and writes the live bytes by stack on request (`lumatv-ui --host --eval MS:heap-LABEL` with
//! `LUMATV_HEAP_AUDIT=DIR`; `tests/heap_audit.py` runs the scenario and names the owners from the stacks with `addr2line`).
//!
//! The bookkeeping lives in two anonymous mappings made on first use (a pointer table and a stack table), so it never calls back into the
//! allocator; a thread that is inside it (or dumping) is marked and its allocations are not recorded. Stacks come from glibc's
//! `backtrace`, which walks the unwind tables: slow, so the harness must run with `--virtual`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

const DEPTH: usize = 24;
const PTR_BITS: u32 = 23;
const PTRS: usize = 1 << PTR_BITS;
const SITES: usize = 1 << 16;

/// One live block: its address, and its size (low 48 bits) with the stack's index above.
#[derive(Clone, Copy)]
struct Block {
    ptr: usize,
    packed: u64,
}

struct Site {
    frames: [usize; DEPTH],
    depth: u32,
    live_bytes: i64,
    live_count: i64,
    peak_bytes: i64,
    total_bytes: u64,
    total_count: u64,
}

struct Tables {
    blocks: *mut Block,
    sites: *mut Site,
    used_sites: usize,
}

static TABLES: AtomicPtr<Tables> = AtomicPtr::new(std::ptr::null_mut());
static LOCK: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static INSIDE: Cell<bool> = const { Cell::new(false) };
}

pub struct Audit;

fn map(bytes: usize) -> *mut u8 {
    // SAFETY: an anonymous private mapping; zero-filled pages are what both tables start from.
    let p = unsafe { libc::mmap(std::ptr::null_mut(), bytes, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE, -1, 0) };
    assert!(p != libc::MAP_FAILED, "heap audit: no room for its tables");
    p.cast()
}

/// Makes the tables and starts recording. Call once, first thing in `main`.
pub fn arm() {
    INSIDE.with(|i| i.set(true));
    // The first backtrace loads the unwinder, which allocates; do it with recording off.
    let mut warm = [std::ptr::null_mut(); 4];
    // SAFETY: `warm` holds the 4 pointers asked for.
    unsafe { libc::backtrace(warm.as_mut_ptr(), 4) };
    // The tables live as long as the process: a mapping of their own, never freed.
    let tables: *mut Tables = map(size_of::<Tables>()).cast();
    // SAFETY: a fresh mapping, large enough and aligned (page) for `Tables`.
    unsafe { tables.write(Tables { blocks: map(PTRS * size_of::<Block>()).cast(), sites: map(SITES * size_of::<Site>()).cast(), used_sites: 1 }) };
    TABLES.store(tables, Ordering::Release);
    ARMED.store(true, Ordering::Release);
    INSIDE.with(|i| i.set(false));
}

fn lock() {
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        std::hint::spin_loop();
    }
}

fn unlock() {
    LOCK.store(false, Ordering::Release);
}

fn slot(ptr: usize) -> usize {
    ((ptr as u64 >> 4).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - PTR_BITS)) as usize
}

/// The stack's index in the site table (0 when the table is full), made if it is new.
///
/// SAFETY: the lock is held and `t` is the armed table.
unsafe fn site_of(t: &mut Tables, frames: &[usize]) -> usize {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for f in frames {
        h = (h ^ *f as u64).wrapping_mul(0x100_0000_01b3);
    }
    let mut i = (h as usize) % (SITES - 1) + 1;
    for _ in 0..SITES {
        // SAFETY: `i` is below SITES, the mapping's length in sites.
        let s = unsafe { &mut *t.sites.add(i) };
        if s.depth == 0 {
            s.depth = frames.len() as u32;
            s.frames[..frames.len()].copy_from_slice(frames);
            t.used_sites += 1;
            return i;
        }
        if s.depth as usize == frames.len() && s.frames[..frames.len()] == *frames {
            return i;
        }
        i = i % (SITES - 1) + 1;
    }
    0
}

/// SAFETY: the lock is held and `t` is the armed table.
unsafe fn insert(t: &mut Tables, ptr: usize, size: usize, site: usize) {
    let mut i = slot(ptr);
    loop {
        // SAFETY: `i` is below PTRS.
        let b = unsafe { &mut *t.blocks.add(i) };
        if b.ptr == 0 || b.ptr == ptr {
            *b = Block { ptr, packed: size as u64 | (site as u64) << 48 };
            break;
        }
        i = (i + 1) & (PTRS - 1);
    }
    // SAFETY: `site` came from `site_of`.
    let s = unsafe { &mut *t.sites.add(site) };
    s.live_bytes += size as i64;
    s.live_count += 1;
    s.peak_bytes = s.peak_bytes.max(s.live_bytes);
    s.total_bytes += size as u64;
    s.total_count += 1;
}

/// Takes `ptr` out of the table (backward-shift deletion, so no tombstones), and its bytes off its stack.
///
/// SAFETY: the lock is held and `t` is the armed table.
unsafe fn remove(t: &mut Tables, ptr: usize) {
    let mut i = slot(ptr);
    loop {
        // SAFETY: `i` is below PTRS.
        let b = unsafe { *t.blocks.add(i) };
        if b.ptr == 0 {
            return;
        }
        if b.ptr == ptr {
            // SAFETY: the site index was stored by `insert`.
            let s = unsafe { &mut *t.sites.add((b.packed >> 48) as usize) };
            s.live_bytes -= (b.packed & ((1 << 48) - 1)) as i64;
            s.live_count -= 1;
            break;
        }
        i = (i + 1) & (PTRS - 1);
    }
    let mut hole = i;
    let mut j = i;
    loop {
        j = (j + 1) & (PTRS - 1);
        // SAFETY: `j` is below PTRS.
        let b = unsafe { *t.blocks.add(j) };
        if b.ptr == 0 {
            break;
        }
        let home = slot(b.ptr);
        // `b` may move into the hole unless its home lies cyclically between the hole (exclusive) and `j` (inclusive).
        if (j.wrapping_sub(home) & (PTRS - 1)) >= (j.wrapping_sub(hole) & (PTRS - 1)) {
            // SAFETY: both below PTRS.
            unsafe { *t.blocks.add(hole) = b };
            hole = j;
        }
    }
    // SAFETY: below PTRS.
    unsafe { *t.blocks.add(hole) = Block { ptr: 0, packed: 0 } };
}

fn record(ptr: *mut u8, size: usize) {
    if ptr.is_null() || !ARMED.load(Ordering::Acquire) || INSIDE.with(|i| i.replace(true)) {
        return;
    }
    let mut stack = [std::ptr::null_mut(); DEPTH + 2];
    // SAFETY: `stack` holds the pointers asked for.
    let n = unsafe { libc::backtrace(stack.as_mut_ptr(), (DEPTH + 2) as i32) }.max(0) as usize;
    // The first two frames are this function and the allocator's own entry.
    let mut flat = [0usize; DEPTH];
    let have = n.saturating_sub(2).min(DEPTH);
    for k in 0..have {
        flat[k] = stack[k + 2] as usize;
    }
    lock();
    // SAFETY: armed, so the table exists; the lock is held.
    unsafe {
        let t = &mut *TABLES.load(Ordering::Acquire);
        let site = site_of(t, &flat[..have.max(1)]);
        insert(t, ptr as usize, size, site);
    }
    unlock();
    INSIDE.with(|i| i.set(false));
}

fn forget(ptr: *mut u8) {
    // A thread that is dumping holds the lock and frees its own buffers: those were never recorded.
    if !ARMED.load(Ordering::Acquire) || INSIDE.with(Cell::get) {
        return;
    }
    lock();
    // SAFETY: armed, so the table exists; the lock is held.
    unsafe { remove(&mut *TABLES.load(Ordering::Acquire), ptr as usize) };
    unlock();
}

// SAFETY: every call is forwarded to the system allocator; the table only observes.
unsafe impl GlobalAlloc for Audit {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        record(p, layout.size());
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        record(p, layout.size());
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        forget(ptr);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        forget(ptr);
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        record(p, new_size);
        p
    }
}

/// Writes `DIR/LABEL.heap`: the process's own sizes, glibc's account of the heap, `extra` (the caller's lines), and one line per stack that has held bytes: `site LIVE_BYTES LIVE_COUNT TOTAL_BYTES TOTAL_COUNT PEAK_BYTES FRAME...` (hex return addresses); and `DIR/LABEL.maps`.
pub fn dump(label: &str, extra: &str) -> std::io::Result<()> {
    let Some(dir) = std::env::var_os("LUMATV_HEAP_AUDIT") else { return Ok(()) };
    INSIDE.with(|i| i.set(true));
    let mut out = String::new();
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines().filter(|l| l.starts_with("VmRSS") || l.starts_with("RssAnon") || l.starts_with("RssFile") || l.starts_with("RssShmem")) {
        let _ = writeln!(out, "# {line}");
    }
    // SAFETY: plain read of glibc's counters.
    let m = unsafe { libc::mallinfo2() };
    let _ = writeln!(out, "# mallinfo2 arena {} in_use {} free {} mmapped {} keepcost {}", m.arena, m.uordblks, m.fordblks, m.hblkhd, m.keepcost);
    out.push_str(extra);
    lock();
    // SAFETY: the table is only written under the lock.
    if let Some(t) = unsafe { TABLES.load(Ordering::Acquire).as_ref() } {
        for i in 1..SITES {
            // SAFETY: below SITES.
            let s = unsafe { &*t.sites.add(i) };
            if s.depth > 0 && s.peak_bytes > 0 {
                let _ = write!(out, "site {} {} {} {} {}", s.live_bytes, s.live_count, s.total_bytes, s.total_count, s.peak_bytes);
                for f in &s.frames[..s.depth as usize] {
                    let _ = write!(out, " {f:x}");
                }
                out.push('\n');
            }
        }
    }
    unlock();
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(format!("{label}.heap")), out)?;
    std::fs::copy("/proc/self/maps", dir.join(format!("{label}.maps")))?;
    INSIDE.with(|i| i.set(false));
    Ok(())
}
