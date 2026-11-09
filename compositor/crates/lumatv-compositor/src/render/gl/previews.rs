//! The app switcher's scrim and pictures in the GL renderer (`scene/backdrop.rs`).
//!
//! A picture is a window's own buffer drawn again, scaled: no copy to the CPU, no read back, nothing of the video plane (a window
//! with a hole never gets a picture, `switcher.rs`). The window is 1920 by 1080 and the card 400 by 225, a shrink by almost five:
//! one bilinear draw straight from the buffer would shimmer on every fine line. So each picture has a chain of two smaller copies,
//! half and a quarter of its size, each made by drawing the one above it at exactly half size (bilinear at that ratio is an exact
//! 2x2 average); a picture is drawn from the smallest copy that is still at least as wide as the card, never shrunk by more than
//! two. The chain is made when the switcher opens, while the shell places the cards (`prepare_previews`), and again when the
//! window's buffer changes; it is dropped with the switcher. The card's corners are rounded in the fragment shader.

use std::collections::HashMap;
use std::ffi::c_void;

use wayland_server::Resource;
use wayland_server::backend::ObjectId;
use wayland_server::protocol::wl_buffer::WlBuffer;

use super::{Gl, VS, egl, link, set_params};
use crate::Rect;
use crate::scene::{Backdrop, PlannedPreview};
use egl::GLuint;

/// The picture's fragment shader: the buffer (or a flat colour, when `u_color.a` is not negative) times how much of the pixel is inside a
/// rounded rectangle of `u_a.zw` pixels with corners of `u_a.y`, times the opacity `u_a.x`. Premultiplied in, premultiplied out.
const FS: &str = "#ifdef GL_FRAGMENT_PRECISION_HIGH\nprecision highp float;\n#else\nprecision mediump float;\n#endif\n\
uniform sampler2D u_tex; uniform vec4 u_color; uniform vec4 u_a; varying vec2 v_uv;\n\
void main() {\n\
  vec2 size = u_a.zw;\n\
  vec2 q = abs(v_uv - 0.5) * size - (size * 0.5 - vec2(u_a.y));\n\
  float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - u_a.y;\n\
  float cover = clamp(0.5 - d, 0.0, 1.0) * u_a.x;\n\
  vec4 c = u_color.a >= 0.0 ? u_color : texture2D(u_tex, v_uv);\n\
  gl_FragColor = c * cover;\n\
}\n";

struct Program {
    id: GLuint,
    a_pos: GLuint,
    a_uv: GLuint,
    u_tex: i32,
    u_color: i32,
    u_a: i32,
}

struct Level {
    fbo: GLuint,
    tex: GLuint,
    size: (i32, i32),
}

/// The copies of one buffer: half and a quarter of its size, rows top first (as a `y_invert` texture's are).
struct Chain {
    levels: Vec<Level>,
}

/// What the renderer keeps for the switcher while it is open.
#[derive(Default)]
pub(super) struct Previews {
    program: Option<Program>,
    failed: bool,
    chains: HashMap<ObjectId, Chain>,
}

impl Gl {
    fn preview_program(&mut self) -> Option<&Program> {
        if self.previews.program.is_none() && !self.previews.failed {
            match self.make_preview_program() {
                Ok(p) => self.previews.program = Some(p),
                Err(e) => {
                    self.previews.failed = true;
                    log::error!("the app switcher's shader: {e}; its pictures are not drawn");
                }
            }
        }
        self.previews.program.as_ref()
    }

    fn make_preview_program(&mut self) -> std::io::Result<Program> {
        let id = link(&self.egl, VS, FS)?;
        let a = &self.egl.api;
        // SAFETY: lookups on a linked program of this context.
        let (a_pos, a_uv, u_tex, u_color, u_a) = unsafe {
            (
                (a.glGetAttribLocation)(id, c"a_pos".as_ptr()),
                (a.glGetAttribLocation)(id, c"a_uv".as_ptr()),
                (a.glGetUniformLocation)(id, c"u_tex".as_ptr()),
                (a.glGetUniformLocation)(id, c"u_color".as_ptr()),
                (a.glGetUniformLocation)(id, c"u_a".as_ptr()),
            )
        };
        if a_pos < 0 || a_uv < 0 || u_tex < 0 || u_color < 0 || u_a < 0 {
            return Err(egl::other("the switcher's program lacks an attribute or a uniform"));
        }
        Ok(Program { id, a_pos: a_pos as GLuint, a_uv: a_uv as GLuint, u_tex, u_color, u_a })
    }

    /// Makes the copies of these buffers (the switcher is opening). A buffer the driver is still busy with is left for the frame to skip.
    pub fn prepare_previews(&mut self, buffers: &[WlBuffer]) {
        if self.preview_program().is_none() {
            return;
        }
        for b in buffers {
            self.ensure_chain(b, false);
        }
        self.restore_target();
    }

    /// Links the switcher's program and draws `tex` through it into `corner` (`Gl::warm_up`): paid at the first opening, it held the
    /// loop 22.8 ms in one pass on the set (8 ms running, 10 ms asleep in the driver), two frames late; later openings took 2.5 ms.
    pub(super) fn warm_previews(&mut self, tex: GLuint, corner: &Rect) {
        let Some(p) = self.preview_program() else { return };
        let (pid, a_pos, a_uv, u_tex, u_color, u_a) = (p.id, p.a_pos, p.a_uv, p.u_tex, p.u_color, p.u_a);
        let a = &self.egl.api;
        // SAFETY: state calls on the current context with this renderer's program.
        unsafe {
            (a.glUseProgram)(pid);
            (a.glUniform1i)(u_tex, 0);
            (a.glUniform4f)(u_color, 0.0, 0.0, 0.0, -1.0);
            (a.glUniform4f)(u_a, 1.0, 8.0, corner.w as f32, corner.h as f32);
        }
        self.quad(a_pos, a_uv, tex, corner, true);
        // SAFETY: as above.
        unsafe { (self.egl.api.glUseProgram)(self.program) };
    }

    /// Puts the current target back as the frame draws into it, after something else was drawn.
    pub(super) fn restore_target(&self) {
        let a = &self.egl.api;
        // SAFETY: state calls on the current context and this renderer's own framebuffer.
        unsafe {
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, self.targets[self.current.0].fbo);
            (a.glViewport)(0, 0, self.width as i32, self.height as i32);
            (a.glUseProgram)(self.program);
            (a.glEnable)(egl::GL_SCISSOR_TEST);
            (a.glEnable)(egl::GL_BLEND);
            (a.glBlendFunc)(egl::GL_ONE, egl::GL_ONE_MINUS_SRC_ALPHA);
        }
    }

    /// Makes (or, when the buffer changed, makes again) the chain of `buffer`. `false` when it cannot be drawn now.
    fn ensure_chain(&mut self, buffer: &WlBuffer, changed: bool) -> bool {
        let id = buffer.id();
        if self.previews.chains.contains_key(&id) && !changed {
            return true;
        }
        // The driver is still busy with it (its warm draw has not finished): a draw from it now would wait on that.
        if self.textures.get(&id).is_some_and(|t| t.external && (t.warm_sync.is_some() || t.held)) {
            return false;
        }
        let Some((tex, size, y_invert, _)) = self.texture_for(buffer, None, &Rect::default()) else { return false };
        if size.0 <= 0 || size.1 <= 0 {
            return false;
        }
        if !self.previews.chains.contains_key(&id) {
            let mut levels = Vec::with_capacity(2);
            let mut at = size;
            for _ in 0..2 {
                at = ((at.0 + 1) / 2, (at.1 + 1) / 2);
                match self.make_level(at) {
                    Ok(l) => levels.push(l),
                    Err(e) => {
                        log::warn!("the app switcher's copy of a picture: {e}");
                        for l in levels {
                            self.free_level(&l);
                        }
                        return false;
                    }
                }
            }
            self.previews.chains.insert(id.clone(), Chain { levels });
        }
        let Some(chain) = self.previews.chains.get(&id) else { return false };
        let (mut from, mut flipped) = (tex, y_invert);
        let steps: Vec<(GLuint, GLuint, (i32, i32))> = chain.levels.iter().map(|l| (l.fbo, l.tex, l.size)).collect();
        for (fbo, to_tex, to_size) in steps {
            self.shrink(from, flipped, fbo, to_size);
            (from, flipped) = (to_tex, true);
        }
        true
    }

    fn make_level(&self, size: (i32, i32)) -> std::io::Result<Level> {
        let a = &self.egl.api;
        // SAFETY: GL object creation with a current context; the framebuffer is checked.
        unsafe {
            let mut tex = 0;
            (a.glGenTextures)(1, &mut tex);
            (a.glBindTexture)(egl::GL_TEXTURE_2D, tex);
            (a.glTexImage2D)(egl::GL_TEXTURE_2D, 0, egl::GL_RGBA as i32, size.0, size.1, 0, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, std::ptr::null::<c_void>());
            set_params(a);
            let mut fbo = 0;
            (a.glGenFramebuffers)(1, &mut fbo);
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
            (a.glFramebufferTexture2D)(egl::GL_FRAMEBUFFER, egl::GL_COLOR_ATTACHMENT0, egl::GL_TEXTURE_2D, tex, 0);
            let status = (a.glCheckFramebufferStatus)(egl::GL_FRAMEBUFFER);
            if status != egl::GL_FRAMEBUFFER_COMPLETE {
                (a.glDeleteFramebuffers)(1, &fbo);
                (a.glDeleteTextures)(1, &tex);
                return Err(egl::other(format!("framebuffer incomplete: 0x{status:x}")));
            }
            Ok(Level { fbo, tex, size })
        }
    }

    fn free_level(&self, l: &Level) {
        let a = &self.egl.api;
        // SAFETY: objects this renderer made, freed once.
        unsafe {
            (a.glDeleteFramebuffers)(1, &l.fbo);
            (a.glDeleteTextures)(1, &l.tex);
        }
    }

    /// One step of a chain: `from` drawn into `fbo` at half its size, picture top at row 0 (so the result is a `y_invert` texture).
    fn shrink(&self, from: GLuint, from_top_down: bool, fbo: GLuint, size: (i32, i32)) {
        let a = &self.egl.api;
        let (vt, vb) = if from_top_down { (0.0f32, 1.0f32) } else { (1.0, 0.0) };
        let verts: [f32; 16] = [-1.0, -1.0, 0.0, vt, 1.0, -1.0, 1.0, vt, -1.0, 1.0, 0.0, vb, 1.0, 1.0, 1.0, vb];
        // SAFETY: state calls on the current context; client-side vertex arrays of 16 floats; the program and textures are this renderer's.
        unsafe {
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
            (a.glViewport)(0, 0, size.0, size.1);
            (a.glDisable)(egl::GL_SCISSOR_TEST);
            (a.glDisable)(egl::GL_BLEND);
            (a.glUseProgram)(self.program);
            (a.glActiveTexture)(egl::GL_TEXTURE0);
            (a.glBindTexture)(egl::GL_TEXTURE_2D, from);
            (a.glVertexAttribPointer)(self.a_pos, 2, egl::GL_FLOAT, 0, 16, verts.as_ptr() as *const c_void);
            (a.glVertexAttribPointer)(self.a_uv, 2, egl::GL_FLOAT, 0, 16, verts.as_ptr().add(2) as *const c_void);
            (a.glEnableVertexAttribArray)(self.a_pos);
            (a.glEnableVertexAttribArray)(self.a_uv);
            (a.glDrawArrays)(egl::GL_TRIANGLE_STRIP, 0, 4);
        }
    }

    /// Frees the copies once the switcher is closed (the plan carries no backdrop).
    pub(super) fn free_previews(&mut self) {
        if self.previews.chains.is_empty() {
            return;
        }
        for (_, chain) in std::mem::take(&mut self.previews.chains) {
            for l in &chain.levels {
                self.free_level(l);
            }
        }
    }

    /// Which texture a picture is drawn from: the smallest of the buffer and its two copies that is still wider than the card.
    fn picture_source(&self, p: &PlannedPreview) -> Option<(GLuint, bool)> {
        let chain = self.previews.chains.get(&p.buffer.id())?;
        let t = self.textures.get(&p.buffer.id())?;
        let w = p.rect.w;
        Some(match chain.levels.as_slice() {
            [_, q] if w <= q.size.0 => (q.tex, true),
            [h, _] if w <= h.size.0 => (h.tex, true),
            _ => (t.tex, t.y_invert),
        })
    }

    /// The scrim over the whole output and the pictures on it, inside `rects`.
    pub(super) fn draw_backdrop(&mut self, b: &Backdrop, rects: &[Rect]) {
        if self.preview_program().is_none() {
            return;
        }
        // Every copy is made before the frame's own target is bound again below: making one binds others.
        let mut ready = Vec::with_capacity(b.previews.len());
        for p in &b.previews {
            ready.push(self.ensure_chain(&p.buffer, p.dirty));
        }
        self.restore_target();
        let Some(prog) = self.previews.program.as_ref() else { return };
        let (pid, a_pos, a_uv, u_tex, u_color, u_a) = (prog.id, prog.a_pos, prog.a_uv, prog.u_tex, prog.u_color, prog.u_a);
        let full = Rect::new(0, 0, self.width as i32, self.height as i32);
        let dummy = self.cursor_texture();
        let a = &self.egl.api;
        // SAFETY: state calls on the current context with this renderer's program.
        unsafe {
            (a.glUseProgram)(pid);
            (a.glUniform1i)(u_tex, 0);
        }
        if b.scrim[3] > 0.0 {
            // SAFETY: as above.
            unsafe { (a.glUniform4f)(u_color, b.scrim[0], b.scrim[1], b.scrim[2], b.scrim[3]) };
            // SAFETY: as above.
            unsafe { (a.glUniform4f)(u_a, 1.0, 0.0, full.w as f32, full.h as f32) };
            for r in rects {
                let clip = full.intersect(r);
                if !clip.is_empty() {
                    self.scissor(&clip);
                    self.quad(a_pos, a_uv, dummy, &full, true);
                }
            }
        }
        for (p, ok) in b.previews.iter().zip(ready) {
            let Some((tex, y_invert)) = ok.then(|| self.picture_source(p)).flatten() else { continue };
            let a = &self.egl.api;
            // SAFETY: as above.
            unsafe {
                (a.glUniform4f)(u_color, 0.0, 0.0, 0.0, -1.0);
                (a.glUniform4f)(u_a, p.alpha, p.radius, p.rect.w as f32, p.rect.h as f32);
            }
            for r in rects {
                let clip = p.rect.intersect(r);
                if !clip.is_empty() {
                    self.scissor(&clip);
                    self.quad(a_pos, a_uv, tex, &p.rect, y_invert);
                }
            }
        }
        // SAFETY: as above.
        unsafe { (self.egl.api.glUseProgram)(self.program) };
    }

    /// `draw_quad` for the switcher's program (its attributes may sit at other locations than the frame's).
    fn quad(&self, a_pos: GLuint, a_uv: GLuint, tex: GLuint, rect: &Rect, y_invert: bool) {
        let (w, h) = (self.width as f32, self.height as f32);
        let x0 = rect.x as f32 / w * 2.0 - 1.0;
        let x1 = rect.right() as f32 / w * 2.0 - 1.0;
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
}
