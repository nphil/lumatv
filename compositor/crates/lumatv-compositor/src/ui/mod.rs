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
//! only inside the tiles the page owes, which the ring below says:
//!
//! ```text
//! let mut tiles = <the windows' damage this frame>;          // Plan::tiles
//! ui.plan(now_ns, &mut tiles)?;                              // adds what the two stacks changed
//! let owed = ring.frame(page, &tiles);                       // damage::Ring, as for the windows alone
//! gl.bind(page) ...
//! ui.draw_under(fbo, owed)?;                                 // clears the owed tiles and draws stack 0 into them
//! <the windows, inside the owed tiles>
//! ui.draw_over(fbo, owed)?;                                  // stack 1 over the windows, under the pointer
//! ui.finish(presented_ns)?;                                  // the app is told the frame was shown
//! ```
//!
//! `Gl::render_rects` clears the owed tiles itself today, which would wipe stack 0; the two calls go where it clears (`draw_under`
//! takes the clear over) and after its items, before the cursor (`draw_over`). The context is the compositor's: every call saves its
//! GL state, makes the context the toolkit's and puts the state back ([`lumatv_render::Gles::enter`]), so `Gl` need not know. The
//! toolkit's shaders are GLSL ES 3.00: the compositor's context has to be OpenGL ES 3.0 or later (`render/egl.rs` asks for 2, which
//! Mesa answers with 3.2 and the Mali blob answers [INFERENCE: not seen on the set] with 3.x too; [`Ui::new`] refuses an ES 2
//! context and says which).

pub mod demo;

use crate::Rect;
use crate::damage::Tiles;
use lumatv_render::host::{Engine, FrameCost, HostConfig};
use lumatv_render::tiles::TileSet;
use lumatv_render::{Demand, Gles};
use std::ffi::CString;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// The engine, and the scratch the compositor's tiles are turned into the engine's areas with.
pub struct Ui {
    engine: Engine,
    rects: Vec<Rect>,
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

/// The rectangles of `tiles` as the engine's areas, at most [`crate::damage::MAX_RECTS`] of them (more collapse to their bounds).
pub fn areas_of(tiles: &Tiles, rects: &mut Vec<Rect>, areas: &mut Vec<[i32; 4]>) {
    tiles.rects(rects);
    areas.clear();
    areas.extend(rects.iter().map(|r| [r.x, r.y, r.right(), r.bottom()]));
}

impl Ui {
    /// The engine, in the context that is current on this thread (the compositor's). `driver` is the EGL/GLES library the context
    /// is the driver's (`libmali-...so` on the set, Mesa's `libEGL.so.1` on a host). `system_z` is where stack 1 begins. The app's
    /// end of the socket is returned, and [`Ui::finish_handshake`] is called once the app has connected over it.
    ///
    /// # Safety
    /// A context of `driver` is current on this thread and stays so for as long as the `Ui` is used; every call of it is made on
    /// this thread.
    pub unsafe fn new(driver: &str, width: u32, height: u32, system_z: i32, shm_dir: PathBuf) -> io::Result<(Self, UnixStream)> {
        let driver = CString::new(driver).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "driver path"))?;
        // SAFETY: the caller's.
        let gles = unsafe { Gles::current(&driver) }?;
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
        Ok((Self { engine, rects: Vec::new(), scratch: Vec::new(), areas: Vec::new() }, app))
    }

    pub fn finish_handshake(&mut self) -> io::Result<()> {
        self.engine.finish_handshake()
    }

    /// The engine, for what this module does not wrap (housekeeping, the idle moments, a test's reads).
    pub fn engine(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// Reads what the app has sent. `true` when the app has closed its end.
    pub fn pump(&mut self) -> io::Result<bool> {
        Ok(self.engine.pump()?.closed)
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

    /// When the engine next wants a frame on its own (a wallpaper's step), if no animation runs.
    pub fn due_at(&self) -> Option<u64> {
        match self.engine.demand() {
            Demand::At(at) => Some(at),
            Demand::EveryRefresh | Demand::Idle => None,
        }
    }

    /// The first half of a frame: takes in what the app committed, steps the scene to `now_ns`, and adds what each stack changes
    /// from the frame before to `damage`, the tiles the frame changes (which the compositor then takes the page's account of).
    pub fn plan(&mut self, now_ns: u64, damage: &mut Tiles) -> io::Result<()> {
        self.engine.plan(now_ns)?;
        for stack in 0..2 {
            if let Some(changed) = self.engine.changed(stack) {
                add_tiles(damage, changed, &mut self.scratch);
            }
        }
        Ok(())
    }

    /// Clears the tiles of `owed` in the framebuffer `fbo` (a page) and draws stack 0 into them.
    pub fn draw_under(&mut self, fbo: u32, owed: &Tiles) -> io::Result<()> {
        areas_of(owed, &mut self.rects, &mut self.areas);
        self.engine.draw_stack(0, fbo, &self.areas)
    }

    /// Draws stack 1 into the tiles of `owed`, over what the compositor has drawn there (when it has any layer: otherwise there is
    /// nothing to draw and nothing is touched).
    pub fn draw_over(&mut self, fbo: u32, owed: &Tiles) -> io::Result<()> {
        if !self.engine.has_layers(1) {
            return Ok(());
        }
        areas_of(owed, &mut self.rects, &mut self.areas);
        self.engine.draw_stack(1, fbo, &self.areas)
    }

    /// The second half of a frame, once the page is drawn: what it showed is kept, and the app is told the frame was shown at
    /// `presented_ns` (`CLOCK_MONOTONIC`), which retires what its commits replaced.
    pub fn finish(&mut self, presented_ns: u64) -> io::Result<FrameCost> {
        let cost = self.engine.finish()?;
        self.engine.frame_done(presented_ns, &cost);
        Ok(cost)
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
