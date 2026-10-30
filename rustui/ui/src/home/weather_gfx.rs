//! The shapes the weather view paints into its pictures besides rounded bars and discs (`Canvas`): arcs, lines and bars with a gradient, each
//! with analytic coverage (a pixel is covered by how much of it the shape's edge leaves), composited over what is there. Angles are in radians,
//! 0 at the top and growing clockwise.

use super::widgets::Canvas;
use compositor_protocol::Rgba;
use std::f32::consts::TAU;
use ui::geometry::Frame;

/// `col` (premultiplied) over the pixel at (`x`, `y`) by `cov`.
fn blend(c: &mut Canvas, x: i32, y: i32, col: Rgba, cov: f32) {
    if cov <= 0.0 || x < 0 || y < 0 || x >= c.w as i32 || y >= c.h as i32 {
        return;
    }
    let i = ((y as u32 * c.w + x as u32) * 4) as usize;
    let s = [f32::from(col.0) * cov, f32::from(col.1) * cov, f32::from(col.2) * cov, f32::from(col.3) * cov];
    let k = 1.0 - s[3] / 255.0;
    for (ch, v) in s.iter().enumerate() {
        c.px[i + ch] = (v + f32::from(c.px[i + ch]) * k).round().min(255.0) as u8;
    }
}

fn cov(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

#[allow(clippy::too_many_arguments)]
/// An arc of the circle of radius `r` at (`cx`, `cy`) from angle `from` to `to` (`to` not before `from`, at most a turn on), `width` thick, with round ends.
pub fn arc(c: &mut Canvas, cx: f32, cy: f32, r: f32, width: f32, from: f32, to: f32, col: Rgba) {
    let pad = r + width + 2.0;
    let sweep = (to - from).clamp(0.0, TAU);
    let ends = [(cx + r * from.sin(), cy - r * from.cos()), (cx + r * (from + sweep).sin(), cy - r * (from + sweep).cos())];
    for y in (cy - pad).floor() as i32..(cy + pad).ceil() as i32 {
        for x in (cx - pad).floor() as i32..(cx + pad).ceil() as i32 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let (dx, dy) = (px - cx, py - cy);
            let ang = dx.atan2(-dy);
            let d = if (ang - from).rem_euclid(TAU) <= sweep {
                ((dx * dx + dy * dy).sqrt() - r).abs() - width / 2.0
            } else {
                ends.iter().map(|e| ((px - e.0).powi(2) + (py - e.1).powi(2)).sqrt()).fold(f32::MAX, f32::min) - width / 2.0
            };
            blend(c, x, y, col, cov(d));
        }
    }
}

/// A line from `a` to `b`, `width` thick, with round ends.
pub fn line(c: &mut Canvas, a: (f32, f32), b: (f32, f32), width: f32, col: Rgba) {
    let pad = width / 2.0 + 2.0;
    let (bx, by) = (b.0 - a.0, b.1 - a.1);
    let len2 = (bx * bx + by * by).max(1e-6);
    for y in (a.1.min(b.1) - pad).floor() as i32..(a.1.max(b.1) + pad).ceil() as i32 {
        for x in (a.0.min(b.0) - pad).floor() as i32..(a.0.max(b.0) + pad).ceil() as i32 {
            let (px, py) = (x as f32 + 0.5 - a.0, y as f32 + 0.5 - a.1);
            let t = ((px * bx + py * by) / len2).clamp(0.0, 1.0);
            let d = ((px - bx * t).powi(2) + (py - by * t).powi(2)).sqrt() - width / 2.0;
            blend(c, x, y, col, cov(d));
        }
    }
}

/// A triangle with its tip at `tip`, pointing along `angle`, `len` long and `base` wide.
pub fn head(c: &mut Canvas, tip: (f32, f32), angle: f32, len: f32, base: f32, col: Rgba) {
    let (dx, dy) = (angle.sin(), -angle.cos());
    let (nx, ny) = (-dy, dx);
    let back = (tip.0 - dx * len, tip.1 - dy * len);
    let p = [tip, (back.0 + nx * base / 2.0, back.1 + ny * base / 2.0), (back.0 - nx * base / 2.0, back.1 - ny * base / 2.0)];
    let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[1].1 - p[0].1) * (p[2].0 - p[0].0);
    let orient = if area > 0.0 { 1.0 } else { -1.0 };
    let (x0, x1) = (p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor() as i32 - 1, p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil() as i32 + 1);
    let (y0, y1) = (p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor() as i32 - 1, p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil() as i32 + 1);
    for y in y0..y1 {
        for x in x0..x1 {
            let q = (x as f32 + 0.5, y as f32 + 0.5);
            // the signed distance to the triangle (negative inside): the farthest of the three edges
            let mut d = f32::MIN;
            for i in 0..3 {
                let (a, b) = (p[i], p[(i + 1) % 3]);
                let (ex, ey) = (b.0 - a.0, b.1 - a.1);
                d = d.max(((q.0 - a.0) * ey - (q.1 - a.1) * ex) / (ex * ex + ey * ey).sqrt().max(1e-6) * orient);
            }
            blend(c, x, y, col, cov(d));
        }
    }
}

/// The y of the curve through `pts` (x increasing) at `x`: a cosine ease between each two, so that it is smooth through every point and level at it.
fn eased_y(pts: &[(f32, f32)], x: f32) -> f32 {
    let i = pts.windows(2).position(|w| x <= w[1].0).unwrap_or(pts.len().saturating_sub(2));
    let (a, b) = (pts[i], pts[(i + 1).min(pts.len() - 1)]);
    let u = if b.0 > a.0 { ((x - a.0) / (b.0 - a.0)).clamp(0.0, 1.0) } else { 0.0 };
    a.1 + (b.1 - a.1) * (1.0 - (std::f32::consts::PI * u).cos()) / 2.0
}

/// A smooth curve through `pts` (x increasing), `width` thick.
pub fn curve(c: &mut Canvas, pts: &[(f32, f32)], width: f32, col: Rgba) {
    if pts.len() < 2 {
        return;
    }
    let (x0, x1) = (pts[0].0, pts[pts.len() - 1].0);
    let steps = ((x1 - x0) / 3.0).ceil().max(1.0) as usize;
    let mut last = (x0, eased_y(pts, x0));
    for s in 1..=steps {
        let x = x0 + (x1 - x0) * s as f32 / steps as f32;
        let p = (x, eased_y(pts, x));
        line(c, last, p, width, col);
        last = p;
    }
}

/// The area under the curve through `pts` down to `bottom`, `col` at the curve and fading to nothing at the bottom.
pub fn area(c: &mut Canvas, pts: &[(f32, f32)], bottom: f32, col: Rgba) {
    if pts.len() < 2 {
        return;
    }
    let (x0, x1) = (pts[0].0.floor() as i32, pts[pts.len() - 1].0.ceil() as i32);
    for x in x0..x1 {
        let top = eased_y(pts, x as f32 + 0.5);
        let span = (bottom - top).max(1.0);
        for y in top.floor() as i32..bottom.ceil() as i32 {
            let t = ((y as f32 + 0.5 - top) / span).clamp(0.0, 1.0);
            let edge = ((y as f32 + 1.0 - top).clamp(0.0, 1.0)) * (1.0 - t) * (1.0 - t);
            blend(c, x, y, col, edge);
        }
    }
}

/// A rounded bar whose colour runs along `stops` (position 0 to 1 along its width, premultiplied colour).
pub fn gradient_bar(c: &mut Canvas, f: Frame, radius: f32, stops: &[(f32, Rgba)]) {
    let r = radius.min(f.w / 2.0).min(f.h / 2.0);
    let colour_at = |u: f32| -> Rgba {
        let u = u.clamp(0.0, 1.0);
        let mut prev = stops[0];
        for s in stops {
            if u <= s.0 {
                let t = if s.0 > prev.0 { (u - prev.0) / (s.0 - prev.0) } else { 0.0 };
                let m = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
                return Rgba(m(prev.1.0, s.1.0), m(prev.1.1, s.1.1), m(prev.1.2, s.1.2), m(prev.1.3, s.1.3));
            }
            prev = *s;
        }
        prev.1
    };
    for y in f.y.floor() as i32..f.bottom().ceil() as i32 {
        for x in f.x.floor() as i32..f.right().ceil() as i32 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = (f.x + r - px).max(px - (f.right() - r)).max(0.0);
            let dy = (f.y + r - py).max(py - (f.bottom() - r)).max(0.0);
            let d = (dx * dx + dy * dy).sqrt() - r;
            blend(c, x, y, colour_at((px - f.x) / f.w), cov(d));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn alpha(px: &[u8], w: u32, x: u32, y: u32) -> u8 {
        px[((y * w + x) * 4 + 3) as usize]
    }

    #[test]
    fn an_arc_covers_its_ring_between_its_angles_and_nothing_else() {
        let mut px = vec![0u8; 100 * 100 * 4];
        let mut c = Canvas { px: &mut px, w: 100, h: 100 };
        // a quarter, from the top clockwise to the right
        arc(&mut c, 50.0, 50.0, 30.0, 4.0, 0.0, FRAC_PI_2, Rgba(255, 255, 255, 255));
        assert!(alpha(&px, 100, 50, 20) > 200, "the top end");
        assert!(alpha(&px, 100, 80, 50) > 200, "the right end");
        assert!(alpha(&px, 100, 71, 29) > 200, "between them");
        assert_eq!(alpha(&px, 100, 50, 80), 0, "not the bottom");
        assert_eq!(alpha(&px, 100, 20, 50), 0, "not the left");
        assert_eq!(alpha(&px, 100, 50, 50), 0, "not the middle");
    }

    #[test]
    fn a_curve_goes_through_its_points_and_the_area_under_it_fades_downwards() {
        let pts = [(10.0, 60.0), (50.0, 20.0), (90.0, 50.0)];
        assert!((eased_y(&pts, 10.0) - 60.0).abs() < 1e-3 && (eased_y(&pts, 50.0) - 20.0).abs() < 1e-3 && (eased_y(&pts, 90.0) - 50.0).abs() < 1e-3);
        let mid = eased_y(&pts, 30.0);
        assert!((mid - 40.0).abs() < 1e-3, "halfway is halfway: {mid}");
        let mut px = vec![0u8; 100 * 100 * 4];
        let mut c = Canvas { px: &mut px, w: 100, h: 100 };
        curve(&mut c, &pts, 4.0, Rgba(255, 255, 255, 255));
        assert!(alpha(&px, 100, 50, 20) > 240 && alpha(&px, 100, 30, 40) > 200 && alpha(&px, 100, 50, 60) == 0);
        let mut px = vec![0u8; 100 * 100 * 4];
        let mut c = Canvas { px: &mut px, w: 100, h: 100 };
        area(&mut c, &pts, 95.0, Rgba(255, 255, 255, 255));
        assert!(alpha(&px, 100, 50, 22) > alpha(&px, 100, 50, 60) && alpha(&px, 100, 50, 60) > alpha(&px, 100, 50, 90), "fading downwards");
        assert_eq!(alpha(&px, 100, 50, 10), 0, "nothing above the curve");
    }

    #[test]
    fn a_line_a_head_and_a_gradient_cover_what_they_describe() {
        let mut px = vec![0u8; 100 * 100 * 4];
        let mut c = Canvas { px: &mut px, w: 100, h: 100 };
        line(&mut c, (10.0, 50.0), (90.0, 50.0), 6.0, Rgba(255, 255, 255, 255));
        assert!(alpha(&px, 100, 50, 50) > 250 && alpha(&px, 100, 50, 49) > 250 && alpha(&px, 100, 50, 60) == 0);
        let mut px = vec![0u8; 100 * 100 * 4];
        let mut c = Canvas { px: &mut px, w: 100, h: 100 };
        // an arrow head with its tip at the top of the picture's middle, pointing up
        head(&mut c, (50.0, 20.0), 0.0, 30.0, 30.0, Rgba(255, 255, 255, 255));
        assert!(alpha(&px, 100, 50, 30) > 250 && alpha(&px, 100, 50, 45) > 250 && alpha(&px, 100, 40, 48) > 250, "the body");
        assert_eq!((alpha(&px, 100, 50, 10), alpha(&px, 100, 20, 30), alpha(&px, 100, 50, 60)), (0, 0, 0), "and not beyond its tip or its sides");
        let mut px = vec![0u8; 100 * 20 * 4];
        let mut c = Canvas { px: &mut px, w: 100, h: 20 };
        gradient_bar(&mut c, Frame::new(0.0, 4.0, 100.0, 12.0), 6.0, &[(0.0, Rgba(255, 0, 0, 255)), (1.0, Rgba(0, 0, 255, 255))]);
        let at = |x: usize| (px[(10 * 100 + x) * 4], px[(10 * 100 + x) * 4 + 2]);
        assert!(at(10).0 > 200 && at(10).1 < 60, "red at the start");
        assert!(at(90).0 < 60 && at(90).1 > 200, "blue at the end");
    }
}
