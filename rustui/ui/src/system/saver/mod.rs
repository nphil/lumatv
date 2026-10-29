//! The screensaver (`Screensaver.qml`, `SaverVote.qml`) for the Rust shell as a calm Lucent night face: an analog clock on black, nothing
//! brighter than the cap, everything slowly moving and dimming. [`look`] holds the numbers (the hard brightness cap, the dim, the drift, the
//! spots), [`clock`] rasters the face and the hands, [`model`] is when the saver is up (LG's state, tvpower's vote, keys, the pointer,
//! standby) and its timeline, [`glance`] is what it says besides the time (the date, the weather, the slots and the notices under the
//! owner's rules) and [`surface`] draws all of it with the kit.

pub mod clock;
pub mod glance;
pub mod look;
pub mod model;
pub mod surface;
