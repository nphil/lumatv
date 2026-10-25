//! The first draw of a producer's buffer, made where no frame is waiting for it.
//!
//! A buffer a producer fills and the GPU draws from is memory the GPU has not read until something draws it. Whatever the driver does
//! the first time (mapping it, making its descriptor, the first job that names it) is its own business and has a price; paid in the frame
//! that shows the first picture it is a refresh that lands late. [`Touch`] pays it between frames: the housekeeping of the live sources
//! (`live::Sources::tidy`) has the buffer drawn once, as a quad of the image program over a target of a few pixels that nobody
//! shows, before the source is announced to its producer.

use super::ImageProgram;
use crate::geometry::Panel;
use crate::gl::{GL_BLEND, GL_SCISSOR_TEST, Gles, RenderTarget};
use crate::log::log;
use compositor_protocol::frames::Spec;

/// The target's side, in pixels: four of them are all a draw of a quad over it needs.
const SIDE: u32 = 2;

/// The target the draws go to and the vertices they are made of.
pub(super) struct Touch {
    /// `None` when the GPU would not make a target (said once): nothing is drawn then.
    target: Option<RenderTarget>,
    vertices: u32,
}

impl Touch {
    /// Made with the renderer, so that what it costs is the start's and no frame's.
    pub(super) fn new(gl: &Gles) -> Self {
        let target = match gl.new_render_target(SIDE, SIDE) {
            Ok(target) => Some(target),
            Err(e) => {
                log(format!(
                    "no target to draw a producer's buffers once, so they are first drawn in a frame: {e}"
                ));
                None
            }
        };
        Self {
            target,
            vertices: gl.new_vertex_buffer(),
        }
    }

    /// Draws the picture `texture` shows (of `spec`'s size) over the whole of the target, with the image program the layers are drawn
    /// with, and hands the draw to the GPU. The frames set the state they need (target, viewport, blending, vertices) whenever
    /// they begin a pass, so nothing is put back.
    pub(super) fn draw(
        &mut self,
        gl: &Gles,
        image: &ImageProgram,
        panel: &Panel,
        spec: &Spec,
        texture: u32,
    ) {
        let Some(target) = self.target else {
            return;
        };
        let (width, height) = (spec.width as f32, spec.height as f32);
        // The program places vertices by the panel's pixels (`u_ndc`) and reads the picture by its own (`u_texel`): the quad is the
        // panel's size and shows the picture's corners at its own.
        let (panel_w, panel_h) = (panel.width as f32, panel.height as f32);
        let quad = [
            [0.0, 0.0, 0.0, 0.0],
            [panel_w, 0.0, width, 0.0],
            [0.0, panel_h, 0.0, height],
            [panel_w, panel_h, width, height],
        ];
        gl.bind_framebuffer(target.framebuffer);
        gl.viewport(0, 0, target.width as i32, target.height as i32);
        gl.disable(GL_SCISSOR_TEST);
        gl.disable(GL_BLEND);
        gl.use_program(image.id);
        gl.use_vertex_buffer(self.vertices);
        gl.stream_vertices(self.vertices, &quad);
        gl.bind_texture(texture);
        gl.uniform2f(image.texel, 1.0 / width, 1.0 / height);
        gl.uniform1f(image.opacity, 1.0);
        gl.draw_triangle_strip(0, 4);
        gl.flush();
    }

    /// The first pixel of the target: what the last draw left there, for the tests.
    #[cfg(test)]
    pub(super) fn pixel(&self, gl: &Gles) -> Option<[u8; 4]> {
        let target = self.target?;
        gl.bind_framebuffer(target.framebuffer);
        let pixels = gl.read_pixels(0, 0, 1, 1);
        Some([pixels[0], pixels[1], pixels[2], pixels[3]])
    }
}
