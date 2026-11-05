//! Lucide glyphs drawn from their vector source at the exact pixel size, for the places that must be sharp (the status bar's buttons).
//!
//! The PNG masks of `icon.rs` are 64 px renders that the shell takes down by an area mean to the size it wants: a 28 px glyph has strokes of 2.3 px whose
//! edges fall between pixels, so every edge is a grey ramp two pixels wide. A glyph drawn here comes from Lucide's own 24-unit paths
//! (`compositor/assets/ui/icons/lucide/paths.json`, made by `generators/gen-lucide.py`) at a whole multiple of that grid, 24 px or 48 px, with a stroke of
//! a whole number of pixels (2 or 4) and the round caps and joins Lucide has. The grid's centre lines then fall on pixel boundaries: a straight stroke is
//! exactly its width of full ink with nothing beside it, and a curve is antialiased once, by its exact distance to the pixel centre, and filtered no more.
//! The page is 1920 by 1080 and the display doubles it, so whole pixels here are whole pairs of panel pixels there.

use std::collections::HashMap;
use std::sync::LazyLock;

const SOURCE: &str = include_str!("../../../../compositor/assets/ui/icons/lucide/paths.json");

/// The side of Lucide's design grid.
pub const GRID: u32 = 24;
/// Lucide's stroke on that grid, in its units; a glyph at a whole multiple `k` of the grid has a stroke of `STROKE * k` pixels.
pub const STROKE: f32 = 2.0;

static TABLE: LazyLock<HashMap<String, String>> = LazyLock::new(|| serde_json::from_str::<HashMap<String, String>>(SOURCE).unwrap_or_default());

fn table() -> &'static HashMap<String, String> {
    &TABLE
}

/// Whether the vector source has an icon called `name`.
pub fn has(name: &str) -> bool {
    table().contains_key(name)
}

/// The whole multiple of the grid a glyph is drawn at for a text size: 1 up to 149 %, 2 up to 249 %, and so on, so that the glyph grows with the
/// chrome and its stroke stays a whole number of pixels.
pub fn multiple(text_scale: f32) -> u32 {
    (text_scale.max(1.0).round() as u32).max(1)
}

#[derive(Clone, Debug, PartialEq)]
struct Poly {
    pts: Vec<(f32, f32)>,
    closed: bool,
}

/// What an icon is made of, in grid units: stroked lines, and filled discs (a dot of Lucide's `fill="currentColor"`).
#[derive(Clone, Debug, Default, PartialEq)]
struct Shapes {
    lines: Vec<Poly>,
    dots: Vec<((f32, f32), f32)>,
}

// ---- reading the source

/// The elements of a fragment: the tag and its attributes (`<path d="..."/>`).
fn elements(fragment: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out = Vec::new();
    let mut rest = fragment;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>') else { break };
        let inner = rest[open + 1..open + close].trim_end_matches('/').trim();
        rest = &rest[open + close + 1..];
        let (tag, mut attrs) = match inner.split_once(char::is_whitespace) {
            Some((t, a)) => (t.to_string(), a),
            None => (inner.to_string(), ""),
        };
        let mut list = Vec::new();
        while let Some(eq) = attrs.find("=\"") {
            let name = attrs[..eq].trim().to_string();
            let Some(end) = attrs[eq + 2..].find('"') else { break };
            list.push((name, attrs[eq + 2..eq + 2 + end].to_string()));
            attrs = &attrs[eq + 3 + end..];
        }
        out.push((tag, list));
    }
    out
}

fn number(attrs: &[(String, String)], name: &str) -> Option<f32> {
    attrs.iter().find(|(n, _)| n == name).and_then(|(_, v)| v.trim().parse().ok())
}

/// A scanner over the text of a path or a point list.
struct Scan<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Scan<'a> {
    fn new(s: &'a str) -> Self {
        Self { s: s.as_bytes(), i: 0 }
    }

    fn skip(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }

    fn done(&mut self) -> bool {
        self.skip();
        self.i >= self.s.len()
    }

    /// The next byte if it is a command letter.
    fn command(&mut self) -> Option<u8> {
        self.skip();
        let b = *self.s.get(self.i)?;
        if b.is_ascii_alphabetic() {
            self.i += 1;
            Some(b)
        } else {
            None
        }
    }

    /// A number: `-.5`, `1.25`, `3e-2`; a second `.` starts another number (`.5.5` is two).
    fn num(&mut self) -> Option<f32> {
        self.skip();
        let start = self.i;
        let mut seen_dot = false;
        if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
            self.i += 1;
        }
        while let Some(&b) = self.s.get(self.i) {
            if b.is_ascii_digit() {
                self.i += 1;
            } else if b == b'.' && !seen_dot {
                seen_dot = true;
                self.i += 1;
            } else if (b == b'e' || b == b'E') && self.i > start {
                self.i += 1;
                if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }

    /// An arc's flag: one character, `0` or `1`, which may touch the number after it.
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let b = *self.s.get(self.i)?;
        self.i += 1;
        match b {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }
}

type Pt = (f32, f32);

fn cubic(out: &mut Vec<Pt>, p0: Pt, p1: Pt, p2: Pt, p3: Pt) {
    const N: usize = 24;
    for i in 1..=N {
        let t = i as f32 / N as f32;
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push((a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1));
    }
}

fn quad(out: &mut Vec<Pt>, p0: Pt, p1: Pt, p2: Pt) {
    const N: usize = 16;
    for i in 1..=N {
        let t = i as f32 / N as f32;
        let u = 1.0 - t;
        let (a, b, c) = (u * u, 2.0 * u * t, t * t);
        out.push((a * p0.0 + b * p1.0 + c * p2.0, a * p0.1 + b * p1.1 + c * p2.1));
    }
}

/// An elliptical arc from `p0` to `p1` as the SVG path says it (radii, rotation, the large-arc and sweep flags), in short chords.
#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<Pt>, p0: Pt, rx: f32, ry: f32, rot_deg: f32, large: bool, sweep: bool, p1: Pt) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if (p0.0 == p1.0 && p0.1 == p1.1) || rx == 0.0 || ry == 0.0 {
        out.push(p1);
        return;
    }
    let (sin, cos) = rot_deg.to_radians().sin_cos();
    let (dx, dy) = ((p0.0 - p1.0) / 2.0, (p0.1 - p1.1) / 2.0);
    let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let k = if den == 0.0 { 0.0 } else { (num / den).max(0.0).sqrt() } * if large == sweep { -1.0 } else { 1.0 };
    let (cxp, cyp) = (k * rx * y1 / ry, -k * ry * x1 / rx);
    let (cx, cy) = (cos * cxp - sin * cyp + (p0.0 + p1.0) / 2.0, sin * cxp + cos * cyp + (p0.1 + p1.1) / 2.0);
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let theta = angle(1.0, 0.0, (x1 - cxp) / rx, (y1 - cyp) / ry);
    let mut delta = angle((x1 - cxp) / rx, (y1 - cyp) / ry, (-x1 - cxp) / rx, (-y1 - cyp) / ry);
    if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    }
    let steps = ((delta.abs() / (std::f32::consts::PI / 48.0)).ceil() as usize).max(4);
    for i in 1..=steps {
        let a = theta + delta * i as f32 / steps as f32;
        let (ex, ey) = (rx * a.cos(), ry * a.sin());
        out.push((cos * ex - sin * ey + cx, sin * ex + cos * ey + cy));
    }
    // the last chord ends where the path says, not where the rounding left it
    if let Some(last) = out.last_mut() {
        *last = p1;
    }
}

fn path(d: &str, out: &mut Vec<Poly>) {
    let mut s = Scan::new(d);
    let (mut cur, mut start): (Pt, Pt) = ((0.0, 0.0), (0.0, 0.0));
    let (mut c2, mut q1): (Option<Pt>, Option<Pt>) = (None, None);
    let mut pts: Vec<Pt> = Vec::new();
    let mut cmd = 0u8;
    let flush = |pts: &mut Vec<Pt>, closed: bool, out: &mut Vec<Poly>| {
        if pts.len() >= 2 {
            out.push(Poly { pts: std::mem::take(pts), closed });
        } else {
            pts.clear();
        }
    };
    while !s.done() {
        if let Some(c) = s.command() {
            cmd = c;
        } else if cmd == b'M' {
            cmd = b'L';
        } else if cmd == b'm' {
            cmd = b'l';
        } else if cmd == 0 || matches!(cmd, b'Z' | b'z') {
            return;
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { cur } else { (0.0, 0.0) };
        macro_rules! n {
            () => {
                match s.num() {
                    Some(v) => v,
                    None => return,
                }
            };
        }
        let upper = cmd.to_ascii_uppercase();
        if upper != b'C' && upper != b'S' {
            c2 = None;
        }
        if upper != b'Q' && upper != b'T' {
            q1 = None;
        }
        match upper {
            b'M' => {
                flush(&mut pts, false, out);
                cur = (ox + n!(), oy + n!());
                start = cur;
                pts.push(cur);
            }
            b'L' => {
                cur = (ox + n!(), oy + n!());
                pts.push(cur);
            }
            b'H' => {
                cur = (if rel { cur.0 } else { 0.0 } + n!(), cur.1);
                pts.push(cur);
            }
            b'V' => {
                cur = (cur.0, if rel { cur.1 } else { 0.0 } + n!());
                pts.push(cur);
            }
            b'C' => {
                let (a, b, e) = ((ox + n!(), oy + n!()), (ox + n!(), oy + n!()), (ox + n!(), oy + n!()));
                cubic(&mut pts, cur, a, b, e);
                c2 = Some(b);
                cur = e;
            }
            b'S' => {
                let a = c2.map_or(cur, |p| (2.0 * cur.0 - p.0, 2.0 * cur.1 - p.1));
                let (b, e) = ((ox + n!(), oy + n!()), (ox + n!(), oy + n!()));
                cubic(&mut pts, cur, a, b, e);
                c2 = Some(b);
                cur = e;
            }
            b'Q' => {
                let (a, e) = ((ox + n!(), oy + n!()), (ox + n!(), oy + n!()));
                quad(&mut pts, cur, a, e);
                q1 = Some(a);
                cur = e;
            }
            b'T' => {
                let a = q1.map_or(cur, |p| (2.0 * cur.0 - p.0, 2.0 * cur.1 - p.1));
                let e = (ox + n!(), oy + n!());
                quad(&mut pts, cur, a, e);
                q1 = Some(a);
                cur = e;
            }
            b'A' => {
                let (rx, ry, rot) = (n!(), n!(), n!());
                let (Some(large), Some(sweep)) = (s.flag(), s.flag()) else { return };
                let e = (ox + n!(), oy + n!());
                arc(&mut pts, cur, rx, ry, rot, large, sweep, e);
                cur = e;
            }
            b'Z' => {
                flush(&mut pts, true, out);
                cur = start;
                pts.push(cur);
            }
            _ => return,
        }
    }
    flush(&mut pts, false, out);
}

fn ellipse(c: Pt, rx: f32, ry: f32) -> Poly {
    const N: usize = 96;
    Poly { pts: (0..N).map(|i| (c.0 + rx * (i as f32 / N as f32 * std::f32::consts::TAU).cos(), c.1 + ry * (i as f32 / N as f32 * std::f32::consts::TAU).sin())).collect(), closed: true }
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) -> Poly {
    let (rx, ry) = (rx.min(w / 2.0), ry.min(h / 2.0));
    let mut pts = Vec::new();
    let corners = [((x + w - rx, y + ry), -90.0f32), ((x + w - rx, y + h - ry), 0.0), ((x + rx, y + h - ry), 90.0), ((x + rx, y + ry), 180.0)];
    for ((cx, cy), from) in corners {
        for i in 0..=16 {
            let a = (from + 90.0 * i as f32 / 16.0).to_radians();
            pts.push((cx + rx * a.cos(), cy + ry * a.sin()));
        }
    }
    Poly { pts, closed: true }
}

fn shapes_of(fragment: &str) -> Shapes {
    let mut shapes = Shapes::default();
    for (tag, attrs) in elements(fragment) {
        let get = |n: &str| number(&attrs, n).unwrap_or(0.0);
        match tag.as_str() {
            "path" => {
                if let Some((_, d)) = attrs.iter().find(|(n, _)| n == "d") {
                    path(d, &mut shapes.lines);
                }
            }
            "circle" => {
                let (c, r) = ((get("cx"), get("cy")), get("r"));
                shapes.lines.push(ellipse(c, r, r));
                if attrs.iter().any(|(n, v)| n == "fill" && v != "none") {
                    shapes.dots.push((c, r));
                }
            }
            "line" => shapes.lines.push(Poly { pts: vec![(get("x1"), get("y1")), (get("x2"), get("y2"))], closed: false }),
            "polyline" => {
                if let Some((_, p)) = attrs.iter().find(|(n, _)| n == "points") {
                    let mut s = Scan::new(p);
                    let mut pts = Vec::new();
                    while let (Some(x), Some(y)) = (s.num(), s.num()) {
                        pts.push((x, y));
                    }
                    shapes.lines.push(Poly { pts, closed: false });
                }
            }
            "rect" => {
                let rx = number(&attrs, "rx").or_else(|| number(&attrs, "ry")).unwrap_or(0.0);
                let ry = number(&attrs, "ry").unwrap_or(rx);
                shapes.lines.push(rounded_rect(get("x"), get("y"), get("width"), get("height"), rx, ry));
            }
            _ => {}
        }
    }
    shapes
}

// ---- drawing

/// A glyph's alpha, `side` by `side`, row by row: 255 where the stroke is, 0 where it is not, the exact coverage of the pixel between.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    pub side: u32,
    pub alpha: Vec<u8>,
}

/// The icon `name` at `side` pixels, its stroke `stroke` pixels wide (round caps and joins); `None` for a name the source does not have. A `side` that is
/// a whole multiple of [`GRID`] and a whole `stroke` are what put the strokes on whole pixels.
pub fn draw(name: &str, side: u32, stroke: f32) -> Option<Glyph> {
    let shapes = shapes_of(table().get(name)?);
    let k = side as f32 / GRID as f32;
    let half = stroke / 2.0;
    let mut segments: Vec<(f32, f32, f32, f32)> = Vec::new();
    for poly in &shapes.lines {
        let n = poly.pts.len();
        for i in 0..n.saturating_sub(1) {
            let (a, b) = (poly.pts[i], poly.pts[i + 1]);
            segments.push((a.0 * k, a.1 * k, b.0 * k, b.1 * k));
        }
        if poly.closed && n > 2 {
            let (a, b) = (poly.pts[n - 1], poly.pts[0]);
            segments.push((a.0 * k, a.1 * k, b.0 * k, b.1 * k));
        }
    }
    let mut alpha = vec![0u8; (side * side) as usize];
    for y in 0..side {
        for x in 0..side {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut d2 = f32::MAX;
            for &(ax, ay, bx, by) in &segments {
                let (vx, vy) = (bx - ax, by - ay);
                let len2 = vx * vx + vy * vy;
                let t = if len2 > 0.0 { (((px - ax) * vx + (py - ay) * vy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let (qx, qy) = (ax + t * vx - px, ay + t * vy - py);
                d2 = d2.min(qx * qx + qy * qy);
            }
            let mut cover = (half + 0.5 - d2.sqrt()).clamp(0.0, 1.0);
            for &((cx, cy), r) in &shapes.dots {
                let d = ((px - cx * k).powi(2) + (py - cy * k).powi(2)).sqrt();
                cover = cover.max((r * k - d + 0.5).clamp(0.0, 1.0));
            }
            alpha[(y * side + x) as usize] = (cover * 255.0).round() as u8;
        }
    }
    Some(Glyph { side, alpha })
}

impl Glyph {
    /// How far to move the glyph, in whole pixels, to sit optically in the middle of a box its own size: the middle of its ink's bounding box and of its
    /// ink's weight, taken half and half (a bell's clapper adds to the box and little to the weight; a magnifier's ring is the weight and its handle the
    /// reach), at most two pixels either way.
    pub fn optical_offset(&self) -> (i32, i32) {
        let side = self.side as usize;
        let (mut x0, mut y0, mut x1, mut y1) = (side, side, 0usize, 0usize);
        let (mut sum, mut sx, mut sy) = (0.0f64, 0.0f64, 0.0f64);
        for y in 0..side {
            for x in 0..side {
                let a = f64::from(self.alpha[y * side + x]);
                if a >= 64.0 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
                sum += a;
                sx += a * (x as f64 + 0.5);
                sy += a * (y as f64 + 0.5);
            }
        }
        if sum == 0.0 || x1 <= x0 {
            return (0, 0);
        }
        let middle = side as f64 / 2.0;
        let cx = 0.5 * (x0 + x1) as f64 / 2.0 + 0.5 * sx / sum;
        let cy = 0.5 * (y0 + y1) as f64 / 2.0 + 0.5 * sy / sum;
        (((middle - cx).round() as i32).clamp(-2, 2), ((middle - cy).round() as i32).clamp(-2, 2))
    }

    /// The glyph in `ink`, premultiplied RGBA for an image layer.
    pub fn tinted(&self, ink: compositor_protocol::Rgba) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.alpha.len() * 4);
        for &a in &self.alpha {
            let a = u32::from(a);
            out.extend_from_slice(&[(u32::from(ink.0) * a / 255) as u8, (u32::from(ink.1) * a / 255) as u8, (u32::from(ink.2) * a / 255) as u8, (u32::from(ink.3) * a / 255) as u8]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: [&str; 4] = ["search", "camera", "bell", "settings"];

    fn at(g: &Glyph, x: u32, y: u32) -> u8 {
        g.alpha[(y * g.side + x) as usize]
    }

    #[test]
    fn the_source_has_the_status_glyphs_and_the_names_the_generator_made() {
        for name in STATUS {
            assert!(has(name), "{name}");
            assert!(!shapes_of(table().get(name).unwrap()).lines.is_empty(), "{name} has strokes");
        }
        assert!(!has("no-such-icon"));
        assert!(table().len() >= 160);
    }

    #[test]
    fn a_straight_stroke_on_the_grid_is_its_width_of_full_ink_and_nothing_beside_it() {
        // the camera's left side is the line x = 2 from y = 9 to y = 18: a 2 px stroke is the columns 1 and 2
        let g = draw("camera", 24, 2.0).unwrap();
        for y in 11..16 {
            assert_eq!([at(&g, 0, y), at(&g, 1, y), at(&g, 2, y), at(&g, 3, y)], [0, 255, 255, 0], "row {y}");
        }
        // twice the size, twice the stroke: the same strokes on whole pixels
        let big = draw("camera", 48, 4.0).unwrap();
        for y in 22..32 {
            assert_eq!([1, 2, 3, 4, 5, 6].map(|x| at(&big, x, y)), [0, 255, 255, 255, 255, 0], "row {y}");
        }
    }

    #[test]
    fn a_curve_is_antialiased_once_and_a_ring_is_as_thick_as_the_stroke() {
        // the magnifier's ring is centred on (11, 11) with radius 8: along the row through the centre the ring is 2 px of ink either side
        let g = draw("search", 24, 2.0).unwrap();
        let row: Vec<u8> = (0..24).map(|x| at(&g, x, 10)).collect();
        let left: u32 = row[0..6].iter().map(|v| u32::from(*v)).sum();
        assert!((left as i32 - 2 * 255).abs() <= 30, "the ring's left side has the weight of a 2 px stroke: {row:?}");
        assert_eq!(row[0..2], [0, 0], "nothing outside the ring");
    }

    #[test]
    fn every_status_glyph_has_the_same_stroke_and_sits_within_a_pixel_of_the_middle() {
        for (side, stroke) in [(24, 2.0), (48, 4.0)] {
            for name in STATUS {
                let g = draw(name, side, stroke).unwrap();
                let (dx, dy) = g.optical_offset();
                assert!(dx.abs() <= 1 && dy.abs() <= 1, "{name} at {side}: {dx}, {dy}");
                // the ink is inside the box with the grid's margin: the stroke never reaches the edge
                let side = side as usize;
                assert!(g.alpha[..side].iter().all(|a| *a == 0) && g.alpha[side * (side - 1)..].iter().all(|a| *a == 0), "{name}");
            }
        }
    }

    #[test]
    fn a_larger_text_size_has_a_larger_whole_multiple() {
        assert_eq!([1.0, 1.25, 1.49, 1.5, 2.0, 2.4, 3.0].map(multiple), [1, 1, 1, 2, 2, 2, 3]);
    }

    #[test]
    fn arcs_curves_and_the_other_elements_of_the_source_are_read() {
        // a quarter circle of radius 4 from (4, 0) to (0, 4): sweeps through (2.83, 2.83)
        let mut pts = vec![(4.0, 0.0)];
        arc(&mut pts, (4.0, 0.0), 4.0, 4.0, 0.0, false, true, (0.0, 4.0));
        assert!(pts.iter().all(|(x, y)| ((x * x + y * y).sqrt() - 4.0).abs() < 0.01), "{pts:?}");
        assert!(pts.iter().any(|(x, y)| (x - 2.83).abs() < 0.1 && (y - 2.83).abs() < 0.1));
        let s = shapes_of(r#"<path d="M3 3h6v6H3z"/><circle cx="12" cy="12" r="2" fill="currentColor"/><line x1="0" x2="4" y1="0" y2="0"/><rect width="4" height="4" x="1" y="1" rx="1"/><polyline points="1 1 2 2 3 1"/>"#);
        assert_eq!(s.lines.len(), 5);
        assert!(s.lines[0].closed && s.lines[0].pts.len() >= 4);
        assert_eq!(s.dots, vec![((12.0, 12.0), 2.0)]);
    }
}
