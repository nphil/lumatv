//! The weather card's picture: a vector illustration of the current condition, drawn once and moved by the compositor.
//!
//! Every shape is a signed-distance function evaluated per pixel of a small sprite (analytic anti-aliasing, a gradient with a
//! dither so a dark ramp does not band), so a sprite is exact at the size it is made for and the picture is never a scaled bitmap.
//! The motion is not drawn: each sprite is an image layer, and the compositor moves it (`Request::Animate`, linear) along a
//! closed-form path the app only restates every [`TICK_MS`]: a cloud's float and the sun's breathing are sines sampled into
//! straight segments of 0.5 s (a step of 4 % of a period at most, so the corners do not show), the rain and snow are one tall
//! sheet each whose pattern repeats every `P` pixels and which slides exactly `P` and is put back (the pictures are the same),
//! and the lightning is a slow raised-cosine pulse of two sprites, never a strobe. Nothing is drawn, allocated or sent per frame:
//! a tick is one `Animate` per moving sprite and a commit, twice a second, and when the card is not seen (an app over Home, the
//! screensaver, standby, a Home layer over the card, reduced motion) there is no tick at all and the sprites rest on whole pixels.
//!
//! The picture is designed in a square of 200 units and scaled to the box the card gives it.

use compositor_protocol::{Props, Rect};
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
}

// ---- the colours (Rosé Pine: Main for the dark look, Dawn for the light; modest, for an OLED panel)
type Rgb = [f32; 3];

fn hex(v: u32) -> Rgb {
    [((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0]
}

#[derive(Clone, Copy)]
struct Pal {
    sun: Rgb,
    sun_hi: Rgb,
    sun_glow: Rgb,
    moon: Rgb,
    moon_hi: Rgb,
    moon_glow: Rgb,
    cloud_hi: Rgb,
    cloud_lo: Rgb,
    cloud_rim: f32,
    storm_hi: Rgb,
    storm_lo: Rgb,
    rain: Rgb,
    snow: Rgb,
    fog: Rgb,
    bolt: Rgb,
    star: Rgb,
    flash: Rgb,
}

impl Pal {
    fn of(light: bool) -> Pal {
        if light {
            Pal {
                sun: hex(0xEA9D34),
                sun_hi: hex(0xF6C177),
                sun_glow: hex(0xEA9D34),
                moon: hex(0x907AA9),
                moon_hi: hex(0xB4A2C9),
                moon_glow: hex(0x907AA9),
                cloud_hi: hex(0xFFFFFF),
                cloud_lo: hex(0xC9C3D6),
                cloud_rim: 0.9,
                storm_hi: hex(0x9893A5),
                storm_lo: hex(0x6E6A86),
                rain: hex(0x286983),
                snow: hex(0x56949F),
                fog: hex(0x797593),
                bolt: hex(0xEA9D34),
                star: hex(0x907AA9),
                flash: hex(0xEA9D34),
            }
        } else {
            Pal {
                sun: hex(0xF6C177),
                sun_hi: hex(0xFFE3B0),
                sun_glow: hex(0xF6C177),
                moon: hex(0xE6E0F0),
                moon_hi: hex(0xFFFFFF),
                moon_glow: hex(0xC4A7E7),
                cloud_hi: hex(0xD9D6EC),
                cloud_lo: hex(0x8B88A6),
                cloud_rim: 0.55,
                storm_hi: hex(0x77738F),
                storm_lo: hex(0x4A4760),
                rain: hex(0x9CCFD8),
                snow: hex(0xE0DEF4),
                fog: hex(0x9C98B4),
                bolt: hex(0xF6C177),
                star: hex(0xE0DEF4),
                flash: hex(0xE8EDFF),
            }
        }
    }
}

// ---- the raster: premultiplied float accumulation, one dithered quantisation at the end
#[derive(Clone)]
pub struct Raster {
    pub w: u32,
    pub h: u32,
    /// Premultiplied RGBA8.
    pub px: Vec<u8>,
}

struct Acc {
    w: usize,
    h: usize,
    a: Vec<[f32; 4]>,
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix3(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)]
}

/// Coverage of a pixel whose centre is `d` pixels outside the edge.
fn cov(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

fn sd_circle(x: f32, y: f32, cx: f32, cy: f32, r: f32) -> f32 {
    ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r
}

/// Distance to a segment, and how far along it the nearest point is (0 to 1).
fn seg(x: f32, y: f32, a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    let (px, py, bx, by) = (x - a.0, y - a.1, b.0 - a.0, b.1 - a.1);
    let len2 = (bx * bx + by * by).max(1e-6);
    let t = ((px * bx + py * by) / len2).clamp(0.0, 1.0);
    (((px - bx * t).powi(2) + (py - by * t).powi(2)).sqrt(), t)
}

fn sd_box(x: f32, y: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let (qx, qy) = ((x - cx).abs() - hw + r, (y - cy).abs() - hh + r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

fn sd_poly(x: f32, y: f32, p: &[(f32, f32)]) -> f32 {
    let mut d = f32::MAX;
    let mut inside = false;
    let mut j = p.len() - 1;
    for i in 0..p.len() {
        d = d.min(seg(x, y, p[j], p[i]).0);
        if (p[i].1 > y) != (p[j].1 > y) && x < (p[j].0 - p[i].0) * (y - p[i].1) / (p[j].1 - p[i].1) + p[i].0 {
            inside = !inside;
        }
        j = i;
    }
    if inside { -d } else { d }
}

impl Acc {
    fn new(w: u32, h: u32) -> Acc {
        Acc { w: w as usize, h: h as usize, a: vec![[0.0; 4]; (w * h) as usize] }
    }

    /// Composites `f(x, y)` (a straight colour and a coverage, at the pixel's centre) over every pixel of the box `(x0, y0)..(x1, y1)`.
    fn paint(&mut self, bounds: (f32, f32, f32, f32), f: impl Fn(f32, f32) -> Option<(Rgb, f32)>) {
        let x0 = bounds.0.floor().max(0.0) as usize;
        let y0 = bounds.1.floor().max(0.0) as usize;
        let x1 = (bounds.2.ceil().max(0.0) as usize).min(self.w);
        let y1 = (bounds.3.ceil().max(0.0) as usize).min(self.h);
        for y in y0..y1 {
            for x in x0..x1 {
                if let Some((c, a)) = f(x as f32 + 0.5, y as f32 + 0.5)
                    && a > 0.0
                {
                    let d = &mut self.a[y * self.w + x];
                    let k = 1.0 - a;
                    *d = [c[0] * a + d[0] * k, c[1] * a + d[1] * k, c[2] * a + d[2] * k, a + d[3] * k];
                }
            }
        }
    }

    fn whole(&self) -> (f32, f32, f32, f32) {
        (0.0, 0.0, self.w as f32, self.h as f32)
    }

    fn finish(self, opacity: f32) -> Raster {
        let mut px = vec![0u8; self.w * self.h * 4];
        for y in 0..self.h {
            for x in 0..self.w {
                let s = self.a[y * self.w + x];
                // Interleaved gradient noise (Jimenez): one level of dither, so a faint ramp is a ramp and not steps
                let n = (52.982_92 * ((0.067_110_56 * x as f32 + 0.005_837_15 * y as f32).fract())).fract() - 0.5;
                let i = (y * self.w + x) * 4;
                for c in 0..4 {
                    let v = s[c] * opacity * 255.0;
                    px[i + c] = if v < 0.004 { 0 } else { (v + n).round().clamp(0.0, 255.0) as u8 };
                }
                // premultiplied: a colour never exceeds its alpha
                let a = px[i + 3];
                for c in 0..3 {
                    px[i + c] = px[i + c].min(a);
                }
            }
        }
        Raster { w: self.w as u32, h: self.h as u32, px }
    }
}

// ---- the motion
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wave {
    Zero,
    /// `amp * sin(2 pi (t / period + phase))`, in pixels (position), or as a fraction (scale, alpha).
    Sine { amp: f32, period_ms: u32, phase: f32 },
    /// `from` to `to` over each period, then again: the sheets of rain and snow and the wind's streaks.
    Saw { from: f32, to: f32, period_ms: u32 },
    /// A raised cosine of `width_ms` at `at_ms` of every `period_ms`, up to `peak`: the lightning.
    Pulse { peak: f32, period_ms: u32, at_ms: u32, width_ms: u32 },
}

impl Wave {
    /// The value at `t` ms. A saw is wrapped into its period, or, with an `anchor` (the time of the tick), carried on past the period from where it is
    /// at the anchor: the sheet's pattern repeats, so a path that runs over the end is the same picture and the sprite is put back at the next tick.
    fn at(self, t: f64, anchor: Option<f64>) -> f32 {
        match self {
            Wave::Zero => 0.0,
            Wave::Sine { amp, period_ms, phase } => amp * (std::f64::consts::TAU * (t / period_ms as f64 + phase as f64)).sin() as f32,
            Wave::Saw { from, to, period_ms } => {
                let p = period_ms as f64;
                let f = match anchor {
                    Some(a) => (a / p).fract() + (t - a) / p,
                    None => (t / p).fract(),
                };
                from + (to - from) * f as f32
            }
            Wave::Pulse { peak, period_ms, at_ms, width_ms } => {
                let m = (t % period_ms as f64) - at_ms as f64;
                if m <= 0.0 || m >= width_ms as f64 { 0.0 } else { peak * (0.5 - 0.5 * (std::f64::consts::TAU * m / width_ms as f64).cos()) as f32 }
            }
        }
    }

    /// Where the wave rests when nothing moves.
    fn rest(self) -> f32 {
        match self {
            Wave::Saw { from, to, .. } => from + (to - from) * 0.35,
            _ => 0.0,
        }
    }

    fn period(self) -> Option<u32> {
        match self {
            Wave::Saw { period_ms, .. } => Some(period_ms),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub x: Wave,
    pub y: Wave,
    /// Added to 1.
    pub scale: Wave,
    /// Added to the base alpha.
    pub alpha: Wave,
    /// Fades the sprite in and out over each period of its saw (a streak of wind), so it is never seen to start or be put back.
    pub bump: bool,
}

impl Motion {
    const STILL: Motion = Motion { x: Wave::Zero, y: Wave::Zero, scale: Wave::Zero, alpha: Wave::Zero, bump: false };
}

pub struct Part {
    pub raster: Raster,
    /// The sprite's top left in the box, on whole pixels.
    pub at: (f32, f32),
    pub alpha: f32,
    /// The alpha it has at rest, when that is not `alpha` (the bolt shows dimly in a still picture).
    pub rest_alpha: f32,
    pub motion: Motion,
    /// Only this rectangle of the box (box pixels) is drawn, its bottom and sides fading out.
    pub clip: Option<(Rect, f32)>,
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
            let f = ((t % p as f64) / p as f64) as f32;
            alpha *= smooth(0.0, 0.2, f) * (1.0 - smooth(0.8, 1.0, f));
        }
        Pose { x: self.at.0 + m.x.at(t, anchor), y: self.at.1 + m.y.at(t, anchor), scale: 1.0 + m.scale.at(t, anchor), alpha }
    }

    pub fn rest_pose(&self) -> Pose {
        let m = &self.motion;
        Pose { x: (self.at.0 + m.x.rest()).round(), y: (self.at.1 + m.y.rest()).round(), scale: 1.0, alpha: if m.bump { self.alpha } else { self.rest_alpha } }
    }

    /// Whether a period of this part's saw ended between the ticks at `before` and `now`: the sprite is put back.
    pub fn wrapped(&self, before: u64, now: u64) -> bool {
        self.motion.x.period().or(self.motion.y.period()).is_some_and(|p| now / p as u64 != before / p as u64)
    }

    pub fn props(&self, p: &Pose, origin: (f32, f32)) -> Props {
        let mut props = Props { opacity: p.alpha, scale: p.scale, ..Props::at(origin.0 + p.x, origin.1 + p.y) };
        if let Some((r, feather)) = self.clip {
            props.clip = Some(Rect { x: origin.0 + r.x, y: origin.1 + r.y, w: r.w, h: r.h });
            props.feather = compositor_protocol::Edges::new(feather, feather, 0.0, feather * 1.6);
        }
        props
    }
}

// ---- the sprites
struct Ctx {
    light: bool,
    /// Pixels per design unit.
    k: f32,
    pal: Pal,
}

impl Ctx {
    fn px(&self, units: f32) -> f32 {
        units * self.k
    }

    fn size(&self, w: f32, h: f32) -> (u32, u32) {
        ((w * self.k).ceil().max(1.0) as u32, (h * self.k).ceil().max(1.0) as u32)
    }

    /// The top left, on whole pixels, of a sprite of `size` centred on (`cx`, `cy`) units.
    fn centred(&self, cx: f32, cy: f32, size: (u32, u32)) -> (f32, f32) {
        ((cx * self.k - size.0 as f32 / 2.0).round(), (cy * self.k - size.1 as f32 / 2.0).round())
    }
}

/// A soft radial glow: `colour` at `alpha` in the middle, nothing at the edge of the square.
fn glow(c: &Ctx, side_u: f32, colour: Rgb, alpha: f32) -> Raster {
    let (w, h) = c.size(side_u, side_u);
    let mut acc = Acc::new(w, h);
    let r = w as f32 / 2.0;
    acc.paint(acc.whole(), |x, y| {
        let d = ((x - r).powi(2) + (y - r).powi(2)).sqrt() / r;
        let a = (1.0 - smooth(0.0, 1.0, d)).powi(2) * alpha;
        Some((colour, a))
    });
    acc.finish(1.0)
}

fn disc(c: &Ctx, r_u: f32, hi: Rgb, lo: Rgb) -> Raster {
    let r = c.px(r_u);
    let side = (2.0 * r).ceil() as u32 + 4;
    let mut acc = Acc::new(side, side);
    let m = side as f32 / 2.0;
    acc.paint(acc.whole(), |x, y| {
        let d = ((x - m).powi(2) + (y - m).powi(2)).sqrt();
        let a = cov(d - r);
        // lit from the upper left
        let t = ((x - m) * 0.45 + (y - m) * 0.55 + r) / (2.0 * r);
        Some((mix3(hi, lo, t.clamp(0.0, 1.0).powf(1.4)), a))
    });
    acc.finish(1.0)
}

/// `n` rays round a centre, from `r0` to `r1` units, `width` wide, fading out towards their tips; `first` is the angle of the first.
fn rays(c: &Ctx, n: usize, first: f32, (r0, r1): (f32, f32), width: f32, colour: Rgb, alpha: f32) -> Raster {
    let (w, h) = c.size(2.0 * r1 + 12.0, 2.0 * r1 + 12.0);
    let mut acc = Acc::new(w, h);
    let (mx, my) = (w as f32 / 2.0, h as f32 / 2.0);
    let wid = c.px(width) / 2.0;
    for i in 0..n {
        let a = first + std::f32::consts::TAU * i as f32 / n as f32;
        let (dx, dy) = (a.cos(), a.sin());
        let p0 = (mx + dx * c.px(r0), my + dy * c.px(r0));
        let p1 = (mx + dx * c.px(r1), my + dy * c.px(r1));
        acc.paint((p0.0.min(p1.0) - wid - 2.0, p0.1.min(p1.1) - wid - 2.0, p0.0.max(p1.0) + wid + 2.0, p0.1.max(p1.1) + wid + 2.0), |x, y| {
            let (d, t) = seg(x, y, p0, p1);
            // tapering: the tip is thinner than the root
            let wt = wid * (1.0 - 0.45 * t);
            Some((colour, cov(d - wt) * alpha * (1.0 - 0.55 * t)))
        });
    }
    acc.finish(1.0)
}

/// A cloud of the canonical shape (about 104 by 72 units at scale 1).
fn cloud(c: &Ctx, scale: f32, hi: Rgb, lo: Rgb, rim: f32) -> Raster {
    let s = c.k * scale;
    let (w, h) = c.size(120.0 * scale, 90.0 * scale);
    let mut acc = Acc::new(w, h);
    let (ox, oy) = (60.0 * s, 50.0 * s);
    let sdf = move |x: f32, y: f32| -> f32 {
        let (x, y) = (x - ox, y - oy);
        let a = sd_circle(x, y, -27.0 * s, 0.0, 21.0 * s);
        let b = sd_circle(x, y, 3.0 * s, -12.0 * s, 29.0 * s);
        let cc = sd_circle(x, y, 33.0 * s, 3.0 * s, 19.0 * s);
        let base = sd_box(x, y, 0.0, 14.0 * s, 50.0 * s, 17.0 * s, 17.0 * s);
        let k = 7.0 * s;
        smin(smin(a, b, k), smin(cc, base, k), k)
    };
    let (top, bottom) = (oy - 41.0 * s, oy + 31.0 * s);
    let lift = (3.5 * s).max(2.0);
    acc.paint(acc.whole(), |x, y| {
        let d = sdf(x, y);
        let a = cov(d);
        if a <= 0.0 {
            return None;
        }
        let t = ((y - top) / (bottom - top)).clamp(0.0, 1.0);
        let mut col = mix3(hi, lo, smooth(0.0, 1.0, t));
        // a light rim along the top of every puff (the shape shifted down leaves a band at the top), a shade at the foot
        let rim_band = smooth(-1.0, 1.5, sdf(x, y - lift));
        let foot = smooth(-1.0, 1.5, sdf(x, y + lift * 1.4));
        col = mix3(col, [1.0, 1.0, 1.0], rim_band * 0.55 * rim);
        col = mix3(col, lo, foot * 0.35);
        Some((col, a))
    });
    acc.finish(1.0)
}

/// A four-point sparkle of radius `r` units.
fn sparkle(acc: &mut Acc, c: &Ctx, cx: f32, cy: f32, r: f32, colour: Rgb, alpha: f32) {
    let (cx, cy, r) = (c.px(cx), c.px(cy), c.px(r));
    let thin = (r * 0.17).max(0.55);
    acc.paint((cx - r - 1.0, cy - r - 1.0, cx + r + 1.0, cy + r + 1.0), |x, y| {
        let (dx, dy) = (x - cx, y - cy);
        let e = |a: f32, b: f32| ((dx / a).powi(2) + (dy / b).powi(2)).sqrt() - 1.0;
        let d = (e(r, thin) * thin).min(e(thin, r) * thin);
        let core = cov(sd_circle(x, y, cx, cy, (r * 0.2).max(0.7)));
        Some((colour, cov(d).max(core) * alpha))
    });
}

fn stars(c: &Ctx, list: &[(f32, f32, f32)]) -> Raster {
    let (w, h) = c.size(UNITS, UNITS);
    let mut acc = Acc::new(w, h);
    for &(x, y, r) in list {
        sparkle(&mut acc, c, x, y, r, c.pal.star, 1.0);
    }
    acc.finish(1.0)
}

/// The crescent: a disc with a bite out of the upper right.
fn crescent(c: &Ctx, r_u: f32) -> Raster {
    let r = c.px(r_u);
    let side = (2.0 * r).ceil() as u32 + 6;
    let mut acc = Acc::new(side, side);
    let m = side as f32 / 2.0;
    let (bx, by, br) = (m + r * 0.42, m - r * 0.30, r * 0.86);
    let (hi, lo) = (c.pal.moon_hi, c.pal.moon);
    acc.paint(acc.whole(), |x, y| {
        let d = sd_circle(x, y, m, m, r).max(-sd_circle(x, y, bx, by, br));
        let t = (((x - m) * -0.6 + (y - m) * 0.4) / r * 0.5 + 0.5).clamp(0.0, 1.0);
        Some((mix3(lo, hi, t), cov(d)))
    });
    // the dark side, barely there
    acc.paint(acc.whole(), |x, y| Some((c.pal.moon, cov(sd_circle(x, y, m, m, r)) * if c.light { 0.0 } else { 0.07 })));
    acc.finish(1.0)
}

/// Soft horizontal haze of `w` by `h` units.
fn haze(c: &Ctx, w_u: f32, h_u: f32, colour: Rgb) -> Raster {
    let (w, h) = c.size(w_u + 16.0, h_u + 16.0);
    let mut acc = Acc::new(w, h);
    let (hw, hh) = (c.px(w_u) / 2.0, c.px(h_u) / 2.0);
    let (mx, my) = (w as f32 / 2.0, h as f32 / 2.0);
    acc.paint(acc.whole(), |x, y| {
        let d = sd_box(x, y, mx, my, hw, hh, hh);
        let a = 1.0 - smooth(-hh * 0.9, c.px(4.5), d);
        Some((colour, a * 0.9))
    });
    acc.finish(1.0)
}

/// A tall sheet of streaks whose pattern repeats every `period` px, `streaks` of them in `width` px, each `len` px long leaning `lean` px
/// over its length. `snow` draws flakes instead. The sheet is `height` px.
struct Sheet {
    width: u32,
    height: u32,
    period: f32,
    streaks: usize,
    len: (f32, f32),
    thick: f32,
    lean: f32,
    colour: Rgb,
    seed: u32,
    snow: bool,
}

fn hash(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

fn sheet(s: &Sheet) -> Raster {
    let mut acc = Acc::new(s.width, s.height);
    let copies = (s.height as f32 / s.period).ceil() as i32 + 1;
    // the streaks sit on lanes of the width (jittered), so none stack and the gaps are even; the pattern wraps in y
    for i in 0..s.streaks {
        let lane = (i as f32 + 0.15 + 0.7 * hash(s.seed + i as u32 * 7)) / s.streaks as f32;
        let x = lane * s.width as f32;
        let y = ((i as f32 * 0.618_034 + hash(s.seed + 31 * i as u32) * 0.35) % 1.0) * s.period;
        let len = mix(s.len.0, s.len.1, hash(s.seed + 53 * i as u32));
        for n in -1..copies {
            let y0 = y + n as f32 * s.period;
            if s.snow {
                let r = len * 0.5;
                let (cx, cy) = (x, y0);
                let flake = r >= 2.4;
                acc.paint((cx - r - 2.0, cy - r - 2.0, cx + r + 2.0, cy + r + 2.0), |px, py| {
                    if !flake {
                        return Some((s.colour, cov(sd_circle(px, py, cx, cy, r.max(1.0)))));
                    }
                    let mut d = f32::MAX;
                    for a in 0..3 {
                        let ang = a as f32 * std::f32::consts::PI / 3.0;
                        let (dx, dy) = (ang.cos() * r, ang.sin() * r);
                        d = d.min(seg(px, py, (cx - dx, cy - dy), (cx + dx, cy + dy)).0 - s.thick * 0.5);
                    }
                    Some((s.colour, cov(d).max(cov(sd_circle(px, py, cx, cy, s.thick * 0.9)))))
                });
            } else {
                let (a, b) = ((x, y0), (x - s.lean * len / 20.0, y0 + len));
                acc.paint((a.0.min(b.0) - s.thick - 2.0, a.1 - 2.0, a.0.max(b.0) + s.thick + 2.0, b.1 + s.thick + 2.0), |px, py| {
                    let (d, t) = seg(px, py, a, b);
                    // a head at the bottom, a tail that fades upwards
                    Some((s.colour, cov(d - s.thick * 0.5 * (0.55 + 0.45 * t)) * t.powf(0.8)))
                });
            }
        }
    }
    acc.finish(1.0)
}

fn bolt(c: &Ctx, h_u: f32) -> Raster {
    let s = c.px(h_u) / 70.0;
    let (w, h) = ((30.0 * s).ceil() as u32 + 8, (70.0 * s).ceil() as u32 + 8);
    let mut acc = Acc::new(w, h);
    let (mx, my) = (w as f32 / 2.0, h as f32 / 2.0);
    let p: Vec<(f32, f32)> = [(8.0, -35.0), (-14.0, 4.0), (-1.0, 4.0), (-9.0, 35.0), (16.0, -8.0), (3.0, -8.0), (14.0, -35.0)].iter().map(|&(x, y)| (mx + x * s, my + y * s)).collect();
    let (hi, lo) = ([1.0, 0.97, 0.85], c.pal.bolt);
    acc.paint(acc.whole(), |x, y| {
        let d = sd_poly(x, y, &p) - 0.6 * s;
        Some((mix3(hi, lo, ((y - my) / (35.0 * s) * 0.5 + 0.5).clamp(0.0, 1.0)), cov(d)))
    });
    acc.finish(1.0)
}

/// A streak of wind: a line that ends in a curl.
fn wind(c: &Ctx, len_u: f32, colour: Rgb) -> Raster {
    let (w, h) = c.size(len_u + 20.0, 34.0);
    let mut acc = Acc::new(w, h);
    let k = c.k;
    let (r, y0) = (8.0 * k, h as f32 - 8.0 * k);
    let x1 = w as f32 - 12.0 * k;
    let tail = 2.0 * k;
    let mut path = vec![(tail, y0), (x1, y0)];
    // a curl up and back at the head: a circle that shrinks a little as it goes round
    for i in 1..=18 {
        let th = std::f32::consts::PI * 1.45 * i as f32 / 18.0;
        let rr = r * (1.0 - 0.28 * i as f32 / 18.0);
        path.push((x1 + rr * th.sin(), y0 - r + rr * th.cos()));
    }
    let width = (0.7 * k).max(0.8);
    acc.paint(acc.whole(), |x, y| {
        let mut d = f32::MAX;
        let mut at = 1.0;
        for i in 1..path.len() {
            let (dd, t) = seg(x, y, path[i - 1], path[i]);
            if dd < d {
                d = dd;
                at = if i == 1 { (path[0].0 + (path[1].0 - path[0].0) * t - tail) / (x1 - tail) } else { 1.0 };
            }
        }
        // the line grows from nothing at its tail
        Some((colour, cov(d - width) * smooth(0.0, 0.6, at.clamp(0.0, 1.0)) * 0.95))
    });
    acc.finish(1.0)
}

// ---- the picture of a condition
/// Builds the parts of `cond`, back to front, for a square box of `side` px. `light` is the light look.
pub fn parts(cond: Cond, side: f32, light: bool) -> Vec<Part> {
    let c = Ctx { light, k: side / UNITS, pal: Pal::of(light) };
    let mut out: Vec<Part> = Vec::new();
    let pal = c.pal;
    let k = c.k;
    // a sine in pixels
    let sine = |amp_u: f32, period_s: f32, phase: f32| Wave::Sine { amp: amp_u * k, period_ms: (period_s * 1000.0) as u32, phase };
    let frac = |amp: f32, period_s: f32, phase: f32| Wave::Sine { amp, period_ms: (period_s * 1000.0) as u32, phase };
    let add = |out: &mut Vec<Part>, raster: Raster, centre: (f32, f32), alpha: f32, motion: Motion| {
        let at = c.centred(centre.0, centre.1, (raster.w, raster.h));
        out.push(Part { raster, at, alpha, rest_alpha: alpha, motion, clip: None });
    };
    // the whole sun (or moon) drifts a few pixels over minutes, so that nothing bright sits on the same pixels
    let drift = |px: f32, py: f32| Motion { x: Wave::Sine { amp: px * k, period_ms: 41_000, phase: 0.1 }, y: Wave::Sine { amp: py * k, period_ms: 53_000, phase: 0.6 }, ..Motion::STILL };
    let sun = |c: &Ctx, out: &mut Vec<Part>, at: (f32, f32), size: f32| {
        let g = glow(c, 190.0 * size, pal.sun_glow, 0.5);
        let breathe = Motion { scale: frac(0.045, 9.0, 0.0), alpha: frac(0.12, 9.0, 0.0), ..drift(3.0 * size, 2.5 * size) };
        let a = c.centred(at.0, at.1, (g.w, g.h));
        out.push(Part { raster: g, at: a, alpha: 0.88, rest_alpha: 0.88, motion: breathe, clip: None });
        for (n, (first, r0, r1, wd, phase)) in [(0.0f32, 50.0f32, 70.0f32, 5.5f32, 0.0f32), (std::f32::consts::PI / 6.0, 52.0, 80.0, 4.5, 0.5)].into_iter().enumerate() {
            let r = rays(c, 6, first, (r0 * size, r1 * size), wd * size.max(0.8) * 1.2, pal.sun, 0.95);
            let a = c.centred(at.0, at.1, (r.w, r.h));
            let m = Motion { scale: frac(0.04, 7.0, phase), alpha: frac(0.22, 7.0, phase), ..drift(3.0 * size, 2.5 * size) };
            let _ = n;
            out.push(Part { raster: r, at: a, alpha: 0.78, rest_alpha: 0.78, motion: m, clip: None });
        }
        let d = disc(c, 34.0 * size, pal.sun_hi, pal.sun);
        let a = c.centred(at.0, at.1, (d.w, d.h));
        out.push(Part { raster: d, at: a, alpha: 1.0, rest_alpha: 1.0, motion: drift(3.0 * size, 2.5 * size), clip: None });
    };
    let moon = |c: &Ctx, out: &mut Vec<Part>, at: (f32, f32), size: f32| {
        let g = glow(c, 170.0 * size, pal.moon_glow, if light { 0.22 } else { 0.26 });
        let a = c.centred(at.0, at.1, (g.w, g.h));
        out.push(Part { raster: g, at: a, alpha: 0.9, rest_alpha: 0.9, motion: Motion { scale: frac(0.04, 11.0, 0.0), alpha: frac(0.1, 11.0, 0.0), ..drift(2.5 * size, 2.0 * size) }, clip: None });
        let m = crescent(c, 38.0 * size);
        let a = c.centred(at.0, at.1, (m.w, m.h));
        out.push(Part { raster: m, at: a, alpha: 1.0, rest_alpha: 1.0, motion: drift(2.5 * size, 2.0 * size), clip: None });
    };
    let sky_stars = |c: &Ctx, out: &mut Vec<Part>, groups: &[&[(f32, f32, f32)]]| {
        for (i, g) in groups.iter().enumerate() {
            let r = stars(c, g);
            let a = c.centred(UNITS / 2.0, UNITS / 2.0, (r.w, r.h));
            let periods = [5.3f32, 6.7, 8.1];
            let m = Motion { alpha: frac(0.38, periods[i % 3], 0.31 * i as f32), scale: frac(0.03, periods[i % 3], 0.31 * i as f32), ..Motion::STILL };
            out.push(Part { raster: r, at: a, alpha: 0.62, rest_alpha: 0.62, motion: m, clip: None });
        }
    };
    let (cloud_hi, cloud_lo, cloud_rim) = (pal.cloud_hi, pal.cloud_lo, pal.cloud_rim);
    let storm = matches!(cond.sky, Sky::Storm) || (matches!(cond.sky, Sky::Rain) && cond.heavy == 2);
    let (ch, cl) = if storm { (pal.storm_hi, pal.storm_lo) } else { (cloud_hi, cloud_lo) };
    let float = |ax: f32, px: f32, ph: f32, ay: f32, py: f32| Motion { x: sine(ax, px, ph), y: sine(ay, py, ph + 0.3), ..Motion::STILL };

    if cond.wind {
        // behind everything: the streaks show in the clear sky between the clouds
        for (i, (y, len, period)) in [(44.0f32, 110.0f32, 7_200u32), (150.0, 90.0, 9_600), (96.0, 100.0, 8_400)].into_iter().enumerate() {
            if cond.sky == Sky::Storm && i == 2 {
                continue;
            }
            let colour = if light { pal.fog } else { pal.snow };
            let r = wind(&c, len, colour);
            let a = c.centred(0.0, y, (r.w, r.h));
            let travel = (UNITS + len) * k;
            let m = Motion { x: Wave::Saw { from: -(len * 0.5) * k, to: travel - len * 0.5 * k, period_ms: period }, bump: true, ..Motion::STILL };
            out.push(Part { raster: r, at: (a.0, a.1), alpha: 0.6, rest_alpha: 0.6, motion: m, clip: Some((Rect { x: 0.0, y: 0.0, w: side, h: side }, 14.0 * k)) });
        }
    }
    match cond.sky {
        Sky::Clear if cond.day => sun(&c, &mut out, (100.0, 100.0), 1.0),
        Sky::Clear => {
            sky_stars(&c, &mut out, &[&[(34.0, 44.0, 6.0), (166.0, 30.0, 5.0), (172.0, 126.0, 4.0), (40.0, 158.0, 3.5)], &[(66.0, 20.0, 4.0), (150.0, 74.0, 3.5), (22.0, 100.0, 4.5), (150.0, 178.0, 3.0)], &[(182.0, 168.0, 3.5), (118.0, 178.0, 3.0), (84.0, 176.0, 4.0), (118.0, 22.0, 3.0)]]);
            moon(&c, &mut out, (100.0, 100.0), 1.0);
        }
        Sky::Partly => {
            if cond.day {
                sun(&c, &mut out, (72.0, 70.0), 0.72);
            } else {
                sky_stars(&c, &mut out, &[&[(150.0, 36.0, 5.0), (28.0, 130.0, 4.0), (170.0, 104.0, 3.5)], &[(104.0, 20.0, 3.5), (22.0, 64.0, 4.0)]]);
                moon(&c, &mut out, (72.0, 70.0), 0.72);
            }
            if cond.heavy >= 1 {
                add(&mut out, cloud(&c, 0.62, cloud_hi, cloud_lo, cloud_rim), (156.0, 78.0), 0.85, float(7.0, 37.0, 0.4, 1.5, 11.0));
            }
            let s = if cond.heavy == 0 { 0.82 } else { 1.0 };
            add(&mut out, cloud(&c, s, cloud_hi, cloud_lo, cloud_rim), (112.0, 124.0), 1.0, float(9.0, 31.0, 0.0, 2.0, 13.0));
        }
        Sky::Cloudy => {
            add(&mut out, cloud(&c, 1.0, pal.storm_hi, pal.storm_lo, cloud_rim * 0.6), (62.0, 80.0), 0.78, float(12.0, 43.0, 0.5, 1.5, 17.0));
            add(&mut out, cloud(&c, 1.15, cloud_hi, cloud_lo, cloud_rim), (134.0, 98.0), 0.92, float(-10.0, 34.0, 0.2, 2.0, 12.0));
            add(&mut out, cloud(&c, 1.28, cloud_hi, cloud_lo, cloud_rim), (94.0, 136.0), 1.0, float(14.0, 27.0, 0.0, 2.0, 10.0));
        }
        Sky::Fog => {
            add(&mut out, cloud(&c, 0.9, pal.storm_hi, pal.storm_lo, cloud_rim * 0.5), (100.0, 58.0), 0.7, float(8.0, 39.0, 0.0, 1.5, 15.0));
            for (i, (y, w, a)) in [(104.0f32, 150.0f32, 0.5f32), (128.0, 168.0, 0.6), (152.0, 150.0, 0.5), (176.0, 130.0, 0.4)].into_iter().enumerate() {
                let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                let periods = [17.0f32, 21.0, 15.0, 19.0];
                add(&mut out, haze(&c, w, 11.0, pal.fog), (100.0, y), a, Motion { x: sine(side * 16.0, periods[i], 0.2 * i as f32), alpha: frac(0.12, periods[i] * 0.6, 0.3 * i as f32), ..Motion::STILL });
            }
        }
        Sky::Drizzle | Sky::Rain | Sky::Storm | Sky::Snow => {
            let snow = cond.sky == Sky::Snow;
            let (cy, scale) = match cond.sky {
                Sky::Drizzle => (62.0, 1.05),
                Sky::Snow => (60.0, 1.1),
                _ => (60.0, 1.2),
            };
            let (win_x, win_y, win_w, win_h) = (40.0f32, 92.0f32, 120.0f32, 100.0f32);
            if cond.sky == Sky::Storm {
                // the flash lights the sky behind everything: a wide soft glow, then the bolt between the clouds and the rain
                let g = glow(&c, 200.0, pal.flash, 0.42);
                let a = c.centred(100.0, 100.0, (g.w, g.h));
                let (period, at) = if cond.heavy == 2 { (6_000, 1_200) } else { (8_000, 1_500) };
                out.push(Part { raster: g, at: a, alpha: 0.0, rest_alpha: 0.0, motion: Motion { alpha: Wave::Pulse { peak: 0.9, period_ms: period, at_ms: at, width_ms: 1_200 }, ..Motion::STILL }, clip: None });
            }
            if cond.sky == Sky::Rain && cond.heavy == 2 || cond.sky == Sky::Storm {
                add(&mut out, cloud(&c, 0.9, pal.storm_hi, pal.storm_lo, cloud_rim * 0.5), (142.0, 52.0), 0.85, float(-8.0, 36.0, 0.5, 1.5, 12.0));
            } else if cond.heavy >= 1 && !snow {
                add(&mut out, cloud(&c, 0.8, cloud_hi, cloud_lo, cloud_rim), (150.0, 54.0), 0.8, float(-8.0, 36.0, 0.5, 1.5, 12.0));
            }
            // the sheets go behind the cloud
            let wp = win_w * k;
            let (hp, top) = (win_h * k, win_y * k);
            let (wpx, window) = (wp.ceil() as u32, Rect { x: (win_x * k).round(), y: top.round(), w: wp.round(), h: hp.round() });
            let mut sheets: Vec<(Sheet, u32, f32)> = Vec::new();
            let colour = if snow { pal.snow } else { pal.rain };
            match (cond.sky, cond.heavy) {
                (Sky::Snow, h) => {
                    let n = [9usize, 13, 20][h as usize];
                    sheets.push((Sheet { width: wpx, height: 0, period: 52.0 * k, streaks: n, len: (4.5 * k, 8.5 * k), thick: 1.1 * k.max(0.8), lean: 0.0, colour, seed: 11, snow: true }, 6_000, 0.95));
                    sheets.push((Sheet { width: wpx, height: 0, period: 52.0 * k, streaks: n + 5, len: (2.2 * k, 4.2 * k), thick: 0.8, lean: 0.0, colour, seed: 29, snow: true }, 8_400, 0.6));
                }
                (Sky::Drizzle, h) => {
                    sheets.push((Sheet { width: wpx, height: 0, period: 40.0 * k, streaks: 6 + 2 * h as usize, len: (6.0 * k, 9.0 * k), thick: 1.3 * k.max(0.8), lean: 3.0, colour, seed: 3, snow: false }, 2_000, 0.6));
                }
                (_, h) => {
                    let n = [5usize, 7, 10][h as usize];
                    sheets.push((Sheet { width: wpx, height: 0, period: 48.0 * k, streaks: n + 3, len: (8.0 * k, 13.0 * k), thick: 1.2 * k.max(0.8), lean: 4.0, colour, seed: 5, snow: false }, 1_600, 0.5));
                    sheets.push((Sheet { width: wpx, height: 0, period: 48.0 * k, streaks: n, len: (11.0 * k, 17.0 * k), thick: 1.9 * k.max(0.9), lean: 4.0, colour, seed: 17, snow: false }, 1_200, 0.85));
                    if h == 2 {
                        sheets.push((Sheet { width: wpx, height: 0, period: 48.0 * k, streaks: n, len: (9.0 * k, 14.0 * k), thick: 1.5 * k.max(0.8), lean: 4.0, colour, seed: 41, snow: false }, 2_000, 0.65));
                    }
                }
            }
            for (mut s, period_ms, alpha) in sheets {
                // the sheet covers the window at every offset of the slide: the window, a period more, and what the lead of an animation runs on
                let travel = s.period * (1.0 + LEAD_MS as f32 / period_ms as f32);
                s.height = (hp + travel).ceil() as u32 + 2;
                let r = sheet(&s);
                let x = window.x;
                let motion = Motion { y: Wave::Saw { from: 0.0, to: s.period, period_ms }, x: if snow { sine(4.0, 5.1 + alpha, 0.0) } else { Wave::Zero }, ..Motion::STILL };
                let base_y = top.round() - (s.height as f32 - hp.round());
                out.push(Part { raster: r, at: (x, base_y), alpha, rest_alpha: alpha, motion, clip: Some((window, 12.0 * k)) });
            }
            if cond.sky == Sky::Storm {
                let b = bolt(&c, 62.0);
                let (period, at) = if cond.heavy == 2 { (6_000, 1_200) } else { (8_000, 1_500) };
                let a = c.centred(104.0, 112.0, (b.w, b.h));
                out.push(Part { raster: b, at: a, alpha: 0.0, rest_alpha: 0.72, motion: Motion { alpha: Wave::Pulse { peak: 1.0, period_ms: period, at_ms: at, width_ms: 1_200 }, ..Motion::STILL }, clip: None });
            }
            add(&mut out, cloud(&c, scale, ch, cl, cloud_rim * if storm { 0.5 } else { 1.0 }), (94.0, cy), 1.0, float(10.0, 29.0, 0.0, 2.0, 11.0));
        }
    }
    out
}

/// Whether anything in `parts` moves.
pub fn moves(parts: &[Part]) -> bool {
    parts.iter().any(|p| p.motion != Motion::STILL)
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
        let (sw, sh) = (part.raster.w as f32 * pose.scale, part.raster.h as f32 * pose.scale);
        let (cx, cy) = (pose.x + part.raster.w as f32 / 2.0, pose.y + part.raster.h as f32 / 2.0);
        let (left, top) = (cx - sw / 2.0, cy - sh / 2.0);
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let (u, v) = ((fx - left) / pose.scale, (fy - top) / pose.scale);
                if u < 0.0 || v < 0.0 || u >= part.raster.w as f32 || v >= part.raster.h as f32 {
                    continue;
                }
                let mut fade = 1.0;
                if let Some((r, feather)) = part.clip {
                    if fx < r.x || fx >= r.x + r.w || fy < r.y || fy >= r.y + r.h {
                        continue;
                    }
                    fade = ((fx - r.x) / feather).min((r.x + r.w - fx) / feather).min((r.y + r.h - fy) / (feather * 1.6)).clamp(0.0, 1.0);
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
    pub fn new(kit: &mut Kit, cond: Cond, side: f32, light: bool, origin: (f32, f32), z: i32) -> std::io::Result<Live> {
        let parts = parts(cond, side, light);
        let mut layers = Vec::with_capacity(parts.len());
        for p in &parts {
            let rest = p.rest_pose();
            let mut l = kit.ui.scene.image(p.raster.w, p.raster.h, z, p.props(&rest, origin))?;
            l.pixels_mut().copy_from_slice(&p.raster.px);
            l.damage(None);
            layers.push(l);
        }
        Ok(Live { cond, light, parts, layers, origin, running: false, last: 0 })
    }

    pub fn is(&self, cond: Cond, light: bool) -> bool {
        self.cond == cond && self.light == light
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
    fn every_condition_is_drawn_inside_its_box_on_whole_pixels_at_rest() {
        for side in [96.0f32, 164.0, 200.0, 328.0] {
            for cond in every() {
                let p = parts(cond, side, cond.day);
                assert!(!p.is_empty(), "{cond:?}");
                for part in &p {
                    let rest = part.rest_pose();
                    assert_eq!((rest.x.fract(), rest.y.fract()), (0.0, 0.0), "{cond:?} at {side}: a sprite at rest is on whole pixels");
                    assert!(part.raster.px.chunks_exact(4).any(|q| q[3] > 0), "{cond:?}: a sprite that draws nothing");
                    assert!(part.raster.px.chunks_exact(4).all(|q| q[0] <= q[3] && q[1] <= q[3] && q[2] <= q[3]), "{cond:?}: premultiplied");
                    if part.clip.is_none() {
                        let slack = 0.12 * side;
                        assert!(rest.x >= -slack && rest.y >= -slack && rest.x + part.raster.w as f32 <= side + slack && rest.y + part.raster.h as f32 <= side + slack, "{cond:?} at {side}: a sprite outside its box: {rest:?} {}x{}", part.raster.w, part.raster.h);
                    } else if let Some((r, _)) = part.clip {
                        assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= side + 0.5 && r.y + r.h <= side + 0.5, "{cond:?}: a clip outside the box {r:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_picture_made_for_a_look_is_not_a_scaled_bitmap() {
        // the same condition at twice the side has twice the sprites' sides: it is drawn again at the size, not stretched
        let a = parts(Cond { sky: Sky::Cloudy, day: true, heavy: 1, wind: false }, 100.0, false);
        let b = parts(Cond { sky: Sky::Cloudy, day: true, heavy: 1, wind: false }, 200.0, false);
        for (x, y) in a.iter().zip(&b) {
            assert!((y.raster.w as i32 - 2 * x.raster.w as i32).abs() <= 2, "{} vs {}", x.raster.w, y.raster.w);
        }
    }

    #[test]
    fn the_sheets_of_rain_and_snow_repeat_every_period_so_putting_one_back_is_not_seen() {
        for sky in [Sky::Rain, Sky::Snow, Sky::Drizzle] {
            let p = parts(Cond { sky, day: true, heavy: 1, wind: false }, 200.0, false);
            let sheets: Vec<&Part> = p.iter().filter(|q| matches!(q.motion.y, Wave::Saw { .. })).collect();
            assert!(!sheets.is_empty(), "{sky:?} has sheets");
            for s in sheets {
                let Wave::Saw { to, period_ms, .. } = s.motion.y else { unreachable!() };
                let period = to.round() as usize;
                assert!(period_ms as u64 % TICK_MS == 0, "a sheet's period is whole ticks, so it is put back on a tick");
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
            }
        }
    }

    #[test]
    fn the_paths_are_continuous_across_a_tick_and_a_wrap_is_a_whole_period() {
        let p = parts(Cond { sky: Sky::Rain, day: true, heavy: 2, wind: true }, 164.0, false);
        for part in &p {
            for t in (0..40_000u64).step_by(TICK_MS as usize) {
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
                // and a move between two ticks is small: no sprite jumps by more than a few pixels in a lead (a sheet's whole period excepted)
                let now = part.pose(t as f64, None);
                if part.motion.y.period().is_none() && part.motion.x.period().is_none() {
                    assert!((a.x - now.x).abs() < 6.0 && (a.y - now.y).abs() < 6.0, "t={t}: a jump of {} {}", a.x - now.x, a.y - now.y);
                }
            }
        }
    }

    #[test]
    fn lightning_is_a_soft_pulse_that_never_strobes() {
        let p = parts(Cond { sky: Sky::Storm, day: true, heavy: 2, wind: false }, 164.0, false);
        let flash: Vec<&Part> = p.iter().filter(|q| matches!(q.motion.alpha, Wave::Pulse { .. })).collect();
        assert_eq!(flash.len(), 2, "the glow and the bolt");
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
        let p = parts(Cond { sky: Sky::Storm, day: true, heavy: 1, wind: false }, 164.0, false);
        assert!(p.iter().any(|q| q.rest_alpha > 0.5 && matches!(q.motion.alpha, Wave::Pulse { .. })));
        assert!(moves(&p));
        assert!(!moves(&[]));
    }

    /// `cargo test --release -p lumatv-ui --lib weather_art::tests::cost -- --ignored --nocapture`: what a picture costs to make and to move.
    #[test]
    #[ignore]
    fn cost() {
        use std::time::Instant;
        for (name, cond) in [
            ("clear-day", Cond { sky: Sky::Clear, day: true, heavy: 0, wind: false }),
            ("partly", Cond { sky: Sky::Partly, day: true, heavy: 1, wind: false }),
            ("rain", Cond { sky: Sky::Rain, day: true, heavy: 1, wind: false }),
            ("storm", Cond { sky: Sky::Storm, day: true, heavy: 2, wind: true }),
            ("snow", Cond { sky: Sky::Snow, day: true, heavy: 2, wind: false }),
        ] {
            let t = Instant::now();
            let p = parts(cond, 164.0, false);
            let make = t.elapsed();
            let bytes: usize = p.iter().map(|q| q.raster.px.len()).sum();
            let (_h, mut kit) = crate::kit::testkit::kit(1.0);
            let t = Instant::now();
            let mut live = Live::new(&mut kit, cond, 164.0, false, (32.0, 28.0), 13).unwrap();
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
            println!("{name}: {} sprites ({} moving), {} KiB of pixels; sprites made in {make:?}, layers made in {build:?}; one tick {per:?} (2 ticks a second)", p.len(), moving, bytes / 1024);
        }
    }
}
