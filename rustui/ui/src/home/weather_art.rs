//! The weather's picture: a vector illustration of the current condition that moves, drawn once and moved by the compositor.
//!
//! Every shape is a signed-distance function evaluated per pixel of a small sprite (`weather_sprites.rs`), so a sprite is exact at the size
//! it is made for and the picture is never a scaled bitmap (a soft glow is made smaller and shown larger). The motion is not drawn: each
//! sprite is an image layer, and the compositor moves it (`Request::Animate`, linear) along a closed-form path the app only restates every
//! [`TICK_MS`]: a cloud's float and the sun's breathing are sines sampled into straight segments of 0.5 s (a step of 4 % of a period at
//! most, so the corners do not show), clouds and streaks of wind cross the sky on a saw that fades them in and out, the rain and snow far
//! away are one tall sheet each whose pattern repeats every `P` pixels and which slides exactly `P` and is put back (the pictures are the
//! same), the streaks and flakes near the eye are sprites of their own, a splash, a flash and a shooting star are a raised cosine of a
//! second or so, and the sun's rays turn by one ring after another taking over from the last. Nothing is drawn, allocated or sent per
//! frame: a tick is one `Animate` per moving sprite and a commit, two and a half times a second, and when the picture is not seen (an
//! app over Home, the screensaver, standby, a Home layer over it, reduced motion) there is no tick at all and the sprites rest on whole
//! pixels.
//!
//! The picture is made for a [`Stage`] (a box and where its hero, the sun or moon with its clouds, sits and how large it is) and drawn in
//! a design square of 200 units round the hero, so the card's small sky and the view's large one are the same picture.

use super::weather_sprites::*;
use compositor_protocol::{Edges, Props, Rect};
use serde_json::Value;

/// How often the paths are restated, and how far ahead of now each animation runs, so that a tick that comes late still finds the sprite moving.
pub const TICK_MS: u64 = 400;
pub const LEAD_MS: u32 = 500;
/// The design square.
const UNITS: f32 = 200.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sky {
    Clear,
    Partly,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Storm,
    Snow,
}

/// What the picture shows: the sky, whether it is day, how much (0 light, 1 moderate, 2 heavy; for a partly cloudy sky how much cloud) and whether it is windy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cond {
    pub sky: Sky,
    pub day: bool,
    pub heavy: u8,
    pub wind: bool,
}

/// Wind at which the picture gets its streaks.
const WINDY_MPH: f64 = 18.0;

impl Cond {
    /// From the weather document (`topic.doc`): the WMO code and day flag of `current` when it is there, else the card's `display.nowKind`.
    pub fn from_doc(doc: &Value) -> Option<Cond> {
        let wind = doc.pointer("/current/windMph").and_then(Value::as_f64).is_some_and(|w| w >= WINDY_MPH);
        if let (Some(code), Some(day)) = (doc.pointer("/current/code").and_then(Value::as_i64), doc.pointer("/current/isDay").and_then(Value::as_bool)) {
            let (sky, heavy) = match code {
                0 => (Sky::Clear, 0),
                1 => (Sky::Partly, 0),
                2 => (Sky::Partly, 1),
                3 => (Sky::Cloudy, 1),
                45 | 48 => (Sky::Fog, 1),
                51 => (Sky::Drizzle, 0),
                53 | 56 => (Sky::Drizzle, 1),
                55 | 57 => (Sky::Drizzle, 2),
                61 | 80 => (Sky::Rain, 0),
                63 | 66 | 81 => (Sky::Rain, 1),
                65 | 67 | 82 => (Sky::Rain, 2),
                71 | 77 | 85 => (Sky::Snow, 0),
                73 => (Sky::Snow, 1),
                75 | 86 => (Sky::Snow, 2),
                95 => (Sky::Storm, 1),
                c if c >= 96 => (Sky::Storm, 2),
                c if (58..=94).contains(&c) => (Sky::Rain, 1),
                _ => (Sky::Cloudy, 1),
            };
            return Some(Cond { sky, day, heavy, wind });
        }
        let kind = doc.pointer("/display/nowKind").and_then(Value::as_str)?;
        let (sky, day, heavy) = match kind {
            "sun" => (Sky::Clear, true, 0),
            "moon" => (Sky::Clear, false, 0),
            "partly" => (Sky::Partly, true, 1),
            "partly-night" => (Sky::Partly, false, 1),
            "cloud" => (Sky::Cloudy, true, 1),
            "fog" => (Sky::Fog, true, 1),
            "drizzle" => (Sky::Drizzle, true, 1),
            "rain" => (Sky::Rain, true, 1),
            "storm" => (Sky::Storm, true, 1),
            "snow" => (Sky::Snow, true, 1),
            _ => return None,
        };
        // a night kind the helper has no name for: the clock of the reading decides (the document's own `isDay` is the better source, above)
        Some(Cond { sky, day, heavy, wind })
    }

    /// The sky's tint over the glass behind it, from the top of the picture (straight RGB and alpha) to nothing at its foot: the mood of the
    /// weather at a glance from across the room. Modest in both looks, so the glass still reads as glass (and an OLED panel is not lit by it).
    pub fn wash(&self, light: bool) -> ([u8; 3], f32) {
        let (rgb, a): ([u8; 3], f32) = match (self.sky, self.day, light) {
            (Sky::Clear, true, false) => ([0xF6, 0xA8, 0x6B], 0.20),
            (Sky::Clear, false, false) => ([0x4B, 0x3F, 0x9E], 0.34),
            (Sky::Partly, true, false) => ([0x8E, 0x86, 0xD6], 0.24),
            (Sky::Partly, false, false) => ([0x4B, 0x42, 0x95], 0.30),
            (Sky::Cloudy, _, false) => ([0x6E, 0x6A, 0xA6], 0.28),
            (Sky::Fog, _, false) => ([0x9A, 0x96, 0xB8], 0.20),
            (Sky::Drizzle | Sky::Rain, _, false) => ([0x3E, 0x6C, 0x9E], 0.34),
            (Sky::Storm, _, false) => ([0x2A, 0x2A, 0x6E], 0.46),
            (Sky::Snow, _, false) => ([0x8C, 0xA2, 0xD8], 0.26),
            (Sky::Clear, true, true) => ([0xF6, 0xB0, 0x5E], 0.30),
            (Sky::Clear, false, true) => ([0x90, 0x7A, 0xC9], 0.22),
            (Sky::Partly, true, true) => ([0xA6, 0xB4, 0xE6], 0.30),
            (Sky::Partly, false, true) => ([0x90, 0x88, 0xC4], 0.24),
            (Sky::Cloudy, _, true) => ([0x9C, 0x98, 0xBC], 0.28),
            (Sky::Fog, _, true) => ([0xB4, 0xB0, 0xC8], 0.30),
            (Sky::Drizzle | Sky::Rain, _, true) => ([0x6E, 0xA0, 0xC8], 0.34),
            (Sky::Storm, _, true) => ([0x6E, 0x6A, 0x96], 0.40),
            (Sky::Snow, _, true) => ([0xA6, 0xC0, 0xE2], 0.32),
        };
        (rgb, a)
    }
}

// ---- the motion
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wave {
    Zero,
    /// `amp * sin(2 pi (t / period + phase))`, in pixels (position), or as a fraction (scale, alpha).
    Sine { amp: f32, period_ms: u32, phase: f32 },
    /// `from` to `to` over each period, then again, starting `phase` of a period in: the sheets of rain and snow, the clouds and streaks that cross the sky.
    Saw { from: f32, to: f32, period_ms: u32, phase: f32 },
    /// A raised cosine of `width_ms` at `at_ms` of every `period_ms`, up to `peak`: a flash, a splash.
    Pulse { peak: f32, period_ms: u32, at_ms: u32, width_ms: u32 },
    /// `from` until `at_ms` of every `period_ms`, then straight to `to` over `width_ms` and `to` for the rest: a splash widening, a shooting star crossing.
    Ramp { from: f32, to: f32, period_ms: u32, at_ms: u32, width_ms: u32 },
    /// A tent: `peak` where the period's phase is `centre`, falling in a straight line to nothing `half` of a period either side: the rings of the sun's rays taking over from one another.
    Tent { peak: f32, period_ms: u32, centre: f32, half: f32 },
}

impl Wave {
    /// The value at `t` ms. A saw is wrapped into its period, or, with an `anchor` (the time of the tick), carried on past the period from where it is
    /// at the anchor: the sheet's pattern repeats, so a path that runs over the end is the same picture and the sprite is put back at the next tick.
    fn at(self, t: f64, anchor: Option<f64>) -> f32 {
        match self {
            Wave::Zero => 0.0,
            Wave::Sine { amp, period_ms, phase } => amp * (std::f64::consts::TAU * (t / period_ms as f64 + phase as f64)).sin() as f32,
            Wave::Saw { from, to, period_ms, phase } => {
                let p = period_ms as f64;
                let f = match anchor {
                    Some(a) => (a / p + phase as f64).fract() + (t - a) / p,
                    None => (t / p + phase as f64).fract(),
                };
                from + (to - from) * f as f32
            }
            Wave::Pulse { peak, period_ms, at_ms, width_ms } => {
                let m = (t % period_ms as f64) - at_ms as f64;
                if m <= 0.0 || m >= width_ms as f64 { 0.0 } else { peak * (0.5 - 0.5 * (std::f64::consts::TAU * m / width_ms as f64).cos()) as f32 }
            }
            Wave::Ramp { from, to, period_ms, at_ms, width_ms } => {
                let m = (t % period_ms as f64) - at_ms as f64;
                if m <= 0.0 { from } else { from + (to - from) * (m / width_ms as f64).min(1.0) as f32 }
            }
            Wave::Tent { peak, period_ms, centre, half } => {
                let f = (t / period_ms as f64).fract() as f32;
                let d = (f - centre).abs();
                let d = d.min(1.0 - d);
                peak * (1.0 - d / half).max(0.0)
            }
        }
    }

    /// Where the wave rests when nothing moves.
    fn rest(self) -> f32 {
        match self {
            // inside the stretch where a fading saw is fully there, and apart from the others by their phases
            Wave::Saw { from, to, phase, .. } => from + (to - from) * (0.25 + 0.5 * (0.35 + phase).fract()),
            _ => 0.0,
        }
    }

    fn period(self) -> Option<u32> {
        match self {
            Wave::Saw { period_ms, .. } => Some(period_ms),
            _ => None,
        }
    }

    fn phase(self) -> f64 {
        match self {
            Wave::Saw { phase, .. } => phase as f64,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub x: Wave,
    pub y: Wave,
    /// Added to 1 (and multiplied by the part's own scale).
    pub scale: Wave,
    /// Added to the base alpha.
    pub alpha: Wave,
    /// Fades the sprite in and out over each period of its saw (a cloud crossing the sky, a streak of wind), so it is never seen to start or be put back.
    pub bump: bool,
}

impl Motion {
    pub const STILL: Motion = Motion { x: Wave::Zero, y: Wave::Zero, scale: Wave::Zero, alpha: Wave::Zero, bump: false };
}

pub struct Part {
    pub raster: Raster,
    /// The sprite's top left in the box, on whole pixels.
    pub at: (f32, f32),
    pub alpha: f32,
    /// The alpha it has at rest, when that is not `alpha` (the bolt shows dimly in a still picture, a splash not at all).
    pub rest_alpha: f32,
    /// How many times larger than its pixels it is shown (a glow made at a quarter of its size is shown four times as large).
    pub scale0: f32,
    pub motion: Motion,
    /// Only this rectangle of the box (box pixels) is drawn, fading out over the edges' widths.
    pub clip: Option<(Rect, Edges)>,
}

/// Where a part is: position (relative to the box), scale and alpha.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub alpha: f32,
}

impl Part {
    /// The pose at `t` ms; with an `anchor` a saw runs on past its period from where it is at the anchor (see [`Wave::at`]).
    pub fn pose(&self, t: f64, anchor: Option<f64>) -> Pose {
        let m = &self.motion;
        let mut alpha = (self.alpha + m.alpha.at(t, anchor)).clamp(0.0, 1.0);
        if m.bump
            && let Some(p) = m.x.period().or(m.y.period())
        {
            let ph = if m.x.period().is_some() { m.x.phase() } else { m.y.phase() };
            let f = (t / p as f64 + ph).fract() as f32;
            alpha *= smooth(0.0, 0.2, f) * (1.0 - smooth(0.8, 1.0, f));
        }
        Pose { x: self.at.0 + m.x.at(t, anchor), y: self.at.1 + m.y.at(t, anchor), scale: self.scale0 * (1.0 + m.scale.at(t, anchor)), alpha }
    }

    pub fn rest_pose(&self) -> Pose {
        let m = &self.motion;
        Pose { x: (self.at.0 + m.x.rest()).round(), y: (self.at.1 + m.y.rest()).round(), scale: self.scale0, alpha: if m.bump { self.alpha } else { self.rest_alpha } }
    }

    /// Whether a period of this part's saw ended between the ticks at `before` and `now`: the sprite is put back.
    pub fn wrapped(&self, before: u64, now: u64) -> bool {
        let m = &self.motion;
        match m.x.period().or(m.y.period()) {
            Some(p) => {
                let ph = if m.x.period().is_some() { m.x.phase() } else { m.y.phase() };
                let cycle = |t: u64| (t as f64 / p as f64 + ph).floor() as i64;
                cycle(now) != cycle(before)
            }
            None => false,
        }
    }

    pub fn props(&self, p: &Pose, origin: (f32, f32)) -> Props {
        let mut props = Props { opacity: p.alpha, scale: p.scale, ..Props::at(origin.0 + p.x, origin.1 + p.y) };
        if let Some((r, edges)) = self.clip {
            props.clip = Some(Rect { x: origin.0 + r.x, y: origin.1 + r.y, w: r.w, h: r.h });
            props.feather = edges;
        }
        props
    }
}

/// The box a picture is made for, in pixels, and where in it the hero sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub w: f32,
    pub h: f32,
    /// The hero's centre, and the side of the 200-unit square it is designed in.
    pub hero: (f32, f32),
    pub size: f32,
    /// Where rain lands (box y).
    pub ground: f32,
    /// How far in from the box's left, right, top and bottom edges what crosses the sky fades out.
    pub fade: [f32; 4],
    /// How much of the width of the window of rain and snow, from its left, they fade in over: they are shy of the words that are on that side.
    pub shy: f32,
}

impl Stage {
    /// A square box with the hero in the middle (the examples and the first card's picture).
    pub fn square(side: f32) -> Stage {
        Stage { w: side, h: side, hero: (side / 2.0, side / 2.0), size: side, ground: (side * 0.94).round(), fade: [side * 0.07, side * 0.07, 0.0, side * 0.11], shy: 0.22 }
    }

    fn rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.w, h: self.h }
    }

    fn edges(&self) -> Edges {
        Edges::new(self.fade[0], self.fade[1], self.fade[2], self.fade[3])
    }
}

// ---- the picture of a condition
struct Builder {
    c: Ctx,
    st: Stage,
    out: Vec<Part>,
    /// Where the hero's design square starts in the box.
    origin: (f32, f32),
}

/// What crosses the sky: how far away it is (0 far, 1 in the middle, 2 near), where (a fraction of the box's height), how large, its shape, and
/// how far round its crossing it is at the start (a fraction).
struct Cross {
    depth: u8,
    y: f32,
    scale: f32,
    kind: u8,
    phase: f32,
}

/// A period in whole ticks, so that a saw is put back on a tick.
fn whole_ticks(ms: f32) -> u32 {
    ((ms / TICK_MS as f32).round().max(2.0) as u32) * TICK_MS as u32
}

impl Builder {
    fn new(st: Stage, light: bool) -> Builder {
        let k = st.size / UNITS;
        Builder { c: Ctx { light, k, pal: Pal::of(light) }, st, out: Vec::new(), origin: (st.hero.0 - 100.0 * k, st.hero.1 - 100.0 * k) }
    }

    fn k(&self) -> f32 {
        self.c.k
    }

    /// A point of the hero's design square, in box pixels.
    fn u(&self, ux: f32, uy: f32) -> (f32, f32) {
        (self.origin.0 + ux * self.c.k, self.origin.1 + uy * self.c.k)
    }

    /// A sine in pixels (`amp_u` units).
    fn sine(&self, amp_u: f32, period_s: f32, phase: f32) -> Wave {
        Wave::Sine { amp: amp_u * self.c.k, period_ms: (period_s * 1000.0) as u32, phase }
    }

    /// A sine of a fraction (scale, alpha).
    fn frac(&self, amp: f32, period_s: f32, phase: f32) -> Wave {
        Wave::Sine { amp, period_ms: (period_s * 1000.0) as u32, phase }
    }

    fn float(&self, ax: f32, px: f32, ph: f32, ay: f32, py: f32) -> Motion {
        Motion { x: self.sine(ax, px, ph), y: self.sine(ay, py, ph + 0.3), ..Motion::STILL }
    }

    /// The whole hero drifts a few pixels over minutes, so that nothing bright sits on the same pixels.
    fn drift(&self, px: f32, py: f32) -> Motion {
        Motion { x: Wave::Sine { amp: px * self.c.k, period_ms: 41_000, phase: 0.1 }, y: Wave::Sine { amp: py * self.c.k, period_ms: 53_000, phase: 0.6 }, ..Motion::STILL }
    }

    /// The top left, on whole pixels, of a sprite of `size` centred on `centre` (box pixels).
    fn centred(&self, centre: (f32, f32), r: &Raster) -> (f32, f32) {
        ((centre.0 - r.w as f32 / 2.0).round(), (centre.1 - r.h as f32 / 2.0).round())
    }

    /// A sprite centred on a point of the hero's square.
    fn add_u(&mut self, raster: Raster, at_u: (f32, f32), alpha: f32, motion: Motion) {
        let centre = self.u(at_u.0, at_u.1);
        self.add(raster, centre, alpha, 1.0, motion);
    }

    /// A sprite centred on a point of the box, shown `scale0` times as large as its pixels.
    fn add(&mut self, raster: Raster, centre: (f32, f32), alpha: f32, scale0: f32, motion: Motion) {
        let at = self.centred(centre, &raster);
        self.out.push(Part { raster, at, alpha, rest_alpha: alpha, scale0, motion, clip: None });
    }

    /// A sprite that is only seen inside the stage (and fades out towards its edges).
    #[allow(clippy::too_many_arguments)]
    fn add_clipped(&mut self, raster: Raster, at: (f32, f32), alpha: f32, rest_alpha: f32, scale0: f32, motion: Motion, clip: (Rect, Edges)) {
        self.out.push(Part { raster, at, alpha, rest_alpha, scale0, motion, clip: Some(clip) });
    }

    fn whole_stage(&self) -> (Rect, Edges) {
        (self.st.rect(), self.st.edges())
    }

    // ---- the sun and the moon
    fn sun(&mut self, at_u: (f32, f32), size: f32) {
        let (pal, light) = (self.c.pal, self.c.light);
        // the bloom: a wide warm glow that breathes, made at a quarter of its size and shown four times as large
        let low = self.c.low(4.0);
        let g = glow(&low, 300.0 * size, pal.bloom, if light { 0.20 } else { 0.30 });
        let m = Motion { scale: self.frac(0.06, 13.0, 0.0), alpha: self.frac(0.16, 13.0, 0.4), ..self.drift(3.0 * size, 2.5 * size) };
        let centre = self.u(at_u.0, at_u.1);
        self.add(g, centre, 0.9, 4.0, m);
        let g = glow(&low, 215.0 * size, pal.sun_glow, 0.55);
        let m = Motion { scale: self.frac(0.05, 9.0, 0.0), alpha: self.frac(0.12, 9.0, 0.0), ..self.drift(3.0 * size, 2.5 * size) };
        self.add(g, centre, 0.9, 4.0, m);
        // the rays: a ring of twelve, made in RINGS turns across a sixth of a revolution (the pattern repeats every sixth), one fading in as the
        // one before it fades out, so that the ring is seen to turn all the time: a sixth of a revolution every `RING_S` seconds
        // (a ring that is large is made at half its size and shown twice as large)
        let div = if self.c.px(2.0 * 84.0 * size + 12.0) > 230.0 { 2.0 } else { 1.0 };
        let ring = self.c.low(div);
        for j in 0..RINGS {
            let turn = std::f32::consts::PI / 3.0 * j as f32 / RINGS as f32;
            let r = sun_rays(&ring, turn, size, pal.sun, 0.9);
            let m = Motion { alpha: Wave::Tent { peak: 0.82, period_ms: RING_S * 1000, centre: j as f32 / RINGS as f32, half: 1.0 / RINGS as f32 }, ..self.drift(3.0 * size, 2.5 * size) };
            let at = self.centred(centre, &r);
            self.out.push(Part { raster: r, at, alpha: 0.0, rest_alpha: if j == 0 { 0.82 } else { 0.0 }, scale0: div, motion: m, clip: None });
        }
        let d = disc(&self.c, 34.0 * size, pal.sun_hi, pal.sun);
        let m = Motion { scale: self.frac(0.012, 7.0, 0.0), ..self.drift(3.0 * size, 2.5 * size) };
        self.add(d, centre, 1.0, 1.0, m);
    }

    fn moon(&mut self, at_u: (f32, f32), size: f32) {
        let (pal, light) = (self.c.pal, self.c.light);
        let low = self.c.low(4.0);
        let centre = self.u(at_u.0, at_u.1);
        let g = glow(&low, 250.0 * size, pal.moon_glow, if light { 0.20 } else { 0.24 });
        let m = Motion { scale: self.frac(0.05, 11.0, 0.0), alpha: self.frac(0.14, 11.0, 0.0), ..self.drift(2.5 * size, 2.0 * size) };
        self.add(g, centre, 0.9, 4.0, m);
        let g = glow(&low, 150.0 * size, pal.moon_glow, if light { 0.2 } else { 0.28 });
        let m = Motion { scale: self.frac(0.04, 7.0, 0.3), alpha: self.frac(0.1, 7.0, 0.3), ..self.drift(2.5 * size, 2.0 * size) };
        self.add(g, centre, 0.9, 4.0, m);
        let cr = crescent(&self.c, 38.0 * size);
        self.add(cr, centre, 1.0, 1.0, self.drift(2.5 * size, 2.0 * size));
    }

    /// A night sky: stars that twinkle each in its own time, and now and then a shooting star.
    fn night_sky(&mut self, shooting: bool, many: usize) {
        let (w, h, k) = (self.st.w, self.st.h, self.k());
        let hero = self.st.hero;
        let reach = 0.58 * self.st.size;
        let mut placed: Vec<(f32, f32)> = Vec::new();
        let mut i = 0u32;
        while placed.len() < many && i < 400 {
            i += 1;
            let (x, y) = (hash(900 + i * 3) * w, hash(901 + i * 3) * h * 0.9);
            // not on the moon, not on the star before, and thinner over the left (where the temperature is written)
            let thin = x < w * 0.38 && hash(902 + i * 3) < 0.55;
            if thin || (x - hero.0).hypot(y - hero.1) < reach || placed.iter().any(|p| (p.0 - x).hypot(p.1 - y) < 26.0 * k) {
                continue;
            }
            placed.push((x, y));
        }
        for (n, (x, y)) in placed.into_iter().enumerate() {
            let r_px = (2.1 + 3.6 * hash(950 + n as u32 * 5).powi(2)) * k.max(0.9);
            let s = star(&self.c, r_px / k);
            let period = 3.0 + 5.2 * hash(960 + n as u32 * 5);
            let ph = hash(970 + n as u32 * 5);
            let m = Motion { alpha: self.frac(0.42, period, ph), scale: self.frac(0.1, period, ph), ..Motion::STILL };
            let r = self.whole_stage();
            let at = self.centred((x, y), &s);
            self.add_clipped(s, at, 0.7, 0.7, 1.0, m, r);
        }
        if shooting {
            for (n, (period_s, at_ms, from, angle)) in [(14.0f32, 5_200u32, (0.12f32, 0.14f32), 0.38f32), (23.2, 15_600, (0.5, 0.06), 0.45)].into_iter().enumerate() {
                let len = 46.0 + 8.0 * n as f32;
                let s = shooting_star(&self.c, len, angle, self.c.pal.moon_hi);
                let (sx, sy) = (from.0 * self.st.w, from.1 * self.st.h);
                let (dx, dy) = ((len * 2.0) * k * angle.cos(), (len * 2.0) * k * angle.sin());
                let period = whole_ticks(period_s * 1000.0);
                let m = Motion {
                    x: Wave::Ramp { from: 0.0, to: dx, period_ms: period, at_ms, width_ms: 1_200 },
                    y: Wave::Ramp { from: 0.0, to: dy, period_ms: period, at_ms, width_ms: 1_200 },
                    alpha: Wave::Pulse { peak: 0.95, period_ms: period, at_ms, width_ms: 1_200 },
                    ..Motion::STILL
                };
                self.add_clipped(s, (sx.round(), sy.round()), 0.0, 0.0, 1.0, m, (self.st.rect(), Edges::NONE));
            }
        }
    }

    // ---- clouds
    /// The clouds that cross the sky, nearest last: far ones are small, pale, soft and slow, near ones large, bright and quick, and the
    /// ones in the middle between. Each is a sprite that crosses the box on a saw and fades in and out at its ends.
    fn cross(&mut self, list: &[Cross], stormy: bool, night: bool) {
        let pal = self.c.pal;
        let k = self.k();
        let (hi, lo, rim) = if stormy { (pal.storm_hi, pal.storm_lo, pal.cloud_rim * 0.5) } else { (pal.cloud_hi, pal.cloud_lo, pal.cloud_rim) };
        // a night's clouds are dimmer
        let dim = if night { 0.78 } else { 1.0 };
        for cl in list {
            let (div, alpha, speed) = match cl.depth {
                0 => (2.0f32, 0.40, 5.0f32),
                1 => (1.0, 0.66, 9.0),
                _ => (1.0, 0.9, 15.0),
            };
            let (chi, clo) = match cl.depth {
                0 => (mix3(hi, lo, 0.6), mix3(lo, [0.0; 3], 0.1)),
                1 => (mix3(hi, lo, 0.25), lo),
                _ => (hi, lo),
            };
            let ctx = self.c.low(div);
            let r = cloud(&ctx, cl.kind, cl.scale, chi, clo, rim);
            let cw = r.w as f32 * div;
            let travel = self.st.w + cw;
            let period = whole_ticks(travel / (speed * k) * 1000.0);
            let m = Motion { x: Wave::Saw { from: 0.0, to: travel, period_ms: period, phase: cl.phase }, bump: true, ..Motion::STILL };
            // the sprite is `r.w` wide and shown `div` times as large about its centre, which starts a cloud's width left of the box and ends as far right of it
            let at = ((-(cw + r.w as f32) / 2.0).round(), (cl.y * self.st.h - r.h as f32 / 2.0).round());
            let clip = self.whole_stage();
            self.add_clipped(r, at, alpha * dim, alpha * dim, div, m, clip);
        }
    }

    // ---- wind
    /// Streamlines of wind crossing the box, a few of them, long and thin, each bobbing a little as it goes.
    fn wind_lines(&mut self, many: usize) {
        let colour = self.c.pal.wind;
        for i in 0..many {
            let len = (92.0 + 62.0 * hash(1100 + i as u32 * 3)) * 1.0;
            let curl = 6.0 + 5.0 * hash(1101 + i as u32 * 3);
            let r = wind(&self.c, len, curl, 3.0 + 3.0 * hash(1102 + i as u32 * 3), colour);
            let travel = self.st.w + r.w as f32;
            let secs = 4.4 + 3.6 * hash(1103 + i as u32 * 3) + (many - i) as f32 * 0.2;
            let period = whole_ticks(secs * 1000.0);
            let y = self.st.h * (0.1 + 0.8 * (i as f32 + 0.5 * hash(1104 + i as u32)) / many as f32);
            let m = Motion {
                x: Wave::Saw { from: 0.0, to: travel, period_ms: period, phase: hash(1105 + i as u32 * 3) },
                y: self.sine(3.5, 3.1 + 1.7 * hash(1106 + i as u32), hash(1107 + i as u32)),
                bump: true,
                ..Motion::STILL
            };
            let clip = self.whole_stage();
            let at = (-(r.w as f32), (y - r.h as f32 / 2.0).round());
            self.add_clipped(r, at, 0.55, 0.55, 1.0, m, clip);
        }
    }

    // ---- rain and snow
    /// The window the rain or snow falls in: under the hero, as wide as `reach` hero sizes either side of it, from its middle to the ground.
    fn window(&self, reach: f32) -> Rect {
        let (hx, hy) = self.st.hero;
        // however large the picture, the curtain of rain is no wider than 760 px (its sheets are as wide as it is)
        let reach = reach.min(380.0 / self.st.size);
        let (x0, x1) = ((hx - reach * self.st.size).max(0.0).round(), (hx + reach * self.st.size).min(self.st.w).round());
        let y0 = (hy - 0.1 * self.st.size).max(0.0).round();
        Rect { x: x0, y: y0, w: x1 - x0, h: (self.st.ground - y0).max(8.0).round() }
    }

    /// A sheet of far-off precipitation that slides one period in `period_ms` and is put back (the pattern repeats).
    fn sheet(&mut self, win: Rect, mut s: Sheet, period_ms: u32, alpha: f32, sway: Wave, edges: Edges) {
        // the sheet covers the window at every offset of the slide: the window, a period more, and what the lead of an animation runs on
        let travel = s.period * (1.0 + LEAD_MS as f32 / period_ms as f32);
        s.height = (win.h + travel).ceil() as u32 + 2;
        let base_y = win.y - (s.height as f32 - win.h);
        let r = sheet(&s);
        let motion = Motion { y: Wave::Saw { from: 0.0, to: s.period, period_ms, phase: 0.0 }, x: sway, ..Motion::STILL };
        self.add_clipped(r, (win.x, base_y), alpha, alpha, 1.0, motion, (win, edges));
    }

    /// Streaks of rain near the eye: sprites that fall the window's height on a saw of their own and fade in and out at its ends.
    fn near_rain(&mut self, win: Rect, many: usize, alpha: f32, lean: f32, seed: u32) {
        let colour = self.c.pal.rain;
        for i in 0..many {
            let len = 20.0 + 9.0 * hash(seed + i as u32 * 3);
            let r = streak(&self.c, len, 1.9 + 0.5 * hash(seed + 1 + i as u32 * 3), lean * len / 20.0, colour);
            let fall = win.h + r.h as f32;
            let x = win.x + win.w * (i as f32 + 0.1 + 0.8 * hash(seed + 2 + i as u32 * 3)) / many as f32;
            let period = if hash(seed + 50 + i as u32) < 0.5 { 800 } else { 1_200 };
            let m = Motion { y: Wave::Saw { from: 0.0, to: fall, period_ms: period, phase: hash(seed + 60 + i as u32 * 3) }, bump: true, ..Motion::STILL };
            let edges = Edges::new(win.w * self.st.shy, win.w * 0.04, 0.0, 0.0);
            self.add_clipped(r, (x.round(), win.y - 4.0), alpha, alpha, 1.0, m, (win, edges));
        }
    }

    /// The rings drops make where they land: they widen and fade where the rain meets the ground.
    fn splashes(&mut self, win: Rect, many: usize, alpha: f32) {
        let colour = self.c.pal.rain;
        for i in 0..many {
            let wu = 15.0 + 9.0 * hash(1300 + i as u32 * 3);
            let r = ripple(&self.c, wu, colour);
            let period = 2_000 + 400 * (hash(1301 + i as u32 * 3) * 6.0) as u32;
            let at_ms = (hash(1302 + i as u32 * 3) * period as f32 / 400.0) as u32 * 400;
            let m = Motion {
                scale: Wave::Ramp { from: -0.55, to: 0.0, period_ms: period, at_ms, width_ms: 1_200 },
                alpha: Wave::Pulse { peak: alpha, period_ms: period, at_ms, width_ms: 1_200 },
                ..Motion::STILL
            };
            let x = win.x + win.w * (0.06 + 0.88 * (i as f32 + hash(1303 + i as u32 * 3)) / many as f32);
            let y = self.st.ground - r.h as f32 * 0.5 - hash(1304 + i as u32) * 2.0 * self.k();
            let at = self.centred((x, y), &r);
            self.add_clipped(r, at, 0.0, 0.0, 1.0, m, (self.st.rect(), Edges::NONE));
        }
    }

    /// Flakes near the eye: each its own size, softer the nearer it is, falling on a saw of its own and swaying from side to side.
    fn near_snow(&mut self, win: Rect, many: usize, alpha: f32, seed: u32) {
        let k = self.k();
        let colour = self.c.pal.snow;
        for i in 0..many {
            let big = hash(seed + i as u32 * 3);
            let r_u = 2.4 + 4.6 * big;
            let soft = if big > 0.55 { 0.45 * r_u } else { 0.0 };
            let r = flake_sprite(&self.c, r_u, soft, colour);
            let fall = win.h + r.h as f32;
            let x = win.x + win.w * (i as f32 + 0.1 + 0.8 * hash(seed + 2 + i as u32 * 3)) / many as f32;
            let secs = (win.h + r.h as f32) / ((60.0 + 28.0 * big) * k);
            let period = whole_ticks(secs * 1000.0);
            let m = Motion {
                y: Wave::Saw { from: 0.0, to: fall, period_ms: period, phase: hash(seed + 60 + i as u32 * 3) },
                x: self.sine(6.0 + 9.0 * big, 3.6 + 3.0 * hash(seed + 70 + i as u32), hash(seed + 80 + i as u32)),
                bump: true,
                ..Motion::STILL
            };
            let edges = Edges::new(win.w * self.st.shy, win.w * 0.1, 0.0, 6.0 * k);
            let at = (x.round(), (win.y - r.h as f32 * 0.5).round());
            self.add_clipped(r, at, alpha * (0.7 + 0.3 * big), alpha * (0.7 + 0.3 * big), 1.0, m, (win, edges));
        }
    }
}

/// The sun's rays are made in this many turns across a sixth of a revolution (the ring of twelve repeats every sixth) and take over from
/// one another in turn; all of them in `RING_S` seconds, so the rays turn once in six times that, 2.4 minutes.
const RINGS: usize = 10;
const RING_S: u32 = 24;

/// Builds the parts of `cond`, back to front, for `stage`. `light` is the light look.
pub fn parts(cond: Cond, st: Stage, light: bool) -> Vec<Part> {
    let mut b = Builder::new(st, light);
    let pal = b.c.pal;
    let k = b.k();
    // many things of the sky scale with the box: a stage as wide as the card's is a quarter of a view's
    let wide = (st.w / (UNITS * k)).max(1.0);
    let few = |card: usize| ((card as f32) * wide.sqrt()).round() as usize;
    let storm = matches!(cond.sky, Sky::Storm) || (matches!(cond.sky, Sky::Rain) && cond.heavy == 2);
    let (ch, cl) = if storm { (pal.storm_hi, pal.storm_lo) } else { (pal.cloud_hi, pal.cloud_lo) };
    let rim = pal.cloud_rim;
    let night = !cond.day;

    if cond.wind {
        // behind everything: the streaks show in the clear sky between the clouds
        b.wind_lines(few(4).max(3) - usize::from(cond.sky == Sky::Storm));
    }
    match cond.sky {
        Sky::Clear if cond.day => b.sun((100.0, 100.0), 1.0),
        Sky::Clear => {
            b.night_sky(true, few(11));
            b.moon((100.0, 100.0), 1.0);
        }
        Sky::Partly => {
            if cond.day {
                b.sun((72.0, 70.0), 0.72);
            } else {
                b.night_sky(cond.heavy == 0, few(8));
                b.moon((72.0, 70.0), 0.72);
            }
            // high thin clouds crossing in front of the sun and moon, behind the clouds near it
            let mut list = vec![Cross { depth: 0, y: 0.2, scale: 0.9, kind: 1, phase: 0.1 }, Cross { depth: 0, y: 0.52, scale: 0.7, kind: 2, phase: 0.6 }];
            if cond.heavy >= 1 {
                list.push(Cross { depth: 1, y: 0.36, scale: 0.7, kind: 0, phase: 0.35 });
            }
            b.cross(&list, false, night);
            if cond.heavy >= 1 {
                let r = cloud(&b.c, 0, 0.62, ch, cl, rim);
                let m = b.float(7.0, 37.0, 0.4, 1.5, 11.0);
                b.add_u(r, (156.0, 78.0), 0.85, m);
            }
            let s = if cond.heavy == 0 { 0.82 } else { 1.0 };
            let r = cloud(&b.c, 0, s, ch, cl, rim);
            let m = b.float(9.0, 31.0, 0.0, 2.0, 13.0);
            b.add_u(r, (112.0, 124.0), 1.0, m);
            if cond.heavy >= 1 {
                b.cross(&[Cross { depth: 2, y: 0.72, scale: 0.8, kind: 1, phase: 0.8 }], false, night);
            }
        }
        Sky::Cloudy => {
            if night {
                b.night_sky(false, few(4));
            }
            b.cross(&[Cross { depth: 0, y: 0.18, scale: 0.9, kind: 1, phase: 0.05 }, Cross { depth: 0, y: 0.46, scale: 0.8, kind: 0, phase: 0.45 }, Cross { depth: 0, y: 0.76, scale: 0.7, kind: 2, phase: 0.75 }, Cross { depth: 1, y: 0.3, scale: 0.85, kind: 1, phase: 0.2 }, Cross { depth: 1, y: 0.62, scale: 0.7, kind: 0, phase: 0.6 }], false, night);
            let r = cloud(&b.c, 0, 1.0, pal.storm_hi, pal.storm_lo, rim * 0.6);
            let m = b.float(12.0, 43.0, 0.5, 1.5, 17.0);
            b.add_u(r, (62.0, 80.0), 0.78, m);
            let r = cloud(&b.c, 0, 1.15, ch, cl, rim);
            let m = b.float(-10.0, 34.0, 0.2, 2.0, 12.0);
            b.add_u(r, (134.0, 98.0), 0.92, m);
            let r = cloud(&b.c, 0, 1.28, ch, cl, rim);
            let m = b.float(14.0, 27.0, 0.0, 2.0, 10.0);
            b.add_u(r, (94.0, 136.0), 1.0, m);
            b.cross(&[Cross { depth: 2, y: 0.74, scale: 0.8, kind: 1, phase: 0.5 }], false, night);
        }
        Sky::Fog => {
            b.cross(&[Cross { depth: 0, y: 0.16, scale: 0.9, kind: 1, phase: 0.2 }, Cross { depth: 0, y: 0.4, scale: 0.7, kind: 2, phase: 0.7 }], true, night);
            let r = cloud(&b.c, 0, 0.9, pal.storm_hi, pal.storm_lo, rim * 0.5);
            let m = b.float(8.0, 39.0, 0.0, 1.5, 15.0);
            b.add_u(r, (100.0, 58.0), 0.7, m);
            // bands of haze, as wide as the box, drifting to and fro and thickening and thinning
            let div = if k > 1.4 { 2.0 } else { 1.0 };
            let low = b.c.low(div);
            for i in 0..5usize {
                let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                let secs = 15.0 + 3.0 * hash(1400 + i as u32);
                let w_u = b.st.w / k * (0.8 + 0.3 * hash(1401 + i as u32));
                let r = haze(&low, w_u, 24.0 + 16.0 * hash(1402 + i as u32), pal.fog);
                let y = b.st.h * (0.36 + 0.56 * i as f32 / 4.0);
                let a = 0.30 + 0.16 * hash(1403 + i as u32);
                let m = Motion { x: b.sine(side * (22.0 + 12.0 * hash(1404 + i as u32)) * wide.sqrt(), secs, 0.2 * i as f32), alpha: b.frac(0.18, secs * 0.6, 0.3 * i as f32), ..Motion::STILL };
                let centre = (b.st.w * (0.5 + 0.06 * side), y);
                let at = b.centred(centre, &r);
                let clip = b.whole_stage();
                b.add_clipped(r, at, a, a, div, m, clip);
            }
        }
        Sky::Drizzle | Sky::Rain | Sky::Storm | Sky::Snow => {
            let snow = cond.sky == Sky::Snow;
            let (cy, scale) = match cond.sky {
                Sky::Drizzle => (62.0, 1.05),
                Sky::Snow => (60.0, 1.1),
                _ => (60.0, 1.2),
            };
            if night && !storm {
                b.night_sky(false, few(3));
            }
            // the sky behind: far clouds crossing, and the flash
            let far = [Cross { depth: 0, y: 0.14, scale: 0.9, kind: 1, phase: 0.1 }, Cross { depth: 0, y: 0.34, scale: 0.8, kind: 0, phase: 0.55 }, Cross { depth: 1, y: 0.22, scale: 0.75, kind: 1, phase: 0.8 }];
            b.cross(&far, !snow, night);
            if cond.sky == Sky::Storm {
                // the flash lights the sky behind everything: a wide soft glow, then the bolts between the clouds and the rain
                let low = b.c.low(4.0);
                let g = glow_ellipse(&low, 1.7 * b.st.w / k, 1.5 * b.st.h / k, pal.flash, 0.42);
                let (period, at) = if cond.heavy == 2 { (6_000, 1_200) } else { (8_000, 1_600) };
                let centre = (b.st.w * 0.55, b.st.h * 0.35);
                b.add(g, centre, 0.0, 4.0, Motion { alpha: Wave::Pulse { peak: 0.9, period_ms: period, at_ms: at, width_ms: 1_600 }, ..Motion::STILL });
            }
            if storm {
                let r = cloud(&b.c, 0, 0.9, pal.storm_hi, pal.storm_lo, rim * 0.5);
                let m = b.float(-8.0, 36.0, 0.5, 1.5, 12.0);
                b.add_u(r, (142.0, 52.0), 0.85, m);
            } else if cond.heavy >= 1 && !snow {
                let r = cloud(&b.c, 0, 0.8, ch, cl, rim);
                let m = b.float(-8.0, 36.0, 0.5, 1.5, 12.0);
                b.add_u(r, (150.0, 54.0), 0.8, m);
            }
            // what falls, behind the cloud it falls from
            let win = b.window(if snow { 1.25 } else { 0.9 });
            let h = cond.heavy as usize;
            let edges = Edges::new(win.w * st.shy, win.w * 0.06, 0.0, 0.0);
            let wpx = win.w;
            if snow {
                // streaks per pixel of the window: the bigger the picture, the more of them (their size grows to a point, their number with its width)
                let n = |per_px: f32| ((wpx / (per_px * k.min(1.2))).round() as usize).max(4);
                // the pattern of a sheet repeats over this many pixels; a sheet is as tall as the window and a period and what it slides in a lead, so a very large picture's sheets stay in a size
                let ks = k.min(1.5);
                let far_n = [n(15.0), n(12.0), n(9.0)][h];
                let mid_n = [n(34.0), n(26.0), n(19.0)][h];
                let sw = |b: &Builder, amp: f32, secs: f32| b.sine(amp, secs, 0.0);
                let sway = sw(&b, 4.0, 7.3);
                b.sheet(win, Sheet { width: wpx.ceil() as u32 + 16, height: 0, period: (40.0 * ks).round(), streaks: far_n, len: (1.5 * k.max(0.9), 2.9 * k.max(0.9)), thick: 0.8, lean: 0.0, colour: pal.snow, seed: 29, snow: true, soft: 0.0 }, 1_400, 0.55, sway, edges);
                let sway = sw(&b, 7.0, 6.1);
                b.sheet(win, Sheet { width: wpx.ceil() as u32 + 16, height: 0, period: (56.0 * ks).round(), streaks: mid_n, len: (3.4 * k.max(0.9), 6.4 * k.max(0.9)), thick: 1.1 * k.max(0.8), lean: 0.0, colour: pal.snow, seed: 11, snow: true, soft: 0.0 }, 1_600, 0.8, sway, edges);
                b.near_snow(win, [few(4), few(6), few(8)][h], 0.85, 40);
            } else {
                let n = |per_px: f32| ((wpx / (per_px * k.min(1.2))).round() as usize).max(4);
                let ks = k.min(1.5);
                let drizzle = cond.sky == Sky::Drizzle;
                let far_n = [n(11.0), n(9.0), n(7.0)][h];
                let mid_n = [n(22.0), n(17.0), n(13.0)][h];
                let (fa, ma) = if drizzle { (0.34, 0.5) } else { ([0.36, 0.42, 0.48][h], [0.52, 0.62, 0.72][h]) };
                let lean = if cond.wind { 7.0 } else { 4.0 };
                b.sheet(win, Sheet { width: wpx.ceil() as u32 + 16, height: 0, period: (48.0 * ks).round(), streaks: far_n, len: (7.0 * k, 11.0 * k), thick: 0.95 * k.max(0.85), lean, colour: pal.rain, seed: 5, snow: false, soft: 0.0 }, 340, fa, Wave::Zero, edges);
                b.sheet(win, Sheet { width: wpx.ceil() as u32 + 16, height: 0, period: (70.0 * ks).round(), streaks: mid_n, len: (12.0 * k, 17.0 * k), thick: 1.4 * k.max(0.9), lean, colour: pal.rain, seed: 17, snow: false, soft: 0.0 }, 260, ma, Wave::Zero, edges);
                let near_n = if drizzle { few(2) } else { [few(3), few(5), few(7)][h] };
                b.near_rain(win, near_n, if drizzle { 0.55 } else { 0.9 }, lean, 70);
                b.splashes(win, if drizzle { few(3) } else { [few(4), few(6), few(8)][h] }, if drizzle { 0.34 } else { 0.5 });
            }
            if cond.sky == Sky::Storm {
                // the bolts are seen between the clouds and the rain, one place and then another
                let (period, at) = if cond.heavy == 2 { (6_000u32, 1_200u32) } else { (8_000, 1_600) };
                let bo = bolt(&b.c, 62.0, false);
                b.add_u(bo, (104.0, 112.0), 0.0, Motion { alpha: Wave::Pulse { peak: 1.0, period_ms: period, at_ms: at + 200, width_ms: 1_200 }, ..Motion::STILL });
                let last = b.out.len() - 1;
                b.out[last].rest_alpha = 0.72;
                let bo = bolt(&b.c, 48.0, true);
                b.add_u(bo, (150.0, 118.0), 0.0, Motion { alpha: Wave::Pulse { peak: 0.9, period_ms: period * 2 + 400, at_ms: at + period + 200, width_ms: 1_200 }, ..Motion::STILL });
            }
            let r = cloud(&b.c, 0, scale, ch, cl, rim * if storm { 0.5 } else { 1.0 });
            let m = b.float(10.0, 29.0, 0.0, 2.0, 11.0);
            b.add_u(r, (94.0, cy), 1.0, m);
        }
    }
    b.out
}

/// Whether anything in `parts` moves.
pub fn moves(parts: &[Part]) -> bool {
    parts.iter().any(|p| p.motion != Motion::STILL)
}

/// The pixels a part draws over the box at `pose`: the sprite's rectangle, clipped. Not used by the shell.
fn extent(part: &Part, pose: &Pose) -> (f32, f32, f32, f32) {
    let (sw, sh) = (part.raster.w as f32 * pose.scale, part.raster.h as f32 * pose.scale);
    let (cx, cy) = (pose.x + part.raster.w as f32 / 2.0, pose.y + part.raster.h as f32 / 2.0);
    (cx - sw / 2.0, cy - sh / 2.0, cx + sw / 2.0, cy + sh / 2.0)
}

/// Where a still picture of `parts` has every sprite, for the host's renders at time `t` ms: position, scale and alpha, composited over `ground` at (`w`, `h`).
/// Not used by the shell, which has the compositor draw the sprites.
pub fn compose(parts: &[Part], w: u32, h: u32, t: Option<f64>, ground: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    for p in out.chunks_exact_mut(4) {
        p.copy_from_slice(&ground);
    }
    for part in parts {
        let pose = match t {
            Some(t) => part.pose(t, None),
            None => part.rest_pose(),
        };
        if pose.alpha <= 0.0 {
            continue;
        }
        let (left, top, right, bottom) = extent(part, &pose);
        let (x0, x1) = ((left.floor().max(0.0)) as u32, (right.ceil().min(w as f32)) as u32);
        let (y0, y1) = ((top.floor().max(0.0)) as u32, (bottom.ceil().min(h as f32)) as u32);
        for y in y0..y1 {
            for x in x0..x1 {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let (u, v) = ((fx - left) / pose.scale, (fy - top) / pose.scale);
                if u < 0.0 || v < 0.0 || u >= part.raster.w as f32 || v >= part.raster.h as f32 {
                    continue;
                }
                let mut fade = 1.0;
                if let Some((r, e)) = part.clip {
                    if fx < r.x || fx >= r.x + r.w || fy < r.y || fy >= r.y + r.h {
                        continue;
                    }
                    let f = |d: f32, width: f32| if width <= 0.0 { 1.0 } else { (d / width).clamp(0.0, 1.0) };
                    fade = f(fx - r.x, e.left) * f(r.x + r.w - fx, e.right) * f(fy - r.y, e.top) * f(r.y + r.h - fy, e.bottom);
                }
                // bilinear, as the compositor samples
                let (iu, iv) = ((u - 0.5).floor(), (v - 0.5).floor());
                let (tu, tv) = (u - 0.5 - iu, v - 0.5 - iv);
                let at = |i: f32, j: f32| -> [f32; 4] {
                    let (i, j) = (i.clamp(0.0, part.raster.w as f32 - 1.0) as usize, j.clamp(0.0, part.raster.h as f32 - 1.0) as usize);
                    let o = (j * part.raster.w as usize + i) * 4;
                    [part.raster.px[o] as f32, part.raster.px[o + 1] as f32, part.raster.px[o + 2] as f32, part.raster.px[o + 3] as f32]
                };
                let (a, b, c, d) = (at(iu, iv), at(iu + 1.0, iv), at(iu, iv + 1.0), at(iu + 1.0, iv + 1.0));
                let o = ((y * w + x) * 4) as usize;
                let op = pose.alpha * fade;
                let sa = ((1.0 - tu) * (1.0 - tv) * a[3] + tu * (1.0 - tv) * b[3] + (1.0 - tu) * tv * c[3] + tu * tv * d[3]) / 255.0 * op;
                for ch in 0..3 {
                    let s = ((1.0 - tu) * (1.0 - tv) * a[ch] + tu * (1.0 - tv) * b[ch] + (1.0 - tu) * tv * c[ch] + tu * tv * d[ch]) * op;
                    out[o + ch] = (s + out[o + ch] as f32 * (1.0 - sa)).round().clamp(0.0, 255.0) as u8;
                }
                out[o + 3] = (sa * 255.0 + out[o + 3] as f32 * (1.0 - sa)).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

// ---- the live picture: the parts as image layers in the card's group
use crate::kit::Kit;
use compositor_protocol::Ease;
use ui::scene::ImageLayer;

pub struct Live {
    cond: Cond,
    light: bool,
    stage: Stage,
    parts: Vec<Part>,
    layers: Vec<ImageLayer>,
    /// The box's top left in the card's space.
    origin: (f32, f32),
    running: bool,
    /// The tick the paths were last restated at.
    last: u64,
}

impl Live {
    /// Makes the layers (the caller is inside the card's `adopting`), at rest.
    pub fn new(kit: &mut Kit, cond: Cond, stage: Stage, light: bool, origin: (f32, f32), z: i32) -> std::io::Result<Live> {
        let parts = parts(cond, stage, light);
        let mut layers = Vec::with_capacity(parts.len());
        for p in &parts {
            let rest = p.rest_pose();
            let mut l = kit.ui.scene.image(p.raster.w, p.raster.h, z, p.props(&rest, origin))?;
            l.pixels_mut().copy_from_slice(&p.raster.px);
            l.damage(None);
            layers.push(l);
        }
        Ok(Live { cond, light, stage, parts, layers, origin, running: false, last: 0 })
    }

    pub fn is(&self, cond: Cond, stage: Stage, light: bool) -> bool {
        self.cond == cond && self.light == light && self.stage == stage
    }

    /// Where the card is (its content origin changed): the sprites are put on the new place at rest.
    pub fn place(&mut self, origin: (f32, f32)) {
        if self.origin != origin {
            self.origin = origin;
            self.rest();
        }
    }

    /// Stops the motion and rests every sprite on whole pixels.
    pub fn rest(&mut self) {
        self.running = false;
        for (p, l) in self.parts.iter().zip(&self.layers) {
            l.set(p.props(&p.rest_pose(), self.origin));
        }
    }

    pub fn running(&self) -> bool {
        self.running
    }

    /// The pixels the picture holds, in bytes.
    pub fn bytes(&self) -> usize {
        self.parts.iter().map(|p| p.raster.px.len()).sum()
    }

    /// Restates the paths of the moving sprites for the next [`LEAD_MS`] (once per [`TICK_MS`]); returns when the next is due.
    pub fn step(&mut self, now: u64) -> u64 {
        let t = now - now % TICK_MS;
        if self.running && t == self.last {
            return t + TICK_MS;
        }
        for (p, l) in self.parts.iter().zip(&self.layers) {
            if p.motion == Motion::STILL {
                continue;
            }
            if !self.running || p.wrapped(self.last, t) {
                l.set(p.props(&p.pose(t as f64, None), self.origin));
            }
            l.animate(p.props(&p.pose((t + LEAD_MS as u64) as f64, Some(t as f64)), self.origin), LEAD_MS, Ease::Linear);
        }
        self.running = true;
        self.last = t;
        t + TICK_MS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn every() -> Vec<Cond> {
        let mut v = Vec::new();
        for sky in [Sky::Clear, Sky::Partly, Sky::Cloudy, Sky::Fog, Sky::Drizzle, Sky::Rain, Sky::Storm, Sky::Snow] {
            for day in [true, false] {
                for heavy in 0..3 {
                    for wind in [false, true] {
                        v.push(Cond { sky, day, heavy, wind });
                    }
                }
            }
        }
        v
    }

    /// The card's sky and the view's, and the box the first card's picture was made in.
    fn stages() -> [Stage; 3] {
        [Stage::square(164.0), Stage { w: 433.0, h: 244.0, hero: (318.0, 104.0), size: 190.0, ground: 236.0, fade: [14.0, 14.0, 10.0, 30.0], shy: 0.5 }, Stage { w: 1160.0, h: 620.0, hero: (560.0, 276.0), size: 470.0, ground: 600.0, fade: [150.0, 150.0, 50.0, 150.0], shy: 0.2 }]
    }

    #[test]
    fn the_wmo_code_and_the_day_flag_choose_the_picture_and_the_kind_is_the_fallback() {
        let doc = |code: i64, day: bool, wind: f64| json!({"current": {"code": code, "isDay": day, "windMph": wind}, "display": {"nowKind": "cloud"}});
        assert_eq!(Cond::from_doc(&doc(0, false, 3.0)), Some(Cond { sky: Sky::Clear, day: false, heavy: 0, wind: false }));
        assert_eq!(Cond::from_doc(&doc(65, true, 20.0)), Some(Cond { sky: Sky::Rain, day: true, heavy: 2, wind: true }));
        assert_eq!(Cond::from_doc(&doc(81, true, 0.0)).map(|c| (c.sky, c.heavy)), Some((Sky::Rain, 1)), "showers are rain");
        assert_eq!(Cond::from_doc(&doc(96, true, 0.0)).map(|c| (c.sky, c.heavy)), Some((Sky::Storm, 2)));
        assert_eq!(Cond::from_doc(&doc(71, true, 0.0)).map(|c| (c.sky, c.heavy)), Some((Sky::Snow, 0)));
        assert_eq!(Cond::from_doc(&doc(45, false, 0.0)).map(|c| c.sky), Some(Sky::Fog));
        assert_eq!(Cond::from_doc(&json!({"display": {"nowKind": "partly-night"}})).map(|c| (c.sky, c.day)), Some((Sky::Partly, false)));
        assert_eq!(Cond::from_doc(&json!({"display": {"nowKind": "moon"}})).map(|c| (c.sky, c.day)), Some((Sky::Clear, false)));
        assert_eq!(Cond::from_doc(&json!({"display": {}})), None);
    }

    #[test]
    fn every_condition_is_drawn_on_whole_pixels_at_rest_and_nothing_is_empty_or_unpremultiplied() {
        for st in stages() {
            for cond in every() {
                let p = parts(cond, st, !cond.day);
                assert!(!p.is_empty(), "{cond:?}");
                for part in &p {
                    let rest = part.rest_pose();
                    assert_eq!((rest.x.fract(), rest.y.fract()), (0.0, 0.0), "{cond:?} at {}: a sprite at rest is on whole pixels", st.w);
                    assert!(part.raster.px.chunks_exact(4).any(|q| q[3] > 0), "{cond:?}: a sprite that draws nothing");
                    assert!(part.raster.px.chunks_exact(4).all(|q| q[0] <= q[3] && q[1] <= q[3] && q[2] <= q[3]), "{cond:?}: premultiplied");
                }
            }
        }
    }

    #[test]
    fn what_is_seen_at_rest_is_inside_its_box() {
        for st in stages() {
            for cond in every() {
                for part in parts(cond, st, cond.day) {
                    let rest = part.rest_pose();
                    if rest.alpha <= 0.0 {
                        continue;
                    }
                    // a glow made small and shown large fades out to nothing at its edge, which may be outside the box
                    if part.scale0 > 1.0 {
                        continue;
                    }
                    let (l, t, r, b) = extent(&part, &rest);
                    let slack = 0.2 * st.size;
                    match part.clip {
                        None => assert!(l >= -slack && t >= -slack && r <= st.w + slack && b <= st.h + slack, "{cond:?} at {}: a sprite outside its box: {l} {t} {r} {b}", st.w),
                        Some((c, _)) => assert!(c.x >= -0.5 && c.y >= -0.5 && c.x + c.w <= st.w + 0.5 && c.y + c.h <= st.h + 0.5, "{cond:?}: a clip outside the box {c:?}"),
                    }
                }
            }
        }
    }

    #[test]
    fn a_picture_made_for_a_look_is_not_a_scaled_bitmap() {
        // the same condition at twice the side has twice the sprites' sides: it is drawn again at the size, not stretched
        let a = parts(Cond { sky: Sky::Cloudy, day: true, heavy: 1, wind: false }, Stage::square(100.0), false);
        let b = parts(Cond { sky: Sky::Cloudy, day: true, heavy: 1, wind: false }, Stage::square(200.0), false);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert!((y.raster.w as i32 - 2 * x.raster.w as i32).abs() <= 3, "{} vs {}", x.raster.w, y.raster.w);
        }
    }

    #[test]
    fn the_sheets_of_rain_and_snow_repeat_every_period_so_putting_one_back_is_not_seen() {
        for st in stages() {
            for sky in [Sky::Rain, Sky::Snow, Sky::Drizzle] {
                let p = parts(Cond { sky, day: true, heavy: 1, wind: false }, st, false);
                let sheets: Vec<&Part> = p.iter().filter(|q| matches!(q.motion.y, Wave::Saw { .. }) && !q.motion.bump).collect();
                assert!(!sheets.is_empty(), "{sky:?} has sheets");
                for s in sheets {
                    let Wave::Saw { to, period_ms, .. } = s.motion.y else { unreachable!() };
                    let period = to.round() as usize;
                    let (w, h) = (s.raster.w as usize, s.raster.h as usize);
                    let mut worst = 0i32;
                    for y in 0..h - period {
                        for x in 0..w * 4 {
                            worst = worst.max((s.raster.px[y * w * 4 + x] as i32 - s.raster.px[(y + period) * w * 4 + x] as i32).abs());
                        }
                    }
                    assert!(worst <= 3, "{sky:?}: rows a period apart differ by {worst} levels (only the dither may)");
                    // and it covers its window at every offset of the slide and the lead
                    let travel = to * (1.0 + LEAD_MS as f32 / period_ms as f32);
                    let (clip, _) = s.clip.expect("a sheet is clipped to its window");
                    assert!(s.at.1 + travel <= clip.y + 0.5, "{sky:?}: at the end of the slide the sheet's top is below the window's");
                    assert!(s.at.1 + s.raster.h as f32 >= clip.y + clip.h - 0.5, "{sky:?}: at the start of the slide the sheet's bottom is above the window's");
                    assert!(s.at.0 <= clip.x && s.at.0 + s.raster.w as f32 >= clip.x + clip.w - 0.5, "{sky:?}: the sheet is as wide as its window");
                }
            }
        }
    }

    #[test]
    fn the_paths_are_continuous_across_a_tick_and_a_wrap_is_a_whole_period() {
        let p = parts(Cond { sky: Sky::Rain, day: true, heavy: 2, wind: true }, stages()[1], false);
        for part in &p {
            for t in (0..60_000u64).step_by(TICK_MS as usize) {
                let a = part.pose(t as f64 + LEAD_MS as f64, Some(t as f64));
                let w = part.pose(t as f64 + LEAD_MS as f64, None);
                if let Some(period) = part.motion.y.period().or(part.motion.x.period()) {
                    // an unwrapped target is the wrapped pose plus whole periods of the path
                    let span = match (part.motion.y, part.motion.x) {
                        (Wave::Saw { from, to, .. }, _) => (to - from, a.y - w.y),
                        (_, Wave::Saw { from, to, .. }) => (to - from, a.x - w.x),
                        _ => unreachable!(),
                    };
                    let k = span.1 / span.0;
                    assert!((k - k.round()).abs() < 1e-3, "t={t} period {period}: off by {k} periods");
                } else {
                    assert_eq!((a.x, a.y, a.alpha), (w.x, w.y, w.alpha));
                }
            }
        }
    }

    #[test]
    fn a_path_between_two_ticks_is_straight_enough_unless_it_is_a_saw_or_a_ramp_that_is_seen_only_while_it_is_lit() {
        for cond in every() {
            for part in parts(cond, stages()[1], false) {
                let ramps = [part.motion.x, part.motion.y, part.motion.scale].iter().any(|w| matches!(w, Wave::Ramp { .. }));
                if part.motion.x.period().is_some() || part.motion.y.period().is_some() || ramps {
                    continue;
                }
                for t in (0..60_000u64).step_by(TICK_MS as usize) {
                    // the compositor moves a sprite along the straight line between the poses at the two ends of a lead: the true path is never more than
                    // a pixel from it (a sine of a 12 s period moves 2.6 px of its 10 in a lead; the chord it is replaced by is within 0.2 px of it)
                    let (a, b, mid) = (part.pose(t as f64, None), part.pose((t + LEAD_MS as u64) as f64, None), part.pose((t + LEAD_MS as u64 / 2) as f64, None));
                    assert!((mid.x - (a.x + b.x) / 2.0).abs() < 1.0 && (mid.y - (a.y + b.y) / 2.0).abs() < 1.0, "{cond:?} t={t}: the path bows {} {} from its chord", mid.x - (a.x + b.x) / 2.0, mid.y - (a.y + b.y) / 2.0);
                    assert!((mid.scale - (a.scale + b.scale) / 2.0).abs() < 0.02 * part.scale0.max(1.0), "{cond:?} t={t}: the scale bows");
                }
            }
        }
    }

    #[test]
    fn what_crosses_the_sky_is_put_back_while_it_cannot_be_seen() {
        for cond in every() {
            for part in parts(cond, stages()[1], cond.day) {
                if !part.motion.bump {
                    continue;
                }
                let period = part.motion.x.period().or(part.motion.y.period()).expect("a fading sprite has a saw");
                let phase = if part.motion.x.period().is_some() { part.motion.x.phase() } else { part.motion.y.phase() };
                assert_eq!(period as u64 % TICK_MS, 0, "{cond:?}: a crossing is a whole number of ticks");
                // at the instants a period ends, and a moment either side of them, the sprite has no light
                for k in 1..=2 {
                    let at = (k as f64 - phase).max(0.0) * period as f64;
                    let (before, after) = (part.pose((at - 1.0).max(0.0), None), part.pose(at + 1.0, None));
                    assert!(before.alpha < 0.03 && after.alpha < 0.03, "{cond:?}: put back at {at} while seen: {} {}", before.alpha, after.alpha);
                }
            }
        }
    }

    #[test]
    fn the_rays_of_the_sun_turn_by_one_ring_taking_over_from_the_next_and_the_light_stays_even() {
        let p = parts(Cond { sky: Sky::Clear, day: true, heavy: 0, wind: false }, stages()[1], false);
        let rings: Vec<&Part> = p.iter().filter(|q| matches!(q.motion.alpha, Wave::Tent { .. })).collect();
        assert_eq!(rings.len(), RINGS);
        assert_eq!(rings.iter().filter(|q| q.rest_alpha > 0.0).count(), 1, "a still sun shows one ring");
        for t in (0..RING_S as u64 * 1000).step_by(250) {
            let sum: f32 = rings.iter().map(|q| q.pose(t as f64, None).alpha).sum();
            assert!((sum - 0.82).abs() < 0.01, "t={t}: the rings add up to {sum}");
        }
    }

    #[test]
    fn lightning_is_a_soft_pulse_that_never_strobes() {
        let p = parts(Cond { sky: Sky::Storm, day: true, heavy: 2, wind: false }, stages()[1], false);
        let flash: Vec<&Part> = p.iter().filter(|q| matches!(q.motion.alpha, Wave::Pulse { peak, .. } if peak >= 0.85)).collect();
        assert_eq!(flash.len(), 3, "the sky's glow and two bolts (a splash is a pulse too, but a softer one)");
        for f in flash {
            let Wave::Pulse { width_ms, period_ms, .. } = f.motion.alpha else { unreachable!() };
            assert!(width_ms >= 1_000, "a pulse of a second or more");
            assert!(period_ms >= 5_000, "no more than one in five seconds");
            // at most one lit stretch a period, rising and falling in steps no larger than a third of the peak per 100 ms
            let mut last = f.pose(0.0, None).alpha;
            for t in (0..period_ms * 3).step_by(100) {
                let a = f.pose(t as f64, None).alpha;
                assert!((a - last).abs() <= 0.34, "t={t}: {last} to {a}");
                last = a;
            }
        }
    }

    #[test]
    fn the_still_picture_shows_the_bolt_dimly_and_the_motion_is_absent() {
        let p = parts(Cond { sky: Sky::Storm, day: true, heavy: 1, wind: false }, stages()[1], false);
        assert!(p.iter().any(|q| q.rest_alpha > 0.5 && matches!(q.motion.alpha, Wave::Pulse { .. })));
        assert!(moves(&p));
        assert!(!moves(&[]));
    }

    #[test]
    fn a_picture_is_few_layers_the_scene_has_256_for_all_of_the_shell_and_home_holds_over_a_hundred() {
        for cond in every() {
            for (st, cap) in [(stages()[1], 38usize), (stages()[2], 46usize)] {
                let n = parts(cond, st, cond.day).len();
                assert!(n <= cap, "{cond:?} at {}: {n} layers (at most {cap})", st.w);
            }
        }
    }

    #[test]
    fn a_picture_is_a_few_megabytes_at_most_even_for_the_view() {
        for cond in every() {
            for (st, cap) in [(stages()[1], 2_200_000usize), (stages()[2], 6_000_000usize)] {
                let bytes: usize = parts(cond, st, cond.day).iter().map(|q| q.raster.px.len()).sum();
                assert!(bytes <= cap, "{cond:?} at {}: {bytes} bytes", st.w);
            }
        }
    }

    /// `cargo test --release -p lumatv-ui --lib weather_art::tests::cost -- --ignored --nocapture`: what a picture costs to make and to move.
    #[test]
    #[ignore]
    fn cost() {
        use std::time::Instant;
        for (name, cond) in [
            ("clear-day", Cond { sky: Sky::Clear, day: true, heavy: 0, wind: false }),
            ("clear-night", Cond { sky: Sky::Clear, day: false, heavy: 0, wind: false }),
            ("partly", Cond { sky: Sky::Partly, day: true, heavy: 1, wind: false }),
            ("cloudy", Cond { sky: Sky::Cloudy, day: true, heavy: 1, wind: false }),
            ("rain", Cond { sky: Sky::Rain, day: true, heavy: 1, wind: false }),
            ("storm", Cond { sky: Sky::Storm, day: true, heavy: 2, wind: true }),
            ("snow", Cond { sky: Sky::Snow, day: true, heavy: 2, wind: false }),
        ] {
            for (which, st) in [("card", stages()[1]), ("view", stages()[2])] {
                let t = Instant::now();
                let p = parts(cond, st, false);
                let make = t.elapsed();
                let bytes: usize = p.iter().map(|q| q.raster.px.len()).sum();
                let (_h, mut kit) = crate::kit::testkit::kit(1.0);
                let t = Instant::now();
                let mut live = Live::new(&mut kit, cond, st, false, (0.0, 0.0), 13).unwrap();
                let build = t.elapsed();
                let moving = live.parts.iter().filter(|q| q.motion != Motion::STILL).count();
                let n = 2_000u64;
                let t = Instant::now();
                for i in 0..n {
                    live.step(1_000_000 + i * TICK_MS);
                    if i % 50 == 0 {
                        let _ = kit.ui.scene.try_commit();
                    }
                }
                let per = t.elapsed() / n as u32;
                println!("{name} {which}: {} sprites ({} moving), {} KiB of pixels; sprites made in {make:?}, layers made in {build:?}; one tick {per:?} (2.5 ticks a second)", p.len(), moving, bytes / 1024);
            }
        }
    }
}
