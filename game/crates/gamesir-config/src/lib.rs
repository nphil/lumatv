//! Settings for the GameSir Cyclone 2 over its vendor HID interface (vendor 0x3537, usage page 0xFFF0), as GameSir Connect 1.14.5
//! drives it. The interface exists only while the controller enumerates as a GameSir device: on the USB cable, or through the 2.4 GHz
//! receiver in its X-input mode. In DS4, Switch and Bluetooth modes there is no configuration interface to talk to.
//!
//! No firmware-update command is encoded here. The protocol notes are in `docs/research/gamesir-cyclone2-protocol.md`.

use std::fmt;
use std::io;

pub mod device;
pub mod frame;
pub mod profile;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The controller never answered the command, after every resend.
    NoReply {
        command: u8,
    },
    /// A block of settings had the wrong size.
    Length {
        expected: usize,
        actual: usize,
    },
    /// A setting outside the range GameSir Connect offers.
    Range {
        setting: &'static str,
        value: u32,
    },
    /// Profile slots run 1 to 4.
    Slot(u8),
    /// After a switch the controller still reported another profile as active.
    Switch {
        wanted: u8,
        active: u8,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => e.fmt(f),
            Error::NoReply { command } => write!(f, "no reply to command {command:#04x}"),
            Error::Length { expected, actual } => write!(f, "expected {expected} bytes, got {actual}"),
            Error::Range { setting, value } => write!(f, "{setting} cannot be {value}"),
            Error::Slot(slot) => write!(f, "profile slot {slot} is not 1-4"),
            Error::Switch { wanted, active } => write!(f, "asked for profile {wanted}, the controller stayed on {active}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}
