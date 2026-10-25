//! The helper's log: one file in the data dir, size-capped, one line per event.
//!
//! The data dir is the set's flash, so the log is small on purpose: when the file passes its cap it becomes `<name>.1` (replacing
//! the previous one) and a new file starts, which bounds the log to twice the cap. Lines carry UTC stamps. Components log
//! changes and failures, never every poll.

use crate::timeutil::{now_ms, utc_stamp};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// How much a line matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    fn tag(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN ",
            Self::Info => "INFO ",
            Self::Debug => "DEBUG",
        }
    }

    /// `error`, `warn`, `info` or `debug`.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "error" => Some(Self::Error),
            "warn" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            _ => None,
        }
    }
}

struct Sink {
    file: Option<File>,
    path: PathBuf,
    size: u64,
}

/// A log file with a level and a size cap. Safe to share between threads.
pub struct Logger {
    sink: Mutex<Sink>,
    level: Level,
    max_bytes: u64,
    stderr: bool,
}

impl Logger {
    /// Opens (appending) `path`, creating its directory. With `stderr` every line is also written to standard error.
    pub fn open(path: &Path, level: Level, max_bytes: u64, stderr: bool) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let size = file.metadata().map_or(0, |m| m.len());
        Ok(Self {
            sink: Mutex::new(Sink {
                file: Some(file),
                path: path.to_owned(),
                size,
            }),
            level,
            max_bytes: max_bytes.max(1024),
            stderr,
        })
    }

    /// Writes one line if `level` is enabled.
    pub fn log(&self, level: Level, message: fmt::Arguments<'_>) {
        self.log_at(now_ms(), level, message);
    }

    fn log_at(&self, at_ms: u64, level: Level, message: fmt::Arguments<'_>) {
        if level > self.level {
            return;
        }
        let line = format!("{} {} {}\n", utc_stamp(at_ms), level.tag(), message);
        if self.stderr {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
        let Ok(mut sink) = self.sink.lock() else {
            return;
        };
        let len = line.len() as u64;
        if sink.size + len > self.max_bytes {
            sink.file = None;
            let old = rotated(&sink.path);
            let _ = fs::rename(&sink.path, old);
            sink.file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&sink.path)
                .ok();
            sink.size = 0;
        }
        if let Some(file) = sink.file.as_mut()
            && file.write_all(line.as_bytes()).is_ok()
        {
            sink.size += len;
        }
    }
}

/// `<path>.1`
fn rotated(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Installs the process-wide logger. The first call wins; later ones are ignored.
pub fn init(logger: Logger) {
    let _ = LOGGER.set(logger);
}

/// Logs through the process-wide logger; nothing is written before [`init`].
pub fn log(level: Level, message: fmt::Arguments<'_>) {
    if let Some(logger) = LOGGER.get() {
        logger.log(level, message);
    }
}

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

    #[test]
    fn lines_carry_a_utc_stamp_and_level() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log-helper");
        let logger = Logger::open(&path, Level::Info, 64 * 1024, false).unwrap();
        logger.log_at(1_700_000_000_000, Level::Info, format_args!("hello {}", 7));
        logger.log_at(1_700_000_001_000, Level::Warn, format_args!("careful"));
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "2023-11-14T22:13:20Z INFO  hello 7\n2023-11-14T22:13:21Z WARN  careful\n"
        );
    }

    #[test]
    fn levels_below_the_setting_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log-helper");
        let logger = Logger::open(&path, Level::Warn, 64 * 1024, false).unwrap();
        logger.log(Level::Info, format_args!("quiet"));
        logger.log(Level::Debug, format_args!("quieter"));
        logger.log(Level::Error, format_args!("loud"));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("loud") && !text.contains("quiet"));
    }

    #[test]
    fn a_full_file_becomes_dot_one_and_the_log_stays_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log-helper");
        let logger = Logger::open(&path, Level::Info, 1024, false).unwrap();
        for i in 0..100 {
            logger.log(
                Level::Info,
                format_args!("line number {i:04} {}", "x".repeat(40)),
            );
        }
        let current = fs::metadata(&path).unwrap().len();
        let old = fs::metadata(rotated(&path)).unwrap().len();
        assert!(current <= 1024 && old <= 1024, "{current} {old}");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("line number 0099"));
        assert!(!dir.path().join("log-helper.2").exists());
    }

    #[test]
    fn an_existing_file_is_appended_to() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("log-helper");
        {
            let logger = Logger::open(&path, Level::Info, 64 * 1024, false).unwrap();
            logger.log(Level::Info, format_args!("first"));
        }
        let logger = Logger::open(&path, Level::Info, 64 * 1024, false).unwrap();
        logger.log(Level::Info, format_args!("second"));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("first") && text.contains("second"));
    }

    #[test]
    fn level_words_parse() {
        assert_eq!(Level::parse("debug"), Some(Level::Debug));
        assert_eq!(Level::parse("loud"), None);
    }
}
