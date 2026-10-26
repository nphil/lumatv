//! A buffer's first-use costs are paid at its commit, not in the frame that first draws it (Mesa's software GL under the
//! real `State`): the clients' pass imports a new buffer (`prepared` counts it before any frame exists), the frame that
//! then draws it imports nothing (`frame imports` stays 0) and shows it right. A buffer the client destroys is retired
//! by the frame that drops it and freed by a later one, with no GL error either way. While the output says the driver
//! is still busy with a prepared buffer (`warming`), the tick draws nothing and the client's frame waits with it.

mod harness;

use harness::{Client, Server};
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl};
use lumatv_compositor::render::software::Software;
use lumatv_compositor::scene::Plan;
use lumatv_compositor::surface::Surfaces;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use wayland_server::Resource;
use wayland_server::backend::ObjectId;
use wayland_server::protocol::wl_buffer::WlBuffer;

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";

#[test]
fn commit_imports_before_the_frame_and_a_destroyed_buffer_is_freed_after_one() {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return;
    }
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let mut server = Server::start_with(Box::new(|| Box::new(Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap())));
    server.ticking.store(false, Ordering::SeqCst);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Warm");
    let first = client.shm_buffer(300, 200, 0xff19_80e6);
    client.present(&card.surface, &first);
    client.roundtrip();
    let (prepared, drawn, errors) = server.eval(|s| (s.stats.prepared, s.stats.frames_drawn, s.stats.gl_errors));
    assert_eq!((prepared, drawn), (1, 0), "the commit imported the buffer before any frame was drawn");
    assert_eq!(errors, 0, "the warm-up draw is clean");

    server.ticking.store(true, Ordering::SeqCst);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    let (imports, image) = server.eval(|s| (s.stats.frame_imports, s.output.capture().unwrap()));
    assert_eq!(imports, 0, "the frame found its texture ready");
    assert_eq!(image.pixel(100, 100), [25, 128, 230, 255], "the shm content is uploaded by the frame, as before");

    let second = client.shm_buffer(300, 200, 0xffe6_8019);
    client.present(&card.surface, &second);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 2));
    first.destroy();
    client.roundtrip();
    // Two more frames: the first retires the dead texture, the next frees it.
    for n in 3..5 {
        client.present(&card.surface, &second);
        assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= n));
    }
    let (prepared, imports, errors, image) = server.eval(|s| (s.stats.prepared, s.stats.frame_imports, s.stats.gl_errors, s.output.capture().unwrap()));
    assert_eq!((prepared, imports, errors), (2, 0, 0));
    assert_eq!(image.pixel(100, 100), [230, 128, 25, 255], "the second buffer shows after the first was freed");
}

/// The software output, reporting the driver busy for a set number of ticks after each buffer it prepares for the first
/// time (`Gl::prepare` imports and warms a buffer once; a swapchain's buffers come round warm).
struct Warming {
    inner: Software,
    /// Ticks still to hold.
    hold: Arc<AtomicU32>,
    /// Ticks to hold per prepared buffer.
    per_buffer: u32,
    warmed: HashSet<ObjectId>,
}

impl Output for Warming {
    fn name(&self) -> &'static str {
        "warming"
    }
    fn size(&self) -> (u32, u32) {
        self.inner.size()
    }
    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        self.inner.query_buffer(buffer)
    }
    fn prepare_buffer(&mut self, buffer: &WlBuffer) -> bool {
        if !self.warmed.insert(buffer.id()) {
            return false;
        }
        self.hold.fetch_add(self.per_buffer, Ordering::SeqCst);
        true
    }
    fn warming(&mut self) -> bool {
        // Only the server thread calls this.
        let n = self.hold.load(Ordering::SeqCst);
        if n == 0 {
            return false;
        }
        self.hold.store(n - 1, Ordering::SeqCst);
        true
    }
    fn forget_buffer(&mut self, buffer: &WlBuffer) {
        self.inner.forget_buffer(buffer)
    }
    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> std::io::Result<RenderStats> {
        self.inner.render(plan, surfaces)
    }
    fn present(&mut self) -> std::io::Result<Presented> {
        self.inner.present()
    }
    fn capture(&mut self) -> std::io::Result<Image> {
        self.inner.capture()
    }
}

#[test]
fn the_tick_holds_the_frame_while_a_prepared_buffer_warms() {
    let hold = Arc::new(AtomicU32::new(0));
    let hold2 = hold.clone();
    let mut server = Server::start_with(Box::new(move || Box::new(Warming { inner: Software::new(1920, 1080), hold: hold2, per_buffer: 5, warmed: HashSet::new() })));
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Warm");
    let first = client.shm_buffer(300, 200, 0xff19_80e6);
    client.present(&card.surface, &first);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1), "the frame comes once the hold is over");
    let (held, drawn, image) = server.eval(|s| (s.stats.held_ticks, s.stats.frames_drawn, s.output.capture().unwrap()));
    assert_eq!(held, 5, "five ticks drew nothing while the buffer warmed");
    assert_eq!(drawn, 1, "then one frame drew it");
    assert_eq!(image.pixel(100, 100), [25, 128, 230, 255]);
    assert_eq!(hold.load(Ordering::SeqCst), 0);

    // A second buffer of the same surface: the first stays on the screen for the five held ticks.
    let second = client.shm_buffer(300, 200, 0xffe6_8019);
    client.present(&card.surface, &second);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 2));
    let (held, drawn, image) = server.eval(|s| (s.stats.held_ticks, s.stats.frames_drawn, s.output.capture().unwrap()));
    assert_eq!((held, drawn), (10, 2));
    assert_eq!(image.pixel(100, 100), [230, 128, 25, 255]);
    assert!(server.summary().contains("held ticks 10"), "{}", server.summary());
}

/// X1's start on the eighth gate, under a driver that warms each new buffer: Home on the screen, X1's window made,
/// typed and named with no buffer, then X1's three swapchain buffers, each prepared at its commit and held while it
/// warms, then the first again, warm by now. The window alone draws no frame: the redraw of the same picture there
/// flipped 15 ms before X1's first commit on the set, and that frame's flip two refreshes later read as a skipped one.
#[test]
fn x1s_start_holds_each_new_buffer_and_draws_nothing_for_its_window() {
    let hold = Arc::new(AtomicU32::new(0));
    let hold2 = hold.clone();
    let mut server = Server::start_with(Box::new(move || Box::new(Warming { inner: Software::new(1920, 1080), hold: hold2, per_buffer: 3, warmed: HashSet::new() })));
    let mut client = Client::connect(&mut server);
    let home = client.card("_WEBOS_WINDOW_TYPE_CARD", "Home");
    let page = client.shm_buffer(1920, 1080, 0xff20_2020);
    client.present(&home.surface, &page);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    assert_eq!(server.eval(|s| s.stats.frames_drawn), 1);

    let x1 = client.card("_WEBOS_WINDOW_TYPE_OVERLAY", "X1");
    x1.webos.set_property("title".into(), "LumaTV X1 floor probe".into());
    client.roundtrip();
    let start = server.eval(|s| s.stats.ticks);
    let deadline = Instant::now() + Duration::from_secs(3);
    while server.eval(|s| s.stats.ticks) < start + 5 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(server.eval(|s| s.stats.ticks) >= start + 5, "the server ticked");
    assert_eq!(server.eval(|s| s.stats.frames_drawn), 1, "a window without a buffer changes nothing on the screen");

    let swapchain: Vec<_> = (1..=3).map(|i| client.shm_buffer(1920, 1080, 0xff00_0000 | (0x40 * i))).collect();
    for (i, b) in swapchain.iter().enumerate() {
        client.present(&x1.surface, b);
        assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 2 + i as u32), "X1's frame {}", i + 1);
    }
    let (held, drawn, prepared) = server.eval(|s| (s.stats.held_ticks, s.stats.frames_drawn, s.stats.prepared));
    assert_eq!(prepared, 4, "Home's buffer and X1's three, each at its commit");
    assert_eq!(held, 4 * 3, "each new buffer held the frame while it warmed");
    assert_eq!(drawn, 4, "one frame per buffer, none for the window");

    client.present(&x1.surface, &swapchain[0]);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 5));
    let (held, drawn, image) = server.eval(|s| (s.stats.held_ticks, s.stats.frames_drawn, s.output.capture().unwrap()));
    assert_eq!((held, drawn), (12, 5), "a buffer the swapchain brings round is warm: drawn at once");
    assert_eq!(image.pixel(960, 540), [0, 0, 0x40, 255]);
    assert_eq!(hold.load(Ordering::SeqCst), 0);
}
