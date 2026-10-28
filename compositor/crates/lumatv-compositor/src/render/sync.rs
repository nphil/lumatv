//! Reading a GL native fence (`EGL_ANDROID_native_fence_sync`, a kernel sync file): whether it has signalled and when,
//! on the clock the page flips and the log carry.

use std::os::fd::{AsRawFd, OwnedFd};

/// `struct sync_fence_info` and `struct sync_file_info` of `<linux/sync_file.h>`, and `SYNC_IOC_FILE_INFO`
/// (`_IOWR('>', 4, struct sync_file_info)`, 56 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct SyncFenceInfo {
    obj_name: [u8; 32],
    driver_name: [u8; 32],
    status: i32,
    flags: u32,
    timestamp_ns: u64,
}

#[repr(C)]
struct SyncFileInfo {
    name: [u8; 32],
    status: i32,
    flags: u32,
    num_fences: u32,
    pad: u32,
    sync_fence_info: u64,
}

const SYNC_IOC_FILE_INFO: libc::c_ulong = ((3u64 << 30) | ((std::mem::size_of::<SyncFileInfo>() as u64) << 16) | ((b'>' as u64) << 8) | 4) as libc::c_ulong;

/// When the sync file `fence` signalled: the latest of its fences' timestamps, CLOCK_MONOTONIC ns (`dma_fence` stamps
/// with `ktime_get`). `None` while it has not, for a file of more fences than a GL fence has, or when the kernel does
/// not answer.
pub fn signal_ns(fence: &OwnedFd) -> Option<u64> {
    const MOST: usize = 4;
    let mut info = SyncFileInfo { name: [0; 32], status: 0, flags: 0, num_fences: 0, pad: 0, sync_fence_info: 0 };
    // SAFETY: an ioctl on a sync file with a struct of the kernel's layout; `num_fences` 0 asks for the count alone.
    if unsafe { libc::ioctl(fence.as_raw_fd(), SYNC_IOC_FILE_INFO, &mut info) } != 0 || info.status != 1 {
        return None;
    }
    let count = info.num_fences as usize;
    if count == 0 || count > MOST {
        return None;
    }
    let mut fences = [SyncFenceInfo { obj_name: [0; 32], driver_name: [0; 32], status: 0, flags: 0, timestamp_ns: 0 }; MOST];
    info.num_fences = count as u32;
    info.sync_fence_info = fences.as_mut_ptr() as u64;
    // SAFETY: the kernel writes `count` entries into the array, which holds MOST; `count` was checked above.
    if unsafe { libc::ioctl(fence.as_raw_fd(), SYNC_IOC_FILE_INFO, &mut info) } != 0 {
        return None;
    }
    fences[..count].iter().filter(|f| f.status == 1 && f.timestamp_ns != 0).map(|f| f.timestamp_ns).max()
}
