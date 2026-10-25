#![allow(non_snake_case)]
//! EGL 1.5 and OpenGL ES 2 entry points of the 64-bit Mali blob, loaded at run time.
//!
//! The blob (`libmali-bifrost-g52-g24p0-wayland-gbm.so`, Rockchip's build for the same Bifrost G52 that LG's
//! 32-bit `libmali.so.0.46.0` drives) is opened by path with `dlopen`, so this crate builds on any host and only
//! needs the driver where it runs. The display is created the way X2 r8b proved on the set: the GBM platform with a
//! null device (the blob refuses LG's render node for GBM) and, when no pbuffer config exists, a no-config
//! surfaceless context.

use std::ffi::{CStr, CString, c_char, c_void};
use std::io;

pub type EGLDisplay = *mut c_void;
pub type EGLConfig = *mut c_void;
pub type EGLSurface = *mut c_void;
pub type EGLContext = *mut c_void;
pub type EGLImage = *mut c_void;
pub type EGLSync = *mut c_void;
pub type EGLint = i32;
pub type EGLBoolean = u32;
pub type GLenum = u32;
pub type GLuint = u32;
pub type GLint = i32;
pub type GLsizei = i32;
pub type GLfloat = f32;

pub const EGL_FALSE: EGLBoolean = 0;
pub const EGL_NONE: EGLint = 0x3038;
pub const EGL_SUCCESS: EGLint = 0x3000;
pub const EGL_WIDTH: EGLint = 0x3057;
pub const EGL_HEIGHT: EGLint = 0x3056;
pub const EGL_TEXTURE_FORMAT: EGLint = 0x3080;
pub const EGL_WAYLAND_Y_INVERTED_WL: EGLint = 0x31DB;
pub const EGL_WAYLAND_BUFFER_WL: GLenum = 0x31D5;
pub const EGL_LINUX_DMA_BUF_EXT: GLenum = 0x3270;
pub const EGL_LINUX_DRM_FOURCC_EXT: EGLint = 0x3271;
pub const EGL_DMA_BUF_PLANE0_FD_EXT: EGLint = 0x3272;
pub const EGL_DMA_BUF_PLANE0_OFFSET_EXT: EGLint = 0x3273;
pub const EGL_DMA_BUF_PLANE0_PITCH_EXT: EGLint = 0x3274;
pub const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: EGLint = 0x3443;
pub const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: EGLint = 0x3444;
pub const EGL_PLATFORM_GBM_KHR: GLenum = 0x31D7;
pub const EGL_OPENGL_ES_API: GLenum = 0x30A0;
pub const EGL_SURFACE_TYPE: EGLint = 0x3033;
pub const EGL_PBUFFER_BIT: EGLint = 0x0001;
pub const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
pub const EGL_OPENGL_ES2_BIT: EGLint = 0x0004;
pub const EGL_RED_SIZE: EGLint = 0x3024;
pub const EGL_GREEN_SIZE: EGLint = 0x3023;
pub const EGL_BLUE_SIZE: EGLint = 0x3022;
pub const EGL_ALPHA_SIZE: EGLint = 0x3021;
pub const EGL_CONTEXT_CLIENT_VERSION: EGLint = 0x3098;
pub const EGL_VENDOR: EGLint = 0x3053;
pub const EGL_VERSION: EGLint = 0x3054;
pub const EGL_EXTENSIONS: EGLint = 0x3055;
pub const EGL_SYNC_FENCE_KHR: GLenum = 0x30F9;
pub const EGL_SYNC_FLUSH_COMMANDS_BIT_KHR: EGLint = 0x0001;
pub const EGL_FOREVER_KHR: u64 = u64::MAX;
pub const EGL_CONDITION_SATISFIED_KHR: EGLint = 0x30F6;

pub const GL_TEXTURE_2D: GLenum = 0x0DE1;
pub const GL_TEXTURE0: GLenum = 0x84C0;
pub const GL_FRAMEBUFFER: GLenum = 0x8D40;
pub const GL_COLOR_ATTACHMENT0: GLenum = 0x8CE0;
pub const GL_FRAMEBUFFER_COMPLETE: GLenum = 0x8CD5;
pub const GL_RGBA: GLenum = 0x1908;
pub const GL_BGRA_EXT: GLenum = 0x80E1;
pub const GL_UNSIGNED_BYTE: GLenum = 0x1401;
pub const GL_TEXTURE_MIN_FILTER: GLenum = 0x2801;
pub const GL_TEXTURE_MAG_FILTER: GLenum = 0x2800;
pub const GL_TEXTURE_WRAP_S: GLenum = 0x2802;
pub const GL_TEXTURE_WRAP_T: GLenum = 0x2803;
pub const GL_LINEAR: GLint = 0x2601;
pub const GL_NEAREST: GLint = 0x2600;
pub const GL_CLAMP_TO_EDGE: GLint = 0x812F;
pub const GL_UNPACK_ALIGNMENT: GLenum = 0x0CF5;
pub const GL_UNPACK_ROW_LENGTH: GLenum = 0x0CF2;
pub const GL_COLOR_BUFFER_BIT: GLenum = 0x4000;
pub const GL_BLEND: GLenum = 0x0BE2;
pub const GL_SCISSOR_TEST: GLenum = 0x0C11;
pub const GL_ONE: GLenum = 1;
pub const GL_ONE_MINUS_SRC_ALPHA: GLenum = 0x0303;
pub const GL_VERTEX_SHADER: GLenum = 0x8B31;
pub const GL_FRAGMENT_SHADER: GLenum = 0x8B30;
pub const GL_COMPILE_STATUS: GLenum = 0x8B81;
pub const GL_LINK_STATUS: GLenum = 0x8B82;
pub const GL_FLOAT: GLenum = 0x1406;
pub const GL_TRIANGLE_STRIP: GLenum = 0x0005;
pub const GL_EXTENSIONS: GLenum = 0x1F03;
pub const GL_RENDERER: GLenum = 0x1F01;
pub const GL_VERSION: GLenum = 0x1F02;
pub const GL_NO_ERROR: GLenum = 0;

macro_rules! entry_points {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        #[allow(non_snake_case)]
        pub struct Api {
            $(pub $name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }
        impl Api {
            fn load(lib: *mut c_void, gpa: unsafe extern "C" fn(*const c_char) -> *mut c_void, missing: &mut Vec<&'static str>) -> Option<Api> {
                $(
                    let $name = {
                        let cname = concat!(stringify!($name), "\0");
                        // SAFETY: a NUL-terminated literal; dlsym and eglGetProcAddress only look the name up.
                        let mut p = unsafe { libc::dlsym(lib, cname.as_ptr() as *const c_char) };
                        if p.is_null() {
                            p = unsafe { gpa(cname.as_ptr() as *const c_char) };
                        }
                        if p.is_null() {
                            missing.push(stringify!($name));
                        }
                        p
                    };
                )*
                if !missing.is_empty() {
                    return None;
                }
                // SAFETY: every pointer was checked non-null and names a function of the declared C signature.
                Some(unsafe { Api { $($name: std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($arg),*) $(-> $ret)?>($name),)* } })
            }
        }
    };
}

entry_points! {
    eglGetError: fn() -> EGLint;
    eglQueryString: fn(EGLDisplay, EGLint) -> *const c_char;
    eglGetDisplay: fn(*mut c_void) -> EGLDisplay;
    eglInitialize: fn(EGLDisplay, *mut EGLint, *mut EGLint) -> EGLBoolean;
    eglTerminate: fn(EGLDisplay) -> EGLBoolean;
    eglBindAPI: fn(GLenum) -> EGLBoolean;
    eglChooseConfig: fn(EGLDisplay, *const EGLint, *mut EGLConfig, EGLint, *mut EGLint) -> EGLBoolean;
    eglCreateContext: fn(EGLDisplay, EGLConfig, EGLContext, *const EGLint) -> EGLContext;
    eglCreatePbufferSurface: fn(EGLDisplay, EGLConfig, *const EGLint) -> EGLSurface;
    eglMakeCurrent: fn(EGLDisplay, EGLSurface, EGLSurface, EGLContext) -> EGLBoolean;
    eglCreateImageKHR: fn(EGLDisplay, EGLContext, GLenum, *mut c_void, *const EGLint) -> EGLImage;
    eglDestroyImageKHR: fn(EGLDisplay, EGLImage) -> EGLBoolean;
    glGetString: fn(GLenum) -> *const u8;
    glGetError: fn() -> GLenum;
    glGenTextures: fn(GLsizei, *mut GLuint);
    glDeleteTextures: fn(GLsizei, *const GLuint);
    glBindTexture: fn(GLenum, GLuint);
    glActiveTexture: fn(GLenum);
    glTexImage2D: fn(GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, *const c_void);
    glTexSubImage2D: fn(GLenum, GLint, GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *const c_void);
    glTexParameteri: fn(GLenum, GLenum, GLint);
    glPixelStorei: fn(GLenum, GLint);
    glEGLImageTargetTexture2DOES: fn(GLenum, EGLImage);
    glGenFramebuffers: fn(GLsizei, *mut GLuint);
    glDeleteFramebuffers: fn(GLsizei, *const GLuint);
    glBindFramebuffer: fn(GLenum, GLuint);
    glFramebufferTexture2D: fn(GLenum, GLenum, GLenum, GLuint, GLint);
    glCheckFramebufferStatus: fn(GLenum) -> GLenum;
    glViewport: fn(GLint, GLint, GLsizei, GLsizei);
    glScissor: fn(GLint, GLint, GLsizei, GLsizei);
    glEnable: fn(GLenum);
    glDisable: fn(GLenum);
    glClearColor: fn(GLfloat, GLfloat, GLfloat, GLfloat);
    glClear: fn(GLenum);
    glBlendFunc: fn(GLenum, GLenum);
    glCreateShader: fn(GLenum) -> GLuint;
    glShaderSource: fn(GLuint, GLsizei, *const *const c_char, *const GLint);
    glCompileShader: fn(GLuint);
    glGetShaderiv: fn(GLuint, GLenum, *mut GLint);
    glGetShaderInfoLog: fn(GLuint, GLsizei, *mut GLsizei, *mut c_char);
    glCreateProgram: fn() -> GLuint;
    glAttachShader: fn(GLuint, GLuint);
    glLinkProgram: fn(GLuint);
    glGetProgramiv: fn(GLuint, GLenum, *mut GLint);
    glGetProgramInfoLog: fn(GLuint, GLsizei, *mut GLsizei, *mut c_char);
    glUseProgram: fn(GLuint);
    glGetAttribLocation: fn(GLuint, *const c_char) -> GLint;
    glGetUniformLocation: fn(GLuint, *const c_char) -> GLint;
    glUniform1i: fn(GLint, GLint);
    glVertexAttribPointer: fn(GLuint, GLint, GLenum, u8, GLsizei, *const c_void);
    glEnableVertexAttribArray: fn(GLuint);
    glDrawArrays: fn(GLenum, GLint, GLsizei);
    glReadPixels: fn(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *mut c_void);
    glFinish: fn();
    glFlush: fn();
}

/// Entry points that may be absent; each is checked before use.
#[allow(non_snake_case)]
pub struct Optional {
    pub eglGetPlatformDisplayEXT: Option<unsafe extern "C" fn(GLenum, *mut c_void, *const EGLint) -> EGLDisplay>,
    pub eglBindWaylandDisplayWL: Option<unsafe extern "C" fn(EGLDisplay, *mut c_void) -> EGLBoolean>,
    pub eglUnbindWaylandDisplayWL: Option<unsafe extern "C" fn(EGLDisplay, *mut c_void) -> EGLBoolean>,
    pub eglQueryWaylandBufferWL: Option<unsafe extern "C" fn(EGLDisplay, *mut c_void, EGLint, *mut EGLint) -> EGLBoolean>,
    pub eglCreateSyncKHR: Option<unsafe extern "C" fn(EGLDisplay, GLenum, *const EGLint) -> EGLSync>,
    pub eglDestroySyncKHR: Option<unsafe extern "C" fn(EGLDisplay, EGLSync) -> EGLBoolean>,
    pub eglClientWaitSyncKHR: Option<unsafe extern "C" fn(EGLDisplay, EGLSync, EGLint, u64) -> EGLint>,
    pub eglDupNativeFenceFDANDROID: Option<unsafe extern "C" fn(EGLDisplay, EGLSync) -> EGLint>,
}

pub struct Egl {
    lib: *mut c_void,
    pub api: Api,
    pub opt: Optional,
    pub display: EGLDisplay,
    pub context: EGLContext,
    pub extensions: String,
    pub gl_extensions: String,
    pub vendor: String,
    pub version: String,
    pub renderer: String,
}

impl Egl {
    /// Opens `path`, initialises a display and makes an ES 2 context current.
    pub fn open(path: &str) -> io::Result<Egl> {
        let cpath = CString::new(path).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "driver path"))?;
        // SAFETY: dlopen of a path; RTLD_GLOBAL so the blob's own dependencies resolve its symbols.
        let lib = unsafe { libc::dlopen(cpath.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
        if lib.is_null() {
            return Err(other(format!("dlopen {path}: {}", dlerror())));
        }
        // SAFETY: looking up a symbol by a NUL-terminated name.
        let gpa = unsafe { libc::dlsym(lib, c"eglGetProcAddress".as_ptr()) };
        if gpa.is_null() {
            return Err(other("driver has no eglGetProcAddress"));
        }
        // SAFETY: eglGetProcAddress has this signature in every EGL.
        let gpa: unsafe extern "C" fn(*const c_char) -> *mut c_void = unsafe { std::mem::transmute(gpa) };
        let mut missing = Vec::new();
        let Some(api) = Api::load(lib, gpa, &mut missing) else {
            return Err(other(format!("driver lacks {}", missing.join(", "))));
        };
        let lookup = |name: &CStr| -> *mut c_void {
            // SAFETY: as above.
            let mut p = unsafe { libc::dlsym(lib, name.as_ptr()) };
            if p.is_null() {
                p = unsafe { gpa(name.as_ptr()) };
            }
            p
        };
        macro_rules! opt {
            ($name:literal) => {{
                let p = lookup($name);
                // SAFETY: the pointer, when present, is the named EGL entry point, whose C signature the field declares.
                #[allow(clippy::missing_transmute_annotations)]
                if p.is_null() { None } else { Some(unsafe { std::mem::transmute(p) }) }
            }};
        }
        let opt = Optional {
            eglGetPlatformDisplayEXT: opt!(c"eglGetPlatformDisplayEXT").or_else(|| opt!(c"eglGetPlatformDisplay")),
            eglBindWaylandDisplayWL: opt!(c"eglBindWaylandDisplayWL"),
            eglUnbindWaylandDisplayWL: opt!(c"eglUnbindWaylandDisplayWL"),
            eglQueryWaylandBufferWL: opt!(c"eglQueryWaylandBufferWL"),
            eglCreateSyncKHR: opt!(c"eglCreateSyncKHR"),
            eglDestroySyncKHR: opt!(c"eglDestroySyncKHR"),
            eglClientWaitSyncKHR: opt!(c"eglClientWaitSyncKHR"),
            eglDupNativeFenceFDANDROID: opt!(c"eglDupNativeFenceFDANDROID"),
        };

        // SAFETY: EGL calls in the documented order; every handle is checked.
        let (display, context, extensions) = unsafe {
            let mut display: EGLDisplay = std::ptr::null_mut();
            if let Some(gpd) = opt.eglGetPlatformDisplayEXT {
                display = gpd(EGL_PLATFORM_GBM_KHR, std::ptr::null_mut(), std::ptr::null());
            }
            if display.is_null() {
                display = (api.eglGetDisplay)(std::ptr::null_mut());
            }
            if display.is_null() {
                return Err(other("no EGL display"));
            }
            let (mut major, mut minor) = (0, 0);
            if (api.eglInitialize)(display, &mut major, &mut minor) == EGL_FALSE {
                return Err(other(format!("eglInitialize: 0x{:x}", (api.eglGetError)())));
            }
            let extensions = cstr((api.eglQueryString)(display, EGL_EXTENSIONS));
            (api.eglBindAPI)(EGL_OPENGL_ES_API);
            let ctx_attribs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
            let cfg_attribs = [
                EGL_SURFACE_TYPE,
                EGL_PBUFFER_BIT,
                EGL_RENDERABLE_TYPE,
                EGL_OPENGL_ES2_BIT,
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
            let mut config: EGLConfig = std::ptr::null_mut();
            let mut n = 0;
            let mut context: EGLContext = std::ptr::null_mut();
            if (api.eglChooseConfig)(display, cfg_attribs.as_ptr(), &mut config, 1, &mut n) != EGL_FALSE && n > 0 {
                context = (api.eglCreateContext)(display, config, std::ptr::null_mut(), ctx_attribs.as_ptr());
                let pb_attribs = [EGL_WIDTH, 4, EGL_HEIGHT, 4, EGL_NONE];
                let pbuffer = (api.eglCreatePbufferSurface)(display, config, pb_attribs.as_ptr());
                if context.is_null() || pbuffer.is_null() || (api.eglMakeCurrent)(display, pbuffer, pbuffer, context) == EGL_FALSE {
                    log::warn!("egl: pbuffer context failed (0x{:x}); trying surfaceless", (api.eglGetError)());
                    context = std::ptr::null_mut();
                }
            }
            if context.is_null() {
                // The 64-bit blob without a GBM device has no pbuffer configs: EGL_KHR_no_config_context plus
                // EGL_KHR_surfaceless_context (X2 r8b).
                context = (api.eglCreateContext)(display, std::ptr::null_mut(), std::ptr::null_mut(), ctx_attribs.as_ptr());
                if context.is_null() || (api.eglMakeCurrent)(display, std::ptr::null_mut(), std::ptr::null_mut(), context) == EGL_FALSE {
                    return Err(other(format!("surfaceless context: 0x{:x}", (api.eglGetError)())));
                }
            }
            (display, context, extensions)
        };
        // SAFETY: a context is current.
        let (vendor, version, renderer, gl_extensions) = unsafe {
            (
                cstr((api.eglQueryString)(display, EGL_VENDOR)),
                cstr((api.eglQueryString)(display, EGL_VERSION)),
                cstr((api.glGetString)(GL_RENDERER) as *const c_char),
                cstr((api.glGetString)(GL_EXTENSIONS) as *const c_char),
            )
        };
        Ok(Egl { lib, api, opt, display, context, extensions, gl_extensions, vendor, version, renderer })
    }

    pub fn has_extension(&self, name: &str) -> bool {
        self.extensions.split(' ').any(|e| e == name)
    }

    pub fn has_gl_extension(&self, name: &str) -> bool {
        self.gl_extensions.split(' ').any(|e| e == name)
    }

    /// `eglBindWaylandDisplayWL`: the driver registers `mali_buffer_sharing` (and `wl_drm`) on `wl_display`.
    ///
    /// # Safety
    /// `wl_display` is the live `wl_display` of the C libwayland backend this process dispatches.
    pub unsafe fn bind_wayland_display(&self, wl_display: *mut c_void) -> io::Result<()> {
        let Some(bind) = self.opt.eglBindWaylandDisplayWL else {
            return Err(other("driver has no eglBindWaylandDisplayWL"));
        };
        // SAFETY: a live wl_display from the server backend.
        if unsafe { bind(self.display, wl_display) } == EGL_FALSE {
            return Err(other(format!("eglBindWaylandDisplayWL: 0x{:x}", self.error())));
        }
        Ok(())
    }

    pub fn error(&self) -> EGLint {
        // SAFETY: no arguments.
        unsafe { (self.api.eglGetError)() }
    }

    pub fn gl_error(&self) -> GLenum {
        // SAFETY: no arguments.
        unsafe { (self.api.glGetError)() }
    }

    /// Size and row order of a buffer the driver created (`EGL_WAYLAND_BUFFER_WL`).
    ///
    /// # Safety
    /// `resource` is a live `wl_resource` of a `wl_buffer` the bound driver created.
    pub unsafe fn query_wayland_buffer(&self, resource: *mut c_void) -> Option<((i32, i32), bool)> {
        let q = self.opt.eglQueryWaylandBufferWL?;
        let (mut w, mut h, mut inv) = (0, 0, 0);
        // SAFETY: resource is a live wl_resource of a wl_buffer; outputs are stack ints.
        unsafe {
            if q(self.display, resource, EGL_WIDTH, &mut w) == EGL_FALSE || q(self.display, resource, EGL_HEIGHT, &mut h) == EGL_FALSE {
                return None;
            }
            q(self.display, resource, EGL_WAYLAND_Y_INVERTED_WL, &mut inv);
        }
        Some(((w, h), inv != 0))
    }

    /// # Safety
    /// `resource` is a live `wl_resource` of a `wl_buffer` the bound driver created.
    pub unsafe fn image_from_wayland_buffer(&self, resource: *mut c_void) -> io::Result<EGLImage> {
        // SAFETY: as above; a null image is an error.
        let img = unsafe { (self.api.eglCreateImageKHR)(self.display, std::ptr::null_mut(), EGL_WAYLAND_BUFFER_WL, resource, std::ptr::null()) };
        if img.is_null() {
            return Err(other(format!("eglCreateImageKHR(EGL_WAYLAND_BUFFER_WL): 0x{:x}", self.error())));
        }
        Ok(img)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn image_from_dmabuf(&self, fd: i32, width: i32, height: i32, fourcc: u32, offset: u32, pitch: u32, modifier: u64) -> io::Result<EGLImage> {
        let mut attribs = vec![
            EGL_WIDTH,
            width,
            EGL_HEIGHT,
            height,
            EGL_LINUX_DRM_FOURCC_EXT,
            fourcc as EGLint,
            EGL_DMA_BUF_PLANE0_FD_EXT,
            fd,
            EGL_DMA_BUF_PLANE0_OFFSET_EXT,
            offset as EGLint,
            EGL_DMA_BUF_PLANE0_PITCH_EXT,
            pitch as EGLint,
        ];
        if modifier != 0 || self.has_extension("EGL_EXT_image_dma_buf_import_modifiers") {
            attribs.extend_from_slice(&[
                EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT,
                modifier as u32 as EGLint,
                EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT,
                (modifier >> 32) as u32 as EGLint,
            ]);
        }
        attribs.push(EGL_NONE);
        // SAFETY: attribute list is NONE-terminated; the fd is a live dma-buf.
        let img = unsafe { (self.api.eglCreateImageKHR)(self.display, std::ptr::null_mut(), EGL_LINUX_DMA_BUF_EXT, std::ptr::null_mut(), attribs.as_ptr()) };
        if img.is_null() {
            return Err(other(format!(
                "eglCreateImageKHR(dma-buf {width}x{height} {} mod 0x{modifier:x}): 0x{:x}",
                crate::buffer::fourcc::name(fourcc),
                self.error()
            )));
        }
        Ok(img)
    }

    /// # Safety
    /// `image` came from this display and is destroyed once.
    pub unsafe fn destroy_image(&self, image: EGLImage) {
        if !image.is_null() {
            // SAFETY: an image this object created.
            unsafe { (self.api.eglDestroyImageKHR)(self.display, image) };
        }
    }

    /// A fence after the draw (LumaShow lesson 3: on Mali a fence, not `glFlush`, is what starts the work).
    pub fn kick_fence(&self) -> Option<EGLSync> {
        let create = self.opt.eglCreateSyncKHR?;
        // SAFETY: a fence sync with no attributes.
        let sync = unsafe { create(self.display, EGL_SYNC_FENCE_KHR, std::ptr::null()) };
        if sync.is_null() { None } else { Some(sync) }
    }

    /// # Safety
    /// `sync` came from `kick_fence` on this display and is not used afterwards.
    pub unsafe fn wait_fence(&self, sync: EGLSync, timeout_ns: u64) -> bool {
        let (Some(wait), Some(destroy)) = (self.opt.eglClientWaitSyncKHR, self.opt.eglDestroySyncKHR) else { return false };
        // SAFETY: a sync this object created, destroyed once.
        unsafe {
            let r = wait(self.display, sync, EGL_SYNC_FLUSH_COMMANDS_BIT_KHR, timeout_ns);
            destroy(self.display, sync);
            r == EGL_CONDITION_SATISFIED_KHR
        }
    }
}

impl Drop for Egl {
    fn drop(&mut self) {
        // SAFETY: tearing down what `open` made; the library stays loaded (the blob does not unload cleanly).
        unsafe {
            (self.api.eglMakeCurrent)(self.display, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
            (self.api.eglTerminate)(self.display);
        }
        let _ = self.lib;
    }
}

fn dlerror() -> String {
    // SAFETY: dlerror returns a static or thread-local string, or null.
    unsafe { cstr(libc::dlerror()) }
}

/// Copies a C string; empty for null.
///
/// # Safety
/// `p` is null or points at a NUL-terminated string.
pub unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

pub fn other(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}
