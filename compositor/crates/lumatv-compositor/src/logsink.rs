//! Fixed stack records sent once to the RAM collector. Missing or full collectors drop records; neither the frame loop nor
//! shutdown waits for logging, and no producer opens a retained log file.

use std::fmt::{self, Write as _};
use std::io::{self, BufRead};
use std::os::unix::net::UnixDatagram;
use std::sync::LazyLock;

use crate::output::monotonic_ns;

pub const LINE_MAX: usize = 4096;
pub const SOCKET: &str = "/tmp/lumatv-log.sock";
pub const DIRECTORY: &str = "/tmp/lumatv-logs";
static SOCKET_FD: LazyLock<Option<UnixDatagram>> = LazyLock::new(|| UnixDatagram::unbound().and_then(|s| { s.set_nonblocking(true)?; Ok(s) }).ok());

pub(crate) struct Line {
    pub(crate) buf: [u8; LINE_MAX],
    pub(crate) len: usize,
    cut: bool,
}

impl Line {
    pub(crate) fn new() -> Self {
        Self { buf: [0; LINE_MAX], len: 0, cut: false }
    }

    pub(crate) fn bytes(&mut self) -> &[u8] {
        if self.cut {
            self.buf[self.len..self.len + 3].copy_from_slice("…".as_bytes());
            self.len += 3;
            self.cut = false;
        }
        &self.buf[..self.len]
    }
}

impl fmt::Write for Line {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.cut { return Err(fmt::Error); }
        for part in text.split_inclusive(|c: char| c.is_control()) {
            let last = part.chars().next_back();
            let control = last.filter(|c| c.is_control());
            let printable = &part[..part.len() - control.map_or(0, char::len_utf8)];
            let mut take = printable.len().min(LINE_MAX - 3 - self.len);
            while !printable.is_char_boundary(take) { take -= 1; }
            self.buf[self.len..self.len + take].copy_from_slice(&printable.as_bytes()[..take]);
            self.len += take;
            if take < printable.len() { self.cut = true; return Err(fmt::Error); }
            if control.is_some() {
                if self.len == LINE_MAX - 3 { self.cut = true; return Err(fmt::Error); }
                self.buf[self.len] = b' ';
                self.len += 1;
            }
        }
        Ok(())
    }
}

fn socket() -> Option<&'static UnixDatagram> {
    SOCKET_FD.as_ref()
}

/// Initializes the descriptor before the real-time frame loop; send_to also reconnects naturally after a collector restart.
pub fn init() { let _ = socket(); }

pub(crate) fn send(tag: &str, severity: u8, args: fmt::Arguments<'_>) {
    let Some(socket) = socket() else { return };
    let mut line = Line::new();
    let _ = write!(line, "<{}>{tag}: {args}", 128 + severity.min(7));
    let _ = socket.send_to(line.bytes(), SOCKET);
}

pub struct Logger;
impl Logger {
    pub fn start() -> Self { init(); Self }
}

impl log::Log for Logger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool { true }
    fn log(&self, record: &log::Record<'_>) {
        let (severity, level) = match record.level() {
            log::Level::Error => (3, "error"),
            log::Level::Warn => (4, "warn"),
            log::Level::Info => (6, "info"),
            log::Level::Debug => (7, "debug"),
            log::Level::Trace => (7, "trace"),
        };
        send("lumatv-compositor", severity, format_args!("[srv {:10.3}] {level} {}", monotonic_ns() as f64 / 1e9, record.args()));
        crate::ring::note(format_args!("{level} {}", record.args()));
    }
    fn flush(&self) {}
}

/// The central reader strips only the protocol header, preserving producer timestamps and message formatting.
pub fn tail(tag: &str, count: usize) -> Vec<String> {
    tail_in(std::path::Path::new(DIRECTORY), tag, count)
}

pub fn tail_in(directory: &std::path::Path, tag: &str, count: usize) -> Vec<String> {
    let mut lines = std::collections::VecDeque::with_capacity(count.min(4096));
    if count == 0 { return Vec::new(); }
    for name in ["previous.log", "current.log"] {
        let Ok(file) = std::fs::File::open(directory.join(name)) else { continue };
        let mut reader = io::BufReader::new(file).take(524288);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) { Ok(0) | Err(_) => break, Ok(_) => {} }
            let Some((pri, message)) = line.strip_prefix('<').and_then(|s| s.split_once('>')) else { continue };
            if pri.is_empty() || !pri.bytes().all(|c| c.is_ascii_digit()) || !pri.parse::<u8>().is_ok_and(|n| n <= 191) { continue; }
            let Some(message) = message.strip_prefix(tag).and_then(|s| s.strip_prefix(": ")) else { continue };
            if lines.len() == count { lines.pop_front(); }
            lines.push_back(message.trim_end_matches('\n').to_owned());
        }
    }
    lines.into_iter().collect()
}

use std::io::Read as _;
