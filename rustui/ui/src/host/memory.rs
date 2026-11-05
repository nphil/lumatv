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
