//! Format marks drawn from art (`icons/audio` for the receiver's sound, `icons/video` for the badge): black on transparent, read once and
//! tinted with the look's ink when drawn, so they change with the theme.

use compositor_protocol::Rgba;
use std::path::Path;

/// The marks of the format badge, in `icons/video` (`generators/gen-audio-logos.py --set video`).
pub const VIDEO_MARKS: [&str; 22] = [
    "uhd", "fhd", "hd", "120hz", "144hz", "dolby-vision", "hdr10plus", "hdr10", "hlg", "hdr", "sdr", "atmos", "dolby-digital-plus",
    "dolby-digital", "dolby-truehd", "dolby-audio", "dtsx", "dts-hd", "dts", "pcm", "5.1", "7.1",
];

pub struct LogoArt {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    alpha: Vec<u8>,
}

/// `dir/<name>.png` (8-bit RGBA; only its alpha is kept).
pub fn load(dir: &Path, name: &'static str) -> Option<LogoArt> {
    let file = std::fs::File::open(dir.join(format!("{name}.png"))).ok()?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let alpha = buf[..info.buffer_size()].chunks_exact(4).map(|p| p[3]).collect();
    Some(LogoArt { name, width: info.width, height: info.height, alpha })
}

/// The marks of `names` that `dir` has.
pub fn load_all(dir: &Path, names: &[&'static str]) -> Vec<LogoArt> {
    names.iter().filter_map(|n| load(dir, n)).collect()
}

impl LogoArt {
    /// The width of the mark drawn `h` pixels high.
    pub fn width_at(&self, h: u32) -> u32 {
        (h as f32 * self.width as f32 / self.height.max(1) as f32).round().max(1.0) as u32
    }

    /// The mark `h` pixels high (its width by its aspect, at most `max_w`), in `ink`, box-filtered and premultiplied.
    pub fn render(&self, h: u32, max_w: f32, ink: Rgba) -> (u32, u32, Vec<u8>) {
        let aspect = self.width as f32 / self.height.max(1) as f32;
        let h = (h as f32).min(max_w / aspect).round().max(1.0) as u32;
        let w = (h as f32 * aspect).round().max(1.0) as u32;
        let mut out = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            let (sy0, sy1) = ((y * self.height / h) as usize, (((y + 1) * self.height).div_ceil(h) as usize).min(self.height as usize));
            for x in 0..w {
                let (sx0, sx1) = ((x * self.width / w) as usize, (((x + 1) * self.width).div_ceil(w) as usize).min(self.width as usize));
                let mut sum = 0u32;
                let mut n = 0u32;
                for sy in sy0..sy1.max(sy0 + 1) {
                    for sx in sx0..sx1.max(sx0 + 1) {
                        sum += u32::from(self.alpha[sy * self.width as usize + sx]);
                        n += 1;
                    }
                }
                let a = sum / n.max(1);
                let i = ((y * w + x) * 4) as usize;
                out[i] = (u32::from(ink.0) * a / 255) as u8;
                out[i + 1] = (u32::from(ink.1) * a / 255) as u8;
                out[i + 2] = (u32::from(ink.2) * a / 255) as u8;
                out[i + 3] = (u32::from(ink.3) * a / 255) as u8;
            }
        }
        (w, h, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_video_mark_ships_as_alpha_art_at_the_generators_height() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../compositor/assets/ui/icons/video");
        let marks = load_all(&dir, &VIDEO_MARKS);
        assert_eq!(marks.len(), VIDEO_MARKS.len());
        for m in &marks {
            assert_eq!(m.height, 112, "{}", m.name);
            let (w, h, px) = m.render(28, 1000.0, Rgba(200, 100, 50, 255));
            assert_eq!((w, h), (m.width_at(28), 28));
            assert!(px.chunks_exact(4).any(|p| p[3] == 255), "{} has solid ink at 28 px", m.name);
        }
    }
}
