//! The pixels of the weather pictures' sprites: every shape is a signed-distance function evaluated per pixel of a small raster (analytic
//! anti-aliasing, a gradient with a dither so a dark ramp does not band), made once for the box and look it is shown in. The compositor
//! moves the sprites (`weather_art.rs`); nothing here runs on a frame.
//!
//! Shapes are measured in design units (`Ctx::k` pixels each), so one description serves the card's small sky and the view's large one. A
//! soft shape (a glow, a far cloud) may be made at a fraction of its size (`Ctx::low`) and shown scaled up by the compositor: the gradient
//! has no detail to lose and the memory falls with the square of the fraction.

use std::f32::consts::{PI, TAU};

pub type Rgb = [f32; 3];

pub fn hex(v: u32) -> Rgb {
    [((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0]
}

/// The colours (Rosé Pine: Main for the dark look, Dawn for the light; modest, for an OLED panel).
#[derive(Clone, Copy)]
pub struct Pal {
    pub sun: Rgb,
    pub sun_hi: Rgb,
    pub sun_glow: Rgb,
    /// The wide warm bloom round the sun.
    pub bloom: Rgb,
    pub moon: Rgb,
    pub moon_hi: Rgb,
    pub moon_glow: Rgb,
    pub cloud_hi: Rgb,
    pub cloud_lo: Rgb,
    pub cloud_rim: f32,
    pub storm_hi: Rgb,
    pub storm_lo: Rgb,
    pub rain: Rgb,
    pub snow: Rgb,
    pub fog: Rgb,
    pub bolt: Rgb,
    pub star: Rgb,
    pub flash: Rgb,
    pub wind: Rgb,
}

impl Pal {
    pub fn of(light: bool) -> Pal {
        if light {
            Pal {
                sun: hex(0xEA9D34),
                sun_hi: hex(0xF6C177),
                sun_glow: hex(0xEA9D34),
                bloom: hex(0xD7827E),
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
                wind: hex(0x797593),
            }
        } else {
            Pal {
                sun: hex(0xF6C177),
                sun_hi: hex(0xFFE3B0),
                sun_glow: hex(0xF6C177),
                bloom: hex(0xEB6F92),
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
                wind: hex(0xC4C0DC),
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

pub struct Acc {
    w: usize,
    h: usize,
    a: Vec<[f32; 4]>,
}

pub fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn mix3(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [mix(a[0], b[0], t), mix(a[1], b[1], t), mix(a[2], b[2], t)]
}

/// Coverage of a pixel whose centre is `d` pixels outside the edge.
pub fn cov(d: f32) -> f32 {
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
    pub fn new(w: u32, h: u32) -> Acc {
        Acc { w: w as usize, h: h as usize, a: vec![[0.0; 4]; (w * h) as usize] }
    }

    /// Composites `f(x, y)` (a straight colour and a coverage, at the pixel's centre) over every pixel of the box `(x0, y0)..(x1, y1)`.
    pub fn paint(&mut self, bounds: (f32, f32, f32, f32), f: impl Fn(f32, f32) -> Option<(Rgb, f32)>) {
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

    pub fn whole(&self) -> (f32, f32, f32, f32) {
        (0.0, 0.0, self.w as f32, self.h as f32)
    }

    pub fn finish(self, opacity: f32) -> Raster {
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

// ---- the context of a sprite
pub struct Ctx {
    pub light: bool,
    /// Pixels per design unit.
    pub k: f32,
    pub pal: Pal,
}

impl Ctx {
    pub fn px(&self, units: f32) -> f32 {
        units * self.k
    }

    pub fn size(&self, w: f32, h: f32) -> (u32, u32) {
        ((w * self.k).ceil().max(1.0) as u32, (h * self.k).ceil().max(1.0) as u32)
    }

    /// The same look with every unit `div` times fewer pixels: for a soft shape that the compositor shows scaled up by `div`.
    pub fn low(&self, div: f32) -> Ctx {
        Ctx { light: self.light, k: self.k / div, pal: self.pal }
    }
}

/// A soft radial glow: `colour` at `alpha` in the middle, nothing at the edge of the square of `side_u` units.
pub fn glow(c: &Ctx, side_u: f32, colour: Rgb, alpha: f32) -> Raster {
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

/// A soft elliptical glow of `w_u` by `h_u` units (the sky a flash of lightning lights).
pub fn glow_ellipse(c: &Ctx, w_u: f32, h_u: f32, colour: Rgb, alpha: f32) -> Raster {
    let (w, h) = c.size(w_u, h_u);
    let mut acc = Acc::new(w, h);
    let (rx, ry) = (w as f32 / 2.0, h as f32 / 2.0);
    acc.paint(acc.whole(), |x, y| {
        let d = (((x - rx) / rx).powi(2) + ((y - ry) / ry).powi(2)).sqrt();
        Some((colour, (1.0 - smooth(0.0, 1.0, d)).powi(2) * alpha))
    });
    acc.finish(1.0)
}

pub fn disc(c: &Ctx, r_u: f32, hi: Rgb, lo: Rgb) -> Raster {
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

/// One ray of the sun from `r0` to `r1` units out, `width` units wide at the root and thinner at the tip, fading out towards the tip.
fn ray(acc: &mut Acc, c: &Ctx, (mx, my): (f32, f32), angle: f32, (r0, r1): (f32, f32), width: f32, colour: Rgb, alpha: f32) {
    let wid = c.px(width) / 2.0;
    let (dx, dy) = (angle.cos(), angle.sin());
    let p0 = (mx + dx * c.px(r0), my + dy * c.px(r0));
    let p1 = (mx + dx * c.px(r1), my + dy * c.px(r1));
    acc.paint((p0.0.min(p1.0) - wid - 2.0, p0.1.min(p1.1) - wid - 2.0, p0.0.max(p1.0) + wid + 2.0, p0.1.max(p1.1) + wid + 2.0), |x, y| {
        let (d, t) = seg(x, y, p0, p1);
        // tapering: the tip is thinner than the root
        let wt = wid * (1.0 - 0.45 * t);
        Some((colour, cov(d - wt) * alpha * (1.0 - 0.55 * t)))
    });
}

/// The ring of the sun's twelve rays, turned `turn` radians: long and short ones alternating, so the ring repeats every sixth of a turn.
pub fn sun_rays(c: &Ctx, turn: f32, size: f32, colour: Rgb, alpha: f32) -> Raster {
    let out = 84.0 * size;
    let (w, h) = c.size(2.0 * out + 12.0, 2.0 * out + 12.0);
    let mut acc = Acc::new(w, h);
    let m = (w as f32 / 2.0, h as f32 / 2.0);
    let wscale = size.max(0.8) * 1.2;
    for i in 0..12 {
        let a = turn + TAU * i as f32 / 12.0;
        let (r, wd) = if i % 2 == 0 { ((52.0, 80.0), 5.4) } else { ((50.0, 68.0), 6.4) };
        ray(&mut acc, c, m, a, (r.0 * size, r.1 * size), wd * wscale, colour, alpha);
    }
    acc.finish(1.0)
}

/// A cloud of one of three silhouettes (0 the canonical cumulus about 104 by 72 units at scale 1, 1 a wide low one, 2 a small round one).
pub fn cloud(c: &Ctx, kind: u8, scale: f32, hi: Rgb, lo: Rgb, rim: f32) -> Raster {
    // (puffs: x, y, radius), the base (centre y, half width, half height), the box (w, h) and where the shape's origin is in it
    type Puffs = &'static [(f32, f32, f32)];
    let (puffs, base, bx, by, (ox, oy)): (Puffs, (f32, f32, f32), f32, f32, (f32, f32)) = match kind {
        1 => (&[(-40.0, 4.0, 15.0), (-17.0, -8.0, 23.0), (11.0, -4.0, 20.0), (37.0, 5.0, 15.0)], (13.0, 56.0, 13.0), 150.0, 84.0, (75.0, 46.0)),
        2 => (&[(-15.0, 3.0, 17.0), (9.0, -8.0, 22.0)], (12.0, 30.0, 12.0), 84.0, 66.0, (42.0, 38.0)),
        _ => (&[(-27.0, 0.0, 21.0), (3.0, -12.0, 29.0), (33.0, 3.0, 19.0)], (14.0, 50.0, 17.0), 120.0, 90.0, (60.0, 50.0)),
    };
    let s = c.k * scale;
    let (w, h) = c.size(bx * scale, by * scale);
    let mut acc = Acc::new(w, h);
    let (ox, oy) = (ox * s, oy * s);
    let sdf = move |x: f32, y: f32| -> f32 {
        let (x, y) = (x - ox, y - oy);
        let k = 7.0 * s;
        let mut d = sd_box(x, y, 0.0, base.0 * s, base.1 * s, base.2 * s, base.2 * s);
        for &(px, py, pr) in puffs {
            d = smin(d, sd_circle(x, y, px * s, py * s, pr * s), k);
        }
        d
    };
    let top = oy + puffs.iter().map(|p| p.1 - p.2).fold(f32::MAX, f32::min) * s;
    let bottom = oy + (base.0 + base.2) * s;
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

/// Sparkles at (x, y, radius) in design units over a square of `side_u`.
pub fn stars(c: &Ctx, side_u: (f32, f32), list: &[(f32, f32, f32)]) -> Raster {
    let (w, h) = c.size(side_u.0, side_u.1);
    let mut acc = Acc::new(w, h);
    for &(x, y, r) in list {
        sparkle(&mut acc, c, x, y, r, c.pal.star, 1.0);
    }
    acc.finish(1.0)
}

/// One star on its own: the sprite is `4 r` units square.
pub fn star(c: &Ctx, r_u: f32) -> Raster {
    let (w, h) = c.size(2.0 * r_u + 6.0, 2.0 * r_u + 6.0);
    let mut acc = Acc::new(w, h);
    sparkle(&mut acc, c, r_u + 3.0, r_u + 3.0, r_u, c.pal.star, 1.0);
    acc.finish(1.0)
}

/// The crescent: a disc with a bite out of the upper right.
pub fn crescent(c: &Ctx, r_u: f32) -> Raster {
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

/// A soft horizontal band of haze `w_u` long and about `h_u` units thick: a bell across and thinning out at both ends, with no edge to it.
pub fn haze(c: &Ctx, w_u: f32, h_u: f32, colour: Rgb) -> Raster {
    let (w, h) = c.size(w_u, h_u * 2.6);
    let mut acc = Acc::new(w, h);
    let sigma = c.px(h_u) * 0.5;
    acc.paint(acc.whole(), |x, y| {
        let dy = (y - h as f32 / 2.0) / sigma;
        let u = x / w as f32;
        let ends = smooth(0.0, 0.32, u) * smooth(0.0, 0.32, 1.0 - u);
        Some((colour, (-0.5 * dy * dy).exp() * ends))
    });
    acc.finish(1.0)
}

/// A snow flake of radius `r` pixels at (`cx`, `cy`): a dot while small, a six-armed crystal from 2.4 px, and with `soft` pixels of edge a
/// soft disc (a flake out of focus, near the eye).
fn flake(acc: &mut Acc, (cx, cy): (f32, f32), r: f32, thick: f32, soft: f32, colour: Rgb, alpha: f32) {
    let pad = r + soft + 2.0;
    acc.paint((cx - pad, cy - pad, cx + pad, cy + pad), |px, py| {
        if soft > 0.0 {
            let d = sd_circle(px, py, cx, cy, r);
            return Some((colour, (1.0 - smooth(-soft, 0.5, d)) * alpha));
        }
        if r < 2.4 {
            return Some((colour, cov(sd_circle(px, py, cx, cy, r.max(1.0))) * alpha));
        }
        let mut d = f32::MAX;
        for a in 0..3 {
            let ang = a as f32 * PI / 3.0;
            let (dx, dy) = (ang.cos() * r, ang.sin() * r);
            d = d.min(seg(px, py, (cx - dx, cy - dy), (cx + dx, cy + dy)).0 - thick * 0.5);
        }
        Some((colour, cov(d).max(cov(sd_circle(px, py, cx, cy, thick * 0.9))) * alpha))
    });
}

/// One flake on its own (`r_u` units of radius), for the flakes near the eye that drift on paths of their own.
pub fn flake_sprite(c: &Ctx, r_u: f32, soft_u: f32, colour: Rgb) -> Raster {
    let r = c.px(r_u);
    let soft = c.px(soft_u);
    let side = (2.0 * (r + soft) + 6.0).ceil() as u32;
    let mut acc = Acc::new(side, side);
    flake(&mut acc, (side as f32 / 2.0, side as f32 / 2.0), r, (c.k * 1.1).max(0.9), soft, colour, 1.0);
    acc.finish(1.0)
}

/// A tall sheet of streaks whose pattern repeats every `period` px, `streaks` of them in `width` px, each `len` px long leaning `lean` px
/// over its length. `snow` draws flakes instead (`len` is then the flake's diameter, `soft` the edge of the biggest). The sheet is `height` px.
pub struct Sheet {
    pub width: u32,
    pub height: u32,
    pub period: f32,
    pub streaks: usize,
    pub len: (f32, f32),
    pub thick: f32,
    pub lean: f32,
    pub colour: Rgb,
    pub seed: u32,
    pub snow: bool,
    pub soft: f32,
}

pub fn hash(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

pub fn sheet(s: &Sheet) -> Raster {
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
                // the bigger the flake, the softer: the near ones are out of focus
                let soft = s.soft * smooth(0.5, 1.0, (len - s.len.0) / (s.len.1 - s.len.0).max(0.01));
                flake(&mut acc, (x, y0), len * 0.5, s.thick, soft, s.colour, 1.0);
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

/// One streak of rain near the eye, `len_u` long, leaning `lean_u` to the left over its length: the sprite is as wide as the lean and the thickness.
pub fn streak(c: &Ctx, len_u: f32, thick_u: f32, lean_u: f32, colour: Rgb) -> Raster {
    let (len, thick, lean) = (c.px(len_u), c.px(thick_u).max(1.0), c.px(lean_u));
    let (w, h) = ((lean + thick + 6.0).ceil() as u32, (len + thick + 4.0).ceil() as u32);
    let mut acc = Acc::new(w, h);
    let (a, b) = ((w as f32 - thick / 2.0 - 3.0, 2.0), (3.0 + thick / 2.0, len + 2.0));
    acc.paint(acc.whole(), |x, y| {
        let (d, t) = seg(x, y, a, b);
        Some((colour, cov(d - thick * 0.5 * (0.5 + 0.5 * t)) * t.powf(0.7)))
    });
    acc.finish(1.0)
}

/// The ring a drop makes where it lands: an ellipse `w_u` wide and a third as high, with a faint fill inside it.
pub fn ripple(c: &Ctx, w_u: f32, colour: Rgb) -> Raster {
    let (w, h) = c.size(w_u + 6.0, w_u * 0.36 + 6.0);
    let mut acc = Acc::new(w, h);
    let (mx, my) = (w as f32 / 2.0, h as f32 / 2.0);
    let (rx, ry) = (c.px(w_u) / 2.0, c.px(w_u) * 0.18);
    let stroke = (c.k * 1.3).max(1.0);
    acc.paint(acc.whole(), |x, y| {
        let e = (((x - mx) / rx).powi(2) + ((y - my) / ry).powi(2)).sqrt();
        let d = (e - 1.0) * ry.min(rx) * 0.9;
        let ring = cov(d.abs() - stroke / 2.0);
        let inside = if e < 1.0 { 0.16 * (1.0 - e) } else { 0.0 };
        Some((colour, (ring * 0.9).max(inside)))
    });
    acc.finish(1.0)
}

/// A streak of wind drawn as a long thin wave with a head that curls, `len_u` long and growing from nothing at its tail.
pub fn wind(c: &Ctx, len_u: f32, curl_u: f32, wave_u: f32, colour: Rgb) -> Raster {
    let (w, h) = c.size(len_u + 2.0 * curl_u + 8.0, 2.0 * (curl_u + wave_u) + 10.0);
    let mut acc = Acc::new(w, h);
    let k = c.k;
    let r = curl_u * k;
    let y0 = h as f32 - wave_u * k - 5.0 * k;
    let x1 = w as f32 - r - 6.0 * k;
    let tail = 2.0 * k;
    let mut path: Vec<(f32, f32)> = Vec::new();
    // the body: a slow wave, taller towards the head
    let n = 40;
    for i in 0..=n {
        let u = i as f32 / n as f32;
        let x = tail + (x1 - tail) * u;
        path.push((x, y0 - wave_u * k * (u * 2.4 * PI).sin() * (0.3 + 0.7 * u)));
    }
    let (hx, hy) = *path.last().unwrap_or(&(x1, y0));
    // a curl up and back at the head: a circle that shrinks a little as it goes round
    for i in 1..=18 {
        let th = PI * 1.45 * i as f32 / 18.0;
        let rr = r * (1.0 - 0.28 * i as f32 / 18.0);
        path.push((hx + rr * th.sin(), hy - r + rr * th.cos()));
    }
    let width = (0.8 * k).max(0.9);
    let body = n + 1;
    acc.paint(acc.whole(), |x, y| {
        let mut d = f32::MAX;
        let mut at = 1.0;
        for i in 1..path.len() {
            let (dd, t) = seg(x, y, path[i - 1], path[i]);
            if dd < d {
                d = dd;
                at = if i < body { ((i - 1) as f32 + t) / n as f32 } else { 1.0 };
            }
        }
        // the line grows from nothing at its tail
        Some((colour, cov(d - width) * smooth(0.0, 0.55, at.clamp(0.0, 1.0)) * 0.95))
    });
    acc.finish(1.0)
}

/// A shooting star: a streak `len_u` long at `angle` radians below the horizontal, thin and faint at its tail and bright at its head.
pub fn shooting_star(c: &Ctx, len_u: f32, angle: f32, colour: Rgb) -> Raster {
    let (len, k) = (c.px(len_u), c.k);
    let (w, h) = ((len * angle.cos()).ceil() as u32 + 10, (len * angle.sin()).ceil() as u32 + 10);
    let mut acc = Acc::new(w, h);
    let (a, b) = ((5.0, 5.0), (w as f32 - 5.0, h as f32 - 5.0));
    let head = (c.px(1.7)).max(1.2);
    acc.paint(acc.whole(), |x, y| {
        let (d, t) = seg(x, y, a, b);
        let line = cov(d - (0.25 + 0.55 * t) * k.max(0.8)) * t.powf(1.6);
        let glow = (1.0 - smooth(0.0, head * 3.0, sd_circle(x, y, b.0, b.1, 0.0))) * 0.5;
        let core = cov(sd_circle(x, y, b.0, b.1, head));
        Some((colour, line.max(glow).max(core)))
    });
    acc.finish(1.0)
}

/// A bolt of lightning `h_u` units high; `flip` mirrors it.
pub fn bolt(c: &Ctx, h_u: f32, flip: bool) -> Raster {
    let s = c.px(h_u) / 70.0;
    let (w, h) = ((30.0 * s).ceil() as u32 + 8, (70.0 * s).ceil() as u32 + 8);
    let mut acc = Acc::new(w, h);
    let (mx, my) = (w as f32 / 2.0, h as f32 / 2.0);
    let sx = if flip { -1.0 } else { 1.0 };
    let p: Vec<(f32, f32)> = [(8.0, -35.0), (-14.0, 4.0), (-1.0, 4.0), (-9.0, 35.0), (16.0, -8.0), (3.0, -8.0), (14.0, -35.0)].iter().map(|&(x, y)| (mx + sx * x * s, my + y * s)).collect();
    let (hi, lo) = ([1.0, 0.97, 0.85], c.pal.bolt);
    acc.paint(acc.whole(), |x, y| {
        let d = sd_poly(x, y, &p) - 0.6 * s;
        Some((mix3(hi, lo, ((y - my) / (35.0 * s) * 0.5 + 0.5).clamp(0.0, 1.0)), cov(d)))
    });
    acc.finish(1.0)
}
