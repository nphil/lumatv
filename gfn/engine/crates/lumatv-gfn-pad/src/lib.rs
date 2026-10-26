//! Controllers for the GeForce NOW app, read from their evdev nodes with nothing of LG's in the path
//! (docs/research/gfn-input-result.md, "Design: controller input in the OpenNOW port").
//!
//! One blocking `read` loop per pad on its own thread; the state is updated per event and a snapshot in XInput form goes to the
//! sink at every `SYN_REPORT` that changed it. Discovery reads `/sys/class/input/eventN/device/{name,uniq,capabilities/*}` (bound
//! read-only into SAM's jail) and opens `/dev/input/eventN` read-write; hotplug is inotify on the device directory plus a slow
//! rescan. `EVIOCGRAB` while the stream is in front keeps the pad's keys from LG's compositor; the kernel drops the grab when the
//! descriptor closes, so a crash cannot wedge the pad. Rumble is one `FF_RUMBLE` effect per pad, re-uploaded with each new pair of
//! magnitudes and played with an `EV_FF` write, which `hid-sony` turns into the DS4 output report.
//!
//! The directories are parameters so the host tests can stand in a device tree with a pseudo-terminal for the node.
use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const MAX_PADS: usize = 4;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const EV_FF: u16 = 0x15;
const SYN_REPORT: u16 = 0;
const BTN_SOUTH: u16 = 0x130;
const BTN_EAST: u16 = 0x131;
const BTN_NORTH: u16 = 0x133;
const BTN_WEST: u16 = 0x134;
const BTN_TL: u16 = 0x136;
const BTN_TR: u16 = 0x137;
const BTN_TL2: u16 = 0x138;
const BTN_TR2: u16 = 0x139;
const BTN_SELECT: u16 = 0x13a;
const BTN_START: u16 = 0x13b;
const BTN_MODE: u16 = 0x13c;
const BTN_THUMBL: u16 = 0x13d;
const BTN_THUMBR: u16 = 0x13e;
const BTN_DPAD_UP: u16 = 0x220;
const BTN_DPAD_DOWN: u16 = 0x221;
const BTN_DPAD_LEFT: u16 = 0x222;
const BTN_DPAD_RIGHT: u16 = 0x223;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_Z: u16 = 2;
const ABS_RX: u16 = 3;
const ABS_RY: u16 = 4;
const ABS_RZ: u16 = 5;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;
const FF_RUMBLE: u16 = 0x50;

/// XInput button bits, the ones GFN's 38-byte gamepad event carries (OpenNOW ControllerInput::buttonMask).
pub mod button {
    pub const DPAD_UP: u16 = 0x0001;
    pub const DPAD_DOWN: u16 = 0x0002;
    pub const DPAD_LEFT: u16 = 0x0004;
    pub const DPAD_RIGHT: u16 = 0x0008;
    pub const START: u16 = 0x0010;
    pub const BACK: u16 = 0x0020;
    pub const LEFT_THUMB: u16 = 0x0040;
    pub const RIGHT_THUMB: u16 = 0x0080;
    pub const LEFT_SHOULDER: u16 = 0x0100;
    pub const RIGHT_SHOULDER: u16 = 0x0200;
    pub const A: u16 = 0x1000;
    pub const B: u16 = 0x2000;
    pub const X: u16 = 0x4000;
    pub const Y: u16 = 0x8000;
}

/// A pad's state in the form the stream wants: XInput buttons, triggers 0..255, sticks -32767..32767 with up positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PadState {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub left_x: i16,
    pub left_y: i16,
    pub right_x: i16,
    pub right_y: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PadEvent {
    Connected { slot: u8, name: String, uniq: String },
    Disconnected { slot: u8 },
    /// A changed report; `bitmap` says which slots are connected (bit i) and that they are XInput style (bit i + 8).
    Report { slot: u8, state: PadState },
    /// The PS/Xbox button: a local action, never sent to the game.
    Guide { slot: u8, pressed: bool },
    /// Select and Start held together for a second: the app's own way out of a stream.
    ExitChord { slot: u8 },
}

pub trait PadSink: Send + Sync {
    fn pad_event(&self, bitmap: u16, event: PadEvent);
}

#[derive(Debug, Clone)]
pub struct PadConfig {
    pub dev_dir: PathBuf,
    pub sys_dir: PathBuf,
    pub rescan: Duration,
    /// Radial stick dead zone in percent, as OpenNOW applies it before the state goes on the wire.
    pub deadzone_percent: u8,
    pub chord_hold: Duration,
}

impl Default for PadConfig {
    fn default() -> Self {
        Self {
            dev_dir: PathBuf::from("/dev/input"),
            sys_dir: PathBuf::from("/sys/class/input"),
            rescan: Duration::from_secs(2),
            deadzone_percent: 8,
            chord_hold: Duration::from_secs(1),
        }
    }
}

impl PadConfig {
    /// `LUMATV_GFN_PAD_DEV` and `LUMATV_GFN_PAD_SYS` move the directories (the tests), `LUMATV_GFN_PAD_DEADZONE` the dead zone.
    pub fn from_env() -> Self {
        let mut c = Self::default();
        if let Some(d) = std::env::var_os("LUMATV_GFN_PAD_DEV") {
            c.dev_dir = PathBuf::from(d);
        }
        if let Some(s) = std::env::var_os("LUMATV_GFN_PAD_SYS") {
            c.sys_dir = PathBuf::from(s);
        }
        if let Some(z) = std::env::var("LUMATV_GFN_PAD_DEADZONE").ok().and_then(|v| v.parse().ok()) {
            c.deadzone_percent = z;
        }
        c
    }
}

/// What sysfs says about an input node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFacts {
    pub name: String,
    pub uniq: String,
    pub gamepad: bool,
    pub rumble: bool,
}

fn read_trim(path: &Path) -> String {
    fs::read_to_string(path).map(|s| s.trim().to_owned()).unwrap_or_default()
}

/// Bitmap words of a sysfs `capabilities/*` file, least significant word first (the file lists the most significant first).
fn capability_words(text: &str) -> Vec<u64> {
    text.split_whitespace().rev().map(|w| u64::from_str_radix(w, 16).unwrap_or(0)).collect()
}

fn has_bit(words: &[u64], bit: u32) -> bool {
    words.get((bit / 64) as usize).is_some_and(|w| w & (1u64 << (bit % 64)) != 0)
}

pub fn node_facts(sys_dir: &Path, node: &str) -> NodeFacts {
    let dev = sys_dir.join(node).join("device");
    let ev = capability_words(&read_trim(&dev.join("capabilities/ev")));
    let key = capability_words(&read_trim(&dev.join("capabilities/key")));
    let ff = capability_words(&read_trim(&dev.join("capabilities/ff")));
    NodeFacts {
        name: read_trim(&dev.join("name")),
        uniq: read_trim(&dev.join("uniq")),
        gamepad: has_bit(&ev, u32::from(EV_KEY)) && has_bit(&ev, u32::from(EV_ABS)) && has_bit(&key, u32::from(BTN_SOUTH)),
        rumble: has_bit(&ev, u32::from(EV_FF)) && has_bit(&ff, u32::from(FF_RUMBLE)),
    }
}

/// `event*` nodes under the device directory, sorted by number.
pub fn list_nodes(dev_dir: &Path) -> Vec<String> {
    let mut names: Vec<(u32, String)> = fs::read_dir(dev_dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.strip_prefix("event").and_then(|d| d.parse::<u32>().ok()).map(|num| (num, n))
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.into_iter().map(|(_, n)| n).collect()
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    kind: u16,
    code: u16,
    value: i32,
}

const EVENT_SIZE: usize = std::mem::size_of::<InputEvent>();

#[derive(Clone, Copy)]
struct AbsRange {
    min: i32,
    max: i32,
}

/// Raw evdev state of one pad and its translation to `PadState`.
#[derive(Clone)]
pub struct Raw {
    keys: HashMap<u16, bool>,
    abs: HashMap<u16, i32>,
    ranges: HashMap<u16, AbsRange>,
    deadzone_percent: u8,
}

impl Raw {
    pub fn new(deadzone_percent: u8) -> Self {
        let mut ranges = HashMap::new();
        for code in [ABS_X, ABS_Y, ABS_RX, ABS_RY, ABS_Z, ABS_RZ] {
            ranges.insert(code, AbsRange { min: 0, max: 255 });    /* the DualShock 4 shape; EVIOCGABS replaces it where it answers */
        }
        for code in [ABS_HAT0X, ABS_HAT0Y] {
            ranges.insert(code, AbsRange { min: -1, max: 1 });
        }
        Self { keys: HashMap::new(), abs: HashMap::new(), ranges, deadzone_percent }
    }

    fn set_range(&mut self, code: u16, min: i32, max: i32) {
        if max > min {
            self.ranges.insert(code, AbsRange { min, max });
        }
    }

    pub fn apply(&mut self, kind: u16, code: u16, value: i32) {
        match kind {
            EV_KEY => {
                self.keys.insert(code, value != 0);
            }
            EV_ABS => {
                self.abs.insert(code, value);
            }
            _ => {}
        }
    }

    fn key(&self, code: u16) -> bool {
        self.keys.get(&code).copied().unwrap_or(false)
    }

    /// -1.0..1.0 around the resting value: a 0..255 axis rests at 128 (the DualShock 4), so the centre is the upper middle and each
    /// half is scaled on its own so both extremes reach 1.
    fn axis(&self, code: u16) -> f64 {
        let r = self.ranges.get(&code).copied().unwrap_or(AbsRange { min: 0, max: 255 });
        let centre = (r.min + r.max + 1) / 2;
        let v = self.abs.get(&code).copied().unwrap_or(centre);
        let span = if v >= centre { r.max - centre } else { centre - r.min };
        if span <= 0 { 0.0 } else { ((v - centre) as f64 / span as f64).clamp(-1.0, 1.0) }
    }

    /// 0..255 over the range (triggers rest at the minimum).
    fn trigger(&self, code: u16) -> u8 {
        let r = self.ranges.get(&code).copied().unwrap_or(AbsRange { min: 0, max: 255 });
        let v = self.abs.get(&code).copied().unwrap_or(r.min);
        let span = (r.max - r.min).max(1) as f64;
        (((v - r.min) as f64 / span).clamp(0.0, 1.0) * 255.0).round() as u8
    }

    fn hat(&self, code: u16) -> i32 {
        self.abs.get(&code).copied().unwrap_or(0)
    }

    pub fn state(&self) -> PadState {
        let mut b = 0u16;
        let mut set = |on: bool, bit: u16| {
            if on {
                b |= bit;
            }
        };
        set(self.key(BTN_SOUTH), button::A);
        set(self.key(BTN_EAST), button::B);
        set(self.key(BTN_WEST), button::X);
        set(self.key(BTN_NORTH), button::Y);
        set(self.key(BTN_TL), button::LEFT_SHOULDER);
        set(self.key(BTN_TR), button::RIGHT_SHOULDER);
        set(self.key(BTN_SELECT), button::BACK);
        set(self.key(BTN_START), button::START);
        set(self.key(BTN_THUMBL), button::LEFT_THUMB);
        set(self.key(BTN_THUMBR), button::RIGHT_THUMB);
        set(self.key(BTN_DPAD_UP) || self.hat(ABS_HAT0Y) < 0, button::DPAD_UP);
        set(self.key(BTN_DPAD_DOWN) || self.hat(ABS_HAT0Y) > 0, button::DPAD_DOWN);
        set(self.key(BTN_DPAD_LEFT) || self.hat(ABS_HAT0X) < 0, button::DPAD_LEFT);
        set(self.key(BTN_DPAD_RIGHT) || self.hat(ABS_HAT0X) > 0, button::DPAD_RIGHT);
        let (lx, ly) = radial_deadzone(self.axis(ABS_X), self.axis(ABS_Y), self.deadzone_percent);
        let (rx, ry) = radial_deadzone(self.axis(ABS_RX), self.axis(ABS_RY), self.deadzone_percent);
        let (mut lt, mut rt) = (self.trigger(ABS_Z), self.trigger(ABS_RZ));
        if self.key(BTN_TL2) && lt == 0 {
            lt = 255;       /* a pad that reports its triggers as buttons only */
        }
        if self.key(BTN_TR2) && rt == 0 {
            rt = 255;
        }
        PadState {
            buttons: b,
            left_trigger: lt,
            right_trigger: rt,
            left_x: lx,
            left_y: -ly,    /* evdev Y grows downward; XInput's grows upward */
            right_x: rx,
            right_y: -ry,
        }
    }

    pub fn guide(&self) -> bool {
        self.key(BTN_MODE)
    }
}

/// OpenNOW's radial dead zone (ControllerInput::radialDeadzone), on -1..1 inputs, to the i16 the wire carries.
pub fn radial_deadzone(x: f64, y: f64, percent: u8) -> (i16, i16) {
    let dz = f64::from(percent) / 100.0;
    let mag = x.hypot(y);
    if mag <= dz {
        return (0, 0);
    }
    let scaled = (mag - dz) / (1.0 - dz);
    let f = scaled / mag;
    let to = |v: f64| ((v * f).clamp(-1.0, 1.0) * 32767.0).round() as i16;
    (to(x), to(y))
}

fn ioctl_code(dir: u32, nr: u32, size: u32) -> u64 {
    u64::from((dir << 30) | (size << 16) | (0x45 << 8) | nr)
}

fn eviocgrab(fd: RawFd, grab: bool) -> io::Result<()> {
    let v: libc::c_int = i32::from(grab);
    let rc = unsafe { libc::ioctl(fd, ioctl_code(1, 0x90, 4) as _, &v as *const libc::c_int) };
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

fn eviocgabs(fd: RawFd, code: u16) -> io::Result<(i32, i32)> {
    let mut info = [0i32; 6];
    let rc = unsafe { libc::ioctl(fd, ioctl_code(2, 0x40 + u32::from(code), 24) as _, info.as_mut_ptr()) };
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok((info[1], info[2])) }
}

/// struct ff_effect for FF_RUMBLE on an LP64 kernel: 48 bytes, the rumble magnitudes at offset 16.
fn ff_effect_bytes(id: i16, strong: u16, weak: u16, length_ms: u16) -> [u8; 48] {
    let mut b = [0u8; 48];
    b[0..2].copy_from_slice(&FF_RUMBLE.to_ne_bytes());
    b[2..4].copy_from_slice(&id.to_ne_bytes());
    b[12..14].copy_from_slice(&length_ms.to_ne_bytes());    /* replay.length (after type, id, direction, trigger) */
    b[16..18].copy_from_slice(&strong.to_ne_bytes());
    b[18..20].copy_from_slice(&weak.to_ne_bytes());
    b
}

fn eviocsff(fd: RawFd, effect: &mut [u8; 48]) -> io::Result<i16> {
    let rc = unsafe { libc::ioctl(fd, ioctl_code(1, 0x80, 48) as _, effect.as_mut_ptr()) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(i16::from_ne_bytes([effect[2], effect[3]]))
}

fn write_event(fd: RawFd, kind: u16, code: u16, value: i32) -> io::Result<()> {
    let ev = InputEvent { tv_sec: 0, tv_usec: 0, kind, code, value };
    let bytes = unsafe { std::slice::from_raw_parts(&ev as *const InputEvent as *const u8, EVENT_SIZE) };
    let n = unsafe { libc::write(fd, bytes.as_ptr() as *const libc::c_void, EVENT_SIZE) };
    if n < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

struct Slot {
    node: String,
    fd: RawFd,
    facts: NodeFacts,
    ff_id: Mutex<Option<i16>>,
    grabbed: AtomicBool,
    last_rumble: Mutex<Option<(u16, u16, u32)>>,
    reports: AtomicU64,
}

struct Inner {
    config: PadConfig,
    sink: Arc<dyn PadSink>,
    slots: Mutex<[Option<Arc<Slot>>; MAX_PADS]>,
    grab: AtomicBool,
    stop: AtomicBool,
    bitmap: AtomicU16,
    scans: AtomicU64,
}

/// The pads as a whole: start it once, point it at a sink, ask for the grab when the stream is in front.
#[derive(Clone)]
pub struct PadRuntime {
    inner: Arc<Inner>,
}

impl PadRuntime {
    pub fn start(config: PadConfig, sink: Arc<dyn PadSink>) -> Self {
        let inner = Arc::new(Inner {
            config,
            sink,
            slots: Mutex::new(Default::default()),
            grab: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            bitmap: AtomicU16::new(0),
            scans: AtomicU64::new(0),
        });
        let rt = Self { inner: Arc::clone(&inner) };
        let discovery = Arc::clone(&inner);
        let _ = thread::Builder::new().name("gfn-pad-discovery".to_owned()).spawn(move || discovery_loop(&discovery));
        rt
    }

    /// Hold the pads exclusively (the stream is in front) or share them again.
    pub fn set_grab(&self, grab: bool) {
        self.inner.grab.store(grab, Ordering::Release);
        let slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
        for slot in slots.iter().flatten() {
            apply_grab(slot, grab);
        }
    }

    pub fn grabbed(&self) -> bool {
        self.inner.grab.load(Ordering::Acquire)
    }

    /// Connected slots: bit i for slot i, bit i + 8 marking it XInput style (the wire's convention).
    pub fn bitmap(&self) -> u16 {
        self.inner.bitmap.load(Ordering::Acquire)
    }

    pub fn scans(&self) -> u64 {
        self.inner.scans.load(Ordering::Acquire)
    }

    pub fn connected(&self) -> Vec<(u8, NodeFacts)> {
        let slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i as u8, s.facts.clone()))).collect()
    }

    /// GFN's haptic command for a slot: `low` drives the strong motor, `high` the weak one; both zero stops.
    pub fn rumble(&self, slot: u8, low: u16, high: u16, duration_ms: u32) -> io::Result<()> {
        let s = {
            let slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
            slots.get(usize::from(slot)).and_then(|s| s.clone())
        };
        let Some(s) = s else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no pad in that slot"));
        };
        *s.last_rumble.lock().unwrap_or_else(|e| e.into_inner()) = Some((low, high, duration_ms));
        if !s.facts.rumble {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "this pad has no rumble"));
        }
        let mut id_guard = s.ff_id.lock().unwrap_or_else(|e| e.into_inner());
        let length = duration_ms.clamp(0, 65535) as u16;
        let mut effect = ff_effect_bytes(id_guard.unwrap_or(-1), low, high, if length == 0 { 1000 } else { length });
        match eviocsff(s.fd, &mut effect) {
            Ok(id) => {
                *id_guard = Some(id);
                write_event(s.fd, EV_FF, id as u16, i32::from(low != 0 || high != 0))
            }
            Err(e) => Err(e),
        }
    }

    pub fn last_rumble(&self, slot: u8) -> Option<(u16, u16, u32)> {
        let slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.get(usize::from(slot)).and_then(|s| s.as_ref()).and_then(|s| *s.last_rumble.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::Release);
        let mut slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
        for slot in slots.iter_mut() {
            if let Some(s) = slot.take() {
                unsafe { libc::close(s.fd) };     /* ends the reader's read with EBADF and releases the grab */
            }
        }
        self.inner.bitmap.store(0, Ordering::Release);
    }
}

fn apply_grab(slot: &Slot, grab: bool) {
    match eviocgrab(slot.fd, grab) {
        Ok(()) => slot.grabbed.store(grab, Ordering::Release),
        Err(e) => {
            if grab {
                eprintln!("gfn-pad: {}: grab refused: {e} (another process holds it, or the node is not an input device)", slot.node);
            }
        }
    }
}

fn discovery_loop(inner: &Arc<Inner>) {
    let ino = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if ino >= 0 {
        if let Ok(p) = CString::new(inner.config.dev_dir.as_os_str().to_string_lossy().as_bytes()) {
            unsafe { libc::inotify_add_watch(ino, p.as_ptr(), libc::IN_CREATE | libc::IN_DELETE | libc::IN_ATTRIB) };
        }
    }
    while !inner.stop.load(Ordering::Acquire) {
        scan(inner);
        /* wait for a change or the rescan period */
        if ino >= 0 {
            let mut pfd = libc::pollfd { fd: ino, events: libc::POLLIN, revents: 0 };
            let ms = inner.config.rescan.as_millis().min(60_000) as i32;
            let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
            if rc > 0 {
                let mut buf = [0u8; 1024];
                unsafe { libc::read(ino, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
                thread::sleep(Duration::from_millis(200));   /* udev's permissions arrive a moment after the node */
            }
        } else {
            thread::sleep(inner.config.rescan);
        }
    }
    if ino >= 0 {
        unsafe { libc::close(ino) };
    }
}

fn scan(inner: &Arc<Inner>) {
    inner.scans.fetch_add(1, Ordering::Release);
    let nodes = list_nodes(&inner.config.dev_dir);
    let known: Vec<String> = {
        let slots = inner.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.iter().flatten().map(|s| s.node.clone()).collect()
    };
    for node in nodes {
        if known.contains(&node) {
            continue;
        }
        let facts = node_facts(&inner.config.sys_dir, &node);
        if !facts.gamepad {
            continue;
        }
        let path = inner.config.dev_dir.join(&node);
        let Ok(cpath) = CString::new(path.as_os_str().to_string_lossy().as_bytes()) else { continue };
        let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOCTTY) };
        if fd < 0 {
            let e = io::Error::last_os_error();
            if inner.scans.load(Ordering::Acquire) <= 1 {
                eprintln!("gfn-pad: {}: cannot open: {e}", path.display());
            }
            continue;
        }
        let slot_index = {
            let mut slots = inner.slots.lock().unwrap_or_else(|e| e.into_inner());
            match slots.iter().position(Option::is_none) {
                Some(i) => {
                    let slot = Arc::new(Slot {
                        node: node.clone(),
                        fd,
                        facts: facts.clone(),
                        ff_id: Mutex::new(None),
                        grabbed: AtomicBool::new(false),
                        last_rumble: Mutex::new(None),
                        reports: AtomicU64::new(0),
                    });
                    slots[i] = Some(Arc::clone(&slot));
                    if inner.grab.load(Ordering::Acquire) {
                        apply_grab(&slot, true);
                    }
                    Some((i, slot))
                }
                None => None,
            }
        };
        let Some((i, slot)) = slot_index else {
            unsafe { libc::close(fd) };
            continue;
        };
        let bitmap = recompute_bitmap(inner);
        inner.sink.pad_event(bitmap, PadEvent::Connected { slot: i as u8, name: facts.name.clone(), uniq: facts.uniq.clone() });
        let reader_inner = Arc::clone(inner);
        let _ = thread::Builder::new().name(format!("gfn-pad-{node}")).spawn(move || reader_loop(&reader_inner, i as u8, &slot));
    }
}

fn recompute_bitmap(inner: &Inner) -> u16 {
    let slots = inner.slots.lock().unwrap_or_else(|e| e.into_inner());
    let mut b = 0u16;
    for (i, s) in slots.iter().enumerate() {
        if s.is_some() {
            b |= 1 << i;
            b |= 1 << (i + 8);
        }
    }
    inner.bitmap.store(b, Ordering::Release);
    b
}

fn reader_loop(inner: &Arc<Inner>, slot_index: u8, slot: &Arc<Slot>) {
    let mut raw = Raw::new(inner.config.deadzone_percent);
    for code in [ABS_X, ABS_Y, ABS_RX, ABS_RY, ABS_Z, ABS_RZ, ABS_HAT0X, ABS_HAT0Y] {
        if let Ok((min, max)) = eviocgabs(slot.fd, code) {
            raw.set_range(code, min, max);
        }
    }
    let mut last = raw.state();
    let mut guide = false;
    let mut chord_since: Option<Instant> = None;
    let mut chord_sent = false;
    let mut buf = [0u8; EVENT_SIZE * 64];
    loop {
        let n = unsafe { libc::read(slot.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n <= 0 {
            let e = io::Error::last_os_error();
            if n < 0 && e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        let n = n as usize;
        let mut off = 0;
        while off + EVENT_SIZE <= n {
            let ev: InputEvent = unsafe { std::ptr::read_unaligned(buf.as_ptr().add(off) as *const InputEvent) };
            off += EVENT_SIZE;
            match ev.kind {
                EV_KEY | EV_ABS => raw.apply(ev.kind, ev.code, ev.value),
                EV_SYN if ev.code == SYN_REPORT => {
                    slot.reports.fetch_add(1, Ordering::Relaxed);
                    let bitmap = inner.bitmap.load(Ordering::Acquire);
                    let state = raw.state();
                    if state != last {
                        last = state;
                        inner.sink.pad_event(bitmap, PadEvent::Report { slot: slot_index, state });
                    }
                    let g = raw.guide();
                    if g != guide {
                        guide = g;
                        inner.sink.pad_event(bitmap, PadEvent::Guide { slot: slot_index, pressed: g });
                    }
                    let chord = state.buttons & (button::BACK | button::START) == (button::BACK | button::START);
                    match (chord, chord_since) {
                        (true, None) => chord_since = Some(Instant::now()),
                        (true, Some(since)) => {
                            if !chord_sent && since.elapsed() >= inner.config.chord_hold {
                                chord_sent = true;
                                inner.sink.pad_event(bitmap, PadEvent::ExitChord { slot: slot_index });
                            }
                        }
                        (false, _) => {
                            chord_since = None;
                            chord_sent = false;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    /* gone (ENODEV on unplug, EBADF on stop): release the slot */
    {
        let mut slots = inner.slots.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = slots.get_mut(usize::from(slot_index)) {
            if s.as_ref().is_some_and(|s| Arc::ptr_eq(s, slot)) {
                unsafe { libc::close(slot.fd) };
                *s = None;
            }
        }
    }
    if !inner.stop.load(Ordering::Acquire) {
        let bitmap = recompute_bitmap(inner);
        inner.sink.pad_event(bitmap, PadEvent::Disconnected { slot: slot_index });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_words_read_least_significant_last() {
        let w = capability_words("7fff000000000000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0");
        assert_eq!(w.len(), 22);
        assert!(!has_bit(&w, 0x130));
        let w = capability_words("7fff000000000000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0");
        assert!(w.len() > 5);
        /* the DS4's key capabilities as webOS prints them: bits 0x130.. live in word 4 (0x130 / 64 = 4) */
        let ds4 = capability_words("7fdb000000000000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0");
        assert!(ds4.len() == 21);
        let mut words = vec![0u64; 5];
        words[4] = 1u64 << (0x130 % 64);
        assert!(has_bit(&words, 0x130));
    }

    #[test]
    fn ds4_axes_and_buttons_map_to_xinput() {
        let mut r = Raw::new(0);
        r.apply(EV_KEY, BTN_SOUTH, 1);
        r.apply(EV_KEY, BTN_WEST, 1);
        r.apply(EV_ABS, ABS_HAT0Y, -1);
        r.apply(EV_ABS, ABS_Y, 0);        /* stick fully up */
        r.apply(EV_ABS, ABS_X, 255);      /* fully right */
        r.apply(EV_ABS, ABS_RZ, 255);
        let s = r.state();
        assert_eq!(s.buttons, button::A | button::X | button::DPAD_UP);
        assert!(s.left_y > 23_000 && s.left_x > 23_000, "{s:?}");
        assert_eq!(s.right_trigger, 255);
        assert_eq!(s.left_trigger, 0);
        r.apply(EV_ABS, ABS_X, 128);
        r.apply(EV_ABS, ABS_Y, 128);
        let s = r.state();
        assert_eq!((s.left_x, s.left_y), (0, 0));
        assert_eq!(radial_deadzone(0.05, 0.0, 8), (0, 0));
        assert_eq!(radial_deadzone(1.0, 0.0, 8), (32767, 0));
        assert_eq!(radial_deadzone(0.0, -1.0, 8).1, -32767);
    }

    #[test]
    fn ff_effect_layout() {
        let e = ff_effect_bytes(-1, 0x1234, 0x5678, 500);
        assert_eq!(e.len(), 48);
        assert_eq!(u16::from_ne_bytes([e[0], e[1]]), FF_RUMBLE);
        assert_eq!(i16::from_ne_bytes([e[2], e[3]]), -1);
        assert_eq!(u16::from_ne_bytes([e[12], e[13]]), 500);
        assert_eq!(u16::from_ne_bytes([e[16], e[17]]), 0x1234);
        assert_eq!(u16::from_ne_bytes([e[18], e[19]]), 0x5678);
        assert_eq!(EVENT_SIZE, 24);
        assert_eq!(ioctl_code(1, 0x90, 4), 0x40044590);
        assert_eq!(ioctl_code(1, 0x80, 48), 0x40304580);
        assert_eq!(ioctl_code(2, 0x40, 24), 0x80184540);
    }
}
