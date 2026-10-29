//! Marks: the small shapes a row, a key or a message paints beside its words: an icon, a hairline, a disc, a bar of a signal, a
//! tinted square. A mark is a *shape of coverage* (a byte a pixel, as a glyph's) in one colour, so it can be made two ways that
//! agree to the pixel: blended into a picture ([`Mark::paint`], what a layer of pixels does) or kept in the glyph atlas as a
//! [`Sprite`] and drawn as one more quad of the run that carries the words ([`crate::components::TextLayer`]), at no layer and no
//! pixel of its own.
//!
//! Every colour here is premultiplied, as every colour of the toolkit is: a mark of colour `c` and coverage `a` makes the pixel
//! `c * a / 255` over what is under it, which is what text of that colour makes.

use super::icon::Icon;
use crate::atlas::Sprite;
use crate::text::blend;
use compositor_protocol::Rgba;

/// What a mark is the shape of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Form {
    /// An icon of the toolkit, `side` pixels square.
    Icon(Icon, u32),
    /// A line of this width and one pixel high.
    Rule(u32),
    /// A disc of this diameter.
    Disc(u32),
    /// A rounded rectangle.
    Round { w: u32, h: u32, radius: u32 },
    /// A bar of a signal mark: its four corner pixels take half the coverage.
    Bar { w: u32, h: u32 },
    /// A rounded rectangle of one level of the ink, `alpha` of 255: a hairline at a tenth of the ink, a tinted square.
    Veil {
        w: u32,
        h: u32,
        radius: u32,
        alpha: u8,
    },
    /// A rounded bar hatched with diagonal stripes of two levels of the ink (what a redacted line of text is drawn as): the
    /// stripes run up to the right and are 7 units wide measured across them, the darker one first from the top-left corner, at
    /// `dark` and `light` of 255.
    Hatch {
        w: u32,
        h: u32,
        radius: u32,
        dark: u8,
        light: u8,
    },
}

impl Form {
    /// The width and height of the mark's pixels.
    pub fn size(&self) -> (u32, u32) {
        match *self {
            Form::Icon(_, side) => (side, side),
            Form::Rule(w) => (w, 1),
            Form::Disc(d) => (d, d),
            Form::Round { w, h, .. } | Form::Bar { w, h } => (w, h),
            Form::Veil { w, h, .. } | Form::Hatch { w, h, .. } => (w, h),
        }
    }

    /// The sprite the atlas keeps for it. A hairline is one strip of the atlas's width, cut to the width asked.
    pub fn sprite(&self) -> Sprite {
        match *self {
            Form::Icon(icon, side) => Sprite::Icon(icon, side),
            Form::Rule(_) => Sprite::Rule,
            Form::Disc(d) => Sprite::Disc(d),
            Form::Round { w, h, radius } => Sprite::Round(w, h, radius),
            Form::Bar { w, h } => Sprite::Bar(w, h),
            Form::Veil {
                w,
                h,
                radius,
                alpha,
            } => Sprite::Veil(w, h, radius, alpha),
            Form::Hatch {
                w,
                h,
                radius,
                dark,
                light,
            } => Sprite::Hatch(w, h, radius, dark, light),
        }
    }

    /// The coverage of the mark: `width * height` bytes, rows packed, 0 to 255.
    pub fn coverage(&self) -> Vec<u8> {
        let (w, h) = self.size();
        let (w, h) = (w as usize, h as usize);
        match *self {
            Form::Icon(icon, side) => {
                let mut rgba = vec![0u8; w * h * 4];
                icon.draw(&mut rgba, side, Rgba(255, 255, 255, 255));
                rgba.chunks_exact(4).map(|p| p[3]).collect()
            }
            Form::Rule(_) => vec![255; w],
            Form::Disc(_) => {
                let r = w as f32 / 2.0;
                let mut out = Vec::with_capacity(w * h);
                for y in 0..h {
                    for x in 0..w {
                        let d =
                            ((x as f32 + 0.5 - r).powi(2) + (y as f32 + 0.5 - r).powi(2)).sqrt();
                        out.push(to_byte((r - d + 0.5).clamp(0.0, 1.0)));
                    }
                }
                out
            }
            Form::Round { radius, .. } => {
                let r = radius as f32;
                let mut out = Vec::with_capacity(w * h);
                for y in 0..h {
                    for x in 0..w {
                        // How far this pixel's centre is outside the corner circle it is nearest to (negative: inside).
                        let cx = (x as f32 + 0.5).clamp(r, w as f32 - r);
                        let cy = (y as f32 + 0.5).clamp(r, h as f32 - r);
                        let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2))
                            .sqrt()
                            - r;
                        out.push(to_byte((0.5 - d).clamp(0.0, 1.0)));
                    }
                }
                out
            }
            Form::Veil {
                w: vw,
                h: vh,
                radius,
                alpha,
            } => {
                if radius == 0 {
                    // A square corner has no edge to blur: every pixel is whole.
                    return vec![alpha; w * h];
                }
                let round = Form::Round {
                    w: vw,
                    h: vh,
                    radius,
                }
                .coverage();
                round
                    .into_iter()
                    .map(|c| (u32::from(c) * u32::from(alpha) / 255) as u8)
                    .collect()
            }
            Form::Hatch {
                w: hw,
                h: hh,
                radius,
                dark,
                light,
            } => {
                let round = Form::Round {
                    w: hw,
                    h: hh,
                    radius,
                }
                .coverage();
                let mut out = Vec::with_capacity(round.len());
                for y in 0..h {
                    for x in 0..w {
                        // How far along the gradient's own line (135 degrees: towards the bottom right) the pixel's centre is.
                        let along = (x as f32 + 0.5 + y as f32 + 0.5) / std::f32::consts::SQRT_2;
                        let level = if (along / 7.0).floor() as i64 % 2 == 0 {
                            dark
                        } else {
                            light
                        };
                        out.push((u32::from(round[y * w + x]) * u32::from(level) / 255) as u8);
                    }
                }
                out
            }
            Form::Bar { .. } => {
                let mut out = vec![255u8; w * h];
                for y in [0, h - 1] {
                    for x in [0, w - 1] {
                        out[y * w + x] = 127;
                    }
                }
                out
            }
        }
    }
}

fn to_byte(cover: f32) -> u8 {
    (cover * 255.0).round() as u8
}

/// A shape in a colour at a place: the top-left corner of its pixels, in the pixels of the layer that carries it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub form: Form,
    pub x: i32,
    pub y: i32,
    /// Premultiplied.
    pub colour: Rgba,
}

impl Mark {
    pub fn icon(icon: Icon, side: u32, x: i32, y: i32, colour: Rgba) -> Self {
        Self {
            form: Form::Icon(icon, side),
            x,
            y,
            colour,
        }
    }

    /// A line `w` wide with its left end at (`x`, `y`).
    pub fn rule(x: i32, y: i32, w: u32, colour: Rgba) -> Self {
        Self {
            form: Form::Rule(w),
            x,
            y,
            colour,
        }
    }

    pub fn disc(x: i32, y: i32, diameter: u32, colour: Rgba) -> Self {
        Self {
            form: Form::Disc(diameter),
            x,
            y,
            colour,
        }
    }

    pub fn round(x: i32, y: i32, w: u32, h: u32, radius: u32, colour: Rgba) -> Self {
        Self {
            form: Form::Round { w, h, radius },
            x,
            y,
            colour,
        }
    }

    /// A rounded rectangle of one level of `colour`, `alpha` of 255 (a [`Form::Veil`]): what a tinted square is, in the ink of the glyph
    /// that stands on it. Not the same as [`Mark::round`] in `colour` at `alpha`, which is a colour of its own and takes an ink of its
    /// own in a run (four are all a run has); the two agree to the pixel but for a unit of rounding.
    pub fn veil(x: i32, y: i32, w: u32, h: u32, radius: u32, alpha: u8, colour: Rgba) -> Self {
        Self {
            form: Form::Veil {
                w,
                h,
                radius,
                alpha,
            },
            x,
            y,
            colour,
        }
    }

    pub fn bar(x: i32, y: i32, w: u32, h: u32, colour: Rgba) -> Self {
        Self {
            form: Form::Bar { w, h },
            x,
            y,
            colour,
        }
    }

    /// Blends the mark into `pixels` (premultiplied RGBA, rows of `width` pixels) the way text of its colour is: what falls
    /// outside the picture is skipped.
    pub fn paint(&self, pixels: &mut [u8], width: usize) {
        let (w, h) = self.form.size();
        let rows = pixels.len() / 4 / width.max(1);
        let coverage = self.form.coverage();
        for row in 0..h as usize {
            let y = self.y + row as i32;
            if y < 0 || y as usize >= rows {
                continue;
            }
            for col in 0..w as usize {
                let x = self.x + col as i32;
                if x < 0 || x as usize >= width {
                    continue;
                }
                let a = u32::from(coverage[row * w as usize + col]);
                if a != 0 {
                    blend(
                        &mut pixels[(y as usize * width + x as usize) * 4..][..4],
                        self.colour,
                        a,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: Rgba = Rgba(255, 255, 255, 255);

    fn painted(mark: &Mark, w: usize, h: usize) -> Vec<u8> {
        let mut px = vec![0u8; w * h * 4];
        mark.paint(&mut px, w);
        px
    }

    #[test]
    fn an_icon_marks_the_pixels_the_icon_draws() {
        let side = 24;
        let mut drawn = vec![0u8; (side * side * 4) as usize];
        Icon::ChevronRight.draw(&mut drawn, side, Rgba(200, 120, 40, 255));
        let marked = painted(
            &Mark::icon(Icon::ChevronRight, side, 0, 0, Rgba(200, 120, 40, 255)),
            side as usize,
            side as usize,
        );
        assert_eq!(
            marked, drawn,
            "on a clear layer a mark is the icon's own pixels"
        );
        assert!(marked.iter().any(|b| *b != 0));
    }

    #[test]
    fn a_mark_is_cut_at_the_edges_of_the_picture_and_blends_over_what_is_there() {
        let mut px = vec![0u8; 8 * 4 * 4];
        // A white hairline of 10 from x = -1: its last 7 pixels are on the 8 wide picture, on row 3 (the last).
        Mark::rule(-1, 3, 10, WHITE).paint(&mut px, 8);
        assert_eq!(&px[3 * 8 * 4..][..4], &[255, 255, 255, 255]);
        assert_eq!(
            &px[..3 * 8 * 4],
            &vec![0u8; 3 * 8 * 4][..],
            "the other rows are clear"
        );
        // Above and below the picture, nothing is drawn and nothing breaks.
        Mark::rule(0, -1, 8, WHITE).paint(&mut px, 8);
        Mark::rule(0, 4, 8, WHITE).paint(&mut px, 8);
        // A half-covered pixel of a translucent colour over an opaque one keeps what is under it.
        let mut under = vec![0, 0, 100, 255, 0, 0, 100, 255];
        Mark::rule(0, 0, 2, Rgba(100, 0, 0, 100)).paint(&mut under, 2);
        assert!(
            under[0] >= 100 && under[2] < 100 && under[3] == 255,
            "{under:?}"
        );
    }

    #[test]
    fn the_shapes_have_the_coverage_their_old_painters_gave() {
        // A disc: solid in the middle, clear in the corner, soft at the rim.
        let disc = Form::Disc(24).coverage();
        assert_eq!(disc.len(), 24 * 24);
        assert_eq!((disc[12 * 24 + 12], disc[0]), (255, 0));
        assert!(disc.iter().any(|c| *c > 0 && *c < 255));
        // A rounded square: solid inside, corners cut.
        let round = Form::Round {
            w: 56,
            h: 56,
            radius: 16,
        }
        .coverage();
        assert_eq!((round[28 * 56 + 28], round[0], round[55]), (255, 0, 0));
        assert_eq!(round[28 * 56], 255, "the middle of an edge is solid");
        // A bar: its four corners are half, the rest whole.
        let bar = Form::Bar { w: 5, h: 8 }.coverage();
        assert_eq!(
            (bar[0], bar[4], bar[5 * 7], bar[5 * 8 - 1]),
            (127, 127, 127, 127)
        );
        assert_eq!(bar[5 + 2], 255);
        // A hairline is solid.
        assert_eq!(Form::Rule(7).coverage(), vec![255u8; 7]);
    }

    #[test]
    fn the_atlas_keeps_one_strip_for_every_width_of_hairline_and_one_sprite_for_each_other_shape() {
        assert_eq!(Form::Rule(3).sprite(), Form::Rule(900).sprite());
        assert_ne!(
            Form::Disc(24).sprite(),
            Form::Disc(20).sprite(),
            "a disc of another size is another sprite"
        );
        assert_eq!(Form::Icon(Icon::Check, 24).size(), (24, 24));
    }

    #[test]
    fn a_veil_is_a_round_rectangle_of_one_level_and_a_hatch_has_exactly_two_levels_inside_its_rounding()
     {
        let veil = Form::Veil {
            w: 40,
            h: 1,
            radius: 0,
            alpha: 23,
        }
        .coverage();
        assert_eq!(
            veil,
            vec![23u8; 40],
            "a tenth of the ink along its whole width"
        );
        let veil = Form::Veil {
            w: 30,
            h: 30,
            radius: 10,
            alpha: 100,
        }
        .coverage();
        assert_eq!((veil[15 * 30 + 15], veil[0]), (100, 0));

        let hatch = Form::Hatch {
            w: 120,
            h: 20,
            radius: 10,
            dark: 38,
            light: 18,
        };
        let cover = hatch.coverage();
        // Inside the rounded ends only: the two levels of the stripes and nothing between (the ends are anti-aliased).
        let mut levels: Vec<u8> = (0..20usize)
            .flat_map(|y| (12..108usize).map(move |x| (y, x)))
            .map(|(y, x)| cover[y * 120 + x])
            .collect();
        levels.sort_unstable();
        levels.dedup();
        assert_eq!(levels, vec![18, 38], "dark and light stripes");
        assert_eq!(cover[0], 0, "the corner is cut away");
        // Stripes of one level run along the diagonal: a pixel and the one a step right and a step up from it are on one stripe.
        assert_eq!(cover[10 * 120 + 60], cover[9 * 120 + 61]);
        assert_ne!(
            hatch.sprite(),
            Form::Round {
                w: 120,
                h: 20,
                radius: 10
            }
            .sprite()
        );
    }
}
