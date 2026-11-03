//! How the threads that carry a frame to the panel are scheduled, agreed between the processes they are in.
//!
//! A frame reaches the screen through a chain of wake-ups: the display's interrupt, the kernel worker that releases the
//! buffer's fence, the display service that waits for it, the compositor's present thread that waits for the service's
//! answer. On the Echo Show the eMMC's request thread (`mmcqd`) runs at real-time priority 1 and, while flash is being
//! written (the logs, a record), takes its core back from an ordinary thread every few microseconds: measured on the
//! Show (notes/g2/present-leg, 10-08), the display service was woken and then waited 0.3-0.5 ms for its core behind it,
//! and the compositor's stamp of the flip came that much later. A thread of the chain that is real-time, above `mmcqd`,
//! runs as soon as it is woken.
//!
//! The two threads here do bounded work and sleep between frames (ioctls, a fence wait, a socket round trip): they are
//! safe at a real-time priority. The render thread, which draws for milliseconds, stays an ordinary thread at nice -10.

use std::io;

/// The display service's one thread (`lumashow-displayd`).
pub const DISPLAY_SERVICE_PRIORITY: i32 = 10;
/// The compositor's present thread. The compositor runs without privileges: the service that starts it raises its
/// `RLIMIT_RTPRIO` to at least this before it drops them ([`RTPRIO_LIMIT`]).
pub const PRESENT_PRIORITY: i32 = 10;
/// The real-time priority limit the compositor's identity is given: what [`PRESENT_PRIORITY`] needs.
pub const RTPRIO_LIMIT: u64 = PRESENT_PRIORITY as u64;

/// Makes the calling thread real-time (`SCHED_FIFO`) at `priority`. Fails with `EPERM` where the thread may not (no
/// `CAP_SYS_NICE` and an `RLIMIT_RTPRIO` below `priority`): the caller stays an ordinary thread and says so.
///
/// The kernel is asked directly: musl's `sched_setscheduler` is a stub that answers `ENOSYS` (POSIX's is per process, the
/// kernel's per thread, and musl will not pretend), and the display service is a musl binary.
pub fn enter_realtime(priority: i32) -> io::Result<()> {
    // SAFETY: `sched_param` is a plain C struct (musl's has sporadic-server fields besides the priority) and all zeros is a
    // value of it; only the priority is read for SCHED_FIFO. The call is the kernel's own, on the calling thread (0), with a
    // complete `sched_param` the kernel only reads.
    let mut param: libc::sched_param = unsafe { std::mem::zeroed() };
    param.sched_priority = priority;
    let done = unsafe {
        libc::syscall(
            libc::SYS_sched_setscheduler,
            0,
            libc::SCHED_FIFO,
            &param as *const libc::sched_param,
        )
    };
    if done != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The scheduling policy and priority of the calling thread, as the kernel has them (asked directly, as above).
pub fn current() -> io::Result<(i32, i32)> {
    // SAFETY: plain scheduler queries on the calling thread; `param` is a plain C struct the kernel fills in.
    let policy = unsafe { libc::syscall(libc::SYS_sched_getscheduler, 0) };
    if policy < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut param: libc::sched_param = unsafe { std::mem::zeroed() };
    let got = unsafe {
        libc::syscall(
            libc::SYS_sched_getparam,
            0,
            &mut param as *mut libc::sched_param,
        )
    };
    if got != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((policy as i32, param.sched_priority))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether this process may make a thread real-time at `priority`: root with `CAP_SYS_NICE`, or a limit that allows it.
    fn permitted(priority: i32) -> bool {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: a plain query; `limit` is written by the kernel.
        let got = unsafe { libc::getrlimit(libc::RLIMIT_RTPRIO, &mut limit) } == 0;
        let by_limit = got && limit.rlim_cur >= priority as u64;
        // CAP_SYS_NICE lets any priority through; a probe with the lowest tells.
        let probe = std::thread::spawn(|| enter_realtime(1).is_ok())
            .join()
            .unwrap_or(false);
        by_limit || probe
    }

    #[test]
    fn the_threads_of_the_chain_ask_for_a_priority_above_the_emmc_request_thread() {
        // `mmcqd` runs at SCHED_FIFO 1 on the Show's kernel; both threads must stand above it, and below the kernel's own
        // display threads (SCHED_RR 87 and 94).
        for priority in [DISPLAY_SERVICE_PRIORITY, PRESENT_PRIORITY] {
            assert!((2..87).contains(&priority), "{priority}");
        }
        assert!(RTPRIO_LIMIT >= PRESENT_PRIORITY as u64);
    }

    #[test]
    fn entering_real_time_makes_the_thread_fifo_at_the_priority_asked_or_says_it_may_not() {
        let outcome = std::thread::spawn(|| {
            let entered = enter_realtime(PRESENT_PRIORITY);
            (entered.map_err(|e| e.raw_os_error()), current().unwrap())
        })
        .join()
        .unwrap();
        match outcome {
            (Ok(()), (policy, priority)) => {
                assert_eq!((policy, priority), (libc::SCHED_FIFO, PRESENT_PRIORITY));
            }
            (Err(code), (policy, _)) => {
                // Where it may not (a container without CAP_SYS_NICE): the refusal is the kernel's, and the thread is as
                // it was.
                assert!(
                    !permitted(PRESENT_PRIORITY),
                    "allowed to, but refused with {code:?}"
                );
                assert_eq!(code, Some(libc::EPERM));
                assert_eq!(policy, libc::SCHED_OTHER);
            }
        }
    }
}
