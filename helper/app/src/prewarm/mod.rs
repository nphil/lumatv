//! Prewarm: the apps behind the remote's buttons (Marquee, Netflix, Prime Video, Apple TV, Live TV) are kept ready in the
//! background, so a press opens one in about a second less than a cold start.
//!
//! The mechanism is LG's own preload: SAM `launch` with `preload: "partial"`, `launchHidden`, `noSplash`, asked as
//! `com.webos.service.preloadmanager`. The app starts in the background with no window on screen, and the next plain launch of
//! it takes the app's relaunch path. LG's own manager cannot cover this set (its list lacks Marquee and Apple TV and has no
//! order), so this module is the one owner of the listed apps; an app LG preloaded already counts as ready.
//!
//! Which apps, in which order, and on or off: `system.prewarm` of theme.json (`{"on": true, "apps": [..]}`, Settings > System).
//! Priority is the order: launches go first to last while memory allows, and under pressure the last goes first. The order
//! also reaches [`crate::app_priority`] through [`KEEP`], which closes any app that is not listed before a listed one, and a
//! listed one last to first.
//!
//! Nothing is launched while the screen is off or has just come on, a pixel refresher run is going, a game is in front or game
//! mode is on, an app is on screen (only Home idle for a few seconds), anything is playing, or less than 400 MB of headroom
//! (`min(usable_pages, MemAvailable)`) would remain. Below 300 MB the lowest-priority app that is out of sight and silent is
//! closed, one every five seconds, and it is not launched again for ten minutes. One launch at a time; at most three per app
//! per ten minutes. An app that shows itself or makes sound when preloaded is closed, Home is asked for, and the app is left
//! out until the helper restarts. Marquee and Apple TV ignore the preload mode and ask for fullscreen as they start; the
//! compositor holds the window of a process started as a preload until the app is launched, so they are preloaded like the
//! others (without that hold they would show themselves and be left out).
//!
//! The `prewarm` topic: `{on, order, apps: [{id, state: ready|launching|cold|skipped, costMb, note}], headroomMb, waiting}`.
//! Nothing is written to disk.

pub mod policy;
#[cfg(test)]
mod tests;

use crate::app_priority::game::GAMES;
use crate::app_priority::lg::{Lg, Luna};
use crate::app_priority::levels::PAGES_PER_MB;
use crate::app_priority::os::{Fs, Os};
use crate::ctx::Ctx;
use crate::hub::Hub;
use parking_lot::RwLock;
use policy::{Action, HOME, Inputs, Policy, Settings};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const TOPIC: &str = "prewarm";
const TICK: Duration = Duration::from_secs(5);
/// SAM's list is asked again at most this often unless memory is short or a launch is being watched.
const ASK_EVERY_MS: u64 = 30_000;
const MAX_THEME_BYTES: u64 = 4 * 1024 * 1024;
const SAM: &str = "luna://com.webos.applicationManager";
/// The caller LG's preload manager launches as.
const CALLER: &str = "com.webos.service.preloadmanager";

/// The owner's order, as [`crate::app_priority`] reads it to decide who is closed first.
pub static KEEP: Keep = Keep::new();

pub struct Keep(RwLock<Vec<String>>);

impl Keep {
    pub const fn new() -> Self {
        Self(RwLock::new(Vec::new()))
    }

    pub fn set(&self, order: Vec<String>) {
        *self.0.write() = order;
    }

    /// How long `app` is kept: 0 for an app that is not listed, then 1 for the last listed up to the number listed for the first.
    pub fn protection(&self, app: &str) -> usize {
        let list = self.0.read();
        list.iter().position(|a| a == app).map_or(0, |i| list.len() - i)
    }
}

impl Default for Keep {
    fn default() -> Self {
        Self::new()
    }
}

/// What the policy acts through: SAM, the media server and the kernel's memory numbers.
pub trait Platform {
    fn running(&self) -> Option<Vec<String>>;
    fn playing(&self) -> Option<BTreeSet<String>>;
    fn launch_hidden(&self, app: &str) -> bool;
    fn close(&self, app: &str) -> bool;
    fn home(&self) -> bool;
    fn headroom_mb(&self) -> Option<u64>;
}

/// What the hub says about the moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    pub screen_on: bool,
    pub refresher: bool,
    pub game: bool,
    pub front: Option<String>,
}

/// The policy with what it has been told about SAM, and the platform calls its actions make.
#[derive(Default)]
pub struct Driver {
    policy: Policy,
    running: Vec<String>,
    playing: Option<BTreeSet<String>>,
    asked_at: Option<u64>,
    front: Option<String>,
}

impl Driver {
    /// One step at `now_ms`; the topic's value.
    pub fn tick(&mut self, s: &Settings, env: &Env, plat: &dyn Platform, now_ms: u64) -> Value {
        if !s.on || !env.screen_on {
            let idle = Inputs { now_ms, screen_on: false, refresher: false, game: false, front: None, running: &[], playing: None, headroom_mb: None };
            self.policy.step(s, &idle);
            return self.policy.status(s, &self.running, None);
        }
        let headroom = plat.headroom_mb();
        let short = headroom.is_some_and(|h| h < policy::EVICT_BELOW_MB + 50);
        let moved = self.front != env.front;
        self.front.clone_from(&env.front);
        if self.asked_at.is_none_or(|t| now_ms.saturating_sub(t) >= ASK_EVERY_MS) || short || moved || self.policy.watching() {
            if let Some(running) = plat.running() {
                self.running = running;
            }
            self.playing = plat.playing();
            self.asked_at = Some(now_ms);
        }
        let inputs = Inputs {
            now_ms,
            screen_on: true,
            refresher: env.refresher,
            game: env.game,
            front: env.front.as_deref(),
            running: &self.running,
            playing: self.playing.as_ref(),
            headroom_mb: headroom,
        };
        let actions = self.policy.step(s, &inputs);
        for action in actions {
            self.asked_at = None;
            match action {
                Action::Launch(app) => {
                    if !plat.launch_hidden(&app) {
                        crate::log_warn!("prewarm: SAM refused to preload {app}");
                        self.policy.failed(&app, now_ms);
                    }
                }
                Action::Close { app, why } => {
                    crate::log_info!("prewarm: closing {app} ({why})");
                    plat.close(&app);
                }
                Action::Home => {
                    plat.home();
                }
            }
        }
        self.policy.status(s, &self.running, headroom)
    }
}

/// The settings in theme.json; the default when the file is missing, unreadable or over 4 MiB.
fn read_settings(path: &Path) -> Settings {
    let mut bytes = Vec::new();
    let read = std::fs::File::open(path).and_then(|f| f.take(MAX_THEME_BYTES).read_to_end(&mut bytes));
    match read.ok().and_then(|_| serde_json::from_slice::<Value>(&bytes).ok()) {
        Some(theme) => Settings::from_theme(&theme),
        None => Settings::default(),
    }
}

/// Where the screen and the apps are now, from the hub. An app list that is not exactly Home counts as an app on screen.
fn env_of(hub: &Hub) -> Env {
    let front_ids: Vec<String> = hub
        .get("app_priority")
        .and_then(|v| v.get("front").and_then(Value::as_array).cloned())
        .map(|a| a.iter().filter_map(|id| id.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    let front = front_ids.iter().find(|id| *id != HOME).or_else(|| front_ids.first()).cloned();
    let game_mode = hub.get("gamemode").and_then(|v| v.get("on").and_then(Value::as_bool)) == Some(true);
    Env {
        screen_on: crate::screen::is_on(hub),
        refresher: crate::refreshguard::refresher_current().get("running").and_then(Value::as_bool) == Some(true),
        game: game_mode || front_ids.iter().any(|id| GAMES.contains(&id.as_str())),
        front,
    }
}

struct Sys {
    send: String,
    lg: Luna,
    os: Fs,
}

impl Platform for Sys {
    fn running(&self) -> Option<Vec<String>> {
        self.lg.running().map(|apps| apps.into_iter().map(|a| a.id).collect())
    }

    fn playing(&self) -> Option<BTreeSet<String>> {
        self.lg.playing()
    }

    fn launch_hidden(&self, app: &str) -> bool {
        let payload = json!({"id": app, "preload": "partial", "noSplash": true, "launchHidden": true, "params": {}});
        crate::luna::once_as(&self.send, Some(CALLER), &format!("{SAM}/launch"), &payload.to_string())
            .and_then(|r| r.get("returnValue").and_then(Value::as_bool))
            == Some(true)
    }

    fn close(&self, app: &str) -> bool {
        self.lg.close(app)
    }

    fn home(&self) -> bool {
        crate::luna::once(&self.send, &format!("{SAM}/launch"), &json!({"id": HOME}).to_string())
            .and_then(|r| r.get("returnValue").and_then(Value::as_bool))
            == Some(true)
    }

    /// LG's `usable_pages` (what its memory levels are judged on) or `MemAvailable`, whichever is less.
    fn headroom_mb(&self) -> Option<u64> {
        let usable = self.os.usable_pages().map(|p| p / PAGES_PER_MB);
        let available = self.os.available_kb().map(|kb| kb / 1024);
        usable.into_iter().chain(available).min()
    }
}

pub fn spawn(ctx: &Ctx) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new().name("prewarm".to_owned()).stack_size(256 * 1024).spawn(move || {
        let send = ctx.config.luna.send.clone();
        let sys = Sys { lg: Luna::new(&send), send, os: Fs::lg() };
        let mut driver = Driver::default();
        let started = Instant::now();
        let theme = ctx.paths.theme_file();
        loop {
            let settings = read_settings(&theme);
            KEEP.set(if settings.on { settings.order.clone() } else { Vec::new() });
            let now_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let status = driver.tick(&settings, &env_of(&ctx.hub), &sys, now_ms);
            ctx.hub.set(TOPIC, status);
            if ctx.stop.sleep(TICK) {
                break;
            }
        }
        KEEP.set(Vec::new());
    })
}
