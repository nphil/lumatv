//! A second-screen (SSAP) gateway for LG webOS TVs, wire-compatible with LG's own on port 3001: one thread around `poll`,
//! the TV's bus reached through `luna-send`, and LG's gateway on port 3000 for what it does not do itself.

pub mod log;

mod adapter;
mod iface;
mod luna;
mod net;
mod pins;
mod server;
mod session;
mod store;
mod util;
mod ws;

pub use adapter::TOAST_SOURCE_ID;
pub use server::{Config, EXIT_UNUSABLE, run};
