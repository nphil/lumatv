//! LumaTV's memory manager: LG's memchute's jobs, done by the engine while the shim `lumatv-mm` serves
//! `com.webos.memorymanager` (`mm.rs` links the two; `docs/research/lg-memchute.md` is the design).
//!
//! In this mode ("ours") the engine also makes each app's memory and freezer groups and puts SAM's processes in them, writes
//! `memory.swappiness`, judges LG's levels and signals them, admits launches by closing cached apps until the launch's memory is
//! free, gates preloads while an `EfsTarget` app is in front, and asks the systrim daemons to trim. LG's own apps in the background
//! become closable, after every third-party app; Home and the keep lists never are. Back in stock mode everything the policy
//! changed is put back and memchute's coexistence rules apply again.

use super::levels::{Level, PAGES_PER_MB};
use super::{Band, Class, Engine};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// memchute's `DefaultRequiredMemory`: what a launch that names none needs, in MB.
const DEFAULT_REQUIRED_MB: u64 = 120;
/// memchute's `LaunchThreshold`: what is kept free above a launch's need, in MB.
const LAUNCH_THRESHOLD_MB: u64 = 120;
/// memchute's `MaxTimeout`: how long a launch's admission keeps closing.
const MAX_WAIT: Duration = Duration::from_millis(7000);
/// How often an admission looks again while it waits for a closed app to go.
const ADMIT_STEP: Duration = Duration::from_millis(100);
const LEVEL_EVERY: Duration = Duration::from_secs(1);
/// The shim answers from the helper's state for 10 s; this keeps it fresh.
const STATE_EVERY: Duration = Duration::from_secs(5);
/// `memory.swappiness` of an app on screen or coming up, and of every other: memchute's values.
const SWAPPINESS_FRONT: u32 = 10;
const SWAPPINESS_BEHIND: u32 = 100;
const PRELOAD_STATUS: &str = "luna://com.webos.service.preloadmanager/setPreloadStatus";

/// The shim's end of the bridge: one JSON line per message.
pub trait Shim: Send + Sync {
    fn send(&self, message: Value);
}

struct Admission {
    app: String,
    /// Usable pages the launch needs.
    need: u64,
    until: Instant,
    closed: Vec<String>,
    /// The app closed last, until its processes are gone.
    waiting: Option<(String, Instant)>,
    next: Instant,
}

pub(super) struct Manager {
    shim: Arc<dyn Shim>,
    level: Level,
    prev: Level,
    usable: Option<u64>,
    leveled: Option<Instant>,
    told: Option<Instant>,
    /// The preload status last sent: `true` for `limit`.
    limit: Option<bool>,
    swappiness: BTreeMap<String, u32>,
    /// Apps seen running in this mode: their groups go when they do.
    grouped: BTreeSet<String>,
    admission: Option<Admission>,
}

impl Manager {
    fn new(shim: Arc<dyn Shim>) -> Self {
        Self {
            shim,
            level: Level::Normal,
            prev: Level::Normal,
            usable: None,
            leveled: None,
            told: None,
            limit: None,
            swappiness: BTreeMap::new(),
            grouped: BTreeSet::new(),
            admission: None,
        }
    }

    fn signal(&self, name: &str, payload: Value) {
        self.shim.send(json!({"kind": "signal", "name": name, "payload": payload}));
    }
}

impl Engine {
    pub fn ours(&self) -> bool {
        self.mm.is_some()
    }

    /// The shim serves the name: memchute's jobs are the engine's from now. Groups memchute froze are adopted when the app is
    /// cached and may be frozen here, and thawed otherwise.
    pub fn take_over(&mut self, shim: Arc<dyn Shim>, now: Instant) {
        if self.mm.is_some() {
            self.give_back("the shim started again");
        }
        crate::log_info!("memory manager: LumaTV's serves com.webos.memorymanager");
        self.mm = Some(Manager::new(shim));
        self.refresh(now);
        let bands = self.bands(now);
        for app in self.running.clone() {
            if self.os.freezer(&app).as_deref() != Some("FROZEN") {
                continue;
            }
            let freezable = matches!(bands.get(&app), Some(Band::Cached(_))) && self.class(&app) == Some(Class::Other);
            if freezable {
                self.held.entry(app.clone()).or_default().frozen = true;
            } else if let Err(e) = self.os.set_freezer(&app, "THAWED") {
                crate::log_warn!("memory manager: thawing {app}: {e}");
            }
        }
        self.reconcile(now);
        self.mm_levels(now);
        self.mm_gate();
        self.mm_tell(now);
    }

    /// LG's memchute serves again: everything the policy changed is put back, and the stock rules apply.
    pub fn give_back(&mut self, why: &str) {
        if self.mm.take().is_none() {
            return;
        }
        crate::log_info!("memory manager: back to LG's memchute ({why})");
        self.restore_all(why);
    }

    /// `setRequiredMemory`, already answered by the shim: the app's groups are made, and unless it is a preload, cached apps are
    /// closed one at a time until the launch's memory is free.
    pub fn launch(&mut self, request: &Value, now: Instant) {
        let Some(mm) = &self.mm else { return };
        let Some(app) = request.get("appId").and_then(Value::as_str).filter(|a| super::lg::valid_id(a)).map(str::to_owned) else {
            return;
        };
        if mm.admission.as_ref().is_some_and(|a| a.app != app) {
            self.admit_end(now);
        }
        if let Err(e) = self.os.make_groups(&app) {
            crate::log_warn!("memory manager: groups of {app}: {e}");
        }
        let grace = self.timing.launch_grace;
        self.seen_mut(&app, now).launching_until = Some(now + grace);
        if request.get("preload").is_some_and(|p| !p.is_null()) {
            return;
        }
        let required = request.get("requiredMemory").and_then(Value::as_u64).filter(|&m| m > 0).unwrap_or(DEFAULT_REQUIRED_MB);
        let need = (required + LAUNCH_THRESHOLD_MB) * PAGES_PER_MB;
        if let Some(mm) = &mut self.mm {
            mm.admission = Some(Admission { app, need, until: now + MAX_WAIT, closed: Vec::new(), waiting: None, next: now });
        }
        self.admit(now);
    }

    /// One step of a launch's admission.
    fn admit(&mut self, now: Instant) {
        let Some(adm) = self.mm.as_mut().and_then(|m| m.admission.as_mut()) else { return };
        if now < adm.next {
            return;
        }
        adm.next = now + ADMIT_STEP;
        if let Some((app, at)) = &adm.waiting {
            let gone = self.os.group_pids(app).is_none_or(|p| p.is_empty());
            if !gone && now.saturating_duration_since(*at) < self.timing.close_wait {
                return;
            }
            adm.waiting = None;
        }
        // SAM's list can still name an app this admission closed until it is asked again.
        let (need, launching, closed) = (adm.need, adm.app.clone(), adm.closed.clone());
        let usable = self.os.usable_pages();
        if usable.is_none_or(|u| u >= need) || now >= adm.until {
            self.admit_end(now);
            return;
        }
        let bands = self.bands(now);
        let Some(victim) = self.closable(&bands).into_iter().find(|a| *a != launching && !closed.contains(a)) else {
            self.admit_end(now);
            return;
        };
        let why = format!("the launch of {launching}");
        if self.close_app(&victim, &why, true, now) {
            if let Some(adm) = self.mm.as_mut().and_then(|m| m.admission.as_mut()) {
                adm.closed.push(victim.clone());
                adm.waiting = Some((victim, now));
            }
        } else {
            self.admit_end(now);
        }
    }

    /// The admission is over: one `clearContainers` and one toast for what it closed.
    fn admit_end(&mut self, _now: Instant) {
        let Some(adm) = self.mm.as_mut().and_then(|m| m.admission.take()) else { return };
        if adm.closed.is_empty() {
            return;
        }
        self.mm_cleared();
        let (lg, names) = (self.lg.clone(), adm.closed);
        crate::log_warn!("memory manager: closed {} for the launch of {}", names.join(", "), adm.app);
        (self.run)(Box::new(move || {
            let titles: Vec<String> = names.into_iter().map(|a| lg.title(&a).unwrap_or(a)).collect();
            lg.toast(&format!("Closed {} to free memory for the app starting", titles.join(", ")));
        }));
    }

    /// Before SAM closes an app: WAM's `getCloseAppId` subscription hears of it first.
    pub(super) fn mm_closing(&self, app: &str) {
        let Some(mm) = &self.mm else { return };
        let app_type = self.sam.iter().find(|r| r.id == app).map_or("", |r| r.app_type.as_str());
        mm.shim.send(json!({"kind": "closing", "id": app, "appType": app_type}));
    }

    /// After a batch of closes: WAM drops its spare containers.
    pub(super) fn mm_cleared(&self) {
        if let Some(mm) = &self.mm {
            mm.signal("clearContainers", json!({"currentLevel": mm.level.name()}));
        }
    }

    /// After SAM's list was asked again: every running app's groups exist and hold SAM's processes; groups of apps gone are removed.
    pub(super) fn mm_groups(&mut self) {
        let Some(mm) = &mut self.mm else { return };
        let mut owners: BTreeMap<i32, usize> = BTreeMap::new();
        for pid in self.sam.iter().flat_map(|r| &r.pids) {
            *owners.entry(*pid).or_default() += 1;
        }
        for r in &self.sam {
            if let Err(e) = self.os.make_groups(&r.id) {
                crate::log_warn!("memory manager: groups of {}: {e}", r.id);
                continue;
            }
            mm.grouped.insert(r.id.clone());
            // A process SAM names for two apps (a shared web process) belongs to neither alone.
            for pid in r.pids.iter().filter(|p| owners.get(p) == Some(&1)) {
                match self.os.attach(&r.id, *pid) {
                    Ok(true) => crate::log_info!("memory manager: {pid} put in {}'s groups", r.id),
                    Ok(false) => {}
                    Err(e) => crate::log_warn!("memory manager: putting {pid} in {}'s groups: {e}", r.id),
                }
            }
        }
        let gone: Vec<String> = mm.grouped.iter().filter(|a| !self.sam.iter().any(|r| &r.id == *a)).cloned().collect();
        for app in gone {
            self.os.remove_groups(&app);
            mm.grouped.remove(&app);
            mm.swappiness.remove(&app);
        }
    }

    /// `memory.swappiness` by state, written only when it changes.
    pub(super) fn mm_swappiness(&mut self, now: Instant) {
        let Some(mm) = &mut self.mm else { return };
        let front = self.front.clone().unwrap_or_default();
        for r in &self.sam {
            let launching = self.seen.get(&r.id).and_then(|s| s.launching_until).is_some_and(|t| now < t);
            let value = if front.contains(&r.id) || launching { SWAPPINESS_FRONT } else { SWAPPINESS_BEHIND };
            if mm.swappiness.get(&r.id) == Some(&value) {
                continue;
            }
            match self.os.set_swappiness(&r.id, value) {
                Ok(()) => {
                    mm.swappiness.insert(r.id.clone(), value);
                }
                Err(e) => crate::log_warn!("memory manager: swappiness of {}: {e}", r.id),
            }
        }
    }

    /// LG's level from `usable_pages`: a change is signalled to WAM, and a worsening to medium or below asks the systrim daemons to
    /// trim.
    fn mm_levels(&mut self, now: Instant) {
        let Some(mm) = &mut self.mm else { return };
        mm.leveled = Some(now);
        let Some(usable) = self.os.usable_pages() else { return };
        mm.usable = Some(usable);
        let next = mm.level.step(usable);
        if next == mm.level {
            return;
        }
        let worse = next > mm.level;
        (mm.prev, mm.level) = (mm.level, next);
        crate::log_info!("memory manager: level {} (usable {} MB)", next.name(), usable / PAGES_PER_MB);
        mm.signal("thresholdChanged", json!({"previous": mm.prev.name(), "current": next.name()}));
        mm.told = None;
        if worse && next >= Level::Medium {
            let os = self.os.clone();
            (self.run)(Box::new(move || {
                let asked = os.systrim();
                crate::log_info!("memory manager: {asked} daemon(s) asked to trim");
            }));
        }
    }

    /// Preloads wait while an app memchute preloads for (`EfsTarget`) is in front; told on a change only.
    pub(super) fn mm_gate(&mut self) {
        let limit = self.front.as_ref().and_then(|f| f.first()).is_some_and(|top| self.memchute.as_ref().is_some_and(|m| m.efs(top)));
        let Some(mm) = &mut self.mm else { return };
        if mm.limit == Some(limit) {
            return;
        }
        mm.limit = Some(limit);
        mm.told = None;
        let status = if limit { "limit" } else { "start" };
        mm.shim.send(json!({"kind": "outbound", "uri": PRELOAD_STATUS, "payload": {"status": status}}));
    }

    /// The state the shim answers from while the helper is busy or away.
    fn mm_tell(&mut self, now: Instant) {
        let Some(mm) = &self.mm else { return };
        if mm.told.is_some_and(|t| now.saturating_duration_since(t) < STATE_EVERY) {
            return;
        }
        let bands = self.bands(now);
        let units: Vec<Value> = self
            .sam
            .iter()
            .map(|r| {
                json!({
                    "appId": r.id,
                    "pid": r.pids.first().copied().unwrap_or(0),
                    "appType": r.app_type,
                    "state": bands.get(&r.id).map_or("kept", |b| b.name()),
                    "frozen": self.held.get(&r.id).is_some_and(|h| h.frozen),
                })
            })
            .collect();
        let swap_used = self.os.sample().map_or(0, |s| s.swap_total_kb.saturating_sub(s.swap_free_kb) / 1024);
        let policy = json!({
            "levels": super::levels::table(),
            "defaultRequiredMemoryMB": DEFAULT_REQUIRED_MB,
            "launchThresholdMB": LAUNCH_THRESHOLD_MB,
            "maxTimeoutMs": MAX_WAIT.as_millis() as u64,
            "lists": self.memchute.as_ref().map_or(Value::Null, |m| m.policy()),
        });
        mm.shim.send(json!({
            "kind": "state",
            "level": mm.level.name(),
            "prevLevel": mm.prev.name(),
            "limit": mm.limit == Some(true),
            "swapUsed": swap_used,
            "units": units,
            "policy": policy,
        }));
        if let Some(mm) = &mut self.mm {
            mm.told = Some(now);
        }
    }

    /// The timers of this mode: the level each second, an admission's next step, the state for the shim.
    pub(super) fn mm_tick(&mut self, now: Instant) {
        let Some(mm) = &self.mm else { return };
        if mm.leveled.is_none_or(|t| now.saturating_duration_since(t) >= LEVEL_EVERY) {
            self.mm_levels(now);
        }
        self.admit(now);
        self.mm_tell(now);
    }

    pub(super) fn mm_admitting(&self) -> bool {
        self.mm.as_ref().is_some_and(|m| m.admission.is_some())
    }

    pub(super) fn mm_next(&self, now: Instant) -> Option<Instant> {
        let mm = self.mm.as_ref()?;
        let mut next = mm.leveled.map_or(now, |t| t + LEVEL_EVERY);
        if let Some(t) = mm.told {
            next = next.min(t + STATE_EVERY);
        }
        if let Some(adm) = &mm.admission {
            next = next.min(adm.next);
        }
        Some(next)
    }

    pub(super) fn mm_status(&self) -> Value {
        let Some(mm) = &self.mm else { return json!({"serving": "lg"}) };
        json!({
            "serving": "lumatv",
            "level": mm.level.name(),
            "usableMB": mm.usable.map(|u| u / PAGES_PER_MB),
            "preload": mm.limit.map(|l| if l { "limit" } else { "start" }),
            "admitting": mm.admission.as_ref().map(|a| &a.app),
        })
    }
}
