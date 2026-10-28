//! A recording's pipeline, on a thread of its own at low priority: pictures from the loop, converted to NV12, through the encoder, into
//! the MP4. The loop never waits for it: a picture goes in through a channel that is never allowed to block, and a pipeline that cannot
//! keep up costs frames of the recording, counted, not frames of the screen.
//!
//! The thread also keeps the recording alive when the screen does not change: after a second with no picture it feeds the encoder the
//! last one again (a skip frame, a few dozen bytes), so the file has a sample at least every second and its timeline never has a hole.
//!
//! It ends the recording by itself when the file reaches its size, when it has run its time, when the disk's free space falls under the
//! floor, or when the encoder fails; every ending leaves a file that plays up to its last whole fragment.

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::encoder::{Encoder, EncoderFactory, EncoderSpec};
use super::mp4::Mp4Writer;
use super::nv12::{nv12_len, rgba_to_nv12};
use crate::output::{Image, monotonic_ns};

/// The pipeline thread's nice level: below the frame loop and the apps.
const NICE: i32 = 10;
/// Pictures that may wait for the pipeline; one more is dropped.
const QUEUE: usize = 3;
/// How often an idle pipeline looks at its stop flag and the clock.
const TICK: Duration = Duration::from_millis(100);

#[derive(Clone, Debug)]
pub struct RecordSpec {
    pub encoder: EncoderSpec,
    /// Where the file is written while it grows, and where it goes when it is whole.
    pub part: PathBuf,
    pub path: PathBuf,
    pub max_bytes: u64,
    pub max_seconds: u64,
    /// The recording ends when the filesystem of `path` has less than this free.
    pub min_free: u64,
    /// The recording's time zero: CLOCK_MONOTONIC ns, the clock `pts_us` of every picture is counted on.
    pub start_ns: u64,
    pub keepalive: Duration,
    /// How often the free space is looked at.
    pub space_check: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    /// Asked for.
    Stopped,
    /// A limit that is not an error: `size`, `time` or `space`.
    Limit(&'static str),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Finished {
    pub path: PathBuf,
    pub bytes: u64,
    pub samples: u64,
    pub seconds: f64,
    pub how: Ended,
    /// What the encoder said about the run when it closed.
    pub encoder: String,
}

#[derive(Default)]
struct Progress {
    ready: Option<Result<(), String>>,
    bytes: u64,
    samples: u64,
    frames_in: u64,
    kept_alive: u64,
    finished: Option<Finished>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub ready: Option<Result<(), String>>,
    pub bytes: u64,
    pub samples: u64,
    pub frames_in: u64,
    pub kept_alive: u64,
    pub dropped: u64,
    pub finished: Option<Finished>,
}

struct Shared {
    progress: Mutex<Progress>,
    dropped: AtomicU64,
    /// Microseconds of the recording at which it was asked to stop (`u64::MAX`: not asked).
    stop_at_us: AtomicU64,
    gone: AtomicBool,
}

enum Msg {
    Frame(Image, u64),
    Wake,
}

pub struct Recorder {
    tx: SyncSender<Msg>,
    free: Receiver<Vec<u8>>,
    shared: Arc<Shared>,
    join: Option<JoinHandle<()>>,
    spec: RecordSpec,
}

impl Recorder {
    /// Starts the pipeline; the encoder opens on its thread (`snapshot().ready` says when, and whether).
    pub fn start(spec: RecordSpec, factory: Arc<dyn EncoderFactory>) -> io::Result<Recorder> {
        let (tx, rx) = sync_channel(QUEUE);
        let (free_tx, free) = sync_channel(QUEUE + 2);
        let shared =
            Arc::new(Shared { progress: Mutex::default(), dropped: AtomicU64::new(0), stop_at_us: AtomicU64::new(u64::MAX), gone: AtomicBool::new(false) });
        let (s, sh) = (spec.clone(), shared.clone());
        let join = std::thread::Builder::new().name("record".into()).spawn(move || {
            run(s, factory, rx, free_tx, &sh);
            sh.gone.store(true, Ordering::Release);
        })?;
        Ok(Recorder { tx, free, shared, join: Some(join), spec })
    }

    /// A picture's bytes the pipeline is done with, to draw the next one into (an empty vector until it has returned one).
    pub fn spare(&self) -> Vec<u8> {
        self.free.try_recv().unwrap_or_default()
    }

    /// Hands a picture over; `false` when the pipeline is behind and the picture is dropped.
    pub fn send(&self, image: Image, pts_us: u64) -> bool {
        match self.tx.try_send(Msg::Frame(image, pts_us)) {
            Ok(()) => true,
            Err(_) => {
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Asks the pipeline to end the recording as of `at_us` of its clock; it finishes the file on its own thread.
    pub fn stop(&self, at_us: u64) {
        self.shared.stop_at_us.store(at_us, Ordering::Release);
        let _ = self.tx.try_send(Msg::Wake);
    }

    pub fn snapshot(&self) -> Snapshot {
        let p = self.shared.progress.lock();
        Snapshot {
            ready: p.ready.clone(),
            bytes: p.bytes,
            samples: p.samples,
            frames_in: p.frames_in,
            kept_alive: p.kept_alive,
            dropped: self.shared.dropped.load(Ordering::Relaxed),
            finished: p.finished.clone(),
        }
    }

    /// Whether the thread has ended (its file is whole).
    pub fn is_gone(&self) -> bool {
        self.shared.gone.load(Ordering::Acquire)
    }

    pub fn spec(&self) -> &RecordSpec {
        &self.spec
    }

    /// Waits for the thread (at exit, so the file is whole).
    pub fn join(&mut self) {
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // A recorder dropped while running is a stop: the file is finished rather than left as a part.
        if !self.is_gone() {
            self.stop(0);
        }
        self.join();
    }
}

fn run(spec: RecordSpec, factory: Arc<dyn EncoderFactory>, rx: Receiver<Msg>, free: SyncSender<Vec<u8>>, shared: &Shared) {
    // SAFETY: setpriority on the calling thread (on Linux a PRIO_PROCESS id of 0 is the thread).
    unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, NICE) };
    let (w, h) = (spec.encoder.width as usize, spec.encoder.height as usize);
    let fail = |how: String| {
        let _ = std::fs::remove_file(&spec.part);
        let mut p = shared.progress.lock();
        p.ready.get_or_insert(Err(how.clone()));
        p.finished = Some(Finished { path: spec.path.clone(), bytes: 0, samples: 0, seconds: 0.0, how: Ended::Failed(how), encoder: String::new() });
    };
    let file = match File::create(&spec.part) {
        Ok(f) => f,
        Err(e) => return fail(format!("{}: {e}", spec.part.display())),
    };
    let mut encoder: Box<dyn Encoder> = match factory.open(&spec.encoder) {
        Ok(e) => e,
        Err(e) => return fail(format!("the encoder did not open: {e}")),
    };
    shared.progress.lock().ready = Some(Ok(()));
    let mut mp4 = Mp4Writer::new(BufWriter::with_capacity(256 << 10, file), spec.encoder.width, spec.encoder.height);
    let mut nv12 = vec![0u8; nv12_len(w, h)];
    let mut have_frame = false;
    let mut last_encode = Instant::now();
    let mut last_space_check = Instant::now();
    let mut first = true;
    let mut seen_pts = 0u64;
    let pts_now = || monotonic_ns().saturating_sub(spec.start_ns) / 1000;

    let how = 'run: loop {
        let stop_at = shared.stop_at_us.load(Ordering::Acquire);
        if stop_at != u64::MAX {
            break 'run Ended::Stopped;
        }
        let pts = match rx.recv_timeout(TICK) {
            Ok(Msg::Frame(img, pts)) => {
                if (img.width as usize, img.height as usize) == (w, h) {
                    rgba_to_nv12(&img.rgba, w, h, &mut nv12);
                    have_frame = true;
                    shared.progress.lock().frames_in += 1;
                    let _ = free.try_send(img.rgba);
                    Some(pts)
                } else {
                    None
                }
            }
            Ok(Msg::Wake) => None,
            Err(RecvTimeoutError::Timeout) => {
                if have_frame && last_encode.elapsed() >= spec.keepalive {
                    shared.progress.lock().kept_alive += 1;
                    Some(pts_now())
                } else {
                    None
                }
            }
            Err(RecvTimeoutError::Disconnected) => break 'run Ended::Stopped,
        };
        if let Some(pts) = pts {
            // The pipeline's clock must not run backwards for the file: a keepalive taken after a late picture is not earlier than it.
            let pts = pts.max(seen_pts);
            seen_pts = pts;
            match encoder.encode(&nv12, pts, first) {
                Ok(unit) => {
                    first = false;
                    last_encode = Instant::now();
                    if let Some(u) = unit
                        && let Err(e) = mp4.push(&u.data, u.pts_us)
                    {
                        break 'run Ended::Failed(format!("writing the file: {e}"));
                    }
                    let mut p = shared.progress.lock();
                    p.bytes = mp4.size_so_far();
                    p.samples = mp4.samples();
                }
                Err(e) => break 'run Ended::Failed(format!("the encoder: {e}")),
            }
        }
        if mp4.size_so_far() >= spec.max_bytes {
            break 'run Ended::Limit("size");
        }
        if seen_pts / 1_000_000 >= spec.max_seconds {
            break 'run Ended::Limit("time");
        }
        if last_space_check.elapsed() >= spec.space_check {
            last_space_check = Instant::now();
            if super::store::free_bytes(spec.path.parent().unwrap_or(&spec.path)) < spec.min_free {
                break 'run Ended::Limit("space");
            }
        }
    };

    let end_us = match shared.stop_at_us.load(Ordering::Acquire) {
        u64::MAX => pts_now().max(seen_pts),
        t => t.max(seen_pts),
    };
    // Pictures still queued are not wanted: the recording ends at the stop.
    while rx.try_recv().is_ok() {}
    let said = encoder.close();
    let samples = shared.progress.lock().samples;
    let seconds = end_us as f64 / 1e6;
    let wrote = mp4.finish(end_us).and_then(|bytes| {
        mp4.into_inner().into_inner().map_err(io::Error::from)?.sync_data()?;
        Ok(bytes)
    });
    let (how, bytes) = match wrote {
        Ok(bytes) => (how, bytes),
        Err(e) => {
            let how = if matches!(how, Ended::Stopped | Ended::Limit(_)) { Ended::Failed(format!("finishing the file: {e}")) } else { how };
            (how, std::fs::metadata(&spec.part).map_or(0, |m| m.len()))
        }
    };
    let mut p = shared.progress.lock();
    if samples == 0 {
        let _ = std::fs::remove_file(&spec.part);
        let how = if matches!(how, Ended::Stopped | Ended::Limit(_)) { Ended::Failed("nothing was recorded".into()) } else { how };
        p.finished = Some(Finished { path: spec.path.clone(), bytes: 0, samples: 0, seconds, how, encoder: said });
        return;
    }
    p.finished = Some(match std::fs::rename(&spec.part, &spec.path) {
        Ok(()) => {
            p.bytes = bytes;
            Finished { path: spec.path.clone(), bytes, samples, seconds, how, encoder: said }
        }
        Err(e) => Finished { path: spec.part.clone(), bytes, samples, seconds, how: Ended::Failed(format!("renaming the finished file: {e}")), encoder: said },
    });
}

#[cfg(test)]
mod tests {
    use super::super::encoder::fake::FakeFactory;
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lumatv-record-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn spec(dir: &std::path::Path) -> RecordSpec {
        RecordSpec {
            encoder: EncoderSpec { width: 32, height: 16, fps: 30, kbps: 500, gop: 4 },
            part: dir.join("rec-test.mp4.part"),
            path: dir.join("rec-test.mp4"),
            max_bytes: 10 << 20,
            max_seconds: 3600,
            min_free: 0,
            start_ns: monotonic_ns(),
            keepalive: Duration::from_millis(100),
            space_check: Duration::from_secs(1),
        }
    }

    fn solid(v: u8) -> Image {
        Image { width: 32, height: 16, rgba: [v, v, v, 255].repeat(32 * 16) }
    }

    fn wait_finished(r: &Recorder) -> Finished {
        let t = Instant::now();
        loop {
            if let Some(f) = r.snapshot().finished {
                return f;
            }
            assert!(t.elapsed() < Duration::from_secs(10), "the recording did not end");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The top-level boxes of a file: `ftyp`, `moov`, then a `moof` and an `mdat` for every fragment.
    fn top_level(file: &[u8]) -> Vec<String> {
        let mut v = Vec::new();
        let mut i = 0;
        while i + 8 <= file.len() {
            let size = u32::from_be_bytes(file[i..i + 4].try_into().unwrap()) as usize;
            v.push(String::from_utf8_lossy(&file[i + 4..i + 8]).into_owned());
            assert!(size >= 8 && i + size <= file.len(), "a box runs past the end of the file");
            i += size;
        }
        v
    }

    #[test]
    fn pictures_go_through_the_encoder_into_a_playable_file_and_the_part_becomes_the_file() {
        let dir = tmp("flow");
        let factory = FakeFactory::new();
        let seen = factory.seen.clone();
        let rec = Recorder::start(spec(&dir), Arc::new(factory)).unwrap();
        for i in 0..10u64 {
            while !rec.send(solid(if i < 5 { 20 } else { 200 }), i * 33_000) {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        std::thread::sleep(Duration::from_millis(60));
        rec.stop(400_000);
        let f = wait_finished(&rec);
        assert_eq!(f.how, Ended::Stopped, "{f:?}");
        assert_eq!(f.path, dir.join("rec-test.mp4"));
        assert!(dir.join("rec-test.mp4").exists() && !dir.join("rec-test.mp4.part").exists());
        assert_eq!(f.samples, 10);
        assert!(f.seconds >= 0.4 && f.seconds < 0.5, "{}", f.seconds);
        let file = std::fs::read(&f.path).unwrap();
        assert_eq!(file.len() as u64, f.bytes);
        let boxes = top_level(&file);
        assert_eq!(&boxes[..2], ["ftyp", "moov"]);
        // A keyframe every 4 frames: three fragments (frames 0-3, 4-7, 8-9), each a moof and an mdat.
        assert_eq!(boxes[2..].iter().filter(|b| *b == "moof").count(), 3, "{boxes:?}");
        let s = seen.lock();
        assert!(s.closed && s.opened == Some(spec(&dir).encoder));
        // The encoder saw what was drawn: luma 20 (BT.709 limited: 16 + 219 * 20/255) for the first five, then 200 for the rest.
        let lumas: Vec<u8> = s.frames.iter().map(|f| f.1).collect();
        assert_eq!(&lumas[..10], &[33, 33, 33, 33, 33, 188, 188, 188, 188, 188]);
        assert_eq!(s.frames.iter().filter(|f| f.2).count(), 3);
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_that_does_not_change_is_kept_alive_with_the_last_one() {
        let dir = tmp("alive");
        let factory = FakeFactory::new();
        let seen = factory.seen.clone();
        let rec = Recorder::start(spec(&dir), Arc::new(factory)).unwrap();
        assert!(rec.send(solid(120), 0));
        std::thread::sleep(Duration::from_millis(1100));
        rec.stop(1_100_000);
        let f = wait_finished(&rec);
        let s = seen.lock();
        assert!(s.frames.len() >= 4, "one picture and about ten keepalives in 1.1 s at 100 ms: {}", s.frames.len());
        assert!(s.frames.iter().all(|f| f.1 == s.frames[0].1), "every one is the picture that was drawn");
        assert!(s.frames.windows(2).all(|w| w[1].0 >= w[0].0), "the clock never goes back");
        drop(s);
        assert_eq!(f.how, Ended::Stopped);
        assert!(rec.snapshot().kept_alive >= 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_file_ends_the_recording_with_a_whole_file() {
        let dir = tmp("size");
        let mut s = spec(&dir);
        s.max_bytes = 4000;
        let mut factory = FakeFactory::new();
        factory.unit_bytes = 900;
        let rec = Recorder::start(s, Arc::new(factory)).unwrap();
        for i in 0..40u64 {
            if rec.is_gone() {
                break;
            }
            rec.send(solid(90), i * 33_000);
            std::thread::sleep(Duration::from_millis(5));
        }
        let f = wait_finished(&rec);
        assert_eq!(f.how, Ended::Limit("size"), "{f:?}");
        assert!(f.bytes >= 4000 && f.bytes < 9000, "{}", f.bytes);
        let boxes = top_level(&std::fs::read(&f.path).unwrap());
        assert_eq!(boxes.last().unwrap(), "mdat", "the last fragment is whole");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_time_limit_and_the_free_space_floor_end_it_too() {
        let dir = tmp("limits");
        let mut s = spec(&dir);
        s.max_seconds = 1;
        let rec = Recorder::start(s, Arc::new(FakeFactory::new())).unwrap();
        rec.send(solid(1), 0);
        rec.send(solid(2), 1_500_000);
        assert_eq!(wait_finished(&rec).how, Ended::Limit("time"));
        let mut s = spec(&dir);
        s.min_free = u64::MAX;
        s.space_check = Duration::from_millis(30);
        s.path = dir.join("rec-space.mp4");
        s.part = dir.join("rec-space.mp4.part");
        let rec = Recorder::start(s, Arc::new(FakeFactory::new())).unwrap();
        rec.send(solid(1), 0);
        assert_eq!(wait_finished(&rec).how, Ended::Limit("space"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_encoder_that_fails_mid_recording_leaves_what_was_recorded_and_says_why() {
        let dir = tmp("fail");
        let mut factory = FakeFactory::new();
        factory.fail_after = Some(6);
        let rec = Recorder::start(spec(&dir), Arc::new(factory)).unwrap();
        for i in 0..12u64 {
            rec.send(solid(60), i * 33_000);
            std::thread::sleep(Duration::from_millis(8));
        }
        let f = wait_finished(&rec);
        match &f.how {
            Ended::Failed(why) => assert!(why.contains("the fake encoder failed"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(f.samples, 6);
        assert!(f.path.exists(), "the six recorded pictures are a file");
        let boxes = top_level(&std::fs::read(&f.path).unwrap());
        assert_eq!(&boxes[..2], ["ftyp", "moov"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_encoder_that_will_not_open_leaves_nothing_behind() {
        let dir = tmp("open");
        let mut factory = FakeFactory::new();
        factory.open_error = Some("no /dev/venc".into());
        let rec = Recorder::start(spec(&dir), Arc::new(factory)).unwrap();
        let f = wait_finished(&rec);
        assert!(matches!(&f.how, Ended::Failed(w) if w.contains("no /dev/venc")), "{f:?}");
        assert!(matches!(rec.snapshot().ready, Some(Err(_))));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "no part, no file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recording_that_never_got_a_picture_is_nothing_and_says_so() {
        let dir = tmp("empty");
        let rec = Recorder::start(spec(&dir), Arc::new(FakeFactory::new())).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        rec.stop(50_000);
        let f = wait_finished(&rec);
        assert!(matches!(&f.how, Ended::Failed(w) if w == "nothing was recorded"), "{f:?}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pipeline_that_is_behind_drops_pictures_and_the_sender_never_waits() {
        let dir = tmp("slow");
        let mut factory = FakeFactory::new();
        factory.delay = Duration::from_millis(60);
        let rec = Recorder::start(spec(&dir), Arc::new(factory)).unwrap();
        let t = Instant::now();
        let accepted = (0..30u64).filter(|i| rec.send(solid(70), i * 1000)).count();
        assert!(t.elapsed() < Duration::from_millis(200), "thirty sends took {:?}", t.elapsed());
        assert!(accepted < 30 && rec.snapshot().dropped == 30 - accepted as u64, "{accepted} accepted, {} dropped", rec.snapshot().dropped);
        rec.stop(100_000);
        let _ = wait_finished(&rec);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dropped_recorder_finishes_its_file() {
        let dir = tmp("drop");
        let s = spec(&dir);
        let path = s.path.clone();
        {
            let rec = Recorder::start(s, Arc::new(FakeFactory::new())).unwrap();
            rec.send(solid(5), 0);
            std::thread::sleep(Duration::from_millis(80));
        }
        assert!(path.exists(), "the file was finished, not left as a part");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recycled_buffers_come_back_for_the_next_picture() {
        let dir = tmp("pool");
        let rec = Recorder::start(spec(&dir), Arc::new(FakeFactory::new())).unwrap();
        assert!(rec.spare().is_empty());
        rec.send(solid(9), 0);
        let t = Instant::now();
        let back = loop {
            let b = rec.spare();
            if !b.is_empty() || t.elapsed() > Duration::from_secs(5) {
                break b;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(back.len(), 32 * 16 * 4, "the picture's own bytes");
        rec.stop(1);
        let _ = wait_finished(&rec);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
