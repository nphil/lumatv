//! Ambient light for LumaTV: what the video hardware reports about the picture becomes a colour per light, and the
//! lights are driven over Hue Entertainment (HueStream over DTLS, to Bifrost or a bridge) and WLED (DDP).
//!
//! The 32-bit capture helper (`ambient/capture/`) polls LG's histogram API and hands areas to the helper; this crate
//! is everything after that, with no dependency on LG's library, so it builds and tests on the workstation:
//!
//! - [`zones`]: the four hardware presets (area rectangles), histogram to colour, replay of probe dumps.
//! - [`layout`]: light positions and the area-to-light weights.
//! - [`bars`]: letterbox and pillarbox detection, which picks the hardware preset.
//! - [`delay`], [`tuning`], [`smooth`]: output delay, colour shaping and the dark gate, time smoothing.
//! - [`engine`]: those, wired in order, with the Hue channel and WLED LED output conversions.
//! - [`huestream`] and [`dtls`]: the HueStream v2 packet and the DTLS 1.2 PSK client under it.
//! - [`ddp`]: WLED's UDP realtime packets.

#![forbid(unsafe_code)]

pub mod bars;
pub mod ddp;
pub mod delay;
pub mod dtls;
pub mod engine;
pub mod huestream;
pub mod layout;
mod presets;
pub mod rgb;
pub mod smooth;
pub mod tuning;
pub mod zones;

pub use engine::{Config, ConfigError, Engine, hue_channels, wled_leds};
pub use layout::{Light, Target};
pub use rgb::Rgb;
pub use zones::{Preset, RawZone, ZoneFrame};
