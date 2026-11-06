//! The QR code of a code row (`Kind::Code`): the modules of what the row holds (an address a phone opens), drawn here as whole pixels in
//! the look's own colours. The plate is the lighter of the look's ink and canvas and the modules the darker, so the code is dark on light
//! in either look (what a phone's camera reads best) and changes with the theme; the plate's corners are the theme's control radius.

use super::layout::Sizes;
use super::paint::{Paint, tile_pixels};
use compositor_protocol::Rgba;
use qrcodegen::{QrCode, QrCodeEcc};

/// The light margin the standard asks for around the modules, in modules.
const QUIET: u32 = 4;

/// The modules of `text` (error correction M: a code of about 30 characters is 29 modules across), `None` for text too long for a code.
pub fn modules(text: &str) -> Option<QrCode> {
    (!text.is_empty()).then(|| QrCode::encode_text(text, QrCodeEcc::Medium).ok()).flatten()
}

/// The side of one module and of the whole picture (the quiet margin in): about five targets across, in whole pixels per module.
pub fn sides(sz: &Sizes, qr: &QrCode) -> (u32, u32) {
    let across = qr.size().max(1) as u32 + 2 * QUIET;
    let module = ((sz.target * 5.0) as u32 / across).max(2);
    (module, module * across)
}

/// The plate and the modules, both opaque.
pub fn inks(paint: &Paint) -> (Rgba, Rgba) {
    let (a, b) = (opaque(paint.ink), opaque(paint.canvas));
    if luma(a) >= luma(b) { (a, b) } else { (b, a) }
}

fn opaque(c: Rgba) -> Rgba {
    if c.3 == 0 {
        return Rgba(0, 0, 0, 255);
    }
    let un = |v: u8| (u32::from(v) * 255 / u32::from(c.3)).min(255) as u8;
    Rgba(un(c.0), un(c.1), un(c.2), 255)
}

fn luma(c: Rgba) -> f32 {
    0.2126 * f32::from(c.0) + 0.7152 * f32::from(c.1) + 0.0722 * f32::from(c.2)
}

/// The picture: a rounded plate of `light` with the dark modules on it, `module` pixels each, premultiplied RGBA rows from the top.
pub fn raster(qr: &QrCode, module: u32, light: Rgba, dark: Rgba, radius: f32) -> Vec<u8> {
    let n = qr.size().max(0) as u32;
    let side = module * (n + 2 * QUIET);
    let mut px = tile_pixels(side, radius, light, Rgba(0, 0, 0, 0), None);
    let row = side as usize * 4;
    for my in 0..n {
        for mx in 0..n {
            if !qr.get_module(mx as i32, my as i32) {
                continue;
            }
            let (x0, y0) = (((QUIET + mx) * module) as usize, ((QUIET + my) * module) as usize);
            for y in y0..y0 + module as usize {
                for p in px[y * row + x0 * 4..y * row + (x0 + module as usize) * 4].chunks_exact_mut(4) {
                    p.copy_from_slice(&[dark.0, dark.1, dark.2, dark.3]);
                }
            }
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_modules_are_whole_pixels_of_the_darker_ink_inside_a_light_quiet_margin() {
        let qr = modules("https://plex.tv/link/?pin=WBVQ").unwrap();
        assert_eq!(qr.size(), 29);
        let (light, dark) = (Rgba(240, 238, 250, 255), Rgba(12, 10, 20, 255));
        let px = raster(&qr, 3, light, dark, 0.0);
        let side = 3 * (29 + 8) as usize;
        assert_eq!(px.len(), side * side * 4);
        let at = |x: usize, y: usize| &px[(y * side + x) * 4..(y * side + x) * 4 + 4];
        // the quiet margin is the plate; the finder's corner module at (4, 4) is dark to its last pixel
        assert_eq!(at(11, 11), [240, 238, 250, 255]);
        assert_eq!((at(12, 12), at(14, 14)), ([12, 10, 20, 255].as_slice(), [12, 10, 20, 255].as_slice()));
        assert_eq!(modules(&"x".repeat(4000)).map(|q| q.size()), None, "too long for a code");
    }

    #[test]
    fn the_plate_is_the_lighter_ink_in_either_look() {
        let (_harness, kit) = crate::kit::testkit::kit(1.0);
        let mut paint = Paint::of(&kit);
        paint.ink = Rgba(245, 245, 250, 255);
        paint.canvas = Rgba(10, 10, 14, 255);
        assert_eq!(inks(&paint), (Rgba(245, 245, 250, 255), Rgba(10, 10, 14, 255)));
        (paint.ink, paint.canvas) = (Rgba(20, 20, 30, 255), Rgba(250, 250, 252, 255));
        assert_eq!(inks(&paint), (Rgba(250, 250, 252, 255), Rgba(20, 20, 30, 255)));
        // a translucent ink is taken at full strength
        paint.ink = Rgba(10, 10, 15, 128);
        assert_eq!(inks(&paint).1, Rgba(19, 19, 29, 255));
    }
}
