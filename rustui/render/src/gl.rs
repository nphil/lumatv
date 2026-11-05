//! EGL and OpenGL ES 3 on the Echo Show's Mali driver, loaded at run time.
//!
//! The vendor's `libGLES_mali.so` exports EGL itself. Android's own `libEGL.so` must never be loaded instead: it
//! waits for system services that do not exist on this OS. The driver is opened by path with `dlopen` rather than
//! linked, so this crate builds anywhere and finds the driver only where it runs (inside the Android chroot).
//! The tests load Mesa's software renderer instead ([`Gles::software`]) and draw real frames with it.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_void};
use std::io;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::programs;

type EglDisplay = *mut c_void;
type EglConfig = *mut c_void;
type EglSurface = *mut c_void;
type EglContext = *mut c_void;
type EglImage = *mut c_void;
type EglSync = *mut c_void;

const DRIVER: &CStr = c"/vendor/lib/egl/libGLES_mali.so";

const EGL_NONE: i32 = 0x3038;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_PBUFFER_BIT: i32 = 0x0001;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_ES3_BIT: i32 = 0x0040;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_ALPHA_SIZE: i32 = 0x3021;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;
const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
const EGL_NATIVE_BUFFER_ANDROID: u32 = 0x3140;
const EGL_IMAGE_PRESERVED_KHR: i32 = 0x30D2;
const EGL_TRUE: i32 = 1;
const EGL_SYNC_FENCE_KHR: u32 = 0x30F9;
const EGL_CONDITION_SATISFIED_KHR: i32 = 0x30F6;
#[cfg(test)]
const EGL_GL_TEXTURE_2D_KHR: u32 = 0x30B1;
#[cfg(test)]
const EGL_GL_TEXTURE_LEVEL_KHR: i32 = 0x30BC;
const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;
const EGL_PLATFORM_WAYLAND_KHR: u32 = 0x31D8;
const EGL_WINDOW_BIT: i32 = 0x0004;
const EGL_OPENGL_ES_API: u32 = 0x30A0;
const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
const EGL_TIMEOUT_EXPIRED_KHR: i32 = 0x30F5;

const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_ARRAY_BUFFER: u32 = 0x8892;
const GL_ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
const GL_TRIANGLE_STRIP: u32 = 0x0005;
const GL_TRIANGLES: u32 = 0x0004;
pub const GL_BLEND: u32 = 0x0BE2;
pub const GL_SCISSOR_TEST: u32 = 0x0C11;
const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
const GL_VERTEX_SHADER: u32 = 0x8B31;
const GL_FRAGMENT_SHADER: u32 = 0x8B30;
pub const GL_ONE: u32 = 1;
pub const GL_ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
const GL_FLOAT: u32 = 0x1406;
const GL_STREAM_DRAW: u32 = 0x88E0;
const GL_STATIC_DRAW: u32 = 0x88E4;
const GL_TEXTURE0: u32 = 0x84C0;
const GL_PACK_ALIGNMENT: u32 = 0x0D05;
pub const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_NO_ERROR: u32 = 0;
const GL_COMPILE_STATUS: u32 = 0x8B81;
const GL_LINK_STATUS: u32 = 0x8B82;
const GL_MAX_TEXTURE_SIZE: u32 = 0x0D33;
const GL_VERSION: u32 = 0x1F02;
const GL_RENDERER: u32 = 0x1F01;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
const GL_LINEAR: i32 = 0x2601;
const GL_CLAMP_TO_EDGE: i32 = 0x812F;
const GL_RGBA: u32 = 0x1908;
const GL_RGBA8: i32 = 0x8058;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_UNSIGNED_SHORT: u32 = 0x1403;
const GL_UNPACK_ROW_LENGTH: u32 = 0x0CF2;
const GL_RED: u32 = 0x1903;
const GL_R8: i32 = 0x8229;
const GL_UNPACK_ALIGNMENT: u32 = 0x0CF5;
/// What GL starts with: rows of pixels start on 4-byte boundaries.
const DEFAULT_UNPACK_ALIGNMENT: i32 = 4;

/// Declares a table of driver entry points, each resolved by name when the table is loaded.
macro_rules! api {
    ($table:ident { $($(#[$attribute:meta])* $field:ident = $symbol:literal fn($($arg:ty),*) $(-> $ret:ty)?;)* }) => {
        #[derive(Clone, Copy)]
        pub(super) struct $table {
            $($(#[$attribute])* $field: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }

        impl $table {
            pub(super) fn load(resolve: &dyn Fn(&str) -> Option<*const c_void>) -> io::Result<Self> {
                Ok(Self {
                    $($(#[$attribute])* $field: {
                        let address = resolve($symbol).ok_or_else(|| {
                            io::Error::other(concat!($symbol, " is missing from the GL driver"))
                        })?;
                        // SAFETY: the driver's entry point of this name has exactly this signature.
                        unsafe {
                            std::mem::transmute::<*const c_void, unsafe extern "C" fn($($arg),*) $(-> $ret)?>(address)
                        }
                    },)*
                })
            }
        }
    };
}

#[cfg(test)]
mod counted;
pub mod mesh;
#[cfg(test)]
pub use counted::{Counts, take_counts, take_uploads};

api!(EglCore {
    get_display = "eglGetDisplay" fn(*mut c_void) -> EglDisplay;
    initialize = "eglInitialize" fn(EglDisplay, *mut i32, *mut i32) -> u32;
    choose_config = "eglChooseConfig" fn(EglDisplay, *const i32, *mut EglConfig, i32, *mut i32) -> u32;
    create_pbuffer_surface = "eglCreatePbufferSurface" fn(EglDisplay, EglConfig, *const i32) -> EglSurface;
    create_context = "eglCreateContext" fn(EglDisplay, EglConfig, EglContext, *const i32) -> EglContext;
    make_current = "eglMakeCurrent" fn(EglDisplay, EglSurface, EglSurface, EglContext) -> u32;
    get_error = "eglGetError" fn() -> i32;
    create_window_surface = "eglCreateWindowSurface" fn(EglDisplay, EglConfig, *mut c_void, *const i32) -> EglSurface;
    swap_buffers = "eglSwapBuffers" fn(EglDisplay, EglSurface) -> u32;
    swap_interval = "eglSwapInterval" fn(EglDisplay, i32) -> u32;
    bind_api = "eglBindAPI" fn(u32) -> u32;
    get_current_context = "eglGetCurrentContext" fn() -> EglContext;
    get_current_display = "eglGetCurrentDisplay" fn() -> EglDisplay;
});

api!(EglExtensions {
    create_image = "eglCreateImageKHR" fn(EglDisplay, EglContext, u32, *mut c_void, *const i32) -> EglImage;
    destroy_image = "eglDestroyImageKHR" fn(EglDisplay, EglImage) -> u32;
    create_sync = "eglCreateSyncKHR" fn(EglDisplay, u32, *const i32) -> EglSync;
    destroy_sync = "eglDestroySyncKHR" fn(EglDisplay, EglSync) -> u32;
    client_wait_sync = "eglClientWaitSyncKHR" fn(EglDisplay, EglSync, i32, u64) -> i32;
});

api!(GlApi {
    get_string = "glGetString" fn(u32) -> *const u8;
    get_error = "glGetError" fn() -> u32;
    get_integerv = "glGetIntegerv" fn(u32, *mut i32);
    create_shader = "glCreateShader" fn(u32) -> u32;
    shader_source = "glShaderSource" fn(u32, i32, *const *const c_char, *const i32);
    compile_shader = "glCompileShader" fn(u32);
    get_shaderiv = "glGetShaderiv" fn(u32, u32, *mut i32);
    get_shader_info_log = "glGetShaderInfoLog" fn(u32, i32, *mut i32, *mut c_char);
    delete_shader = "glDeleteShader" fn(u32);
    create_program = "glCreateProgram" fn() -> u32;
    attach_shader = "glAttachShader" fn(u32, u32);
    link_program = "glLinkProgram" fn(u32);
    get_programiv = "glGetProgramiv" fn(u32, u32, *mut i32);
    get_program_info_log = "glGetProgramInfoLog" fn(u32, i32, *mut i32, *mut c_char);
    delete_program = "glDeleteProgram" fn(u32);
    use_program = "glUseProgram" fn(u32);
    get_uniform_location = "glGetUniformLocation" fn(u32, *const c_char) -> i32;
    uniform1i = "glUniform1i" fn(i32, i32);
    uniform1f = "glUniform1f" fn(i32, f32);
    uniform2f = "glUniform2f" fn(i32, f32, f32);
    uniform3f = "glUniform3f" fn(i32, f32, f32, f32);
    uniform4f = "glUniform4f" fn(i32, f32, f32, f32, f32);
    uniform4fv = "glUniform4fv" fn(i32, i32, *const f32);
    viewport = "glViewport" fn(i32, i32, i32, i32);
    scissor = "glScissor" fn(i32, i32, i32, i32);
    enable = "glEnable" fn(u32);
    disable = "glDisable" fn(u32);
    blend_func = "glBlendFunc" fn(u32, u32);
    clear_color = "glClearColor" fn(f32, f32, f32, f32);
    clear = "glClear" fn(u32);
    gen_textures = "glGenTextures" fn(i32, *mut u32);
    delete_textures = "glDeleteTextures" fn(i32, *const u32);
    bind_texture = "glBindTexture" fn(u32, u32);
    tex_parameteri = "glTexParameteri" fn(u32, u32, i32);
    tex_image_2d = "glTexImage2D" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
    tex_sub_image_2d = "glTexSubImage2D" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
    pixel_storei = "glPixelStorei" fn(u32, i32);
    active_texture = "glActiveTexture" fn(u32);
    // The tests and the host's screenshots read a frame back.
    read_pixels = "glReadPixels" fn(i32, i32, i32, i32, u32, u32, *mut c_void);
    delete_fbo = "glDeleteFramebuffers" fn(i32, *const u32);
    gen_framebuffers = "glGenFramebuffers" fn(i32, *mut u32);
    bind_framebuffer = "glBindFramebuffer" fn(u32, u32);
    framebuffer_texture_2d = "glFramebufferTexture2D" fn(u32, u32, u32, u32, i32);
    check_framebuffer_status = "glCheckFramebufferStatus" fn(u32) -> u32;
    gen_buffers = "glGenBuffers" fn(i32, *mut u32);
    bind_buffer = "glBindBuffer" fn(u32, u32);
    buffer_data = "glBufferData" fn(u32, isize, *const c_void, u32);
    enable_vertex_attrib_array = "glEnableVertexAttribArray" fn(u32);
    vertex_attrib_pointer = "glVertexAttribPointer" fn(u32, i32, u32, u8, i32, *const c_void);
    draw_arrays = "glDrawArrays" fn(u32, i32, i32);
    draw_elements = "glDrawElements" fn(u32, i32, u32, *const c_void);
    flush = "glFlush" fn();
    finish = "glFinish" fn();
    image_target_texture_2d = "glEGLImageTargetTexture2DOES" fn(u32, EglImage);
    is_enabled = "glIsEnabled" fn(u32) -> u8;
    get_vertex_attribiv = "glGetVertexAttribiv" fn(u32, u32, *mut i32);
    disable_vertex_attrib_array = "glDisableVertexAttribArray" fn(u32);
    blend_func_separate = "glBlendFuncSeparate" fn(u32, u32, u32, u32);
    bind_vertex_array = "glBindVertexArray" fn(u32);
});

fn egl_error(egl: &EglCore, what: &str) -> io::Error {
    // SAFETY: a plain query of the calling thread's EGL error.
    io::Error::other(format!("{what}: EGL error {:#x}", unsafe {
        (egl.get_error)()
    }))
}

/// A GLES 3 context on the Mali, current on the thread that created it, and the entry points to use it. It is
/// deliberately not `Send`: GL calls belong to one thread.
/// Render targets made since the last [`take_target_cost`]: how many, their pixels, and the time they took.
static TARGETS_MADE: AtomicU64 = AtomicU64::new(0);
static TARGET_PIXELS: AtomicU64 = AtomicU64::new(0);
static TARGET_NS: AtomicU64 = AtomicU64::new(0);

/// In tests, a seed for the noise that the textures that are made undefined (`new_texture`), the pool's render targets (taken
/// new or again) and the plates are filled with (0: nothing, the contents stay as the driver left them): what the memory of a
/// target that was not made clear would hold. A frame that shows anything of a texture nothing drew into then shows noise, and
/// two runs of one script with different seeds differ exactly where a frame read what nothing wrote.
#[cfg(test)]
pub(crate) static POISON: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    /// Programs this thread's GL has drawn with, and those first drawn with since [`take_first_uses`] (a program the driver finishes
    /// the first time it is used: what the host's replay of a script says a screen's first frames are the first to need).
    static USED_PROGRAMS: RefCell<(std::collections::HashSet<u32>, Vec<u32>)> = RefCell::default();
}

/// The programs first drawn with since this was last called (the ids the driver gave them, in the order of their first use).
#[cfg(test)]
pub fn take_first_uses() -> Vec<u32> {
    USED_PROGRAMS.with(|used| std::mem::take(&mut used.borrow_mut().1))
}

#[cfg(test)]
thread_local! {
    /// How many times this thread has handed work to the GPU with [`Fences::kick`].
    static KICKS: Cell<u64> = const { Cell::new(0) };
}

/// The hand-overs of [`Fences::kick`] this thread has made.
#[cfg(test)]
pub fn kicks() -> u64 {
    KICKS.with(Cell::get)
}

/// What making render targets has cost since this was last called: (targets, pixels, nanoseconds).
pub fn take_target_cost() -> (u64, u64, u64) {
    (
        TARGETS_MADE.swap(0, Ordering::Relaxed),
        TARGET_PIXELS.swap(0, Ordering::Relaxed),
        TARGET_NS.swap(0, Ordering::Relaxed),
    )
}

/// What the GL was asked since [`Gles::take_calls`]: the calls the wrapper passed to the driver, by kind, and the bytes of vertices it was
/// given. A frame's CPU time between its ops being applied and its draw calls being issued is, to a good approximation, these calls
/// (the trace's `nd nu np nb ns nk vb`), so a test can pin what a frame asks of the driver without a clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Calls {
    /// `glDrawArrays` and `glDrawElements`.
    pub draws: u32,
    /// `glUniform*`.
    pub uniforms: u32,
    /// `glUseProgram`.
    pub programs: u32,
    /// `glBindTexture` and `glActiveTexture`.
    pub binds: u32,
    /// The rest of the state: enable, disable, blend function, scissor, viewport, framebuffer binds, clears.
    pub states: u32,
    /// Calls the wrapper did not make because the state they set was already set.
    pub skipped: u32,
    /// Bytes handed to the driver as vertices (`glBufferData`).
    pub vertex_bytes: u64,
}

impl Calls {
    /// The sum of two frames' (or two targets') counts.
    pub fn add(&mut self, other: &Calls) {
        self.draws += other.draws;
        self.uniforms += other.uniforms;
        self.programs += other.programs;
        self.binds += other.binds;
        self.states += other.states;
        self.skipped += other.skipped;
        self.vertex_bytes += other.vertex_bytes;
    }
}

/// How many texture units the cache follows (the compositor draws from unit 0 and a few passes use unit 1).
const UNITS: usize = 4;

/// What the driver has been told, so that a call that would set what is set already is not made ([`Calls::skipped`]): the program in use
/// and the last value of each of its uniforms (a program keeps them while another is in use), the texture bound on each unit, and the few
/// pieces of state a frame sets again and again. Every call that changes any of it goes through [`Gles`], which is what makes leaving a
/// call out safe: a texture that is deleted is forgotten, and a program that is made starts with no values. `LUMASHOW_GL_CACHE=off` in
/// the environment makes every call go through, to see what the cache saves.
struct Known {
    on: bool,
    program: Cell<u32>,
    unit: Cell<u32>,
    textures: [Cell<u32>; UNITS],
    blend: Cell<Option<bool>>,
    scissor_test: Cell<Option<bool>>,
    blend_func: Cell<Option<(u32, u32)>>,
    scissor: Cell<Option<[i32; 4]>>,
    viewport: Cell<Option<[i32; 4]>>,
    /// The last value given to each uniform, by program and location, as bits (the components a kind of uniform does not have are 0).
    uniforms: RefCell<Vec<Vec<Option<[u32; 4]>>>>,
}

impl Known {
    fn new() -> Self {
        let off =
            std::env::var("LUMASHOW_GL_CACHE").is_ok_and(|v| matches!(v.as_str(), "off" | "0"));
        Self {
            on: !off,
            program: Cell::new(0),
            unit: Cell::new(0),
            textures: Default::default(),
            blend: Cell::new(None),
            scissor_test: Cell::new(None),
            blend_func: Cell::new(None),
            scissor: Cell::new(None),
            viewport: Cell::new(None),
            uniforms: RefCell::new(Vec::new()),
        }
    }

    /// Whether uniform `location` of the program in use has `bits` already; remembers them when it has not.
    fn uniform_is(&self, location: i32, bits: [u32; 4]) -> bool {
        // A location of -1 is a uniform the program does not have: GL ignores the call, and there is nothing to remember.
        let Ok(location) = usize::try_from(location) else {
            return false;
        };
        // After `Gles::enter` the program in use is not known: the value goes to the driver, and nothing is remembered.
        if !self.on || self.program.get() == u32::MAX {
            return false;
        }
        let program = self.program.get() as usize;
        let mut all = self.uniforms.borrow_mut();
        if all.len() <= program {
            all.resize_with(program + 1, Vec::new);
        }
        let slots = &mut all[program];
        if slots.len() <= location {
            slots.resize(location + 1, None);
        }
        if slots[location] == Some(bits) {
            return true;
        }
        slots[location] = Some(bits);
        false
    }

    /// A program that was just made has had no values set.
    fn forget_program(&self, program: u32) {
        if let Some(slots) = self.uniforms.borrow_mut().get_mut(program as usize) {
            slots.clear();
        }
    }

    /// The texture bound on the unit in use, when the cache follows that unit.
    fn bound(&self) -> Option<&Cell<u32>> {
        self.textures.get(self.unit.get() as usize)
    }

    /// A texture that is gone is bound nowhere.
    fn forget_texture(&self, texture: u32) {
        for bound in &self.textures {
            if bound.get() == texture {
                bound.set(0);
            }
        }
    }

    /// The state of a context that someone else has had since the toolkit last drew in it is not known: nothing of what was
    /// set is assumed ([`Gles::enter`]). The values of a program's uniforms are the program's own, and are kept.
    fn forget_context(&self) {
        self.program.set(u32::MAX);
        self.unit.set(0);
        for bound in &self.textures {
            bound.set(u32::MAX);
        }
        self.blend.set(None);
        self.scissor_test.set(None);
        self.blend_func.set(None);
        self.scissor.set(None);
        self.viewport.set(None);
    }

    /// The remembered state of a capability that the cache follows (blending and the scissor test, which every frame sets again and again).
    fn capability(&self, capability: u32) -> Option<&Cell<Option<bool>>> {
        match capability {
            GL_BLEND => Some(&self.blend),
            GL_SCISSOR_TEST => Some(&self.scissor_test),
            _ => None,
        }
    }
}

pub struct Gles {
    egl: EglCore,
    ext: EglExtensions,
    gl: GlApi,
    mesh_api: mesh::MeshGl,
    display: EglDisplay,
    /// The window surface frames are presented on, when the context was made for one ([`Gles::window`]).
    window: Option<EglSurface>,
    calls: Cell<Calls>,
    /// Every call made into the driver since the context was made, which nothing resets ([`Gles::touched`]).
    touched: Cell<u64>,
    known: Known,
    on_creating_thread: PhantomData<*const ()>,
    /// Where programs come from before one is linked and go back to when this is dropped (LumaTV: the compositor links the engine's
    /// programs once, off its loop thread, `programs::Cache`). `None`: every program is linked here, as a window's renderer does.
    programs: Option<programs::Lease>,
    /// The programs this linked itself.
    linked: Cell<u32>,
}

/// A GL framebuffer object whose colour attachment is a gralloc buffer imported through an `EGLImage`. Neither is
/// ever destroyed: the buffers live until the process exits.
pub struct Target {
    pub framebuffer: u32,
}

/// A texture that shows the memory behind an `EGLImage` (a gralloc buffer that stands for a producer's dma-buf; in the
/// tests, another texture's storage) without copying it: whatever is written to that memory is what the next draw samples.
/// Unlike a [`Target`] it is ours to delete ([`Gles::delete_import`]), on the thread that made it.
pub struct ImageTexture {
    pub texture: u32,
    image: EglImage,
}

/// A texture that a framebuffer object draws into, for the passes that make pictures of their own (backdrops,
/// meshes): unlike a [`Target`], it is ours to make and delete.
#[derive(Clone, Copy, Debug)]
pub struct RenderTarget {
    pub texture: u32,
    pub framebuffer: u32,
    pub width: u32,
    pub height: u32,
}

/// A point in the GPU's command stream that the present thread waits for.
pub struct Fence(EglSync);

// SAFETY: an EGL sync handle is only waited on and destroyed once, by whichever thread receives it (EGL sync
// objects may be used from any thread).
unsafe impl Send for Fence {}

/// Creates and waits for fences. Shared with the present thread, which waits for the GPU without owning the
/// GL context.
#[derive(Clone, Copy)]
pub struct Fences {
    display: EglDisplay,
    create: unsafe extern "C" fn(EglDisplay, u32, *const i32) -> EglSync,
    destroy: unsafe extern "C" fn(EglDisplay, EglSync) -> u32,
    wait: unsafe extern "C" fn(EglDisplay, EglSync, i32, u64) -> i32,
}

// SAFETY: EGL_KHR_fence_sync objects may be waited on from any thread, and the display handle is never mutated.
unsafe impl Send for Fences {}
unsafe impl Sync for Fences {}

impl Fences {
    /// Marks the current end of the GL command stream. Call it on the GL thread after `Gles::flush`.
    pub fn create(&self) -> io::Result<Fence> {
        // SAFETY: EGL_KHR_fence_sync on the display; the caller's GL context is current.
        let sync = unsafe { (self.create)(self.display, EGL_SYNC_FENCE_KHR, std::ptr::null()) };
        if sync.is_null() {
            return Err(io::Error::other("eglCreateSyncKHR failed"));
        }
        Ok(Fence(sync))
    }

    /// Gets the GPU started on what was drawn and flushed since the last fence, for a draw that no frame's fence follows (the
    /// catch-up's, `engine::FrameLoop::catch_up`): a fence is made behind it and destroyed at once. Nothing waits for it.
    ///
    /// A `glFlush` alone does not do it on the Show's Mali driver: what is drawn into a scanout buffer stays queued in the driver
    /// until a fence is made, and the next frame makes one and then waits for the GPU to draw it, with its own work. The catch-up's
    /// panel (5.4 to 6.8 ms of GPU) came out of the quiet and into the first frame of a touch, in every swipe home from the
    /// keyboard of an alarm's label (frame 1, white: 8.6 ms of render, 5.4 of them waiting for the GPU; with a fence made behind
    /// the catch-up's draw 3.4 ms and 0.2, what 008d's frames, which had fences, left it).
    pub fn kick(&self) -> io::Result<()> {
        #[cfg(test)]
        KICKS.with(|kicks| kicks.set(kicks.get() + 1));
        let fence = self.create()?;
        // SAFETY: a valid sync object, destroyed exactly once, here.
        unsafe { (self.destroy)(self.display, fence.0) };
        Ok(())
    }

    /// Blocks until the GPU has finished everything before the fence, for at most `timeout`, and destroys it.
    /// `Ok(false)` means the GPU did not finish in time.
    pub fn wait(&self, fence: Fence, timeout: Duration) -> io::Result<bool> {
        let nanos = u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: a valid sync object, waited on and then destroyed exactly once.
        let status = unsafe { (self.wait)(self.display, fence.0, 0, nanos) };
        // SAFETY: as above; the object is not used again.
        unsafe { (self.destroy)(self.display, fence.0) };
        match status {
            EGL_CONDITION_SATISFIED_KHR => Ok(true),
            EGL_TIMEOUT_EXPIRED_KHR => Ok(false),
            other => Err(io::Error::other(format!(
                "eglClientWaitSyncKHR returned {other:#x}"
            ))),
        }
    }
}

fn open_driver(path: &CStr) -> io::Result<*mut c_void> {
    // SAFETY: dlopen of the vendor GL driver by absolute path.
    let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if handle.is_null() {
        // SAFETY: dlerror returns a C string owned by the loader, valid until the next dl call.
        let why = unsafe {
            let message = libc::dlerror();
            if message.is_null() {
                "unknown error".into()
            } else {
                CStr::from_ptr(message).to_string_lossy().into_owned()
            }
        };
        return Err(io::Error::other(format!(
            "cannot load {}: {why}",
            path.to_string_lossy()
        )));
    }
    Ok(handle)
}

/// How EGL finds a display.
#[derive(Clone, Copy)]
enum Platform {
    /// The driver's own default: the Mali's only display.
    Default,
    /// Mesa's platform without a window system, for software rendering into textures and a pbuffer of `size`.
    Surfaceless { size: (i32, i32) },
    /// A Wayland client: the driver's display on `wl_display` and a window surface over `wl_egl_window`.
    Window {
        wl_display: *mut c_void,
        wl_egl_window: *mut c_void,
    },
}

impl Gles {
    /// Loads the Mali driver and makes a GLES 3 context current on the calling thread. Rendering goes to
    /// framebuffer objects over gralloc buffers ([`Gles::target`]); the context only needs a token 1x1 surface.
    pub fn new() -> io::Result<Self> {
        Self::open(DRIVER, Platform::Default)
    }

    /// The tests' software GL: the EGL library at `library` (Mesa's), on a display of its own without a window system.
    pub fn software(library: &CStr) -> io::Result<Self> {
        Self::open(library, Platform::Surfaceless { size: (1, 1) })
    }

    /// The same with a pbuffer of `width` x `height` as the default framebuffer, which a host draws a window's frames into.
    pub fn software_pbuffer(library: &CStr, width: u32, height: u32) -> io::Result<Self> {
        Self::open(
            library,
            Platform::Surfaceless {
                size: (width as i32, height as i32),
            },
        )
    }

    /// A context on the window surface `wl_egl_window` over the Wayland connection `wl_display`, from the EGL/GLES library at `driver`
    /// (the 64-bit Mali blob, which exports both). Frames are presented with [`Gles::swap_buffers`].
    ///
    /// # Safety
    /// Both pointers must stay valid for as long as the returned value lives.
    pub unsafe fn window(
        driver: &CStr,
        wl_display: *mut c_void,
        wl_egl_window: *mut c_void,
    ) -> io::Result<Self> {
        Self::open(
            driver,
            Platform::Window {
                wl_display,
                wl_egl_window,
            },
        )
    }

    /// Shows the frame drawn into the window surface's framebuffer (0). With a swap interval of 1 this waits for the compositor's
    /// frame callback.
    pub fn swap_buffers(&self) -> io::Result<()> {
        let Some(surface) = self.window else {
            return Err(io::Error::other("this context has no window surface"));
        };
        // SAFETY: the display and surface are the ones this context was made with.
        if unsafe { (self.egl.swap_buffers)(self.display, surface) } == 0 {
            return Err(egl_error(&self.egl, "eglSwapBuffers"));
        }
        Ok(())
    }

    /// `eglSwapInterval` of the window surface's display: 0 does not wait for the compositor's frame callback, 1 does.
    pub fn set_swap_interval(&self, interval: i32) {
        // SAFETY: the display is the one this context was made with.
        unsafe { (self.egl.swap_interval)(self.display, interval) };
    }

    /// The same over the EGL/GLES library at `driver`.
    fn open(driver: &CStr, platform: Platform) -> io::Result<Self> {
        let driver = open_driver(driver)?;
        let symbol = |name: &str| -> Option<*const c_void> {
            let name = CString::new(name).ok()?;
            // SAFETY: a symbol lookup in the driver opened above.
            let address = unsafe { libc::dlsym(driver, name.as_ptr()) };
            (!address.is_null()).then_some(address as *const c_void)
        };
        let egl = EglCore::load(&symbol)?;
        let get_proc_address = symbol("eglGetProcAddress")
            .ok_or_else(|| io::Error::other("eglGetProcAddress is missing from the GL driver"))?;
        // SAFETY: `eglGetProcAddress` has this signature.
        let get_proc_address = unsafe {
            std::mem::transmute::<*const c_void, unsafe extern "C" fn(*const c_char) -> *const c_void>(
                get_proc_address,
            )
        };
        let resolve = |name: &str| -> Option<*const c_void> {
            symbol(name).or_else(|| {
                let name = CString::new(name).ok()?;
                // SAFETY: an entry-point lookup by name.
                let address = unsafe { get_proc_address(name.as_ptr()) };
                (!address.is_null()).then_some(address)
            })
        };

        // SAFETY: standard EGL bring-up; every handle is checked before it is used.
        let display = unsafe {
            let display = match platform {
                Platform::Default => (egl.get_display)(std::ptr::null_mut()),
                Platform::Window { wl_display, .. } => {
                    let get = resolve("eglGetPlatformDisplay")
                        .or_else(|| resolve("eglGetPlatformDisplayEXT"))
                        .ok_or_else(|| {
                            io::Error::other("eglGetPlatformDisplay is missing from the EGL library")
                        })?;
                    let get = std::mem::transmute::<
                        *const c_void,
                        unsafe extern "C" fn(u32, *mut c_void, *const isize) -> EglDisplay,
                    >(get);
                    get(EGL_PLATFORM_WAYLAND_KHR, wl_display, std::ptr::null())
                }
                Platform::Surfaceless { .. } => {
                    let get = resolve("eglGetPlatformDisplayEXT").ok_or_else(|| {
                        io::Error::other("eglGetPlatformDisplayEXT is missing from the EGL library")
                    })?;
                    let get = std::mem::transmute::<
                        *const c_void,
                        unsafe extern "C" fn(u32, *mut c_void, *const i32) -> EglDisplay,
                    >(get);
                    get(
                        EGL_PLATFORM_SURFACELESS_MESA,
                        std::ptr::null_mut(),
                        std::ptr::null(),
                    )
                }
            };
            if display.is_null()
                || (egl.initialize)(display, std::ptr::null_mut(), std::ptr::null_mut()) == 0
            {
                return Err(egl_error(&egl, "eglInitialize"));
            }
            let surface_type = match platform {
                Platform::Window { .. } => EGL_WINDOW_BIT,
                _ => EGL_PBUFFER_BIT,
            };
            // A driver may list its ES 3 configs under the ES 2 bit alone: a version 3 context is still made on them.
            let mut config: EglConfig = std::ptr::null_mut();
            let mut chosen = false;
            for renderable in [EGL_OPENGL_ES3_BIT, EGL_OPENGL_ES2_BIT] {
                let config_attributes = [
                    EGL_SURFACE_TYPE,
                    surface_type,
                    EGL_RENDERABLE_TYPE,
                    renderable,
                    EGL_RED_SIZE,
                    8,
                    EGL_GREEN_SIZE,
                    8,
                    EGL_BLUE_SIZE,
                    8,
                    EGL_ALPHA_SIZE,
                    8,
                    EGL_NONE,
                ];
                let mut count = 0;
                if (egl.choose_config)(
                    display,
                    config_attributes.as_ptr(),
                    &mut config,
                    1,
                    &mut count,
                ) != 0
                    && count == 1
                {
                    chosen = true;
                    break;
                }
            }
            if !chosen {
                return Err(egl_error(&egl, "eglChooseConfig"));
            }
            (egl.bind_api)(EGL_OPENGL_ES_API);
            let (surface, window) = match platform {
                Platform::Window { wl_egl_window, .. } => {
                    let attributes = [EGL_NONE];
                    let surface = (egl.create_window_surface)(
                        display,
                        config,
                        wl_egl_window,
                        attributes.as_ptr(),
                    );
                    if surface.is_null() {
                        return Err(egl_error(&egl, "eglCreateWindowSurface"));
                    }
                    (surface, Some(surface))
                }
                Platform::Default => (std::ptr::null_mut(), None),
                Platform::Surfaceless { size } => {
                    let attributes = [EGL_WIDTH, size.0, EGL_HEIGHT, size.1, EGL_NONE];
                    let surface =
                        (egl.create_pbuffer_surface)(display, config, attributes.as_ptr());
                    if surface.is_null() {
                        return Err(egl_error(&egl, "eglCreatePbufferSurface"));
                    }
                    (surface, None)
                }
            };
            let surface = if surface.is_null() {
                let attributes = [EGL_WIDTH, 1, EGL_HEIGHT, 1, EGL_NONE];
                let surface = (egl.create_pbuffer_surface)(display, config, attributes.as_ptr());
                if surface.is_null() {
                    return Err(egl_error(&egl, "eglCreatePbufferSurface"));
                }
                surface
            } else {
                surface
            };
            let context_attributes = [EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE];
            let context = (egl.create_context)(
                display,
                config,
                std::ptr::null_mut(),
                context_attributes.as_ptr(),
            );
            if context.is_null() || (egl.make_current)(display, surface, surface, context) == 0 {
                return Err(egl_error(&egl, "eglCreateContext/eglMakeCurrent"));
            }
            (display, window)
        };
        let (display, window) = display;
        let gl_api = GlApi::load(&resolve)?;
        Ok(Self {
            egl,
            ext: EglExtensions::load(&resolve)?,
            mesh_api: mesh::MeshGl::load(&resolve, &gl_api)?,
            gl: gl_api,
            display,
            window,
            calls: Cell::new(Calls::default()),
            touched: Cell::new(0),
            known: Known::new(),
            on_creating_thread: PhantomData,
            programs: None,
            linked: Cell::new(0),
        })
    }

    pub fn fences(&self) -> Fences {
        Fences {
            display: self.display,
            create: self.ext.create_sync,
            destroy: self.ext.destroy_sync,
            wait: self.ext.client_wait_sync,
        }
    }

    fn string(&self, name: u32) -> String {
        // SAFETY: glGetString returns a static NUL-terminated string, or null.
        let p = unsafe { (self.gl.get_string)(name) };
        if p.is_null() {
            return "?".into();
        }
        // SAFETY: non-null, NUL-terminated.
        unsafe { CStr::from_ptr(p as *const c_char) }
            .to_string_lossy()
            .into_owned()
    }

    /// The driver's renderer and version strings, for the log.
    pub fn describe(&self) -> String {
        format!("{} | {}", self.string(GL_RENDERER), self.string(GL_VERSION))
    }

    pub fn max_texture_size(&self) -> u32 {
        let mut size = 0;
        // SAFETY: writes one integer.
        unsafe { (self.gl.get_integerv)(GL_MAX_TEXTURE_SIZE, &mut size) };
        u32::try_from(size).unwrap_or(0)
    }

    /// Wraps a gralloc buffer as a render target.
    ///
    /// # Safety
    /// `native_buffer` must point to a live `ANativeWindowBuffer` that outlives the returned target.
    pub unsafe fn target(&self, native_buffer: *mut c_void) -> io::Result<Target> {
        let attributes = [EGL_IMAGE_PRESERVED_KHR, EGL_TRUE, EGL_NONE];
        // SAFETY: EGL_ANDROID_image_native_buffer over a buffer the caller keeps alive.
        let image = unsafe {
            (self.ext.create_image)(
                self.display,
                std::ptr::null_mut(),
                EGL_NATIVE_BUFFER_ANDROID,
                native_buffer,
                attributes.as_ptr(),
            )
        };
        if image.is_null() {
            return Err(egl_error(&self.egl, "eglCreateImageKHR"));
        }
        let (mut texture, mut framebuffer) = (0u32, 0u32);
        // SAFETY: GL object creation on the current context; completeness is checked below.
        let (status, error) = unsafe {
            (self.gl.gen_textures)(1, &mut texture);
            (self.gl.bind_texture)(GL_TEXTURE_2D, texture);
            if let Some(bound) = self.known.bound() {
                bound.set(texture);
            }
            (self.gl.image_target_texture_2d)(GL_TEXTURE_2D, image);
            (self.gl.gen_framebuffers)(1, &mut framebuffer);
            (self.gl.bind_framebuffer)(GL_FRAMEBUFFER, framebuffer);
            (self.gl.framebuffer_texture_2d)(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_TEXTURE_2D,
                texture,
                0,
            );
            (
                (self.gl.check_framebuffer_status)(GL_FRAMEBUFFER),
                (self.gl.get_error)(),
            )
        };
        if status != GL_FRAMEBUFFER_COMPLETE || error != GL_NO_ERROR {
            // SAFETY: the image was created above and is not used again.
            unsafe { (self.ext.destroy_image)(self.display, image) };
            return Err(io::Error::other(format!(
                "a gralloc buffer is not a usable render target: framebuffer status {status:#x}, GL error {error:#x}"
            )));
        }
        Ok(Target { framebuffer })
    }

    /// Makes a texture over the gralloc buffer `native_buffer` names, to be drawn from: nothing is copied, so the picture is
    /// whatever the buffer's memory holds at each draw. The texture is left bound.
    ///
    /// # Safety
    /// `native_buffer` must point to a live `ANativeWindowBuffer` (and the descriptor in its handle must stay open) until
    /// [`Gles::delete_import`] has been called with the result.
    pub unsafe fn import_native(&self, native_buffer: *mut c_void) -> io::Result<ImageTexture> {
        let attributes = [EGL_IMAGE_PRESERVED_KHR, EGL_TRUE, EGL_NONE];
        // SAFETY: EGL_ANDROID_image_native_buffer over a buffer the caller keeps alive.
        let image = unsafe {
            (self.ext.create_image)(
                self.display,
                std::ptr::null_mut(),
                EGL_NATIVE_BUFFER_ANDROID,
                native_buffer,
                attributes.as_ptr(),
            )
        };
        self.image_texture(image)
    }

    /// A texture over `image`, destroying the image if it cannot be one.
    fn image_texture(&self, image: EglImage) -> io::Result<ImageTexture> {
        self.touch();
        if image.is_null() {
            return Err(egl_error(&self.egl, "eglCreateImageKHR"));
        }
        let mut texture = 0u32;
        // SAFETY: GL object creation on the current context; the error is checked below.
        let error = unsafe {
            (self.gl.gen_textures)(1, &mut texture);
            (self.gl.bind_texture)(GL_TEXTURE_2D, texture);
            if let Some(bound) = self.known.bound() {
                bound.set(texture);
            }
            (self.gl.image_target_texture_2d)(GL_TEXTURE_2D, image);
            (self.gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            (self.gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            (self.gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (self.gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            (self.gl.get_error)()
        };
        if error != GL_NO_ERROR {
            self.delete_texture(texture);
            // SAFETY: the image was made by the caller's call and is not used again.
            unsafe { (self.ext.destroy_image)(self.display, image) };
            return Err(io::Error::other(format!(
                "an EGLImage is not a usable texture: GL error {error:#x}"
            )));
        }
        Ok(ImageTexture { texture, image })
    }

    /// Lets go of the texture and the image it shows (the memory behind them is the owner's).
    pub fn delete_import(&self, imported: ImageTexture) {
        self.delete_texture(imported.texture);
        // SAFETY: the image was made by `import_native` or `alias` and is destroyed once, here.
        unsafe { (self.ext.destroy_image)(self.display, imported.image) };
    }

    /// A texture that shares the storage of texture `source` (`EGL_KHR_gl_texture_2D_image`): the tests' stand-in for a
    /// producer's buffer, which they fill with `upload` the way a camera's DMA fills a dma-buf, outside the code under test.
    #[cfg(test)]
    pub fn alias(&self, source: u32) -> io::Result<ImageTexture> {
        let attributes = [EGL_GL_TEXTURE_LEVEL_KHR, 0, EGL_NONE];
        // SAFETY: EGL_KHR_gl_texture_2D_image over a texture of the current context; `source` is a name, not a pointer.
        let image = unsafe {
            (self.ext.create_image)(
                self.display,
                (self.egl.get_current_context)(),
                EGL_GL_TEXTURE_2D_KHR,
                source as usize as *mut c_void,
                attributes.as_ptr(),
            )
        };
        self.image_texture(image)
    }

    /// The first pending GL error, if any (and clears it).
    pub fn take_error(&self) -> Option<u32> {
        // SAFETY: a plain query.
        let error = unsafe { (self.gl.get_error)() };
        (error != GL_NO_ERROR).then_some(error)
    }

    // The calls below are the thin, safe end of the API. GL calls with plain-value arguments cannot violate memory
    // safety; they are only meaningful with the context current, which owning a `Gles` on its thread guarantees.

    /// Counts one call (see [`Calls`]).
    #[inline]
    fn count(&self, f: impl FnOnce(&mut Calls)) {
        self.touch();
        let mut calls = self.calls.get();
        f(&mut calls);
        self.calls.set(calls);
    }

    /// Notes a call into the driver that [`Calls`] does not count (a texture made, a picture copied, a buffer bound).
    #[inline]
    fn touch(&self) {
        self.touched.set(self.touched.get().wrapping_add(1));
    }

    /// How many calls this has made into the driver since the context was made: a host that lets the toolkit draw outside its state
    /// scope (`Gles::enter`) checks that none were made.
    pub fn touched(&self) -> u64 {
        self.touched.get()
    }

    /// Counts a call that was left out because the state it sets was set already.
    #[inline]
    fn skip(&self) {
        let mut calls = self.calls.get();
        calls.skipped += 1;
        self.calls.set(calls);
    }

    /// What the GL was asked since this was last called (or the context was made).
    pub fn take_calls(&self) -> Calls {
        self.calls.take()
    }

    pub fn use_program(&self, program: u32) {
        #[cfg(test)]
        USED_PROGRAMS.with(|used| {
            let mut used = used.borrow_mut();
            if used.0.insert(program) {
                used.1.push(program);
            }
        });
        if self.known.on && self.known.program.get() == program {
            return self.skip();
        }
        self.known.program.set(program);
        self.count(|c| c.programs += 1);
        // SAFETY: see above.
        unsafe { (self.gl.use_program)(program) }
    }

    pub fn uniform1i(&self, location: i32, v: i32) {
        if self.known.uniform_is(location, [v as u32, 0, 0, 0]) {
            return self.skip();
        }
        self.count(|c| c.uniforms += 1);
        // SAFETY: see above.
        unsafe { (self.gl.uniform1i)(location, v) }
    }

    pub fn uniform1f(&self, location: i32, v: f32) {
        if self.known.uniform_is(location, [v.to_bits(), 0, 0, 0]) {
            return self.skip();
        }
        self.count(|c| c.uniforms += 1);
        // SAFETY: see above.
        unsafe { (self.gl.uniform1f)(location, v) }
    }

    pub fn uniform2f(&self, location: i32, x: f32, y: f32) {
        if self
            .known
            .uniform_is(location, [x.to_bits(), y.to_bits(), 0, 0])
        {
            return self.skip();
        }
        self.count(|c| c.uniforms += 1);
        // SAFETY: see above.
        unsafe { (self.gl.uniform2f)(location, x, y) }
    }

    pub fn uniform3f(&self, location: i32, x: f32, y: f32, z: f32) {
        if self
            .known
            .uniform_is(location, [x.to_bits(), y.to_bits(), z.to_bits(), 0])
        {
            return self.skip();
        }
        self.count(|c| c.uniforms += 1);
        // SAFETY: see above.
        unsafe { (self.gl.uniform3f)(location, x, y, z) }
    }

    pub fn uniform4f(&self, location: i32, x: f32, y: f32, z: f32, w: f32) {
        if self.known.uniform_is(
            location,
            [x.to_bits(), y.to_bits(), z.to_bits(), w.to_bits()],
        ) {
            return self.skip();
        }
        self.count(|c| c.uniforms += 1);
        // SAFETY: see above.
        unsafe { (self.gl.uniform4f)(location, x, y, z, w) }
    }

    /// Sets a uniform array of `values.len() / 4` vectors of four floats, from its first element's `location`.
    pub fn uniform4fv(&self, location: i32, values: &[f32]) {
        self.touch();
        debug_assert!(!values.is_empty() && values.len() % 4 == 0);
        // SAFETY: see above; `values` holds the whole count of vectors the call reads.
        unsafe { (self.gl.uniform4fv)(location, (values.len() / 4) as i32, values.as_ptr()) }
    }

    pub fn viewport(&self, x: i32, y: i32, w: i32, h: i32) {
        if self.known.on && self.known.viewport.get() == Some([x, y, w, h]) {
            return self.skip();
        }
        self.known.viewport.set(Some([x, y, w, h]));
        self.count(|c| c.states += 1);
        // SAFETY: see above.
        unsafe { (self.gl.viewport)(x, y, w, h) }
    }

    pub fn scissor(&self, x: i32, y: i32, w: i32, h: i32) {
        if self.known.on && self.known.scissor.get() == Some([x, y, w, h]) {
            return self.skip();
        }
        self.known.scissor.set(Some([x, y, w, h]));
        self.count(|c| c.states += 1);
        // SAFETY: see above.
        unsafe { (self.gl.scissor)(x, y, w, h) }
    }

    pub fn enable(&self, capability: u32) {
        if self.known.on {
            if let Some(known) = self.known.capability(capability) {
                if known.replace(Some(true)) == Some(true) {
                    return self.skip();
                }
            }
        }
        self.count(|c| c.states += 1);
        // SAFETY: see above.
        unsafe { (self.gl.enable)(capability) }
    }

    pub fn disable(&self, capability: u32) {
        if self.known.on {
            if let Some(known) = self.known.capability(capability) {
                if known.replace(Some(false)) == Some(false) {
                    return self.skip();
                }
            }
        }
        self.count(|c| c.states += 1);
        // SAFETY: see above.
        unsafe { (self.gl.disable)(capability) }
    }

    pub fn blend_func(&self, source: u32, destination: u32) {
        if self.known.on && self.known.blend_func.get() == Some((source, destination)) {
            return self.skip();
        }
        self.known.blend_func.set(Some((source, destination)));
        self.count(|c| c.states += 1);
        // SAFETY: see above.
        unsafe { (self.gl.blend_func)(source, destination) }
    }

    pub fn clear(&self, r: f32, g: f32, b: f32, a: f32) {
        self.count(|c| c.states += 1);
        // SAFETY: see above.
        unsafe {
            (self.gl.clear_color)(r, g, b, a);
            (self.gl.clear)(GL_COLOR_BUFFER_BIT);
        }
    }

    pub fn bind_framebuffer(&self, framebuffer: u32) {
        self.count(|c| c.states += 1);
        // SAFETY: see above.
        unsafe { (self.gl.bind_framebuffer)(GL_FRAMEBUFFER, framebuffer) }
    }

    /// Hands the queued GL commands to the GPU.
    pub fn flush(&self) {
        self.touch();
        // SAFETY: see above.
        unsafe { (self.gl.flush)() }
    }

    /// Waits until the GPU has done everything handed to it. Only for measuring: a frame never waits.
    pub fn finish(&self) {
        self.touch();
        // SAFETY: see above.
        unsafe { (self.gl.finish)() }
    }

    pub fn bind_texture(&self, texture: u32) {
        if let Some(bound) = self.known.bound() {
            if self.known.on && bound.get() == texture {
                return self.skip();
            }
            bound.set(texture);
        }
        self.count(|c| c.binds += 1);
        // SAFETY: see above.
        unsafe { (self.gl.bind_texture)(GL_TEXTURE_2D, texture) }
    }

    pub fn uniform_location(&self, program: u32, name: &CStr) -> i32 {
        // SAFETY: `name` is NUL-terminated.
        unsafe { (self.gl.get_uniform_location)(program, name.as_ptr()) }
    }

    pub fn delete_texture(&self, texture: u32) {
        self.touch();
        self.known.forget_texture(texture);
        // SAFETY: reads one name.
        unsafe { (self.gl.delete_textures)(1, &texture) }
    }

    /// A texture of `width` × `height` RGBA8 pixels with undefined contents, filtered linearly and clamped at the
    /// edges. It is left bound. Only for a texture whose every pixel is uploaded before anything samples it (an image, an
    /// atlas page: see `Texture` in the renderer); one that is drawn into is a [render target](Gles::new_render_target), and
    /// is made clear.
    pub fn new_texture(&self, width: u32, height: u32) -> u32 {
        let texture = self.make_texture(width, height, false);
        #[cfg(test)]
        self.poison(width, height);
        texture
    }

    /// [`Gles::new_texture`], with transparent black in every pixel: for a texture that something can sample before all of it
    /// has been drawn or uploaded into (a render target, which is drawn into in parts; an image whose file could not be read).
    pub fn new_clear_texture(&self, width: u32, height: u32) -> u32 {
        self.make_texture(width, height, true)
    }

    /// A texture as [`Gles::new_texture`] makes it, with transparent black in every pixel when `clear` (the driver's memory is
    /// not otherwise anything: a new allocation can be what a process that has gone left in it).
    fn make_texture(&self, width: u32, height: u32, clear: bool) -> u32 {
        self.touch();
        let mut texture = 0;
        // `vec![0; n]` is the allocator's zeroed memory, not a loop over it.
        let zeros = clear.then(|| vec![0u8; width as usize * height as usize * 4]);
        let pixels = zeros
            .as_ref()
            .map_or(std::ptr::null(), |zeros| zeros.as_ptr() as *const c_void);
        // SAFETY: creates and configures a texture; `pixels` is null (nothing is read) or the `width * height` RGBA8 pixels
        // of `zeros`, which lives to the end of the function and the upload is complete by the time the call returns.
        unsafe {
            (self.gl.gen_textures)(1, &mut texture);
            (self.gl.bind_texture)(GL_TEXTURE_2D, texture);
            if let Some(bound) = self.known.bound() {
                bound.set(texture);
            }
            (self.gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            (self.gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            (self.gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (self.gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            (self.gl.tex_image_2d)(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8,
                width as i32,
                height as i32,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels,
            );
        }
        texture
    }

    /// Fills the bound texture's `width` × `height` pixels with noise of the seed in [`POISON`] (nothing when it is 0), the way
    /// a driver's memory that held something else would be: random bytes, alpha too, in blocks of 4 by 4 pixels as a tiled GPU
    /// would show them.
    #[cfg(test)]
    pub(crate) fn poison(&self, width: u32, height: u32) {
        let seed = POISON.load(Ordering::Relaxed);
        if seed == 0 {
            return;
        }
        let mut state = seed ^ (u64::from(width) << 20) ^ u64::from(height) ^ 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let blocks_wide = width.div_ceil(4) as usize;
        let blocks_high = height.div_ceil(4) as usize;
        let blocks: Vec<[u8; 4]> = (0..blocks_wide * blocks_high)
            .map(|_| (next() as u32).to_le_bytes())
            .collect();
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height as usize {
            for x in 0..width as usize {
                pixels.extend_from_slice(&blocks[y / 4 * blocks_wide + x / 4]);
            }
        }
        // SAFETY: `pixels` holds `height` rows of `width` pixels, and the texture is `width` by `height`.
        unsafe { self.upload(0, 0, width, height, width, pixels.as_ptr()) };
    }

    /// Makes every pixel of the bound `width` × `height` texture transparent black.
    pub fn blank(&self, width: u32, height: u32) {
        let zeros = vec![0u8; width as usize * height as usize * 4];
        // SAFETY: `zeros` holds the `height` rows of `width` pixels the upload reads.
        unsafe { self.upload(0, 0, width, height, width, zeros.as_ptr()) };
    }

    /// [`Gles::poison`] for a render target, which is made clear: what the noise stands in for is what a frame would read of it
    /// if a bug in it showed what no frame drew, and what the pool's target held for another picture.
    #[cfg(test)]
    pub(crate) fn poison_target(&self, target: &RenderTarget) {
        self.bind_texture(target.texture);
        self.poison(target.width, target.height);
    }

    /// Copies a `width` × `height` block of RGBA8 pixels to (`x`, `y`) of the bound texture. Rows in memory are
    /// `row_pixels` apart, so the block can be read straight out of a larger image.
    ///
    /// # Safety
    /// `pixels` must point at the block's first pixel, with `height` rows of `width` pixels readable at that
    /// `row_pixels` stride.
    pub unsafe fn upload(
        &self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        row_pixels: u32,
        pixels: *const u8,
    ) {
        self.touch();
        // SAFETY: the caller guarantees the memory; the rest is plain GL state.
        unsafe {
            (self.gl.pixel_storei)(GL_UNPACK_ROW_LENGTH, row_pixels as i32);
            (self.gl.tex_sub_image_2d)(
                GL_TEXTURE_2D,
                0,
                x as i32,
                y as i32,
                width as i32,
                height as i32,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels as *const c_void,
            );
            (self.gl.pixel_storei)(GL_UNPACK_ROW_LENGTH, 0);
        }
    }

    /// A texture of `width` × `height` coverage bytes (`R8`: one byte to a pixel, read as the red channel), all zero,
    /// filtered linearly and clamped at the edges: a glyph atlas. It is left bound. Zero and not undefined, because the
    /// filter of a glyph that is not on whole pixels reads the texels next to its rectangle, and they must not be
    /// whatever the driver's memory held.
    pub fn new_alpha_texture(&self, width: u32, height: u32) -> u32 {
        self.touch();
        let zeros = vec![0u8; width as usize * height as usize];
        let mut texture = 0;
        // SAFETY: creates and configures a texture; `zeros` holds the `width * height` bytes the upload reads, one byte
        // to a pixel, so rows need no alignment (and the default is put back).
        unsafe {
            (self.gl.gen_textures)(1, &mut texture);
            (self.gl.bind_texture)(GL_TEXTURE_2D, texture);
            if let Some(bound) = self.known.bound() {
                bound.set(texture);
            }
            (self.gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            (self.gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            (self.gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (self.gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            (self.gl.pixel_storei)(GL_UNPACK_ALIGNMENT, 1);
            (self.gl.tex_image_2d)(
                GL_TEXTURE_2D,
                0,
                GL_R8,
                width as i32,
                height as i32,
                0,
                GL_RED,
                GL_UNSIGNED_BYTE,
                zeros.as_ptr() as *const c_void,
            );
            (self.gl.pixel_storei)(GL_UNPACK_ALIGNMENT, DEFAULT_UNPACK_ALIGNMENT);
        }
        texture
    }

    /// Copies the coverage `bytes` of a `width` × `height` rectangle, rows packed at `width` bytes, to (`x`, `y`) of the
    /// bound alpha texture (see [`Gles::new_alpha_texture`]), which the rectangle must lie inside.
    pub fn upload_alpha(&self, x: u32, y: u32, width: u32, height: u32, bytes: &[u8]) {
        self.touch();
        assert!(
            bytes.len() >= width as usize * height as usize,
            "an upload of {width}x{height} coverage bytes is given {}",
            bytes.len()
        );
        // SAFETY: `bytes` holds the `width * height` bytes the upload reads (checked above), one byte to a pixel, so
        // rows need no alignment (and the default is put back).
        unsafe {
            (self.gl.pixel_storei)(GL_UNPACK_ALIGNMENT, 1);
            (self.gl.tex_sub_image_2d)(
                GL_TEXTURE_2D,
                0,
                x as i32,
                y as i32,
                width as i32,
                height as i32,
                GL_RED,
                GL_UNSIGNED_BYTE,
                bytes.as_ptr() as *const c_void,
            );
            (self.gl.pixel_storei)(GL_UNPACK_ALIGNMENT, DEFAULT_UNPACK_ALIGNMENT);
        }
    }

    /// An RGBA8 texture of `width` × `height` pixels that can be drawn into and then read as a texture: filtered
    /// linearly, clamped at the edges, transparent black until drawn. A target is made clear, not undefined: what a frame
    /// reads of one that was drawn into in part (a plate that only some of its groups were drawn into, a picture the
    /// blur chain could not finish) is nothing, where the driver's memory would be a rectangle of noise that no frame drew.
    /// It costs the allocation one pass over the pixels, once, where targets are made (the pool and the plates keep them).
    pub fn new_render_target(&self, width: u32, height: u32) -> io::Result<RenderTarget> {
        let began = std::time::Instant::now();
        let made = self.make_render_target(width, height);
        TARGETS_MADE.fetch_add(1, Ordering::Relaxed);
        TARGET_PIXELS.fetch_add(u64::from(width) * u64::from(height), Ordering::Relaxed);
        TARGET_NS.fetch_add(began.elapsed().as_nanos() as u64, Ordering::Relaxed);
        made
    }

    fn make_render_target(&self, width: u32, height: u32) -> io::Result<RenderTarget> {
        self.touch();
        let texture = self.new_clear_texture(width, height);
        let mut framebuffer = 0u32;
        // SAFETY: creates a framebuffer object and attaches the texture just made; completeness is checked below.
        let status = unsafe {
            (self.gl.gen_framebuffers)(1, &mut framebuffer);
            (self.gl.bind_framebuffer)(GL_FRAMEBUFFER, framebuffer);
            (self.gl.framebuffer_texture_2d)(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_TEXTURE_2D,
                texture,
                0,
            );
            (self.gl.check_framebuffer_status)(GL_FRAMEBUFFER)
        };
        if status != GL_FRAMEBUFFER_COMPLETE {
            self.delete_render_target(RenderTarget {
                texture,
                framebuffer,
                width,
                height,
            });
            return Err(io::Error::other(format!(
                "a {width}x{height} texture is not a usable render target: framebuffer status {status:#x}"
            )));
        }
        Ok(RenderTarget {
            texture,
            framebuffer,
            width,
            height,
        })
    }

    pub fn delete_render_target(&self, target: RenderTarget) {
        self.touch();
        self.delete_texture(target.texture);
        // SAFETY: reads one name.
        unsafe { (self.gl.delete_fbo)(1, &target.framebuffer) }
    }

    /// Chooses the texture unit that `bind_texture` and the samplers of the next draw use (0 is the default).
    pub fn active_texture(&self, unit: u32) {
        if self.known.on && self.known.unit.get() == unit {
            return self.skip();
        }
        self.known.unit.set(unit);
        self.count(|c| c.binds += 1);
        // SAFETY: see above.
        unsafe { (self.gl.active_texture)(GL_TEXTURE0 + unit) }
    }

    /// The RGBA8 pixels of a `width` × `height` block of the bound framebuffer, from its lower-left corner (`x`, `y`),
    /// row after row from the bottom. It waits for the GPU to finish everything drawn: for the tests and screenshots,
    /// never for a frame.
    pub fn read_pixels(&self, x: i32, y: i32, width: u32, height: u32) -> Vec<u8> {
        self.touch();
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        // SAFETY: the buffer holds exactly `width * height` RGBA8 pixels, and PACK_ALIGNMENT 4 keeps rows unpadded.
        unsafe {
            (self.gl.pixel_storei)(GL_PACK_ALIGNMENT, 4);
            (self.gl.read_pixels)(
                x,
                y,
                width as i32,
                height as i32,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels.as_mut_ptr() as *mut c_void,
            );
        }
        pixels
    }

    /// Draws `count` vertices, starting at vertex `first`, as one triangle strip.
    pub fn draw_triangle_strip(&self, first: i32, count: i32) {
        self.count(|c| c.draws += 1);
        // SAFETY: draws vertices of the buffer bound to the attribute arrays; the caller has filled it.
        unsafe { (self.gl.draw_arrays)(GL_TRIANGLE_STRIP, first, count) }
    }

    /// Draws `count` vertices, starting at vertex `first`, as separate triangles.
    pub fn draw_triangles(&self, first: i32, count: i32) {
        self.count(|c| c.draws += 1);
        // SAFETY: see `draw_triangle_strip`.
        unsafe { (self.gl.draw_arrays)(GL_TRIANGLES, first, count) }
    }

    /// A buffer of 16-bit vertex indices that is written once: [`Gles::draw_indexed`] draws triangles from it.
    pub fn new_index_buffer(&self, indices: &[u16]) -> u32 {
        self.touch();
        let mut buffer = 0;
        // SAFETY: creates a buffer and fills it from a slice's pointer and byte length.
        unsafe {
            (self.gl.gen_buffers)(1, &mut buffer);
            (self.gl.bind_buffer)(GL_ELEMENT_ARRAY_BUFFER, buffer);
            (self.gl.buffer_data)(
                GL_ELEMENT_ARRAY_BUFFER,
                std::mem::size_of_val(indices) as isize,
                indices.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );
        }
        buffer
    }

    /// Draws `count` indices of `indices` (made by [`Gles::new_index_buffer`]) as separate triangles over the vertices
    /// of the vertex buffer in use. Every index must name a vertex of that buffer.
    pub fn draw_indexed(&self, indices: u32, count: i32) {
        self.count(|c| c.draws += 1);
        // SAFETY: draws from an index buffer the caller made for the vertex buffer in use.
        unsafe {
            (self.gl.bind_buffer)(GL_ELEMENT_ARRAY_BUFFER, indices);
            (self.gl.draw_elements)(GL_TRIANGLES, count, GL_UNSIGNED_SHORT, std::ptr::null());
        }
    }

    /// A buffer for streaming vertex data, with attribute 0 (panel position) and 1 (layer position) set up over
    /// interleaved `[f32; 4]` vertices.
    pub fn new_vertex_buffer(&self) -> u32 {
        self.touch();
        let mut buffer = 0;
        // SAFETY: creates a buffer and enables two attribute arrays.
        unsafe {
            (self.gl.gen_buffers)(1, &mut buffer);
            (self.gl.enable_vertex_attrib_array)(0);
            (self.gl.enable_vertex_attrib_array)(1);
        }
        self.use_vertex_buffer(buffer);
        buffer
    }

    /// Makes `buffer` (made by [`Gles::new_vertex_buffer`]) the one draws read from: attribute pointers name the
    /// buffer that was bound when they were set, so drawing from another buffer needs them set again.
    pub fn use_vertex_buffer(&self, buffer: u32) {
        self.touch();
        // SAFETY: binds a buffer and points two attributes at offsets inside it.
        unsafe {
            (self.gl.bind_buffer)(GL_ARRAY_BUFFER, buffer);
            (self.gl.vertex_attrib_pointer)(0, 2, GL_FLOAT, 0, 16, std::ptr::null());
            (self.gl.vertex_attrib_pointer)(1, 2, GL_FLOAT, 0, 16, std::ptr::without_provenance(8));
        }
    }

    /// Replaces the contents of the vertex buffer. Uploading into a fresh store each frame (rather than editing the
    /// old one) keeps the CPU from waiting for the GPU to finish reading the previous frame's vertices.
    pub fn stream_vertices(&self, buffer: u32, vertices: &[[f32; 4]]) {
        self.upload_vertices(buffer, vertices, GL_STREAM_DRAW);
    }

    /// Fills a vertex buffer that is written once and drawn many times (a wallpaper's grid).
    pub fn static_vertices(&self, buffer: u32, vertices: &[[f32; 4]]) {
        self.upload_vertices(buffer, vertices, GL_STATIC_DRAW);
    }

    fn upload_vertices(&self, buffer: u32, vertices: &[[f32; 4]], usage: u32) {
        self.count(|c| c.vertex_bytes += std::mem::size_of_val(vertices) as u64);
        // SAFETY: passes a slice's pointer and byte length.
        unsafe {
            (self.gl.bind_buffer)(GL_ARRAY_BUFFER, buffer);
            (self.gl.buffer_data)(
                GL_ARRAY_BUFFER,
                std::mem::size_of_val(vertices) as isize,
                vertices.as_ptr() as *const c_void,
                usage,
            );
        }
    }

    /// From now on programs wait in `cache` for the renderers that share it: this takes a program of the make it is asked for from there
    /// when one waits, and every program it has (taken or linked) goes back to the cache when it is dropped.
    pub fn use_program_cache(&mut self, cache: Arc<programs::Cache>) {
        self.programs = Some(programs::Lease::new(cache));
    }

    /// The programs this linked itself, and those it took from its cache.
    pub fn programs_linked(&self) -> u32 {
        self.linked.get()
    }

    pub fn programs_taken(&self) -> u32 {
        self.programs.as_ref().map_or(0, programs::Lease::taken)
    }

    /// Says that the renderer this is the GL wrapper of was built whole: the cache has, or this holds, every program the engine uses.
    pub fn programs_complete(&self) {
        if let Some(lease) = &self.programs {
            lease.mark_complete();
        }
    }

    /// A linked program for these shaders: one the cache has, or a new one, compiled and linked here and reporting the driver's log on
    /// failure.
    pub fn program(&self, vertex: &CStr, fragment: &CStr) -> io::Result<u32> {
        self.touch();
        let Some(lease) = &self.programs else {
            return self.link(vertex, fragment);
        };
        if lease.cancelled() {
            return Err(io::Error::other("the programs are not wanted any more"));
        }
        let key = programs::Key::of(vertex, fragment);
        while let Some(program) = lease.take(key) {
            // A program that does not read as linked in this context is not used: another is looked for, and the last resort is a link.
            if self.is_linked(program) {
                self.known.forget_program(program);
                return Ok(program);
            }
            // SAFETY: a program name this lease took out of the cache, and does not hand back.
            unsafe { (self.gl.delete_program)(program) };
            lease.forget(program);
        }
        if lease.strict() {
            // `Unsupported` is how a host tells this miss from the driver's own errors (`programs::Lease`).
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "a program of the engine is not in the cache, and an engine built from a complete cache links none",
            ));
        }
        let program = self.link(vertex, fragment)?;
        lease.hold(key, program);
        Ok(program)
    }

    fn is_linked(&self, program: u32) -> bool {
        let mut linked = 0;
        // SAFETY: a status query that writes one integer, or none for a name that is not a program.
        unsafe { (self.gl.get_programiv)(program, GL_LINK_STATUS, &mut linked) };
        linked != 0
    }

    fn link(&self, vertex: &CStr, fragment: &CStr) -> io::Result<u32> {
        let vs = self.shader(GL_VERTEX_SHADER, vertex)?;
        let fs = self.shader(GL_FRAGMENT_SHADER, fragment)?;
        // SAFETY: program creation and link with status and log checks; the shaders may be deleted once attached.
        unsafe {
            let program = (self.gl.create_program)();
            (self.gl.attach_shader)(program, vs);
            (self.gl.attach_shader)(program, fs);
            (self.gl.link_program)(program);
            (self.gl.delete_shader)(vs);
            (self.gl.delete_shader)(fs);
            let mut linked = 0;
            (self.gl.get_programiv)(program, GL_LINK_STATUS, &mut linked);
            if linked == 0 {
                let log = self.log(|capacity, length, buffer| {
                    (self.gl.get_program_info_log)(program, capacity, length, buffer)
                });
                (self.gl.delete_program)(program);
                return Err(io::Error::other(format!(
                    "shader program does not link: {log}"
                )));
            }
            self.known.forget_program(program);
            self.linked.set(self.linked.get() + 1);
            programs::stand_in_delay();
            Ok(program)
        }
    }

    fn shader(&self, kind: u32, source: &CStr) -> io::Result<u32> {
        // SAFETY: shader creation and compilation with status and log checks.
        unsafe {
            let shader = (self.gl.create_shader)(kind);
            let sources = [source.as_ptr()];
            (self.gl.shader_source)(shader, 1, sources.as_ptr(), std::ptr::null());
            (self.gl.compile_shader)(shader);
            let mut compiled = 0;
            (self.gl.get_shaderiv)(shader, GL_COMPILE_STATUS, &mut compiled);
            if compiled == 0 {
                let log = self.log(|capacity, length, buffer| {
                    (self.gl.get_shader_info_log)(shader, capacity, length, buffer)
                });
                (self.gl.delete_shader)(shader);
                return Err(io::Error::other(format!("shader does not compile: {log}")));
            }
            Ok(shader)
        }
    }

    /// Reads a driver info log through `fetch(capacity, length_out, buffer)`.
    fn log(&self, fetch: impl Fn(i32, *mut i32, *mut c_char)) -> String {
        let mut buffer = vec![0 as c_char; 2048];
        let mut length = 0;
        fetch(buffer.len() as i32, &mut length, buffer.as_mut_ptr());
        let length = usize::try_from(length).unwrap_or(0).min(buffer.len());
        let bytes: Vec<u8> = buffer[..length].iter().map(|&c| c as u8).collect();
        String::from_utf8_lossy(&bytes).trim().to_owned()
    }
}

/// What the toolkit changes of a context that is not its own, and the host of the context relies on ([`Gles::enter`]).
#[derive(Clone, Copy, Debug)]
pub struct Saved {
    program: u32,
    array_buffer: u32,
    element_buffer: u32,
    vertex_array: u32,
    framebuffer: u32,
    viewport: [i32; 4],
    scissor: [i32; 4],
    scissor_test: bool,
    blend: bool,
    depth_stencil_cull: [bool; 3],
    blend_func: [u32; 4],
    active_texture: u32,
    texture: [u32; UNITS],
    attribs: [bool; ATTRIBS],
}

/// The vertex attribute arrays whose state is saved: the toolkit's programs use the first few.
const ATTRIBS: usize = 8;

impl Gles {
    /// A `Gles` over the context that is current on this thread: the one a compositor draws its own frame in, which the toolkit
    /// then draws in as well (`Gles::enter` and `Gles::leave` around each of its calls). Nothing is made and nothing is
    /// destroyed: the display and the context are the host's, and so is every texture it made. `driver` is the EGL/GLES library the
    /// context is the driver's (the entry points are looked up in it).
    ///
    /// The toolkit's shaders are GLSL ES 3.00, so the context has to be OpenGL ES 3.0 or later: an ES 2 one is refused here and not
    /// by the first shader.
    ///
    /// # Safety
    /// A context of `driver` must be current on this thread, and stay so for as long as the returned value is used.
    pub unsafe fn current(driver: &CStr) -> io::Result<Self> {
        let driver = open_driver(driver)?;
        let symbol = |name: &str| -> Option<*const c_void> {
            let name = CString::new(name).ok()?;
            // SAFETY: a symbol lookup in the driver opened above.
            let address = unsafe { libc::dlsym(driver, name.as_ptr()) };
            (!address.is_null()).then_some(address as *const c_void)
        };
        let egl = EglCore::load(&symbol)?;
        let get_proc_address = symbol("eglGetProcAddress")
            .ok_or_else(|| io::Error::other("eglGetProcAddress is missing from the GL driver"))?;
        // SAFETY: `eglGetProcAddress` has this signature.
        let get_proc_address = unsafe {
            std::mem::transmute::<*const c_void, unsafe extern "C" fn(*const c_char) -> *const c_void>(
                get_proc_address,
            )
        };
        let resolve = |name: &str| -> Option<*const c_void> {
            symbol(name).or_else(|| {
                let name = CString::new(name).ok()?;
                // SAFETY: an entry-point lookup by name.
                let address = unsafe { get_proc_address(name.as_ptr()) };
                (!address.is_null()).then_some(address)
            })
        };
        // SAFETY: plain queries of the calling thread's EGL state.
        let (display, context) = unsafe { ((egl.get_current_display)(), (egl.get_current_context)()) };
        if display.is_null() || context.is_null() {
            return Err(io::Error::other("no EGL context is current on this thread"));
        }
        let gl_api = GlApi::load(&resolve)?;
        let gles = Self {
            egl,
            ext: EglExtensions::load(&resolve)?,
            mesh_api: mesh::MeshGl::load(&resolve, &gl_api)?,
            gl: gl_api,
            display,
            window: None,
            calls: Cell::new(Calls::default()),
            touched: Cell::new(0),
            known: Known::new(),
            on_creating_thread: PhantomData,
            programs: None,
            linked: Cell::new(0),
        };
        match gles.version() {
            Some((major, _)) if major >= 3 => Ok(gles),
            other => Err(io::Error::other(format!(
                "the context is {:?} ({other:?}): the toolkit needs OpenGL ES 3.0 (EGL_CONTEXT_CLIENT_VERSION 3)",
                gles.string(GL_VERSION)
            ))),
        }
    }

    /// The version of the context's OpenGL ES, from `GL_VERSION` ("OpenGL ES 3.2 ...").
    pub fn version(&self) -> Option<(u32, u32)> {
        let text = self.string(GL_VERSION);
        let mut numbers = text.split("ES ").nth(1)?.split(|c: char| !c.is_ascii_digit());
        Some((numbers.next()?.parse().ok()?, numbers.next()?.parse().ok()?))
    }

    /// The toolkit is about to draw in a context that someone else draws in too: what that someone has set since the toolkit was
    /// last here is not assumed to be what the toolkit left (so every call it makes goes through), and what the toolkit changes of
    /// the context is saved for [`Gles::leave`]. The toolkit takes it that every state this does not name is the default (depth,
    /// stencil and culling off, all colours written, the blend equation add): a host that changes any of them changes them back.
    pub fn enter(&self) -> Saved {
        const CURRENT_PROGRAM: u32 = 0x8B8D;
        const ARRAY_BUFFER_BINDING: u32 = 0x8894;
        const ELEMENT_ARRAY_BUFFER_BINDING: u32 = 0x8895;
        const VERTEX_ARRAY_BINDING: u32 = 0x85B5;
        const FRAMEBUFFER_BINDING: u32 = 0x8CA6;
        const VIEWPORT: u32 = 0x0BA2;
        const SCISSOR_BOX: u32 = 0x0C10;
        const BLEND_FUNC: [u32; 4] = [0x80C9, 0x80C8, 0x80CB, 0x80CA];
        const ACTIVE_TEXTURE: u32 = 0x84E0;
        const TEXTURE_BINDING_2D: u32 = 0x8069;
        const ATTRIB_ARRAY_ENABLED: u32 = 0x8622;
        // SAFETY: state queries on the current context; each writes the integers it names, and the texture unit is put back.
        let saved = unsafe {
            let get = |name: u32| -> u32 {
                let mut value = 0;
                (self.gl.get_integerv)(name, &mut value);
                value as u32
            };
            let get4 = |name: u32| -> [i32; 4] {
                let mut value = [0; 4];
                (self.gl.get_integerv)(name, value.as_mut_ptr());
                value
            };
            let active_texture = get(ACTIVE_TEXTURE);
            let mut texture = [0; UNITS];
            for (unit, bound) in texture.iter_mut().enumerate() {
                (self.gl.active_texture)(GL_TEXTURE0 + unit as u32);
                *bound = get(TEXTURE_BINDING_2D);
            }
            let mut attribs = [false; ATTRIBS];
            for (index, on) in attribs.iter_mut().enumerate() {
                let mut value = 0;
                (self.gl.get_vertex_attribiv)(index as u32, ATTRIB_ARRAY_ENABLED, &mut value);
                *on = value != 0;
            }
            // the toolkit starts on unit 0
            (self.gl.active_texture)(GL_TEXTURE0);
            Saved {
                program: get(CURRENT_PROGRAM),
                array_buffer: get(ARRAY_BUFFER_BINDING),
                element_buffer: get(ELEMENT_ARRAY_BUFFER_BINDING),
                vertex_array: get(VERTEX_ARRAY_BINDING),
                framebuffer: get(FRAMEBUFFER_BINDING),
                viewport: get4(VIEWPORT),
                scissor: get4(SCISSOR_BOX),
                scissor_test: (self.gl.is_enabled)(GL_SCISSOR_TEST) != 0,
                blend: (self.gl.is_enabled)(GL_BLEND) != 0,
                depth_stencil_cull: [0x0B71u32, 0x0B90, 0x0B44].map(|capability| (self.gl.is_enabled)(capability) != 0),
                blend_func: BLEND_FUNC.map(get),
                active_texture,
                texture,
                attribs,
            }
        };
        self.known.forget_context();
        // The toolkit draws from its own vertex buffers into the default vertex array, with attribute arrays 0 and 1 on and
        // no other: `new_vertex_buffer` turns them on once, and `leave` puts the host's back (a compositor that has not drawn
        // a quad of its own yet has them off, and every draw of the toolkit was then four vertices at the origin: nothing on
        // the page, no GL error, 2026-10-25).
        // SAFETY: binding the default vertex array and setting its attribute arrays.
        unsafe {
            (self.gl.bind_vertex_array)(0);
            (self.gl.enable_vertex_attrib_array)(0);
            (self.gl.enable_vertex_attrib_array)(1);
            for index in 2..ATTRIBS as u32 {
                (self.gl.disable_vertex_attrib_array)(index);
            }
        }
        // What the toolkit sets once and relies on: its blend function (a compositor's own may be another) and no depth test,
        // stencil test or face culling.
        // SAFETY: state calls on the current context.
        unsafe {
            (self.gl.blend_func_separate)(GL_ONE, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
            for capability in [0x0B71u32, 0x0B90, 0x0B44] {
                (self.gl.disable)(capability);
            }
        }
        self.known.blend_func.set(Some((GL_ONE, GL_ONE_MINUS_SRC_ALPHA)));
        saved
    }

    /// The toolkit is done in the context for now: what [`Gles::enter`] saved is put back.
    pub fn leave(&self, saved: &Saved) {
        // SAFETY: state calls on the current context with the values `enter` read from it.
        unsafe {
            (self.gl.bind_vertex_array)(saved.vertex_array);
            (self.gl.bind_buffer)(GL_ARRAY_BUFFER, saved.array_buffer);
            (self.gl.bind_buffer)(GL_ELEMENT_ARRAY_BUFFER, saved.element_buffer);
            (self.gl.use_program)(saved.program);
            (self.gl.bind_framebuffer)(GL_FRAMEBUFFER, saved.framebuffer);
            (self.gl.viewport)(saved.viewport[0], saved.viewport[1], saved.viewport[2], saved.viewport[3]);
            (self.gl.scissor)(saved.scissor[0], saved.scissor[1], saved.scissor[2], saved.scissor[3]);
            for (capability, on) in [(GL_SCISSOR_TEST, saved.scissor_test), (GL_BLEND, saved.blend)] {
                if on {
                    (self.gl.enable)(capability);
                } else {
                    (self.gl.disable)(capability);
                }
            }
            for (capability, on) in [0x0B71u32, 0x0B90, 0x0B44].into_iter().zip(saved.depth_stencil_cull) {
                if on {
                    (self.gl.enable)(capability);
                } else {
                    (self.gl.disable)(capability);
                }
            }
            let [src_rgb, dst_rgb, src_alpha, dst_alpha] = saved.blend_func;
            (self.gl.blend_func_separate)(src_rgb, dst_rgb, src_alpha, dst_alpha);
            for (unit, &bound) in saved.texture.iter().enumerate() {
                (self.gl.active_texture)(GL_TEXTURE0 + unit as u32);
                (self.gl.bind_texture)(GL_TEXTURE_2D, bound);
            }
            (self.gl.active_texture)(saved.active_texture);
            for (index, &on) in saved.attribs.iter().enumerate() {
                if on {
                    (self.gl.enable_vertex_attrib_array)(index as u32);
                } else {
                    (self.gl.disable_vertex_attrib_array)(index as u32);
                }
            }
        }
        self.known.forget_context();
    }
}
