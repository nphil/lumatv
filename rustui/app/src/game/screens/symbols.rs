//! Missing game-specific symbols on the toolkit's 24-unit, rounded 1.9-unit stroke grid, and the face-button markings of the pads
//! hints can name.
use crate::game::input::{Face, PadKind};
use compositor_protocol::Rgba;
use ui::ImageLayer;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
    Library,
    Controller,
    Heart,
    More,
    /// The marking printed on one face button of a pad that is not Xbox-styled, in a 24-unit box the size of the button. `ring` adds the
    /// button's rim, for a button that must not vanish into a dark sheet.
    Face { kind: PadKind, face: Face, ring: bool },
}
#[derive(Clone, Copy)]
enum Shape {
    Line(f32, f32, f32, f32),
    Dot(f32, f32, f32),
    /// A circle's outline: centre, radius and half the stroke.
    Ring(f32, f32, f32, f32),
}
fn segment(x: f32, y: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    let (vx, vy) = (c - a, d - b);
    let t = (((x - a) * vx + (y - b) * vy) / (vx * vx + vy * vy).max(0.001)).clamp(0.0, 1.0);
    (x - a - t * vx).hypot(y - b - t * vy)
}
/// The stroke's half-width on the toolkit's grid.
const STROKE: f32 = 0.95;

/// A polyline along the ellipse's arc from `from` to `to` degrees (0 is right, 90 is down).
fn arc(out: &mut Vec<Shape>, cx: f32, cy: f32, rx: f32, ry: f32, from: f32, to: f32) {
    const STEPS: usize = 12;
    let at = |i: usize| {
        let a = (from + (to - from) * i as f32 / STEPS as f32).to_radians();
        (cx + rx * a.cos(), cy + ry * a.sin())
    };
    for i in 0..STEPS {
        let ((a, b), (c, d)) = (at(i), at(i + 1));
        out.push(Shape::Line(a, b, c, d));
    }
}

/// The GameSir Cyclone 2's letters: white, light geometric sans, about 0.46 of the button tall with a stroke of about 0.06 of its
/// width (docs/research/controller-glyphs.md).
fn gamesir(face: Face) -> Vec<Shape> {
    use Shape::Line;
    let mut s = Vec::new();
    match face {
        Face::A => s.extend([Line(7.6, 17.5, 12.0, 6.5), Line(12.0, 6.5, 16.4, 17.5), Line(9.3, 13.7, 14.7, 13.7)]),
        Face::B => {
            s.extend([Line(8.8, 6.5, 8.8, 17.5), Line(8.8, 6.5, 12.3, 6.5), Line(8.8, 12.0, 12.6, 12.0), Line(8.8, 17.5, 12.6, 17.5)]);
            arc(&mut s, 12.3, 9.25, 2.7, 2.75, -90.0, 90.0);
            arc(&mut s, 12.6, 14.75, 3.0, 2.75, -90.0, 90.0);
        }
        Face::X => s.extend([Line(7.3, 6.5, 16.7, 17.5), Line(16.7, 6.5, 7.3, 17.5)]),
        Face::Y => s.extend([Line(7.2, 6.5, 12.0, 12.2), Line(16.8, 6.5, 12.0, 12.2), Line(12.0, 12.2, 12.0, 17.5)]),
    }
    s
}

/// The Leadjoy Xeno Plus's letters are printed as dot matrices. `#` is a dot.
fn leadjoy(face: Face) -> Vec<Shape> {
    const A: [&str; 8] = [".#####.", "#######", "##...##", "##...##", "#######", "#######", "##...##", "##...##"];
    const B: [&str; 8] = ["######.", "#######", "##...##", "######.", "#######", "##...##", "#######", "######."];
    const X: [&str; 8] = ["##....##", "###..###", ".######.", "..####..", "..####..", ".######.", "###..###", "##....##"];
    const Y: [&str; 9] = ["##....##", "###..###", ".######.", "..####..", "...##...", "...##...", "...##...", "...##...", "...##..."];
    let rows: &[&str] = match face {
        Face::A => &A,
        Face::B => &B,
        Face::X => &X,
        Face::Y => &Y,
    };
    // Dots are 0.8 of the pitch across, as printed; the pitch gives the letter about half the button's height.
    const PITCH: f32 = 1.6;
    let (cols, height) = (rows[0].len(), rows.len());
    let (x0, y0) = (12.0 - (cols - 1) as f32 * PITCH / 2.0, 12.0 - (height - 1) as f32 * PITCH / 2.0);
    let mut s = Vec::new();
    for (y, row) in rows.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            if c == b'#' {
                s.push(Shape::Dot(x0 + x as f32 * PITCH, y0 + y as f32 * PITCH, PITCH * 0.42));
            }
        }
    }
    s
}

fn playstation(face: Face) -> Vec<Shape> {
    use Shape::Line;
    match face {
        Face::A => vec![Line(7.8, 7.8, 16.2, 16.2), Line(16.2, 7.8, 7.8, 16.2)],
        Face::B => vec![Shape::Ring(12.0, 12.0, 4.6, STROKE)],
        Face::X => vec![Line(7.6, 7.6, 16.4, 7.6), Line(16.4, 7.6, 16.4, 16.4), Line(16.4, 16.4, 7.6, 16.4), Line(7.6, 16.4, 7.6, 7.6)],
        Face::Y => vec![Line(12.0, 6.6, 17.0, 15.2), Line(17.0, 15.2, 7.0, 15.2), Line(7.0, 15.2, 12.0, 6.6)],
    }
}

/// The shapes of `symbol` and the half-width of its lines.
fn shapes(symbol: Symbol) -> (Vec<Shape>, f32) {
    use Shape::*;
    let fixed: &[Shape] = match symbol {
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
        Symbol::Face { kind, face, ring } => {
            let (mut s, half) = match kind {
                PadKind::GameSir => (gamesir(face), 0.8),
                PadKind::Leadjoy => (leadjoy(face), STROKE),
                PadKind::PlayStation | PadKind::Xbox => (playstation(face), STROKE),
            };
            if ring {
                // Inside the button's edge, which is the 12-unit radius.
                s.push(Ring(12.0, 12.0, 11.4, 0.6));
            }
            return (s, half);
        }
    };
    (fixed.to_vec(), STROKE)
}
pub fn draw(layer: &mut ImageLayer, symbol: Symbol, colour: Rgba) {
    let (shapes, half) = shapes(symbol);
    let w = layer.width();
    let h = layer.height();
    let scale = w.min(h) as f32 / 24.0;
    for y in 0..h {
        for x in 0..w {
            let px = (x as f32 + 0.5) / scale;
            let py = (y as f32 + 0.5) / scale;
            let mut distance = f32::MAX;
            for shape in &shapes {
                let d = match *shape {
                    Shape::Line(a, b, c, d) => segment(px, py, a, b, c, d) - half,
                    Shape::Dot(a, b, r) => (px - a).hypot(py - b) - r,
                    Shape::Ring(a, b, r, half) => ((px - a).hypot(py - b) - r).abs() - half,
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
