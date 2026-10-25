//! What every component of the helper is handed.

use crate::config::{Config, Paths};
use crate::control::Stop;
use crate::hub::Hub;
use crate::settings::SettingsStore;
use std::sync::Arc;

/// Shared by the components: configuration, the hub they publish to, the settings they read, and the stop they obey.
///
/// This is the seam for later phases: the MCP server (phase 3) and the plugin broker (phase 9) are components that take a
/// `Ctx`, read topics from `hub`, register owners with `settings`, and end when `stop` is requested.
#[derive(Clone)]
pub struct Ctx {
    pub config: Arc<Config>,
    pub paths: Paths,
    pub hub: Arc<Hub>,
    pub settings: Arc<SettingsStore>,
    pub stop: Stop,
}
