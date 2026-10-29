//! `Gl::acquire`, the barrier job direct scan-out puts behind a client's rendering, on Mesa's llvmpipe: one texel of the buffer is sampled into the warm
//! target and a fence is kicked, with no GL error, and the frame that is composed after it (the same ring, the same program) is drawn as it was
//! before. A buffer Mesa can sample is an shm one here; on the set it is the driver's dma-buf, imported the same way by the commit that made it.

mod harness;

use std::io;
use std::path::Path;
use std::sync::Arc;

use harness::{Client, Server};
use lumatv_compositor::kms::frames::Frames;
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl, TargetId};
use lumatv_compositor::scene::Plan;
use lumatv_compositor::surface::Surfaces;
use parking_lot::Mutex;
use wayland_server::protocol::wl_buffer::WlBuffer;

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";

/// The ring's frame logic over Mesa, with the acquire made on the window's buffer before every frame (as `Kms::render` does for a frame it scans out).
struct AcquireOut {
    gl: Gl,
    frames: Frames,
    pages: Vec<TargetId>,
    acquired: Arc<Mutex<Vec<bool>>>,
}

impl Output for AcquireOut {
    fn name(&self) -> &'static str {
        "acquire-test"
    }

    fn size(&self) -> (u32, u32) {
        self.gl.size()
    }

    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        self.gl.query_buffer(buffer)
    }

    fn forget_buffer(&mut self, buffer: &WlBuffer) {
        self.gl.forget_buffer(buffer)
    }

    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats> {
        if let Some(item) = plan.items.first() {
            let ok = self.gl.acquire(&item.buffer);
            self.acquired.lock().push(ok);
        }
        let (_, stats) = self.frames.draw(&mut self.gl, plan, surfaces, None)?;
        Ok(stats)
    }

    fn present(&mut self) -> io::Result<Presented> {
        self.gl.present()
    }

    fn capture(&mut self) -> io::Result<Image> {
        self.gl.set_target(self.pages[self.frames.last_page()]);
        self.gl.capture()
    }
}

#[test]
fn the_acquire_barrier_leaves_the_renderer_as_it_found_it() {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return;
    }
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let acquired = Arc::new(Mutex::new(Vec::new()));
    let log = acquired.clone();
    let mut server = Server::start_with(Box::new(move || {
        let mut gl = Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap();
        let pages: Vec<TargetId> = (0..3).map(|_| gl.add_target().unwrap()).collect();
        let frames = Frames::new(pages.clone(), 1920, 1080);
        Box::new(AcquireOut { gl, frames, pages, acquired: log })
    }));
    server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    let first = client.shm_buffer(1920, 1080, 0xff33_6699);
    client.present(&win.surface, &first);
    client.roundtrip();
    let img = server.eval(harness::drawn);
    assert_eq!(img.pixel(100, 100), [0x33, 0x66, 0x99, 0xff], "the composed frame after the barrier shows the window");
    assert_eq!(acquired.lock().as_slice(), [true], "the buffer was sampled and the fence kicked without a GL error");

    // Again, a new buffer: the barrier's own state (the warm target, the scissor of one texel) must not leak into the frame after it.
    let second = client.shm_buffer(1920, 1080, 0xffcc_5511);
    client.present(&win.surface, &second);
    client.roundtrip();
    let img = server.eval(harness::drawn);
    assert_eq!(img.pixel(100, 100), [0xcc, 0x55, 0x11, 0xff]);
    assert_eq!(img.pixel(1900, 1000), [0xcc, 0x55, 0x11, 0xff], "the whole window, not the one texel the barrier drew");
    assert_eq!(acquired.lock().as_slice(), [true, true]);
}
