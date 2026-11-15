//! Missing game-specific symbols on the toolkit's 24-unit, rounded 1.9-unit stroke grid.
use compositor_protocol::Rgba;
use ui::ImageLayer;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
    Library,
    Controller,
    Heart,
    More,
}
#[derive(Clone, Copy)]
enum Shape {
    Line(f32, f32, f32, f32),
    Dot(f32, f32, f32),
}
fn segment(x: f32, y: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    let (vx, vy) = (c - a, d - b);
    let t = (((x - a) * vx + (y - b) * vy) / (vx * vx + vy * vy).max(0.001)).clamp(0.0, 1.0);
    (x - a - t * vx).hypot(y - b - t * vy)
}
pub fn draw(layer: &mut ImageLayer, symbol: Symbol, colour: Rgba) {
    use Shape::*;
    let shapes: &[Shape] = match symbol {
        Symbol::Library => &[
            Line(3.0, 4.0, 7.0, 4.0),
            Line(7.0, 4.0, 7.0, 20.0),
            Line(7.0, 20.0, 3.0, 20.0),
            Line(3.0, 20.0, 3.0, 4.0),
            Line(10.0, 4.0, 13.0, 4.0),
            Line(13.0, 4.0, 13.0, 20.0),
            Line(13.0, 20.0, 10.0, 20.0),
            Line(10.0, 20.0, 10.0, 4.0),
            Line(16.0, 3.0, 20.0, 19.0),
            Line(20.0, 19.0, 23.0, 18.0),
            Line(23.0, 18.0, 19.0, 2.0),
            Line(19.0, 2.0, 16.0, 3.0),
        ],
        Symbol::Controller => &[
            Line(6.0, 6.0, 18.0, 6.0),
            Line(18.0, 6.0, 20.0, 8.0),
            Line(20.0, 8.0, 22.0, 17.0),
            Line(22.0, 17.0, 20.0, 20.0),
            Line(20.0, 20.0, 15.0, 16.0),
            Line(15.0, 16.0, 9.0, 16.0),
            Line(9.0, 16.0, 4.0, 20.0),
            Line(4.0, 20.0, 2.0, 17.0),
            Line(2.0, 17.0, 4.0, 8.0),
            Line(4.0, 8.0, 6.0, 6.0),
            Line(5.0, 11.0, 10.0, 11.0),
            Line(7.5, 8.5, 7.5, 13.5),
            Dot(16.0, 10.0, 0.8),
            Dot(19.0, 13.0, 0.8),
        ],
        Symbol::Heart => &[
            Line(12.0, 21.0, 3.0, 12.0),
            Line(3.0, 12.0, 2.0, 8.0),
            Line(2.0, 8.0, 4.0, 4.0),
            Line(4.0, 4.0, 8.0, 3.0),
            Line(8.0, 3.0, 12.0, 7.0),
            Line(12.0, 7.0, 16.0, 3.0),
            Line(16.0, 3.0, 20.0, 4.0),
            Line(20.0, 4.0, 22.0, 8.0),
            Line(22.0, 8.0, 21.0, 12.0),
            Line(21.0, 12.0, 12.0, 21.0),
        ],
        Symbol::More => &[Dot(4.0, 12.0, 1.4), Dot(12.0, 12.0, 1.4), Dot(20.0, 12.0, 1.4)],
    };
    let w = layer.width();
    let h = layer.height();
    let scale = w.min(h) as f32 / 24.0;
    for y in 0..h {
        for x in 0..w {
            let px = (x as f32 + 0.5) / scale;
            let py = (y as f32 + 0.5) / scale;
            let mut distance = f32::MAX;
            for shape in shapes {
                let d = match *shape {
                    Line(a, b, c, d) => segment(px, py, a, b, c, d) - 0.95,
                    Dot(a, b, r) => (px - a).hypot(py - b) - r,
                };
                distance = distance.min(d);
            }
            let coverage = (0.5 - distance * scale).clamp(0.0, 1.0);
            let i = ((y * w + x) * 4) as usize;
            for (c, value) in [colour.0, colour.1, colour.2, colour.3].into_iter().enumerate() {
                layer.pixels_mut()[i + c] = (f32::from(value) * coverage).round() as u8;
            }
        }
    }
    layer.damage(None);
}
