//! The box dim in the GL renderer (`boxdim`): a copy of the item program with one more texel fetch, the map's texture, and the sample
//! that reads the app's pictures at the grid's resolution.
//!
//! The gained program is used for app items only while the plan carries a map; every other item, and every frame without a map, goes
//! through the plain program as before. The map is a 64 x 36 RGBA texture (the gain in every channel), re-uploaded when the plan's serial
//! changes, sampled bilinearly at the fragment's screen position (`v_gain`, from the vertex position; `u_gy` turns it for a target whose
//! row 0 is the top).
//!
//! The sample is two passes of a 15-tap box filter, taps two output pixels apart (a bilinear tap between two texels averages both, so 15
//! taps are 30 pixels exactly): across, the app items into a 64 x 1080 target (each column a cell's width, holes cleared black before
//! their item), then down into 64 x 36. Each cell gets its mean luma, that of its left 14 columns and that of its lower 14 rows. A pass of
//! 15 taps is a small shader: the single pass of 15 x 15 taps it replaces cost the loop 56 ms the first time on the set (round 16, its
//! build and first draw), and its read 2.8 ms each time after, the GPU waited for. Now the read waits for nothing: the passes are
//! followed by a fence, and the 9 KB are read on a later tick, once the fence has signalled. The programs and targets are made ahead, a
//! piece per quiet tick (`Gl::boxdim_warm`).

use std::ffi::c_void;
use std::io;
use std::time::{Duration, Instant};

use wayland_server::Resource;

use super::{Gl, VS, egl, link, set_params};
use crate::boxdim::{CELL, CELLS, COLS, ROWS, Sig};
use crate::scene::{Item, Plan};
use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect};
use egl::GLuint;

/// A sample whose fence has not signalled by then is given up.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(1);

const GAIN_VS: &str = "attribute vec2 a_pos; attribute vec2 a_uv; uniform vec2 u_gy; varying vec2 v_uv; varying vec2 v_gain;\n\
void main() { gl_Position = vec4(a_pos, 0.0, 1.0); v_uv = a_uv; v_gain = vec2(a_pos.x * 0.5 + 0.5, (a_pos.y * 0.5 + 0.5) * u_gy.x + u_gy.y); }\n";
/// The item program's fragment shader (`gl.rs` `FS`) times the map: premultiplied, the alpha left.
const GAIN_FS: &str = "precision mediump float; uniform sampler2D u_tex; uniform sampler2D u_gain; uniform float u_dim; varying vec2 v_uv; varying vec2 v_gain;\n\
void main() { vec4 c = texture2D(u_tex, v_uv); gl_FragColor = vec4(c.rgb * (u_dim * texture2D(u_gain, v_gain).r), c.a); }\n";
/// Across: 15 taps `u_step` apart (two pixels of the item's texture) centred on a cell's column; out, premultiplied, the luma of all 30
/// pixels, of the left 14 and of all 30 again (the lower part is the next pass's), and the alpha. The taps' positions need full precision
/// (a step is a thousandth of the texture).
const ACROSS_FS: &str = "#ifdef GL_FRAGMENT_PRECISION_HIGH\nprecision highp float;\n#else\nprecision mediump float;\n#endif\n\
uniform sampler2D u_tex; uniform float u_step; varying vec2 v_uv;\n\
void main() {\n\
  vec4 sum = vec4(0.0); vec4 part = vec4(0.0);\n\
  for (int i = 0; i < 15; i++) {\n\
    vec4 c = texture2D(u_tex, v_uv + vec2((float(i) - 7.0) * u_step, 0.0));\n\
    sum += c;\n\
    if (i < 7) { part += c; }\n\
  }\n\
  vec3 w = vec3(0.2126, 0.7152, 0.0722);\n\
  float whole = dot(sum.rgb, w) / 15.0;\n\
  gl_FragColor = vec4(whole, dot(part.rgb, w) / 7.0, whole, sum.a / 15.0);\n\
}\n";
/// Down: 15 taps `u_step` apart (two rows of the across target) centred on a cell's row; out, the mean of the cell, of its left part and
/// of its lower 14 rows (the lower taps: row 0 of a target is its bottom).
const DOWN_FS: &str = "#ifdef GL_FRAGMENT_PRECISION_HIGH\nprecision highp float;\n#else\nprecision mediump float;\n#endif\n\
uniform sampler2D u_tex; uniform float u_step; varying vec2 v_uv;\n\
void main() {\n\
  vec4 sum = vec4(0.0); vec4 part = vec4(0.0);\n\
  for (int i = 0; i < 15; i++) {\n\
    vec4 c = texture2D(u_tex, v_uv + vec2(0.0, (float(i) - 7.0) * u_step));\n\
    sum += c;\n\
    if (i < 7) { part += c; }\n\
  }\n\
  gl_FragColor = vec4(sum.r / 15.0, sum.g / 15.0, part.b / 7.0, 1.0);\n\
}\n";

struct Gain {
    id: GLuint,
    a_pos: GLuint,
    a_uv: GLuint,
    u_dim: i32,
    u_gy: i32,
    /// The map's texture (unit 1) and the plan serial it holds.
    tex: GLuint,
    serial: Option<u64>,
    dim: f32,
    gy: (f32, f32),
}

/// One pass of the sample: its program and where its uniforms are.
struct Pass {
    id: GLuint,
    a_pos: GLuint,
    a_uv: GLuint,
    u_step: i32,
}

/// The sample's two targets: across (64 x 1080, read bilinearly by the pass down) and down (64 x 36, read back).
struct Targets {
    across_fbo: GLuint,
    across_tex: GLuint,
    down_fbo: GLuint,
}

/// A sample begun and not read yet: the fence after its passes (`None` without fences: the read waits for the GPU then).
#[derive(Clone, Copy)]
struct InFlight {
    sync: Option<egl::EGLSync>,
    since: Instant,
}

/// What the renderer keeps for the box dim, made ahead by [`Gl::boxdim_warm`] (or when first needed).
#[derive(Default)]
pub(super) struct BoxDim {
    gain: Option<Gain>,
    across: Option<Pass>,
    down: Option<Pass>,
    targets: Option<Targets>,
    in_flight: Option<InFlight>,
    pixels: Vec<u8>,
    failed: bool,
}

impl Gl {
    fn gain_program(&mut self) -> Option<&Gain> {
        if self.boxdim.gain.is_none() && !self.boxdim.failed {
            match self.make_gain_program() {
                Ok(g) => self.boxdim.gain = Some(g),
                Err(e) => {
                    self.boxdim.failed = true;
                    log::error!("the box dim's shader: {e}; nothing is dimmed");
                }
            }
        }
        self.boxdim.gain.as_ref()
    }

    fn make_gain_program(&self) -> io::Result<Gain> {
        let id = link(&self.egl, GAIN_VS, GAIN_FS)?;
        let a = &self.egl.api;
        // SAFETY: lookups and uniform sets on a linked program of this context; the texture is this renderer's.
        unsafe {
            (a.glUseProgram)(id);
            (a.glUniform1i)((a.glGetUniformLocation)(id, c"u_tex".as_ptr()), 0);
            (a.glUniform1i)((a.glGetUniformLocation)(id, c"u_gain".as_ptr()), 1);
            let u_dim = (a.glGetUniformLocation)(id, c"u_dim".as_ptr());
            let u_gy = (a.glGetUniformLocation)(id, c"u_gy".as_ptr());
            (a.glUniform1f)(u_dim, 1.0);
            (a.glUniform2f)(u_gy, 1.0, 0.0);
            let mut tex = 0;
            (a.glGenTextures)(1, &mut tex);
            (a.glActiveTexture)(egl::GL_TEXTURE1);
            (a.glBindTexture)(egl::GL_TEXTURE_2D, tex);
            set_params(a);
            let flat = [255u8; 4];
            (a.glTexImage2D)(egl::GL_TEXTURE_2D, 0, egl::GL_RGBA as i32, 1, 1, 0, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, flat.as_ptr() as *const c_void);
            (a.glActiveTexture)(egl::GL_TEXTURE0);
            (a.glUseProgram)(self.program);
            Ok(Gain {
                id,
                a_pos: (a.glGetAttribLocation)(id, c"a_pos".as_ptr()) as GLuint,
                a_uv: (a.glGetAttribLocation)(id, c"a_uv".as_ptr()) as GLuint,
                u_dim,
                u_gy,
                tex,
                serial: None,
                dim: 1.0,
                gy: (1.0, 0.0),
            })
        }
    }

    /// Before a frame's items: the map's texture holds the plan's map and is bound on unit 1, the gained program knows which way the
    /// target's rows run. Whether app items are to be drawn through it this frame.
    pub(super) fn boxdim_frame(&mut self, plan: &Plan) -> bool {
        let Some(map) = plan.gain() else { return false };
        let top_down = self.targets[self.current.0].top_down;
        if self.gain_program().is_none() {
            return false;
        }
        let a = &self.egl.api;
        let g = self.boxdim.gain.as_mut().unwrap();
        // SAFETY: state calls on the current context; the texture and program are this renderer's; the upload is 64 x 36 x 4 bytes.
        unsafe {
            (a.glActiveTexture)(egl::GL_TEXTURE1);
            (a.glBindTexture)(egl::GL_TEXTURE_2D, g.tex);
            if g.serial != Some(plan.gain_serial) {
                let mut rgba = [0u8; CELLS * 4];
                for (px, c) in rgba.chunks_exact_mut(4).zip(map.cells.iter()) {
                    px.fill(*c);
                }
                (a.glPixelStorei)(egl::GL_UNPACK_ALIGNMENT, 4);
                (a.glTexImage2D)(
                    egl::GL_TEXTURE_2D,
                    0,
                    egl::GL_RGBA as i32,
                    COLS as i32,
                    ROWS as i32,
                    0,
                    egl::GL_RGBA,
                    egl::GL_UNSIGNED_BYTE,
                    rgba.as_ptr() as *const c_void,
                );
                g.serial = Some(plan.gain_serial);
            }
            (a.glActiveTexture)(egl::GL_TEXTURE0);
            let gy = if top_down { (1.0, 0.0) } else { (-1.0, 1.0) };
            if g.gy != gy {
                (a.glUseProgram)(g.id);
                (a.glUniform2f)(g.u_gy, gy.0, gy.1);
                (a.glUseProgram)(self.program);
                g.gy = gy;
            }
        }
        true
    }

    /// An app item's quad through the gained program (the plain one is current before and after), at the item's brightness.
    pub(super) fn draw_gained(&mut self, tex: GLuint, rect: &Rect, y_invert: bool, dim: f32) {
        let Some(g) = self.boxdim.gain.as_mut() else { return };
        let a = &self.egl.api;
        // SAFETY: program switches and a uniform set on this renderer's programs.
        unsafe {
            (a.glUseProgram)(g.id);
            if g.dim != dim {
                (a.glUniform1f)(g.u_dim, dim);
                g.dim = dim;
            }
        }
        let (a_pos, a_uv) = (g.a_pos, g.a_uv);
        self.draw_quad_with(a_pos, a_uv, tex, rect, y_invert);
        // SAFETY: as above.
        unsafe { (self.egl.api.glUseProgram)(self.program) };
    }

    fn make_pass(&self, fs: &str) -> io::Result<Pass> {
        let id = link(&self.egl, VS, fs)?;
        let a = &self.egl.api;
        // SAFETY: lookups and a uniform set on a linked program of this context.
        unsafe {
            (a.glUseProgram)(id);
            (a.glUniform1i)((a.glGetUniformLocation)(id, c"u_tex".as_ptr()), 0);
            let pass = Pass {
                id,
                a_pos: (a.glGetAttribLocation)(id, c"a_pos".as_ptr()) as GLuint,
                a_uv: (a.glGetAttribLocation)(id, c"a_uv".as_ptr()) as GLuint,
                u_step: (a.glGetUniformLocation)(id, c"u_step".as_ptr()),
            };
            (a.glUseProgram)(self.program);
            Ok(pass)
        }
    }

    fn make_targets(&self) -> io::Result<Targets> {
        let a = &self.egl.api;
        let target = |w: usize, h: usize, linear: bool| -> io::Result<(GLuint, GLuint)> {
            // SAFETY: GL object creation with a current context; the framebuffer is checked.
            unsafe {
                let mut tex = 0;
                (a.glGenTextures)(1, &mut tex);
                (a.glBindTexture)(egl::GL_TEXTURE_2D, tex);
                (a.glTexImage2D)(egl::GL_TEXTURE_2D, 0, egl::GL_RGBA as i32, w as i32, h as i32, 0, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, std::ptr::null());
                if linear {
                    set_params(a);
                } else {
                    (a.glTexParameteri)(egl::GL_TEXTURE_2D, egl::GL_TEXTURE_MIN_FILTER, egl::GL_NEAREST);
                    (a.glTexParameteri)(egl::GL_TEXTURE_2D, egl::GL_TEXTURE_MAG_FILTER, egl::GL_NEAREST);
                }
                let mut fbo = 0;
                (a.glGenFramebuffers)(1, &mut fbo);
                (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
                (a.glFramebufferTexture2D)(egl::GL_FRAMEBUFFER, egl::GL_COLOR_ATTACHMENT0, egl::GL_TEXTURE_2D, tex, 0);
                let status = (a.glCheckFramebufferStatus)(egl::GL_FRAMEBUFFER);
                if status != egl::GL_FRAMEBUFFER_COMPLETE {
                    (a.glDeleteFramebuffers)(1, &fbo);
                    (a.glDeleteTextures)(1, &tex);
                    return Err(egl::other(format!("box dim sample target incomplete: 0x{status:x}")));
                }
                Ok((fbo, tex))
            }
        };
        let (across_fbo, across_tex) = target(COLS, OUTPUT_HEIGHT as usize, true)?;
        let (down_fbo, _) = target(COLS, ROWS, false)?;
        Ok(Targets { across_fbo, across_tex, down_fbo })
    }

    /// Makes the next piece of what the box dim needs: the pass across, the pass down, the gained program, then the targets with a first
    /// draw through each program (the driver finishes a program at its first draw). `Ok(true)` once all is made. One piece is a compile or
    /// a few small draws: milliseconds, on a tick that has nothing else to do.
    pub fn boxdim_warm(&mut self) -> io::Result<bool> {
        if self.boxdim.failed {
            return Err(egl::other("the box dim's shaders did not build"));
        }
        let made = if self.boxdim.across.is_none() {
            self.make_pass(ACROSS_FS).map(|p| self.boxdim.across = Some(p))
        } else if self.boxdim.down.is_none() {
            self.make_pass(DOWN_FS).map(|p| self.boxdim.down = Some(p))
        } else if self.boxdim.gain.is_none() {
            self.make_gain_program().map(|g| self.boxdim.gain = Some(g))
        } else if self.boxdim.targets.is_none() {
            self.make_targets().map(|t| {
                self.boxdim.targets = Some(t);
                self.first_draws();
            })
        } else {
            return Ok(true);
        };
        self.restore_target();
        match made {
            Ok(()) => Ok(self.boxdim.targets.is_some()),
            Err(e) => {
                self.boxdim.failed = true;
                log::error!("the box dim's sample: {e}; damage is taken for change, nothing is dimmed");
                Err(e)
            }
        }
    }

    /// One draw through each of the box dim's programs into its own targets, with nothing in them: what the driver does at a program's
    /// first draw is done here, not in a frame.
    fn first_draws(&mut self) {
        let (Some(t), Some(across), Some(down), Some(g)) = (&self.boxdim.targets, &self.boxdim.across, &self.boxdim.down, &self.boxdim.gain) else {
            return;
        };
        let full = Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
        let a = &self.egl.api;
        let blank = self.xf.blank;
        // SAFETY: state calls and draws on the current context with this renderer's objects; client-side vertex arrays of 16 floats.
        unsafe {
            (a.glDisable)(egl::GL_SCISSOR_TEST);
            (a.glDisable)(egl::GL_BLEND);
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, t.across_fbo);
            (a.glViewport)(0, 0, COLS as i32, OUTPUT_HEIGHT as i32);
            (a.glUseProgram)(across.id);
            sample_quad(a, across, blank, &full, true, 2.0 / OUTPUT_WIDTH as f32);
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, t.down_fbo);
            (a.glViewport)(0, 0, COLS as i32, ROWS as i32);
            (a.glUseProgram)(down.id);
            sample_quad(a, down, t.across_tex, &full, false, 2.0 / OUTPUT_HEIGHT as f32);
            (a.glUseProgram)(g.id);
            (a.glActiveTexture)(egl::GL_TEXTURE1);
            (a.glBindTexture)(egl::GL_TEXTURE_2D, g.tex);
            (a.glActiveTexture)(egl::GL_TEXTURE0);
            sample_quad(a, &Pass { id: g.id, a_pos: g.a_pos, a_uv: g.a_uv, u_step: -1 }, blank, &full, true, 0.0);
        }
        if let Some(sync) = self.egl.kick_fence() {
            // SAFETY: a fence this renderer just created, consumed here (a zero timeout only starts the work).
            unsafe { self.egl.wait_fence(sync, 0) };
        }
    }

    /// Makes at once whatever of the sample the warm-up has not made yet (the sample is wanted before the warm-up finished).
    fn ensure_sampler(&mut self) -> io::Result<()> {
        while self.boxdim.across.is_none() || self.boxdim.down.is_none() || self.boxdim.targets.is_none() {
            self.boxdim_warm()?;
        }
        Ok(())
    }

    /// Begins the box dim's sample of the plan's app items (not posed, not ghosts; each where its window places it, `Item::drift` taken
    /// away): the two passes, then a fence. `Ok(false)` while one is in flight or an item's buffer is still warming (a draw from it would
    /// wait on the driver); an error when the sample cannot be made at all.
    pub fn sample_begin(&mut self, plan: &Plan) -> io::Result<bool> {
        if self.boxdim.in_flight.is_some() {
            return Ok(false);
        }
        let sampled = |i: &Item| i.gain && i.pose.is_none() && !i.ghost;
        let warming = plan
            .items
            .iter()
            .filter(|i| sampled(i))
            .any(|i| self.textures.get(&i.buffer.id()).is_some_and(|t| t.external && (t.warm_sync.is_some() || t.held)));
        if warming {
            return Ok(false);
        }
        self.ensure_sampler()?;
        let (across_fbo, across_tex, down_fbo) = {
            let t = self.boxdim.targets.as_ref().unwrap();
            (t.across_fbo, t.across_tex, t.down_fbo)
        };
        let a = &self.egl.api;
        // SAFETY: state calls on the current context and this renderer's objects.
        unsafe {
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, across_fbo);
            (a.glViewport)(0, 0, COLS as i32, OUTPUT_HEIGHT as i32);
            (a.glDisable)(egl::GL_SCISSOR_TEST);
            (a.glClearColor)(0.0, 0.0, 0.0, 1.0);
            (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
            (a.glEnable)(egl::GL_BLEND);
            (a.glBlendFunc)(egl::GL_ONE, egl::GL_ONE_MINUS_SRC_ALPHA);
        }
        for (k, item) in plan.items.iter().enumerate() {
            if !sampled(item) {
                continue;
            }
            for hole in plan.holes.iter().filter(|h| h.before == k) {
                // Every cell column the hole touches, its own rows: what is under the video is not bright.
                let Some((c0, _, c1, _)) = crate::boxdim::cells_of(&hole.rect) else { continue };
                let a = &self.egl.api;
                // SAFETY: as above; the scissor's rows count from the target's bottom.
                unsafe {
                    (a.glEnable)(egl::GL_SCISSOR_TEST);
                    (a.glScissor)(c0 as i32, OUTPUT_HEIGHT as i32 - hole.rect.bottom(), (c1 - c0 + 1) as i32, hole.rect.h);
                    (a.glClearColor)(0.0, 0.0, 0.0, 0.0);
                    (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
                    (a.glDisable)(egl::GL_SCISSOR_TEST);
                }
            }
            let rect = item.rect.translated(-item.drift.0, -item.drift.1);
            if rect.w <= 0 || rect.h <= 0 {
                continue;
            }
            let Some((tex, _, y_invert, _)) = self.texture_for(&item.buffer, Some(&[]), &item.rect) else { continue };
            let across = self.boxdim.across.as_ref().unwrap();
            // SAFETY: the pass's program on this context, the item's texture.
            unsafe {
                (self.egl.api.glUseProgram)(across.id);
                sample_quad(&self.egl.api, across, tex, &rect, y_invert, 2.0 / rect.w as f32);
            }
        }
        let down = self.boxdim.down.as_ref().unwrap();
        let a = &self.egl.api;
        // SAFETY: as above.
        unsafe {
            (a.glDisable)(egl::GL_BLEND);
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, down_fbo);
            (a.glViewport)(0, 0, COLS as i32, ROWS as i32);
            (a.glUseProgram)(down.id);
            sample_quad(a, down, across_tex, &Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32), false, 2.0 / OUTPUT_HEIGHT as f32);
        }
        let sync = self.egl.kick_fence();
        if sync.is_none() {
            // SAFETY: plain call: without a fence the work is started by a flush.
            unsafe { (self.egl.api.glFlush)() };
        }
        self.boxdim.in_flight = Some(InFlight { sync, since: Instant::now() });
        self.restore_target();
        let err = self.egl.gl_error();
        if err != egl::GL_NO_ERROR {
            return Err(egl::other(format!("box dim sample: gl error 0x{err:x}")));
        }
        Ok(true)
    }

    /// The sample begun last, once its fence has signalled: read into `sigs` (row 0 the top). `Ok(false)` while the GPU is still at it;
    /// an error, and the sample dropped, when it is not done within [`SAMPLE_TIMEOUT`].
    pub fn sample_take(&mut self, sigs: &mut [Sig; CELLS]) -> io::Result<bool> {
        let Some(f) = self.boxdim.in_flight else { return Ok(false) };
        if let Some(sync) = f.sync {
            // SAFETY: a fence this renderer made for this sample and has not destroyed; destroyed once below.
            let done = unsafe { self.egl.fence_signalled(sync, true) };
            if !done && f.since.elapsed() < SAMPLE_TIMEOUT {
                return Ok(false);
            }
            // SAFETY: as above.
            unsafe { self.egl.destroy_fence(sync) };
            if !done {
                self.boxdim.in_flight = None;
                return Err(egl::other("box dim sample: the GPU did not finish it within a second"));
            }
        }
        self.boxdim.in_flight = None;
        let Some(t) = &self.boxdim.targets else { return Ok(false) };
        let fbo = t.down_fbo;
        let mut pixels = std::mem::take(&mut self.boxdim.pixels);
        pixels.resize(CELLS * 4, 0);
        let a = &self.egl.api;
        // SAFETY: the read fills exactly 64 x 36 x 4 bytes of a vector of that size, from this renderer's framebuffer.
        unsafe {
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
            (a.glPixelStorei)(egl::GL_PACK_ALIGNMENT, 4);
            (a.glReadPixels)(0, 0, COLS as i32, ROWS as i32, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, pixels.as_mut_ptr() as *mut c_void);
        }
        for (i, px) in pixels.chunks_exact(4).enumerate() {
            let (c, r) = (i % COLS, i / COLS);
            // Row 0 of the read is the target's bottom: the screen's last row of cells.
            sigs[(ROWS - 1 - r) * COLS + c] = [px[0], px[1], px[2]];
        }
        self.boxdim.pixels = pixels;
        self.restore_target();
        let err = self.egl.gl_error();
        if err != egl::GL_NO_ERROR {
            return Err(egl::other(format!("box dim sample read: gl error 0x{err:x}")));
        }
        Ok(true)
    }
}

/// A quad of `tex` over `rect` (output pixels) into the bound sample target, through `pass` (current), with its step `step` (texture
/// coordinates; a pass without one has a negative location and the set is a no-op). The target's row 0 is its bottom, as the GL draws
/// it: the screen's top is +1. `top_first`: the texture's row 0 is the picture's top (a `y_invert` buffer; the across target is not).
///
/// # Safety
/// A context is current with `pass`'s program in use; `tex` is a texture of it.
unsafe fn sample_quad(a: &egl::Api, pass: &Pass, tex: GLuint, rect: &Rect, top_first: bool, step: f32) {
    let (w, h) = (OUTPUT_WIDTH as f32, OUTPUT_HEIGHT as f32);
    let (x0, x1) = (rect.x as f32 / w * 2.0 - 1.0, rect.right() as f32 / w * 2.0 - 1.0);
    let (y0, y1) = (1.0 - rect.y as f32 / h * 2.0, 1.0 - rect.bottom() as f32 / h * 2.0);
    let (vt, vb) = if top_first { (0.0f32, 1.0f32) } else { (1.0, 0.0) };
    let verts: [f32; 16] = [x0, y0, 0.0, vt, x1, y0, 1.0, vt, x0, y1, 0.0, vb, x1, y1, 1.0, vb];
    // SAFETY: the caller's; client-side vertex arrays of 16 floats.
    unsafe {
        (a.glUniform1f)(pass.u_step, step);
        (a.glActiveTexture)(egl::GL_TEXTURE0);
        (a.glBindTexture)(egl::GL_TEXTURE_2D, tex);
        (a.glVertexAttribPointer)(pass.a_pos, 2, egl::GL_FLOAT, 0, 16, verts.as_ptr() as *const c_void);
        (a.glVertexAttribPointer)(pass.a_uv, 2, egl::GL_FLOAT, 0, 16, verts.as_ptr().add(2) as *const c_void);
        (a.glEnableVertexAttribArray)(pass.a_pos);
        (a.glEnableVertexAttribArray)(pass.a_uv);
        (a.glDrawArrays)(egl::GL_TRIANGLE_STRIP, 0, 4);
    }
}

/// The cell edge the passes are built for: 15 taps two pixels apart.
const _: () = assert!(CELL == 30);
