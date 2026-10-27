//! A frame drawn while the driver is still busy with a surface's new buffer (its warm draw's fence has not signalled):
//! with a page holding the last composed frame, the KMS frame logic copies the surface's rectangle from that page instead
//! of holding the frame, the client's callback comes with the frame, and the new content follows in a frame of its own
//! once the buffer is warm. Mesa's software GL over pages in the context's own memory, under the real `State::tick`.

mod harness;

use harness::{Client, Server};
use lumatv_compositor::kms::frames::Frames;
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl, TargetId};
use lumatv_compositor::scene::Plan;
use lumatv_compositor::surface::Surfaces;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use wayland_server::protocol::wl_buffer::WlBuffer;

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";

/// The KMS output without the device: the GL renderer, three pages of its own and the ring's frame logic. `hold_next`
/// makes the next buffer a commit prepares count as one the driver is busy with, until `release`.
struct ScanOut {
    gl: Gl,
    frames: Frames,
    pages: Vec<TargetId>,
    hold_next: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    held: Option<WlBuffer>,
}

impl Output for ScanOut {
    fn name(&self) -> &'static str {
        "standin-test"
    }

    fn size(&self) -> (u32, u32) {
        self.gl.size()
    }

    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        self.gl.query_buffer(buffer)
    }

    fn prepare_buffer(&mut self, buffer: &WlBuffer) -> bool {
        let new = self.gl.prepare(buffer);
        if new && self.hold_next.swap(false, Ordering::SeqCst) {
            self.gl.hold_warm(buffer, true);
            self.held = Some(buffer.clone());
        }
        new
    }

    fn warming(&mut self) -> bool {
        if let Some(b) = self.held.take() {
            if self.release.load(Ordering::SeqCst) {
                self.gl.hold_warm(&b, false);
            } else {
                self.held = Some(b);
            }
        }
        self.gl.warming()
    }

    fn forget_buffer(&mut self, buffer: &WlBuffer) {
        self.gl.forget_buffer(buffer)
    }

    fn ui_needs_frame(&mut self) -> io::Result<bool> {
        Ok(self.gl.ui_needs_frame()? || self.gl.standin_due())
    }

    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats> {
        let (_, stats) = self.frames.draw(&mut self.gl, plan, surfaces, None)?;
        Ok(stats)
    }

    fn present(&mut self) -> io::Result<Presented> {
        self.gl.present()
    }

    /// The page last composed: what the panel would show.
    fn capture(&mut self) -> io::Result<Image> {
        self.gl.set_target(self.pages[self.frames.last_page()]);
        self.gl.capture()
    }
}

#[test]
fn a_warming_buffer_is_stood_in_for_by_the_last_frame_and_drawn_once_warm() {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return;
    }
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let hold_next = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let (h, r) = (hold_next.clone(), release.clone());
    let mut server = Server::start_with(Box::new(move || {
        let mut gl = Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap();
        let pages: Vec<TargetId> = (0..3).map(|_| gl.add_target().unwrap()).collect();
        let frames = Frames::new(pages.clone(), 1920, 1080);
        Box::new(ScanOut { gl, frames, pages, hold_next: h, release: r, held: None })
    }));
    server.ticking.store(false, Ordering::SeqCst);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Standin");
    let blue = client.shm_buffer(300, 200, 0xff19_80e6);
    client.present(&card.surface, &blue);
    client.roundtrip();
    let image = server.eval(harness::drawn);
    assert_eq!(image.pixel(100, 100), [25, 128, 230, 255], "the first frame shows the first buffer");
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));

    // The second buffer: the driver stays busy with it. The frame is not held: it goes out with the first buffer's
    // content where the window is, and the client's callback with it.
    hold_next.store(true, Ordering::SeqCst);
    let orange = client.shm_buffer(300, 200, 0xffe6_8019);
    client.present(&card.surface, &orange);
    client.roundtrip();
    let (drawn, held, image) = server.eval(|s| {
        s.tick();
        (s.stats.frames_drawn, s.stats.held_ticks, s.output.capture().unwrap())
    });
    assert_eq!((drawn, held), (2, 0), "the frame was drawn, not held");
    assert_eq!(image.pixel(100, 100), [25, 128, 230, 255], "the window keeps the last frame's content while its buffer warms");
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 2), "the client's callback came with the frame");

    // Nothing to draw while the buffer still warms and no commit comes.
    let drawn = server.eval(|s| {
        s.tick();
        s.stats.frames_drawn
    });
    assert_eq!(drawn, 2);

    // Warm now: a frame of its own shows the new content, and nothing is owed after it.
    release.store(true, Ordering::SeqCst);
    let (drawn, image) = server.eval(|s| {
        s.tick();
        (s.stats.frames_drawn, s.output.capture().unwrap())
    });
    assert_eq!(drawn, 3, "the owed frame was drawn once the buffer was warm");
    assert_eq!(image.pixel(100, 100), [230, 128, 25, 255], "the new buffer's content is on the page");
    let (drawn, errors) = server.eval(|s| {
        s.tick();
        (s.stats.frames_drawn, s.stats.gl_errors)
    });
    assert_eq!((drawn, errors), (3, 0), "nothing more is owed, and no GL error on the way");
}
