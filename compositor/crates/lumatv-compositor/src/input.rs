//! Keys from the set's input devices (C4). `lginput2` turns every remote (IR `LGE RCU`, the Magic Remote `LGE
//! M-RCU`, network keys as `Smart Remote RCU Input`, …) into evdev devices; the compositor reads them all and
//! routes each key by the focused window's key mask and access policies, keeping the compositor's own keys
//! (Home, the hold-Home escape) for itself. Reads are allocation-free (one fixed buffer per device).

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

const EV_KEY: u16 = 1;

pub struct Device {
    pub path: String,
    pub name: String,
    /// `Smart Remote RCU Input`: keys from the network (SSAP), told apart from the physical remotes (PLAN, AI access).
    pub from_network: bool,
    file: File,
    buf: [InputEvent; 64],
}

impl Device {
    pub fn open(path: &str) -> io::Result<Device> {
        let file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(path)?;
        let mut name = [0u8; 128];
        // SAFETY: EVIOCGNAME(len) writes at most len bytes into the buffer.
        let n = unsafe { libc::ioctl(file.as_raw_fd(), eviocgname(name.len()), name.as_mut_ptr()) };
        let name = if n > 0 { String::from_utf8_lossy(&name[..(n as usize).min(name.len())]).trim_end_matches('\0').to_string() } else { String::new() };
        let from_network = name.contains("Smart Remote RCU Input");
        Ok(Device { path: path.to_string(), name, from_network, file, buf: [InputEvent::default(); 64] })
    }

    pub fn fd(&self) -> i32 {
        self.file.as_raw_fd()
    }

    /// Reads what is pending; `f(code, pressed)` per key event (repeats are reported as presses).
    pub fn drain(&mut self, mut f: impl FnMut(u32, bool)) -> io::Result<()> {
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
            for ev in &self.buf[..n / size] {
                if ev.kind == EV_KEY {
                    f(ev.code as u32, ev.value != 0);
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

/// Every `/dev/input/event*` that opens.
pub fn open_all() -> Vec<Device> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/dev/input") else { return out };
    let mut paths: Vec<String> = dir.filter_map(|e| e.ok()).map(|e| e.path().to_string_lossy().into_owned()).filter(|p| p.contains("/event")).collect();
    paths.sort();
    for p in paths {
        match Device::open(&p) {
            Ok(d) => {
                log::info!("input: {} {:?}{}", d.path, d.name, if d.from_network { " (network keys)" } else { "" });
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
    /// One key from a device.
    pub fn key_event(&mut self, code: u32, pressed: bool, from_network: bool) {
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
                if seat::send_key(self, code, pressed) {
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
        }
        self.seat.last_input = "key";
    }

    /// Called every tick: a Home held 650 ms is the escape to LG's compositor (PLAN).
    pub fn poll_holds(&mut self) {
        if let Some(t) = self.keys.home_down_since
            && !self.keys.home_escape_sent
            && t.elapsed() >= HOLD_HOME
        {
            self.keys.home_escape_sent = true;
            self.journal("input: Home held 650 ms: asking the supervisor to fall back to LG's compositor".into());
            self.luna.send(ToBridge::Fallback { reason: "hold Home 650 ms".into() });
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
