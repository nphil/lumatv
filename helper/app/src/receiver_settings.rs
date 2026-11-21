//! Settings observed on the current receiver session. A reply is not model identification.

use lumatv_denon::settings::{self, Channel, Reply, Setting, SoundCategory, SoundMode};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

const KEYS: &[&str] = &[
    "power",
    "input",
    "soundMode",
    "quickSelect",
    "dialog",
    "dialogControl",
    "subwoofer",
    "lfe",
    "bass",
    "treble",
    "toneEnabled",
    "cinemaEq",
    "drc",
    "loudness",
    "subwooferDirect",
    "multEq",
    "dynEq",
    "refOffset",
    "dynVol",
    "zone2Power",
    "zone2Volume",
    "zone2Input",
    "zone2Mute",
    "eco",
];
/// One level per speaker. `CV?` answers only for configured speakers, so a channel is offered once it has been heard.
const CHANNELS: [(Channel, &str); 20] = [
    (Channel::FrontLeft, "channelFL"),
    (Channel::FrontRight, "channelFR"),
    (Channel::Center, "channelC"),
    (Channel::Subwoofer, "channelSW"),
    (Channel::Subwoofer2, "channelSW2"),
    (Channel::SurroundLeft, "channelSL"),
    (Channel::SurroundRight, "channelSR"),
    (Channel::SurroundBackLeft, "channelSBL"),
    (Channel::SurroundBackRight, "channelSBR"),
    (Channel::SurroundBack, "channelSB"),
    (Channel::FrontHeightLeft, "channelFHL"),
    (Channel::FrontHeightRight, "channelFHR"),
    (Channel::TopFrontLeft, "channelTFL"),
    (Channel::TopFrontRight, "channelTFR"),
    (Channel::TopMiddleLeft, "channelTML"),
    (Channel::TopMiddleRight, "channelTMR"),
    (Channel::FrontDolbyLeft, "channelFDL"),
    (Channel::FrontDolbyRight, "channelFDR"),
    (Channel::SurroundDolbyLeft, "channelSDL"),
    (Channel::SurroundDolbyRight, "channelSDR"),
];
const CATEGORIES: [(SoundCategory, &str); 4] = [
    (SoundCategory::Movie, "Movie"),
    (SoundCategory::Music, "Music"),
    (SoundCategory::Game, "Game"),
    (SoundCategory::Pure, "Pure"),
];
/// The S760H manual's sound modes; it has no Pure Direct. Any identified model is offered the first two.
const STYLES: [(SoundMode, &str); 12] = [
    (SoundMode::Stereo, "Stereo"),
    (SoundMode::Direct, "Direct"),
    (SoundMode::Auto, "Auto"),
    (SoundMode::Dolby, "Dolby"),
    (SoundMode::Dts, "DTS"),
    (SoundMode::MultiChStereo, "Multi Ch Stereo"),
    (SoundMode::Virtual, "Virtual"),
    (SoundMode::MonoMovie, "Mono Movie"),
    (SoundMode::RockArena, "Rock Arena"),
    (SoundMode::JazzClub, "Jazz Club"),
    (SoundMode::Matrix, "Matrix"),
    (SoundMode::VideoGame, "Video Game"),
];
/// How long an accepted write may wait to be sent before it is dropped.
const FRESH: Duration = Duration::from_secs(8);

fn keys() -> impl Iterator<Item = &'static str> {
    KEYS.iter()
        .copied()
        .chain(CHANNELS.iter().map(|(_, key)| *key))
}

fn channel_key(channel: Channel) -> Option<&'static str> {
    CHANNELS
        .iter()
        .find(|(c, _)| *c == channel)
        .map(|(_, key)| *key)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Profile {
    /// Commands from the FY21 chart's AVR-S960H column, narrowed to what the AVR-S760H manual describes.
    S760h,
    /// Another identified model: power, input and the Stereo and Direct modes, which every chart since 2015 shares.
    Generic,
}

impl Profile {
    fn of(model: &str) -> Self {
        if model.eq_ignore_ascii_case("AVR-S760H") {
            Self::S760h
        } else {
            Self::Generic
        }
    }
}
#[derive(Default)]
struct Entry {
    value: Option<Value>,
    at_ms: Option<u64>,
    /// The link's context revision when the value was read: a value read before the latest mode, signal, power or input
    /// change is still shown but not written until read again.
    revision: u64,
    pending: Option<String>,
    result: Option<Value>,
}

#[derive(Default)]
pub(crate) struct SettingsState {
    entries: BTreeMap<&'static str, Entry>,
    pub session: Option<u64>,
    context_revision: u64,
    /// Revision of the latest confirmed mode, signal, power or input change.
    changed_revision: u64,
    identity_attempted: bool,
    next_identity: Option<Instant>,
    profile: Option<Profile>,
    channels: BTreeSet<&'static str>,
    signal: Option<String>,
    prerequisites: Option<settings::Prerequisites>,
    /// The controls were read since the last change that can alter them.
    prerequisites_current: bool,
    next_prerequisites: Option<Instant>,
    pub interest_until: Option<Instant>,
    boot: u64,
    next_request: u64,
}

impl SettingsState {
    pub fn new(boot: u64) -> Self {
        Self {
            entries: keys().map(|k| (k, Entry::default())).collect(),
            boot,
            ..Self::default()
        }
    }

    pub fn connected(&mut self, session: u64) {
        self.invalidate("Receiver connection changed");
        self.session = Some(session);
        self.context_revision = 0;
        self.changed_revision = 0;
        self.identity_attempted = false;
        self.next_identity = None;
        self.profile = None;
    }

    pub fn observe_context(&mut self, session: u64, revision: u64) {
        if self.session == Some(session) {
            self.context_revision = self.context_revision.max(revision);
        }
    }

    pub fn context_changed(&mut self, session: u64, revision: u64, _field: &str) {
        if self.session != Some(session) {
            return;
        }
        self.observe_context(session, revision);
        self.context_moved();
    }

    pub fn identity_due(&self) -> bool {
        self.session.is_some()
            && !self.identity_attempted
            && self.interest_until.is_some_and(|end| Instant::now() < end)
            && self.next_identity.is_none_or(|at| Instant::now() >= at)
    }

    pub fn qualify(&mut self, model: Option<&str>) {
        let model = model.map(|name| name.trim().trim_start_matches('*').trim());
        let model = model.filter(|name| !name.is_empty());
        self.identity_attempted = model.is_some();
        self.next_identity =
            (!self.identity_attempted).then(|| Instant::now() + Duration::from_secs(5));
        self.profile = model.map(Profile::of);
    }

    fn offered(&self, key: &str) -> bool {
        match self.profile {
            Some(Profile::S760h) => !key.starts_with("channel") || self.channels.contains(key),
            Some(Profile::Generic) => matches!(key, "power" | "input" | "soundMode"),
            None => false,
        }
    }

    pub fn prerequisites_due(&self) -> bool {
        self.profile.is_some()
            && self.session.is_some()
            && self.interest_until.is_some_and(|end| Instant::now() < end)
            && self
                .next_prerequisites
                .is_none_or(|at| Instant::now() >= at)
    }

    /// A failed read keeps the last controls: one timed-out request says nothing about what the receiver offers.
    pub fn set_prerequisites(&mut self, values: Option<settings::Prerequisites>) {
        if let Some(values) = values {
            match &mut self.prerequisites {
                Some(known) => known.merge(values),
                None => self.prerequisites = Some(values),
            }
            self.prerequisites_current = true;
        }
        self.next_prerequisites = Some(Instant::now() + Duration::from_secs(5));
    }

    fn prerequisites(&self) -> Option<&settings::Prerequisites> {
        self.prerequisites.as_ref()
    }

    pub fn token(&self) -> Option<String> {
        if self.profile.is_some() {
            self.session.map(|s| format!("{}-{s}", self.boot))
        } else {
            None
        }
    }

    pub fn invalidate(&mut self, reason: &str) {
        for entry in self.entries.values_mut() {
            entry.value = None;
            entry.at_ms = None;
            entry.revision = 0;
            if let Some(id) = entry.pending.take() {
                entry.result = Some(json!({"requestId": id, "status": "failed", "error": reason}));
            }
        }
        self.session = None;
        self.signal = None;
        self.prerequisites = None;
        self.prerequisites_current = false;
        self.next_prerequisites = None;
        self.profile = None;
        self.channels.clear();
        self.identity_attempted = false;
        self.next_identity = None;
    }

    pub fn mode(&mut self, session: u64, value: String, at_ms: u64) {
        if self.session != Some(session) {
            return;
        }
        if self.entries["soundMode"]
            .value
            .as_ref()
            .is_some_and(|old| old.as_str() != Some(value.as_str()))
        {
            self.context_moved();
        }
        let revision = self.context_revision;
        let entry = self.entries.get_mut("soundMode").unwrap();
        entry.value = Some(json!(value));
        entry.at_ms = Some(at_ms);
        entry.revision = revision;
    }

    pub fn signal(&mut self, session: u64, signal: String) {
        if self.session != Some(session) {
            return;
        }
        if self.signal.as_ref().is_some_and(|old| *old != signal) {
            self.context_moved();
        }
        self.signal = Some(signal);
    }

    /// A mode, signal, power or input change. Values stay on screen; the ones that depend on the change are re-read
    /// before they can be written again, and the receiver's controls are asked for at once.
    fn context_moved(&mut self) {
        self.changed_revision = self.changed_revision.max(self.context_revision);
        self.controls_moved();
    }

    fn controls_moved(&mut self) {
        self.prerequisites_current = false;
        self.next_prerequisites = None;
    }

    pub fn observe(&mut self, session: u64, reply: Reply, at_ms: u64) {
        if self.session != Some(session) {
            return;
        }
        // The exact mode name arrives through `mode`; the reply only names its style.
        let Some((key, value)) = reply_value(reply) else {
            return;
        };
        if key.starts_with("channel") {
            self.channels.insert(key);
        }
        let changed = self
            .entries
            .get(key)
            .and_then(|e| e.value.as_ref())
            .is_some_and(|old| old != &value);
        if changed && matches!(key, "power" | "input") {
            self.context_moved();
        }
        // These switch Audyssey and tone controls on and off on the receiver.
        if changed && matches!(key, "multEq" | "dynEq" | "toneEnabled") {
            self.controls_moved();
        }
        let revision = self.context_revision;
        let entry = self.entries.get_mut(key).unwrap();
        entry.value = Some(value);
        entry.at_ms = Some(at_ms);
        entry.revision = revision;
    }

    fn current(&self, key: &str) -> Option<&Value> {
        self.entries.get(key).and_then(|e| e.value.as_ref())
    }

    fn current_signal(&self) -> Option<&str> {
        self.signal.as_deref()
    }

    /// Power, Eco and Zone 2 do not depend on what the main zone plays.
    fn confirmed(&self, key: &str) -> bool {
        matches!(key, "power" | "eco")
            || key.starts_with("zone2")
            || self
                .entries
                .get(key)
                .is_some_and(|e| e.revision >= self.changed_revision)
    }

    /// Settings whose availability comes from the receiver's reported controls.
    fn uses_controls(key: &str) -> bool {
        !matches!(
            key,
            "power" | "eco" | "quickSelect" | "subwoofer" | "dialog" | "subwooferDirect"
        ) && !key.starts_with("channel")
    }

    /// Why `key` cannot be changed in the current mode and signal, in words for its row; `None` when it can.
    fn limit(&self, key: &str) -> Option<&'static str> {
        let prereq = self.prerequisites();
        let reported = |field: fn(&settings::Prerequisites) -> Option<bool>| prereq.and_then(field);
        let unless = |ok: bool, why: &'static str| (!ok).then_some(why);
        match key {
            "power" | "eco" => return None,
            "zone2Power" => {
                return unless(
                    reported(|p| p.zone2_present) == Some(true),
                    "Zone 2 is not set up on the receiver",
                );
            }
            _ => {}
        }
        if key.starts_with("zone2") {
            if self.current("zone2Power") != Some(&json!(true)) {
                return Some("Zone 2 is off");
            }
            let present = reported(|p| p.zone2_present) == Some(true);
            return match key {
                "zone2Volume" => unless(
                    present
                        && prereq.is_some_and(|p| {
                            p.zone2_volume_variable == Some(true)
                                && p.zone2_volume_limit.is_some_and(|max| max.fract() == 0.0)
                        })
                        && self
                            .current("zone2Volume")
                            .and_then(Value::as_f64)
                            .is_some_and(|v| v.fract() == 0.0),
                    "Zone 2 volume is fixed or its limit has not been read",
                ),
                "zone2Input" => unless(
                    present
                        && prereq
                            .and_then(|p| p.zone2_sources.as_ref())
                            .is_some_and(|v| !v.is_empty()),
                    "Zone 2 sources have not been read",
                ),
                "zone2Mute" => unless(present, "Zone 2 is not set up on the receiver"),
                _ => None,
            };
        }
        if self.current("power") != Some(&json!(true)) {
            return Some("Receiver is off");
        }
        let signal = self.current_signal();
        let dolby = signal.is_some_and(|s| s.starts_with("DOLBY"));
        let dts = signal.is_some_and(|s| s.starts_with("DTS"));
        if key == "soundMode" {
            let recognized = signal.is_some_and(|signal| {
                signal == "PCM"
                    || signal.starts_with("PCM ")
                    || matches!(
                        signal,
                        "DOLBY DIGITAL"
                            | "DOLBY DIGITAL PLUS"
                            | "DOLBY TRUEHD"
                            | "DOLBY ATMOS"
                            | "DTS"
                            | "DTS-HD"
                            | "DTS:X"
                    )
            });
            // The S760H reports Live TV's Dolby Digital as `SYSDA Unknown`; its own mode buttons still answer.
            return match reported(|p| p.sound_mode_available) {
                Some(true) => None,
                Some(false) => Some("The receiver is not taking mode changes now"),
                None => unless(recognized, "Waiting for the receiver to name the input signal"),
            };
        }
        let mode = self.current("soundMode").and_then(Value::as_str);
        let needs_mode = !matches!(key, "input" | "quickSelect" | "subwoofer" | "lfe" | "drc" | "loudness" | "dialogControl")
            && !key.starts_with("channel");
        if mode.is_none() && needs_mode {
            return Some("Waiting for the receiver's sound mode");
        }
        let mode = mode.unwrap_or_default();
        let style = SoundMode::reported(mode);
        let direct = mode.contains("DIRECT");
        let audyssey_mode = !direct && !mode.contains("VIRTUAL") && !mode.contains("DTS:X");
        let audyssey_on = self
            .current("multEq")
            .and_then(Value::as_str)
            .is_some_and(|v| v != "OFF");
        let audyssey = |field: fn(&settings::Prerequisites) -> Option<bool>| {
            if !audyssey_mode {
                return Some("Not available in this sound mode");
            }
            match reported(field) {
                Some(true) => None,
                Some(false) => Some("The receiver does not offer this now"),
                None => Some("Reading the receiver's Audyssey settings"),
            }
        };
        // What the receiver reports for the current stream wins; the manual's signal rules are the fallback.
        let surround = |field: fn(&settings::Prerequisites) -> Option<bool>, fallback: bool, why| {
            unless(reported(field).unwrap_or(fallback), why)
        };
        match key {
            "input" => unless(
                prereq
                    .and_then(|p| p.main_sources.as_ref())
                    .is_some_and(|v| !v.is_empty()),
                "Input names have not been read",
            ),
            "bass" | "treble" | "toneEnabled" if direct => Some("Not available in Direct modes"),
            "bass" | "treble" | "toneEnabled" if self.current("dynEq") != Some(&json!(false)) => {
                Some("Turn off Dynamic EQ to use tone control")
            }
            "bass" | "treble" | "toneEnabled" if reported(|p| p.tone_available) != Some(true) => {
                Some("Tone control is not available now")
            }
            "bass" | "treble" => unless(
                reported(|p| p.tone_enabled) == Some(true)
                    && self.current("toneEnabled") == Some(&json!(true)),
                "Turn on Tone control first",
            ),
            "cinemaEq" => surround(
                |p| p.cinema_eq_available,
                mode.contains("DOLBY") || mode.contains("DTS"),
                "Only in Dolby and DTS modes",
            ),
            // The manual's conditions: Dialog Enhancer is not set in Direct or Virtual.
            "dialog" => unless(
                !direct && style != Some(SoundMode::Virtual),
                "Not available in Direct and Virtual modes",
            ),
            "lfe" => surround(|p| p.lfe_available, dolby || dts, "Only for Dolby and DTS sound"),
            "drc" => surround(|p| p.drc_available, dolby || dts, "Only for Dolby and DTS sound"),
            "loudness" => surround(|p| p.loudness_available, dolby, "Only for Dolby sound"),
            "dialogControl" => surround(
                |p| p.dialog_control_available,
                signal == Some("DTS:X"),
                "Only for DTS:X sound",
            ),
            "subwooferDirect" => unless(
                matches!(style, Some(SoundMode::Direct | SoundMode::Stereo)),
                "Only in Direct and Stereo modes",
            ),
            "multEq" => audyssey(|p| p.mult_eq_available),
            "dynEq" | "dynVol" | "refOffset" if audyssey_mode && !audyssey_on => {
                Some("Turn on MultEQ first")
            }
            "dynEq" => audyssey(|p| p.dynamic_eq_available),
            "dynVol" => audyssey(|p| p.dynamic_volume_available),
            "refOffset" if audyssey_mode && self.current("dynEq") != Some(&json!(true)) => {
                Some("Turn on Dynamic EQ first")
            }
            "refOffset" => audyssey(|p| p.reference_level_available),
            _ => None,
        }
    }

    fn writable(&self, key: &str) -> Result<(), &'static str> {
        if let Some(why) = self.limit(key) {
            return Err(why);
        }
        if !self.confirmed(key) || (Self::uses_controls(key) && !self.prerequisites_current) {
            return Err("Checking with the receiver");
        }
        Ok(())
    }

    pub fn document(&self) -> Value {
        let mut map = serde_json::Map::new();
        for key in keys() {
            let e = &self.entries[key];
            let live = self.current(key);
            let offered = self.offered(key);
            let supported = offered && live.is_some();
            // A pending write is reported in `pending`, not by clearing `writable`: the page queues the next change behind
            // it instead of disabling the row.
            let gate = self.writable(key);
            let writable = supported && gate.is_ok();
            let support = if supported {
                "supported"
            } else if !offered && (self.profile.is_some() || key.starts_with("channel")) {
                "unsupported"
            } else {
                "unknown"
            };
            let mut desc = json!({"support": support, "value": live, "updatedAt": if live.is_some() {e.at_ms} else {None}, "writable": writable, "options": if supported { self.options(key, live) } else { Vec::new() }, "pending": e.pending.as_ref().map(|id| json!({"requestId":id})), "result":e.result});
            if !supported {
                desc["reason"] = json!(if self.profile.is_none() {
                    "Receiver model has not been identified for this connection"
                } else if !offered && key.starts_with("channel") {
                    "No speaker is configured on this channel"
                } else if !offered {
                    "Not offered on this receiver model"
                } else {
                    "Waiting for a current receiver reply"
                });
            } else if let Err(why) = gate {
                desc["reason"] = json!(why);
            }
            if key == "soundMode"
                && let Some(style) = live.and_then(Value::as_str).and_then(SoundMode::reported)
            {
                desc["style"] = json!(style.as_str());
                desc["category"] = json!(style.category().map(SoundCategory::as_str));
            }
            match key {
                "bass" | "treble" => {
                    desc["min"] = json!(-6);
                    desc["max"] = json!(6);
                    desc["step"] = json!(1);
                    desc["unit"] = json!("dB");
                }
                "refOffset" => {
                    desc["min"] = json!(0);
                    desc["max"] = json!(15);
                    desc["step"] = json!(5);
                    desc["unit"] = json!("dB");
                }
                "zone2Volume" => {
                    desc["unit"] = json!("absolute");
                    if let Some(max) = self
                        .prerequisites()
                        .and_then(|p| p.zone2_volume_limit)
                        .filter(|_| self.limit(key).is_none())
                    {
                        desc["min"] = json!(0);
                        desc["max"] = json!(max);
                        desc["step"] = json!(1);
                    }
                }
                trim if trim == "subwoofer" || trim.starts_with("channel") => {
                    desc["min"] = json!(-12);
                    desc["max"] = json!(12);
                    desc["step"] = json!(0.5);
                    desc["unit"] = json!("dB");
                }
                "lfe" => {
                    desc["min"] = json!(-10);
                    desc["max"] = json!(0);
                    desc["step"] = json!(1);
                    desc["unit"] = json!("dB");
                }
                "dialogControl" => {
                    desc["min"] = json!(0);
                    desc["max"] = json!(6);
                    desc["step"] = json!(1);
                }
                _ => {}
            }
            map.insert(key.to_owned(), desc);
        }
        Value::Object(map)
    }

    pub fn prepare(
        &mut self,
        key: &str,
        value: &Value,
        token: &str,
    ) -> Result<(String, u64, u64, Setting, Instant), &'static str> {
        if self.token().as_deref() != Some(token) {
            return Err("Receiver session changed or is unavailable");
        }
        if !self.entries.contains_key(key) {
            return Err("Unknown setting");
        }
        if self.entries.values().any(|entry| entry.pending.is_some()) {
            return Err("Another change is pending");
        }
        let setting = to_setting(key, value).ok_or("Invalid setting value")?;
        if !self.offered(key) || self.current(key).is_none() || self.writable(key).is_err() {
            return Err("Setting is unavailable");
        }
        let allowed = self.options(key, self.current(key));
        if !allowed.is_empty() && !allowed.iter().any(|o| o["value"] == *value) {
            return Err("Value is not currently advertised");
        }
        if key == "zone2Volume"
            && !value.as_f64().is_some_and(|v| {
                v.fract() == 0.0
                    && self
                        .prerequisites()
                        .and_then(|p| p.zone2_volume_limit)
                        .is_some_and(|max| v <= max)
            })
        {
            return Err("Value is not currently advertised");
        }
        // The link refuses the write if the mode, signal, power or input moved since this revision.
        let valid_until = Instant::now() + FRESH;
        self.next_request += 1;
        Ok((
            format!("{}-{}", self.boot, self.next_request),
            self.session.unwrap(),
            self.context_revision,
            setting,
            valid_until,
        ))
    }

    fn options(&self, key: &str, current: Option<&Value>) -> Vec<Value> {
        if key == "soundMode" {
            let full = self.profile == Some(Profile::S760h);
            let categories = CATEGORIES.iter().filter(|_| full).map(|(category, label)| {
                json!({"value": category.as_str(), "label": label, "kind": "category"})
            });
            let styles = STYLES.iter().take(if full { STYLES.len() } else { 2 }).map(
                |(style, label)| json!({"value": style.as_str(), "label": label, "kind": "style"}),
            );
            return categories.chain(styles).collect();
        }
        if key == "drc" {
            // The manual rules out Auto for DTS signals.
            let dts = self.current_signal().is_some_and(|s| s.starts_with("DTS"));
            return [
                (settings::Drc::Off, "Off"),
                (settings::Drc::Auto, "Auto"),
                (settings::Drc::Low, "Low"),
                (settings::Drc::Medium, "Medium"),
                (settings::Drc::High, "High"),
            ]
            .into_iter()
            .filter(|(value, _)| !(dts && *value == settings::Drc::Auto))
            .map(|(value, label)| json!({"value": value.as_str(), "label": label}))
            .collect();
        }
        let sources = match key {
            "input" => self.prerequisites().and_then(|p| p.main_sources.as_ref()),
            "zone2Input" => self.prerequisites().and_then(|p| p.zone2_sources.as_ref()),
            _ => return options(key, current),
        };
        sources
            .map(|items| {
                items
                    .iter()
                    .map(|item| json!({"value":item.id.as_str(), "label":item.name}))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn pending(&mut self, key: &str, request_id: String) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.pending = Some(request_id);
            entry.result = None;
        }
    }

    pub fn finish(&mut self, request_id: &str, status: &str, error: Option<String>) {
        if let Some(entry) = self
            .entries
            .values_mut()
            .find(|e| e.pending.as_deref() == Some(request_id))
        {
            entry.pending = None;
            entry.result = Some(json!({"requestId":request_id,"status":status,"error":error}));
        }
    }
}

fn options(key: &str, current: Option<&Value>) -> Vec<Value> {
    let vals: Vec<Value> = match key {
        "power" | "toneEnabled" | "cinemaEq" | "dynEq" | "zone2Power" | "zone2Mute"
        | "loudness" | "subwooferDirect" => {
            vec![json!(false), json!(true)]
        }
        "quickSelect" => (1..=4).map(|n| json!(n)).collect(),
        "dialog" => ["off", "low", "medium", "high"]
            .into_iter()
            .map(|s| json!(s))
            .collect(),
        "eco" => ["OFF", "AUTO", "ON"]
            .into_iter()
            .map(|s| json!(s))
            .collect(),
        "multEq" => ["OFF", "AUDYSSEY", "BYP.LR", "FLAT"]
            .into_iter()
            .map(|s| json!(s))
            .collect(),
        "dynVol" => ["OFF", "LIGHT", "MEDIUM", "HEAVY"]
            .into_iter()
            .map(|s| json!(s))
            .collect(),
        "refOffset" => [0, 5, 10, 15].into_iter().map(|n| json!(n)).collect(),
        "input" | "zone2Input" => current.cloned().into_iter().collect(),
        _ => Vec::new(),
    };
    vals.into_iter().map(|v| json!({"label": v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()), "value": v})).collect()
}

fn reply_value(reply: Reply) -> Option<(&'static str, Value)> {
    Some(match reply {
        Reply::Power(v) => ("power", json!(v)),
        Reply::Source(v) => ("input", json!(v.as_str())),
        // Stands for the style; the exact name the entry holds comes from `SettingsState::mode`.
        Reply::SoundMode(_) => return None,
        Reply::QuickSelect(v) => ("quickSelect", json!(v.as_str().parse::<u8>().ok())),
        Reply::DialogEnhancer(v) => (
            "dialog",
            json!(match v {
                settings::DialogEnhancer::Off => "off",
                settings::DialogEnhancer::Low => "low",
                settings::DialogEnhancer::Medium => "medium",
                settings::DialogEnhancer::High => "high",
            }),
        ),
        Reply::Eco(v) => ("eco", json!(v.as_str())),
        Reply::Bass(v) => ("bass", json!(v.db())),
        Reply::Treble(v) => ("treble", json!(v.db())),
        Reply::ToneEnabled(v) => ("toneEnabled", json!(v)),
        Reply::CinemaEq(v) => ("cinemaEq", json!(v)),
        Reply::MultEq(v) => ("multEq", json!(v.as_str())),
        Reply::DynamicEq(v) => ("dynEq", json!(v)),
        Reply::ReferenceLevel(v) => ("refOffset", json!(v.as_str().parse::<u8>().ok())),
        Reply::DynamicVolume(v) => (
            "dynVol",
            json!(match v {
                settings::DynamicVolume::Light => "LIGHT",
                settings::DynamicVolume::Medium => "MEDIUM",
                settings::DynamicVolume::Heavy => "HEAVY",
                _ => "OFF",
            }),
        ),
        Reply::Zone2Power(v) => ("zone2Power", json!(v)),
        Reply::Zone2Source(v) => ("zone2Input", json!(v.as_str())),
        Reply::Zone2Volume(v) => ("zone2Volume", json!(v.value())),
        Reply::Zone2Mute(v) => ("zone2Mute", json!(v)),
        Reply::DialogControl(v) => ("dialogControl", json!(v.level())),
        Reply::SubwooferLevel(v) => ("subwoofer", json!(v.db())),
        Reply::SubwooferDirect(v) => ("subwooferDirect", json!(v)),
        Reply::ChannelLevel(channel, v) => (channel_key(channel)?, json!(v.db())),
        Reply::Lfe(v) => ("lfe", json!(v.db())),
        Reply::Drc(v) => ("drc", json!(v.as_str())),
        Reply::Loudness(v) => ("loudness", json!(v)),
    })
}

fn to_setting(key: &str, value: &Value) -> Option<Setting> {
    let text = || value.as_str();
    let whole = || {
        value
            .as_f64()
            .filter(|v| v.fract() == 0.0 && v.abs() <= 127.0)
            .map(|v| v as i8)
    };
    let tone = || settings::Tone::new(whole()?);
    if let Some((channel, _)) = CHANNELS.iter().find(|(_, k)| *k == key) {
        return Some(Setting::ChannelLevel(
            *channel,
            settings::Trim::new(value.as_f64()?)?,
        ));
    }
    Some(match key {
        "power" => Setting::Power(value.as_bool()?),
        "input" => Setting::Source(settings::Source::parse(text()?)?),
        "soundMode" => {
            let name = text()?;
            match SoundCategory::parse(name) {
                Some(category) => Setting::SoundCategory(category),
                None => Setting::SoundMode(SoundMode::parse(name).filter(|m| m.writable())?),
            }
        }
        "quickSelect" => Setting::QuickSelect(
            settings::QuickSelect::parse(&value.as_u64()?.to_string())
                .filter(|q| *q != settings::QuickSelect::Unset)?,
        ),
        "dialogControl" => Setting::DialogControl(settings::DialogControl::new(
            u8::try_from(value.as_u64()?).ok()?,
        )?),
        "subwoofer" => Setting::SubwooferLevel(settings::Trim::new(value.as_f64()?)?),
        "lfe" => Setting::Lfe(settings::Lfe::new(whole()?)?),
        "drc" => Setting::Drc(settings::Drc::parse(text()?)?),
        "loudness" => Setting::Loudness(value.as_bool()?),
        "subwooferDirect" => Setting::SubwooferDirect(value.as_bool()?),
        "zone2Mute" => Setting::Zone2Mute(value.as_bool()?),
        "dialog" => Setting::DialogEnhancer(match text()? {
            "off" => settings::DialogEnhancer::Off,
            "low" => settings::DialogEnhancer::Low,
            "medium" => settings::DialogEnhancer::Medium,
            "high" => settings::DialogEnhancer::High,
            _ => return None,
        }),
        "eco" => Setting::Eco(settings::Eco::parse(text()?)?),
        "bass" => Setting::Bass(tone()?),
        "treble" => Setting::Treble(tone()?),
        "toneEnabled" => Setting::ToneEnabled(value.as_bool()?),
        "cinemaEq" => Setting::CinemaEq(value.as_bool()?),
        "multEq" => Setting::MultEq(settings::MultEq::parse(text()?)?),
        "dynEq" => Setting::DynamicEq(value.as_bool()?),
        "refOffset" => Setting::ReferenceLevel(settings::ReferenceLevel::parse(
            &value.as_u64()?.to_string(),
        )?),
        "dynVol" => Setting::DynamicVolume(settings::DynamicVolume::parse(match text()? {
            "LIGHT" => "LIT",
            "MEDIUM" => "MED",
            "HEAVY" => "HEV",
            v => v,
        })?),
        "zone2Power" => Setting::Zone2Power(value.as_bool()?),
        "zone2Input" => Setting::Zone2Source(settings::Source::parse(text()?)?),
        "zone2Volume" => Setting::Zone2Volume(settings::ZoneVolume::new(value.as_f64()?)?),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_identity_retries_only_with_interest() {
        let mut state = SettingsState::new(1);
        state.connected(1);
        assert!(!state.identity_due());
        state.interest_until = Some(Instant::now() + Duration::from_secs(20));
        assert!(state.identity_due());
        state.qualify(None);
        assert!(!state.identity_due());
        state.next_identity = Some(Instant::now() - Duration::from_secs(1));
        assert!(state.identity_due());
        state.qualify(Some("*   "));
        assert!(!state.identity_due());
        state.next_identity = Some(Instant::now() - Duration::from_secs(1));
        state.qualify(Some("AVR-S760H"));
        assert!(!state.identity_due());
        assert_eq!(state.token().as_deref(), Some("1-1"));
    }

    #[test]
    fn settings_are_session_qualified() {
        let mut state = SettingsState::new(42);
        state.connected(1);
        state.observe(1, Reply::Power(true), 100);
        assert_eq!(state.document()["power"]["support"], "unknown");
        state.qualify(Some("AVR-S760H"));
        assert_eq!(state.document()["power"]["updatedAt"], 100);
        assert!(state.prepare("power", &json!(false), "42-1").is_ok());
        assert!(state.prepare("power", &json!("false"), "42-1").is_err());
        state.connected(2);
        state.qualify(Some("AVR-S760H"));
        assert!(state.prepare("power", &json!(false), "42-1").is_err());
        assert!(state.document()["power"]["value"].is_null());
    }
    #[test]
    fn zone2_limit_is_not_invented() {
        let mut state = SettingsState::new(1);
        state.connected(1);
        state.qualify(Some("AVR-S760H"));
        state.observe(1, Reply::Zone2Power(true), 1);
        state.observe(
            1,
            Reply::Zone2Volume(settings::ZoneVolume::new(35.0).unwrap()),
            2,
        );
        let zone = &state.document()["zone2Volume"];
        assert_eq!(zone["value"], 35.0);
        assert_eq!(zone["writable"], false);
        assert!(zone.get("max").is_none());
    }
    #[test]
    fn controls_require_read_prerequisites_and_distinct_zone_choices() {
        let mut state = SettingsState::new(2);
        state.connected(1);
        state.qualify(Some("AVR-S760H"));
        state.observe(1, Reply::Power(true), 1);
        state.mode(1, "STEREO".into(), 2);
        state.observe(1, Reply::DynamicEq(false), 3);
        state.observe(1, Reply::ToneEnabled(true), 4);
        state.observe(1, Reply::Bass(settings::Tone::new(0).unwrap()), 5);
        state.observe(1, Reply::Source(settings::Source::Tv), 6);
        state.observe(1, Reply::Zone2Power(true), 7);
        state.observe(1, Reply::Zone2Source(settings::Source::Cd), 8);
        assert_eq!(state.document()["bass"]["writable"], false);
        let p = settings::Prerequisites {
            tone_available: Some(true),
            tone_enabled: Some(true),
            zone2_present: Some(true),
            main_sources: Some(vec![settings::SourceChoice {
                id: settings::Source::Tv,
                name: "TV".into(),
            }]),
            zone2_sources: Some(vec![settings::SourceChoice {
                id: settings::Source::Cd,
                name: "CD".into(),
            }]),
            ..Default::default()
        };
        state.set_prerequisites(Some(p));
        let doc = state.document();
        assert_eq!(doc["bass"]["writable"], true);
        assert_eq!(doc["soundMode"]["writable"], false);
        assert!(state.prepare("soundMode", &json!("STEREO"), "2-1").is_err());
        assert_eq!(doc["input"]["options"][0]["value"], "TV");
        assert_eq!(doc["zone2Input"]["options"][0]["value"], "CD");
        assert!(state.prepare("zone2Input", &json!("TV"), "2-1").is_err());
        state.signal(1, "PCM ZERO".into());
        let mode = &state.document()["soundMode"];
        assert_eq!(mode["writable"], true);
        let options = mode["options"].as_array().unwrap();
        assert_eq!(options.len(), 16);
        assert_eq!(
            options[0],
            json!({"value":"MOVIE","label":"Movie","kind":"category"})
        );
        assert_eq!(
            options[5],
            json!({"value":"DIRECT","label":"Direct","kind":"style"})
        );
        assert!(state.prepare("soundMode", &json!("DIRECT"), "2-1").is_ok());

        // An input change keeps every value on screen; what depends on it waits for a fresh read.
        state.context_changed(1, 5, "input");
        let doc = state.document();
        assert_eq!(doc["input"]["value"], "TV");
        assert_eq!(doc["bass"]["value"], 0);
        assert_eq!(doc["soundMode"]["reason"], "Checking with the receiver");
        assert!(state.prepare("soundMode", &json!("DIRECT"), "2-1").is_err());
        state.mode(1, "STEREO".into(), 10);
        state.set_prerequisites(None);
        assert_eq!(state.document()["input"]["options"][0]["value"], "TV");
        assert!(state.prepare("soundMode", &json!("DIRECT"), "2-1").is_err());
        state.set_prerequisites(Some(Default::default()));
        assert!(state.prepare("soundMode", &json!("DIRECT"), "2-1").is_ok());
        assert!(
            state
                .prepare("soundMode", &json!("PURE DIRECT"), "2-1")
                .is_err()
        );
        assert_eq!(state.document()["bass"]["writable"], false);
        state.observe(1, Reply::Bass(settings::Tone::new(1).unwrap()), 11);
        assert_eq!(state.document()["bass"]["writable"], true);
        state.mode(1, "DIRECT".into(), 12);
        assert_eq!(state.document()["bass"]["reason"], "Not available in Direct modes");
    }

    #[test]
    fn live_tv_dolby_follows_the_controls_the_receiver_reports() {
        let mut state = SettingsState::new(5);
        state.connected(1);
        state.qualify(Some("AVR-S760H"));
        state.observe(1, Reply::Power(true), 1);
        state.mode(1, "DOLBY AUDIO-DSUR".into(), 2);
        // Live TV's Dolby Digital as the S760H names it.
        state.signal(1, "Unknown".into());
        state.observe(1, Reply::MultEq(settings::MultEq::Audyssey), 3);
        state.observe(1, Reply::DynamicEq(true), 4);
        state.observe(1, Reply::Lfe(settings::Lfe::new(0).unwrap()), 5);
        state.observe(1, Reply::Loudness(true), 6);
        state.observe(1, Reply::Drc(settings::Drc::Off), 7);
        state.observe(
            1,
            Reply::DialogControl(settings::DialogControl::new(0).unwrap()),
            8,
        );
        state.set_prerequisites(Some(settings::Prerequisites {
            mult_eq_available: Some(true),
            dynamic_eq_available: Some(true),
            lfe_available: Some(true),
            loudness_available: Some(true),
            drc_available: Some(true),
            dialog_control_available: Some(false),
            sound_mode_available: Some(true),
            ..Default::default()
        }));
        let doc = state.document();
        for key in ["soundMode", "multEq", "dynEq", "lfe", "loudness", "drc"] {
            assert_eq!(doc[key]["writable"], true, "{key}");
        }
        assert_eq!(doc["dialogControl"]["reason"], "Only for DTS:X sound");

        state.set_prerequisites(None);
        assert_eq!(state.document()["multEq"]["writable"], true);

        state.observe_context(1, 3);
        state.signal(1, "DOLBY DIGITAL".into());
        let lfe = &state.document()["lfe"];
        assert_eq!((&lfe["value"], &lfe["writable"]), (&json!(0), &json!(false)));
        state.set_prerequisites(Some(Default::default()));
        state.observe(1, Reply::Lfe(settings::Lfe::new(-2).unwrap()), 9);
        assert_eq!(state.document()["lfe"]["writable"], true);
    }

    #[test]
    fn models_get_only_the_controls_they_offer() {
        let mut state = SettingsState::new(3);
        state.connected(1);
        state.qualify(Some("AVR-X1700H"));
        state.observe(1, Reply::Power(true), 1);
        state.observe(1, Reply::Eco(settings::Eco::Auto), 2);
        state.mode(1, "STEREO".into(), 3);
        state.signal(1, "PCM".into());
        let doc = state.document();
        assert_eq!(doc["power"]["writable"], true);
        assert_eq!(doc["eco"]["support"], "unsupported");
        assert_eq!(
            doc["soundMode"]["options"],
            json!([
                {"value":"STEREO","label":"Stereo","kind":"style"},
                {"value":"DIRECT","label":"Direct","kind":"style"}
            ])
        );
        assert!(state.prepare("eco", &json!("ON"), "3-1").is_err());
        assert!(state.prepare("soundMode", &json!("MOVIE"), "3-1").is_err());
        state.qualify(Some("AVR-S760H"));
        let trim = settings::Trim::new(-1.5).unwrap();
        state.observe(
            1,
            Reply::ChannelLevel(settings::Channel::FrontLeft, trim),
            4,
        );
        let doc = state.document();
        assert_eq!(doc["eco"]["support"], "supported");
        assert_eq!(doc["channelFL"]["value"], -1.5);
        assert_eq!(doc["channelSDR"]["support"], "unsupported");
        assert!(state.prepare("channelFL", &json!(12.5), "3-1").is_err());
        assert!(matches!(
            state.prepare("channelFL", &json!(-12), "3-1"),
            Ok((
                _,
                _,
                _,
                Setting::ChannelLevel(settings::Channel::FrontLeft, _),
                _
            ))
        ));
    }

    #[test]
    fn sound_modes_and_surround_parameters_follow_mode_and_signal() {
        let mut state = SettingsState::new(4);
        state.connected(1);
        state.qualify(Some("AVR-S760H"));
        state.set_prerequisites(Some(Default::default()));
        state.observe(1, Reply::Power(true), 1);
        state.mode(1, "DOLBY AUDIO-DD+DSUR".into(), 2);
        state.observe(1, Reply::SoundMode(settings::SoundMode::Dolby), 3);
        state.signal(1, "DOLBY DIGITAL".into());
        state.observe(1, Reply::Loudness(true), 4);
        state.observe(1, Reply::Drc(settings::Drc::Auto), 5);
        state.observe(1, Reply::Lfe(settings::Lfe::new(0).unwrap()), 6);
        state.observe(1, Reply::QuickSelect(settings::QuickSelect::Unset), 7);
        state.observe(
            1,
            Reply::DialogEnhancer(settings::DialogEnhancer::Off),
            8,
        );
        let doc = state.document();
        assert_eq!(doc["soundMode"]["value"], "DOLBY AUDIO-DD+DSUR");
        assert_eq!(doc["soundMode"]["style"], "DOLBY DIGITAL");
        assert!(doc["soundMode"]["category"].is_null());
        assert!(matches!(
            state.prepare("soundMode", &json!("MOVIE"), "4-1"),
            Ok((_, _, _, Setting::SoundCategory(SoundCategory::Movie), _))
        ));
        assert!(
            state
                .prepare("soundMode", &json!("MULTI CH IN"), "4-1")
                .is_err()
        );
        assert_eq!(doc["loudness"]["writable"], true);
        assert_eq!(doc["dialog"]["writable"], true);
        assert_eq!(doc["drc"]["options"][1]["value"], "AUTO");
        assert!(state.prepare("lfe", &json!(-11), "4-1").is_err());
        assert!(state.prepare("lfe", &json!(-10), "4-1").is_ok());
        assert_eq!(doc["quickSelect"]["value"], 0);
        assert!(state.prepare("quickSelect", &json!(0), "4-1").is_err());
        assert!(state.prepare("quickSelect", &json!(4), "4-1").is_ok());
        state.signal(1, "DTS:X".into());
        state.set_prerequisites(Some(Default::default()));
        state.observe(1, Reply::Loudness(false), 9);
        state.observe(1, Reply::Drc(settings::Drc::Off), 10);
        state.observe(
            1,
            Reply::DialogControl(settings::DialogControl::new(0).unwrap()),
            11,
        );
        let doc = state.document();
        assert_eq!(doc["loudness"]["reason"], "Only for Dolby sound");
        assert_eq!(doc["dialogControl"]["writable"], true);
        assert!(
            !doc["drc"]["options"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["value"] == "AUTO")
        );
        state.mode(1, "VIRTUAL".into(), 12);
        state.observe(1, Reply::SubwooferDirect(false), 14);
        let doc = state.document();
        assert_eq!(doc["dialog"]["writable"], false);
        assert_eq!(doc["subwooferDirect"]["reason"], "Only in Direct and Stereo modes");
    }
}
