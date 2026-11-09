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
//!
//! A composed frame with the system UI over Home (a panel, Settings, a toast; and the few frames after, while the pages still owe
//! its place: `Gl::ui_over_home`) draws only what it changes. What its page owes from the frames before (a ring of three pages owes
//! each change three times) is what the page the last frame went into holds there, and it is copied from that page first
//! (`Gl::copy_from_page`): a whole-screen change costs one frame of drawing and two copies, not three frames. Otherwise (Home's own
//! walks) the page draws all it owes: with the copies and the engine's floor, Home's walks dropped 29 (apps-grid) and 28
//! (home-widgets) frames at 120 Hz against 3 and 3 without (round 27). `LUMATV_PAGE_COPY=0` draws all a page owes always.
//!
//! A calm frame copies as well (`Frames::set_calm`): one that comes [`AMBIENT_COPY_REFRESHES`] refreshes or more after the frame before,
//! while nothing the owner drives is moving (the engine asks for no frame at every refresh). The weather's sky and the wallpaper's steps
//! (10 a second) make such frames; with a ring of three pages every one of them owed the whole screen, 90 layers and 119 draws, for the
//! change of a sky's box. Copying costs one blit (a pass over the page, with the time for it) and the frame draws what it changed.
//! `LUMATV_AMBIENT_COPY=0` draws all a page owes in them.
//!
//! A frame that changes nothing composes nothing (`Frames::set_still`): no window damage, nothing of the UI changed (Home applied again
//! with what it had, the focus's rest redrawing the same, a lift's slot freed at its rest pose, the engine's floor brought up to date,
//! a redraw after work between frames that changed nothing shown), nothing moving after the frame before, and the page on the screen
//! the last one composed. That page holds the frame already, so no page is drawn or turned and nothing is flipped; the engine's frame
//! is ended and answered at once. Such commits came 2 to 6 refreshes after a motion's last frame, and each flip of the same picture
//! was a refresh missed to the 120 Hz gate (session 10: home-widgets 6, home-dpad 1, where the host's walks have them).
//! A motion's first frame is one too: the engine starts a motion when its commit is taken in, so the frame drawn then shows the
//! motion at its start, the picture already on the screen. Flipped, it began the motion a refresh before the first frame that moved
//! (820992 px, 8 ms to GPU done), which then went out at the refresh after the next: 3 of the 4 refreshes Home's d-pad missed on
//! 2026-11-09 (the ring's `U` lines). `LUMATV_KEEP_UNCHANGED=0` draws and flips them.

use std::io;

use crate::Rect;
use crate::damage::{Ring, Tiles};
use crate::output::RenderStats;
use crate::render::gl::{Gl, TargetId};
use crate::scene::Plan;
use crate::surface::Surfaces;

use super::assign::{Assignment, Facts, Source, decide, needs_commit};

/// Refreshes a frame must come after the frame before to be calm (`Frames::set_calm`); the engine's host counts the same
/// (`lumatv_render::host::AMBIENT_COPY_REFRESHES`).
pub const AMBIENT_COPY_REFRESHES: u64 = 3;

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
    /// The page the last composed frame went into, which holds that frame whole: the next page copies from it what it owes from
    /// the frames before its own. None after a frame that composed nothing, or a ring or a page forgotten.
    composed: Option<usize>,
    page_copy: bool,
    /// Calm frames copy too (`LUMATV_AMBIENT_COPY=0` switches it off), and whether the next frame is one (`Frames::set_calm`).
    ambient_copy: bool,
    calm: bool,
    /// Whether the next frame may compose nothing when it changes nothing (`Frames::set_still`), and whether a motion ran after the frame
    /// drawn last: the frame after one is drawn whatever it changes, so that a motion's last refresh is a flip like the ones before it
    /// ([`keeps_unchanged`]).
    still: bool,
    moving: bool,
    /// `LUMATV_KEEP_UNCHANGED=0`: a frame that changes nothing is drawn and flipped, as before.
    keep: bool,
    copied: Tiles,
    copy_rects: Vec<Rect>,
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
            composed: None,
            page_copy: std::env::var("LUMATV_PAGE_COPY").as_deref() != Ok("0"),
            ambient_copy: std::env::var("LUMATV_AMBIENT_COPY").as_deref() != Ok("0"),
            calm: false,
            still: false,
            moving: false,
            keep: std::env::var("LUMATV_KEEP_UNCHANGED").as_deref() != Ok("0"),
            copied: Tiles::new(width, height),
            copy_rects: Vec::with_capacity(16),
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

    /// Says whether the next frame is calm: it comes [`AMBIENT_COPY_REFRESHES`] or more after the frame before. It counts for that frame only.
    pub fn set_calm(&mut self, calm: bool) {
        self.calm = calm;
    }

    /// Says whether the next frame may compose nothing when it changes nothing: no flip is in flight and no frame waits behind one (the
    /// screen shows the page composed last), and no capture waits for a page. It counts for that frame only.
    pub fn set_still(&mut self, still: bool) {
        self.still = still;
    }

    /// The page the next composed frame goes into: not on the screen and not in a flip (the ring turns one ahead of both, and a frame that was
    /// superseded gave its page back through [`Frames::redraw`]).
    pub fn next_page(&self) -> usize {
        self.next
    }

    /// Page `page` is another buffer now (`Kms::set_page_kind`): the ring draws into `target` and owes the page whole. Returns the target it had.
    pub fn replace_target(&mut self, page: usize, target: TargetId) -> TargetId {
        self.ring.forget_page(page);
        if self.composed == Some(page) {
            self.composed = None;
        }
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
        // (before the redraw check: a change of the answer is a frame that draws all of its page)
        gl.set_home_hidden(plan.home_hidden);
        if gl.take_redraw_all() {
            self.ring.forget();
            self.ui_ring.forget();
            self.composed = None;
            // The pages hold layers that are gone: nothing to stand in from until this frame is composed.
            gl.set_standin_source(None);
        }
        let standin = gl.prepare_ui_standin(plan);
        let direct = if standin { None } else { direct };
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
        // (a frame with the screen showing the page composed last and no motion before it: `set_still`, the module's notes)
        let still = keeps_unchanged(std::mem::take(&mut self.still) && self.keep && self.composed.is_some() && !standin, self.moving);
        self.moving = plan.animating || gl.ui_animating();
        if still && !needs_commit(&assignment, !tiles.is_empty()) {
            self.calm = false;
            gl.skip_ui_frame()?;
            return Ok((Assignment { primary: Source::Off, overlay: None, at: (0, 0) }, RenderStats { items: plan.items.len(), unchanged: true, ..Default::default() }));
        }
        let stats = match assignment {
            Assignment { primary: Source::Direct(_), overlay: None, .. } => {
                gl.skip_ui_frame()?;
                gl.set_standin_source(None);
                self.last_direct = true;
                self.last_split = false;
                self.composed = None;
                RenderStats { items: plan.items.len(), ..Default::default() }
            }
            Assignment { primary: Source::Direct(_), overlay: Some((Source::UiPage(page), _)), .. } => {
                if !std::mem::replace(&mut self.last_split, true) {
                    self.ui_ring.forget();
                }
                gl.set_standin_source(None);
                self.last_direct = true;
                self.composed = None;
                let owed = self.ui_ring.frame(page, &tiles);
                owed.rects(&mut self.rects);
                gl.merge_ui_rects(&mut self.rects);
                gl.set_target(self.ui_targets[page]);
                let stats = gl.render_ui_layer(&self.rects)?;
                self.ui_next = (page + 1) % self.ui_targets.len();
                RenderStats { items: plan.items.len(), ..stats }
            }
            _ => {
                if std::mem::take(&mut self.last_direct) {
                    self.ring.forget();
                    self.composed = None;
                }
                self.last_split = false;
                let page = self.next;
                let owed = self.ring.frame(page, &tiles);
                gl.set_target(self.targets[page]);
                if standin { gl.snapshot_aliased_standin()?; }
                // What the page owes from the frames before this one is in the page the last frame went into.
                let mut copied = false;
                // (a calm frame: nothing of the owner's is moving, and the engine's next frame is not at the next refresh)
                let calm = std::mem::take(&mut self.calm) && self.ambient_copy && !gl.ui_animating();
                if let Some(last) = self.composed.filter(|&p| p != page && self.page_copy && (calm || gl.ui_over_home())) {
                    self.copied.clear();
                    self.copied.union(owed);
                    self.copied.subtract(&tiles);
                    self.copied.rects(&mut self.copy_rects);
                    copied = gl.copy_from_page(self.targets[last], &self.copy_rects);
                }
                if copied { tiles.rects(&mut self.rects) } else { owed.rects(&mut self.rects) }
                gl.merge_ui_rects(&mut self.rects);
                let stats = gl.render_rects(plan, surfaces, &self.rects)?;
                // What the page only copied from the last one it owes again; the page is the next frame's stand-in.
                self.ring.owe(page, gl.standin_tiles());
                gl.set_standin_source(Some(self.targets[page]));
                self.next = (page + 1) % self.targets.len();
                self.composed = Some(page);
                stats
            }
        };
        Ok((assignment, stats))
    }
}

/// Whether a frame that changes nothing may compose nothing: it `may` (no flip in flight, no frame waiting, the screen showing the page
/// composed last, no stand-in owed), and no motion ran after the frame before (`moved`). A motion that starts with this frame does not
/// count: its first frame shows the picture on the screen; the frame after it is drawn and flipped whatever it changes.
fn keeps_unchanged(may: bool, moved: bool) -> bool {
    may && !moved
}

#[cfg(test)]
mod tests {
    use super::keeps_unchanged;

    #[test]
    fn a_motions_unchanged_first_frame_is_kept_and_the_frame_after_a_motion_is_flipped() {
        assert!(keeps_unchanged(true, false), "nothing moved before it: kept, whether or not a motion starts with it");
        assert!(!keeps_unchanged(true, true), "a motion ran after the frame before: drawn and flipped");
        assert!(!keeps_unchanged(false, false));
    }
}
