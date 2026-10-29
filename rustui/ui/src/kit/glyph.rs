//! The remote-key hint glyphs (`assets/glyphs`, GlyphHints 2026-10-29) and the kit's own marks, rastered on the CPU into
//! premultiplied pixels: the same strokes as the SVGs (a 24-unit box, 1.9-unit round strokes, rings at half ink), so a hint on
//! the TV is the glyph of the Magic Remote's key and never the words "OK" or "Back" (Lucent section 4). A glyph is drawn once per
//! size and colour into an image layer and kept.

use compositor_protocol::Rgba;
use std::f32::consts::PI;

/// A stroke or a fill in the 24-unit box.
#[derive(Clone, Debug, PartialEq)]
enum Prim {
    /// A polyline with round caps and joins, `width` units wide, at `alpha` of the ink.
    Stroke { pts: Vec<(f32, f32)>, width: f32, alpha: f32, closed: bool },
    /// A ring (`width` > 0) or a disc (`width` 0).
    Circle { c: (f32, f32), r: f32, width: f32, alpha: f32 },
}

const W: f32 = 1.9;

fn stroke(pts: &[(f32, f32)]) -> Prim {
    Prim::Stroke { pts: pts.to_vec(), width: W, alpha: 1.0, closed: false }
}

fn closed(pts: &[(f32, f32)]) -> Prim {
    Prim::Stroke { pts: pts.to_vec(), width: W, alpha: 1.0, closed: true }
}

fn ring(r: f32, alpha: f32) -> Prim {
    Prim::Circle { c: (12.0, 12.0), r, width: W, alpha }
}

fn disc(c: (f32, f32), r: f32) -> Prim {
    Prim::Circle { c, r, width: 0.0, alpha: 1.0 }
}

/// Points along an arc of `r` about `c` from `a0` to `a1` degrees (clockwise on the screen as the angle grows).
fn arc(c: (f32, f32), r: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
    let n = (((a1 - a0).abs() / 15.0).ceil() as usize).max(2);
    (0..=n)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / n as f32) * PI / 180.0;
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect()
}

/// The chevron of the arrow keys, pointing right; `rot` degrees turn it.
fn chevron(rot: f32) -> Prim {
    let pts = [(10.0, 8.0), (14.0, 12.0), (10.0, 16.0)];
    let (s, co) = ((rot * PI / 180.0).sin(), (rot * PI / 180.0).cos());
    let turned: Vec<(f32, f32)> = pts
        .iter()
        .map(|&(x, y)| {
            let (dx, dy) = (x - 12.0, y - 12.0);
            (12.0 + dx * co - dy * s, 12.0 + dx * s + dy * co)
        })
        .collect();
    stroke(&turned)
}

/// The strokes of a glyph by its name in `assets/glyphs/index.json`, plus the kit's own `volume-x` and `info`; `None` for a name
/// nobody drew.
fn shape(name: &str) -> Option<Vec<Prim>> {
    let colour_key = || vec![Prim::Stroke { pts: vec![(7.0, 12.0), (17.0, 12.0)], width: 6.0, alpha: 1.0, closed: false }];
    Some(match name {
        "key-ok" => vec![ring(9.5, 0.5), disc((12.0, 12.0), 4.2)],
        "key-ok-hold" => {
            let mut v = vec![ring(6.2, 0.5), disc((12.0, 12.0), 2.8)];
            v.push(stroke(&arc((12.0, 12.0), 9.5, -90.0, 180.0)));
            v
        }
        "key-up" => vec![ring(9.5, 0.5), chevron(270.0)],
        "key-down" => vec![ring(9.5, 0.5), chevron(90.0)],
        "key-left" => vec![ring(9.5, 0.5), chevron(180.0)],
        "key-right" => vec![ring(9.5, 0.5), chevron(0.0)],
        "key-back" => {
            let mut pts = vec![(4.0, 9.0), (14.5, 9.0)];
            pts.extend(arc((14.5, 14.5), 5.5, -90.0, 90.0));
            pts.push((11.0, 20.0));
            vec![stroke(&[(9.0, 14.0), (4.0, 9.0), (9.0, 4.0)]), stroke(&pts)]
        }
        "key-home" => vec![
            closed(&[(3.0, 10.0), (3.7, 8.5), (10.7, 2.5), (13.3, 2.5), (20.3, 8.5), (21.0, 10.0), (21.0, 19.0), (19.0, 21.0), (5.0, 21.0), (3.0, 19.0)]),
            stroke(&[(15.0, 21.0), (15.0, 13.0), (14.0, 12.0), (10.0, 12.0), (9.0, 13.0), (9.0, 21.0)]),
        ],
        "key-menu" => vec![stroke(&[(4.0, 7.0), (20.0, 7.0)]), stroke(&[(4.0, 12.0), (20.0, 12.0)]), stroke(&[(4.0, 17.0), (20.0, 17.0)])],
        "key-mic" => {
            let mut bowl = vec![(19.0, 10.0), (19.0, 12.0)];
            bowl.extend(arc((12.0, 12.0), 7.0, 0.0, 180.0));
            bowl.push((5.0, 10.0));
            let mut body = vec![(9.0, 12.0), (9.0, 5.0)];
            body.extend(arc((12.0, 5.0), 3.0, 180.0, 360.0));
            body.push((15.0, 12.0));
            body.extend(arc((12.0, 12.0), 3.0, 0.0, 180.0));
            vec![stroke(&[(12.0, 19.0), (12.0, 22.0)]), stroke(&bowl), closed(&body)]
        }
        "key-red" | "key-green" | "key-yellow" | "key-blue" => colour_key(),
        "expand" => vec![
            stroke(&[(15.0, 3.0), (21.0, 3.0), (21.0, 9.0)]),
            stroke(&[(9.0, 21.0), (3.0, 21.0), (3.0, 15.0)]),
            stroke(&[(21.0, 3.0), (14.0, 10.0)]),
            stroke(&[(3.0, 21.0), (10.0, 14.0)]),
        ],
        "collapse" => vec![
            stroke(&[(14.0, 10.0), (21.0, 3.0)]),
            stroke(&[(20.0, 10.0), (14.0, 10.0), (14.0, 4.0)]),
            stroke(&[(3.0, 21.0), (10.0, 14.0)]),
            stroke(&[(4.0, 14.0), (10.0, 14.0), (10.0, 20.0)]),
        ],
        "close" => vec![stroke(&[(18.0, 6.0), (6.0, 18.0)]), stroke(&[(6.0, 6.0), (18.0, 18.0)])],
        "open" => vec![closed(&[(5.0, 5.0), (5.0, 19.0), (20.0, 12.0)])],
        "confirm" => vec![stroke(&[(20.0, 6.0), (9.0, 17.0), (4.0, 12.0)])],
        "options" => vec![disc((5.0, 12.0), 1.95), disc((12.0, 12.0), 1.95), disc((19.0, 12.0), 1.95)],
        "move" => vec![
            stroke(&[(12.0, 2.0), (12.0, 22.0)]),
            stroke(&[(2.0, 12.0), (22.0, 12.0)]),
            stroke(&[(15.0, 19.0), (12.0, 22.0), (9.0, 19.0)]),
            stroke(&[(9.0, 5.0), (12.0, 2.0), (15.0, 5.0)]),
            stroke(&[(19.0, 9.0), (22.0, 12.0), (19.0, 15.0)]),
            stroke(&[(5.0, 9.0), (2.0, 12.0), (5.0, 15.0)]),
        ],
        "edit" => vec![closed(&[(18.5, 2.5), (21.5, 5.5), (7.3, 19.7), (2.5, 21.5), (4.3, 16.7)]), stroke(&[(15.0, 5.0), (19.0, 9.0)])],
        "hold" => vec![
            stroke(&[(5.0, 22.0), (19.0, 22.0)]),
            stroke(&[(5.0, 2.0), (19.0, 2.0)]),
            stroke(&[(17.0, 22.0), (17.0, 17.8), (12.0, 12.0), (7.0, 17.8), (7.0, 22.0)]),
            stroke(&[(7.0, 2.0), (7.0, 6.2), (12.0, 12.0), (17.0, 6.2), (17.0, 2.0)]),
        ],
        // the kit's own: the muted speaker of the volume display and the toasts' info mark (Lucide's)
        "volume-x" => vec![
            closed(&[(11.0, 5.0), (6.0, 9.0), (2.0, 9.0), (2.0, 15.0), (6.0, 15.0), (11.0, 19.0)]),
            stroke(&[(22.0, 9.0), (16.0, 15.0)]),
            stroke(&[(16.0, 9.0), (22.0, 15.0)]),
        ],
        "volume" => {
            let mut v = vec![closed(&[(11.0, 5.0), (6.0, 9.0), (2.0, 9.0), (2.0, 15.0), (6.0, 15.0), (11.0, 19.0)])];
            v.push(stroke(&arc((11.0, 12.0), 4.5, -50.0, 50.0)));
            v.push(stroke(&arc((11.0, 12.0), 8.5, -50.0, 50.0)));
            v
        }
        "info" => vec![ring(10.0, 1.0), stroke(&[(12.0, 16.0), (12.0, 12.0)]), stroke(&[(12.0, 8.0), (12.0, 8.01)])],
        // a filled disc half the box wide (the live dot of a "Live" button: 12 px in a 24 px box)
        "dot" => vec![disc((12.0, 12.0), 6.0)],
        _ => return None,
    })
}

/// Whether the kit can draw `name`.
pub fn known(name: &str) -> bool {
    shape(name).is_some()
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let len2 = abx * abx + aby * aby;
    let t = if len2 <= 0.0 { 0.0 } else { (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / len2).clamp(0.0, 1.0) };
    let (cx, cy) = (a.0 + abx * t, a.1 + aby * t);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// The coverage (0 to 1) of one primitive at `p` (in units), with a one-pixel anti-aliasing ramp (`px` units per pixel).
fn coverage(prim: &Prim, p: (f32, f32), px: f32) -> f32 {
    let ramp = |inside: f32| (inside / px + 0.5).clamp(0.0, 1.0);
    match prim {
        Prim::Stroke { pts, width, alpha, closed } => {
            let mut d = f32::INFINITY;
            for i in 0..pts.len().saturating_sub(1) {
                d = d.min(segment_distance(p, pts[i], pts[i + 1]));
            }
            if *closed && pts.len() > 2 {
                d = d.min(segment_distance(p, pts[pts.len() - 1], pts[0]));
            }
            if pts.len() == 1 {
                d = segment_distance(p, pts[0], pts[0]);
            }
            alpha * ramp(width / 2.0 - d)
        }
        Prim::Circle { c, r, width, alpha } => {
            let d = ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt();
            let inside = if *width > 0.0 { width / 2.0 - (d - r).abs() } else { r - d };
            alpha * ramp(inside)
        }
    }
}

/// Draws `name` `side` pixels square in `colour` (premultiplied) into `pixels` (premultiplied RGBA, `side` wide), over what is
/// there. `false` for a name the kit does not know (nothing is drawn).
pub fn draw(name: &str, side: u32, colour: Rgba, pixels: &mut [u8]) -> bool {
    let Some(prims) = shape(name) else { return false };
    let unit = 24.0 / side as f32;
    for y in 0..side {
        for x in 0..side {
            let p = ((x as f32 + 0.5) * unit, (y as f32 + 0.5) * unit);
            let mut a = 0.0f32;
            for prim in &prims {
                let c = coverage(prim, p, unit);
                a = a + c - a * c;
            }
            if a <= 0.0 {
                continue;
            }
            let i = ((y * side + x) * 4) as usize;
            let src = |v: u8| (v as f32 * a).round() as u32;
            let (sr, sg, sb, sa) = (src(colour.0), src(colour.1), src(colour.2), src(colour.3));
            let keep = 255 - sa;
            pixels[i] = (sr + pixels[i] as u32 * keep / 255).min(255) as u8;
            pixels[i + 1] = (sg + pixels[i + 1] as u32 * keep / 255).min(255) as u8;
            pixels[i + 2] = (sb + pixels[i + 2] as u32 * keep / 255).min(255) as u8;
            pixels[i + 3] = (sa + pixels[i + 3] as u32 * keep / 255).min(255) as u8;
        }
    }
    true
}

/// The glyph as fresh premultiplied pixels.
pub fn render(name: &str, side: u32, colour: Rgba) -> Option<Vec<u8>> {
    let mut pixels = vec![0u8; (side * side * 4) as usize];
    draw(name, side, colour, &mut pixels).then_some(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: Rgba = Rgba(255, 255, 255, 255);

    fn alpha_at(pixels: &[u8], side: u32, x: u32, y: u32) -> u8 {
        pixels[((y * side + x) * 4 + 3) as usize]
    }

    #[test]
    fn every_glyph_of_the_index_is_drawn_and_marks_pixels() {
        let index: serde_json::Value = serde_json::from_str(include_str!("../../../../assets/glyphs/index.json")).unwrap();
        for (name, _) in index["glyphs"].as_object().unwrap() {
            let pixels = render(name, 38, INK).unwrap_or_else(|| panic!("{name} is not drawn"));
            let inked = pixels.chunks_exact(4).filter(|p| p[3] > 0).count();
            assert!(inked > 20, "{name}: {inked} pixels");
        }
        assert!(render("volume-x", 32, INK).is_some());
        assert!(render("nothing-of-the-kind", 32, INK).is_none());
    }

    #[test]
    fn the_ok_glyph_is_a_half_ink_ring_round_a_full_dot() {
        let side = 48;
        let p = render("key-ok", side, INK).unwrap();
        assert_eq!(alpha_at(&p, side, 24, 24), 255, "the dot is full ink");
        let ring = alpha_at(&p, side, 24, 5);
        assert!((100..=140).contains(&ring), "the ring is at half ink: {ring}");
        assert_eq!(alpha_at(&p, side, 24, 14), 0, "nothing between the ring and the dot");
    }

    #[test]
    fn the_arrow_glyphs_point_their_way() {
        let side = 48;
        let right = render("key-right", side, INK).unwrap();
        let left = render("key-left", side, INK).unwrap();
        // the right chevron's point is right of centre, the left one's left of it
        assert!(alpha_at(&right, side, 28, 24) > 200);
        assert!(alpha_at(&left, side, 20, 24) > 200);
        assert!(alpha_at(&right, side, 20, 24) < 40);
    }

    #[test]
    fn the_colour_keys_are_a_capsule_in_the_colour_given() {
        let side = 24;
        let p = render("key-red", side, Rgba(220, 40, 60, 255)).unwrap();
        assert_eq!(&p[((12 * side + 12) * 4) as usize..((12 * side + 12) * 4 + 4) as usize], &[220, 40, 60, 255]);
        assert_eq!(alpha_at(&p, side, 12, 2), 0);
    }
}
