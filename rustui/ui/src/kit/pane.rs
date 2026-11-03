//! Panes of the TV's glass (`kit/LuPane.qml` of the QML shell): a sheet, a card or a pill of the Lucent material, at the TV's
//! radii, drawn by the compositor. The material comes from the toolkit's resolver (the mode's edge, lip and shadow), the radius
//! from the theme. Over an app's picture a pane lets the picture through down to the theme's `readingAlpha`, the least that keeps
//! the ink readable against the worst frame; the picture is never sampled there (the video plane cannot be, and a window's
//! buffer is not a layer of the engine), so the backdrop is the tint alone until Home is a layer stack of the engine.

use super::Kit;
use compositor_protocol::{Backdrop, Glass, Props, Rect};
use lumatv_tokens::{CardGlass, Radius};
use ui::geometry::Frame;
use ui::lucent::material::{ControlState, MaterialContext, SmartMaterial};
use ui::lucent::palette::{PaletteRole, Straight};
use ui::scene::GlassLayer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneKind {
    /// A sheet or a dialog: `radius.sheet`.
    Sheet,
    /// A card or a toast: `radius.card`.
    Card,
    /// A pill as round as its height.
    Pill,
    /// A control: `radius.control`.
    Control,
    /// A bar over panes (Settings' title bar): the card's material at half the sheet's radius.
    Strip,
}

impl PaneKind {
    fn radius(self, kit: &Kit, h: f32) -> f32 {
        match self {
            PaneKind::Sheet => kit.tokens.radius(Radius::Sheet),
            PaneKind::Card => kit.tokens.radius(Radius::Card),
            PaneKind::Pill => (h / 2.0).floor(),
            PaneKind::Control => kit.tokens.radius(Radius::Control),
            PaneKind::Strip => (kit.tokens.radius(Radius::Sheet) / 2.0).floor(),
        }
    }
}

/// The glass of a pane of `kind` and height `h`: `over_video` paints the sheet at the reading alpha (the picture shows through
/// that much), else at the sheet's own.
pub fn glass(kit: &Kit, kind: PaneKind, h: f32, over_video: bool) -> Glass {
    let role = match kind {
        PaneKind::Sheet => SmartMaterial::Overlay,
        PaneKind::Card | PaneKind::Pill | PaneKind::Strip => SmartMaterial::Card,
        PaneKind::Control => SmartMaterial::Button,
    };
    let pair = kit.ui.look.material(role, &ControlState::default(), MaterialContext::Independent);
    let sheet = kit.ui.look.theme.roles[PaletteRole::Sheet];
    let tint = if over_video {
        Straight { a: (kit.tokens.extra(kit.mode).reading_alpha * 255.0).round() as u8, ..sheet }.premultiply(0.0)
    } else {
        sheet.premultiply(0.0)
    };
    decorate(kit, kind, Glass { corner_radius: kind.radius(kit, h), tint, backdrop: Backdrop::None, ..pair.rest }, over_video)
}

/// Applies the selected surface and independent geometric depth to every pane.
pub fn decorate(kit: &Kit, kind: PaneKind, mut glass: Glass, over_video: bool) -> Glass {
    let strength = kit.tokens.glass_strength().clamp(0.0, 1.0);
    let depth = kit.tokens.depth_scale.max(0.0);
    if depth == 0.0 {
        glass.shadows = [None, None];
        glass.depth = None;
    } else {
        for shadow in glass.shadows.iter_mut().flatten() {
            shadow.offset_y *= depth;
            shadow.blur *= depth;
        }
        if let Some(d) = &mut glass.depth {
            d.offset *= depth;
            d.blur *= depth;
        }
    }
    let scale = |c: compositor_protocol::Rgba, k: f32| compositor_protocol::Rgba(
        (c.0 as f32 * k).round() as u8, (c.1 as f32 * k).round() as u8,
        (c.2 as f32 * k).round() as u8, (c.3 as f32 * k).round() as u8);
    match kit.tokens.card_glass {
        CardGlass::Clear => {
            glass.edge = scale(glass.edge, strength);
            glass.highlight = scale(glass.highlight, strength);
            if let Some(sheen) = &mut glass.sheen { sheen.peak *= strength; sheen.floor *= strength; }
        }
        CardGlass::Frosted => {
            glass.saturation += 0.35 * (1.0 - strength);
            if let Backdrop::Cached { radius } = &mut glass.backdrop { *radius *= 0.5 + 0.5 * strength; }
            if glass.backdrop == Backdrop::None && !over_video && strength < 1.0 {
                // A backdrop-free pane has no blur to vary; its tint is the matte scattering layer.
                let alpha = glass.tint.3 as f32;
                let floor = if matches!(kind, PaneKind::Sheet) { 0.88 * 255.0 } else { 0.0 };
                let next = (alpha * (0.90 + 0.10 * strength)).max(floor).min(alpha);
                if alpha > 0.0 { glass.tint = scale(glass.tint, next / alpha); }
            }
        }
        CardGlass::Liquid if !over_video && kit.tier != ui::lucent::material::MaterialTier::T0 && glass.backdrop != Backdrop::None => {
            glass.edge_lens = Some(compositor_protocol::EdgeLens { bevel: 12.0, lens: 24.0 * strength, rim_clear: 0.6 * strength });
            let colour = glass.tint;
            let boost = 1.0 + 0.08 * strength;
            let bottom = compositor_protocol::Rgba(
                (colour.0 as f32 * boost).min(255.0).round() as u8,
                (colour.1 as f32 * boost).min(255.0).round() as u8,
                (colour.2 as f32 * boost).min(255.0).round() as u8,
                (colour.3 as f32 * boost).min(255.0).round() as u8,
            );
            glass.tint_gradient = Some(compositor_protocol::TintGradient { top: colour, bottom });
        }
        CardGlass::Glazed => {
            let minimum = if matches!(kind, PaneKind::Sheet) || over_video { 0.96 } else if kit.mode == ui::lucent::palette::Mode::Dark { 0.62 } else { 0.72 };
            let sheet = if matches!(kind, PaneKind::Sheet) || over_video {
                kit.ui.look.theme.roles[PaletteRole::Sheet]
            } else {
                let raised = kit.ui.look.theme.roles[PaletteRole::GlassRaised];
                if kit.mode == ui::lucent::palette::Mode::Dark {
                    let premultiplied = raised.premultiply(0.0);
                    Straight { r: premultiplied.0, g: premultiplied.1, b: premultiplied.2, ..raised }
                } else { raised }
            };
            let alpha = (minimum + (1.0 - minimum) * (1.0 - strength) * 0.12).min(1.0);
            glass.tint = Straight { a: (alpha * 255.0).round() as u8, ..sheet }.premultiply(0.0);
            let bottom = Straight { a: ((alpha + 0.04 * strength).min(1.0) * 255.0).round() as u8, ..sheet }.premultiply(0.0);
            glass.tint_gradient = Some(compositor_protocol::TintGradient { top: glass.tint, bottom });
            glass.highlight = scale(glass.highlight, 0.6 + 0.4 * strength);
            if let Some(sheen) = &mut glass.sheen { sheen.peak *= strength; sheen.floor *= strength; }
        }
        _ => {}
    }
    if kit.tier == ui::lucent::material::MaterialTier::T0 {
        let sheet = kit.ui.look.theme.roles[PaletteRole::Sheet];
        glass.tint = Straight { a: 255, ..sheet }.premultiply(0.0);
        glass.tint_gradient = None;
        glass.edge_lens = None;
    }
    glass
}

pub struct Pane {
    layer: GlassLayer,
    frame: Frame,
    kind: PaneKind,
    over_video: bool,
    /// Only this part of the glass is drawn (logical pixels of the screen): a card with a hole in it is made of several panes of one frame, each
    /// showing a band of it.
    clip: Option<Rect>,
}

impl Pane {
    pub fn new(kit: &Kit, frame: Frame, z: i32, kind: PaneKind, over_video: bool) -> Self {
        let layer = kit.ui.scene.glass(frame.w.round() as u32, frame.h.round() as u32, z, glass(kit, kind, frame.h, over_video), Props::at(frame.x, frame.y));
        Self { layer, frame, kind, over_video, clip: None }
    }

    /// A pane that shows only the part of its glass inside `clip` (the screen's logical pixels): the other bands of the same frame, side by side, make the
    /// glass whole but for what they leave out (the hole a video plays through).
    pub fn clipped(kit: &Kit, frame: Frame, z: i32, kind: PaneKind, over_video: bool, clip: Rect) -> Self {
        let at = Props { clip: Some(clip), ..Props::at(frame.x, frame.y) };
        let layer = kit.ui.scene.glass(frame.w.round() as u32, frame.h.round() as u32, z, glass(kit, kind, frame.h, over_video), at);
        Self { layer, frame, kind, over_video, clip: Some(clip) }
    }

    pub fn layer(&self) -> &GlassLayer {
        &self.layer
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    /// The pane's rest properties: where it was made.
    pub fn at(&self) -> Props {
        Props { clip: self.clip, ..Props::at(self.frame.x, self.frame.y) }
    }

    /// Moves the pane's rest (its size cannot change: a pane that must be wider is made again).
    pub fn move_to(&mut self, x: f32, y: f32) {
        self.frame.x = x;
        self.frame.y = y;
    }

    /// Restyles for a new look or a change of what is under it.
    pub fn restyle(&mut self, kit: &Kit, over_video: bool) {
        self.over_video = over_video;
        self.layer.restyle(glass(kit, self.kind, self.frame.h, over_video));
    }

    pub fn over_video(&self) -> bool {
        self.over_video
    }
}

#[cfg(test)]
mod material_tests {
    use super::*;
    use crate::kit::testkit;
    use ui::lucent::palette::Mode;

    #[test]
    fn material_strength_and_depth_are_independent() {
        let (_h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        kit.tokens.card_glass = CardGlass::Liquid;
        let mut base = glass(&kit, PaneKind::Card, 200.0, false);
        base.backdrop = Backdrop::Cached { radius: 24.0 };
        kit.tokens.glass_strengths[CardGlass::Liquid.index()] = 0.5;
        kit.tokens.depth_scale = 0.0;
        let liquid = decorate(&kit, PaneKind::Card, base, false);
        assert_eq!(liquid.edge_lens.unwrap().lens, 12.0);
        assert_eq!(liquid.shadows, [None, None]);
        assert_eq!(liquid.depth, None);
        kit.tokens.depth_scale = 1.5;
        let deeper = decorate(&kit, PaneKind::Card, base, false);
        assert_eq!(deeper.edge_lens, liquid.edge_lens);
        assert_eq!(deeper.shadows[0].map(|s| s.offset_y), base.shadows[0].map(|s| s.offset_y * 1.5));
    }

    #[test]
    fn glazed_and_video_surfaces_stay_readable() {
        for mode in [Mode::Dark, Mode::Light] {
            let (_h, mut kit) = testkit::kit_in(mode, 1.0);
            kit.tokens.card_glass = CardGlass::Glazed;
            let card = glass(&kit, PaneKind::Card, 200.0, false);
            assert!(card.tint.3 >= if mode == Mode::Dark { 158 } else { 183 });
            let sheet = glass(&kit, PaneKind::Sheet, 200.0, true);
            assert!(sheet.tint.3 >= 244);
            assert_eq!(sheet.backdrop, Backdrop::None);
            assert_eq!(sheet.edge_lens, None);
        }
    }

    #[test]
    fn frosted_matte_changes_without_sampling_video() {
        let (_h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        kit.tokens.card_glass = CardGlass::Frosted;
        kit.tokens.glass_strengths[CardGlass::Frosted.index()] = 1.0;
        let mut base = glass(&kit, PaneKind::Sheet, 200.0, false);
        base.tint = compositor_protocol::Rgba(40, 40, 40, 255);
        let full = decorate(&kit, PaneKind::Sheet, base, false);
        let video = glass(&kit, PaneKind::Sheet, 200.0, true);
        kit.tokens.glass_strengths[CardGlass::Frosted.index()] = 0.0;
        let light = decorate(&kit, PaneKind::Sheet, base, false);
        assert!(light.tint.3 < full.tint.3);
        assert!(light.tint.3 >= 224);
        assert_eq!(glass(&kit, PaneKind::Sheet, 200.0, true).tint, video.tint);
    }

    #[test]
    fn glazed_keeps_both_depth_shadows() {
        let (_h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        kit.tokens.card_glass = CardGlass::Glazed;
        let mut base = glass(&kit, PaneKind::Card, 200.0, false);
        let shadow = compositor_protocol::Shadow { offset_y: 2.0, blur: 4.0, colour: compositor_protocol::Rgba(0, 0, 0, 30) };
        base.shadows = [Some(shadow), Some(shadow)];
        kit.tokens.depth_scale = 1.5;
        let result = decorate(&kit, PaneKind::Card, base, false);
        assert_eq!(result.shadows[1].unwrap().offset_y, 3.0);
        assert_eq!(result.shadows[1].unwrap().blur, 6.0);
    }
}
