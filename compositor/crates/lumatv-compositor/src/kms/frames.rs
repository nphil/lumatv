//! The frame-to-frame logic of the scan-out rings, apart from the device: which page the next frame is composed into, what each
//! page owes (`damage::Ring`), when the sole full-screen window goes to the plane directly instead, when the system UI goes to
//! the overlay plane over it, and what the UI engine does in a frame that composes nothing. Nothing here touches DRM, so
//! `tests/ui_scanout.rs` drives it on a host with the GL renderer and pages in its own memory.
//!
//! Direct scan-out is taken whenever the plan allows it (one opaque window that covers the output, no hole, no cursor). While
//! the UI shows nothing over the windows the overlay is off: what the UI has under them is covered, so a UI with Home alone
//! does not cost a composed frame per refresh. The frame a system layer appears (a volume bar) the UI alone is drawn into a
//! page of the UI ring (cleared to transparent where it owes, stack 1 over that) and that page goes to the overlay plane with
//! the window still direct under it; without a UI ring (no overlay plane, or one that takes no alpha format) the frame is
//! composed as any other. The pages of a ring miss every frame it is not drawn into, so the first frame back into it is drawn
//! whole: the ring forgets its pages.

use std::io;

use crate::Rect;
use crate::damage::{Ring, Tiles};
use crate::output::RenderStats;
use crate::render::gl::{Gl, TargetId};
use crate::scene::Plan;
use crate::surface::Surfaces;

use super::assign::{Assignment, Facts, Source, decide};

pub struct Frames {
    targets: Vec<TargetId>,
    next: usize,
    ring: Ring,
    ui_targets: Vec<TargetId>,
    ui_next: usize,
    ui_ring: Ring,
    rects: Vec<Rect>,
    /// What the last frame did: a frame into a ring the last one skipped redraws the ring whole.
    last_direct: bool,
    last_split: bool,
    width: u32,
    height: u32,
}

impl Frames {
    /// A ring over `targets`, the pages in the order they are turned through; each owes everything at first.
    pub fn new(targets: Vec<TargetId>, width: u32, height: u32) -> Frames {
        assert!(!targets.is_empty(), "a ring needs a page");
        Frames {
            ring: Ring::new(targets.len(), width, height),
            targets,
            next: 0,
            ui_targets: Vec::new(),
            ui_next: 0,
            ui_ring: Ring::new(1, width, height),
            rects: Vec::with_capacity(16),
            last_direct: false,
            last_split: false,
            width,
            height,
        }
    }

    /// Adds the UI ring: pages with alpha for the overlay plane. Without it the UI over a direct window means composition.
    pub fn with_ui_pages(mut self, targets: Vec<TargetId>) -> Frames {
        assert!(!targets.is_empty(), "a ring needs a page");
        self.ui_ring = Ring::new(targets.len(), self.width, self.height);
        self.ui_targets = targets;
        self
    }

    /// The page the next composed frame goes into: not on the screen and not in a flip (the ring turns one ahead of both, and a frame that was
    /// superseded gave its page back through [`Frames::redraw`]).
    pub fn next_page(&self) -> usize {
        self.next
    }

    /// Page `page` is another buffer now (`Kms::set_page_kind`): the ring draws into `target` and owes the page whole. Returns the target it had.
    pub fn replace_target(&mut self, page: usize, target: TargetId) -> TargetId {
        self.ring.forget_page(page);
        std::mem::replace(&mut self.targets[page], target)
    }

    /// The page the last composed frame went into.
    pub fn last_page(&self) -> usize {
        (self.next + self.targets.len() - 1) % self.targets.len()
    }

    /// The UI page the last split frame went into.
    pub fn last_ui_page(&self) -> usize {
        (self.ui_next + self.ui_targets.len() - 1) % self.ui_targets.len().max(1)
    }

    /// A frame that was composed into a page never went out (superseded while a flip was pending): the next frame into that
    /// ring is drawn into the same page again. The ring owes it the damage since its own drawing, as for any page.
    pub fn redraw(&mut self, source: Source) {
        match source {
            Source::Page(i) => self.next = i,
            Source::UiPage(i) => self.ui_next = i,
            Source::Direct(_) | Source::Off => {}
        }
    }

    /// Draws the frame: `direct` is the sole full-screen window's framebuffer when the plane could take it, with where the plane is to put it
    /// (the OLED care's orbit, or the origin). The UI engine takes the app's commits in either way, and a direct frame carries its receipt as
    /// a composed one does.
    pub fn draw(&mut self, gl: &mut Gl, plan: &Plan, surfaces: &Surfaces, direct: Option<(u32, (i32, i32))>) -> io::Result<(Assignment, RenderStats)> {
        let mut tiles = plan.tiles.as_ref().cloned().unwrap_or_else(|| Tiles::new(self.width, self.height));
        tiles.add(&plan.damage);
        if gl.take_redraw_all() {
            self.ring.forget();
            self.ui_ring.forget();
            // The pages hold layers that are gone: nothing to stand in from until this frame is composed.
            gl.set_standin_source(None);
        }
        gl.plan_ui(&mut tiles)?;
        // The rectangles the last frame stood in for (a buffer the driver was busy with): their real content is owed.
        gl.owed_standins(&mut tiles);
        let facts = Facts {
            sole_fullscreen: direct.is_some(),
            layout_accepted: direct.is_some(),
            direct_fb: direct,
            next_page: self.next,
            ui_over: gl.ui_over_windows(),
            ui_page: if self.ui_targets.is_empty() { None } else { Some(self.ui_next) },
            output: Rect::new(0, 0, self.width as i32, self.height as i32),
        };
        let assignment = decide(&facts);
        let stats = match assignment {
            Assignment { primary: Source::Direct(_), overlay: None, .. } => {
                gl.skip_ui_frame()?;
                gl.set_standin_source(None);
                self.last_direct = true;
                self.last_split = false;
                RenderStats { items: plan.items.len(), ..Default::default() }
            }
            Assignment { primary: Source::Direct(_), overlay: Some((Source::UiPage(page), _)), .. } => {
                if !std::mem::replace(&mut self.last_split, true) {
                    self.ui_ring.forget();
                }
                gl.set_standin_source(None);
                self.last_direct = true;
                let owed = self.ui_ring.frame(page, &tiles);
                owed.rects(&mut self.rects);
                gl.set_target(self.ui_targets[page]);
                let stats = gl.render_ui_layer(&self.rects)?;
                self.ui_next = (page + 1) % self.ui_targets.len();
                RenderStats { items: plan.items.len(), ..stats }
            }
            _ => {
                if std::mem::take(&mut self.last_direct) {
                    self.ring.forget();
                }
                self.last_split = false;
                let page = self.next;
                let owed = self.ring.frame(page, &tiles);
                owed.rects(&mut self.rects);
                gl.set_target(self.targets[page]);
                let stats = gl.render_rects(plan, surfaces, &self.rects)?;
                // What the page only copied from the last one it owes again; the page is the next frame's stand-in.
                self.ring.owe(page, gl.standin_tiles());
                gl.set_standin_source(Some(self.targets[page]));
                self.next = (page + 1) % self.targets.len();
                stats
            }
        };
        Ok((assignment, stats))
    }
}
