//! Finding the configuration interface and talking to it, one request in flight at a time like GameSir Connect's queue (`Uo`).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::Error;
use crate::frame::{self, CONFIG_USAGE_PAGE, FirmwareVersion, Link, Model, REPORT_LEN, REQUEST_ID, Region, Report, Response, cmd};

pub trait Transport {
    fn send(&mut self, report: &Report) -> io::Result<()>;
    /// Waits up to `timeout` for one input report; `Ok(None)` when none came.
    fn recv(&mut self, buf: &mut [u8; REPORT_LEN], timeout: Duration) -> io::Result<Option<usize>>;
}

pub struct Hidraw(File);

impl Hidraw {
    pub fn open(path: &Path) -> io::Result<Self> {
        OpenOptions::new().read(true).write(true).open(path).map(Self)
    }
}

impl Transport for Hidraw {
    fn send(&mut self, report: &Report) -> io::Result<()> {
        // hidraw takes exactly one output report per write, report id first.
        let sent = self.0.write(report)?;
        if sent == report.len() { Ok(()) } else { Err(io::Error::new(io::ErrorKind::WriteZero, format!("hidraw took {sent} of {} bytes", report.len()))) }
    }

    fn recv(&mut self, buf: &mut [u8; REPORT_LEN], timeout: Duration) -> io::Result<Option<usize>> {
        let mut fd = libc::pollfd { fd: self.0.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = timeout.as_micros().div_ceil(1000).min(i32::MAX as u128) as libc::c_int;
        // SAFETY: `fd` is one valid pollfd that outlives the call.
        match unsafe { libc::poll(&mut fd, 1, ms) } {
            0 => Ok(None),
            n if n < 0 => {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted { Ok(None) } else { Err(e) }
            }
            _ => self.0.read(buf).map(Some),
        }
    }
}

/// A hidraw node that belongs to GameSir hardware.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub node: PathBuf,
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    /// `None` when the controller enumerates in a mode without a configuration interface (DS4, Switch, Bluetooth HID).
    pub model: Option<(Model, Link)>,
    /// The node's report descriptor carries the vendor usage page and report 0x0F, so settings can be read and written through it.
    pub config: bool,
}

pub fn discover() -> io::Result<Vec<Found>> {
    discover_in(Path::new("/sys/class/hidraw"), Path::new("/dev"))
}

fn discover_in(class: &Path, dev: &Path) -> io::Result<Vec<Found>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(class)? {
        let entry = entry?;
        let device = entry.path().join("device");
        let Ok(uevent) = fs::read_to_string(device.join("uevent")) else { continue };
        let Some((vendor, product, name)) = parse_uevent(&uevent) else { continue };
        let model = frame::identify(vendor, product);
        // In DS4 mode the receiver uses Sony's ids; only its HID name gives it away.
        if model.is_none() && !name.contains("Chicken Run") && !name.to_ascii_lowercase().contains("gamesir") {
            continue;
        }
        let config = model.is_some() && fs::read(device.join("report_descriptor")).is_ok_and(|d| has_config_reports(&d));
        found.push(Found { node: dev.join(entry.file_name()), name, vendor, product, model, config });
    }
    found.sort_by(|a, b| a.node.cmp(&b.node));
    Ok(found)
}

/// Vendor id, product id and name from a hidraw parent's `uevent` (`HID_ID=0003:00003537:0000102A`).
pub fn parse_uevent(text: &str) -> Option<(u16, u16, String)> {
    let (mut ids, mut name) = (None, String::new());
    for line in text.lines() {
        if let Some(id) = line.strip_prefix("HID_ID=") {
            let mut parts = id.split(':').skip(1).map(|p| u32::from_str_radix(p, 16).ok().and_then(|v| u16::try_from(v).ok()));
            ids = Some((parts.next()??, parts.next()??));
        } else if let Some(n) = line.strip_prefix("HID_NAME=") {
            n.clone_into(&mut name);
        }
    }
    ids.map(|(vendor, product)| (vendor, product, name))
}

/// Walks the short items of a HID report descriptor looking for usage page 0xFFF0 and report id 0x0F. Both are required before
/// anything is sent, so a node of some other interface never receives configuration reports.
pub fn has_config_reports(descriptor: &[u8]) -> bool {
    let (mut page, mut id) = (false, false);
    let mut i = 0;
    while let Some(&prefix) = descriptor.get(i) {
        if prefix == 0xFE {
            // Long item: size byte, tag byte, data.
            i += 3 + usize::from(*descriptor.get(i + 1).unwrap_or(&0));
            continue;
        }
        let size = match prefix & 0x03 {
            3 => 4,
            s => usize::from(s),
        };
        let Some(data) = descriptor.get(i + 1..i + 1 + size) else { break };
        let value = data.iter().rev().fold(0u32, |v, &b| v << 8 | u32::from(b));
        match prefix & 0xFC {
            0x04 => page |= value == u32::from(CONFIG_USAGE_PAGE),
            0x84 => id |= value == u32::from(REQUEST_ID),
            _ => {}
        }
        i += 1 + size;
    }
    page && id
}

/// Connect's queue resends an unanswered request after 200 ms and drops it after the tenth send.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub resend_after: Duration,
    pub max_sends: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Timing { resend_after: Duration::from_millis(200), max_sends: 10 }
    }
}

/// Connect repeats its heartbeat every second while the window is focused.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(1);

#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    Match,
    /// The first byte that read back differently from what was written.
    Mismatch {
        offset: usize,
        wanted: u8,
        found: u8,
    },
}

#[derive(Debug)]
pub struct Written {
    /// The write requests, sent or (on a dry run) not.
    pub reports: Vec<Report>,
    /// Write requests that drew no acknowledgement after every resend; the read-back still decides whether they stuck.
    pub unacked: usize,
    /// The read-back comparison; `None` when nothing was sent.
    pub check: Option<Check>,
}

pub struct Controller<T> {
    io: T,
    timing: Timing,
    last_heartbeat: Option<Instant>,
}

impl<T: Transport> Controller<T> {
    pub fn new(io: T) -> Self {
        Self::with_timing(io, Timing::default())
    }

    pub fn with_timing(io: T, timing: Timing) -> Self {
        Controller { io, timing, last_heartbeat: None }
    }

    pub fn into_inner(self) -> T {
        self.io
    }

    fn keep_alive(&mut self) -> io::Result<()> {
        if self.last_heartbeat.is_none_or(|at| at.elapsed() >= HEARTBEAT_EVERY) {
            self.io.send(&frame::heartbeat())?;
            self.last_heartbeat = Some(Instant::now());
        }
        Ok(())
    }

    /// Sends `request` until `accept` takes a reply. Gamepad reports and other replies arriving meanwhile are skipped.
    fn exchange<R>(&mut self, request: &Report, mut accept: impl FnMut(Response<'_>) -> Option<R>) -> Result<R, Error> {
        self.keep_alive()?;
        let mut buf = [0; REPORT_LEN];
        for _ in 0..self.timing.max_sends {
            self.io.send(request)?;
            let deadline = Instant::now() + self.timing.resend_after;
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                let Some(n) = self.io.recv(&mut buf, left)? else { continue };
                if let Some(found) = frame::parse(&buf[..n]).and_then(&mut accept) {
                    return Ok(found);
                }
            }
        }
        Err(Error::NoReply { command: request[1] })
    }

    pub fn firmware_version(&mut self) -> Result<FirmwareVersion, Error> {
        self.exchange(&frame::read_firmware_version(), |reply| match reply {
            Response::FirmwareVersion(version) => Some(version),
            _ => None,
        })
    }

    /// The active profile, 1-4. Connect treats a reported 0 as profile 1.
    pub fn current_profile(&mut self) -> Result<u8, Error> {
        self.exchange(&frame::read_current_profile(), |reply| match reply {
            Response::CurrentProfile(n) => Some(n.max(1)),
            _ => None,
        })
    }

    /// Makes `slot` the active profile and confirms it by asking which profile is active. Which reply acknowledges a switch is not
    /// confirmed on hardware, so silence is tolerated and the confirmation decides.
    pub fn switch_profile(&mut self, slot: u8) -> Result<(), Error> {
        let region = Region::profile(slot).ok_or(Error::Slot(slot))?;
        let acked = self.exchange(&frame::switch_profile(region), |reply| {
            matches!(
                reply,
                Response::Ack { busy: false } | Response::Other(cmd::SWITCH_PROFILE) | Response::ProfileChanged { region: frame::ACTIVE_PROFILE_NOTICE, .. }
            )
            .then_some(())
        });
        match acked {
            Ok(()) | Err(Error::NoReply { .. }) => {}
            Err(e) => return Err(e),
        }
        match self.current_profile()? {
            active if active == slot => Ok(()),
            active => Err(Error::Switch { wanted: slot, active }),
        }
    }

    pub fn read(&mut self, region: Region) -> Result<Vec<u8>, Error> {
        let mut out = vec![0; region.size()];
        for request in frame::read_plan(region) {
            let offset = u16::from_be_bytes([request[3], request[4]]);
            let len = usize::from(request[5]);
            self.exchange(&request, |reply| match reply {
                Response::Data { region: r, offset: o, data } if r == region.number() && o == offset && data.len() == len => {
                    out[usize::from(offset)..][..len].copy_from_slice(data);
                    Some(())
                }
                _ => None,
            })?;
        }
        Ok(out)
    }

    /// Writes the bytes that differ between `before` (what the controller holds) and `after`, then reads the region back and compares.
    /// A profile takes writes only while it is the active one (gamesir-linux-tools, on hardware; Connect switches before editing), so
    /// another profile is made active for the write and the previous one restored after the read-back.
    pub fn write(&mut self, region: Region, before: &[u8], after: &[u8], dry_run: bool) -> Result<Written, Error> {
        let reports = frame::write_plan(region, before, after)?;
        if dry_run || reports.is_empty() {
            return Ok(Written { reports, unacked: 0, check: None });
        }
        let restore = match region.slot() {
            Some(slot) => match self.current_profile()? {
                active if active == slot => None,
                active => {
                    self.switch_profile(slot)?;
                    Some(active)
                }
            },
            None => None,
        };
        let mut unacked = 0;
        for request in &reports {
            // A busy ack means the controller is still storing, so the request is resent. gamesir-linux-tools saw no reply to writes
            // at all, so silence after every resend is counted rather than fatal.
            match self.exchange(request, |reply| matches!(reply, Response::Ack { busy: false } | Response::Other(cmd::WRITE)).then_some(())) {
                Ok(()) => {}
                Err(Error::NoReply { .. }) => unacked += 1,
                Err(e) => return Err(e),
            }
        }
        let back = self.read(region)?;
        if let Some(active) = restore {
            self.switch_profile(active)?;
        }
        let check = match after.iter().zip(&back).position(|(a, b)| a != b) {
            None => Check::Match,
            Some(offset) => Check::Mismatch { offset, wanted: after[offset], found: back[offset] },
        };
        Ok(Written { reports, unacked, check: Some(check) })
    }

    /// Runs the motors at `left` and `right` strength (0-255) until the next call; 0, 0 stops them. Nothing is acknowledged.
    pub fn vibrate(&mut self, left: u8, right: u8) -> Result<(), Error> {
        self.keep_alive()?;
        self.io.send(&frame::vibrate(left, right))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::frame::{LIGHT_LEN, PROFILE_LEN, REPLY_ID, reply};

    /// An in-memory controller that answers like the firmware Connect expects.
    struct Fake {
        profile: Vec<u8>,
        light: Vec<u8>,
        sent: Vec<Report>,
        replies: VecDeque<Vec<u8>>,
        silent: bool,
        busy_writes: u32,
        /// Store writes but never acknowledge them.
        mute_writes: bool,
        ignore_switches: bool,
        /// As reported: 0 is how a fresh controller reports profile 1.
        active: u8,
        /// A byte the fake refuses to store, to provoke a read-back mismatch.
        stuck: Option<usize>,
    }

    impl Fake {
        fn new() -> Self {
            let profile = (0..PROFILE_LEN).map(|i| i as u8).collect();
            Fake {
                profile,
                light: vec![0; LIGHT_LEN],
                sent: Vec::new(),
                replies: VecDeque::new(),
                silent: false,
                busy_writes: 0,
                mute_writes: false,
                ignore_switches: false,
                active: 0,
                stuck: None,
            }
        }

        fn region(&mut self, n: u8) -> &mut Vec<u8> {
            if n == Region::LIGHT.number() { &mut self.light } else { &mut self.profile }
        }
    }

    impl Transport for Fake {
        fn send(&mut self, r: &Report) -> io::Result<()> {
            self.sent.push(*r);
            if self.silent {
                return Ok(());
            }
            // Gamepad state keeps streaming between replies.
            self.replies.push_back(vec![0x12, 0x80, 0x80, 0x80, 0x80]);
            let (offset, len) = (usize::from(u16::from_be_bytes([r[3], r[4]])), usize::from(r[5]));
            let answer = match r[1] {
                cmd::READ => [&[REPLY_ID, reply::DATA], &r[2..6], &self.region(r[2])[offset..offset + len]].concat(),
                cmd::WRITE if self.busy_writes > 0 => {
                    self.busy_writes -= 1;
                    vec![REPLY_ID, reply::ACK, 1]
                }
                cmd::WRITE => {
                    // As on hardware, a profile that is not the active one keeps its bytes.
                    let open = Region::profile(r[2]).is_none_or(|p| p.number() == self.active.max(1));
                    let stuck = self.stuck;
                    let stored = self.region(r[2]);
                    for (i, &b) in r[6..6 + len].iter().enumerate() {
                        if open && stuck != Some(offset + i) {
                            stored[offset + i] = b;
                        }
                    }
                    if self.mute_writes {
                        return Ok(());
                    }
                    vec![REPLY_ID, reply::ACK, 0]
                }
                cmd::SWITCH_PROFILE => {
                    if !self.ignore_switches {
                        self.active = r[2];
                    }
                    vec![REPLY_ID, cmd::SWITCH_PROFILE]
                }
                cmd::READ_FIRMWARE_VERSION => [&[REPLY_ID, reply::FIRMWARE_VERSION, 0, 0][..], b"123  ", &[0, 0, 0], b"121  "].concat(),
                cmd::READ_CURRENT_PROFILE => vec![REPLY_ID, reply::CURRENT_PROFILE, self.active],
                other => vec![REPLY_ID, other],
            };
            self.replies.push_back(answer);
            Ok(())
        }

        /// hidraw hands over whole reports, so replies are padded to the full length.
        fn recv(&mut self, buf: &mut [u8; REPORT_LEN], _: Duration) -> io::Result<Option<usize>> {
            Ok(self.replies.pop_front().map(|r| {
                *buf = [0; REPORT_LEN];
                buf[..r.len()].copy_from_slice(&r);
                REPORT_LEN
            }))
        }
    }

    fn quick(fake: Fake) -> Controller<Fake> {
        Controller::with_timing(fake, Timing { resend_after: Duration::from_millis(1), max_sends: 3 })
    }

    #[test]
    fn reads_a_region_and_sends_a_heartbeat_first() {
        let mut c = quick(Fake::new());
        let bytes = c.read(Region::profile(1).unwrap()).unwrap();
        let fake = c.into_inner();
        assert_eq!(bytes, fake.profile);
        assert_eq!(fake.sent[0][..3], [REQUEST_ID, cmd::HEARTBEAT, 0]);
        assert_eq!(fake.sent.len(), 1 + 12, "one heartbeat, then one request per chunk");
    }

    #[test]
    fn writes_the_difference_and_verifies_it() {
        let mut c = quick(Fake::new());
        let region = Region::profile(1).unwrap();
        let before = c.read(region).unwrap();
        let mut after = before.clone();
        after[46] = 2;

        let dry = c.write(region, &before, &after, true).unwrap();
        assert_eq!((dry.reports.len(), dry.check), (1, None));
        assert_eq!(c.read(region).unwrap(), before, "a dry run sends nothing");

        let done = c.write(region, &before, &after, false).unwrap();
        assert_eq!(done.check, Some(Check::Match));
        assert_eq!(c.into_inner().profile, after);
    }

    #[test]
    fn reports_bytes_that_did_not_stick() {
        let mut fake = Fake::new();
        fake.stuck = Some(46);
        let mut c = quick(fake);
        let region = Region::profile(1).unwrap();
        let before = c.read(region).unwrap();
        let mut after = before.clone();
        after[46] = 2;
        after[47] = 9;
        let done = c.write(region, &before, &after, false).unwrap();
        assert_eq!(done.check, Some(Check::Mismatch { offset: 46, wanted: 2, found: 46 }));
    }

    #[test]
    fn resends_while_busy_and_gives_up_on_silence() {
        let mut fake = Fake::new();
        fake.busy_writes = 2;
        let mut c = quick(fake);
        let region = Region::LIGHT;
        let before = vec![0; LIGHT_LEN];
        let mut after = before.clone();
        after[0] = 3;
        assert_eq!(c.write(region, &before, &after, false).unwrap().check, Some(Check::Match));
        let mut fake = c.into_inner();
        assert_eq!(fake.sent.iter().filter(|r| r[1] == cmd::WRITE).count(), 3, "two busy acks, then stored");

        fake.silent = true;
        fake.sent.clear();
        let mut c = quick(fake);
        assert!(matches!(c.firmware_version(), Err(Error::NoReply { command: cmd::READ_FIRMWARE_VERSION })));
        assert_eq!(c.into_inner().sent.len(), 1 + 3, "a heartbeat and three sends");
    }

    #[test]
    fn small_queries() {
        let mut c = quick(Fake::new());
        let version = c.firmware_version().unwrap();
        assert_eq!((version.controller.as_str(), version.receiver.as_str()), ("1.2.3", "1.2.1"));
        assert_eq!(c.current_profile().unwrap(), 1, "0 reads as profile 1");
        c.switch_profile(3).unwrap();
        assert!(matches!(c.switch_profile(5), Err(Error::Slot(5))));
        c.vibrate(255, 0).unwrap();
        let sent = c.into_inner().sent;
        assert_eq!(sent.iter().filter(|r| r[1] == cmd::HEARTBEAT).count(), 1, "one heartbeat within the first second");
        assert_eq!(sent.last().unwrap()[..6], [REQUEST_ID, cmd::VIBRATE, 0x66, 0x55, 255, 0]);
    }

    #[test]
    fn a_switch_is_confirmed_by_the_active_profile() {
        let mut fake = Fake::new();
        fake.ignore_switches = true;
        let mut c = quick(fake);
        assert!(matches!(c.switch_profile(2), Err(Error::Switch { wanted: 2, active: 1 })));
    }

    #[test]
    fn writes_another_profile_while_it_is_active_then_switches_back() {
        let mut c = quick(Fake::new());
        let region = Region::profile(2).unwrap();
        let before = c.read(region).unwrap();
        let mut after = before.clone();
        after[46] = 2;
        assert_eq!(c.write(region, &before, &after, false).unwrap().check, Some(Check::Match));
        let fake = c.into_inner();
        let order: Vec<(u8, u8)> = fake.sent.iter().filter(|r| matches!(r[1], cmd::SWITCH_PROFILE | cmd::WRITE)).map(|r| (r[1], r[2])).collect();
        assert_eq!(order, [(cmd::SWITCH_PROFILE, 2), (cmd::WRITE, 2), (cmd::SWITCH_PROFILE, 1)]);
        assert_eq!(fake.active, 1);
    }

    #[test]
    fn unacknowledged_writes_are_judged_by_the_read_back() {
        let mut fake = Fake::new();
        fake.mute_writes = true;
        let mut c = quick(fake);
        let before = vec![0; LIGHT_LEN];
        let mut after = before.clone();
        after[0] = 3;
        let done = c.write(Region::LIGHT, &before, &after, false).unwrap();
        assert_eq!((done.unacked, done.check), (1, Some(Check::Match)));
        assert_eq!(c.into_inner().sent.iter().filter(|r| r[1] == cmd::WRITE).count(), 3, "sent until the last try");
    }

    #[test]
    fn reads_ids_from_uevent() {
        let uevent = "DRIVER=hid-generic\nHID_ID=0003:00003537:0000102A\nHID_NAME=GameSir CYCLONE 2\nHID_PHYS=usb-1/input2\n";
        assert_eq!(parse_uevent(uevent), Some((0x3537, 0x102A, "GameSir CYCLONE 2".into())));
        assert_eq!(parse_uevent("HID_NAME=x\n"), None);
    }

    #[test]
    fn config_interface_needs_vendor_page_and_report_id() {
        // Usage Page (0xFFF0), Usage (1), Collection, Report ID (0x0F), Report Count (63), Report Size (8), Input, End Collection.
        let vendor = [0x06, 0xF0, 0xFF, 0x09, 0x01, 0xA1, 0x01, 0x85, 0x0F, 0x95, 0x3F, 0x75, 0x08, 0x81, 0x02, 0xC0];
        assert!(has_config_reports(&vendor));
        // A gamepad interface: Generic Desktop page, report id 0x12.
        let gamepad = [0x05, 0x01, 0x09, 0x05, 0xA1, 0x01, 0x85, 0x12, 0xC0];
        assert!(!has_config_reports(&gamepad));
        assert!(!has_config_reports(&vendor[..7]), "a truncated descriptor without the report id");
    }

    #[test]
    fn discovers_nodes_from_sysfs() {
        let root = std::env::temp_dir().join(format!("gamesir-config-test-{}", std::process::id()));
        let node = |name: &str, uevent: &str, descriptor: &[u8]| {
            let dir = root.join(name).join("device");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("uevent"), uevent).unwrap();
            fs::write(dir.join("report_descriptor"), descriptor).unwrap();
        };
        node("hidraw0", "HID_ID=0003:0000054C:000009CC\nHID_NAME=Guangzhou Chicken Run Network Technology Co., Ltd. Gamepad\n", &[]);
        node("hidraw1", "HID_ID=0003:00003537:0000102A\nHID_NAME=GameSir CYCLONE 2\n", &[0x06, 0xF0, 0xFF, 0x85, 0x0F]);
        node("hidraw2", "HID_ID=0003:0000046D:0000C52B\nHID_NAME=Logitech USB Receiver\n", &[]);
        let found = discover_in(&root, Path::new("/dev")).unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(found.len(), 2);
        assert_eq!((found[0].model, found[0].config), (None, false));
        assert_eq!((found[1].node.as_path(), found[1].model, found[1].config), (Path::new("/dev/hidraw1"), Some((Model::Cyclone2, Link::Receiver)), true));
    }
}
