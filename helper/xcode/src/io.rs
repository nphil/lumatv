//! The per-child pipe work. A pipe read or write blocks for as long as the child likes, so each runs on a thread of its own (`xcode-in`, `xcode-out`,
//! `xcode-err`, 128 KiB stacks) and none of them touches scheduler state: they pass messages to the engine and units to the job's buffer.

use crate::proto::{self, Record};
use crate::types::{Au, Out, PushError};
use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) const STACK: usize = 128 << 10;
/// What may wait to be written to a child; a push beyond it is refused, never waited on.
pub(crate) const WRITE_AUS: usize = 32;
pub(crate) const WRITE_BYTES: usize = 8 << 20;
/// The last lines of a child's log kept for the status page and for failure messages.
pub(crate) const TAIL_LINES: usize = 8;
pub(crate) const LINE_MAX: usize = 512;
pub(crate) const LOG_PER_SECOND: u32 = 5;

/// When a child last sent a record, on the engine's clock: how the silence watchdog sees the reader thread without a message per record.
pub(crate) struct Heard {
    origin: Instant,
    nanos: AtomicU64,
}

impl Heard {
    pub(crate) fn new(origin: Instant) -> Self {
        Self { origin, nanos: AtomicU64::new(0) }
    }

    pub(crate) fn touch(&self) {
        let nanos = u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
        // 0 means never
        self.nanos.store(nanos.max(1), Ordering::Relaxed);
    }

    pub(crate) fn get(&self) -> Option<Duration> {
        match self.nanos.load(Ordering::Relaxed) {
            0 => None,
            nanos => Some(Duration::from_nanos(nanos)),
        }
    }
}

/// What a reader thread tells the engine about its child.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChildMsg {
    Ready,
    Error { status: u32, text: String },
    Protocol(String),
    OutClosed,
    ErrClosed,
}

pub(crate) enum Item {
    /// A record that is already whole.
    Raw(Vec<u8>),
    Au { head: [u8; proto::HEADER_LEN], body: Vec<u8> },
}

impl Item {
    fn body_len(&self) -> usize {
        match self {
            Self::Raw(_) => 0,
            Self::Au { body, .. } => body.len(),
        }
    }

    fn write_to(&self, w: &mut dyn Write) -> io::Result<()> {
        match self {
            Self::Raw(bytes) => w.write_all(bytes),
            Self::Au { head, body } => w.write_all(head).and_then(|()| w.write_all(body)),
        }
    }
}

struct Pending {
    config: Option<Item>,
    aus: VecDeque<Item>,
    bytes: usize,
    idr: bool,
    kbps: Option<u32>,
    closed: bool,
}

/// What waits to be written to a child's stdin. The configuration goes first, then control records (an IDR request and a bit rate each collapse to one
/// record however often they are asked for), then access units in order.
pub(crate) struct Writer {
    pending: Mutex<Pending>,
    wake: Condvar,
}

impl Writer {
    pub(crate) fn new(cfg: &proto::Cfg) -> Self {
        Self {
            pending: Mutex::new(Pending {
                config: Some(Item::Raw(proto::encode_config(cfg))),
                aus: VecDeque::new(),
                bytes: 0,
                idr: false,
                kbps: None,
                closed: false,
            }),
            wake: Condvar::new(),
        }
    }

    pub(crate) fn push_au(&self, au: Au) -> Result<(), PushError> {
        let len = au.data.len();
        if len > proto::MAX_PAYLOAD {
            return Err(PushError::Full);
        }
        let mut pending = self.pending.lock();
        if pending.closed || pending.aus.len() >= WRITE_AUS || pending.bytes.saturating_add(len) > WRITE_BYTES {
            return Err(PushError::Full);
        }
        pending.bytes += len;
        pending.aus.push_back(Item::Au { head: proto::encode_au_head(au.key, au.pts, len), body: au.data });
        drop(pending);
        self.wake.notify_one();
        Ok(())
    }

    pub(crate) fn idr(&self) {
        self.pending.lock().idr = true;
        self.wake.notify_one();
    }

    pub(crate) fn bitrate(&self, kbps: u32) {
        self.pending.lock().kbps = Some(kbps);
        self.wake.notify_one();
    }

    /// Drops whatever is queued and ends the writer thread; later pushes are refused.
    pub(crate) fn close(&self) {
        let mut pending = self.pending.lock();
        pending.closed = true;
        pending.aus.clear();
        pending.bytes = 0;
        drop(pending);
        self.wake.notify_all();
    }

    /// The next record to write; waits for one, and is `None` once closed.
    pub(crate) fn next(&self) -> Option<Item> {
        let mut pending = self.pending.lock();
        loop {
            if pending.closed {
                return None;
            }
            if let Some(config) = pending.config.take() {
                return Some(config);
            }
            if std::mem::take(&mut pending.idr) {
                return Some(Item::Raw(proto::encode_idr().to_vec()));
            }
            if let Some(kbps) = pending.kbps.take() {
                return Some(Item::Raw(proto::encode_bitrate(kbps).to_vec()));
            }
            if let Some(au) = pending.aus.pop_front() {
                pending.bytes = pending.bytes.saturating_sub(au.body_len());
                return Some(au);
            }
            self.wake.wait(&mut pending);
        }
    }
}

/// The `xcode-in` thread: writes until the writer is closed or the child's stdin breaks. Returning drops the pipe, which is the child's end of input.
pub(crate) fn run_writer(writer: &Writer, out: &mut dyn Write) {
    while let Some(item) = writer.next() {
        if item.write_to(out).is_err() {
            // the child is gone or has stopped reading: nothing queued will ever be written
            writer.close();
            return;
        }
    }
}

struct OutState {
    ready: bool,
    reported: bool,
}

fn handle(record: Record, state: &mut OutState, deliver: &mut impl FnMut(Out), emit: &impl Fn(ChildMsg)) -> Result<(), String> {
    match record {
        Record::Ready(ready) => {
            if state.ready {
                return Err("a second ready record".into());
            }
            if ready.version != proto::VERSION {
                return Err(format!("the converter speaks protocol version {} and the engine {}", ready.version, proto::VERSION));
            }
            state.ready = true;
            emit(ChildMsg::Ready);
        }
        Record::Out(unit) => {
            if !state.ready {
                return Err("a picture before the ready record".into());
            }
            deliver(Out { pts: unit.pts, key: unit.key, discontinuity: false, encode_tenths_ms: unit.aux, data: unit.data });
        }
        // only the first report is passed on: a child that repeats itself must not fill the engine's channel
        Record::Event(evt) if evt.kind == proto::EVT_ERROR && !state.reported => {
            state.reported = true;
            emit(ChildMsg::Error { status: evt.a, text: evt.text });
        }
        Record::Event(_) => {}
    }
    Ok(())
}

/// The `xcode-out` thread: reads the child's records until its output ends or breaks the protocol. Units go straight to `deliver`; the engine hears
/// of readiness, the child's own error report, a protocol error, and the end of the output, each at most once.
pub(crate) fn run_out(r: &mut dyn Read, heard: &Heard, mut deliver: impl FnMut(Out), emit: impl Fn(ChildMsg)) {
    let mut state = OutState { ready: false, reported: false };
    loop {
        match proto::read_record(r) {
            // a child killed mid-record is a child that ended, not one that spoke out of turn
            Ok(None) | Err(proto::Error::Io(_) | proto::Error::Truncated) => break,
            Err(e) => {
                emit(ChildMsg::Protocol(e.to_string()));
                break;
            }
            Ok(Some(record)) => {
                heard.touch();
                if let Err(why) = handle(record, &mut state, &mut deliver, &emit) {
                    emit(ChildMsg::Protocol(why));
                    break;
                }
            }
        }
    }
    emit(ChildMsg::OutClosed);
}

/// At most `max` lines a second get through; the rest are counted.
pub(crate) struct Throttle {
    max: u32,
    window: Option<Duration>,
    shown: u32,
    hidden: u32,
}

impl Throttle {
    pub(crate) fn new(max: u32) -> Self {
        Self { max, window: None, shown: 0, hidden: 0 }
    }

    /// Whether a line at `now` may be logged, and how many lines the window before this one hid.
    pub(crate) fn admit(&mut self, now: Duration) -> (bool, u32) {
        let mut hidden_before = 0;
        if self.window.is_none_or(|start| now.saturating_sub(start) >= Duration::from_secs(1)) {
            self.window = Some(now);
            self.shown = 0;
            hidden_before = std::mem::take(&mut self.hidden);
        }
        if self.shown < self.max {
            self.shown += 1;
            (true, hidden_before)
        } else {
            self.hidden += 1;
            (false, hidden_before)
        }
    }

    pub(crate) fn hidden(&self) -> u32 {
        self.hidden
    }
}

pub(crate) struct Line {
    pub(crate) text: String,
    pub(crate) show: bool,
    pub(crate) hidden_before: u32,
}

/// Cuts a child's stderr into lines of at most `LINE_MAX` bytes (the rest of a longer line is dropped) with the throttle's verdict on each.
pub(crate) struct LineLog {
    line: Vec<u8>,
    throttle: Throttle,
}

impl LineLog {
    pub(crate) fn new() -> Self {
        Self { line: Vec::with_capacity(LINE_MAX), throttle: Throttle::new(LOG_PER_SECOND) }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8], now: Duration, on_line: &mut dyn FnMut(Line)) {
        for &byte in bytes {
            if byte == b'\n' {
                self.finish(now, on_line);
            } else if self.line.len() < LINE_MAX {
                self.line.push(byte);
            }
        }
    }

    /// The line so far is complete (a newline, or the stream ended).
    pub(crate) fn finish(&mut self, now: Duration, on_line: &mut dyn FnMut(Line)) {
        let text = String::from_utf8_lossy(&self.line).trim_end().to_owned();
        self.line.clear();
        if text.is_empty() {
            return;
        }
        let (show, hidden_before) = self.throttle.admit(now);
        on_line(Line { text, show, hidden_before });
    }

    pub(crate) fn hidden(&self) -> u32 {
        self.throttle.hidden()
    }
}

/// The `xcode-err` thread: the child's log. The last `TAIL_LINES` lines are kept in `tail`, and `LOG_PER_SECOND` lines a second reach `log`.
pub(crate) fn run_err(r: &mut dyn Read, pid: u32, tail: &Mutex<VecDeque<String>>, emit: impl Fn(ChildMsg)) {
    let started = Instant::now();
    let mut lines = LineLog::new();
    let mut on_line = |line: Line| {
        if line.hidden_before > 0 {
            log::info!("lumaxcode: child {pid}: {} lines not shown", line.hidden_before);
        }
        if line.show {
            log::info!("lumaxcode: child {pid}: {}", line.text);
        }
        let mut tail = tail.lock();
        if tail.len() >= TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line.text);
    };
    let mut buf = [0u8; 1024];
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if let Some(chunk) = buf.get(..n) {
            lines.feed(chunk, started.elapsed(), &mut on_line);
        }
    }
    lines.finish(started.elapsed(), &mut on_line);
    if lines.hidden() > 0 {
        log::info!("lumaxcode: child {pid}: {} lines not shown", lines.hidden());
    }
    emit(ChildMsg::ErrClosed);
}

/// Counts the I/O threads that are running, so shutdown can wait for them for a bounded time without joining one that is stuck in a pipe.
#[derive(Default)]
pub(crate) struct Threads {
    live: Mutex<usize>,
    none: Condvar,
}

pub(crate) struct Running(Arc<Threads>);

impl Threads {
    pub(crate) fn enter(self: &Arc<Self>) -> Running {
        *self.live.lock() += 1;
        Running(Arc::clone(self))
    }

    pub(crate) fn live(&self) -> usize {
        *self.live.lock()
    }

    /// Waits until no thread is running, or the deadline; whether none is.
    pub(crate) fn wait_idle(&self, deadline: Instant) -> bool {
        let mut live = self.live.lock();
        while *live > 0 {
            if self.none.wait_until(&mut live, deadline).timed_out() {
                return *live == 0;
            }
        }
        true
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let mut live = self.0.live.lock();
        *live = live.saturating_sub(1);
        if *live == 0 {
            self.0.none.notify_all();
        }
    }
}

/// Starts a counted thread with the I/O stack size. The handle is dropped: the thread is detached and reports through the counter.
pub(crate) fn spawn(threads: &Arc<Threads>, name: &str, work: impl FnOnce() + Send + 'static) -> io::Result<()> {
    let running = threads.enter();
    thread::Builder::new().name(name.to_owned()).stack_size(STACK).spawn(move || {
        let _running = running;
        work();
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{Cfg, Command, Evt, Ready};
    use std::cell::RefCell;
    use std::sync::mpsc;

    fn au(pts: u64, len: usize) -> Au {
        Au { pts, key: pts == 0, data: vec![7; len] }
    }

    fn wire(writer: &Writer, stop: usize) -> Vec<Command> {
        let mut bytes = Vec::new();
        for _ in 0..stop {
            if let Some(item) = writer.next() {
                item.write_to(&mut bytes).unwrap();
            }
        }
        let mut commands = Vec::new();
        let mut rd = bytes.as_slice();
        while let Some(c) = proto::read_command(&mut rd).unwrap() {
            commands.push(c);
        }
        commands
    }

    #[test]
    fn the_config_goes_first_then_controls_once_then_units_in_order() {
        let cfg = Cfg { version: 1, codec: 2, kbps: 900, ..Cfg::default() };
        let writer = Writer::new(&cfg);
        writer.push_au(au(0, 3)).unwrap();
        writer.push_au(au(1, 2)).unwrap();
        for _ in 0..3 {
            writer.idr();
        }
        writer.bitrate(500);
        writer.bitrate(700);
        assert_eq!(
            wire(&writer, 5),
            [
                Command::Config(cfg),
                Command::Idr,
                Command::Bitrate(700),
                Command::Au { key: true, pts: 0, data: vec![7; 3] },
                Command::Au { key: false, pts: 1, data: vec![7; 2] },
            ]
        );
    }

    #[test]
    fn the_writer_refuses_beyond_its_bounds_and_has_room_again_after_a_write() {
        let writer = Writer::new(&Cfg::default());
        let _ = writer.next();
        for n in 0..WRITE_AUS {
            writer.push_au(au(n as u64, 1)).unwrap();
        }
        assert_eq!(writer.push_au(au(99, 1)), Err(PushError::Full));
        let _ = writer.next();
        writer.push_au(au(99, 1)).unwrap();

        let big = Writer::new(&Cfg::default());
        let _ = big.next();
        big.push_au(au(1, 3 << 20)).unwrap();
        big.push_au(au(2, 3 << 20)).unwrap();
        assert_eq!(big.push_au(au(3, 3 << 20)), Err(PushError::Full));
        assert_eq!(big.push_au(au(4, proto::MAX_PAYLOAD + 1)), Err(PushError::Full));
        let _ = big.next();
        big.push_au(au(5, 2 << 20)).unwrap();
    }

    #[test]
    fn a_closed_writer_refuses_and_ends_its_thread() {
        let writer = Writer::new(&Cfg::default());
        writer.push_au(au(0, 1)).unwrap();
        writer.close();
        assert!(writer.next().is_none());
        assert_eq!(writer.push_au(au(1, 1)), Err(PushError::Full));
        let mut sink = Vec::new();
        run_writer(&writer, &mut sink);
        assert!(sink.is_empty());
    }

    #[test]
    fn a_broken_pipe_closes_the_writer() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let writer = Writer::new(&Cfg::default());
        run_writer(&writer, &mut Broken);
        assert_eq!(writer.push_au(au(0, 1)), Err(PushError::Full));
    }

    fn run(bytes: Vec<u8>) -> (Vec<Out>, Vec<ChildMsg>, bool) {
        let units = RefCell::new(Vec::new());
        let msgs = RefCell::new(Vec::new());
        let heard = Heard::new(Instant::now());
        run_out(&mut bytes.as_slice(), &heard, |u| units.borrow_mut().push(u), |m| msgs.borrow_mut().push(m));
        (units.into_inner(), msgs.into_inner(), heard.get().is_some())
    }

    fn ready() -> Vec<u8> {
        proto::encode_ready(&Ready { version: proto::VERSION, max_w: 1920, max_h: 1088, out_size: 1 << 20, ncap: 6, hma_bytes: 0 })
    }

    fn error(status: u32, text: &str) -> Vec<u8> {
        proto::encode_event(&Evt { kind: proto::EVT_ERROR, a: status, text: text.into(), ..Evt::default() })
    }

    #[test]
    fn the_reader_delivers_units_and_reports_ready_the_error_and_the_end() {
        let mut wire = ready();
        wire.extend(proto::encode_out(true, 90_000, 25, &[1, 2, 3]));
        wire.extend(proto::encode_event(&Evt { kind: proto::EVT_STATS, ..Evt::default() }));
        wire.extend(error(4, "decoder: no memory"));
        wire.extend(error(4, "again"));
        let (units, msgs, heard) = run(wire);
        assert_eq!(units, [Out { pts: 90_000, key: true, discontinuity: false, encode_tenths_ms: 25, data: vec![1, 2, 3] }]);
        assert_eq!(msgs, [ChildMsg::Ready, ChildMsg::Error { status: 4, text: "decoder: no memory".into() }, ChildMsg::OutClosed]);
        assert!(heard);
    }

    #[test]
    fn a_unit_before_ready_a_second_ready_a_wrong_version_and_garbage_are_protocol_errors() {
        let early = proto::encode_out(true, 0, 0, &[1]);
        let mut twice = ready();
        twice.extend(ready());
        let old = proto::encode_ready(&Ready { version: 2, ..Ready::default() });
        let mut garbage = ready();
        garbage.extend([0x5A; 40]);
        for (wire, what) in [(early, "before the ready"), (twice, "second ready"), (old, "protocol version 2"), (garbage, "unknown record")] {
            let (units, msgs, _) = run(wire);
            assert!(units.is_empty());
            let Some(ChildMsg::Protocol(why)) = msgs.iter().find(|m| matches!(m, ChildMsg::Protocol(_))) else { panic!("no protocol error for {what}: {msgs:?}") };
            assert!(why.contains(what), "{why} for {what}");
            assert_eq!(msgs.last(), Some(&ChildMsg::OutClosed));
        }
    }

    #[test]
    fn a_stream_that_ends_inside_a_record_is_an_end_not_an_error() {
        let mut wire = ready();
        let mut cut = proto::encode_out(true, 1, 1, &[1, 2, 3, 4]);
        cut.truncate(proto::HEADER_LEN + 2);
        wire.extend(cut);
        let (units, msgs, _) = run(wire);
        assert!(units.is_empty());
        assert_eq!(msgs, [ChildMsg::Ready, ChildMsg::OutClosed]);
    }

    fn lines(chunks: &[&[u8]], at: &[Duration]) -> Vec<(String, bool, u32)> {
        let mut log = LineLog::new();
        let mut got = Vec::new();
        let mut on_line = |l: Line| got.push((l.text, l.show, l.hidden_before));
        for (chunk, now) in chunks.iter().zip(at) {
            log.feed(chunk, *now, &mut on_line);
        }
        log.finish(at.last().copied().unwrap_or_default(), &mut on_line);
        got
    }

    #[test]
    fn log_lines_are_cut_from_chunks_trimmed_and_capped() {
        let t = Duration::ZERO;
        let long = vec![b'x'; 2000];
        let got = lines(&[b"one\r\ntw", b"o\n\n   \nthr", b"ee\n", &long, b"\nlast"], &[t; 5]);
        let text: Vec<&str> = got.iter().map(|g| g.0.as_str()).collect();
        assert_eq!(text[..3], ["one", "two", "three"]);
        assert_eq!(text[3].len(), LINE_MAX);
        assert_eq!(text[4..], ["last"]);
    }

    #[test]
    fn the_throttle_lets_five_lines_a_second_through_and_counts_the_rest() {
        let mut t = Throttle::new(5);
        let s = Duration::from_secs;
        let ms = Duration::from_millis;
        let mut shown = 0;
        for i in 0..100 {
            if t.admit(ms(i)).0 {
                shown += 1;
            }
        }
        assert_eq!((shown, t.hidden()), (5, 95));
        // the next window reports what the last one hid, and lets five more through
        assert_eq!(t.admit(s(1)), (true, 95));
        assert_eq!(t.admit(s(1) + ms(1)), (true, 0));
        assert_eq!(t.hidden(), 0);
    }

    #[test]
    fn the_stderr_tail_keeps_the_last_eight_lines() {
        let tail = Mutex::new(VecDeque::new());
        let log: String = (0..50).map(|n| format!("line {n}\n")).collect();
        let (tx, rx) = mpsc::channel();
        run_err(&mut log.as_bytes(), 1, &tail, |m| tx.send(m).unwrap());
        assert_eq!(rx.recv().unwrap(), ChildMsg::ErrClosed);
        let tail: Vec<String> = tail.lock().iter().cloned().collect();
        assert_eq!(tail.len(), TAIL_LINES);
        assert_eq!(tail.first().map(String::as_str), Some("line 42"));
        assert_eq!(tail.last().map(String::as_str), Some("line 49"));
    }

    #[test]
    fn the_thread_counter_reaches_zero_when_the_threads_finish() {
        let threads = Arc::new(Threads::default());
        assert!(threads.wait_idle(Instant::now()));
        let (release, hold) = mpsc::channel::<()>();
        spawn(&threads, "test-io", move || {
            let _ = hold.recv();
        })
        .unwrap();
        assert_eq!(threads.live(), 1);
        release.send(()).unwrap();
        assert!(threads.wait_idle(Instant::now() + Duration::from_secs(10)));
        assert_eq!(threads.live(), 0);
    }
}
