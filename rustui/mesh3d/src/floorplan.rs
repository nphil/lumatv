//! The graphics lab's 3D test: a one-storey flat, cut away at 1.1 m the way Home Assistant's 3D floor plan cards
//! show a home. A floor slab, five rooms with their own floor colours, walls with door gaps and furniture, all
//! in Rosé Pine colours, about two thousand triangles, in metres. It is the scene the compositor's mesh layer is
//! measured with, so it is deliberately the size and kind of thing a real floor plan card would be.
//!
//! The flat spans x 0..10 and z 0..7 (north is -z, so a top-down view has the kitchen at the top right).

use compositor_protocol::{Mesh, MeshData, MeshVertex, Rgba};

/// Height the walls are cut at.
const WALL_HEIGHT: f32 = 1.1;
const WALL_THICKNESS: f32 = 0.15;
const SLAB_MARGIN: f32 = 0.35;
const SLAB_DEPTH: f32 = 0.25;

/// A room and the point where its label belongs: the middle of the room at just over wall height, so a glass
/// label projected there floats over the room.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    pub name: &'static str,
    pub anchor: [f32; 3],
}

pub const ROOMS: [Room; 5] = [
    Room {
        name: "Living room",
        anchor: [2.75, 1.4, 2.0],
    },
    Room {
        name: "Kitchen",
        anchor: [7.75, 1.4, 1.75],
    },
    Room {
        name: "Bedroom",
        anchor: [2.0, 1.4, 5.5],
    },
    Room {
        name: "Bathroom",
        anchor: [5.25, 1.4, 5.5],
    },
    Room {
        name: "Study",
        anchor: [8.25, 1.4, 5.5],
    },
];

/// Where the rooms are, in the order of [`ROOMS`]: x0, z0, x1, z1 and the floor's Rosé Pine colour.
const LAYOUT: [(f32, f32, f32, f32, u32); 5] = [
    (0.0, 0.0, 5.5, 4.0, 0x2a3346),
    (5.5, 0.0, 10.0, 3.5, 0x2f3b4a),
    (0.0, 4.0, 4.0, 7.0, 0x33293f),
    (4.0, 4.0, 6.5, 7.0, 0x26414f),
    (6.5, 4.0, 10.0, 7.0, 0x322f45),
];

/// The camera the lab looks at the flat with: from the south, 43° above the floor, the whole flat in view at
/// 1.6:1 and on every side of an orbit. Transparent behind, lit from the upper left. The app sets the orbit rate
/// and antialiasing from the tier, so both are off here.
pub const CAMERA: Mesh = Mesh {
    eye: [5.0, 9.0, 12.7],
    target: [5.0, 0.4, 3.5],
    fov_deg: 40.0,
    light: [-0.45, 0.8, 0.4],
    ambient: 0.5,
    orbit_deg_per_s: 0.0,
    antialias: false,
    background: Rgba(0, 0, 0, 0),
};

const LOVE: u32 = 0xeb6f92;
const GOLD: u32 = 0xf6c177;
const ROSE: u32 = 0xebbcba;
const PINE: u32 = 0x31748f;
const FOAM: u32 = 0x9ccfd8;
const IRIS: u32 = 0xc4a7e7;
const SURFACE: u32 = 0x1f1d2e;
const OVERLAY: u32 = 0x26233a;
const HIGHLIGHT_MED: u32 = 0x403d52;
const HIGHLIGHT_HIGH: u32 = 0x524f67;
const MUTED: u32 = 0x6e6a86;
const SUBTLE: u32 = 0x908caa;
const TEXT: u32 = 0xe0def4;

fn opaque(hex: u32) -> Rgba {
    Rgba((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255)
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        let (x, y) = (((a >> shift) & 255) as f32, ((b >> shift) & 255) as f32);
        (x + (y - x) * t).round() as u32
    };
    channel(16) << 16 | channel(8) << 8 | channel(0)
}

type Vec3 = [f32; 3];

#[derive(Default)]
struct Builder {
    vertices: Vec<MeshVertex>,
    indices: Vec<u16>,
}

impl Builder {
    /// One flat quad whose corners run anticlockwise seen from the side the normal points to.
    fn quad(&mut self, origin: Vec3, u: Vec3, v: Vec3, colour: u32) {
        let normal = normalize(cross(u, v));
        let base = self.vertices.len() as u16;
        for (a, b) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            self.vertices.push(MeshVertex {
                position: [
                    origin[0] + u[0] * a + v[0] * b,
                    origin[1] + u[1] * a + v[1] * b,
                    origin[2] + u[2] * a + v[2] * b,
                ],
                normal,
                colour: opaque(colour),
            });
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// A box from `low` to `high` without a bottom (nobody sees it), sides in `colour`, top in `top`.
    fn block(&mut self, low: Vec3, high: Vec3, colour: u32, top: u32) {
        let d = [high[0] - low[0], high[1] - low[1], high[2] - low[2]];
        self.quad(
            [high[0], low[1], low[2]],
            [0.0, d[1], 0.0],
            [0.0, 0.0, d[2]],
            colour,
        );
        self.quad(low, [0.0, 0.0, d[2]], [0.0, d[1], 0.0], colour);
        self.quad(
            [low[0], high[1], low[2]],
            [0.0, 0.0, d[2]],
            [d[0], 0.0, 0.0],
            top,
        );
        self.quad(
            [low[0], low[1], high[2]],
            [d[0], 0.0, 0.0],
            [0.0, d[1], 0.0],
            colour,
        );
        self.quad(low, [0.0, d[1], 0.0], [d[0], 0.0, 0.0], colour);
    }

    /// A cone or cylinder standing on the floor: `sides` facets, radius `r0` at `y0` and `r1` at `y1` (0 for a
    /// point), smooth sides and a flat top.
    #[allow(clippy::too_many_arguments)]
    fn frustum(
        &mut self,
        centre: [f32; 2],
        y0: f32,
        y1: f32,
        r0: f32,
        r1: f32,
        sides: u16,
        colour: u32,
        top: u32,
    ) {
        let height = y1 - y0;
        let base = self.vertices.len() as u16;
        for i in 0..=sides {
            let theta = f32::from(i) / f32::from(sides) * std::f32::consts::TAU;
            let (sin, cos) = theta.sin_cos();
            let normal = normalize([cos * height, r0 - r1, sin * height]);
            for (y, r) in [(y0, r0), (y1, r1)] {
                self.vertices.push(MeshVertex {
                    position: [centre[0] + cos * r, y, centre[1] + sin * r],
                    normal,
                    colour: opaque(colour),
                });
            }
        }
        for i in 0..sides {
            let (a, b) = (base + 2 * i, base + 2 * i + 2);
            // Bottom, bottom of the next, top, top of the next; anticlockwise seen from outside.
            self.indices
                .extend_from_slice(&[a, a + 1, b + 1, a, b + 1, b]);
        }
        if r1 > 0.0 {
            let middle = self.vertices.len() as u16;
            self.vertices.push(MeshVertex {
                position: [centre[0], y1, centre[1]],
                normal: [0.0, 1.0, 0.0],
                colour: opaque(top),
            });
            let ring = middle + 1;
            for i in 0..=sides {
                let theta = f32::from(i) / f32::from(sides) * std::f32::consts::TAU;
                let (sin, cos) = theta.sin_cos();
                self.vertices.push(MeshVertex {
                    position: [centre[0] + cos * r1, y1, centre[1] + sin * r1],
                    normal: [0.0, 1.0, 0.0],
                    colour: opaque(top),
                });
            }
            for i in 0..sides {
                self.indices
                    .extend_from_slice(&[middle, ring + i + 1, ring + i]);
            }
        }
    }

    /// A wall along x or z between two points with gaps (ranges along the wall) left open to the floor.
    fn wall(&mut self, from: [f32; 2], to: [f32; 2], gaps: &[(f32, f32)]) {
        let along_x = (to[0] - from[0]).abs() > (to[1] - from[1]).abs();
        let (start, end) = if along_x {
            (from[0], to[0])
        } else {
            (from[1], to[1])
        };
        let mut edges = vec![start];
        for (a, b) in gaps {
            edges.extend([*a, *b]);
        }
        edges.push(end);
        let half = WALL_THICKNESS / 2.0;
        let colour = mix(HIGHLIGHT_MED, SUBTLE, 0.35);
        for pair in edges.chunks(2) {
            let (a, b) = (pair[0], pair[1]);
            let (low, high) = if along_x {
                (
                    [a - half, 0.0, from[1] - half],
                    [b + half, WALL_HEIGHT, from[1] + half],
                )
            } else {
                (
                    [from[0] - half, 0.0, a - half],
                    [from[0] + half, WALL_HEIGHT, b + half],
                )
            };
            self.block(low, high, colour, mix(SUBTLE, TEXT, 0.4));
        }
    }

    fn finish(self) -> MeshData {
        MeshData {
            vertices: self.vertices,
            indices: self.indices,
        }
    }
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(a: Vec3) -> Vec3 {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

/// Books in a row on a shelf: `count` thin blocks of varying height starting at `x`.
fn books(b: &mut Builder, x: f32, y: f32, z: f32, count: u32, along_x: bool) {
    let palette = [LOVE, GOLD, PINE, IRIS, FOAM, ROSE];
    for i in 0..count {
        let h = 0.22 + 0.06 * ((i * 7 % 5) as f32);
        let off = i as f32 * 0.055;
        let colour = palette[(i % palette.len() as u32) as usize];
        let (low, high) = if along_x {
            ([x + off, y, z], [x + off + 0.045, y + h, z + 0.2])
        } else {
            ([x, y, z + off], [x + 0.2, y + h, z + off + 0.045])
        };
        b.block(low, high, colour, mix(colour, TEXT, 0.3));
    }
}

fn plant(b: &mut Builder, x: f32, z: f32) {
    b.frustum([x, z], 0.0, 0.3, 0.2, 0.16, 16, mix(ROSE, LOVE, 0.4), MUTED);
    b.frustum([x, z], 0.3, 0.95, 0.28, 0.0, 12, mix(PINE, FOAM, 0.2), FOAM);
    b.frustum([x, z], 0.55, 1.05, 0.2, 0.0, 12, mix(PINE, FOAM, 0.4), FOAM);
}

fn living_room(b: &mut Builder) {
    // Rug, sofa with back, arms and cushions, coffee table, TV unit, shelf and a plant.
    b.block(
        [1.0, 0.0, 1.5],
        [3.8, 0.02, 3.0],
        mix(IRIS, SURFACE, 0.6),
        mix(IRIS, SURFACE, 0.55),
    );
    b.block(
        [1.2, 0.0, 3.1],
        [3.6, 0.42, 3.85],
        mix(IRIS, SURFACE, 0.25),
        IRIS,
    );
    b.block(
        [1.2, 0.42, 3.65],
        [3.6, 0.9, 3.85],
        mix(IRIS, SURFACE, 0.25),
        IRIS,
    );
    for x in [1.2, 3.4] {
        b.block(
            [x, 0.0, 3.1],
            [x + 0.2, 0.62, 3.85],
            mix(IRIS, SURFACE, 0.35),
            IRIS,
        );
    }
    for i in 0..3 {
        let x = 1.45 + i as f32 * 0.65;
        b.block(
            [x, 0.42, 3.15],
            [x + 0.6, 0.55, 3.62],
            mix(IRIS, TEXT, 0.2),
            mix(IRIS, TEXT, 0.35),
        );
    }
    b.frustum([2.4, 2.25], 0.0, 0.32, 0.12, 0.12, 16, MUTED, MUTED);
    b.frustum(
        [2.4, 2.25],
        0.32,
        0.38,
        0.6,
        0.6,
        24,
        GOLD,
        mix(GOLD, TEXT, 0.25),
    );
    b.block([1.4, 0.0, 0.2], [3.4, 0.5, 0.65], HIGHLIGHT_HIGH, SUBTLE);
    b.block(
        [1.7, 0.5, 0.35],
        [3.1, 1.05, 0.42],
        SURFACE,
        mix(SURFACE, PINE, 0.5),
    );
    b.block(
        [0.1, 0.0, 0.9],
        [0.45, 1.0, 2.7],
        mix(GOLD, SURFACE, 0.45),
        GOLD,
    );
    for y in [0.05, 0.4, 0.72] {
        books(b, 0.15, y, 0.95, 28, false);
    }
    plant(b, 5.0, 0.45);
    plant(b, 5.0, 3.55);
}

fn kitchen(b: &mut Builder) {
    let steel = mix(FOAM, TEXT, 0.35);
    b.block(
        [5.8, 0.0, 0.1],
        [9.8, 0.9, 0.7],
        mix(PINE, SURFACE, 0.3),
        mix(SUBTLE, TEXT, 0.5),
    );
    b.block(
        [9.3, 0.0, 0.7],
        [9.8, 0.9, 3.0],
        mix(PINE, SURFACE, 0.3),
        mix(SUBTLE, TEXT, 0.5),
    );
    b.block([5.8, 0.0, 0.7], [6.5, 1.1, 1.4], steel, TEXT);
    b.block(
        [7.0, 0.0, 1.8],
        [8.6, 0.9, 2.5],
        mix(PINE, SURFACE, 0.1),
        mix(SUBTLE, TEXT, 0.55),
    );
    for i in 0..3 {
        let x = 7.2 + i as f32 * 0.6;
        b.frustum([x, 2.95], 0.0, 0.6, 0.05, 0.05, 12, MUTED, MUTED);
        b.frustum(
            [x, 2.95],
            0.6,
            0.68,
            0.19,
            0.19,
            20,
            LOVE,
            mix(LOVE, TEXT, 0.25),
        );
    }
    // A hob with four rings, a sink and a pot.
    for (x, z) in [(6.9, 0.25), (7.3, 0.25), (6.9, 0.55), (7.3, 0.55)] {
        b.frustum([x, z], 0.9, 0.92, 0.1, 0.1, 16, SURFACE, SURFACE);
    }
    b.block([8.3, 0.85, 0.15], [9.1, 0.9, 0.65], steel, TEXT);
    b.frustum(
        [7.9, 2.15],
        0.9,
        1.1,
        0.14,
        0.12,
        16,
        GOLD,
        mix(GOLD, TEXT, 0.2),
    );
    plant(b, 9.5, 3.2);
}

fn bedroom(b: &mut Builder) {
    b.block([0.3, 0.0, 4.4], [2.3, 0.35, 6.6], HIGHLIGHT_HIGH, SUBTLE);
    b.block(
        [0.35, 0.35, 4.45],
        [2.25, 0.55, 6.55],
        mix(ROSE, SURFACE, 0.3),
        ROSE,
    );
    b.block(
        [0.1, 0.0, 4.4],
        [0.3, 1.0, 6.6],
        mix(GOLD, SURFACE, 0.5),
        GOLD,
    );
    for z in [4.6, 5.65] {
        b.block(
            [0.4, 0.55, z],
            [0.85, 0.68, z + 0.85],
            mix(TEXT, ROSE, 0.2),
            TEXT,
        );
    }
    b.block(
        [1.0, 0.55, 4.45],
        [2.25, 0.6, 6.55],
        mix(LOVE, SURFACE, 0.35),
        mix(LOVE, SURFACE, 0.2),
    );
    for z in [4.05, 6.65] {
        b.block(
            [0.3, 0.0, z],
            [0.75, 0.45, z + 0.3],
            mix(GOLD, SURFACE, 0.4),
            GOLD,
        );
        b.frustum(
            [0.52, z + 0.15],
            0.45,
            0.75,
            0.08,
            0.16,
            16,
            mix(GOLD, TEXT, 0.4),
            TEXT,
        );
    }
    b.block(
        [2.6, 0.0, 6.45],
        [3.9, 1.6, 6.9],
        mix(HIGHLIGHT_MED, SUBTLE, 0.3),
        SUBTLE,
    );
    b.block([1.2, 0.0, 4.15], [1.9, 0.02, 4.4], LOVE, LOVE);
    plant(b, 3.6, 4.4);
}

fn bathroom(b: &mut Builder) {
    b.block(
        [4.3, 0.0, 6.0],
        [6.3, 0.55, 6.85],
        mix(FOAM, TEXT, 0.5),
        TEXT,
    );
    b.block([4.4, 0.55, 6.1], [6.2, 0.56, 6.75], FOAM, FOAM);
    b.frustum([4.6, 4.5], 0.0, 0.4, 0.15, 0.2, 16, TEXT, TEXT);
    b.block([4.45, 0.4, 4.25], [4.75, 0.8, 4.5], TEXT, TEXT);
    b.block(
        [5.5, 0.0, 4.2],
        [6.2, 0.85, 4.6],
        mix(PINE, SURFACE, 0.2),
        mix(SUBTLE, TEXT, 0.5),
    );
    b.frustum([5.85, 4.4], 0.85, 0.92, 0.2, 0.2, 20, TEXT, FOAM);
    b.block(
        [5.2, 0.0, 5.3],
        [6.0, 0.02, 5.8],
        mix(FOAM, SURFACE, 0.4),
        mix(FOAM, SURFACE, 0.35),
    );
}

fn study(b: &mut Builder) {
    b.block(
        [7.0, 0.72, 6.1],
        [9.4, 0.78, 6.85],
        mix(GOLD, SURFACE, 0.3),
        GOLD,
    );
    for x in [7.05, 9.25] {
        b.block([x, 0.0, 6.15], [x + 0.1, 0.72, 6.8], HIGHLIGHT_HIGH, SUBTLE);
    }
    b.block(
        [7.9, 0.78, 6.55],
        [8.7, 1.25, 6.62],
        SURFACE,
        mix(SURFACE, PINE, 0.6),
    );
    b.frustum([8.3, 6.3], 0.78, 0.98, 0.09, 0.09, 12, MUTED, SUBTLE);
    b.frustum([8.3, 5.45], 0.0, 0.45, 0.05, 0.05, 12, MUTED, MUTED);
    b.frustum([8.3, 5.45], 0.45, 0.52, 0.24, 0.24, 20, PINE, FOAM);
    b.block([8.05, 0.52, 5.0], [8.55, 0.95, 5.1], PINE, FOAM);
    b.block(
        [9.75, 0.0, 4.3],
        [9.95, 1.5, 5.6],
        mix(GOLD, SURFACE, 0.45),
        GOLD,
    );
    books(b, 9.55, 0.05, 4.4, 20, false);
    books(b, 9.55, 0.6, 4.4, 20, false);
    b.block(
        [6.7, 0.0, 4.4],
        [7.3, 0.42, 5.4],
        mix(IRIS, SURFACE, 0.3),
        IRIS,
    );
    plant(b, 6.95, 6.6);
}

/// The whole flat.
pub fn floorplan() -> MeshData {
    let mut b = Builder::default();
    // The slab under everything.
    let (x1, z1) = (10.0 + SLAB_MARGIN, 7.0 + SLAB_MARGIN);
    b.block(
        [-SLAB_MARGIN, -SLAB_DEPTH, -SLAB_MARGIN],
        [x1, -0.01, z1],
        mix(OVERLAY, SURFACE, 0.4),
        mix(OVERLAY, MUTED, 0.3),
    );
    for &(x0, z0, x1, z1, floor) in &LAYOUT {
        b.quad(
            [x0, 0.0, z0],
            [0.0, 0.0, z1 - z0],
            [x1 - x0, 0.0, 0.0],
            floor,
        );
    }
    // Outer walls, then the walls between rooms with their doors.
    b.wall([0.0, 0.0], [10.0, 0.0], &[(6.6, 7.6)]);
    b.wall([0.0, 7.0], [10.0, 7.0], &[]);
    b.wall([0.0, 0.0], [0.0, 7.0], &[]);
    b.wall([10.0, 0.0], [10.0, 7.0], &[]);
    b.wall([5.5, 0.0], [5.5, 4.0], &[(1.5, 2.5)]);
    b.wall(
        [0.0, 4.0],
        [10.0, 4.0],
        &[(1.4, 2.4), (4.8, 5.7), (7.6, 8.6)],
    );
    b.wall([4.0, 4.0], [4.0, 7.0], &[(5.2, 6.1)]);
    b.wall([6.5, 4.0], [6.5, 7.0], &[(5.2, 6.1)]);
    living_room(&mut b);
    kitchen(&mut b);
    bedroom(&mut b);
    bathroom(&mut b);
    study(&mut b);
    b.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Bounds, layer_position};

    #[test]
    fn the_flat_is_a_valid_card_sized_mesh() {
        let mesh = floorplan();
        mesh.validate().expect("drawable");
        let triangles = mesh.indices.len() / 3;
        assert!(
            (1500..20_000).contains(&triangles),
            "{triangles} triangles is not floor plan card sized"
        );
        assert!(mesh.encode().is_ok(), "it fits the file format");
    }

    #[test]
    fn every_triangle_faces_the_way_its_normal_says() {
        // A wrong winding would be culled or lit from behind: compare each triangle's winding with its normal.
        let mesh = floorplan();
        for (n, t) in mesh.indices.chunks_exact(3).enumerate() {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.vertices[usize::from(i)]);
            let e1 = [
                b.position[0] - a.position[0],
                b.position[1] - a.position[1],
                b.position[2] - a.position[2],
            ];
            let e2 = [
                c.position[0] - a.position[0],
                c.position[1] - a.position[1],
                c.position[2] - a.position[2],
            ];
            let geometric = cross(e1, e2);
            let normal = a.normal;
            let agree =
                geometric[0] * normal[0] + geometric[1] * normal[1] + geometric[2] * normal[2];
            // Degenerate slivers (a cone's point) have no area to disagree with.
            let area = (geometric[0].powi(2) + geometric[1].powi(2) + geometric[2].powi(2)).sqrt();
            assert!(
                area < 1e-9 || agree > 0.0,
                "triangle {n} is wound against its normal"
            );
        }
    }

    #[test]
    fn normals_are_unit_and_colours_are_opaque() {
        for v in &floorplan().vertices {
            let l = (v.normal[0].powi(2) + v.normal[1].powi(2) + v.normal[2].powi(2)).sqrt();
            assert!((l - 1.0).abs() < 1e-4, "{:?}", v.normal);
            assert_eq!(v.colour.3, 255);
        }
    }

    #[test]
    fn every_room_label_lands_inside_a_640_by_400_layer_at_every_angle_of_the_orbit() {
        let mesh = floorplan();
        let bounds = Bounds::of(mesh.vertices.iter().map(|v| v.position));
        for angle in [0.0, 60.0, 120.0, 180.0, 240.0, 300.0] {
            for room in ROOMS {
                let [x, y] = layer_position(&CAMERA, angle, 640, 400, &bounds, room.anchor)
                    .expect("in front of the camera");
                assert!(
                    (0.0..640.0).contains(&x) && (0.0..400.0).contains(&y),
                    "{} at angle {angle}: {x}, {y}",
                    room.name
                );
            }
        }
    }

    #[test]
    fn the_whole_flat_is_in_view_of_the_still_camera() {
        let mesh = floorplan();
        let bounds = Bounds::of(mesh.vertices.iter().map(|v| v.position));
        for v in &mesh.vertices {
            let [x, y] = layer_position(&CAMERA, 0.0, 640, 400, &bounds, v.position).unwrap();
            assert!(
                (0.0..640.0).contains(&x) && (0.0..400.0).contains(&y),
                "{:?} lands at {x}, {y}",
                v.position
            );
        }
    }

    #[test]
    fn labels_sit_over_their_own_rooms() {
        for (room, layout) in ROOMS.iter().zip(&LAYOUT) {
            let (x0, z0, x1, z1, _) = *layout;
            let [x, _, z] = room.anchor;
            assert!(
                (x0..x1).contains(&x) && (z0..z1).contains(&z),
                "{} is labelled outside itself",
                room.name
            );
        }
    }
}
