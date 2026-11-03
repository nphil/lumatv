//! The look-at camera of a `Content::Mesh` layer, as plain arithmetic: where an orbiting eye is, the matrix that
//! maps the scene to the layer, and how far the near and far planes must reach.
//!
//! World space is right-handed with y up. Everything that ends on the screen uses the layer's own orientation:
//! x to the right and **y down**, as in an image, because the compositor draws the scene straight into a texture
//! that is then shown like an image layer. [`project`] therefore returns positions an app can use unchanged as
//! layer pixels.

use compositor_protocol::Mesh;

pub type Vec3 = [f32; 3];
/// Column-major, like OpenGL: element (row, column) is at `column * 4 + row`.
pub type Mat4 = [f32; 16];

/// Nearer than this fraction of the eye's distance to the middle of the scene the near plane is never put, so the
/// depth buffer keeps its precision when the eye is inside the scene's bounding sphere.
const MIN_NEAR_FRACTION: f32 = 0.02;

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}

/// `a` scaled to length 1, or `fallback` for a vector too short to have a direction.
pub fn normalize(a: Vec3, fallback: Vec3) -> Vec3 {
    let l = length(a);
    if l < 1e-6 {
        fallback
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}

/// Where the eye is after turning `angle_deg` about the vertical axis through `target`. Positive angles turn
/// anticlockwise seen from above (from +x towards -z).
pub fn orbit_eye(eye: Vec3, target: Vec3, angle_deg: f32) -> Vec3 {
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    let (dx, dz) = (eye[0] - target[0], eye[2] - target[2]);
    [
        target[0] + dx * cos + dz * sin,
        eye[1],
        target[2] - dx * sin + dz * cos,
    ]
}

/// The mesh's camera as it is `angle_deg` into its orbit: the same style with the eye turned and the orbit rate
/// dropped. Two frames whose looks are equal show the same picture, which is how the compositor knows a cached
/// render is still good.
pub fn look_at_phase(mesh: &Mesh, angle_deg: f32) -> Mesh {
    Mesh {
        eye: orbit_eye(mesh.eye, mesh.target, angle_deg),
        orbit_deg_per_s: 0.0,
        ..*mesh
    }
}

fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            out[column * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[column * 4 + k]).sum();
        }
    }
    out
}

/// The matrix taking world points to clip space for a camera at `eye` looking at `target`, with a vertical field
/// of view of `fov_deg` and a layer `aspect` (width over height) wide. Clip y is flipped (down is positive), see
/// the module documentation.
///
/// The camera's up is world up, except when it looks straight up or down, where world -z is up on the screen so
/// that a top-down plan has north at the top.
pub fn view_projection(
    eye: Vec3,
    target: Vec3,
    fov_deg: f32,
    aspect: f32,
    near: f32,
    far: f32,
) -> Mat4 {
    let forward = normalize(sub(target, eye), [0.0, 0.0, -1.0]);
    let mut up = [0.0, 1.0, 0.0];
    if length(cross(forward, up)) < 1e-4 {
        up = [0.0, 0.0, -1.0];
    }
    let right = normalize(cross(forward, up), [1.0, 0.0, 0.0]);
    let true_up = cross(right, forward);
    let view: Mat4 = [
        right[0],
        true_up[0],
        -forward[0],
        0.0,
        right[1],
        true_up[1],
        -forward[1],
        0.0,
        right[2],
        true_up[2],
        -forward[2],
        0.0,
        -dot(right, eye),
        -dot(true_up, eye),
        dot(forward, eye),
        1.0,
    ];
    let focal = 1.0 / (fov_deg.to_radians() / 2.0).tan();
    let depth = near - far;
    let projection: Mat4 = [
        focal / aspect,
        0.0,
        0.0,
        0.0,
        0.0,
        -focal,
        0.0,
        0.0,
        0.0,
        0.0,
        (far + near) / depth,
        -1.0,
        0.0,
        0.0,
        2.0 * far * near / depth,
        0.0,
    ];
    multiply(&projection, &view)
}

/// Normalised device coordinates (x right, y down, depth -1 at the near plane to 1 at the far one) of a world
/// point, or `None` if it is behind the camera.
pub fn project(matrix: &Mat4, point: Vec3) -> Option<Vec3> {
    let m = matrix;
    let at = |row: usize| {
        m[row] * point[0] + m[4 + row] * point[1] + m[8 + row] * point[2] + m[12 + row]
    };
    let w = at(3);
    (w > 0.0).then(|| [at(0) / w, at(1) / w, at(2) / w])
}

/// The sphere that holds a whole scene: the middle of its bounding box and the distance to the farthest point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub centre: Vec3,
    pub radius: f32,
}

impl Bounds {
    /// Bounds of `points`; a point at the origin for none.
    pub fn of(points: impl IntoIterator<Item = Vec3>) -> Self {
        let points: Vec<Vec3> = points.into_iter().collect();
        if points.is_empty() {
            return Self {
                centre: [0.0; 3],
                radius: 0.0,
            };
        }
        let mut low = points[0];
        let mut high = points[0];
        for p in &points {
            for axis in 0..3 {
                low[axis] = low[axis].min(p[axis]);
                high[axis] = high[axis].max(p[axis]);
            }
        }
        let centre = [
            (low[0] + high[0]) / 2.0,
            (low[1] + high[1]) / 2.0,
            (low[2] + high[2]) / 2.0,
        ];
        let radius = points
            .iter()
            .map(|p| length(sub(*p, centre)))
            .fold(0.0, f32::max);
        Self { centre, radius }
    }
}

/// The near and far plane distances that just hold `bounds` seen from `eye`.
pub fn depth_range(eye: Vec3, bounds: &Bounds) -> (f32, f32) {
    let distance = length(sub(eye, bounds.centre));
    let near = (distance - bounds.radius)
        .max(distance * MIN_NEAR_FRACTION)
        .max(1e-3);
    let far = (distance + bounds.radius).max(near * 2.0);
    (near, far)
}

/// The matrix for `mesh` as it is `angle_deg` into its orbit, drawn into a `width` × `height` layer, with the
/// planes fitted to `bounds`.
pub fn mesh_matrix(mesh: &Mesh, angle_deg: f32, width: u32, height: u32, bounds: &Bounds) -> Mat4 {
    let eye = orbit_eye(mesh.eye, mesh.target, angle_deg);
    let (near, far) = depth_range(eye, bounds);
    view_projection(
        eye,
        mesh.target,
        mesh.fov_deg,
        width as f32 / height.max(1) as f32,
        near,
        far,
    )
}

/// Where a world point lands in a `width` × `height` mesh layer, in layer pixels from its top-left corner (the
/// same maths the compositor draws with), or `None` if it is behind the camera. `angle_deg` is how far into its
/// orbit the mesh is: 0 for a still camera.
pub fn layer_position(
    mesh: &Mesh,
    angle_deg: f32,
    width: u32,
    height: u32,
    bounds: &Bounds,
    point: Vec3,
) -> Option<[f32; 2]> {
    let matrix = mesh_matrix(mesh, angle_deg, width, height, bounds);
    let ndc = project(&matrix, point)?;
    Some([
        (ndc[0] + 1.0) / 2.0 * width as f32,
        (ndc[1] + 1.0) / 2.0 * height as f32,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use compositor_protocol::Rgba;

    fn mesh(eye: Vec3, target: Vec3) -> Mesh {
        Mesh {
            eye,
            target,
            fov_deg: 40.0,
            light: [0.0, 1.0, 0.0],
            ambient: 0.4,
            orbit_deg_per_s: 12.0,
            antialias: false,
            background: Rgba(0, 0, 0, 0),
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    const UNIT: Bounds = Bounds {
        centre: [0.0, 0.0, 0.0],
        radius: 1.0,
    };

    #[test]
    fn the_target_is_in_the_middle_of_the_picture() {
        let m = view_projection([3.0, 4.0, 5.0], [1.0, 0.5, -1.0], 40.0, 1.6, 0.1, 50.0);
        let ndc = project(&m, [1.0, 0.5, -1.0]).unwrap();
        assert!(close(ndc[0], 0.0) && close(ndc[1], 0.0), "{ndc:?}");
    }

    #[test]
    fn right_is_right_and_up_is_towards_the_top_of_the_layer() {
        // Looking down the -z axis from +z: +x is on the right, +y is up.
        let m = view_projection([0.0, 0.0, 5.0], [0.0; 3], 40.0, 1.0, 0.1, 50.0);
        let right = project(&m, [1.0, 0.0, 0.0]).unwrap();
        let above = project(&m, [0.0, 1.0, 0.0]).unwrap();
        assert!(right[0] > 0.1 && close(right[1], 0.0), "{right:?}");
        assert!(
            above[1] < -0.1 && close(above[0], 0.0),
            "y is down: {above:?}"
        );
    }

    #[test]
    fn the_field_of_view_is_vertical_and_the_aspect_widens_only_x() {
        let at = 10.0;
        let half = at * (20.0f32).to_radians().tan();
        for aspect in [1.0, 1.6] {
            let m = view_projection([0.0, 0.0, at], [0.0; 3], 40.0, aspect, 0.1, 50.0);
            let top = project(&m, [0.0, half, 0.0]).unwrap();
            assert!(close(top[1], -1.0), "aspect {aspect}: {top:?}");
            let side = project(&m, [half * aspect, 0.0, 0.0]).unwrap();
            assert!(close(side[0], 1.0), "aspect {aspect}: {side:?}");
        }
    }

    #[test]
    fn depth_runs_from_the_near_plane_to_the_far_plane() {
        let m = view_projection([0.0, 0.0, 10.0], [0.0; 3], 40.0, 1.0, 2.0, 30.0);
        assert!(close(project(&m, [0.0, 0.0, 8.0]).unwrap()[2], -1.0));
        assert!(close(project(&m, [0.0, 0.0, -20.0]).unwrap()[2], 1.0));
        assert!(project(&m, [0.0, 0.0, 11.0]).is_none(), "behind the eye");
    }

    #[test]
    fn looking_straight_down_puts_north_at_the_top() {
        let m = view_projection([0.0, 10.0, 0.0], [0.0; 3], 40.0, 1.0, 0.1, 50.0);
        let north = project(&m, [0.0, 0.0, -1.0]).unwrap();
        let east = project(&m, [1.0, 0.0, 0.0]).unwrap();
        assert!(north[1] < -0.05 && close(north[0], 0.0), "{north:?}");
        assert!(east[0] > 0.05 && close(east[1], 0.0), "{east:?}");
    }

    #[test]
    fn an_eye_on_its_target_still_gives_a_finite_matrix() {
        let m = view_projection([1.0; 3], [1.0; 3], 40.0, 1.0, 0.1, 50.0);
        assert!(m.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn orbiting_turns_the_eye_about_the_vertical_axis_through_the_target() {
        let target = [2.0, 1.0, 3.0];
        let eye = [5.0, 4.0, 3.0];
        let quarter = orbit_eye(eye, target, 90.0);
        // From +x round to -z, height unchanged.
        assert!(
            close(quarter[0], 2.0) && close(quarter[2], 0.0),
            "{quarter:?}"
        );
        assert_eq!(quarter[1], 4.0);
        let full = orbit_eye(eye, target, 360.0);
        assert!((0..3).all(|i| close(full[i], eye[i])), "{full:?}");
        let distance = |e: Vec3| length(sub(e, target));
        assert!(close(distance(orbit_eye(eye, target, 37.0)), distance(eye)));
        assert_eq!(orbit_eye(eye, target, 0.0), eye);
    }

    #[test]
    fn a_look_ignores_the_orbit_rate_so_a_still_frame_matches_its_cache() {
        let m = mesh([5.0, 4.0, 3.0], [0.0; 3]);
        let a = look_at_phase(&m, 30.0);
        let b = look_at_phase(
            &Mesh {
                orbit_deg_per_s: 0.0,
                ..m
            },
            30.0,
        );
        assert_eq!(a, b);
        assert_ne!(a, look_at_phase(&m, 31.0));
        assert_eq!(look_at_phase(&m, 0.0).eye, m.eye);
    }

    #[test]
    fn the_planes_hold_the_scene_from_every_side_of_an_orbit() {
        let bounds = Bounds {
            centre: [5.0, 0.5, 3.5],
            radius: 6.0,
        };
        let m = mesh([5.0, 9.0, 12.7], [5.0, 0.4, 3.5]);
        for angle in [0.0, 45.0, 90.0, 180.0, 270.0] {
            let matrix = mesh_matrix(&m, angle, 640, 400, &bounds);
            // The nearest and farthest points of the bounding sphere along the line from the eye.
            let eye = orbit_eye(m.eye, m.target, angle);
            let dir = normalize(sub(bounds.centre, eye), [0.0, 0.0, -1.0]);
            for sign in [-1.0f32, 1.0] {
                let p = [
                    bounds.centre[0] + dir[0] * bounds.radius * sign,
                    bounds.centre[1] + dir[1] * bounds.radius * sign,
                    bounds.centre[2] + dir[2] * bounds.radius * sign,
                ];
                let depth = project(&matrix, p).unwrap()[2];
                assert!((-1.001..=1.001).contains(&depth), "angle {angle}: {depth}");
            }
        }
    }

    #[test]
    fn an_eye_inside_the_scene_keeps_a_usable_near_plane() {
        let (near, far) = depth_range(
            [0.0; 3],
            &Bounds {
                radius: 5.0,
                ..UNIT
            },
        );
        assert!(near >= 1e-3 && far > near);
        let (near, _) = depth_range([0.0, 0.0, 10.0], &UNIT);
        assert!(close(near, 9.0));
    }

    #[test]
    fn bounds_hold_every_point() {
        let b = Bounds::of([[0.0, 0.0, 0.0], [4.0, 2.0, 0.0], [0.0, 0.0, 6.0]]);
        assert_eq!(b.centre, [2.0, 1.0, 3.0]);
        for p in [[0.0, 0.0, 0.0], [4.0, 2.0, 0.0], [0.0, 0.0, 6.0]] {
            assert!(length(sub(p, b.centre)) <= b.radius + 1e-5);
        }
        assert_eq!(Bounds::of([]).radius, 0.0);
    }

    #[test]
    fn a_layer_position_is_in_pixels_from_the_top_left() {
        let m = mesh([0.0, 0.0, 5.0], [0.0; 3]);
        let [x, y] = layer_position(&m, 0.0, 640, 400, &UNIT, [0.0; 3]).unwrap();
        assert!(close(x, 320.0) && close(y, 200.0), "{x} {y}");
        let [x, y] = layer_position(&m, 0.0, 640, 400, &UNIT, [0.5, 0.5, 0.0]).unwrap();
        assert!(
            x > 320.0 && y < 200.0,
            "right and up of the middle: {x} {y}"
        );
    }
}
