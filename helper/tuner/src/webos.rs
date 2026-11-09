//! An LG webOS TV as the tuner's [`Hardware`], as measured on the set (webOS 10.2.1, kernel 5.4.268; `docs/research/live-tv-capture.md`):
//!
//! - The tuner is a windowless `SCAN` pipeline of LG's legacy broadcast service, played with `playSignaling`. That asks LG's resource
//!   manager for the tuner and the section decoder alone (`WATCH_SECTION_PARSE`), so it tunes with the TV on and any app in front, takes
//!   nothing from that app's picture or sound, and leaves the TV's last channel as it was. `utp`'s `changeChannel` asks for the whole
//!   watch chain and was refused while the app in front held the video decoders (Netflix in 4K holds all four; 2026-11-08). The pipeline
//!   outlives the connection that opened it, so the open one is on record ([`WebosConfig::pipeline_file`]) and one that a run ended
//!   without closing is closed before the next opens.
//! - The stream is the kernel's own: a second demux channel (`demux2`) takes the demod's port with the values LG's stack uses, section
//!   filters read the PSI, and a PVR download (`dvr2`) carries the routed PIDs as 192-byte units (a 4-byte timestamp, then the packet) once
//!   the download has been certified. The certificate is LG's documented one: the device's seed key (a call into LG's HAL, made in the
//!   set's 32-bit python because a 64-bit program cannot load it) xor a constant table. The seed is kept in memory and never written or
//!   logged.
//!
//! The DVB nodes and the HAL need root. The luna bus is the host's ([`Luna`]); [`LunaSend`] is the set's own `luna-send`, which is what a
//! program that cannot link LG's 32-bit luna library has.

use crate::psi::{self, UNIT};
use crate::stream::{Failure, Hardware, Hold, Signal, Tap};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// LG's legacy broadcast service, the one `utp`'s own drives.
const BROADCAST: &str = "luna://com.webos.service.legacybroadcast.broadcast";
const LIVE_TV: &str = "com.webos.app.livetv";
/// The state of a broadcast pipeline that is its opener's: once Live TV opens, LG unrealizes the helper's.
const RENDERING: &str = "BROADCAST_STATE_RENDERING";
/// How long a luna call may take.
const CALL_WAIT: Duration = Duration::from_secs(10);
/// How long the frontend gets to lock once the channel is played, and how often it is asked. Measured on the set: every RF channel that
/// locks (the weak RF 29 too) locked 0.3 to 0.35 s after `utp`'s `changeChannel` answered (2026-11-05), and within the first second of
/// `playSignaling` (2026-11-08); the set answers `getSignalStatus` in about 30 ms, and an RF channel that does not lock (RF 35) reads
/// strength 0 throughout.
const LOCK_WAIT: Duration = Duration::from_secs(3);
const LOCK_POLL: Duration = Duration::from_millis(100);
/// The demodulator's report on the frequency it is tuned to.
const DEMOD_STATUS: &str = "/proc/lg/demod/signal_status";

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

/// How the TV's luna bus is reached.
pub trait Luna: Send + Sync + 'static {
    /// One call and its first reply: an object, or the bare array some services answer with. `Err` is the transport's failure, in words.
    fn call(&self, uri: &str, payload: &Value, wait: Duration) -> Result<Value, String>;
}

/// The set's `luna-send`, one call each (`-n 1`).
pub struct LunaSend {
    program: String,
}

impl LunaSend {
    /// `program` is the path of `luna-send`, or its name in `PATH`.
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
        }
    }
}

impl Luna for LunaSend {
    fn call(&self, uri: &str, payload: &Value, wait: Duration) -> Result<Value, String> {
        let text = run(
            Command::new(&self.program).args(["-n", "1", uri, &payload.to_string()]),
            wait,
        )?;
        first_value(&text).ok_or_else(|| "no answer".to_owned())
    }
}

/// The first JSON value in what a program wrote: an object or an array, after whatever came before it.
fn first_value(text: &str) -> Option<Value> {
    let start = text.find(['{', '['])?;
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

/// Where the TV's tuner and its HAL are.
#[derive(Clone, Debug)]
pub struct WebosConfig {
    /// The set's 32-bit python, which loads LG's HAL for the PVR seed key.
    pub python: String,
    /// The DVB adapter's directory, and the demux channel (with its PVR download) the tuner uses: LG's own stack keeps channel 0.
    pub dvb_dir: String,
    pub channel: u8,
    /// Where the open broadcast pipeline's id is kept. LG keeps a pipeline when the program that opened it ends, so the next tune closes the
    /// one a run left open when it ended mid-stream.
    pub pipeline_file: PathBuf,
}

impl Default for WebosConfig {
    fn default() -> Self {
        Self {
            python: "python3".to_owned(),
            dvb_dir: "/dev/dvb/adapter0".to_owned(),
            channel: 2,
            pipeline_file: std::env::temp_dir().join("lumatuner-pipeline"),
        }
    }
}

/// The TV, through luna and the DVB nodes.
pub struct Tv {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: WebosConfig,
    bus: Arc<dyn Luna>,
    seed: Mutex<Option<[u8; 16]>>,
}

impl Tv {
    pub fn new(cfg: WebosConfig, bus: Arc<dyn Luna>) -> Self {
        Self {
            inner: Arc::new(Inner {
                cfg,
                bus,
                seed: Mutex::new(None),
            }),
        }
    }
}

impl Inner {
    /// A call LG's service answered with `returnValue: true`.
    fn luna(&self, uri: &str, payload: &Value) -> Result<Value, Failure> {
        let reply = self
            .bus
            .call(uri, payload, CALL_WAIT)
            .map_err(|e| hw(format!("{uri}: {e}")))?;
        if reply["returnValue"].as_bool() == Some(true) {
            Ok(reply)
        } else {
            Err(hw(format!(
                "{uri}: {}",
                reply["errorText"].as_str().unwrap_or("refused")
            )))
        }
    }

    /// Opens the windowless scan pipeline and puts its id on record, after closing the one on record (a run that ended mid-stream left it).
    fn open_pipeline(&self) -> Result<String, Failure> {
        let record = &self.cfg.pipeline_file;
        if let Ok(left) = std::fs::read_to_string(record)
            && self.close(left.trim())
        {
            let _ = std::fs::remove_file(record);
        }
        let uri = format!("{BROADCAST}/open");
        let reply = self.luna(
            &uri,
            &json!({"options": {"appId": "com.webos.app.channelsetting", "windowId": "", "purpose": "SCAN"}}),
        )?;
        let id = reply["broadcastId"]
            .as_str()
            .ok_or_else(|| hw(format!("{uri}: no broadcastId")))?;
        let _ = std::fs::write(record, id);
        Ok(id.to_owned())
    }

    /// Plays `channel_id` on the pipeline for its tables (`WATCH_SECTION_PARSE`): the tuner and the section decoder, no video or audio
    /// decoder, and the TV's last channel stays as it was.
    fn play(&self, id: &str, channel_id: &str) -> Result<(), Failure> {
        self.luna(
            &format!("{BROADCAST}/playSignaling"),
            &json!({"broadcastId": id, "channelId": channel_id}),
        )
        .map(|_| ())
    }

    /// Stops and closes a pipeline; true when LG closed it.
    fn close(&self, id: &str) -> bool {
        let id = json!({"broadcastId": id});
        let _ = self.luna(&format!("{BROADCAST}/stop"), &id);
        self.luna(&format!("{BROADCAST}/close"), &id).is_ok()
    }

    /// The pipeline's state as LG's broadcast service gives it: [`RENDERING`] while it is the helper's.
    fn state(&self, id: &str) -> Result<String, Failure> {
        let reply = self.luna(
            &format!("{BROADCAST}/getBroadcastState"),
            &json!({"broadcastId": id}),
        )?;
        Ok(reply["state"].as_str().unwrap_or("no state").to_owned())
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
                return Err(Failure::NoSignal(format!(
                    "the tuner did not lock (strength {})",
                    status["signal"]["strength"].as_i64().unwrap_or(0)
                )));
            }
            thread::sleep(LOCK_POLL);
        }
    }

    /// UMS's pipelines (`getActivePipelines`): a bare array of objects, or `returnValue` true and `data: "pipeline empty"` when there are
    /// none.
    fn pipelines(&self) -> Result<Vec<Value>, Failure> {
        let uri = "luna://com.webos.media/getActivePipelines";
        match self
            .bus
            .call(uri, &json!({}), CALL_WAIT)
            .map_err(|e| hw(format!("{uri}: {e}")))?
        {
            Value::Array(list) => Ok(list),
            reply if reply["returnValue"].as_bool() == Some(true) => Ok(Vec::new()),
            _ => Err(hw(format!("{uri} gave no list"))),
        }
    }

    /// The TV's own use of the tuner: Live TV in front, or its pipeline running (with the panel off too); a TV that cannot be asked is
    /// not known to be free.
    fn in_use(&self) -> Option<String> {
        let front = match self.luna(
            "luna://com.webos.applicationManager/getForegroundAppInfo",
            &json!({}),
        ) {
            Ok(front) => front,
            Err(f) => return Some(f.reason()),
        };
        if front["appId"].as_str() == Some(LIVE_TV) {
            return Some("Live TV is in front".to_owned());
        }
        match self.pipelines() {
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

    /// The seed key, fetched once per run.
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

/// How far before and after now the guide is read. `getEventList` takes the broadcast's wall clock, whose offset from UTC is not known
/// here: the window is asked for as if UTC were that clock, which moves it by at most 14 hours, so a day of margin keeps now inside it.
const GUIDE_BEFORE_MS: u64 = 24 * 3600 * 1000;
const GUIDE_AHEAD_MS: u64 = 6 * 24 * 3600 * 1000;

/// `2026,10,25,04,22,00` for Unix milliseconds: the UTC time strings of LG's guide service.
pub(crate) fn lg_stamp(ms: u64) -> String {
    let secs = i64::try_from(ms / 1000).unwrap_or(i64::MAX / 2);
    // Howard Hinnant's `civil_from_days`
    let z = secs.div_euclid(86_400) + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let rem = secs.rem_euclid(86_400);
    format!(
        "{year:04},{month:02},{day:02},{:02},{:02},{:02}",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// The `getEventList` request for one channel's programmes around `now_ms`.
fn events_request(channel_id: &str, now_ms: u64) -> Value {
    json!({
        "dataType": 0,
        "localStartTime": lg_stamp(now_ms.saturating_sub(GUIDE_BEFORE_MS)),
        "localEndTime": lg_stamp(now_ms + GUIDE_AHEAD_MS),
        "channelId": [channel_id],
    })
}

pub(crate) fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The signal [`DEMOD_STATUS`] reports: the lock, the demodulator's mean square error against its constellation and the signal to noise
/// it makes against the constellation's power (8VSB's levels of 1, 3, 5 and 7 average 21, which the set gives as `Const Power` 21504:
/// the MSE's scale of 1024), the packets it could not correct and the bit errors it corrected. Read on the set on 2026-11-05: RF 32 locked
/// at MSE 25 to 37 (28 to 29 dB) with no packet errors and BER 0, weak RF 29 at 225 to 365 (18 to 20 dB) with 21 to 255 and BER 320,000
/// to 585,000, RF 35 unlocked at 65535.
pub fn parse_signal(text: &str) -> Option<Signal> {
    let field = |name: &str| {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == name).then(|| v.trim())
        })
    };
    let locked = field("Demod status")? == "LOCK";
    let mse: u32 = field("MSE")?.parse().ok()?;
    let power: f32 = field("Const Power")
        .and_then(|v| v.parse().ok())
        .unwrap_or(21_504.0);
    let snr = (locked && mse > 0).then(|| 10.0 * (power / mse as f32).log10());
    Some(Signal {
        locked,
        mse: locked.then_some(mse),
        snr,
        errors: field("Packet Error").and_then(|v| v.parse().ok()),
        ber: field("BER").and_then(|v| v.parse().ok()),
    })
}

impl Hardware for Tv {
    fn signal(&self) -> Option<Signal> {
        // LG's handler writes its whole report (482 bytes on 2026-11-05) into the reader's buffer whatever size was asked for: a smaller
        // read, like the 32-byte first read of `read_to_string`, overruns the buffer (the helper aborted with a corrupted heap, as `dd
        // bs=32` does). One read into a buffer far larger than the report, and no second read.
        let mut report = [0u8; 16 * 1024];
        let n = File::open(DEMOD_STATUS).ok()?.read(&mut report).ok()?;
        parse_signal(std::str::from_utf8(&report[..n]).ok()?)
    }

    fn channel_list(&self) -> Result<Value, Failure> {
        self.inner.luna(
            "luna://com.webos.service.iepg/getChannelList",
            &json!({"dataType": 0, "sort": 0, "channelMode": ["Tuner"], "channelGroup": ["All"], "requestField": ["channelId", "channelNumber", "channelName", "skipped", "Invisible"]}),
        )
    }

    fn programmes(&self, channel_id: &str) -> Result<Value, Failure> {
        self.inner.luna(
            "luna://com.webos.service.iepg/getEventList",
            &events_request(channel_id, unix_ms()),
        )
    }

    fn in_use(&self) -> Option<String> {
        self.inner.in_use()
    }

    fn tune(&self, channel_id: &str) -> Result<Box<dyn Tap>, Failure> {
        let inner = self.inner.clone();
        let seed = inner.seed()?;
        // from here the pipeline is closed however this ends: `Tuned` is the guard
        let mut tuned = Tuned {
            pipeline: inner.open_pipeline()?,
            inner: inner.clone(),
            dvb: None,
        };
        inner.play(&tuned.pipeline, channel_id)?;
        inner.wait_lock(&tuned.pipeline)?;
        tuned.dvb = Some(Dvb::open(&inner.cfg, seed)?);
        Ok(Box::new(tuned))
    }

    fn hold(&self, channel_id: &str) -> Result<Box<dyn Hold>, Failure> {
        let inner = self.inner.clone();
        let tuned = Tuned {
            pipeline: inner.open_pipeline()?,
            inner: inner.clone(),
            dvb: None,
        };
        inner.play(&tuned.pipeline, channel_id)?;
        Ok(Box::new(tuned))
    }

    fn tuned_to(&self) -> Option<String> {
        self.inner.last_channel().ok()
    }
}

impl Hold for Tuned {
    fn taken(&mut self) -> Option<String> {
        Tap::taken(self)
    }
}

/// A tuned antenna: the pipeline's id and the demux tap. Dropping it lets go in the order that cannot overflow the download: the tap first,
/// then the pipeline.
struct Tuned {
    inner: Arc<Inner>,
    pipeline: String,
    dvb: Option<Dvb>,
}

impl Drop for Tuned {
    fn drop(&mut self) {
        self.dvb = None;
        // Live TV that has taken the tuner keeps playing through a `stop` and `close` of this pipeline (measured on the set on 2026-11-05).
        // The record may be a newer pipeline's already: a stream that takes the tuner from the meter opens before the meter lets go.
        let record = &self.inner.cfg.pipeline_file;
        if self.inner.close(&self.pipeline)
            && std::fs::read_to_string(record).is_ok_and(|id| id == self.pipeline)
        {
            let _ = std::fs::remove_file(record);
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
        match self.inner.state(&self.pipeline) {
            Ok(state) if state == RENDERING => None,
            Ok(state) => Some(format!("the TV took the tuner ({state})")),
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

fn node(cfg: &WebosConfig, kind: &str, write: bool) -> Result<File, Failure> {
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
    cfg: WebosConfig,
    seed: [u8; 16],
    /// The demux fd the channel's input is set on; the channel keeps it for as long as this lives.
    input: Option<File>,
    dvr: Option<File>,
    taps: Vec<File>,
    rest: Vec<u8>,
    chunk: Vec<u8>,
}

impl Dvb {
    fn open(cfg: &WebosConfig, seed: [u8; 16]) -> Result<Self, Failure> {
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
    use super::*;
    use crate::docs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};

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

    #[test]
    fn the_demodulator_s_report_is_a_lock_a_signal_to_noise_and_its_errors() {
        // read on the set on 2026-11-05, RF 32 at rest
        let report = "*****************DEMOD_Debug_TotalInfo*****************\nDemod status\t=\tLOCK\nOperMode    =       VSB\n\
                      *********************Signal Status*********************\nPacket Error =    0\nBER =             0\nAGC =             625\n\
                      MSE =             31\nbMSE =            0\nConst Power\t=     21504\nFrequency Offset= 6310\n";
        let s = parse_signal(report).unwrap();
        assert!(s.locked);
        assert!((s.snr.unwrap() - 28.4).abs() < 0.1, "{s:?}");
        assert_eq!((s.mse, s.errors, s.ber), (Some(31), Some(0), Some(0)));
        // RF 29 the same day
        let weak = parse_signal(&report.replace("Error =    0", "Error =    81").replace("BER =             0", "BER = 584211").replace("31\nbMSE", "355\nbMSE"))
            .unwrap();
        assert_eq!((weak.mse, weak.errors, weak.ber), (Some(355), Some(81), Some(584_211)));
        let none =
            parse_signal(&report.replace("\tLOCK", "\tUNLOCK").replace("31", "65535")).unwrap();
        assert_eq!((none.locked, none.snr, none.mse), (false, None, None));
        assert_eq!(parse_signal("no such report"), None);
    }

    #[test]
    fn the_guide_is_asked_for_a_day_before_now_and_six_days_after() {
        // 2026-11-05 15:00:00 UTC
        assert_eq!(
            events_request("1_32_2_1_0_0_0", 1_793_890_800_000),
            json!({"dataType": 0, "localStartTime": "2026,11,04,15,00,00", "localEndTime": "2026,11,11,15,00,00", "channelId": ["1_32_2_1_0_0_0"]})
        );
        // 2024-02-29 23:59:59 UTC and 2026-10-25 04:22:00 UTC
        assert_eq!(lg_stamp(1_709_251_199_000), "2024,02,29,23,59,59");
        assert_eq!(lg_stamp(1_792_902_120_000), "2026,10,25,04,22,00");
    }

    /// The bus as the set answered, every call in order.
    #[derive(Default)]
    struct FakeLuna {
        log: Mutex<Vec<String>>,
        front: Mutex<String>,
        pipelines: Mutex<Value>,
        state: Mutex<String>,
        down: AtomicBool,
    }

    impl Luna for FakeLuna {
        fn call(&self, uri: &str, payload: &Value, _: Duration) -> Result<Value, String> {
            if self.down.load(Ordering::Relaxed) {
                return Err("could not run it".to_owned());
            }
            self.log.lock().push(format!("{uri} {payload}"));
            Ok(match uri.rsplit('/').next().unwrap_or("") {
                "open" => {
                    let opened = self.log.lock().iter().filter(|l| l.contains("/open ")).count();
                    json!({"returnValue": true, "broadcastId": format!("0000004c-{opened}")})
                }
                "getSignalStatus" => {
                    json!({"returnValue": true, "signal": {"locked": true, "strength": 77}})
                }
                "getChannelList" => {
                    json!({"returnValue": true, "channelList": [{"channelId": "1_32_2_4_0_0_0", "channelName": "WEST"}]})
                }
                "getForegroundAppInfo" => json!({"returnValue": true, "appId": *self.front.lock()}),
                "getActivePipelines" => self.pipelines.lock().clone(),
                "getBroadcastState" => json!({"returnValue": true, "state": *self.state.lock()}),
                "playSignaling" | "stop" | "close" => json!({"returnValue": true}),
                _ => json!({"returnValue": false, "errorText": "unexpected"}),
            })
        }
    }

    impl FakeLuna {
        /// The calls in order, those on the broadcast service as its method and the pipeline and channel they name.
        fn broadcast(&self) -> Vec<String> {
            let service = format!("{BROADCAST}/");
            self.log
                .lock()
                .iter()
                .map(|line| {
                    let Some((method, payload)) =
                        line.strip_prefix(&service).and_then(|l| l.split_once(' '))
                    else {
                        return line.clone();
                    };
                    let payload: Value = serde_json::from_str(payload).unwrap();
                    [Some(method), payload["broadcastId"].as_str(), payload["channelId"].as_str()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect()
        }
    }

    /// The TV over the fake bus, with its pipeline's record in a directory of its own; the seed is the one LG's HAL would give, so no
    /// python runs.
    fn tv() -> (Tv, Arc<FakeLuna>, tempfile::TempDir) {
        let luna = Arc::new(FakeLuna::default());
        let dir = tempfile::tempdir().unwrap();
        let tv = Tv::new(
            WebosConfig {
                dvb_dir: "/nonexistent/dvb".to_owned(),
                pipeline_file: dir.path().join("pipeline"),
                ..WebosConfig::default()
            },
            luna.clone(),
        );
        *tv.inner.seed.lock() = Some([0x11; 16]);
        (tv, luna, dir)
    }

    #[test]
    fn the_channel_list_comes_from_the_tv_and_live_tv_in_front_means_in_use() {
        let (tv, luna, _dir) = tv();
        let list = docs::entries(&tv.channel_list().unwrap());
        assert_eq!(
            list.iter()
                .map(|e| (e.guide.as_str(), e.name.as_str()))
                .collect::<Vec<_>>(),
            [("2.4", "WEST")]
        );
        *luna.front.lock() = "com.webos.app.home".into();
        *luna.pipelines.lock() =
            json!([{"currentAppId": "com.webos.app.channelsetting", "type": "tv"}]);
        assert_eq!(
            tv.in_use(),
            None,
            "the tuner's own pipeline is not the TV's use"
        );
        *luna.pipelines.lock() = json!({"returnValue": true, "data": "pipeline empty"});
        assert_eq!(tv.in_use(), None);
        *luna.front.lock() = "netflix".into();
        *luna.pipelines.lock() = json!([{"currentAppId": "netflix", "type": "media"}, {"currentAppId": "com.webos.app.channelsetting", "type": "tv"}]);
        assert_eq!(
            tv.in_use(),
            None,
            "another app in front, playing video, is not the TV's use of the tuner"
        );
        *luna.pipelines.lock() = json!([{"currentAppId": "com.webos.app.livetv", "type": "tv"}]);
        assert_eq!(
            tv.in_use(),
            Some("Live TV is running".to_owned()),
            "with the panel off, in standby, Live TV is not in front but has the tuner"
        );
        *luna.front.lock() = LIVE_TV.into();
        assert_eq!(tv.in_use(), Some("Live TV is in front".to_owned()));
        luna.down.store(true, Ordering::Relaxed);
        assert!(
            tv.in_use().is_some(),
            "a TV that cannot be asked is not known to be free"
        );
    }

    #[test]
    fn the_tuner_is_taken_when_the_pipeline_is_no_longer_the_one_rendering() {
        let (tv, luna, _dir) = tv();
        let mut tuned = Tuned {
            pipeline: tv.inner.open_pipeline().unwrap(),
            inner: tv.inner.clone(),
            dvb: None,
        };
        *luna.state.lock() = "BROADCAST_STATE_RENDERING".into();
        assert_eq!(Tap::taken(&mut tuned), None);
        *luna.state.lock() = "BROADCAST_STATE_UNREALIZED".into();
        assert_eq!(
            Tap::taken(&mut tuned),
            Some("the TV took the tuner (BROADCAST_STATE_UNREALIZED)".to_owned())
        );
        luna.down.store(true, Ordering::Relaxed);
        assert!(
            Tap::taken(&mut tuned).is_some(),
            "a pipeline that cannot be asked about is gone"
        );
    }

    #[test]
    fn a_tune_plays_the_channel_for_its_tables_alone_and_lets_go_with_a_stop_and_a_close() {
        let (tv, luna, dir) = tv();
        let failed = tv.tune("1_27_5_1_0_0_0").err();
        assert!(
            matches!(&failed, Some(Failure::Hardware(why)) if why.contains("demux")),
            "no DVB nodes: {failed:?}"
        );
        assert_eq!(
            luna.broadcast(),
            [
                "open",
                "playSignaling 0000004c-1 1_27_5_1_0_0_0",
                "getSignalStatus 0000004c-1",
                "stop 0000004c-1",
                "close 0000004c-1"
            ],
            "the TV's last channel is neither read nor moved"
        );
        let open = luna.log.lock()[0].clone();
        assert!(
            open.contains(r#""purpose":"SCAN""#) && open.contains(r#""windowId":"""#),
            "windowless: {open}"
        );
        assert!(
            !dir.path().join("pipeline").exists(),
            "a closed pipeline is off the record"
        );
    }

    #[test]
    fn the_pipeline_on_record_is_closed_before_the_next_opens_and_stays_on_record_until_it_closes() {
        let (tv, luna, dir) = tv();
        let record = dir.path().join("pipeline");
        // a run that ended mid-stream
        std::fs::write(&record, "left-1\n").unwrap();
        let meter = tv.hold("1_27_5_1_0_0_0").unwrap();
        assert_eq!(std::fs::read_to_string(&record).unwrap(), "0000004c-1");
        // a stream that takes the tuner before the meter lets go
        let stream = tv.hold("1_32_2_1_0_0_0").unwrap();
        drop(meter);
        assert_eq!(std::fs::read_to_string(&record).unwrap(), "0000004c-2");
        drop(stream);
        assert_eq!(
            luna.broadcast(),
            [
                "stop left-1",
                "close left-1",
                "open",
                "playSignaling 0000004c-1 1_27_5_1_0_0_0",
                "stop 0000004c-1",
                "close 0000004c-1",
                "open",
                "playSignaling 0000004c-2 1_32_2_1_0_0_0",
                "stop 0000004c-1",
                "close 0000004c-1",
                "stop 0000004c-2",
                "close 0000004c-2"
            ]
        );
        assert!(!record.exists());
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
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            match result {
                Err(e) if e.raw_os_error() == Some(26) => thread::sleep(Duration::from_millis(10)),
                _ => return,
            }
        }
    }

    #[test]
    fn luna_send_answers_with_its_first_value() {
        let dir = tempfile::tempdir().unwrap();
        let program = script(
            dir.path(),
            "luna-send",
            r#"case "$3" in
  */getActivePipelines) echo '[{"currentAppId":"com.webos.app.livetv"}]' ;;
  *) echo 'a warning first'; echo '{"returnValue":true,"echo":'"$4"'}' ;;
esac"#,
        );
        let luna = LunaSend::new(program);
        let wait = Duration::from_secs(5);
        assert_eq!(
            luna.call("luna://x/y", &json!({"a": 1}), wait).unwrap(),
            json!({"returnValue": true, "echo": {"a": 1}})
        );
        assert_eq!(
            luna.call(
                "luna://com.webos.media/getActivePipelines",
                &json!({}),
                wait
            )
            .unwrap(),
            json!([{"currentAppId": "com.webos.app.livetv"}])
        );
        assert!(
            LunaSend::new(dir.path().join("none").display().to_string())
                .call("luna://x/y", &json!({}), wait)
                .is_err()
        );
    }

    #[test]
    fn the_seed_key_is_read_once_from_the_hal_and_must_be_sixteen_non_zero_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let tv = |python: String| {
            Tv::new(
                WebosConfig {
                    python,
                    ..WebosConfig::default()
                },
                Arc::new(FakeLuna::default()),
            )
        };
        let good = tv(script(
            dir.path(),
            "python3",
            "printf '0 00112233445566778899aabbccddeeff'",
        ));
        assert_eq!(good.inner.seed().unwrap()[..3], [0x00, 0x11, 0x22]);
        std::fs::remove_file(dir.path().join("python3")).unwrap();
        assert!(good.inner.seed().is_ok(), "kept in memory");
        for (n, (output, why)) in [
            ("1 00112233445566778899aabbccddeeff", "refused"),
            ("0 0011", "short"),
            ("0 00000000000000000000000000000000", "zero"),
            ("0 zz112233445566778899aabbccddeeff", "not hex"),
        ]
        .into_iter()
        .enumerate()
        {
            let python = script(
                dir.path(),
                &format!("python{n}"),
                &format!("printf '{output}'"),
            );
            assert!(
                matches!(tv(python).inner.seed(), Err(Failure::Hardware(_))),
                "{why}"
            );
        }
    }
}
