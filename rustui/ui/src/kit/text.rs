//! Text in the TV's type roles: one line in a box ([`Text`], the toolkit's `Label` with the role's type at the person's text size)
//! and the measure a layout needs before it places anything. Glyphs are quads of the shared atlas; a role's characters are warmed
//! by [`Kit::new`], so a label made on a frame rasters nothing.

use super::Kit;
use compositor_protocol::{Props, Rgba};
use lumatv_tokens::TypeRole;
use ui::components::{Align, Label, Style, style};
use ui::geometry::Frame;
use ui::text::Weight;

/// The style of `role` in `colour`, aligned `align`, at the person's text size; `weight` overrides the role's.
pub fn role_style(kit: &Kit, role: TypeRole, weight: Option<Weight>, colour: Rgba, align: Align) -> Style {
    let mut ty = kit.ty(role);
    if let Some(w) = weight {
        ty.weight = w;
    }
    style(ty, colour, align)
}

/// The width `text` takes in `style`'s type, whole pixels, with a pixel to spare at each side.
pub fn width_of(kit: &mut Kit, style: Style, text: &str) -> f32 {
    kit.ui.fonts.measure(text, style.ty).width.ceil() + 2.0
}

/// The width of the widest of `texts` (a number's box is sized to "100" so the bar's start never moves as the digits change).
pub fn widest(kit: &mut Kit, style: Style, texts: &[&str]) -> f32 {
    texts.iter().map(|t| width_of(kit, style, t)).fold(0.0, f32::max)
}

/// One line of text in a fixed box.
pub struct Text {
    label: Label,
    frame: Frame,
}

impl Text {
    pub fn new(kit: &mut Kit, frame: Frame, z: i32, style: Style, text: &str) -> std::io::Result<Self> {
        Ok(Self { label: Label::new(&mut kit.ui, frame, z, style, text)?, frame })
    }

    /// A box one line of `style` high, `w` wide, with its text centred on the line through `cy`.
    pub fn line_at(kit: &mut Kit, x: f32, cy: f32, w: f32, z: i32, style: Style, text: &str) -> std::io::Result<Self> {
        let h = ui::components::line_height(style.ty);
        Self::new(kit, Frame::new(x, (cy - h / 2.0).round(), w, h), z, style, text)
    }

    pub fn set(&mut self, kit: &mut Kit, text: &str) {
        self.label.set(&mut kit.ui, text);
    }

    pub fn set_colour(&mut self, kit: &mut Kit, colour: Rgba) {
        self.label.set_colour(&mut kit.ui, colour);
    }

    pub fn text(&self) -> &str {
        self.label.text()
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn at(&self) -> Props {
        Props::at(self.frame.x, self.frame.y)
    }

    pub fn layer(&self) -> &ui::scene::Layer {
        self.label.layer()
    }

    pub fn set_props(&mut self, props: Props) {
        self.label.set_props(props);
    }

    /// Whether the text is glyph quads and not a picture (the host harness reports it).
    pub fn is_glyph_run(&self) -> bool {
        self.label.is_glyph_run()
    }
}
