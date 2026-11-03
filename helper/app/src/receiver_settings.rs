//! Settings observed on the current receiver session. A reply is not model identification.

use lumatv_denon::settings::{self, Reply, Setting};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const KEYS: &[&str] = &[
    "power",
    "input",
    "soundMode",
    "quickSelect",
    "dialog",
    "subwoofer",
    "bass",
    "treble",
    "toneEnabled",
    "cinemaEq",
    "multEq",
    "dynEq",
    "refOffset",
    "dynVol",
    "zone2Power",
    "zone2Volume",
    "zone2Input",
    "eco",
];
const FRESH: Duration = Duration::from_secs(8);

#[derive(Default)]
struct Entry {
    value: Option<Value>,
    at: Option<Instant>,
    at_ms: Option<u64>,
    pending: Option<String>,
    result: Option<Value>,
}

#[derive(Default)]
pub(crate) struct SettingsState {
    entries: BTreeMap<&'static str, Entry>,
    pub session: Option<u64>,
    context_revision: u64,
    identity_attempted: bool,
    next_identity: Option<Instant>,
    qualified: bool,
    signal: Option<(String, Instant)>,
    prerequisites: Option<(settings::Prerequisites, Instant)>,
    next_prerequisites: Option<Instant>,
    pub interest_until: Option<Instant>,
    boot: u64,
    next_request: u64,
}

impl SettingsState {
    pub fn new(boot: u64) -> Self {
        Self {
            entries: KEYS.iter().map(|k| (*k, Entry::default())).collect(),
            boot,
            ..Self::default()
        }
    }

    pub fn connected(&mut self, session: u64) {
        self.invalidate("Receiver connection changed");
        self.session = Some(session);
        self.context_revision = 0;
        self.identity_attempted = false;
        self.next_identity = None;
        self.qualified = false;
    }

    pub fn observe_context(&mut self, session: u64, revision: u64) {
        if self.session == Some(session) {
            self.context_revision = self.context_revision.max(revision);
        }
    }

    pub fn context_changed(&mut self, session: u64, revision: u64, field: &str) {
        if self.session != Some(session) {
            return;
        }
        self.observe_context(session, revision);
        self.invalidate_context();
        if matches!(field, "power" | "input" | "signal") {
            self.signal = None;
        }
        for key in [field, "soundMode"] {
            if let Some(entry) = self.entries.get_mut(key) {
                entry.value = None;
                entry.at = None;
                entry.at_ms = None;
            }
        }
    }

    pub fn identity_due(&self) -> bool {
        self.session.is_some()
            && !self.identity_attempted
            && self.interest_until.is_some_and(|end| Instant::now() < end)
            && self.next_identity.is_none_or(|at| Instant::now() >= at)
    }

    pub fn qualify(&mut self, model: Option<&str>) {
        let model = model.map(|name| name.trim().trim_start_matches('*').trim());
        self.identity_attempted = model.is_some_and(|name| !name.is_empty());
        self.next_identity =
            (!self.identity_attempted).then(|| Instant::now() + Duration::from_secs(5));
        self.qualified = model.is_some_and(|name| name.eq_ignore_ascii_case("AVR-S760H"));
    }

    pub fn prerequisites_due(&self) -> bool {
        self.qualified
            && self.session.is_some()
            && self.interest_until.is_some_and(|end| Instant::now() < end)
            && self
                .next_prerequisites
                .is_none_or(|at| Instant::now() >= at)
    }

    pub fn set_prerequisites(&mut self, values: Option<settings::Prerequisites>) {
        let now = Instant::now();
        self.prerequisites = values.map(|v| (v, now));
        self.next_prerequisites = Some(now + Duration::from_secs(5));
    }

    fn prerequisites(&self) -> Option<&settings::Prerequisites> {
        self.prerequisites
            .as_ref()
            .filter(|(_, at)| at.elapsed() <= FRESH)
            .map(|(v, _)| v)
    }

    pub fn token(&self) -> Option<String> {
        if self.qualified {
            self.session.map(|s| format!("{}-{s}", self.boot))
        } else {
            None
        }
    }

    pub fn invalidate(&mut self, reason: &str) {
        for entry in self.entries.values_mut() {
            entry.value = None;
            entry.at = None;
            entry.at_ms = None;
            if let Some(id) = entry.pending.take() {
                entry.result = Some(json!({"requestId": id, "status": "failed", "error": reason}));
            }
        }
        self.session = None;
        self.signal = None;
        self.prerequisites = None;
        self.next_prerequisites = None;
        self.qualified = false;
        self.identity_attempted = false;
        self.next_identity = None;
    }

    pub fn mode(&mut self, session: u64, value: String, at: Instant, at_ms: u64) {
        if self.session != Some(session) {
            return;
        }
        if self.entries["soundMode"]
            .value
            .as_ref()
            .is_some_and(|old| old.as_str() != Some(value.as_str()))
        {
            self.invalidate_context();
        }
        let entry = self.entries.get_mut("soundMode").unwrap();
        entry.value = Some(json!(value));
        entry.at = Some(at);
        entry.at_ms = Some(at_ms);
    }

    pub fn signal(&mut self, session: u64, signal: String, at: Instant) {
        if self.session != Some(session) {
            return;
        }
        if self.signal.as_ref().is_some_and(|(old, _)| *old != signal) {
            self.invalidate_context();
        }
        self.signal = Some((signal, at));
    }

    fn invalidate_context(&mut self) {
        self.prerequisites = None;
        self.next_prerequisites = None;
        for (key, entry) in &mut self.entries {
            if !matches!(
                *key,
                "power"
                    | "input"
                    | "soundMode"
                    | "zone2Power"
                    | "zone2Input"
                    | "zone2Volume"
                    | "eco"
            ) {
                entry.value = None;
                entry.at = None;
                entry.at_ms = None;
            }
        }
    }

    pub fn observe(&mut self, session: u64, reply: Reply, at: Instant, at_ms: u64) {
        if self.session != Some(session) {
            return;
        }
        let (key, value) = reply_value(reply);
        let changed = self
            .entries
            .get(key)
            .and_then(|e| e.value.as_ref())
            .is_some_and(|old| old != &value);
        if changed && matches!(key, "power" | "input" | "soundMode") {
            self.invalidate_context();
            if matches!(key, "power" | "input") {
                self.signal = None;
            }
        }
        if changed && matches!(key, "multEq" | "dynEq" | "toneEnabled") {
            self.prerequisites = None;
            self.next_prerequisites = None;
            for (name, entry) in &mut self.entries {
                let dependent = match key {
                    "multEq" => matches!(*name, "dynEq" | "dynVol" | "refOffset"),
                    "dynEq" => matches!(*name, "refOffset" | "bass" | "treble"),
                    "toneEnabled" => matches!(*name, "bass" | "treble"),
                    _ => false,
                };
                if dependent {
                    entry.value = None;
                    entry.at = None;
                    entry.at_ms = None;
                }
            }
        }
        let entry = self.entries.get_mut(key).unwrap();
        entry.value = Some(value);
        entry.at = Some(at);
        entry.at_ms = Some(at_ms);
    }

    fn current(&self, key: &str) -> Option<&Value> {
        self.entries
            .get(key)
            .filter(|e| e.at.is_some_and(|at| at.elapsed() <= FRESH))
            .and_then(|e| e.value.as_ref())
    }

    fn eligible(&self, key: &str) -> bool {
        let prereq = self.prerequisites();
        if key == "soundMode" {
            return self.current("power") == Some(&json!(true))
                && self.signal.as_ref().is_some_and(|(signal, at)| {
                    at.elapsed() <= FRESH
                        && (signal == "PCM"
                            || signal.starts_with("PCM ")
                            || matches!(
                                signal.as_str(),
                                "DOLBY DIGITAL"
                                    | "DOLBY DIGITAL PLUS"
                                    | "DOLBY TRUEHD"
                                    | "DOLBY ATMOS"
                                    | "DTS"
                                    | "DTS-HD"
                                    | "DTS:X"
                            ))
                });
        }
        if key == "zone2Volume" {
            return self.current("zone2Power") == Some(&json!(true))
                && prereq.is_some_and(|p| {
                    p.zone2_present == Some(true)
                        && p.zone2_volume_variable == Some(true)
                        && p.zone2_volume_limit.is_some_and(|max| max.fract() == 0.0)
                        && self
                            .current("zone2Volume")
                            .and_then(Value::as_f64)
                            .is_some_and(|v| v.fract() == 0.0)
                });
        }
        if key == "zone2Power" {
            return prereq.is_some_and(|p| p.zone2_present == Some(true));
        }
        if key == "power" || key == "eco" {
            return true;
        }
        if key.starts_with("zone2") {
            if key == "zone2Input" {
                return self.current("zone2Power") == Some(&json!(true))
                    && prereq.is_some_and(|p| {
                        p.zone2_present == Some(true)
                            && p.zone2_sources.as_ref().is_some_and(|v| !v.is_empty())
                    });
            }
            return self.current("zone2Power") == Some(&json!(true));
        }
        if self.current("power") != Some(&json!(true)) {
            return false;
        }
        let mode = self.current("soundMode").and_then(Value::as_str);
        let direct = mode.is_none_or(|m| m.contains("DIRECT"));
        let audyssey_mode =
            !direct && mode.is_some_and(|m| !m.contains("VIRTUAL") && !m.contains("DTS:X"));
        let audyssey_on = self
            .current("multEq")
            .and_then(Value::as_str)
            .is_some_and(|v| v != "OFF");
        match key {
            "input" => prereq
                .and_then(|p| p.main_sources.as_ref())
                .is_some_and(|v| !v.is_empty()),
            "bass" | "treble" => {
                !direct
                    && prereq.is_some_and(|p| {
                        p.tone_available == Some(true) && p.tone_enabled == Some(true)
                    })
                    && self.current("toneEnabled") == Some(&json!(true))
                    && self.current("dynEq") == Some(&json!(false))
            }
            "toneEnabled" => {
                !direct
                    && prereq.is_some_and(|p| p.tone_available == Some(true))
                    && self.current("dynEq") == Some(&json!(false))
            }
            "cinemaEq" => mode.is_some_and(|m| m.contains("DOLBY") || m.contains("DTS")),
            "multEq" => audyssey_mode && prereq.is_some_and(|p| p.mult_eq_available == Some(true)),
            "dynEq" => {
                audyssey_mode
                    && audyssey_on
                    && prereq.is_some_and(|p| p.dynamic_eq_available == Some(true))
            }
            "dynVol" => {
                audyssey_mode
                    && audyssey_on
                    && prereq.is_some_and(|p| p.dynamic_volume_available == Some(true))
            }
            "refOffset" => {
                audyssey_mode
                    && audyssey_on
                    && self.current("dynEq") == Some(&json!(true))
                    && prereq.is_some_and(|p| p.reference_level_available == Some(true))
            }
            _ => true,
        }
    }

    pub fn document(&self) -> Value {
        let mut map = serde_json::Map::new();
        for key in KEYS {
            let e = &self.entries[*key];
            let live = self.current(key);
            let supported = self.qualified && live.is_some();
            let writable = supported && self.eligible(key) && e.pending.is_none();
            let mut desc = json!({"support": if supported { "supported" } else { "unknown" }, "value": live, "updatedAt": if live.is_some() {e.at_ms} else {None}, "writable": writable, "options": if supported { self.options(key, live) } else { Vec::new() }, "pending": e.pending.as_ref().map(|id| json!({"requestId":id})), "result":e.result});
            if !supported {
                desc["reason"] = json!(if *key == "subwoofer" {
                    "Global subwoofer level protocol has not been qualified"
                } else if !self.qualified {
                    "Receiver model has not been identified for this connection"
                } else {
                    "Waiting for a current receiver reply"
                });
            } else if !writable {
                desc["reason"] = json!(if e.pending.is_some() {
                    "A change is pending"
                } else if *key == "soundMode" {
                    "Waiting for a recognized current input signal"
                } else if *key == "zone2Volume" {
                    "Zone 2 volume limit and fixed-volume setting have not been read"
                } else {
                    "Unavailable in the current receiver mode"
                });
            }
            if *key == "soundMode" && writable {
                desc["reason"] = json!(
                    "Stereo and Direct are documented for this model and input signal; the receiver does not report a mode list"
                );
            }
            match *key {
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
                        .filter(|_| self.eligible(key))
                    {
                        desc["min"] = json!(0);
                        desc["max"] = json!(max);
                        desc["step"] = json!(1);
                    }
                }
                _ => {}
            }
            map.insert((*key).to_owned(), desc);
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
        if !self.qualified || self.current(key).is_none() || !self.eligible(key) {
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
        let dependencies = [
            Some(key),
            (key.starts_with("zone2") && key != "zone2Power").then_some("zone2Power"),
            (!key.starts_with("zone2") && !matches!(key, "power" | "eco")).then_some("power"),
            matches!(
                key,
                "cinemaEq"
                    | "multEq"
                    | "dynEq"
                    | "dynVol"
                    | "refOffset"
                    | "bass"
                    | "treble"
                    | "toneEnabled"
            )
            .then_some("soundMode"),
            matches!(key, "dynEq" | "dynVol" | "refOffset").then_some("multEq"),
            matches!(key, "refOffset" | "bass" | "treble" | "toneEnabled").then_some("dynEq"),
            matches!(key, "bass" | "treble").then_some("toneEnabled"),
        ];
        let mut valid_until = Instant::now() + FRESH;
        for name in dependencies.into_iter().flatten() {
            let at = self
                .entries
                .get(name)
                .and_then(|e| e.at)
                .ok_or("Setting is unavailable")?;
            valid_until = valid_until.min(at + FRESH);
        }
        if key == "soundMode" {
            valid_until =
                valid_until.min(self.signal.as_ref().ok_or("Setting is unavailable")?.1 + FRESH);
        }
        if matches!(
            key,
            "input"
                | "zone2Power"
                | "zone2Input"
                | "zone2Volume"
                | "bass"
                | "treble"
                | "toneEnabled"
                | "multEq"
                | "dynEq"
                | "dynVol"
                | "refOffset"
        ) {
            valid_until = valid_until.min(
                self.prerequisites
                    .as_ref()
                    .ok_or("Setting is unavailable")?
                    .1
                    + FRESH,
            );
        }
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
            return if self.eligible(key) {
                vec![
                    json!({"value":"STEREO","label":"Stereo"}),
                    json!({"value":"DIRECT","label":"Direct"}),
                ]
            } else {
                Vec::new()
            };
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
        "power" | "toneEnabled" | "cinemaEq" | "dynEq" | "zone2Power" => {
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
        "input" | "soundMode" | "zone2Input" => current.cloned().into_iter().collect(),
        _ => Vec::new(),
    };
    vals.into_iter().map(|v| json!({"label": v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()), "value": v})).collect()
}

fn reply_value(reply: Reply) -> (&'static str, Value) {
    match reply {
        Reply::Power(v) => ("power", json!(v)),
        Reply::Source(v) => ("input", json!(v.as_str())),
        Reply::SoundMode(v) => ("soundMode", json!(v.as_str())),
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
    }
}

fn to_setting(key: &str, value: &Value) -> Option<Setting> {
    let text = || value.as_str();
    let tone = || {
        let db = value.as_f64()?;
        if db.fract() != 0.0 || !(-6.0..=6.0).contains(&db) {
            return None;
        }
        settings::Tone::new(db as i8)
    };
    Some(match key {
        "power" => Setting::Power(value.as_bool()?),
        "input" => Setting::Source(settings::Source::parse(text()?)?),
        "soundMode" => Setting::SoundMode(settings::SoundMode::parse(text()?)?),
        "quickSelect" => {
            Setting::QuickSelect(settings::QuickSelect::parse(&value.as_u64()?.to_string())?)
        }
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
    fn settings_are_session_qualified_and_have_independent_freshness() {
        let mut state = SettingsState::new(42);
        state.connected(1);
        state.observe(1, Reply::Power(true), Instant::now(), 100);
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
        state.observe(1, Reply::Zone2Power(true), Instant::now(), 1);
        state.observe(
            1,
            Reply::Zone2Volume(settings::ZoneVolume::new(35.0).unwrap()),
            Instant::now(),
            2,
        );
        let zone = &state.document()["zone2Volume"];
        assert_eq!(zone["value"], 35.0);
        assert_eq!(zone["writable"], false);
        assert!(zone.get("max").is_none());
    }
    #[test]
    fn controls_require_current_prerequisites_and_distinct_zone_choices() {
        let mut state = SettingsState::new(2);
        state.connected(1);
        state.qualify(Some("AVR-S760H"));
        let now = Instant::now();
        state.observe(1, Reply::Power(true), now, 1);
        state.mode(1, "STEREO".into(), now, 2);
        state.observe(1, Reply::DynamicEq(false), now, 3);
        state.observe(1, Reply::ToneEnabled(true), now, 4);
        state.observe(1, Reply::Bass(settings::Tone::new(0).unwrap()), now, 5);
        state.observe(1, Reply::Source(settings::Source::Tv), now, 6);
        state.observe(1, Reply::Zone2Power(true), now, 7);
        state.observe(1, Reply::Zone2Source(settings::Source::Cd), now, 8);
        assert_eq!(state.document()["bass"]["writable"], false);
        let mut p = settings::Prerequisites::default();
        p.tone_available = Some(true);
        p.tone_enabled = Some(true);
        p.zone2_present = Some(true);
        p.main_sources = Some(vec![settings::SourceChoice {
            id: settings::Source::Tv,
            name: "TV".into(),
        }]);
        p.zone2_sources = Some(vec![settings::SourceChoice {
            id: settings::Source::Cd,
            name: "CD".into(),
        }]);
        state.set_prerequisites(Some(p));
        let doc = state.document();
        assert_eq!(doc["bass"]["writable"], true);
        assert_eq!(doc["soundMode"]["writable"], false);
        assert!(state.prepare("soundMode", &json!("STEREO"), "2-1").is_err());
        assert_eq!(doc["input"]["options"][0]["value"], "TV");
        assert_eq!(doc["zone2Input"]["options"][0]["value"], "CD");
        assert!(state.prepare("zone2Input", &json!("TV"), "2-1").is_err());
        state.signal(1, "PCM ZERO".into(), now);
        let mode = &state.document()["soundMode"];
        assert_eq!(mode["writable"], true);
        assert_eq!(
            mode["options"],
            json!([{"value":"STEREO","label":"Stereo"},{"value":"DIRECT","label":"Direct"}])
        );
        assert!(state.prepare("soundMode", &json!("DIRECT"), "2-1").is_ok());
        state.context_changed(1, 5, "input");
        assert!(state.document()["input"]["value"].is_null());
        assert!(state.document()["soundMode"]["value"].is_null());
        assert!(state.prepare("soundMode", &json!("DIRECT"), "2-1").is_err());
        state.mode(1, "STEREO".into(), now, 10);
        state.signal(1, "PCM ZERO".into(), now);
        assert!(
            state
                .prepare("soundMode", &json!("PURE DIRECT"), "2-1")
                .is_err()
        );
        state.signal(1, "DSD".into(), now);
        assert_eq!(state.document()["soundMode"]["writable"], false);
        state.mode(1, "DIRECT".into(), now, 9);
        assert_eq!(state.document()["bass"]["writable"], false);
    }
}
