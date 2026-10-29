//! Flat shapes the compositor draws: a rounded plate of one colour, and where it goes. Not a material (a button or a card is a
//! glass surface, [`super::surface`]): a plate is for a caret, a scrim, a progress bar, a ring.

use crate::context::{Look, Ui};
use crate::geometry::Frame;
use crate::lucent::palette::PaletteRole;
use crate::scene::ShapeLayer;
use compositor_protocol::{Fill, Props, Rgba, Shape};

/// Nothing: the fill of a ring, the edge of a plate that has none.
pub const CLEAR: Rgba = Rgba(0, 0, 0, 0);

/// A rounded plate of one colour.
pub fn solid(colour: Rgba, radius: f32) -> Shape {
    Shape {
        corner_radius: radius,
        fill: Fill::Solid(colour),
        border_width: 0.0,
        border: CLEAR,
    }
}

/// A ring: no fill, a border `width` wide inside the edge.
pub fn ring(colour: Rgba, radius: f32, width: f32) -> Shape {
    Shape {
        corner_radius: radius,
        fill: Fill::Solid(CLEAR),
        border_width: width,
        border: colour,
    }
}

/// A shape layer of `look` covering `at`.
pub fn shape(ui: &Ui, at: Frame, z: i32, look: Shape) -> ShapeLayer {
    ui.scene.shape(
        at.w.round() as u32,
        at.h.round() as u32,
        z,
        look,
        Props::at(at.x, at.y),
    )
}

/// The plain background of a shell with no wallpaper yet, as a plate as big as the display: the sheet's colour at the top left
/// running into the canvas at the bottom right, opaque.
pub fn backdrop(look: &Look) -> Shape {
    let top = look.colour(PaletteRole::Sheet);
    let canvas = look.canvas();
    Shape {
        corner_radius: 0.0,
        fill: Fill::Linear {
            from: Rgba(top.0, top.1, top.2, 255),
            to: canvas,
            angle_deg: 35.0,
        },
        border_width: 0.0,
        border: canvas,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lucent::palette::Mode;

    #[test]
    fn the_backdrop_is_opaque_all_the_way_and_ends_in_the_canvas() {
        for mode in [Mode::Dark, Mode::Light] {
            let look = Look::standard(mode);
            let Fill::Linear { from, to, .. } = backdrop(&look).fill else {
                panic!("the backdrop is a gradient");
            };
            assert_eq!(
                (from.3, to.3),
                (255, 255),
                "{mode:?}: nothing shows through to a black panel"
            );
            assert_eq!(to, look.canvas());
            assert_ne!(from, to);
        }
    }
}
