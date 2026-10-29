//! Keys and the pointer from the set's input devices (C4). `lginput2` turns every remote (IR `LGE RCU`, the Magic
//! Remote `LGE M-RCU`, network keys as `Smart Remote RCU Input`, …) into evdev devices; the compositor reads them
//! all and routes each key by the focused window's key mask and access policies, keeping the compositor's own
//! keys (Home, the hold-Home escape) for itself. The Magic Remote's pointer arrives as absolute axes (the census in
//! `docs/research/arch/input-devices-01.txt`: `ABS=3` on every `LGE M-RCU`, the only relative axis is the wheel);
//! a turn of the wheel shows the cursor (`remote.pointerShow` `wheel`, the default; `movement` is LG's: a motion shows it),
//! motion moves it while it shows, OK clicks while it shows, a navigation key or 10 s of stillness hides it. Reads are
//! allocation-free (one fixed buffer per device).

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

use crate::State;
use crate::keys::{self, Action, Ctx};
use crate::luna::ToBridge;
use crate::server::seat;
use crate::window::{WindowType, key_mask};

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
    pub const KEY_MUTE: u32 = 113;
    pub const KEY_VOLUMEDOWN: u32 = 114;
    pub const KEY_VOLUMEUP: u32 = 115;
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

/// How long Home is held for the escape to LG's compositor (PLAN: 650 ms).
pub const HOLD_HOME: Duration = Duration::from_millis(650);

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

/// What a device reports: one key, the pointer's new position in output pixels (once per `SYN_REPORT`), or a notch of the wheel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Key { code: u32, pressed: bool },
    Motion { x: i32, y: i32 },
    Wheel,
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
                match ev.kind {
                    EV_KEY => f(Event::Key { code: ev.code as u32, pressed: ev.value != 0 }),
                    EV_REL if self.wheel && matches!(ev.code, REL_WHEEL | REL_WHEEL_HI_RES) && ev.value != 0 => f(Event::Wheel),
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

/// `EVIOCSCLOCKID` (`_IOW('E', 0xa0, int)`): the clock the kernel stamps this descriptor's events with.
const EVIOCSCLOCKID: libc::c_ulong = 0x4004_45a0;

const fn eviocgname(len: usize) -> libc::c_ulong {
    // _IOC(_IOC_READ, 'E', 0x06, len)
    ((2u64 << 30) | ((len as u64) << 16) | (b'E' as u64) << 8 | 0x06) as libc::c_ulong
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

/// Every `event*` of `dir` (`/dev/input` on the set) that opens.
pub fn open_all(dir: &std::path::Path) -> Vec<Device> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir(dir) else { return out };
    let mut paths: Vec<String> = dir.filter_map(|e| e.ok()).map(|e| e.path().to_string_lossy().into_owned()).filter(|p| p.contains("/event")).collect();
    paths.sort();
    for p in paths {
        match Device::open(&p) {
            Ok(d) => {
                log::info!(
                    "input: {} {:?}{}{}",
                    d.path,
                    d.name,
                    if d.from_network { " (network keys)" } else { "" },
                    d.abs.map(|a| format!(" pointer x {:?} y {:?}", a.x, a.y)).unwrap_or_default()
                );
                out.push(d);
            }
            Err(e) => log::warn!("input: {p}: {e}"),
        }
    }
    out
}

/// The compositor's key state: the Home hold, the remote's own keys (`keys.rs`) and what was delivered.
#[derive(Default)]
pub struct Keys {
    home_down_since: Option<Instant>,
    home_escape_sent: bool,
    pub delivered: u64,
    pub dropped: u64,
    pub router: keys::Router,
    /// Settings > Remote's app buttons, re-read when the theme file changes.
    pub app_keys: keys::AppKeyConfig,
    /// What shows the Magic Remote's pointer (`remote.pointerShow`), read with the app buttons.
    pub pointer_show: seat::PointerShow,
    theme: ThemeFile,
    /// Codes in LG's own range that the key table has no name for, once each: a button of this remote it lacks.
    pub unnamed: Vec<u32>,
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
            self.app_keys = keys::AppKeyConfig::default();
            self.pointer_show = seat::PointerShow::default();
            self.theme.stamp = None;
        } else if let Some(theme) = std::fs::read_to_string(&self.theme.path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
            self.app_keys = keys::AppKeyConfig::from_theme(&theme);
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
        // Told over LG's Input Manager queue that the pointer shows (`im_queue`), lginput2 sends the wheel's press as the mouse button, BTN_LEFT, in
        // place of KEY_ENTER (entries 0 and 185 of its key table). It is the same OK: a click while the pointer shows, the key when it hid first.
        let code = if code == seat::BTN_LEFT { code::KEY_ENTER } else { code };
        self.scene.input(crate::output::monotonic_ns());
        // While the Magic Remote's cursor shows, OK is a click on what it points at (LG's behaviour); an arrow or
        // Back hides the cursor and goes on as a key. A release goes the way its press went: a press taken as a key (by the Rust shell, or by an app's
        // keyboard) has its release as a key even if the cursor showed in between, and one taken as a click has its release as one.
        let key_press_open = !pressed && (self.shell_rust.took.iter().any(|(c, _)| *c == code) || self.seat.pressed.iter().any(|(_, c)| *c == code));
        if matches!(code, code::KEY_ENTER | code::KEY_OK) && !key_press_open && self.seat.cursor_visible && (self.seat.pointer_focus.is_some() || self.shell_rust.pointer_in) {
            if seat::pointer_button(self, pressed) {
                self.keys.delivered += 1;
                return;
            }
        } else if pressed && matches!(code, code::KEY_UP | code::KEY_DOWN | code::KEY_LEFT | code::KEY_RIGHT | code::KEY_BACK | code::KEY_PREVIOUS) {
            seat::hide_cursor(self);
        }
        self.note_unnamed_key(code, pressed);
        // The remote's own keys (gear, Input, the app buttons, the holds) are decided before any app sees one.
        if pressed && matches!(self.keys.router.table.classify(code), keys::Logical::App(_)) {
            self.keys.refresh_theme();
        }
        let foreground = self.foreground_app();
        let keyboard_up = self.keyboard_up();
        let picker_ready = self.shell_can("picker");
        let logical = self.keys.router.table.classify(code);
        let first_press = pressed && !self.keys.router.is_down(code);
        let out = {
            let Keys { router, app_keys, .. } = &mut self.keys;
            router.event(code, pressed, now, &Ctx { foreground: &foreground, keyboard_up, app_keys, picker_ready })
        };
        self.run_actions(out.actions);
        // The gear is down: its tap opens the quick panel at the release, and the shell puts the panel in the scene now, off the
        // screen, so the slide of the opening starts in a scene that has it (LG's filter chain gave the same cue).
        if first_press && logical == keys::Logical::Gear {
            self.shell_show("quick", true);
        }
        // The overlay cards' OK and Back, which the shell's system layer asks for by name while an app has the focus (`takes`): they go to
        // the layer as `syskey` events, the arrows and everything else stay the app's (`shell_takes_key`).
        if self.shell_takes_key(seat, code, pressed, out.swallow, now) {
            self.seat.last_input = "key";
            return;
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
        // The shell's windows (LumaTV's Home, its system layer with a panel open) get every key but the compositor's own: Back
        // closes what they have open, never them, and Home stays the escape and the Home app's.
        let shell_focus = focused.is_some_and(|w| w.kind.is_shell());
        let r = if shell_focus
            && !matches!(code, code::KEY_HOME | code::KEY_HOMEPAGE | code::KEY_RF_HOME | code::KEY_VOLUMEUP | code::KEY_VOLUMEDOWN | code::KEY_MUTE)
        {
            Route::Client
        } else {
            route(code, mask, back, home)
        };
        if matches!(code, code::KEY_HOME | code::KEY_HOMEPAGE | code::KEY_RF_HOME) {
            if pressed {
                if self.keys.home_down_since.is_none() {
                    self.keys.home_down_since = Some(now);
                    self.keys.home_escape_sent = false;
                }
            } else {
                let held = self.keys.home_down_since.take().map(|t| now.saturating_duration_since(t)).unwrap_or_default();
                if !self.keys.home_escape_sent && r == Route::Home && held < HOLD_HOME {
                    self.home_tap(now, from_network, self.shell_hears_home_key(shell_focus));
                }
            }
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

    /// Called every tick: a Home held 650 ms is the escape to LG's compositor (PLAN); the holds of the remote's own keys come
    /// due; a cursor still for [`seat::CURSOR_TIMEOUT`] hides; the shell client is looked after.
    pub fn poll_holds(&mut self) {
        self.poll_holds_at(Instant::now());
    }

    pub fn poll_holds_at(&mut self, now: Instant) {
        if let Some(t) = self.keys.home_down_since
            && !self.keys.home_escape_sent
            && now.saturating_duration_since(t) >= HOLD_HOME
        {
            self.keys.home_escape_sent = true;
            self.journal("input: Home held 650 ms: asking the supervisor to fall back to LG's compositor".into());
            self.luna.send(ToBridge::Fallback { reason: "hold Home 650 ms".into() });
        }
        self.shell_tick(now);
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
                let Keys { router, app_keys, .. } = &mut self.keys;
                router.poll(now, &Ctx { foreground: &foreground, keyboard_up, app_keys, picker_ready })
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

    /// Whether a keyboard is on the screen with a text field to close it for: its window is mapped (LG's) or the shell's keyboard is up, and
    /// a field holds the activation. A keyboard window nobody asked for (no field) is none of the keys' business.
    pub fn keyboard_up(&self) -> bool {
        self.ime.active_model().is_some() && (self.ime.own_taking() || self.scene.windows().any(|w| w.kind == WindowType::Keyboard && w.map_serial > 0))
    }

    pub(crate) fn home_pressed(&mut self, from_network: bool) {
        self.journal(format!("input: Home{}: launchDefaultApp", if from_network { " (network key)" } else { "" }));
        self.luna_call(
            "luna://com.webos.applicationManager/launchDefaultApp".into(),
            serde_json::json!({"category": "home", "params": {"launchType": "homeKey", "activateType": "home"}}),
        );
    }

    /// Back on a window that does not take it, Exit, Back held: LG's compositor closes the window
    /// (`_WEBOS_LAUNCH_PREV_APP_AFTER_CLOSING` decides what shows next); here the window gets `close` and the app manager is told. The
    /// Home app is not closed by either (LG closes its Home view and nothing else): a Home that closed would leave a black screen, and
    /// neither is the shell's window that has the keys (it got the key itself and closes what it has open).
    pub(crate) fn back_pressed(&mut self) {
        let Some(wid) = self.seat.focus.and_then(|s| self.scene.window_of(s)) else { return };
        let (app, shell) = self.scene.window(wid).map(|w| (w.app_id().to_string(), w.kind.is_shell())).unwrap_or_default();
        if app == crate::scene::HOME_APP || shell {
            self.journal(format!("input: Back or Exit on {}: nothing to close", if shell { "LumaTV's shell" } else { "Home" }));
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
        let path = std::env::temp_dir().join(format!("lumatv-wheel-{}-{}", std::process::id(), events.len()));
        let mut bytes = Vec::new();
        for &(kind, code, value) in events {
            let ev = InputEvent { kind, code, value, ..InputEvent::default() };
            // SAFETY: InputEvent is repr(C) plain data; its bytes are what the kernel writes for one event.
            bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(&ev as *const InputEvent as *const u8, std::mem::size_of::<InputEvent>()) });
        }
        std::fs::write(&path, bytes).unwrap();
        let mut device = Device::open(path.to_str().unwrap()).unwrap();
        device.wheel = wheel;
        let mut out = Vec::new();
        device.drain(|e| out.push(e)).unwrap();
        let _ = std::fs::remove_file(path);
        out
    }

    #[test]
    fn a_notch_of_the_wheel_is_a_wheel_event_and_no_other_relative_axis_is() {
        assert_eq!(read(&[(EV_REL, REL_WHEEL, 1), (EV_REL, REL_WHEEL, -1), (EV_SYN, 0, 0)], true), vec![Event::Wheel, Event::Wheel]);
        assert_eq!(read(&[(EV_REL, REL_WHEEL_HI_RES, 120)], true), vec![Event::Wheel]);
        assert_eq!(
            read(&[(EV_REL, 0, 5), (EV_REL, 1, -5), (EV_REL, 6, 1), (EV_REL, REL_WHEEL, 0)], true),
            vec![],
            "pointing and a sideways wheel are not a turn"
        );
        assert_eq!(
            read(&[(EV_KEY, 28, 1), (EV_REL, REL_WHEEL, 1), (EV_KEY, 28, 0)], true),
            vec![Event::Key { code: 28, pressed: true }, Event::Wheel, Event::Key { code: 28, pressed: false }]
        );
    }

    #[test]
    fn only_the_magic_remotes_wheel_shows_the_pointer() {
        assert_eq!(
            read(&[(EV_REL, REL_WHEEL, 1), (EV_KEY, 28, 1)], false),
            vec![Event::Key { code: 28, pressed: true }],
            "a wheel that is not a remote's is no turn"
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
