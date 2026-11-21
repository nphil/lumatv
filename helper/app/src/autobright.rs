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
//! - **The decision** is a [`Decide`]: the accepted lux, the recent samples, the time of day and (when known) the picture's level in,
//!   a level out. [`Curve`] is the one in use: from the owner's floor in the dark ([`DARK_LUX`] and below) to his level at
//!   [`BRIGHT_LUX`], on a log scale, shaped by a gamma as Android's adjustment is.
//! - **The owner's level is the ceiling.** Nothing here writes a value above the one the picture mode had when auto brightness took it
//!   over, or a value the owner chose since. Moving the slider while auto brightness is on is read as Android reads it: the value
//!   chosen is the level wanted in this light, and the curve is fitted through it (the gamma, then the floor); a value above the level
//!   becomes the new level.
//! - **The ramp.** One unit a write toward the target, never more: a unit is the smallest step the panel takes, 2% of a level of 50,
//!   so what keeps a step unseen is the time between steps. A drift goes at most [`DARKEN_RATE`] (down) or [`BRIGHTEN_RATE`] (up) of
//!   the level a second, never closer than [`MIN_GAP`], and starts only for a target [`START_DEADBAND`] units away. A sudden shift (a
//!   lamp on or off: [`JUMP_RATIO`] and [`JUMP_LUX`] past the accepted light for [`JUMP_UP_SAMPLES`] or [`JUMP_DOWN_SAMPLES`]
//!   samples in a row) ramps at [`FAST_BRIGHTEN_RATE`] or [`FAST_DARKEN_RATE`], eased at both ends. The switch coming on ramps from the
//!   picture's value; a picture mode coming into force (a video starting) goes to its target in one write, hidden by the mode change.
//!   The design follows `docs/research/auto-brightness-research.md`.
//! - **LG's Energy saving Auto** reads the same sensor and dims on its own; two of them would fight. A mode whose Energy saving is
//!   Auto has it switched off while this drives it, and put back exactly when this lets go. Minimum, Medium and Maximum are fixed caps
//!   and are left as they are. HDR and Dolby Vision modes carry no Energy saving and are never touched.
//! - **One owner of the value at a time.** Nothing is written while the compositor's panel dim holds the value (its journal exists),
//!   while a game is in front or LG's Game mode is in force, or while the screen is off.
//! - **Journal.** `autobright.json` in the data directory holds, per mode, the owner's level, the value last written and the Energy
//!   saving value replaced, before any write. Switching it off, the helper's stop, the sensor failing [`FAILS`] times in a row and the
//!   next start after a crash with the switch off all put every mode back: the owner's level where the value is still the one written
//!   (a value someone changed since stays), and Energy saving as it was. The curve's fit stays in the file.
//!
//! The switch is `system.autoBrightness` of theme.json (off unless `true`). Topic `autobright`. LumaTV's sliders send the owner's
//! brightness through `POST /v1/autobright/level` while it is on, so the value is written and learned at once and the topic's
//! `level`, which they show, never lags the slider.

use crate::ctx::Ctx;
use crate::paneldim::{self, Settings};
use crate::refreshguard::LunaSource;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const TOPIC: &str = "autobright";
/// The journal and the curve's fit, in the data directory.
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
/// The gamma's range (Android's `config_autoBrightnessAdjustmentMaxGamma`).
pub const MAX_GAMMA: f64 = 3.0;

/// What a decision is made from.
pub struct Inputs<'a> {
    /// The accepted room light, in lux.
    pub lux: f64,
    /// The samples of the long horizon: seconds ago and lux, oldest first.
    pub history: &'a [(f64, f64)],
    /// Minutes since local midnight, when the clock can say.
    pub minute: Option<u32>,
    /// The picture's average level, 0 to 1, when something measures it.
    pub content: Option<f64>,
}

/// A decision: a level for the inputs, and what to follow after the owner chose a level himself.
pub trait Decide: Send {
    /// The level wanted, between 0 and `ceiling`.
    fn target(&self, inputs: &Inputs, ceiling: f64) -> f64;
    /// The owner chose `chosen` in these inputs; afterwards [`Decide::target`] gives it for them. Returns the new ceiling, which is
    /// above `ceiling` only when `chosen` is.
    fn learn(&mut self, inputs: &Inputs, ceiling: f64, chosen: f64) -> f64;
    /// What is kept in the journal.
    fn save(&self) -> Value;
}

/// The level from the floor to the ceiling over the log of the room's light, shaped by a gamma.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    /// The level in the dark, as a fraction of the ceiling.
    pub floor: f64,
    pub gamma: f64,
}

impl Default for Curve {
    fn default() -> Self {
        Curve { floor: 0.5, gamma: 1.0 }
    }
}

impl Curve {
    pub fn load(v: &Value) -> Curve {
        serde_json::from_value::<Curve>(v.clone())
            .ok()
            .filter(|c| (0.0..=1.0).contains(&c.floor) && (1.0 / MAX_GAMMA..=MAX_GAMMA).contains(&c.gamma))
            .unwrap_or_default()
    }

    /// How far the room is from dark (0) to lit (1).
    pub fn reach(lux: f64) -> f64 {
        let x = (lux - DARK_LUX).max(0.0);
        ((1.0 + x).ln() / (1.0 + BRIGHT_LUX - DARK_LUX).ln()).min(1.0)
    }

    fn level(&self, reach: f64, ceiling: f64) -> f64 {
        let floor = self.floor * ceiling;
        floor + (ceiling - floor) * reach.powf(self.gamma)
    }
}

impl Decide for Curve {
    fn target(&self, inputs: &Inputs, ceiling: f64) -> f64 {
        self.level(Curve::reach(inputs.lux), ceiling).clamp(0.0, ceiling)
    }

    fn learn(&mut self, inputs: &Inputs, ceiling: f64, chosen: f64) -> f64 {
        let g = Curve::reach(inputs.lux);
        let x = chosen.clamp(0.0, 100.0);
        if x >= ceiling {
            // the most he wants is what he wants here: brighter rooms cannot go above it, darker ones keep their share of it
            if g < 1.0 {
                self.floor = 1.0;
            }
            return x;
        }
        if ceiling <= 0.0 {
            return x;
        }
        if g <= 0.0 {
            self.floor = x / ceiling;
            return ceiling;
        }
        if g >= 1.0 {
            return x;
        }
        let floor = self.floor * ceiling;
        let p = if ceiling - floor > f64::EPSILON { (x - floor) / (ceiling - floor) } else { 0.0 };
        self.gamma = if p > 0.0 && p < 1.0 { (p.ln() / g.ln()).clamp(1.0 / MAX_GAMMA, MAX_GAMMA) } else { MAX_GAMMA };
        // the gamma alone may not reach `x` (its range is bounded): the floor makes up the rest, so the level here is exactly `x`
        let q = g.powf(self.gamma);
        self.floor = ((x - ceiling * q) / (1.0 - q)).clamp(0.0, x) / ceiling;
        ceiling
    }

    fn save(&self) -> Value {
        json!(self)
    }
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

    /// The samples of the long horizon as seconds ago and lux.
    pub fn history(&self, now: Instant) -> Vec<(f64, f64)> {
        self.samples.iter().filter(|(t, _)| *t + LONG >= now).map(|(t, l)| (now.saturating_duration_since(*t).as_secs_f64(), *l)).collect()
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
    /// The owner's level: the ceiling.
    pub own: u32,
    /// The value last written.
    pub written: u32,
    /// LG's Energy saving value this replaced with `off`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    #[serde(default)]
    pub modes: BTreeMap<String, Entry>,
    #[serde(default)]
    pub curve: Value,
}

pub fn load(path: &Path) -> Journal {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// The setting in force: its mode and input, the value and Energy saving. The reply carries no dynamic range (a read of these keys
/// names only `pictureMode` and `input` in its dimension, measured on the set), so the range is [`Env::range`].
#[derive(Clone, Debug, PartialEq)]
pub struct Here {
    pub mode: String,
    pub input: String,
    pub value: u32,
    pub energy: Option<String>,
}

impl Here {
    pub fn parse(reply: &Value) -> Option<Here> {
        let d = reply.get("dimension")?;
        let s = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_owned);
        Some(Here {
            mode: s(d.get("pictureMode")).or_else(|| s(reply.pointer("/settings/pictureMode")))?,
            input: s(d.get("input"))?,
            value: paneldim::backlight(reply)?,
            energy: s(reply.pointer("/settings/energySaving")),
        })
    }

    fn key(&self) -> String {
        format!("{}/{}", self.input, self.mode)
    }
}

/// What the rest of the helper says about now.
#[derive(Clone, Debug, Default)]
pub struct Env {
    /// The owner's switch.
    pub on: bool,
    pub screen: bool,
    /// A game in front, LumaTV's game mode or a named game.
    pub game: bool,
    /// The compositor's panel dim holds the value.
    pub panel_dim: bool,
    pub minute: Option<u32>,
    /// The picture in force as the picture module last heard it (mode, input, range): a change is looked at at once, so a mode
    /// coming into force goes to its target with the mode change.
    pub context: String,
    /// The picture's dynamic range from the same place (`sdr`, `hdr10`, `dolbyVision`...); empty while it is not known. Only SDR is
    /// written.
    pub range: String,
}

/// LG's Game mode: its low-latency picture is left alone.
const LG_GAME: [&str; 2] = ["game", "hdrGame"];

pub struct Engine {
    tv: Box<dyn Settings + Send>,
    path: Option<PathBuf>,
    journal: Journal,
    decide: Box<dyn Decide>,
    ambient: Ambient,
    here: Option<Here>,
    /// The mode driven at the last turn.
    key: Option<String>,
    /// The values last written in that mode, newest last: a read that lags a write is not the owner's.
    recent: VecDeque<u32>,
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
    context: String,
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

impl Engine {
    pub fn new(tv: Box<dyn Settings + Send>, journal: Journal, path: Option<PathBuf>, now: Instant) -> Engine {
        let decide = Box::new(Curve::load(&journal.curve));
        Engine {
            tv,
            path,
            journal,
            decide,
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
            context: String::new(),
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

    pub fn tick(&mut self, now: Instant, env: &Env) {
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
            if !self.journal.modes.is_empty() && ((was && !env.on) || now >= self.next_turn) {
                self.release();
                self.next_turn = now + LOOK;
            }
            return;
        }
        if !env.screen {
            self.hold = Some("screen");
            self.idle(now);
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
        if env.context != self.context {
            self.context.clone_from(&env.context);
            self.next_turn = now;
        }
        if env.game || env.panel_dim {
            self.hold = Some(if env.game { "game" } else { "panel dim" });
            // the mode is snapped to its target when this takes it back
            self.key = None;
            return;
        }
        if now < self.next_turn {
            return;
        }
        self.next_turn = now + LOOK;
        if let Err(e) = self.turn(now, lux, env) {
            self.failed(now, e, false);
        }
    }

    fn idle(&mut self, now: Instant) {
        self.ambient.reset();
        self.here = None;
        self.key = None;
        self.target = None;
        self.ramp = None;
        self.running = false;
        self.next_sample = now + SAMPLE;
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

    fn inputs<'a>(&self, lux: f64, history: &'a [(f64, f64)], env: &Env) -> Inputs<'a> {
        Inputs { lux, history, minute: env.minute, content: None }
    }

    fn turn(&mut self, now: Instant, lux: f64, env: &Env) -> Result<(), String> {
        let reply = self.tv.call(READ_URI, &json!({"category": "picture", "keys": ["backlight", "energySaving"]}))?;
        let here = Here::parse(&reply).ok_or_else(|| format!("no picture setting: {reply}"))?;
        self.fails = 0;
        self.error = None;
        self.here = Some(here.clone());
        if env.range.is_empty() {
            // the picture module has not said yet: look again soon, write nothing
            self.hold = Some("range");
            self.key = None;
            self.target = None;
            self.next_turn = now + MIN_GAP;
            return Ok(());
        }
        if !env.range.eq_ignore_ascii_case("sdr") || LG_GAME.contains(&here.mode.as_str()) {
            self.hold = Some(if LG_GAME.contains(&here.mode.as_str()) { "game" } else { "hdr" });
            self.key = None;
            self.target = None;
            self.ramp = None;
            self.fresh = false;
            return Ok(());
        }
        self.hold = None;
        let key = here.key();
        let history = self.ambient.history(now);
        let inputs = self.inputs(lux, &history, env);
        let dimension = json!({"pictureMode": here.mode, "input": here.input});
        let fresh = std::mem::take(&mut self.fresh);
        let mut entered = self.key.as_deref() != Some(key.as_str());
        let known = self.journal.modes.get(&key).cloned();
        if fresh && known.as_ref().is_some_and(|e| e.written == here.value) {
            // a restart with the switch on: the value is the one this wrote, so it carries on from there
            entered = false;
            self.recent = VecDeque::from([here.value]);
        }
        let mut entry = if let Some(entry) = known {
            entry
        } else {
            // a mode taken over: its value is the owner's level. The owner just switched this on: it ramps from there. A mode change
            // hides it: it goes to its target at once. Then LG's Energy saving Auto is switched off.
            let mut entry = Entry {
                mode: here.mode.clone(),
                input: here.input.clone(),
                own: here.value,
                written: here.value,
                energy: here.energy.clone().filter(|e| e == "auto"),
            };
            if !fresh {
                let target = self.decide.target(&inputs, f64::from(here.value)).round() as u32;
                entry.written = target.min(here.value);
            }
            crate::log_info!("auto brightness: {key} taken over at {} (level {}), lux {lux:.1}", entry.written, entry.own);
            self.journal.modes.insert(key.clone(), entry.clone());
            self.save();
            self.key = Some(key.clone());
            self.recent = VecDeque::from([entry.written]);
            self.target = Some(entry.written);
            self.ramp = None;
            if entry.written != here.value {
                self.write(&dimension, json!({"backlight": entry.written.to_string()}))?;
            }
            if entry.energy.is_some() {
                self.write(&dimension, json!({"energySaving": "off"}))?;
            }
            if !fresh {
                self.next_turn = now + MIN_GAP;
                return Ok(());
            }
            entered = false;
            entry
        };
        // Energy saving put back to Auto meanwhile (a preset, LG's own menu): this is the one switch, so it goes off again and the
        // value it had is what a release puts back
        if here.energy.as_deref() == Some("auto") {
            entry.energy = Some("auto".into());
            self.journal.modes.insert(key.clone(), entry.clone());
            self.save();
            self.write(&dimension, json!({"energySaving": "off"}))?;
        }
        // a read that lags a write of this mode is not the owner's; the other mode's writes say nothing about this one
        let ours = here.value == entry.written || (!entered && self.recent.contains(&here.value));
        if entered {
            self.recent = VecDeque::from([entry.written]);
            if !ours {
                // changed while the mode was not in force: whoever changed it set the mode's level
                entry.own = here.value;
            }
        } else if !ours {
            // set by hand elsewhere (LG's own menu): the value chosen is the level wanted in this light
            self.ramp = None;
            self.learned(&inputs, &key, entry, here.value);
            return Ok(());
        }
        let target = (self.decide.target(&inputs, f64::from(entry.own)).round() as u32).min(entry.own);
        self.target = Some(target);
        self.key = Some(key.clone());
        if std::mem::take(&mut self.sudden) {
            self.ramp = Some(Ramp { from: here.value, fast: true });
        }
        let value = if entered {
            self.ramp = None;
            target
        } else {
            let apart = target.abs_diff(here.value);
            if apart == 0 {
                self.ramp = None;
                return Ok(());
            }
            if self.ramp.is_none() {
                if apart < START_DEADBAND {
                    return Ok(());
                }
                self.ramp = Some(Ramp { from: here.value, fast: false });
            }
            if target > here.value { here.value + 1 } else { here.value - 1 }
        };
        if entered && value == here.value {
            if entry != self.journal.modes[&key] {
                self.journal.modes.insert(key, entry);
                self.save();
            }
            return Ok(());
        }
        entry.written = value;
        self.journal.modes.insert(key, entry);
        self.save();
        self.recent.push_back(value);
        while self.recent.len() > 4 {
            self.recent.pop_front();
        }
        self.write(&dimension, json!({"backlight": value.to_string()}))?;
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

    /// The owner chose `value` in this light: the decision is fitted through it and the mode's level rises only to it.
    fn learned(&mut self, inputs: &Inputs, key: &str, mut entry: Entry, value: u32) {
        let ceiling = self.decide.learn(inputs, f64::from(entry.own), f64::from(value));
        entry.own = (ceiling.round() as u32).max(value).min(100);
        entry.written = value;
        crate::log_info!("auto brightness: {key} set to {value} by hand at lux {:.1}; level {}", inputs.lux, entry.own);
        self.journal.curve = self.decide.save();
        self.journal.modes.insert(key.to_owned(), entry);
        self.save();
        self.recent = VecDeque::from([value]);
        self.target = Some(value);
        self.ramp = None;
    }

    /// The owner's slider (`POST /v1/autobright/level`): written at once, and while this drives the mode in force, learned as
    /// [`Engine::turn`] learns a value set elsewhere, without waiting for the next look.
    pub fn by_hand(&mut self, now: Instant, value: u32, env: &Env) -> Result<Value, String> {
        let value = value.min(100);
        let reply = self.tv.call(READ_URI, &json!({"category": "picture", "keys": ["backlight", "energySaving"]}))?;
        let mut here = Here::parse(&reply).ok_or_else(|| format!("no picture setting: {reply}"))?;
        let key = here.key();
        let driving = self.on && self.hold.is_none() && self.key.as_deref() == Some(key.as_str());
        if let (true, Some(lux), Some(entry)) = (driving, self.ambient.lux(), self.journal.modes.get(&key).cloned()) {
            let history = self.ambient.history(now);
            self.learned(&self.inputs(lux, &history, env), &key, entry, value);
            self.next_turn = now + LOOK;
        }
        self.write(&json!({"pictureMode": here.mode, "input": here.input}), json!({"backlight": value.to_string()}))?;
        here.value = value;
        self.here = Some(here);
        Ok(self.status())
    }

    fn write(&self, dimension: &Value, settings: Value) -> Result<(), String> {
        let r = self.tv.call(WRITE_URI, &json!({"category": "picture", "settings": settings, "dimension": dimension}))?;
        if r.get("returnValue").and_then(Value::as_bool) == Some(true) { Ok(()) } else { Err(format!("write refused: {r}")) }
    }

    /// Puts every mode back: the owner's level where the value is still the one written, Energy saving as it was. A mode the service
    /// did not answer for stays in the journal for the next try. True when nothing is left.
    pub fn release(&mut self) -> bool {
        let keys: Vec<String> = self.journal.modes.keys().cloned().collect();
        for key in keys {
            let e = self.journal.modes[&key].clone();
            let dimension = json!({"pictureMode": e.mode, "input": e.input});
            let done = (|| -> Result<(), String> {
                let r = self.tv.call(READ_URI, &json!({"category": "picture", "keys": ["backlight", "energySaving"], "dimension": dimension}))?;
                let value = paneldim::backlight(&r);
                if value == Some(e.written) && e.written != e.own {
                    self.write(&dimension, json!({"backlight": e.own.to_string()}))?;
                }
                if let Some(prev) = &e.energy
                    && r.pointer("/settings/energySaving").and_then(Value::as_str) == Some("off")
                {
                    self.write(&dimension, json!({"energySaving": prev}))?;
                }
                Ok(())
            })();
            match done {
                Ok(()) => {
                    crate::log_info!("auto brightness: {key} put back to {}", e.own);
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
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, body).and_then(|()| std::fs::rename(&tmp, path)) {
            crate::log_warn!("auto brightness: saving {}: {e}", path.display());
        }
    }

    pub fn status(&self) -> Value {
        let entry = self.here.as_ref().and_then(|h| self.journal.modes.get(&h.key()));
        json!({
            "on": self.on,
            "active": self.on && self.hold.is_none() && entry.is_some(),
            "hold": self.hold,
            "lux": self.ambient.lux().map(|l| (l * 10.0).round() / 10.0),
            "level": self.here.as_ref().map(|h| h.value),
            "target": self.target,
            "ceiling": entry.map(|e| e.own),
            "mode": self.here.as_ref().map(|h| h.mode.clone()),
            "energySaving": entry.and_then(|e| e.energy.clone()),
            "curve": self.decide.save(),
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

fn minute_now() -> Option<u32> {
    let s = i64::try_from(crate::timeutil::now_ms() / 1000).ok()?;
    let local = s + crate::dnd::utc_offset_s(s)?;
    u32::try_from(local.rem_euclid(86_400) / 60).ok()
}

enum Msg {
    Stop,
    Level(u32, mpsc::Sender<Result<Value, String>>),
}

/// The door to the thread for `POST /v1/autobright/level`.
#[derive(Default)]
pub struct Remote(Mutex<Option<mpsc::Sender<Msg>>>);

impl Remote {
    /// `None` while the thread is not running.
    pub fn level(&self, value: u32) -> Option<Result<Value, String>> {
        let (tx, rx) = mpsc::channel();
        self.0.lock().as_ref()?.send(Msg::Level(value, tx)).ok()?;
        // a read and a write, each up to luna-send's deadline
        rx.recv_timeout(Duration::from_secs(10)).ok()
    }
}

pub fn spawn(ctx: &Ctx, remote: Arc<Remote>) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new().name("autobright".to_owned()).stack_size(256 * 1024).spawn(move || run(&ctx, &remote))
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

fn run(ctx: &Ctx, remote: &Remote) {
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        ctx.stop.on_stop(move || {
            let _ = tx.send(Msg::Stop);
        });
    }
    remote.0.lock().replace(tx);
    let path = ctx.paths.data_dir.join(STATE);
    let mut switch = Switch { path: ctx.paths.theme_file(), seen: None, on: false };
    let dim = ctx.paths.data_dir.join(paneldim::JOURNAL);
    let mut engine = Engine::new(Box::new(LunaSource::new(&ctx.config.luna.send)), load(&path), Some(path), Instant::now());
    let env = |switch: &mut Switch| {
        let picture = ctx.hub.get(crate::picture::TOPIC);
        Env {
            on: switch.on(),
            screen: crate::screen::is_on(&ctx.hub),
            game: crate::picture::game_in_front(&ctx.hub),
            panel_dim: dim.exists(),
            minute: minute_now(),
            context: picture.as_ref().and_then(|p| p.get("context").map(Value::to_string)).unwrap_or_default(),
            range: picture.as_ref().and_then(|p| p.pointer("/context/range").and_then(Value::as_str).map(str::to_owned)).unwrap_or_default(),
        }
    };
    loop {
        engine.tick(Instant::now(), &env(&mut switch));
        ctx.hub.set(TOPIC, engine.status());
        // the switch is looked at every second
        let wait = engine.next().saturating_duration_since(Instant::now()).clamp(Duration::from_millis(50), Duration::from_secs(1));
        match rx.recv_timeout(wait) {
            Ok(Msg::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Level(value, reply)) => {
                let _ = reply.send(engine.by_hand(Instant::now(), value, &env(&mut switch)));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    remote.0.lock().take();
    engine.release();
}

#[cfg(test)]
mod tests;
