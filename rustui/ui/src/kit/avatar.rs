//! A profile's picture (`docs/research/profiles-design.md`, Avatars and Screens): a rounded square, never a circle (the corner about a
//! fifth of the side, Lucent's tile radius), holding the picture (one of the bundled avatars or the person's own photo, a PNG or a JPEG the
//! helper keeps as a file) or, with none, the profile's colour and its letter. The current person carries a tick on a corner. Drawn into a
//! [`Px`] that a screen puts in an image layer; the pictures are decoded and scaled once per file and size and kept, so a strip of avatars
//! that is built again costs no decode.

use super::Kit;
use super::picture::{Fit, Picture, decode};
use super::widgets::Px;
use compositor_protocol::Rgba;
use std::cell::RefCell;
use std::collections::HashMap;
use ui::components::Align;
use ui::geometry::Frame;
use ui::lucent::palette::PaletteRole;
use ui::text::{Canvas, Type, Weight};

/// Who an avatar is of: the letter and the colour of the profile, and its picture file if it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Who {
    pub initial: String,
    /// `#RRGGBB`.
    pub colour: String,
    pub file: Option<String>,
}

/// The corner of an avatar of `side`: a fifth of it.
pub fn radius(side: u32) -> f32 {
    (side as f32 * 0.22).round()
}

const KEPT: usize = 64;

thread_local! {
    static PICTURES: RefCell<HashMap<(String, u32), Option<Picture>>> = RefCell::new(HashMap::new());
}

/// The picture of `file` as a square of `side` with round corners: decoded and scaled once; `None` for a file that is not there or not a
/// picture (the colour and the letter stand in).
pub fn picture(file: &str, side: u32) -> Option<Picture> {
    PICTURES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(hit) = cache.get(&(file.to_string(), side)) {
            return hit.clone();
        }
        let made = std::fs::read(file).ok().and_then(|bytes| decode(&bytes).ok()).map(|p| {
            let mut fitted = p.fitted(side, side, Fit::Cover);
            fitted.round_corners(radius(side));
            fitted
        });
        if cache.len() >= KEPT {
            cache.clear();
        }
        cache.insert((file.to_string(), side), made.clone());
        made
    })
}

/// The picture of `file` shown whole inside `w` by `h` (an app's own icon, not cropped): decoded and scaled once; `None` for a file that is
/// not there or not a picture.
pub fn contained(file: &str, w: u32, h: u32) -> Option<Picture> {
    PICTURES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = (format!("{file}\u{0}{w}x{h}"), 0);
        if let Some(hit) = cache.get(&key) {
            return hit.clone();
        }
        let made = super::picture::load_icon(file, None).map(|p| p.fitted(w, h, Fit::Contain));
        if cache.len() >= KEPT {
            cache.clear();
        }
        cache.insert(key, made.clone());
        made
    })
}

/// A colour of the theme's `#RRGGBB` as an opaque colour; the fallback when it is not one.
pub fn parse(colour: &str, fallback: Rgba) -> Rgba {
    match lumatv_tokens::colour(colour) {
        Some(c) => Rgba(c.r, c.g, c.b, 255),
        None => fallback,
    }
}

fn luminance(c: Rgba) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2)
}

/// The ink that reads best on `ground`: black or white.
pub fn ink_on(ground: Rgba) -> Rgba {
    let l = luminance(ground);
    if (l + 0.05) / 0.05 >= (1.05) / (l + 0.05) { Rgba(0, 0, 0, 255) } else { Rgba(255, 255, 255, 255) }
}

/// The avatar of `who` at `side`, with the tick of the current person when `tick`.
pub fn px(kit: &mut Kit, who: &Who, side: u32, tick: bool) -> Px {
    let mut out = Px::new(side, side);
    match who.file.as_deref().and_then(|f| picture(f, side)) {
        Some(p) => out.lay(&p.rgba, p.width, p.height, 0, 0),
        None => {
            let ground = parse(&who.colour, kit.ui.look.accent());
            out.round_rect(Frame::new(0.0, 0.0, side as f32, side as f32), radius(side), ground, None);
            let letter: String = who.initial.chars().take(1).collect::<String>().to_uppercase();
            if !letter.is_empty() {
                let ty = Type { px: ((side as f32) * 0.5).round() as u32, weight: Weight::SemiBold };
                let extent = kit.ui.fonts.measure(&letter, ty);
                let x = ((side as f32 - extent.width) / 2.0).round();
                let baseline = ((side as f32 - extent.height()) / 2.0 + extent.ascent).round();
                let _ = Align::Centre;
                kit.ui.fonts.draw(&mut Canvas { pixels: &mut out.data, width: side as usize }, &letter, ty, (x, baseline), ink_on(ground));
            }
        }
    }
    if tick {
        let r = (side as f32 * 0.22).round().max(9.0);
        let (cx, cy) = (side as f32 - r - 1.0, side as f32 - r - 1.0);
        let look = &kit.ui.look;
        let (sheet, accent, accent_ink) = (look.colour(PaletteRole::Sheet), look.accent(), look.accent_ink());
        out.circle(cx, cy, r + 2.0, Rgba(sheet.0, sheet.1, sheet.2, 255), None);
        out.circle(cx, cy, r, accent, None);
        let g = (r * 1.3).round() as u32;
        if let Some(check) = kit.icons.draw("check", g, accent_ink) {
            out.lay(&check, g, g, (cx - g as f32 / 2.0).round() as i32, (cy - g as f32 / 2.0).round() as i32);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;

    fn write_png(path: &std::path::Path, w: u32, h: u32, rgb: [u8; 3]) {
        let file = std::fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        let data: Vec<u8> = (0..w * h).flat_map(|_| rgb).collect();
        writer.write_image_data(&data).unwrap();
    }

    #[test]
    fn with_no_picture_it_is_the_colour_and_the_letter_in_the_ink_that_reads_on_it_with_round_corners() {
        let (_h, mut kit) = testkit::kit(1.0);
        let who = Who { initial: "n".into(), colour: "#A79BF5".into(), file: None };
        let a = px(&mut kit, &who, 56, false);
        assert_eq!(a.at(0, 0)[3], 0, "a rounded square, not a circle: the corner is cut, the middle of the edge is not");
        assert!(a.at(28, 1)[3] > 200 && a.at(1, 28)[3] > 200);
        assert_eq!(&a.at(4, 28)[..3], &[0xA7, 0x9B, 0xF5], "the profile's colour");
        let inked = (14..42).flat_map(|y| (14..42).map(move |x| (x, y))).filter(|(x, y)| a.at(*x, *y)[..3] != [0xA7, 0x9B, 0xF5]).count();
        assert!(inked > 30, "the letter is drawn: {inked} pixels");
        assert_eq!(ink_on(Rgba(0xA7, 0x9B, 0xF5, 255)), Rgba(0, 0, 0, 255), "a light colour takes black");
        assert_eq!(ink_on(Rgba(0x28, 0x69, 0x83, 255)), Rgba(255, 255, 255, 255), "a dark one white");
    }

    #[test]
    fn a_picture_file_is_scaled_to_the_square_with_round_corners_and_kept() {
        let dir = std::env::temp_dir().join(format!("avatar-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.png");
        write_png(&file, 96, 96, [200, 30, 40]);
        let (_h, mut kit) = testkit::kit(1.0);
        let who = Who { initial: "I".into(), colour: "#9CCFD8".into(), file: Some(file.display().to_string()) };
        let a = px(&mut kit, &who, 64, false);
        assert_eq!(a.at(0, 0)[3], 0);
        assert_eq!(&a.at(32, 32)[..3], &[200, 30, 40], "the picture, not the colour");
        std::fs::remove_file(&file).unwrap();
        let b = px(&mut kit, &who, 64, false);
        assert_eq!(a, b, "kept: the file is not read again");
        let gone = Who { initial: "I".into(), colour: "#9CCFD8".into(), file: Some("/no/such/file.png".into()) };
        assert_eq!(&px(&mut kit, &gone, 64, false).at(4, 32)[..3], &[0x9C, 0xCF, 0xD8], "a file that is not there is the colour and the letter");
    }

    #[test]
    fn the_current_person_carries_a_tick_on_a_corner() {
        let (_h, mut kit) = testkit::kit(1.0);
        let who = Who { initial: "N".into(), colour: "#A79BF5".into(), file: None };
        let plain = px(&mut kit, &who, 56, false);
        let ticked = px(&mut kit, &who, 56, true);
        assert_ne!(plain, ticked);
        assert_eq!(plain.at(5, 5), ticked.at(5, 5), "the far corner is as it was");
        assert_ne!(plain.at(43, 30), ticked.at(43, 30), "the ring round the tick is in the sheet's colour");
    }
}
