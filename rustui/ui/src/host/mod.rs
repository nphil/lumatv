//! The host's side of the shell: the long-poll feeds of the UI channel and the helper ([`http`]), the key socket ([`keys`]), and the
//! clock. The loop that joins them to the engine's socket is `main.rs`.

pub mod apps;
pub mod clock;
pub mod http;
pub mod inputs;
pub mod keys;
pub mod net;
pub mod power;
pub mod receiver;
pub mod sio;
pub mod theme;

/// Wall-clock milliseconds, the clock the models run on (the helper stamps its readings with it).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
