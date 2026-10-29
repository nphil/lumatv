//! Pictures: a JPEG, a BMP or a PNG decoded into premultiplied RGBA (what an image layer holds), scaled to the box it is shown in and
//! given round corners. The helper's stills and thumbnails are JPEGs on the host's stand-in and 24 or 32 bit BMPs on the set (the
//! hardware decoder's answer, `helper/app/src/images.rs`); the saver's prints are JPEG files; a user's avatar is a PNG. The decode is
//! pure: nothing here touches a layer but [`Picture::paint`].

use ui::scene::ImageLayer;

/// A decoded picture: `rgba` is premultiplied, row by row from the top, `width * height * 4` bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// How a picture fills a box of another shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// The box is filled and what overhangs is cut, centred (a camera's still in its tile).
    Cover,
    /// The whole picture shows, centred; the rest of the box is transparent (a print in its matte).
    Contain,
}

/// The longest side a picture may have: a still is a few megapixels at most, anything larger is refused rather than allocated.
const MAX_SIDE: u32 = 8192;

/// Decodes a JPEG, a BMP or a PNG by its first bytes.
pub fn decode(bytes: &[u8]) -> Result<Picture, String> {
    if bytes.starts_with(&[0xff, 0xd8]) {
        decode_jpeg(bytes)
    } else if bytes.starts_with(b"BM") {
        decode_bmp(bytes)
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        decode_png(bytes)
    } else {
        Err("not a picture this shell reads (JPEG, BMP, PNG)".into())
    }
}

fn check(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("a picture of {width} by {height}"));
    }
    Ok(())
}

fn decode_jpeg(bytes: &[u8]) -> Result<Picture, String> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    let pixels = decoder.decode().map_err(|e| format!("jpeg: {e}"))?;
    let info = decoder.info().ok_or("jpeg: no header")?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    check(w, h)?;
    let n = (w * h) as usize;
    let mut rgba = Vec::with_capacity(n * 4);
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            for p in pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        jpeg_decoder::PixelFormat::L8 => {
            for &l in &pixels {
                rgba.extend_from_slice(&[l, l, l, 255]);
            }
        }
        jpeg_decoder::PixelFormat::L16 => {
            for p in pixels.chunks_exact(2) {
                rgba.extend_from_slice(&[p[0], p[0], p[0], 255]);
            }
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            // Adobe's CMYK is stored inverted
            for p in pixels.chunks_exact(4) {
                let k = u32::from(p[3]);
                let c = |v: u8| (u32::from(v) * k / 255) as u8;
                rgba.extend_from_slice(&[c(p[0]), c(p[1]), c(p[2]), 255]);
            }
        }
    }
    if rgba.len() != n * 4 {
        return Err("jpeg: the pixels do not fill the picture".into());
    }
    Ok(Picture { width: w, height: h, rgba })
}

fn le_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A BMP of 24 or 32 bits with no compression (or the bit-fields of the usual BGRA order): bottom-up, or top-down for a negative
/// height. A 32-bit BMP's fourth byte is the alpha only when the header says it is a 124-byte V5 header with an alpha mask; the
/// helper's is `B, G, R, 255` under a 40-byte header, so a 32-bit picture is opaque unless it says otherwise.
fn decode_bmp(b: &[u8]) -> Result<Picture, String> {
    let bad = || "bmp: a header that is cut short".to_string();
    let offset = le_u32(b, 10).ok_or_else(bad)? as usize;
    let dib = le_u32(b, 14).ok_or_else(bad)? as usize;
    if dib < 40 {
        return Err("bmp: an old header".into());
    }
    let width = le_u32(b, 18).ok_or_else(bad)? as i32;
    let height = le_u32(b, 22).ok_or_else(bad)? as i32;
    let bpp = le_u16(b, 28).ok_or_else(bad)?;
    let compression = le_u32(b, 30).ok_or_else(bad)?;
    if width <= 0 || height == 0 {
        return Err("bmp: a size that is not a picture".into());
    }
    if !(compression == 0 || compression == 3) || !(bpp == 24 || bpp == 32) {
        return Err(format!("bmp: {bpp} bits, compression {compression}"));
    }
    let (w, h, top_down) = (width as u32, height.unsigned_abs(), height < 0);
    check(w, h)?;
    let alpha_mask = if dib >= 56 { le_u32(b, 54 + 12).unwrap_or(0) } else { 0 };
    let has_alpha = bpp == 32 && alpha_mask != 0;
    let bytes_per = (bpp / 8) as usize;
    let stride = (w as usize * bytes_per).div_ceil(4) * 4;
    let end = offset.checked_add(stride.checked_mul(h as usize).ok_or_else(bad)?).ok_or_else(bad)?;
    let data = b.get(offset..end).ok_or("bmp: the pixels are cut short")?;
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for row in 0..h as usize {
        let src_row = if top_down { row } else { h as usize - 1 - row };
        let src = &data[src_row * stride..src_row * stride + w as usize * bytes_per];
        let dst = &mut rgba[row * w as usize * 4..(row + 1) * w as usize * 4];
        for (s, d) in src.chunks_exact(bytes_per).zip(dst.chunks_exact_mut(4)) {
            let a = if has_alpha { s[3] } else { 255 };
            let m = |v: u8| (u32::from(v) * u32::from(a) / 255) as u8;
            d.copy_from_slice(&[m(s[2]), m(s[1]), m(s[0]), a]);
        }
    }
    Ok(Picture { width: w, height: h, rgba })
}

fn decode_png(bytes: &[u8]) -> Result<Picture, String> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().map_err(|e| format!("png: {e}"))?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("png: {e}"))?;
    check(info.width, info.height)?;
    let data = &buf[..info.buffer_size()];
    if info.bit_depth != png::BitDepth::Eight {
        return Err("png: only eight bits a channel".into());
    }
    let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
    let premul = |r: u8, g: u8, b: u8, a: u8| {
        let m = |v: u8| (u32::from(v) * u32::from(a) / 255) as u8;
        [m(r), m(g), m(b), a]
    };
    match info.color_type {
        png::ColorType::Rgba => data.chunks_exact(4).for_each(|p| rgba.extend_from_slice(&premul(p[0], p[1], p[2], p[3]))),
        png::ColorType::Rgb => data.chunks_exact(3).for_each(|p| rgba.extend_from_slice(&[p[0], p[1], p[2], 255])),
        png::ColorType::Grayscale => data.iter().for_each(|&g| rgba.extend_from_slice(&[g, g, g, 255])),
        png::ColorType::GrayscaleAlpha => data.chunks_exact(2).for_each(|p| rgba.extend_from_slice(&premul(p[0], p[0], p[0], p[1]))),
        png::ColorType::Indexed => return Err("png: an indexed picture".into()),
    }
    Ok(Picture { width: info.width, height: info.height, rgba })
}

impl Picture {
    /// This picture in a box of `w` by `h` (both at least 1): `Cover` fills the box and crops, `Contain` fits inside it and leaves the
    /// rest transparent. Shrinking takes the mean of the source area under each pixel, growing samples bilinearly.
    pub fn fitted(&self, w: u32, h: u32, fit: Fit) -> Picture {
        let (w, h) = (w.max(1), h.max(1));
        let (sw, sh) = (self.width as f32, self.height as f32);
        let scale = match fit {
            Fit::Cover => (w as f32 / sw).max(h as f32 / sh),
            Fit::Contain => (w as f32 / sw).min(h as f32 / sh),
        };
        // the source rectangle that lands on the box (cover) or the box rectangle the picture lands on (contain)
        let (dw, dh) = ((sw * scale).round().max(1.0), (sh * scale).round().max(1.0));
        let (ox, oy) = (((w as f32 - dw) / 2.0).round(), ((h as f32 - dh) / 2.0).round());
        let mut out = vec![0u8; (w * h * 4) as usize];
        let inv = 1.0 / scale;
        for y in 0..h {
            let dy = y as f32 - oy;
            if dy < 0.0 || dy >= dh {
                continue;
            }
            for x in 0..w {
                let dx = x as f32 - ox;
                if dx < 0.0 || dx >= dw {
                    continue;
                }
                let px = self.sample(dx * inv, dy * inv, (dx + 1.0) * inv, (dy + 1.0) * inv);
                let i = ((y * w + x) * 4) as usize;
                out[i..i + 4].copy_from_slice(&px);
            }
        }
        Picture { width: w, height: h, rgba: out }
    }

    /// The mean of the source rectangle (x0, y0)-(x1, y1) when it spans more than a pixel, the bilinear sample at its centre when it does
    /// not.
    fn sample(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> [u8; 4] {
        let at = |x: i64, y: i64| -> [f32; 4] {
            let cx = x.clamp(0, i64::from(self.width) - 1) as usize;
            let cy = y.clamp(0, i64::from(self.height) - 1) as usize;
            let i = (cy * self.width as usize + cx) * 4;
            [f32::from(self.rgba[i]), f32::from(self.rgba[i + 1]), f32::from(self.rgba[i + 2]), f32::from(self.rgba[i + 3])]
        };
        let mut acc = [0f32; 4];
        if x1 - x0 > 1.0 || y1 - y0 > 1.0 {
            let mut weight = 0.0;
            let mut sy = y0.floor();
            while sy < y1 {
                let wy = (sy + 1.0).min(y1) - sy.max(y0);
                let mut sx = x0.floor();
                while sx < x1 {
                    let wx = (sx + 1.0).min(x1) - sx.max(x0);
                    let p = at(sx as i64, sy as i64);
                    for c in 0..4 {
                        acc[c] += p[c] * wx * wy;
                    }
                    weight += wx * wy;
                    sx += 1.0;
                }
                sy += 1.0;
            }
            for v in &mut acc {
                *v /= weight.max(f32::EPSILON);
            }
        } else {
            let (cx, cy) = ((x0 + x1) / 2.0 - 0.5, (y0 + y1) / 2.0 - 0.5);
            let (ix, iy) = (cx.floor(), cy.floor());
            let (tx, ty) = (cx - ix, cy - iy);
            let (ix, iy) = (ix as i64, iy as i64);
            let (a, b, c, d) = (at(ix, iy), at(ix + 1, iy), at(ix, iy + 1), at(ix + 1, iy + 1));
            for k in 0..4 {
                acc[k] = (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty;
            }
        }
        [acc[0].round() as u8, acc[1].round() as u8, acc[2].round() as u8, acc[3].round() as u8]
    }

    /// Rounds the corners to `radius` px with an antialiased edge (the tile's radius on a still): what is outside the arc goes
    /// transparent, premultiplied.
    pub fn round_corners(&mut self, radius: f32) {
        let r = radius.min(self.width as f32 / 2.0).min(self.height as f32 / 2.0);
        if r < 1.0 {
            return;
        }
        let (w, h) = (self.width as f32, self.height as f32);
        let reach = r.ceil() as u32;
        for corner in 0..4u32 {
            for dy in 0..reach {
                for dx in 0..reach {
                    let (x, y) = match corner {
                        0 => (dx, dy),
                        1 => (self.width - 1 - dx, dy),
                        2 => (dx, self.height - 1 - dy),
                        _ => (self.width - 1 - dx, self.height - 1 - dy),
                    };
                    // the pixel's centre against the arc's centre r in from the corner
                    let (cx, cy) = (if x as f32 + 0.5 < w / 2.0 { r } else { w - r }, if y as f32 + 0.5 < h / 2.0 { r } else { h - r });
                    let dist = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                    let inside = (r - dist + 0.5).clamp(0.0, 1.0);
                    if inside < 1.0 {
                        let i = ((y * self.width + x) * 4) as usize;
                        for c in 0..4 {
                            self.rgba[i + c] = (f32::from(self.rgba[i + c]) * inside).round() as u8;
                        }
                    }
                }
            }
        }
    }

    /// Copies the picture into `layer`, which must have been made at this size; `false` when it was not.
    pub fn paint(&self, layer: &mut ImageLayer) -> bool {
        if layer.width() != self.width || layer.height() != self.height {
            return false;
        }
        layer.pixels_mut().copy_from_slice(&self.rgba);
        layer.damage(None);
        true
    }

    /// The pixel at (`x`, `y`) as `[r, g, b, a]`, premultiplied.
    pub fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bmp(width: u32, height: i32, bpp: u16, pixels: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 54];
        b[0..2].copy_from_slice(b"BM");
        b[2..6].copy_from_slice(&((54 + pixels.len()) as u32).to_le_bytes());
        b[10..14].copy_from_slice(&54u32.to_le_bytes());
        b[14..18].copy_from_slice(&40u32.to_le_bytes());
        b[18..22].copy_from_slice(&width.to_le_bytes());
        b[22..26].copy_from_slice(&height.to_le_bytes());
        b[26..28].copy_from_slice(&1u16.to_le_bytes());
        b[28..30].copy_from_slice(&bpp.to_le_bytes());
        b.extend_from_slice(pixels);
        b
    }

    #[test]
    fn a_bottom_up_32_bit_bmp_is_read_top_row_first_and_opaque() {
        // two rows of two pixels, B G R 255 each; the first stored row is the picture's bottom
        let px = [0, 0, 255, 255, 0, 255, 0, 255, /* bottom: red, green */ 255, 0, 0, 255, 255, 255, 255, 255 /* top: blue, white */];
        let p = decode(&bmp(2, 2, 32, &px)).expect("decoded");
        assert_eq!((p.width, p.height), (2, 2));
        assert_eq!(p.at(0, 0), [0, 0, 255, 255], "the top left is blue");
        assert_eq!(p.at(1, 0), [255, 255, 255, 255]);
        assert_eq!(p.at(0, 1), [255, 0, 0, 255], "the bottom left is red");
        assert_eq!(p.at(1, 1), [0, 255, 0, 255]);
    }

    #[test]
    fn a_24_bit_bmp_has_rows_padded_to_four_bytes_and_a_negative_height_is_top_down() {
        // 1 pixel wide: 3 bytes and a pad byte per row
        let up = [10, 20, 30, 0, 40, 50, 60, 0];
        let p = decode(&bmp(1, 2, 24, &up)).expect("decoded");
        assert_eq!(p.at(0, 0), [60, 50, 40, 255], "the second stored row is the top");
        assert_eq!(p.at(0, 1), [30, 20, 10, 255]);
        let down = decode(&bmp(1, -2, 24, &up)).expect("decoded");
        assert_eq!(down.at(0, 0), [30, 20, 10, 255], "top-down keeps the stored order");
        assert!(decode(&bmp(2, 2, 24, &up)).is_err(), "pixels that are cut short are refused");
        assert!(decode(&bmp(2, 2, 16, &up)).is_err());
    }

    #[test]
    fn a_jpeg_and_a_png_are_decoded_from_the_shipped_files() {
        let art = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../payload/compositor/lumatv/art/great-wave.jpg")).expect("the shipped print");
        let p = decode(&art).expect("a jpeg");
        assert_eq!((p.width, p.height), (1280, 860));
        assert!(p.rgba.chunks_exact(4).all(|px| px[3] == 255));
        let icon = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../payload/compositor/lumatv/icons/lucide/bell.png")).expect("an icon");
        let q = decode(&icon).expect("a png");
        assert_eq!((q.width, q.height), (64, 64));
        assert!(q.rgba.chunks_exact(4).any(|px| px[3] == 255) && q.rgba.chunks_exact(4).any(|px| px[3] == 0));
        assert!(decode(b"hello").is_err());
        assert!(decode(&[]).is_err());
    }

    #[test]
    fn cover_fills_the_box_and_crops_contain_fits_and_leaves_the_rest_clear() {
        let mut rgba = Vec::new();
        for y in 0..4u32 {
            for _x in 0..8u32 {
                let v = if y < 2 { 255 } else { 0 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let p = Picture { width: 8, height: 4, rgba };
        let cover = p.fitted(4, 4, Fit::Cover);
        assert!(cover.rgba.chunks_exact(4).all(|px| px[3] == 255), "cover fills every pixel");
        assert_eq!(cover.at(0, 0)[0], 255);
        assert_eq!(cover.at(3, 3)[0], 0);
        let contain = p.fitted(4, 4, Fit::Contain);
        assert_eq!(contain.at(0, 0)[3], 0, "the bars are transparent");
        assert_eq!(contain.at(0, 3)[3], 0);
        assert_eq!(contain.at(2, 1)[3], 255, "the picture is in the middle");
        let down = p.fitted(2, 1, Fit::Cover);
        assert_eq!(down.at(0, 0)[0], 128, "shrinking takes the mean of the area (half white, half black)");
        let up = p.fitted(16, 8, Fit::Cover);
        assert_eq!((up.width, up.height), (16, 8));
    }

    #[test]
    fn round_corners_clears_the_corner_and_keeps_the_middle() {
        let p = Picture { width: 20, height: 20, rgba: vec![255; 20 * 20 * 4] };
        let mut r = p.clone();
        r.round_corners(8.0);
        assert_eq!(r.at(0, 0), [0, 0, 0, 0]);
        assert_eq!(r.at(19, 19), [0, 0, 0, 0]);
        assert_eq!(r.at(10, 10), [255, 255, 255, 255]);
        assert_eq!(r.at(10, 0), [255, 255, 255, 255], "the straight edge is whole");
        let edge = r.at(2, 2)[3];
        assert!(edge > 0 && edge < 255, "the arc is antialiased: {edge}");
        for px in r.rgba.chunks_exact(4) {
            assert!(px[0] <= px[3], "still premultiplied");
        }
    }
}
