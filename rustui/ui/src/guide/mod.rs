//! The channel guide of the Rust shell (`docs/research/channel-guide.md`): LG's guide rebuilt in LumaTV's own look. The model ([`model`]) is
//! pure and tested on the host: the channels and their programmes from LG's guide service ([`crate::host::channels`]), the remote's keys, what
//! is asked and when. The surface ([`surface`], measured by [`layout`]) draws it with the TV kit and the host ([`host`]) joins the two to the
//! shell. The fixture ([`fixture`]) is a make-believe set of channels and listings for the host harness.

pub mod fixture;
pub mod host;
pub mod layout;
pub mod model;
pub mod surface;

#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod surface_tests;
