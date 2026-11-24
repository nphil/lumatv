//! What a job is, why it is not running, and what the engine reports: the vocabulary hosts speak to the engine in.

use serde::Serialize;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// A ring never holds more than this, whatever a job asks for: the ceiling that keeps the engine's memory bounded.
pub const MAX_RING_BYTES: usize = 128 << 20;

/// Which child to run and how long the engine waits for it.
#[derive(Clone, Debug)]
pub struct Config {
    /// The `lumatv-venc` binary, started as `<child> xcode`.
    pub child: PathBuf,
    pub limits: Limits,
}

/// Every timing and size the engine keeps to. The defaults are the TV's; tests scale the times to milliseconds.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Spawn to the child's ready record.
    pub ready: Duration,
    /// A ready child that sends no record at all for this long has stalled (it sends a statistics record every second).
    pub silence: Duration,
    /// Stop request (SIGTERM) to SIGKILL.
    pub term_grace: Duration,
    /// SIGKILL to the child still being alive: the hardware is wedged. `term_grace + kill_wait` is the most a stop takes.
    pub kill_wait: Duration,
    /// Restart delay of a job the engine restarts: the first, and the most it doubles to.
    pub backoff: (Duration, Duration),
    /// This many child failures within the window end the job that failed and open the circuit.
    pub flap: (u32, Duration),
    /// How long an open circuit refuses live starts.
    pub cooldown: Duration,
    /// Jobs that are not ended.
    pub max_jobs: usize,
    /// A delivery queue holds at most this many units...
    pub queue_units: usize,
    /// ...and this many bytes, whichever is reached first.
    pub queue_bytes: usize,
    /// The size of a ring whose job gives no size of its own (`max_bytes` 0).
    pub ring_bytes: usize,
    /// How often `Event::Bytes` is sent while a child runs.
    pub stats_every: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            ready: Duration::from_millis(1500),
            silence: Duration::from_secs(4),
            term_grace: Duration::from_millis(300),
            kill_wait: Duration::from_millis(700),
            backoff: (Duration::from_millis(250), Duration::from_secs(8)),
            flap: (4, Duration::from_secs(60)),
            cooldown: Duration::from_secs(30),
            max_jobs: 8,
            queue_units: 64,
            queue_bytes: 8 << 20,
            ring_bytes: 64 << 20,
            stats_every: Duration::from_secs(1),
        }
    }
}

/// Who gets the encoder when jobs compete: higher beats lower.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Priority {
    /// The continuous instant-replay ring.
    Replay,
    /// An explicit recording.
    Recording,
    /// A viewer watching now.
    Live,
}

impl Priority {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Replay => "replay",
            Self::Recording => "recording",
            Self::Live => "live",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum SourceCodec {
    Mpeg2,
    H264,
}

/// Where the capture tap takes the screen: video with the OSD, graphics only, or video after picture processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ScreenPoint {
    Blended,
    Osd,
    Display,
}

#[derive(Clone, Debug)]
pub enum Source {
    /// The owner pushes access units with `Job::push`. `width` x `height` is the largest decoded picture (0, 0: 1920x1088) and sizes the child's buffers.
    Units { codec: SourceCodec, width: u32, height: u32 },
    /// The composed screen through the capture tap. `lumatv-venc xcode` cannot take it yet, so `Engine::start` answers `StartError::Unsupported`; the
    /// variant exists so the API does not change when the child learns it.
    Screen { point: ScreenPoint },
}

/// H.264 profiles, numbered as `LX_VENC_H264_PROFILE_*` in `venc/venc_abi.h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum H264Profile {
    Baseline,
    Main,
    High,
}

/// What the encoder makes.
#[derive(Clone, Debug)]
pub struct Output {
    /// 0, 0: the decoded size; otherwise the decoded picture scaled down to it, never up.
    pub width: u32,
    pub height: u32,
    /// Encode every Nth picture; 0 is 1.
    pub keep_every: u32,
    /// The rate the rate control plans for.
    pub fps: u32,
    pub kbps: u32,
    /// Constant bit rate.
    pub cbr: bool,
    /// Pictures between IDRs; 0: only on `Job::force_idr`.
    pub gop: u32,
    pub profile: H264Profile,
    /// The shape of one output pixel, width and height, when it is not square: the child writes it into the SPS of every key unit as the pixel aspect
    /// ratio, so a player draws a 720x480 picture of a 16:9 show at 16:9 and not at 3:2. `None` writes none, and a player assumes square pixels.
    /// Both numbers must be above 0.
    pub sar: Option<(u16, u16)>,
}

/// What `Engine::start` does when the job cannot run now (the gate is closed, or a higher priority job has the encoder).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// Answer the error at once (a viewer who is told no, and asks again).
    Fail,
    /// Keep the job, paused with the reason, and run it when it can.
    Queue,
}

/// Where a job's units go.
#[derive(Clone, Debug)]
pub enum Sink {
    /// Units reach the owner through `Job::pull` and `Job::recv_timeout`, in a queue of `Limits::queue_units` and `Limits::queue_bytes`. A full queue drops
    /// units up to the next key unit, and the next unit delivered carries `discontinuity` (and `Event::Dropped` is sent).
    Deliver,
    /// Units stay in a ring inside the engine and `Engine::save_last` copies a clip out. `max_bytes` (0: `Limits::ring_bytes`) is at most
    /// [`MAX_RING_BYTES`]; `max_seconds` (0: no limit but the bytes) keeps at least that much once it is made, to the next key unit; `pts_hz` is the unit
    /// of the pts the job's units carry (90_000 for MPEG-TS, 1_000_000 for microseconds).
    Ring { max_bytes: usize, max_seconds: u32, pts_hz: u32 },
}

#[derive(Clone, Debug)]
pub struct JobSpec {
    /// Free text for status ("tuner 1.2.3.4", "lumagame").
    pub owner: String,
    pub priority: Priority,
    pub source: Source,
    pub output: Output,
    pub wait: Wait,
    pub sink: Sink,
}

/// Counts up from 1 and is never reused.
pub type JobId = u64;

/// One access unit in.
#[derive(Clone, Debug)]
pub struct Au {
    pub pts: u64,
    /// A random access point: the child starts decoding on one.
    pub key: bool,
    pub data: Vec<u8>,
}

/// One H.264 access unit out, as Annex B, in display order, with the pts of the access unit it was made from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Out {
    pub pts: u64,
    /// An IDR with SPS and PPS.
    pub key: bool,
    /// The unit follows a gap: the job was paused or restarted, or the queue dropped units before it.
    pub discontinuity: bool,
    /// What the encoder spent on this picture, in tenths of a millisecond.
    pub encode_tenths_ms: u32,
    pub data: Vec<u8>,
}

/// Why a job is not running, or why it ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Reason {
    /// A higher priority job has the encoder.
    Preempted { by: JobId },
    /// The gate is closed: `<key>: <reason>` of the first hold.
    Held(String),
    /// The child failed and the engine is backing off before it tries again.
    Restarting { fault: String },
    /// Waiting for the encoder.
    Queued,
    /// The owner stopped it, or dropped its handle.
    Stopped,
    /// A newer job of the same priority took over (live, replay).
    Replaced,
    /// The engine is shutting down.
    Shutdown,
    /// The child's failure ended the job.
    Fault(Fault),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preempted { by } => write!(f, "a higher priority job (#{by}) has the video hardware"),
            Self::Held(why) => write!(f, "the video hardware is on hold ({why})"),
            Self::Restarting { fault } => write!(f, "the converter failed ({fault}) and is starting again"),
            Self::Queued => f.write_str("waiting for the video hardware"),
            Self::Stopped => f.write_str("stopped"),
            Self::Replaced => f.write_str("replaced by a newer job"),
            Self::Shutdown => f.write_str("the engine is shutting down"),
            Self::Fault(fault) => fault.fmt(f),
        }
    }
}

/// A child's failure. Mirrors `lumatuner::transcode::Fault`, so a host maps one to the other 1:1; each message is a sentence a person can read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Fault {
    /// Not for this job: the child rejected its configuration or the stream changed size, a source it cannot take, or the circuit is open.
    Unsupported(String),
    /// The child or its hardware failed, spoke out of turn, did not start or did not become ready.
    Failed(String),
    /// Pictures went in and nothing came out, or the child went silent.
    Stalled(String),
    /// A call into the video hardware did not return, or the child outlived SIGKILL. The hardware may stay stuck until the TV restarts.
    Wedged(String),
}

impl Fault {
    pub fn message(&self) -> &str {
        let (Self::Unsupported(why) | Self::Failed(why) | Self::Stalled(why) | Self::Wedged(why)) = self;
        why
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Fault {}

/// What `Engine::start` answers when it does not return a running job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartError {
    /// The gate is closed and the job does not wait.
    Held(String),
    /// A higher priority job has the encoder (or another recording runs) and the job does not wait.
    Busy(String),
    /// The video hardware is wedged until the helper restarts.
    Wedged(String),
    /// The circuit is open after repeated child failures: a live start is refused for this much longer.
    Cooling(Duration),
    /// The job asks for something the engine or the child cannot do (a screen source, a ring above the ceiling).
    Unsupported(String),
    /// `Limits::max_jobs` jobs are alive.
    TooMany,
    /// The child did not spawn or did not become ready for a job that does not wait, or the engine is not running.
    Setup(Fault),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Held(why) => write!(f, "the video hardware is on hold ({why})"),
            Self::Busy(why) | Self::Unsupported(why) => f.write_str(why),
            Self::Wedged(why) => write!(f, "the video hardware stopped answering ({why})"),
            Self::Cooling(left) => write!(f, "the converter failed repeatedly and is resting for another {} s", left.as_secs() + 1),
            Self::TooMany => f.write_str("too many jobs at once"),
            Self::Setup(fault) => fault.fmt(f),
        }
    }
}

impl std::error::Error for StartError {}

/// Why `Job::push` did not take an access unit. It never blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushError {
    /// The job is not running (queued, held, preempted): the unit is dropped.
    Paused(Reason),
    /// After a (re)start the first unit must be a key unit: this one is dropped.
    WantKey,
    /// The child's input queue (32 units or 8 MiB) is full, or the unit is larger than a record can be: the unit is dropped.
    Full,
    Ended(Reason),
}

impl fmt::Display for PushError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Paused(why) | Self::Ended(why) => why.fmt(f),
            Self::WantKey => f.write_str("the converter needs a key picture first"),
            Self::Full => f.write_str("the converter is not taking pictures fast enough"),
        }
    }
}

impl std::error::Error for PushError {}

/// Why `Job::recv_timeout` returned no unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecvError {
    Timeout,
    /// The job ended and everything it made was taken.
    Ended(Reason),
}

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("no unit yet"),
            Self::Ended(why) => why.fmt(f),
        }
    }
}

impl std::error::Error for RecvError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipError {
    NoSuchJob,
    NotARing,
    Empty,
}

impl fmt::Display for ClipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoSuchJob => "no such job",
            Self::NotARing => "the job keeps no ring",
            Self::Empty => "nothing recorded yet",
        })
    }
}

impl std::error::Error for ClipError {}

/// Where a job is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum JobState {
    /// Waiting for the encoder (first start, or the child is starting).
    Queued,
    Running,
    Paused(Reason),
    Ended(Reason),
}

/// What the engine tells its subscribers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Event {
    /// A child is ready for the job (first start, or after a restart or resume).
    Started { job: JobId, restarts: u32, setup_ms: u32 },
    /// The job stopped running. `stopped_ms` is the stop request to the child being gone (0 if no child was running), so it is sent once the hardware is free.
    Paused { job: JobId, reason: Reason, stopped_ms: u32 },
    /// A paused job runs again; its first unit carries `discontinuity`.
    Resumed { job: JobId },
    /// Every `Limits::stats_every` while the job runs: what the child made so far and its rate over the last interval.
    Bytes { job: JobId, units: u64, bytes: u64, kbps: u32 },
    /// A full delivery queue dropped this many units since the last report.
    Dropped { job: JobId, units: u32 },
    Ended { job: JobId, reason: Reason },
    /// A gate input changed: `None` is a release.
    Hold { key: String, reason: Option<String> },
    /// The video hardware is wedged; sent once, and every later start is refused.
    Wedged,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum EngineState {
    #[default]
    Idle,
    Running,
    Held,
    Wedged,
    Cooling,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum JobKind {
    Queued,
    Running,
    Paused,
    Ended,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HoldInfo {
    pub key: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobInfo {
    pub id: JobId,
    pub owner: String,
    pub priority: Priority,
    pub state: JobKind,
    /// Why the job is paused or ended.
    pub reason: Option<Reason>,
    /// Milliseconds since the state last changed.
    pub since_ms: u64,
    /// What the child made for the job, all children together.
    pub units: u64,
    pub bytes: u64,
    /// The rate over the last statistics interval.
    pub kbps: u32,
    /// Child failures the engine restarted the job after.
    pub restarts: u32,
    pub last_fault: Option<Fault>,
    /// What a ring job holds.
    pub ring_bytes: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub state: EngineState,
    pub holds: Vec<HoldInfo>,
    /// The live jobs, and the last few that ended.
    pub jobs: Vec<JobInfo>,
    pub child_pid: Option<u32>,
    pub child_starts: u64,
    pub child_failures: u64,
    /// The latest stop request to the child being gone.
    pub last_stop_ms: Option<u32>,
    /// The last lines the current or latest child wrote to its log.
    pub child_log: Vec<String>,
    /// Milliseconds since the engine thread last ran: large when it is stuck (the snapshot is then an empty one).
    pub last_tick_ms: u64,
}

/// A copy of the newest units of a ring job, from a key unit.
#[derive(Clone, Debug)]
pub struct Clip {
    pub pts_hz: u32,
    /// Shared with the ring: cheap clones, never copies of the data.
    pub units: Vec<Arc<Out>>,
    /// From the first unit's pts to the last unit's.
    pub seconds: f32,
}
