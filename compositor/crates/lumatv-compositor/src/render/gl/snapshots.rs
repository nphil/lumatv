//! Half-size app-only GPU copies, and painted cards for the windows that cannot be copied. No client-buffer references or readback survive
//! capture.

use wayland_server::Resource;

use super::{Gl, egl, offscreen_target, set_params};
use crate::buffer::{BufferKindRef, kind_of};
use crate::scene::{Plan, Posed};
use crate::snapshot::{BUDGET_BYTES, Draw, PICTURE_BYTES, PICTURE_HEIGHT, PICTURE_WIDTH, Receipt, SnapshotId, Snapshots};
use crate::surface::Surfaces;
use crate::transition::FRect;
use crate::window::WindowId;
use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect};

pub(super) struct Picture {
    id: SnapshotId,
    tex: egl::GLuint,
    fbo: egl::GLuint,
    top_down: bool,
    content: u64,
}

impl Gl {
    pub fn app_snapshot_count(&self) -> usize {
        self.app_snapshots.len()
    }

    /// An allocation for one more picture: an evicted one's, recycled in the same GL stream (deleting then allocating could briefly retain
    /// one image too many in the driver), or a new one while the budget has room. Leaves the new target's framebuffer bound.
    fn picture_allocation(&mut self, live: &Snapshots) -> Option<(egl::GLuint, egl::GLuint)> {
        let reusable = self.app_snapshots.iter().position(|p| !live.contains(p.id));
        if (self.width, self.height) != (OUTPUT_WIDTH, OUTPUT_HEIGHT)
            || (reusable.is_none() && (self.app_snapshots.len() + 1) * PICTURE_BYTES > BUDGET_BYTES)
        {
            return None;
        }
        if let Some(index) = reusable {
            let p = self.app_snapshots.remove(index);
            return Some((p.fbo, p.tex));
        }
        let Ok(target) = offscreen_target(&self.egl, PICTURE_WIDTH as u32, PICTURE_HEIGHT as u32) else {
            self.restore_target();
            return None;
        };
        Some(target)
    }

    pub fn freeze_app(&mut self, id: SnapshotId, window: WindowId, plan: &Plan, live: &Snapshots) -> bool {
        let items = || plan.items.iter().filter(|i| i.window == window);
        if items().next().is_none() || items().any(|i| {
            !self.textures.get(&i.buffer.id()).is_some_and(|t| t.snapshot_rgb && t.uploaded && t.warm_sync.is_none() && !t.held)
        }) {
            return false;
        }
        let Some((fbo, tex)) = self.picture_allocation(live) else { return false };
        // SAFETY: the texture belongs to this context.
        unsafe {
            (self.egl.api.glBindTexture)(egl::GL_TEXTURE_2D, tex);
            set_params(&self.egl.api);
        }
        if !self.copy_app_picture(fbo, window, plan) {
            self.free_app_picture(Picture { id, tex, fbo, top_down: false, content: 0 });
            return false;
        }
        self.app_snapshots.push(Picture { id, tex, fbo, top_down: self.targets[self.current.0].top_down, content: 0 });
        // Submit the copy before a client's background notification can make its next writes runnable.
        if let Some(fence) = self.egl.kick_fence() {
            // SAFETY: consumes this context's fence without waiting.
            unsafe { self.egl.wait_fence(fence, 0) };
        } else {
            // SAFETY: submits this context's work; no CPU readback or wait.
            unsafe { (self.egl.api.glFlush)() };
        }
        true
    }

    /// Keeps `rgba` (premultiplied, `PICTURE_WIDTH` by `PICTURE_HEIGHT`, top row first) as the window's picture: the card of a window that
    /// is video on the plane and has no pixels of its own to copy (`card.rs`).
    pub fn freeze_card(&mut self, id: SnapshotId, rgba: &[u8], live: &Snapshots) -> bool {
        if rgba.len() != PICTURE_BYTES {
            return false;
        }
        let Some((fbo, tex)) = self.picture_allocation(live) else { return false };
        // SAFETY: the texture belongs to this context and is the picture's size; `rgba` is one picture of tightly packed rows.
        unsafe {
            let a = &self.egl.api;
            (a.glBindTexture)(egl::GL_TEXTURE_2D, tex);
            set_params(a);
            (a.glPixelStorei)(egl::GL_UNPACK_ALIGNMENT, 4);
            (a.glTexSubImage2D)(egl::GL_TEXTURE_2D, 0, 0, 0, PICTURE_WIDTH as i32, PICTURE_HEIGHT as i32, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, rgba.as_ptr().cast());
        }
        let error = self.egl.gl_error();
        self.restore_target();
        let picture = Picture { id, tex, fbo, top_down: true, content: 0 };
        if error != egl::GL_NO_ERROR {
            self.free_app_picture(picture);
            return false;
        }
        self.app_snapshots.push(picture);
        true
    }

    fn copy_app_picture(&mut self, fbo: egl::GLuint, window: WindowId, plan: &Plan) -> bool {
        self.use_plain();
        self.set_dim(1.0);
        // Preserve the app group's own alpha; neither a system background nor an invented opaque canvas belongs in this picture.
        // SAFETY: this independent framebuffer and every source texture belong to this context.
        unsafe {
            let a = &self.egl.api;
            (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
            (a.glViewport)(0, 0, PICTURE_WIDTH as i32, PICTURE_HEIGHT as i32);
            (a.glDisable)(egl::GL_SCISSOR_TEST);
            (a.glClearColor)(0.0, 0.0, 0.0, 0.0);
            (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
            (a.glEnable)(egl::GL_BLEND);
            (a.glBlendFunc)(egl::GL_ONE, egl::GL_ONE_MINUS_SRC_ALPHA);
        }
        for item in plan.items.iter().filter(|i| i.window == window) {
            let t = &self.textures[&item.buffer.id()];
            self.draw_quad(t.tex, &item.rect, t.y_invert);
        }
        let error = self.egl.gl_error();
        self.restore_target();
        error == egl::GL_NO_ERROR
    }

    fn free_app_picture(&self, picture: Picture) {
        // SAFETY: these independent objects belong to this renderer; GL defers freeing in-flight uses.
        unsafe {
            (self.egl.api.glDeleteFramebuffers)(1, &picture.fbo);
            (self.egl.api.glDeleteTextures)(1, &picture.tex);
        }
    }

    pub fn retain_app_snapshots(&mut self, live: &Snapshots) {
        let mut i = 0;
        while i < self.app_snapshots.len() {
            if live.contains(self.app_snapshots[i].id) {
                i += 1;
            } else {
                let picture = self.app_snapshots.remove(i);
                self.free_app_picture(picture);
            }
        }
    }

    pub(super) fn free_app_snapshots(&mut self) {
        while let Some(picture) = self.app_snapshots.pop() {
            self.free_app_picture(picture);
        }
    }

    /// A complete new app picture, not callbacks, an imported-but-busy buffer, or another window's damage.
    pub(super) fn prepare_snapshot_live(&mut self, plan: &Plan, surfaces: &Surfaces) -> Option<Receipt> {
        let full = Rect::new(0, 0, self.width as i32, self.height as i32);
        let draw = plan.snapshots.iter().find(|p| p.resume.is_some() && p.pose.is_rest_on(&FRect::from_rect(full)))?;
        let previous_content = self.app_snapshots.iter().find(|p| p.id == draw.id)?.content;
        let resume = draw.resume?;
        let items = || plan.items.iter().filter(|i| i.window == draw.window);
        let content = items().filter_map(|i| surfaces.get(i.surface).map(|s| s.current.content_serial)).max()?;
        if content <= resume.after || !items().any(|i| i.surface == draw.root && i.rect == full)
            || items().any(|i| i.ghost || i.xform || i.drift != (0, 0) || i.dim != 1.0)
            || plan.gain_on || plan.masks.iter().any(|m| m.window == draw.window)
        {
            return None;
        }
        for item in items() {
            self.import(&item.buffer)?;
            let t = self.textures.get(&item.buffer.id())?;
            if !t.snapshot_rgb || t.warm_sync.is_some() || t.held { return None; }
        }
        if previous_content != content {
            // Damage can be consumed while the cover hides a shared-memory surface. Its first complete live group needs one full upload,
            // through the ordinary drawing path, before its receipt or saved picture can describe these commits.
            for item in items() {
                if matches!(kind_of(&item.buffer), BufferKindRef::Shm(_)) {
                    self.textures.get_mut(&item.buffer.id())?.uploaded = false;
                }
            }
        }
        Some(Receipt { id: draw.id, attempt: resume.attempt, content })
    }

    /// Whether the window's own items are left out of the frame for its picture. They are drawn under the picture on the receipt's frame, and
    /// all through its dissolve.
    pub(super) fn snapshot_replaces(&self, plan: &Plan, window: WindowId, receipt: Option<Receipt>) -> bool {
        plan.snapshots.iter().any(|p| p.window == window && !p.fading
            && self.app_snapshots.iter().any(|image| image.id == p.id)
            && !receipt.is_some_and(|r| r.id == p.id))
    }

    pub(super) fn draw_snapshots_before(&mut self, plan: &Plan, before: usize, receipt: Option<Receipt>, rects: &[Rect]) {
        for draw in plan.snapshots.iter().filter(|p| p.before == before) {
            // The picture is drawn over the live frame the receipt vouches for: that frame is under it from now on, and the picture is not
            // replaced by it (a window still warming its next buffer is covered by the same picture).
            if let Some(r) = receipt.filter(|r| r.id == draw.id)
                && let Some(p) = self.app_snapshots.iter_mut().find(|p| p.id == draw.id)
            {
                p.content = r.content;
            }
            self.draw_app_snapshot(draw, rects);
        }
    }

    fn draw_app_snapshot(&self, draw: &Draw, rects: &[Rect]) {
        let Some(picture) = self.app_snapshots.iter().find(|p| p.id == draw.id) else { return };
        let p = draw.pose;
        self.draw_pose_item(picture.tex, &Posed { quad: p.rect, clip: p.clip, radius: p.radius, alpha: p.alpha, shade: p.shade },
            1.0, picture.top_down, rects);
    }
}
