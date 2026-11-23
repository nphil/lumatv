//! What the basic screen draws with: Manrope SemiBold (the shell's own face), a cache of its rasterised glyphs, and a premultiplied RGBA
//! canvas. The compositor draws this screen when the UI process cannot, so nothing here may depend on the UI's toolkit or fonts.

use std::borrow::Cow;
use std::collections::HashMap;

use fontdue::{Font, FontSettings};

use crate::Rect;

const MANROPE: &[u8] = include_bytes!("../../../../../payload/fonts/Manrope-SemiBold.ttf");

/// A colour, `[r, g, b]`.
pub(crate) type Rgb = [u8; 3];
/// A colour with its opacity, `[r, g, b, a]`, not premultiplied (the canvas premultiplies on the way in).
pub(crate) type Rgba = [u8; 4];

pub(crate) fn opaque(c: Rgb) -> Rgba {
    [c[0], c[1], c[2], 255]
}

pub(crate) fn with_alpha(c: Rgb, alpha: f32) -> Rgba {
    [c[0], c[1], c[2], (alpha.clamp(0.0, 1.0) * 255.0).round() as u8]
}

/// `a` moved `t` (0 to 1) of the way to `b`.
pub(crate) fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

/// `x / 255`, rounded, for `x` up to 255 * 255.
fn div255(x: u32) -> u32 {
    (x + 128 + ((x + 128) >> 8)) >> 8
}

struct Glyph {
    advance: f32,
    xmin: i32,
    /// The bitmap's bottom edge above the baseline.
    ymin: i32,
    w: usize,
    h: usize,
    coverage: Vec<u8>,
}

static NO_GLYPH: Glyph = Glyph { advance: 0.0, xmin: 0, ymin: 0, w: 0, h: 0, coverage: Vec::new() };

/// The face and the glyphs rasterised so far, per character and pixel size. Whole pixels only: the screen is drawn on an integer grid.
pub(crate) struct Typeface {
    /// `None` only if the embedded file fails to parse; text then measures and draws as nothing.
    font: Option<Font>,
    glyphs: HashMap<(char, u32), Glyph>,
}

impl Typeface {
    pub(crate) fn new() -> Typeface {
        Typeface { font: Font::from_bytes(MANROPE, FontSettings::default()).ok(), glyphs: HashMap::new() }
    }

    pub(crate) fn has(&self, c: char) -> bool {
        self.font.as_ref().is_some_and(|f| f.lookup_glyph_index(c) != 0)
    }

    /// The glyph drawn for `c`: a character the face lacks (an app's title may be in any script) shows as `?`, not as the face's blank box.
    fn glyph(&mut self, c: char, px: u32) -> &Glyph {
        let Some(font) = self.font.as_ref() else { return &NO_GLYPH };
        let c = if c.is_control() {
            ' '
        } else if font.lookup_glyph_index(c) != 0 {
            c
        } else {
            '?'
        };
        self.glyphs.entry((c, px)).or_insert_with(|| {
            let (m, coverage) = font.rasterize(c, px as f32);
            Glyph { advance: m.advance_width, xmin: m.xmin, ymin: m.ymin, w: m.width, h: m.height, coverage }
        })
    }

    /// Pixels from the top of a line to its baseline.
    pub(crate) fn ascent(&self, px: u32) -> i32 {
        match self.font.as_ref().and_then(|f| f.horizontal_line_metrics(px as f32)) {
            Some(m) => m.ascent.round() as i32,
            None => px as i32,
        }
    }

    /// Height of a line of text, ascent to descent.
    pub(crate) fn line_height(&self, px: u32) -> i32 {
        match self.font.as_ref().and_then(|f| f.horizontal_line_metrics(px as f32)) {
            Some(m) => (m.ascent - m.descent).ceil() as i32,
            None => px as i32 * 5 / 4,
        }
    }

    pub(crate) fn measure(&mut self, text: &str, px: u32) -> i32 {
        text.chars().map(|c| self.glyph(c, px).advance).sum::<f32>().ceil() as i32
    }

    fn ellipsis(&self) -> &'static str {
        if self.has('\u{2026}') { "\u{2026}" } else { "..." }
    }

    /// `text`, or its head and an ellipsis when it is wider than `max_width`.
    pub(crate) fn fit<'a>(&mut self, text: &'a str, px: u32, max_width: i32) -> Cow<'a, str> {
        if self.measure(text, px) <= max_width {
            return Cow::Borrowed(text);
        }
        let dots = self.ellipsis();
        let room = max_width as f32 - self.measure(dots, px) as f32;
        if room < 0.0 {
            return Cow::Borrowed("");
        }
        let (mut used, mut end) = (0.0, 0);
        for (i, c) in text.char_indices() {
            used += self.glyph(c, px).advance;
            if used > room {
                break;
            }
            end = i + c.len_utf8();
        }
        Cow::Owned(format!("{}{dots}", text[..end].trim_end()))
    }
}

/// How much of the pixel centred on `(x, y)` a shape covers, from its signed distance there: 1 inside by half a pixel or more, 0 outside by
/// as much.
fn coverage(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

/// A rounded rectangle's signed distance field.
struct Rounded {
    cx: f32,
    cy: f32,
    hx: f32,
    hy: f32,
    radius: f32,
}

impl Rounded {
    fn new(r: Rect, radius: f32) -> Rounded {
        let (hx, hy) = (r.w as f32 / 2.0, r.h as f32 / 2.0);
        Rounded { cx: r.x as f32 + hx, cy: r.y as f32 + hy, hx, hy, radius: radius.clamp(0.0, hx.min(hy)) }
    }

    fn distance(&self, x: f32, y: f32) -> f32 {
        let qx = (x - self.cx).abs() - (self.hx - self.radius);
        let qy = (y - self.cy).abs() - (self.hy - self.radius);
        qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - self.radius
    }

    fn at(&self, x: i32, y: i32) -> f32 {
        coverage(self.distance(x as f32 + 0.5, y as f32 + 0.5))
    }
}

/// Premultiplied RGBA over a transparent start, in output pixel coordinates: every call takes the screen's coordinates and what falls
/// outside the canvas's own rectangle is dropped.
pub(crate) struct Canvas {
    rect: Rect,
    px: Vec<u8>,
}

impl Canvas {
    pub(crate) fn new(rect: Rect) -> Canvas {
        Canvas { rect, px: vec![0; rect.w.max(0) as usize * rect.h.max(0) as usize * 4] }
    }

    pub(crate) fn rect(&self) -> Rect {
        self.rect
    }

    pub(crate) fn into_rgba(self) -> Vec<u8> {
        self.px
    }

    #[cfg(test)]
    pub(crate) fn pixel(&self, x: i32, y: i32) -> [u8; 4] {
        let i = self.index(x, y);
        [self.px[i], self.px[i + 1], self.px[i + 2], self.px[i + 3]]
    }

    /// Composites premultiplied `rgba` (a band's pixels) over the canvas at `r`, as the compositor does.
    #[cfg(test)]
    pub(crate) fn blit(&mut self, r: Rect, rgba: &[u8]) {
        for y in 0..r.h {
            for x in 0..r.w {
                let s = &rgba[(y * r.w + x) as usize * 4..][..4];
                let i = self.index(r.x + x, r.y + y);
                let keep = 255 - s[3] as u32;
                for k in 0..4 {
                    self.px[i + k] = (s[k] as u32 + div255(self.px[i + k] as u32 * keep)) as u8;
                }
            }
        }
    }

    fn index(&self, x: i32, y: i32) -> usize {
        ((y - self.rect.y) as usize * self.rect.w as usize + (x - self.rect.x) as usize) * 4
    }

    /// Source-over of `c` at `alpha` (0 to 255, the shape's coverage already folded in) onto the pixel at `i`.
    fn over(&mut self, i: usize, c: Rgba, alpha: u32) {
        if alpha == 0 {
            return;
        }
        let keep = 255 - alpha;
        let d = &mut self.px[i..i + 4];
        for k in 0..3 {
            d[k] = div255(c[k] as u32 * alpha + d[k] as u32 * keep) as u8;
        }
        d[3] = div255(255 * alpha + d[3] as u32 * keep) as u8;
    }

    fn over_covered(&mut self, x: i32, y: i32, c: Rgba, cover: f32) {
        let i = self.index(x, y);
        self.over(i, c, div255(c[3] as u32 * (cover * 255.0).round() as u32));
    }

    fn span(&mut self, y: i32, x0: i32, x1: i32, c: Rgba) {
        if x1 <= x0 {
            return;
        }
        let (a, b) = (self.index(x0, y), self.index(x1 - 1, y) + 4);
        if c[3] == 255 {
            for p in self.px[a..b].chunks_exact_mut(4) {
                p.copy_from_slice(&c);
            }
        } else {
            for i in (a..b).step_by(4) {
                self.over(i, c, c[3] as u32);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn fill_rect(&mut self, r: Rect, c: Rgba) {
        let a = self.rect.intersect(&r);
        for y in a.y..a.bottom() {
            self.span(y, a.x, a.right(), c);
        }
    }

    /// A rounded rectangle with an anti-aliased edge. Only the rows and columns the corners touch are measured; the rest is a plain fill.
    pub(crate) fn rounded_rect(&mut self, r: Rect, radius: f32, c: Rgba) {
        let a = self.rect.intersect(&r);
        let shape = Rounded::new(r, radius);
        let corner = shape.radius.ceil() as i32;
        let (flat_x0, flat_x1) = (r.x + corner, r.right() - corner);
        for y in a.y..a.bottom() {
            if y >= r.y + corner && y < r.bottom() - corner {
                self.span(y, a.x, a.right(), c);
                continue;
            }
            for x in a.x..a.right().min(flat_x0) {
                self.over_covered(x, y, c, shape.at(x, y));
            }
            self.span(y, a.x.max(flat_x0), a.right().min(flat_x1), c);
            for x in a.x.max(flat_x1).max(flat_x0)..a.right() {
                self.over_covered(x, y, c, shape.at(x, y));
            }
        }
    }

    /// The band of `thickness` pixels just inside a rounded rectangle's edge.
    pub(crate) fn ring(&mut self, r: Rect, radius: f32, thickness: i32, c: Rgba) {
        let a = self.rect.intersect(&r);
        let outer = Rounded::new(r, radius);
        let inner = Rounded::new(Rect::new(r.x + thickness, r.y + thickness, r.w - 2 * thickness, r.h - 2 * thickness), radius - thickness as f32);
        // A box well inside the hole, whose corners the inner shape's own corner arcs cannot cut: nothing there to measure.
        let k = thickness + (0.3 * inner.radius).ceil() as i32 + 1;
        let hole = Rect::new(r.x + k, r.y + k, r.w - 2 * k, r.h - 2 * k);
        for y in a.y..a.bottom() {
            let in_hole_rows = y >= hole.y && y < hole.bottom();
            for x in a.x..a.right() {
                if in_hole_rows && x >= hole.x && x < hole.right() {
                    continue;
                }
                self.over_covered(x, y, c, outer.at(x, y) * (1.0 - inner.at(x, y)));
            }
        }
    }

    /// A triangle, anti-aliased by 4 x 4 sampling.
    pub(crate) fn triangle(&mut self, p: [(f32, f32); 3], c: Rgba) {
        let (min_x, max_x) = (p.iter().map(|q| q.0).fold(f32::MAX, f32::min), p.iter().map(|q| q.0).fold(f32::MIN, f32::max));
        let (min_y, max_y) = (p.iter().map(|q| q.1).fold(f32::MAX, f32::min), p.iter().map(|q| q.1).fold(f32::MIN, f32::max));
        let bounds = Rect::new(min_x.floor() as i32, min_y.floor() as i32, (max_x.ceil() - min_x.floor()) as i32, (max_y.ceil() - min_y.floor()) as i32);
        let a = self.rect.intersect(&bounds);
        let side = |a: (f32, f32), b: (f32, f32), x: f32, y: f32| (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
        for y in a.y..a.bottom() {
            for x in a.x..a.right() {
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let (fx, fy) = (x as f32 + (sx as f32 + 0.5) / 4.0, y as f32 + (sy as f32 + 0.5) / 4.0);
                        let s = [side(p[0], p[1], fx, fy), side(p[1], p[2], fx, fy), side(p[2], p[0], fx, fy)];
                        if s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0) {
                            hits += 1;
                        }
                    }
                }
                self.over_covered(x, y, c, hits as f32 / 16.0);
            }
        }
    }

    /// One line of text with its top-left at `(x, top)`; wider than `max_width` it ends in an ellipsis. Returns the width drawn.
    pub(crate) fn text(&mut self, face: &mut Typeface, text: &str, (x, top): (i32, i32), px: u32, c: Rgba, max_width: Option<i32>) -> i32 {
        let text = match max_width {
            Some(w) => face.fit(text, px, w),
            None => Cow::Borrowed(text),
        };
        let baseline = top + face.ascent(px);
        let mut pen = x as f32;
        for ch in text.chars() {
            let g = face.glyph(ch, px);
            let (gx, gy) = (pen.round() as i32 + g.xmin, baseline - g.ymin - g.h as i32);
            let a = self.rect.intersect(&Rect::new(gx, gy, g.w as i32, g.h as i32));
            for y in a.y..a.bottom() {
                let row = (y - gy) as usize * g.w;
                for px_x in a.x..a.right() {
                    let cover = g.coverage[row + (px_x - gx) as usize] as u32;
                    let i = self.index(px_x, y);
                    self.over(i, c, div255(c[3] as u32 * cover));
                }
            }
            pen += g.advance;
        }
        (pen - x as f32).ceil() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(w: i32, h: i32) -> Canvas {
        Canvas::new(Rect::new(0, 0, w, h))
    }

    #[test]
    fn a_translucent_fill_is_stored_premultiplied() {
        let mut c = canvas(4, 4);
        c.fill_rect(Rect::new(0, 0, 4, 4), [200, 100, 50, 128]);
        let p = c.pixel(1, 1);
        assert_eq!(p[3], 128);
        assert_eq!(p[0], div255(200 * 128) as u8);
        assert_eq!(p[1], div255(100 * 128) as u8);
        assert!(p[0] <= p[3], "a premultiplied channel never exceeds alpha");
    }

    #[test]
    fn two_translucent_fills_compose_as_source_over() {
        let mut c = canvas(1, 1);
        c.fill_rect(Rect::new(0, 0, 1, 1), [255, 0, 0, 255]);
        c.fill_rect(Rect::new(0, 0, 1, 1), [0, 0, 255, 128]);
        let p = c.pixel(0, 0);
        assert_eq!(p[3], 255);
        assert!((p[0] as i32 - 127).abs() <= 1 && (p[2] as i32 - 128).abs() <= 1, "{p:?}");
    }

    #[test]
    fn drawing_outside_the_canvas_is_dropped() {
        let mut c = Canvas::new(Rect::new(10, 10, 20, 20));
        c.fill_rect(Rect::new(-50, -50, 500, 500), [255, 255, 255, 255]);
        assert_eq!(c.pixel(10, 10), [255; 4]);
        assert_eq!(c.pixel(29, 29), [255; 4]);
        let mut d = Canvas::new(Rect::new(10, 10, 20, 20));
        d.fill_rect(Rect::new(0, 0, 10, 10), [255; 4]);
        d.rounded_rect(Rect::new(-5, 25, 100, 100), 14.0, [255; 4]);
        d.ring(Rect::new(0, 0, 15, 15), 8.0, 4, [255; 4]);
        d.triangle([(-30.0, -30.0), (100.0, 0.0), (0.0, 100.0)], [255; 4]);
        d.text(&mut Typeface::new(), "clip", (-10, 0), 26, [255; 4], None);
        assert_eq!(d.into_rgba().len(), 20 * 20 * 4);
    }

    #[test]
    fn a_rounded_rect_cuts_its_corners_and_keeps_its_edges() {
        let mut c = canvas(60, 40);
        c.rounded_rect(Rect::new(0, 0, 60, 40), 14.0, [255; 4]);
        assert!(c.pixel(0, 0)[3] < 16, "the corner pixel is outside the arc");
        assert!(c.pixel(1, 1)[3] < 128);
        assert_eq!(c.pixel(30, 0)[3], 255);
        assert_eq!(c.pixel(0, 20)[3], 255);
        assert_eq!(c.pixel(30, 20)[3], 255);
        assert!((0..8).any(|x| (0..8).any(|y| (1..255).contains(&c.pixel(x, y)[3]))), "the arc is anti-aliased");
    }

    #[test]
    fn a_ring_leaves_its_middle_empty() {
        let mut c = canvas(80, 60);
        c.ring(Rect::new(0, 0, 80, 60), 20.0, 5, [255; 4]);
        assert_eq!(c.pixel(40, 2)[3], 255);
        assert_eq!(c.pixel(2, 30)[3], 255);
        assert_eq!(c.pixel(40, 30)[3], 0);
        assert_eq!(c.pixel(40, 8)[3], 0);
        assert!(c.pixel(0, 0)[3] < 16);
    }

    #[test]
    fn a_triangle_covers_its_inside_only() {
        let mut c = canvas(40, 40);
        c.triangle([(0.0, 0.0), (0.0, 40.0), (40.0, 20.0)], [255; 4]);
        assert_eq!(c.pixel(5, 20)[3], 255);
        assert_eq!(c.pixel(38, 2)[3], 0);
        assert_eq!(c.pixel(38, 38)[3], 0);
    }

    #[test]
    fn text_that_is_too_wide_ends_in_an_ellipsis_that_fits() {
        let mut face = Typeface::new();
        let title = "A very long application title that cannot fit";
        for width in [60, 120, 300] {
            let shown = face.fit(title, 26, width).into_owned();
            assert!(shown.ends_with(face.ellipsis()), "{shown}");
            assert!(face.measure(&shown, 26) <= width, "{shown} is {} wide", face.measure(&shown, 26));
            assert!(shown.len() < title.len() + 3);
        }
        assert_eq!(face.fit("Plex", 26, 300), "Plex");
        assert_eq!(face.fit(title, 26, 4), "", "not even the ellipsis fits");
    }

    #[test]
    fn drawn_text_stays_inside_its_width() {
        let mut face = Typeface::new();
        let mut c = canvas(400, 60);
        let w = c.text(&mut face, "A very long application title", (20, 10), 26, [255; 4], Some(150));
        assert!(w <= 150);
        let rgba = c.into_rgba();
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        for (i, p) in rgba.as_chunks::<4>().0.iter().enumerate() {
            if p[3] > 0 {
                let x = (i % 400) as i32;
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
        assert!(lo >= 20 && hi < 20 + 150, "ink spans {lo}..{hi}");
    }

    #[test]
    fn a_character_the_face_lacks_draws_as_a_question_mark() {
        let mut face = Typeface::new();
        assert!(!face.has('\u{4e2d}'));
        assert_eq!(face.measure("\u{4e2d}", 26), face.measure("?", 26));
        assert_eq!(face.measure("\n", 26), face.measure(" ", 26));
    }

    #[test]
    fn mixing_runs_from_one_colour_to_the_other() {
        assert_eq!(mix([0, 0, 0], [200, 100, 50], 0.0), [0, 0, 0]);
        assert_eq!(mix([0, 0, 0], [200, 100, 50], 1.0), [200, 100, 50]);
        assert_eq!(mix([0, 0, 0], [200, 100, 50], 0.5), [100, 50, 25]);
        assert_eq!(with_alpha([1, 2, 3], 0.5), [1, 2, 3, 128]);
    }
}
