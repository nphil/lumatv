//! Draws a frame's layers, back to front, into a render target. It owns the GL context and everything the layers
//! need on the GPU: the programs and one texture per image layer.

use crate::atlas::{self, Atlases};
use crate::backdrop::{self, Area, Ask, Backdrops, Refresh};
use crate::damage::{self, Damage, Piece, Region, Ring, Sight};
use crate::feather;
use crate::geometry::{Panel, PixRect, Placement, Ramp, Vertex};
use crate::gl::{GL_BLEND, GL_ONE, GL_ONE_MINUS_SRC_ALPHA, GL_SCISSOR_TEST, Gles, RenderTarget};
use crate::glass;
use crate::ladder;
use crate::live;
use crate::log::log;
use crate::mesh::MeshRenderer;
use crate::ops::Op;
use crate::plates::{Geometry, Sets};
use crate::prepared::{NoGroups, Source};
use crate::profile::{Part, Profile, Skips};
use crate::scene::Drawn;
use crate::shaders;
use crate::shm::ImageFile;
use crate::still::{self, Still};
use crate::tiles::{self, TileSet};
use crate::wallpapers::{self, Look, Wallpapers};
use compositor_protocol::{
    Backdrop, Content, Fill, LayerId, Props, Rgba, Shape, SourceId, frames, ideas,
};
use std::collections::HashMap;
use std::ffi::CStr;
use std::io;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(test)]
mod feather_tests;
#[cfg(test)]
mod glyph_tests;
mod prepare;
#[cfg(test)]
mod prepared_tests;
mod touch;
pub(crate) mod trim;

/// Textures waiting to be freed beyond which [`Renderer::tidy`] frees them all at once, whatever its budget: memory is
/// not for hoarding.
const GRAVEYARD_LIMIT: usize = 256;

/// Frames a plate of still glass is kept with nothing standing in it (ten seconds at 60 a second) before it is let go of: glass that
/// stops and starts again (a list that is scrolled and left) finds its plate where it was, and a screen that is changed for
/// another does not hold the first one's plates.
pub(crate) const PLATE_KEEP_FRAMES: u64 = 600;

/// How long a picture's file has to have been unread for the pages of its next read to be let go of after it ([`upload`]): a layer that
/// is changed every frame keeps its pages mapped, one that is changed now and then maps what it reads again.
const QUIET: Duration = Duration::from_secs(1);

/// The most a shape's quad is grown for anti-aliasing, in layer pixels, however small the layer is drawn.
const MAX_PAD: f32 = 4.0;

struct Texture {
    /// The GPU's texture: 0 until something needs it ([`Texture::made`]), which is the frame that draws the layer or the housekeeping
    /// that uploads what no frame drew.
    name: u32,
    file: Arc<ImageFile>,
    width: u32,
    height: u32,
    /// What the app has changed since the last upload; everything at first.
    dirty: Option<PixRect>,
    /// The number of the last frame that drew it (see `Renderer::frame`); 0 while none has.
    used: u64,
    /// Whether the texture holds what an upload or a clear put there, all of it: it is made with whatever the driver's memory held,
    /// and until the file's pixels are copied (the first upload) that is not something to show.
    defined: bool,
    /// When its pixels were last copied to the GPU: a file that was read at once before is being read again and again (a video's), and
    /// is not let go of ([`QUIET`]).
    uploaded: Option<Instant>,
    /// Whether every pixel on the GPU is opaque (alpha 255), learned from the pixels as they are copied ([`upload`]), and only
    /// for a picture at least as large as the screen: the one kind that can stand for the ground under everything
    /// (`scanout::plan::candidates`). A smaller picture is never looked at and never opaque here; nor is one whose change
    /// has not been copied yet.
    opaque: Option<bool>,
}

impl Texture {
    /// The GPU's texture, made if it is not yet. A texture is seven calls into the driver, 0.1 ms on the Show, and a screen that
    /// is mounted makes dozens of layers it does not show (the launcher's tiles, the pages of Settings behind the home): made
    /// when the layer is created they were the apply of the commit that mounts it. Made when the first upload needs them, the
    /// layers that are shown pay for theirs in the frame that draws them, as they always did, and the hidden ones in the
    /// housekeeping between frames ([`Renderer::tidy`]); one that is deleted before anything wanted it costs no texture at all.
    fn made(&mut self, gl: &Gles) -> u32 {
        if self.name == 0 {
            self.name = gl.new_texture(self.width, self.height);
        }
        self.name
    }
}

/// What [`Renderer::tidy`] and [`Renderer::prepare`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tidied {
    /// Textures freed, and pixels uploaded ahead of the frame that will draw them.
    pub freed: u32,
    pub uploaded_pixels: u64,
    /// Bytes of atlas chunks copied to their textures.
    pub uploaded_bytes: u64,
    /// Plates of prepared backdrops drawn and blurred.
    pub captured: u32,
    /// Buffers of producers' sources put on the GPU, drawn once off screen, or let go of (`live::Sources::tidy`).
    pub live: u32,
    /// There is more to do.
    pub more: bool,
}

struct ImageProgram {
    id: u32,
    texel: i32,
    opacity: i32,
    feather: feather::Slot,
}

/// Glyph runs: an atlas's coverage, in the colour of the palette that each span of quads (its ink) names.
struct GlyphProgram {
    id: u32,
    texel: i32,
    colour: i32,
    feather: feather::Slot,
}

/// A square, borderless rectangle of one colour.
struct SolidProgram {
    id: u32,
    from: i32,
    feather: feather::Slot,
}

/// A square, borderless rectangle with a gradient.
struct GradientProgram {
    id: u32,
    size: i32,
    gradient: i32,
    from: i32,
    delta: i32,
    feather: feather::Slot,
}

/// Any other shape.
struct RoundedProgram {
    id: u32,
    size: i32,
    gradient: i32,
    radius: i32,
    border_width: i32,
    scale: i32,
    pixel: i32,
    opacity: i32,
    from: i32,
    to: i32,
    border: i32,
    feather: feather::Slot,
}

/// Which of the three shape programs draws a shape. The cheap ones have no anti-aliased edge, so they are only used
/// where the quad's edges fall on whole pixels; the picture is then the same as the rounded program would draw,
/// and a layer that comes to rest does not visibly change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShapeKind {
    Solid,
    Gradient,
    Rounded,
}

fn shape_kind(shape: &Shape, props: &Props) -> ShapeKind {
    let on_whole_pixels = props.scale == 1.0 && props.x.fract() == 0.0 && props.y.fract() == 0.0;
    let square = shape.corner_radius <= 0.0 && shape.border_width <= 0.0;
    match shape.fill {
        _ if !(on_whole_pixels && square) => ShapeKind::Rounded,
        Fill::Solid(_) => ShapeKind::Solid,
        Fill::Linear { .. } => ShapeKind::Gradient,
    }
}

/// Whether a square shape covers what is under it completely, so that blending can be switched off (blending
/// costs the GPU a read of the pixel underneath).
pub(crate) fn covers(shape: &Shape, opacity: f32) -> bool {
    let opaque = |c: Rgba| c.3 == 255;
    opacity >= 1.0
        && match shape.fill {
            Fill::Solid(c) => opaque(c),
            Fill::Linear { from, to, .. } => opaque(from) && opaque(to),
        }
}

/// What drawing a frame cost and did, for the log and the frame statistics.
#[derive(Clone, Copy, Debug, Default)]
pub struct DrawReport {
    pub drawn: u32,
    pub uploaded_pixels: u64,
    /// Nanoseconds the frame spent copying image layers' pixels into their textures.
    pub upload_ns: u64,
    /// Backdrops blurred for this frame: each live one, and each cached one taken again.
    pub backdrops: u16,
    /// Mesh pictures drawn again for this frame: layers that something looks at (`looked_at`) and whose camera or style is
    /// not the one their picture shows.
    pub meshes: u16,
    /// The first GL error the frame raised, if any.
    pub gl_error: Option<u32>,
    /// What the frame changed from the frame before it, on the panel: nothing (or, for a frame drawn without the account
    /// of the frames before, the whole panel). `damage_px` counts the pixels of the tiles it changed, and `damage` is the
    /// rectangles they are cut into (`TileSet::region`).
    pub damage: Region,
    pub damage_px: u64,
    /// Nanoseconds the frame spent working out what changed and what its target owes (`damage_px` and the rest of it).
    pub damage_ns: u64,
    /// The panel pixels the frame drew: the whole panel for a frame drawn in full, the part its target owed otherwise; and
    /// the rectangles it drew them as (1 for a frame drawn whole, none for a frame that drew nothing).
    pub region_px: u64,
    pub rects: u16,
    /// Layers not drawn because an opaque layer above them covered them completely.
    pub culled: u32,
    /// A glass surface that has stopped moving is getting its refraction back (`ideas::ADAPTIVE`): frames are wanted until it has.
    pub fading: bool,
    /// The glass surfaces shown from a plate in this frame, whether it was made again in it or not (`ideas::STILL_GLASS`).
    pub still: u16,
    /// Where the frame's time went on the CPU, in nanoseconds (the trace's `tp`, `tq`, `tb`, `td`): placing every layer (bringing
    /// textures up to date, the quads of glyph runs, the pieces and the shadow meshes of glass), planning what the frame needs
    /// (backdrops, what covers what), making the backdrops, and drawing the frame itself.
    pub place_ns: u64,
    pub plan_ns: u64,
    pub backdrop_ns: u64,
    pub draw_ns: u64,
    /// Shadow meshes made in the frame for glass surfaces placed the first time or at another size, and the nanoseconds they took
    /// (the trace's `mb` and `mn`): a glass surface that has a mesh of its shape already makes none.
    pub shadow_meshes: u32,
    pub shadow_ns: u64,
    /// What the GL was asked in the frame (`gl::Calls`): draw calls, uniforms, program switches, texture binds, other state, the calls
    /// left out because the state was set already, and the bytes of vertices handed over. In the first report of a frame.
    pub gl: crate::gl::Calls,
}

impl DrawReport {
    /// Adds the report of another render target of the same frame: what was drawn is summed, and what changed is the
    /// union of what each target's frame changed.
    pub fn add(&mut self, other: &DrawReport) {
        self.drawn += other.drawn;
        self.uploaded_pixels += other.uploaded_pixels;
        self.upload_ns += other.upload_ns;
        self.backdrops += other.backdrops;
        self.meshes += other.meshes;
        self.gl_error = self.gl_error.or(other.gl_error);
        self.damage.union(&other.damage);
        self.damage_px = self.damage_px.max(other.damage_px);
        self.damage_ns += other.damage_ns;
        self.region_px += other.region_px;
        self.rects += other.rects;
        self.culled += other.culled;
        self.fading |= other.fading;
        self.still += other.still;
        self.place_ns += other.place_ns;
        self.plan_ns += other.plan_ns;
        self.backdrop_ns += other.backdrop_ns;
        self.draw_ns += other.draw_ns;
        self.shadow_meshes += other.shadow_meshes;
        self.shadow_ns += other.shadow_ns;
        self.gl.add(&other.gl);
    }
}

/// Whether layers are drawn as they are now, or as they rest (without the animation that is running on them, the
/// wallpapers still): the second is what a cached backdrop is a picture of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Now,
    Rest,
}

fn props_of(layer: &Drawn, view: View) -> &Props {
    match view {
        View::Now => &layer.props,
        View::Rest => &layer.rest,
    }
}

fn is_shown(props: &Props) -> bool {
    props.visible && props.opacity > 0.0 && props.scale > 0.0
}

pub(crate) fn placement(layer: &Drawn, props: &Props) -> Placement {
    Placement {
        x: props.x,
        y: props.y,
        width: layer.width as f32,
        height: layer.height as f32,
        scale: props.scale,
    }
}

/// The programs that draw the kinds of layer that are one quad, or a run of them.
struct ProgramSet {
    image: ImageProgram,
    glyphs: GlyphProgram,
    solid: SolidProgram,
    gradient: GradientProgram,
    rounded: RoundedProgram,
}

impl ProgramSet {
    /// Compiles and links the five programs: as they are, or, for `feathered`, each with the fade of a feathered clip
    /// compiled in (`shaders::fade`; the programs the plain set has are left as they are).
    fn new(gl: &Gles, ndc: [f32; 2], feathered: bool) -> io::Result<Self> {
        let at = |program: u32, name: &CStr| gl.uniform_location(program, name);
        let make = |vertex: &CStr, fragment: &CStr| -> io::Result<u32> {
            let id = if feathered {
                gl.program(vertex, &shaders::fade::feathered(fragment))?
            } else {
                gl.program(vertex, fragment)?
            };
            gl.use_program(id);
            gl.uniform2f(at(id, c"u_ndc"), ndc[0], ndc[1]);
            Ok(id)
        };
        let slot = |program: u32| {
            if feathered {
                feather::Slot::new(at(program, c"u_feather"))
            } else {
                feather::Slot::none()
            }
        };
        let image_id = make(shaders::IMAGE_VERTEX, shaders::IMAGE_FRAGMENT)?;
        gl.uniform1i(at(image_id, c"u_image"), 0);
        let glyph_id = make(shaders::IMAGE_VERTEX, shaders::GLYPH_FRAGMENT)?;
        gl.uniform1i(at(glyph_id, c"u_atlas"), 0);
        let solid_id = make(shaders::SHAPE_VERTEX, shaders::FLAT_SOLID_FRAGMENT)?;
        let gradient_id = make(shaders::SHAPE_VERTEX, shaders::FLAT_GRADIENT_FRAGMENT)?;
        let rounded_id = make(shaders::SHAPE_VERTEX, shaders::ROUNDED_FRAGMENT)?;
        Ok(Self {
            image: ImageProgram {
                id: image_id,
                texel: at(image_id, c"u_texel"),
                opacity: at(image_id, c"u_opacity"),
                feather: slot(image_id),
            },
            glyphs: GlyphProgram {
                id: glyph_id,
                texel: at(glyph_id, c"u_texel"),
                colour: at(glyph_id, c"u_colour"),
                feather: slot(glyph_id),
            },
            solid: SolidProgram {
                id: solid_id,
                from: at(solid_id, c"u_from"),
                feather: slot(solid_id),
            },
            gradient: GradientProgram {
                id: gradient_id,
                size: at(gradient_id, c"u_size"),
                gradient: at(gradient_id, c"u_gradient"),
                from: at(gradient_id, c"u_from"),
                delta: at(gradient_id, c"u_delta"),
                feather: slot(gradient_id),
            },
            rounded: RoundedProgram {
                id: rounded_id,
                size: at(rounded_id, c"u_size"),
                gradient: at(rounded_id, c"u_gradient"),
                radius: at(rounded_id, c"u_radius"),
                border_width: at(rounded_id, c"u_border_width"),
                scale: at(rounded_id, c"u_scale"),
                pixel: at(rounded_id, c"u_pixel"),
                opacity: at(rounded_id, c"u_opacity"),
                from: at(rounded_id, c"u_from"),
                to: at(rounded_id, c"u_to"),
                border: at(rounded_id, c"u_border"),
                feather: slot(rounded_id),
            },
        })
    }
}

/// Every program that draws a layer that is one quad or a run of them, plain and with the fade of a feathered clip. A layer
/// is drawn by the feathered set only where its clip's fade reaches it (`Job::feather`): every other layer, and every layer
/// of a screen that has no feather, is drawn by the plain set exactly as it was before the feather existed.
struct Programs {
    plain: ProgramSet,
    feathered: ProgramSet,
}

pub struct Renderer {
    gl: Gles,
    panel: Panel,
    programs: Programs,
    glass: glass::Surfaces,
    backdrops: Backdrops,
    wallpapers: Wallpapers,
    profile: Profile,
    meshes: MeshRenderer,
    vertex_buffer: u32,
    textures: HashMap<LayerId, Texture>,
    /// The textures of layers that have gone, not yet freed: see [`Renderer::tidy`].
    graveyard: Vec<u32>,
    /// The number of the frame drawn last; 1 for the first.
    frame: u64,
    /// The frame's vertices, reused between frames.
    vertices: Vec<Vertex>,
    /// The layers as drawn now, and, in a frame that makes a cached backdrop, as they rest.
    jobs: Vec<Job>,
    rest_jobs: Vec<Job>,
    /// Whether a layer that an opaque layer covers completely is left undrawn (`LUMASHOW_DRAW`).
    culling: bool,
    /// The most rectangles a frame's region is drawn as (`damage::MAX_RECTS`; 1 draws the one cover of them all).
    rects: usize,
    /// What each set of render targets (one plane's) owes when it is drawn into, and the frame before it (see
    /// [`Renderer::draw_damaged`]): one account for a route with one plane.
    accounts: Vec<Account>,
    /// The layers of the frame drawn last that nothing of showed (`looked_at`).
    unseen: Vec<LayerId>,
    /// The glass ideas that are on (`compositor_protocol::ideas`).
    ideas: u32,
    /// The time of the frame being drawn, set by the engine.
    now_ns: u64,
    /// `ideas::ADAPTIVE`: each glass surface as it was placed the frame before and when it last moved, and how calm each
    /// layer of the frame being drawn is (0 in motion, 1 long at rest; empty while the idea is off).
    motion: HashMap<LayerId, (Props, Option<u64>)>,
    calm: Vec<f32>,
    /// How long refraction takes to come back once a surface stands still (`ideas::ADAPTIVE`); 0: at once.
    calm_ns: u64,
    /// `ideas::STILL_GLASS`: which surfaces stand in the plates, and the plates: textures of the panel's size, none while the
    /// idea is off. A plate is made when a surface is first shown from it and let go of when nothing has stood in it for
    /// [`PLATE_KEEP_FRAMES`] frames: a screen whose glass does not overlap needs one, and a plate is 1.8 MB of the GPU's memory
    /// that counts in the compositor's resident size.
    still: Still,
    plates: Vec<Option<RenderTarget>>,
    /// The number of the last frame in which something stood in each plate.
    plates_used: [u64; still::PLATES],
    /// Events for the client that the renderer's own work produced (a prepared generation made, an atlas upload done),
    /// handed on by the engine (`Renderer::drain_events`).
    events: Vec<compositor_protocol::Event>,
    /// Plate captures made and bytes of atlas and picture uploads done since the last frame event took the counts
    /// (`Renderer::take_frame_counts`).
    captures_since: u16,
    uploaded_since: u32,
    /// Glyph atlases and the runs drawn from them.
    atlases: Atlases,
    /// The prepared backdrops.
    sets: Sets,
    /// The vertex buffer a prepared backdrop's picture is composed from, and what composing one keeps between calls: its
    /// quads and the plates they show.
    compose_buffer: u32,
    compose_vertices: Vec<Vertex>,
    compose_draws: Vec<prepare::PlateDraw>,
    /// The layers of a frame as the renderer draws them (carrying the revisions it owns, with the glass adapted to motion),
    /// what the picture of the prepared backdrop shown on each is a picture of (a revision), and the layers of the plate
    /// being made: buffers reused from frame to frame.
    revised: Vec<Drawn>,
    revisions: Vec<u64>,
    plate_layers: Vec<Drawn>,
    /// Which runs of shared shadows a pass has cast, by run (`Drawing::draw`): a scratch buffer kept from frame to frame, so
    /// that no pass allocates one (a frame draws one pass per rectangle of its region).
    cast_done: Vec<bool>,
    /// The screen is out of date: work done between frames (a glyph list swapped in, a prepared generation published under
    /// glass that shows it) changed what the next frame draws. See [`Renderer::wants_frame`].
    stale: bool,
    /// What producers of frames have made (`live`): the buffers on the GPU that `Content::Live` layers draw from.
    live: live::Sources,
    /// Where a producer's buffer is drawn once, before it is announced (`live::Sources::tidy`).
    touch: touch::Touch,
    /// The steps of the Full ladder in force (`ladder`): 0 draws the layers as the app made them, which is all that the levels
    /// below Full ever do. Set by the engine for every frame (`Renderer::set_ladder`).
    ladder: u8,
}

/// What the frames drawn into one set of render targets leave to say about the next one.
struct Account {
    /// What each render target owes when it is drawn into.
    ring: tiles::Ring,
    /// The layers of the frame before, as a damaged frame shows them; `None` before the first, and after the targets were
    /// forgotten.
    prev: Option<Vec<Frozen>>,
    /// What the app has damaged of image layers' pictures since the frame before: where each picture changed.
    damaged: Vec<(LayerId, PixRect)>,
    /// The sets that held what each layer showed in the frame before that, for the next frame to fill again.
    spare: Vec<TileSet>,
}

impl Account {
    fn new(targets: usize, everything: Area) -> Self {
        Self {
            ring: tiles::Ring::new(targets, everything[2] as u32, everything[3] as u32),
            prev: None,
            damaged: Vec::new(),
            spare: Vec::new(),
        }
    }
}

/// What one render target is a picture of: layers `from..to` of a frame's list, drawn over nothing or over black. A
/// route puts the scene on one or several planes, and the ones a plane does not show are still in the pictures that the
/// glass on it blurs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    /// Which account of the frames drawn into it (see [`Renderer::set_targets`]): one for each plane.
    pub stack: usize,
    /// The layers it shows. Those below `from` are on a plane under it and those from `to` on are on planes over it.
    pub from: usize,
    pub to: usize,
    /// Cleared to nothing instead of opaque black: the plane is over another, and shows it where nothing is drawn.
    pub transparent: bool,
}

#[cfg(test)]
impl Surface {
    /// All of `layers` layers, over black: what a route with one plane draws.
    pub fn whole(layers: usize) -> Self {
        Self {
            stack: 0,
            from: 0,
            to: layers,
            transparent: false,
        }
    }
}

/// One render target of a frame: what it is a picture of, its place in its stack's ring (`None` draws all of it, and
/// needs no account of what it holds), and its framebuffer object.
#[derive(Clone, Copy, Debug)]
pub struct Draw {
    pub surface: Surface,
    /// Its place in its stack's ring: the renderer keeps the account of what each target of the ring owes and draws that. With
    /// none, the target is drawn whole, or in `areas`.
    pub slot: Option<usize>,
    pub target: u32,
    /// With no `slot`: the part of the target to draw, when the caller keeps the account (a compositor's ring of pages holds more
    /// than this renderer's layers). What the frame changes is still worked out and kept for the frame after
    /// ([`Renderer::damage`]), so that the caller can add it to its own account. `None` draws all of the target.
    pub areas: Option<Region>,
    /// Clear what is drawn before drawing it. `false` draws over what the target holds there: a plane that is drawn over windows.
    pub clear: bool,
}

/// What a frame knows of the layers of one render target, by layer: the layers, the part of each that is drawn in the
/// frame (`seen`), everything each draws (`extents`), what each asks of a backdrop, and what the picture of the prepared
/// backdrop on it (a glass surface's) is a picture of (0: none).
#[derive(Clone)]
struct Facts<'a> {
    /// The layers the target shows, among `layers`.
    members: std::ops::Range<usize>,
    layers: &'a [Drawn],
    seen: &'a [TileSet],
    extents: &'a [Option<Area>],
    asks: &'a [Option<Ask>],
    revisions: &'a [u64],
}

/// What the first render target of a frame that draws anything does for all of them.
#[derive(Default)]
struct Done {
    /// The vertices of the frame are uploaded.
    uploaded: bool,
    /// The pictures of the meshes that are looked at are drawn.
    meshed: bool,
    /// The backdrops' frame is open.
    opened: bool,
}

/// What planning a frame settles for every render target it is drawn into.
struct Frame {
    layers: Vec<Drawn>,
    panel_area: Area,
    /// The job of each layer, and everything each draws on the panel (seen or not).
    job_of: Vec<Option<usize>>,
    extents: Vec<Option<Area>>,
    /// What the picture of the prepared backdrop shown on each glass layer is a picture of (0 for the others).
    revisions: Vec<u64>,
    /// The glass surfaces' backdrops: the layers as the planner sees them, what the frame needs, its fingerprints, which
    /// surfaces share a run of shadows, and which cached ones are out of date.
    plan_layers: Vec<backdrop::Layer>,
    plan: Vec<backdrop::Backdrop>,
    fingerprints: Vec<u64>,
    casts: Casts,
    stale: Vec<bool>,
    /// What of all the layers the frame shows (by job), as a route with one plane would draw it, and the layers something
    /// looks at (`looked_at`).
    sights: Vec<Sight>,
    looked: Vec<bool>,
}

/// A frame whose layers are placed and whose backdrops are planned, waiting to be drawn into render targets one at a time
/// ([`Renderer::plan_frame`], [`Renderer::draw_planned`], [`Renderer::finish_frame`]).
pub struct Planned {
    frame: Frame,
    /// What planning cost and did: the report of the first target drawn has it.
    first: DrawReport,
    done: Done,
    /// What the frame changes from the one before, by stack, once it has been worked out ([`Renderer::damage`]).
    changed: Vec<Option<TileSet>>,
    backdrops: u16,
    uploaded: u64,
}

impl Planned {
    /// What the frame changes of stack `stack` from the frame before, in tiles, once [`Renderer::damage`] has worked it out.
    pub fn changed(&self, stack: usize) -> Option<&TileSet> {
        self.changed.get(stack)?.as_ref()
    }

    /// `region` as the passes the frame is cheapest drawn in, the way the renderer merges the passes of the pages it keeps the
    /// account of: two rectangles are one pass over their cover where that costs less than a pass each, a pass being worth its
    /// pixels and every layer it draws. For a compositor that draws its own pages in the rectangles they owe. The pass draws the
    /// layers `layers`, and the ones below them as one picture it copies (none for a range from 0).
    pub fn merged(&self, region: Region, layers: std::ops::Range<usize>) -> Region {
        Renderer::merge_passes(region, &self.frame, &self.frame.sights, layers)
    }
}

/// How long refraction takes to come back once a glass surface has stopped moving (`ideas::ADAPTIVE`): at once. Nitin
/// compared 200 ms, 100 ms and none on the Show (10-01) and could not tell them apart; with no fade a surface that stops
/// is the same in every frame after, so still glass can hold it three frames later instead of after the fade. The
/// compositor's environment can still set one to compare by eye: `LUMASHOW_CALM_MS=200`.
const CALM_NS: u64 = 0;

fn calm_ns_from_env() -> u64 {
    match std::env::var("LUMASHOW_CALM_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
    {
        Some(ms) => {
            log(format!(
                "adaptive glass: refraction comes back over {ms} ms (LUMASHOW_CALM_MS)"
            ));
            ms * 1_000_000
        }
        None => CALM_NS,
    }
}

/// How far refraction is back (0 none, 1 all) at `now_ns` for a surface that last moved at `moved_ns` (`None`: never, or
/// long enough ago), fading back over `fade_ns`; with no fade it is all back as soon as the surface stands.
fn calm_at(now_ns: u64, moved_ns: Option<u64>, fade_ns: u64) -> f32 {
    match moved_ns {
        None => 1.0,
        Some(at) if fade_ns == 0 => {
            if now_ns > at {
                1.0
            } else {
                0.0
            }
        }
        Some(at) => (now_ns.saturating_sub(at) as f32 / fade_ns as f32).clamp(0.0, 1.0),
    }
}
/// A blur that is held for a moving surface is made again every this many frames.
const HOLD_FRAMES: u64 = 4;

/// One layer ready to draw: its index in the frame's layer list, its scissor and where its vertices are in the
/// frame's.
struct Job {
    layer: usize,
    scissor: Option<PixRect>,
    draw: Vertices,
    /// Everything the layer draws on the panel (its body, and the fringe, shadows or light around it), less what its clip
    /// leaves out and what is off the panel.
    extent: Area,
    /// Where it certainly covers what is below it completely (nowhere, for a layer seen as it rests).
    cover: Cover,
    /// The fade of its clip's feathered edges, when the layer reaches into it: the layer is then drawn by the programs that
    /// have the fade (`None` for every layer that is not feathered or lies wholly where the fade is 1: the plain programs).
    feather: Option<Ramp>,
}

/// The side of the tiles the GPU (a Mali-T720) draws in, in pixels.
const GPU_TILE: i32 = 16;

/// The most rectangles one layer covers completely: the interior of a glass surface is three.
const MAX_COVER: usize = 3;

/// Rectangles of the panel that a layer covers completely, whatever is below it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Cover {
    rects: [Area; MAX_COVER],
    len: usize,
}

impl Cover {
    fn push(&mut self, area: Area) {
        if area[2] > area[0] && area[3] > area[1] && self.len < MAX_COVER {
            self.rects[self.len] = area;
            self.len += 1;
        }
    }

    fn rects(&self) -> &[Area] {
        &self.rects[..self.len]
    }
}

/// The pixels a rectangle covers completely: rounded inwards, where `bounds` rounds outwards.
pub(crate) fn inner_bounds(quad: &[Vertex; 4]) -> Area {
    let mut area = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for v in quad {
        area[0] = area[0].min(v[0]);
        area[1] = area[1].min(v[1]);
        area[2] = area[2].max(v[0]);
        area[3] = area[3].max(v[1]);
    }
    [
        area[0].ceil() as i32,
        area[1].ceil() as i32,
        area[2].floor() as i32,
        area[3].floor() as i32,
    ]
}

/// `area` a pixel in on every side: the anti-aliased edge of a rounded shape is not certainly opaque.
fn shrunk(area: Area, by: i32) -> Area {
    [area[0] + by, area[1] + by, area[2] - by, area[3] - by]
}

/// Whether a glass surface draws its interior opaque: its tint is, or it shows a backdrop. A prepared backdrop is shown once
/// a generation of its set is finished; until then the surface is drawn without one, the tint alone.
fn draws_opaque(glass: &compositor_protocol::Glass, sets: &Sets) -> bool {
    let backdrop = match glass.backdrop {
        Backdrop::None => false,
        Backdrop::Cached { .. } | Backdrop::Live { .. } => true,
        Backdrop::Prepared { id } => sets.has_picture(id),
    };
    glass.tint.3 == 255 || backdrop
}

/// Where `layer`, drawn as `props` place it, certainly covers what is below it completely, in panel pixels, and inside
/// `clip` (the layer's scissor) and the panel, and inside what `fade` leaves whole when its clip fades out where the layer
/// is (a faded pixel lets what is below show).
///
/// A shape with an opaque fill covers its body (a rounded one all but its corners and its anti-aliased edge); a glass
/// surface covers its interior when it draws it opaque: over a backdrop (which the frame is assumed to have made: a
/// surface whose picture could not be made is drawn without one, which is the same only if its tint is opaque, and the
/// GPU being out of memory for one picture is not a case this is careful about), or with a tint that is. Nothing else
/// says it covers what is below it: an image has whatever alpha its pixels have, and so has the coverage of a glyph run.
fn cover_of(
    layer: &Drawn,
    props: &Props,
    panel: &Panel,
    clip: Option<PixRect>,
    fade: Option<&Ramp>,
    sets: &Sets,
) -> Cover {
    let mut cover = Cover::default();
    if props.opacity < 1.0 {
        return cover;
    }
    let at = placement(layer, props);
    let size = (at.width, at.height);
    let mut add = |rect: [f32; 4], grow: i32| {
        let inside = shrunk(inner_bounds(&panel.quad_of(&at, rect)), grow);
        let visible = clip.map_or(Some(inside), |c| damage::intersect(&inside, &area_of(c)));
        let whole =
            visible.and_then(|a| fade.map_or(Some(a), |f| damage::intersect(&a, &f.solid())));
        if let Some(area) = whole.and_then(|a| damage::intersect(&a, &panel_area_of(panel))) {
            cover.push(area);
        }
    };
    match layer.content {
        Content::Shape(shape) if covers(&shape, props.opacity) => {
            let radius = glass::effective_radius(shape.corner_radius, size.0, size.1);
            if radius <= 0.0 {
                add([0.0, 0.0, size.0, size.1], 0);
            } else {
                add([radius, 0.0, size.0 - radius, size.1], 1);
                add([0.0, radius, size.0, size.1 - radius], 1);
            }
        }
        Content::Glass(glass) if draws_opaque(&glass, sets) => {
            let uniforms = glass::SurfaceUniforms::new(&glass, size, pad(props.scale));
            for rect in glass::slices(size, uniforms.radius, uniforms.inner, 0.0).core {
                add(rect, 0);
            }
        }
        Content::Shape(_)
        | Content::Glass(_)
        | Content::Image
        | Content::Wallpaper(_)
        | Content::Mesh(_)
        | Content::Glyphs(_)
        | Content::Live(_) => {}
    }
    cover
}

fn panel_area_of(panel: &Panel) -> Area {
    [0, 0, panel.width as i32, panel.height as i32]
}

/// Where a layer is drawn in a frame, and what the picture of the prepared backdrop on it is a picture of: what the frame is
/// compared with the one before by (`Frozen`).
#[derive(Clone, Copy)]
struct Now<'a> {
    /// The tiles of the layer that are drawn, and everything it draws, seen or not.
    seen: &'a TileSet,
    extent: Option<Area>,
    revision: u64,
}

/// What a layer changes from the frame before: where the tiles it was seen in (`[0]`) and is seen in (`[1]`) are drawn again
/// (within `within`, when it is a picture the app changed in places), and where what it draws is changed in the pictures behind
/// the glass above it (everything it drew, everything it draws).
#[derive(Clone, Copy, Default)]
struct Dirt {
    seen: [bool; 2],
    within: Option<Area>,
    extent: [Damage; 2],
}

/// The bytes of `pixels` RGBA pixels, for the count of bytes uploaded.
fn picture_bytes(pixels: u64) -> u32 {
    u32::try_from(pixels * 4).unwrap_or(u32::MAX)
}

/// The nanoseconds since `began`.
fn nanos(began: Instant) -> u64 {
    u64::try_from(began.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// A layer as the frame before showed it, for saying what has changed since.
struct Frozen {
    layer: Drawn,
    /// What the picture of the prepared backdrop on the layer was a picture of (0: it had none).
    revision: u64,
    /// Whether the render target shows the layer (it is one of the layers `Surface::from..to`), and not only has it in what
    /// its glass blurs. A layer that moves from one plane to another is drawn where it was not, and no longer where it was,
    /// though it is what it was in every other way.
    member: bool,
    /// The tiles of it that were drawn in the frame (`tiles::visible`); none when none of it was.
    seen: TileSet,
    /// Everything it draws, seen or not: what the pictures behind the glass above it are made of.
    extent: Option<Area>,
}

/// Where a layer's vertices are.
enum Vertices {
    /// A quad: the index of the first of its four vertices.
    Quad(i32),
    Glass(glass::Placed),
    /// The quads of a glyph run, two triangles each, a span of them for each ink.
    Glyphs(atlas::InkSpans),
    /// A wallpaper: drawn from meshes made at start, so nothing of it is in the frame's vertices.
    Wallpaper,
}

fn colour(c: Rgba) -> [f32; 4] {
    [
        f32::from(c.0) / 255.0,
        f32::from(c.1) / 255.0,
        f32::from(c.2) / 255.0,
        f32::from(c.3) / 255.0,
    ]
}

/// The shader's gradient direction and reciprocal length across a `width` × `height` layer for a gradient at
/// `angle_deg` (0 = left to right, 90 = top to bottom): the extent of the layer along the direction, so that the
/// gradient runs from one corner to the opposite one.
fn gradient_uniform(angle_deg: f32, width: f32, height: f32) -> [f32; 3] {
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    let length = (width * cos).abs() + (height * sin).abs();
    [cos, sin, 1.0 / length.max(1e-3)]
}

/// Anti-aliasing draws a fringe half a panel pixel outside a shape's edge, so its quad is grown a little.
fn pad(scale: f32) -> f32 {
    (1.0 / scale).min(MAX_PAD)
}

/// The rectangle of a layer that anything of it is drawn in, `[left, top, right, bottom]` in layer pixels: the body,
/// and the anti-aliased fringe or the shadows around it. One definition, so that what the backdrops are planned from
/// is what is drawn.
fn extent_of(content: &Content, props: &Props, size: (f32, f32)) -> [f32; 4] {
    match content {
        Content::Image
        | Content::Wallpaper(_)
        | Content::Mesh(_)
        | Content::Glyphs(_)
        | Content::Live(_) => [0.0, 0.0, size.0, size.1],
        Content::Shape(shape) => {
            let pad = match shape_kind(shape, props) {
                ShapeKind::Rounded => pad(props.scale),
                ShapeKind::Solid | ShapeKind::Gradient => 0.0,
            };
            [-pad, -pad, size.0 + pad, size.1 + pad]
        }
        Content::Glass(glass) => glass::Surfaces::extent(glass, size, pad(props.scale)),
    }
}

/// Adds a quad to the frame's vertices; returns the index of its first vertex.
fn push_quad(vertices: &mut Vec<Vertex>, quad: &[Vertex; 4]) -> i32 {
    let first = vertices.len() as i32;
    vertices.extend_from_slice(quad);
    first
}

/// The smallest a blur's picture is made under `ideas::HALF_RES`: a sixteenth of the panel each way.
const MAX_HALF_RES_DOWNSAMPLE: u8 = 16;

/// How many times smaller than the panel a cached backdrop's picture is: a wide blur needs fewer texels.
pub(crate) fn cached_downsample(radius: f32) -> u8 {
    if radius >= 40.0 {
        8
    } else if radius >= 10.0 {
        4
    } else {
        2
    }
}

fn area_of(rect: PixRect) -> Area {
    [
        rect.x as i32,
        rect.y as i32,
        (rect.x + rect.w) as i32,
        (rect.y + rect.h) as i32,
    ]
}

fn pix_rect_of(area: Area) -> Option<PixRect> {
    let (x0, y0) = (area[0].max(0), area[1].max(0));
    (area[2] > x0 && area[3] > y0).then(|| PixRect {
        x: x0 as u32,
        y: y0 as u32,
        w: (area[2] - x0) as u32,
        h: (area[3] - y0) as u32,
    })
}

/// The panel pixels a quad (rotated by whole quarters, so a rectangle) covers, rounded outwards, less what a clip
/// leaves out. `None` when nothing is left.
pub(crate) fn bounds(quad: &[Vertex; 4], clip: Option<PixRect>) -> Option<Area> {
    let mut area = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
    for v in quad {
        area[0] = area[0].min(v[0].floor() as i32);
        area[1] = area[1].min(v[1].floor() as i32);
        area[2] = area[2].max(v[0].ceil() as i32);
        area[3] = area[3].max(v[1].ceil() as i32);
    }
    let rect = pix_rect_of(area)?;
    Some(area_of(clip.map_or(Some(rect), |c| rect.intersect(c))?))
}

/// Which layers of a frame are looked at: the frame shows something of the layer (it has a job that is not hidden by what
/// is above it), or a picture behind a glass surface that shows has it in it. Such a picture is made of the layers below
/// its first surface, and is made this frame if it is a live one, or a cached one that is out of date (`stale`, by
/// backdrop of `plan`). `job_of` gives each layer's job, if it has one (a layer that is nowhere on the panel has none, and
/// no pixels for anyone to look at), and `sights` each job's sight. What is not looked at costs the frame nothing, and
/// nothing of it is seen, whatever it does.
fn looked_at(
    job_of: &[Option<usize>],
    sights: &[Sight],
    plan: &[backdrop::Backdrop],
    stale: &[bool],
) -> Vec<bool> {
    let seen = |layer: usize| job_of[layer].is_some_and(|job| sights[job] != Sight::Hidden);
    let pictured = plan
        .iter()
        .zip(stale)
        .filter(|(b, stale)| {
            (b.refresh == Refresh::Live || **stale) && b.users.iter().any(|&user| seen(user))
        })
        .map(|(b, _)| b.upto)
        .max()
        .unwrap_or(0);
    (0..job_of.len())
        .map(|layer| seen(layer) || (layer < pictured && job_of[layer].is_some()))
        .collect()
}

/// The panel pixels a glass surface's body is drawn in, with the fringe of its edge: where its plate holds it (`ideas::STILL_GLASS`).
/// `None` when it is nowhere on the panel. A surface under a feathered clip is held too: its plate is drawn with the fade of the clip's
/// edges in it, as the frame would draw it (the plate pass draws the body by the same programs, `Drawing::glass_draw`), and the key
/// of what stands (`still::Key`) holds the clip and its feather, so that a plate is made again when either changes.
fn plate_rect(panel: &Panel, layer: &Drawn) -> Option<Area> {
    let props = &layer.props;
    if !is_shown(props) {
        return None;
    }
    let at = placement(layer, props);
    let fringe = pad(props.scale);
    let body = panel.quad_of(
        &at,
        [-fringe, -fringe, at.width + fringe, at.height + fringe],
    );
    damage::intersect(&bounds(&body, None)?, &panel_area_of(panel))
}

/// What is in the live picture of `b` besides the surfaces that share it, as still glass looks at it: the layers below that show
/// anywhere in it. A wallpaper's own motion is left out (a picture of moving wallpapers is what the glass then keeps for a few
/// frames), but not the ripples of a tap.
fn under_of(jobs: &[Job], layers: &[Drawn], b: &backdrop::Backdrop) -> Vec<Drawn> {
    jobs.iter()
        .take_while(|job| job.layer < b.upto)
        .filter(|job| backdrop::overlaps(&job.extent, &b.area))
        .map(|job| {
            let layer = layers[job.layer];
            match layer.content {
                Content::Wallpaper(_) if layer.ripples.count == 0 => Drawn {
                    phase: 0.0,
                    ripples: crate::wallpaper::Ripples::default(),
                    ..layer
                },
                _ => layer,
            }
        })
        .collect()
}

impl Renderer {
    pub fn new(gl: Gles, panel: Panel) -> io::Result<Self> {
        let started = Instant::now();
        let ndc = [2.0 / panel.width as f32, 2.0 / panel.height as f32];
        let programs = Programs {
            plain: ProgramSet::new(&gl, ndc, false)?,
            feathered: ProgramSet::new(&gl, ndc, true)?,
        };
        let glass = glass::Surfaces::new(&gl, &panel)?;
        let backdrops = Backdrops::new(&gl, (panel.width, panel.height))?;
        let wallpapers = Wallpapers::new(&gl, (panel.width, panel.height), Look::default())?;
        let meshes = MeshRenderer::new(&gl)?;
        log(format!(
            "shader programs compiled and linked in {} ms",
            started.elapsed().as_millis()
        ));
        let vertex_buffer = gl.new_vertex_buffer();
        let compose_buffer = gl.new_vertex_buffer();
        let touch = touch::Touch::new(&gl);
        gl.enable(GL_BLEND);
        gl.blend_func(GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
        let mut renderer = Self {
            programs,
            glass,
            backdrops,
            wallpapers,
            profile: Profile::from_env(panel.width * panel.height),
            meshes,
            gl,
            panel,
            vertex_buffer,
            textures: HashMap::new(),
            graveyard: Vec::new(),
            frame: 0,
            vertices: Vec::new(),
            jobs: Vec::new(),
            rest_jobs: Vec::new(),
            culling: true,
            rects: damage::MAX_RECTS,
            accounts: vec![Account::new(
                0,
                [0, 0, panel.width as i32, panel.height as i32],
            )],
            unseen: Vec::new(),
            ideas: 0,
            now_ns: 0,
            motion: HashMap::new(),
            calm: Vec::new(),
            calm_ns: calm_ns_from_env(),
            still: Still::default(),
            plates: Vec::new(),
            plates_used: [0; still::PLATES],
            events: Vec::new(),
            captures_since: 0,
            uploaded_since: 0,
            atlases: Atlases::default(),
            sets: Sets::default(),
            compose_buffer,
            compose_vertices: Vec::new(),
            compose_draws: Vec::new(),
            revised: Vec::new(),
            revisions: Vec::new(),
            plate_layers: Vec::new(),
            cast_done: Vec::new(),
            stale: false,
            live: live::Sources::new(None),
            touch,
            ladder: 0,
        };
        renderer.set_ideas(ideas::DEFAULT);
        Ok(renderer)
    }

    /// Draws one frame of a scene that uses every program the real screens use (the three shape programs, a wallpaper of
    /// each kind that needs no picture, live and cached glass with every part of it, and its blurs) into a render target of
    /// its own, and throws it away. The GPU driver does some of its work the first time something is drawn that way, and
    /// whoever's screen that happens on would otherwise pay for it on the frame that shows it. Returns the GL error the
    /// frame raised, if any.
    pub fn warm_up(&mut self) -> Option<u32> {
        use compositor_protocol::{
            Backdrop, Depth, Fill, Glass, Rgba, Shadow, Shape, Sheen, Wallpaper, WallpaperKind,
            Weather,
        };
        let (w, h) = (self.panel.logical_width, self.panel.logical_height);
        let ink = Rgba(40, 36, 58, 255);
        let layer = |n: u32, width: f32, height: f32, content: Content, x: f32, y: f32| Drawn {
            id: LayerId(u32::MAX - n),
            width: width as u32,
            height: height as u32,
            content,
            props: Props::at(x, y),
            rest: Props::at(x, y),
            version: 1,
            phase: 0.0,
            ripples: Default::default(),
        };
        let wallpaper = |kind| {
            Content::Wallpaper(Wallpaper {
                kind,
                speed: 1.0,
                intensity: 0.7,
                fps: 60,
            })
        };
        let glass = |backdrop, depth| {
            Content::Glass(Glass {
                corner_radius: 24.0,
                tint: Rgba(30, 28, 44, 150),
                edge: Rgba(60, 60, 80, 80),
                edge_width: 1.0,
                highlight: Rgba(70, 70, 90, 90),
                sheen: Some(Sheen {
                    peak: 0.2,
                    floor: 0.05,
                    start: 0.2,
                    end: 0.8,
                }),
                shadows: [
                    Some(Shadow {
                        offset_y: 8.0,
                        blur: 24.0,
                        colour: Rgba(0, 0, 0, 90),
                    }),
                    Some(Shadow {
                        offset_y: 2.0,
                        blur: 6.0,
                        colour: Rgba(0, 0, 0, 60),
                    }),
                ],
                depth,
                backdrop,
                saturation: 1.2,
                refraction: 6.0,
            })
        };
        let shape = |radius, fill, border| {
            Content::Shape(Shape {
                corner_radius: radius,
                fill,
                border_width: border,
                border: Rgba(80, 80, 100, 120),
            })
        };
        let depth = Some(Depth {
            inset: false,
            offset: 4.0,
            blur: 8.0,
            light: Rgba(60, 60, 80, 60),
            shade: Rgba(0, 0, 0, 80),
        });
        // The pictures of the layers that have one, as the app's are: files in the shared-memory directory, zeroes.
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        let mut picture = |renderer: &mut Self, n: u32, width: f32, height: f32| -> bool {
            let id = LayerId(u32::MAX - n);
            let (width, height) = (width as u32, height as u32);
            let path = std::path::PathBuf::from(compositor_protocol::image_path(id));
            let file = std::fs::write(&path, vec![0u8; width as usize * height as usize * 4])
                .map_err(|e| e.to_string())
                .and_then(|()| ImageFile::open(&path, width, height));
            files.push(path);
            let Ok(file) = file else {
                return false;
            };
            renderer.apply(&Op::Create {
                id,
                width,
                height,
                z: 0,
                content: Content::Image,
                props: Props::at(0.0, 0.0),
                image: Some(Arc::new(file)),
                mesh: None,
            });
            true
        };
        let silk = picture(self, 20, w, h);
        let pictures: Vec<bool> = [(21, 400.0, 28.0), (22, 848.0, 64.0), (23, 120.0, 120.0)]
            .into_iter()
            .map(|(n, width, height)| picture(self, n, width, height))
            .collect();
        let mut layers = vec![
            layer(
                0,
                w,
                h,
                wallpaper(WallpaperKind::Aurora { colours: [ink; 4] }),
                0.0,
                0.0,
            ),
            layer(1, w, h, wallpaper(WallpaperKind::Starfield), 0.0, 0.0),
            layer(
                2,
                w,
                h,
                wallpaper(WallpaperKind::Fireflies {
                    colour: Rgba(200, 170, 90, 255),
                }),
                0.0,
                0.0,
            ),
            layer(
                3,
                w,
                h,
                wallpaper(WallpaperKind::Weather(Weather::Rain)),
                0.0,
                0.0,
            ),
            layer(
                4,
                300.0,
                80.0,
                shape(0.0, Fill::Solid(ink), 0.0),
                20.0,
                20.0,
            ),
            layer(
                5,
                300.0,
                80.0,
                shape(
                    0.0,
                    Fill::Linear {
                        from: ink,
                        to: Rgba(70, 60, 100, 255),
                        angle_deg: 35.0,
                    },
                    0.0,
                ),
                20.0,
                120.0,
            ),
            layer(
                6,
                300.0,
                80.0,
                shape(16.0, Fill::Solid(Rgba(50, 46, 70, 200)), 1.0),
                20.0,
                220.0,
            ),
            layer(
                7,
                420.0,
                200.0,
                glass(
                    Backdrop::Live {
                        radius: 30.0,
                        downsample: 8,
                    },
                    depth,
                ),
                400.0,
                40.0,
            ),
            layer(
                8,
                300.0,
                120.0,
                glass(Backdrop::Cached { radius: 30.0 }, None),
                400.0,
                280.0,
            ),
            layer(9, 200.0, 60.0, glass(Backdrop::None, None), 740.0, 380.0),
        ];
        // The wallpaper that has a picture goes with the other wallpapers, the images among the shapes.
        let mut walls = 4;
        if silk {
            layers.insert(0, layer(20, w, h, wallpaper(WallpaperKind::Silk), 0.0, 0.0));
            walls += 1;
        }
        for (n, ok, (width, height, x, y)) in [
            (21, pictures[0], (400.0, 28.0, 20.0, 330.0)),
            (22, pictures[1], (848.0, 64.0, 20.0, 380.0)),
            (23, pictures[2], (120.0, 120.0, 840.0, 20.0)),
        ] {
            if ok {
                layers.insert(walls, layer(n, width, height, Content::Image, x, y));
            }
        }
        let Ok(target) = self
            .gl
            .new_render_target(self.panel.width, self.panel.height)
        else {
            return None;
        };
        let Ok(upper) = self
            .gl
            .new_render_target(self.panel.width, self.panel.height)
        else {
            self.gl.delete_render_target(target);
            return None;
        };
        // The way the overlay route draws a frame: the wallpapers over black, and over them a plane of everything else
        // that shows what is under it where nothing is drawn.
        let plane = |target: &RenderTarget, from, to, transparent| Draw {
            surface: Surface {
                stack: 0,
                from,
                to,
                transparent,
            },
            slot: None,
            target: target.framebuffer,
            areas: None,
            clear: true,
        };
        let planes = [
            plane(&target, 0, walls, false),
            plane(&upper, walls, layers.len(), true),
        ];
        let reports = self.draw_frame(&layers, &planes, &NoGroups);
        let mut error = reports.iter().find_map(|r| r.gl_error);
        // And once more with everything that is not a wallpaper under a clip whose fade reaches the whole screen, so that the
        // twins of the programs that have the fade (`shaders::fade`) have been drawn once as well.
        let screen = compositor_protocol::Rect {
            x: 0.0,
            y: 0.0,
            w: self.panel.logical_width,
            h: self.panel.logical_height,
        };
        let faded: Vec<Drawn> = layers
            .iter()
            .enumerate()
            .map(|(n, layer)| {
                let props = if n < walls {
                    layer.props
                } else {
                    layer
                        .props
                        .clip_feathered(screen, compositor_protocol::Edges::all(600.0))
                };
                Drawn {
                    props,
                    rest: props,
                    ..*layer
                }
            })
            .collect();
        let reports = self.draw_frame(&faded, &planes, &NoGroups);
        error = error.or_else(|| reports.iter().find_map(|r| r.gl_error));
        self.gl.finish();
        self.gl.delete_render_target(target);
        self.gl.delete_render_target(upper);
        for l in &layers {
            self.apply(&Op::Delete { id: l.id });
        }
        for n in [20, 21, 22, 23] {
            self.apply(&Op::Delete {
                id: LayerId(u32::MAX - n),
            });
        }
        for path in files {
            let _ = std::fs::remove_file(path);
        }
        error
    }

    /// Turns on LumaGlass's optional vignette, specular ridges and bloom for silk wallpapers; all are off until asked.
    pub fn set_silk_look(&mut self, look: Look) {
        self.wallpapers.set_look(look);
    }

    pub fn gles(&self) -> &Gles {
        &self.gl
    }

    /// The GL context, for a test that puts a counter in front of the driver's calls.
    #[cfg(test)]
    pub fn gles_mut(&mut self) -> &mut Gles {
        &mut self.gl
    }

    /// Applies what a change means for the GPU: image layers are noted (their textures are made when the first upload needs them,
    /// and lost with them), and damage is noted for the next upload; prepared sets and atlases are made, held and let go, and the
    /// quads of a glyph run replaced. The ops that are the scene's alone (where a layer is, which group it hangs under, what a
    /// motion track does) are none of the renderer's: it is handed layers already placed.
    pub fn apply(&mut self, op: &Op) {
        match op {
            Op::Create {
                id,
                width,
                height,
                content,
                image,
                mesh,
                ..
            } => {
                self.remove(*id);
                self.glass.forget(*id);
                if let Content::Glyphs(style) = content {
                    self.atlases.run_made(*id, style.atlas);
                }
                if let Some(data) = mesh {
                    self.meshes.create(&self.gl, *id, *width, *height, data);
                }
                if let Some(file) = image {
                    // A picture that is new is all of it changed. Its texture is made when something wants it.
                    self.note_damage(*id, PixRect::full(*width, *height));
                    self.textures.insert(
                        *id,
                        Texture {
                            name: 0,
                            file: Arc::clone(file),
                            width: *width,
                            height: *height,
                            dirty: Some(PixRect::full(*width, *height)),
                            used: 0,
                            defined: false,
                            uploaded: None,
                            opaque: (*width as f32 >= self.panel.logical_width
                                && *height as f32 >= self.panel.logical_height)
                                .then_some(false),
                        },
                    );
                }
            }
            Op::Damage { id, rect } => {
                if let Some(texture) = self.textures.get_mut(id) {
                    texture.dirty = Some(texture.dirty.map_or(*rect, |d| d.union(*rect)));
                    self.note_damage(*id, *rect);
                }
            }
            Op::Restyle { id, content } => {
                if let Content::Glyphs(style) = content {
                    self.atlases.run_restyled(*id, style.atlas);
                }
            }
            Op::Delete { id } => {
                self.remove(*id);
                self.glass.forget(*id);
            }
            Op::Clear => {
                let ids: Vec<LayerId> = self.textures.keys().copied().collect();
                for id in ids {
                    self.remove(id);
                }
                self.glass.forget_all();
                self.meshes.clear(&self.gl);
                self.atlases.clear(&mut self.graveyard);
                self.sets.clear(&mut self.backdrops);
                self.set_ideas(ideas::DEFAULT);
            }
            Op::Ideas(mask) => self.set_ideas(*mask),
            Op::SmoothFirst(level) => self.set_ideas(level.ideas()),
            Op::BackdropCreate { id, spec } => {
                let panel = (self.panel.width, self.panel.height);
                let geometry = Geometry::of(&self.backdrops, panel, spec.radius, spec.downsample);
                self.sets
                    .create(*id, spec.clone(), geometry, &mut self.backdrops);
            }
            Op::BackdropRefresh { id, content } => self.sets.refresh(*id, *content),
            Op::BackdropFreeze { id } => {
                self.sets.freeze(*id, &mut self.events, &mut self.backdrops);
            }
            Op::BackdropRelease { id } => self.sets.release(*id),
            Op::BackdropDelete { id } => self.sets.delete(*id, &mut self.backdrops),
            Op::AtlasCreate { id, width, height } => {
                self.atlases
                    .create(*id, *width, *height, &mut self.graveyard);
            }
            Op::AtlasUpload {
                id,
                slot,
                generation,
                rect,
                path,
            } => self.atlases.queue(
                *id,
                *slot,
                *generation,
                *rect,
                path.clone(),
                &mut self.events,
            ),
            Op::AtlasDelete { id } => self.atlases.delete(*id, &mut self.graveyard),
            Op::SetGlyphs { id, after, glyphs } => {
                self.atlases.set_glyphs(*id, *after, glyphs.clone());
            }
            Op::LiveAttach {
                source,
                spec,
                buffers,
            } => {
                // The op keeps its descriptors (the transaction is only borrowed here); the source gets duplicates, and nothing more
                // is done in the frame: the buffers are put on the GPU in `tidy`.
                match buffers
                    .iter()
                    .map(OwnedFd::try_clone)
                    .collect::<io::Result<Vec<_>>>()
                {
                    Ok(fds) => self.live.attach(*source, *spec, fds),
                    Err(e) => self.live.refuse(
                        *source,
                        format!("the buffers' descriptors cannot be duplicated: {e}"),
                    ),
                }
            }
            Op::LiveFrame { source, buffer, .. } => self.live.frame(*source, *buffer),
            Op::LiveClear { source } => self.live.clear(*source),
            Op::LiveDetach { source } => self.live.detach(*source),
            Op::Ripple { .. }
            | Op::Set { .. }
            | Op::Animate { .. }
            | Op::HoldGpuClock(_)
            | Op::Display(_)
            | Op::GroupCreate { .. }
            | Op::GroupSet { .. }
            | Op::GroupParent { .. }
            | Op::GroupBind { .. }
            | Op::GroupUnbind { .. }
            | Op::GroupDelete { .. }
            | Op::Attach { .. }
            | Op::MotionCreate { .. }
            | Op::MotionDelete { .. }
            | Op::MotionDrive { .. }
            | Op::MotionSettle { .. }
            | Op::MotionCancel { .. } => {}
        }
    }

    /// The events the renderer's own work has produced since the last call, for the engine to send to the client.
    pub fn drain_events(&mut self) -> Vec<compositor_protocol::Event> {
        std::mem::take(&mut self.events)
    }

    /// How the device puts producers' buffers on the GPU (`None`: it cannot, and their sources are refused).
    pub fn set_importer(&mut self, importer: Option<Box<dyn live::Importer>>) {
        self.live = live::Sources::new(importer);
    }

    /// What the sources of frames have to tell their producers since the last call: buffers released, sources made and ended.
    pub fn drain_live_events(&mut self) -> Vec<(SourceId, frames::Event)> {
        self.live.take_events()
    }

    /// What the sources have made of their buffers so far, for the tests.
    #[cfg(test)]
    pub fn live_counts(&self) -> live::Counts {
        self.live.counts()
    }

    /// What the last draw of a producer's buffer off screen left in its target (its first pixel), for the tests.
    #[cfg(test)]
    pub fn touched_pixel(&self) -> Option<[u8; 4]> {
        self.touch.pixel(&self.gl)
    }

    /// Plate captures made and bytes uploaded since the last call, for the next `Event::Frame`.
    pub fn take_frame_counts(&mut self) -> (u16, u32) {
        (
            std::mem::take(&mut self.captures_since),
            std::mem::take(&mut self.uploaded_since),
        )
    }

    /// Switches the glass ideas: what the next frame draws is not what the ones before drew, so every render target owes it
    /// all.
    fn set_ideas(&mut self, mask: u32) {
        if mask != self.ideas {
            // What the plates hold was drawn as the ideas were.
            self.still = Still::default();
            if (mask ^ self.ideas) & ideas::STILL_GLASS != 0 {
                self.set_plates(mask & ideas::STILL_GLASS != 0);
            }
            self.ideas = mask;
            self.motion.clear();
            self.forget_targets();
        }
    }

    /// Puts the steps `1..=level` of the Full ladder in force (`ladder::rewrite`): from the next frame the glass is drawn as they
    /// leave it, and a level that is not the one before asks for a frame at once, as work between frames does (`wants_frame`), so
    /// that a ladder switched off while the screen stands is not left plain. Which layers a step changes is left to the layers'
    /// own account of the frame before (`Frozen`, the plates' keys, the cached backdrops' fingerprints): none of them keeps what it
    /// drew at another level.
    pub fn set_ladder(&mut self, level: u8) {
        if level != self.ladder {
            self.ladder = level;
            self.stale = true;
        }
    }

    #[cfg(test)]
    /// From now on the GPU has room for `room` more render targets for the pictures of backdrops (`None`: for as many as are asked).
    pub(crate) fn set_picture_room(&mut self, room: Option<usize>) {
        self.backdrops.set_room(room);
    }

    #[cfg(test)]
    /// How many render targets for pictures the room set by [`Renderer::set_picture_room`] has refused.
    pub(crate) fn pictures_refused(&self) -> usize {
        self.backdrops.refused()
    }

    #[cfg(test)]
    /// Whether a picture is kept for the cached backdrop of the surface `leader`.
    pub(crate) fn keeps_picture(&self, leader: LayerId) -> bool {
        self.backdrops.keeps(leader)
    }
    /// What the renderer holds on the GPU, by kind (see [`crate::census`]).
    pub fn census(&self) -> crate::census::Census {
        let mut census = crate::census::Census::default();
        for texture in self.textures.values().filter(|t| t.name != 0) {
            census.layers.texture(texture.width, texture.height, 4);
        }
        for plate in self.plates.iter().flatten() {
            census.still.texture(plate.width, plate.height, 4);
        }
        census.graveyard = self.graveyard.len() as u32;
        self.atlases.census(&mut census);
        self.backdrops.census(&mut census);
        self.sets.census(&mut census);
        census
    }

    /// Makes room for the plates of `ideas::STILL_GLASS` (`on`: none are made until a surface is shown from one, see
    /// [`Renderer::make_plates`]) or lets go of them.
    fn set_plates(&mut self, on: bool) {
        for plate in std::mem::take(&mut self.plates).into_iter().flatten() {
            self.gl.delete_render_target(plate);
        }
        if on {
            self.plates = vec![None; still::PLATES];
        }
        self.plates_used = [0; still::PLATES];
    }

    /// Makes the plates the frame's plan shows glass from, and lets go of the ones no surface has been shown from for a long time
    /// (what stood in them is made again when it stands again). A plate that cannot be made leaves its surfaces drawn as they
    /// always were.
    fn make_plates(
        &mut self,
        layers: &[Drawn],
        plan: &[backdrop::Backdrop],
        planned: &mut still::Plan,
    ) {
        for reuse in planned.reuse.iter().flatten() {
            self.plates_used[reuse.plate] = self.frame;
        }
        for p in 0..self.plates.len() {
            if self.plates[p].is_some()
                && self.frame.saturating_sub(self.plates_used[p]) > PLATE_KEEP_FRAMES
            {
                if let Some(plate) = self.plates[p].take() {
                    self.gl.delete_render_target(plate);
                }
                self.still.drop_plate(p);
            }
        }
        // A group's surfaces can stand in more than one plate (two cards a few pixels apart share a picture, and each has its own
        // rectangle): every plate they are to be drawn into is made before any is drawn into.
        for b in plan {
            for &user in &b.users {
                let Some(p) = planned.reuse[user]
                    .map(|reuse| reuse.plate)
                    .filter(|&p| self.plates[p].is_none())
                else {
                    continue;
                };
                match self
                    .gl
                    .new_render_target(self.panel.width, self.panel.height)
                {
                    Ok(plate) => {
                        // A plate is clear when it is made. In tests it holds noise, as the memory it was made of could.
                        #[cfg(test)]
                        self.gl.poison_target(&plate);
                        self.plates[p] = Some(plate);
                    }
                    Err(e) => {
                        log(format!("no plate for still glass: {e}"));
                        for &member in &b.users {
                            planned.reuse[member] = None;
                        }
                        self.still.forget(layers[b.users[0]].id);
                        break;
                    }
                }
            }
        }
    }

    /// The time of the frame about to be drawn (`ideas::ADAPTIVE` fades by it).
    pub fn set_time(&mut self, now_ns: u64) {
        self.now_ns = now_ns;
    }

    /// The layers as this frame draws them, in `out`: each glyph run with the revision of its quads mixed into its version
    /// (the scene does not know when a list is swapped in, and a version is what a cached backdrop's fingerprint and a damaged
    /// frame go by); under the Full ladder (`self.ladder`), the glass as the steps in force leave it, with the level mixed into the
    /// version of each surface a step changed (the surfaces below a cached backdrop are part of its picture, so that a step that
    /// changes how one looks is a change below it); and, under `ideas::ADAPTIVE` (`adapting`), the glass as it is for its motion,
    /// with how calm each layer is in `self.calm`. A glass surface that is moving (not where it was the frame before, or on its
    /// way to somewhere: an animation runs on it) has no refraction and holds its blur, and both come back over `calm_ns` once
    /// it stands still.
    fn revise(&mut self, layers: &[Drawn], adapting: bool, out: &mut Vec<Drawn>) {
        let now = self.now_ns;
        let mut last = if adapting {
            std::mem::take(&mut self.motion)
        } else {
            HashMap::new()
        };
        self.calm.clear();
        out.clear();
        for layer in layers {
            let mut layer = *layer;
            if let (level @ 1.., Content::Glass(glass)) = (self.ladder, layer.content) {
                let plainer = ladder::rewrite(glass, level);
                if plainer != glass {
                    layer.content = Content::Glass(plainer);
                    layer.version = atlas::mixed(layer.version, u64::from(level));
                }
            }
            match layer.content {
                Content::Glyphs(_) => {
                    layer.version = atlas::mixed(layer.version, self.atlases.revision(layer.id));
                }
                Content::Glass(glass) if adapting => {
                    let (was, moved) = last.remove(&layer.id).unwrap_or((layer.props, None));
                    let moving = layer.props != was || layer.props != layer.rest;
                    let moved = if moving { Some(now) } else { moved };
                    let calm = calm_at(now, moved, self.calm_ns);
                    self.motion
                        .insert(layer.id, (layer.props, moved.filter(|_| calm < 1.0)));
                    self.calm.push(calm);
                    layer.content = Content::Glass(compositor_protocol::Glass {
                        refraction: glass.refraction * calm,
                        ..glass
                    });
                    out.push(layer);
                    continue;
                }
                Content::Glass(_)
                | Content::Image
                | Content::Shape(_)
                | Content::Wallpaper(_)
                | Content::Mesh(_)
                | Content::Live(_) => {}
            }
            if adapting {
                self.calm.push(1.0);
            }
            out.push(layer);
        }
    }

    /// Notes where the app changed a picture, for the next damaged frame to say what it changes: only while frames are
    /// drawn that way (each of them clears the notes).
    fn note_damage(&mut self, id: LayerId, rect: PixRect) {
        for account in self.accounts.iter_mut().filter(|a| a.ring.len() > 0) {
            account.damaged.push((id, rect));
        }
    }

    fn remove(&mut self, id: LayerId) {
        if let Some(texture) = self.textures.remove(&id) {
            // Freeing a texture costs the driver a sizeable fraction of a millisecond (0.15 ms at the Echo Show's
            // lowest CPU clock), and a page of a settings screen going away is fifty of them: not something to do
            // while the frame that follows waits. They are freed when nothing waits, in `tidy`. A layer that went before
            // anything wanted its texture has none to free.
            if texture.name != 0 {
                self.graveyard.push(texture.name);
            }
        }
        self.meshes.remove(&self.gl, id);
        self.atlases.run_deleted(id);
    }

    /// Does the housekeeping that is not needed to draw the frame that was just handed over, for at most `budget` (at
    /// least one thing is done, and everything if the graveyard has outgrown [`GRAVEYARD_LIMIT`]): frees the textures
    /// of layers that are gone, then makes the textures of layers no frame has drawn yet and uploads what the app changed in
    /// textures the last frame did not draw, so that the frame that first does (a page sliding in, a card fading up) finds them
    /// ready instead of taking milliseconds to make and upload them on its own critical path, then copies one chunk of a
    /// glyph atlas to its texture (a call lands at most one, of at most [`compositor_protocol::resources::MAX_UPLOAD_BYTES`]), and
    /// last works through the buffers of producers' sources (`live::Sources::tidy`: lets go of those of sources that are over, puts
    /// those of new ones on the GPU and draws each once off screen, so that no frame pays for either).
    /// Call it while the frame is on the GPU, or when nothing is due.
    ///
    /// A texture the last frame drew may be in use by the GPU as this runs, and is left alone.
    pub fn tidy(&mut self, budget: Duration) -> Tidied {
        let began = Instant::now();
        let mut done = Tidied::default();
        // With the whole graveyard due, nothing is left waiting; otherwise at least one thing is done, and no more
        // once the budget is spent.
        let urgent = self.graveyard.len() > GRAVEYARD_LIMIT;
        let mut worked = false;
        while let Some(name) = self.graveyard.last().copied() {
            if worked && !urgent && began.elapsed() >= budget {
                done.more = true;
                return done;
            }
            self.graveyard.pop();
            self.gl.delete_texture(name);
            done.freed += 1;
            worked = true;
        }
        for texture in self.textures.values_mut() {
            if texture.dirty.is_none() || texture.used == self.frame {
                continue;
            }
            if worked && began.elapsed() >= budget {
                done.more = true;
                return done;
            }
            // A file the app cut short leaves the damage pending and uploads nothing: not something to spin on.
            let pixels = upload(&self.gl, texture);
            done.uploaded_pixels += pixels;
            self.uploaded_since = self.uploaded_since.saturating_add(picture_bytes(pixels));
            worked |= pixels > 0;
        }
        if self.atlases.pending() {
            if worked && began.elapsed() >= budget {
                done.more = true;
                return done;
            }
            let landed = self.atlases.land_next(&self.gl, &mut self.events);
            done.uploaded_bytes += landed.bytes;
            self.uploaded_since = self
                .uploaded_since
                .saturating_add(u32::try_from(landed.bytes).unwrap_or(u32::MAX));
            // A list that was waiting for the chunk is shown, or a picture it is part of changed: the screen is out of date.
            self.stale |= landed.changed > 0;
            worked |= landed.bytes > 0;
            done.more = self.atlases.pending();
        }
        if self.live.busy() {
            if worked && began.elapsed() >= budget {
                done.more = true;
                return done;
            }
            let Self {
                live,
                gl,
                touch,
                programs,
                panel,
                ..
            } = self;
            let image = &programs.plain.image;
            let left = budget.saturating_sub(began.elapsed());
            let made = live.tidy(gl, left, &mut |gl, spec, texture| {
                touch.draw(gl, image, panel, spec, texture);
            });
            done.live += made.buffers;
            done.more |= made.more;
        }
        done
    }

    /// Places every layer that is shown (as `view` has it) and on the panel: brings its texture up to date (`pictures`) and adds
    /// its vertices to `vertices`. The jobs are in the layers' order.
    fn place(
        placer: &mut Placer,
        layers: &[Drawn],
        view: View,
        pictures: Pictures,
        vertices: &mut Vec<Vertex>,
        jobs: &mut Vec<Job>,
        report: &mut DrawReport,
    ) {
        let Placer {
            gl,
            panel,
            glass,
            textures,
            atlases,
            sets,
            frame,
            skips,
        } = placer;
        jobs.clear();
        for (index, layer) in layers.iter().enumerate() {
            let props = props_of(layer, view);
            if !is_shown(props) {
                continue;
            }
            let scissor = match props.clip.map(|clip| panel.scissor(clip)) {
                Some(None) => continue,
                Some(rect) => rect,
                None => None,
            };
            let at = placement(layer, props);
            let extent =
                panel.quad_of(&at, extent_of(&layer.content, props, (at.width, at.height)));
            if !panel.is_on_panel(&extent, scissor) {
                continue;
            }
            // Everything of it that is drawn on the panel; a layer with none is not drawn at all.
            let Some(mut extent_area) = bounds(&extent, scissor)
                .and_then(|area| damage::intersect(&area, &panel_area_of(panel)))
            else {
                continue;
            };
            // A wallpaper is a background for its layer: a star of its turning sky, a drop of its rain that lies outside
            // the layer's rectangle is left out, so that the rectangle is all of the panel the layer draws in (which
            // what covers what, and what changes between frames, are judged by).
            let scissor = if matches!(layer.content, Content::Wallpaper(_))
                && extent_area != panel_area_of(panel)
            {
                pix_rect_of(extent_area)
            } else {
                scissor
            };
            let draw = match layer.content {
                Content::Image => {
                    let Some(texture) = textures.get_mut(&layer.id) else {
                        continue;
                    };
                    if pictures == Pictures::Now {
                        texture.used = *frame;
                        upload_timed(gl, texture, report);
                    }
                    Vertices::Quad(push_quad(vertices, &extent))
                }
                Content::Shape(_) => Vertices::Quad(push_quad(vertices, &extent)),
                Content::Glass(glass_content) => Vertices::Glass(glass.place(
                    layer.id,
                    &glass_content,
                    &at,
                    panel,
                    pad(props.scale),
                    vertices,
                )),
                Content::Wallpaper(wallpaper) => {
                    // A picture of the layers at rest keeps no overlay: a raindrop in it would stay for good.
                    if view == View::Rest && !wallpapers::shows_at_rest(&wallpaper.kind) {
                        continue;
                    }
                    if wallpaper.kind.uses_image() {
                        let Some(texture) = textures.get_mut(&layer.id) else {
                            continue;
                        };
                        if pictures == Pictures::Now {
                            texture.used = *frame;
                            upload_timed(gl, texture, report);
                        }
                    }
                    Vertices::Wallpaper
                }
                // Its picture is made when the frame is known to look at it (`draw_frame`).
                Content::Mesh(_) => Vertices::Quad(push_quad(vertices, &extent)),
                // Its picture is the producer's newest, if it has one (`draw_pass` skips the layer when there is none).
                Content::Live(_) => Vertices::Quad(push_quad(vertices, &extent)),
                Content::Glyphs(_) => {
                    // Its quads are drawn within the layer's clip, and where that is on the panel.
                    let within = scissor.unwrap_or(PixRect::full(panel.width, panel.height));
                    let Some(span) = atlases.place(layer.id, &at, panel, within, vertices) else {
                        continue;
                    };
                    // All of the panel it draws on is where its quads are, not its rectangle (a line's art is a run of
                    // thousands of pixels with ink here and there): what covers it, and what it changes when it moves, are
                    // judged by that.
                    let Some(inked) = damage::intersect(&extent_area, &span.ink) else {
                        continue;
                    };
                    extent_area = inked;
                    Vertices::Glyphs(span)
                }
            };
            // The fade of a feathered clip, when the layer reaches into it: a layer that lies where it is 1 is drawn by the
            // plain programs, as it would be with a hard clip, and a wallpaper (a background that fills its rectangle) has a
            // hard edge whatever its clip says.
            let feather = props
                .clip
                .filter(|_| !skips.feather && !matches!(layer.content, Content::Wallpaper(_)))
                .and_then(|clip| panel.ramp(clip, props.feather))
                .filter(|ramp| !damage::contains(&ramp.solid(), &extent_area));
            jobs.push(Job {
                layer: index,
                scissor,
                draw,
                extent: extent_area,
                cover: match view {
                    View::Now => cover_of(layer, props, panel, scissor, feather.as_ref(), sets),
                    View::Rest => Cover::default(),
                },
                feather,
            });
        }
    }

    /// What a layer of the backdrop planner's kind is: where it is drawn now and at rest, and what it asks of a
    /// backdrop.
    fn plan_layer(&self, layer: &Drawn) -> backdrop::Layer {
        let area_in = |view: View| -> Option<Area> {
            let props = props_of(layer, view);
            // What a picture of the layers at rest keeps of an overlay is nothing: it does not count there.
            if view == View::Rest
                && matches!(layer.content, Content::Wallpaper(w) if !wallpapers::shows_at_rest(&w.kind))
            {
                return None;
            }
            if !is_shown(props) {
                return None;
            }
            let clip = match props.clip.map(|clip| self.panel.scissor(clip)) {
                Some(None) => return None,
                Some(rect) => rect,
                None => None,
            };
            let at = placement(layer, props);
            let extent = extent_of(&layer.content, props, (at.width, at.height));
            bounds(&self.panel.quad_of(&at, extent), clip)
        };
        let body_in = |view: View| -> Area {
            let at = placement(layer, props_of(layer, view));
            let body = [0.0, 0.0, at.width, at.height];
            bounds(&self.panel.quad_of(&at, body), None).unwrap_or([0; 4])
        };
        // A blur is soft, so its picture can be small: with `ideas::HALF_RES` half of the size it was, in each direction (which
        // is one halving less for the blur's chain to make).
        let smaller = |downsample: u8| {
            if self.ideas & ideas::HALF_RES != 0 {
                downsample.saturating_mul(2).min(MAX_HALF_RES_DOWNSAMPLE)
            } else {
                downsample
            }
        };
        let wanted = match layer.content {
            Content::Glass(glass) => match glass.backdrop {
                Backdrop::None => None,
                Backdrop::Cached { radius } => {
                    Some((Refresh::Cached, radius, smaller(cached_downsample(radius))))
                }
                Backdrop::Live { radius, downsample } => {
                    Some((Refresh::Live, radius, smaller(downsample)))
                }
                // A prepared set makes its picture of its own accord, when the app asks, and the frame only composes it from
                // the set's plates: there is nothing for the planner to make, to share or to keep up to date, and the layers
                // below the surface are not looked at for it.
                Backdrop::Prepared { .. } => None,
            },
            Content::Image
            | Content::Shape(_)
            | Content::Wallpaper(_)
            | Content::Mesh(_)
            | Content::Glyphs(_)
            | Content::Live(_) => None,
        };
        backdrop::Layer {
            id: layer.id,
            version: layer.version,
            now: area_in(View::Now),
            rest: area_in(View::Rest),
            ask: wanted.map(|(refresh, radius, downsample)| Ask {
                refresh,
                radius,
                downsample,
                body_now: body_in(View::Now),
                body_rest: body_in(View::Rest),
            }),
        }
    }

    /// How the frame shows its glass (`ideas::STILL_GLASS`): which surfaces come from plates, and which pictures need not be made.
    /// `drawn` says which of the pictures' surfaces the frame draws; `fingerprints` and `stale` are those of the cached ones.
    fn plan_plates(
        &mut self,
        layers: &[Drawn],
        plan: &[backdrop::Backdrop],
        drawn: &[bool],
        fingerprints: &[u64],
        stale: &[bool],
    ) -> still::Plan {
        // A render target that draws none of the glass has nothing to decide (the plane of the wallpapers, on a route with two).
        if self.ideas & ideas::STILL_GLASS == 0
            || self.plates.len() != still::PLATES
            || !drawn.contains(&true)
        {
            return still::Plan::direct(layers.len(), plan.len());
        }
        let mut rects = vec![None; layers.len()];
        let groups = plan
            .iter()
            .enumerate()
            .map(|(index, b)| {
                for &user in &b.users {
                    rects[user] = plate_rect(&self.panel, &layers[user]);
                }
                let cached = b.refresh == Refresh::Cached;
                still::Group {
                    leader: layers[b.users[0]].id,
                    users: b.users.clone(),
                    refresh: b.refresh,
                    radius: b.radius,
                    downsample: b.downsample,
                    fingerprint: if cached { fingerprints[index] } else { 0 },
                    stale: stale[index],
                    below: if cached {
                        Vec::new()
                    } else {
                        under_of(&self.jobs, layers, b)
                    },
                    drawn: drawn[index],
                }
            })
            .collect();
        self.still.classify(self.frame, layers, groups, &rects)
    }

    /// How many glass surfaces the frame draws and how many panel pixels their bodies, and the shadows and raised
    /// controls' light and shade around them, cover: one count for each surface, so that overlaps add up.
    fn glass_coverage(&self, layers: &[Drawn]) -> (u16, u64, u64, u64, [u32; 4]) {
        let (mut surfaces, mut bodies, mut around, mut rims) = (0u16, 0u64, 0u64, 0u64);
        let mut vertices = [0u32; 4];
        let pixels = |area: Option<Area>| {
            area.map_or(0u64, |a| {
                (a[2] - a[0]).max(0) as u64 * (a[3] - a[1]).max(0) as u64
            })
        };
        for job in &self.jobs {
            let layer = &layers[job.layer];
            let (Content::Glass(glass), Vertices::Glass(placed)) = (&layer.content, &job.draw)
            else {
                continue;
            };
            let props = &layer.props;
            let at = placement(layer, props);
            let body = bounds(
                &self.panel.quad_of(&at, [0.0, 0.0, at.width, at.height]),
                job.scissor,
            );
            let extent = bounds(
                &self
                    .panel
                    .quad_of(&at, extent_of(&layer.content, props, (at.width, at.height))),
                job.scissor,
            );
            let panel = PixRect::full(self.panel.width, self.panel.height);
            let on_panel = |area: Option<Area>| {
                area.and_then(pix_rect_of)
                    .and_then(|r| r.intersect(panel))
                    .map(area_of)
            };
            surfaces += 1;
            let (body, extent) = (pixels(on_panel(body)), pixels(on_panel(extent)));
            bodies += body;
            around += extent.saturating_sub(body);
            let size = (at.width, at.height);
            let uniforms = glass::SurfaceUniforms::new(glass, size, pad(props.scale));
            let pieces = glass::slices(size, uniforms.radius, uniforms.inner, 0.0);
            let rim: f32 = pieces
                .rim
                .iter()
                .map(|r| (r[2] - r[0]) * (r[3] - r[1]))
                .sum();
            rims += (rim * props.scale * props.scale) as u64;
            for (total, count) in vertices.iter_mut().zip(placed.vertex_counts()) {
                *total += count;
            }
        }
        (surfaces, bodies, around, rims, vertices)
    }

    /// Draws `layers` (back to front) into the framebuffer object `target`: everything of them that can be seen. (The
    /// engine draws every frame with [`Renderer::draw_frame`]; this and the next are the tests' one-target frames.)
    #[cfg(test)]
    pub fn draw(&mut self, target: u32, layers: &[Drawn], source: &dyn Source) -> DrawReport {
        let draw = Draw {
            surface: Surface::whole(layers.len()),
            slot: None,
            target,
            areas: None,
            clear: true,
        };
        self.draw_frame(layers, &[draw], source).remove(0)
    }

    /// Draws `layers` into render target `slot`, whose framebuffer object is `target`, but only the part that differs
    /// from what the target holds: what the frame changes from the one before, and what the frames drawn since the target
    /// was last drawn into changed (`damage::Ring`). Every frame of a run of them has to be drawn this way, into the
    /// targets [`Renderer::set_targets`] was told of; a frame that changes nothing draws nothing.
    #[cfg(test)]
    pub fn draw_damaged(
        &mut self,
        slot: usize,
        target: u32,
        layers: &[Drawn],
        source: &dyn Source,
    ) -> DrawReport {
        let draw = Draw {
            surface: Surface::whole(layers.len()),
            slot: Some(slot),
            target,
            areas: None,
            clear: true,
        };
        self.draw_frame(layers, &[draw], source).remove(0)
    }

    /// The number of render targets frames of plane `stack` are drawn into in turn. Nothing is known of what they hold
    /// until each has been drawn into whole.
    pub fn set_targets(&mut self, stack: usize, count: usize) {
        let everything = panel_area_of(&self.panel);
        while self.accounts.len() <= stack {
            self.accounts.push(Account::new(0, everything));
        }
        self.accounts[stack] = Account::new(count, everything);
    }

    /// Nothing is known any more of what the render targets hold: a frame that may not have been drawn whole. The next
    /// frame into each draws everything.
    pub fn forget_targets(&mut self) {
        for account in &mut self.accounts {
            account.ring.forget();
            account.prev = None;
        }
    }

    /// Whether the renderer knows what every plane's last frame showed: a frame has been drawn into each plane since the targets were
    /// made or forgotten ([`Renderer::forget_targets`]). Until then a frame that shows nothing new is still owed, because what the
    /// screen shows is not known to be what the layers say.
    pub fn knows_the_screen(&self) -> bool {
        !self.accounts.is_empty() && self.accounts.iter().all(|account| account.prev.is_some())
    }

    /// The pixels the render target `slot` of plane `stack` owes: what the frames drawn into the plane's other targets since it was
    /// last drawn into changed. The next frame into it draws all of that before anything of its own, however little its own is: a
    /// finger that lands on a screen that has come to rest is drawn into the target the frame before did not go into, which holds
    /// the screen as it was two frames ago (the engine brings it up to date while nothing is asked, [`Renderer::draw_frame`] with
    /// the layers the last frame drew).
    pub fn owed_pixels(&self, stack: usize, slot: usize) -> u64 {
        self.accounts
            .get(stack)
            .map_or(0, |account| account.ring.owed_pixels(slot))
    }

    /// Whether a layer an opaque layer above it covers completely is left undrawn. On unless the environment says
    /// otherwise; a test or a measurement turns it off to see what it does.
    pub fn set_culling(&mut self, on: bool) {
        self.culling = on;
    }

    /// Whether the part of the panel a frame draws again is drawn as the rectangles the account keeps apart (on) or as
    /// their one cover: a measurement turns it off to see what the rectangles save.
    pub fn set_rects(&mut self, on: bool) {
        self.rects = if on { damage::MAX_RECTS } else { 1 };
    }

    /// The layers of the frame drawn last that nothing of showed: an opaque layer covered them completely and no picture
    /// behind glass had them in it (or they were nowhere on the panel). Whatever they do by themselves, a wallpaper's
    /// motion or a mesh's orbit, nobody sees; in drawing order.
    pub fn unseen(&self) -> &[LayerId] {
        &self.unseen
    }

    /// Whether the picture of image layer `id`, as it is on the GPU now, is opaque in every pixel: known only of a picture at
    /// least as large as the screen (`Texture::opaque`), and not while a change of it waits to be copied (the frame being planned
    /// copies it first, and what it brings is not known yet).
    pub fn opaque_picture(&self, id: LayerId) -> bool {
        self.textures
            .get(&id)
            .is_some_and(|t| t.dirty.is_none() && t.opaque == Some(true))
    }

    /// Whether the screen is out of date: work done since the last frame (a glyph list swapped in once the chunks it waited
    /// for landed, a prepared generation published under glass that shows its set) changed what the next frame draws, and no
    /// commit or running animation is going to ask for a frame. The engine draws one at once; `draw_frame` clears it.
    pub fn wants_frame(&self) -> bool {
        self.stale
    }

    /// Says which layers the frame just drawn had in it (the engine does, after every frame). Work between frames on a glyph run of
    /// any other layer (a list swapped in once its chunk landed under a page made ahead, which is not shown) changes nothing on the
    /// panel, so it does not make the screen out of date ([`Renderer::wants_frame`]); the run has its list for the frame that
    /// shows the layer. Until the engine has said, every layer counts.
    pub fn set_shown(&mut self, layers: &[Drawn]) {
        self.atlases.set_shown(layers.iter().map(|layer| layer.id));
    }

    /// The panel pixels an area of `layer` covers, a pixel wider on every side for the filter's reach.
    fn image_area(&self, layer: &Drawn, rect: PixRect) -> Option<Area> {
        let at = placement(layer, &layer.props);
        let quad = self.panel.quad_of(
            &at,
            [
                rect.x as f32,
                rect.y as f32,
                (rect.x + rect.w) as f32,
                (rect.y + rect.h) as f32,
            ],
        );
        let area = bounds(&quad, None)?;
        let grown = [area[0] - 1, area[1] - 1, area[2] + 1, area[3] + 1];
        damage::intersect(&grown, &panel_area_of(&self.panel))
    }

    /// What of the panel differs between the frame before and this one, in tiles: the part of each layer that changed (where
    /// it was seen and where it is now), the parts of the pictures the app damaged, the places layers that are gone were, and
    /// what a glass surface shows of what changed below it.
    ///
    /// `seen` is what can be seen of each layer in the frame (`tiles::visible`), and `extents` all that it draws, seen or not:
    /// a layer that an opaque one covers is not in the frame, but it is in the blur behind the glass above it, so what changes
    /// in it changes that glass. `stale` are the layers whose cached backdrop is made again (they look different), and `sharing`
    /// the layers of each live backdrop that several surfaces share (they cast their shadows into one picture, which every one
    /// of them shows).
    fn frame_damage(
        &self,
        stack: usize,
        facts: Facts,
        stale: &[usize],
        sharing: &[&[usize]],
    ) -> TileSet {
        let Facts {
            members,
            layers,
            seen,
            extents,
            asks,
            revisions,
        } = facts;
        let (width, height) = (self.panel.width, self.panel.height);
        let Some(prev) = &self.accounts[stack].prev else {
            return TileSet::full(width, height);
        };
        let before: HashMap<LayerId, &Frozen> = prev.iter().map(|f| (f.layer.id, f)).collect();
        // Layers that were there and are still are stacked as they were (a layer that is made again is a new one, and
        // can be anywhere): if not, all bets are off.
        let same_order = prev
            .iter()
            .filter(|f| layers.iter().any(|l| l.id == f.layer.id))
            .map(|f| f.layer.id)
            .eq(layers
                .iter()
                .filter(|l| before.contains_key(&l.id))
                .map(|l| l.id));
        if !same_order {
            return TileSet::full(width, height);
        }
        // Layer by layer: what each changes in the frame, and in the pictures behind the glass above it.
        let count = layers.len();
        let mut dirt: Vec<Dirt> = layers
            .iter()
            .enumerate()
            .map(|(i, layer)| match before.get(&layer.id) {
                None => Dirt {
                    seen: [false, true],
                    within: None,
                    extent: [None, extents[i]],
                },
                Some(old) => {
                    let now = Now {
                        seen: &seen[i],
                        extent: extents[i],
                        revision: revisions[i],
                    };
                    self.change(stack, old, members.contains(&i), layer, now)
                }
            })
            .collect();
        // A cached backdrop that is made again shows on its surfaces, but not in what is drawn above.
        for &i in stale {
            dirt[i].seen = [true, true];
            dirt[i].within = None;
        }
        // Where layers that are gone were.
        let here: std::collections::HashSet<LayerId> = layers.iter().map(|l| l.id).collect();
        let mut total = TileSet::new(width, height);
        let mut gone_mirrored = Region::NONE;
        for f in prev.iter().filter(|f| !here.contains(&f.layer.id)) {
            total.union(&f.seen);
            gone_mirrored.add_damage(f.extent);
        }
        // A glass surface with a live backdrop shows the layers below it, blurred: what changes anywhere near its body
        // (as far as the blur reaches) changes it, all of it; and the surfaces that share its picture show what any of them
        // casts into it. What a surface does to the ones above it in this way is no change of its own (it is drawn flat in
        // their pictures).
        let panel = panel_area_of(&self.panel);
        let mut forced = vec![false; count];
        loop {
            let mut grew = false;
            let mut below = gone_mirrored;
            for i in 0..count {
                if let Some(ask) = asks[i].filter(|a| a.refresh == Refresh::Live) {
                    let reach = self.backdrops.reach(ask.radius, ask.downsample);
                    let watched = [
                        (ask.body_now[0] - reach).max(panel[0]),
                        (ask.body_now[1] - reach).max(panel[1]),
                        (ask.body_now[2] + reach).min(panel[2]),
                        (ask.body_now[3] + reach).min(panel[3]),
                    ];
                    if !forced[i] && below.overlaps(&watched) {
                        forced[i] = true;
                        grew = true;
                    }
                }
                below.add_damage(dirt[i].extent[0]);
                below.add_damage(dirt[i].extent[1]);
            }
            for users in sharing {
                if users
                    .iter()
                    .any(|&u| dirt[u].extent != [None; 2] || stale.contains(&u))
                {
                    for &u in *users {
                        grew |= !std::mem::replace(&mut forced[u], true);
                    }
                }
            }
            if !grew {
                break;
            }
        }
        for (i, layer) in layers.iter().enumerate() {
            if forced[i] {
                dirt[i].seen = [true, true];
                dirt[i].within = None;
            }
            let Dirt {
                seen: asked,
                within,
                ..
            } = dirt[i];
            let was = before.get(&layer.id).map(|f| &f.seen).filter(|_| asked[0]);
            for set in [was, asked[1].then_some(&seen[i])].into_iter().flatten() {
                match within {
                    Some(area) => total.union_within(set, area),
                    None => total.union(set),
                }
            }
        }
        total
    }

    /// What a layer changes from the frame before: in the frame (where it was seen and where it is seen now) and in the
    /// pictures behind the glass above it (everywhere it was drawn and is). `now` is where it is drawn now, and what the
    /// picture of the prepared backdrop on it is a picture of.
    fn change(&self, stack: usize, old: &Frozen, member: bool, layer: &Drawn, now: Now) -> Dirt {
        let Now {
            extent, revision, ..
        } = now;
        if old.member != member {
            // Another plane's layer now, or this plane's no longer: what it was and what it is are both to be drawn.
            return Dirt {
                seen: [true, true],
                within: None,
                extent: [old.extent, extent],
            };
        }
        if old.layer == *layer {
            if old.revision == revision {
                return Dirt::default();
            }
            // What a glass surface shows through its body is another picture: the surface is drawn again, and nothing above
            // or below it is touched (a picture that glass above makes of it has it flat, without its backdrop).
            return Dirt {
                seen: [true, true],
                within: None,
                extent: [None; 2],
            };
        }
        // A picture the app changed in places, on a layer that is what it was in every other way: only those places.
        let unversioned = Drawn {
            version: old.layer.version,
            ..*layer
        };
        if matches!(layer.content, Content::Image) && unversioned == old.layer {
            let mut places: Damage = None;
            let mut any = false;
            for (_, rect) in self.accounts[stack]
                .damaged
                .iter()
                .filter(|(id, _)| *id == layer.id)
            {
                any = true;
                places = damage::union(places, self.image_area(layer, *rect));
            }
            if any {
                return match places {
                    Some(within) => Dirt {
                        seen: [false, true],
                        within: Some(within),
                        extent: [None, extent.and_then(|e| damage::intersect(&within, &e))],
                    },
                    None => Dirt::default(),
                };
            }
        }
        Dirt {
            seen: [true, true],
            within: None,
            extent: [old.extent, extent],
        }
    }

    /// Keeps the layers of this frame, and where they are drawn, to say what the next one changes. The tiles each was seen in
    /// are taken from `seen`, which is left empty; the sets of the frame before them are kept for the frame after.
    fn remember(
        &mut self,
        stack: usize,
        members: std::ops::Range<usize>,
        layers: &[Drawn],
        seen: &mut Vec<TileSet>,
        extents: &[Option<Area>],
        revisions: &[u64],
    ) {
        let account = &mut self.accounts[stack];
        let spare: Vec<TileSet> = account
            .prev
            .take()
            .map(|prev| prev.into_iter().map(|f| f.seen).collect())
            .unwrap_or_default();
        account.prev = Some(
            layers
                .iter()
                .enumerate()
                .zip(seen.drain(..).zip(extents))
                .map(|((i, layer), (seen, &extent))| Frozen {
                    layer: *layer,
                    revision: revisions[i],
                    member: members.contains(&i),
                    seen,
                    extent,
                })
                .collect(),
        );
        account.spare = spare;
        account.damaged.clear();
    }

    /// Draws a frame's `layers` (back to front) into the render targets `draws` name, each a picture of some of them. One
    /// report for each target; what is the frame's as a whole (pictures uploaded, meshes drawn, glass still getting its
    /// refraction back) is in the first. It is [`Renderer::plan_frame`], a [`Renderer::draw_planned`] for each target, and
    /// [`Renderer::finish_frame`].
    pub fn draw_frame(
        &mut self,
        layers: &[Drawn],
        draws: &[Draw],
        source: &dyn Source,
    ) -> Vec<DrawReport> {
        let mut planned = self.plan_frame(layers, source);
        let reports = draws
            .iter()
            .map(|draw| self.draw_planned(&mut planned, draw))
            .collect();
        self.finish_frame(planned);
        reports
    }

    /// Plans a frame: what belongs to the frame (adapting the glass to its motion, placing the layers, planning the glass
    /// surfaces' backdrops) is done once, and each target is then drawn with [`Renderer::draw_planned`], which knows what it
    /// shows and owes. [`Renderer::damage`] says what the frame changes, before anything is drawn.
    ///
    /// `source` is the scene as the groups of the prepared backdrops see it: the glass that shows one is drawn over a picture
    /// composed of its plates, each placed where its group is now, which is asked of the source for every frame.
    pub fn plan_frame(&mut self, layers: &[Drawn], source: &dyn Source) -> Planned {
        let mut first = DrawReport::default();
        // The count of what the GL is asked is the frame's: what the housekeeping between frames asked is not in it.
        self.gl.take_calls();
        // Whatever work between frames changed is drawn now.
        self.stale = false;
        self.frame += 1;
        // A glyph list that waits for chunks of its atlas that are queued is shown by this frame and not the one after: they
        // are landed first (within a chunk's worth of bytes).
        let landed = self
            .atlases
            .land_gated(&self.gl, &mut self.events, atlas::FRAME_UPLOAD_BYTES);
        self.uploaded_since = self
            .uploaded_since
            .saturating_add(u32::try_from(landed.bytes).unwrap_or(u32::MAX));
        // `ideas::ADAPTIVE` is about the frame and not about a target (it remembers where each surface was the frame before):
        // the layers are adapted once, and every target draws them as adapted. The renderer's own revisions are mixed into the
        // versions of the glyph runs at the same time.
        let adapting = self.ideas & ideas::ADAPTIVE != 0;
        let mut revised = std::mem::take(&mut self.revised);
        let layers = if adapting
            || self.ladder > 0
            || layers
                .iter()
                .any(|l| matches!(l.content, Content::Glyphs(_)))
        {
            self.revise(layers, adapting, &mut revised);
            first.fading = adapting
                && (self.calm.iter().any(|&calm| calm < 1.0 && calm > 0.0)
                    || self.motion.values().any(|(_, moved)| moved.is_some()));
            &revised[..]
        } else {
            self.calm.clear();
            layers
        };
        // What the picture of the prepared backdrop on each glass surface is a picture of, which is looked at once a frame:
        // where the groups of its plates are now decides whether it is composed again, and a frame that shows it is a frame
        // that has glass over it.
        let mut revisions = std::mem::take(&mut self.revisions);
        revisions.clear();
        revisions.extend(layers.iter().map(|layer| {
            match layer.content {
                Content::Glass(glass) => match glass.backdrop {
                    Backdrop::Prepared { id } => self
                        .sets
                        .observe(id, self.frame, |group| source.group_world(group)),
                    Backdrop::None | Backdrop::Cached { .. } | Backdrop::Live { .. } => 0,
                },
                _ => 0,
            }
        }));
        self.profile.frame_begins();

        let placing = Instant::now();
        self.vertices.clear();
        let mut placer = Placer {
            gl: &self.gl,
            panel: &self.panel,
            glass: &mut self.glass,
            textures: &mut self.textures,
            atlases: &self.atlases,
            sets: &self.sets,
            frame: self.frame,
            skips: self.profile.skips(),
        };
        Self::place(
            &mut placer,
            layers,
            View::Now,
            Pictures::Later,
            &mut self.vertices,
            &mut self.jobs,
            &mut first,
        );
        first.place_ns += nanos(placing);
        let planning = Instant::now();

        if self.profile.enabled() {
            let (surfaces, bodies, shadows, rims, vertices) = self.glass_coverage(layers);
            self.profile
                .glass(surfaces, bodies, shadows, rims, vertices);
        }

        let panel_area = panel_area_of(&self.panel);
        let mut job_of: Vec<Option<usize>> = vec![None; layers.len()];
        for (k, job) in self.jobs.iter().enumerate() {
            job_of[job.layer] = Some(k);
        }
        let extents: Vec<Option<Area>> = job_of
            .iter()
            .map(|job| job.map(|k| self.jobs[k].extent))
            .collect();

        // The glass surfaces' backdrops: which the frame needs, and which of the cached ones are out of date.
        let plan_layers: Vec<backdrop::Layer> =
            layers.iter().map(|layer| self.plan_layer(layer)).collect();
        let backdrops = &self.backdrops;
        let plan = backdrop::plan(&plan_layers, panel_area, |radius, downsample| {
            backdrops.reach(radius, downsample)
        });
        let fingerprints: Vec<u64> = plan
            .iter()
            .map(|b| backdrop::fingerprint(&plan_layers, b.upto, b.radius, b.downsample))
            .collect();
        let casts = Casts::of(&plan, layers.len());
        let stale: Vec<bool> = plan
            .iter()
            .enumerate()
            .map(|(index, b)| {
                b.refresh == Refresh::Cached
                    && self
                        .backdrops
                        .is_stale(b, layers[b.users[0]].id, fingerprints[index])
            })
            .collect();
        // What nothing of shows, in the frame or behind glass: what it does by itself is not worth a frame (`Scene::observe`).
        // The planes are composed by the display, so it is the frame as a whole that hides a layer, not the plane it is on.
        let everything: Vec<usize> = (0..self.jobs.len()).collect();
        let sights = self.sights(&everything);
        let looked = looked_at(&job_of, &sights, &plan, &stale);
        self.unseen.clear();
        self.unseen.extend(
            layers
                .iter()
                .zip(&looked)
                .filter(|(_, looked)| !**looked)
                .map(|(layer, _)| layer.id),
        );
        // The pictures of the layers that something looks at are brought up to date now, before anything is drawn. A layer an opaque one
        // covers, with no glass showing it through, is not looked at, and its picture waits for the housekeeping between frames
        // (`tidy`): a screen that is mounted under a curtain makes dozens of pictures, and copying them in the frame that draws the
        // curtain was the whole of that frame (13 to 19 ms on the Show for what no one saw).
        let uploading = Instant::now();
        for job in &self.jobs {
            let layer = &layers[job.layer];
            let picture = match layer.content {
                Content::Image => true,
                Content::Wallpaper(wallpaper) => wallpaper.kind.uses_image(),
                _ => false,
            };
            if picture && looked[job.layer] {
                if let Some(texture) = self.textures.get_mut(&layer.id) {
                    texture.used = self.frame;
                    upload_timed(&self.gl, texture, &mut first);
                }
            }
        }
        first.place_ns += nanos(uploading);
        if stale.iter().any(|&stale| stale) {
            let mut placer = Placer {
                gl: &self.gl,
                panel: &self.panel,
                glass: &mut self.glass,
                textures: &mut self.textures,
                atlases: &self.atlases,
                sets: &self.sets,
                frame: self.frame,
                skips: self.profile.skips(),
            };
            Self::place(
                &mut placer,
                layers,
                View::Rest,
                Pictures::Now,
                &mut self.vertices,
                &mut self.rest_jobs,
                &mut first,
            );
        } else {
            self.rest_jobs.clear();
        }
        // What the meshes of the glass cost is counted whichever view placed it.
        let (meshes, mesh_ns) = self.glass.take_built();
        first.shadow_meshes += meshes;
        first.shadow_ns += mesh_ns;
        first.plan_ns += nanos(planning);

        let owned = layers.to_vec();
        self.revised = revised;
        let frame = Frame {
            layers: owned,
            panel_area,
            job_of,
            extents,
            revisions,
            plan_layers,
            plan,
            fingerprints,
            casts,
            stale,
            sights,
            looked,
        };
        Planned {
            frame,
            first,
            done: Done::default(),
            changed: Vec::new(),
            backdrops: 0,
            uploaded: 0,
        }
    }

    /// Draws the frame `planned` into one render target: what of its layers can be seen, what of the target is owed, the
    /// pictures behind its glass, and the layers themselves.
    pub fn draw_planned(&mut self, planned: &mut Planned, draw: &Draw) -> DrawReport {
        let mut report = std::mem::take(&mut planned.first);
        self.draw_surface(
            &planned.frame,
            draw,
            &mut planned.done,
            &mut planned.changed,
            &mut report,
        );
        report.gl = self.gl.take_calls();
        planned.backdrops += report.backdrops;
        planned.uploaded += report.uploaded_pixels;
        report
    }

    /// What the frame `planned` changes of the plane `surface` is on, from the frame before it, in tiles. It is worked out the
    /// first time it is asked (or when the plane is first drawn), and what the plane showed is kept for the frame after: ask
    /// for every plane that is shown, once a frame, whether it is drawn or not.
    pub fn damage<'p>(&mut self, planned: &'p mut Planned, surface: Surface) -> &'p TileSet {
        self.account(&planned.frame, surface, &mut planned.changed);
        planned.changed[surface.stack]
            .as_ref()
            .expect("the plane was accounted")
    }

    /// The frame is over: its buffers are kept for the next. What planning cost and did, if no target was drawn to take it into
    /// its report, is returned.
    pub fn finish_frame(&mut self, planned: Planned) -> DrawReport {
        let Planned {
            frame,
            first,
            done,
            backdrops,
            uploaded,
            ..
        } = planned;
        self.revisions = frame.revisions;
        // The backdrops are only counted as used, and the ones nobody used dropped, by a frame that drew any.
        if done.opened {
            self.backdrops.end_frame();
        }
        self.profile.frame_ends(backdrops);
        self.uploaded_since = self.uploaded_since.saturating_add(picture_bytes(uploaded));
        first
    }

    /// What of the layers of the jobs `members` can be seen: a layer that an opaque one of them covers is not drawn, and one
    /// that it covers in part is drawn where it is not. (The pictures the glass surfaces are made of below still draw
    /// everything below them: a layer under an opaque one is in the blur behind a surface between the two.) The other jobs
    /// are hidden.
    fn sights(&self, members: &[usize]) -> Vec<Sight> {
        let mut sights = vec![Sight::Hidden; self.jobs.len()];
        if self.culling {
            let pieces: Vec<Piece> = members
                .iter()
                .map(|&k| Piece {
                    extent: self.jobs[k].extent,
                    opaque: self.jobs[k].cover.rects(),
                })
                .collect();
            for (&k, sight) in members.iter().zip(damage::sights(&pieces)) {
                sights[k] = sight;
            }
        } else {
            for &k in members {
                sights[k] = Sight::Seen(self.jobs[k].extent);
            }
        }
        sights
    }

    /// `region` with the rectangles merged that are cheaper drawn in one pass over their cover than in a pass each ([`Region::merged`]).
    /// A pass over a rectangle draws every layer that meets it, so the pass costs the rectangle's pixels and [`damage::DRAW_WORTH`]
    /// for each of them: the rows of a page that scrolls are a band each, with a few percent of the page between them, and under
    /// them are the page's card, its shadows and the wallpaper, drawn in all of the passes where the cover draws each once. A pass
    /// draws the layers `layers`, and those below them as one picture that it copies (a host's floor): they count as one layer.
    ///
    /// A cover that would bring the surfaces of a backdrop into the frame that no rectangle brings is not taken, whatever it saves:
    /// a live backdrop is a blur made for the frame (once, however the passes are cut) for every group that is drawn.
    fn merge_passes(region: Region, frame: &Frame, sights: &[Sight], layers: std::ops::Range<usize>) -> Region {
        // One bit for each backdrop of the plan; a plan of more than that leaves the region as it is.
        if region.rects().len() < 2 || frame.plan.len() > u64::BITS as usize {
            return region;
        }
        let seen_in = |layer: usize, area: &Area| {
            frame.job_of[layer]
                .and_then(|job| sights[job].area())
                .is_some_and(|seen| backdrop::overlaps(&seen, area))
        };
        // The jobs are in the layers' order: the first job of a layer at or past `index` is where those layers' jobs begin.
        let job_from = |index: usize| {
            frame.job_of[index.min(frame.job_of.len())..]
                .iter()
                .flatten()
                .next()
                .copied()
                .unwrap_or(sights.len())
                .min(sights.len())
        };
        let (first, end) = (job_from(layers.start), job_from(layers.end));
        let cost = |area: &Area| {
            let drawn = sights[first..end.max(first)]
                .iter()
                .filter(|sight| {
                    sight
                        .area()
                        .is_some_and(|seen| backdrop::overlaps(&seen, area))
                })
                .count() as u64
                + u64::from(first > 0);
            damage::RECT_WORTH + damage::pixels(area) + damage::DRAW_WORTH * drawn
        };
        let groups = |area: &Area| {
            frame
                .plan
                .iter()
                .enumerate()
                .filter(|(_, b)| b.users.iter().any(|&user| seen_in(user, area)))
                .fold(0u64, |groups, (group, _)| groups | 1 << group)
        };
        region.merged(cost, |a, b, one| groups(one) == groups(a) | groups(b))
    }

    /// Works out what the frame changes of the plane `surface` is on, once for the frame (what the plane showed is then what
    /// the next frame is compared with), and keeps it in `memo` by stack.
    fn account(&mut self, frame: &Frame, surface: Surface, memo: &mut Vec<Option<TileSet>>) {
        let Surface {
            stack, from, to, ..
        } = surface;
        if memo.get(stack).is_some_and(Option::is_some) {
            return;
        }
        let (width, height) = (self.panel.width, self.panel.height);
        let layers = &frame.layers[..];
        // What of each layer can be seen, in tiles.
        let mut seen = std::mem::take(&mut self.accounts[stack].spare);
        let culling = self.culling;
        let pieces: Vec<(Area, &[Area])> = (0..to)
            .map(|layer| match frame.job_of[layer] {
                Some(k) if layer >= from => {
                    let job = &self.jobs[k];
                    (job.extent, if culling { job.cover.rects() } else { &[] })
                }
                _ => ([0; 4], &[][..]),
            })
            .collect();
        tiles::visible(width, height, &pieces, &mut seen);
        drop(pieces);
        let asks: Vec<Option<Ask>> = frame.plan_layers[..to].iter().map(|l| l.ask).collect();
        let stale_layers: Vec<usize> = frame
            .plan
            .iter()
            .zip(&frame.stale)
            .filter(|(_, stale)| **stale)
            .flat_map(|(b, _)| b.users.iter().copied())
            .filter(|&user| user < to)
            .collect();
        let sharing: Vec<&[usize]> = frame
            .plan
            .iter()
            .filter(|b| b.refresh == Refresh::Live && b.users.len() > 1)
            .filter(|b| b.users.iter().all(|&user| user < to))
            .map(|b| b.users.as_slice())
            .collect();
        let facts = Facts {
            members: from..to,
            layers: &layers[..to],
            seen: &seen,
            extents: &frame.extents[..to],
            asks: &asks,
            revisions: &frame.revisions[..to],
        };
        let changed = self.frame_damage(stack, facts, &stale_layers, &sharing);
        self.remember(
            stack,
            from..to,
            &layers[..to],
            &mut seen,
            &frame.extents[..to],
            &frame.revisions[..to],
        );
        if memo.len() <= stack {
            memo.resize(stack + 1, None);
        }
        memo[stack] = Some(changed);
    }

    /// One render target of a frame: what of its layers can be seen, what of it is owed, the pictures behind its glass, and
    /// the layers themselves.
    fn draw_surface(
        &mut self,
        frame: &Frame,
        draw: &Draw,
        done: &mut Done,
        changed: &mut Vec<Option<TileSet>>,
        report: &mut DrawReport,
    ) {
        let Surface {
            stack,
            from,
            to,
            transparent,
        } = draw.surface;
        let (layers, panel_area) = (&frame.layers[..], frame.panel_area);

        // What of the frame can be seen: a layer that an opaque layer above it covers is not drawn, and one that it covers
        // in part is drawn where it is not. (The pictures the glass surfaces are made of below still draw everything below
        // them: a layer under an opaque one is in the blur behind a surface between the two.) A layer the target does not
        // show is not in its frame at all.
        let members: Vec<usize> = (0..self.jobs.len())
            .filter(|&k| (from..to).contains(&self.jobs[k].layer))
            .collect();
        let own;
        let sights = if members.len() == self.jobs.len() {
            &frame.sights
        } else {
            own = self.sights(&members);
            &own
        };
        report.culled = members
            .iter()
            .filter(|&&k| sights[k] == Sight::Hidden)
            .count() as u32;
        let (width, height) = (self.panel.width, self.panel.height);

        // What the frame has to draw: all of it, or the part of it that this render target owes (the renderer's account of
        // the ring) or that the caller asks for. The plane's account covers the layers it shows and the ones below them,
        // which show through the glass above.
        let region = match (draw.slot, draw.areas) {
            (None, None) => {
                report.damage = Region::of(panel_area);
                report.damage_px = damage::pixels(&panel_area);
                Region::of(panel_area)
            }
            (slot, areas) => {
                let accounting = Instant::now();
                self.account(frame, draw.surface, changed);
                let changed = changed[stack].as_ref().expect("the plane was accounted");
                report.damage = changed.region();
                report.damage_px = changed.pixels();
                let region = match (slot, areas) {
                    (Some(slot), _) => {
                        let mut owed = TileSet::new(width, height);
                        self.accounts[stack].ring.frame(slot, changed, &mut owed);
                        owed.region()
                    }
                    (None, areas) => areas.unwrap_or(Region::of(panel_area)),
                };
                report.damage_ns += nanos(accounting);
                region
            }
        };
        if region.is_empty() {
            // What the target holds is what this frame shows.
            report.gl_error = self.gl.take_error();
            return;
        }
        // A region that is nearly all of the panel is drawn as all of it: a scissor would save nothing. Otherwise each of
        // its rectangles is drawn in whole tiles (drawing more of what the frame shows is never wrong), on its own, as far
        // as the account keeps them apart (`LUMASHOW_DRAW_OFF=rects` draws their one cover, to measure what that costs) and
        // as far as a pass of its own pays: the layers it draws are drawn again in every other pass they meet. The areas a
        // caller asks for are drawn as they are: what is in the target around them is not this frame's to draw over.
        let region = if draw.slot.is_none() && draw.areas.is_some() {
            region
        } else {
            let region = region.tiled(GPU_TILE, panel_area).limited(self.rects);
            Self::merge_passes(region, frame, sights, 0..frame.layers.len())
        };
        let exact = draw.slot.is_none() && draw.areas.is_some();
        let partial =
            (exact || region.pixels() * 20 < damage::pixels(&panel_area) * 19).then_some(region);
        let drawn_region = partial.unwrap_or(Region::of(panel_area));
        report.region_px = drawn_region.pixels();
        report.rects = drawn_region.rects().len() as u16;

        // The pictures of the meshes that are looked at, each drawn into its own target now: not between the passes that
        // follow. What nobody looks at is not drawn, however it turns, and is drawn as it is when someone does.
        if !std::mem::replace(&mut done.meshed, true) {
            for (layer, _) in layers
                .iter()
                .zip(&frame.looked)
                .filter(|(_, looked)| **looked)
            {
                if let Content::Mesh(style) = &layer.content {
                    // The orbit's angle is kept whole in the scene, and turns are all the picture needs.
                    let angle = (layer.phase % 360.0) as f32;
                    report.meshes +=
                        u16::from(self.meshes.prepare(&self.gl, layer.id, style, angle));
                }
            }
        }

        // Glass that stands still is shown from plates (`ideas::STILL_GLASS`): which surfaces, and whose pictures are made.
        let drawn_groups: Vec<bool> = frame
            .plan
            .iter()
            .map(|b| {
                b.users.iter().any(|&user| {
                    frame.job_of[user].is_some_and(|job| {
                        sights[job]
                            .area()
                            .is_some_and(|area| drawn_region.overlaps(&area))
                    })
                })
            })
            .collect();
        let mut planned = self.plan_plates(
            layers,
            &frame.plan,
            &drawn_groups,
            &frame.fingerprints,
            &frame.stale,
        );
        self.make_plates(layers, &frame.plan, &mut planned);
        let gl = &self.gl;
        if !std::mem::replace(&mut done.uploaded, true) {
            gl.use_vertex_buffer(self.vertex_buffer);
            gl.stream_vertices(self.vertex_buffer, &self.vertices);
            self.profile.part_ends(gl, Part::Place);
        }

        let drawing = Drawing {
            gl,
            panel: &self.panel,
            programs: &self.programs,
            glass: &self.glass,
            textures: &self.textures,
            atlases: &self.atlases,
            wallpapers: &self.wallpapers,
            meshes: &self.meshes,
            live: &self.live,
            skips: self.profile.skips(),
            vertex_buffer: self.vertex_buffer,
            plates: &self.plates,
            origin: [0, 0],
            cast_done: std::cell::RefCell::new(std::mem::take(&mut self.cast_done)),
            fills: self
                .profile
                .enabled()
                .then(|| std::cell::RefCell::new(Vec::new())),
        };
        if !std::mem::replace(&mut done.opened, true) {
            self.backdrops.begin_frame();
        }
        let vertex_buffer = self.vertex_buffer;
        let mut pictures: Vec<Option<u32>> = vec![None; layers.len()];
        let backdrops_began = Instant::now();
        // The held blurs of the frame, counted as they come: each is made again every `HOLD_FRAMES` frames and the k-th of them k
        // frames later than the first, so that a frame with three of them makes one and not all three (see below).
        let mut held_before = 0u64;
        for (index, b) in frame.plan.iter().enumerate() {
            // A surface nothing of which is drawn needs no picture, nor does one shown from plates that stand; a cached one is
            // kept for when it is.
            if !drawn_groups[index] || !planned.picture[index] {
                if b.refresh == Refresh::Cached {
                    if drawn_groups[index] {
                        self.backdrops.standing(layers[b.users[0]].id);
                    } else {
                        self.backdrops.keep(layers[b.users[0]].id);
                    }
                }
                continue;
            }
            let jobs = match b.refresh {
                Refresh::Live => &self.jobs,
                Refresh::Cached => &self.rest_jobs,
            };
            let view = match b.refresh {
                Refresh::Live => View::Now,
                Refresh::Cached => View::Rest,
            };
            let factor = 1.0 / f32::from(b.downsample);
            // The blur of a surface that is moving is held (`ideas::ADAPTIVE`): kept, as a picture of the whole panel that
            // stands in for it, and made again every `HOLD_FRAMES` frames, which is too soon for anything to have moved that
            // the blur could show. Each held picture has its own phase, its rank among the held pictures of the frame: with the
            // one clock for all of them (`frame / HOLD_FRAMES`) the three blurs of a page that slides over another were made in
            // the same frame, one in four, and that frame cost 12 to 14 ms where the three between cost 8 (ha_dashboard's room
            // slide on the Show: `tb` 3.3 ms in it and none in the others). Every picture is still made every fourth frame and is
            // never older than it was.
            let held = self.ideas & ideas::ADAPTIVE != 0
                && b.refresh == Refresh::Live
                && b.users
                    .iter()
                    .all(|&user| self.calm.get(user).is_some_and(|&calm| calm < 1.0));
            let held_as = held.then(|| backdrop::Backdrop {
                refresh: Refresh::Cached,
                area: panel_area,
                ..b.clone()
            });
            let (asked, fingerprint) = match &held_as {
                Some(held_as) => {
                    let phase = held_before % HOLD_FRAMES;
                    held_before += 1;
                    (held_as, (self.frame + phase) / HOLD_FRAMES)
                }
                None => (b, frame.fingerprints[index]),
            };
            let made = self.backdrops.picture(
                gl,
                asked,
                layers[b.users[0]].id,
                fingerprint,
                |mirror, area| {
                    // The layers below, again, a small picture of them within the area that matters. The blur of the
                    // picture before this one left its own vertices bound: the frame's are needed.
                    gl.use_vertex_buffer(vertex_buffer);
                    gl.bind_framebuffer(mirror.framebuffer);
                    gl.viewport(0, 0, mirror.width as i32, mirror.height as i32);
                    let region = pix_rect_of(area);
                    if let Some(region) = region {
                        let pixels = region.scaled(factor);
                        gl.enable(GL_SCISSOR_TEST);
                        gl.scissor(
                            pixels.x as i32,
                            pixels.y as i32,
                            pixels.w as i32,
                            pixels.h as i32,
                        );
                    }
                    gl.enable(GL_BLEND);
                    gl.clear(0.0, 0.0, 0.0, 1.0);
                    drawing.draw(
                        layers,
                        jobs,
                        &Pass {
                            view,
                            factor,
                            upto: b.upto,
                            region,
                            pictures: &[],
                            casts: &frame.casts,
                            // A live picture has the shadows of the surfaces that share it in it; a cached one, which
                            // must not be made again for a surface moving, has none.
                            cast_after: if b.refresh == Refresh::Live && b.users.len() > 1 {
                                &b.users
                            } else {
                                &[]
                            },
                            sights: None,
                            reuse: &[],
                        },
                    );
                    gl.disable(GL_SCISSOR_TEST);
                },
            );
            if let Some(made) = made {
                report.backdrops += u16::from(made.remade);
                if made.remade && b.refresh == Refresh::Cached {
                    self.profile.remade(
                        layers[b.users[0]].id,
                        frame.plan_layers[..b.upto]
                            .iter()
                            .filter(|l| l.rest.is_some())
                            .map(|l| (l.id, l.version))
                            .collect(),
                    );
                }
                for &user in &b.users {
                    pictures[user] = Some(made.texture);
                }
            }
            self.profile.part_ends(gl, Part::Backdrops);
        }
        report.backdrop_ns += nanos(backdrops_began);

        // The glass over a prepared backdrop that the target draws reads the picture of its set, composed from the plates of the
        // generation shown, each where its group is now. It is composed again only when a generation was published or a group
        // moved since it was (a few textured quads); it makes no capture, and what is below the glass is not looked at. A set with
        // no generation yet has no picture, and the glass is the tint alone.
        let composer = prepare::Composer {
            gl,
            panel: &self.panel,
            image: &self.programs.plain.image,
            feathered: &self.programs.feathered.image,
            buffer: self.compose_buffer,
            frame_buffer: vertex_buffer,
        };
        for (k, job) in self.jobs.iter().enumerate() {
            let Content::Glass(glass) = layers[job.layer].content else {
                continue;
            };
            let Backdrop::Prepared { id } = glass.backdrop else {
                continue;
            };
            let drawn = sights[k]
                .area()
                .is_some_and(|area| drawn_region.overlaps(&area));
            if drawn {
                pictures[job.layer] = composer.compose(
                    &mut self.sets,
                    id,
                    &mut self.compose_vertices,
                    &mut self.compose_draws,
                );
            }
        }

        // The surfaces shown from plates that are made afresh in this frame: each drawn into its plate, alone, over nothing. A
        // group whose picture could not be made is drawn as it always was, and lets go of its plates.
        for b in &frame.plan {
            let render = |user: &usize| planned.reuse[*user].is_some_and(|reuse| reuse.render);
            if pictures[b.users[0]].is_none() && b.users.iter().any(render) {
                for &user in &b.users {
                    planned.reuse[user] = None;
                }
                self.still.forget(layers[b.users[0]].id);
            }
        }
        if planned.reuse.iter().flatten().any(|reuse| reuse.render) {
            gl.use_vertex_buffer(vertex_buffer);
            gl.enable(GL_BLEND);
            gl.enable(GL_SCISSOR_TEST);
            let alone = Pass {
                view: View::Now,
                factor: 1.0,
                upto: usize::MAX,
                region: None,
                pictures: &pictures,
                casts: &frame.casts,
                cast_after: &[],
                sights: None,
                reuse: &[],
            };
            let mut bound = usize::MAX;
            for job in &self.jobs {
                let Some(reuse) = planned.reuse[job.layer].filter(|reuse| reuse.render) else {
                    continue;
                };
                let layer = &layers[job.layer];
                let (Content::Glass(glass), Vertices::Glass(placed), Some(rect)) =
                    (&layer.content, &job.draw, pix_rect_of(reuse.rect))
                else {
                    continue;
                };
                let plate = self.plates[reuse.plate].expect("a plate drawn into is made first");
                if bound != reuse.plate {
                    bound = reuse.plate;
                    gl.bind_framebuffer(plate.framebuffer);
                    gl.viewport(0, 0, plate.width as i32, plate.height as i32);
                }
                gl.scissor(rect.x as i32, rect.y as i32, rect.w as i32, rect.h as i32);
                gl.clear(0.0, 0.0, 0.0, 0.0);
                let draw = drawing.glass_draw(layer, job, &alone);
                drawing.glass.draw_body(gl, glass, placed, &draw);
            }
            gl.disable(GL_SCISSOR_TEST);
        }
        let drawing_began = Instant::now();
        // The frame itself: all of it, or each rectangle of the region on its own, cleared and drawn within its scissor (the
        // layers that touch none of it are not drawn at all).
        gl.bind_framebuffer(draw.target);
        gl.viewport(0, 0, self.panel.width as i32, self.panel.height as i32);
        // The passes before may have ended with blending off.
        gl.enable(GL_BLEND);
        gl.use_vertex_buffer(self.vertex_buffer);
        let pass = |rect: Option<PixRect>| Pass {
            view: View::Now,
            factor: 1.0,
            upto: usize::MAX,
            region: rect,
            pictures: &pictures,
            casts: &frame.casts,
            cast_after: &[],
            sights: Some(sights),
            reuse: &planned.reuse,
        };
        // Clearing costs nothing on a tile-based GPU, and tells it there is nothing to load from memory. A plane over
        // another is cleared to nothing, so that what it does not draw shows the plane below.
        let clear_to = if transparent { 0.0 } else { 1.0 };
        match partial {
            None => {
                gl.disable(GL_SCISSOR_TEST);
                if draw.clear {
                    gl.clear(0.0, 0.0, 0.0, clear_to);
                }
                report.drawn += drawing.draw(layers, &self.jobs, &pass(None));
            }
            Some(region) => {
                gl.enable(GL_SCISSOR_TEST);
                for rect in region.rects().iter().copied().filter_map(pix_rect_of) {
                    gl.scissor(rect.x as i32, rect.y as i32, rect.w as i32, rect.h as i32);
                    if draw.clear {
                        gl.clear(0.0, 0.0, 0.0, clear_to);
                    }
                    // The pass before may have ended with blending off, and a pass begins with it on.
                    gl.enable(GL_BLEND);
                    report.drawn += drawing.draw(layers, &self.jobs, &pass(Some(rect)));
                }
            }
        }
        gl.disable(GL_SCISSOR_TEST);
        gl.flush();
        report.draw_ns += nanos(drawing_began);
        self.profile.part_ends(gl, Part::Frame);
        report.still = planned.reuse.iter().flatten().count() as u16;
        report.gl_error = gl.take_error();
        self.cast_done = drawing.cast_done.into_inner();
        if let Some(fills) = drawing.fills {
            self.profile.fills(&fills.into_inner());
        }
    }
}

/// When [`Renderer::place`] brings the pictures of the layers it places up to date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pictures {
    /// As each layer is placed: what a picture of the layers at rest or a plate is drawn from is all of it, and drawn at once.
    Now,
    /// Not at all: the frame brings up to date the pictures of the layers something looks at (`looked_at`), once it knows which
    /// they are, and leaves the others to the housekeeping between frames.
    Later,
}

/// What placing layers needs and changes: textures are brought up to date, glass keeps its shadow meshes; the quads of a
/// glyph run and whether a prepared backdrop has a picture are looked up.
struct Placer<'a> {
    gl: &'a Gles,
    panel: &'a Panel,
    glass: &'a mut glass::Surfaces,
    textures: &'a mut HashMap<LayerId, Texture>,
    atlases: &'a Atlases,
    sets: &'a Sets,
    /// The number of the frame being drawn.
    frame: u64,
    /// Parts left out for the profiler.
    skips: Skips,
}

/// What drawing a run of jobs keeps track of, so that GL state is only set when it changes.
struct State {
    program: u32,
    scissor_on: bool,
    blending: bool,
}

/// The glass surfaces that share a backdrop cast their shadows together, before the first of their bodies is drawn
/// (`backdrop` explains why).
struct Casts {
    /// The run a layer's surface belongs to, by layer index; `None` for one that has its backdrop, or none, to itself.
    run_of: Vec<Option<usize>>,
    /// The surfaces of each run, as layer indices in drawing order.
    runs: Vec<Vec<usize>>,
}

impl Casts {
    fn of(plan: &[backdrop::Backdrop], layers: usize) -> Self {
        let mut run_of = vec![None; layers];
        let mut runs = Vec::new();
        for shared in plan.iter().filter(|b| b.users.len() > 1) {
            for &user in &shared.users {
                run_of[user] = Some(runs.len());
            }
            runs.push(shared.users.clone());
        }
        Self { run_of, runs }
    }

    /// No surface shares a backdrop with another: each casts its own shadows (the glass of a plate is drawn flat).
    fn none() -> Self {
        Self {
            run_of: Vec::new(),
            runs: Vec::new(),
        }
    }
}

/// What one run of layers is drawn as: into the frame, or into a backdrop's small picture.
struct Pass<'a> {
    view: View,
    /// Output pixels per panel pixel: 1 for the frame, less for a picture of it.
    factor: f32,
    /// Only the layers before this index are drawn.
    upto: usize,
    /// Only this part of the panel is drawn, when it is a picture of part of it.
    region: Option<PixRect>,
    /// The backdrop picture of each layer that has one, by layer index.
    pictures: &'a [Option<u32>],
    casts: &'a Casts,
    /// Surfaces whose shadows are drawn last, over the layers before `upto`: the ones that share the backdrop this pass
    /// is the picture of.
    cast_after: &'a [usize],
    /// What can be seen of each job's layer (by job), when the pass is the frame itself: what is covered is not drawn.
    sights: Option<&'a [Sight]>,
    /// The surfaces shown from a plate, by layer index (`ideas::STILL_GLASS`); none in a picture, which draws glass flat.
    reuse: &'a [Option<still::Reuse>],
}

/// Everything drawing a run of layers needs, borrowed from the renderer.
struct Drawing<'a> {
    gl: &'a Gles,
    panel: &'a Panel,
    programs: &'a Programs,
    glass: &'a glass::Surfaces,
    textures: &'a HashMap<LayerId, Texture>,
    atlases: &'a Atlases,
    wallpapers: &'a Wallpapers,
    meshes: &'a MeshRenderer,
    /// The pictures producers of frames have on the GPU.
    live: &'a live::Sources,
    /// Parts left out for the profiler.
    skips: Skips,
    /// The frame's vertices' buffer, which whatever drew from another has to make the next layers' again.
    vertex_buffer: u32,
    /// The plates that still glass is shown from.
    plates: &'a [Option<RenderTarget>],
    /// Where the viewport's corner is in the render target, in its pixels: the plate of a prepared backdrop has a margin round
    /// what is drawn, and the scissor, which is in the target's pixels, is put by it. Zero for every other target.
    origin: [i32; 2],
    /// Which runs of shared shadows the pass being drawn has cast (by run): the renderer's scratch buffer, lent for the frame.
    cast_done: std::cell::RefCell<Vec<bool>>,
    /// For the profiler: the panel pixels each plain layer (not glass, not a wallpaper) was drawn over in the frame, by layer, when a
    /// profile is being made; glass is counted by its own placement.
    fills: Option<std::cell::RefCell<Vec<crate::profile::Fill>>>,
}

impl Drawing<'_> {
    /// The scissor the `index`th job of `pass` is drawn within, or `None` when none of it is drawn: what its clip, what
    /// covers it (in a pass that is the frame) and the part of the panel the pass draws leave of it.
    fn scissor_of(job: &Job, index: usize, pass: &Pass) -> Option<Option<PixRect>> {
        let mut scissor = job.scissor;
        if let Some(sights) = pass.sights {
            match sights[index] {
                Sight::Hidden => return None,
                Sight::Seen(area) if area != job.extent => {
                    let rect = pix_rect_of(area)?;
                    scissor = Some(match scissor {
                        Some(clip) => clip.intersect(rect)?,
                        None => rect,
                    });
                }
                Sight::Seen(_) => {}
            }
        }
        if let Some(region) = pass.region {
            if !backdrop::overlaps(&job.extent, &area_of(region)) {
                return None;
            }
        }
        Some(scissor)
    }

    /// Sets the scissor for a job drawn within `scissor` (see [`Drawing::scissor_of`]) inside what `pass` draws, and says
    /// whether anything of it is inside.
    fn clip(&self, scissor: Option<PixRect>, pass: &Pass, state: &mut State) -> bool {
        let clip = match (scissor, pass.region) {
            (Some(clip), Some(region)) => match clip.intersect(region) {
                Some(both) => Some(both),
                None => return false,
            },
            (clip, region) => clip.or(region),
        };
        match clip {
            Some(rect) => {
                let rect = rect.scaled(pass.factor);
                if !state.scissor_on {
                    self.gl.enable(GL_SCISSOR_TEST);
                    state.scissor_on = true;
                }
                self.gl.scissor(
                    rect.x as i32 + self.origin[0],
                    rect.y as i32 + self.origin[1],
                    rect.w as i32,
                    rect.h as i32,
                );
            }
            None if state.scissor_on => {
                self.gl.disable(GL_SCISSOR_TEST);
                state.scissor_on = false;
            }
            None => {}
        }
        true
    }

    /// How the glass surface of `layer`, placed by `job`, is drawn in `pass`.
    fn glass_draw(&self, layer: &Drawn, job: &Job, pass: &Pass) -> glass::Draw {
        let props = props_of(layer, pass.view);
        let at = placement(layer, props);
        glass::Draw {
            size: (layer.width as f32, layer.height as f32),
            scale: props.scale * pass.factor,
            opacity: props.opacity,
            origin: {
                let corner = self.panel.quad_of(&at, [0.0; 4])[0];
                [corner[0], corner[1]]
            },
            axes: self.panel.axes(props.scale),
            backdrop: pass.pictures.get(job.layer).copied().flatten(),
            feather: job
                .feather
                .map(|ramp| feather::Uniforms::new(&ramp, pass.factor, self.origin)),
            skips: self.skips,
        }
    }

    /// Draws the shadows of the surfaces `members` (layer indices) that have a job: the surfaces that share a backdrop
    /// cast them together, where the first of them is, and none of their bodies is drawn over one of the shadows.
    fn cast(
        &self,
        layers: &[Drawn],
        jobs: &[Job],
        members: &[usize],
        pass: &Pass,
        state: &mut State,
    ) {
        for &member in members {
            let Ok(at) = jobs.binary_search_by_key(&member, |job| job.layer) else {
                continue;
            };
            let job = &jobs[at];
            let layer = &layers[member];
            let (Content::Glass(glass), Vertices::Glass(placed)) = (&layer.content, &job.draw)
            else {
                continue;
            };
            let Some(scissor) = Self::scissor_of(job, at, pass) else {
                continue;
            };
            if !self.clip(scissor, pass, state) {
                continue;
            }
            if !state.blending {
                state.blending = true;
                self.gl.enable(GL_BLEND);
            }
            let draw = self.glass_draw(layer, job, pass);
            self.glass.draw_shadows(self.gl, glass, placed, &draw);
        }
        // The shadows' program was used; the next layer must choose its own again.
        state.program = 0;
    }

    /// Draws the jobs `pass` asks for, in order. Returns how many were drawn.
    fn draw(&self, layers: &[Drawn], jobs: &[Job], pass: &Pass) -> u32 {
        let gl = self.gl;
        let mut drawn = 0;
        let mut state = State {
            program: 0,
            scissor_on: pass.region.is_some(),
            blending: true,
        };
        let mut cast_done = self.cast_done.borrow_mut();
        cast_done.clear();
        cast_done.resize(pass.casts.runs.len(), false);
        for (index, job) in jobs
            .iter()
            .enumerate()
            .take_while(|(_, job)| job.layer < pass.upto)
        {
            let layer = &layers[job.layer];

            let props = props_of(layer, pass.view);
            let run = pass.casts.run_of.get(job.layer).copied().flatten();
            if let Some(run) = run.filter(|&run| !std::mem::replace(&mut cast_done[run], true)) {
                // Only the members below the picture being made: the ones above it are not in it.
                let members = &pass.casts.runs[run];
                let below = members.partition_point(|&member| member < pass.upto);
                self.cast(layers, jobs, &members[..below], pass, &mut state);
            }
            let Some(scissor) = Self::scissor_of(job, index, pass) else {
                continue;
            };
            if !self.clip(scissor, pass, &mut state) {
                continue;
            }
            let (width, height) = (layer.width as f32, layer.height as f32);
            let opacity = props.opacity;
            let scale = props.scale * pass.factor;
            // A layer that reaches into the fade of its clip is drawn by the programs that have the fade (which are not
            // opaque where it is not 1); every other layer is drawn by the plain ones, as it ever was.
            let fade = job
                .feather
                .map(|ramp| feather::Uniforms::new(&ramp, pass.factor, self.origin));
            let faded = fade.is_some();
            let set = if faded {
                &self.programs.feathered
            } else {
                &self.programs.plain
            };
            let (used, blend, slot) = match layer.content {
                Content::Image => (set.image.id, true, &set.image.feather),
                // A run is placed only once its atlas has a texture; this keeps a run whose atlas is gone from drawing from
                // nothing.
                Content::Glyphs(style) if self.atlases.texture(style.atlas).is_none() => continue,
                Content::Glyphs(_) => (set.glyphs.id, true, &set.glyphs.feather),
                // Nobody looked at the mesh, or its picture could not be made: there is nothing of it to show.
                Content::Mesh(_) if self.meshes.texture(layer.id).is_none() => continue,
                Content::Mesh(_) => (set.image.id, true, &set.image.feather),
                // No picture yet (or a `Clear`, or a source the GPU never took): nothing of the layer is shown.
                Content::Live(live) if self.live.texture(live.source).is_none() => continue,
                Content::Live(_) => (set.image.id, true, &set.image.feather),
                Content::Shape(shape) => match shape_kind(&shape, props) {
                    ShapeKind::Solid => (
                        set.solid.id,
                        faded || !covers(&shape, opacity),
                        &set.solid.feather,
                    ),
                    ShapeKind::Gradient => (
                        set.gradient.id,
                        faded || !covers(&shape, opacity),
                        &set.gradient.feather,
                    ),
                    ShapeKind::Rounded => (set.rounded.id, true, &set.rounded.feather),
                },
                Content::Glass(glass) => {
                    let Vertices::Glass(placed) = &job.draw else {
                        unreachable!("a glass layer was placed as glass");
                    };
                    if !state.blending {
                        state.blending = true;
                        gl.enable(GL_BLEND);
                    }
                    let draw = self.glass_draw(layer, job, pass);
                    match pass.reuse.get(job.layer).copied().flatten() {
                        // Shown from its plate: its shadows (unless they were cast with the rest of its run's) are drawn as
                        // ever, and its body is copied.
                        Some(reuse) => {
                            if run.is_none() {
                                self.glass.draw_shadows(gl, &glass, placed, &draw);
                            }
                            // What is copied has the fade of its clip in it where there is one, and so has an alpha short of 1 at
                            // the clip's edges: it is blended, not written.
                            let opaque = glass.backdrop != Backdrop::None
                                && draw.opacity >= 1.0
                                && draw.feather.is_none();
                            let plate = self.plates[reuse.plate]
                                .expect("a surface is shown from a plate that was made")
                                .texture;
                            self.glass.blit_body(gl, placed, opaque, plate);
                        }
                        // Its shadows were cast with the rest of its run's.
                        None if run.is_some() => self.glass.draw_body(gl, &glass, placed, &draw),
                        None => self.glass.draw(gl, &glass, placed, &draw),
                    }
                    drawn += 1;
                    // The glass programs were used; the next layer must choose its own again.
                    state.program = 0;
                    continue;
                }
                Content::Wallpaper(_) if self.skips.wallpaper => continue,
                Content::Wallpaper(wallpaper) => {
                    let at = placement(layer, props);
                    self.wallpapers.draw(
                        gl,
                        &wallpaper,
                        &wallpapers::Draw {
                            size: (width, height),
                            opacity,
                            origin: {
                                let corner = self.panel.quad_of(&at, [0.0; 4])[0];
                                [corner[0], corner[1]]
                            },
                            axes: self.panel.axes(props.scale),
                            phase: layer.phase,
                            ripples: &layer.ripples,
                            still: pass.view == View::Rest,
                            picture: self.textures.get(&layer.id).map(|t| t.name),
                        },
                    );
                    // Wallpapers leave blending on, and draw from meshes of their own.
                    state.blending = true;
                    gl.use_vertex_buffer(self.vertex_buffer);
                    drawn += 1;
                    state.program = 0;
                    continue;
                }
            };
            if used != state.program {
                state.program = used;
                gl.use_program(used);
            }
            if blend != state.blending {
                state.blending = blend;
                if blend {
                    gl.enable(GL_BLEND);
                } else {
                    gl.disable(GL_BLEND);
                }
            }
            if let Some(fade) = &fade {
                slot.set(gl, fade);
            }
            match layer.content {
                Content::Image => {
                    gl.bind_texture(self.textures[&layer.id].name);
                    gl.uniform2f(set.image.texel, 1.0 / width, 1.0 / height);
                    gl.uniform1f(set.image.opacity, opacity);
                }
                // The producer's buffer, sampled where it is: the layer's rectangle shows all of it, whatever its size.
                Content::Live(live) => {
                    let picture = self.live.texture(live.source);
                    gl.bind_texture(picture.expect("checked before"));
                    gl.uniform2f(set.image.texel, 1.0 / width, 1.0 / height);
                    gl.uniform1f(set.image.opacity, opacity);
                }
                Content::Mesh(_) => {
                    let picture = self.meshes.texture(layer.id);
                    gl.bind_texture(picture.expect("checked before"));
                    gl.uniform2f(set.image.texel, 1.0 / width, 1.0 / height);
                    gl.uniform1f(set.image.opacity, opacity);
                }
                Content::Shape(shape) => {
                    self.set_shape(set, &shape, props, scale, width, height);
                }
                Content::Glyphs(style) => {
                    let (texture, atlas_width, atlas_height) =
                        self.atlases.texture(style.atlas).expect("checked before");
                    gl.bind_texture(texture);
                    gl.uniform2f(set.glyphs.texel, 1.0 / atlas_width, 1.0 / atlas_height);
                }
                Content::Glass(_) | Content::Wallpaper(_) => {}
            }
            match job.draw {
                Vertices::Quad(first) => gl.draw_triangle_strip(first, 4),
                Vertices::Glyphs(spans) => {
                    let Content::Glyphs(style) = layer.content else {
                        unreachable!("a glyph run's vertices are a glyph layer's");
                    };
                    // One draw per ink, each in its own colour: premultiplied, and so is what blending expects, and the
                    // layer's opacity scales all four. A transparent ink draws nothing, but its quads still take their
                    // place among the vertices.
                    let mut first = spans.first;
                    for (count, palette) in spans.counts.into_iter().zip(style.palette) {
                        if count > 0 && palette != Rgba(0, 0, 0, 0) {
                            let c = colour(palette);
                            gl.uniform4f(
                                set.glyphs.colour,
                                c[0] * opacity,
                                c[1] * opacity,
                                c[2] * opacity,
                                c[3] * opacity,
                            );
                            gl.draw_triangles(first, count);
                        }
                        first += count;
                    }
                }
                Vertices::Glass(_) | Vertices::Wallpaper => {
                    unreachable!("glass and wallpapers are drawn above")
                }
            }
            drawn += 1;
            if let Some(fills) = &self.fills {
                let clip = match (scissor, pass.region) {
                    (Some(clip), Some(region)) => clip.intersect(region),
                    (clip, region) => clip.or(region),
                };
                let drawn_over = match (pix_rect_of(job.extent), clip) {
                    (Some(rect), Some(clip)) => rect.intersect(clip),
                    (rect, None) => rect,
                    (None, _) => None,
                };
                if let Some(rect) = drawn_over {
                    fills.borrow_mut().push(crate::profile::Fill {
                        id: layer.id,
                        kind: match layer.content {
                            Content::Image => "picture",
                            Content::Glyphs(_) => "glyphs",
                            Content::Shape(_) => "shape",
                            Content::Mesh(_) => "mesh",
                            Content::Live(_) => "live",
                            Content::Glass(_) | Content::Wallpaper(_) => "other",
                        },
                        size: (layer.width, layer.height),
                        px: u64::from(rect.w) * u64::from(rect.h),
                    });
                }
            }
        }
        // A picture of what is below a run of surfaces has their shadows in it, drawn last: on top of everything below.
        self.cast(layers, jobs, pass.cast_after, pass, &mut state);
        drawn
    }

    /// Sets the uniforms of the program `shape_kind` chose for `shape` from `set`, drawn `scale` output pixels to a layer
    /// pixel.
    fn set_shape(
        &self,
        set: &ProgramSet,
        shape: &Shape,
        props: &Props,
        scale: f32,
        width: f32,
        height: f32,
    ) {
        let gl = self.gl;
        let (from, to, gradient) = match shape.fill {
            Fill::Solid(c) => (colour(c), colour(c), [0.0; 3]),
            Fill::Linear {
                from,
                to,
                angle_deg,
            } => (
                colour(from),
                colour(to),
                gradient_uniform(angle_deg, width, height),
            ),
        };
        let opacity = props.opacity;
        match shape_kind(shape, props) {
            ShapeKind::Solid => {
                let p = &set.solid;
                gl.uniform4f(
                    p.from,
                    from[0] * opacity,
                    from[1] * opacity,
                    from[2] * opacity,
                    from[3] * opacity,
                );
            }
            ShapeKind::Gradient => {
                let p = &set.gradient;
                gl.uniform2f(p.size, width, height);
                gl.uniform3f(p.gradient, gradient[0], gradient[1], gradient[2]);
                gl.uniform4f(
                    p.from,
                    from[0] * opacity,
                    from[1] * opacity,
                    from[2] * opacity,
                    from[3] * opacity,
                );
                gl.uniform4f(
                    p.delta,
                    (to[0] - from[0]) * opacity,
                    (to[1] - from[1]) * opacity,
                    (to[2] - from[2]) * opacity,
                    (to[3] - from[3]) * opacity,
                );
            }
            ShapeKind::Rounded => {
                let p = &set.rounded;
                gl.uniform2f(p.size, width, height);
                gl.uniform3f(p.gradient, gradient[0], gradient[1], gradient[2]);
                gl.uniform4f(p.from, from[0], from[1], from[2], from[3]);
                gl.uniform4f(p.to, to[0], to[1], to[2], to[3]);
                let border = colour(shape.border);
                gl.uniform4f(p.border, border[0], border[1], border[2], border[3]);
                // A radius past half the shorter side would make the distance field fold over itself.
                gl.uniform1f(p.radius, shape.corner_radius.min(width.min(height) / 2.0));
                gl.uniform1f(p.border_width, shape.border_width);
                gl.uniform1f(p.scale, scale);
                gl.uniform1f(p.pixel, 1.0 / scale);
                gl.uniform1f(p.opacity, opacity);
            }
        }
    }
}

/// Copies the changed part of an image layer to its texture. Returns the pixels copied.
fn upload(gl: &Gles, texture: &mut Texture) -> u64 {
    let Some(rect) = texture.dirty else {
        return 0;
    };
    // The texture exists from here on, whatever the file holds: a layer that is drawn has one.
    let name = texture.made(gl);
    let Some(pixels) = texture.file.pixel(rect.x, rect.y) else {
        // The app cut the file short. Keep showing what the texture has; the damage stays pending. A texture that has had no
        // pixels yet has what the driver's memory held, and shows nothing instead.
        if !texture.defined {
            gl.bind_texture(name);
            gl.blank(texture.width, texture.height);
            texture.defined = true;
        }
        return 0;
    };
    gl.bind_texture(name);
    // SAFETY: `rect` was clamped to the layer when the damage was accepted, `pixel` gave the address of its first
    // pixel inside a mapping of `width * height` pixels, and rows in the file are `width` pixels apart.
    unsafe { gl.upload(rect.x, rect.y, rect.w, rect.h, texture.width, pixels) };
    if let Some(opaque) = texture.opaque.as_mut() {
        // Every pixel is opaque after this copy if what it copied is, and the rest (if there is a rest) was.
        // SAFETY: the same mapping `gl.upload` just read, row by row.
        let all = unsafe { alpha_opaque(pixels, rect.w, rect.h, texture.width) };
        let whole = rect == PixRect::full(texture.width, texture.height);
        *opaque = all && (whole || *opaque);
    }
    texture.defined |= rect == PixRect::full(texture.width, texture.height);
    texture.dirty = None;
    // What was read is on the GPU now. Unless the file is read again and again, its pages are not kept mapped for the next change:
    // a wallpaper's 1.8 MB (as many bytes of the compositor's resident size as of the texture) or a screen's pictures are read
    // once, and a change reads (and so maps) only the part it damaged.
    let quiet = texture.uploaded.is_none_or(|at| at.elapsed() >= QUIET);
    texture.uploaded = Some(Instant::now());
    if quiet {
        texture.file.release();
    }
    u64::from(rect.w) * u64::from(rect.h)
}

/// Whether every pixel of a `w` by `h` block of RGBA pixels at `first`, in rows `stride` pixels apart, has alpha 255.
///
/// # Safety
/// `first` must point at the first pixel of a mapping that holds `h` rows of `stride` pixels, the block `w` wide within them.
unsafe fn alpha_opaque(first: *const u8, w: u32, h: u32, stride: u32) -> bool {
    let (w, h, stride) = (w as usize, h as usize, stride as usize);
    (0..h).all(|row| {
        // SAFETY: row `row` of the block lies inside the mapping, by the contract above.
        let pixels = unsafe { std::slice::from_raw_parts(first.add(row * stride * 4), w * 4) };
        pixels.chunks_exact(4).all(|px| px[3] == 255)
    })
}

/// [`upload`] for a frame that is being drawn: the pixels and the time it took go into the frame's report.
fn upload_timed(gl: &Gles, texture: &mut Texture, report: &mut DrawReport) {
    if texture.dirty.is_none() {
        return;
    }
    let began = Instant::now();
    report.uploaded_pixels += upload(gl, texture);
    report.upload_ns += began.elapsed().as_nanos() as u64;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgl::{drawn, rig};

    fn shape(radius: f32, border: f32, fill: Fill) -> Shape {
        Shape {
            corner_radius: radius,
            fill,
            border_width: border,
            border: Rgba(0, 0, 0, 0),
        }
    }

    const SOLID: Fill = Fill::Solid(Rgba(1, 2, 3, 255));
    const GRADIENT: Fill = Fill::Linear {
        from: Rgba(0, 0, 0, 255),
        to: Rgba(9, 9, 9, 255),
        angle_deg: 35.0,
    };

    #[test]
    fn warming_up_raises_no_error_leaves_no_layer_behind_and_changes_nothing_that_is_drawn_after() {
        use compositor_protocol::{Backdrop, Glass, Rgba, Wallpaper, WallpaperKind};
        let scene = || {
            let glass = Glass {
                corner_radius: 20.0,
                tint: Rgba(30, 28, 44, 150),
                edge: Rgba(60, 60, 80, 80),
                edge_width: 1.0,
                highlight: Rgba(0, 0, 0, 0),
                sheen: None,
                shadows: [None, None],
                depth: None,
                backdrop: Backdrop::Live {
                    radius: 20.0,
                    downsample: 4,
                },
                saturation: 1.0,
                refraction: 0.0,
            };
            vec![
                drawn(
                    1,
                    (960, 480),
                    Content::Wallpaper(Wallpaper {
                        kind: WallpaperKind::Starfield,
                        speed: 0.0,
                        intensity: 0.5,
                        fps: 0,
                    }),
                    Props::at(0.0, 0.0),
                ),
                drawn(
                    2,
                    (300, 160),
                    Content::Glass(glass),
                    Props::at(100.0, 100.0),
                ),
                drawn(
                    3,
                    (200, 60),
                    Content::Shape(shape(12.0, 1.0, SOLID)),
                    Props::at(500.0, 300.0),
                ),
            ]
        };
        let Some(mut cold) = rig() else { return };
        let expected = cold.draw(&scene());
        drop(cold);
        let Some(mut warmed) = rig() else { return };
        assert_eq!(
            warmed.renderer.warm_up(),
            None,
            "the warm-up's frame is clean GL"
        );
        // The textures of the pictures it made went to the graveyard (freed when nothing waits, in `tidy`); with the
        // shared-memory directory present there are some, and after the tidy nothing of it is left.
        warmed.tidy_all();
        assert!(
            warmed.renderer.textures.is_empty() && warmed.renderer.graveyard.is_empty(),
            "its pictures are gone"
        );
        let shown = warmed.draw(&scene());
        assert_eq!(shown.first_difference(&expected, 0), None);
    }

    #[test]
    fn plain_rectangles_on_whole_pixels_take_the_cheap_programs() {
        let at = Props::at(12.0, 40.0);
        assert_eq!(shape_kind(&shape(0.0, 0.0, SOLID), &at), ShapeKind::Solid);
        assert_eq!(
            shape_kind(&shape(0.0, 0.0, GRADIENT), &at),
            ShapeKind::Gradient
        );
    }

    #[test]
    fn rounded_corners_borders_and_off_pixel_edges_need_the_anti_aliasing_program() {
        let at = Props::at(12.0, 40.0);
        assert_eq!(shape_kind(&shape(8.0, 0.0, SOLID), &at), ShapeKind::Rounded);
        assert_eq!(shape_kind(&shape(0.0, 1.0, SOLID), &at), ShapeKind::Rounded);
        assert_eq!(
            shape_kind(&shape(0.0, 0.0, SOLID), &Props::at(12.5, 40.0)),
            ShapeKind::Rounded,
            "half a pixel across"
        );
        assert_eq!(
            shape_kind(&shape(0.0, 0.0, GRADIENT), &Props::at(12.0, 40.25)),
            ShapeKind::Rounded
        );
        let scaled = Props { scale: 1.5, ..at };
        assert_eq!(
            shape_kind(&shape(0.0, 0.0, SOLID), &scaled),
            ShapeKind::Rounded
        );
    }

    #[test]
    fn blending_is_only_switched_off_for_shapes_that_cover_what_is_under_them() {
        assert!(covers(&shape(0.0, 0.0, SOLID), 1.0));
        assert!(covers(&shape(0.0, 0.0, GRADIENT), 1.0));
        assert!(!covers(&shape(0.0, 0.0, SOLID), 0.99));
        assert!(!covers(
            &shape(0.0, 0.0, Fill::Solid(Rgba(1, 2, 3, 254))),
            1.0
        ));
        let half = Fill::Linear {
            from: Rgba(0, 0, 0, 255),
            to: Rgba(0, 0, 0, 100),
            angle_deg: 0.0,
        };
        assert!(!covers(&shape(0.0, 0.0, half), 1.0));
    }

    /// The gradient parameter the vertex shader computes for a point, from the uniform.
    fn along(g: [f32; 3], size: (f32, f32), p: (f32, f32)) -> f32 {
        ((p.0 - size.0 / 2.0) * g[0] + (p.1 - size.1 / 2.0) * g[1]) * g[2] + 0.5
    }

    #[test]
    fn a_gradient_runs_from_one_corner_of_the_layer_to_the_opposite_one() {
        let size = (960.0, 480.0);
        for angle in [0.0, 35.0, 90.0, 135.0, 200.0, 270.0] {
            let g = gradient_uniform(angle, size.0, size.1);
            let corners = [
                along(g, size, (0.0, 0.0)),
                along(g, size, (size.0, 0.0)),
                along(g, size, (0.0, size.1)),
                along(g, size, (size.0, size.1)),
            ];
            let low = corners.iter().copied().fold(f32::MAX, f32::min);
            let high = corners.iter().copied().fold(f32::MIN, f32::max);
            assert!(
                low.abs() < 1e-4 && (high - 1.0).abs() < 1e-4,
                "{angle}: {corners:?}"
            );
        }
    }

    #[test]
    fn gradient_angles_follow_the_protocols_convention() {
        let size = (200.0, 100.0);
        // 0 degrees: left to right. 90: top to bottom.
        let left_to_right = gradient_uniform(0.0, size.0, size.1);
        assert!(along(left_to_right, size, (0.0, 50.0)) < 1e-6);
        assert!((along(left_to_right, size, (200.0, 50.0)) - 1.0).abs() < 1e-6);
        assert!((along(left_to_right, size, (100.0, 0.0)) - 0.5).abs() < 1e-6);
        let top_to_bottom = gradient_uniform(90.0, size.0, size.1);
        assert!(along(top_to_bottom, size, (100.0, 0.0)) < 1e-5);
        assert!((along(top_to_bottom, size, (100.0, 100.0)) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_shrunken_layer_is_padded_more_but_never_by_much() {
        assert_eq!(pad(1.0), 1.0);
        assert_eq!(pad(2.0), 0.5);
        assert_eq!(pad(0.01), MAX_PAD);
    }

    fn red_picture(size: (u32, u32)) -> Vec<u8> {
        [255, 0, 0, 255].repeat(size.0 as usize * size.1 as usize)
    }

    /// Layers `ids` as a screen that has been showing them for a while has them: made, and their textures made and filled by the
    /// housekeeping between frames (of which this is the first, drawing none of them).
    fn pictures(rig: &mut crate::testgl::Rig, ids: std::ops::RangeInclusive<u32>) {
        for id in ids {
            rig.set_pixels(id, (8, 8), &red_picture((8, 8)));
        }
        rig.draw(&[]);
        rig.tidy_all();
    }

    #[test]
    fn the_textures_of_deleted_layers_are_freed_when_nothing_waits_and_not_when_they_are_deleted() {
        let Some(mut rig) = rig() else { return };
        pictures(&mut rig, 1..=3);
        for id in [1, 2] {
            rig.renderer.apply(&Op::Delete { id: LayerId(id) });
        }
        assert_eq!(rig.renderer.graveyard.len(), 2, "not freed yet");
        let done = rig.renderer.tidy(Duration::from_secs(1));
        assert_eq!((done.freed, done.more), (2, false));
        assert!(rig.renderer.graveyard.is_empty());
        assert_eq!(rig.renderer.gles().take_error(), None);
        // The layer that stayed still draws.
        let shot = rig.draw(&[drawn(3, (8, 8), Content::Image, Props::at(0.0, 0.0))]);
        assert_eq!(shot.pixel(4, 4), [255, 0, 0, 255]);
    }

    #[test]
    fn tidying_always_frees_something_however_small_the_budget_and_says_when_there_is_more() {
        let Some(mut rig) = rig() else { return };
        pictures(&mut rig, 1..=5);
        for id in 1..=5 {
            rig.renderer.apply(&Op::Delete { id: LayerId(id) });
        }
        let mut calls = 0;
        loop {
            let done = rig.renderer.tidy(Duration::ZERO);
            calls += 1;
            assert_eq!(done.freed, 1, "one at a time with no time to spare");
            if !done.more {
                break;
            }
        }
        assert_eq!(calls, 5);
        assert!(rig.renderer.graveyard.is_empty());
    }

    #[test]
    fn a_picture_that_is_hidden_is_uploaded_ahead_and_the_frame_that_shows_it_uploads_nothing() {
        let Some(mut rig) = rig() else { return };
        rig.set_pixels(1, (100, 100), &red_picture((100, 100)));
        // A frame that does not draw it.
        rig.draw(&[]);
        assert_eq!(rig.report.uploaded_pixels, 0);
        let done = rig.renderer.tidy(Duration::from_secs(1));
        assert_eq!((done.uploaded_pixels, done.more), (10_000, false));
        // The frame that shows it finds it ready, and it is the right picture.
        let shot = rig.draw(&[drawn(1, (100, 100), Content::Image, Props::at(0.0, 0.0))]);
        assert_eq!(rig.report.uploaded_pixels, 0);
        assert_eq!(shot.pixel(50, 50), [255, 0, 0, 255]);
        assert_eq!(rig.renderer.tidy(Duration::from_secs(1)), Tidied::default());
    }

    /// A screen is mounted under a curtain (the lab's shell screens, the product's hand-off): the frame that draws the curtain places
    /// every picture of the screen, and used to copy each of them (the shell's mount: 2 Mpx, 13 to 19 ms on the Show) for pixels
    /// the curtain covers. A layer nothing looks at is no part of the frame: the housekeeping copies it between frames, and the
    /// frame that takes the curtain away finds it ready.
    #[test]
    fn a_picture_under_an_opaque_layer_is_not_copied_by_the_frame_that_covers_it_but_between_frames()
     {
        let Some(mut rig) = rig() else { return };
        for id in 1..=3 {
            rig.set_pixels(id, (100, 100), &red_picture((100, 100)));
        }
        let picture = |id: u32, x: f32| drawn(id, (100, 100), Content::Image, Props::at(x, 100.0));
        let curtain = drawn(
            9,
            (960, 480),
            Content::Shape(shape(0.0, 0.0, Fill::Solid(Rgba(0, 0, 255, 255)))),
            Props::at(0.0, 0.0),
        );
        let shot = rig.draw(&[
            picture(1, 0.0),
            picture(2, 200.0),
            picture(3, 400.0),
            curtain,
        ]);
        assert_eq!(rig.report.culled, 3, "the curtain covers all three");
        assert_eq!(
            rig.report.uploaded_pixels, 0,
            "a frame does not copy what it does not draw"
        );
        assert_eq!(shot.pixel(50, 150), [0, 0, 255, 255]);
        let done = rig.renderer.tidy(Duration::from_secs(1));
        assert_eq!(
            (done.uploaded_pixels, done.more),
            (30_000, false),
            "the housekeeping copies them between frames"
        );
        let shot = rig.draw(&[picture(1, 0.0), picture(2, 200.0), picture(3, 400.0)]);
        assert_eq!(
            rig.report.uploaded_pixels, 0,
            "the frame that shows them finds them ready"
        );
        for x in [50, 250, 450] {
            assert_eq!(shot.pixel(x, 150), [255, 0, 0, 255], "at {x}");
        }
    }

    /// What no one sees is not copied, but what a glass surface blurs is looked at, whatever covers it in the frame: a picture that
    /// an opaque layer above a live glass surface covers entirely is in the blur at the surface's edge, and is copied by the frame.
    #[test]
    fn a_picture_behind_live_glass_is_copied_by_the_frame_though_an_opaque_layer_above_covers_it() {
        let Some(mut rig) = rig() else { return };
        rig.set_pixels(1, (100, 100), &red_picture((100, 100)));
        let glass = glass_over(
            Backdrop::Live {
                radius: 30.0,
                downsample: 8,
            },
            Rgba(20, 16, 30, 110),
            false,
        );
        let cover = Content::Shape(shape(0.0, 0.0, Fill::Solid(Rgba(0, 0, 255, 255))));
        rig.draw(&[
            drawn(1, (100, 100), Content::Image, Props::at(100.0, 100.0)),
            drawn(2, (400, 100), glass, Props::at(0.0, 100.0)),
            drawn(3, (100, 100), cover, Props::at(100.0, 100.0)),
        ]);
        assert_eq!(rig.report.uploaded_pixels, 10_000);
    }

    #[test]
    fn a_picture_makes_no_texture_when_it_is_made_and_the_frame_or_the_housekeeping_that_wants_it_makes_it()
     {
        let Some(mut rig) = rig() else { return };
        rig.set_pixels(1, (100, 100), &red_picture((100, 100)));
        rig.set_pixels(2, (100, 100), &red_picture((100, 100)));
        // Nothing has been asked of the driver for either: a screen's commit holds dozens of layers that are not shown.
        let made =
            |rig: &crate::testgl::Rig, id: u32| rig.renderer.textures[&LayerId(id)].name != 0;
        assert!(!made(&rig, 1) && !made(&rig, 2));
        // A frame that draws the first makes its texture and fills it, on its own critical path, as it always did.
        let shot = rig.draw(&[drawn(1, (100, 100), Content::Image, Props::at(0.0, 0.0))]);
        assert_eq!(rig.report.uploaded_pixels, 10_000);
        assert_eq!(shot.pixel(50, 50), [255, 0, 0, 255]);
        assert!(made(&rig, 1) && !made(&rig, 2));
        // The housekeeping makes the second one's, and fills it, between frames.
        let done = rig.renderer.tidy(Duration::from_secs(1));
        assert_eq!((done.uploaded_pixels, done.more), (10_000, false));
        assert!(made(&rig, 2));
        assert_eq!(rig.renderer.gles().take_error(), None);
        // And the frame that shows it has nothing left to make or upload.
        let shot = rig.draw(&[drawn(2, (100, 100), Content::Image, Props::at(0.0, 0.0))]);
        assert_eq!(rig.report.uploaded_pixels, 0);
        assert_eq!(shot.pixel(50, 50), [255, 0, 0, 255]);
    }

    #[test]
    fn a_picture_that_goes_before_anything_wanted_it_costs_no_texture_and_leaves_none_to_free() {
        let Some(mut rig) = rig() else { return };
        rig.set_pixels(1, (100, 100), &red_picture((100, 100)));
        rig.renderer.apply(&Op::Delete { id: LayerId(1) });
        assert!(rig.renderer.textures.is_empty());
        assert!(
            rig.renderer.graveyard.is_empty(),
            "no texture was ever made"
        );
        assert_eq!(rig.renderer.tidy(Duration::from_secs(1)), Tidied::default());
    }

    #[test]
    fn a_picture_the_last_frame_drew_is_left_alone_for_the_frame_that_draws_it_next() {
        let Some(mut rig) = rig() else { return };
        rig.set_pixels(1, (100, 100), &red_picture((100, 100)));
        let layer = drawn(1, (100, 100), Content::Image, Props::at(0.0, 0.0));
        rig.draw(std::slice::from_ref(&layer));
        assert_eq!(rig.report.uploaded_pixels, 10_000);
        // The app changes it. The GPU may still be reading it for the frame just drawn, so tidying does not touch it;
        // the next frame that draws it brings it up to date.
        rig.set_pixels(1, (100, 100), &[0, 0, 255, 255].repeat(10_000));
        assert_eq!(rig.renderer.tidy(Duration::from_secs(1)), Tidied::default());
        let shot = rig.draw(std::slice::from_ref(&layer));
        assert_eq!(rig.report.uploaded_pixels, 10_000);
        assert_eq!(shot.pixel(50, 50), [0, 0, 255, 255]);
    }

    #[test]
    fn a_picture_a_frame_did_not_draw_is_tidied_once_and_a_picture_that_is_up_to_date_never() {
        let Some(mut rig) = rig() else { return };
        rig.set_pixels(1, (10, 10), &red_picture((10, 10)));
        rig.set_pixels(2, (10, 10), &red_picture((10, 10)));
        // Frame 1 draws layer 1 only; layer 2 was never drawn.
        rig.draw(&[drawn(1, (10, 10), Content::Image, Props::at(0.0, 0.0))]);
        assert_eq!(
            rig.renderer.tidy(Duration::from_secs(1)).uploaded_pixels,
            100
        );
        assert_eq!(rig.renderer.tidy(Duration::from_secs(1)).uploaded_pixels, 0);
    }

    #[test]
    fn a_shape_lands_where_the_rotation_puts_it_and_the_rest_stays_black() {
        let Some(mut rig) = rig() else { return };
        let red = Content::Shape(shape(0.0, 0.0, Fill::Solid(Rgba(255, 0, 0, 255))));
        let shot = rig.draw(&[drawn(1, (200, 100), red, Props::at(100.0, 50.0))]);
        assert_eq!(shot.pixel(150, 80), [255, 0, 0, 255]);
        assert_eq!(shot.pixel(299, 149), [255, 0, 0, 255]);
        for outside in [
            (99, 80),
            (300, 80),
            (150, 49),
            (150, 150),
            (0, 0),
            (959, 479),
        ] {
            assert_eq!(
                shot.pixel(outside.0, outside.1),
                [0, 0, 0, 255],
                "{outside:?}"
            );
        }
    }

    /// Share of horizontally neighbouring pixels in `rows` whose colours differ. Rounded to eight bits, a gradient
    /// changes level once in tens of pixels and the rows change at the same places (a band); dithered, the two levels
    /// it lies between mix along the row.
    fn share_changing_across(shot: &crate::testgl::Shot, rows: std::ops::Range<u32>) -> f32 {
        let pairs = rows.len() as u32 * (shot.width() - 1);
        let changing = rows
            .flat_map(|y| (1..shot.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| shot.pixel(x, y) != shot.pixel(x - 1, y))
            .count();
        changing as f32 / pairs as f32
    }

    #[test]
    fn a_screen_wide_gradient_is_dithered_and_does_not_break_into_bands() {
        let Some(mut rig) = rig() else { return };
        // The home screen's backdrop: two dark colours a few levels apart across the whole screen at 35 degrees.
        let fill = Fill::Linear {
            from: Rgba(38, 35, 58, 255),
            to: Rgba(25, 23, 36, 255),
            angle_deg: 35.0,
        };
        for (radius, program) in [(0.0, "flat"), (16.0, "rounded")] {
            let backdrop = Content::Shape(shape(radius, 0.0, fill));
            let shot = rig.draw(&[drawn(1, (960, 480), backdrop, Props::at(0.0, 0.0))]);
            let across = share_changing_across(&shot, 200..240);
            assert!(across > 0.2, "{program} gradient: {across}");
        }
    }

    /// The share of pixel pairs side by side in `rows` between columns `cols` whose colours differ.
    fn share_changing_between(
        shot: &crate::testgl::Shot,
        rows: std::ops::Range<u32>,
        cols: std::ops::Range<u32>,
    ) -> f32 {
        let pairs = rows.len() * (cols.len() - 1);
        let changing = rows
            .flat_map(|y| (cols.start + 1..cols.end).map(move |x| (x, y)))
            .filter(|&(x, y)| shot.pixel(x, y) != shot.pixel(x - 1, y))
            .count();
        changing as f32 / pairs as f32
    }

    fn glass_over(backdrop: Backdrop, tint: Rgba, sheen: bool) -> Content {
        Content::Glass(compositor_protocol::Glass {
            corner_radius: 24.0,
            tint,
            edge: Rgba(0, 0, 0, 0),
            edge_width: 0.0,
            highlight: Rgba(0, 0, 0, 0),
            sheen: sheen.then_some(compositor_protocol::Sheen {
                peak: 0.16,
                floor: 0.05,
                start: 0.38,
                end: 0.6,
            }),
            shadows: [None, None],
            depth: None,
            backdrop,
            saturation: 1.0,
            refraction: 0.0,
        })
    }

    #[test]
    fn glass_over_a_smooth_gradient_is_dithered_and_does_not_break_into_bands() {
        let Some(mut rig) = rig() else { return };
        // The home screen's backdrop again, and a surface over it whose interior is that blurred, under a tint and a sheen: a
        // ramp of a few levels over hundreds of pixels, which eight bits turn into bands unless what is written is dithered.
        let ground = Content::Shape(shape(
            0.0,
            0.0,
            Fill::Linear {
                from: Rgba(38, 35, 58, 255),
                to: Rgba(25, 23, 36, 255),
                angle_deg: 35.0,
            },
        ));
        for (name, backdrop) in [
            (
                "live",
                Backdrop::Live {
                    radius: 30.0,
                    downsample: 8,
                },
            ),
            ("cached", Backdrop::Cached { radius: 30.0 }),
        ] {
            let card = glass_over(backdrop, Rgba(20, 16, 30, 110), true);
            let shot = rig.draw(&[
                drawn(1, (960, 480), ground, Props::at(0.0, 0.0)),
                drawn(2, (600, 300), card, Props::at(180.0, 90.0)),
            ]);
            let inside = share_changing_between(&shot, 200..240, 300..660);
            assert!(inside > 0.2, "{name} glass: {inside}");
        }
    }

    #[test]
    fn glass_of_one_solid_colour_stays_its_colour_to_within_a_level() {
        let Some(mut rig) = rig() else { return };
        // Nothing to blur, no sheen, an opaque tint: the interior is one colour. Noise of under half a level does not move a
        // colour that eight bits hold exactly.
        let tint = Rgba(30, 40, 60, 255);
        let card = glass_over(Backdrop::None, tint, false);
        let shot = rig.draw(&[drawn(2, (400, 200), card, Props::at(100.0, 100.0))]);
        for (x, y) in [(150, 150), (300, 200), (450, 250), (110, 190)] {
            let p = shot.pixel(x, y);
            for (got, want) in p.iter().zip([30u8, 40, 60, 255]) {
                assert!(got.abs_diff(want) <= 1, "({x}, {y}): {p:?}");
            }
        }
    }

    #[test]
    fn a_solid_colour_is_not_dithered_and_stays_exactly_its_colour() {
        let Some(mut rig) = rig() else { return };
        let grey = Rgba(100, 100, 100, 255);
        for radius in [0.0, 16.0] {
            let solid = Content::Shape(shape(radius, 0.0, Fill::Solid(grey)));
            let shot = rig.draw(&[drawn(1, (400, 200), solid, Props::at(100.0, 100.0))]);
            for y in 130..270 {
                for x in 130..470 {
                    assert_eq!(
                        shot.pixel(x, y),
                        [100, 100, 100, 255],
                        "radius {radius} at {x},{y}"
                    );
                }
            }
        }
    }

    fn picture_of(refresh: Refresh, upto: usize, users: &[usize]) -> backdrop::Backdrop {
        backdrop::Backdrop {
            refresh,
            radius: 30.0,
            downsample: 4,
            upto,
            area: [0, 0, 480, 960],
            users: users.to_vec(),
        }
    }

    #[test]
    fn a_layer_is_looked_at_when_the_frame_shows_it_or_a_picture_behind_glass_that_shows_has_it() {
        let seen = Sight::Seen([0, 0, 10, 10]);
        // Four layers, all with jobs: a picture, a wallpaper, a glass surface (2) and a page (3) over it all.
        let job_of = [Some(0), Some(1), Some(2), Some(3)];
        let live = [picture_of(Refresh::Live, 2, &[2])];
        // The page covers everything below it and nothing is behind glass: only the page is looked at.
        let hidden = [Sight::Hidden, Sight::Hidden, Sight::Hidden, seen];
        assert_eq!(
            looked_at(&job_of, &hidden, &[], &[]),
            [false, false, false, true]
        );
        // A surface that is itself covered makes no picture, live or not.
        assert_eq!(
            looked_at(&job_of, &hidden, &live, &[false]),
            [false, false, false, true]
        );
        // One that shows makes a live picture of the layers below it, which look at the layers the page covers.
        let glass_shows = [Sight::Hidden, Sight::Hidden, seen, seen];
        assert_eq!(
            looked_at(&job_of, &glass_shows, &live, &[false]),
            [true, true, true, true]
        );
        // A cached picture is made of them only when it is out of date; a picture that stands is nobody looking.
        let cached = [picture_of(Refresh::Cached, 2, &[2])];
        assert_eq!(
            looked_at(&job_of, &glass_shows, &cached, &[false]),
            [false, false, true, true]
        );
        assert_eq!(
            looked_at(&job_of, &glass_shows, &cached, &[true]),
            [true, true, true, true]
        );
        // Only what is below the surface is in its picture.
        let above = [picture_of(Refresh::Live, 1, &[1])];
        let wallpaper_and_glass = [Sight::Hidden, seen, Sight::Hidden, Sight::Hidden];
        assert_eq!(
            looked_at(&job_of, &wallpaper_and_glass, &above, &[false]),
            [true, true, false, false]
        );
        // A layer with no job is nowhere on the panel and has no pixels for anyone to look at, not even a picture behind glass.
        let placed = [Some(0), None, Some(1)];
        let sights = [seen, seen];
        let behind = [picture_of(Refresh::Live, 2, &[2])];
        assert_eq!(
            looked_at(&placed, &sights, &behind, &[false]),
            [true, false, true]
        );
    }
}

#[cfg(test)]
mod calm_tests {
    use super::calm_at;

    #[test]
    fn refraction_fades_back_over_the_fade_and_with_no_fade_returns_the_frame_after_motion() {
        let ms = 1_000_000;
        assert_eq!(calm_at(1000 * ms, None, 200 * ms), 1.0);
        assert_eq!(calm_at(1000 * ms, Some(1000 * ms), 200 * ms), 0.0);
        assert_eq!(calm_at(1100 * ms, Some(1000 * ms), 200 * ms), 0.5);
        assert_eq!(calm_at(1300 * ms, Some(1000 * ms), 200 * ms), 1.0);
        assert_eq!(calm_at(1050 * ms, Some(1000 * ms), 100 * ms), 0.5);
        // No fade: none while it moves (this frame), all of it the frame after.
        assert_eq!(calm_at(1000 * ms, Some(1000 * ms), 0), 0.0);
        assert_eq!(calm_at(1017 * ms, Some(1000 * ms), 0), 1.0);
    }
}
