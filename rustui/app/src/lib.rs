//! LumaHome and the card app as clients of LumaShow's Rust UI toolkit: the Home scene (`home`), its look (`look`), the drawing it does
//! for itself (`raster`) and the remote's scripts (`script`); the card app's rules (`card`), its page (`card_ui`) and its channel to the
//! launcher SAM starts (`lifecycle`); and the window both live in (`window`, with the `window` feature). The binaries (`main.rs`,
//! `bin/card.rs`) run them as windows of LG's compositor or, for Home, into a pbuffer; the tests run them in-process through the
//! renderer's host, and the card app against LumaTV's compositor.

pub mod card;
pub mod card_ui;
pub mod home;
pub mod lifecycle;
pub mod look;
pub mod raster;
pub mod rig;
pub mod script;
#[cfg(feature = "window")]
pub mod window;
