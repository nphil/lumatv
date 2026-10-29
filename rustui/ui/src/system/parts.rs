//! The parts a system screen is made of (a pane, a plate, a glyph, a line of words), each with the place it rests at, so a screen shows
//! and hides them as one: fading in over the layer beat on arrival and out on leaving, or at once. A screen builds its parts when it
//! opens and drops the [`Sheet`] when it has gone (the scene's budget rule), so nothing hidden is kept alive.

use crate::kit::Kit;
use crate::kit::icon;
use crate::kit::motion::Beat;
use crate::kit::pane::Pane;
use crate::kit::text::Text;
use compositor_protocol::{Props, Rgba};
use lumatv_tokens::Radius;
use ui::components::plate;
use ui::geometry::Frame;
use ui::scene::{ImageLayer, Layer, ShapeLayer};

pub const HIDDEN: Props = Props { opacity: 0.0, visible: false, ..Props::at(0.0, 0.0) };

pub enum Part {
    Shape(ShapeLayer),
    Image(ImageLayer),
    Pane(Pane),
    Text(Text),
}

impl Part {
    pub fn layer(&self) -> &Layer {
        match self {
            Part::Shape(l) => l,
            Part::Image(l) => l,
            Part::Pane(p) => p.layer(),
            Part::Text(t) => t.layer(),
        }
    }
}

#[derive(Default)]
pub struct Sheet {
    parts: Vec<(Part, Props)>,
    shown: bool,
}

impl Sheet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a part at its rest place; it is hidden until [`Sheet::show`]. Returns its index.
    pub fn add(&mut self, part: Part, at: Props) -> usize {
        part.layer().set(HIDDEN);
        self.parts.push((part, at));
        self.parts.len() - 1
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

    /// How many layers the parts are.
    pub fn layers(&self) -> usize {
        self.parts.len()
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    pub fn text_mut(&mut self, i: usize) -> Option<&mut Text> {
        match self.parts.get_mut(i) {
            Some((Part::Text(t), _)) => Some(t),
            _ => None,
        }
    }

    pub fn shape_mut(&mut self, i: usize) -> Option<&ShapeLayer> {
        match self.parts.get(i) {
            Some((Part::Shape(s), _)) => Some(s),
            _ => None,
        }
    }

    pub fn image_mut(&mut self, i: usize) -> Option<&mut ImageLayer> {
        match self.parts.get_mut(i) {
            Some((Part::Image(l), _)) => Some(l),
            _ => None,
        }
    }

    /// The rest place of part `i`.
    pub fn at(&self, i: usize) -> Option<Props> {
        self.parts.get(i).map(|(_, at)| *at)
    }

    /// Moves part `i`'s rest place (and its layer, when the sheet is shown).
    pub fn move_part(&mut self, i: usize, at: Props) {
        if let Some((part, rest)) = self.parts.get_mut(i) {
            *rest = at;
            if self.shown {
                part.layer().set(at);
            }
        }
    }

    /// Shows every part at its rest place: fading in over the layer beat, or at once.
    pub fn show(&mut self, kit: &Kit, at_once: bool) {
        for (part, at) in &self.parts {
            kit.arrive(part.layer(), *at, Beat::Layer, at_once);
        }
        self.shown = true;
    }

    /// Takes every part away: fading out over the layer beat, or at once.
    pub fn hide(&mut self, kit: &Kit, at_once: bool) {
        for (part, at) in &self.parts {
            if at_once {
                part.layer().set(Props { opacity: 0.0, visible: false, ..*at });
            } else {
                kit.leave(part.layer(), *at, Beat::Layer);
            }
        }
        self.shown = false;
    }
}

/// The rounded square an icon sits in (`kit/LuTile.qml`): a quiet wash and an edge at rest, the accent with the ink that reads on it when
/// `accent`, the icon at 55 % of the tile. Returns the plate and the icon (`None` for a name that is not an icon), to be added to a
/// sheet at the tile's place.
pub fn icon_tile(kit: &mut Kit, size: f32, z: i32, name: &str, accent: bool) -> std::io::Result<(ShapeLayer, Option<ImageLayer>)> {
    let look = &kit.ui.look;
    let (fill, border, glyph_ink) = if accent {
        (look.accent(), Rgba(0, 0, 0, 0), look.accent_ink())
    } else {
        (crate::kit::button::faded(look.ink(), kit.tokens.states.selected_wash), look.colour(ui::lucent::palette::PaletteRole::Edge), look.ink())
    };
    let radius = (size * 0.32).round();
    let shape = compositor_protocol::Shape {
        corner_radius: radius,
        fill: compositor_protocol::Fill::Solid(fill),
        border_width: if accent { 0.0 } else { 1.0 },
        border,
    };
    let plate = kit.ui.scene.shape(size as u32, size as u32, z, shape, HIDDEN);
    let side = (size * 0.55).round() as u32;
    let glyph = icon::layer(kit, name, side, glyph_ink, z + 1, HIDDEN)?;
    Ok((plate, glyph))
}

/// The tile's icon is placed in the middle of the tile at `at`.
pub fn tile_glyph_at(at: (f32, f32), size: f32) -> (f32, f32) {
    let side = (size * 0.55).round();
    (at.0 + ((size - side) / 2.0).round(), at.1 + ((size - side) / 2.0).round())
}

/// A solid plate of the theme's radius, for a scrim or a cell.
pub fn plate_of(kit: &Kit, frame: Frame, z: i32, fill: Rgba, radius: Radius) -> ShapeLayer {
    plate::shape(&kit.ui, frame, z, plate::solid(fill, kit.tokens.radius(radius)))
}
