//! The process's allocator, set for pictures that come and go. The weather view's map arrives as a body of 12 MB and is copied into a layer, and
//! a zoom brings another a moment later. glibc's mmap threshold starts at 128 KiB and rises to the size of every block of it that is freed (to
//! 32 MB at most), and then serves blocks of that size from the heap, where freeing them leaves the memory with the process: a closed view held
//! 26 MB of heap (`tests/weather_memory.rs`) that the process would have kept until it was restarted. A threshold that is set stays where it is, so
//! a block of 1 MiB or more is a mapping of its own and goes back to the system when it is freed.

/// The size from which a block is a mapping of its own.
pub const MMAP_THRESHOLD: i32 = 1 << 20;

/// Fixes the allocator's mmap threshold. Called once, before the first picture.
pub fn tune() {
    #[cfg(target_env = "gnu")]
    // SAFETY: `mallopt` only sets the allocator's parameters; nothing of ours is touched.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, MMAP_THRESHOLD);
    }
}

/// Turns core dumps off for the UI and what it starts. SAM and the boot hook start processes with an unlimited core size and the kernel's core
/// pattern is the relative `core`, so a crash drops a 160-290 MB file (a copy of this memory, mode 0777) into the working directory. The
/// compositor's crash bundle is the evidence that is kept. The hard limit goes too, so nothing below can raise it again.
pub fn no_core_dumps() -> std::io::Result<()> {
    let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: a stack rlimit for setrlimit to read.
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_soft_and_the_hard_core_limit_are_zero() {
        no_core_dumps().unwrap();
        let mut now = libc::rlimit { rlim_cur: 1, rlim_max: 1 };
        // SAFETY: a stack rlimit for getrlimit to fill.
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut now) }, 0);
        assert_eq!((now.rlim_cur, now.rlim_max), (0, 0));
    }
}
