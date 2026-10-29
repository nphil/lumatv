//! What the Cameras surfaces share: a set of layers that shows and hides as one (with the app's own fade times), the palette they read,
//! and sizes that follow the person's text size.

use crate::kit::Kit;
use crate::kit::pane::Pane;
use crate::kit::text::Text;
use crate::system::parts::{HIDDEN, Part};
use compositor_protocol::{Ease, Props, Rgba};
use ui::scene::{ImageLayer, ShapeLayer};

/// The colours the screens paint with, read from the look once per build.
#[derive(Clone, Copy, Debug)]
pub struct Ink {
    pub ink: Rgba,
    pub ink2: Rgba,
    pub ink3: Rgba,
    pub accent: Rgba,
    pub accent_ink: Rgba,
    pub live: Rgba,
    pub warning: Rgba,
    pub danger: Rgba,
    pub edge: Rgba,
    pub edge_raised: Rgba,
}

impl Ink {
    pub fn of(kit: &Kit) -> Self {
        use ui::lucent::palette::PaletteRole as R;
        let look = &kit.ui.look;
        Self {
            ink: look.ink(),
            ink2: look.ink2(),
            ink3: look.ink3(),
            accent: look.accent(),
            accent_ink: look.accent_ink(),
            live: look.colour(R::Live),
            warning: look.warning(),
            danger: look.danger(),
            edge: look.colour(R::Edge),
            edge_raised: look.colour(R::EdgeRaised),
        }
    }
}

/// The text size as a factor, one for 100 % and up to two.
pub fn scale(kit: &Kit) -> f32 {
    kit.text_scale.clamp(1.0, 2.0)
}

/// An icon beside text: 32 px at 100 % text, growing half as fast as the type (the mocks keep their icons at 32 px at 200 %, which reads
/// small against 56 px type).
pub fn icon_side(kit: &Kit, base: f32) -> u32 {
    (base * (1.0 + (scale(kit) - 1.0) * 0.5)).round() as u32
}

/// A group of layers that shows and hides as one, each at the place it rests at.
#[derive(Default)]
pub struct Parts {
    items: Vec<(Part, Props)>,
    shown: bool,
}

impl Parts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a part at its rest place; it is hidden until [`Parts::fade`]. Returns its index.
    pub fn add(&mut self, part: Part, at: Props) -> usize {
        part.layer().set(HIDDEN);
        self.items.push((part, at));
        self.items.len() - 1
    }

    pub fn shape(&mut self, layer: ShapeLayer, at: (f32, f32)) -> usize {
        self.add(Part::Shape(layer), Props::at(at.0, at.1))
    }

    pub fn image(&mut self, layer: ImageLayer, at: (f32, f32)) -> usize {
        self.add(Part::Image(layer), Props::at(at.0, at.1))
    }

    pub fn pane(&mut self, pane: Pane) -> usize {
        let at = pane.at();
        self.add(Part::Pane(pane), at)
    }

    pub fn text(&mut self, text: Text) -> usize {
        let at = text.at();
        self.add(Part::Text(text), at)
    }

    pub fn layers(&self) -> usize {
        self.items.len()
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    pub fn text_mut(&mut self, i: usize) -> Option<&mut Text> {
        match self.items.get_mut(i) {
            Some((Part::Text(t), _)) => Some(t),
            _ => None,
        }
    }

    pub fn shape_ref(&self, i: usize) -> Option<&ShapeLayer> {
        match self.items.get(i) {
            Some((Part::Shape(s), _)) => Some(s),
            _ => None,
        }
    }

    pub fn image_mut(&mut self, i: usize) -> Option<&mut ImageLayer> {
        match self.items.get_mut(i) {
            Some((Part::Image(l), _)) => Some(l),
            _ => None,
        }
    }

    pub fn at(&self, i: usize) -> Option<Props> {
        self.items.get(i).map(|(_, at)| *at)
    }

    /// Moves part `i`'s rest place (and its layer, when the group is shown).
    pub fn move_part(&mut self, i: usize, at: Props) {
        if let Some((part, rest)) = self.items.get_mut(i) {
            *rest = at;
            if self.shown {
                part.layer().set(at);
            }
        }
    }

    /// Shows the group (`on`) or takes it away, over `ms` (the look's reduced-motion ceiling applied) or at once for 0.
    pub fn fade(&mut self, kit: &Kit, on: bool, ms: u32) {
        let ms = if ms == 0 { 0 } else { kit.ui.look.duration_ms(ms) };
        for (part, at) in &self.items {
            let layer = part.layer();
            if on {
                if ms == 0 {
                    layer.set(*at);
                } else {
                    layer.set(Props { opacity: 0.0, ..*at });
                    layer.animate(*at, ms, Ease::Out);
                }
            } else if ms == 0 {
                layer.set(Props { opacity: 0.0, visible: false, ..*at });
            } else {
                layer.animate(Props { opacity: 0.0, visible: false, ..*at }, ms, Ease::In);
            }
        }
        self.shown = on;
    }

    /// Sets every part to `opacity` of its rest (a veil of the whole group).
    pub fn set_opacity(&mut self, opacity: f32) {
        for (part, at) in &self.items {
            part.layer().set(Props { opacity, visible: opacity > 0.0, ..*at });
        }
        self.shown = opacity > 0.0;
    }
}

/// A piece of a screen that is made again when what it says changes: its signature, what it is, and where it sits for the lint.
pub struct Slot<S, E> {
    pub sig: Option<S>,
    pub parts: Option<Parts>,
    pub extra: Option<E>,
}

impl<S: PartialEq, E> Default for Slot<S, E> {
    fn default() -> Self {
        Self { sig: None, parts: None, extra: None }
    }
}

impl<S: PartialEq, E> Slot<S, E> {
    /// Makes the piece for `sig` when it is not the one that is there (dropping the old one), or drops it for `None`. `build` runs only
    /// when it must. The piece is left hidden: the caller shows it with the screen's own fade.
    pub fn set<F>(&mut self, sig: Option<S>, build: F) -> std::io::Result<bool>
    where
        F: FnOnce() -> std::io::Result<(Parts, E)>,
    {
        if self.sig == sig {
            return Ok(false);
        }
        self.parts = None;
        self.extra = None;
        if sig.is_some() {
            let (parts, extra) = build()?;
            self.parts = Some(parts);
            self.extra = Some(extra);
        }
        self.sig = sig;
        Ok(true)
    }

    pub fn layers(&self) -> usize {
        self.parts.as_ref().map_or(0, Parts::layers)
    }
}
