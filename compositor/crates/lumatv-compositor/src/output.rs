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

    /// This picture at `w` x `h` (every output pixel the average of the source pixels it covers), opaque. The compositor's pixels are
    /// premultiplied, so dropping the alpha is compositing over black: a hole left for video to show through comes out black, never
    /// transparent. `into` is cleared and filled (its capacity kept), so a recording draws every picture into the same vectors.
    pub fn scaled(&self, w: u32, h: u32, mut into: Vec<u8>) -> Image {
        into.clear();
        into.reserve(w as usize * h as usize * 4);
        let (sw, sh) = (self.width as usize, self.height as usize);
        let (w, h) = (w as usize, h as usize);
        for oy in 0..h {
            let (y0, y1) = (oy * sh / h, ((oy + 1) * sh).div_ceil(h).clamp(oy * sh / h + 1, sh));
            for ox in 0..w {
                let (x0, x1) = (ox * sw / w, ((ox + 1) * sw).div_ceil(w).clamp(ox * sw / w + 1, sw));
                let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
                for y in y0..y1 {
                    for px in self.rgba[(y * sw + x0) * 4..(y * sw + x1) * 4].chunks_exact(4) {
                        r += px[0] as u32;
                        g += px[1] as u32;
                        b += px[2] as u32;
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)) as u32;
                into.extend_from_slice(&[((r + n / 2) / n) as u8, ((g + n / 2) / n) as u8, ((b + n / 2) / n) as u8, 255]);
            }
        }
        Image { width: w as u32, height: h as u32, rgba: into }
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
    /// Pictures in a transition left out of this frame because the driver was still busy with their buffer (the transition's clock starts
    /// again at the next frame: `Transitions::drawn`).
    pub posed_waiting: u32,
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

    /// The app switcher is opening: the output makes what drawing these buffers scaled needs, now and not in the frame that first shows
    /// them. The default does nothing.
    fn prepare_previews(&mut self, _buffers: &[WlBuffer]) {}

    /// Whether the driver is still busy with a buffer `prepare_buffer` started on (its warm draw's fence has not
    /// signalled): a frame drawn now would wait on it, so the tick keeps the previous frame on the screen instead
    /// and tries again next time. The default is never.
    fn warming(&mut self) -> bool {
        false
    }

    /// Whether the frees of what clients destroyed may wait: a transition is moving, and a free of a driver import stalls the loop for
    /// milliseconds (`Gl::bury`), a frame of the move late. They are done once it has ended. The default has nothing to free.
    fn defer_frees(&mut self, _on: bool) {}

    /// How many retired textures wait to be freed ([`Output::defer_frees`]). The default holds none.
    fn retired(&self) -> usize {
        0
    }

    /// Forgets anything cached for a buffer the client destroyed. A released buffer keeps its import: clients
    /// re-attach the same buffers every few frames.
    fn forget_buffer(&mut self, buffer: &WlBuffer);

    /// Whether the display reads `buffer`, or will once a frame already drawn is committed (direct scan-out): the client may not draw into it,
    /// so it is not released to it, until a later frame has replaced it on the screen. The default holds nothing: a frame that was composed has
    /// copied what it needed.
    fn holds(&self, _buffer: &WlBuffer) -> bool {
        false
    }

    /// The direct scan-out's counters and state, for `ctl status` and `ctl scanout`; `None` for an output without one.
    fn scanout_report(&self) -> Option<serde_json::Value> {
        None
    }

    /// Switches direct scan-out (`ctl scanout`); an output that has none says so.
    fn set_scanout(&mut self, _mode: crate::scanout::Mode) -> io::Result<()> {
        Err(io::Error::other("this output has no direct scan-out"))
    }

    /// Journal lines the output has to tell (direct scan-out's changes of state), taken once.
    fn take_journal(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Takes in system UI requests, including changes that do not damage any application window.
    fn ui_needs_frame(&mut self) -> io::Result<bool> {
        Ok(false)
    }

    /// The system UI app's socket, for the main loop to poll: readable means requests wait (`State::ui_wake`). `None`
    /// without a UI.
    fn ui_fd(&self) -> Option<RawFd> {
        None
    }

    /// Makes the UI engine in this output's context, for a client in another process (`shell_rust.rs`): the client's end of the
    /// engine's socket, to hand that process at its start. The handshake completes later ([`Output::ui_handshake`]). An output
    /// without the engine, or with a client already attached, says so.
    fn ui_attach(&mut self, _driver: &str, _system_z: i32, _shm: std::path::PathBuf) -> io::Result<std::os::unix::net::UnixStream> {
        Err(io::Error::other("this output has no UI engine"))
    }

    /// Finishes the engine's handshake once the client has said hello: `true` the first time it is complete (the socket is then
    /// [`Output::ui_fd`]), `false` while the client has not connected yet or there is nothing to finish.
    fn ui_handshake(&mut self) -> io::Result<bool> {
        Ok(false)
    }

    /// Drops the engine and the client's layers with it (the client is gone or is being restarted); the next frame draws
    /// everything without them.
    fn ui_detach(&mut self) {}

    /// Whether an engine is attached (a client's, connected or awaited).
    fn ui_attached(&self) -> bool {
        false
    }

    /// Draws the plan into the output's target.
    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats>;

    /// Puts the last rendered frame on the output. Offscreen: a timestamp. KMS (C3): the atomic commit, with
    /// the in-fence and the presented-time feedback the pacer needs.
    fn present(&mut self) -> io::Result<Presented>;

    /// Reads the target back.
    fn capture(&mut self) -> io::Result<Image>;

    /// The picture the screen shows, at `size`, for a screenshot or a recording; `spare` is a vector to draw it into. Only what this
    /// compositor composed is in it: video is on a plane of its own that no page ever holds, and the holes left for it read black.
    /// The default reads the target back and scales it on the CPU; KMS does both on the GPU from the page that holds the last frame.
    fn grab(&mut self, size: (u32, u32), spare: Vec<u8>) -> io::Result<Image> {
        Ok(self.capture()?.scaled(size.0, size.1, spare))
    }

    /// While on, every frame is composed into a page (no window goes to the plane directly), so that `grab` finds the screen in the last
    /// page. Offscreen outputs always compose; the default does nothing.
    fn set_compose_only(&mut self, _on: bool) {}

    /// A screenshot or recording is over: what `grab` kept for it (a scratch target) is given back. The default has nothing to give.
    fn grab_end(&mut self) {}

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

    /// The frame just drawn was an OLED-care beat alone (`Scene::care_frame`): a still screen's one-pixel step or dim, owed nothing after it. The
    /// output may call the screen at rest once that frame has flipped, rather than a second after it, so the tick stays at its slowed rate.
    fn quiet_frame(&mut self) {}

    /// Begins a sample of the app's pictures in `plan` for the box dim (`boxdim`): each cell of its grid's numbers (`boxdim::Sig`), composed
    /// at the grid's resolution with the plan's holes black, each app item where its window places it (`Item::drift` taken away). The
    /// work is started, not waited for: [`Output::sample_take`] reads it once done. `Ok(false)`: not now (one is in flight, a buffer is
    /// still warming); an error: this output cannot sample, and the box dim takes damage for change. The default cannot.
    fn sample_begin(&mut self, _plan: &Plan, _surfaces: &Surfaces) -> io::Result<bool> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    /// The sample begun last, once it is done: into `sigs` (row 0 the top), `Ok(true)`. `Ok(false)` while it is not done (or none was
    /// begun); an error when it never will be (it is dropped). The default has none.
    fn sample_take(&mut self, _sigs: &mut [crate::boxdim::Sig; crate::boxdim::CELLS]) -> io::Result<bool> {
        Ok(false)
    }

    /// Makes a piece of what sampling and the box dim's map need ahead of their first use (a shader's compile costs the loop
    /// milliseconds on the set), one piece a call, on ticks with nothing else to do: `Ok(true)` once all is made. The default has nothing
    /// to make.
    fn boxdim_warm(&mut self) -> io::Result<bool> {
        Ok(true)
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

    /// Runs the output at `hz` (`refresh`, 60 or 120): KMS writes the OSD's vblank divider and keys its pacer to the period; an
    /// output without a panel only takes the number. Answers what is in force, or why the change was refused (nothing changed then).
    fn set_refresh(&mut self, hz: u32) -> io::Result<u32> {
        Ok(hz)
    }

    /// The refresh mode's measure since the last switch (`refresh::Measure::report`), for `ctl refresh`; `None` without flips.
    fn refresh_report(&self) -> Option<serde_json::Value> {
        None
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
