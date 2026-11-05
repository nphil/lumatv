//! The weather view's pictures, held: which of the helper's mosaics the page holds for its view, asked for when the view moves and let go
//! when they are not wanted, so that what the page keeps in memory and on the graphics chip is a few pictures and not every frame of a day.
//!
//! The model is pure (no decode, no drawing): it hands the surface pictures to make layers of ([`Pics::take_pics`]) and the ids of pictures whose
//! layers are to be dropped ([`Pics::take_released`]), and the surface makes and drops them. What the helper sends is described in
//! `helper/app/src/wxpic.rs`: for the view (the world point at the screen's middle, the zoom, the screen's size) it picks the level and the
//! whole cells that cover it and answers one mosaic of raw premultiplied RGBA and a trailer, so a picture needs no decode here.
//!
//! **What is held.** Four pictures at most are wanted at once: the map for the view, a coarser map under it (zoom 7 and over), and the radar's
//! frame on show and the one after it (the loop's next step, asked for ahead so that a step is a change of opacity and not a wait). A
//! picture serves the view while it covers the screen and is of the level the view calls for ([`Role::level`], the same rule as the helper's
//! `wxpic`): a zoom within a half level or a small pan asks for nothing, and past it a new picture is asked for and the old one stays until
//! it comes, so that there is never a bare place; the replaced picture is kept [`RETIRE_MS`] more for the cross-fade and then released.
//! Frames the loop has gone past are released the same way.
//!
//! **Cancelling.** Every ask carries the view's number (`epoch`, a time in milliseconds that only grows): the helper drops work it had queued
//! for an older number. When the view moves the asks in flight are forgotten here (their answers are dropped when they come) and the
//! pictures that serve nothing are asked for again once the view has rested [`SETTLE_MS`], so that a held wheel is not a request a notch.
//! [`Pics::close`] releases every picture, held or on its way to the surface, and forgets every ask.

use crate::kit::picture::Picture;
use crate::kit::screen::{Effect, Input};
use std::collections::HashMap;

/// The tags of the page's picture asks.
pub const TAG_FIRST: u64 = 0x5700_0200;
pub const TAG_LAST: u64 = 0x5700_02ff;
/// The screen the pictures cover, in pixels.
pub const SCREEN_W: f64 = 1920.0;
pub const SCREEN_H: f64 = 1080.0;
/// The zoom the page opens at: the helper's background warming draws the pictures of this view.
pub const OPEN_ZOOM: f64 = 9.0;
/// Under this zoom the globe is the picture and no map or radar is held (a half level under the globe's hand-over, so a glide through it
/// keeps the map).
pub const MIN_ZOOM: f64 = 6.5;
/// Asks in flight at most.
const IN_FLIGHT: usize = 4;
/// How long after an answer of "coming", of a failure, and of a picture that was not whole an ask is made again.
const RETRY_COMING_MS: u64 = 100;
const RETRY_FAILED_MS: u64 = 3_000;
const RETRY_PARTIAL_MS: u64 = 2_000;
/// How long a view rests before the pictures that serve it no more are asked for again.
const SETTLE_MS: u64 = 150;
/// How long a replaced picture is kept: the cross-fade's time and a little over.
pub const RETIRE_MS: u64 = 400;
/// The most bytes of pictures the page may hold, replaced ones included. About 40 MB is the usual most (a map, its backdrop, two frames and
/// the map before a zoom).
pub const MAX_BYTES: usize = 72 << 20;

/// The trailer after a picture's pixels (`helper/app/src/wxpic.rs`).
const TRAILER: usize = 64;
const MAGIC: [u8; 4] = *b"WXP2";

/// What a picture is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// The base map for the view.
    Map,
    /// The same view two zoom levels coarser, under the map.
    Backdrop,
    /// The radar at one time (`rev` names the source's version of it).
    Radar { t: i64, rev: u32 },
}

impl Role {
    fn word(self) -> &'static str {
        match self {
            Role::Map => "map",
            Role::Backdrop => "backdrop",
            Role::Radar { .. } => "radar",
        }
    }

    /// The zoom the helper cuts the picture at for a view at `zoom` (the map's grid is in half levels, the radar's tile zoom whole), or none:
    /// `wxpic::map_grid`, `BACKDROP_STEP` and `radar_zoom`.
    pub fn level(self, zoom: f64) -> Option<f64> {
        let grid = (zoom * 2.0).round().clamp(10.0, 22.0);
        match self {
            Role::Map => Some(grid / 2.0),
            Role::Backdrop => (grid - 4.0 >= 10.0).then(|| (grid - 4.0) / 2.0),
            Role::Radar { .. } => Some((zoom - 3.0).round().clamp(2.0, 7.0)),
        }
    }
}

/// What the page is looking at: the world point (Web Mercator, 0 to 1 each way, y south) at the middle of its screen and the zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub cx: f64,
    pub cy: f64,
    pub zoom: f64,
}

impl View {
    /// The view as it is asked for, rounded to what the query holds (and so to what two asks of one view agree on).
    fn quantised(self) -> Self {
        let q = |v: f64, k: f64| (v * k).round() / k;
        Self { cx: q(self.cx, 1e9), cy: q(self.cy, 1e9), zoom: q(self.zoom, 1e3) }
    }

    /// Half the screen in world units, across and down.
    fn half(&self) -> (f64, f64) {
        let px = 256.0 * 2f64.powf(self.zoom);
        (SCREEN_W / 2.0 / px, SCREEN_H / 2.0 / px)
    }
}

/// A picture for the surface to make a layer of.
#[derive(Debug, PartialEq)]
pub struct MapPic {
    /// Names the picture to [`Pics::take_released`].
    pub id: u64,
    pub role: Role,
    /// The zoom it is cut at: the map's grid in whole and half levels, the radar's tile zoom.
    pub zoom: f64,
    /// The part of the world it covers: left, top, right, bottom.
    pub world: [f64; 4],
    pub picture: Picture,
    /// Whether it has all its cells (one that has not is asked for again).
    pub complete: bool,
}

/// Where a picture's layer goes in a space that is the world at `frame_zoom`, seen from `centre` on a screen of [`SCREEN_W`] by [`SCREEN_H`]:
/// the layer's top-left, as `Props::x` and `Props::y` have it for a layer of `px` pixels, and its `Props::scale` (which turns about the layer's
/// middle).
pub fn placement(world: [f64; 4], px: (u32, u32), centre: (f64, f64), frame_zoom: f64) -> (f32, f32, f32) {
    let k = 256.0 * 2f64.powf(frame_zoom);
    let scale = (world[2] - world[0]) * k / f64::from(px.0);
    let (mx, my) = ((world[0] + world[2]) / 2.0, (world[1] + world[3]) / 2.0);
    let (sx, sy) = (SCREEN_W / 2.0 + (mx - centre.0) * k, SCREEN_H / 2.0 + (my - centre.1) * k);
    ((sx - f64::from(px.0) / 2.0) as f32, (sy - f64::from(px.1) / 2.0) as f32, scale as f32)
}

struct Held {
    id: u64,
    role: Role,
    zoom: f64,
    world: [f64; 4],
    bytes: usize,
    complete: bool,
}

impl Held {
    /// Whether the picture serves the view: it covers the screen (a side of it at the edge of the world covers what is past it) and is of the level
    /// the view calls for.
    fn serves(&self, view: &View) -> bool {
        const EPS: f64 = 1e-9;
        let (hw, hh) = view.half();
        let [l, t, r, b] = self.world;
        self.complete
            && self.role.level(view.zoom).is_some_and(|z| (z - self.zoom).abs() < 1e-6)
            && (l <= view.cx - hw + EPS || l <= EPS)
            && (r >= view.cx + hw - EPS || r >= 1.0 - EPS)
            && (t <= view.cy - hh + EPS || t <= EPS)
            && (b >= view.cy + hh - EPS || b >= 1.0 - EPS)
    }
}

/// An ask in flight.
struct Ask {
    role: Role,
    view: View,
}

pub struct Pics {
    /// The helper is asked in this view number; it only grows.
    epoch: u64,
    /// The view the epoch is of.
    epoch_view: Option<View>,
    view: Option<View>,
    dark: bool,
    /// What is wanted now, by priority.
    wanted: Vec<Role>,
    held: Vec<Held>,
    /// Replaced pictures and the time they go.
    retiring: Vec<(Held, u64)>,
    inflight: HashMap<u64, Ask>,
    retry_at: HashMap<Role, u64>,
    arrived: Vec<MapPic>,
    released: Vec<u64>,
    /// Bytes of pictures that arrived and are not taken yet.
    arrived_bytes: usize,
    /// When the view rests enough for the asks to be made.
    settle_at: u64,
    next_id: u64,
    next_tag: u64,
    /// The helper no longer has the view (it restarted, or let go after ten minutes): the page opens it again.
    pub lost: bool,
}

impl Default for Pics {
    fn default() -> Self {
        Self {
            epoch: 0,
            epoch_view: None,
            view: None,
            dark: false,
            wanted: Vec::new(),
            held: Vec::new(),
            retiring: Vec::new(),
            inflight: HashMap::new(),
            retry_at: HashMap::new(),
            arrived: Vec::new(),
            released: Vec::new(),
            arrived_bytes: 0,
            settle_at: 0,
            next_id: 1,
            next_tag: TAG_FIRST,
            lost: false,
        }
    }
}

impl Pics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a picture of `role` is held (a replaced one that is only kept for its cross-fade is not).
    pub fn holds(&self, role: Role) -> bool {
        self.held.iter().any(|h| h.role == role)
    }

    /// The view and what of it is wanted (`roles`, the most wanted first) are as now: pictures that serve are kept, those that serve nothing
    /// asked for once the view has rested, and those no longer wanted let go. `view` is `None` where there is no map to show (the globe).
    pub fn update(&mut self, view: Option<View>, roles: &[Role], dark: bool, now: u64) -> Vec<Effect> {
        let view = view.filter(|v| v.zoom >= MIN_ZOOM).map(View::quantised);
        if dark != self.dark {
            // pictures of the other look serve nothing
            let all: Vec<Held> = std::mem::take(&mut self.held);
            for h in all {
                self.retire(h, now);
            }
            self.dark = dark;
        }
        let moved = view != self.view;
        let had_view = self.view.is_some();
        self.view = view;
        self.wanted = match view {
            Some(v) => roles.iter().copied().filter(|r| r.level(v.zoom).is_some()).collect(),
            None => Vec::new(),
        };
        // what is no longer wanted at all goes; what is wanted and serves nothing stays until its successor comes
        let keep: Vec<Role> = self.wanted.clone();
        let (stay, go): (Vec<Held>, Vec<Held>) = std::mem::take(&mut self.held).into_iter().partition(|h| keep.contains(&h.role));
        self.held = stay;
        for h in go {
            self.retire(h, now);
        }
        if moved {
            // the asks of the view that was are forgotten; when the view rests, those that serve nothing are made again
            self.inflight.clear();
            self.retry_at.clear();
            // the first view is asked for at once; a view that moves is waited on until it rests, whatever is held
            self.settle_at = if had_view { now + SETTLE_MS } else { now };
        }
        self.tick(now)
    }

    /// Time passed: replaced pictures that have had their cross-fade go, and the asks that are due are made.
    pub fn tick(&mut self, now: u64) -> Vec<Effect> {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.retiring).into_iter().partition(|(_, at)| *at <= now);
        self.retiring = kept;
        for (h, _) in gone {
            self.released.push(h.id);
        }
        self.ask(now)
    }

    /// When something is due: the view's rest, an ask to be made again, a replaced picture to go.
    pub fn due_at(&self) -> Option<u64> {
        let askable = self.view.is_some() && self.wanted.iter().any(|r| !self.serving(*r) && !self.inflight.values().any(|a| a.role == *r) && !self.retry_at.contains_key(r));
        let rest = askable.then_some(self.settle_at);
        let retry = self.retry_at.values().copied().min();
        let retire = self.retiring.iter().map(|(_, at)| *at).min();
        [rest, retry, retire].into_iter().flatten().min()
    }

    fn serving(&self, role: Role) -> bool {
        self.view.is_some_and(|v| self.held.iter().any(|h| h.role == role && h.serves(&v)))
    }

    /// Whether a picture of `role` that covers what the view shows, at the level it calls for, is held.
    pub fn serves(&self, role: Role) -> bool {
        self.serving(role)
    }

    fn retire(&mut self, h: Held, now: u64) {
        self.retiring.push((h, now + RETIRE_MS));
    }

    fn ask(&mut self, now: u64) -> Vec<Effect> {
        let Some(view) = self.view else { return Vec::new() };
        if now < self.settle_at {
            return Vec::new();
        }
        let needs: Vec<Role> = self.wanted.iter().copied().filter(|r| !self.serving(*r)).collect();
        // an ask for a view or a thing that is no longer needed is forgotten
        self.inflight.retain(|_, a| a.view == view && needs.contains(&a.role));
        let mut out = Vec::new();
        for role in needs {
            if self.inflight.len() >= IN_FLIGHT {
                break;
            }
            if self.inflight.values().any(|a| a.role == role) || self.retry_at.get(&role).is_some_and(|at| now < *at) {
                continue;
            }
            self.retry_at.remove(&role);
            if self.epoch_view != Some(view) {
                // the helper drops what it had queued for an older view
                self.epoch = (self.epoch + 1).max(now);
                self.epoch_view = Some(view);
            }
            let tag = self.next_tag;
            self.next_tag = if tag >= TAG_LAST { TAG_FIRST } else { tag + 1 };
            let mut path = format!("/v1/wx/pic?k={}&cx={}&cy={}&zoom={}&w={}&h={}&dark={}&epoch={}", role.word(), view.cx, view.cy, view.zoom, SCREEN_W, SCREEN_H, u8::from(self.dark), self.epoch);
            if let Role::Radar { t, .. } = role {
                path.push_str(&format!("&t={t}"));
            }
            self.inflight.insert(tag, Ask { role, view });
            out.push(Effect::Fetch { tag, path });
        }
        out
    }

    /// An answer to one of the asks. Whether anything changed that the surface must see.
    pub fn input(&mut self, input: &Input, now: u64) -> (Vec<Effect>, bool) {
        let (tag, status) = match input {
            Input::Bytes { tag, status, .. } | Input::Fetched { tag, status, .. } => (*tag, *status),
            _ => return (Vec::new(), false),
        };
        if !(TAG_FIRST..=TAG_LAST).contains(&tag) {
            return (Vec::new(), false);
        }
        // an ask that was forgotten (the view moved, or the page closed): the answer is dropped
        let Some(ask) = self.inflight.remove(&tag) else { return (Vec::new(), false) };
        let mut changed = false;
        match (input, status) {
            (Input::Bytes { bytes, .. }, 200) => match parse(bytes) {
                Some((w, h, complete, zoom_code, world)) => {
                    let rgba = bytes[..w as usize * h as usize * 4].to_vec();
                    let zoom = match ask.role {
                        Role::Radar { .. } => f64::from(zoom_code),
                        _ => f64::from(zoom_code) / 2.0,
                    };
                    let id = self.next_id;
                    self.next_id += 1;
                    let replaced: Vec<Held> = {
                        let (same, rest): (Vec<Held>, Vec<Held>) = std::mem::take(&mut self.held).into_iter().partition(|h| h.role == ask.role);
                        self.held = rest;
                        same
                    };
                    for h in replaced {
                        self.retire(h, now);
                    }
                    let bytes_len = rgba.len();
                    self.held.push(Held { id, role: ask.role, zoom, world, bytes: bytes_len, complete });
                    self.arrived_bytes += bytes_len;
                    self.arrived.push(MapPic { id, role: ask.role, zoom, world, picture: Picture { width: w, height: h, rgba }, complete });
                    if !complete {
                        self.retry_at.insert(ask.role, now + RETRY_PARTIAL_MS);
                    }
                    changed = true;
                    self.shed();
                }
                None => {
                    self.retry_at.insert(ask.role, now + RETRY_FAILED_MS);
                }
            },
            (_, 202) => {
                self.retry_at.insert(ask.role, now + RETRY_COMING_MS);
            }
            (_, 204) => {
                // no such picture for this view: nothing is asked of it again until the view moves
                self.retry_at.insert(ask.role, now + 60_000);
            }
            (_, 409) => {
                // asked again once the page has had the helper open its view
                self.lost = true;
                self.retry_at.insert(ask.role, now + RETRY_COMING_MS);
            }
            (_, 410) => {
                // the helper has a newer view than this ask's: not one of the page's own current ones, which carry the newest number
                self.retry_at.insert(ask.role, now + RETRY_COMING_MS);
            }
            _ => {
                self.retry_at.insert(ask.role, now + RETRY_FAILED_MS);
            }
        }
        let fx = self.ask(now);
        (fx, changed)
    }

    /// Lets the replaced pictures go at once when the pictures held are over the ceiling.
    fn shed(&mut self) {
        while self.bytes() > MAX_BYTES && !self.retiring.is_empty() {
            let (h, _) = self.retiring.remove(0);
            self.released.push(h.id);
        }
    }

    /// The pictures that arrived since the last call: the surface makes a layer of each.
    pub fn take_pics(&mut self) -> Vec<MapPic> {
        self.arrived_bytes = 0;
        std::mem::take(&mut self.arrived)
    }

    /// The ids of the pictures whose layers are to be dropped.
    pub fn take_released(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.released)
    }

    /// The pictures held now, replaced ones that are still kept included: (id, role).
    pub fn held(&self) -> Vec<(u64, Role)> {
        self.held.iter().map(|h| (h.id, h.role)).chain(self.retiring.iter().map(|(h, _)| (h.id, h.role))).collect()
    }

    /// The bytes of pictures in the page's hands: held, being replaced, and on their way to the surface.
    pub fn bytes(&self) -> usize {
        self.held.iter().map(|h| h.bytes).sum::<usize>() + self.retiring.iter().map(|(h, _)| h.bytes).sum::<usize>()
    }

    /// Lets every picture go and forgets every ask: the ids of the pictures the surface has are in [`Pics::take_released`], and the pictures
    /// that had not been taken are dropped.
    pub fn close(&mut self) {
        let ids: Vec<u64> = self.held.iter().map(|h| h.id).chain(self.retiring.iter().map(|(h, _)| h.id)).collect();
        self.released.extend(ids);
        self.held.clear();
        self.retiring.clear();
        self.arrived = Vec::new();
        self.arrived_bytes = 0;
        self.inflight.clear();
        self.retry_at.clear();
        self.wanted.clear();
        self.view = None;
        self.epoch_view = None;
    }

    /// Asks in flight, for the tests and the log.
    pub fn in_flight(&self) -> usize {
        self.inflight.len()
    }

    /// The view number the helper is asked in.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

/// A picture's size, whether it is whole, its level's code and the part of the world it covers, from the bytes the helper sent.
fn parse(bytes: &[u8]) -> Option<(u32, u32, bool, u8, [f64; 4])> {
    let at = bytes.len().checked_sub(TRAILER)?;
    let t = &bytes[at..];
    if t[..4] != MAGIC {
        return None;
    }
    let u32_at = |i: usize| -> Option<u32> { Some(u32::from_le_bytes(t.get(i..i + 4)?.try_into().ok()?)) };
    let f64_at = |i: usize| -> Option<f64> { Some(f64::from_le_bytes(t.get(i..i + 8)?.try_into().ok()?)) };
    let (w, h) = (u32_at(4)?, u32_at(8)?);
    if w == 0 || h == 0 || w > 4096 || h > 4096 || w as usize * h as usize * 4 != at {
        return None;
    }
    let world = [f64_at(24)?, f64_at(32)?, f64_at(40)?, f64_at(48)?];
    if world.iter().any(|v| !v.is_finite()) || world[2] <= world[0] || world[3] <= world[1] {
        return None;
    }
    Some((w, h, u32_at(12)? & 1 == 1, t[20], world))
}

/// What the other tests of the page's models need of the helper's picture format.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A picture as the helper seals it (its layout, `wxpic::PicQuery::seal`): `w` by `h` pixels of `fill`, then the trailer.
    pub fn sealed(w: u32, h: u32, fill: u8, complete: bool, level: u8, world: [f64; 4]) -> Vec<u8> {
        let mut b = vec![fill; (w * h * 4) as usize];
        b.extend_from_slice(b"WXP2");
        b.extend_from_slice(&w.to_le_bytes());
        b.extend_from_slice(&h.to_le_bytes());
        b.extend_from_slice(&u32::from(complete).to_le_bytes());
        b.extend_from_slice(&[1, 0, 1, 0]);
        b.push(level);
        b.extend_from_slice(&[0; 3]);
        for v in world {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[0; 8]);
        b
    }

    /// The world rect a map picture of grid `g` for a view has: the cells that cover the screen (the helper's rule, `wxpic::ViewQuery::plan`).
    pub fn cells_of(v: View, g: u8) -> [f64; 4] {
        let cell = 2f64.powf(-f64::from(g) / 2.0);
        let (hw, hh) = v.half();
        let span = |c: f64, half: f64| (((c - half) / cell).floor().max(0.0), ((c + half) / cell - 1e-9).floor());
        let ((x0, x1), (y0, y1)) = (span(v.cx, hw), span(v.cy, hh));
        [x0 * cell, y0 * cell, (x1 + 1.0) * cell, (y1 + 1.0) * cell]
    }

    pub fn tags(fx: &[Effect]) -> Vec<(u64, String)> {
        fx.iter().filter_map(|e| if let Effect::Fetch { tag, path } = e { Some((*tag, path.clone())) } else { None }).collect()
    }

    /// The answer the helper gives to the ask `path`: its picture.
    pub fn answer(path: &str, tag: u64) -> Input {
        let param = |k: &str| path.split(&['?', '&'][..]).find_map(|p| p.strip_prefix(&format!("{k}="))).unwrap().to_owned();
        let v = View { cx: param("cx").parse().unwrap(), cy: param("cy").parse().unwrap(), zoom: param("zoom").parse().unwrap() };
        let role = match param("k").as_str() {
            "map" => Role::Map,
            "backdrop" => Role::Backdrop,
            _ => Role::Radar { t: param("t").parse().unwrap(), rev: 0 },
        };
        let level = role.level(v.zoom).unwrap();
        let (code, world) = match role {
            Role::Radar { .. } => {
                let cell = 2f64.powf(-level);
                let (hw, hh) = v.half();
                ((level) as u8, [((v.cx - hw) / cell).floor() * cell, ((v.cy - hh) / cell).floor() * cell, ((v.cx + hw) / cell).ceil() * cell, ((v.cy + hh) / cell).ceil() * cell])
            }
            _ => ((level * 2.0) as u8, cells_of(v, (level * 2.0) as u8)),
        };
        Input::Bytes { tag, status: 200, bytes: sealed(4, 4, 7, true, code, world) }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{answer, cells_of, sealed, tags};
    use super::*;

    const ATLANTA: (f64, f64) = (0.26703, 0.41163);

    fn view(zoom: f64) -> View {
        View { cx: ATLANTA.0, cy: ATLANTA.1, zoom }
    }

    const MAP_ONLY: [Role; 1] = [Role::Map];
    const FRAMES: [Role; 2] = [Role::Radar { t: 100, rev: 1 }, Role::Radar { t: 700, rev: 2 }];
    const WANT: [Role; 4] = [Role::Backdrop, Role::Map, FRAMES[0], FRAMES[1]];

    fn open(p: &mut Pics, v: View, now: u64) -> Vec<(u64, String)> {
        tags(&p.update(Some(v), &WANT, true, now))
    }

    /// Answers every ask in flight with its picture, at `now`.
    fn answer_all(p: &mut Pics, asks: &[(u64, String)], now: u64) -> Vec<Effect> {
        let mut more = Vec::new();
        for (tag, path) in asks {
            more.extend(p.input(&answer(path, *tag), now).0);
        }
        more
    }

    #[test]
    fn a_view_asks_for_the_backdrop_the_map_and_the_frame_on_show_and_the_next() {
        let mut p = Pics::new();
        let asks = open(&mut p, view(9.0), 1_000);
        assert_eq!(asks.len(), 4, "{asks:?}");
        let kinds: Vec<&str> = asks.iter().map(|(_, a)| a.split(&['?', '&'][..]).find_map(|p| p.strip_prefix("k=")).unwrap()).collect();
        assert_eq!(kinds, ["backdrop", "map", "radar", "radar"]);
        assert!(asks[1].1.contains("cx=0.26703&cy=0.41163&zoom=9&w=1920&h=1080&dark=1&epoch="), "{}", asks[1].1);
        assert!(asks[2].1.ends_with("&t=100") && asks[3].1.ends_with("&t=700"), "the frame on show and the next: {asks:?}");
        assert_eq!(p.in_flight(), 4);
        assert!(p.update(Some(view(9.0)), &WANT, true, 1_001).is_empty(), "nothing more is asked for what is in flight");
    }

    #[test]
    fn a_picture_that_comes_is_handed_to_the_surface_with_its_place_and_counted() {
        let mut p = Pics::new();
        let asks = open(&mut p, view(9.0), 1_000);
        answer_all(&mut p, &asks, 1_100);
        let pics = p.take_pics();
        assert_eq!(pics.len(), 4);
        let map = pics.iter().find(|m| m.role == Role::Map).unwrap();
        assert_eq!((map.zoom, map.picture.width, map.complete), (9.0, 4, true));
        assert_eq!(map.world, cells_of(view(9.0), 18));
        let back = pics.iter().find(|m| m.role == Role::Backdrop).unwrap();
        assert_eq!(back.zoom, 7.0, "two levels coarser");
        assert_eq!(p.bytes(), 4 * 4 * 4 * 4);
        assert!(p.take_pics().is_empty(), "handed over once");
        assert_eq!(p.update(Some(view(9.0)), &WANT, true, 1_200), vec![], "and they serve the view: nothing is asked");
    }

    #[test]
    fn a_small_zoom_or_pan_asks_for_nothing_and_one_that_leaves_the_picture_does() {
        let mut p = Pics::new();
        let asks = tags(&p.update(Some(view(9.0)), &MAP_ONLY, true, 0));
        answer_all(&mut p, &asks, 100);
        p.take_pics();
        // a small zoom (the same half level) and a pan inside what the cells cover
        assert!(p.update(Some(view(9.2)), &MAP_ONLY, true, 1_000).is_empty());
        assert!(p.serves(Role::Map), "the map covers the view: what is under it is not seen");
        let [l, _, r, _] = cells_of(view(9.0), 18);
        let slack = ((view(9.2).cx - view(9.2).half().0) - l).min(r - (view(9.2).cx + view(9.2).half().0));
        assert!(slack > 0.0, "the cells hold more than the screen at this zoom: {slack}");
        let nudged = View { cx: view(9.2).cx + slack * 0.5, ..view(9.2) };
        assert!(p.update(Some(nudged), &MAP_ONLY, true, 2_000).is_empty());
        assert_eq!(p.due_at(), None, "nothing to wait for");
        // a pan past the picture's edge: asked for once the view has rested
        let far = View { cx: ATLANTA.0 + 0.002, ..view(9.2) };
        assert!(p.update(Some(far), &MAP_ONLY, true, 3_000).is_empty(), "the view has not rested");
        assert!(!p.serves(Role::Map), "and the map no longer covers it: what is under it is seen");
        assert_eq!(p.due_at(), Some(3_000 + SETTLE_MS));
        let asks = tags(&p.tick(3_000 + SETTLE_MS));
        assert_eq!(asks.len(), 1);
        assert!(asks[0].1.contains("k=map") && asks[0].1.contains("cx=0.26903"), "{asks:?}");
        // a zoom to the next half level asks for the map, the backdrop and the radar at their new levels
        let mut p = Pics::new();
        let asks = open(&mut p, view(9.0), 0);
        answer_all(&mut p, &asks, 100);
        p.take_pics();
        p.update(Some(view(9.6)), &WANT, true, 5_000);
        let asks = tags(&p.tick(5_000 + SETTLE_MS));
        assert!(asks.iter().any(|(_, a)| a.contains("k=map") && a.contains("zoom=9.6")), "{asks:?}");
        assert_eq!(asks.len(), 4, "the radar tile zoom changes at 9.5 too: {asks:?}");
    }

    #[test]
    fn a_picture_that_is_replaced_stays_for_the_cross_fade_and_is_then_released() {
        let mut p = Pics::new();
        let asks = open(&mut p, view(9.0), 0);
        answer_all(&mut p, &asks, 100);
        let first: Vec<u64> = p.take_pics().iter().filter(|m| m.role == Role::Map).map(|m| m.id).collect();
        p.update(Some(view(9.6)), &WANT, true, 1_000);
        let asks = tags(&p.tick(1_000 + SETTLE_MS));
        assert!(p.take_released().is_empty(), "the old map stays until its successor is there");
        answer_all(&mut p, &asks, 1_400);
        let now_pics = p.take_pics();
        assert!(now_pics.iter().any(|m| m.role == Role::Map && m.zoom == 9.5));
        assert!(p.take_released().is_empty(), "and then a while more, for the fade");
        assert!(p.held().iter().any(|(id, _)| first.contains(id)));
        p.tick(1_400 + RETIRE_MS - 1);
        assert!(p.take_released().is_empty());
        p.tick(1_400 + RETIRE_MS);
        let released = p.take_released();
        assert!(first.iter().all(|id| released.contains(id)), "{released:?}");
        assert!(!p.held().iter().any(|(id, _)| first.contains(id)));
    }

    #[test]
    fn the_loop_stepping_releases_the_frame_it_has_gone_past_and_the_next_is_already_there() {
        let mut p = Pics::new();
        let asks = open(&mut p, view(9.0), 0);
        answer_all(&mut p, &asks, 100);
        let pics = p.take_pics();
        let second = pics.iter().find(|m| m.role == FRAMES[1]).map(|m| m.id).unwrap();
        // the loop is on the second frame; the one after it is asked for, the first goes after the fade
        let after = Role::Radar { t: 1300, rev: 3 };
        let asks = tags(&p.update(Some(view(9.0)), &[Role::Backdrop, Role::Map, FRAMES[1], after], true, 600));
        assert_eq!(asks.len(), 1, "only the frame after is asked for: {asks:?}");
        assert!(asks[0].1.ends_with("&t=1300"));
        p.tick(600 + RETIRE_MS);
        let released = p.take_released();
        assert_eq!(released.len(), 1, "the first frame");
        assert!(!released.contains(&second));
        assert_eq!(p.held().iter().filter(|(_, r)| matches!(r, Role::Radar { .. })).count(), 1, "{:?}", p.held());
    }

    #[test]
    fn asks_for_a_view_that_has_gone_are_forgotten_and_their_answers_dropped() {
        let mut p = Pics::new();
        let asks = open(&mut p, view(9.0), 1_000);
        let epoch = p.epoch();
        // the view moves before any answer: the asks of the one that was are forgotten, and the new view is asked for, in a newer number, once it rests
        assert!(p.update(Some(view(9.6)), &WANT, true, 1_050).is_empty());
        assert_eq!(p.in_flight(), 0);
        let (fx, changed) = p.input(&answer(&asks[1].1, asks[1].0), 1_060);
        assert!((fx.is_empty(), changed, p.take_pics().len(), p.bytes()) == (true, false, 0, 0), "a late answer is dropped, not held");
        let fresh = tags(&p.tick(1_050 + SETTLE_MS));
        assert_eq!(fresh.len(), 4);
        assert!(p.epoch() > epoch, "the helper is told by a newer number");
        assert!(fresh[0].1.contains(&format!("epoch={}", p.epoch())));
        assert_eq!(p.in_flight(), 4);
    }

    #[test]
    fn the_helpers_other_answers_are_waited_out_or_noted() {
        let mut p = Pics::new();
        let asks = open(&mut p, view(9.0), 0);
        // coming: asked again soon
        let (fx, _) = p.input(&Input::Fetched { tag: asks[1].0, status: 202, data: serde_json::json!({"coming": true}) }, 100);
        assert!(fx.is_empty());
        assert_eq!(p.due_at(), Some(100 + RETRY_COMING_MS));
        assert_eq!(tags(&p.tick(100 + RETRY_COMING_MS)).len(), 1);
        // a refusal of a view that has moved on and a failure
        let (_, _) = p.input(&Input::Fetched { tag: asks[0].0, status: 410, data: serde_json::Value::Null }, 200);
        let (_, _) = p.input(&Input::Fetched { tag: asks[2].0, status: 0, data: serde_json::Value::Null }, 200);
        assert!(p.due_at().is_some());
        // the helper no longer has the view
        assert!(!p.lost);
        p.input(&Input::Fetched { tag: asks[3].0, status: 409, data: serde_json::Value::Null }, 300);
        assert!(p.lost);
    }

    #[test]
    fn a_picture_that_is_not_whole_is_used_and_asked_for_again() {
        let mut p = Pics::new();
        let asks = open(&mut p, view(9.0), 0);
        let (tag, path) = asks.iter().find(|(_, a)| a.contains("k=map")).cloned().unwrap();
        let Input::Bytes { bytes, .. } = answer(&path, tag) else { unreachable!() };
        let mut partial = bytes;
        let at = partial.len() - 64 + 12;
        partial[at] = 0;
        p.input(&Input::Bytes { tag, status: 200, bytes: partial }, 100);
        let pics = p.take_pics();
        assert!(!pics[0].complete);
        assert_eq!(p.due_at(), Some(100 + RETRY_PARTIAL_MS));
        assert_eq!(tags(&p.tick(100 + RETRY_PARTIAL_MS)).len(), 1, "asked for again");
    }

    #[test]
    fn the_globe_and_the_far_view_hold_no_map_and_a_closed_page_holds_nothing() {
        let mut p = Pics::new();
        let asks = open(&mut p, view(9.0), 0);
        answer_all(&mut p, &asks, 100);
        let mut ids: Vec<u64> = p.take_pics().iter().map(|m| m.id).collect();
        ids.sort_unstable();
        // zoomed out to the globe: nothing is wanted, everything goes after the fade
        assert!(p.update(Some(view(5.0)), &WANT, true, 1_000).is_empty());
        p.tick(1_000 + RETIRE_MS);
        let mut released = p.take_released();
        released.sort_unstable();
        assert_eq!(released, ids);
        assert_eq!(p.bytes(), 0);
        // back in, and closed with asks in flight and pictures on the way to the surface
        let asks = open(&mut p, view(9.0), 5_000);
        answer_all(&mut p, &asks[..2], 5_100);
        let in_flight = asks.len() - 2;
        assert_eq!(p.in_flight(), in_flight);
        p.close();
        let released = p.take_released();
        assert_eq!(released.len(), 2, "the two that had come are released, and what the surface was not yet given is dropped: {released:?}");
        assert!((p.bytes(), p.in_flight(), p.take_pics().len()) == (0, 0, 0) && p.due_at().is_none());
        let (fx, changed) = p.input(&answer(&asks[3].1, asks[3].0), 5_200);
        assert!(fx.is_empty() && !changed && p.bytes() == 0, "an answer after the close is dropped");
    }

    #[test]
    fn the_replaced_pictures_go_at_once_when_the_page_holds_more_than_its_ceiling() {
        let mut p = Pics::new();
        let v = view(9.0);
        let _ = open(&mut p, v, 0);
        // a held map of nearly the ceiling, replaced by another
        p.held.push(Held { id: 900, role: Role::Map, zoom: 9.0, world: [0.0, 0.0, 1.0, 1.0], bytes: MAX_BYTES - 10, complete: true });
        p.retire(Held { id: 901, role: Role::Map, zoom: 9.0, world: [0.0, 0.0, 1.0, 1.0], bytes: 100, complete: true }, 0);
        p.shed();
        assert_eq!(p.take_released(), vec![901], "{:?}", p.held());
    }

    #[test]
    fn a_layer_is_placed_by_the_world_it_covers() {
        // a picture of the screen's worth at the zoom the view is at: no scale, its top-left the screen's
        let v = view(9.0);
        let (hw, hh) = v.half();
        let world = [v.cx - hw, v.cy - hh, v.cx + hw, v.cy + hh];
        let (x, y, s) = placement(world, (1920, 1080), (v.cx, v.cy), 9.0);
        assert!(x.abs() < 1e-2 && y.abs() < 1e-2 && (s - 1.0).abs() < 1e-6, "{x} {y} {s}");
        // the same picture at the next zoom: twice the size, turning about its middle, which is still the screen's
        let (x, y, s) = placement(world, (1920, 1080), (v.cx, v.cy), 10.0);
        assert!(x.abs() < 1e-2 && y.abs() < 1e-2 && (s - 2.0).abs() < 1e-6, "{x} {y} {s}");
        // a picture that starts at the middle of the screen and runs right and down a screen's worth: its middle is at the screen's corner
        let (x, y, s) = placement([v.cx, v.cy, v.cx + 2.0 * hw, v.cy + 2.0 * hh], (1920, 1080), (v.cx, v.cy), 9.0);
        assert!((x - 960.0).abs() < 1e-2 && (y - 540.0).abs() < 1e-2 && (s - 1.0).abs() < 1e-6, "{x} {y} {s}");
        // a picture of half the pixels that covers the screen is stretched twice
        let (_, _, s) = placement(world, (960, 540), (v.cx, v.cy), 9.0);
        assert!((s - 2.0).abs() < 1e-6);
    }

    #[test]
    fn a_mosaic_that_is_not_what_the_helper_sends_is_refused() {
        let ok = sealed(4, 4, 1, true, 18, [0.1, 0.1, 0.2, 0.2]);
        assert!(parse(&ok).is_some());
        let mut bad_magic = ok.clone();
        bad_magic[4 * 4 * 4] = b'X';
        let mut short = ok.clone();
        short.remove(0);
        for bytes in [bad_magic, short, vec![0; 10], sealed(4, 4, 1, true, 18, [0.2, 0.1, 0.1, 0.2])] {
            assert!(parse(&bytes).is_none());
        }
    }
}
