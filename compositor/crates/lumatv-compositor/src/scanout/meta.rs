//! What the driver was told about the buffers it serves. With `--buffers driver` the Mali blob creates the `wl_buffer`s of LG's clients
//! itself (`eglBindWaylandDisplayWL`), so the compositor never sees the dma-buf descriptor its client sent: only the driver can read the
//! resource. The protocol tap (`tap.rs`) copies the descriptor out of the request on its way to the driver, and keeps it here, by the
//! client's `wl_buffer` id, for as long as that buffer lives. The descriptor is metadata plus a duplicate of the fd; the driver goes on
//! creating, owning and importing the resource as before.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;

use parking_lot::Mutex;
use wayland_server::protocol::wl_buffer::WlBuffer;

/// `(wl_client, wl_buffer id)`. The client part is a pointer used for identity only, never dereferenced here.
pub type Key = (usize, u32);

/// One `mali_buffer_sharing.create_buffer`, as the client sent it.
#[derive(Debug)]
pub struct Descriptor {
    pub width: i32,
    pub height: i32,
    pub stride: u32,
    pub format: u32,
    pub modifier: u64,
    /// A duplicate of the client's dma-buf fd, ours until the buffer is destroyed (or the client leaves).
    pub fd: OwnedFd,
    /// Which `create_buffer` this is, counted from 1 by the registry: an id reused after its buffer went is another descriptor.
    pub serial: u64,
}

impl Descriptor {
    /// The dma-buf's size in bytes (`lseek` to its end), when the fd answers.
    pub fn bytes(&self) -> Option<u64> {
        fd_bytes(self.fd.as_raw_fd())
    }
}

/// The size of a dma-buf fd in bytes: `lseek(SEEK_END)` is what the kernel's dma-buf files answer.
pub fn fd_bytes(fd: i32) -> Option<u64> {
    // SAFETY: lseek on a descriptor the caller holds; the file offset of a dma-buf is not used for reads.
    let end = unsafe { libc::lseek(fd, 0, libc::SEEK_END) };
    if end < 0 {
        return None;
    }
    // SAFETY: back to the start, as the kernel's dma-buf llseek allows (it accepts offset 0 only).
    unsafe { libc::lseek(fd, 0, libc::SEEK_SET) };
    Some(end as u64)
}

#[derive(Default)]
struct Inner {
    map: HashMap<Key, Arc<Descriptor>>,
    created: u64,
    destroyed: u64,
    with_client: u64,
    undecoded: u64,
    /// The distinct paths of the driver's `alloc_device` events, in the order they came: where it tells its clients to allocate from.
    alloc_devices: Vec<String>,
}

/// The descriptors retained, shared between the tap (which runs inside libwayland's dispatch) and the output.
#[derive(Clone, Default)]
pub struct Registry {
    inner: Arc<Mutex<Inner>>,
}

/// What the registry has seen, for the control socket.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegistryStats {
    pub retained: usize,
    pub created: u64,
    pub destroyed: u64,
    pub with_client: u64,
    pub undecoded: u64,
}

impl Registry {
    /// Keeps the descriptor of a buffer a client just created. A second create under the same key (an id reused after a destroy the tap
    /// missed) replaces the first.
    pub fn insert(&self, key: Key, mut d: Descriptor) {
        let mut g = self.inner.lock();
        g.created += 1;
        d.serial = g.created;
        g.map.insert(key, Arc::new(d));
    }

    /// The client destroyed the buffer.
    pub fn remove(&self, key: Key) -> bool {
        let mut g = self.inner.lock();
        let gone = g.map.remove(&key).is_some();
        g.destroyed += u64::from(gone);
        gone
    }

    /// The client is gone: every buffer it had goes with it.
    pub fn client_gone(&self, client: usize) -> usize {
        let mut g = self.inner.lock();
        let before = g.map.len();
        g.map.retain(|k, _| k.0 != client);
        let n = before - g.map.len();
        g.with_client += n as u64;
        n
    }

    /// A `create_buffer` the tap could not read (an unexpected signature, a descriptor that is not one).
    pub fn note_undecoded(&self) {
        self.inner.lock().undecoded += 1;
    }

    /// The driver told a client to allocate from `path` (`mali_buffer_sharing.alloc_device`). Each distinct path is kept once, eight at most.
    pub fn note_alloc_device(&self, path: String) {
        let mut g = self.inner.lock();
        if g.alloc_devices.len() < 8 && !g.alloc_devices.contains(&path) {
            g.alloc_devices.push(path);
        }
    }

    pub fn alloc_devices(&self) -> Vec<String> {
        self.inner.lock().alloc_devices.clone()
    }

    pub fn lookup(&self, key: Key) -> Option<Arc<Descriptor>> {
        self.inner.lock().map.get(&key).cloned()
    }

    /// The descriptor of a driver-served buffer, by the resource's own client and id.
    pub fn lookup_buffer(&self, buffer: &WlBuffer) -> Option<Arc<Descriptor>> {
        self.lookup(key_of(buffer)?)
    }

    /// Every key held, for a diagnostic to see what is new.
    pub fn keys(&self) -> Vec<Key> {
        self.inner.lock().map.keys().copied().collect()
    }

    pub fn stats(&self) -> RegistryStats {
        let g = self.inner.lock();
        RegistryStats { retained: g.map.len(), created: g.created, destroyed: g.destroyed, with_client: g.with_client, undecoded: g.undecoded }
    }
}

/// The key of a `wl_buffer` resource: the C backend's own client pointer and object id, the same pair the tap reads out of the request.
#[cfg(target_arch = "aarch64")]
pub fn key_of(buffer: &WlBuffer) -> Option<Key> {
    use wayland_server::Resource;
    // SAFETY: a `WlBuffer`'s object id on the C backend is its live `wl_resource` (the renderer reads it the same way).
    unsafe { super::tap::resource_key(buffer.id().as_ptr() as *mut std::ffi::c_void) }
}

/// Host builds run the pure-Rust backend, which has no `wl_client` pointers: the key is the client id's hash and the object id, which is
/// what the tests that stand in for the tap use.
#[cfg(not(target_arch = "aarch64"))]
pub fn key_of(buffer: &WlBuffer) -> Option<Key> {
    use std::hash::{Hash, Hasher};
    use wayland_server::Resource;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    buffer.client()?.id().hash(&mut h);
    Some((h.finish() as usize, buffer.id().protocol_id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::FromRawFd;

    fn memfd(len: u64) -> OwnedFd {
        // SAFETY: a memfd for the test, sized below.
        let fd = unsafe { libc::memfd_create(c"scanout-meta".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(fd >= 0);
        // SAFETY: the descriptor is ours.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: sizing the memfd.
        assert_eq!(unsafe { libc::ftruncate(fd.as_raw_fd(), len as i64) }, 0);
        fd
    }

    fn descriptor(len: u64) -> Descriptor {
        Descriptor { width: 1920, height: 1080, stride: 7680, format: crate::buffer::fourcc::XBGR8888, modifier: 0, fd: memfd(len), serial: 0 }
    }

    #[test]
    fn a_descriptor_is_kept_until_its_buffer_or_its_client_goes() {
        let r = Registry::default();
        r.insert((7, 3), descriptor(4096));
        r.insert((7, 4), descriptor(4096));
        r.insert((8, 3), descriptor(4096));
        assert_eq!(r.stats().retained, 3);
        assert_eq!(r.lookup((7, 3)).map(|d| d.stride), Some(7680));
        assert!(r.remove((7, 3)));
        assert!(!r.remove((7, 3)), "a destroy of what is not held is no event");
        assert!(r.lookup((7, 3)).is_none());
        assert_eq!(r.client_gone(7), 1, "the client's other buffer goes with it");
        assert_eq!(r.keys(), vec![(8, 3)], "another client's buffer with the same id stays");
        let s = r.stats();
        assert_eq!((s.created, s.destroyed, s.with_client, s.retained), (3, 1, 1, 1));
    }

    #[test]
    fn a_reused_id_replaces_the_descriptor_it_outlived() {
        let r = Registry::default();
        r.insert((1, 9), Descriptor { width: 256, height: 256, ..descriptor(4096) });
        r.insert((1, 9), Descriptor { width: 1920, ..descriptor(4096) });
        assert_eq!(r.lookup((1, 9)).map(|d| d.width), Some(1920));
        assert_eq!(r.stats().retained, 1);
    }

    #[test]
    fn the_size_is_the_dma_buf_files() {
        assert_eq!(descriptor(8_486_912).bytes(), Some(8_486_912));
        assert_eq!(fd_bytes(-1), None);
    }
}
