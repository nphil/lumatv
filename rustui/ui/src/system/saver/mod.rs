//! The screensaver (`Screensaver.qml`, `SaverVote.qml`) for the Rust shell as a calm Lucent night face: an analog clock on black, nothing
//! brighter than the cap, everything slowly moving and dimming. [`look`] holds the numbers (the hard brightness cap, the dim, the drift, the
//! spots), [`clock`] rasters the face and the hands, and [`model`] is when the saver is up (LG's state, tvpower's vote, keys, the pointer,
//! standby) and its timeline. The surface that draws the date, the weather, the notices and the plugins' slots around the clock, and its
//! wiring into the shell's `saver` kind, are not here yet.

pub mod clock;
pub mod look;
pub mod model;
