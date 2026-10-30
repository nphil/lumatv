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
//! | [`ha_store`] | the saved address and token of a Home Assistant link, one private file (voice search's and the plugin lease's) |
//! | [`scrypted`], [`live`] | cameras through Scrypted's LumaShow bridge: the link (`scrypted-link`), pinned HTTPS, stills, push events and pop-ups, the Cameras page, the timeline routes; the full view's RTSP stream on the video plane through the GFN feeder |
//! | [`onhome`] | On Home: what Home Assistant's LumaShow integration shares with the TV (it dials the TV, which holds no token) as tiles on Home, and the controls of those tiles, narrowed to what the TV may do |
//! | [`voice`] | voice search: the mic key's words as text for the search box, through Home Assistant's Assist pipeline |
//! | [`profiles`] | local profiles (Nitin, Isabel): each person's Home kept apart from the household's settings, the switch, the PINs, the `profiles` topic; the working copy `theme/` stays what the Home layer reads |
//! | [`preload`] | keeps the apps the owner switched off from preloading, at each switch-on (webOS forgets the policy) |
//! | [`framerate`] | the panel's rate after 25 or 50 fps video: back to 120 Hz once Home is in front with no video window |
//! | [`ble`], [`ble_proxy`], [`ble_scan`], [`ble_duty`], [`ble_store`] | the Bluetooth proxy: the TV as a Home Assistant Bluetooth proxy (ESPHome native API) for presence and BLE sensors, through LG's `bluetooth2` scan with the radio's duty limits |
//! | [`refreshguard`] | LG's panel refresh (the `panel` topic) and the guard that keeps an assistant's wake from cutting a run short, behind `theme.oledcare.refreshGuard` |
//! | [`paneldim`] | the net under the compositor's panel dim: a journal left by a compositor that died puts the panel's OLED pixel brightness back |
//! | [`updateblock`] | whether LG's system updates are blocked (the Homebrew Channel's flag and its `/etc/hosts` block), read only |
//! | [`openapps`] | open apps without updating: LG's list of apps that must be updated before they open, kept empty while the owner has it on, and put back when off |
//! | [`crash`] | the crash notice Settings shows: what the supervisor saved when LumaTV's compositor was put out of the way (reason, time, last log and trace lines), read when the page opens |
//! | [`capture`] | screenshots and screen recordings: the door to the compositor's capture, behind `capture.enabled` (off by default) |
//! | [`previews`] | the picker's input pictures: LG's capture service asked for the video plane, kept small, behind Settings' `inputs.previews` (on) |
//! | [`gfn_boost`] | the GeForce NOW app's pad reader thread at real-time priority (FIFO 10, off CPU 0), the one privilege its jail cannot give itself |
//! | [`displaylab`] | the display lab: the shell's guided look at the OSD sharpener, dimming and a cut-out window, written through one hard list, journaled, read back and put back by every way out (the dead-man, the helper's stop, the next start); the TV's own shell only |
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
pub mod ble;
pub mod ble_cli;
pub mod ble_duty;
pub mod ble_proxy;
pub mod ble_scan;
pub mod ble_store;
pub mod capture;
pub mod cli;
pub mod config;
pub mod control;
pub mod crash;
pub mod ctx;
pub mod device;
pub mod displaylab;
pub mod fetcher;
pub mod framerate;
pub mod gfn_boost;
pub mod ha_store;
pub mod homefeeds;
pub mod http;
pub mod hub;
pub mod hue;
pub mod hue_rest;
pub mod hue_store;
pub mod images;
pub mod live;
pub mod log;
pub mod placement;
pub mod plugins;
pub mod preload;
pub mod previews;
pub mod paneldim;
pub mod privacy;
pub mod profiles;
pub mod receiver;
pub mod receiver_telnet;
pub mod receiver_volume;
pub mod refreshguard;
pub mod screen;
pub mod saver;
pub mod scrypted;
pub mod service;
pub mod settings;
pub mod sys;
pub mod timeutil;
pub mod voice;
pub mod wxview;
pub mod onhome;
pub mod openapps;
pub mod updateblock;
pub mod voice_store;
pub mod voice_ws;

/// The version of the API paths and documents: `/v1/...`. A change that breaks a client is `/v2`.
pub const API_VERSION: u32 = 1;
/// The helper's own version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
