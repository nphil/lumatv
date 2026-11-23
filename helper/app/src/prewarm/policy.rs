//! The prewarm decision: pure state in, actions out, on a millisecond clock the tests move by hand.
//!
//! Measured on the C4 (2026-11-23, Home only in front, 690-710 MB `MemAvailable`, 830 MB LG usable): a preload launch (`launch`
//! with `preload: partial, launchHidden, noSplash`, the call LG's preload manager makes) costs Netflix 76-92 MB, Prime Video
//! 113-122 MB, Live TV 32-40 MB; Marquee (49 MB) and Apple TV (98 MB) ignore the preload mode and come to the front, so they
//! are not preloaded (see [`shows_when_preloaded`]).

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const HOME: &str = "com.webos.app.home";
pub const MARQUEE: &str = "com.beb.plxnative";
pub const NETFLIX: &str = "netflix";
pub const PRIME: &str = "amazon";
pub const APPLE_TV: &str = "com.apple.appletv";
pub const LIVE_TV: &str = "com.webos.app.livetv";
/// The default order, first kept longest: Marquee, Netflix, Prime Video, Apple TV, Live TV.
pub const DEFAULT_ORDER: [&str; 5] = [MARQUEE, NETFLIX, PRIME, APPLE_TV, LIVE_TV];
pub const MAX_APPS: usize = 8;

/// Apps measured to ignore the preload mode and come to the front when launched hidden (2026-11-23): they are never preloaded, only
/// kept (not closed first) once the owner has opened them, until their own side or the compositor holds the card.
pub fn shows_when_preloaded(app: &str) -> bool {
    matches!(app, MARQUEE | APPLE_TV)
}

/// What the first launch is assumed to cost until one is measured, MB.
fn assumed_cost(app: &str) -> u64 {
    match app {
        MARQUEE => 50,
        NETFLIX => 90,
        PRIME => 120,
        APPLE_TV => 100,
        LIVE_TV => 40,
        _ => 100,
    }
}

/// Memory left for the app on screen after a launch: a launch needs this much headroom (`min(usable, MemAvailable)`) beyond its own
/// cost. LG's memory level turns `medium` at 250 MB usable, and a 4K Dolby Vision title in Netflix holds 200 MB.
pub const START_FLOOR_MB: u64 = 400;
/// After an eviction the app comes back only with this much more room.
pub const REARM_EXTRA_MB: u64 = 100;
/// Below this headroom the lowest-priority app that is out of sight is closed, one every [`EVICT_GAP_MS`] (above LG's `medium`).
pub const EVICT_BELOW_MB: u64 = 300;
pub const EVICT_GAP_MS: u64 = 5_000;
/// Nothing is launched until the screen has been on this long (boot and wake settle), nor until Home has been in front this long.
pub const SCREEN_DWELL_MS: u64 = 20_000;
pub const HOME_DWELL_MS: u64 = 5_000;
/// One launch at a time: its cost is read this long after it ran, and it is given up on after [`LAUNCH_WAIT_MS`].
pub const SETTLE_MS: u64 = 10_000;
pub const LAUNCH_WAIT_MS: u64 = 20_000;
/// A launch is checked for a window that came to the front or for sound this long after it ran.
pub const CHECK_MS: u64 = 2_000;
/// Between two launches.
pub const LAUNCH_GAP_MS: u64 = 3_000;
/// An evicted app, or one whose launches keep failing, is left alone this long; at most [`MAX_LAUNCHES`] per [`WINDOW_MS`].
pub const COOL_MS: u64 = 10 * 60_000;
pub const WINDOW_MS: u64 = 10 * 60_000;
pub const MAX_LAUNCHES: usize = 3;

/// The owner's setting: `system.prewarm` of theme.json, `{"on": true, "apps": [ids in priority order]}`; on, with the default
/// order, when it is missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub on: bool,
    pub order: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { on: true, order: DEFAULT_ORDER.iter().map(|s| (*s).to_owned()).collect() }
    }
}

fn plain(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

impl Settings {
    /// From the whole theme document. Ids that are not plain app ids or repeat an earlier one are dropped, and at most
    /// [`MAX_APPS`] are kept; a missing or non-list `apps` is the default order.
    pub fn from_theme(theme: &Value) -> Self {
        let node = theme.pointer("/system/prewarm");
        let on = node.and_then(|n| n.get("on")).and_then(Value::as_bool).unwrap_or(true);
        let Some(list) = node.and_then(|n| n.get("apps")).and_then(Value::as_array) else { return Self { on, ..Self::default() } };
        let mut order: Vec<String> = Vec::new();
        for id in list.iter().filter_map(Value::as_str).filter(|id| plain(id)) {
            if order.len() < MAX_APPS && !order.iter().any(|o| o == id) {
                order.push(id.to_owned());
            }
        }
        Self { on, order }
    }
}

/// What the policy is told each step.
pub struct Inputs<'a> {
    pub now_ms: u64,
    pub screen_on: bool,
    /// LG's pixel refresher is running.
    pub refresher: bool,
    /// A game or game stream is in front or game mode is on.
    pub game: bool,
    /// The app on screen, when known.
    pub front: Option<&'a str>,
    /// SAM's running apps.
    pub running: &'a [String],
    /// Apps holding an audio decoder or output; `None` while the media server has not answered.
    pub playing: Option<&'a BTreeSet<String>>,
    /// `min(usable, MemAvailable)` in MB; `None` when unreadable.
    pub headroom_mb: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// A hidden preload launch.
    Launch(String),
    Close { app: String, why: &'static str },
    /// The app came to the front when it was meant to stay hidden: SAM is asked for Home.
    Home,
}

#[derive(Default)]
struct Slot {
    cost_mb: Option<u64>,
    /// The app showed itself or made sound when preloaded: not launched again until the helper restarts.
    ignored: Option<&'static str>,
    cool_until: u64,
    launched: VecDeque<u64>,
    /// When an eviction closed it.
    evicted: Option<u64>,
}

struct Pending {
    app: String,
    at: u64,
    before_mb: Option<u64>,
    checked: bool,
}

#[derive(Default)]
pub struct Policy {
    slots: BTreeMap<String, Slot>,
    pending: Option<Pending>,
    screen_since: Option<u64>,
    home_since: Option<u64>,
    next_launch: u64,
    next_evict: u64,
    /// Why nothing is being launched, for the topic.
    waiting: &'static str,
}

impl Policy {
    pub fn step(&mut self, s: &Settings, i: &Inputs) -> Vec<Action> {
        let mut out = Vec::new();
        self.screen_since = if i.screen_on { Some(self.screen_since.unwrap_or(i.now_ms)) } else { None };
        self.home_since = if i.front == Some(HOME) { Some(self.home_since.unwrap_or(i.now_ms)) } else { None };
        if !s.on {
            self.pending = None;
            self.waiting = "off";
            return out;
        }
        if !i.screen_on {
            self.pending = None;
            self.waiting = "screen off";
            return out;
        }
        self.check_pending(i, &mut out);
        self.evict(s, i, &mut out);
        if out.is_empty() {
            self.launch(s, i, &mut out);
        }
        out
    }

    /// A launch is being watched, so SAM's list is wanted fresh.
    pub fn watching(&self) -> bool {
        self.pending.is_some()
    }

    /// SAM refused the launch: the app is left alone for [`COOL_MS`].
    pub fn failed(&mut self, app: &str, now_ms: u64) {
        self.slot(app).cool_until = now_ms + COOL_MS;
        if self.pending.as_ref().is_some_and(|p| p.app == app) {
            self.pending = None;
        }
    }

    fn slot(&mut self, app: &str) -> &mut Slot {
        self.slots.entry(app.to_owned()).or_default()
    }

    /// The launch in flight: shown or sounding means the app ignored the preload mode; otherwise its cost is read once it settled.
    fn check_pending(&mut self, i: &Inputs, out: &mut Vec<Action>) {
        let Some(p) = &mut self.pending else { return };
        let age = i.now_ms.saturating_sub(p.at);
        let running = i.running.contains(&p.app);
        let app = p.app.clone();
        let sounding = i.playing.is_some_and(|set| set.contains(&app));
        if running && age >= CHECK_MS && !p.checked {
            p.checked = true;
            if i.front == Some(app.as_str()) {
                out.push(Action::Home);
                out.push(Action::Close { app: app.clone(), why: "shown when preloaded" });
                self.slot(&app).ignored = Some("shows itself when preloaded");
                self.pending = None;
                return;
            }
        }
        if sounding {
            out.push(Action::Close { app: app.clone(), why: "sound when preloaded" });
            self.slot(&app).ignored = Some("plays sound when preloaded");
            self.pending = None;
            return;
        }
        let Some(p) = &self.pending else { return };
        if running && age >= SETTLE_MS {
            let cost = match (p.before_mb, i.headroom_mb) {
                (Some(before), Some(now)) => Some(before.saturating_sub(now).clamp(10, 300)),
                _ => None,
            };
            if let Some(c) = cost {
                self.slot(&app).cost_mb = Some(c);
            }
            self.pending = None;
        } else if !running && age >= LAUNCH_WAIT_MS {
            self.slot(&app).cool_until = i.now_ms + COOL_MS;
            self.pending = None;
        }
    }

    /// Under pressure, the lowest-priority app that is running, out of sight and silent goes first.
    fn evict(&mut self, s: &Settings, i: &Inputs, out: &mut Vec<Action>) {
        let Some(room) = i.headroom_mb else { return };
        if room >= EVICT_BELOW_MB || i.now_ms < self.next_evict || !out.is_empty() {
            return;
        }
        let victim = s.order.iter().rev().find(|app| {
            i.running.iter().any(|r| r == *app) && i.front != Some(app.as_str()) && i.playing.is_some_and(|p| !p.contains(*app))
        });
        if let Some(app) = victim.cloned() {
            self.next_evict = i.now_ms + EVICT_GAP_MS;
            self.slot(&app).evicted = Some(i.now_ms);
            self.slot(&app).cool_until = i.now_ms + COOL_MS;
            if self.pending.as_ref().is_some_and(|p| p.app == app) {
                self.pending = None;
            }
            out.push(Action::Close { app, why: "memory" });
        }
    }

    fn launch(&mut self, s: &Settings, i: &Inputs, out: &mut Vec<Action>) {
        self.waiting = "";
        let reason = if self.pending.is_some() {
            "launching"
        } else if i.refresher {
            "pixel refresher"
        } else if i.game {
            "game"
        } else if self.screen_since.is_none_or(|t| i.now_ms.saturating_sub(t) < SCREEN_DWELL_MS) {
            "screen just on"
        } else if self.home_since.is_none_or(|t| i.now_ms.saturating_sub(t) < HOME_DWELL_MS) {
            "Home not idle"
        } else if i.playing.is_none_or(|p| !p.is_empty()) {
            "sound playing"
        } else if i.now_ms < self.next_launch {
            "between launches"
        } else {
            ""
        };
        if !reason.is_empty() {
            self.waiting = reason;
            return;
        }
        let Some(room) = i.headroom_mb else {
            self.waiting = "memory unknown";
            return;
        };
        for app in &s.order {
            if i.running.iter().any(|r| r == app) {
                continue;
            }
            let slot = self.slots.entry(app.clone()).or_default();
            if slot.ignored.is_some() || shows_when_preloaded(app) {
                continue;
            }
            while slot.launched.front().is_some_and(|t| i.now_ms.saturating_sub(*t) >= WINDOW_MS) {
                slot.launched.pop_front();
            }
            if i.now_ms < slot.cool_until || slot.launched.len() >= MAX_LAUNCHES {
                self.waiting = "cooling off";
                break;
            }
            let need = slot.cost_mb.unwrap_or_else(|| assumed_cost(app)) + START_FLOOR_MB + if slot.evicted.is_some() { REARM_EXTRA_MB } else { 0 };
            if room < need {
                self.waiting = "not enough memory";
                break;
            }
            slot.launched.push_back(i.now_ms);
            self.next_launch = i.now_ms + LAUNCH_GAP_MS;
            self.pending = Some(Pending { app: app.clone(), at: i.now_ms, before_mb: Some(room), checked: false });
            out.push(Action::Launch(app.clone()));
            return;
        }
    }

    /// The topic: per listed app `ready`, `launching`, `cold` or `skipped`, and what holds launches back.
    pub fn status(&self, s: &Settings, running: &[String], headroom_mb: Option<u64>) -> Value {
        let apps: Vec<Value> = s
            .order
            .iter()
            .map(|app| {
                let slot = self.slots.get(app);
                let state = if slot.is_some_and(|sl| sl.ignored.is_some()) || (shows_when_preloaded(app) && !running.iter().any(|r| r == app)) {
                    "skipped"
                } else if self.pending.as_ref().is_some_and(|p| p.app == *app) {
                    "launching"
                } else if running.iter().any(|r| r == app) {
                    "ready"
                } else {
                    "cold"
                };
                let note = slot.and_then(|sl| sl.ignored).or_else(|| shows_when_preloaded(app).then_some("opens normally the first time"));
                json!({"id": app, "state": state, "costMb": slot.and_then(|sl| sl.cost_mb), "note": note})
            })
            .collect();
        json!({"on": s.on, "order": s.order, "apps": apps, "headroomMb": headroom_mb, "waiting": self.waiting})
    }
}
