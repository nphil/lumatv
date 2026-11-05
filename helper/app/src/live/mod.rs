//! The live path: a camera's H.264 or H.265 stream on the TV's video plane, from Scrypted's RTSP rebroadcast (live) or the bridge's own RTSP
//! server (a recording or a clip), through the GFN feeder (`docs/research/scrypted-cameras-design.md` 2.2 and 2.5).
//!
//! ```text
//! Scrypted --RTSP/TCP--> reader (rtsp-client: Annex B access units) --queue--> writer --unix socket--> lumatv-gfn-feeder --> NDL --> video plane
//! ```
//!
//! The TV never holds a camera's own address or password: the stream address is what the bridge answered, a capability that is never logged
//! or published. One session at a time (one decoder; the TV check says whether a second is ever given). A [`Session`] is the feeder's lease
//! and the writer thread; a reader thread is attached to it for each stream it plays, so a seek, a pause and the return to live reuse the
//! pipeline (and the picture that is on the plane) when the stream is the same camera's and the same size, and a new camera or size loads
//! the pipeline again.
//!
//! **Fast start.** `start` returns at once. The writer leases the feeder (a process kept for `feederIdleSeconds` after the last session, so
//! a second open skips its start) and loads the pipeline with what the bridge said the stream is, while the reader dials: the load (0.2 to
//! 0.4 s on the set) and the handshake and the replay burst run side by side. The writer holds the first pictures 150 ms and starts at the
//! last random access point that came in that time, so the catch-up from Scrypted's prebuffer is at most one key interval of pictures.
//!
//! **Keeping live.** The queue between the two threads holds at most 15 pictures once the stream is caught up; past that it drops to the next
//! key picture instead of growing. A stream whose pictures run more than 500 ms behind the clock they arrive by is dialled again, once; one
//! that is quiet for 8 s ends with a reason in words. **Fast stop.** `stop` flags the session and returns: the unload, the RTSP teardown and
//! the feeder's end happen on the session's own threads, off the caller's path. **A feeder that breaks** under a session (a write refused, its process gone,
//! its own error) is replaced once, with the pipeline and the picture in hand, and nothing is shown; one that breaks again fails the session.
//!
//! The topic `cameras.live` ([`View`]) says where it stands: `{camera, mode (live|playback|clip), state (idle|asking|starting|playing|failed|
//! stopped), reason, codec, width, height, fps, startedAt, positionMs, rate, fed, dropped, lost, delayMs, window, since, paused, purpose}`. `purpose`
//! is what the stream was asked for (`pop-up 41`, `full view`), kept by a stopped or failed view too: a screen that asked for its own stream tells it from the
//! previous one's last words.

pub mod gf;

use parking_lot::{Condvar, Mutex};
use rtsp_client::{Codec, Frame, Options, Session as Rtsp, Url};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How long the feeder has to make its socket and say hello.
const FEEDER_WITHIN: Duration = Duration::from_secs(8);
/// How long a session waits for the feeder when another session still holds it (the old one is told to go and does so in a moment).
const FEEDER_WAIT: Duration = Duration::from_secs(5);
/// The most pictures the queue holds once the stream is caught up, and the most bytes while it is catching up (the replay burst is old
/// pictures: the reader stops reading, and so the network stops sending, while the queue is full).
const QUEUE_UNITS: usize = 15;
const QUEUE_BYTES: usize = 8 << 20;
/// A key picture that arrives with this many pictures waiting drops them: it is a better place to start from.
const FLUSH_BACKLOG: usize = 2;
/// The writer holds the first pictures this long, to start at the last random access point of the burst.
const HOLD: Duration = Duration::from_millis(150);
/// A stream with nothing for this long is over.
const QUIET: Duration = Duration::from_secs(8);
/// Pictures that arrive this far behind the clock the stream was caught up by are a stream to dial again.
const LAG_MAX_MS: i64 = 500;
const LAG_FOR: Duration = Duration::from_secs(1);
/// How often the writer says where the picture is.
const PUBLISH_EVERY: Duration = Duration::from_millis(250);

/// How the feeder is started; `None` leaves the live path off (the pictures' path stays).
#[derive(Clone, Debug)]
pub struct FeederSpec {
    pub program: PathBuf,
    /// The window type the feeder owns (`card`, `overlay`, `screensaver`): the TV check decides.
    pub window: String,
    /// The NDL library to load (`-L`): empty for the feeder's default.
    pub library: String,
    pub extra_args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Where the socket and the stats file go.
    pub run_dir: PathBuf,
    /// How long a feeder that no session holds is kept (zero: it ends with the session).
    pub idle: Duration,
}

/// The default feeder on the set: the GFN feeder staged beside the helper.
pub fn default_spec(data_dir: &Path, window: &str, library: &str, idle: Duration) -> FeederSpec {
    // LG's AV connector opens a Wayland connection of its own to register the window id and reads these two to find the compositor; the helper's
    // service has neither (without them the pipeline says "fail to create AVProxy": `vplane.sh` sets the same two)
    let env = [("XDG_RUNTIME_DIR", "/tmp/xdg"), ("WAYLAND_DISPLAY", "wayland-0")]
        .into_iter()
        .filter(|(k, _)| std::env::var_os(k).is_none_or(|v| v.is_empty()))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    FeederSpec {
        program: data_dir.join("bin/lumatv-gfn-feeder"),
        window: window.to_owned(),
        library: library.to_owned(),
        extra_args: Vec::new(),
        env,
        run_dir: std::env::temp_dir().join("lumatv-cameras"),
        idle,
    }
}

/// How a stream address is dialled (a test dials its fake camera on the loopback).
pub type Dial = Arc<dyn Fn(&str) -> Result<Rtsp, rtsp_client::Error> + Send + Sync>;

/// What went wrong with a stream, in words that name no address: the client's own messages carry the host and port of the stream, and a
/// stream's address is a capability that is never shown, logged or published.
pub fn words_of(e: &rtsp_client::Error) -> String {
    use rtsp_client::Kind;
    match e.kind {
        Kind::Url | Kind::Resolve | Kind::NotLocal => {
            "Scrypted gave a stream address this TV cannot use"
        }
        Kind::Connect => "Scrypted's stream for this camera could not be reached",
        Kind::Io => "The connection to the stream failed",
        Kind::Protocol | Kind::Sdp | Kind::Sps => "Scrypted's stream was not understood",
        Kind::Status => "Scrypted refused the stream",
        Kind::Auth => "Scrypted wants a sign-in for the stream",
        Kind::Unsupported => {
            "This camera's picture is not one the TV can decode (8-bit 4:2:0 only)"
        }
        Kind::Timeout => "Scrypted sent no picture in time",
        Kind::Closed => "Scrypted ended the stream",
    }
    .to_owned()
}

/// The real dial: RTSP over TCP to a home-network address.
pub fn real_dial() -> Dial {
    Arc::new(|address| {
        let url = Url::parse(address)?;
        Rtsp::connect(
            &url,
            &Options {
                connect_timeout: Duration::from_secs(3),
                reply_timeout: Duration::from_secs(4),
                // a camera that sends its parameter sets only with the next key picture (not prebuffered): its interval is up to 4 s
                first_picture_timeout: Duration::from_secs(8),
                user_agent: "LumaTV".to_owned(),
                ..Options::default()
            },
        )
    })
}

pub struct Deps {
    pub feeder: Option<FeederSpec>,
    pub dial: Dial,
}

/// What is being played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Live,
    Playback,
    Clip,
}

impl Mode {
    pub fn word(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Playback => "playback",
            Self::Clip => "clip",
        }
    }
}

/// A stream to play.
pub struct Plan {
    pub camera: String,
    pub mode: Mode,
    /// The stream's address: a capability, never shown.
    pub url: String,
    /// What the bridge says the stream is (empty or 0 where it did not say); the pipeline is loaded with it before the stream is dialled.
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// A recording: the moment (epoch milliseconds) the stream's first picture belongs to, and the pace it runs at.
    pub started_at: i64,
    pub rate: u8,
    /// The app id the decoder is opened for (`GF_APP`): empty is the video window's own. LG's video path connects a video to its sink only for an
    /// app on SAM's foreground list, which the window's own id is not (docs/research/camera-popup-result.md).
    pub ndl_app: String,
    /// The picture is decoded in the feeder and drawn by the compositor as a texture (a claim the windows could not carry, `slot texture`): no pipeline,
    /// no app id, no video plane (`gfn/feeder/src/gf_swdec.c`).
    pub texture: bool,
    /// The compositor granted the sub window beside an app's video (`slot sub`): the load runs in the starfish pipeline (`GF_PIPE smp`, the one an app's own
    /// video has, no game mode) and asks for the sub sink (`GF_SINK sub_video`); `ndl_app` is then the app in front, which SAM lists already.
    pub sub: bool,
    /// What the stream is for, in words for the log (`popup 41`); empty for the full view.
    pub purpose: String,
    /// The window type of the feeder this stream plays in (`video`: the compositor's own video window, played on once a claim is granted to it); empty:
    /// the configured one.
    pub window: String,
    /// Called once, off the caller's path, when this stream is over for any reason (the bridge is told to end its playback session).
    pub on_end: Option<Box<dyn FnOnce() + Send>>,
}

impl std::fmt::Debug for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Plan({} {} {}x{}, address hidden)",
            self.camera,
            self.mode.word(),
            self.width,
            self.height
        )
    }
}

/// What the shell sees.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub camera: String,
    pub mode: &'static str,
    pub state: &'static str,
    pub reason: String,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub started_at: Option<i64>,
    pub position_ms: Option<i64>,
    pub rate: u8,
    pub fed: u64,
    /// Pictures a second the feeder took over the last second or so (the measured rate, not the stream's declared one).
    pub fed_fps: f32,
    pub dropped: u64,
    pub lost: u64,
    pub delay_ms: u64,
    pub window: String,
    pub since: u64,
    pub paused: bool,
    /// What the stream was asked for, as the plan says it (`Plan::purpose`); empty for a start that is nobody's pop-up.
    pub purpose: String,
}

impl Default for View {
    fn default() -> Self {
        Self {
            camera: String::new(),
            mode: "live",
            state: "idle",
            reason: String::new(),
            codec: String::new(),
            width: 0,
            height: 0,
            fps: 0,
            started_at: None,
            position_ms: None,
            rate: 1,
            fed: 0,
            fed_fps: 0.0,
            dropped: 0,
            lost: 0,
            delay_ms: 0,
            window: String::new(),
            since: 0,
            paused: false,
            purpose: String::new(),
        }
    }
}

impl View {
    pub fn json(&self) -> Value {
        let text = |s: &str| {
            if s.is_empty() {
                Value::Null
            } else {
                Value::String(s.to_owned())
            }
        };
        json!({
            "camera": text(&self.camera),
            "mode": self.mode,
            "state": self.state,
            "reason": self.reason,
            "codec": text(&self.codec),
            "width": self.width,
            "height": self.height,
            "fps": self.fps,
            "startedAt": self.started_at,
            "positionMs": self.position_ms,
            "rate": self.rate,
            "fed": self.fed,
            "fedFps": (f64::from(self.fed_fps) * 10.0).round() / 10.0,
            "dropped": self.dropped,
            "lost": self.lost,
            "delayMs": self.delay_ms,
            "window": text(&self.window),
            "since": self.since,
            "paused": self.paused,
            "purpose": text(&self.purpose),
        })
    }
}

fn codec_word(codec: Codec) -> &'static str {
    match codec {
        Codec::H264 => "h264",
        Codec::H265 => "h265",
    }
}

fn codec_of(word: &str) -> Option<Codec> {
    match word.to_ascii_lowercase().as_str() {
        "h264" | "avc" => Some(Codec::H264),
        "h265" | "hevc" => Some(Codec::H265),
        _ => None,
    }
}

// ---- waking a reader -----------------------------------------------------------------------------------------------------------------

/// A pipe a thread that waits in `poll` on its stream is woken through when it is told to go.
struct Waker {
    read: OwnedFd,
    write: OwnedFd,
}

impl Waker {
    fn new() -> io::Result<Self> {
        let mut fds = [0i32; 2];
        // SAFETY: `pipe2` fills two descriptors, which are then owned here.
        let made = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
        if made != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the two descriptors were just made and belong to nobody else.
        Ok(unsafe {
            Self {
                read: OwnedFd::from_raw_fd(fds[0]),
                write: OwnedFd::from_raw_fd(fds[1]),
            }
        })
    }

    fn wake(&self) {
        let byte = 1u8;
        // SAFETY: one byte from a valid buffer; a full pipe (EAGAIN) already wakes the reader.
        unsafe {
            libc::write(self.write.as_raw_fd(), (&raw const byte).cast(), 1);
        }
    }

    /// Waits up to `wait` for `fd` to have something to read or for a wake; true when `fd` has.
    fn wait(&self, fd: i32, wait: Duration) -> bool {
        let mut polls = [
            libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: self.read.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: two valid pollfd structs.
        unsafe {
            libc::poll(
                polls.as_mut_ptr(),
                2,
                wait.as_millis().min(i32::MAX as u128) as i32,
            );
        }
        if polls[1].revents != 0 {
            let mut sink = [0u8; 16];
            // SAFETY: a non-blocking read into a buffer of that size.
            unsafe {
                libc::read(self.read.as_raw_fd(), sink.as_mut_ptr().cast(), sink.len());
            }
        }
        polls[0].revents != 0
    }
}

// ---- the queue between the reader and the writer -------------------------------------------------------------------------------------

struct Unit {
    data: Vec<u8>,
    pts_us: i64,
    key: bool,
    /// What the picture's own sequence parameter set says of its size, when it carries one.
    size: Option<(u32, u32)>,
    arrived: Instant,
}

/// What a stream is, as its description says once the reader is connected: the first thing of an epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StreamInfo {
    codec: Codec,
    width: u32,
    height: u32,
    /// A hint for the pipeline (0: not known): a stream that says another rate than the bridge did is not another stream.
    fps: u32,
}

impl StreamInfo {
    /// Whether the pipeline loaded for `self` plays `other` as it is: the codec and the picture's size, not the rate.
    fn same_pipeline(&self, other: &StreamInfo) -> bool {
        self.codec == other.codec && self.width == other.width && self.height == other.height
    }
}

/// What streams turned out to be where the bridge declared another size, by camera and declared size. A portrait doorbell's low stream is declared 640x480
/// and is 480x640: without this the pipeline is loaded for the declared size and again for the real one a moment later, on every start.
#[derive(Default)]
struct Sizes(HashMap<(String, u32, u32), (u32, u32)>);

impl Sizes {
    /// Declared sizes are few (a camera has two or three streams); a camera list that keeps changing does not grow this without end.
    const MAX: usize = 64;

    fn real(&self, camera: &str, declared: (u32, u32)) -> (u32, u32) {
        self.0.get(&(camera.to_owned(), declared.0, declared.1)).copied().unwrap_or(declared)
    }

    fn learn(&mut self, camera: &str, declared: (u32, u32), real: (u32, u32)) {
        let key = (camera.to_owned(), declared.0, declared.1);
        if declared == real {
            // declared right now: an old finding does not hold
            self.0.remove(&key);
            return;
        }
        if self.0.len() >= Self::MAX && !self.0.contains_key(&key) {
            self.0.clear();
        }
        self.0.insert(key, real);
    }
}

enum Item {
    Stream(StreamInfo),
    Unit(Unit),
}

#[derive(Default)]
struct QState {
    items: VecDeque<Item>,
    units: usize,
    bytes: usize,
    /// The stream whose pictures are taken: a reader that was replaced gives nothing more.
    epoch: u64,
    /// While set the writer takes no picture (the burst gathers); cleared when the time comes or the stream is caught up.
    hold_until: Option<Instant>,
    /// The stream has caught up with its live edge: the queue's size is bounded in pictures from now on.
    steady: bool,
    /// Pictures are being dropped up to the next key picture.
    waiting_key: bool,
    dropped: u64,
    closed: bool,
}

struct Queue {
    state: Mutex<QState>,
    changed: Condvar,
}

enum Give {
    /// In the queue (or dropped by its rules, which is also done with).
    Done,
    /// The reader is not the stream's any more.
    Stale,
    /// The burst's bytes are all queued: wait for room and give it again.
    Full(Unit),
}

enum Taken {
    Item(Item),
    Nothing,
    Closed,
}

impl Queue {
    fn new() -> Self {
        Self {
            state: Mutex::new(QState::default()),
            changed: Condvar::new(),
        }
    }

    /// A new stream starts: what was queued of the old one is nothing, and the first pictures are held while the burst gathers.
    fn begin(&self, epoch: u64) {
        let mut s = self.state.lock();
        s.items.clear();
        s.units = 0;
        s.bytes = 0;
        s.epoch = epoch;
        s.hold_until = Some(Instant::now() + HOLD);
        s.steady = false;
        s.waiting_key = false;
        self.changed.notify_all();
    }

    fn stream(&self, epoch: u64, info: StreamInfo) -> bool {
        let mut s = self.state.lock();
        if s.epoch != epoch || s.closed {
            return false;
        }
        s.items.push_front(Item::Stream(info));
        self.changed.notify_all();
        true
    }

    fn give(&self, epoch: u64, unit: Unit) -> Give {
        let mut s = self.state.lock();
        if s.closed || s.epoch != epoch {
            return Give::Stale;
        }
        if s.waiting_key {
            if !unit.key {
                s.dropped += 1;
                return Give::Done;
            }
            s.waiting_key = false;
        }
        // a key picture with a backlog behind it is a better place to start than the backlog
        if unit.key && s.units >= FLUSH_BACKLOG {
            Self::drop_units(&mut s, None);
        }
        if s.steady {
            if s.units >= QUEUE_UNITS {
                // behind: drop to the newest key picture waiting, else to the next one that comes
                let last_key = s
                    .items
                    .iter()
                    .rposition(|i| matches!(i, Item::Unit(u) if u.key));
                match last_key {
                    Some(at) => Self::drop_units(&mut s, Some(at)),
                    None => Self::drop_units(&mut s, None),
                }
                if s.units >= QUEUE_UNITS || (!unit.key && last_key.is_none()) {
                    Self::drop_units(&mut s, None);
                    if !unit.key {
                        s.waiting_key = true;
                        s.dropped += 1;
                        return Give::Done;
                    }
                }
            }
        } else if s.bytes + unit.data.len() > QUEUE_BYTES && s.units > 0 {
            return Give::Full(unit);
        }
        s.units += 1;
        s.bytes += unit.data.len();
        s.items.push_back(Item::Unit(unit));
        self.changed.notify_all();
        Give::Done
    }

    /// Drops the pictures before item `before` (all of them with `None`); the stream marker stays.
    fn drop_units(s: &mut QState, before: Option<usize>) {
        let end = before.unwrap_or(s.items.len());
        let mut kept = VecDeque::with_capacity(s.items.len());
        for (at, item) in s.items.drain(..).enumerate() {
            match item {
                Item::Unit(u) if at < end => {
                    s.dropped += 1;
                    s.bytes -= u.data.len();
                    s.units -= 1;
                }
                other => kept.push_back(other),
            }
        }
        s.items = kept;
    }

    /// The reader found the stream caught up with its live edge.
    fn settle(&self) {
        let mut s = self.state.lock();
        s.steady = true;
        s.hold_until = None;
        self.changed.notify_all();
    }

    fn take(&self, wait: Duration) -> Taken {
        let deadline = Instant::now() + wait;
        let mut s = self.state.lock();
        loop {
            let front_is_unit = matches!(s.items.front(), Some(Item::Unit(_)));
            if let Some(hold) = s.hold_until
                && front_is_unit
                && !s.steady
            {
                if Instant::now() < hold {
                    self.changed.wait_until(&mut s, hold.min(deadline));
                    if Instant::now() >= deadline {
                        return Taken::Nothing;
                    }
                    continue;
                }
                s.hold_until = None;
            }
            if let Some(item) = s.items.pop_front() {
                if let Item::Unit(u) = &item {
                    s.units -= 1;
                    s.bytes -= u.data.len();
                }
                self.changed.notify_all();
                return Taken::Item(item);
            }
            if s.closed {
                return Taken::Closed;
            }
            if self.changed.wait_until(&mut s, deadline).timed_out() {
                return Taken::Nothing;
            }
        }
    }

    /// Waits up to `wait` for the burst's queue to have room.
    fn wait_room(&self, wait: Duration) {
        let deadline = Instant::now() + wait;
        let mut s = self.state.lock();
        while !s.closed && !s.steady && s.bytes >= QUEUE_BYTES / 2 {
            if self.changed.wait_until(&mut s, deadline).timed_out() {
                return;
            }
        }
    }

    fn dropped(&self) -> u64 {
        self.state.lock().dropped
    }

    fn close(&self) {
        let mut s = self.state.lock();
        s.closed = true;
        self.changed.notify_all();
    }
}

// ---- keeping live --------------------------------------------------------------------------------------------------------------------

/// Notices a stream that has been left behind: pictures arriving later and later by the clock they were made on. The replay burst (pictures
/// arriving faster than they were made) is not lag; the stream is *caught up* once its pictures arrive at about the pace they were made, and
/// lag is measured from there.
struct LagWatch {
    window: Option<(Instant, i64)>,
    settled: bool,
    base: Option<(Instant, i64)>,
    behind_since: Option<Instant>,
}

impl LagWatch {
    fn new() -> Self {
        Self {
            window: None,
            settled: false,
            base: None,
            behind_since: None,
        }
    }

    /// A picture made at `pts_us` arrived at `now`. Returns what changed: the stream is caught up, or it is too far behind.
    fn note(&mut self, now: Instant, pts_us: i64) -> Lag {
        if !self.settled {
            let (t0, p0) = *self.window.get_or_insert((now, pts_us));
            let wall = now.duration_since(t0);
            if wall >= Duration::from_millis(100) {
                let made_ms = (pts_us - p0) / 1000;
                // pictures arriving at less than one and a half times the pace they were made at: the live edge
                if made_ms * 2 < wall.as_millis() as i64 * 3 {
                    self.settled = true;
                    self.base = Some((now, pts_us));
                    return Lag::Settled;
                }
                self.window = Some((now, pts_us));
            }
            return Lag::Fine;
        }
        let (t_b, p_b) = self.base.expect("a settled stream has its base");
        let lag_ms = now.duration_since(t_b).as_millis() as i64 - (pts_us - p_b) / 1000;
        if lag_ms > LAG_MAX_MS {
            let since = *self.behind_since.get_or_insert(now);
            if now.duration_since(since) >= LAG_FOR {
                return Lag::Behind;
            }
        } else {
            self.behind_since = None;
        }
        Lag::Fine
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lag {
    Fine,
    Settled,
    Behind,
}

// ---- the feeder ----------------------------------------------------------------------------------------------------------------------

/// A feeder process and its socket; the process is ended when this goes.
struct Feeder {
    /// `None` only in a test, whose feeder is a thread behind a socket pair.
    child: Option<Child>,
    socket: PathBuf,
    client: gf::Client,
    window: String,
    /// The feeder loaded LG's NDL library. Without it a texture still plays (it needs no library of LG's); a window's video has no decoder.
    library: bool,
    /// The window type it was started with (`-w`): a session that wants another starts another feeder.
    kind: String,
}

impl Feeder {
    fn start(spec: &FeederSpec, kind: &str) -> Result<Self, String> {
        std::fs::create_dir_all(&spec.run_dir)
            .map_err(|e| format!("{}: {e}", spec.run_dir.display()))?;
        let socket = spec
            .run_dir
            .join(format!("feeder-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let mut command = Command::new(&spec.program);
        command
            .arg("--serve")
            .arg(&socket)
            .arg("-w")
            .arg(kind)
            // the feeder ends when the helper's connection does, whatever happens to the helper
            .arg("--once")
            .arg("--stats")
            .arg(spec.run_dir.join("feeder-stats.txt"));
        if !spec.library.is_empty() {
            command.arg("-L").arg(&spec.library);
        }
        command.args(&spec.extra_args);
        for (k, v) in &spec.env {
            command.env(k, v);
        }
        // the feeder's own words (its window, the app its decoder is opened for, what NDL answered) go to a file: a stream that shows nothing is read from it
        let log = std::fs::File::create(spec.run_dir.join("feeder.log")).map_or_else(|_| Stdio::null(), Stdio::from);
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
        let child = command
            .spawn()
            .map_err(|e| format!("cannot start the feeder {}: {e}", spec.program.display()))?;
        let (client, hello) = match gf::Client::connect(&socket, FEEDER_WITHIN) {
            Ok(pair) => pair,
            Err(e) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("the feeder did not answer on its socket: {e}"));
            }
        };
        let (window, library) = match hello {
            gf::Message::Hello {
                library, window_id, ..
            } => (window_id, library),
            _ => (String::new(), true),
        };
        Ok(Self {
            child: Some(child),
            socket,
            client,
            window,
            library,
            kind: kind.to_owned(),
        })
    }

    fn alive(&mut self) -> bool {
        self.child.as_mut().is_none_or(|c| matches!(c.try_wait(), Ok(None)))
    }

    /// How the feeder's process stands and the last lines it wrote (`feeder.log`, which holds the last run only), for the log when it breaks under a
    /// session: the helper sees only a refused write, and the process may be going or gone.
    fn last_words(&mut self, run_dir: &Path) -> String {
        let status = match self.child.as_mut().map(Child::try_wait) {
            Some(Ok(Some(status))) => format!("the process ended ({status})"),
            Some(Ok(None)) => "the process is still there".to_owned(),
            Some(Err(e)) => format!("the process is unknown ({e})"),
            None => "no process".to_owned(),
        };
        let log = std::fs::read_to_string(run_dir.join("feeder.log")).unwrap_or_default();
        // the GLib assertion it prints every second while it plays says nothing of why it stopped
        let mut tail: Vec<&str> = log.lines().rev().filter(|l| !l.trim().is_empty() && !l.contains("GLib-GObject-CRITICAL")).take(3).collect();
        tail.reverse();
        format!("{status}, its last lines: {}", if tail.is_empty() { "none".to_owned() } else { tail.join(" | ") })
    }
}

impl Drop for Feeder {
    fn drop(&mut self) {
        let _ = self.client.send_unload();
        // the feeder ends by itself once its connection is gone (--once); a moment, then it is ended
        self.client.close();
        if let Some(child) = &mut self.child {
            let deadline = Instant::now() + Duration::from_millis(1500);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }
        }
        let _ = std::fs::remove_file(&self.socket);
    }
}

enum Slot {
    Empty,
    /// A feeder nobody holds, since when, and whether a pipeline is still loaded in it.
    Idle(Box<Feeder>, Instant, bool),
    Leased,
}

/// How a feeder is made (a test makes one behind a socket pair).
type Spawn = Box<dyn Fn(&FeederSpec, &str) -> Result<Feeder, String> + Send + Sync>;

/// The one feeder, kept between sessions for `idle`.
struct Pool {
    spec: FeederSpec,
    spawn: Spawn,
    slot: Mutex<Slot>,
    changed: Condvar,
    reaper: Mutex<Option<JoinHandle<()>>>,
}

impl Pool {
    fn new(spec: FeederSpec) -> Arc<Self> {
        Self::with_spawn(spec, Box::new(Feeder::start))
    }

    fn with_spawn(spec: FeederSpec, spawn: Spawn) -> Arc<Self> {
        Arc::new(Self {
            spec,
            spawn,
            slot: Mutex::new(Slot::Empty),
            changed: Condvar::new(),
            reaper: Mutex::new(None),
        })
    }

    /// The feeder with a window of type `window` (empty: the spec's), started if there is none; waits up to `FEEDER_WAIT` for a session that is going to
    /// give it up.
    fn lease(self: &Arc<Self>, window: &str) -> Result<Lease, String> {
        let window = if window.is_empty() { self.spec.window.as_str() } else { window };
        let deadline = Instant::now() + FEEDER_WAIT;
        let mut slot = self.slot.lock();
        loop {
            match std::mem::replace(&mut *slot, Slot::Leased) {
                Slot::Idle(mut feeder, _, loaded) => {
                    if feeder.alive() && feeder.kind == window {
                        drop(slot);
                        return Ok(Lease {
                            pool: self.clone(),
                            feeder: Some(feeder),
                            loaded,
                        });
                    }
                    // it died while it idled, or its window is not the kind wanted: another is started
                    drop(slot);
                    drop(feeder);
                    return self.start_new(window);
                }
                Slot::Empty => {
                    drop(slot);
                    return self.start_new(window);
                }
                Slot::Leased => {
                    if self.changed.wait_until(&mut slot, deadline).timed_out() {
                        return Err("the video path is busy with another stream".to_owned());
                    }
                }
            }
        }
    }

    /// Starts a feeder; the slot is already `Leased`, so nobody else starts one meanwhile.
    fn start_new(self: &Arc<Self>, window: &str) -> Result<Lease, String> {
        match (self.spawn)(&self.spec, window) {
            Ok(feeder) => Ok(Lease {
                pool: self.clone(),
                feeder: Some(Box::new(feeder)),
                loaded: false,
            }),
            Err(why) => {
                *self.slot.lock() = Slot::Empty;
                self.changed.notify_all();
                Err(why)
            }
        }
    }

    fn give_back(self: &Arc<Self>, feeder: Box<Feeder>, loaded: bool) {
        if self.spec.idle.is_zero() {
            drop(feeder);
            *self.slot.lock() = Slot::Empty;
            self.changed.notify_all();
            return;
        }
        *self.slot.lock() = Slot::Idle(feeder, Instant::now(), loaded);
        self.changed.notify_all();
        self.ensure_reaper();
    }

    /// One thread, started when a feeder is first parked, ends the feeder that nobody took within `idle`.
    fn ensure_reaper(self: &Arc<Self>) {
        let mut reaper = self.reaper.lock();
        if reaper.as_ref().is_some_and(|h| !h.is_finished()) {
            return;
        }
        let weak: Weak<Pool> = Arc::downgrade(self);
        *reaper = thread::Builder::new()
            .name("cameras-feeder-idle".to_owned())
            .stack_size(64 * 1024)
            .spawn(move || {
                loop {
                    let Some(pool) = weak.upgrade() else { return };
                    let mut slot = pool.slot.lock();
                    let wait = match &*slot {
                        Slot::Idle(_, since, _) => {
                            let end = *since + pool.spec.idle;
                            if Instant::now() >= end {
                                let old = std::mem::replace(&mut *slot, Slot::Empty);
                                drop(slot);
                                // ended off the lock: it waits for the process
                                drop(old);
                                continue;
                            }
                            end - Instant::now()
                        }
                        Slot::Leased => Duration::from_secs(60),
                        // nothing kept: the thread is not needed until a feeder is parked again
                        Slot::Empty => return,
                    };
                    pool.changed.wait_for(&mut slot, wait);
                }
            })
            .ok();
    }

    /// Ends the idle feeder now (the helper is stopping).
    fn close(&self) {
        let old = std::mem::replace(&mut *self.slot.lock(), Slot::Empty);
        drop(old);
        self.changed.notify_all();
    }
}

/// The feeder, held by one session.
struct Lease {
    pool: Arc<Pool>,
    feeder: Option<Box<Feeder>>,
    /// A pipeline is loaded in the feeder (and the last picture is on the plane).
    loaded: bool,
}

impl Lease {
    fn feeder(&mut self) -> &mut Feeder {
        self.feeder.as_mut().expect("a lease holds its feeder")
    }

    /// Gives the feeder back, with its pipeline unloaded when `unload`.
    fn release(mut self, unload: bool, healthy: bool) {
        let Some(mut feeder) = self.feeder.take() else {
            return;
        };
        if !healthy || feeder.client.broken() || !feeder.alive() {
            drop(feeder);
            *self.pool.slot.lock() = Slot::Empty;
            self.pool.changed.notify_all();
            return;
        }
        if unload && self.loaded {
            let _ = feeder.client.send_unload();
            self.loaded = false;
        }
        self.pool.give_back(feeder, self.loaded);
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        // a lease that is dropped without `release` (a panic): the feeder goes with it
        if self.feeder.take().is_some() {
            *self.pool.slot.lock() = Slot::Empty;
            self.pool.changed.notify_all();
        }
    }
}

// ---- the session ---------------------------------------------------------------------------------------------------------------------

struct Counters {
    fed: AtomicU64,
    dropped: AtomicU64,
    /// Pictures that never reached the feeder: packets lost on the way and pictures the queue let go.
    lost: AtomicU64,
    /// The newest picture's time in its stream (microseconds), and the epoch it belongs to.
    pts_us: AtomicI64,
}

struct ReaderHandle {
    stop: Arc<AtomicBool>,
    waker: Arc<Waker>,
}

impl ReaderHandle {
    fn end(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.waker.wake();
    }
}

/// What the writer is told of the stream it plays (from the plan, and again from each reader that connects).
#[derive(Clone)]
struct Playing {
    camera: String,
    mode: Mode,
    started_at: i64,
    rate: u8,
    expect: Option<StreamInfo>,
    ndl_app: String,
    texture: bool,
    sub: bool,
    purpose: String,
    window: String,
}

/// The bridge keeps one playback session for each camera and ends the old one (its stream is closed) as soon as it takes a call for another, so
/// the end of a recording's stream while the module asks for another recording of that camera is expected, not the end of the recording. The
/// answer to the call says what happens next: a stream to play (a new epoch), or a refusal, which this remembers until the old stream's end shows
/// that the bridge did end it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Replaced {
    /// Nothing is asked for; the end of the stream is the end of the recording.
    No,
    /// Another recording of the camera is being asked for.
    Asking,
    /// It was asked for and the stream ended meanwhile: the answer says what comes.
    Swallowed,
    /// The call was refused (these are its words) while the old stream still ran: if it ends now, that is why.
    Refused(String),
}

struct Session {
    id: u64,
    inner: Weak<Inner>,
    queue: Arc<Queue>,
    counters: Arc<Counters>,
    /// The session is over: the writer ends and the feeder is given back.
    stop: AtomicBool,
    unload: AtomicBool,
    /// An error the feeder's answers found.
    error: Mutex<Option<String>>,
    reader: Mutex<Option<ReaderHandle>>,
    playing: Mutex<Playing>,
    epoch: AtomicU64,
    /// The next picture the feeder accepts is the stream's first: the view goes to `playing` then.
    awaiting: AtomicBool,
    /// The video window the feeder owns (what the view says), kept here for a view that was reset while the session lived on.
    window: Mutex<String>,
    /// When the session began, and when the feeder took its first picture (for the log: the time to the first picture and the rate it ran at).
    began: Instant,
    first_at: Mutex<Option<Instant>>,
    /// Where a recording stands with the bridge's one-playback-session-per-camera rule.
    replaced: Mutex<Replaced>,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl Session {
    fn ended(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Ends the session: nothing waits for it here; the threads finish by themselves.
    fn end(&self, unload: bool) {
        self.unload.store(unload, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
        if let Some(reader) = self.reader.lock().take() {
            reader.end();
        }
        self.queue.close();
    }

    /// Stops reading the current stream (a seek is about to start another, a pause, the end of a recording): the writer, the feeder and the
    /// picture on the plane stay.
    fn detach(&self) {
        if let Some(reader) = self.reader.lock().take() {
            reader.end();
        }
    }
}

struct Inner {
    deps: Deps,
    pool: Option<Arc<Pool>>,
    /// The session the view is of (0: none) and the view.
    view: Mutex<(u64, View)>,
    session: Mutex<Option<Arc<Session>>>,
    next: AtomicU64,
    publish: Box<dyn Fn(Value) + Send + Sync>,
    /// What streams turned out to be where the bridge declared another size.
    sizes: Mutex<Sizes>,
}

impl Inner {
    /// What the plan's stream is, for loading the pipeline with: the plan's words, with the size the same declaration turned out to have last time.
    fn expected(&self, plan: &Plan) -> Option<StreamInfo> {
        let mut info = stream_of(plan)?;
        (info.width, info.height) = self.sizes.lock().real(&plan.camera, (info.width, info.height));
        Some(info)
    }

    fn emit(&self, view: &View) {
        (self.publish)(view.json());
    }

    /// Changes the view if the session is still the one it is of; publishes it when it changed.
    fn update(&self, session: u64, change: impl FnOnce(&mut View)) {
        let view = {
            let mut guard = self.view.lock();
            if guard.0 != session {
                return;
            }
            let before = guard.1.clone();
            change(&mut guard.1);
            if guard.1 == before {
                return;
            }
            guard.1.clone()
        };
        self.emit(&view);
    }
}

/// The one session, and the door the module and the routes use.
#[derive(Clone)]
pub struct Live {
    inner: Arc<Inner>,
}

impl Live {
    pub fn new(deps: Deps, publish: Box<dyn Fn(Value) + Send + Sync>) -> Self {
        let pool = deps.feeder.clone().map(Pool::new);
        let me = Self {
            inner: Arc::new(Inner {
                deps,
                pool,
                view: Mutex::new((0, View::default())),
                session: Mutex::new(None),
                next: AtomicU64::new(1),
                publish,
                sizes: Mutex::new(Sizes::default()),
            }),
        };
        let view = me.view();
        me.inner.emit(&view);
        me
    }

    /// Whether the live path can run at all (a feeder is configured).
    pub fn available(&self) -> bool {
        self.inner.pool.is_some()
    }

    pub fn view(&self) -> View {
        self.inner.view.lock().1.clone()
    }

    /// The camera and mode of the session that is up, if any.
    pub fn current(&self) -> Option<(String, Mode)> {
        let session = self.inner.session.lock().clone()?;
        if session.ended() {
            return None;
        }
        let playing = session.playing.lock();
        Some((playing.camera.clone(), playing.mode))
    }

    /// The bridge is being asked for a stream: said when nothing else is up (a session that plays goes on playing until the answer).
    pub fn asking(&self, camera: &str, mode: Mode, purpose: &str) {
        if mode != Mode::Live
            && let Some(session) = self.inner.session.lock().clone()
        {
            let playing = session.playing.lock();
            if playing.mode != Mode::Live && playing.camera == camera {
                *session.replaced.lock() = Replaced::Asking;
            }
        }
        let view = {
            let mut guard = self.inner.view.lock();
            if matches!(guard.1.state, "starting" | "playing") {
                return;
            }
            guard.0 = 0;
            guard.1 = View {
                camera: camera.to_owned(),
                mode: mode.word(),
                state: "asking",
                since: crate::timeutil::now_ms(),
                purpose: purpose.to_owned(),
                ..View::default()
            };
            guard.1.clone()
        };
        self.inner.emit(&view);
    }

    /// The bridge refused or did not answer: while the view said `asking` it says `failed`; a recording that plays says so when the bridge
    /// did end it for the call (now, or when its stream ends), and goes on when it did not.
    pub fn asked_in_vain(&self, camera: &str, reason: &str) {
        let session = self.inner.session.lock().clone();
        let view = {
            let mut guard = self.inner.view.lock();
            let replaced = session.as_ref().is_some_and(|session| {
                let mut state = session.replaced.lock();
                match &*state {
                    Replaced::Swallowed => {
                        *state = Replaced::No;
                        true
                    }
                    Replaced::Asking => {
                        *state = Replaced::Refused(reason.to_owned());
                        false
                    }
                    _ => false,
                }
            });
            if replaced {
                if let Some(session) = &session {
                    guard.0 = session.id;
                }
            } else if guard.1.state != "asking" || guard.1.camera != camera {
                return;
            }
            guard.1.state = "failed";
            guard.1.reason = reason.to_owned();
            guard.1.paused = false;
            guard.1.clone()
        };
        self.inner.emit(&view);
    }

    /// Plays `plan`: the same pipeline when a session of the same camera and stream size is up, else a new session. Returns at once; where
    /// it stands is the view. `Err` says why not, now.
    pub fn start(&self, plan: Plan) -> Result<(), String> {
        let Some(pool) = self.inner.pool.clone() else {
            return Err("the live path is off (cameras.live)".to_owned());
        };
        let want = self.inner.expected(&plan);
        let mut current = self.inner.session.lock();
        if let Some(session) = current.as_ref()
            && !session.ended()
            && Self::compatible(session, &plan, want)
        {
            let session = session.clone();
            drop(current);
            return self.attach(&session, plan);
        }
        if let Some(old) = current.take() {
            // the next session loads the pipeline over this one: nothing is unloaded in between
            old.end(false);
        }
        let id = self.inner.next.fetch_add(1, Ordering::SeqCst);
        let session = Arc::new(Session {
            id,
            inner: Arc::downgrade(&self.inner),
            queue: Arc::new(Queue::new()),
            counters: Arc::new(Counters {
                fed: AtomicU64::new(0),
                dropped: AtomicU64::new(0),
                lost: AtomicU64::new(0),
                pts_us: AtomicI64::new(0),
            }),
            stop: AtomicBool::new(false),
            unload: AtomicBool::new(true),
            error: Mutex::new(None),
            reader: Mutex::new(None),
            playing: Mutex::new(Playing {
                camera: plan.camera.clone(),
                mode: plan.mode,
                started_at: plan.started_at,
                rate: plan.rate.max(1),
                expect: want,
                ndl_app: plan.ndl_app.clone(),
                texture: plan.texture,
                sub: plan.sub,
                purpose: plan.purpose.clone(),
                window: plan.window.clone(),
            }),
            epoch: AtomicU64::new(0),
            awaiting: AtomicBool::new(true),
            window: Mutex::new(String::new()),
            began: Instant::now(),
            first_at: Mutex::new(None),
            replaced: Mutex::new(Replaced::No),
            writer: Mutex::new(None),
        });
        {
            let mut view = self.inner.view.lock();
            view.0 = id;
            view.1 = View {
                camera: plan.camera.clone(),
                mode: plan.mode.word(),
                state: "starting",
                codec: want
                    .map(|w| codec_word(w.codec).to_owned())
                    .unwrap_or_default(),
                width: want.map_or(0, |w| w.width),
                height: want.map_or(0, |w| w.height),
                fps: want.map_or(0, |w| w.fps),
                started_at: (plan.mode != Mode::Live).then_some(plan.started_at),
                position_ms: (plan.mode != Mode::Live).then_some(plan.started_at),
                rate: plan.rate.max(1),
                since: crate::timeutil::now_ms(),
                purpose: plan.purpose.clone(),
                ..View::default()
            };
            let snapshot = view.1.clone();
            drop(view);
            self.inner.emit(&snapshot);
        }
        *current = Some(session.clone());
        drop(current);
        let writer = {
            let (session, pool, inner) = (session.clone(), pool, self.inner.clone());
            thread::Builder::new()
                .name("cameras-live".to_owned())
                .stack_size(256 * 1024)
                .spawn(move || write_session(&inner, &pool, &session))
        };
        match writer {
            Ok(handle) => *session.writer.lock() = Some(handle),
            Err(e) => {
                self.inner.update(id, |v| {
                    v.state = "failed";
                    v.reason = e.to_string();
                });
                session.end(true);
                return Err(e.to_string());
            }
        }
        self.attach(&session, plan)
    }

    /// Whether the session's pipeline fits the stream the plan describes (the writer reloads it itself when the stream turns out otherwise).
    fn compatible(session: &Session, plan: &Plan, want: Option<StreamInfo>) -> bool {
        let playing = session.playing.lock();
        playing.camera == plan.camera
            && playing.ndl_app == plan.ndl_app
            && playing.texture == plan.texture
            && playing.sub == plan.sub
            && playing.window == plan.window
            && match (playing.expect, want) {
                (Some(loaded), Some(want)) => loaded.same_pipeline(&want),
                _ => true,
            }
    }

    /// Starts reading the plan's stream into `session`, in place of the one that is read.
    fn attach(&self, session: &Arc<Session>, plan: Plan) -> Result<(), String> {
        session.detach();
        let epoch = session.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *session.replaced.lock() = Replaced::No;
        session.queue.begin(epoch);
        session.awaiting.store(true, Ordering::SeqCst);
        {
            let mut playing = session.playing.lock();
            playing.mode = plan.mode;
            playing.started_at = plan.started_at;
            playing.rate = plan.rate.max(1);
            playing.ndl_app.clone_from(&plan.ndl_app);
            playing.texture = plan.texture;
            playing.sub = plan.sub;
            playing.purpose.clone_from(&plan.purpose);
            playing.window.clone_from(&plan.window);
            if let Some(want) = self.inner.expected(&plan) {
                playing.expect = Some(want);
            }
        }
        // The view is this session's again: it may have been reset to `asking` while the session (a recording that ended, a failure) lived on,
        // so what the session knows of its pipeline is put back with the new stream's words.
        let shape = session.playing.lock().expect;
        let view = {
            let mut guard = self.inner.view.lock();
            guard.0 = session.id;
            let v = &mut guard.1;
            v.camera = plan.camera.clone();
            v.mode = plan.mode.word();
            v.rate = plan.rate.max(1);
            v.started_at = (plan.mode != Mode::Live).then_some(plan.started_at);
            v.position_ms = (plan.mode != Mode::Live).then_some(plan.started_at);
            v.paused = false;
            v.reason = String::new();
            v.state = "starting";
            v.purpose.clone_from(&plan.purpose);
            v.window = session.window.lock().clone();
            if let Some(shape) = shape {
                v.codec = codec_word(shape.codec).to_owned();
                v.width = shape.width;
                v.height = shape.height;
                v.fps = shape.fps;
            }
            if v.since == 0 {
                v.since = crate::timeutil::now_ms();
            }
            guard.1.clone()
        };
        self.inner.emit(&view);
        let stop = Arc::new(AtomicBool::new(false));
        let waker = Arc::new(Waker::new().map_err(|e| e.to_string())?);
        *session.reader.lock() = Some(ReaderHandle {
            stop: stop.clone(),
            waker: waker.clone(),
        });
        let (inner, session2) = (self.inner.clone(), session.clone());
        let spawned = thread::Builder::new()
            .name("cameras-live-rtsp".to_owned())
            .stack_size(256 * 1024)
            .spawn(move || read_stream(&inner, &session2, plan, epoch, &stop, &waker));
        spawned.map(|_| ()).map_err(|e| e.to_string())
    }

    /// Stops reading a recording and keeps the last picture on the plane (`paused` in the view); the module asks again where it stopped
    /// to play on.
    pub fn pause(&self) -> Result<(), String> {
        let Some(session) = self.inner.session.lock().clone() else {
            return Err("nothing is playing".to_owned());
        };
        if session.ended() || session.playing.lock().mode == Mode::Live {
            return Err("only a recording can be paused".to_owned());
        }
        session.detach();
        self.inner.update(session.id, |v| v.paused = true);
        Ok(())
    }

    /// `POST /v1/cameras/live/stop`, the viewer closing, the link going: nothing waits here. A view that is still `asking` is settled too
    /// (the answer to that ask is refused by the ticket), and a view reset while its session lived on goes back to being the session's.
    pub fn stop(&self, why: &str) {
        let session = self.inner.session.lock().take();
        if let Some(session) = &session {
            session.end(true);
        }
        let view = {
            let mut guard = self.inner.view.lock();
            if session.is_none() && guard.1.state != "asking" {
                return;
            }
            if let Some(session) = &session {
                guard.0 = session.id;
            }
            let before = guard.1.clone();
            let v = &mut guard.1;
            v.state = "stopped";
            v.reason = why.to_owned();
            v.window = String::new();
            v.paused = false;
            (guard.1 != before).then(|| guard.1.clone())
        };
        if let Some(view) = view {
            self.inner.emit(&view);
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(session) = self.session.get_mut().take() {
            session.end(true);
        }
        if let Some(pool) = &self.pool {
            pool.close();
        }
    }
}

/// What a plan says the stream is, when it says enough to load the pipeline with.
fn stream_of(plan: &Plan) -> Option<StreamInfo> {
    let codec = codec_of(&plan.codec)?;
    (plan.width > 0 && plan.height > 0).then_some(StreamInfo {
        codec,
        width: plan.width,
        height: plan.height,
        fps: plan.fps,
    })
}

// ---- the writer ----------------------------------------------------------------------------------------------------------------------

/// The feeder's answers, read on a thread of their own so that the writer never waits for one.
struct Answers {
    handle: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    waker: Arc<Waker>,
}

impl Answers {
    /// Reads the feeder's replies for `session` until it is dropped. Everything before the answer to the first ping is the last session's.
    fn start(feeder: &Feeder, session: &Arc<Session>) -> Result<Self, String> {
        let mut stream = feeder.client.try_clone().map_err(|e| e.to_string())?;
        let waker = Arc::new(Waker::new().map_err(|e| e.to_string())?);
        let stop = Arc::new(AtomicBool::new(false));
        let (session, stop2, waker2) = (session.clone(), stop.clone(), waker.clone());
        let fd = stream.as_raw_fd();
        let handle = thread::Builder::new()
            .name("cameras-live-feeder".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut barrier = true;
                while !stop2.load(Ordering::SeqCst) {
                    // a frame of the feeder's is whole in a moment once its first byte is there
                    if !waker2.wait(fd, Duration::from_secs(1)) {
                        continue;
                    }
                    let Ok((kind, payload)) = gf::read_frame(&mut stream) else {
                        break;
                    };
                    let message = gf::decode(kind, &payload);
                    if barrier {
                        if matches!(message, gf::Message::Pong { echo, .. } if echo == session.id as i64) {
                            barrier = false;
                        }
                        continue;
                    }
                    match message {
                        gf::Message::Accept(_) => {
                            session.counters.fed.fetch_add(1, Ordering::Relaxed);
                            if session.awaiting.swap(false, Ordering::SeqCst)
                                && let Some(inner) = session.inner.upgrade()
                            {
                                session.first_at.lock().get_or_insert_with(Instant::now);
                                inner.update(session.id, |v| {
                                    if v.state == "starting" {
                                        v.state = "playing";
                                    }
                                });
                            }
                        }
                        gf::Message::Drop { .. } => {
                            session.counters.dropped.fetch_add(1, Ordering::Relaxed);
                        }
                        gf::Message::Loaded { ok: false, text } => {
                            *session.error.lock() = Some(format!("the pipeline refused the load: {text}"));
                        }
                        gf::Message::Error { code, text } => {
                            *session.error.lock() = Some(format!("the feeder failed ({code}): {text}"));
                        }
                        _ => {}
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            handle: Some(handle),
            stop,
            waker,
        })
    }
}

impl Drop for Answers {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.waker.wake();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Loads the pipeline with `info` in the leased feeder, for the app named (empty: the video window's own).
/// The size of the picture the feeder paints for a texture: the stream's own, scaled down to fit 640x360 (a camera's second stream is that or less; a
/// larger one is halved by the feeder before it is stretched over the claim's rectangle, which at the UI's 1920x1080 is about the box's own size).
pub fn texture_size(width: u32, height: u32) -> (u32, u32) {
    const FIT: (u32, u32) = (640, 360);
    if width == 0 || height == 0 {
        return FIT;
    }
    let scale = (f64::from(FIT.0) / f64::from(width)).min(f64::from(FIT.1) / f64::from(height)).min(1.0);
    let even = |v: f64| ((v * scale).round() as u32).max(16) & !1;
    (even(f64::from(width)), even(f64::from(height)))
}

fn load(lease: &mut Lease, info: StreamInfo, ndl_app: &str, texture: bool, sub: bool) -> Result<(), String> {
    let codec = match info.codec {
        Codec::H264 => gf::CODEC_H264,
        Codec::H265 => gf::CODEC_H265,
    };
    if !texture && !lease.feeder().library {
        return Err("the feeder has no decoder library".to_owned());
    }
    let client = &mut lease.feeder().client;
    let (tw, th) = if texture { texture_size(info.width, info.height) } else { (0, 0) };
    client.send_texture(tw, th).map_err(|e| e.to_string())?;
    client.send_app(ndl_app).map_err(|e| e.to_string())?;
    // the sub window: the starfish pipeline asking for the sub sink; everything else the NDL session on the main window, as before
    client.send_pipe(if sub { "smp" } else { "ndl" }).map_err(|e| e.to_string())?;
    client.send_sink(if sub { "sub_video" } else { "" }).map_err(|e| e.to_string())?;
    client
        .send_load(codec, info.width, info.height, info.fps)
        .map_err(|e| e.to_string())?;
    client.send_state(true).map_err(|e| e.to_string())?;
    lease.loaded = true;
    Ok(())
}

/// One line of the log when a stream's session is over: what it was for, how long the first picture took and how many a second the feeder took.
fn log_end(session: &Session) {
    let (camera, purpose) = {
        let playing = session.playing.lock();
        (playing.camera.clone(), playing.purpose.clone())
    };
    let (fed, dropped) = (
        session.counters.fed.load(Ordering::Relaxed),
        session.counters.dropped.load(Ordering::Relaxed),
    );
    let lost = session.counters.lost.load(Ordering::Relaxed) + session.queue.dropped();
    let for_ = if purpose.is_empty() { String::new() } else { format!(" for {purpose}") };
    match *session.first_at.lock() {
        Some(first) => {
            let secs = first.elapsed().as_secs_f64();
            crate::log_info!(
                "cameras: live{for_} on camera {camera} ended: first picture after {} ms, {fed} pictures in {secs:.1} s = {:.1} a second, {dropped} refused, {lost} lost",
                first.duration_since(session.began).as_millis(),
                if secs > 0.0 { fed as f64 / secs } else { 0.0 },
            );
        }
        None => crate::log_info!(
            "cameras: live{for_} on camera {camera} ended with no picture shown after {:.1} s ({fed} taken, {dropped} refused, {lost} lost)",
            session.began.elapsed().as_secs_f64()
        ),
    }
}

/// `why` names no address (`words_of`), so the log may carry it: without it the line says only that a session failed.
fn fail(inner: &Inner, session: &Session, why: &str) {
    let camera = session.playing.lock().camera.clone();
    crate::log_warn!("cameras: live session {} on camera {camera} failed: {why}", session.id);
    inner.update(session.id, |v| {
        v.state = "failed";
        v.reason = why.to_owned();
    });
}

/// The session's thread: leases the feeder, loads it, and feeds it what the readers queue until the session ends.
fn write_session(inner: &Arc<Inner>, pool: &Arc<Pool>, session: &Arc<Session>) {
    write_session_with(inner, pool, session, Answers::start);
}

/// How many times a feeder that breaks under a session is replaced. Seen on the set: the compositor closes a video window whose claim ended (a stream that
/// no longer fits it), the feeder goes with its window, and the next session was already handed the process a moment before it went. One new feeder
/// is the cure for that; one that breaks again is not a cure for anything, and the person is told.
const RESTARTS: u8 = 1;

fn write_session_with(
    inner: &Arc<Inner>,
    pool: &Arc<Pool>,
    session: &Arc<Session>,
    mut start_answers: impl FnMut(&Feeder, &Arc<Session>) -> Result<Answers, String>,
) {
    let window_kind = session.playing.lock().window.clone();
    let mut lease = match pool.lease(&window_kind) {
        Ok(lease) => lease,
        Err(why) => {
            fail(inner, session, &why);
            session.end(true);
            return;
        }
    };
    let mut restarts = 0;
    let mut resume = Resume::default();
    let healthy = loop {
        let window = lease.feeder().window.clone();
        session.window.lock().clone_from(&window);
        inner.update(session.id, |v| v.window = window);
        let answers = match start_answers(lease.feeder(), session) {
            Ok(answers) => answers,
            Err(why) => {
                fail(inner, session, &why);
                lease.release(true, false);
                finish(inner, session);
                return;
            }
        };
        // what the feeder said before this session began is read past: its answer to this ping is the line
        let fed = match lease.feeder().client.send_ping(session.id as i64) {
            Ok(()) => feed(inner, session, &mut lease, std::mem::take(&mut resume)),
            Err(e) => Fed::Broke(format!("the feeder went: {e}"), std::mem::take(&mut resume)),
        };
        // the answers' thread goes first, so that the feeder is not given back with a reader still on its socket
        drop(answers);
        let (why, carry) = match fed {
            Fed::Done(healthy) => break healthy,
            Fed::Broke(why, carry) => (why, carry),
        };
        let again = restarts < RESTARTS && !session.ended();
        let camera = session.playing.lock().camera.clone();
        crate::log_warn!(
            "cameras: the feeder broke under session {} on camera {camera}: {why}; {}{}",
            session.id,
            lease.feeder().last_words(&pool.spec.run_dir),
            if again { "; a new one takes over" } else { "" }
        );
        if !again {
            fail(inner, session, &why);
            break false;
        }
        restarts += 1;
        // what the broken feeder said last is not this session's to act on
        *session.error.lock() = None;
        lease.release(false, false);
        lease = match pool.lease(&window_kind) {
            Ok(lease) => lease,
            Err(why) => {
                fail(inner, session, &why);
                log_end(session);
                finish(inner, session);
                return;
            }
        };
        resume = carry;
    };
    log_end(session);
    lease.release(session.unload.load(Ordering::SeqCst), healthy);
    finish(inner, session);
}

/// A session that is over is not one a new stream can be attached to; a failed one has said why in `fail`.
fn finish(inner: &Inner, session: &Session) {
    session.end(true);
    let mut current = inner.session.lock();
    if current.as_ref().is_some_and(|s| s.id == session.id) {
        *current = None;
    }
}

/// What a feeding loop ended with.
enum Fed {
    /// The session is over, or it failed for a reason of its own (`fail` has said so): whether the feeder is fit to be given back.
    Done(bool),
    /// The feeder broke under the session (a write failed, it is gone, or it said it failed): another can carry on from what is kept.
    Broke(String, Resume),
}

/// What a new feeder needs to carry on where a broken one stopped.
#[derive(Default)]
struct Resume {
    /// The pipeline the broken feeder held, or was being loaded with.
    loaded: Option<StreamInfo>,
    /// The picture that was in hand when it broke.
    unit: Option<Unit>,
}

/// The feeding loop.
fn feed(inner: &Arc<Inner>, session: &Arc<Session>, lease: &mut Lease, resume: Resume) -> Fed {
    let went = |e: io::Error| format!("the feeder went: {e}");
    let (expect, ndl_app, texture, sub) = {
        let playing = session.playing.lock();
        (playing.expect, playing.ndl_app.clone(), playing.texture, playing.sub)
    };
    let mut loaded: Option<StreamInfo> = None;
    let mut pending = resume.unit;
    // a pipeline just loaded takes a stream from a random access point: what comes before the next one is of no use to it
    let mut need_key = resume.loaded.is_some();
    // the load the bridge's description allows is started before the stream is dialled; a new feeder first takes up the pipeline the broken one held
    if let Some(info) = resume.loaded.or(expect) {
        if let Err(why) = load(lease, info, &ndl_app, texture, sub) {
            return Fed::Broke(why, Resume { loaded: Some(info), unit: pending });
        }
        loaded = Some(info);
    }
    let mut last_pts: i64 = -1;
    let mut next_base: i64 = 0;
    let mut epoch_first: Option<i64> = None;
    let mut delay_max = Duration::ZERO;
    let mut last_publish = Instant::now();
    let mut last_second = Instant::now();
    let mut rate_mark = (Instant::now(), 0u64);
    loop {
        if session.ended() {
            return Fed::Done(true);
        }
        if let Some(why) = session.error.lock().take() {
            return Fed::Broke(why, Resume { loaded, unit: pending });
        }
        if last_publish.elapsed() >= PUBLISH_EVERY {
            last_publish = Instant::now();
            publish_counts(
                inner,
                session,
                last_second.elapsed() >= Duration::from_secs(1).saturating_sub(PUBLISH_EVERY),
                &mut delay_max,
                &mut last_second,
                &mut rate_mark,
            );
        }
        let item = match pending.take() {
            Some(unit) => Item::Unit(unit),
            None => match session.queue.take(PUBLISH_EVERY) {
                Taken::Item(item) => item,
                Taken::Nothing => continue,
                Taken::Closed => return Fed::Done(true),
            },
        };
        match item {
            Item::Stream(info) => {
                // a stream of another codec or size than the pipeline holds loads it again; the same one flushes what the pipeline still
                // holds of the last stream and goes on, with times that keep rising
                if !loaded.is_some_and(|l| l.same_pipeline(&info)) {
                    if lease.loaded
                        && let Err(e) = lease.feeder().client.send_unload()
                    {
                        return Fed::Broke(went(e), Resume { loaded, unit: None });
                    }
                    if let Err(why) = load(lease, info, &ndl_app, texture, sub) {
                        return Fed::Broke(why, Resume { loaded: Some(info), unit: None });
                    }
                    loaded = Some(info);
                    need_key = false;
                    last_pts = -1;
                    next_base = 0;
                } else if last_pts >= 0 {
                    if let Err(e) = lease.feeder().client.send_flush() {
                        return Fed::Broke(went(e), Resume { loaded, unit: None });
                    }
                    next_base = last_pts + 100_000_000;
                }
                epoch_first = None;
                inner.update(session.id, |v| {
                    v.codec = codec_word(info.codec).to_owned();
                    v.width = info.width;
                    v.height = info.height;
                    if info.fps > 0 {
                        v.fps = info.fps;
                    }
                });
            }
            Item::Unit(unit) => {
                let Some(info) = loaded else { continue };
                if need_key {
                    if !unit.key {
                        continue;
                    }
                    need_key = false;
                }
                // a picture whose own parameter set says another size than the pipeline holds: the stream changed under the reader
                if unit.key
                    && let Some((w, h)) = unit.size
                    && (w, h) != (info.width, info.height)
                {
                    let changed = StreamInfo {
                        width: w,
                        height: h,
                        ..info
                    };
                    if let Err(e) = lease.feeder().client.send_unload() {
                        return Fed::Broke(went(e), Resume { loaded, unit: Some(unit) });
                    }
                    if let Err(why) = load(lease, changed, &ndl_app, texture, sub) {
                        return Fed::Broke(why, Resume { loaded: Some(changed), unit: Some(unit) });
                    }
                    loaded = Some(changed);
                    // the shell fits the video window by the view's size, so the view follows the picture that is on the plane
                    inner.update(session.id, |v| {
                        v.width = w;
                        v.height = h;
                    });
                    last_pts = -1;
                    next_base = 0;
                    epoch_first = None;
                }
                let first = *epoch_first.get_or_insert(unit.pts_us);
                let mut pts_ns = next_base + (unit.pts_us - first).max(0) * 1000;
                // the pipeline insists on rising times: a picture whose time goes back is placed just after the last
                if pts_ns <= last_pts {
                    pts_ns = last_pts + 1_000_000;
                }
                last_pts = pts_ns;
                delay_max = delay_max.max(unit.arrived.elapsed());
                if let Err(e) = lease.feeder().client.send_video(pts_ns, unit.key, &unit.data) {
                    return Fed::Broke(went(e), Resume { loaded, unit: Some(unit) });
                }
                session.counters.pts_us.store(unit.pts_us, Ordering::Relaxed);
            }
        }
    }
}

fn publish_counts(
    inner: &Inner,
    session: &Session,
    second: bool,
    delay_max: &mut Duration,
    last_second: &mut Instant,
    rate_mark: &mut (Instant, u64),
) {
    let (fed, dropped) = (
        session.counters.fed.load(Ordering::Relaxed),
        session.counters.dropped.load(Ordering::Relaxed),
    );
    let lost = session.counters.lost.load(Ordering::Relaxed) + session.queue.dropped();
    let (position, delay) = {
        let playing = session.playing.lock();
        let pts_ms = session.counters.pts_us.load(Ordering::Relaxed) / 1000;
        // the recorder paces its stream to the rate it was asked for: a second of stream is `rate` seconds of recording [INFERENCE: TV check C10]
        let position = (playing.mode != Mode::Live)
            .then(|| playing.started_at + pts_ms * i64::from(playing.rate));
        (position, delay_max.as_millis() as u64)
    };
    // the rate the feeder takes pictures at, over the second that has passed (the declared one is the camera's own claim)
    let fed_fps = if second {
        let secs = rate_mark.0.elapsed().as_secs_f32();
        let fps = (secs > 0.2).then(|| fed.saturating_sub(rate_mark.1) as f32 / secs);
        *rate_mark = (Instant::now(), fed);
        fps
    } else {
        None
    };
    inner.update(session.id, |v| {
        v.fed = fed;
        v.dropped = dropped;
        v.lost = lost;
        if second {
            v.delay_ms = delay;
        }
        if let Some(fps) = fed_fps {
            v.fed_fps = fps;
        }
        if position.is_some() && !v.paused && v.state != "stopped" {
            v.position_ms = position;
        }
    });
    if second {
        *delay_max = Duration::ZERO;
        *last_second = Instant::now();
    }
}

// ---- the reader ----------------------------------------------------------------------------------------------------------------------

/// How a stream ended.
enum Ended {
    /// The session or the plan went: nothing to say.
    Told,
    /// The stream's end: a recording that is over.
    End,
    /// Nothing came for `QUIET`.
    Quiet,
    /// The pictures run behind.
    Behind,
    /// The connection ended or failed.
    Lost(String),
}

fn read_stream(
    inner: &Arc<Inner>,
    session: &Arc<Session>,
    mut plan: Plan,
    epoch: u64,
    stop: &AtomicBool,
    waker: &Waker,
) {
    let mode = plan.mode;
    let mut redials = u8::from(mode == Mode::Live);
    let outcome = loop {
        let outcome = read_once(inner, session, &plan, epoch, stop, waker);
        // a live stream that fell behind or dropped is dialled again, once
        if matches!(outcome, Ended::Behind | Ended::Lost(_))
            && redials > 0
            && !stop.load(Ordering::SeqCst)
            && !session.ended()
        {
            redials -= 1;
            // the old connection is closed by now; a moment, so that the sender has let go of it
            thread::sleep(Duration::from_millis(200));
            continue;
        }
        break outcome;
    };
    let told = stop.load(Ordering::SeqCst) || session.ended();
    // the stream is over: the bridge is told, so that its playback session is free for the next
    if let Some(end) = plan.on_end.take() {
        end();
    }
    if told || !is_current(session, epoch) {
        return;
    }
    if matches!(outcome, Ended::End | Ended::Lost(_)) {
        let mut replaced = session.replaced.lock();
        match &*replaced {
            // the bridge ended this recording to take the call for another: the call's answer says what plays next
            Replaced::Asking => {
                *replaced = Replaced::Swallowed;
                return;
            }
            // the call was refused, and the bridge had ended the recording first: that is what the person is told
            Replaced::Refused(why) => {
                let why = why.clone();
                *replaced = Replaced::No;
                drop(replaced);
                fail(inner, session, &why);
                return;
            }
            _ => {}
        }
    }
    match outcome {
        Ended::Told => {}
        Ended::End => {
            // the last pictures are shown before the end is said
            let deadline = Instant::now() + Duration::from_secs(2);
            while session.counters.fed.load(Ordering::Relaxed)
                + session.counters.dropped.load(Ordering::Relaxed)
                == 0
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(20));
            }
            inner.update(session.id, |v| {
                v.state = "stopped";
                v.reason = "End of recording".to_owned();
            });
        }
        Ended::Quiet => fail(inner, session, "the stream went quiet"),
        Ended::Behind => fail(inner, session, "the stream ran behind and did not catch up"),
        Ended::Lost(why) => fail(inner, session, &why),
    }
}

fn is_current(session: &Session, epoch: u64) -> bool {
    session.epoch.load(Ordering::SeqCst) == epoch
}

fn read_once(
    inner: &Arc<Inner>,
    session: &Arc<Session>,
    plan: &Plan,
    epoch: u64,
    stop: &AtomicBool,
    waker: &Waker,
) -> Ended {
    let mut rtsp = match (inner.deps.dial)(&plan.url) {
        Ok(rtsp) => rtsp,
        Err(e) => {
            return if stop.load(Ordering::SeqCst) {
                Ended::Told
            } else {
                Ended::Lost(words_of(&e))
            };
        }
    };
    if stop.load(Ordering::SeqCst) || session.ended() {
        return Ended::Told;
    }
    let stream = rtsp.stream();
    let info = StreamInfo {
        codec: stream.codec,
        width: stream.info.width,
        height: stream.info.height,
        fps: stream.info.fps.map_or(plan.fps, |f| f.round() as u32),
    };
    // what the stream says it is, against what the bridge declared: the next start of this camera and declaration loads the pipeline for the real size
    if plan.width > 0 && plan.height > 0 && info.width > 0 && info.height > 0 {
        inner.sizes.lock().learn(&plan.camera, (plan.width, plan.height), (info.width, info.height));
    }
    if !session.queue.stream(epoch, info) {
        return Ended::Told;
    }
    let mut lag = LagWatch::new();
    let mut frames: Vec<Frame> = Vec::new();
    let mut waiting: VecDeque<Unit> = VecDeque::new();
    loop {
        if stop.load(Ordering::SeqCst) || session.ended() {
            return Ended::Told;
        }
        // a picture the queue had no room for goes first; while there is none the connection is not read, so the sender is held back
        while let Some(unit) = waiting.pop_front() {
            match session.queue.give(epoch, unit) {
                Give::Done => {}
                Give::Stale => return Ended::Told,
                Give::Full(unit) => {
                    waiting.push_front(unit);
                    session.queue.wait_room(Duration::from_millis(100));
                    break;
                }
            }
        }
        if !waiting.is_empty() {
            continue;
        }
        let now = Instant::now();
        if rtsp.keepalive_due(now)
            && let Err(e) = rtsp.send_keepalive()
        {
            return Ended::Lost(words_of(&e));
        }
        if rtsp.stalled(now, QUIET) {
            return Ended::Quiet;
        }
        let until_keepalive = rtsp.keepalive_at().saturating_duration_since(now);
        if !waker.wait(rtsp.fd(), until_keepalive.min(Duration::from_millis(500))) {
            continue;
        }
        frames.clear();
        let read = rtsp.read(&mut frames);
        let arrived = Instant::now();
        for frame in frames.drain(..) {
            match lag.note(arrived, frame.pts_us) {
                Lag::Settled => session.queue.settle(),
                Lag::Behind => return Ended::Behind,
                Lag::Fine => {}
            }
            waiting.push_back(Unit {
                size: frame.sps.map(|i| (i.width, i.height)),
                data: frame.annexb,
                pts_us: frame.pts_us,
                key: frame.random_access,
                arrived,
            });
        }
        let stats = rtsp.stats();
        session
            .counters
            .lost
            .store(stats.lost_packets + stats.dropped, Ordering::Relaxed);
        if let Err(e) = read {
            // what had come is given to the queue before the end is taken in (the burst's queue may be full: it is waited for, a while)
            let deadline = Instant::now() + Duration::from_secs(2);
            while let Some(unit) = waiting.pop_front() {
                match session.queue.give(epoch, unit) {
                    Give::Done => {}
                    Give::Stale => break,
                    Give::Full(unit) => {
                        waiting.push_front(unit);
                        if Instant::now() >= deadline || stop.load(Ordering::SeqCst) {
                            break;
                        }
                        session.queue.wait_room(Duration::from_millis(50));
                    }
                }
            }
            return match e.kind {
                rtsp_client::Kind::Closed => Ended::End,
                _ => Ended::Lost(words_of(&e)),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(key: bool, pts_us: i64, size: usize) -> Unit {
        Unit {
            data: vec![0; size],
            pts_us,
            key,
            size: None,
            arrived: Instant::now(),
        }
    }

    fn units_of(queue: &Queue) -> Vec<(bool, i64)> {
        let s = queue.state.lock();
        s.items
            .iter()
            .filter_map(|i| match i {
                Item::Unit(u) => Some((u.key, u.pts_us)),
                Item::Stream(_) => None,
            })
            .collect()
    }

    #[test]
    fn a_key_picture_with_a_backlog_behind_it_is_where_the_stream_starts_from() {
        let q = Queue::new();
        q.begin(1);
        for (i, key) in [true, false, false, false, true, false]
            .into_iter()
            .enumerate()
        {
            assert!(matches!(q.give(1, unit(key, i as i64, 10)), Give::Done));
        }
        // the first key picture's group is gone when the second came with three pictures waiting
        assert_eq!(units_of(&q), vec![(true, 4), (false, 5)]);
        assert_eq!(
            q.dropped(),
            4,
            "the pictures before the second key one were dropped"
        );
    }

    #[test]
    fn a_stale_reader_gives_nothing_and_a_new_stream_empties_the_queue() {
        let q = Queue::new();
        q.begin(1);
        assert!(matches!(q.give(1, unit(true, 0, 10)), Give::Done));
        assert!(matches!(q.give(2, unit(true, 1, 10)), Give::Stale));
        q.begin(2);
        assert!(units_of(&q).is_empty());
        assert!(
            matches!(q.give(1, unit(true, 2, 10)), Give::Stale),
            "the replaced reader is told to stop"
        );
        assert!(matches!(q.give(2, unit(true, 3, 10)), Give::Done));
        q.close();
        assert!(matches!(q.give(2, unit(true, 4, 10)), Give::Stale));
    }

    #[test]
    fn once_caught_up_the_queue_holds_fifteen_pictures_and_drops_to_the_next_key_picture() {
        let q = Queue::new();
        q.begin(1);
        q.settle();
        for i in 0..QUEUE_UNITS {
            assert!(matches!(q.give(1, unit(i == 0, i as i64, 10)), Give::Done));
        }
        assert_eq!(units_of(&q).len(), QUEUE_UNITS);
        // one more, with the only key picture at the front of fifteen: everything is dropped and the stream waits for the next key
        assert!(matches!(q.give(1, unit(false, 100, 10)), Give::Done));
        assert!(units_of(&q).is_empty(), "{:?}", units_of(&q));
        // a picture that cannot be shown without its key picture is not kept
        assert!(matches!(q.give(1, unit(false, 101, 10)), Give::Done));
        assert!(units_of(&q).is_empty());
        assert!(matches!(q.give(1, unit(true, 102, 10)), Give::Done));
        assert_eq!(units_of(&q), vec![(true, 102)]);
        assert_eq!(q.dropped(), 15 + 2);
        // with a newer key picture among the fifteen the stream goes on from it
        let q = Queue::new();
        q.begin(1);
        q.settle();
        for i in 0..QUEUE_UNITS {
            assert!(matches!(
                q.give(1, unit(i == 0 || i == 10, i as i64, 10)),
                Give::Done
            ));
        }
        assert!(matches!(q.give(1, unit(false, 100, 10)), Give::Done));
        let kept = units_of(&q);
        assert_eq!(
            (kept.first(), kept.last()),
            (Some(&(true, 10)), Some(&(false, 100)))
        );
        assert_eq!(kept.len(), 6);
    }

    #[test]
    fn while_catching_up_the_burst_is_held_back_by_bytes_not_dropped() {
        let q = Queue::new();
        q.begin(1);
        let big = QUEUE_BYTES / 4 + 1;
        // a long group of pictures: nothing can be dropped, so the reader is made to wait
        assert!(matches!(q.give(1, unit(true, 0, big)), Give::Done));
        assert!(matches!(q.give(1, unit(false, 1, big)), Give::Done));
        assert!(matches!(q.give(1, unit(false, 2, big)), Give::Done));
        let held = match q.give(1, unit(false, 3, big)) {
            Give::Full(unit) => unit,
            _ => panic!("the burst is full"),
        };
        assert_eq!(q.dropped(), 0);
        // the writer takes one (the hold ends when the stream is caught up) and there is room again
        q.settle();
        assert!(matches!(
            q.take(Duration::from_millis(50)),
            Taken::Item(Item::Unit(_))
        ));
        assert!(matches!(q.give(1, held), Give::Done));
    }

    #[test]
    fn the_writer_is_held_back_for_the_first_pictures_and_not_for_the_streams_description() {
        let q = Queue::new();
        q.begin(1);
        assert!(q.stream(
            1,
            StreamInfo {
                codec: Codec::H264,
                width: 640,
                height: 360,
                fps: 15
            }
        ));
        assert!(matches!(q.give(1, unit(true, 0, 10)), Give::Done));
        let started = Instant::now();
        assert!(
            matches!(q.take(Duration::from_secs(1)), Taken::Item(Item::Stream(_))),
            "the description is not held"
        );
        match q.take(Duration::from_secs(1)) {
            Taken::Item(Item::Unit(_)) => {}
            _ => panic!("a picture"),
        }
        assert!(
            started.elapsed() >= HOLD - Duration::from_millis(20),
            "the picture waited for the burst to gather"
        );
        let quiet = Queue::new();
        assert!(matches!(
            quiet.take(Duration::from_millis(20)),
            Taken::Nothing
        ));
        quiet.close();
        assert!(matches!(
            quiet.take(Duration::from_millis(20)),
            Taken::Closed
        ));
    }

    #[test]
    fn a_stream_is_caught_up_when_its_pictures_arrive_at_the_pace_they_were_made_and_behind_when_they_slip()
     {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut lag = LagWatch::new();
        // the replay burst: two seconds of pictures in 150 ms
        let mut settled = false;
        for i in 0..40u64 {
            let now = at(i * 4);
            match lag.note(now, i as i64 * 50_000) {
                Lag::Settled => settled = true,
                Lag::Behind => panic!("a burst is not lag"),
                Lag::Fine => {}
            }
        }
        assert!(!settled, "40 pictures in 160 ms are still a burst");
        // then pictures at their own pace: the live edge
        let mut base_ms = 160u64;
        let mut pts = 40i64 * 50_000;
        let mut got = false;
        for _ in 0..8 {
            base_ms += 50;
            pts += 50_000;
            if lag.note(at(base_ms), pts) == Lag::Settled {
                got = true;
            }
        }
        assert!(got, "the pace of the live edge settles the stream");
        // now the network slips: each picture is 60 ms later than the last should have been
        let mut result = Lag::Fine;
        let mut ms = base_ms;
        for _ in 0..60 {
            ms += 110;
            pts += 50_000;
            result = lag.note(at(ms), pts);
            if result == Lag::Behind {
                break;
            }
        }
        assert_eq!(result, Lag::Behind);
        // a stream that stays on pace is never behind
        let mut steady = LagWatch::new();
        let (mut m, mut p) = (0u64, 0i64);
        for _ in 0..400 {
            m += 50;
            p += 50_000;
            assert_ne!(steady.note(at(m), p), Lag::Behind);
        }
    }

    #[test]
    fn a_texture_is_the_streams_own_size_scaled_to_fit_640_by_360_and_even() {
        assert_eq!(texture_size(1280, 720), (640, 360));
        assert_eq!(texture_size(1920, 1080), (640, 360));
        assert_eq!(texture_size(640, 360), (640, 360));
        assert_eq!(texture_size(320, 180), (320, 180), "a small stream is not made larger");
        assert_eq!(texture_size(640, 480), (480, 360), "the aspect is kept");
        assert_eq!(texture_size(3840, 2160), (640, 360));
        assert_eq!(texture_size(0, 0), (640, 360), "a stream that has not said its size");
        let (w, h) = texture_size(721, 405);
        assert!(w % 2 == 0 && h % 2 == 0 && w <= 640 && h <= 360, "{w}x{h}");
    }

    #[test]
    fn a_plan_names_what_the_pipeline_is_loaded_with_only_when_it_says_enough() {
        let mut plan = Plan {
            camera: "273".into(),
            mode: Mode::Live,
            url: "rtsp://192.168.1.69:34135/abc".into(),
            codec: "H265".into(),
            width: 3840,
            height: 2160,
            fps: 15,
            started_at: 0,
            rate: 1,
            ndl_app: String::new(),
            texture: false,
            sub: false,
            purpose: String::new(),
            window: String::new(),
            on_end: None,
        };
        assert_eq!(
            stream_of(&plan),
            Some(StreamInfo {
                codec: Codec::H265,
                width: 3840,
                height: 2160,
                fps: 15
            })
        );
        assert!(!format!("{plan:?}").contains("34135"));
        plan.codec = "av1".into();
        assert_eq!(stream_of(&plan), None);
        plan.codec = "h264".into();
        plan.width = 0;
        assert_eq!(stream_of(&plan), None);
    }

    #[test]
    fn the_view_says_what_the_shell_reads() {
        let v = View {
            camera: "273".into(),
            state: "playing",
            codec: "h265".into(),
            width: 3840,
            height: 2160,
            fps: 15,
            ..View::default()
        }
        .json();
        for key in [
            "camera",
            "mode",
            "state",
            "reason",
            "codec",
            "width",
            "height",
            "fps",
            "startedAt",
            "positionMs",
            "rate",
            "fed",
            "dropped",
            "lost",
            "delayMs",
            "window",
            "since",
            "paused",
            "purpose",
        ] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert_eq!(
            (
                v["codec"].as_str(),
                v["window"].is_null(),
                v["positionMs"].is_null()
            ),
            (Some("h265"), true, true)
        );
        assert!(View::default().json()["camera"].is_null());
    }

    fn test_session(id: u64, inner: &Arc<Inner>, info: StreamInfo) -> Arc<Session> {
        Arc::new(Session {
            id,
            inner: Arc::downgrade(inner),
            queue: Arc::new(Queue::new()),
            counters: Arc::new(Counters {
                fed: AtomicU64::new(0),
                dropped: AtomicU64::new(0),
                lost: AtomicU64::new(0),
                pts_us: AtomicI64::new(0),
            }),
            stop: AtomicBool::new(false),
            unload: AtomicBool::new(true),
            error: Mutex::new(None),
            reader: Mutex::new(None),
            playing: Mutex::new(Playing {
                camera: "273".into(),
                mode: Mode::Live,
                started_at: 0,
                rate: 1,
                expect: Some(info),
                ndl_app: String::new(),
                texture: true,
                sub: false,
                purpose: String::new(),
                window: String::new(),
            }),
            epoch: AtomicU64::new(0),
            awaiting: AtomicBool::new(true),
            window: Mutex::new(String::new()),
            began: Instant::now(),
            first_at: Mutex::new(None),
            replaced: Mutex::new(Replaced::No),
            writer: Mutex::new(None),
        })
    }

    /// A feeder that is a thread behind a socket pair (no process); `peer` is the process's end of the socket.
    fn test_feeder(dir: &Path, stall: Duration) -> (Feeder, std::os::unix::net::UnixStream) {
        let (ours, peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let feeder = Feeder {
            child: None,
            socket: dir.join("feeder.sock"),
            client: gf::Client::from_stream(ours, stall).unwrap(),
            window: "_Window_Id_1".into(),
            library: true,
            kind: "card".into(),
        };
        (feeder, peer)
    }

    fn test_spec(dir: &Path) -> FeederSpec {
        FeederSpec {
            program: PathBuf::from("unused"),
            window: "card".into(),
            library: String::new(),
            extra_args: Vec::new(),
            env: Vec::new(),
            run_dir: dir.to_path_buf(),
            idle: Duration::from_secs(60),
        }
    }

    /// A pool holding one idle "feeder": a socket whose other end is `peer`'s.
    fn test_pool(dir: &Path, stall: Duration) -> (Arc<Pool>, std::os::unix::net::UnixStream) {
        let pool = Pool::new(test_spec(dir));
        let (feeder, peer) = test_feeder(dir, stall);
        *pool.slot.lock() = Slot::Idle(Box::new(feeder), Instant::now(), false);
        (pool, peer)
    }

    /// A pool whose idle feeder is gone (its end of the socket is closed, as a feeder whose window the compositor closed is a moment later) and that
    /// makes the feeders in `next`, one for each start, then none. The second is how many it has made.
    fn pool_with_a_gone_feeder(dir: &Path, next: Vec<Feeder>) -> (Arc<Pool>, Arc<std::sync::atomic::AtomicUsize>) {
        let made = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (counted, spare) = (made.clone(), Mutex::new(next));
        let pool = Pool::with_spawn(
            test_spec(dir),
            Box::new(move |_, _| {
                counted.fetch_add(1, Ordering::SeqCst);
                let mut spare = spare.lock();
                if spare.is_empty() { Err("no feeder to start".to_owned()) } else { Ok(spare.remove(0)) }
            }),
        );
        let (gone, peer) = test_feeder(dir, Duration::from_secs(1));
        drop(peer);
        *pool.slot.lock() = Slot::Idle(Box::new(gone), Instant::now(), false);
        (pool, made)
    }

    /// The kinds of the messages a feeder's other end receives, as they come.
    fn listen(mut peer: std::os::unix::net::UnixStream) -> std::sync::mpsc::Receiver<u8> {
        let (kinds, heard) = std::sync::mpsc::channel();
        thread::spawn(move || {
            while let Ok((kind, _)) = gf::read_frame(&mut peer) {
                if kinds.send(kind).is_err() {
                    break;
                }
            }
        });
        heard
    }

    fn quiet_answers() -> Answers {
        Answers { handle: None, stop: Arc::new(AtomicBool::new(false)), waker: Arc::new(Waker::new().unwrap()) }
    }

    /// An `Inner` with one session (id 1) that it holds and whose view is `starting`.
    fn test_inner() -> (Arc<Inner>, Arc<Session>) {
        let inner = Arc::new(Inner {
            deps: Deps { feeder: None, dial: real_dial() },
            pool: None,
            view: Mutex::new((1, View { state: "starting", ..View::default() })),
            session: Mutex::new(None),
            next: AtomicU64::new(2),
            publish: Box::new(|_| {}),
            sizes: Mutex::new(Sizes::default()),
        });
        let session = test_session(1, &inner, SMALL);
        *inner.session.lock() = Some(session.clone());
        (inner, session)
    }

    const SMALL: StreamInfo = StreamInfo { codec: Codec::H264, width: 640, height: 360, fps: 15 };

    #[test]
    fn a_session_whose_answers_cannot_be_read_ends_and_is_not_one_a_new_stream_attaches_to() {
        let dir = tempfile::tempdir().unwrap();
        let (pool, _peer) = test_pool(dir.path(), Duration::from_secs(1));
        let (inner, session) = test_inner();
        write_session_with(&inner, &pool, &session, |_, _| Err("no thread".to_owned()));
        assert!(session.ended());
        assert!(inner.session.lock().is_none(), "a later start must not find this session");
        assert_eq!(inner.view.lock().1.state, "failed");
        assert!(matches!(*pool.slot.lock(), Slot::Empty), "the video path is free for the next stream");
    }

    #[test]
    fn a_feeder_that_stops_reading_ends_the_session_and_frees_the_video_path() {
        let dir = tempfile::tempdir().unwrap();
        let (pool, _peer_never_reads) = test_pool(dir.path(), Duration::from_millis(200));
        let (inner, session) = test_inner();
        session.queue.begin(1);
        session.queue.settle();
        for i in 0..QUEUE_UNITS {
            assert!(matches!(session.queue.give(1, unit(i == 0, i as i64 * 1000, 256 * 1024)), Give::Done));
        }
        let (done, finished) = std::sync::mpsc::channel();
        {
            let (inner, pool, session) = (inner.clone(), pool.clone(), session.clone());
            thread::spawn(move || {
                write_session(&inner, &pool, &session);
                let _ = done.send(());
            });
        }
        assert!(finished.recv_timeout(Duration::from_secs(10)).is_ok(), "the writer is stuck on a feeder that does not read");
        assert!(session.ended());
        assert!(inner.session.lock().is_none());
        assert_eq!(inner.view.lock().1.state, "failed");
        assert!(matches!(*pool.slot.lock(), Slot::Empty), "the stalled feeder is let go, not parked");
        // the slot is free: a lease does not wait for a holder (it tries to start a feeder, which this pool has no program for)
        let asked = Instant::now();
        assert!(pool.lease("card").is_err());
        assert!(asked.elapsed() < FEEDER_WAIT);
    }

    /// Runs the session's writer on its own thread, with answers that say nothing.
    fn run_writer(inner: &Arc<Inner>, pool: &Arc<Pool>, session: &Arc<Session>) -> thread::JoinHandle<()> {
        let (inner, pool, session) = (inner.clone(), pool.clone(), session.clone());
        thread::spawn(move || write_session_with(&inner, &pool, &session, |_, _| Ok(quiet_answers())))
    }

    /// Reads what the feeder is sent until a picture comes: the messages before it.
    fn until_a_picture(heard: &std::sync::mpsc::Receiver<u8>) -> Vec<u8> {
        let mut kinds = Vec::new();
        while !kinds.contains(&gf::VIDEO) {
            kinds.push(heard.recv_timeout(Duration::from_secs(5)).expect("the feeder is sent a picture"));
        }
        kinds
    }

    #[test]
    fn a_feeder_that_was_gone_before_the_first_message_is_replaced_and_the_stream_plays_with_no_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let (fresh, fresh_peer) = test_feeder(dir.path(), Duration::from_secs(1));
        let heard = listen(fresh_peer);
        let (pool, made) = pool_with_a_gone_feeder(dir.path(), vec![fresh]);
        let (inner, session) = test_inner();
        session.queue.begin(1);
        session.queue.settle();
        assert!(matches!(session.queue.give(1, unit(true, 0, 100)), Give::Done));
        let writer = run_writer(&inner, &pool, &session);
        let kinds = until_a_picture(&heard);
        assert!(kinds.contains(&gf::LOAD), "the new feeder is loaded first: {kinds:?}");
        assert_eq!(made.load(Ordering::SeqCst), 1);
        let view = inner.view.lock().1.clone();
        assert_eq!((view.state, view.reason.as_str()), ("starting", ""), "nothing was shown of it");
        session.end(true);
        writer.join().unwrap();
    }

    #[test]
    fn a_feeder_that_breaks_with_a_picture_in_hand_gives_it_to_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let (first, mut first_peer) = test_feeder(dir.path(), Duration::from_secs(1));
        let (second, second_peer) = test_feeder(dir.path(), Duration::from_secs(1));
        let heard = listen(second_peer);
        let (pool, made) = pool_with_a_gone_feeder(dir.path(), vec![second]);
        *pool.slot.lock() = Slot::Idle(Box::new(first), Instant::now(), false);
        let (inner, session) = test_inner();
        session.queue.begin(1);
        session.queue.settle();
        let writer = run_writer(&inner, &pool, &session);
        // the first feeder takes the ping and the load (seven messages), then goes: the picture that follows is the first write that finds it gone
        let (gone, went) = std::sync::mpsc::channel();
        thread::spawn(move || {
            for _ in 0..7 {
                gf::read_frame(&mut first_peer).unwrap();
            }
            drop(first_peer);
            gone.send(()).unwrap();
        });
        went.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(session.queue.give(1, unit(true, 0, 100)), Give::Done));
        let kinds = until_a_picture(&heard);
        assert!(kinds.contains(&gf::LOAD), "{kinds:?}");
        assert_eq!(made.load(Ordering::SeqCst), 1);
        assert_eq!(inner.view.lock().1.state, "starting");
        session.end(true);
        writer.join().unwrap();
    }

    #[test]
    fn a_feeder_that_breaks_again_after_the_replacement_fails_the_session_with_the_cause() {
        let dir = tempfile::tempdir().unwrap();
        let (second, second_peer) = test_feeder(dir.path(), Duration::from_secs(1));
        drop(second_peer);
        let (pool, made) = pool_with_a_gone_feeder(dir.path(), vec![second]);
        let (inner, session) = test_inner();
        write_session_with(&inner, &pool, &session, |_, _| Ok(quiet_answers()));
        assert_eq!(made.load(Ordering::SeqCst), 1, "one replacement, not a loop");
        let view = inner.view.lock().1.clone();
        assert_eq!(view.state, "failed");
        assert!(view.reason.contains("the feeder went") && view.reason.contains("Broken pipe"), "{}", view.reason);
        assert!(session.ended() && inner.session.lock().is_none());
        assert!(matches!(*pool.slot.lock(), Slot::Empty), "the video path is free");
    }

    #[test]
    fn a_write_that_failed_says_which_one_in_the_sends_that_follow() {
        let dir = tempfile::tempdir().unwrap();
        let (mut feeder, peer) = test_feeder(dir.path(), Duration::from_secs(1));
        drop(peer);
        let first = feeder.client.send_ping(1).unwrap_err().to_string();
        let later = feeder.client.send_unload().unwrap_err().to_string();
        assert!(later.starts_with("an earlier write to the feeder failed") && later.contains(&first), "{later}");
    }

    #[test]
    fn a_stream_declared_wrong_is_loaded_for_its_real_size_from_the_next_start() {
        let (inner, _) = test_inner();
        let size = |camera: &str, width: u32, height: u32| {
            let plan = Plan {
                camera: camera.into(),
                mode: Mode::Live,
                url: "rtsp://192.168.1.69:1/abc".into(),
                codec: "h264".into(),
                width,
                height,
                fps: 0,
                started_at: 0,
                rate: 1,
                ndl_app: String::new(),
                texture: false,
                sub: false,
                purpose: String::new(),
                window: String::new(),
                on_end: None,
            };
            inner.expected(&plan).map(|i| (i.width, i.height))
        };
        assert_eq!(size("104", 640, 480), Some((640, 480)), "nothing is known yet: the declaration");
        inner.sizes.lock().learn("104", (640, 480), (480, 640));
        assert_eq!(size("104", 640, 480), Some((480, 640)));
        assert_eq!(size("103", 640, 480), Some((640, 480)), "another camera's");
        assert_eq!(size("104", 1920, 2560), Some((1920, 2560)), "another stream of the camera");
        inner.sizes.lock().learn("104", (640, 480), (640, 480));
        assert_eq!(size("104", 640, 480), Some((640, 480)), "declared right again");
    }
}
