//! The glow around the whole screen while the voice assistant listens and thinks: a soft band along each edge in the theme's accent, info
//! and live colours that flows clockwise (faster while it thinks) and breathes.
//!
//! The compositor has no edge-glow program, so the band is four baked strips. One strip is the pattern for one period along the
//! perimeter, in as many identical tiles as cover the strip's length and its travel; the tiles hang under a group whose translation
//! follows a motion track, so the flow costs the app nothing per frame. A tile is at most 2048 px across (a layer is guaranteed that and
//! no more: `Limits::for_texture_size`). The track runs forward all the time, so every period the group is bound again one period back,
//! which no one can see because the tiles are the same.
//!
//! Panel care. The glow is transient (the surface gives it at most 35 s), it never rests: it flows, or with reduced motion it breathes.
//! It reaches 64 px in from the edge, falling off smoothly to nothing. What it adds is bounded by [`GLOW_CAP_OF_INK`]: no pixel of the
//! baked bands, and no place where two bands overlap at a corner, has a premultiplied linear-light luminance above that share of the
//! theme's brightest UI colour.

use crate::kit::Kit;
use compositor_protocol::{Ease, Edges, GroupPose, PoseBinding, PoseProperties, Props, Rect};
use std::io;
use ui::error::UiError;
use ui::group::Group;
use ui::lucent::palette::{PaletteRole, Roles, Straight};
use ui::motion::{CancelPolicy, Motion, MotionBounds, MotionToken, Settle, Units};
use ui::scene::ImageLayer;

/// How far in from the edge the glow reaches: 5.9 % of the screen's height.
pub const BAND_PX: u32 = 64;
/// The most light the glow adds, as a share of the linear luminance of the theme's `ink` (or its canvas in a light theme).
pub const GLOW_CAP_OF_INK: f32 = 0.30;
/// The strongest the glow looks (alpha), where the cap is not the lower limit.
pub const PEAK_ALPHA: f32 = 0.55;
/// One period of the pattern: the perimeter (6000 px) is four of them, and a tile is one.
pub const PERIOD: u32 = 1_500;
const W: u32 = 1920;
const H: u32 = 1080;
const FADE_IN_MS: u32 = 250;
pub const FADE_OUT_MS: u32 = 600;
/// Clockwise speeds in px/s: a period in 4 s while it listens, in 2 s while it thinks.
const LISTEN_PX_S: f32 = 375.0;
const THINK_PX_S: f32 = 750.0;
/// A flow settle runs this long (the longest a timed settle may); the surface's 35 s is shorter, and a change of pace starts another.
const RUN_MS: u32 = 50_000;
/// Half a breath: the glow brightens and dims between [`BREATH_LOW`] and 1.
const BREATH_LISTEN_MS: u32 = 2_200;
const BREATH_THINK_MS: u32 = 1_400;
const BREATH_REDUCED_MS: u32 = 4_000;
const BREATH_LOW: f32 = 0.85;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    Listening,
    Thinking,
}

impl Pace {
    fn speed(self) -> f32 {
        match self {
            Self::Listening => LISTEN_PX_S,
            Self::Thinking => THINK_PX_S,
        }
    }

    fn breath_ms(self) -> u32 {
        match self {
            Self::Listening => BREATH_LISTEN_MS,
            Self::Thinking => BREATH_THINK_MS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

const SIDES: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

impl Side {
    fn horizontal(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }

    /// Which way the pattern moves on this side, going clockwise.
    fn dir(self) -> (f32, f32) {
        match self {
            Self::Top => (1.0, 0.0),
            Self::Right => (0.0, 1.0),
            Self::Bottom => (-1.0, 0.0),
            Self::Left => (0.0, -1.0),
        }
    }

    /// The tiles of the side, as multiples of the period from the strip's start: enough to cover its length at any translation the
    /// flow is allowed to reach before it is bound again (-1016 px to 2936 px along the top, -356 to 2936 on the sides).
    fn tiles(self) -> std::ops::RangeInclusive<i32> {
        if self.horizontal() { -2..=1 } else { -2..=0 }
    }

    /// The pattern coordinate (the position along the perimeter, clockwise from the top left corner, modulo the period) of `along`
    /// pixels into the tile.
    fn pattern(self, along: u32) -> usize {
        let p = PERIOD;
        let s = match self {
            Self::Top => along,
            Self::Right => W % p + along,
            Self::Bottom => (2 * W + H) % p + p - along,
            Self::Left => (2 * W + 2 * H) % p + p - along,
        };
        (s % p) as usize
    }

    /// The tile's size and where the strip lies on the screen (its corner), and the window that shows it.
    fn size(self) -> (u32, u32) {
        if self.horizontal() { (PERIOD, BAND_PX) } else { (BAND_PX, PERIOD) }
    }

    fn corner(self) -> (f32, f32) {
        match self {
            Self::Top | Self::Left => (0.0, 0.0),
            Self::Right => ((W - BAND_PX) as f32, 0.0),
            Self::Bottom => (0.0, (H - BAND_PX) as f32),
        }
    }

    /// The window: the strip with its ends running past the screen's corner by a band, and the feather that takes the strip to half
    /// where the screen ends and to nothing past it, so that two strips meeting at a corner add up to one.
    fn window(self) -> (Rect, Edges) {
        let b = BAND_PX as f32;
        let (x, y) = self.corner();
        if self.horizontal() {
            (Rect { x: -b, y, w: W as f32 + 2.0 * b, h: b }, Edges::new(2.0 * b, 2.0 * b, 0.0, 0.0))
        } else {
            (Rect { x, y: -b, w: b, h: H as f32 + 2.0 * b }, Edges::new(0.0, 0.0, 2.0 * b, 2.0 * b))
        }
    }
}

/// The sRGB transfer to linear light.
fn lin(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// The linear-light luminance of premultiplied bytes as the compositor holds them.
pub fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
}

/// A byte for `v` (0 to 1), rounded down so that quantising never adds light.
fn byte(v: f32) -> u8 {
    (v * 255.0).floor().clamp(0.0, 255.0) as u8
}

fn rgb(c: Straight) -> [f32; 3] {
    [f32::from(c.r) / 255.0, f32::from(c.g) / 255.0, f32::from(c.b) / 255.0]
}

/// The most light the glow may add under `roles`.
pub fn ceiling(roles: &Roles) -> f32 {
    GLOW_CAP_OF_INK * luminance(rgb(roles[PaletteRole::Ink])).max(luminance(rgb(roles[PaletteRole::Canvas])))
}

/// How much of the colour `c` may be laid on (alpha) so that the bytes of its premultiplied pixel stay under `cap`.
fn alpha_cap(c: [f32; 3], cap: f32) -> f32 {
    let light = |a: f32| luminance([0, 1, 2].map(|i| f32::from(byte(c[i] * a)) / 255.0));
    if light(1.0) <= cap {
        return 1.0;
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..28 {
        let mid = (lo + hi) / 2.0;
        if light(mid) <= cap { lo = mid } else { hi = mid }
    }
    lo
}

/// The colour of the pattern at `s` (0 to the period): the accent, the info and the live colours one into the next, round and back.
fn colour_at(stops: &[[f32; 3]; 3], s: usize) -> [f32; 3] {
    let pos = s as f32 / PERIOD as f32 * 3.0;
    let (i, u) = (pos.floor() as usize % 3, pos.fract());
    let u = u * u * (3.0 - 2.0 * u);
    let (a, b) = (stops[i], stops[(i + 1) % 3]);
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * u)
}

/// What is laid on at each point of the pattern where the band is at its fullest: the premultiplied colour and the alpha. The light
/// varies along the edge, 1 at the accent and .2 less between the others.
fn columns(roles: &Roles) -> Vec<[f32; 4]> {
    let stops = [PaletteRole::Accent, PaletteRole::Info, PaletteRole::Live].map(|r| rgb(roles[r]));
    let cap = ceiling(roles);
    (0..PERIOD as usize)
        .map(|s| {
            let c = colour_at(&stops, s);
            let swell = 0.6 + 0.4 * (std::f32::consts::TAU * s as f32 / PERIOD as f32).cos();
            let a = (PEAK_ALPHA * swell).min(alpha_cap(c, cap));
            [c[0] * a, c[1] * a, c[2] * a, a]
        })
        .collect()
}

/// How much of the glow is left `u` of the way in from the edge (0 to 1): all of it at the edge, none at the band's inner side.
pub fn falloff(u: f32) -> f32 {
    (1.0 - u.clamp(0.0, 1.0)).powi(2)
}

/// The pixels of one tile of `side`.
fn bake(side: Side, cols: &[[f32; 4]]) -> Vec<u8> {
    let (w, h) = side.size();
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let (along, across) = if side.horizontal() { (x, y) } else { (y, x) };
            let from_edge = if matches!(side, Side::Top | Side::Left) { across } else { BAND_PX - 1 - across };
            let f = falloff((from_edge as f32 + 0.5) / BAND_PX as f32);
            let [r, g, b, a] = cols[side.pattern(along)];
            let i = ((y * w + x) * 4) as usize;
            pixels[i..i + 4].copy_from_slice(&[byte(r * f), byte(g * f), byte(b * f), byte(a * f)]);
        }
    }
    pixels
}

fn other(e: UiError) -> io::Error {
    io::Error::other(format!("{e}"))
}

pub struct Glow {
    root: Group,
    movers: Vec<(Side, Group)>,
    /// What keeps the rest of the groups and the layers alive.
    kept: (Vec<Group>, Vec<ImageLayer>),
    fade: Motion,
    breath: Motion,
    flow: Motion,
    reduced: bool,
    on: Option<Pace>,
    shown: bool,
    hide_at: Option<u64>,
    /// The flow's translation follows `tau + (track - at)`, in px.
    anchor: (f32, f32),
    anchor_at: Option<u64>,
    breath_up: bool,
    breath_at: Option<u64>,
}

impl Glow {
    /// The glow in the colours of the current look, hidden. 14 layers, 10 groups and 3 tracks.
    pub fn new(kit: &mut Kit, z: i32) -> io::Result<Self> {
        let scene = kit.ui.scene.clone();
        let cols = columns(&kit.ui.look.theme.roles);
        let hidden = GroupPose { visible: false, ..GroupPose::IDENTITY };
        let root = scene.group(None, hidden).map_err(other)?;
        let breath_group = scene.group(Some(root.id()), GroupPose::IDENTITY).map_err(other)?;
        let fade = scene.motion(0.0, MotionBounds::range(0.0, 1.0, Units::Progress)).map_err(other)?;
        let breath = scene.motion(0.0, MotionBounds::range(0.0, 1.0, Units::Progress)).map_err(other)?;
        let flow = scene.motion(0.0, MotionBounds::range(0.0, 1.0e7, Units::Pixels)).map_err(other)?;
        root.bind(&fade, PoseBinding { values: [0.0, 1.0], from: GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }, to: GroupPose::IDENTITY, properties: PoseProperties::OPACITY })
            .map_err(other)?;
        breath_group
            .bind(&breath, PoseBinding { values: [0.0, 1.0], from: GroupPose { opacity: BREATH_LOW, ..GroupPose::IDENTITY }, to: GroupPose::IDENTITY, properties: PoseProperties::OPACITY })
            .map_err(other)?;
        let (mut windows, mut movers, mut tiles) = (Vec::new(), Vec::new(), Vec::new());
        for side in SIDES {
            let (clip, feather) = side.window();
            let window = scene.group(Some(breath_group.id()), GroupPose { clip: Some(clip), feather, ..GroupPose::IDENTITY }).map_err(other)?;
            let mover = scene.group(Some(window.id()), GroupPose::IDENTITY).map_err(other)?;
            let baked = bake(side, &cols);
            let (w, h) = side.size();
            let (cx, cy) = side.corner();
            for k in side.tiles() {
                let at = k as f32 * PERIOD as f32;
                let local = if side.horizontal() { Props::at(at, cy) } else { Props::at(cx, at) };
                let mut tile = scene.image(w, h, z, Props { visible: true, ..local })?;
                tile.pixels_mut().copy_from_slice(&baked);
                tile.damage(None);
                mover.attach(&tile, local).map_err(other)?;
                tiles.push(tile);
            }
            windows.push(window);
            movers.push((side, mover));
        }
        windows.extend([breath_group]);
        Ok(Self {
            root,
            movers,
            kept: (windows, tiles),
            fade,
            breath,
            flow,
            reduced: kit.ui.look.reduced_motion,
            on: None,
            shown: false,
            hide_at: None,
            anchor: (0.0, 0.0),
            anchor_at: None,
            breath_up: false,
            breath_at: None,
        })
    }

    /// Layers, groups and tracks it holds.
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.kept.1.len(), self.kept.0.len() + self.movers.len() + 1, 3)
    }

    /// Whether the glow is on the screen, fading in or out included.
    pub fn visible(&self) -> bool {
        self.shown
    }

    #[cfg(test)]
    fn flowing(&self) -> bool {
        self.flow.is_active()
    }

    #[cfg(test)]
    fn breathing(&self) -> bool {
        self.breath.is_active()
    }

    fn bind_flow(&mut self, at: f32, tau: f32) -> io::Result<()> {
        for (side, mover) in &self.movers {
            let (dx, dy) = side.dir();
            let pose = |t: f32| GroupPose::at(dx * t, dy * t);
            mover
                .bind(&self.flow, PoseBinding { values: [at, at + 1.0], from: pose(tau), to: pose(tau + 1.0), properties: PoseProperties::TRANSLATION })
                .map_err(other)?;
        }
        self.anchor = (at, tau);
        Ok(())
    }

    fn tau(&self) -> f32 {
        self.anchor.1 + (self.flow.value() - self.anchor.0)
    }

    fn arm_flow(&mut self, pace: Pace, now: u64) -> io::Result<()> {
        let from = self.flow.value();
        let token = MotionToken { duration_ms: RUN_MS, ease: Ease::Linear };
        self.flow.settle(Some(from + pace.speed() * RUN_MS as f32 / 1000.0), None, Settle::Timed(token)).map_err(other)?;
        self.schedule_anchor(pace, now);
        Ok(())
    }

    fn schedule_anchor(&mut self, pace: Pace, now: u64) {
        let left = (PERIOD as f32 - self.tau()).max(0.0);
        self.anchor_at = Some(now + ((left / pace.speed() * 1000.0) as u64).max(10));
    }

    fn arm_breath(&mut self, pace: Pace, now: u64) -> io::Result<()> {
        self.breath_up = !self.breath_up;
        let ms = if self.reduced { BREATH_REDUCED_MS } else { pace.breath_ms() };
        let token = MotionToken { duration_ms: ms, ease: Ease::Spring };
        self.breath.settle(Some(if self.breath_up { 1.0 } else { 0.0 }), None, Settle::Timed(token)).map_err(other)?;
        self.breath_at = Some(now + u64::from(ms));
        Ok(())
    }

    fn fade_to(&self, to: f32, ms: u32, ease: Ease) -> io::Result<()> {
        self.fade.settle(Some(to), None, Settle::Timed(MotionToken { duration_ms: ms, ease })).map_err(other)
    }

    /// What the glow does now: `Some` pace to be on at, `None` to go. A change of pace carries on from where the flow is.
    pub fn set(&mut self, kit: &Kit, want: Option<Pace>, now: u64) -> io::Result<()> {
        self.reduced = kit.ui.look.reduced_motion;
        match (want, self.on) {
            (Some(pace), Some(was)) if pace == was => {}
            (Some(pace), Some(_)) => {
                if !self.reduced {
                    self.arm_flow(pace, now)?;
                }
                self.on = Some(pace);
            }
            (Some(pace), None) => {
                self.root.set_pose(GroupPose::IDENTITY);
                self.shown = true;
                self.hide_at = None;
                self.fade_to(1.0, kit.ui.look.duration_ms(FADE_IN_MS), Ease::Out)?;
                if !self.reduced {
                    // the track starts again from nothing, so that it never runs out of range over many turns
                    self.flow.settle(Some(0.0), Some(0.0), Settle::Timed(MotionToken { duration_ms: 0, ease: Ease::Linear })).map_err(other)?;
                    self.bind_flow(0.0, 0.0)?;
                    self.arm_flow(pace, now)?;
                }
                self.breath_up = false;
                self.arm_breath(pace, now)?;
                self.on = Some(pace);
            }
            (None, Some(_)) => {
                let ms = kit.ui.look.duration_ms(FADE_OUT_MS);
                self.fade_to(0.0, ms, Ease::In)?;
                self.hide_at = Some(now + u64::from(ms) + 50);
                self.on = None;
            }
            (None, None) => {}
        }
        Ok(())
    }

    /// The glow's own timers: the flow bound again a period back, the breath turned, the hiding once it has faded.
    pub fn tick(&mut self, now: u64) -> io::Result<()> {
        if self.hide_at.is_some_and(|at| at <= now) {
            self.root.set_pose(GroupPose { visible: false, ..GroupPose::IDENTITY });
            // a track that is settling has the compositor draw every frame: none is left to run with nothing to show
            self.flow.cancel(CancelPolicy::Hold).map_err(other)?;
            self.breath.cancel(CancelPolicy::Hold).map_err(other)?;
            self.shown = false;
            self.hide_at = None;
            self.anchor_at = None;
            self.breath_at = None;
        }
        let Some(pace) = self.on else { return Ok(()) };
        if self.anchor_at.is_some_and(|at| at <= now) {
            let (at, tau) = (self.flow.value(), self.tau());
            if tau >= PERIOD as f32 {
                self.bind_flow(at, tau - PERIOD as f32)?;
            }
            self.schedule_anchor(pace, now);
        }
        if self.breath_at.is_some_and(|at| at <= now) {
            self.arm_breath(pace, now)?;
        }
        Ok(())
    }

    pub fn due_at(&self) -> Option<u64> {
        [self.hide_at, self.anchor_at.filter(|_| self.on.is_some()), self.breath_at.filter(|_| self.on.is_some())].into_iter().flatten().min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use lumatv_tokens::Tokens;
    use serde_json::json;
    use ui::lucent::palette::Mode;

    fn themes() -> Vec<Roles> {
        let custom = Tokens::from_json(&json!({"dark": {"accent": "#FFFFFF", "info": "#FFFFC0", "live": "#FFC0FF", "ink": "#808080", "canvas": "#000000"}}));
        vec![
            *Tokens::shipped().roles(Mode::Dark),
            *Tokens::shipped().roles(Mode::Light),
            *custom.roles(Mode::Dark),
        ]
    }

    /// The pixel of the tile of `side` that is at the screen point (`x`, `y`) when the flow is `tau` along, and the share of it the
    /// window lets through there: `None` outside the strip.
    fn seen(side: Side, tiles: &[Vec<u8>; 4], x: f32, y: f32, tau: f32) -> Option<([f32; 4], f32)> {
        let (cx, cy) = side.corner();
        let b = BAND_PX as f32;
        let (w, _) = side.size();
        let (long, short, extent) = if side.horizontal() { (x, y - cy, W as f32) } else { (y, x - cx, H as f32) };
        if !(0.0..b).contains(&short) || long < -b || long > extent + b {
            return None;
        }
        let dir = if matches!(side, Side::Top | Side::Right) { 1.0 } else { -1.0 };
        let along = (long - dir * tau).rem_euclid(PERIOD as f32).floor() as u32;
        let across = short.floor() as u32;
        let (px, py) = if side.horizontal() { (along, across) } else { (across, along) };
        let i = ((py * w + px) * 4) as usize;
        let tile = &tiles[SIDES.iter().position(|s| *s == side).unwrap()];
        let weight = ((long + b) / (2.0 * b)).clamp(0.0, 1.0).min(((extent + b - long) / (2.0 * b)).clamp(0.0, 1.0));
        Some(([0, 1, 2, 3].map(|k| f32::from(tile[i + k]) / 255.0 * weight), weight))
    }

    fn baked(roles: &Roles) -> [Vec<u8>; 4] {
        let cols = columns(roles);
        SIDES.map(|side| bake(side, &cols))
    }

    #[test]
    fn no_pixel_of_the_glow_and_no_corner_where_two_bands_meet_has_more_light_than_the_cap() {
        for roles in themes() {
            let cap = ceiling(&roles);
            let tiles = baked(&roles);
            for tile in &tiles {
                assert!(tile.chunks_exact(4).all(|p| luminance([0, 1, 2].map(|i| f32::from(p[i]) / 255.0)) <= cap + 1e-6));
                assert!(tile.chunks_exact(4).all(|p| p[..3].iter().all(|c| *c <= p[3])), "premultiplied");
            }
            let mut brightest = 0.0f32;
            for tau in [0.0, 313.0, 777.0, 1201.0, 1499.0, 2200.0] {
                for (a, b, corner) in [(Side::Top, Side::Left, (0.0, 0.0)), (Side::Top, Side::Right, (W as f32 - 1.0, 0.0)), (Side::Bottom, Side::Right, (W as f32 - 1.0, H as f32 - 1.0)), (Side::Bottom, Side::Left, (0.0, H as f32 - 1.0))] {
                    for dx in 0..BAND_PX {
                        for dy in 0..BAND_PX {
                            let x = if corner.0 == 0.0 { dx as f32 } else { corner.0 - dx as f32 };
                            let y = if corner.1 == 0.0 { dy as f32 } else { corner.1 - dy as f32 };
                            let (Some((p, _)), Some((q, _))) = (seen(a, &tiles, x, y, tau), seen(b, &tiles, x, y, tau)) else { continue };
                            // one band over the other
                            let out = [0, 1, 2].map(|k| q[k] + (1.0 - q[3]) * p[k]);
                            brightest = brightest.max(luminance(out));
                        }
                    }
                }
            }
            assert!(brightest <= cap + 1e-6, "{brightest} over {cap}");
            assert!(brightest > 0.0);
        }
    }

    #[test]
    fn the_band_is_no_more_than_six_percent_of_the_height_and_falls_to_nothing() {
        assert!(BAND_PX as f32 <= 0.06 * H as f32);
        assert_eq!((falloff(0.0), falloff(1.0)), (1.0, 0.0));
        assert!((0..100).all(|i| falloff(i as f32 / 100.0) >= falloff((i + 1) as f32 / 100.0)));
        let tile = &baked(&themes()[0])[0];
        let alpha = |row: u32, col: u32| tile[((row * PERIOD + col) * 4 + 3) as usize];
        assert!(alpha(0, 0) > alpha(BAND_PX / 2, 0));
        assert_eq!(alpha(BAND_PX - 1, 0), 0, "the inner side is nothing");
    }

    #[test]
    fn the_pattern_comes_round_without_a_seam_and_runs_on_between_the_strips() {
        for roles in themes() {
            let cols = columns(&roles);
            let step = |a: [f32; 4], b: [f32; 4]| (0..4).map(|k| (a[k] - b[k]).abs()).fold(0.0, f32::max);
            assert!(step(cols[PERIOD as usize - 1], cols[0]) < 0.02, "the seam");
            let widest = cols.windows(2).map(|w| step(w[0], w[1])).fold(0.0, f32::max);
            assert!(widest < 0.02, "{widest}");
        }
        // along the perimeter: the pattern of one strip's end is the next strip's start
        assert_eq!((Side::Top.pattern(W % PERIOD - 1), Side::Right.pattern(0)), (419, 420));
        assert_eq!((Side::Right.pattern(H - 1), Side::Bottom.pattern(W % PERIOD - 1)), (1499, 1));
        assert_eq!((Side::Bottom.pattern(0), Side::Left.pattern(H - 1)), (420, 421));
        assert_eq!((Side::Left.pattern(0), Side::Top.pattern(0)), (0, 0));
    }

    #[test]
    fn it_is_fourteen_layers_ten_groups_and_three_tracks_and_shows_nothing_until_asked() {
        let (mut h, mut kit) = testkit::kit(1.0);
        let before = kit.room();
        let glow = Glow::new(&mut kit, 100).unwrap();
        let after = kit.room();
        assert_eq!(glow.counts(), (14, 10, 3));
        assert_eq!((before.layers - after.layers, before.groups - after.groups, before.tracks - after.tracks), (14, 10, 3));
        h.commit();
        assert!(!glow.visible() && !glow.flowing() && !glow.breathing());
    }

    #[test]
    fn it_flows_and_breathes_while_it_is_on_and_stops_asking_for_frames_once_it_has_faded() {
        let (_h, mut kit) = testkit::kit(1.0);
        let mut glow = Glow::new(&mut kit, 100).unwrap();
        glow.set(&kit, Some(Pace::Listening), 0).unwrap();
        assert!(glow.visible() && glow.flowing() && glow.breathing());
        glow.set(&kit, Some(Pace::Thinking), 1000).unwrap();
        assert!(glow.flowing());
        glow.set(&kit, None, 2000).unwrap();
        assert!(glow.visible(), "it fades before it goes");
        let hide = glow.due_at().unwrap();
        assert_eq!(hide, 2000 + u64::from(FADE_OUT_MS) + 50);
        glow.tick(hide).unwrap();
        assert!(!glow.visible() && !glow.flowing() && !glow.breathing());
        assert_eq!(glow.due_at(), None);
    }

    #[test]
    fn with_reduced_motion_it_only_breathes() {
        let (_h, mut kit) = testkit::kit(1.0);
        kit.ui.look.reduced_motion = true;
        let mut glow = Glow::new(&mut kit, 100).unwrap();
        glow.set(&kit, Some(Pace::Listening), 0).unwrap();
        assert!(glow.breathing() && !glow.flowing());
        assert_eq!(glow.due_at(), Some(u64::from(BREATH_REDUCED_MS)));
    }

    #[test]
    fn a_new_theme_is_a_new_glow() {
        let (mut h, mut kit) = testkit::kit(1.0);
        let first = baked(&kit.ui.look.theme.roles);
        let custom = Tokens::from_json(&json!({"dark": {"accent": "#00FF00", "info": "#0000FF", "live": "#FF0000"}}));
        kit.retheme(custom, Mode::Dark, 1.0);
        assert_ne!(baked(&kit.ui.look.theme.roles), first);
        h.commit();
    }
}
