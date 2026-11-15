//! Offline-first Leadjoy/Zhiwan configuration codec, recovered from Leadjoy PC 1.4.0.
//! No firmware updater, device discovery, or implicit hardware access is included.
//! Protocol evidence and model limits: `docs/research/leadjoy-xeno-plus-protocol.md`.

use serde::Deserialize;
use serde_json::Value;
use std::{fmt, io};

pub mod command;
pub mod frame;
pub mod session;
pub mod settings;

pub use command::{Change, Command, LedFrame, Lighting, LightingMode, Model, Profile, Request, Response};
pub use session::{CommandReceipt, Controller, Execution, RunMode, Transport, dry_run};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug)]
pub enum Error {
    Invalid(&'static str),
    Encode(rmp_serde::encode::Error),
    Decode(rmp_serde::decode::Error),
    Io(io::Error),
    NoReply,
    Device(u8),
    ReadBack,
    /// The write was acknowledged, but its read-back failed, so the new value is unconfirmed.
    /// A controller-mode change can legitimately end here when the interface re-enumerates.
    Unverified(Box<Error>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => f.write_str(reason),
            Self::Encode(error) => error.fmt(f),
            Self::Decode(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
            Self::NoReply => f.write_str("controller did not reply before the deadline"),
            Self::Device(code) => write!(f, "controller returned error {code}"),
            Self::ReadBack => f.write_str("settings read back differently from the requested values"),
            Self::Unverified(error) => write!(f, "controller acknowledged the change, but reading it back failed: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// MessagePack maps put the vendor's selectors before dependent fields. This does not send anything.
pub fn encode(value: &Value) -> Result<Vec<frame::Report>> {
    let payload = rmp_serde::to_vec_named(&command::WireValue(value)).map_err(Error::Encode)?;
    frame::encode(&payload)
}

/// Decodes a reassembled payload, rejecting trailing data and oversized input.
pub fn decode(payload: &[u8]) -> Result<Value> {
    if payload.is_empty() || payload.len() > frame::MAX_PAYLOAD {
        return Err(Error::Invalid("MessagePack payload length"));
    }
    let mut input = io::Cursor::new(payload);
    let value = Value::deserialize(&mut rmp_serde::Deserializer::new(&mut input)).map_err(Error::Decode)?;
    if input.position() != payload.len() as u64 {
        return Err(Error::Invalid("trailing MessagePack data"));
    }
    Ok(value)
}
