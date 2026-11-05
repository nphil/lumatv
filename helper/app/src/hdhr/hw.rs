//! The TV's side of the facade, as measured on the set (webOS 10.2.1, kernel 5.4.268; `docs/research/live-tv-capture.md`):
//!
//! - LG's `utp` frees a broadcast pipeline when the connection that opened it goes, so the opener is a `luna-send -i` that stays up for the
//!   stream; `changeChannel` on that pipeline (not `setAtscManualTuning`, which leaves the frontend where it was) moves the frontend and the
//!   TV's last channel, and the last channel is put back at the end.
//! - The stream is the kernel's own: a second demux channel (`demux2`) takes the demod's port with the values LG's stack uses, section
//!   filters read the PSI, and a PVR download (`dvr2`) carries the routed PIDs as 192-byte units (a 4-byte timestamp, then the packet) once
//!   the download has been certified. The certificate is LG's documented one: the device's seed key (a call into LG's HAL, made in the
//!   set's 32-bit python because this binary is 64-bit) xor a constant table. The seed is kept in memory and never written or logged.

use super::psi::{self, UNIT};
use super::stream::{Failure, Hardware, Tap};
use crate::config::HdhrConfig;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

const BROADCAST: &str = "luna://com.webos.service.utp.broadcast";
const LIVE_TV: &str = "com.webos.app.livetv";
/// How long a luna call may take.
const CALL_WAIT: Duration = Duration::from_secs(10);
/// How long the frontend gets to lock after `changeChannel`, and to settle on the new multiplex (the previous one's tables are still on
/// the demux for a moment).
const LOCK_WAIT: Duration = Duration::from_secs(10);

// The ioctls, computed from LG's headers (`include/linux/dvbv5-ext`, `linux/dvb/dmx.h`) and used as measured on the set.
const DMX_SET_FILTER: i32 = 0x403c_6f2b;
const DMX_SET_PES_FILTER: i32 = 0x4014_6f2c;
const DMX_EXT_S_CTL: i32 = 0x4010_6f35;
const PVR_EXT_S_CTL: i32 = 0x4010_7000;
const CID_CERT: u32 = 3;
const CID_INPUTSOURCE: u32 = 7;
const SRC_IN_DEMOD: u32 = 0;
const SRC_NULL: u32 = 5;
const PORT_PARALLEL: u32 = 1;
const PORT_NONE: u32 = 3;
const CHECK_CRC: u32 = 1;
const ONESHOT: u32 = 2;
const IMMEDIATE_START: u32 = 4;
const OUT_TS_TAP: u32 = 2;
const PES_OTHER: u32 = 20;
/// `randTbl` of `dvbv5-ext-pvr.h`: the cert is `seed[i % 16] ^ CERT_TABLE[i]`.
const CERT_TABLE: [u8; 32] = [
    0x6a, 0xb0, 0xad, 0x8c, 0x4e, 0x69, 0x40, 0x2c, 0x58, 0x2e, 0x1e, 0xc0, 0xf5, 0xbe, 0xc9, 0x83,
    0x21, 0xe1, 0x07, 0x46, 0x5b, 0xa9, 0x7c, 0xaa, 0x9b, 0x0f, 0x2b, 0x25, 0x80, 0x88, 0x3e, 0x6b,
];
/// The seed key from LG's HAL, which only a 32-bit process can load: `<return code> <16 bytes in hex>`.
const KEY_SCRIPT: &str = "import ctypes,sys\nl=ctypes.CDLL('libhal_lg115x.so.2')\nf=l.HAL_PVR_CRYPTO_GetSecureKey\nf.restype=ctypes.c_int\nf.argtypes=[ctypes.c_void_p,ctypes.c_void_p]\nb=(ctypes.c_ubyte*256)(*[165]*256)\nr=f(None,ctypes.addressof(b))\nsys.stdout.write('%d %s'%(r,bytes(b[:16]).hex()))";

#[repr(C)]
struct ExtControl {
    id: u32,
    size: u32,
    value: u64,
}

#[repr(C)]
struct ExtSource {
    src: u32,
    port: u32,
    port_type: u32,
}

#[repr(C)]
struct SectionFilter {
    pid: u16,
    filter: [u8; 16],
    mask: [u8; 16],
    mode: [u8; 16],
    timeout: u32,
    flags: u32,
}

#[repr(C)]
struct PesFilter {
    pid: u16,
    input: u32,
    output: u32,
    pes_type: u32,
    flags: u32,
}

const _: () = assert!(
    size_of::<ExtControl>() == 16
        && size_of::<ExtSource>() == 12
        && size_of::<SectionFilter>() == 60
        && size_of::<PesFilter>() == 20
);

fn hw(what: impl std::fmt::Display) -> Failure {
    Failure::Hardware(what.to_string())
}

fn ioctl<T>(file: &File, request: i32, arg: &mut T) -> std::io::Result<()> {
    // SAFETY: `arg` is a live, exclusively borrowed `repr(C)` struct of the size `request` encodes (asserted above), and the fd is open.
    let r = unsafe { libc::ioctl(file.as_raw_fd(), request as _, std::ptr::from_mut(arg)) };
    if r < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Waits until `file` can be read, at most `wait`.
fn readable(file: &File, wait: Duration) -> bool {
    let mut p = libc::pollfd {
        fd: file.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd, count 1.
    unsafe { libc::poll(&mut p, 1, wait.as_millis().min(i32::MAX as u128) as i32) > 0 }
}

/// The certificate for the PVR download.
fn cert(seed: &[u8; 16]) -> [u8; 32] {
    std::array::from_fn(|i| seed[i % 16] ^ CERT_TABLE[i])
}

fn first_object(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    serde_json::Deserializer::from_str(&text[start..])
        .into_iter::<Value>()
        .next()?
        .ok()
}

/// Runs `command` and returns what it wrote, killing it after `wait`.
fn run(command: &mut Command, wait: Duration) -> Result<String, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run it: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("no output")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        let _ = tx.send(text);
    });
    match rx.recv_timeout(wait) {
        Ok(text) => {
            let _ = child.wait();
            Ok(text)
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!("no answer in {} s", wait.as_secs()))
        }
    }
}

/// A pipeline's opener: the `luna-send` whose connection keeps it alive. Dropping it kills the process and the pipeline goes with it.
struct Holder {
    child: Child,
    _stdin: ChildStdin,
    id: String,
}

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The TV, through luna and the DVB nodes.
pub struct Tv {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: HdhrConfig,
    luna_send: String,
    seed: Mutex<Option<[u8; 16]>>,
}

impl Tv {
    /// `luna_send` is the helper's `luna.send` program.
    pub fn new(cfg: HdhrConfig, luna_send: String) -> Self {
        Self {
            inner: Arc::new(Inner {
                cfg,
                luna_send,
                seed: Mutex::new(None),
            }),
        }
    }
}

impl Inner {
    fn luna(&self, uri: &str, payload: &Value) -> Result<Value, Failure> {
        let text = run(
            Command::new(&self.luna_send).args(["-n", "1", uri, &payload.to_string()]),
            CALL_WAIT,
        )
        .map_err(|e| hw(format!("luna-send {uri}: {e}")))?;
        let reply =
            first_object(&text).ok_or_else(|| hw(format!("luna-send {uri} gave no answer")))?;
        if reply["returnValue"].as_bool() == Some(true) {
            Ok(reply)
        } else {
            Err(hw(format!(
                "{uri}: {}",
                reply["errorText"].as_str().unwrap_or("refused")
            )))
        }
    }

    /// Opens the windowless scan pipeline from a connection that stays up.
    fn open_pipeline(&self) -> Result<Holder, Failure> {
        let payload = json!({"options": {"appId": "com.webos.app.channelsetting", "windowId": "", "purpose": "SCAN"}}).to_string();
        let uri = format!("{BROADCAST}/open");
        let mut child = Command::new(&self.luna_send)
            .args(["-i", &uri, &payload])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| hw(format!("could not run luna-send: {e}")))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(hw("luna-send has no pipes"));
        };
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut reader, mut text, mut line) =
                (BufReader::new(stdout), String::new(), String::new());
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                text.push_str(&line);
                if let Some(reply) = first_object(&text) {
                    let _ = tx.send(reply);
                    text.clear();
                }
            }
        });
        let mut holder = Holder {
            child,
            _stdin: stdin,
            id: String::new(),
        };
        let reply = rx
            .recv_timeout(CALL_WAIT)
            .map_err(|_| hw(format!("{uri}: no answer")))?;
        holder.id = match (
            reply["returnValue"].as_bool(),
            reply["broadcastId"].as_str(),
        ) {
            (Some(true), Some(id)) => id.to_owned(),
            _ => {
                return Err(hw(format!(
                    "{uri}: {}",
                    reply["errorText"].as_str().unwrap_or("refused")
                )));
            }
        };
        Ok(holder)
    }

    fn change_channel(&self, id: &str, channel_id: &str) -> Result<(), Failure> {
        self.luna(
            &format!("{BROADCAST}/changeChannel"),
            &json!({"broadcastId": id, "channelId": channel_id}),
        )
        .map(|_| ())
    }

    fn wait_lock(&self, id: &str) -> Result<(), Failure> {
        let end = Instant::now() + LOCK_WAIT;
        loop {
            let status = self.luna(
                &format!("{BROADCAST}/getSignalStatus"),
                &json!({"broadcastId": id}),
            )?;
            if status["signal"]["locked"].as_bool() == Some(true) {
                return Ok(());
            }
            if Instant::now() >= end {
                return Err(Failure::NoSignal("the tuner did not lock".to_owned()));
            }
            thread::sleep(Duration::from_millis(500));
        }
    }

    /// UMS's pipelines (`getActivePipelines`): a bare array of objects, one with `data: "pipeline empty"` when there are none.
    fn pipelines(&self) -> Result<Vec<Value>, Failure> {
        let uri = "luna://com.webos.media/getActivePipelines";
        let text = run(
            Command::new(&self.luna_send).args(["-n", "1", uri, "{}"]),
            CALL_WAIT,
        )
        .map_err(|e| hw(format!("luna-send {uri}: {e}")))?;
        match serde_json::from_str::<Value>(text.trim()) {
            Ok(Value::Array(list)) => Ok(list),
            Ok(reply) if reply["returnValue"].as_bool() == Some(true) => Ok(Vec::new()),
            _ => Err(hw(format!("luna-send {uri} gave no list"))),
        }
    }

    fn last_channel(&self) -> Result<String, Failure> {
        let reply = self.luna(
            "luna://com.webos.service.utp.channel/getLastChannelId",
            &json!({"sourceType": "INPUTSOURCE_RF"}),
        )?;
        reply["channelId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| hw("no last channel"))
    }

    /// The seed key, fetched once per run of the helper.
    fn seed(&self) -> Result<[u8; 16], Failure> {
        if let Some(seed) = *self.seed.lock() {
            return Ok(seed);
        }
        let text = run(
            Command::new(&self.cfg.python).arg("-c").arg(KEY_SCRIPT),
            Duration::from_secs(20),
        )
        .map_err(|e| hw(format!("the seed key: {e}")))?;
        let (code, hex) = text
            .trim()
            .split_once(' ')
            .ok_or_else(|| hw("the seed key: no answer"))?;
        let mut seed = [0u8; 16];
        let ok = code == "0" && hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_hexdigit());
        if !ok {
            return Err(hw("the seed key: LG's HAL refused"));
        }
        for (i, byte) in seed.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)
                .map_err(|_| hw("the seed key: not hex"))?;
        }
        if seed == [0; 16] {
            return Err(hw("the seed key is all zero"));
        }
        *self.seed.lock() = Some(seed);
        Ok(seed)
    }
}

impl Hardware for Tv {
    fn channel_list(&self) -> Result<Value, Failure> {
        self.inner.luna(
            "luna://com.webos.service.iepg/getChannelList",
            &json!({"dataType": 0, "sort": 0, "channelMode": ["Tuner"], "channelGroup": ["All"], "requestField": ["channelId", "channelNumber", "channelName", "skipped", "Invisible"]}),
        )
    }

    fn in_use(&self) -> Option<String> {
        let front = match self.inner.luna(
            "luna://com.webos.applicationManager/getForegroundAppInfo",
            &json!({}),
        ) {
            Ok(front) => front,
            Err(f) => return Some(f.reason()),
        };
        if front["appId"].as_str() == Some(LIVE_TV) {
            return Some("Live TV is in front".to_owned());
        }
        // Live TV's own pipeline (it unrealizes the helper's, and runs with the panel off too)
        match self.inner.pipelines() {
            Ok(list)
                if list
                    .iter()
                    .any(|p| p["currentAppId"].as_str() == Some(LIVE_TV)) =>
            {
                Some("Live TV is running".to_owned())
            }
            Ok(_) => None,
            Err(f) => Some(f.reason()),
        }
    }

    fn tune(&self, channel_id: &str) -> Result<Box<dyn Tap>, Failure> {
        let inner = self.inner.clone();
        let seed = inner.seed()?;
        let previous = inner.last_channel()?;
        let holder = inner.open_pipeline()?;
        // from here the TV is put back however this ends: `Tuned` is the guard
        let mut tuned = Tuned {
            inner: inner.clone(),
            holder: Some(holder),
            previous,
            moved: false,
            dvb: None,
        };
        let id = tuned
            .holder
            .as_ref()
            .map(|h| h.id.clone())
            .unwrap_or_default();
        inner.change_channel(&id, channel_id)?;
        tuned.moved = true;
        inner.wait_lock(&id)?;
        thread::sleep(Duration::from_millis(inner.cfg.settle_ms));
        tuned.dvb = Some(Dvb::open(&inner.cfg, seed)?);
        Ok(Box::new(tuned))
    }
}

/// A tuned antenna: the pipeline, the channel it moved and the demux tap. Dropping it gives the TV back, in the order that cannot overflow
/// the download: the tap first, then the channel, then the pipeline.
struct Tuned {
    inner: Arc<Inner>,
    holder: Option<Holder>,
    previous: String,
    moved: bool,
    dvb: Option<Dvb>,
}

impl Drop for Tuned {
    fn drop(&mut self) {
        self.dvb = None;
        if let Some(holder) = self.holder.take() {
            if self.moved {
                let _ = self.inner.change_channel(&holder.id, &self.previous);
            }
            let _ = self.inner.luna(
                &format!("{BROADCAST}/stop"),
                &json!({"broadcastId": holder.id}),
            );
            let _ = self.inner.luna(
                &format!("{BROADCAST}/close"),
                &json!({"broadcastId": holder.id}),
            );
        }
    }
}

impl Tap for Tuned {
    fn section(&mut self, pid: u16, table_id: u8, number: u8, wait: Duration) -> Option<Vec<u8>> {
        self.dvb.as_mut()?.section(pid, table_id, number, wait)
    }

    fn start(&mut self, pids: &[u16]) -> Result<(), Failure> {
        self.dvb.as_mut().ok_or_else(|| hw("no demux"))?.start(pids)
    }

    fn taken(&mut self) -> Option<String> {
        let id = &self.holder.as_ref()?.id;
        match self.inner.luna(
            &format!("{BROADCAST}/getBroadcastState"),
            &json!({"broadcastId": id}),
        ) {
            Ok(state) if state["state"].as_str() == Some("BROADCAST_STATE_RENDERING") => None,
            Ok(state) => Some(format!(
                "the TV took the tuner ({})",
                state["state"].as_str().unwrap_or("no state")
            )),
            Err(f) => Some(f.reason()),
        }
    }

    fn read(&mut self, out: &mut Vec<u8>, wait: Duration) -> Result<(), Failure> {
        self.dvb
            .as_mut()
            .ok_or_else(|| hw("no demux"))?
            .read(out, wait)
    }
}

fn node(cfg: &HdhrConfig, kind: &str, write: bool) -> Result<File, Failure> {
    let path = format!("{}/{kind}{}", cfg.dvb_dir, cfg.channel);
    OpenOptions::new()
        .read(true)
        .write(write)
        .custom_flags(libc::O_NONBLOCK)
        .open(&path)
        .map_err(|e| hw(format!("{path}: {e}")))
}

/// Demux channel `cfg.channel` and its download: LG's own stack keeps channel 0, so this one is nobody else's.
struct Dvb {
    cfg: HdhrConfig,
    seed: [u8; 16],
    /// The demux fd the channel's input is set on; the channel keeps it for as long as this lives.
    input: Option<File>,
    dvr: Option<File>,
    taps: Vec<File>,
    rest: Vec<u8>,
    chunk: Vec<u8>,
}

impl Dvb {
    fn open(cfg: &HdhrConfig, seed: [u8; 16]) -> Result<Self, Failure> {
        let input = node(cfg, "demux", true)?;
        // the demod's port with the values LG's stack sets on its own channel: the driver finds the port configured as it is and leaves it be
        let mut source = ExtSource {
            src: SRC_IN_DEMOD,
            port: 0,
            port_type: PORT_PARALLEL,
        };
        let mut control = ExtControl {
            id: CID_INPUTSOURCE,
            size: size_of::<ExtSource>() as u32,
            value: std::ptr::from_mut(&mut source) as u64,
        };
        ioctl(&input, DMX_EXT_S_CTL, &mut control).map_err(|e| hw(format!("demux input: {e}")))?;
        Ok(Self {
            cfg: cfg.clone(),
            seed,
            input: Some(input),
            dvr: None,
            taps: Vec::new(),
            rest: Vec::new(),
            chunk: vec![0; UNIT * 512],
        })
    }

    /// One section of the table, filtered in the demux (the section number is the fifth filter byte), within `wait`.
    fn section(&mut self, pid: u16, table_id: u8, number: u8, wait: Duration) -> Option<Vec<u8>> {
        let mut file = node(&self.cfg, "demux", true).ok()?;
        let mut q = SectionFilter {
            pid,
            filter: [0; 16],
            mask: [0; 16],
            mode: [0; 16],
            timeout: 0,
            flags: CHECK_CRC | ONESHOT | IMMEDIATE_START,
        };
        q.filter[0] = table_id;
        q.mask[0] = 0xFF;
        q.filter[4] = number;
        q.mask[4] = 0xFF;
        ioctl(&file, DMX_SET_FILTER, &mut q).ok()?;
        let end = Instant::now() + wait;
        let mut buf = vec![0u8; 4096];
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if !readable(&file, left) {
                continue;
            }
            match file.read(&mut buf) {
                Ok(n) if n > 0 => return Some(buf[..n].to_vec()),
                _ => {}
            }
        }
        None
    }

    fn start(&mut self, pids: &[u16]) -> Result<(), Failure> {
        let dvr = node(&self.cfg, "dvr", false)?;
        let mut key = cert(&self.seed);
        let mut control = ExtControl {
            id: CID_CERT,
            size: 32,
            value: key.as_mut_ptr() as u64,
        };
        let certified = ioctl(&dvr, PVR_EXT_S_CTL, &mut control);
        key.fill(0);
        certified.map_err(|e| hw(format!("the PVR download refused the certificate: {e}")))?;
        for &pid in pids {
            let file = node(&self.cfg, "demux", true)?;
            let mut q = PesFilter {
                pid,
                input: 0,
                output: OUT_TS_TAP,
                pes_type: PES_OTHER,
                flags: IMMEDIATE_START,
            };
            ioctl(&file, DMX_SET_PES_FILTER, &mut q)
                .map_err(|e| hw(format!("tap on PID 0x{pid:x}: {e}")))?;
            self.taps.push(file);
        }
        self.dvr = Some(dvr);
        Ok(())
    }

    fn read(&mut self, out: &mut Vec<u8>, wait: Duration) -> Result<(), Failure> {
        let Some(dvr) = self.dvr.as_ref() else {
            return Err(hw("the download is not started"));
        };
        if !readable(dvr, wait) {
            return Ok(());
        }
        let n = match (&*dvr).read(&mut self.chunk) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(hw(format!("the download: {e}"))),
        };
        self.rest.extend_from_slice(&self.chunk[..n]);
        let whole = self.rest.len() / UNIT * UNIT;
        psi::strip_stamps(&self.rest[..whole], out)
            .map_err(|_| hw("the download lost its alignment"))?;
        self.rest.drain(..whole);
        Ok(())
    }
}

impl Drop for Dvb {
    fn drop(&mut self) {
        // stop routing, stop the download, put the channel's input back to what it was: nothing
        self.taps.clear();
        self.dvr = None;
        if let Some(input) = self.input.take() {
            let mut source = ExtSource {
                src: SRC_NULL,
                port: 0,
                port_type: PORT_NONE,
            };
            let mut control = ExtControl {
                id: CID_INPUTSOURCE,
                size: size_of::<ExtSource>() as u32,
                value: std::ptr::from_mut(&mut source) as u64,
            };
            let _ = ioctl(&input, DMX_EXT_S_CTL, &mut control);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::docs;
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    #[test]
    fn the_certificate_is_the_seed_xor_the_table_twice_over() {
        assert_eq!(cert(&[0; 16]), CERT_TABLE);
        let seed: [u8; 16] = std::array::from_fn(|i| i as u8 + 1);
        let c = cert(&seed);
        assert_eq!(
            (c[0], c[1], c[16], c[17]),
            (1 ^ 0x6a, 2 ^ 0xb0, 1 ^ 0x21, 2 ^ 0xe1)
        );
    }

    /// A fake program; it does nothing when `settle` runs it with `probe`.
    fn script(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!("#!/bin/sh\n[ \"$1\" = probe ] && exit 0\n{body}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        settle(&path);
        path.display().to_string()
    }

    /// Runs a script that was just written until the system lets it run. Another test thread's fork can hold the script's write
    /// descriptor for a moment, and exec of a file that is open for writing fails with "text file busy"; once one exec has worked, no
    /// later one can fail that way.
    fn settle(program: &Path) {
        for _ in 0..300 {
            let result = std::process::Command::new(program)
                .arg("probe")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            match result {
                Err(e) if e.raw_os_error() == Some(26) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                _ => return,
            }
        }
    }

    /// A fake `luna-send`: logs each call (`-n 1`: one answer; `-i`: the opener, which answers and then waits for its stdin to close, as
    /// the real one stays connected) and answers like the set did.
    fn fake_luna(dir: &Path) -> String {
        script(
            dir,
            "luna-send",
            &format!(
                r#"d={dir:?}
if [ "$1" = "-i" ]; then
  echo $$ > "$d/opener.pid"
  echo "$2 $3" >> "$d/calls.log"
  echo '{{"broadcastId":"0000004c-1","returnValue":true}}'
  cat > /dev/null
  exit 0
fi
echo "$3 $4" >> "$d/calls.log"
case "$3" in
  */getLastChannelId) echo '{{"returnValue":true,"channelId":"1_32_2_4_0_0_0"}}' ;;
  */getSignalStatus) echo '{{"returnValue":true,"signal":{{"locked":true,"strength":77}}}}' ;;
  */getChannelList) echo '{{"returnValue":true,"channelList":[{{"channelId":"1_32_2_4_0_0_0","channelName":"WEST"}}]}}' ;;
  */getForegroundAppInfo) echo "{{\"returnValue\":true,\"appId\":\"$(cat "$d/front")\"}}" ;;
  */getActivePipelines) cat "$d/pipelines" ;;
  */getBroadcastState) echo "{{\"returnValue\":true,\"state\":\"$(cat "$d/bstate")\"}}" ;;
  */changeChannel|*/stop|*/close) echo '{{"returnValue":true}}' ;;
  *) echo '{{"returnValue":false,"errorText":"unexpected"}}' ;;
esac"#
            ),
        )
    }

    fn tv(dir: &Path) -> Tv {
        Tv::new(config(dir), fake_luna(dir))
    }

    fn config(dir: &Path) -> HdhrConfig {
        HdhrConfig {
            python: script(
                dir,
                "python3",
                "printf '0 00112233445566778899aabbccddeeff'",
            ),
            dvb_dir: dir.join("dvb").display().to_string(),
            settle_ms: 0,
            ..HdhrConfig::default()
        }
    }

    fn calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_seed_key_is_read_once_from_the_hal_and_must_be_sixteen_non_zero_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let tv = tv(dir.path());
        assert_eq!(tv.inner.seed().unwrap()[..3], [0x00, 0x11, 0x22]);
        std::fs::remove_file(dir.path().join("python3")).unwrap();
        assert!(tv.inner.seed().is_ok(), "kept in memory");
        for (output, why) in [
            ("1 00112233445566778899aabbccddeeff", "refused"),
            ("0 0011", "short"),
            ("0 00000000000000000000000000000000", "zero"),
            ("0 zz112233445566778899aabbccddeeff", "not hex"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut cfg = config(dir.path());
            cfg.python = script(dir.path(), "python3", &format!("printf '{output}'"));
            assert!(
                matches!(
                    Tv::new(cfg, fake_luna(dir.path())).inner.seed(),
                    Err(Failure::Hardware(_))
                ),
                "{why}"
            );
        }
    }

    #[test]
    fn the_channel_list_comes_from_the_tv_and_live_tv_in_front_means_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let tv = tv(dir.path());
        let list = docs::entries(&tv.channel_list().unwrap());
        assert_eq!(
            list.iter()
                .map(|e| (e.guide.as_str(), e.name.as_str()))
                .collect::<Vec<_>>(),
            [("2.4", "WEST")]
        );
        std::fs::write(dir.path().join("front"), "com.webos.app.home").unwrap();
        std::fs::write(
            dir.path().join("pipelines"),
            r#"[{"currentAppId":"com.webos.app.channelsetting","type":"tv"}]"#,
        )
        .unwrap();
        assert_eq!(
            tv.in_use(),
            None,
            "the helper's own pipeline is not the TV's use"
        );
        std::fs::write(
            dir.path().join("pipelines"),
            r#"{"returnValue":true,"data":"pipeline empty"}"#,
        )
        .unwrap();
        assert_eq!(tv.in_use(), None);
        std::fs::write(
            dir.path().join("pipelines"),
            r#"[{"currentAppId":"com.webos.app.livetv","type":"tv"}]"#,
        )
        .unwrap();
        assert_eq!(
            tv.in_use(),
            Some("Live TV is running".to_owned()),
            "with the panel off, in standby, Live TV is not in front but has the tuner"
        );
        std::fs::write(dir.path().join("front"), LIVE_TV).unwrap();
        assert_eq!(tv.in_use(), Some("Live TV is in front".to_owned()));
        std::fs::remove_file(dir.path().join("luna-send")).unwrap();
        assert!(
            tv.in_use().is_some(),
            "a TV that cannot be asked is not known to be free"
        );
    }

    #[test]
    fn the_tuner_is_taken_when_the_pipeline_is_no_longer_the_one_rendering() {
        let dir = tempfile::tempdir().unwrap();
        let inner = tv(dir.path()).inner;
        let holder = inner.open_pipeline().unwrap();
        let mut tuned = Tuned {
            inner,
            holder: Some(holder),
            previous: String::new(),
            moved: false,
            dvb: None,
        };
        std::fs::write(dir.path().join("bstate"), "BROADCAST_STATE_RENDERING").unwrap();
        assert_eq!(tuned.taken(), None);
        std::fs::write(dir.path().join("bstate"), "BROADCAST_STATE_UNREALIZED").unwrap();
        assert_eq!(
            tuned.taken(),
            Some("the TV took the tuner (BROADCAST_STATE_UNREALIZED)".to_owned())
        );
        std::fs::remove_file(dir.path().join("luna-send")).unwrap();
        assert!(
            tuned.taken().is_some(),
            "a pipeline that cannot be asked about is gone"
        );
    }

    #[test]
    fn a_tune_that_fails_at_the_demux_puts_back_the_channel_and_closes_the_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let tv = tv(dir.path());
        let failed = tv.tune("1_27_5_1_0_0_0").err();
        assert!(
            matches!(&failed, Some(Failure::Hardware(why)) if why.contains("demux")),
            "no DVB nodes in the temp dir: {failed:?}"
        );
        let log = calls(dir.path());
        let at = |needle: &str, also: &str| {
            log.iter()
                .position(|l| l.contains(needle) && l.contains(also))
                .unwrap_or_else(|| panic!("{needle} {also} in {log:?}"))
        };
        let (to, back) = (
            at("changeChannel", "1_27_5_1_0_0_0"),
            at("changeChannel", "1_32_2_4_0_0_0"),
        );
        let (stop, close) = (at("/stop", "0000004c-1"), at("/close", "0000004c-1"));
        assert!(
            to < back && back < stop && stop < close,
            "moved, put back, stopped, closed: {log:?}"
        );
        let pid = std::fs::read_to_string(dir.path().join("opener.pid")).unwrap();
        assert!(
            !Path::new(&format!("/proc/{}", pid.trim())).exists(),
            "the opener's connection went with the pipeline"
        );
    }
}
