//! Mesh layers (`Content::Mesh`): a lit, depth-tested 3D scene drawn into the layer's own off-screen target, then
//! shown in the frame like an image layer.
//!
//! Drawing the scene is far dearer than showing a picture of it, so the picture is kept: a layer is re-rendered
//! only when its look (camera, light, colours) is not the one the picture was drawn for. A still mesh costs one
//! render and then nothing; an orbiting one renders every frame, and the engine keeps frames coming for it.
//!
//! Lighting is per vertex (Lambert: `ambient + (1 - ambient) * max(0, n·l)`), so the fragment shader only writes
//! the interpolated colour. Vertex colours are premultiplied and stay so. Triangles are opaque and depth-tested
//! (a translucent vertex colour is not blended with what is behind it in the mesh); the background is cleared to
//! `Mesh::background`, so a transparent one shows the layers below when the result is composited.

use crate::gl::Gles;
use crate::gl::mesh::{Geometry, Offscreen};
use crate::log::log;
use compositor_protocol::{LayerId, MESH_VERTEX_BYTES, Mesh, MeshData};
use mesh3d::camera::{self, Bounds, Mat4};
use std::collections::HashMap;
use std::ffi::CStr;
use std::io;
use std::time::Instant;

/// Samples per pixel of an antialiased layer, if the GPU has them.
const WANTED_SAMPLES: u32 = 4;

const VERTEX: &CStr = c"#version 300 es
layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in mediump vec4 a_colour;
uniform mat4 u_view_projection;
uniform vec3 u_light;
uniform float u_ambient;
out mediump vec4 v_colour;
void main() {
    float lit = max(dot(normalize(a_normal), u_light), 0.0);
    float shade = u_ambient + (1.0 - u_ambient) * lit;
    v_colour = vec4(a_colour.rgb * shade, a_colour.a);
    gl_Position = u_view_projection * vec4(a_position, 1.0);
}";

const FRAGMENT: &CStr = c"#version 300 es
precision mediump float;
in mediump vec4 v_colour;
out vec4 o_color;
void main() {
    o_color = v_colour;
}";

/// The vertices in the layout the GPU reads them in, which is the mesh file's: position, normal as signed bytes,
/// colour.
pub fn pack_vertices(data: &MeshData) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.vertices.len() * MESH_VERTEX_BYTES);
    for v in &data.vertices {
        for c in v.position {
            out.extend_from_slice(&c.to_le_bytes());
        }
        for c in v.normal {
            out.push((c.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8);
        }
        out.push(0);
        out.extend_from_slice(&[v.colour.0, v.colour.1, v.colour.2, v.colour.3]);
    }
    out
}

/// Whether a layer's picture is still the one its look calls for.
#[derive(Default)]
struct Freshness {
    drawn: Option<Mesh>,
}

impl Freshness {
    /// Records that a picture for `look` is about to be drawn, unless one already is. Returns whether to draw.
    fn needs_render(&mut self, look: Mesh) -> bool {
        if self.drawn == Some(look) {
            return false;
        }
        self.drawn = Some(look);
        true
    }

    /// The picture is gone (its target was replaced).
    fn forget(&mut self) {
        self.drawn = None;
    }
}

struct MeshLayer {
    width: u32,
    height: u32,
    geometry: Geometry,
    bounds: Bounds,
    target: Option<Offscreen>,
    /// The most samples a target of this size was ever made with; 0 after a multisampled one failed.
    samples: u32,
    freshness: Freshness,
}

pub struct MeshRenderer {
    program: u32,
    view_projection: i32,
    light: i32,
    ambient: i32,
    samples: u32,
    layers: HashMap<LayerId, MeshLayer>,
}

impl MeshRenderer {
    /// Builds the program now, so that no shader is compiled while the screen is in use.
    pub fn new(gl: &Gles) -> io::Result<Self> {
        let started = Instant::now();
        let program = gl.program(VERTEX, FRAGMENT)?;
        let at = |name: &CStr| gl.uniform_location(program, name);
        let samples = gl.max_samples().min(WANTED_SAMPLES);
        log(format!(
            "mesh program ready in {:.1} ms, {samples}x multisampling resolved {}",
            started.elapsed().as_secs_f64() * 1e3,
            if gl.resolves_in_tile() {
                "in the tile"
            } else {
                "by blit"
            }
        ));
        Ok(Self {
            view_projection: at(c"u_view_projection"),
            light: at(c"u_light"),
            ambient: at(c"u_ambient"),
            program,
            samples,
            layers: HashMap::new(),
        })
    }

    /// Takes in a new layer's geometry. Its target is made when it is first drawn, for the look it has then.
    pub fn create(&mut self, gl: &Gles, id: LayerId, width: u32, height: u32, data: &MeshData) {
        self.remove(gl, id);
        let bounds = Bounds::of(data.vertices.iter().map(|v| v.position));
        self.layers.insert(
            id,
            MeshLayer {
                width,
                height,
                geometry: gl.geometry(&pack_vertices(data), &data.indices),
                bounds,
                target: None,
                samples: self.samples,
                freshness: Freshness::default(),
            },
        );
    }

    pub fn remove(&mut self, gl: &Gles, id: LayerId) {
        if let Some(layer) = self.layers.remove(&id) {
            gl.delete_geometry(layer.geometry);
            if let Some(target) = layer.target {
                gl.delete_offscreen(target);
            }
        }
    }

    pub fn clear(&mut self, gl: &Gles) {
        let ids: Vec<LayerId> = self.layers.keys().copied().collect();
        for id in ids {
            self.remove(gl, id);
        }
    }

    /// The texture holding the layer's picture, once it has been drawn.
    pub fn texture(&self, id: LayerId) -> Option<u32> {
        self.layers.get(&id)?.target.as_ref().map(|t| t.texture)
    }

    /// Draws the layer's picture for `style`, `phase_deg` into its orbit, unless the picture it has is that one
    /// already. Must run before the frame pass: it changes the framebuffer. Returns whether it drew.
    pub fn prepare(&mut self, gl: &Gles, id: LayerId, style: &Mesh, phase_deg: f32) -> bool {
        let Some(layer) = self.layers.get_mut(&id) else {
            return false;
        };
        let samples = if style.antialias { layer.samples } else { 0 };
        if layer.target.as_ref().is_none_or(|t| t.samples() != samples) {
            if let Some(old) = layer.target.take() {
                gl.delete_offscreen(old);
            }
            let made = gl
                .offscreen(layer.width, layer.height, samples)
                .or_else(|e| {
                    // Draw without anti-aliasing rather than not at all.
                    log(format!(
                        "mesh layer {}: {e}; drawing without multisampling",
                        id.0
                    ));
                    layer.samples = 0;
                    gl.offscreen(layer.width, layer.height, 0)
                });
            match made {
                Ok(target) => layer.target = Some(target),
                Err(e) => {
                    log(format!("mesh layer {}: {e}", id.0));
                    return false;
                }
            }
            layer.freshness.forget();
        }
        let look = camera::look_at_phase(style, phase_deg);
        if !layer.freshness.needs_render(look) {
            return false;
        }
        let target = layer.target.as_ref().expect("made above");
        let matrix: Mat4 =
            camera::mesh_matrix(style, phase_deg, layer.width, layer.height, &layer.bounds);
        let light = camera::normalize(style.light, [0.0, 1.0, 0.0]);
        let background = style.background;
        gl.begin_offscreen(
            target,
            [
                f32::from(background.0) / 255.0,
                f32::from(background.1) / 255.0,
                f32::from(background.2) / 255.0,
                f32::from(background.3) / 255.0,
            ],
        );
        gl.use_program(self.program);
        gl.uniform_matrix4(self.view_projection, &matrix);
        gl.uniform3f(self.light, light[0], light[1], light[2]);
        gl.uniform1f(self.ambient, style.ambient);
        gl.draw_geometry(&layer.geometry);
        gl.end_offscreen(target);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compositor_protocol::{MeshVertex, Rgba};

    fn style() -> Mesh {
        Mesh {
            eye: [5.0, 9.0, 12.0],
            target: [5.0, 0.0, 3.0],
            fov_deg: 40.0,
            light: [-0.4, 0.8, 0.4],
            ambient: 0.5,
            orbit_deg_per_s: 10.0,
            antialias: true,
            background: Rgba(0, 0, 0, 0),
        }
    }

    // The tests below draw with the real renderer on a software GL and look at the pixels. A mesh layer of 200x100
    // at (100, 50) sits over a blue screen; the camera is 5 in front of the origin looking down -z at 40 degrees.

    use crate::ops::Op;
    use crate::scene::Drawn;
    use crate::testgl::{Rig, Shot, drawn, rig};
    use compositor_protocol::{Content, Fill, Props, Shape};
    use std::sync::Arc;

    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn quad(
        vertices: &mut Vec<MeshVertex>,
        indices: &mut Vec<u16>,
        [x0, y0, x1, y1]: [f32; 4],
        z: f32,
        normal: [f32; 3],
        colour: Rgba,
    ) {
        let base = vertices.len() as u16;
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            vertices.push(MeshVertex {
                position: [x, y, z],
                normal,
                colour,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn camera() -> Mesh {
        Mesh {
            eye: [0.0, 0.0, 5.0],
            target: [0.0; 3],
            fov_deg: 40.0,
            light: [0.0, 0.0, 1.0],
            ambient: 0.0,
            orbit_deg_per_s: 0.0,
            antialias: false,
            background: Rgba(0, 0, 0, 0),
        }
    }

    /// A red square behind, a green bar in front of it to the right of the middle, a yellow tab above the middle.
    fn scene_data() -> MeshData {
        let (mut v, mut i) = (Vec::new(), Vec::new());
        let facing = [0.0, 0.0, 1.0];
        quad(
            &mut v,
            &mut i,
            [-1.0, -1.0, 1.0, 1.0],
            -1.0,
            facing,
            Rgba(255, 0, 0, 255),
        );
        quad(
            &mut v,
            &mut i,
            [0.0, -0.5, 2.0, 0.5],
            1.0,
            facing,
            Rgba(0, 255, 0, 255),
        );
        quad(
            &mut v,
            &mut i,
            [-0.2, 0.5, 0.2, 1.0],
            0.0,
            facing,
            Rgba(255, 255, 0, 255),
        );
        MeshData {
            vertices: v,
            indices: i,
        }
    }

    fn rig_with_mesh(data: MeshData, style: Mesh) -> Option<Rig> {
        let mut rig = rig()?;
        let backdrop = Shape {
            corner_radius: 0.0,
            fill: Fill::Solid(Rgba(0, 0, 255, 255)),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        };
        for op in [
            Op::Create {
                id: LayerId(1),
                width: 960,
                height: 480,
                z: 0,
                content: Content::Shape(backdrop),
                props: Props::at(0.0, 0.0),
                image: None,
                mesh: None,
            },
            Op::Create {
                id: LayerId(2),
                width: 200,
                height: 100,
                z: 1,
                content: Content::Mesh(style),
                props: Props::at(100.0, 50.0),
                image: None,
                mesh: Some(Arc::new(data)),
            },
        ] {
            rig.renderer.apply(&op);
        }
        Some(rig)
    }

    fn frame(rig: &mut Rig, style: Mesh, phase: f64) -> Shot {
        let backdrop = Shape {
            corner_radius: 0.0,
            fill: Fill::Solid(Rgba(0, 0, 255, 255)),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        };
        let mut mesh = drawn(2, (200, 100), Content::Mesh(style), Props::at(100.0, 50.0));
        mesh.phase = phase;
        rig.draw(&[
            drawn(1, (960, 480), Content::Shape(backdrop), Props::at(0.0, 0.0)),
            mesh,
        ])
    }

    /// The screen's pixel at a position inside the mesh layer.
    fn at(shot: &Shot, x: u32, y: u32) -> [u8; 4] {
        shot.pixel(100 + x, 50 + y)
    }

    fn near(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
    }

    #[test]
    fn a_mesh_layer_shows_its_scene_the_right_way_up_and_in_depth_order() {
        let Some(mut rig) = rig_with_mesh(scene_data(), camera()) else {
            return;
        };
        let shot = frame(&mut rig, camera(), 0.0);
        shot.save("mesh-scene");
        let red = [255, 0, 0, 255];
        let green = [0, 255, 0, 255];
        let yellow = [255, 255, 0, 255];
        assert!(
            near(at(&shot, 150, 50), green),
            "the bar is right of the middle"
        );
        assert!(
            near(at(&shot, 85, 50), red),
            "the square shows left of the bar"
        );
        assert!(
            near(at(&shot, 110, 50), green),
            "the near bar hides the far square"
        );
        assert!(
            near(at(&shot, 100, 30), yellow),
            "the tab is above the middle"
        );
        assert!(
            near(at(&shot, 100, 75), BLUE),
            "below the tab there is only the square: it is 73 px tall"
        );
        assert!(near(at(&shot, 100, 70), red));
        assert!(
            near(at(&shot, 10, 10), BLUE),
            "where no triangle is the layers below show through"
        );
    }

    #[test]
    fn light_and_ambient_shade_faces_by_how_squarely_they_face_the_light() {
        let (mut v, mut i) = (Vec::new(), Vec::new());
        let white = Rgba(200, 200, 200, 255);
        quad(
            &mut v,
            &mut i,
            [-1.5, -0.5, -0.5, 0.5],
            0.0,
            [0.0, 0.0, 1.0],
            white,
        );
        quad(
            &mut v,
            &mut i,
            [0.5, -0.5, 1.5, 0.5],
            0.0,
            [0.6, 0.0, 0.8],
            white,
        );
        quad(
            &mut v,
            &mut i,
            [-0.4, -0.5, 0.4, 0.5],
            0.0,
            [0.0, 0.0, -1.0],
            white,
        );
        let style = Mesh {
            ambient: 0.2,
            ..camera()
        };
        let Some(mut rig) = rig_with_mesh(
            MeshData {
                vertices: v,
                indices: i,
            },
            style,
        ) else {
            return;
        };
        let shot = frame(&mut rig, style, 0.0);
        // Facing the light: full colour. At an angle: ambient + (1 - ambient) * cos = 0.2 + 0.8 * 0.8. Facing away:
        // ambient only. The squares are 27.5 px per unit from the middle: at x = 72, 128 and 100.
        assert!(near(at(&shot, 72, 50), [200, 200, 200, 255]));
        assert!(near(at(&shot, 128, 50), [168, 168, 168, 255]));
        assert!(near(at(&shot, 100, 50), [40, 40, 40, 255]));
    }

    #[test]
    fn the_orbit_turns_the_camera_about_the_target() {
        let Some(mut rig) = rig_with_mesh(scene_data(), camera()) else {
            return;
        };
        // A quarter turn: the eye goes from +z round to +x; the flat scene is then seen edge-on and is gone.
        let shot = frame(&mut rig, camera(), 90.0);
        shot.save("mesh-quarter-turn");
        assert!(near(at(&shot, 150, 50), BLUE));
        assert!(near(at(&shot, 85, 50), BLUE));
    }

    #[test]
    fn a_picture_is_drawn_again_only_when_the_camera_or_style_moved() {
        let Some(mut rig) = rig_with_mesh(scene_data(), camera()) else {
            return;
        };
        frame(&mut rig, camera(), 0.0);
        assert_eq!(rig.report.meshes, 1, "a new layer is drawn");
        frame(&mut rig, camera(), 0.0);
        assert_eq!(rig.report.meshes, 0, "nothing moved");
        // A slower or faster orbit rate is not a new picture; only where the eye is.
        let faster = Mesh {
            orbit_deg_per_s: 30.0,
            ..camera()
        };
        frame(&mut rig, faster, 0.0);
        assert_eq!(rig.report.meshes, 0);
        frame(&mut rig, faster, 0.5);
        assert_eq!(rig.report.meshes, 1, "the eye moved");
        let dimmer = Mesh {
            ambient: 0.3,
            ..faster
        };
        frame(&mut rig, dimmer, 0.5);
        assert_eq!(rig.report.meshes, 1, "the look changed");
        frame(&mut rig, dimmer, 0.5);
        assert_eq!(rig.report.meshes, 0);
    }

    #[test]
    fn a_turn_of_360_degrees_and_none_are_the_same_picture() {
        let Some(mut rig) = rig_with_mesh(scene_data(), camera()) else {
            return;
        };
        frame(&mut rig, camera(), 0.0);
        frame(&mut rig, camera(), 360.0);
        assert_eq!(rig.report.meshes, 0);
    }

    fn orbiting() -> Mesh {
        Mesh {
            orbit_deg_per_s: 30.0,
            ..camera()
        }
    }

    /// The backdrop, the mesh and above them what the test puts over the mesh's layer.
    fn frame_under(rig: &mut Rig, style: Mesh, phase: f64, over: Drawn) -> Shot {
        let backdrop = Shape {
            corner_radius: 0.0,
            fill: Fill::Solid(Rgba(0, 0, 255, 255)),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        };
        let mut mesh = drawn(2, (200, 100), Content::Mesh(style), Props::at(100.0, 50.0));
        mesh.phase = phase;
        rig.draw(&[
            drawn(1, (960, 480), Content::Shape(backdrop), Props::at(0.0, 0.0)),
            mesh,
            over,
        ])
    }

    fn page() -> Drawn {
        let page = Shape {
            corner_radius: 0.0,
            fill: Fill::Solid(Rgba(9, 9, 9, 255)),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        };
        drawn(3, (960, 480), Content::Shape(page), Props::at(0.0, 0.0))
    }

    #[test]
    fn a_mesh_an_opaque_page_covers_is_not_drawn_as_it_turns_and_is_when_the_page_goes() {
        let Some(mut rig) = rig_with_mesh(scene_data(), orbiting()) else {
            return;
        };
        frame(&mut rig, orbiting(), 0.0);
        assert_eq!(rig.report.meshes, 1, "it shows");
        // Nobody sees it under the page, however far it turns.
        for phase in [10.0, 20.0, 30.0] {
            frame_under(&mut rig, orbiting(), phase, page());
            assert_eq!(rig.report.culled, 2, "the page covers it and the backdrop");
            assert_eq!(rig.report.meshes, 0, "at {phase} degrees");
        }
        // With the page gone, the frame that shows the mesh draws it as it is now, not as it was: a quarter turn on the
        // flat scene is edge-on and gone.
        let shot = frame(&mut rig, orbiting(), 90.0);
        assert_eq!(rig.report.meshes, 1);
        assert!(near(at(&shot, 150, 50), BLUE));
        assert!(near(at(&shot, 85, 50), BLUE));
    }

    #[test]
    fn a_mesh_glass_covers_is_drawn_for_the_blur_behind_the_glass_that_shows() {
        use compositor_protocol::{Backdrop, Glass};
        let Some(mut rig) = rig_with_mesh(scene_data(), orbiting()) else {
            return;
        };
        let glass = |backdrop| {
            let content = Content::Glass(Glass { corner_radius: 0.0,
            tint: Rgba(0, 0, 0, 0),
            edge: Rgba(0, 0, 0, 0),
            edge_width: 0.0,
            highlight: Rgba(0, 0, 0, 0),
            sheen: None,
            shadows: [None, None],
            depth: None,
            backdrop,
            saturation: 1.0, refraction: 0.0, edge_lens: None, tint_gradient: None, });
            drawn(3, (300, 160), content, Props::at(50.0, 20.0))
        };
        let live = glass(Backdrop::Live {
            radius: 10.0,
            downsample: 4,
        });
        frame_under(&mut rig, orbiting(), 0.0, live);
        assert_eq!(rig.report.culled, 1, "the glass covers it");
        assert_eq!(rig.report.meshes, 1, "and blurs it live");
        frame_under(&mut rig, orbiting(), 10.0, live);
        assert_eq!(rig.report.meshes, 1, "as it turns");
        // A page over the glass hides that as well, and there is no blur to look at the mesh.
        let covered = |rig: &mut Rig, phase| {
            let backdrop = Shape {
                corner_radius: 0.0,
                fill: Fill::Solid(Rgba(0, 0, 255, 255)),
                border_width: 0.0,
                border: Rgba(0, 0, 0, 0),
            };
            let mut mesh = drawn(
                2,
                (200, 100),
                Content::Mesh(orbiting()),
                Props::at(100.0, 50.0),
            );
            mesh.phase = phase;
            rig.draw(&[
                drawn(1, (960, 480), Content::Shape(backdrop), Props::at(0.0, 0.0)),
                mesh,
                live,
                Drawn {
                    id: LayerId(4),
                    ..page()
                },
            ])
        };
        covered(&mut rig, 20.0);
        assert_eq!(rig.report.meshes, 0);
    }

    #[test]
    fn antialiasing_smooths_the_edges_of_triangles_and_only_when_asked() {
        // One triangle with a diagonal edge.
        let tri = |colour| MeshData {
            vertices: [[-1.0, -1.0], [1.0, -1.0], [0.0, 1.0]]
                .iter()
                .map(|p| MeshVertex {
                    position: [p[0], p[1], 0.0],
                    normal: [0.0, 0.0, 1.0],
                    colour,
                })
                .collect(),
            indices: vec![0, 1, 2],
        };
        let mixed = |aa: bool| {
            let style = Mesh {
                antialias: aa,
                ..camera()
            };
            let mut rig = rig_with_mesh(tri(Rgba(255, 0, 0, 255)), style)?;
            let shot = frame(&mut rig, style, 0.0);
            let mut between = 0;
            for y in 0..100 {
                for x in 0..200 {
                    let p = at(&shot, x, y);
                    if !near(p, BLUE) && !near(p, [255, 0, 0, 255]) {
                        between += 1;
                    }
                }
            }
            Some(between)
        };
        let (Some(plain), Some(smooth)) = (mixed(false), mixed(true)) else {
            return;
        };
        assert_eq!(
            plain, 0,
            "without antialiasing every pixel is all one or the other"
        );
        assert!(
            smooth > 20,
            "the diagonal edges have blended pixels: {smooth}"
        );
    }

    #[test]
    fn a_lost_picture_is_drawn_again() {
        let mut fresh = Freshness::default();
        let look = camera::look_at_phase(&style(), 0.0);
        assert!(fresh.needs_render(look));
        fresh.forget();
        assert!(fresh.needs_render(look));
    }

    #[test]
    fn packed_vertices_are_the_mesh_files_vertices() {
        let v = |x: f32, n: [f32; 3], c: Rgba| MeshVertex {
            position: [x, 2.0, -3.5],
            normal: n,
            colour: c,
        };
        let data = MeshData {
            vertices: vec![
                v(1.0, [0.0, 1.0, 0.0], Rgba(9, 8, 7, 255)),
                v(-4.25, [-0.6, 0.0, 0.8], Rgba(1, 2, 3, 4)),
                v(0.0, [0.0, 0.0, -1.0], Rgba(0, 0, 0, 0)),
            ],
            indices: vec![0, 1, 2],
        };
        let file = data.encode().unwrap();
        let packed = pack_vertices(&data);
        assert_eq!(packed.len(), 3 * MESH_VERTEX_BYTES);
        assert_eq!(&file[16..16 + packed.len()], &packed[..]);
    }
}
