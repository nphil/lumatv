//! Keys from the remote through evdev: the arrows, OK and Back, read without grabbing the devices (LG's compositor and the key filters
//! keep receiving them). A host has none and the set's scripted runs do not use it.

use lumatv_rustui::home::Key;
use std::fs;
use std::io::Read;
use std::os::fd::AsRawFd;

pub struct Evdev {
    devices: Vec<fs::File>,
}

// linux/input-event-codes.h
const KEY_BACK: u16 = 158;
const KEY_ESC: u16 = 1;
const KEY_ENTER: u16 = 28;
const KEY_UP: u16 = 103;
const KEY_LEFT: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_DOWN: u16 = 108;
const KEY_OK: u16 = 0x160;
const KEY_SELECT: u16 = 0x161;

fn map(code: u16) -> Option<Key> {
    Some(match code {
        KEY_UP => Key::Up,
        KEY_DOWN => Key::Down,
        KEY_LEFT => Key::Left,
        KEY_RIGHT => Key::Right,
        KEY_ENTER | KEY_OK | KEY_SELECT => Key::Ok,
        KEY_BACK | KEY_ESC => Key::Back,
        _ => return None,
    })
}

impl Evdev {
    pub fn open() -> Self {
        let mut devices = Vec::new();
        if let Ok(dir) = fs::read_dir("/dev/input") {
            for entry in dir.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with("event") {
                    continue;
                }
                if let Ok(file) = fs::File::open(entry.path()) {
                    // SAFETY: a plain fcntl on an open descriptor.
                    unsafe {
                        let fd = file.as_raw_fd();
                        let flags = libc::fcntl(fd, libc::F_GETFL);
                        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                    }
                    devices.push(file);
                }
            }
        }
        Self { devices }
    }

    /// The presses (not releases or repeats of other keys; a repeat of an arrow counts, as it does for the QML Home) since the last call.
    pub fn poll(&mut self) -> Vec<Key> {
        let mut out = Vec::new();
        let mut buf = [0u8; 24 * 32];
        for device in &mut self.devices {
            loop {
                match device.read(&mut buf) {
                    Ok(n) if n >= 24 => {
                        for ev in buf[..n - n % 24].chunks_exact(24) {
                            let (kind, code, value) = (
                                u16::from_ne_bytes([ev[16], ev[17]]),
                                u16::from_ne_bytes([ev[18], ev[19]]),
                                i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]),
                            );
                            // EV_KEY, value 1 pressed, 2 repeat
                            if kind == 1 && value >= 1 {
                                if let Some(key) = map(code) {
                                    out.push(key);
                                }
                            }
                        }
                    }
                    _ => break,
                }
            }
        }
        out
    }
}
