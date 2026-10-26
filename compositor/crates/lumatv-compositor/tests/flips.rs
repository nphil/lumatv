//! A page flip's frame callbacks go out as soon as the compositor learns the flip landed, not after whatever else the
//! loop does next: a client like X1 draws its next frame on that callback, so a slow draw handled first is a gap in
//! the client (eighth gate: a 20.4 ms gap on X1's surface with the flip reached 3.6 ms after it landed). The output
//! here holds every frame on the way until the test lands its flip, and takes 150 ms to draw one.

mod harness;

use harness::{Client, Server};
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::render::software::Software;
use lumatv_compositor::scene::Plan;
use lumatv_compositor::surface::Surfaces;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use wayland_server::protocol::wl_buffer::WlBuffer;

const DRAW: Duration = Duration::from_millis(150);

/// The software output with KMS's timing: a presented frame is pending until its flip lands (`landed`, set by the
/// test), and a draw takes `DRAW`.
struct Flipping {
    inner: Software,
    landed: Arc<AtomicU32>,
    drawn_at: Arc<Mutex<Vec<Instant>>>,
}

impl Output for Flipping {
    fn name(&self) -> &'static str {
        "flipping"
    }
    fn size(&self) -> (u32, u32) {
        self.inner.size()
    }
    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        self.inner.query_buffer(buffer)
    }
    fn forget_buffer(&mut self, buffer: &WlBuffer) {
        self.inner.forget_buffer(buffer)
    }
    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> std::io::Result<RenderStats> {
        std::thread::sleep(DRAW);
        let st = self.inner.render(plan, surfaces);
        self.drawn_at.lock().push(Instant::now());
        st
    }
    fn present(&mut self) -> std::io::Result<Presented> {
        Ok(Presented { pending: true, ..self.inner.present()? })
    }
    fn dispatch_events(&mut self) -> std::io::Result<u32> {
        Ok(self.landed.swap(0, Ordering::SeqCst))
    }
    fn capture(&mut self) -> std::io::Result<Image> {
        self.inner.capture()
    }
}

#[test]
fn a_landed_flip_has_its_callbacks_out_before_the_next_frame_is_drawn() {
    let landed = Arc::new(AtomicU32::new(0));
    let drawn_at = Arc::new(Mutex::new(Vec::new()));
    let (landed2, drawn_at2) = (landed.clone(), drawn_at.clone());
    let mut server = Server::start_with(Box::new(move || Box::new(Flipping { inner: Software::new(1920, 1080), landed: landed2, drawn_at: drawn_at2 })));
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "X1");
    let first = client.shm_buffer(1920, 1080, 0xff19_80e6);
    client.present(&card.surface, &first);
    let deadline = Instant::now() + Duration::from_secs(3);
    while drawn_at.lock().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(drawn_at.lock().len(), 1, "the first frame was drawn");
    assert!(!client.wait_until(Duration::from_millis(50), |s| s.frames_done >= 1), "its callback waits for its flip");

    // The next frame's commit is in and the first frame's flip lands before the tick that draws the second.
    server.ticking.store(false, Ordering::SeqCst);
    let second = client.shm_buffer(1920, 1080, 0xffe6_8019);
    client.present(&card.surface, &second);
    client.roundtrip();
    landed.store(1, Ordering::SeqCst);
    server.ticking.store(true, Ordering::SeqCst);

    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1), "the first frame's callback came");
    let done_at = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(3);
    while drawn_at.lock().len() < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    let second_drawn = *drawn_at.lock().get(1).expect("the second frame was drawn");
    assert!(done_at < second_drawn, "the callback reached the client {:?} after the second frame's draw ended", done_at - second_drawn);
}
