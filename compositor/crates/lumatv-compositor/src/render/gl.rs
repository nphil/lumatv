//! The GL renderer: imports client buffers as textures (driver buffers through `EGL_WAYLAND_BUFFER_WL`, dma-bufs
//! through `EGL_EXT_image_dma_buf_import`, shm by upload) and draws them into an offscreen 1080p framebuffer.
//! C3 swaps the framebuffer for a scan-out buffer behind the same [`Output`] trait.

use std::collections::HashMap;
use std::ffi::{CString, c_void};
use std::io;

use wayland_backend::server::ObjectId;
use wayland_server::Resource;
use wayland_server::protocol::wl_buffer::WlBuffer;

use super::egl::{self, EGLImage, Egl, GLuint, Platform};
use crate::Rect;
use crate::buffer::{BufferKindRef, kind_of};
use crate::output::{FrameClock, Image, Output, Presented, RenderStats};
use crate::scene::Plan;
use crate::surface::Surfaces;

/// Which global serves the clients' GPU buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferPath {
    /// The driver's own `mali_buffer_sharing`, after `eglBindWaylandDisplayWL` (X2 r8b, proven on the set).
    Driver,
    /// The compositor's own global: it keeps the dma-buf fd (C3's direct scan-out needs it) and imports it.
    Own,
}

struct Tex {
    /// Kept for liveness: the entry goes when the client destroys the buffer (`prune`).
    buffer: WlBuffer,
    tex: GLuint,
    image: EGLImage,
    size: (i32, i32),
    y_invert: bool,
    /// Shm textures are re-uploaded on damage; imported ones never change.
    uploaded: bool,
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

pub struct Gl {
    egl: Egl,
    width: u32,
    height: u32,
    targets: Vec<Target>,
    current: TargetId,
    program: GLuint,
    a_pos: GLuint,
    a_uv: GLuint,
    textures: HashMap<ObjectId, Tex>,
    bgra_upload: bool,
    clock: FrameClock,
    path: BufferPath,
    import_failures: u64,
    gl_errors: u64,
    /// The pointer's sprite, uploaded once (RGBA, premultiplied) the first time the cursor shows.
    cursor_tex: Option<GLuint>,
    rects: Vec<Rect>,
    redraw_all: bool,
    #[cfg(feature = "ui")]
    ui: Option<crate::ui::Ui>,
    #[cfg(feature = "ui")]
    ui_frame: Option<lumatv_render::host::FrameCost>,
}

const VS: &str = "attribute vec2 a_pos; attribute vec2 a_uv; varying vec2 v_uv;\nvoid main() { gl_Position = vec4(a_pos, 0.0, 1.0); v_uv = a_uv; }\n";
const FS: &str = "precision mediump float; uniform sampler2D u_tex; varying vec2 v_uv;\nvoid main() { gl_FragColor = texture2D(u_tex, v_uv); }\n";

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
        let (program, a_pos, a_uv) = unsafe {
            let program = link(&egl)?;
            let a_pos = (a.glGetAttribLocation)(program, c"a_pos".as_ptr());
            let a_uv = (a.glGetAttribLocation)(program, c"a_uv".as_ptr());
            (a.glUseProgram)(program);
            (a.glUniform1i)((a.glGetUniformLocation)(program, c"u_tex".as_ptr()), 0);
            (program, a_pos as GLuint, a_uv as GLuint)
        };
        let bgra_upload = egl.has_gl_extension("GL_EXT_texture_format_BGRA8888");
        Ok(Gl {
            egl,
            width,
            height,
            targets: vec![Target { fbo, tex: target, image: std::ptr::null_mut(), top_down: false }],
            current: TargetId(0),
            program,
            a_pos,
            a_uv,
            textures: HashMap::new(),
            bgra_upload,
            clock: FrameClock::new(),
            path,
            import_failures: 0,
            gl_errors: 0,
            cursor_tex: None,
            rects: Vec::new(),
            redraw_all: false,
            #[cfg(feature = "ui")]
            ui: None,
            #[cfg(feature = "ui")]
            ui_frame: None,
        })
    }

    /// Attaches a system-layer client before rendering starts; the returned socket carries its scene protocol.
    #[cfg(feature = "ui")]
    pub fn attach_ui(&mut self, driver: &str, system_z: i32, shm: std::path::PathBuf) -> io::Result<std::os::unix::net::UnixStream> {
        if self.ui.is_some() {
            return Err(io::Error::other("a UI client is already attached"));
        }
        // SAFETY: this renderer owns the current context, and drops the UI before that context.
        let (ui, app) = unsafe { crate::ui::Ui::new(driver, self.width, self.height, system_z, shm) }?;
        self.ui = Some(ui);
        for target in &mut self.targets {
            target.top_down = true;
        }
        Ok(app)
    }

    #[cfg(feature = "ui")]
    pub fn ui(&mut self) -> Option<&mut crate::ui::Ui> {
        self.ui.as_mut()
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

    pub fn plan_ui(&mut self, _damage: &mut crate::damage::Tiles) -> io::Result<()> {
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui {
            ui.plan(lumatv_render::now_ns(), _damage)?;
        }
        Ok(())
    }

    #[cfg(feature = "ui")]
    pub fn take_ui_frame(&mut self) -> Option<lumatv_render::host::FrameCost> {
        self.ui_frame.take()
    }

    #[cfg(feature = "ui")]
    pub fn ui_presented(&mut self, at: u64, cost: &lumatv_render::host::FrameCost) {
        if let Some(ui) = &mut self.ui {
            ui.presented(at, cost);
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

    fn texture_for(&mut self, buffer: &WlBuffer, damage: Option<&[Rect]>, rect: &Rect) -> Option<(GLuint, (i32, i32), bool)> {
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
                    Tex { buffer: buffer.clone(), tex: t, image: std::ptr::null_mut(), size: (shm.width, shm.height), y_invert: true, uploaded: false }
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
                    Tex { buffer: buffer.clone(), tex: bind_image(a, image), image, size: (d.width, d.height), y_invert: d.y_invert, uploaded: true }
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
                    Tex { buffer: buffer.clone(), tex: bind_image(a, image), image, size, y_invert, uploaded: true }
                }
            };
            self.textures.insert(id.clone(), tex);
        }
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
                        let mut rows: Vec<u8> = Vec::with_capacity((r.w * r.h * 4) as usize);
                        for y in r.y..r.bottom() {
                            let off = shm.offset as usize + y as usize * shm.stride as usize + r.x as usize * 4;
                            let Some(row) = bytes.get(off..off + r.w as usize * 4) else { return };
                            if bgra {
                                rows.extend_from_slice(row);
                            } else {
                                for px in row.chunks_exact(4) {
                                    rows.extend_from_slice(&[px[2], px[1], px[0], if shm.format == 1 { 255 } else { px[3] }]);
                                }
                            }
                        }
                        if bgra && shm.format == 1 {
                            for px in rows.chunks_exact_mut(4) {
                                px[3] = 255;
                            }
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
        Some((entry.tex, entry.size, entry.y_invert))
    }

    fn draw_quad(&self, tex: GLuint, rect: &Rect, y_invert: bool) {
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
            (a.glVertexAttribPointer)(self.a_pos, 2, egl::GL_FLOAT, 0, 16, verts.as_ptr() as *const c_void);
            (a.glVertexAttribPointer)(self.a_uv, 2, egl::GL_FLOAT, 0, 16, verts.as_ptr().add(2) as *const c_void);
            (a.glEnableVertexAttribArray)(self.a_pos);
            (a.glEnableVertexAttribArray)(self.a_uv);
            (a.glDrawArrays)(egl::GL_TRIANGLE_STRIP, 0, 4);
        }
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

    /// Draws the plan into the current target, but only inside `rects` (output pixels): the tiles a scan-out page
    /// owes, or the frame's damage bounds.
    pub fn render_rects(&mut self, plan: &Plan, surfaces: &Surfaces, rects: &[Rect]) -> io::Result<RenderStats> {
        let t0 = std::time::Instant::now();
        self.prune(surfaces);
        let mut stats = RenderStats { items: plan.items.len(), ..Default::default() };
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui {
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
            if !self.has_ui() {
                (a.glClearColor)(0.0, 0.0, 0.0, 1.0);
                for r in rects {
                    self.scissor(r);
                    (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
                }
            }
            (a.glEnable)(egl::GL_BLEND);
            (a.glBlendFunc)(egl::GL_ONE, egl::GL_ONE_MINUS_SRC_ALPHA);
        }
        for item in &plan.items {
            let Some((tex, _size, y_invert)) = self.texture_for(&item.buffer, plan.item_damage(item), &item.rect) else {
                stats.skipped += 1;
                continue;
            };
            let mut drawn = false;
            for r in rects {
                let clip = item.rect.intersect(r);
                if clip.is_empty() {
                    continue;
                }
                self.scissor(&clip);
                self.draw_quad(tex, &item.rect, y_invert);
                drawn = true;
            }
            if drawn {
                stats.drawn += 1;
            }
        }
        for h in &plan.holes {
            for r in rects {
                let c = h.intersect(r);
                if c.is_empty() {
                    continue;
                }
                self.scissor(&c);
                // SAFETY: as above.
                unsafe {
                    (self.egl.api.glClearColor)(0.0, 0.0, 0.0, 0.0);
                    (self.egl.api.glClear)(egl::GL_COLOR_BUFFER_BIT);
                }
            }
        }
        #[cfg(feature = "ui")]
        if let Some(ui) = &mut self.ui {
            ui.draw_over(fbo, rects)?;
        }
        self.finish_ui()?;
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
        stats.render_us = t0.elapsed().as_micros() as u64;
        Ok(stats)
    }

    /// Drops textures of buffers the clients destroyed and no surface shows any more. A destroyed buffer that is
    /// still a surface's current one keeps its import: the surface goes on showing it until the client commits
    /// another (the driver's own buffers have no destroy callback here, so liveness is the only signal).
    fn prune(&mut self, surfaces: &Surfaces) {
        let dead: Vec<ObjectId> = self
            .textures
            .iter()
            .filter(|(id, t)| !t.buffer.is_alive() && !surfaces.iter().any(|s| s.current.buffer.as_ref().map(|b| b.id() == **id).unwrap_or(false)))
            .map(|(id, _)| id.clone())
            .collect();
        for id in dead {
            if let Some(t) = self.textures.remove(&id) {
                // SAFETY: objects this renderer created, dropped once.
                unsafe {
                    (self.egl.api.glDeleteTextures)(1, &t.tex);
                    self.egl.destroy_image(t.image);
                }
            }
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
            return Err(egl::other(format!("offscreen framebuffer incomplete: 0x{status:x} (gl 0x{:x})", (a.glGetError)())));
        }
        (a.glViewport)(0, 0, width as i32, height as i32);
        (a.glDisable)(egl::GL_SCISSOR_TEST);
        (a.glClearColor)(0.0, 0.0, 0.0, 1.0);
        (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
        Ok((fbo, tex))
    }
}

fn link(egl: &Egl) -> io::Result<GLuint> {
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
    let vs = compile(egl::GL_VERTEX_SHADER, VS)?;
    let fs = compile(egl::GL_FRAGMENT_SHADER, FS)?;
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
            for t in self.textures.values() {
                (self.egl.api.glDeleteTextures)(1, &t.tex);
                self.egl.destroy_image(t.image);
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

    fn forget_buffer(&mut self, buffer: &WlBuffer) {
        if let Some(t) = self.textures.remove(&buffer.id()) {
            // SAFETY: objects this renderer created, dropped once.
            unsafe {
                (self.egl.api.glDeleteTextures)(1, &t.tex);
                self.egl.destroy_image(t.image);
            }
        }
    }

    fn ui_needs_frame(&mut self) -> io::Result<bool> {
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
            if ui.wants_frame(now) {
                return Ok(true);
            }
            ui.idle(now);
            return Ok(ui.wants_frame(now));
        }
        Ok(false)
    }

    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats> {
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
