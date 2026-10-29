//! The volume and mute display's model (`panels/VolumeOsd.qml` of the QML shell, the hold-target rule of 8204d58; the mocks
//! `docs/mocks/11-volume-osd`): pure, driven by the channel's `audio` topic, the helper's `receiver` topic, the remote's keys as
//! the audio service reports them, and time. The surface ([`super::volume_surface`]) draws [`View`].
//!
//! Whose volume it shows: the receiver's own level (the helper's `receiver` topic, the Denon's push) while the sound leaves over
//! HDMI ARC or optical and the Denon answers; the TV's own (the audio service) while the TV's speakers, headphones or Bluetooth
//! are the output or the receiver is off, unreachable or switched off. A reading that is not in date is never shown; a key after a
//! quiet spell waits `decide_ms` for a receiver that may answer before the TV's counter is shown.
//!
//! The number is the target and the receiver is driven to it: a tap is one receiver step (0.5); a held key moves the target by time
//! (`rate_start` steps a second, rising by `rate_accel` a second to `rate_max`), never past the receiver's limit; the target is
//! posted to the helper at most every `send_ms` (`POST /v1/receiver/volume`), and the bar keeps it until the receiver reports that
//! level or `settle_ms` has passed since the last key.
//!
//! It opens on a change the audio service reports and on a new receiver reading, for `hold_ms` after the last. While the sound is
//! muted a small mark stays at the top and moves every `hop_ms` (a still mark is what an OLED must not hold), holding still while
//! a panel is up. Never in the lower third or the middle of the picture.

use super::format;
use crate::kit::screen::{Effect, Input, Model};
use serde_json::{Value, json};

/// Whose volume is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Receiver,
    Tv,
    /// The receiver may answer and has not yet.
    Wait,
    /// The TV has not said where the sound goes and the receiver does not answer.
    None,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Receiver => "receiver",
            Source::Tv => "tv",
            Source::Wait => "wait",
            Source::None => "none",
        }
    }
}

/// What the surface draws.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// The bar is up (the pill, with a source that is decided).
    pub up: bool,
    pub source: Source,
    /// The level as a whole number, or "" when there is none to show.
    pub level: String,
    pub fraction: f32,
    pub muted: bool,
    /// The format's mark (`atmos`, `dolby`, `dtsx`, `dts`) and its words, or "".
    pub logo: String,
    pub format: String,
    /// The mute mark stays shown, at this place of its round.
    pub mark_up: bool,
    pub mark_place: usize,
    pub over_video: bool,
    pub avoid_top_right: bool,
    /// Nothing to read: "Receiver not answering".
    pub offline: bool,
}

impl View {
    /// Something of the display is on screen: the layer that holds it is drawn for it.
    pub fn live(&self) -> bool {
        self.up || self.mark_up
    }
}

/// The TV's own volume as the audio service reports it (`kit/TvVolume.qml`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tv {
    pub output: String,
    pub volume: i32,
    pub volume_max: i32,
    pub muted: bool,
    pub cause: String,
    pub reports: u32,
}

impl Tv {
    pub fn known(&self) -> bool {
        !self.output.is_empty()
    }

    /// The sound leaves over HDMI ARC or optical: the receiver is in the way.
    pub fn arc(&self) -> bool {
        self.output.contains("external_arc") || self.output == "external_optical"
    }

    /// Takes a push of the `audio` topic; `true` when it is a change (a key, a mute, an app or the receiver moving the volume),
    /// never the state read at the start, which LG's popup does not open for.
    fn apply(&mut self, doc: &Value) -> bool {
        let Some(v) = doc.get("volumeStatus") else { return false };
        let was = (self.reports > 0).then_some((self.volume, self.muted));
        if let Some(n) = v.get("volume").and_then(Value::as_i64) {
            self.volume = n as i32;
        }
        if let Some(n) = v.get("maxVolume").and_then(Value::as_i64).filter(|n| *n > 0) {
            self.volume_max = n as i32;
        }
        if let Some(m) = v.get("muteStatus").and_then(Value::as_bool) {
            self.muted = m;
        }
        if let Some(o) = v.get("soundOutput").and_then(Value::as_str) {
            self.output = o.to_string();
        }
        self.cause = v.get("cause").and_then(Value::as_str).unwrap_or("").to_string();
        self.reports += 1;
        !self.cause.is_empty() || was.is_some_and(|(vol, m)| vol != self.volume || m != self.muted)
    }
}

/// The timings, in milliseconds, as the QML's properties.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub hold_ms: u64,
    pub stale_ms: u64,
    pub decide_ms: u64,
    pub hop_ms: u64,
    pub grace_ms: u64,
    pub settle_ms: u64,
    pub hold_gap_ms: u64,
    pub send_ms: u64,
    pub rate_start: f64,
    pub rate_accel: f64,
    pub rate_max: f64,
    pub receiver_step: f64,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            hold_ms: 2500,
            stale_ms: 6000,
            decide_ms: 250,
            hop_ms: 10000,
            grace_ms: 1500,
            settle_ms: 1500,
            hold_gap_ms: 200,
            send_ms: 100,
            rate_start: 6.0,
            rate_accel: 4.0,
            rate_max: 16.0,
            receiver_step: 0.5,
        }
    }
}

/// The tag the model's `GET /v1/state/receiver` confirmation answers come back with.
pub const CONFIRM_TAG: u64 = 0x5641;

#[derive(Clone, Debug)]
pub struct Volume {
    pub timing: Timing,
    pub enabled: bool,
    tv: Tv,
    helper_up: bool,
    /// The helper's newest copy of the `receiver` topic and when it came (the event's, or a confirmation's if newer).
    rc: Option<Value>,
    rc_at: u64,
    stale: bool,
    expiry: Option<u64>,
    asking: bool,
    ask_left: u8,
    ask_due: Option<u64>,
    // the keys' answer
    keyed: bool,
    opt_abs: f64,
    opt_mute: i32,
    last_key_at: u64,
    last_dir: i32,
    held_since: u64,
    owed: f64,
    sent_abs: f64,
    sent_at: u64,
    settle_due: Option<u64>,
    hold_due: Option<u64>,
    send_due: Option<u64>,
    // the display
    pill: bool,
    hide_due: Option<u64>,
    deciding: bool,
    decide_due: Option<u64>,
    nudged_at: u64,
    level_key: String,
    muted_rx: bool,
    grace_due: Option<u64>,
    // the mark
    mark_place: usize,
    hop_due: Option<u64>,
    pub still: bool,
    pub avoid_top_right: bool,
    over_video: bool,
    changed: bool,
}

impl Default for Volume {
    fn default() -> Self {
        Self::new(Timing::default())
    }
}

impl Volume {
    pub fn new(timing: Timing) -> Self {
        Self {
            timing,
            enabled: true,
            tv: Tv::default(),
            helper_up: false,
            rc: None,
            rc_at: 0,
            stale: true,
            expiry: None,
            asking: false,
            ask_left: 1,
            ask_due: None,
            keyed: false,
            opt_abs: -1.0,
            opt_mute: -1,
            last_key_at: 0,
            last_dir: 0,
            held_since: 0,
            owed: 0.0,
            sent_abs: -1.0,
            sent_at: 0,
            settle_due: None,
            hold_due: None,
            send_due: None,
            pill: false,
            hide_due: None,
            deciding: false,
            decide_due: None,
            nudged_at: 0,
            level_key: String::new(),
            muted_rx: false,
            grace_due: None,
            mark_place: 0,
            hop_due: None,
            still: false,
            avoid_top_right: false,
            over_video: false,
            changed: false,
        }
    }

    // ---- what is read

    fn rc_f64(&self, key: &str) -> Option<f64> {
        self.rc.as_ref()?.get(key)?.as_f64()
    }

    fn rc_bool(&self, key: &str) -> Option<bool> {
        self.rc.as_ref()?.get(key)?.as_bool()
    }

    fn rc_str(&self, key: &str) -> &str {
        self.rc.as_ref().and_then(|r| r.get(key)).and_then(Value::as_str).unwrap_or("")
    }

    /// The receiver's reading may be shown: the helper is up, the copy says fresh and reachable, and its stamp is in date.
    fn fresh(&self) -> bool {
        self.helper_up && self.rc.is_some() && self.rc_bool("reachable") == Some(true) && self.rc_bool("fresh") == Some(true) && !self.stale
    }

    fn tv_output(&self) -> bool {
        self.tv.known() && !self.tv.arc()
    }

    fn receiver_on(&self) -> bool {
        self.fresh() && self.rc_str("power") != "standby"
    }

    fn receiver_may(&self) -> bool {
        self.helper_up
            && self.rc.is_some()
            && self.rc_bool("enabled") != Some(false)
            && self.rc_bool("reachable") != Some(false)
            && self.rc_str("power") != "standby"
    }

    pub fn source(&self) -> Source {
        if self.tv_output() {
            Source::Tv
        } else if self.receiver_on() {
            Source::Receiver
        } else if self.receiver_may() && self.deciding {
            Source::Wait
        } else if self.tv.known() {
            Source::Tv
        } else {
            Source::None
        }
    }

    fn via_receiver(&self) -> bool {
        self.source() == Source::Receiver
    }

    fn read_abs(&self) -> f64 {
        self.rc_f64("volumeAbs").unwrap_or(-1.0)
    }

    /// The receiver's own limit (MVMAX), when the helper has heard it.
    fn limit(&self) -> f64 {
        match self.rc_f64("volumeMax") {
            Some(m) if m > 0.0 => m.min(98.0),
            _ => 98.0,
        }
    }

    fn use_key(&self) -> bool {
        self.keyed && self.via_receiver() && self.opt_abs >= 0.0
    }

    fn abs(&self) -> f64 {
        if !self.via_receiver() {
            -1.0
        } else if self.use_key() {
            self.opt_abs
        } else {
            self.read_abs()
        }
    }

    pub fn muted(&self) -> bool {
        match self.source() {
            Source::Receiver => {
                if self.keyed && self.opt_mute >= 0 {
                    self.opt_mute == 1
                } else {
                    self.rc_bool("mute").unwrap_or(false)
                }
            }
            Source::Tv => self.tv.muted,
            _ => false,
        }
    }

    fn muted_shown(&self) -> bool {
        if self.source() == Source::Tv { self.tv.muted } else { self.muted_rx }
    }

    pub fn view(&self) -> View {
        let source = self.source();
        let abs = self.abs();
        let (level, fraction) = match source {
            Source::Receiver => {
                if abs >= 0.0 {
                    (format!("{}", abs.round() as i64), (abs / 98.0).clamp(0.0, 1.0) as f32)
                } else {
                    (String::new(), 0.0)
                }
            }
            Source::Tv => (self.tv.volume.to_string(), (self.tv.volume as f32 / self.tv.volume_max.max(1) as f32).clamp(0.0, 1.0)),
            _ => (String::new(), 0.0),
        };
        let muted = self.muted();
        let (logo, format) = if source == Source::Receiver && !muted {
            let l = format::logo(self.rc_str("soundMode"), self.rc_str("signal"));
            (l.to_string(), if l.is_empty() { String::new() } else { format::describe(self.rc_str("soundMode"), self.rc_str("signal")) })
        } else {
            (String::new(), String::new())
        };
        let mark_up = self.enabled && self.muted_shown() && !self.pill;
        View {
            up: self.pill && source != Source::Wait,
            source,
            level,
            fraction,
            muted,
            logo,
            format,
            mark_up,
            mark_place: self.mark_place,
            over_video: self.over_video,
            avoid_top_right: self.avoid_top_right,
            offline: self.pill && source == Source::None,
        }
    }

    pub fn mark_spots(&self) -> Vec<(i32, i32)> {
        mark_spots(self.over_video, self.avoid_top_right)
    }

    // ---- the keys' answer before the receiver's

    fn end_keyed(&mut self) {
        self.keyed = false;
        self.sent_abs = -1.0;
        self.settle_due = None;
        self.changed = true;
    }

    /// The receiver's reading says what the keys made it: the bar takes the reading from here.
    fn confirm(&mut self) {
        if !self.keyed || self.rc.is_none() || self.hold_due.is_some() {
            return;
        }
        if self.opt_abs >= 0.0 && (self.read_abs() - self.opt_abs).abs() >= 0.25 {
            return;
        }
        if self.opt_mute >= 0 && self.rc_bool("mute").unwrap_or(false) != (self.opt_mute == 1) {
            return;
        }
        self.end_keyed();
    }

    /// How many receiver steps this report is worth: one for a tap; for a report of a held key the time since the last times the rate.
    fn steps(&mut self, now: u64, dir: i32) -> f64 {
        if dir != self.last_dir || self.last_key_at == 0 || now.saturating_sub(self.last_key_at) > self.timing.hold_gap_ms {
            self.held_since = 0;
            self.owed = 0.0;
            return 1.0;
        }
        if self.held_since == 0 {
            self.held_since = self.last_key_at;
        }
        let t = &self.timing;
        let rate = (t.rate_start + t.rate_accel * (now - self.held_since) as f64 / 1000.0).min(t.rate_max);
        self.owed += rate * (now - self.last_key_at) as f64 / 1000.0;
        let whole = self.owed.floor();
        self.owed -= whole;
        whole
    }

    fn key(&mut self, now: u64, out: &mut Vec<Effect>) {
        let cause = self.tv.cause.to_lowercase();
        let (up, down) = (cause.contains("up"), cause.contains("down"));
        let base = if self.keyed && self.opt_abs >= 0.0 { self.opt_abs } else { self.read_abs() };
        if base >= 0.0 && (up || down) {
            let dir = if up { 1 } else { -1 };
            let next = base + dir as f64 * self.steps(now, dir) * self.timing.receiver_step;
            self.opt_abs = if up { next.min(self.limit().max(base)) } else { next.max(0.0) };
            self.last_key_at = now;
            self.last_dir = dir;
            self.hold_due = Some(now + self.timing.hold_gap_ms);
        }
        // the Denon cancels its mute on a volume key: a report with no direction keeps what is believed
        let was = if self.keyed && self.opt_mute >= 0 { self.opt_mute == 1 } else { self.rc_bool("mute").unwrap_or(false) };
        self.opt_mute = if up || down {
            0
        } else if was {
            1
        } else {
            0
        };
        self.keyed = true;
        self.settle_due = Some(now + self.timing.settle_ms);
        self.send(now, out);
    }

    /// The target to the helper: on the key if the last post is old enough, else when it is.
    fn send(&mut self, now: u64, out: &mut Vec<Effect>) {
        if !self.via_receiver() || self.opt_abs < 0.0 || self.opt_abs == self.sent_abs {
            return;
        }
        if now.saturating_sub(self.sent_at) < self.timing.send_ms {
            if self.send_due.is_none() {
                self.send_due = Some(self.sent_at + self.timing.send_ms);
            }
            return;
        }
        self.sent_abs = self.opt_abs;
        self.sent_at = now;
        self.send_due = None;
        out.push(Effect::Post { path: "/v1/receiver/volume".into(), body: json!({"level": self.opt_abs}) });
    }

    /// The mute key's report has no direction, but the TV's own mute flag moves with it: when it differs from what the receiver last
    /// said, the key was the mute key and the receiver follows in 0.5 to 1 s, so the bar shows it now.
    fn mute_key(&mut self, now: u64) {
        if !self.via_receiver() || self.rc.is_none() || self.tv.muted == self.rc_bool("mute").unwrap_or(false) {
            return;
        }
        self.opt_mute = if self.tv.muted { 1 } else { 0 };
        self.keyed = true;
        self.settle_due = Some(now + self.timing.settle_ms);
        self.changed = true;
    }

    // ---- showing

    fn show(&mut self, now: u64) {
        if !self.enabled {
            return;
        }
        self.pill = true;
        self.hide_due = Some(now + self.timing.hold_ms);
        self.changed = true;
    }

    /// The audio service reported a change: the bar now. If the receiver may be the one that changed, the helper is asked to read it.
    fn cue(&mut self, now: u64, out: &mut Vec<Effect>) {
        if !self.enabled {
            return;
        }
        if !self.tv_output() && self.receiver_may() {
            self.deciding = !self.receiver_on();
            if self.deciding {
                self.decide_due = Some(now + self.timing.decide_ms);
            }
            if now.saturating_sub(self.nudged_at) > 300 {
                self.nudged_at = now;
                out.push(Effect::Post { path: "/v1/receiver/nudge".into(), body: json!({}) });
            }
            if self.stale {
                self.ask(out);
            }
        }
        if self.via_receiver() {
            self.key(now, out);
        }
        out.push(Effect::Log(format!("[VolumeOsd] cue: the bar {}", if self.pill { "is up" } else { "appears now" })));
        self.show(now);
    }

    // ---- the receiver's reading and its age

    /// Asks the helper for the topic's newest copy.
    fn ask(&mut self, out: &mut Vec<Effect>) {
        if self.asking || !self.helper_up {
            return;
        }
        self.asking = true;
        out.push(Effect::Fetch { tag: CONFIRM_TAG, path: "/v1/state/receiver".into() });
    }

    /// Whether the copy is in date, and the expiry that makes it not.
    fn evaluate(&mut self, now: u64) {
        self.expiry = None;
        let Some(rc) = &self.rc else {
            self.stale = true;
            return;
        };
        let at = rc.get("updatedAt").and_then(Value::as_f64).unwrap_or(0.0);
        let remaining = self.timing.stale_ms as f64 - (now as f64 - at);
        let fresh = rc.get("fresh").and_then(Value::as_bool) == Some(true) && rc.get("reachable").and_then(Value::as_bool) == Some(true);
        self.stale = !(fresh && at.is_finite() && at > 0.0 && remaining > 0.0 && remaining <= self.timing.stale_ms as f64 + 1000.0);
        if self.stale {
            return;
        }
        self.expiry = Some(now + remaining.min(self.timing.stale_ms as f64).ceil().max(1.0) as u64);
    }

    /// A reading that is in date: a change of level or of mute against the last one shows the bar, while it is the receiver that is read.
    fn read(&mut self, now: u64) {
        self.deciding = false;
        self.decide_due = None;
        let next_key = format!("{}|{}", self.read_abs(), self.rc_bool("mute").unwrap_or(false));
        if self.level_key != next_key && !self.level_key.is_empty() && !self.tv_output() {
            self.show(now);
        }
        self.level_key = next_key;
        self.muted_rx = self.rc_bool("mute").unwrap_or(false);
        self.grace_due = None;
        self.confirm();
        self.changed = true;
    }

    fn take_receiver(&mut self, doc: &Value, now: u64, out: &mut Vec<Effect>) {
        let was_fresh = self.fresh();
        self.rc = if doc.is_null() { None } else { Some(doc.clone()) };
        self.rc_at = now;
        self.evaluate(now);
        if self.fresh() {
            self.read(now);
        } else if was_fresh && self.muted_rx {
            self.grace_due = Some(now + self.timing.grace_ms);
        }
        self.ask_newest(now, out);
    }

    /// After a restart the helper's copy carries an old stamp; while it is aged it is confirmed at once and once more 2.5 s later.
    fn ask_newest(&mut self, now: u64, out: &mut Vec<Effect>) {
        self.ask_left = 1;
        self.ask_due = Some(now + 2500);
        if self.helper_up && self.stale {
            self.ask(out);
        }
    }

    fn set_over_video(&mut self, over: bool) {
        if self.over_video != over {
            self.over_video = over;
            if self.mark_place >= self.mark_spots().len() {
                self.mark_place = 0;
            }
            self.changed = true;
        }
    }

    fn hop(&mut self) {
        let n = self.mark_spots().len();
        self.mark_place = (self.mark_place + 1) % n.max(1);
        self.changed = true;
    }

    // ---- the whole, for the tests and the gate

    pub fn tv(&self) -> &Tv {
        &self.tv
    }

    pub fn target(&self) -> Option<f64> {
        (self.keyed && self.opt_abs >= 0.0).then_some(self.opt_abs)
    }

    pub fn is_up(&self) -> bool {
        self.pill
    }

    /// What the model holds, for the host harness's `--eval volume`.
    pub fn debug(&self) -> Value {
        json!({"helperUp": self.helper_up, "stale": self.stale, "expiry": self.expiry, "rc": self.rc, "tv": {"output": self.tv.output, "known": self.tv.known(), "arc": self.tv.arc()}, "deciding": self.deciding, "keyed": self.keyed})
    }

    /// The state for the compositor's gate (`ctl shell`'s `ui.osd`).
    pub fn state(&self) -> Value {
        let v = self.view();
        json!({"osd": v.up, "source": v.source.name(), "level": v.level, "muted": v.muted, "mark": v.mark_up, "markPlace": v.mark_place, "logo": v.logo, "target": self.target()})
    }
}

impl Model for Volume {
    type View = View;

    fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        self.changed = false;
        match input {
            Input::Topic { topic, data } if topic == "audio" => {
                let was_muted = self.tv.muted;
                if self.tv.apply(data) {
                    self.cue(now, &mut out);
                }
                if self.tv.muted != was_muted {
                    self.mute_key(now);
                }
                self.changed = true;
            }
            Input::Topic { topic, data } if topic == "foreground" => {
                let home = data.get("home").and_then(Value::as_bool).unwrap_or(false);
                self.set_over_video(!home);
            }
            Input::Helper { topic, data } if topic == "receiver" => self.take_receiver(data, now, &mut out),
            Input::Helper { topic, data } if topic == "up" => {
                let up = data.as_bool().unwrap_or(false);
                if up != self.helper_up {
                    self.helper_up = up;
                    self.evaluate(now);
                    if self.fresh() {
                        self.read(now);
                    }
                    self.ask_newest(now, &mut out);
                    self.changed = true;
                }
            }
            Input::Fetched { tag, status, data } if *tag == CONFIRM_TAG => {
                self.asking = false;
                if *status == 200
                    && let Some(value) = data.get("value")
                    && !value.is_null()
                {
                    self.rc = Some(value.clone());
                    self.rc_at = now;
                    self.evaluate(now);
                    if self.fresh() {
                        self.read(now);
                    }
                }
            }
            Input::Tick => {
                if self.expiry.is_some_and(|t| now >= t) {
                    let was_fresh = self.fresh();
                    self.evaluate(now);
                    if was_fresh && !self.fresh() {
                        if self.muted_rx {
                            self.grace_due = Some(now + self.timing.grace_ms);
                        }
                        if self.muted_rx {
                            self.ask(&mut out);
                        }
                        self.changed = true;
                    }
                }
                if self.hide_due.is_some_and(|t| now >= t) {
                    self.hide_due = None;
                    self.pill = false;
                    self.changed = true;
                }
                if self.decide_due.is_some_and(|t| now >= t) {
                    self.decide_due = None;
                    self.deciding = false;
                    self.changed = true;
                }
                if self.hold_due.is_some_and(|t| now >= t) {
                    self.hold_due = None;
                    self.held_since = 0;
                    self.owed = 0.0;
                    self.confirm();
                }
                if self.settle_due.is_some_and(|t| now >= t) {
                    self.end_keyed();
                }
                if self.send_due.is_some_and(|t| now >= t) {
                    self.send_due = None;
                    self.send(now, &mut out);
                }
                if self.grace_due.is_some_and(|t| now >= t) {
                    self.grace_due = None;
                    if !self.fresh() {
                        self.muted_rx = false;
                        self.changed = true;
                    }
                }
                if self.ask_due.is_some_and(|t| now >= t) {
                    self.ask_due = None;
                    if self.ask_left > 0 {
                        self.ask_left -= 1;
                        if self.helper_up && self.stale {
                            self.ask(&mut out);
                        } else {
                            self.ask_left = 0;
                        }
                    }
                }
                let v = self.view();
                if v.mark_up && !self.still {
                    match self.hop_due {
                        Some(t) if now >= t => {
                            self.hop();
                            self.hop_due = Some(now + self.timing.hop_ms);
                        }
                        None => self.hop_due = Some(now + self.timing.hop_ms),
                        _ => {}
                    }
                } else {
                    self.hop_due = None;
                }
            }
            _ => {}
        }
        if self.changed {
            out.push(Effect::Redraw);
        }
        out
    }

    fn view(&self) -> View {
        Volume::view(self)
    }

    fn due_at(&self) -> Option<u64> {
        [self.expiry, self.hide_due, self.decide_due, self.hold_due, self.settle_due, self.send_due, self.grace_due, self.ask_due, self.hop_due]
            .into_iter()
            .flatten()
            .min()
    }
}

/// Where the mute mark may be: over an app the top corners, inside the safe area and below Home's status bar; over Home the status
/// bar's row between its groups, where Home draws nothing. `avoid_top_right`: the camera pop-up is up there.
pub fn mark_spots(over_video: bool, avoid_top_right: bool) -> Vec<(i32, i32)> {
    const EDGE_X: i32 = 96;
    const TOP_Y: i32 = 134;
    const INNER: i32 = 94;
    const BAR_Y: i32 = 52;
    if over_video {
        if avoid_top_right {
            vec![(EDGE_X, TOP_Y), (EDGE_X + INNER, TOP_Y + INNER)]
        } else {
            vec![(EDGE_X, TOP_Y), (1920 - EDGE_X - 64, TOP_Y), (1920 - EDGE_X - 64 - INNER, TOP_Y + INNER), (EDGE_X + INNER, TOP_Y + INNER)]
        }
    } else if avoid_top_right {
        vec![(560, BAR_Y), (760, BAR_Y)]
    } else {
        vec![(560, BAR_Y), (760, BAR_Y), (1160, BAR_Y), (1360, BAR_Y)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(volume: i32, muted: bool, output: &str, cause: &str) -> Input {
        Input::Topic {
            topic: "audio".into(),
            data: json!({"volumeStatus": {"volume": volume, "muteStatus": muted, "soundOutput": output, "maxVolume": 100, "cause": cause}}),
        }
    }

    fn receiver(abs: f64, mute: bool, at: u64) -> Input {
        Input::Helper {
            topic: "receiver".into(),
            data: json!({"power": "on", "enabled": true, "reachable": true, "fresh": true, "updatedAt": at, "volumeAbs": abs, "mute": mute, "volumeMax": 98, "soundMode": "DOLBY ATMOS", "signal": "Dolby Atmos - DD+"}),
        }
    }

    fn up() -> Input {
        Input::Helper { topic: "up".into(), data: json!(true) }
    }

    fn posts(effects: &[Effect]) -> Vec<f64> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::Post { path, body } if path == "/v1/receiver/volume" => body["level"].as_f64(),
                _ => None,
            })
            .collect()
    }

    /// A receiver at 42.5 with the sound going to it, read at `t0`.
    fn arc(t0: u64) -> Volume {
        let mut v = Volume::default();
        v.update(&up(), t0);
        v.update(&receiver(42.5, false, t0), t0);
        v.update(&audio(12, false, "external_arc", ""), t0);
        v
    }

    #[test]
    fn a_change_of_the_tvs_volume_puts_the_bar_up_for_the_hold_and_the_first_report_does_not() {
        let mut v = Volume::default();
        let e = v.update(&audio(31, false, "tv_speaker", ""), 1000);
        assert!(!v.view().up, "the state read at the start opens nothing: {e:?}");
        v.update(&audio(31, false, "tv_speaker", "volumeUp"), 1100);
        let view = v.view();
        assert!(view.up);
        assert_eq!(view.source, Source::Tv);
        assert_eq!(view.level, "31");
        assert!((view.fraction - 0.31).abs() < 0.001);
        assert_eq!(v.due_at(), Some(1100 + 2500));
        v.update(&Input::Tick, 3600);
        assert!(!v.view().up, "down after holdMs");
    }

    #[test]
    fn a_key_moves_the_receivers_level_at_once_and_posts_the_target() {
        let mut v = arc(10_000);
        assert_eq!(v.view().source, Source::Receiver);
        assert_eq!(v.view().level, "43", "42.5 rounds to a whole number");
        let e = v.update(&audio(13, false, "external_arc", "volumeUp"), 11_000);
        assert_eq!(v.view().level, "43", "one tap is one receiver step: 43.0");
        assert_eq!(v.target(), Some(43.0));
        assert_eq!(posts(&e), vec![43.0]);
        assert!(v.view().up);
        // an old reading does not move it back
        v.update(&receiver(42.5, false, 11_050), 11_050);
        assert_eq!(v.target(), Some(43.0));
        assert_eq!(v.view().level, "43");
        // the receiver reaches the target once the hold gap is over: the reading is taken
        v.update(&Input::Tick, 11_250);
        v.update(&receiver(43.0, false, 11_600), 11_600);
        assert_eq!(v.target(), None, "confirmed by the reading");
        assert_eq!(v.view().level, "43");
    }

    #[test]
    fn a_level_the_receiver_never_reaches_gives_way_to_its_reading_after_the_settle() {
        let mut v = arc(10_000);
        v.update(&audio(13, false, "external_arc", "volumeUp"), 11_000);
        v.update(&audio(14, false, "external_arc", "volumeUp"), 11_300);
        assert_eq!(v.target(), Some(43.5));
        v.update(&Input::Tick, 12_900);
        assert_eq!(v.target(), None, "settleMs after the last key the reading is what is shown");
        assert_eq!(v.view().level, "43", "the receiver's own 42.5");
    }

    #[test]
    fn a_held_key_moves_the_target_by_time_and_posts_at_most_ten_times_a_second() {
        let mut v = arc(10_000);
        let mut sent = Vec::new();
        let mut t = 11_000;
        for _ in 0..26 {
            sent.extend(posts(&v.update(&audio(20, false, "external_arc", "volumeUp"), t)));
            t += 40;
        }
        sent.extend(posts(&v.update(&Input::Tick, t + 100)));
        let target = v.target().unwrap();
        // one tap, then a second of holding at 6 steps a second rising to 10: about 8 steps of 0.5 over the tap
        assert!(target > 44.0 && target < 48.0, "{target}");
        assert!(sent.len() <= 12, "{} posts in one second: {sent:?}", sent.len());
        assert_eq!(sent.last().copied(), Some(target), "the last post is the target");
    }

    #[test]
    fn the_target_stays_under_the_receivers_limit_and_a_turn_starts_a_new_count() {
        let mut v = Volume::default();
        v.update(&up(), 10_000);
        v.update(
            &Input::Helper {
                topic: "receiver".into(),
                data: json!({"power": "on", "enabled": true, "reachable": true, "fresh": true, "updatedAt": 10_000, "volumeAbs": 69.5, "mute": false, "volumeMax": 70}),
            },
            10_000,
        );
        v.update(&audio(12, false, "external_arc", ""), 10_000);
        v.update(&audio(13, false, "external_arc", "volumeUp"), 11_000);
        assert_eq!(v.target(), Some(70.0));
        v.update(&audio(14, false, "external_arc", "volumeUp"), 11_040);
        assert_eq!(v.target(), Some(70.0), "never past MVMAX");
        v.update(&audio(13, false, "external_arc", "volumeDown"), 11_080);
        assert_eq!(v.target(), Some(69.5), "a turn is one step the other way");
    }

    #[test]
    fn a_volume_key_over_a_muted_receiver_shows_it_unmuted_and_the_mute_key_shows_the_mute_at_once() {
        let mut v = Volume::default();
        v.update(&up(), 10_000);
        v.update(&receiver(40.0, true, 10_000), 10_000);
        v.update(&audio(12, false, "external_arc", ""), 10_000);
        assert!(v.view().muted);
        v.update(&audio(13, false, "external_arc", "volumeUp"), 11_000);
        assert!(!v.view().muted, "the Denon cancels its mute on a volume key");
        v.update(&Input::Tick, 13_000);
        v.update(&receiver(40.5, false, 13_000), 13_000);
        v.update(&audio(13, true, "external_arc", ""), 14_000);
        assert!(v.view().muted, "the TV's own mute flag moved: the mute key");
        assert!(v.view().up);
    }

    #[test]
    fn without_a_receiver_it_is_the_tvs_volume_and_the_tvs_speakers_win_over_a_receiver_that_is_on() {
        let mut v = Volume::default();
        v.update(&audio(20, false, "external_arc", "volumeUp"), 1000);
        assert_eq!(v.view().source, Source::Tv, "no receiver topic at all: the TV's counter");
        assert_eq!(v.view().level, "20");
        let mut w = arc(10_000);
        w.update(&audio(55, false, "tv_speaker", "volumeUp"), 11_000);
        assert_eq!(w.view().source, Source::Tv);
        assert_eq!(w.view().level, "55");
        let before = w.view();
        w.update(&receiver(44.0, false, 11_500), 11_500);
        assert_eq!(w.view().level, before.level, "a receiver change is not shown while the TV's speakers are the output");
    }

    #[test]
    fn a_receiver_that_may_answer_is_waited_for_and_the_tvs_counter_is_shown_only_when_it_does_not() {
        let mut v = Volume::default();
        // the helper comes up with no copy yet: it is asked for one, and answers that it has none
        let mut e = v.update(&up(), 10_000);
        e.extend(v.update(&Input::Fetched { tag: CONFIRM_TAG, status: 200, data: json!({"ok": true, "value": null}) }, 10_010));
        // a copy that is aged out: the receiver may answer, but nothing in date says so; the aged copy is confirmed with the helper at once
        e.extend(v.update(&receiver(40.0, false, 1_000), 10_000));
        e.extend(v.update(&audio(12, false, "external_arc", "volumeUp"), 10_100));
        assert_eq!(v.view().source, Source::Wait);
        assert!(!v.view().up, "the pill waits");
        assert!(e.iter().any(|e| matches!(e, Effect::Post { path, .. } if path == "/v1/receiver/nudge")));
        assert_eq!(
            e.iter().filter(|e| matches!(e, Effect::Fetch { tag, .. } if *tag == CONFIRM_TAG)).count(),
            2,
            "asked once for a copy, once to confirm the aged one"
        );
        v.update(&Input::Tick, 10_400);
        assert_eq!(v.view().source, Source::Tv, "decideMs later the TV's counter");
        assert!(v.view().up);
        // and when it does answer in time, its level is what is shown
        let mut w = Volume::default();
        w.update(&up(), 10_000);
        w.update(&receiver(40.0, false, 1_000), 10_000);
        w.update(&audio(12, false, "external_arc", "volumeUp"), 10_100);
        assert_eq!(w.view().source, Source::Wait);
        w.update(&Input::Fetched { tag: CONFIRM_TAG, status: 200, data: json!({"value": {"power": "on", "enabled": true, "reachable": true, "fresh": true, "updatedAt": 10_150, "volumeAbs": 40.0, "mute": false}}) }, 10_200);
        assert_eq!(w.view().source, Source::Receiver);
        assert_eq!(w.view().level, "40");
    }

    #[test]
    fn the_format_is_a_mark_and_words_only_while_the_receiver_is_read_and_not_muted() {
        let mut v = arc(10_000);
        v.update(&audio(13, false, "external_arc", "volumeUp"), 11_000);
        let view = v.view();
        assert_eq!(view.logo, "atmos");
        assert_eq!(view.format, "Dolby Atmos");
        v.update(&audio(13, true, "external_arc", ""), 11_100);
        assert_eq!(v.view().logo, "", "muted: no mark");
    }

    #[test]
    fn the_mute_mark_stays_hops_every_ten_seconds_holds_still_under_a_panel_and_goes_with_the_sound() {
        let mut v = Volume::default();
        v.update(&up(), 10_000);
        v.update(&receiver(40.0, true, 10_000), 10_000);
        // the receiver is what is muted (its own remote); the TV's own flag stays off
        v.update(&audio(12, false, "external_arc", "setVolume"), 10_000);
        assert!(v.view().up);
        assert!(v.view().muted);
        v.update(&Input::Tick, 12_600);
        let view = v.view();
        assert!(!view.up && view.mark_up, "the bar went, the mark stays");
        assert_eq!(view.mark_place, 0);
        assert_eq!(v.mark_spots().len(), 4);
        // the helper keeps reading the receiver: a reading every five seconds keeps the mute in date
        for t in [15_000, 20_000] {
            v.update(&receiver(40.0, true, t), t);
        }
        v.update(&Input::Tick, 22_700);
        assert_eq!(v.view().mark_place, 1, "hopped after hopMs");
        v.still = true;
        for t in [25_000, 30_000] {
            v.update(&receiver(40.0, true, t), t);
        }
        v.update(&Input::Tick, 32_800);
        assert_eq!(v.view().mark_place, 1, "still under a panel");
        assert!(v.view().mark_up);
        v.still = false;
        v.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "netflix", "home": false}) }, 33_000);
        assert_eq!(v.mark_spots().len(), 4);
        assert_eq!(v.mark_spots()[0], (96, 134), "over an app: the top corners");
        // the helper stops reading: the reading ages out, the TV's own flag is what is read, and it is off
        v.update(&Input::Tick, 36_100);
        assert!(!v.view().mark_up, "aged out: gone");
        // the sound comes back: the mark goes with it
        v.update(&receiver(40.0, true, 42_000), 42_000);
        assert!(v.view().mark_up);
        v.update(&receiver(40.0, false, 43_000), 43_000);
        assert!(!v.view().mark_up);
    }

    #[test]
    fn a_reading_that_is_not_in_date_is_not_shown_and_the_next_is() {
        let mut v = Volume::default();
        v.update(&up(), 10_000);
        v.update(&receiver(40.0, false, 2_000), 10_000);
        v.update(&audio(12, false, "external_arc", ""), 10_000);
        assert_eq!(v.view().source, Source::Tv, "a stale reading: the receiver's 40 is never shown, the TV's own counter is");
        assert_eq!(v.view().level, "12");
        v.update(&receiver(41.0, false, 10_500), 10_500);
        assert_eq!(v.view().source, Source::Receiver);
        assert_eq!(v.view().level, "41");
    }

    #[test]
    fn over_home_the_bar_is_the_same_and_the_state_names_what_the_gate_reads() {
        let mut v = Volume::default();
        v.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true}) }, 1000);
        v.update(&audio(31, false, "tv_speaker", "volumeUp"), 1100);
        let s = v.state();
        assert_eq!(s["osd"], true);
        assert_eq!(s["source"], "tv");
        assert_eq!(s["level"], "31");
        assert!(!v.view().over_video);
    }
}
