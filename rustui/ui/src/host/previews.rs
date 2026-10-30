//! The pictures of the inputs the picker shows: what is on an input now (live), and the last picture it showed (with its age). The helper
//! takes them (`helper/app/src/previews.rs`, LG's capture service on the video plane) and keeps them on the TV; this is the shell's side:
//! when to ask, what has come back, and the decoded pictures the picker's tiles are drawn from. Pure, as the screens' models are
//! (`update(Input, now) -> Vec<Effect>`, `due_at()`).
//!
//! When it asks, and why (the helper keeps the picture however it was asked for; the reason decides whether it goes to the flash at once):
//!
//! - **The picker comes up** over an input (`open`): one picture at once, and then, while the picker stays up, one every
//!   [`LIVE_MS`] for the tile of the input in front: that is the live tile. Three that come back black or refused in a row end the live
//!   tile for this opening: the tile shows the last picture with its age.
//! - **An input comes to the front** (`arrive`, [`ARRIVE_MS`] after it did, once the picture has settled), then every [`KEEP_MS`] while it
//!   stays (`periodic`): the last pictures are never far behind, so one is there when the person leaves or the TV goes off.
//!   Both wait while anything else of the shell is on the screen (a panel, Settings, the saver): the timers are for the quiet.
//! - **The screen goes off** with an input in front (`standby`): one more, if the picture is still there.
//!
//! Only inputs have pictures (Live TV and the HDMI ports; apps are the switcher's, which never reads the video plane), only while the
//! owner's switch is on (`inputs.previews`, Settings > Inputs & devices > Show input previews, on by default), the helper is up and the
//! screen is on. Turning the switch off forgets every picture here and has the helper delete its own (`DELETE /v1/previews`, asked again
//! at the next start when the helper was not there to hear it).

use crate::kit::picture::{self, Picture};
use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

/// The helper's routes.
pub const PATH: &str = "/v1/previews";
pub const CAPTURE: &str = "/v1/previews/capture";

/// The tags of the asks (`0x5500_0000..`).
pub mod tags {
    pub const INDEX: u64 = 0x5500_0001;
    pub const CAPTURE: u64 = 0x5500_0002;
    pub const CLEAR: u64 = 0x5500_0003;
    /// The pictures' own: a counter inside this range.
    pub const PIC_FIRST: u64 = 0x5500_0100;
    pub const PIC_LAST: u64 = 0x5500_01FF;
}

/// The time between two live pictures of the tile of the input in front while the picker is up. The capture service's cost on the set
/// decides whether this stays (`docs/research/input-preview-result.md`).
pub const LIVE_MS: u64 = 500;
/// A live picture this recent is live.
pub const LIVE_FRESH_MS: u64 = 2_500;
/// Live pictures that came back black or refused, in a row, after which the tile shows the last picture.
pub const LIVE_GIVE_UP: u32 = 3;
/// A live picture that was not taken (too soon, a failure) is asked for again after this long.
pub const LIVE_RETRY_MS: u64 = 1_500;
/// After an input comes to the front, the first picture is taken this long after.
pub const ARRIVE_MS: u64 = 3_000;
/// The quiet timer: a picture of the input in front this often.
pub const KEEP_MS: u64 = 30_000;
/// A timer that found the shell busy tries again after this long.
pub const RETRY_MS: u64 = 5_000;
/// An ask the helper never answered is let go after this long.
pub const INFLIGHT_MS: u64 = 9_000;
/// After the helper says it keeps no pictures (its own reading of the switch disagrees), nothing is asked for this long.
pub const PARK_MS: u64 = 60_000;

/// The key of the input an app id is: `livetv`, `hdmi1`..`hdmi9`; any other app has none.
pub fn key_of(app: &str) -> Option<String> {
    if app == "com.webos.app.livetv" {
        return Some("livetv".into());
    }
    let d = app.strip_prefix("com.webos.app.hdmi")?;
    (d.len() == 1 && matches!(d.as_bytes()[0], b'1'..=b'9')).then(|| format!("hdmi{d}"))
}

/// What the previews need to know of the TV, as the panels' environment has it.
#[derive(Clone, Copy, Debug)]
pub struct Facts<'a> {
    /// The owner's switch (`inputs.previews`).
    pub on: bool,
    pub helper_up: bool,
    pub screen_on: bool,
    /// The app in front, whether it is Home, and whether the compositor has said yet.
    pub app: &'a str,
    pub home: bool,
    pub known: bool,
}

/// What a tile shows of a picture: its revision (it changes with each new picture), whether it is live now, and the words for its age.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    pub rev: u64,
    pub live: bool,
    pub age: String,
}

/// How long ago, in few words: "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago".
pub fn age_text(now_ms: u64, at_ms: u64) -> String {
    let s = now_ms.saturating_sub(at_ms) / 1000;
    match s {
        0..45 => "just now".into(),
        45..3600 => format!("{} min ago", (s / 60).max(1)),
        3600..86_400 => format!("{} h ago", s / 3600),
        86_400..172_800 => "yesterday".into(),
        _ => format!("{} days ago", s / 86_400),
    }
}

struct Held {
    at: u64,
    rev: u64,
}

struct Inflight {
    key: String,
    why: &'static str,
    at_ms: u64,
}

/// The bytes of a picture that arrived while the picker was moving.
struct Pending {
    key: String,
    at: u64,
    bytes: Vec<u8>,
}

pub struct Previews {
    /// This process answers the picker (the only kind that shows the pictures).
    active: bool,
    on: bool,
    helper_up: bool,
    screen_on: bool,
    /// The key of the input in front (none at Home or in an app).
    front: Option<String>,
    /// The picker is up.
    open: bool,
    /// What the helper says it keeps: key to when the picture was taken.
    index: BTreeMap<String, u64>,
    held: BTreeMap<String, Held>,
    pics: HashMap<String, Picture>,
    /// The asks for a picture's bytes in flight: the tag to the key and the time the helper gave it.
    wanted: HashMap<u64, (String, u64)>,
    next_tag: u64,
    inflight: Option<Inflight>,
    arrive_due: Option<u64>,
    keep_due: Option<u64>,
    live_due: Option<u64>,
    live_fails: u32,
    /// When the tile of the input in front last had a live picture.
    live_ok: Option<(String, u64)>,
    lapse_due: Option<u64>,
    index_owed: bool,
    clear_owed: bool,
    parked_until: u64,
    /// Decoding waits for this time: the picker is opening or its focus is moving, and a decode of some milliseconds in the middle of an
    /// animation is a frame the person sees dropped.
    calm_at: u64,
    /// Pictures that came while it was not calm, not decoded yet (the newest of each input).
    pending: Vec<Pending>,
}

impl Default for Previews {
    fn default() -> Self {
        Self::new()
    }
}

impl Previews {
    pub fn new() -> Self {
        Self {
            active: false,
            on: true,
            helper_up: false,
            screen_on: true,
            front: None,
            open: false,
            index: BTreeMap::new(),
            held: BTreeMap::new(),
            pics: HashMap::new(),
            wanted: HashMap::new(),
            next_tag: tags::PIC_FIRST,
            inflight: None,
            arrive_due: None,
            keep_due: None,
            live_due: None,
            live_fails: 0,
            live_ok: None,
            lapse_due: None,
            index_owed: true,
            clear_owed: false,
            parked_until: 0,
            calm_at: 0,
            pending: Vec::new(),
        }
    }

    /// The shell answers the picker: the pictures are wanted. Without it nothing is asked.
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    /// Whether the previews are in use at all: the picker is ours and the owner has them on.
    pub fn is_on(&self) -> bool {
        self.active && self.on
    }

    fn can_ask(&self, now: u64) -> bool {
        self.active && self.on && self.helper_up && now >= self.parked_until
    }

    fn clear_ask() -> Effect {
        Effect::Request { tag: tags::CLEAR, method: "DELETE".into(), path: PATH.into(), body: None }
    }

    fn index_ask() -> Effect {
        Effect::Fetch { tag: tags::INDEX, path: PATH.into() }
    }

    fn forget_all(&mut self) {
        self.index.clear();
        self.pending.clear();
        self.held.clear();
        self.pics.clear();
        self.wanted.clear();
        self.inflight = None;
        self.arrive_due = None;
        self.keep_due = None;
        self.live_due = None;
        self.live_ok = None;
        self.lapse_due = None;
    }

    /// Brings the model in line with what the TV is doing: the switch, the helper, the screen and the input in front. Returns what that asks
    /// for (the delete after the switch went off, the index when the helper came up, the last picture as the screen goes off).
    fn sync(&mut self, f: &Facts, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if !self.active {
            return out;
        }
        let key = if f.known && !f.home { key_of(f.app) } else { None };
        let screen_fell = self.screen_on && !f.screen_on;
        let screen_rose = !self.screen_on && f.screen_on;
        // the last picture as the screen goes: it may already be gone, and the helper then keeps the one before
        if screen_fell && self.front.is_some() && self.can_ask(now) {
            let k = self.front.clone().unwrap_or_default();
            out.extend(self.ask(&k, "standby", now));
        }
        self.screen_on = f.screen_on;
        if self.on && !f.on {
            // off: nothing is kept anywhere
            self.forget_all();
            self.clear_owed = true;
            out.push(Effect::Redraw);
        }
        if !self.on && f.on {
            self.index_owed = true;
            self.arrive_due = key.as_ref().map(|_| now + ARRIVE_MS);
        }
        self.on = f.on;
        if !self.helper_up && f.helper_up {
            self.index_owed = true;
            // a start with the switch off asks the helper to hold nothing, once it is there
            self.clear_owed |= !self.on;
        }
        self.helper_up = f.helper_up;
        if key != self.front {
            self.front = key.clone();
            self.live_ok = None;
            self.lapse_due = None;
            self.live_fails = 0;
            self.keep_due = None;
            self.arrive_due = key.as_ref().map(|_| now + ARRIVE_MS);
            if self.open {
                self.live_due = key.as_ref().map(|_| now + LIVE_MS);
            }
            out.push(Effect::Redraw);
        }
        if screen_rose && self.front.is_some() {
            self.arrive_due = Some(now + ARRIVE_MS);
            self.keep_due = None;
        }
        if self.helper_up {
            if self.clear_owed {
                self.clear_owed = false;
                out.push(Self::clear_ask());
            }
            if self.index_owed && self.on {
                self.index_owed = false;
                out.push(Self::index_ask());
            }
        }
        out
    }

    /// A picture of `key` is asked for, unless another is on its way.
    fn ask(&mut self, key: &str, why: &'static str, now: u64) -> Vec<Effect> {
        if self.inflight.is_some() || !self.can_ask(now) {
            return Vec::new();
        }
        self.inflight = Some(Inflight { key: key.to_owned(), why, at_ms: now });
        vec![Effect::Request { tag: tags::CAPTURE, method: "POST".into(), path: CAPTURE.into(), body: Some(json!({"key": key, "why": why})) }]
    }

    /// The picture of `key` the helper dates `at`, unless it is held or on its way.
    fn fetch_picture(&mut self, key: &str, at: u64) -> Vec<Effect> {
        if self.held.get(key).is_some_and(|h| h.at >= at) || self.wanted.values().any(|(k, a)| k == key && *a >= at) {
            return Vec::new();
        }
        self.next_tag = if self.next_tag >= tags::PIC_LAST { tags::PIC_FIRST } else { self.next_tag + 1 };
        let tag = self.next_tag;
        self.wanted.insert(tag, (key.to_owned(), at));
        vec![Effect::Fetch { tag, path: format!("{PATH}/{key}") }]
    }

    /// The picker came up: the index (what the helper has kept) and, over an input, a picture at once.
    pub fn opened(&mut self, f: &Facts, now: u64) -> Vec<Effect> {
        let mut out = self.sync(f, now);
        self.open = true;
        self.live_fails = 0;
        self.live_due = None;
        if self.can_ask(now) {
            self.index_owed = false;
            out.push(Self::index_ask());
            if self.screen_on
                && let Some(k) = self.front.clone()
            {
                let asked = self.ask(&k, "open", now);
                if asked.is_empty() {
                    self.live_due = Some(now + LIVE_MS);
                }
                out.extend(asked);
            }
        }
        out
    }

    /// The picker went away: the live tile is over, and the quiet timer takes the input in front again.
    pub fn closed(&mut self, now: u64) {
        self.open = false;
        self.live_due = None;
        self.live_ok = None;
        self.lapse_due = None;
        if self.front.is_some() {
            self.keep_due = Some(now + KEEP_MS);
        }
    }

    /// An input of the host. `facts` are as the panels' environment has them after this input.
    pub fn update(&mut self, input: &Input, f: &Facts, quiet: bool, now: u64) -> (Vec<Effect>, bool) {
        let mut out = self.sync(f, now);
        let mut changed = out.iter().any(|e| matches!(e, Effect::Redraw));
        match input {
            Input::Fetched { tag, status, data } if *tag == tags::CAPTURE => {
                let (e, c) = self.captured(*status, data, now);
                out.extend(e);
                changed |= c;
            }
            Input::Fetched { tag, status, data } if *tag == tags::INDEX => out.extend(self.indexed(*status, data, now)),
            Input::Bytes { tag, status, bytes } if (tags::PIC_FIRST..=tags::PIC_LAST).contains(tag) => changed |= self.arrived(*tag, *status, bytes, now),
            Input::Tick => {
                let (e, c) = self.tick(quiet, now);
                out.extend(e);
                changed |= c;
            }
            _ => {}
        }
        (out, changed)
    }

    /// The timers: the live tile while the picker is up, the quiet timers otherwise.
    fn tick(&mut self, quiet: bool, now: u64) -> (Vec<Effect>, bool) {
        let mut out = Vec::new();
        let mut changed = false;
        // pictures that waited for the picker to be still
        if !self.pending.is_empty() && now >= self.calm_at {
            for p in std::mem::take(&mut self.pending) {
                changed |= self.store(p.key, p.at, &p.bytes);
            }
            if changed {
                out.push(Effect::Redraw);
            }
        }
        if self.inflight.as_ref().is_some_and(|i| now.saturating_sub(i.at_ms) > INFLIGHT_MS) {
            self.inflight = None;
            if self.open {
                self.live_due = Some(now + LIVE_RETRY_MS);
            }
        }
        if self.lapse_due.is_some_and(|d| now >= d) {
            // the last live picture is no longer live: the tile says how old it is
            self.lapse_due = None;
            changed = true;
            out.push(Effect::Redraw);
        }
        if !(self.active && self.on && self.helper_up && self.screen_on) {
            return (out, changed);
        }
        let Some(key) = self.front.clone() else { return (out, changed) };
        if self.open {
            if self.live_due.is_some_and(|d| now >= d) {
                self.live_due = None;
                let asked = self.ask(&key, "live", now);
                if asked.is_empty() && self.inflight.is_some() {
                    // one is on its way: its answer asks for the next
                } else if asked.is_empty() {
                    self.live_due = Some(now + LIVE_RETRY_MS);
                }
                out.extend(asked);
            }
            return (out, changed);
        }
        if self.arrive_due.is_some_and(|d| now >= d) {
            if quiet {
                self.arrive_due = None;
                self.keep_due = Some(now + KEEP_MS);
                out.extend(self.ask(&key, "arrive", now));
            } else {
                self.arrive_due = Some(now + RETRY_MS);
            }
        } else if self.keep_due.is_some_and(|d| now >= d) {
            if quiet {
                self.keep_due = Some(now + KEEP_MS);
                out.extend(self.ask(&key, "periodic", now));
            } else {
                self.keep_due = Some(now + RETRY_MS);
            }
        }
        (out, changed)
    }

    /// The helper's answer to a capture.
    fn captured(&mut self, status: u16, data: &Value, now: u64) -> (Vec<Effect>, bool) {
        let Some(asked) = self.inflight.take() else { return (Vec::new(), false) };
        let mut out = Vec::new();
        let live_now = self.open && self.front.as_deref() == Some(asked.key.as_str());
        if status == 200 {
            let at = data.get("at").and_then(Value::as_u64).unwrap_or(now);
            self.index.insert(asked.key.clone(), at);
            if self.front.as_deref() == Some(asked.key.as_str()) {
                // a picture of the input in front was just taken: the quiet timer starts from it
                self.arrive_due = None;
                self.keep_due = Some(now + KEEP_MS);
            }
            if live_now {
                self.live_ok = Some((asked.key.clone(), now));
                self.lapse_due = Some(now + LIVE_FRESH_MS);
                self.live_fails = 0;
                self.live_due = Some((asked.at_ms + LIVE_MS).max(now));
            }
            out.extend(self.fetch_picture(&asked.key, at));
            return (out, false);
        }
        let code = data.pointer("/error/code").and_then(Value::as_str).unwrap_or("");
        match code {
            // the helper holds that no picture is wanted: the switch it reads says off while this one says on
            "previews_disabled" => self.parked_until = now + PARK_MS,
            // a moment ago, or one at a time: the same ask again shortly
            "too_soon" | "busy" => {
                if live_now {
                    self.live_due = Some(now + LIVE_MS);
                }
            }
            "screen_off" => {}
            // black or refused (the picture is protected, there is no signal, nothing plays), or a fault: the last picture stays
            _ => {
                if live_now {
                    self.live_fails += 1;
                    self.live_due = (self.live_fails < LIVE_GIVE_UP).then_some(now + if code == "no_picture" { LIVE_MS * 2 } else { LIVE_RETRY_MS });
                    if self.lapse_due.is_none() {
                        self.lapse_due = Some(now + LIVE_FRESH_MS);
                    }
                }
            }
        }
        (out, false)
    }

    /// The helper's index: what it keeps, and when each picture was taken.
    fn indexed(&mut self, status: u16, data: &Value, now: u64) -> Vec<Effect> {
        if status != 200 {
            return Vec::new();
        }
        let Some(p) = data.get("previews") else { return Vec::new() };
        if p.get("enabled").and_then(Value::as_bool) == Some(false) {
            // the helper reads the owner's switch as off; this side reads it as on: not asked again for a while
            if self.on {
                self.parked_until = now + PARK_MS;
            }
            return Vec::new();
        }
        let items = p.get("items").and_then(Value::as_object);
        let mut index = BTreeMap::new();
        for (k, v) in items.into_iter().flatten() {
            if let Some(at) = v.get("at").and_then(Value::as_u64) {
                index.insert(k.clone(), at);
            }
        }
        // a picture the helper no longer has is let go here
        self.held.retain(|k, _| index.contains_key(k));
        self.pics.retain(|k, _| index.contains_key(k));
        self.index = index;
        let want: Vec<(String, u64)> = self.index.iter().map(|(k, a)| (k.clone(), *a)).collect();
        want.into_iter().flat_map(|(k, a)| self.fetch_picture(&k, a)).collect()
    }

    /// A picture's bytes came back. Returns whether a tile's picture is another now (not while the picker is not calm: then the bytes wait).
    fn arrived(&mut self, tag: u64, status: u16, bytes: &[u8], now: u64) -> bool {
        let Some((key, at)) = self.wanted.remove(&tag) else { return false };
        if status != 200 {
            if status == 404 {
                self.index.remove(&key);
            }
            return false;
        }
        // an older answer that came after a newer one changes nothing
        if self.held.get(&key).is_some_and(|h| h.at > at) {
            return false;
        }
        if now < self.calm_at {
            self.pending.retain(|p| p.key != key || p.at > at);
            if !self.pending.iter().any(|p| p.key == key) {
                self.pending.push(Pending { key, at, bytes: bytes.to_vec() });
            }
            return false;
        }
        self.store(key, at, bytes)
    }

    /// Decodes a picture and keeps it. Returns whether it was one.
    fn store(&mut self, key: String, at: u64, bytes: &[u8]) -> bool {
        match picture::decode(bytes) {
            Ok(pic) => {
                let rev = self.held.get(&key).map_or(1, |h| h.rev + 1);
                self.pics.insert(key.clone(), pic);
                self.held.insert(key, Held { at, rev });
                true
            }
            Err(_) => false,
        }
    }

    /// The picker is opening or its focus is moving: pictures that arrive are decoded at `t` or after, not while it moves.
    pub fn hold_until(&mut self, t: u64) {
        self.calm_at = self.calm_at.max(t);
    }

    /// What the tile of `key` shows of its picture, if it has one.
    pub fn shot(&self, key: &str, now: u64) -> Option<Shot> {
        if !self.active || !self.on {
            return None;
        }
        let h = self.held.get(key)?;
        let live =
            self.open && self.front.as_deref() == Some(key) && self.live_ok.as_ref().is_some_and(|(k, t)| k == key && now.saturating_sub(*t) < LIVE_FRESH_MS);
        Some(Shot { rev: h.rev, live, age: if live { "live".into() } else { age_text(now, h.at) } })
    }

    /// The decoded picture of `key`.
    pub fn picture(&self, key: &str) -> Option<&Picture> {
        self.pics.get(key)
    }

    /// When the model next wants a tick.
    pub fn due_at(&self) -> Option<u64> {
        if !self.active {
            return None;
        }
        let mut due = [self.inflight.as_ref().map(|i| i.at_ms + INFLIGHT_MS + 1), self.lapse_due, (!self.pending.is_empty()).then_some(self.calm_at)]
            .into_iter()
            .flatten()
            .min();
        if self.on && self.helper_up && self.screen_on && self.front.is_some() {
            let timers = if self.open { [self.live_due, None, None] } else { [self.arrive_due, self.keep_due, None] };
            due = [due, timers.into_iter().flatten().min()].into_iter().flatten().min();
        }
        due
    }

    /// For the harness's report: what the model holds.
    pub fn report(&self) -> Value {
        json!({
            "on": self.on,
            "front": self.front,
            "open": self.open,
            "held": self.held.iter().map(|(k, h)| (k.clone(), json!({"at": h.at, "rev": h.rev}))).collect::<serde_json::Map<_, _>>(),
            "inflight": self.inflight.as_ref().map(|i| json!({"key": i.key, "why": i.why})),
            "liveFails": self.live_fails,
            "pending": self.pending.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HDMI2: &str = "com.webos.app.hdmi2";
    const HOME: &str = "com.webos.app.home";

    fn facts(app: &str, home: bool) -> Facts<'_> {
        Facts { on: true, helper_up: true, screen_on: true, app, home, known: true }
    }

    fn bmp(w: u32, h: u32, bgr: [u8; 3]) -> Vec<u8> {
        let stride = (w * 3).div_ceil(4) * 4;
        let mut b = vec![0u8; 54];
        b[0..2].copy_from_slice(b"BM");
        b[2..6].copy_from_slice(&(54 + stride * h).to_le_bytes());
        b[10..14].copy_from_slice(&54u32.to_le_bytes());
        b[14..18].copy_from_slice(&40u32.to_le_bytes());
        b[18..22].copy_from_slice(&w.to_le_bytes());
        b[22..26].copy_from_slice(&h.to_le_bytes());
        b[26..28].copy_from_slice(&1u16.to_le_bytes());
        b[28..30].copy_from_slice(&24u16.to_le_bytes());
        for _ in 0..h {
            for _ in 0..w {
                b.extend_from_slice(&bgr);
            }
            b.extend(std::iter::repeat_n(0u8, (stride - w * 3) as usize));
        }
        b
    }

    /// A model the picker is ours for, with the helper up and HDMI 2 in front at time 0, and nothing asked yet.
    fn up() -> Previews {
        let mut p = Previews::new();
        p.set_active(true);
        p.update(&Input::Tick, &facts(HDMI2, false), true, 0);
        p
    }

    fn tick(p: &mut Previews, now: u64) -> Vec<Effect> {
        p.update(&Input::Tick, &facts(HDMI2, false), true, now).0
    }

    fn posts(fx: &[Effect]) -> Vec<(String, String)> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::Request { tag, method, path, body } if *tag == tags::CAPTURE => {
                    assert_eq!((method.as_str(), path.as_str()), ("POST", CAPTURE));
                    let b = body.as_ref().unwrap();
                    Some((b["key"].as_str().unwrap().to_owned(), b["why"].as_str().unwrap().to_owned()))
                }
                _ => None,
            })
            .collect()
    }

    fn fetches(fx: &[Effect]) -> Vec<(u64, String)> {
        fx.iter().filter_map(|e| if let Effect::Fetch { tag, path } = e { Some((*tag, path.clone())) } else { None }).collect()
    }

    /// The helper's answer to the capture in flight, as it reaches the model with `f` in force.
    fn answer_with(p: &mut Previews, f: &Facts, status: u16, data: Value, now: u64) -> Vec<Effect> {
        p.update(&Input::Fetched { tag: tags::CAPTURE, status, data }, f, true, now).0
    }

    fn answer(p: &mut Previews, status: u16, data: Value, now: u64) -> Vec<Effect> {
        answer_with(p, &facts(HDMI2, false), status, data, now)
    }

    fn ok_at(at: u64) -> Value {
        json!({"ok": true, "key": "hdmi2", "at": at})
    }

    fn fail(code: &str) -> Value {
        json!({"ok": false, "error": {"code": code, "message": "x"}})
    }

    /// The bytes of the picture the first fetch in `fx` asked for.
    fn deliver(p: &mut Previews, fx: &[Effect], bytes: Vec<u8>, now: u64) -> bool {
        let (tag, _) = fetches(fx).into_iter().next().expect("a fetch");
        p.update(&Input::Bytes { tag, status: 200, bytes }, &facts(HDMI2, false), true, now).1
    }

    fn requests(fx: &[Effect]) -> usize {
        fx.iter().filter(|e| matches!(e, Effect::Request { .. } | Effect::Fetch { .. })).count()
    }

    #[test]
    fn an_app_id_is_an_input_when_it_is_live_tv_or_an_hdmi_port() {
        assert_eq!(key_of("com.webos.app.livetv").as_deref(), Some("livetv"));
        assert_eq!(key_of("com.webos.app.hdmi1").as_deref(), Some("hdmi1"));
        assert_eq!(key_of("com.webos.app.hdmi4").as_deref(), Some("hdmi4"));
        for app in ["netflix", HOME, "com.webos.app.hdmi0", "com.webos.app.hdmi10", "com.webos.app.hdmi", "", "com.webos.app.hdmix"] {
            assert_eq!(key_of(app), None, "{app}");
        }
    }

    #[test]
    fn age_is_said_in_few_words() {
        let at = 1_000_000_000_000u64;
        let s = |secs: u64| age_text(at + secs * 1000, at);
        assert_eq!(s(0), "just now");
        assert_eq!(s(44), "just now");
        assert_eq!(s(45), "1 min ago");
        assert_eq!(s(119), "1 min ago");
        assert_eq!(s(120), "2 min ago");
        assert_eq!(s(59 * 60), "59 min ago");
        assert_eq!(s(3600), "1 h ago");
        assert_eq!(s(23 * 3600 + 3599), "23 h ago");
        assert_eq!(s(86_400), "yesterday");
        assert_eq!(s(86_400 * 2 - 1), "yesterday");
        assert_eq!(s(86_400 * 2), "2 days ago");
        assert_eq!(s(86_400 * 29), "29 days ago");
        assert_eq!(age_text(at - 5000, at), "just now", "a clock that is behind the picture's is not a negative age");
    }

    #[test]
    fn the_picker_over_an_input_asks_for_the_index_and_a_picture_at_once_and_the_live_tile_goes_on_while_it_is_up() {
        let mut p = up();
        let fx = p.opened(&facts(HDMI2, false), 1000);
        assert_eq!(posts(&fx), vec![("hdmi2".to_owned(), "open".to_owned())]);
        assert!(fx.contains(&Effect::Fetch { tag: tags::INDEX, path: PATH.into() }));
        assert_eq!(p.shot("hdmi2", 1000), None, "nothing yet");
        // the answer asks for the picture's bytes
        let fx = answer(&mut p, 200, ok_at(1100), 1150);
        let (tag, path) = fetches(&fx).remove(0);
        assert_eq!(path, "/v1/previews/hdmi2");
        assert!((tags::PIC_FIRST..=tags::PIC_LAST).contains(&tag));
        assert!(deliver(&mut p, &fx, bmp(32, 18, [10, 20, 200]), 1200), "a tile has a picture now");
        let shot = p.shot("hdmi2", 1250).unwrap();
        assert_eq!((shot.live, shot.age.as_str(), shot.rev), (true, "live", 1));
        assert_eq!(p.picture("hdmi2").map(|q| (q.width, q.height)), Some((32, 18)));
        // the next live picture is due LIVE_MS after the last was asked for
        assert_eq!(p.due_at(), Some(1000 + LIVE_MS));
        assert!(posts(&tick(&mut p, 1400)).is_empty());
        let fx = tick(&mut p, 1650);
        assert_eq!(posts(&fx), vec![("hdmi2".to_owned(), "live".to_owned())]);
        // one at a time: nothing is asked while that one is out
        assert!(posts(&tick(&mut p, 1700)).is_empty());
        let fx = answer(&mut p, 200, ok_at(1700), 1750);
        assert!(deliver(&mut p, &fx, bmp(32, 18, [200, 20, 10]), 1800));
        assert_eq!(p.shot("hdmi2", 1800).unwrap().rev, 2, "a new picture is a new revision");
    }

    #[test]
    fn when_the_pictures_stop_coming_the_live_tile_gives_way_to_the_last_picture_and_its_age() {
        let mut p = up();
        p.opened(&facts(HDMI2, false), 1000);
        let fx = answer(&mut p, 200, ok_at(1000), 1100);
        deliver(&mut p, &fx, bmp(16, 9, [1, 2, 3]), 1150);
        assert!(p.shot("hdmi2", 1200).unwrap().live);
        // three that come back black in a row end the live tile for this opening
        let mut t = 1700;
        for n in 0..LIVE_GIVE_UP {
            let fx = tick(&mut p, t);
            assert_eq!(posts(&fx).len(), 1, "ask {n}");
            answer(&mut p, 409, fail("no_picture"), t + 50);
            t += LIVE_MS * 3;
        }
        assert!(posts(&tick(&mut p, t + 10_000)).is_empty(), "no more live asks");
        // the live word lapses by itself and the age takes its place
        let shot = p.shot("hdmi2", 1100 + LIVE_FRESH_MS + 1).unwrap();
        assert!(!shot.live);
        assert_eq!(shot.age, "just now");
        assert_eq!(p.shot("hdmi2", 1000 + 5 * 60_000).unwrap().age, "5 min ago");
    }

    #[test]
    fn the_tile_is_told_to_be_drawn_again_when_its_live_picture_goes_stale() {
        let mut p = up();
        p.opened(&facts(HDMI2, false), 1000);
        let fx = answer(&mut p, 200, ok_at(1000), 1100);
        deliver(&mut p, &fx, bmp(16, 9, [1, 2, 3]), 1150);
        assert_eq!(p.due_at(), Some(1000 + LIVE_MS), "the next live ask is due first");
        let (_, early) = p.update(&Input::Tick, &facts(HDMI2, false), true, 1200);
        assert!(!early);
        // nothing answers the next ask: the live word is on the tile until the freshness runs out, then the tile is drawn without it
        let fx = tick(&mut p, 1650);
        assert_eq!(posts(&fx).len(), 1);
        let (_, changed) = p.update(&Input::Tick, &facts(HDMI2, false), true, 1100 + LIVE_FRESH_MS);
        assert!(changed, "the live word is to come off");
    }

    #[test]
    fn a_failure_of_the_service_or_a_busy_one_is_asked_again_and_not_counted_as_black() {
        let mut p = up();
        p.opened(&facts(HDMI2, false), 1000);
        answer(&mut p, 429, fail("too_soon"), 1050);
        assert_eq!(p.live_fails, 0);
        assert_eq!(posts(&tick(&mut p, 1060 + LIVE_MS)).len(), 1);
        answer(&mut p, 502, fail("capture_failed"), 1700);
        assert_eq!(p.live_fails, 1);
        assert_eq!(posts(&tick(&mut p, 1700 + LIVE_RETRY_MS)).len(), 1, "tried again soon");
        // the helper that says the switch is off parks the previews for a while
        answer(&mut p, 403, fail("previews_disabled"), 4000);
        p.closed(4100);
        assert_eq!(requests(&p.opened(&facts(HDMI2, false), 5000)), 0, "parked: not even the index is asked for");
        p.closed(5100);
        let fx = p.opened(&facts(HDMI2, false), 4000 + PARK_MS + 1);
        assert_eq!(posts(&fx), vec![("hdmi2".to_owned(), "open".to_owned())], "asked again once the pause is over");
    }

    #[test]
    fn nothing_is_asked_over_home_or_an_app_or_with_the_helper_away_or_the_screen_off() {
        let mut p = up();
        let home = facts(HOME, true);
        let fx = p.opened(&home, 1000);
        assert!(posts(&fx).is_empty(), "Home is in front: nothing is playing on an input");
        p.closed(1100);
        let app = facts("netflix", false);
        assert!(posts(&p.opened(&app, 1200)).is_empty());
        p.closed(1300);
        let away = Facts { helper_up: false, ..facts(HDMI2, false) };
        assert_eq!(requests(&p.opened(&away, 1400)), 0);
        p.closed(1500);
        let dark = Facts { screen_on: false, ..facts(HDMI2, false) };
        let fx = p.opened(&dark, 1600);
        assert!(posts(&fx).is_empty(), "the screen is off: {fx:?}");
    }

    #[test]
    fn an_input_that_comes_to_the_front_is_pictured_after_it_has_settled_and_then_every_half_minute_while_the_shell_is_quiet() {
        let mut p = Previews::new();
        p.set_active(true);
        p.update(&Input::Tick, &facts(HOME, true), true, 0);
        assert_eq!(p.due_at(), None, "Home is in front: no timer");
        p.update(&Input::Topic { topic: "foreground".into(), data: json!({}) }, &facts(HDMI2, false), true, 10_000);
        assert_eq!(p.due_at(), Some(10_000 + ARRIVE_MS));
        assert!(posts(&tick(&mut p, 12_999)).is_empty());
        let fx = tick(&mut p, 13_000);
        assert_eq!(posts(&fx), vec![("hdmi2".to_owned(), "arrive".to_owned())]);
        answer(&mut p, 200, ok_at(13_050), 13_100);
        assert_eq!(p.due_at(), Some(13_100 + KEEP_MS), "the quiet timer runs from the picture just taken");
        let fx = tick(&mut p, 13_100 + KEEP_MS);
        assert_eq!(posts(&fx), vec![("hdmi2".to_owned(), "periodic".to_owned())]);
        let t = 13_100 + KEEP_MS;
        answer(&mut p, 200, ok_at(t), t + 50);
        // busy: the timer waits and tries again; nothing is asked meanwhile
        let t = t + 50 + KEEP_MS;
        let busy = p.update(&Input::Tick, &facts(HDMI2, false), false, t).0;
        assert!(posts(&busy).is_empty());
        assert_eq!(p.due_at(), Some(t + RETRY_MS));
        let fx = tick(&mut p, t + RETRY_MS);
        assert_eq!(posts(&fx), vec![("hdmi2".to_owned(), "periodic".to_owned())]);
        answer(&mut p, 200, ok_at(t + RETRY_MS), t + RETRY_MS + 20);
        // leaving the input stops the timers
        p.update(&Input::Tick, &facts(HOME, true), true, t + RETRY_MS + 100);
        assert_eq!(p.due_at(), None);
    }

    #[test]
    fn the_screen_going_off_with_an_input_in_front_asks_for_one_last_picture_and_the_timers_rest() {
        let mut p = up();
        tick(&mut p, 1000);
        let off = Facts { screen_on: false, ..facts(HDMI2, false) };
        let fx = p.update(&Input::Topic { topic: "screen".into(), data: json!({"on": false}) }, &off, true, 5000).0;
        assert_eq!(posts(&fx), vec![("hdmi2".to_owned(), "standby".to_owned())]);
        answer_with(&mut p, &off, 409, fail("no_picture"), 5100);
        assert_eq!(p.due_at(), None, "no timer while the screen is off");
        assert!(posts(&p.update(&Input::Tick, &off, true, 100_000).0).is_empty());
        // on again: the arrival timer starts over
        p.update(&Input::Topic { topic: "screen".into(), data: json!({"on": true}) }, &facts(HDMI2, false), true, 200_000);
        assert_eq!(p.due_at(), Some(200_000 + ARRIVE_MS));
    }

    #[test]
    fn turning_the_switch_off_forgets_the_pictures_and_has_the_helper_delete_its_own_once() {
        let mut p = up();
        p.opened(&facts(HDMI2, false), 1000);
        let fx = answer(&mut p, 200, ok_at(1000), 1100);
        deliver(&mut p, &fx, bmp(16, 9, [1, 2, 3]), 1150);
        assert!(p.shot("hdmi2", 1200).is_some());
        let off = Facts { on: false, ..facts(HDMI2, false) };
        let (fx, changed) = p.update(&Input::Tick, &off, true, 1300);
        assert!(changed);
        assert_eq!(fx.iter().filter(|e| matches!(e, Effect::Request { tag, method, .. } if *tag == tags::CLEAR && method == "DELETE")).count(), 1);
        assert_eq!((p.shot("hdmi2", 1300), p.picture("hdmi2").is_none()), (None, true));
        assert_eq!(requests(&p.update(&Input::Tick, &off, true, 1400).0), 0, "once");
        assert_eq!(p.due_at(), None);
        assert_eq!(requests(&p.opened(&off, 1500)), 0, "an opened picker asks for nothing while the switch is off");
        // on again: pictures come back
        let fx = p.update(&Input::Tick, &facts(HDMI2, false), true, 2000).0;
        assert!(fx.contains(&Effect::Fetch { tag: tags::INDEX, path: PATH.into() }));
    }

    #[test]
    fn a_delete_the_helper_was_not_there_to_hear_is_asked_again_when_it_is_and_a_start_with_the_switch_off_asks_for_one() {
        let mut p = Previews::new();
        p.set_active(true);
        let gone = Facts { helper_up: false, on: false, ..facts(HDMI2, false) };
        assert_eq!(requests(&p.update(&Input::Tick, &gone, true, 0).0), 0, "nothing to say to a helper that is not there");
        let back = Facts { on: false, ..facts(HDMI2, false) };
        let fx = p.update(&Input::Helper { topic: "up".into(), data: json!(true) }, &back, true, 100).0;
        assert_eq!(fx.iter().filter(|e| matches!(e, Effect::Request { tag, .. } if *tag == tags::CLEAR)).count(), 1);
        assert!(!fx.contains(&Effect::Fetch { tag: tags::INDEX, path: PATH.into() }), "no index while the switch is off");
    }

    #[test]
    fn the_index_brings_the_last_pictures_a_restart_left_and_only_what_is_newer_than_what_is_held() {
        let mut p = up();
        let doc = json!({"ok": true, "previews": {"enabled": true, "items": {"hdmi1": {"at": 1000}, "livetv": {"at": 2000}}}});
        let fx = p.update(&Input::Fetched { tag: tags::INDEX, status: 200, data: doc.clone() }, &facts(HDMI2, false), true, 5000).0;
        let f = fetches(&fx);
        assert_eq!(f.len(), 2, "{f:?}");
        for (tag, _) in &f {
            p.update(&Input::Bytes { tag: *tag, status: 200, bytes: bmp(16, 9, [9, 9, 9]) }, &facts(HDMI2, false), true, 5100);
        }
        // dated by the helper's time, not by when it arrived
        assert_eq!(p.shot("hdmi1", 1000 + 5_000_000).unwrap().age, "1 h ago");
        assert_eq!(p.shot("livetv", 2000 + 90_000).unwrap().age, "1 min ago");
        // the same index again: nothing to fetch
        let fx = p.update(&Input::Fetched { tag: tags::INDEX, status: 200, data: doc }, &facts(HDMI2, false), true, 6000).0;
        assert!(fetches(&fx).is_empty());
        // a newer picture of one: that one only; a picture the helper dropped is dropped here
        let doc = json!({"ok": true, "previews": {"enabled": true, "items": {"hdmi1": {"at": 9000}}}});
        let fx = p.update(&Input::Fetched { tag: tags::INDEX, status: 200, data: doc }, &facts(HDMI2, false), true, 7000).0;
        assert_eq!(fetches(&fx).len(), 1);
        assert!(p.shot("livetv", 7000).is_none() && p.picture("livetv").is_none());
    }

    #[test]
    fn a_helper_that_reads_the_switch_as_off_parks_the_previews() {
        let mut p = up();
        let doc = json!({"ok": true, "previews": {"enabled": false, "items": {}}});
        p.update(&Input::Fetched { tag: tags::INDEX, status: 200, data: doc }, &facts(HDMI2, false), true, 1000);
        assert!(posts(&p.opened(&facts(HDMI2, false), 2000)).is_empty());
        p.closed(2100);
        assert_eq!(posts(&p.opened(&facts(HDMI2, false), 1000 + PARK_MS + 1)).len(), 1);
    }

    /// The next live ask, made at `ask_at`, answered at once with a picture dated `at`.
    fn tick_and_answer(p: &mut Previews, ask_at: u64, at: u64) -> Vec<Effect> {
        let fx = tick(p, ask_at);
        assert_eq!(posts(&fx).len(), 1, "a live ask at {ask_at}");
        answer(p, 200, ok_at(at), ask_at + 20)
    }

    #[test]
    fn a_picture_that_is_not_one_a_tag_nobody_asked_for_and_an_old_picture_arriving_late_change_nothing() {
        let mut p = up();
        p.opened(&facts(HDMI2, false), 1000);
        let fx = answer(&mut p, 200, ok_at(1000), 1100);
        let (tag, _) = fetches(&fx).remove(0);
        // the bytes are not a picture: no tile, no panic
        assert!(!p.update(&Input::Bytes { tag, status: 200, bytes: b"<html>".to_vec() }, &facts(HDMI2, false), true, 1200).1);
        assert!(p.shot("hdmi2", 1200).is_none());
        assert!(!p.update(&Input::Bytes { tag: tags::PIC_FIRST + 77, status: 200, bytes: bmp(4, 4, [1, 1, 1]) }, &facts(HDMI2, false), true, 1300).1);
        // two pictures asked for one after the other, their bytes coming back the other way round: the newer stays
        let first = tick_and_answer(&mut p, 1700, 2000);
        let second = tick_and_answer(&mut p, 2700, 3000);
        let (t1, t2) = (fetches(&first)[0].0, fetches(&second)[0].0);
        assert!(p.update(&Input::Bytes { tag: t2, status: 200, bytes: bmp(8, 8, [1, 1, 1]) }, &facts(HDMI2, false), true, 3200).1);
        assert!(!p.update(&Input::Bytes { tag: t1, status: 200, bytes: bmp(4, 4, [9, 9, 9]) }, &facts(HDMI2, false), true, 3300).1);
        assert_eq!(p.picture("hdmi2").map(|q| q.width), Some(8));
        // a 404 for a picture that was there forgets it in the index
        let third = tick_and_answer(&mut p, 3700, 4000);
        let (t3, _) = fetches(&third).remove(0);
        p.update(&Input::Bytes { tag: t3, status: 404, bytes: Vec::new() }, &facts(HDMI2, false), true, 4100);
        assert!(!p.index.contains_key("hdmi2"));
    }

    #[test]
    fn an_ask_the_helper_never_answers_is_let_go_and_the_live_tile_goes_on() {
        let mut p = up();
        let fx = p.opened(&facts(HDMI2, false), 1000);
        assert_eq!(posts(&fx).len(), 1);
        assert_eq!(p.due_at(), Some(1000 + INFLIGHT_MS + 1));
        assert!(posts(&tick(&mut p, 1000 + INFLIGHT_MS)).is_empty(), "not yet");
        let t = 1000 + INFLIGHT_MS + 1;
        assert!(posts(&tick(&mut p, t)).is_empty());
        assert!(p.inflight.is_none(), "let go");
        assert_eq!(p.due_at(), Some(t + LIVE_RETRY_MS));
        // a late answer to the ask that was let go is nothing
        assert!(answer(&mut p, 200, ok_at(1), t + 100).is_empty());
        let fx = tick(&mut p, t + LIVE_RETRY_MS);
        assert_eq!(posts(&fx), vec![("hdmi2".to_owned(), "live".to_owned())]);
    }

    #[test]
    fn closing_the_picker_ends_the_live_tile_and_the_quiet_timer_takes_over() {
        let mut p = up();
        p.opened(&facts(HDMI2, false), 1000);
        let fx = answer(&mut p, 200, ok_at(1000), 1100);
        deliver(&mut p, &fx, bmp(16, 9, [1, 2, 3]), 1150);
        assert!(p.shot("hdmi2", 1200).unwrap().live);
        p.closed(2000);
        let shot = p.shot("hdmi2", 2000).unwrap();
        assert!(!shot.live, "the picker is gone: whatever is shown elsewhere is a last picture");
        assert_eq!(p.due_at(), Some(2000 + KEEP_MS));
        assert!(posts(&tick(&mut p, 3000)).is_empty(), "no live asks with the picker away");
    }

    #[test]
    fn the_other_tiles_keep_their_last_picture_with_its_age_whatever_is_in_front() {
        let mut p = up();
        let doc = json!({"ok": true, "previews": {"enabled": true, "items": {"hdmi1": {"at": 1_000_000}}}});
        let fx = p.update(&Input::Fetched { tag: tags::INDEX, status: 200, data: doc }, &facts(HDMI2, false), true, 1_000_100).0;
        deliver(&mut p, &fx, bmp(16, 9, [1, 2, 3]), 1_000_200);
        p.opened(&facts(HDMI2, false), 1_001_000);
        let shot = p.shot("hdmi1", 1_000_000 + 7 * 60_000).unwrap();
        assert_eq!((shot.live, shot.age.as_str()), (false, "7 min ago"), "an input that is not in front is never live");
    }

    #[test]
    fn without_the_kind_nothing_is_asked_and_nothing_is_shown() {
        let mut p = Previews::new();
        assert!(p.update(&Input::Tick, &facts(HDMI2, false), true, 0).0.is_empty());
        assert!(p.opened(&facts(HDMI2, false), 100).is_empty());
        assert_eq!((p.due_at(), p.shot("hdmi2", 0)), (None, None));
    }

    #[test]
    fn pictures_that_arrive_while_the_picker_moves_wait_for_it_and_only_the_newest_of_an_input_is_decoded() {
        let mut p = up();
        p.opened(&facts(HDMI2, false), 1000);
        p.hold_until(5000);
        let first = answer(&mut p, 200, ok_at(1000), 1100);
        let second = tick_and_answer(&mut p, 1650, 2000);
        // the bytes come while the sheet slides: nothing is decoded, the tile is not told to change
        assert!(!deliver(&mut p, &first, bmp(8, 8, [1, 1, 1]), 1800));
        assert!(!deliver(&mut p, &second, bmp(16, 16, [9, 9, 9]), 1900));
        assert_eq!((p.picture("hdmi2").is_none(), p.shot("hdmi2", 1900)), (true, None));
        assert_eq!(p.report()["pending"], 1, "the older one was let go for the newer");
        assert!(p.due_at().is_some_and(|d| d <= 5000), "a tick is due by the end of the hold: {:?}", p.due_at());
        // before the hold is over a tick decodes nothing; at it the newest is decoded once and the tile is told
        p.update(&Input::Tick, &facts(HDMI2, false), true, 4999);
        assert!(p.picture("hdmi2").is_none() && p.report()["pending"] == 1, "nothing is decoded before the hold is over");
        let (fx, changed) = p.update(&Input::Tick, &facts(HDMI2, false), true, 5000);
        assert!(changed && fx.contains(&Effect::Redraw));
        assert_eq!(p.picture("hdmi2").map(|q| q.width), Some(16));
        assert_eq!(p.shot("hdmi2", 5000).map(|s| s.rev), Some(1), "one picture, one revision");
        assert_eq!(p.report()["pending"], 0);
    }

    #[test]
    fn a_hold_is_only_ever_extended_and_a_picture_that_comes_when_it_is_over_is_decoded_at_once() {
        let mut p = up();
        p.hold_until(3000);
        p.hold_until(2000);
        p.opened(&facts(HDMI2, false), 100);
        let fx = answer(&mut p, 200, ok_at(100), 150);
        assert!(!deliver(&mut p, &fx, bmp(8, 8, [1, 1, 1]), 2999), "still held at 2999");
        let mut q = up();
        q.hold_until(1000);
        q.opened(&facts(HDMI2, false), 100);
        let fx = answer(&mut q, 200, ok_at(100), 150);
        assert!(deliver(&mut q, &fx, bmp(8, 8, [1, 1, 1]), 1000), "the hold is over");
    }
}
