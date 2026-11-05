//! The screensaver's model: when it is up, what tvpower asks it, when it moves and dims, and what it does on a key (`Screensaver.qml` and
//! `SaverVote.qml`, case for case).
//!
//! **When it is up.** `active` is the owner's saver not being off and either a preview (`forced`), LG's power state `Screen Saver` (LG's saver
//! was started: under LumaTV's compositor this one is drawn above it) or `owned`: tvpower asked, while Home was in front and this saver wanted
//! to draw, whether LG's saver may start, and the answer was no (the power state stays `Active`, LG's saver is never started). An owned saver is
//! up only over Home; LG's confirmed saver and previews are up over any foreground app while the TV is on. Ownership ends on a key, a pointer
//! that moves more than a few pixels, Home no longer in front, the TV switched off or the owner turning the saver off.
//!
//! **The vote.** While Home is in front and the saver is wanted (`takesOver`: `own` is not false and the style not off) the model is on
//! tvpower's two screen-saver lists, `/power2` and `/power`, with `registerScreenSaverRequest {clientName, subscribe: true}`, asks again when a
//! list has not taken it (3 s, five times, then once a minute) and answers each question it hears: no while wanted (and then `owned`), yes
//! otherwise. Nothing here is a setting of the TV: the subscription is the client's connection, so a shell that is not running leaves LG's saver
//! as it always was.
//!
//! **A key.** Any key while it is up asks the TV to turn the screen on when LG's state is `Screen Saver` (or a preview), and ends the saver when
//! it is the only one; the key does nothing else. **Standby** after `standbyHours` up (never a wake from standby: the panel is not left on a
//! picture overnight, and it is not lit again by the saver).
//!
//! The block's own timeline is here too, so that it is a model and a test: the spot, the shift, the dim, and the fade out and in at each move.

use super::look::{self, Config, Rng};
use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

const TVPOWER: &str = "luna://com.webos.service.tvpower";
const LISTS: [&str; 2] = ["/power2", "/power"];
const TAG_SUB: [u64; 2] = [0x5359_0010, 0x5359_0011];
const TAG_WAKE: u64 = 0x5359_0020;
const TAG_STANDBY: u64 = 0x5359_0021;
/// The answers to tvpower's questions: one tag each, in a ring.
const TAG_ANSWER: u64 = 0x5359_0100;
const ANSWER_RING: u64 = 64;
/// A list that has not taken the subscription is asked again after this long, this many times, then once in `RETRY_SLOW_MS`.
pub const RETRY_MS: u64 = 3000;
pub const RETRY_FAST: u32 = 5;
pub const RETRY_SLOW_MS: u64 = 60_000;
/// A pointer that moves more than this many pixels from where it was when the saver came up wakes it.
pub const POINTER_MOVE: f32 = 12.0;
/// How often the dim and the shift are looked at (they change in steps far smaller than this).
pub const LOOK_MS: u64 = 5000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Sub {
    tag: Option<u64>,
    live: bool,
    fails: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Showing, until the move at `next_move`.
    Shown,
    /// Faded out; the new spot is taken at `at` and the block fades in.
    Out { at: u64 },
}

/// What the surface draws.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub up: bool,
    /// The block's brightness of its first, from the dim.
    pub level: f32,
    /// The block's top-left corner, with the shift.
    pub pos: (i32, i32),
    /// 1 shown, 0 faded out, and how long the fade takes.
    pub fade: f32,
    pub fade_ms: u32,
    pub cap: f32,
    pub glance: bool,
    pub reduced: bool,
}

pub struct Saver {
    cfg: Config,
    client: String,
    // what the channel and the helper say
    home_front: bool,
    /// Something full screen is over Home (the `covered` of the foreground topic).
    covered: bool,
    /// A video is on the display (the compositor's `playing` topic: an app's pipeline, a sink of `videooutputd`, one of LumaTV's own windows).
    video: bool,
    /// The shell's own live view is up (the Cameras viewer, the camera pop-up): its picture is not the compositor's to see.
    live_view: bool,
    power_state: String,
    tv_on: bool,
    forced: bool,
    owned: bool,
    // the vote
    subs: [Sub; 2],
    voting: bool,
    retry_at: Option<u64>,
    asked: u32,
    refused: u32,
    answers: u64,
    last_refusal: String,
    // the timeline
    up_since: Option<u64>,
    spot: (f32, f32),
    rng: Rng,
    phase: Phase,
    next_move: u64,
    look_at: u64,
    standby_sent: bool,
    screen: (f32, f32),
    block: (f32, f32),
    pointer: Option<(f32, f32)>,
    /// The helper's count of requests to wake the screen; `None` until the first document.
    wake_seen: Option<u64>,
    /// The volume and mute as the `audio` topic last said: a change while the saver is up is a volume key (the compositor takes those keys
    /// itself), and the bar it shows is under the saver, so the saver ends.
    audio: Option<(Option<i64>, Option<bool>)>,
    redraw: bool,
    /// The compositor says the panel's own light carries the dim (`paneldim` topic, `carry`): the block is drawn at its first brightness and the
    /// dim over time is [`Saver::state`]'s `level`, which the compositor asks of the panel. Without it the saver dims its own picture.
    panel_carry: bool,
    /// The dim's level as last looked at (every [`LOOK_MS`]), of the first brightness.
    level: f32,
}

impl Saver {
    pub fn new(cfg: Config, client: &str, seed: u64) -> Self {
        Self {
            cfg,
            client: client.to_string(),
            home_front: false,
            covered: false,
            video: false,
            live_view: false,
            power_state: String::new(),
            tv_on: true,
            forced: false,
            owned: false,
            subs: [Sub::default(); 2],
            voting: false,
            retry_at: None,
            asked: 0,
            refused: 0,
            answers: 0,
            last_refusal: String::new(),
            up_since: None,
            spot: (0.36, 0.58),
            rng: Rng::new(seed),
            phase: Phase::Shown,
            next_move: 0,
            look_at: 0,
            standby_sent: false,
            screen: (1920.0, 1080.0),
            block: (900.0, 540.0),
            pointer: None,
            wake_seen: None,
            audio: None,
            redraw: false,
            panel_carry: false,
            level: 1.0,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// A new theme: the owner may have turned the saver off or taken LG's back.
    pub fn set_config(&mut self, cfg: Config, now: u64) -> Vec<Effect> {
        self.cfg = cfg;
        let mut out = Vec::new();
        self.settle(now, &mut out);
        out
    }

    /// The screen and the block it moves, in pixels.
    pub fn set_geometry(&mut self, screen: (f32, f32), block: (f32, f32)) {
        self.screen = screen;
        self.block = block;
    }

    /// A preview: shown without LG's power state (Settings' button, the harness).
    pub fn force(&mut self, on: bool, now: u64) -> Vec<Effect> {
        self.forced = on;
        let mut out = Vec::new();
        self.settle(now, &mut out);
        out
    }

    // ---- what is true now -----------------------------------------------------------------------------------------------------------

    fn takes_over(&self) -> bool {
        self.cfg.own && !self.cfg.off
    }

    fn want(&self) -> bool {
        self.takes_over() && !self.forced && self.home_front
    }

    /// Whether this saver may draw over what is on the screen: not over a video or a live view of LumaTV's, and not under a cover. It still answers
    /// tvpower's question no there (LG's saver must not start over them either); it only does not take the screen. LG's confirmed `Screen Saver` state
    /// is LG's own policy and is drawn whatever this says.
    fn can_draw(&self) -> bool {
        !self.covered && !self.video && !self.live_view
    }

    fn active(&self) -> bool {
        !self.cfg.off && (self.forced || self.power_state == "Screen Saver" || self.owned)
    }

    pub fn up(&self) -> bool {
        self.tv_on && self.active() && (self.home_front || self.lg())
    }

    /// LG's own state is the saver's (a key must ask the TV to turn the screen on), or this is a preview.
    fn lg(&self) -> bool {
        self.power_state == "Screen Saver" || self.forced
    }

    // ---- input ------------------------------------------------------------------------------------------------------------------

    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        match input {
            Input::Topic { topic, data } if topic == "foreground" => {
                self.home_front = data.get("home").and_then(Value::as_bool) == Some(true);
                self.covered = data.get("covered").and_then(Value::as_bool) == Some(true);
            }
            Input::Topic { topic, data } if topic == "playing" => {
                self.video = data.get("video").and_then(Value::as_bool) == Some(true);
            }
            // the Home key is the compositor's own and does not come over the key socket: it is told as a topic, and ends the saver
            Input::Topic { topic, .. } if topic == "homekey" && self.up() => self.wake(now, &mut out),
            Input::Topic { topic, data } if topic == "paneldim" => {
                let carry = data.get("carry").and_then(Value::as_bool) == Some(true);
                if carry != self.panel_carry {
                    self.panel_carry = carry;
                    self.redraw = true;
                }
            }
            Input::Topic { topic, data } if topic == "audio" => {
                if let Some(v) = data.get("volumeStatus") {
                    let heard = (v.get("volume").and_then(Value::as_i64), v.get("muteStatus").and_then(Value::as_bool));
                    let changed = self.audio.is_some_and(|was| was != heard);
                    self.audio = Some(heard);
                    if changed && self.up() {
                        self.wake(now, &mut out);
                    }
                }
            }
            Input::Helper { topic, data } if topic == "screen" => {
                self.power_state = data.get("state").and_then(Value::as_str).unwrap_or("").to_string();
                let on = data.get("on").and_then(Value::as_bool) != Some(false);
                if on != self.tv_on {
                    self.tv_on = on;
                    // the TV came on again after being off: the subscription is made anew, in case tvpower kept none through standby
                    if on && self.voting {
                        self.close_vote(&mut out);
                        self.open_vote(now, &mut out);
                    }
                }
            }
            Input::Helper { topic, data } if topic == "saver" => {
                // the helper asks for the screen to be woken (a doorbell): acted on when the count has moved since the first document
                let seq = data.pointer("/wake/seq").and_then(Value::as_u64).unwrap_or(0);
                match self.wake_seen {
                    None => self.wake_seen = Some(seq),
                    Some(seen) if seq != seen => {
                        self.wake_seen = Some(seq);
                        if self.up() {
                            self.wake(now, &mut out);
                        }
                    }
                    _ => {}
                }
            }
            Input::Luna { tag, payload } => self.heard(*tag, payload, now, &mut out),
            Input::Key { down: true, .. } if self.up() => self.wake(now, &mut out),
            Input::Pointer { x, y, buttons, visible } if self.up() && *visible => {
                if *buttons != 0 {
                    self.wake(now, &mut out);
                } else {
                    match self.pointer {
                        None => self.pointer = Some((*x, *y)),
                        Some((px, py)) if (x - px).abs() + (y - py).abs() > POINTER_MOVE => self.wake(now, &mut out),
                        _ => {}
                    }
                }
            }
            Input::Tick => self.tick(now, &mut out),
            _ => {}
        }
        self.settle(now, &mut out);
        out
    }

    /// Any key, the pointer or the helper's request: the TV is asked to turn the screen on when LG's state is the saver's; a saver that is
    /// the only one simply ends.
    fn wake(&mut self, _now: u64, out: &mut Vec<Effect>) {
        let lg = self.lg();
        self.owned = false;
        self.forced = false;
        if self.power_state == "Screen Saver" {
            self.power_state = "Active".into();
        }
        if lg {
            out.push(Effect::Luna { tag: TAG_WAKE, uri: format!("{TVPOWER}/power/turnOnScreen"), payload: json!({}), subscribe: false });
        }
        out.push(Effect::Redraw);
    }

    /// The shell's own live view (the Cameras viewer, the camera pop-up) came up or went: the saver does not draw over it.
    pub fn set_live_view(&mut self, on: bool, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if self.live_view != on {
            self.live_view = on;
            self.settle(now, &mut out);
        }
        out
    }

    pub fn wake_for_event(&mut self, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if self.up() {
            self.wake(now, &mut out);
            self.settle(now, &mut out);
        }
        out
    }

    // ---- the vote ---------------------------------------------------------------------------------------------------------------------

    fn open_vote(&mut self, now: u64, out: &mut Vec<Effect>) {
        for (i, list) in LISTS.iter().enumerate() {
            if self.subs[i].tag.is_some() {
                continue;
            }
            self.subs[i].tag = Some(TAG_SUB[i]);
            out.push(Effect::Luna {
                tag: TAG_SUB[i],
                uri: format!("{TVPOWER}{list}/registerScreenSaverRequest"),
                payload: json!({"clientName": self.client, "subscribe": true}),
                subscribe: true,
            });
        }
        self.arm(now);
    }

    fn close_vote(&mut self, out: &mut Vec<Effect>) {
        for sub in &mut self.subs {
            if let Some(tag) = sub.tag.take() {
                out.push(Effect::Cancel { tag });
            }
            *sub = Sub::default();
        }
        self.retry_at = None;
    }

    fn pending(&self) -> bool {
        self.subs.iter().any(|s| !s.live)
    }

    fn delay(&self) -> u64 {
        if self.subs.iter().any(|s| !s.live && s.fails < RETRY_FAST) { RETRY_MS } else { RETRY_SLOW_MS }
    }

    fn arm(&mut self, now: u64) {
        if !(self.voting && self.pending()) {
            self.retry_at = None;
        } else if self.retry_at.is_none() {
            self.retry_at = Some(now + self.delay());
        }
    }

    /// An answer of tvpower: to a subscription (live, or refused), or a question on it (it has a timestamp).
    fn heard(&mut self, tag: u64, payload: &Value, now: u64, out: &mut Vec<Effect>) {
        let Some(i) = TAG_SUB.iter().position(|t| *t == tag) else {
            return;
        };
        if payload.get("returnValue").and_then(Value::as_bool) == Some(false) {
            // tvpower said no (not up yet, not in a state to take it): the call is let go, and asked for again
            if let Some(t) = self.subs[i].tag.take() {
                out.push(Effect::Cancel { tag: t });
            }
            self.subs[i].live = false;
            self.subs[i].fails += 1;
            self.last_refusal = format!("{} {}", LISTS[i], payload.to_string().replace(['{', '}', '"', '\\'], "").chars().take(160).collect::<String>());
            self.retry_at = None;
            self.arm(now);
            return;
        }
        if !self.subs[i].live {
            self.subs[i].live = true;
            self.subs[i].fails = 0;
            out.push(Effect::Log(format!("[saver] on tvpower's list {}", LISTS[i])));
        }
        self.arm(now);
        // the subscription's own answer has no timestamp; a question has one
        let stamp = payload.get("timestamp").filter(|t| !t.is_null() && t.as_str() != Some(""));
        let Some(stamp) = stamp else {
            return;
        };
        let yes = !self.want();
        self.asked += 1;
        if !yes {
            self.refused += 1;
        }
        self.answers += 1;
        out.push(Effect::Luna {
            tag: TAG_ANSWER + self.answers % ANSWER_RING,
            uri: format!("{TVPOWER}{}/responseScreenSaverRequest", LISTS[i]),
            payload: json!({"clientName": self.client, "timestamp": stamp.clone(), "ack": yes}),
            subscribe: false,
        });
        out.push(Effect::Log(format!(
            "[saver] {} asked: {}",
            LISTS[i],
            match (yes, self.can_draw()) {
                (true, _) => "yes (not ours to draw)",
                (false, true) => "no (LumaTV's saver draws)",
                (false, false) => "no (a video or a cover is on the screen: nothing is drawn)",
            }
        )));
        if !yes && self.can_draw() {
            self.owned = true;
        }
    }

    // ---- time -------------------------------------------------------------------------------------------------------------------------

    fn tick(&mut self, now: u64, out: &mut Vec<Effect>) {
        if self.retry_at.is_some_and(|t| now >= t) {
            self.retry_at = None;
            for sub in &mut self.subs {
                if sub.live {
                    continue;
                }
                if let Some(tag) = sub.tag.take() {
                    out.push(Effect::Cancel { tag });
                    sub.fails += 1;
                }
            }
            self.open_vote(now, out);
        }
        let Some(since) = self.up_since else {
            return;
        };
        if self.cfg.standby_ms > 0 && !self.standby_sent && now.saturating_sub(since) >= self.cfg.standby_ms {
            self.standby_sent = true;
            out.push(Effect::Luna { tag: TAG_STANDBY, uri: format!("{TVPOWER}/power/powerOff"), payload: json!({"reason": "remoteKey"}), subscribe: false });
        }
        match self.phase {
            Phase::Shown if now >= self.next_move => {
                self.phase = Phase::Out { at: now + u64::from(self.cfg.fade_ms) };
                self.redraw = true;
            }
            Phase::Out { at } if now >= at => {
                self.spot = look::pick(&mut self.rng, self.spot, look::room(self.screen, self.block));
                self.phase = Phase::Shown;
                self.next_move = now + self.cfg.move_every_ms + u64::from(self.cfg.fade_ms);
                self.redraw = true;
            }
            _ => {}
        }
        if now >= self.look_at {
            self.look_at = now + LOOK_MS.min(self.cfg.shift_every_ms.max(1000));
            self.level = look::level(&self.cfg, now.saturating_sub(since));
            self.redraw = true;
        }
    }

    /// After every input: what changed in what is wanted and what is up.
    fn settle(&mut self, now: u64, out: &mut Vec<Effect>) {
        // `owned` ends with the things it depends on
        if !self.home_front || !self.tv_on || !self.takes_over() || !self.can_draw() {
            self.owned = false;
        }
        let want = self.want();
        if want != self.voting {
            self.voting = want;
            if want {
                self.open_vote(now, out);
            } else {
                self.close_vote(out);
            }
        }
        let up = self.up();
        if up && self.up_since.is_none() {
            self.up_since = Some(now);
            self.level = 1.0;
            self.spot = look::pick(&mut self.rng, self.spot, look::room(self.screen, self.block));
            self.phase = Phase::Shown;
            self.next_move = now + self.cfg.move_every_ms + u64::from(self.cfg.fade_ms);
            self.look_at = now + LOOK_MS;
            self.standby_sent = false;
            self.pointer = None;
            self.redraw = true;
        } else if !up && self.up_since.is_some() {
            self.up_since = None;
            self.pointer = None;
            self.redraw = true;
        }
        if self.redraw {
            self.redraw = false;
            out.push(Effect::Redraw);
        }
    }

    pub fn due_at(&self) -> Option<u64> {
        let mut due = self.retry_at;
        if self.up_since.is_some() {
            let next = match self.phase {
                Phase::Shown => self.next_move,
                Phase::Out { at } => at,
            };
            due = [due, Some(next), Some(self.look_at)].into_iter().flatten().min();
            if let (Some(since), false) = (self.up_since, self.standby_sent) {
                if self.cfg.standby_ms > 0 {
                    due = due.into_iter().chain([since + self.cfg.standby_ms]).min();
                }
            }
        }
        due
    }

    pub fn view(&self, now: u64) -> View {
        let up_ms = self.up_since.map_or(0, |s| now.saturating_sub(s));
        let shift = look::drift(&self.cfg, up_ms);
        let base = look::place(self.screen, self.block, self.spot);
        View {
            up: self.up(),
            level: if self.panel_carry { 1.0 } else { look::level(&self.cfg, up_ms) },
            pos: (base.0 + shift.0, base.1 + shift.1),
            fade: if matches!(self.phase, Phase::Out { .. }) { 0.0 } else { 1.0 },
            fade_ms: self.cfg.fade_ms,
            cap: self.cfg.cap,
            glance: self.cfg.glance,
            reduced: self.cfg.reduced,
        }
    }

    /// What the compositor is told: the saver is up (OLED care stands aside; the helper's rules read it), and a read of the vote for the gate.
    pub fn state(&self) -> Value {
        // `level` and `carry` are what the compositor reads to dim the panel in the saver's place (`paneldim`): only while the saver says it does not
        // draw the dim itself
        json!({"up": self.up(), "active": self.active(), "owned": self.owned, "forced": self.forced, "level": self.level, "carry": self.panel_carry,
               "canDraw": self.can_draw(),
               "vote": {"want": self.want(), "registered": self.subs.iter().any(|s| s.live), "power2": self.subs[0].live, "power": self.subs[1].live,
                        "fails": self.subs[0].fails + self.subs[1].fails, "asked": self.asked, "refused": self.refused, "lastRefusal": self.last_refusal}})
    }

    /// Whether the saver wants every key (any key wakes it): while it is up.
    pub fn wants_keys(&self) -> bool {
        self.up()
    }

    pub fn power_state(&self) -> &str {
        &self.power_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000;

    fn saver() -> Saver {
        Saver::new(Config::from_theme(&json!({}), false, 90), "org.nphil.lumatv", 5)
    }

    fn topic(t: &str, v: Value) -> Input {
        Input::Topic { topic: t.into(), data: v }
    }

    fn helper(t: &str, v: Value) -> Input {
        Input::Helper { topic: t.into(), data: v }
    }

    fn home(s: &mut Saver, now: u64) -> Vec<Effect> {
        s.update(&topic("foreground", json!({"app": "", "home": true})), now)
    }

    fn luna_calls(fx: &[Effect]) -> Vec<(u64, String, Value, bool)> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Luna { tag, uri, payload, subscribe } => Some((*tag, uri.clone(), payload.clone(), *subscribe)),
                _ => None,
            })
            .collect()
    }

    fn question(s: &mut Saver, list: usize, stamp: u64, now: u64) -> Vec<Effect> {
        s.update(&Input::Luna { tag: TAG_SUB[list], payload: json!({"returnValue": true, "timestamp": stamp}) }, now)
    }

    #[test]
    fn native_confirmed_saver_appears_over_apps_only_after_lg_starts_it() {
        for app in ["com.webos.app.marquee", "com.nvidia.geforcenow", "arbitrary.app"] {
            let mut s = saver();
            s.update(&topic("foreground", json!({"app": app, "home": false})), NOW);
            s.update(&helper("screen", json!({"on": true, "state": "Active"})), NOW + 1);
            assert!(!s.up(), "{app}: active app must remain visible");
            let calls = luna_calls(&question(&mut s, 0, 7, NOW + 2));
            assert_eq!(calls[0].2["ack"], true, "{app}: a question is not approval");
            assert!(!s.up() && s.state()["owned"] == false);
            let fx = s.update(&helper("screen", json!({"on": true, "state": "Screen Saver"})), NOW + 3);
            assert!(s.up() && s.view(NOW + 3).up && s.wants_keys(), "{app}: LG approved the saver");
            assert!(fx.contains(&Effect::Redraw));
        }
    }

    #[test]
    fn native_app_saver_wakes_like_home_and_follows_native_pointer_and_power_state() {
        let mut s = saver();
        s.update(&topic("foreground", json!({"app": "marquee", "home": false})), NOW);
        s.update(&helper("screen", json!({"on": true, "state": "Screen Saver"})), NOW + 1);
        let pointer = |x, buttons| Input::Pointer { x, y: 10.0, buttons, visible: true };
        assert!(luna_calls(&s.update(&pointer(1.0, 0), NOW + 2)).is_empty());
        assert!(luna_calls(&s.update(&pointer(5.0, 0), NOW + 3)).is_empty());
        assert!(s.up(), "small pointer movement does not wake");
        let fx = s.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, NOW + 4);
        assert!(luna_calls(&fx).is_empty() && s.up());
        let fx = s.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, NOW + 5);
        let calls = luna_calls(&fx);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "luna://com.webos.service.tvpower/power/turnOnScreen");
        assert!(!s.up() && !s.wants_keys() && fx.contains(&Effect::Redraw));
        s.update(&helper("screen", json!({"on": true, "state": "Active"})), NOW + 6);
        assert!(!s.up());
        s.update(&helper("screen", json!({"on": true, "state": "Screen Saver"})), NOW + 7);
        s.update(&pointer(1.0, 0), NOW + 8);
        let fx = s.update(&pointer(100.0, 0), NOW + 9);
        assert_eq!(luna_calls(&fx)[0].1, "luna://com.webos.service.tvpower/power/turnOnScreen");
        assert!(!s.up() && fx.contains(&Effect::Redraw));
        s.update(&helper("screen", json!({"on": true, "state": "Screen Saver"})), NOW + 10);
        let fx = s.update(&pointer(100.0, 1), NOW + 11);
        assert_eq!(luna_calls(&fx)[0].1, "luna://com.webos.service.tvpower/power/turnOnScreen");
        assert!(!s.up());
    }

    #[test]
    fn app_transition_drops_home_ownership_but_keeps_native_confirmation() {
        let mut s = saver();
        home(&mut s, NOW);
        question(&mut s, 0, 1, NOW + 1);
        let fx = s.update(&topic("foreground", json!({"app": "game", "home": false})), NOW + 2);
        assert!(!s.up() && s.state()["owned"] == false && fx.contains(&Effect::Redraw));
        let fx = s.update(&helper("screen", json!({"on": true, "state": "Screen Saver"})), NOW + 3);
        assert!(s.up() && fx.contains(&Effect::Redraw));
        home(&mut s, NOW + 4);
        s.update(&topic("foreground", json!({"app": "game", "home": false})), NOW + 5);
        assert!(s.up() && s.state()["owned"] == false);
        s.update(&helper("screen", json!({"on": false, "state": "Screen Saver"})), NOW + 6);
        assert!(!s.up(), "the TV being off takes precedence over a stale native state");
        s.update(&helper("screen", json!({"on": true, "state": "Screen Saver"})), NOW + 7);
        assert!(s.up());
        s.set_config(Config::from_theme(&json!({"screensaver": {"style": "off"}}), false, 90), NOW + 8);
        assert!(!s.up());
        let mut lg = Saver::new(Config::from_theme(&json!({"screensaver": {"own": false}}), false, 90), "c", 1);
        let fx = lg.update(&topic("foreground", json!({"app": "game", "home": false})), NOW);
        assert!(luna_calls(&fx).is_empty());
        lg.update(&helper("screen", json!({"on": true, "state": "Screen Saver"})), NOW + 1);
        assert!(lg.up() && lg.state()["owned"] == false, "own=false only disables the Home vote");
    }

    #[test]
    fn native_app_saver_never_writes_lgs_timeout_preference() {
        let mut s = saver();
        let mut effects = s.update(&topic("foreground", json!({"app": "game", "home": false})), NOW);
        effects.extend(s.update(&helper("screen", json!({"on": true, "state": "Active"})), NOW + 1));
        effects.extend(s.update(&Input::Tick, NOW + 3_600_000));
        assert!(!s.up(), "time alone cannot start the saver");
        effects.extend(s.update(&helper("screen", json!({"on": true, "state": "Screen Saver"})), NOW + 3_600_001));
        effects.extend(s.update(&Input::Key { name: "left".into(), down: true, repeat: false }, NOW + 3_600_002));
        let calls = luna_calls(&effects);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].1.ends_with("/power/turnOnScreen"));
    }

    #[test]
    fn with_home_in_front_it_is_on_both_of_tvpowers_lists_and_with_an_app_in_front_it_is_on_neither() {
        let mut s = saver();
        assert!(luna_calls(&s.update(&topic("foreground", json!({"app": "netflix", "home": false})), NOW)).is_empty());
        let fx = home(&mut s, NOW + 1);
        let calls = luna_calls(&fx);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, "luna://com.webos.service.tvpower/power2/registerScreenSaverRequest");
        assert_eq!(calls[1].1, "luna://com.webos.service.tvpower/power/registerScreenSaverRequest");
        assert_eq!(calls[0].2, json!({"clientName": "org.nphil.lumatv", "subscribe": true}));
        assert!(calls.iter().all(|c| c.3), "subscriptions");
        let fx = s.update(&topic("foreground", json!({"app": "netflix", "home": false})), NOW + 2);
        assert_eq!(fx.iter().filter(|e| matches!(e, Effect::Cancel { .. })).count(), 2, "left again when Home goes");
    }

    #[test]
    fn the_owner_taking_lgs_saver_back_or_turning_it_off_leaves_no_vote() {
        let mut s = Saver::new(Config::from_theme(&json!({"screensaver": {"own": false}}), false, 90), "c", 1);
        assert!(luna_calls(&home(&mut s, NOW)).is_empty(), "own: false leaves LG's saver alone");
        let mut s = Saver::new(Config::from_theme(&json!({"screensaver": {"style": "off"}}), false, 90), "c", 1);
        assert!(luna_calls(&home(&mut s, NOW)).is_empty());
        let mut s = saver();
        home(&mut s, NOW);
        let fx = s.set_config(Config::from_theme(&json!({"screensaver": {"own": false}}), false, 90), NOW + 5);
        assert_eq!(fx.iter().filter(|e| matches!(e, Effect::Cancel { .. })).count(), 2, "the switch is read live");
    }

    #[test]
    fn a_question_while_wanted_is_answered_no_and_the_saver_is_up_owned_and_a_question_when_not_wanted_is_answered_yes() {
        let mut s = saver();
        home(&mut s, NOW);
        let fx = question(&mut s, 0, 77, NOW + 10);
        let calls = luna_calls(&fx);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "luna://com.webos.service.tvpower/power2/responseScreenSaverRequest");
        assert_eq!(calls[0].2, json!({"clientName": "org.nphil.lumatv", "timestamp": 77, "ack": false}), "no: LG's saver is not started");
        assert!(!calls[0].3);
        assert!(s.up() && s.state()["owned"] == true && s.state()["vote"]["refused"] == 1);
        assert!(fx.contains(&Effect::Redraw));
        // the subscription's own answer has no timestamp and asks nothing
        let mut t = saver();
        home(&mut t, NOW);
        assert!(luna_calls(&t.update(&Input::Luna { tag: TAG_SUB[1], payload: json!({"returnValue": true}) }, NOW + 1)).is_empty());
        assert_eq!(t.state()["vote"]["power"], true);
        // an app comes to the front: the question that arrives then is yes
        let mut u = saver();
        home(&mut u, NOW);
        u.update(&Input::Luna { tag: TAG_SUB[0], payload: json!({"returnValue": true}) }, NOW + 1);
        u.update(&topic("foreground", json!({"app": "netflix", "home": false})), NOW + 2);
        let late = luna_calls(&question(&mut u, 0, 5, NOW + 3));
        assert_eq!(late[0].2["ack"], true, "not wanted: yes, as if this client were not there");
        assert!(!u.up());
    }

    #[test]
    fn it_ends_on_a_key_and_asks_the_tv_to_turn_the_screen_on_only_when_lgs_state_is_the_saver() {
        let mut s = saver();
        home(&mut s, NOW);
        question(&mut s, 0, 1, NOW + 1);
        assert!(s.up());
        let fx = s.update(&Input::Key { name: "left".into(), down: true, repeat: false }, NOW + 50);
        assert!(luna_calls(&fx).is_empty(), "the state is Active: there is nothing to ask, it simply ends");
        assert!(!s.up());
        // LG's state is Screen Saver: the TV is asked
        let mut t = saver();
        home(&mut t, NOW);
        t.update(&helper("screen", json!({"on": true, "state": "Screen Saver"})), NOW + 1);
        assert!(t.up());
        let fx = t.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, NOW + 2);
        let calls = luna_calls(&fx);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "luna://com.webos.service.tvpower/power/turnOnScreen");
        t.update(&helper("screen", json!({"on": true, "state": "Active"})), NOW + 3);
        assert!(!t.up(), "the power state ends it");
        // a key while it is not up is not its business
        assert!(luna_calls(&t.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, NOW + 4)).is_empty());
        // a release does not wake
        let mut u = saver();
        home(&mut u, NOW);
        u.update(&helper("screen", json!({"state": "Screen Saver"})), NOW + 1);
        assert!(luna_calls(&u.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, NOW + 2)).is_empty());
        assert!(u.up());
    }

    #[test]
    fn a_pointer_that_moves_wakes_an_owned_saver_and_the_first_sample_is_only_where_it_rests() {
        let mut s = saver();
        home(&mut s, NOW);
        question(&mut s, 0, 1, NOW + 1);
        let at = |x: f32, y: f32, buttons: u32| Input::Pointer { x, y, buttons, visible: true };
        s.update(&at(500.0, 500.0, 0), NOW + 2);
        s.update(&at(505.0, 503.0, 0), NOW + 3);
        assert!(s.up(), "a few pixels is not a move");
        s.update(&at(520.0, 500.0, 0), NOW + 4);
        assert!(!s.up(), "twenty is");
        let mut t = saver();
        home(&mut t, NOW);
        question(&mut t, 0, 1, NOW + 1);
        t.update(&at(5.0, 5.0, 1), NOW + 2);
        assert!(!t.up(), "a button wakes at once");
        // The compositor also asks LG to wake when it receives the pointer over LG's own saver.
        let mut u = saver();
        home(&mut u, NOW);
        u.update(&helper("screen", json!({"state": "Screen Saver"})), NOW + 1);
        u.update(&at(1.0, 1.0, 0), NOW + 2);
        let fx = u.update(&at(900.0, 900.0, 0), NOW + 3);
        assert!(!u.up());
        assert_eq!(luna_calls(&fx)[0].1, "luna://com.webos.service.tvpower/power/turnOnScreen");
    }

    #[test]
    fn a_volume_key_ends_the_saver_because_the_bar_it_would_show_is_under_it_and_the_first_reading_is_only_the_level() {
        let mut s = saver();
        home(&mut s, NOW);
        question(&mut s, 0, 1, NOW + 1);
        let audio = |volume: i64, muted: bool| topic("audio", json!({"volumeStatus": {"volume": volume, "muteStatus": muted}}));
        s.update(&audio(20, false), NOW + 2);
        assert!(s.up(), "the first reading is where the volume already was");
        s.update(&audio(20, false), NOW + 3);
        assert!(s.up(), "the same reading again changes nothing");
        s.update(&audio(21, false), NOW + 4);
        assert!(!s.up(), "a step of the volume ends it");
        let mut t = saver();
        home(&mut t, NOW);
        question(&mut t, 0, 1, NOW + 1);
        t.update(&audio(20, false), NOW + 2);
        t.update(&audio(20, true), NOW + 3);
        assert!(!t.up(), "so does the mute key");
        // LG's own saver: the TV is asked to turn the screen on
        let mut u = saver();
        home(&mut u, NOW);
        u.update(&helper("screen", json!({"on": true, "state": "Screen Saver"})), NOW + 1);
        u.update(&audio(20, false), NOW + 2);
        let fx = u.update(&audio(25, false), NOW + 3);
        assert_eq!(luna_calls(&fx).len(), 1, "the screen is asked to come on: {fx:?}");
    }

    #[test]
    fn the_home_key_ends_the_saver_it_is_told_as_a_topic_because_the_key_socket_does_not_carry_it() {
        let mut s = saver();
        home(&mut s, NOW);
        question(&mut s, 0, 1, NOW + 1);
        assert!(s.up());
        s.update(&topic("homekey", json!({"n": 1, "switcher": false})), NOW + 2);
        assert!(!s.up());
        let mut t = saver();
        home(&mut t, NOW);
        t.update(&helper("screen", json!({"on": true, "state": "Screen Saver"})), NOW + 1);
        let calls = luna_calls(&t.update(&topic("homekey", json!({"n": 2})), NOW + 2));
        assert_eq!(calls.len(), 1, "LG's own state: the screen is asked to come on");
        let mut u = saver();
        home(&mut u, NOW);
        assert!(luna_calls(&u.update(&topic("homekey", json!({"n": 3})), NOW + 1)).is_empty(), "not up: not its business");
    }

    #[test]
    fn the_saver_ends_when_home_is_not_in_front_the_tv_goes_off_or_the_owner_turns_it_off_and_a_tv_that_comes_back_asks_again() {
        let mut s = saver();
        home(&mut s, NOW);
        question(&mut s, 0, 1, NOW + 1);
        s.update(&topic("foreground", json!({"app": "netflix", "home": false})), NOW + 2);
        assert!(!s.up() && s.state()["owned"] == false);
        let mut t = saver();
        home(&mut t, NOW);
        question(&mut t, 0, 1, NOW + 1);
        t.update(&helper("screen", json!({"on": false, "state": "Active Standby"})), NOW + 2);
        assert!(!t.up(), "off");
        let fx = t.update(&helper("screen", json!({"on": true, "state": "Active"})), NOW + 3);
        let calls = luna_calls(&fx);
        assert_eq!(calls.iter().filter(|c| c.3).count(), 2, "the subscriptions are made anew when the TV comes on");
        let mut u = saver();
        home(&mut u, NOW);
        question(&mut u, 0, 1, NOW + 1);
        u.set_config(Config::from_theme(&json!({"screensaver": {"style": "off"}}), false, 90), NOW + 2);
        assert!(!u.up());
    }

    #[test]
    fn a_subscription_that_was_refused_is_asked_again_in_three_seconds_five_times_and_then_once_a_minute() {
        let mut s = saver();
        home(&mut s, NOW);
        assert_eq!(s.due_at(), Some(NOW + RETRY_MS));
        let refuse =
            |s: &mut Saver, now: u64| s.update(&Input::Luna { tag: TAG_SUB[0], payload: json!({"returnValue": false, "errorText": "not ready"}) }, now);
        s.update(&Input::Luna { tag: TAG_SUB[1], payload: json!({"returnValue": true}) }, NOW + 1);
        let fx = refuse(&mut s, NOW + 2);
        assert!(fx.iter().any(|e| matches!(e, Effect::Cancel { tag } if *tag == TAG_SUB[0])), "the refused call is let go");
        assert_eq!(s.state()["vote"]["fails"], 1);
        assert!(s.state()["vote"]["lastRefusal"].as_str().unwrap().contains("/power2"));
        let mut now = NOW + 2;
        for fails in 2..=5 {
            let due = s.due_at().unwrap();
            assert_eq!(due, now + RETRY_MS, "fast retries: {fails}");
            now = due;
            let fx = s.update(&Input::Tick, now);
            assert_eq!(luna_calls(&fx).iter().filter(|c| c.3).count(), 1, "only the list that has not taken it is asked");
            refuse(&mut s, now);
            let _ = fails;
        }
        assert_eq!(s.due_at(), Some(now + RETRY_SLOW_MS), "after five fails: once a minute");
        let due = s.due_at().unwrap();
        s.update(&Input::Tick, due);
        s.update(&Input::Luna { tag: TAG_SUB[0], payload: json!({"returnValue": true}) }, due + 1);
        assert_eq!(s.due_at(), None, "both lists have it: nothing more is asked");
    }

    #[test]
    fn the_block_moves_every_so_often_fading_out_and_in_and_the_dim_runs_down() {
        let mut s = Saver::new(Config::from_theme(&json!({"screensaver": {"moveEverySec": 60, "fadeMs": 2000}}), false, 90), "c", 3);
        home(&mut s, NOW);
        s.update(&helper("screen", json!({"state": "Screen Saver"})), NOW);
        assert!(s.up());
        let v0 = s.view(NOW);
        assert_eq!((v0.fade, v0.level), (1.0, 1.0));
        let first = v0.pos;
        s.update(&Input::Tick, NOW + 62_000);
        assert_eq!(s.view(NOW + 62_000).fade, 0.0, "fading out");
        assert!(s.due_at().unwrap() <= NOW + 64_001);
        s.update(&Input::Tick, NOW + 64_100);
        let v = s.view(NOW + 64_100);
        assert_eq!(v.fade, 1.0, "and in at the new spot");
        assert_ne!(v.pos, first);
        assert!(v.level < 1.0);
        let late = s.view(NOW + 30 * 60_000);
        assert!((late.level - 0.4).abs() < 0.006);
    }

    #[test]
    fn with_the_panel_carrying_the_dim_the_block_is_drawn_at_its_first_brightness_and_the_level_is_told_to_the_compositor() {
        let mut s = saver();
        home(&mut s, NOW);
        question(&mut s, 0, 1, NOW + 1);
        assert!(s.up());
        // not told: the saver dims its own picture and says so
        s.update(&Input::Tick, NOW + 10 * 60_000);
        let state = s.state();
        assert_eq!(state["carry"], false);
        assert!((s.view(NOW + 10 * 60_000).level - 0.7).abs() < 0.006, "its own dim");
        // told: drawn at 1, and the level the dim has reached is what the compositor reads
        let fx = s.update(&topic("paneldim", json!({"carry": true})), NOW + 10 * 60_000 + 1);
        assert!(fx.contains(&Effect::Redraw), "the picture is drawn again at its first brightness");
        assert_eq!(s.view(NOW + 10 * 60_000 + 1).level, 1.0);
        s.update(&Input::Tick, NOW + 20 * 60_000);
        let state = s.state();
        assert_eq!(state["carry"], true);
        assert!((state["level"].as_f64().unwrap() - 0.4).abs() < 0.006, "the dim's own curve, for the panel: {state}");
        assert_eq!(s.view(NOW + 20 * 60_000).level, 1.0);
        // taken away again (the panel failed): the saver dims its own picture from where the curve is
        s.update(&topic("paneldim", json!({"carry": false})), NOW + 20 * 60_000 + 2);
        assert!((s.view(NOW + 20 * 60_000 + 2).level - 0.4).abs() < 0.006);
        // a new time up starts at 1 again
        s.update(&Input::Key { name: "left".into(), down: true, repeat: false }, NOW + 20 * 60_000 + 3);
        assert!(!s.up());
        home(&mut s, NOW + 21 * 60_000);
        question(&mut s, 0, 2, NOW + 21 * 60_000 + 1);
        assert_eq!(s.state()["level"].as_f64(), Some(1.0));
    }

    #[test]
    fn after_the_standby_hours_the_tv_is_put_in_standby_once() {
        let mut s = Saver::new(Config::from_theme(&json!({"screensaver": {"standbyHours": 1}}), false, 90), "c", 3);
        home(&mut s, NOW);
        for tag in TAG_SUB {
            s.update(&Input::Luna { tag, payload: json!({"returnValue": true}) }, NOW);
        }
        s.update(&helper("screen", json!({"state": "Screen Saver"})), NOW);
        let mut now = NOW;
        let mut sent = 0;
        while now < NOW + 2 * 3_600_000 {
            now = s.due_at().unwrap().max(now);
            for c in luna_calls(&s.update(&Input::Tick, now)) {
                assert_eq!((c.1.as_str(), c.2.clone()), ("luna://com.webos.service.tvpower/power/powerOff", json!({"reason": "remoteKey"})));
                assert!(now >= NOW + 3_600_000, "not before the hour");
                sent += 1;
            }
            now += 1;
        }
        assert_eq!(sent, 1);
        let mut never = Saver::new(Config::from_theme(&json!({"screensaver": {"standbyHours": 0}}), false, 90), "c", 3);
        home(&mut never, NOW);
        never.update(&helper("screen", json!({"state": "Screen Saver"})), NOW);
        assert!(luna_calls(&never.update(&Input::Tick, NOW + 9 * 3_600_000)).iter().all(|c| !c.1.ends_with("powerOff")), "0 hours is never");
    }

    #[test]
    fn the_helper_asking_for_a_wake_ends_a_saver_that_is_up_but_the_first_document_is_only_the_count() {
        let mut s = saver();
        home(&mut s, NOW);
        s.update(&helper("screen", json!({"state": "Screen Saver"})), NOW);
        s.update(&helper("saver", json!({"items": [], "wake": {"seq": 4}})), NOW + 1);
        assert!(s.up(), "the first document is the count it starts from");
        let fx = s.update(&helper("saver", json!({"items": [], "wake": {"seq": 5, "class": "doorbell"}})), NOW + 2);
        assert_eq!(luna_calls(&fx).len(), 1, "the TV is asked to turn the screen on");
        let same = s.update(&helper("saver", json!({"items": [], "wake": {"seq": 5}})), NOW + 3);
        assert!(luna_calls(&same).is_empty());
    }

    #[test]
    fn a_preview_is_up_without_lgs_state_and_needs_no_vote() {
        let mut s = saver();
        let fx = s.force(true, NOW);
        assert!(s.up() && luna_calls(&fx).is_empty());
        assert!(s.wants_keys());
        let fx = s.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, NOW + 1);
        assert_eq!(luna_calls(&fx).len(), 1, "it asks the TV, as it always did");
    }

    #[test]
    fn the_view_is_inside_the_safe_area_whatever_the_shift() {
        let mut s = saver();
        s.set_geometry((1920.0, 1080.0), (900.0, 540.0));
        home(&mut s, NOW);
        s.update(&helper("screen", json!({"state": "Screen Saver"})), NOW);
        for step in 0..400u64 {
            let t = NOW + step * 4000;
            s.update(&Input::Tick, t);
            let v = s.view(t);
            assert!(v.pos.0 >= 96 - 30 && v.pos.1 >= 54 - 20 && v.pos.0 + 900 <= 1920 - 96 + 30 && v.pos.1 + 540 <= 1080 - 54 + 20, "{:?}", v.pos);
        }
    }

    /// Home in front with the vote on its way: a saver that takes the screen over Home when tvpower asks.
    fn voting_saver() -> Saver {
        let mut s = saver();
        home(&mut s, NOW);
        s.update(&helper("screen", json!({"on": true, "state": "Active"})), NOW);
        s
    }

    #[test]
    fn tvpowers_question_is_answered_no_but_nothing_is_drawn_over_a_video_a_cover_or_a_live_view() {
        for blocked in ["video", "covered", "live view"] {
            let mut s = voting_saver();
            match blocked {
                "video" => drop(s.update(&topic("playing", json!({"video": true})), NOW + 1)),
                "covered" => drop(s.update(&topic("foreground", json!({"app": "", "home": true, "covered": true})), NOW + 1)),
                _ => drop(s.set_live_view(true, NOW + 1)),
            }
            let calls = luna_calls(&question(&mut s, 0, 7, NOW + 2));
            assert_eq!(calls[0].2["ack"], false, "{blocked}: LG's saver must not start over it either");
            assert!(!s.up() && s.state()["owned"] == false, "{blocked}: nothing drawn");
            // it is gone: the next question takes the screen
            match blocked {
                "video" => drop(s.update(&topic("playing", json!({"video": false})), NOW + 3)),
                "covered" => drop(s.update(&topic("foreground", json!({"app": "", "home": true, "covered": false})), NOW + 3)),
                _ => drop(s.set_live_view(false, NOW + 3)),
            }
            question(&mut s, 0, 8, NOW + 4);
            assert!(s.up(), "{blocked}: the next question takes the screen");
        }
    }

    #[test]
    fn a_saver_that_is_up_ends_when_a_video_or_a_live_view_starts_and_lgs_confirmed_saver_is_not_this_rule() {
        let mut s = voting_saver();
        question(&mut s, 0, 7, NOW + 1);
        assert!(s.up());
        let fx = s.update(&topic("playing", json!({"video": true})), NOW + 2);
        assert!(!s.up() && fx.contains(&Effect::Redraw), "a video started under it");
        assert!(luna_calls(&fx).is_empty(), "it only stops drawing: tvpower is not asked to wake anything");
        s.update(&topic("playing", json!({"video": false})), NOW + 3);
        question(&mut s, 0, 8, NOW + 4);
        assert!(s.up());
        s.set_live_view(true, NOW + 5);
        assert!(!s.up(), "the viewer came up");
        // LG approved a saver over a paused video (its policy): it is drawn whatever the compositor says of video
        let mut s = saver();
        s.update(&topic("foreground", json!({"app": "netflix", "home": false, "covered": true})), NOW);
        s.update(&topic("playing", json!({"video": true})), NOW);
        s.update(&helper("screen", json!({"on": true, "state": "Screen Saver"})), NOW + 1);
        assert!(s.up(), "LG's confirmed saver is LG's decision");
    }
}
