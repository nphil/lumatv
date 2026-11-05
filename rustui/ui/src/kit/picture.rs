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

/// Where an app's icon is, as the QML Home's `Image` takes it (`Dock.iconUrl`, `Dock.themeIcon`): an absolute path, a `file://` URL, or a
/// name in `<data>/theme/` (what the owner's `tiles.icons` may hold). A URL of another scheme is not a file.
pub fn icon_file(src: &str, data: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    if let Some(path) = src.strip_prefix("file://") {
        return Some(path.into());
    }
    if src.starts_with('/') {
        return Some(src.into());
    }
    let scheme = src.split_once(':').is_some_and(|(s, _)| s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)));
    if src.is_empty() || scheme {
        return None;
    }
    data.map(|d| d.join("theme").join(src))
}

/// The picture of an app's icon (see [`icon_file`]); `None` for a file that is not there or not one this shell reads.
pub fn load_icon(src: &str, data: Option<&std::path::Path>) -> Option<Picture> {
    decode(&std::fs::read(icon_file(src, data)?).ok()?).ok()
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

/// Any PNG a launch point can have: palette pictures (most app icons: Plex, Prime, LG's own), 1 to 4 bit greys and 16-bit channels are
/// expanded to eight-bit grey, RGB or RGBA, with `tRNS` as the alpha, before the pixels are read.
fn decode_png(bytes: &[u8]) -> Result<Picture, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("png: {e}"))?;
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

/// How much of the edge [`Picture::fitted_sharp`] puts back after the resample (the amount of an unsharp mask over a 3 by 3 blur). Over the
/// store logos on the dock (192 and 130 px sources at 120) the edge's 10 to 90 % rise goes from 1.8, 2.2, 1.8 and 3.0 px (the area mean)
/// to 1.5, 1.7, 1.4 and 1.9 (the resample alone: 1.6, 1.8, 1.6 and 2.1); LG's Home draws the same logos at 1.4 to 2.1.
const ICON_SHARPEN: f32 = 0.45;

/// The Lanczos-3 kernel: a windowed sinc, zero from 3 out.
fn lanczos(x: f32) -> f32 {
    let x = x.abs();
    if x < 1e-6 {
        1.0
    } else if x >= 3.0 {
        0.0
    } else {
        let px = std::f32::consts::PI * x;
        3.0 * px.sin() * (px / 3.0).sin() / (px * px)
    }
}

/// For each of `dst` output samples of a line resized from `src` samples: the first source index and the weights from there (summing
/// to 1; the kernel widens by the shrink so that a reduction is filtered, not skipped). Indices may fall outside the line: the caller clamps.
fn lanczos_taps(src: usize, dst: usize) -> Vec<(i64, Vec<f32>)> {
    let ratio = src as f32 / dst as f32;
    let stretch = ratio.max(1.0);
    (0..dst)
        .map(|i| {
            let centre = (i as f32 + 0.5) * ratio - 0.5;
            let first = (centre - 3.0 * stretch).ceil() as i64;
            let last = (centre + 3.0 * stretch).floor() as i64;
            let mut weights: Vec<f32> = (first..=last).map(|k| lanczos((k as f32 - centre) / stretch)).collect();
            let sum: f32 = weights.iter().sum();
            weights.iter_mut().for_each(|w| *w /= sum);
            (first, weights)
        })
        .collect()
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
        if scale == 1.0 {
            // the picture is drawn at its own size (a still asked of the helper at the size of its cell, a cover of the same size): every pixel
            // is a source pixel, so the rows are copied, centred and cut as the box asks, not sampled
            let (ox, oy) = (ox as i64, oy as i64);
            let (x0, x1) = (ox.max(0), (ox + dw as i64).min(i64::from(w)));
            for y in 0..i64::from(h) {
                let sy = y - oy;
                if sy < 0 || sy >= dh as i64 || x1 <= x0 {
                    continue;
                }
                let (to, from, n) = (((y * i64::from(w) + x0) * 4) as usize, ((sy * i64::from(self.width) + x0 - ox) * 4) as usize, ((x1 - x0) * 4) as usize);
                out[to..to + n].copy_from_slice(&self.rgba[from..from + n]);
            }
            return Picture { width: w, height: h, rgba: out };
        }
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

    /// This picture in a box of `w` by `h` with its whole shape kept, centred, the rest of the box transparent (`Fit::Contain`), resampled
    /// with a Lanczos-3 kernel in both directions: edges stay a pixel wide, where [`Picture::fitted`]'s area mean and bilinear growth
    /// round them off. For app icons and logos, which are drawn at one size and read up close. A picture already of the size is copied.
    pub fn fitted_sharp(&self, w: u32, h: u32) -> Picture {
        let (w, h) = (w.max(1), h.max(1));
        let scale = (w as f32 / self.width as f32).min(h as f32 / self.height as f32);
        let (dw, dh) = (((self.width as f32 * scale).round() as u32).clamp(1, w), ((self.height as f32 * scale).round() as u32).clamp(1, h));
        let mut inner = self.resized(dw, dh);
        if (dw, dh) != (self.width, self.height) {
            inner.unsharp(ICON_SHARPEN);
        }
        if (dw, dh) == (w, h) {
            return inner;
        }
        let (ox, oy) = ((w - dw) / 2, (h - dh) / 2);
        let mut out = vec![0u8; (w * h * 4) as usize];
        for y in 0..dh {
            let from = (y * dw * 4) as usize;
            let to = (((y + oy) * w + ox) * 4) as usize;
            out[to..to + (dw * 4) as usize].copy_from_slice(&inner.rgba[from..from + (dw * 4) as usize]);
        }
        Picture { width: w, height: h, rgba: out }
    }

    /// Adds `amount` of what a 3 by 3 binomial blur takes off, to the colour only (the alpha, and so the shape, is kept), held to the
    /// alpha. The display chip's x2 scaler takes an edge a pixel wide to two or three on the panel; this puts back a part of the edge that
    /// the resample's own band limit took off, so the logo reads as sharp as the source drawn at this size by LG's Home.
    fn unsharp(&mut self, amount: f32) {
        let (w, h) = (self.width as usize, self.height as usize);
        let at = |x: i64, y: i64, c: usize, px: &[u8]| f32::from(px[(y.clamp(0, h as i64 - 1) as usize * w + x.clamp(0, w as i64 - 1) as usize) * 4 + c]);
        let src = self.rgba.clone();
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let o = (y as usize * w + x as usize) * 4;
                let a = src[o + 3];
                if a == 0 {
                    continue;
                }
                for c in 0..3 {
                    let mut blur = 0.0;
                    for (dy, wy) in [(-1, 1.0), (0, 2.0), (1, 1.0)] {
                        for (dx, wx) in [(-1, 1.0), (0, 2.0), (1, 1.0)] {
                            blur += at(x + dx, y + dy, c, &src) * wx * wy;
                        }
                    }
                    let v = f32::from(src[o + c]);
                    self.rgba[o + c] = (v + amount * (v - blur / 16.0)).round().clamp(0.0, f32::from(a)) as u8;
                }
            }
        }
    }

    /// The whole picture at `w` by `h` (stretched to it), by Lanczos-3 over the premultiplied channels; the colour is held to the alpha
    /// so the kernel's overshoot cannot leave a pixel brighter than it is opaque.
    fn resized(&self, w: u32, h: u32) -> Picture {
        if (w, h) == (self.width, self.height) {
            return self.clone();
        }
        let (sw, sh, w, h) = (self.width as usize, self.height as usize, w as usize, h as usize);
        let across = lanczos_taps(sw, w);
        let down = lanczos_taps(sh, h);
        // horizontal pass: every source row to `w` pixels
        let mut rows = vec![0f32; sh * w * 4];
        for y in 0..sh {
            let src = &self.rgba[y * sw * 4..(y + 1) * sw * 4];
            for (x, (first, weights)) in across.iter().enumerate() {
                let mut acc = [0f32; 4];
                for (k, wt) in weights.iter().enumerate() {
                    let i = (first + k as i64).clamp(0, sw as i64 - 1) as usize * 4;
                    for c in 0..4 {
                        acc[c] += f32::from(src[i + c]) * wt;
                    }
                }
                rows[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&acc);
            }
        }
        let mut out = vec![0u8; w * h * 4];
        for (y, (first, weights)) in down.iter().enumerate() {
            for x in 0..w {
                let mut acc = [0f32; 4];
                for (k, wt) in weights.iter().enumerate() {
                    let r = (first + k as i64).clamp(0, sh as i64 - 1) as usize;
                    let i = (r * w + x) * 4;
                    for c in 0..4 {
                        acc[c] += rows[i + c] * wt;
                    }
                }
                let a = acc[3].round().clamp(0.0, 255.0);
                let o = (y * w + x) * 4;
                for c in 0..3 {
                    out[o + c] = acc[c].round().clamp(0.0, a) as u8;
                }
                out[o + 3] = a as u8;
            }
        }
        Picture { width: w as u32, height: h as u32, rgba: out }
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
        let art = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../compositor/assets/ui/art/great-wave.jpg")).expect("the shipped print");
        let p = decode(&art).expect("a jpeg");
        assert_eq!((p.width, p.height), (1280, 860));
        assert!(p.rgba.chunks_exact(4).all(|px| px[3] == 255));
        let icon = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../compositor/assets/ui/icons/lucide/bell.png")).expect("an icon");
        let q = decode(&icon).expect("a png");
        assert_eq!((q.width, q.height), (64, 64));
        assert!(q.rgba.chunks_exact(4).any(|px| px[3] == 255) && q.rgba.chunks_exact(4).any(|px| px[3] == 0));
        assert!(decode(b"hello").is_err());
        assert!(decode(&[]).is_err());
    }

    fn png(color: png::ColorType, depth: png::BitDepth, (w, h): (u32, u32), data: &[u8], setup: impl FnOnce(&mut png::Encoder<&mut Vec<u8>>)) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(depth);
        setup(&mut enc);
        let mut writer = enc.write_header().expect("header");
        writer.write_image_data(data).expect("pixels");
        writer.finish().expect("finished");
        out
    }

    #[test]
    fn a_palette_png_with_a_transparent_entry_a_sixteen_bit_png_and_a_four_bit_grey_png_are_read_like_the_eight_bit_ones() {
        // the shapes app icons come in (Plex, Prime and LG's own are palette pictures): two entries, the second clear
        let indexed = png(png::ColorType::Indexed, png::BitDepth::Eight, (2, 1), &[0, 1], |e| {
            e.set_palette(vec![200, 10, 20, 1, 2, 3]);
            e.set_trns(vec![255, 0]);
        });
        let p = decode(&indexed).expect("a palette picture");
        assert_eq!((p.width, p.height), (2, 1));
        assert_eq!(p.at(0, 0), [200, 10, 20, 255]);
        assert_eq!(p.at(1, 0)[3], 0, "the transparent entry is clear");
        // a palette picture with no transparency is opaque
        let plain = png(png::ColorType::Indexed, png::BitDepth::Two, (2, 1), &[0b0001_0000], |e| e.set_palette(vec![9, 8, 7, 1, 1, 1]));
        assert_eq!(decode(&plain).expect("two bits").at(1, 0), [1, 1, 1, 255]);
        // 16 bits a channel keep their high byte
        let deep = png(png::ColorType::Rgba, png::BitDepth::Sixteen, (1, 1), &[0x80, 0x00, 0x40, 0x00, 0xff, 0x00, 0xff, 0xff], |_| {});
        assert_eq!(decode(&deep).expect("sixteen bits").at(0, 0), [0x80, 0x40, 0xff, 255]);
        // a grey picture of four bits: 0 and 15 are black and white
        let grey = png(png::ColorType::Grayscale, png::BitDepth::Four, (2, 1), &[0x0f], |_| {});
        let g = decode(&grey).expect("four bits");
        assert_eq!((g.at(0, 0), g.at(1, 0)), ([0, 0, 0, 255], [255, 255, 255, 255]));
    }

    #[test]
    fn an_icon_is_a_path_a_file_url_or_a_name_in_the_theme_folder_and_never_another_url() {
        let data = std::path::Path::new("/var/lib/lumatv");
        let at = |s: &str| icon_file(s, Some(data)).map(|p| p.display().to_string());
        assert_eq!(at("/media/developer/apps/x/icon.png").as_deref(), Some("/media/developer/apps/x/icon.png"));
        assert_eq!(at("file:///usr/palm/applications/y/icon.png").as_deref(), Some("/usr/palm/applications/y/icon.png"));
        assert_eq!(at("plex.png").as_deref(), Some("/var/lib/lumatv/theme/plex.png"));
        assert_eq!(at("http://x/y.png"), None);
        assert_eq!(at(""), None);
        assert_eq!(icon_file("plex.png", None), None, "no data folder, no theme folder");
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
    fn a_fit_at_the_pictures_own_size_moves_its_pixels_as_the_sampler_would_without_sampling() {
        // every pixel different, so a blended one would show
        let p = Picture { width: 6, height: 4, rgba: (0..24).flat_map(|i| [i as u8 * 10, i as u8, 7, 255]).collect() };
        assert_eq!(p.fitted(6, 4, Fit::Cover), p, "a box of its own size is the picture");
        // a box as tall and two narrower: a column cut from each side
        let cut = p.fitted(4, 4, Fit::Cover);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(cut.at(x, y), p.at(x + 1, y));
                assert_eq!(cut.at(x, y), p.sample((x + 1) as f32, y as f32, (x + 2) as f32, (y + 1) as f32), "the sampler's own answer at this scale");
            }
        }
        // a box as wide and four taller: two clear rows above and below
        let boxed = p.fitted(6, 8, Fit::Contain);
        for x in 0..6 {
            for y in [0, 1, 6, 7] {
                assert_eq!(boxed.at(x, y), [0, 0, 0, 0]);
            }
            for y in 0..4 {
                assert_eq!(boxed.at(x, y + 2), p.at(x, y));
            }
        }
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

    #[test]
    fn a_sharp_fit_keeps_an_edge_a_pixel_wide_where_the_area_mean_spreads_it() {
        // a 192 px picture, left half white, the edge on a whole source pixel, shown at 120: the mean smears the edge over 2 pixels
        let (w, h) = (192u32, 8u32);
        let mut rgba = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                let v = if x < 96 { 255 } else { 0 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let p = Picture { width: w, height: h, rgba };
        let sharp = p.fitted_sharp(120, 5);
        let row: Vec<u8> = (0..120).map(|x| sharp.at(x, 2)[0]).collect();
        let between = row.iter().filter(|&&v| v > 20 && v < 235).count();
        assert!(between <= 2, "the edge is at most two pixels wide: {row:?}");
        assert!(row[10] > 245 && row[110] < 10, "flat parts stay flat");
        assert!(sharp.rgba.chunks_exact(4).all(|px| px[0] <= px[3]), "still premultiplied");
    }

    #[test]
    fn a_sharp_fit_of_the_size_is_a_copy_a_contained_one_centres_and_a_clear_picture_stays_clear() {
        let p = Picture { width: 4, height: 2, rgba: (0..8).flat_map(|i| [i as u8 * 30, 0, 0, 255]).collect() };
        assert_eq!(p.fitted_sharp(4, 2), p);
        let boxed = p.fitted_sharp(8, 8);
        assert_eq!((boxed.width, boxed.height), (8, 8));
        assert_eq!(boxed.at(0, 0)[3], 0, "the bars are clear");
        assert_eq!(boxed.at(4, 4)[3], 255, "the picture is in the middle");
        let clear = Picture { width: 3, height: 3, rgba: vec![0; 36] };
        assert!(clear.fitted_sharp(7, 7).rgba.iter().all(|&b| b == 0));
    }
}
