//! Foreground priority: the app on screen comes first and an app left in the background cannot slow it, after Android's process
//! states (`ProcessList`, `CachedAppOptimizer`, lmkd). Core, always on while the helper runs; `appPriority.enabled: false` in
//! `helper.json` is the diagnostic off switch (with it off, the journal of an earlier run is still put back at start).
//!
//! **States.** From SAM: the apps on screen (`getForegroundAppInfo` with `extraInfo`), each app's life status (`getAppLifeStatus`),
//! the running apps (`running`) and the media server's pipelines (`getActivePipelines`). An app is
//! - *top*: first on screen; *visible*: on screen beside it (Multi View); *launching*: a launch-like status within [`Timing::launch_grace`];
//! - *perceptible*: in the background holding an audio decoder or a sound output (playing, or paused with its media loaded);
//! - *cached*: every other running app, ranked by when it left the screen, the longest gone highest.
//!
//! **What each state gets.**
//! - `oom_score_adj`: top `min(original, 0)`, visible and launching `min(original, 100)`, perceptible `min(original, 200)`, cached
//!   900 + 10 per rank up to 999 (Android's `FOREGROUND_APP_ADJ`, `VISIBLE_APP_ADJ`, `PERCEPTIBLE_APP_ADJ`, `CACHED_APP_MIN_ADJ`..
//!   `CACHED_APP_MAX_ADJ`). SAM starts every app at -1000 (`sam.service` `OOMScoreAdjust`), so in practice only cached apps change,
//!   and the kernel's own OOM killer then takes the oldest cached app before any LG service.
//! - CPU: a cached app's processes go to `cpu,cpuacct/lumatv-background` (`cpu.shares` 52 against the 1024 of everything left in the
//!   root group, Android's background weight). Only processes whose every thread is in the root group move, so going back is exact.
//! - Memory: a cached app's `memory.soft_limit_in_bytes` goes to 0, so the kernel's reclaim takes from cached apps first.
//! - Freezer: a cached app is frozen [`Timing::freeze_after`] after it left the screen (Android's freezer debounce, 10 s) through the
//!   app's freezer group, only when that reads `THAWED`; it is thawed at the first status that is not `background`.
//! - Pressure (lmkd, [`pressure`]): one sample a second while a cached app could be reclaimed or closed. Medium pressure reclaims
//!   the oldest frozen app's memory (`memory.force_empty`, what memchute's hibernation does) on its own thread; lmkd's reasons close
//!   the oldest cached app through SAM, thawed first, with a toast naming it. One close at a time: the next waits until the app's
//!   processes are gone (or [`Timing::close_wait`]) and lmkd's 100 ms after, at most three a minute, and only after two seconds of
//!   pressure unless every task stalled.
//!
//! **Never touched.** LG's apps (`com.webos.`, `com.lge.`, `com.palm.`), anything not in SAM's list (LG's services, the media and
//! audio daemons, the compositor, this helper), processes owned by root, apps with no memory group of their own, and LumaTV's own
//! apps (`org.nphil.`, the brand's): a LumaGame stream behind Home still has to answer its keepalives.
//!
//! **memchute.** While LG's memory manager serves (stock mode), what the policy does only so as not to fight it (the apps it keeps
//! are never frozen or closed, its swappiness is left alone) is in [`memchute`]. While LumaTV's shim serves its name instead (`mm`,
//! switched on with `lumatv mm on`), the engine does memchute's jobs too ([`manager`]): LG's background apps (and AirPlay) are ranked
//! and closed after every third-party app but never frozen or demoted, and Home and memchute's keep lists are never closed.
//!
//! **Undo.** Every change is in a journal (`/run/lumatv-app-priority/state.json`) with the value it replaced; an app that comes back
//! gets its values back at once, and the helper's stop, the off switch, a lost SAM subscription, the return to stock mode and the
//! next start after a crash put back everything.
//!
//! Topic `app_priority`: `{enabled, front, apps: [{id, state, adj, background, frozen, softLimit}], pressure, closed, memoryManager}`;
//! `memoryManager` is `{serving: "lg" | "lumatv", level, usableMB, preload, admitting}` with the link's `{switch, crash, gaveUp}`.
//!
//! Game mode ([`game`]) sits on top: while a native game is in front it closes and reclaims what the policy would only demote.

pub mod game;
pub mod levels;
pub mod lg;
pub mod manager;
pub mod memchute;
pub mod mm;
pub mod os;
pub mod pressure;

use crate::ctx::Ctx;
use crate::luna::Follow;
use lg::{Lg, Luna, Running};
use memchute::Memchute;
use os::{Fs, Os, Pid};
use pressure::{Reason, Verdict, Watch};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const TOPIC: &str = "app_priority";
const JOURNAL: &str = "/run/lumatv-app-priority/state.json";
const LG_PREFIXES: [&str; 3] = ["com.webos.", "com.lge.", "com.palm."];
const OURS_PREFIX: &str = "org.nphil.";
/// LG's apps without an LG prefix, in ours mode closed with LG's.
const LG_APPS: [&str; 1] = ["airplay"];
/// Home, never closed in ours mode even when no configuration layer names it.
const HOME: &str = "com.webos.app.home";
/// A cached app's soft limit: everything it holds is over it.
const CACHED_SOFT_LIMIT: u64 = 0;

#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub freeze_after: Duration,
    /// How long a launch-like status keeps an app out of the cached state while it comes up.
    pub launch_grace: Duration,
    /// How often SAM's list and the pipelines are asked again while an app is cached.
    pub refresh: Duration,
    pub sample: Duration,
    pub close_wait: Duration,
    /// lmkd's `kill_timeout_ms`.
    pub kill_timeout: Duration,
    pub closes_per_minute: usize,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            freeze_after: Duration::from_secs(10),
            launch_grace: Duration::from_secs(5),
            refresh: Duration::from_secs(30),
            sample: Duration::from_secs(1),
            close_wait: Duration::from_secs(3),
            kill_timeout: Duration::from_millis(100),
            closes_per_minute: 3,
        }
    }
}

/// How much of the policy an app gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// memchute keeps it: no freeze, no close.
    Kept,
    /// LG's own, in ours mode: ranked and closed after every other cached app, never frozen or demoted.
    Lg,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Band {
    Top,
    Visible,
    Launching,
    Perceptible,
    Cached(u16),
}

impl Band {
    fn adj(self, original: i32) -> i32 {
        match self {
            Band::Top => original.min(0),
            Band::Visible | Band::Launching => original.min(100),
            Band::Perceptible => original.min(200),
            Band::Cached(rank) => (900 + 10 * i32::from(rank)).min(999),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Band::Top => "top",
            Band::Visible => "visible",
            Band::Launching => "launching",
            Band::Perceptible => "perceptible",
            Band::Cached(_) => "cached",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ProcHeld {
    start: u64,
    /// The value replaced, while the process holds another.
    #[serde(default)]
    oom: Option<i32>,
    /// In the background CPU group.
    #[serde(default)]
    cpu: bool,
}

/// What the policy changed of one app, with what it replaced.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Held {
    #[serde(default)]
    procs: BTreeMap<i32, ProcHeld>,
    /// The value a process the app starts while demoted had before it inherited the demotion.
    #[serde(default)]
    base_oom: Option<i32>,
    #[serde(default)]
    frozen: bool,
    #[serde(default)]
    soft: Option<u64>,
    #[serde(skip)]
    trimmed: bool,
}

impl Held {
    fn is_empty(&self) -> bool {
        self.procs.is_empty() && !self.frozen && self.soft.is_none()
    }
}

struct Seen {
    left_front: Instant,
    launching_until: Option<Instant>,
}

pub type Runner = Box<dyn Fn(Box<dyn FnOnce() + Send>) + Send>;

pub struct Engine {
    os: Arc<dyn Os>,
    lg: Arc<dyn Lg>,
    timing: Timing,
    memchute: Option<Memchute>,
    ours: Vec<String>,
    journal: Option<PathBuf>,
    /// Runs work that may block (a reclaim, a toast) away from the policy's thread.
    run: Runner,
    started: Instant,
    front: Option<Vec<String>>,
    running: Vec<String>,
    /// `None` until the media server answers: no app is frozen or closed while it is unknown which one is playing.
    playing: Option<BTreeSet<String>>,
    seen: BTreeMap<String, Seen>,
    held: BTreeMap<String, Held>,
    refreshed: Option<Instant>,
    sampled: Option<Instant>,
    watch: Watch,
    victim: Option<(String, Instant)>,
    quiet_until: Option<Instant>,
    closes: VecDeque<Closed>,
    trimming: Arc<AtomicBool>,
    game: game::Game,
    /// SAM's running apps with their processes and types.
    sam: Vec<Running>,
    /// Set while LumaTV's memory manager serves memchute's name.
    mm: Option<manager::Manager>,
}

impl Engine {
    /// `memchute`: LG's memory manager runs beside this one, and the policy keeps out of its way.
    #[allow(clippy::too_many_arguments)] // every outside thing the engine touches, each one a fake in the tests
    pub fn new(
        os: Arc<dyn Os>,
        lg: Arc<dyn Lg>,
        timing: Timing,
        memchute: Option<Memchute>,
        ours: Vec<String>,
        journal: Option<PathBuf>,
        run: Runner,
        now: Instant,
    ) -> Self {
        Self {
            os,
            lg,
            timing,
            memchute,
            ours,
            journal,
            run,
            started: now,
            front: None,
            running: Vec::new(),
            playing: None,
            seen: BTreeMap::new(),
            held: BTreeMap::new(),
            refreshed: None,
            sampled: None,
            watch: Watch::new(pressure::Tuning::default()),
            victim: None,
            quiet_until: None,
            closes: VecDeque::new(),
            sam: Vec::new(),
            mm: None,
            trimming: Arc::new(AtomicBool::new(false)),
            game: game::Game::new(),
        }
    }

    fn class(&self, app: &str) -> Option<Class> {
        if app.starts_with(OURS_PREFIX) || self.ours.iter().any(|o| o == app) {
            return None;
        }
        let lg = LG_PREFIXES.iter().any(|p| app.starts_with(p));
        let memchute = self.memchute.as_ref();
        if self.mm.is_none() {
            return if lg {
                None
            } else if memchute.is_some_and(|m| m.keeps(app)) {
                Some(Class::Kept)
            } else {
                Some(Class::Other)
            };
        }
        if app == HOME || memchute.is_some_and(|m| m.home(app)) {
            None
        } else if memchute.is_some_and(|m| m.keeps_ours(app)) {
            (!lg).then_some(Class::Kept)
        } else if lg || LG_APPS.contains(&app) {
            Some(Class::Lg)
        } else {
            Some(Class::Other)
        }
    }

    /// SAM's list and the pipelines, asked again; an answer that does not come keeps the last.
    pub fn refresh(&mut self, now: Instant) {
        if let Some(running) = self.lg.running() {
            self.running = running.iter().map(|r| r.id.clone()).collect();
            self.sam = running;
            self.mm_groups();
        }
        self.playing = self.lg.playing();
        self.refreshed = Some(now);
    }

    /// The apps on screen changed.
    pub fn front(&mut self, ids: Vec<String>, now: Instant) {
        let left: Vec<String> = self.front.iter().flatten().filter(|id| !ids.contains(id)).cloned().collect();
        for id in ids.iter().chain(&left) {
            let seen = self.seen_mut(id, now);
            (seen.left_front, seen.launching_until) = (now, None);
        }
        self.front = Some(ids);
        self.reconcile(now);
        self.mm_gate();
        self.game_check(now);
    }

    /// An app's life status changed: anything but `background` thaws it at once.
    pub fn life(&mut self, app: &str, status: &str, now: Instant) {
        let grace = self.timing.launch_grace;
        let seen = self.seen_mut(app, now);
        seen.left_front = now;
        seen.launching_until = (status != "background" && status != "stop").then_some(now + grace);
        self.reconcile(now);
        self.refresh(now);
        self.reconcile(now);
    }

    fn seen_mut(&mut self, app: &str, now: Instant) -> &mut Seen {
        self.seen.entry(app.to_owned()).or_insert(Seen { left_front: now, launching_until: None })
    }

    /// The subscriptions ended: with no way to tell which app is in front, nothing stays demoted.
    pub fn lost(&mut self, why: &str) {
        self.game_exit("subscription");
        self.front = None;
        if !self.held.is_empty() {
            self.restore_all(why);
        }
    }

    fn bands(&self, now: Instant) -> BTreeMap<String, Band> {
        let Some(front) = &self.front else { return BTreeMap::new() };
        let mut bands = BTreeMap::new();
        let mut cached: Vec<(Instant, &str)> = Vec::new();
        for app in &self.running {
            if self.class(app).is_none() {
                continue;
            }
            let seen = self.seen.get(app.as_str());
            let band = if front.first() == Some(app) {
                Band::Top
            } else if front.contains(app) {
                Band::Visible
            } else if seen.and_then(|s| s.launching_until).is_some_and(|t| now < t) {
                Band::Launching
            } else if self.playing.as_ref().is_none_or(|p| p.contains(app)) {
                Band::Perceptible
            } else {
                cached.push((seen.map_or(self.started, |s| s.left_front), app));
                continue;
            };
            bands.insert(app.clone(), band);
        }
        cached.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
        for (rank, (_, app)) in cached.into_iter().enumerate() {
            bands.insert(app.to_owned(), Band::Cached(rank.min(usize::from(u16::MAX)) as u16));
        }
        bands
    }

    fn left_front(&self, app: &str) -> Instant {
        self.seen.get(app).map_or(self.started, |s| s.left_front)
    }

    /// Brings every app's processes and groups to what its state asks, and puts back what an app that is gone or no longer
    /// cached was given.
    pub fn reconcile(&mut self, now: Instant) {
        let bands = self.bands(now);
        let gone: Vec<String> = self.held.keys().filter(|a| !bands.contains_key(*a)).cloned().collect();
        for app in gone {
            if let Some(held) = self.held.remove(&app) {
                restore_app(&*self.os, &app, &held);
            }
        }
        for (app, band) in &bands {
            let Some(class) = self.class(app).filter(|c| *c != Class::Lg) else { continue };
            let mut held = self.held.remove(app).unwrap_or_default();
            let cached = matches!(band, Band::Cached(_));
            let freeze = cached
                && class == Class::Other
                && now.saturating_duration_since(self.left_front(app)) >= self.timing.freeze_after;
            self.apply(app, *band, cached, freeze, &mut held);
            if !cached {
                held.trimmed = false;
            }
            if !held.is_empty() {
                self.held.insert(app.clone(), held);
            }
        }
        self.mm_swappiness(now);
        self.save();
    }

    fn apply(&self, app: &str, band: Band, cached: bool, freeze: bool, held: &mut Held) {
        let os = &*self.os;
        if !freeze && held.frozen {
            match os.set_freezer(app, "THAWED") {
                Ok(()) => {
                    held.frozen = false;
                    crate::log_info!("app priority: thawed {app}");
                }
                Err(e) => crate::log_warn!("app priority: thawing {app}: {e}"),
            }
        }
        let procs = os.app_procs(app).unwrap_or_default();
        held.procs.retain(|pid, h| procs.iter().any(|p| p.pid == *pid && p.start == h.start));
        if held.procs.is_empty() {
            held.base_oom = None;
        }
        for p in &procs {
            let mut h = held.procs.remove(&p.pid).unwrap_or(ProcHeld { start: p.start, oom: None, cpu: false });
            if let Some(current) = os.oom(*p) {
                // 900..=999 is only ever this policy's cached range: a process the app started while demoted inherited it.
                let inherited = cached && (900..=999).contains(&current);
                let original = h.oom.or(held.base_oom.filter(|_| inherited)).unwrap_or(current);
                let target = band.adj(original);
                if target == current || os.set_oom(*p, target).is_ok() {
                    h.oom = (target != original).then_some(original);
                    if h.oom.is_some() && held.base_oom.is_none() {
                        held.base_oom = Some(original);
                    }
                }
            }
            if cached && !h.cpu && os.cpu_root(*p) {
                h.cpu = os.to_background(*p).is_ok();
            } else if !cached && h.cpu && os.to_root(p.pid).is_ok() {
                h.cpu = false;
            }
            if h.oom.is_some() || h.cpu {
                held.procs.insert(p.pid, h);
            }
        }
        if cached && held.soft.is_none() {
            if let Some(original) = os.soft_limit(app).filter(|&v| v != CACHED_SOFT_LIMIT) {
                if os.set_soft_limit(app, CACHED_SOFT_LIMIT).is_ok() {
                    held.soft = Some(original);
                }
            }
        } else if let Some(original) = held.soft.filter(|_| !cached) {
            if os.set_soft_limit(app, original).is_ok() {
                held.soft = None;
            }
        }
        if freeze && !held.frozen && os.freezer(app).as_deref() == Some("THAWED") {
            match os.set_freezer(app, "FROZEN") {
                Ok(()) => {
                    held.frozen = true;
                    crate::log_info!("app priority: froze {app}");
                }
                Err(e) => crate::log_warn!("app priority: freezing {app}: {e}"),
            }
        }
    }

    /// Whether some cached app could be reclaimed or closed, which is what sampling the pressure is for: the oldest first, and in
    /// ours mode LG's after every other.
    fn closable(&self, bands: &BTreeMap<String, Band>) -> Vec<String> {
        let mut apps: Vec<(bool, u16, u64, String)> = bands
            .iter()
            .filter_map(|(app, band)| match (band, self.class(app)) {
                (Band::Cached(rank), Some(c @ (Class::Other | Class::Lg))) => Some((c == Class::Lg, *rank, self.os.usage(app), app.clone())),
                _ => None,
            })
            .collect();
        apps.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
        apps.into_iter().map(|(_, _, _, app)| app).collect()
    }

    /// The timers: SAM asked again, freezes that are due, and a pressure sample while something could be closed.
    pub fn tick(&mut self, now: Instant) {
        self.mm_tick(now);
        if self.front.is_none() {
            return;
        }
        self.game_check(now);
        let due = |at: Option<Instant>, every: Duration| at.is_none_or(|t| now.saturating_duration_since(t) >= every);
        if !self.held.is_empty() && due(self.refreshed, self.timing.refresh) {
            self.refresh(now);
        }
        self.reconcile(now);
        let bands = self.bands(now);
        if self.closable(&bands).is_empty() {
            self.watch.pause();
            self.sampled = None;
            return;
        }
        if !due(self.sampled, self.timing.sample) {
            return;
        }
        self.sampled = Some(now);
        let Some(sample) = self.os.sample() else { return };
        if let Some((app, at)) = &self.victim {
            let exited = self.os.app_procs(app).is_none_or(|p| p.is_empty());
            if exited || now.saturating_duration_since(*at) >= self.timing.close_wait {
                self.victim = None;
                self.quiet_until = Some(now + self.timing.kill_timeout);
            }
        }
        match self.watch.step(now, sample) {
            Verdict::Calm => {}
            Verdict::Trim => self.trim(&bands),
            Verdict::Kill(reason) => {
                if !self.close(reason, now) {
                    self.trim(&bands);
                }
            }
        }
    }

    /// Reclaims the oldest frozen app not reclaimed since it was cached, one at a time.
    fn trim(&mut self, bands: &BTreeMap<String, Band>) {
        if self.trimming.load(Ordering::Relaxed) {
            return;
        }
        let Some(app) = self.closable(bands).into_iter().find(|a| self.held.get(a).is_some_and(|h| h.frozen && !h.trimmed)) else {
            return;
        };
        if let Some(held) = self.held.get_mut(&app) {
            held.trimmed = true;
        }
        self.trimming.store(true, Ordering::Relaxed);
        let (os, flag) = (self.os.clone(), self.trimming.clone());
        crate::log_info!("app priority: reclaiming {app}'s memory");
        (self.run)(Box::new(move || {
            if let Err(e) = os.reclaim(&app) {
                crate::log_warn!("app priority: reclaiming {app}: {e}");
            }
            flag.store(false, Ordering::Relaxed);
        }));
    }

    /// Closes the oldest cached app when the limits allow; `false` when nothing was closed.
    fn close(&mut self, reason: Reason, now: Instant) -> bool {
        while self.closes.front().is_some_and(|c| now.saturating_duration_since(c.at) >= Duration::from_secs(60)) {
            self.closes.pop_front();
        }
        let stall = reason == Reason::Stall;
        // Launch admissions close on their own schedule and are not held to the cap.
        if self.victim.is_some()
            || self.mm_admitting()
            || self.quiet_until.is_some_and(|t| now < t)
            || self.closes.iter().filter(|c| !c.launch).count() >= self.timing.closes_per_minute
            || (!stall && (self.watch.streak < 2 || self.trimming.load(Ordering::Relaxed)))
        {
            return false;
        }
        self.refresh(now);
        self.reconcile(now);
        let Some(app) = self.closable(&self.bands(now)).into_iter().next() else {
            return false;
        };
        if !self.close_app(&app, reason.text(), false, now) {
            self.quiet_until = Some(now + self.timing.close_wait);
            return false;
        }
        self.mm_cleared();
        self.watch.closed(reason);
        self.victim = Some((app.clone(), now));
        let lg = self.lg.clone();
        (self.run)(Box::new(move || {
            let title = lg.title(&app).unwrap_or(app);
            lg.toast(&format!("Closed {title} to free memory for the app on screen"));
        }));
        true
    }

    /// Closes one app through SAM, thawed first; in ours mode WAM hears of it before.
    fn close_app(&mut self, app: &str, why: &str, launch: bool, now: Instant) -> bool {
        if let Some(held) = self.held.get_mut(app).filter(|h| h.frozen) {
            if self.os.set_freezer(app, "THAWED").is_ok() {
                held.frozen = false;
            }
        }
        self.mm_closing(app);
        if !self.lg.close(app) {
            crate::log_warn!("app priority: SAM did not close {app}");
            return false;
        }
        crate::log_warn!("app priority: closed {app} ({why})");
        self.closes.push_back(Closed { at: now, app: app.to_owned(), why: why.to_owned(), launch });
        self.save();
        true
    }

    /// When the policy next has something to do on its own.
    pub fn next(&self, now: Instant) -> Option<Instant> {
        let mut next: Option<Instant> = self.mm_next(now);
        if self.front.is_none() {
            return next.map(|t| t.max(now));
        }
        if let Some(t) = self.game_next(now) {
            next = Some(next.map_or(t, |n| n.min(t)));
        }
        let bands = self.bands(now);
        let mut at = |t: Instant| next = Some(next.map_or(t, |n| n.min(t)));
        for (app, band) in &bands {
            if let Some(t) = self.seen.get(app).and_then(|s| s.launching_until) {
                at(t);
            }
            if matches!(band, Band::Cached(_)) && self.class(app) == Some(Class::Other) && !self.held.get(app).is_some_and(|h| h.frozen) {
                at(self.left_front(app) + self.timing.freeze_after);
            }
        }
        if !self.held.is_empty() {
            at(self.refreshed.map_or(now, |t| t + self.timing.refresh));
        }
        if !self.closable(&bands).is_empty() {
            at(self.sampled.map_or(now, |t| t + self.timing.sample));
        }
        next.map(|t| t.max(now))
    }

    pub fn restore_all(&mut self, why: &str) {
        for (app, held) in std::mem::take(&mut self.held) {
            restore_app(&*self.os, &app, &held);
        }
        if let Err(e) = self.os.drop_background() {
            crate::log_warn!("app priority: removing the background group: {e}");
        }
        self.save();
        crate::log_info!("app priority: everything put back ({why})");
    }

    fn save(&self) {
        let Some(path) = &self.journal else { return };
        if self.held.is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, serde_json::to_vec(&self.held).map_err(std::io::Error::other)?)?;
            std::fs::rename(tmp, path)
        };
        if let Err(e) = write() {
            crate::log_warn!("app priority: journal: {e}");
        }
    }

    pub fn status(&self, now: Instant) -> Value {
        let bands = self.bands(now);
        let apps: Vec<Value> = bands
            .iter()
            .map(|(app, band)| {
                let held = self.held.get(app);
                let adj = self.os.app_procs(app).and_then(|p| p.first().and_then(|p| self.os.oom(*p)));
                json!({
                    "id": app,
                    "state": band.name(),
                    "adj": adj,
                    "background": held.is_some_and(|h| h.procs.values().any(|p| p.cpu)),
                    "frozen": held.is_some_and(|h| h.frozen),
                    "softLimit": held.is_some_and(|h| h.soft.is_some()),
                })
            })
            .collect();
        json!({
            "enabled": true,
            "front": self.front,
            "apps": apps,
            "pressure": {"episode": self.watch.episode, "someMs": self.watch.some_ms, "fullMs": self.watch.full_ms, "thrashing": self.watch.thrashing},
            "closed": self.closes.iter().map(|c| json!({"id": c.app, "reason": c.why})).collect::<Vec<_>>(),
            "memoryManager": self.mm_status(),
        })
    }
}

/// Puts back what was changed of one app, as far as each process is still the one it was.
fn restore_app(os: &dyn Os, app: &str, held: &Held) {
    if held.frozen && matches!(os.freezer(app).as_deref(), Some("FROZEN" | "FREEZING")) {
        if let Err(e) = os.set_freezer(app, "THAWED") {
            crate::log_warn!("app priority: thawing {app}: {e}");
        }
    }
    if let Some(original) = held.soft {
        let _ = os.set_soft_limit(app, original);
    }
    for (&pid, h) in &held.procs {
        let p = Pid { pid, start: h.start };
        if !os.alive(p) {
            continue;
        }
        if let Some(original) = h.oom {
            let _ = os.set_oom(p, original);
        }
        if h.cpu {
            let _ = os.to_root(pid);
        }
    }
}

/// Puts back what a run that did not end cleanly left changed.
pub fn recover(os: &dyn Os, journal: &Path) {
    if let Some(held) = std::fs::read(journal).ok().and_then(|b| serde_json::from_slice::<BTreeMap<String, Held>>(&b).ok()) {
        for (app, held) in &held {
            restore_app(os, app, held);
        }
        crate::log_info!("app priority: put back {} app(s) an earlier run left changed", held.len());
    }
    let _ = std::fs::remove_file(journal);
    let _ = os.drop_background();
}

/// A close, for the status and the cap.
struct Closed {
    at: Instant,
    app: String,
    why: String,
    /// For a launch's admission, which the cap does not count.
    launch: bool,
}

pub(crate) enum Msg {
    Front(Vec<String>),
    Life(String, String),
    /// The API's game mode request, answered with game mode's status.
    Game(bool, String, mpsc::Sender<Value>),
    /// LumaTV's shim serves memchute's name.
    MmUp(Arc<dyn manager::Shim>),
    /// It no longer does: LG's memchute serves, or the link broke.
    MmDown(String),
    /// `setRequiredMemory`'s payload, already answered by the shim.
    Launch(Value),
    Stop,
}

/// Puts back an earlier run's changes and, unless switched off, starts the policy's thread.
pub fn spawn(ctx: &Ctx, remote: Arc<game::Remote>, policy: ha_link::AddressPolicy) -> std::io::Result<JoinHandle<()>> {
    let os: Arc<dyn Os> = Arc::new(Fs::lg());
    recover(&*os, Path::new(JOURNAL));
    let ctx = ctx.clone();
    thread::Builder::new()
        .name("app-priority".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || {
            if ctx.config.app_priority.enabled {
                run(&ctx, os, &remote, policy);
            } else {
                ctx.hub.set(TOPIC, json!({"enabled": false}));
                // Game mode is part of the policy: off with it.
                ctx.hub.set(game::TOPIC, json!({"switch": false, "on": false, "app": null, "since": null, "steps": [], "lastExit": null}));
            }
        })
}

fn run(ctx: &Ctx, os: Arc<dyn Os>, remote: &game::Remote, policy: ha_link::AddressPolicy) {
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        ctx.stop.on_stop(move || {
            let _ = tx.send(Msg::Stop);
        });
    }
    let luna = ctx.config.luna.send.clone();
    let lg: Arc<dyn Lg> = Arc::new(Luna::new(&luna));
    let ours = ctx.config.brand_app_id.iter().cloned().collect();
    let run: Runner = Box::new(|work| {
        if let Err(e) = thread::Builder::new().name("app-priority-work".to_owned()).stack_size(128 * 1024).spawn(work) {
            crate::log_warn!("app priority: {e}");
        }
    });
    let memchute = Some(Memchute::read(Path::new(memchute::LAYERS)));
    let mut engine = Engine::new(os, lg, Timing::default(), memchute, ours, Some(PathBuf::from(JOURNAL)), run, Instant::now());
    engine.set_game(game::theme_switch(ctx.paths.theme_file()), &game::FEEDS);
    remote.attach(Some(tx.clone()));
    let link = mm::Link::spawn(ctx, tx.clone(), policy);
    let mut follows: Vec<Follow> = Vec::new();
    let mut retry_at: Option<Instant> = None;
    loop {
        let now = Instant::now();
        if follows.len() != 2 || !follows.iter().all(Follow::alive) {
            if !follows.is_empty() {
                engine.lost("SAM subscription ended");
                follows.clear();
                retry_at = Some(now + Duration::from_secs(10));
            }
            if retry_at.is_none_or(|t| now >= t) {
                match subscribe(&luna, &tx) {
                    Ok(started) => {
                        follows = started;
                        engine.refresh(now);
                    }
                    Err(e) => {
                        crate::log_warn!("app priority: subscribing to SAM: {e}");
                        retry_at = Some(now + Duration::from_secs(10));
                    }
                }
            }
        }
        let wait = engine.next(now).map_or(Duration::from_secs(10), |t| t.saturating_duration_since(now)).min(Duration::from_secs(10));
        match rx.recv_timeout(wait) {
            Ok(Msg::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Front(ids)) => engine.front(ids, Instant::now()),
            Ok(Msg::Life(app, status)) => engine.life(&app, &status, Instant::now()),
            Ok(Msg::Game(on, app, reply)) => {
                let _ = reply.send(engine.game_request(on, &app, Instant::now()));
            }
            Ok(Msg::MmUp(shim)) => engine.take_over(shim, Instant::now()),
            Ok(Msg::MmDown(why)) => engine.give_back(&why),
            Ok(Msg::Launch(request)) => engine.launch(&request, Instant::now()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let now = Instant::now();
        engine.tick(now);
        let mut status = engine.status(now);
        link.annotate(&mut status);
        ctx.hub.set_ignoring(TOPIC, status, &["pressure"]);
        ctx.hub.set(game::TOPIC, engine.game_status());
    }
    remote.attach(None);
    // The shim answers alone from the last state and gives the name back if the helper does not return within its grace.
    link.stop();
    drop(follows);
    engine.game_exit("stop");
    engine.restore_all("helper stop");
    ctx.hub.set(TOPIC, engine.status(Instant::now()));
    ctx.hub.set(game::TOPIC, engine.game_status());
}

fn subscribe(luna: &str, tx: &mpsc::Sender<Msg>) -> std::io::Result<Vec<Follow>> {
    let front_tx = tx.clone();
    let front = Follow::start(
        "app-priority-front",
        luna,
        lg::FRONT_URI,
        lg::FRONT_PAYLOAD,
        Box::new(move |reply| {
            if let Some(ids) = lg::front(reply) {
                let _ = front_tx.send(Msg::Front(ids));
            }
        }),
    )?;
    let life_tx = tx.clone();
    let life = Follow::start(
        "app-priority-life",
        luna,
        lg::LIFE_URI,
        lg::LIFE_PAYLOAD,
        Box::new(move |reply| {
            if let Some((app, status)) = lg::life(reply) {
                let _ = life_tx.send(Msg::Life(app, status));
            }
        }),
    )?;
    Ok(vec![front, life])
}

#[cfg(test)]
mod tests;
