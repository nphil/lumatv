//! The media runtime for the TV: every encoded unit the engine hands over goes to lumatv-gfn-feeder over its socket, and what the
//! feeder reports (accepted, dropped, the pipeline's events and its per-second status) comes back as the engine's `MediaFeedback`.
//! Decoding, HDR and the video plane are LG's; this process never sees a picture (docs/research/opennow-result.md, "Which media API").
//!
//! VIDEO carries absolute CLOCK_MONOTONIC nanoseconds. The shared feeder owns load-relative millisecond
//! conversion, audio timing, loss recovery, and fatal-player latching for every engine.
//! `LUMATV_GFN_SENDER_TIME=1` (off by default) also sends each VIDEO's own sender timestamp ahead of it, so the feeder can stamp the player on the
//! sender's timeline (gfn/feeder/src/gf_vclock.h); the hand-over time stays the key by which ACCEPT and DROP name the unit.
//! The TV's HDR mode follows the stream's own colour tags (`set_hdr_mode`), and leaving HDR for SDR loads the feeder again (`reload_feeder`);
//! what the session answered is only logged.
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use opennow_streamer_protocol::{
    AudioDevice, AudioOutputDevice, GraphicsAdapterCapability, RecordingCutReason, RenderSurface, ReplayBufferConfig,
    VideoBackendCapability,
};

use lumagame_feeder::{self as feeder, FeederClient, FeederMessage, Hdr, Load};
use crate::colour::{Colour, ColourWatch, HdrMode};
use crate::pacing::{self, Pacer, Report};
use crate::pads::PadBridge;
use crate::recording::{EncodedRecordingReceiver, ReplaySnapshot};
use crate::types::{
    CapturedInputQueue, DecodeStageTimings, DecodeTimingsReport, EncodedFrame, MediaCodec, MediaFeedback, MediaStreamConfig,
    MediaVideoCodec, PushOutcome,
};
use crate::{MicrophoneSession, video_backends};
use crate::vrr_capture::{Capture, Meta, NO_SLOT};

const BACKEND: &str = "ndl";
const LOAD_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaRuntimeControl {
    PointerLock,
}

/// Where the feeder is and how it is started: the installed app's feeder is the app's `main` and already serves; the host tests
/// and a standalone probe start one.
#[derive(Debug, Clone)]
pub struct FeederConfig {
    pub socket: PathBuf,
    pub spawn: Option<PathBuf>,
    pub spawn_args: Vec<String>,
    pub drop_threshold: i32,
    pub pts_unit: i32,
    pub max_nits: u32,
    pub max_average_nits: u32,
    /// Send each video unit's sender timestamp ahead of it (`LUMATV_GFN_SENDER_TIME=1`). Off by default: the player is stamped with the hand-over time.
    pub sender_time: bool,
}

impl FeederConfig {
    pub fn from_env() -> Self {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Self {
            socket: env("LUMAGAME_FEEDER_SOCKET").or_else(|| env("LUMATV_GFN_FEEDER_SOCKET"))
                .map_or_else(|| PathBuf::from("/tmp/lumatv-gfn/feed.sock"), PathBuf::from),
            spawn: env("LUMATV_GFN_FEEDER_SPAWN").map(PathBuf::from),
            spawn_args: env("LUMATV_GFN_FEEDER_ARGS").map(|a| a.split_whitespace().map(str::to_owned).collect()).unwrap_or_default(),
            drop_threshold: env("LUMATV_GFN_DROP_THRESHOLD").and_then(|v| v.parse().ok()).unwrap_or(2),
            pts_unit: feeder::pts_unit::MS,
            max_nits: env("LUMATV_GFN_HDR_MAX_NITS").and_then(|v| v.parse().ok()).unwrap_or(1000),
            max_average_nits: env("LUMATV_GFN_HDR_MAX_AVERAGE_NITS").and_then(|v| v.parse().ok()).unwrap_or(400),
            sender_time: env("LUMATV_GFN_SENDER_TIME").as_deref() == Some("1"),
        }
    }
}

/// The load for `stream`. HDR is armed for the codecs that carry it and never announced: the feeder enters the panel's HDR mode only for
/// metadata that names PQ or HLG, and which of them (or neither) a stream is comes from its own colour tags (`set_hdr_mode`), not from
/// what the session answered. A load therefore carries no metadata, and a stream that is SDR cannot end up in HDR mode.
fn load_for(stream: &MediaStreamConfig, cfg: &FeederConfig) -> Load {
    Load {
        codec: match stream.codec {
            MediaVideoCodec::H264 => feeder::codec::H264,
            MediaVideoCodec::H265 => feeder::codec::H265,
            MediaVideoCodec::Av1 => feeder::codec::AV1,
        },
        width: stream.width,
        height: stream.height,
        fps: stream.fps,
        audio: feeder::audio::OPUS,
        channels: 2,
        sample_rate: 48_000,
        hdr: u32::from(stream.codec != MediaVideoCodec::H264),
        hdr_info: Hdr::default(),
        drop_threshold: cfg.drop_threshold,
        pts_unit: cfg.pts_unit,
    }
}

/// Loads the feeder for the session's stream and waits for its answer: the first load of a session, and the reload that leaves an HDR
/// mode (`reload_feeder`), since a LOAD on a connection with a pipeline up stops that pipeline and starts the new one.
fn load_feeder(shared: &Shared, client: &mut FeederClient) -> Result<(), String> {
    let answers = shared.loaded.lock().unwrap_or_else(|e| e.into_inner());
    while answers.try_recv().is_ok() {}   // a pipeline error since the last load, with nobody waiting for it
    client.send_load(&load_for(&shared.stream, &shared.runtime.feeder)).map_err(|e| format!("feeder load: {e}"))?;
    match answers.recv_timeout(LOAD_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(text)) => Err(format!("the pipeline refused the stream: {text}")),
        Err(_) => Err("the feeder did not answer the load in time".to_owned()),
    }
}

struct RuntimeInner {
    feeder: FeederConfig,
    captured_input: Arc<CapturedInputQueue>,
    pads: PadBridge,
    muted: AtomicBool,
    paused: AtomicBool,
    child: Mutex<Option<Child>>,
    shutdown: Mutex<Option<Sender<()>>>,
    sessions: AtomicU64,
    /// The pacer's latest second of cadence, added to the next telemetry event.
    cadence: Mutex<Option<Report>>,
}

static RUNTIME: OnceLock<Arc<RuntimeInner>> = OnceLock::new();

/// Rumble from the engine's `controller-rumble` event (the binary routes it here before writing the event out).
pub fn controller_rumble(controller_id: u8, low_frequency: u16, high_frequency: u16, duration_ms: u32) -> Result<(), String> {
    let Some(rt) = RUNTIME.get() else { return Err("no media runtime".to_owned()) };
    rt.pads.rumble(controller_id, low_frequency, high_frequency, duration_ms)
}

/// The streamer's `telemetry` event, forwarded to the engine's side socket (the binary calls this before writing the event out).
pub fn stream_telemetry(telemetry: &serde_json::Value) {
    if let Some(rt) = RUNTIME.get() {
        match rt.cadence.lock().unwrap_or_else(|e| e.into_inner()).take() {
            Some(report) => {
                let mut telemetry = telemetry.clone();
                report.add_to(&mut telemetry);
                rt.pads.stream_telemetry(&telemetry);
            }
            None => rt.pads.stream_telemetry(telemetry),
        }
    }
}

/// The streamer's terminal `status` names a `termination` (the stream ended without being asked to); its cause goes to the
/// engine's side socket (the binary calls this before writing the event out).
pub fn stream_ended(termination: &serde_json::Value) {
    if let Some(rt) = RUNTIME.get() {
        rt.pads.stream_ended(termination);
    }
}

/// The streamer's `session-timer` (the far side's idle countdown) goes to the engine's side socket; the core passes it on only as an
/// event the engine never receives (the binary calls this before writing the event out).
pub fn stream_idle(timer: &serde_json::Value) {
    if let Some(rt) = RUNTIME.get() {
        rt.pads.stream_idle(timer);
    }
}

/// A `clip` line for the engine's side socket (clip.rs).
pub(crate) fn stream_clip(line: String) {
    if let Some(rt) = RUNTIME.get() {
        rt.pads.stream_clip(line);
    }
}

#[derive(Clone)]
pub struct MediaRuntime {
    inner: Arc<RuntimeInner>,
}

/// The main thread's part: upstream runs its window loop here; the TV has no window in this process, so it waits for the shutdown.
pub struct MainThreadHost {
    done: Receiver<()>,
}

impl MainThreadHost {
    pub fn run(self) {
        let _ = self.done.recv();
    }
}

pub fn create_runtime() -> Result<(MainThreadHost, MediaRuntime), String> {
    let captured_input = Arc::new(CapturedInputQueue::default());
    let (tx, rx) = mpsc::channel();
    let inner = Arc::new(RuntimeInner {
        feeder: FeederConfig::from_env(),
        pads: PadBridge::new(Arc::clone(&captured_input)),
        captured_input,
        muted: AtomicBool::new(false),
        paused: AtomicBool::new(false),
        child: Mutex::new(None),
        shutdown: Mutex::new(Some(tx)),
        sessions: AtomicU64::new(0),
        cadence: Mutex::new(None),
    });
    let _ = RUNTIME.set(Arc::clone(&inner));
    inner.pads.start();
    Ok((MainThreadHost { done: rx }, MediaRuntime { inner }))
}

impl MediaRuntime {
    pub fn published_video_frames(&self) -> Option<u64> {
        None
    }

    pub fn set_audio_muted(&self, muted: bool) {
        self.inner.muted.store(muted, Ordering::Release);
    }

    pub const fn is_embedded(&self) -> bool {
        false
    }

    pub fn video_backends(&self) -> Vec<VideoBackendCapability> {
        video_backends()
    }

    pub fn graphics_adapters(&self) -> Vec<GraphicsAdapterCapability> {
        Vec::new()
    }

    pub fn captured_input(&self) -> Arc<CapturedInputQueue> {
        Arc::clone(&self.inner.captured_input)
    }

    pub fn start_microphone(&self, _device_id: &str) -> Result<MicrophoneSession, String> {
        Err("the TV has no microphone input for the stream (the remote's voice key is LG's)".to_owned())
    }

    pub fn audio_devices(&self) -> Result<Vec<AudioDevice>, String> {
        Ok(vec![AudioDevice { id: String::new(), name: "TV sound output (the set's own setting)".to_owned() }])
    }

    pub fn validate_backend(&self, requested: &str) -> Result<(), String> {
        match requested.trim().to_ascii_lowercase().as_str() {
            "" | "auto" | "hardware" | BACKEND => Ok(()),
            other => Err(format!("the {other} backend does not exist on the TV; the stream decodes through {BACKEND}")),
        }
    }

    pub fn start(&self, feedback: Sender<MediaFeedback>, stream: MediaStreamConfig) -> Result<MediaSession, String> {
        self.start_with_audio_device(feedback, stream, "auto", AudioOutputDevice::default())
    }

    pub fn start_with_backend(&self, feedback: Sender<MediaFeedback>, stream: MediaStreamConfig, backend: &str) -> Result<MediaSession, String> {
        self.start_with_audio_device(feedback, stream, backend, AudioOutputDevice::default())
    }

    pub fn start_with_audio_device(
        &self,
        feedback: Sender<MediaFeedback>,
        stream: MediaStreamConfig,
        backend: &str,
        _audio_device: AudioOutputDevice,
    ) -> Result<MediaSession, String> {
        self.validate_backend(backend)?;
        self.ensure_feeder()?;
        let cfg = &self.inner.feeder;
        let (mut client, hello) = FeederClient::connect(&cfg.socket, Duration::from_secs(10)).map_err(|e| e.to_string())?;
        let FeederMessage::Hello { library, window_id, library_name, .. } = hello else { unreachable!() };
        if !library {
            return Err("the feeder has no NDL library: nothing can show the stream on this set".to_owned());
        }
        let smooth = pacing::smooth_from_prefs();
        opennow_streamer_protocol::log::log_line(
            "INFO",
            "ndl",
            &format!("feeder connected: window {window_id}, library {library_name}, codec {:?} {}x{} at {} fps, the session answered hdr {}, pacing {}",
                stream.codec, stream.width, stream.height, stream.fps, stream.hdr, if smooth { "smooth" } else { "latency" }),
        );
        let pacer = Pacer::new(stream.fps, smooth);
        let epoch = self.inner.sessions.load(Ordering::Acquire) as u32 + 1;
        let capture = Capture::arm(
            std::env::var_os("LUMATV_GFN_PREFS").map(PathBuf::from).as_deref(),
            epoch,
            Meta {
                build_id: String::new(),
                codec: match stream.codec { MediaVideoCodec::H264 => "H264", MediaVideoCodec::H265 => "H265", MediaVideoCodec::Av1 => "AV1" },
                width: stream.width,
                height: stream.height,
                fps: stream.fps,
                pacing: if smooth { "Smooth" } else { "Lowest latency" },
            },
        )
        .map(Arc::new);
        let (loaded_tx, loaded_rx) = mpsc::channel::<Result<(), String>>();
        let shared = Arc::new(Shared {
            feedback,
            stream,
            stopped: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            runtime: Arc::clone(&self.inner),
            in_flight: Mutex::new(VecDeque::new()),
            accepted: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            started_at: Instant::now(),
            last_status: Mutex::new(None),
            video_mid: Mutex::new(String::new()),
            pace: Mutex::new(Pace { pacer, held: VecDeque::new(), sending: false }),
            pace_wake: Condvar::new(),
            capture,
            colour: ColourWatch::default(),
            hdr_mode: Mutex::new(HdrMode::Sdr),
            loaded: Mutex::new(loaded_rx),
        });
        let reader_shared = Arc::clone(&shared);
        client
            .spawn_reader(move |m| reader_shared.on_message(m, &loaded_tx))
            .map_err(|e| format!("feeder reader: {e}"))?;
        // Before the load, so the tee sees the stream's shape and its first keyframe.
        if let Some(recorder) = crate::clip::arm(std::env::var_os("LUMATV_GFN_PREFS").map(PathBuf::from).as_deref()) {
            client.set_tee(recorder);
        }
        if let Err(e) = load_feeder(&shared, &mut client) {
            crate::clip::disarm();
            client.close();
            return Err(e);
        }
        self.inner.sessions.fetch_add(1, Ordering::Release);
        *self.inner.cadence.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let client = Arc::new(Mutex::new(Some(client)));
        if smooth {
            let (paced, feeder) = (Arc::clone(&shared), Arc::clone(&client));
            if let Err(e) = std::thread::Builder::new().name("gfn-pacer".to_owned()).spawn(move || pace_loop(&paced, &feeder)) {
                opennow_streamer_protocol::log::log_line("INFO", "ndl", &format!("frames go unpaced: no pacing thread ({e})"));
                shared.pace.lock().unwrap_or_else(|e| e.into_inner()).pacer = Pacer::new(shared.stream.fps, false);
            }
        }
        if let Some(capture) = shared.capture.clone() {
            let writer = Arc::clone(&shared);
            if let Err(e) = std::thread::Builder::new().name("gfn-capture".to_owned()).spawn(move || capture_writer(&writer, &capture)) {
                opennow_streamer_protocol::log::log_line("INFO", "ndl", &format!("the timing capture did not start: {e}"));
            }
        }
        self.inner.pads.set_active(true);
        self.inner.pads.set_grab(true);
        Ok(MediaSession {
            shared,
            client,
            captured_input: Arc::clone(&self.inner.captured_input),
            runtime: Arc::clone(&self.inner),
        })
    }

    pub fn update_surface(&self, _surface: RenderSurface) -> Result<(), String> {
        Ok(())   /* the picture fills the screen; the UI's window is the app's own */
    }

    pub fn set_paused(&self, paused: bool) -> Result<(), String> {
        self.inner.paused.store(paused, Ordering::Release);
        self.inner.pads.set_paused(paused);
        Ok(())
    }

    pub fn control(&self, control: MediaRuntimeControl) -> Result<(), String> {
        match control {
            MediaRuntimeControl::PointerLock => Err("pointer lock does not apply on the TV".to_owned()),
        }
    }

    pub fn shutdown(&self) {
        self.inner.pads.stop();
        if let Some(mut child) = self.inner.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(tx) = self.inner.shutdown.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = tx.send(());
        }
    }

    fn ensure_feeder(&self) -> Result<(), String> {
        let cfg = &self.inner.feeder;
        if cfg.socket.exists() {
            return Ok(());
        }
        let Some(bin) = cfg.spawn.as_ref() else {
            return Err(format!("no feeder is serving {} and none is configured to start (LUMATV_GFN_FEEDER_SPAWN)", cfg.socket.display()));
        };
        let mut child = self.inner.child.lock().unwrap_or_else(|e| e.into_inner());
        if child.as_mut().is_some_and(|c| c.try_wait().ok().flatten().is_none()) {
            return Ok(());
        }
        if let Some(dir) = cfg.socket.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let spawned = Command::new(bin)
            .args(&cfg.spawn_args)
            .arg("--serve")
            .arg(&cfg.socket)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("cannot start the feeder {}: {e}", bin.display()))?;
        *child = Some(spawned);
        Ok(())
    }
}

struct InFlight {
    pts_ns: i64,
    frame_index: Option<u32>,
    timestamp: u64,
    bytes: u32,
    keyframe: bool,
}

struct Shared {
    feedback: Sender<MediaFeedback>,
    stream: MediaStreamConfig,
    stopped: AtomicBool,
    paused: AtomicBool,
    playing: AtomicBool,
    runtime: Arc<RuntimeInner>,
    in_flight: Mutex<VecDeque<InFlight>>,
    accepted: AtomicU64,
    dropped: AtomicU64,
    started_at: Instant,
    video_mid: Mutex<String>,
    last_status: Mutex<Option<feeder::Status>>,
    pace: Mutex<Pace>,
    /// Wakes the pacing thread for a newly held frame, a finished write or the stop.
    pace_wake: Condvar,
    /// The armed diagnostics-only timing capture (vrr_capture.rs), `None` unless the UI armed one: the per-frame cost is this test.
    capture: Option<Arc<Capture>>,
    /// What the stream's keyframes say about their colour (colour.rs). The engine is told, and the transfer characteristic sets the TV's HDR
    /// mode (`set_hdr_mode`); what is forwarded never changes.
    colour: ColourWatch,
    /// The HDR mode the TV is in: SDR until the tags first name PQ or HLG, and SDR again once `reload_feeder` has left it.
    hdr_mode: Mutex<HdrMode>,
    /// The feeder's answers to a LOAD, and its pipeline errors: `load_feeder` waits on it, for the first load and for each reload.
    loaded: Mutex<Receiver<Result<(), String>>>,
}

/// Video frames waiting for their release time, oldest first. `sending` covers a write in progress outside the lock, so no later
/// frame overtakes it.
struct Pace {
    pacer: Pacer,
    held: VecDeque<(EncodedFrame, i64, u32)>,
    sending: bool,
}

impl Shared {
    fn on_message(&self, m: Option<FeederMessage>, loaded_tx: &Sender<Result<(), String>>) {
        match m {
            None => {
                if !self.stopped.swap(true, Ordering::AcqRel) {
                    let _ = self.feedback.send(MediaFeedback::OutputError { message: "the feeder went away".to_owned() });
                }
            }
            Some(FeederMessage::Loaded { ok, text }) => {
                let _ = loaded_tx.send(if ok { Ok(()) } else { Err(text) });
            }
            Some(FeederMessage::Accept(a)) => {
                self.accepted.fetch_add(1, Ordering::Relaxed);
                if let Some(f) = self.take_in_flight(a.pts_ns) {
                    let _ = self.feedback.send(MediaFeedback::VideoFrameAccepted {
                        frame_index: f.frame_index,
                        timestamp: f.timestamp,
                        bytes: f.bytes,
                        keyframe: f.keyframe,
                    });
                }
            }
            Some(FeederMessage::Drop(d)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                self.take_in_flight(d.pts_ns);
                let _ = self.feedback.send(MediaFeedback::QueueDropped { media: "video", count: 1 });
            }
            Some(FeederMessage::Keyframe) => {
                if !self.stopped.load(Ordering::Acquire) {
                    let _ = self.feedback.send(MediaFeedback::RequestKeyframe {
                        mid: self.video_mid.lock().unwrap_or_else(|e| e.into_inner()).clone(),
                        reason: "the feeder is holding video for a recovery anchor".to_owned(),
                    });
                }
            }
            Some(FeederMessage::Event { kind: 26, .. })
                if !self.stopped.load(Ordering::Acquire) && !self.playing.swap(true, Ordering::AcqRel) => {
                    let _ = self.feedback.send(MediaFeedback::PlaybackStarted { backend: BACKEND });
                }
            Some(FeederMessage::Status(s)) => {
                *self.last_status.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
                let _ = self.feedback.send(MediaFeedback::DecodeTimings(self.timings(&s)));
            }
            Some(FeederMessage::Error { code, text }) => {
                let _ = loaded_tx.send(Err(text.clone()));
                if !self.stopped.swap(true, Ordering::AcqRel) {
                    let _ = self.feedback.send(MediaFeedback::OutputError { message: format!("feeder error {code}: {text}") });
                }
            }
            Some(FeederMessage::Hello { .. } | FeederMessage::Pong { .. } | FeederMessage::Event { .. } | FeederMessage::Unknown { .. }) => {}
        }
    }

    fn take_in_flight(&self, pts_ns: i64) -> Option<InFlight> {
        let mut q = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        let mut discarded = 0;
        let found = loop {
            match q.pop_front() {
                None => break None,
                Some(front) if front.pts_ns == pts_ns => break Some(front),
                Some(front) if front.pts_ns > pts_ns => {
                    q.push_front(front);
                    break None;
                }
                Some(_) => discarded += 1,
            }
        };
        drop(q);
        if let Some(capture) = &self.capture {
            if discarded > 0 {
                capture.evicted(discarded);
            }
            if found.is_none() {
                capture.unmatched();
            }
        }
        found
    }

    /// The feeder's second as the engine's timing report: "call" is the time a unit waited on a full buffer, "residence" the render
    /// queue's depth in frame periods; outputs are the units the pipeline took.
    fn timings(&self, s: &feeder::Status) -> DecodeTimingsReport {
        let now = Instant::now();
        let period_us = 1_000_000 / u64::from(self.stream.fps.max(1));
        let samples = (s.video_fed + s.video_dropped) as usize;
        DecodeTimingsReport {
            call: Some(DecodeStageTimings { p50_us: 0, p95_us: u64::from(s.wait_p99_us), max_us: u64::from(s.wait_max_us) }),
            residence: Some(DecodeStageTimings {
                p50_us: u64::from(s.queue_mean_x100) * period_us / 100,
                p95_us: u64::from(s.queue_p99) * period_us,
                max_us: u64::from(s.queue_max) * period_us,
            }),
            call_window_samples: samples,
            residence_window_samples: samples,
            submissions_total: s.video_fed + s.video_dropped,
            outputs_total: s.video_fed,
            output_calls_total: s.video_fed,
            last_submission_at: Some(now),
            last_output_at: if s.playing != 0 && s.video_fed > 0 { Some(now) } else { None },
            in_flight: s.queue_max as usize,
            oldest_in_flight_at: None,
            epoch: 1,
            epoch_started_at: Some(self.started_at),
            unmatched_outputs: 0,
            unmatched_submissions: 0,
        }
    }
}

fn now_ns() -> i64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec * 1_000_000_000 + ts.tv_nsec
}

#[derive(Clone)]
pub struct MediaSink {
    shared: Arc<Shared>,
    client: Arc<Mutex<Option<FeederClient>>>,
}

impl MediaSink {
    pub fn push(&self, frame: EncodedFrame) -> PushOutcome {
        if self.shared.stopped.load(Ordering::Acquire) {
            return PushOutcome::Closed;
        }
        match frame.codec {
            MediaCodec::H264 | MediaCodec::H265 | MediaCodec::Av1 => {
                // Ahead of pace(), which consumes the frame: the colour is read whether the feeder gets the frame now, later or not at all,
                // and what the tags ask of the feeder (HDR metadata, or the reload that leaves HDR mode) is done before the keyframe that
                // carries them is sent.
                if let Some(colour) = self.shared.colour.observe(&frame, |line| self.shared.runtime.pads.stream_colour(line)) {
                    set_hdr_mode(&self.shared, &self.client, colour);
                    if self.shared.stopped.load(Ordering::Acquire) {
                        return PushOutcome::Closed;
                    }
                }
                self.pace(frame)
            }
            MediaCodec::Opus { .. } if self.shared.runtime.muted.load(Ordering::Acquire) => PushOutcome::Queued,
            MediaCodec::Opus { .. } => send(&self.shared, &self.client, &frame, false, NO_SLOT),
            MediaCodec::Unsupported(_) => PushOutcome::Unsupported,
        }
    }

    /// A video frame goes at its release time: straight away when it is due and nothing is held, otherwise behind the held frames
    /// on the pacing thread, which keeps their order.
    fn pace(&self, frame: EncodedFrame) -> PushOutcome {
        let mut mid = self.shared.video_mid.lock().unwrap_or_else(|e| e.into_inner());
        if *mid != frame.mid { mid.clone_from(&frame.mid); }
        drop(mid);
        let now = now_ns();
        let resync = frame.keyframe || !frame.contiguous;
        let mut pace = self.shared.pace.lock().unwrap_or_else(|e| e.into_inner());
        let release = pace.pacer.schedule(now, frame.timestamp, frame.clock_rate_hz, resync);
        let slot = self.shared.capture.as_ref().map_or(NO_SLOT, |c| c.enter(now, &frame, pace.held.len(), resync, release, pace.pacer.smooth()));
        if let Some(report) = pace.pacer.take_report() {
            *self.shared.runtime.cadence.lock().unwrap_or_else(|e| e.into_inner()) = Some(report);
        }
        if !pace.pacer.smooth() {
            drop(pace);
            return send(&self.shared, &self.client, &frame, true, slot);
        }
        if release <= now && pace.held.is_empty() && !pace.sending {
            pace.sending = true;
            drop(pace);
            let outcome = send(&self.shared, &self.client, &frame, true, slot);
            self.shared.pace.lock().unwrap_or_else(|e| e.into_inner()).sending = false;
            self.shared.pace_wake.notify_one();
            return outcome;
        }
        if resync {
            for held in &mut pace.held {
                held.1 = held.1.min(now);
            }
        }
        pace.held.push_back((frame, release, slot));
        drop(pace);
        self.shared.pace_wake.notify_one();
        PushOutcome::Queued
    }
}

/// Hands held video frames to the feeder at their release times, in order, until the session stops.
fn pace_loop(shared: &Shared, client: &Mutex<Option<FeederClient>>) {
    let mut pace = shared.pace.lock().unwrap_or_else(|e| e.into_inner());
    while !shared.stopped.load(Ordering::Acquire) {
        let now = now_ns();
        match pace.held.front().map(|held| held.1) {
            Some(at) if at <= now && !pace.sending => {
                let Some((frame, _, slot)) = pace.held.pop_front() else { continue };
                pace.sending = true;
                drop(pace);
                send(shared, client, &frame, true, slot);
                pace = shared.pace.lock().unwrap_or_else(|e| e.into_inner());
                pace.sending = false;
            }
            Some(at) if !pace.sending => {
                let wait = Duration::from_nanos(u64::try_from(at - now).unwrap_or(0));
                pace = shared.pace_wake.wait_timeout(pace, wait).unwrap_or_else(|e| e.into_inner()).0;
            }
            _ => pace = shared.pace_wake.wait(pace).unwrap_or_else(|e| e.into_inner()),
        }
    }
}

/// Writes one unit to the feeder. A video unit's pts is its hand-over time, by which the feeder's ACCEPT and DROP name it. With `sender_time` on, a
/// VIDEO_CLOCK naming that pts and carrying the sender's own timestamp is written right before the VIDEO, under the same lock.
fn send(shared: &Shared, client: &Mutex<Option<FeederClient>>, frame: &EncodedFrame, is_video: bool, slot: u32) -> PushOutcome {
    let mut guard = client.lock().unwrap_or_else(|e| e.into_inner());
    let Some(client) = guard.as_mut() else { return PushOutcome::Closed };
    let mut pts = now_ns();
    let mut in_flight = 0;
    if is_video {
        let mut q = shared.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(last) = q.back() {
            if pts <= last.pts_ns {
                pts = last.pts_ns + 1;   /* ACCEPT/DROP correlation must remain unique, even inside one clock tick. */
            }
        }
        q.push_back(InFlight {
            pts_ns: pts,
            frame_index: frame.frame_index,
            timestamp: frame.timestamp,
            bytes: frame.data.len() as u32,
            keyframe: frame.keyframe,
        });
        while q.len() > 64 {
            q.pop_front();
            if let Some(capture) = &shared.capture {
                capture.evicted(1);
            }
        }
        in_flight = q.len();
    }
    let kind = if is_video { feeder::msg::VIDEO } else { feeder::msg::AUDIO };
    let flush = is_video && !frame.contiguous && !frame.keyframe;
    let sent = (|| {
        if flush { client.send_flush()?; }
        if is_video && shared.runtime.feeder.sender_time {
            // the 32-bit RTP timestamp: the feeder unwraps it, so only its low bits matter
            client.send_video_clock(&feeder::VideoClock {
                pts_ns: pts,
                rtp: frame.timestamp as u32,
                clock_hz: frame.clock_rate_hz,
                flags: if frame.contiguous { 0 } else { feeder::video_clock::GAP },
                pad: 0,
            })?;
        }
        client.send_media(kind, pts, frame.keyframe, &frame.data)
    })();
    if slot != NO_SLOT {
        if let Some(capture) = &shared.capture {
            capture.handed(slot, pts, now_ns(), sent.is_ok(), flush, in_flight);
        }
    }
    match sent {
        Ok(()) => PushOutcome::Queued,
        Err(e) => {
            if !shared.stopped.swap(true, Ordering::AcqRel) {
                let _ = shared.feedback.send(MediaFeedback::OutputError { message: format!("feeder write: {e}") });
            }
            PushOutcome::Closed
        }
    }
}

/// The metadata the feeder is sent for `mode`: HDR10 as GFN streams it, and for HLG the same with HLG's transfer. SDR has none.
fn hdr_metadata(mode: HdrMode, cfg: &FeederConfig) -> Option<Hdr> {
    let transfer_characteristics = match mode {
        HdrMode::Sdr => return None,
        HdrMode::Hdr10 => 16,
        HdrMode::Hlg => 18,
    };
    Some(Hdr { transfer_characteristics, ..Hdr::hdr10(cfg.max_nits, cfg.max_average_nits) })
}

/// Puts the TV's HDR mode where the stream's own colour tags say, the only place it is decided (`load_for` arms the feeder and the
/// session's answer is only logged): PQ is HDR10, HLG is HLG, anything else is SDR. The metadata goes out before the keyframe that
/// carries the tags, so the feeder applies it with the first unit it accepts, as for a load that announced HDR, and nothing is entered
/// and left again. A later change from one HDR mode to the other, or from SDR to either, is the feeder's re-apply path, which re-enters
/// the panel's HDR mode and has dropped a 60 Hz session to a lower rate before (docs/research/rea-firmware-result.md, "Mid-stream HDR
/// update and the 60 Hz drop"). Tags that turn SDR after HDR load the feeder again before their keyframe is sent: the feeder cannot
/// leave an HDR mode otherwise, and SDR frames shown in it would be brighter than the owner's SDR level.
fn set_hdr_mode(shared: &Shared, client: &Mutex<Option<FeederClient>>, colour: Colour) {
    let wanted = colour.hdr_mode();
    let held = std::mem::replace(&mut *shared.hdr_mode.lock().unwrap_or_else(|e| e.into_inner()), wanted);
    let answered = shared.stream.hdr;
    let log = |note: String| {
        let level = if answered == (wanted != HdrMode::Sdr) { "INFO" } else { "WARN" };
        let said = format!("the stream's tags say {} (transfer {}) and the session answered hdr {answered}", wanted.label(), colour.transfer());
        opennow_streamer_protocol::log::log_async(level, "ndl", &format!("hdr: {said}: {note}"));
    };
    match (wanted, held) {
        (HdrMode::Sdr, HdrMode::Sdr) => log("the TV stays in SDR".to_owned()),
        (HdrMode::Sdr, held) => {
            log(format!("the feeder is loaded again to leave {}, which it cannot do otherwise", held.label()));
            let started = Instant::now();
            match reload_feeder(shared, client) {
                Ok(()) => log(format!("the feeder is loaded again after {} ms and the TV is in SDR", started.elapsed().as_millis())),
                Err(message) => {
                    opennow_streamer_protocol::log::log_async("WARN", "ndl", &format!("hdr: {message}"));
                    if !shared.stopped.swap(true, Ordering::AcqRel) {
                        let _ = shared.feedback.send(MediaFeedback::OutputError { message });
                    }
                }
            }
        }
        (wanted, held) if wanted == held => {}
        (wanted, held) => {
            let Some(hdr) = hdr_metadata(wanted, &shared.runtime.feeder) else { return };
            let sent = client.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(|client| client.send_hdr(&hdr));
            match sent {
                Some(Err(e)) => log(format!("the metadata could not be sent to the feeder: {e}")),
                _ if held == HdrMode::Sdr => log(format!("{} metadata sent ahead of the keyframe", wanted.label())),
                _ => log(format!("{} metadata sent again, from {}", wanted.label(), held.label())),
            }
        }
    }
}

/// Loads the feeder again on its open connection, the only way the TV leaves an HDR mode: the new pipeline is armed for HDR and carries
/// no metadata (`load_for`), so the panel stays in SDR until the tags name HDR again. Frames still held for their release belong to the
/// old picture and are dropped, which the engine is told; a frame the pacing thread has already taken is still written, after the load,
/// ahead of the keyframe the caller sends next. Writers wait on the connection meanwhile, and the feeder counts frames from zero again.
fn reload_feeder(shared: &Shared, client: &Mutex<Option<FeederClient>>) -> Result<(), String> {
    let stale = {
        let mut pace = shared.pace.lock().unwrap_or_else(|e| e.into_inner());
        let stale = pace.held.len();
        pace.held.clear();
        stale
    };
    if stale > 0 {
        let _ = shared.feedback.send(MediaFeedback::QueueDropped { media: "video", count: stale });
    }
    match client.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        Some(client) => load_feeder(shared, client).map_err(|e| format!("the feeder could not be loaded again to leave HDR: {e}")),
        None => Ok(()), // the session is over
    }
}

/// Writes the capture once its window and the grace after it are over, or when the stream ends first. Its own thread, so the file's
/// I/O is never on the frame path. A stream that ended before any frame leaves the arm request in place for the next stream.
fn capture_writer(shared: &Shared, capture: &Capture) {
    let ended = loop {
        if capture.finished(now_ns()) {
            break None;
        }
        if shared.stopped.load(Ordering::Acquire) {
            break Some("the stream stopped before the capture window ended");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let line = if capture.start_ns() == 0 {
        "the timing capture saw no video frame: nothing written".to_owned()
    } else {
        match capture.write(ended, now_ns()) {
            Ok(()) => format!("the timing capture is in {}", capture.out().display()),
            Err(e) => format!("the timing capture could not be written: {e}"),
        }
    };
    opennow_streamer_protocol::log::log_line("INFO", "ndl", &line);
}

pub struct MediaSession {
    shared: Arc<Shared>,
    client: Arc<Mutex<Option<FeederClient>>>,
    captured_input: Arc<CapturedInputQueue>,
    runtime: Arc<RuntimeInner>,
}

#[derive(Clone)]
pub struct MediaControl {
    shared: Arc<Shared>,
    client: Arc<Mutex<Option<FeederClient>>>,
    runtime: Arc<RuntimeInner>,
}

impl MediaSession {
    pub fn captured_input(&self) -> Arc<CapturedInputQueue> {
        Arc::clone(&self.captured_input)
    }

    pub fn sink(&self) -> MediaSink {
        MediaSink { shared: Arc::clone(&self.shared), client: Arc::clone(&self.client) }
    }

    pub fn control(&self) -> MediaControl {
        MediaControl { shared: Arc::clone(&self.shared), client: Arc::clone(&self.client), runtime: Arc::clone(&self.runtime) }
    }

    /// The overlay is up: the pads go to the UI instead of the game.
    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Release);
        self.runtime.pads.set_paused(paused);
    }

    pub fn stop(self) {
        self.control().stop();
    }
}

impl MediaControl {
    /// Adds the transport's RTCP sender-report summary to the armed timing capture's file; with no capture armed it does nothing.
    pub fn sender_reports_from(&self, source: impl Fn(i64) -> serde_json::Value + Send + Sync + 'static) {
        if let Some(capture) = &self.shared.capture {
            capture.attach_sender_reports(Box::new(source));
        }
    }

    /// Adds the transport's per-picture sender prefix rows to the armed timing capture's file. Returns whether a capture is armed, which
    /// is the caller's cue to start recording; with none armed nothing is attached.
    pub fn frame_prefixes_from(&self, source: impl Fn(i64) -> serde_json::Value + Send + Sync + 'static) -> bool {
        let Some(capture) = &self.shared.capture else { return false };
        capture.attach_frame_prefixes(Box::new(source));
        true
    }

    pub fn start_replay(&self, _config: ReplayBufferConfig, _reserved: Arc<std::sync::atomic::AtomicUsize>) {}

    pub fn stop_replay(&self) {}

    pub fn replay_snapshot(&self) -> Result<(MediaStreamConfig, ReplaySnapshot), &'static str> {
        Err("replay-not-enabled")
    }

    pub fn subscribe_recording(&self) -> Result<(MediaStreamConfig, EncodedRecordingReceiver), String> {
        Err("recording is not available on the TV".to_owned())
    }

    pub fn unsubscribe_recording(&self) {}

    pub fn cut_recording(&self, _reason: RecordingCutReason) {}

    pub fn update_cursor(&self, _bytes: Vec<u8>) {}

    pub fn stop(&self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.shared.pace.lock().unwrap_or_else(|e| e.into_inner()).held.clear();
        self.shared.pace_wake.notify_all();
        *self.runtime.cadence.lock().unwrap_or_else(|e| e.into_inner()) = None;
        crate::clip::disarm();
        self.runtime.pads.set_grab(false);
        self.runtime.pads.set_active(false);
        if let Some(mut client) = self.client.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = client.send_unload();
            client.close();
        }
    }
}

impl Drop for MediaSession {
    fn drop(&mut self) {
        self.control().stop();
    }
}

#[cfg(test)]
mod feeder_feedback_tests {
    use super::*;
    use crate::types::{MediaColorQuality, StreamShortcutBindings};
    use crate::colour::{SPS_BT709, SPS_FULL_NO_DESCRIPTION, SPS_HLG, SPS_PQ, hevc_keyframe};

    fn shared() -> (Shared, Receiver<MediaFeedback>, Sender<Result<(), String>>) {
        shared_with(false)
    }

    fn shared_with(sender_time: bool) -> (Shared, Receiver<MediaFeedback>, Sender<Result<(), String>>) {
        let captured_input = Arc::new(CapturedInputQueue::default());
        let runtime = Arc::new(RuntimeInner {
            feeder: FeederConfig { sender_time, ..FeederConfig::from_env() },
            pads: PadBridge::new(Arc::clone(&captured_input)),
            captured_input,
            muted: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            child: Mutex::new(None),
            shutdown: Mutex::new(None),
            sessions: AtomicU64::new(0),
            cadence: Mutex::new(None),
        });
        let (feedback, rx) = mpsc::channel();
        let (loaded, answers) = mpsc::channel();
        let shared = Shared {
            feedback,
            stream: MediaStreamConfig { codec: MediaVideoCodec::H265, color_quality: MediaColorQuality::TenBit420,
                hdr: true, width: 1920, height: 1080, fps: 120, bitrate_bps: 50_000_000, cloud_gsync: false,
                shortcuts: StreamShortcutBindings::default() },
            stopped: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            runtime,
            in_flight: Mutex::new(VecDeque::new()),
            capture: None,
            accepted: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            started_at: Instant::now(),
            video_mid: Mutex::new("video-0".to_owned()),
            last_status: Mutex::new(None),
            pace: Mutex::new(Pace { pacer: Pacer::new(120, true), held: VecDeque::new(), sending: false }),
            pace_wake: Condvar::new(),
            colour: ColourWatch::default(),
            hdr_mode: Mutex::new(HdrMode::Sdr),
            loaded: Mutex::new(answers),
        };
        (shared, rx, loaded)
    }

    #[test]
    fn fatal_player_failure_ends_output_once_and_eof_cannot_repeat_it() {
        let (shared, rx, loaded) = shared();
        shared.on_message(Some(FeederMessage::Event { kind: 18, num: 600, text: "decode failed".to_owned() }), &loaded);
        for _ in 0..2 {
            shared.on_message(Some(FeederMessage::Error { code: 2, text: "decode failed".to_owned() }), &loaded);
        }
        shared.on_message(None, &loaded);
        assert!(shared.stopped.load(Ordering::Acquire));
        let feedback: Vec<_> = rx.try_iter().collect();
        assert_eq!(feedback.len(), 1);
        assert!(matches!(&feedback[0], MediaFeedback::OutputError { message } if message.contains("decode failed")));
    }

    #[test]
    fn held_drops_do_not_flood_the_engine_with_keyframe_requests() {
        let (shared, rx, loaded) = shared();
        shared.on_message(Some(FeederMessage::Drop(feeder::Drop { pts_ns: 1, reason: 1, pad: 0 })), &loaded);
        shared.on_message(Some(FeederMessage::Keyframe), &loaded);
        shared.on_message(Some(FeederMessage::Drop(feeder::Drop { pts_ns: 2, reason: 4, pad: 0 })), &loaded);
        let feedback: Vec<_> = rx.try_iter().collect();
        assert_eq!(feedback.len(), 3);
        assert_eq!(feedback.iter().filter(|m| matches!(m, MediaFeedback::RequestKeyframe { .. })).count(), 1);
        assert!(matches!(&feedback[1], MediaFeedback::RequestKeyframe { mid, .. } if mid == "video-0"));
    }

    /// A feeder stand-in on a socket of its own: greets with protocol 2, answers each LOAD (`loaded`: the pipeline took it, or refused it),
    /// and keeps what the engine writes until the engine hangs up.
    fn fake_feeder(loaded: bool) -> (FeederClient, std::thread::JoinHandle<Vec<u8>>) {
        use std::io::{Read, Write};
        static SOCKETS: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("opennow-feeder-{}-{}.sock", std::process::id(), SOCKETS.fetch_add(1, Ordering::Relaxed)));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let mut hello = vec![23, 0, 0, 0, 112, 0, 0, 0];
            hello.extend_from_slice(&feeder::PROTO_VERSION.to_le_bytes());
            hello.resize(8 + 112, 0);
            peer.write_all(&hello).unwrap();
            let mut written = Vec::new();
            let mut head = [0u8; 8];
            while peer.read_exact(&mut head).is_ok() {
                let mut payload = vec![0u8; u32::from_le_bytes(head[4..].try_into().unwrap()) as usize];
                peer.read_exact(&mut payload).unwrap();
                written.extend_from_slice(&head);
                written.extend_from_slice(&payload);
                if head[0] == feeder::msg::LOAD {
                    let text: &[u8] = if loaded { b"window" } else { b"no pipeline" };
                    let mut answer = vec![feeder::msg::LOADED, 0, 0, 0];
                    answer.extend_from_slice(&(4 + text.len() as u32).to_le_bytes());
                    answer.extend_from_slice(&i32::from(loaded).to_le_bytes());
                    answer.extend_from_slice(text);
                    peer.write_all(&answer).unwrap();
                }
            }
            written
        });
        let (client, _) = FeederClient::connect(&path, Duration::from_secs(5)).unwrap();
        let _ = std::fs::remove_file(&path);
        (client, server)
    }

    /// Every (type, payload) message in what the engine wrote.
    fn frames(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            out.push((bytes[at], bytes[at + 8..at + 8 + len].to_vec()));
            at += 8 + len;
        }
        out
    }

    fn unit(timestamp: u64, keyframe: bool, contiguous: bool) -> EncodedFrame {
        EncodedFrame {
            mid: "video-0".into(), codec: MediaCodec::H265, data: Arc::from(vec![9u8; 5]), frame_index: Some(1), timestamp,
            clock_rate_hz: 90_000, keyframe, contiguous, ssrc: None,
        }
    }

    /// What the engine writes for a keyframe, a unit after a gap (which flushes first) and an audio packet. The timestamps are past the 32-bit wrap.
    fn written_for(sender_time: bool) -> Vec<(u8, Vec<u8>)> {
        let (shared, _rx, _loaded) = shared_with(sender_time);
        let (client, server) = fake_feeder(true);
        let client = Mutex::new(Some(client));
        let audio = EncodedFrame { codec: MediaCodec::Opus { channels: 2 }, ..unit(1, false, true) };
        for (frame, video) in [(unit(0x1_0000_0000 + 5000, true, true), true), (unit(0x1_0000_0000 + 5750, false, false), true), (audio, false)] {
            assert!(matches!(send(&shared, &client, &frame, video, NO_SLOT), PushOutcome::Queued));
        }
        drop(client);
        frames(&server.join().unwrap())
    }

    /// A VIDEO or AUDIO message without its key, which is a hand-over time and differs from run to run.
    fn without_key(message: &(u8, Vec<u8>)) -> (u8, Vec<u8>) {
        let keyed = message.0 == feeder::msg::VIDEO || message.0 == feeder::msg::AUDIO;
        (message.0, message.1[if keyed { 8 } else { 0 }..].to_vec())
    }

    #[test]
    fn with_sender_time_off_the_wire_carries_only_the_messages_it_always_did() {
        let types: Vec<u8> = written_for(false).iter().map(|m| m.0).collect();
        assert_eq!(types, [feeder::msg::VIDEO, feeder::msg::FLUSH, feeder::msg::VIDEO, feeder::msg::AUDIO]);
    }

    #[test]
    fn with_sender_time_on_each_video_follows_a_clock_naming_its_key_and_nothing_else_changes() {
        let key = |p: &[u8]| i64::from_le_bytes(p[..8].try_into().unwrap());
        let word = |p: &[u8], at: usize| u32::from_le_bytes(p[at..at + 4].try_into().unwrap());
        let on = written_for(true);
        let mut clocks = Vec::new();
        let mut rest = Vec::new();
        for (i, message) in on.iter().enumerate() {
            if message.0 == feeder::msg::VIDEO_CLOCK {
                assert_eq!(on[i + 1].0, feeder::msg::VIDEO);
                assert_eq!(key(&message.1), key(&on[i + 1].1));
                clocks.push((message.1.len(), word(&message.1, 8), word(&message.1, 12), word(&message.1, 16)));
            } else {
                rest.push(without_key(message));
            }
        }
        // the low 32 bits of the timestamps, the rate, and the gap flag on the unit that was not contiguous
        assert_eq!(clocks, [(24, 5000, 90_000, 0), (24, 5750, 90_000, feeder::video_clock::GAP)]);
        // everything else is what the switch-off run wrote, the keys apart
        assert_eq!(rest, written_for(false).iter().map(without_key).collect::<Vec<_>>());
    }

    /// A sink writing to a feeder stand-in whose answer to a LOAD is `loaded`, read as the runtime reads the feeder; the receiver is what the
    /// engine would be told. No pacing thread runs here, so frames go out as they arrive.
    fn sink_to_fake_feeder(answered_hdr: bool, loaded: bool) -> (MediaSink, std::thread::JoinHandle<Vec<u8>>, Receiver<MediaFeedback>) {
        let (mut shared, rx, answers) = shared();
        shared.stream.hdr = answered_hdr;
        shared.pace = Mutex::new(Pace { pacer: Pacer::new(120, false), held: VecDeque::new(), sending: false });
        let shared = Arc::new(shared);
        let (mut client, server) = fake_feeder(loaded);
        let reader = Arc::clone(&shared);
        client.spawn_reader(move |m| reader.on_message(m, &answers)).unwrap();
        (MediaSink { shared, client: Arc::new(Mutex::new(Some(client))) }, server, rx)
    }

    fn keyframe_with(sps: &str) -> EncodedFrame {
        EncodedFrame { data: Arc::from(hevc_keyframe(sps)), ..unit(0, true, true) }
    }

    #[derive(Debug, PartialEq)]
    enum Wire {
        Video,
        Hdr(Hdr),
        /// A LOAD: whether it arms HDR, and the metadata it carries.
        Load { armed: u32, metadata: Hdr },
    }

    fn hdr_of(w: &[u32]) -> Hdr {
        Hdr {
            display_primaries_x0: w[0], display_primaries_y0: w[1], display_primaries_x1: w[2], display_primaries_y1: w[3],
            display_primaries_x2: w[4], display_primaries_y2: w[5], white_point_x: w[6], white_point_y: w[7],
            max_display_mastering_luminance: w[8], min_display_mastering_luminance: w[9], max_content_light_level: w[10],
            max_pic_average_light_level: w[11], transfer_characteristics: w[12], color_primaries: w[13], matrix_coeffs: w[14],
        }
    }

    /// What the feeder stand-in was sent once the sink is gone: the video units, the HDR metadata and the loads, in order.
    fn wire(sink: MediaSink, server: std::thread::JoinHandle<Vec<u8>>) -> Vec<Wire> {
        drop(sink);
        let words = |payload: &[u8]| -> Vec<u32> { payload.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect() };
        frames(&server.join().unwrap())
            .into_iter()
            .map(|(kind, payload)| match kind {
                feeder::msg::VIDEO => Wire::Video,
                feeder::msg::HDR => {
                    assert_eq!(payload.len(), 60);
                    Wire::Hdr(hdr_of(&words(&payload)))
                }
                feeder::msg::LOAD => {
                    assert_eq!(payload.len(), 100);
                    let w = words(&payload);
                    Wire::Load { armed: w[7], metadata: hdr_of(&w[8..23]) }
                }
                other => panic!("message {other}"),
            })
            .collect()
    }

    #[test]
    fn pq_and_hlg_tags_put_their_metadata_on_the_wire_ahead_of_the_keyframe_whatever_the_session_answered() {
        for answered in [true, false] {
            for (what, sps, transfer) in [("PQ", SPS_PQ, 16), ("HLG", SPS_HLG, 18)] {
                let (sink, server, _rx) = sink_to_fake_feeder(answered, true);
                let cfg = sink.shared.runtime.feeder.clone();
                assert_eq!(sink.push(keyframe_with(sps)), PushOutcome::Queued);
                assert_eq!(sink.push(unit(1, false, true)), PushOutcome::Queued);
                let hdr = Hdr { transfer_characteristics: transfer, ..Hdr::hdr10(cfg.max_nits, cfg.max_average_nits) };
                assert_eq!(wire(sink, server), [Wire::Hdr(hdr), Wire::Video, Wire::Video], "{what}, the session answered hdr {answered}");
            }
        }
    }

    #[test]
    fn sdr_unspecified_and_unreadable_tags_send_no_metadata_even_when_the_session_answered_hdr() {
        for (what, keyframe) in [("BT.709", keyframe_with(SPS_BT709)), ("no colour description", keyframe_with(SPS_FULL_NO_DESCRIPTION)), ("no parameter set", unit(0, true, true))] {
            let (sink, server, _rx) = sink_to_fake_feeder(true, true);
            sink.push(keyframe);
            assert_eq!(wire(sink, server), [Wire::Video], "{what}");
        }
    }

    #[test]
    fn a_change_of_tags_sends_its_metadata_once_and_tags_that_turn_sdr_load_the_feeder_again_once() {
        let (sink, server, rx) = sink_to_fake_feeder(true, true);
        let cfg = sink.shared.runtime.feeder.clone();
        let hdr = |transfer| Wire::Hdr(Hdr { transfer_characteristics: transfer, ..Hdr::hdr10(cfg.max_nits, cfg.max_average_nits) });
        let mode = |sink: &MediaSink| *sink.shared.hdr_mode.lock().unwrap();
        for sps in [SPS_PQ, SPS_PQ, SPS_HLG] {
            assert_eq!(sink.push(keyframe_with(sps)), PushOutcome::Queued);
        }
        assert_eq!(mode(&sink), HdrMode::Hlg);
        // a unit still waiting for its release when the tags turn SDR belongs to the picture the reload ends
        sink.shared.pace.lock().unwrap().held.push_back((unit(5, false, true), i64::MAX, NO_SLOT));
        assert_eq!(sink.push(keyframe_with(SPS_BT709)), PushOutcome::Queued);
        assert_eq!((mode(&sink), sink.shared.pace.lock().unwrap().held.len()), (HdrMode::Sdr, 0));
        for sps in [SPS_BT709, SPS_PQ] {
            assert_eq!(sink.push(keyframe_with(sps)), PushOutcome::Queued);
        }
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [MediaFeedback::QueueDropped { media: "video", count: 1 }]);
        assert_eq!(wire(sink, server), [
            hdr(16), Wire::Video, // PQ
            Wire::Video,          // the same parameter set again: nothing new
            hdr(18), Wire::Video, // HLG: the feeder re-applies
            Wire::Load { armed: 1, metadata: Hdr::default() }, Wire::Video, // BT.709: loaded again, HDR armed and no metadata, the keyframe its first unit
            Wire::Video,          // BT.709 again: nothing new
            hdr(16), Wire::Video, // PQ again: the armed load takes metadata afresh
        ]);
    }

    #[test]
    fn a_refused_reload_ends_the_session_and_nothing_is_sent_after_it() {
        let (sink, server, rx) = sink_to_fake_feeder(true, false);
        assert_eq!(sink.push(keyframe_with(SPS_PQ)), PushOutcome::Queued);
        assert_eq!(sink.push(keyframe_with(SPS_BT709)), PushOutcome::Closed);
        assert_eq!(sink.push(unit(1, false, true)), PushOutcome::Closed);
        let told: Vec<_> = rx.try_iter().collect();
        assert!(matches!(&told[..], [MediaFeedback::OutputError { message }] if message.contains("leave HDR") && message.contains("no pipeline")), "{told:?}");
        assert!(matches!(&wire(sink, server)[..], [Wire::Hdr(_), Wire::Video, Wire::Load { .. }]));
    }

    #[test]
    fn the_load_arms_hdr_for_the_codecs_that_carry_it_and_carries_no_metadata() {
        let cfg = FeederConfig::from_env();
        for answered in [true, false] {
            for (codec, armed) in [(MediaVideoCodec::H265, 1), (MediaVideoCodec::Av1, 1), (MediaVideoCodec::H264, 0)] {
                let load = load_for(&MediaStreamConfig { codec, hdr: answered, ..MediaStreamConfig::default() }, &cfg);
                assert_eq!((load.hdr, load.hdr_info), (armed, Hdr::default()), "{codec:?}, the session answered hdr {answered}");
            }
        }
    }

    #[test]
    fn hdr10_metadata_is_gfns_and_hlg_differs_from_it_only_in_the_transfer() {
        let cfg = FeederConfig::from_env();
        assert_eq!(hdr_metadata(HdrMode::Sdr, &cfg), None);
        let pq = hdr_metadata(HdrMode::Hdr10, &cfg).unwrap();
        assert_eq!(pq, Hdr::hdr10(cfg.max_nits, cfg.max_average_nits));
        assert_eq!(hdr_metadata(HdrMode::Hlg, &cfg), Some(Hdr { transfer_characteristics: 18, ..pq }));
    }
}
