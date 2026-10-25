//! Ambient light, the helper's side: the settings the person chose, the engine that turns the picture's areas into colours, and
//! the lights they go to.
//!
//! ```text
//!   Settings page ──► base.ambient ──► plan (lumatv-ambient: engine config + the strip's lights + the Hue area's lights)
//!   capture helper ──► unix socket ──► Engine ──┬─► DDP over UDP ──► the WLED strip named in helper.json
//!                                               └─► hue::Output ──► the Hue bridge: HTTPS start/stop, DTLS HueStream
//! ```
//!
//! - **Nothing is sent to a device nobody named.** The strip's address is `ambient.wled` in `helper.json` (a private IPv4
//!   address: the picture's colours never leave the home network); the Hue bridge is the one `lumatv-helper hue-link` saved
//!   (`hue_store`, also a private address, with the keys and the certificate the bridge must present). With neither, ambient light
//!   has nothing to drive, says so in the `ambient` topic's `problem`, and the test pattern is refused.
//! - **Nothing runs while it is not wanted.** The thread sleeps until the settings, the screen, the context or a frame change what
//!   is wanted; it ticks at 30 Hz (the reader polls at 10 a second by default) only while the picture is moving the lights, sends a still colour once a second (WLED leaves
//!   realtime mode 2.5 s after the last packet), and after the lights have faded to black it sends one black frame and goes quiet.
//!   The capture socket exists only while a picture is wanted. The Hue stream is open exactly while the lights are lit: when the
//!   screen goes off (or ambient light is switched off, or Home covers the picture in the mode that goes dark there) the lights
//!   fade, then the stream ends and the area is handed back to the bridge. Before the bridge's lights are known (the engine needs
//!   one light per channel) the area is read once, without starting it, as soon as ambient light is on and the screen is lit.
//! - **The picture comes from the capture helper** (32-bit, `ambient/capture`): one `wire` frame per poll, already
//!   reduced to a colour per area. The helper answers with one byte, the hardware preset the engine wants (`Engine::wanted_preset`:
//!   plain, letterbox, pillarbox or games), on connect and whenever it changes.
//! - **The capture permission flag is not touched here.** LG's library refuses the capture to any process but its own Hue Sync
//!   service unless `/var/luna/preferences/vt_devmode` exists; creating it is the owner's decision, made for short supervised probe
//!   slots (`ambient/capture/tv_probe_slot.py`), and no code of the helper creates it.
//!
//! The topic `ambient`: `{enabled, mode, brightness, sections, available, strip, hue, hueState, state, problem, covered}`. `state` is
//! `off` (not enabled), `idle` (enabled, but nothing to do now: the screen is off, or Home covers the picture in the mode that goes
//! dark there), `waiting` (enabled, no picture frames yet), `running` or `testing`. `available` says there is a light to drive;
//! `strip` and `hue` say which (`hue`: a bridge is linked with an area); `hueState` is the Hue output's own: `idle`, `connecting`,
//! `streaming` or `error` (its words are in `problem`).

use crate::config::WledConfig;
use crate::control::Stop;
use crate::ctx::Ctx;
use crate::hue::{self, Demand, Event, Frame};
use crate::hue_rest::Tls;
use crate::hue_store;
use crate::screen;
use crate::settings::{SettingsStore, wake_on_change};
use lumatv_ambient::ddp::{self, Format};
use lumatv_ambient::engine::Engine;
use lumatv_ambient::plan::{Edges, Mode, Plan, Settings, Still};
use lumatv_ambient::wire;
use lumatv_ambient::zones::Preset;
use lumatv_ambient::{Light, Rgb, Target, ZoneFrame, hue_channels, wled_leds};
use parking_lot::{Condvar, Mutex};
use serde_json::{Value as Json, json};
use settings_schema::{Value, Values};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The hub topic.
pub const TOPIC: &str = "ambient";
/// The settings owner (`schemas/base.ambient.json`).
pub const OWNER: &str = "base.ambient";

/// The picture's tick: 30 lights updates a second is what the capture helper polls at and what a strip shows.
const TICK: Duration = Duration::from_millis(33);
/// A still colour is sent again this often, so WLED stays in realtime mode (it leaves 2.5 s after the last packet).
const KEEPALIVE: Duration = Duration::from_secs(1);
/// How long each section of the test pattern is lit, and the most it runs (two passes, never more than a minute).
const TEST_STEP: Duration = Duration::from_millis(120);
const TEST_MAX: Duration = Duration::from_secs(60);
const TEST_PASSES: u32 = 2;
/// How long after the last frame the lights are left to fade (the engine's own `stale_ms` plus its settle time).
const FADE_GRACE: Duration = Duration::from_millis(400);
/// After a failed send the strip is tried again no sooner than this.
const RETRY: Duration = Duration::from_secs(2);

/// The settings as the person left them (the schema's defaults for what is missing or unusable).
pub fn read_settings(values: &Values) -> Settings {
    let d = Settings::default();
    let flag = |key: &str, default: bool| match values.get(key) {
        Some(Value::Bool(b)) => *b,
        _ => default,
    };
    let int = |key: &str, default: i64| match values.get(key) {
        Some(Value::Integer(n)) => *n,
        _ => default,
    };
    let choice = |key: &str| match values.get(key) {
        Some(Value::Choice(c)) => Some(c.to_string()),
        _ => None,
    };
    let count = |key: &str, default: u16| {
        u16::try_from(int(key, i64::from(default)).clamp(0, 120)).unwrap_or(default)
    };
    Settings {
        enabled: flag("enabled", d.enabled),
        mode: choice("mode")
            .and_then(|m| Mode::parse(&m))
            .unwrap_or(d.mode),
        brightness: u8::try_from(int("brightness", i64::from(d.brightness)).clamp(0, 100))
            .unwrap_or(d.brightness),
        still: choice("still_colour")
            .and_then(|c| Still::parse(&c))
            .unwrap_or(d.still),
        saturation: u8::try_from(int("saturation", i64::from(d.saturation)).clamp(50, 150))
            .unwrap_or(d.saturation),
        dark_hold: flag("dark_hold", d.dark_hold),
        skip_bars: flag("skip_bars", d.skip_bars),
        game_edges: flag("game_edges", d.game_edges),
        delay_ms: u32::try_from(int("delay_ms", i64::from(d.delay_ms)).clamp(0, 2000))
            .unwrap_or(d.delay_ms),
        edges: Edges {
            top: count("top", d.edges.top),
            right: count("right", d.edges.right),
            bottom: count("bottom", d.edges.bottom),
            left: count("left", d.edges.left),
        },
    }
}

/// Why the test pattern was not started.
#[derive(Debug, PartialEq, Eq)]
pub enum TestError {
    /// No strip is named in `helper.json`.
    NoStrip,
    /// The settings make no lights (every edge has no sections).
    NoLights(String),
}

struct TestRun {
    started: Instant,
}

struct Inner {
    settings: Settings,
    plan: Option<Plan>,
    engine: Option<Engine>,
    /// Why there is no plan, or what last went wrong sending.
    problem: Option<String>,
    covered: bool,
    game: bool,
    test: Option<TestRun>,
    last_frame: Option<Instant>,
    /// Lights may be lit: a black frame is owed before the thread goes quiet.
    live: bool,
    last_sent: Option<(Vec<u8>, Instant)>,
    retry_at: Option<Instant>,
    /// The preset last told to the capture link.
    told_preset: Option<u8>,
    state: &'static str,
    /// The thread is waiting with no deadline: a frame or a change must wake it.
    sleeping: bool,
    dirty: bool,
    /// The Hue output, when a bridge is linked with an area.
    hue: Option<Arc<hue::Output>>,
    /// Why Hue is not driven although a bridge was linked: no area chosen, or a link that cannot be read.
    hue_note: Option<String>,
    /// The area's channels as lights, once the area has been read (nothing until then).
    hue_lights: Vec<Light>,
    /// What the Hue output is sent after this step.
    hue_frame: Option<Frame>,
    /// The last Hue frame had a lit channel.
    hue_lit: bool,
}

/// What one step of the thread has to do.
#[derive(Debug, PartialEq)]
pub struct Step {
    /// The strip's pixels to send now (RGB bytes, one LED after the other), if any.
    pub frame: Option<Vec<u8>>,
    /// How long to sleep before the next step; `None` waits for a change.
    pub next: Option<Duration>,
}

pub struct Ambient {
    ctx: Ctx,
    target: Option<SocketAddr>,
    origin: Instant,
    inner: Mutex<Inner>,
    wake: Condvar,
}

impl Ambient {
    /// The component, not yet running: its status and routes work, no thread exists.
    pub fn new(ctx: &Ctx) -> Arc<Self> {
        let target = ctx.config.ambient.wled.as_ref().and_then(target_of);
        let settings = settings_of(&ctx.settings);
        let ambient = Arc::new(Self {
            ctx: ctx.clone(),
            target,
            origin: Instant::now(),
            inner: Mutex::new(Inner {
                settings,
                plan: None,
                engine: None,
                problem: None,
                covered: false,
                game: false,
                test: None,
                last_frame: None,
                live: false,
                last_sent: None,
                retry_at: None,
                told_preset: None,
                state: "off",
                sleeping: false,
                dirty: true,
                hue: None,
                hue_note: None,
                hue_lights: Vec::new(),
                hue_frame: None,
                hue_lit: false,
            }),
            wake: Condvar::new(),
        });
        {
            let weak = Arc::downgrade(&ambient);
            wake_on_change(&ctx.settings, OWNER, &ctx.stop, move || {
                if let Some(a) = weak.upgrade() {
                    a.settings_changed();
                }
            });
        }
        ambient.reload_hue();
        {
            let mut g = ambient.inner.lock();
            ambient.rebuild(&mut g);
            ambient.refresh_state(&mut g, screen::is_on(&ctx.hub), Instant::now());
            ambient.publish(&g);
        }
        ambient
    }

    /// Something that decides what the lights do has changed (the settings, the screen, the context): look again.
    pub fn wake(&self) {
        self.inner.lock().dirty = true;
        self.wake.notify_all();
    }

    /// Brings the state the status shows up to date now, for a change the API has just made (the thread would only get to it a
    /// moment later, and the caller reads the status in its answer).
    fn settle(&self, g: &mut Inner) {
        let on = screen::is_on(&self.ctx.hub);
        self.refresh_state(g, on, Instant::now());
        self.publish(g);
    }

    fn settings_changed(&self) {
        let settings = settings_of(&self.ctx.settings);
        let mut g = self.inner.lock();
        if g.settings != settings {
            g.settings = settings;
        }
        g.dirty = true;
        drop(g);
        self.wake.notify_all();
    }

    /// What covers the picture: Home or a menu (`covered`), a game on the input (`game`). Either may be left out.
    pub fn set_context(&self, covered: Option<bool>, game: Option<bool>) {
        let mut g = self.inner.lock();
        if let Some(c) = covered {
            g.covered = c;
        }
        if let Some(on) = game {
            g.game = on;
        }
        g.dirty = true;
        self.settle(&mut g);
        drop(g);
        self.wake.notify_all();
    }

    /// Starts (`run`) or stops the test pattern: each section in turn, twice round, so the layout can be checked.
    pub fn test(&self, run: bool) -> Result<(), TestError> {
        let mut g = self.inner.lock();
        if !run {
            g.test = None;
            g.dirty = true;
            self.settle(&mut g);
            drop(g);
            self.wake.notify_all();
            return Ok(());
        }
        if self.target.is_none() {
            return Err(TestError::NoStrip);
        }
        self.rebuild(&mut g);
        match &g.plan {
            Some(plan) if plan.sections > 0 => {}
            _ => {
                return Err(TestError::NoLights(
                    g.problem
                        .clone()
                        .unwrap_or_else(|| "no sections on any edge".to_owned()),
                ));
            }
        }
        g.test = Some(TestRun {
            started: Instant::now(),
        });
        g.dirty = true;
        self.settle(&mut g);
        drop(g);
        self.wake.notify_all();
        Ok(())
    }

    /// A frame of the picture, from the capture link (or a test).
    pub fn push_frame(&self, frame: ZoneFrame, now: Instant) {
        let mut g = self.inner.lock();
        let ms = self.ms(now);
        let Some(engine) = g.engine.as_mut() else {
            return;
        };
        engine.push_frame(ms, frame);
        g.last_frame = Some(now);
        let wake = g.sleeping;
        drop(g);
        if wake {
            self.wake.notify_all();
        }
    }

    /// The hardware preset the engine wants the capture helper to run, when it has changed since it was last told.
    pub fn preset_to_tell(&self) -> Option<u8> {
        let mut g = self.inner.lock();
        let wanted = g.engine.as_ref()?.wanted_preset().index();
        (g.told_preset != Some(wanted)).then(|| {
            g.told_preset = Some(wanted);
            wanted
        })
    }

    /// Forgets what the capture link was told (a new capture helper connected).
    pub fn forget_told(&self) {
        self.inner.lock().told_preset = None;
    }

    fn ms(&self, now: Instant) -> u64 {
        u64::try_from(now.saturating_duration_since(self.origin).as_millis()).unwrap_or(u64::MAX)
    }

    /// Makes the plan and the engine from the settings now in `g`. A plan that cannot be made is the `problem`.
    fn rebuild(&self, g: &mut Inner) {
        g.dirty = false;
        let others = g.hue_lights.clone();
        match g.settings.plan(&others) {
            Ok(plan) => match Engine::new(plan.engine.clone(), plan.lights.clone()) {
                Ok(mut engine) => {
                    engine.set_games(plan.game_edges && g.game);
                    // An engine made anew starts with no picture: it is told again what preset it wants.
                    g.told_preset = None;
                    g.engine = Some(engine);
                    g.plan = Some(plan);
                    g.problem = None;
                }
                Err(e) => {
                    g.engine = None;
                    g.plan = None;
                    g.problem = Some(format!("the lights cannot be set up: {e}"));
                }
            },
            Err(e) => {
                g.engine = None;
                g.plan = None;
                g.problem = Some(format!("the lights cannot be set up: {e}"));
            }
        }
    }

    /// Whether the plan has a light something of this helper can show: a strip's LED when there is a strip to send to, a Hue channel
    /// when there is a bridge.
    fn drives(&self, hue: bool, plan: &Plan) -> bool {
        (self.target.is_some() && plan.sections > 0) || (hue && plan.lights.len() > plan.sections)
    }

    /// What the Hue output is asked for after a step: the stream while the lights are lit; a look at the area (without starting it)
    /// while its lights are not known and ambient light is on; nothing otherwise, which ends a stream that is open (the screen went
    /// off, ambient light was switched off, the lights have faded).
    fn offer_hue(&self, g: &mut Inner, want: Want, screen_on: bool) {
        let frame = g.hue_frame.take();
        let Some(output) = g.hue.clone() else {
            return;
        };
        let known = !g.hue_lights.is_empty();
        let demand = if known && g.live && want != Want::Test {
            Demand::Stream
        } else if !known && g.settings.enabled && screen_on {
            Demand::Discover
        } else {
            Demand::Off
        };
        output.set(demand, frame);
    }

    /// Reads the saved Hue link again and starts the Hue output for it (a running one is ended first). The command line asks for
    /// this after it changes the link (`POST /v1/ambient/refresh`).
    pub fn reload_hue(self: &Arc<Self>) {
        let (output, note) = match hue_store::load(&self.ctx.paths.data_dir) {
            Ok(Some(link)) if link.area.is_some() => {
                let weak = Arc::downgrade(self);
                let events: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |event| {
                    if let Some(ambient) = weak.upgrade() {
                        ambient.hue_event(event);
                    }
                });
                let tls = Tls::from_config(&self.ctx.config.fetch);
                match hue::Output::spawn(link, &self.ctx.paths.data_dir, tls, events) {
                    Ok(output) => (Some(Arc::new(output)), None),
                    Err(e) => (None, Some(format!("the Hue output cannot start: {e}"))),
                }
            }
            Ok(Some(_)) => (
                None,
                Some(
                    "Hue is linked but no entertainment area is chosen: run lumatv-helper hue-area"
                        .to_owned(),
                ),
            ),
            Ok(None) => (None, None),
            Err(e) => (None, Some(format!("the Hue link cannot be used: {e}"))),
        };
        if let Some(note) = &note {
            crate::log_warn!("ambient: {note}");
        }
        let old = {
            let mut g = self.inner.lock();
            let old = std::mem::replace(&mut g.hue, output);
            g.hue_note = note;
            g.hue_lights.clear();
            g.hue_frame = None;
            g.hue_lit = false;
            g.dirty = true;
            self.publish(&g);
            old
        };
        // Ending a stream takes a moment (the bridge is told): not with the lock held.
        drop(old);
        self.wake.notify_all();
    }

    /// What the Hue output says, from its own thread.
    fn hue_event(&self, event: Event) {
        let mut g = self.inner.lock();
        if let Event::Lights(lights) = event
            && g.hue_lights != lights
        {
            g.hue_lights = lights;
            g.dirty = true;
        }
        self.publish(&g);
        let dirty = g.dirty;
        drop(g);
        if dirty {
            self.wake.notify_all();
        }
    }

    /// The topic's value.
    fn topic_json(&self, g: &Inner) -> Json {
        let hue = g.hue.as_ref().map(|o| o.status());
        let hue_words = hue
            .as_ref()
            .and_then(|s| s.problem.as_deref())
            .map(|p| format!("Hue: {p}"));
        let problem = g
            .problem
            .clone()
            .or(hue_words)
            .or_else(|| g.hue_note.clone())
            .or_else(|| {
                (self.target.is_none() && g.hue.is_none()).then(|| {
                    "No light is set up: helper.json needs ambient.wled.host, or run lumatv-helper hue-link".to_owned()
                })
            });
        json!({
            "enabled": g.settings.enabled,
            "mode": g.settings.mode.word(),
            "brightness": g.settings.brightness,
            "sections": g.settings.edges.sum(),
            "available": self.target.is_some() || g.hue.is_some(),
            "strip": self.target.is_some(),
            "hue": g.hue.is_some(),
            "hueState": hue.map_or("off", |s| s.state),
            "state": g.state,
            "problem": problem,
            "covered": g.covered,
        })
    }

    fn publish(&self, g: &Inner) {
        self.ctx.hub.set(TOPIC, self.topic_json(g));
    }

    /// The status the API serves.
    pub fn status(&self) -> Json {
        self.topic_json(&self.inner.lock())
    }

    /// What is wanted of the lights now.
    fn want(&self, g: &mut Inner, screen_on: bool, now: Instant) -> Want {
        if let Some(run) = &g.test {
            let sections = g.plan.as_ref().map_or(0, |p| p.sections);
            if self.target.is_some()
                && sections > 0
                && now.saturating_duration_since(run.started) < test_length(sections)
            {
                return Want::Test;
            }
            g.test = None;
        }
        let hue = g.hue.is_some();
        let Some(plan) = g.plan.as_ref().filter(|p| self.drives(hue, p)) else {
            return Want::Idle;
        };
        if !plan.enabled || !screen_on {
            return Want::Idle;
        }
        match plan.mode {
            Mode::Colour => Want::Still,
            Mode::PictureOffHome if g.covered => Want::Idle,
            Mode::Picture | Mode::PictureOffHome => Want::Picture,
        }
    }

    fn refresh_state(&self, g: &mut Inner, screen_on: bool, now: Instant) -> Want {
        let want = self.want(g, screen_on, now);
        g.state = match want {
            Want::Test => "testing",
            _ if !g.settings.enabled => "off",
            Want::Idle => "idle",
            Want::Still => "running",
            Want::Picture => {
                let fresh = g
                    .last_frame
                    .is_some_and(|t| now.saturating_duration_since(t) < FADE_GRACE * 4);
                if fresh { "running" } else { "waiting" }
            }
        };
        want
    }

    /// One step: decide, tick the engine, and say what to send and when to look again. No I/O: the thread sends.
    pub fn step(&self, now: Instant) -> Step {
        let screen_on = screen::is_on(&self.ctx.hub);
        let mut g = self.inner.lock();
        if g.dirty {
            self.rebuild(&mut g);
        }
        if g.retry_at.is_some_and(|t| now < t) {
            let wait = g.retry_at.map(|t| t.saturating_duration_since(now));
            return Step {
                frame: None,
                next: wait,
            };
        }
        let want = self.refresh_state(&mut g, screen_on, now);
        let leds = g.plan.as_ref().map_or(0, |p| p.sections);
        g.hue_frame = None;
        let step = match want {
            Want::Test => self.step_test(&mut g, now, leds),
            Want::Still => self.step_still(&mut g, now, leds),
            Want::Picture => self.step_picture(&mut g, now, leds, true),
            Want::Idle => {
                if g.engine.is_some()
                    && g.live
                    && (g.hue_lit
                        || g.last_sent
                            .as_ref()
                            .is_some_and(|(f, _)| f.iter().any(|b| *b != 0)))
                {
                    // The picture's lights fade out through the engine; anything else is cut to black at once.
                    if g.settings.mode.follows_picture() && g.last_frame.is_some() {
                        self.step_picture(&mut g, now, leds, false)
                    } else {
                        Self::black(&mut g, now, leds)
                    }
                } else {
                    g.live = false;
                    Step {
                        frame: None,
                        next: None,
                    }
                }
            }
        };
        g.sleeping = step.next.is_none();
        // The state may have moved while stepping (a test ended, the lights went dark).
        let screen_now = screen_on;
        let _ = self.refresh_state(&mut g, screen_now, now);
        self.offer_hue(&mut g, want, screen_on);
        self.publish(&g);
        step
    }

    fn black(g: &mut Inner, now: Instant, leds: usize) -> Step {
        g.live = false;
        g.hue_lit = false;
        let frame = vec![0u8; leds * 3];
        g.last_sent = Some((frame.clone(), now));
        Step {
            frame: Some(frame),
            next: None,
        }
    }

    fn step_still(&self, g: &mut Inner, now: Instant, leds: usize) -> Step {
        let Some(plan) = &g.plan else {
            return Step {
                frame: None,
                next: None,
            };
        };
        let rgb = plan.still.to_u8();
        let frame: Vec<u8> = (0..leds).flat_map(|_| rgb).collect();
        g.live = true;
        let on_hue = |l: &Light| matches!(l.target, Target::Hue { .. });
        let hue_lit = plan.lights.iter().any(on_hue) && rgb != [0; 3];
        let colours = vec![plan.still; plan.lights.len()];
        g.hue_frame = Some(hue_channels(&plan.lights, &colours));
        g.hue_lit = hue_lit;
        let due = g.last_sent.as_ref().is_none_or(|(sent, at)| {
            *sent != frame || now.saturating_duration_since(*at) >= KEEPALIVE
        });
        if due {
            g.last_sent = Some((frame.clone(), now));
            Step {
                frame: Some(frame),
                next: Some(KEEPALIVE),
            }
        } else {
            let waited = g
                .last_sent
                .as_ref()
                .map_or(Duration::ZERO, |(_, at)| now.saturating_duration_since(*at));
            Step {
                frame: None,
                next: Some(KEEPALIVE.saturating_sub(waited)),
            }
        }
    }

    /// The picture's tick. With `on` false the engine is being faded out.
    fn step_picture(&self, g: &mut Inner, now: Instant, leds: usize, on: bool) -> Step {
        let ms = self.ms(now);
        let game = g.plan.as_ref().is_some_and(|p| p.game_edges) && g.game;
        let last_frame = g.last_frame;
        let (Some(engine), Some(plan)) = (g.engine.as_mut(), g.plan.as_ref()) else {
            return Step {
                frame: None,
                next: None,
            };
        };
        engine.set_enabled(on);
        engine.set_games(game);
        let colours: Vec<Rgb> = engine.tick(ms).to_vec();
        let frame = wled_leds(&plan.lights, &colours, leds);
        let hue_frame = hue_channels(&plan.lights, &colours);
        let hue_lit = plan
            .lights
            .iter()
            .zip(&colours)
            .any(|(l, c)| matches!(l.target, Target::Hue { .. }) && c.to_u8() != [0; 3]);
        // Dark is every light, the strip's and the Hue area's, rounding to black.
        let dark = colours.iter().all(|c| c.to_u8() == [0; 3]);
        let quiet =
            last_frame.is_none_or(|t| now.saturating_duration_since(t) >= FADE_GRACE) || !on;
        if dark && quiet {
            // Faded out and nothing arriving: one black frame (if the last thing sent was not), then wait for a change.
            // Nothing sent yet means the strip was never lit by this: there is nothing to take down.
            let already_black = g
                .last_sent
                .as_ref()
                .is_none_or(|(f, _)| f.iter().all(|b| *b == 0));
            g.live = false;
            g.hue_lit = false;
            if already_black {
                return Step {
                    frame: None,
                    next: None,
                };
            }
            g.last_sent = Some((frame.clone(), now));
            return Step {
                frame: Some(frame),
                next: None,
            };
        }
        g.live = true;
        g.hue_lit = hue_lit;
        g.hue_frame = Some(hue_frame);
        g.last_sent = Some((frame.clone(), now));
        Step {
            frame: Some(frame),
            next: Some(TICK),
        }
    }

    /// The test pattern: one section white, each in turn.
    fn step_test(&self, g: &mut Inner, now: Instant, leds: usize) -> Step {
        let started = g.test.as_ref().map_or(now, |t| t.started);
        let n = u128::try_from(leds.max(1)).unwrap_or(1);
        let at = now.saturating_duration_since(started).as_millis() / TEST_STEP.as_millis();
        let lit = usize::try_from(at % n).unwrap_or(0);
        let mut frame = vec![0u8; leds * 3];
        // 60 %: bright enough to see across a room, not a flash.
        frame[lit * 3..lit * 3 + 3].copy_from_slice(&[153, 153, 153]);
        g.live = true;
        g.hue_lit = false;
        g.last_sent = Some((frame.clone(), now));
        let into = now.saturating_duration_since(started).as_millis() % TEST_STEP.as_millis();
        let next =
            TEST_STEP.saturating_sub(Duration::from_millis(u64::try_from(into).unwrap_or(0)));
        Step {
            frame: Some(frame),
            next: Some(next.max(Duration::from_millis(5))),
        }
    }

    /// Whether the capture socket should exist: a picture is wanted and something can show it.
    pub fn wants_capture(&self) -> bool {
        let g = self.inner.lock();
        let hue = g.hue.is_some();
        (self.target.is_some() || hue)
            && screen::is_on(&self.ctx.hub)
            && g.plan
                .as_ref()
                .is_some_and(|p| p.enabled && self.drives(hue, p) && p.mode.follows_picture())
    }

    /// Starts the thread that drives the strip.
    pub fn spawn(self: &Arc<Self>) -> std::io::Result<JoinHandle<()>> {
        let this = self.clone();
        thread::Builder::new()
            .name("ambient".to_owned())
            .stack_size(512 * 1024)
            .spawn(move || this.run())
    }

    fn run(self: Arc<Self>) {
        let mut sender: Option<ddp::Sender> = None;
        let mut link: Option<Link> = None;
        let mut last_warning = String::new();
        loop {
            if self.ctx.stop.is_set() {
                break;
            }
            let step = self.step(Instant::now());
            if let Some(frame) = &step.frame {
                self.send(&mut sender, frame, &mut last_warning);
            }
            // The capture socket lives exactly as long as a picture is wanted.
            match (self.wants_capture(), link.is_some()) {
                (true, false) => link = Link::start(&self),
                (false, true) => link = None,
                _ => {}
            }
            let mut g = self.inner.lock();
            if g.dirty || self.ctx.stop.is_set() {
                continue;
            }
            match step.next {
                Some(wait) => {
                    let _ = self
                        .wake
                        .wait_for(&mut g, wait.max(Duration::from_millis(1)));
                }
                None => {
                    // Nothing to do until something changes; the screen watcher and the settings wake this, and a minute's
                    // sleep is the backstop (a missed wake never leaves the lights wrong for longer).
                    g.sleeping = true;
                    let _ = self.wake.wait_for(&mut g, Duration::from_secs(60));
                }
            }
        }
        drop(link);
        // The stream to the bridge ends with the helper: black, closed, and the area handed back.
        let output = self.inner.lock().hue.take();
        if let Some(output) = output {
            output.shutdown();
        }
    }

    fn send(&self, sender: &mut Option<ddp::Sender>, frame: &[u8], last_warning: &mut String) {
        let Some(target) = self.target else {
            return;
        };
        if sender.is_none() {
            match ddp::Sender::connect(target, Format::Rgb) {
                Ok(s) => *sender = Some(s),
                Err(e) => return self.send_failed(&e.to_string(), last_warning),
            }
        }
        if let Some(s) = sender.as_mut()
            && let Err(e) = s.send(frame)
        {
            *sender = None;
            return self.send_failed(&e.to_string(), last_warning);
        }
        let mut g = self.inner.lock();
        if g.problem
            .as_deref()
            .is_some_and(|p| p.starts_with("cannot send"))
        {
            g.problem = None;
            last_warning.clear();
            self.publish(&g);
        }
    }

    fn send_failed(&self, why: &str, last_warning: &mut String) {
        let words = format!("cannot send to the strip: {why}");
        if *last_warning != words {
            crate::log_warn!("ambient: {words}");
            *last_warning = words.clone();
        }
        let mut g = self.inner.lock();
        g.problem = Some(words);
        g.retry_at = Some(Instant::now() + RETRY);
        self.publish(&g);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Want {
    Idle,
    Still,
    Picture,
    Test,
}

fn test_length(sections: usize) -> Duration {
    let passes = TEST_STEP * u32::try_from(sections).unwrap_or(u32::MAX) * TEST_PASSES;
    passes.min(TEST_MAX)
}

fn target_of(wled: &WledConfig) -> Option<SocketAddr> {
    let ip: Ipv4Addr = wled.host.parse().ok()?;
    Some(SocketAddr::from((ip, wled.port)))
}

fn settings_of(store: &SettingsStore) -> Settings {
    store
        .effective(OWNER)
        .map_or_else(Settings::default, |v| read_settings(&v))
}

// -- the capture socket ------------------------------------------------------------------------------------------------------

/// The socket the capture helper connects to, and the thread that reads its frames. Dropping it closes the socket and removes the
/// file.
struct Link {
    path: PathBuf,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Link {
    fn start(ambient: &Arc<Ambient>) -> Option<Link> {
        let path = ambient
            .ctx
            .config
            .ambient
            .capture_socket
            .clone()
            .map_or_else(
                || ambient.ctx.paths.data_dir.join("ambient.sock"),
                PathBuf::from,
            );
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                crate::log_warn!("ambient: cannot listen on {}: {e}", path.display());
                return None;
            }
        };
        // Nobody but the helper's own user (root on the TV) talks to it.
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread = {
            let (ambient, stop) = (ambient.clone(), stop.clone());
            thread::Builder::new()
                .name("ambient-capture".to_owned())
                .stack_size(256 * 1024)
                .spawn(move || serve(&ambient, &listener, &stop))
                .ok()
        };
        thread.is_some().then_some(Link { path, stop, thread })
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Waits until `fd` is readable or `ms` pass.
fn readable(fd: std::os::fd::RawFd, ms: i32) -> bool {
    let mut item = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd for a descriptor the caller owns for the whole call.
    unsafe { libc::poll(&mut item, 1, ms) > 0 }
}

fn serve(ambient: &Arc<Ambient>, listener: &UnixListener, stop: &std::sync::atomic::AtomicBool) {
    use std::sync::atomic::Ordering::SeqCst;
    let stopped = |ctx_stop: &Stop| stop.load(SeqCst) || ctx_stop.is_set();
    while !stopped(&ambient.ctx.stop) {
        if !readable(listener.as_raw_fd(), 250) {
            continue;
        }
        let Ok((mut client, _)) = listener.accept() else {
            continue;
        };
        ambient.forget_told();
        let _ = client.set_read_timeout(Some(Duration::from_millis(250)));
        while !stopped(&ambient.ctx.stop) {
            if let Some(index) = ambient.preset_to_tell()
                && client.write_all(&[index]).is_err()
            {
                break;
            }
            match read_frame(&mut client, stop) {
                Arrival::Frame(frame) => ambient.push_frame(*frame, Instant::now()),
                Arrival::Idle => {}
                Arrival::Closed => break,
            }
        }
    }
}

enum Arrival {
    /// A frame of the picture (boxed: it is much larger than the other two).
    Frame(Box<ZoneFrame>),
    /// Nothing arrived within the read timeout.
    Idle,
    /// The capture helper went away, or sent what is not a frame.
    Closed,
}

fn read_frame(client: &mut UnixStream, stop: &std::sync::atomic::AtomicBool) -> Arrival {
    use std::sync::atomic::Ordering::SeqCst;
    let mut header = [0u8; wire::HEADER];
    match read_exact_or_idle(client, &mut header, stop, true) {
        Some(true) => {}
        Some(false) => return Arrival::Idle,
        None => return Arrival::Closed,
    }
    let Some(preset) = Preset::from_index(header[4]) else {
        return Arrival::Closed;
    };
    let mut bytes = vec![0u8; wire::frame_len(preset)];
    bytes[..wire::HEADER].copy_from_slice(&header);
    if stop.load(SeqCst)
        || read_exact_or_idle(client, &mut bytes[wire::HEADER..], stop, false) != Some(true)
    {
        return Arrival::Closed;
    }
    match wire::decode(&bytes) {
        Ok((frame, _)) => Arrival::Frame(Box::new(frame)),
        Err(_) => Arrival::Closed,
    }
}

/// Reads exactly `buf.len()` bytes. `Some(true)`: done. `Some(false)`: the timeout passed with nothing read (only when `idle_ok`, at
/// the start of a frame). `None`: closed, or the frame stopped in the middle.
fn read_exact_or_idle(
    client: &mut UnixStream,
    buf: &mut [u8],
    stop: &std::sync::atomic::AtomicBool,
    idle_ok: bool,
) -> Option<bool> {
    let mut got = 0;
    while got < buf.len() {
        match client.read(&mut buf[got..]) {
            Ok(0) => return None,
            Ok(n) => got += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if got == 0 && idle_ok {
                    return Some(false);
                }
                // A frame that starts must finish within a few timeouts.
                if stop.load(std::sync::atomic::Ordering::SeqCst) {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AmbientConfig, Config, Paths};
    use crate::hub::Hub;
    use lumatv_ambient::zones::MAX_ZONES;
    use settings_schema::SettingChange;
    use std::net::UdpSocket;

    struct Rig {
        ambient: Arc<Ambient>,
        ctx: Ctx,
        _dir: tempfile::TempDir,
    }

    fn rig(strip: Option<SocketAddr>) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(Hub::new(1));
        let settings = Arc::new(SettingsStore::new(dir.path().join("settings"), hub.clone()));
        settings.register_builtin().unwrap();
        let config = Config {
            ambient: AmbientConfig {
                wled: strip.map(|a| WledConfig {
                    host: a.ip().to_string(),
                    port: a.port(),
                }),
                capture_socket: None,
            },
            ..Config::default()
        };
        let ctx = Ctx {
            config: Arc::new(config),
            paths: Paths::new(dir.path()),
            hub: hub.clone(),
            settings,
            stop: Stop::new(),
        };
        hub.set("screen", json!({"on": true, "state": "Active"}));
        Rig {
            ambient: Ambient::new(&ctx),
            ctx,
            _dir: dir,
        }
    }

    fn set(rig: &Rig, changes: Vec<(&str, Value)>) {
        let changes = changes
            .into_iter()
            .map(|(k, v)| SettingChange {
                key: k.parse().unwrap(),
                value: v,
            })
            .collect();
        rig.ctx.settings.apply(OWNER, changes, None).unwrap();
    }

    fn choice(word: &str) -> Value {
        Value::Choice(word.parse().unwrap())
    }

    fn strip() -> SocketAddr {
        ([127, 0, 0, 1], 4048).into()
    }

    /// A frame whose every area is `c`.
    fn solid(c: Rgb) -> ZoneFrame {
        let mut f = ZoneFrame::blank(Preset::Plain);
        for i in 0..MAX_ZONES {
            f.color[i] = c;
            f.count[i] = 1000;
        }
        f
    }

    #[test]
    fn what_the_person_chose_is_read_with_the_schemas_defaults_for_the_rest() {
        let r = rig(None);
        assert_eq!(
            read_settings(&r.ctx.settings.effective(OWNER).unwrap()),
            Settings::default()
        );
        set(
            &r,
            vec![
                ("enabled", Value::Bool(true)),
                ("brightness", Value::Integer(35)),
                ("mode", choice("colour")),
                ("still_colour", choice("iris")),
                ("saturation", Value::Integer(120)),
                ("dark_hold", Value::Bool(false)),
                ("delay_ms", Value::Integer(250)),
                ("top", Value::Integer(20)),
                ("left", Value::Integer(0)),
            ],
        );
        let s = read_settings(&r.ctx.settings.effective(OWNER).unwrap());
        assert!(s.enabled && !s.dark_hold && s.skip_bars);
        assert_eq!((s.brightness, s.saturation, s.delay_ms), (35, 120, 250));
        assert_eq!((s.mode, s.still), (Mode::Colour, Still::Iris));
        assert_eq!(
            (s.edges.top, s.edges.right, s.edges.bottom, s.edges.left),
            (20, 11, 18, 0)
        );
    }

    #[test]
    fn with_no_strip_named_nothing_is_sent_and_the_status_says_why() {
        let r = rig(None);
        set(
            &r,
            vec![("enabled", Value::Bool(true)), ("mode", choice("colour"))],
        );
        let step = r.ambient.step(Instant::now());
        assert_eq!(
            step,
            Step {
                frame: None,
                next: None
            }
        );
        let s = r.ambient.status();
        assert_eq!(s["available"], false);
        assert_eq!(s["state"], "idle");
        assert!(
            s["problem"].as_str().unwrap().contains("ambient.wled.host"),
            "{s}"
        );
        assert_eq!(r.ambient.test(true), Err(TestError::NoStrip));
    }

    #[test]
    fn a_still_colour_is_sent_at_the_chosen_brightness_again_each_second_and_when_it_changes() {
        let r = rig(Some(strip()));
        let t0 = Instant::now();
        // Off: nothing, and the state says so.
        assert_eq!(
            r.ambient.step(t0),
            Step {
                frame: None,
                next: None
            }
        );
        assert_eq!(r.ambient.status()["state"], "off");
        set(
            &r,
            vec![
                ("enabled", Value::Bool(true)),
                ("mode", choice("colour")),
                ("brightness", Value::Integer(50)),
            ],
        );
        let step = r.ambient.step(t0);
        let frame = step.frame.unwrap();
        assert_eq!(frame.len(), 58 * 3, "one pixel per section");
        let warm = Still::Warm.rgb() * 0.5;
        assert_eq!(&frame[..3], &warm.to_u8());
        assert!(frame.chunks(3).all(|p| p == &frame[..3]));
        assert_eq!(step.next, Some(KEEPALIVE));
        assert_eq!(r.ambient.status()["state"], "running");
        // Nothing new in between, the same again after a second.
        let quiet = r.ambient.step(t0 + Duration::from_millis(400));
        assert_eq!(quiet.frame, None);
        assert_eq!(quiet.next, Some(Duration::from_millis(600)));
        let again = r.ambient.step(t0 + Duration::from_millis(1000));
        assert_eq!(again.frame.as_deref(), Some(frame.as_slice()));
        // A change goes out at once.
        set(&r, vec![("still_colour", choice("foam"))]);
        let foam = r
            .ambient
            .step(t0 + Duration::from_millis(1100))
            .frame
            .unwrap();
        assert_eq!(&foam[..3], &(Still::Foam.rgb() * 0.5).to_u8());
        // Switching it off sends black once, then it is quiet.
        set(&r, vec![("enabled", Value::Bool(false))]);
        let off = r.ambient.step(t0 + Duration::from_millis(1200));
        assert_eq!(off.frame, Some(vec![0u8; 58 * 3]));
        assert_eq!(off.next, None);
        assert_eq!(
            r.ambient.step(t0 + Duration::from_millis(1300)),
            Step {
                frame: None,
                next: None
            }
        );
        assert_eq!(r.ambient.status()["state"], "off");
    }

    #[test]
    fn the_screen_going_off_darkens_the_strip_once_and_coming_on_brings_it_back() {
        let r = rig(Some(strip()));
        set(
            &r,
            vec![("enabled", Value::Bool(true)), ("mode", choice("colour"))],
        );
        let t0 = Instant::now();
        assert!(r.ambient.step(t0).frame.is_some());
        r.ctx
            .hub
            .set("screen", json!({"on": false, "state": "Active Standby"}));
        let dark = r.ambient.step(t0 + Duration::from_millis(100));
        assert_eq!(dark.frame, Some(vec![0u8; 58 * 3]));
        assert_eq!(r.ambient.status()["state"], "idle");
        assert_eq!(r.ambient.step(t0 + Duration::from_millis(200)).frame, None);
        r.ctx
            .hub
            .set("screen", json!({"on": true, "state": "Active"}));
        assert!(
            r.ambient
                .step(t0 + Duration::from_millis(300))
                .frame
                .is_some()
        );
    }

    #[test]
    fn the_picture_moves_the_lights_at_thirty_a_second_and_they_fade_when_it_stops() {
        let r = rig(Some(strip()));
        set(
            &r,
            vec![
                ("enabled", Value::Bool(true)),
                ("brightness", Value::Integer(100)),
                ("delay_ms", Value::Integer(0)),
            ],
        );
        let t0 = Instant::now();
        // Enabled, picture mode, no frames yet: waiting, and quiet.
        let first = r.ambient.step(t0);
        assert_eq!(first.frame, None);
        assert_eq!(r.ambient.status()["state"], "waiting");
        // A green picture: after a few ticks the strip is green.
        let mut t = t0;
        let mut last = Vec::new();
        for _ in 0..20 {
            r.ambient.push_frame(solid(Rgb::new(0.1, 0.9, 0.2)), t);
            let step = r.ambient.step(t);
            assert_eq!(step.next, Some(TICK));
            if let Some(f) = step.frame {
                last = f;
            }
            t += TICK;
        }
        assert_eq!(last.len(), 58 * 3);
        let pixel = &last[3 * 20..3 * 20 + 3];
        assert!(
            pixel[1] > 200 && pixel[0] < 60 && pixel[2] < 90,
            "{pixel:?}"
        );
        assert_eq!(r.ambient.status()["state"], "running");
        // No more frames: the lights fade, then one black frame, then it is quiet.
        let mut sent_black = false;
        for _ in 0..80 {
            t += TICK;
            let step = r.ambient.step(t);
            if step.next.is_none() {
                assert!(
                    sent_black
                        || step
                            .frame
                            .as_ref()
                            .is_some_and(|f| f.iter().all(|b| *b == 0)),
                    "{step:?}"
                );
                sent_black = true;
                break;
            }
        }
        assert!(sent_black, "the strip never went quiet");
        assert_eq!(
            r.ambient.step(t + TICK),
            Step {
                frame: None,
                next: None
            }
        );
        assert_eq!(r.ambient.status()["state"], "waiting");
    }

    #[test]
    fn the_mode_that_goes_dark_on_home_does_so_while_something_covers_the_picture() {
        let r = rig(Some(strip()));
        set(
            &r,
            vec![
                ("enabled", Value::Bool(true)),
                ("mode", choice("picture-off-home")),
                ("delay_ms", Value::Integer(0)),
            ],
        );
        let mut t = Instant::now();
        for _ in 0..10 {
            r.ambient.push_frame(solid(Rgb::new(1.0, 1.0, 1.0)), t);
            r.ambient.step(t);
            t += TICK;
        }
        assert_eq!(r.ambient.status()["state"], "running");
        r.ambient.set_context(Some(true), None);
        let mut quiet = false;
        for _ in 0..60 {
            r.ambient.push_frame(solid(Rgb::new(1.0, 1.0, 1.0)), t);
            let step = r.ambient.step(t);
            t += TICK;
            if step.next.is_none() {
                quiet = true;
                break;
            }
        }
        assert!(
            quiet,
            "covered, the lights go out although frames still come"
        );
        assert_eq!(r.ambient.status()["state"], "idle");
        assert_eq!(r.ambient.status()["covered"], true);
        // The plain picture mode keeps following under a menu.
        set(&r, vec![("mode", choice("picture"))]);
        for _ in 0..5 {
            r.ambient.push_frame(solid(Rgb::new(1.0, 1.0, 1.0)), t);
            assert_eq!(r.ambient.step(t).next, Some(TICK));
            t += TICK;
        }
    }

    #[test]
    fn the_test_pattern_lights_each_section_in_turn_and_ends() {
        let r = rig(Some(strip()));
        // The test runs whether or not ambient light is switched on.
        r.ambient.test(true).unwrap();
        let t0 = Instant::now();
        let started = r.ambient.inner.lock().test.as_ref().unwrap().started;
        let lit = |frame: &[u8]| -> Vec<usize> {
            frame
                .chunks(3)
                .enumerate()
                .filter(|(_, p)| p.iter().any(|b| *b != 0))
                .map(|(i, _)| i)
                .collect()
        };
        let first = r.ambient.step(started);
        assert_eq!(lit(first.frame.as_ref().unwrap()), [0]);
        assert_eq!(r.ambient.status()["state"], "testing");
        let second = r.ambient.step(started + TEST_STEP);
        assert_eq!(lit(second.frame.as_ref().unwrap()), [1]);
        let around = r.ambient.step(started + TEST_STEP * 58);
        assert_eq!(lit(around.frame.as_ref().unwrap()), [0], "a second pass");
        // Over: black, and ambient is off again.
        let end = r
            .ambient
            .step(started + test_length(58) + Duration::from_millis(5));
        assert_eq!(end.frame, Some(vec![0u8; 58 * 3]));
        assert_eq!(end.next, None);
        assert_eq!(r.ambient.status()["state"], "off");
        // Stopped by hand.
        r.ambient.test(true).unwrap();
        r.ambient.test(false).unwrap();
        let stopped = r.ambient.step(t0 + Duration::from_secs(1));
        assert!(
            stopped.frame.is_none()
                || stopped
                    .frame
                    .as_ref()
                    .is_some_and(|f| f.iter().all(|b| *b == 0))
        );
    }

    #[test]
    fn a_test_with_no_sections_is_refused_in_words() {
        let r = rig(Some(strip()));
        set(
            &r,
            vec![
                ("top", Value::Integer(0)),
                ("right", Value::Integer(0)),
                ("bottom", Value::Integer(0)),
                ("left", Value::Integer(0)),
            ],
        );
        let e = r.ambient.test(true).unwrap_err();
        assert!(
            matches!(&e, TestError::NoLights(why) if why.contains("no sections")),
            "{e:?}"
        );
        assert!(
            r.ambient.status()["problem"]
                .as_str()
                .unwrap()
                .contains("no sections")
        );
    }

    #[test]
    fn what_is_sent_reaches_the_strip_as_ddp_packets_and_nothing_is_sent_by_this_test_elsewhere() {
        let listener = UdpSocket::bind("127.0.0.1:0").unwrap();
        listener
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let r = rig(Some(addr));
        set(
            &r,
            vec![
                ("enabled", Value::Bool(true)),
                ("mode", choice("colour")),
                ("brightness", Value::Integer(100)),
            ],
        );
        let handle = r.ambient.spawn().unwrap();
        r.ambient.wake();
        let mut buf = [0u8; 2048];
        let n = listener.recv(&mut buf).unwrap();
        let packet = ddp::parse(&buf[..n]).unwrap();
        assert_eq!(packet.data.len(), 58 * 3);
        assert_eq!(&packet.data[..3], &Still::Warm.rgb().to_u8());
        // Off: a black frame follows.
        set(&r, vec![("enabled", Value::Bool(false))]);
        let mut saw_black = false;
        for _ in 0..10 {
            let n = listener.recv(&mut buf).unwrap();
            if ddp::parse(&buf[..n]).unwrap().data.iter().all(|b| *b == 0) {
                saw_black = true;
                break;
            }
        }
        assert!(saw_black);
        r.ctx.stop.request();
        r.ambient.wake();
        handle.join().unwrap();
    }

    #[test]
    fn the_capture_helper_hands_frames_over_a_socket_and_is_told_the_preset() {
        let listener = UdpSocket::bind("127.0.0.1:0").unwrap();
        listener
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let r = rig(Some(addr));
        let socket = r._dir.path().join("capture.sock");
        let config = Config {
            ambient: AmbientConfig {
                wled: Some(WledConfig {
                    host: addr.ip().to_string(),
                    port: addr.port(),
                }),
                capture_socket: Some(socket.display().to_string()),
            },
            ..Config::default()
        };
        let ctx = Ctx {
            config: Arc::new(config),
            ..r.ctx.clone()
        };
        let ambient = Ambient::new(&ctx);
        set(
            &r,
            vec![
                ("enabled", Value::Bool(true)),
                ("brightness", Value::Integer(100)),
                ("delay_ms", Value::Integer(0)),
            ],
        );
        let handle = ambient.spawn().unwrap();
        ambient.wake();
        // The socket exists while a picture is wanted.
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let mut capture = UnixStream::connect(&socket).unwrap();
        capture
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        // It is told the preset first (plain).
        let mut told = [0u8; 1];
        capture.read_exact(&mut told).unwrap();
        assert_eq!(told[0], Preset::Plain.index());
        let bytes = wire::encode(&solid(Rgb::new(0.9, 0.1, 0.1)), 1000);
        let mut buf = [0u8; 2048];
        let mut red = false;
        for _ in 0..40 {
            capture.write_all(&bytes).unwrap();
            thread::sleep(Duration::from_millis(15));
            if let Ok(n) = listener.recv(&mut buf) {
                let data = ddp::parse(&buf[..n]).unwrap().data;
                if data.len() == 58 * 3 && data[0] > 200 && data[1] < 60 {
                    red = true;
                    break;
                }
            }
        }
        assert!(red, "the strip never showed the picture's red");
        // Nobody wants a picture any more: the socket goes.
        set(&r, vec![("enabled", Value::Bool(false))]);
        for _ in 0..100 {
            if !socket.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!socket.exists());
        ctx.stop.request();
        ambient.wake();
        handle.join().unwrap();
    }

    /// The reader's host build (fake libvtcapture) drives the whole chain: reader at its default rate -> socket -> engine -> DDP
    /// packets at a fake WLED socket. Needs `ambient/capture/build.sh` to have run; says so and passes without it.
    #[test]
    fn the_capture_reader_at_its_default_rate_drives_the_strip_end_to_end() {
        let build =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ambient/capture/build/host");
        let bin = build.join("lumatv-capture");
        if !bin.exists() {
            eprintln!(
                "skipped: {} is not built (run ambient/capture/build.sh)",
                bin.display()
            );
            return;
        }
        let wled = UdpSocket::bind("127.0.0.1:0").unwrap();
        wled.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let addr = wled.local_addr().unwrap();
        let r = rig(Some(addr));
        let socket = r._dir.path().join("capture.sock");
        let ctx = Ctx {
            config: Arc::new(Config {
                ambient: AmbientConfig {
                    wled: Some(WledConfig {
                        host: addr.ip().to_string(),
                        port: addr.port(),
                    }),
                    capture_socket: Some(socket.display().to_string()),
                },
                ..Config::default()
            }),
            ..r.ctx.clone()
        };
        let ambient = Ambient::new(&ctx);
        set(
            &r,
            vec![
                ("enabled", Value::Bool(true)),
                ("brightness", Value::Integer(100)),
                ("delay_ms", Value::Integer(0)),
            ],
        );
        let handle = ambient.spawn().unwrap();
        ambient.wake();
        // No -r: the reader's own default is what is under test.
        let log = r._dir.path().join("reader.log");
        let mut reader = std::process::Command::new(&bin)
            .args(["-s", socket.to_str().unwrap(), "-i", "50", "-b", "400"])
            .env("LD_LIBRARY_PATH", &build)
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        let start = Instant::now();
        let mut packets = 0;
        let mut strip = Vec::new();
        let mut buf = [0u8; 2048];
        while start.elapsed() < Duration::from_secs(2) {
            if let Ok(n) = wled.recv(&mut buf) {
                let data = ddp::parse(&buf[..n]).unwrap().data;
                assert_eq!(data.len(), 58 * 3);
                packets += 1;
                strip = data.to_vec();
            }
        }
        // The picture reached the strip: lit, and different along it (the fake gives each area its own colour).
        let leds: Vec<&[u8]> = strip.chunks(3).collect();
        assert!(
            leds.iter().any(|p| p[1] > 30),
            "the strip stayed dark: {strip:?}"
        );
        assert!(
            leds.iter()
                .any(|a| leds.iter().any(|b| a[1].abs_diff(b[1]) > 20)),
            "one colour along the strip"
        );
        // The engine ticks at 30 Hz whatever the reader's rate, so the strip is fed about 30 packets a second from 10 frames.
        assert!(packets >= 25, "{packets} packets in 2 s");
        let _ = reader.kill();
        let _ = reader.wait();
        let said = std::fs::read_to_string(&log).unwrap();
        assert!(said.contains("10 polls a second"), "{said}");
        ctx.stop.request();
        ambient.wake();
        handle.join().unwrap();
    }
}
