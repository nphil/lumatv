//! Keys and the pointer from the set's input devices (C4). `lginput2` turns every remote (IR `LGE RCU`, the Magic
//! Remote `LGE M-RCU`, network keys as `Smart Remote RCU Input`, …) into evdev devices; the compositor reads them
//! all and routes each key by the focused window's key mask and access policies, keeping the compositor's own
//! keys (Home, the hold-Home escape) for itself. The Magic Remote's pointer arrives as absolute axes (the census in
//! `docs/research/arch/input-devices-01.txt`: `ABS=3` on every `LGE M-RCU`, the only relative axis is the wheel);
//! a motion shows the cursor, OK clicks while it shows, a navigation key or 10 s of stillness hides it. Reads are
//! allocation-free (one fixed buffer per device).

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

use crate::State;
use crate::luna::ToBridge;
use crate::server::seat;
use crate::window::key_mask;

/// Linux input event codes the routing knows by name (`linux/input-event-codes.h`).
pub mod code {
    pub const KEY_HOME: u32 = 102;
    pub const KEY_HOMEPAGE: u32 = 172;
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
        KEY_HOME | KEY_HOMEPAGE => {
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
const EV_ABS: u16 = 3;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;

/// What a device reports: one key, or the pointer's new position in output pixels (once per `SYN_REPORT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Key { code: u32, pressed: bool },
    Motion { x: i32, y: i32 },
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
    file: File,
    buf: [InputEvent; 64],
    /// The position the pending `SYN_REPORT` will report, and the last one reported.
    pending: Option<(i32, i32)>,
    last: (i32, i32),
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
        Ok(Device { path: path.to_string(), name, from_network, abs, file, buf: [InputEvent::default(); 64], pending: None, last: (-1, -1) })
    }

    pub fn fd(&self) -> i32 {
        self.file.as_raw_fd()
    }

    /// Reads what is pending; `f` per key event (repeats are reported as presses) and per pointer motion.
    pub fn drain(&mut self, mut f: impl FnMut(Event)) -> io::Result<()> {
        loop {
            let size = std::mem::size_of::<InputEvent>();
            // SAFETY: the buffer is a plain array of repr(C) structs; read fills whole events.
            let bytes = unsafe { std::slice::from_raw_parts_mut(self.buf.as_mut_ptr() as *mut u8, size * self.buf.len()) };
            let n = match self.file.read(bytes) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            };
            for i in 0..n / size {
                let ev = self.buf[i];
                match ev.kind {
                    EV_KEY => f(Event::Key { code: ev.code as u32, pressed: ev.value != 0 }),
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

/// Every `/dev/input/event*` that opens.
pub fn open_all() -> Vec<Device> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/dev/input") else { return out };
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

/// The compositor's key state: the Home hold and what was delivered.
#[derive(Default)]
pub struct Keys {
    home_down_since: Option<Instant>,
    home_escape_sent: bool,
    pub delivered: u64,
    pub dropped: u64,
}

impl State {
    /// One key from a device, delivered on `seat` (the device's own; seat 0 for the control socket and the tests).
    pub fn key_event(&mut self, seat: seat::SeatId, code: u32, pressed: bool, from_network: bool) {
        // While the Magic Remote's cursor shows, OK is a click on what it points at (LG's behaviour); an arrow or
        // Back hides the cursor and goes on as a key.
        if matches!(code, code::KEY_ENTER | code::KEY_OK) && self.seat.cursor_visible && self.seat.pointer_focus.is_some() {
            if seat::pointer_button(self, pressed) {
                self.keys.delivered += 1;
                return;
            }
        } else if pressed && matches!(code, code::KEY_UP | code::KEY_DOWN | code::KEY_LEFT | code::KEY_RIGHT | code::KEY_BACK | code::KEY_PREVIOUS) {
            seat::hide_cursor(self);
        }
        let (mask, back, home) = self
            .seat
            .focus
            .and_then(|s| self.scene.window_of(s))
            .and_then(|w| self.scene.window(w))
            .map(|w| (w.key_mask, w.access_policy("_WEBOS_ACCESS_POLICY_KEYS_BACK"), w.access_policy("_WEBOS_ACCESS_POLICY_KEYS_HOME")))
            .unwrap_or((0, false, false));
        let r = route(code, mask, back, home);
        if matches!(code, code::KEY_HOME | code::KEY_HOMEPAGE) {
            if pressed {
                if self.keys.home_down_since.is_none() {
                    self.keys.home_down_since = Some(Instant::now());
                    self.keys.home_escape_sent = false;
                }
            } else {
                let held = self.keys.home_down_since.take().map(|t| t.elapsed()).unwrap_or_default();
                if !self.keys.home_escape_sent && r == Route::Home && held < HOLD_HOME {
                    self.home_pressed(from_network);
                }
            }
        }
        match r {
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

    /// Called every tick: a Home held 650 ms is the escape to LG's compositor (PLAN); a cursor still for
    /// [`seat::CURSOR_TIMEOUT`] hides.
    pub fn poll_holds(&mut self) {
        if let Some(t) = self.keys.home_down_since
            && !self.keys.home_escape_sent
            && t.elapsed() >= HOLD_HOME
        {
            self.keys.home_escape_sent = true;
            self.journal("input: Home held 650 ms: asking the supervisor to fall back to LG's compositor".into());
            self.luna.send(ToBridge::Fallback { reason: "hold Home 650 ms".into() });
        }
        if self.seat.cursor_visible
            && let Some(t) = self.seat.cursor_shown_at
            && t.elapsed() >= seat::CURSOR_TIMEOUT
        {
            seat::hide_cursor(self);
        }
    }

    fn home_pressed(&mut self, from_network: bool) {
        self.journal(format!("input: Home{}: launchDefaultApp", if from_network { " (network key)" } else { "" }));
        self.luna_call(
            "luna://com.webos.applicationManager/launchDefaultApp".into(),
            serde_json::json!({"category": "home", "params": {"launchType": "homeKey", "activateType": "home"}}),
        );
    }

    /// Back on a window that does not take it: LG's compositor closes the window (`_WEBOS_LAUNCH_PREV_APP_AFTER_CLOSING`
    /// decides what shows next); here the window gets `close` and the app manager is told.
    fn back_pressed(&mut self) {
        let Some(wid) = self.seat.focus.and_then(|s| self.scene.window_of(s)) else { return };
        let app = self.scene.window(wid).map(|w| w.app_id().to_string()).unwrap_or_default();
        if let Some(res) = self.webos_shell_resource(wid) {
            res.close();
        }
        if !app.is_empty() {
            self.luna_call("luna://com.webos.applicationManager/closeByAppId".into(), serde_json::json!({"id": app, "reason": "windowPolicy"}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
