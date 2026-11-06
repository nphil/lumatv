//! The colours and the pixels Settings paints with, from the look in force (`kit/Lu.qml`'s washes, `LuTile`, `LuSegmented`'s plates):
//! one place for what a row, a tab or a card is coloured, so the surface reads tokens and never holds a colour of its own.

use crate::kit::Kit;
use crate::kit::button::faded;
use compositor_protocol::Rgba;
use ui::lucent::palette::{Mode, PaletteRole};

/// The colours of the look in force. All are premultiplied, as the compositor takes them.
#[derive(Clone, Copy, Debug)]
pub struct Paint {
    pub light: bool,
    pub ink: Rgba,
    pub ink2: Rgba,
    pub ink3: Rgba,
    pub accent: Rgba,
    pub accent_ink: Rgba,
    pub danger: Rgba,
    /// The danger colour as words: pushed away from the material until it reads (`Lu.dangerInk`).
    pub danger_ink: Rgba,
    /// The verdicts' colours of a meter: good and so-so (bad is `danger`).
    pub positive: Rgba,
    pub warning: Rgba,
    pub edge: Rgba,
    pub edge_raised: Rgba,
    pub track_off: Rgba,
    pub sheet: Rgba,
    /// The look's ground: the darker or the lighter of a code's two colours (`code.rs`).
    pub canvas: Rgba,
    pub scrim: Rgba,
    pub wash_focus: Rgba,
    pub wash_hover: Rgba,
    pub wash_selected: Rgba,
    pub disabled: f32,
}

fn scaled(c: Rgba, k: f32) -> Rgba {
    let s = |v: u8| (f32::from(v) * k).round().clamp(0.0, 255.0) as u8;
    Rgba(s(c.0), s(c.1), s(c.2), c.3)
}

impl Paint {
    pub fn of(kit: &Kit) -> Self {
        let look = &kit.ui.look;
        let light = kit.mode == Mode::Light;
        let ink = look.ink();
        let st = kit.tokens.states;
        let danger = look.danger();
        Paint {
            light,
            ink,
            ink2: look.ink2(),
            ink3: look.ink3(),
            accent: look.accent(),
            accent_ink: look.accent_ink(),
            danger,
            danger_ink: if light { scaled(danger, 1.0 / 3.2) } else { scaled(danger, 1.25) },
            positive: look.colour(PaletteRole::Positive),
            warning: look.colour(PaletteRole::Warning),
            edge: look.colour(PaletteRole::Edge),
            edge_raised: look.colour(PaletteRole::EdgeRaised),
            track_off: look.theme.materials.track_off,
            sheet: look.colour(PaletteRole::Sheet),
            canvas: look.colour(PaletteRole::Canvas),
            scrim: look.colour(PaletteRole::Scrim),
            wash_focus: faded(ink, st.focus_wash[usize::from(light)]),
            wash_hover: faded(ink, st.hover_wash),
            wash_selected: faded(ink, st.selected_wash),
            disabled: st.disabled_opacity,
        }
    }

    /// The tray under a segmented row's options.
    pub fn tray(&self) -> Rgba {
        if self.light { self.track_off } else { Rgba(0, 0, 0, 56) }
    }

    /// The plate under the focused option: a shade on the dark glass and a tint on the light, away from the label either way.
    pub fn seg_focus(&self) -> Rgba {
        if self.light { Rgba(87, 87, 87, 87) } else { Rgba(0, 0, 0, 71) }
    }

    /// The plate under the chosen option: the accent at a low alpha.
    pub fn seg_selected(&self) -> Rgba {
        faded(self.accent, if self.light { 0.2 } else { 0.26 })
    }

    /// `c` at `k` of its opacity: a dimmed row's ink.
    pub fn dim(&self, c: Rgba, enabled: bool) -> Rgba {
        if enabled { c } else { faded(c, self.disabled) }
    }
}

/// How much of the pixel (`x`, `y`) of a `w` by `h` rounded rectangle of corner radius `r` is inside it: a distance field, so the
/// edge is anti-aliased at any size.
fn coverage(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (px, py) = ((x + 0.5 - cx).abs(), (y + 0.5 - cy).abs());
    let (qx, qy) = (px - (cx - r), py - (cy - r));
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    (0.5 - (outside + inside - r)).clamp(0.0, 1.0)
}

/// Premultiplied "src over dst".
fn over(dst: &mut [u8], src: [u8; 4]) {
    let a = u32::from(src[3]);
    for i in 0..4 {
        dst[i] = (u32::from(src[i]) + u32::from(dst[i]) * (255 - a) / 255).min(255) as u8;
    }
}

/// The rounded plate of a tile: `side` square, corner `radius`, `fill` and a 1 px `border` (0 alpha for none).
fn plate_raster(side: u32, radius: f32, fill: Rgba, border: Rgba) -> Vec<u8> {
    let s = side as usize;
    let mut out = vec![0u8; s * s * 4];
    let sf = side as f32;
    for y in 0..s {
        for x in 0..s {
            let outer = coverage(x as f32, y as f32, sf, sf, radius);
            if outer <= 0.0 {
                continue;
            }
            let i = (y * s + x) * 4;
            let inner = if border.3 > 0 { coverage(x as f32 - 1.0, y as f32 - 1.0, sf - 2.0, sf - 2.0, (radius - 1.0).max(0.0)) } else { 1.0 };
            let mix = |f: u8, b: u8| (f32::from(f) * inner + f32::from(b) * (1.0 - inner)) * outer;
            let px = [mix(fill.0, border.0), mix(fill.1, border.1), mix(fill.2, border.2), mix(fill.3, border.3)];
            out[i..i + 4].copy_from_slice(&[px[0].round() as u8, px[1].round() as u8, px[2].round() as u8, px[3].round() as u8]);
        }
    }
    out
}

/// The plates made lately, by what makes one: every row of a page has the same plate under its own glyph (a tile's rasterising was 60 to 70 us of
/// the host's UI thread, a third of a row's making), so each is made once and copied. A few are kept (the accent's, the plain, the dimmed).
const PLATES_KEPT: usize = 32;

fn plate(side: u32, radius: f32, fill: Rgba, border: Rgba) -> Vec<u8> {
    use std::cell::RefCell;
    use std::collections::HashMap;
    type Key = (u32, u32, [u8; 4], [u8; 4]);
    thread_local! {
        static KEPT: RefCell<HashMap<Key, Vec<u8>>> = RefCell::new(HashMap::new());
    }
    let key: Key = (side, radius.to_bits(), [fill.0, fill.1, fill.2, fill.3], [border.0, border.1, border.2, border.3]);
    KEPT.with(|kept| {
        let mut kept = kept.borrow_mut();
        if let Some(px) = kept.get(&key) {
            return px.clone();
        }
        let px = plate_raster(side, radius, fill, border);
        if kept.len() >= PLATES_KEPT {
            kept.clear();
        }
        kept.insert(key, px.clone());
        px
    })
}

/// A square tile (`LuTile`): a rounded plate of `fill` with a 1 px `border` (0 alpha for none) and `glyph` (premultiplied pixels of
/// `glyph_side`) in its middle, as one picture.
pub fn tile_pixels(side: u32, radius: f32, fill: Rgba, border: Rgba, glyph: Option<(&[u8], u32)>) -> Vec<u8> {
    let s = side as usize;
    let mut out = plate(side, radius, fill, border);
    if let Some((g, gs)) = glyph {
        let off = (side.saturating_sub(gs) / 2) as usize;
        for gy in 0..gs as usize {
            for gx in 0..gs as usize {
                let j = (gy * gs as usize + gx) * 4;
                let i = ((gy + off) * s + gx + off) * 4;
                if i + 4 <= out.len() && g[j + 3] > 0 {
                    over(&mut out[i..i + 4], [g[j], g[j + 1], g[j + 2], g[j + 3]]);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_is_round_at_the_corners_opaque_in_the_middle_and_carries_its_glyph() {
        let fill = Rgba(40, 40, 40, 255);
        let glyph = vec![255u8; 8 * 8 * 4];
        let t = tile_pixels(48, 15.0, fill, Rgba(0, 0, 0, 0), Some((&glyph, 8)));
        assert_eq!(&t[0..4], &[0, 0, 0, 0], "the corner pixel is outside the rounded plate");
        let mid = (24 * 48 + 24) * 4;
        assert_eq!(&t[mid..mid + 4], &[255, 255, 255, 255], "the glyph is over the plate in the middle");
        let edge = (24 * 48 + 4) * 4;
        assert_eq!(&t[edge..edge + 4], &[40, 40, 40, 255], "the plate itself is opaque");
    }

    #[test]
    fn a_border_is_one_pixel_inside_the_edge() {
        let t = tile_pixels(32, 8.0, Rgba(10, 10, 10, 255), Rgba(200, 200, 200, 255), None);
        let on_border = (16 * 32) * 4;
        let inside = (16 * 32 + 5) * 4;
        assert!(t[on_border] > 150, "{}", t[on_border]);
        assert_eq!(t[inside], 10);
    }

    #[test]
    fn a_kept_plate_is_the_same_pixels_as_one_made_again() {
        let (fill, border) = (Rgba(30, 20, 50, 255), Rgba(40, 40, 40, 40));
        let made = plate_raster(40, 12.0, fill, border);
        assert_eq!(plate(40, 12.0, fill, border), made);
        assert_eq!(plate(40, 12.0, fill, border), made, "and the second time");
        assert_ne!(plate(40, 12.0, Rgba(31, 20, 50, 255), border), made, "another fill is another plate");
        let glyph = vec![200u8; 10 * 10 * 4];
        let tile = tile_pixels(40, 12.0, fill, border, Some((&glyph, 10)));
        assert_eq!(&tile[..4], &made[..4], "the plate is under the glyph and the same outside it");
        assert_ne!(tile, made);
    }
}
