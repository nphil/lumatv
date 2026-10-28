//! The box dim in the GL renderer (`boxdim`): a copy of the item program with one more texel fetch, the map's texture, and the sample
//! pass that reads the app's pictures at the grid's resolution.
//!
//! The gained program is used for app items only while the plan carries a map; every other item, and every frame without a map, goes
//! through the plain program as before. The map is a 64 x 36 RGBA texture (the gain in every channel), re-uploaded when the plan's serial
//! changes, sampled bilinearly at the fragment's screen position (`v_gain`, from the vertex position; `u_gy` turns it for a target whose
//! row 0 is the top). The sample pass draws the app items into a 64 x 36 target with a box filter of 15 x 15 bilinear taps two output
//! pixels apart, which averages all 900 texels of a cell exactly, holes cleared black before their item, then reads the 9 KB back.

use std::ffi::c_void;
use std::io;

use wayland_server::Resource;

use super::{Gl, VS, egl, link, set_params};
use crate::Rect;
use crate::boxdim::{CELLS, COLS, ROWS};
use crate::scene::Plan;
use crate::surface::Surfaces;
use egl::GLuint;

const GAIN_VS: &str = "attribute vec2 a_pos; attribute vec2 a_uv; uniform vec2 u_gy; varying vec2 v_uv; varying vec2 v_gain;\n\
void main() { gl_Position = vec4(a_pos, 0.0, 1.0); v_uv = a_uv; v_gain = vec2(a_pos.x * 0.5 + 0.5, (a_pos.y * 0.5 + 0.5) * u_gy.x + u_gy.y); }\n";
/// The item program's fragment shader (`gl.rs` `FS`) times the map: premultiplied, the alpha left.
const GAIN_FS: &str = "precision mediump float; uniform sampler2D u_tex; uniform sampler2D u_gain; uniform float u_dim; varying vec2 v_uv; varying vec2 v_gain;\n\
void main() { vec4 c = texture2D(u_tex, v_uv); gl_FragColor = vec4(c.rgb * (u_dim * texture2D(u_gain, v_gain).r), c.a); }\n";
/// The box filter: the mean of 15 x 15 bilinear taps `u_step` apart (two output pixels in the item's texture coordinates), centred on the
/// fragment. The taps' positions need full precision (a step is a thousandth of the texture).
const SAMPLE_FS: &str = "#ifdef GL_FRAGMENT_PRECISION_HIGH\nprecision highp float;\n#else\nprecision mediump float;\n#endif\n\
uniform sampler2D u_tex; uniform vec2 u_step; varying vec2 v_uv;\n\
void main() {\n\
  vec4 acc = vec4(0.0);\n\
  for (int j = 0; j < 15; j++) {\n\
    for (int i = 0; i < 15; i++) {\n\
      acc += texture2D(u_tex, v_uv + (vec2(float(i), float(j)) - 7.0) * u_step);\n\
    }\n\
  }\n\
  gl_FragColor = acc * (1.0 / 225.0);\n\
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

struct Sampler {
    id: GLuint,
    a_pos: GLuint,
    a_uv: GLuint,
    u_step: i32,
    fbo: GLuint,
    pixels: Vec<u8>,
}

/// What the renderer keeps for the box dim, made on first use.
#[derive(Default)]
pub(super) struct BoxDim {
    gain: Option<Gain>,
    sampler: Option<Sampler>,
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

    fn sampler(&mut self) -> Option<&Sampler> {
        if self.boxdim.sampler.is_none() && !self.boxdim.failed {
            match self.make_sampler() {
                Ok(s) => self.boxdim.sampler = Some(s),
                Err(e) => {
                    self.boxdim.failed = true;
                    log::error!("the box dim's sample pass: {e}; nothing is dimmed");
                }
            }
        }
        self.boxdim.sampler.as_ref()
    }

    fn make_sampler(&self) -> io::Result<Sampler> {
        let id = link(&self.egl, VS, SAMPLE_FS)?;
        let a = &self.egl.api;
        // SAFETY: GL object creation with a current context; the framebuffer is checked; the program is linked.
        unsafe {
            (a.glUseProgram)(id);
            (a.glUniform1i)((a.glGetUniformLocation)(id, c"u_tex".as_ptr()), 0);
            let u_step = (a.glGetUniformLocation)(id, c"u_step".as_ptr());
            (a.glUseProgram)(self.program);
            let mut tex = 0;
            (a.glGenTextures)(1, &mut tex);
            (a.glBindTexture)(egl::GL_TEXTURE_2D, tex);
            (a.glTexImage2D)(egl::GL_TEXTURE_2D, 0, egl::GL_RGBA as i32, COLS as i32, ROWS as i32, 0, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, std::ptr::null());
            (a.glTexParameteri)(egl::GL_TEXTURE_2D, egl::GL_TEXTURE_MIN_FILTER, egl::GL_NEAREST);
            (a.glTexParameteri)(egl::GL_TEXTURE_2D, egl::GL_TEXTURE_MAG_FILTER, egl::GL_NEAREST);
            let mut fbo = 0;
            (a.glGenFramebuffers)(1, &mut fbo);
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
            (a.glFramebufferTexture2D)(egl::GL_FRAMEBUFFER, egl::GL_COLOR_ATTACHMENT0, egl::GL_TEXTURE_2D, tex, 0);
            let status = (a.glCheckFramebufferStatus)(egl::GL_FRAMEBUFFER);
            if status != egl::GL_FRAMEBUFFER_COMPLETE {
                (a.glDeleteFramebuffers)(1, &fbo);
                (a.glDeleteTextures)(1, &tex);
                return Err(egl::other(format!("sample framebuffer incomplete: 0x{status:x}")));
            }
            Ok(Sampler {
                id,
                a_pos: (a.glGetAttribLocation)(id, c"a_pos".as_ptr()) as GLuint,
                a_uv: (a.glGetAttribLocation)(id, c"a_uv".as_ptr()) as GLuint,
                u_step,
                fbo,
                pixels: vec![0; CELLS * 4],
            })
        }
    }

    /// The box dim's sample: the plan's app items composed into the 64 x 36 target through the box filter, holes black, read back as one
    /// luma per cell (row 0 the top). `false` when an item's buffer is still warming (the driver is busy with it) or the pass cannot be made.
    pub fn sample_cells(&mut self, plan: &Plan, _surfaces: &Surfaces, luma: &mut [u8; CELLS]) -> io::Result<bool> {
        let items: Vec<usize> = plan.items.iter().enumerate().filter(|(_, i)| i.gain && i.pose.is_none() && !i.ghost).map(|(k, _)| k).collect();
        if items.iter().any(|k| self.textures.get(&plan.items[*k].buffer.id()).is_some_and(|t| t.external && (t.warm_sync.is_some() || t.held))) {
            return Ok(false);
        }
        let Some(s) = self.sampler() else { return Ok(false) };
        let (fbo, program, a_pos, a_uv, u_step) = (s.fbo, s.id, s.a_pos, s.a_uv, s.u_step);
        let (w, h) = (self.width as f32, self.height as f32);
        let a = &self.egl.api;
        // SAFETY: state calls on the current context and this renderer's objects.
        unsafe {
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
            (a.glViewport)(0, 0, COLS as i32, ROWS as i32);
            (a.glUseProgram)(program);
            (a.glDisable)(egl::GL_SCISSOR_TEST);
            (a.glClearColor)(0.0, 0.0, 0.0, 1.0);
            (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
            (a.glEnable)(egl::GL_BLEND);
            (a.glBlendFunc)(egl::GL_ONE, egl::GL_ONE_MINUS_SRC_ALPHA);
        }
        for k in items {
            let item = &plan.items[k];
            for hole in plan.holes.iter().filter(|h| h.before == k) {
                // Every cell the hole touches, cleared to transparent: what is under the video is not bright.
                let Some((c0, r0, c1, r1)) = crate::boxdim::cells_of(&hole.rect) else { continue };
                let a = &self.egl.api;
                // SAFETY: as above; the scissor is in the target's rows, which run bottom first.
                unsafe {
                    (a.glEnable)(egl::GL_SCISSOR_TEST);
                    (a.glScissor)(c0 as i32, (ROWS - 1 - r1) as i32, (c1 - c0 + 1) as i32, (r1 - r0 + 1) as i32);
                    (a.glClearColor)(0.0, 0.0, 0.0, 0.0);
                    (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
                    (a.glDisable)(egl::GL_SCISSOR_TEST);
                }
            }
            let Some((tex, _, y_invert, _)) = self.texture_for(&item.buffer, Some(&[]), &item.rect) else { continue };
            let r = item.rect;
            if r.w <= 0 || r.h <= 0 {
                continue;
            }
            let (x0, x1) = (r.x as f32 / w * 2.0 - 1.0, r.right() as f32 / w * 2.0 - 1.0);
            let (y0, y1) = (1.0 - r.y as f32 / h * 2.0, 1.0 - r.bottom() as f32 / h * 2.0);
            let (vt, vb) = if y_invert { (0.0f32, 1.0f32) } else { (1.0, 0.0) };
            let verts: [f32; 16] = [x0, y0, 0.0, vt, x1, y0, 1.0, vt, x0, y1, 0.0, vb, x1, y1, 1.0, vb];
            let a = &self.egl.api;
            // SAFETY: client-side vertex arrays of 16 floats, the sample program and the item's texture bound.
            unsafe {
                (a.glUniform2f)(u_step, 2.0 / r.w as f32, 2.0 / r.h as f32);
                (a.glActiveTexture)(egl::GL_TEXTURE0);
                (a.glBindTexture)(egl::GL_TEXTURE_2D, tex);
                (a.glVertexAttribPointer)(a_pos, 2, egl::GL_FLOAT, 0, 16, verts.as_ptr() as *const c_void);
                (a.glVertexAttribPointer)(a_uv, 2, egl::GL_FLOAT, 0, 16, verts.as_ptr().add(2) as *const c_void);
                (a.glEnableVertexAttribArray)(a_pos);
                (a.glEnableVertexAttribArray)(a_uv);
                (a.glDrawArrays)(egl::GL_TRIANGLE_STRIP, 0, 4);
            }
        }
        let s = self.boxdim.sampler.as_mut().unwrap();
        let a = &self.egl.api;
        // SAFETY: the read fills exactly 64 x 36 x 4 bytes of a vector of that size.
        unsafe {
            (a.glPixelStorei)(egl::GL_PACK_ALIGNMENT, 4);
            (a.glReadPixels)(0, 0, COLS as i32, ROWS as i32, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, s.pixels.as_mut_ptr() as *mut c_void);
        }
        for (i, px) in s.pixels.chunks_exact(4).enumerate() {
            let (c, r) = (i % COLS, i / COLS);
            // Row 0 of the read is the target's bottom, the screen's last row of cells.
            luma[(ROWS - 1 - r) * COLS + c] = ((54 * px[0] as u32 + 183 * px[1] as u32 + 19 * px[2] as u32) >> 8) as u8;
        }
        self.restore_target();
        let err = self.egl.gl_error();
        if err != egl::GL_NO_ERROR {
            return Err(egl::other(format!("box dim sample: gl error 0x{err:x}")));
        }
        Ok(true)
    }
}
