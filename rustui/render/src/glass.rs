//! Glass surfaces on the GPU: how a `Glass` is cut into the pieces that draw it, which program draws each, and the
//! numbers they are given. The GLSL is in `shaders::glass`, the shadow meshes in `shadow`.
//!
//! Everything a shader is told is worked out here in layer pixels from the protocol's `Glass`, as plain data, so
//! that the rules (a radius past half the shorter side is pulled in, an edge of width 0 draws nothing, the sheen's
//! divisions are guarded, the pieces tile the body exactly) can be tested without a GPU.

use crate::feather;
use crate::geometry::{Panel, Placement, Vertex};
use crate::gl::{GL_BLEND, Gles};
use crate::profile::Skips;
use crate::shaders::fade;
use crate::shaders::glass::{self as shader, RimFeatures};
use crate::shadow::{self, Field};
use compositor_protocol::{Depth, Glass, LayerId, Rgba, Sheen};
use std::collections::HashMap;
use std::ffi::CStr;
use std::io;

/// A premultiplied colour as the shaders take it.
pub fn premultiplied(c: Rgba) -> [f32; 4] {
    [
        f32::from(c.0) / 255.0,
        f32::from(c.1) / 255.0,
        f32::from(c.2) / 255.0,
        f32::from(c.3) / 255.0,
    ]
}

fn scaled(c: Rgba, k: f32) -> [f32; 4] {
    premultiplied(c).map(|channel| channel * k)
}

/// The corner radius a shape can actually have: past half the shorter side its distance field would fold over itself.
pub fn effective_radius(radius: f32, width: f32, height: f32) -> f32 {
    radius.clamp(0.0, width.min(height) / 2.0)
}

/// A blur of width `blur` (CSS `box-shadow` meaning: a Gaussian of standard deviation `blur / 2`) also rounds the
/// corners of what it blurs: the radius of the rounded box whose soft edge stands in for the blurred shape.
fn blurred_radius(radius: f32, blur: f32, half_size: [f32; 2]) -> f32 {
    let sigma = blur / 2.0;
    (radius * radius + sigma * sigma)
        .sqrt()
        .min(half_size[0].min(half_size[1]))
}

fn has_light(depth: &Depth) -> bool {
    depth.light.3 > 0 || depth.shade.3 > 0
}

/// The raised control's light and shade copies, drawn outside the body; a well's inset is drawn by the rim.
fn raised_depth(glass: &Glass) -> Option<Depth> {
    glass.depth.filter(|d| !d.inset && has_light(d))
}

fn inset_depth(glass: &Glass) -> Option<Depth> {
    glass.depth.filter(|d| d.inset && has_light(d))
}

/// The shadows of `glass` as a mesh spec: what is cast, and how far out it reaches (left, top, right, bottom).
pub fn shadow_spec(glass: &Glass, size: (f32, f32)) -> Option<shadow::Spec> {
    let radius = effective_radius(glass.corner_radius, size.0, size.1);
    let half = [size.0 / 2.0, size.1 / 2.0];
    let mut reach = [0.0f32; 4];
    let mut fields = [None; 2];
    let casts = glass.shadows.iter().flatten().filter(|s| s.colour.3 > 0);
    for (slot, shadow) in fields.iter_mut().zip(casts) {
        *slot = Some(Field {
            shift: [0.0, shadow.offset_y],
            width: shadow.blur,
            radius: blurred_radius(radius, shadow.blur, half),
        });
        // Nothing is drawn past one blur width beyond the moved-down shape.
        reach[0] = reach[0].max(shadow.blur);
        reach[2] = reach[2].max(shadow.blur);
        reach[1] = reach[1].max(shadow.blur - shadow.offset_y);
        reach[3] = reach[3].max(shadow.blur + shadow.offset_y);
    }
    fields[0]?;
    Some(shadow::Spec {
        size,
        radius,
        reach,
        fields,
    })
}

/// A raised control's light copy (towards the upper left) and shade copy (towards the lower right).
pub fn raised_spec(glass: &Glass, size: (f32, f32)) -> Option<shadow::Spec> {
    let depth = raised_depth(glass)?;
    let radius = effective_radius(glass.corner_radius, size.0, size.1);
    let half = [size.0 / 2.0, size.1 / 2.0];
    let field = |sign: f32| Field {
        shift: [sign * depth.offset, sign * depth.offset],
        width: depth.blur,
        radius: blurred_radius(radius, depth.blur, half),
    };
    Some(shadow::Spec {
        size,
        radius,
        reach: [depth.offset + depth.blur; 4],
        fields: [Some(field(-1.0)), Some(field(1.0))],
    })
}

/// How far past the body's edges, in layer pixels, anything is drawn: left, top, right, bottom.
pub fn outside_reach(glass: &Glass, size: (f32, f32)) -> [f32; 4] {
    let mut reach = [0.0f32; 4];
    for spec in [shadow_spec(glass, size), raised_spec(glass, size)]
        .into_iter()
        .flatten()
    {
        for (all, this) in reach.iter_mut().zip(spec.reach) {
            *all = all.max(this);
        }
    }
    reach
}

/// What the rim program is compiled with for `glass`, given whether it will be shown a backdrop picture.
pub fn features(glass: &Glass, has_backdrop: bool) -> RimFeatures {
    RimFeatures {
        backdrop: has_backdrop,
        refraction: has_backdrop && glass.refraction > 0.0,
        inset: inset_depth(glass).is_some(),
    }
}

/// The numbers the core and the rim programs are given, in layer pixels and premultiplied colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceUniforms {
    pub radius: f32,
    pub tint: [f32; 4],
    pub edge: [f32; 4],
    pub edge_width: f32,
    pub highlight: [f32; 4],
    /// Peak, floor, start, end of the sheen; all zero for none.
    pub sheen: [f32; 4],
    /// Reciprocals of the sheen's two ramps, so that the shader multiplies.
    pub sheen_k: [f32; 2],
    /// Distance from a straight edge within which anything but the plain fill can show: the rim's thickness.
    pub inner: f32,
    pub saturation: f32,
    /// How far the backdrop is bent at the edge and over what distance.
    pub bend: [f32; 2],
    /// The inset's offset, the width of its soft edge and the corner radius of the blurred shape.
    pub inset: [f32; 4],
    pub inset_shade: [f32; 4],
    pub inset_light: [f32; 4],
}

impl SurfaceUniforms {
    /// `size` is the body in layer pixels; `pixel` is the size of one output pixel in layer pixels.
    pub fn new(glass: &Glass, size: (f32, f32), pixel: f32) -> Self {
        let radius = effective_radius(glass.corner_radius, size.0, size.1);
        let edge_width = glass.edge_width.max(0.0);
        let edge = if edge_width > 0.0 {
            premultiplied(glass.edge)
        } else {
            [0.0; 4]
        };
        let (sheen, sheen_k) = match glass.sheen {
            Some(Sheen {
                peak,
                floor,
                start,
                end,
            }) => (
                [peak, floor, start, end],
                // A ramp of no length is a step, which a huge slope makes.
                [1.0 / start.max(1.0e-4), 1.0 / (end - start).max(1.0e-4)],
            ),
            None => ([0.0, 0.0, 0.5, 1.0], [2.0, 2.0]),
        };
        // The edge line, the one-pixel highlight below it and a pixel of anti-aliasing.
        let mut inner = edge_width + 2.0;
        let bend = if glass.refraction > 0.0 {
            // Gone after three times the bend, and at least a few pixels: a smooth falloff needs room.
            [glass.refraction, (glass.refraction * 3.0).max(8.0)]
        } else {
            [0.0, 1.0]
        };
        if glass.refraction > 0.0 {
            inner = inner.max(bend[1]);
        }
        let (mut inset, mut inset_shade, mut inset_light) = ([0.0; 4], [0.0; 4], [0.0; 4]);
        if let Some(depth) = inset_depth(glass) {
            let pad_half = [size.0 / 2.0 - edge_width, size.1 / 2.0 - edge_width];
            let pad_radius = (radius - edge_width).max(0.0);
            inset = [
                depth.offset,
                depth.blur.max(0.0),
                blurred_radius(pad_radius, depth.blur, pad_half).max(0.0),
                0.0,
            ];
            inset_shade = premultiplied(depth.shade);
            inset_light = premultiplied(depth.light);
            inner = inner.max(edge_width + depth.offset + depth.blur + 1.0);
        }
        Self {
            radius,
            tint: premultiplied(glass.tint),
            edge,
            edge_width,
            highlight: premultiplied(glass.highlight),
            sheen,
            sheen_k,
            inner: inner + pixel,
            saturation: glass.saturation,
            bend,
            inset,
            inset_shade,
            inset_light,
        }
    }
}

/// A rectangle of the body, `[left, top, right, bottom]` in layer pixels.
pub type Rect = [f32; 4];

/// The body cut into the pieces that draw it: the interior for the cheap core program and the strips and corners
/// for the rim program, which between them tile the body exactly. The rim's outer sides reach `pad` past the body
/// for the anti-aliased fringe.
#[derive(Clone, Debug, PartialEq)]
pub struct Slices {
    pub core: Vec<Rect>,
    pub rim: Vec<Rect>,
}

pub fn slices(size: (f32, f32), radius: f32, inner: f32, pad: f32) -> Slices {
    let (w, h) = size;
    let corner = radius.max(inner).min(w / 2.0).min(h / 2.0);
    // A surface too small to have an interior is all rim.
    if inner >= w.min(h) / 2.0 || corner <= 0.0 {
        return Slices {
            core: Vec::new(),
            rim: vec![[-pad, -pad, w + pad, h + pad]],
        };
    }
    let (c, m) = (corner, inner.min(corner));
    let rim = vec![
        // The four corners, c square.
        [-pad, -pad, c, c],
        [w - c, -pad, w + pad, c],
        [-pad, h - c, c, h + pad],
        [w - c, h - c, w + pad, h + pad],
        // And the strips between them, m thick.
        [c, -pad, w - c, m],
        [c, h - m, w - c, h + pad],
        [-pad, c, m, h - c],
        [w - m, c, w + pad, h - c],
    ];
    let core = [
        [c, m, w - c, h - m],
        // Beside the centre, between the corners' inner edges and the strips: only when the corners are bigger
        // than the strips are thick.
        [m, c, c, h - c],
        [w - c, c, w - m, h - c],
    ];
    Slices {
        core: core
            .into_iter()
            .filter(|r| r[2] > r[0] && r[3] > r[1])
            .collect(),
        rim: rim
            .into_iter()
            .filter(|r| r[2] > r[0] && r[3] > r[1])
            .collect(),
    }
}

/// The largest a cell of a surface's interior is, in layer pixels, when its corners work out the sheen: the sheen's
/// ramps are hundreds of pixels long and change slope by a tenth of a level in a hundred, so between corners this far
/// apart they are as good as exact.
const CORE_CELL: f32 = 64.0;

/// `rects` cut into cells no larger than `cell` each way, which tile them exactly.
fn subdivided(rects: &[Rect], cell: f32) -> Vec<Rect> {
    let mut out = Vec::new();
    for r in rects {
        let across = ((r[2] - r[0]) / cell).ceil().max(1.0) as usize;
        let down = ((r[3] - r[1]) / cell).ceil().max(1.0) as usize;
        let at = |from: f32, to: f32, k: usize, n: usize| from + (to - from) * k as f32 / n as f32;
        for row in 0..down {
            for col in 0..across {
                out.push([
                    at(r[0], r[2], col, across),
                    at(r[1], r[3], row, down),
                    at(r[0], r[2], col + 1, across),
                    at(r[1], r[3], row + 1, down),
                ]);
            }
        }
    }
    out
}

/// The two triangles of a strip-ordered quad (top-left, top-right, bottom-left, bottom-right).
fn triangles(q: [Vertex; 4]) -> [Vertex; 6] {
    [q[0], q[1], q[2], q[1], q[3], q[2]]
}

/// A run of vertices in the frame's buffer.
#[derive(Clone, Copy, Debug)]
pub struct Span {
    pub first: i32,
    pub count: i32,
}

/// Where the pieces of one glass surface are in the frame's vertices.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    shadows: Option<Span>,
    raised: Option<Span>,
    core: Option<Span>,
    rim: Span,
}

impl Placed {
    /// How many vertices the pieces of the surface take: its shadows, a raised control's light and shade, its interior
    /// and its rim.
    pub fn vertex_counts(&self) -> [u32; 4] {
        let of = |span: Option<Span>| span.map_or(0, |s| s.count as u32);
        [
            of(self.shadows),
            of(self.raised),
            of(self.core),
            self.rim.count as u32,
        ]
    }
}

/// How a glass surface is drawn this frame.
#[derive(Clone, Copy, Debug)]
pub struct Draw {
    /// The body in layer pixels.
    pub size: (f32, f32),
    /// Output pixels per layer pixel: the layer's scale, times how much smaller than the panel the target is.
    pub scale: f32,
    pub opacity: f32,
    /// The panel position of the layer's corner, and panel pixels per layer pixel along its two axes.
    pub origin: [f32; 2],
    pub axes: [f32; 4],
    /// The texture of the blurred picture of what is below, if it has one.
    pub backdrop: Option<u32>,
    /// The fade of the surface's clip, when the surface reaches into it: every piece of the surface is then drawn by a program
    /// that has the fade, and what is not opaque where the fade is not 1 is blended. `None`: the programs as they ever were.
    pub feather: Option<feather::Uniforms>,
    /// Parts left out for the profiler (`profile`): none in ordinary use.
    pub skips: Skips,
}

struct CoreProgram {
    id: u32,
    backdrop: bool,
    /// Whether the fragment shader changes the backdrop's saturation.
    saturating: bool,
    /// Whether it has the fade of a feathered clip.
    feathered: bool,
    size: i32,
    opacity: i32,
    tint: i32,
    sheen: i32,
    sheen_k: i32,
    saturation: i32,
    feather: feather::Slot,
}

struct RimProgram {
    id: u32,
    features: RimFeatures,
    feathered: bool,
    size: i32,
    radius: i32,
    scale: i32,
    pixel: i32,
    opacity: i32,
    tint: i32,
    edge: i32,
    edge_width: i32,
    highlight: i32,
    sheen: i32,
    sheen_k: i32,
    saturation: i32,
    axes: i32,
    bend: i32,
    inset: i32,
    inset_shade: i32,
    inset_light: i32,
    feather: feather::Slot,
}

struct RingProgram {
    id: u32,
    origin: i32,
    axes: i32,
    colour_a: i32,
    colour_b: i32,
    feather: feather::Slot,
}

/// What shows a still surface's plate (`ideas::STILL_GLASS`) where the surface would have been drawn.
struct PlateProgram {
    id: u32,
}

/// A surface's shadow meshes: the shapes they were made for (when a surface changes shape they are looked for again) and the
/// meshes, which every surface of the same shape holds.
struct Meshes {
    specs: [Option<shadow::Spec>; 2],
    shadows: Option<shadow::Shared>,
    raised: Option<shadow::Shared>,
}

/// The glass programs and the shadow meshes of the surfaces on screen.
pub struct Surfaces {
    /// Every core and rim program twice: as it is, and with the fade of a feathered clip (`fade`; a surface that reaches into
    /// its clip's fade is drawn by the second).
    cores: Vec<CoreProgram>,
    rims: Vec<RimProgram>,
    ring: RingProgram,
    ring_feathered: RingProgram,
    plate: PlateProgram,
    meshes: HashMap<LayerId, Meshes>,
    /// The meshes that have been made, for the surfaces of one shape to share: a screen's controls are a handful of shapes.
    shapes: shadow::Cache,
}

fn loc(gl: &Gles, program: u32, name: &CStr) -> i32 {
    gl.uniform_location(program, name)
}

impl Surfaces {
    /// Compiles and links every variant now, so that nothing is compiled while the user is touching the screen.
    pub fn new(gl: &Gles, panel: &Panel) -> io::Result<Self> {
        let surface_vertex = shader::surface_vertex();
        let ndc = [2.0 / panel.width as f32, 2.0 / panel.height as f32];
        let inv_panel = [1.0 / panel.width as f32, 1.0 / panel.height as f32];
        let common = |program: u32| {
            gl.use_program(program);
            gl.uniform2f(loc(gl, program, c"u_ndc"), ndc[0], ndc[1]);
            gl.uniform2f(loc(gl, program, c"u_inv_panel"), inv_panel[0], inv_panel[1]);
            gl.uniform1i(loc(gl, program, c"u_backdrop"), 0);
        };
        // The fragment shader a program of the feathered half is made of: the plain one's twin.
        let twin = |fragment: std::ffi::CString, feathered: bool| {
            if feathered {
                fade::feathered(&fragment)
            } else {
                fragment
            }
        };
        let slot = |id: u32, feathered: bool| {
            if feathered {
                feather::Slot::new(loc(gl, id, c"u_feather"))
            } else {
                feather::Slot::none()
            }
        };
        let mut cores = Vec::new();
        let mut rims = Vec::new();
        for feathered in [false, true] {
            for (backdrop, saturating) in [(false, false), (true, false), (true, true)] {
                let id = gl.program(
                    &shader::core_vertex(backdrop),
                    &twin(shader::core(backdrop, saturating), feathered),
                )?;
                common(id);
                let at = |name: &CStr| loc(gl, id, name);
                cores.push(CoreProgram {
                    id,
                    backdrop,
                    saturating,
                    feathered,
                    size: at(c"u_size"),
                    opacity: at(c"u_opacity"),
                    tint: at(c"u_tint"),
                    sheen: at(c"u_sheen"),
                    sheen_k: at(c"u_sheen_k"),
                    saturation: at(c"u_saturation"),
                    feather: slot(id, feathered),
                });
            }
            for features in RimFeatures::ALL {
                let id = gl.program(&surface_vertex, &twin(shader::rim(features), feathered))?;
                common(id);
                let at = |name: &CStr| loc(gl, id, name);
                rims.push(RimProgram {
                    id,
                    features,
                    feathered,
                    size: at(c"u_size"),
                    radius: at(c"u_radius"),
                    scale: at(c"u_scale"),
                    pixel: at(c"u_pixel"),
                    opacity: at(c"u_opacity"),
                    tint: at(c"u_tint"),
                    edge: at(c"u_edge"),
                    edge_width: at(c"u_edge_width"),
                    highlight: at(c"u_highlight"),
                    sheen: at(c"u_sheen"),
                    sheen_k: at(c"u_sheen_k"),
                    saturation: at(c"u_saturation"),
                    axes: at(c"u_axes"),
                    bend: at(c"u_bend"),
                    inset: at(c"u_inset"),
                    inset_shade: at(c"u_inset_shade"),
                    inset_light: at(c"u_inset_light"),
                    feather: slot(id, feathered),
                });
            }
        }
        let ring_of = |feathered: bool| -> io::Result<RingProgram> {
            let id = gl.program(
                &shader::ring_vertex(),
                &twin(shader::ring_fragment(), feathered),
            )?;
            gl.use_program(id);
            gl.uniform2f(loc(gl, id, c"u_ndc"), ndc[0], ndc[1]);
            Ok(RingProgram {
                id,
                origin: loc(gl, id, c"u_origin"),
                axes: loc(gl, id, c"u_axes"),
                colour_a: loc(gl, id, c"u_colour_a"),
                colour_b: loc(gl, id, c"u_colour_b"),
                feather: slot(id, feathered),
            })
        };
        let ring = ring_of(false)?;
        let ring_feathered = ring_of(true)?;
        let id = gl.program(&shader::plate_vertex(), &shader::plate_fragment())?;
        gl.use_program(id);
        gl.uniform2f(loc(gl, id, c"u_ndc"), ndc[0], ndc[1]);
        gl.uniform2f(loc(gl, id, c"u_inv_panel"), inv_panel[0], inv_panel[1]);
        gl.uniform1i(loc(gl, id, c"u_plate"), 0);
        let plate = PlateProgram { id };
        Ok(Self {
            cores,
            rims,
            ring,
            ring_feathered,
            plate,
            meshes: HashMap::new(),
            shapes: shadow::Cache::default(),
        })
    }

    /// The layer is gone: so are its meshes.
    pub fn forget(&mut self, id: LayerId) {
        self.meshes.remove(&id);
    }

    pub fn forget_all(&mut self) {
        self.meshes.clear();
    }

    /// The shadow meshes made since the last call and the nanoseconds they took.
    pub fn take_built(&mut self) -> (u32, u64) {
        self.shapes.take_built()
    }

    /// The rectangle of a `glass` surface's layer that anything of it is drawn in, `[left, top, right, bottom]`:
    /// the body and how far the shadows reach past it, with the anti-aliased fringe.
    pub fn extent(glass: &Glass, size: (f32, f32), pad: f32) -> Rect {
        let reach = outside_reach(glass, size);
        [
            -reach[0] - pad,
            -reach[1] - pad,
            size.0 + reach[2] + pad,
            size.1 + reach[3] + pad,
        ]
    }

    /// Adds the vertices that draw `glass` (layer `id`, placed at `at`) to the frame's and says where they are.
    pub fn place(
        &mut self,
        id: LayerId,
        glass: &Glass,
        at: &Placement,
        panel: &Panel,
        pad: f32,
        vertices: &mut Vec<Vertex>,
    ) -> Placed {
        let size = (at.width, at.height);
        let specs = [shadow_spec(glass, size), raised_spec(glass, size)];
        let meshes = match self.meshes.get(&id) {
            Some(meshes) if meshes.specs == specs => meshes,
            _ => {
                let [shadows, raised] = specs.map(|spec| spec.map(|s| self.shapes.get(&s)));
                self.meshes.insert(
                    id,
                    Meshes {
                        specs,
                        shadows,
                        raised,
                    },
                );
                &self.meshes[&id]
            }
        };
        let span = |mesh: Option<&shadow::Shared>, vertices: &mut Vec<Vertex>| {
            mesh.filter(|mesh| !mesh.is_empty()).map(|mesh| {
                let span = Span {
                    first: vertices.len() as i32,
                    count: mesh.len() as i32,
                };
                vertices.extend_from_slice(mesh);
                span
            })
        };
        let shadows = span(meshes.shadows.as_ref(), vertices);
        let raised = span(meshes.raised.as_ref(), vertices);

        let uniforms = SurfaceUniforms::new(glass, size, pad);
        let pieces = slices(size, uniforms.radius, uniforms.inner, pad);
        let triangles_of = |rects: &[Rect], vertices: &mut Vec<Vertex>| {
            let first = vertices.len() as i32;
            for rect in rects {
                vertices.extend_from_slice(&triangles(panel.quad_of(at, *rect)));
            }
            Span {
                first,
                count: vertices.len() as i32 - first,
            }
        };
        // The sheen is worked out at the corners of the interior: cut into cells where there is one.
        let core_cells = if glass.sheen.is_some() {
            subdivided(&pieces.core, CORE_CELL)
        } else {
            pieces.core.clone()
        };
        let core = (!core_cells.is_empty()).then(|| triangles_of(&core_cells, vertices));
        let rim = triangles_of(&pieces.rim, vertices);
        Placed {
            shadows,
            raised,
            core,
            rim,
        }
    }

    /// Draws a surface `place` put in the frame: its shadows, then its body.
    pub fn draw(&self, gl: &Gles, glass: &Glass, placed: &Placed, draw: &Draw) {
        self.draw_shadows(gl, glass, placed, draw);
        self.draw_body(gl, glass, placed, draw);
    }

    /// Draws what a surface has outside its body: its soft shadows, and a raised control's light and shade. The meshes
    /// leave the body itself out, so a surface's own shadow is never seen under it.
    pub fn draw_shadows(&self, gl: &Gles, glass: &Glass, placed: &Placed, draw: &Draw) {
        let ring = if draw.feather.is_some() {
            &self.ring_feathered
        } else {
            &self.ring
        };
        let cast = |span: Option<Span>, a: Rgba, b: Rgba| {
            let Some(span) = span else { return };
            gl.use_program(ring.id);
            if let Some(fade) = &draw.feather {
                ring.feather.set(gl, fade);
            }
            gl.uniform2f(ring.origin, draw.origin[0], draw.origin[1]);
            let axes = draw.axes;
            gl.uniform4f(ring.axes, axes[0], axes[1], axes[2], axes[3]);
            let (a, b) = (scaled(a, draw.opacity), scaled(b, draw.opacity));
            gl.uniform4f(ring.colour_a, a[0], a[1], a[2], a[3]);
            gl.uniform4f(ring.colour_b, b[0], b[1], b[2], b[3]);
            gl.draw_triangle_strip(span.first, span.count);
        };
        let colours = |shadow: Option<compositor_protocol::Shadow>| {
            shadow.map_or(Rgba(0, 0, 0, 0), |s| s.colour)
        };
        let casting: Vec<_> = glass
            .shadows
            .iter()
            .flatten()
            .filter(|s| s.colour.3 > 0)
            .collect();
        if !draw.skips.shadows {
            cast(
                placed.shadows,
                colours(casting.first().map(|s| **s)),
                colours(casting.get(1).map(|s| **s)),
            );
        }
        if let Some(depth) = raised_depth(glass).filter(|_| !draw.skips.raised) {
            cast(placed.raised, depth.light, depth.shade);
        }
    }

    /// Draws a surface's body: the interior, then the rim.
    pub fn draw_body(&self, gl: &Gles, glass: &Glass, placed: &Placed, draw: &Draw) {
        if let Some(texture) = draw.backdrop {
            gl.active_texture(0);
            gl.bind_texture(texture);
        }
        let uniforms = SurfaceUniforms::new(glass, draw.size, 1.0 / draw.scale);
        let has_backdrop = draw.backdrop.is_some();
        let faded = draw.feather.is_some();
        let v4 = |location: i32, v: [f32; 4]| gl.uniform4f(location, v[0], v[1], v[2], v[3]);
        if let Some(span) = placed.core.filter(|_| !draw.skips.core) {
            let saturating = has_backdrop && uniforms.saturation != 1.0;
            let core = self
                .cores
                .iter()
                .find(|c| {
                    c.backdrop == has_backdrop && c.saturating == saturating && c.feathered == faded
                })
                .expect("every combination of the core's features is compiled");
            gl.use_program(core.id);
            if let Some(fade) = &draw.feather {
                core.feather.set(gl, fade);
            }
            gl.uniform2f(core.size, draw.size.0, draw.size.1);
            gl.uniform1f(core.opacity, draw.opacity);
            v4(core.tint, uniforms.tint);
            v4(core.sheen, uniforms.sheen);
            gl.uniform2f(core.sheen_k, uniforms.sheen_k[0], uniforms.sheen_k[1]);
            if saturating {
                gl.uniform1f(core.saturation, uniforms.saturation);
            }
            // Over a backdrop the interior is opaque, and what is under it is not read: blending costs the GPU a read
            // of every pixel it writes. Where the clip fades it out it is not opaque.
            let opaque = has_backdrop && draw.opacity >= 1.0 && !faded;
            if opaque {
                gl.disable(GL_BLEND);
            }
            gl.draw_triangles(span.first, span.count);
            if opaque {
                gl.enable(GL_BLEND);
            }
        }
        if draw.skips.rim {
            return;
        }
        let wanted = features(glass, has_backdrop);
        let rim = self
            .rims
            .iter()
            .find(|r| r.features == wanted && r.feathered == faded)
            .expect("every combination of features is compiled");
        gl.use_program(rim.id);
        if let Some(fade) = &draw.feather {
            rim.feather.set(gl, fade);
        }
        gl.uniform2f(rim.size, draw.size.0, draw.size.1);
        gl.uniform1f(rim.radius, uniforms.radius);
        gl.uniform1f(rim.scale, draw.scale);
        gl.uniform1f(rim.pixel, 1.0 / draw.scale);
        gl.uniform1f(rim.opacity, draw.opacity);
        v4(rim.tint, uniforms.tint);
        v4(rim.edge, uniforms.edge);
        gl.uniform1f(rim.edge_width, uniforms.edge_width);
        v4(rim.highlight, uniforms.highlight);
        v4(rim.sheen, uniforms.sheen);
        gl.uniform2f(rim.sheen_k, uniforms.sheen_k[0], uniforms.sheen_k[1]);
        gl.uniform1f(rim.saturation, uniforms.saturation);
        v4(rim.axes, draw.axes);
        gl.uniform2f(rim.bend, uniforms.bend[0], uniforms.bend[1]);
        v4(rim.inset, uniforms.inset);
        v4(rim.inset_shade, uniforms.inset_shade);
        v4(rim.inset_light, uniforms.inset_light);
        gl.draw_triangles(placed.rim.first, placed.rim.count);
    }

    /// Draws a surface's body from its plate (`ideas::STILL_GLASS`) where `draw_body` would have drawn it: the interior, opaque
    /// where it was drawn opaque, and then the rim. The pieces are the ones the body is drawn in, each pixel read back from the
    /// plate at the panel pixel it lands on.
    pub fn blit_body(&self, gl: &Gles, placed: &Placed, opaque: bool, plate: u32) {
        gl.use_program(self.plate.id);
        gl.active_texture(0);
        gl.bind_texture(plate);
        if let Some(span) = placed.core {
            if opaque {
                gl.disable(GL_BLEND);
            }
            gl.draw_triangles(span.first, span.count);
            if opaque {
                gl.enable(GL_BLEND);
            }
        }
        gl.draw_triangles(placed.rim.first, placed.rim.count);
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_glass_program_with_the_fade_has_one_uniform_more_than_the_program_it_is_the_twin_of() {
        let Some((gl, _serial)) = crate::testgl::soft_gl() else {
            return;
        };
        let surfaces = Surfaces::new(&gl, &crate::testgl::echo_show_panel()).expect("the programs");
        let plain_cores: Vec<_> = surfaces.cores.iter().filter(|c| !c.feathered).collect();
        assert_eq!(plain_cores.len(), 3);
        for plain in plain_cores {
            let twin = surfaces
                .cores
                .iter()
                .find(|c| {
                    c.feathered && c.backdrop == plain.backdrop && c.saturating == plain.saturating
                })
                .expect("every core has a twin");
            assert_eq!(
                gl.active_uniforms(twin.id),
                gl.active_uniforms(plain.id) + 1
            );
        }
        let plain_rims: Vec<_> = surfaces.rims.iter().filter(|r| !r.feathered).collect();
        assert_eq!(plain_rims.len(), RimFeatures::ALL.len());
        for plain in plain_rims {
            let twin = surfaces
                .rims
                .iter()
                .find(|r| r.feathered && r.features == plain.features)
                .expect("every rim has a twin");
            assert_eq!(
                gl.active_uniforms(twin.id),
                gl.active_uniforms(plain.id) + 1
            );
        }
        assert_eq!(
            gl.active_uniforms(surfaces.ring_feathered.id),
            gl.active_uniforms(surfaces.ring.id) + 1
        );
    }
    use super::*;
    use compositor_protocol::{Backdrop, Shadow};

    fn bare() -> Glass {
        Glass {
            corner_radius: 20.0,
            tint: Rgba(10, 20, 30, 100),
            edge: Rgba(50, 50, 50, 50),
            edge_width: 1.0,
            highlight: Rgba(0, 0, 0, 0),
            sheen: None,
            shadows: [None, None],
            depth: None,
            backdrop: Backdrop::None,
            saturation: 1.0,
            refraction: 0.0,
        }
    }

    fn shadow(offset_y: f32, blur: f32) -> Option<Shadow> {
        Some(Shadow {
            offset_y,
            blur,
            colour: Rgba(0, 0, 0, 80),
        })
    }

    fn depth(inset: bool) -> Depth {
        Depth {
            inset,
            offset: 2.0,
            blur: 4.0,
            light: Rgba(30, 30, 30, 30),
            shade: Rgba(0, 0, 0, 80),
        }
    }

    const SIZE: (f32, f32) = (300.0, 150.0);

    #[test]
    fn a_surface_without_shadows_or_raised_copies_has_nothing_outside_its_body() {
        assert_eq!(outside_reach(&bare(), SIZE), [0.0; 4]);
        let inset = Glass {
            depth: Some(depth(true)),
            ..bare()
        };
        assert_eq!(
            outside_reach(&inset, SIZE),
            [0.0; 4],
            "a well is drawn inside"
        );
        let invisible = Glass {
            shadows: [
                shadow(10.0, 20.0).map(|s| Shadow {
                    colour: Rgba(0, 0, 0, 0),
                    ..s
                }),
                None,
            ],
            ..bare()
        };
        assert_eq!(outside_reach(&invisible, SIZE), [0.0; 4]);
        assert!(shadow_spec(&invisible, SIZE).is_none());
        assert!(raised_spec(&bare(), SIZE).is_none());
    }

    #[test]
    fn a_shadow_reaches_a_blur_width_past_the_shape_moved_down() {
        let glass = Glass {
            shadows: [shadow(24.0, 60.0), None],
            ..bare()
        };
        // Sideways one blur; above it one blur less the offset; below one blur more.
        assert_eq!(outside_reach(&glass, SIZE), [60.0, 36.0, 60.0, 84.0]);
        // A shadow that is cast upwards reaches further above than below.
        let up = Glass {
            shadows: [shadow(-10.0, 30.0), None],
            ..bare()
        };
        assert_eq!(outside_reach(&up, SIZE), [30.0, 40.0, 30.0, 20.0]);
    }

    #[test]
    fn several_shadows_and_raised_copies_add_up_to_the_widest_reach_on_each_side() {
        let glass = Glass {
            shadows: [shadow(34.0, 80.0), shadow(10.0, 24.0)],
            depth: Some(depth(false)),
            ..bare()
        };
        // The big shadow decides on every side; the raised copies (2 + 4) never reach as far.
        assert_eq!(outside_reach(&glass, SIZE), [80.0, 46.0, 80.0, 114.0]);
        let small = Glass {
            depth: Some(depth(false)),
            ..bare()
        };
        assert_eq!(outside_reach(&small, SIZE), [6.0; 4]);
        // Two shadows are two fields of one mesh; the raised copies are the light and the shade of another.
        let spec = shadow_spec(&glass, SIZE).unwrap();
        assert!(spec.fields.iter().all(Option::is_some));
        let raised = raised_spec(&glass, SIZE).unwrap();
        assert_eq!(raised.fields[0].unwrap().shift, [-2.0, -2.0]);
        assert_eq!(raised.fields[1].unwrap().shift, [2.0, 2.0]);
    }

    #[test]
    fn a_radius_past_half_the_shorter_side_is_pulled_in() {
        let glass = Glass {
            corner_radius: 999.0,
            ..bare()
        };
        assert_eq!(
            SurfaceUniforms::new(&glass, (200.0, 40.0), 1.0).radius,
            20.0
        );
        assert_eq!(effective_radius(-3.0, 10.0, 10.0), 0.0);
    }

    #[test]
    fn an_edge_of_no_width_draws_no_edge() {
        let glass = Glass {
            edge_width: 0.0,
            ..bare()
        };
        assert_eq!(
            SurfaceUniforms::new(&glass, (100.0, 50.0), 1.0).edge,
            [0.0; 4]
        );
        assert_ne!(
            SurfaceUniforms::new(&bare(), (100.0, 50.0), 1.0).edge,
            [0.0; 4]
        );
    }

    #[test]
    fn the_sheen_ramps_are_guarded_against_division_by_nothing() {
        let sheen = |start: f32, end: f32| Glass {
            sheen: Some(Sheen {
                peak: 0.2,
                floor: 0.1,
                start,
                end,
            }),
            ..bare()
        };
        let u = SurfaceUniforms::new(&sheen(0.38, 0.6), (100.0, 50.0), 1.0);
        assert!((u.sheen_k[0] - 1.0 / 0.38).abs() < 1e-5);
        assert!((u.sheen_k[1] - 1.0 / 0.22).abs() < 1e-4);
        for (start, end) in [(0.0, 0.0), (0.5, 0.5), (0.0, 0.3)] {
            let k = SurfaceUniforms::new(&sheen(start, end), (100.0, 50.0), 1.0).sheen_k;
            assert!(k.iter().all(|k| k.is_finite() && *k > 0.0), "{start} {end}");
        }
        // No sheen: nothing to add whatever the position.
        let none = SurfaceUniforms::new(&bare(), (100.0, 50.0), 1.0);
        assert_eq!((none.sheen[0], none.sheen[1]), (0.0, 0.0));
    }

    #[test]
    fn the_shader_looks_at_the_outline_as_far_in_as_anything_is_drawn_there() {
        let plain = SurfaceUniforms::new(&bare(), (200.0, 100.0), 1.0).inner;
        // The edge line, the one-pixel highlight, and a pixel of anti-aliasing.
        assert_eq!(plain, 1.0 + 2.0 + 1.0);
        let bent = Glass {
            refraction: 10.0,
            ..bare()
        };
        let u = SurfaceUniforms::new(&bent, (200.0, 100.0), 1.0);
        assert_eq!(u.bend, [10.0, 30.0]);
        assert!(u.inner >= 30.0);
        let well = Glass {
            depth: Some(depth(true)),
            ..bare()
        };
        // The inset reaches its offset plus its blur past the edge line.
        assert!(SurfaceUniforms::new(&well, (200.0, 100.0), 1.0).inner >= 1.0 + 2.0 + 4.0);
    }

    #[test]
    fn the_program_variant_follows_what_the_surface_and_its_backdrop_ask_for() {
        let bent = Glass {
            refraction: 5.0,
            ..bare()
        };
        let well = Glass {
            depth: Some(depth(true)),
            ..bare()
        };
        let raised = Glass {
            depth: Some(depth(false)),
            ..bare()
        };
        let f = |g: &Glass, backdrop| {
            let f = features(g, backdrop);
            (f.backdrop, f.refraction, f.inset)
        };
        assert_eq!(f(&bare(), false), (false, false, false));
        assert_eq!(f(&bare(), true), (true, false, false));
        // A bend needs something to bend.
        assert_eq!(f(&bent, false), (false, false, false));
        assert_eq!(f(&bent, true), (true, true, false));
        assert_eq!(f(&well, true), (true, false, true));
        // A raised control's copies are drawn outside, not by the rim.
        assert_eq!(f(&raised, true), (true, false, false));
        for glass in [bare(), bent, well, raised] {
            for backdrop in [false, true] {
                assert!(RimFeatures::ALL.contains(&features(&glass, backdrop)));
            }
        }
    }

    fn area(rects: &[Rect]) -> f32 {
        rects.iter().map(|r| (r[2] - r[0]) * (r[3] - r[1])).sum()
    }

    fn overlap(a: &Rect, b: &Rect) -> f32 {
        let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
        let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
        w * h
    }

    #[test]
    fn the_core_and_the_rim_tile_the_body_exactly_and_the_rim_reaches_past_it_by_the_fringe() {
        for (size, radius, inner) in [
            ((288.0, 150.0), 24.0, 4.0),
            ((912.0, 56.0), 28.0, 4.0),
            ((288.0, 150.0), 24.0, 30.0),
            ((100.0, 100.0), 4.0, 8.0),
            ((96.0, 96.0), 20.0, 9.0),
            ((300.0, 40.0), 0.0, 3.0),
        ] {
            let pad = 1.0;
            let pieces = slices(size, radius, inner, pad);
            let body = [0.0, 0.0, size.0, size.1];
            let inside = |r: &Rect| {
                [
                    r[0].max(0.0),
                    r[1].max(0.0),
                    r[2].min(size.0),
                    r[3].min(size.1),
                ]
            };
            let all: Vec<Rect> = pieces.core.iter().chain(&pieces.rim).map(inside).collect();
            // Together they cover the body once: the areas add up to it and no two overlap.
            assert!(
                (area(&all) - size.0 * size.1).abs() < 1e-2,
                "{size:?} {radius} {inner}: {} of {}",
                area(&all),
                size.0 * size.1
            );
            for (i, a) in all.iter().enumerate() {
                for b in &all[i + 1..] {
                    assert!(overlap(a, b) < 1e-3, "{size:?}: {a:?} and {b:?}");
                }
            }
            // The whole outline is the rim's, out to the fringe.
            let rim_top = pieces.rim.iter().map(|r| r[1]).fold(f32::MAX, f32::min);
            let rim_right = pieces.rim.iter().map(|r| r[2]).fold(f32::MIN, f32::max);
            assert_eq!((rim_top, rim_right), (-pad, size.0 + pad));
            // The core stays inside the body and away from every straight edge by the rim's thickness.
            for r in &pieces.core {
                assert!(
                    r[0] >= inner.min(size.0) - 1e-3 || r[1] >= inner - 1e-3,
                    "{r:?}"
                );
                assert!(r[0] >= 0.0 && r[1] >= 0.0 && r[2] <= body[2] && r[3] <= body[3]);
            }
        }
    }

    #[test]
    fn the_interior_is_cut_into_cells_that_tile_it_exactly_and_are_no_larger_than_asked() {
        let rects = [[10.0, 5.0, 210.0, 105.0], [0.0, 0.0, 30.0, 40.0]];
        let cells = subdivided(&rects, 64.0);
        // 200 by 100 is four by two cells of 50 by 50; the small one needs no cutting.
        assert_eq!(cells.len(), 4 * 2 + 1);
        let area = |r: &Rect| (r[2] - r[0]) * (r[3] - r[1]);
        let total: f32 = cells.iter().map(area).sum();
        let wanted: f32 = rects.iter().map(area).sum();
        assert!((total - wanted).abs() < 1e-2, "{total} against {wanted}");
        assert!(
            cells
                .iter()
                .all(|c| c[2] - c[0] <= 64.0 + 1e-3 && c[3] - c[1] <= 64.0 + 1e-3)
        );
        // Neighbours meet edge to edge: no cell's right edge falls short of the next one's left.
        let mut lefts: Vec<f32> = cells[..8].iter().map(|c| c[0]).collect();
        lefts.sort_by(f32::total_cmp);
        lefts.dedup();
        assert_eq!(lefts.len(), 4);
        for pair in cells[..4].windows(2) {
            assert_eq!(pair[0][2], pair[1][0]);
        }
        // An empty set of rectangles has no cells.
        assert!(subdivided(&[], 64.0).is_empty());
    }

    #[test]
    fn a_surface_too_small_for_an_interior_is_all_rim() {
        let pieces = slices((44.0, 44.0), 14.0, 22.0, 1.0);
        assert!(pieces.core.is_empty());
        assert_eq!(pieces.rim, vec![[-1.0, -1.0, 45.0, 45.0]]);
        // No rounded corners and a thin rim still leaves a core.
        assert!(!slices((44.0, 44.0), 0.0, 3.0, 1.0).core.is_empty());
    }

    #[test]
    fn a_big_card_is_mostly_core_and_a_thin_strip_mostly_rim() {
        let card = slices((288.0, 150.0), 24.0, 4.0, 1.0);
        let core = area(&card.core);
        assert!(core / (288.0 * 150.0) > 0.8, "{core}");
        let strip = slices((912.0, 56.0), 28.0, 4.0, 1.0);
        assert!(area(&strip.core) / (912.0 * 56.0) > 0.3);
    }

    // --- Drawing: the whole surface, read back from a software GL. ---

    use crate::testgl::{Shot, drawn, rig};
    use compositor_protocol::{Content, Fill, Props, Shape};

    /// A surface that draws nothing until a test turns something on.
    fn plain() -> Glass {
        Glass {
            corner_radius: 0.0,
            tint: Rgba(0, 0, 0, 0),
            edge: Rgba(0, 0, 0, 0),
            edge_width: 0.0,
            highlight: Rgba(0, 0, 0, 0),
            sheen: None,
            shadows: [None, None],
            depth: None,
            backdrop: Backdrop::None,
            saturation: 1.0,
            refraction: 0.0,
        }
    }

    fn solid(colour: Rgba) -> Content {
        Content::Shape(Shape {
            corner_radius: 0.0,
            fill: Fill::Solid(colour),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        })
    }

    /// The whole screen in one colour, under whatever a test draws.
    fn ground(colour: Rgba) -> crate::scene::Drawn {
        drawn(1, (960, 480), solid(colour), Props::at(0.0, 0.0))
    }

    fn surface(glass: Glass, size: (u32, u32), props: Props) -> crate::scene::Drawn {
        drawn(2, size, Content::Glass(glass), props)
    }

    fn assert_close(shot: &Shot, at: (u32, u32), expected: [u8; 4], tolerance: u8) {
        let got = shot.pixel(at.0, at.1);
        let close = got
            .iter()
            .zip(expected)
            .all(|(g, e)| g.abs_diff(e) <= tolerance);
        assert!(
            close,
            "pixel {at:?} is {got:?}, expected {expected:?} +-{tolerance}"
        );
    }

    const WHITE: Rgba = Rgba(255, 255, 255, 255);

    #[test]
    fn the_tint_goes_over_what_is_below_and_the_shadow_stays_outside_the_body() {
        let Some(mut rig) = rig() else { return };
        let glass = Glass {
            tint: Rgba(0, 0, 60, 120),
            shadows: [
                Some(Shadow {
                    offset_y: 20.0,
                    blur: 40.0,
                    colour: Rgba(0, 0, 0, 200),
                }),
                None,
            ],
            ..plain()
        };
        let shot = rig.draw(&[
            ground(WHITE),
            surface(glass, (300, 150), Props::at(100.0, 100.0)),
        ]);
        shot.save("glass-tint-and-shadow");
        // Blue tint at 120/255 over white: 135 in red and green, 195 in blue. The pixel two rows above the body's
        // bottom edge sits well inside the shadow's reach, and shows none of it: an outer shadow is clipped to
        // outside the box.
        assert_close(&shot, (250, 175), [135, 135, 195, 255], 2);
        assert_close(&shot, (250, 248), [135, 135, 195, 255], 2);
        // Twelve rows below the body: the shadow's edge is 8 pixels inside the moved-down shape, so 0.648 of the
        // colour's 200/255 opacity: 255 * (1 - 0.648 * 0.784) = 125.
        assert_close(&shot, (250, 262), [125, 125, 125, 255], 6);
        // The shadow is cast down: at the same distance above the body it is almost gone, and far away it is not there.
        assert_close(&shot, (250, 90), [246, 246, 246, 255], 6);
        assert_close(&shot, (250, 400), [255, 255, 255, 255], 0);
    }

    #[test]
    fn the_edge_is_the_outermost_rows_the_highlight_the_row_below_them_and_corners_are_cut() {
        let Some(mut rig) = rig() else { return };
        let glass = Glass {
            corner_radius: 20.0,
            edge: Rgba(255, 0, 0, 255),
            edge_width: 2.0,
            highlight: Rgba(0, 255, 0, 255),
            ..plain()
        };
        let shot = rig.draw(&[surface(glass, (200, 100), Props::at(100.0, 100.0))]);
        shot.save("glass-edge-and-highlight");
        let (red, green, black) = ([255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 0, 255]);
        // Top: two rows of edge, then one row of highlight, then nothing (the tint is clear, the ground black).
        assert_close(&shot, (200, 100), red, 2);
        assert_close(&shot, (200, 101), red, 2);
        assert_close(&shot, (200, 102), green, 2);
        assert_close(&shot, (200, 103), black, 2);
        // The left and bottom edges have the line but no highlight: it is the lip of light on the top only.
        assert_close(&shot, (100, 150), red, 2);
        assert_close(&shot, (101, 150), red, 2);
        assert_close(&shot, (102, 150), black, 2);
        assert_close(&shot, (200, 199), red, 2);
        assert_close(&shot, (200, 197), black, 2);
        // The corner of the box is cut away by the radius; a pixel inside the arc is there.
        assert_close(&shot, (103, 103), black, 2);
        assert_close(&shot, (108, 108), black, 2);
        assert_close(&shot, (101, 121), red, 40);
    }

    #[test]
    fn the_sheen_is_brightest_at_the_upper_left_and_gone_by_its_end() {
        let Some(mut rig) = rig() else { return };
        let glass = Glass {
            sheen: Some(Sheen {
                peak: 0.4,
                floor: 0.2,
                start: 0.3,
                end: 0.6,
            }),
            ..plain()
        };
        let shot = rig.draw(&[surface(glass, (400, 200), Props::at(100.0, 100.0))]);
        shot.save("glass-sheen");
        // The position along the diagonal is (x + y) / (width + height) of the layer. White light over black at alpha
        // 0.4 easing to 0.2 at 0.3 and to nothing at 0.6. At (5, 5): 0.4 - 0.2 * (10/600) / 0.3 = 0.389.
        assert_close(&shot, (105, 105), [99, 99, 99, 255], 4);
        // At (200, 100): t = 0.5, two thirds of the way from the floor to nothing: 0.2 / 3 = 0.067.
        assert_close(&shot, (300, 200), [17, 17, 17, 255], 4);
        // Past the end, and in the far corner, none.
        assert_close(&shot, (490, 290), [0, 0, 0, 255], 1);
        // Along the anti-diagonal the light is the same: it is a function of x + y, at 135 degrees.
        let a = shot.pixel(150, 250);
        let b = shot.pixel(250, 150);
        assert!(a[0].abs_diff(b[0]) <= 1, "{a:?} {b:?}");
    }

    #[test]
    fn opacity_fades_the_whole_surface_and_scale_shrinks_it_about_its_centre() {
        let Some(mut rig) = rig() else { return };
        let glass = Glass {
            tint: Rgba(0, 0, 0, 255),
            ..plain()
        };
        let half = Props {
            opacity: 0.5,
            ..Props::at(100.0, 100.0)
        };
        let shot = rig.draw(&[ground(WHITE), surface(glass, (200, 100), half)]);
        assert_close(&shot, (200, 150), [128, 128, 128, 255], 2);
        // At half scale the 200 x 100 body covers x 150..250, y 125..175.
        let small = Props {
            scale: 0.5,
            ..Props::at(100.0, 100.0)
        };
        let shot = rig.draw(&[ground(WHITE), surface(glass, (200, 100), small)]);
        assert_close(&shot, (200, 150), [0, 0, 0, 255], 0);
        assert_close(&shot, (155, 130), [0, 0, 0, 255], 0);
        assert_close(&shot, (140, 150), [255, 255, 255, 255], 0);
        assert_close(&shot, (200, 190), [255, 255, 255, 255], 0);
    }

    #[test]
    fn a_well_is_shaded_at_its_upper_left_inside_and_lit_at_its_lower_right() {
        let Some(mut rig) = rig() else { return };
        let glass = Glass {
            corner_radius: 14.0,
            tint: Rgba(128, 128, 128, 255),
            depth: Some(Depth {
                inset: true,
                offset: 2.0,
                blur: 4.0,
                light: Rgba(255, 255, 255, 255),
                shade: Rgba(0, 0, 0, 255),
            }),
            ..plain()
        };
        let shot = rig.draw(&[surface(glass, (200, 60), Props::at(100.0, 100.0))]);
        shot.save("glass-well");
        assert_close(&shot, (200, 130), [128, 128, 128, 255], 1);
        // Just inside the top edge and the left edge: dark. Just inside the bottom and right edges: light.
        assert!(shot.pixel(200, 101)[0] < 60, "{:?}", shot.pixel(200, 101));
        assert!(shot.pixel(101, 130)[0] < 60, "{:?}", shot.pixel(101, 130));
        assert!(shot.pixel(200, 158)[0] > 200, "{:?}", shot.pixel(200, 158));
        assert!(shot.pixel(298, 130)[0] > 200, "{:?}", shot.pixel(298, 130));
        // And the effect is only a few pixels deep.
        assert_close(&shot, (200, 112), [128, 128, 128, 255], 3);
    }

    #[test]
    fn a_raised_control_is_lit_on_the_upper_left_outside_and_shaded_on_the_lower_right() {
        let Some(mut rig) = rig() else { return };
        let glass = Glass {
            corner_radius: 14.0,
            tint: Rgba(40, 40, 40, 255),
            depth: Some(Depth {
                inset: false,
                offset: 2.0,
                blur: 4.0,
                light: Rgba(255, 255, 255, 255),
                shade: Rgba(0, 0, 0, 255),
            }),
            ..plain()
        };
        let shot = rig.draw(&[
            ground(Rgba(128, 128, 128, 255)),
            surface(glass, (200, 60), Props::at(100.0, 100.0)),
        ]);
        shot.save("glass-raised");
        // Outside the left and top edges the light copy brightens the ground, outside the right and bottom the shade
        // copy darkens it; the body itself keeps its own colour.
        assert!(shot.pixel(99, 130)[0] > 150, "{:?}", shot.pixel(99, 130));
        assert!(shot.pixel(200, 99)[0] > 150, "{:?}", shot.pixel(200, 99));
        assert!(shot.pixel(300, 130)[0] < 100, "{:?}", shot.pixel(300, 130));
        assert!(shot.pixel(200, 160)[0] < 100, "{:?}", shot.pixel(200, 160));
        assert_close(&shot, (200, 130), [40, 40, 40, 255], 1);
        // Six pixels out it is over: the ground is as it was.
        assert_close(&shot, (90, 130), [128, 128, 128, 255], 1);
    }
}
