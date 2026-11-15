//! The panel's side of LumaGame's stats overlay: the video window's status, the refresh counters and the DE firmware's status words.
//! Sampled only while LumaGame keeps asking (its overlay is up over a running stream); nothing here writes to a device.
#![allow(unsafe_code)]

use super::device::{LgDevices, ioctl};
use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// SYS_VERI_10 and SYS_VERI_11, the DE firmware's status words (field positions from its own status print).
const SYS_VERI_10: u32 = 0xCC48_2228;
const SYS_VERI_11: u32 = 0xCC48_222C;
/// `DE_IOR_REG_RD`. `DE_IO_RegRd` takes the register's value for an error code: a nonzero register fails with EIO and its value
/// is printed to the kernel log under the reader's pid and tid, which is where it is read from. Each such read adds a kernel log
/// line, so the words are read every five seconds, again when the window changes, and only for the full panel.
const DE_IOR_REG_RD: u32 = 0x8004_6164;
const REGISTERS_EVERY: Duration = Duration::from_secs(5);
/// Refreshes are counted per 100 ms: at 120 Hz a window holds 12, so one held refresh shows as 11.
const WINDOW: Duration = Duration::from_millis(100);
const WINDOWS: usize = 10;
const STATUS_EVERY: Duration = Duration::from_millis(250);
/// The sampler stops this long after the last ask.
const IDLE: Duration = Duration::from_secs(2);
/// The DE's output refresh; the graphics plane's `osd_irq` counts the same where it is missing.
const IRQS: [&str; 2] = ["DE2VSYNC", "osd_irq"];

pub trait Source: Send {
    /// `/proc/lg/de/win0/status` as text.
    fn status(&mut self) -> io::Result<String>;
    /// The refresh interrupt's name and its count over every CPU.
    fn refreshes(&mut self) -> io::Result<(&'static str, u64)>;
    /// One DE register, through the read-only ioctl.
    fn register(&mut self, address: u32) -> io::Result<u32>;
}

/// The window's status lines (`localmute=0`, `h/v active : 3840 / 2160`, `vFreq : 1200`) as the overlay shows them. `vFreq` is the
/// source's frame rate in 0.1 Hz, not the panel's refresh.
fn window(text: &str) -> Value {
    let mut out = Map::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=').or_else(|| line.split_once(':')) else { continue };
        let key = key.trim();
        if key == "h/v active" {
            let mut sides = value.split('/').map(|s| s.trim().parse::<u32>().ok());
            if let (Some(Some(w)), Some(Some(h))) = (sides.next(), sides.next()) {
                out.insert("width".into(), json!(w));
                out.insert("height".into(), json!(h));
            }
            continue;
        }
        let Ok(n) = value.trim().parse::<i64>() else { continue };
        let (name, value) = match key {
            "localmute" => ("muted", json!(n != 0)),
            "lowdelaymode" => ("lowDelay", json!(n != 0)),
            "vFreq" => ("sourceHz", json!(n as f64 / 10.0)),
            "hdr_type" => ("hdrType", json!(n)),
            _ => continue,
        };
        out.insert(name.into(), value);
    }
    Value::Object(out)
}

fn bit(value: u32, n: u32) -> bool {
    value >> n & 1 != 0
}

fn veri10(v: u32) -> Value {
    json!({"videoDelayUs": v & 0xF_FFFF, "picInit": bit(v, 22), "vrr": bit(v, 23), "justScan": bit(v, 24), "frc": bit(v, 25),
           "frameDelay": v >> 26 & 3, "lowDelay": bit(v, 30), "dcoLock": bit(v, 31)})
}

/// Bits 31:16 read 11862 while 120 Hz arrived (2026-11-15): the firmware's measured input rate in 0.01 Hz (inferred).
fn veri11(v: u32) -> Value {
    json!({"rateIn": v & 0xFF, "rateOut": v >> 8 & 0xFF, "measuredHz": f64::from(v >> 16) / 100.0})
}

/// The value `DE_IO_RegRd` printed for this thread: `de ERR (03962:03970) DE_IO_RegRd 1197: Wrong ret(0x4c000)`.
fn logged_value(record: &str, tag: &str) -> Option<u32> {
    if !record.contains(tag) || !record.contains("DE_IO_RegRd") {
        return None;
    }
    let hex = record.split("Wrong ret(0x").nth(1)?.split(')').next()?;
    u32::from_str_radix(hex, 16).ok()
}

/// The CPU count from the header of `/proc/interrupts`, then the line of the first of [`IRQS`] there is: its number and its count.
fn irq_line(text: &str) -> Option<(&'static str, u32, u64)> {
    let mut lines = text.lines();
    let cpus = lines.next()?.split_whitespace().filter(|f| f.starts_with("CPU")).count();
    let mut found: Option<(usize, &'static str, u32, u64)> = None;
    for line in lines {
        let Some(name) = line.split_whitespace().next_back() else { continue };
        let Some(rank) = IRQS.iter().position(|n| *n == name) else { continue };
        if found.is_some_and(|(r, ..)| r <= rank) {
            continue;
        }
        let mut fields = line.split_whitespace();
        let Some(number) = fields.next().and_then(|n| n.strip_suffix(':')).and_then(|n| n.parse().ok()) else { continue };
        let counts: Option<Vec<u64>> = fields.take(cpus).map(|c| c.parse().ok()).collect();
        if let Some(counts) = counts.filter(|c| c.len() == cpus) {
            found = Some((rank, IRQS[rank], number, counts.iter().sum()));
        }
    }
    found.map(|(_, name, number, count)| (name, number, count))
}

pub struct LgSource {
    devices: LgDevices,
    status: PathBuf,
    interrupts: PathBuf,
    irqs: PathBuf,
    /// The refresh interrupt once found, and its per-CPU counts in sysfs when the kernel has them (a much smaller read).
    irq: Option<(&'static str, u32, Option<PathBuf>)>,
    de: Option<File>,
    kmsg: Option<File>,
    tag: String,
}

impl LgSource {
    pub fn new() -> Self {
        Self {
            devices: LgDevices::default(),
            status: "/proc/lg/de/win0/status".into(),
            interrupts: "/proc/interrupts".into(),
            irqs: "/sys/kernel/irq".into(),
            irq: None,
            de: None,
            kmsg: None,
            // SAFETY: gettid takes nothing and cannot fail; the tag is this sampler thread's for its whole run.
            tag: format!("({:05}:{:05})", std::process::id(), unsafe { libc::syscall(libc::SYS_gettid) }),
        }
    }

    fn logged(&mut self) -> io::Result<u32> {
        let kmsg = self.kmsg.as_mut().ok_or_else(|| io::Error::other("no kernel log cursor"))?;
        let mut record = [0u8; 1024];
        let mut value = None;
        loop {
            match kmsg.read(&mut record) {
                Ok(0) => break,
                Ok(n) => value = logged_value(&String::from_utf8_lossy(&record[..n]), &self.tag).or(value),
                // the cursor fell behind the ring: the records it missed are gone, the next ones still come
                Err(e) if e.raw_os_error() == Some(libc::EPIPE) => continue,
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => break,
                Err(e) => return Err(e),
            }
        }
        value.ok_or_else(|| io::Error::other("the register's value was not in the kernel log"))
    }
}

impl Source for LgSource {
    fn status(&mut self) -> io::Result<String> {
        fs::read_to_string(&self.status)
    }

    fn refreshes(&mut self) -> io::Result<(&'static str, u64)> {
        if let Some((name, _, Some(path))) = &self.irq {
            let text = fs::read_to_string(path)?;
            let total = text.trim().split(',').map(|c| c.trim().parse::<u64>().map_err(|_| io::Error::other("bad per_cpu_count"))).sum::<io::Result<u64>>()?;
            return Ok((name, total));
        }
        let text = fs::read_to_string(&self.interrupts)?;
        let (name, number, count) = irq_line(&text).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no refresh interrupt"))?;
        if self.irq.is_none() {
            let per_cpu = self.irqs.join(number.to_string()).join("per_cpu_count");
            self.irq = Some((name, number, per_cpu.exists().then_some(per_cpu)));
        }
        Ok((name, count))
    }

    fn register(&mut self, address: u32) -> io::Result<u32> {
        if self.de.is_none() {
            self.de = Some(self.devices.de_reader()?);
        }
        if self.kmsg.is_none() {
            let mut kmsg = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open("/dev/kmsg")?;
            // SEEK_END moves only this descriptor's cursor, never the syslog reader's.
            kmsg.seek(SeekFrom::End(0))?;
            self.kmsg = Some(kmsg);
        }
        let mut value = address;
        match ioctl(self.de.as_ref().expect("opened above"), DE_IOR_REG_RD, &mut value) {
            Ok(()) => Ok(value),
            Err(e) if e.raw_os_error() == Some(libc::EIO) => self.logged(),
            Err(e) => Err(e),
        }
    }
}

/// The refresh counts of the last second, per window.
#[derive(Default)]
struct Refresh {
    irq: &'static str,
    last: Option<(u64, Instant)>,
    windows: VecDeque<(u64, Duration)>,
}

impl Refresh {
    fn push(&mut self, irq: &'static str, count: u64, at: Instant) {
        if irq != self.irq {
            *self = Self { irq, ..Self::default() };
        }
        if let Some((before, then)) = self.last {
            let span = at.saturating_duration_since(then);
            if count < before {
                self.windows.clear();
            } else if !span.is_zero() {
                self.windows.push_back((count - before, span));
                if self.windows.len() > WINDOWS {
                    self.windows.pop_front();
                }
            }
        }
        self.last = Some((count, at));
    }

    fn json(&self) -> Value {
        let tenth = |hz: f64| (hz * 10.0).round() / 10.0;
        let rate = |n: u64, d: Duration| n as f64 / d.as_secs_f64();
        let (Some(fewest), Some(slowest)) = (self.windows.iter().map(|w| w.0).min(), self.windows.iter().map(|&(n, d)| rate(n, d)).reduce(f64::min)) else {
            return Value::Null;
        };
        let (count, span) = self.windows.iter().fold((0, Duration::ZERO), |(n, d), w| (n + w.0, d + w.1));
        json!({"irq": self.irq, "hz": tenth(rate(count, span)), "minHz": tenth(slowest), "fewest": fewest, "windowMs": WINDOW.as_millis() as u64,
               "windows": self.windows.iter().map(|&(n, d)| tenth(rate(n, d))).collect::<Vec<_>>()})
    }
}

#[derive(Default)]
struct Ctl {
    asked: Option<Instant>,
    full: bool,
    running: bool,
}

struct Inner {
    make: Box<dyn Fn() -> Box<dyn Source> + Send + Sync>,
    ctl: Mutex<Ctl>,
    latest: Mutex<Value>,
}

pub struct Display {
    inner: Arc<Inner>,
}

impl Display {
    pub fn new(make: Box<dyn Fn() -> Box<dyn Source> + Send + Sync>) -> Self {
        Self { inner: Arc::new(Inner { make, ctl: Mutex::new(Ctl::default()), latest: Mutex::new(Value::Null) }) }
    }

    pub fn lg() -> Self {
        Self::new(Box::new(|| Box::new(LgSource::new())))
    }

    /// LumaGame's ask: the last readings (`null` until the first are in), and the sampler kept going or started. `full` adds the
    /// firmware's status words.
    pub fn ask(&self, full: bool) -> Value {
        let mut ctl = self.inner.ctl.lock();
        ctl.asked = Some(Instant::now());
        ctl.full = full;
        if !ctl.running {
            let inner = self.inner.clone();
            ctl.running = thread::Builder::new().name("gaming-display".into()).stack_size(128 * 1024).spawn(move || sample(&inner)).is_ok();
        }
        drop(ctl);
        self.inner.latest.lock().clone()
    }

    /// The plugin went off: the sampler ends at its next window.
    pub fn stop(&self) {
        self.inner.ctl.lock().asked = None;
    }
}

fn sample(inner: &Inner) {
    let mut source = (inner.make)();
    let mut refresh = Refresh::default();
    let (mut status_at, mut registers_at): (Option<Instant>, Option<Instant>) = (None, None);
    let (mut text, mut win, mut firmware) = (String::new(), Value::Null, Value::Null);
    let mut errors = Map::new();
    loop {
        let full = {
            let mut ctl = inner.ctl.lock();
            if ctl.asked.is_none_or(|at| at.elapsed() > IDLE) {
                ctl.running = false;
                *inner.latest.lock() = Value::Null;
                return;
            }
            ctl.full
        };
        let now = Instant::now();
        match source.refreshes() {
            Ok((irq, count)) => {
                refresh.push(irq, count, now);
                errors.remove("refresh");
            }
            Err(e) => {
                refresh = Refresh::default();
                errors.insert("refresh".into(), json!(e.to_string()));
            }
        }
        if status_at.is_none_or(|at| now.duration_since(at) >= STATUS_EVERY) {
            status_at = Some(now);
            match source.status() {
                Ok(t) => {
                    if t != text {
                        registers_at = None;
                        win = window(&t);
                        text = t;
                    }
                    errors.remove("status");
                }
                Err(e) => {
                    win = Value::Null;
                    errors.insert("status".into(), json!(e.to_string()));
                }
            }
        }
        if !full {
            firmware = Value::Null;
            registers_at = None;
        } else if registers_at.is_none_or(|at| now.duration_since(at) >= REGISTERS_EVERY) {
            registers_at = Some(now);
            match (source.register(SYS_VERI_10), source.register(SYS_VERI_11)) {
                (Ok(a), Ok(b)) => {
                    firmware = json!({"sysVeri10": veri10(a), "sysVeri11": veri11(b)});
                    errors.remove("registers");
                }
                (Err(e), _) | (_, Err(e)) => {
                    firmware = Value::Null;
                    errors.insert("registers".into(), json!(e.to_string()));
                }
            }
        }
        *inner.latest.lock() = json!({"window": win, "refresh": refresh.json(), "firmware": firmware, "errors": errors});
        thread::sleep((now + WINDOW).saturating_duration_since(Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_status_reads_as_the_tv_prints_it() {
        let text = "localmute=0\nlowdelaymode=1\nh/v active : 3840 / 2160\nscanType   : 1\nvFreq      : 1200\nsvp        : 0\nhdr_type   : 1\n";
        assert_eq!(window(text), json!({"muted": false, "lowDelay": true, "width": 3840, "height": 2160, "sourceHz": 120.0, "hdrType": 1}));
        assert_eq!(window("garbage\n"), json!({}));
    }

    #[test]
    fn the_status_words_split_into_the_firmwares_fields() {
        // as read during 4K120 HDR play on 2026-11-15
        let a = veri10(0xF100_3E01);
        assert_eq!((a["videoDelayUs"].as_u64(), a["lowDelay"].as_bool(), a["vrr"].as_bool(), a["frc"].as_bool()), (Some(15873), Some(true), Some(false), Some(false)));
        assert_eq!((a["frameDelay"].as_u64(), a["justScan"].as_bool(), a["dcoLock"].as_bool()), (Some(0), Some(true), Some(true)));
        assert_eq!(veri11(0x2E56_7878), json!({"rateIn": 120, "rateOut": 120, "measuredHz": 118.62}));
    }

    #[test]
    fn a_registers_value_is_taken_only_from_this_threads_line() {
        let line = "3,4417,18949149571,-;de           ERR (03962:03970) DE_IO_RegRd              1197: Wrong ret(0x4c000)";
        assert_eq!(logged_value(line, "(03962:03970)"), Some(0x4c000));
        assert_eq!(logged_value(line, "(03962:03962)"), None, "another thread's read");
        assert_eq!(logged_value("6,1,1,-;(03962:03970) something else", "(03962:03970)"), None);
    }

    #[test]
    fn the_refresh_interrupt_is_found_and_summed_over_cpus() {
        let text = "           CPU0       CPU1\n 45:        10         20     GICv2  77 Level     osd_irq\n 46:       100        200     GICv2  78 Level     DE2VSYNC\n";
        assert_eq!(irq_line(text), Some(("DE2VSYNC", 46, 300)));
        assert_eq!(irq_line("  CPU0\n 45: 7 GICv2 osd_irq\n"), Some(("osd_irq", 45, 7)));
        assert_eq!(irq_line("  CPU0\n 45: 7 GICv2 osd_irq_extra\n"), None);
    }

    #[test]
    fn refresh_is_the_average_and_the_slowest_window_of_the_last_second() {
        let t = Instant::now();
        let mut r = Refresh::default();
        assert_eq!(r.json(), Value::Null);
        let mut count = 0;
        r.push("DE2VSYNC", count, t);
        for (i, n) in [12, 12, 11, 12].into_iter().enumerate() {
            count += n;
            r.push("DE2VSYNC", count, t + WINDOW * (i as u32 + 1));
        }
        let j = r.json();
        assert_eq!((j["hz"].as_f64(), j["minHz"].as_f64(), j["fewest"].as_u64()), (Some(117.5), Some(110.0), Some(11)));
        assert_eq!(j["windows"], json!([120.0, 120.0, 110.0, 120.0]));
        r.push("DE2VSYNC", 0, t + WINDOW * 6);
        assert_eq!(r.json(), Value::Null, "a counter that went back starts over");
    }
}
