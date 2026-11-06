//! Panel dim: when the screensaver or an idle Home wants the whole screen dimmer, the panel is dimmed, not the picture
//! (`theme.oledcare.panelDim`, Settings > System > OLED care > Dim with the panel; design and reasons: `docs/research/panel-dim-result.md`).
//!
//! The panel's own light is LG's picture setting "OLED pixel brightness" (`picture.backlight`, 0 to 100 per picture mode), written through
//! the settings service exactly as Settings > Picture writes it. A frame drawn at full brightness can go to the display plane whole
//! (`Plan::sole_fullscreen` needs every item at 1.0), so a dimmed screen costs no composing; the GPU multiplier of `oledcare` stays for what
//! the panel cannot do (one box, one app).
//!
//! What this module is: a state machine with one call to the service in flight at a time. The loop gives it a [`Look`] at every tick (what
//! fraction of the user's value the screen wants, and whether something forbids a dim now) and carries out the [`Call`] it returns; the
//! service's answer comes back through [`PanelDim::answer`]. Time is passed in, so tests drive it with a stand-in service.
//!
//! - **Baseline.** The first time a dim is wanted the picture setting is read (value, and the picture mode, input and dynamic range it
//!   belongs to: the "dimension"). A dimension that is HDR or Dolby Vision, or an answer without one, is never written (no dim at all for
//!   the first; the GPU dim for the second, see [`PanelDim::carries`]).
//! - **Ramp.** One unit a write, at most one write every `panelDimStepSec` (2 s), toward `round(value x want)`, never below [`FLOOR`] and
//!   never above the user's value. The picture mode and input the baseline came with ([`call_dimension`]) go with every write, so a mode switch under the dim never makes
//!   a write land on another mode's value.
//! - **Journal.** Before the first write below the user's value, `paneldim.json` in the data directory holds the user's value, the value last
//!   written, the dimension and the compositor's pid; it is rewritten with each write and removed when the user's value is back. A run that
//!   dies leaves it: the next start of the compositor, and the helper's watch while the compositor is not running, put the value back
//!   (`recover`, `helper/app/src/paneldim.rs`).
//! - **Put back.** The user's exact value is written again on a key or the pointer, a screen that wants to be bright (the saver leaving,
//!   Home touched, an app in front), a video or a hardware video window, standby, the switch turned off, a write that failed and the exit
//!   of the compositor. Before it the setting is read: a value that is neither the one last written nor the user's was changed by someone
//!   else meanwhile, and is left alone (their change wins, the journal goes). Standby and exit skip that read (the caller is on its way
//!   out); a put back that fails is tried again with a growing wait, the journal staying.
//! - **While dimmed** the setting is read every [`POLL_NS`]: another value is someone else's change (left alone, no dim for
//!   [`USER_HOLD_NS`]), another dimension means the picture mode, input or range changed (put back at once).
//! - **Failure.** A read without a usable answer or a refused write before anything was dimmed switches the module off for [`LATCH_NS`]:
//!   [`PanelDim::carries`] is false and the GPU dim and the saver's own dim do the work as before. A refused write in the middle of a dim puts
//!   the value back and does the same.
//!
//! Nothing here touches LG's power state, the panel refresh (`refresh-guard-result.md`) or any service but `com.webos.service.settings`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::State;
use crate::luna::{FromBridge, Outbound, ToBridge};

/// The settings service, as the shell's `Sio` calls it (`rustui/ui/src/host/sio.rs`).
pub const READ_URI: &str = "luna://com.webos.service.settings/getSystemSettings";
pub const WRITE_URI: &str = "luna://com.webos.service.settings/setSystemSettings";

const SEC: u64 = 1_000_000_000;
/// The panel is never taken below this OLED pixel brightness (0 is a dark panel: a clock on the saver would not be seen), nor below the
/// user's own value when that is lower.
pub const FLOOR: u32 = 10;
/// While dimmed the setting is read this often.
pub const POLL_NS: u64 = 20 * SEC;
/// A baseline that has waited this long for its first write is read again: the user may have moved the value meanwhile.
pub const BASELINE_NS: u64 = 10 * SEC;
/// A call the service has not answered in this long has failed.
pub const CALL_TIMEOUT_NS: u64 = 3 * SEC;
/// After a key or the pointer no new dim begins for this long: the screen that asked for it is still on its way out.
pub const HOLD_NS: u64 = 3 * SEC;
/// After a failed baseline or dim write the module stays off this long.
pub const LATCH_NS: u64 = 10 * 60 * SEC;
/// After finding the setting changed by someone else no dim begins for this long.
pub const USER_HOLD_NS: u64 = 5 * 60 * SEC;
/// An HDR or Dolby Vision dimension is looked at again after this.
pub const HDR_RETRY_NS: u64 = 30 * SEC;
/// The wait after a failed put back starts at a second and doubles to this.
const RETRY_MAX_NS: u64 = 30 * SEC;

/// `theme.oledcare.panelDim` and `panelDimStepSec`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Dim with the panel (on unless the theme says otherwise).
    pub enabled: bool,
    /// The least time between two writes of a ramp.
    pub step_ns: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { enabled: true, step_ns: 2 * SEC }
    }
}

impl Settings {
    /// The keys of the `oledcare` object: `panelDim`, `panelDimStepSec` (1 to 60). Wrong types are the shipped value.
    pub fn from_object(o: &Map<String, Value>) -> Settings {
        let d = Settings::default();
        let num = |k: &str| o.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
        Settings {
            enabled: o.get("panelDim").and_then(Value::as_bool).unwrap_or(d.enabled),
            step_ns: num("panelDimStepSec").map_or(d.step_ns, |v| (v.clamp(1.0, 60.0) * SEC as f64) as u64),
        }
    }
}

/// Why a dim is not allowed (or not allowed to go on) right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    /// A video is on the screen (a hole, a hardware video window): the panel's light is the picture's.
    Video,
    /// The screen is off or going off.
    Standby,
}

impl Block {
    fn word(self) -> &'static str {
        match self {
            Block::Video => "video",
            Block::Standby => "standby",
        }
    }
}

/// What the loop knows at a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub now_ns: u64,
    /// The fraction of the user's value the screen wants: 1 for nothing, 0.4 for the saver at its dimmest.
    pub want: f32,
    pub block: Option<Block>,
}

impl Default for Look {
    fn default() -> Self {
        Look { now_ns: 0, want: 1.0, block: None }
    }
}

/// A call to make on the settings service (through the luna bridge).
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub uri: &'static str,
    pub payload: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Baseline,
    Poll,
    Verify,
    Write { value: u32, restore: bool },
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Baseline => "baseline",
            Kind::Poll => "poll",
            Kind::Verify => "verify",
            Kind::Write { restore: false, .. } => "write",
            Kind::Write { restore: true, .. } => "restore",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Flight {
    kind: Kind,
    since_ns: u64,
}

/// A value of the user's that is dimmed (or about to be).
#[derive(Clone, Debug)]
struct Held {
    /// The user's value, which is put back.
    orig: u32,
    /// The picture mode, input and range it belongs to, as the service named them.
    dimension: Value,
    /// The value last written (sent, whether or not it has been confirmed).
    written: u32,
    /// The value last confirmed by the service.
    confirmed: u32,
    /// When the service last answered about this value, a write's answer included: the next poll is [`POLL_NS`] after.
    checked_ns: u64,
    /// Taken from the journal of a run that died.
    recovered: bool,
}

impl Held {
    fn ours(&self, value: u32) -> bool {
        value == self.written || value == self.confirmed
    }
}

/// The picture setting as a read answered it.
#[derive(Debug, PartialEq)]
struct Reading {
    value: u32,
    dimension: Option<Value>,
}

fn reading(payload: &Value) -> Option<Reading> {
    let v = payload.pointer("/settings/backlight")?;
    let n = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    if !n.is_finite() || !(0.0..=100.0).contains(&n) {
        return None;
    }
    Some(Reading { value: n.round() as u32, dimension: payload.get("dimension").filter(|d| d.is_object()).cloned() })
}

/// Whether a picture dimension is an HDR or Dolby Vision one: the panel's light is then part of what the film is graded for.
pub fn hdr_dimension(d: &Value) -> bool {
    let range = d.get("dynamicRange").and_then(Value::as_str).unwrap_or("sdr");
    let mode = d.get("pictureMode").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
    !range.eq_ignore_ascii_case("sdr") || mode.contains("hdr") || mode.contains("dolby")
}

/// The part of a dimension the settings service takes in a call: `{pictureMode, input}`. A dimension that carries `dynamicRange` too, as the
/// service names it in its own answers, is refused by a read with "no result from DB" (TV round 20), so calls name these two only.
pub fn call_dimension(d: &Value) -> Value {
    let mut out = Map::new();
    for k in ["pictureMode", "input"] {
        if let Some(v) = d.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    if out.is_empty() { Value::Null } else { Value::Object(out) }
}

/// Whether two dimensions are the same picture mode, input and range, comparing what both name (a journal's has no `dynamicRange`).
pub fn same_dimension(a: &Value, b: &Value) -> bool {
    ["pictureMode", "input", "dynamicRange"].iter().all(|k| match (a.get(k), b.get(k)) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    })
}

/// The value the panel is taken to for `want` of the user's `orig`.
pub fn target(orig: u32, want: f32) -> u32 {
    let t = (orig as f32 * want.clamp(0.0, 1.0)).round() as u32;
    t.clamp(FLOOR.min(orig), orig)
}

/// The journal of a dim in progress, in the data directory (the helper reads the same file).
pub const JOURNAL: &str = "paneldim.json";

#[derive(Debug, PartialEq)]
struct Journal {
    orig: u32,
    written: u32,
    dimension: Value,
    pid: u32,
}

fn journal_read(path: &Path) -> Option<Journal> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let n = |k: &str| v.get(k).and_then(Value::as_u64).filter(|n| *n <= 100).map(|n| n as u32);
    Some(Journal {
        orig: n("orig")?,
        written: n("written")?,
        dimension: v.get("dimension").filter(|d| d.is_object()).cloned().unwrap_or(Value::Null),
        pid: v.get("pid").and_then(Value::as_u64).unwrap_or(0) as u32,
    })
}

pub struct PanelDim {
    pub settings: Settings,
    path: PathBuf,
    look: Look,
    flight: Option<Flight>,
    held: Option<Held>,
    /// Put the value back as soon as it can be, whatever the look says (a key, the exit, a journal found at the start).
    force: bool,
    force_why: &'static str,
    /// The put back is on its way out and does not read first (standby, exit).
    hurried: bool,
    /// The setting has been read since the put back began (or the read failed): the put back is written now.
    verified: bool,
    /// The compositor is leaving: nothing starts again.
    exited: bool,
    /// A bridge to the luna service is connected (set by the state at each tick): without one nothing can be asked and the GPU does the dim.
    bridge: bool,
    next_write_ns: u64,
    next_try_ns: u64,
    backoff_ns: u64,
    latched_until_ns: u64,
    hold_until_ns: u64,
    retry_at_ns: u64,
    hdr: bool,
    events: Vec<String>,
    pub writes: u64,
    pub reads: u64,
    pub restores: u64,
    pub failures: u64,
    last_error: String,
    pid: u32,
}

impl PanelDim {
    /// A panel dim with its journal in `data_dir`.
    pub fn new(data_dir: &Path) -> PanelDim {
        PanelDim {
            settings: Settings::default(),
            path: data_dir.join(JOURNAL),
            look: Look::default(),
            flight: None,
            held: None,
            force: false,
            force_why: "",
            hurried: false,
            verified: false,
            exited: false,
            bridge: true,
            next_write_ns: 0,
            next_try_ns: 0,
            backoff_ns: 0,
            latched_until_ns: 0,
            hold_until_ns: 0,
            retry_at_ns: 0,
            hdr: false,
            events: Vec::new(),
            writes: 0,
            reads: 0,
            restores: 0,
            failures: 0,
            last_error: String::new(),
            pid: std::process::id(),
        }
    }

    /// Whether the panel does the dimming now: the switch is on and the last try did not fail. The GPU dim (Home) and the saver's own dim (the
    /// Rust shell is told) stand aside while this holds, and take over the moment it does not.
    pub fn carries(&self) -> bool {
        self.settings.enabled && self.bridge && self.look.now_ns >= self.latched_until_ns
    }

    pub fn set_bridge(&mut self, connected: bool) {
        self.bridge = connected;
    }

    /// Whether the user's value is below where they put it, or a write that may have changed it is still unconfirmed.
    pub fn dimmed(&self) -> bool {
        self.held.as_ref().is_some_and(|h| h.written != h.orig || h.confirmed != h.orig)
    }

    pub fn busy(&self) -> bool {
        self.flight.is_some()
    }

    /// The lines the journal (of the compositor) is owed.
    pub fn take_events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.events)
    }

    /// `ctl status`'s `oledcare.panelDim`.
    pub fn status(&self) -> Value {
        let now = self.look.now_ns;
        json!({
            "enabled": self.settings.enabled,
            "carries": self.carries(),
            "dimmed": self.dimmed(),
            "orig": self.held.as_ref().map(|h| h.orig),
            "written": self.held.as_ref().map(|h| h.written),
            "inFlight": self.flight.map(|f| f.kind.word()),
            "want": self.look.want,
            "block": self.look.block.map(Block::word),
            "hdr": self.hdr,
            "latchedSec": self.latched_until_ns.saturating_sub(now) / SEC,
            "writes": self.writes,
            "reads": self.reads,
            "restores": self.restores,
            "failures": self.failures,
            "lastError": self.last_error,
        })
    }

    /// One tick: the look, and the call to make now, if any.
    pub fn step(&mut self, look: Look) -> Option<Call> {
        self.look = look;
        if let Some(f) = self.flight
            && look.now_ns.saturating_sub(f.since_ns) >= CALL_TIMEOUT_NS
        {
            self.flight = None;
            self.failed(f.kind, "the settings service did not answer");
        }
        self.decide()
    }

    /// A key or the pointer: whatever is dimmed is put back now, and nothing dims again for [`HOLD_NS`].
    pub fn input(&mut self, now_ns: u64) -> Option<Call> {
        self.look.now_ns = self.look.now_ns.max(now_ns);
        self.hold_until_ns = now_ns + HOLD_NS;
        if self.held.is_some() {
            self.force = true;
            self.force_why = "key or pointer";
            self.hurried = false;
            self.next_try_ns = 0;
        }
        self.decide()
    }

    /// The compositor is leaving: the user's value goes back without reading first, whatever is in flight.
    pub fn exit(&mut self, now_ns: u64) -> Option<Call> {
        self.look.now_ns = self.look.now_ns.max(now_ns);
        self.flight = None;
        self.exited = true;
        if self.held.is_some() {
            self.force = true;
            self.force_why = "exit";
            self.hurried = true;
            self.next_try_ns = 0;
        }
        self.decide()
    }

    /// At the start: a journal that a run left is read, and the value it dimmed is put back (read first: a value that someone changed since is
    /// left alone). The call to make, if there is a journal.
    pub fn recover(&mut self, now_ns: u64) -> Option<Call> {
        self.look.now_ns = now_ns;
        let j = journal_read(&self.path)?;
        if j.pid == self.pid {
            return None;
        }
        self.events.push(format!("paneldim: a journal of a run that ended (value {} under {}): putting it back", j.written, j.orig));
        self.held = Some(Held { orig: j.orig, dimension: j.dimension, written: j.written, confirmed: j.written, checked_ns: now_ns, recovered: true });
        self.force = true;
        self.force_why = "left by a run that ended";
        self.hurried = false;
        self.next_try_ns = 0;
        self.decide()
    }

    /// The service's answer to the call in flight, and the call that follows, if any.
    pub fn answer(&mut self, now_ns: u64, payload: &Value) -> Option<Call> {
        self.look.now_ns = self.look.now_ns.max(now_ns);
        let now = self.look.now_ns;
        let f = self.flight.take()?;
        match f.kind {
            Kind::Baseline => match reading(payload) {
                None => self.failed(f.kind, "no OLED pixel brightness in the answer"),
                Some(Reading { dimension: None, .. }) => self.failed(f.kind, "the answer names no picture mode"),
                Some(Reading { value, dimension: Some(d) }) => {
                    if hdr_dimension(&d) {
                        if !self.hdr {
                            self.events.push(format!(
                                "paneldim: {} is an HDR picture mode: the panel is left alone",
                                d.get("pictureMode").and_then(Value::as_str).unwrap_or("?")
                            ));
                        }
                        self.hdr = true;
                        self.retry_at_ns = now + HDR_RETRY_NS;
                    } else if value <= FLOOR {
                        self.hdr = false;
                        self.retry_at_ns = now + POLL_NS;
                    } else {
                        self.hdr = false;
                        self.held = Some(Held { orig: value, dimension: d, written: value, confirmed: value, checked_ns: now, recovered: false });
                    }
                }
            },
            Kind::Poll => match reading(payload) {
                None => self.failed(f.kind, "no OLED pixel brightness in the answer"),
                Some(r) => {
                    let Some(h) = self.held.as_mut() else { return self.decide() };
                    h.checked_ns = now;
                    if r.dimension.as_ref().is_some_and(|d| !same_dimension(d, &h.dimension)) {
                        self.events.push("paneldim: the picture mode, input or range changed under the dim".into());
                        self.force = true;
                        self.force_why = "picture mode changed";
                        self.hurried = true;
                        self.next_try_ns = 0;
                    } else if !h.ours(r.value) {
                        self.left_alone(now, r.value);
                    }
                }
            },
            Kind::Verify => {
                let Some(h) = self.held.as_mut() else { return self.decide() };
                match reading(payload) {
                    // not told, or told of a dimension that is not the one that was dimmed: nothing to compare, the put back goes
                    None => {}
                    Some(r) if r.dimension.as_ref().is_some_and(|d| !same_dimension(d, &h.dimension)) => {}
                    Some(r) if h.ours(r.value) => {}
                    Some(r) => {
                        self.left_alone(now, r.value);
                        return self.decide();
                    }
                }
                self.verified = true;
            }
            Kind::Write { value, restore } => {
                if payload.get("returnValue").and_then(Value::as_bool) == Some(false) {
                    self.failed(
                        f.kind,
                        &format!("the settings service refused {value}: {}", payload.get("errorText").and_then(Value::as_str).unwrap_or("no reason")),
                    );
                } else if let Some(h) = self.held.as_mut() {
                    h.confirmed = value;
                    h.checked_ns = now;
                    self.backoff_ns = 0;
                    if restore {
                        let (orig, why) = (h.orig, self.force_why);
                        self.finish(format!("paneldim: panel back at {orig} ({})", if why.is_empty() { self.want_why() } else { why.to_string() }));
                        self.restores += 1;
                    } else {
                        self.next_write_ns = now + self.settings.step_ns;
                    }
                }
            }
        }
        self.decide()
    }

    /// The value is not the one that was written and not the user's: someone changed it meanwhile. It stays as it is.
    fn left_alone(&mut self, now: u64, value: u32) {
        self.events.push(format!("paneldim: OLED pixel brightness is {value}, changed by someone else while dimmed: left alone"));
        self.drop_held();
        self.hold_until_ns = self.hold_until_ns.max(now + USER_HOLD_NS);
    }

    fn drop_held(&mut self) {
        self.held = None;
        self.force = false;
        self.hurried = false;
        self.verified = false;
        self.backoff_ns = 0;
        let _ = std::fs::remove_file(&self.path);
    }

    fn finish(&mut self, line: String) {
        self.events.push(line);
        self.drop_held();
    }

    fn failed(&mut self, kind: Kind, why: &str) {
        let now = self.look.now_ns;
        self.failures += 1;
        self.last_error = format!("{}: {why}", kind.word());
        self.events.push(format!("paneldim: {} failed: {why}", kind.word()));
        match kind {
            Kind::Baseline => self.latch(now),
            // a read that fails while dimmed is asked again at the next poll; the dim goes on
            Kind::Poll => {
                if let Some(h) = self.held.as_mut() {
                    h.checked_ns = now;
                }
            }
            // the put back goes on without the read (`put_back` sees the read has been tried)
            Kind::Verify => self.verified = true,
            Kind::Write { restore: false, .. } => {
                if let Some(h) = self.held.as_mut() {
                    h.written = h.confirmed;
                }
                self.latch(now);
                if self.held.as_ref().is_some_and(|h| h.confirmed != h.orig) {
                    self.force = true;
                    self.force_why = "a write failed";
                    self.hurried = false;
                }
            }
            Kind::Write { restore: true, .. } => {
                if let Some(h) = self.held.as_mut() {
                    h.written = h.confirmed;
                }
                self.backoff_ns = (self.backoff_ns * 2).clamp(SEC, RETRY_MAX_NS);
                self.next_try_ns = now + self.backoff_ns;
            }
        }
    }

    fn latch(&mut self, now: u64) {
        self.latched_until_ns = now + LATCH_NS;
        self.events.push(format!("paneldim: off for {} min: the GPU dim does the work", LATCH_NS / (60 * SEC)));
    }

    fn want_why(&self) -> String {
        if !self.settings.enabled {
            "switched off".into()
        } else if !self.bridge || self.look.now_ns < self.latched_until_ns {
            "failed".into()
        } else if let Some(b) = self.look.block {
            b.word().into()
        } else {
            "the screen is bright again".into()
        }
    }

    fn read_call(&self, with_dimension: bool) -> Call {
        let mut payload = json!({"category": "picture", "keys": ["backlight"]});
        if let (true, Some(h)) = (with_dimension, &self.held)
            && let d @ Value::Object(_) = call_dimension(&h.dimension)
        {
            payload["dimension"] = d;
        }
        Call { uri: READ_URI, payload }
    }

    fn start(&mut self, kind: Kind, call: Call) -> Option<Call> {
        match kind {
            Kind::Write { .. } => self.writes += 1,
            _ => self.reads += 1,
        }
        self.flight = Some(Flight { kind, since_ns: self.look.now_ns });
        Some(call)
    }

    fn write(&mut self, value: u32, restore: bool) -> Option<Call> {
        let h = self.held.as_mut()?;
        h.written = value;
        let dimension = call_dimension(&h.dimension);
        let (orig, first) = (h.orig, !restore && h.confirmed == h.orig);
        if !restore {
            self.save_journal(value);
            if first {
                self.events.push(format!("paneldim: dimming the panel from {orig} (the screen asked for {:.2})", self.look.want));
            }
        }
        let mut payload = json!({"category": "picture", "settings": {"backlight": value.to_string()}});
        if dimension.is_object() {
            payload["dimension"] = dimension;
        }
        self.start(Kind::Write { value, restore }, Call { uri: WRITE_URI, payload })
    }

    fn save_journal(&mut self, written: u32) {
        let Some(h) = &self.held else { return };
        let body = json!({"orig": h.orig, "written": written, "dimension": call_dimension(&h.dimension), "pid": self.pid, "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)});
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, body.to_string()).and_then(|_| std::fs::rename(&tmp, &self.path)).is_err() {
            self.last_error = "the journal could not be written".into();
        }
    }

    /// What the state wants done next, one call at most.
    fn decide(&mut self) -> Option<Call> {
        if self.flight.is_some() {
            return None;
        }
        if self.exited && self.held.is_none() {
            return None;
        }
        let Look { now_ns: now, want, block } = self.look;
        let s = self.settings;
        let latched = now < self.latched_until_ns;
        if let Some(h) = &self.held {
            let (orig, written, untouched) = (h.orig, h.written, h.written == h.orig && h.confirmed == h.orig);
            let aim = target(orig, want);
            let must = self.force || !s.enabled || latched || block.is_some() || aim > written;
            if untouched {
                // nothing was written: nothing to put back. No dim begins on a screen that does not want one, and a baseline that has
                // waited longer than a fresh read is taken again (the user may have moved the value meanwhile)
                let stale = now.saturating_sub(h.checked_ns) >= BASELINE_NS;
                if must || want >= 1.0 || now < self.hold_until_ns || stale {
                    self.drop_held();
                    return None;
                }
            } else if must {
                return self.put_back(now);
            }
            if aim < written && now >= self.next_write_ns {
                return self.write(written - 1, false);
            }
            if aim == written && written == orig {
                return None;
            }
            if !untouched && self.held.as_ref().is_some_and(|h| now.saturating_sub(h.checked_ns) >= POLL_NS) {
                return self.start(Kind::Poll, self.read_call(false));
            }
            return None;
        }
        if s.enabled && self.bridge && !self.exited && !latched && block.is_none() && want < 1.0 && now >= self.hold_until_ns && now >= self.retry_at_ns {
            return self.start(Kind::Baseline, self.read_call(false));
        }
        None
    }

    /// The user's value goes back: after a read of the setting, unless the caller is leaving.
    fn put_back(&mut self, now: u64) -> Option<Call> {
        if now < self.next_try_ns {
            return None;
        }
        let hurried = self.hurried || self.look.block == Some(Block::Standby);
        let h = self.held.as_ref()?;
        let (orig, recovered) = (h.orig, h.recovered);
        if hurried || self.verified { self.write(orig, true) } else { self.start(Kind::Verify, self.read_call(recovered)) }
    }
}

/// The state's side: what the screen wants of the panel, the calls carried to the luna bridge, and the answers carried back.
impl State {
    /// What the screen wants of the panel as a fraction of the user's value: the Rust shell's saver at its own level while it says it is not
    /// drawing that dim itself (`saverState.carry`), else Home's ease while the care is on, else nothing (1).
    pub(crate) fn panel_want(&self) -> f32 {
        if let Some(w) = self.panel_want_ctl {
            return w.clamp(0.0, 1.0);
        }
        let care = &self.scene.care;
        if care.saver {
            let st = &self.shell_rust.ui_state;
            let up = st.get("saver").and_then(Value::as_bool) == Some(true);
            let carry = st.pointer("/saverState/carry").and_then(Value::as_bool) == Some(true);
            return match st.pointer("/saverState/level").and_then(Value::as_f64) {
                Some(level) if up && carry => level as f32,
                _ => 1.0,
            };
        }
        care.home_want()
    }

    /// One look at the panel dim, after the frame of the tick: the screen's want and what forbids a dim, the call it makes, and the Rust
    /// shell told whether the panel carries the saver's dim.
    pub fn panel_dim_tick(&mut self, now_ns: u64) {
        let want = self.panel_want();
        let video = self.video_playing(now_ns);
        let block = if !self.luna_state.power.rendering {
            Some(Block::Standby)
        } else if video {
            Some(Block::Video)
        } else {
            None
        };
        let call = self.panel.step(Look { now_ns, want, block });
        self.panel_send(call);
        if self.shell_rust.answers("saver") {
            let carry = self.panel.carries();
            self.shell_rust.push_if_changed("paneldim", json!({"carry": carry}));
            // The saver does not draw over a video: told apart from the foreground topic, which an app's own video context fills, and without
            // the panel's staleness (`State::video_on_screen`).
            let shown = self.video_on_screen();
            self.shell_rust.push_if_changed("playing", json!({"video": shown}));
        }
        for line in self.panel.take_events() {
            self.journal(line);
        }
    }

    /// The screen is going off (tvpower's prepare-off was just taken): the put back goes out now, ahead of the answer that lets the TV power
    /// down, not at the next tick.
    pub fn panel_standby(&mut self) {
        let now = crate::output::monotonic_ns();
        let want = self.panel_want();
        let call = self.panel.step(Look { now_ns: now, want, block: Some(Block::Standby) });
        self.panel_send(call);
    }

    /// A key or the pointer (`Scene::input` and the panel).
    pub fn care_input(&mut self) {
        self.care_input_with(true);
    }

    /// [`State::care_input`], with or without the wake of LG's saver: the Power key asks for standby and does not wake the screen it is
    /// about to turn off.
    pub fn care_input_with(&mut self, wake_saver: bool) {
        let now = crate::output::monotonic_ns();
        self.nooperation_activity();
        self.vwin_input(now);
        if wake_saver {
            self.wake_lg_saver();
        }
        self.scene.input(now);
        let call = self.panel.input(now);
        self.panel_send(call);
    }

    fn panel_send(&mut self, call: Option<Call>) {
        let Some(c) = call else { return };
        let token = self.luna_token();
        self.panel_token = Some(token);
        self.luna.send(ToBridge::Outbound(Outbound { token, uri: c.uri.into(), payload: c.payload, subscribe: false }));
    }

    /// The answer to the panel dim's call, when `token` is its: true when it was.
    pub fn panel_answer(&mut self, token: u64, payload: &Value) -> bool {
        if self.panel_token != Some(token) {
            return false;
        }
        self.panel_token = None;
        let call = self.panel.answer(crate::output::monotonic_ns(), payload);
        self.panel_send(call);
        for line in self.panel.take_events() {
            self.journal(line);
        }
        true
    }

    /// At the start, once the bridge is up: a journal that a run left puts its value back.
    pub fn panel_recover(&mut self) {
        self.panel.settings = self.scene.care.settings.paneldim;
        let call = self.panel.recover(crate::output::monotonic_ns());
        self.panel_send(call);
        for line in self.panel.take_events() {
            self.journal(line);
        }
    }

    /// At the exit: the user's value goes back and the answer is waited for (up to `wait`), the bridge being polled here since the loop is over.
    pub fn panel_exit(&mut self, wait: std::time::Duration) {
        let call = self.panel.exit(crate::output::monotonic_ns());
        self.panel_send(call);
        let end = std::time::Instant::now() + wait;
        while self.panel.busy() && std::time::Instant::now() < end {
            for m in self.luna.poll() {
                if let FromBridge::Event(e) = m {
                    self.panel_answer(e.token, &e.payload);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        for line in self.panel.take_events() {
            log::info!("{line}");
        }
    }
}

#[cfg(test)]
mod tests;
