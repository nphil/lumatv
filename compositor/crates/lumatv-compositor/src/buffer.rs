//! What a `wl_buffer` is to the compositor: shared memory it can read, a dma-buf it can import, or a buffer the
//! GL driver created on its own global (`eglBindWaylandDisplayWL`), which only the driver can read.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;

use parking_lot::Mutex;

use wayland_server::Resource;
use wayland_server::protocol::wl_buffer::WlBuffer;

/// DRM fourcc codes the output and the clients use. `AR24` is `wl_shm` ARGB8888 in memory order B, G, R, A.
pub mod fourcc {
    pub const ARGB8888: u32 = 0x3432_5241; // AR24
    pub const XRGB8888: u32 = 0x3432_5258; // XR24
    pub const ABGR8888: u32 = 0x3432_4241; // AB24, what LG's libmali hands over
    pub const XBGR8888: u32 = 0x3432_4258; // XB24

    pub fn name(f: u32) -> String {
        f.to_le_bytes().iter().map(|&c| if (32..127).contains(&c) { c as char } else { '?' }).collect()
    }
}

/// DRM format modifiers seen on the set.
pub mod modifier {
    pub const LINEAR: u64 = 0;
    /// What LG's libmali attaches by default: ARM AFBC, 16x16 blocks, YTR, sparse, tiled (X2 r1/r6).
    pub const AFBC_16X16_YTR_SPARSE_TILED: u64 = 0x0800_0000_0000_0151;
    /// What the OSD planes scan out: AFBC 32x8, split, sparse (DRM plane modifier list).
    pub const AFBC_32X8_SPLIT_SPARSE: u64 = 0x0800_0000_0000_0062;
}

/// A client's `wl_shm_pool`: one mapping, shared by every buffer made from it. The client may grow it with
/// `resize`, so the mapping is behind a lock and a buffer reads through it, never through a cached pointer.
pub struct ShmPool {
    map: Mutex<Mapping>,
    fd: OwnedFd,
}

struct Mapping {
    ptr: *mut u8,
    len: usize,
}

// SAFETY: the mapping is only ever read through `with_bytes`, under the lock.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl ShmPool {
    pub fn new(fd: OwnedFd, size: i32) -> std::io::Result<Arc<Self>> {
        let map = Mapping::map(fd.as_raw_fd(), size)?;
        Ok(Arc::new(ShmPool { map: Mutex::new(map), fd }))
    }

    pub fn resize(&self, size: i32) -> std::io::Result<()> {
        let new = Mapping::map(self.fd.as_raw_fd(), size)?;
        let mut m = self.map.lock();
        m.unmap();
        *m = new;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.map.lock().len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Runs `f` over the pool's bytes. A buffer whose extent lies beyond the mapping (a client lied about its
    /// size) is given an empty slice rather than a fault.
    pub fn with_bytes<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        let m = self.map.lock();
        if m.ptr.is_null() || m.len == 0 {
            return f(&[]);
        }
        // SAFETY: ptr/len describe a live MAP_SHARED read-only mapping held until unmap under this lock.
        let bytes = unsafe { std::slice::from_raw_parts(m.ptr, m.len) };
        f(bytes)
    }
}

impl Mapping {
    fn map(fd: i32, size: i32) -> std::io::Result<Mapping> {
        if size <= 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "shm pool size"));
        }
        let len = size as usize;
        // SAFETY: plain mmap of a client fd; failure is reported, the pointer is checked.
        let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, fd, 0) };
        if ptr == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Mapping { ptr: ptr as *mut u8, len })
    }

    fn unmap(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: ptr/len came from mmap above and are unmapped once.
            unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.len) };
            self.ptr = std::ptr::null_mut();
            self.len = 0;
        }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        self.unmap();
    }
}

/// A buffer made from a `wl_shm_pool`.
#[derive(Clone)]
pub struct ShmBuffer {
    pub pool: Arc<ShmPool>,
    pub offset: i32,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    /// `wl_shm.format`: 0 is ARGB8888, 1 is XRGB8888 (the others are refused at creation).
    pub format: u32,
}

impl ShmBuffer {
    /// Reads pixel (`x`, `y`) as premultiplied 0xAARRGGBB (opaque for XRGB). `None` outside the mapping.
    pub fn pixel(&self, bytes: &[u8], x: i32, y: i32) -> Option<u32> {
        let off = self.offset as usize + y as usize * self.stride as usize + x as usize * 4;
        let px = bytes.get(off..off + 4)?;
        let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
        Some(if self.format == 1 { v | 0xff00_0000 } else { v })
    }
}

/// One plane of a dma-buf.
#[derive(Debug)]
pub struct DmabufPlane {
    pub fd: OwnedFd,
    pub offset: u32,
    pub stride: u32,
}

/// A dma-buf handed over by `zwp_linux_dmabuf_v1` or by our own `mali_buffer_sharing` global. The fds live as
/// long as the buffer: C3's direct scan-out needs them for `ADDFB2`.
#[derive(Debug)]
pub struct DmabufBuffer {
    pub width: i32,
    pub height: i32,
    pub format: u32,
    pub modifier: u64,
    pub planes: Vec<DmabufPlane>,
    pub y_invert: bool,
}

/// The compositor's view of one `wl_buffer`.
pub enum BufferKind {
    Shm(ShmBuffer),
    Dmabuf(Arc<DmabufBuffer>),
    /// Created by the GL driver on the global it serves after `eglBindWaylandDisplayWL`; readable only through
    /// `EGL_WAYLAND_BUFFER_WL`. Its size is learnt at import.
    External,
}

impl BufferKind {
    pub fn kind_name(&self) -> &'static str {
        match self {
            BufferKind::Shm(_) => "shm",
            BufferKind::Dmabuf(_) => "dmabuf",
            BufferKind::External => "external",
        }
    }
}

/// The user data behind every `wl_buffer` the compositor created itself.
pub struct BufferData {
    pub kind: BufferKind,
}

/// Classifies a `wl_buffer` resource: ours (shm or dma-buf) or the driver's.
pub fn kind_of(buffer: &WlBuffer) -> BufferKindRef<'_> {
    match buffer.data::<BufferData>() {
        Some(d) => match &d.kind {
            BufferKind::Shm(s) => BufferKindRef::Shm(s),
            BufferKind::Dmabuf(d) => BufferKindRef::Dmabuf(d),
            BufferKind::External => BufferKindRef::External,
        },
        None => BufferKindRef::External,
    }
}

pub enum BufferKindRef<'a> {
    Shm(&'a ShmBuffer),
    Dmabuf(&'a Arc<DmabufBuffer>),
    External,
}

/// The size a buffer reports before import: known for shm and dma-bufs, unknown for the driver's.
pub fn size_of(buffer: &WlBuffer) -> Option<(i32, i32)> {
    match kind_of(buffer) {
        BufferKindRef::Shm(s) => Some((s.width, s.height)),
        BufferKindRef::Dmabuf(d) => Some((d.width, d.height)),
        BufferKindRef::External => None,
    }
}
