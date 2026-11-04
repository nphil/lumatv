//! The pictures the card makes by hand, and the colour arithmetic they need.
//!
//! Colours that are mixed (a lamp that goes from one colour to another, the wheel of the colour sheet) are mixed in OKLab, where equal steps
//! look equal: a mix in sRGB passes through a muddy middle (red to blue goes by a dark purple). Everything here is pure: bytes in, bytes out.

/// A colour in OKLab: lightness, and the two axes of its colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
}

fn to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_linear(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

pub fn to_lab(rgb: [u8; 3]) -> Lab {
    let (r, g, b) = (to_linear(rgb[0]), to_linear(rgb[1]), to_linear(rgb[2]));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    Lab { l: 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s, a: 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s, b: 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s }
}

/// The sRGB colour of a point of OKLab, clipped into the gamut by clamping each channel.
pub fn from_lab(lab: Lab) -> [u8; 3] {
    let l = (lab.l + 0.396_337_78 * lab.a + 0.215_803_76 * lab.b).powi(3);
    let m = (lab.l - 0.105_561_346 * lab.a - 0.063_854_17 * lab.b).powi(3);
    let s = (lab.l - 0.089_484_18 * lab.a - 1.291_485_5 * lab.b).powi(3);
    [
        from_linear(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_93 * s),
        from_linear(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s),
        from_linear(-0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s),
    ]
}

/// The colour `t` of the way from `a` to `b`, mixed in OKLab.
pub fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let (x, y) = (to_lab(a), to_lab(b));
    let t = t.clamp(0.0, 1.0);
    from_lab(Lab { l: x.l + (y.l - x.l) * t, a: x.a + (y.a - x.a) * t, b: x.b + (y.b - x.b) * t })
}

/// How far apart two colours look: the distance in OKLab (about 0.02 is the least that can be told apart).
pub fn distance(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (x, y) = (to_lab(a), to_lab(b));
    ((x.l - y.l).powi(2) + (x.a - y.a).powi(2) + (x.b - y.b).powi(2)).sqrt()
}

/// WCAG's relative luminance.
pub fn luminance(rgb: [u8; 3]) -> f32 {
    0.2126 * to_linear(rgb[0]) + 0.7152 * to_linear(rgb[1]) + 0.0722 * to_linear(rgb[2])
}

/// WCAG's contrast ratio of two colours, 1 to 21.
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `rgb` as it is, or lighter (or darker, against a light ground) with the same hue and chroma until it has `min` contrast against `ground`.
pub fn legible(rgb: [u8; 3], ground: [u8; 3], min: f32) -> [u8; 3] {
    if contrast(rgb, ground) >= min {
        return rgb;
    }
    let lab = to_lab(rgb);
    let lighter = luminance(ground) < 0.4;
    let mut best = rgb;
    for step in 1..=40 {
        let l = if lighter { (lab.l + step as f32 * 0.02).min(1.0) } else { (lab.l - step as f32 * 0.02).max(0.0) };
        best = from_lab(Lab { l, ..lab });
        if contrast(best, ground) >= min {
            return best;
        }
    }
    best
}

/// The light of a white at `kelvin` as sRGB (Tanner Helland's fit, good from 1000 to 12000 K): what a white lamp is shown in.
pub fn kelvin_rgb(kelvin: f32) -> [u8; 3] {
    let t = kelvin.clamp(1000.0, 12000.0) / 100.0;
    let r = if t <= 66.0 { 255.0 } else { 329.698_73 * (t - 60.0).powf(-0.133_204_76) };
    let g = if t <= 66.0 { 99.470_8 * t.ln() - 161.119_57 } else { 288.122_17 * (t - 60.0).powf(-0.075_514_85) };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_73 * (t - 10.0).ln() - 305.044_8
    };
    [r, g, b].map(|c: f32| c.clamp(0.0, 255.0).round() as u8)
}

/// A rounded rectangle in the pixels of a picture: what a glow is kept inside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keep {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
    /// How far inside its edge what is kept dies out, so that the edge is never seen where the glass under it has grown past it (a lifted item);
    /// 0 is a hard edge, one pixel of anti-aliasing.
    pub soft: f32,
}

impl Keep {
    /// How much of the pixel centred at (`px`, `py`) is kept, 0 to 1: nothing at the edge, all of it `soft` pixels inside.
    pub fn cover(&self, px: f32, py: f32) -> f32 {
        let (cx, cy) = (self.x + self.w / 2.0, self.y + self.h / 2.0);
        let (hx, hy) = (self.w / 2.0 - self.radius, self.h / 2.0 - self.radius);
        let (dx, dy) = ((px - cx).abs() - hx, (py - cy).abs() - hy);
        let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0) - self.radius;
        if self.soft <= 1.0 {
            return (0.5 - outside).clamp(0.0, 1.0);
        }
        let t = (-outside / self.soft).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

/// A glow of `rgb` in premultiplied RGBA `px` (`w` by `h`): a disc centred at (`cx`, `cy`) that is `strength` opaque at its middle and nothing
/// at `r`, kept inside `keep`. The fall-off is smooth, so the edge of the disc is never seen.
#[allow(clippy::too_many_arguments)]
pub fn glow(px: &mut [u8], w: u32, h: u32, cx: f32, cy: f32, r: f32, rgb: [u8; 3], strength: f32, keep: Keep) {
    let (r2, s) = (r * r, strength.clamp(0.0, 1.0));
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let d2 = (fx - cx).powi(2) + (fy - cy).powi(2);
            if d2 >= r2 {
                continue;
            }
            let t = 1.0 - (d2 / r2).sqrt();
            let a = s * t * t * (3.0 - 2.0 * t) * keep.cover(fx, fy);
            if a <= 0.0 {
                continue;
            }
            let i = ((y * w + x) * 4) as usize;
            let src = [(f32::from(rgb[0]) * a), (f32::from(rgb[1]) * a), (f32::from(rgb[2]) * a), 255.0 * a];
            let inv = 1.0 - a;
            for (k, v) in src.iter().enumerate() {
                px[i + k] = (*v + f32::from(px[i + k]) * inv).round().min(255.0) as u8;
            }
        }
    }
}

/// `src` (premultiplied RGBA, `sw` by `sh`) laid over `dst` (`dw` wide) at (`x`, `y`), clipped to `dst`'s rows.
#[allow(clippy::too_many_arguments)]
pub fn lay(dst: &mut [u8], dw: u32, dh: u32, src: &[u8], sw: u32, sh: u32, x: i32, y: i32) {
    for row in 0..sh as i32 {
        let ty = y + row;
        if ty < 0 || ty >= dh as i32 {
            continue;
        }
        for col in 0..sw as i32 {
            let tx = x + col;
            if tx < 0 || tx >= dw as i32 {
                continue;
            }
            let s = ((row as u32 * sw + col as u32) * 4) as usize;
            let a = src[s + 3];
            if a == 0 {
                continue;
            }
            let d = ((ty as u32 * dw + tx as u32) * 4) as usize;
            let inv = 255 - u32::from(a);
            for k in 0..4 {
                dst[d + k] = (u32::from(src[s + k]) + (u32::from(dst[d + k]) * inv + 127) / 255).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_round_trips_and_a_mix_passes_through_a_bright_middle_not_a_muddy_one() {
        for c in [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255], [0, 0, 0], [12, 200, 140], [255, 177, 110]] {
            let back = from_lab(to_lab(c));
            assert!(c.iter().zip(back).all(|(a, b)| (i32::from(*a) - i32::from(b)).abs() <= 1), "{c:?} {back:?}");
        }
        // red to blue: the middle in OKLab is as light as the ends on average, the middle in sRGB is a dark purple
        let mid = mix([255, 0, 0], [0, 0, 255], 0.5);
        let naive = [127u8, 0, 127];
        assert!(to_lab(mid).l > to_lab(naive).l + 0.05, "{mid:?} {naive:?}");
        assert_eq!(mix([10, 20, 30], [200, 100, 50], 0.0), [10, 20, 30]);
        assert_eq!(mix([10, 20, 30], [200, 100, 50], 1.0), [200, 100, 50]);
        assert!(distance([255, 0, 0], [0, 0, 255]) > 0.4 && distance([10, 10, 10], [10, 10, 10]) == 0.0);
    }

    #[test]
    fn a_colour_with_too_little_contrast_is_lightened_until_it_has_enough_and_one_with_enough_is_left() {
        let ground = [30, 28, 40];
        assert_eq!(legible([255, 177, 110], ground, 3.0), [255, 177, 110]);
        let blue = legible([0, 0, 255], ground, 3.0);
        assert!(contrast(blue, ground) >= 3.0 && blue != [0, 0, 255], "{blue:?}");
        // still blue: the blue channel leads
        assert!(blue[2] > blue[0] && blue[2] > blue[1], "{blue:?}");
        // against a light ground it is darkened
        let yellow = legible([255, 255, 0], [250, 248, 240], 3.0);
        assert!(contrast(yellow, [250, 248, 240]) >= 3.0 && luminance(yellow) < luminance([255, 255, 0]));
    }

    #[test]
    fn a_glow_is_strongest_in_the_middle_gone_at_its_radius_and_never_outside_what_it_is_kept_in() {
        let (w, h) = (64u32, 64u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        let keep = Keep { x: 0.0, y: 0.0, w: 64.0, h: 40.0, radius: 12.0, soft: 0.0 };
        glow(&mut px, w, h, 20.0, 20.0, 30.0, [255, 0, 0], 0.8, keep);
        let at = |x: u32, y: u32| px[((y * w + x) * 4 + 3) as usize];
        assert!(at(20, 20) > 180, "{}", at(20, 20));
        assert!(at(20, 20) > at(30, 20) && at(30, 20) > at(40, 20));
        assert_eq!(at(55, 20), 0, "beyond the radius");
        assert_eq!(at(20, 50), 0, "below the box it is kept in");
        assert_eq!(at(0, 0), 0, "the corner is rounded");
        // premultiplied: the colour is never more than the alpha
        assert!(px.chunks_exact(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
    }

    #[test]
    fn a_picture_is_laid_over_another_with_its_alpha_and_clipped_to_it() {
        let mut dst = vec![0u8; 4 * 4 * 4];
        let src = vec![255u8; 2 * 2 * 4];
        lay(&mut dst, 4, 4, &src, 2, 2, 3, 3);
        assert_eq!(&dst[(3 * 4 + 3) * 4..(3 * 4 + 3) * 4 + 4], &[255, 255, 255, 255]);
        assert_eq!(&dst[0..4], &[0, 0, 0, 0]);
        lay(&mut dst, 4, 4, &src, 2, 2, -1, -1);
        assert_eq!(&dst[0..4], &[255, 255, 255, 255]);
    }
}
