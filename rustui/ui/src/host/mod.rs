//! The host's side of the shell: the long-poll feeds of the UI channel and the helper ([`http`]), the key socket ([`keys`]), and the
//! clock. The loop that joins them to the engine's socket is `main.rs`.

pub mod apps;
pub mod buttons;
pub mod channels;
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

/// The harness's own time, when it keeps one (`lumatv-ui --host --virtual`): the clock reads it instead of the system's, and the harness
/// moves it on a refresh at a time. 0: the system's clock.
static VIRTUAL_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Wall-clock milliseconds, the clock the models run on (the helper stamps its readings with it).
pub fn now_ms() -> u64 {
    match VIRTUAL_MS.load(std::sync::atomic::Ordering::Relaxed) {
        0 => (system_ms() + OFFSET_MS.load(std::sync::atomic::Ordering::Relaxed)).max(0) as u64,
        ms => ms,
    }
}

/// Makes the models' clock read `epoch_ms` now; it goes on from there.
pub fn set_clock(epoch_ms: u64) {
    OFFSET_MS.store(epoch_ms as i64 - system_ms(), std::sync::atomic::Ordering::Relaxed);
}

/// Makes the models' clock read `epoch_ms` until it is set again: the harness's time stands still between its steps.
pub fn set_virtual_ms(epoch_ms: u64) {
    VIRTUAL_MS.store(epoch_ms.max(1), std::sync::atomic::Ordering::Relaxed);
}
