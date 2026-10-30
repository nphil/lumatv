//! The weather view's radar page, the model: what the helper's `/v1/wx/...` service says (the frames that exist, which of them are ready,
//! where the square they show lies on the map), which of its pictures the shell holds, which frame is shown and whether the loop plays.
//!
//! Nothing here draws or decodes on a frame of the screen: the helper fetches, colours and encodes (`helper/app/src/wxview.rs`), this asks
//! for what the page will show (the frames nearest the clock first, three at a time, the future ones too), decodes a PNG as it arrives
//! (a few milliseconds, once) and hands the picture to the surface, which makes a layer of it that stays on the graphics chip; a step of
//! the loop and a scrub are then a change of two layers' opacity, with no copy and no upload.
//!
//! The frames shown are the window the page wants: two hours of radar and six hours of the model's forecast ahead of it, marked as such.

use crate::kit::picture::{self, Picture};
use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// The tags of this page's requests: the open, the state, the map, and a block of frames.
pub const TAG_OPEN: u64 = 0x5700_0001;
pub const TAG_STATE: u64 = 0x5700_0002;
pub const TAG_MAP: u64 = 0x5700_0003;
pub const TAG_GLOBE: u64 = 0x5700_0004;
pub const TAG_FRAME_FIRST: u64 = 0x5700_0100;
pub const TAG_FRAME_LAST: u64 = 0x5700_01ff;

/// Frames asked for at once.
const IN_FLIGHT: usize = 3;
/// The state is read this often while frames are coming, and this often once they are all there.
const POLL_LOADING_MS: u64 = 700;
const POLL_READY_MS: u64 = 20_000;
/// A step of the loop, and how long the loop rests on its last frame.
pub const STEP_MS: u64 = 520;
const END_HOLD_MS: u64 = 1_400;
/// What of the helper's frames the page shows: this far back and ahead of the clock (seconds).
pub const BACK_S: i64 = 2 * 3_600;
pub const AHEAD_S: i64 = 6 * 3_600;
/// The map the page asks for: the whole screen, in pixels (its zoom is for the two levels below).
pub const MAP_W: u32 = 1920;
pub const MAP_H: u32 = 1080;
/// The globe's picture is a square this many pixels on a side (the helper's most is 1024), in the middle of the screen.
pub const GLOBE_PX: u32 = 760;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Observed,
    Forecast,
    Outlook,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub t: i64,
    pub kind: Kind,
    pub ready: bool,
    pub rev: u32,
}

/// The two views: the region about home (475 km) and the wide one (1,900 km).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Home,
    Wide,
    /// The whole earth: not a radar view but the helper's picture of the globe (NASA's rain and cloud).
    Globe,
}

impl Level {
    /// The radar level the helper is asked for, and the zoom of the base map under it.
    pub fn radar_zoom(self) -> u8 {
        match self {
            Level::Home | Level::Globe => 6,
            Level::Wide => 4,
        }
    }

    /// Three zooms more than the radar's, so that the square a radar frame covers (a 2,048 px square on the map) fills a screen of 1920 by 1080.
    pub fn map_zoom(self) -> u8 {
        match self {
            Level::Home | Level::Globe => 9,
            Level::Wide => 7,
        }
    }
}

#[derive(Default)]
pub struct Radar {
    pub open: bool,
    pub level: Option<Level>,
    pub lat: f64,
    pub lon: f64,
    pub dark: bool,
    pub status: String,
    pub why: String,
    pub have: usize,
    pub want: usize,
    pub frames: Vec<Frame>,
    /// The square the frames cover, in world units (0 to 1): left, top, right, bottom.
    pub extent: [f64; 4],
    pub frame_px: u32,
    pub attribution: String,
    /// The helper's clock (unix seconds), as of its last answer.
    pub now_s: i64,
    pub newest_s: Option<i64>,
    /// Whether the helper answered at all.
    pub reached: bool,
    pub playing: bool,
    /// The frame shown, by time.
    pub cur: Option<i64>,
    /// Pictures that came and are not yet made into layers, and the base map's.
    pub arrived: Vec<(i64, u32, Picture)>,
    pub map: Option<Picture>,
    /// What the shell holds: (time, revision).
    loaded: HashSet<(i64, u32)>,
    inflight: HashMap<u64, (i64, u32)>,
    retry_at: HashMap<i64, u64>,
    map_asked: Option<(Level, bool, u64)>,
    map_complete: bool,
    map_have: bool,
    /// The globe: where it is seen from (longitude, latitude), its picture, and what was asked.
    pub globe_at: (f64, f64),
    /// How much of the earth is shown: 1, 1.7 or 2.8 times its size.
    pub globe_zoom: f64,
    pub globe: Option<Picture>,
    globe_asked: Option<((i64, i64), bool, u64)>,
    globe_have: Option<((i64, i64), bool)>,
    globe_asked_at: u64,
    next_poll: u64,
    step_at: u64,
    next_tag: u64,
}

fn kind_of(n: u64) -> Kind {
    match n {
        0 => Kind::Observed,
        1 => Kind::Forecast,
        _ => Kind::Outlook,
    }
}

impl Radar {
    pub fn level(&self) -> Level {
        self.level.unwrap_or(Level::Home)
    }

    /// The page is opened for a place: the helper starts its engines.
    pub fn open(&mut self, lat: f64, lon: f64, level: Level, dark: bool, now: u64) -> Vec<Effect> {
        *self = Radar { open: true, level: Some(level), globe_at: (lon, lat.clamp(-70.0, 70.0)), globe_zoom: 1.0, lat, lon, dark, playing: true, next_tag: TAG_FRAME_FIRST, next_poll: now + POLL_LOADING_MS, status: "loading".into(), ..Radar::default() };
        vec![self.open_request()]
    }

    fn open_request(&self) -> Effect {
        Effect::Request { tag: TAG_OPEN, method: "POST".into(), path: "/v1/wx/open".into(), body: Some(json!({"lat": self.lat, "lon": self.lon, "zoom": self.level().radar_zoom()})) }
    }

    /// The page is left or Home is covered: the helper lets its engines go.
    pub fn close(&mut self) -> Vec<Effect> {
        if !self.open {
            return Vec::new();
        }
        *self = Radar::default();
        vec![Effect::Post { path: "/v1/wx/close".into(), body: json!({}) }]
    }

    /// The other view: the same place at the other level (the helper's engines are asked again, the pictures are made again).
    pub fn set_level(&mut self, level: Level, now: u64) -> Vec<Effect> {
        if !self.open || self.level() == level {
            return Vec::new();
        }
        let (lat, lon, dark) = (self.lat, self.lon, self.dark);
        if level == Level::Globe {
            // the globe is the helper's picture: nothing is asked to start, and the radar's engines are left as they are
            self.level = Some(level);
            self.globe = None;
            self.globe_have = None;
            self.globe_asked = None;
            self.globe_at = (lon, lat.clamp(-70.0, 70.0));
            self.globe_zoom = 1.0;
            return self.ask(now);
        }
        let playing = self.playing;
        let out = self.open(lat, lon, level, dark, now);
        self.playing = playing;
        out
    }

    /// The globe turned: `dlon` and `dlat` degrees.
    pub fn turn(&mut self, dlon: f64, dlat: f64, now: u64) -> Vec<Effect> {
        let (lon, lat) = self.globe_at;
        self.globe_at = ((lon + dlon + 540.0).rem_euclid(360.0) - 180.0, (lat + dlat).clamp(-70.0, 70.0));
        self.ask(now)
    }

    /// Frames inside the window, oldest first: what the page shows.
    pub fn window(&self) -> Vec<Frame> {
        let now = self.now_s;
        self.frames.iter().copied().filter(|f| f.kind != Kind::Outlook && f.t >= now - BACK_S && f.t <= now + AHEAD_S).collect()
    }

    /// The frames in the window that the shell holds, oldest first: what a step and a scrub go over.
    pub fn held(&self) -> Vec<Frame> {
        self.window().into_iter().filter(|f| self.loaded.contains(&(f.t, f.rev))).collect()
    }

    pub fn holds(&self, f: &Frame) -> bool {
        self.loaded.contains(&(f.t, f.rev))
    }

    /// An answer or a picture of the helper's.
    pub fn input(&mut self, input: &Input, now: u64) -> (Vec<Effect>, bool) {
        let mut out = Vec::new();
        let mut changed = false;
        match input {
            Input::Fetched { tag, status, data } if *tag == TAG_OPEN || *tag == TAG_STATE => {
                self.reached = *status == 200 || self.reached;
                if *status == 409 && *tag == TAG_STATE {
                    // the helper let the engines go (it restarted, or nobody asked for a while): open again
                    out.push(self.open_request());
                } else if *status == 200 {
                    changed |= self.take_state(data, now);
                }
            }
            Input::Fetched { tag, status, .. } if (TAG_FRAME_FIRST..=TAG_FRAME_LAST).contains(tag) => {
                // 202: the frame is still coming; asked again soon
                if let Some((t, _)) = self.inflight.remove(tag) {
                    self.retry_at.insert(t, now + if *status == 202 { 500 } else { 3_000 });
                }
            }
            Input::Fetched { tag, .. } if *tag == TAG_MAP => {
                self.map_asked = None;
            }
            Input::Bytes { tag, status, bytes } if (TAG_FRAME_FIRST..=TAG_FRAME_LAST).contains(tag) => {
                if let Some((t, rev)) = self.inflight.remove(tag) {
                    match (*status == 200).then(|| picture::decode(bytes).ok()).flatten() {
                        Some(p) => {
                            self.loaded.insert((t, rev));
                            self.arrived.push((t, rev, p));
                            if self.cur.is_none() {
                                self.cur = Some(t);
                            }
                            changed = true;
                        }
                        None => {
                            self.retry_at.insert(t, now + 3_000);
                        }
                    }
                }
            }
            Input::Fetched { tag, .. } if *tag == TAG_GLOBE => {
                self.globe_asked = None;
                self.globe_asked_at = now + 2_000;
            }
            Input::Bytes { tag, status, bytes } if *tag == TAG_GLOBE => {
                if let Some((key, dark, _)) = self.globe_asked.take() {
                    if dark == self.dark && (*status == 200 || *status == 206) {
                        if let Ok(p) = picture::decode(bytes) {
                            self.globe = Some(p);
                            self.globe_have = Some((key, *status == 200));
                            self.globe_asked_at = now + 1_500;
                            changed = true;
                        }
                    }
                }
            }
            Input::Bytes { tag, status, bytes } if *tag == TAG_MAP => {
                if let Some((level, dark, _)) = self.map_asked.take() {
                    if level == self.level() && dark == self.dark && (*status == 200 || *status == 206) {
                        if let Ok(p) = picture::decode(bytes) {
                            self.map = Some(p);
                            self.map_have = true;
                            self.map_complete = *status == 200;
                            changed = true;
                        }
                    }
                }
            }
            _ => {}
        }
        out.extend(self.ask(now));
        (out, changed)
    }

    fn take_state(&mut self, v: &Value, now: u64) -> bool {
        let status = v.get("status").and_then(Value::as_str).unwrap_or("loading").to_owned();
        let frames: Vec<Frame> = v
            .get("frames")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|f| {
                        Some(Frame { t: f.get(0)?.as_i64()?, kind: kind_of(f.get(1)?.as_u64()?), ready: f.get(2)?.as_bool()?, rev: f.get(3)?.as_u64().unwrap_or(0) as u32 })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let ext = |k: &str| v.pointer(&format!("/extent/{k}")).and_then(Value::as_f64).unwrap_or(0.0);
        let before = (self.status.clone(), self.have, self.want, self.frames.len(), self.frames.iter().filter(|f| f.ready).count());
        self.status = status;
        self.why = v.get("why").and_then(Value::as_str).unwrap_or("").to_owned();
        self.have = v.get("have").and_then(Value::as_u64).unwrap_or(0) as usize;
        self.want = v.get("want").and_then(Value::as_u64).unwrap_or(0) as usize;
        self.frames = frames;
        self.extent = [ext("x0"), ext("y0"), ext("x1"), ext("y1")];
        self.frame_px = v.get("framePx").and_then(Value::as_u64).unwrap_or(0) as u32;
        self.attribution = v.get("attribution").and_then(Value::as_str).unwrap_or("").to_owned();
        self.now_s = v.get("now").and_then(Value::as_i64).unwrap_or(self.now_s);
        self.newest_s = v.get("newest").and_then(Value::as_i64);
        // a frame of an older revision is not the one held: it is asked for again, and the held set forgets what the helper replaced
        let current: HashSet<(i64, u32)> = self.frames.iter().filter(|f| f.ready).map(|f| (f.t, f.rev)).collect();
        self.loaded.retain(|k| current.contains(k));
        if self.cur.is_none_or(|c| !self.frames.iter().any(|f| f.t == c)) {
            self.cur = self.held().last().map(|f| f.t);
        }
        let complete = self.want > 0 && self.have >= self.want;
        self.next_poll = now + if complete { POLL_READY_MS } else { POLL_LOADING_MS };
        before != (self.status.clone(), self.have, self.want, self.frames.len(), self.frames.iter().filter(|f| f.ready).count())
    }

    /// The requests that are due: the state, the map, and frames (nearest the clock first, three at a time).
    fn ask(&mut self, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if !self.open || !self.reached && self.frames.is_empty() && now < self.next_poll {
            return out;
        }
        if now >= self.next_poll {
            self.next_poll = now + POLL_LOADING_MS;
            out.push(Effect::Fetch { tag: TAG_STATE, path: "/v1/wx/state".into() });
        }
        let level = self.level();
        let stale_map = self.map_asked.is_none() && (!self.map_have || !self.map_complete);
        if self.reached && stale_map && now >= self.map_asked.map_or(0, |m| m.2) {
            self.map_asked = Some((level, self.dark, now));
            out.push(Effect::Fetch { tag: TAG_MAP, path: format!("/v1/wx/map?w={MAP_W}&h={MAP_H}&zoom={}&dark={}", level.map_zoom(), u8::from(self.dark)) });
        }
        if level == Level::Globe && self.reached {
            let key = ((self.globe_at.0 * 10.0).round() as i64, (self.globe_at.1 * 10.0).round() as i64 + (self.globe_zoom * 1000.0) as i64 * 100_000);
            let wanted = self.globe_asked.is_none() && (self.globe_have.is_none_or(|(k, complete)| k != key || (!complete && now >= self.globe_retry_at())));
            if wanted {
                self.globe_asked = Some((key, self.dark, now));
                out.push(Effect::Fetch { tag: TAG_GLOBE, path: format!("/v1/wx/globe?lon={:.1}&lat={:.1}&size={GLOBE_PX}&zoom={:.1}&dark={}", self.globe_at.0, self.globe_at.1, self.globe_zoom, u8::from(self.dark)) });
            }
            return out;
        }
        // frames: the ready ones the shell does not hold, the nearest the clock first
        let mut want: Vec<Frame> = self.window().into_iter().filter(|f| f.ready && !self.loaded.contains(&(f.t, f.rev)) && !self.inflight.values().any(|v| *v == (f.t, f.rev)) && self.retry_at.get(&f.t).is_none_or(|at| now >= *at)).collect();
        let clock = self.now_s;
        want.sort_by_key(|f| (f.t - clock).abs());
        for f in want {
            if self.inflight.len() >= IN_FLIGHT {
                break;
            }
            let tag = self.next_tag;
            self.next_tag = if self.next_tag >= TAG_FRAME_LAST { TAG_FRAME_FIRST } else { self.next_tag + 1 };
            self.inflight.insert(tag, (f.t, f.rev));
            out.push(Effect::Fetch { tag, path: format!("/v1/wx/frame?t={}&dark={}", f.t, u8::from(self.dark)) });
        }
        out
    }

    /// Time passed: the polls and the loop's step.
    pub fn tick(&mut self, now: u64, loops: bool) -> (Vec<Effect>, bool) {
        let mut changed = false;
        let out = if self.open { self.ask(now) } else { Vec::new() };
        if self.open && self.playing && loops && now >= self.step_at {
            changed = self.step(1, now);
        }
        (out, changed)
    }

    /// When the next poll or step is due.
    pub fn due_at(&self, loops: bool) -> Option<u64> {
        if !self.open {
            return None;
        }
        let step = (self.playing && loops && self.held().len() > 1).then_some(self.step_at);
        [Some(self.next_poll), step].into_iter().flatten().min()
    }

    /// One frame on (`d` = 1) or back (-1) over the frames held; the loop rests on its last frame before it starts again.
    pub fn step(&mut self, d: i32, now: u64) -> bool {
        let held = self.held();
        if held.len() < 2 {
            return false;
        }
        let i = self.cur.and_then(|c| held.iter().position(|f| f.t == c)).unwrap_or(held.len() - 1) as i32;
        let n = held.len() as i32;
        let j = (i + d).rem_euclid(n);
        self.cur = Some(held[j as usize].t);
        self.step_at = now + if j == n - 1 && d > 0 { STEP_MS + END_HOLD_MS } else { STEP_MS };
        true
    }

    /// A key of the page: Left and Right scrub (and hold the loop where it is), OK plays and holds, 1 and 2 are the two views.
    pub fn key(&mut self, name: &str, now: u64) -> (Vec<Effect>, bool) {
        if self.level() == Level::Globe && matches!(name, "left" | "right" | "up" | "down" | "ok") {
            return self.globe_key(name, now);
        }
        match name {
            "left" => {
                self.playing = false;
                (Vec::new(), self.step(-1, now))
            }
            "right" => {
                self.playing = false;
                (Vec::new(), self.step(1, now))
            }
            "ok" => {
                self.playing = !self.playing;
                self.step_at = now + STEP_MS;
                (Vec::new(), true)
            }
            "1" => (self.set_level(Level::Home, now), true),
            "2" => (self.set_level(Level::Wide, now), true),
            "3" => (self.set_level(Level::Globe, now), true),
            _ => (Vec::new(), false),
        }
    }

    fn globe_key(&mut self, name: &str, now: u64) -> (Vec<Effect>, bool) {
        match name {
            "left" => (self.turn(-20.0, 0.0, now), true),
            "right" => (self.turn(20.0, 0.0, now), true),
            "up" => (self.turn(0.0, 12.0, now), true),
            "down" => (self.turn(0.0, -12.0, now), true),
            _ => {
                // OK steps the zoom 1, 1.7, 2.8 and then back to the whole earth turned to home
                if self.globe_zoom < 2.0 {
                    self.globe_zoom = if self.globe_zoom < 1.2 { 1.7 } else { 2.8 };
                } else {
                    self.globe_zoom = 1.0;
                    self.globe_at = (self.lon, self.lat.clamp(-70.0, 70.0));
                }
                (self.ask(now), true)
            }
        }
    }

    pub fn globe_ready(&self) -> bool {
        self.globe_have.is_some()
    }

    pub fn take_globe(&mut self) -> Option<Picture> {
        self.globe.take()
    }

    /// The frame shown.
    pub fn current(&self) -> Option<Frame> {
        let c = self.cur?;
        self.frames.iter().copied().find(|f| f.t == c)
    }

    fn globe_retry_at(&self) -> u64 {
        self.globe_asked_at
    }

    /// The pictures the surface has to make layers of.
    pub fn take_arrived(&mut self) -> Vec<(i64, u32, Picture)> {
        std::mem::take(&mut self.arrived)
    }

    pub fn take_map(&mut self) -> Option<Picture> {
        self.map.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(frames: &[(i64, u64, bool, u64)], have: usize, want: usize) -> Value {
        json!({"status": "loading", "have": have, "want": want, "frames": frames.iter().map(|f| json!([f.0, f.1, f.2, f.3])).collect::<Vec<_>>(),
               "framePx": 384, "attribution": "x", "now": 10_000, "extent": {"x0": 0.2, "y0": 0.3, "x1": 0.21, "y1": 0.31}, "newest": 9_600})
    }

    fn png() -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, 2, 2);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&[255u8; 16]).unwrap();
        out
    }

    fn fetched(r: &mut Radar, v: Value, now: u64) -> Vec<Effect> {
        r.input(&Input::Fetched { tag: TAG_STATE, status: 200, data: v }, now).0
    }

    #[test]
    fn opening_asks_the_helper_to_start_and_the_state_names_the_frames_to_ask_for_nearest_the_clock_first() {
        let mut r = Radar::default();
        let out = r.open(33.7, -84.4, Level::Home, true, 1_000);
        assert!(matches!(&out[0], Effect::Request { tag: TAG_OPEN, method, path, .. } if method == "POST" && path == "/v1/wx/open"));
        // a day of observed frames and the model's run: only the window is the page's, the ones that are ready are asked for
        let mut frames: Vec<(i64, u64, bool, u64)> = (0..30).map(|i| (10_000 - 86_400 + i * 2_880, 0, true, 0)).collect();
        frames.extend((0..12).map(|i| (10_000 - 6_000 + i * 600, 0, true, 0)));
        frames.extend((1..=30).map(|i| (10_000 + i * 900, 1, true, 0)));
        frames.extend((1..=4).map(|i| (10_000 + 4 * 86_400 + i * 3_600, 2, true, 0)));
        let out = fetched(&mut r, state(&frames, 5, 40), 1_100);
        let asked: Vec<&Effect> = out.iter().filter(|e| matches!(e, Effect::Fetch { path, .. } if path.starts_with("/v1/wx/frame"))).collect();
        assert_eq!(asked.len(), IN_FLIGHT, "three at a time");
        let first = match asked[0] {
            Effect::Fetch { path, .. } => path.clone(),
            _ => unreachable!(),
        };
        assert!(first.contains("t=10000") || first.contains("t=10300") || first.contains("t=9700") || first.contains("t=10400") || first.contains("t=10200"), "the nearest the clock first: {first}");
        assert!(out.iter().any(|e| matches!(e, Effect::Fetch { path, .. } if path.starts_with("/v1/wx/map?w=1920&h=1080&zoom=9&dark=1"))), "and the base map, once");
        assert!(r.window().iter().all(|f| f.t >= 10_000 - BACK_S && f.t <= 10_000 + AHEAD_S && f.kind != Kind::Outlook), "the window: two hours back, six ahead, no outlook");
    }

    #[test]
    fn a_picture_that_comes_is_decoded_once_held_and_the_next_is_asked_for() {
        let mut r = Radar::default();
        r.open(33.7, -84.4, Level::Home, true, 1_000);
        let frames: Vec<(i64, u64, bool, u64)> = (0..8).map(|i| (10_000 - 1_200 + i * 600, 0, true, 0)).collect();
        let out = fetched(&mut r, state(&frames, 0, 8), 1_100);
        let tags: Vec<u64> = out.iter().filter_map(|e| match e { Effect::Fetch { tag, path } if path.starts_with("/v1/wx/frame") => Some(*tag), _ => None }).collect();
        assert_eq!(tags.len(), 3);
        let (out, changed) = r.input(&Input::Bytes { tag: tags[0], status: 200, bytes: png() }, 1_200);
        assert!(changed);
        assert_eq!(r.held().len(), 1);
        assert_eq!(r.take_arrived().len(), 1);
        assert!(r.take_arrived().is_empty(), "handed over once");
        assert_eq!(out.iter().filter(|e| matches!(e, Effect::Fetch { path, .. } if path.starts_with("/v1/wx/frame"))).count(), 1, "one more takes its place");
        // a frame that is not a picture is asked for again later, not at once
        let (_, changed) = r.input(&Input::Bytes { tag: tags[1], status: 200, bytes: b"nope".to_vec() }, 1_300);
        assert!(!changed);
        assert_eq!(r.held().len(), 1);
    }

    #[test]
    fn the_loop_steps_over_what_is_held_rests_on_its_last_frame_and_a_scrub_holds_it() {
        let mut r = Radar::default();
        r.open(33.7, -84.4, Level::Home, true, 0);
        let frames: Vec<(i64, u64, bool, u64)> = (0..4).map(|i| (10_000 - 1_800 + i * 600, 0, true, 0)).collect();
        let out = fetched(&mut r, state(&frames, 0, 4), 100);
        let mut tags: Vec<u64> = out.iter().filter_map(|e| match e { Effect::Fetch { tag, path } if path.starts_with("/v1/wx/frame") => Some(*tag), _ => None }).collect();
        let mut at = 200;
        while let Some(tag) = tags.pop() {
            let (more, _) = r.input(&Input::Bytes { tag, status: 200, bytes: png() }, at);
            tags.extend(more.iter().filter_map(|e| match e { Effect::Fetch { tag, path } if path.starts_with("/v1/wx/frame") => Some(*tag), _ => None }));
            at += 10;
        }
        assert_eq!(r.held().len(), 4);
        r.cur = Some(8_200);
        let order: Vec<i64> = (0..5).map(|i| { r.step(1, 1_000 + i * 10); r.cur.unwrap() }).collect();
        assert_eq!(order, [8_800, 9_400, 10_000, 8_200, 8_800], "over what is held, round to the oldest again");
        // the loop rests on its newest frame for longer than a step
        r.cur = Some(9_400);
        r.step(1, 5_000);
        assert_eq!(r.cur, Some(10_000));
        assert!(r.step_at >= 5_000 + STEP_MS + END_HOLD_MS, "rests on the newest");
        r.cur = Some(8_800);
        r.step(1, 7_000);
        assert_eq!(r.step_at, 7_000 + STEP_MS);
        r.playing = true;
        let (_, _) = r.key("left", 6_000);
        assert!(!r.playing, "a scrub holds the loop");
        let (_, _) = r.key("ok", 6_100);
        assert!(r.playing, "OK plays again");
    }

    #[test]
    fn a_helper_that_let_the_engines_go_is_asked_to_open_again_and_the_levels_are_two_views() {
        let mut r = Radar::default();
        r.open(33.7, -84.4, Level::Home, false, 0);
        let (out, _) = r.input(&Input::Fetched { tag: TAG_STATE, status: 409, data: Value::Null }, 5_000);
        assert!(out.iter().any(|e| matches!(e, Effect::Request { tag: TAG_OPEN, .. })));
        let out = r.set_level(Level::Wide, 6_000);
        assert!(matches!(&out[0], Effect::Request { body: Some(b), .. } if b["zoom"] == 4), "{out:?}");
        assert_eq!(Level::Wide.map_zoom(), 7);
        assert_eq!(r.close(), vec![Effect::Post { path: "/v1/wx/close".into(), body: json!({}) }]);
        assert!(!r.open);
        assert!(r.close().is_empty());
    }
}
