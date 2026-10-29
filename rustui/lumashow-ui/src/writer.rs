//! The bounded, nonblocking writer behind `Scene::try_commit` (design section 3.1, item 5).
//!
//! Nothing here ever waits. Two lanes feed one socket:
//!
//! * **Reliable**: committed transactions, framed (a `u32` length and the postcard body, as `compositor_protocol::write_frame`
//!   makes them) into *segments* of at most 64 KiB of whole frames. At most [`MAX_SEGMENTS`] (three) are held: the one the
//!   socket is taking now (*in flight*, possibly half written) and two queued behind it. A transaction that does not fit the
//!   free segments is refused with `WouldBlock` before anything about it changes; one that could never fit is `TooLarge`.
//!   Nothing accepted is dropped, reordered or sent twice.
//! * **Urgent**: the requests that act at once (pointer samples, settles, cancels, backdrop leases, the display and clock
//!   switches). Each is a short frame of its own. At every frame boundary an urgent frame goes before the next reliable
//!   frame, so input overtakes everything not yet started and waits for at most the frame already half written. Only a pointer
//!   *move* that has not started yet is replaced by a newer one of the same contact on the same track; a down, up or cancel, a
//!   settle and everything else is delivered, in order (an up never replaces the move before it, which the compositor needs
//!   for the velocity). The lane is bounded as well: filling it means the compositor is not reading, and is an error.
//!
//! An urgent frame about a track or a set that a transaction creates (`after_tag`) waits until that transaction has been
//! written completely; the compositor's session would refuse it before.

use crate::error::SubmitError;
use compositor_protocol::{MAX_FRAME, Request};
use std::collections::VecDeque;
use std::io;

/// One segment: a frame of the largest legal size always fits an empty one.
pub(crate) const SEGMENT_BYTES: usize = MAX_FRAME as usize + 4;
/// Segments held: one in flight and two queued.
pub(crate) const MAX_SEGMENTS: usize = 3;
/// Urgent frames waiting at once.
pub(crate) const URGENT_CAP: usize = 128;
/// The longest urgent frame (length prefix included): a pointer sample is 25 bytes, a settle 40.
pub(crate) const URGENT_FRAME: usize = 64;
/// Ready urgent frames are put together here, so a backlog costs one write.
const SCRATCH_BYTES: usize = 1024;

/// Where the bytes go. `write` never blocks: it takes what it can or says `WouldBlock`.
pub(crate) trait Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize>;
}

/// What the last `pump` left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pump {
    /// Everything accepted so far has been written.
    Idle,
    /// The sink is full: bytes are waiting for it to become writable again.
    Blocked,
}

/// Counters for the tests and the diagnostics.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stats {
    /// Pointer moves replaced by a newer one before they started.
    pub replaced: u64,
    /// Transactions taken.
    pub transactions: u64,
}

struct Segment {
    buf: Vec<u8>,
    /// Bytes of frames in `buf`.
    used: usize,
    /// Bytes the sink has taken.
    sent: usize,
    /// A frame boundary at or before `sent`: where the walk to the boundary around `sent` starts.
    frame_start: usize,
    /// The tag of the commit that ends this segment's last frame, if it does.
    commit: Option<u64>,
}

enum PushError {
    Full,
    Encoding(postcard::Error),
}

impl Segment {
    fn new() -> Self {
        Self {
            buf: vec![0; SEGMENT_BYTES],
            used: 0,
            sent: 0,
            frame_start: 0,
            commit: None,
        }
    }

    fn reset(&mut self) {
        self.used = 0;
        self.sent = 0;
        self.frame_start = 0;
        self.commit = None;
    }

    /// Frames `request` after the ones already here.
    fn push(&mut self, request: &Request) -> Result<(), PushError> {
        let at = self.used;
        let Some(room) = self.buf.get_mut(at + 4..) else {
            return Err(PushError::Full);
        };
        match postcard::to_slice(request, room) {
            Ok(body) => {
                // The room is at most MAX_FRAME, so the body is a legal length.
                let n = body.len();
                self.buf[at..at + 4].copy_from_slice(&(n as u32).to_le_bytes());
                self.used = at + 4 + n;
                Ok(())
            }
            Err(postcard::Error::SerializeBufferFull) => Err(PushError::Full),
            Err(e) => Err(PushError::Encoding(e)),
        }
    }

    fn frame_len(&self, at: usize) -> usize {
        4 + u32::from_le_bytes(self.buf[at..at + 4].try_into().expect("four bytes")) as usize
    }

    /// Whether the sink has taken a whole number of frames (so an urgent frame may go next).
    fn at_boundary(&mut self) -> bool {
        while self.frame_start < self.sent {
            let end = self.frame_start + self.frame_len(self.frame_start);
            if end <= self.sent {
                self.frame_start = end;
            } else {
                break;
            }
        }
        self.frame_start == self.sent
    }

    /// Where the frame the sink is in the middle of ends (after `at_boundary` said it is not at a boundary).
    fn frame_end(&self) -> usize {
        self.frame_start + self.frame_len(self.frame_start)
    }
}

/// How an urgent request is queued.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Urgent {
    /// The commit that creates what the request is about: it is not written before that commit has been (0: nothing).
    pub after_tag: u64,
    /// What the request is about (a track, a set); frames of one stream keep their order. 0: none.
    pub stream: u32,
    /// A pointer move of this contact: a newer one replaces it while it has not started.
    pub replaceable: Option<u32>,
}

#[derive(Clone, Copy)]
struct UrgentFrame {
    bytes: [u8; URGENT_FRAME],
    len: u8,
    how: Urgent,
}

pub(crate) struct Writer<S> {
    sink: S,
    /// Front: in flight. At most [`MAX_SEGMENTS`].
    queue: VecDeque<Segment>,
    /// Empty segments kept for the next transaction, so committing allocates nothing once warm.
    pool: Vec<Segment>,
    /// Segments of the transaction being framed.
    taken: Vec<Segment>,
    urgent: VecDeque<UrgentFrame>,
    scratch: Vec<u8>,
    scratch_len: usize,
    scratch_off: usize,
    /// The tag of the last commit whose frame has been written completely.
    written_tag: u64,
    failed: Option<(io::ErrorKind, String)>,
    stats: Stats,
}

impl<S: Sink> Writer<S> {
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            queue: VecDeque::with_capacity(MAX_SEGMENTS),
            // Every segment a transaction can need at once (three held and one being framed) exists from the start, so that
            // committing allocates nothing, ever: the pages are zero pages until a frame is written into them.
            pool: (0..=MAX_SEGMENTS).map(|_| Segment::new()).collect(),
            taken: Vec::with_capacity(MAX_SEGMENTS),
            urgent: VecDeque::with_capacity(URGENT_CAP),
            scratch: vec![0; SCRATCH_BYTES],
            scratch_len: 0,
            scratch_off: 0,
            written_tag: 0,
            failed: None,
            stats: Stats::default(),
        }
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    #[cfg(test)]
    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    #[cfg(test)]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The failure that ended the connection, if one did (sticky).
    pub fn failure(&self) -> Option<io::Error> {
        self.failed
            .as_ref()
            .map(|(kind, message)| io::Error::new(*kind, message.clone()))
    }

    fn fail(&mut self, e: io::Error) -> io::Error {
        if self.failed.is_none() {
            self.failed = Some((e.kind(), e.to_string()));
        }
        e
    }

    /// The tag of the last commit written completely to the sink.
    #[cfg(test)]
    pub fn written_tag(&self) -> u64 {
        self.written_tag
    }

    /// Whether a transaction could be taken now.
    pub fn has_room(&self) -> bool {
        self.queue.len() < MAX_SEGMENTS
    }

    #[cfg(test)]
    pub fn queued_segments(&self) -> usize {
        self.queue.len()
    }

    #[cfg(test)]
    pub fn pending_urgent(&self) -> usize {
        self.urgent.len()
    }

    /// Whether the sink should be waited on for writability: something is accepted and not yet written. (An urgent frame that
    /// waits for a transaction nobody has committed yet is nothing to wait for.)
    pub fn wants_writable(&self) -> bool {
        !self.queue.is_empty()
            || self.scratch_off < self.scratch_len
            || self
                .urgent
                .iter()
                .any(|f| f.how.after_tag <= self.written_tag)
    }

    fn fresh(&mut self) -> Segment {
        match self.pool.pop() {
            Some(mut s) => {
                s.reset();
                s
            }
            None => Segment::new(),
        }
    }

    fn recycle(&mut self, mut s: Segment) {
        s.reset();
        if self.pool.len() <= MAX_SEGMENTS {
            self.pool.push(s);
        }
    }

    /// Takes `requests` and their commit `tag` as one transaction, or refuses without any change: `WouldBlock` when the free
    /// segments cannot hold it now, `TooLarge` when they never could. On success the bytes are queued; call [`Writer::pump`]
    /// to start writing them.
    pub fn submit(&mut self, requests: &[Request], tag: u64) -> Result<(), SubmitError> {
        if let Some(e) = self.failure() {
            return Err(SubmitError::Io(e));
        }
        let room = MAX_SEGMENTS - self.queue.len();
        if room == 0 {
            return Err(SubmitError::WouldBlock);
        }
        let commit = Request::Commit { tag };
        let mut cur = self.fresh();
        self.taken.clear();
        let mut verdict: Result<(), SubmitError> = Ok(());
        for request in requests.iter().chain(std::iter::once(&commit)) {
            let mut pushed = cur.push(request);
            if matches!(pushed, Err(PushError::Full)) {
                let next = self.fresh();
                self.taken.push(std::mem::replace(&mut cur, next));
                if self.taken.len() + 1 > MAX_SEGMENTS {
                    verdict = Err(SubmitError::TooLarge);
                    break;
                }
                pushed = cur.push(request);
            }
            match pushed {
                Ok(()) => {}
                Err(PushError::Full) => unreachable!("a legal frame fits an empty segment"),
                Err(PushError::Encoding(e)) => {
                    verdict = Err(SubmitError::Io(io::Error::new(
                        io::ErrorKind::InvalidData,
                        e,
                    )));
                    break;
                }
            }
        }
        if verdict.is_ok() && self.taken.len() + 1 > room {
            verdict = Err(SubmitError::WouldBlock);
        }
        match verdict {
            Ok(()) => {
                cur.commit = Some(tag);
                self.queue.extend(self.taken.drain(..));
                self.queue.push_back(cur);
                self.stats.transactions += 1;
                Ok(())
            }
            Err(e) => {
                while let Some(s) = self.taken.pop() {
                    self.recycle(s);
                }
                self.recycle(cur);
                Err(e)
            }
        }
    }

    /// Queues an urgent request and writes what the sink will take. The error is the connection's: the sink failed, or the
    /// lane filled up because the compositor is not reading.
    pub fn push_urgent(&mut self, request: &Request, how: Urgent) -> io::Result<()> {
        if let Some(e) = self.failure() {
            return Err(e);
        }
        let mut frame = UrgentFrame {
            bytes: [0; URGENT_FRAME],
            len: 0,
            how,
        };
        let n = postcard::to_slice(request, &mut frame.bytes[4..])
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .len();
        frame.bytes[..4].copy_from_slice(&(n as u32).to_le_bytes());
        frame.len = (4 + n) as u8;
        let mut replaced = false;
        if let Some(contact) = how.replaceable {
            if let Some(last) = self
                .urgent
                .iter_mut()
                .rev()
                .find(|f| f.how.stream == how.stream)
            {
                if last.how.replaceable == Some(contact) {
                    *last = frame;
                    replaced = true;
                    self.stats.replaced += 1;
                }
            }
        }
        if !replaced {
            if self.urgent.len() >= URGENT_CAP {
                return Err(self.fail(io::Error::other(format!(
                    "the compositor is not reading: {URGENT_CAP} urgent requests are waiting"
                ))));
            }
            self.urgent.push_back(frame);
        }
        self.pump().map(|_| ())
    }

    /// Forgets the urgent requests about `stream` that have not started: what they are about is going away, and a request
    /// about it that reached the compositor after its deletion would be refused. (Stream 0, the requests about nothing, is
    /// never forgotten.)
    pub fn forget_stream(&mut self, stream: u32) {
        if stream != 0 {
            self.urgent.retain(|f| f.how.stream != stream);
        }
    }

    /// Moves the urgent frames that may go now into the scratch buffer, in order. Whether any did.
    fn fill_scratch(&mut self) -> bool {
        let mut filled = false;
        let mut i = 0;
        while i < self.urgent.len() {
            let f = self.urgent[i];
            if f.how.after_tag > self.written_tag {
                i += 1;
                continue;
            }
            let len = usize::from(f.len);
            if self.scratch_len + len > SCRATCH_BYTES {
                break;
            }
            self.scratch[self.scratch_len..self.scratch_len + len].copy_from_slice(&f.bytes[..len]);
            self.scratch_len += len;
            self.urgent.remove(i);
            filled = true;
        }
        filled
    }

    /// Writes as much as the sink takes, urgent frames first at every frame boundary. Never waits.
    pub fn pump(&mut self) -> io::Result<Pump> {
        if let Some(e) = self.failure() {
            return Err(e);
        }
        loop {
            let result = if self.scratch_off < self.scratch_len {
                let wrote = self
                    .sink
                    .write(&self.scratch[self.scratch_off..self.scratch_len]);
                if let Ok(n) = wrote {
                    self.scratch_off += n;
                    if self.scratch_off == self.scratch_len {
                        self.scratch_off = 0;
                        self.scratch_len = 0;
                    }
                }
                wrote
            } else {
                let boundary = self.queue.front_mut().is_none_or(Segment::at_boundary);
                if boundary && self.fill_scratch() {
                    continue;
                }
                let Some(seg) = self.queue.front_mut() else {
                    return Ok(Pump::Idle);
                };
                let end = if boundary { seg.used } else { seg.frame_end() };
                let wrote = self.sink.write(&seg.buf[seg.sent..end]);
                if let Ok(n) = wrote {
                    seg.sent += n;
                    if seg.sent == seg.used {
                        let done = self.queue.pop_front().expect("the front segment");
                        if let Some(tag) = done.commit {
                            self.written_tag = tag;
                        }
                        self.recycle(done);
                    }
                }
                wrote
            };
            match result {
                Ok(0) => return Err(self.fail(io::ErrorKind::WriteZero.into())),
                Ok(_) => {}
                Err(e) => match e.kind() {
                    io::ErrorKind::WouldBlock => return Ok(Pump::Blocked),
                    io::ErrorKind::Interrupted => {}
                    _ => return Err(self.fail(e)),
                },
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use compositor_protocol::{
        Content, DrivePhase, DriveSample, Fill, LayerId, MotionId, Props, Rgba, Shape, read_frame,
    };

    /// A sink that takes `budget` bytes at most `chunk` at a time, then says `WouldBlock`; or fails.
    pub(crate) struct Fake {
        pub budget: usize,
        pub chunk: usize,
        pub wire: Vec<u8>,
        pub error: Option<io::ErrorKind>,
        pub calls: usize,
    }

    impl Fake {
        pub fn new(budget: usize) -> Self {
            Self {
                budget,
                chunk: usize::MAX,
                wire: Vec::new(),
                error: None,
                calls: 0,
            }
        }
    }

    impl Sink for Fake {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            if let Some(kind) = self.error {
                return Err(kind.into());
            }
            if self.budget == 0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let n = buf.len().min(self.budget).min(self.chunk);
            self.wire.extend_from_slice(&buf[..n]);
            self.budget -= n;
            Ok(n)
        }
    }

    /// Every request on a wire, in order. A torn frame is a panic.
    pub(crate) fn decode(mut wire: &[u8]) -> Vec<Request> {
        let mut out = Vec::new();
        let mut buf = Vec::new();
        while let Some(r) = read_frame::<Request>(&mut wire, &mut buf).expect("whole frames") {
            out.push(r);
        }
        out
    }

    fn create(n: u32) -> Request {
        Request::Create {
            id: LayerId(n),
            width: 10,
            height: 10,
            z: 0,
            content: Content::Shape(Shape {
                corner_radius: 1.0,
                fill: Fill::Solid(Rgba(1, 2, 3, 4)),
                border_width: 0.0,
                border: Rgba(0, 0, 0, 0),
            }),
            props: Props::at(n as f32, 0.0),
        }
    }

    fn drive(track: u32, contact: u32, phase: DrivePhase, position: f32) -> Request {
        Request::MotionDrive {
            id: MotionId(track),
            sample: DriveSample {
                contact,
                phase,
                position,
                time_ns: position as u64 * 1000,
            },
        }
    }

    fn urgent(
        w: &mut Writer<Fake>,
        r: &Request,
        stream: u32,
        after_tag: u64,
        contact: Option<u32>,
    ) {
        w.push_urgent(
            r,
            Urgent {
                after_tag,
                stream,
                replaceable: contact,
            },
        )
        .unwrap();
    }

    fn mv(w: &mut Writer<Fake>, track: u32, contact: u32, position: f32) {
        urgent(
            w,
            &drive(track, contact, DrivePhase::Move, position),
            track,
            0,
            Some(contact),
        );
    }

    fn other(w: &mut Writer<Fake>, track: u32, contact: u32, phase: DrivePhase, position: f32) {
        urgent(w, &drive(track, contact, phase, position), track, 0, None);
    }

    /// A tag no test commit reaches until it submits it: frames held behind it stay in the lane, unstarted.
    const HELD: u64 = 99;

    fn hold(w: &mut Writer<Fake>, r: &Request, stream: u32, contact: Option<u32>) {
        urgent(w, r, stream, HELD, contact);
    }

    #[test]
    fn a_transaction_goes_out_whole_in_order_and_with_its_commit_last() {
        let mut w = Writer::new(Fake::new(usize::MAX));
        let staged = vec![create(1), create(2), create(3)];
        w.submit(&staged, 7).unwrap();
        assert_eq!(w.pump().unwrap(), Pump::Idle);
        let mut expect = staged.clone();
        expect.push(Request::Commit { tag: 7 });
        assert_eq!(decode(&w.sink().wire), expect);
        assert_eq!(w.written_tag(), 7);
        assert!(!w.wants_writable());
    }

    #[test]
    fn a_full_queue_refuses_at_once_and_the_same_transaction_goes_out_whole_once_there_is_room() {
        // A sink that takes nothing: three transactions fill the three segments, the fourth is refused.
        let mut w = Writer::new(Fake::new(0));
        let staged = vec![create(1), create(2)];
        for tag in 1..=3 {
            w.submit(&staged, tag).unwrap();
            assert_eq!(w.pump().unwrap(), Pump::Blocked);
        }
        assert!(!w.has_room());
        let again = vec![create(9)];
        assert!(matches!(w.submit(&again, 4), Err(SubmitError::WouldBlock)));
        assert_eq!(w.queued_segments(), 3, "a refusal changes nothing");
        assert!(w.sink().wire.is_empty());
        // The reader catches up; the refused transaction is taken as it was and everything arrives in order.
        w.sink_mut().budget = usize::MAX;
        assert_eq!(w.pump().unwrap(), Pump::Idle);
        w.submit(&again, 4).unwrap();
        w.pump().unwrap();
        let wire = decode(&w.sink().wire);
        let tags: Vec<u64> = wire
            .iter()
            .filter_map(|r| match r {
                Request::Commit { tag } => Some(*tag),
                _ => None,
            })
            .collect();
        assert_eq!(tags, vec![1, 2, 3, 4]);
        assert_eq!(wire.len(), 3 * 3 + 2);
        assert_eq!(wire[wire.len() - 2], create(9));
    }

    #[test]
    fn a_transaction_bigger_than_a_segment_spans_segments_and_one_bigger_than_the_writer_is_too_large()
     {
        let mut w = Writer::new(Fake::new(usize::MAX));
        // About 100 bytes each: 1500 of them are two segments.
        let big: Vec<Request> = (0..1500).map(create).collect();
        w.submit(&big, 1).unwrap();
        assert_eq!(w.queued_segments(), 2);
        w.pump().unwrap();
        let wire = decode(&w.sink().wire);
        assert_eq!(wire.len(), 1501);
        assert_eq!(wire[1500], Request::Commit { tag: 1 });
        assert_eq!(&wire[..1500], &big[..]);
        // Four segments of requests can never be held, however empty the queue.
        let huge: Vec<Request> = (0..4500).map(create).collect();
        assert!(matches!(w.submit(&huge, 2), Err(SubmitError::TooLarge)));
        assert_eq!(w.queued_segments(), 0);
        // And with a segment busy the same transaction is blocked only if it could fit when free.
        let mut w = Writer::new(Fake::new(0));
        w.submit(&[create(1)], 1).unwrap();
        w.pump().unwrap();
        w.submit(&[create(1)], 2).unwrap();
        let two_segments: Vec<Request> = (0..1500).map(create).collect();
        assert!(matches!(
            w.submit(&two_segments, 3),
            Err(SubmitError::WouldBlock)
        ));
        assert!(
            matches!(w.submit(&huge, 3), Err(SubmitError::TooLarge)),
            "never blocked by room it cannot use"
        );
    }

    #[test]
    fn pointer_moves_that_have_not_started_are_replaced_by_newer_ones_and_nothing_else_is() {
        let mut w = Writer::new(Fake::new(usize::MAX));
        // Held behind a commit that has not been made, so that they are all unstarted together.
        hold(&mut w, &drive(1, 5, DrivePhase::Down, 10.0), 1, None);
        for p in [11.0, 12.0, 13.0, 14.0] {
            hold(&mut w, &drive(1, 5, DrivePhase::Move, p), 1, Some(5));
        }
        // The last move before the up stays; the up follows it and replaces nothing.
        hold(&mut w, &drive(1, 5, DrivePhase::Up, 15.0), 1, None);
        hold(&mut w, &drive(1, 5, DrivePhase::Cancel, 15.0), 1, None);
        assert_eq!(w.pending_urgent(), 4, "down, the newest move, up, cancel");
        assert_eq!(w.stats().replaced, 3);
        w.submit(&[], HELD).unwrap();
        w.pump().unwrap();
        let sent: Vec<(DrivePhase, f32)> = decode(&w.sink().wire)
            .into_iter()
            .filter_map(|r| match r {
                Request::MotionDrive { sample, .. } => Some((sample.phase, sample.position)),
                Request::Commit { tag } => {
                    assert_eq!(tag, HELD);
                    None
                }
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            sent,
            vec![
                (DrivePhase::Down, 10.0),
                (DrivePhase::Move, 14.0),
                (DrivePhase::Up, 15.0),
                (DrivePhase::Cancel, 15.0)
            ]
        );
    }

    #[test]
    fn moves_of_another_contact_or_track_and_moves_after_other_requests_are_not_replaced() {
        let mut w = Writer::new(Fake::new(usize::MAX));
        hold(&mut w, &drive(1, 5, DrivePhase::Move, 1.0), 1, Some(5));
        hold(&mut w, &drive(2, 5, DrivePhase::Move, 2.0), 2, Some(5)); // another track
        hold(&mut w, &drive(1, 6, DrivePhase::Move, 3.0), 1, Some(6)); // another contact: not the move before it
        assert_eq!(w.pending_urgent(), 3, "contacts do not replace each other");
        // A cancel between two moves of one contact: the later move may not replace the earlier one past it.
        let cancel = Request::MotionCancel {
            id: MotionId(2),
            policy: compositor_protocol::CancelPolicy::Hold,
        };
        hold(&mut w, &cancel, 2, None);
        hold(&mut w, &drive(2, 5, DrivePhase::Move, 4.0), 2, Some(5));
        assert_eq!(
            w.pending_urgent(),
            5,
            "the move after the cancel is a new frame"
        );
        assert_eq!(w.stats().replaced, 0);
    }

    #[test]
    fn input_goes_before_commits_that_have_not_started_and_never_inside_a_frame() {
        // The sink takes 7 bytes at a time and 200 in all, so a commit is half written when input arrives.
        let mut w = Writer::new(Fake::new(0));
        w.sink_mut().chunk = 7;
        let staged: Vec<Request> = (1..=4).map(create).collect();
        w.submit(&staged, 1).unwrap();
        w.sink_mut().budget = 30;
        w.pump().unwrap();
        assert_eq!(w.sink().wire.len(), 28 + 2, "writes in sevens: 4 * 7 + 2");
        w.submit(&staged, 2).unwrap();
        other(&mut w, 9, 1, DrivePhase::Down, 42.0);
        w.sink_mut().budget = usize::MAX;
        w.pump().unwrap();
        let sent = decode(&w.sink().wire);
        // The commit being written finishes the frame it was in, then the input, then the rest of both commits.
        let down = sent
            .iter()
            .position(|r| matches!(r, Request::MotionDrive { .. }))
            .expect("the down arrived");
        let first_commit = sent
            .iter()
            .position(|r| matches!(r, Request::Commit { tag: 1 }))
            .unwrap();
        assert!(
            down < first_commit,
            "input overtook the rest of the commit that was half written: {down} vs {first_commit}"
        );
        assert!(down > 0, "but not the frame that was half written");
        assert_eq!(sent.len(), 4 + 1 + 4 + 1 + 1);
        assert_eq!(
            sent.iter()
                .filter(|r| matches!(r, Request::Commit { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn a_request_about_a_track_waits_for_the_commit_that_creates_it() {
        let mut w = Writer::new(Fake::new(usize::MAX));
        // The track is created by commit 5, which has not been committed: its drive samples wait, an unrelated request does not.
        urgent(&mut w, &drive(3, 1, DrivePhase::Down, 1.0), 3, 5, None);
        urgent(&mut w, &Request::Stats, 0, 0, None);
        assert_eq!(decode(&w.sink().wire), vec![Request::Stats]);
        assert_eq!(w.pending_urgent(), 1);
        assert!(
            !w.wants_writable(),
            "waiting for a commit nobody made is nothing to wait on the socket for"
        );
        w.submit(&[create(1)], 5).unwrap();
        w.pump().unwrap();
        let sent = decode(&w.sink().wire);
        assert_eq!(sent.len(), 4);
        assert_eq!(sent[1], create(1));
        assert_eq!(sent[2], Request::Commit { tag: 5 });
        assert!(
            matches!(sent[3], Request::MotionDrive { .. }),
            "after the commit, not before it"
        );
    }

    #[test]
    fn urgent_requests_for_a_blocked_commit_wait_with_it_and_go_when_it_has_been_written() {
        let mut w = Writer::new(Fake::new(0));
        w.submit(&[create(1)], 5).unwrap();
        urgent(&mut w, &drive(3, 1, DrivePhase::Down, 1.0), 3, 5, None);
        assert!(w.wants_writable());
        w.sink_mut().budget = usize::MAX;
        w.pump().unwrap();
        let sent = decode(&w.sink().wire);
        assert_eq!(sent.len(), 3);
        assert!(matches!(sent[2], Request::MotionDrive { .. }));
    }

    #[test]
    fn a_dead_peer_is_an_error_and_stays_one() {
        let mut w = Writer::new(Fake::new(usize::MAX));
        w.sink_mut().error = Some(io::ErrorKind::BrokenPipe);
        w.submit(&[create(1)], 1).unwrap();
        let e = w.pump().unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::BrokenPipe);
        match w.submit(&[create(2)], 2) {
            Err(SubmitError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::BrokenPipe),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            w.push_urgent(
                &Request::Stats,
                Urgent {
                    after_tag: 0,
                    stream: 0,
                    replaceable: None
                }
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::BrokenPipe
        );
        // A sink that takes zero bytes is as dead.
        struct Zero;
        impl Sink for Zero {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Ok(0)
            }
        }
        let mut w = Writer::new(Zero);
        w.submit(&[create(1)], 1).unwrap();
        assert_eq!(w.pump().unwrap_err().kind(), io::ErrorKind::WriteZero);
    }

    #[test]
    fn the_urgent_lane_is_bounded_and_a_full_one_is_an_error_not_growth_or_a_dropped_request() {
        let mut w = Writer::new(Fake::new(usize::MAX));
        hold(&mut w, &drive(1, 1, DrivePhase::Move, 0.0), 1, Some(1));
        for n in 1..URGENT_CAP {
            hold(&mut w, &drive(2, 1, DrivePhase::Down, n as f32), 2, None);
        }
        assert_eq!(w.pending_urgent(), URGENT_CAP);
        // A move still replaces the one pending: the lane does not grow for it.
        hold(&mut w, &drive(1, 1, DrivePhase::Move, 9.0), 1, Some(1));
        assert_eq!(w.pending_urgent(), URGENT_CAP);
        // Anything else would be the 129th: refused, and nothing already waiting is lost.
        let e = w
            .push_urgent(
                &drive(2, 1, DrivePhase::Up, 0.0),
                Urgent {
                    after_tag: HELD,
                    stream: 2,
                    replaceable: None,
                },
            )
            .unwrap_err();
        assert!(e.to_string().contains("not reading"), "{e}");
        assert_eq!(w.pending_urgent(), URGENT_CAP);
        assert!(
            w.push_urgent(
                &Request::Stats,
                Urgent {
                    after_tag: 0,
                    stream: 0,
                    replaceable: None
                }
            )
            .is_err(),
            "and the failure is the connection's"
        );
    }

    /// A small xorshift, so the property test needs no crate and repeats exactly.
    fn next(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    #[test]
    fn whatever_the_sink_takes_and_whenever_input_arrives_every_frame_arrives_whole_once_and_in_order()
     {
        for seed in 1..=40u64 {
            let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let mut w = Writer::new(Fake::new(0));
            w.sink_mut().chunk = 1 + (next(&mut rng) % 90) as usize;
            let mut committed = Vec::new();
            let mut tag = 0;
            let mut ups = 0;
            for step in 0..300 {
                match next(&mut rng) % 5 {
                    0 | 1 => {
                        let n = 1 + (next(&mut rng) % 6) as u32;
                        let staged: Vec<Request> = (0..n).map(|k| create(step * 10 + k)).collect();
                        if w.submit(&staged, tag + 1).is_ok() {
                            tag += 1;
                            committed.push(tag);
                        }
                    }
                    2 => {
                        mv(&mut w, 1, 1, step as f32);
                    }
                    3 => {
                        other(&mut w, 1, 1, DrivePhase::Up, step as f32);
                        ups += 1;
                    }
                    _ => {
                        w.sink_mut().budget += (next(&mut rng) % 400) as usize;
                        w.pump().unwrap();
                    }
                }
            }
            w.sink_mut().budget = usize::MAX;
            w.pump().unwrap();
            let sent = decode(&w.sink().wire);
            let commits: Vec<u64> = sent
                .iter()
                .filter_map(|r| match r {
                    Request::Commit { tag } => Some(*tag),
                    _ => None,
                })
                .collect();
            assert_eq!(
                commits, committed,
                "seed {seed}: every accepted commit, once, in order"
            );
            let got_ups = sent
                .iter()
                .filter(|r| matches!(r, Request::MotionDrive { sample, .. } if sample.phase == DrivePhase::Up))
                .count();
            assert_eq!(got_ups, ups, "seed {seed}: no up is ever dropped");
        }
    }
}
