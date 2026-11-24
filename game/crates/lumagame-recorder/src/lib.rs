//! The game recorder's tee. The engine copies each coded video and audio unit it hands to the feeder into a bounded queue; one worker
//! thread files them into a RAM ring (whole GOPs, 116 MB), and a clip or recording is muxed from the ring to fragmented MP4 and streamed
//! to a destination on the LAN (BeastNAS). Nothing is written on the TV. The decode path only ever calls [`Recorder::offer`], which
//! copies the unit into the queue or drops it and counts the drop; it never waits for the worker, the ring or the network.
//!
//! Memory budget, 128 MB in all: the ring ([`RING_CAP`]), the hand-off queue ([`HANDOFF_CAP`]) and one fragment being muxed
//! ([`FRAGMENT_CAP`]). A job reads the ring unit by unit and holds only the fragment's references.
mod bitstream;
mod fmp4;
mod http;
mod ring;

use std::io::{self, BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

pub use bitstream::Codec;
pub use http::HttpDestination;

use ring::{Read, Ring};

pub const TOTAL_CAP: usize = 128 << 20;
pub const HANDOFF_CAP: usize = 4 << 20;
pub const FRAGMENT_CAP: usize = 8 << 20;
pub const RING_CAP: usize = TOTAL_CAP - HANDOFF_CAP - FRAGMENT_CAP;
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

/// Where a finished file goes. `open` starts one file; the writer it returns is told when the file is complete.
pub trait Destination: Send + Sync {
    fn open(&self, name: &str) -> io::Result<Box<dyn Upload>>;
}

pub trait Upload: Write + Send {
    /// Ends the file; an error means the destination did not take it whole.
    fn finish(self: Box<Self>) -> io::Result<()>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A recording began.
    Recording,
    /// A clip or recording reached the destination.
    Saved { name: String, seconds: u32, bytes: u64, recording: bool },
    Failed { reason: String },
}

impl Event {
    /// What the viewer is told of this event: whether it worked and one sentence. A recording starting needs no message.
    pub fn sentence(&self) -> Option<(bool, String)> {
        match self {
            Event::Recording => None,
            Event::Saved { name, seconds, bytes, .. } => Some((true, format!("Saved {name}, {seconds} s, {} MB", bytes.div_ceil(1 << 20)))),
            Event::Failed { reason } => Some((false, reason.clone())),
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
    /// Whether a clip or recording is running; one at a time keeps the fragment budget honest.
    busy: AtomicBool,
    stop_recording: AtomicBool,
    clips: AtomicU64,
    failures: AtomicU64,
    destination: Arc<dyn Destination>,
    on_event: Box<dyn Fn(Event) + Send + Sync>,
    prefix: String,
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
    /// The ring holds no keyframe with a decoder config yet.
    NothingYet,
    NoThread,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::Busy => "a clip is already being saved",
            Refusal::NothingYet => "no picture has been recorded yet",
            Refusal::NoThread => "the clip could not be started",
        })
    }
}

impl Recorder {
    pub fn new(ring_cap: usize, prefix: &str, destination: Arc<dyn Destination>, on_event: impl Fn(Event) + Send + Sync + 'static) -> Self {
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
            stop_recording: AtomicBool::new(false),
            clips: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            destination,
            on_event: Box::new(on_event),
            prefix: prefix.to_owned(),
        });
        let worker = shared.clone();
        thread::Builder::new().name("lumagame-tee".to_owned()).spawn(move || worker_loop(&worker, &rx)).expect("spawn the tee worker");
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
        }
    }

    /// Saves the last `seconds` (at least that much when the ring holds it, starting on a keyframe) as one file; the answer comes as an
    /// [`Event`] from the job's thread.
    pub fn save_last(&self, seconds: u32) -> Result<(), Refusal> {
        let (start, end) = {
            let ring = lock(&self.shared.ring);
            (ring.start_for_last(seconds).ok_or(Refusal::NothingYet)?, Some(ring.next_seq()))
        };
        self.spawn_job(start, end, false)
    }

    /// Starts a recording at the open GOP's keyframe; it runs until [`stop_recording`](Self::stop_recording).
    pub fn start_recording(&self) -> Result<(), Refusal> {
        let start = lock(&self.shared.ring).start_of_open_gop().ok_or(Refusal::NothingYet)?;
        self.shared.stop_recording.store(false, Ordering::Release);
        self.spawn_job(start, None, true)
    }

    pub fn stop_recording(&self) {
        self.shared.stop_recording.store(true, Ordering::Release);
        self.shared.arrived.notify_all();
    }

    pub fn recording(&self) -> bool {
        self.shared.busy.load(Ordering::Acquire) && !self.shared.stop_recording.load(Ordering::Acquire)
    }

    fn spawn_job(&self, start: u64, end: Option<u64>, recording: bool) -> Result<(), Refusal> {
        let s = self.shared.clone();
        if s.busy.swap(true, Ordering::AcqRel) {
            return Err(Refusal::Busy);
        }
        let spawned = thread::Builder::new().name("lumagame-clip".to_owned()).spawn(move || {
            let result = run_job(&s, start, end, recording);
            match result {
                Ok(saved) => {
                    s.clips.fetch_add(1, Ordering::Relaxed);
                    (s.on_event)(saved);
                }
                Err(reason) => {
                    s.failures.fetch_add(1, Ordering::Relaxed);
                    (s.on_event)(Event::Failed { reason });
                }
            }
            s.busy.store(false, Ordering::Release);
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
        self.shared.stop_recording.store(true, Ordering::Release);
        self.shared.arrived.notify_all();
    }
}

fn worker_loop(s: &Arc<Shared>, rx: &Receiver<Msg>) {
    let mut last_config: Option<Arc<Config>> = None;
    let mut seen_drops = 0;
    while !s.closed.load(Ordering::Acquire) {
        let msg = match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(m) => m,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let (track, pts_ns, key, data) = match msg {
            Msg::Unit { track, pts_ns, key, data } => (track, pts_ns, key, data),
            #[cfg(test)]
            Msg::Barrier(done) => {
                let _ = done.send(());
                continue;
            }
        };
        s.queued.fetch_sub(data.len(), Ordering::Relaxed);
        let config = (track == Track::Video && key).then(|| config_for(s, &data, &mut last_config)).flatten();
        let mut ring = lock(&s.ring);
        let drops = s.dropped.load(Ordering::Relaxed);
        if drops != seen_drops {
            seen_drops = drops;
            ring.mark_damaged();
        }
        ring.push(track, pts_ns, key, data, config);
        drop(ring);
        s.arrived.notify_all();
    }
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

/// UTC `YYYYMMDD-HHMMSS` of a Unix time, for file names.
fn stamp(unix_s: u64) -> String {
    let (days, rem) = ((unix_s / 86_400) as i64, unix_s % 86_400);
    // Civil date from days since 1970-01-01 (Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn run_job(s: &Arc<Shared>, start: u64, end: Option<u64>, recording: bool) -> Result<Event, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let name = format!("{}-{}{}.mp4", s.prefix, stamp(now), if recording { "-rec" } else { "" });
    let (first_config, mut seq) = match lock(&s.ring).read(start) {
        Read::Unit(_, Some(config)) => (config, start),
        _ => return Err("the picture to start from was already dropped".to_owned()),
    };
    let upload = s.destination.open(&name).map_err(|e| format!("cannot reach the destination: {e}"))?;
    let mut mux = fmp4::Muxer::new(BufWriter::with_capacity(64 << 10, upload), first_config).map_err(|e| format!("upload: {e}"))?;
    if recording {
        (s.on_event)(Event::Recording);
    }
    let mut end = end;
    let (mut first_pts, mut last_pts) = (None, 0i64);
    loop {
        if end.is_none() && s.stop_recording.load(Ordering::Acquire) {
            end = Some(lock(&s.ring).next_seq());
        }
        if end.is_some_and(|e| seq >= e) {
            break;
        }
        let read = {
            let mut ring = lock(&s.ring);
            let mut read = ring.read(seq);
            if matches!(read, Read::Wait) {
                // Re-checked under the lock, so a stop or a new unit between the checks cannot be missed for long.
                let (guard, _) = s.arrived.wait_timeout(ring, Duration::from_millis(100)).unwrap_or_else(|e| e.into_inner());
                ring = guard;
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
            }
            Read::Wait => {
                if s.closed.load(Ordering::Acquire) {
                    break;
                }
            }
            Read::Evicted => return Err("the upload fell behind the stream and the ring dropped the rest".to_owned()),
        }
    }
    mux.close().map_err(|e| format!("upload: {e}"))?;
    let bytes = mux.bytes;
    let writer = mux.finish().map_err(|e| format!("upload: {e}"))?;
    let upload = writer.into_inner().map_err(|e| format!("upload: {}", e.error()))?;
    upload.finish().map_err(|e| format!("the destination did not take the file: {e}"))?;
    let length = first_pts.map_or(0, |f| ((last_pts - f + 500_000_000) / 1_000_000_000) as u32);
    Ok(Event::Saved { name, seconds: length, bytes, recording })
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

    /// A destination that keeps what it is sent; `open` can be made to fail.
    #[derive(Default)]
    struct Memory {
        files: Mutex<Vec<(String, Vec<u8>)>>,
        refuse: AtomicBool,
    }
    struct MemoryFile {
        name: String,
        bytes: Vec<u8>,
        to: Arc<Memory>,
    }
    impl Write for MemoryFile {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Upload for MemoryFile {
        fn finish(self: Box<Self>) -> io::Result<()> {
            lock(&self.to.files).push((self.name, self.bytes));
            Ok(())
        }
    }
    struct MemoryDestination(Arc<Memory>);
    impl Destination for MemoryDestination {
        fn open(&self, name: &str) -> io::Result<Box<dyn Upload>> {
            if self.0.refuse.load(Ordering::Relaxed) {
                return Err(io::Error::new(io::ErrorKind::ConnectionRefused, "refused"));
            }
            Ok(Box::new(MemoryFile { name: name.to_owned(), bytes: Vec::new(), to: self.0.clone() }))
        }
    }

    fn recorder(cap: usize) -> (Recorder, Arc<Memory>, Receiver<Event>) {
        let memory = Arc::new(Memory::default());
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let r = Recorder::new(cap, "test", Arc::new(MemoryDestination(memory.clone())), move |e| {
            let _ = lock(&tx).send(e);
        });
        let c = h264_config();
        r.set_stream(StreamInfo { codec: Some(Codec::H264), width: c.width, height: c.height, fps: 30, hdr: None, audio: c.audio.clone() });
        (r, memory, rx)
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
        let (r, memory, events) = recorder(RING_CAP);
        feed(&r, 0, 10);
        r.save_last(3).unwrap();
        let Event::Saved { name, seconds, recording: false, .. } = events.recv_timeout(Duration::from_secs(5)).unwrap() else { panic!() };
        assert!(name.starts_with("test-") && name.ends_with(".mp4"));
        let files = lock(&memory.files);
        let (_, file) = &files[0];
        // The newest picture is 9.97 s in; the keyframe at or before 6.97 s is 6.0: four whole seconds of 30 pictures.
        assert_eq!(video_samples(file), 120);
        assert_eq!(seconds, 4);
        assert!(file.windows(4).any(|w| w == b"avcC"));
    }

    #[test]
    fn a_recording_follows_the_stream_until_stopped() {
        let (r, memory, events) = recorder(RING_CAP);
        feed(&r, 0, 2);
        r.start_recording().unwrap();
        assert_eq!(events.recv_timeout(Duration::from_secs(5)).unwrap(), Event::Recording);
        assert_eq!(r.save_last(1), Err(Refusal::Busy));
        feed(&r, 2, 2);
        r.stop_recording();
        let Event::Saved { recording: true, name, .. } = events.recv_timeout(Duration::from_secs(5)).unwrap() else { panic!() };
        assert!(name.ends_with("-rec.mp4"));
        // From the open GOP's keyframe (second 1) to the end: three seconds of pictures.
        assert_eq!(video_samples(&lock(&memory.files)[0].1), 90);
        assert!(!r.recording());
    }

    #[test]
    fn a_saved_clip_is_one_sentence_and_a_recording_start_is_silent() {
        assert_eq!(Event::Recording.sentence(), None);
        assert_eq!(Event::Saved { name: "a.mp4".into(), seconds: 15, bytes: 3 << 20, recording: false }.sentence(), Some((true, "Saved a.mp4, 15 s, 3 MB".into())));
        assert_eq!(Event::Failed { reason: "no".into() }.sentence(), Some((false, "no".into())));
    }

    #[test]
    fn nothing_is_saved_before_a_keyframe_and_an_unreachable_destination_is_reported() {
        let (r, memory, events) = recorder(RING_CAP);
        assert_eq!(r.save_last(5), Err(Refusal::NothingYet));
        r.offer(Track::Video, 0, false, &h264_delta(1));
        r.sync();
        assert_eq!(r.save_last(5), Err(Refusal::NothingYet));
        feed(&r, 0, 2);
        memory.refuse.store(true, Ordering::Relaxed);
        r.save_last(1).unwrap();
        assert!(matches!(events.recv_timeout(Duration::from_secs(5)).unwrap(), Event::Failed { reason } if reason.contains("destination")));
        assert_eq!(r.stats().failures, 1);
    }

    #[test]
    fn a_stalled_worker_makes_offers_drop_and_count_without_blocking_the_caller() {
        let (r, _memory, _events) = recorder(RING_CAP);
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
        let (r, _memory, _events) = recorder(cap);
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

    #[test]
    fn file_names_use_utc_dates() {
        assert_eq!(stamp(0), "19700101-000000");
        assert_eq!(stamp(1_700_000_000), "20231114-221320");
        assert_eq!(stamp(951_782_400 + 86_399), "20000229-235959");
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
            let (r, memory, events) = recorder(RING_CAP);
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
            r.save_last(1).unwrap();
            let Event::Saved { bytes, .. } = events.recv_timeout(Duration::from_secs(10)).unwrap() else { panic!("{name}: not saved") };
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
