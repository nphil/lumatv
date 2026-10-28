//! Where frames go. C2 has one output, offscreen (LG's compositor holds the DRM master); C3 adds the KMS one.
//! The trait is the seam: the scene and the server never learn which they are talking to.

use std::io;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use wayland_server::protocol::wl_buffer::WlBuffer;

use crate::scene::Plan;
use crate::surface::Surfaces;

/// A frame the output took. `at_ms` is what `wl_callback.done` carries (CLOCK_MONOTONIC in ms, as libwayland
/// clients expect); `seq` counts presented frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Presented {
    pub seq: u64,
    pub at_ms: u32,
    /// The frame is on its way to the screen and lands later ([`Output::dispatch_events`] counts it): its frame
    /// callbacks wait for that, so clients pace to the refresh and not to the compositor's moving latch.
    pub pending: bool,
    /// The frame took the place of one that was waiting behind the flip in flight and never went out; the two
    /// land as one.
    pub replaced: bool,
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
    /// GL errors seen so far (cumulative); zero for the software renderer.
    pub gl_errors: u64,
    /// Buffers this frame imported for the first time (none when the commit prepared them).
    pub imports: u32,
    /// Surfaces drawn from the last frame's page because the driver was still busy with their new buffer.
    pub stood_in: u32,
}

pub trait Output {
    fn name(&self) -> &'static str;

    fn size(&self) -> (u32, u32);

    /// The size of a buffer the output can read but the protocol layer cannot (the GL driver's own buffers),
    /// and whether its rows are stored bottom-up. `None` when the output cannot tell.
    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)>;

    /// A buffer a client just committed for the first time: the output pays its first-use costs now (import, first
    /// draw) instead of in the frame that first shows it. `true` when it did something. The default does nothing.
    fn prepare_buffer(&mut self, _buffer: &WlBuffer) -> bool {
        false
    }

    /// Whether the driver is still busy with a buffer `prepare_buffer` started on (its warm draw's fence has not
    /// signalled): a frame drawn now would wait on it, so the tick keeps the previous frame on the screen instead
    /// and tries again next time. The default is never.
    fn warming(&mut self) -> bool {
        false
    }

    /// Forgets anything cached for a buffer the client destroyed. A released buffer keeps its import: clients
    /// re-attach the same buffers every few frames.
    fn forget_buffer(&mut self, buffer: &WlBuffer);

    /// Takes in system UI requests, including changes that do not damage any application window.
    fn ui_needs_frame(&mut self) -> io::Result<bool> {
        Ok(false)
    }

    /// The system UI app's socket, for the main loop to poll: readable means requests wait (`State::ui_wake`). `None`
    /// without a UI.
    fn ui_fd(&self) -> Option<RawFd> {
        None
    }

    /// Draws the plan into the output's target.
    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats>;

    /// Puts the last rendered frame on the output. Offscreen: a timestamp. KMS (C3): the atomic commit, with
    /// the in-fence and the presented-time feedback the pacer needs.
    fn present(&mut self) -> io::Result<Presented>;

    /// Reads the target back.
    fn capture(&mut self) -> io::Result<Image>;

    /// When the next frame should start, for outputs that pace themselves (KMS: the pacer's latch, from the
    /// page-flip timestamps) while frames are following one another. `None` means the fixed tick.
    fn next_frame_at(&self, _now: Instant) -> Option<Instant> {
        None
    }

    /// When the fixed tick should next fire while no run of frames is on, for an output that knows the panel's
    /// refresh grid (KMS: the last flip's stamp and whole refreshes since): a frame drawn by a tick that fires a few
    /// milliseconds before a refresh waits a whole one. `None` means one interval from the last tick.
    fn idle_tick_at(&self, _now: Instant) -> Option<Instant> {
        None
    }

    /// Whether the screen is at rest: nothing in flight, owed or drawn in the last second, so a tick finds only housekeeping and the
    /// loop may slow it (`ticker`). The default is no: an output that draws at the tick (the offscreen ones) needs it at its rate.
    fn idle_ok(&self) -> bool {
        false
    }

    /// `idle_tick_at` for a tick no sooner than `wait` from now (the slowed tick), still on the panel's grid. `None` as there.
    fn idle_tick_after(&self, _now: Instant, _wait: Duration) -> Option<Instant> {
        None
    }

    /// A file descriptor the main loop polls for the output's own events (KMS: the DRM device, readable when a
    /// page flip has landed). `None` for outputs without events.
    fn event_fd(&self) -> Option<RawFd> {
        None
    }

    /// Handles the events pending on `event_fd`; returns the number of frames that landed.
    fn dispatch_events(&mut self) -> io::Result<u32> {
        Ok(0)
    }

    /// When the last frame reached the panel (KMS: the stamp of the last page flip, CLOCK_MONOTONIC ns), for the loop
    /// to read its own distance from it. `None` for outputs without flips.
    fn last_flip_ns(&self) -> Option<u64> {
        None
    }

    /// Whether a commit that arrived now should be drawn at once rather than at the next tick: the output shows a
    /// still frame (no flip in flight, no run of frames to keep in step with), so waiting for the tick would only
    /// add latency. The offscreen outputs say no: their frames are driven by the tick alone, which is what the
    /// host tests and the X2 runs count on.
    fn draws_on_damage(&self, _now: Instant) -> bool {
        false
    }

    /// Output-specific counters for the summary line (KMS: flips, late and lost refreshes, direct scan-outs).
    fn stats_line(&self) -> String {
        String::new()
    }
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
        Presented { seq: self.seq, at_ms: monotonic_ms(), ..Default::default() }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

/// CLOCK_MONOTONIC in milliseconds, truncated to 32 bits like libwayland's own frame callbacks.
pub fn monotonic_ms() -> u32 {
    (monotonic_ns() / 1_000_000) as u32
}

/// CLOCK_MONOTONIC in ns: the clock the kernel stamps page flips with and the log's lines carry.
pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: plain clock read into a stack struct.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}
