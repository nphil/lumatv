//! The game recorder's tee. The engine copies each coded video and audio unit it hands to the feeder into a bounded queue; one worker
//! thread files them into a RAM ring (whole GOPs, 108 MB). From the ring the stream goes to the receiver on the NAS as fragmented MP4:
//! continuously while long replay or a recording is on (the live feed), and as one clip of the last seconds otherwise. Nothing is
//! written on the TV. The decode path only ever calls [`Recorder::offer`], which copies the unit into the queue or drops it and counts
//! the drop; it never waits for the worker, the ring or the network. A NAS that stalls makes the live feed fall behind the ring and
//! stop: the viewer is told once, and Save clip goes back to the ring's own seconds.
//!
//! Memory budget, 128 MB in all: the ring ([`RING_CAP`]), the hand-off queue ([`HANDOFF_CAP`]) and the fragment being muxed
//! ([`FRAGMENT_CAP`]) by the live feed and by one clip. Both read the ring unit by unit and hold only their fragment's references.
mod bitstream;
mod cut;
mod fmp4;
mod http;
mod ring;

use std::io::{self, BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

pub use bitstream::Codec;
pub use http::HttpDestination;

use ring::{Read, Ring};
use cut::{Cutter, Move};

pub const TOTAL_CAP: usize = 128 << 20;
pub const HANDOFF_CAP: usize = 4 << 20;
pub const FRAGMENT_CAP: usize = 8 << 20;
pub const RING_CAP: usize = TOTAL_CAP - HANDOFF_CAP - 2 * FRAGMENT_CAP;
/// Units the hand-off queue may hold, whatever their size.
const QUEUE_UNITS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    Video,
    Audio,
}

/// HDR10 or HLG tags for the video track; the mastering block is present for PQ only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hdr10 {
    pub colour_primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub mastering: Option<Mastering>,
}

/// SMPTE ST 2086 and content light levels, in the units MP4's `mdcv` and `clli` boxes use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mastering {
    /// Green, blue, red as x, y.
    pub primaries: [u16; 6],
    pub white: [u16; 2],
    pub max_luminance: u32,
    pub min_luminance: u32,
    pub max_cll: u16,
    pub max_fall: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpusMapping {
    pub streams: u8,
    pub coupled: u8,
    pub map: [u8; 6],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioConfig {
    pub channels: u8,
    pub sample_rate: u32,
    /// Present for a surround layout; stereo and mono need none.
    pub mapping: Option<OpusMapping>,
}

/// What a clip needs to start playing: the stream's shape and the decoder record of the keyframe it starts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub record: Vec<u8>,
    pub hdr: Option<Hdr10>,
    pub audio: Option<AudioConfig>,
}

/// What the feeder was told about the stream: the tee keeps it to label the keyframes it files.
#[derive(Clone, Debug, Default)]
pub struct StreamInfo {
    pub codec: Option<Codec>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub hdr: Option<Hdr10>,
    pub audio: Option<AudioConfig>,
}

/// The receiver on the NAS. A `path` is a request's path with its query.
pub trait Destination: Send + Sync {
    /// Starts an upload; the writer it returns is told when the body is complete.
    fn open(&self, path: &str) -> io::Result<Box<dyn Upload>>;
    /// One short request.
    fn ask(&self, path: &str) -> Result<Answer, AskError>;
}

pub trait Upload: Write + Send {
    /// Ends the upload; an error means the receiver did not take it whole.
    fn finish(self: Box<Self>) -> io::Result<Answer>;
}

/// The receiver's answer: `key=value` lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer(Vec<(String, String)>);

impl Answer {
    pub fn parse(body: &str) -> Self {
        Self(body.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect())
    }
    /// The value; empty when the answer has no such line.
    pub fn get(&self, key: &str) -> &str {
        self.0.iter().find(|(k, _)| k == key).map_or("", |(_, v)| v)
    }
    pub fn number(&self, key: &str) -> u64 {
        self.get(key).parse().unwrap_or(0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskError {
    /// The receiver said no, in its own sentence.
    Refused(String),
    /// The receiver did not answer.
    Down(String),
}

/// What the viewer set for long replay and recordings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Long replay is on when the stream starts.
    pub replay: bool,
    /// How far back a clip from the NAS reaches.
    pub reach_s: u32,
    /// The stream's bitrate cap, by which the receiver decides whether the reach fits its buffer.
    pub mbps: u32,
    /// The longest one recording may run.
    pub record_max_s: u32,
    /// Seconds between the keyframes the recorder asks the game server for while it runs. 0 asks only when Save clip or Record needs
    /// one, and a clip then starts at the press and runs forward.
    pub keyframe_s: u32,
}

/// Asks the game server for a keyframe. The engine supplies it, because how the ask reaches the server differs by streaming service.
/// It returns at once and is called from the recorder's own threads, never from the decode path.
pub type KeyframeRequest = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Long replay or a recording went on or off, or a clip became possible or impossible: `ready` says the ring holds a keyframe to
    /// start a clip from (always true when clips start at the press).
    State { replay: bool, recording: bool, ready: bool },
    /// A clip or recording is on the NAS.
    Saved { name: String, seconds: u32, bytes: u64 },
    /// What the viewer asked for did not happen.
    Failed { reason: String },
    /// Something the viewer did not ask for just now and should know.
    Notice { text: String },
}

fn saved(name: &str, seconds: u32, bytes: u64) -> String {
    let length = if seconds < 120 { format!("{seconds} s") } else { format!("{} min", (seconds + 30) / 60) };
    let size = if bytes < 1 << 30 { format!("{} MB", bytes.div_ceil(1 << 20)) } else { format!("{:.1} GB", bytes as f64 / f64::from(1u32 << 30)) };
    format!("Saved {name}, {length}, {size}")
}

impl Event {
    /// What the viewer is told of this event: whether it is good news, and one sentence. A change of state needs no message.
    pub fn sentence(&self) -> Option<(bool, String)> {
        match self {
            Event::State { .. } => None,
            Event::Saved { name, seconds, bytes } => Some((true, saved(name, *seconds, *bytes))),
            Event::Failed { reason } => Some((false, reason.clone())),
            Event::Notice { text } => Some((false, text.clone())),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub offered: u64,
    /// Units refused at the hand-off because the worker was behind; each shows as a glitch in a clip that spans it.
    pub dropped: u64,
    pub ring_bytes: usize,
    pub ring_gops: usize,
    pub evicted_gops: u64,
    pub oversize: u64,
    pub clips: u64,
    pub failures: u64,
    /// A clip can start now.
    pub ready: bool,
    /// Keyframes asked of the game server.
    pub keyframe_asks: u64,
    /// The live feed reaches the receiver.
    pub live: bool,
    /// Times the live feed stopped because the receiver failed or fell behind.
    pub live_failures: u64,
}

enum Msg {
    Unit { track: Track, pts_ns: i64, key: bool, data: Arc<[u8]> },
    #[cfg(test)]
    Barrier(SyncSender<()>),
}

struct Shared {
    ring: Mutex<Ring>,
    arrived: Condvar,
    stream: Mutex<StreamInfo>,
    queued: AtomicUsize,
    offered: AtomicU64,
    dropped: AtomicU64,
    closed: AtomicBool,
    /// Whether a clip is being saved; one at a time keeps the fragment budget honest.
    busy: AtomicBool,
    clips: AtomicU64,
    failures: AtomicU64,
    live: Mutex<Live>,
    /// Wakes the live feed's thread while it idles or waits to try again.
    live_wake: Condvar,
    live_failures: AtomicU64,
    /// A reach the receiver shortened is said once a stream.
    reach_told: AtomicBool,
    plan: Plan,
    keyframes: KeyframeRequest,
    /// Milliseconds on the recorder's own clock; a fake one in the tests.
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    cutter: Mutex<Cutter>,
    /// No keyframe is asked for before this time: the stream showed trouble.
    quiet_until: AtomicU64,
    /// The ring holds a keyframe to start a clip from; always true when clips start at the press.
    ready: AtomicBool,
    video_seen: AtomicBool,
    asked: AtomicU64,
    /// Urgent waits for a keyframe that were given up on.
    gave_up: AtomicU64,
    destination: Arc<dyn Destination>,
    on_event: Box<dyn Fn(Event) + Send + Sync>,
}

/// What is wanted of the live feed, and what it has.
#[derive(Default)]
struct Live {
    want_replay: bool,
    /// The game a recording was asked for, until the receiver starts it.
    want_record: Option<String>,
    /// Where the ring stood, and how many waits for a keyframe had been given up on, when the recording was asked for: it starts at
    /// the first keyframe from there on, and fails if a wait is given up on first.
    record_from: u64,
    record_gen: u64,
    stop_record: bool,
    /// The feed reaches the receiver, as the stream it knows by `session`.
    up: bool,
    session: String,
    /// The reach the receiver granted; 0 while its buffer is off.
    reach_s: u32,
    recording: bool,
}

impl Live {
    fn wanted(&self) -> bool {
        self.want_replay || self.want_record.is_some() || self.recording
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Recorder {
    shared: Arc<Shared>,
    tx: SyncSender<Msg>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Busy,
    NoThread,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::Busy => "a clip is already being saved",
            Refusal::NoThread => "the clip could not be started",
        })
    }
}

impl Recorder {
    pub fn new(ring_cap: usize, destination: Arc<dyn Destination>, plan: Plan, keyframes: KeyframeRequest, on_event: impl Fn(Event) + Send + Sync + 'static) -> Self {
        let epoch = Instant::now();
        Self::with_clock(ring_cap, destination, plan, keyframes, Arc::new(move || epoch.elapsed().as_millis() as u64), on_event)
    }

    fn with_clock(
        ring_cap: usize,
        destination: Arc<dyn Destination>,
        plan: Plan,
        keyframes: KeyframeRequest,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
        on_event: impl Fn(Event) + Send + Sync + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::sync_channel(QUEUE_UNITS);
        let shared = Arc::new(Shared {
            ring: Mutex::new(Ring::new(ring_cap)),
            arrived: Condvar::new(),
            stream: Mutex::new(StreamInfo::default()),
            queued: AtomicUsize::new(0),
            offered: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            clips: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            live: Mutex::new(Live { want_replay: plan.replay, ..Live::default() }),
            live_wake: Condvar::new(),
            live_failures: AtomicU64::new(0),
            reach_told: AtomicBool::new(false),
            plan,
            keyframes,
            cutter: Mutex::new(Cutter::new(plan.keyframe_s, clock())),
            clock,
            quiet_until: AtomicU64::new(0),
            ready: AtomicBool::new(plan.keyframe_s == 0),
            video_seen: AtomicBool::new(false),
            asked: AtomicU64::new(0),
            gave_up: AtomicU64::new(0),
            destination,
            on_event: Box::new(on_event),
        });
        let worker = shared.clone();
        thread::Builder::new().name("lumagame-tee".to_owned()).spawn(move || worker_loop(&worker, &rx)).expect("spawn the tee worker");
        let live = shared.clone();
        thread::Builder::new().name("lumagame-live".to_owned()).spawn(move || live_loop(&live)).expect("spawn the live feed");
        Self { shared, tx }
    }

    /// The stream the feeder was loaded with, and from the next keyframe on the label of what is filed. Not for the decode path.
    pub fn set_stream(&self, info: StreamInfo) {
        *lock(&self.shared.stream) = info;
    }

    /// Updates the HDR tags of the stream; takes effect at the next keyframe.
    pub fn set_hdr(&self, hdr: Option<Hdr10>) {
        lock(&self.shared.stream).hdr = hdr;
    }

    /// Copies one coded unit toward the ring. Called on the decode path: it takes no lock another thread can hold, never waits, and when
    /// the hand-off is full it drops the unit and counts it. A drop inside a GOP marks that GOP damaged, so clips prefer to start elsewhere.
    pub fn offer(&self, track: Track, pts_ns: i64, key: bool, data: &[u8]) {
        let s = &self.shared;
        s.offered.fetch_add(1, Ordering::Relaxed);
        let len = data.len();
        if s.queued.fetch_add(len, Ordering::Relaxed) + len > HANDOFF_CAP {
            s.queued.fetch_sub(len, Ordering::Relaxed);
            s.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) = self.tx.try_send(Msg::Unit { track, pts_ns, key, data: Arc::from(data) }) {
            s.queued.fetch_sub(len, Ordering::Relaxed);
            s.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn stats(&self) -> Stats {
        let s = &self.shared;
        let live = lock(&s.live).up;
        let ring = lock(&s.ring);
        Stats {
            offered: s.offered.load(Ordering::Relaxed),
            dropped: s.dropped.load(Ordering::Relaxed),
            ring_bytes: ring.bytes(),
            ring_gops: ring.gops(),
            evicted_gops: ring.evicted_gops,
            oversize: ring.oversize,
            clips: s.clips.load(Ordering::Relaxed),
            failures: s.failures.load(Ordering::Relaxed),
            ready: s.ready.load(Ordering::Relaxed),
            keyframe_asks: s.asked.load(Ordering::Relaxed),
            live,
            live_failures: s.live_failures.load(Ordering::Relaxed),
        }
    }

    /// Whether long replay is on for this stream, whether a recording runs, and whether a clip can start now.
    pub fn state(&self) -> (bool, bool, bool) {
        let live = lock(&self.shared.live);
        (live.want_replay, live.recording, self.shared.ready.load(Ordering::Relaxed))
    }

    /// Tells the recorder the stream is in trouble: the decoder is recovering or pictures were lost on the way. It asks the game server
    /// for no keyframe until things have been quiet a few seconds. Two atomics and no lock, so any thread may call it.
    pub fn trouble(&self) {
        let s = &self.shared;
        s.quiet_until.fetch_max((s.clock)() + cut::HOLD_MS, Ordering::Relaxed);
    }

    /// Turns long replay on or off for this stream.
    pub fn set_replay(&self, on: bool) {
        lock(&self.shared.live).want_replay = on;
        self.shared.live_wake.notify_all();
        tell_state(&self.shared);
    }

    /// Asks the NAS to record `game`, starting at a keyframe that arrives from now on: the recorder asks the game server for one at
    /// once. What became of it comes as [`Event`]s from the live feed's thread.
    pub fn start_recording(&self, game: &str) {
        let s = &self.shared;
        let from = lock(&s.ring).next_seq();
        let mut live = lock(&s.live);
        let asked = !live.recording;
        if asked {
            live.want_record = Some(game.to_owned());
            live.stop_record = false;
            live.record_from = from;
            live.record_gen = s.gave_up.load(Ordering::Relaxed);
        }
        drop(live);
        if asked {
            need_keyframe(s);
        }
        s.live_wake.notify_all();
    }

    pub fn stop_recording(&self) {
        let mut live = lock(&self.shared.live);
        live.want_record = None;
        live.stop_record = live.recording;
    }

    /// Saves a clip of `game` as one file: the last minutes out of the NAS's buffer while long replay reaches it, and otherwise, or when
    /// the NAS cannot give it, `seconds` out of the ring. By default these are the last `seconds` (at least that much when the ring holds
    /// it, starting on a keyframe); with [`Plan::keyframe_s`] 0 the clip starts at the press, on a keyframe the recorder asks for, and
    /// runs `seconds` forward. The answer comes as an [`Event`] from the job's thread.
    pub fn save_clip(&self, game: &str, seconds: u32) -> Result<(), Refusal> {
        let s = self.shared.clone();
        if s.busy.swap(true, Ordering::AcqRel) {
            return Err(Refusal::Busy);
        }
        let cut = {
            let live = lock(&s.live);
            (live.up && live.reach_s > 0).then(|| format!("/save?session={}&seconds={}&game={}", live.session, live.reach_s, http::encode(game)))
        };
        let game = game.to_owned();
        let spawned = thread::Builder::new().name("lumagame-clip".to_owned()).spawn(move || {
            let from_nas = cut.and_then(|path| s.destination.ask(&path).ok()).map(|told| Event::Saved {
                name: told.get("name").to_owned(),
                seconds: told.number("seconds") as u32,
                bytes: told.number("bytes"),
            });
            let ring_span = if s.plan.keyframe_s == 0 { Span::Forward(seconds) } else { Span::Back(seconds) };
            let event = match from_nas.map_or_else(|| clip_from_ring(&s, &game, ring_span), Ok) {
                Ok(saved) => {
                    s.clips.fetch_add(1, Ordering::Relaxed);
                    saved
                }
                Err(reason) => {
                    s.failures.fetch_add(1, Ordering::Relaxed);
                    Event::Failed { reason }
                }
            };
            s.busy.store(false, Ordering::Release);
            (s.on_event)(event);
        });
        if spawned.is_err() {
            self.shared.busy.store(false, Ordering::Release);
            return Err(Refusal::NoThread);
        }
        Ok(())
    }

    /// Blocks until the worker has filed everything offered before this call.
    #[cfg(test)]
    fn sync(&self) {
        let (tx, rx) = mpsc::sync_channel(1);
        self.tx.send(Msg::Barrier(tx)).unwrap();
        rx.recv().unwrap();
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        self.shared.arrived.notify_all();
        self.shared.live_wake.notify_all();
    }
}

fn worker_loop(s: &Arc<Shared>, rx: &Receiver<Msg>) {
    let mut last_config: Option<Arc<Config>> = None;
    let mut seen_drops = 0;
    let mut paced = (s.clock)();
    while !s.closed.load(Ordering::Acquire) {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Msg::Unit { track, pts_ns, key, data }) => file_unit(s, &mut last_config, &mut seen_drops, track, pts_ns, key, data),
            #[cfg(test)]
            Ok(Msg::Barrier(done)) => {
                pace(s);
                let _ = done.send(());
                continue;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        let now = (s.clock)();
        if now.saturating_sub(paced) >= PACE_MS {
            paced = now;
            pace(s);
        }
    }
}

/// How often the worker looks at the cutter.
const PACE_MS: u64 = 100;

fn file_unit(s: &Shared, last_config: &mut Option<Arc<Config>>, seen_drops: &mut u64, track: Track, pts_ns: i64, key: bool, data: Arc<[u8]>) {
    s.queued.fetch_sub(data.len(), Ordering::Relaxed);
    let video = track == Track::Video;
    if video {
        s.video_seen.store(true, Ordering::Relaxed);
    }
    let config = (video && key).then(|| config_for(s, &data, last_config)).flatten();
    let usable_key = config.is_some();
    let mut ring = lock(&s.ring);
    let drops = s.dropped.load(Ordering::Relaxed);
    if drops != *seen_drops {
        *seen_drops = drops;
        ring.mark_damaged();
        // Units lost at the hand-off are trouble of the recorder's own: a keyframe asked for now would be one more large unit to lose.
        s.quiet_until.fetch_max((s.clock)() + cut::HOLD_MS, Ordering::Relaxed);
    }
    let outgrown = ring.oversize;
    ring.push(track, pts_ns, key, data, config);
    let cleared = ring.oversize != outgrown;
    let ready = (s.plan.keyframe_s != 0 && (usable_key || cleared)).then(|| ring.ready());
    drop(ring);
    s.arrived.notify_all();
    if usable_key {
        lock(&s.cutter).key_arrived((s.clock)());
    }
    if let Some(ready) = ready {
        if s.ready.swap(ready, Ordering::Relaxed) != ready {
            tell_state(s);
        }
    }
    if cleared {
        need_keyframe(s);
    }
}

/// Runs the cutter: sends the keyframe request when one is due, and wakes whoever waits when an urgent one is given up on. Called from
/// the worker and from the threads that need a keyframe, never from the decode path; the request goes out with no lock held.
fn pace(s: &Shared) {
    let (ready, video) = (s.ready.load(Ordering::Relaxed), s.video_seen.load(Ordering::Relaxed));
    let verdict = lock(&s.cutter).poll((s.clock)(), s.quiet_until.load(Ordering::Relaxed), ready, video);
    match verdict {
        Move::Idle => {}
        Move::Ask => {
            s.asked.fetch_add(1, Ordering::Relaxed);
            (s.keyframes)();
        }
        Move::GaveUp => {
            s.gave_up.fetch_add(1, Ordering::Relaxed);
            s.arrived.notify_all();
            s.live_wake.notify_all();
        }
    }
}

/// Something waits for a keyframe now: ask for one at once.
fn need_keyframe(s: &Shared) {
    lock(&s.cutter).urgent((s.clock)());
    pace(s);
}

/// The config a keyframe files under: the stream's shape and the keyframe's own parameter sets; the previous one when nothing changed.
fn config_for(s: &Shared, key_unit: &[u8], last: &mut Option<Arc<Config>>) -> Option<Arc<Config>> {
    let info = lock(&s.stream).clone();
    let codec = info.codec?;
    let record = bitstream::record(codec, key_unit)?.record;
    let config = Config { codec, width: info.width, height: info.height, fps: info.fps, record, hdr: info.hdr, audio: info.audio };
    if last.as_deref() != Some(&config) {
        *last = Some(Arc::new(config));
    }
    last.clone()
}

type ClipMuxer = fmp4::Muxer<BufWriter<Box<dyn Upload>>>;

/// Closes the last fragment and the upload; the bytes muxed and the receiver's answer.
fn end_upload(mut mux: ClipMuxer) -> Result<(u64, Answer), String> {
    mux.close().map_err(|e| format!("upload: {e}"))?;
    let bytes = mux.bytes;
    let writer = mux.finish().map_err(|e| format!("upload: {e}"))?;
    let upload = writer.into_inner().map_err(|e| format!("upload: {}", e.error()))?;
    let told = upload.finish().map_err(|e| format!("the NAS did not take it: {e}"))?;
    Ok((bytes, told))
}

/// Which part of the stream a clip from the ring takes.
#[derive(Clone, Copy)]
enum Span {
    /// The last seconds: from the newest keyframe at or before that far back, to what the ring holds now.
    Back(u32),
    /// From a keyframe that arrives after the press, this many seconds on.
    Forward(u32),
}

/// A clip stops growing here whatever its length: the receiver refuses a clip larger than it can hold, and one so large is a recording.
const CLIP_MAX_BYTES: u64 = 176 << 20;

/// Waits for the ring to hold what `found` looks for, asking the game server for a keyframe first. Fails when the ask is given up on.
fn await_keyframe<T>(s: &Shared, found: impl Fn(&Ring) -> Option<T>) -> Result<T, String> {
    let gave_up = s.gave_up.load(Ordering::Relaxed);
    let mut asked = false;
    let mut ring = lock(&s.ring);
    loop {
        if let Some(found) = found(&ring) {
            return Ok(found);
        }
        if s.gave_up.load(Ordering::Relaxed) != gave_up || s.closed.load(Ordering::Acquire) {
            return Err("Save clip could not start: the game sent no keyframe to start from".to_owned());
        }
        if asked {
            ring = s.arrived.wait_timeout(ring, Duration::from_millis(100)).unwrap_or_else(|e| e.into_inner()).0;
        } else {
            asked = true;
            drop(ring);
            need_keyframe(s);
            ring = lock(&s.ring);
        }
    }
}

/// One clip of the ring, from a keyframe, uploaded as one file.
fn clip_from_ring(s: &Arc<Shared>, game: &str, span: Span) -> Result<Event, String> {
    if !s.video_seen.load(Ordering::Relaxed) {
        return Err("no picture has been recorded yet".to_owned());
    }
    let (start, end, forward_s) = match span {
        Span::Back(seconds) => {
            let start = await_keyframe(s, |ring| ring.start_for_last(seconds))?;
            (start, Some(lock(&s.ring).next_seq()), 0)
        }
        Span::Forward(seconds) => {
            let from = lock(&s.ring).next_seq();
            (await_keyframe(s, |ring| ring.first_key_from(from))?, None, seconds)
        }
    };
    let (first_config, mut seq) = match lock(&s.ring).read(start) {
        Read::Unit(_, Some(config)) => (config, start),
        _ => return Err("the picture to start from was already dropped".to_owned()),
    };
    let upload = s.destination.open(&format!("/clip?game={}", http::encode(game))).map_err(|e| format!("cannot reach the NAS: {e}"))?;
    let mut mux = fmp4::Muxer::new(BufWriter::with_capacity(64 << 10, upload), first_config).map_err(|e| format!("upload: {e}"))?;
    let (mut first_pts, mut last_pts) = (None, 0i64);
    let until = (s.clock)() + (u64::from(forward_s) + 5) * 1000;
    while end.is_none_or(|end| seq < end) {
        let read = {
            let mut ring = lock(&s.ring);
            let mut read = ring.read(seq);
            if matches!(read, Read::Wait) && end.is_none() && (s.clock)() < until && !s.closed.load(Ordering::Acquire) {
                ring = s.arrived.wait_timeout(ring, Duration::from_millis(100)).unwrap_or_else(|e| e.into_inner()).0;
                read = ring.read(seq);
            }
            read
        };
        match read {
            Read::Unit(unit, config) => {
                if config.as_ref().is_some_and(|c| **c != **mux.config()) {
                    break; // the stream's shape or parameter sets changed: the file ends where its sample entry stops being true
                }
                debug_assert_eq!(unit.seq, seq);
                mux.push(&unit).map_err(|e| format!("upload: {e}"))?;
                if unit.track == Track::Video {
                    first_pts.get_or_insert(unit.pts_ns);
                    last_pts = unit.pts_ns;
                }
                seq += 1;
                let long_enough = first_pts.is_some_and(|f| last_pts - f >= i64::from(forward_s) * 1_000_000_000);
                if end.is_none() && (long_enough || mux.bytes >= CLIP_MAX_BYTES) {
                    break;
                }
            }
            Read::Wait if end.is_some() || (s.clock)() >= until || s.closed.load(Ordering::Acquire) => break,
            Read::Wait => {}
            Read::Evicted => return Err("the upload fell behind the stream and the ring dropped the rest".to_owned()),
        }
    }
    let (bytes, told) = end_upload(mux)?;
    let seconds = first_pts.map_or(0, |f| ((last_pts - f + 500_000_000) / 1_000_000_000) as u32);
    Ok(Event::Saved { name: told.get("name").to_owned(), seconds, bytes })
}

/// How long the live feed waits before it tries the NAS again: the first wait, doubled each time up to the longest.
const RETRY: (Duration, Duration) = (Duration::from_secs(5), Duration::from_secs(60));
/// A feed that ran this long was working: the next outage is a new one, said again and retried from the first wait.
const STEADY: Duration = Duration::from_secs(60);
/// How often a running recording is asked after: the receiver ends one by itself at its longest length or when the share fills.
const RECORD_CHECK: Duration = Duration::from_secs(5);

fn pause(s: &Shared, time: Duration) {
    let guard = lock(&s.live);
    drop(s.live_wake.wait_timeout(guard, time));
}

fn tell_state(s: &Shared) {
    let (replay, recording) = {
        let live = lock(&s.live);
        (live.want_replay, live.recording)
    };
    (s.on_event)(Event::State { replay, recording, ready: s.ready.load(Ordering::Relaxed) });
}

/// Says once when the receiver's buffer holds less than the reach asked for.
fn reach_granted(s: &Shared, told: &Answer) {
    let note = told.get("note");
    if !note.is_empty() && !s.reach_told.swap(true, Ordering::Relaxed) {
        (s.on_event)(Event::Notice { text: note.to_owned() });
    }
}

/// The receiver closed a recording: the viewer learns what it saved, and why when it ended on the receiver's own account.
fn record_ended(s: &Shared, told: &Answer, prefix: &str) {
    lock(&s.live).recording = false;
    tell_state(s);
    let (name, note) = (told.get(&format!("{prefix}name")).to_owned(), told.get(&format!("{prefix}note")).to_owned());
    let (seconds, bytes) = (told.number(&format!("{prefix}seconds")) as u32, told.number(&format!("{prefix}bytes")));
    (s.on_event)(match (name.is_empty(), note.is_empty()) {
        (true, true) => Event::Failed { reason: "Nothing was recorded".to_owned() },
        (true, false) => Event::Failed { reason: note },
        (false, true) => Event::Saved { name, seconds, bytes },
        (false, false) => Event::Notice { text: format!("{note}. {}", saved(&name, seconds, bytes)) },
    });
}

/// A recording waits for a keyframe the game never sent: it is given up on, and the viewer told.
fn record_timed_out(s: &Shared) {
    let mut live = lock(&s.live);
    if live.want_record.is_some() && s.gave_up.load(Ordering::Relaxed) != live.record_gen {
        live.want_record = None;
        drop(live);
        (s.on_event)(Event::Failed { reason: "Record could not start: the game sent no keyframe".to_owned() });
    }
}

/// The live feed's thread: it runs the feed while long replay or a recording wants it, and after a failure says so once and tries
/// again later. A failed recording is not retried; long replay is.
fn live_loop(s: &Arc<Shared>) {
    let (mut retry, mut told) = (RETRY.0, false);
    while !s.closed.load(Ordering::Acquire) {
        if !lock(&s.live).wanted() {
            pause(s, Duration::from_millis(500));
            continue;
        }
        let began = Instant::now();
        let result = feed(s);
        let (was_recording, wanted_record) = {
            let mut live = lock(&s.live);
            let was = (live.recording, live.want_record.is_some());
            live.up = false;
            live.reach_s = 0;
            live.recording = false;
            live.stop_record = false;
            if result.is_err() {
                live.want_record = None;
            }
            was
        };
        if began.elapsed() >= STEADY {
            (retry, told) = (RETRY.0, false);
        }
        let Err(reason) = result else { continue };
        s.live_failures.fetch_add(1, Ordering::Relaxed);
        let text = if was_recording {
            tell_state(s);
            Some(format!("Recording stopped: {reason}"))
        } else if wanted_record {
            Some(format!("Recording could not start: {reason}"))
        } else {
            (!told).then(|| format!("Long replay is paused: {reason}. Save clip keeps the TV's own last seconds."))
        };
        told = true;
        if let Some(text) = text {
            (s.on_event)(Event::Notice { text });
        }
        pause(s, retry);
        retry = (retry * 2).min(RETRY.1);
    }
}

/// One run of the live feed, from the newest keyframe on, until nothing wants it, the stream changes shape, or the NAS fails.
fn feed(s: &Arc<Shared>) -> Result<(), String> {
    let start = {
        let ring = lock(&s.ring);
        ring.start_of_open_gop().and_then(|seq| match ring.read(seq) {
            Read::Unit(_, Some(config)) => Some((config, seq)),
            _ => None,
        })
    };
    let Some((config, mut seq)) = start else {
        record_timed_out(s);
        pause(s, Duration::from_millis(200));
        return Ok(());
    };
    let down = || "the NAS is not answering".to_owned();
    let ask = |path: String| {
        s.destination.ask(&path).map_err(|e| match e {
            AskError::Refused(note) => note,
            AskError::Down(_) => down(),
        })
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let session = format!("{now:x}-{:x}", std::process::id());
    let plan = s.plan;
    let mut replay = lock(&s.live).want_replay;
    let opened = ask(format!("/open?session={session}&replay={}&reach={}&mbps={}", u8::from(replay), plan.reach_s, plan.mbps))?;
    reach_granted(s, &opened);
    let upload = s.destination.open(&format!("/live?session={session}")).map_err(|_| down())?;
    let mut mux: ClipMuxer = fmp4::Muxer::new(BufWriter::with_capacity(64 << 10, upload), config).map_err(|_| down())?;
    {
        let mut live = lock(&s.live);
        live.up = true;
        live.session.clone_from(&session);
        live.reach_s = opened.number("reach") as u32;
    }
    let mut checked = Instant::now();
    loop {
        record_timed_out(s);
        let (want_replay, stop, recording, waiting) = {
            let mut live = lock(&s.live);
            let stop = std::mem::take(&mut live.stop_record) && live.recording;
            (live.want_replay, stop, live.recording, live.want_record.is_some())
        };
        if s.closed.load(Ordering::Acquire) || !(want_replay || recording || waiting) {
            break;
        }
        if want_replay != replay {
            let told = ask(format!("/replay?session={session}&on={}&reach={}&mbps={}", u8::from(want_replay), plan.reach_s, plan.mbps))?;
            replay = want_replay;
            lock(&s.live).reach_s = told.number("reach") as u32;
            reach_granted(s, &told);
        }
        if stop {
            record_ended(s, &ask(format!("/record/stop?session={session}"))?, "");
        } else if recording && checked.elapsed() >= RECORD_CHECK {
            checked = Instant::now();
            let status = ask(format!("/status?session={session}"))?;
            if status.get("recording") != "1" {
                record_ended(s, &status, "last_");
            }
        }
        let read = {
            let mut ring = lock(&s.ring);
            let mut read = ring.read(seq);
            if matches!(read, Read::Wait) {
                // Re-checked under the lock, so a unit that arrives between the checks cannot be missed for long.
                let (guard, _) = s.arrived.wait_timeout(ring, Duration::from_millis(100)).unwrap_or_else(|e| e.into_inner());
                ring = guard;
                read = ring.read(seq);
            }
            read
        };
        match read {
            Read::Unit(unit, config) => {
                if config.as_ref().is_some_and(|c| **c != **mux.config()) {
                    break; // the stream changed shape: this upload ends where its sample entry stops being true, and the next one begins
                }
                if unit.track == Track::Video && unit.key {
                    // A recording asked for starts at the first keyframe from then on: the receiver begins a file at the first keyframe
                    // it is sent after it is told, so telling it just before this one is filed makes this the start.
                    let game = {
                        let mut live = lock(&s.live);
                        if live.recording || unit.seq < live.record_from { None } else { live.want_record.take() }
                    };
                    if let Some(game) = game {
                        match s.destination.ask(&format!("/record/start?session={session}&game={}&max={}", http::encode(&game), plan.record_max_s)) {
                            Ok(_) => {
                                lock(&s.live).recording = true;
                                checked = Instant::now();
                                tell_state(s);
                            }
                            Err(AskError::Refused(note)) => (s.on_event)(Event::Failed { reason: note }),
                            Err(AskError::Down(_)) => {
                                lock(&s.live).want_record = Some(game);
                                return Err(down());
                            }
                        }
                    }
                }
                mux.push(&unit).map_err(|_| down())?;
                seq += 1;
            }
            Read::Wait => {}
            Read::Evicted => return Err("the NAS is not keeping up with the stream".to_owned()),
        }
    }
    // The receiver ends a recording with its stream; asked first, it says what it saved.
    if lock(&s.live).recording {
        if let Ok(told) = s.destination.ask(&format!("/record/stop?session={session}")) {
            record_ended(s, &told, "");
        }
    }
    end_upload(mux).map(|_| ()).map_err(|_| down())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const SC: [u8; 4] = [0, 0, 0, 1];
    const SPS: [u8; 21] = [0x67, 0x64, 0x00, 0x1f, 0xac, 0xd9, 0x40, 0x50, 0x05, 0xbb, 0x01, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10, 0x00, 0x00, 0x03, 0x01];
    const PPS: [u8; 4] = [0x68, 0xeb, 0xe3, 0xcb];
    pub const H264_IDR: [u8; 6] = [0x65, 0x88, 0x84, 0x00, 0x20, 0xff];

    pub fn nals(parts: &[&[u8]]) -> Vec<u8> {
        parts.iter().flat_map(|p| SC.iter().chain(p.iter()).copied().collect::<Vec<_>>()).collect()
    }
    /// An access unit delimiter, SPS, PPS and an IDR slice.
    pub fn h264_key() -> Vec<u8> {
        nals(&[&[0x09, 0xf0], &SPS, &PPS, &H264_IDR])
    }
    pub fn h264_delta(n: u8) -> Vec<u8> {
        nals(&[&[0x41, 0x9a, n, 0x10, 0x20]])
    }
    pub fn opus_unit() -> Vec<u8> {
        vec![0xfc, 1, 2, 3, 4, 5, 6, 7]
    }
    pub fn test_config() -> Arc<Config> {
        h264_config()
    }
    pub fn h264_config() -> Arc<Config> {
        let record = bitstream::h264_record(&h264_key()).unwrap().record;
        Arc::new(Config {
            codec: Codec::H264,
            width: 1920,
            height: 1080,
            fps: 30,
            record,
            hdr: None,
            audio: Some(AudioConfig { channels: 2, sample_rate: 48_000, mapping: None }),
        })
    }

    /// A receiver that keeps what it is sent and answers with the real one's lines.
    #[derive(Default)]
    struct Memory {
        /// Finished uploads, by request path.
        files: Mutex<Vec<(String, Vec<u8>)>>,
        asked: Mutex<Vec<String>>,
        /// No upload can be opened.
        refuse: AtomicBool,
        /// Short requests get no answer.
        deaf: AtomicBool,
        /// The live upload's writes wait.
        stall: AtomicBool,
        /// The reach `/open` and `/replay` grant, and their note.
        grant: Mutex<(u32, &'static str)>,
    }
    struct MemoryFile {
        path: String,
        bytes: Vec<u8>,
        to: Arc<Memory>,
    }
    impl Write for MemoryFile {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            while self.path.starts_with("/live") && self.to.stall.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1));
            }
            self.bytes.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Upload for MemoryFile {
        fn finish(self: Box<Self>) -> io::Result<Answer> {
            lock(&self.to.files).push((self.path, self.bytes));
            Ok(Answer::parse("name=Hades/clip.mp4\n"))
        }
    }
    struct MemoryDestination(Arc<Memory>);
    impl Destination for MemoryDestination {
        fn open(&self, path: &str) -> io::Result<Box<dyn Upload>> {
            if self.0.refuse.load(Ordering::Relaxed) {
                return Err(io::Error::new(io::ErrorKind::ConnectionRefused, "refused"));
            }
            Ok(Box::new(MemoryFile { path: path.to_owned(), bytes: Vec::new(), to: self.0.clone() }))
        }
        fn ask(&self, path: &str) -> Result<Answer, AskError> {
            if self.0.deaf.load(Ordering::Relaxed) {
                return Err(AskError::Down("no answer".to_owned()));
            }
            lock(&self.0.asked).push(path.to_owned());
            let (reach, note) = *lock(&self.0.grant);
            Ok(Answer::parse(&match path.split('?').next().unwrap() {
                "/open" | "/replay" => format!("reach={reach}\nnote={note}\n"),
                "/save" => "name=Hades/cut.mp4\nseconds=300\nbytes=2147483648\n".to_owned(),
                "/record/start" => "name=Hades/rec.mp4\n".to_owned(),
                "/record/stop" => "name=Hades/rec.mp4\nseconds=7\nbytes=1048576\nnote=\n".to_owned(),
                "/status" => "recording=1\n".to_owned(),
                _ => return Err(AskError::Refused("No such request".to_owned())),
            }))
        }
    }

    const OFF: Plan = Plan { replay: false, reach_s: 300, mbps: 65, record_max_s: 7200, keyframe_s: 5 };

    /// A recorder on a clock the test moves, with a game server that counts the keyframes it is asked for.
    struct Rig {
        r: Recorder,
        memory: Arc<Memory>,
        events: Receiver<Event>,
        clock: Arc<AtomicU64>,
        asks: Arc<AtomicU64>,
    }

    impl Rig {
        /// Moves the clock on and lets the worker look at it.
        fn advance(&self, ms: u64) {
            self.clock.fetch_add(ms, Ordering::Relaxed);
            self.r.sync();
        }
        fn asked(&self) -> u64 {
            self.asks.load(Ordering::Relaxed)
        }
        /// One 30 fps second from `from_s`: delta pictures, or with `key` a keyframe first, as the game server answers an ask.
        fn second(&self, from_s: i64, key: bool) {
            for f in 0..30 {
                let pts = from_s * 1_000_000_000 + f * 33_333_333;
                if f == 0 && key {
                    self.r.offer(Track::Video, pts, true, &h264_key());
                } else {
                    self.r.offer(Track::Video, pts, false, &h264_delta(f as u8));
                }
            }
            self.r.sync();
        }
    }

    fn rig(cap: usize, plan: Plan) -> Rig {
        let memory = Arc::new(Memory { grant: Mutex::new((plan.reach_s, "")), ..Memory::default() });
        let (tx, events) = mpsc::channel();
        let tx = Mutex::new(tx);
        let (clock, asks) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        let (now, counted) = (clock.clone(), asks.clone());
        let r = Recorder::with_clock(
            cap,
            Arc::new(MemoryDestination(memory.clone())),
            plan,
            Arc::new(move || {
                counted.fetch_add(1, Ordering::Relaxed);
            }),
            Arc::new(move || now.load(Ordering::Relaxed)),
            move |e| {
                let _ = lock(&tx).send(e);
            },
        );
        let c = h264_config();
        r.set_stream(StreamInfo { codec: Some(Codec::H264), width: c.width, height: c.height, fps: 30, hdr: None, audio: c.audio.clone() });
        Rig { r, memory, events, clock, asks }
    }

    fn recorder(cap: usize, plan: Plan) -> (Recorder, Arc<Memory>, Receiver<Event>) {
        let Rig { r, memory, events, .. } = rig(cap, plan);
        (r, memory, events)
    }

    fn next(events: &Receiver<Event>) -> Event {
        events.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    /// The next event that is news to the viewer: not a change of state.
    fn told(events: &Receiver<Event>) -> Event {
        loop {
            let event = next(events);
            if !matches!(event, Event::State { .. }) {
                return event;
            }
        }
    }

    fn wait(done: impl Fn() -> bool) {
        let until = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < until, "waited five seconds");
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// A keyframe of 400 KB: a stream of these outgrows a small ring within seconds.
    fn big_key() -> Vec<u8> {
        let mut v = h264_key();
        v.extend(std::iter::repeat_n(1u8, 400_000));
        v
    }

    /// Feeds `seconds` of 30 fps video with a keyframe each second, plus 20 ms audio, starting at `from_s`.
    fn feed(r: &Recorder, from_s: i64, seconds: i64) {
        for s in from_s..from_s + seconds {
            for f in 0..30 {
                let pts = s * 1_000_000_000 + f * 33_333_333;
                if f == 0 {
                    r.offer(Track::Video, pts, true, &h264_key());
                } else {
                    r.offer(Track::Video, pts, false, &h264_delta(f as u8));
                }
                for a in 0..2 {
                    r.offer(Track::Audio, pts + a * 16_000_000, false, &opus_unit());
                }
            }
            r.sync();
        }
    }

    fn video_samples(file: &[u8]) -> usize {
        use crate::fmp4::parse::*;
        boxes(file, 0)
            .iter()
            .filter(|b| &b.ty == b"moof")
            .flat_map(|m| boxes(m.body, 0).into_iter().filter(|b| &b.ty == b"traf").collect::<Vec<_>>())
            .filter(|t| u32::from_be_bytes(child(t, b"tfhd", 0).unwrap().body[4..8].try_into().unwrap()) == 1)
            .map(|t| u32::from_be_bytes(child(&t, b"trun", 0).unwrap().body[4..8].try_into().unwrap()) as usize)
            .sum()
    }

    #[test]
    fn a_saved_clip_starts_on_a_keyframe_and_covers_at_least_the_seconds_asked() {
        let (r, memory, events) = recorder(RING_CAP, OFF);
        feed(&r, 0, 10);
        r.save_clip("Hades II", 3).unwrap();
        let Event::Saved { name, seconds, .. } = told(&events) else { panic!() };
        assert_eq!(name, "Hades/clip.mp4");
        let files = lock(&memory.files);
        let (path, file) = &files[0];
        assert_eq!(path, "/clip?game=Hades%20II");
        // The newest picture is 9.97 s in; the keyframe at or before 6.97 s is 6.0: four whole seconds of 30 pictures.
        assert_eq!(video_samples(file), 120);
        assert_eq!(seconds, 4);
        assert!(file.windows(4).any(|w| w == b"avcC"));
        assert!(lock(&memory.asked).is_empty() && !r.stats().live, "with long replay off nothing else goes to the NAS");
    }

    #[test]
    fn with_long_replay_on_the_stream_goes_to_the_nas_and_a_clip_is_cut_there() {
        let (r, memory, events) = recorder(RING_CAP, Plan { replay: true, ..OFF });
        feed(&r, 0, 1);
        wait(|| r.stats().live);
        r.save_clip("Hades II", 15).unwrap();
        assert_eq!(told(&events), Event::Saved { name: "Hades/cut.mp4".into(), seconds: 300, bytes: 2 << 30 });
        let asked = lock(&memory.asked).clone();
        assert!(asked[0].starts_with("/open?session=") && asked[0].ends_with("&replay=1&reach=300&mbps=65"), "{asked:?}");
        assert!(asked[1].starts_with("/save?session=") && asked[1].ends_with("&seconds=300&game=Hades%20II"), "{asked:?}");
        r.set_replay(false);
        assert_eq!(next(&events), Event::State { replay: false, recording: false, ready: true });
        wait(|| !lock(&memory.files).is_empty());
        let files = lock(&memory.files);
        assert!(files[0].0.starts_with("/live?session=") && video_samples(&files[0].1) > 0, "the upload ends whole when nothing wants it");
    }

    #[test]
    fn a_stalled_nas_holds_nothing_up_is_said_once_and_save_clip_goes_back_to_the_ring() {
        let cap = 6 << 20;
        let (r, memory, events) = recorder(cap, Plan { replay: true, ..OFF });
        let big = big_key();
        r.offer(Track::Video, 0, true, &big);
        r.sync();
        wait(|| r.stats().live);
        memory.stall.store(true, Ordering::Relaxed);
        memory.deaf.store(true, Ordering::Relaxed);
        for s in 1..30i64 {
            r.offer(Track::Video, s * 1_000_000_000, true, &big);
            for f in 1..5 {
                r.offer(Track::Video, s * 1_000_000_000 + f * 33_000_000, false, &big);
            }
            r.sync();
        }
        let st = r.stats();
        assert!(st.dropped == 0 && st.evicted_gops > 0 && st.ring_bytes <= cap, "the tee kept filing while the upload stood still: {st:?}");
        r.save_clip("Hades", 2).unwrap();
        memory.stall.store(false, Ordering::Relaxed);
        let mut told = [told(&events), told(&events)];
        told.sort_by_key(|e| matches!(e, Event::Notice { .. }));
        assert!(matches!(&told[0], Event::Saved { name, .. } if name == "Hades/clip.mp4"), "{told:?}");
        assert_eq!(told[1], Event::Notice { text: "Long replay is paused: the NAS is not keeping up with the stream. Save clip keeps the TV's own last seconds.".into() });
        assert_eq!((r.stats().live_failures, r.stats().live), (1, false));
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn a_recording_starts_at_a_keyframe_asked_for_and_is_stopped_on_the_nas_and_the_feed_runs_only_for_it() {
        let g = rig(RING_CAP, OFF);
        let (r, memory, events) = (&g.r, &g.memory, &g.events);
        g.second(0, true);
        r.start_recording("Hades");
        assert_eq!(g.asked(), 1, "the press asks for a keyframe at once");
        wait(|| r.stats().live);
        assert!(!lock(&memory.asked).iter().any(|a| a.starts_with("/record/start")), "the keyframe already held is not the start");
        g.second(1, true);
        wait(|| r.state().1);
        assert_eq!(r.state(), (false, true, true));
        r.stop_recording();
        assert_eq!(told(events), Event::Saved { name: "Hades/rec.mp4".into(), seconds: 7, bytes: 1 << 20 });
        wait(|| !lock(&memory.files).is_empty());
        let asked = lock(&memory.asked).clone();
        assert!(asked[0].ends_with("&replay=0&reach=300&mbps=65"), "{asked:?}");
        assert!(asked[1].starts_with("/record/start?session=") && asked[1].ends_with("&game=Hades&max=7200"), "{asked:?}");
        assert!(asked[2].starts_with("/record/stop?session=") && asked.len() == 3, "{asked:?}");
    }

    #[test]
    fn a_recording_that_gets_no_keyframe_is_asked_for_three_times_and_then_said_calmly() {
        let g = rig(RING_CAP, OFF);
        g.second(0, true);
        g.r.start_recording("Hades");
        for _ in 0..4 {
            g.advance(2100);
        }
        assert_eq!(told(&g.events), Event::Failed { reason: "Record could not start: the game sent no keyframe".into() });
        assert_eq!((g.asked(), g.r.state()), (3, (false, false, true)));
        assert!(!lock(&g.memory.asked).iter().any(|a| a.starts_with("/record/start")));
    }

    #[test]
    fn a_recording_the_nas_cannot_take_is_said_and_not_retried() {
        let (r, memory, events) = recorder(RING_CAP, OFF);
        feed(&r, 0, 1);
        memory.deaf.store(true, Ordering::Relaxed);
        r.start_recording("Hades");
        assert_eq!(told(&events), Event::Notice { text: "Recording could not start: the NAS is not answering".into() });
        assert_eq!((r.state(), r.stats().live_failures), ((false, false, true), 1));
        assert!(!lock(&r.shared.live).wanted());
    }

    #[test]
    fn a_reach_the_nas_shortens_is_said_once() {
        let (r, memory, events) = recorder(RING_CAP, Plan { replay: true, reach_s: 900, ..OFF });
        let note = "The NAS buffer is 6 GB: at 150 Mbit/s it holds 5 min 40 s, not 15 min";
        *lock(&memory.grant) = (340, note);
        feed(&r, 0, 1);
        assert_eq!(told(&events), Event::Notice { text: note.into() });
        wait(|| r.stats().live);
        r.set_replay(false);
        wait(|| lock(&memory.files).len() == 1);
        r.set_replay(true);
        wait(|| lock(&memory.asked).iter().filter(|a| a.starts_with("/open")).count() == 2 && r.stats().live);
        r.save_clip("Hades", 15).unwrap();
        wait(|| lock(&memory.asked).iter().any(|a| a.starts_with("/save?") && a.contains("&seconds=340&")));
        assert!(events.try_iter().all(|e| !matches!(e, Event::Notice { .. })));
    }

    #[test]
    fn a_saved_clip_is_one_sentence_and_a_change_of_state_is_silent() {
        assert_eq!(Event::State { replay: true, recording: true, ready: false }.sentence(), None);
        assert_eq!(Event::Saved { name: "Hades/a.mp4".into(), seconds: 15, bytes: 3 << 20 }.sentence(), Some((true, "Saved Hades/a.mp4, 15 s, 3 MB".into())));
        assert_eq!(Event::Saved { name: "a.mp4".into(), seconds: 300, bytes: 5 << 29 }.sentence(), Some((true, "Saved a.mp4, 5 min, 2.5 GB".into())));
        assert_eq!(Event::Failed { reason: "no".into() }.sentence(), Some((false, "no".into())));
        assert_eq!(Event::Notice { text: "paused".into() }.sentence(), Some((false, "paused".into())));
    }

    #[test]
    fn nothing_is_saved_before_a_picture_and_an_unreachable_nas_is_reported() {
        let (r, memory, events) = recorder(RING_CAP, OFF);
        r.save_clip("Hades", 5).unwrap();
        assert!(matches!(told(&events), Event::Failed { reason } if reason.contains("no picture")));
        feed(&r, 0, 2);
        memory.refuse.store(true, Ordering::Relaxed);
        r.save_clip("Hades", 1).unwrap();
        assert!(matches!(told(&events), Event::Failed { reason } if reason.contains("cannot reach the NAS")));
        assert_eq!(r.stats().failures, 2);
    }

    #[test]
    fn a_save_with_no_keyframe_in_the_ring_asks_for_one_and_starts_there() {
        let g = rig(RING_CAP, OFF);
        g.r.offer(Track::Video, 0, false, &h264_delta(1));
        g.r.sync();
        assert!(!g.r.state().2, "a feed with no keyframe yet is not ready");
        g.r.save_clip("Hades", 5).unwrap();
        wait(|| g.asked() == 1);
        g.second(1, true);
        assert!(matches!(told(&g.events), Event::Saved { name, .. } if name == "Hades/clip.mp4"));
        let files = lock(&g.memory.files);
        assert!((1..=30).contains(&video_samples(&files[0].1)) && files[0].1.windows(4).any(|w| w == b"avcC"));
    }

    #[test]
    fn a_save_the_game_never_answers_fails_in_one_calm_sentence() {
        let g = rig(RING_CAP, OFF);
        g.r.offer(Track::Video, 0, false, &h264_delta(1));
        g.r.sync();
        g.r.save_clip("Hades", 5).unwrap();
        wait(|| g.asked() == 1);
        for _ in 0..4 {
            g.advance(2100);
        }
        assert_eq!(told(&g.events), Event::Failed { reason: "Save clip could not start: the game sent no keyframe to start from".into() });
        assert_eq!((g.asked(), g.r.stats().failures), (3, 1));
    }

    #[test]
    fn a_single_keyframe_gets_a_cut_point_every_interval_and_a_stream_with_its_own_costs_nothing() {
        let g = rig(RING_CAP, OFF);
        g.second(0, true);
        let mut answered = 0;
        for s in 1..=12 {
            g.advance(1000);
            let key = g.asked() > answered;
            answered = g.asked();
            g.second(s, key);
        }
        assert_eq!((g.asked(), g.r.stats().ring_gops), (2, 3), "asks at 5 s and 10 s; each answer is a new GOP");

        let own = rig(RING_CAP, OFF);
        for s in 0..=12 {
            own.advance(1000);
            own.second(s, s % 3 == 0);
        }
        assert_eq!(own.asked(), 0);
    }

    #[test]
    fn a_keyframe_is_not_asked_for_while_the_stream_is_in_trouble() {
        let g = rig(RING_CAP, OFF);
        g.second(0, true);
        g.advance(3000);
        g.r.trouble();
        g.advance(2500);
        assert_eq!(g.asked(), 0, "due at 5 s, held until 3 s after the trouble");
        g.advance(1000);
        assert_eq!(g.asked(), 1);
    }

    #[test]
    fn only_when_i_save_nothing_is_asked_until_the_press_and_the_clip_starts_there() {
        let g = rig(RING_CAP, Plan { keyframe_s: 0, ..OFF });
        g.second(0, true);
        for _ in 0..4 {
            g.advance(10_000);
        }
        assert_eq!(g.asked(), 0);
        assert!(g.r.state().2);
        g.r.save_clip("Hades", 2).unwrap();
        wait(|| g.asked() == 1);
        for s in 100..103 {
            g.second(s, s == 100);
        }
        let Event::Saved { seconds, .. } = told(&g.events) else { panic!() };
        assert_eq!(seconds, 2);
        assert_eq!(video_samples(&lock(&g.memory.files)[0].1), 61, "the keyframe asked for, then two seconds on");
    }

    #[test]
    fn the_drawer_is_told_when_a_clip_can_start_and_when_it_cannot() {
        let g = rig(6 << 20, OFF);
        let (mut big_delta, big) = (h264_delta(1), big_key());
        big_delta.extend(std::iter::repeat_n(1u8, 400_000));
        g.r.offer(Track::Video, 0, true, &big);
        g.r.sync();
        assert_eq!(next(&g.events), Event::State { replay: false, recording: false, ready: true });
        for f in 1..17 {
            g.r.offer(Track::Video, f * 33_000_000, false, &big_delta);
            g.r.sync();
        }
        assert_eq!(next(&g.events), Event::State { replay: false, recording: false, ready: false });
        assert_eq!(g.asked(), 1, "the only keyframe fell out of the ring: one is asked for");
        g.r.offer(Track::Video, 1_000_000_000, true, &big);
        g.r.sync();
        assert_eq!(next(&g.events), Event::State { replay: false, recording: false, ready: true });
    }

    #[test]
    fn a_stalled_worker_makes_offers_drop_and_count_without_blocking_the_caller() {
        let (r, _memory, _events) = recorder(RING_CAP, OFF);
        let unit = vec![7u8; 1 << 20];
        let held = lock(&r.shared.ring); // the worker blocks on the ring, so the hand-off fills
        for i in 0..2000 {
            r.offer(Track::Video, i, i == 0, &unit);
        }
        let queued = r.shared.queued.load(Ordering::Relaxed);
        assert!(queued <= HANDOFF_CAP, "{queued} bytes queued");
        let s_dropped = r.shared.dropped.load(Ordering::Relaxed);
        // Four units fit the 4 MB hand-off and the worker holds one more; everything else was refused at once.
        assert!(s_dropped >= 1995, "{s_dropped} dropped of 2000");
        drop(held);
        r.sync();
        let stats = r.stats();
        assert_eq!(stats.offered, 2000);
        assert!(stats.ring_bytes <= 5 << 20);
    }

    #[test]
    fn the_ring_obeys_its_cap_while_a_long_stream_runs() {
        let cap = 6 << 20;
        let (r, _memory, _events) = recorder(cap, OFF);
        let big = {
            let mut v = h264_key();
            v.extend(std::iter::repeat_n(1u8, 400_000));
            v
        };
        for s in 0..30i64 {
            r.offer(Track::Video, s * 1_000_000_000, true, &big);
            for f in 1..5 {
                r.offer(Track::Video, s * 1_000_000_000 + f * 33_000_000, false, &big);
            }
            r.sync();
            assert!(r.stats().ring_bytes <= cap);
        }
        let st = r.stats();
        assert!(st.evicted_gops > 0 && st.ring_gops >= 1 && st.dropped == 0, "{st:?}");
    }

    /// Splits an encoder's elementary stream into access units at each delimiter (H.264/H.265 AUD NAL, AV1 temporal delimiter).
    fn split_units(codec: Codec, es: &[u8]) -> Vec<(bool, Vec<u8>)> {
        let mut cuts = Vec::new();
        match codec {
            Codec::Av1 => {
                let mut at = 0;
                for (ty, obu, _) in bitstream::obus(es) {
                    if ty == 2 {
                        cuts.push(at);
                    }
                    at += obu.len();
                }
            }
            _ => {
                for (s, e) in bitstream::nal_units(es) {
                    let delimiter = if codec == Codec::H264 { es[s] & 0x1f == 9 } else { (es[s] >> 1) & 0x3f == 35 };
                    if delimiter && e > s {
                        cuts.push(if s >= 4 && es[s - 4] == 0 { s - 4 } else { s - 3 });
                    }
                }
            }
        }
        cuts.push(es.len());
        cuts.windows(2)
            .map(|w| {
                let au = es[w[0]..w[1]].to_vec();
                let key = match codec {
                    Codec::H264 => bitstream::nal_units(&au).iter().any(|&(s, _)| au[s] & 0x1f == 5),
                    Codec::H265 => bitstream::nal_units(&au).iter().any(|&(s, _)| matches!((au[s] >> 1) & 0x3f, 16..=21)),
                    Codec::Av1 => bitstream::obus(&au).iter().any(|(ty, _, _)| *ty == 1),
                };
                (key, au)
            })
            .collect()
    }

    /// With ffmpeg and ffprobe on the machine: real encoder output of each codec, one second at 30 fps, goes through the recorder, and
    /// the file must probe as the right codec and size, count 30 pictures, carry an Opus track, and decode without a single error.
    /// Machines without them skip it; the structural tests above always run.
    #[test]
    fn ffmpeg_decodes_clips_of_each_codec() {
        use std::process::Command;
        let have = |tool: &str| Command::new(tool).arg("-version").output().is_ok_and(|o| o.status.success());
        if !have("ffmpeg") || !have("ffprobe") {
            eprintln!("ffmpeg or ffprobe is not installed: skipping the player check");
            return;
        }
        for (codec, name, encoder, format) in [
            (Codec::H264, "h264", &["-c:v", "libx264", "-x264-params", "aud=1:keyint=15:bframes=0"][..], "h264"),
            (Codec::H265, "hevc", &["-c:v", "libx265", "-x265-params", "aud=1:keyint=15:bframes=0:log-level=none"][..], "hevc"),
            (Codec::Av1, "av1", &["-c:v", "libsvtav1", "-g", "15"][..], "obu"),
        ] {
            let made = Command::new("ffmpeg")
                .args(["-v", "error", "-f", "lavfi", "-i", "testsrc=size=320x180:rate=30", "-t", "1", "-pix_fmt", "yuv420p"])
                .args(encoder)
                .args(["-f", format, "-"])
                .output()
                .unwrap();
            if !made.status.success() {
                eprintln!("no {name} encoder here: skipping it");
                continue;
            }
            let units = split_units(codec, &made.stdout);
            assert_eq!(units.len(), 30, "{name}: the encoder's access units");
            let (r, memory, events) = recorder(RING_CAP, OFF);
            r.set_stream(StreamInfo {
                codec: Some(codec),
                width: 320,
                height: 180,
                fps: 30,
                hdr: None,
                audio: Some(AudioConfig { channels: 2, sample_rate: 48_000, mapping: None }),
            });
            for (i, (key, au)) in units.iter().enumerate() {
                let pts = i as i64 * 33_333_333;
                r.offer(Track::Video, pts, *key, au);
                r.offer(Track::Audio, pts, false, &opus_unit());
                if i % 10 == 9 {
                    r.sync();
                }
            }
            r.sync();
            r.save_clip("Test", 1).unwrap();
            let Event::Saved { bytes, .. } = told(&events) else { panic!("{name}: not saved") };
            let file = lock(&memory.files)[0].1.clone();
            assert_eq!(file.len() as u64, bytes);
            let path = std::env::temp_dir().join(format!("lumagame-recorder-{name}-{}.mp4", std::process::id()));
            std::fs::write(&path, &file).unwrap();
            let probe = Command::new("ffprobe")
                .args(["-v", "error", "-count_frames", "-show_entries", "stream=codec_name,width,height,nb_read_frames", "-of", "csv=p=0"])
                .arg(&path)
                .output()
                .unwrap();
            let decode = Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(&path).args(["-map", "0:v", "-f", "null", "-"]).output().unwrap();
            let _ = std::fs::remove_file(&path);
            let probed = String::from_utf8_lossy(&probe.stdout).into_owned();
            assert!(probe.status.success() && probe.stderr.is_empty(), "{name}: ffprobe: {}", String::from_utf8_lossy(&probe.stderr));
            assert!(probed.contains(&format!("{name},320,180,30")), "{name}: video stream: {probed}");
            assert!(probed.contains("opus"), "{name}: audio stream: {probed}");
            assert!(decode.status.success() && decode.stderr.is_empty(), "{name}: ffmpeg: {}", String::from_utf8_lossy(&decode.stderr));
        }
    }
}
