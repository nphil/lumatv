//! LumaTV's native helper: a static aarch64 daemon beside LG's compositor.
//!
//! It serves QML (and anything else on the TV) over a loopback JSON API: what the helper knows is a set of topics on the
//! [`hub`], change is announced by events a client long-polls, and settings live in a schema-checked store. Components:
//!
//! | Module | What |
//! |---|---|
//! | [`screen`] | whether the panel is lit (`tvpower` through `luna-send`); gates all network work |
//! | [`receiver`] | the Denon AVR's volume, mute, input and power, polled only while the screen is on |
//! | [`homefeeds`], [`fetcher`] | Home's weather and news: saved first, fetched while the screen is on; the HTTPS transport |
//! | [`settings`] | the data layer on LumaShow's `settings-schema` |
//! | [`api`], [`http`] | the v1 API and the loopback server it runs on |
//! | [`ai`] | the MCP server: AI assistants on the LAN, every permission held on the TV and changed only with the physical remote |
//! | [`plugins`] | the plugin broker: LumaShow's plugin API level 1 for settings pages, cards, tiles and notifications |
//! | [`device`] | what this device is (screen class, how a person drives it): the facts plugins get in `welcome`, and `/v1/device` |
//! | [`ambient`] | ambient light: the Settings page's values as the engine's plan, the picture from the capture helper, DDP to the strip |
//! | [`ambient_audio`], [`ambient_audio_capture`] | ambient light's sound mode: the TV's own sound mix reduced to loudness, bass and beats, and what they do to the colours |
//! | [`hue`], [`hue_rest`], [`hue_store`] | ambient light's Hue Entertainment output: the stream thread, the bridge's HTTPS, the saved link |
//! | [`ambient_z2m`], [`ambient_z2m_frame`], [`ambient_z2m_mqtt`], [`ambient_z2m_store`] | ambient light's Zigbee2MQTT output: Hue entertainment frames over MQTT, no bridge |
//! | [`voice`] | voice search: the mic key's words as text for the search box, through Home Assistant's Assist pipeline |
//! | [`preload`] | keeps the apps the owner switched off from preloading, at each switch-on (webOS forgets the policy) |
//! | [`service`], [`cli`] | start and stop, the lock, the command line |

pub mod ai;
pub mod ambient;
pub mod ambient_audio;
pub mod ambient_audio_capture;
pub mod ambient_z2m;
pub mod ambient_z2m_frame;
pub mod ambient_z2m_mqtt;
pub mod ambient_z2m_store;
pub mod api;
pub mod cli;
pub mod config;
pub mod control;
pub mod ctx;
pub mod device;
pub mod fetcher;
pub mod homefeeds;
pub mod http;
pub mod hub;
pub mod hue;
pub mod hue_rest;
pub mod hue_store;
pub mod images;
pub mod log;
pub mod plugins;
pub mod preload;
pub mod receiver;
pub mod receiver_telnet;
pub mod screen;
pub mod service;
pub mod settings;
pub mod sys;
pub mod timeutil;
pub mod voice;
pub mod voice_store;
pub mod voice_ws;

/// The version of the API paths and documents: `/v1/...`. A change that breaks a client is `/v2`.
pub const API_VERSION: u32 = 1;
/// The helper's own version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
