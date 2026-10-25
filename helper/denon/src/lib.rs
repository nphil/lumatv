//! Denon AVR status over the receiver's own interfaces.
//!
//! The AVR-S760H answers `GET /goform/formMainZone_MainZoneXmlStatusLite.xml` on port 8080 with a few lines of XML and
//! needs no held connection. Measured on the set's receiver: the reply is `HTTP/1.0` from `mongoose2.5` with no
//! `Content-Length` (the body ends when the receiver closes), and the fuller `formMainZone_MainZoneXml.xml` answers
//! `403 Forbidden` on this port, so only the Lite status is read. The telnet port pushes every change to each connected
//! client and accepts a second one beside Home Assistant's; [`telnet`] is that read-only link.
//!
//! [`parse_status_lite`] and [`telnet::parse_line`] are pure; [`Client`] and [`telnet::Session`] are blocking transports with
//! a hard deadline. None keeps state between calls: the helper's receiver thread decides when to ask and what a change is.

#![forbid(unsafe_code)]

mod client;
mod status;
pub mod telnet;

pub use client::{Client, ClientError, STATUS_LITE_PATH, mute_command, volume_command};
pub use status::{ParseError, Power, Status, VolumeDisplay, parse_status_lite};

/// The port the receiver's web interface listens on for the status XML.
pub const DEFAULT_PORT: u16 = 8080;
