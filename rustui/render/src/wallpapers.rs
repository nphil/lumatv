//! Wallpapers on the GPU: the meshes and programs that draw each `WallpaperKind`, and the numbers they are given. How a
//! wallpaper keeps time (steps, `fps`, taps) is `wallpaper`; the GLSL is `shaders::wallpapers`, whose top explains why
//! every kind is drawn the way it is.
//!
//! A wallpaper is a *base* (silk, aurora, liquid, the night sky: covers the layer, so at full opacity it is drawn
//! without blending) or an *overlay* (fireflies, weather: transparent between its particles, drawn over what is below).
//! A cached backdrop is a picture of the layers *at rest* (`renderer::View::Rest`): a base is then drawn still, with
//! no motion and no taps, and an overlay is left out, because a snapshot that kept a raindrop would keep it for good.
//!
//! Everything a kind draws is made once, at start, into vertex buffers that never change: grids for the fields, sprites
//! for the particles, discs for the glows. A frame only sets uniforms and draws.

use crate::geometry::Vertex;
use crate::gl::{GL_BLEND, Gles};
use crate::shaders::wallpapers as shader;
use crate::wallpaper::{MAX_RIPPLES, Ripples};
use compositor_protocol::{Rgba, Wallpaper, WallpaperKind, Weather};
use std::collections::HashMap;
use std::f32::consts::TAU;
use std::ffi::{CStr, CString};
use std::io;

/// One loop of the clock the shaders are given as `u_time`: whatever repeats does so a whole number of times in it, so
/// that a clock that has run for weeks can be given to them as a small number without a jump.
pub const LOOP_S: f64 = 300.0;

/// The slower clock (`u_slow`), for what turns in tens of minutes: the sky, the drift of the clouds. A whole number of
/// [`LOOP_S`], so that both clocks wrap together.
pub const SLOW_LOOP_S: f64 = 1800.0;

// LumaGlass's silk (`theme.json`, `wallpaperMotion`), as designed: 4 px of 1920 of drift, a quarter of depth, a third
// of detail, waves at their natural size, and the dark material's scrim.
const SILK_AMPLITUDE: f32 = 4.0 / 1920.0;
const SILK_DEPTH: f32 = 0.25;
const SILK_DETAIL: f32 = 0.35;
const SILK_SCALE: f32 = 1.0;
const SILK_SCRIM: [f32; 3] = [0.10, 0.18, 0.46];

// The water's swell is the silk's warp, wider and gentler on the light; a ripple's numbers are in the shader.
const LIQUID_AMPLITUDE: f32 = 7.0 / 1920.0;
const LIQUID_DEPTH: f32 = 0.10;
const LIQUID_DETAIL: f32 = 0.25;
/// The height of a ripple, in pixels, the shift of the picture it makes per unit of slope, and how much the slopes
/// facing the light are brightened.
const LIQUID_WAVE: [f32; 3] = [16.0, 5.0, 0.5];

/// How much of the aurora's colours a cloud's centre shows.
const AURORA_STRENGTH: f32 = 0.55;

const STARS: usize = 520;
const FIREFLIES: usize = 64;
const RAINDROPS: usize = 300;
const STORM_DROPS: usize = 380;
const SNOWFLAKES: usize = 240;
const RAYS: usize = 9;
const PUFFS: usize = 8;
const PUFF_RINGS: usize = 7;
const PUFF_SEGMENTS: usize = 24;

/// The cells of the grids, across and down. The silk's has a row exactly on the scrim's middle stop (0.55 of the
/// height) and cells 20 by 24 px on the Show; the water's is finer where a ripple's crests, 76 px apart, need it.
const SILK_GRID: (usize, usize) = (48, 20);
const COARSE_GRID: (usize, usize) = (32, 16);
const FINE_GRID: (usize, usize) = (80, 40);

/// The optional extras of LumaGlass's silk (`wallpaperMotion.vignette`, `.specular`, `.bloom`): all off unless asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    /// How much the corners are darkened, 0 to 1.
    pub vignette: f32,
    /// How much the crests of the folds gleam, 0 to 1.
    pub specular: f32,
    /// How much the brightest parts of the picture glow, 0 to 1.
    pub bloom: f32,
}

/// Home's controls in theme units, applied only to its named Silk layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Silk {
    pub amplitude: f32,
    pub depth: f32,
    pub scale: f32,
    pub detail: f32,
    pub look: Look,
}

impl Default for Silk {
    fn default() -> Self {
        Self { amplitude: 4.0, depth: SILK_DEPTH, scale: SILK_SCALE, detail: SILK_DETAIL, look: Look::default() }
    }
}

/// Whether a kind is an overlay: it draws over what is below instead of covering it.
pub fn is_overlay(kind: &WallpaperKind) -> bool {
    matches!(
        kind,
        WallpaperKind::Fireflies { .. } | WallpaperKind::Weather(_)
    )
}

/// Whether a kind is part of a picture of the layers at rest.
pub fn shows_at_rest(kind: &WallpaperKind) -> bool {
    !is_overlay(kind)
}

/// The clock as a shader takes it: `phase` seconds of the wallpaper's own time, modulo one loop.
pub fn loop_time(phase: f64) -> f32 {
    phase.rem_euclid(LOOP_S) as f32
}

/// The same for what is slower.
pub fn slow_time(phase: f64) -> f32 {
    phase.rem_euclid(SLOW_LOOP_S) as f32
}

/// How many of `most` things an `intensity` (0 to 1) asks for.
pub fn share(most: usize, intensity: f32) -> usize {
    (most as f32 * intensity.clamp(0.0, 1.0)).round() as usize
}

/// A colour as the shaders take it: premultiplied colours are brought back to the colour itself, as the palette names.
fn straight(c: Rgba) -> [f32; 3] {
    let alpha = f32::from(c.3) / 255.0;
    if alpha <= 0.0 {
        return [0.0; 3];
    }
    [c.0, c.1, c.2].map(|channel| (f32::from(channel) / 255.0 / alpha).min(1.0))
}

/// The three phases the silk's fields are given, each of them at its own rate and wrapped to one turn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Warp {
    /// LumaGlass's `phase`: 0.70 t, 0.50 t, 0.40 t, 0.90 t.
    pub phase: [f32; 4],
    /// The fifth: 0.56 t, which is 0.8 of the first.
    pub phase5: f32,
    /// Amplitude (of the layer's width), depth, wave scale and detail.
    pub params: [f32; 4],
}

impl Warp {
    /// The warp at `phase` seconds. `motion` (0 to 1) scales its amplitude and its light, which is 0 for a wallpaper at rest.
    pub fn new(phase: f64, amplitude: f32, depth: f32, detail: f32, motion: f32) -> Self {
        let turn = |rate: f64| (phase * rate).rem_euclid(f64::from(TAU)) as f32;
        Self {
            phase: [turn(0.70), turn(0.50), turn(0.40), turn(0.90)],
            phase5: turn(0.56),
            params: [amplitude * motion, depth * motion, SILK_SCALE, detail],
        }
    }
}

/// The taps that are alive as the water's shader takes them (x, y, age in seconds, strength), and how many; none for a
/// wallpaper at rest.
pub fn ripple_uniforms(ripples: &Ripples, still: bool) -> ([[f32; 4]; MAX_RIPPLES], usize) {
    let mut out = [[0.0; 4]; MAX_RIPPLES];
    if still {
        return (out, 0);
    }
    for (slot, tap) in out.iter_mut().zip(&ripples.items[..ripples.count]) {
        *slot = [tap.x, tap.y, tap.age_s, 1.0];
    }
    (out, ripples.count)
}

/// A storm's lightning at `phase` seconds: 0 between strikes, up to 1 in a flash. Time is cut into slots of 6.5 seconds;
/// a slot has a strike, three in four do, at a moment of its own, and a strike is a bright flash, a dip and a fainter one
/// after it. Made from the slot's number, so that it is the same every time the storm is looked at.
pub fn lightning(phase: f64) -> f32 {
    const SLOT_S: f64 = 6.5;
    let slot = (phase / SLOT_S).floor();
    let mut rng = Rng::new(slot as i64 as u64 ^ 0x51_7C_C1_B7);
    let (strikes, at) = (rng.unit(), 0.8 + 3.6 * f64::from(rng.unit()));
    if strikes < 0.25 {
        return 0.0;
    }
    let since = phase - slot * SLOT_S - at;
    if !(0.0..1.0).contains(&since) {
        return 0.0;
    }
    let first = (-since / 0.06).exp();
    let second = if since > 0.16 {
        0.7 * (-(since - 0.16) / 0.09).exp()
    } else {
        0.0
    };
    (first + second).min(1.0) as f32
}

/// A small deterministic generator (SplitMix64) for the meshes' seeds and homes, so that a wallpaper looks the same on
/// every start.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in 0 to 1 (never 1), 24 bits of it.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The meshes, as data.
// ---------------------------------------------------------------------------------------------------------------

/// `cols` × `rows` cells over the unit square, as vertices `[u, v, 0, 0]` and two triangles a cell.
fn grid_mesh(cols: usize, rows: usize) -> (Vec<Vertex>, Vec<u16>) {
    let mut vertices = Vec::with_capacity((cols + 1) * (rows + 1));
    for row in 0..=rows {
        for col in 0..=cols {
            vertices.push([col as f32 / cols as f32, row as f32 / rows as f32, 0.0, 0.0]);
        }
    }
    let mut indices = Vec::with_capacity(cols * rows * 6);
    let stride = cols + 1;
    for row in 0..rows {
        for col in 0..cols {
            let first = (row * stride + col) as u16;
            let (right, below) = (first + 1, first + stride as u16);
            indices.extend_from_slice(&[first, right, below, right, below + 1, below]);
        }
    }
    (vertices, indices)
}

/// The two triangles of a sprite's quad, in the order the shaders' corner numbers (0 upper left, 1 upper right, 2 lower
/// left, 3 lower right) go.
const QUAD: [f32; 6] = [0.0, 1.0, 2.0, 1.0, 3.0, 2.0];

/// `count` sprites, six vertices each: `[home.x, home.y, seed, corner]`.
fn sprite_mesh(count: usize, seed: u64, home: impl Fn(&mut Rng) -> [f32; 2]) -> Vec<Vertex> {
    let mut rng = Rng::new(seed);
    let mut out = Vec::with_capacity(count * 6);
    for _ in 0..count {
        let at = home(&mut rng);
        let seed = rng.unit();
        out.extend(QUAD.iter().map(|&corner| [at[0], at[1], seed, corner]));
    }
    out
}

/// How many corners a ray has across each of its two rows: enough for the alpha to be a smooth bell across it.
const RAY_CORNERS: usize = 5;

/// The sun's rays: `count` wedges of triangles, their corners `[across, along, seed, 0]` where across runs from -1 to 1
/// over `RAY_CORNERS` corners and along is 0 (the apex, a single corner), 0.5 or 1.
fn ray_mesh(count: usize) -> Vec<Vertex> {
    let mut rng = Rng::new(0x5EED_5A1E);
    let mut out = Vec::new();
    for _ in 0..count {
        let seed = rng.unit();
        let corner = |across: f32, along: f32| [across, along, seed, 0.0];
        let across = |k: usize| -1.0 + 2.0 * k as f32 / (RAY_CORNERS - 1) as f32;
        let apex = corner(0.0, 0.0);
        for k in 0..RAY_CORNERS - 1 {
            let (a, b) = (across(k), across(k + 1));
            // The near half is a fan from the apex; the far half is a strip of quads between the two rows.
            out.extend_from_slice(&[apex, corner(a, 0.5), corner(b, 0.5)]);
            out.extend_from_slice(&[corner(a, 0.5), corner(b, 0.5), corner(a, 1.0)]);
            out.extend_from_slice(&[corner(b, 0.5), corner(b, 1.0), corner(a, 1.0)]);
        }
    }
    out
}

/// `count` discs of `rings` rings of `segments` corners round a centre, as `[x, y, ring, seed]` with x and y in radii
/// and ring the corner's distance from the centre in radii; the triangles' indices follow.
fn disc_mesh(count: usize, rings: usize, segments: usize) -> (Vec<Vertex>, Vec<u16>) {
    let mut rng = Rng::new(0xC10D_5EED);
    let per = 1 + rings * segments;
    let mut vertices = Vec::with_capacity(count * per);
    let mut indices = Vec::new();
    for disc in 0..count {
        let seed = if count == 1 { 0.0 } else { rng.unit() };
        let base = disc * per;
        vertices.push([0.0, 0.0, 0.0, seed]);
        for ring in 1..=rings {
            let radius = ring as f32 / rings as f32;
            for segment in 0..segments {
                let angle = TAU * segment as f32 / segments as f32;
                vertices.push([radius * angle.cos(), radius * angle.sin(), radius, seed]);
            }
        }
        let at = |ring: usize, segment: usize| {
            (base + 1 + (ring - 1) * segments + segment % segments) as u16
        };
        for segment in 0..segments {
            indices.extend_from_slice(&[base as u16, at(1, segment), at(1, segment + 1)]);
        }
        for ring in 1..rings {
            for segment in 0..segments {
                let (a, b) = (at(ring, segment), at(ring, segment + 1));
                let (c, d) = (at(ring + 1, segment), at(ring + 1, segment + 1));
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
    (vertices, indices)
}

/// Where a sprite lives, in the layer's width and height.
fn home_in_layer(rng: &mut Rng) -> [f32; 2] {
    [rng.unit(), rng.unit()]
}

/// Where a falling thing starts across the layer; how far down it is comes from its clock.
fn home_across(rng: &mut Rng) -> [f32; 2] {
    [rng.unit(), 0.0]
}

/// Where a star is: evenly over the disc, in half-diagonals of the layer, that the sky turns inside.
fn home_in_disc(rng: &mut Rng) -> [f32; 2] {
    let radius = rng.unit().sqrt();
    let angle = TAU * rng.unit();
    [radius * angle.cos(), radius * angle.sin()]
}

/// The unit square as two triangles: a flat colour over the whole layer.
const UNIT_QUAD: [Vertex; 6] = [
    [0.0, 0.0, 0.0, 0.0],
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [1.0, 0.0, 0.0, 0.0],
    [1.0, 1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
];

// ---------------------------------------------------------------------------------------------------------------
// On the GPU.
// ---------------------------------------------------------------------------------------------------------------

/// A mesh that lives in the GPU's memory.
struct Mesh {
    vertices: u32,
    indices: Option<u32>,
    /// Vertices, or indices where there are some.
    count: i32,
}

impl Mesh {
    fn new(gl: &Gles, vertices: &[Vertex], indices: Option<&[u16]>) -> Self {
        let buffer = gl.new_vertex_buffer();
        gl.static_vertices(buffer, vertices);
        let indices_buffer = indices.map(|i| gl.new_index_buffer(i));
        Self {
            vertices: buffer,
            indices: indices_buffer,
            count: indices.map_or(vertices.len(), <[u16]>::len) as i32,
        }
    }

    fn draw(&self, gl: &Gles) {
        self.draw_first(gl, self.count);
    }

    /// The first `count` vertices (or indices).
    fn draw_first(&self, gl: &Gles, count: i32) {
        if count <= 0 {
            return;
        }
        gl.use_vertex_buffer(self.vertices);
        match self.indices {
            Some(indices) => gl.draw_indexed(indices, count),
            None => gl.draw_triangles(0, count),
        }
    }
}

/// What every program is told of where the layer is.
const PLACEMENT: [&str; 5] = ["u_ndc", "u_origin", "u_axes", "u_size", "u_opacity"];

struct Program {
    id: u32,
    at: HashMap<&'static str, i32>,
}

impl Program {
    /// Builds a program and finds the locations of `uniforms` and of the placement's.
    fn new(
        gl: &Gles,
        vertex: &CStr,
        fragment: &CStr,
        uniforms: &[&'static str],
        ndc: [f32; 2],
    ) -> io::Result<Self> {
        let id = gl.program(vertex, fragment)?;
        gl.use_program(id);
        let at: HashMap<&'static str, i32> = PLACEMENT
            .iter()
            .chain(uniforms)
            .map(|&name| {
                let c_name = CString::new(name).expect("a uniform's name has no NUL");
                (name, gl.uniform_location(id, &c_name))
            })
            .collect();
        gl.uniform2f(at["u_ndc"], ndc[0], ndc[1]);
        if let Some(&picture) = at.get("u_picture") {
            gl.uniform1i(picture, 0);
        }
        Ok(Self { id, at })
    }

    fn u(&self, name: &str) -> i32 {
        *self
            .at
            .get(name)
            .unwrap_or_else(|| panic!("uniform {name} is not one of this program's"))
    }
}

/// How one wallpaper is drawn this time.
#[derive(Clone, Copy)]
pub struct Draw<'a> {
    pub layer: compositor_protocol::LayerId,
    /// The layer's size in its own pixels.
    pub size: (f32, f32),
    pub opacity: f32,
    /// The panel position of the layer's upper-left corner, and panel pixels per layer pixel along its two axes.
    pub origin: [f32; 2],
    pub axes: [f32; 4],
    /// The wallpaper's clock, in seconds of its designed pace.
    pub phase: f64,
    pub ripples: &'a Ripples,
    /// Drawn as it rests, for a picture of the layers that keeps: no motion and no taps.
    pub still: bool,
    /// The texture of the picture of a kind that has one.
    pub picture: Option<u32>,
}

/// The programs and meshes of every kind of wallpaper.
pub struct Wallpapers {
    look: Look,
    home_silk: Option<(compositor_protocol::LayerId, Silk)>,
    silk_revision: u64,
    silk: Program,
    /// Silk with LumaGlass's optional extras compiled in, used only when one of them is asked for.
    silk_look: Program,
    liquid: Program,
    aurora: Program,
    sky: Program,
    fog: Program,
    stars: Program,
    fireflies: Program,
    snow: Program,
    rain: Program,
    rays: Program,
    glow: Program,
    puffs: Program,
    flat: Program,
    silk_grid: Mesh,
    fine_grid: Mesh,
    coarse_grid: Mesh,
    star_sprites: Mesh,
    firefly_sprites: Mesh,
    rain_sprites: Mesh,
    storm_sprites: Mesh,
    snow_sprites: Mesh,
    ray_wedges: Mesh,
    glow_disc: Mesh,
    puff_discs: Mesh,
    quad: Mesh,
}

const RIPPLES: [&str; MAX_RIPPLES] = [
    "u_ripple[0]",
    "u_ripple[1]",
    "u_ripple[2]",
    "u_ripple[3]",
    "u_ripple[4]",
    "u_ripple[5]",
];
const COLOURS: [&str; 4] = ["u_colour[0]", "u_colour[1]", "u_colour[2]", "u_colour[3]"];
const SILK_UNIFORMS: [&str; 5] = ["u_phase", "u_phase5", "u_params", "u_scrim", "u_picture"];
const SILK_LOOK_UNIFORMS: [&str; 6] = [
    "u_phase",
    "u_phase5",
    "u_params",
    "u_scrim",
    "u_picture",
    "u_look",
];
const LIQUID_UNIFORMS: [&str; 12] = [
    "u_phase",
    "u_phase5",
    "u_params",
    "u_count",
    "u_wave",
    "u_picture",
    RIPPLES[0],
    RIPPLES[1],
    RIPPLES[2],
    RIPPLES[3],
    RIPPLES[4],
    RIPPLES[5],
];
const AURORA_UNIFORMS: [&str; 6] = [
    COLOURS[0],
    COLOURS[1],
    COLOURS[2],
    COLOURS[3],
    "u_time",
    "u_strength",
];

impl Wallpapers {
    /// Compiles and links every program and makes every mesh now, so that nothing is compiled while a wallpaper is
    /// being chosen on the screen.
    pub fn new(gl: &Gles, panel: (u32, u32), look: Look) -> io::Result<Self> {
        let ndc = [2.0 / panel.0 as f32, 2.0 / panel.1 as f32];
        let program = |vertex: CString, fragment: &CString, uniforms: &[&'static str]| {
            Program::new(gl, &vertex, fragment, uniforms, ndc)
        };
        let picture = shader::liquid_fragment();
        let field = shader::field_fragment();
        let disc = shader::disc_fragment();
        let mesh = |vertices: &[Vertex], indices: Option<&[u16]>| Mesh::new(gl, vertices, indices);
        let grid = |(cols, rows): (usize, usize)| {
            let (vertices, indices) = grid_mesh(cols, rows);
            mesh(&vertices, Some(&indices))
        };
        let discs = |count, rings, segments| {
            let (vertices, indices) = disc_mesh(count, rings, segments);
            mesh(&vertices, Some(&indices))
        };
        let sprites = |count, seed, home: fn(&mut Rng) -> [f32; 2]| {
            mesh(&sprite_mesh(count, seed, home), None)
        };
        Ok(Self {
            home_silk: None,
            silk_revision: 0,
            look,
            silk: program(
                shader::silk_vertex(false),
                &shader::silk_fragment(false),
                &SILK_UNIFORMS,
            )?,
            silk_look: program(
                shader::silk_vertex(true),
                &shader::silk_fragment(true),
                &SILK_LOOK_UNIFORMS,
            )?,
            liquid: program(shader::liquid_vertex(), &picture, &LIQUID_UNIFORMS)?,
            aurora: program(shader::aurora_vertex(), &field, &AURORA_UNIFORMS)?,
            sky: program(shader::sky_vertex(), &field, &["u_slow"])?,
            fog: program(shader::fog_vertex(), &field, &["u_time", "u_fog"])?,
            stars: program(
                shader::stars_vertex(),
                &disc,
                &["u_time", "u_slow", "u_disc"],
            )?,
            fireflies: program(
                shader::fireflies_vertex(),
                &disc,
                &["u_time", "u_colour", "u_disc"],
            )?,
            snow: program(
                shader::snow_vertex(),
                &disc,
                &["u_time", "u_colour", "u_disc"],
            )?,
            rain: program(
                shader::rain_vertex(),
                &shader::streak_fragment(),
                &["u_time", "u_rain"],
            )?,
            rays: program(shader::rays_vertex(), &field, &["u_time", "u_sun"])?,
            glow: program(shader::glow_vertex(), &field, &["u_sun"])?,
            puffs: program(
                shader::puffs_vertex(),
                &field,
                &["u_time", "u_slow", "u_cloud"],
            )?,
            flat: program(
                shader::flat_vertex(),
                &shader::flat_fragment(),
                &["u_colour"],
            )?,
            silk_grid: grid(SILK_GRID),
            fine_grid: grid(FINE_GRID),
            coarse_grid: grid(COARSE_GRID),
            star_sprites: sprites(STARS, 0x57A2_F1E1, home_in_disc),
            firefly_sprites: sprites(FIREFLIES, 0xF1EF_11E5, home_in_layer),
            rain_sprites: sprites(RAINDROPS, 0x0A11_0001, home_across),
            storm_sprites: sprites(STORM_DROPS, 0x0A11_0002, home_across),
            snow_sprites: sprites(SNOWFLAKES, 0x5A0F_F1A4, home_across),
            ray_wedges: mesh(&ray_mesh(RAYS), None),
            glow_disc: discs(1, 6, 32),
            puff_discs: discs(PUFFS, PUFF_RINGS, PUFF_SEGMENTS),
            quad: mesh(&UNIT_QUAD, None),
        })
    }

    /// Asks for LumaGlass's optional extras of the silk (all off unless this is called with some on).
    pub fn set_look(&mut self, look: Look) {
        self.look = look;
    }

    pub fn set_silk(&mut self, id: compositor_protocol::LayerId, silk: Silk) -> bool {
        if self.home_silk == Some((id, silk)) { return false; }
        self.home_silk = Some((id, silk));
        self.silk_revision = self.silk_revision.wrapping_add(1).max(1);
        true
    }

    pub fn clear_silk(&mut self) -> bool { self.home_silk.take().is_some() }

    pub fn revision(&self, id: compositor_protocol::LayerId) -> u64 {
        if self.home_silk.is_some_and(|(of, _)| of == id) { self.silk_revision } else { 0 }
    }

    /// Sets the program's placement and opacity, and blending on or off.
    fn begin(&self, gl: &Gles, program: &Program, draw: &Draw, blend: bool) {
        gl.use_program(program.id);
        gl.uniform2f(program.u("u_origin"), draw.origin[0], draw.origin[1]);
        let axes = draw.axes;
        gl.uniform4f(program.u("u_axes"), axes[0], axes[1], axes[2], axes[3]);
        gl.uniform2f(program.u("u_size"), draw.size.0, draw.size.1);
        gl.uniform1f(program.u("u_opacity"), draw.opacity);
        if blend {
            gl.enable(GL_BLEND);
        } else {
            gl.disable(GL_BLEND);
        }
    }

    /// Draws a wallpaper. Blending is left on, whatever it was; the program and the vertex buffer are not the caller's.
    pub fn draw(&self, gl: &Gles, wallpaper: &Wallpaper, draw: &Draw) {
        if draw.still && is_overlay(&wallpaper.kind) {
            return;
        }
        let intensity = wallpaper.intensity.clamp(0.0, 1.0);
        // The base kinds cover the layer: at full opacity there is nothing under them to blend with.
        let blend_base = draw.opacity < 1.0;
        let (time, slow) = if draw.still {
            (0.0, 0.0)
        } else {
            (loop_time(draw.phase), slow_time(draw.phase))
        };
        let motion = if draw.still { 0.0 } else { intensity };
        match wallpaper.kind {
            WallpaperKind::Silk => {
                let Some(picture) = draw.picture else { return };
                let home = self.home_silk.filter(|(id, _)| *id == draw.layer).map(|(_, silk)| silk);
                let silk = home.unwrap_or(Silk { look: self.look, ..Silk::default() });
                let look = silk.look;
                let with_look = look != Look::default();
                let p = if with_look {
                    &self.silk_look
                } else {
                    &self.silk
                };
                self.begin(gl, p, draw, blend_base);
                let mut warp = Warp::new(draw.phase, silk.amplitude / 1920.0, silk.depth, silk.detail, motion);
                warp.params[2] = silk.scale;
                Self::set_warp(gl, p, &warp);
                if with_look {
                    gl.uniform3f(
                        p.u("u_look"),
                        look.vignette,
                        look.specular,
                        look.bloom,
                    );
                }
                gl.uniform3f(p.u("u_scrim"), SILK_SCRIM[0], SILK_SCRIM[1], SILK_SCRIM[2]);
                gl.active_texture(0);
                gl.bind_texture(picture);
                self.silk_grid.draw(gl);
            }
            WallpaperKind::Liquid => {
                let Some(picture) = draw.picture else { return };
                let p = &self.liquid;
                self.begin(gl, p, draw, blend_base);
                let warp = Warp::new(
                    draw.phase,
                    LIQUID_AMPLITUDE,
                    LIQUID_DEPTH,
                    LIQUID_DETAIL,
                    motion,
                );
                Self::set_warp(gl, p, &warp);
                let (taps, count) = ripple_uniforms(draw.ripples, draw.still);
                gl.uniform1i(p.u("u_count"), count as i32);
                for (name, tap) in RIPPLES.iter().zip(taps).take(count) {
                    gl.uniform4f(p.u(name), tap[0], tap[1], tap[2], tap[3]);
                }
                gl.uniform3f(
                    p.u("u_wave"),
                    LIQUID_WAVE[0],
                    LIQUID_WAVE[1],
                    LIQUID_WAVE[2],
                );
                gl.active_texture(0);
                gl.bind_texture(picture);
                // The fine grid is only worth its vertices while there is a ripple to resolve.
                if count > 0 {
                    self.fine_grid.draw(gl);
                } else {
                    self.silk_grid.draw(gl);
                }
            }
            WallpaperKind::Aurora { colours } => {
                let p = &self.aurora;
                self.begin(gl, p, draw, blend_base);
                gl.uniform1f(p.u("u_time"), time);
                gl.uniform1f(p.u("u_strength"), AURORA_STRENGTH * intensity);
                for (name, colour) in COLOURS.iter().zip(colours) {
                    let rgb = straight(colour);
                    gl.uniform3f(p.u(name), rgb[0], rgb[1], rgb[2]);
                }
                self.coarse_grid.draw(gl);
            }
            WallpaperKind::Starfield => {
                let p = &self.sky;
                self.begin(gl, p, draw, blend_base);
                gl.uniform1f(p.u("u_slow"), slow);
                self.coarse_grid.draw(gl);
                let stars = share(STARS, intensity);
                if stars > 0 {
                    let p = &self.stars;
                    self.begin(gl, p, draw, true);
                    gl.uniform1f(p.u("u_time"), time);
                    gl.uniform1f(p.u("u_slow"), slow);
                    gl.uniform2f(p.u("u_disc"), 0.45, 0.35);
                    self.star_sprites.draw_first(gl, 6 * stars as i32);
                }
            }
            WallpaperKind::Fireflies { colour } => {
                let p = &self.fireflies;
                self.begin(gl, p, draw, true);
                let rgb = straight(colour);
                gl.uniform1f(p.u("u_time"), time);
                gl.uniform3f(p.u("u_colour"), rgb[0], rgb[1], rgb[2]);
                gl.uniform2f(p.u("u_disc"), 0.20, 0.55);
                self.firefly_sprites
                    .draw_first(gl, 6 * share(FIREFLIES, intensity) as i32);
            }
            WallpaperKind::Weather(weather) => {
                self.draw_weather(gl, weather, draw, intensity, time, slow);
            }
        }
        gl.enable(GL_BLEND);
    }

    fn set_warp(gl: &Gles, program: &Program, warp: &Warp) {
        let p = warp.phase;
        gl.uniform4f(program.u("u_phase"), p[0], p[1], p[2], p[3]);
        gl.uniform1f(program.u("u_phase5"), warp.phase5);
        let q = warp.params;
        gl.uniform4f(program.u("u_params"), q[0], q[1], q[2], q[3]);
    }

    fn draw_weather(
        &self,
        gl: &Gles,
        weather: Weather,
        draw: &Draw,
        intensity: f32,
        time: f32,
        slow: f32,
    ) {
        let rain = |sprites: &Mesh, drops: usize, rain: [f32; 4]| {
            let p = &self.rain;
            self.begin(gl, p, draw, true);
            gl.uniform1f(p.u("u_time"), time);
            gl.uniform4f(p.u("u_rain"), rain[0], rain[1], rain[2], rain[3]);
            sprites.draw_first(gl, 6 * share(drops, intensity) as i32);
        };
        match weather {
            Weather::Rain => rain(&self.rain_sprites, RAINDROPS, [0.16, 1.0, 1.0, 0.0]),
            Weather::Storm => {
                // The darkness under the rain, the rain slanted harder, and the lightning over it.
                let p = &self.flat;
                self.begin(gl, p, draw, true);
                let dark = 0.30 * intensity;
                gl.uniform4f(
                    p.u("u_colour"),
                    0.020 * dark,
                    0.024 * dark,
                    0.060 * dark,
                    dark,
                );
                self.quad.draw(gl);
                rain(&self.storm_sprites, STORM_DROPS, [0.34, 1.25, 1.15, 0.0]);
                let flash = lightning(draw.phase) * intensity;
                if flash > 0.02 {
                    let p = &self.flat;
                    self.begin(gl, p, draw, true);
                    let a = 0.55 * flash;
                    gl.uniform4f(p.u("u_colour"), 0.85 * a, 0.90 * a, 1.0 * a, a);
                    self.quad.draw(gl);
                }
            }
            Weather::Snow => {
                let p = &self.snow;
                self.begin(gl, p, draw, true);
                gl.uniform1f(p.u("u_time"), time);
                gl.uniform3f(p.u("u_colour"), 0.93, 0.95, 1.0);
                gl.uniform2f(p.u("u_disc"), 0.85, 0.15);
                self.snow_sprites
                    .draw_first(gl, 6 * share(SNOWFLAKES, intensity) as i32);
            }
            Weather::Sun => {
                // The glow, and the rays across it, from a sun just above the upper right corner.
                let sun = [0.80, -0.05, 0.16 * intensity, 0.34 * intensity];
                let p = &self.glow;
                self.begin(gl, p, draw, true);
                gl.uniform4f(p.u("u_sun"), sun[0], sun[1], sun[2], sun[3]);
                self.glow_disc.draw(gl);
                let p = &self.rays;
                self.begin(gl, p, draw, true);
                gl.uniform1f(p.u("u_time"), time);
                gl.uniform4f(p.u("u_sun"), sun[0], sun[1], sun[2], sun[3]);
                self.ray_wedges.draw(gl);
            }
            Weather::Clouds => {
                let puffs = share(PUFFS, intensity);
                if puffs == 0 {
                    return;
                }
                let p = &self.puffs;
                self.begin(gl, p, draw, true);
                gl.uniform1f(p.u("u_time"), time);
                gl.uniform1f(p.u("u_slow"), slow);
                gl.uniform2f(p.u("u_cloud"), 0.36 * (0.5 + 0.5 * intensity), 0.0);
                // The puffs' triangles follow one another in the mesh, a puff's worth of indices each.
                self.puff_discs
                    .draw_first(gl, (disc_indices(PUFF_RINGS, PUFF_SEGMENTS) * puffs) as i32);
            }
            Weather::Fog => {
                let p = &self.fog;
                self.begin(gl, p, draw, true);
                gl.uniform1f(p.u("u_time"), time);
                gl.uniform2f(p.u("u_fog"), 0.42 * intensity, 0.0);
                self.coarse_grid.draw(gl);
            }
        }
    }
}

/// The indices of one disc of `rings` rings of `segments` corners.
fn disc_indices(rings: usize, segments: usize) -> usize {
    3 * segments + 6 * (rings - 1) * segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallpaper::Ripple;

    fn area(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
        ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
    }

    #[test]
    fn the_grid_tiles_the_unit_square_exactly_with_two_triangles_a_cell() {
        for (cols, rows) in [(1, 1), (3, 2), SILK_GRID, FINE_GRID] {
            let (vertices, indices) = grid_mesh(cols, rows);
            assert_eq!(vertices.len(), (cols + 1) * (rows + 1));
            assert_eq!(indices.len(), cols * rows * 6);
            assert!(indices.iter().all(|&i| usize::from(i) < vertices.len()));
            let total: f32 = indices
                .chunks(3)
                .map(|t| {
                    let p = |i: u16| {
                        let v = vertices[usize::from(i)];
                        [v[0], v[1]]
                    };
                    area(p(t[0]), p(t[1]), p(t[2]))
                })
                .sum();
            assert!((total - 1.0).abs() < 1e-3, "{cols}x{rows}: {total}");
        }
    }

    #[test]
    fn the_silk_grid_has_a_row_on_the_scrims_middle_stop() {
        let (vertices, _) = grid_mesh(SILK_GRID.0, SILK_GRID.1);
        assert!(
            vertices.iter().any(|v| (v[1] - 0.55).abs() < 1e-6),
            "the scrim bends at 0.55 of the height, and a grid row there keeps the bend sharp"
        );
    }

    #[test]
    fn a_sprite_is_a_quad_whose_corners_the_shader_can_tell_apart() {
        let sprites = sprite_mesh(5, 1, |rng| [rng.unit(), rng.unit()]);
        assert_eq!(sprites.len(), 30);
        for sprite in sprites.chunks(6) {
            // One home and one seed for all six, and the corners' triangles cover the square twice over (2 x 2 = 4
            // in corner units, the quad's area) without a gap.
            assert!(sprite.iter().all(|v| v[..3] == sprite[0][..3]));
            let corner = |c: f32| [c % 2.0 * 2.0 - 1.0, (c / 2.0).floor() * 2.0 - 1.0];
            let total: f32 = sprite
                .chunks(3)
                .map(|t| area(corner(t[0][3]), corner(t[1][3]), corner(t[2][3])))
                .sum();
            assert!((total - 4.0).abs() < 1e-6, "{total}");
        }
        // Reproducible: the same wallpaper on every start.
        assert_eq!(sprites, sprite_mesh(5, 1, |rng| [rng.unit(), rng.unit()]));
        assert_ne!(sprites, sprite_mesh(5, 2, |rng| [rng.unit(), rng.unit()]));
    }

    #[test]
    fn a_ray_is_a_wedge_that_the_alpha_can_fade_along_and_across() {
        let rays = ray_mesh(3);
        assert_eq!(rays.len(), 3 * 36);
        for ray in rays.chunks(36) {
            // Twelve triangles tile the wedge: in across and along units, a fan from the apex to the near row (base 2,
            // height a half: area 0.5) and a strip of quads between the rows (2 wide, a half high: area 1).
            let total: f32 = ray
                .chunks(3)
                .map(|t| area([t[0][0], t[0][1]], [t[1][0], t[1][1]], [t[2][0], t[2][1]]))
                .sum();
            assert!((total - 1.5).abs() < 1e-6, "{total}");
            // The one apex is the only corner at along 0, and there is a corner in the middle of each row.
            assert!(ray.iter().all(|v| v[3] == 0.0 && v[2] == ray[0][2]));
            assert!(ray.iter().any(|v| v[0] == 0.0 && v[1] == 0.5));
            assert!(ray.iter().any(|v| v[0] == 0.0 && v[1] == 1.0));
            assert!(
                ray.iter()
                    .all(|v| (-1.0..=1.0).contains(&v[0]) && [0.0, 0.5, 1.0].contains(&v[1]))
            );
        }
    }

    #[test]
    fn a_disc_is_a_round_mesh_that_can_have_its_alpha_fall_off_by_ring() {
        let (vertices, indices) = disc_mesh(2, 5, 16);
        let per = 1 + 5 * 16;
        assert_eq!(vertices.len(), 2 * per);
        assert_eq!(indices.len(), 2 * disc_indices(5, 16));
        assert!(indices.iter().all(|&i| usize::from(i) < vertices.len()));
        // One disc's triangles cover a regular polygon of 16 sides.
        let one = &indices[..disc_indices(5, 16)];
        let total: f32 = one
            .chunks(3)
            .map(|t| {
                let p = |i: u16| {
                    let v = vertices[usize::from(i)];
                    [v[0], v[1]]
                };
                area(p(t[0]), p(t[1]), p(t[2]))
            })
            .sum();
        let polygon = 0.5 * 16.0 * (TAU / 16.0).sin();
        assert!((total - polygon).abs() < 1e-3, "{total} against {polygon}");
        // A vertex's ring is its distance from the centre, and the two discs have seeds of their own.
        for v in &vertices {
            assert!(((v[0] * v[0] + v[1] * v[1]).sqrt() - v[2]).abs() < 1e-5);
        }
        assert_ne!(vertices[0][3], vertices[per][3]);
        // A lone disc (the sun's glow) has none.
        assert_eq!(disc_mesh(1, 3, 8).0[0][3], 0.0);
    }

    #[test]
    fn every_disc_of_the_puffs_fits_in_the_16_bit_indices() {
        let (vertices, _) = disc_mesh(PUFFS, PUFF_RINGS, PUFF_SEGMENTS);
        assert!(vertices.len() < usize::from(u16::MAX));
        let (vertices, _) = grid_mesh(FINE_GRID.0, FINE_GRID.1);
        assert!(vertices.len() < usize::from(u16::MAX));
    }

    #[test]
    fn the_clocks_the_shaders_get_stay_small_and_continuous_however_long_the_compositor_runs() {
        // A month of seconds, and a step of one refresh later.
        let month = 30.0 * 86_400.0;
        let step = 0.0335;
        let before = loop_time(month);
        let after = loop_time(month + step);
        assert!((0.0..LOOP_S as f32).contains(&before));
        assert!(
            ((after - before) - step as f32).abs() < 1e-3 || after < before,
            "a step is a step, except across the wrap"
        );
        // The two clocks wrap together: the slow one is a whole number of the fast one's loops.
        assert_eq!(SLOW_LOOP_S % LOOP_S, 0.0);
        assert!((slow_time(3.0 * LOOP_S + 5.0) - (3.0 * LOOP_S as f32 + 5.0)).abs() < 1e-3);
        assert_eq!(loop_time(3.0 * LOOP_S + 5.0), 5.0);
    }

    #[test]
    fn the_silks_fields_are_phases_at_their_own_rates_that_wrap_without_a_jump() {
        let a = Warp::new(10.0, SILK_AMPLITUDE, SILK_DEPTH, SILK_DETAIL, 1.0);
        for (phase, rate) in a.phase.iter().zip([0.70, 0.50, 0.40, 0.90]) {
            assert!((f64::from(*phase) - (10.0 * rate) % f64::from(TAU)).abs() < 1e-5);
        }
        assert!((f64::from(a.phase5) - (10.0 * 0.56) % f64::from(TAU)).abs() < 1e-5);
        // A month on, a step later: each phase has moved by rate x step (mod a turn) to the precision of a float.
        let month = 30.0 * 86_400.0;
        let (now, next) = (
            Warp::new(month, SILK_AMPLITUDE, SILK_DEPTH, SILK_DETAIL, 1.0),
            Warp::new(month + 0.0335, SILK_AMPLITUDE, SILK_DEPTH, SILK_DETAIL, 1.0),
        );
        for k in 0..4 {
            let moved = (f64::from(next.phase[k] - now.phase[k])).rem_euclid(f64::from(TAU));
            let rate = [0.70, 0.50, 0.40, 0.90][k];
            assert!((moved - 0.0335 * rate).abs() < 1e-4, "field {k}: {moved}");
        }
        assert!(a.phase.iter().all(|p| (0.0..TAU).contains(p)));
    }

    #[test]
    fn a_wallpaper_at_rest_does_not_warp_and_the_motion_scales_the_amount() {
        let full = Warp::new(3.0, SILK_AMPLITUDE, SILK_DEPTH, SILK_DETAIL, 1.0);
        assert_eq!(full.params, [SILK_AMPLITUDE, SILK_DEPTH, 1.0, SILK_DETAIL]);
        let half = Warp::new(3.0, SILK_AMPLITUDE, SILK_DEPTH, SILK_DETAIL, 0.5);
        assert_eq!(half.params[0], SILK_AMPLITUDE / 2.0);
        assert_eq!(half.params[1], SILK_DEPTH / 2.0);
        let still = Warp::new(3.0, SILK_AMPLITUDE, SILK_DEPTH, SILK_DETAIL, 0.0);
        assert_eq!((still.params[0], still.params[1]), (0.0, 0.0));
    }

    #[test]
    fn only_fireflies_and_weather_lie_over_what_is_below() {
        let aurora = WallpaperKind::Aurora {
            colours: [Rgba(1, 2, 3, 255); 4],
        };
        for base in [
            WallpaperKind::Silk,
            WallpaperKind::Liquid,
            WallpaperKind::Starfield,
            aurora,
        ] {
            assert!(!is_overlay(&base) && shows_at_rest(&base), "{base:?}");
        }
        for over in [
            WallpaperKind::Fireflies {
                colour: Rgba(255, 200, 100, 255),
            },
            WallpaperKind::Weather(Weather::Rain),
            WallpaperKind::Weather(Weather::Fog),
        ] {
            assert!(is_overlay(&over) && !shows_at_rest(&over), "{over:?}");
        }
    }

    #[test]
    fn intensity_is_the_share_of_the_particles_drawn() {
        assert_eq!(share(300, 1.0), 300);
        assert_eq!(share(300, 0.5), 150);
        assert_eq!(share(300, 0.0), 0);
        // Out of range values are held to it.
        assert_eq!(share(300, 3.0), 300);
        assert_eq!(share(300, -1.0), 0);
    }

    #[test]
    fn the_colours_an_aurora_is_given_are_the_colours_themselves() {
        assert_eq!(straight(Rgba(255, 128, 0, 255)), [1.0, 128.0 / 255.0, 0.0]);
        // A colour that is half transparent was premultiplied: the colour is what it was before.
        let half = straight(Rgba(100, 50, 0, 128));
        assert!((half[0] - 100.0 / 128.0).abs() < 1e-3 && (half[1] - 50.0 / 128.0).abs() < 1e-3);
        assert_eq!(straight(Rgba(0, 0, 0, 0)), [0.0; 3]);
    }

    #[test]
    fn only_the_taps_that_are_alive_reach_the_water_and_none_at_rest() {
        let mut ripples = Ripples::default();
        ripples.items[0] = Ripple {
            x: 10.0,
            y: 20.0,
            age_s: 0.5,
        };
        ripples.items[1] = Ripple {
            x: 30.0,
            y: 40.0,
            age_s: 1.5,
        };
        ripples.count = 2;
        let (taps, count) = ripple_uniforms(&ripples, false);
        assert_eq!(count, 2);
        assert_eq!(taps[0], [10.0, 20.0, 0.5, 1.0]);
        assert_eq!(taps[1], [30.0, 40.0, 1.5, 1.0]);
        assert_eq!(ripple_uniforms(&ripples, true).1, 0);
    }

    #[test]
    fn lightning_is_dark_between_strikes_bright_in_them_and_the_same_every_time() {
        let mut flashes = 0;
        let mut brightest = 0.0f32;
        let mut previous = 0.0;
        // Ten minutes at 60 frames a second.
        for frame in 0..36_000 {
            let phase = f64::from(frame) / 60.0;
            let l = lightning(phase);
            assert!((0.0..=1.0).contains(&l), "{phase}: {l}");
            assert_eq!(l, lightning(phase), "deterministic");
            if l > 0.02 && previous <= 0.02 {
                flashes += 1;
            }
            previous = l;
            brightest = brightest.max(l);
        }
        // About three strikes in four of the 92 slots, each of two flashes at most: well over 40 and not 400.
        assert!((40..400).contains(&flashes), "{flashes}");
        assert!(brightest > 0.9, "{brightest}");
        // Most of the time it is dark: a flash lasts well under half a second in 6.5.
        let lit = (0..6_500)
            .filter(|&ms| lightning(f64::from(ms) / 1000.0 + 1_000.0) > 0.02)
            .count();
        assert!(lit < 6_500 / 6, "{lit}");
    }

    // --- The whole thing, drawn by the renderer into a software GL. ---

    use crate::scene::Drawn;
    use crate::testgl::{Rig, Shot, drawn, rig};
    use compositor_protocol::{Backdrop, Content, Fill, Glass, Props, Shape};

    const SCREEN: (u32, u32) = (960, 480);

    fn wallpaper(kind: WallpaperKind, intensity: f32) -> Content {
        Content::Wallpaper(Wallpaper {
            kind,
            speed: 1.0,
            intensity,
            fps: 30,
        })
    }

    /// A wallpaper over the whole screen, `phase` seconds into its motion.
    fn layer(id: u32, kind: WallpaperKind, intensity: f32, phase: f64) -> Drawn {
        let mut layer = drawn(id, SCREEN, wallpaper(kind, intensity), Props::at(0.0, 0.0));
        layer.phase = phase;
        layer
    }

    fn picture(pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let mut out = Vec::with_capacity((SCREEN.0 * SCREEN.1 * 4) as usize);
        for y in 0..SCREEN.1 {
            for x in 0..SCREEN.0 {
                let [r, g, b] = pixel(x, y);
                out.extend_from_slice(&[r, g, b, 255]);
            }
        }
        out
    }

    fn flat(grey: u8) -> Vec<u8> {
        picture(|_, _| [grey; 3])
    }

    /// A triangle wave along x in red and along y in green, seven levels a pixel and 64 pixels to a period, and blue a
    /// constant: where a pixel of the screen has read the picture from can be told from its colour to a tenth of a pixel
    /// (it is continuous, so nothing blends across a jump), and how bright the light made it from the blue.
    fn waves() -> Vec<u8> {
        picture(|x, y| [wave(x), wave(y), 220])
    }

    fn wave(at: u32) -> u8 {
        let t = at % 64;
        (if t < 32 { t } else { 64 - t } * 7) as u8
    }

    /// How far, in pixels, the picture was read from (right, down) of a pixel of `shot` that was drawn from `waves()`,
    /// and the light that made it (the blue over its 220). Only where it can be told: on the middle of a rising or a
    /// falling slope, ten pixels from the turns, which is farther than the picture is ever moved.
    fn read_from(shot: &Shot, x: u32, y: u32) -> Option<(f32, f32, f32)> {
        let [r, g, b, _] = shot.pixel(x, y);
        let light = f32::from(b) / 220.0;
        let source = |level: u8, at: u32| -> Option<f32> {
            let t = (at % 64) as f32;
            let seen = f32::from(level) / light / 7.0;
            match t {
                10.0..=22.0 => Some(seen - t),
                42.0..=54.0 => Some(64.0 - seen - t),
                _ => None,
            }
        };
        Some((source(r, x)?, source(g, y)?, light))
    }

    /// Where `read_from` can be asked, over the whole screen.
    fn readable() -> impl Iterator<Item = (u32, u32)> {
        (0..SCREEN.1)
            .step_by(4)
            .flat_map(|y| (0..SCREEN.0).step_by(4).map(move |x| (x, y)))
    }

    fn scrim_alpha(y: f32) -> f32 {
        if y < 0.55 {
            SILK_SCRIM[0] + (SILK_SCRIM[1] - SILK_SCRIM[0]) * y / 0.55
        } else {
            SILK_SCRIM[1] + (SILK_SCRIM[2] - SILK_SCRIM[1]) * (y - 0.55) / 0.45
        }
    }

    fn luma(p: [u8; 4]) -> f32 {
        0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2])
    }

    fn pixels(shot: &Shot) -> impl Iterator<Item = (u32, u32, [u8; 4])> + '_ {
        (0..SCREEN.1).flat_map(move |y| (0..SCREEN.0).map(move |x| (x, y, shot.pixel(x, y))))
    }

    fn differs(a: [u8; 4], b: [u8; 4], by: u8) -> bool {
        (0..3).any(|c| a[c].abs_diff(b[c]) > by)
    }

    fn silk(rig: &mut Rig, intensity: f32, phase: f64, pictured: Vec<u8>) -> Shot {
        rig.set_pixels(1, SCREEN, &pictured);
        rig.draw(&[layer(1, WallpaperKind::Silk, intensity, phase)])
    }

    #[test]
    fn silk_without_motion_is_the_picture_under_the_three_stop_scrim() {
        let Some(mut rig) = rig() else { return };
        let shot = silk(&mut rig, 0.0, 3.0, flat(200));
        shot.save("silk-still");
        for y in [0u32, 60, 132, 264, 380, 479] {
            let at = (y as f32 + 0.5) / SCREEN.1 as f32;
            let want = 200.0 * (1.0 - scrim_alpha(at));
            for x in [10u32, 480, 949] {
                let got = shot.pixel(x, y);
                assert!(
                    got[..3].iter().all(|&c| (f32::from(c) - want).abs() <= 2.5),
                    "({x}, {y}): {got:?} against {want}"
                );
                assert_eq!(got[3], 255);
            }
        }
    }

    #[test]
    fn silk_moves_the_picture_a_few_pixels_and_lights_its_folds_and_a_zero_intensity_holds_it_still()
     {
        let Some(mut rig) = rig() else { return };
        let moving = silk(&mut rig, 1.0, 12.0, waves());
        moving.save("silk-moving");
        let mut shifts = Vec::new();
        let mut lights = Vec::new();
        for (x, y) in readable() {
            if let Some((dx, dy, light)) = read_from(&moving, x, y) {
                shifts.push((dx, dy));
                lights.push(light / (1.0 - scrim_alpha((y as f32 + 0.5) / SCREEN.1 as f32)));
            }
        }
        assert!(shifts.len() > 3000, "{}", shifts.len());
        // LumaGlass's amplitude is 4 px of 1920: two pixels a unit of the field across the Show's 960, half that down
        // its 480, and the fields add up to less than three units. A few pixels, never eight.
        let most = shifts
            .iter()
            .map(|&(dx, dy)| dx.abs().max(dy.abs()))
            .fold(0.0, f32::max);
        assert!(
            (1.5..6.0).contains(&most),
            "the most it moved a pixel: {most}"
        );
        // Its folds are lit and shaded by up to about a tenth, and average out.
        let (low, high) = lights
            .iter()
            .fold((9.0f32, 0.0f32), |(l, h), &v| (l.min(v), h.max(v)));
        let mean = lights.iter().sum::<f32>() / lights.len() as f32;
        assert!(
            low > 0.85 && high < 1.15 && high - low > 0.05,
            "{low}..{high}"
        );
        assert!((mean - 1.0).abs() < 0.05, "{mean}");
        // With no intensity nothing is read from anywhere else, and nothing is lit (to within the dither's half level,
        // which moves what a pixel says of where it was read from by up to 0.15 px).
        let still = silk(&mut rig, 0.0, 12.0, waves());
        for (x, y) in readable() {
            if let Some((dx, dy, _)) = read_from(&still, x, y) {
                assert!(dx.abs() < 0.35 && dy.abs() < 0.35, "({x}, {y}): {dx}, {dy}");
            }
        }
    }

    #[test]
    fn the_silks_optional_extras_are_off_by_default_and_darken_the_corners_gleam_or_glow_when_asked()
     {
        let Some(mut rig) = rig() else { return };
        let plain = silk(&mut rig, 0.0, 0.0, flat(150));
        let (centre, corner) = ((480, 240), (6, 6));
        let brightest = |shot: &Shot| pixels(shot).map(|(_, _, p)| p[0]).max().unwrap();
        // The vignette darkens the corners and leaves the middle.
        rig.renderer.set_silk_look(Look {
            vignette: 1.0,
            ..Look::default()
        });
        let vignetted = silk(&mut rig, 0.0, 0.0, flat(150));
        assert_eq!(
            vignetted.pixel(centre.0, centre.1),
            plain.pixel(centre.0, centre.1)
        );
        assert!(
            i32::from(vignetted.pixel(corner.0, corner.1)[0]) + 25
                < i32::from(plain.pixel(corner.0, corner.1)[0])
        );
        // The specular ridges add light on the crests of the folds.
        rig.renderer.set_silk_look(Look {
            specular: 1.0,
            ..Look::default()
        });
        let gleaming = silk(&mut rig, 0.0, 0.0, flat(150));
        assert!(brightest(&gleaming) > brightest(&plain) + 15);
        // The bloom makes what is bright in the picture glow.
        rig.renderer.set_silk_look(Look {
            bloom: 1.0,
            ..Look::default()
        });
        let glowing = silk(&mut rig, 0.0, 0.0, flat(200));
        let unlit = silk(&mut rig, 0.0, 0.0, flat(60));
        assert!(glowing.pixel(centre.0, centre.1)[0] > 230);
        assert!(
            unlit.pixel(centre.0, centre.1)[0] < 100,
            "nothing below 0.3 glows"
        );
        // And back to none.
        rig.renderer.set_silk_look(Look::default());
        assert_eq!(
            silk(&mut rig, 0.0, 0.0, flat(150)).pixel(corner.0, corner.1),
            plain.pixel(corner.0, corner.1)
        );
    }

    #[test]
    fn silk_at_consecutive_steps_is_almost_the_same_picture() {
        // A step of the wallpaper is 33 ms: it must not jump. The most any pixel's source moves in one is a fraction of
        // a pixel.
        let Some(mut rig) = rig() else { return };
        let a = silk(&mut rig, 1.0, 40.0, waves());
        let b = silk(&mut rig, 1.0, 40.0335, waves());
        let mut most = 0.0f32;
        for (x, y) in readable() {
            if let (Some(p), Some(q)) = (read_from(&a, x, y), read_from(&b, x, y)) {
                most = most.max((p.0 - q.0).abs()).max((p.1 - q.1).abs());
            }
        }
        // The fields' fastest is 0.9 radians a second, of a shift of at most about five pixels: about a tenth of a pixel
        // a step, and readings good to a tenth. A jump, like a phase that wrapped badly, would be pixels.
        assert!(most < 0.4, "{most}");
    }

    #[test]
    fn a_month_of_running_is_the_same_silk_as_the_start_of_the_month_one_period_on() {
        // Every field of the silk repeats every 2 pi over its rate, so at a time that is a whole number of periods of all of
        // them (a multiple of 2 pi / 0.1 seconds, the rates being multiples of 0.1) the picture is what it was at 0.
        let period = f64::from(std::f32::consts::TAU) * 10.0;
        let Some(mut rig) = rig() else { return };
        let start = silk(&mut rig, 1.0, 0.0, waves());
        let later = silk(&mut rig, 1.0, 40_000.0 * period, waves());
        let mut most = 0.0f32;
        for (x, y) in readable() {
            if let (Some(p), Some(q)) = (read_from(&start, x, y), read_from(&later, x, y)) {
                most = most.max((p.0 - q.0).abs()).max((p.1 - q.1).abs());
            }
        }
        assert!(most < 0.2, "{most}");
    }

    fn aurora(colours: [Rgba; 4]) -> WallpaperKind {
        WallpaperKind::Aurora { colours }
    }

    #[test]
    fn an_aurora_is_clouds_of_its_own_colours_over_a_dark_ground() {
        let Some(mut rig) = rig() else { return };
        let primaries = [
            Rgba(255, 0, 0, 255),
            Rgba(0, 255, 0, 255),
            Rgba(0, 0, 255, 255),
            Rgba(255, 255, 255, 255),
        ];
        // Without intensity only the ground is left: dark.
        let bare = rig.draw(&[layer(1, aurora(primaries), 0.0, 10.0)]);
        assert!(pixels(&bare).all(|(_, _, p)| p[..3].iter().all(|&c| c < 40)));
        // With it the clouds show, each of its colours in places, and it moves.
        let mut dominant = [false; 3];
        let mut brightest = 0u8;
        let mut frames = Vec::new();
        for phase in [0.0, 50.0, 100.0, 150.0] {
            let shot = rig.draw(&[layer(1, aurora(primaries), 1.0, phase)]);
            shot.save(&format!("aurora-{phase}"));
            for (_, _, p) in pixels(&shot).step_by(7) {
                brightest = brightest.max(p[0].max(p[1]).max(p[2]));
                for c in 0..3 {
                    let others = (0..3).filter(|&o| o != c).map(|o| p[o]).max().unwrap();
                    if p[c] > others.saturating_add(40) {
                        dominant[c] = true;
                    }
                }
            }
            frames.push(shot);
        }
        assert!(brightest > 120, "{brightest}");
        assert_eq!(
            dominant, [true; 3],
            "red, green and blue each rule somewhere"
        );
        let moved = pixels(&frames[0])
            .filter(|&(x, y, p)| differs(p, frames[1].pixel(x, y), 8))
            .count();
        assert!(
            moved > 100_000,
            "the clouds are somewhere else 50 s later: {moved}"
        );
        // And between two steps of a wallpaper at 30 fps they have barely moved.
        let next = rig.draw(&[layer(1, aurora(primaries), 1.0, 0.0335)]);
        let first = &frames[0];
        assert!(pixels(first).all(|(x, y, p)| !differs(p, next.pixel(x, y), 4)));
    }

    #[test]
    fn the_night_sky_is_dark_with_stars_that_turn_and_a_loop_that_ends_where_it_began() {
        let Some(mut rig) = rig() else { return };
        let sky =
            |rig: &mut Rig, phase: f64| rig.draw(&[layer(1, WallpaperKind::Starfield, 1.0, phase)]);
        let start = sky(&mut rig, 0.0);
        start.save("starfield");
        let stars = |shot: &Shot| -> Vec<(u32, u32)> {
            pixels(shot)
                .filter(|&(_, _, p)| luma(p) > 100.0)
                .map(|(x, y, _)| (x, y))
                .collect()
        };
        let here = stars(&start);
        assert!((60..1500).contains(&here.len()), "{}", here.len());
        // The ground between them is dark.
        let dark = pixels(&start).filter(|&(_, _, p)| luma(p) < 40.0).count();
        assert!(dark > 440_000, "{dark}");
        // Ten minutes on the sky has turned a third of a turn: the stars are in other places.
        let later = sky(&mut rig, 600.0);
        let there = stars(&later);
        assert!((60..1500).contains(&there.len()), "{}", there.len());
        let both = here.iter().filter(|p| there.contains(p)).count();
        assert!(both * 5 < here.len(), "{both} of {} stayed", here.len());
        // The sky turns once in 30 minutes and twinkles in loops that divide it: 30 minutes on, or a month on, it is what it
        // was, exactly.
        for phase in [1800.0, 30.0 * 86_400.0] {
            let again = sky(&mut rig, phase);
            assert!(
                pixels(&start).all(|(x, y, p)| p == again.pixel(x, y)),
                "at {phase} seconds"
            );
        }
        // Stars twinkle: a few seconds on, with the sky hardly turned, the same stars are brighter or dimmer.
        let twinkled = sky(&mut rig, 2.0);
        let changed = here
            .iter()
            .filter(|&&(x, y)| (luma(start.pixel(x, y)) - luma(twinkled.pixel(x, y))).abs() > 25.0)
            .count();
        assert!(changed > here.len() / 10, "{changed} of {}", here.len());
    }

    /// The mean of `channel` over each `size`-pixel square of `shot`, in reading order.
    fn square_means(shot: &Shot, size: u32, channel: usize) -> Vec<f32> {
        let mut means = Vec::new();
        for top in (0..=SCREEN.1 - size).step_by(size as usize) {
            for left in (0..=SCREEN.0 - size).step_by(size as usize) {
                let sum: u32 = (0..size)
                    .flat_map(|y| (0..size).map(move |x| (x, y)))
                    .map(|(x, y)| u32::from(shot.pixel(left + x, top + y)[channel]))
                    .sum();
                means.push(sum as f32 / (size * size) as f32);
            }
        }
        means
    }

    /// The share of neighbouring pairs across a row, in the rows `rows` of `shot`, that are not the same colour.
    fn share_changing_across(shot: &Shot, rows: std::ops::Range<u32>) -> f32 {
        let pairs = rows.clone().count() as u32 * (SCREEN.0 - 1);
        let changing = rows
            .flat_map(|y| (1..SCREEN.0).map(move |x| (x, y)))
            .filter(|&(x, y)| shot.pixel(x, y) != shot.pixel(x - 1, y))
            .count();
        changing as f32 / pairs as f32
    }

    #[test]
    fn a_field_of_colour_shows_the_fractions_of_a_level_that_eight_bits_cannot_hold() {
        let Some(mut rig) = rig() else { return };
        // An aurora with no colours is its dark ground, the same all over: 0.040, 0.036 and 0.068, lifted by
        // 1 - exp(-1.15 c), which is 11.465, 10.342 and 19.180 of the 255 steps a channel has.
        let shot = rig.draw(&[layer(1, aurora([Rgba(0, 0, 0, 255); 4]), 1.0, 0.0)]);
        shot.save("aurora-ground");
        for (channel, exact) in [11.465f32, 10.342, 19.180].into_iter().enumerate() {
            // Rounded, every pixel would be 11, 10 or 19 and no square would average the fraction; with noise of a step
            // that the rounding follows, each does, to a twentieth of a step.
            for mean in square_means(&shot, 16, channel) {
                assert!(
                    (mean - exact).abs() < 0.05,
                    "channel {channel}: {mean} against {exact}"
                );
            }
        }
    }

    #[test]
    fn silk_and_liquid_keep_the_fractions_of_a_level_that_eight_bits_cannot_hold() {
        let Some(mut rig) = rig() else { return };
        // A flat picture of 101 at 85 % opacity is 85.85 levels, and under silk's scrim a little darker going down the
        // screen: the light and the scrim are smooth, and only the noise before the rounding lets a square show the fraction.
        for kind in [WallpaperKind::Silk, WallpaperKind::Liquid] {
            rig.set_pixels(1, SCREEN, &flat(101));
            let mut wall = layer(1, kind, 0.0, 0.0);
            wall.props.opacity = 0.85;
            wall.rest = wall.props;
            let shot = rig.draw(&[wall]);
            for (n, mean) in square_means(&shot, 16, 0).into_iter().enumerate() {
                // The scrim's mean over the square's rows (it has a kink, at 55 % of the height).
                let top = (n as u32 / (SCREEN.0 / 16)) * 16;
                let scrim = |row: u32| match kind {
                    WallpaperKind::Silk => {
                        scrim_alpha((top + row) as f32 / SCREEN.1 as f32 + 0.5 / 480.0)
                    }
                    _ => 0.0,
                };
                let exact = 85.85 * (1.0 - (0..16).map(scrim).sum::<f32>() / 16.0);
                assert!(
                    (mean - exact).abs() < 0.1,
                    "{kind:?} square {n}: {mean} against {exact}"
                );
            }
        }
    }

    // A gradient rounded to eight bits changes level once in tens of pixels along a row and the rows above and below
    // change at the same places: a band. Dithered, the levels it falls between mix along the row, and many of the
    // neighbours across differ.

    #[test]
    fn the_sky_is_dithered_and_does_not_break_into_bands() {
        let Some(mut rig) = rig() else { return };
        let sky = rig.draw(&[layer(1, WallpaperKind::Starfield, 0.0, 0.0)]);
        sky.save("sky-ground");
        let across = share_changing_across(&sky, 0..40);
        assert!(across > 0.2, "the sky's top: {across}");
    }

    #[test]
    fn the_fog_is_dithered_and_does_not_break_into_bands() {
        let Some(mut rig) = rig() else { return };
        let ground = overlay_ground(&mut rig, 100);
        let fog = rig.draw(&[ground, overlay(WallpaperKind::Weather(Weather::Fog), 12.0)]);
        fog.save("fog-over-grey");
        let across = share_changing_across(&fog, 200..240);
        assert!(across > 0.2, "the fog: {across}");
    }

    fn overlay_ground(rig: &mut Rig, grey: u8) -> Drawn {
        rig.set_pixels(1, SCREEN, &flat(grey));
        drawn(1, SCREEN, Content::Image, Props::at(0.0, 0.0))
    }

    fn overlay(kind: WallpaperKind, phase: f64) -> Drawn {
        layer(2, kind, 1.0, phase)
    }

    fn fireflies() -> WallpaperKind {
        WallpaperKind::Fireflies {
            colour: Rgba(246, 193, 119, 255),
        }
    }

    #[test]
    fn the_particles_of_an_overlay_are_all_it_draws_and_the_ground_shows_between_them() {
        let Some(mut rig) = rig() else { return };
        let ground = overlay_ground(&mut rig, 100);
        let base = [100, 100, 100, 255];
        for (name, kind, phase, most) in [
            ("fireflies", fireflies(), 21.7, 0.08),
            ("rain", WallpaperKind::Weather(Weather::Rain), 5.3, 0.12),
            ("snow", WallpaperKind::Weather(Weather::Snow), 33.3, 0.12),
        ] {
            let shot = rig.draw(&[ground, overlay(kind, phase)]);
            shot.save(&format!("overlay-{name}"));
            let changed = pixels(&shot)
                .filter(|&(_, _, p)| differs(p, base, 1))
                .count();
            let share = changed as f32 / (SCREEN.0 * SCREEN.1) as f32;
            assert!((0.0015..most).contains(&share), "{name}: {share}");
            // It only adds light.
            assert!(
                pixels(&shot).all(|(_, _, p)| p[..3].iter().all(|&c| c >= 99)),
                "{name}"
            );
            assert!(
                pixels(&shot).any(|(_, _, p)| p[0] > 140),
                "{name} has bright parts"
            );
        }
    }

    #[test]
    fn the_weather_that_covers_the_sky_only_lightens_or_darkens_it_a_little() {
        let Some(mut rig) = rig() else { return };
        let ground = overlay_ground(&mut rig, 100);
        for (name, weather) in [
            ("sun", Weather::Sun),
            ("clouds", Weather::Clouds),
            ("fog", Weather::Fog),
        ] {
            let shot = rig.draw(&[ground, overlay(WallpaperKind::Weather(weather), 12.0)]);
            shot.save(&format!("overlay-{name}"));
            let mean: f32 = pixels(&shot)
                .step_by(11)
                .map(|(_, _, p)| luma(p))
                .sum::<f32>()
                / (SCREEN.0 * SCREEN.1 / 11) as f32;
            // Lighter than the ground, but a veil and not a wall.
            assert!(
                pixels(&shot).all(|(_, _, p)| p[..3].iter().all(|&c| c >= 99)),
                "{name}"
            );
            assert!(mean > 103.0 && mean < 190.0, "{name}: mean {mean}");
            let changed = pixels(&shot)
                .filter(|&(_, _, p)| differs(p, [100, 100, 100, 255], 2))
                .count();
            assert!(
                changed > SCREEN.0 as usize * SCREEN.1 as usize / 10,
                "{name}: {changed}"
            );
        }
    }

    #[test]
    fn a_storm_darkens_what_is_below_and_flashes_with_the_lightning() {
        let Some(mut rig) = rig() else { return };
        let ground = overlay_ground(&mut rig, 100);
        let storm = WallpaperKind::Weather(Weather::Storm);
        // A moment with no lightning: darker than the ground, with drops that are lighter.
        assert_eq!(lightning(0.3), 0.0);
        let calm = rig.draw(&[ground, overlay(storm, 0.3)]);
        calm.save("overlay-storm");
        let mean: f32 = pixels(&calm)
            .step_by(11)
            .map(|(_, _, p)| luma(p))
            .sum::<f32>()
            / (SCREEN.0 * SCREEN.1 / 11) as f32;
        assert!((55.0..92.0).contains(&mean), "{mean}");
        assert!(
            pixels(&calm).any(|(_, _, p)| p[0] > 130),
            "the rain is lighter than the ground"
        );
        // The peak of some flash: the whole screen lights up.
        let peak = (0..2_000)
            .map(|i| f64::from(i) * 0.01)
            .max_by(|a, b| lightning(*a).total_cmp(&lightning(*b)))
            .unwrap();
        assert!(lightning(peak) > 0.9);
        let flash = rig.draw(&[ground, overlay(storm, peak)]);
        flash.save("overlay-storm-flash");
        let lit: f32 = pixels(&flash)
            .step_by(11)
            .map(|(_, _, p)| luma(p))
            .sum::<f32>()
            / (SCREEN.0 * SCREEN.1 / 11) as f32;
        assert!(lit > mean + 40.0, "{lit} against {mean}");
    }

    #[test]
    fn a_liquid_wallpaper_ripples_where_it_is_tapped_and_only_for_a_while() {
        let Some(mut rig) = rig() else { return };
        rig.set_pixels(1, SCREEN, &waves());
        let water = |rig: &mut Rig, taps: &[(f32, f32, f32)]| {
            let mut water = layer(1, WallpaperKind::Liquid, 1.0, 2.0);
            for (n, &(x, y, age_s)) in taps.iter().enumerate() {
                water.ripples.items[n] = crate::wallpaper::Ripple { x, y, age_s };
            }
            water.ripples.count = taps.len();
            rig.draw(&[water])
        };
        let calm = water(&mut rig, &[]);
        calm.save("liquid-calm");
        let tapped = water(&mut rig, &[(480.0, 240.0, 0.8)]);
        tapped.save("liquid-tapped");
        // The ring is 190 px a second out from the tap: at 0.8 s it is at 152 px, and the picture is bent there.
        let bend = |shot: &Shot, x: u32, y: u32| -> Option<f32> {
            let (a, b) = (read_from(shot, x, y)?, read_from(&calm, x, y)?);
            Some((a.0 - b.0).abs().max((a.1 - b.1).abs()))
        };
        let (mut ring, mut far, mut inside) = (0.0f32, 0.0f32, 0.0f32);
        for (x, y) in readable() {
            let Some(bent) = bend(&tapped, x, y) else {
                continue;
            };
            let distance = ((x as f32 - 480.0).powi(2) + (y as f32 - 240.0).powi(2)).sqrt();
            if (110.0..200.0).contains(&distance) {
                ring = ring.max(bent);
            } else if distance > 400.0 {
                far = far.max(bent);
            } else if distance < 30.0 {
                inside = inside.max(bent);
            }
        }
        assert!(
            (1.2..12.0).contains(&ring),
            "the ring bends the picture by a few pixels: {ring}"
        );
        // Far from it there is the swell alone, which the two grids read alike within a fraction of a pixel.
        assert!(far < 0.3, "{far}");
        assert!(inside < 1.0, "the middle is calm again: {inside}");
        // A tap that has just landed is not there yet, and one that has run its course is gone: no jump at either end.
        for age in [0.0, 2.5, 3.0] {
            let shot = water(&mut rig, &[(480.0, 240.0, age)]);
            let most = readable()
                .filter_map(|(x, y)| bend(&shot, x, y))
                .fold(0.0, f32::max);
            assert!(most < 0.3, "at age {age}: {most}");
        }
        // A wallpaper at rest (a cached picture's) has none of it: see `the_picture_of_the_layers_at_rest...`.
    }

    fn glass(backdrop: Backdrop) -> Glass {
        Glass { corner_radius: 0.0,
        tint: Rgba(0, 0, 0, 0),
        edge: Rgba(0, 0, 0, 0),
        edge_width: 0.0,
        highlight: Rgba(0, 0, 0, 0),
        sheen: None,
        shadows: [None, None],
        depth: None,
        backdrop,
        saturation: 1.0, refraction: 0.0, edge_lens: None, tint_gradient: None, }
    }

    fn glass_layer(id: u32, backdrop: Backdrop, at: (f32, f32), size: (u32, u32)) -> Drawn {
        drawn(
            id,
            size,
            Content::Glass(glass(backdrop)),
            Props::at(at.0, at.1),
        )
    }

    /// Bars of black and white, 40 px each: something for a blur to smear that a warp moves.
    fn bars() -> Vec<u8> {
        picture(|x, _| if (x / 40) % 2 == 0 { [235; 3] } else { [20; 3] })
    }

    fn interior_differs(a: &Shot, b: &Shot, by: u8) -> bool {
        (320..580).any(|x| (200..320).any(|y| differs(a.pixel(x, y), b.pixel(x, y), by)))
    }

    #[test]
    fn a_cached_backdrop_shows_a_wallpaper_at_rest_and_is_not_made_again_as_it_moves() {
        let Some(mut rig) = rig() else { return };
        rig.set_pixels(1, SCREEN, &bars());
        let cached = Backdrop::Cached { radius: 20.0 };
        let scene = |phase: f64| {
            [
                layer(1, WallpaperKind::Silk, 1.0, phase),
                glass_layer(2, cached, (300.0, 180.0), (300, 160)),
            ]
        };
        let first = rig.draw(&scene(0.0));
        assert_eq!(rig.report.backdrops, 1);
        let later = rig.draw(&scene(5.0));
        assert_eq!(
            rig.report.backdrops, 0,
            "a wallpaper's motion does not retake a cached picture"
        );
        first.save("cached-over-silk");
        // The glass shows the same picture; the wallpaper around it has moved on.
        assert!(!interior_differs(&first, &later, 0));
        assert!(
            pixels(&first)
                .any(|(x, y, p)| !(300..600).contains(&x) && differs(p, later.pixel(x, y), 8)),
            "outside the glass the wallpaper moves"
        );
        // And what it shows is the wallpaper at rest, whatever moment the picture was taken at.
        drop(rig);
        let mut other = crate::testgl::rig().unwrap();
        other.set_pixels(1, SCREEN, &bars());
        let taken_late = other.draw(&scene(9.0));
        assert!(!interior_differs(&first, &taken_late, 1));
    }

    #[test]
    fn a_live_backdrop_over_a_moving_wallpaper_is_made_again_for_every_frame_and_follows_it() {
        let Some(mut rig) = rig() else { return };
        rig.set_pixels(1, SCREEN, &bars());
        let live = Backdrop::Live {
            radius: 8.0,
            downsample: 2,
        };
        let scene = |phase: f64| {
            [
                layer(1, WallpaperKind::Silk, 1.0, phase),
                glass_layer(2, live, (300.0, 180.0), (300, 160)),
            ]
        };
        let first = rig.draw(&scene(0.0));
        assert_eq!(rig.report.backdrops, 1);
        let later = rig.draw(&scene(5.0));
        assert_eq!(rig.report.backdrops, 1);
        assert!(
            interior_differs(&first, &later, 3),
            "the glass sees the warped bars, not the ones at rest"
        );
    }

    #[test]
    fn an_overlay_is_not_part_of_the_picture_of_the_layers_at_rest_but_is_of_a_live_one() {
        let Some(mut rig) = rig() else { return };
        rig.set_pixels(1, SCREEN, &flat(90));
        let ground = drawn(1, SCREEN, Content::Image, Props::at(0.0, 0.0));
        let fog = layer(2, WallpaperKind::Weather(Weather::Fog), 1.0, 12.0);
        let (cached, live) = (
            Backdrop::Cached { radius: 20.0 },
            Backdrop::Live {
                radius: 20.0,
                downsample: 4,
            },
        );
        let at = |backdrop| glass_layer(3, backdrop, (300.0, 240.0), (300, 200));
        let without = rig.draw(&[ground, at(cached)]);
        let with_cached = rig.draw(&[ground, fog, at(cached)]);
        let with_live = rig.draw(&[ground, fog, at(live)]);
        with_live.save("live-over-fog");
        let inside = |a: &Shot, b: &Shot| {
            (320..580).all(|x| (260..420).all(|y| !differs(a.pixel(x, y), b.pixel(x, y), 1)))
        };
        assert!(
            inside(&without, &with_cached),
            "the fog is not in a cached picture"
        );
        assert!(!inside(&without, &with_live), "and it is in a live one");
        // Outside the glass the fog is drawn either way.
        assert!(differs(
            without.pixel(100, 440),
            with_cached.pixel(100, 440),
            3
        ));
    }

    #[test]
    fn a_base_wallpaper_below_full_opacity_lets_what_is_under_it_show_and_at_full_opacity_covers_it()
     {
        let Some(mut rig) = rig() else { return };
        rig.set_pixels(1, SCREEN, &picture(|_, _| [200, 0, 0]));
        let ground = drawn(1, SCREEN, Content::Image, Props::at(0.0, 0.0));
        let colours = [Rgba(0, 0, 255, 255); 4];
        let over = |opacity: f32| {
            let mut over = layer(2, aurora(colours), 1.0, 10.0);
            over.props.opacity = opacity;
            over
        };
        let covered = rig.draw(&[ground, over(1.0)]);
        assert!(
            pixels(&covered).all(|(_, _, p)| p[0] < 60),
            "the red ground is covered"
        );
        let half = rig.draw(&[ground, over(0.5)]);
        assert!(
            pixels(&half).all(|(_, _, p)| (95..=125).contains(&p[0]) && p[3] == 255),
            "half of red 200 shows through: {:?}",
            half.pixel(100, 100)
        );
    }

    #[test]
    fn a_wallpaper_is_drawn_where_its_layer_is_and_as_large_as_its_scale() {
        let Some(mut rig) = rig() else { return };
        let colours = [Rgba(255, 255, 255, 255); 4];
        let mut small = drawn(
            1,
            (200, 100),
            wallpaper(aurora(colours), 1.0),
            Props::at(100.0, 50.0),
        );
        let lit = |shot: &Shot, x, y| luma(shot.pixel(x, y)) > 8.0;
        let shot = rig.draw(&[small]);
        assert!(lit(&shot, 150, 80) && lit(&shot, 290, 140));
        assert!(
            !lit(&shot, 90, 80)
                && !lit(&shot, 310, 80)
                && !lit(&shot, 150, 40)
                && !lit(&shot, 150, 160)
        );
        // Scaled to half about its middle (200, 100) it covers 150..250 by 75..125.
        small.props.scale = 0.5;
        let shot = rig.draw(&[small]);
        assert!(lit(&shot, 200, 100) && lit(&shot, 155, 80));
        assert!(!lit(&shot, 120, 100) && !lit(&shot, 280, 100) && !lit(&shot, 200, 60));
        // A layer that is only half on the screen is cut where the screen ends.
        let shot = rig.draw(&[drawn(
            1,
            SCREEN,
            wallpaper(aurora(colours), 1.0),
            Props::at(500.0, 0.0),
        )]);
        assert!(lit(&shot, 700, 240) && !lit(&shot, 300, 240));
    }

    #[test]
    fn a_shape_over_a_wallpaper_is_drawn_over_it_and_the_next_layer_is_unharmed() {
        // A wallpaper leaves its own vertex buffer and program bound: what is drawn after it must not care.
        let Some(mut rig) = rig() else { return };
        let red = Content::Shape(Shape {
            corner_radius: 0.0,
            fill: Fill::Solid(Rgba(255, 0, 0, 255)),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        });
        let shot = rig.draw(&[
            layer(1, WallpaperKind::Starfield, 1.0, 0.0),
            drawn(2, (100, 100), red, Props::at(400.0, 200.0)),
            layer(3, fireflies(), 1.0, 5.0),
            drawn(4, (50, 50), red, Props::at(700.0, 300.0)),
        ]);
        assert_eq!(shot.pixel(450, 250), [255, 0, 0, 255]);
        assert_eq!(shot.pixel(725, 325), [255, 0, 0, 255]);
        assert_eq!(rig.report.drawn, 4);
    }
}
