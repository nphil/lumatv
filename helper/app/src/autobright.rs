//! Auto brightness: OLED pixel brightness follows the room's light, from the TV's own light sensor, never above the owner's level
//! (`docs/AGENTS.md`, "Auto brightness").
//!
//! - **The sensor.** pqcontroller's `getInfo {"key": "eyeQLux"}` is the light sensor LG's Energy saving Auto reads, in whole lux
//!   (measured at night: 1 in the dark, 6 to 8 with a lamp on, 2 to 4 with it dimmed). It has no subscription, so it is read every
//!   [`SAMPLE`] while the switch is on and the screen is lit.
//! - **The filter** is Android's `AutomaticBrightnessController`: a short and a long weighted mean of the samples (recent ones weigh
//!   more), and the accepted room light moves only when both cross a band around it (up [`BRIGHTEN_BAND`], down [`DARKEN_BAND`], at
//!   least [`MIN_BAND_LUX`]) and every sample since has stayed past it for the debounce ([`BRIGHTEN_DEBOUNCE`], [`DARKEN_DEBOUNCE`]). The
//!   screen's own light reaches the sensor, so the darkening side is the slow one.
//! - **The decision** is a level as a share of the owner's ceiling for the accepted lux and the picture mode: [`learn::Learner`], the
//!   owner's own curve fitted through his corrections and starting from a default one ([`learn::prior`]: from half the ceiling in the
//!   dark ([`DARK_LUX`] and below) to the ceiling at [`BRIGHT_LUX`], on a log scale).
//! - **The owner's level is the ceiling.** The mode's manual LG setting is the shared limit. A person can explicitly raise their
//!   own limit with the slider, but that does not raise it for another profile. A value chosen in any light is a preference within
//!   the limit, not a new shared limit. A run of changes (20, 25, 30, 25, 20 in 19 s) is one correction, taken once the value has
//!   been left alone for [`SETTLE`]; nothing is written over the value meanwhile.
//! - **The ramp.** One unit a write toward the target, never more: a unit is the smallest step the panel takes, 2% of a level of 50,
//!   so what keeps a step unseen is the time between steps. A drift goes at most [`DARKEN_RATE`] (down) or [`BRIGHTEN_RATE`] (up) of
//!   the level a second, never closer than [`MIN_GAP`], and starts only for a target [`START_DEADBAND`] units away. A sudden shift (a
//!   lamp on or off: [`JUMP_RATIO`] and [`JUMP_LUX`] past the accepted light for [`JUMP_UP_SAMPLES`] or [`JUMP_DOWN_SAMPLES`]
//!   samples in a row) ramps at [`FAST_BRIGHTEN_RATE`] or [`FAST_DARKEN_RATE`], eased at both ends. The switch coming on ramps from the
//!   picture's value; a picture mode coming into force (a video starting) goes to its target in one write, hidden by the mode change.
//!   The design follows `docs/research/auto-brightness-research.md`.
//! - **LG's Energy saving Auto** reads the same sensor and dims on its own; two of them would fight. A mode whose Energy saving is
//!   Auto has it switched off while this drives it, and put back exactly when this lets go. Minimum, Medium and Maximum are fixed caps
//!   and are left as they are. HDR and Dolby Vision modes are never driven. SDR Game Optimizer (`sdrALLM`) is supported without
//!   touching the mode or its latency settings.
//! - **One owner of the value at a time.** Nothing is driven while the compositor's panel dim holds the value (its journal exists),
//!   while the picture's range is unknown, or while the screen is off.
//! - **Journal.** `autobright.json` holds the per-mode LG baseline, the last value written and the Energy saving value replaced.
//!   Switching off, helper stop, repeated sensor failure and recovery with the switch off put the settings back if still owned.
//!   Learning lives in the active profile's directory; the old unattributed global history is left untouched.
//!
//! The switch is `system.autoBrightness` of theme.json (off unless `true`). Topic `autobright`. LumaTV's sliders send the owner's
//! brightness through `POST /v1/autobright/level` while it drives the picture, so the value is written at once.
//! `POST /v1/autobright/reset` forgets the active profile's learned adjustments.

use crate::ctx::Ctx;
use crate::paneldim::{self, Settings};
use crate::refreshguard::LunaSource;
use learn::Learner;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const TOPIC: &str = "autobright";
/// The journal, in the data directory.
pub const STATE: &str = "autobright.json";
const SENSOR_URI: &str = "luna://com.webos.service.pqcontroller/getInfo";
const READ_URI: &str = "luna://com.webos.service.settings/getSystemSettings";
const WRITE_URI: &str = "luna://com.webos.service.settings/setSystemSettings";

/// How often the sensor is read. LG's own reading moves about this often.
pub const SAMPLE: Duration = Duration::from_secs(2);
/// The fast mean's horizon (Android: 2 s; slower here, a TV's room changes less often than a phone's).
pub const SHORT: Duration = Duration::from_secs(6);
/// The slow mean's horizon (Android: 10 s).
pub const LONG: Duration = Duration::from_secs(30);
/// How long the room must stay brighter before it counts (Android: 4 s).
pub const BRIGHTEN_DEBOUNCE: Duration = Duration::from_secs(10);
/// How long the room must stay darker before it counts (Android: 8 s): a dark scene darkens the room the sensor sees.
pub const DARKEN_DEBOUNCE: Duration = Duration::from_secs(30);
/// The band around the accepted lux, as a fraction of it (Android's `config_ambientBrighteningThresholds` 10%, darkening 20%): the
/// sensor reads whole lux and jumps between 1 and 3 in a dark room, so the bands are wider.
pub const BRIGHTEN_BAND: f64 = 0.3;
pub const DARKEN_BAND: f64 = 0.4;
/// The least width of either band, in lux (Android's minimum thresholds).
pub const MIN_BAND_LUX: f64 = 2.0;
/// The fastest drift down and up, as a fraction of the level per second. A step is one unit of the setting, the smallest the panel
/// takes, which is 2% of a level of 50: the rate is what spaces the steps in time so that none is seen as a step, a unit every 5 s
/// down and every 2.5 s up at level 50. Up is twice as fast: a room that has lit up should not wait long, one that has dimmed can.
pub const DARKEN_RATE: f64 = 0.004;
pub const BRIGHTEN_RATE: f64 = 0.008;
/// Never two drift writes closer than this (the compositor's panel dim uses the same).
pub const MIN_GAP: Duration = Duration::from_secs(2);
/// A drift does not start for a target less than this many units away: the accepted light moves only past its bands, so this is only
/// the rounding of the curve, and it never leaves a ramp half done (a ramp that has started goes to its target).
pub const START_DEADBAND: u32 = 2;
/// A sudden shift (a lamp on or off): the newest [`JUMP_UP_SAMPLES`] samples all at least [`JUMP_RATIO`] times and [`JUMP_LUX`] above
/// the accepted light, or the newest [`JUMP_DOWN_SAMPLES`] all as far below it. Samples are [`SAMPLE`] apart, so a lamp on counts after
/// 2 s and a lamp off after 4 s: one reading is a hand or a person passing the sensor, and the screen's own light never makes a jump
/// this size (4 to 5 lux blips in the dark). Darkening wants one more sample than brightening for the same reason the debounce is longer.
pub const JUMP_RATIO: f64 = 2.0;
pub const JUMP_LUX: f64 = 4.0;
pub const JUMP_UP_SAMPLES: usize = 2;
pub const JUMP_DOWN_SAMPLES: usize = 3;
/// The ramp after a sudden shift, at its fastest: 10% of the level a second up, 6% down (35 units in about 8 s up and 13 s down at level
/// 60), eased: its speed rises from [`EASE_FLOOR`] of that at the start to all of it in the middle and falls again, so a fade has no
/// edge where it begins or ends. Never a write closer than [`FAST_GAP`] (each write is a luna call).
pub const FAST_BRIGHTEN_RATE: f64 = 0.10;
pub const FAST_DARKEN_RATE: f64 = 0.06;
pub const EASE_FLOOR: f64 = 0.3;
pub const FAST_GAP: Duration = Duration::from_millis(250);
/// How often the setting is read while the level is at its target: the owner's slider, a mode change.
pub const LOOK: Duration = Duration::from_secs(10);
/// This many failed calls in a row put everything back and stop for [`LATCH`].
pub const FAILS: u32 = 3;
pub const LATCH: Duration = Duration::from_secs(10 * 60);
/// At this lux and below the room is dark: the floor.
pub const DARK_LUX: f64 = 1.0;
/// At this lux and above the room is lit: the owner's level. A lamp-lit room at night reads 6 to 8.
pub const BRIGHT_LUX: f64 = 30.0;
/// How long a hand-set value is left alone before it counts as a correction (a slider's run of steps is one).
pub const SETTLE: Duration = Duration::from_secs(5);
/// A hand change this soon after the scene in front changed is probably a reaction to the scene before.
pub const SWITCH_GRACE: Duration = Duration::from_secs(3);
/// The look interval while that is in doubt.
const WATCH: Duration = Duration::from_secs(1);
/// Keep a manual choice for this long at most, so a later time-of-day preference can take over without a jump at a minute boundary.
const HOLD_FOR: Duration = Duration::from_secs(60 * 60);
const READ_LAG: Duration = Duration::from_secs(5);

/// What a decision is made from.
pub struct Inputs<'a> {
    /// The accepted room light, in lux.
    pub lux: f64,
    /// The picture mode in force, as the journal names it.
    pub mode: &'a str,
    /// Minutes since local midnight, when the clock can say.
    pub minute: Option<u32>,
    /// Unix seconds: what the learned points' age is counted from.
    pub epoch: u64,
    pub scene: &'a Scene,
}

/// How far the room is from dark (0) to lit (1), on a log scale.
pub fn reach(lux: f64) -> f64 {
    let x = (lux - DARK_LUX).max(0.0);
    ((1.0 + x).ln() / (1.0 + BRIGHT_LUX - DARK_LUX).ln()).min(1.0)
}

/// Writes `body` to `path` through a temporary file beside it, so a crash leaves the old file or the new, never half of one.
fn write_atomic(path: &Path, body: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let open = || std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp);
    let mut file = open().or_else(|e| {
        if e.kind() != io::ErrorKind::AlreadyExists {
            return Err(e);
        }
        std::fs::remove_file(&tmp)?;
        open()
    })?;
    file.write_all(body)?;
    drop(file);
    std::fs::rename(tmp, path)
}

/// What [`Ambient::update`] did to the accepted light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Still,
    /// The room drifted: a slow ramp follows.
    Drift,
    /// The room changed at once and stayed so: a faster, eased ramp follows.
    Sudden,
}

/// The room's light: Android's short and long means, bands and debounce, and the sudden shifts above them.
#[derive(Default)]
pub struct Ambient {
    samples: VecDeque<(Instant, f64)>,
    lux: Option<f64>,
}

impl Ambient {
    pub fn push(&mut self, at: Instant, lux: f64) {
        self.samples.push_back((at, lux));
        // one sample older than the horizon stays: it covers the horizon's start
        while self.samples.len() > 2 && self.samples[1].0 + LONG.max(DARKEN_DEBOUNCE) < at {
            self.samples.pop_front();
        }
    }

    pub fn reset(&mut self) {
        self.samples.clear();
        self.lux = None;
    }

    /// The accepted room light.
    pub fn lux(&self) -> Option<f64> {
        self.lux
    }

    pub fn last(&self) -> Option<f64> {
        self.samples.back().map(|s| s.1)
    }

    /// Android's `calculateAmbientLux`: each sample weighs the integral of `x + LONG` over the time it held (x in seconds, negative
    /// into the past), so recent samples weigh more; the newest holds until a little after now.
    fn mean(&self, now: Instant, horizon: Duration) -> Option<f64> {
        let start = now.checked_sub(horizon);
        let first = self.samples.iter().rposition(|(t, _)| start.is_none_or(|s| *t <= s)).unwrap_or(0);
        let rel = |t: Instant| -> f64 {
            if t >= now { t.duration_since(now).as_secs_f64() } else { -now.duration_since(t).as_secs_f64() }
        };
        let intercept = LONG.as_secs_f64();
        let integral = |x: f64| x * (x * 0.5 + intercept);
        let (mut sum, mut total) = (0.0, 0.0);
        let n = self.samples.len();
        for i in first..n {
            let from = rel(self.samples[i].0).max(-horizon.as_secs_f64());
            let to = if i + 1 < n { rel(self.samples[i + 1].0) } else { 0.1 };
            let w = integral(to) - integral(from);
            if w > 0.0 {
                sum += self.samples[i].1 * w;
                total += w;
            }
        }
        if total > 0.0 { Some(sum / total) } else { self.last() }
    }

    fn bands(lux: f64) -> (f64, f64) {
        (lux + (lux * BRIGHTEN_BAND).max(MIN_BAND_LUX), lux - (lux * DARKEN_BAND).max(MIN_BAND_LUX))
    }

    /// Since when every sample, to the newest, has met `past`; `None` when the newest does not.
    fn past_since(&self, past: impl Fn(f64) -> bool) -> Option<Instant> {
        let mut since = None;
        for (t, l) in self.samples.iter().rev() {
            if !past(*l) {
                break;
            }
            since = Some(*t);
        }
        since
    }

    /// The mean of the newest `n` samples when all of them satisfy `past`.
    fn tail_mean(&self, n: usize, past: impl Fn(f64) -> bool) -> Option<f64> {
        let len = self.samples.len();
        if len < n {
            return None;
        }
        let tail: Vec<f64> = self.samples.iter().skip(len - n).map(|s| s.1).collect();
        tail.iter().all(|l| past(*l)).then(|| tail.iter().sum::<f64>() / n as f64)
    }

    /// Moves the accepted light when the room has changed for long enough, or at once when it changed by a lot and stayed so.
    pub fn update(&mut self, now: Instant) -> Move {
        let Some(fast) = self.mean(now, SHORT) else { return Move::Still };
        let Some(lux) = self.lux else {
            self.lux = Some(fast);
            return Move::Drift;
        };
        let (up_jump, down_jump) = ((lux * JUMP_RATIO).max(lux + JUMP_LUX), (lux / JUMP_RATIO).min(lux - JUMP_LUX));
        if let Some(l) = self.tail_mean(JUMP_UP_SAMPLES, |l| l >= up_jump).or_else(|| self.tail_mean(JUMP_DOWN_SAMPLES, |l| l <= down_jump)) {
            self.lux = Some(l);
            return Move::Sudden;
        }
        let slow = self.mean(now, LONG).unwrap_or(fast);
        let (up, down) = Ambient::bands(lux);
        let brighter = fast >= up && slow >= up && self.past_since(|l| l > up).is_some_and(|t| t + BRIGHTEN_DEBOUNCE <= now);
        let darker = fast <= down && slow <= down && self.past_since(|l| l < down).is_some_and(|t| t + DARKEN_DEBOUNCE <= now);
        if brighter || darker {
            self.lux = Some(fast);
            return Move::Drift;
        }
        Move::Still
    }
}

/// One picture mode this drives.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub mode: String,
    pub input: String,
    /// The manual LG setting to restore, shared by profiles.
    pub own: u32,
    /// The last requested value.
    pub written: u32,
    /// The previous value, until the settings service confirms the new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous: Option<u32>,
    /// LG's Energy saving value this replaced with `off`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy: Option<String>,
}

impl Entry {
    fn owns(&self, value: u32) -> bool {
        self.written == value || self.previous == Some(value)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    #[serde(default)]
    pub modes: BTreeMap<String, Entry>,
    #[serde(default)]
    baselines: BTreeMap<String, Baseline>,
}

/// Keeps a physical limit's revision even after releasing the mode, so a later return to the same number cannot revive an old raise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Baseline {
    value: u32,
    revision: u64,
}

impl Journal {
    fn observe_baseline(&mut self, key: &str, value: u32) -> (u64, bool) {
        if let Some(b) = self.baselines.get_mut(key) {
            if b.value == value {
                return (b.revision, false);
            }
            b.value = value;
            b.revision = b.revision.saturating_add(1);
            return (b.revision, true);
        }
        let revision = crate::timeutil::now_ms();
        self.baselines.insert(key.to_owned(), Baseline { value, revision });
        (revision, true)
    }

    fn revision(&self, key: &str) -> u64 {
        self.baselines.get(key).map_or(0, |b| b.revision)
    }
}

pub fn load(path: &Path) -> Journal {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// The current LG setting. Reading `pictureMode` with `backlight` also asks LG for the dynamic range.
#[derive(Clone, Debug, PartialEq)]
pub struct Here {
    pub mode: String,
    pub input: String,
    pub range: String,
    pub value: u32,
    pub energy: Option<String>,
}

impl Here {
    pub fn parse(reply: &Value) -> Option<Here> {
        let dimension = reply.get("dimension")?;
        let s = |k: &str| dimension.get(k).and_then(Value::as_str);
        let mode = s("pictureMode").or_else(|| reply.pointer("/settings/pictureMode").and_then(Value::as_str))?;
        if reply.pointer("/settings/pictureMode").and_then(Value::as_str).is_some_and(|actual| actual != mode) {
            return None;
        }
        Some(Here {
            mode: mode.to_owned(),
            input: s("input")?.to_owned(),
            range: s("dynamicRange").unwrap_or_default().to_owned(),
            value: paneldim::backlight(reply)?,
            energy: reply.pointer("/settings/energySaving").and_then(Value::as_str).map(str::to_owned),
        })
    }

    fn key(&self) -> String {
        format!("{}/{}", self.input, self.mode)
    }

    fn same_picture(&self, other: &Here) -> bool {
        self.mode == other.mode && self.input == other.input && self.range.eq_ignore_ascii_case(&other.range)
    }

    fn hold(&self) -> Option<&'static str> {
        if ["hdr", "dolby", "hlg"].iter().any(|p| self.mode.get(..p.len()).is_some_and(|s| s.eq_ignore_ascii_case(p))) {
            Some("hdr")
        } else if self.range.is_empty() || self.mode.is_empty() || self.input.is_empty() {
            Some("range")
        } else if !self.range.eq_ignore_ascii_case("sdr") && !self.range.eq_ignore_ascii_case("sdrALLM") {
            Some("hdr")
        } else {
            None
        }
    }

    fn game(&self) -> bool {
        self.mode.eq_ignore_ascii_case("game") || self.range.eq_ignore_ascii_case("sdrALLM")
    }

    fn scene<'a>(&self, hint: &'a Scene) -> Cow<'a, Scene> {
        if !hint.game && self.game() {
            Cow::Owned(Scene { game: true, ..hint.clone() })
        } else {
            Cow::Borrowed(hint)
        }
    }
}

/// What was in front: the app, full-screen playback and whether it was a game.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scene {
    pub app: String,
    pub playing: bool,
    pub game: bool,
}

/// What the rest of the helper says about now.
#[derive(Clone, Debug, Default)]
pub struct Env {
    /// The owner's switch.
    pub on: bool,
    pub screen: bool,
    /// The live profile id and the time it was activated, or unknown during startup/recovery.
    pub profile: Option<(String, u64)>,
    /// The compositor's panel dim holds the value.
    pub panel_dim: bool,
    pub minute: Option<u32>,
    /// Unix seconds, from the helper's clock.
    pub epoch: u64,
    /// The picture topic wakes the next look quickly; the settings read decides whether to write.
    pub context: Value,
    /// The app in front; the direct settings read can also identify Game Optimizer.
    pub scene: Scene,
}

fn valid_profile(id: &str) -> bool {
    !id.is_empty() && id.len() <= crate::profiles::NAME_MAX && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn profile_file(journal: &Path, id: &str) -> Option<PathBuf> {
    if !valid_profile(id) {
        return None;
    }
    let root = journal.with_file_name("profiles");
    let dir = root.join(id);
    if ![&root, &dir].into_iter().all(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())) {
        return None;
    }
    let file = dir.join(learn::FILE);
    match std::fs::symlink_metadata(&file) {
        Ok(m) if m.is_file() => Some(file),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Some(file),
        _ => None,
    }
}

pub struct Engine {
    tv: Box<dyn Settings + Send>,
    path: Option<PathBuf>,
    journal: Journal,
    profile: Option<(String, u64)>,
    learner: Learner,
    /// A run of hand changes not yet settled.
    pending: Option<Pending>,
    ambient: Ambient,
    here: Option<Here>,
    /// The mode driven at the last turn.
    key: Option<String>,
    /// The recent writes in the active mode: a read that lags a write is not a correction.
    recent: VecDeque<(Instant, u32)>,
    target: Option<u32>,
    next_sample: Instant,
    next_turn: Instant,
    /// Failed calls in a row to the settings service and to the light sensor, each reset by its own success: a ramp's settings calls
    /// succeeding must not hide a dead sensor.
    fails: u32,
    sensor_fails: u32,
    latch: Option<Instant>,
    hold: Option<&'static str>,
    error: Option<String>,
    on: bool,
    context: Value,
    /// What was in front at the last pass, and since when it last changed.
    scene: Option<Scene>,
    scene_since: Option<Instant>,
    held: Option<Held>,
    /// Set once a pass has run since this last stood idle: the first pass after the switch, the screen or the sensor comes back is
    /// not held back by the sample and look timers.
    running: bool,
    /// The first pass since `running` began: the mode in force is taken over by ramping from where it is, not by a jump (a mode
    /// coming into force later is hidden by its own change and does jump).
    fresh: bool,
    /// A sudden shift of the room was accepted and is not ramped yet.
    sudden: bool,
    /// The ramp in progress, from the value it began at.
    ramp: Option<Ramp>,
}

/// A ramp toward the target: where it began, and whether a sudden shift started it (faster, eased).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ramp {
    from: u32,
    fast: bool,
}

/// A run of hand changes in one mode, counted as one correction once it has been left alone for [`SETTLE`].
#[derive(Clone, Debug, PartialEq)]
struct Pending {
    key: String,
    /// The value the run has reached.
    value: u32,
    /// The conditions at the latest change, not at the end of the settle window.
    lux: f64,
    minute: Option<u32>,
    epoch: u64,
    /// The latest change.
    at: Instant,
    /// What was in front at the latest change.
    scene: Scene,
    /// Begun right after a scene change and not added to since: not counted.
    suspect: bool,
}

/// The value the owner chose at the light and in the scene it was learned in: the target while they hold, whatever the curve says
/// (it may meet a new app's first correction only partway).
struct Held {
    key: String,
    scene: Scene,
    lux: f64,
    value: u32,
    until: Instant,
}

impl Engine {
    pub fn new(tv: Box<dyn Settings + Send>, journal: Journal, path: Option<PathBuf>, now: Instant) -> Engine {
        Engine {
            tv,
            path,
            journal,
            profile: None,
            learner: Learner::default(),
            pending: None,
            ambient: Ambient::default(),
            here: None,
            key: None,
            recent: VecDeque::new(),
            target: None,
            next_sample: now,
            next_turn: now,
            fails: 0,
            sensor_fails: 0,
            latch: None,
            hold: None,
            error: None,
            on: false,
            scene: None,
            scene_since: None,
            held: None,
            context: Value::Null,
            running: false,
            fresh: false,
            sudden: false,
            ramp: None,
        }
    }

    /// When the next tick has something to do.
    pub fn next(&self) -> Instant {
        self.next_sample.min(self.next_turn)
    }

    fn select(&mut self, now: Instant, profile: Option<&(String, u64)>) {
        let profile = profile.filter(|(id, _)| valid_profile(id));
        if self.profile.as_ref() == profile {
            return;
        }
        self.learner = profile.zip(self.path.as_deref()).and_then(|((id, _), path)| profile_file(path, id))
            .map_or_else(Learner::default, |path| Learner::load(&path));
        self.profile = profile.cloned();
        self.pending = None;
        self.held = None;
        self.ramp = None;
        self.target = None;
        self.scene = None;
        self.scene_since = None;
        self.context = Value::Null;
        self.fresh = true;
        self.sudden = false;
        self.next_turn = now;
    }

    pub fn tick(&mut self, now: Instant, env: &Env) {
        self.select(now, env.profile.as_ref());
        let was = self.on;
        self.on = env.on;
        if self.latch.is_some_and(|t| now >= t) {
            self.latch = None;
            self.fails = 0;
            self.sensor_fails = 0;
        }
        if !env.on || self.latch.is_some() {
            self.hold = Some(if env.on { "error" } else { "off" });
            self.idle(now);
            if !env.panel_dim && !self.journal.modes.is_empty() && ((was && !env.on) || now >= self.next_turn) {
                self.release();
                self.next_turn = now + LOOK;
            } else if now >= self.next_turn {
                self.next_turn = now + LOOK;
            }
            return;
        }
        if !env.screen || self.profile.is_none() {
            self.hold = Some(if env.screen { "profile" } else { "screen" });
            self.idle(now);
            self.next_turn = now + LOOK;
            return;
        }
        if !self.running {
            self.running = true;
            self.fresh = true;
            self.next_sample = now;
            self.next_turn = now;
        }
        if now >= self.next_sample {
            self.next_sample = now + SAMPLE;
            match self.read_lux() {
                Ok(lux) => {
                    self.sensor_fails = 0;
                    self.ambient.push(now, lux);
                    if self.ambient.update(now) == Move::Sudden {
                        self.sudden = true;
                        self.next_turn = now;
                    }
                }
                Err(e) => {
                    return self.failed(now, format!("light sensor: {e}"), true);
                }
            }
        }
        let Some(lux) = self.ambient.lux() else {
            self.hold = Some("sensor");
            return;
        };
        if self.scene.as_ref().is_none_or(|s| {
            s.app != env.scene.app || s.playing != env.scene.playing || s.game != (env.scene.game || self.here.as_ref().is_some_and(Here::game))
        }) {
            self.next_turn = now;
        }
        if env.context != self.context {
            self.context.clone_from(&env.context);
            self.next_turn = now;
        }
        if env.panel_dim {
            self.pause("panel dim");
            self.next_turn = now + WATCH;
            return;
        }
        if self.hold == Some("panel dim") {
            self.next_turn = now;
        }
        if now < self.next_turn {
            return;
        }
        self.next_turn = now + LOOK;
        if let Err(e) = self.turn(now, lux, env) {
            self.pause("settings");
            self.failed(now, e, false);
        } else {
            self.fails = 0;
            self.error = None;
        }
    }

    fn idle(&mut self, now: Instant) {
        self.ambient.reset();
        self.here = None;
        self.scene = None;
        self.scene_since = None;
        self.sudden = false;
        self.clear_drive();
        self.running = false;
        self.next_sample = now + SAMPLE;
    }

    fn clear_drive(&mut self) {
        self.key = None;
        self.target = None;
        self.ramp = None;
        self.pending = None;
        self.held = None;
        self.recent.clear();
    }

    fn pause(&mut self, reason: &'static str) {
        self.hold = Some(reason);
        self.clear_drive();
    }

    fn failed(&mut self, now: Instant, e: String, sensor: bool) {
        *(if sensor { &mut self.sensor_fails } else { &mut self.fails }) += 1;
        if self.fails >= FAILS || self.sensor_fails >= FAILS {
            crate::log_warn!("auto brightness: {e}; everything put back for {} min", LATCH.as_secs() / 60);
            self.error = Some(e);
            self.latch = Some(now + LATCH);
            self.release();
            self.idle(now);
            self.next_turn = now + LOOK;
        }
    }

    fn read_lux(&self) -> Result<f64, String> {
        let r = self.tv.call(SENSOR_URI, &json!({"key": "eyeQLux"}))?;
        r.get("eyeQLux").and_then(Value::as_f64).filter(|l| l.is_finite() && *l >= 0.0).ok_or_else(|| format!("no reading: {r}"))
    }

    fn read_here(&self) -> Result<Here, String> {
        let reply = self.tv.call(READ_URI, &json!({"category": "picture", "keys": ["pictureMode", "backlight", "energySaving"]}))?;
        let mut here = Here::parse(&reply).ok_or_else(|| format!("no picture setting: {reply}"))?;
        if here.range.is_empty() {
            // Some replies omit the range with mixed keys. Trust only a fresh pictureMode reply for the same mode and input.
            let picture = self.tv.call(READ_URI, &json!({"category": "picture", "keys": ["pictureMode"]}))?;
            if let Some(p) = crate::picture::Context::parse(&picture).filter(|p| p.mode == here.mode && p.input == here.input) {
                here.range = p.range;
            }
        }
        Ok(here)
    }

    fn turn(&mut self, now: Instant, lux: f64, env: &Env) -> Result<(), String> {
        let here = self.read_here()?;
        let same_picture = self.here.as_ref().is_some_and(|h| h.same_picture(&here));
        let changed_value = same_picture && self.here.as_ref().is_some_and(|h| h.value != here.value);
        self.here = Some(here.clone());
        let scene = here.scene(&env.scene);
        self.see(now, &scene);
        let watching = self.watching(now);
        self.next_turn = now + if watching { WATCH } else { LOOK };
        if let Some(reason) = here.hold() {
            self.pause(reason);
            if reason == "hdr" {
                self.fresh = false;
            }
            self.next_turn = now + MIN_GAP;
            return Ok(());
        }
        self.hold = None;
        if !same_picture {
            self.clear_drive();
        }
        let key = here.key();
        let inputs = Inputs { lux, mode: &key, scene: &scene, minute: env.minute, epoch: env.epoch };
        let dimension = json!({"pictureMode": here.mode, "input": here.input});
        let fresh = std::mem::take(&mut self.fresh);
        let entered = self.key.as_deref() != Some(key.as_str());
        let known = self.journal.modes.get(&key).cloned();
        let new = known.is_none();
        let mut entry = known.unwrap_or_else(|| Entry {
            mode: here.mode.clone(), input: here.input.clone(), own: here.value, written: here.value, previous: None, energy: None,
        });
        let ours = entry.owns(here.value) || self.recent.iter().any(|&(at, value)| value == here.value && now.saturating_duration_since(at) <= READ_LAG);
        let disable_auto = here.energy.as_deref() == Some("auto");
        if here.value == entry.written {
            entry.previous = None;
        }
        if entered && !ours && !disable_auto && here.value < entry.own {
            // A lower manual LG setting while away becomes the physical cap, not a correction for the profile entering now.
            entry.own = here.value;
            entry.written = here.value;
            entry.previous = None;
        }
        let (revision, baseline_changed) = self.journal.observe_baseline(&key, entry.own);
        self.learner.rebase_ceiling(&key, entry.own, revision);
        if here.value == entry.written && let Some(saved) = self.journal.modes.get_mut(&key) {
            saved.previous = None;
        }
        let remember_auto = disable_auto && entry.energy.is_none();
        if remember_auto {
            entry.energy = Some("auto".into());
        }
        if new || baseline_changed || remember_auto {
            self.journal.modes.insert(key.clone(), entry.clone());
            self.save();
        }
        if disable_auto {
            self.write(&dimension, json!({"energySaving": "off"}))?;
        }
        if new {
            crate::log_info!("auto brightness: {key} taken over at {} with lux {lux:.1}", entry.own);
        }
        if !fresh && !entered && !watching && changed_value && !ours && !disable_auto {
            // Only infer an LG menu adjustment in a continuously owned picture, never on entry or from a late read of our own write.
            entry.written = here.value;
            entry.previous = None;
            self.journal.modes.insert(key.clone(), entry);
            self.save();
            self.hand(now, &inputs, here.value);
            return Ok(());
        }
        if entered || !ours {
            self.remember(now, here.value);
        }
        if let Some(p) = &self.pending {
            if now < p.at + SETTLE {
                self.next_turn = p.at + SETTLE;
                return Ok(());
            }
            self.settle(now, &inputs, here.value);
        }
        if self.held.as_ref().is_some_and(|h| h.key != key || h.scene != *scene || h.lux != lux || now >= h.until) {
            self.held = None;
        }
        let ceiling = self.learner.ceiling(&key, entry.own, revision);
        let wanted = self.held.as_ref().map_or_else(|| self.learner.target(&inputs, f64::from(ceiling)), |h| f64::from(h.value));
        let target = (wanted.round() as u32).min(ceiling);
        self.target = Some(target);
        if entered {
            self.key = Some(key.clone());
        }
        if std::mem::take(&mut self.sudden) {
            self.ramp = Some(Ramp { from: here.value, fast: true });
        }
        let value = if entered && !fresh {
            self.ramp = None;
            target
        } else {
            let apart = target.abs_diff(here.value);
            if apart == 0 {
                self.ramp = None;
                return Ok(());
            }
            if self.ramp.is_none() {
                if apart < START_DEADBAND && here.value <= ceiling {
                    return Ok(());
                }
                self.ramp = Some(Ramp { from: here.value, fast: false });
            }
            (if target > here.value { here.value + 1 } else { here.value - 1 }).min(ceiling)
        };
        if value == here.value {
            return Ok(());
        }
        self.write_backlight(now, key, entry, &dimension, value)?;
        if let Some(current) = &mut self.here {
            current.value = value;
        }
        match self.ramp {
            Some(ramp) if value != target => {
                let total = f64::from(target.abs_diff(ramp.from).max(1));
                let eased = ramp.fast.then(|| f64::from(value.abs_diff(ramp.from)) / total);
                self.next_turn = now + gap(value, value > here.value, eased);
            }
            _ => self.ramp = None,
        }
        Ok(())
    }

    fn remember(&mut self, now: Instant, value: u32) {
        if self.recent.len() == 4 {
            self.recent.pop_front();
        }
        self.recent.push_back((now, value));
    }

    fn watching(&self, now: Instant) -> bool {
        self.scene_since.is_some_and(|t| now.saturating_duration_since(t) < SWITCH_GRACE)
    }

    /// Never finish a pending correction or keep its temporary hold in a different scene.
    fn see(&mut self, now: Instant, scene: &Scene) {
        if self.scene.as_ref() == Some(scene) {
            return;
        }
        if self.scene.is_some() {
            self.scene_since = Some(now);
            self.pending = None;
            self.held = None;
        }
        self.scene = Some(scene.clone());
    }

    /// The value was chosen by hand and reached the TV; coalesce a slider's run of changes into its final position.
    fn hand(&mut self, now: Instant, inputs: &Inputs, value: u32) {
        let suspect = self.watching(now) && self.pending.as_ref().is_none_or(|p| p.suspect);
        self.pending = Some(Pending {
            key: inputs.mode.to_owned(), value, at: now, scene: inputs.scene.clone(), lux: inputs.lux, minute: inputs.minute, epoch: inputs.epoch, suspect,
        });
        self.held = None;
        self.remember(now, value);
        self.target = Some(value);
        self.ramp = None;
        self.next_turn = now + SETTLE;
    }

    fn settle(&mut self, now: Instant, inputs: &Inputs, observed: u32) {
        let Some(p) = self.pending.take_if(|p| now >= p.at + SETTLE) else { return };
        if p.suspect || p.key != inputs.mode || p.scene != *inputs.scene || p.value != observed {
            return;
        }
        let Some(entry) = self.journal.modes.get(&p.key) else { return };
        let revision = self.journal.revision(&p.key);
        self.learner.raise_ceiling(&p.key, entry.own, revision, p.value);
        let own = self.learner.ceiling(&p.key, entry.own, revision);
        if own > 0 {
            self.learner.add(&p.key, &p.scene, p.lux, f64::from(p.value) / f64::from(own), p.minute, p.epoch);
            self.held = Some(Held { key: p.key, scene: p.scene, lux: p.lux, value: p.value, until: now + HOLD_FOR });
        }
    }

    /// Forgets the active profile's learned points, not its explicit limit.
    pub fn reset_learning(&mut self, now: Instant, profile: Option<&(String, u64)>) -> Value {
        self.select(now, profile);
        if self.profile.is_some() {
            self.learner.reset();
        }
        self.pending = None;
        self.held = None;
        self.ramp = None;
        self.target = None;
        self.next_turn = now;
        self.status()
    }

    /// The Quick Settings and Picture sliders use this route while auto brightness drives the picture.
    pub fn by_hand(&mut self, now: Instant, value: u32, env: &Env) -> Result<Value, String> {
        self.select(now, env.profile.as_ref());
        if !env.on || !env.screen || env.panel_dim || self.profile.is_none() || self.hold.is_some() {
            return Err("auto brightness is not controlling this picture".into());
        }
        let lux = self.ambient.lux().ok_or("the room light is not known yet")?;
        let value = value.min(100);
        let mut here = self.read_here()?;
        let key = here.key();
        if here.hold().is_some() || self.key.as_deref() != Some(key.as_str()) || self.here.as_ref().is_none_or(|h| !h.same_picture(&here)) {
            return Err("the picture changed; try the slider again".into());
        }
        let mut entry = self.journal.modes.get(&key).cloned().ok_or("auto brightness has not taken over this mode yet")?;
        if here.value == entry.written {
            entry.previous = None;
        }
        let scene = here.scene(&env.scene);
        self.see(now, &scene);
        let inputs = Inputs { lux, mode: &key, scene: &scene, minute: env.minute, epoch: env.epoch };
        if value != here.value {
            let dimension = json!({"pictureMode": here.mode, "input": here.input});
            self.write_backlight(now, key.clone(), entry, &dimension, value)?;
            self.settle(now, &inputs, here.value);
            self.remember(now, here.value);
            self.hand(now, &inputs, value);
            here.value = value;
        }
        self.here = Some(here);
        Ok(self.status())
    }

    fn write(&self, dimension: &Value, settings: Value) -> Result<(), String> {
        let r = self.tv.call(WRITE_URI, &json!({"category": "picture", "settings": settings, "dimension": dimension}))?;
        if r.get("returnValue").and_then(Value::as_bool) == Some(true) { Ok(()) } else { Err(format!("write refused: {r}")) }
    }

    fn write_backlight(&mut self, now: Instant, key: String, mut entry: Entry, dimension: &Value, value: u32) -> Result<(), String> {
        // Either reading after a timeout or crash still identifies a setting to put back; only an acknowledged slider write learns.
        if entry.written != value {
            entry.previous = entry.previous.or(Some(entry.written));
            entry.written = value;
            self.journal.modes.insert(key, entry);
            self.save();
        }
        self.remember(now, value);
        self.write(dimension, json!({"backlight": value.to_string()}))
    }

    /// Puts every mode back: the owner's level where the value is still the one written, Energy saving as it was. A mode the service
    /// did not answer for stays in the journal for the next try. True when nothing is left.
    pub fn release(&mut self) -> bool {
        if self.journal.modes.is_empty() {
            return true;
        }
        if self.path.as_deref().is_some_and(|p| p.with_file_name(paneldim::JOURNAL).exists()) {
            return false;
        }
        let keys: Vec<String> = self.journal.modes.keys().cloned().collect();
        for key in keys {
            let e = self.journal.modes[&key].clone();
            let dimension = json!({"pictureMode": e.mode, "input": e.input});
            let done = (|| -> Result<u32, String> {
                let r = self.tv.call(READ_URI, &json!({"category": "picture", "keys": ["backlight", "energySaving"], "dimension": dimension}))?;
                let value = paneldim::backlight(&r).ok_or_else(|| format!("no picture setting: {r}"))?;
                if e.owns(value) && value != e.own {
                    self.write(&dimension, json!({"backlight": e.own.to_string()}))?;
                }
                if let Some(prev) = &e.energy
                    && r.pointer("/settings/energySaving").and_then(Value::as_str) == Some("off")
                {
                    self.write(&dimension, json!({"energySaving": prev}))?;
                }
                Ok(if e.owns(value) { e.own } else { value })
            })();
            match done {
                Ok(value) => {
                    if value == e.own {
                        crate::log_info!("auto brightness: {key} put back to {}", e.own);
                    }
                    self.journal.observe_baseline(&key, value);
                    self.journal.modes.remove(&key);
                }
                Err(err) => crate::log_warn!("auto brightness: putting {key} back: {err}"),
            }
        }
        self.save();
        self.key = None;
        self.journal.modes.is_empty()
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let body = serde_json::to_vec_pretty(&self.journal).unwrap_or_default();
        if let Err(e) = write_atomic(path, &body) {
            crate::log_warn!("auto brightness: saving {}: {e}", path.display());
        }
    }

    pub fn status(&self) -> Value {
        let entry = self.key.as_ref().and_then(|key| self.journal.modes.get(key).map(|e| (key, e)));
        let ceiling = entry.map(|(key, e)| {
            let cap = self.learner.ceiling(key, e.own, self.journal.revision(key));
            self.pending.as_ref().filter(|p| &p.key == key && !p.suspect).map_or(cap, |p| cap.max(p.value))
        });
        json!({
            "on": self.on,
            "active": self.on && self.profile.is_some() && self.hold.is_none() && entry.is_some(),
            "profile": self.profile.as_ref().map(|(id, _)| id),
            "hold": self.hold,
            "lux": self.ambient.lux().map(|l| (l * 10.0).round() / 10.0),
            "level": self.here.as_ref().map(|h| h.value),
            "target": self.target,
            "ceiling": ceiling,
            "mode": self.here.as_ref().map(|h| &h.mode),
            "energySaving": entry.and_then(|(_, e)| e.energy.as_deref()),
            "learned": self.learner.adjustments(),
            "apps": self.learner.apps(),
            "error": self.error,
        })
    }
}

/// The wait after a one-unit step at `level`. A drift (`eased` is `None`) goes at its rate, never under [`MIN_GAP`]. A ramp after a
/// sudden shift is eased by its `progress` (0 at its start, 1 at its end): slowest at both ends, at its rate in the middle.
pub fn gap(level: u32, up: bool, eased: Option<f64>) -> Duration {
    let level = f64::from(level.max(1));
    let secs = match eased {
        None => (1.0 / ((if up { BRIGHTEN_RATE } else { DARKEN_RATE }) * level)).max(MIN_GAP.as_secs_f64()),
        Some(progress) => {
            let speed = EASE_FLOOR + (1.0 - EASE_FLOOR) * (std::f64::consts::PI * progress.clamp(0.0, 1.0)).sin();
            (1.0 / ((if up { FAST_BRIGHTEN_RATE } else { FAST_DARKEN_RATE }) * level * speed)).max(FAST_GAP.as_secs_f64())
        }
    };
    Duration::from_secs_f64(secs)
}

/// The owner's switch: `system.autoBrightness` of theme.json, off unless `true`.
fn switch_on(theme: &Path) -> bool {
    std::fs::read(theme)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.pointer("/system/autoBrightness").and_then(Value::as_bool))
        == Some(true)
}

fn minute_at(epoch: u64) -> Option<u32> {
    let s = i64::try_from(epoch).ok()?;
    let local = s + crate::dnd::utc_offset_s(s)?;
    u32::try_from(local.rem_euclid(86_400) / 60).ok()
}

enum Msg {
    Stop,
    Level((String, u64), u32, mpsc::Sender<Result<Value, String>>),
    Reset((String, u64), mpsc::Sender<Value>),
}

/// The door to the thread for `POST /v1/autobright/level` and `/reset`.
#[derive(Default)]
pub struct Remote(Mutex<Option<Mailbox>>);

struct Mailbox {
    tx: mpsc::Sender<Msg>,
    profiles: Arc<crate::profiles::Profiles>,
}

impl Remote {
    fn send(&self, msg: impl FnOnce((String, u64)) -> Msg) -> Option<()> {
        let mailbox = self.0.lock();
        let mailbox = mailbox.as_ref()?;
        let profile = mailbox.profiles.active_selection()?;
        mailbox.tx.send(msg(profile)).ok()
    }

    /// `None` while the thread is not running.
    pub fn level(&self, value: u32) -> Option<Result<Value, String>> {
        let (tx, rx) = mpsc::channel();
        self.send(|profile| Msg::Level(profile, value, tx))?;
        rx.recv_timeout(Duration::from_secs(10)).ok()
    }

    /// Forgets what was learned; the new status. `None` while the thread is not running.
    pub fn reset(&self) -> Option<Value> {
        let (tx, rx) = mpsc::channel();
        self.send(|profile| Msg::Reset(profile, tx))?;
        rx.recv_timeout(Duration::from_secs(10)).ok()
    }
}

pub fn spawn(ctx: &Ctx, remote: Arc<Remote>, profiles: Arc<crate::profiles::Profiles>) -> io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new().name("autobright".to_owned()).stack_size(256 * 1024).spawn(move || run(&ctx, &remote, profiles))
}

/// The owner's switch, read again only when theme.json changes.
struct Switch {
    path: PathBuf,
    seen: Option<std::time::SystemTime>,
    on: bool,
}

impl Switch {
    fn on(&mut self) -> bool {
        let modified = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if modified.is_none() || modified != self.seen {
            self.seen = modified;
            self.on = switch_on(&self.path);
        }
        self.on
    }
}

/// Use the foreground game rather than a video window left over from the preceding app.
fn scene(hub: &crate::hub::Hub, picture: &Value) -> Scene {
    let game = crate::picture::game_in_front(hub);
    let fronts = hub.get(crate::app_priority::TOPIC);
    let front = fronts.as_ref().and_then(|a| a.pointer("/front/0")).and_then(Value::as_str);
    let playing = picture.get("playing").filter(|v| v.get("full").and_then(Value::as_bool) == Some(true));
    let video = playing.and_then(|v| v.get("app")).and_then(Value::as_str);
    let app = if game { front.or(video) } else { video.or(front) };
    Scene { app: app.unwrap_or_default().to_owned(), playing: playing.is_some() && (!game || video == app), game }
}

fn run(ctx: &Ctx, remote: &Remote, profiles: Arc<crate::profiles::Profiles>) {
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        ctx.stop.on_stop(move || {
            let _ = tx.send(Msg::Stop);
        });
    }
    remote.0.lock().replace(Mailbox { tx, profiles: profiles.clone() });
    let path = ctx.paths.data_dir.join(STATE);
    let mut switch = Switch { path: ctx.paths.theme_file(), seen: None, on: false };
    let dim = ctx.paths.data_dir.join(paneldim::JOURNAL);
    let mut engine = Engine::new(Box::new(LunaSource::new(&ctx.config.luna.send)), load(&path), Some(path), Instant::now());
    let env = |switch: &mut Switch| {
        let mut picture = ctx.hub.get(crate::picture::TOPIC).unwrap_or_default();
        let scene = scene(&ctx.hub, &picture);
        let epoch = crate::timeutil::now_ms() / 1000;
        Env {
            on: switch.on(),
            screen: crate::screen::is_on(&ctx.hub),
            profile: profiles.active_selection(),
            panel_dim: dim.exists(),
            minute: minute_at(epoch),
            epoch,
            context: picture.get_mut("context").map_or(Value::Null, Value::take),
            scene,
        }
    };
    loop {
        engine.tick(Instant::now(), &env(&mut switch));
        ctx.hub.set(TOPIC, engine.status());
        // the switch is looked at every second
        let wait = engine.next().saturating_duration_since(Instant::now()).clamp(Duration::from_millis(50), Duration::from_secs(1));
        match rx.recv_timeout(wait) {
            Ok(Msg::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Level(profile, value, reply)) => {
                let env = env(&mut switch);
                let result = if env.profile.as_ref() == Some(&profile) {
                    engine.by_hand(Instant::now(), value, &env)
                } else {
                    Err("the active profile changed; try the slider again".into())
                };
                let _ = reply.send(result);
            }
            Ok(Msg::Reset(profile, reply)) => {
                let current = profiles.active_selection();
                if current.as_ref() == Some(&profile) {
                    let _ = reply.send(engine.reset_learning(Instant::now(), current.as_ref()));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    remote.0.lock().take();
    engine.release();
}

mod learn;
#[cfg(test)]
mod tests;
