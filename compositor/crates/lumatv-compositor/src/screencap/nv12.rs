//! RGBA to NV12 for the encoder: BT.709 matrix, limited range (16-235 luma, 16-240 chroma), chroma averaged over each 2x2 block. HD
//! players assume exactly this when a stream says nothing about its colour, and the hardware encoder's stream says nothing.
//!
//! Integer arithmetic with the chroma rows summing to exactly zero, so a grey input comes out at exactly 128 in both chroma planes.

/// Bytes an NV12 frame of this size takes: the luma plane, then the interleaved Cb/Cr plane at half the height.
pub fn nv12_len(width: usize, height: usize) -> usize {
    width * height + width * height / 2
}

/// Converts `rgba` (`width * height` pixels, rows top to bottom, R G B A bytes; alpha is ignored: the compositor's pixels are
/// premultiplied, so what is over black is already what the bytes say) into `out` as NV12 (`width` is the luma stride and the chroma
/// stride). `width` and `height` are even. Panics when a buffer is shorter than the size says.
pub fn rgba_to_nv12(rgba: &[u8], width: usize, height: usize, out: &mut [u8]) {
    assert!(width % 2 == 0 && height % 2 == 0, "NV12 needs even sizes");
    assert!(rgba.len() >= width * height * 4 && out.len() >= nv12_len(width, height));
    let (luma, chroma) = out.split_at_mut(width * height);
    let chroma = &mut chroma[..width * height / 2];
    for ((src, y_rows), uv_row) in rgba.chunks_exact(width * 8).zip(luma.chunks_exact_mut(width * 2)).zip(chroma.chunks_exact_mut(width)) {
        let (top, bottom) = src.split_at(width * 4);
        let (y_top, y_bottom) = y_rows.split_at_mut(width);
        for (((a, b), (ya, yb)), uv) in
            top.chunks_exact(8).zip(bottom.chunks_exact(8)).zip(y_top.chunks_exact_mut(2).zip(y_bottom.chunks_exact_mut(2))).zip(uv_row.chunks_exact_mut(2))
        {
            ya[0] = luma_of(a[0], a[1], a[2]);
            ya[1] = luma_of(a[4], a[5], a[6]);
            yb[0] = luma_of(b[0], b[1], b[2]);
            yb[1] = luma_of(b[4], b[5], b[6]);
            let r = a[0] as i32 + a[4] as i32 + b[0] as i32 + b[4] as i32;
            let g = a[1] as i32 + a[5] as i32 + b[1] as i32 + b[5] as i32;
            let bl = a[2] as i32 + a[6] as i32 + b[2] as i32 + b[6] as i32;
            // The coefficients are /256 and the sums are of four pixels: shift by 10, rounding.
            uv[0] = (((-26 * r - 86 * g + 112 * bl + 512) >> 10) + 128) as u8;
            uv[1] = (((112 * r - 102 * g - 10 * bl + 512) >> 10) + 128) as u8;
        }
    }
}

#[inline(always)]
fn luma_of(r: u8, g: u8, b: u8) -> u8 {
    (((47 * r as u32 + 157 * g as u32 + 16 * b as u32 + 128) >> 8) + 16) as u8
}

/// An all-black NV12 frame (the first frame of a recording that has not been given a picture yet).
pub fn black_nv12(width: usize, height: usize) -> Vec<u8> {
    let mut v = vec![16u8; nv12_len(width, height)];
    v[width * height..].fill(128);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The matrix in floating point, the way the standard states it.
    fn reference(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
        let y = 16.0 + 219.0 * (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
        let cb = 128.0 + 224.0 * (b / 255.0 - (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0) / (2.0 * (1.0 - 0.0722));
        let cr = 128.0 + 224.0 * (r / 255.0 - (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0) / (2.0 * (1.0 - 0.2126));
        (y, cb, cr)
    }

    fn one(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
        let rgba: Vec<u8> = (0..4).flat_map(|_| [r, g, b, 255]).collect();
        let mut out = vec![0u8; nv12_len(2, 2)];
        rgba_to_nv12(&rgba, 2, 2, &mut out);
        (out[0], out[4], out[5])
    }

    #[test]
    fn black_and_white_land_on_the_legal_range_and_grey_has_neutral_chroma() {
        assert_eq!(one(0, 0, 0), (16, 128, 128));
        assert_eq!(one(255, 255, 255), (235, 128, 128));
        for v in [1u8, 17, 64, 100, 127, 128, 200, 254] {
            let (_, cb, cr) = one(v, v, v);
            assert_eq!((cb, cr), (128, 128), "grey {v}");
        }
    }

    #[test]
    fn every_corner_of_the_colour_cube_is_within_two_levels_of_the_standard_matrix() {
        for r in [0u8, 255] {
            for g in [0u8, 255] {
                for b in [0u8, 255] {
                    let (y, cb, cr) = one(r, g, b);
                    let (ry, rcb, rcr) = reference(r as f64, g as f64, b as f64);
                    for (got, want, what) in [(y, ry, "Y"), (cb, rcb, "Cb"), (cr, rcr, "Cr")] {
                        assert!((got as f64 - want).abs() <= 2.0, "({r},{g},{b}) {what}: {got} against {want:.1}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_ramp_of_mixed_colours_stays_close_to_the_reference_and_inside_the_legal_range() {
        let mut worst = 0.0f64;
        for i in 0..64u32 {
            let (r, g, b) = ((i * 4) as u8, (255 - i * 4) as u8, ((i * 7) % 256) as u8);
            let (y, cb, cr) = one(r, g, b);
            let (ry, rcb, rcr) = reference(r as f64, g as f64, b as f64);
            worst = worst.max((y as f64 - ry).abs()).max((cb as f64 - rcb).abs()).max((cr as f64 - rcr).abs());
            assert!((16..=235).contains(&y) && (16..=240).contains(&cb) && (16..=240).contains(&cr), "({r},{g},{b}) -> {y} {cb} {cr}");
        }
        assert!(worst <= 2.0, "worst error {worst}");
    }

    #[test]
    fn pixels_land_where_they_belong_and_each_chroma_pair_averages_its_own_block() {
        // 4x4: the left two columns white, the right two black; the top two rows have a red pixel in the first block's corner.
        let (w, h) = (4usize, 4usize);
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let v = if x < 2 { 255 } else { 0 };
                rgba[(y * w + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        rgba[..4].copy_from_slice(&[255, 0, 0, 255]);
        let mut out = vec![0u8; nv12_len(w, h)];
        rgba_to_nv12(&rgba, w, h, &mut out);
        let y_of = |x: usize, y: usize| out[y * w + x];
        assert_eq!(y_of(0, 0), luma_of(255, 0, 0));
        assert_eq!(y_of(1, 0), 235);
        assert_eq!(y_of(2, 0), 16);
        assert_eq!(y_of(3, 3), 16);
        let uv = &out[w * h..];
        // Block (0,0): three white pixels and one red: reddish, so Cr above neutral and Cb below it.
        assert!(uv[1] > 128 && uv[0] < 128, "block 0: cb {} cr {}", uv[0], uv[1]);
        // Block (1,0) and the lower row's blocks: neutral.
        assert_eq!((uv[2], uv[3]), (128, 128));
        assert_eq!((uv[w], uv[w + 1]), (128, 128));
        assert_eq!((uv[w + 2], uv[w + 3]), (128, 128));
    }

    #[test]
    fn black_is_what_a_black_picture_converts_to() {
        let rgba = vec![0u8; 8 * 6 * 4];
        let mut out = vec![9u8; nv12_len(8, 6)];
        rgba_to_nv12(&rgba, 8, 6, &mut out);
        assert_eq!(out, black_nv12(8, 6));
    }

    #[test]
    fn a_720p_frame_converts_in_a_few_milliseconds_even_in_a_debug_build_budget() {
        let (w, h) = (1280usize, 720usize);
        let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i * 31 % 251) as u8).collect();
        let mut out = vec![0u8; nv12_len(w, h)];
        let t = std::time::Instant::now();
        rgba_to_nv12(&rgba, w, h, &mut out);
        // Not a gate (a debug build on a busy host): only that it is not pathological.
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
    }
}
