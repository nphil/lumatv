//! A phone as a TV's remote on the home network, protocol version 1: the phone pairs once, by a code the TV shows, and then drives the TV
//! over an authenticated session: the remote's keys, typing into the active text field, a pointer, the host's actions and lists. The
//! protocol is written down in LumaTV's `docs/research/phone-remote.md` and as exact bytes in [`vectors`]; LumaRemoteKit
//! (`nphil/LumaRemoteKit`) is the phone's side.
//!
//! The engine has no host in it. What a host gives it:
//!
//! | Hook | What |
//! |---|---|
//! | [`server::Host`] | the TV: keys, text and edits into the active field, the pointer, actions, lists, and the state phones are shown |
//! | [`server::Config`] | the address to listen on, the TV's name and wired MAC, the file the paired phones are kept in |
//! | the `log` facade | sessions opened and ended and connections refused, under the target `lumaremote`; never anything typed |
//!
//! | Module | What |
//! |---|---|
//! | [`wire`] | the preamble, the Noise handshake's framing and the session's frames |
//! | [`pairing`] | codes, the pairing key and the pairing window |
//! | [`proto`] | the session's messages from the phone, and the remote's key names |
//! | [`datagram`] | pointer, ping and pong datagrams and what pointer totals mean |
//! | [`link`] | the pairing link a QR code carries and the mDNS TXT record |
//! | [`store`] | the paired phones |
//! | [`server`] | the TV's side: listener, pairing, sessions |
//! | [`client`] | the phone's side, for the `lumaremote` test client and the tests |

pub mod client;
pub mod datagram;
pub mod link;
pub mod pairing;
pub mod proto;
pub mod server;
pub mod store;
pub mod vectors;
pub mod wire;

/// TCP and UDP.
pub const PORT: u16 = 18733;
/// The mDNS service type.
pub const SERVICE: &str = "_lumatv-remote._tcp";
/// The preamble's version byte and TXT `v`.
pub const VERSION: u8 = 1;

/// Why the TV refused a message: the `code` of an `error` frame or a `done` with an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Not JSON, an unknown `t`, a member missing or wrong.
    BadMessage,
    BadKey,
    BadAction,
    TooLong,
    /// `text` or `edit` while no text field is active.
    NoField,
    /// The plugin an action needs is off.
    Off,
    /// The screen is off: only the `power` key, `ping` and `bye` work.
    Standby,
    /// An unknown app, channel or camera.
    NotFound,
    /// The part of the TV that would do it is not running.
    Unavailable,
    Rate,
}

impl Fault {
    pub fn code(self) -> &'static str {
        match self {
            Fault::BadMessage => "bad-message",
            Fault::BadKey => "bad-key",
            Fault::BadAction => "bad-action",
            Fault::TooLong => "too-long",
            Fault::NoField => "no-field",
            Fault::Off => "off",
            Fault::Standby => "standby",
            Fault::NotFound => "not-found",
            Fault::Unavailable => "unavailable",
            Fault::Rate => "rate",
        }
    }
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `N` bytes from exactly `2N` hex digits, either case.
pub fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != 2 * N || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// Milliseconds since the Unix epoch.
pub fn unix_ms(at: std::time::SystemTime) -> u64 {
    at.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}
