//! Panes of the TV's glass (`kit/LuPane.qml` of the QML shell): a sheet, a card or a pill of the Lucent material, at the TV's
//! radii, drawn by the compositor. The material comes from the toolkit's resolver (the mode's edge, lip and shadow), the radius
//! from the theme. Over an app's picture a pane lets the picture through down to the theme's `readingAlpha`, the least that keeps
//! the ink readable against the worst frame; the picture is never sampled there (the video plane cannot be, and a window's
//! buffer is not a layer of the engine), so the backdrop is the tint alone until Home is a layer stack of the engine.

use super::Kit;
use compositor_protocol::{Backdrop, Glass, Props, Rect};
use lumatv_tokens::Radius;
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
    Glass { corner_radius: kind.radius(kit, h), tint, backdrop: Backdrop::None, ..pair.rest }
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
