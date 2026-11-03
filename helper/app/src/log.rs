//! Fixed-size, nonblocking producer for the helper-owned RAM collector.
use std::fmt::{self, Write};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level { Error, Warn, Info, Debug }
impl Level {
    pub fn parse(word: &str) -> Option<Self> {
        match word { "error" => Some(Self::Error), "warn" => Some(Self::Warn), "info" => Some(Self::Info), "debug" => Some(Self::Debug), _ => None }
    }
    fn priority(self) -> u8 { match self { Self::Error => 11, Self::Warn => 12, Self::Info => 14, Self::Debug => 15 } }
}

struct Record { bytes: [u8; crate::logging::RECORD_LIMIT], len: usize }
impl Record {
    fn new() -> Self { Self { bytes: [0; crate::logging::RECORD_LIMIT], len: 0 } }
}
impl fmt::Write for Record {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let mut count = text.len().min(self.bytes.len() - self.len);
        while !text.is_char_boundary(count) { count -= 1; }
        for byte in &text.as_bytes()[..count] {
            self.bytes[self.len] = if byte.is_ascii_control() { b' ' } else { *byte };
            self.len += 1;
        }
        Ok(())
    }
}

pub struct Logger { socket: UnixDatagram, destination: PathBuf, level: Level }
impl Logger {
    pub fn new(level: Level) -> std::io::Result<Self> { Self::at(Path::new(crate::logging::SOCKET), level) }
    fn at(destination: &Path, level: Level) -> std::io::Result<Self> {
        let socket = UnixDatagram::unbound()?;
        socket.set_nonblocking(true)?;
        crate::logging::socket_buffer(&socket)?;
        Ok(Self { socket, destination: destination.to_owned(), level })
    }
    pub fn log(&self, level: Level, message: fmt::Arguments<'_>) {
        if level > self.level { return; }
        let mut record = Record::new();
        let _ = write!(&mut record, "<{}>lumatv-helper: {} {}", level.priority(), crate::timeutil::now_ms(), message);
        let _ = self.socket.send_to(&record.bytes[..record.len], &self.destination);
    }
}
static LOGGER: OnceLock<Logger> = OnceLock::new();
pub fn init(logger: Logger) { let _ = LOGGER.set(logger); }
pub fn log(level: Level, message: fmt::Arguments<'_>) { if let Some(logger) = LOGGER.get() { logger.log(level, message); } }
#[macro_export]
macro_rules! log_error { ($($arg:tt)*) => { $crate::log::log($crate::log::Level::Error, format_args!($($arg)*)) }; }
#[macro_export]
macro_rules! log_warn { ($($arg:tt)*) => { $crate::log::log($crate::log::Level::Warn, format_args!($($arg)*)) }; }
#[macro_export]
macro_rules! log_info { ($($arg:tt)*) => { $crate::log::log($crate::log::Level::Info, format_args!($($arg)*)) }; }
#[macro_export]
macro_rules! log_debug { ($($arg:tt)*) => { $crate::log::log($crate::log::Level::Debug, format_args!($($arg)*)) }; }

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn actual_producer_bounds_filters_and_drops_when_full() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.sock");
        let input = UnixDatagram::bind(&path).unwrap();
        input.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let logger = Logger::at(&path, Level::Info).unwrap();
        logger.log(Level::Debug, format_args!("filtered"));
        logger.log(Level::Info, format_args!("tagged-smoke\n{}", "é".repeat(8192)));
        let mut bytes = [0; crate::logging::RECORD_LIMIT + 1];
        let n = input.recv(&mut bytes).unwrap();
        assert!(n <= crate::logging::RECORD_LIMIT);
        assert!(std::str::from_utf8(&bytes[..n]).unwrap().contains("tagged-smoke "));
        assert!(bytes[..n].starts_with(b"<14>lumatv-helper:"));
        let started = Instant::now();
        for _ in 0..10000 { logger.log(Level::Warn, format_args!("full")); }
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(input);
        logger.log(Level::Error, format_args!("unavailable"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
