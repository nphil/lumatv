//! Where frames go. C2 has one output, offscreen (LG's compositor holds the DRM master); C3 adds the KMS one.
//! The trait is the seam: the scene and the server never learn which they are talking to.

use std::io;
use std::time::Instant;

use wayland_server::protocol::wl_buffer::WlBuffer;

use crate::scene::Plan;
use crate::surface::Surfaces;

/// A frame the output took. `at_ms` is what `wl_callback.done` carries (CLOCK_MONOTONIC in ms, as libwayland
/// clients expect); `seq` counts presented frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Presented {
    pub seq: u64,
    pub at_ms: u32,
}

/// An RGBA8 picture, rows top to bottom, premultiplied alpha as the compositor holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    pub items: usize,
    pub drawn: usize,
    pub skipped: usize,
    pub render_us: u64,
}

pub trait Output {
    fn name(&self) -> &'static str;

    fn size(&self) -> (u32, u32);

    /// The size of a buffer the output can read but the protocol layer cannot (the GL driver's own buffers),
    /// and whether its rows are stored bottom-up. `None` when the output cannot tell.
    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)>;

    /// Forgets anything cached for a buffer that was released or destroyed.
    fn forget_buffer(&mut self, buffer: &WlBuffer);

    /// Draws the plan into the output's target.
    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats>;

    /// Puts the last rendered frame on the output. Offscreen: a timestamp. KMS (C3): the atomic commit, with
    /// the in-fence and the presented-time feedback the pacer needs.
    fn present(&mut self) -> io::Result<Presented>;

    /// Reads the target back.
    fn capture(&mut self) -> io::Result<Image>;
}

/// Monotonic milliseconds since the first call, 32-bit wrapping, for frame callbacks.
pub struct FrameClock {
    start: Instant,
    seq: u64,
}

impl Default for FrameClock {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameClock {
    pub fn new() -> Self {
        FrameClock { start: Instant::now(), seq: 0 }
    }

    pub fn stamp(&mut self) -> Presented {
        self.seq += 1;
        Presented { seq: self.seq, at_ms: monotonic_ms() }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

/// CLOCK_MONOTONIC in milliseconds, truncated to 32 bits like libwayland's own frame callbacks.
pub fn monotonic_ms() -> u32 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: plain clock read into a stack struct.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000) as u32
}
