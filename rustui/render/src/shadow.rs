//! The soft shadows around a glass surface, as geometry.
//!
//! A shadow (or a raised control's light and shade copies) is a blurred, moved copy of the body's rounded
//! rectangle, seen only outside the body. It varies slowly: over a blur width, tens of pixels for Lucent's. Working it
//! out for every pixel it covers costs the Mali about nine cycles a pixel, and Lucent's shadows cover more than the
//! screen (`shadow.rest` reaches 84 pixels below a card): measured, three cards took 7.9 ms of the frame. So it is
//! worked out only at the corners of a mesh of rings around the body, and the GPU interpolates between them, one
//! cycle a pixel.
//!
//! The mesh is a single triangle strip in layer pixels. Its innermost ring is the body's outline, so the shadow is
//! cut off exactly where the body begins and never lies under it; its outermost ring is where the shadow has faded
//! to nothing. Rings are spaced and the outline sampled finely enough, in proportion to the blur, that the
//! interpolated value is within a percent of the exact one. Each vertex carries the coverage of up to two fields (two
//! shadows, or the light and the shade of a raised control), which the fragment shader tints.

use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;
use std::time::Instant;

/// A vertex: position in layer pixels, then the coverage of field 0 and of field 1. The same 16 bytes as every
/// vertex the compositor draws.
pub type Vertex = [f32; 4];

/// The narrowest soft edge there is: a hard shadow is still an anti-aliased one pixel wide.
const MIN_WIDTH: f32 = 1.0;
/// A ring, and a step along the outline, are at most this fraction of the soft edge's width apart.
const STEP_OF_WIDTH: f32 = 0.25;
/// Bounds on the mesh's fineness, so that a very wide blur or a very long outline cannot make a huge mesh.
const MAX_RINGS: usize = 24;
const MAX_STEPS_PER_SIDE: usize = 96;
const MIN_STEP: f32 = 2.0;
/// A step along the outline is this many of the steps between rings. A shadow changes across its rings on the scale of
/// its blur, but along its outline only slowly, and only near the corners: meshes this much coarser that way are as
/// good (within five percent, see the tests) with under half the vertices, which the GPU pays for in every frame.
const ALONG_STEPS: f32 = 3.0;

/// One moved and blurred copy of the body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    /// How far the copy is moved, in layer pixels.
    pub shift: [f32; 2],
    /// The half width of the soft edge: a CSS `blur` of `b` is a Gaussian of standard deviation `b / 2`, whose edge
    /// this smoothstep of width `b` stands in for.
    pub width: f32,
    /// The corner radius of the blurred copy: a blur rounds corners.
    pub radius: f32,
}

/// The distance from `p` (relative to the centre) to the edge of a rounded rectangle; negative inside.
pub fn rounded_box(p: [f32; 2], half: [f32; 2], radius: f32) -> f32 {
    let q = [p[0].abs() - half[0] + radius, p[1].abs() - half[1] + radius];
    let outside = (q[0].max(0.0).powi(2) + q[1].max(0.0).powi(2)).sqrt();
    outside + q[0].max(q[1]).min(0.0) - radius
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 1 inside a shape whose edge is `d` away, falling to 0 over `width` on either side of the edge.
pub fn soft_edge(d: f32, width: f32) -> f32 {
    let w = width.max(MIN_WIDTH);
    1.0 - smoothstep(-w, w, d)
}

/// How much of `field`'s copy of a `size` body covers the point `local` (layer pixels from the upper-left corner).
pub fn coverage(local: [f32; 2], size: (f32, f32), field: &Field) -> f32 {
    let half = [size.0 / 2.0, size.1 / 2.0];
    let p = [
        local[0] - half[0] - field.shift[0],
        local[1] - half[1] - field.shift[1],
    ];
    soft_edge(rounded_box(p, half, field.radius), field.width)
}

/// What to build the mesh of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    pub size: (f32, f32),
    /// The body's corner radius (already limited to half its shorter side).
    pub radius: f32,
    /// How far out the mesh reaches, in layer pixels: left, top, right, bottom.
    pub reach: [f32; 4],
    pub fields: [Option<Field>; 2],
}

/// A place on the body's outline that every ring has a vertex for.
#[derive(Clone, Copy)]
enum Anchor {
    /// Along a side, from its first end (`s` = 0) to its second (`s` = 1): top left to right, right top to bottom,
    /// bottom right to left, left bottom to top.
    Top(f32),
    Right(f32),
    Bottom(f32),
    Left(f32),
    /// Round a corner (0 upper right, 1 lower right, 2 lower left, 3 upper left): `t` from 0 to 1 is a quarter turn.
    Corner(usize, f32),
}

/// The anchors along a straight side of `length` (without its far end, which the next arc begins with). A shadow
/// only varies along a side near its ends, within `dense` of a corner, where the moved and blurred copy has its own
/// corner; in between it is constant and two anchors describe it exactly.
fn side(length: f32, step: f32, dense: f32, make: fn(f32) -> Anchor) -> Vec<Anchor> {
    let length = length.max(0.0);
    let per_end = ((dense.min(length / 2.0) / step).ceil() as usize).clamp(1, MAX_STEPS_PER_SIDE);
    let mut positions: Vec<f32> = Vec::new();
    if length < step {
        positions.push(0.0);
    } else {
        positions.extend((0..=per_end).map(|k| (k as f32 * step / length).min(1.0)));
        positions.extend((1..=per_end).map(|k| (1.0 - k as f32 * step / length).max(0.0)));
        positions.sort_by(f32::total_cmp);
        positions.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        positions.retain(|&s| s < 1.0 - 1e-4);
    }
    positions.into_iter().map(make).collect()
}

/// The anchors of the whole outline, clockwise from the upper end of the top side's straight part.
fn anchors(spec: &Spec, step: f32) -> Vec<Anchor> {
    let (w, h, r) = (spec.size.0, spec.size.1, spec.radius);
    // The outermost ring's arcs are the longest, and they decide how many pieces a quarter turn is cut into.
    let outer = r + spec.reach.iter().copied().fold(0.0, f32::max);
    let arc = ((FRAC_PI_2 * outer.max(1.0) / step).ceil() as usize).clamp(3, 24);
    let dense = spec
        .fields
        .iter()
        .flatten()
        .map(|f| f.shift[0].abs().max(f.shift[1].abs()) + f.radius + f.width)
        .fold(0.0, f32::max);
    let corner = |c: usize| (0..arc).map(move |k| Anchor::Corner(c, k as f32 / arc as f32));
    let mut out = side(w - 2.0 * r, step, dense, Anchor::Top);
    out.extend(corner(0));
    out.extend(side(h - 2.0 * r, step, dense, Anchor::Right));
    out.extend(corner(1));
    out.extend(side(w - 2.0 * r, step, dense, Anchor::Bottom));
    out.extend(corner(2));
    out.extend(side(h - 2.0 * r, step, dense, Anchor::Left));
    out.extend(corner(3));
    out
}

/// How far round its corner's arc centre (the quarter turn being the unit) the anchor `t` of the way round corner `corner` is,
/// as an angle in screen coordinates (y down).
fn corner_angle(corner: usize, t: f32) -> f32 {
    (corner as f32 + t) * FRAC_PI_2 - FRAC_PI_2
}

/// The cosine and the sine of an anchor's angle round its corner: the same on every ring, so worked out once. (Zeroes for an
/// anchor along a side, which needs none.)
fn turn_of(anchor: Anchor) -> [f32; 2] {
    match anchor {
        Anchor::Corner(corner, t) => {
            let angle = corner_angle(corner, t);
            [angle.cos(), angle.sin()]
        }
        _ => [0.0; 2],
    }
}

/// Where `anchor` is on the ring that is `f` (0..=1) of the way out to the mesh's reach. A ring is the outline of
/// the body, pushed out by a fraction of the reach on each side, with elliptical arcs at the corners. `turn` is
/// [`turn_of`] the anchor.
fn place(spec: &Spec, anchor: Anchor, turn: [f32; 2], f: f32) -> [f32; 2] {
    let (w, h, r) = (spec.size.0, spec.size.1, spec.radius);
    let [left, top, right, bottom] = spec.reach.map(|reach| reach * f);
    let lerp = |a: f32, b: f32, s: f32| a + (b - a) * s;
    match anchor {
        Anchor::Top(s) => [lerp(r, w - r, s), -top],
        Anchor::Right(s) => [w + right, lerp(r, h - r, s)],
        Anchor::Bottom(s) => [lerp(w - r, r, s), h + bottom],
        Anchor::Left(s) => [-left, lerp(h - r, r, s)],
        Anchor::Corner(corner, _) => {
            // Angles measured from the corner's arc centre, in screen coordinates (y down).
            let centre = match corner {
                0 => [w - r, r],
                1 => [w - r, h - r],
                2 => [r, h - r],
                _ => [r, r],
            };
            let (reach_x, reach_y) = match corner {
                0 => (right, top),
                1 => (right, bottom),
                2 => (left, bottom),
                _ => (left, top),
            };
            [
                centre[0] + (r + reach_x) * turn[0],
                centre[1] + (r + reach_y) * turn[1],
            ]
        }
    }
}

/// The vertex of `anchor` on the ring `f` of the way out: where it is, and how much of each field covers that place.
fn vertex(spec: &Spec, anchor: Anchor, turn: [f32; 2], f: f32) -> Vertex {
    let at = place(spec, anchor, turn, f);
    let mut cover = [0.0f32; 2];
    for (out, field) in cover.iter_mut().zip(&spec.fields) {
        if let Some(field) = field {
            *out = coverage(at, spec.size, field);
        }
    }
    [at[0], at[1], cover[0], cover[1]]
}

/// How many rings the mesh has and which places of the outline each has a vertex for; `None` for a spec with no field, which
/// has nothing to draw.
fn layout(spec: &Spec) -> Option<(usize, Vec<Anchor>)> {
    let finest = spec
        .fields
        .iter()
        .flatten()
        .map(|f| f.width.max(MIN_WIDTH))
        .fold(f32::MAX, f32::min);
    if finest == f32::MAX {
        return None;
    }
    let step = (finest * STEP_OF_WIDTH).max(MIN_STEP);
    let widest = spec.reach.iter().copied().fold(0.0, f32::max);
    let rings = ((widest / step).ceil() as usize).clamp(1, MAX_RINGS);
    Some((rings, anchors(spec, step * ALONG_STEPS)))
}

/// The mesh, as a triangle strip: rings one after another, joined by degenerate triangles. Each ring is worked out once (it is
/// the outer edge of one strip and the inner edge of the next), and each corner's angle once for all of them: on the Show a mesh
/// took 100 to 190 us, and a screen's first frame makes a pair for every glass surface it has.
pub fn build(spec: &Spec) -> Vec<Vertex> {
    let Some((rings, anchors)) = layout(spec) else {
        return Vec::new();
    };
    let turns: Vec<[f32; 2]> = anchors.iter().map(|&anchor| turn_of(anchor)).collect();
    let ring_at = |ring: usize| -> Vec<Vertex> {
        let f = ring as f32 / rings as f32;
        anchors
            .iter()
            .zip(&turns)
            .map(|(&anchor, &turn)| vertex(spec, anchor, turn, f))
            .collect()
    };
    let n = anchors.len();
    let mut out = Vec::with_capacity(rings * (2 * n + 4));
    let mut inner = ring_at(0);
    for ring in 0..rings {
        let outer = ring_at(ring + 1);
        if let Some(&last) = out.last() {
            out.push(last);
            out.push(inner[0]);
        }
        // Round once, and back to the start to close the loop.
        for k in 0..=n {
            out.push(inner[k % n]);
            out.push(outer[k % n]);
        }
        inner = outer;
    }
    out
}

/// A mesh as every surface of its shape holds it.
pub type Shared = Arc<[Vertex]>;

/// Every number a mesh is made of, as bits: two specs are the same mesh when these are equal.
type Key = [u32; 17];

fn key_of(spec: &Spec) -> Key {
    let mut key = [0u32; 17];
    key[0] = spec.size.0.to_bits();
    key[1] = spec.size.1.to_bits();
    key[2] = spec.radius.to_bits();
    for (slot, reach) in key[3..7].iter_mut().zip(spec.reach) {
        *slot = reach.to_bits();
    }
    for (n, field) in spec.fields.iter().enumerate() {
        if let Some(field) = field {
            let at = 7 + 5 * n;
            key[at] = 1;
            key[at + 1] = field.shift[0].to_bits();
            key[at + 2] = field.shift[1].to_bits();
            key[at + 3] = field.width.to_bits();
            key[at + 4] = field.radius.to_bits();
        }
    }
    key
}

/// How many meshes are kept once nothing holds them: a screen's shapes (a few dozen) come back with the next one's, and the
/// meshes of a surface that is being resized are not kept for ever.
const KEPT: usize = 192;

/// The meshes made so far, by what they are meshes of: the controls of a screen are a handful of shapes, so the surfaces of one
/// shape share a mesh and a screen's first frame makes one for each shape, not one for each surface.
#[derive(Default)]
pub struct Cache {
    meshes: HashMap<Key, Shared>,
    built: u32,
    built_ns: u64,
}

impl Cache {
    /// The mesh of `spec`: the one made for the same numbers before, or a new one.
    pub fn get(&mut self, spec: &Spec) -> Shared {
        let key = key_of(spec);
        if let Some(mesh) = self.meshes.get(&key) {
            return Arc::clone(mesh);
        }
        if self.meshes.len() >= KEPT {
            // What no surface holds goes; what some surface does stays whatever the count.
            self.meshes.retain(|_, mesh| Arc::strong_count(mesh) > 1);
        }
        let began = Instant::now();
        let mesh: Shared = build(spec).into();
        self.built += 1;
        self.built_ns += u64::try_from(began.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.meshes.insert(key, Arc::clone(&mesh));
        mesh
    }

    /// The meshes made since the last call and the nanoseconds they took.
    pub fn take_built(&mut self) -> (u32, u64) {
        (
            std::mem::take(&mut self.built),
            std::mem::take(&mut self.built_ns),
        )
    }

    /// How many meshes are kept.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.meshes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f32, f32) = (288.0, 150.0);

    /// Lucent's `shadow.rest` on a card: 24 down, 60 blur.
    fn rest() -> Spec {
        let field = Field {
            shift: [0.0, 24.0],
            width: 60.0,
            radius: 24.0f32.hypot(30.0),
        };
        Spec {
            size: SIZE,
            radius: 24.0,
            reach: [60.0, 36.0, 60.0, 84.0],
            fields: [Some(field), None],
        }
    }

    #[test]
    fn a_soft_edge_is_half_at_the_edge_whole_inside_and_gone_a_width_outside() {
        assert!((soft_edge(0.0, 20.0) - 0.5).abs() < 1e-6);
        assert_eq!(soft_edge(-20.0, 20.0), 1.0);
        assert_eq!(soft_edge(-50.0, 20.0), 1.0);
        assert_eq!(soft_edge(20.0, 20.0), 0.0);
        assert_eq!(soft_edge(200.0, 20.0), 0.0);
        let mut last = 1.0;
        for step in -20..=20 {
            let now = soft_edge(step as f32, 20.0);
            assert!(now <= last + 1e-6, "falls steadily");
            last = now;
        }
        // No blur is a one pixel anti-aliased edge, never a division by nothing.
        assert!(soft_edge(0.0, 0.0).is_finite());
        assert_eq!(soft_edge(2.0, 0.0), 0.0);
        assert_eq!(soft_edge(-2.0, 0.0), 1.0);
    }

    #[test]
    fn a_copy_of_the_body_is_covered_where_the_moved_body_is() {
        let field = Field {
            shift: [0.0, 24.0],
            width: 10.0,
            radius: 24.0,
        };
        // The centre of the moved body, well inside, is whole; the point the same distance above the top edge of
        // the moved body is nothing; the moved body's own edge is half.
        assert_eq!(coverage([144.0, 75.0 + 24.0], SIZE, &field), 1.0);
        assert_eq!(coverage([144.0, 24.0 - 30.0], SIZE, &field), 0.0);
        assert!((coverage([144.0, 24.0], SIZE, &field) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_innermost_ring_is_the_bodys_outline_and_the_outermost_has_faded_out() {
        let spec = rest();
        let mesh = build(&spec);
        let half = [SIZE.0 / 2.0, SIZE.1 / 2.0];
        let mut on_outline = 0;
        let mut faded = 0;
        for v in &mesh {
            let d = rounded_box([v[0] - half[0], v[1] - half[1]], half, spec.radius);
            // Vertices of the first ring sit on the outline (within the flattening of the arcs).
            if d.abs() < 0.3 {
                on_outline += 1;
            }
            // Those of the last ring are a whole reach out and carry no shadow at all.
            if v[2] < 1e-3 {
                faded += 1;
            }
            assert!(v.iter().all(|c| c.is_finite()));
        }
        assert!(on_outline > 20, "{on_outline} vertices on the outline");
        assert!(faded > 20, "{faded} vertices with no shadow");
        assert!(mesh.iter().all(|v| v[2] >= 0.0 && v[2] <= 1.0));
    }

    #[test]
    fn nothing_is_drawn_past_the_reach() {
        let spec = rest();
        for v in build(&spec) {
            let [left, top, right, bottom] = spec.reach;
            assert!(
                v[0] >= -left - 1e-3 && v[0] <= SIZE.0 + right + 1e-3,
                "{v:?}"
            );
            assert!(
                v[1] >= -top - 1e-3 && v[1] <= SIZE.1 + bottom + 1e-3,
                "{v:?}"
            );
        }
    }

    /// The two coverages the GPU would interpolate at `p`: found by looking for the triangle of the strip it lies in.
    fn interpolated(mesh: &[Vertex], p: [f32; 2]) -> Option<[f32; 2]> {
        for w in mesh.windows(3) {
            let (a, b, c) = (w[0], w[1], w[2]);
            let det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
            if det.abs() < 1e-6 {
                continue;
            }
            let l1 = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / det;
            let l2 = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / det;
            let l3 = 1.0 - l1 - l2;
            if l1 >= -1e-5 && l2 >= -1e-5 && l3 >= -1e-5 {
                return Some([
                    l1 * a[2] + l2 * b[2] + l3 * c[2],
                    l1 * a[3] + l2 * b[3] + l3 * c[3],
                ]);
            }
        }
        None
    }

    #[test]
    fn the_mesh_reproduces_the_shadow_within_five_percent_wherever_it_covers() {
        // Lucent's two biggest shadows on a card, and a control's small raised copies.
        let raised = Spec {
            size: SIZE,
            radius: 24.0,
            reach: [80.0, 46.0, 80.0, 114.0],
            fields: [
                Some(Field {
                    shift: [0.0, 34.0],
                    width: 80.0,
                    radius: 24.0f32.hypot(40.0),
                }),
                Some(Field {
                    shift: [0.0, 10.0],
                    width: 24.0,
                    radius: 24.0f32.hypot(12.0),
                }),
            ],
        };
        let small = Spec {
            size: (96.0, 96.0),
            radius: 20.0,
            reach: [6.0; 4],
            fields: [
                Some(Field {
                    shift: [-2.0, -2.0],
                    width: 4.0,
                    radius: 20.0f32.hypot(2.0),
                }),
                Some(Field {
                    shift: [2.0, 2.0],
                    width: 4.0,
                    radius: 20.0f32.hypot(2.0),
                }),
            ],
        };
        for spec in [rest(), raised, small] {
            let mesh = build(&spec);
            let [left, top, right, bottom] = spec.reach;
            let half = [spec.size.0 / 2.0, spec.size.1 / 2.0];
            let mut worst = 0.0f32;
            let mut looked = 0;
            for i in 0..60 {
                for j in 0..40 {
                    let p = [
                        -left + (spec.size.0 + left + right) * (i as f32 + 0.5) / 60.0,
                        -top + (spec.size.1 + top + bottom) * (j as f32 + 0.5) / 40.0,
                    ];
                    // Only outside the body: inside it the mesh does not draw.
                    if rounded_box([p[0] - half[0], p[1] - half[1]], half, spec.radius) < 0.5 {
                        continue;
                    }
                    let Some(got) = interpolated(&mesh, p) else {
                        continue;
                    };
                    for (channel, field) in spec.fields.iter().enumerate() {
                        let want = field.map_or(0.0, |f| coverage(p, spec.size, &f));
                        worst = worst.max((got[channel] - want).abs());
                    }
                    looked += 1;
                }
            }
            assert!(looked > 300, "{looked} points looked at");
            // Of a shadow that is at most half opaque: a few levels of 255 at the very worst point.
            assert!(worst < 0.05, "worst error {worst} for {spec:?}");
        }
    }

    #[test]
    fn the_mesh_stays_small_however_soft_or_large_the_shadow() {
        let huge = Spec {
            size: (960.0, 480.0),
            radius: 28.0,
            reach: [128.0, 96.0, 128.0, 160.0],
            fields: [
                Some(Field {
                    shift: [0.0, 32.0],
                    width: 128.0,
                    radius: 28.0f32.hypot(64.0),
                }),
                None,
            ],
        };
        assert!(build(&huge).len() < 4000);
        assert!(build(&rest()).len() < 700);
        // A control's raised copies, of 4 px of blur: a few hundred, not a thousand and more.
        let control = Spec {
            size: (96.0, 96.0),
            radius: 20.0,
            reach: [6.0; 4],
            fields: [
                Some(Field {
                    shift: [-2.0, -2.0],
                    width: 4.0,
                    radius: 20.0f32.hypot(2.0),
                }),
                Some(Field {
                    shift: [2.0, 2.0],
                    width: 4.0,
                    radius: 20.0f32.hypot(2.0),
                }),
            ],
        };
        assert!(build(&control).len() < 600, "{}", build(&control).len());
        // Nothing to draw, nothing built.
        let none = Spec {
            fields: [None, None],
            ..rest()
        };
        assert!(build(&none).is_empty());
    }

    #[test]
    fn a_fully_round_body_and_a_square_one_both_make_a_closed_mesh() {
        for (size, radius) in [
            ((60.0, 60.0), 30.0),
            ((100.0, 40.0), 20.0),
            ((80.0, 80.0), 0.0),
        ] {
            let spec = Spec {
                size,
                radius,
                reach: [10.0; 4],
                fields: [
                    Some(Field {
                        shift: [0.0, 4.0],
                        width: 10.0,
                        radius: radius.hypot(5.0).min(size.0.min(size.1) / 2.0),
                    }),
                    None,
                ],
            };
            let mesh = build(&spec);
            assert!(mesh.len() > 12, "{size:?} {radius}");
            assert!(mesh.iter().all(|v| v.iter().all(|c| c.is_finite())));
        }
    }

    /// The mesh as it was first written: every vertex of every strip worked out on its own, the angle of a corner again for each.
    fn plainly_built(spec: &Spec) -> Vec<Vertex> {
        let Some((rings, anchors)) = layout(spec) else {
            return Vec::new();
        };
        let at = |anchor: Anchor, ring: usize| {
            vertex(spec, anchor, turn_of(anchor), ring as f32 / rings as f32)
        };
        let mut out = Vec::new();
        for ring in 0..rings {
            let mut strip = Vec::new();
            for k in 0..=anchors.len() {
                let anchor = anchors[k % anchors.len()];
                strip.push(at(anchor, ring));
                strip.push(at(anchor, ring + 1));
            }
            if let (Some(&last), Some(&first)) = (out.last(), strip.first()) {
                out.push(last);
                out.push(first);
            }
            out.extend(strip);
        }
        out
    }

    /// A card's, a control's and a pill's shadows, a raised pair and a huge soft one: the shapes the screens are made of.
    fn shapes() -> Vec<Spec> {
        let field = |shift: [f32; 2], width: f32, radius: f32| {
            Some(Field {
                shift,
                width,
                radius,
            })
        };
        let cast = |size: (f32, f32), radius: f32, offset_y: f32, blur: f32| Spec {
            size,
            radius,
            reach: [blur, blur - offset_y, blur, blur + offset_y],
            fields: [
                field(
                    [0.0, offset_y],
                    blur,
                    radius.hypot(blur / 2.0).min(size.0.min(size.1) / 2.0),
                ),
                None,
            ],
        };
        let pair = |size: (f32, f32), radius: f32, offset: f32, blur: f32| {
            let radius_of = radius.hypot(blur / 2.0).min(size.0.min(size.1) / 2.0);
            Spec {
                size,
                radius,
                reach: [offset + blur; 4],
                fields: [
                    field([-offset, -offset], blur, radius_of),
                    field([offset, offset], blur, radius_of),
                ],
            }
        };
        vec![
            rest(),
            cast((400.0, 200.0), 24.0, 10.0, 24.0),
            cast((880.0, 64.0), 16.0, 24.0, 60.0),
            cast((160.0, 56.0), 28.0, 24.0, 60.0),
            cast((960.0, 480.0), 28.0, 32.0, 128.0),
            cast((60.0, 60.0), 30.0, 4.0, 10.0),
            cast((80.0, 80.0), 0.0, 4.0, 10.0),
            pair((120.0, 48.0), 24.0, 2.0, 4.0),
            pair((400.0, 200.0), 24.0, 2.0, 4.0),
            pair((96.0, 96.0), 20.0, 2.0, 4.0),
        ]
    }

    #[test]
    fn the_mesh_is_to_the_bit_what_working_out_every_vertex_of_every_strip_on_its_own_makes() {
        for spec in shapes() {
            let mesh = build(&spec);
            assert!(mesh.len() > 12, "{spec:?}");
            assert_eq!(mesh, plainly_built(&spec), "{spec:?}");
        }
    }

    #[test]
    fn surfaces_of_one_shape_share_a_mesh_and_each_other_shape_makes_its_own() {
        let mut cache = Cache::default();
        let specs = shapes();
        let first = cache.get(&specs[0]);
        let again = cache.get(&specs[0]);
        assert!(Arc::ptr_eq(&first, &again), "the same shape, the same mesh");
        assert_eq!(&first[..], &build(&specs[0])[..]);
        assert_eq!(cache.take_built().0, 1, "made once for two surfaces");
        for spec in &specs[1..] {
            cache.get(spec);
        }
        assert_eq!(cache.take_built().0 as usize, specs.len() - 1);
        // A shape that differs by anything a mesh is made of is another mesh: a size a pixel wider, a blur, a shift.
        let wider = Spec {
            size: (specs[0].size.0 + 1.0, specs[0].size.1),
            ..specs[0]
        };
        let mut other_field = specs[0];
        if let Some(field) = &mut other_field.fields[0] {
            field.shift[1] += 1.0;
        }
        let (a, b) = (cache.get(&wider), cache.get(&other_field));
        assert!(!Arc::ptr_eq(&a, &first) && !Arc::ptr_eq(&b, &first) && !Arc::ptr_eq(&a, &b));
        assert_eq!(cache.take_built().0, 2);
    }

    #[test]
    fn meshes_nothing_holds_go_when_the_cache_is_full_and_a_held_one_stays() {
        let mut cache = Cache::default();
        let spec = |n: usize| Spec {
            size: (100.0 + n as f32, 40.0),
            ..rest()
        };
        let held: Vec<Shared> = (0..3).map(|n| cache.get(&spec(n))).collect();
        // A surface that is being resized makes a new shape every frame: none of them is held for long.
        for n in 3..3 * KEPT {
            cache.get(&spec(n));
            assert!(cache.len() <= KEPT, "{} meshes after {n}", cache.len());
        }
        cache.take_built();
        for (n, mesh) in held.iter().enumerate() {
            assert!(Arc::ptr_eq(mesh, &cache.get(&spec(n))), "shape {n} stayed");
        }
        assert_eq!(cache.take_built().0, 0, "and was not made again");
    }
}
