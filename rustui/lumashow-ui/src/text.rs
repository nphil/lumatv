//! Text on the CPU: glyphs are rasterised once per (weight, character, size) and blended from the cache after
//! that (experiments/s2-raster: 200 cached glyphs ≈ 0.45 ms on the Echo Show; an uncached 180 px glyph ≈ 10 ms,
//! so big sizes are warmed at start, never on a frame).

use compositor_protocol::Rgba;
use fontdue::{Font, FontSettings, Metrics};
use std::collections::HashMap;

/// Manrope, the Lucent typeface, as static instances of the pinned variable font (SIL Open Font License;
/// assets/fonts/Manrope-OFL.txt and Manrope.SOURCE). The glyph rasteriser cannot select a variable font's
/// weight, and the variable font's default instance is ExtraLight.
const FACES: [&[u8]; 3] = [
    include_bytes!("../../../../LumaShow/lumashow/assets/fonts/Manrope-Regular.ttf"),
    include_bytes!("../../../../LumaShow/lumashow/assets/fonts/Manrope-Medium.ttf"),
    include_bytes!("../../../../LumaShow/lumashow/assets/fonts/Manrope-SemiBold.ttf"),
];

/// The Lucent weights: body and captions Regular (400), display and labels Medium (500), titles SemiBold (600).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular = 0,
    Medium = 1,
    SemiBold = 2,
}

/// A text size and weight, as the design tokens name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Type {
    pub px: u32,
    pub weight: Weight,
}

/// The glyph index of Manrope's tabular `zero.tf` (`one.tf` to `nine.tf` follow it): the font's `tnum` feature swaps each digit for
/// its tabular form, the same outline set in a cell of one width for every digit. The rasteriser has no OpenType features, so the
/// forms are drawn by index. The three bundled faces are instances of one variable font and share its glyph order; the tests hold the
/// index to what it is for (ten glyphs, one advance, [`TABULAR_EM`]).
const TABULAR_ZERO: u16 = 551;

/// How wide a digit is in Manrope's tabular figures, in em: one width for every digit and every weight (1240 of the font's 2000
/// units), which is what a browser lays out `font-variant-numeric: tabular-nums` with.
pub const TABULAR_EM: f32 = 0.62;

/// The pixels of one glyph made by a [`GlyphSource`]: the bitmap's left edge is `xmin` from the pen and its bottom edge `ymin` above the
/// baseline, as the rasteriser's own metrics say it, and `coverage` is `width * height` bytes, rows packed, 0 to 255.
pub struct Raster {
    pub xmin: i32,
    pub ymin: i32,
    pub width: usize,
    pub height: usize,
    pub coverage: Vec<u8>,
}

/// A rasteriser that stands in for the built-in one (LumaTV's grid-fitted glyphs). `face` is the font file of the weight, `px` the size and
/// `glyph` the font's glyph index; `None` leaves that glyph to the built-in rasteriser. The advance is the built-in one's whatever the source
/// does, so layout and measure never change.
pub trait GlyphSource: Send {
    fn raster(&mut self, face: &'static [u8], px: f32, glyph: u16) -> Option<Raster>;
}

pub struct Fonts {
    faces: [Font; 3],
    source: Option<Box<dyn GlyphSource>>,
    glyphs: HashMap<(Type, char), (Metrics, Vec<u8>)>,
    /// The tabular forms of the digits, by the digit they stand for.
    tabular: HashMap<(Type, char), (Metrics, Vec<u8>)>,
}

/// A premultiplied RGBA buffer `width` pixels wide (four bytes per pixel, rows packed).
pub struct Canvas<'a> {
    pub pixels: &'a mut [u8],
    pub width: usize,
}

/// The pixel extent of a line of text: `width`, and `ascent`/`descent` around the baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

impl Extent {
    pub fn height(&self) -> f32 {
        self.ascent + self.descent
    }
}

/// How a glyph sits on its line, as the rasteriser reports it: the pen advances by `advance` after it, its bitmap's left edge
/// is `xmin` from the pen and its bottom edge `ymin` above the baseline (negative below it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphMetrics {
    pub xmin: i32,
    pub ymin: i32,
    pub width: usize,
    pub height: usize,
    pub advance: f32,
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    pub fn new() -> Self {
        let faces = FACES.map(|bytes| {
            Font::from_bytes(bytes, FontSettings::default()).expect("the bundled Manrope parses")
        });
        Self {
            faces,
            source: None,
            glyphs: HashMap::new(),
            tabular: HashMap::new(),
        }
    }

    /// Puts `source` in place of the built-in rasteriser for every glyph made from now on (or takes it away with `None`); what was rastered
    /// before is forgotten, so a change of source is made before the first text, or its atlases are made again.
    pub fn set_glyph_source(&mut self, source: Option<Box<dyn GlyphSource>>) {
        self.source = source;
        self.glyphs.clear();
        self.tabular.clear();
    }

    /// The glyph `index` of the face of `weight` at `px`: the source's pixels when it has them, else the built-in rasteriser's.
    fn make(
        font: &Font,
        face: &'static [u8],
        source: &mut Option<Box<dyn GlyphSource>>,
        index: u16,
        px: f32,
    ) -> (Metrics, Vec<u8>) {
        let mut metrics = font.metrics_indexed(index, px);
        if let Some(r) = source.as_mut().and_then(|s| s.raster(face, px, index)) {
            assert_eq!(r.coverage.len(), r.width * r.height, "a source's glyph has its pixels");
            metrics.xmin = r.xmin;
            metrics.ymin = r.ymin;
            metrics.width = r.width;
            metrics.height = r.height;
            return (metrics, r.coverage);
        }
        font.rasterize_indexed(index, px)
    }

    fn glyph(&mut self, ty: Type, c: char) -> &(Metrics, Vec<u8>) {
        let font = &self.faces[ty.weight as usize];
        let source = &mut self.source;
        self.glyphs.entry((ty, c)).or_insert_with(|| {
            let index = font.lookup_glyph_index(c);
            if source.is_none() {
                return font.rasterize(c, ty.px as f32);
            }
            Self::make(font, FACES[ty.weight as usize], source, index, ty.px as f32)
        })
    }

    /// What `c` is drawn with: its tabular form when `tabular` is asked for and it is a digit, else the glyph it always is.
    fn form(&mut self, ty: Type, c: char, tabular: bool) -> &(Metrics, Vec<u8>) {
        if tabular && c.is_ascii_digit() {
            let font = &self.faces[ty.weight as usize];
            let source = &mut self.source;
            let index = TABULAR_ZERO + (c as u16 - '0' as u16);
            self.tabular.entry((ty, c)).or_insert_with(|| {
                Self::make(font, FACES[ty.weight as usize], source, index, ty.px as f32)
            })
        } else {
            self.glyph(ty, c)
        }
    }

    /// Rasterises `chars` now so drawing them later never pays the first-raster cost.
    pub fn warm(&mut self, chars: &str, ty: Type) {
        for c in chars.chars() {
            self.glyph(ty, c);
        }
    }

    /// [`Fonts::warm`] for text drawn with [`Fonts::draw_tabular`]: the digits among `chars` in their tabular forms, the rest as they are.
    pub fn warm_tabular(&mut self, chars: &str, ty: Type) {
        for c in chars.chars() {
            self.form(ty, c, true);
        }
    }

    /// How many glyphs are rastered and kept (tabular forms included): a test of warming sees whether drawing text later adds any.
    pub fn cached_glyphs(&self) -> usize {
        self.glyphs.len() + self.tabular.len()
    }

    /// The raster of `c` at `ty` that `draw` blends from: its metrics, and the coverage bytes (`width * height`, rows packed,
    /// 0 to 255). It is the cache's, made now if it was not there; a glyph atlas is built from exactly this
    /// (`Atlas::prepare`), so a glyph run is pixel for pixel what `draw` makes.
    pub fn raster(&mut self, ty: Type, c: char) -> (GlyphMetrics, &[u8]) {
        let (m, coverage) = self.glyph(ty, c);
        (
            GlyphMetrics {
                xmin: m.xmin,
                ymin: m.ymin,
                width: m.width,
                height: m.height,
                advance: m.advance_width,
            },
            coverage,
        )
    }

    /// The extent of `text`, from the font's line metrics (so every string of one type has the same ascent and
    /// descent and lines stay aligned).
    pub fn measure(&mut self, text: &str, ty: Type) -> Extent {
        self.measure_in(text, ty, false)
    }

    /// [`Fonts::measure`] of text set with the font's tabular figures: every digit [`TABULAR_EM`] wide.
    pub fn measure_tabular(&mut self, text: &str, ty: Type) -> Extent {
        self.measure_in(text, ty, true)
    }

    fn measure_in(&mut self, text: &str, ty: Type, tabular: bool) -> Extent {
        let width = text
            .chars()
            .map(|c| self.form(ty, c, tabular).0.advance_width)
            .sum();
        let line = self.faces[ty.weight as usize]
            .horizontal_line_metrics(ty.px as f32)
            .expect("Manrope has horizontal metrics");
        Extent {
            width,
            ascent: line.ascent,
            descent: -line.descent,
        }
    }

    /// Blends `text` in `colour` into `canvas`, starting at `origin` = (x, baseline). Pixels outside the canvas
    /// are skipped. Returns the pen position after the text.
    pub fn draw(
        &mut self,
        canvas: &mut Canvas<'_>,
        text: &str,
        ty: Type,
        origin: (f32, f32),
        colour: Rgba,
    ) -> f32 {
        self.draw_in(canvas, text, ty, origin, colour, false)
    }

    /// [`Fonts::draw`] of text set with the font's tabular figures: each digit is the font's `tnum` form of it, [`TABULAR_EM`] wide,
    /// as a browser sets `font-variant-numeric: tabular-nums`. Everything else is as `draw` makes it.
    pub fn draw_tabular(
        &mut self,
        canvas: &mut Canvas<'_>,
        text: &str,
        ty: Type,
        origin: (f32, f32),
        colour: Rgba,
    ) -> f32 {
        self.draw_in(canvas, text, ty, origin, colour, true)
    }

    fn draw_in(
        &mut self,
        canvas: &mut Canvas<'_>,
        text: &str,
        ty: Type,
        origin: (f32, f32),
        colour: Rgba,
        tabular: bool,
    ) -> f32 {
        let (x, baseline) = origin;
        let (pixels, width) = (&mut *canvas.pixels, canvas.width);
        let height = pixels.len() / (width * 4);
        let mut pen = x;
        for c in text.chars() {
            let (m, coverage) = self.form(ty, c, tabular);
            let left = (pen + m.xmin as f32).round() as i64;
            let top = (baseline - m.height as f32 - m.ymin as f32).round() as i64;
            for row in 0..m.height {
                let y = top + row as i64;
                if y < 0 || y >= height as i64 {
                    continue;
                }
                for col in 0..m.width {
                    let x = left + col as i64;
                    if x < 0 || x >= width as i64 {
                        continue;
                    }
                    let a = u32::from(coverage[row * m.width + col]);
                    if a != 0 {
                        blend(
                            &mut pixels[(y as usize * width + x as usize) * 4..][..4],
                            colour,
                            a,
                        );
                    }
                }
            }
            pen += m.advance_width;
        }
        pen
    }
}

/// Source-over of `colour` at coverage `a` (0..=255) onto one premultiplied pixel.
pub(crate) fn blend(dst: &mut [u8], colour: Rgba, a: u32) {
    let src = [colour.0, colour.1, colour.2, colour.3].map(|c| u32::from(c) * a / 255);
    let keep = 255 - src[3];
    for (d, s) in dst.iter_mut().zip(src) {
        *d = (s + u32::from(*d) * keep / 255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: Rgba = Rgba(255, 255, 255, 255);
    const BODY: Type = Type {
        px: 24,
        weight: Weight::Regular,
    };

    #[test]
    fn drawn_text_covers_pixels_inside_its_measured_box_and_none_outside() {
        let mut fonts = Fonts::new();
        let (w, h) = (200usize, 60usize);
        let mut px = vec![0u8; w * h * 4];
        let ty = Type {
            px: 32,
            weight: Weight::Regular,
        };
        let e = fonts.measure("Hg", ty);
        let baseline = 10.0 + e.ascent;
        fonts.draw(
            &mut Canvas {
                pixels: &mut px,
                width: w,
            },
            "Hg",
            ty,
            (20.0, baseline),
            WHITE,
        );
        let mut inked = 0;
        for y in 0..h {
            for x in 0..w {
                if px[(y * w + x) * 4 + 3] != 0 {
                    inked += 1;
                    assert!(
                        (19.0..=20.0 + e.width + 1.0).contains(&(x as f32)),
                        "ink at x {x} outside the width"
                    );
                    assert!(
                        (9.0..=baseline + e.descent + 1.0).contains(&(y as f32)),
                        "ink at y {y} outside the line"
                    );
                }
            }
        }
        assert!(inked > 100, "only {inked} pixels inked");
    }

    #[test]
    fn premultiplied_blending_never_exceeds_alpha_and_full_coverage_replaces() {
        let mut dst = [10, 20, 30, 40];
        blend(&mut dst, Rgba(100, 50, 25, 128), 255);
        assert!(
            dst[..3].iter().all(|&c| c <= dst[3]),
            "colour above alpha: {dst:?}"
        );
        let mut dst = [10, 20, 30, 40];
        blend(&mut dst, WHITE, 255);
        assert_eq!(dst, [255, 255, 255, 255]);
    }

    #[test]
    fn every_string_of_one_type_shares_ascent_and_descent_and_heavier_is_wider() {
        let mut fonts = Fonts::new();
        assert_eq!(
            fonts.measure("ace", BODY).ascent,
            fonts.measure("Tg|", BODY).ascent
        );
        assert!(fonts.measure("Wide text", BODY).width > fonts.measure("i", BODY).width);
        let bold = Type {
            weight: Weight::SemiBold,
            ..BODY
        };
        assert!(
            fonts.measure("Wide text", bold).width > fonts.measure("Wide text", BODY).width,
            "the faces must really differ in weight"
        );
    }

    #[test]
    fn text_off_the_buffer_is_clipped_not_a_panic() {
        let mut fonts = Fonts::new();
        let mut px = vec![0u8; 20 * 10 * 4];
        fonts.draw(
            &mut Canvas {
                pixels: &mut px,
                width: 20,
            },
            "Clipped",
            BODY,
            (-30.0, 5.0),
            WHITE,
        );
    }

    /// The columns (first, last + 1) and rows (first, last + 1) that have any ink in a `w` pixel wide buffer.
    fn ink(px: &[u8], w: usize) -> ((usize, usize), (usize, usize)) {
        let (mut cols, mut rows) = ((usize::MAX, 0), (usize::MAX, 0));
        for (i, p) in px.chunks_exact(4).enumerate() {
            if p[3] != 0 {
                let (x, y) = (i % w, i / w);
                cols = (cols.0.min(x), cols.1.max(x + 1));
                rows = (rows.0.min(y), rows.1.max(y + 1));
            }
        }
        (cols, rows)
    }

    #[test]
    fn the_tabular_figures_are_ten_digits_of_one_width_in_every_weight_and_size_and_the_plain_ones_are_not()
     {
        let mut fonts = Fonts::new();
        for weight in [Weight::Regular, Weight::Medium, Weight::SemiBold] {
            for px in [24u32, 40, 208] {
                let ty = Type { px, weight };
                for d in '0'..='9' {
                    let width = fonts.measure_tabular(&d.to_string(), ty).width;
                    assert!(
                        (width - TABULAR_EM * px as f32).abs() < 0.01,
                        "{d} at {px} px, {weight:?}: {width}"
                    );
                }
                let (one, zero) = (fonts.measure("1", ty).width, fonts.measure("0", ty).width);
                assert!(
                    zero - one > 0.1 * px as f32,
                    "the plain ones differ: {zero} {one}"
                );
            }
        }
    }

    #[test]
    fn a_tabular_digit_is_the_outline_of_the_plain_one_set_where_the_font_sets_it_in_its_cell() {
        // Where the font's `tnum` forms of Manrope Medium put the left edge of each digit's ink, in 2000ths of an em (read from the
        // font's own glyphs `zero.tf` to `nine.tf`): the table holds the indices to being those glyphs and not their neighbours.
        const LEFT: [u32; 10] = [136, 330, 140, 138, 139, 140, 129, 203, 149, 129];
        let mut fonts = Fonts::new();
        let ty = Type {
            px: 208,
            weight: Weight::Medium,
        };
        for (i, d) in ('0'..='9').enumerate() {
            let plain = fonts.glyph(ty, d).0;
            let tab = fonts.form(ty, d, true).0;
            assert_eq!(
                tab.xmin,
                (LEFT[i] as f32 * 208.0 / 2000.0).floor() as i32,
                "{d}: the left edge of its tabular form"
            );
            assert_eq!(
                (tab.ymin, tab.height),
                (plain.ymin, plain.height),
                "{d}: as tall"
            );
            assert!(
                tab.width.abs_diff(plain.width) <= 1,
                "{d}: as wide ({} against {})",
                tab.width,
                plain.width
            );
        }
    }

    #[test]
    fn tabular_text_draws_as_plain_text_does_where_it_has_no_digits_and_the_digits_in_their_cells_where_it_has()
     {
        let mut fonts = Fonts::new();
        let ty = Type {
            px: 40,
            weight: Weight::Medium,
        };
        let draw = |fonts: &mut Fonts, text: &str, tabular: bool| {
            let mut px = vec![0u8; 400 * 60 * 4];
            let mut canvas = Canvas {
                pixels: &mut px,
                width: 400,
            };
            let end = if tabular {
                fonts.draw_tabular(&mut canvas, text, ty, (10.0, 44.0), WHITE)
            } else {
                fonts.draw(&mut canvas, text, ty, (10.0, 44.0), WHITE)
            };
            (px, end)
        };
        let (plain, a) = draw(&mut fonts, "Wednesday Oct", false);
        let (tab, b) = draw(&mut fonts, "Wednesday Oct", true);
        assert_eq!((plain, a), (tab, b));
        // "Mon 06:30" is four digits of 0.62 em and a colon and a space and three letters, the sum the browser lays it out with.
        let (_, end) = draw(&mut fonts, "06:30", true);
        let colon = fonts.measure(":", ty).width;
        assert!(
            (end - 10.0 - (4.0 * 0.62 * 40.0 + colon)).abs() < 0.01,
            "{end}"
        );
        assert_eq!(fonts.measure_tabular("06:30", ty).width, end - 10.0);
    }

    #[test]
    fn warming_the_tabular_digits_leaves_a_drawing_of_any_of_them_nothing_to_raster() {
        let mut fonts = Fonts::new();
        let ty = Type {
            px: 96,
            weight: Weight::Medium,
        };
        let before = fonts.cached_glyphs();
        fonts.warm_tabular("0123456789:", ty);
        assert_eq!(
            fonts.cached_glyphs(),
            before + 11,
            "ten tabular forms and the colon"
        );
        let warm = fonts.cached_glyphs();
        let mut px = vec![0u8; 800 * 120 * 4];
        let mut canvas = Canvas {
            pixels: &mut px,
            width: 800,
        };
        fonts.draw_tabular(&mut canvas, "19:47", ty, (0.0, 100.0), WHITE);
        fonts.draw_tabular(&mut canvas, "20:58", ty, (0.0, 100.0), WHITE);
        assert_eq!(
            fonts.cached_glyphs(),
            warm,
            "no glyph is rastered for a reading"
        );
        let (cols, rows) = ink(&px, 800);
        assert!(cols.1 > cols.0 && rows.1 > rows.0);
    }
}
