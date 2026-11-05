//! The weather view's radar page, the model: what the helper's `/v1/wx/...` service says (the frames that exist), which pictures of it the page holds
//! for the view it is looking at ([`super::radar_pics`]), which frame is shown and whether the loop plays.
//!
//! Nothing here draws or decodes on a frame of the screen: the helper fetches, colours and composes (`helper/app/src/wxview.rs`), this asks for
//! what the view needs (the map for it, a coarser one under it, the frame on show and the one the loop shows next) and hands the pictures to the
//! surface, which makes a layer of each that stays on the graphics chip; a step of the loop and a scrub are then a change of two layers' opacity,
//! with no copy and no upload. The view is the surface's to say ([`Radar::set_view`]: the place at the middle of the screen, which the zoom's
//! pan moves, and the zoom the compositor is easing to).
//!
//! The frames shown are the window the page wants: two hours of radar and six hours of the model's forecast ahead of it, marked as such. The
//! loop goes over the window; it steps when the next frame's picture is in, which was asked for as the loop went onto the frame before it.

use super::radar_pics::{MapPic, Pics, Role, TAG_FIRST, TAG_LAST, View};
use super::radar_zoom::{Z_WIDE, Zoom};
use crate::kit::picture::{self, Picture};
use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};

/// The tags of this page's requests: the open, the state and the globe (the pictures' are [`super::radar_pics`]).
pub const TAG_OPEN: u64 = 0x5700_0001;
pub const TAG_STATE: u64 = 0x5700_0002;
pub const TAG_GLOBE: u64 = 0x5700_0004;

/// The state is read this often while the radar is loading, and this often once it is ready.
const POLL_LOADING_MS: u64 = 700;
const POLL_READY_MS: u64 = 20_000;
/// A step of the loop, and how long the loop rests on its last frame.
pub const STEP_MS: u64 = 520;
const END_HOLD_MS: u64 = 1_400;
/// What of the helper's frames the page shows: this far back and ahead of the clock (seconds).
pub const BACK_S: i64 = 2 * 3_600;
pub const AHEAD_S: i64 = 6 * 3_600;
/// The screen the page draws, in pixels.
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

pub struct Radar {
    pub open: bool,
    pub lat: f64,
    pub lon: f64,
    pub dark: bool,
    pub status: String,
    pub why: String,
    pub have: usize,
    pub want: usize,
    pub frames: Vec<Frame>,
    pub attribution: String,
    /// The helper's clock (unix seconds), as of its last answer.
    pub now_s: i64,
    pub newest_s: Option<i64>,
    /// Whether the helper answered at all.
    pub reached: bool,
    pub playing: bool,
    /// The frame shown, by time.
    pub cur: Option<i64>,
    /// The pictures of the view that are held, asked for and on their way ([`super::radar_pics`]).
    pics: Pics,
    /// What the surface says the screen will show, and whether the pictures have been asked for it yet.
    view: Option<View>,
    view_new: bool,
    /// A new open is on its way (the helper had let the view go): another is not asked for.
    reopening: bool,
    /// Where the view is going ([`super::radar_zoom`]); the surface eases the picture there.
    pub zoom: Zoom,
    /// The globe shows instead of the map: its picture, where it is seen from (longitude, latitude), how much of the earth is shown (1, 1.7 or
    /// 2.8 times its size) and what was asked.
    globe_on: bool,
    pub globe_at: (f64, f64),
    pub globe_zoom: f64,
    pub globe: Option<Picture>,
    globe_asked: Option<((i64, i64), bool, u64)>,
    globe_have: Option<((i64, i64), bool)>,
    globe_asked_at: u64,
    next_poll: u64,
    step_at: u64,
}

impl Default for Radar {
    fn default() -> Self {
        Self {
            open: false,
            lat: 0.0,
            lon: 0.0,
            dark: false,
            status: String::new(),
            why: String::new(),
            have: 0,
            want: 0,
            frames: Vec::new(),
            attribution: String::new(),
            now_s: 0,
            newest_s: None,
            reached: false,
            playing: false,
            cur: None,
            pics: Pics::new(),
            view: None,
            view_new: false,
            reopening: false,
            zoom: Zoom::default(),
            globe_on: false,
            globe_at: (0.0, 0.0),
            globe_zoom: 1.0,
            globe: None,
            globe_asked: None,
            globe_have: None,
            globe_asked_at: 0,
            next_poll: 0,
            step_at: 0,
        }
    }
}

fn kind_of(n: u64) -> Kind {
    match n {
        0 => Kind::Observed,
        1 => Kind::Forecast,
        _ => Kind::Outlook,
    }
}

impl Radar {
    /// Whether the earth is what the view shows: zoomed out past where the map's own radar views end.
    pub fn globe(&self) -> bool {
        self.zoom.target < Z_WIDE
    }

    /// The page is opened for a place: the helper starts its engines.
    pub fn open(&mut self, lat: f64, lon: f64, dark: bool, now: u64) -> Vec<Effect> {
        let zoom = self.zoom;
        *self = Radar { open: true, globe_at: (lon, lat.clamp(-70.0, 70.0)), globe_on: zoom.target < Z_WIDE, lat, lon, dark, playing: true, zoom, next_poll: now + POLL_LOADING_MS, status: "loading".into(), ..Radar::default() };
        vec![self.open_request()]
    }

    fn open_request(&self) -> Effect {
        Effect::Request { tag: TAG_OPEN, method: "POST".into(), path: "/v1/wx/open".into(), body: Some(json!({"lat": self.lat, "lon": self.lon})) }
    }

    /// The page is left or Home is covered: the helper lets its engines go, and every picture, ask and buffer of the page's goes with this (the
    /// surface's layers go with its page).
    pub fn close(&mut self) -> Vec<Effect> {
        if !self.open {
            return Vec::new();
        }
        *self = Radar::default();
        vec![Effect::Post { path: "/v1/wx/close".into(), body: json!({}) }]
    }

    /// Zooms `d` levels (in is positive) about `anchor`, a place on the screen (the pointer's) or none for the middle. Past the radar's own views
    /// the earth is shown; the pictures of the map are asked for the view once the surface has said where the zoom puts it.
    pub fn zoom_by(&mut self, d: f32, anchor: Option<(f32, f32)>, now: u64) -> Vec<Effect> {
        if !self.open || !self.zoom.by(d, anchor) {
            return Vec::new();
        }
        let globe = self.globe();
        if globe != self.globe_on {
            self.globe_on = globe;
            if globe {
                // the globe is the helper's picture: it is asked for from where the place is
                self.globe = None;
                self.globe_have = None;
                self.globe_asked = None;
                self.globe_at = (self.lon, self.lat.clamp(-70.0, 70.0));
                self.globe_zoom = 1.0;
            }
        }
        self.ask(now)
    }

    /// Where the screen's middle is on the map and the zoom it is eased to: the surface says it whenever it takes up a zoom, so that the pictures
    /// that are asked for are those of the view that will be there and not those of the view that was. Nothing is asked here: the next tick does.
    pub fn set_view(&mut self, view: View) {
        if self.view != Some(view) {
            self.view = Some(view);
            self.view_new = true;
        }
    }

    /// Frames inside the window, oldest first: what the page shows.
    pub fn window(&self) -> Vec<Frame> {
        let now = self.now_s;
        self.frames.iter().copied().filter(|f| f.kind != Kind::Outlook && f.t >= now - BACK_S && f.t <= now + AHEAD_S).collect()
    }

    /// Whether the picture of the frame is in the page's hands.
    pub fn has_picture(&self, f: &Frame) -> bool {
        self.pics.holds(Role::Radar { t: f.t, rev: f.rev })
    }

    /// Whether the map picture held covers what the view shows: the coarser one under it is then not seen.
    pub fn map_serves(&self) -> bool {
        self.pics.serves(Role::Map)
    }

    /// Whether any frame of the radar is in the page's hands.
    pub fn has_any_picture(&self) -> bool {
        self.pics.held().iter().any(|(_, role)| matches!(role, Role::Radar { .. }))
    }

    /// An answer or a picture of the helper's.
    pub fn input(&mut self, input: &Input, now: u64) -> (Vec<Effect>, bool) {
        let mut out = Vec::new();
        let mut changed = false;
        match input {
            Input::Fetched { tag, status, data } if *tag == TAG_OPEN || *tag == TAG_STATE => {
                self.reached = *status == 200 || self.reached;
                if *tag == TAG_OPEN {
                    self.reopening = false;
                }
                if *status == 409 && *tag == TAG_STATE {
                    // the helper let the engines go (it restarted, or nobody asked for a while): open again
                    out.push(self.open_request());
                    self.reopening = true;
                } else if *status == 200 {
                    changed |= self.take_state(data, now);
                }
            }
            Input::Fetched { tag, .. } | Input::Bytes { tag, .. } if (TAG_FIRST..=TAG_LAST).contains(tag) => {
                let (fx, c) = self.pics.input(input, now);
                out.extend(fx);
                changed |= c;
                if std::mem::take(&mut self.pics.lost) && !self.reopening {
                    // the helper no longer has the view
                    self.reopening = true;
                    out.push(self.open_request());
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
        let before = (self.status.clone(), self.have, self.want, self.frames.len(), self.frames.iter().filter(|f| f.ready).count());
        self.status = status;
        self.why = v.get("why").and_then(Value::as_str).unwrap_or("").to_owned();
        self.have = v.get("have").and_then(Value::as_u64).unwrap_or(0) as usize;
        self.want = v.get("want").and_then(Value::as_u64).unwrap_or(0) as usize;
        self.frames = frames;
        self.attribution = v.get("attribution").and_then(Value::as_str).unwrap_or("").to_owned();
        self.now_s = v.get("now").and_then(Value::as_i64).unwrap_or(self.now_s);
        self.newest_s = v.get("newest").and_then(Value::as_i64);
        // the frame shown is the one nearest the clock until the loop or a scrub moves it; one the window has left is replaced the same way
        if self.cur.is_none_or(|c| !self.window().iter().any(|f| f.t == c)) {
            let clock = self.now_s;
            self.cur = self.window().iter().min_by_key(|f| ((f.t - clock).abs(), f.t)).map(|f| f.t);
        }
        let complete = self.want > 0 && self.have >= self.want;
        self.next_poll = now + if complete { POLL_READY_MS } else { POLL_LOADING_MS };
        before != (self.status.clone(), self.have, self.want, self.frames.len(), self.frames.iter().filter(|f| f.ready).count())
    }

    /// What the page wants of the helper now, the most wanted first: the map's pictures for the view, the frame on show and the one after it (the
    /// loop's next step, which is asked for ahead so that a step is a change of opacity and not a wait).
    fn roles(&self) -> Vec<Role> {
        let mut roles = vec![Role::Backdrop, Role::Map];
        let win = self.window();
        if let Some(i) = self.cur.and_then(|c| win.iter().position(|f| f.t == c)) {
            let at = |f: &Frame| Role::Radar { t: f.t, rev: f.rev };
            roles.push(at(&win[i]));
            if win.len() > 1 {
                roles.push(at(&win[(i + 1) % win.len()]));
            }
        }
        roles
    }

    /// The frame after the one shown, in the loop's order.
    fn next_frame(&self) -> Option<Frame> {
        let win = self.window();
        let i = self.cur.and_then(|c| win.iter().position(|f| f.t == c))?;
        (win.len() > 1).then(|| win[(i + 1) % win.len()])
    }

    /// The pictures are asked for the view as it is, and let go when they are not wanted.
    fn sync_pics(&mut self, now: u64) -> Vec<Effect> {
        self.view_new = false;
        let view = if self.globe_on || !self.reached { None } else { self.view };
        let roles = if view.is_some() { self.roles() } else { Vec::new() };
        self.pics.update(view, &roles, self.dark, now)
    }

    /// The requests that are due: the state, the globe or the pictures of the map.
    fn ask(&mut self, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if !self.open || !self.reached && self.frames.is_empty() && now < self.next_poll {
            return out;
        }
        if now >= self.next_poll {
            self.next_poll = now + POLL_LOADING_MS;
            out.push(Effect::Fetch { tag: TAG_STATE, path: "/v1/wx/state".into() });
        }
        out.extend(self.sync_pics(now));
        if self.globe_on && self.reached {
            let key = ((self.globe_at.0 * 10.0).round() as i64, (self.globe_at.1 * 10.0).round() as i64 + (self.globe_zoom * 1000.0) as i64 * 100_000);
            let wanted = self.globe_asked.is_none() && (self.globe_have.is_none_or(|(k, complete)| k != key || (!complete && now >= self.globe_retry_at())));
            if wanted {
                self.globe_asked = Some((key, self.dark, now));
                out.push(Effect::Fetch { tag: TAG_GLOBE, path: format!("/v1/wx/globe?lon={:.1}&lat={:.1}&size={GLOBE_PX}&zoom={:.1}&dark={}", self.globe_at.0, self.globe_at.1, self.globe_zoom, u8::from(self.dark)) });
            }
        }
        out
    }

    /// Time passed: the polls, the pictures' asks and the loop's step.
    pub fn tick(&mut self, now: u64, loops: bool) -> (Vec<Effect>, bool) {
        if !self.open {
            return (Vec::new(), false);
        }
        let mut out = self.ask(now);
        let mut changed = false;
        // the loop goes on when the picture of the frame after the one shown is in; else it rests on this one until it is
        if self.playing && loops && now >= self.step_at && self.next_frame().is_some_and(|f| self.has_picture(&f)) {
            changed = self.step(1, now);
            // the frame after the one now shown is asked for as the loop goes onto it
            out.extend(self.ask(now));
        }
        (out, changed)
    }

    /// When the next poll, ask or step is due.
    pub fn due_at(&self, loops: bool) -> Option<u64> {
        if !self.open {
            return None;
        }
        let step = (self.playing && loops && self.next_frame().is_some_and(|f| self.has_picture(&f))).then_some(self.step_at);
        // a view the surface has just said is asked for at once
        let view = self.view_new.then_some(0);
        [Some(self.next_poll), step, self.pics.due_at(), view].into_iter().flatten().min()
    }

    /// One frame on (`d` = 1) or back (-1) over the window; the loop rests on its last frame before it starts again.
    pub fn step(&mut self, d: i32, now: u64) -> bool {
        let win = self.window();
        if win.len() < 2 {
            return false;
        }
        let i = self.cur.and_then(|c| win.iter().position(|f| f.t == c)).unwrap_or(win.len() - 1) as i32;
        let n = win.len() as i32;
        let j = (i + d).rem_euclid(n);
        self.cur = Some(win[j as usize].t);
        self.step_at = now + if j == n - 1 && d > 0 { STEP_MS + END_HOLD_MS } else { STEP_MS };
        true
    }

    /// A key of the page: the channel keys step the loop a frame back and ahead (and hold it where it is), OK plays and holds it. The zoom is
    /// [`Radar::zoom_by`]; Left and Right are the view's pages. A frame that was stepped to is asked for as the step is made; the one shown before
    /// it stays on the screen until its picture is there.
    pub fn key(&mut self, name: &str, now: u64) -> (Vec<Effect>, bool) {
        match name {
            "chdown" => {
                self.playing = false;
                let changed = self.step(-1, now);
                (self.ask(now), changed)
            }
            "chup" => {
                self.playing = false;
                let changed = self.step(1, now);
                (self.ask(now), changed)
            }
            "ok" => {
                self.playing = !self.playing;
                self.step_at = now + STEP_MS;
                (Vec::new(), true)
            }
            _ => (Vec::new(), false),
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

    /// The pictures that came since the last call: the surface makes a layer of each.
    pub fn take_pics(&mut self) -> Vec<MapPic> {
        self.pics.take_pics()
    }

    /// The ids of the pictures whose layers the surface drops now.
    pub fn take_released(&mut self) -> Vec<u64> {
        self.pics.take_released()
    }

    /// The view the surface last said, for the tests.
    #[cfg(test)]
    pub fn view(&self) -> Option<View> {
        self.view
    }

    /// The bytes of pictures in the page's hands, for the tests.
    pub fn picture_bytes(&self) -> usize {
        self.pics.bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::super::radar_pics::testing::{answer, tags};
    use super::super::radar_zoom::{Z_GLOBE, Z_HOME, Z_MAX};
    use super::*;

    const HERE: View = View { cx: 0.26703, cy: 0.41163, zoom: 9.0 };

    fn state(frames: &[(i64, u64, bool, u64)], have: usize, want: usize) -> Value {
        json!({"status": "ready", "have": have, "want": want, "frames": frames.iter().map(|f| json!([f.0, f.1, f.2, f.3])).collect::<Vec<_>>(),
               "attribution": "x", "now": 10_000, "newest": 9_600})
    }

    fn fetched(r: &mut Radar, v: Value, now: u64) -> Vec<Effect> {
        r.input(&Input::Fetched { tag: TAG_STATE, status: 200, data: v }, now).0
    }

    /// A page that is open on a window of `n` frames ten minutes apart, the clock at the newest, and a view the surface has said.
    fn opened(n: i64) -> Radar {
        let mut r = Radar::default();
        r.open(33.7, -84.4, true, 1_000);
        let frames: Vec<(i64, u64, bool, u64)> = (0..n).map(|i| (10_000 - (n - 1 - i) * 600, 0, true, 0)).collect();
        fetched(&mut r, state(&frames, n as usize, n as usize), 1_100);
        r.set_view(HERE);
        r
    }

    /// Asks for what the page wants and answers each at once, as the helper does when it has the picture.
    fn settle(r: &mut Radar, mut now: u64) -> u64 {
        let mut asks = tags(&r.tick(now, false).0);
        while !asks.is_empty() {
            now += 10;
            let mut more = Vec::new();
            for (tag, path) in asks {
                more.extend(tags(&r.input(&answer(&path, tag), now).0));
            }
            asks = more;
        }
        now
    }

    #[test]
    fn opening_asks_the_helper_to_start_and_the_state_and_a_view_name_what_to_ask_for() {
        let mut r = Radar::default();
        let out = r.open(33.7, -84.4, true, 1_000);
        assert!(matches!(&out[0], Effect::Request { tag: TAG_OPEN, method, path, body: Some(b) } if method == "POST" && path == "/v1/wx/open" && b["lat"] == 33.7), "{out:?}");
        // a day of observed frames and the model's run: only the window is the page's
        let mut frames: Vec<(i64, u64, bool, u64)> = (0..30).map(|i| (10_000 - 86_400 + i * 2_880, 0, true, 0)).collect();
        frames.extend((0..12).map(|i| (10_000 - 6_000 + i * 600, 0, true, 0)));
        frames.extend((1..=30).map(|i| (10_000 + i * 900, 1, true, 0)));
        frames.extend((1..=4).map(|i| (10_000 + 4 * 86_400 + i * 3_600, 2, true, 0)));
        let out = fetched(&mut r, state(&frames, 5, 40), 1_100);
        assert!(tags(&out).is_empty(), "no view is said yet: nothing is asked for the map");
        assert_eq!(r.cur, Some(10_000 - 6_000 + 10 * 600), "the frame shown is the one nearest the clock");
        assert!(r.window().iter().all(|f| f.t >= 10_000 - BACK_S && f.t <= 10_000 + AHEAD_S && f.kind != Kind::Outlook), "the window: two hours back, six ahead, no outlook");
        r.set_view(HERE);
        assert_eq!(r.due_at(false), Some(0), "a view that is said is asked for at once");
        let asks = tags(&r.tick(1_200, false).0);
        let kinds: Vec<&str> = asks.iter().map(|(_, a)| a.split(&['?', '&'][..]).find_map(|p| p.strip_prefix("k=")).unwrap()).collect();
        assert_eq!(kinds, ["backdrop", "map", "radar", "radar"], "the map for the view, the one under it, the frame on show and the next");
        assert!(asks[1].1.contains("zoom=9&w=1920&h=1080&dark=1"), "{}", asks[1].1);
        assert!(asks[2].1.ends_with("&t=10000"), "the frame nearest the clock: {}", asks[2].1);
        assert!(r.tick(1_201, false).0.is_empty(), "and once");
    }

    #[test]
    fn the_loop_steps_over_the_window_when_the_next_picture_is_in_and_asks_for_the_one_after_as_it_goes() {
        let mut r = opened(4);
        assert_eq!(r.cur, Some(10_000), "nearest the clock");
        let asks = tags(&r.tick(1_200, true).0);
        assert_eq!(asks.len(), 4);
        // the picture of the frame on show comes; the next has not: the loop rests
        let radar: Vec<&(u64, String)> = asks.iter().filter(|(_, a)| a.contains("k=radar")).collect();
        let (cur_tag, cur_path) = radar[0].clone();
        r.input(&answer(&cur_path, cur_tag), 1_250);
        assert!(r.has_any_picture());
        assert!(!r.tick(2_000, true).1, "the next picture is not in: it rests where it is");
        assert_eq!(r.cur, Some(10_000));
        assert!(r.due_at(true).is_none_or(|d| d > 2_000), "and no tight loop waits for it: {:?}", r.due_at(true));
        // the next comes: the step is due, goes, and the frame after is asked for
        let (next_tag, next_path) = radar[1].clone();
        r.input(&answer(&next_path, next_tag), 2_100);
        assert!(r.due_at(true).is_some_and(|d| d <= 2_100), "due as soon as it is in");
        let (fx, changed) = r.tick(2_100, true);
        assert!(changed, "the loop stepped");
        assert_eq!(r.cur, Some(10_000 - 1_800), "round to the oldest: the window ends at the clock");
        assert_eq!(r.step_at, 2_100 + STEP_MS);
        assert!(tags(&fx).iter().any(|(_, a)| a.contains("k=radar") && a.contains("&t=8800")), "the frame after the one now shown is asked for: {fx:?}");
        // and it rests on its newest frame for longer than on the rest
        r.cur = Some(10_000 - 600);
        r.step(1, 5_000);
        assert_eq!((r.cur, r.step_at), (Some(10_000), 5_000 + STEP_MS + END_HOLD_MS));
        // a scrub holds the loop and moves at once; the picture follows
        r.playing = true;
        let (fx, changed) = r.key("chup", 3_000);
        assert!(!r.playing && changed && r.cur == Some(10_000 - 1_800), "a scrub holds the loop and moves at once");
        assert!(tags(&fx).is_empty() || tags(&fx).iter().all(|(_, a)| a.contains("k=radar")));
        let _ = r.key("ok", 3_100);
        assert!(r.playing, "OK plays again");
    }

    #[test]
    fn a_helper_that_let_the_engines_go_is_asked_to_open_again_once_and_the_view_is_asked_for_again_when_it_has() {
        let mut r = opened(3);
        let (out, _) = r.input(&Input::Fetched { tag: TAG_STATE, status: 409, data: Value::Null }, 5_000);
        assert!(out.iter().any(|e| matches!(e, Effect::Request { tag: TAG_OPEN, .. })));
        // a picture asked for in the meantime is refused the same way: no second open
        let asks = tags(&out);
        assert_eq!(asks.len(), 4, "the view was said and is asked for: {out:?}");
        let (out, _) = r.input(&Input::Fetched { tag: asks[0].0, status: 409, data: Value::Null }, 5_200);
        assert!(!out.iter().any(|e| matches!(e, Effect::Request { tag: TAG_OPEN, .. })), "{out:?}");
        let (out, _) = r.input(&Input::Fetched { tag: TAG_OPEN, status: 200, data: state(&[], 0, 0) }, 5_300);
        assert!(!out.iter().any(|e| matches!(e, Effect::Request { .. })));
        let (out, _) = r.input(&Input::Fetched { tag: asks[1].0, status: 409, data: Value::Null }, 5_400);
        assert!(out.iter().any(|e| matches!(e, Effect::Request { tag: TAG_OPEN, .. })), "a later loss is asked for again: {out:?}");
    }

    #[test]
    fn the_zoom_is_kept_and_the_earth_is_what_it_ends_in_and_the_page_starts_over_when_it_is_left() {
        let mut r = Radar::default();
        r.open(33.7, -84.4, false, 0);
        assert_eq!((r.zoom.target, r.globe()), (Z_HOME, false));
        assert!(r.zoom_by(0.25, None, 10).is_empty(), "nothing is asked of the helper for a zoom: the surface says where it goes");
        let out = r.zoom_by(-2.0, None, 20);
        assert_eq!((r.zoom.target, r.globe()), (Z_HOME - 1.75, false));
        assert!(out.is_empty());
        r.reached = true;
        let out = r.zoom_by(-1.0, None, 30);
        assert!(r.globe(), "past the wide view the earth is the picture");
        assert!(out.iter().any(|e| matches!(e, Effect::Fetch { tag: TAG_GLOBE, .. })), "and it is asked for: {out:?}");
        assert!(tags(&r.tick(40, false).0).is_empty(), "no map is asked for under it");
        assert!(r.zoom_by(-9.0, None, 50).len() <= 1 && r.zoom.target == Z_GLOBE, "clamped at the earth");
        assert!(r.zoom_by(-1.0, None, 60).is_empty(), "and nothing more is asked there");
        r.zoom_by(9.0, None, 70);
        assert_eq!((r.zoom.target, r.globe()), (Z_MAX, false));
        r.close();
        assert_eq!(r.zoom.target, Z_HOME);
        assert!(r.zoom_by(1.0, None, 80).is_empty(), "closed: no zoom");
    }

    #[test]
    fn closing_lets_the_helper_go_and_frees_every_picture_ask_and_buffer_of_the_page() {
        let mut r = opened(4);
        r.reached = true;
        let now = settle(&mut r, 1_200);
        assert!(r.picture_bytes() > 0 && r.has_any_picture());
        // and an ask still in flight
        r.set_view(View { zoom: 9.6, ..HERE });
        r.tick(now + 1_000, false);
        let out = r.close();
        assert_eq!(out, vec![Effect::Post { path: "/v1/wx/close".into(), body: json!({}) }]);
        assert!(!r.open && r.picture_bytes() == 0 && !r.has_any_picture());
        assert!(r.take_pics().is_empty() && r.take_released().is_empty(), "nothing is left to hand over");
        assert!(r.close().is_empty());
        assert_eq!(r.due_at(true), None);
        // an answer that comes after the close is dropped
        let (fx, changed) = r.input(&Input::Bytes { tag: TAG_FIRST, status: 200, bytes: vec![1, 2, 3] }, now + 2_000);
        assert!(fx.is_empty() && !changed);
    }
}
