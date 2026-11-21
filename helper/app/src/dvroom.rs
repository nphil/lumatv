//! Dolby Vision room mode: while Dolby Vision plays, LG's Cinema for a dark room (`dolbyHdrCinema`) or for a bright one
//! (`dolbyHdrCinemaBright`), picked from the room's light and the time of day (`docs/AGENTS.md`, "Picture modes").
//!
//! - **A session** starts when the picture's dynamic range becomes `dolbyHdr`, and again when the video's app or the input changes
//!   while it is. It ends when Dolby Vision has been gone for [`DROPOUT`]: a seek or a rebuffer that drops it for a moment does not.
//! - **At its start** the room is read [`START_SAMPLES`] times [`START_GAP`] apart and the median decides: at [`DARK_LUX`] or below a
//!   dark room, at [`BRIGHT_LUX`] or above a bright one, between them the time of day (night is dark). Night is the sun below the
//!   horizon at the weather card's place, or 19:00 to 07:00 on the TV's clock when there is no place. The mode is written unless the
//!   one in force is Game or Personalized: those are the owner's on purpose (a console's latency, his own calibration) and the session
//!   leaves them alone. Vivid and Standard give way, as LG's default and its showroom mode are not a choice made for this room.
//! - **During it** the mode moves only when the room's light (the median of [`WINDOW`]) stays past the far side of the band for
//!   [`SUSTAIN`], and never sooner than [`SPACING`] after the last write. Time of day alone never switches a session.
//! - **The owner's hand.** A mode change during the session that is not the echo of this module's own write (the same value within
//!   [`ECHO`] of it) ends the managing until the next session.
//! - **The write** is `pictureMode` alone, under its own dimension (`input`, `dynamicRange: dolbyHdr`), after a read that finds Dolby
//!   Vision still in force with the mode this last saw: SettingsService refuses a key written under another key's dimension, and a
//!   write pinned to Dolby Vision can never land on the SDR or HDR10 picture. Nothing else is ever written: no brightness, and none
//!   of LG's panel protections.
//!
//! The light sensor is read only during a session. The switch is `system.dolbyVisionRoom` of theme.json (on unless `false`). Topic
//! `dvroom`.

use crate::ctx::Ctx;
use crate::paneldim::Settings;
use crate::refreshguard::LunaSource;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const TOPIC: &str = "dvroom";
const SENSOR_URI: &str = "luna://com.webos.service.pqcontroller/getInfo";
const READ_URI: &str = "luna://com.webos.service.settings/getSystemSettings";
const WRITE_URI: &str = "luna://com.webos.service.settings/setSystemSettings";

pub const DARK_MODE: &str = "dolbyHdrCinema";
pub const BRIGHT_MODE: &str = "dolbyHdrCinemaBright";
/// Modes the owner picks on purpose: a session that starts in one leaves it.
const OWNERS: [&str; 2] = ["dolbyHdrGame", "dolbyHdrPersonalized"];

/// At or below this the room is dark. The set's sensor reads 1 to 3 lux in the dark room (`autobright`'s measurements), the
/// screen's own light adds blips of 4 to 5, so above 3 the room is no longer surely dark.
pub const DARK_LUX: f64 = 3.0;
/// At or above this the room is bright: past the lamp-lit evening (6 to 8 lux) and the curtained afternoon (5, read on 2026-11-21),
/// so only daylight or the room's main lights count; between the two the time of day decides.
pub const BRIGHT_LUX: f64 = 10.0;
/// Dolby Vision gone this long ends the session.
pub const DROPOUT: Duration = Duration::from_secs(10);
/// The room past the band this long switches a session.
pub const SUSTAIN: Duration = Duration::from_secs(2 * 60);
/// At least this long between two writes of a session.
pub const SPACING: Duration = Duration::from_secs(15 * 60);
/// The room's light during a session is the median of this window, so one blip (a hand, the screen's flash) moves nothing.
pub const WINDOW: Duration = Duration::from_secs(30);
/// The readings a session starts from, and their spacing: the decision lands about 2 s into the video, under its opening.
pub const START_SAMPLES: usize = 3;
pub const START_GAP: Duration = Duration::from_secs(1);
/// The sensor's reading interval after the start.
pub const SAMPLE: Duration = Duration::from_secs(5);
/// A mode seen this soon after this module wrote it is the write's echo, not the owner.
pub const ECHO: Duration = Duration::from_secs(10);
/// Night on the TV's clock when the weather card has no place: before 07:00 and from 19:00.
const MORNING_MIN: u32 = 7 * 60;
const EVENING_MIN: u32 = 19 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Room {
    Dark,
    Bright,
}

impl Room {
    pub fn mode(self) -> &'static str {
        match self {
            Room::Dark => DARK_MODE,
            Room::Bright => BRIGHT_MODE,
        }
    }

    fn of(mode: &str) -> Option<Room> {
        match mode {
            DARK_MODE => Some(Room::Dark),
            BRIGHT_MODE => Some(Room::Bright),
            _ => None,
        }
    }

    fn other(self) -> Room {
        match self {
            Room::Dark => Room::Bright,
            Room::Bright => Room::Dark,
        }
    }

    /// The room for a level of light, `None` inside the band.
    pub fn classify(lux: f64) -> Option<Room> {
        if lux <= DARK_LUX {
            Some(Room::Dark)
        } else if lux >= BRIGHT_LUX {
            Some(Room::Bright)
        } else {
            None
        }
    }
}

/// What the rest of the helper says about now.
#[derive(Clone, Debug, Default)]
pub struct Seen {
    /// The owner's switch.
    pub on: bool,
    /// The picture's dynamic range is Dolby Vision.
    pub dv: bool,
    /// The picture mode in force.
    pub mode: String,
    pub input: String,
    /// The app of the video on the main sink; empty while none is reported.
    pub app: String,
    /// A game in front: nothing is written.
    pub game: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Reading the room before the first choice.
    Starting,
    /// Managing.
    Auto,
    /// The owner changed the mode during the session.
    Hand,
    /// The session started in a mode the owner picks on purpose.
    Owner,
    /// The TV did not take a write.
    Refused,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            State::Starting => "starting",
            State::Auto => "auto",
            State::Hand => "hand",
            State::Owner => "owner",
            State::Refused => "refused",
        }
    }
}

#[derive(Debug)]
struct Session {
    app: String,
    input: String,
    state: State,
    /// The mode last seen in force under Dolby Vision.
    mode: String,
    samples: VecDeque<(Instant, f64)>,
    next_sample: Instant,
    /// Since when Dolby Vision has been gone.
    gone: Option<Instant>,
    /// Since when the room has stayed past the band.
    beyond: Option<Instant>,
    /// This module's last write: the mode and when.
    wrote: Option<(&'static str, Instant)>,
}

impl Session {
    fn median(&self, since: Option<Instant>) -> Option<f64> {
        let mut v: Vec<f64> = self.samples.iter().filter(|(t, _)| since.is_none_or(|s| *t >= s)).map(|(_, l)| *l).collect();
        if v.is_empty() {
            return None;
        }
        v.sort_by(f64::total_cmp);
        Some(v[v.len() / 2])
    }
}

/// The decision, without the TV: [`Engine::step`] says what to write, [`Engine::wrote`] and [`Engine::refused`] what came of it.
#[derive(Default)]
pub struct Engine {
    session: Option<Session>,
    error: Option<String>,
}

impl Engine {
    /// Whether the sensor should be read now.
    pub fn sample_due(&self, now: Instant) -> bool {
        self.session.as_ref().is_some_and(|s| s.gone.is_none() && matches!(s.state, State::Starting | State::Auto) && now >= s.next_sample)
    }

    pub fn sample(&mut self, now: Instant, lux: f64) {
        let Some(s) = &mut self.session else { return };
        s.samples.push_back((now, lux));
        let starting = s.state == State::Starting;
        // a start keeps all its readings; after it, the window
        while !starting && s.samples.front().is_some_and(|(t, _)| now.saturating_duration_since(*t) > WINDOW) {
            s.samples.pop_front();
        }
        s.next_sample = now + if starting && s.samples.len() < START_SAMPLES { START_GAP } else { SAMPLE };
    }

    /// The mode to write now, if any. `night` is asked only when a session makes its first choice.
    pub fn step(&mut self, now: Instant, seen: &Seen, night: &dyn Fn() -> bool) -> Option<&'static str> {
        self.observe(now, seen);
        let s = self.session.as_mut()?;
        if !seen.dv || seen.game {
            return None;
        }
        if s.wrote.is_some_and(|(mode, at)| mode != s.mode && now.saturating_duration_since(at) > ECHO) && s.state == State::Auto {
            crate::log_warn!("dv room: {} was written but not taken; left alone until the next video", s.wrote.map_or("", |w| w.0));
            s.state = State::Refused;
            return None;
        }
        match s.state {
            State::Starting => {
                if s.samples.len() < START_SAMPLES {
                    return None;
                }
                let lux = s.median(None)?;
                let room = Room::classify(lux).unwrap_or_else(|| if night() { Room::Dark } else { Room::Bright });
                s.state = State::Auto;
                s.samples.retain(|(t, _)| now.saturating_duration_since(*t) <= WINDOW);
                crate::log_info!("dv room: {} at {lux:.0} lux, {} in force", room.mode(), s.mode);
                (s.mode != room.mode()).then_some(room.mode())
            }
            State::Auto => {
                let current = Room::of(&s.mode)?;
                let lux = s.median(now.checked_sub(WINDOW))?;
                let beyond = match current {
                    Room::Dark => lux >= BRIGHT_LUX,
                    Room::Bright => lux <= DARK_LUX,
                };
                if !beyond {
                    s.beyond = None;
                    return None;
                }
                let since = *s.beyond.get_or_insert(now);
                let spaced = s.wrote.is_none_or(|(_, at)| now.saturating_duration_since(at) >= SPACING);
                (now.saturating_duration_since(since) >= SUSTAIN && spaced).then(|| current.other().mode())
            }
            State::Hand | State::Owner | State::Refused => None,
        }
    }

    fn observe(&mut self, now: Instant, seen: &Seen) {
        if !seen.on {
            self.session = None;
            return;
        }
        if !seen.dv {
            if let Some(s) = &mut self.session
                && now.saturating_duration_since(*s.gone.get_or_insert(now)) >= DROPOUT
            {
                self.session = None;
            }
            return;
        }
        if let Some(s) = &self.session {
            let app_changed = !seen.app.is_empty() && !s.app.is_empty() && seen.app != s.app;
            if app_changed || seen.input != s.input {
                self.session = None;
            }
        }
        let Some(s) = &mut self.session else {
            let state = if OWNERS.contains(&seen.mode.as_str()) { State::Owner } else { State::Starting };
            crate::log_info!("dv room: Dolby Vision in {} on {} ({}): {}", seen.mode, seen.input, seen.app, state.name());
            self.session = Some(Session {
                app: seen.app.clone(),
                input: seen.input.clone(),
                state,
                mode: seen.mode.clone(),
                samples: VecDeque::new(),
                next_sample: now,
                gone: None,
                beyond: None,
                wrote: None,
            });
            self.error = None;
            return;
        };
        s.gone = None;
        if s.app.is_empty() {
            s.app.clone_from(&seen.app);
        }
        if seen.mode != s.mode {
            let echo = s.wrote.is_some_and(|(mode, at)| mode == seen.mode && now.saturating_duration_since(at) <= ECHO);
            if !echo && matches!(s.state, State::Starting | State::Auto) {
                crate::log_info!("dv room: {} picked by hand; left alone until the next video", seen.mode);
                s.state = State::Hand;
            }
            s.mode.clone_from(&seen.mode);
        }
    }

    /// The TV took `mode`.
    pub fn wrote(&mut self, now: Instant, mode: &'static str) {
        if let Some(s) = &mut self.session {
            s.wrote = Some((mode, now));
            s.beyond = None;
        }
    }

    /// The TV refused the write, or the read before it found the picture changed: nothing more this session.
    pub fn refused(&mut self, e: String) {
        crate::log_warn!("dv room: {e}; left alone until the next video");
        if let Some(s) = &mut self.session {
            s.state = State::Refused;
        }
        self.error = Some(e);
    }

    pub fn status(&self, on: bool) -> Value {
        let session = self.session.as_ref().map(|s| {
            json!({
                "app": s.app,
                "state": s.state.name(),
                "mode": s.mode,
                "auto": s.state == State::Auto && Room::of(&s.mode).is_some(),
                "lux": s.samples.back().map(|(_, l)| *l),
            })
        });
        json!({"on": on, "session": session, "error": self.error})
    }
}

/// Reads the picture in force and writes `mode` under Dolby Vision's own dimension when it is still Dolby Vision in `expected`.
fn write(tv: &dyn Settings, input: &str, expected: &str, mode: &str) -> Result<(), String> {
    let r = tv.call(READ_URI, &json!({"category": "picture", "keys": ["pictureMode"]}))?;
    let ctx = crate::picture::Context::parse(&r).ok_or_else(|| format!("no picture mode in {r}"))?;
    if !ctx.range.eq_ignore_ascii_case("dolbyHdr") || ctx.input != input || ctx.mode != expected {
        return Err(format!("the picture changed to {} {} {} before the write", ctx.input, ctx.range, ctx.mode));
    }
    let payload = json!({"category": "picture", "settings": {"pictureMode": mode}, "dimension": {"input": input, "dynamicRange": "dolbyHdr"}});
    let r = tv.call(WRITE_URI, &payload)?;
    if r.get("returnValue").and_then(Value::as_bool) == Some(true) { Ok(()) } else { Err(format!("the TV refused {mode}: {r}")) }
}

fn read_lux(tv: &dyn Settings) -> Result<f64, String> {
    let r = tv.call(SENSOR_URI, &json!({"key": "eyeQLux"}))?;
    r.get("eyeQLux").and_then(Value::as_f64).filter(|l| l.is_finite() && *l >= 0.0).ok_or_else(|| format!("no reading: {r}"))
}

/// The owner's switch: `system.dolbyVisionRoom` of theme.json, on unless it says `false`.
fn switch_on(theme: &Path) -> bool {
    std::fs::read(theme)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.pointer("/system/dolbyVisionRoom").and_then(Value::as_bool))
        != Some(false)
}

/// Night: the sun below the horizon at the weather card's place, else the TV's clock outside 07:00 to 19:00.
fn night(hub: &crate::hub::Hub) -> bool {
    let s = i64::try_from(crate::timeutil::now_ms() / 1000).unwrap_or_default();
    let place = hub.get(crate::homefeeds::WEATHER_TOPIC).and_then(|w| Some((w.pointer("/doc/lat")?.as_f64()?, w.pointer("/doc/lon")?.as_f64()?)));
    if let Some(home) = place.filter(|p| *p != (0.0, 0.0)) {
        return !crate::ha_weather::sun_up(s, home);
    }
    let minute = crate::dnd::utc_offset_s(s).map_or(12 * 60, |off| u32::try_from((s + off).rem_euclid(86_400) / 60).unwrap_or_default());
    !(MORNING_MIN..EVENING_MIN).contains(&minute)
}

fn seen(ctx: &Ctx, on: bool) -> Seen {
    let picture = ctx.hub.get(crate::picture::TOPIC).unwrap_or(Value::Null);
    let s = |p: &str| picture.pointer(p).and_then(Value::as_str).unwrap_or("").to_owned();
    Seen {
        on: on && crate::screen::is_on(&ctx.hub),
        dv: s("/context/range").eq_ignore_ascii_case("dolbyHdr"),
        mode: s("/context/mode"),
        input: s("/context/input"),
        app: s("/playing/app"),
        game: crate::picture::game_in_front(&ctx.hub),
    }
}

pub fn spawn(ctx: &Ctx) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new().name("dvroom".to_owned()).stack_size(256 * 1024).spawn(move || run(&ctx))
}

fn run(ctx: &Ctx) {
    let (tx, rx) = mpsc::channel::<()>();
    ctx.stop.on_stop(move || {
        let _ = tx.send(());
    });
    let tv = LunaSource::new(&ctx.config.luna.send);
    let theme = ctx.paths.theme_file();
    let mut engine = Engine::default();
    loop {
        let now = Instant::now();
        let on = switch_on(&theme);
        let seen = seen(ctx, on);
        if engine.sample_due(now) {
            match read_lux(&tv) {
                Ok(lux) => engine.sample(now, lux),
                Err(e) => crate::log_warn!("dv room: light sensor: {e}"),
            }
        }
        if let Some(mode) = engine.step(now, &seen, &|| night(&ctx.hub)) {
            match write(&tv, &seen.input, &seen.mode, mode) {
                Ok(()) => engine.wrote(Instant::now(), mode),
                Err(e) => engine.refused(e),
            }
        }
        ctx.hub.set(TOPIC, engine.status(on));
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
mod tests;
