//! The TV's remote keys: reading them from the evdev devices and injecting the transport keys an assistant may press.
//!
//! The helper is a 64-bit process, so its `struct input_event` is 24 bytes (`tv_sec: i64, tv_usec: i64, type: u16,
//! code: u16, value: i32`, little-endian), not the 16 bytes of the TV's 32-bit userland. Names come from
//! `/sys/class/input/eventN/device/name` (with a trailing newline), nodes are `/dev/input/eventN`. Network keys (SSAP and
//! this module's own) arrive on "Smart Remote RCU Input"; the reader reports the device name with each key so the caller
//! can tell a physical remote from a network one.

// SAFETY (file-wide): the only `unsafe` is `libc::poll` over descriptors owned by `std::fs::File`s that outlive the call,
// and `libc::mkfifo` in the tests.
#![allow(unsafe_code)]

use crate::ai::types::{ActionError, KeyEvent, KeyInjector, KeySession, KeySource};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Size of a 64-bit `struct input_event`.
pub const INPUT_EVENT_SIZE: usize = 24;

/// The evdev device that carries network keys.
pub const NETWORK_KEY_DEVICE: &str = "Smart Remote RCU Input";

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const SYN_REPORT: u16 = 0;
/// How long a key is held between the press and the release.
const HOLD: Duration = Duration::from_millis(50);
/// Longest single sleep while a session has no device to poll.
const IDLE_CHUNK: Duration = Duration::from_millis(50);

/// Splits one `input_event` into (type, code, value); the timestamp is ignored.
pub fn parse_input_event(bytes: &[u8; INPUT_EVENT_SIZE]) -> (u16, u16, i32) {
    let kind = u16::from_le_bytes([bytes[16], bytes[17]]);
    let code = u16::from_le_bytes([bytes[18], bytes[19]]);
    let value = i32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    (kind, code, value)
}

/// One `input_event` with a zero timestamp (the kernel stamps injected events itself).
fn build_event(kind: u16, code: u16, value: i32) -> [u8; INPUT_EVENT_SIZE] {
    let mut out = [0u8; INPUT_EVENT_SIZE];
    out[16..18].copy_from_slice(&kind.to_le_bytes());
    out[18..20].copy_from_slice(&code.to_le_bytes());
    out[20..24].copy_from_slice(&value.to_le_bytes());
    out
}

/// The `eventN` entries under `sys_dir`, ordered by N.
fn event_nodes(sys_dir: &Path) -> Vec<(u32, String)> {
    let Ok(entries) = std::fs::read_dir(sys_dir) else {
        return Vec::new();
    };
    let mut nodes: Vec<(u32, String)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let n = name.strip_prefix("event")?.parse::<u32>().ok()?;
            Some((n, name))
        })
        .collect();
    nodes.sort();
    nodes
}

/// The kernel's name for `node` (`eventN`), without the trailing newline; `None` when it cannot be read.
pub fn device_name(sys_dir: &Path, node: &str) -> Option<String> {
    let raw = std::fs::read_to_string(sys_dir.join(node).join("device").join("name")).ok()?;
    Some(raw.trim_end_matches(['\n', '\r']).to_string())
}

/// The node under `dev_dir` of the device whose name is exactly `name`.
pub fn find_node(sys_dir: &Path, dev_dir: &Path, name: &str) -> Option<PathBuf> {
    event_nodes(sys_dir)
        .into_iter()
        .find(|(_, node)| device_name(sys_dir, node).as_deref() == Some(name))
        .map(|(_, node)| dev_dir.join(node))
}

// ------------------------------------------------------------------------------------------------ reading

/// Reads every `/dev/input/event*` device the kernel names.
pub struct EvdevKeys {
    sys_dir: PathBuf,
    dev_dir: PathBuf,
}

impl EvdevKeys {
    pub fn new() -> Self {
        Self::with_dirs("/sys/class/input", "/dev/input")
    }

    pub fn with_dirs(sys_dir: impl Into<PathBuf>, dev_dir: impl Into<PathBuf>) -> Self {
        Self {
            sys_dir: sys_dir.into(),
            dev_dir: dev_dir.into(),
        }
    }
}

impl Default for EvdevKeys {
    fn default() -> Self {
        Self::new()
    }
}

impl KeySource for EvdevKeys {
    fn open(&self) -> Box<dyn KeySession> {
        let nodes = event_nodes(&self.sys_dir);
        let mut devices = Vec::new();
        for (_, node) in &nodes {
            let Some(name) = device_name(&self.sys_dir, node) else {
                continue;
            };
            let opened = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(self.dev_dir.join(node));
            if let Ok(file) = opened {
                devices.push(Device {
                    file,
                    name,
                    buf: Vec::new(),
                });
            }
        }
        crate::log_debug!(
            "evdev: opened {} of {} input devices",
            devices.len(),
            nodes.len()
        );
        Box::new(EvdevSession {
            devices,
            queue: VecDeque::new(),
        })
    }
}

struct Device {
    file: File,
    name: String,
    /// Bytes read that do not yet make a whole event.
    buf: Vec<u8>,
}

struct EvdevSession {
    devices: Vec<Device>,
    queue: VecDeque<KeyEvent>,
}

impl EvdevSession {
    /// Reads what `devices[i]` has ready into the queue; `false` when the device is gone (EOF, ENODEV, EIO, other error).
    fn drain(&mut self, i: usize) -> bool {
        let dev = &mut self.devices[i];
        let mut chunk = [0u8; 4096];
        loop {
            match (&dev.file).read(&mut chunk) {
                Ok(0) => return false,
                Ok(n) => {
                    dev.buf.extend_from_slice(&chunk[..n]);
                    let whole = dev.buf.len() / INPUT_EVENT_SIZE * INPUT_EVENT_SIZE;
                    for raw in dev.buf[..whole].chunks_exact(INPUT_EVENT_SIZE) {
                        let raw: &[u8; INPUT_EVENT_SIZE] = raw.try_into().expect("chunk size");
                        let (kind, code, value) = parse_input_event(raw);
                        if kind == EV_KEY {
                            self.queue.push_back(KeyEvent {
                                device: dev.name.clone(),
                                code,
                                value,
                                at: Instant::now(),
                            });
                        }
                    }
                    dev.buf.drain(..whole);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return true,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
        }
    }
}

impl KeySession for EvdevSession {
    fn next(&mut self, timeout: Duration) -> Option<KeyEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(ev) = self.queue.pop_front() {
                return Some(ev);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if self.devices.is_empty() {
                if remaining.is_zero() {
                    return None;
                }
                std::thread::sleep(remaining.min(IDLE_CHUNK));
                continue;
            }
            // Rounded up so a sub-millisecond remainder does not spin.
            let ms = remaining
                .as_millis()
                .saturating_add(u128::from(remaining.subsec_nanos() % 1_000_000 != 0))
                .min(i32::MAX as u128) as i32;
            let mut fds: Vec<libc::pollfd> = self
                .devices
                .iter()
                .map(|d| libc::pollfd {
                    fd: d.file.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                })
                .collect();
            // SAFETY: `fds` is a live, correctly sized array of `pollfd`; every descriptor belongs to a `File` in
            // `self.devices`, which is not modified during the call.
            let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms) };
            if r < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return None;
            }
            if r == 0 {
                return None;
            }
            let mut gone = Vec::new();
            for (i, p) in fds.iter().enumerate() {
                if p.revents != 0 && !self.drain(i) {
                    gone.push(i);
                }
            }
            for i in gone.into_iter().rev() {
                self.devices.remove(i);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ injecting

/// Presses keys on the network key device: key down, a short hold, key up.
pub struct EvdevInjector {
    sys_dir: PathBuf,
    dev_dir: PathBuf,
    device_name: String,
}

impl EvdevInjector {
    pub fn new(device_name: impl Into<String>) -> Self {
        Self::with_dirs("/sys/class/input", "/dev/input", device_name)
    }

    pub fn with_dirs(
        sys_dir: impl Into<PathBuf>,
        dev_dir: impl Into<PathBuf>,
        device_name: impl Into<String>,
    ) -> Self {
        Self {
            sys_dir: sys_dir.into(),
            dev_dir: dev_dir.into(),
            device_name: device_name.into(),
        }
    }
}

fn write_failed(e: &std::io::Error) -> ActionError {
    ActionError::new(
        "failed",
        format!(
            "writing to the TV's network key device failed ({:?})",
            e.kind()
        ),
    )
}

impl KeyInjector for EvdevInjector {
    fn tap(&self, code: u16) -> Result<(), ActionError> {
        // Looked up on every call: the node can change when the compositor respawns the device.
        let node = find_node(&self.sys_dir, &self.dev_dir, &self.device_name).ok_or_else(|| {
            ActionError::new("unavailable", "the TV's network key device was not found")
        })?;
        let mut file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(&node)
            .map_err(|e| {
                ActionError::new(
                    "failed",
                    format!(
                        "opening the TV's network key device failed ({:?})",
                        e.kind()
                    ),
                )
            })?;

        let mut press = [0u8; 2 * INPUT_EVENT_SIZE];
        press[..24].copy_from_slice(&build_event(EV_KEY, code, 1));
        press[24..].copy_from_slice(&build_event(EV_SYN, SYN_REPORT, 0));
        let mut release = [0u8; 2 * INPUT_EVENT_SIZE];
        release[..24].copy_from_slice(&build_event(EV_KEY, code, 0));
        release[24..].copy_from_slice(&build_event(EV_SYN, SYN_REPORT, 0));

        // A press that reached the device is followed by a release whatever happens next.
        if let Err(e) = file.write(&press) {
            return Err(write_failed(&e));
        }
        std::thread::sleep(HOLD);
        // The press went out, so the release is mandatory: one retry, so a key is not left down.
        let mut result = file.write(&release);
        if !matches!(result, Ok(48)) {
            result = file.write(&release);
        }
        match result {
            Ok(48) => Ok(()),
            Ok(_) => Err(ActionError::new(
                "failed",
                "the TV's network key device took only part of the key release",
            )),
            Err(e) => Err(write_failed(&e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    fn ev(kind: u16, code: u16, value: i32) -> [u8; 24] {
        let mut b = [0u8; 24];
        b[0..8].copy_from_slice(&1_700_000_000i64.to_le_bytes());
        b[8..16].copy_from_slice(&123i64.to_le_bytes());
        b[16..18].copy_from_slice(&kind.to_le_bytes());
        b[18..20].copy_from_slice(&code.to_le_bytes());
        b[20..24].copy_from_slice(&value.to_le_bytes());
        b
    }

    fn add_dev(sys: &Path, n: u32, name: &str) {
        let d = sys.join(format!("event{n}")).join("device");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("name"), format!("{name}\n")).unwrap();
    }

    fn mkfifo(path: &Path) {
        let c = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: `c` is a valid NUL-terminated path that outlives the call.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    }

    fn writer(path: &Path) -> File {
        OpenOptions::new().write(true).open(path).unwrap()
    }

    struct Tree {
        _tmp: tempfile::TempDir,
        sys: PathBuf,
        dev: PathBuf,
    }

    fn tree() -> Tree {
        let tmp = tempfile::tempdir().unwrap();
        let sys = tmp.path().join("sys");
        let dev = tmp.path().join("dev");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::create_dir_all(&dev).unwrap();
        Tree {
            _tmp: tmp,
            sys,
            dev,
        }
    }

    fn fifo_dev(t: &Tree, n: u32, name: &str) -> PathBuf {
        add_dev(&t.sys, n, name);
        let p = t.dev.join(format!("event{n}"));
        mkfifo(&p);
        p
    }

    const SHORT: Duration = Duration::from_millis(100);

    #[test]
    fn names_drop_the_trailing_newline() {
        let t = tree();
        add_dev(&t.sys, 3, "LGE RCU");
        assert_eq!(device_name(&t.sys, "event3").as_deref(), Some("LGE RCU"));
        assert_eq!(device_name(&t.sys, "event4"), None);
    }

    #[test]
    fn parse_reads_type_code_and_value() {
        assert_eq!(parse_input_event(&ev(1, 28, 1)), (1, 28, 1));
        assert_eq!(parse_input_event(&ev(1, 773, 2)), (1, 773, 2));
        assert_eq!(parse_input_event(&ev(2, 0, -5)), (2, 0, -5));
        assert_eq!(
            parse_input_event(&ev(3, 0xFFFF, i32::MIN)),
            (3, 0xFFFF, i32::MIN)
        );
        assert_eq!(INPUT_EVENT_SIZE, 24);
    }

    #[test]
    fn events_from_two_devices_keep_their_names_and_order() {
        let t = tree();
        let a = fifo_dev(&t, 0, "LGE RCU");
        let b = fifo_dev(&t, 2, "LGE M-RCU - Builtin [0]");
        let mut s = EvdevKeys::with_dirs(&t.sys, &t.dev).open();
        let mut wa = writer(&a);
        let mut wb = writer(&b);
        wa.write_all(&ev(1, 28, 1)).unwrap();
        wb.write_all(&ev(1, 412, 1)).unwrap();
        wa.write_all(&ev(1, 28, 0)).unwrap();

        let mut got = Vec::new();
        while let Some(e) = s.next(SHORT) {
            got.push((e.device, e.code, e.value));
        }
        assert_eq!(
            got,
            vec![
                ("LGE RCU".to_string(), 28, 1),
                ("LGE RCU".to_string(), 28, 0),
                ("LGE M-RCU - Builtin [0]".to_string(), 412, 1),
            ]
        );
    }

    #[test]
    fn one_device_keeps_arrival_order() {
        let t = tree();
        let a = fifo_dev(&t, 0, "LGE RCU");
        let mut s = EvdevKeys::with_dirs(&t.sys, &t.dev).open();
        let mut w = writer(&a);
        for (code, value) in [(103, 1), (103, 0), (108, 1), (108, 0)] {
            w.write_all(&ev(1, code, value)).unwrap();
        }
        let mut got = Vec::new();
        while let Some(e) = s.next(SHORT) {
            got.push((e.code, e.value));
        }
        assert_eq!(got, vec![(103, 1), (103, 0), (108, 1), (108, 0)]);
    }

    #[test]
    fn non_key_events_are_dropped() {
        let t = tree();
        let a = fifo_dev(&t, 0, "LGE RCU");
        let mut s = EvdevKeys::with_dirs(&t.sys, &t.dev).open();
        let mut w = writer(&a);
        w.write_all(&ev(0, 0, 0)).unwrap();
        w.write_all(&ev(2, 0, 4)).unwrap();
        w.write_all(&ev(3, 1, 100)).unwrap();
        w.write_all(&ev(1, 28, 1)).unwrap();
        w.write_all(&ev(0, 0, 0)).unwrap();
        let e = s.next(SHORT).unwrap();
        assert_eq!((e.code, e.value), (28, 1));
        assert!(s.next(SHORT).is_none());
    }

    #[test]
    fn a_split_event_is_reassembled() {
        let t = tree();
        let a = fifo_dev(&t, 0, "LGE RCU");
        let mut s = EvdevKeys::with_dirs(&t.sys, &t.dev).open();
        let mut w = writer(&a);
        let raw = ev(1, 773, 1);
        w.write_all(&raw[..10]).unwrap();
        assert!(s.next(Duration::from_millis(50)).is_none());
        w.write_all(&raw[10..]).unwrap();
        let e = s.next(SHORT).unwrap();
        assert_eq!((e.device.as_str(), e.code, e.value), ("LGE RCU", 773, 1));
    }

    #[test]
    fn a_closed_device_is_dropped_and_the_session_goes_on() {
        let t = tree();
        let a = fifo_dev(&t, 0, "LGE RCU");
        let b = fifo_dev(&t, 1, "LGE M-RCU - Builtin [1]");
        let mut s = EvdevKeys::with_dirs(&t.sys, &t.dev).open();
        let mut wb = writer(&b);
        {
            let mut wa = writer(&a);
            wa.write_all(&ev(1, 28, 1)).unwrap();
        }
        let e = s.next(SHORT).unwrap();
        assert_eq!((e.device.as_str(), e.code), ("LGE RCU", 28));
        assert!(s.next(Duration::from_millis(50)).is_none());
        wb.write_all(&ev(1, 412, 1)).unwrap();
        let e = s.next(SHORT).unwrap();
        assert_eq!(
            (e.device.as_str(), e.code),
            ("LGE M-RCU - Builtin [1]", 412)
        );
    }

    #[test]
    fn missing_nodes_and_unnamed_entries_are_skipped() {
        let t = tree();
        add_dev(&t.sys, 0, "No Node");
        std::fs::create_dir_all(t.sys.join("event5").join("device")).unwrap();
        std::fs::create_dir_all(t.sys.join("input7")).unwrap();
        let a = fifo_dev(&t, 1, "LGE RCU");
        let mut s = EvdevKeys::with_dirs(&t.sys, &t.dev).open();
        let mut w = writer(&a);
        w.write_all(&ev(1, 28, 1)).unwrap();
        assert_eq!(s.next(SHORT).unwrap().code, 28);
    }

    #[test]
    fn an_empty_tree_waits_out_the_timeout() {
        let t = tree();
        let mut s = EvdevKeys::with_dirs(&t.sys, &t.dev).open();
        let start = Instant::now();
        assert!(s.next(Duration::from_millis(120)).is_none());
        let took = start.elapsed();
        assert!(took >= Duration::from_millis(120), "{took:?}");
        assert!(took < Duration::from_millis(400), "{took:?}");
        let missing = EvdevKeys::with_dirs(t.sys.join("nope"), t.dev.join("nope"));
        assert!(missing.open().next(Duration::from_millis(10)).is_none());
    }

    #[test]
    fn devices_without_data_time_out() {
        let t = tree();
        let _a = fifo_dev(&t, 0, "LGE RCU");
        let mut s = EvdevKeys::with_dirs(&t.sys, &t.dev).open();
        let _w = writer(&t.dev.join("event0"));
        let start = Instant::now();
        assert!(s.next(Duration::from_millis(80)).is_none());
        let took = start.elapsed();
        assert!(took >= Duration::from_millis(80), "{took:?}");
        assert!(took < Duration::from_millis(400), "{took:?}");
    }

    #[test]
    fn find_node_matches_the_whole_name() {
        let t = tree();
        add_dev(&t.sys, 8, "Smart Remote RCU Input 2");
        add_dev(&t.sys, 9, "Smart Remote RCU Input");
        add_dev(&t.sys, 10, "LGE Network Input");
        assert_eq!(
            find_node(&t.sys, &t.dev, NETWORK_KEY_DEVICE),
            Some(t.dev.join("event9"))
        );
        assert_eq!(find_node(&t.sys, &t.dev, "Smart Remote RCU"), None);
    }

    #[test]
    fn tap_writes_down_and_up_in_the_64_bit_layout() {
        let t = tree();
        add_dev(&t.sys, 9, NETWORK_KEY_DEVICE);
        let node = t.dev.join("event9");
        std::fs::write(&node, b"").unwrap();
        let inj = EvdevInjector::with_dirs(&t.sys, &t.dev, NETWORK_KEY_DEVICE);
        inj.tap(773).unwrap();
        let bytes = std::fs::read(&node).unwrap();
        assert_eq!(bytes.len(), 96);
        let events: Vec<_> = bytes
            .chunks_exact(24)
            .map(|c| parse_input_event(c.try_into().unwrap()))
            .collect();
        assert_eq!(events, vec![(1, 773, 1), (0, 0, 0), (1, 773, 0), (0, 0, 0)]);
        assert!(bytes[..16].iter().all(|b| *b == 0));
    }

    #[test]
    fn tap_without_the_device_is_unavailable() {
        let t = tree();
        add_dev(&t.sys, 8, "Smart Remote RCU Input 2");
        let inj = EvdevInjector::with_dirs(&t.sys, &t.dev, NETWORK_KEY_DEVICE);
        let err = inj.tap(28).unwrap_err();
        assert_eq!(err.code, "unavailable");
        assert_eq!(err.message, "the TV's network key device was not found");
    }

    #[test]
    fn tap_on_an_unopenable_node_fails() {
        let t = tree();
        add_dev(&t.sys, 9, NETWORK_KEY_DEVICE);
        let inj = EvdevInjector::with_dirs(&t.sys, &t.dev, NETWORK_KEY_DEVICE);
        let err = inj.tap(28).unwrap_err();
        assert_eq!(err.code, "failed");
        assert!(!err.message.contains('\n'));
    }
}
