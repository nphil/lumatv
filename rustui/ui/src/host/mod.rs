//! The host's side of the shell: the long-poll feeds of the UI channel and the helper ([`http`]), the key socket ([`keys`]), and the
//! clock. The loop that joins them to the engine's socket is `main.rs`.

pub mod apps;
pub mod buttons;
pub mod clock;
pub mod http;
pub mod inputs;
pub mod keys;
pub mod net;
pub mod power;
pub mod previews;
pub mod receiver;
pub mod sio;
pub mod theme;
pub mod vwin;

/// What the models' clock is ahead of the system's, in milliseconds (the harness's `--clock` sets it, so a render of the saver's clock does
/// not depend on the day it is made).
static OFFSET_MS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

fn system_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Wall-clock milliseconds, the clock the models run on (the helper stamps its readings with it).
pub fn now_ms() -> u64 {
    (system_ms() + OFFSET_MS.load(std::sync::atomic::Ordering::Relaxed)).max(0) as u64
}

/// Makes the models' clock read `epoch_ms` now; it goes on from there.
pub fn set_clock(epoch_ms: u64) {
    OFFSET_MS.store(epoch_ms as i64 - system_ms(), std::sync::atomic::Ordering::Relaxed);
}
