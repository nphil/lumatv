//! The Lucide icons of the module (`<module>/icons/lucide/*.png`, 64 by 64, white on transparent: only the alpha is the icon), scaled
//! to a side and tinted into premultiplied pixels for an image layer. One loader for every screen: a name that is not in the folder
//! draws nothing (`None`), never a stand-in, and a size and colour is drawn once and kept.

use super::Kit;
use compositor_protocol::{Props, Rgba};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use ui::scene::ImageLayer;

/// What a folder of masks is asked for at most before the cache starts over: a screen draws a handful of sizes and colours.
const KEPT: usize = 192;

struct Mask {
    w: u32,
    h: u32,
    alpha: Vec<u8>,
}

#[derive(Default)]
pub struct Icons {
    dir: Option<PathBuf>,
    /// `None`: looked for and not there.
    masks: HashMap<String, Option<Mask>>,
    kept: HashMap<(String, u32, [u8; 4]), Vec<u8>>,
}

fn read_mask(path: &Path) -> Option<Mask> {
    decode_mask(std::fs::read(path).ok()?.as_slice())
}

fn decode_mask(png_bytes: &[u8]) -> Option<Mask> {
    let mut reader = png::Decoder::new(png_bytes).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let bytes = &buf[..info.buffer_size()];
    let alpha: Vec<u8> = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => bytes.chunks_exact(4).map(|p| p[3]).collect(),
        (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => bytes.chunks_exact(2).map(|p| p[1]).collect(),
        _ => return None,
    };
    Some(Mask { w: info.width, h: info.height, alpha })
}

impl Mask {
    /// The alpha at (`x`, `y`) of the mask scaled to `side`, `side` a square: the mean of the source area it covers when the icon
    /// shrinks, the bilinear sample at its centre when it grows.
    fn sample(&self, x: u32, y: u32, side: u32) -> f32 {
        let (sw, sh) = (self.w as f32, self.h as f32);
        let (fx, fy) = (sw / side as f32, sh / side as f32);
        let at = |ix: i64, iy: i64| -> f32 {
            let cx = ix.clamp(0, i64::from(self.w) - 1) as usize;
            let cy = iy.clamp(0, i64::from(self.h) - 1) as usize;
            f32::from(self.alpha[cy * self.w as usize + cx])
        };
        if fx > 1.0 || fy > 1.0 {
            let (x0, x1) = (x as f32 * fx, (x as f32 + 1.0) * fx);
            let (y0, y1) = (y as f32 * fy, (y as f32 + 1.0) * fy);
            let mut sum = 0.0;
            let mut weight = 0.0;
            let mut sy = y0.floor();
            while sy < y1 {
                let wy = (sy + 1.0).min(y1) - sy.max(y0);
                let mut sx = x0.floor();
                while sx < x1 {
                    let wx = (sx + 1.0).min(x1) - sx.max(x0);
                    sum += at(sx as i64, sy as i64) * wx * wy;
                    weight += wx * wy;
                    sx += 1.0;
                }
                sy += 1.0;
            }
            sum / weight.max(f32::EPSILON)
        } else {
            let (cx, cy) = ((x as f32 + 0.5) * fx - 0.5, (y as f32 + 0.5) * fy - 0.5);
            let (ix, iy) = (cx.floor(), cy.floor());
            let (tx, ty) = (cx - ix, cy - iy);
            let (ix, iy) = (ix as i64, iy as i64);
            let top = at(ix, iy) * (1.0 - tx) + at(ix + 1, iy) * tx;
            let bottom = at(ix, iy + 1) * (1.0 - tx) + at(ix + 1, iy + 1) * tx;
            top * (1.0 - ty) + bottom * ty
        }
    }
}

impl Icons {
    /// The icons of `module_dir` (`<module>/icons/lucide`, else the program's copy); masks are read when first asked for.
    pub fn new(module_dir: &Path) -> Self {
        Self { dir: Some(module_dir.join("icons/lucide")), ..Self::default() }
    }

    /// Whether `name` is an icon of the folder.
    pub fn has(&mut self, name: &str) -> bool {
        self.mask(name).is_some()
    }

    fn mask(&mut self, name: &str) -> Option<&Mask> {
        if !self.masks.contains_key(name) {
            // a name is a file's stem: nothing that could leave the folder
            let ok = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            // The module's file first; the program's own copy when the directory is gone, so the glyphs never depend on it.
            let mask = ok
                .then(|| self.dir.as_ref().and_then(|d| read_mask(&d.join(format!("{name}.png")))))
                .flatten()
                .or_else(|| ok.then(|| crate::embedded::icon(&format!("lucide/{name}.png")).and_then(decode_mask)).flatten());
            self.masks.insert(name.to_string(), mask);
        }
        self.masks.get(name).and_then(Option::as_ref)
    }

    /// The icon `name` at `side` by `side` in `ink`, premultiplied RGBA; `None` for a name the folder does not have.
    pub fn draw(&mut self, name: &str, side: u32, ink: Rgba) -> Option<Vec<u8>> {
        let side = side.max(1);
        let key = (name.to_string(), side, [ink.0, ink.1, ink.2, ink.3]);
        if let Some(px) = self.kept.get(&key) {
            return Some(px.clone());
        }
        let mask = self.mask(name)?;
        let mut out = vec![0u8; (side * side * 4) as usize];
        for y in 0..side {
            for x in 0..side {
                let a = mask.sample(x, y, side).round().clamp(0.0, 255.0) as u32;
                let i = ((y * side + x) * 4) as usize;
                out[i] = (u32::from(ink.0) * a / 255) as u8;
                out[i + 1] = (u32::from(ink.1) * a / 255) as u8;
                out[i + 2] = (u32::from(ink.2) * a / 255) as u8;
                out[i + 3] = (u32::from(ink.3) * a / 255) as u8;
            }
        }
        if self.kept.len() >= KEPT {
            self.kept.clear();
        }
        self.kept.insert(key, out.clone());
        Some(out)
    }
}

/// An image layer holding the icon `name` at `side`, in `ink`, at depth `z` and `at`; `Ok(None)` for a name that is not an icon.
pub fn layer(kit: &mut Kit, name: &str, side: u32, ink: Rgba, z: i32, at: Props) -> std::io::Result<Option<ImageLayer>> {
    let Some(pixels) = kit.icons.draw(name, side, ink) else { return Ok(None) };
    let mut layer = kit.ui.scene.image(side, side, z, at)?;
    layer.pixels_mut().copy_from_slice(&pixels);
    layer.damage(None);
    Ok(Some(layer))
}

/// Paints the icon again into a layer made for the same side (a new ink, a new name).
pub fn repaint(kit: &mut Kit, layer: &mut ImageLayer, name: &str, ink: Rgba) -> bool {
    let side = layer.width();
    if layer.height() != side {
        return false;
    }
    let Some(pixels) = kit.icons.draw(name, side, ink) else {
        layer.clear();
        layer.damage(None);
        return false;
    };
    layer.pixels_mut().copy_from_slice(&pixels);
    layer.damage(None);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../compositor/assets/ui"))
    }

    #[test]
    fn the_glyphs_draw_with_no_module_directory() {
        let mut icons = Icons::new(Path::new("/nonexistent"));
        assert!(icons.has("bell-ring"));
        assert!(icons.draw("bell-ring", 32, Rgba(255, 255, 255, 255)).is_some());
        assert!(!icons.has("no-such-icon"));
    }

    #[test]
    fn an_icon_is_scaled_and_tinted_premultiplied_and_kept() {
        let mut icons = Icons::new(&module());
        assert!(icons.has("bell-ring"));
        assert!(!icons.has("no-such-icon"));
        assert!(!icons.has("../theme/theme"), "a name cannot leave the folder");
        let ink = Rgba(200, 100, 50, 255);
        let small = icons.draw("bell-ring", 32, ink).expect("drawn");
        assert_eq!(small.len(), 32 * 32 * 4);
        assert!(small.chunks_exact(4).any(|p| p[3] == 255), "the stroke is solid somewhere");
        assert!(small.chunks_exact(4).any(|p| p[3] == 0), "and the box is not filled");
        for p in small.chunks_exact(4) {
            assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3], "premultiplied: no channel above the alpha");
        }
        let solid = small.chunks_exact(4).find(|p| p[3] == 255).expect("a solid pixel");
        assert_eq!((solid[0], solid[1], solid[2]), (200, 100, 50), "the ink where the icon is solid");
        let big = icons.draw("bell-ring", 96, ink).expect("grown");
        assert_eq!(big.len(), 96 * 96 * 4);
        assert!(big.chunks_exact(4).any(|p| p[3] == 255));
        assert_eq!(icons.draw("bell-ring", 32, ink), Some(small), "the same size and ink are the same pixels");
    }

    #[test]
    fn the_icons_the_system_layer_draws_are_all_in_the_folder() {
        let mut icons = Icons::new(&module());
        for name in [
            "bell-ring",
            "alarm-clock",
            "timer",
            "circle-alert",
            "trophy",
            "triangle-alert",
            "bell",
            "cable",
            "video",
            "volume-2",
            "volume-x",
            "info",
            "user",
            "car",
            "paw-print",
            "package",
            "activity",
            "camera",
            "monitor",
            "plug",
            "chevron-down",
            "chevron-right",
        ] {
            assert!(icons.has(name), "{name}");
        }
    }
}
