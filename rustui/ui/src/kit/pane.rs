//! Panes of the TV's glass (`kit/LuPane.qml` of the QML shell): a sheet, a card or a pill of the Lucent material, at the TV's
//! radii, drawn by the compositor. The material comes from the toolkit's resolver (the mode's edge, lip and shadow), the radius
//! from the theme. Over an app's picture a pane lets the picture through down to the theme's `readingAlpha`, the least that keeps
//! the ink readable against the worst frame; the picture is never sampled there (the video plane cannot be, and a window's
//! buffer is not a layer of the engine), so the backdrop is the tint alone. Over Home the shared glass ([`glass_of`]) samples the
//! wallpaper as Home's cards do.

use super::Kit;
use compositor_protocol::{Backdrop, BackdropId, Glass, Props, Rect, Rgba, TintGradient};
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

/// The theme's glass, the one Home's cards have: the material's recipe with the wallpaper's prepared blur behind it (or the cached blur where
/// there is none), the theme's glass type and depth, the tint at the theme's `glass`; `lit` is the focused recipe. Over an app's picture it is
/// [`glass`]'s reading tint with no backdrop. Every pane that floats over Home or an app takes it, so a change of glass type or depth reaches all
/// of them at once.
pub fn glass_of(kit: &Kit, kind: PaneKind, h: f32, lit: bool, over_video: bool) -> Glass {
    let role = match kind {
        PaneKind::Sheet => SmartMaterial::Overlay,
        PaneKind::Card | PaneKind::Pill | PaneKind::Strip => SmartMaterial::Card,
        PaneKind::Control => SmartMaterial::Button,
    };
    let state = ControlState { focus_visible: lit, ..ControlState::default() };
    let pair = kit.ui.look.material(role, &state, MaterialContext::Independent);
    let base = glass(kit, kind, h, over_video);
    // The `clear` card glass shows the picture's own blur where Home has made it (the QML Home's look: the glass samples the picture, the page's scrim is not
    // under it); `frosted` shows the page below, scrim and all.
    let backdrop = match (over_video || kit.tier == ui::lucent::material::MaterialTier::T0, kit.wall_backdrop, kind) {
        (true, ..) => Backdrop::None,
        (false, Some(id), PaneKind::Card | PaneKind::Pill | PaneKind::Strip | PaneKind::Control) => Backdrop::Prepared { id },
        _ => Backdrop::Cached { radius: kit.tokens.blur },
    };
    let tint = if lit { kit.ui.look.colour(PaletteRole::GlassRaised) } else { kit.ui.look.colour(PaletteRole::Glass) };
    let edge = if lit { kit.tokens.extra(kit.mode).glass_focus_edge } else { kit.ui.look.colour(PaletteRole::Edge) };
    decorate(kit, kind, Glass { corner_radius: base.corner_radius, tint: if over_video { base.tint } else { tint }, edge, backdrop, saturation: kit.tokens.saturation, ..pair.rest }, over_video)
}

/// How opaque a pane must be over a picture nobody controls, with words on it, for what is behind it not to read through. Lucent's `scrim.hidesMin`
/// is .97 (`LANGUAGE.md` section 3: a weaker veil lets the words behind it read through); a banner over an ad's print holds to .985: at .97 black
/// print on white leaves 8 levels in 255 of ghost under the glass, which reads at twice the size, and at .985 it leaves 4, which does not.
pub const HIDES_MIN: f32 = 0.985;

/// `tint` made as opaque as `alpha` at the least, in the colour it has (it is premultiplied, so its channels grow with its alpha).
fn opaque_to(tint: Rgba, alpha: u8) -> Rgba {
    if tint.3 == 0 || tint.3 >= alpha {
        return tint;
    }
    let k = f32::from(alpha) / f32::from(tint.3);
    let up = |v: u8| (f32::from(v) * k).round().min(f32::from(alpha)) as u8;
    Rgba(up(tint.0), up(tint.1), up(tint.2), alpha)
}

/// The glass of a pane with words on it over words or a picture that must not read through it (the channel banner over video, the camera pop-up
/// over Home or an app): the pane's own material and the sheet's colour, but no more see-through than [`HIDES_MIN`] or the theme's hiding scrim
/// (`scrimHide`) lets it be, whichever hides more. The reading alpha of [`glass`] (0.8 as shipped) is the least that keeps a label's contrast over
/// the worst frame, and lets a busy picture show under the words.
pub fn reading_glass(kit: &Kit, kind: PaneKind, h: f32) -> Glass {
    let mut glass = glass(kit, kind, h, true);
    let scrim = f32::from(kit.tokens.extra(kit.mode).scrim_hide.3) / 255.0;
    let floor = (HIDES_MIN.max(scrim) * 255.0).round() as u8;
    glass.tint = opaque_to(glass.tint, floor);
    glass.tint_gradient = glass.tint_gradient.map(|g| TintGradient { top: opaque_to(g.top, floor), bottom: opaque_to(g.bottom, floor) });
    glass
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

/// Which glass a pane is restyled to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Recipe {
    /// [`glass`]: the sheet's colour, no backdrop.
    Plain,
    /// [`reading_glass`]: words over video.
    Reading,
    /// [`glass_of`]: the theme's glass, Home's.
    Themed,
}

pub struct Pane {
    layer: GlassLayer,
    frame: Frame,
    kind: PaneKind,
    over_video: bool,
    recipe: Recipe,
    /// The wallpaper's blur a [`Recipe::Themed`] pane was made over: the pane is made again when Home prepares another.
    wall: Option<BackdropId>,
    /// Only this part of the glass is drawn (logical pixels of the screen): a card with a hole in it is made of several panes of one frame, each
    /// showing a band of it.
    clip: Option<Rect>,
}

impl Pane {
    pub fn new(kit: &Kit, frame: Frame, z: i32, kind: PaneKind, over_video: bool) -> Self {
        let layer = kit.ui.scene.glass(frame.w.round() as u32, frame.h.round() as u32, z, glass(kit, kind, frame.h, over_video), Props::at(frame.x, frame.y));
        Self { layer, frame, kind, over_video, recipe: Recipe::Plain, wall: None, clip: None }
    }

    /// A pane of the theme's glass ([`glass_of`]): what Home's cards are made of, for a pane that floats over Home or an app.
    pub fn themed(kit: &Kit, frame: Frame, z: i32, kind: PaneKind, over_video: bool) -> Self {
        let layer = kit.ui.scene.glass(frame.w.round() as u32, frame.h.round() as u32, z, glass_of(kit, kind, frame.h, false, over_video), Props::at(frame.x, frame.y));
        Self { layer, frame, kind, over_video, recipe: Recipe::Themed, wall: kit.wall_backdrop, clip: None }
    }

    /// A pane of `kind` that hides the picture behind it ([`reading_glass`]): for words over video.
    pub fn reading(kit: &Kit, frame: Frame, z: i32, kind: PaneKind) -> Self {
        let layer = kit.ui.scene.glass(frame.w.round() as u32, frame.h.round() as u32, z, reading_glass(kit, kind, frame.h), Props::at(frame.x, frame.y));
        Self { layer, frame, kind, over_video: true, recipe: Recipe::Reading, wall: None, clip: None }
    }

    /// A [`Pane::reading`] pane that shows only the part of its glass inside `clip` (the screen's logical pixels): the other bands of the same frame,
    /// side by side, make the glass whole but for what they leave out (the hole a video plays through).
    pub fn reading_clipped(kit: &Kit, frame: Frame, z: i32, kind: PaneKind, clip: Rect) -> Self {
        let at = Props { clip: Some(clip), ..Props::at(frame.x, frame.y) };
        let layer = kit.ui.scene.glass(frame.w.round() as u32, frame.h.round() as u32, z, reading_glass(kit, kind, frame.h), at);
        Self { layer, frame, kind, over_video: true, recipe: Recipe::Reading, wall: None, clip: Some(clip) }
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
        self.wall = kit.wall_backdrop;
        self.layer.restyle(match self.recipe {
            Recipe::Plain => glass(kit, self.kind, self.frame.h, over_video),
            Recipe::Reading => reading_glass(kit, self.kind, self.frame.h),
            Recipe::Themed => glass_of(kit, self.kind, self.frame.h, false, over_video),
        });
    }

    /// Whether the pane's glass is still what [`Pane::restyle`] would make: a themed pane over Home samples the wallpaper's blur, which Home
    /// prepares again when the wallpaper changes.
    pub fn current(&self, kit: &Kit) -> bool {
        self.recipe != Recipe::Themed || self.over_video || self.wall == kit.wall_backdrop
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

    #[test]
    fn a_reading_pane_hides_the_picture_behind_it_in_every_glass_and_mode_and_keeps_the_sheets_colour() {
        let hides = (HIDES_MIN * 255.0).round() as u8;
        for mode in [Mode::Dark, Mode::Light] {
            for card_glass in [CardGlass::Clear, CardGlass::Frosted, CardGlass::Liquid, CardGlass::Glazed] {
                let (_h, mut kit) = testkit::kit_in(mode, 1.0);
                kit.tokens.card_glass = card_glass;
                let over_video = glass(&kit, PaneKind::Card, 98.0, true);
                let reading = reading_glass(&kit, PaneKind::Card, 98.0);
                assert!(reading.tint.3 >= hides, "{mode:?} {card_glass:?}: {:?}", reading.tint);
                if let Some(g) = reading.tint_gradient {
                    assert!(g.top.3 >= hides && g.bottom.3 >= hides, "{mode:?} {card_glass:?}: {g:?}");
                }
                // the sheet's own colour, only less see-through: premultiplied, the channels stay in step with the alpha
                let straight = |c: Rgba| [c.0, c.1, c.2].map(|v| f32::from(v) / f32::from(c.3.max(1)));
                for (a, b) in straight(over_video.tint).into_iter().zip(straight(reading.tint)) {
                    assert!((a - b).abs() < 0.02, "{mode:?} {card_glass:?}: the colour is the sheet's");
                }
            }
        }
    }
}
