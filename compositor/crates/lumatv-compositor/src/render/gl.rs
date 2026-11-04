//! The GL renderer: imports client buffers as textures (driver buffers through `EGL_WAYLAND_BUFFER_WL`, dma-bufs
//! through `EGL_EXT_image_dma_buf_import`, shm by upload) and draws them into an offscreen 1080p framebuffer.
//! C3 swaps the framebuffer for a scan-out buffer behind the same [`Output`] trait.

use std::collections::HashMap;
use std::ffi::{CString, c_void};
use std::io;
use std::time::{Duration, Instant};

use wayland_backend::server::ObjectId;
use wayland_server::Resource;
use wayland_server::protocol::wl_buffer::WlBuffer;

mod boxdim;
mod previews;
mod readback;

use super::egl::{self, EGLImage, EGLSync, Egl, GLint, GLuint, Platform};
use crate::Rect;
use crate::buffer::{BufferKindRef, kind_of};
use crate::output::{FrameClock, Image, Output, Presented, RenderStats};
use crate::scene::{HoleVeil, Plan, Posed};
use crate::surface::{SurfaceId, Surfaces};
use crate::transition::{FRect, VEIL_RGB};

/// Which global serves the clients' GPU buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferPath {
    /// The driver's own `mali_buffer_sharing`, after `eglBindWaylandDisplayWL` (X2 r8b, proven on the set).
    Driver,
    /// The compositor's own global: it keeps the dma-buf fd (C3's direct scan-out needs it) and imports it.
    Own,
}

/// A warm fence that has not signalled by then is given up on: the frame is drawn anyway.
const WARM_LIMIT: Duration = Duration::from_millis(100);
/// How long one `bury` call may spend freeing driver imports.
const BURY_BUDGET: Duration = Duration::from_millis(2);
/// Retired textures that may wait for a transition's end; past it they are freed at once.
const GRAVEYARD_MAX: usize = 12;
/// Most `warm:` lines logged (three per client start on the set; a measurement, not a watch).
const WARM_LINES: u32 = 120;

struct Tex {
    /// Kept for liveness: the entry goes when the client destroys the buffer (`prune`).
    buffer: WlBuffer,
    tex: GLuint,
    image: EGLImage,
    size: (i32, i32),
    y_invert: bool,
    /// Shm textures are re-uploaded on damage; imported ones never change.
    uploaded: bool,
    /// An import of the driver's own buffer (`EGL_WAYLAND_BUFFER_WL`), the kind whose first GPU job is slow on the set.
    external: bool,
    /// The fence behind the warm draw (`prepare`), while it has not signalled: the driver is still busy with the
    /// buffer. `None` once it signalled, or for a buffer imported by a frame.
    warm_sync: Option<EGLSync>,
    /// A host's stand-in for the driver's busy fence (`hold_warm`, tests): warming until released.
    held: bool,
    prepared: Instant,
    /// What `prepare` cost the loop for this buffer: the import and the warm draw's submission (each in us).
    import_us: u32,
    submit_us: u32,
}

/// A render target: the offscreen one `Gl::new` makes (id 0) or a dma-buf page added for scan-out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TargetId(pub usize);

struct Target {
    fbo: GLuint,
    tex: GLuint,
    image: EGLImage,
    /// Memory row 0 is the screen's top: true for scan-out pages (KMS shows row 0 at the top of the panel), false
    /// for the offscreen FBO, whose row 0 is the bottom as GL draws it and whose capture flips.
    top_down: bool,
}

/// The program that draws a window in a transition, with its uniforms and a blank texture to bind when it draws a flat colour.
struct Xf {
    program: GLuint,
    a_pos: GLuint,
    a_uv: GLuint,
    u_clip: GLint,
    u_mix: GLint,
    u_color: GLint,
    blank: GLuint,
}

pub struct Gl {
    /// The thread that links the engine's programs, while it runs (`attach_ui_when_ready`). Declared before `egl` so that it is dropped
    /// first: it uses the display `egl` ends.
    #[cfg(feature = "ui")]
    ui_compile: Option<crate::ui::compile::Compile>,
    egl: Egl,
    width: u32,
    height: u32,
    targets: Vec<Target>,
    current: TargetId,
    program: GLuint,
    a_pos: GLuint,
    a_uv: GLuint,
    /// The `u_dim` uniform's location and the value it holds (1 between frames: [`Gl::set_dim`]).
    u_dim: egl::GLint,
    dim: f32,
    xf: Xf,
    textures: HashMap<ObjectId, Tex>,
    /// Textures of buffers that died, destroyed later (`bury`): freeing a driver import stalls the caller until the GPU
    /// is done with the last frame that read it (8.9-18.8 ms renders at X1's disconnect on the fourth and fifth gates).
    graveyard: Vec<Tex>,
    /// A transition is moving: `bury` waits ([`Gl::defer_frees`]).
    frees_deferred: bool,
    /// A 16x16 target the first draw from a fresh import goes into (`prepare`), made on first use: `(fbo, texture)`.
    warm: WarmSlot,
    bgra_upload: bool,
    /// The rows of the rectangle an shm upload sends, kept between frames: the frame path allocates nothing once it is as big as the
    /// largest damage (a fresh buffer of the rectangle's size each frame is an `mmap` and a `munmap` of up to 8 MB above glibc's 128 KiB).
    upload_rows: Vec<u8>,
    clock: FrameClock,
    path: BufferPath,
    import_failures: u64,
    gl_errors: u64,
    /// A page copy raised a GL error: the pages draw all they owe from then on (`copy_from_page`).
    page_copy_failed: bool,
    /// The pointer's sprite, uploaded once (RGBA, premultiplied) the first time the cursor shows.
    cursor_tex: Option<GLuint>,
    rects: Vec<Rect>,
    redraw_all: bool,
    /// The page holding the last composed frame, whole, while there is one (none before the first composed frame and after
    /// a direct one): what a frame copies a surface's rectangle from while the driver is still busy with the surface's new
    /// buffer (`render_rects`), so the frame goes out on time and the content follows a refresh or two later. Without it the
    /// tick holds the frame instead (`warming`).
    standin_source: Option<TargetId>,
    /// Surface eligible for whole-frame stand-in popup suppression.
    standin_root: Option<SurfaceId>,
    standin_source_root: Option<SurfaceId>,
    standin_plan_root: Option<SurfaceId>,
    /// Scratch for a stand-in whose source page is also its destination.
    standin_scratch: Option<TargetId>,
    /// The surfaces the last composed frame stood in for, with the rectangle each covered: owed once their buffers are warm
    /// (`standin_due`, `owed_standins`). Rebuilt by every composed frame.
    standins: Vec<(SurfaceId, Rect)>,
    standins_next: Vec<(SurfaceId, Rect)>,
    /// The tiles the frame being drawn copied from the stand-in page: the page owes them again (`Ring::owe`).
    standin_tiles: crate::damage::Tiles,
    warm_lines: u32,
    /// The app switcher's program and the scaled copies of its pictures (`gl/previews.rs`).
    previews: previews::Previews,
    /// The box dim's gained program, map texture and sample pass (`gl/boxdim.rs`).
    boxdim: boxdim::BoxDim,
    /// The scratch target a screenshot or recording reads a page back through (`grab_target`): its index among the targets and its size.
    /// Made when the first picture is wanted, given back when the capture is over (`release_grab`), so nothing is held while none runs.
    grab_scratch: Option<(TargetId, u32, u32)>,
    /// The pixel pack buffers a recording's frames are read through, a frame behind (`gl/readback.rs`).
    readback: readback::Readback,
    /// The Rust Home (the engine's stack 0) is not drawn in these frames: an app's card covers it (`Plan::home_hidden`, `set_home_hidden`).
    home_hidden: bool,
    #[cfg(feature = "ui")]
    ui: Option<crate::ui::Ui>,
    #[cfg(feature = "ui")]
    ui_frame: Option<lumatv_render::host::FrameCost>,
    #[cfg(feature = "ui")]
    ui_stats: UiStats,
    /// The engine's programs, linked once and handed from one engine to the next (`lumatv_render::programs`).
    #[cfg(feature = "ui")]
    ui_programs: std::sync::Arc<lumatv_render::programs::Cache>,
}

/// What the UI's frames added up to since the start: the summary's account of the engine (a TV measurement reads it).
#[cfg(feature = "ui")]
#[derive(Default)]
struct UiStats {
    frames: u64,
    /// Frames that drew nothing of the UI (nothing changed in the owed rectangles, or a direct frame).
    empty: u64,
    region_px: u64,
    damage_px: u64,
    draw_calls: u64,
    layers: u64,
    cpu_us: u64,
    cpu_us_max: u32,
    gl_errors: u64,
}

#[cfg(feature = "ui")]
impl UiStats {
    fn add(&mut self, cost: &lumatv_render::host::FrameCost) {
        self.frames += 1;
        self.empty += u64::from(cost.region_px == 0);
        self.region_px += cost.region_px;
        self.damage_px += cost.damage_px;
        self.draw_calls += u64::from(cost.draw_calls);
        self.layers += u64::from(cost.layers);
        self.cpu_us += u64::from(cost.cpu_us);
        self.cpu_us_max = self.cpu_us_max.max(cost.cpu_us);
        self.gl_errors += u64::from(cost.gl_error.is_some());
    }

    /// Per-frame means over the frames that drew something, and the CPU's worst frame.
    fn line(&self) -> String {
        if self.frames == 0 {
            return String::new();
        }
        let drew = (self.frames - self.empty).max(1);
        format!(
            "ui frames {} (empty {}) px/frame {} changed {} draws/frame {} layers/frame {} cpu {} us/frame max {} us gl errors {}",
            self.frames,
            self.empty,
            self.region_px / drew,
            self.damage_px / drew,
            self.draw_calls / drew,
            self.layers / drew,
            self.cpu_us / self.frames,
            self.cpu_us_max,
            self.gl_errors
        )
    }
}

const VS: &str = "attribute vec2 a_pos; attribute vec2 a_uv; varying vec2 v_uv;\nvoid main() { gl_Position = vec4(a_pos, 0.0, 1.0); v_uv = a_uv; }\n";
/// `u_dim` scales the colour and leaves the alpha: the buffers are premultiplied, so a dimmed layer is the same layer with less light
/// (the OLED care's dim, `oledcare`); 1 for everything else.
const FS: &str = "precision mediump float; uniform sampler2D u_tex; uniform float u_dim; varying vec2 v_uv;\nvoid main() { vec4 c = texture2D(u_tex, v_uv); gl_FragColor = vec4(c.rgb * u_dim, c.a); }\n";
/// The fragment shader of a window in a transition (`transition.rs`): the picture, or a flat colour (a veil), through a rounded rectangle (the
/// window's mask, in framebuffer pixels; the same signed-distance rule as `transition::mask_coverage`, which the software renderer draws), at an
/// opacity and a brightness. Premultiplied, like everything else drawn here. The pixel's own position needs full precision (a 1920 wide target
/// is 2 pixels to a step in half precision).
const XF_FS: &str = "#ifdef GL_FRAGMENT_PRECISION_HIGH\nprecision highp float;\n#else\nprecision mediump float;\n#endif\n\
uniform sampler2D u_tex; uniform vec4 u_clip; uniform vec4 u_mix; uniform vec4 u_color; varying vec2 v_uv;\n\
void main() {\n\
  vec4 c = mix(texture2D(u_tex, v_uv), vec4(u_color.rgb, 1.0), u_mix.w);\n\
  vec2 hs = 0.5 * (u_clip.zw - u_clip.xy);\n\
  vec2 q = abs(gl_FragCoord.xy - 0.5 * (u_clip.xy + u_clip.zw)) - hs + vec2(u_mix.x);\n\
  float d = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - u_mix.x;\n\
  float m = clamp(0.5 - d, 0.0, 1.0);\n\
  gl_FragColor = vec4(c.rgb * u_mix.z, c.a) * (u_mix.y * m);\n\
}\n";

impl Gl {
    /// Opens the driver, binds the Wayland display when `path` is `Driver`, and makes the offscreen target.
    pub fn new(driver: &str, platform: Platform, width: u32, height: u32, path: BufferPath, wl_display: Option<*mut c_void>) -> io::Result<Gl> {
        let egl = Egl::open(driver, platform)?;
        log::info!("egl: {} {} on {}", egl.vendor, egl.version, egl.renderer);
        log::debug!("egl extensions: {}", egl.extensions);
        if path == BufferPath::Driver {
            let Some(dpy) = wl_display else {
                return Err(egl::other("the driver buffer path needs the C libwayland display (aarch64 build)"));
            };
            // SAFETY: the pointer is the backend's live wl_display (main.rs).
            unsafe { egl.bind_wayland_display(dpy)? };
            log::info!("egl: eglBindWaylandDisplayWL ok, the driver serves mali_buffer_sharing on this socket");
        }
        let (fbo, target) = offscreen_target(&egl, width, height)?;
        let a = &egl.api;
        // SAFETY: a context is current; the program is checked by `link`.
        let (program, a_pos, a_uv, u_dim) = unsafe {
            let program = link(&egl, VS, FS)?;
            let a_pos = (a.glGetAttribLocation)(program, c"a_pos".as_ptr());
            let a_uv = (a.glGetAttribLocation)(program, c"a_uv".as_ptr());
            (a.glUseProgram)(program);
            (a.glUniform1i)((a.glGetUniformLocation)(program, c"u_tex".as_ptr()), 0);
            let u_dim = (a.glGetUniformLocation)(program, c"u_dim".as_ptr());
            (a.glUniform1f)(u_dim, 1.0);
            (program, a_pos as GLuint, a_uv as GLuint, u_dim)
        };
        // SAFETY: as above; the blank texture is one transparent texel.
        let xf = unsafe {
            let program = link(&egl, VS, XF_FS)?;
            let a_pos = (a.glGetAttribLocation)(program, c"a_pos".as_ptr());
            let a_uv = (a.glGetAttribLocation)(program, c"a_uv".as_ptr());
            (a.glUseProgram)(program);
            (a.glUniform1i)((a.glGetUniformLocation)(program, c"u_tex".as_ptr()), 0);
            let mut blank = 0;
            (a.glGenTextures)(1, &mut blank);
            (a.glBindTexture)(egl::GL_TEXTURE_2D, blank);
            set_params(a);
            (a.glTexImage2D)(egl::GL_TEXTURE_2D, 0, egl::GL_RGBA as i32, 1, 1, 0, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, [0u8; 4].as_ptr() as *const c_void);
            Xf {
                program,
                a_pos: a_pos as GLuint,
                a_uv: a_uv as GLuint,
                u_clip: (a.glGetUniformLocation)(program, c"u_clip".as_ptr()),
                u_mix: (a.glGetUniformLocation)(program, c"u_mix".as_ptr()),
                u_color: (a.glGetUniformLocation)(program, c"u_color".as_ptr()),
                blank,
            }
        };
        // SAFETY: back to the frame's own program.
        unsafe { (a.glUseProgram)(program) };
        let bgra_upload = egl.has_gl_extension("GL_EXT_texture_format_BGRA8888");
        Ok(Gl {
            #[cfg(feature = "ui")]
            ui_compile: None,
            egl,
            width,
            height,
            targets: vec![Target { fbo, tex: target, image: std::ptr::null_mut(), top_down: false }],
            current: TargetId(0),
            program,
            a_pos,
            a_uv,
            u_dim,
            dim: 1.0,
            xf,
            textures: HashMap::new(),
            graveyard: Vec::new(),
            frees_deferred: false,
            warm: WarmSlot::default(),
            bgra_upload,
            upload_rows: Vec::new(),
            clock: FrameClock::new(),
            path,
            import_failures: 0,
            gl_errors: 0,
            page_copy_failed: false,
            cursor_tex: None,
            rects: Vec::new(),
            redraw_all: false,
            standin_source: None,
            standin_root: None,
            standin_source_root: None,
            standin_plan_root: None,
            standin_scratch: None,
            standins: Vec::new(),
            standins_next: Vec::new(),
            standin_tiles: crate::damage::Tiles::new(width, height),
            warm_lines: 0,
            previews: previews::Previews::default(),
            boxdim: boxdim::BoxDim::default(),
            grab_scratch: None,
            readback: readback::Readback::default(),
            home_hidden: false,
            #[cfg(feature = "ui")]
            ui: None,
            #[cfg(feature = "ui")]
            ui_frame: None,
            #[cfg(feature = "ui")]
            ui_stats: UiStats::default(),
            #[cfg(feature = "ui")]
            ui_programs: lumatv_render::programs::Cache::new(),
        })
    }

    /// Attaches a system-layer client before rendering starts; the returned socket carries its scene protocol. Whatever programs the
    /// process has not linked yet are linked on the calling thread, which takes the set's Mali seconds: the thread must not be the
    /// loop's while the loop runs ([`Gl::attach_ui_when_ready`] is the attach for that).
    #[cfg(feature = "ui")]
    pub fn attach_ui(&mut self, driver: &str, system_z: i32, shm: std::path::PathBuf) -> io::Result<std::os::unix::net::UnixStream> {
        if self.ui.is_some() {
            return Err(io::Error::other("a UI client is already attached"));
        }
        // SAFETY: this renderer owns the current context, and drops the UI before that context.
        let (ui, app) = unsafe { crate::ui::Ui::new(driver, self.width, self.height, system_z, shm, &self.ui_programs) }?;
        self.ui = Some(ui);
        for target in &mut self.targets {
            target.top_down = true;
        }
        Ok(app)
    }

    /// Attaches the engine without linking a program on the calling thread, which is the loop's when the compositor runs: the first
    /// call starts a thread that links them in a context sharing this one's (`ui::compile`), and the calls until it has ended answer
    /// `WouldBlock`, for the caller to ask again at its next tick. What is left to do is the engine's own state, a few milliseconds.
    /// An engine that finds a program missing from the thread's cache is refused, not given the link: [INFERENCE: not seen on the set]
    /// that would be a driver that does not share its programs between contexts, and the loop would be held for 1.5 s.
    #[cfg(feature = "ui")]
    pub fn attach_ui_when_ready(&mut self, driver: &str, system_z: i32, shm: std::path::PathBuf) -> io::Result<std::os::unix::net::UnixStream> {
        // The thread's result is taken before anything is attached, even when the cache already says it is complete: the cache is
        // filled by the thread's last act, and the engine takes from it only after that thread has ended.
        if self.ui_compile.is_some() || !self.ui_programs.complete() {
            self.link_ui_programs(driver)?;
        }
        let began = Instant::now();
        let app = self.attach_ui(driver, system_z, shm).map_err(|e| {
            if e.kind() == io::ErrorKind::Unsupported {
                io::Error::other(format!("{e}; the kinds are kept, and the next start of the compositor attaches the engine before its loop runs"))
            } else {
                e
            }
        })?;
        log::info!("ui: the engine attached in {:.1} ms on the loop thread", began.elapsed().as_secs_f64() * 1e3);
        Ok(app)
    }

    /// Starts the thread that links the engine's programs when none runs, and takes its result when it has ended: `Ok` once they are in the
    /// cache, `WouldBlock` while the thread runs, the thread's error (the next call tries again) when it failed.
    #[cfg(feature = "ui")]
    fn link_ui_programs(&mut self, driver: &str) -> io::Result<()> {
        let Some(compile) = &mut self.ui_compile else {
            let (width, height) = ((self.width / 4).max(64), (self.height / 4).max(64));
            let cache = std::sync::Arc::clone(&self.ui_programs);
            self.ui_compile = Some(crate::ui::compile::Compile::start(self.egl.sharing(), driver, width, height, cache)?);
            log::info!("ui: linking the engine's programs on a thread of its own; the loop is not held for them");
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        };
        let Some(result) = compile.poll() else {
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        };
        self.ui_compile = None;
        let made = result.map_err(|e| io::Error::other(format!("the engine's programs: {e}")))?;
        log::info!(
            "ui: {} programs linked on a thread of its own in {} ms, drawn once in {} ms{}",
            made.programs,
            made.link_ms,
            made.warm_ms,
            made.gl_error.map_or(String::new(), |e| format!(" (GL error 0x{e:x})"))
        );
        Ok(())
    }

    #[cfg(feature = "ui")]
    pub fn ui(&mut self) -> Option<&mut crate::ui::Ui> {
        self.ui.as_mut()
    }

    /// Drops the engine (the client is gone or is being restarted): its layers go, and the next frame draws everything without them.
    pub fn detach_ui(&mut self) {
        #[cfg(feature = "ui")]
        if self.ui.take().is_some() {
            self.redraw_all = true;
        }
    }

    /// Finishes the engine's handshake once the client has connected, without waiting (`Output::ui_handshake`).
    pub fn ui_try_handshake(&mut self) -> io::Result<bool> {
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui {
            return ui.try_finish_handshake();
        }
        Ok(false)
    }

    pub fn has_ui(&self) -> bool {
        #[cfg(feature = "ui")]
        {
            self.ui.is_some()
        }
        #[cfg(not(feature = "ui"))]
        {
            false
        }
    }

    /// Whether the frame about to be planned draws the Rust Home: not while an app's card covers it (`Plan::home_hidden`). A change of the answer
    /// in either direction is a frame that draws all of its page again, with the engine told that what it showed is not what the pages hold: the pages
    /// have Home's pixels or the black under the card as the frames before had them, and the frame after the change must not trust that.
    pub fn set_home_hidden(&mut self, hidden: bool) {
        if hidden == self.home_hidden {
            return;
        }
        self.home_hidden = hidden;
        self.redraw_all = true;
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui {
            ui.forget();
        }
    }

    pub fn plan_ui(&mut self, _damage: &mut crate::damage::Tiles) -> io::Result<()> {
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui {
            ui.plan(lumatv_render::now_ns(), _damage, self.home_hidden)?;
        }
        Ok(())
    }

    #[cfg(feature = "ui")]
    pub fn take_ui_frame(&mut self) -> Option<lumatv_render::host::FrameCost> {
        self.ui_frame.take()
    }

    #[cfg(feature = "ui")]
    pub fn ui_presented(&mut self, at: u64, cost: &lumatv_render::host::FrameCost) {
        self.ui_stats.add(cost);
        if let Some(ui) = &mut self.ui {
            ui.presented(at, cost);
        }
    }

    /// The UI's frames so far, for the summary line: how much of the output each drew and what each cost the CPU. Empty
    /// without a UI frame.
    pub fn ui_stats_line(&self) -> String {
        #[cfg(feature = "ui")]
        {
            self.ui_stats.line()
        }
        #[cfg(not(feature = "ui"))]
        {
            String::new()
        }
    }

    /// Whether the planned frame has a layer over the windows (the system layer shows something): a full-screen window
    /// cannot be scanned out directly under it. Meaningful after `plan_ui`.
    pub fn ui_over_windows(&self) -> bool {
        #[cfg(feature = "ui")]
        {
            self.ui.as_ref().is_some_and(|ui| ui.over_windows())
        }
        #[cfg(not(feature = "ui"))]
        {
            false
        }
    }

    /// Whether something of the system layer is over Home in the planned frame or was a few frames before (`Engine::over_home`):
    /// the pages copy what they owe from the page drawn last only then (`kms::frames`). Meaningful after `plan_ui`.
    pub fn ui_over_home(&self) -> bool {
        #[cfg(feature = "ui")]
        {
            self.ui.as_ref().is_some_and(|ui| ui.over_home())
        }
        #[cfg(not(feature = "ui"))]
        {
            false
        }
    }

    /// Whether an animation of the engine's runs after the frame just drawn: the next frame is due at the next refresh.
    pub fn ui_animating(&self) -> bool {
        #[cfg(feature = "ui")]
        {
            self.ui.as_ref().is_some_and(|ui| ui.animating())
        }
        #[cfg(not(feature = "ui"))]
        {
            false
        }
    }

    /// The rectangles a page owes as the planned UI frame is cheapest drawn in ([`crate::ui::Ui::merge`]): covers of some of them,
    /// holding all they did and apart from one another, where one pass over the engine's layers costs less than a pass each.
    /// Without an engine they stay as they are. Meaningful after `plan_ui`.
    pub fn merge_ui_rects(&mut self, _rects: &mut Vec<Rect>) {
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui
            && !self.home_hidden
        {
            ui.merge(_rects);
        }
    }

    /// Copies `rects` of page `source` into the current target as they are (a framebuffer blit, no filter): what a page owes from
    /// the frames before the one being drawn is what the page drawn last holds there, so the frame draws only what it changes
    /// (`kms::frames`). `false` when nothing was copied: the driver has no blit, the pages are not alike, or a copy raised a GL error
    /// (once is enough: no page copies after it).
    pub fn copy_from_page(&mut self, source: TargetId, rects: &[Rect]) -> bool {
        let Some(blit) = self.egl.opt.glBlitFramebuffer.filter(|_| !self.page_copy_failed) else { return false };
        let (Some(from), Some(to)) = (self.targets.get(source.0), self.targets.get(self.current.0)) else { return false };
        if source == self.current || from.top_down != to.top_down {
            return false;
        }
        let (from, to, top_down) = (from.fbo, to.fbo, to.top_down);
        // An error left from before the copy is the frame's, counted as `render_rects` counts its own.
        if self.egl.gl_error() != egl::GL_NO_ERROR {
            self.gl_errors += 1;
        }
        // SAFETY: GL state calls and blits between two framebuffers this renderer made, on the current context.
        unsafe {
            let a = &self.egl.api;
            (a.glDisable)(egl::GL_SCISSOR_TEST);
            (a.glBindFramebuffer)(egl::GL_READ_FRAMEBUFFER, from);
            (a.glBindFramebuffer)(egl::GL_DRAW_FRAMEBUFFER, to);
            for r in rects.iter().filter(|r| !r.is_empty()) {
                // GL counts rows from row 0: the bottom offscreen, the top on a scan-out page (as `scissor` does).
                let y = if top_down { r.y } else { self.height as i32 - r.bottom() };
                blit(r.x, y, r.right(), y + r.h, r.x, y, r.right(), y + r.h, egl::GL_COLOR_BUFFER_BIT, egl::GL_NEAREST as egl::GLenum);
            }
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, to);
        }
        let err = self.egl.gl_error();
        if err != egl::GL_NO_ERROR {
            log::warn!("page copy: GL error {err:#x}; pages draw all they owe from now on");
            self.page_copy_failed = true;
            return false;
        }
        true
    }

    /// Ends the planned UI frame without drawing it (the window below is scanned out directly, and nothing of the UI
    /// shows): the app still gets its receipt when that frame reaches the panel.
    pub fn skip_ui_frame(&mut self) -> io::Result<()> {
        self.finish_ui()
    }

    /// The engine's second half: the frame's receipt is kept for the presentation, and its GL errors count with the
    /// compositor's (the engine clears the error flag as it reads it, so `render_rects` would not see them).
    fn finish_ui(&mut self) -> io::Result<()> {
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui {
            let cost = ui.finish()?;
            ui.tidy();
            if let Some(err) = cost.gl_error {
                self.gl_errors += 1;
                log::warn!("gl error 0x{err:x} in the system UI's frame");
            }
            self.ui_frame = Some(cost);
        }
        Ok(())
    }

    /// Whether the next frame must draw all of its target (a ring forgets what its pages hold): the system UI went away,
    /// and the pages show its layers wherever nothing has been drawn since.
    pub fn take_redraw_all(&mut self) -> bool {
        std::mem::take(&mut self.redraw_all)
    }

    /// The sprite's texture: made on first use. RGBA is uploaded as RGBA whatever the shm path uses, so this
    /// never depends on the BGRA extension.
    fn cursor_texture(&mut self) -> GLuint {
        if let Some(t) = self.cursor_tex {
            return t;
        }
        let bytes = super::cursor::sprite();
        let a = &self.egl.api;
        let mut t = 0;
        // SAFETY: GL object creation with a current context; the upload reads exactly SIZE*SIZE*4 bytes.
        unsafe {
            (a.glGenTextures)(1, &mut t);
            (a.glBindTexture)(egl::GL_TEXTURE_2D, t);
            set_params(a);
            (a.glTexImage2D)(
                egl::GL_TEXTURE_2D,
                0,
                egl::GL_RGBA as i32,
                super::cursor::SIZE,
                super::cursor::SIZE,
                0,
                egl::GL_RGBA,
                egl::GL_UNSIGNED_BYTE,
                bytes.as_ptr() as *const c_void,
            );
        }
        self.cursor_tex = Some(t);
        t
    }

    pub fn egl(&self) -> &Egl {
        &self.egl
    }

    pub fn path(&self) -> BufferPath {
        self.path
    }

    /// The driver's `wl_resource` for a buffer: only exists on the C backend.
    fn resource_ptr(buffer: &WlBuffer) -> Option<*mut c_void> {
        #[cfg(target_arch = "aarch64")]
        {
            let p = buffer.id().as_ptr();
            if p.is_null() { None } else { Some(p as *mut c_void) }
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            let _ = buffer;
            None
        }
    }

    /// Makes `buffer` a texture if it is not one yet: `Some(true)` for a fresh import, `Some(false)` for one already
    /// made, `None` when the driver refuses it (counted in `import_failures`).
    fn import(&mut self, buffer: &WlBuffer) -> Option<bool> {
        let id = buffer.id();
        let a = &self.egl.api;
        if !self.textures.contains_key(&id) {
            let tex = match kind_of(buffer) {
                BufferKindRef::Shm(shm) => {
                    let mut t = 0;
                    // In ES 2 a TexSubImage's format must equal the texture's own (ES 2.0 §3.7.2), so a BGRA upload
                    // needs a BGRA texture (GL_EXT_texture_format_BGRA8888 allows it as the internal format).
                    // Uploading BGRA into an RGBA texture was GL_INVALID_OPERATION on the set (first light
                    // 2026-10-25, shm case); Mesa tolerates it, so the host test cannot see the difference.
                    let fmt = if self.bgra_upload { egl::GL_BGRA_EXT } else { egl::GL_RGBA };
                    // SAFETY: GL object creation with a current context.
                    unsafe {
                        (a.glGenTextures)(1, &mut t);
                        (a.glBindTexture)(egl::GL_TEXTURE_2D, t);
                        set_params(a);
                        (a.glTexImage2D)(egl::GL_TEXTURE_2D, 0, fmt as i32, shm.width, shm.height, 0, fmt, egl::GL_UNSIGNED_BYTE, std::ptr::null());
                    }
                    Tex {
                        buffer: buffer.clone(),
                        tex: t,
                        image: std::ptr::null_mut(),
                        size: (shm.width, shm.height),
                        y_invert: true,
                        uploaded: false,
                        external: false,
                        warm_sync: None,
                        held: false,
                        prepared: Instant::now(),
                        import_us: 0,
                        submit_us: 0,
                    }
                }
                BufferKindRef::Dmabuf(d) => {
                    let plane = d.planes.first()?;
                    let image = match self.egl.image_from_dmabuf(plane.fd.as_raw_fd(), d.width, d.height, d.format, plane.offset, plane.stride, d.modifier) {
                        Ok(i) => i,
                        Err(e) => {
                            self.import_failures += 1;
                            log::warn!("import: {e}");
                            return None;
                        }
                    };
                    Tex {
                        buffer: buffer.clone(),
                        tex: bind_image(a, image),
                        image,
                        size: (d.width, d.height),
                        y_invert: d.y_invert,
                        uploaded: true,
                        external: false,
                        warm_sync: None,
                        held: false,
                        prepared: Instant::now(),
                        import_us: 0,
                        submit_us: 0,
                    }
                }
                BufferKindRef::External => {
                    let Some(ptr) = Self::resource_ptr(buffer) else {
                        self.import_failures += 1;
                        return None;
                    };
                    // SAFETY: a live wl_resource of the attached wl_buffer, created by the bound driver.
                    let Some((size, y_invert)) = (unsafe { self.egl.query_wayland_buffer(ptr) }) else {
                        self.import_failures += 1;
                        log::warn!("import: eglQueryWaylandBufferWL failed: 0x{:x}", self.egl.error());
                        return None;
                    };
                    // SAFETY: as above.
                    let image = match unsafe { self.egl.image_from_wayland_buffer(ptr) } {
                        Ok(i) => i,
                        Err(e) => {
                            self.import_failures += 1;
                            log::warn!("import: {e}");
                            return None;
                        }
                    };
                    Tex {
                        buffer: buffer.clone(),
                        tex: bind_image(a, image),
                        image,
                        size,
                        y_invert,
                        uploaded: true,
                        external: true,
                        warm_sync: None,
                        held: false,
                        prepared: Instant::now(),
                        import_us: 0,
                        submit_us: 0,
                    }
                }
            };
            self.textures.insert(id, tex);
            return Some(true);
        }
        Some(false)
    }

    /// A buffer's first-use costs, paid when the client commits it rather than in the frame that first draws it: the
    /// import (the driver's mapping of a client buffer) and one draw from it into the warm target, started with a fence
    /// and not waited for. `true` when the buffer was new. On the set a new client's second and third buffers each cost
    /// the frame that first drew them a refresh (X1's first second on the third to fifth gates: 13.7-18.8 ms passes with
    /// nothing of it in the draw itself), so the cost belongs in the clients' pass, before the pacer's latch. The fence
    /// is kept: while it has not signalled the driver is still busy with the buffer, and a frame drawn then waits on
    /// it (`warming`).
    pub fn prepare(&mut self, buffer: &WlBuffer) -> bool {
        let began = Instant::now();
        if self.import(buffer) != Some(true) {
            return false;
        }
        let imported = Instant::now();
        let (tex, y_invert) = {
            let t = &self.textures[&buffer.id()];
            (t.tex, t.y_invert)
        };
        let Some((fbo, _)) = self.warm_target() else { return true };
        let a = &self.egl.api;
        // SAFETY: state calls on the current context; the warm framebuffer and the program are this renderer's.
        unsafe {
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
            (a.glViewport)(0, 0, 16, 16);
            (a.glUseProgram)(self.program);
            (a.glDisable)(egl::GL_SCISSOR_TEST);
            (a.glDisable)(egl::GL_BLEND);
        }
        // The whole output rectangle fills the 16x16 viewport: every texel row is sampled.
        self.draw_quad(tex, &Rect::new(0, 0, self.width as i32, self.height as i32), y_invert);
        // An EGL fence, not a native one: exporting a native fence waits for the blob's submission thread to take the
        // draw (5.1 ms once at a session's start on the tenth gate, in the clients' pass), the EGL fence's flush does
        // not. With that thread in SCHED_FIFO the warm's own fence signalled 0.1 ms after the draw (tenth gate, read
        // through a native fence's kernel stamp), so the tick that polls it is what the line's time measures.
        let kicked = self.egl.kick_fence();
        let submitted = Instant::now();
        if let Some(t) = self.textures.get_mut(&buffer.id()) {
            t.import_us = (imported - began).as_micros() as u32;
            t.submit_us = (submitted - imported).as_micros() as u32;
            t.prepared = submitted;
        }
        if let Some(sync) = kicked {
            // SAFETY: a fence this renderer just created; the flush starts the job, the sync lives on in the entry.
            if unsafe { self.egl.fence_signalled(sync, true) } {
                // SAFETY: as above, done with.
                unsafe { self.egl.destroy_fence(sync) };
            } else if let Some(t) = self.textures.get_mut(&buffer.id()) {
                t.warm_sync = Some(sync);
            }
        } else {
            // SAFETY: plain call.
            unsafe { (self.egl.api.glFlush)() };
        }
        let err = self.egl.gl_error();
        if err != egl::GL_NO_ERROR {
            self.gl_errors += 1;
            log::warn!("gl error 0x{err:x} warming a buffer");
        }
        true
    }

    /// The 16x16 target the warm draws and the acquire samples go into, made on first use.
    fn warm_target(&mut self) -> Option<(GLuint, GLuint)> {
        self.warm.get_or_make(|| offscreen_target(&self.egl, 16, 16))
    }

    /// The acquire barrier for a buffer the display is about to read directly (`scanout`): one texel of it is sampled into the warm target, so that
    /// a job exists that waits for whatever the dma-buf's fences say its client is still doing, and the commit's fence ([`Gl::export_fence`]) is
    /// behind that job. `false` when the buffer cannot be imported or sampled.
    pub fn acquire(&mut self, buffer: &WlBuffer) -> bool {
        if self.import(buffer).is_none() {
            return false;
        }
        let (tex, y_invert) = {
            let t = &self.textures[&buffer.id()];
            (t.tex, t.y_invert)
        };
        let Some((fbo, _)) = self.warm_target() else { return false };
        let a = &self.egl.api;
        // SAFETY: state calls on the current context; the warm framebuffer and the program are this renderer's.
        unsafe {
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
            (a.glViewport)(0, 0, 16, 16);
            (a.glUseProgram)(self.program);
            (a.glEnable)(egl::GL_SCISSOR_TEST);
            (a.glScissor)(0, 0, 1, 1);
            (a.glDisable)(egl::GL_BLEND);
        }
        self.set_dim(1.0);
        self.draw_quad(tex, &Rect::new(0, 0, self.width as i32, self.height as i32), y_invert);
        // The same start as a frame's (LumaShow lesson 3: a fence, not glFlush, is what starts the work on Mali).
        if let Some(sync) = self.egl.kick_fence() {
            // SAFETY: a fence this renderer just created, consumed here.
            unsafe { self.egl.wait_fence(sync, 0) };
        } else {
            // SAFETY: plain call.
            unsafe { (self.egl.api.glFlush)() };
        }
        let err = self.egl.gl_error();
        if err != egl::GL_NO_ERROR {
            self.gl_errors += 1;
            log::warn!("gl error 0x{err:x} sampling a buffer to scan out");
            return false;
        }
        true
    }

    /// Whether a frame must not be drawn now: a driver buffer's warm draw is still running and there is no page to stand
    /// in from (`standin_source`). On the set the first job on a driver buffer (LG's `mali_buffer_sharing` imports:
    /// Home's, HDMI 2's, Netflix's player's) held the frame that came while it ran for 10-16 ms, in the draw or in the
    /// fence export (sixth gate); with a stand-in page the frame is drawn without that buffer (`render_rects`). Fences
    /// of other buffers are polled and dropped too; a fence that has not signalled after 100 ms is given up on. Each
    /// buffer's warm gets one line when its fence is seen done, with what `prepare` cost the loop for it and, for a native
    /// fence, the kernel's own time of the signal.
    pub fn warming(&mut self) -> bool {
        let mut busy = false;
        for t in self.textures.values_mut() {
            if t.held {
                busy = true;
                continue;
            }
            let Some(sync) = t.warm_sync else { continue };
            // SAFETY: a fence this renderer created, still alive.
            let done = unsafe { self.egl.fence_signalled(sync, false) };
            let stale = !done && t.prepared.elapsed() > WARM_LIMIT;
            if done || stale {
                // SAFETY: as above, destroyed once.
                unsafe { self.egl.destroy_fence(sync) };
                t.warm_sync = None;
                if stale {
                    log::warn!("warm draw of a buffer not done after {} ms; drawing anyway", WARM_LIMIT.as_millis());
                } else if self.warm_lines < WARM_LINES {
                    self.warm_lines += 1;
                    log::info!(
                        "warm: {} buffer {} {}x{}: import {:.1} ms, warm draw submitted in {:.1} ms, its fence seen done {:.1} ms after (polled at the ticks)",
                        if t.external { "driver" } else { "dma-buf" },
                        t.buffer.id().protocol_id(),
                        t.size.0,
                        t.size.1,
                        t.import_us as f64 / 1000.0,
                        t.submit_us as f64 / 1000.0,
                        t.prepared.elapsed().as_secs_f64() * 1000.0
                    );
                }
            } else if t.external {
                busy = true;
            }
        }
        busy && self.standin_source.is_none()
    }

    /// Whether a frame is owed for a surface a frame stood in for: its buffer is warm now and nothing else is warming, so
    /// the frame draws it for real (`owed_standins` says where).
    pub fn standin_due(&self) -> bool {
        !self.standins.is_empty() && !self.textures.values().any(|t| t.external && (t.warm_sync.is_some() || t.held))
    }

    /// Adds the rectangles the last composed frame stood in for to a frame's damage.
    pub fn owed_standins(&self, tiles: &mut crate::damage::Tiles) {
        for (_, r) in &self.standins {
            tiles.add(r);
        }
    }

    /// The tiles the last `render_rects` copied from the stand-in page instead of drawing.
    pub fn standin_tiles(&self) -> &crate::damage::Tiles {
        &self.standin_tiles
    }

    /// The page frames copy a warming surface's rectangle from: the one the last composed frame went into, whole.
    /// `None` when no page holds the last frame (before the first composed one, after a direct one): frames are held
    /// while a buffer warms, and nothing is owed to a stand-in any more.
    pub fn set_standin_source(&mut self, page: Option<TargetId>) {
        self.standin_source = page;
        self.standin_source_root = if page.is_some() && (self.standins.is_empty() || self.standin_source_root == self.standin_plan_root) {
            self.standin_plan_root
        } else {
            None
        };
        if page.is_none() {
            self.standins.clear();
            #[cfg(feature = "ui")]
            if let Some(ui) = &mut self.ui {
                ui.set_standin_popup_suppressed(false);
            }
        }
    }

    /// Suppresses fresh system popups over a whole-frame stand-in from the same qualified root.
    pub fn prepare_ui_standin(&mut self, plan: &Plan) -> bool {
        self.standin_plan_root = match plan.items.as_slice() {
            [item] if Some(item.surface) == self.standin_root
                && !item.ghost && item.pose.is_none()
                && item.rect == Rect::new(0, 0, self.width as i32, self.height as i32) => self.standin_root,
            _ => None,
        };
        #[cfg(feature = "ui")]
        {
            let suppress = self.ui.is_some() && self.standin_source.is_some_and(|p| p.0 < self.targets.len() && self.targets[p.0].tex != 0)
                && self.standin_plan_root.is_some()
                && self.standin_plan_root == self.standin_source_root
                && plan.items.iter().any(|item| {
                    Some(item.surface) == self.standin_plan_root
                        && item.pose.is_none()
                        && self.textures.get(&item.buffer.id()).is_some_and(|t| t.external && (t.warm_sync.is_some() || t.held))
                });
            if let Some(ui) = &mut self.ui {
                ui.set_standin_popup_suppressed(suppress);
            }
            suppress
        }
        #[cfg(not(feature = "ui"))]
        { false }
    }

    /// A host's stand-in for a driver that is busy with `buffer` after its commit (tests): the texture counts as a
    /// driver buffer warming until `held` is false again.
    pub fn hold_warm(&mut self, buffer: &WlBuffer, held: bool) {
        if let Some(t) = self.textures.get_mut(&buffer.id()) {
            t.external = true;
            t.held = held;
        }
    }

    /// The texture for a buffer, imported now if the commit did not (`prepare`); `new` says which. Shm content is
    /// uploaded here, where the frame's damage is known.
    fn texture_for(&mut self, buffer: &WlBuffer, damage: Option<&[Rect]>, rect: &Rect) -> Option<(GLuint, (i32, i32), bool, bool)> {
        let new = self.import(buffer)?;
        let id = buffer.id();
        let mut rows = std::mem::take(&mut self.upload_rows);
        let a = &self.egl.api;
        let entry = self.textures.get_mut(&id)?;
        if let BufferKindRef::Shm(shm) = kind_of(buffer) {
            // Upload what changed (clients draw into the same shm buffer between commits).
            let full = Rect::new(0, 0, shm.width, shm.height);
            let regions: Vec<Rect> = match (entry.uploaded, damage) {
                (false, _) | (true, None) => vec![full],
                (true, Some(d)) => d.iter().map(|r| r.translated(-rect.x, -rect.y).intersect(&full)).filter(|r| !r.is_empty()).collect(),
            };
            let bgra = self.bgra_upload;
            let tex = entry.tex;
            shm.pool.with_bytes(|bytes| {
                // SAFETY: the texture exists; the pixel pointer stays valid while the pool lock is held.
                unsafe {
                    (a.glBindTexture)(egl::GL_TEXTURE_2D, tex);
                    (a.glPixelStorei)(egl::GL_UNPACK_ALIGNMENT, 4);
                    for r in &regions {
                        if !pack_shm_rows(&mut rows, bytes, shm.offset as usize, shm.stride as usize, shm.format, r, bgra) {
                            return;
                        }
                        (a.glTexSubImage2D)(
                            egl::GL_TEXTURE_2D,
                            0,
                            r.x,
                            r.y,
                            r.w,
                            r.h,
                            if bgra { egl::GL_BGRA_EXT } else { egl::GL_RGBA },
                            egl::GL_UNSIGNED_BYTE,
                            rows.as_ptr() as *const c_void,
                        );
                    }
                }
            });
            entry.uploaded = true;
        }
        self.upload_rows = rows;
        Some((entry.tex, entry.size, entry.y_invert, new))
    }

    /// Sets the brightness the next draws are made at (the program is in use); a call that changes nothing makes no GL call.
    fn set_dim(&mut self, dim: f32) {
        if dim != self.dim {
            // SAFETY: a uniform of the program the renderer has in use.
            unsafe { (self.egl.api.glUniform1f)(self.u_dim, dim) };
            self.dim = dim;
        }
    }

    fn draw_quad(&self, tex: GLuint, rect: &Rect, y_invert: bool) {
        self.draw_quad_with(self.a_pos, self.a_uv, tex, rect, y_invert);
    }

    /// [`Gl::draw_quad`] with the vertex attributes of the program in use (the plain one's by default, the box dim's gained one through
    /// `gl/boxdim.rs`).
    fn draw_quad_with(&self, a_pos: GLuint, a_uv: GLuint, tex: GLuint, rect: &Rect, y_invert: bool) {
        let w = self.width as f32;
        let h = self.height as f32;
        let x0 = rect.x as f32 / w * 2.0 - 1.0;
        let x1 = rect.right() as f32 / w * 2.0 - 1.0;
        // Top of the screen is +1 in NDC on the offscreen target, so its row 0 is the screen bottom (capture flips).
        // A scan-out page is shown row 0 first at the top of the panel, so there the top is -1. Composed KMS frames
        // were upside down on the set until this (2026-10-25 gate captures); direct scan-out never went through here.
        let (y0, y1) = if self.targets[self.current.0].top_down {
            (rect.y as f32 / h * 2.0 - 1.0, rect.bottom() as f32 / h * 2.0 - 1.0)
        } else {
            (1.0 - rect.y as f32 / h * 2.0, 1.0 - rect.bottom() as f32 / h * 2.0)
        };
        let (vt, vb) = if y_invert { (0.0f32, 1.0f32) } else { (1.0, 0.0) };
        let verts: [f32; 16] = [x0, y0, 0.0, vt, x1, y0, 1.0, vt, x0, y1, 0.0, vb, x1, y1, 1.0, vb];
        let a = &self.egl.api;
        // SAFETY: client-side vertex arrays of 16 floats, program and texture bound.
        unsafe {
            (a.glActiveTexture)(egl::GL_TEXTURE0);
            (a.glBindTexture)(egl::GL_TEXTURE_2D, tex);
            (a.glVertexAttribPointer)(a_pos, 2, egl::GL_FLOAT, 0, 16, verts.as_ptr() as *const c_void);
            (a.glVertexAttribPointer)(a_uv, 2, egl::GL_FLOAT, 0, 16, verts.as_ptr().add(2) as *const c_void);
            (a.glEnableVertexAttribArray)(a_pos);
            (a.glEnableVertexAttribArray)(a_uv);
            (a.glDrawArrays)(egl::GL_TRIANGLE_STRIP, 0, 4);
        }
    }

    /// A window's picture, or a flat colour, through a pose: `quad` is where the picture goes, `clip` the rounded rectangle it shows through, at
    /// `alpha` and `shade` (`transition.rs`). Draws with the pose program, which the caller made current ([`Gl::use_xf`]), into the scissor it set.
    #[allow(clippy::too_many_arguments)]
    fn draw_posed(&self, tex: GLuint, quad: &FRect, clip: &FRect, radius: f32, alpha: f32, shade: f32, y_invert: bool, solid: Option<[f32; 3]>) {
        let (w, h) = (self.width as f32, self.height as f32);
        let top_down = self.targets[self.current.0].top_down;
        let (x0, x1) = (quad.x / w * 2.0 - 1.0, quad.right() / w * 2.0 - 1.0);
        let (y0, y1) = if top_down { (quad.y / h * 2.0 - 1.0, quad.bottom() / h * 2.0 - 1.0) } else { (1.0 - quad.y / h * 2.0, 1.0 - quad.bottom() / h * 2.0) };
        let (vt, vb) = if y_invert { (0.0f32, 1.0f32) } else { (1.0, 0.0) };
        let verts: [f32; 16] = [x0, y0, 0.0, vt, x1, y0, 1.0, vt, x0, y1, 0.0, vb, x1, y1, 1.0, vb];
        // The mask in framebuffer rows: counted from the bottom offscreen, from the top on a scan-out page (as `scissor`).
        let (cy0, cy1) = if top_down { (clip.y, clip.bottom()) } else { (h - clip.bottom(), h - clip.y) };
        let radius = radius.clamp(0.0, (clip.w.min(clip.h) * 0.5).max(0.0));
        let [r, g, b] = solid.unwrap_or([0.0; 3]);
        let a = &self.egl.api;
        // SAFETY: uniforms of the current pose program and client-side vertex arrays of 16 floats; the texture is bound here.
        unsafe {
            (a.glUniform4f)(self.xf.u_clip, clip.x, cy0, clip.right(), cy1);
            (a.glUniform4f)(self.xf.u_mix, radius, alpha, shade, if solid.is_some() { 1.0 } else { 0.0 });
            (a.glUniform4f)(self.xf.u_color, r, g, b, 1.0);
            (a.glActiveTexture)(egl::GL_TEXTURE0);
            (a.glBindTexture)(egl::GL_TEXTURE_2D, if solid.is_some() { self.xf.blank } else { tex });
            (a.glVertexAttribPointer)(self.xf.a_pos, 2, egl::GL_FLOAT, 0, 16, verts.as_ptr() as *const c_void);
            (a.glVertexAttribPointer)(self.xf.a_uv, 2, egl::GL_FLOAT, 0, 16, verts.as_ptr().add(2) as *const c_void);
            (a.glEnableVertexAttribArray)(self.xf.a_pos);
            (a.glEnableVertexAttribArray)(self.xf.a_uv);
            (a.glDrawArrays)(egl::GL_TRIANGLE_STRIP, 0, 4);
        }
    }

    /// A veil: the flat dark fill of an arriving card's hole, inside `rects` (output pixels), through the card's mask.
    fn draw_veil(&self, v: &HoleVeil, bounds: &Rect, rects: &[Rect]) {
        self.use_xf();
        let rgb = [VEIL_RGB[0] as f32 / 255.0, VEIL_RGB[1] as f32 / 255.0, VEIL_RGB[2] as f32 / 255.0];
        for r in rects {
            let c = bounds.intersect(r);
            if c.is_empty() {
                continue;
            }
            self.scissor(&c);
            self.draw_posed(0, &v.quad, &v.clip, v.radius, v.alpha, 1.0, false, Some(rgb));
        }
        self.use_plain();
    }

    fn draw_home_over_wallpaper(&mut self, plan: &Plan, rects: &[Rect]) -> io::Result<()> {
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui {
            ui.draw_under_over(self.targets[self.current.0].fbo, rects)?;
        }
        self.draw_home_dim(plan.home_dim, rects);
        Ok(())
    }

    fn draw_home_dim(&self, dim: f32, rects: &[Rect]) {
        if dim <= 0.0 { return; }
        let full = FRect::new(0.0, 0.0, self.width as f32, self.height as f32);
        self.use_xf();
        for r in rects {
            self.scissor(r);
            self.draw_posed(0, &full, &full, 0.0, dim, 1.0, false, Some([0.0; 3]));
        }
        self.use_plain();
    }

    /// `dim` is the OLED care's (the item's own, 1 for nearly everything): it multiplies the pose's brightness.
    fn draw_pose_item(&self, tex: GLuint, p: &Posed, dim: f32, y_invert: bool, rects: &[Rect]) -> bool {
        let mut drawn = false;
        self.use_xf();
        for r in rects {
            let clip = p.bounds().intersect(r);
            if clip.is_empty() {
                continue;
            }
            self.scissor(&clip);
            self.draw_posed(tex, &p.quad, &p.clip, p.radius, p.alpha, p.shade * dim, y_invert, None);
            drawn = true;
        }
        self.use_plain();
        drawn
    }

    /// The pose program is the current one (a window in a transition); [`Gl::use_plain`] puts the frame's own back.
    fn use_xf(&self) {
        // SAFETY: plain state call with a program made at creation.
        unsafe { (self.egl.api.glUseProgram)(self.xf.program) };
    }

    fn use_plain(&self) {
        // SAFETY: as above.
        unsafe { (self.egl.api.glUseProgram)(self.program) };
    }

    fn scissor(&self, r: &Rect) {
        // GL scissor counts rows from row 0: the bottom of the screen offscreen, the top on a scan-out page.
        let y = if self.targets[self.current.0].top_down { r.y } else { self.height as i32 - r.bottom() };
        // SAFETY: plain state call.
        unsafe { (self.egl.api.glScissor)(r.x, y, r.w, r.h) };
    }

    pub fn import_failures(&self) -> u64 {
        self.import_failures
    }

    /// The modifiers the GPU driver lists for each format a scan-out page may have (`afbc::Probe`): `None` for the driver's lack of the query.
    pub fn dmabuf_modifiers(&self) -> Option<Vec<crate::afbc::EglModifier>> {
        use crate::buffer::fourcc;
        let mut all = Vec::new();
        let mut asked = false;
        for f in [fourcc::ABGR8888, fourcc::XBGR8888, fourcc::ARGB8888, fourcc::XRGB8888] {
            if let Some(list) = self.egl.dmabuf_modifiers(f) {
                asked = true;
                all.extend(list.into_iter().map(|(modifier, external_only)| crate::afbc::EglModifier { fourcc: f, modifier, external_only }));
            }
        }
        asked.then_some(all)
    }

    /// Makes a dma-buf (a scan-out page) a render target: imported as an EGLImage, bound to a texture and an FBO.
    pub fn add_dmabuf_target(&mut self, fd: i32, width: i32, height: i32, fourcc: u32, pitch: u32, modifier: u64) -> io::Result<TargetId> {
        let image = self.egl.image_from_dmabuf(fd, width, height, fourcc, 0, pitch, modifier)?;
        let a = &self.egl.api;
        let tex = bind_image(a, image);
        let mut fbo = 0;
        // SAFETY: GL object creation with a current context; the texture exists.
        let status = unsafe {
            (a.glGenFramebuffers)(1, &mut fbo);
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
            (a.glFramebufferTexture2D)(egl::GL_FRAMEBUFFER, egl::GL_COLOR_ATTACHMENT0, egl::GL_TEXTURE_2D, tex, 0);
            (a.glCheckFramebufferStatus)(egl::GL_FRAMEBUFFER)
        };
        if status != egl::GL_FRAMEBUFFER_COMPLETE {
            // SAFETY: the objects made above, bound to nothing that outlives this call.
            unsafe {
                (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, 0);
                (a.glDeleteFramebuffers)(1, &fbo);
                (a.glDeleteTextures)(1, &tex);
                self.egl.destroy_image(image);
            }
            return Err(egl::other(format!("dma-buf target framebuffer incomplete: 0x{status:x}")));
        }
        // Defined-texture rule: a new page starts opaque black.
        // SAFETY: as above.
        unsafe {
            (a.glDisable)(egl::GL_SCISSOR_TEST);
            (a.glClearColor)(0.0, 0.0, 0.0, 1.0);
            (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
            (a.glFinish)();
        }
        self.targets.push(Target { fbo, tex, image, top_down: true });
        Ok(TargetId(self.targets.len() - 1))
    }

    /// Frees a page's render target (a page that was replaced): its image, texture and framebuffer object go, and the slot stays, dead, so the other
    /// targets keep their ids. Nothing may draw into or read from it afterwards; the stand-in and the current target are moved off it.
    pub fn free_target(&mut self, id: TargetId) {
        let dead = Target { fbo: 0, tex: 0, image: std::ptr::null_mut(), top_down: true };
        let old = std::mem::replace(&mut self.targets[id.0], dead);
        // SAFETY: objects this renderer made, no longer in use (the caller finished the GPU's work on the page first).
        unsafe {
            (self.egl.api.glDeleteFramebuffers)(1, &old.fbo);
            (self.egl.api.glDeleteTextures)(1, &old.tex);
            self.egl.destroy_image(old.image);
        }
        if self.current == id {
            self.current = TargetId(0);
        }
        if self.standin_source == Some(id) {
            self.set_standin_source(None);
        }
    }

    /// Pays the driver's first-use costs before any client does: a quad drawn through the frame's program from an
    /// uploaded texture and, when `page` names a dma-buf target, from that page's EGLImage into the current target
    /// (the first draw from an imported image cost X1's first frame a refresh on the third gate). Returns how long it
    /// took, GPU included.
    /// While on, [`Gl::bury`] frees nothing (a transition is moving).
    pub fn defer_frees(&mut self, on: bool) {
        self.frees_deferred = on;
    }

    /// Retired textures waiting for [`Gl::bury`].
    pub fn retired(&self) -> usize {
        self.graveyard.len()
    }

    pub fn warm_up(&mut self, page: Option<TargetId>) -> std::time::Duration {
        let began = std::time::Instant::now();
        let full = Rect::new(0, 0, self.width as i32, self.height as i32);
        let a = &self.egl.api;
        let mut tex = 0;
        // SAFETY: GL object creation and draws on the current context with this renderer's program; the pixels are
        // the 4 bytes given.
        unsafe {
            (a.glGenTextures)(1, &mut tex);
            (a.glBindTexture)(egl::GL_TEXTURE_2D, tex);
            set_params(a);
            let fmt = if self.bgra_upload { egl::GL_BGRA_EXT } else { egl::GL_RGBA };
            (a.glTexImage2D)(egl::GL_TEXTURE_2D, 0, fmt as i32, 1, 1, 0, fmt, egl::GL_UNSIGNED_BYTE, [0u8, 0, 0, 255].as_ptr() as *const c_void);
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, self.targets[self.current.0].fbo);
            (a.glViewport)(0, 0, self.width as i32, self.height as i32);
            (a.glUseProgram)(self.program);
            (a.glEnable)(egl::GL_SCISSOR_TEST);
            (a.glEnable)(egl::GL_BLEND);
            (a.glBlendFunc)(egl::GL_ONE, egl::GL_ONE_MINUS_SRC_ALPHA);
        }
        self.scissor(&full);
        self.draw_quad(tex, &full, true);
        if let Some(page) = page.filter(|p| p.0 < self.targets.len() && *p != self.current) {
            self.draw_quad(self.targets[page.0].tex, &full, false);
        }
        // The pose program's first use compiles and links its pipeline in the driver: paid here (a quad of a texture and of a flat colour,
        // through a mask), not by the first transition. A corner of the target only; the clear below puts it back.
        let corner = FRect::new(0.0, 0.0, 64.0, 36.0);
        self.use_xf();
        self.scissor(&Rect::new(0, 0, 64, 36));
        self.draw_posed(tex, &corner, &corner, 8.0, 0.5, 0.8, true, None);
        self.draw_posed(tex, &corner, &corner, 8.0, 0.5, 1.0, true, Some([0.1, 0.1, 0.1]));
        self.use_plain();
        self.scissor(&full);
        // SAFETY: the texture was made above; the clear puts the target back to black; glFinish waits for the GPU.
        unsafe {
            (a.glClearColor)(0.0, 0.0, 0.0, 1.0);
            (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
            (a.glFinish)();
            (a.glDeleteTextures)(1, &tex);
        }
        let err = self.egl.gl_error();
        if err != egl::GL_NO_ERROR {
            log::warn!("gl warm-up: error 0x{err:x}");
        }
        began.elapsed()
    }

    /// Adds a page in the context's own memory, laid out as a scan-out page is (row 0 at the top): a host's stand-in for a
    /// dma-buf page, so that the ring's frame logic runs without a device.
    pub fn add_target(&mut self) -> io::Result<TargetId> {
        let (fbo, tex) = offscreen_target(&self.egl, self.width, self.height)?;
        self.targets.push(Target { fbo, tex, image: std::ptr::null_mut(), top_down: true });
        Ok(TargetId(self.targets.len() - 1))
    }

    /// Selects the target the next `render` draws into.
    pub fn set_target(&mut self, target: TargetId) {
        if target.0 < self.targets.len() {
            self.current = target;
        }
    }

    /// Snapshots an aliased stand-in before the destination is modified.
    pub fn snapshot_aliased_standin(&mut self) -> io::Result<()> {
        let Some(source) = self.standin_source.filter(|source| *source == self.current) else { return Ok(()) };
        let scratch = match self.standin_scratch {
            Some(id) => id,
            None => {
                let id = self.add_target()?;
                self.standin_scratch = Some(id);
                id
            }
        };
        let (tex, top_down) = (self.targets[source.0].tex, self.targets[source.0].top_down);
        let previous = std::mem::replace(&mut self.current, scratch);
        let full = Rect::new(0, 0, self.width as i32, self.height as i32);
        if self.egl.gl_error() != egl::GL_NO_ERROR { self.gl_errors += 1; }
        // SAFETY: both textures and framebuffer belong to this current context.
        unsafe {
            let a = &self.egl.api;
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, self.targets[scratch.0].fbo);
            (a.glViewport)(0, 0, self.width as i32, self.height as i32);
            (a.glUseProgram)(self.program);
            (a.glDisable)(egl::GL_SCISSOR_TEST);
            (a.glDisable)(egl::GL_BLEND);
        }
        self.set_dim(1.0);
        self.draw_quad(tex, &full, top_down);
        self.current = previous;
        let err = self.egl.gl_error();
        if err != egl::GL_NO_ERROR {
            self.gl_errors += 1;
            return Err(egl::other(format!("standin snapshot: GL error {err:#x}")));
        }
        self.standin_source = Some(scratch);
        Ok(())
    }

    /// A native fence fd for the work submitted so far (`EGL_ANDROID_native_fence_sync`, which the blob on the set
    /// has): the atomic commit's `IN_FENCE_FD`. `None` without the extension.
    pub fn export_fence(&mut self) -> Option<std::os::fd::OwnedFd> {
        let (Some(create), Some(dup), Some(destroy)) = (self.egl.opt.eglCreateSyncKHR, self.egl.opt.eglDupNativeFenceFDANDROID, self.egl.opt.eglDestroySyncKHR)
        else {
            return None;
        };
        const EGL_SYNC_NATIVE_FENCE_ANDROID: u32 = 0x3144;
        // SAFETY: a native fence sync with no attributes; the fd is owned from here; the sync is destroyed.
        unsafe {
            let sync = create(self.egl.display, EGL_SYNC_NATIVE_FENCE_ANDROID, std::ptr::null());
            if sync.is_null() {
                return None;
            }
            (self.egl.api.glFlush)();
            let fd = dup(self.egl.display, sync);
            destroy(self.egl.display, sync);
            if fd < 0 { None } else { Some(<std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd)) }
        }
    }

    /// Finishes every queued GL command (before a CPU read of a page).
    pub fn finish(&self) {
        // SAFETY: plain call on the current context.
        unsafe { (self.egl.api.glFinish)() };
    }

    /// The target a page is read back through, `w` x `h`, made now or kept from the last picture of that size.
    fn grab_scratch(&mut self, w: u32, h: u32) -> io::Result<TargetId> {
        if let Some((id, sw, sh)) = self.grab_scratch
            && (sw, sh) == (w, h)
        {
            return Ok(id);
        }
        let (fbo, tex) = offscreen_target(&self.egl, w, h)?;
        let target = Target { fbo, tex, image: std::ptr::null_mut(), top_down: true };
        let id = match self.grab_scratch {
            Some((id, ..)) => {
                let old = std::mem::replace(&mut self.targets[id.0], target);
                // SAFETY: objects this renderer made and no longer uses.
                unsafe {
                    (self.egl.api.glDeleteFramebuffers)(1, &old.fbo);
                    (self.egl.api.glDeleteTextures)(1, &old.tex);
                }
                id
            }
            None => {
                self.targets.push(target);
                TargetId(self.targets.len() - 1)
            }
        };
        self.grab_scratch = Some((id, w, h));
        Ok(id)
    }

    /// The picture `source` holds, at `size`, drawn into the scratch target: one textured quad from the page (the GPU does the scaling), its alpha
    /// then made opaque (the page's pixels are premultiplied, so a hole for video reads black), with the scratch target's framebuffer left bound
    /// for the read. Nothing of the frame path's state is touched: no page is drawn into, no texture uploaded, no damage taken.
    fn grab_draw(&mut self, source: TargetId, size: (u32, u32)) -> io::Result<()> {
        let (w, h) = size;
        if source.0 >= self.targets.len() || w == 0 || h == 0 {
            return Err(egl::other(format!("grab: page {} at {w}x{h}", source.0)));
        }
        let scratch = self.grab_scratch(w, h)?;
        let (tex, top_down) = (self.targets[source.0].tex, self.targets[source.0].top_down);
        let previous = std::mem::replace(&mut self.current, scratch);
        let full = Rect::new(0, 0, self.width as i32, self.height as i32);
        // SAFETY: GL state calls on the current context and this renderer's objects.
        unsafe {
            let a = &self.egl.api;
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, self.targets[scratch.0].fbo);
            (a.glViewport)(0, 0, w as i32, h as i32);
            (a.glUseProgram)(self.program);
            (a.glDisable)(egl::GL_SCISSOR_TEST);
            (a.glDisable)(egl::GL_BLEND);
        }
        // A scan-out page holds its row 0 at the screen's top, as the scratch target does: the picture goes in as it is.
        // The page already holds whatever the OLED care dimmed; copying it takes none of that light away again.
        self.set_dim(1.0);
        self.draw_quad(tex, &full, top_down);
        // SAFETY: as above; a clear with only the alpha channel writable.
        unsafe {
            let a = &self.egl.api;
            (a.glColorMask)(0, 0, 0, 1);
            (a.glClearColor)(0.0, 0.0, 0.0, 1.0);
            (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
            (a.glColorMask)(1, 1, 1, 1);
        }
        self.current = previous;
        Ok(())
    }

    /// The picture `source` holds, at `size`, read into `spare` (see [`Gl::grab_draw`]) before this returns: the loop waits for the GPU.
    pub fn grab_target(&mut self, source: TargetId, size: (u32, u32), mut spare: Vec<u8>) -> io::Result<Image> {
        let (w, h) = size;
        self.grab_draw(source, size)?;
        let len = w as usize * h as usize * 4;
        spare.clear();
        spare.reserve(len);
        // SAFETY: the read fills exactly w*h*4 bytes of the spare's capacity (GL_PACK_ALIGNMENT 4 matches the rows of a 4-byte pixel); the length is
        // set only once no error says it did not.
        unsafe {
            let a = &self.egl.api;
            (a.glPixelStorei)(egl::GL_PACK_ALIGNMENT, 4);
            (a.glReadPixels)(0, 0, w as i32, h as i32, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, spare.as_mut_ptr() as *mut c_void);
        }
        let err = self.egl.gl_error();
        if err != egl::GL_NO_ERROR {
            return Err(egl::other(format!("grab: glReadPixels 0x{err:x}")));
        }
        // SAFETY: see above.
        unsafe { spare.set_len(len) };
        Ok(Image { width: w, height: h, rgba: spare })
    }

    /// A capture is over: the scratch target is given back (when it is the newest target, which it is unless a page was added since).
    pub fn release_grab(&mut self) {
        self.readback_free();
        let Some((id, ..)) = self.grab_scratch else { return };
        if id.0 + 1 != self.targets.len() {
            return;
        }
        if let Some(t) = self.targets.pop() {
            // SAFETY: objects this renderer made and no longer uses.
            unsafe {
                (self.egl.api.glDeleteFramebuffers)(1, &t.fbo);
                (self.egl.api.glDeleteTextures)(1, &t.tex);
            }
        }
        self.grab_scratch = None;
    }

    /// Draws the plan into the current target, but only inside `rects` (output pixels): the tiles a scan-out page
    /// owes, or the frame's damage bounds.
    pub fn render_rects(&mut self, plan: &Plan, surfaces: &Surfaces, rects: &[Rect]) -> io::Result<RenderStats> {
        let t0 = std::time::Instant::now();
        self.prune(surfaces, &plan.items);
        let mut stats = RenderStats {
            items: plan.items.len(),
            rects: rects.len() as u32,
            owed_px: rects.iter().map(|r| r.w.max(0) as u64 * r.h.max(0) as u64).sum(),
            ..Default::default()
        };
        // (the engine's stack 0 clears the owed rectangles and draws Home into them; with Home hidden under an app's card they are cleared here
        // as they are without an engine, to the opaque black that LG's compositor has under an app, and the card and the system layer go over it)
        let wall_home = plan.home_before > 0 && !self.home_hidden && self.has_ui();
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui
            && !self.home_hidden
            && !wall_home
        {
            ui.draw_under(self.targets[self.current.0].fbo, rects)?;
        }
        let fbo = self.targets[self.current.0].fbo;
        // SAFETY: GL state calls on the current context and this renderer's objects.
        unsafe {
            let a = &self.egl.api;
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
            (a.glViewport)(0, 0, self.width as i32, self.height as i32);
            (a.glUseProgram)(self.program);
            (a.glEnable)(egl::GL_SCISSOR_TEST);
            if !self.has_ui() || self.home_hidden || wall_home {
                (a.glClearColor)(0.0, 0.0, 0.0, 1.0);
                for r in rects {
                    self.scissor(r);
                    (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
                }
            }
            (a.glEnable)(egl::GL_BLEND);
            (a.glBlendFunc)(egl::GL_ONE, egl::GL_ONE_MINUS_SRC_ALPHA);
        }
        if !wall_home && !self.home_hidden && self.has_ui() {
            self.draw_home_dim(plan.home_dim, rects);
        }
        let (mut imports, mut import_us) = (0u32, 0u64);
        self.standin_tiles.clear();
        let mut standins = std::mem::take(&mut self.standins_next);
        standins.clear();
        let standin_page = self.standin_source.filter(|p| p.0 < self.targets.len());
        if plan.backdrop.is_none() {
            self.free_previews();
        }
        // The box dim's map on the app's pictures while one is engaged (`gl/boxdim.rs`); nothing of it otherwise.
        let gained = self.boxdim_frame(plan);
        for (index, item) in plan.items.iter().enumerate() {
            if wall_home && index == plan.home_before {
                self.draw_home_over_wallpaper(plan, rects)?;
            }
            if let Some(b) = plan.backdrop.as_ref().filter(|b| b.before == index) {
                self.draw_backdrop(b, rects);
            }
            self.clear_holes_before(plan, index, rects);
            let began = std::time::Instant::now();
            let Some((tex, _size, y_invert, new)) = self.texture_for(&item.buffer, plan.item_damage(item), &item.rect) else {
                stats.skipped += 1;
                continue;
            };
            if new {
                imports += 1;
                import_us += began.elapsed().as_micros() as u64;
            }
            // The driver is still busy with this buffer (its warm draw has not finished): a draw from it now would wait
            // on that, so the surface's rectangle is copied from the page holding the last frame instead, and the page
            // being drawn owes it again. The copy is opaque: the page already composes everything under the surface.
            let warming = self.textures.get(&item.buffer.id()).is_some_and(|t| t.external && (t.warm_sync.is_some() || t.held));
            // A picture on its way (a transition) whose buffer is still warming is not drawn this frame: what is under it shows, and the
            // next frame of the move draws it.
            if warming && item.pose.is_some() {
                stats.posed_waiting += 1;
                continue;
            }
            if let (true, Some(page)) = (warming, standin_page) {
                let full = Rect::new(0, 0, self.width as i32, self.height as i32);
                self.set_dim(1.0);
                // SAFETY: plain state calls on the current context.
                unsafe { (self.egl.api.glDisable)(egl::GL_BLEND) };
                for r in rects {
                    let clip = item.rect.intersect(r);
                    if clip.is_empty() {
                        continue;
                    }
                    // A page drawn again (a superseded frame's) already holds the last frame: nothing to copy.
                    if page != self.current {
                        self.scissor(&clip);
                        self.draw_quad(self.targets[page.0].tex, &full, true);
                    }
                    self.standin_tiles.add(&clip);
                }
                // SAFETY: as above.
                unsafe { (self.egl.api.glEnable)(egl::GL_BLEND) };
                if !self.standins.iter().any(|(s, _)| *s == item.surface) && self.warm_lines < WARM_LINES {
                    log::info!("standin: surface {} keeps the last frame's content while its new buffer warms", item.surface.0);
                }
                standins.push((item.surface, item.rect));
                stats.stood_in += 1;
                continue;
            }
            let mut drawn = false;
            self.set_dim(item.dim);
            if let Some(p) = &item.pose {
                drawn = self.draw_pose_item(tex, p, item.dim, y_invert, rects);
            } else {
                let through_map = gained && item.gain;
                // what the shell's overlay stands in for (`mask.rs`) is left undrawn: the page holds what was under it, with the video's hole
                // the video
                let blank = plan.blank_of(item);
                for r in rects {
                    let clip = item.rect.intersect(r);
                    if clip.is_empty() {
                        continue;
                    }
                    crate::mask::cut(clip, blank, &mut |piece| {
                        self.scissor(&piece);
                        if through_map {
                            self.draw_gained(tex, &item.rect, y_invert, item.dim);
                        } else {
                            self.draw_quad(tex, &item.rect, y_invert);
                        }
                        drawn = true;
                    });
                }
            }
            if drawn {
                stats.drawn += 1;
            }
        }
        if wall_home && plan.home_before == plan.items.len() {
            self.draw_home_over_wallpaper(plan, rects)?;
        }
        // an idle system layer leaves the backdrop above every item
        if let Some(b) = plan.backdrop.as_ref().filter(|b| b.before >= plan.items.len()) {
            self.draw_backdrop(b, rects);
        }
        self.set_dim(1.0);
        self.standins_next = std::mem::replace(&mut self.standins, standins);
        // (a hole is cleared as its item is reached, above: what is drawn above the item stays)
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui {
            ui.draw_over(fbo, rects)?;
        }
        self.finish_ui()?;
        #[cfg(feature = "ui")]
        if let Some(cost) = &self.ui_frame {
            stats.ui_draws = cost.draw_calls;
            stats.ui_layers = cost.layers;
        }
        if let Some(c) = plan.cursor {
            let tex = self.cursor_texture();
            for r in rects {
                let clip = c.intersect(r);
                if clip.is_empty() {
                    continue;
                }
                self.scissor(&clip);
                self.draw_quad(tex, &c, true);
            }
        }
        // Start the GPU work now (LumaShow lesson 3: a fence, not glFlush, is what starts it on Mali). The KMS
        // output exports its own native fence for the commit right after.
        if let Some(sync) = self.egl.kick_fence() {
            // SAFETY: a fence this renderer just created, consumed here.
            unsafe { self.egl.wait_fence(sync, 0) };
        } else {
            // SAFETY: plain call.
            unsafe { (self.egl.api.glFlush)() };
        }
        let err = self.egl.gl_error();
        if err != egl::GL_NO_ERROR {
            self.gl_errors += 1;
            log::warn!("gl error 0x{err:x} during render");
        }
        stats.gl_errors = self.gl_errors;
        stats.imports = imports;
        stats.render_us = t0.elapsed().as_micros() as u64;
        // A frame that cost more than half a refresh on the CPU is one the loop's own line will show as a stall; what
        // of it was the driver taking in a client's buffer for the first time is only known here (X1's first second
        // on the third gate: a 13.7 ms pass, a refresh lost).
        if stats.render_us > 4000 {
            log::info!(
                "render: {:.1} ms for {} items in {} rects, {} new imports in {:.1} ms",
                stats.render_us as f64 / 1000.0,
                plan.items.len(),
                rects.len(),
                imports,
                import_us as f64 / 1000.0
            );
        }
        Ok(stats)
    }

    /// Clears, inside `rects`, the holes that belong to item `index` of the plan to transparent black, so the display engine's video shows
    /// through what is under the item: LG's order (the punch-through is at z -1 inside its window), which keeps the item's own pixels and
    /// everything drawn above it.
    fn clear_holes_before(&mut self, plan: &Plan, index: usize, rects: &[Rect]) {
        for h in plan.holes.iter().filter(|h| h.before == index) {
            // An arriving card's veil lies over what is under the card (no clear) until it starts to fade away; then the hole is cleared
            // and the veil, less opaque, goes on the clear, so the video the display engine blends under it shows through what is left.
            if h.veil.is_none_or(|v| v.clear) {
                for r in rects {
                    let c = h.rect.intersect(r);
                    if c.is_empty() {
                        continue;
                    }
                    self.scissor(&c);
                    // SAFETY: GL state calls on the current context.
                    unsafe {
                        (self.egl.api.glClearColor)(0.0, 0.0, 0.0, 0.0);
                        (self.egl.api.glClear)(egl::GL_COLOR_BUFFER_BIT);
                    }
                }
            }
            if let Some(v) = &h.veil {
                self.draw_veil(v, &h.rect, rects);
            }
        }
    }

    /// Draws the system layer alone into the current target (a page with alpha for the overlay plane), inside `rects`:
    /// they are cleared to transparent and stack 1 is drawn over that, so the window under the plane shows through
    /// everywhere the UI is not. No window, no stack 0, no cursor.
    pub fn render_ui_layer(&mut self, rects: &[Rect]) -> io::Result<RenderStats> {
        let t0 = std::time::Instant::now();
        let mut stats = RenderStats { rects: rects.len() as u32, owed_px: rects.iter().map(|r| r.w.max(0) as u64 * r.h.max(0) as u64).sum(), ..Default::default() };
        let fbo = self.targets[self.current.0].fbo;
        // SAFETY: GL state calls on the current context and this renderer's objects.
        unsafe {
            let a = &self.egl.api;
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
            (a.glViewport)(0, 0, self.width as i32, self.height as i32);
            (a.glEnable)(egl::GL_SCISSOR_TEST);
            (a.glClearColor)(0.0, 0.0, 0.0, 0.0);
            for r in rects {
                self.scissor(r);
                (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
            }
        }
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui {
            ui.draw_over(fbo, rects)?;
        }
        self.finish_ui()?;
        if let Some(sync) = self.egl.kick_fence() {
            // SAFETY: a fence this renderer just created, consumed here.
            unsafe { self.egl.wait_fence(sync, 0) };
        } else {
            // SAFETY: plain call.
            unsafe { (self.egl.api.glFlush)() };
        }
        let err = self.egl.gl_error();
        if err != egl::GL_NO_ERROR {
            self.gl_errors += 1;
            log::warn!("gl error 0x{err:x} during the UI layer's draw");
        }
        stats.gl_errors = self.gl_errors;
        stats.render_us = t0.elapsed().as_micros() as u64;
        if stats.render_us > 4000 {
            log::info!("render: {:.1} ms for the UI layer in {} rects", stats.render_us as f64 / 1000.0, rects.len());
        }
        Ok(stats)
    }

    /// Retires textures of buffers the clients destroyed and no surface shows any more; `bury` frees them. A destroyed
    /// buffer that is still a surface's current one keeps its import: the surface goes on showing it until the client
    /// commits another (the driver's own buffers have no destroy callback here, so liveness is the only signal).
    pub fn prune(&mut self, surfaces: &Surfaces, drawn: &[crate::scene::Item]) {
        let dead: Vec<ObjectId> = self
            .textures
            .iter()
            .filter(|(id, t)| {
                !t.buffer.is_alive()
                    && !surfaces.iter().any(|s| s.current.buffer.as_ref().map(|b| b.id() == **id).unwrap_or(false))
                    // (a destroyed window's last picture that a transition still draws)
                    && !drawn.iter().any(|i| i.buffer.id() == **id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in dead {
            if let Some(t) = self.textures.remove(&id) {
                self.graveyard.push(t);
            }
        }
    }

    /// Frees retired textures, a few at a time. Called once the GPU is done with the last frame that could have read
    /// them (KMS: when the next page flip lands; offscreen: before the next frame), never between a frame's draw and
    /// its commit. Each free of a driver import stalls the caller until the driver has unmapped it: Netflix's buffers
    /// cost a 10.1 ms pass at its exit and WebAppMgr's first ones 13.2 ms (sixth gate), so a call stops after
    /// `BURY_BUDGET` and leaves the rest for the next one. Returns how many are left.
    pub fn bury(&mut self) -> usize {
        if self.graveyard.is_empty() {
            return 0;
        }
        // A move on the screen is no time for a free that stalls the loop on the driver (10 ms for a Netflix buffer, an app's last
        // swapchain): it waits for the move's end, unless so many have piled up that waiting costs memory.
        if self.frees_deferred && self.graveyard.len() < GRAVEYARD_MAX {
            return self.graveyard.len();
        }
        let began = Instant::now();
        let mut freed = 0;
        while let Some(t) = self.graveyard.pop() {
            // SAFETY: objects this renderer created, dropped once.
            unsafe { self.free(t) };
            freed += 1;
            if began.elapsed() >= BURY_BUDGET {
                break;
            }
        }
        let left = self.graveyard.len();
        if left > 0 || began.elapsed() >= BURY_BUDGET {
            log::info!("bury: {freed} freed in {:.1} ms, {left} left for the next pass", began.elapsed().as_secs_f64() * 1000.0);
        }
        left
    }

    /// # Safety
    /// Objects this renderer created, dropped once.
    unsafe fn free(&self, t: Tex) {
        // SAFETY: the caller's.
        unsafe {
            if let Some(sync) = t.warm_sync {
                self.egl.destroy_fence(sync);
            }
            (self.egl.api.glDeleteTextures)(1, &t.tex);
            self.egl.destroy_image(t.image);
        }
    }
}

fn set_params(a: &egl::Api) {
    // SAFETY: texture bound by the caller.
    unsafe {
        (a.glTexParameteri)(egl::GL_TEXTURE_2D, egl::GL_TEXTURE_MIN_FILTER, egl::GL_LINEAR);
        (a.glTexParameteri)(egl::GL_TEXTURE_2D, egl::GL_TEXTURE_MAG_FILTER, egl::GL_LINEAR);
        (a.glTexParameteri)(egl::GL_TEXTURE_2D, egl::GL_TEXTURE_WRAP_S, egl::GL_CLAMP_TO_EDGE);
        (a.glTexParameteri)(egl::GL_TEXTURE_2D, egl::GL_TEXTURE_WRAP_T, egl::GL_CLAMP_TO_EDGE);
    }
}

fn bind_image(a: &egl::Api, image: EGLImage) -> GLuint {
    let mut t = 0;
    // SAFETY: GL object creation with a current context; the image is valid.
    unsafe {
        (a.glGenTextures)(1, &mut t);
        (a.glBindTexture)(egl::GL_TEXTURE_2D, t);
        set_params(a);
        (a.glEGLImageTargetTexture2DOES)(egl::GL_TEXTURE_2D, image);
    }
    t
}

/// The warm target's slot: made on first use, and a refusal is remembered, so a driver that will not make it is asked once, not on every
/// direct-scanout attempt.
#[derive(Default)]
struct WarmSlot {
    made: Option<(GLuint, GLuint)>,
    refused: bool,
}

impl WarmSlot {
    fn get_or_make(&mut self, make: impl FnOnce() -> io::Result<(GLuint, GLuint)>) -> Option<(GLuint, GLuint)> {
        if self.made.is_none() && !self.refused {
            match make() {
                Ok(w) => self.made = Some(w),
                Err(e) => {
                    log::warn!("warm target: {e}");
                    self.refused = true;
                }
            }
        }
        self.made
    }
}

/// A render target in the context's own memory: a texture with a framebuffer over it, cleared to opaque black (the defined-texture
/// rule, LumaShow lesson 15: never driver noise). Returns `(fbo, texture)`.
fn offscreen_target(egl: &Egl, width: u32, height: u32) -> io::Result<(GLuint, GLuint)> {
    let a = &egl.api;
    // SAFETY: GL object creation with a current context; the framebuffer is checked.
    unsafe {
        let mut tex = 0;
        (a.glGenTextures)(1, &mut tex);
        (a.glBindTexture)(egl::GL_TEXTURE_2D, tex);
        (a.glTexImage2D)(egl::GL_TEXTURE_2D, 0, egl::GL_RGBA as i32, width as i32, height as i32, 0, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, std::ptr::null());
        (a.glTexParameteri)(egl::GL_TEXTURE_2D, egl::GL_TEXTURE_MIN_FILTER, egl::GL_NEAREST);
        (a.glTexParameteri)(egl::GL_TEXTURE_2D, egl::GL_TEXTURE_MAG_FILTER, egl::GL_NEAREST);
        let mut fbo = 0;
        (a.glGenFramebuffers)(1, &mut fbo);
        (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
        (a.glFramebufferTexture2D)(egl::GL_FRAMEBUFFER, egl::GL_COLOR_ATTACHMENT0, egl::GL_TEXTURE_2D, tex, 0);
        let status = (a.glCheckFramebufferStatus)(egl::GL_FRAMEBUFFER);
        if status != egl::GL_FRAMEBUFFER_COMPLETE {
            let gl_error = (a.glGetError)();
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, 0);
            (a.glDeleteFramebuffers)(1, &fbo);
            (a.glDeleteTextures)(1, &tex);
            return Err(egl::other(format!("offscreen framebuffer incomplete: 0x{status:x} (gl 0x{gl_error:x})")));
        }
        (a.glViewport)(0, 0, width as i32, height as i32);
        (a.glDisable)(egl::GL_SCISSOR_TEST);
        (a.glClearColor)(0.0, 0.0, 0.0, 1.0);
        (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
        Ok((fbo, tex))
    }
}

fn link(egl: &Egl, vs_src: &str, fs_src: &str) -> io::Result<GLuint> {
    let a = &egl.api;
    let compile = |kind: u32, src: &str| -> io::Result<GLuint> {
        let csrc = CString::new(src).unwrap();
        // SAFETY: standard shader compilation; the log buffer is sized and NUL-terminated by GL.
        unsafe {
            let s = (a.glCreateShader)(kind);
            let p = csrc.as_ptr();
            (a.glShaderSource)(s, 1, &p, std::ptr::null());
            (a.glCompileShader)(s);
            let mut ok = 0;
            (a.glGetShaderiv)(s, egl::GL_COMPILE_STATUS, &mut ok);
            if ok == 0 {
                let mut buf = vec![0u8; 512];
                let mut n = 0;
                (a.glGetShaderInfoLog)(s, 512, &mut n, buf.as_mut_ptr() as *mut _);
                return Err(egl::other(format!("shader: {}", String::from_utf8_lossy(&buf[..n.max(0) as usize]))));
            }
            Ok(s)
        }
    };
    let vs = compile(egl::GL_VERTEX_SHADER, vs_src)?;
    let fs = compile(egl::GL_FRAGMENT_SHADER, fs_src)?;
    // SAFETY: as above.
    unsafe {
        let p = (a.glCreateProgram)();
        (a.glAttachShader)(p, vs);
        (a.glAttachShader)(p, fs);
        (a.glLinkProgram)(p);
        let mut ok = 0;
        (a.glGetProgramiv)(p, egl::GL_LINK_STATUS, &mut ok);
        if ok == 0 {
            let mut buf = vec![0u8; 512];
            let mut n = 0;
            (a.glGetProgramInfoLog)(p, 512, &mut n, buf.as_mut_ptr() as *mut _);
            return Err(egl::other(format!("program: {}", String::from_utf8_lossy(&buf[..n.max(0) as usize]))));
        }
        Ok(p)
    }
}

use std::os::fd::AsRawFd;

impl Drop for Gl {
    fn drop(&mut self) {
        #[cfg(feature = "ui")]
        drop(self.ui.take());
        // SAFETY: objects this renderer created, dropped once; the EGL display outlives them until `egl` drops.
        unsafe {
            for t in std::mem::take(&mut self.textures).into_values().chain(std::mem::take(&mut self.graveyard)) {
                self.free(t);
            }
            if let Some((fbo, tex)) = self.warm.made {
                (self.egl.api.glDeleteFramebuffers)(1, &fbo);
                (self.egl.api.glDeleteTextures)(1, &tex);
            }
            for t in &self.targets {
                (self.egl.api.glDeleteFramebuffers)(1, &t.fbo);
                (self.egl.api.glDeleteTextures)(1, &t.tex);
                self.egl.destroy_image(t.image);
            }
        }
    }
}

impl Output for Gl {
    fn name(&self) -> &'static str {
        "gl"
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        match kind_of(buffer) {
            BufferKindRef::External => {
                let ptr = Self::resource_ptr(buffer)?;
                // SAFETY: a live wl_resource of a wl_buffer the bound driver created.
                unsafe { self.egl.query_wayland_buffer(ptr) }
            }
            BufferKindRef::Dmabuf(d) => Some(((d.width, d.height), d.y_invert)),
            BufferKindRef::Shm(s) => Some(((s.width, s.height), true)),
        }
    }

    fn prepare_buffer(&mut self, buffer: &WlBuffer) -> bool {
        self.prepare(buffer)
    }

    fn warming(&mut self) -> bool {
        Gl::warming(self)
    }

    fn ui_set_exclusion(&mut self, area: Option<Rect>) {
        #[cfg(not(feature = "ui"))]
        let _ = area;
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui {
            ui.set_exclusion(area);
        }
    }

    fn ui_set_standin_root(&mut self, root: Option<SurfaceId>) {
        self.standin_root = root;
    }

    fn defer_frees(&mut self, on: bool) {
        Gl::defer_frees(self, on);
    }

    fn retired(&self) -> usize {
        self.graveyard.len()
    }

    fn sample_begin(&mut self, plan: &Plan, _surfaces: &Surfaces) -> io::Result<bool> {
        Gl::sample_begin(self, plan)
    }

    fn sample_take(&mut self, sigs: &mut [crate::boxdim::Sig; crate::boxdim::CELLS]) -> io::Result<bool> {
        Gl::sample_take(self, sigs)
    }

    fn boxdim_warm(&mut self) -> io::Result<bool> {
        Gl::boxdim_warm(self)
    }

    fn forget_buffer(&mut self, buffer: &WlBuffer) {
        if let Some(t) = self.textures.remove(&buffer.id()) {
            self.graveyard.push(t);
        }
    }

    fn prepare_previews(&mut self, buffers: &[WlBuffer]) {
        Gl::prepare_previews(self, buffers);
    }

    fn ui_state(&mut self, state: &serde_json::Value) -> bool {
        #[cfg(feature = "ui")]
        { self.ui.as_mut().is_some_and(|ui| lumatv_render::silk_report::apply(ui.engine(), state)) }
        #[cfg(not(feature = "ui"))]
        { let _ = state; false }
    }

    fn ui_needs_frame(&mut self) -> io::Result<bool> {
        // A redraw the UI's going left owed is a frame whoever asks (the wake that saw the hang-up may not have drawn).
        if self.redraw_all {
            return Ok(true);
        }
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui {
            let now = lumatv_render::now_ns();
            if ui.pump()? {
                // The app is gone, and its layers with it: the next frame draws everything without them.
                log::error!("system UI: the app closed its socket; its layers are dropped");
                self.ui = None;
                self.redraw_all = true;
                return Ok(true);
            }
            // With `LUMATV_AHEAD=1`, a large commit (a screen made) is taken in now, in the time to the frame's latch, not in the frame.
            ui.engine().take_in_ahead(now);
            if ui.wants_frame(now) {
                return Ok(true);
            }
            ui.idle(now);
            return Ok(ui.wants_frame(now));
        }
        Ok(false)
    }

    fn ui_fd(&self) -> Option<std::os::fd::RawFd> {
        #[cfg(feature = "ui")]
        {
            self.ui.as_ref().and_then(|ui| ui.fd())
        }
        #[cfg(not(feature = "ui"))]
        {
            None
        }
    }

    fn ui_attach(&mut self, driver: &str, system_z: i32, shm: std::path::PathBuf) -> io::Result<std::os::unix::net::UnixStream> {
        #[cfg(feature = "ui")]
        {
            self.attach_ui_when_ready(driver, system_z, shm)
        }
        #[cfg(not(feature = "ui"))]
        {
            let _ = (driver, system_z, shm);
            Err(io::Error::other("built without feature ui"))
        }
    }

    fn ui_handshake(&mut self) -> io::Result<bool> {
        self.ui_try_handshake()
    }

    fn ui_detach(&mut self) {
        self.detach_ui();
    }

    fn ui_attached(&self) -> bool {
        self.has_ui()
    }

    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats> {
        // Offscreen there is no flip to wait for: the frame before this one is the last that could have read them.
        self.bury();
        self.set_home_hidden(plan.home_hidden);
        let mut tiles = plan.tiles.as_ref().cloned().unwrap_or_else(|| crate::damage::Tiles::new(self.width, self.height));
        tiles.add(&plan.damage);
        if self.take_redraw_all() {
            tiles.fill();
        }
        self.plan_ui(&mut tiles)?;
        let mut rects = std::mem::take(&mut self.rects);
        tiles.rects(&mut rects);
        let result = self.render_rects(plan, surfaces, &rects);
        self.rects = rects;
        result
    }

    fn stats_line(&self) -> String {
        self.ui_stats_line()
    }

    fn present(&mut self) -> io::Result<Presented> {
        #[cfg(feature = "ui")]
        if let Some(cost) = self.take_ui_frame() {
            self.ui_presented(lumatv_render::now_ns(), &cost);
        }
        Ok(self.clock.stamp())
    }

    fn capture(&mut self) -> io::Result<Image> {
        let (w, h) = (self.width as usize, self.height as usize);
        let mut bottom_up = vec![0u8; w * h * 4];
        // SAFETY: reading the bound offscreen framebuffer into a buffer of exactly w*h*4 bytes.
        unsafe {
            let a = &self.egl.api;
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, self.targets[self.current.0].fbo);
            (a.glFinish)();
            (a.glReadPixels)(0, 0, w as i32, h as i32, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, bottom_up.as_mut_ptr() as *mut c_void);
        }
        let err = self.egl.gl_error();
        if err != egl::GL_NO_ERROR {
            return Err(egl::other(format!("glReadPixels: 0x{err:x}")));
        }
        if self.targets[self.current.0].top_down {
            return Ok(Image { width: self.width, height: self.height, rgba: bottom_up });
        }
        let mut rgba = Vec::with_capacity(bottom_up.len());
        for row in bottom_up.chunks_exact(w * 4).rev() {
            rgba.extend_from_slice(row);
        }
        Ok(Image { width: self.width, height: self.height, rgba })
    }
}

/// The rows of `r` in an shm pool's bytes, packed into `out` (emptied first) as the GL upload wants them: as stored for a BGRA upload, else
/// swapped to RGBA; XRGB's alpha byte forced opaque either way. `false` when the pool is shorter than the rectangle needs. `out` keeps its
/// capacity, so the second upload of a rectangle that size allocates nothing.
pub(crate) fn pack_shm_rows(out: &mut Vec<u8>, bytes: &[u8], offset: usize, stride: usize, format: u32, r: &Rect, bgra: bool) -> bool {
    out.clear();
    out.reserve((r.w * r.h * 4) as usize);
    for y in r.y..r.bottom() {
        let off = offset + y as usize * stride + r.x as usize * 4;
        let Some(row) = bytes.get(off..off + r.w as usize * 4) else { return false };
        if bgra {
            out.extend_from_slice(row);
        } else {
            for px in row.chunks_exact(4) {
                out.extend_from_slice(&[px[2], px[1], px[0], if format == 1 { 255 } else { px[3] }]);
            }
        }
    }
    if bgra && format == 1 {
        for px in out.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }
    true
}

#[cfg(test)]
mod warm_slot_tests {
    use super::*;

    #[test]
    fn a_refused_warm_target_is_not_asked_for_again() {
        let mut slot = WarmSlot::default();
        let mut asked = 0;
        for _ in 0..3 {
            assert_eq!(slot.get_or_make(|| { asked += 1; Err(egl::other("incomplete")) }), None);
        }
        assert_eq!(asked, 1);
    }

    #[test]
    fn a_made_warm_target_is_kept() {
        let mut slot = WarmSlot::default();
        assert_eq!(slot.get_or_make(|| Ok((3, 4))), Some((3, 4)));
        assert_eq!(slot.get_or_make(|| panic!("made twice")), Some((3, 4)));
    }
}
