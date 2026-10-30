//! LumaTV's UI engine inside the compositor's frame (feature `ui`).
//!
//! What goes away with LG's compositor (LumaHome, the volume and mute OSD, toasts, alerts, the PIN prompt, the quick panel; the
//! plan is "Surfaces under LumaTV's compositor" in `docs/research/compositor-c2-result.md`) is drawn by LumaShow's renderer, which
//! `rustui/render` carries, in the compositor's own GL context and on its own thread, straight into the scan-out page: no window,
//! no copy pass, no second composition, no frame callback. The app that owns the layers (`lumatv-ui`, or the demo here) speaks
//! LumaShow's `compositor-protocol` over the socket [`Ui::new`] returns the end of.
//!
//! One engine, two stacks. Stack 0 is the layers under the windows (LumaHome, while the Home stub's card is in front); stack 1 is
//! the system layer over them (the layers whose z is `system_z` or more). A page is drawn as stack 0, the windows, stack 1, each
//! only inside the tiles the page owes, which the ring below says (`Gl::render_rects` and `kms::Kms::render`):
//!
//! ```text
//! if ui.wants_frame(now_ns) || <the windows changed> {               // every tick: Output::ui_needs_frame, else ui.idle
//!     let mut tiles = <the windows' damage this frame>;              // Plan::tiles
//!     ui.plan(now_ns, &mut tiles, home_hidden)?;                     // adds what the two stacks changed (stack 0 not while Home is hidden)
//!     let owed = ring.frame(page, &tiles);                           // damage::Ring, as for the windows alone
//!     ui.merge(&mut owed_rects);                                     // covers where one pass over the layers costs less than several
//!     gl.bind(page) ...
//!     ui.draw_under(fbo, owed)?;                                     // clears the owed rectangles and copies stack 0 there from its floor
//!     <the windows, inside the owed rectangles>
//!     ui.draw_over(fbo, owed)?;                                      // stack 1 over the windows, under the pointer
//!     let cost = ui.finish()?;                                       // the frame's receipt, kept with the page
//!     ... ui.presented(presented_ns, &cost);                         // once the page's flip has landed: the app is told
//! }
//! ```
//!
//! Stack 0 is kept whole in a picture of the engine's own, the floor (`lumatv_render::host`): a frame draws into it what Home
//! changed, and `draw_under` copies it into each owed rectangle, so a panel or Settings moving over a Home that stands draws its own
//! layers and one copy, not Home's 70 to 90.
//!
//! The context is the compositor's: every call saves its GL state, makes the context the toolkit's and puts the state back
//! ([`lumatv_render::Gles::enter`]), so `Gl` need not know. The toolkit's shaders are GLSL ES 3.00: the compositor's context has
//! to be OpenGL ES 3.0 or later (`render/egl.rs` asks for 2, which Mesa answers with 3.2 and the Mali blob answers [INFERENCE: not
//! seen on the set] with 3.x too; [`Ui::new`] refuses an ES 2 context and says which).
//!
//! The engine's 48 programs take the set's Mali 1.3 s to link, so an engine that attaches while the loop runs does not link them: a
//! thread of its own does, in a context that shares the loop's ([`compile`]), and [`Ui::new`] takes them from the cache that thread
//! left them in.

pub mod compile;
pub mod demo;

use crate::Rect;
use crate::damage::Tiles;
use lumatv_render::host::{Engine, FrameCost, HostConfig};
use lumatv_render::programs::Cache;
use lumatv_render::tiles::TileSet;
use lumatv_render::{Demand, Gles};
use std::ffi::CString;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;

/// The engine, and the scratch the compositor's tiles are turned into the engine's areas with.
pub struct Ui {
    engine: Engine,
    scratch: Vec<[i32; 4]>,
    areas: Vec<[i32; 4]>,
}

/// Adds every tile of `set` to `into` (the engine's tiles are the compositor's: 16 px from the output's corner).
pub fn add_tiles(into: &mut Tiles, set: &TileSet, scratch: &mut Vec<[i32; 4]>) {
    set.rects(scratch);
    for area in scratch.iter() {
        into.add(&Rect::new(area[0], area[1], area[2] - area[0], area[3] - area[1]));
    }
}

impl Ui {
    /// The engine, in the context that is current on this thread (the compositor's). `driver` is the EGL/GLES library the context
    /// is the driver's (`libmali-...so` on the set, Mesa's `libEGL.so.1` on a host). `system_z` is where stack 1 begins. The app's
    /// end of the socket is returned, and [`Ui::finish_handshake`] is called once the app has connected over it. The programs come from
    /// `programs` where it has them and are linked here, on the calling thread, where it has not (seconds on the set: only before the
    /// loop runs, or after [`compile`] has filled the cache), and go back to it when the `Ui` is dropped.
    ///
    /// # Safety
    /// A context of `driver` is current on this thread and stays so for as long as the `Ui` is used; every call of it is made on
    /// this thread.
    pub unsafe fn new(driver: &str, width: u32, height: u32, system_z: i32, shm_dir: PathBuf, programs: &Arc<Cache>) -> io::Result<(Self, UnixStream)> {
        let driver = CString::new(driver).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "driver path"))?;
        // SAFETY: the caller's.
        let mut gles = unsafe { Gles::current(&driver) }?;
        gles.use_program_cache(Arc::clone(programs));
        let (engine, app) = Engine::new(
            gles,
            HostConfig {
                width,
                height,
                refresh_hz: 60.0,
                shm_dir,
                direct: false,
                gpu_sync: false,
                idle_trim: false,
                pages: 0,
                reference: false,
                blit: false,
                external: true,
                system_z,
            },
        )?;
        Ok((Self { engine, scratch: Vec::new(), areas: Vec::new() }, app))
    }

    pub fn finish_handshake(&mut self) -> io::Result<()> {
        self.engine.finish_handshake()
    }

    /// Finishes the handshake if the app has said hello, without waiting ([`Engine::try_finish_handshake`]).
    pub fn try_finish_handshake(&mut self) -> io::Result<bool> {
        self.engine.try_finish_handshake()
    }

    /// Whether the app has connected (the handshake is done and its socket is this engine's).
    pub fn connected(&self) -> bool {
        !self.engine.handshake_pending() && self.engine.fd().is_some()
    }

    /// The engine, for what this module does not wrap (housekeeping, the idle moments, a test's reads).
    pub fn engine(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// Reads what the app has sent. `true` when the app has closed its end.
    pub fn pump(&mut self) -> io::Result<bool> {
        Ok(self.engine.pump()?.closed)
    }

    /// The app's socket, for the compositor's poll set: a commit wakes the loop instead of waiting for the tick.
    pub fn fd(&self) -> Option<std::os::fd::RawFd> {
        self.engine.fd()
    }

    /// Whether a frame is due at `now_ns`: commits wait, work between frames changed the picture, an animation runs, or a
    /// wallpaper's step is due.
    pub fn wants_frame(&self, now_ns: u64) -> bool {
        self.engine.has_commits()
            || self.engine.redraw_wanted()
            || match self.engine.demand() {
                Demand::EveryRefresh => true,
                Demand::At(at) => now_ns >= at,
                Demand::Idle => false,
            }
    }

    /// Whether an animation of the engine runs, so that the frame after this one is due at the next refresh (the demand at every refresh).
    pub fn animating(&self) -> bool {
        matches!(self.engine.demand(), Demand::EveryRefresh)
    }

    /// A tick with no frame due: the commits that show nothing by themselves (a glyph atlas made, chunks of it) are applied, and
    /// the upload work they leave is done now (`tidy_more` only says what the last tidy left over), then a round of housekeeping
    /// while there is any left. What they change asks for a frame through [`Ui::wants_frame`]. Nothing enters the context when
    /// there is neither.
    pub fn idle(&mut self, now_ns: u64) {
        let had = self.engine.has_waiting();
        if had {
            self.engine.apply_waiting(now_ns);
        }
        if had || self.engine.tidy_more() {
            self.engine.tidy();
        }
    }

    /// When the engine next wants a frame on its own (a wallpaper's step), if no animation runs.
    pub fn due_at(&self) -> Option<u64> {
        match self.engine.demand() {
            Demand::At(at) => Some(at),
            Demand::EveryRefresh | Demand::Idle => None,
        }
    }

    /// The first half of a frame: takes in what the app committed, steps the scene to `now_ns`, and adds what each stack changes
    /// from the frame before to `damage`, the tiles the frame changes (which the compositor then takes the page's account of). With
    /// `home_hidden` the frame does not draw stack 0 (Home under an app's card), so what it changed owes nothing.
    pub fn plan(&mut self, now_ns: u64, damage: &mut Tiles, home_hidden: bool) -> io::Result<()> {
        self.engine.plan(now_ns)?;
        for stack in usize::from(home_hidden)..2 {
            if let Some(changed) = self.engine.changed(stack) {
                add_tiles(damage, changed, &mut self.scratch);
            }
        }
        Ok(())
    }

    /// Whether the planned frame shows anything of stack 1 (a layer that is visible, over the windows). Nothing of stack 0 shows
    /// under a full-screen opaque window, so this alone says whether that window can go to the panel directly.
    pub fn over_windows(&self) -> bool {
        self.engine.has_layers(1)
    }

    /// `owed`, the rectangles a page owes, as the planned frame is cheapest drawn in ([`Engine::merge_areas`]): a pass draws every
    /// layer that meets its rectangle, so a dozen bands of a list that scrolls are cheaper as one cover. The covers hold all that the
    /// rectangles did and do not overlap, so the windows and both stacks are drawn in them as they would be in the rectangles.
    pub fn merge(&mut self, owed: &mut Vec<Rect>) {
        self.scratch.clear();
        self.scratch.extend(owed.iter().map(|r| [r.x, r.y, r.right(), r.bottom()]));
        self.engine.merge_areas(&mut self.scratch);
        owed.clear();
        owed.extend(self.scratch.iter().map(|a| Rect::new(a[0], a[1], a[2] - a[0], a[3] - a[1])));
    }

    /// Clears the owed rectangles and draws the layer stack under application windows.
    pub fn draw_under(&mut self, fbo: u32, owed: &[Rect]) -> io::Result<()> {
        self.areas.clear();
        self.areas.extend(owed.iter().map(|r| [r.x, r.y, r.right(), r.bottom()]));
        self.engine.draw_stack(0, fbo, &self.areas)
    }

    /// Draws system layers over application windows, before the pointer.
    pub fn draw_over(&mut self, fbo: u32, owed: &[Rect]) -> io::Result<()> {
        if !self.engine.has_layers(1) {
            return Ok(());
        }
        self.areas.clear();
        self.areas.extend(owed.iter().map(|r| [r.x, r.y, r.right(), r.bottom()]));
        self.engine.draw_stack(1, fbo, &self.areas)
    }

    /// Completes drawing without acknowledging presentation; a queued frame may never reach the panel.
    pub fn finish(&mut self) -> io::Result<FrameCost> {
        self.engine.finish()
    }

    pub fn presented(&mut self, presented_ns: u64, cost: &FrameCost) {
        self.engine.frame_done(presented_ns, cost);
    }

    /// Housekeeping between frames (textures to free, pictures to upload ahead).
    pub fn tidy(&mut self) {
        self.engine.tidy();
    }

    /// The pages may not hold what the account says (a lost frame, a resize): the next frame changes everything.
    pub fn forget(&mut self) {
        self.engine.forget();
    }
}
