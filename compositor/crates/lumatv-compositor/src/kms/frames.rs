//! The frame-to-frame logic of the scan-out ring, apart from the device: which page the next frame is composed into, what each
//! page owes (`damage::Ring`), when the sole full-screen window goes to the plane directly instead, and what the UI engine does
//! in a frame that composes nothing. Nothing here touches DRM, so `tests/ui_scanout.rs` drives it on a host with the GL renderer
//! and pages in its own memory.
//!
//! Direct scan-out is taken whenever the plan allows it (one opaque window that covers the output, no hole, no cursor) and the
//! UI shows nothing over the windows: what the UI has under them is covered, so a UI with Home alone does not cost a composed
//! frame per refresh. The frame a system layer appears (a volume bar) is composed, and the one it goes is direct again. The
//! pages miss every frame of a direct run (the window's own buffers went to the plane), so the first composed frame after one
//! is drawn whole: the ring forgets its pages.

use std::io;

use crate::Rect;
use crate::damage::{Ring, Tiles};
use crate::output::RenderStats;
use crate::render::gl::{Gl, TargetId};
use crate::scene::Plan;
use crate::surface::Surfaces;

use super::assign::Source;

pub struct Frames {
    targets: Vec<TargetId>,
    next: usize,
    ring: Ring,
    rects: Vec<Rect>,
    last_direct: bool,
    width: u32,
    height: u32,
}

impl Frames {
    /// A ring over `targets`, the pages in the order they are turned through; each owes everything at first.
    pub fn new(targets: Vec<TargetId>, width: u32, height: u32) -> Frames {
        assert!(!targets.is_empty(), "a ring needs a page");
        Frames { ring: Ring::new(targets.len(), width, height), targets, next: 0, rects: Vec::with_capacity(16), last_direct: false, width, height }
    }

    /// The page the last composed frame went into.
    pub fn last_page(&self) -> usize {
        (self.next + self.targets.len() - 1) % self.targets.len()
    }

    /// A frame that was composed into `page` never went out (superseded while a flip was pending): the next frame is drawn into
    /// that page again. The ring owes it the damage since its own drawing, as for any page.
    pub fn redraw(&mut self, page: usize) {
        self.next = page;
    }

    /// Draws the frame: `direct` is the sole full-screen window's framebuffer when the plane could take it, which it gets unless
    /// the UI shows something over the windows; otherwise the frame is composed into the next page, inside what that page owes.
    /// The UI engine takes the app's commits in either way, and a direct frame carries its receipt as a composed one does.
    pub fn draw(&mut self, gl: &mut Gl, plan: &Plan, surfaces: &Surfaces, direct: Option<u32>) -> io::Result<(Source, RenderStats)> {
        let mut tiles = plan.tiles.as_ref().cloned().unwrap_or_else(|| Tiles::new(self.width, self.height));
        tiles.add(&plan.damage);
        if gl.take_redraw_all() {
            self.ring.forget();
        }
        gl.plan_ui(&mut tiles)?;
        if let Some(fb) = direct.filter(|_| !gl.ui_over_windows()) {
            gl.skip_ui_frame()?;
            self.last_direct = true;
            return Ok((Source::Direct(fb), RenderStats { items: plan.items.len(), ..Default::default() }));
        }
        if std::mem::take(&mut self.last_direct) {
            self.ring.forget();
        }
        let page = self.next;
        let owed = self.ring.frame(page, &tiles);
        owed.rects(&mut self.rects);
        gl.set_target(self.targets[page]);
        let stats = gl.render_rects(plan, surfaces, &self.rects)?;
        self.next = (page + 1) % self.targets.len();
        Ok((Source::Page(page), stats))
    }
}
