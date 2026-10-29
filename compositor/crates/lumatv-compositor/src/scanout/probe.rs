//! The off-screen proof (`--scanout-probe`, `tools/tv-scanout-proof.sh`): for every buffer a driver-served client creates, the descriptor the
//! tap copied (size, stride, format, modifier, the dma-buf's size and exporter) and whether the display driver would import it. The import
//! is a PRIME import of the fd into lg1k and a close of the handle, on a descriptor of `/dev/dri/card0` that never becomes master: no
//! framebuffer, no mode set, no atomic commit, no read of the pixels, nothing the screen could show. A refusal is a result: a buffer of
//! scattered pages is `EINVAL` there (lg1k imports through the CMA helper, which needs one run of addresses).

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};

use drm::Device;
use drm::control::Device as ControlDevice;
use serde_json::{Value, json};

use super::meta::{Key, Registry};
use super::modifier_name;
use crate::buffer::fourcc;

/// The DRM device file, opened without becoming master.
struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl Device for Card {}
impl ControlDevice for Card {}

/// One buffer's line of the proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub seq: u32,
    pub width: i32,
    pub height: i32,
    pub stride: u32,
    pub format: u32,
    pub modifier: u64,
    pub bytes: Option<u64>,
    pub exporter: Option<String>,
    /// The GEM handle the import gave (closed again), or the errno it was refused with; `None` when there was no device to ask.
    pub import: Option<Result<u32, i32>>,
}

impl Entry {
    pub fn text(&self) -> String {
        let import = match &self.import {
            Some(Ok(h)) => format!("import ok (gem handle {h}, closed)"),
            Some(Err(e)) => format!("import refused: {} ({e})", std::io::Error::from_raw_os_error(*e)),
            None => "no display device to ask".into(),
        };
        format!(
            "scanout-probe: buffer {} {}x{} {} {} stride {} {} bytes exporter {}: {import}",
            self.seq,
            self.width,
            self.height,
            fourcc::name(self.format),
            modifier_name(self.modifier),
            self.stride,
            self.bytes.map_or("?".to_string(), |b| b.to_string()),
            self.exporter.as_deref().unwrap_or("?"),
        )
    }
}

pub struct Probe {
    card: Option<Card>,
    error: Option<String>,
    /// The serial of the descriptor seen under each key: a key reused for another buffer is looked at again.
    seen: HashMap<Key, u64>,
    entries: Vec<Entry>,
    /// The paths the driver told its clients to allocate from, as of the last look.
    alloc: Vec<String>,
}

impl Probe {
    /// Opens `path` read-write (PRIME import wants a writable node) without taking master. A device that cannot be opened is kept as the reason:
    /// the descriptors are still recorded.
    pub fn open(path: &str) -> Probe {
        match OpenOptions::new().read(true).write(true).open(path) {
            Ok(f) => Probe { card: Some(Card(f)), error: None, seen: HashMap::new(), entries: Vec::new(), alloc: Vec::new() },
            Err(e) => Probe { card: None, error: Some(format!("{path}: {e}")), seen: HashMap::new(), entries: Vec::new(), alloc: Vec::new() },
        }
    }

    /// Looks at the descriptors the tap holds that this has not: each is recorded and imported once. The lines are for the journal.
    pub fn poll(&mut self, registry: &Registry) -> Vec<String> {
        self.alloc = registry.alloc_devices();
        let keys = registry.keys();
        let present: HashSet<Key> = keys.iter().copied().collect();
        self.seen.retain(|k, _| present.contains(k));
        let mut lines = Vec::new();
        for key in keys {
            let Some(d) = registry.lookup(key) else { continue };
            if self.seen.get(&key) == Some(&d.serial) {
                continue;
            }
            self.seen.insert(key, d.serial);
            let exporter = fdinfo_exporter(&std::fs::read_to_string(format!("/proc/self/fdinfo/{}", d.fd.as_raw_fd())).unwrap_or_default());
            let import = self.card.as_ref().map(|card| match card.prime_fd_to_buffer(d.fd.as_fd()) {
                Ok(handle) => {
                    let raw: u32 = handle.into();
                    let _ = card.close_buffer(handle);
                    Ok(raw)
                }
                Err(e) => Err(e.raw_os_error().unwrap_or(libc::EIO)),
            });
            let entry = Entry {
                seq: self.entries.len() as u32 + 1,
                width: d.width,
                height: d.height,
                stride: d.stride,
                format: d.format,
                modifier: d.modifier,
                bytes: d.bytes(),
                exporter,
                import,
            };
            lines.push(entry.text());
            self.entries.push(entry);
        }
        lines
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Buffers seen, imported, refused.
    pub fn counts(&self) -> (usize, usize, usize) {
        let ok = self.entries.iter().filter(|e| matches!(e.import, Some(Ok(_)))).count();
        let refused = self.entries.iter().filter(|e| matches!(e.import, Some(Err(_)))).count();
        (self.entries.len(), ok, refused)
    }

    /// The last line of a run: what every buffer was and whether lg1k took it.
    pub fn summary(&self) -> String {
        let (n, ok, refused) = self.counts();
        let mut layouts: Vec<String> = self.entries.iter().map(|e| format!("{} {}", fourcc::name(e.format), modifier_name(e.modifier))).collect();
        layouts.sort();
        layouts.dedup();
        format!(
            "scanout-probe summary: {n} buffers, {ok} imported by the display driver, {refused} refused; layouts: {}; alloc_device: {}{}",
            if layouts.is_empty() { "none".to_string() } else { layouts.join("; ") },
            if self.alloc.is_empty() { "none seen".to_string() } else { self.alloc.join(", ") },
            self.error.as_ref().map(|e| format!("; device: {e}")).unwrap_or_default()
        )
    }

    pub fn report(&self) -> Value {
        let (n, ok, refused) = self.counts();
        json!({
            "device": self.error.as_deref().unwrap_or("open"),
            "allocDevices": self.alloc,
            "buffers": n,
            "imported": ok,
            "refused": refused,
            "entries": self.entries.iter().map(|e| json!({
                "seq": e.seq,
                "size": [e.width, e.height],
                "stride": e.stride,
                "format": fourcc::name(e.format),
                "modifier": format!("0x{:016x}", e.modifier),
                "bytes": e.bytes,
                "exporter": e.exporter,
                "import": match &e.import { Some(Ok(_)) => json!("ok"), Some(Err(errno)) => json!({"errno": errno}), None => json!(null) },
            })).collect::<Vec<_>>(),
        })
    }
}

/// The exporter's name from a dma-buf's `fdinfo` (`exp_name: dma_buf_te`), when the kernel prints it.
pub fn fdinfo_exporter(text: &str) -> Option<String> {
    text.lines().find_map(|l| l.strip_prefix("exp_name:")).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanout::meta::Descriptor;
    use std::os::fd::{FromRawFd, OwnedFd};

    fn descriptor(width: i32) -> Descriptor {
        // SAFETY: a memfd for the test, sized below.
        let fd = unsafe { libc::memfd_create(c"scanout-probe".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(fd >= 0);
        // SAFETY: the descriptor is ours.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: sizing the memfd.
        assert_eq!(unsafe { libc::ftruncate(fd.as_raw_fd(), 262_144) }, 0);
        Descriptor { width, height: 256, stride: width as u32 * 4, format: fourcc::ABGR8888, modifier: 0x0800_0000_0000_0151, fd, serial: 0 }
    }

    #[test]
    fn the_exporter_is_read_from_fdinfo() {
        assert_eq!(fdinfo_exporter("pos:\t0\nflags:\t02000002\nsize:\t8986624\ncount:\t3\nexp_name:\tdma_buf_te\n").as_deref(), Some("dma_buf_te"));
        assert_eq!(fdinfo_exporter("pos:\t0\nsize:\t4096\n"), None, "a kernel that does not print it");
        assert_eq!(fdinfo_exporter("exp_name:\n"), None);
    }

    #[test]
    fn each_descriptor_is_recorded_once_and_asked_of_the_display_once() {
        let registry = Registry::default();
        // No device to ask: a path that is not there. The descriptors are recorded all the same.
        let mut probe = Probe::open("/nonexistent/card0");
        registry.insert((1, 5), descriptor(256));
        let first = probe.poll(&registry);
        assert_eq!(first.len(), 1);
        assert!(first[0].contains("256x256 AB24") && first[0].contains("0x0800000000000151") && first[0].contains("stride 1024"), "{first:?}");
        assert!(first[0].contains("262144 bytes") && first[0].contains("no display device"), "{first:?}");
        assert!(probe.poll(&registry).is_empty(), "a buffer already seen is not recorded again");
        registry.insert((1, 6), descriptor(512));
        assert_eq!(probe.poll(&registry).len(), 1, "a new one is");
        registry.remove((1, 5));
        registry.remove((1, 6));
        registry.insert((1, 5), descriptor(256));
        assert_eq!(probe.poll(&registry).len(), 1, "an id reused after its buffer went is a new buffer");
        assert_eq!(probe.counts(), (3, 0, 0));
        let s = probe.summary();
        assert!(s.contains("3 buffers") && s.contains("AB24 AFBC 16x16") && s.contains("device: /nonexistent/card0"), "{s}");
        assert_eq!(probe.report()["entries"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn a_refusal_and_an_import_read_differently() {
        let mut e = Entry {
            seq: 1,
            width: 256,
            height: 256,
            stride: 1024,
            format: fourcc::ABGR8888,
            modifier: 0,
            bytes: Some(262_144),
            exporter: None,
            import: Some(Ok(3)),
        };
        assert!(e.text().contains("import ok (gem handle 3, closed)") && e.text().contains("exporter ?"));
        e.import = Some(Err(libc::EINVAL));
        assert!(e.text().contains("import refused") && e.text().contains("(22)"), "{}", e.text());
    }
}
