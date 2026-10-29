//! The saver's analog clock, rastered on the CPU into premultiplied RGBA (the compositor draws pictures, not rotated shapes): a hairline ring
//! and twelve ticks that never change, an hour and a minute hand with a hub, and a thin second hand. Each stroke has a halo of its own that is
//! a hint of light (alpha at most [`GLOW`] of the cap), so the hands have a soft edge on black and no more.
//!
//! Every colour passed in is already at or under the saver's cap, strokes are drawn with "over" (a convex mix of values at or under the
//! cap), and so no pixel of any of the three pictures can be brighter than the cap: the test below checks it on the pixels.

use super::look::GLOW;

/// A rectangle of whole pixels, `x1` and `y1` not included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Px {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Px {
    pub fn union(self, o: Px) -> Px {
        Px { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }

    pub fn width(self) -> i32 {
        self.x1 - self.x0
    }

    pub fn height(self) -> i32 {
        self.y1 - self.y0
    }
}

/// The colours of the face, each a channel triple at or under the cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The hands and the hub.
    pub ink1: [u8; 3],
    /// The quarter ticks.
    pub ink2: [u8; 3],
    /// The ring and the minor ticks.
    pub ink3: [u8; 3],
    /// The second hand.
    pub second: [u8; 3],
}

/// Premultiplied RGBA.
pub struct Canvas {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Self {
        Self { w, h, px: vec![0; (w * h * 4) as usize] }
    }

    pub fn clear(&mut self) {
        self.px.fill(0);
    }

    pub fn clear_rect(&mut self, r: Px) {
        let r = self.clip(r);
        for y in r.y0..r.y1 {
            let i = ((y as u32 * self.w + r.x0 as u32) * 4) as usize;
            self.px[i..i + (r.width() as usize) * 4].fill(0);
        }
    }

    fn clip(&self, r: Px) -> Px {
        Px { x0: r.x0.clamp(0, self.w as i32), y0: r.y0.clamp(0, self.h as i32), x1: r.x1.clamp(0, self.w as i32), y1: r.y1.clamp(0, self.h as i32) }
    }

    /// `rgb` at `alpha` over what is there.
    fn over(&mut self, x: i32, y: i32, rgb: [u8; 3], alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        let a = alpha.min(1.0);
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        for k in 0..3 {
            let src = f32::from(rgb[k]) * a;
            let dst = f32::from(self.px[i + k]);
            self.px[i + k] = (src + dst * (1.0 - a)).round().min(255.0) as u8;
        }
        let dst_a = f32::from(self.px[i + 3]);
        self.px[i + 3] = (255.0 * a + dst_a * (1.0 - a)).round().min(255.0) as u8;
    }

    fn each(&mut self, r: Px, mut f: impl FnMut(&mut Canvas, i32, i32)) {
        let r = self.clip(r);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                f(self, x, y);
            }
        }
    }

    /// A circle's line `width` wide about `c` at `radius`.
    pub fn ring(&mut self, c: (f32, f32), radius: f32, width: f32, rgb: [u8; 3], alpha: f32) {
        let reach = radius + width + 2.0;
        let bounds =
            Px { x0: (c.0 - reach).floor() as i32, y0: (c.1 - reach).floor() as i32, x1: (c.0 + reach).ceil() as i32, y1: (c.1 + reach).ceil() as i32 };
        self.each(bounds, |s, x, y| {
            let d = (((x as f32 + 0.5 - c.0).powi(2) + (y as f32 + 0.5 - c.1).powi(2)).sqrt() - radius).abs();
            let cover = (width / 2.0 - d + 0.5).clamp(0.0, 1.0);
            s.over(x, y, rgb, cover * alpha);
        });
    }

    /// A filled disc.
    pub fn disc(&mut self, c: (f32, f32), radius: f32, rgb: [u8; 3], alpha: f32) {
        let reach = radius + 2.0;
        let bounds =
            Px { x0: (c.0 - reach).floor() as i32, y0: (c.1 - reach).floor() as i32, x1: (c.0 + reach).ceil() as i32, y1: (c.1 + reach).ceil() as i32 };
        self.each(bounds, |s, x, y| {
            let d = ((x as f32 + 0.5 - c.0).powi(2) + (y as f32 + 0.5 - c.1).powi(2)).sqrt() - radius;
            s.over(x, y, rgb, (0.5 - d).clamp(0.0, 1.0) * alpha);
        });
    }

    /// A segment `a` to `b` with round ends, `w0` wide at `a` and `w1` at `b`.
    pub fn capsule(&mut self, a: (f32, f32), b: (f32, f32), w0: f32, w1: f32, rgb: [u8; 3], alpha: f32) {
        let reach = w0.max(w1) / 2.0 + 2.0;
        let bounds = seg_bounds(a, b, reach);
        self.each(bounds, |s, x, y| {
            let (d, t) = seg_distance(a, b, (x as f32 + 0.5, y as f32 + 0.5));
            let half = (w0 + (w1 - w0) * t) / 2.0;
            s.over(x, y, rgb, (half - d + 0.5).clamp(0.0, 1.0) * alpha);
        });
    }

    /// A soft halo along the segment: `alpha` at the line, falling off with a Gaussian of `sigma`, out to three sigma.
    pub fn halo(&mut self, a: (f32, f32), b: (f32, f32), sigma: f32, rgb: [u8; 3], alpha: f32) {
        let bounds = seg_bounds(a, b, 3.0 * sigma + 1.0);
        self.each(bounds, |s, x, y| {
            let (d, _) = seg_distance(a, b, (x as f32 + 0.5, y as f32 + 0.5));
            if d < 3.0 * sigma {
                s.over(x, y, rgb, alpha * (-(d * d) / (2.0 * sigma * sigma)).exp());
            }
        });
    }
}

/// The pixels a segment's stroke and halo touch, `reach` around it.
pub fn seg_bounds(a: (f32, f32), b: (f32, f32), reach: f32) -> Px {
    Px {
        x0: (a.0.min(b.0) - reach).floor() as i32,
        y0: (a.1.min(b.1) - reach).floor() as i32,
        x1: (a.0.max(b.0) + reach).ceil() as i32,
        y1: (a.1.max(b.1) + reach).ceil() as i32,
    }
}

/// The distance from `p` to the segment `a` `b`, and how far along it the nearest point is (0 to 1).
fn seg_distance(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= f32::EPSILON { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    let (nx, ny) = (a.0 + t * dx, a.1 + t * dy);
    (((p.0 - nx).powi(2) + (p.1 - ny).powi(2)).sqrt(), t)
}

/// The hands' angles in degrees clockwise from twelve for `day_seconds` into the day: the hour hand creeps with the minutes, the minute hand with
/// the seconds.
pub fn angles(day_seconds: f64) -> (f32, f32, f32) {
    let s = day_seconds.rem_euclid(86_400.0);
    let hour = (s / 3600.0 % 12.0) * 30.0;
    let minute = (s / 60.0 % 60.0) * 6.0;
    let second = (s % 60.0) * 6.0;
    (hour as f32, minute as f32, second as f32)
}

/// Where a point `len` from the centre at `deg` clockwise from twelve is (a negative `len` is the tail).
fn at(c: (f32, f32), deg: f32, len: f32) -> (f32, f32) {
    let r = deg.to_radians();
    (c.0 + len * r.sin(), c.1 - len * r.cos())
}

/// The face's own numbers for a diameter `d`: the radius, and a unit that scales strokes (1 at 320).
fn unit(d: u32) -> (f32, f32) {
    (d as f32 / 2.0, d as f32 / 320.0)
}

/// The ring and the twelve ticks.
pub fn paint_face(c: &mut Canvas, d: u32, p: &Palette) {
    let (r, k) = unit(d);
    let centre = (r, r);
    c.ring(centre, r - 1.5 * k.max(1.0), 1.5 * k.max(1.0), p.ink3, 1.0);
    for i in 0..12 {
        let deg = i as f32 * 30.0;
        let quarter = i % 3 == 0;
        let (len, width, rgb) = if quarter { (0.10 * r, 3.5 * k, p.ink2) } else { (0.05 * r, 2.0 * k, p.ink3) };
        let outer = r - 7.0 * k;
        c.capsule(at(centre, deg, outer), at(centre, deg, outer - len), width, width, rgb, 1.0);
    }
}

/// The hour and the minute hand and the hub, each over a halo.
pub fn paint_hands(c: &mut Canvas, d: u32, p: &Palette, hour_deg: f32, minute_deg: f32) {
    let (r, k) = unit(d);
    let centre = (r, r);
    for (deg, tip, tail, w0, w1) in [(hour_deg, 0.52 * r, 0.10 * r, 7.0 * k, 4.0 * k), (minute_deg, 0.80 * r, 0.10 * r, 5.5 * k, 3.0 * k)] {
        let (a, b) = (at(centre, deg, -tail), at(centre, deg, tip));
        c.halo(a, b, 5.0 * k.max(1.0), p.ink1, GLOW);
        c.capsule(a, b, w0, w1, p.ink1, 1.0);
    }
    c.disc(centre, 7.0 * k, p.ink1, 1.0);
}

/// The second hand: a thin line with a short tail and a halo. Returns the pixels it touched, which the caller clears before the next one.
pub fn paint_second(c: &mut Canvas, d: u32, p: &Palette, deg: f32) -> Px {
    let (r, k) = unit(d);
    let centre = (r, r);
    let (a, b) = (at(centre, deg, -0.20 * r), at(centre, deg, 0.88 * r));
    let sigma = 4.0 * k.max(1.0);
    c.halo(a, b, sigma, p.second, GLOW);
    c.capsule(a, b, 2.2 * k.max(1.0), 1.6 * k.max(1.0), p.second, 1.0);
    c.disc(centre, 3.2 * k.max(1.0), p.second, 1.0);
    seg_bounds(a, b, 3.0 * sigma + 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: u32 = 320;

    fn palette() -> Palette {
        Palette { ink1: [95, 94, 97], ink2: [74, 73, 76], ink3: [50, 49, 51], second: [87, 20, 31] }
    }

    fn lit(c: &Canvas, x: u32, y: u32) -> u8 {
        c.px[((y * c.w + x) * 4) as usize..][..3].iter().copied().max().unwrap()
    }

    fn alpha(c: &Canvas, x: u32, y: u32) -> u8 {
        c.px[((y * c.w + x) * 4 + 3) as usize]
    }

    fn all(c: &Canvas) -> impl Iterator<Item = [u8; 4]> + '_ {
        c.px.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]])
    }

    #[test]
    fn the_hands_point_where_the_time_is() {
        assert_eq!(angles(0.0), (0.0, 0.0, 0.0));
        let (h, m, s) = angles(3.0 * 3600.0);
        assert_eq!((h, m, s), (90.0, 0.0, 0.0), "three o'clock: the hour hand to the right");
        let (h, m, s) = angles(10.0 * 3600.0 + 9.0 * 60.0 + 30.0);
        assert!((h - 304.75).abs() < 0.01, "the hour hand creeps with the minutes: {h}");
        assert!((m - 57.0).abs() < 0.01 && (s - 180.0).abs() < 0.01, "9 min 30 s: {m} {s}");
        assert_eq!(angles(86_400.0 + 15.0).2, 90.0, "past midnight it wraps");
        assert_eq!(angles(13.0 * 3600.0).0, 30.0, "the twelve hour dial");
    }

    #[test]
    fn the_face_is_a_hairline_ring_and_twelve_ticks_on_clear_black() {
        let mut c = Canvas::new(D, D);
        paint_face(&mut c, D, &palette());
        assert!(lit(&c, 160, 1) > 30, "the ring at the top, a hairline: {}", lit(&c, 160, 1));
        assert!(lit(&c, 160, 4) == 0, "and nothing just inside it");
        assert!(lit(&c, 1, 160) > 30, "and at the left");
        assert_eq!(alpha(&c, 160, 160), 0, "the middle is clear");
        assert_eq!(alpha(&c, 160, 100), 0, "so is the space between");
        assert!(lit(&c, 160, 15) >= 70, "the twelve o'clock tick is the brighter one: {}", lit(&c, 160, 15));
        let minor_at = |deg: f32| {
            let r = deg.to_radians();
            (160.0 + 148.0 * r.sin(), 160.0 - 148.0 * r.cos())
        };
        let (mx, my) = minor_at(30.0);
        assert!(lit(&c, mx as u32, my as u32) >= 40, "a minor tick at one o'clock");
        let lit_count = all(&c).filter(|p| p[3] > 0).count();
        assert!(lit_count < 5_000, "a hairline ring and twelve small ticks: {lit_count} of {}", D * D);
    }

    #[test]
    fn the_hands_are_where_they_point_and_the_halo_is_fainter_than_the_stroke() {
        let mut c = Canvas::new(D, D);
        paint_hands(&mut c, D, &palette(), 90.0, 0.0);
        assert!(lit(&c, 160 + 60, 160) >= 90, "the hour hand points right at three");
        assert!(lit(&c, 160, 160 - 100) >= 90, "the minute hand points up at the hour");
        assert_eq!(alpha(&c, 160 - 100, 160 + 100), 0, "nothing in the opposite corner");
        let near = lit(&c, 160 + 60, 160 + 10);
        assert!(near > 0 && near < 15, "the halo beside the stroke is a hint: {near}");
        assert!(lit(&c, 160, 160) >= 90, "the hub");
    }

    #[test]
    fn the_second_hand_is_cleared_where_it_was_and_reports_what_it_touched() {
        let mut c = Canvas::new(D, D);
        let first = paint_second(&mut c, D, &palette(), 0.0);
        assert!(alpha(&c, 160, 100) > 200, "straight up");
        assert!(first.y0 <= 160 - 140 && first.x0 < 160 && first.x1 > 160, "{first:?}");
        c.clear_rect(first);
        assert!(all(&c).all(|p| p == [0, 0, 0, 0]), "cleared");
        let second = paint_second(&mut c, D, &palette(), 90.0);
        assert!(alpha(&c, 160 + 100, 160) > 200 && alpha(&c, 160, 100) == 0, "now to the right, and the old place is empty");
        assert!(second.x1 >= 160 + 140);
        let both = first.union(second);
        assert!(both.width() >= 140 && both.height() >= 140);
    }

    #[test]
    fn no_pixel_of_the_face_or_the_hands_is_brighter_than_the_cap_and_every_pixel_is_premultiplied() {
        let p = palette();
        let top = p.ink1.iter().copied().max().unwrap();
        let mut face = Canvas::new(D, D);
        paint_face(&mut face, D, &p);
        let mut hands = Canvas::new(D, D);
        paint_hands(&mut hands, D, &p, 304.75, 57.0);
        let mut second = Canvas::new(D, D);
        paint_second(&mut second, D, &p, 181.0);
        for (name, c) in [("face", &face), ("hands", &hands), ("second", &second)] {
            for px in all(c) {
                assert!(px[0].max(px[1]).max(px[2]) <= top, "{name}: {px:?} is over the cap");
                assert!(px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3], "{name}: {px:?} is not premultiplied");
            }
        }
        let big = 480;
        let mut c = Canvas::new(big, big);
        paint_hands(&mut c, big, &p, 10.0, 200.0);
        assert!(all(&c).all(|px| px[0].max(px[1]).max(px[2]) <= top), "the larger face too");
    }

    #[test]
    fn a_stroke_antialiases_so_a_diagonal_has_no_stairs() {
        let mut c = Canvas::new(64, 64);
        c.capsule((8.0, 8.0), (56.0, 40.0), 3.0, 3.0, [100, 100, 100], 1.0);
        let partial = all(&c).filter(|p| p[3] > 0 && p[3] < 255).count();
        let full = all(&c).filter(|p| p[3] == 255).count();
        assert!(partial > 40 && full > 20, "{partial} edge pixels, {full} solid");
    }
}
