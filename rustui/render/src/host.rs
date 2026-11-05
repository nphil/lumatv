//! The compositor of one app, in the app's own process: LumaShow's session (which checks the app's requests), scene (which
//! animates layers on a clock) and renderer (which draws them on the GPU), driven by the host's loop instead of the
//! compositor's render and present threads.
//!
//! The app speaks to it as it speaks to LumaShow's compositor: `ui::Scene` over a socket (a socket pair here) with the same
//! requests, the same shared-memory files for pictures, the same events back (`Frame`, `AtlasUploaded`, `BackdropReady`,
//! `Motion`). Nothing of the toolkit knows the compositor is in the same process; the host calls [`Engine::pump`] to read
//! what the app sent, [`Engine::draw`] to draw a frame and [`Engine::frame_done`] to tell the app it was shown.
//!
//! Drawing. The renderer draws the way a LumaShow scan-out page is laid out: panel row 0 is the first row of the target's
//! memory, which is the top of the screen. A window surface is the other way up (GL row 0 is the bottom of the window), so
//! the scene is drawn into a render target of the panel's size and one more pass copies it into the window's framebuffer
//! turned over. [`HostConfig::direct`] skips the copy and draws the scene straight into the window's framebuffer, which then
//! shows upside down: for measuring what the copy costs.
//!
//! Only what changed. With [`HostConfig::pages`] of three or more, frames go into a ring of render targets and each is drawn only
//! where it differs from what it holds: what the frames since it was last drawn into changed, and what this frame changes (16 px
//! tiles, `tiles`). [`HostConfig::reference`] draws every frame whole as well and compares, which is how that is tested.
//!
//! Inside a compositor. With [`HostConfig::external`] the engine draws into the pages of a compositor that is not this crate's
//! (a scan-out ring, with windows drawn in the same page), in the compositor's own GL context ([`Gles::current`]), and the
//! compositor keeps the account of what each page owes, since the page holds more than the engine's layers:
//! [`Engine::plan`] says what a frame changes ([`Engine::changed`]), the compositor adds that to its ring and draws what the page
//! owes, [`Engine::draw_stack`] draws the layers under the windows (stack 0) and, after the compositor has drawn the windows, the
//! layers over them (stack 1, the system layer: the layers whose z is at least [`HostConfig::system_z`]), and [`Engine::finish`]
//! ends the frame. Every call that touches GL saves the compositor's GL state, makes the context the toolkit's, and puts the
//! state back ([`Gles::enter`], [`Gles::leave`]).
//!
//! Stack 0 from a picture. While something of stack 1 is over Home (a panel, Settings, a toast, and the frames after it while the
//! pages owe its place: [`OVER_FRAMES`]), stack 0 comes from a render
//! target of its own, the floor: a page gets stack 0 as one copy of the floor in each area it owes, and what the floor owes there
//! (what stack 0 changed since) is drawn into it first. A panel or a sheet that moves over a Home that stands costs the page its
//! own layers and the copy, not Home's 70 to 90 layers in every pass: on the set a frame's GPU time is about 1.5 ms and 0.085 ms
//! for each layer a pass draws, so a refresh of 120 Hz holds about 80 (round 23, `docs/research/frames-120-result.md`). With
//! nothing over Home, Home is drawn straight into the page as before the floor (its own walks cost more through it, round 27),
//! and the first frame in which Home stands and nothing waits to move brings the floor up to date, so an opening only copies it.
//! With a `system_z` below `i32::MAX` and no `external`, the host draws its own pages that way too, and as the compositor does
//! (`kms::frames`): its ring keeps the account a compositor keeps, a page copies from the page drawn last what it owes from the
//! frames before (while something is over Home), and the frame draws only what it changes, so what the host measures is what the
//! set draws. `LUMATV_FLOOR=0` and `LUMATV_PAGE_COPY=0` switch either off, to compare.

use crate::backdrop::Area;
use crate::clock::now_ns;
use crate::damage::Region;
use crate::geometry::Panel;
use crate::gl::{Gles, RenderTarget};
use crate::idle::{Idle, Process};
use crate::ops::{Op, Transaction};
use crate::programs::Cache;
use crate::renderer::{Draw, DrawReport, Planned, Renderer, Surface};
use crate::scene::{Demand, Drawn, Scene};
use crate::session::{Config, Limits, Session};
use crate::stats::SharedStats;
use crate::tiles::{Ring, TileSet};
use compositor_protocol::{Event, LayerId, PROTOCOL_VERSION, Request, read_frame, write_frame};
use device_profile::Rotation;
use std::collections::HashMap;
use std::ffi::CStr;
use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How long housekeeping (freeing textures, uploading what the next frame will draw) may take in one go.
const TIDY_BUDGET: Duration = Duration::from_millis(2);

pub struct HostConfig {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: f32,
    /// Where the app's picture files live (`ui::SceneOptions::shm_dir`).
    pub shm_dir: PathBuf,
    /// Draw the scene straight into the window's framebuffer (it shows upside down), without the copy that turns it over.
    pub direct: bool,
    /// Wait for the GPU to finish each frame before `draw` returns, so that its time is in [`FrameCost::gpu_us`].
    pub gpu_sync: bool,
    /// Run the compositor's idle housekeeping (frees pooled render targets and heap pages after 5 s of quiet).
    pub idle_trim: bool,
    /// Render targets frames go into in turn, each drawn only where it differs from what it holds (0 or 1: every frame in full).
    pub pages: usize,
    /// Also draw every frame whole into another render target and compare it with the one the frame went into: what `draw`
    /// reports as [`FrameCost::mismatch_px`]. For the tests and a host's check; it doubles the GPU's work.
    pub reference: bool,
    /// Copy the page into the window's framebuffer, turned over (the window is the other way up from a scan-out page). A
    /// compositor draws into its own page and needs none; a measurement of the renderer leaves it out.
    pub blit: bool,
    /// The engine is inside a compositor's frame ([`Engine::plan`], [`Engine::draw_stack`], [`Engine::finish`]) and makes no pages,
    /// copy or reference of its own: `pages`, `direct`, `reference` and `blit` do nothing.
    pub external: bool,
    /// The layers whose z is at least this are the system layer, drawn over the compositor's windows (stack 1), and the ones below
    /// it stack 0, drawn from the floor. Without `external` the host draws its pages the way a compositor does (`pages` of them,
    /// at least one; not `direct`). `i32::MAX`: there is no such layer, and a host draws every layer into its page as one stack.
    pub system_z: i32,
}

/// What one frame cost the host.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameCost {
    /// The last app commit included in this frame, retained while a later frame is already being rendered.
    pub tag: u64,
    /// Taking in the app's commits, placing and drawing the layers, and the copy into the window: the CPU time to issue the frame.
    pub cpu_us: u32,
    /// The same until the GPU had finished (`HostConfig::gpu_sync`), else 0.
    pub gpu_us: u32,
    /// Of `cpu_us`: placing the layers, planning backdrops, making backdrops, drawing.
    pub place_us: u32,
    pub plan_us: u32,
    pub backdrop_us: u32,
    pub draw_us: u32,
    /// Of `draw_us`: working out what changed and what the frame's page owes.
    pub damage_us: u32,
    pub layers: u32,
    pub culled: u32,
    pub backdrops: u16,
    pub uploaded_bytes: u32,
    pub draw_calls: u32,
    /// What the frame asked of the GL (`gl::Calls`), summed over its passes.
    pub calls: crate::gl::Calls,
    pub gl_error: Option<u32>,
    /// The panel pixels the frame drew, and the passes it drew them in.
    pub region_px: u64,
    pub rects: u16,
    /// The panel pixels the frame changed from the one before.
    pub damage_px: u64,
    /// How many pixels differ between the frame as it was drawn and the same frame drawn whole (`HostConfig::reference`).
    pub mismatch_px: u64,
    /// The panel pixels of the floor drawn again: what stack 0 changed since the floor was last drawn into.
    pub floor_px: u64,
    /// Of `cpu_us`: taking in the app's commits in the frame (none for a commit taken in ahead, [`Engine::take_in_ahead`]).
    pub apply_us: u32,
    /// Of `cpu_us`: saving the compositor's GL state and putting it back round each of the engine's calls (`Engine::scoped`), stepping the scene
    /// to the frame's time and listing its layers, and ending the frame (what it showed is kept, what nothing showed is told to the scene). What
    /// is left of `cpu_us` after these and the phases above is the renderer's own work between them.
    pub scope_us: u32,
    pub step_us: u32,
    pub finish_us: u32,
    /// How many scopes of the context's state the engine's calls took in the frame (each is some eighty calls into the driver).
    pub scopes: u32,
}

/// What reading the app's requests did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pumped {
    /// Commits taken in (a frame is due for them).
    pub commits: u32,
    /// Requests that were not commits (changes staged for the next commit, or acted on at once).
    pub requests: u32,
    /// The app closed its end.
    pub closed: bool,
}

/// A frame-local protected panel area and the two independent layer groups it excludes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exclusion {
    pub area: [i32; 4],
    pub popup_z: (i32, i32),
    pub marker_z: (i32, i32),
}

pub struct Engine {
    renderer: Renderer,
    scene: Scene,
    session: Session,
    stats: Arc<SharedStats>,
    stream: Option<UnixStream>,
    handshake: Option<JoinHandle<io::Result<UnixStream>>>,
    inbox: Vec<u8>,
    outbox: Vec<u8>,
    drawn: Vec<Drawn>,
    shown: Vec<Drawn>,
    pages: Vec<RenderTarget>,
    next_page: usize,
    reference: Option<RenderTarget>,
    mismatch: Option<Mismatch>,
    blit: Option<Blit>,
    copy: bool,
    window: (u32, u32),
    direct: bool,
    gpu_sync: bool,
    /// Frames are cleared to nothing, not to opaque black ([`Engine::set_transparent`]).
    transparent: bool,
    highest_tag: u64,
    taken_tag: u64,
    /// The newest commit's tag the app has been told the frame of (`Event::Frame`).
    told_tag: u64,
    /// Ops taken in since the last frame, applied with the frame (as the render thread applies a wake's batches).
    waiting: Vec<Transaction>,
    idle: Option<Idle>,
    tidy_more: bool,
    uploaded_bytes: u32,
    /// A frame drawn in two stacks, between [`Engine::plan`] and [`Engine::finish`]: into a compositor's pages when the engine is
    /// inside one ([`HostConfig::external`]), or into the host's own ([`HostConfig::system_z`]).
    external: Option<External>,
    /// Stack 0 kept whole (the module's notes), when the layers are drawn in two stacks and the floor is not switched off
    /// (`LUMATV_FLOOR=0`).
    floor: Option<Floor>,
    /// Copies a picture of the panel's size into a target: the floor into a page, and on the host's pages what a page owes from
    /// the frames before from the page drawn last.
    copier: Option<PictureCopy>,
    /// A large commit was taken in ahead of its frame ([`Engine::take_in_ahead`]): the next frame is due for it.
    ahead: bool,
    /// Commits are taken in ahead of their frame, with `LUMATV_AHEAD=1` (round 27 at the set: the frame after OK on "All settings"
    /// still spent 7.7-10.9 ms of the loop with it, so it is not what that frame waits on).
    take_ahead: bool,
    /// The time the frame being drawn spent taking in commits.
    apply_ns: u64,
    /// Frames left in which stack 0 comes from the floor and the pages copy what they owe ([`Engine::over_home`]).
    over_frames: u8,
    /// Home shows in the frame planned (the compositor's: not under an app's picture, [`Engine::set_home_shown`]).
    home_shown: bool,
}

/// A waiting commit of at least this many operations is taken in ahead of its frame ([`Engine::take_in_ahead`]). Settings made at its
/// opening is several hundred (about 100 layers, each made, styled and placed); a frame of a motion is a handful.
const AHEAD_OPS: usize = 64;

/// Frames, counting the last with something over Home, in which the floor and the pages' copies stay in use: the frame that takes
/// a panel away changes its place, and the two other pages of a ring of three still owe it after (with Home standing under it).
const OVER_FRAMES: u8 = 4;

/// A frame drawn in two stacks, between [`Engine::plan`] and [`Engine::finish`].
struct External {
    /// Inside a compositor's frame, in its context. Otherwise the host's own pages are drawn the way a compositor draws them.
    compositor: bool,
    /// What each of the host's pages owes, kept as a compositor keeps its own account (none inside a compositor).
    ring: Option<Ring>,
    /// The host's page the last frame was drawn into, which holds that frame whole.
    last_page: Option<usize>,
    /// When the last frame was drawn on the host's own clock (`draw_stacks`), and the panel's refresh: the compositor's own account of the time
    /// between frames is `kms`'s.
    last_frame_ns: Option<u64>,
    period_ns: u64,
    /// The host's pages also copy what they owe in a calm frame of ambient motion (`AMBIENT_COPY_REFRESHES`), as the compositor's do
    /// (`LUMATV_AMBIENT_COPY=0`: they draw all they owe in it, as before).
    ambient_copy: bool,
    /// The host's pages copy from the last one what they owe from the frames before, as the compositor's do (`LUMATV_PAGE_COPY=0`:
    /// they draw it, as before).
    page_copy: bool,
    /// Scratch for [`Engine::merge_areas`].
    merging: TileSet,
    system_z: i32,
    exclusion: Option<Exclusion>,
    exclusion_dirty: bool,
    standin_groups: Option<((i32, i32), (i32, i32))>,
    /// The z of each layer, from the commits: the scene lists layers in z order, and where the system layer begins is the first
    /// whose z is `system_z` or more.
    z: HashMap<LayerId, i32>,
    planned: Option<Planned>,
    /// Where stack 1 begins in `Engine::drawn`.
    split: usize,
    /// What the draws of the frame added up to.
    report: DrawReport,
    /// How long the engine's own calls took in the frame: what the compositor's time between them is not.
    busy: Duration,
    /// Of `busy`: the GL state saved and put back, the scene stepped, the frame ended (`FrameCost::scope_us`, `step_us`, `finish_us`).
    spent: Spent,
    /// The engine's calls run in a scope of the context's state (`Engine::scoped`): always inside a compositor, and in a host that asks
    /// for it (`LUMATV_SCOPES=1`: a measurement of what the scopes cost).
    scope: bool,
    /// A call that needs the context only when there is something for it to do takes no scope when there is none ([`Engine::scoped_when`]),
    /// unless `LUMATV_LAZY_SCOPES=0` or a call made in the context outside a scope has shown that the engine's account of when it does is wrong.
    lazy: bool,
}

/// Where a frame's engine time went besides the renderer's phases.
#[derive(Clone, Copy, Default)]
struct Spent {
    scope: Duration,
    step: Duration,
    finish: Duration,
    /// How many scopes the frame took.
    scopes: u32,
}

/// Whether applying `op` leaves the GL alone: the ops that only change what the scene is made of, where a layer is or how it moves, a glyph
/// run's list and the engine's own hints. A picture's damage is not one (the frame that shows it copies its pixels), and neither are a layer
/// made or deleted, an atlas, a backdrop, a producer's source or the ideas (a texture, a buffer or a target is made or freed for each).
fn leaves_gl_alone(op: &Op) -> bool {
    matches!(
        op,
        Op::Set { .. }
            | Op::Animate { .. }
            | Op::Restyle { .. }
            | Op::SetGlyphs { .. }
            | Op::Ripple { .. }
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
            | Op::MotionCancel { .. }
            | Op::HoldGpuClock(_)
            | Op::Display(_)
    )
}

impl External {
    fn surface(&self, stack: usize, layers: usize) -> Surface {
        if stack == 0 {
            Surface { stack: 0, from: 0, to: self.split, transparent: false }
        } else {
            Surface { stack: 1, from: self.split, to: layers, transparent: true }
        }
    }
}

/// A frame no motion of the user's drives (a sky, a wallpaper stepping: nothing at every refresh) that comes this many refreshes after the frame
/// before is calm: the pages copy what they owe from the page drawn last in it, though nothing is over Home, and draw only what it changes
/// (`kms::frames`, which counts the same). Frames of Home's walks come a refresh apart and draw what they owe, as round 27 found they must at
/// 120 Hz; a calm frame has the time for a copy and draws a quarter of the layers.
pub const AMBIENT_COPY_REFRESHES: u64 = 3;

/// The most rectangles a compositor's page is drawn in (`damage::MAX_RECTS` of the compositor): past it the page owes their bounds.
const COMPOSITOR_RECTS: usize = 12;

/// Frames planned with stack 0 not drawn (Home under an app's picture) before the floor's memory is given back.
const FLOOR_UNUSED_FRAMES: u32 = 240;

/// A merge of rectangles into covers is taken only where the covers draw at most a quarter more of the panel than the rectangles
/// (in quarters). On the set a pass's pixels cost as well as its layers: in round 26 the dock's tiles and the library's cells merged
/// into one cover of about 1 Mpx took 7.2-7.6 ms of GPU at the median, where round 23 drew the same frames as 8-10 rectangles of
/// 0.4-0.6 Mpx in 4.7 ms, and apps-grid and home-widgets dropped 13 and 24 refreshes at 120 Hz against 3 and 3. A list's rows are
/// bands with a few pixels between them, and stay one cover.
const MERGE_GROWTH_QUARTERS: u64 = 5;

/// `merged` where its covers draw no more than [`MERGE_GROWTH_QUARTERS`] of `region`'s pixels, else `region` as it was.
fn within_growth(merged: Region, region: Region) -> Region {
    if merged.pixels() * 4 <= region.pixels() * MERGE_GROWTH_QUARTERS { merged } else { region }
}

/// Stack 0 kept whole in a render target of its own (the module's notes).
struct Floor {
    /// Made by the first frame that draws stack 0, and given back when stack 0 has not been drawn for a while.
    target: Option<RenderTarget>,
    /// What of the floor is out of date: what stack 0 changed since those tiles were last drawn into it.
    owed: TileSet,
    /// Scratch for a page's areas: the tiles of them Home changed, then those the floor owes.
    here: TileSet,
    /// Frames planned since stack 0 was last drawn.
    unused: u32,
    /// This frame drew Home straight into a page, the floor owing those tiles.
    direct: bool,
    /// Frames drew Home straight into the pages since the floor was last brought up to date: the first frame Home stands in
    /// draws what the floor owes, so that an opening only copies it.
    behind: bool,
    /// The pixels drawn into the floor in the frame being drawn.
    drawn_px: u64,
}

/// Copies a picture of the panel's size into a target: each pixel of the target is the picture's at the same place, read as it
/// is (`texelFetch`, no filter), inside the scissor of each area.
struct PictureCopy {
    program: u32,
    buffer: u32,
}

const COPY_VERTEX: &CStr = c"#version 300 es
layout(location = 0) in vec2 a_panel;
layout(location = 1) in vec2 a_local;
uniform vec2 u_ndc;
void main() {
    gl_Position = vec4(a_panel * u_ndc - 1.0, 0.0, 1.0);
}";

const COPY_FRAGMENT: &CStr = c"#version 300 es
precision highp float;
uniform highp sampler2D u_picture;
out highp vec4 o_color;
void main() {
    o_color = texelFetch(u_picture, ivec2(gl_FragCoord.xy), 0);
}";

impl PictureCopy {
    fn new(gl: &Gles, width: u32, height: u32) -> io::Result<Self> {
        let program = gl.program(COPY_VERTEX, COPY_FRAGMENT)?;
        gl.use_program(program);
        let ndc = gl.uniform_location(program, c"u_ndc");
        let picture = gl.uniform_location(program, c"u_picture");
        gl.uniform2f(ndc, 2.0 / width as f32, 2.0 / height as f32);
        gl.uniform1i(picture, 0);
        let buffer = gl.new_vertex_buffer();
        let (w, h) = (width as f32, height as f32);
        gl.static_vertices(buffer, &[[0.0, 0.0, 0.0, 0.0], [w, 0.0, w, 0.0], [0.0, h, 0.0, h], [w, h, w, h]]);
        Ok(Self { program, buffer })
    }

    /// Copies `picture` into the framebuffer `target` inside each of `areas`, blending off; blending is on after it, as a pass
    /// leaves it.
    fn run(&self, gl: &Gles, picture: &RenderTarget, target: u32, areas: &[Area]) {
        if areas.is_empty() {
            return;
        }
        gl.bind_framebuffer(target);
        gl.viewport(0, 0, picture.width as i32, picture.height as i32);
        gl.disable(crate::gl::GL_BLEND);
        gl.enable(crate::gl::GL_SCISSOR_TEST);
        gl.use_program(self.program);
        gl.use_vertex_buffer(self.buffer);
        gl.active_texture(0);
        gl.bind_texture(picture.texture);
        for area in areas {
            gl.scissor(area[0], area[1], area[2] - area[0], area[3] - area[1]);
            gl.draw_triangle_strip(0, 4);
        }
        gl.disable(crate::gl::GL_SCISSOR_TEST);
        gl.enable(crate::gl::GL_BLEND);
    }
}

/// One pass that copies the scene's render target into the window's framebuffer, row order reversed.
struct Blit {
    program: u32,
    buffer: u32,
    ndc: i32,
    texel: i32,
    image: i32,
}

const BLIT_VERTEX: &CStr = c"#version 300 es
layout(location = 0) in vec2 a_panel;
layout(location = 1) in vec2 a_local;
uniform vec2 u_ndc;
out vec2 v_local;
void main() {
    v_local = a_local;
    gl_Position = vec4(a_panel * u_ndc - 1.0, 0.0, 1.0);
}";

const BLIT_FRAGMENT: &CStr = c"#version 300 es
precision mediump float;
in vec2 v_local;
uniform sampler2D u_image;
uniform vec2 u_texel;
out mediump vec4 o_color;
void main() {
    o_color = texture(u_image, v_local * u_texel);
}";

impl Blit {
    fn new(gl: &Gles, width: u32, height: u32) -> io::Result<Self> {
        let program = gl.program(BLIT_VERTEX, BLIT_FRAGMENT)?;
        gl.use_program(program);
        let ndc = gl.uniform_location(program, c"u_ndc");
        let texel = gl.uniform_location(program, c"u_texel");
        let image = gl.uniform_location(program, c"u_image");
        gl.uniform2f(ndc, 2.0 / width as f32, 2.0 / height as f32);
        gl.uniform2f(texel, 1.0 / width as f32, 1.0 / height as f32);
        gl.uniform1i(image, 0);
        let buffer = gl.new_vertex_buffer();
        let (w, h) = (width as f32, height as f32);
        // Window position (x, h - row) shows the target's pixel (x, row): the top of the screen is the target's row 0.
        gl.static_vertices(buffer, &[[0.0, h, 0.0, 0.0], [w, h, w, 0.0], [0.0, 0.0, 0.0, h], [w, 0.0, w, h]]);
        Ok(Self { program, buffer, ndc, texel, image })
    }

    fn run(&self, gl: &Gles, source: u32, width: u32, height: u32) {
        gl.bind_framebuffer(0);
        gl.viewport(0, 0, width as i32, height as i32);
        gl.disable(crate::gl::GL_SCISSOR_TEST);
        gl.disable(crate::gl::GL_BLEND);
        gl.use_program(self.program);
        gl.use_vertex_buffer(self.buffer);
        gl.active_texture(0);
        gl.bind_texture(source);
        gl.draw_triangle_strip(0, 4);
        let _ = (self.ndc, self.texel, self.image);
    }
}

/// What [`precompile`] made.
#[derive(Clone, Copy, Debug)]
pub struct Precompiled {
    /// The programs linked, and how long the renderer took to be built from them.
    pub programs: u32,
    pub link_ms: u64,
    /// How long the draws that went through every program took, and the GL error they raised, if any.
    pub warm_ms: u64,
    pub gl_error: Option<u32>,
}

/// Links every program the engine uses in `gles`' context, draws each of them once, and leaves them in `cache` for the engines of the
/// compositor's context to take: a renderer is built through the cache and dropped, and the cache is complete when this returns. Meant
/// for a thread of its own with a context that shares the compositor's, so that nothing is linked on the compositor's loop thread when a
/// client's engine attaches (`ui::compile` in the compositor). The panel can be small: the programs do not depend on its size, and the
/// draws only have to go through them.
pub fn precompile(mut gles: Gles, cache: &Arc<Cache>, width: u32, height: u32) -> io::Result<Precompiled> {
    gles.use_program_cache(Arc::clone(cache));
    let panel = Panel { rotation: Rotation::R0, logical_width: width as f32, logical_height: height as f32, width, height };
    let began = Instant::now();
    let mut renderer = Renderer::new(gles, panel)?;
    // The picture copy is the host's own program, not the renderer's: it goes to the cache with them.
    let copy = PictureCopy::new(renderer.gles(), width, height)?;
    let link_ms = began.elapsed().as_millis() as u64;
    let programs = renderer.gles().programs_linked();
    let began = Instant::now();
    let gl_error = if std::env::var("LUMASHOW_WARMUP").as_deref() == Ok("off") {
        None
    } else {
        let error = renderer.warm_up();
        let (floor, page) = (renderer.gles().new_render_target(width, height)?, renderer.gles().new_render_target(width, height)?);
        copy.run(renderer.gles(), &floor, page.framebuffer, &[[0, 0, width as i32, height as i32]]);
        renderer.gles().finish();
        renderer.gles().delete_render_target(floor);
        renderer.gles().delete_render_target(page);
        error.or(renderer.gles().take_error())
    };
    renderer.gles().finish();
    // The pictures the draws made go back (textures are the share group's, and outlive the renderer and the thread's context).
    renderer.trim();
    renderer.gles().finish();
    let _ = crate::gl::take_target_cost();
    let warm_ms = began.elapsed().as_millis() as u64;
    // The programs go to the cache with the renderer, and the cache is not complete before they have.
    drop(renderer);
    cache.mark_complete();
    Ok(Precompiled { programs, link_ms, warm_ms, gl_error })
}

impl Engine {
    /// A compositor for `gles` (the window's context, current on this thread) and the app's end of the socket it speaks over.
    /// The handshake (`Hello`, `Welcome`) is answered on a thread so that `ui::Scene::over_with`, which blocks on it, can run
    /// on this one; call [`Engine::finish_handshake`] once it has returned.
    pub fn new(gles: Gles, config: HostConfig) -> io::Result<(Self, UnixStream)> {
        let max_texture = gles.max_texture_size();
        let panel = Panel {
            rotation: Rotation::R0,
            logical_width: config.width as f32,
            logical_height: config.height as f32,
            width: config.width,
            height: config.height,
        };
        // Inside a compositor the context is its own: what is set in it is saved and put back round everything made here.
        let entered = config.external.then(|| gles.enter());
        let (pages, reference, blit) = if config.external {
            (Vec::new(), None, None)
        } else {
            (
                (0..config.pages.max(1)).map(|_| gles.new_render_target(config.width, config.height)).collect::<io::Result<Vec<_>>>()?,
                config.reference.then(|| gles.new_render_target(config.width, config.height)).transpose()?,
                Some(Blit::new(&gles, config.width, config.height)?),
            )
        };
        let built = Instant::now();
        let mut renderer = Renderer::new(gles, panel)?;
        renderer.set_importer(None);
        // Two stacks: inside a compositor, or the host's pages drawn the way a compositor draws them.
        let stacks = config.external || (config.system_z < i32::MAX && !config.direct);
        if stacks {
            // One account for each stack. They have no ring of their own (the compositor keeps it, or the host as a compositor
            // does) but are asked to note what the app damaged of its pictures, which only an account with targets does.
            renderer.set_targets(0, 1);
            renderer.set_targets(1, 1);
        } else if config.pages > 1 {
            renderer.set_targets(0, config.pages);
        }
        // A renderer that took its programs from the cache has nothing to warm up: they were drawn once where they were linked
        // ([`precompile`], or the engine this process had before a client restarted), and a warm-up here is a pass of tens of
        // milliseconds on the compositor's loop thread for nothing.
        let linked = renderer.gles().programs_linked();
        if linked > 0 && std::env::var("LUMASHOW_WARMUP").as_deref() != Ok("off") {
            let began = Instant::now();
            let error = renderer.warm_up();
            let _ = crate::gl::take_target_cost();
            crate::log::log(format!(
                "GPU warm-up: one frame of every program in {} ms{}",
                began.elapsed().as_millis(),
                error.map_or(String::new(), |e| format!(" (GL error {e:#x})"))
            ));
        }
        let copy = if stacks {
            match PictureCopy::new(renderer.gles(), config.width, config.height) {
                Ok(copy) => Some(copy),
                Err(e) => {
                    crate::log::log(format!("stack 0 is drawn into each page, not from a floor: {e}"));
                    None
                }
            }
        } else {
            None
        };
        // `LUMATV_FLOOR=0` draws stack 0 straight into each page, as before the floor: for comparing the two on the set.
        let floor = (copy.is_some() && std::env::var("LUMATV_FLOOR").as_deref() != Ok("0"))
            .then(|| Floor {
                target: None,
                owed: TileSet::full(config.width, config.height),
                here: TileSet::new(config.width, config.height),
                unused: 0,
                direct: false,
                behind: false,
                drawn_px: 0,
            });
        if let Some(saved) = &entered {
            renderer.gles().leave(saved);
        }
        renderer.gles().programs_complete();
        crate::log::log(format!(
            "engine built in {:.1} ms: {} programs linked here, {} taken from the cache",
            built.elapsed().as_secs_f64() * 1e3,
            linked,
            renderer.gles().programs_taken()
        ));
        let stats = Arc::new(SharedStats::default());
        let session_config = Arc::new(Config {
            limits: Limits::for_texture_size(max_texture),
            panel_width: config.width,
            panel_height: config.height,
            refresh_hz: config.refresh_hz,
            shm_dir: config.shm_dir,
            gpu_opp_khz: Vec::new(),
        });
        let mut session = Session::new(session_config, Arc::clone(&stats), 1);
        // The app's Hello is answered by the thread below; the session is told of it here so that it is greeted.
        let _ = session.handle(Request::Hello { version: PROTOCOL_VERSION });
        let (app, ours) = UnixStream::pair()?;
        let (width, height, hz) = (config.width, config.height, config.refresh_hz);
        let handshake = thread::Builder::new().name("handshake".into()).spawn(move || {
            let mut ours = ours;
            let mut buf = Vec::new();
            match read_frame::<Request>(&mut ours, &mut buf)? {
                Some(Request::Hello { version }) if version == PROTOCOL_VERSION => {}
                other => return Err(io::Error::other(format!("expected Hello, got {other:?}"))),
            }
            write_frame(
                &mut ours,
                &Event::Welcome { version: PROTOCOL_VERSION, panel_width: width, panel_height: height, refresh_hz: hz, smooth_first_full: false },
            )?;
            Ok(ours)
        })?;
        let now = now_ns();
        Ok((
            Self {
                renderer,
                scene: Scene::new(panel.logical_width, panel.logical_height, (1e9 / f64::from(config.refresh_hz)) as u64),
                session,
                stats,
                stream: None,
                handshake: Some(handshake),
                inbox: Vec::new(),
                outbox: Vec::new(),
                drawn: Vec::new(),
                shown: Vec::new(),
                pages,
                reference,
                mismatch: None,
                next_page: 0,
                blit,
                window: (config.width, config.height),
                direct: config.direct,
                copy: config.blit,
                gpu_sync: config.gpu_sync,
                transparent: false,
                highest_tag: 0,
                taken_tag: 0,
                told_tag: 0,
                waiting: Vec::new(),
                idle: config.idle_trim.then(|| Idle::from_env(now)),
                tidy_more: false,
                uploaded_bytes: 0,
                external: stacks.then(|| External {
                    exclusion: None,
                    exclusion_dirty: false,
                    standin_groups: None,
                    compositor: config.external,
                    ring: (!config.external).then(|| Ring::new(config.pages.max(1), config.width, config.height)),
                    last_page: None,
                    last_frame_ns: None,
                    period_ns: (1e9 / f64::from(config.refresh_hz)) as u64,
                    ambient_copy: std::env::var("LUMATV_AMBIENT_COPY").as_deref() != Ok("0"),
                    page_copy: std::env::var("LUMATV_PAGE_COPY").as_deref() != Ok("0"),
                    merging: TileSet::new(config.width, config.height),
                    system_z: config.system_z,
                    z: HashMap::new(),
                    planned: None,
                    split: 0,
                    report: DrawReport::default(),
                    busy: Duration::ZERO,
                    spent: Spent::default(),
                    scope: config.external || std::env::var("LUMATV_SCOPES").as_deref() == Ok("1"),
                    lazy: std::env::var("LUMATV_LAZY_SCOPES").as_deref() != Ok("0"),
                }),
                floor,
                copier: copy,
                ahead: false,
                over_frames: 0,
                home_shown: true,
                take_ahead: std::env::var("LUMATV_AHEAD").as_deref() == Ok("1"),
                apply_ns: 0,
            },
            app,
        ))
    }

    pub fn finish_handshake(&mut self) -> io::Result<()> {
        let handle = self.handshake.take().ok_or_else(|| io::Error::other("handshake already finished"))?;
        let stream = handle.join().map_err(|_| io::Error::other("the handshake thread panicked"))??;
        stream.set_nonblocking(true)?;
        self.stream = Some(stream);
        Ok(())
    }

    /// Whether the handshake is still awaited (the app has not said hello, or nothing has been finished yet).
    pub fn handshake_pending(&self) -> bool {
        self.handshake.is_some()
    }

    /// Finishes the handshake if the app has answered, without waiting: `Ok(true)` when it is complete now, `Ok(false)` while the
    /// app has not connected yet or there is nothing to finish; a handshake that failed (the wrong hello, the socket closed) is the
    /// error, and nothing is left to finish.
    pub fn try_finish_handshake(&mut self) -> io::Result<bool> {
        if !self.handshake.as_ref().is_some_and(|h| h.is_finished()) {
            return Ok(false);
        }
        self.finish_handshake().map(|()| true)
    }

    pub fn gles(&self) -> &Gles {
        self.renderer.gles()
    }

    /// Runs `f`, which draws or makes GL objects, in the compositor's context as the toolkit's: the context's state is saved and
    /// put back round it (an engine that is not inside a compositor owns its context, and `f` just runs).
    fn scoped<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        if !self.external.as_ref().is_some_and(|external| external.scope) {
            return f(self);
        }
        let began = Instant::now();
        let saved = self.renderer.gles().enter();
        let entered = began.elapsed();
        let out = f(self);
        let leaving = Instant::now();
        self.renderer.gles().leave(&saved);
        if let Some(external) = self.external.as_mut() {
            external.busy += began.elapsed();
            external.spent.scope += entered + leaving.elapsed();
            external.spent.scopes += 1;
        }
        out
    }

    /// [`Engine::scoped`] for a call that needs the context only when `gl` says it does. Saving the context's state and putting it back is some
    /// eighty calls into the driver (`Gles::enter` and `leave`), and a frame at rest takes them for each call of the compositor's (planning,
    /// two stacks, ending the frame, housekeeping) though planning, ending and housekeeping call the GL only when a picture is to be copied,
    /// a chunk of an atlas to land, a texture to free or a floor to bring up to date. Without scope the call is checked: if it reached the driver
    /// all the same, the context's state may be left as the compositor did not leave it, which is said once, and every call takes its scope
    /// from then on.
    fn scoped_when<R>(&mut self, gl: bool, f: impl FnOnce(&mut Self) -> R) -> R {
        if gl || !self.external.as_ref().is_some_and(|external| external.scope && external.lazy) {
            return self.scoped(f);
        }
        let began = Instant::now();
        let touched = self.renderer.gles().touched();
        let out = f(self);
        let wrong = self.renderer.gles().touched() != touched;
        if let Some(external) = self.external.as_mut() {
            external.busy += began.elapsed();
            if wrong {
                external.lazy = false;
            }
        }
        if wrong {
            crate::log::log("engine: a call made outside the context's state scope reached the GL; every call takes its scope from now on".to_string());
        }
        out
    }

    /// Reads what the app has sent, without waiting, runs it through the session and keeps the commits for the next frame.
    pub fn pump(&mut self) -> io::Result<Pumped> {
        let mut pumped = Pumped::default();
        let Some(stream) = self.stream.as_mut() else {
            return Ok(pumped);
        };
        let mut chunk = [0u8; 16 * 1024];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => {
                    pumped.closed = true;
                    break;
                }
                Ok(n) => self.inbox.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        let mut at = 0;
        let mut buf = Vec::new();
        loop {
            let rest = &self.inbox[at..];
            if rest.len() < 4 {
                break;
            }
            let len = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
            if rest.len() < 4 + len {
                break;
            }
            let mut frame = &rest[..4 + len];
            let request = read_frame::<Request>(&mut frame, &mut buf)?.ok_or_else(|| io::Error::other("an empty frame"))?;
            at += 4 + len;
            let is_commit = matches!(request, Request::Commit { .. });
            let outcome = self.session.handle(request);
            for event in outcome.replies {
                self.send(&event);
            }
            if let Some(mut transaction) = outcome.commit {
                transaction.received_ns = now_ns();
                if let Some(tag) = transaction.tag {
                    self.highest_tag = self.highest_tag.max(tag);
                }
                self.waiting.push(transaction);
            }
            if is_commit {
                pumped.commits += 1;
            } else {
                pumped.requests += 1;
            }
            if outcome.close {
                pumped.closed = true;
            }
        }
        self.inbox.drain(..at);
        self.flush();
        Ok(pumped)
    }

    fn send(&mut self, event: &Event) {
        let _ = write_frame(&mut self.outbox, event);
    }

    /// Writes the events queued for the app. A full socket keeps the rest for the next call: the app reads its events on a thread
    /// of its own, so this clears at once in practice.
    fn flush(&mut self) {
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        let mut sent = 0;
        while sent < self.outbox.len() {
            match io::Write::write(stream, &self.outbox[sent..]) {
                Ok(0) => break,
                Ok(n) => sent += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        self.outbox.drain(..sent);
    }

    /// Commits are waiting for a frame.
    pub fn has_commits(&self) -> bool {
        self.waiting.iter().any(Transaction::asks_for_frame)
    }

    /// What the commits that ask for a frame hold, as `Set x3 Animate x40` (the kinds of their ops and how many of each): for a host that says why a
    /// frame was drawn.
    pub fn waiting_summary(&self) -> String {
        let mut kinds: Vec<(String, usize)> = Vec::new();
        for op in self.waiting.iter().filter(|t| t.asks_for_frame()).flat_map(|t| &t.ops) {
            let text = format!("{op:?}");
            let kind = text.split([' ', '{', '(']).next().unwrap_or("").to_string();
            match kinds.iter_mut().find(|(k, _)| *k == kind) {
                Some((_, n)) => *n += 1,
                None => kinds.push((kind, 1)),
            }
        }
        kinds.iter().map(|(k, n)| format!("{k}x{n}")).collect::<Vec<_>>().join(",")
    }

    /// Commits are waiting, whether or not any asks for a frame.
    pub fn has_waiting(&self) -> bool {
        !self.waiting.is_empty()
    }

    /// The engine's end of the app's socket, for a host that polls it (a commit then wakes the host's loop). `None` until the
    /// handshake is finished.
    pub fn fd(&self) -> Option<std::os::fd::RawFd> {
        use std::os::fd::AsRawFd;
        self.stream.as_ref().map(|stream| stream.as_raw_fd())
    }

    /// What the scene wants on its own: an animation at every refresh, a wallpaper's step at its time, or nothing.
    pub fn demand(&self) -> Demand {
        self.scene.demand()
    }

    /// Work between frames changed what the next frame draws (a glyph list swapped in, a backdrop published), or the floor is
    /// behind and the scene has nothing more to show, so a frame can bring the floor up to date ([`Engine::finish`]).
    pub fn redraw_wanted(&self) -> bool {
        self.renderer.wants_frame()
            || self.ahead
            || self.external.as_ref().is_some_and(|external| external.exclusion_dirty)
            || (self.floor.as_ref().is_some_and(|floor| floor.behind) && self.home_shown && matches!(self.scene.demand(), Demand::Idle))
    }

    /// Takes in now the waiting commits when they are large ([`AHEAD_OPS`]), so that the frame that shows them does not: the
    /// frame after a key that makes a screen (Settings, about 100 layers) spent 7.0-7.2 ms of the compositor's loop at the set
    /// with 2.8-3.0 ms of drawing, at a latch the pacer had set from the cheap frames before it (8.8 ms before the refresh), and
    /// missed its refresh (round 26, 60 Hz). Taken in when the commit arrives, the work is done in the time to the latch. The
    /// next frame is due for it ([`Engine::redraw_wanted`]); its motion starts from when it was taken in. Whether it took them.
    pub fn take_in_ahead(&mut self, now_ns: u64) -> bool {
        let ops: usize = self.waiting.iter().map(|batch| batch.ops.len()).sum();
        if !self.take_ahead || ops < AHEAD_OPS {
            return false;
        }
        self.scoped(|engine| {
            engine.apply_commits(now_ns);
            engine.apply_ns = 0;
        });
        self.ahead = true;
        true
    }

    /// Whether housekeeping has more to do.
    pub fn tidy_more(&self) -> bool {
        self.tidy_more
    }

    /// Housekeeping for at most a couple of milliseconds: the work a frame leaves undone (textures to free and to upload, a chunk
    /// of an atlas, a prepared backdrop). Call it while the GPU has the frame, or when nothing is due.
    pub fn tidy(&mut self) {
        let gl = self.renderer.tidy_pending();
        self.scoped_when(gl, |engine| {
            let mut done = engine.renderer.tidy(TIDY_BUDGET);
            if !done.more {
                let made = engine.renderer.prepare(&engine.scene, TIDY_BUDGET);
                done.captured += made.captured;
                done.more = made.more;
            }
            engine.uploaded_bytes = engine.uploaded_bytes.saturating_add(u32::try_from(done.uploaded_bytes).unwrap_or(u32::MAX));
            engine.tidy_more = done.more;
            engine.forward_events();
        });
    }

    /// The quiet moments (`idle.rs`): after a few seconds of nothing, frees what a still screen does not need.
    pub fn idle_moment(&mut self, busy: bool) {
        self.scoped(|engine| {
            let now = now_ns();
            if let Some(idle) = engine.idle.as_mut() {
                if busy {
                    idle.active(now);
                } else {
                    idle.moment(&mut engine.renderer, &mut Process, now, true);
                }
            }
        });
    }

    /// When the idle housekeeping next wants the loop to wake.
    pub fn idle_wake_at(&self, ready: bool) -> Option<u64> {
        self.idle.as_ref().and_then(|idle| idle.wake_at(ready))
    }

    fn forward_events(&mut self) {
        for event in self.renderer.drain_events() {
            self.send(&event);
        }
        let mut notices = Vec::new();
        self.scene.take_notices(&mut notices);
        for (id, notice) in notices {
            self.send(&Event::Motion {
                id,
                generation: notice.generation,
                kind: notice.kind,
                value: notice.value,
                velocity: notice.velocity,
                time_ns: notice.time_ns,
            });
        }
        self.flush();
    }

    /// Applies the commits that wait to the scene and the renderer, in order, with no frame for them: what the app staged that
    /// shows nothing by itself (an atlas chunk, a layer made ahead). `draw` does the same for the commits a frame is due for.
    pub fn apply_waiting(&mut self, now_ns: u64) {
        let gl = self.waiting_touches_gl();
        self.scoped_when(gl, |engine| engine.apply_commits(now_ns));
    }

    /// Whether applying the commits that wait can call the GL (or leave a picture to be copied): one of their ops is not among those that
    /// leave it alone.
    fn waiting_touches_gl(&self) -> bool {
        self.waiting.iter().flat_map(|batch| &batch.ops).any(|op| !leaves_gl_alone(op))
    }

    fn apply_commits(&mut self, now_ns: u64) {
        let began = Instant::now();
        for batch in std::mem::take(&mut self.waiting) {
            if let Some(tag) = batch.tag {
                self.taken_tag = self.taken_tag.max(tag);
            }
            for op in &batch.ops {
                if let Some(external) = self.external.as_mut() {
                    match op {
                        Op::Create { id, z, .. } => {
                            external.z.insert(*id, *z);
                        }
                        Op::Delete { id } => {
                            external.z.remove(id);
                        }
                        Op::Clear => external.z.clear(),
                        _ => {}
                    }
                }
                self.scene.apply(op, now_ns);
                self.renderer.apply(op);
            }
        }
        self.forward_events();
        self.apply_ns += began.elapsed().as_nanos() as u64;
    }

    /// Clears each frame to nothing instead of opaque black, so that the window shows what is under it wherever no layer is drawn:
    /// an app over a video plane (the GeForce NOW app over its stream). A scene whose layers cover the whole window looks the same.
    pub fn set_transparent(&mut self, on: bool) {
        self.transparent = on;
    }

    pub fn set_silk(&mut self, id: LayerId, silk: crate::Silk) -> bool {
        self.renderer.set_silk(id, silk)
    }

    pub fn clear_silk(&mut self) -> bool { self.renderer.clear_silk() }

    /// Takes in the waiting commits, draws the scene as it is at `now_ns` and leaves it in the window's framebuffer, ready for
    /// the swap.
    pub fn draw(&mut self, now_ns: u64) -> FrameCost {
        if self.external.as_ref().is_some_and(|external| !external.compositor) {
            return self.draw_stacks(now_ns);
        }
        let began = Instant::now();
        self.ahead = false;
        self.apply_ns = 0;
        self.apply_waiting(now_ns);
        self.scene.advance(now_ns);
        self.scene.resolve(now_ns, &mut self.drawn);
        self.renderer.set_time(now_ns);
        let page = self.next_page;
        let ring = self.pages.len() > 1;
        if ring {
            self.next_page = (page + 1) % self.pages.len();
        }
        let target = if self.direct { 0 } else { self.pages[page].framebuffer };
        let slot = ring.then_some(page);
        let surface = Surface { stack: 0, from: 0, to: self.drawn.len(), transparent: self.transparent };
        let own = Draw { surface, slot, target, areas: None, clear: true };
        let whole = self.reference.map(|r| Draw { surface, slot: None, target: r.framebuffer, areas: None, clear: true });
        let draws = [own, whole.unwrap_or(own)];
        let draws = &draws[..1 + usize::from(whole.is_some())];
        let report: DrawReport = self.renderer.draw_frame(&self.drawn, draws, &self.scene).remove(0);
        if let Some(reference) = self.reference {
            self.mismatch = self.compare(&self.pages[page], &reference);
        }
        if let (Some(blit), false, true) = (&self.blit, self.direct, self.copy) {
            blit.run(self.renderer.gles(), self.pages[page].texture, self.window.0, self.window.1);
        }
        self.shown.clone_from(&self.drawn);
        self.renderer.set_shown(&self.shown);
        self.scene.observe(self.renderer.unseen());
        self.scene.fade(report.fading, true);
        let cpu_us = began.elapsed().as_micros() as u32;
        let gpu_us = if self.gpu_sync {
            self.renderer.gles().finish();
            began.elapsed().as_micros() as u32
        } else {
            0
        };
        self.cost(&report, cpu_us, gpu_us)
    }

    /// [`Engine::draw`] for a host that draws its pages the way the compositor on the set does ([`HostConfig::system_z`]): the frame
    /// is planned; the page owes what the two stacks changed over the frames since it was drawn into (as the compositor's ring keeps
    /// it); what it owes from the frames before this one is copied from the page drawn last, which holds that frame whole; and stack
    /// 0 then stack 1 are drawn where this frame changed (in at most [`COMPOSITOR_RECTS`] rectangles, merged where one pass costs less).
    /// The reference is the same frame drawn whole, every layer straight into it, the floor left out.
    fn draw_stacks(&mut self, now_ns: u64) -> FrameCost {
        let began = Instant::now();
        if self.plan(now_ns).is_err() {
            return FrameCost::default();
        }
        let page = self.next_page;
        self.next_page = (page + 1) % self.pages.len();
        let (width, height) = self.window;
        let mut changed = TileSet::new(width, height);
        for stack in 0..2 {
            if let Some(set) = self.changed(stack) {
                changed.union(set);
            }
        }
        let mut owed = TileSet::new(width, height);
        let mut last = None;
        // As the compositor does: the pages copy what they owe only while something is over Home (`kms::frames`).
        let over = self.over_home();
        if let Some(external) = self.external.as_mut() {
            if let Some(ring) = external.ring.as_mut() {
                ring.frame(page, &changed, &mut owed);
            }
            let calm = external.ambient_copy
                && external.last_frame_ns.is_some_and(|at| now_ns.saturating_sub(at) >= AMBIENT_COPY_REFRESHES * external.period_ns)
                && !matches!(self.scene.demand(), Demand::EveryRefresh);
            last = external.last_page.filter(|&p| p != page && external.page_copy && (over || calm));
        }
        let mut areas = Vec::new();
        if let (Some(last), Some(copier)) = (last, self.copier.as_ref()) {
            owed.subtract(&changed);
            owed.rects(&mut areas);
            if areas.len() > COMPOSITOR_RECTS {
                areas.clear();
                areas.extend(owed.bounds());
            }
            copier.run(self.renderer.gles(), &self.pages[last], self.pages[page].framebuffer, &areas);
            owed.clone_from(&changed);
        }
        owed.rects(&mut areas);
        if areas.len() > COMPOSITOR_RECTS {
            areas.clear();
            areas.extend(owed.bounds());
        }
        self.merge_areas(&mut areas);
        let target = self.pages[page].framebuffer;
        if !areas.is_empty() {
            let _ = self.draw_stack(0, target, &areas);
            // (as the compositor does: no layer over the windows, no pass over them)
            if self.has_layers(1) {
                let _ = self.draw_stack(1, target, &areas);
            }
        }
        if let (Some(reference), Some(external)) = (self.reference, self.external.as_mut())
            && let Some(planned) = external.planned.as_mut()
        {
            let surface = Surface { stack: 0, from: 0, to: self.drawn.len(), transparent: false };
            self.renderer.draw_planned(planned, &Draw { surface, slot: None, target: reference.framebuffer, areas: None, clear: true });
        }
        let mut cost = self.finish().unwrap_or_default();
        if let Some(external) = self.external.as_mut() {
            external.last_page = Some(page);
            external.last_frame_ns = Some(now_ns);
        }
        if let Some(reference) = self.reference {
            self.mismatch = self.compare(&self.pages[page], &reference);
            cost.mismatch_px = self.mismatch.map_or(0, |m| m.pixels);
        }
        if let (Some(blit), true) = (&self.blit, self.copy) {
            blit.run(self.renderer.gles(), self.pages[page].texture, self.window.0, self.window.1);
        }
        cost.region_px = areas.iter().map(crate::damage::pixels).sum();
        cost.rects = areas.len() as u16;
        cost.damage_px = changed.pixels();
        // (with the scopes of a compositor's frame, `LUMATV_SCOPES=1`, what the engine's own calls took, as the compositor counts it: not the host's
        // account of its pages, which the compositor keeps itself)
        if !self.external.as_ref().is_some_and(|external| external.scope) {
            cost.cpu_us = began.elapsed().as_micros() as u32;
        }
        if self.gpu_sync {
            self.renderer.gles().finish();
            cost.gpu_us = began.elapsed().as_micros() as u32;
        }
        cost
    }

    fn cost(&mut self, report: &DrawReport, cpu_us: u32, gpu_us: u32) -> FrameCost {
        let (_captures, uploaded) = self.renderer.take_frame_counts();
        let spent = self.external.as_mut().map(|external| std::mem::take(&mut external.spent)).unwrap_or_default();
        FrameCost {
            tag: self.taken_tag,
            cpu_us,
            gpu_us,
            place_us: (report.place_ns / 1000) as u32,
            plan_us: (report.plan_ns / 1000) as u32,
            backdrop_us: (report.backdrop_ns / 1000) as u32,
            draw_us: (report.draw_ns / 1000) as u32,
            damage_us: (report.damage_ns / 1000) as u32,
            layers: report.drawn,
            culled: report.culled,
            backdrops: report.backdrops,
            uploaded_bytes: uploaded.saturating_add(self.uploaded_bytes),
            draw_calls: report.gl.draws,
            calls: report.gl,
            gl_error: report.gl_error,
            region_px: report.region_px,
            rects: report.rects,
            damage_px: report.damage_px,
            mismatch_px: self.mismatch.map_or(0, |m| m.pixels),
            floor_px: self.floor.as_ref().map_or(0, |floor| floor.drawn_px),
            apply_us: (std::mem::take(&mut self.apply_ns) / 1000) as u32,
            scope_us: spent.scope.as_micros() as u32,
            step_us: spent.step.as_micros() as u32,
            finish_us: spent.finish.as_micros() as u32,
            scopes: spent.scopes,
        }
    }

    /// Sets the protected area for subsequent compositor frame plans.
    pub fn set_exclusion(&mut self, exclusion: Option<Exclusion>) {
        if let Some(external) = self.external.as_mut() {
            external.exclusion_dirty |= external.exclusion != exclusion;
            external.exclusion = exclusion;
        }
    }

    /// Omits these two layer groups while a composed stand-in supplies old app pixels.
    pub fn set_standin_groups(&mut self, groups: Option<((i32, i32), (i32, i32))>) {
        if let Some(external) = self.external.as_mut() {
            external.exclusion_dirty |= external.standin_groups != groups;
            external.standin_groups = groups;
        }
    }

    /// The first half of a frame inside a compositor ([`HostConfig::external`]): takes in the waiting commits, steps the scene to
    /// `now_ns`, places the layers and works out what each stack changes from the frame before ([`Engine::changed`]). Nothing is
    /// drawn: the compositor adds the change to its own account of what each page owes, and draws.
    pub fn plan(&mut self, now_ns: u64) -> io::Result<()> {
        if self.external.as_ref().is_none_or(|external| external.planned.is_some()) {
            return Err(io::Error::other("plan: not an engine inside a compositor, or a frame is already planned"));
        }
        let gl = self.waiting_touches_gl() || self.renderer.plan_touches_gl();
        self.scoped_when(gl, |engine| {
            engine.ahead = false;
            engine.apply_ns = 0;
            engine.apply_commits(now_ns);
            let stepping = Instant::now();
            engine.scene.advance(now_ns);
            engine.scene.resolve(now_ns, &mut engine.drawn);
            engine.renderer.set_time(now_ns);
            let Some(external) = engine.external.as_mut() else { return };
            external.spent.step += stepping.elapsed();
            external.exclusion_dirty = false;
            if let Some((popup, marker)) = external.standin_groups {
                engine.drawn.retain(|layer| external.z.get(&layer.id).is_none_or(|&z| {
                    !(z >= popup.0 && z < popup.1 || z >= marker.0 && z < marker.1)
                }));
            }
            if let Some(exclusion) = external.exclusion {
                let overlaps = |range: (i32, i32)| {
                    engine.drawn.iter().any(|layer| {
                        external.z.get(&layer.id).is_some_and(|&z| z >= range.0 && z < range.1)
                            && engine.renderer.painted_extent(layer).is_some_and(|area| {
                                crate::damage::intersect(&area, &exclusion.area).is_some()
                            })
                    })
                };
                let popup = overlaps(exclusion.popup_z);
                let marker = overlaps(exclusion.marker_z);
                engine.drawn.retain(|layer| {
                    external.z.get(&layer.id).is_none_or(|&z| {
                        !(popup && z >= exclusion.popup_z.0 && z < exclusion.popup_z.1
                            || marker && z >= exclusion.marker_z.0 && z < exclusion.marker_z.1)
                    })
                });
            }
            external.split = engine.drawn.partition_point(|layer| external.z.get(&layer.id).is_some_and(|&z| z < external.system_z));
            engine.over_frames = if external.split < engine.drawn.len() { OVER_FRAMES } else { engine.over_frames.saturating_sub(1) };
            let mut planned = engine.renderer.plan_frame(&engine.drawn, &engine.scene);
            for stack in 0..2 {
                let surface = external.surface(stack, engine.drawn.len());
                engine.renderer.damage(&mut planned, surface);
            }
            // What stack 0 changed is owed by the floor until it is drawn into, in whichever frame that is.
            if let Some(floor) = engine.floor.as_mut() {
                if let Some(changed) = planned.changed(0) {
                    floor.owed.union(changed);
                }
                floor.unused = floor.unused.saturating_add(1);
                floor.direct = false;
                floor.drawn_px = 0;
            }
            external.planned = Some(planned);
            external.report = DrawReport::default();
        });
        Ok(())
    }

    /// What the frame planned changes of stack 0 (under the windows) or 1 (over them) from the frame before, in tiles of the panel:
    /// what the compositor adds to every page's account, whether it draws the stack in this page or not.
    pub fn changed(&self, stack: usize) -> Option<&TileSet> {
        self.external.as_ref()?.planned.as_ref()?.changed(stack)
    }

    /// Whether the planned frame has any layer on `stack`.
    pub fn has_layers(&self, stack: usize) -> bool {
        let Some(external) = self.external.as_ref() else { return false };
        if stack == 0 { external.split > 0 } else { external.split < self.drawn.len() }
    }

    /// Whether something of stack 1 is over Home in the planned frame, or was in one of the [`OVER_FRAMES`] before it: while it
    /// is, stack 0 comes from the floor and the pages copy what they owe (the module's notes).
    pub fn over_home(&self) -> bool {
        self.over_frames > 0
    }

    /// Whether Home shows in the frames planned from now (the compositor's: not while an app's picture covers it): the floor is
    /// brought up to date only while it does.
    pub fn set_home_shown(&mut self, shown: bool) {
        self.home_shown = shown;
    }

    /// `areas`, what a page owes, as the rectangles the planned frame is cheapest drawn in ([`Planned::merged`]): a pass draws every
    /// layer that meets its rectangle, so the rows of a list that scrolls (a dozen bands, each with the sheet and the copy of the
    /// floor under it) are cheaper as one cover. Covers that would draw much more of the panel than the rectangles are not taken
    /// ([`MERGE_GROWTH_QUARTERS`]). The rectangles that come back hold all that `areas` did and do not overlap: the page draws its
    /// stacks and its windows over all of them in turn, and an overlap would be drawn over twice. Left as they are with no frame
    /// planned.
    pub fn merge_areas(&mut self, areas: &mut Vec<Area>) {
        let (floor, layers) = (self.floor.is_some() && self.over_home(), self.drawn.len());
        let Some(external) = self.external.as_mut() else { return };
        let Some(planned) = external.planned.as_ref() else { return };
        let Some(region) = Region::exact(areas) else { return };
        // With the floor (something over Home), stack 0 is one copy in each pass, whatever it holds.
        let merged = within_growth(planned.merged(region, if floor { external.split..layers } else { 0..layers }), region);
        if merged == region {
            return;
        }
        let tiles = &mut external.merging;
        tiles.clear();
        for &area in merged.rects() {
            tiles.add(area);
        }
        tiles.rects(areas);
        if areas.len() > COMPOSITOR_RECTS {
            areas.clear();
            areas.extend(tiles.bounds());
        }
    }

    /// Draws stack 0 or 1 of the planned frame into the framebuffer `target` (a page of the compositor's), inside `areas` and
    /// nowhere else: stack 0 clears them first, and stack 1 is drawn over what the compositor has drawn there. The areas are
    /// what the page owes, so that no pixel of the windows around them is touched. While something is over Home, stack 0 comes
    /// from the floor: what the floor owes under the areas is drawn into it first, and each area is then one copy of it.
    pub fn draw_stack(&mut self, stack: usize, target: u32, areas: &[Area]) -> io::Result<()> {
        self.draw_stack_from(stack, target, areas, true, stack == 0)
    }

    /// [`Engine::draw_stack`] with every layer of the stack drawn straight into `target`, the floor left out and left as it is:
    /// what a check compares a page drawn from the floor with.
    pub fn draw_stack_direct(&mut self, stack: usize, target: u32, areas: &[Area]) -> io::Result<()> {
        self.draw_stack_from(stack, target, areas, false, stack == 0)
    }

    /// Draws a stack over existing compositor pixels without clearing or using the opaque Home floor.
    pub fn draw_stack_over(&mut self, stack: usize, target: u32, areas: &[Area]) -> io::Result<()> {
        self.draw_stack_from(stack, target, areas, false, false)
    }

    fn draw_stack_from(&mut self, stack: usize, target: u32, areas: &[Area], from_floor: bool, clear: bool) -> io::Result<()> {
        let Some(region) = Region::exact(areas) else {
            return Err(io::Error::other("draw_stack: more areas than a region keeps apart"));
        };
        if self.external.as_ref().is_none_or(|external| external.planned.is_none()) {
            return Err(io::Error::other("draw_stack: no frame is planned"));
        }
        let over = self.over_home();
        self.scoped(|engine| {
            let Some(external) = engine.external.as_mut() else { return };
            let surface = external.surface(stack, engine.drawn.len());
            let Some(planned) = external.planned.as_mut() else { return };
            if stack == 0
                && from_floor
                && surface.to > 0
                && let (Some(floor), Some(copier)) = (engine.floor.as_mut(), engine.copier.as_ref())
            {
                floor.unused = 0;
                if !over {
                    // Nothing over Home: the floor is for what moves over it. Home's own walks drew through it 122 layers and a copy
                    // where round 23 drew 39 straight (0.9 Mpx: 7.5 against 4.2 ms of GPU), and dropped 29 (apps-grid) and 28
                    // (home-widgets) frames at 120 Hz against 3 and 3 (round 27). Home is drawn straight into the page, and the
                    // floor keeps owing what changed until a frame in which Home stands brings it up to date (`finish`). A page that
                    // owes nothing is not drawn into, though the compositor calls this for it: counted as drawing Home straight, no frame
                    // could be the one that brings the floor up to date, and a frame was wanted at every refresh that drew nothing
                    // ([`Engine::redraw_wanted`]).
                    if !region.is_empty() {
                        floor.direct = true;
                        floor.behind |= !floor.owed.is_empty();
                    }
                } else {
                    if floor.target.is_none() {
                        match engine.renderer.gles().new_render_target(engine.window.0, engine.window.1) {
                            Ok(made) => {
                                floor.target = Some(made);
                                floor.owed.fill();
                            }
                            Err(e) => crate::log::log(format!("no floor for stack 0 this frame: {e}")),
                        }
                    }
                    if let Some(picture) = floor.target {
                        // What the floor owes under the areas is drawn into it first, and nothing else of it: a frame draws the
                        // floor where it copies from it. Its passes are merged as the renderer merges its own pages' (a pass draws
                        // every layer of Home that meets it), within the same growth; they may overlap, which a picture of one
                        // stack, cleared and drawn whole in each, does not mind.
                        floor.here.clear();
                        for &area in region.rects() {
                            floor.here.union_within(&floor.owed, area);
                        }
                        if !floor.here.is_empty() {
                            let owed = floor.here.region();
                            let owed = within_growth(planned.merged(owed, 0..surface.to), owed);
                            let draw = Draw { surface, slot: None, target: picture.framebuffer, areas: Some(owed), clear: true };
                            let report = engine.renderer.draw_planned(planned, &draw);
                            external.report.add(&report);
                            floor.drawn_px += owed.pixels();
                            floor.owed.subtract(&floor.here);
                        }
                        copier.run(engine.renderer.gles(), &picture, target, region.rects());
                        // A copy is a layer drawn in each area, as the frame's cost counts layers.
                        external.report.drawn += region.rects().len() as u32;
                        return;
                    }
                }
            }
            let draw = Draw { surface, slot: None, target, areas: Some(region), clear };
            let report = engine.renderer.draw_planned(planned, &draw);
            external.report.add(&report);
        });
        Ok(())
    }

    /// The second half of a frame inside a compositor: what it showed is kept to say what the next one changes, and the scene is
    /// told what nothing showed of. Call it after the last `draw_stack`, whether anything was drawn or not.
    pub fn finish(&mut self) -> io::Result<FrameCost> {
        let Some(mut planned) = self.external.as_mut().and_then(|external| external.planned.take()) else {
            return Err(io::Error::other("finish: no frame is planned"));
        };
        let mut report = DrawReport::default();
        // The floor is brought up to date (drawn into) or given back (a target freed) in the frames that say so below; ending the frame
        // touches the context in no other.
        let floor_gl = self.floor.as_ref().is_some_and(|floor| {
            floor.unused >= FLOOR_UNUSED_FRAMES || (floor.behind && !floor.direct && self.home_shown && matches!(self.scene.demand(), Demand::Idle))
        });
        self.scoped_when(floor_gl, |engine| {
            // After frames that drew Home straight into the pages, the first frame in which Home shows and stands and nothing waits
            // to move draws what the floor owes (the floor whole, the first time), so that a panel opening over Home only copies it.
            if let Some(floor) = engine.floor.as_mut()
                && floor.behind
                && !floor.direct
                && engine.home_shown
                && matches!(engine.scene.demand(), Demand::Idle)
            {
                floor.behind = false;
                if floor.target.is_none() {
                    match engine.renderer.gles().new_render_target(engine.window.0, engine.window.1) {
                        Ok(made) => {
                            floor.target = Some(made);
                            floor.owed.fill();
                        }
                        Err(e) => crate::log::log(format!("no floor for stack 0: {e}")),
                    }
                }
                if let (Some(picture), Some(external)) = (floor.target, engine.external.as_mut())
                    && !floor.owed.is_empty()
                {
                    let surface = external.surface(0, engine.drawn.len());
                    let owed = floor.owed.region();
                    let owed = within_growth(planned.merged(owed, 0..surface.to), owed);
                    let draw = Draw { surface, slot: None, target: picture.framebuffer, areas: Some(owed), clear: true };
                    let report = engine.renderer.draw_planned(&mut planned, &draw);
                    external.report.add(&report);
                    floor.drawn_px += owed.pixels();
                    floor.owed.clear();
                }
            }
            let ending = Instant::now();
            let left = engine.renderer.finish_frame(planned);
            if let Some(external) = engine.external.as_mut() {
                external.report.add(&left);
                report = external.report;
            }
            engine.shown.clone_from(&engine.drawn);
            engine.renderer.set_shown(&engine.shown);
            engine.scene.observe(engine.renderer.unseen());
            engine.scene.fade(report.fading, true);
            if let Some(external) = engine.external.as_mut() {
                external.spent.finish += ending.elapsed();
            }
            // A floor that has not been drawn from for a while (Home under an app's picture) gives its memory back; the next frame
            // that draws stack 0 makes it again, whole.
            if let Some(floor) = engine.floor.as_mut()
                && floor.unused >= FLOOR_UNUSED_FRAMES
                && let Some(picture) = floor.target.take()
            {
                engine.renderer.gles().delete_render_target(picture);
                floor.owed.fill();
                floor.behind = false;
            }
        });
        let busy = self.external.as_mut().map_or(Duration::ZERO, |external| std::mem::take(&mut external.busy));
        let gpu_us = if self.gpu_sync {
            let waiting = Instant::now();
            self.renderer.gles().finish();
            (busy + waiting.elapsed()).as_micros() as u32
        } else {
            0
        };
        Ok(self.cost(&report, busy.as_micros() as u32, gpu_us))
    }

    /// Nothing is known any more of what the pages hold or what the layers showed: the next frame changes everything. For a
    /// stack that was not drawn for a while (covered by a window), or a page that may not have been drawn whole.
    pub fn forget(&mut self) {
        self.renderer.forget_targets();
        if let Some(floor) = self.floor.as_mut() {
            floor.owed.fill();
        }
        if let Some(external) = self.external.as_mut() {
            if let Some(ring) = external.ring.as_mut() {
                ring.forget();
            }
            external.last_page = None;
        }
    }

    /// The frame drawn last reached the screen at `presented_ns`: tells the app (which retires what the commit replaced). A frame that
    /// shows no commit the app has not been told of (an ambient animation's, a clock's) and had nothing to upload or make says nothing:
    /// the app learns from a frame which of its commits is on the screen and how long that took, and the UI process woke for each of
    /// these, at every refresh of a moving sky.
    pub fn frame_done(&mut self, presented_ns: u64, cost: &FrameCost) {
        self.stats.frame(cost.cpu_us.max(cost.gpu_us), 0);
        self.uploaded_bytes = 0;
        if cost.tag != self.told_tag || cost.uploaded_bytes > 0 || cost.backdrops > 0 {
            self.told_tag = cost.tag;
            self.send(&Event::Frame {
                tag: cost.tag,
                presented_ns,
                render_us: cost.cpu_us.max(cost.gpu_us),
                missed: 0,
                backdrops: cost.backdrops,
                gpu_mhz: 0,
                captures: 0,
                uploaded_bytes: cost.uploaded_bytes,
                quality: 0,
            });
        }
        self.forward_events();
    }

    /// The window's framebuffer as RGBA rows from the bottom (for a screenshot on a host without a window system).
    pub fn read_window(&self) -> Vec<u8> {
        let gl = self.renderer.gles();
        gl.bind_framebuffer(0);
        gl.read_pixels(0, 0, self.window.0, self.window.1)
    }

    /// Where the frame drawn last differs from the same frame drawn whole, if it does (`HostConfig::reference`): the pages are read
    /// back and compared pixel for pixel.
    pub fn mismatch(&self) -> Option<Mismatch> {
        self.mismatch
    }

    fn compare(&self, page: &RenderTarget, whole: &RenderTarget) -> Option<Mismatch> {
        let gl = self.renderer.gles();
        gl.bind_framebuffer(page.framebuffer);
        let drawn = gl.read_pixels(0, 0, page.width, page.height);
        gl.bind_framebuffer(whole.framebuffer);
        let expected = gl.read_pixels(0, 0, whole.width, whole.height);
        let mut mismatch: Option<Mismatch> = None;
        for (at, (a, b)) in drawn.chunks_exact(4).zip(expected.chunks_exact(4)).enumerate() {
            if a != b {
                let m = mismatch.get_or_insert(Mismatch {
                    pixels: 0,
                    first: (at as u32 % page.width, at as u32 / page.width),
                    page: [a[0], a[1], a[2], a[3]],
                    whole: [b[0], b[1], b[2], b[3]],
                });
                m.pixels += 1;
            }
        }
        mismatch
    }

    /// The pixels of render target `page` of the ring, as RGBA rows from the first in memory (the top of the screen).
    pub fn read_page(&self, page: usize) -> Vec<u8> {
        let gl = self.renderer.gles();
        gl.bind_framebuffer(self.pages[page].framebuffer);
        gl.read_pixels(0, 0, self.window.0, self.window.1)
    }

    pub fn layers(&self) -> usize {
        self.scene.len()
    }
}

/// Where a frame drawn as it was owed differs from the same frame drawn whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mismatch {
    /// How many pixels differ.
    pub pixels: u64,
    /// The first one (x, y; row 0 is the top of the screen), as the frame was drawn and as drawn whole.
    pub first: (u32, u32),
    pub page: [u8; 4],
    pub whole: [u8; 4],
}
