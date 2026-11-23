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
//! Two more things live beside the readers. A boost thread asks `lumatv-helper` to give each reader `SCHED_FIFO` 10 and an affinity
//! off CPU 0 while the stream is in front ([`boost`]); a stats thread follows every button press from the kernel's time stamp to the
//! engine's handoff and writes the figures out ([`latency`]).
//!
//! The directories are parameters so the host tests can stand in a device tree with a pseudo-terminal for the node.
use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub mod boost;
pub mod lab;
pub mod latency;
pub mod menu;
pub mod tool;

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
    /// A changed report; `bitmap` says which slots are connected (bit i) and that they are XInput style (bit i + 8). `press` names the
    /// press in the latency tap when a button went down in this report; the sink gives it back to [`latency::Tap::link`].
    Report { slot: u8, state: PadState, press: Option<u64> },
    /// The PS/Xbox button, or Start held alone for [`PadConfig::chord_hold`] (released when Start is): a local action, never sent to
    /// the game as a button of its own.
    Guide { slot: u8, pressed: bool },
    /// Select and Start held together for a second: the app's own way out of a stream.
    ExitChord { slot: u8 },
}

/// One report as the kernel stamped it, before any dead zone or curve: what a controller test draws and times.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RawReport {
    /// The kernel's stamp, on the monotonic clock.
    pub kernel_ns: u64,
    /// When the reader woke with it, on the monotonic clock.
    pub read_ns: u64,
    /// Sticks -1..1 with up positive.
    pub left: (f32, f32),
    pub right: (f32, f32),
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub buttons: u16,
}

pub trait PadSink: Send + Sync {
    fn pad_event(&self, bitmap: u16, event: PadEvent);

    /// The latency figures after a second in which presses finished (the engine puts them on the app's stats line).
    fn latency(&self, _summary: &latency::Summary) {}

    /// Every report, changed or not, while [`PadConfig::raw`] asks for them.
    fn raw(&self, _slot: u8, _report: &RawReport) {}
}

/// How a stick's travel past the dead zone maps to what the game sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Curve {
    #[default]
    Linear,
    /// Squared: fine control near the centre.
    Precise,
    /// Square root: quick turns off the centre.
    Quick,
}

impl Curve {
    pub fn name(self) -> &'static str {
        match self {
            Curve::Linear => "linear",
            Curve::Precise => "precise",
            Curve::Quick => "quick",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Curve::Linear, Curve::Precise, Curve::Quick].into_iter().find(|c| c.name() == name)
    }

    fn apply(self, t: f64) -> f64 {
        match self {
            Curve::Linear => t,
            Curve::Precise => t * t,
            Curve::Quick => t.sqrt(),
        }
    }
}

/// A stick's dead zone, anti-deadzone and curve, in the order they apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Shaping {
    pub deadzone_percent: u8,
    /// The smallest deflection a moved stick sends: games with a dead zone of their own ignore anything below theirs.
    pub anti_deadzone_percent: u8,
    pub curve: Curve,
}

#[derive(Debug, Clone)]
pub struct PadConfig {
    pub dev_dir: PathBuf,
    pub sys_dir: PathBuf,
    pub rescan: Duration,
    /// Radial stick dead zone in percent, as OpenNOW applies it before the state goes on the wire.
    pub deadzone_percent: u8,
    pub anti_deadzone_percent: u8,
    pub curve: Curve,
    /// Hand every report to [`PadSink::raw`]: a controller test wants them, a stream does not.
    pub raw: bool,
    /// How long Select and Start together are held before [`PadEvent::ExitChord`], and Start alone before [`PadEvent::Guide`].
    pub chord_hold: Duration,
    /// The helper's loopback address for the readers' real-time boost; `None` asks for nothing.
    pub boost: Option<SocketAddr>,
    /// How often the boost is renewed while the stream is in front (the helper's lease is 90 s).
    pub boost_renew: Duration,
    /// Where `latency.tsv` and `latency.json` go; `None` keeps the figures in memory for the stats line.
    pub latency_dir: Option<PathBuf>,
    /// How often finished presses are collected and published.
    pub stats_every: Duration,
    /// The press tap: the engine's own unless a test brings one.
    pub tap: Arc<latency::Tap>,
    /// The clock an event's stamp is on when the node refuses `EVIOCSCLOCKID` (a stand-in node does; a real one never).
    pub stamp_clock: latency::Clock,
}

impl Default for PadConfig {
    fn default() -> Self {
        Self {
            dev_dir: PathBuf::from("/dev/input"),
            sys_dir: PathBuf::from("/sys/class/input"),
            rescan: Duration::from_secs(2),
            deadzone_percent: 8,
            anti_deadzone_percent: 0,
            curve: Curve::Linear,
            raw: false,
            chord_hold: Duration::from_secs(1),
            boost: None,
            boost_renew: Duration::from_secs(30),
            latency_dir: None,
            stats_every: Duration::from_secs(1),
            tap: Arc::clone(latency::global()),
            stamp_clock: latency::Clock::Real,
        }
    }
}

impl PadConfig {
    /// `LUMATV_GFN_PAD_DEV` and `LUMATV_GFN_PAD_SYS` move the directories (the tests), `LUMATV_GFN_PAD_DEADZONE` the dead zone,
    /// `LUMATV_GFN_BOOST=0` or `LUMATV_GFN_BOOST_ADDR` the helper's boost, `LUMATV_GFN_LATENCY_DIR` where the latency files go.
    pub fn from_env() -> Self {
        let mut c = Self { boost: boost::addr_from_env(), latency_dir: std::env::var_os("LUMATV_GFN_LATENCY_DIR").map(PathBuf::from), ..Self::default() };
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

    pub fn shaping(&self) -> Shaping {
        Shaping { deadzone_percent: self.deadzone_percent, anti_deadzone_percent: self.anti_deadzone_percent, curve: self.curve }
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

/// LG's own input nodes ("Smart Remote RCU Input", "LGE Network Input" and the remotes) declare every key and every axis, so they pass
/// the gamepad test; read and grabbed as pads they were extra controllers in the game beside the real one.
fn lg_virtual(name: &str) -> bool {
    name.starts_with("LGE ") || name.contains("Smart Remote")
}

pub fn node_facts(sys_dir: &Path, node: &str) -> NodeFacts {
    let dev = sys_dir.join(node).join("device");
    let ev = capability_words(&read_trim(&dev.join("capabilities/ev")));
    let key = capability_words(&read_trim(&dev.join("capabilities/key")));
    let ff = capability_words(&read_trim(&dev.join("capabilities/ff")));
    let name = read_trim(&dev.join("name"));
    NodeFacts {
        gamepad: !lg_virtual(&name) && has_bit(&ev, u32::from(EV_KEY)) && has_bit(&ev, u32::from(EV_ABS)) && has_bit(&key, u32::from(BTN_SOUTH)),
        name,
        uniq: read_trim(&dev.join("uniq")),
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
    shaping: Shaping,
}

impl Raw {
    pub fn new(shaping: Shaping) -> Self {
        let mut ranges = HashMap::new();
        for code in [ABS_X, ABS_Y, ABS_RX, ABS_RY, ABS_Z, ABS_RZ] {
            ranges.insert(code, AbsRange { min: 0, max: 255 });    /* the DualShock 4 shape; EVIOCGABS replaces it where it answers */
        }
        for code in [ABS_HAT0X, ABS_HAT0Y] {
            ranges.insert(code, AbsRange { min: -1, max: 1 });
        }
        Self { keys: HashMap::new(), abs: HashMap::new(), ranges, shaping }
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

    /// This report's sticks unshaped, beside `state`'s buttons and triggers (this report's [`Raw::state`]), with its stamps.
    pub fn report(&self, state: &PadState, kernel_ns: u64, read_ns: u64) -> RawReport {
        RawReport {
            kernel_ns,
            read_ns,
            left: (self.axis(ABS_X) as f32, -self.axis(ABS_Y) as f32),
            right: (self.axis(ABS_RX) as f32, -self.axis(ABS_RY) as f32),
            left_trigger: state.left_trigger,
            right_trigger: state.right_trigger,
            buttons: state.buttons,
        }
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
        let (lx, ly) = shape(self.axis(ABS_X), self.axis(ABS_Y), self.shaping);
        let (rx, ry) = shape(self.axis(ABS_RX), self.axis(ABS_RY), self.shaping);
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

/// The radial dead zone, then the curve, then the anti-deadzone, on -1..1 inputs, to the i16 the wire carries. With no curve and no
/// anti-deadzone it is exactly [`radial_deadzone`].
pub fn shape(x: f64, y: f64, s: Shaping) -> (i16, i16) {
    let dz = f64::from(s.deadzone_percent) / 100.0;
    let mag = x.hypot(y);
    if mag <= dz {
        return (0, 0);
    }
    let t = ((mag - dz) / (1.0 - dz)).min(1.0);
    let anti = f64::from(s.anti_deadzone_percent.min(99)) / 100.0;
    let out = anti + (1.0 - anti) * s.curve.apply(t);
    let f = out / mag;
    let to = |v: f64| ((v * f).clamp(-1.0, 1.0) * 32767.0).round() as i16;
    (to(x), to(y))
}

fn ioctl_code(dir: u32, nr: u32, size: u32) -> u64 {
    u64::from((dir << 30) | (size << 16) | (0x45 << 8) | nr)
}

/// `EVIOCGRAB` takes its flag as the ioctl argument itself: the kernel grabs on any non-zero argument, so a pointer to 0 asked
/// for a second grab (`EBUSY`) and the menu kept the pad through every stream.
fn eviocgrab(fd: RawFd, grab: bool) -> io::Result<()> {
    grab_ioctl(fd, libc::c_ulong::from(grab))
}

#[cfg(not(test))]
fn grab_ioctl(fd: RawFd, arg: libc::c_ulong) -> io::Result<()> {
    let rc = unsafe { libc::ioctl(fd, ioctl_code(1, 0x90, 4) as _, arg) };
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// The kernel's grab rule for the tests' pseudo-terminals, which have no `EVIOCGRAB`: one holder per device, any second grab is
/// `EBUSY` (the holder's own included), and only the holder can let go.
#[cfg(test)]
fn grab_ioctl(fd: RawFd, arg: libc::c_ulong) -> io::Result<()> {
    let grab = arg != 0;
    static HOLDERS: Mutex<Vec<((u64, u64), RawFd)>> = Mutex::new(Vec::new());
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let device = (st.st_dev, st.st_ino);
    let mut holders = HOLDERS.lock().unwrap_or_else(|e| e.into_inner());
    match (grab, holders.iter().position(|(d, _)| *d == device)) {
        (true, None) => {
            holders.push((device, fd));
            Ok(())
        }
        (true, Some(_)) => Err(io::Error::from_raw_os_error(libc::EBUSY)),
        (false, Some(i)) if holders[i].1 == fd => {
            holders.remove(i);
            Ok(())
        }
        (false, _) => Err(io::Error::from_raw_os_error(libc::EINVAL)),
    }
}

fn eviocgabs(fd: RawFd, code: u16) -> io::Result<(i32, i32)> {
    let mut info = [0i32; 6];
    let rc = unsafe { libc::ioctl(fd, ioctl_code(2, 0x40 + u32::from(code), 24) as _, info.as_mut_ptr()) };
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok((info[1], info[2])) }
}

/// `EVIOCSCLOCKID`: the clock this descriptor's event stamps use (`CLOCK_MONOTONIC`, so they compare with the engine's own times).
fn eviocsclockid(fd: RawFd, clock: libc::c_int) -> io::Result<()> {
    let rc = unsafe { libc::ioctl(fd, ioctl_code(1, 0xa0, 4) as _, &clock as *const libc::c_int) };
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
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
    /// The reader thread's kernel thread id, 0 until it has started: what the boost is asked for.
    tid: AtomicI32,
}

struct Inner {
    config: PadConfig,
    sink: Arc<dyn PadSink>,
    slots: Mutex<[Option<Arc<Slot>>; MAX_PADS]>,
    grab: AtomicBool,
    stop: AtomicBool,
    bitmap: AtomicU16,
    scans: AtomicU64,
    /// Set when the boost thread has something to do (the grab changed, a reader started, the runtime stops); it also wakes on a timer.
    wake: (Mutex<bool>, Condvar),
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
            wake: (Mutex::new(false), Condvar::new()),
        });
        let rt = Self { inner: Arc::clone(&inner) };
        let discovery = Arc::clone(&inner);
        let _ = thread::Builder::new().name("gfn-pad-discovery".to_owned()).spawn(move || discovery_loop(&discovery));
        if let Some(addr) = inner.config.boost {
            let boosting = Arc::clone(&inner);
            let _ = thread::Builder::new().name("gfn-pad-boost".to_owned()).spawn(move || boost_loop(&boosting, addr));
        }
        let stats = Arc::clone(&inner);
        let _ = thread::Builder::new().name("gfn-pad-stats".to_owned()).spawn(move || stats_loop(&stats));
        rt
    }

    /// Hold the pads exclusively (the stream is in front) or share them again. The flag changes under the slots' lock, which the
    /// rescan's retry also holds while it reads the flag, so a retry can never take a grab back after it was given up.
    pub fn set_grab(&self, grab: bool) {
        let slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
        self.inner.grab.store(grab, Ordering::Release);
        for slot in slots.iter().flatten().filter(|s| s.grabbed.load(Ordering::Acquire) != grab) {
            apply_grab(slot, grab);
        }
        drop(slots);
        wake_boost(&self.inner);
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
        wake_boost(&self.inner);
        let mut slots = self.inner.slots.lock().unwrap_or_else(|e| e.into_inner());
        for slot in slots.iter_mut() {
            if let Some(s) = slot.take() {
                // The reader's blocked read keeps the file, and with it the grab, alive past close: let go first.
                if s.grabbed.load(Ordering::Acquire) {
                    apply_grab(&s, false);
                }
                unsafe { libc::close(s.fd) };
            }
        }
        self.inner.bitmap.store(0, Ordering::Release);
    }
}

fn apply_grab(slot: &Slot, grab: bool) {
    match eviocgrab(slot.fd, grab) {
        Ok(()) => slot.grabbed.store(grab, Ordering::Release),
        // A failed release still leaves this process without the grab (it was never held, or the node is gone).
        Err(_) if !grab => slot.grabbed.store(false, Ordering::Release),
        Err(e) => {
            eprintln!("gfn-pad: {}: grab refused: {e} (another process holds it, or the node is not an input device)", slot.node);
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
    // A grab refused while another process held the pad (the menu, until it lets go) is asked for again. The flag is read under
    // the slots' lock: see `set_grab`.
    {
        let slots = inner.slots.lock().unwrap_or_else(|e| e.into_inner());
        if inner.grab.load(Ordering::Acquire) {
            for slot in slots.iter().flatten().filter(|s| !s.grabbed.load(Ordering::Acquire)) {
                if eviocgrab(slot.fd, true).is_ok() {
                    slot.grabbed.store(true, Ordering::Release);
                    eprintln!("gfn-pad: {}: grabbed after an earlier refusal", slot.node);
                }
            }
        }
    }
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
                        tid: AtomicI32::new(0),
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

fn wake_boost(inner: &Inner) {
    *inner.wake.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
    inner.wake.1.notify_all();
}

fn reader_tids(inner: &Inner) -> Vec<i32> {
    let slots = inner.slots.lock().unwrap_or_else(|e| e.into_inner());
    slots.iter().flatten().map(|s| s.tid.load(Ordering::Acquire)).filter(|&t| t > 0).collect()
}

/// While the grab is held (the stream is in front): ask for each reader's boost now and at every renewal; when it is let go, or the
/// runtime stops, release what was boosted. The helper's refusals are logged once for each thread and reason.
fn boost_loop(inner: &Arc<Inner>, addr: SocketAddr) {
    let mut boosted: Vec<i32> = Vec::new();
    let mut said: Vec<(i32, String)> = Vec::new();
    loop {
        {
            let mut dirty = inner.wake.0.lock().unwrap_or_else(|e| e.into_inner());
            if !*dirty {
                dirty = inner.wake.1.wait_timeout(dirty, inner.config.boost_renew).unwrap_or_else(|e| e.into_inner()).0;
            }
            *dirty = false;
        }
        let stopping = inner.stop.load(Ordering::Acquire);
        if inner.grab.load(Ordering::Acquire) && !stopping {
            let tids = reader_tids(inner);
            for &tid in &tids {
                if let Err(why) = boost::boost(addr, tid) {
                    if !said.iter().any(|(t, w)| *t == tid && *w == why) {
                        eprintln!("gfn-pad: no boost for thread {tid}: {why}");
                        said.push((tid, why));
                    }
                }
            }
            boosted = tids;
        } else {
            for tid in boosted.drain(..) {
                boost::unboost(addr, tid);
            }
            said.clear();
        }
        if stopping {
            break;
        }
    }
}

/// Once a second: collect the presses that finished, append them to `latency.tsv`, rewrite `latency.json`, tell the sink.
fn stats_loop(inner: &Arc<Inner>) {
    let mut tsv: Option<fs::File> = None;
    let mut lines = 0usize;
    while !inner.stop.load(Ordering::Acquire) {
        thread::sleep(inner.config.stats_every);
        let fresh = inner.config.tap.take_finished(latency::mono_ns());
        if fresh.is_empty() {
            continue;
        }
        let summary = inner.config.tap.summary();
        if let Some(dir) = &inner.config.latency_dir {
            let _ = fs::create_dir_all(dir);
            if tsv.is_none() {
                let path = dir.join("latency.tsv");
                let fresh_file = fs::metadata(&path).map(|m| m.len() == 0).unwrap_or(true);
                tsv = fs::OpenOptions::new().create(true).append(true).open(path).ok();
                if let (Some(f), true) = (tsv.as_mut(), fresh_file) {
                    let _ = writeln!(f, "{}", latency::Done::TSV_HEADER);
                }
            }
            // A file that cannot grow without bound: 50 000 presses is hours of play.
            if let (Some(f), true) = (tsv.as_mut(), lines < 50_000) {
                let text: String = fresh.iter().map(|d| format!("{}\n", d.tsv())).collect();
                let _ = f.write_all(text.as_bytes());
                lines += fresh.len();
            }
            let tmp = dir.join("latency.json.tmp");
            if fs::write(&tmp, summary.json()).is_ok() {
                let _ = fs::rename(&tmp, dir.join("latency.json"));
            }
        }
        inner.sink.latency(&summary);
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

/// What a button held for the hold time has to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// Select and Start together.
    Chord,
    /// Start alone: the Guide's press, and its release when Start comes up.
    Guide(bool),
}

/// The two long holds on Start. A Start that ever had Select beside it belongs to the chord until it is released, so the chord wins
/// and never also opens the menu.
#[derive(Debug, Default)]
struct Holds {
    chord: Option<Instant>,
    chord_sent: bool,
    start: Option<Instant>,
    start_sent: bool,
    chorded: bool,
}

impl Holds {
    /// Moves on to the buttons held at `now`; the hold that has just been reached, if any.
    fn step(&mut self, buttons: u16, now: Instant, hold: Duration) -> Option<Held> {
        let start = buttons & button::START != 0;
        let chord = start && buttons & button::BACK != 0;
        let mut held = None;
        if !start {
            if std::mem::take(&mut self.start_sent) {
                held = Some(Held::Guide(false));
            }
            self.start = None;
            self.chorded = false;
        } else if chord {
            self.chorded = true;
            self.start = None;
        } else if !self.chorded && self.start.is_none() {
            self.start = Some(now);
        }
        if chord {
            self.chord.get_or_insert(now);
        } else {
            self.chord = None;
            self.chord_sent = false;
        }
        if self.chord.is_some_and(|since| !self.chord_sent && now.duration_since(since) >= hold) {
            self.chord_sent = true;
            held = Some(Held::Chord);
        }
        if self.start.is_some_and(|since| !self.start_sent && now.duration_since(since) >= hold) {
            self.start_sent = true;
            held = Some(Held::Guide(true));
        }
        held
    }

    /// When the next hold comes due, so a reader with nothing else to wait for still wakes for it.
    fn due(&self, hold: Duration) -> Option<Instant> {
        let chord = self.chord.filter(|_| !self.chord_sent);
        let start = self.start.filter(|_| !self.start_sent);
        chord.into_iter().chain(start).map(|since| since + hold).min()
    }
}

fn reader_loop(inner: &Arc<Inner>, slot_index: u8, slot: &Arc<Slot>) {
    // SAFETY: gettid takes nothing and cannot fail.
    slot.tid.store(unsafe { libc::syscall(libc::SYS_gettid) } as i32, Ordering::Release);
    wake_boost(inner);
    let clock = if eviocsclockid(slot.fd, libc::CLOCK_MONOTONIC).is_ok() { latency::Clock::Mono } else { inner.config.stamp_clock };
    let tap = &inner.config.tap;
    let mut raw = Raw::new(inner.config.shaping());
    for code in [ABS_X, ABS_Y, ABS_RX, ABS_RY, ABS_Z, ABS_RZ, ABS_HAT0X, ABS_HAT0Y] {
        if let Ok((min, max)) = eviocgabs(slot.fd, code) {
            raw.set_range(code, min, max);
        }
    }
    let mut last = raw.state();
    let mut guide = false;
    let mut holds = Holds::default();
    let hold = inner.config.chord_hold;
    let announce = |held: Held| {
        let event = match held {
            Held::Chord => PadEvent::ExitChord { slot: slot_index },
            Held::Guide(pressed) => PadEvent::Guide { slot: slot_index, pressed },
        };
        inner.sink.pad_event(inner.bitmap.load(Ordering::Acquire), event);
    };
    let mut buf = [0u8; EVENT_SIZE * 64];
    loop {
        // A hold falls due while the pad sits still, so the wait for its next report ends when the hold does.
        if let Some(at) = holds.due(hold) {
            let mut pfd = libc::pollfd { fd: slot.fd, events: libc::POLLIN, revents: 0 };
            let ms = i32::try_from(at.saturating_duration_since(Instant::now()).as_millis() + 1).unwrap_or(i32::MAX);
            // SAFETY: one valid pollfd, and the count says so.
            let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
            if rc == 0 {
                if let Some(held) = holds.step(last.buttons, Instant::now(), hold) {
                    announce(held);
                }
                continue;
            }
            if rc < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
        }
        let n = unsafe { libc::read(slot.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n <= 0 {
            let e = io::Error::last_os_error();
            if n < 0 && e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        let read_ns = latency::mono_ns();
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
                    if inner.config.raw {
                        inner.sink.raw(slot_index, &raw.report(&state, latency::stamp_ns(ev.tv_sec, ev.tv_usec, clock), read_ns));
                    }
                    if state != last {
                        let new_buttons = state.buttons & !last.buttons;
                        last = state;
                        let press = (new_buttons != 0).then(|| tap.begin(slot_index, new_buttons, latency::stamp_ns(ev.tv_sec, ev.tv_usec, clock), read_ns));
                        inner.sink.pad_event(bitmap, PadEvent::Report { slot: slot_index, state, press });
                        if let Some(id) = press {
                            tap.pushed(id, latency::mono_ns());
                        }
                    }
                    let g = raw.guide();
                    if g != guide {
                        guide = g;
                        inner.sink.pad_event(bitmap, PadEvent::Guide { slot: slot_index, pressed: g });
                    }
                    if let Some(held) = holds.step(state.buttons, Instant::now(), hold) {
                        announce(held);
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
        let mut r = Raw::new(Shaping::default());
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
    fn sticks_take_the_dead_zone_then_the_curve_then_the_anti_deadzone() {
        let plain = Shaping { deadzone_percent: 8, ..Shaping::default() };
        for (x, y) in [(0.05, 0.0), (0.3, -0.4), (1.0, 0.0), (-0.7, 0.7)] {
            assert_eq!(shape(x, y, plain), radial_deadzone(x, y, 8));
        }
        let lifted = Shaping { anti_deadzone_percent: 20, ..plain };
        assert_eq!(shape(0.05, 0.0, lifted), (0, 0), "a resting stick stays at rest");
        assert_eq!(shape(0.1, 0.0, lifted).0, 7123, "just past the dead zone it starts at 20 %");
        assert_eq!(shape(1.0, 0.0, lifted).0, 32767);
        assert_eq!(shape(0.5, 0.0, Shaping { curve: Curve::Precise, ..Shaping::default() }).0, 8192);
        assert_eq!(shape(0.0, 0.25, Shaping { curve: Curve::Quick, ..Shaping::default() }).1, 16384);
        assert_eq!(Curve::from_name("quick"), Some(Curve::Quick));
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
        assert_eq!(ioctl_code(1, 0xa0, 4), 0x400445a0, "EVIOCSCLOCKID");
    }

    /// A pseudo-terminal standing in for `/dev/input/event0`: the test writes `input_event` records to the master and the reader
    /// reads them from the slave exactly as it would from the evdev node (the ioctls fail with ENOTTY, as on any non-evdev file).
    struct FakePad {
        root: PathBuf,
        master: RawFd,
        slave: RawFd,
    }

    impl FakePad {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!("gfn-pad-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let (dev, sys) = (root.join("dev"), root.join("sys/event0/device"));
            fs::create_dir_all(&dev).unwrap();
            fs::create_dir_all(sys.join("capabilities")).unwrap();
            fs::write(sys.join("name"), "Wireless Controller\n").unwrap();
            fs::write(sys.join("uniq"), "1c:a0:b8:4c:7a:99\n").unwrap();
            fs::write(sys.join("capabilities/ev"), "b\n").unwrap();
            fs::write(sys.join("capabilities/key"), "1000000000000 0 0 0 0\n").unwrap();
            fs::write(sys.join("capabilities/ff"), "0\n").unwrap();
            // SAFETY: plain libc calls on descriptors this function owns.
            let (master, slave, name) = unsafe {
                let m = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
                assert!(m >= 0 && libc::grantpt(m) == 0 && libc::unlockpt(m) == 0);
                let name = std::ffi::CStr::from_ptr(libc::ptsname(m)).to_string_lossy().into_owned();
                let s = libc::open(CString::new(name.clone()).unwrap().as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
                assert!(s >= 0);
                let mut t: libc::termios = std::mem::zeroed();
                libc::tcgetattr(s, &mut t);
                libc::cfmakeraw(&mut t);
                libc::tcsetattr(s, libc::TCSANOW, &t);
                (m, s, name)
            };
            std::os::unix::fs::symlink(name, dev.join("event0")).unwrap();
            Self { root, master, slave }
        }

        fn config(&self) -> PadConfig {
            PadConfig {
                dev_dir: self.root.join("dev"),
                sys_dir: self.root.join("sys"),
                rescan: Duration::from_millis(50),
                stats_every: Duration::from_millis(40),
                tap: Arc::new(latency::Tap::new(None)),
                stamp_clock: latency::Clock::Mono,
                latency_dir: Some(self.root.join("out")),
                ..PadConfig::default()
            }
        }

        /// One report: the events, then `SYN_REPORT`, all stamped `stamp_ns` on the monotonic clock, in a single write.
        fn report(&self, events: &[(u16, u16, i32)], stamp_ns: u64) {
            let mut bytes = Vec::new();
            for &(kind, code, value) in events.iter().chain([(EV_SYN, SYN_REPORT, 0)].iter()) {
                let ev = InputEvent { tv_sec: (stamp_ns / 1_000_000_000) as i64, tv_usec: ((stamp_ns % 1_000_000_000) / 1000) as i64, kind, code, value };
                bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(&ev as *const InputEvent as *const u8, EVENT_SIZE) });
            }
            assert_eq!(unsafe { libc::write(self.master, bytes.as_ptr().cast(), bytes.len()) }, bytes.len() as isize);
        }
    }

    impl Drop for FakePad {
        fn drop(&mut self) {
            unsafe {
                libc::close(self.master);
                libc::close(self.slave);
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// A sink that keeps what it is given and, like the engine's, tags each press's packet and hands it on.
    struct Collect {
        tap: Arc<latency::Tap>,
        events: Mutex<Vec<PadEvent>>,
        summaries: Mutex<Vec<latency::Summary>>,
        raws: Mutex<Vec<RawReport>>,
    }

    impl PadSink for Collect {
        fn pad_event(&self, _bitmap: u16, event: PadEvent) {
            if let PadEvent::Report { press: Some(id), .. } = &event {
                let at = Instant::now();
                self.tap.link(*id, at);
                self.tap.handoff(at, latency::mono_ns());
            }
            self.events.lock().unwrap().push(event);
        }

        fn latency(&self, summary: &latency::Summary) {
            self.summaries.lock().unwrap().push(summary.clone());
        }

        fn raw(&self, _slot: u8, report: &RawReport) {
            self.raws.lock().unwrap().push(*report);
        }
    }

    fn collect(cfg: &PadConfig) -> Arc<Collect> {
        Arc::new(Collect { tap: Arc::clone(&cfg.tap), events: Mutex::default(), summaries: Mutex::default(), raws: Mutex::default() })
    }

    fn wait_for(what: &str, cond: impl Fn() -> bool) {
        let t0 = Instant::now();
        while !cond() {
            assert!(t0.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn reports(sink: &Collect) -> Vec<(PadState, Option<u64>)> {
        sink.events.lock().unwrap().iter().filter_map(|e| if let PadEvent::Report { state, press, .. } = e { Some((*state, *press)) } else { None }).collect()
    }

    #[test]
    fn every_report_reaches_the_raw_sink_unshaped_with_its_stamp() {
        let pad = FakePad::new("raw");
        let cfg = PadConfig { raw: true, deadzone_percent: 50, ..pad.config() };
        let sink = collect(&cfg);
        let rt = PadRuntime::start(cfg.clone(), sink.clone());
        wait_for("the pad", || rt.bitmap() == 0x0101);
        let stamp = latency::mono_ns();
        pad.report(&[(EV_ABS, ABS_X, 160), (EV_ABS, ABS_Y, 0)], stamp);
        pad.report(&[(EV_ABS, ABS_X, 160)], stamp + 4_000_000);
        wait_for("both reports", || sink.raws.lock().unwrap().len() == 2);
        let raws = sink.raws.lock().unwrap().clone();
        assert_eq!((raws[0].kernel_ns, raws[1].kernel_ns - raws[0].kernel_ns), (stamp / 1000 * 1000, 4_000_000));
        assert!((raws[0].left.0 - 0.252).abs() < 0.001 && raws[0].left.1 == 1.0, "up is positive: {raws:?}");
        assert_eq!(reports(&sink).len(), 1, "the unchanged second report is not a pad event");
        rt.stop();
    }

    #[test]
    fn a_press_is_stamped_from_the_kernels_time_and_followed_to_the_handoff_and_into_the_files() {
        let pad = FakePad::new("latency");
        let cfg = pad.config();
        let sink = collect(&cfg);
        let rt = PadRuntime::start(cfg.clone(), sink.clone());
        wait_for("the pad", || rt.bitmap() == 0x0101);
        // the kernel stamped the report 300 us before this process wrote it
        let stamp = latency::mono_ns() - 300_000;
        pad.report(&[(EV_KEY, BTN_SOUTH, 1)], stamp);
        wait_for("the press", || !reports(&sink).is_empty());
        pad.report(&[(EV_KEY, BTN_SOUTH, 0)], latency::mono_ns());
        wait_for("the release", || reports(&sink).len() == 2);
        let r = reports(&sink);
        assert_eq!(r[0].0.buttons, button::A);
        assert!(r[0].1.is_some(), "a button going down is a press");
        assert_eq!((r[1].0.buttons, r[1].1), (0, None), "a release is not");
        let tsv = cfg.latency_dir.as_ref().unwrap().join("latency.tsv");
        wait_for("latency.tsv", || fs::read_to_string(&tsv).is_ok_and(|t| t.lines().count() >= 2));
        let rows = tool::parse_tsv(&fs::read_to_string(&tsv).unwrap());
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].kernel_ns, rows[0].buttons, rows[0].slot), (stamp / 1000 * 1000, button::A, 0));
        assert!(rows[0].wake_us >= 300 && rows[0].wake_us < 500_000, "{rows:?}");
        // the test's sink hands the packet on from inside the report call, so the handoff falls before the reader's "queued" stamp
        assert!(rows[0].pushed_us >= rows[0].wake_us && rows[0].handoff_us >= rows[0].wake_us, "{rows:?}");
        let json = fs::read_to_string(cfg.latency_dir.as_ref().unwrap().join("latency.json")).unwrap();
        assert!(json.contains("\"samples\":1") && json.contains("\"handoff_us\":{\"p50\":"), "{json}");
        wait_for("the sink's summary", || !sink.summaries.lock().unwrap().is_empty());
        assert_eq!(sink.summaries.lock().unwrap()[0].samples, 1);
        rt.stop();
    }

    #[test]
    fn a_realtime_stamp_is_used_when_the_node_will_not_switch_clocks() {
        let pad = FakePad::new("realtime");
        let mut cfg = pad.config();
        cfg.stamp_clock = latency::Clock::Real;
        let sink = collect(&cfg);
        let rt = PadRuntime::start(cfg.clone(), sink.clone());
        wait_for("the pad", || rt.bitmap() == 0x0101);
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
        let real = ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64 - 1_000_000;
        pad.report(&[(EV_KEY, BTN_EAST, 1)], real);
        wait_for("the press", || !reports(&sink).is_empty());
        let tsv = cfg.latency_dir.as_ref().unwrap().join("latency.tsv");
        wait_for("latency.tsv", || fs::read_to_string(&tsv).is_ok_and(|t| t.lines().count() >= 2));
        let row = tool::parse_tsv(&fs::read_to_string(&tsv).unwrap())[0];
        assert!(row.wake_us >= 1000 && row.wake_us < 500_000, "1 ms old on the realtime clock, read {} us after the stamp", row.wake_us);
        rt.stop();
    }

    /// A stand-in for the helper's loopback API: records each request's path and body, answers `status` with `reply`.
    struct FakeHelper {
        addr: SocketAddr,
        seen: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl FakeHelper {
        fn start(status: u16, reply: &'static str) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
            let log = Arc::clone(&seen);
            thread::spawn(move || {
                use std::io::Read;
                for stream in listener.incoming() {
                    let Ok(mut s) = stream else { break };
                    let mut raw = Vec::new();
                    let mut buf = [0u8; 512];
                    loop {
                        let n = s.read(&mut buf).unwrap_or(0);
                        raw.extend_from_slice(&buf[..n]);
                        let text = String::from_utf8_lossy(&raw).into_owned();
                        if let Some((head, body)) = text.split_once("\r\n\r\n") {
                            let want = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                            if body.len() >= want {
                                let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                                assert!(head.to_ascii_lowercase().contains("x-luma-client: gfn-pad"), "{head}");
                                log.lock().unwrap().push((path, body.to_owned()));
                                break;
                            }
                        }
                        if n == 0 {
                            break;
                        }
                    }
                    let _ = s.write_all(format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes());
                }
            });
            Self { addr, seen }
        }

        fn count(&self, path: &str) -> usize {
            self.seen.lock().unwrap().iter().filter(|(p, _)| p == path).count()
        }
    }

    #[test]
    fn the_reader_is_boosted_while_the_grab_is_held_renewed_and_released_when_it_ends() {
        let helper = FakeHelper::start(200, r#"{"ok":true,"api":1}"#);
        let pad = FakePad::new("boost");
        let mut cfg = pad.config();
        cfg.boost = Some(helper.addr);
        cfg.boost_renew = Duration::from_millis(100);
        let sink = collect(&cfg);
        let rt = PadRuntime::start(cfg, sink.clone());
        wait_for("the pad", || rt.bitmap() == 0x0101);
        thread::sleep(Duration::from_millis(250));
        assert_eq!(helper.count(boost::BOOST_PATH), 0, "nothing is asked before the stream is in front");
        rt.set_grab(true);
        wait_for("the boost request", || helper.count(boost::BOOST_PATH) >= 1);
        let (path, body) = helper.seen.lock().unwrap()[0].clone();
        assert_eq!(path, boost::BOOST_PATH);
        let v: serde_like::Ids = serde_like::parse(&body);
        assert_eq!(v.pid, std::process::id());
        let comm = fs::read_to_string(format!("/proc/self/task/{}/comm", v.tid)).unwrap();
        assert_eq!(comm.trim(), "gfn-pad-event0", "the thread asked for is the reader's");
        wait_for("a renewal", || helper.count(boost::BOOST_PATH) >= 3);
        rt.set_grab(false);
        wait_for("the release", || helper.count(boost::UNBOOST_PATH) >= 1);
        assert!(helper.seen.lock().unwrap().iter().any(|(p, b)| p == boost::UNBOOST_PATH && *b == body));
        let asked = helper.count(boost::BOOST_PATH);
        thread::sleep(Duration::from_millis(350));
        assert_eq!(helper.count(boost::BOOST_PATH), asked, "no renewal once released");
        rt.set_grab(true);
        wait_for("a second boost", || helper.count(boost::BOOST_PATH) > asked);
        rt.stop();
        wait_for("the release at stop", || helper.count(boost::UNBOOST_PATH) >= 2);
    }

    #[test]
    fn a_refusing_or_absent_helper_leaves_the_pad_working() {
        let refusing = FakeHelper::start(403, r#"{"ok":false,"api":1,"error":{"code":"refused","message":"only the app's pad reader thread is boosted"}}"#);
        let reply = boost::post(refusing.addr, boost::BOOST_PATH, "{}").unwrap();
        assert_eq!((reply.status, reply.why().as_str()), (403, "only the app's pad reader thread is boosted"));
        let closed = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        for addr in [refusing.addr, closed] {
            let pad = FakePad::new("refused");
            let mut cfg = pad.config();
            cfg.boost = Some(addr);
            cfg.boost_renew = Duration::from_millis(50);
            let sink = collect(&cfg);
            let rt = PadRuntime::start(cfg, sink.clone());
            wait_for("the pad", || rt.bitmap() == 0x0101);
            rt.set_grab(true);
            thread::sleep(Duration::from_millis(200));
            pad.report(&[(EV_KEY, BTN_NORTH, 1)], latency::mono_ns());
            wait_for("the press", || reports(&sink).len() == 1);
            assert_eq!(reports(&sink)[0].0.buttons, button::Y);
            rt.stop();
        }
        assert!(boost::boost(closed, 1).unwrap_err().contains("did not answer"));
    }

    fn holds(rt: &PadRuntime) -> bool {
        rt.inner.slots.lock().unwrap().iter().flatten().any(|s| s.grabbed.load(Ordering::Acquire))
    }

    /// The menu (LumaGame's UI) and the stream's engine each run a runtime on the same pad, as on the TV.
    #[test]
    fn the_menu_hands_the_grab_to_the_stream_and_never_takes_it_back_unasked() {
        let pad = FakePad::new("handoff");
        let cfg = pad.config();
        let menu = PadRuntime::start(cfg.clone(), collect(&cfg));
        let stream = PadRuntime::start(cfg.clone(), collect(&cfg));
        wait_for("both readers", || menu.bitmap() == 0x0101 && stream.bitmap() == 0x0101);
        menu.set_grab(true);
        assert!(holds(&menu));
        stream.set_grab(true);
        assert!(!holds(&stream), "the kernel refuses a second holder");
        menu.set_grab(false);
        assert!(!holds(&menu));
        wait_for("the stream's rescan to take the grab", || holds(&stream));
        stream.set_grab(false);
        let scans = menu.scans();
        wait_for("two rescans of the menu", || menu.scans() >= scans + 2);
        assert!(!holds(&menu) && !holds(&stream), "a released grab stays released");
        stream.set_grab(true);
        stream.stop();
        menu.set_grab(true);
        assert!(holds(&menu), "stop lets go of the grab before closing");
        menu.stop();
    }

    #[test]
    fn start_held_alone_is_one_guide_and_the_chord_still_wins() {
        let hold = Duration::from_secs(1);
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut long = Holds::default();
        assert_eq!(long.step(button::START, at(0), hold), None);
        assert_eq!(long.due(hold), Some(at(1000)));
        assert_eq!(long.step(button::START, at(999), hold), None);
        assert_eq!(long.step(button::START, at(1000), hold), Some(Held::Guide(true)));
        assert_eq!(long.step(button::START, at(5000), hold), None, "one press, however long it is held");
        assert_eq!(long.due(hold), None);
        assert_eq!(long.step(0, at(5100), hold), Some(Held::Guide(false)));
        assert_eq!(long.step(button::START, at(6000), hold), None);
        assert_eq!(long.step(0, at(6900), hold), None, "a shorter press says nothing, and owes no release");
        // Select joins Start: only the chord, and Start stays the chord's after Select lets go.
        assert_eq!(long.step(button::START, at(7000), hold), None);
        assert_eq!(long.step(button::START | button::BACK, at(7500), hold), None);
        assert_eq!(long.due(hold), Some(at(8500)));
        assert_eq!(long.step(button::START | button::BACK, at(8500), hold), Some(Held::Chord));
        assert_eq!(long.step(button::START, at(8600), hold), None);
        assert_eq!(long.step(button::START, at(12_000), hold), None, "the chord's Start never opens the menu");
        assert_eq!(long.step(0, at(12_100), hold), None);
        // Select first, then Start: the same chord.
        assert_eq!(long.step(button::BACK, at(13_000), hold), None);
        assert_eq!(long.step(button::BACK | button::START, at(13_100), hold), None);
        assert_eq!(long.step(button::BACK | button::START, at(14_100), hold), Some(Held::Chord));
    }

    #[test]
    fn a_hold_is_announced_while_the_pad_sits_still() {
        let pad = FakePad::new("hold");
        let cfg = PadConfig { chord_hold: Duration::from_millis(30), ..pad.config() };
        let sink = collect(&cfg);
        let rt = PadRuntime::start(cfg.clone(), sink.clone());
        wait_for("the pad", || rt.bitmap() == 0x0101);
        let guides = || sink.events.lock().unwrap().iter().filter_map(|e| if let PadEvent::Guide { pressed, .. } = e { Some(*pressed) } else { None }).collect::<Vec<_>>();
        pad.report(&[(EV_KEY, BTN_START, 1)], latency::mono_ns());
        wait_for("the Guide", || guides() == [true]);
        pad.report(&[(EV_KEY, BTN_START, 0)], latency::mono_ns());
        wait_for("its release", || guides() == [true, false]);
        pad.report(&[(EV_KEY, BTN_SELECT, 1), (EV_KEY, BTN_START, 1)], latency::mono_ns());
        wait_for("the chord", || sink.events.lock().unwrap().contains(&PadEvent::ExitChord { slot: 0 }));
        assert_eq!(guides(), [true, false]);
        rt.stop();
    }

    /// `{"pid":n,"tid":n}` without a JSON dependency: the pad crate has none.
    mod serde_like {
        pub struct Ids {
            pub pid: u32,
            pub tid: i32,
        }

        pub fn parse(body: &str) -> Ids {
            let num = |key: &str| -> i64 {
                let i = body.find(key).unwrap() + key.len();
                body[i..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap()
            };
            Ids { pid: num("\"pid\":") as u32, tid: num("\"tid\":") as i32 }
        }
    }
}
