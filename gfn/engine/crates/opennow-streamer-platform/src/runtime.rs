//! The media runtime for the TV: every encoded unit the engine hands over goes to lumatv-gfn-feeder over its socket, and what the
//! feeder reports (accepted, dropped, the pipeline's events and its per-second status) comes back as the engine's `MediaFeedback`.
//! Decoding, HDR and the video plane are LG's; this process never sees a picture (docs/research/opennow-result.md, "Which media API").
//!
//! VIDEO carries absolute CLOCK_MONOTONIC nanoseconds. The shared feeder owns load-relative millisecond
//! conversion, audio timing, loss recovery, and fatal-player latching for every engine.
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
        }
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
            &format!("feeder connected: window {window_id}, library {library_name}, codec {:?} {}x{} at {} fps, hdr {}, pacing {}",
                stream.codec, stream.width, stream.height, stream.fps, stream.hdr, if smooth { "smooth" } else { "latency" }),
        );
        let pacer = Pacer::new(stream.fps, smooth);
        let load = Load {
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
            hdr: u32::from(stream.hdr),
            hdr_info: if stream.hdr { Hdr::hdr10(cfg.max_nits, cfg.max_average_nits) } else { Hdr::default() },
            drop_threshold: cfg.drop_threshold,
            pts_unit: cfg.pts_unit,
        };
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
        });
        let (loaded_tx, loaded_rx) = mpsc::channel::<Result<(), String>>();
        let reader_shared = Arc::clone(&shared);
        client
            .spawn_reader(move |m| reader_shared.on_message(m, &loaded_tx))
            .map_err(|e| format!("feeder reader: {e}"))?;
        client.send_load(&load).map_err(|e| format!("feeder load: {e}"))?;
        match loaded_rx.recv_timeout(LOAD_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(text)) => {
                client.close();
                return Err(format!("the pipeline refused the stream: {text}"));
            }
            Err(_) => {
                client.close();
                return Err("the feeder did not answer the load in time".to_owned());
            }
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
            MediaCodec::H264 | MediaCodec::H265 | MediaCodec::Av1 => self.pace(frame),
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

/// Writes one unit to the feeder. A video unit's pts is its hand-over time, by which the feeder's ACCEPT and DROP name it.
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
        match capture.write(ended) {
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

    fn shared() -> (Shared, Receiver<MediaFeedback>, Sender<Result<(), String>>) {
        let captured_input = Arc::new(CapturedInputQueue::default());
        let runtime = Arc::new(RuntimeInner {
            feeder: FeederConfig::from_env(),
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
        let (loaded, _) = mpsc::channel();
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
}
