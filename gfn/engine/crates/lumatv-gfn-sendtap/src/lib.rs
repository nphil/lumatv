//! When did the packet leave? The engine's transport is upstream code that sends with `std::net::UdpSocket::send_to`, so the only
//! place to stamp the return of that call without patching upstream is the libc symbol under it. This library, loaded with
//! `LD_PRELOAD` for a latency run only (never in normal use), wraps `send`, `sendto` and `sendmsg`: each successful call stores the
//! `CLOCK_MONOTONIC` time of its return in a ring, and `lumatv_sendtap_first_after` answers "when did the first send after T
//! return?" for the pad crate's latency tap (docs/research/gfn-input-result.md, "Phase 4").
//!
//! The ring is lock-free and fixed (8192 sends: seconds of traffic at the stream's packet rate); a wrapper costs one clock read and
//! one atomic store. Nothing else about the call is changed or kept: no address, no payload.
use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const RING: usize = 8192;

static ENDS: [AtomicU64; RING] = [const { AtomicU64::new(0) }; RING];
static NEXT: AtomicUsize = AtomicUsize::new(0);
static REAL_SEND: AtomicUsize = AtomicUsize::new(0);
static REAL_SENDTO: AtomicUsize = AtomicUsize::new(0);
static REAL_SENDMSG: AtomicUsize = AtomicUsize::new(0);

fn mono_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a valid out-parameter.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

fn record(result: isize) {
    if result >= 0 {
        let i = NEXT.fetch_add(1, Ordering::Relaxed) % RING;
        ENDS[i].store(mono_ns(), Ordering::Release);
    }
}

fn next_symbol(cache: &AtomicUsize, name: &std::ffi::CStr) -> usize {
    let mut p = cache.load(Ordering::Relaxed);
    if p == 0 {
        // SAFETY: RTLD_NEXT with a NUL-terminated name.
        p = unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr()) } as usize;
        cache.store(p, Ordering::Relaxed);
    }
    p
}

/// # Safety
/// Called by the program's own code with `send`'s contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn send(fd: c_int, buf: *const c_void, len: usize, flags: c_int) -> isize {
    type F = unsafe extern "C" fn(c_int, *const c_void, usize, c_int) -> isize;
    let p = next_symbol(&REAL_SEND, c"send");
    if p == 0 {
        return -1;
    }
    // SAFETY: the address is libc's `send`, the next definition after this library's.
    let r = unsafe { std::mem::transmute::<usize, F>(p)(fd, buf, len, flags) };
    record(r);
    r
}

/// # Safety
/// Called by the program's own code with `sendto`'s contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sendto(fd: c_int, buf: *const c_void, len: usize, flags: c_int, addr: *const libc::sockaddr, alen: libc::socklen_t) -> isize {
    type F = unsafe extern "C" fn(c_int, *const c_void, usize, c_int, *const libc::sockaddr, libc::socklen_t) -> isize;
    let p = next_symbol(&REAL_SENDTO, c"sendto");
    if p == 0 {
        return -1;
    }
    // SAFETY: as for `send`.
    let r = unsafe { std::mem::transmute::<usize, F>(p)(fd, buf, len, flags, addr, alen) };
    record(r);
    r
}

/// # Safety
/// Called by the program's own code with `sendmsg`'s contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sendmsg(fd: c_int, msg: *const libc::msghdr, flags: c_int) -> isize {
    type F = unsafe extern "C" fn(c_int, *const libc::msghdr, c_int) -> isize;
    let p = next_symbol(&REAL_SENDMSG, c"sendmsg");
    if p == 0 {
        return -1;
    }
    // SAFETY: as for `send`.
    let r = unsafe { std::mem::transmute::<usize, F>(p)(fd, msg, flags) };
    record(r);
    r
}

/// The `CLOCK_MONOTONIC` nanoseconds at which the first send that returned at or after `after_ns` returned; 0 when none did (yet).
#[unsafe(no_mangle)]
pub extern "C" fn lumatv_sendtap_first_after(after_ns: u64) -> u64 {
    ENDS.iter().map(|e| e.load(Ordering::Acquire)).filter(|&t| t >= after_ns && t != 0).min().unwrap_or(0)
}

/// How many sends have been stamped since the library loaded.
#[unsafe(no_mangle)]
pub extern "C" fn lumatv_sendtap_count() -> u64 {
    NEXT.load(Ordering::Relaxed) as u64
}
