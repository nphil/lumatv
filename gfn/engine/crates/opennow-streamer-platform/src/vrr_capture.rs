//! Diagnostics-only timing capture for one 10 s window of a stream (docs/research/decoded-vrr-4k.md, "upstream timing capture").
//!
//! It answers one question: does each frame's source identity and source timestamp survive to the feeder's hand-over? Nothing here
//! changes a PTS, a release time or a byte on the feeder's wire. Every frame leaves one record in a preallocated ring; the file is
//! written once, by its own thread, after the window ends.
//!
//! The UI arms it (rustui/app/src/game/capture.rs) by writing `vrr-capture.arm` beside the prefs file the streamer already reads when
//! a stream starts. Without that file `Capture::arm` returns `None` and the per-frame cost is one `Option` test.
//!
//! The file's clock is the TV's CLOCK_MONOTONIC. The sender's RTP clock is a separate domain: the raw 32-bit stamps and the declared
//! rate are recorded and unwrapped offline.
//!
//! The file also carries the transport's RTCP sender-report summary under `sender_reports` and its per-picture sender prefix rows under
//! `frame_prefix` (each `null` when none was attached). Both only observe: the first covers the whole transport session up to the write,
//! the second every picture from the first until a little past the window, and nothing in the stream depends on either.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering::{Acquire, Relaxed, Release}};
use std::sync::Mutex;

use serde_json::{Value, json};

pub(crate) const CAPACITY: usize = 2048;
pub(crate) const WINDOW_NS: i64 = 10_000_000_000;
/// After the window, frames still held by the pacer or answered late are given this long to be handed over before the file is written.
pub(crate) const GRACE_NS: i64 = 1_000_000_000;
/// "No record": the frame was outside the window or the ring was full.
pub(crate) const NO_SLOT: u32 = u32::MAX;

pub(crate) const F_INDEX_VALID: u64 = 1;
pub(crate) const F_CONTIGUOUS: u64 = 2;
pub(crate) const F_KEYFRAME: u64 = 4;
pub(crate) const F_RESYNC: u64 = 8;
pub(crate) const F_SENT: u64 = 32;
pub(crate) const F_FAILED: u64 = 64;
pub(crate) const F_HANDED: u64 = 128;
/// A flush message went to the feeder just before the unit.
pub(crate) const F_FLUSHED: u64 = 256;
pub(crate) const F_SMOOTH: u64 = 512;

/// Ten words: 80 bytes, written only by atomics so the pushing thread and the pacing thread never share a lock.
struct Slot([AtomicU64; 10]);

const W_SEQ_EPOCH: usize = 0;
const W_FLAGS_HZ: usize = 1;
const W_INDEX_BYTES: usize = 2;
const W_RTP: usize = 3;
const W_SINK: usize = 4;
const W_RELEASE: usize = 5;
const W_KEY: usize = 6;
const W_DONE: usize = 7;
const W_DEPTHS: usize = 8;
/// `seq + 1`, stored last with Release: a zero slot was never written.
const W_COMMIT: usize = 9;

/// What a record's file header says about the stream; every field is also in the file itself.
pub(crate) struct Meta {
    pub(crate) build_id: String,
    pub(crate) codec: &'static str,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps: u32,
    pub(crate) pacing: &'static str,
}

/// A transport summary, asked for with this file's clock reading at the time of the write.
type Summary = Box<dyn Fn(i64) -> Value + Send + Sync>;

pub(crate) struct Capture {
    slots: Box<[Slot]>,
    next: AtomicU32,
    start_ns: AtomicI64,
    epoch: u32,
    nonce: u64,
    out: PathBuf,
    meta: Meta,
    clock_hz: AtomicU32,
    omitted: AtomicU64,
    evicted: AtomicU64,
    unmatched: AtomicU64,
    resets: AtomicU64,
    sender_reports: Mutex<Option<Summary>>,
    frame_prefixes: Mutex<Option<Summary>>,
}

/// What the UI wrote: the capture's name, where its files go, and the LumaGame build.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Arm {
    pub(crate) nonce: u64,
    pub(crate) dir: PathBuf,
    pub(crate) build_id: String,
}

impl Arm {
    /// Lines: `vrr-capture 1`, the nonce, the output directory, the build id. Anything else is not an arm request.
    pub(crate) fn parse(text: &str) -> Option<Arm> {
        let mut lines = text.lines();
        if lines.next()? != "vrr-capture 1" {
            return None;
        }
        let nonce = lines.next()?.trim().parse().ok()?;
        let dir = PathBuf::from(lines.next()?.trim());
        let build_id = lines.next()?.trim().to_owned();
        (dir.is_absolute() && !build_id.is_empty() && build_id.len() <= 64).then_some(Arm { nonce, dir, build_id })
    }
}

impl Capture {
    /// Off the hot path, at stream start: the armed capture for this stream, or `None`. A nonce that already has its file is not
    /// captured again, so a restart of the streamer does not repeat it.
    pub(crate) fn arm(prefs: Option<&Path>, epoch: u32, mut meta: Meta) -> Option<Capture> {
        let arm = Arm::parse(&fs::read_to_string(prefs?.parent()?.join("vrr-capture.arm")).ok()?)?;
        let out = arm.dir.join(format!("{}-streamer.json", arm.nonce));
        if out.exists() {
            return None;
        }
        meta.build_id = arm.build_id;
        Some(Capture::new(arm.nonce, out, epoch, meta))
    }

    pub(crate) fn new(nonce: u64, out: PathBuf, epoch: u32, meta: Meta) -> Capture {
        let slots = (0..CAPACITY).map(|_| Slot(std::array::from_fn(|_| AtomicU64::new(0)))).collect();
        Capture {
            slots, next: AtomicU32::new(0), start_ns: AtomicI64::new(0), epoch, nonce, out, meta,
            clock_hz: AtomicU32::new(0), omitted: AtomicU64::new(0), evicted: AtomicU64::new(0), unmatched: AtomicU64::new(0),
            resets: AtomicU64::new(0), sender_reports: Mutex::new(None), frame_prefixes: Mutex::new(None),
        }
    }

    /// A frame entered the sink. Returns its slot, or `NO_SLOT` (window over, ring full: counted, never waited for).
    pub(crate) fn enter(&self, now_ns: i64, frame: &crate::EncodedFrame, held: usize, resync: bool, release_ns: i64, smooth: bool) -> u32 {
        let start = self.start_ns.load(Relaxed);
        if start == 0 {
            self.start_ns.store(now_ns, Relaxed);
        } else if now_ns - start >= WINDOW_NS {
            return NO_SLOT;
        }
        let seq = self.next.fetch_add(1, Relaxed);
        if seq as usize >= CAPACITY {
            self.omitted.fetch_add(1, Relaxed);
            return NO_SLOT;
        }
        if frame.clock_rate_hz != 0 {
            self.clock_hz.store(frame.clock_rate_hz, Relaxed);
        }
        if resync {
            self.resets.fetch_add(1, Relaxed);
        }
        let flags = (u64::from(frame.frame_index.is_some()) * F_INDEX_VALID)
            | (u64::from(frame.contiguous) * F_CONTIGUOUS)
            | (u64::from(frame.keyframe) * F_KEYFRAME)
            | (u64::from(resync) * F_RESYNC)
            | (u64::from(smooth) * F_SMOOTH);
        let w = &self.slots[seq as usize].0;
        w[W_SEQ_EPOCH].store(u64::from(seq) | u64::from(self.epoch) << 32, Relaxed);
        w[W_FLAGS_HZ].store(flags | u64::from(frame.clock_rate_hz) << 32, Relaxed);
        w[W_INDEX_BYTES].store(u64::from(frame.frame_index.unwrap_or(0)) | (frame.data.len() as u64) << 32, Relaxed);
        w[W_RTP].store(frame.timestamp, Relaxed);
        w[W_SINK].store(now_ns as u64, Relaxed);
        w[W_RELEASE].store(release_ns as u64, Relaxed);
        w[W_DEPTHS].store(held as u64, Relaxed);
        w[W_COMMIT].store(u64::from(seq) + 1, Release);
        seq
    }

    /// The unit went to the feeder (or failed to) with `pts_ns` as its wire key.
    pub(crate) fn handed(&self, slot: u32, pts_ns: i64, done_ns: i64, sent: bool, flushed: bool, in_flight: usize) {
        let Some(slot) = self.slots.get(slot as usize) else { return };
        let w = &slot.0;
        w[W_KEY].store(pts_ns as u64, Relaxed);
        w[W_DONE].store(done_ns as u64, Relaxed);
        let flags = F_HANDED | if sent { F_SENT } else { F_FAILED } | (u64::from(flushed) * F_FLUSHED);
        w[W_FLAGS_HZ].fetch_or(flags, Relaxed);
        w[W_DEPTHS].fetch_or((in_flight as u64) << 32, Relaxed);
    }

    /// An in-flight entry was discarded before the feeder answered for it.
    pub(crate) fn evicted(&self, n: u64) {
        self.evicted.fetch_add(n, Relaxed);
    }

    /// The feeder answered for a unit that has no in-flight entry.
    pub(crate) fn unmatched(&self) {
        self.unmatched.fetch_add(1, Relaxed);
    }

    /// The transport's sender-report summary, written into the file under `sender_reports`. A later source replaces an earlier one.
    pub(crate) fn attach_sender_reports(&self, source: Summary) {
        *self.sender_reports.lock().unwrap_or_else(|e| e.into_inner()) = Some(source);
    }

    /// The transport's per-picture sender prefix rows, written into the file under `frame_prefix`. A later source replaces an earlier one.
    pub(crate) fn attach_frame_prefixes(&self, source: Summary) {
        *self.frame_prefixes.lock().unwrap_or_else(|e| e.into_inner()) = Some(source);
    }

    pub(crate) fn start_ns(&self) -> i64 {
        self.start_ns.load(Relaxed)
    }

    /// The window and the grace after it have passed.
    pub(crate) fn finished(&self, now_ns: i64) -> bool {
        let start = self.start_ns();
        start != 0 && now_ns - start >= WINDOW_NS + GRACE_NS
    }

    /// The whole file. `ended` names why the stream stopped before the window was over, if it did. `now_ns` is this file's clock now.
    pub(crate) fn to_json(&self, ended: Option<&'static str>, now_ns: i64) -> Value {
        let taken = (self.next.load(Relaxed) as usize).min(CAPACITY);
        let mut records = Vec::with_capacity(taken);
        let mut uncommitted = 0u64;
        for (i, slot) in self.slots[..taken].iter().enumerate() {
            let w = &slot.0;
            if w[W_COMMIT].load(Acquire) != i as u64 + 1 {
                uncommitted += 1;
                continue;
            }
            let ns = |word: usize| w[word].load(Relaxed) as i64;
            let flags_hz = w[W_FLAGS_HZ].load(Relaxed);
            let index_bytes = w[W_INDEX_BYTES].load(Relaxed);
            let depths = w[W_DEPTHS].load(Relaxed);
            let seq_epoch = w[W_SEQ_EPOCH].load(Relaxed);
            let flags = flags_hz & 0xffff_ffff;
            let handed = flags & F_HANDED != 0;
            records.push(json!([
                seq_epoch >> 32, seq_epoch & 0xffff_ffff, flags,
                (flags & F_INDEX_VALID != 0).then_some(index_bytes & 0xffff_ffff),
                w[W_RTP].load(Relaxed), flags_hz >> 32, index_bytes >> 32,
                ns(W_SINK), ns(W_RELEASE), handed.then(|| ns(W_KEY)), handed.then(|| ns(W_DONE)),
                depths & 0xffff_ffff, depths >> 32,
            ]));
        }
        let omitted = self.omitted.load(Relaxed);
        let mut incomplete = Vec::new();
        if omitted > 0 { incomplete.push("ring full: records omitted"); }
        if uncommitted > 0 { incomplete.push("records still being written"); }
        if self.start_ns() == 0 { incomplete.push("no video frame reached the sink"); }
        if let Some(why) = ended { incomplete.push(why); }
        let sender_reports = self.sender_reports.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(Value::Null, |source| source(now_ns));
        let frame_prefix = self.frame_prefixes.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(Value::Null, |source| source(now_ns));
        json!({
            "format": "lumagame-vrr-pts-capture", "version": 1, "producer": "streamer", "clock": "CLOCK_MONOTONIC",
            "nonce": self.nonce, "complete": incomplete.is_empty(), "incomplete": incomplete,
            "header": {
                "build_id": self.meta.build_id, "codec": self.meta.codec, "width": self.meta.width, "height": self.meta.height,
                "fps": self.meta.fps, "rtp_clock_hz": self.clock_hz.load(Relaxed), "frame_pacing": self.meta.pacing,
                "window_ns": WINDOW_NS, "start_ns": self.start_ns(), "capacity": CAPACITY, "epoch": self.epoch,
                "counts": {
                    "omitted": omitted, "evicted": self.evicted.load(Relaxed), "unmatched": self.unmatched.load(Relaxed),
                    "resets": self.resets.load(Relaxed), "stored": records.len(),
                },
                "definitions": {
                    "omitted": "records not stored because the ring was full",
                    "evicted": "in-flight entries discarded before the feeder answered for them",
                    "unmatched": "ACCEPT or DROP answers with no in-flight entry",
                    "resets": "frames that restarted the pacer's grid (keyframe or a gap in the sequence)",
                },
                "unavailable": ["decoder completion", "selected-frame id", "release", "optical times"],
            },
            "fields": [
                "epoch", "seq", "flags", "frame_index", "rtp_timestamp", "rtp_clock_hz", "bytes", "sink_entry_ns", "release_ns",
                "wire_key_ns", "handover_done_ns", "held_at_entry", "in_flight_at_send",
            ],
            "flags": {
                "index_valid": F_INDEX_VALID, "contiguous": F_CONTIGUOUS, "keyframe": F_KEYFRAME, "resync": F_RESYNC,
                "sent": F_SENT, "failed": F_FAILED, "handed": F_HANDED, "flushed": F_FLUSHED, "smooth": F_SMOOTH,
            },
            "sender_reports": sender_reports, "frame_prefix": frame_prefix, "records": records,
        })
    }

    /// Writes `<nonce>-streamer.json` once, atomically, readable by its owner only. Never overwrites.
    pub(crate) fn write(&self, ended: Option<&'static str>, now_ns: i64) -> std::io::Result<()> {
        if let Some(dir) = self.out.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.out.with_extension("json.tmp");
        {
            let mut file = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
            serde_json::to_writer(&mut file, &self.to_json(ended, now_ns))?;
            file.write_all(b"\n")?;
            file.sync_all()?;
        }
        fs::rename(tmp, &self.out)
    }

    pub(crate) fn out(&self) -> &Path {
        &self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EncodedFrame, MediaCodec};
    use std::sync::Arc;

    fn meta() -> Meta {
        Meta { build_id: "b".into(), codec: "H265", width: 3840, height: 2160, fps: 120, pacing: "Smooth" }
    }

    fn frame(index: Option<u32>, ts: u64) -> EncodedFrame {
        EncodedFrame {
            mid: "0".into(), codec: MediaCodec::H265, data: Arc::from(vec![0u8; 4]), frame_index: index, timestamp: ts,
            clock_rate_hz: 90_000, keyframe: false, contiguous: true, ssrc: None,
        }
    }

    #[test]
    fn a_full_ring_omits_counts_and_marks_the_capture_incomplete() {
        let c = Capture::new(1, PathBuf::from("/nonexistent/1-streamer.json"), 1, meta());
        let mut slots = 0;
        for i in 0..CAPACITY + 5 {
            if c.enter(1_000 + i as i64, &frame(Some(i as u32), i as u64 * 750), 0, false, 1_000, true) != NO_SLOT {
                slots += 1;
            }
        }
        assert_eq!(slots, CAPACITY);
        let v = c.to_json(None, 0);
        assert_eq!(v["header"]["counts"]["omitted"], 5);
        assert_eq!(v["header"]["counts"]["stored"], CAPACITY);
        assert_eq!(v["complete"], false);
        assert!(v["incomplete"].as_array().unwrap().iter().any(|r| r.as_str().unwrap().contains("ring full")));
    }

    #[test]
    fn frames_after_the_window_are_not_recorded_and_do_not_make_it_incomplete() {
        let c = Capture::new(1, PathBuf::from("/nonexistent/x.json"), 1, meta());
        let t0 = 5_000_000_000;
        assert_ne!(c.enter(t0, &frame(Some(1), 1), 0, false, t0, true), NO_SLOT);
        assert_eq!(c.enter(t0 + WINDOW_NS, &frame(Some(2), 2), 0, false, t0, true), NO_SLOT);
        assert!(!c.finished(t0 + WINDOW_NS));
        assert!(c.finished(t0 + WINDOW_NS + GRACE_NS));
        let v = c.to_json(None, 0);
        assert_eq!(v["complete"], true);
        assert_eq!(v["header"]["counts"]["omitted"], 0);
    }

    #[test]
    fn a_record_keeps_source_identity_the_wire_key_and_the_header_has_the_stream() {
        let c = Capture::new(7, PathBuf::from("/nonexistent/x.json"), 3, meta());
        let slot = c.enter(10, &frame(None, 0xffff_fff0), 2, true, 20, false);
        c.handed(slot, 25, 30, true, false, 4);
        c.evicted(2);
        c.unmatched();
        let v = c.to_json(Some("the stream stopped"), 0);
        let r = &v["records"][0];
        assert_eq!(r[0], 3);
        assert!(r[3].is_null(), "an unset index is absent, not zero");
        assert_eq!((r[4].as_u64(), r[9].as_i64(), r[10].as_i64(), r[11].as_u64(), r[12].as_u64()), (Some(0xffff_fff0), Some(25), Some(30), Some(2), Some(4)));
        let h = &v["header"];
        assert_eq!((h["build_id"].as_str(), h["codec"].as_str(), h["width"].as_u64(), h["fps"].as_u64(), h["rtp_clock_hz"].as_u64(), h["frame_pacing"].as_str()),
            (Some("b"), Some("H265"), Some(3840), Some(120), Some(90_000), Some("Smooth")));
        assert_eq!((h["counts"]["evicted"].as_u64(), h["counts"]["unmatched"].as_u64(), h["counts"]["resets"].as_u64()), (Some(2), Some(1), Some(1)));
        assert_eq!(v["complete"], false);
    }

    #[test]
    fn arm_requests_are_strict_and_a_captured_nonce_is_not_repeated() {
        assert_eq!(Arm::parse("vrr-capture 1\n42\n/data/x\nabc123\n"), Some(Arm { nonce: 42, dir: "/data/x".into(), build_id: "abc123".into() }));
        assert_eq!(Arm::parse("vrr-capture 2\n42\n/data/x\nabc\n"), None);
        assert_eq!(Arm::parse("vrr-capture 1\nx\n/data/x\nabc\n"), None);
        assert_eq!(Arm::parse("vrr-capture 1\n42\nrelative\nabc\n"), None);
        let dir = std::env::temp_dir().join(format!("vrr-capture-arm-{}", std::process::id()));
        fs::create_dir_all(dir.join("out")).unwrap();
        let prefs = dir.join("runtime-prefs.json");
        fs::write(dir.join("vrr-capture.arm"), format!("vrr-capture 1\n9\n{}\nbuild-9\n", dir.join("out").display())).unwrap();
        let c = Capture::arm(Some(&prefs), 1, meta()).expect("armed");
        assert_eq!(c.out(), dir.join("out/9-streamer.json"));
        c.write(None, 0).unwrap();
        assert!(Capture::arm(Some(&prefs), 2, meta()).is_none());
        let written: Value = serde_json::from_slice(&fs::read(dir.join("out/9-streamer.json")).unwrap()).unwrap();
        assert_eq!(written["header"]["build_id"], "build-9");
        assert!(Capture::arm(None, 1, meta()).is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_sender_report_summary_is_asked_for_when_the_file_is_written() {
        let c = Capture::new(1, PathBuf::from("/nonexistent/x.json"), 1, meta());
        assert!(c.to_json(None, 5)["sender_reports"].is_null(), "nothing attached");
        c.attach_sender_reports(Box::new(|now_ns| json!({ "asked_at_ns": now_ns })));
        assert_eq!(c.to_json(None, 42)["sender_reports"]["asked_at_ns"], 42);
    }

    #[test]
    fn the_frame_prefix_rows_are_asked_for_when_the_file_is_written() {
        let c = Capture::new(1, PathBuf::from("/nonexistent/x.json"), 1, meta());
        assert!(c.to_json(None, 5)["frame_prefix"].is_null(), "nothing attached");
        c.attach_frame_prefixes(Box::new(|now_ns| json!({ "asked_at_ns": now_ns })));
        let v = c.to_json(None, 42);
        assert_eq!(v["frame_prefix"]["asked_at_ns"], 42);
        assert!(v["sender_reports"].is_null(), "the two sources are separate");
    }
}
