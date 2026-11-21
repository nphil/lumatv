//! Picture presets and two remembered SDR picture modes: one for LumaTV and the apps' menus, one for videos (`docs/AGENTS.md`,
//! "Picture modes").
//!
//! LG keeps one picture mode per input and dynamic range (`pictureMode`'s dimension is `[input, dynamicRange]`), and LumaTV, Home
//! and every app share the input `default`. So the colourful shell and a film in Marquee or Netflix get the same mode unless
//! something switches it. This module does: SDR video playing full screen in an app for [`PLAY`] switches to the `video` slot's
//! mode, and [`STOP`] without it switches back to the `ui` slot's. HDMI inputs and Live TV have inputs of their own and keep LG's
//! memory; HDR and Dolby Vision have their own range and are LG's; nothing is switched while a game is in front or LG's Game mode
//! is in force. A mode the owner picks while a slot is in force (LG's menu, Quick Settings, Settings) becomes that slot's.
//!
//! A preset is a mode and the keys LumaTV writes into it ([`PRESETS`]); each preset has a mode of its own, so a switch is one
//! `pictureMode` write and keys the owner changes later stay. Every value a preset replaces is journalled in `picture.json` of
//! the data directory before the write and put back exactly by `restore`. White balance and colour management are never written.
//!
//! The switch is `system.separatePicture` of theme.json (on unless `false`); off puts the `video` slot's mode back as the one mode.
//! Topic `picture`, routes `GET /v1/picture`, `POST /v1/picture/{preset,slot,restore}`.

use crate::ctx::Ctx;
use crate::paneldim::Settings;
use crate::refreshguard::LunaSource;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const TOPIC: &str = "picture";
/// The slots and the journal, in the data directory.
pub const STATE: &str = "picture.json";
const READ_URI: &str = "luna://com.webos.service.settings/getSystemSettings";
const WRITE_URI: &str = "luna://com.webos.service.settings/setSystemSettings";
const PICTURE_PAYLOAD: &str = r#"{"category":"picture","keys":["pictureMode"],"subscribe":true}"#;
const VIDEO_URI: &str = "luna://com.webos.service.videooutput/getStatus";
const VIDEO_PAYLOAD: &str = r#"{"subscribe":true}"#;
/// The input LG files LumaTV's and every app's picture under.
pub const APPS_INPUT: &str = "default";
/// LG's Game mode (Game Optimizer, ALLM): its low-latency processing is left alone.
const LG_GAME: &str = "game";

/// Playback this long switches to the video slot: a preview or a trailer shorter than this changes nothing.
pub const PLAY: Duration = Duration::from_secs(2);
/// No playback this long switches back: a seek, a rebuffer or the gap to the next episode does not.
pub const STOP: Duration = Duration::from_secs(3);
/// After a switch, a mode change is LG's own (Filmmaker auto start acts as a film starts), not the owner's choice.
pub const SETTLE: Duration = Duration::from_secs(5);
/// How long a mode write has to show in the subscription before it counts as not taken.
const ECHO: Duration = Duration::from_secs(4);
/// At least this long between mode writes, and at most [`PER_MINUTE`] in a minute.
const MIN_GAP: Duration = Duration::from_secs(2);
const PER_MINUTE: usize = 6;
/// Three writes that did not take stop the switching this long.
const BACKOFF: Duration = Duration::from_secs(60);
/// How often the switch and the game state are read while nothing else happens.
const POLL: Duration = Duration::from_secs(5);
/// LG's SDR modes a slot may hold. Game is LG's low-latency mode for consoles and is not offered.
pub const SDR_MODES: [&str; 10] = ["filmMaker", "cinema", "normal", "vivid", "sports", "eco", "expert1", "expert2", "personalized", LG_GAME];

/// A mode and the keys LumaTV writes into it. Values are strings, as the service takes them.
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub mode: &'static str,
    pub settings: &'static [(&'static str, &'static str)],
}

/// RTINGS' SDR settings for the LG C4 (rtings.com/tv/reviews/lg/c4-oled/settings): contrast 85, black level 50, gamma 2.2, Auto
/// Dynamic Contrast off, colour 50, tint 0, gamut Auto Detect, Warm 50, sharpness 0, the four clean-up filters off.
/// `colorTemperature` -50 is Warm 50: this set's Filmmaker and Expert modes ship at -50, Vivid at +50. RTINGS' Peak Brightness
/// High is left out: nothing LumaTV writes raises the panel's light output above the owner's Filmmaker (peak off, 75).
macro_rules! rtings {
    ($gamut:literal $(, $extra:expr)*) => {
        &[
            ("contrast", "85"),
            ("brightness", "50"),
            ("peakBrightness", "off"),
            ("gamma", "medium"),
            ("dynamicContrast", "off"),
            ("color", "50"),
            ("tint", "0"),
            ("colorGamut", $gamut),
            ("colorTemperature", "-50"),
            ("sharpness", "0"),
            ("superResolution", "off"),
            ("noiseReduction", "off"),
            ("mpegNoiseReduction", "off"),
            ("smoothGradation", "off"),
            $($extra,)*
        ]
    };
}

pub const PRESETS: [Preset; 3] = [
    // RTINGS calibrates in Expert (Dark Space, night): LG's `expert2`, whose own OLED pixel brightness is 60.
    Preset { id: "rtings", name: "RTINGS accurate", mode: "expert2", settings: rtings!("auto") },
    // RTINGS' alternative for saturated colour, gamut Native; in Expert (Bright Space) so it keeps a mode of its own, at Expert
    // (Dark Space)'s 60 rather than Bright Space's own 100.
    Preset { id: "rtingsRich", name: "RTINGS richer colour", mode: "expert1", settings: rtings!("native", ("backlight", "60")) },
    // For the shell's glass themes (`rustui/tokens/src/palettes.json`), on the set's most static picture, so the pop comes from
    // colour and not from light: OLED pixel brightness 75 with Energy saving Auto, contrast 85, peak brightness off and dynamic
    // contrast off are the owner's Filmmaker values. It lives in LG's Eco mode, the one SDR mode besides Filmmaker that has Energy
    // saving on this set. Dark canvases are 0.3-2.1% luminance with glass tiers 0.2-1% apart: gamma 2.2 rather than BT.1886 and
    // black level 50 keep the tiers apart. Pastel dark-theme accents (HSV saturation 0.44-0.67) gain from the panel's Native gamut
    // and colour 60; light-theme accents already at saturation 1.0 are not pushed further (Vivid's 70 and Dynamic colour would
    // clip them). Warm 20 rather than Warm 50 makes the white edge lights (12% and 20% alpha) read crisp on the cool palettes
    // (glacier, ocean, slate) without tinting the warm ones. Smooth gradation low against banding in the wallpaper gradients (1%
    // to 40% in a ramp); the noise, MPEG and super-resolution filters off, which soften text and glass.
    Preset {
        id: "lumaVivid",
        name: "LumaTV vivid",
        mode: "eco",
        settings: &[
            ("backlight", "75"),
            ("energySaving", "auto"),
            ("contrast", "85"),
            ("brightness", "50"),
            ("peakBrightness", "off"),
            ("gamma", "medium"),
            ("dynamicContrast", "off"),
            ("dynamicColor", "off"),
            ("color", "60"),
            ("tint", "0"),
            ("colorGamut", "native"),
            ("colorTemperature", "-20"),
            ("sharpness", "10"),
            ("superResolution", "off"),
            ("noiseReduction", "off"),
            ("mpegNoiseReduction", "off"),
            ("smoothGradation", "low"),
        ],
    },
];

/// The ui slot's mode before the owner has chosen one.
pub const DEFAULT_UI: &str = "lumaVivid";

pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Ui,
    Video,
}

impl Slot {
    pub fn name(self) -> &'static str {
        match self {
            Slot::Ui => "ui",
            Slot::Video => "video",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ui" => Some(Slot::Ui),
            "video" => Some(Slot::Video),
            _ => None,
        }
    }
}

/// What `picture.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Saved {
    pub ui: Option<String>,
    pub video: Option<String>,
    /// Mode, key, and the value LumaTV replaced: the first one, kept until it is put back.
    pub journal: BTreeMap<String, BTreeMap<String, Value>>,
    /// The preset last written into a mode.
    pub applied: BTreeMap<String, String>,
}

impl Saved {
    fn slot(&self, slot: Slot) -> Option<&String> {
        match slot {
            Slot::Ui => self.ui.as_ref(),
            Slot::Video => self.video.as_ref(),
        }
    }

    fn set(&mut self, slot: Slot, mode: &str) {
        let s = Some(mode.to_owned());
        match slot {
            Slot::Ui => self.ui = s,
            Slot::Video => self.video = s,
        }
    }
}

pub fn load(path: &Path) -> Saved {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            crate::log_warn!("picture: {} unreadable ({e}); starting without slots", path.display());
            Saved::default()
        }),
        Err(_) => Saved::default(),
    }
}

/// The picture in force: the mode, and the input and range LG filed it under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub mode: String,
    pub input: String,
    pub range: String,
}

impl Context {
    /// A `getSystemSettings` answer for `pictureMode` (a reply of the subscription, or a read without a dimension).
    pub fn parse(reply: &Value) -> Option<Self> {
        let mode = reply.pointer("/settings/pictureMode")?.as_str().filter(|m| !m.is_empty())?;
        let d = reply.get("dimension");
        let s = |k: &str| d.and_then(|d| d.get(k)).and_then(Value::as_str).unwrap_or("").to_owned();
        Some(Self { mode: mode.to_owned(), input: s("input"), range: s("dynamicRange") })
    }

    /// The apps' input in SDR: the one place the slots apply.
    fn ours(&self) -> bool {
        self.input == APPS_INPUT && self.range.eq_ignore_ascii_case("sdr")
    }
}

/// The video on the main sink, as videooutputd reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Video {
    pub app: String,
    /// `videoInfo.hdrType`: `none` for SDR.
    pub hdr: String,
    pub full: bool,
}

impl Video {
    /// `getStatus`: `Some(None)` when nothing plays on the main sink, `None` for an answer without the list.
    pub fn parse(reply: &Value) -> Option<Option<Self>> {
        let list = reply.get("video")?.as_array()?;
        Some(list.iter().find_map(|v| {
            let connected = v.get("connected").and_then(Value::as_bool) == Some(true);
            let muted = v.get("muted").and_then(Value::as_bool) == Some(true);
            if v.get("sink").and_then(Value::as_str) != Some("MAIN") || !connected || muted {
                return None;
            }
            let out = v.get("displayOutput");
            let n = |k: &str| out.and_then(|o| o.get(k)).and_then(Value::as_u64).unwrap_or(0);
            // the panel is 3840 by 2160; a window of 90% of it counts as full screen
            let full = v.get("fullScreen").and_then(Value::as_bool) == Some(true) || n("width") * n("height") >= 3840 * 2160 * 9 / 10;
            Some(Self {
                app: v.get("appId").and_then(Value::as_str).unwrap_or("").to_owned(),
                hdr: v.pointer("/videoInfo/hdrType").and_then(Value::as_str).unwrap_or("none").to_owned(),
                full,
            })
        }))
    }
}

/// Requests from the API.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// Write a preset into its mode and make it the choice of the slot in force.
    Preset(String),
    /// A slot's choice: an LG mode or a preset.
    Slot(Slot, String),
    /// Put back what presets replaced in a mode.
    Restore(String),
}

pub struct Engine {
    tv: Box<dyn Settings + Send>,
    saved: Saved,
    path: Option<PathBuf>,
    /// LumaTV's own app ids besides `org.nphil.`: their video (cameras, Home's wallpaper) is the shell's.
    ours: Vec<String>,
    ctx: Option<Context>,
    playing: Option<Video>,
    /// Whether an SDR video of an app plays full screen, and since when that has been so; `None` before videooutputd answered.
    eligible: Option<(bool, Instant)>,
    active: Option<Slot>,
    /// A mode write waiting to show in the subscription.
    pending: Option<(String, Instant)>,
    settle_until: Option<Instant>,
    writes: VecDeque<Instant>,
    misses: u32,
    hold_until: Option<Instant>,
    separate: bool,
    game: bool,
    error: Option<String>,
}

impl Engine {
    pub fn new(tv: Box<dyn Settings + Send>, saved: Saved, path: Option<PathBuf>, ours: Vec<String>, separate: bool) -> Self {
        Self {
            tv,
            saved,
            path,
            ours,
            ctx: None,
            playing: None,
            eligible: None,
            active: None,
            pending: None,
            settle_until: None,
            writes: VecDeque::new(),
            misses: 0,
            hold_until: None,
            separate,
            game: false,
            error: None,
        }
    }

    pub fn saved(&self) -> &Saved {
        &self.saved
    }

    pub fn active(&self) -> Option<Slot> {
        self.active
    }

    fn persist(&self) {
        let Some(path) = &self.path else { return };
        let tmp = path.with_extension("json.tmp");
        let write = || -> std::io::Result<()> {
            std::fs::write(&tmp, serde_json::to_vec_pretty(&self.saved).map_err(std::io::Error::other)?)?;
            std::fs::rename(&tmp, path)
        };
        if let Err(e) = write() {
            crate::log_warn!("picture: saving {}: {e}", path.display());
        }
    }

    fn is_ours(&self, app: &str) -> bool {
        app.is_empty() || app.starts_with("org.nphil.") || app == "com.webos.app.home" || self.ours.iter().any(|o| o == app)
    }

    /// The picture subscription's answer, or a read's.
    pub fn picture(&mut self, now: Instant, ctx: Context) {
        let prev = self.ctx.replace(ctx.clone());
        if self.pending.as_ref().is_some_and(|(m, _)| *m == ctx.mode) {
            self.pending = None;
            self.misses = 0;
            return;
        }
        if self.saved.video.is_none() && ctx.ours() && ctx.mode != LG_GAME {
            // the mode in force when this first runs is the owner's choice for videos
            self.saved.video = Some(ctx.mode.clone());
            self.persist();
        }
        let same_place = prev.as_ref().is_some_and(|p| p.input == ctx.input && p.range == ctx.range && p.mode != ctx.mode);
        if same_place && ctx.ours() && ctx.mode != LG_GAME && self.learning(now) {
            let slot = if self.separate { self.active.unwrap_or(Slot::Ui) } else { Slot::Video };
            crate::log_info!("picture: {} chosen for the {} slot", ctx.mode, slot.name());
            self.saved.set(slot, &ctx.mode);
            self.persist();
        }
    }

    fn learning(&self, now: Instant) -> bool {
        !self.game && self.pending.is_none() && self.settle_until.is_none_or(|t| now >= t) && (self.active.is_some() || !self.separate)
    }

    /// videooutputd's main sink: what plays there, if anything.
    pub fn video(&mut self, now: Instant, video: Option<Video>) {
        let eligible = video.as_ref().is_some_and(|v| v.full && v.hdr.eq_ignore_ascii_case("none") && !self.is_ours(&v.app) && !crate::app_priority::game::GAMES.contains(&v.app.as_str()));
        self.playing = video;
        match self.eligible {
            None => {
                // what plays as the helper starts is not a change: its slot is in force at once
                self.eligible = Some((eligible, now));
                self.active = Some(if eligible { Slot::Video } else { Slot::Ui });
            }
            Some((was, _)) if was != eligible => {
                self.eligible = Some((eligible, now));
                // Filmmaker Auto Start switches as a film starts or ends: not the owner's choice
                self.settle_until = Some(now + SETTLE);
            }
            Some(_) => {}
        }
    }

    pub fn set_game(&mut self, game: bool) {
        self.game = game;
    }

    /// The owner's switch. Turning it off puts the video slot's mode back as the one mode.
    pub fn set_separate(&mut self, now: Instant, on: bool) {
        if self.separate == on {
            return;
        }
        self.separate = on;
        crate::log_info!("picture: separate modes {}", if on { "on" } else { "off" });
        if !on && let Some(mode) = self.saved.video.clone() {
            self.go(now, &mode, true);
        }
    }

    fn want(&self, now: Instant) -> Option<Slot> {
        let (eligible, since) = self.eligible?;
        let slot = if eligible { Slot::Video } else { Slot::Ui };
        let wait = if eligible { PLAY } else { STOP };
        Some(if now.saturating_duration_since(since) >= wait { slot } else { self.active.unwrap_or(slot) })
    }

    pub fn tick(&mut self, now: Instant) {
        if let Some((_, at)) = &self.pending
            && now.saturating_duration_since(*at) >= ECHO
        {
            self.pending = None;
            self.misses += 1;
            if self.misses >= 3 {
                self.misses = 0;
                self.hold_until = Some(now + BACKOFF);
                self.error = Some("The TV did not take the picture mode three times".into());
                crate::log_warn!("picture: three mode writes did not take; waiting {} s", BACKOFF.as_secs());
            }
        }
        if let Some(want) = self.want(now)
            && self.active != Some(want)
        {
            self.active = Some(want);
            self.settle_until = Some(now + SETTLE);
        }
        if !self.separate || self.hold_until.is_some_and(|t| now < t) || self.pending.is_some() {
            return;
        }
        let Some(slot) = self.active else { return };
        if self.saved.slot(slot).is_none() && slot == Slot::Ui && self.ctx.as_ref().is_some_and(Context::ours) && !self.game {
            match self.write_preset(DEFAULT_UI) {
                Ok(mode) => {
                    self.saved.ui = Some(mode.to_owned());
                    self.persist();
                }
                Err(e) => {
                    self.fail(now, e);
                    return;
                }
            }
        }
        if let Some(mode) = self.saved.slot(slot).cloned() {
            self.go(now, &mode, false);
        }
    }

    fn fail(&mut self, now: Instant, e: String) {
        crate::log_warn!("picture: {e}");
        self.error = Some(e);
        self.hold_until = Some(now + BACKOFF);
    }

    /// Why nothing is switched now, if so.
    fn hold(&self) -> Option<&'static str> {
        if self.separate { self.blocked() } else { Some("off") }
    }

    /// A game, or a picture that is LG's: another input, HDR or Dolby Vision, or LG's Game mode.
    fn blocked(&self) -> Option<&'static str> {
        if self.game {
            return Some("game");
        }
        let ctx = self.ctx.as_ref()?;
        if ctx.input != APPS_INPUT {
            Some("input")
        } else if !ctx.range.eq_ignore_ascii_case("sdr") {
            Some("hdr")
        } else if ctx.mode == LG_GAME {
            Some("lgGame")
        } else {
            None
        }
    }

    fn rate_ok(&mut self, now: Instant) -> bool {
        while self.writes.front().is_some_and(|t| now.saturating_duration_since(*t) >= Duration::from_secs(60)) {
            self.writes.pop_front();
        }
        self.writes.len() < PER_MINUTE && self.writes.back().is_none_or(|t| now.saturating_duration_since(*t) >= MIN_GAP)
    }

    /// Switches to `mode` when the apps' SDR picture is in force and it is another; `asked` is the owner's request (no rate limit).
    fn go(&mut self, now: Instant, mode: &str, asked: bool) {
        if self.blocked().is_some() || self.ctx.as_ref().is_none_or(|c| c.mode == mode) {
            return;
        }
        if !asked && !self.rate_ok(now) {
            return;
        }
        // read the picture in force first: the input or range may have changed since the last answer, and an unpinned write lands
        // wherever the TV is now
        match self.tv.call(READ_URI, &json!({"category": "picture", "keys": ["pictureMode"]})).ok().as_ref().and_then(Context::parse) {
            Some(fresh) => {
                let stop = !fresh.ours() || fresh.mode == mode || fresh.mode == LG_GAME;
                self.ctx = Some(fresh);
                if stop {
                    return;
                }
            }
            None => return self.fail(now, "the TV did not say which picture mode is in force".into()),
        }
        // unpinned, as Quick Settings writes it: a write carrying the old mode's dimension left the TV on the old mode (r19w)
        let write = json!({"category": "picture", "settings": {"pictureMode": mode}});
        self.writes.push_back(now);
        match self.tv.call(WRITE_URI, &write) {
            Ok(r) if r.get("returnValue").and_then(Value::as_bool) != Some(false) => {
                crate::log_info!("picture: {mode} for the {} slot", self.active.map_or("video", Slot::name));
                self.pending = Some((mode.to_owned(), now));
                self.error = None;
            }
            Ok(r) => self.fail(now, format!("the TV refused {mode}: {}", r.get("errorText").and_then(Value::as_str).unwrap_or("refused"))),
            Err(e) => self.fail(now, e),
        }
    }

    /// Writes a preset's keys into its mode, journalling each replaced value first. The mode is the preset's.
    fn write_preset(&mut self, id: &str) -> Result<&'static str, String> {
        let p = preset(id).ok_or_else(|| format!("no preset {id}"))?;
        let dimension = json!({"pictureMode": p.mode, "input": APPS_INPUT});
        let keys: Vec<&str> = p.settings.iter().map(|(k, _)| *k).collect();
        let read = json!({"category": "picture", "keys": keys, "dimension": dimension});
        let now = self.tv.call(READ_URI, &read)?;
        // a read names the keys the mode lacks in `errorKey` and answers `returnValue: false` with the rest
        let have = now.get("settings").and_then(Value::as_object).ok_or_else(|| format!("the TV did not answer for {}", p.mode))?;
        let journal = self.saved.journal.entry(p.mode.to_owned()).or_default();
        let mut write = Map::new();
        for (key, value) in p.settings {
            let Some(current) = have.get(*key) else { continue };
            journal.entry((*key).to_owned()).or_insert_with(|| current.clone());
            if text(current) != *value {
                write.insert((*key).to_owned(), json!(value));
            }
        }
        // the journal is on disk before anything it covers is written
        self.persist();
        if !write.is_empty() {
            let r = self.tv.call(WRITE_URI, &json!({"category": "picture", "settings": write, "dimension": dimension}))?;
            if r.get("returnValue").and_then(Value::as_bool) == Some(false) {
                return Err(format!("the TV refused {}: {}", p.name, r.get("errorText").and_then(Value::as_str).unwrap_or("refused")));
            }
            let back = self.tv.call(READ_URI, &read)?;
            if let Some((key, _)) = write.iter().find(|(k, v)| back.pointer(&format!("/settings/{k}")).map(text) != v.as_str().map(str::to_owned)) {
                return Err(format!("the TV kept its own {key} in {}", p.mode));
            }
        }
        self.saved.applied.insert(p.mode.to_owned(), p.id.to_owned());
        self.persist();
        crate::log_info!("picture: {} written into {} ({} keys changed)", p.name, p.mode, write.len());
        Ok(p.mode)
    }

    /// Puts back every value presets replaced in `mode`.
    fn restore(&mut self, mode: &str) -> Result<(), String> {
        let Some(journal) = self.saved.journal.get(mode).filter(|j| !j.is_empty()) else {
            return Err(format!("LumaTV has not changed {mode}"));
        };
        let dimension = json!({"pictureMode": mode, "input": APPS_INPUT});
        let settings: Map<String, Value> = journal.iter().map(|(k, v)| (k.clone(), json!(text(v)))).collect();
        let r = self.tv.call(WRITE_URI, &json!({"category": "picture", "settings": settings, "dimension": dimension}))?;
        if r.get("returnValue").and_then(Value::as_bool) == Some(false) {
            return Err(format!("the TV refused: {}", r.get("errorText").and_then(Value::as_str).unwrap_or("refused")));
        }
        let keys: Vec<&String> = settings.keys().collect();
        let back = self.tv.call(READ_URI, &json!({"category": "picture", "keys": keys, "dimension": dimension}))?;
        if let Some((key, _)) = settings.iter().find(|(k, v)| back.pointer(&format!("/settings/{k}")).map(text) != v.as_str().map(str::to_owned)) {
            return Err(format!("the TV kept LumaTV's {key} in {mode}"));
        }
        self.saved.journal.remove(mode);
        self.saved.applied.remove(mode);
        self.persist();
        crate::log_info!("picture: {mode} has its own settings back");
        Ok(())
    }

    pub fn command(&mut self, now: Instant, cmd: Cmd) -> Result<Value, String> {
        match cmd {
            Cmd::Preset(id) => {
                let mode = self.write_preset(&id)?;
                let slot = if self.separate { self.active.unwrap_or(Slot::Ui) } else { Slot::Video };
                self.choose(now, slot, mode);
            }
            Cmd::Slot(slot, choice) => {
                let mode = match preset(&choice) {
                    Some(_) => self.write_preset(&choice)?.to_owned(),
                    None if SDR_MODES.contains(&choice.as_str()) && choice != LG_GAME => choice,
                    None => return Err(format!("{choice} is not an SDR picture mode or a preset")),
                };
                self.choose(now, slot, &mode);
            }
            Cmd::Restore(mode) => self.restore(&mode)?,
        }
        Ok(self.status())
    }

    fn choose(&mut self, now: Instant, slot: Slot, mode: &str) {
        self.saved.set(slot, mode);
        self.persist();
        let in_force = if self.separate { self.active == Some(slot) } else { slot == Slot::Video };
        if in_force {
            self.settle_until = Some(now + SETTLE);
            self.go(now, mode, true);
        }
    }

    /// When the next thing is due: a debounce ending, a write's echo, the settle, the hold or the rate limit.
    pub fn next(&self, now: Instant) -> Option<Instant> {
        let mut due: Vec<Instant> = Vec::new();
        if let Some((eligible, since)) = self.eligible
            && self.active != Some(if eligible { Slot::Video } else { Slot::Ui })
        {
            due.push(since + if eligible { PLAY } else { STOP });
        }
        if let Some((_, at)) = &self.pending {
            due.push(*at + ECHO);
        }
        due.extend(self.settle_until);
        due.extend(self.hold_until);
        if let Some(last) = self.writes.back() {
            due.push(*last + MIN_GAP);
        }
        if self.writes.len() >= PER_MINUTE {
            due.extend(self.writes.front().map(|t| *t + Duration::from_secs(60)));
        }
        due.into_iter().filter(|t| *t > now).min()
    }

    pub fn status(&self) -> Value {
        let slot = |s: Slot| {
            let mode = self.saved.slot(s);
            json!({"mode": mode, "preset": mode.and_then(|m| self.saved.applied.get(m))})
        };
        json!({
            "separate": self.separate,
            "active": self.active.map(Slot::name),
            "ui": slot(Slot::Ui),
            "video": slot(Slot::Video),
            "presets": PRESETS.iter().map(|p| json!({"id": p.id, "name": p.name, "mode": p.mode, "applied": self.saved.applied.get(p.mode).is_some_and(|a| a == p.id)})).collect::<Vec<_>>(),
            "changed": self.saved.journal.iter().filter(|(_, j)| !j.is_empty()).map(|(m, _)| m.clone()).collect::<Vec<_>>(),
            "context": self.ctx.as_ref().map(|c| json!({"mode": c.mode, "input": c.input, "range": c.range})),
            "playing": self.playing.as_ref().map(|v| json!({"app": v.app, "hdr": v.hdr, "full": v.full})),
            "hold": self.hold(),
            "error": self.error,
        })
    }
}

/// A setting's value as the service compares it: numbers and strings alike.
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The owner's switch: `system.separatePicture` of theme.json, on unless it says `false`.
fn separate_on(theme: &Path) -> bool {
    std::fs::read(theme)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.pointer("/system/separatePicture").and_then(Value::as_bool))
        != Some(false)
}

/// A game in front: game mode on (a named game or an API lease), or a named game first on screen.
fn game_in_front(hub: &crate::hub::Hub) -> bool {
    use crate::app_priority::game;
    hub.get(game::TOPIC).and_then(|g| g.get("on").and_then(Value::as_bool)) == Some(true)
        || hub
            .get(crate::app_priority::TOPIC)
            .and_then(|a| a.pointer("/front/0").and_then(Value::as_str).map(|f| game::GAMES.contains(&f)))
            == Some(true)
}

enum Msg {
    Picture(Context),
    Video(Option<Video>),
    Cmd(Cmd, mpsc::Sender<Result<Value, String>>),
    Stop,
}

/// The way in for the API: requests go to the thread, which answers with the status or why not.
#[derive(Default)]
pub struct Remote(Mutex<Option<mpsc::Sender<Msg>>>);

impl Remote {
    /// `None` while the thread is not running.
    pub fn request(&self, cmd: Cmd) -> Option<Result<Value, String>> {
        let (tx, rx) = mpsc::channel();
        self.0.lock().as_ref()?.send(Msg::Cmd(cmd, tx)).ok()?;
        // a preset is a read, a write and a read back, each up to luna-send's deadline
        rx.recv_timeout(Duration::from_secs(15)).ok()
    }
}

pub fn spawn(ctx: &Ctx, remote: std::sync::Arc<Remote>) -> std::io::Result<JoinHandle<()>> {
    let ctx = ctx.clone();
    thread::Builder::new().name("picture".to_owned()).stack_size(256 * 1024).spawn(move || run(&ctx, &remote))
}

fn run(ctx: &Ctx, remote: &Remote) {
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        ctx.stop.on_stop(move || {
            let _ = tx.send(Msg::Stop);
        });
    }
    let luna = ctx.config.luna.send.clone();
    let path = ctx.paths.data_dir.join(STATE);
    let theme = ctx.paths.theme_file();
    let mut engine = Engine::new(Box::new(LunaSource::new(&luna)), load(&path), Some(path), ctx.config.brand_app_id.iter().cloned().collect(), separate_on(&theme));
    remote.0.lock().replace(tx.clone());
    let mut follows: Vec<crate::luna::Follow> = Vec::new();
    let mut retry_at: Option<Instant> = None;
    let mut looked = Instant::now();
    loop {
        let now = Instant::now();
        if follows.len() != 2 || !follows.iter().all(crate::luna::Follow::alive) {
            follows.clear();
            if retry_at.is_none_or(|t| now >= t) {
                match subscribe(&luna, &tx) {
                    Ok(started) => follows = started,
                    Err(e) => crate::log_warn!("picture: subscribing: {e}"),
                }
                retry_at = Some(now + Duration::from_secs(10));
            }
        }
        let wait = engine.next(now).map_or(POLL, |t| t.saturating_duration_since(now)).min(POLL);
        match rx.recv_timeout(wait) {
            Ok(Msg::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Picture(c)) => engine.picture(Instant::now(), c),
            Ok(Msg::Video(v)) => engine.video(Instant::now(), v),
            Ok(Msg::Cmd(cmd, reply)) => {
                let _ = reply.send(engine.command(Instant::now(), cmd));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let now = Instant::now();
        engine.set_game(game_in_front(&ctx.hub));
        // theme.json is read at most once a second, not on every video event
        if now.saturating_duration_since(looked) >= Duration::from_secs(1) {
            looked = now;
            engine.set_separate(now, separate_on(&theme));
        }
        engine.tick(now);
        ctx.hub.set(TOPIC, engine.status());
    }
    remote.0.lock().take();
}

fn subscribe(luna: &str, tx: &mpsc::Sender<Msg>) -> std::io::Result<Vec<crate::luna::Follow>> {
    let picture_tx = tx.clone();
    let picture = crate::luna::Follow::start(
        "picture-mode",
        luna,
        READ_URI,
        PICTURE_PAYLOAD,
        Box::new(move |reply| {
            if let Some(c) = Context::parse(reply) {
                let _ = picture_tx.send(Msg::Picture(c));
            }
        }),
    )?;
    let video_tx = tx.clone();
    let video = crate::luna::Follow::start(
        "picture-video",
        luna,
        VIDEO_URI,
        VIDEO_PAYLOAD,
        Box::new(move |reply| {
            if let Some(v) = Video::parse(reply) {
                let _ = video_tx.send(Msg::Video(v));
            }
        }),
    )?;
    Ok(vec![picture, video])
}

#[cfg(test)]
mod tests;
