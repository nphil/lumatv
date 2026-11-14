//! One line per message on stderr: `<LEVEL> <message>`, LEVEL one of INFO, WARN, ERROR. Debug lines print as INFO and only
//! when `SSGW_LOG=debug`.

use std::fmt::Arguments;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static DEBUG: AtomicBool = AtomicBool::new(false);

pub fn set_debug(on: bool) {
    DEBUG.store(on, Ordering::Relaxed);
}

pub fn debug_enabled() -> bool {
    DEBUG.load(Ordering::Relaxed)
}

pub fn emit(level: &str, args: Arguments<'_>) {
    let mut line = format!("{level} {args}");
    // a message carrying a newline (a peer's text) would otherwise read as a second, unlevelled line
    if line.contains(['\n', '\r']) {
        line = line.replace(['\n', '\r'], " ");
    }
    line.push('\n');
    if cfg!(test) {
        // the test harness captures only what goes through its own print macros
        eprint!("{line}");
        return;
    }
    let _ = std::io::stderr().lock().write_all(line.as_bytes());
}

#[macro_export]
macro_rules! info {
    ($($t:tt)*) => { $crate::log::emit("INFO", format_args!($($t)*)) };
}

#[macro_export]
macro_rules! warn {
    ($($t:tt)*) => { $crate::log::emit("WARN", format_args!($($t)*)) };
}

#[macro_export]
macro_rules! error {
    ($($t:tt)*) => { $crate::log::emit("ERROR", format_args!($($t)*)) };
}

#[macro_export]
macro_rules! debug {
    ($($t:tt)*) => {
        if $crate::log::debug_enabled() {
            $crate::log::emit("INFO", format_args!($($t)*))
        }
    };
}
