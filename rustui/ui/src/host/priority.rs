//! The UI's threads by what waits on them (`docs/PLAN.md`, Priority: the UI over plugins). The compositor starts the UI at nice -10
//! (`shell::UI_NICE`), which the loop keeps: it turns keys and topics into the frames the compositor draws. The key reader, the engine's
//! events and the compositor's channel stay with it, being on a key's way to the screen. What the helper and its plugins send (their
//! topics, stills and answers) and the display sync can wait: those threads lower themselves to [`BACKGROUND_NICE`], where the helper
//! and its plugins run.

/// The nice value of the UI's background threads: the helper's own (`helper/app/src/config.rs` `nice`).
pub const BACKGROUND_NICE: i32 = 10;

/// Lowers the calling thread to [`BACKGROUND_NICE`]. A nice value is a thread's own on Linux, and a thread starts with its creator's, so a
/// background thread calls this first. Lowering needs no privilege; where it fails the thread stays as it was.
pub fn background() {
    // SAFETY: `gettid` and `setpriority` take and return plain integers and touch no memory of ours.
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, BACKGROUND_NICE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nice() -> i32 {
        // SAFETY: as above; `getpriority` of the calling thread.
        unsafe { libc::getpriority(libc::PRIO_PROCESS, libc::syscall(libc::SYS_gettid) as libc::id_t) }
    }

    #[test]
    fn a_background_thread_is_lowered_and_its_creator_is_not() {
        let before = nice();
        let lowered = std::thread::spawn(|| {
            background();
            nice()
        })
        .join()
        .unwrap();
        assert_eq!((lowered, nice()), (BACKGROUND_NICE.max(before), before));
    }
}
