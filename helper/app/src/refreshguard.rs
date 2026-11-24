//! LG's panel refresh (off-RS) and the guard that keeps LumaTV from cutting it short.
//!
//! When the TV goes to standby and the panel's counter has passed `offrsLastTime + offrsInterval` (panel hours; the interval is 4 h
//! on the C4), LG runs a compensation of about 10 minutes with the screen dark. A wake in that time aborts it and LG tries again at
//! the next standby, so a TV that is woken every few minutes never finishes one (58 of the 86 standbys of 2026-10-25 to 28 were
//! under 15 minutes, all from test rounds).
//!
//! Two things live here:
//! - the `panel` topic (`GET /v1/state/panel`): what LG says about the refresher (`luna://com.webos.service.oledepl/
//!   getPixelRefresherInfoList`: `offrsCount`, `offrsLastTime`, `offrsInterval`) and the panel's usage (`panelcontroller/
//!   getPanelUsageTime`: `panelUsageTime` in ten-minute ticks, 6 to the hour). Read at start, at every change of the screen and
//!   every ten minutes while the screen is on; never polled during standby. `due` is the usage reaching last run + interval.
//! - [`RefreshGuard`], on unless `theme.oledcare.refreshGuard` is false: a wake that LumaTV starts itself (an assistant opening an
//!   app, switching an input or pressing a transport key on a TV in standby, see [`GuardedTv`]; the phone remote's power key; the
//!   installers, through `GET /v1/panel/refresher`) is refused while a run is in progress. A run is in progress when eplmanager's
//!   state file (`/mnt/lg/cmn_data/pnwash/state`, first field `2`, measured on the C4 2026-11-23: `1` idle at standby, `2` running,
//!   `8` done, `0` on) says so, for at most [`SAFE_STANDBY_MS`] from the first read of `2`. Where the file says neither (`1`, a
//!   missing or unreadable file) the older inference stands: the TV has been in standby for under [`SAFE_STANDBY_MS`], the run was
//!   due when standby began, and the run count has not gone up since. The end of a run is pushed by a `getPixelRefresherInfoList`
//!   subscription (`offrsCount` goes up; see [`follow`]), so nothing polls during standby. Each refusal is logged and counted in
//!   the topic.
//!
//! The owner is never held up. The guard looks only at standby, and a key on the remote wakes the TV by itself in LG's own code;
//! from then on the screen is on and nothing is refused. If LG does not answer, the guard lets the call through (it is blind, and
//! a broken reader must not make the TV unusable); the topic says so in `error`.
//!
//! Topic `panel`: `{read, offrsCount, lastRunHours, intervalHours, usageHours, due, lastStandbyMinutes, standby, guard, error,
//! readAt}`. `standby` is `null` while the screen is on, else `{since, dueAtStart, countAtStart}` (Unix ms, `null` where not known).
//! `guard` is `{enabled, deferrals, last}` with `last` `{at, what, standbyMinutes}`. The same rule is in
//! `compositor/tools/refresh_guard.py` for the test tooling.

use crate::ai::types::{ActionError, AppInfo, ForegroundApp, InputInfo, MediaKey, TvControl};
use crate::control::Stop;
use crate::ctx::Ctx;
use crate::hub::Hub;
use crate::luna::Feed;
use crate::screen;
use crate::timeutil::now_ms;
use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// The hub topic.
pub const TOPIC: &str = "panel";
const REFRESHER_URI: &str = "luna://com.webos.service.oledepl/getPixelRefresherInfoList";
const USAGE_URI: &str = "luna://com.webos.service.panelcontroller/getPanelUsageTime";
/// `getPanelUsageTime` refuses `{}` (errorCode 3, "Failed to validate against schema", measured on the TV 2026-10-28); `subscribe`
/// is its one required field, and false asks for the one answer.
const USAGE_PAYLOAD: &str = r#"{"subscribe":false}"#;
/// A run takes about 10 minutes; this is the standby after which none is taken to be in progress.
pub const SAFE_STANDBY_MS: u64 = 15 * 60_000;
/// `getPanelUsageTime` counts in ten-minute ticks.
const TICKS_PER_HOUR: f64 = 6.0;
/// How often the info is read while the screen is on.
const READ_EVERY: Duration = Duration::from_secs(600);
/// The longest one `luna-send` may take.
const DEADLINE: Duration = Duration::from_secs(4);
/// The most of a reply that is read.
const MAX_REPLY: u64 = 64 * 1024;
/// eplmanager's state file: its first field is `1` idle at standby, `2` running, `8` done (`0` while the TV is on).
pub const PNWASH_STATE: &str = "/mnt/lg/cmn_data/pnwash/state";
const PNWASH_RUNNING: u32 = 2;
const PNWASH_DONE: u32 = 8;
/// A subscription that ended is made again after this.
const RESUBSCRIBE: Duration = Duration::from_secs(30);

/// The first field of the state file; `None` where the file is missing or does not start with a number.
fn read_pnwash(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path)
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// What LG says about the refresher and the panel's hours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    /// Runs that completed (`offrsCount`).
    pub count: u64,
    /// The panel hour the last run was at (`offrsLastTime`).
    pub last_run_hours: f64,
    /// Hours between runs (`offrsInterval`).
    pub interval_hours: f64,
    /// The panel's usage in hours (`panelUsageTime` / 6).
    pub usage_hours: f64,
}

impl Info {
    /// A run starts at the next standby.
    pub fn due(&self) -> bool {
        self.usage_hours >= self.last_run_hours + self.interval_hours
    }
}

/// A number LG gave as a number or as text.
fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite() && *n >= 0.0)
}

/// The first object, anywhere in a reply, that carries an `offrs` key: the reply may hold the values itself or a list of entries.
fn refresher_object(v: &Value) -> Option<&Map<String, Value>> {
    match v {
        Value::Object(m) => {
            if m.keys().any(|k| k.starts_with("offrs")) {
                return Some(m);
            }
            m.values().find_map(refresher_object)
        }
        Value::Array(a) => a.iter().find_map(refresher_object),
        _ => None,
    }
}

/// The first JSON value `luna-send` printed (one object per reply, compact or pretty-printed).
fn first_json(text: &str) -> Option<Value> {
    serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .next()?
        .ok()
}

fn ok_reply(text: &str, what: &str) -> Result<Value, String> {
    let v = first_json(text).ok_or_else(|| format!("{what}: no JSON reply"))?;
    if v.get("returnValue").and_then(Value::as_bool) == Some(false) {
        let why = v
            .get("errorText")
            .and_then(Value::as_str)
            .unwrap_or("returnValue false");
        return Err(format!("{what}: {why}"));
    }
    Ok(v)
}

/// [`Info`] from the two replies.
pub fn parse(refresher: &str, usage: &str) -> Result<Info, String> {
    let r = ok_reply(refresher, "getPixelRefresherInfoList")?;
    let o =
        refresher_object(&r).ok_or("getPixelRefresherInfoList: no offrs values in the reply")?;
    let field = |k: &str| {
        o.get(k)
            .and_then(number)
            .ok_or_else(|| format!("getPixelRefresherInfoList: no {k}"))
    };
    let u = ok_reply(usage, "getPanelUsageTime")?;
    let ticks = u
        .get("panelUsageTime")
        .and_then(number)
        .ok_or("getPanelUsageTime: no panelUsageTime")?;
    Ok(Info {
        count: field("offrsCount")? as u64,
        last_run_hours: field("offrsLastTime")?,
        interval_hours: field("offrsInterval")?,
        usage_hours: ticks / TICKS_PER_HOUR,
    })
}

/// Where [`Info`] comes from: the TV, or a test's.
pub trait Source: Send + Sync {
    fn read(&self) -> Result<Info, String>;
}

/// The TV, through `luna-send` (the helper cannot link the set's luna library).
pub struct LunaSource {
    program: String,
}

impl LunaSource {
    pub fn new(program: &str) -> Self {
        Self {
            program: program.to_owned(),
        }
    }

    pub(crate) fn call(&self, uri: &str, payload: &str) -> Result<String, String> {
        let mut child = Command::new(&self.program)
            .args(["-n", "1", uri, payload])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not run {}: {e}", self.program))?;
        let Some(mut out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("no output pipe".to_owned());
        };
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("panel-read".to_owned())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut text = String::new();
                let _ = out.by_ref().take(MAX_REPLY).read_to_string(&mut text);
                let _ = tx.send(text);
            });
        if let Err(e) = spawned {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("could not start a reader: {e}"));
        }
        let text = rx.recv_timeout(DEADLINE).map_err(|_| {
            let _ = child.kill();
            "luna-send did not answer".to_owned()
        });
        let _ = child.wait();
        text
    }
}

impl Source for LunaSource {
    fn read(&self) -> Result<Info, String> {
        let refresher = self.call(REFRESHER_URI, "{}")?;
        let usage = self.call(USAGE_URI, USAGE_PAYLOAD)?;
        parse(&refresher, &usage)
    }
}

/// A standby the guard is tracking.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Standby {
    /// When the screen went dark (Unix ms).
    pub since_ms: u64,
    /// Whether a run was due when it did; `None` where it was not read then (the helper started in standby, or LG did not answer).
    pub due_at_start: Option<bool>,
    /// The run count then.
    pub count_at_start: Option<u64>,
}

/// What the guard says of a wake now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Clear,
    /// A run may be in progress: `standby_minutes` into the standby, `minutes_left` to the end of the window.
    Hold {
        standby_minutes: u64,
        minutes_left: u64,
    },
}

/// The rule, shared with `compositor/tools/refresh_guard.py`: a run may be in progress when the TV has been in standby for under
/// [`SAFE_STANDBY_MS`], one was due at the start (`info` stands in where that is unknown), and the count has not gone up since.
/// Anything unknown is clear.
pub fn verdict(info: Option<&Info>, standby: Option<&Standby>, now_ms: u64) -> Verdict {
    let Some(s) = standby else {
        return Verdict::Clear;
    };
    let elapsed = now_ms.saturating_sub(s.since_ms);
    if elapsed >= SAFE_STANDBY_MS {
        return Verdict::Clear;
    }
    let Some(due) = s.due_at_start.or_else(|| info.map(Info::due)) else {
        return Verdict::Clear;
    };
    if !due {
        return Verdict::Clear;
    }
    if let (Some(i), Some(c)) = (info, s.count_at_start) {
        if i.count > c {
            return Verdict::Clear;
        }
    }
    Verdict::Hold {
        standby_minutes: elapsed / 60_000,
        minutes_left: (SAFE_STANDBY_MS - elapsed).div_ceil(60_000),
    }
}

/// What said a run is in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// The state file says `2`.
    Pnwash,
    /// A due standby under [`SAFE_STANDBY_MS`] whose count has not risen.
    Inferred,
}

/// A run that is in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    /// When it was first seen (Unix ms): the state file's first `2`, or the start of the standby for an inferred one.
    pub since_ms: u64,
    pub signal: Signal,
    /// Minutes of standby so far, and minutes until the cap lets go.
    pub standby_minutes: u64,
    pub minutes_left: u64,
}

#[derive(Default)]
struct State {
    standby: Option<Standby>,
    info: Option<Info>,
    error: Option<String>,
    read_at: u64,
    last_standby_minutes: Option<u64>,
    deferrals: u64,
    last_deferral: Option<(u64, String, u64)>,
    /// When the state file was first read as `2` in this run.
    run_since: Option<u64>,
    /// The standby (its `since_ms`) whose run is known to be over: the state file went from `2` to `8`, or the count was pushed up.
    finished_standby: Option<u64>,
}

/// The guard and the topic's state.
pub struct RefreshGuard {
    source: Arc<dyn Source>,
    hub: Arc<Hub>,
    theme_file: PathBuf,
    pnwash: PathBuf,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    state: Mutex<State>,
    /// Serialises [`RefreshGuard::sync`]: the watcher and a call can both see the same change of the screen.
    sync_lock: Mutex<()>,
}

impl RefreshGuard {
    pub fn new(ctx: &Ctx, source: Arc<dyn Source>) -> Arc<Self> {
        Self::with_clock(
            ctx.hub.clone(),
            ctx.paths.data_dir.join("theme").join("theme.json"),
            source,
            Arc::new(now_ms),
        )
    }

    pub fn with_clock(
        hub: Arc<Hub>,
        theme_file: PathBuf,
        source: Arc<dyn Source>,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Arc<Self> {
        Self::with_paths(hub, theme_file, PathBuf::from(PNWASH_STATE), source, clock)
    }

    /// [`RefreshGuard::with_clock`] reading another state file (a test's).
    pub fn with_paths(
        hub: Arc<Hub>,
        theme_file: PathBuf,
        pnwash: PathBuf,
        source: Arc<dyn Source>,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            source,
            hub,
            theme_file,
            pnwash,
            clock,
            state: Mutex::new(State::default()),
            sync_lock: Mutex::new(()),
        })
    }

    /// `theme.oledcare.refreshGuard`; on unless the key is the boolean `false`.
    pub fn enabled(&self) -> bool {
        std::fs::read_to_string(&self.theme_file)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get("oledcare")?.get("refreshGuard")?.as_bool())
            .unwrap_or(true)
    }

    /// The screen as the watcher last published it: `(on, since)`; `None` before it has.
    fn screen_now(&self) -> Option<(bool, u64)> {
        let v = self.hub.get(screen::TOPIC)?;
        let on = v.get("on").and_then(Value::as_bool)?;
        let since = v
            .get("since")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| (self.clock)());
        Some((on, since))
    }

    fn read_info(&self) -> Option<Info> {
        let result = self.source.read();
        let mut st = self.state.lock();
        st.read_at = (self.clock)();
        match result {
            Ok(info) => {
                st.info = Some(info);
                st.error = None;
                Some(info)
            }
            Err(e) => {
                st.error = Some(e);
                None
            }
        }
    }

    /// Follows the screen: the start of a standby (with what LG says then) and the end of one. Cheap while nothing changed.
    pub fn sync(&self) {
        let _one = self.sync_lock.lock();
        let Some((on, since)) = self.screen_now() else {
            return;
        };
        let tracked = self.state.lock().standby.is_some();
        match (on, tracked) {
            (false, false) => {
                let now = (self.clock)();
                let info = self.read_info();
                // A start seen late (the helper began in standby) has no reading of the moment it began.
                let seen_late = now.saturating_sub(since) > 60_000;
                self.state.lock().standby = Some(Standby {
                    since_ms: since.min(now),
                    due_at_start: info.filter(|_| !seen_late).map(|i| i.due()),
                    count_at_start: info.filter(|_| !seen_late).map(|i| i.count),
                });
            }
            (true, true) => {
                {
                    let mut st = self.state.lock();
                    if let Some(s) = st.standby.take() {
                        st.last_standby_minutes = Some(since.saturating_sub(s.since_ms) / 60_000);
                    }
                }
                self.read_info();
            }
            _ => {}
        }
        self.publish();
    }

    /// Reads LG's numbers now and publishes them.
    pub fn refresh(&self) {
        self.read_info();
        self.publish();
    }

    /// The topic's value.
    pub fn value(&self) -> Value {
        let enabled = self.enabled();
        let st = self.state.lock();
        let (read, count, last, interval, usage, due) = match &st.info {
            Some(i) => (
                true,
                json!(i.count),
                json!(i.last_run_hours),
                json!(i.interval_hours),
                json!(i.usage_hours),
                json!(i.due()),
            ),
            None => (
                false,
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
            ),
        };
        json!({
            "read": read,
            "offrsCount": count,
            "lastRunHours": last,
            "intervalHours": interval,
            "usageHours": usage,
            "due": due,
            "lastStandbyMinutes": st.last_standby_minutes,
            "standby": st.standby.map(|s| json!({"since": s.since_ms, "dueAtStart": s.due_at_start, "countAtStart": s.count_at_start})),
            "guard": {
                "enabled": enabled,
                "deferrals": st.deferrals,
                "last": st.last_deferral.as_ref().map(|(at, what, m)| json!({"at": at, "what": what, "standbyMinutes": m})),
            },
            "error": st.error,
            "readAt": st.read_at,
        })
    }

    pub fn publish(&self) {
        self.hub.set(TOPIC, self.value());
    }

    /// The run in progress now, if any. The state file is read here, so a run that began a second ago is seen. `info` is a reading of
    /// this moment where the caller has one; the one kept from the last read stands in.
    pub fn run_now(&self, info: Option<&Info>) -> Option<Run> {
        let now = (self.clock)();
        let code = read_pnwash(&self.pnwash);
        let mut st = self.state.lock();
        if code == Some(PNWASH_RUNNING) {
            let since = *st.run_since.get_or_insert(now);
            let elapsed = now.saturating_sub(since);
            // A state that stays `2` past the cap is not believed: a stuck file must not hold the TV forever.
            if elapsed >= SAFE_STANDBY_MS {
                return None;
            }
            let from = st.standby.map_or(since, |s| s.since_ms.min(since));
            return Some(Run {
                since_ms: since,
                signal: Signal::Pnwash,
                standby_minutes: now.saturating_sub(from) / 60_000,
                minutes_left: (SAFE_STANDBY_MS - elapsed).div_ceil(60_000),
            });
        }
        let was_running = st.run_since.take().is_some();
        let standby = st.standby?;
        if code == Some(PNWASH_DONE) && was_running {
            st.finished_standby = Some(standby.since_ms);
        }
        if st.finished_standby == Some(standby.since_ms) {
            return None;
        }
        match verdict(info.or(st.info.as_ref()), Some(&standby), now) {
            Verdict::Hold {
                standby_minutes,
                minutes_left,
            } => Some(Run {
                since_ms: standby.since_ms,
                signal: Signal::Inferred,
                standby_minutes,
                minutes_left,
            }),
            Verdict::Clear => None,
        }
    }

    /// `GET /v1/panel/refresher`: `{running, since, source, minutesLeft, state}`; `state` is the state file's first field.
    pub fn refresher(&self) -> Value {
        let run = self.run_now(None);
        json!({
            "running": run.is_some(),
            "since": run.map(|r| r.since_ms),
            "source": run.map(|r| match r.signal {
                Signal::Pnwash => "pnwash",
                Signal::Inferred => "inferred",
            }),
            "minutesLeft": run.map(|r| r.minutes_left),
            "state": read_pnwash(&self.pnwash),
        })
    }

    /// The count LG pushed (`offrsCount`): one above the count at the start of the standby means the run is over.
    pub fn note_count(&self, count: u64) {
        {
            let mut st = self.state.lock();
            if let Some(i) = st.info.as_mut() {
                i.count = i.count.max(count);
            }
            if let Some(s) = st.standby
                && s.count_at_start.is_some_and(|c| count > c)
            {
                st.finished_standby = Some(s.since_ms);
            }
        }
        self.publish();
    }

    /// Whether `what` (a wake LumaTV starts itself) may go ahead. `Err` carries what to tell the caller.
    pub fn check(&self, what: &str) -> Result<(), ActionError> {
        if !self.enabled() {
            return Ok(());
        }
        self.sync();
        match self.screen_now() {
            Some((false, _)) => {}
            _ => return Ok(()),
        }
        // The reading of this moment: a run that finished since the standby began has raised the count.
        let fresh = self.read_info();
        let now = (self.clock)();
        match self.run_now(fresh.as_ref()) {
            None => {
                if fresh.is_none() {
                    crate::log_warn!(
                        "refresh guard: {what} let through unchecked, LG's panel refresh state could not be read"
                    );
                }
                Ok(())
            }
            Some(run) => {
                {
                    let mut st = self.state.lock();
                    st.deferrals += 1;
                    st.last_deferral = Some((now, what.to_owned(), run.standby_minutes));
                }
                crate::log_info!(
                    "refresh guard: {what} deferred, the TV has been in standby {} min with a panel refresh running ({} min to go)",
                    run.standby_minutes,
                    run.minutes_left
                );
                self.publish();
                Err(ActionError::new(
                    "refresh_in_progress",
                    format!(
                        "the TV is running its panel refresh (in standby {} min); it is safe to wake it in about {} min",
                        run.standby_minutes, run.minutes_left
                    ),
                ))
            }
        }
    }
}

/// An assistant's calls that wake the TV go through the guard; the rest pass through untouched.
pub struct GuardedTv {
    inner: Arc<dyn TvControl>,
    guard: Arc<RefreshGuard>,
}

impl GuardedTv {
    pub fn new(inner: Arc<dyn TvControl>, guard: Arc<RefreshGuard>) -> Self {
        Self { inner, guard }
    }
}

impl TvControl for GuardedTv {
    fn foreground_app(&self) -> Result<ForegroundApp, ActionError> {
        self.inner.foreground_app()
    }
    fn launchable_apps(&self) -> Result<Vec<AppInfo>, ActionError> {
        self.inner.launchable_apps()
    }
    fn inputs(&self) -> Result<Vec<InputInfo>, ActionError> {
        self.inner.inputs()
    }
    fn launch_app(&self, app_id: &str) -> Result<AppInfo, ActionError> {
        self.guard.check("open an app")?;
        self.inner.launch_app(app_id)
    }
    fn switch_input(&self, input_id: &str) -> Result<InputInfo, ActionError> {
        self.guard.check("switch the input")?;
        self.inner.switch_input(input_id)
    }
    fn standby(&self) -> Result<(), ActionError> {
        self.inner.standby()
    }
    fn media_key(&self, key: MediaKey) -> Result<(), ActionError> {
        self.guard.check("press a transport key")?;
        self.inner.media_key(key)
    }
}

/// Messages to the watcher.
pub enum Cmd {
    /// The screen changed.
    Wake,
    Stop,
}

/// Starts the watcher: follows the screen, and reads LG's numbers at start and every ten minutes while the screen is on.
pub fn spawn(
    stop: Stop,
    guard: Arc<RefreshGuard>,
    rx: Receiver<Cmd>,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("refreshguard".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || {
            guard.refresh();
            guard.sync();
            loop {
                match rx.recv_timeout(READ_EVERY) {
                    Ok(Cmd::Wake) => guard.sync(),
                    Ok(Cmd::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // A standby is not disturbed with reads: the numbers are read at its start and at its end.
                        if !stop.is_set() && matches!(guard.screen_now(), Some((true, _))) {
                            guard.refresh();
                        }
                    }
                }
                if stop.is_set() {
                    return;
                }
            }
        })
}

/// The channel the watcher listens on.
pub fn channel() -> (Sender<Cmd>, Receiver<Cmd>) {
    mpsc::channel()
}

/// The guard the helper runs, for the callers that have no way to be handed it (the phone remote, the API's route).
static CURRENT: Mutex<Option<Arc<RefreshGuard>>> = Mutex::new(None);

/// Makes `guard` the one [`check_current`] and [`refresher_current`] answer from.
pub fn install(guard: Arc<RefreshGuard>) {
    *CURRENT.lock() = Some(guard);
}

/// [`RefreshGuard::check`] on the installed guard; allowed where there is none.
pub fn check_current(what: &str) -> Result<(), ActionError> {
    let guard = CURRENT.lock().clone();
    guard.map_or(Ok(()), |g| g.check(what))
}

/// [`RefreshGuard::refresher`] on the installed guard; not running where there is none.
pub fn refresher_current() -> Value {
    let guard = CURRENT.lock().clone();
    guard.map_or_else(
        || json!({"running": false, "since": null, "source": null, "minutesLeft": null, "state": null}),
        |g| g.refresher(),
    )
}

/// The count in one pushed reply of the subscription.
fn pushed_count(text: &str) -> Option<u64> {
    let v = first_json(text)?;
    number(refresher_object(&v)?.get("offrsCount")?).map(|n| n as u64)
}

/// Hands every reply read from `out` to the guard. A reply is one line, or several that parse together once joined.
fn pump(guard: &RefreshGuard, out: impl Read) {
    let mut pending = String::new();
    for line in BufReader::new(out.take(1024 * 1024))
        .lines()
        .map_while(Result::ok)
    {
        pending.push_str(&line);
        pending.push('\n');
        // Anything before the reply's opening brace is noise, not the start of one.
        if !pending.trim_start().starts_with('{') {
            pending.clear();
            continue;
        }
        if first_json(&pending).is_some() {
            if let Some(count) = pushed_count(&pending) {
                guard.note_count(count);
            }
            pending.clear();
        } else if pending.len() > 8192 {
            pending.clear();
        }
    }
}

/// Follows `getPixelRefresherInfoList` with `subscribe:true` (measured 2026-11-23: it pushes the new `offrsCount` when a run ends, also
/// in standby, and nothing at its start). The child is killed when the stop is requested and made again 30 s after it ends.
pub fn follow(
    stop: Stop,
    guard: Arc<RefreshGuard>,
    program: String,
) -> std::io::Result<JoinHandle<()>> {
    let slot: Arc<Mutex<Option<Feed>>> = Arc::new(Mutex::new(None));
    stop.on_stop({
        let slot = slot.clone();
        move || end_child(&slot)
    });
    thread::Builder::new()
        .name("refresh-follow".to_owned())
        .stack_size(256 * 1024)
        .spawn(move || {
            loop {
                if stop.is_set() {
                    return;
                }
                match Feed::start(&program, REFRESHER_URI, r#"{"subscribe":true}"#, false) {
                    Ok(mut feed) => {
                        let out = feed.output();
                        *slot.lock() = Some(feed);
                        // A stop that came while the child was being made has already run its hook against an empty slot.
                        if stop.is_set() {
                            end_child(&slot);
                            return;
                        }
                        if let Some(out) = out {
                            pump(&guard, out);
                        }
                        end_child(&slot);
                    }
                    Err(e) => crate::log_debug!("refresh guard: could not run {program}: {e}"),
                }
                if stop.sleep(RESUBSCRIBE) {
                    return;
                }
            }
        })
}

fn end_child(slot: &Mutex<Option<Feed>>) {
    if let Some(mut feed) = slot.lock().take() {
        feed.end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const REFRESHER: &str = r#"{"returnValue":true,"pixelRefresherInfoList":[{"offrsCount":12,"offrsLastTime":1700,"offrsInterval":4}]}"#;
    const FLAT: &str = "{\n  \"returnValue\": true,\n  \"offrsCount\": \"3\",\n  \"offrsLastTime\": 20,\n  \"offrsInterval\": 4\n}\n";
    const USAGE: &str = r#"{"returnValue":true,"panelUsageTime":10230}"#;

    fn info(count: u64, last: f64, usage: f64) -> Info {
        Info {
            count,
            last_run_hours: last,
            interval_hours: 4.0,
            usage_hours: usage,
        }
    }

    #[test]
    fn the_replies_are_read_nested_flat_and_with_numbers_as_text() {
        let i = parse(REFRESHER, USAGE).unwrap();
        assert_eq!(
            (i.count, i.last_run_hours, i.interval_hours, i.usage_hours),
            (12, 1700.0, 4.0, 1705.0)
        );
        assert!(i.due(), "1705 >= 1700 + 4");
        let f = parse(FLAT, r#"{"returnValue":true,"panelUsageTime":"138"}"#).unwrap();
        assert_eq!((f.count, f.usage_hours), (3, 23.0));
        assert!(!f.due(), "23 < 24");
    }

    #[test]
    fn a_reply_that_is_wrong_says_which_call_and_what() {
        let denied = r#"{"returnValue":false,"errorText":"Denied method call"}"#;
        assert!(
            parse(denied, USAGE)
                .unwrap_err()
                .contains("getPixelRefresherInfoList: Denied method call")
        );
        assert!(
            parse(r#"{"returnValue":true}"#, USAGE)
                .unwrap_err()
                .contains("no offrs values")
        );
        assert!(
            parse(r#"{"offrsCount":1,"offrsLastTime":2}"#, USAGE)
                .unwrap_err()
                .contains("no offrsInterval")
        );
        assert!(
            parse(REFRESHER, "")
                .unwrap_err()
                .contains("getPanelUsageTime: no JSON")
        );
        assert!(
            parse(REFRESHER, r#"{"returnValue":true}"#)
                .unwrap_err()
                .contains("no panelUsageTime")
        );
        assert!(parse("", USAGE).is_err());
    }

    fn standby(since: u64, due: Option<bool>, count: Option<u64>) -> Standby {
        Standby {
            since_ms: since,
            due_at_start: due,
            count_at_start: count,
        }
    }

    const MIN: u64 = 60_000;

    #[test]
    fn a_run_is_in_progress_for_15_minutes_of_a_standby_that_began_due() {
        let i = info(5, 100.0, 104.0);
        let s = standby(1_000 * MIN, Some(true), Some(5));
        assert_eq!(
            verdict(Some(&i), Some(&s), 1_000 * MIN),
            Verdict::Hold {
                standby_minutes: 0,
                minutes_left: 15
            }
        );
        assert_eq!(
            verdict(Some(&i), Some(&s), 1_003 * MIN + 1),
            Verdict::Hold {
                standby_minutes: 3,
                minutes_left: 12
            }
        );
        assert_eq!(
            verdict(Some(&i), Some(&s), 1_014 * MIN + 59_999),
            Verdict::Hold {
                standby_minutes: 14,
                minutes_left: 1
            }
        );
        assert_eq!(verdict(Some(&i), Some(&s), 1_015 * MIN), Verdict::Clear);
    }

    #[test]
    fn nothing_is_held_when_awake_not_due_finished_or_unknown() {
        let due = info(5, 100.0, 104.0);
        let s = standby(0, Some(true), Some(5));
        assert_eq!(verdict(Some(&due), None, MIN), Verdict::Clear, "awake");
        assert_eq!(
            verdict(Some(&due), Some(&standby(0, Some(false), Some(5))), MIN),
            Verdict::Clear,
            "not due when standby began"
        );
        assert_eq!(
            verdict(Some(&info(6, 104.0, 104.0)), Some(&s), 5 * MIN),
            Verdict::Clear,
            "the count went up: the run finished"
        );
        assert_eq!(
            verdict(None, Some(&standby(0, None, None)), MIN),
            Verdict::Clear,
            "nothing known"
        );
        // the start was not read: the reading of now stands in
        assert_eq!(
            verdict(Some(&due), Some(&standby(0, None, None)), MIN),
            Verdict::Hold {
                standby_minutes: 1,
                minutes_left: 14
            }
        );
        assert_eq!(
            verdict(
                Some(&info(5, 100.0, 103.9)),
                Some(&standby(0, None, None)),
                MIN
            ),
            Verdict::Clear
        );
    }

    struct Fake {
        info: Mutex<Result<Info, String>>,
        reads: AtomicU64,
    }

    impl Source for Fake {
        fn read(&self) -> Result<Info, String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.info.lock().clone()
        }
    }

    struct Rig {
        guard: Arc<RefreshGuard>,
        hub: Arc<Hub>,
        src: Arc<Fake>,
        now: Arc<AtomicU64>,
        dir: tempfile::TempDir,
        pnwash: PathBuf,
    }

    const T0: u64 = 2_000_000_000_000;

    fn rig(enabled: Option<bool>) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let theme = dir.path().join("theme.json");
        if let Some(on) = enabled {
            std::fs::write(
                &theme,
                json!({"oledcare": {"refreshGuard": on}}).to_string(),
            )
            .unwrap();
        }
        let hub = Arc::new(Hub::new(T0));
        let src = Arc::new(Fake {
            info: Mutex::new(Ok(info(5, 100.0, 104.0))),
            reads: AtomicU64::new(0),
        });
        let now = Arc::new(AtomicU64::new(T0));
        let clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        };
        let pnwash = dir.path().join("pnwash-state");
        let guard = RefreshGuard::with_paths(hub.clone(), theme, pnwash.clone(), src.clone(), clock);
        Rig {
            guard,
            hub,
            src,
            now,
            dir,
            pnwash,
        }
    }

    impl Rig {
        fn screen(&self, on: bool) {
            let at = self.now.load(Ordering::SeqCst);
            self.hub.set("screen", json!({"on": on, "state": if on { "Active" } else { "Active Standby" }, "since": at}));
            self.guard.sync();
        }
        fn advance(&self, minutes: u64) {
            self.now.fetch_add(minutes * MIN, Ordering::SeqCst);
        }
        /// What eplmanager's state file says.
        fn pnwash(&self, text: &str) {
            std::fs::write(&self.pnwash, text).unwrap();
        }
    }

    #[test]
    fn the_guard_is_on_unless_the_theme_key_is_false() {
        let r = rig(Some(false));
        r.screen(false);
        assert!(r.guard.check("open an app").is_ok(), "switched off");
        assert_eq!(r.guard.value()["guard"]["deferrals"], 0);
        for enabled in [None, Some(true)] {
            let r = rig(enabled);
            r.screen(false);
            assert!(r.guard.check("open an app").is_err(), "{enabled:?}");
        }
        let r = rig(None);
        std::fs::write(
            r.dir.path().join("theme.json"),
            r#"{"oledcare": {"refreshGuard": "yes"}}"#,
        )
        .unwrap();
        assert!(r.guard.enabled(), "a wrong type is the default, on");
    }

    #[test]
    fn a_wake_in_the_first_15_minutes_of_a_due_standby_is_deferred_then_allowed() {
        let r = rig(Some(true));
        r.screen(true);
        r.advance(30);
        r.screen(false);
        r.advance(3);
        let e = r.guard.check("open an app").unwrap_err();
        assert_eq!(e.code, "refresh_in_progress");
        assert!(
            e.message.contains("standby 3 min") && e.message.contains("12 min"),
            "{}",
            e.message
        );
        let v = r.guard.value();
        assert_eq!(v["guard"]["deferrals"], 1);
        assert_eq!(v["guard"]["last"]["what"], "open an app");
        assert_eq!(v["guard"]["last"]["standbyMinutes"], 3);
        assert_eq!(v["standby"]["dueAtStart"], true);
        r.advance(12);
        assert!(
            r.guard.check("open an app").is_ok(),
            "15 minutes of standby have passed"
        );
    }

    #[test]
    fn a_run_that_finished_early_releases_the_guard() {
        let r = rig(Some(true));
        r.screen(false);
        r.advance(11);
        assert!(r.guard.check("press a transport key").is_err());
        *r.src.info.lock() = Ok(info(6, 104.0, 104.0));
        assert!(
            r.guard.check("press a transport key").is_ok(),
            "the count went from 5 to 6"
        );
    }

    #[test]
    fn a_standby_that_began_not_due_is_never_held_and_an_awake_tv_is_never_held() {
        let r = rig(Some(true));
        *r.src.info.lock() = Ok(info(5, 100.0, 103.0));
        r.screen(false);
        assert!(r.guard.check("open an app").is_ok());
        r.screen(true);
        *r.src.info.lock() = Ok(info(5, 100.0, 105.0));
        assert!(r.guard.check("open an app").is_ok(), "the screen is on");
        assert_eq!(r.guard.value()["guard"]["deferrals"], 0);
    }

    #[test]
    fn a_reading_that_fails_lets_the_call_through_and_the_topic_says_why() {
        let r = rig(Some(true));
        *r.src.info.lock() = Err("luna-send did not answer".into());
        r.screen(false);
        assert!(r.guard.check("open an app").is_ok());
        let v = r.guard.value();
        assert_eq!(v["error"], "luna-send did not answer");
        assert_eq!(v["read"], false);
    }

    #[test]
    fn the_topic_follows_the_screen_and_keeps_the_length_of_the_last_standby() {
        let r = rig(None);
        r.screen(true);
        assert_eq!(r.guard.value()["standby"], Value::Null);
        r.advance(1);
        r.screen(false);
        let v = r.guard.value();
        assert_eq!(v["standby"]["countAtStart"], 5);
        assert_eq!(v["lastStandbyMinutes"], Value::Null);
        r.advance(7);
        r.screen(true);
        let v = r.guard.value();
        assert_eq!(
            (v["standby"].clone(), v["lastStandbyMinutes"].clone()),
            (Value::Null, json!(7))
        );
        assert_eq!(
            (
                v["offrsCount"].clone(),
                v["lastRunHours"].clone(),
                v["intervalHours"].clone(),
                v["usageHours"].clone(),
                v["due"].clone()
            ),
            (
                json!(5),
                json!(100.0),
                json!(4.0),
                json!(104.0),
                json!(true)
            )
        );
        r.guard.publish();
        assert_eq!(r.hub.get(TOPIC).unwrap()["lastStandbyMinutes"], 7);
    }

    #[test]
    fn a_standby_the_helper_found_on_start_is_counted_from_when_the_screen_went_dark() {
        let r = rig(Some(true));
        // the screen watcher says it went dark 5 minutes ago, and the helper is only now looking
        r.now.fetch_add(5 * MIN, Ordering::SeqCst);
        r.hub.set(
            "screen",
            json!({"on": false, "state": "Active Standby", "since": T0}),
        );
        r.guard.sync();
        let v = r.guard.value();
        assert_eq!(v["standby"]["since"], T0);
        assert_eq!(
            v["standby"]["dueAtStart"],
            Value::Null,
            "not read at the time"
        );
        let e = r.guard.check("open an app").unwrap_err();
        assert!(e.message.contains("standby 5 min"), "{}", e.message);
    }

    /// A `luna-send` that answers like the TV: the refresher takes `{}`, the usage call refuses anything but `{"subscribe":false}`.
    fn tv_luna(dir: &std::path::Path) -> String {
        let path = dir.join("luna-send");
        let script = r#"#!/bin/sh
case "$3" in
  */getPixelRefresherInfoList)
    echo '{"returnValue":true,"pixelRefresherInfoList":[{"offrsCount":2,"offrsLastTime":20,"offrsInterval":4}]}' ;;
  */getPanelUsageTime)
    if [ "$4" = '{"subscribe":false}' ]; then echo '{"returnValue":true,"panelUsageTime":150}'
    else echo '{"returnValue":false,"errorCode":3,"errorText":"Failed to validate against schema"}'; fi ;;
  *) echo '{"returnValue":false,"errorText":"unknown"}' ;;
esac
"#;
        crate::executable::write_executable(&path, script);
        path.display().to_string()
    }

    #[test]
    fn the_usage_call_sends_the_payload_the_tv_validates_and_an_empty_one_is_refused_as_it_is_there()
     {
        let dir = tempfile::tempdir().unwrap();
        let program = tv_luna(dir.path());
        let source = LunaSource::new(&program);
        let info = source.read().unwrap();
        assert_eq!(
            (
                info.count,
                info.last_run_hours,
                info.interval_hours,
                info.usage_hours
            ),
            (2, 20.0, 4.0, 25.0)
        );
        assert!(info.due(), "25 h >= 20 + 4");
        let empty = source.call(USAGE_URI, "{}").unwrap();
        let refused = parse(r#"{"returnValue":true,"pixelRefresherInfoList":[{"offrsCount":2,"offrsLastTime":20,"offrsInterval":4}]}"#, &empty).unwrap_err();
        assert!(
            refused.contains("Failed to validate against schema"),
            "{refused}"
        );
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl TvControl for Recorder {
        fn foreground_app(&self) -> Result<ForegroundApp, ActionError> {
            self.0.lock().push("foreground".into());
            Ok(ForegroundApp {
                app_id: None,
                title: None,
            })
        }
        fn launchable_apps(&self) -> Result<Vec<AppInfo>, ActionError> {
            Ok(vec![])
        }
        fn inputs(&self) -> Result<Vec<InputInfo>, ActionError> {
            Ok(vec![])
        }
        fn launch_app(&self, app_id: &str) -> Result<AppInfo, ActionError> {
            self.0.lock().push(format!("launch {app_id}"));
            Ok(AppInfo {
                id: app_id.into(),
                title: app_id.into(),
            })
        }
        fn switch_input(&self, input_id: &str) -> Result<InputInfo, ActionError> {
            self.0.lock().push(format!("input {input_id}"));
            Ok(InputInfo {
                id: input_id.into(),
                label: input_id.into(),
                connected: true,
                app_id: None,
            })
        }
        fn standby(&self) -> Result<(), ActionError> {
            self.0.lock().push("standby".into());
            Ok(())
        }
        fn media_key(&self, key: MediaKey) -> Result<(), ActionError> {
            self.0.lock().push(format!("key {}", key.id()));
            Ok(())
        }
    }

    #[test]
    fn the_wrapper_stops_the_three_wakes_and_passes_everything_else() {
        let r = rig(Some(true));
        r.screen(false);
        r.advance(2);
        let inner = Arc::new(Recorder::default());
        let tv = GuardedTv::new(inner.clone(), r.guard.clone());
        assert_eq!(
            tv.launch_app("netflix").unwrap_err().code,
            "refresh_in_progress"
        );
        assert_eq!(
            tv.switch_input("HDMI_1").unwrap_err().code,
            "refresh_in_progress"
        );
        assert_eq!(
            tv.media_key(MediaKey::Play).unwrap_err().code,
            "refresh_in_progress"
        );
        assert!(
            tv.foreground_app().is_ok()
                && tv.launchable_apps().is_ok()
                && tv.inputs().is_ok()
                && tv.standby().is_ok()
        );
        assert_eq!(
            *inner.0.lock(),
            ["foreground", "standby"],
            "the three wakes never reached the TV"
        );
        assert_eq!(r.guard.value()["guard"]["deferrals"], 3);
        r.advance(13);
        tv.launch_app("netflix").unwrap();
        tv.media_key(MediaKey::Pause).unwrap();
        assert_eq!(inner.0.lock()[2..], ["launch netflix", "key pause"]);
    }

    #[test]
    fn the_watcher_reads_at_start_and_on_a_screen_change_and_stops() {
        let r = rig(None);
        r.hub.set(
            "screen",
            json!({"on": true, "state": "Active", "since": T0}),
        );
        let stop = Stop::new();
        let (tx, rx) = channel();
        let handle = spawn(stop.clone(), r.guard.clone(), rx).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while r.hub.get(TOPIC).is_none() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(r.hub.get(TOPIC).unwrap()["offrsCount"], 5);
        r.hub.set(
            "screen",
            json!({"on": false, "state": "Active Standby", "since": T0}),
        );
        tx.send(Cmd::Wake).unwrap();
        while r.hub.get(TOPIC).unwrap()["standby"].is_null() && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(r.hub.get(TOPIC).unwrap()["standby"]["dueAtStart"], true);
        tx.send(Cmd::Stop).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn a_state_file_that_says_running_holds_a_standby_that_was_not_due_until_the_cap() {
        let r = rig(Some(true));
        *r.src.info.lock() = Ok(info(5, 100.0, 103.0));
        r.screen(false);
        assert!(r.guard.check("open an app").is_ok(), "not due, file silent");
        r.advance(1);
        r.pnwash("2 1\n");
        let e = r.guard.check("open an app").unwrap_err();
        assert_eq!(e.code, "refresh_in_progress");
        let v = r.guard.refresher();
        assert_eq!(
            (v["running"].clone(), v["since"].clone(), v["source"].clone(), v["state"].clone()),
            (json!(true), json!(T0 + MIN), json!("pnwash"), json!(2))
        );
        r.advance(14);
        assert_eq!(r.guard.refresher()["minutesLeft"], 1);
        r.advance(1);
        assert_eq!(
            r.guard.refresher()["running"],
            false,
            "a file stuck at 2 is not believed past 15 minutes"
        );
        assert!(r.guard.check("open an app").is_ok());
    }

    #[test]
    fn done_in_the_state_file_releases_the_guard_though_the_count_has_not_been_read_up() {
        let r = rig(Some(true));
        r.screen(false);
        r.pnwash("2 1\n");
        assert!(r.guard.check("open an app").is_err());
        r.advance(7);
        r.pnwash("8 2\n");
        assert!(
            r.guard.check("open an app").is_ok(),
            "the reading still says 5 and a run due, which the file overrules"
        );
        assert_eq!(r.guard.refresher()["running"], false);
    }

    #[test]
    fn idle_or_unreadable_state_falls_back_to_the_inference_with_its_15_minute_cap() {
        for text in ["1 0\n", "", "x"] {
            let r = rig(Some(true));
            r.screen(false);
            r.pnwash(text);
            let v = r.guard.refresher();
            assert_eq!((v["running"].clone(), v["source"].clone()), (json!(true), json!("inferred")), "{text:?}");
            assert!(r.guard.check("open an app").is_err(), "{text:?}");
            r.advance(15);
            assert!(r.guard.check("open an app").is_ok(), "{text:?}");
        }
    }

    #[test]
    fn a_pushed_count_ends_the_run_and_the_topic_follows() {
        let r = rig(Some(true));
        r.screen(false);
        assert_eq!(r.guard.refresher()["running"], true);
        r.guard.note_count(5);
        assert_eq!(r.guard.refresher()["running"], true, "the same count");
        r.guard.note_count(6);
        assert_eq!(r.guard.refresher()["running"], false);
        assert_eq!(r.hub.get(TOPIC).unwrap()["offrsCount"], 6);
    }

    #[test]
    fn the_subscription_is_read_from_compact_and_pretty_replies_and_noise() {
        let r = rig(Some(true));
        r.screen(false);
        let pushes = concat!(
            "not json\n",
            "{\"returnValue\":true,\"subscribed\":true,\"offrsCount\":5,\"offrsLastTime\":100,\"offrsInterval\":4}\n",
            "{\n  \"returnValue\": true,\n  \"subscribed\": true,\n  \"offrsCount\": 6\n}\n",
        );
        pump(&r.guard, std::io::Cursor::new(pushes));
        assert_eq!(r.guard.refresher()["running"], false);
        assert_eq!(pushed_count("{\"returnValue\":true}"), None);
    }
}
