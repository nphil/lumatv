//! Screenshots and screen recordings of what LumaTV composed (`docs/research/capture-result.md`).
//!
//! What is in a capture is what this compositor drew into its own pages: LumaTV's screens and the apps' windows. Video is on a plane
//! of its own that no page ever holds, and the hole left for it is black in the picture, so protected video is black by construction, not
//! by a check; nothing here opens the video tap, LG's capture service or any device of the video path. A recording composes every frame
//! into a page (no window is scanned out directly while one runs), reads the last page back on the GPU at the recording's size, and
//! hands it to a pipeline thread (`record`) that converts it and feeds the hardware encoder (`encoder`) behind a fragmented MP4 (`mp4`).
//!
//! Switch: `capture.enabled` in the helper's `helper.json`, off by default and read again on every command (`Settings`). A capture runs
//! only on a command (the control socket's `screencap`, which the helper's `/v1/capture/...` routes forward, `helper/app/src/capture.rs`),
//! and never without something on the screen that says so: the shell's system layer shows the recording dot (topic `capture`) and
//! confirms it in its state (`capture.dot`); a recording does not start, and a running one stops, when that confirmation is missing.
//!
//! The files are in `<data dir>/captures` (`store`), under a size cap that deletes the oldest of them first and a free-space floor that
//! ends a recording.

pub mod encoder;
pub mod mp4;
pub mod nv12;
pub mod record;
pub mod store;

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::State;
use crate::control::Reply;
use crate::output::{Image, monotonic_ns};
use encoder::{EncoderFactory, EncoderSpec, VencFactory};
use record::{Ended, Finished, RecordSpec, Recorder};
use store::{Kind, Store};

/// Free space under which no recording starts and a running one ends.
pub const MIN_FREE: u64 = 128 << 20;
/// How long a screenshot waits for a frame to be composed before it takes the last one as it is.
const SHOT_WAIT: Duration = Duration::from_millis(1500);
/// The most a recording's start may take: the encoder opening, then the shell confirming its dot.
const START_WAIT: Duration = Duration::from_secs(8);
/// After the encoder is open, how long the shell has to say the dot is up.
const DOT_WAIT: Duration = Duration::from_secs(4);
/// How long a running recording survives the shell not saying the dot is up.
const DOT_LOST: Duration = Duration::from_secs(3);
/// Consecutive pictures that failed to read before the recording is ended.
const GRAB_FAILURES: u32 = 5;
/// The nice level of a screenshot's writer thread.
const NICE: i32 = 10;

/// The part of `helper.json` that is about capture, with its defaults: `{"capture": {"enabled": false, "maxBytes": 268435456,
/// "maxRecordSeconds": 300, "record": {"height": 720, "fps": 30, "kbps": 4000}, "encoder": "<data dir>/bin/lumatv-venc"}}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    /// What every capture in the folder may take together.
    pub max_bytes: u64,
    pub max_record_seconds: u64,
    pub record_height: u32,
    pub record_fps: u32,
    pub record_kbps: u32,
    pub encoder: PathBuf,
}

impl Settings {
    pub const FILE: &'static str = "helper.json";

    pub fn defaults(data_dir: &Path) -> Settings {
        Settings {
            enabled: false,
            max_bytes: 256 << 20,
            max_record_seconds: 300,
            record_height: 720,
            record_fps: 30,
            record_kbps: 4000,
            encoder: data_dir.join("bin/lumatv-venc"),
        }
    }

    pub fn from_json(doc: &Value, data_dir: &Path) -> Settings {
        let mut s = Settings::defaults(data_dir);
        let Some(c) = doc.get("capture").filter(|c| c.is_object()) else { return s };
        s.enabled = c.get("enabled").and_then(Value::as_bool).unwrap_or(false);
        if let Some(v) = c.get("maxBytes").and_then(Value::as_u64) {
            s.max_bytes = v.max(16 << 20);
        }
        if let Some(v) = c.get("maxRecordSeconds").and_then(Value::as_u64) {
            s.max_record_seconds = v.clamp(5, 1800);
        }
        if let Some(r) = c.get("record") {
            if let Some(v) = r.get("height").and_then(Value::as_u64).filter(|v| [360, 540, 720, 1080].contains(v)) {
                s.record_height = v as u32;
            }
            if let Some(v) = r.get("fps").and_then(Value::as_u64) {
                s.record_fps = v.clamp(5, 30) as u32;
            }
            if let Some(v) = r.get("kbps").and_then(Value::as_u64) {
                s.record_kbps = v.clamp(250, 20_000) as u32;
            }
        }
        if let Some(p) = c.get("encoder").and_then(Value::as_str).filter(|p| !p.is_empty()) {
            s.encoder = PathBuf::from(p);
        }
        s
    }

    /// Read from `<data dir>/helper.json` (defaults, so off, when it is missing or does not parse).
    pub fn read(data_dir: &Path) -> Settings {
        let doc = std::fs::read_to_string(data_dir.join(Self::FILE)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or(Value::Null);
        Settings::from_json(&doc, data_dir)
    }

    pub fn store(&self, data_dir: &Path) -> Store {
        Store::new(data_dir.join("captures"), self.max_bytes)
    }

    /// What a recording of an output this size is encoded at: the settings' height, the width of the output's aspect rounded to the macroblock.
    pub fn record_size(&self, output: (u32, u32)) -> (u32, u32) {
        let h = self.record_height.min(output.1).max(16) & !1;
        let w = ((h as u64 * output.0 as u64 + output.1 as u64 / 2) / output.1 as u64).div_ceil(16) * 16;
        (w as u32, h)
    }
}

struct PendingShot {
    frames_at_ask: u64,
    asked: Instant,
    reply: Sender<String>,
}

#[derive(Clone, Debug)]
struct ShotInfo {
    name: String,
    bytes: u64,
    width: u32,
    height: u32,
}

enum Done {
    Shot(Result<ShotInfo, String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Starting,
    Running,
    Stopping,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::Starting => "starting",
            Stage::Running => "recording",
            Stage::Stopping => "stopping",
        }
    }
}

struct Rec {
    recorder: Recorder,
    name: String,
    size: (u32, u32),
    period_ns: u64,
    started: Instant,
    since_ms: u64,
    stage: Stage,
    next_grab_ns: u64,
    dirty: bool,
    grab_failures: u32,
    ready_at: Option<Instant>,
    dot_lost_since: Option<Instant>,
    start_reply: Option<Sender<String>>,
    stop_replies: Vec<Sender<String>>,
    /// Why the compositor itself ended the recording, as an error code and words (the indicator was lost, the picture could not be read).
    ended_by: Option<(&'static str, String)>,
}

/// What the last capture came to, for `status` and the topic.
#[derive(Clone, Debug)]
struct Last {
    kind: Kind,
    name: String,
    bytes: u64,
    seconds: f64,
    ok: bool,
    note: String,
    at_ms: u64,
}

impl Last {
    fn json(&self) -> Value {
        json!({
            "kind": if self.kind == Kind::Shot { "shot" } else { "recording" },
            "name": self.name, "bytes": self.bytes, "seconds": self.seconds, "ok": self.ok, "note": self.note, "at": self.at_ms,
        })
    }
}

pub struct Screencap {
    /// The encoder a recording opens: `lumatv-venc` at the settings' path unless a test put another in.
    factory: Option<Arc<dyn EncoderFactory>>,
    shots: Vec<PendingShot>,
    done_tx: Sender<Done>,
    done_rx: Receiver<Done>,
    /// Screenshot writers still running.
    writing: u32,
    rec: Option<Rec>,
    last: Option<Last>,
    /// Screenshots taken so far: the topic's `n`, which the shell answers with a toast.
    taken: u64,
    /// The output has been told to compose every frame.
    armed: bool,
    /// What the topic last said, in a form that is compared without building the JSON: (recording, stage, since, taken, when the last capture ended).
    published: Option<(bool, &'static str, u64, u64, u64)>,
}

impl Default for Screencap {
    fn default() -> Self {
        let (done_tx, done_rx) = channel();
        Screencap { factory: None, shots: Vec::new(), done_tx, done_rx, writing: 0, rec: None, last: None, taken: 0, armed: false, published: None }
    }
}

impl Screencap {
    /// Puts another encoder in place of the hardware one (tests).
    pub fn set_encoder(&mut self, factory: Arc<dyn EncoderFactory>) {
        self.factory = Some(factory);
    }

    /// Nothing asked for, running or left to finish: a tick has nothing to do for it.
    fn idle(&self) -> bool {
        self.shots.is_empty() && self.rec.is_none() && self.writing == 0 && !self.armed
    }

    fn factory(&self, settings: &Settings, data_dir: &Path) -> Arc<dyn EncoderFactory> {
        self.factory.clone().unwrap_or_else(|| Arc::new(VencFactory { binary: settings.encoder.clone(), log: data_dir.join("compositor/venc.log") }))
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn fail(code: &str, error: impl std::fmt::Display) -> String {
    json!({"ok": false, "code": code, "error": error.to_string()}).to_string()
}

fn refuse(code: &str, error: impl std::fmt::Display) -> Reply {
    Reply::Now(fail(code, error))
}

/// A reply that is answered by the loop later: the connection waits on `rx` off the loop.
fn later(rx: Receiver<String>, within: Duration, what: &'static str) -> Reply {
    Reply::Later(Box::new(move || rx.recv_timeout(within).unwrap_or_else(|_| fail("timeout", format!("{what} did not finish in time")))))
}

/// A screenshot as a PNG: RGB, the pixels as they are (premultiplied over black is what the screen showed), written beside its name as
/// a part and renamed when whole, so nobody ever opens half a picture.
fn write_shot(store: &Store, image: &Image) -> Result<ShotInfo, String> {
    store.ensure_dir().map_err(|e| format!("{}: {e}", store.dir().display()))?;
    store.make_room(6 << 20);
    let name = store.new_name(Kind::Shot, SystemTime::now());
    let (path, part) = (store.path(&name), store.path(&format!("{name}.part")));
    let mut png_bytes = Vec::with_capacity(1 << 20);
    let encoded = (|| -> Result<(), png::EncodingError> {
        let mut enc = png::Encoder::new(&mut png_bytes, image.width, image.height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header()?;
        let mut rgb = Vec::with_capacity(image.rgba.len() / 4 * 3);
        for px in image.rgba.chunks_exact(4) {
            rgb.extend_from_slice(&px[..3]);
        }
        writer.write_image_data(&rgb)?;
        writer.finish()
    })();
    let written = encoded.map_err(io::Error::other).and_then(|()| {
        let mut file = File::create(&part)?;
        file.write_all(&png_bytes)?;
        file.sync_data()?;
        std::fs::rename(&part, &path)
    });
    match written {
        Ok(()) => Ok(ShotInfo { name, bytes: png_bytes.len() as u64, width: image.width, height: image.height }),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(format!("{}: {e}", path.display()))
        }
    }
}

impl State {
    /// The control socket's `screencap`: `status`, `shot`, `record start`, `record stop`.
    pub fn screencap_command(&mut self, sub: Option<&str>, arg: Option<&str>) -> Reply {
        match (sub, arg) {
            (Some("status"), _) | (None, _) => Reply::Now(self.screencap_status().to_string()),
            (Some("shot"), _) => self.screencap_shot(),
            (Some("record"), Some("start")) => self.screencap_record_start(),
            (Some("record"), Some("stop")) => self.screencap_record_stop(),
            _ => refuse("bad-command", "screencap [status | shot | record start | record stop]"),
        }
    }

    /// Whether something on the screen can say a capture is happening: the shell's system layer is there.
    fn screencap_indicator_ready(&self) -> bool {
        self.shell.connected
    }

    /// Whether the shell says its recording dot is on the screen.
    fn screencap_dot_up(&self) -> bool {
        self.shell.connected && self.shell.ui_state.pointer("/capture/dot").and_then(Value::as_bool) == Some(true)
    }

    fn screencap_gate(&self, settings: &Settings) -> Result<(), Reply> {
        if !settings.enabled {
            return Err(refuse("disabled", "screen capture is off (capture.enabled in helper.json)"));
        }
        if !self.screencap_indicator_ready() {
            return Err(refuse("no-indicator", "the shell is not running, so nothing could show that the screen is being captured"));
        }
        Ok(())
    }

    /// The output composes every frame from now on, and one is owed now.
    fn screencap_arm(&mut self) {
        if !self.screencap.armed {
            self.screencap.armed = true;
            self.output.set_compose_only(true);
        }
        self.scene.redraw_all();
    }

    fn screencap_shot(&mut self) -> Reply {
        let settings = Settings::read(&self.config.data_dir);
        if let Err(r) = self.screencap_gate(&settings) {
            return r;
        }
        let (tx, rx) = channel();
        self.screencap.shots.push(PendingShot { frames_at_ask: self.stats.frames_drawn, asked: Instant::now(), reply: tx });
        self.screencap_arm();
        later(rx, Duration::from_secs(15), "the screenshot")
    }

    fn screencap_record_start(&mut self) -> Reply {
        let settings = Settings::read(&self.config.data_dir);
        if let Err(r) = self.screencap_gate(&settings) {
            return r;
        }
        if self.screencap.rec.is_some() {
            return refuse("busy", "a recording is already running");
        }
        let factory = self.screencap.factory(&settings, &self.config.data_dir);
        if !factory.available() {
            return refuse("no-encoder", format!("there is no encoder at {} (the hardware encoder's program, lumatv-venc)", factory.describe()));
        }
        let store = settings.store(&self.config.data_dir);
        if let Err(e) = store.ensure_dir() {
            return refuse("write", format!("{}: {e}", store.dir().display()));
        }
        if store::free_bytes(store.dir()) < MIN_FREE {
            return refuse("disk-full", "the TV has too little free space to record");
        }
        // One file may take at most half the cap, so a second recording fits beside it before the oldest has to go.
        let max_bytes = settings.max_bytes / 2;
        let removed = store.make_room(max_bytes);
        if !removed.is_empty() {
            self.journal(format!("screencap: made room: removed {}", removed.join(", ")));
        }
        let size = settings.record_size(self.output.size());
        let name = store.new_name(Kind::Recording, SystemTime::now());
        let start_ns = monotonic_ns();
        let spec = RecordSpec {
            encoder: EncoderSpec { width: size.0, height: size.1, fps: settings.record_fps, kbps: settings.record_kbps, gop: settings.record_fps },
            part: store.path(&format!("{name}.part")),
            path: store.path(&name),
            max_bytes,
            max_seconds: settings.max_record_seconds,
            min_free: MIN_FREE,
            start_ns,
            keepalive: Duration::from_secs(1),
            space_check: Duration::from_secs(1),
        };
        let recorder = match Recorder::start(spec, factory) {
            Ok(r) => r,
            Err(e) => return refuse("write", format!("the recording's thread: {e}")),
        };
        let (tx, rx) = channel();
        self.screencap.rec = Some(Rec {
            recorder,
            name: name.clone(),
            size,
            period_ns: 1_000_000_000 / settings.record_fps as u64,
            started: Instant::now(),
            since_ms: now_ms(),
            stage: Stage::Starting,
            next_grab_ns: start_ns,
            dirty: true,
            grab_failures: 0,
            ready_at: None,
            dot_lost_since: None,
            start_reply: Some(tx),
            stop_replies: Vec::new(),
            ended_by: None,
        });
        self.journal(format!("screencap: recording {name} at {}x{} {} fps {} kbit/s", size.0, size.1, settings.record_fps, settings.record_kbps));
        self.screencap_arm();
        // The dot is asked for at once: its confirmation is part of the start.
        self.screencap_publish();
        later(rx, START_WAIT + Duration::from_secs(2), "the recording's start")
    }

    fn screencap_record_stop(&mut self) -> Reply {
        let Some(rec) = &mut self.screencap.rec else { return refuse("not-recording", "no recording is running") };
        let (tx, rx) = channel();
        rec.stop_replies.push(tx);
        if rec.stage != Stage::Stopping {
            Self::screencap_stop(rec, None);
        }
        later(rx, Duration::from_secs(12), "the recording's end")
    }

    /// Asks the pipeline to end the recording now; `why` (a code and words) when the compositor is the one ending it.
    fn screencap_stop(rec: &mut Rec, why: Option<(&'static str, String)>) {
        rec.stage = Stage::Stopping;
        if rec.ended_by.is_none() {
            rec.ended_by = why;
        }
        let at_us = monotonic_ns().saturating_sub(rec.recorder.spec().start_ns) / 1000;
        rec.recorder.stop(at_us);
    }

    pub fn screencap_status(&self) -> Value {
        let settings = Settings::read(&self.config.data_dir);
        let store = settings.store(&self.config.data_dir);
        let factory = self.screencap.factory(&settings, &self.config.data_dir);
        let rec = self.screencap.rec.as_ref().map(|r| {
            let s = r.recorder.snapshot();
            json!({
                "stage": r.stage.name(), "file": r.name, "width": r.size.0, "height": r.size.1, "since": r.since_ms,
                "seconds": r.started.elapsed().as_secs_f64(), "bytes": s.bytes, "samples": s.samples, "pictures": s.frames_in,
                "dropped": s.dropped, "keptAlive": s.kept_alive,
            })
        });
        json!({
            "ok": true,
            "enabled": settings.enabled,
            "recording": self.screencap.rec.is_some(),
            "recorder": rec,
            "encoder": {"path": factory.describe(), "available": factory.available()},
            "shell": {"connected": self.shell.connected, "dot": self.screencap_dot_up()},
            "settings": {
                "maxBytes": settings.max_bytes, "maxRecordSeconds": settings.max_record_seconds,
                "height": settings.record_height, "fps": settings.record_fps, "kbps": settings.record_kbps,
            },
            "captures": {"dir": store.dir().display().to_string(), "count": store.list().len(), "bytes": store.used_bytes(), "max": store.max_bytes()},
            "freeBytes": store::free_bytes(store.dir()).min(i64::MAX as u64),
            "taken": self.screencap.taken,
            "last": self.screencap.last.as_ref().map(Last::json),
        })
    }

    /// The topic the shell follows: whether to draw the recording dot, and the last capture's outcome for its toast.
    fn screencap_topic(&self) -> Value {
        json!({
            "recording": self.screencap.rec.is_some(),
            "stage": self.screencap.rec.as_ref().map_or("idle", |r| r.stage.name()),
            "since": self.screencap.rec.as_ref().map(|r| r.since_ms),
            "n": self.screencap.taken,
            "last": self.screencap.last.as_ref().map(Last::json),
        })
    }

    /// Pushes the `capture` topic when it changed (also from `shell_sync_topics`, so a client that connects late has it). Nothing is built
    /// while it has not: this runs wherever the scene changed, which is the frame path's neighbourhood.
    pub fn screencap_publish(&mut self) {
        if !(self.shell.enabled() || self.shell.connected) {
            return;
        }
        let key = (
            self.screencap.rec.is_some(),
            self.screencap.rec.as_ref().map_or("idle", |r| r.stage.name()),
            self.screencap.rec.as_ref().map_or(0, |r| r.since_ms),
            self.screencap.taken,
            self.screencap.last.as_ref().map_or(0, |l| l.at_ms),
        );
        // A client that connected since (the sync clears what was pushed) has not heard it, whatever the key says.
        if self.screencap.published == Some(key) && self.shell.topic("capture").is_some() {
            return;
        }
        self.screencap.published = Some(key);
        let topic = self.screencap_topic();
        self.shell.push_if_changed("capture", topic);
    }

    /// Once per tick, after the frame: screenshots waiting for a frame, the recording's state and pictures, writers that finished.
    pub fn screencap_tick(&mut self, drew: bool) {
        if self.screencap.idle() {
            return;
        }
        let now = Instant::now();
        while let Ok(Done::Shot(result)) = self.screencap.done_rx.try_recv() {
            self.screencap.writing = self.screencap.writing.saturating_sub(1);
            let (ok, name, bytes, note) = match &result {
                Ok(i) => (true, i.name.clone(), i.bytes, format!("{}x{}", i.width, i.height)),
                Err(e) => (false, String::new(), 0, e.clone()),
            };
            if ok {
                self.screencap.taken += 1;
            }
            self.screencap.last = Some(Last { kind: Kind::Shot, name: name.clone(), bytes, seconds: 0.0, ok, note: note.clone(), at_ms: now_ms() });
            self.journal(if ok { format!("screencap: screenshot {name} ({bytes} bytes, {note})") } else { format!("screencap: screenshot failed: {note}") });
        }
        self.screencap_shots(now);
        self.screencap_recording(drew, now);
        if self.screencap.armed && self.screencap.shots.is_empty() && self.screencap.rec.is_none() {
            self.screencap.armed = false;
            self.output.set_compose_only(false);
            self.output.grab_end();
        }
        self.screencap_publish();
    }

    /// Screenshots asked for: once a frame has been composed since the ask (or the wait is over), the picture is read and written.
    fn screencap_shots(&mut self, now: Instant) {
        let Some(first) = self.screencap.shots.first() else { return };
        if self.stats.frames_drawn <= first.frames_at_ask && now.duration_since(first.asked) < SHOT_WAIT {
            return;
        }
        let pending = std::mem::take(&mut self.screencap.shots);
        let size = self.output.size();
        let image = match self.output.grab(size, Vec::new()) {
            Ok(i) => i,
            Err(e) => {
                for p in pending {
                    let _ = p.reply.send(fail("grab", &e));
                }
                self.journal(format!("screencap: screenshot: {e}"));
                return;
            }
        };
        let settings = Settings::read(&self.config.data_dir);
        let store = settings.store(&self.config.data_dir);
        let done = self.screencap.done_tx.clone();
        let spawned = std::thread::Builder::new().name("shot".into()).spawn({
            let replies: Vec<Sender<String>> = pending.iter().map(|p| p.reply.clone()).collect();
            move || {
                // SAFETY: setpriority on the calling thread (a PRIO_PROCESS id of 0 is the thread on Linux).
                unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, NICE) };
                let began = Instant::now();
                let result = write_shot(&store, &image);
                let text = match &result {
                    Ok(i) => json!({
                        "ok": true, "name": i.name, "path": store.path(&i.name).display().to_string(), "bytes": i.bytes,
                        "width": i.width, "height": i.height, "ms": began.elapsed().as_millis() as u64,
                    })
                    .to_string(),
                    Err(e) => fail("write", e),
                };
                for r in replies {
                    let _ = r.send(text.clone());
                }
                let _ = done.send(Done::Shot(result));
            }
        });
        match spawned {
            Ok(_) => self.screencap.writing += 1,
            Err(e) => {
                for p in pending {
                    let _ = p.reply.send(fail("write", format!("the writer's thread: {e}")));
                }
            }
        }
    }

    /// The recording's stage: start confirmed or refused, the dot watched, pictures read when one is due, the end concluded.
    fn screencap_recording(&mut self, drew: bool, now: Instant) {
        let dot = self.screencap_dot_up();
        let connected = self.shell.connected;
        let State { screencap, output, .. } = self;
        let Some(rec) = &mut screencap.rec else { return };
        if drew {
            rec.dirty = true;
        }
        let snap = rec.recorder.snapshot();
        if let Some(fin) = snap.finished {
            self.screencap_conclude(fin);
            return;
        }
        match rec.stage {
            Stage::Starting => {
                if matches!(snap.ready, Some(Ok(()))) && rec.ready_at.is_none() {
                    rec.ready_at = Some(now);
                }
                let late = now.duration_since(rec.started) > START_WAIT;
                let no_dot = rec.ready_at.is_some_and(|t| now.duration_since(t) > DOT_WAIT) && !dot;
                if rec.ready_at.is_some() && dot {
                    rec.stage = Stage::Running;
                    rec.next_grab_ns = monotonic_ns();
                    if let Some(r) = rec.start_reply.take() {
                        let _ = r.send(json!({"ok": true, "recording": true, "name": rec.name, "width": rec.size.0, "height": rec.size.1}).to_string());
                    }
                } else if no_dot {
                    Self::screencap_stop(rec, Some(("no-indicator", "the shell did not confirm the recording dot".into())));
                } else if late {
                    Self::screencap_stop(rec, Some(("timeout", "the encoder did not open in time".into())));
                }
            }
            Stage::Running => {
                if dot {
                    rec.dot_lost_since = None;
                } else if rec.dot_lost_since.get_or_insert(now).elapsed() > DOT_LOST {
                    let why = if connected { "the recording dot is no longer confirmed" } else { "the shell is gone" };
                    Self::screencap_stop(rec, Some(("no-indicator", why.into())));
                }
                let now_ns = monotonic_ns();
                if rec.stage == Stage::Running && rec.dirty && now_ns >= rec.next_grab_ns {
                    match output.grab(rec.size, rec.recorder.spare()) {
                        Ok(image) => {
                            rec.grab_failures = 0;
                            rec.dirty = false;
                            let pts = now_ns.saturating_sub(rec.recorder.spec().start_ns) / 1000;
                            rec.recorder.send(image, pts);
                            // On schedule, but never a burst to catch up after a late tick.
                            rec.next_grab_ns = (rec.next_grab_ns + rec.period_ns).max(now_ns);
                        }
                        Err(e) => {
                            rec.grab_failures += 1;
                            log::warn!("screencap: picture: {e}");
                            if rec.grab_failures >= GRAB_FAILURES {
                                Self::screencap_stop(rec, Some(("grab", format!("the screen could not be read: {e}"))));
                            }
                        }
                    }
                }
            }
            Stage::Stopping => {}
        }
    }

    /// The pipeline has ended: what it came to, told to whoever waits and to the shell.
    fn screencap_conclude(&mut self, fin: Finished) {
        let Some(mut rec) = self.screencap.rec.take() else { return };
        let (ok, note) = match (&rec.ended_by, &fin.how) {
            (Some((_, why)), _) => (false, why.clone()),
            (None, Ended::Stopped) => (true, String::new()),
            (None, Ended::Limit(l)) => (true, format!("the {l} limit was reached")),
            (None, Ended::Failed(e)) => (false, e.clone()),
        };
        let name = fin.path.file_name().map_or_else(|| rec.name.clone(), |n| n.to_string_lossy().into_owned());
        self.screencap.last =
            Some(Last { kind: Kind::Recording, name: name.clone(), bytes: fin.bytes, seconds: fin.seconds, ok, note: note.clone(), at_ms: now_ms() });
        let mut reply = json!({
            "ok": ok && fin.samples > 0, "recording": false, "name": name, "path": fin.path.display().to_string(), "bytes": fin.bytes,
            "seconds": fin.seconds, "samples": fin.samples, "note": note, "encoder": fin.encoder,
        });
        if !ok || fin.samples == 0 {
            reply["code"] = json!(rec.ended_by.as_ref().map_or("failed", |(code, _)| code));
            reply["error"] = json!(if note.is_empty() { "the recording ended" } else { &note });
        }
        if let Some(r) = rec.start_reply.take() {
            // Ended before the start was confirmed: the start did not happen.
            let code = rec.ended_by.as_ref().map_or("encoder", |(code, _)| code);
            let _ = r.send(fail(code, if note.is_empty() { "the recording ended before it began" } else { &note }));
        }
        for r in rec.stop_replies.drain(..) {
            let _ = r.send(reply.to_string());
        }
        self.journal(format!(
            "screencap: recording {name} ended: {} ({} bytes, {:.1} s, {} samples){}",
            if ok { "ok" } else { "failed" },
            fin.bytes,
            fin.seconds,
            fin.samples,
            if note.is_empty() { String::new() } else { format!(": {note}") }
        ));
        log::info!("screencap: {}", fin.encoder);
        self.screencap_publish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_is_off_without_a_file_a_section_or_a_true() {
        let dir = std::env::temp_dir().join(format!("lumatv-screencap-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!Settings::read(&dir).enabled, "no file");
        std::fs::write(dir.join("helper.json"), "{ not json").unwrap();
        assert!(!Settings::read(&dir).enabled, "no JSON");
        std::fs::write(dir.join("helper.json"), r#"{"cameras": {"enabled": true}}"#).unwrap();
        assert!(!Settings::read(&dir).enabled, "another feature's switch is not this one");
        std::fs::write(dir.join("helper.json"), r#"{"capture": {"enabled": "yes"}}"#).unwrap();
        assert!(!Settings::read(&dir).enabled, "only a real true counts");
        std::fs::write(dir.join("helper.json"), r#"{"capture": {"enabled": true}}"#).unwrap();
        let s = Settings::read(&dir);
        assert!(s.enabled);
        assert_eq!(s, Settings { enabled: true, ..Settings::defaults(&dir) });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_are_read_in_the_helpers_camel_case_and_kept_inside_their_limits() {
        let d = Path::new("/var/lib/lumatv");
        let s = Settings::from_json(
            &json!({"capture": {"enabled": true, "maxBytes": 1000, "maxRecordSeconds": 99999, "record": {"height": 540, "fps": 60, "kbps": 100}, "encoder": "/x/venc"}}),
            d,
        );
        assert_eq!((s.max_bytes, s.max_record_seconds, s.record_height, s.record_fps, s.record_kbps), (16 << 20, 1800, 540, 30, 250));
        assert_eq!(s.encoder, PathBuf::from("/x/venc"));
        let s = Settings::from_json(&json!({"capture": {"record": {"height": 123}}}), d);
        assert_eq!(s.record_height, 720, "an unlisted height is the default");
        assert_eq!(Settings::defaults(d).encoder, d.join("bin/lumatv-venc"));
        assert_eq!(Settings::defaults(d).store(d).dir(), d.join("captures"));
    }

    #[test]
    fn recordings_are_encoded_at_the_outputs_aspect_on_the_macroblock_grid() {
        let s = Settings::defaults(Path::new("/x"));
        assert_eq!(s.record_size((1920, 1080)), (1280, 720));
        assert_eq!(Settings { record_height: 540, ..s.clone() }.record_size((1920, 1080)), (960, 540));
        assert_eq!(Settings { record_height: 360, ..s.clone() }.record_size((1920, 1080)), (640, 360));
        assert_eq!(Settings { record_height: 1080, ..s.clone() }.record_size((1920, 1080)), (1920, 1080));
        assert_eq!(Settings { record_height: 1080, ..s }.record_size((1280, 720)), (1280, 720), "never larger than the output");
    }
}
