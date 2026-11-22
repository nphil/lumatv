//! The sender's per-picture prefix on NVST video, recorded for the timing capture and never acted
//! on.
//!
//! The first packet of every picture starts with a prefix that the receiver strips before the
//! codec data. The one real stream read so far sent `81 08 01 02`, a 32-bit time, an extension
//! length of 8 and one record, tag `0x202` (the access-unit length). No `0x200` or `0x201` record
//! has been seen on the TV yet.
//!
//! The reading is a port of OpenNOW's `assembled_video_payload` and `SenderFrameTiming`:
//! OpenCloudGaming/OpenNOW `a9de957a5902c54186e9d4b44b1562f0b6beb951` (main, 2026-11-11),
//! `native/opennow-streamer/crates/opennow-streamer-transport/src/nvst.rs` lines 3311-3399 and
//! `.../src/lib.rs`. The pinned `a49b124` has no such parsing. Only the reading is ported: the
//! version and size checks, the capture time at bytes 4..8, the extension's `0x200` sender-paced
//! flag, `0x201` presentation time and `0x202` access-unit length, the same limits, the same
//! malformed cases and the last record winning when a tag repeats. What upstream does with the
//! values (presentation estimator, pacing, access-unit trimming) is not ported, and nothing here
//! accepts, drops or alters a packet.
//!
//! Upstream names the fields and documents none of their meanings, so the meanings are inferences.
//! Each row also records how much the pinned receiver strips next to how much the prefix declares,
//! so a disagreement between the two readings shows.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::nvst_sender_report::capture_clock;

/// Pictures kept. The platform capture's ring is this size (`vrr_capture::CAPACITY`); the crates do
/// not depend on each other, so the figure is repeated. Past about 170 pictures per second the
/// window overflows, and the overflow is counted.
const CAPACITY: usize = 2048;
/// How long pictures are recorded, from the first. The platform capture's 10 s window
/// (`vrr_capture::WINDOW_NS`) opens at its first sink entry, a little after the first picture
/// arrives, so the recording runs 2 s longer to cover it.
const WINDOW_NS: u64 = 12_000_000_000;
const WINDOW: Duration = Duration::from_nanos(WINDOW_NS);
/// An extension longer than this is malformed (upstream's limit).
const MAX_EXTENSION_BYTES: usize = 1024;
/// Records with another tag kept per picture; the rest are only counted.
const MAX_OTHER_RECORDS: usize = 4;
/// The shortest fixed part: version, size, two flag bytes and the 32-bit capture time.
const MIN_HEADER_BYTES: usize = 8;

const TAG_SENDER_PACED: u16 = 0x200;
const TAG_PRESENTATION_TIME: u16 = 0x201;
const TAG_ACCESS_UNIT_LENGTH: u16 = 0x202;

/// The columns of a row in the JSON, in order.
const FIELDS: [&str; 15] = [
    "frame_index",
    "rtp_timestamp",
    "arrival_ns",
    "status",
    "byte0",
    "header_size",
    "byte2",
    "byte3",
    "capture_time_us",
    "sender_paced",
    "presentation_time_us",
    "access_unit_length",
    "prefix_bytes",
    "stripped_bytes",
    "other_tags",
];

/// What the first packet of a picture turned out to be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Status {
    /// A sender prefix was read.
    Read,
    /// The packet does not start with one: GameStream's `GS` header, bare Annex B, nothing.
    #[default]
    Absent,
    /// It starts like one but its sizes do not fit the packet, or no codec data follows it.
    Malformed,
}

impl Status {
    const fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Absent => "absent",
            Self::Malformed => "malformed",
        }
    }
}

/// An extension record with a tag this module does not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OtherRecord {
    pub(crate) tag: u16,
    /// The value's length as written.
    pub(crate) len: u16,
    /// The first 8 bytes of the value, little endian.
    pub(crate) head: u64,
}

/// One picture's prefix as written. A field the packet does not carry is `None`, never zero.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FramePrefix {
    pub(crate) status: Status,
    pub(crate) byte0: Option<u8>,
    pub(crate) header_size: Option<u8>,
    pub(crate) byte2: Option<u8>,
    pub(crate) byte3: Option<u8>,
    pub(crate) capture_time_us: Option<u32>,
    pub(crate) sender_paced: Option<u8>,
    pub(crate) presentation_time_us: Option<u64>,
    pub(crate) access_unit_length: Option<u32>,
    /// Where the codec data starts by the prefix's own sizes.
    pub(crate) prefix_bytes: Option<u32>,
    pub(crate) others: Vec<OtherRecord>,
    /// Every record with another tag, kept or not.
    pub(crate) others_total: u16,
}

impl FramePrefix {
    /// Reads the prefix at the start of a picture's first packet.
    pub(crate) fn read(payload: &[u8]) -> Self {
        let byte0 = payload.first().copied();
        let &[version, header_size, ..] = payload else {
            return Self {
                byte0,
                ..Self::default()
            };
        };
        if version & 0x7f != 1 || header_size == 0 {
            return Self {
                byte0,
                ..Self::default()
            };
        }
        Self::fields(payload, version, header_size).unwrap_or(Self {
            status: Status::Malformed,
            byte0,
            header_size: Some(header_size),
            ..Self::default()
        })
    }

    /// `None` wherever OpenNOW rejects the prefix.
    fn fields(payload: &[u8], version: u8, header_size: u8) -> Option<Self> {
        let base = usize::from(header_size);
        if base < MIN_HEADER_BYTES || payload.len() < base {
            return None;
        }
        let mut paced = None;
        let mut presentation = None;
        let mut access_unit = None;
        let mut others = Vec::new();
        let mut others_total = 0_u16;
        let mut end = base;
        if version & 0x80 != 0 {
            let length = u32::from_le_bytes(*payload.get(base..)?.first_chunk::<4>()?);
            let length = usize::try_from(length)
                .ok()
                .filter(|&length| length <= MAX_EXTENSION_BYTES)?;
            end = base + 4 + length;
            let mut records = payload.get(base + 4..end)?;
            while !records.is_empty() {
                let (header, rest) = records.split_first_chunk::<4>()?;
                let tag = u16::from_le_bytes([header[0], header[1]]);
                let len = u16::from_le_bytes([header[2], header[3]]);
                let (value, rest) = rest.split_at_checked(usize::from(len))?;
                records = rest;
                match tag {
                    TAG_SENDER_PACED => paced = Some(*value.first()?),
                    TAG_PRESENTATION_TIME => {
                        presentation = Some(u64::from_le_bytes(*value.first_chunk::<8>()?));
                    }
                    TAG_ACCESS_UNIT_LENGTH => {
                        access_unit = Some(u32::from_le_bytes(*value.first_chunk::<4>()?));
                    }
                    _ => {
                        others_total += 1;
                        if others.len() < MAX_OTHER_RECORDS {
                            others.push(OtherRecord {
                                tag,
                                len,
                                head: head(value),
                            });
                        }
                    }
                }
            }
        }
        if end == payload.len() {
            return None;
        }
        Some(Self {
            status: Status::Read,
            byte0: Some(version),
            header_size: Some(header_size),
            byte2: Some(payload[2]),
            byte3: Some(payload[3]),
            capture_time_us: Some(u32::from_le_bytes(payload[4..8].try_into().ok()?)),
            sender_paced: paced,
            presentation_time_us: presentation,
            access_unit_length: access_unit,
            prefix_bytes: u32::try_from(end).ok(),
            others,
            others_total,
        })
    }

    fn other_tags(&self) -> Option<Value> {
        (self.others_total > 0).then(|| {
            let kept: Vec<Value> = self
                .others
                .iter()
                .map(|record| json!([record.tag, record.len, record.head]))
                .collect();
            json!({ "total": self.others_total, "records": kept })
        })
    }
}

/// The first 8 bytes of a record's value as a little-endian number.
fn head(value: &[u8]) -> u64 {
    let mut bytes = [0_u8; 8];
    let kept = value.len().min(bytes.len());
    bytes[..kept].copy_from_slice(&value[..kept]);
    u64::from_le_bytes(bytes)
}

#[derive(Debug)]
struct Row {
    frame_index: u32,
    rtp: u32,
    arrival: Instant,
    /// How much the receiver strips from the packet; `None` when it found no codec data.
    stripped: Option<u32>,
    prefix: FramePrefix,
}

impl Row {
    fn to_json(&self, at: &impl Fn(Instant) -> i64) -> Value {
        let prefix = &self.prefix;
        json!([
            self.frame_index,
            self.rtp,
            at(self.arrival),
            prefix.status.name(),
            prefix.byte0,
            prefix.header_size,
            prefix.byte2,
            prefix.byte3,
            prefix.capture_time_us,
            prefix.sender_paced,
            prefix.presentation_time_us,
            prefix.access_unit_length,
            prefix.prefix_bytes,
            self.stripped,
            prefix.other_tags(),
        ])
    }
}

#[derive(Debug, Default)]
struct Recorded {
    /// When the first picture was seen; the window runs from here.
    first: Option<Instant>,
    rows: Vec<Row>,
    seen: u64,
    omitted: u64,
    read: u64,
    absent: u64,
    malformed: u64,
}

/// A bounded record of every picture's prefix during the capture window. Until it is armed it
/// costs one relaxed load per picture.
#[derive(Debug, Default)]
pub(crate) struct FramePrefixLog {
    armed: AtomicBool,
    recorded: Mutex<Recorded>,
}

impl FramePrefixLog {
    fn lock(&self) -> MutexGuard<'_, Recorded> {
        self.recorded.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts recording. The ring is allocated here, off the frame path.
    pub(crate) fn arm(&self) {
        self.lock().rows.reserve_exact(CAPACITY);
        self.armed.store(true, Ordering::Release);
    }

    /// The first packet of a picture: its frame index, its RTP timestamp, its bytes as they came,
    /// and how much of them the receiver strips (`None` when it found no codec data).
    pub(crate) fn observe(
        &self,
        frame_index: u32,
        rtp: u32,
        payload: &[u8],
        stripped: Option<usize>,
    ) {
        if self.armed.load(Ordering::Relaxed) {
            self.record(Instant::now(), frame_index, rtp, payload, stripped);
        }
    }

    fn record(
        &self,
        now: Instant,
        frame_index: u32,
        rtp: u32,
        payload: &[u8],
        stripped: Option<usize>,
    ) {
        let prefix = FramePrefix::read(payload);
        let mut recorded = self.lock();
        let first = *recorded.first.get_or_insert(now);
        if now.saturating_duration_since(first) >= WINDOW {
            self.armed.store(false, Ordering::Relaxed);
            return;
        }
        recorded.seen += 1;
        match prefix.status {
            Status::Read => recorded.read += 1,
            Status::Absent => recorded.absent += 1,
            Status::Malformed => recorded.malformed += 1,
        }
        if recorded.rows.len() == CAPACITY {
            recorded.omitted += 1;
            return;
        }
        recorded.rows.push(Row {
            frame_index,
            rtp,
            arrival: now,
            stripped: stripped.and_then(|bytes| u32::try_from(bytes).ok()),
            prefix,
        });
    }

    /// The rows for the timing capture. Arrivals are converted to the caller's clock: `now_ns` is
    /// that clock's reading at `now`.
    pub(crate) fn to_json(&self, now: Instant, now_ns: i64) -> Value {
        let at = capture_clock(now, now_ns);
        let recorded = self.lock();
        let rows: Vec<Value> = recorded.rows.iter().map(|row| row.to_json(&at)).collect();
        json!({
            "fields": FIELDS,
            "rows": rows,
            "counts": {
                "seen": recorded.seen,
                "stored": recorded.rows.len(),
                "omitted": recorded.omitted,
                "read": recorded.read,
                "absent": recorded.absent,
                "malformed": recorded.malformed,
            },
            "window_ns": WINDOW_NS,
            "capacity": CAPACITY,
            "definitions": {
                "scope": "the first packet of each picture the receiver assembled, from the first \
                    one seen until the window has passed (the capture's own 10 s window opens at \
                    its first sink entry, a little later); join to the capture's records by \
                    frame_index and rtp_timestamp",
                "source": "the reading follows OpenNOW a9de957a5902 (main, 2026-11-11), transport \
                    nvst.rs assembled_video_payload; upstream names the fields and documents none \
                    of their meanings, so every meaning below is an inference, not a measurement",
                "arrival_ns": "when the picture's first packet reached the receiver, on the \
                    capture's CLOCK_MONOTONIC",
                "status": "read: a sender prefix was read; absent: the packet does not start with \
                    one (every field null); malformed: it starts like one but its sizes do not fit \
                    the packet, or no codec data follows it (only byte0 and header_size are kept)",
                "byte0": "the first byte as written: 0x01, or 0x81 when an extension follows",
                "header_size": "the second byte: the length of the fixed part, 8 or more",
                "byte2": "the third byte as written; not interpreted",
                "byte3": "the fourth byte as written; its low nibble is read as the frame type",
                "capture_time_us": "bytes 4..8 as a little-endian u32, read as the server's \
                    capture time in microseconds on a clock of its own; it wraps every 4295 s, so \
                    only differences mean anything",
                "sender_paced": "extension record 0x200, one byte as written (upstream reads any \
                    non-zero value as paced); null when the record is absent; not seen on the TV \
                    so far",
                "presentation_time_us": "extension record 0x201, a little-endian u64; null when \
                    absent; not seen on the TV so far",
                "access_unit_length": "extension record 0x202, a little-endian u32, the picture's \
                    size hint; null when absent",
                "prefix_bytes": "where the codec data starts by the prefix's own sizes: \
                    header_size, plus 4 and the extension's length when byte0 has the high bit",
                "stripped_bytes": "how much the pinned receiver strips before the codec data, \
                    null when it found none; a value different from prefix_bytes means the two \
                    readings of the prefix disagree",
                "other_tags": "extension records with another tag as [tag, length, first 8 bytes \
                    little endian]; total counts them all, at most 4 are kept; null when there \
                    are none",
                "null": "absent or unknown, never zero; a zero in a column is a value the sender \
                    wrote",
                "proves": "none of these fields shows that the game's pixels changed, that Cloud \
                    G-SYNC was accepted (that is the finalized answer in the NVST cloud-gsync log \
                    line) or that the panel scanned a picture on its own clock",
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An extension record: tag, length, value.
    fn record(tag: u16, value: &[u8]) -> Vec<u8> {
        let mut bytes = tag.to_le_bytes().to_vec();
        bytes.extend(u16::try_from(value.len()).unwrap().to_le_bytes());
        bytes.extend(value);
        bytes
    }

    /// A packet with an `0x81 08 01 02` prefix, a capture time, the given records and codec data.
    fn extended(capture: u32, records: &[Vec<u8>]) -> Vec<u8> {
        let body = records.concat();
        let mut payload = vec![0x81, 8, 1, 2];
        payload.extend(capture.to_le_bytes());
        payload.extend(u32::try_from(body.len()).unwrap().to_le_bytes());
        payload.extend(body);
        payload.extend([0, 0, 0, 1, 0x40]);
        payload
    }

    /// The prefix the TV's stream sends, as logged on the first picture.
    fn tv_picture() -> Vec<u8> {
        let mut payload = vec![0x81, 8, 1, 2, 0x44, 0x33, 0x22, 0x11, 8, 0, 0, 0];
        payload.extend([2, 2, 4, 0, 0x68, 0x1e, 0, 0]);
        payload.extend([0, 0, 0, 1, 0x40]);
        payload
    }

    #[test]
    fn reads_the_prefix_the_tv_stream_sends() {
        assert_eq!(
            FramePrefix::read(&tv_picture()),
            FramePrefix {
                status: Status::Read,
                byte0: Some(0x81),
                header_size: Some(8),
                byte2: Some(1),
                byte3: Some(2),
                capture_time_us: Some(0x1122_3344),
                sender_paced: None,
                presentation_time_us: None,
                access_unit_length: Some(7_784),
                prefix_bytes: Some(20),
                others: Vec::new(),
                others_total: 0,
            }
        );
    }

    #[test]
    fn reads_the_paced_flag_and_presentation_time_and_keeps_zero_apart_from_absent() {
        let both = FramePrefix::read(&extended(
            1,
            &[
                record(0x200, &[1]),
                record(0x201, &1_234_567_890_u64.to_le_bytes()),
            ],
        ));
        assert_eq!(
            (
                both.sender_paced,
                both.presentation_time_us,
                both.prefix_bytes
            ),
            (Some(1), Some(1_234_567_890), Some(8 + 4 + 5 + 12))
        );

        let zeros = FramePrefix::read(&extended(
            1,
            &[record(0x200, &[0]), record(0x201, &0_u64.to_le_bytes())],
        ));
        assert_eq!(
            (zeros.sender_paced, zeros.presentation_time_us),
            (Some(0), Some(0))
        );

        let none = FramePrefix::read(&extended(1, &[]));
        assert_eq!(none.status, Status::Read);
        assert_eq!(
            (
                none.sender_paced,
                none.presentation_time_us,
                none.access_unit_length,
                none.prefix_bytes
            ),
            (None, None, None, Some(12))
        );

        let other = FramePrefix::read(&extended(1, &[record(0x200, &[9])]));
        assert_eq!(other.sender_paced, Some(9), "kept as written");
    }

    #[test]
    fn the_last_of_a_repeated_tag_wins() {
        let prefix = FramePrefix::read(&extended(
            1,
            &[
                record(0x200, &[1]),
                record(0x200, &[0]),
                record(0x202, &5_u32.to_le_bytes()),
                record(0x202, &7_u32.to_le_bytes()),
            ],
        ));
        assert_eq!(
            (prefix.sender_paced, prefix.access_unit_length),
            (Some(0), Some(7))
        );
    }

    #[test]
    fn a_prefix_without_the_extension_has_the_fixed_part_only() {
        let short = FramePrefix::read(&[0x01, 8, 0, 2, 16, 0, 0, 0, 0, 0, 1, 0x67]);
        assert_eq!(short.status, Status::Read);
        assert_eq!(
            (short.capture_time_us, short.prefix_bytes, short.byte3),
            (Some(16), Some(8), Some(2))
        );
        let long = FramePrefix::read(&[0x01, 12, 0, 2, 1, 0, 0, 0, 9, 9, 9, 9, 0, 0, 1, 0x67]);
        assert_eq!(
            (long.capture_time_us, long.prefix_bytes),
            (Some(1), Some(12))
        );
        assert_eq!((long.sender_paced, long.others_total), (None, 0));
    }

    #[test]
    fn records_with_other_tags_are_kept_bounded_and_counted() {
        let prefix = FramePrefix::read(&extended(
            1,
            &[
                record(0x203, &[1, 2, 3]),
                record(0x204, &[9; 12]),
                record(0x205, &[]),
                record(0x206, &[7]),
                record(0x207, &[8]),
                record(0x208, &[8]),
            ],
        ));
        assert_eq!(prefix.status, Status::Read);
        assert_eq!(prefix.others_total, 6);
        assert_eq!(
            prefix.others,
            [
                OtherRecord {
                    tag: 0x203,
                    len: 3,
                    head: 0x03_0201
                },
                OtherRecord {
                    tag: 0x204,
                    len: 12,
                    head: 0x0909_0909_0909_0909
                },
                OtherRecord {
                    tag: 0x205,
                    len: 0,
                    head: 0
                },
                OtherRecord {
                    tag: 0x206,
                    len: 1,
                    head: 7
                },
            ]
        );
    }

    #[test]
    fn what_is_not_a_sender_prefix_is_absent_and_keeps_only_the_first_byte() {
        for (payload, first) in [
            (vec![], None),
            (vec![0x81], Some(0x81)),
            (vec![0, 0, 0, 1, 0x67], Some(0)),
            (vec![0x47, 0x53, 0, 2, 1, 0], Some(0x47)),
            (vec![0x01, 0, 0, 2, 0, 0, 0, 0, 0, 0, 1], Some(1)),
            (vec![0x02, 8, 0, 0, 0, 0, 0, 0, 0], Some(2)),
        ] {
            assert_eq!(
                FramePrefix::read(&payload),
                FramePrefix {
                    byte0: first,
                    ..FramePrefix::default()
                },
                "{payload:?}"
            );
        }
    }

    #[test]
    fn a_prefix_that_does_not_fit_is_malformed_and_keeps_only_its_first_two_bytes() {
        let too_long = extended(1, &[record(0x300, &[0; 1021])]);
        let cases = [
            ("fixed part under 8", vec![0x01, 7, 0, 2, 0, 0, 0, 0, 0]),
            (
                "fixed part past the packet",
                vec![0x01, 12, 0, 2, 0, 0, 0, 0],
            ),
            (
                "no room for the extension length",
                vec![0x81, 8, 1, 2, 0, 0, 0, 0, 8, 0, 0],
            ),
            ("extension past the limit", too_long),
            (
                "extension past the packet",
                [
                    vec![0x81, 8, 1, 2, 0, 0, 0, 0, 9, 0, 0, 0],
                    record(0x202, &[0; 4]),
                ]
                .concat(),
            ),
            (
                "record header cut",
                vec![0x81, 8, 1, 2, 0, 0, 0, 0, 3, 0, 0, 0, 2, 2, 4, 9],
            ),
            (
                "record value cut",
                vec![0x81, 8, 1, 2, 0, 0, 0, 0, 6, 0, 0, 0, 2, 2, 4, 0, 1, 2, 9],
            ),
            (
                "paced flag with no value",
                [
                    vec![0x81, 8, 1, 2, 0, 0, 0, 0, 4, 0, 0, 0],
                    record(0x200, &[]),
                    vec![9],
                ]
                .concat(),
            ),
            (
                "presentation time of 4 bytes",
                [
                    vec![0x81, 8, 1, 2, 0, 0, 0, 0, 8, 0, 0, 0],
                    record(0x201, &[1, 2, 3, 4]),
                    vec![9],
                ]
                .concat(),
            ),
            (
                "access-unit length of 2 bytes",
                [
                    vec![0x81, 8, 1, 2, 0, 0, 0, 0, 6, 0, 0, 0],
                    record(0x202, &[1, 2]),
                    vec![9],
                ]
                .concat(),
            ),
            (
                "no codec data after the fixed part",
                vec![0x01, 8, 0, 2, 0, 0, 0, 0],
            ),
            (
                "no codec data after the extension",
                [
                    vec![0x81, 8, 1, 2, 0, 0, 0, 0, 8, 0, 0, 0],
                    record(0x202, &[0; 4]),
                ]
                .concat(),
            ),
        ];
        for (why, payload) in cases {
            assert_eq!(
                FramePrefix::read(&payload),
                FramePrefix {
                    status: Status::Malformed,
                    byte0: Some(payload[0]),
                    header_size: Some(payload[1]),
                    ..FramePrefix::default()
                },
                "{why}"
            );
        }
    }

    #[test]
    fn an_extension_of_exactly_the_limit_is_read() {
        let prefix = FramePrefix::read(&extended(1, &[record(0x300, &[0; 1020])]));
        assert_eq!(prefix.status, Status::Read);
        assert_eq!(prefix.prefix_bytes, Some(8 + 4 + 1024));
        assert_eq!(prefix.others_total, 1);
    }

    fn picture(log: &FramePrefixLog, at: Instant, index: u32) {
        let payload = extended(index * 100, &[record(0x202, &7_u32.to_le_bytes())]);
        log.record(at, index, index * 750, &payload, Some(20));
    }

    fn counts(log: &FramePrefixLog) -> (u64, usize, u64) {
        let json = log.to_json(Instant::now(), 0);
        (
            json["counts"]["seen"].as_u64().unwrap(),
            json["rows"].as_array().unwrap().len(),
            json["counts"]["omitted"].as_u64().unwrap(),
        )
    }

    #[test]
    fn a_log_that_is_not_armed_records_nothing() {
        let log = FramePrefixLog::default();
        log.observe(1, 2, &tv_picture(), Some(20));
        assert_eq!(counts(&log), (0, 0, 0));
        assert!(log.to_json(Instant::now(), 0)["rows"][0].is_null());
        log.arm();
        assert!(
            log.lock().rows.capacity() >= CAPACITY,
            "allocated when armed"
        );
        log.observe(1, 2, &tv_picture(), Some(20));
        assert_eq!(counts(&log), (1, 1, 0));
    }

    #[test]
    fn the_log_is_bounded_and_counts_what_it_omits() {
        let log = FramePrefixLog::default();
        log.arm();
        let now = Instant::now();
        for index in 0..u32::try_from(CAPACITY + 3).unwrap() {
            picture(&log, now, index);
        }
        assert_eq!(counts(&log), (CAPACITY as u64 + 3, CAPACITY, 3));
        let json = log.to_json(now, 0);
        assert_eq!(json["counts"]["read"], CAPACITY as u64 + 3);
        assert_eq!(json["capacity"], CAPACITY);
    }

    #[test]
    fn recording_ends_with_the_window() {
        let log = FramePrefixLog::default();
        log.arm();
        let t0 = Instant::now();
        picture(&log, t0, 1);
        picture(&log, t0 + WINDOW - Duration::from_millis(1), 2);
        assert_eq!(counts(&log).0, 2);
        picture(&log, t0 + WINDOW, 3);
        assert_eq!(
            counts(&log).0,
            2,
            "the first picture past the window is not recorded"
        );
        log.observe(4, 4, &tv_picture(), Some(20));
        assert_eq!(counts(&log).0, 2, "and the log is disarmed from then on");
    }

    #[test]
    fn rows_follow_the_field_list_and_keep_what_is_absent_as_null() {
        let log = FramePrefixLog::default();
        log.arm();
        let t0 = Instant::now();
        log.record(
            t0 + Duration::from_millis(250),
            41,
            3_000,
            &tv_picture(),
            Some(20),
        );
        log.record(
            t0 + Duration::from_millis(260),
            42,
            3_750,
            &[0, 0, 0, 1, 0x67],
            Some(0),
        );
        log.record(
            t0 + Duration::from_millis(270),
            43,
            4_500,
            &[0x01, 4, 0, 0],
            None,
        );
        let json = log.to_json(t0 + Duration::from_secs(1), 5_000_000_000);

        let fields = json["fields"].as_array().unwrap();
        let rows = json["rows"].as_array().unwrap();
        assert!(
            rows.iter()
                .all(|row| row.as_array().unwrap().len() == fields.len())
        );
        let column = |name: &str| fields.iter().position(|field| field == name).unwrap();

        let read = &rows[0];
        assert_eq!(read[column("frame_index")], 41);
        assert_eq!(read[column("rtp_timestamp")], 3_000);
        assert_eq!(read[column("arrival_ns")], 4_250_000_000_i64);
        assert_eq!(read[column("status")], "read");
        assert_eq!(read[column("capture_time_us")], 0x1122_3344);
        assert_eq!(read[column("access_unit_length")], 7_784);
        assert_eq!(read[column("prefix_bytes")], 20);
        assert_eq!(read[column("stripped_bytes")], 20);
        for name in ["sender_paced", "presentation_time_us", "other_tags"] {
            assert_eq!(read[column(name)], Value::Null, "{name}");
        }

        let absent = &rows[1];
        assert_eq!(absent[column("status")], "absent");
        assert_eq!(absent[column("byte0")], 0);
        assert_eq!(
            absent[column("stripped_bytes")],
            0,
            "a length the receiver measured"
        );
        for name in [
            "header_size",
            "capture_time_us",
            "sender_paced",
            "prefix_bytes",
        ] {
            assert_eq!(absent[column(name)], Value::Null, "{name}");
        }

        let malformed = &rows[2];
        assert_eq!(malformed[column("status")], "malformed");
        assert_eq!(malformed[column("header_size")], 4);
        assert_eq!(malformed[column("stripped_bytes")], Value::Null);
        assert_eq!(malformed[column("capture_time_us")], Value::Null);

        let by_status = &json["counts"];
        assert_eq!(
            (
                by_status["read"].clone(),
                by_status["absent"].clone(),
                by_status["malformed"].clone()
            ),
            (json!(1), json!(1), json!(1))
        );
        let proves = json["definitions"]["proves"].as_str().unwrap();
        assert!(proves.contains("Cloud G-SYNC") && proves.contains("pixels"));
    }

    #[test]
    fn other_tags_are_listed_with_their_total() {
        let log = FramePrefixLog::default();
        log.arm();
        let payload = extended(1, &[record(0x203, &[1, 2, 3])]);
        log.record(Instant::now(), 1, 2, &payload, Some(23));
        let json = log.to_json(Instant::now(), 0);
        let column = json["fields"]
            .as_array()
            .unwrap()
            .iter()
            .position(|field| field == "other_tags")
            .unwrap();
        assert_eq!(
            json["rows"][0][column],
            json!({ "total": 1, "records": [[0x203, 3, 0x03_0201]] })
        );
    }
}
