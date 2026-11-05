//! Keys and the pointer from the set's input devices (C4). `lginput2` turns every remote (IR `LGE RCU`, the Magic
//! Remote `LGE M-RCU`, network keys as `Smart Remote RCU Input`, …) into evdev devices; the compositor reads them
//! all and routes each key by the focused window's key mask and access policies, keeping the compositor's own
//! keys (Home) for itself. The Magic Remote's pointer arrives as absolute axes (the census in
//! `docs/research/arch/input-devices-01.txt`: `ABS=3` on every `LGE M-RCU`, the only relative axis is the wheel);
//! a turn of the wheel shows the cursor (`remote.pointerShow` `wheel`, the default; `movement` is LG's: a motion shows it),
//! motion moves it while it shows, OK clicks while it shows, a navigation key or 10 s of stillness hides it. Reads are
//! allocation-free (one fixed buffer per device).

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::State;
use crate::buttons;
use crate::keys::{self, Action, Ctx};
use crate::server::seat;
use crate::window::key_mask;

/// Linux input event codes the routing knows by name (`linux/input-event-codes.h`).
pub mod code {
    pub const KEY_HOME: u32 = 102;
    pub const KEY_HOMEPAGE: u32 = 172;
    /// LG's `RF_HOME` (network-input-service's key table): the code PlainHome reads as the Magic Remote's Home and `perf/kb.py` injects
    /// as Home through the SSAP key device (`Smart Remote RCU Input`); the kernel headers do not name it.
    pub const KEY_RF_HOME: u32 = 773;
    pub const KEY_BACK: u32 = 158;
    /// The remote's Back arrives as KEY_PREVIOUS on this set (HANDOFF).
    pub const KEY_PREVIOUS: u32 = 412;
    pub const KEY_EXIT: u32 = 174;
    pub const KEY_UP: u32 = 103;
    pub const KEY_LEFT: u32 = 105;
    pub const KEY_RIGHT: u32 = 106;
    pub const KEY_DOWN: u32 = 108;
    pub const KEY_ENTER: u32 = 28;
    pub const KEY_OK: u32 = 352;
    pub const KEY_0: u32 = 11;
    pub const KEY_1: u32 = 2;
    pub const KEY_9: u32 = 10;
    pub const KEY_RED: u32 = 398;
    pub const KEY_GREEN: u32 = 399;
    pub const KEY_YELLOW: u32 = 400;
    pub const KEY_BLUE: u32 = 401;
    /// The remote's channel rocker and Info key (LG's table, `network-input-service`; the Info code is `KEY_INFO`).
    pub const KEY_CHANNELUP: u32 = 402;
    pub const KEY_CHANNELDOWN: u32 = 403;
    pub const KEY_INFO: u32 = 358;
    /// The remote's Guide key (`IR_KEY_GUIDE` in keymap.xkb, 370 - 8). Live TV answers it by launching LG's guide itself, 2 ms after the key
    /// (`com.webos.app.livetv` asks SAM to launch `com.webos.app.livemenu` with `activateType: tvguide`; read off the set 2026-11-04).
    pub const KEY_GUIDE: u32 = 362;
    pub const KEY_MUTE: u32 = 113;
    pub const KEY_VOLUMEDOWN: u32 = 114;
    pub const KEY_VOLUMEUP: u32 = 115;
    /// The remote's Power key (`KEY_POWER`). LG starts the standby below the compositor (`KeyEntryPoints.json`: no handler in its chain); the
    /// compositor only forwards the key to the focused window, first of all ([`State::power_key`]).
    pub const KEY_POWER: u32 = 116;
    /// The gear, `IR_KEY_MENU`: the gate's quick-settings step injects it, and LG's own key chain hands this very code to YouTube.
    pub const KEY_MENU: u32 = 139;
    /// The Input key (`TV_VIDEO`, `Key_webOS_InputSource`): read off this set's remote in round 8.
    pub const KEY_VIDEO_NEXT: u32 = 241;
    /// The Netflix and Prime Video buttons, from LG's network-input-service key table (`NETFLIX`, `AMAZON`); not yet read on this set.
    pub const KEY_NETFLIX: u32 = 1037;
    pub const KEY_AMAZON: u32 = 1038;
    /// The Magic Remote's CUSTOM_KEY_CURSOR_OFF (`LGE M-RCU`, key 1199, down and up in one report): lginput2 sends it when the remote is laid down and goes
    /// to sleep (ACTIVE, SLEEP_READY, SLEEP). It is the remote saying its pointer is gone, never the person's OK: round 19 caught the focused tile
    /// launching 3 to 9 ms after it, with no OK before it.
    pub const KEY_CURSOR_OFF: u32 = 1199;
    /// The pulse (1198, CUSTOM_KEY_CURSOR_ON, down and up) the same remote sends every 0.5 s while its pointer is active, to keep it alive: no key at all.
    pub const KEY_POINTER_ALIVE: u32 = 1198;
}

/// A Power key still down this long after its press is held; one pressed again after more is a new press (its release was lost).
const POWER_REPEAT_WINDOW: Duration = Duration::from_secs(2);


/// What a key means to the compositor before any client sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Delivered to the focused window.
    Client,
    /// The compositor's own: Home (launch the Home app), swallowed from clients.
    Home,
    /// Back when the focused window does not take it: the compositor closes or passes it.
    Back,
    /// Not in the focused window's mask and not the compositor's: dropped.
    Dropped,
    /// Volume and Mute: LG's key filters turn them into audio service calls and no app sees them
    /// (`KeyFilters/systemUi.js`, `KeyPolicy.Accepted`); the compositor does the same.
    System,
}

/// Routes `code` for a window with `mask` (`wl_webos_shell_surface.set_key_mask`) that `takes_back`
/// (`_WEBOS_ACCESS_POLICY_KEYS_BACK`) and `takes_home` (`_WEBOS_ACCESS_POLICY_KEYS_HOME`).
pub fn route(code: u32, mask: u32, takes_back: bool, takes_home: bool) -> Route {
    use code::*;
    match code {
        KEY_HOME | KEY_HOMEPAGE | KEY_RF_HOME => {
            if takes_home && mask & key_mask::HOME != 0 {
                Route::Client
            } else {
                Route::Home
            }
        }
        KEY_BACK | KEY_PREVIOUS => {
            if takes_back || mask & key_mask::BACK != 0 {
                Route::Client
            } else {
                Route::Back
            }
        }
        KEY_EXIT => {
            if mask & key_mask::EXIT != 0 {
                Route::Client
            } else {
                Route::Back
            }
        }
        KEY_UP | KEY_DOWN | KEY_LEFT | KEY_RIGHT | KEY_ENTER | KEY_OK => {
            if mask & key_mask::NAVIGATION != 0 {
                Route::Client
            } else {
                Route::Dropped
            }
        }
        KEY_1..=KEY_0 => {
            if mask & key_mask::NUMERIC != 0 {
                Route::Client
            } else {
                Route::Dropped
            }
        }
        KEY_RED | KEY_GREEN | KEY_YELLOW | KEY_BLUE => {
            if mask & key_mask::COLOUR != 0 {
                Route::Client
            } else {
                Route::Dropped
            }
        }
        KEY_VOLUMEUP | KEY_VOLUMEDOWN | KEY_MUTE => Route::System,
        _ => Route::Client,
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputEvent {
    tv_sec: libc::time_t,
    tv_usec: libc::suseconds_t,
    kind: u16,
    code: u16,
    value: i32,
}

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
/// The wheel of the Magic Remote (`REL=100` in the census is bit 8), and the high-resolution report some drivers add to it.
const REL_WHEEL: u16 = 8;
const REL_WHEEL_HI_RES: u16 = 11;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
/// Absolute axes a device can have (`ABS_CNT`), and the kernel's property bit of a motion sensor (`INPUT_PROP_ACCELEROMETER`).
const ABS_COUNT: usize = 0x40;
const INPUT_PROP_ACCELEROMETER: usize = 6;

/// What a device reports: one key, the pointer's new position in output pixels (once per `SYN_REPORT`), or a notch of the wheel (`down`: it was
/// turned towards the person, a scroll down; evdev reports a turn away as positive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Key { code: u32, pressed: bool },
    Motion { x: i32, y: i32 },
    Wheel { down: bool },
    /// A person touched a device whose events nobody routes (a mouse, a game controller, a touchpad) or moved a pointer axis that is not the
    /// Magic Remote's: it counts as input to the idle timers and the saver, and goes no further. At most one per read.
    Activity,
}

/// The range of one absolute axis (`EVIOCGABS`), mapped onto the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbsRange {
    pub x: (i32, i32),
    pub y: (i32, i32),
}

impl AbsRange {
    /// `value` on `axis` as an output coordinate: the axis range maps onto `0..=size-1`; a range of one value
    /// (a device that never set one) takes the value as a pixel.
    pub fn scale(axis: (i32, i32), value: i32, size: i32) -> i32 {
        let (min, max) = axis;
        let v = if max > min { ((value - min) as i64 * (size - 1) as i64 / (max - min) as i64) as i32 } else { value };
        v.clamp(0, size - 1)
    }
}

pub struct Device {
    pub path: String,
    pub name: String,
    /// `Smart Remote RCU Input`: keys from the network (SSAP), told apart from the physical remotes (PLAN, AI access).
    pub from_network: bool,
    /// Pointer axes, when the device has `ABS_X` and `ABS_Y` (the Magic Remotes); their events are dropped otherwise.
    pub abs: Option<AbsRange>,
    /// Whether a turn of this device's wheel shows the pointer: a Magic Remote (see [`Device::turns_pointer`]).
    wheel: bool,
    /// A device nobody routes (mouse, game controller, touchpad): its events only count as activity, and it has no seat.
    watch: bool,
    abs_activity: AbsActivity,
    file: File,
    buf: [InputEvent; 64],
    /// The position the pending `SYN_REPORT` will report, and the last one reported.
    pending: Option<(i32, i32)>,
    last: (i32, i32),
    /// Whether the kernel stamps this device's events with CLOCK_MONOTONIC (`EVIOCSCLOCKID`), and the stamp of the first event of the last read, ns:
    /// how long the loop took to wake for a key (`wake`).
    monotonic: bool,
    stamp_ns: Option<u64>,
}

impl Device {
    pub fn open(path: &str) -> io::Result<Device> {
        let file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(path)?;
        let mut name = [0u8; 128];
        // SAFETY: EVIOCGNAME(len) writes at most len bytes into the buffer.
        let n = unsafe { libc::ioctl(file.as_raw_fd(), eviocgname(name.len()), name.as_mut_ptr()) };
        let name = if n > 0 { String::from_utf8_lossy(&name[..(n as usize).min(name.len())]).trim_end_matches('\0').to_string() } else { String::new() };
        let from_network = name.contains("Smart Remote RCU Input");
        // EVIOCGABS fails on a device without absolute axes (the IR remote); the Magic Remotes and the network
        // key device answer for both.
        let abs = match (absinfo(file.as_raw_fd(), ABS_X), absinfo(file.as_raw_fd(), ABS_Y)) {
            (Some(x), Some(y)) => Some(AbsRange { x, y }),
            _ => None,
        };
        // The default clock of an event is CLOCK_REALTIME; the loop's own are monotonic, and a latency needs both on one.
        let clock: libc::c_int = libc::CLOCK_MONOTONIC;
        // SAFETY: EVIOCSCLOCKID reads one int; the device keeps the choice for this descriptor only.
        let monotonic = unsafe { libc::ioctl(file.as_raw_fd(), EVIOCSCLOCKID, &clock) } == 0;
        Ok(Device {
            path: path.to_string(),
            wheel: Self::turns_pointer(&name, abs.is_some()),
            name,
            from_network,
            abs,
            watch: false,
            abs_activity: AbsActivity::default(),
            file,
            buf: [InputEvent::default(); 64],
            pending: None,
            last: (-1, -1),
            monotonic,
            stamp_ns: None,
        })
    }

    /// A device whose wheel shows the pointer: one with pointer axes that is a remote in the hand (`LGE M-RCU`, `LGE TONE+`, `LGE W-RCU`,
    /// `LGE Simple Premium`). The virtual devices of the network input service (`Smart Remote RCU Input`, `LGE Network Input`) have a wheel
    /// and the axes too, and a phone's scroll is not the Magic Remote's wheel.
    pub fn turns_pointer(name: &str, has_abs: bool) -> bool {
        has_abs && !name.contains("Smart Remote") && !name.contains("Network Input")
    }

    /// Whether this is a remote or a keyboard: a device with a key below `BTN_MISC`. A game controller's buttons start at `BTN_SOUTH`
    /// (0x130) and its touchpad and motion sensors have no keys; its sticks and gyro would move the pointer, so it is the game's alone: it is only
    /// watched for activity ([`Device::can_watch`]). A node that cannot say (a file standing in for a device) is read.
    fn takes_keys(&self) -> bool {
        let mut bits = [0u8; KEY_BITMAP_BYTES];
        // SAFETY: EVIOCGBIT(EV_KEY, len) writes at most len bytes into the buffer.
        let n = unsafe { libc::ioctl(self.file.as_raw_fd(), eviocgbit_key(bits.len()), bits.as_mut_ptr()) };
        n < 0 || has_keys(&bits[..(n as usize).min(bits.len())])
    }

    /// Whether a device without keys is read for activity: any but a motion sensor, whose resting noise would never let the TV idle. A node
    /// that cannot say is read, as for [`Device::takes_keys`].
    fn can_watch(&self) -> bool {
        let mut props = [0u8; 4];
        // SAFETY: EVIOCGPROP(len) writes at most len bytes into the buffer.
        let n = unsafe { libc::ioctl(self.file.as_raw_fd(), eviocgprop(props.len()), props.as_mut_ptr()) };
        let sensor = n >= 0 && props[INPUT_PROP_ACCELEROMETER / 8] & (1 << (INPUT_PROP_ACCELEROMETER % 8)) != 0;
        !sensor && !self.name.contains("Motion Sensors")
    }

    pub fn fd(&self) -> i32 {
        self.file.as_raw_fd()
    }

    /// The kernel's stamp (CLOCK_MONOTONIC ns) of the first event of the last read, once; `None` when the device could not be set to a
    /// monotonic clock or nothing was read since.
    pub fn take_stamp_ns(&mut self) -> Option<u64> {
        self.stamp_ns.take()
    }

    /// Reads what is pending; `f` per key event (repeats are reported as presses) and per pointer motion. An error, or the end of the
    /// file, means the device is gone (a removed evdev node answers `ENODEV` and polls readable for good): the caller stops reading it.
    pub fn drain(&mut self, mut f: impl FnMut(Event)) -> io::Result<()> {
        let mut active = false;
        loop {
            let size = std::mem::size_of::<InputEvent>();
            // SAFETY: the buffer is a plain array of repr(C) structs; read fills whole events.
            let bytes = unsafe { std::slice::from_raw_parts_mut(self.buf.as_mut_ptr() as *mut u8, size * self.buf.len()) };
            let n = match self.file.read(bytes) {
                Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            for i in 0..n / size {
                let ev = self.buf[i];
                if self.monotonic && self.stamp_ns.is_none() && ev.tv_sec != 0 {
                    self.stamp_ns = Some(ev.tv_sec as u64 * 1_000_000_000 + ev.tv_usec as u64 * 1000);
                }
                if self.watch {
                    let fd = self.file.as_raw_fd();
                    let touched = match ev.kind {
                        EV_KEY | EV_REL => ev.value != 0,
                        EV_ABS => self.abs_activity.moved(ev.code, ev.value, || absinfo(fd, ev.code)),
                        _ => false,
                    };
                    if touched && !active {
                        active = true;
                        f(Event::Activity);
                    }
                    continue;
                }
                match ev.kind {
                    EV_KEY => f(Event::Key { code: ev.code as u32, pressed: ev.value != 0 }),
                    EV_REL if self.wheel && matches!(ev.code, REL_WHEEL | REL_WHEEL_HI_RES) && ev.value != 0 => f(Event::Wheel { down: ev.value < 0 }),
                    // The phone remote's touchpad and scroll (the network devices) and any other relative axis: a person is there.
                    EV_REL if ev.value != 0 && !active => {
                        active = true;
                        f(Event::Activity);
                    }
                    EV_ABS => {
                        if let Some(abs) = self.abs {
                            let (mut x, mut y) = self.pending.unwrap_or(self.last);
                            match ev.code {
                                ABS_X => x = AbsRange::scale(abs.x, ev.value, crate::OUTPUT_WIDTH as i32),
                                ABS_Y => y = AbsRange::scale(abs.y, ev.value, crate::OUTPUT_HEIGHT as i32),
                                _ => continue,
                            }
                            self.pending = Some((x, y));
                        }
                    }
                    EV_SYN => {
                        if let Some((x, y)) = self.pending.take()
                            && (x, y) != self.last
                        {
                            // Both axes are known once the first report carried both; a report with one axis
                            // keeps the other where it was.
                            self.last = (x.max(0), y.max(0));
                            f(Event::Motion { x: self.last.0, y: self.last.1 });
                        }
                    }
                    _ => {}
                }
            }
            if n < size * self.buf.len() {
                return Ok(());
            }
        }
    }
}

/// Which absolute-axis reports of a device nobody routes are a person's: a game controller's resting sticks and triggers jitter by a few
/// units, so an axis counts only when it has moved a quarter of its range from where it last counted (a hat switch, whose range is 2, on
/// any change). The first report of an axis only sets where it stands.
#[derive(Default)]
struct AbsActivity {
    axes: Vec<Option<(i32, i32)>>,
}

impl AbsActivity {
    fn moved(&mut self, axis: u16, value: i32, range: impl FnOnce() -> Option<(i32, i32)>) -> bool {
        let axis = axis as usize;
        if axis >= ABS_COUNT {
            return false;
        }
        if self.axes.is_empty() {
            self.axes = vec![None; ABS_COUNT];
        }
        match &mut self.axes[axis] {
            slot @ None => {
                *slot = Some((value, range().map_or(0, |(lo, hi)| (hi.saturating_sub(lo)) / 4)));
                false
            }
            Some((anchor, threshold)) if (i64::from(value) - i64::from(*anchor)).abs() > i64::from(*threshold) => {
                *anchor = value;
                true
            }
            Some(_) => false,
        }
    }
}

/// `EVIOCSCLOCKID` (`_IOW('E', 0xa0, int)`): the clock the kernel stamps this descriptor's events with.
const EVIOCSCLOCKID: libc::c_ulong = 0x4004_45a0;

const fn eviocgname(len: usize) -> libc::c_ulong {
    // _IOC(_IOC_READ, 'E', 0x06, len)
    ((2u64 << 30) | ((len as u64) << 16) | (b'E' as u64) << 8 | 0x06) as libc::c_ulong
}

/// `EVIOCGPROP(len)`: the device's property bits (`_IOC(_IOC_READ, 'E', 0x09, len)`).
const fn eviocgprop(len: usize) -> libc::c_ulong {
    ((2u64 << 30) | ((len as u64) << 16) | (b'E' as u64) << 8 | 0x09) as libc::c_ulong
}

/// `EVIOCGABS(axis)`: `(minimum, maximum)` of one absolute axis, `None` when the device has no such axis.
fn absinfo(fd: i32, axis: u16) -> Option<(i32, i32)> {
    // struct input_absinfo { value, minimum, maximum, fuzz, flat, resolution }: six i32.
    let mut info = [0i32; 6];
    let req = ((2u64 << 30) | ((std::mem::size_of_val(&info) as u64) << 16) | (b'E' as u64) << 8 | (0x40 + axis as u64)) as libc::c_ulong;
    // SAFETY: the kernel writes exactly the struct into the buffer.
    let r = unsafe { libc::ioctl(fd, req, info.as_mut_ptr()) };
    if r < 0 { None } else { Some((info[1], info[2])) }
}

/// Bytes of a key bitmap (`KEY_CNT` bits).
const KEY_BITMAP_BYTES: usize = 96;

/// `EVIOCGBIT(EV_KEY, len)` (`_IOC(_IOC_READ, 'E', 0x20 + EV_KEY, len)`): which keys and buttons a device has.
const fn eviocgbit_key(len: usize) -> libc::c_ulong {
    ((2u64 << 30) | ((len as u64) << 16) | (b'E' as u64) << 8 | (0x20 + EV_KEY as u64)) as libc::c_ulong
}

/// Whether a key bitmap (`EVIOCGBIT`) has a key below `BTN_MISC` (0x100), `KEY_RESERVED` aside.
fn has_keys(bitmap: &[u8]) -> bool {
    bitmap.iter().take(0x100 / 8).enumerate().any(|(i, &b)| if i == 0 { b & 0xfe != 0 } else { b != 0 })
}

/// How long the directory goes unread when nothing says it changed.
const RESCAN_NS: u64 = 2_000_000_000;

/// The kernel's list of input devices: `eventN` is a link in it only while a device is registered behind that node.
const CLASS_DIR: &str = "/sys/class/input";

struct Slot {
    device: Option<Device>,
    seat: seat::SeatId,
    /// The device's name, which one made anew finds its slot and seat by.
    name: String,
}

/// What one [`Inputs::scan`] did.
#[derive(Default)]
pub struct Scan {
    /// The slots a device was opened into, with the descriptor to wait on.
    pub opened: Vec<(usize, i32)>,
    /// One line per device opened.
    pub lines: Vec<String>,
    /// A node that did not open or is no remote or keyboard, told once for as long as it stays that way.
    pub problems: Vec<String>,
}

/// What one look at a node found.
pub enum Probe {
    /// A remote or a keyboard, open.
    Keys(Box<Device>),
    /// A device with no keys that a person can still touch (a mouse, a game controller, a touchpad), open for activity only: no seat, no keys.
    Watch(Box<Device>),
    /// A device with no keys (a game controller's touchpad and sensors, the Magic Remote's touchpad), by name.
    NoKeys(String),
    /// A node that did not open.
    Failed(io::Error),
}

/// Looks at one node: opens it and keeps it if it has keys (a remote or keyboard) or can be touched (watched for activity only). A motion sensor
/// is closed again on the spot, and that close waits for a grace period of the kernel's RCU (`evdev_release`, about 20 ms of the loop); every
/// open of a node with no device behind it runs `modprobe` for its numbers (`request_module`, 7 ms and more). [`Inputs::scan`] therefore looks
/// at a node once, not at every look.
fn probe(path: &str) -> Probe {
    match Device::open(path) {
        Ok(d) if d.takes_keys() => Probe::Keys(Box::new(d)),
        Ok(mut d) if d.can_watch() => {
            d.watch = true;
            Probe::Watch(Box::new(d))
        }
        Ok(d) => Probe::NoKeys(d.name.clone()),
        Err(e) => Probe::Failed(e),
    }
}

/// What a node was when it was looked at: the node itself, and the registration of the device behind it (the kernel's link for it names a
/// new input number whenever a device is made anew). A node that is still the same is not looked at again.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    /// Inode, device numbers and change time of the node.
    node: Option<(u64, u64, i64, i64)>,
    registration: Option<PathBuf>,
}

/// A node's identity; `None` when the kernel's list (`class`, when there is one) says that no device is behind it: LG's init makes
/// `event14` to `event31` whether or not anything is registered there, and a device that is registered later takes one of them over
/// without the node being made.
fn identity(path: &Path, class: Option<&Path>) -> Option<Identity> {
    use std::os::unix::fs::MetadataExt;
    let registration = match class {
        None => None,
        Some(class) => match std::fs::read_link(class.join(path.file_name()?)) {
            Ok(link) => Some(link),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
            Err(_) => None,
        },
    };
    let node = std::fs::metadata(path).ok().map(|m| (m.ino(), m.rdev(), m.ctime(), m.ctime_nsec()));
    Some(Identity { node, registration })
}

/// Where the compositor reads the set's input from: the `event*` nodes of one directory (`/dev/input` on the set), and the seat each
/// device's keys go out on (one wl_seat per device, as LG's compositor has them).
///
/// The directory is read when the loop starts and again when it changes ([`Inputs::watch`]), with a look every two seconds as the
/// back-stop. lginput2 makes its virtual devices (`Smart Remote RCU Input`, where the keys of Home Assistant and the phone apps arrive,
/// and `LGE Network Input`) anew at every wake from standby, long after the compositor started, and a Bluetooth remote or keyboard
/// appears when it is paired. A compositor that read only what it found at its start lost them for good: after the first wake its
/// network keys went nowhere, the idle timer ran out and the saver came up under a gate that thought it was pressing keys.
pub struct Inputs {
    dir: PathBuf,
    /// The kernel's list of the devices behind the nodes (`/sys/class/input`), where `dir` is the set's own `/dev/input`.
    class: Option<PathBuf>,
    slots: Vec<Slot>,
    /// What was said of a node that is left alone, by path, so that a node which stays that way is told once.
    told: HashMap<String, String>,
    /// The nodes that were looked at and not kept, by path, as they were then: a node that is still the same is not looked at again.
    judged: HashMap<String, Identity>,
    watch: Option<OwnedFd>,
    /// When the directory is read next (CLOCK_MONOTONIC ns); never before the first scan.
    next_ns: u64,
}

impl Inputs {
    pub fn new(dir: &Path) -> Inputs {
        // The set's own /dev/input has the kernel's list beside it; a directory standing in for it has not.
        Inputs::listed_by(dir, (dir == Path::new("/dev/input")).then(|| PathBuf::from(CLASS_DIR)))
    }

    /// [`Inputs::new`] with the kernel's list of the devices given (`None`: every node is looked at).
    pub fn listed_by(dir: &Path, class: Option<PathBuf>) -> Inputs {
        Inputs { dir: dir.to_path_buf(), class, slots: Vec::new(), told: HashMap::new(), judged: HashMap::new(), watch: None, next_ns: u64::MAX }
    }

    /// Watches the directory for nodes made or changed; the descriptor is for the loop's epoll. `None` when the kernel refuses: the
    /// two-second look is all there is then.
    pub fn watch(&mut self) -> Option<i32> {
        // SAFETY: plain system calls; the descriptor is owned from here on.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            log::warn!("input: inotify: {}; new devices are found within {} s", io::Error::last_os_error(), RESCAN_NS / 1_000_000_000);
            return None;
        }
        // SAFETY: `fd` is a descriptor this call just made.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let dir = std::ffi::CString::new(self.dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: `dir` is a NUL-terminated path that lives through the call.
        let added = unsafe { libc::inotify_add_watch(fd.as_raw_fd(), dir.as_ptr(), libc::IN_CREATE | libc::IN_MOVED_TO | libc::IN_ATTRIB) };
        if added < 0 {
            log::warn!("input: watching {}: {}; new devices are found within {} s", self.dir.display(), io::Error::last_os_error(), RESCAN_NS / 1_000_000_000);
            return None;
        }
        let raw = fd.as_raw_fd();
        self.watch = Some(fd);
        Some(raw)
    }

    /// The watch has something to say: what it holds is thrown away (the directory is read whole), and the next look is due now.
    pub fn watched(&mut self, now_ns: u64) {
        if let Some(fd) = &self.watch {
            let mut buf = [0u8; 1024];
            // SAFETY: reads into a local buffer on a non-blocking descriptor.
            while unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
        }
        self.next_ns = now_ns;
    }

    /// Whether the directory is to be read now.
    pub fn due(&self, now_ns: u64) -> bool {
        now_ns >= self.next_ns
    }

    /// Opens every remote or keyboard node of the directory that no open device has. A device made anew goes back into the slot, and onto
    /// the seat, of the one that went (the clients that bound that seat hear its keys again); any other gets a slot and a seat from
    /// `new_seat`.
    ///
    /// A node is looked at when it is new or has changed, not at every look. The first version of the look opened every node that was not
    /// open at each of them, every two seconds: the 18 nodes with no device behind them each ran `modprobe` and the 4 of a game controller and
    /// a touchpad each waited for an RCU grace period when they were closed again, 115 to 178 ms of the loop at every look (2026-11-04).
    pub fn scan(&mut self, now_ns: u64, new_seat: &mut dyn FnMut(&str) -> seat::SeatId) -> Scan {
        self.scan_with(now_ns, new_seat, &mut probe)
    }

    /// [`Inputs::scan`] with the look at a node given (the host tests count them).
    pub fn scan_with(&mut self, now_ns: u64, new_seat: &mut dyn FnMut(&str) -> seat::SeatId, look: &mut dyn FnMut(&str) -> Probe) -> Scan {
        self.next_ns = now_ns.saturating_add(RESCAN_NS);
        let mut out = Scan::default();
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return out };
        let mut paths: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().as_bytes().starts_with(b"event"))
            .map(|e| e.path().to_string_lossy().into_owned())
            .collect();
        paths.sort();
        self.told.retain(|p, _| paths.contains(p));
        self.judged.retain(|p, _| paths.contains(p));
        // Without the kernel's list (no `/sys`), every node is looked at once.
        let class = self.class.clone().filter(|c| c.is_dir());
        for path in paths {
            if self.slots.iter().any(|s| s.device.as_ref().is_some_and(|d| d.path == path)) {
                continue;
            }
            let Some(now) = identity(Path::new(&path), class.as_deref()) else {
                self.judged.remove(&path);
                continue;
            };
            if self.judged.get(&path) == Some(&now) {
                continue;
            }
            let problem = match look(&path) {
                Probe::Keys(d) => {
                    self.told.remove(&path);
                    self.judged.remove(&path);
                    self.adopt(*d, new_seat, &mut out);
                    continue;
                }
                Probe::Watch(d) => {
                    self.told.remove(&path);
                    self.judged.remove(&path);
                    self.adopt(*d, new_seat, &mut out);
                    continue;
                }
                Probe::NoKeys(name) => {
                    self.judged.insert(path.clone(), now);
                    format!("input: {path} {name:?} has no keys and cannot be touched (a motion sensor): left alone")
                }
                Probe::Failed(e) => {
                    // "No such device" stays so until the node changes; any other refusal (permissions, memory) is tried again at the next look.
                    if matches!(e.raw_os_error(), Some(libc::ENXIO | libc::ENODEV)) {
                        self.judged.insert(path.clone(), now);
                    }
                    format!("input: {path}: {e}")
                }
            };
            if self.told.insert(path, problem.clone()).as_ref() != Some(&problem) {
                out.problems.push(problem);
            }
        }
        out
    }

    fn adopt(&mut self, d: Device, new_seat: &mut dyn FnMut(&str) -> seat::SeatId, out: &mut Scan) {
        out.lines.push(format!(
            "input: {} {:?}{}{}{}",
            d.path,
            d.name,
            if d.from_network { " (network keys)" } else { "" },
            if d.watch { " (activity only)" } else { "" },
            d.abs.filter(|_| !d.watch).map(|a| format!(" pointer x {:?} y {:?}", a.x, a.y)).unwrap_or_default()
        ));
        let fd = d.fd();
        let i = match self.slots.iter().position(|s| s.device.is_none() && s.name == d.name) {
            Some(i) => i,
            None => {
                // A device that is only watched has no keys to route, so no seat for clients to bind: the shared first one stands in.
                let seat = if d.watch { seat::SeatId(0) } else { new_seat(&d.name) };
                self.slots.push(Slot { device: None, seat, name: d.name.clone() });
                self.slots.len() - 1
            }
        };
        self.slots[i].device = Some(d);
        out.opened.push((i, fd));
    }

    /// Whether slot `i` is a device that is only watched for activity.
    pub fn is_watch(&self, i: usize) -> bool {
        self.slots.get(i).and_then(|s| s.device.as_ref()).is_some_and(|d| d.watch)
    }

    /// The device in slot `i`, while it is open.
    pub fn device(&mut self, i: usize) -> Option<&mut Device> {
        self.slots.get_mut(i)?.device.as_mut()
    }

    /// The seat the keys of slot `i` go out on.
    pub fn seat(&self, i: usize) -> seat::SeatId {
        self.slots[i].seat
    }

    /// The device in slot `i` is gone (it polls hung up for good, or every read fails): out of the loop. Its slot and seat wait for the
    /// device that is made in its place.
    pub fn lose(&mut self, i: usize) -> Option<Device> {
        self.slots.get_mut(i)?.device.take()
    }
}

/// The compositor's key state: the Home hold, the remote's own keys (`keys.rs`) and what was delivered.
#[derive(Default)]
pub struct Keys {
    pub delivered: u64,
    pub dropped: u64,
    pub router: keys::Router,
    /// What the owner made of the remote's buttons (`remote.buttons`), re-read when the theme file changes.
    pub buttons: buttons::ButtonMap,
    /// Remapped or consumed buttons, including a saver wake, retained through repeats and release.
    mapped: Vec<(u32, Option<u32>)>,
    /// What the gear does when held (`remote.gearHold`), read with the app buttons.
    pub gear_hold: keys::HoldAction,
    /// What shows the Magic Remote's pointer (`remote.pointerShow`), read with the app buttons.
    pub pointer_show: seat::PointerShow,
    theme: ThemeFile,
    /// Codes in LG's own range that the key table has no name for, once each: a button of this remote it lacks.
    pub unnamed: Vec<u32>,
    /// The last fresh press of the Power key, and whether it is still down: what the standby's lines in the journal are measured from
    /// ([`State::since_power_key`]).
    power_at: Option<Instant>,
    power_held: bool,
}

/// The theme file the app buttons are read from, and what it was when last read.
#[derive(Default)]
struct ThemeFile {
    path: std::path::PathBuf,
    stamp: Option<(std::time::SystemTime, u64)>,
    /// When the tick last looked at the file ([`Keys::poll_theme`]).
    polled: Option<Instant>,
}

impl Keys {
    /// The keys of a compositor whose data directory is `config.data_dir`: the codes taught in `compositor/keys.conf`, and the theme.
    pub fn new(config: &crate::config::Config) -> Keys {
        let conf = config.data_dir.join("compositor/keys.conf");
        let table = match std::fs::read_to_string(&conf) {
            Ok(text) => {
                let (table, bad) = keys::KeyTable::parse(&text);
                for line in bad {
                    log::warn!("{}: {line}", conf.display());
                }
                table
            }
            Err(_) => keys::KeyTable::default(),
        };
        Keys {
            router: keys::Router::new(table),
            theme: ThemeFile { path: config.data_dir.join("theme/theme.json"), stamp: None, polled: None },
            ..Keys::default()
        }
    }

    /// Reads the theme again when the file is not the one last read. A file that is gone is no choice at all (LG's own targets, the
    /// pointer shown by the wheel); one that cannot be read or parsed now (the theme is written in place, not atomically) leaves what was
    /// read and is tried again at the next press.
    pub fn refresh_theme(&mut self) {
        let stamp = std::fs::metadata(&self.theme.path).ok().map(|m| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len()));
        if stamp == self.theme.stamp {
            return;
        }
        if stamp.is_none() {
            self.buttons = buttons::ButtonMap::default();
            self.gear_hold = keys::HoldAction::default();
            self.pointer_show = seat::PointerShow::default();
            self.theme.stamp = None;
        } else if let Some(theme) = std::fs::read_to_string(&self.theme.path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
            self.buttons = buttons::ButtonMap::from_theme(&theme);
            self.gear_hold = keys::HoldAction::from_theme(&theme);
            self.pointer_show = seat::PointerShow::from_theme(&theme);
            self.theme.stamp = stamp;
        }
    }

    /// [`Keys::refresh_theme`] at most once a second, from the tick: a change of what shows the pointer reaches the next device event
    /// without a stat per motion.
    pub fn poll_theme(&mut self, now: Instant) {
        if self.theme.polled.is_some_and(|t| now.saturating_duration_since(t) < Duration::from_secs(1)) {
            return;
        }
        self.theme.polled = Some(now);
        self.refresh_theme();
    }
}

impl State {
    /// One key from a device, delivered on `seat` (the device's own; seat 0 for the control socket and the tests).
    pub fn key_event(&mut self, seat: seat::SeatId, code: u32, pressed: bool, from_network: bool) {
        self.key_event_at(seat, code, pressed, from_network, Instant::now());
    }

    /// [`State::key_event`] at a given time, which is what the holds are measured against (the host tests move it by hand).
    pub fn key_event_at(&mut self, seat: seat::SeatId, code: u32, pressed: bool, from_network: bool, now: Instant) {
        // The Magic Remote's cursor-on pulse (1198, every 0.5 s while its pointer is active) is no key and no input of the person's: under
        // `remote.pointerShow` `movement` it shows the pointer (LG's behaviour), otherwise nothing.
        // Its cursor-off report (1199, sent as the remote is laid down) is the pointer going away: the cursor is hidden and no client hears a key.
        match code {
            code::KEY_POINTER_ALIVE => {
                if pressed {
                    seat::cursor_on(self);
                }
                return;
            }
            code::KEY_CURSOR_OFF => {
                if pressed {
                    seat::hide_cursor(self);
                }
                return;
            }
            _ => {}
        }
        // Power comes before everything that could hold it up or take it: the owner's button choices, the routing, the shell's layers and
        // the screen's care. The standby itself is LG's, below the compositor.
        if code == code::KEY_POWER {
            self.power_key(seat, pressed, now);
            return;
        }
        // Told over LG's Input Manager queue that the pointer shows (`im_queue`), lginput2 sends the wheel's press as the mouse button, BTN_LEFT, in
        // place of KEY_ENTER (entries 0 and 185 of its key table). It is the same OK: a click while the pointer shows, the key when it hid first.
        let code = if code == seat::BTN_LEFT { code::KEY_ENTER } else { code };
        let saver_up = self.lg_saver_engaged();
        self.care_input();
        if saver_up {
            if pressed {
                if !self.keys.mapped.iter().any(|(c, _)| *c == code) {
                    self.keys.mapped.push((code, None));
                }
            } else if let Some(i) = self.keys.mapped.iter().position(|(c, _)| *c == code) {
                self.keys.mapped.swap_remove(i);
            }
            return;
        }
        // The owner's choices for the remote's buttons come before anything else sees the key.
        let Some(code) = self.remap_button(code, pressed) else {
            self.seat.last_input = "key";
            return;
        };
        // While the Magic Remote's cursor shows, OK is a click on what it points at (LG's behaviour); an arrow or
        // Back hides the cursor and goes on as a key. A release goes the way its press went: a press taken as a key (by the Rust shell, or by an app's
        // keyboard) has its release as a key even if the cursor showed in between, and one taken as a click has its release as one.
        let key_press_open = !pressed && (self.shell_rust.took.iter().any(|(c, _, _)| *c == code) || self.seat.pressed.iter().any(|(_, c)| *c == code));
        if matches!(code, code::KEY_ENTER | code::KEY_OK) && !key_press_open && self.seat.cursor_visible && (self.seat.pointer_focus.is_some() || self.shell_rust.pointer_in) {
            if seat::pointer_button(self, pressed) {
                self.keys.delivered += 1;
                return;
            }
        } else if pressed && matches!(code, code::KEY_UP | code::KEY_DOWN | code::KEY_LEFT | code::KEY_RIGHT | code::KEY_BACK | code::KEY_PREVIOUS) {
            seat::hide_cursor(self);
        }
        self.note_unnamed_key(code, pressed);
        let foreground = self.foreground_app();
        let keyboard_up = self.keyboard_up();
        let picker_ready = self.shell_can("picker");
        let logical = self.keys.router.table.classify(code);
        let first_press = pressed && !self.keys.router.is_down(code);
        let out = {
            let router = &mut self.keys.router;
            router.event(code, pressed, now, &Ctx { foreground: &foreground, keyboard_up, picker_ready })
        };
        self.run_actions(out.actions);
        // The gear is down: its tap opens the quick panel at the release, and the shell puts the panel in the scene now, off the
        // screen, so the slide of the opening starts in a scene that has it (LG's filter chain gave the same cue).
        if first_press && logical == keys::Logical::Gear {
            self.shell_show("quick", true);
        }
        // The Rust shell's layer: every key but the compositor's own while it wants them, and its cards' OK and Back (`shell_rust.rs`).
        if !matches!(code, code::KEY_HOME | code::KEY_HOMEPAGE | code::KEY_RF_HOME | code::KEY_VOLUMEUP | code::KEY_VOLUMEDOWN | code::KEY_MUTE)
            && self.shell_rust_takes_key(seat, code, pressed, out.swallow, now)
        {
            self.seat.last_input = "key";
            return;
        }
        if out.swallow {
            self.seat.last_input = "key";
            return;
        }
        let focused = self.seat.focus.and_then(|s| self.scene.window_of(s)).and_then(|w| self.scene.window(w));
        let (mask, back, home) = focused
            .map(|w| (w.key_mask, w.access_policy("_WEBOS_ACCESS_POLICY_KEYS_BACK"), w.access_policy("_WEBOS_ACCESS_POLICY_KEYS_HOME")))
            .unwrap_or((0, false, false));
        let r = route(code, mask, back, home);
        if matches!(code, code::KEY_HOME | code::KEY_HOMEPAGE | code::KEY_RF_HOME) && !pressed && r == Route::Home {
            self.home_tap(now, from_network, self.shell_hears_home_key());
        }
        // An input method that holds the keyboard grab (the on-screen keyboard) is given every key the app would have got, Back
        // and the arrows included; Home, Volume and Mute stay the compositor's. The shell's own keyboard takes the arrows, OK, Back and
        // the digits only (a hardware keyboard's letters are the app's). A release of a key the app has the press of (the
        // OK that opened the keyboard) goes to the app, so that the key does not stay down in its state.
        let grabbing = self.ime.grabs(code);
        let app_holds = !pressed && self.seat.pressed.contains(&(seat, code)) && !self.ime.grab_down.contains(&code);
        match r {
            Route::Client | Route::Dropped | Route::Back if grabbing && !app_holds => self.grab_key(code, pressed),
            Route::Client => {
                // The shell's overlay stands in for what the app draws after this key (`mask.rs`): the claimed region of its window is blanked
                // from the next plan, before the app's own banner is drawn there.
                if pressed && self.scene.mask_listens(code) {
                    let app = self.seat.focus.and_then(|s| self.scene.window_of(s)).and_then(|w| self.scene.window(w)).map(|w| w.app_id().to_string());
                    if let Some(app) = app {
                        self.scene.mask_key(&app, code);
                    }
                }
                if seat::send_key(self, seat, code, pressed) {
                    self.keys.delivered += 1;
                } else {
                    self.keys.dropped += 1;
                }
            }
            Route::Home => {}
            Route::Back => {
                if pressed {
                    self.back_pressed();
                }
            }
            Route::Dropped => self.keys.dropped += 1,
            Route::System => self.system_key(code, pressed),
        }
        self.seat.last_input = "key";
    }

    /// The Power key: given to the focused window now, press, repeats and release alike, then noted. It is not the owner's to remap (`buttons::protected`)
    /// and no layer of the shell takes it, so a press can never be left waiting on the release of an earlier one. The care of the screen (the
    /// panel's brightness put back, the saver's idle clock) hears of it after the key has gone.
    fn power_key(&mut self, seat: seat::SeatId, pressed: bool, now: Instant) {
        // With LG's saver up the press is for standby: it is not given to the saver's app, which would answer a key by waking the screen, and
        // it does not wake the screen either (the saver's wake is for every other key).
        let saver = self.lg_saver_present();
        if !saver {
            if seat::send_key(self, seat, code::KEY_POWER, pressed) {
                self.keys.delivered += 1;
            } else {
                self.keys.dropped += 1;
            }
        }
        if pressed {
            // A press whose release was lost (the set went to standby under it) must not make the next one look like a repeat.
            let repeat = self.keys.power_held && self.keys.power_at.is_some_and(|t| now.saturating_duration_since(t) < POWER_REPEAT_WINDOW);
            if !repeat {
                let before = self.keys.power_at.map(|t| now.saturating_duration_since(t).as_millis());
                self.keys.power_at = Some(now);
                self.keys.power_held = true;
                self.journal(match before {
                    Some(ms) => format!("input: Power pressed, {ms} ms after the one before"),
                    None => "input: Power pressed".into(),
                });
                // The replacement compositor owns both directions of the remote-key handshake. Without an explicit request,
                // Active hits OFF_BY_KEYTIMEOUT after 8 s, while POWER OFF receives the wake key but stays in active standby.
                match self.luna_state.power.key_request() {
                    Some("powerOff") => {
                        self.luna_call("luna://com.webos.service.tvpower/power/powerOff".into(), serde_json::json!({"reason": "remoteKey"}));
                        self.journal(if saver { "tvpower: power/powerOff (remoteKey) asked, LG's saver up".into() } else { "tvpower: power/powerOff (remoteKey) asked".into() });
                    }
                    Some(_) => {
                        self.luna_call("luna://com.webos.service.tvpower/power/powerOn".into(), serde_json::json!({"reason": "remoteKey"}));
                        self.journal("tvpower: power/powerOn (remoteKey) asked".into());
                    }
                    None => {}
                }
            }
        } else {
            self.keys.power_held = false;
        }
        self.care_input_with(!saver);
        self.seat.last_input = "key";
    }

    /// ", N ms after the Power key" for a journal line about the standby, when the key was pressed in the last minute.
    pub(crate) fn since_power_key(&self) -> String {
        match self.keys.power_at.map(|t| t.elapsed()) {
            Some(d) if d < Duration::from_secs(60) => format!(", {} ms after the Power key", d.as_millis()),
            _ => String::new(),
        }
    }

    /// Called every tick: the remote's holds come due, an idle cursor hides, and the shell client is looked after.
    pub fn poll_holds(&mut self) {
        self.poll_holds_at(Instant::now());
    }

    pub fn poll_holds_at(&mut self, now: Instant) {
        if self.shell_rust_tick(now) {
            self.ui_socket_new = true;
        }
        self.motion_tick(now);
        self.keys.poll_theme(now);
        crate::server::ime::own_tick(self, now);
        if self.keys.router.waiting() {
            let foreground = self.foreground_app();
            let keyboard_up = self.keyboard_up();
            let picker_ready = self.shell_can("picker");
            let actions = {
                let Keys { router, .. } = &mut self.keys;
                router.poll(now, &Ctx { foreground: &foreground, keyboard_up, picker_ready })
            };
            self.run_actions(actions);
        }
        if self.seat.cursor_visible
            && let Some(t) = self.seat.cursor_shown_at
            && t.elapsed() >= seat::CURSOR_TIMEOUT
        {
            seat::hide_cursor(self);
        }
    }

    fn run_actions(&mut self, actions: Vec<Action>) {
        for a in actions {
            match a {
                Action::Request(r) => self.system_request(r),
                Action::ClientRelease(code) => self.release_held(code),
            }
        }
    }

    /// A hold's action ran after the app (or the keyboard) was given the press: it gets the release now, so that no key stays down
    /// in its state while the real release is swallowed.
    fn release_held(&mut self, code: u32) {
        let seats: Vec<seat::SeatId> = self.seat.pressed.iter().filter(|(_, c)| *c == code).map(|(s, _)| *s).collect();
        for s in seats {
            seat::send_key(self, s, code, false);
        }
        if self.ime.grab_down.contains(&code) {
            self.grab_key(code, false);
        }
    }

    /// The owner's choice for this button (`buttons.rs`, Settings > Remote & pointer > Buttons): the key to go on with, the same or the one
    /// the button was set to act as, or `None` for a key taken here, its repeats and release too. A release goes the way its press went,
    /// whatever the theme says by then.
    fn remap_button(&mut self, code: u32, pressed: bool) -> Option<u32> {
        if let Some(i) = self.keys.mapped.iter().position(|(c, _)| *c == code) {
            let to = self.keys.mapped[i].1;
            if !pressed {
                self.keys.mapped.swap_remove(i);
            }
            return to;
        }
        if !pressed {
            return Some(code);
        }
        // A choice made in Settings applies to the next press, not to the next second's look at the file.
        if buttons::ButtonMap::settable(code, &self.keys.router.table) || self.keys.buttons.knows(code, &self.keys.router.table) {
            self.keys.refresh_theme();
        }
        if !self.keys.buttons.knows(code, &self.keys.router.table) {
            return Some(code);
        }
        let foreground = self.foreground_app();
        match self.keys.buttons.decide(code, &self.keys.router.table, &foreground) {
            buttons::Decision::Pass => Some(code),
            buttons::Decision::Replace(to) => {
                self.journal(format!("input: button {code} acts as {to} (the owner's choice)"));
                self.keys.mapped.push((code, Some(to)));
                Some(to)
            }
            buttons::Decision::Take(request) => {
                self.keys.mapped.push((code, None));
                match request {
                    Some(r) => self.system_request(r),
                    None => self.journal(format!("input: button {code}: nothing to do (the owner's choice, or its app is in front)")),
                }
                None
            }
        }
    }

    /// A code in LG's own range (768 and up) that the key table has no name for, noted once with how to teach it: the buttons this set's
    /// remote has that the table was built without (Disney+, LG Channels) are read from the journal after a press.
    fn note_unnamed_key(&mut self, code: u32, pressed: bool) {
        if pressed
            && code >= 768
            && code != code::KEY_RF_HOME
            && self.keys.unnamed.len() < 32
            && !self.keys.router.table.knows(code)
            && !self.keys.unnamed.contains(&code)
        {
            self.keys.unnamed.push(code);
            self.journal(format!(
                "input: key {code} has no name in the key table; for a button the routing should know, add `NAME {code}` to compositor/keys.conf"
            ));
        }
    }

    /// The app id of the foreground card, empty when there is none.
    pub fn foreground_app(&self) -> String {
        self.scene.foreground(&self.surfaces).map(|w| w.app_id().to_string()).unwrap_or_default()
    }

    /// Whether the native keyboard is visible for the active text field.
    pub fn keyboard_up(&self) -> bool {
        self.ime.own_taking()
    }

    pub(crate) fn home_pressed(&mut self, from_network: bool) {
        if self.native_home_key(from_network) {
            return;
        }
        self.journal(format!("input: Home{}: launchDefaultApp", if from_network { " (network key)" } else { "" }));
        self.luna_call(
            "luna://com.webos.applicationManager/launchDefaultApp".into(),
            serde_json::json!({"category": "home", "params": {"launchType": "homeKey", "activateType": "home"}}),
        );
    }

    /// Back or Exit closes the focused app, except Home.
    pub(crate) fn back_pressed(&mut self) {
        let Some(wid) = self.seat.focus.and_then(|s| self.scene.window_of(s)) else { return };
        let app = self.scene.window(wid).map(|w| w.app_id().to_string()).unwrap_or_default();
        if app == crate::scene::HOME_APP {
            self.journal("input: Back or Exit on Home: nothing to close".into());
            return;
        }
        if let Some(res) = self.webos_shell_resource(wid) {
            res.close();
        }
        if !app.is_empty() {
            self.journal(format!("input: Back or Exit: closing {app}"));
            self.luna_call("luna://com.webos.applicationManager/closeByAppId".into(), serde_json::json!({"id": app, "reason": "windowPolicy"}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_a_remote_sends_for_home_is_home() {
        let all = key_mask::DEFAULT;
        for (name, home) in [("KEY_HOME", code::KEY_HOME), ("KEY_HOMEPAGE", code::KEY_HOMEPAGE), ("RF_HOME", code::KEY_RF_HOME)] {
            assert_eq!(route(home, all, false, false), Route::Home, "{name}");
            assert_eq!(route(home, all, false, true), Route::Home, "{name}: the default mask leaves Home out");
            assert_eq!(route(home, all | key_mask::HOME, false, true), Route::Client, "{name}: an app with the policy and the mask gets it");
        }
        assert_eq!((code::KEY_HOME, code::KEY_HOMEPAGE, code::KEY_RF_HOME), (102, 172, 773));
    }

    #[test]
    fn keys_follow_the_mask_and_the_access_policies() {
        let all = key_mask::DEFAULT;
        assert_eq!(route(code::KEY_LEFT, all, false, false), Route::Client);
        assert_eq!(route(code::KEY_LEFT, 0, false, false), Route::Dropped);
        assert_eq!(route(code::KEY_PREVIOUS, 0, false, false), Route::Back, "Back with no policy is the compositor's");
        assert_eq!(route(code::KEY_PREVIOUS, 0, true, false), Route::Client, "KEYS_BACK policy hands Back to the app");
        assert_eq!(route(code::KEY_PREVIOUS, key_mask::BACK, false, false), Route::Client);
        assert_eq!(route(code::KEY_HOMEPAGE, all, false, false), Route::Home, "Home never reaches an app without the policy");
        assert_eq!(route(code::KEY_HOMEPAGE, all, false, true), Route::Home, "the default mask leaves Home out");
        assert_eq!(route(code::KEY_HOMEPAGE, all | key_mask::HOME, false, true), Route::Client);
        assert_eq!(route(code::KEY_RED, key_mask::NAVIGATION, false, false), Route::Dropped);
        assert_eq!(route(code::KEY_RED, key_mask::COLOUR, false, false), Route::Client);
        assert_eq!(route(6, key_mask::NUMERIC, false, false), Route::Client, "KEY_5");
        assert_eq!(route(6, key_mask::NAVIGATION, false, false), Route::Dropped);
        assert_eq!(route(9999, 0, false, false), Route::Client, "unknown keys pass through");
    }
}

#[cfg(test)]
mod abs_tests {
    use super::*;

    #[test]
    fn axes_map_onto_the_output_and_pixels_pass_through() {
        assert_eq!(AbsRange::scale((0, 1919), 0, 1920), 0);
        assert_eq!(AbsRange::scale((0, 1919), 1919, 1920), 1919);
        assert_eq!(AbsRange::scale((0, 1919), 960, 1920), 960);
        assert_eq!(AbsRange::scale((0, 65535), 65535, 1080), 1079, "a 16-bit axis ends at the last pixel");
        assert_eq!(AbsRange::scale((0, 65535), 32768, 1920), 959);
        assert_eq!(AbsRange::scale((0, 0), 2500, 1920), 1919, "no range: the value is a pixel, clamped");
        assert_eq!(AbsRange::scale((0, 0), -5, 1920), 0);
    }
}

#[cfg(test)]
mod wheel_tests {
    use super::*;

    /// A file of raw events read as a device (the ioctls fail on it, which a device without absolute axes also answers).
    fn read(events: &[(u16, u16, i32)], wheel: bool) -> Vec<Event> {
        read_as(events, wheel, false)
    }

    /// The same for a device that is only watched for activity.
    fn read_watched(events: &[(u16, u16, i32)]) -> Vec<Event> {
        read_as(events, false, true)
    }

    fn read_as(events: &[(u16, u16, i32)], wheel: bool, watch: bool) -> Vec<Event> {
        static FILES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!("lumatv-wheel-{}-{}", std::process::id(), FILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let mut bytes = Vec::new();
        for &(kind, code, value) in events {
            let ev = InputEvent { kind, code, value, ..InputEvent::default() };
            // SAFETY: InputEvent is repr(C) plain data; its bytes are what the kernel writes for one event.
            bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(&ev as *const InputEvent as *const u8, std::mem::size_of::<InputEvent>()) });
        }
        std::fs::write(&path, bytes).unwrap();
        let mut device = Device::open(path.to_str().unwrap()).unwrap();
        device.wheel = wheel;
        device.watch = watch;
        let mut out = Vec::new();
        device.drain(|e| out.push(e)).unwrap();
        let _ = std::fs::remove_file(path);
        out
    }

    #[test]
    fn a_notch_of_the_wheel_is_a_wheel_event_and_no_other_relative_axis_is() {
        assert_eq!(read(&[(EV_REL, REL_WHEEL, 1), (EV_REL, REL_WHEEL, -1), (EV_SYN, 0, 0)], true), vec![Event::Wheel { down: false }, Event::Wheel { down: true }], "a turn away is up, a turn towards is down");
        assert_eq!(read(&[(EV_REL, REL_WHEEL_HI_RES, 120)], true), vec![Event::Wheel { down: false }]);
        assert_eq!(read(&[(EV_REL, REL_WHEEL_HI_RES, -240)], true), vec![Event::Wheel { down: true }]);
        assert_eq!(
            read(&[(EV_REL, 0, 5), (EV_REL, 1, -5), (EV_REL, 6, 1), (EV_REL, REL_WHEEL, 0)], true),
            vec![Event::Activity],
            "pointing and a sideways wheel are not a turn, but a person's: one activity a read"
        );
        assert_eq!(read(&[(EV_REL, REL_WHEEL, 0), (EV_REL, 1, 0)], true), vec![], "a report of no movement is nobody's");
        assert_eq!(read(&[(EV_REL, REL_WHEEL, 1)], false), vec![Event::Activity], "a phone's scroll is activity, no turn of the pointer's wheel");
        assert_eq!(read(&[(EV_REL, REL_WHEEL, 1), (EV_REL, REL_WHEEL, -1)], false), vec![Event::Activity], "once for the whole read");
        assert_eq!(
            read(&[(EV_KEY, 28, 1), (EV_REL, REL_WHEEL, 1), (EV_KEY, 28, 0)], true),
            vec![Event::Key { code: 28, pressed: true }, Event::Wheel { down: false }, Event::Key { code: 28, pressed: false }]
        );
    }

    #[test]
    fn a_watched_device_only_counts_a_touch_once_a_read_and_routes_nothing() {
        assert_eq!(read_watched(&[(EV_KEY, 0x110, 1), (EV_KEY, 0x110, 0), (EV_KEY, 0x130, 1)]), vec![Event::Activity], "a mouse button and a pad button, once");
        assert_eq!(read_watched(&[(EV_KEY, 0x110, 0)]), vec![], "a release alone is no touch");
        assert_eq!(read_watched(&[(EV_REL, 0, 3), (EV_REL, REL_WHEEL, 1)]), vec![Event::Activity], "a mouse moved");
        assert_eq!(read_watched(&[(EV_SYN, 0, 0), (EV_REL, 0, 0)]), vec![]);
        assert_eq!(read_watched(&[(EV_ABS, ABS_X, 128), (EV_ABS, ABS_Y, 128), (EV_SYN, 0, 0)]), vec![], "a stick's first report only says where it stands: no pointer motion either");
    }

    #[test]
    fn a_stick_counts_only_when_it_moves_a_quarter_of_its_range() {
        let mut axes = AbsActivity::default();
        let range = || Some((0, 255));
        assert!(!axes.moved(ABS_X, 128, range), "the first report sets where it stands");
        for jitter in [126, 131, 129, 124, 140, 128] {
            assert!(!axes.moved(ABS_X, jitter, range), "rest jitter {jitter}");
        }
        assert!(axes.moved(ABS_X, 200, range), "pushed over a quarter of its range");
        assert!(!axes.moved(ABS_X, 205, range), "counted from where it last counted");
        assert!(axes.moved(ABS_X, 128, range), "and back");
        assert!(!axes.moved(ABS_Y, 0, range), "another axis has its own anchor");
        // a hat switch (range -1..=1) counts on any change; an axis the kernel does not have never counts
        let hat = || Some((-1, 1));
        assert!(!axes.moved(0x10, 0, hat));
        assert!(axes.moved(0x10, -1, hat));
        assert!(!axes.moved(ABS_COUNT as u16, 5, hat));
    }

    #[test]
    fn only_the_magic_remotes_wheel_shows_the_pointer() {
        assert_eq!(
            read(&[(EV_REL, REL_WHEEL, 1), (EV_KEY, 28, 1)], false),
            vec![Event::Activity, Event::Key { code: 28, pressed: true }],
            "a wheel that is not a remote's is no turn of the pointer's"
        );
        for name in ["LGE M-RCU - Builtin [0]", "LGE TONE+ - Builtin [3]", "LGE W-RCU - Builtin [5]", "LGE Simple Premium"] {
            assert!(Device::turns_pointer(name, true), "{name}");
        }
        for (name, abs) in [
            ("Smart Remote RCU Input", true),
            ("LGE Network Input", true),
            ("LGE RCU", false),
            ("LGE Smart Remote - TouchPad", false),
            ("ClickableMouse", false),
        ] {
            assert!(!Device::turns_pointer(name, abs), "{name}");
        }
    }
}

#[cfg(test)]
mod hotplug_tests {
    use super::*;

    /// A directory standing in for `/dev/input`, its nodes files of raw events (the ioctls fail on them, as on a device that does not say).
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Dir {
            let dir = std::env::temp_dir().join(format!("lumatv-hotplug-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }

        /// Makes (or makes anew) the node `name` with a press of `key` waiting on it.
        fn node(&self, name: &str, key: u32) {
            let ev = InputEvent { kind: EV_KEY, code: key as u16, value: 1, ..InputEvent::default() };
            // SAFETY: InputEvent is repr(C) plain data; its bytes are what the kernel writes for one event.
            let bytes = unsafe { std::slice::from_raw_parts(&ev as *const InputEvent as *const u8, std::mem::size_of::<InputEvent>()) };
            std::fs::write(self.0.join(name), bytes).unwrap();
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// What the loop does with a new device: a seat of its own, counted.
    fn seats(count: &mut u32) -> impl FnMut(&str) -> seat::SeatId + '_ {
        move |_| {
            *count += 1;
            seat::SeatId(*count)
        }
    }

    fn keys(inputs: &mut Inputs, slot: usize) -> Vec<Event> {
        let mut got = Vec::new();
        inputs.device(slot).unwrap().drain(|e| got.push(e)).unwrap();
        got
    }

    #[test]
    fn a_device_made_after_the_start_is_opened_and_its_keys_are_read() {
        let dir = Dir::new("late");
        dir.node("event2", code::KEY_LEFT);
        let mut inputs = Inputs::new(&dir.0);
        let mut made = 0;
        assert_eq!(inputs.scan(0, &mut seats(&mut made)).opened.len(), 1, "the start finds the one node");

        // lginput2 makes its virtual device after the compositor is up.
        dir.node("event10", code::KEY_RIGHT);
        dir.node("mouse0", code::KEY_ENTER);
        let found = inputs.scan(1, &mut seats(&mut made));
        assert_eq!(found.opened.iter().map(|o| o.0).collect::<Vec<_>>(), [1], "only the new `event` node");
        assert_eq!(keys(&mut inputs, 1), [Event::Key { code: code::KEY_RIGHT, pressed: true }]);
        assert_eq!(inputs.seat(1), seat::SeatId(2), "a seat of its own");
        assert!(inputs.scan(2, &mut seats(&mut made)).opened.is_empty(), "an open device is not opened again");
    }

    #[test]
    fn a_device_made_anew_goes_back_to_the_slot_and_the_seat_of_the_one_that_went() {
        let dir = Dir::new("anew");
        dir.node("event10", code::KEY_RIGHT);
        let mut inputs = Inputs::new(&dir.0);
        let mut made = 0;
        inputs.scan(0, &mut seats(&mut made));
        assert_eq!(keys(&mut inputs, 0).len(), 1);

        // The wake: the old device is gone, and the node is made again.
        assert!(inputs.lose(0).is_some());
        assert!(inputs.device(0).is_none());
        dir.node("event10", code::KEY_LEFT);
        let found = inputs.scan(1, &mut seats(&mut made));
        assert_eq!(found.opened.iter().map(|o| o.0).collect::<Vec<_>>(), [0]);
        assert_eq!(keys(&mut inputs, 0), [Event::Key { code: code::KEY_LEFT, pressed: true }], "its keys arrive again");
        assert_eq!((made, inputs.seat(0)), (1, seat::SeatId(1)), "no second seat for clients to bind");
    }

    #[test]
    fn a_node_that_does_not_open_is_told_once() {
        let dir = Dir::new("told");
        let node = dir.0.join("event3");
        std::os::unix::fs::symlink(dir.0.join("nowhere"), &node).unwrap();
        let mut inputs = Inputs::new(&dir.0);
        let mut made = 0;
        assert_eq!(inputs.scan(0, &mut seats(&mut made)).problems.len(), 1);
        assert_eq!(inputs.scan(1, &mut seats(&mut made)).problems.len(), 0, "the same trouble is not told again");
        std::fs::remove_file(&node).unwrap();
        inputs.scan(2, &mut seats(&mut made));
        std::os::unix::fs::symlink(dir.0.join("nowhere"), &node).unwrap();
        assert_eq!(inputs.scan(3, &mut seats(&mut made)).problems.len(), 1, "a node that came back is told afresh");
    }

    /// Registers a device behind `name` in the directory standing in for `/sys/class/input`, as input number `input` (made anew: the same
    /// name with another number).
    fn register(class: &Path, name: &str, input: u32) {
        let link = class.join(name);
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(format!("../../devices/virtual/input/input{input}/{name}"), link).unwrap();
    }

    /// One look at the directory, answering `found` for every node it opens: the nodes it looked at, by name, and what it told.
    fn looked_at(inputs: &mut Inputs, at: u64, found: fn(&str) -> Probe) -> (Vec<String>, Vec<String>) {
        let mut asked = Vec::new();
        let mut made = 0;
        let scan = inputs.scan_with(at, &mut seats(&mut made), &mut |path| {
            asked.push(Path::new(path).file_name().unwrap().to_string_lossy().into_owned());
            found(path)
        });
        (asked, scan.problems)
    }

    fn a_pad(_: &str) -> Probe {
        Probe::NoKeys("Wireless Controller".into())
    }

    fn refused(path: &str) -> Probe {
        let errno = if path.ends_with("event4") { libc::ENXIO } else { libc::EACCES };
        Probe::Failed(io::Error::from_raw_os_error(errno))
    }

    #[test]
    fn a_node_with_no_device_behind_it_is_not_opened_until_a_device_is_registered_there() {
        // LG's init makes event14 to event31 with nothing behind them: every open of one runs modprobe (7 ms and more of the loop).
        let (dir, class) = (Dir::new("stale"), Dir::new("stale-class"));
        for name in ["event2", "event15", "event16"] {
            dir.node(name, code::KEY_LEFT);
        }
        register(&class.0, "event2", 3);
        let mut inputs = Inputs::listed_by(&dir.0, Some(class.0.clone()));
        assert_eq!(looked_at(&mut inputs, 0, a_pad).0, ["event2"], "only the node with a device behind it");
        assert!(looked_at(&mut inputs, 1, a_pad).0.is_empty(), "and not again");

        // A device takes over a node that was always there: no node is made, the kernel's list is all that says so.
        register(&class.0, "event15", 4);
        assert_eq!(looked_at(&mut inputs, 2, a_pad).0, ["event15"]);
    }

    #[test]
    fn a_device_with_no_keys_is_looked_at_once_and_again_when_it_is_made_anew() {
        // Each look at one was an open and a close, and a close waits for a grace period of the kernel's RCU (about 20 ms of the loop).
        let (dir, class) = (Dir::new("pad"), Dir::new("pad-class"));
        dir.node("event8", code::KEY_LEFT);
        register(&class.0, "event8", 5);
        let mut inputs = Inputs::listed_by(&dir.0, Some(class.0.clone()));
        let (asked, told) = looked_at(&mut inputs, 0, a_pad);
        assert_eq!((asked, told.len()), (vec!["event8".to_string()], 1));
        for at in 1..5 {
            assert!(looked_at(&mut inputs, at, a_pad).0.is_empty(), "look {at}");
        }
        register(&class.0, "event8", 6);
        assert_eq!(looked_at(&mut inputs, 5, a_pad).0, ["event8"], "the wake made it anew");

        // Without the kernel's list a node is the same until it has been gone from the directory.
        let plain = Dir::new("pad-plain");
        plain.node("event9", code::KEY_LEFT);
        let mut inputs = Inputs::listed_by(&plain.0, None);
        assert_eq!(looked_at(&mut inputs, 0, a_pad).0, ["event9"]);
        assert!(looked_at(&mut inputs, 1, a_pad).0.is_empty());
        std::fs::remove_file(plain.0.join("event9")).unwrap();
        looked_at(&mut inputs, 2, a_pad);
        plain.node("event9", code::KEY_RIGHT);
        assert_eq!(looked_at(&mut inputs, 3, a_pad).0, ["event9"]);
    }

    #[test]
    fn a_device_without_keys_that_can_be_touched_is_watched_with_no_seat_of_its_own() {
        let dir = Dir::new("watch-only");
        dir.node("event8", 0x110);
        let mut inputs = Inputs::listed_by(&dir.0, None);
        let mut made = 0;
        let found = inputs.scan_with(0, &mut seats(&mut made), &mut |path| {
            let mut d = Device::open(path).unwrap();
            d.watch = true;
            Probe::Watch(Box::new(d))
        });
        assert_eq!((found.opened.len(), made, inputs.seat(0), inputs.is_watch(0)), (1, 0, seat::SeatId(0), true), "no seat made for clients to bind");
        assert!(found.lines[0].ends_with("(activity only)"), "{:?}", found.lines);
        assert_eq!(keys(&mut inputs, 0), [Event::Activity], "a button of it is a touch, not a key");
        assert!(inputs.lose(0).is_some());
        assert!(!inputs.is_watch(0), "a device that is gone watches nothing");
    }

    #[test]
    fn a_refusal_of_no_such_device_is_not_asked_again_and_any_other_is() {
        let dir = Dir::new("refused");
        dir.node("event4", code::KEY_LEFT);
        dir.node("event5", code::KEY_LEFT);
        let mut inputs = Inputs::listed_by(&dir.0, None);
        let (asked, told) = looked_at(&mut inputs, 0, refused);
        assert_eq!((asked.len(), told.len()), (2, 2));
        assert_eq!(looked_at(&mut inputs, 1, refused).0, ["event5"], "permissions may be put right; a missing device waits for its node to change");
    }

    #[test]
    fn only_a_device_with_keys_below_the_buttons_is_a_remote_or_a_keyboard() {
        let mut remote = [0u8; KEY_BITMAP_BYTES];
        remote[(code::KEY_ENTER / 8) as usize] |= 1 << (code::KEY_ENTER % 8);
        assert!(has_keys(&remote));
        // A game controller: BTN_SOUTH (0x130) and its neighbours only; its touchpad and motion sensors have no keys at all.
        let mut controller = [0u8; KEY_BITMAP_BYTES];
        controller[0x130 / 8] = 0xff;
        assert!(!has_keys(&controller));
        assert!(!has_keys(&[0u8; KEY_BITMAP_BYTES]));
        let mut reserved = [0u8; KEY_BITMAP_BYTES];
        reserved[0] = 1;
        assert!(!has_keys(&reserved), "KEY_RESERVED is no key");
    }

    #[test]
    fn a_node_made_in_the_directory_wakes_the_watch_and_the_look_is_due_at_once() {
        let dir = Dir::new("watch");
        let mut inputs = Inputs::new(&dir.0);
        assert!(!inputs.due(u64::MAX - 1), "no look before the first");
        let fd = inputs.watch().expect("inotify");
        let readable = |fd| {
            let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            // SAFETY: one pollfd, a zero timeout.
            unsafe { libc::poll(&mut p, 1, 0) == 1 }
        };
        let mut made = 0;
        inputs.scan(100, &mut seats(&mut made));
        assert!(!readable(fd));
        assert!(!inputs.due(100 + RESCAN_NS - 1) && inputs.due(100 + RESCAN_NS), "without a word, the directory is read every {RESCAN_NS} ns");

        dir.node("event7", code::KEY_ENTER);
        assert!(readable(fd), "the node is announced before the call that made it returns");
        inputs.watched(200);
        assert!(!readable(fd), "what was announced is taken");
        assert!(inputs.due(200));
    }
}
