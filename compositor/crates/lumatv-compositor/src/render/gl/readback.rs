//! A recording's read of the screen without the loop waiting for the GPU: the frame is read into a pixel pack buffer (the GPU does it on its
//! own time) and mapped on the next call, a frame later, once it is long done. A screenshot, which is one picture and wanted now, reads
//! synchronously (`Gl::grab_target`); so does a driver that has no pack buffers or fails one.

use std::io;

use super::{Gl, TargetId, egl};
use crate::output::Image;
use egl::GLuint;

/// What the two buffers of the ring hold.
#[derive(Default)]
pub(super) struct Readback {
    buffers: [GLuint; 2],
    /// The size the buffers are made for (zero: none are).
    size: (u32, u32),
    /// The tag of the picture read into each, until it is taken.
    pending: [Option<u64>; 2],
    /// The one the next read goes into.
    next: usize,
    /// A buffer or its mapping failed once: every read after it is the synchronous one.
    failed: bool,
}

impl Gl {
    /// Frees the buffers (a capture is over, or a size changed); a picture still in them is dropped.
    pub(super) fn readback_free(&mut self) {
        let rb = &mut self.readback;
        if rb.size != (0, 0)
            && let Some(delete) = self.egl.opt.glDeleteBuffers
        {
            // SAFETY: buffers this renderer made, on its current context.
            unsafe { delete(2, rb.buffers.as_ptr()) };
        }
        rb.buffers = [0; 2];
        rb.size = (0, 0);
        rb.pending = [None; 2];
        rb.next = 0;
    }

    /// Whether the pack buffers can be used: all their entry points are there, the context is GLES 3 (an ES 2 context has the symbols on some
    /// drivers and fails the calls), and none has failed yet.
    fn readback_usable(&self) -> bool {
        let o = &self.egl.opt;
        let es3 = || {
            // SAFETY: GL_VERSION is a NUL-terminated string on a current context.
            let v = unsafe { (self.egl.api.glGetString)(egl::GL_VERSION) };
            !v.is_null() && unsafe { std::ffi::CStr::from_ptr(v as *const std::ffi::c_char) }.to_bytes().starts_with(b"OpenGL ES 3")
        };
        !self.readback.failed
            && o.glGenBuffers.is_some()
            && o.glDeleteBuffers.is_some()
            && o.glBindBuffer.is_some()
            && o.glBufferData.is_some()
            && o.glMapBufferRange.is_some()
            && o.glUnmapBuffer.is_some()
            && es3()
    }

    fn readback_make(&mut self, size: (u32, u32)) -> bool {
        self.readback_free();
        let (Some(gen_buffers), Some(bind), Some(data)) = (self.egl.opt.glGenBuffers, self.egl.opt.glBindBuffer, self.egl.opt.glBufferData) else { return false };
        let len = size.0 as isize * size.1 as isize * 4;
        // SAFETY: GL object creation on a current context; the buffers are sized for one picture each and unbound afterwards.
        unsafe {
            gen_buffers(2, self.readback.buffers.as_mut_ptr());
            for b in self.readback.buffers {
                bind(egl::GL_PIXEL_PACK_BUFFER, b);
                data(egl::GL_PIXEL_PACK_BUFFER, len, std::ptr::null(), egl::GL_STREAM_READ);
            }
            bind(egl::GL_PIXEL_PACK_BUFFER, 0);
        }
        self.readback.size = size;
        self.egl.gl_error() == egl::GL_NO_ERROR
    }

    /// [`Gl::grab_target`] a frame behind: this picture is read into a pack buffer without waiting, and the one read by the call before comes
    /// back, with the tag it was given, drawn into `spare` (`None` for the first picture of a capture, and after a size change).
    pub fn grab_target_deferred(&mut self, source: TargetId, size: (u32, u32), spare: Vec<u8>, tag: u64) -> io::Result<Option<(u64, Image)>> {
        if !self.readback_usable() {
            return self.grab_target(source, size, spare).map(|image| Some((tag, image)));
        }
        if self.readback.size != size && !self.readback_make(size) {
            self.readback_fail("making the pack buffers");
            return self.grab_target(source, size, spare).map(|image| Some((tag, image)));
        }
        self.grab_draw(source, size)?;
        let (w, h) = size;
        let slot = self.readback.next;
        let Some(bind) = self.egl.opt.glBindBuffer else { return self.grab_target(source, size, spare).map(|image| Some((tag, image))) };
        // SAFETY: the read goes into the bound pack buffer (a null pointer is an offset of 0), which holds w*h*4 bytes; GL_PACK_ALIGNMENT 4 matches
        // the rows of a 4-byte pixel.
        unsafe {
            let a = &self.egl.api;
            (a.glPixelStorei)(egl::GL_PACK_ALIGNMENT, 4);
            bind(egl::GL_PIXEL_PACK_BUFFER, self.readback.buffers[slot]);
            (a.glReadPixels)(0, 0, w as i32, h as i32, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, std::ptr::null_mut());
            bind(egl::GL_PIXEL_PACK_BUFFER, 0);
        }
        self.readback.pending[slot] = Some(tag);
        self.readback.next = 1 - slot;
        if self.egl.gl_error() != egl::GL_NO_ERROR {
            self.readback_fail("reading into a pack buffer");
            return self.grab_target(source, size, spare).map(|image| Some((tag, image)));
        }
        // The one read before this: a whole frame has gone by, so the GPU is done with it and the map does not wait.
        self.readback_take(1 - slot, size, spare)
    }

    /// The picture still in the buffers (read by the last call of [`Gl::grab_target_deferred`]), mapped and drawn into `spare`.
    pub fn grab_take_deferred(&mut self, size: (u32, u32), spare: Vec<u8>) -> io::Result<Option<(u64, Image)>> {
        // `next` is the buffer the next read would go into, so the last one read is the other.
        let last = 1 - self.readback.next;
        if self.readback.size != size {
            return Ok(None);
        }
        self.readback_take(last, size, spare)
    }

    fn readback_take(&mut self, slot: usize, size: (u32, u32), mut rgba: Vec<u8>) -> io::Result<Option<(u64, Image)>> {
        let Some(tag) = self.readback.pending[slot].take() else { return Ok(None) };
        let (Some(bind), Some(map), Some(unmap)) = (self.egl.opt.glBindBuffer, self.egl.opt.glMapBufferRange, self.egl.opt.glUnmapBuffer) else {
            return Ok(None);
        };
        let (w, h) = size;
        let len = w as usize * h as usize * 4;
        rgba.clear();
        // SAFETY: the buffer holds len bytes; the mapping is read once, copied out and released before anything else touches the buffer.
        let mapped = unsafe {
            bind(egl::GL_PIXEL_PACK_BUFFER, self.readback.buffers[slot]);
            let p = map(egl::GL_PIXEL_PACK_BUFFER, 0, len as isize, egl::GL_MAP_READ_BIT);
            if !p.is_null() {
                rgba.extend_from_slice(std::slice::from_raw_parts(p as *const u8, len));
                unmap(egl::GL_PIXEL_PACK_BUFFER);
            }
            bind(egl::GL_PIXEL_PACK_BUFFER, 0);
            !p.is_null()
        };
        if !mapped || self.egl.gl_error() != egl::GL_NO_ERROR {
            self.readback_fail("mapping a pack buffer");
            return Err(egl::other("grab: a pack buffer could not be mapped"));
        }
        Ok(Some((tag, Image { width: w, height: h, rgba })))
    }

    fn readback_fail(&mut self, what: &str) {
        log::warn!("screencap: {what} failed; the recording reads the screen synchronously from here");
        self.readback_free();
        self.readback.failed = true;
    }
}
