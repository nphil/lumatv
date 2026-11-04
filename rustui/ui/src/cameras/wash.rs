//! The fill of the bars beside a picture that is not the shape of its box: a copy of the picture taken down to a few dozen pixels, blurred,
//! dimmed and tinted, drawn to cover the box behind the sharp picture (the idea of the owner's Stashy app, `LiveBlurBackdropView`).
//!
//! The blur costs next to nothing because it is made small: the source is taken to 48 pixels on its longest side by the mean of the area under
//! each pixel, blurred there, and only the part the bars show is made again at one pixel in 4 or 8 of the screen's, so what the CPU writes is a few
//! kilobytes and what the compositor draws is one or two quads it scales up (linearly). No GPU blur pass, no second decode.
//!
//! Pure: pictures in, pictures out. The layers that show them are `surface::pic` (a tile's, a still's) and `surface::full` (the bars either side of
//! the video window, where the picture itself cannot be read: it is on a hardware plane).

use crate::kit::picture::Picture;
use ui::geometry::Frame;

/// The shape of the screen and of a tile.
pub const STANDARD: f32 = 16.0 / 9.0;
/// A picture whose shape is within this share of its box's is the box's shape: a tile covers it, and a cut of a few percent is not worth a fill.
pub const SAME_SHAPE: f32 = 0.04;
/// A bar narrower than this (px of the 1920 by 1080 screen) is left as it is: it is the rounding of the fit.
pub const MIN_BAR: f32 = 16.0;
/// The longest side of the tiny picture, in px.
pub const TINY: u32 = 48;
/// The screen the camera's pictures are placed on: the 1920 by 1080 of the layout, whatever the panel's pixels.
pub const SCREEN: Frame = Frame::new(0.0, 0.0, 1920.0, 1080.0);

/// How the tiny picture is graded so that it sits under the sharp one instead of competing with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grade {
    /// What the picture's colours are multiplied by.
    pub gain: f32,
    /// The tint (r, g, b, 0 to 255) and how much of it is mixed in (0 to 1).
    pub tint: [f32; 3],
    pub mix: f32,
    /// What the colour is multiplied about the grey: a blur of a picture is greyer than the picture, so more than 1 gives the bars back the colour.
    pub sat: f32,
}

/// Whether a picture of `aspect` (width over height) is the shape of a box of `box_aspect`, near enough to be covered.
pub fn same_shape(aspect: f32, box_aspect: f32) -> bool {
    box_aspect > 0.0 && (aspect / box_aspect - 1.0).abs() <= SAME_SHAPE
}

/// The parts of `screen` that a picture shown at `shown` leaves bare and that are worth filling: the sides (the screen's whole height), then what is
/// above and below the picture. Bars narrower than [`MIN_BAR`] are left out.
pub fn bars(screen: Frame, shown: Frame) -> Vec<Frame> {
    let mut out = Vec::new();
    let mut add = |f: Frame, thickness: f32| {
        if thickness >= MIN_BAR && f.w > 0.0 && f.h > 0.0 {
            out.push(f);
        }
    };
    add(Frame::new(screen.x, screen.y, shown.x - screen.x, screen.h), shown.x - screen.x);
    add(Frame::new(shown.right(), screen.y, screen.right() - shown.right(), screen.h), screen.right() - shown.right());
    add(Frame::new(shown.x, screen.y, shown.w, shown.y - screen.y), shown.y - screen.y);
    add(Frame::new(shown.x, shown.bottom(), shown.w, screen.bottom() - shown.bottom()), screen.bottom() - shown.bottom());
    out
}

/// Whether a video window at (`x`, `y`, `w`, `h`) on the [`SCREEN`] leaves bars worth filling.
pub fn has_bars(rect: (i32, i32, i32, i32)) -> bool {
    !bars(SCREEN, Frame::new(rect.0 as f32, rect.1 as f32, rect.2 as f32, rect.3 as f32)).is_empty()
}

/// `pic` taken to `w` by `h` by the mean of the pixels under each new one, reading one in `step` of them each way: the wash is blurred after, so a
/// sparse mean says the same as a full one, at 1/`step`² of the reads (the full mean of a 480 by 640 still took 7 ms; this takes about one).
fn shrink(pic: &Picture, w: usize, h: usize) -> Vec<[f32; 3]> {
    let (pw, ph) = (pic.width as usize, pic.height as usize);
    let step = ((pw / w).max(ph / h) / 3).clamp(1, 8);
    let mut px = Vec::with_capacity(w * h);
    for oy in 0..h {
        let (y0, y1) = (oy * ph / h, ((oy + 1) * ph / h).max(oy * ph / h + 1));
        for ox in 0..w {
            let (x0, x1) = (ox * pw / w, ((ox + 1) * pw / w).max(ox * pw / w + 1));
            let (mut sum, mut n) = ([0u32; 3], 0u32);
            for y in (y0..y1).step_by(step) {
                for x in (x0..x1).step_by(step) {
                    let i = (y * pw + x) * 4;
                    sum[0] += u32::from(pic.rgba[i]);
                    sum[1] += u32::from(pic.rgba[i + 1]);
                    sum[2] += u32::from(pic.rgba[i + 2]);
                    n += 1;
                }
            }
            let n = n.max(1) as f32;
            px.push([sum[0] as f32 / n, sum[1] as f32 / n, sum[2] as f32 / n]);
        }
    }
    px
}

/// The tiny blurred copy of a picture, graded: opaque, a few dozen pixels on a side, keeping the picture's shape.
#[derive(Clone, Debug, PartialEq)]
pub struct Tiny {
    w: usize,
    h: usize,
    px: Vec<[f32; 3]>,
}

impl Tiny {
    /// `pic` taken down to [`TINY`] on its longest side (a picture already that small is only blurred), blurred twice with a 3 by 3 binomial
    /// kernel and graded. `None` for a picture with no pixels.
    pub fn of(pic: &Picture, grade: Grade) -> Option<Tiny> {
        let (pw, ph) = (pic.width as usize, pic.height as usize);
        if pw == 0 || ph == 0 || pic.rgba.len() < pw * ph * 4 {
            return None;
        }
        let scale = (TINY as f32 / pw.max(ph) as f32).min(1.0);
        let (w, h) = (((pw as f32 * scale).round() as usize).max(1), ((ph as f32 * scale).round() as usize).max(1));
        let mut px: Vec<[f32; 3]> = if (w, h) == (pw, ph) {
            pic.rgba.chunks_exact(4).take(w * h).map(|p| [f32::from(p[0]), f32::from(p[1]), f32::from(p[2])]).collect()
        } else {
            shrink(pic, w, h)
        };
        for _ in 0..2 {
            px = blur(&px, w, h);
        }
        let keep = grade.gain * (1.0 - grade.mix);
        for p in &mut px {
            let grey = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
            for (c, t) in p.iter_mut().zip(grade.tint) {
                *c = (grey + (*c - grey) * grade.sat).clamp(0.0, 255.0) * keep + t * grade.mix;
            }
        }
        Some(Tiny { w, h, px })
    }

    pub fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    /// The part of the wash that lies in `region`, as a picture of one pixel in `d` of the screen's: the wash covers `cover` (the box the sharp picture
    /// is shown in, centred), and `region` is the part of the screen the layer shows it in. The layer is `d` times larger than the picture, so
    /// the picture overhangs `region` by less than `d` on the right and below: the layer is clipped to `region`.
    pub fn region(&self, cover: Frame, region: Frame, d: f32) -> Picture {
        let d = d.max(1.0);
        let (rw, rh) = (((region.w / d).ceil() as u32).max(1), ((region.h / d).ceil() as u32).max(1));
        let s = (cover.w / self.w as f32).max(cover.h / self.h as f32).max(0.001);
        let (cx, cy) = cover.centre();
        // where each column and row of the picture lands in the tiny one: the two pixels it lies between and the eased fraction of the way to the
        // second (edge pixels held out to the edges), worked out once for the row or the column, not for each pixel
        let axis = |origin: f32, centre: f32, n: u32, size: usize| -> Vec<(usize, usize, f32)> {
            let last = size as f32 - 1.0;
            (0..n)
                .map(|i| {
                    let u = (origin + (i as f32 + 0.5) * d - centre) / s + size as f32 / 2.0 - 0.5;
                    let floor = u.floor();
                    (floor.clamp(0.0, last) as usize, (floor + 1.0).clamp(0.0, last) as usize, ease(u - floor))
                })
                .collect()
        };
        let (xs, ys) = (axis(region.x, cx, rw, self.w), axis(region.y, cy, rh, self.h));
        let mut rgba = Vec::with_capacity((rw * rh * 4) as usize);
        for &(y0, y1, ty) in &ys {
            let (top, foot) = (&self.px[y0 * self.w..(y0 + 1) * self.w], &self.px[y1 * self.w..(y1 + 1) * self.w]);
            for &(x0, x1, tx) in &xs {
                for k in 0..3 {
                    let (above, below) = (top[x0][k] + (top[x1][k] - top[x0][k]) * tx, foot[x0][k] + (foot[x1][k] - foot[x0][k]) * tx);
                    rgba.push((above + (below - above) * ty).round().clamp(0.0, 255.0) as u8);
                }
                rgba.push(255);
            }
        }
        Picture { width: rw, height: rh, rgba }
    }
}

fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// One pass of the 3 by 3 binomial blur (1 2 1 over 4, in both directions), the edge pixel held out to the edge.
fn blur(src: &[[f32; 3]], w: usize, h: usize) -> Vec<[f32; 3]> {
    let mut across = vec![[0.0; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let (l, m, r) = (src[y * w + x.saturating_sub(1)], src[y * w + x], src[y * w + (x + 1).min(w - 1)]);
            for k in 0..3 {
                across[y * w + x][k] = (l[k] + 2.0 * m[k] + r[k]) * 0.25;
            }
        }
    }
    let mut out = vec![[0.0; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let (u, m, d) = (across[y.saturating_sub(1) * w + x], across[y * w + x], across[(y + 1).min(h - 1) * w + x]);
            for k in 0..3 {
                out[y * w + x][k] = (u[k] + 2.0 * m[k] + d[k]) * 0.25;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Grade = Grade { gain: 1.0, tint: [0.0; 3], mix: 0.0, sat: 1.0 };

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Picture {
        Picture { width: w, height: h, rgba: (0..w * h).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect() }
    }

    /// A picture whose left half is `left` and right half is `right`.
    fn halves(w: u32, h: u32, left: [u8; 3], right: [u8; 3]) -> Picture {
        let mut rgba = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                let c = if x < w / 2 { left } else { right };
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        Picture { width: w, height: h, rgba }
    }

    #[test]
    fn a_picture_the_shape_of_its_box_within_four_percent_has_nothing_to_fill() {
        assert!(same_shape(16.0 / 9.0, STANDARD));
        assert!(same_shape(2688.0 / 1520.0, STANDARD), "the plant room's 2688 by 1520 is 16:9 to within one percent");
        assert!(!same_shape(4.0 / 3.0, STANDARD));
        assert!(!same_shape(3.0 / 4.0, STANDARD));
        assert!(!same_shape(21.0 / 9.0, STANDARD));
    }

    #[test]
    fn the_bars_are_the_sides_of_a_tall_picture_the_top_and_foot_of_a_wide_one_and_none_for_the_screens_shape() {
        // a portrait doorbell, 1920 by 2560, fitted: 810 by 1080 in the middle
        let portrait = Frame::new(555.0, 0.0, 810.0, 1080.0);
        assert_eq!(bars(SCREEN, portrait), vec![Frame::new(0.0, 0.0, 555.0, 1080.0), Frame::new(1365.0, 0.0, 555.0, 1080.0)]);
        // a 32:9 strip: 1920 by 540 in the middle of the height
        let wide = Frame::new(0.0, 270.0, 1920.0, 540.0);
        assert_eq!(bars(SCREEN, wide), vec![Frame::new(0.0, 0.0, 1920.0, 270.0), Frame::new(0.0, 810.0, 1920.0, 270.0)]);
        assert!(bars(SCREEN, SCREEN).is_empty());
        // 2688 by 1520 fitted is 1910 wide: five pixels either side is the fit's rounding, not a bar
        assert!(bars(SCREEN, Frame::new(5.0, 0.0, 1910.0, 1080.0)).is_empty());
    }

    #[test]
    fn a_wash_of_one_colour_is_that_colour_graded_and_opaque() {
        let grade = Grade { gain: 0.5, tint: [20.0, 10.0, 30.0], mix: 0.25, sat: 1.0 };
        let tiny = Tiny::of(&solid(480, 270, [200, 100, 50]), grade).unwrap();
        assert_eq!(tiny.size(), (48, 27));
        let p = tiny.region(SCREEN, Frame::new(0.0, 0.0, 400.0, 400.0), 8.0);
        assert_eq!((p.width, p.height), (50, 50));
        // 200 * 0.5 * 0.75 + 20 * 0.25 = 80, 100 * 0.375 + 2.5 = 40, 50 * 0.375 + 7.5 = 26
        assert_eq!(p.at(25, 25), [80, 40, 26, 255]);
        assert!(p.rgba.chunks_exact(4).all(|px| px[3] == 255));
    }

    #[test]
    fn a_tall_pictures_wash_covers_the_screen_and_the_sides_show_the_sides_of_it() {
        // red on the left half, blue on the right: the left bar is red, the right bar blue, and neither is mixed with the other far from the middle
        let tiny = Tiny::of(&halves(360, 480, [255, 0, 0], [0, 0, 255]), PLAIN).unwrap();
        assert_eq!(tiny.size(), (36, 48), "the shape is kept");
        let left = tiny.region(SCREEN, Frame::new(0.0, 0.0, 555.0, 1080.0), 8.0);
        let right = tiny.region(SCREEN, Frame::new(1365.0, 0.0, 555.0, 1080.0), 8.0);
        assert_eq!((left.width, left.height), (70, 135));
        let l = left.at(10, 60);
        let r = right.at(60, 60);
        assert!(l[0] > 200 && l[2] < 50, "{l:?}");
        assert!(r[2] > 200 && r[0] < 50, "{r:?}");
    }

    #[test]
    fn the_wash_is_smooth_where_the_tiny_picture_has_an_edge() {
        // the colours meet in the middle of the screen: from the left edge to the right the red falls without a step of more than 40 levels (a hard edge is 255)
        let tiny = Tiny::of(&halves(480, 270, [255, 0, 0], [0, 0, 0]), PLAIN).unwrap();
        let strip = tiny.region(SCREEN, Frame::new(0.0, 500.0, 1920.0, 8.0), 8.0);
        let reds: Vec<i32> = (0..strip.width).map(|x| i32::from(strip.at(x, 0)[0])).collect();
        let steepest = reds.windows(2).map(|w| (w[0] - w[1]).abs()).max().unwrap();
        assert!(steepest <= 40, "steepest step {steepest} in {reds:?}");
        assert!(reds[0] > 240 && reds[reds.len() - 1] < 15);
    }

    #[test]
    fn a_pictures_mean_is_kept_by_the_blur_and_a_picture_with_no_pixels_makes_none() {
        let mean = |p: &Picture| p.rgba.chunks_exact(4).map(|px| u32::from(px[0])).sum::<u32>() as f32 / (p.width * p.height) as f32;
        let tiny = Tiny::of(&halves(480, 270, [240, 0, 0], [0, 0, 0]), PLAIN).unwrap();
        let all = tiny.region(Frame::new(0.0, 0.0, 480.0, 270.0), Frame::new(0.0, 0.0, 480.0, 270.0), 2.0);
        assert!((mean(&all) - 120.0).abs() < 6.0, "{}", mean(&all));
        assert!(Tiny::of(&Picture { width: 0, height: 0, rgba: Vec::new() }, PLAIN).is_none());
    }
}
