//! Notable renderer events. A host can install its own sink; standalone clients keep timestamped stderr output.

use crate::clock::now_ns;
use std::fmt::Display;
use std::sync::{LazyLock, OnceLock};

type LogSink = fn(&dyn Display);
static SINK: OnceLock<LogSink> = OnceLock::new();

/// Installs the host's logging path before rendering starts. No allocation or stderr write is made by the renderer once installed.
pub fn set_log_sink(sink: LogSink) -> Result<(), LogSink> {
    SINK.set(sink)
}

pub fn log(message: impl Display) {
    if let Some(sink) = SINK.get() {
        sink(&message);
    } else {
        static START: LazyLock<u64> = LazyLock::new(now_ns);
        let ms = now_ns().saturating_sub(*START) / 1_000_000;
        eprintln!("[{}.{:03}] compositor: {message}", ms / 1000, ms % 1000);
    }
}
