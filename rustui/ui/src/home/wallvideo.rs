//! Home's wallpaper as a clip the TV's video decoder plays (`WallVideo.qml`, `WallVideo.js`; `docs/research/videoplane-saver-result.md`, "The moving
//! wallpaper"), so an idle Home draws no wallpaper frames of its own. The clip is the wallpaper picture through the GPU wallpaper's warp with the
//! dark material's scrim baked in, so it stands in for the GPU wallpaper only while everything it baked is still what Home would draw: the picture,
//! the look (`theme.wallpaperMotion`) and the scrim. Anything else (another wallpaper, a look being adjusted, the light material) draws the GPU wallpaper,
//! as before there was a clip.
//!
//! **The contract.** The player (`lumatv-vplane`, a 32-bit process) takes requests in `/tmp/<shortId>-vplane/req.json` (`{seq, sent, op, ttlMs, role:
//! "wall", clip}`; `op` is `start`, `renew` or `stop`) and says what it does in `status.json` (`{ts, seq, role, state, reason}`), a mailbox it shares with
//! the art saver (one decoder: a saver `start` takes it from the wall, a wall `start` while the saver plays is ignored). LumaTV's compositor keeps a
//! window for the player at its lowest layer, exports the whole 1920 by 1080 as the video element (`videooutput setDisplayWindow` with the source and its
//! `originalInput`, which the TV refuses without it: `luna::video::placement`) and clears that rectangle of the UI plane, the hole, just under Home. Its
//! `wall` topic says the hole is under Home. This controller asks for the clip and tells Home when the plane really shows it: [`WallVideo::up`] is
//! `want && playing && settled && hole`, `SETTLE_MS` after the first picture with the hole under Home. Only then does Home take its own picture and scrim
//! away (`WallpaperSurface::set_hole`) and paint its panes at the reading alpha, which is what the clip's `glass.alpha` (0.80 for `wall-silk`) is. The way
//! back is the other order: `up` falls the moment Home stops wanting the clip, the GPU wallpaper is back in the same frame and the player is told to stop
//! `END_MS` later, so a planned hand-over shows no black plane. The player is renewed every 10 s; unrenewed for 30 s it takes its window away itself.
//!
//! No I/O here: the files are the shell's effects ([`Effect::Read`] of the status and the manifest, [`Effect::Write`] of the request), under the
//! mailbox directory only.

use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

/// The mailbox directory (`/tmp/{{shortId}}-vplane`, brand.json's `shortId`), where the player reads its requests and writes its status.
pub const MAILBOX: &str = "/tmp/lumatv-vplane";
pub const REQ_FILE: &str = "/tmp/lumatv-vplane/req.json";
pub const STATUS_FILE: &str = "/tmp/lumatv-vplane/status.json";
/// The clips' manifest, which `lumatv vplane install` puts in the data directory.
pub const MANIFEST_FILE: &str = "vplane/clips/wall.json";
pub const STATUS_TAG: u64 = 330;
pub const MANIFEST_TAG: u64 = 331;

/// How long a start may take, how often the player is renewed, how long it keeps a window without a renewal, how old a playing player's status may
/// be, how old an idle one's (it writes every 60 s), how long the first picture is let to settle, how long the stop waits after the clip was let go,
/// how often a missing player is asked again, how often the manifest is read.
const START_MS: u64 = 6000;
const RENEW_MS: u64 = 10_000;
const TTL_MS: u64 = 30_000;
const STALE_MS: u64 = 12_000;
const IDLE_STALE_MS: u64 = 150_000;
pub const SETTLE_MS: u64 = 250;
pub const END_MS: u64 = 250;
const RETRY_MS: u64 = 15_000;
const MANIFEST_MS: u64 = 5000;
const POLL_STARTING_MS: u64 = 200;
const POLL_PLAYING_MS: u64 = 500;

/// The settings of a look a clip bakes, with the QML's own defaults for a key the theme lacks.
const KEYS: [(&str, f64); 10] = [
    ("amplitude", 4.0),
    ("speed", 1.0),
    ("depth", 0.25),
    ("scale", 1.0),
    ("detail", 0.35),
    ("vignette", 0.0),
    ("specular", 0.0),
    ("bloom", 0.0),
    ("saturation", 1.0),
    ("contrast", 1.0),
];

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// The names of the settings in `motion` that are not the clip's, in the order of [`KEYS`].
fn differs(entry: &Value, motion: &Value) -> Vec<&'static str> {
    KEYS.iter()
        .filter(|(k, default)| {
            let want = motion.get(*k).and_then(Value::as_f64).unwrap_or(*default);
            let baked = entry.pointer(&format!("/look/motion/{k}")).and_then(Value::as_f64).unwrap_or(f64::NAN);
            !near(baked, want)
        })
        .map(|(k, _)| *k)
        .collect()
}

fn same_scrim(a: &Value, scrim: &[f64]) -> bool {
    let Some(a) = a.as_array() else { return false };
    a.len() == scrim.len() && a.iter().zip(scrim).all(|(x, y)| x.as_f64().is_some_and(|x| near(x, *y)))
}

/// The manifest's clip for the wallpaper `wallpaper` (`theme.wallpaper`, a file of the theme directory by its bare name) under the look `motion` and
/// the dark material's scrim `scrim`: its name for the player ("wall-silk"), or the reason there is none. With several clips the reason is the nearest
/// one's: the clip whose look is the owner's but for the scrim, else the one that differs in the fewest settings.
pub fn pick<'a>(manifest: Option<&'a Value>, wallpaper: &str, motion: &Value, scrim: &[f64]) -> Result<&'a str, String> {
    let Some(walls) = manifest.and_then(|m| m.get("walls")).and_then(Value::as_array).filter(|w| !w.is_empty()) else { return Err("no clips".into()) };
    let mut same = walls.iter().filter(|w| w.pointer("/source/file").and_then(Value::as_str) == Some(wallpaper)).peekable();
    if same.peek().is_none() {
        return Err("no clip for this wallpaper".into());
    }
    let (mut why, mut fewest) = (String::new(), usize::MAX);
    for w in same {
        let bad = differs(w, motion);
        if bad.is_empty() {
            if w.pointer("/look/scrim").is_some_and(|s| same_scrim(s, scrim)) {
                return Ok(w.get("file").and_then(Value::as_str).unwrap_or("").trim_end_matches(".vpc"));
            }
            why = "the scrim is not the clip's".into();
            fewest = 0;
        } else if bad.len() < fewest {
            why = format!("the look is not the clip's ({})", bad.join(", "));
            fewest = bad.len();
        }
    }
    Err(why)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    Starting,
    Playing,
    Failed,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Off => "off",
            State::Starting => "starting",
            State::Playing => "playing",
            State::Failed => "failed",
        }
    }
}

/// What Home says about itself: what the theme asks for, and whether the clip may be on the plane now.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ctx {
    /// The dark material is the one in use (the clip is the dark material's picture).
    pub dark: bool,
    /// Home is the drawn front with the screen on.
    pub active: bool,
    /// The screensaver is over Home (it takes the player).
    pub saver: bool,
}

/// What a read of the status was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    Begin,
    Tick,
}

pub struct WallVideo {
    pub state: State,
    pub reason: String,
    want: bool,
    watch: bool,
    clip: String,
    hole: bool,
    why_not: String,
    manifest: Option<Value>,
    manifest_text: String,
    status: Option<Value>,
    settled: bool,
    seq: u64,
    start_seq: u64,
    asked_at: u64,
    reading: Option<Why>,
    poll_at: Option<u64>,
    renew_at: Option<u64>,
    settle_at: Option<u64>,
    stop_at: Option<u64>,
    retry_at: Option<u64>,
    manifest_at: Option<u64>,
}

impl Default for WallVideo {
    fn default() -> Self {
        Self {
            state: State::Off,
            reason: String::new(),
            want: false,
            watch: false,
            clip: String::new(),
            hole: false,
            why_not: String::new(),
            manifest: None,
            manifest_text: String::new(),
            status: None,
            settled: false,
            seq: 0,
            start_seq: 0,
            asked_at: 0,
            reading: None,
            poll_at: None,
            renew_at: None,
            settle_at: None,
            stop_at: None,
            retry_at: None,
            manifest_at: None,
        }
    }
}

impl WallVideo {
    fn busy(&self) -> bool {
        matches!(self.state, State::Starting | State::Playing)
    }

    /// The clip is on the plane under Home, the hole is open and the first picture has settled: Home's own picture may go.
    pub fn up(&self) -> bool {
        self.want && self.state == State::Playing && self.settled && self.hole
    }

    /// The clip Home asks the player for now, and why it asks for none.
    pub fn clip(&self) -> &str {
        &self.clip
    }

    pub fn state(&self) -> Value {
        json!({"state": self.state.name(), "reason": self.reason, "want": self.want, "clip": self.clip, "why": self.why_not, "hole": self.hole, "settled": self.settled, "up": self.up(),
               "manifest": self.manifest.as_ref().map(|m| m.get("walls").and_then(Value::as_array).map_or(0, Vec::len)), "player": self.status.as_ref().map(|s| json!({"state": s.get("state"), "role": s.get("role")}))})
    }

    /// Writes a request to the player and returns its number: one more than the last, and never below the clock (the player ignores an older one).
    fn send(&mut self, op: &str, clip: Option<&str>, now: u64, out: &mut Vec<Effect>) -> u64 {
        self.seq = (self.seq + 1).max(now);
        let mut req = json!({"seq": self.seq, "sent": now, "op": op, "ttlMs": TTL_MS, "role": "wall"});
        if let Some(c) = clip {
            req["clip"] = json!(c);
        }
        out.push(Effect::Write { path: REQ_FILE.into(), text: req.to_string() });
        self.seq
    }

    fn fail(&mut self, why: &str, now: u64, out: &mut Vec<Effect>) {
        if self.state == State::Failed {
            return;
        }
        self.reason = why.to_owned();
        self.state = State::Failed;
        self.stop_timers();
        self.send("stop", None, now, out);
    }

    fn stop_timers(&mut self) {
        self.poll_at = None;
        self.renew_at = None;
        self.settle_at = None;
        self.settled = false;
    }

    fn give_way(&mut self) {
        self.reason = "the saver has the player".into();
        self.state = State::Off;
        self.stop_timers();
    }

    /// The player's status is asked for (the answer is [`Input::Fetched`] with [`STATUS_TAG`]).
    fn read_status(&mut self, why: Why, out: &mut Vec<Effect>) {
        if self.reading.is_none() {
            self.reading = Some(why);
            out.push(Effect::Read { tag: STATUS_TAG, path: STATUS_FILE.into() });
        }
    }

    /// The start of a clip: asked for when the player's status says it is there and not busy with the saver.
    fn begin_with(&mut self, status: Option<Value>, now: u64, out: &mut Vec<Effect>) {
        let fresh = status.as_ref().and_then(|s| s.get("ts")).and_then(Value::as_u64).is_some_and(|ts| now.saturating_sub(ts) <= IDLE_STALE_MS);
        let Some(s) = status.filter(|_| fresh) else {
            self.reason = "no player".into();
            self.state = State::Off;
            self.retry_at = Some(now + RETRY_MS);
            return;
        };
        let (state, role) = (s.get("state").and_then(Value::as_str).unwrap_or(""), s.get("role").and_then(Value::as_str).unwrap_or(""));
        if state != "idle" && state != "error" && role != "wall" {
            self.give_way();
            return;
        }
        self.status = Some(s);
        self.reason.clear();
        self.retry_at = None;
        let clip = self.clip.clone();
        self.start_seq = self.send("start", Some(&clip), now, out);
        self.asked_at = now;
        self.state = State::Starting;
        self.poll_at = Some(now + POLL_STARTING_MS);
        self.renew_at = Some(now + RENEW_MS);
    }

    /// One look at the status while the clip starts or plays.
    fn tick_with(&mut self, status: Option<Value>, now: u64, out: &mut Vec<Effect>) {
        self.status = status.clone();
        let s = status.as_ref();
        let get = |k: &str| s.and_then(|s| s.get(k)).and_then(Value::as_str).unwrap_or("");
        let (sstate, role) = (get("state"), get("role"));
        let seq = s.and_then(|s| s.get("seq")).and_then(Value::as_u64);
        let ts = s.and_then(|s| s.get("ts")).and_then(Value::as_u64);
        match self.state {
            State::Starting => {
                let mine = seq.is_some_and(|q| q >= self.start_seq);
                if mine && role == "wall" && sstate == "playing" {
                    self.state = State::Playing;
                    self.reason.clear();
                    self.settle_at = Some(now + SETTLE_MS);
                    self.poll_at = Some(now + POLL_PLAYING_MS);
                } else if mine && sstate == "error" {
                    let why = match get("reason") {
                        "" => "the player could not start".to_owned(),
                        r => r.to_owned(),
                    };
                    self.fail(&why, now, out);
                } else if role == "saver" && (sstate == "playing" || sstate == "opening") {
                    self.give_way();
                } else if now.saturating_sub(self.asked_at) > START_MS {
                    self.fail(if mine { "the player did not reach playing" } else { "the player did not answer" }, now, out);
                } else {
                    self.poll_at = Some(now + POLL_STARTING_MS);
                }
            }
            State::Playing => {
                if s.is_none() || sstate != "playing" {
                    let why = match get("reason") {
                        "" => "the player stopped".to_owned(),
                        r => r.to_owned(),
                    };
                    self.fail(&why, now, out);
                } else if role != "wall" {
                    self.give_way();
                } else if ts.is_some_and(|t| now.saturating_sub(t) > STALE_MS) {
                    self.fail("the player went quiet", now, out);
                } else {
                    self.poll_at = Some(now + POLL_PLAYING_MS);
                }
            }
            State::Off | State::Failed => {}
        }
    }

    /// Home stops wanting the clip: a failure is forgotten, anything else is stopped `END_MS` later (the GPU wallpaper first, so no black plane shows).
    fn release(&mut self, now: u64) {
        if self.state == State::Failed {
            self.state = State::Off;
            self.reason.clear();
        } else if self.state != State::Off {
            self.stop_at = Some(now + END_MS);
        }
    }

    /// Fits the controller to what Home says now: whether it wants the clip (the switch is on, the contract holds, the clip is in the manifest),
    /// whether it watches for one, and whether the compositor's hole is under Home. Returns the effects (a request to the player, a read).
    pub fn sync(&mut self, theme: &Value, ctx: Ctx, hole: bool, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        let null = Value::Null;
        let wm = theme.get("wallpaperMotion").unwrap_or(&null);
        let video = wm.get("source").and_then(Value::as_str) == Some("video");
        let on = wm.get("enabled").and_then(Value::as_bool) != Some(false) && theme.pointer("/motion/reduced").and_then(Value::as_bool) != Some(true);
        let base = video && on && ctx.dark && ctx.active && !ctx.saver;
        // the pick is only made while the switch is on: with the GPU wallpaper (the default) nothing here costs anything
        let mut want = false;
        if base {
            let scrim: Vec<f64> = theme.pointer("/dark/wallScrim").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
            let wallpaper = theme.get("wallpaper").and_then(Value::as_str).unwrap_or("wall_1080.png");
            match pick(self.manifest.as_ref(), wallpaper, wm, &scrim) {
                Ok(name) => {
                    want = true;
                    if self.clip != name {
                        self.clip = name.to_owned();
                    }
                    self.why_not.clear();
                }
                Err(why) => {
                    if self.why_not != why {
                        self.why_not = why;
                    }
                }
            }
        }
        if !want {
            self.clip.clear();
        }
        self.hole = hole;
        if base != self.watch {
            self.watch = base;
            self.manifest_at = base.then_some(now);
        }
        if want != self.want {
            self.want = want;
            if want {
                self.stop_at = None;
                if matches!(self.state, State::Off | State::Failed) {
                    self.begin(&mut out);
                }
            } else {
                self.release(now);
                self.retry_at = None;
            }
        }
        out
    }

    fn begin(&mut self, out: &mut Vec<Effect>) {
        if self.state == State::Failed {
            self.state = State::Off;
        }
        self.read_status(Why::Begin, out);
    }

    /// An answer of a file read (the status, the manifest) or the clock: the controller's timers.
    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        match input {
            Input::Fetched { tag: STATUS_TAG, status, data } => {
                let doc = (*status == 200 && data.is_object()).then(|| data.clone());
                match self.reading.take() {
                    Some(Why::Begin) if self.want && matches!(self.state, State::Off | State::Failed) => self.begin_with(doc, now, &mut out),
                    Some(Why::Tick) if self.want && self.busy() => self.tick_with(doc, now, &mut out),
                    _ => {}
                }
            }
            Input::Fetched { tag: MANIFEST_TAG, status, data } => {
                let text = if *status == 200 { data.to_string() } else { String::new() };
                // unchanged: the pick stays as it is
                if text != self.manifest_text || self.manifest.is_none() && !text.is_empty() {
                    self.manifest_text = text;
                    self.manifest = (*status == 200 && data.is_object()).then(|| data.clone());
                }
            }
            Input::Tick => self.tick(now, &mut out),
            _ => {}
        }
        out
    }

    /// The timers: the poll while the clip starts or plays, the renewal, the settle after the first picture, the stop, the retry, the manifest.
    fn tick(&mut self, now: u64, out: &mut Vec<Effect>) {
        let due = |t: Option<u64>| t.is_some_and(|t| now >= t);
        if self.want && self.busy() && due(self.poll_at) {
            self.poll_at = None;
            self.read_status(Why::Tick, out);
        }
        if self.want && self.busy() && due(self.renew_at) {
            self.renew_at = Some(now + RENEW_MS);
            self.send("renew", None, now, out);
        }
        if due(self.settle_at) {
            self.settle_at = None;
            self.settled = self.state == State::Playing;
        }
        if due(self.stop_at) {
            self.stop_at = None;
            if !self.want && self.state != State::Off {
                self.send("stop", None, now, out);
                self.state = State::Off;
                self.reason.clear();
                self.stop_timers();
            }
        }
        if self.want && self.state == State::Off && self.reason == "no player" && due(self.retry_at) {
            self.retry_at = Some(now + RETRY_MS);
            self.read_status(Why::Begin, out);
        }
        if due(self.manifest_at) {
            self.manifest_at = self.watch.then_some(now + MANIFEST_MS);
            out.push(Effect::Read { tag: MANIFEST_TAG, path: MANIFEST_FILE.into() });
        }
        if self.state != State::Playing {
            self.settled = false;
        }
    }

    /// Home is shown again: the manifest is read now (a player or clips installed while Home was away).
    pub fn read_manifest(&mut self, now: u64) {
        if self.watch {
            self.manifest_at = Some(now);
        }
    }

    pub fn due_at(&self) -> Option<u64> {
        let busy = self.want && self.busy();
        [
            self.poll_at.filter(|_| busy),
            self.renew_at.filter(|_| busy),
            self.settle_at,
            self.stop_at,
            self.retry_at.filter(|_| self.want && self.state == State::Off && self.reason == "no player"),
            self.manifest_at.filter(|_| self.watch),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Value {
        serde_json::from_str(include_str!("../../../../payload/vplane/clips/wall.json")).expect("the shipped manifest")
    }

    fn theme(patch: Value) -> Value {
        let mut t = lumatv_tokens::Tokens::shipped().json().clone();
        lumatv_tokens::merge(&mut t, &patch);
        t
    }

    fn video() -> Value {
        theme(json!({"wallpaperMotion": {"source": "video"}}))
    }

    const FRONT: Ctx = Ctx { dark: true, active: true, saver: false };

    fn reads(out: &[Effect], tag: u64) -> usize {
        out.iter().filter(|e| matches!(e, Effect::Read { tag: t, .. } if *t == tag)).count()
    }

    fn requests(out: &[Effect]) -> Vec<Value> {
        out.iter()
            .filter_map(|e| match e {
                Effect::Write { path, text } if path == REQ_FILE => serde_json::from_str(text).ok(),
                _ => None,
            })
            .collect()
    }

    fn status(state: &str, role: &str, seq: u64, ts: u64) -> Input {
        Input::Fetched { tag: STATUS_TAG, status: 200, data: json!({"ts": ts, "seq": seq, "role": role, "state": state, "reason": ""}) }
    }

    /// The player's status as the controller reads it at `now`: its poll is due, it asks, and this is the answer.
    fn observe(w: &mut WallVideo, answer: Input, now: u64) -> Vec<Effect> {
        w.poll_at = Some(now);
        let mut out = w.update(&Input::Tick, now);
        out.extend(w.update(&answer, now));
        out
    }

    /// A controller that has the manifest and is asked for the clip, with the answer of the first status read still to give.
    fn asking(now: u64) -> (WallVideo, Value) {
        let mut w = WallVideo::default();
        let t = video();
        w.sync(&t, FRONT, false, now);
        w.update(&Input::Tick, now);
        w.update(&Input::Fetched { tag: MANIFEST_TAG, status: 200, data: manifest() }, now);
        let out = w.sync(&t, FRONT, false, now);
        assert_eq!(reads(&out, STATUS_TAG), 1, "the player's status is read: {out:?}");
        (w, t)
    }

    #[test]
    fn the_clip_is_picked_only_for_the_look_the_picture_and_the_scrim_it_was_made_with_and_the_reason_names_what_moved() {
        let m = manifest();
        let t = theme(json!({}));
        let scrim: Vec<f64> = t.pointer("/dark/wallScrim").and_then(Value::as_array).unwrap().iter().filter_map(Value::as_f64).collect();
        let motion = t.get("wallpaperMotion").unwrap();
        assert_eq!(pick(Some(&m), "wall_1080.png", motion, &scrim), Ok("wall-silk"), "the shipped look");
        assert_eq!(pick(Some(&m), "other.png", motion, &scrim), Err("no clip for this wallpaper".into()));
        assert_eq!(pick(None, "wall_1080.png", motion, &scrim), Err("no clips".into()));
        let wider = json!({"amplitude": 12, "speed": 1, "depth": 0.25, "scale": 1, "detail": 0.35});
        let why = pick(Some(&m), "wall_1080.png", &wider, &scrim).unwrap_err();
        assert!(why.starts_with("the look is not the clip's (") && why.contains("amplitude"), "{why}");
        assert_eq!(pick(Some(&m), "wall_1080.png", motion, &[0.2, 0.2, 0.2]), Err("the scrim is not the clip's".into()));
        // a key the theme lacks is the QML's own default, which is the clip's
        assert_eq!(pick(Some(&m), "wall_1080.png", &json!({}), &scrim), Ok("wall-silk"));
    }

    #[test]
    fn with_the_gpu_wallpaper_the_default_nothing_is_asked_of_anyone() {
        let mut w = WallVideo::default();
        let out = w.sync(&theme(json!({})), FRONT, true, 1000);
        assert!(out.is_empty() && !w.up() && w.due_at().is_none());
        // and the switch on without a manifest only reads the manifest, every five seconds
        let t = video();
        w.sync(&t, FRONT, true, 1000);
        assert_eq!(w.due_at(), Some(1000));
        assert_eq!(reads(&w.update(&Input::Tick, 1000), MANIFEST_TAG), 1);
        assert_eq!(w.due_at(), Some(6000));
        assert!(!w.up());
    }

    #[test]
    fn the_clip_is_asked_for_when_the_player_is_there_and_is_on_the_plane_only_once_it_plays_has_settled_and_the_hole_is_under_home() {
        let (mut w, t) = asking(10_000);
        // no player: the status file is not there; it is asked again every 15 s
        assert!(w.update(&Input::Fetched { tag: STATUS_TAG, status: 404, data: Value::Null }, 10_000).is_empty());
        assert_eq!((w.state, w.reason.as_str()), (State::Off, "no player"));
        assert_eq!(w.retry_at, Some(25_000));
        assert_eq!(reads(&w.update(&Input::Tick, 25_000), STATUS_TAG), 1);
        // the player answers idle: the start request goes out with the clip's name and the wall role
        let out = w.update(&status("idle", "saver", 5, 24_900), 25_000);
        let req = requests(&out);
        assert_eq!(req.len(), 1, "{out:?}");
        assert_eq!((req[0]["op"].as_str(), req[0]["role"].as_str(), req[0]["clip"].as_str(), req[0]["ttlMs"].as_u64()), (Some("start"), Some("wall"), Some("wall-silk"), Some(30_000)));
        assert!(req[0]["seq"].as_u64().unwrap() >= 25_000);
        let start = req[0]["seq"].as_u64().unwrap();
        assert_eq!(w.state, State::Starting);
        // it is polled every 200 ms while it starts, and a status from before the request is another session's
        assert_eq!(reads(&w.update(&Input::Tick, 25_200), STATUS_TAG), 1);
        w.update(&status("playing", "wall", start - 1, 25_200), 25_210);
        assert_eq!(w.state, State::Starting);
        assert_eq!(reads(&w.update(&Input::Tick, 25_410), STATUS_TAG), 1);
        w.update(&status("playing", "wall", start, 25_400), 25_420);
        assert_eq!(w.state, State::Playing);
        let hole = true;
        w.sync(&t, FRONT, hole, 25_420);
        assert!(!w.up(), "the first picture has not settled");
        w.update(&Input::Tick, 25_420 + SETTLE_MS);
        assert!(w.up(), "playing, settled and the hole under Home");
        w.sync(&t, FRONT, false, 25_700);
        assert!(!w.up(), "without the hole Home keeps its own picture");
        w.sync(&t, FRONT, true, 25_800);
        assert!(w.up());
        // renewed every 10 s while it plays
        let out = w.update(&Input::Tick, 35_410);
        assert_eq!(requests(&out).iter().map(|r| r["op"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>(), vec!["renew"]);
    }

    #[test]
    fn the_way_back_is_the_other_order_the_picture_returns_at_once_and_the_player_is_told_to_stop_a_moment_later() {
        let (mut w, t) = asking(10_000);
        w.update(&status("idle", "saver", 5, 10_000), 10_000);
        let start = w.start_seq;
        observe(&mut w, status("playing", "wall", start, 10_100), 10_100);
        w.sync(&t, FRONT, true, 10_100);
        w.update(&Input::Tick, 10_400);
        assert!(w.up());
        // the owner turns the switch off (or the look moves): Home draws its own wallpaper in this very frame
        let off = theme(json!({"wallpaperMotion": {"source": "gpu"}}));
        let out = w.sync(&off, FRONT, true, 11_000);
        assert!(!w.up(), "the GPU wallpaper is back now");
        assert!(requests(&out).is_empty(), "and the player is not told yet");
        assert_eq!(w.stop_at, Some(11_000 + END_MS));
        let out = w.update(&Input::Tick, 11_000 + END_MS);
        assert_eq!(requests(&out).iter().map(|r| r["op"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>(), vec!["stop"]);
        assert_eq!(w.state, State::Off);
        // wanting the clip again while it still plays just goes on: no second start
        let (mut w, t) = asking(20_000);
        w.update(&status("idle", "saver", 5, 20_000), 20_000);
        let start = w.start_seq;
        observe(&mut w, status("playing", "wall", start, 20_100), 20_100);
        w.sync(&off, FRONT, true, 20_200);
        let out = w.sync(&t, FRONT, true, 20_300);
        assert!(requests(&out).is_empty() && w.state == State::Playing && w.stop_at.is_none(), "it goes on, and the stop that was waiting is cancelled");
        assert!(requests(&w.update(&Input::Tick, 20_600)).iter().all(|r| r["op"] != "stop"));
    }

    #[test]
    fn a_player_busy_with_the_saver_a_failure_and_silence_each_give_the_clip_up_and_the_gpu_wallpaper_stays() {
        // the saver has the decoder: never asked for it
        let (mut w, _) = asking(10_000);
        let out = w.update(&status("playing", "saver", 5, 10_000), 10_000);
        assert!(requests(&out).is_empty());
        assert_eq!((w.state, w.reason.as_str()), (State::Off, "the saver has the player"));
        // it takes the player while the clip starts or plays
        let (mut w, _) = asking(20_000);
        w.update(&status("idle", "saver", 5, 20_000), 20_000);
        let start = w.start_seq;
        observe(&mut w, status("playing", "saver", start, 20_200), 20_200);
        assert_eq!((w.state, w.reason.as_str()), (State::Off, "the saver has the player"));
        // an error answer: failed, stopped, and not tried again
        let (mut w, _) = asking(30_000);
        w.update(&status("idle", "saver", 5, 30_000), 30_000);
        let start = w.start_seq;
        let out = observe(&mut w, Input::Fetched { tag: STATUS_TAG, status: 200, data: json!({"ts": 30_200, "seq": start, "role": "wall", "state": "error", "reason": "no decoder"}) }, 30_210);
        assert_eq!((w.state, w.reason.as_str()), (State::Failed, "no decoder"));
        assert_eq!(requests(&out)[0]["op"], "stop");
        assert!(!w.up() && w.poll_at.is_none() && w.renew_at.is_none(), "nothing is polled or renewed any more");
        // wanting nothing forgets the failure, and wanting again asks again
        w.sync(&theme(json!({})), FRONT, false, 31_000);
        assert_eq!(w.state, State::Off);
        // a clip that plays and then the status says something else, or goes quiet
        let (mut w, _) = asking(40_000);
        w.update(&status("idle", "saver", 5, 40_000), 40_000);
        let start = w.start_seq;
        observe(&mut w, status("playing", "wall", start, 40_100), 40_100);
        observe(&mut w, status("idle", "wall", start, 40_600), 40_610);
        assert_eq!((w.state, w.reason.as_str()), (State::Failed, "the player stopped"));
        let (mut w, _) = asking(50_000);
        w.update(&status("idle", "saver", 5, 50_000), 50_000);
        let start = w.start_seq;
        observe(&mut w, status("playing", "wall", start, 50_100), 50_100);
        observe(&mut w, status("playing", "wall", start, 50_100), 50_000 + STALE_MS + 500);
        assert_eq!((w.state, w.reason.as_str()), (State::Failed, "the player went quiet"));
        // a player that never answers
        let (mut w, _) = asking(60_000);
        w.update(&status("idle", "saver", 5, 60_000), 60_000);
        observe(&mut w, status("idle", "saver", 5, 66_000), 60_000 + START_MS + 110);
        assert_eq!((w.state, w.reason.as_str()), (State::Failed, "the player did not answer"));
    }

    #[test]
    fn the_light_material_the_saver_and_a_home_that_is_not_in_front_are_not_asked_for_the_clip() {
        let t = video();
        for ctx in [Ctx { dark: false, ..FRONT }, Ctx { saver: true, ..FRONT }, Ctx { active: false, ..FRONT }] {
            let mut w = WallVideo::default();
            w.sync(&t, FRONT, false, 1000);
            w.update(&Input::Fetched { tag: MANIFEST_TAG, status: 200, data: manifest() }, 1000);
            let out = w.sync(&t, ctx, false, 1000);
            assert_eq!(reads(&out, STATUS_TAG), 0, "{ctx:?}");
            assert!(!w.up());
        }
        // a look other than the clip's draws the GPU wallpaper
        let mut w = WallVideo::default();
        let moved = theme(json!({"wallpaperMotion": {"source": "video", "amplitude": 9}}));
        w.sync(&moved, FRONT, false, 1000);
        w.update(&Input::Fetched { tag: MANIFEST_TAG, status: 200, data: manifest() }, 1000);
        assert_eq!(reads(&w.sync(&moved, FRONT, false, 1000), STATUS_TAG), 0);
        assert!(w.state().to_string().contains("the look is not the clip's"));
    }
}
