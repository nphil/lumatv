//! What the renderer gives back when the compositor has had nothing to draw for a while ([`crate::idle`] says when): the GPU memory
//! that only makes the next frame cheaper. The resident size of the compositor counts one byte for each byte it holds on the GPU,
//! so a screen that stands still for an hour should not hold what a burst of frames asked for.
//!
//! **Let go of**, and what making each again costs the first frame that wants it:
//! * the graveyard (textures of layers that are gone): nothing, they were to be freed anyway (`Renderer::tidy`).
//! * the pool's free render targets, and the live backdrop pictures of the last frame (a live picture is made again by every frame
//!   that draws the glass): one `glTexImage2D` and one framebuffer for each, about 0.2 ms at the Show's slowest clock, in the
//!   first frame that draws glass. They are the small pictures of the blur (a panel's 1/4 to 1/16), not the panel's size.
//! * the cached backdrops that nothing has drawn from, or shown glass from plates over, for [`UNUSED_FRAMES`] frames: the glass
//!   they were made for is not on the screen (a page that is shut). The blur chain, a few passes, in the frame that shows that
//!   glass again.
//! * the plates of still glass that no surface has stood in for [`UNUSED_FRAMES`] frames (the existing rule lets go after
//!   [`super::PLATE_KEEP_FRAMES`], but it counts frames and an idle screen draws none): a panel of the GPU's memory each (1.8 MB at
//!   the Show). What stood in one is drawn as glass always was until it has stood again (three frames) and is then drawn into a
//!   plate once, which is how a plate is made at all.
//!
//! **Kept**: the texture of every layer (the picture it shows, and what it shows is not rebuilt without the app), the atlas
//! textures (the glyphs the shown layers draw from), the plates and pictures of prepared backdrops (what the glass over them
//! shows), the cached backdrops of glass that is shown, the plates glass is shown from, and the scan-out buffers.
//!
//! Only while no frame is on the GPU: the engine calls it between frames, from its idle moment.

use super::Renderer;
use crate::census::Held;
use crate::ops::Op;
use std::fmt;

/// Frames in a row without a read (`Backdrops::release_idle`) or a surface standing in it (a plate) after which an idle compositor
/// lets go of a cached backdrop or a plate of still glass. Counted in frames drawn, so a screen that drew a few frames after its glass
/// went (a clock's tick) does not lose what the glass will want again, and a page that was shut a second of drawing ago does.
pub const UNUSED_FRAMES: u64 = 60;

/// What [`Renderer::trim`] let go of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Trimmed {
    /// Textures of gone layers freed (their sizes are not known).
    pub graveyard: u32,
    /// Render targets of the pool, free or live.
    pub pool: Held,
    /// Cached backdrops of glass that is not shown.
    pub backdrops: Held,
    /// Plates of still glass nothing stands in.
    pub plates: Held,
}

impl Trimmed {
    /// The bytes of GPU memory let go of (the graveyard's are not counted).
    pub fn bytes(&self) -> u64 {
        self.pool.bytes + self.backdrops.bytes + self.plates.bytes
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl fmt::Display for Trimmed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "pool {}, backdrops {}, still plates {}, graveyard {}",
            self.pool, self.backdrops, self.plates, self.graveyard
        )
    }
}

impl Renderer {
    /// Gives back the memory that only makes the next frame cheaper (see the module's account). Call it with no frame on the GPU
    /// and nothing waiting to be drawn; a frame drawn after it is the frame drawn without it, and a little slower once.
    pub fn trim(&mut self) -> Trimmed {
        let mut done = Trimmed {
            graveyard: u32::try_from(self.graveyard.len()).unwrap_or(u32::MAX),
            ..Trimmed::default()
        };
        for name in self.graveyard.drain(..) {
            self.gl.delete_texture(name);
        }
        let (pool, backdrops) = self.backdrops.release_idle(&self.gl, UNUSED_FRAMES);
        done.pool = pool;
        done.backdrops = backdrops;
        for p in 0..self.plates.len() {
            if self.frame.saturating_sub(self.plates_used[p]) < UNUSED_FRAMES {
                continue;
            }
            if let Some(plate) = self.plates[p].take() {
                done.plates.texture(plate.width, plate.height, 4);
                self.gl.delete_render_target(plate);
                self.still.drop_plate(p);
            }
        }
        if !done.is_empty() {
            // The driver gives a deleted texture's pages back once the commands that name it have been handed on.
            self.gl.flush();
        }
        done
    }

    /// Gives back everything the renderer holds on the GPU but its programs, for a renderer whose client has gone in a context that lives
    /// on (the compositor's): the client's layers, glyph atlases, meshes and prepared sets as [`Op::Clear`] lets go of them, the graveyard,
    /// and what [`Renderer::trim`] keeps, every cached backdrop and every plate. A context keeps the names a dropped renderer made: on the
    /// set each client restart left the compositor 63 MB of GPU memory larger (2026-11-05, five full-screen targets among it). The
    /// programs go back to the cache when the renderer is dropped.
    pub fn release(&mut self) {
        self.apply(&Op::Clear);
        for name in self.graveyard.drain(..) {
            self.gl.delete_texture(name);
        }
        let _ = self.backdrops.release_idle(&self.gl, 0);
        self.set_plates(false);
        self.gl.flush();
    }
}
