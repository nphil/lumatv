//! Writing a captured frame as PNG (straight alpha, so a viewer shows what the panel would).

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::Path;

use crate::output::Image;

pub fn write_png(path: &Path, image: &Image) -> io::Result<()> {
    let file = File::create(path)?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    // Un-premultiply for the file; the compositor keeps premultiplied pixels.
    let mut straight = Vec::with_capacity(image.rgba.len());
    for px in image.rgba.chunks_exact(4) {
        let a = px[3] as u32;
        if a == 0 || a == 255 {
            straight.extend_from_slice(px);
        } else {
            straight.extend_from_slice(&[
                ((px[0] as u32 * 255 + a / 2) / a).min(255) as u8,
                ((px[1] as u32 * 255 + a / 2) / a).min(255) as u8,
                ((px[2] as u32 * 255 + a / 2) / a).min(255) as u8,
                px[3],
            ]);
        }
    }
    writer.write_image_data(&straight).map_err(io::Error::other)
}

/// The X2 pattern: frame `k` is R = 25k, G = 0x80, B = 255 - 25k, A = 255.
pub fn x2_colour(k: u32) -> [u8; 4] {
    [(25 * k).min(255) as u8, 0x80, 255u8.saturating_sub((25 * k).min(255) as u8), 255]
}

/// Whether a frame's three probe pixels (top-left, centre, bottom-right of `rect`) are all `expected`.
pub fn matches(image: &Image, rect: (u32, u32, u32, u32), expected: [u8; 4]) -> bool {
    let (x, y, w, h) = rect;
    if w == 0 || h == 0 {
        return false;
    }
    [(x, y), (x + w / 2, y + h / 2), (x + w - 1, y + h - 1)].iter().all(|&(px, py)| image.pixel(px, py) == expected)
}
