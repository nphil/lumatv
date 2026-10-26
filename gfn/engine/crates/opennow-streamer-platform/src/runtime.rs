//! The media runtime for the TV: every encoded unit the engine hands over goes to lumatv-gfn-feeder over its socket, and what the
//! feeder reports (accepted, dropped, the pipeline's events and its per-second status) comes back as the engine's `MediaFeedback`.
//! Decoding, HDR and the video plane are LG's; this process never sees a picture (docs/research/opennow-result.md, "Which media API").
//!
//! The presentation clock is the arrival time: the ES player shows a unit when it arrives (jellyfin-webos-native docs/ndl.md) and the
//! PTS serves A/V alignment, so audio and video carry the same CLOCK_MONOTONIC reading taken when the engine pushed them.
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use opennow_streamer_protocol::{
    AudioDevice, AudioOutputDevice, GraphicsAdapterCapability, RecordingCutReason, RenderSurface, ReplayBufferConfig,
    VideoBackendCapability,
};

use crate::feeder::{self, FeederClient, FeederMessage, Hdr, Load};
use crate::pads::PadBridge;
use crate::recording::{EncodedRecordingReceiver, ReplaySnapshot};
use crate::types::{
    CapturedInputQueue, DecodeStageTimings, DecodeTimingsReport, EncodedFrame, MediaCodec, MediaFeedback, MediaStreamConfig,
    MediaVideoCodec, PushOutcome,
};
use crate::{MicrophoneSession, video_backends};

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
            socket: env("LUMATV_GFN_FEEDER_SOCKET").map_or_else(|| PathBuf::from("/tmp/lumatv-gfn/feed.sock"), PathBuf::from),
            spawn: env("LUMATV_GFN_FEEDER_SPAWN").map(PathBuf::from),
            spawn_args: env("LUMATV_GFN_FEEDER_ARGS").map(|a| a.split_whitespace().map(str::to_owned).collect()).unwrap_or_default(),
            drop_threshold: env("LUMATV_GFN_DROP_THRESHOLD").and_then(|v| v.parse().ok()).unwrap_or(2),
            pts_unit: match env("LUMATV_GFN_PTS_UNIT").as_deref() {
                Some("us") => feeder::pts_unit::US,
                Some("ms") => feeder::pts_unit::MS,
                _ => feeder::pts_unit::NS,
            },
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
}

static RUNTIME: OnceLock<Arc<RuntimeInner>> = OnceLock::new();

/// Rumble from the engine's `controller-rumble` event (the binary routes it here before writing the event out).
pub fn controller_rumble(controller_id: u8, low_frequency: u16, high_frequency: u16, duration_ms: u32) -> Result<(), String> {
    let Some(rt) = RUNTIME.get() else { return Err("no media runtime".to_owned()) };
    rt.pads.rumble(controller_id, low_frequency, high_frequency, duration_ms)
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
        opennow_streamer_protocol::log::log_line(
            "INFO",
            "ndl",
            &format!("feeder connected: window {window_id}, library {library_name}, codec {:?} {}x{} at {} fps, hdr {}",
                stream.codec, stream.width, stream.height, stream.fps, stream.hdr),
        );
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
        let shared = Arc::new(Shared {
            feedback,
            stream,
            stopped: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            muted: Arc::clone(&self.inner).muted_flag(),
            in_flight: Mutex::new(VecDeque::new()),
            accepted: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            started_at: Instant::now(),
            loaded: Mutex::new(None),
            last_status: Mutex::new(None),
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
        self.inner.pads.set_active(true);
        self.inner.pads.set_grab(true);
        Ok(MediaSession {
            shared,
            client: Arc::new(Mutex::new(Some(client))),
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

impl RuntimeInner {
    fn muted_flag(self: Arc<Self>) -> Arc<RuntimeInner> {
        self
    }
}

struct InFlight {
    pts_ns: i64,
    mid: String,
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
    muted: Arc<RuntimeInner>,
    in_flight: Mutex<VecDeque<InFlight>>,
    accepted: AtomicU64,
    dropped: AtomicU64,
    started_at: Instant,
    loaded: Mutex<Option<Sender<Result<(), String>>>>,
    last_status: Mutex<Option<feeder::Status>>,
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
                let mid = self.take_in_flight(d.pts_ns).map(|f| f.mid).unwrap_or_default();
                let _ = self.feedback.send(MediaFeedback::QueueDropped { media: "video", count: 1 });
                let _ = self.feedback.send(MediaFeedback::RequestKeyframe {
                    mid,
                    reason: match d.reason {
                        1 => "the pipeline's buffer stayed full".to_owned(),
                        2 => "the pipeline is not loaded".to_owned(),
                        _ => "the pipeline refused the unit".to_owned(),
                    },
                });
            }
            Some(FeederMessage::Event { kind, num, text }) => match kind {
                26 => {
                    if !self.playing.swap(true, Ordering::AcqRel) {
                        let _ = self.feedback.send(MediaFeedback::PlaybackStarted { backend: BACKEND });
                    }
                }
                18 => {
                    let _ = self.feedback.send(MediaFeedback::DecoderError {
                        codec: self.stream.codec.label(),
                        message: format!("pipeline error {num}: {text}"),
                    });
                }
                _ => {}
            },
            Some(FeederMessage::Status(s)) => {
                *self.last_status.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
                let _ = self.feedback.send(MediaFeedback::DecodeTimings(self.timings(&s)));
            }
            Some(FeederMessage::Error { code, text }) => {
                let _ = self.feedback.send(MediaFeedback::OutputError { message: format!("feeder error {code}: {text}") });
            }
            Some(FeederMessage::Hello { .. } | FeederMessage::Pong { .. } | FeederMessage::Unknown { .. }) => {}
        }
    }

    fn take_in_flight(&self, pts_ns: i64) -> Option<InFlight> {
        let mut q = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        while let Some(front) = q.pop_front() {
            if front.pts_ns == pts_ns {
                return Some(front);
            }
            if front.pts_ns > pts_ns {
                q.push_front(front);
                return None;
            }
        }
        None
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
        let (kind, is_video) = match frame.codec {
            MediaCodec::H264 | MediaCodec::H265 | MediaCodec::Av1 => (feeder::msg::VIDEO, true),
            MediaCodec::Opus { .. } => (feeder::msg::AUDIO, false),
            MediaCodec::Unsupported(_) => return PushOutcome::Unsupported,
        };
        if !is_video && self.shared.muted.muted.load(Ordering::Acquire) {
            return PushOutcome::Queued;
        }
        let mut pts = now_ns();
        let mut guard = self.client.lock().unwrap_or_else(|e| e.into_inner());
        let Some(client) = guard.as_mut() else { return PushOutcome::Closed };
        if is_video {
            let mut q = self.shared.in_flight.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(last) = q.back() {
                if pts <= last.pts_ns {
                    pts = last.pts_ns + 1;   /* two units inside one tick: the feeder wants a rising PTS */
                }
            }
            q.push_back(InFlight {
                pts_ns: pts,
                mid: frame.mid.clone(),
                frame_index: frame.frame_index,
                timestamp: frame.timestamp,
                bytes: frame.data.len() as u32,
                keyframe: frame.keyframe,
            });
            while q.len() > 64 {
                q.pop_front();
            }
        }
        match client.send_media(kind, pts, frame.keyframe, &frame.data) {
            Ok(()) => PushOutcome::Queued,
            Err(e) => {
                if !self.shared.stopped.swap(true, Ordering::AcqRel) {
                    let _ = self.shared.feedback.send(MediaFeedback::OutputError { message: format!("feeder write: {e}") });
                }
                PushOutcome::Closed
            }
        }
    }
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

    /// The overlay is up: the pads go to the UI instead of the game; the picture keeps running underneath.
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
        if self.shared.stopped.swap(true, Ordering::AcqRel) {
            let _ = self.client.lock().unwrap_or_else(|e| e.into_inner()).take().map(FeederClient::close);
            return;
        }
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
