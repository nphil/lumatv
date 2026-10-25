//! The GL side of `Content::Mesh`: geometry in vertex and index buffers, and the off-screen colour + depth target a
//! mesh layer is drawn into (multisampled when the layer asks for anti-aliasing). It is a child of `gl` so that it
//! can use the driver table and the private constants; the mesh renderer only sees the safe calls at the bottom.
//!
//! The pass is built for a tile-based GPU: the target is cleared before drawing (so nothing is loaded from
//! memory), and the depth buffer and the multisampled colour are invalidated after the resolve (so they are never
//! written back to memory).

use super::{
    GL_ARRAY_BUFFER, GL_BLEND, GL_COLOR_ATTACHMENT0, GL_COLOR_BUFFER_BIT, GL_FLOAT, GL_FRAMEBUFFER,
    GL_FRAMEBUFFER_COMPLETE, GL_NO_ERROR, GL_RGBA8, GL_SCISSOR_TEST, GL_UNSIGNED_BYTE, Gles,
};
use std::ffi::c_void;
use std::io;

const GL_EXTENSIONS: u32 = 0x1F03;
const GL_DEPTH_TEST: u32 = 0x0B71;
const GL_DEPTH_BUFFER_BIT: u32 = 0x0100;
const GL_TRIANGLES: u32 = 0x0004;
const GL_UNSIGNED_SHORT: u32 = 0x1403;
const GL_BYTE: u32 = 0x1400;
const GL_STATIC_DRAW: u32 = 0x88E4;
const GL_ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
const GL_RENDERBUFFER: u32 = 0x8D41;
const GL_DEPTH_COMPONENT24: u32 = 0x81A6;
const GL_DEPTH_ATTACHMENT: u32 = 0x8D00;
const GL_MAX_SAMPLES: u32 = 0x8D57;
const GL_READ_FRAMEBUFFER: u32 = 0x8CA8;
const GL_DRAW_FRAMEBUFFER: u32 = 0x8CA9;
const GL_NEAREST: u32 = 0x2600;
const GL_LINEAR: u32 = 0x2601;

api!(CoreApi {
    gen_vertex_arrays = "glGenVertexArrays" fn(i32, *mut u32);
    bind_vertex_array = "glBindVertexArray" fn(u32);
    delete_vertex_arrays = "glDeleteVertexArrays" fn(i32, *const u32);
    delete_buffers = "glDeleteBuffers" fn(i32, *const u32);
    gen_renderbuffers = "glGenRenderbuffers" fn(i32, *mut u32);
    bind_renderbuffer = "glBindRenderbuffer" fn(u32, u32);
    delete_renderbuffers = "glDeleteRenderbuffers" fn(i32, *const u32);
    renderbuffer_storage_multisample = "glRenderbufferStorageMultisample" fn(u32, i32, u32, i32, i32);
    framebuffer_renderbuffer = "glFramebufferRenderbuffer" fn(u32, u32, u32, u32);
    draw_elements = "glDrawElements" fn(u32, i32, u32, *const c_void);
    uniform_matrix4fv = "glUniformMatrix4fv" fn(i32, i32, u8, *const f32);
    bind_framebuffer_target = "glBindFramebuffer" fn(u32, u32);
    blit = "glBlitFramebuffer" fn(i32, i32, i32, i32, i32, i32, i32, i32, u32, u32);
    invalidate = "glInvalidateFramebuffer" fn(u32, i32, *const u32);
    delete_framebuffers = "glDeleteFramebuffers" fn(i32, *const u32);
});

api!(InTileApi {
    framebuffer_texture_2d_multisample = "glFramebufferTexture2DMultisampleEXT" fn(u32, u32, u32, u32, i32, i32);
    renderbuffer_storage_multisample = "glRenderbufferStorageMultisampleEXT" fn(u32, i32, u32, i32, i32);
});

/// The driver entry points the mesh layer uses. The in-tile resolve is optional: without
/// `GL_EXT_multisampled_render_to_texture` multisampled targets are resolved with an explicit blit.
pub(super) struct MeshGl {
    core: CoreApi,
    in_tile: Option<InTileApi>,
}

impl MeshGl {
    pub(super) fn load(
        resolve: &dyn Fn(&str) -> Option<*const c_void>,
        gl: &super::GlApi,
    ) -> io::Result<Self> {
        // SAFETY: glGetString returns a static NUL-terminated string, or null.
        let extensions = unsafe {
            let p = (gl.get_string)(GL_EXTENSIONS);
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p as *const std::ffi::c_char)
                    .to_string_lossy()
                    .into_owned()
            }
        };
        let listed = extensions
            .split_ascii_whitespace()
            .any(|e| e == "GL_EXT_multisampled_render_to_texture");
        Ok(Self {
            core: CoreApi::load(resolve)?,
            in_tile: if listed {
                InTileApi::load(resolve).ok()
            } else {
                None
            },
        })
    }
}

/// A mesh's vertices and indices, with the attribute layout set up in a vertex array object so that drawing it
/// leaves the state the other layers rely on alone.
pub struct Geometry {
    vertex_array: u32,
    vertex_buffer: u32,
    index_buffer: u32,
    pub index_count: i32,
}

/// A colour + depth render target: the texture the finished picture is in, the framebuffer it is drawn into, and
/// when multisampled the framebuffer the samples are resolved from.
pub struct Offscreen {
    pub texture: u32,
    pub width: u32,
    pub height: u32,
    /// The texture's framebuffer; the one drawn into when not multisampled.
    framebuffer: u32,
    /// The multisampled framebuffer and its colour storage, when antialiased.
    samples: Option<(u32, u32)>,
    sample_count: u32,
    depth: u32,
}

impl Offscreen {
    /// Samples per pixel; 0 when not multisampled.
    pub fn samples(&self) -> u32 {
        self.sample_count
    }
}

impl Gles {
    /// The most samples per pixel the driver's renderbuffers support.
    pub fn max_samples(&self) -> u32 {
        let mut samples = 0;
        // SAFETY: writes one integer.
        unsafe { (self.gl.get_integerv)(GL_MAX_SAMPLES, &mut samples) };
        u32::try_from(samples).unwrap_or(0)
    }

    /// Whether multisampled targets are resolved in the tile (`GL_EXT_multisampled_render_to_texture`) rather
    /// than by a blit.
    pub fn resolves_in_tile(&self) -> bool {
        self.mesh_api.in_tile.is_some()
    }

    /// Uploads a mesh: `vertices` in the 20-byte layout of the mesh file (position `f32`×3, normal `i8`×3 and a
    /// pad byte, premultiplied colour `u8`×4) and `indices` as triangles.
    pub fn geometry(&self, vertices: &[u8], indices: &[u16]) -> Geometry {
        let stride = compositor_protocol::MESH_VERTEX_BYTES as i32;
        let (mut vertex_array, mut buffers) = (0, [0u32; 2]);
        // SAFETY: creates a vertex array and two buffers, fills them from slices whose lengths are passed, and
        // points three attributes at offsets inside the vertex layout.
        unsafe {
            (self.mesh_api.core.gen_vertex_arrays)(1, &mut vertex_array);
            (self.mesh_api.core.bind_vertex_array)(vertex_array);
            (self.gl.gen_buffers)(2, buffers.as_mut_ptr());
            (self.gl.bind_buffer)(GL_ARRAY_BUFFER, buffers[0]);
            (self.gl.buffer_data)(
                GL_ARRAY_BUFFER,
                vertices.len() as isize,
                vertices.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );
            (self.gl.bind_buffer)(GL_ELEMENT_ARRAY_BUFFER, buffers[1]);
            (self.gl.buffer_data)(
                GL_ELEMENT_ARRAY_BUFFER,
                std::mem::size_of_val(indices) as isize,
                indices.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );
            for attribute in 0..3 {
                (self.gl.enable_vertex_attrib_array)(attribute);
            }
            (self.gl.vertex_attrib_pointer)(0, 3, GL_FLOAT, 0, stride, std::ptr::null());
            (self.gl.vertex_attrib_pointer)(
                1,
                3,
                GL_BYTE,
                1,
                stride,
                std::ptr::without_provenance(12),
            );
            (self.gl.vertex_attrib_pointer)(
                2,
                4,
                GL_UNSIGNED_BYTE,
                1,
                stride,
                std::ptr::without_provenance(16),
            );
            (self.mesh_api.core.bind_vertex_array)(0);
        }
        Geometry {
            vertex_array,
            vertex_buffer: buffers[0],
            index_buffer: buffers[1],
            index_count: indices.len() as i32,
        }
    }

    pub fn delete_geometry(&self, geometry: Geometry) {
        let buffers = [geometry.vertex_buffer, geometry.index_buffer];
        // SAFETY: deletes names this geometry owns.
        unsafe {
            (self.mesh_api.core.delete_buffers)(2, buffers.as_ptr());
            (self.mesh_api.core.delete_vertex_arrays)(1, &geometry.vertex_array);
        }
    }

    /// A `width` × `height` target with a 24-bit depth buffer, multisampled `samples` times (0 for none). The
    /// picture ends up in `texture` whichever it is.
    pub fn offscreen(&self, width: u32, height: u32, samples: u32) -> io::Result<Offscreen> {
        let texture = self.new_clear_texture(width, height);
        let (mut framebuffer, mut depth) = (0u32, 0u32);
        let mut multisampled = None;
        let (w, h, n) = (width as i32, height as i32, samples as i32);
        // SAFETY: GL object creation on the current context; completeness is checked below.
        let (status, error) = unsafe {
            let m = &self.mesh_api.core;
            // With the extension the texture itself is the multisampled colour buffer and the GPU resolves it as
            // each tile is written out, which costs far less than resolving a separate renderbuffer afterwards.
            let in_tile = self.mesh_api.in_tile.filter(|_| samples > 0);
            (self.gl.gen_framebuffers)(1, &mut framebuffer);
            (self.gl.bind_framebuffer)(GL_FRAMEBUFFER, framebuffer);
            match in_tile {
                Some(ext) => (ext.framebuffer_texture_2d_multisample)(
                    GL_FRAMEBUFFER,
                    GL_COLOR_ATTACHMENT0,
                    super::GL_TEXTURE_2D,
                    texture,
                    0,
                    n,
                ),
                None => (self.gl.framebuffer_texture_2d)(
                    GL_FRAMEBUFFER,
                    GL_COLOR_ATTACHMENT0,
                    super::GL_TEXTURE_2D,
                    texture,
                    0,
                ),
            }
            (m.gen_renderbuffers)(1, &mut depth);
            (m.bind_renderbuffer)(GL_RENDERBUFFER, depth);
            match in_tile {
                Some(ext) => (ext.renderbuffer_storage_multisample)(
                    GL_RENDERBUFFER,
                    n,
                    GL_DEPTH_COMPONENT24,
                    w,
                    h,
                ),
                None => (m.renderbuffer_storage_multisample)(
                    GL_RENDERBUFFER,
                    n,
                    GL_DEPTH_COMPONENT24,
                    w,
                    h,
                ),
            }
            let mut status = GL_FRAMEBUFFER_COMPLETE;
            if samples > 0 && in_tile.is_none() {
                let (mut samples_fb, mut colour) = (0u32, 0u32);
                (self.gl.gen_framebuffers)(1, &mut samples_fb);
                (m.gen_renderbuffers)(1, &mut colour);
                (m.bind_renderbuffer)(GL_RENDERBUFFER, colour);
                (m.renderbuffer_storage_multisample)(GL_RENDERBUFFER, n, GL_RGBA8 as u32, w, h);
                (self.gl.bind_framebuffer)(GL_FRAMEBUFFER, samples_fb);
                (m.framebuffer_renderbuffer)(
                    GL_FRAMEBUFFER,
                    GL_COLOR_ATTACHMENT0,
                    GL_RENDERBUFFER,
                    colour,
                );
                (m.framebuffer_renderbuffer)(
                    GL_FRAMEBUFFER,
                    GL_DEPTH_ATTACHMENT,
                    GL_RENDERBUFFER,
                    depth,
                );
                status = (self.gl.check_framebuffer_status)(GL_FRAMEBUFFER);
                multisampled = Some((samples_fb, colour));
            } else {
                (m.framebuffer_renderbuffer)(
                    GL_FRAMEBUFFER,
                    GL_DEPTH_ATTACHMENT,
                    GL_RENDERBUFFER,
                    depth,
                );
            }
            if status == GL_FRAMEBUFFER_COMPLETE {
                (self.gl.bind_framebuffer)(GL_FRAMEBUFFER, framebuffer);
                status = (self.gl.check_framebuffer_status)(GL_FRAMEBUFFER);
            }
            (status, (self.gl.get_error)())
        };
        let target = Offscreen {
            texture,
            width,
            height,
            framebuffer,
            samples: multisampled,
            sample_count: samples,
            depth,
        };
        if status != GL_FRAMEBUFFER_COMPLETE || error != GL_NO_ERROR {
            self.delete_offscreen(target);
            return Err(io::Error::other(format!(
                "a {width}x{height} target with {samples} samples is not usable: framebuffer status {status:#x}, GL error {error:#x}"
            )));
        }
        Ok(target)
    }

    pub fn delete_offscreen(&self, target: Offscreen) {
        // SAFETY: deletes names this target owns.
        unsafe {
            self.delete_texture(target.texture);
            (self.mesh_api.core.delete_framebuffers)(1, &target.framebuffer);
            (self.mesh_api.core.delete_renderbuffers)(1, &target.depth);
            if let Some((framebuffer, colour)) = target.samples {
                (self.mesh_api.core.delete_framebuffers)(1, &framebuffer);
                (self.mesh_api.core.delete_renderbuffers)(1, &colour);
            }
        }
    }

    /// Starts a pass into `target`: bound, cleared to `background` and to far depth, depth-tested, not blended.
    pub fn begin_offscreen(&self, target: &Offscreen, background: [f32; 4]) {
        let framebuffer = target.samples.map_or(target.framebuffer, |(f, _)| f);
        self.bind_framebuffer(framebuffer);
        self.viewport(0, 0, target.width as i32, target.height as i32);
        self.disable(GL_SCISSOR_TEST);
        self.disable(GL_BLEND);
        self.enable(GL_DEPTH_TEST);
        // SAFETY: plain state calls.
        unsafe {
            (self.gl.clear_color)(background[0], background[1], background[2], background[3]);
            (self.gl.clear)(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        }
    }

    pub fn draw_geometry(&self, geometry: &Geometry) {
        // SAFETY: draws the indices of the element buffer the vertex array holds, which `geometry` filled.
        unsafe {
            (self.mesh_api.core.bind_vertex_array)(geometry.vertex_array);
            (self.mesh_api.core.draw_elements)(
                GL_TRIANGLES,
                geometry.index_count,
                GL_UNSIGNED_SHORT,
                std::ptr::null(),
            );
        }
    }

    /// Ends the pass: resolves the samples into the texture and lets go of the depth buffer and the samples, then
    /// puts back the state the frame pass expects (no depth test, blending on, the default vertex array).
    pub fn end_offscreen(&self, target: &Offscreen) {
        let rect = [0, 0, target.width as i32, target.height as i32];
        if let Some((samples, _)) = target.samples {
            self.bind_read_framebuffer(samples);
            self.bind_draw_framebuffer(target.framebuffer);
            self.blit_framebuffer(rect, rect, false);
            self.bind_framebuffer(samples);
            self.invalidate_framebuffer(&[GL_COLOR_ATTACHMENT0, GL_DEPTH_ATTACHMENT]);
        } else {
            self.invalidate_framebuffer(&[GL_DEPTH_ATTACHMENT]);
        }
        self.disable(GL_DEPTH_TEST);
        self.enable(GL_BLEND);
        // SAFETY: binds the default vertex array.
        unsafe { (self.mesh_api.core.bind_vertex_array)(0) };
    }

    pub fn uniform_matrix4(&self, location: i32, matrix: &[f32; 16]) {
        // SAFETY: reads sixteen floats.
        unsafe { (self.mesh_api.core.uniform_matrix4fv)(location, 1, 0, matrix.as_ptr()) }
    }

    // Multisample resolve and invalidation: plain calls on the read and draw framebuffers.

    pub fn bind_read_framebuffer(&self, framebuffer: u32) {
        // SAFETY: plain state call.
        unsafe { (self.mesh_api.core.bind_framebuffer_target)(GL_READ_FRAMEBUFFER, framebuffer) }
    }

    pub fn bind_draw_framebuffer(&self, framebuffer: u32) {
        // SAFETY: plain state call.
        unsafe { (self.mesh_api.core.bind_framebuffer_target)(GL_DRAW_FRAMEBUFFER, framebuffer) }
    }

    pub fn blit_framebuffer(&self, src: [i32; 4], dst: [i32; 4], linear: bool) {
        let filter = if linear { GL_LINEAR } else { GL_NEAREST };
        // SAFETY: plain state call.
        unsafe {
            (self.mesh_api.core.blit)(
                src[0],
                src[1],
                src[2],
                src[3],
                dst[0],
                dst[1],
                dst[2],
                dst[3],
                GL_COLOR_BUFFER_BIT,
                filter,
            )
        }
    }

    pub fn invalidate_framebuffer(&self, attachments: &[u32]) {
        // SAFETY: passes a slice's pointer and length.
        unsafe {
            (self.mesh_api.core.invalidate)(
                GL_DRAW_FRAMEBUFFER,
                attachments.len() as i32,
                attachments.as_ptr(),
            )
        }
    }
}
