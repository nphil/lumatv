//! LumaTV's native helper: a static aarch64 daemon beside LG's compositor.
//!
//! It serves QML (and anything else on the TV) over a loopback JSON API: what the helper knows is a set of topics on the
//! [`hub`], change is announced by events a client long-polls, and settings live in a schema-checked store. Components:
//!
//! | Module | What |
//! |---|---|
//! | [`screen`] | whether the panel is lit (`tvpower` through `luna-send`); gates all network work |
//! | [`receiver`] | the Denon AVR's volume, mute, input and power, polled only while the screen is on |
//! | [`homefeeds`] | Home's weather and news: saved first, fetched while the screen is on |
//! | [`settings`] | the data layer on LumaShow's `settings-schema` |
//! | [`api`], [`http`] | the v1 API and the loopback server it runs on |
//! | [`service`], [`cli`] | start and stop, the lock, the command line |
//!
//! Phase 3 adds the MCP server and phase 9 the plugin broker as components of [`service::Service`] (see its docs).

pub mod api;
pub mod cli;
pub mod config;
pub mod control;
pub mod ctx;
pub mod homefeeds;
pub mod http;
pub mod hub;
pub mod log;
pub mod receiver;
pub mod screen;
pub mod service;
pub mod settings;
pub mod sys;
pub mod timeutil;

/// The version of the API paths and documents: `/v1/...`. A change that breaks a client is `/v2`.
pub const API_VERSION: u32 = 1;
/// The helper's own version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
