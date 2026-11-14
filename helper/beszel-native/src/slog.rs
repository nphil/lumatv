//! Log lines in the Go agent's default `slog` form, `2006/01/02 15:04:05 LEVEL message key=value`,
//! because supervisors read the agent's state from them.

use std::fmt::{Display, Write as _};
use std::io::Write as _;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

/// Applies `LOG_LEVEL`; anything other than debug, warn or error leaves info.
pub fn set_level(name: &str) {
    let level = match name.to_ascii_lowercase().as_str() {
        "debug" => Level::Debug,
        "warn" => Level::Warn,
        "error" => Level::Error,
        _ => return,
    };
    LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn enabled(level: Level) -> bool {
    level as u8 >= LEVEL.load(Ordering::Relaxed)
}

pub type Attrs<'a> = &'a [(&'a str, &'a dyn Display)];

pub fn log(level: Level, msg: &str, attrs: Attrs) {
    if !enabled(level) {
        return;
    }
    let mut line = String::with_capacity(96);
    stamp(&mut line);
    line.push(' ');
    line.push_str(level.name());
    line.push(' ');
    line.push_str(msg);
    for (key, value) in attrs {
        let value = value.to_string();
        line.push(' ');
        line.push_str(key);
        line.push('=');
        if needs_quoting(&value) {
            let _ = write!(line, "{value:?}");
        } else {
            line.push_str(&value);
        }
    }
    line.push('\n');
    let _ = std::io::stderr().lock().write_all(line.as_bytes());
}

pub fn debug(msg: &str, attrs: Attrs) {
    log(Level::Debug, msg, attrs);
}

pub fn info(msg: &str, attrs: Attrs) {
    log(Level::Info, msg, attrs);
}

pub fn warn(msg: &str, attrs: Attrs) {
    log(Level::Warn, msg, attrs);
}

pub fn error(msg: &str, attrs: Attrs) {
    log(Level::Error, msg, attrs);
}

/// `log.Fatal`'s line: a time stamp, no level.
pub fn fatal(msg: &str) -> ! {
    let mut line = String::with_capacity(96);
    stamp(&mut line);
    let _ = writeln!(line, " {msg}");
    let _ = std::io::stderr().lock().write_all(line.as_bytes());
    std::process::exit(1);
}

fn needs_quoting(value: &str) -> bool {
    value.is_empty() || value.chars().any(|c| c == '=' || c == '"' || c.is_whitespace() || c.is_control())
}

fn stamp(out: &mut String) {
    // SAFETY: `time` accepts a null pointer; `localtime_r` fills the zeroed `tm` it is given.
    let tm = unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm
    };
    let _ = write!(
        out,
        "{:04}/{:02}/{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_values_the_way_slog_does() {
        assert!(needs_quoting(""));
        assert!(needs_quoting("dial tcp: connection refused"));
        assert!(needs_quoting("a=b"));
        assert!(!needs_quoting("hub.test:8090"));
        assert!(!needs_quoting("/var/lib/beszel-agent"));
    }
}
