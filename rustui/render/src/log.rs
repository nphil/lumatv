//! One line per notable event on stderr, stamped with the seconds since the first line so that a log reads against
//! the app's. Never called per frame.

use crate::clock::now_ns;
use std::fmt::Display;
use std::sync::LazyLock;

pub fn log(message: impl Display) {
    static START: LazyLock<u64> = LazyLock::new(now_ns);
    let ms = now_ns().saturating_sub(*START) / 1_000_000;
    eprintln!("[{}.{:03}] compositor: {message}", ms / 1000, ms % 1000);
}
