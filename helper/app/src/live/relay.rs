//! The relay: the integration reads go2rtc's restream of the camera inside Home Assistant and sends the access units over the paired
//! channel as binary frames (type 2), chunked (lumashow-ha `docs/protocol.md` §10c). This puts the chunks back into units on the bridge's
//! thread (cheap: a copy) and hands them to the feeding thread through a queue; nothing waits for a segment, so a picture reaches the
//! decoder as soon as its last chunk is in.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Instant;

/// The frame type the relay's frames come as.
pub const FRAME_TYPE: u16 = 2;
pub const HEADER: usize = 12;
const FLAG_FIRST: u8 = 1;
const FLAG_LAST: u8 = 2;
const FLAG_KEY: u8 = 4;
/// Units waiting for the feeder beyond this are dropped until the next key picture: the feeder is stalled, and queueing would only add
/// delay to what it shows when it comes back (three seconds at 30 fps).
pub const QUEUE_MAX: u64 = 90;

/// One access unit, whole: Annex B, with the RTP timestamp it came with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub timestamp: u32,
    pub key: bool,
    pub data: Vec<u8>,
    /// When the last chunk arrived: the feeding thread's own measure of its delay.
    pub arrived: Instant,
}

/// The header of one chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    pub stream: u16,
    pub first: bool,
    pub last: bool,
    pub key: bool,
    pub seq: u32,
    pub timestamp: u32,
}

pub fn parse_head(payload: &[u8]) -> Option<Head> {
    if payload.len() < HEADER {
        return None;
    }
    let flags = payload[2];
    Some(Head {
        stream: u16::from_be_bytes([payload[0], payload[1]]),
        first: flags & FLAG_FIRST != 0,
        last: flags & FLAG_LAST != 0,
        key: flags & FLAG_KEY != 0,
        seq: u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]),
        timestamp: u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]),
    })
}

/// Chunks of one stream in, whole units out. A missed frame (a gap in `seq`) loses the unit it belonged to and nothing else.
pub struct Assembler {
    pub stream: u16,
    expected: Option<u32>,
    unit: Vec<u8>,
    building: bool,
    pub units: u64,
    pub lost: u64,
}

impl Assembler {
    pub fn new(stream: u16) -> Self {
        Self {
            stream,
            expected: None,
            unit: Vec::new(),
            building: false,
            units: 0,
            lost: 0,
        }
    }

    /// A frame's payload (header and chunk); the unit it completes, if any.
    pub fn push(&mut self, payload: &[u8]) -> Option<Unit> {
        let head = parse_head(payload)?;
        if head.stream != self.stream {
            return None;
        }
        if let Some(expected) = self.expected
            && head.seq != expected
        {
            self.lost += 1;
            self.building = false;
            self.unit.clear();
        }
        self.expected = Some(head.seq.wrapping_add(1));
        if head.first {
            self.unit.clear();
            self.building = true;
        }
        if !self.building {
            return None;
        }
        self.unit.extend_from_slice(&payload[HEADER..]);
        if !head.last {
            return None;
        }
        self.building = false;
        self.units += 1;
        Some(Unit {
            timestamp: head.timestamp,
            key: head.key,
            data: std::mem::take(&mut self.unit),
            arrived: Instant::now(),
        })
    }
}

/// The queue between the bridge's thread and the feeding thread, with its counts.
pub struct Queue {
    tx: Sender<Unit>,
    pub queued: Arc<AtomicU64>,
    pub dropped: Arc<AtomicU64>,
    skipping: bool,
}

pub struct Taker {
    rx: Receiver<Unit>,
    queued: Arc<AtomicU64>,
}

impl Queue {
    pub fn new() -> (Self, Taker) {
        let (tx, rx) = channel();
        let queued = Arc::new(AtomicU64::new(0));
        (
            Self {
                tx,
                queued: queued.clone(),
                dropped: Arc::new(AtomicU64::new(0)),
                skipping: false,
            },
            Taker { rx, queued },
        )
    }

    /// Queues a unit; a queue grown past `QUEUE_MAX` drops it and every one after until a key picture.
    pub fn give(&mut self, unit: Unit) -> bool {
        if self.queued.load(Ordering::SeqCst) >= QUEUE_MAX {
            self.skipping = true;
        }
        if self.skipping {
            if !unit.key || self.queued.load(Ordering::SeqCst) >= QUEUE_MAX {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            self.skipping = false;
        }
        self.queued.fetch_add(1, Ordering::SeqCst);
        if self.tx.send(unit).is_err() {
            self.queued.fetch_sub(1, Ordering::SeqCst);
            return false;
        }
        true
    }
}

/// What a wait on the queue gave.
pub enum Taken {
    Unit(Unit),
    /// Nothing came in time.
    Nothing,
    /// The giving end is gone.
    Gone,
}

impl Taker {
    /// The next unit, waiting up to `wait`.
    pub fn take(&self, wait: std::time::Duration) -> Taken {
        match self.rx.recv_timeout(wait) {
            Ok(unit) => {
                self.queued.fetch_sub(1, Ordering::SeqCst);
                Taken::Unit(unit)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Taken::Nothing,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Taken::Gone,
        }
    }

    /// Whatever is queued now, without waiting.
    pub fn drain(&self) -> Vec<Unit> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(unit) => {
                    self.queued.fetch_sub(1, Ordering::SeqCst);
                    out.push(unit);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }
}

/// Builds the frames of one unit as the integration sends them: for the tests and the fake integration.
pub fn frames(
    stream: u16,
    seq: &mut u32,
    timestamp: u32,
    key: bool,
    data: &[u8],
    chunk: usize,
) -> Vec<Vec<u8>> {
    let pieces: Vec<&[u8]> = if data.is_empty() {
        vec![&[][..]]
    } else {
        data.chunks(chunk).collect()
    };
    pieces
        .iter()
        .enumerate()
        .map(|(i, piece)| {
            let mut flags = 0u8;
            if i == 0 {
                flags |= FLAG_FIRST;
            }
            if i == pieces.len() - 1 {
                flags |= FLAG_LAST;
            }
            if key {
                flags |= FLAG_KEY;
            }
            *seq = seq.wrapping_add(1);
            let mut frame = Vec::with_capacity(HEADER + piece.len());
            frame.extend_from_slice(&stream.to_be_bytes());
            frame.push(flags);
            frame.push(0);
            frame.extend_from_slice(&seq.to_be_bytes());
            frame.extend_from_slice(&timestamp.to_be_bytes());
            frame.extend_from_slice(piece);
            frame
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_become_units_and_a_lost_chunk_loses_only_its_unit() {
        let mut seq = 0;
        let big: Vec<u8> = (0..150_000u32).map(|i| (i % 251) as u8).collect();
        let mut frames_a = frames(7, &mut seq, 90_000, true, &big, 60_000);
        assert_eq!(frames_a.len(), 3);
        let frames_b = frames(7, &mut seq, 93_000, false, b"small", 60_000);
        let frames_c = frames(7, &mut seq, 96_000, false, b"after", 60_000);
        let mut a = Assembler::new(7);
        assert!(a.push(&frames_a[0]).is_none());
        assert!(a.push(&frames_a[1]).is_none());
        let unit = a.push(&frames_a[2]).unwrap();
        assert_eq!(
            (unit.timestamp, unit.key, unit.data.len()),
            (90_000, true, 150_000)
        );
        assert_eq!(unit.data, big);
        // another stream's frame is ignored
        let other = frames(8, &mut 0, 1, false, b"x", 60_000);
        assert!(a.push(&other[0]).is_none());
        // b's only frame is lost: c still arrives whole, and the loss is counted
        let _ = frames_b;
        let unit = a.push(&frames_c[0]).unwrap();
        assert_eq!(unit.data, b"after");
        assert_eq!((a.units, a.lost), (2, 1));
        // a middle chunk lost: the unit is dropped, the next unit is whole
        let mut frames_d = frames(7, &mut seq, 99_000, true, &big, 60_000);
        frames_d.remove(1);
        assert!(a.push(&frames_d[0]).is_none());
        assert!(a.push(&frames_d[1]).is_none());
        let frames_e = frames(7, &mut seq, 102_000, false, b"e", 60_000);
        assert_eq!(a.push(&frames_e[0]).unwrap().data, b"e");
        assert_eq!((a.units, a.lost), (3, 2));
        frames_a.clear();
        assert!(a.push(b"short").is_none());
    }

    #[test]
    fn a_stalled_feeder_drops_until_a_key_picture() {
        let (mut queue, taker) = Queue::new();
        let unit = |key: bool| Unit {
            timestamp: 0,
            key,
            data: vec![1],
            arrived: Instant::now(),
        };
        for _ in 0..QUEUE_MAX {
            assert!(queue.give(unit(false)));
        }
        assert!(!queue.give(unit(false)));
        assert!(!queue.give(unit(true)), "still full");
        assert_eq!(taker.drain().len() as u64, QUEUE_MAX);
        assert!(!queue.give(unit(false)), "not until a key picture");
        assert!(queue.give(unit(true)));
        assert!(queue.give(unit(false)));
        assert_eq!(queue.dropped.load(Ordering::Relaxed), 3);
        assert!(matches!(
            taker.take(std::time::Duration::from_millis(10)),
            Taken::Unit(Unit { key: true, .. })
        ));
    }
}
