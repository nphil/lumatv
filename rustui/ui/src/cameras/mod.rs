//! The Cameras app of the Rust shell (`docs/research/cameras-ui-spec.md`, the mocks in `docs/mocks/cameras-scrypted/`): the grid of live
//! stills, one camera full screen (live on the video plane, stills where live is not offered), the timeline strip with its scrub
//! thumbnails and event marks, the events and clips sheet, and the per-camera pop-up options page. The model ([`app`]) is pure and
//! tested on the host; the surfaces draw its [`view::View`] with the TV kit.

pub mod app;
pub mod data;
pub mod grid;
pub mod options;
pub mod pics;
pub mod sheet;
pub mod strip;
pub mod surface;
pub mod view;
pub mod words;

#[cfg(test)]
mod app_tests;
