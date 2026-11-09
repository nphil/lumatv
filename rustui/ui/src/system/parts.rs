//! The parts a system screen is made of (a pane, a plate, a glyph, a line of words), each with the place it rests at, so a screen shows
//! and hides them as one: fading in over the layer beat on arrival and out on leaving, or at once. A screen builds its parts when it
//! opens and drops the [`Sheet`] when it has gone (the scene's budget rule), so nothing hidden is kept alive.

use crate::kit::Kit;
use crate::kit::glyph;
use crate::kit::hint::{self, Hint};
use crate::kit::icon;
use crate::kit::motion::Beat;
use crate::kit::pane::Pane;
use crate::kit::text::{Text, role_style, width_of};
use lumatv_tokens::TypeRole;
use ui::components::Align;
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

pub struct Sheet {
    parts: Vec<(Part, Props)>,
    shown: bool,
    alpha: f32,
}

impl Default for Sheet {
    fn default() -> Self {
        Self { parts: Vec::new(), shown: false, alpha: 1.0 }
    }
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

    /// Moves every part by (`dx`, `dy`): the rests move, and the layers glide there while the sheet is shown (a card that moves up to the first place).
    pub fn shift(&mut self, kit: &Kit, dx: f32, dy: f32, at_once: bool) {
        for (part, at) in &mut self.parts {
            at.x += dx;
            at.y += dy;
            if let Part::Pane(p) = part {
                p.move_to(at.x, at.y);
            }
            if self.shown {
                if at_once {
                    part.layer().set(Props { opacity: at.opacity * self.alpha, ..*at });
                } else {
                    kit.glide(part.layer(), Props { opacity: at.opacity * self.alpha, ..*at }, Beat::Card);
                }
            }
        }
    }

    /// Draws the whole sheet at `factor` of its opacity (an item that waits is faint).
    pub fn set_alpha(&mut self, factor: f32) {
        self.alpha = factor;
        if self.shown {
            for (part, at) in &self.parts {
                part.layer().set(Props { opacity: at.opacity * factor, ..*at });
            }
        }
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

    /// Takes part `i` out of its rest place: it fades out over the layer beat when the sheet is shown.
    pub fn leave_part(&mut self, kit: &Kit, i: usize) {
        if let Some((part, rest)) = self.parts.get_mut(i) {
            let from = *rest;
            *rest = Props { opacity: 0.0, visible: false, ..from };
            if self.shown {
                kit.leave(part.layer(), Props { opacity: from.opacity * self.alpha, ..from }, Beat::Layer);
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

    /// Shows every part sliding in `dx` px from the side it comes from while it fades in over the card beat (a card's own motion), or at once.
    pub fn slide_in(&mut self, kit: &Kit, dx: f32, at_once: bool) {
        for (part, at) in &self.parts {
            let layer = part.layer();
            if at_once || kit.beat_ms(Beat::Card) == 0 {
                layer.set(*at);
            } else {
                layer.set(Props { opacity: 0.0, x: at.x + dx, ..*at });
                layer.animate(*at, kit.beat_ms(Beat::Card), compositor_protocol::Ease::Out);
            }
        }
        self.shown = true;
    }

    /// Takes every part away sliding `dx` px out and fading, quicker than it came.
    pub fn slide_out(&mut self, kit: &Kit, dx: f32) {
        for (part, at) in &self.parts {
            let gone = Props { opacity: 0.0, visible: false, x: at.x + dx, ..*at };
            if kit.beat_ms(Beat::Rest) == 0 {
                part.layer().set(gone);
            } else {
                part.layer().animate(gone, kit.beat_ms(Beat::Rest), compositor_protocol::Ease::In);
            }
        }
        self.shown = false;
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
    let ink = if accent { kit.ui.look.accent_ink() } else { kit.ui.look.ink() };
    tile(kit, size, z, name, accent, ink)
}

/// [`icon_tile`] with the icon in `glyph_ink` (a card's tone: the quiet tile with a warning's or a danger's own colour in it).
pub fn tile(kit: &mut Kit, size: f32, z: i32, name: &str, accent: bool, glyph_ink: Rgba) -> std::io::Result<(ShapeLayer, Option<ImageLayer>)> {
    let look = &kit.ui.look;
    let (fill, border) = if accent {
        (look.accent(), Rgba(0, 0, 0, 0))
    } else {
        (crate::kit::button::faded(look.ink(), kit.tokens.states.selected_wash), look.colour(ui::lucent::palette::PaletteRole::Edge))
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

/// A hint (`kit::hint`) laid into `sheet` at (`x`, `y`) in a row `h` high: its key glyphs and then its label, as parts of the sheet, so the
/// row comes and goes with the card. Returns the row's width and its accessible name.
#[allow(clippy::too_many_arguments)]
pub fn hint_into(kit: &mut Kit, sheet: &mut Sheet, hint: &Hint, role: TypeRole, x: f32, y: f32, h: f32, z: i32, ink: Rgba) -> std::io::Result<(f32, String)> {
    let side = hint::glyph_side(kit, role);
    let gap = kit.space(2);
    let mut pen = x;
    for name in hint.glyphs() {
        let colour = hint::glyph_colour_of(kit, name, ink);
        let at = (pen, (y + (h - side as f32) / 2.0).round());
        let mut layer = kit.ui.scene.image(side, side, z, HIDDEN)?;
        glyph::draw(name, side, colour, layer.pixels_mut());
        layer.damage(None);
        sheet.image(layer, at);
        pen += side as f32 + gap;
    }
    match &hint.label {
        Some(label) => {
            let style = role_style(kit, role, None, ink, Align::Left);
            let w = width_of(kit, style, label);
            let line = kit.line(role);
            let t = Text::new(kit, ui::geometry::Frame::new(pen, (y + (h - line) / 2.0).round(), w, line), z, style, label)?;
            sheet.text(t);
            pen += w;
        }
        None => pen -= gap,
    }
    Ok((pen - x, hint.name()))
}

/// The width a hint's row takes, measured the way [`hint_into`] lays it.
pub fn hint_width(kit: &mut Kit, hint: &Hint, role: TypeRole) -> f32 {
    let side = hint::glyph_side(kit, role) as f32;
    let gap = kit.space(2);
    let n = hint.glyphs().len() as f32;
    match &hint.label {
        Some(label) => {
            let style = role_style(kit, role, None, Rgba(0, 0, 0, 255), Align::Left);
            n * (side + gap) + width_of(kit, style, label)
        }
        None => n * (side + gap) - gap,
    }
}
