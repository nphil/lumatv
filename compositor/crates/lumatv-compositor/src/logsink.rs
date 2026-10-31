//! Log file writes belong to a writer thread, not the frame loop. Round 17's trace caught the loop asleep under `write` to the
//! supervisor's ext4 `compositor.log` (`wait_transaction_locked` and `io_schedule`). Logging now formats into a fixed buffer and
//! tries a bounded queue without waiting for room. A full queue drops file lines with a count; the synchronous trace ring still
//! keeps them for the crash bundle. Timestamps are captured before queuing, so the gate can still pair them with client swaps.
//!
//! The writer batches queued lines. A flush is a marker in the same queue, acknowledged only after the preceding lines have been
//! written or a write has failed. Normal exit and the panic hook wait at most 500 ms; no flush is made on the steady-state loop.

use std::fmt::{self, Write as _};
use std::io;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::output::monotonic_ns;
use crate::ring;

/// Includes the newline and truncation marker. Round 17's 19,376 log lines reached 793 bytes (a client's bound interfaces and command).
pub const LINE_MAX: usize = 1024;
/// Fixed queue storage: about 256 KiB. App startup bursts are tens of lines.
pub const QUEUE_LINES: usize = 256;
const BATCH_MAX: usize = 32 * 1024;
const FLUSH_WAIT: Duration = Duration::from_millis(500);

struct Line {
    len: usize,
    cut: bool,
    buf: [u8; LINE_MAX],
}

impl Line {
    fn new() -> Line {
        Line { len: 0, cut: false, buf: [0; LINE_MAX] }
    }

    fn bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    fn end(&mut self) {
        if self.cut {
            self.buf[self.len..self.len + 3].copy_from_slice("…".as_bytes());
            self.len += 3;
        }
        self.buf[self.len] = b'\n';
        self.len += 1;
    }
}

impl fmt::Write for Line {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = LINE_MAX - 4 - self.len;
        if s.len() <= room {
            self.buf[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
            self.len += s.len();
            return Ok(());
        }
        let mut take = room;
        while !s.is_char_boundary(take) {
            take -= 1;
        }
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        self.cut = true;
        Err(fmt::Error)
    }
}

fn level_name(level: log::Level) -> &'static str {
    match level {
        log::Level::Error => "error",
        log::Level::Warn => "warn",
        log::Level::Info => "info",
        log::Level::Debug => "debug",
        log::Level::Trace => "trace",
    }
}

/// Fixed-size entries avoid allocating a string for each log call. Only an explicit flush allocates an acknowledgement.
struct Entry {
    line: Line,
    flushed: Option<Arc<AtomicBool>>,
}

#[derive(Default)]
struct Dropped {
    pending: AtomicU64,
    total: AtomicU64,
    /// Bytes written to the file since it was last emptied.
    written: AtomicU64,
}

/// The file is emptied once this much has been written to it: a debug run with `LUMATV_LOG=debug` or a flood of one warning would
/// otherwise grow the supervisor's `compositor.log` on /var for as long as the compositor runs.
pub const FILE_MAX: u64 = 8 << 20;

/// A bounded, nonblocking file-log producer. `fd` is borrowed and must remain open until the logger is dropped.
pub struct Logger {
    tx: Option<SyncSender<Entry>>,
    dropped: Arc<Dropped>,
    writer: Option<JoinHandle<()>>,
}

impl Logger {
    /// Call before the loop takes its real-time priority: the writer inherits the normal scheduling policy.
    pub fn start(fd: RawFd) -> io::Result<Logger> {
        let (tx, rx) = sync_channel(QUEUE_LINES);
        let dropped = Arc::new(Dropped::default());
        let writer = {
            let dropped = Arc::clone(&dropped);
            thread::Builder::new().name("lumatv-log".into()).stack_size(64 << 10).spawn(move || write_loop(rx, fd, &dropped))?
        };
        Ok(Logger { tx: Some(tx), dropped, writer: Some(writer) })
    }

    /// File lines refused because the queue was full. They are still copied to the trace ring.
    pub fn refused(&self) -> u64 {
        self.dropped.total.load(Ordering::Relaxed)
    }

    /// Waits up to `wait` for the file writer to attempt every line before the marker. Concurrent producers cannot acknowledge it early.
    pub fn flush_within(&self, wait: Duration) -> bool {
        let Some(tx) = &self.tx else { return false };
        let flushed = Arc::new(AtomicBool::new(false));
        let mut marker = Entry { line: Line::new(), flushed: Some(Arc::clone(&flushed)) };
        let end = Instant::now() + wait;
        loop {
            match tx.try_send(marker) {
                Ok(()) => break,
                Err(TrySendError::Full(entry)) => marker = entry,
                Err(TrySendError::Disconnected(_)) => return false,
            }
            if Instant::now() >= end {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
        while !flushed.load(Ordering::Acquire) {
            if Instant::now() >= end {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
        true
    }
}

impl log::Log for Logger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let level = level_name(record.level());
        let mut line = Line::new();
        let _ = write!(line, "[srv {:10.3}] {level} {}", monotonic_ns() as f64 / 1e9, record.args());
        line.end();
        if let Some(tx) = &self.tx
            && let Err(TrySendError::Full(_)) = tx.try_send(Entry { line, flushed: None })
        {
            self.dropped.pending.fetch_add(1, Ordering::Relaxed);
            self.dropped.total.fetch_add(1, Ordering::Relaxed);
        }
        // The ring stays synchronous: even a full file queue cannot remove the last events from a crash bundle.
        ring::note(format_args!("{level} {}", record.args()));
    }

    fn flush(&self) {
        self.flush_within(FLUSH_WAIT);
    }
}

impl Drop for Logger {
    fn drop(&mut self) {
        // Disconnect only after the last producer has gone; recv drains queued entries before returning Disconnected.
        drop(self.tx.take());
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

/// All of `buf` to `fd`; failed writes lose file lines as the old stderr write did. The trace ring is unaffected.
fn write_all(fd: RawFd, mut buf: &[u8]) {
    while !buf.is_empty() {
        // SAFETY: write reads at most the length of this live slice.
        let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
        if n < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if n <= 0 {
            return;
        }
        buf = &buf[n as usize..];
    }
}

fn write_batch(fd: RawFd, dropped: &Dropped, batch: &mut Vec<u8>) {
    let n = dropped.pending.swap(0, Ordering::Relaxed);
    if n > 0 {
        let mut note = Line::new();
        let _ = write!(note, "[srv {:10.3}] warn log: {n} line(s) dropped, the file was not taking them", monotonic_ns() as f64 / 1e9);
        note.end();
        batch.extend_from_slice(note.bytes());
    }
    if !batch.is_empty() {
        if dropped.written.load(Ordering::Relaxed) + batch.len() as u64 > FILE_MAX {
            empty_file(fd);
            dropped.written.store(0, Ordering::Relaxed);
        }
        write_all(fd, batch);
        dropped.written.fetch_add(batch.len() as u64, Ordering::Relaxed);
        batch.clear();
    }
}

/// Back to the start of an empty file; a descriptor that is not a file (a terminal, a pipe) refuses both and is left as it is.
fn empty_file(fd: RawFd) {
    // SAFETY: plain calls on a descriptor the logger borrows.
    unsafe {
        if libc::ftruncate(fd, 0) == 0 {
            libc::lseek(fd, 0, libc::SEEK_SET);
        }
    }
}

fn append(entry: Entry, fd: RawFd, dropped: &Dropped, batch: &mut Vec<u8>) {
    if let Some(flushed) = entry.flushed {
        write_batch(fd, dropped, batch);
        flushed.store(true, Ordering::Release);
    } else {
        batch.extend_from_slice(entry.line.bytes());
        if batch.len() >= BATCH_MAX {
            write_batch(fd, dropped, batch);
        }
    }
}

fn write_loop(rx: Receiver<Entry>, fd: RawFd, dropped: &Dropped) {
    // One oversized line and a drop note fit beyond the batch threshold without growing the buffer.
    let mut batch = Vec::with_capacity(BATCH_MAX + 2 * LINE_MAX);
    while let Ok(entry) = rx.recv() {
        append(entry, fd, dropped, &mut batch);
        while let Ok(entry) = rx.try_recv() {
            append(entry, fd, dropped, &mut batch);
        }
        write_batch(fd, dropped, &mut batch);
    }
    write_batch(fd, dropped, &mut batch);
}

#[cfg(test)]
mod size_tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn a_flood_never_takes_the_file_past_its_cap() {
        let path = std::env::temp_dir().join(format!("lumatv-logsink-{}", std::process::id()));
        let f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&path).unwrap();
        let dropped = Dropped::default();
        let mut batch = Vec::new();
        let mut worst = 0;
        for _ in 0..(3 * FILE_MAX / 4096) {
            batch.extend_from_slice(&[b'x'; 4096]);
            write_batch(f.as_raw_fd(), &dropped, &mut batch);
            worst = worst.max(std::fs::metadata(&path).unwrap().len());
        }
        let _ = std::fs::remove_file(&path);
        assert!(worst <= FILE_MAX, "{worst}");
    }
}
