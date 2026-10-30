//! A tile of the input picker that is a picture of its input: the last (or the live) picture of the input fills the tile, cut to its shape
//! and rounded as the tile is, with a scrim baked into its pixels under the words (Lucent: a card with a bleed picture draws no scrim of its
//! own: the author bakes the reading scrim into the picture, and the contrast check still applies). The picture lifts with the focus and
//! a lit ring and halo come in round it, as a camera's tile does in the Cameras grid; the words on it stay where they are.
//!
//! Words on a picture are white whatever the page's look: the picture is not the page's material, and the scrim under them is black. The
//! scrim is dense where the words are ([`TEXT_SCRIM`], 66 % at the top of the words, a little more at the foot), so the weakest the words'
//! ground can be, over the brightest picture there is, is [`WORST_GROUND`]: 4.5:1 for the secondary words, and far more for the primary.

use crate::kit::flow::Lifted;
use crate::kit::focus::Id;
use crate::kit::picture::{Fit, Picture};
use crate::kit::widgets::{Lift, Spot, edge, lit_edge};
use crate::kit::{Engine, Kit};
use compositor_protocol::{GroupPose, Props, Rgba};
use ui::components::plate;
use ui::geometry::Frame;
use ui::scene::{ImageLayer, ShapeLayer};

/// The words on a picture, the main ones and the quieter ones (premultiplied: 92 % white is over the reading line of .9 alpha).
pub const ON_PICTURE: Rgba = Rgba(255, 255, 255, 255);
pub const ON_PICTURE_2: Rgba = Rgba(235, 235, 235, 235);
/// The black under the words, at the top of the words and at the foot of the tile.
pub const TEXT_SCRIM: f32 = 0.66;
pub const FOOT_SCRIM: f32 = 0.80;
/// How much a picture that is not live is dimmed.
pub const STALE_DIM: f32 = 0.20;
/// The lightest the words' ground can be: a white live picture under the scrim at the top of the words (what a lint reads the words
/// against; a last picture is dimmed and its ground is darker still).
pub const WORST_GROUND: Rgba = Rgba(87, 87, 87, 255);
/// The chip behind the port's name and behind the sound mark: black at 62 %, so white words on it over a white picture are at 6:1.
pub const CHIP: Rgba = Rgba(0, 0, 0, 158);
/// What a lint reads words on a chip against, over a white picture.
pub const CHIP_GROUND: Rgba = Rgba(97, 97, 97, 255);

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How dark the foot of the tile is at `y` (0 at the top of the tile): nothing until `ramp` above where the words begin (`text_top`), the
/// words' own density from there, a little more by the foot.
pub fn foot_scrim(y: f32, text_top: f32, ramp: f32, h: f32) -> f32 {
    let from = text_top - ramp;
    if y <= from {
        0.0
    } else if y < text_top {
        TEXT_SCRIM * smooth((y - from) / ramp.max(1.0))
    } else {
        TEXT_SCRIM + (FOOT_SCRIM - TEXT_SCRIM) * ((y - text_top) / (h - text_top).max(1.0)).clamp(0.0, 1.0)
    }
}

/// The picture as the tile's layer holds it: cut to cover `w` by `h` from its middle, the scrim baked in under the words that begin at
/// `text_top` (a ramp of `ramp` above them), dimmed when it is not live, and the corners rounded to `radius`. The pixels are premultiplied.
pub fn dress(pic: &Picture, w: u32, h: u32, radius: f32, text_top: f32, ramp: f32, live: bool) -> Picture {
    let mut p = pic.fitted(w, h, Fit::Cover);
    let keep_all = if live { 1.0 } else { 1.0 - STALE_DIM };
    for y in 0..h {
        let k = ((1.0 - foot_scrim(y as f32 + 0.5, text_top, ramp, h as f32)) * keep_all * 256.0).round() as u32;
        let row = (y * w * 4) as usize;
        for px in p.rgba[row..row + (w * 4) as usize].chunks_exact_mut(4) {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * k + 128) >> 8) as u8;
            }
        }
    }
    p.round_corners(radius);
    p
}

/// A picture tile on the screen: the picture, its edge, and what the focus brings in (the lit ring and the halo).
pub struct ShotTile {
    pub id: Id,
    picture: ImageLayer,
    _edge: ShapeLayer,
    _halo: ShapeLayer,
    _lit_ring: ShapeLayer,
    /// What the layer holds: the picture's revision and whether it was dressed live.
    pub shown: (u64, bool),
}

impl ShotTile {
    /// Makes the tile under `parent` (its pose is the tile's frame, so it grows about its own centre), registers it with the engine and
    /// gives it its lift. `dressed` is the picture as [`dress`] made it for this size; `z` is the picture's depth, the rings are over it.
    pub fn new(
        kit: &mut Kit,
        engine: &mut Engine,
        parent: &ui::group::Group,
        spot: Spot,
        radius: f32,
        dressed: &Picture,
        shown: (u64, bool),
    ) -> std::io::Result<Self> {
        let (w, h) = (spot.local.w, spot.local.h);
        let other = |e: ui::error::UiError| std::io::Error::other(format!("{e}"));
        let scene = kit.ui.scene.clone();
        let rest = GroupPose { x: spot.local.x, y: spot.local.y, pivot_x: w / 2.0, pivot_y: h / 2.0, ..GroupPose::IDENTITY };
        let lift = scene.group(Some(parent.id()), rest).map_err(other)?;
        let lit = scene.group(Some(lift.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).map_err(other)?;
        let body = scene.group(Some(lift.id()), GroupPose::IDENTITY).map_err(other)?;
        let (accent, rim) = (kit.ui.look.accent(), lit_edge(kit));
        let (picture, hairline) = scene.adopting(&body, || -> std::io::Result<_> {
            let mut layer = scene.image(dressed.width, dressed.height, spot.z, Props::at(0.0, 0.0))?;
            dressed.paint(&mut layer);
            let hairline = plate::shape(&kit.ui, Frame::new(0.0, 0.0, w, h), spot.z + 1, plate::ring(edge(kit), radius, 1.0));
            Ok((layer, hairline))
        })?;
        let (halo, lit_ring) = scene.adopting(&lit, || {
            let halo = plate::shape(
                &kit.ui,
                Frame::new(-6.0, -6.0, w + 12.0, h + 12.0),
                spot.z + 1,
                plate::ring(crate::kit::button::faded(accent, 0.26), radius + 6.0, 6.0),
            );
            let ring = plate::shape(&kit.ui, Frame::new(0.0, 0.0, w, h), spot.z + 1, plate::ring(rim, radius, 3.0));
            (halo, ring)
        });
        engine.focus.register(spot.id, spot.screen, spot.region, spot.scope);
        // grows about its own centre and does not rise: the words on it do not move
        engine.flow.lift(spot.id, Lifted { group: lift, rest, scale: Lift::Card.scale(kit), rise: 0.0, lit: Some(lit) });
        Ok(Self { id: spot.id, picture, _edge: hairline, _halo: halo, _lit_ring: lit_ring, shown })
    }

    /// Puts another picture in the layer (the same size).
    pub fn repaint(&mut self, dressed: &Picture, shown: (u64, bool)) {
        if dressed.paint(&mut self.picture) {
            self.shown = shown;
        }
    }

    /// Gives the tile back: it leaves the registry and loses its lift (its layers go with the handles).
    pub fn destroy(self, engine: &mut Engine) {
        engine.focus.unregister(self.id);
        engine.flow.drop_lift(self.id);
    }

    pub fn layers(&self) -> usize {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: u32, h: u32, rgb: [u8; 3]) -> Picture {
        Picture { width: w, height: h, rgba: (0..w * h).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect() }
    }

    fn lum(p: &[u8; 4]) -> u32 {
        (u32::from(p[0]) * 54 + u32::from(p[1]) * 183 + u32::from(p[2]) * 19) >> 8
    }

    #[test]
    fn the_scrim_is_nothing_above_its_ramp_the_words_density_at_the_words_and_a_little_more_at_the_foot() {
        let (text_top, ramp, h) = (140.0, 24.0, 208.0);
        assert_eq!(foot_scrim(0.0, text_top, ramp, h), 0.0);
        assert_eq!(foot_scrim(116.0, text_top, ramp, h), 0.0);
        assert!((foot_scrim(140.0, text_top, ramp, h) - TEXT_SCRIM).abs() < 1e-6);
        assert!((foot_scrim(208.0, text_top, ramp, h) - FOOT_SCRIM).abs() < 1e-6);
        let mut last = 0.0;
        for y in 0..=208 {
            let a = foot_scrim(y as f32, text_top, ramp, h);
            assert!(a >= last - 1e-6, "no step back at {y}");
            last = a;
        }
    }

    #[test]
    fn a_picture_is_cut_to_the_tile_from_its_middle_darkened_under_the_words_and_rounded() {
        // 16:9 into a tile of 332 by 208: the sides are cut, the middle is what shows
        let mut pic = flat(480, 270, [10, 10, 10]);
        for y in 0..270 {
            for x in 200..280 {
                let i = ((y * 480 + x) * 4) as usize;
                pic.rgba[i..i + 4].copy_from_slice(&[250, 250, 250, 255]);
            }
        }
        let d = dress(&pic, 332, 208, 12.0, 140.0, 24.0, true);
        assert_eq!((d.width, d.height), (332, 208));
        assert!(lum(&d.at(166, 20)) > 200, "the bright band is in the middle: {:?}", d.at(166, 20));
        assert!(lum(&d.at(20, 20)) < 20, "and the dark is at the sides");
        // untouched above the ramp, darker and darker below
        assert!(lum(&d.at(166, 100)) > 200);
        assert!(lum(&d.at(166, 190)) < lum(&d.at(166, 120)));
        // under the words' first line the scrim is the words' density: 66 % black over the bright band
        let at_words = lum(&d.at(166, 141));
        assert!((80..=90).contains(&at_words), "{at_words}");
        // the corner is transparent, the middle of the top edge is not
        assert_eq!(d.at(0, 0)[3], 0);
        assert_eq!(d.at(166, 0)[3], 255);
        // premultiplied: no colour above its alpha
        assert!(d.rgba.chunks_exact(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
    }

    #[test]
    fn a_picture_that_is_not_live_is_dimmer_by_the_stale_dim() {
        let pic = flat(64, 36, [200, 200, 200]);
        let live = dress(&pic, 64, 36, 0.0, 30.0, 6.0, true);
        let old = dress(&pic, 64, 36, 0.0, 30.0, 6.0, false);
        let (a, b) = (live.at(32, 5)[0] as f32, old.at(32, 5)[0] as f32);
        assert!((b / a - (1.0 - STALE_DIM)).abs() < 0.01, "{a} {b}");
    }

    /// The WCAG ratio of a light and a dark (relative luminance of 8-bit greys).
    fn ratio(light: f32, dark: f32) -> f32 {
        let l = |c: f32| {
            let c = c / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        (l(light) + 0.05) / (l(dark) + 0.05)
    }

    #[test]
    fn the_words_ground_over_the_brightest_picture_reads_4_5_to_1_for_the_quiet_words_and_better_for_the_main() {
        // a white picture under the scrim at the top of the words
        let ground = 255.0 * (1.0 - TEXT_SCRIM);
        assert!((ground - f32::from(WORST_GROUND.0)).abs() <= 1.0, "the lint's ground is the picture's: {ground} {}", WORST_GROUND.0);
        // the quiet words are white at 92 %, premultiplied: over the ground they are 235 + ground * (1 - 235/255)
        let quiet = f32::from(ON_PICTURE_2.0) + f32::from(WORST_GROUND.0) * (1.0 - f32::from(ON_PICTURE_2.3) / 255.0);
        assert!(ratio(quiet, f32::from(WORST_GROUND.0)) >= 4.5, "{}", ratio(quiet, f32::from(WORST_GROUND.0)));
        assert!(ratio(255.0, f32::from(WORST_GROUND.0)) >= 6.0);
        // a last picture is dimmed, so its ground is darker still
        assert!(ratio(quiet, f32::from(WORST_GROUND.0) * (1.0 - STALE_DIM)) > ratio(quiet, f32::from(WORST_GROUND.0)));
        // the chip: white words on black at 62 % over a white picture
        let chip_ground = 255.0 * (1.0 - f32::from(CHIP.3) / 255.0);
        assert!((chip_ground - f32::from(CHIP_GROUND.0)).abs() <= 1.0, "{chip_ground} {}", CHIP_GROUND.0);
        assert!(ratio(255.0, f32::from(CHIP_GROUND.0)) >= 6.0, "{}", ratio(255.0, f32::from(CHIP_GROUND.0)));
    }
}
