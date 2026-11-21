//! Game mode: while a native game is in front, as much memory as the TV can spare goes to it, at the cost of a slower way back to
//! Home. SAM has no game category, so the games are named ([`GAMES`]); a game the list misses asks through `POST /v1/gamemode`.
//!
//! **Entry**, in order: background apps that are neither LG's, LumaTV's, kept by memchute nor holding audio are closed through SAM
//! with a toast naming them (none at all while it is unknown who plays); their exit is awaited up to [`CLOSE_WAIT`]; Home's feeds and
//! the guide's downloads pause ([`FEEDS`]); then on a worker thread, with an abort check between steps, the memory groups of the apps
//! left (but the game's, those on screen, those holding audio and LumaTV's) are reclaimed (`memory.force_empty`), memory is compacted
//! and the clean page cache dropped once. Each step logs what it freed (the change of `MemAvailable`).
//!
//! **Exit**: the worker is told to stop, the feeds resume. [`EXIT_AFTER`] after the game left the front; at once when the switch
//! (`system.gameMode` of theme.json) goes off, an API lease lapses, the request says `on: false`, an entry step fails, the SAM
//! subscription ends or the helper stops. Nothing of the game itself is touched, nor any LG service; a closed app is not reopened.
//!
//! Topic `gamemode` and `GET /v1/gamemode`: `{switch, on, app, since, steps: [{step, freedMb, ms, note}], lastExit: {reason, exitMs}}`.

use super::{Class, Engine, Msg, OURS_PREFIX};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const TOPIC: &str = "gamemode";
/// The native games: LumaGame, GeForce NOW and Punktfunk.
pub const GAMES: [&str; 3] = ["org.nphil.lumagame", "org.nphil.lumatv.gfn", "io.unom.punktfunk.client-webos"];
/// How long one `POST /v1/gamemode {on: true}` counts.
pub const LEASE: Duration = Duration::from_secs(30);
/// A game that left the front this long ago ends game mode: a short trip to a menu or a pop-up does not.
pub const EXIT_AFTER: Duration = Duration::from_secs(5);
/// How long the closed apps get to exit before the rest of the entry goes on.
pub const CLOSE_WAIT: Duration = Duration::from_secs(3);
const CLOSE_POLL: Duration = Duration::from_millis(100);
/// While game mode is on: how often the switch is read and the worker looked at.
const POLL: Duration = Duration::from_secs(1);

/// Background work that waits while game mode is on: Home's weather and news and the guide's downloads.
pub struct Pause {
    paused: AtomicBool,
    wakers: Mutex<Vec<Box<dyn Fn() + Send>>>,
}

impl Pause {
    pub const fn new() -> Self {
        Self {
            paused: AtomicBool::new(false),
            wakers: Mutex::new(Vec::new()),
        }
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// `wake` is called when the pause ends, so work that was due runs then rather than at its next timer.
    pub fn on_resume(&self, wake: impl Fn() + Send + 'static) {
        self.wakers.lock().push(Box::new(wake));
    }

    fn set(&self, paused: bool) {
        if self.paused.swap(paused, Ordering::Relaxed) && !paused {
            for wake in self.wakers.lock().iter() {
                wake();
            }
        }
    }
}

impl Default for Pause {
    fn default() -> Self {
        Self::new()
    }
}

/// What `homefeeds` and `guidedata` ask before they fetch.
pub static FEEDS: Pause = Pause::new();

/// Whether the owner's switch is on.
pub type Switch = Box<dyn Fn() -> bool + Send>;

/// The owner's switch: `system.gameMode` of theme.json, on unless it says `false`.
pub fn theme_switch(theme: PathBuf) -> Switch {
    Box::new(move || {
        std::fs::read(&theme)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.pointer("/system/gameMode").and_then(Value::as_bool))
            != Some(false)
    })
}

/// The way in for the API: requests go to the policy's thread, which answers with the status.
#[derive(Default)]
pub struct Remote(Mutex<Option<mpsc::Sender<Msg>>>);

impl Remote {
    pub(super) fn attach(&self, tx: Option<mpsc::Sender<Msg>>) {
        *self.0.lock() = tx;
    }

    /// `None` while the policy is not running.
    pub fn request(&self, on: bool, app: &str) -> Option<Value> {
        let (tx, rx) = mpsc::channel();
        self.0.lock().as_ref()?.send(Msg::Game(on, app.to_owned(), tx)).ok()?;
        rx.recv_timeout(Duration::from_secs(2)).ok()
    }
}

struct Step {
    step: &'static str,
    freed_mb: i64,
    ms: u64,
    note: String,
}

/// One entry's record, shared with its worker.
#[derive(Default)]
struct Work {
    steps: Mutex<Vec<Step>>,
    abort: AtomicBool,
    failed: Mutex<Option<String>>,
}

impl Work {
    fn push(&self, step: &'static str, before: Option<u64>, after: Option<u64>, took: Duration, note: String) {
        let freed_mb = match (before, after) {
            (Some(b), Some(a)) => (a as i64 - b as i64) / 1024,
            _ => 0,
        };
        let ms = took.as_millis() as u64;
        crate::log_info!("game mode: {step}: {freed_mb} MB in {ms} ms ({note})");
        self.steps.lock().push(Step { step, freed_mb, ms, note });
    }
}

enum Phase {
    /// The closed apps are being waited for.
    Closing { apps: Vec<String>, started: Instant, before: Option<u64>, note: String },
    Working,
}

struct Active {
    app: String,
    since_ms: u64,
    phase: Phase,
    work: Arc<Work>,
}

pub struct Game {
    switch: Switch,
    pause: &'static Pause,
    active: Option<Active>,
    lease: Option<(String, Instant)>,
    /// The app `on: false` was asked for: no automatic entry until it leaves the front.
    suppressed: Option<String>,
    left_front: Option<Instant>,
    last: Option<Arc<Work>>,
    last_exit: Option<(&'static str, u64)>,
}

impl Game {
    /// Switched off until [`Engine::set_game`] says otherwise.
    pub(super) fn new() -> Self {
        Self {
            switch: Box::new(|| false),
            pause: &FEEDS,
            active: None,
            lease: None,
            suppressed: None,
            left_front: None,
            last: None,
            last_exit: None,
        }
    }
}

impl Engine {
    pub fn set_game(&mut self, switch: Switch, pause: &'static Pause) {
        self.game.switch = switch;
        self.game.pause = pause;
    }

    fn top(&self) -> Option<String> {
        self.front.as_ref().and_then(|f| f.first().cloned())
    }

    fn lumatvs(&self, app: &str) -> bool {
        app.starts_with(OURS_PREFIX) || self.ours.iter().any(|o| o == app)
    }

    /// Whether `app` may have game mode, the switch aside: a named game not switched off for this spell in front, or a live lease.
    fn game_eligible(&self, app: &str, now: Instant) -> bool {
        let g = &self.game;
        (GAMES.contains(&app) && g.suppressed.as_deref() != Some(app)) || g.lease.as_ref().is_some_and(|(a, until)| a == app && now < *until)
    }

    /// Game mode's triggers and timers: after every change of the front and at every tick.
    pub(super) fn game_check(&mut self, now: Instant) {
        let top = self.top();
        if self.game.suppressed.is_some() && self.game.suppressed != top {
            self.game.suppressed = None;
        }
        if self.game.lease.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.game.lease = None;
        }
        let switch = (self.game.switch)();
        if let Some(active) = &self.game.active {
            let app = active.app.clone();
            let failed = active.work.failed.lock().clone();
            let why = if !switch {
                Some("switch")
            } else if let Some(e) = failed {
                crate::log_warn!("game mode: {e}");
                self.game_spoiled(&app);
                Some("failed")
            } else if !self.game_eligible(&app, now) {
                Some(if self.game.suppressed.as_deref() == Some(app.as_str()) { "off" } else { "lease" })
            } else {
                None
            };
            if let Some(why) = why {
                self.game_exit(why);
            } else if top.as_deref() != Some(app.as_str()) {
                let left = *self.game.left_front.get_or_insert(now);
                if now.saturating_duration_since(left) >= EXIT_AFTER {
                    self.game_exit("left");
                }
            } else {
                self.game.left_front = None;
                self.game_closing(now);
            }
        }
        if self.game.active.is_none()
            && switch
            && let Some(top) = top
            && self.game_eligible(&top, now)
        {
            self.game_enter(top, now);
        }
    }

    /// An entry step failed: no second entry until the game has left the front.
    fn game_spoiled(&mut self, app: &str) {
        self.game.suppressed = Some(app.to_owned());
        if self.game.lease.as_ref().is_some_and(|(a, _)| a == app) {
            self.game.lease = None;
        }
    }

    /// The API's `{on, appId}`.
    pub fn game_request(&mut self, on: bool, app: &str, now: Instant) -> Value {
        if on {
            if (self.game.switch)() {
                self.game.lease = Some((app.to_owned(), now + LEASE));
                if self.game.suppressed.as_deref() == Some(app) {
                    self.game.suppressed = None;
                }
            }
        } else {
            if self.game.lease.as_ref().is_some_and(|(a, _)| a == app) {
                self.game.lease = None;
            }
            if self.top().as_deref() == Some(app) {
                self.game.suppressed = Some(app.to_owned());
            }
            if self.game.active.as_ref().is_some_and(|a| a.app == app) {
                self.game_exit("off");
            }
        }
        self.game_check(now);
        self.game_status()
    }

    /// Step 1: closes what may be closed, and waits for it in [`Self::game_closing`].
    fn game_enter(&mut self, app: String, now: Instant) {
        self.refresh(now);
        let before = self.os.available_kb();
        let front = self.front.clone().unwrap_or_default();
        let (apps, mut note) = match &self.playing {
            None => (Vec::new(), "it is unknown which app plays: none closed".to_owned()),
            Some(playing) => (
                self.running
                    .iter()
                    .filter(|a| *a != &app && !front.contains(a) && !playing.contains(*a) && self.class(a) == Some(Class::Other))
                    .cloned()
                    .collect::<Vec<_>>(),
                String::new(),
            ),
        };
        let work = Arc::new(Work::default());
        crate::log_info!("game mode: on for {app}");
        self.game.left_front = None;
        self.game.last = Some(work.clone());
        self.game.active = Some(Active { app: app.clone(), since_ms: crate::timeutil::now_ms(), phase: Phase::Working, work: work.clone() });
        for a in &apps {
            if let Some(held) = self.held.get_mut(a).filter(|h| h.frozen) {
                if self.os.set_freezer(a, "THAWED").is_ok() {
                    held.frozen = false;
                }
            }
            self.mm_closing(a);
            if !self.lg.close(a) {
                let why = format!("SAM did not close {a}");
                crate::log_warn!("game mode: {why}");
                work.push("close", before, self.os.available_kb(), Duration::ZERO, why);
                self.game_spoiled(&app);
                self.game_exit("failed");
                return;
            }
        }
        if !apps.is_empty() {
            note = format!("closed {}", apps.join(", "));
            self.mm_cleared();
            let (lg, names) = (self.lg.clone(), apps.clone());
            (self.run)(Box::new(move || {
                let titles: Vec<String> = names.into_iter().map(|a| lg.title(&a).unwrap_or(a)).collect();
                lg.toast(&format!("Game mode closed {} to free memory for the game", titles.join(", ")));
            }));
        } else if note.is_empty() {
            note = "nothing to close".to_owned();
        }
        if let Some(active) = &mut self.game.active {
            active.phase = Phase::Closing { apps, started: now, before, note };
        }
        self.game_closing(now);
    }

    /// Step 2: once the closed apps are gone (or [`CLOSE_WAIT`] passed), steps 3 and 4.
    fn game_closing(&mut self, now: Instant) {
        let os = self.os.clone();
        let Some(active) = &mut self.game.active else { return };
        let Phase::Closing { apps, started, .. } = &active.phase else { return };
        let gone = apps.iter().all(|a| os.app_procs(a).is_none_or(|p| p.is_empty()));
        let waited = now.saturating_duration_since(*started);
        if !gone && waited < CLOSE_WAIT {
            return;
        }
        let Phase::Closing { before, note, .. } = std::mem::replace(&mut active.phase, Phase::Working) else { return };
        let (work, game) = (active.work.clone(), active.app.clone());
        let note = if gone { note } else { format!("{note}; still running after {} s", CLOSE_WAIT.as_secs()) };
        work.push("close", before, os.available_kb(), waited, note);
        self.game.pause.set(true);
        work.push("pause", None, None, Duration::ZERO, "weather, news and guide downloads".to_owned());
        let front = self.front.clone().unwrap_or_default();
        // While it is unknown who plays, no group is reclaimed: any could be the one playing.
        let groups: Vec<String> = match &self.playing {
            None => Vec::new(),
            Some(playing) => self
                .running
                .iter()
                .filter(|a| **a != game && !front.contains(a) && !playing.contains(*a) && !self.lumatvs(a) && !GAMES.contains(&a.as_str()))
                .filter(|a| self.os.app_procs(a).is_some())
                .cloned()
                .collect(),
        };
        (self.run)(Box::new(move || reclaim(&*os, &work, &groups)));
    }

    /// Ends game mode: the worker stops at its next step, the paused work resumes.
    pub fn game_exit(&mut self, reason: &'static str) {
        let Some(active) = self.game.active.take() else { return };
        let started = Instant::now();
        active.work.abort.store(true, Ordering::Relaxed);
        self.game.pause.set(false);
        self.game.left_front = None;
        let ms = started.elapsed().as_millis() as u64;
        self.game.last_exit = Some((reason, ms));
        crate::log_info!("game mode: off for {} ({reason}, {ms} ms)", active.app);
    }

    pub(super) fn game_next(&self, now: Instant) -> Option<Instant> {
        let active = self.game.active.as_ref()?;
        let mut next = now + POLL;
        if matches!(active.phase, Phase::Closing { .. }) {
            next = now + CLOSE_POLL;
        }
        if let Some(left) = self.game.left_front {
            next = next.min(left + EXIT_AFTER);
        }
        if let Some((_, until)) = &self.game.lease {
            next = next.min(*until);
        }
        Some(next.max(now))
    }

    pub fn game_status(&self) -> Value {
        let g = &self.game;
        let steps: Vec<Value> = g
            .last
            .iter()
            .flat_map(|w| w.steps.lock().iter().map(|s| json!({"step": s.step, "freedMb": s.freed_mb, "ms": s.ms, "note": s.note})).collect::<Vec<_>>())
            .collect();
        json!({
            "switch": (g.switch)(),
            "on": g.active.is_some(),
            "app": g.active.as_ref().map(|a| &a.app),
            "since": g.active.as_ref().map(|a| a.since_ms),
            "steps": steps,
            "lastExit": g.last_exit.map(|(reason, ms)| json!({"reason": reason, "exitMs": ms})),
        })
    }
}

/// Step 4, on its own thread: the other apps' memory reclaimed, memory compacted, the clean page cache dropped once.
fn reclaim(os: &dyn super::os::Os, work: &Work, groups: &[String]) {
    let step = |name: &'static str, note: String, f: &dyn Fn() -> std::io::Result<()>| -> bool {
        if work.abort.load(Ordering::Relaxed) {
            return false;
        }
        let (before, started) = (os.available_kb(), Instant::now());
        match f() {
            Ok(()) => {
                work.push(name, before, os.available_kb(), started.elapsed(), note);
                true
            }
            Err(e) => {
                *work.failed.lock() = Some(format!("{name}: {e}"));
                false
            }
        }
    };
    let reclaimed = || {
        for app in groups {
            if work.abort.load(Ordering::Relaxed) {
                break;
            }
            // A group that went away with its app has nothing left to reclaim.
            if let Err(e) = os.reclaim(app)
                && os.app_procs(app).is_some()
            {
                return Err(std::io::Error::new(e.kind(), format!("{app}: {e}")));
            }
        }
        Ok(())
    };
    let note = if groups.is_empty() { "no other app".to_owned() } else { groups.join(", ") };
    let _ = step("reclaim", note, &reclaimed)
        && step("compact", String::new(), &|| os.vm("compact_memory", "1"))
        && step("dropCaches", String::new(), &|| os.vm("drop_caches", "1"));
}


