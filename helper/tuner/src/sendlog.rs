//! What a client's stream is sent at, for the log: one line every [`EVERY`] and one at the end (channel, bytes a second over the window,
//! the longest gap between two writes, the bytes the socket still holds unsent), and a warning at once when a write came more than
//! [`GAP`] after the one before it (a client that has stopped reading blocks the write; the PAT goes out ten times a second, so a gap
//! that long is the socket's, not the station's). At most one warning each [`WARN_EVERY`], so a stalled client is a few lines, not a flood.

use crate::stream::Clock;
use std::cell::Cell;
use std::io::{self, Write};
use std::os::fd::RawFd;
use std::time::Duration;

pub(crate) const EVERY: Duration = Duration::from_secs(10);
pub(crate) const GAP: Duration = Duration::from_secs(1);
pub(crate) const WARN_EVERY: Duration = Duration::from_secs(10);
/// After a slate-to-station splice the bytes written are counted in steps of [`STEP`] for [`BURST`], and told in one line.
pub(crate) const BURST: Duration = Duration::from_secs(6);
pub(crate) const STEP: Duration = Duration::from_millis(100);
const STEPS: usize = (BURST.as_millis() / STEP.as_millis()) as usize;

thread_local! {
    /// Set by the splice, on the thread that serves the stream, and taken by that stream's writer at its next write.
    static SPLICED: Cell<bool> = const { Cell::new(false) };
}

/// The stream on this thread has just been spliced from a slate to the station's programme.
pub(crate) fn note_splice() {
    SPLICED.with(|s| s.set(true));
}

fn take_splice() -> bool {
    SPLICED.with(Cell::take)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Line {
    Info(String),
    Warn(String),
}

impl Line {
    fn log(&self) {
        match self {
            Self::Info(text) => log::info!("{text}"),
            Self::Warn(text) => log::warn!("{text}"),
        }
    }
}

/// The accounting of one stream, on the caller's clock.
pub(crate) struct Window {
    label: String,
    start: Duration,
    last: Duration,
    bytes: u64,
    worst: Duration,
    warned: Option<Duration>,
    burst: Option<Burst>,
}

/// The bytes written in each [`STEP`] since a splice, and the longest gap between writes meanwhile.
struct Burst {
    from: Duration,
    bytes: [u64; STEPS],
    worst: Duration,
}

impl Window {
    pub(crate) fn new(label: String, now: Duration) -> Self {
        Self { label, start: now, last: now, bytes: 0, worst: Duration::ZERO, warned: None, burst: None }
    }

    /// The stream was spliced from a slate to the station at `now`: its next [`BURST`] is counted in steps (a splice before the last one is
    /// over starts it again; that one's line is lost with it).
    pub(crate) fn spliced(&mut self, now: Duration) {
        self.burst = Some(Burst { from: now, bytes: [0; STEPS], worst: Duration::ZERO });
    }

    /// A write of `n` bytes finished at `now`. `queued` is asked only when a line is due.
    pub(crate) fn wrote(&mut self, now: Duration, n: usize, queued: &dyn Fn() -> u64) -> Vec<Line> {
        let gap = now.saturating_sub(self.last);
        self.last = now;
        self.bytes += n as u64;
        self.worst = self.worst.max(gap);
        let mut lines = Vec::new();
        if let Some(b) = &mut self.burst {
            let at = now.saturating_sub(b.from);
            if at < BURST {
                b.bytes[(at.as_millis() / STEP.as_millis()) as usize] += n as u64;
                b.worst = b.worst.max(gap);
            } else {
                lines.push(self.burst_line());
            }
        }
        if gap > GAP && self.warned.is_none_or(|at| now.saturating_sub(at) >= WARN_EVERY) {
            self.warned = Some(now);
            lines.push(Line::Warn(format!("tuner: {}: no write for {} ms, {} bytes queued", self.label, gap.as_millis(), queued())));
        }
        if now.saturating_sub(self.start) >= EVERY {
            lines.push(self.report(now, queued()));
        }
        lines
    }

    /// The lines for the stream's end: the window since the last line, and a splice's steps if its [`BURST`] was not over.
    pub(crate) fn finish(&mut self, now: Duration, queued: &dyn Fn() -> u64) -> Vec<Line> {
        let mut lines: Vec<Line> = self.burst.is_some().then(|| self.burst_line()).into_iter().collect();
        lines.push(self.report(now, queued()));
        lines
    }

    /// The splice's steps as one line: the bytes written in each 100 ms from the splice, in order, and the longest gap between writes.
    fn burst_line(&mut self) -> Line {
        let b = self.burst.take().expect("a burst");
        let steps: Vec<String> = b.bytes.iter().map(u64::to_string).collect();
        Line::Info(format!("tuner: {}: bytes per 100 ms for 6 s from the splice: {}; longest gap {} ms", self.label, steps.join(" "), b.worst.as_millis()))
    }

    fn report(&mut self, now: Duration, queued: u64) -> Line {
        let span = now.saturating_sub(self.start);
        let rate = if span.is_zero() { 0 } else { (self.bytes as f64 / span.as_secs_f64()) as u64 };
        let line = Line::Info(format!(
            "tuner: {}: {rate} B/s over {:.1} s, longest gap {} ms, {queued} bytes queued",
            self.label,
            span.as_secs_f64(),
            self.worst.as_millis()
        ));
        self.start = now;
        self.bytes = 0;
        self.worst = Duration::ZERO;
        line
    }
}

/// A writer that reports what goes through it as [`Window`] says, and asks the socket (`fd`) for its unsent bytes when a line is due.
pub(crate) struct Sent<'a> {
    out: &'a mut dyn Write,
    fd: RawFd,
    clock: Clock,
    window: Window,
}

impl<'a> Sent<'a> {
    pub(crate) fn new(out: &'a mut dyn Write, fd: RawFd, clock: Clock, label: String) -> Self {
        let now = clock();
        Self { out, fd, clock, window: Window::new(label, now) }
    }

    /// Logs the last window; the stream is over.
    pub(crate) fn finish(mut self) {
        let fd = self.fd;
        for line in self.window.finish((self.clock)(), &|| unsent(fd)) {
            line.log();
        }
    }
}

impl Write for Sent<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let before = (self.clock)();
        if take_splice() {
            self.window.spliced(before);
        }
        let n = self.out.write(buf)?;
        let fd = self.fd;
        for line in self.window.wrote((self.clock)(), n, &|| unsent(fd)) {
            line.log();
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// The bytes the socket holds that the client has not acknowledged (`SIOCOUTQ`); 0 when it cannot say.
fn unsent(fd: RawFd) -> u64 {
    let mut n: libc::c_int = 0;
    // SAFETY: SIOCOUTQ writes one c_int through the pointer, which is to a live local.
    let ok = unsafe { libc::ioctl(fd, libc::TIOCOUTQ, &mut n) } == 0;
    if ok { n.max(0) as u64 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_line_comes_each_window_with_the_rate_and_the_longest_gap_and_one_at_the_end() {
        let mut w = Window::new("p 2.4".into(), ms(0));
        assert!(w.wrote(ms(100), 1000, &|| 0).is_empty());
        assert!(w.wrote(ms(900), 1000, &|| 0).is_empty(), "a gap under a second is no warning");
        assert!(!w.wrote(ms(9_000), 8000, &|| 0).is_empty(), "8.1 s of nothing is a warning");
        let lines = w.wrote(ms(10_000), 0, &|| 4096);
        assert_eq!(lines, [Line::Info("tuner: p 2.4: 1000 B/s over 10.0 s, longest gap 8100 ms, 4096 bytes queued".into())]);
        assert!(w.wrote(ms(10_500), 500, &|| 0).is_empty());
        assert_eq!(w.finish(ms(12_000), &|| 7), [Line::Info("tuner: p 2.4: 250 B/s over 2.0 s, longest gap 500 ms, 7 bytes queued".into())]);
    }

    #[test]
    fn a_gap_over_a_second_warns_at_once_and_at_most_once_in_ten_seconds() {
        let mut w = Window::new("p 2.4".into(), ms(0));
        let first = w.wrote(ms(1_500), 10, &|| 65_536);
        assert_eq!(first, [Line::Warn("tuner: p 2.4: no write for 1500 ms, 65536 bytes queued".into())]);
        assert!(w.wrote(ms(3_000), 10, &|| 0).is_empty(), "a second stall within ten seconds is not told again");
        assert!(w.wrote(ms(4_600), 10, &|| 0).is_empty());
        let later = w.wrote(ms(12_000), 10, &|| 0);
        assert!(matches!(&later[..], [Line::Warn(t), Line::Info(_)] if t.contains("7400 ms")), "{later:?}");
    }

    #[test]
    fn six_seconds_after_a_splice_the_bytes_of_each_100_ms_come_as_one_line_with_the_longest_gap() {
        let mut w = Window::new("p 2.4".into(), ms(0));
        for at in [500, 1_000, 1_500, 1_990] {
            assert!(w.wrote(ms(at), 1, &|| 0).is_empty());
        }
        w.spliced(ms(2_000));
        for (at, n) in [(2_050, 100), (2_090, 50), (2_100, 7), (2_950, 9)] {
            assert!(w.wrote(ms(at), n, &|| 0).is_empty());
        }
        let lines = w.wrote(ms(8_000), 5, &|| 0);
        let burst = lines.iter().find_map(|l| match l {
            Line::Info(t) => t.strip_prefix("tuner: p 2.4: bytes per 100 ms for 6 s from the splice: "),
            Line::Warn(_) => None,
        });
        let (numbers, gap) = burst.expect("the burst's line").split_once("; ").unwrap();
        let steps: Vec<u64> = numbers.split(' ').map(|n| n.parse().unwrap()).collect();
        assert_eq!(steps.len(), 60);
        assert_eq!((steps[0], steps[1], steps[9], steps[2]), (150, 7, 9, 0), "{steps:?}");
        assert_eq!(gap, "longest gap 850 ms", "the write that closed the burst is not in it");
    }

    #[test]
    fn a_splice_whose_six_seconds_are_not_over_when_the_stream_ends_is_told_then() {
        let mut w = Window::new("p 2.4".into(), ms(0));
        w.spliced(ms(100));
        w.wrote(ms(200), 10, &|| 0);
        let lines = w.finish(ms(300), &|| 0);
        assert_eq!(lines.len(), 2);
        assert!(matches!(&lines[0], Line::Info(t) if t.contains("from the splice") && t.ends_with("longest gap 200 ms")));
    }
}
