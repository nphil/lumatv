//! Grid-fitted glyphs: the shell's text with its stems and bars on whole pixels (`docs/research/text-render-result.md`).
//!
//! The page is 1920 by 1080 and the display chip's OSD scaler takes it to the panel by about 2.01 with an interpolating filter, so an edge
//! that lies inside a page pixel (a stem 2.3 pixels wide has one grey pixel at each side) comes out two page pixels wide on the panel,
//! twice what an edge between whole pixels does. FreeType's automatic hinter at its strong ("mono") target moves every stem and bar to the
//! grid and the outline is then drawn antialiased, so curves keep their grey edges and do not step. `skrifa` carries a port of that
//! hinter, in Rust: [`GridFit`] is a [`GlyphSource`] that rasters the hinted outline with an exact-area scan converter and is handed to
//! `Fonts::set_glyph_source` (the theme's `text.gridFit`, on in the shipped theme).
//!
//! Only the bitmap changes. The advance is the built-in rasteriser's and every glyph keeps its place on the line, so layout, measure and
//! wrapping do not move; a hinted glyph may stand up to half a pixel from where its outline would be, which is what hinting is.

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, Target};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use std::collections::HashMap;
use ui::text::{GlyphSource, Raster};

/// A font file with its outlines and the hinting instances made for it, one a size.
struct Face {
    outlines: skrifa::outline::OutlineGlyphCollection<'static>,
    sizes: HashMap<u32, HintingInstance>,
}

#[derive(Default)]
pub struct GridFit {
    /// By the address of the font file, which is a `'static` slice and so never moves.
    faces: HashMap<usize, Option<Face>>,
}

impl GridFit {
    pub fn new() -> Self {
        Self::default()
    }

    /// The hinted bitmap of `glyph` of the font file `face` at `px`, or `None` for a glyph with no outline (a space) or one the hinter refuses.
    pub fn glyph(&mut self, face: &'static [u8], px: f32, glyph: u16) -> Option<Raster> {
        let entry = self.faces.entry(face.as_ptr() as usize).or_insert_with(|| {
            let font = FontRef::new(face).ok()?;
            Some(Face { outlines: font.outline_glyphs(), sizes: HashMap::new() })
        });
        let face = entry.as_mut()?;
        if !face.sizes.contains_key(&px.to_bits()) {
            let options = HintingOptions { engine: Engine::Auto(None), target: Target::Mono };
            let instance = HintingInstance::new(&face.outlines, Size::new(px), LocationRef::default(), options).ok()?;
            face.sizes.insert(px.to_bits(), instance);
        }
        let instance = &face.sizes[&px.to_bits()];
        let outline = face.outlines.get(GlyphId::new(u32::from(glyph)))?;
        let mut path = Path::default();
        outline.draw(DrawSettings::hinted(instance, false), &mut path).ok()?;
        path.raster()
    }
}

impl GlyphSource for GridFit {
    fn raster(&mut self, face: &'static [u8], px: f32, glyph: u16) -> Option<Raster> {
        self.glyph(face, px, glyph)
    }
}

/// An outline flattened to line segments, in pixels, y up.
#[derive(Default)]
struct Path {
    lines: Vec<[(f32, f32); 2]>,
    start: (f32, f32),
    at: (f32, f32),
}

impl Path {
    fn line(&mut self, to: (f32, f32)) {
        if to != self.at {
            self.lines.push([self.at, to]);
        }
        self.at = to;
    }

    /// The scan conversion of the flattened outline: exact area coverage (nonzero winding, as the glyph's contours are drawn), rows from the top.
    fn raster(self) -> Option<Raster> {
        if self.lines.is_empty() {
            return None;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for [a, b] in &self.lines {
            for (x, y) in [a, b] {
                x0 = x0.min(*x);
                x1 = x1.max(*x);
                y0 = y0.min(*y);
                y1 = y1.max(*y);
            }
        }
        let (left, bottom) = (x0.floor() as i32, y0.floor() as i32);
        let (width, height) = ((x1.ceil() as i32 - left) as usize, (y1.ceil() as i32 - bottom) as usize);
        if width == 0 || height == 0 {
            return None;
        }
        let top = bottom as f32 + height as f32;
        // one extra row's worth: a segment ending on the right edge adds into the next cell, and the running sum below is over the whole buffer
        let mut acc = vec![0f32; width * height + 4];
        for [a, b] in &self.lines {
            let to = |p: (f32, f32)| ((p.0 - left as f32).clamp(0.0, width as f32), (top - p.1).clamp(0.0, height as f32));
            accumulate(&mut acc, width, to(*a), to(*b));
        }
        let mut sum = 0f32;
        let coverage = acc[..width * height]
            .iter()
            .map(|a| {
                sum += a;
                (sum.abs().min(1.0) * 255.0 + 0.5) as u8
            })
            .collect();
        Some(Raster { xmin: left, ymin: bottom, width, height, coverage })
    }
}

/// Adds the signed area a segment covers to the cells it crosses (the accumulation scan converter of font-rs, after Raph Levien): the running
/// sum along the buffer is then each cell's coverage.
fn accumulate(a: &mut [f32], w: usize, p0: (f32, f32), p1: (f32, f32)) {
    if p0.1 == p1.1 {
        return;
    }
    let (dir, p0, p1) = if p0.1 < p1.1 { (1.0, p0, p1) } else { (-1.0, p1, p0) };
    let dxdy = (p1.0 - p0.0) / (p1.1 - p0.1);
    let mut x = p0.0;
    let ystart = p0.1 as usize;
    let h = a.len() / w.max(1);
    for y in ystart..(p1.1.ceil() as usize).min(h) {
        let row = y * w;
        let dy = ((y + 1) as f32).min(p1.1) - (y as f32).max(p0.1);
        let xnext = x + dxdy * dy;
        let d = dy * dir;
        let (lo, hi) = if x < xnext { (x, xnext) } else { (xnext, x) };
        let lo_floor = lo.floor();
        let lo_i = lo_floor as usize;
        let hi_i = hi.ceil() as usize;
        if hi_i <= lo_i + 1 {
            let mid = 0.5 * (x + xnext) - lo_floor;
            a[row + lo_i] += d - d * mid;
            a[row + lo_i + 1] += d * mid;
        } else {
            let s = 1.0 / (hi - lo);
            let lo_f = lo - lo_floor;
            let a0 = 0.5 * s * (1.0 - lo_f) * (1.0 - lo_f);
            let hi_f = hi - hi.ceil() + 1.0;
            let am = 0.5 * s * hi_f * hi_f;
            a[row + lo_i] += d * a0;
            if hi_i == lo_i + 2 {
                a[row + lo_i + 1] += d * (1.0 - a0 - am);
            } else {
                let a1 = s * (1.5 - lo_f);
                a[row + lo_i + 1] += d * (a1 - a0);
                for xi in lo_i + 2..hi_i - 1 {
                    a[row + xi] += d * s;
                }
                let a2 = a1 + (hi_i - lo_i - 3) as f32 * s;
                a[row + hi_i - 1] += d * (1.0 - a2 - am);
            }
            a[row + hi_i] += d * am;
        }
        x = xnext;
    }
}

impl OutlinePen for Path {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = (x, y);
        self.at = (x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.line((x, y));
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.at, (cx, cy), (x, y));
        let n = steps(&[p0, p1, p2]);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            self.line((u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0, u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1));
        }
    }

    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.at, (c1x, c1y), (c2x, c2y), (x, y));
        let n = steps(&[p0, p1, p2, p3]);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.line((b0 * p0.0 + b1 * p1.0 + b2 * p2.0 + b3 * p3.0, b0 * p0.1 + b1 * p1.1 + b2 * p2.1 + b3 * p3.1));
        }
    }

    fn close(&mut self) {
        self.line(self.start);
    }
}

/// Segments for a curve with these control points: the control polygon's length over a third of a pixel, at least two.
fn steps(points: &[(f32, f32)]) -> usize {
    let len: f32 = points.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).sum();
    ((len * 3.0).ceil() as usize).clamp(2, 48)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::text::{Fonts, Type, Weight};

    const MEDIUM: &[u8] = include_bytes!("../../../../../LumaShow/lumashow/assets/fonts/Manrope-Medium.ttf");

    fn glyph_index(c: char) -> u16 {
        let font = fontdue::Font::from_bytes(MEDIUM, fontdue::FontSettings::default()).unwrap();
        font.lookup_glyph_index(c)
    }

    fn rows(r: &Raster) -> Vec<&[u8]> {
        r.coverage.chunks(r.width).collect()
    }

    /// The middle row of a tall straight stem: the coverage of its pixels, left to right.
    fn middle(r: &Raster) -> Vec<u8> {
        rows(r)[r.height / 2].to_vec()
    }

    #[test]
    fn a_stem_has_two_whole_pixel_edges_where_the_built_in_one_leaves_grey_at_both_sides() {
        let mut fit = GridFit::new();
        let mut plain = Fonts::new();
        for px in [20u32, 22, 24, 26, 28, 32] {
            let hinted = fit.glyph(MEDIUM, px as f32, glyph_index('l')).expect("l has an outline");
            let line = middle(&hinted);
            let grey = line.iter().filter(|&&c| c != 0 && c != 255).count();
            assert_eq!(grey, 0, "{px} px: a stem of 'l' is all or nothing across, got {line:?}");
            let (m, built_in) = plain.raster(Type { px, weight: Weight::Medium }, 'l');
            let b = &built_in[m.width * (m.height / 2)..][..m.width];
            assert!(b.iter().any(|&c| c != 0 && c != 255), "{px} px: the built-in stem has a grey pixel at its edge {b:?}");
        }
    }

    #[test]
    fn a_bar_has_whole_pixel_edges_too() {
        let mut fit = GridFit::new();
        let r = fit.glyph(MEDIUM, 26.0, glyph_index('E')).unwrap();
        // the column through the arms' middle: the top arm, the middle arm and the foot are runs of 255 with nothing grey between rows
        let col = r.width / 2 + 1;
        let column: Vec<u8> = rows(&r).iter().map(|row| row[col]).collect();
        let grey = column.iter().filter(|&&c| c != 0 && c != 255).count();
        assert_eq!(grey, 0, "the arms of E sit on whole rows: {column:?}");
    }

    #[test]
    fn the_ink_stays_within_a_stem_rounding_of_the_built_in_rasteriser_and_the_advance_is_untouched() {
        let mut plain = Fonts::new();
        let mut hinted = Fonts::new();
        hinted.set_glyph_source(Some(Box::new(GridFit::new())));
        for (px, weight) in [
            (19, Weight::Regular),
            (21, Weight::Regular),
            (24, Weight::Medium),
            (26, Weight::Medium),
            (28, Weight::Medium),
            (26, Weight::SemiBold),
            (40, Weight::SemiBold),
            (92, Weight::Medium),
        ] {
            let ty = Type { px, weight };
            let (mut a, mut b) = (0u64, 0u64);
            for c in "Hamburger fonts illuminate 0123456789".chars() {
                let (ma, ca) = plain.raster(ty, c);
                let (mb, cb) = hinted.raster(ty, c);
                a += ca.iter().map(|&v| u64::from(v)).sum::<u64>();
                b += cb.iter().map(|&v| u64::from(v)).sum::<u64>();
                assert_eq!(ma.advance, mb.advance, "{c} at {px}: layout does not move");
            }
            let ratio = b as f64 / a as f64;
            assert!((0.75..1.15).contains(&ratio), "{px} px {weight:?}: ink {ratio:.3} of the built-in's");
            assert_eq!(plain.measure("Hamburger", ty).width, hinted.measure("Hamburger", ty).width);
        }
    }

    #[test]
    fn a_curve_keeps_its_grey_edge_and_nothing_goes_over_the_ink() {
        let mut fit = GridFit::new();
        let r = fit.glyph(MEDIUM, 26.0, glyph_index('o')).unwrap();
        let grey = r.coverage.iter().filter(|&&c| c != 0 && c != 255).count();
        assert!(grey > 20, "the bowl of o is antialiased ({grey} grey pixels)");
        assert!(r.coverage.contains(&255));
    }

    #[test]
    fn the_theme_switch_makes_every_glyph_of_the_kit_grid_fitted_and_off_leaves_them_as_they_were() {
        use crate::kit::Kit;
        use lumatv_tokens::Tokens;
        use serde_json::json;
        use ui::lucent::material::MaterialTier;
        use ui::lucent::palette::Mode;
        let ty = Type { px: 26, weight: Weight::Medium };
        let stem_is_grey = |kit: &mut Kit| {
            let (m, c) = kit.ui.fonts.raster(ty, 'l');
            c[m.width * (m.height / 2)..][..m.width].iter().any(|&v| v != 0 && v != 255)
        };
        let h = ui::testing::harness();
        let mut off = Kit::new(h.ui.scene.clone(), Tokens::from_json(&json!({"text": {"gridFit": false}})), Mode::Dark, MaterialTier::T1, 1.0).unwrap();
        assert!(stem_is_grey(&mut off), "off: the built-in glyphs");
        let h = ui::testing::harness();
        let mut on = Kit::new(h.ui.scene.clone(), Tokens::shipped(), Mode::Dark, MaterialTier::T1, 1.0).unwrap();
        assert!(!stem_is_grey(&mut on), "on in the shipped theme: stems of whole pixels");
        assert_eq!(on.ui.fonts.measure("Hamburger", ty).width, off.ui.fonts.measure("Hamburger", ty).width, "the layout is the same");
    }

    #[test]
    fn a_space_has_no_pixels_and_is_left_to_the_built_in_rasteriser() {
        let mut fit = GridFit::new();
        assert!(fit.glyph(MEDIUM, 26.0, glyph_index(' ')).is_none());
        let mut hinted = Fonts::new();
        hinted.set_glyph_source(Some(Box::new(GridFit::new())));
        let (m, c) = hinted.raster(Type { px: 26, weight: Weight::Medium }, ' ');
        assert_eq!((m.width, m.height, c.len()), (0, 0, 0));
    }

    #[test]
    fn a_source_taken_away_gives_the_built_in_pixels_back() {
        let ty = Type { px: 26, weight: Weight::Medium };
        let mut f = Fonts::new();
        let before = f.raster(ty, 'g').1.to_vec();
        f.set_glyph_source(Some(Box::new(GridFit::new())));
        assert_ne!(f.raster(ty, 'g').1, &before[..], "the grid-fitted g differs");
        f.set_glyph_source(None);
        assert_eq!(f.raster(ty, 'g').1, &before[..]);
    }

    #[test]
    fn the_scan_converter_covers_a_square_exactly() {
        let mut p = Path::default();
        p.move_to(0.5, 0.0);
        p.line_to(2.5, 0.0);
        p.line_to(2.5, 2.0);
        p.line_to(0.5, 2.0);
        p.close();
        let r = p.raster().unwrap();
        assert_eq!((r.xmin, r.ymin, r.width, r.height), (0, 0, 3, 2));
        assert_eq!(r.coverage, vec![128, 255, 128, 128, 255, 128]);
    }
}
