//! A page flip's frame callbacks go out as soon as the compositor learns the flip landed, not after whatever else the
//! loop does next: a client like X1 draws its next frame on that callback, so a slow draw handled first is a gap in
//! the client (eighth gate: a 20.4 ms gap on X1's surface with the flip reached 3.6 ms after it landed). The output
//! here holds every frame on the way until the test lands its flip, and takes 150 ms to draw one.

mod harness;

use harness::{Client, Server};
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::render::software::Software;
use lumatv_compositor::scene::Plan;
use lumatv_compositor::server::state::ClientCallbacks;
use lumatv_compositor::surface::Surfaces;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};
use wayland_client::Proxy;
use wayland_server::protocol::wl_buffer::WlBuffer;

const DRAW: Duration = Duration::from_millis(150);
/// A draw short enough for the 250 ms a frame callback is held at most (`State::route_callbacks`) to outlast a test's waits.
const QUICK_DRAW: Duration = Duration::from_millis(20);

/// The software output with KMS's timing: a frame drawn by a tick is pending until its flip lands (`landed`, set by the
/// test; a tick that draws nothing has no flip), and a draw takes `draw`.
struct Flipping {
    inner: Software,
    landed: Arc<AtomicU32>,
    drawn_at: Arc<Mutex<Vec<Instant>>>,
    drew: bool,
    draw: Duration,
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
        std::thread::sleep(self.draw);
        let st = self.inner.render(plan, surfaces);
        self.drawn_at.lock().push(Instant::now());
        self.drew = true;
        st
    }
    fn present(&mut self) -> std::io::Result<Presented> {
        Ok(Presented { pending: std::mem::take(&mut self.drew), ..self.inner.present()? })
    }
    fn dispatch_events(&mut self) -> std::io::Result<u32> {
        Ok(self.landed.swap(0, Ordering::SeqCst))
    }
    fn capture(&mut self) -> std::io::Result<Image> {
        self.inner.capture()
    }
}

/// A server on the flipping output, the counter of flips to land and the log of draws.
fn flipping(draw: Duration) -> (Server, Arc<AtomicU32>, Arc<Mutex<Vec<Instant>>>) {
    let landed = Arc::new(AtomicU32::new(0));
    let drawn_at = Arc::new(Mutex::new(Vec::new()));
    let (landed2, drawn_at2) = (landed.clone(), drawn_at.clone());
    let server =
        Server::start_with(Box::new(move || Box::new(Flipping { inner: Software::new(1920, 1080), landed: landed2, drawn_at: drawn_at2, drew: false, draw })));
    (server, landed, drawn_at)
}

fn wait_drawn(drawn_at: &Mutex<Vec<Instant>>, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while drawn_at.lock().len() < n && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(drawn_at.lock().len() >= n, "frame {n} was drawn");
}

#[test]
fn a_landed_flip_has_its_callbacks_out_before_the_next_frame_is_drawn() {
    let (mut server, landed, drawn_at) = flipping(DRAW);
    // The flip path: `ctl callbacks clients landing`.
    server.eval(|s| s.client_callbacks = ClientCallbacks::Landing);
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

/// Commits a buffer of one colour to the window, with a frame callback.
fn commit(client: &mut Client, window: &harness::Window, argb: u32) -> wayland_client::protocol::wl_buffer::WlBuffer {
    // Small: a full-size buffer takes the test client 100 ms to fill, with half the 250 ms a callback is held at most gone.
    let buffer = client.shm_buffer(64, 36, argb);
    client.present(&window.surface, &buffer);
    buffer
}

#[test]
fn a_shell_windows_callback_goes_out_when_its_frame_is_the_oldest_on_its_way() {
    let (mut server, landed, drawn_at) = flipping(QUICK_DRAW);
    let mut client = Client::connect(&mut server);
    let shell = client.card("_WEBOS_WINDOW_TYPE_LUMATV_HOME", "LumaHome");
    commit(&mut client, &shell, 0xff19_80e6);
    wait_drawn(&drawn_at, 1);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1), "nothing is ahead of the first frame: its callback goes out when it is drawn");

    // The client's next frame, made on that callback, is drawn while the first is on its way: it waits for the first to land.
    commit(&mut client, &shell, 0xffe6_8019);
    wait_drawn(&drawn_at, 2);
    assert!(!client.wait_until(Duration::from_millis(100), |s| s.frames_done >= 2), "behind a frame on its way, a callback waits for it");
    landed.store(1, Ordering::SeqCst);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 2), "the first landed: the second is the oldest and its callback goes out");
    assert_eq!(server.eval(|s| s.stats.callbacks_ahead), 2);
}

#[test]
fn with_the_switch_off_a_shell_windows_callback_waits_for_its_flip() {
    let (mut server, landed, drawn_at) = flipping(QUICK_DRAW);
    server.eval(|s| s.shell_ahead = false);
    let mut client = Client::connect(&mut server);
    let shell = client.card("_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM", "LumaLayer");
    commit(&mut client, &shell, 0xff19_80e6);
    wait_drawn(&drawn_at, 1);
    assert!(!client.wait_until(Duration::from_millis(100), |s| s.frames_done >= 1), "its callback waits for its flip");
    landed.store(1, Ordering::SeqCst);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    assert_eq!(server.eval(|s| s.stats.callbacks_ahead), 0);
}

#[test]
fn a_shell_windows_replaced_buffer_is_released_when_the_frame_that_replaced_it_lands() {
    let (mut server, landed, drawn_at) = flipping(QUICK_DRAW);
    let mut client = Client::connect(&mut server);
    let shell = client.card("_WEBOS_WINDOW_TYPE_LUMATV_HOME", "LumaHome");
    let first = commit(&mut client, &shell, 0xff19_80e6).id().protocol_id();
    wait_drawn(&drawn_at, 1);
    commit(&mut client, &shell, 0xffe6_8019);
    wait_drawn(&drawn_at, 2);
    // The first buffer is on the screen's way until the second frame, the one that replaced it, lands: the client, let run ahead,
    // would draw into it at once if it had it back.
    assert!(!client.wait_until(Duration::from_millis(100), |s| s.releases.contains(&first)), "not given back when the second frame is drawn");
    landed.store(1, Ordering::SeqCst);
    assert!(!client.wait_until(Duration::from_millis(100), |s| s.releases.contains(&first)), "nor when the first frame lands");
    landed.store(1, Ordering::SeqCst);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.releases.contains(&first)), "given back when the second frame lands");
}

#[test]
fn a_cards_callback_and_replaced_buffer_go_out_when_its_frame_is_the_oldest_on_its_way() {
    let (mut server, landed, drawn_at) = flipping(QUICK_DRAW);
    // The draw path: `ctl callbacks clients early`.
    server.eval(|s| s.client_callbacks = ClientCallbacks::Draw);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "X1");
    let first = commit(&mut client, &card, 0xff19_80e6).id().protocol_id();
    wait_drawn(&drawn_at, 1);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1), "nothing is ahead of the first frame: its callback goes out when it is drawn");
    commit(&mut client, &card, 0xffe6_8019);
    wait_drawn(&drawn_at, 2);
    assert!(
        !client.wait_until(Duration::from_millis(100), |s| s.frames_done >= 2 || s.releases.contains(&first)),
        "behind a frame on its way, neither the callback nor the buffer the second frame replaced goes out"
    );
    landed.store(1, Ordering::SeqCst);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.releases.contains(&first)), "the first landed: the buffer is back");
    assert_eq!(client.state.frames_done, 2, "with the second frame's callback");
    assert_eq!(server.eval(|s| s.stats.callbacks_ahead), 2);
}

#[test]
fn with_client_callbacks_at_the_flip_a_cards_replaced_buffer_is_released_when_the_frame_that_replaced_it_is_drawn() {
    let (mut server, _landed, drawn_at) = flipping(QUICK_DRAW);
    server.eval(|s| s.client_callbacks = ClientCallbacks::Landing);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "X1");
    let first = commit(&mut client, &card, 0xff19_80e6).id().protocol_id();
    wait_drawn(&drawn_at, 1);
    commit(&mut client, &card, 0xffe6_8019);
    wait_drawn(&drawn_at, 2);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.releases.contains(&first)), "no flip has landed, and the buffer is back");
    assert_eq!(server.eval(|s| s.stats.callbacks_ahead), 0);
}

/// PlxNative's close as LG's libmali (r46p0) and SDL make it (2026-10-29). libmali presents each swap on a thread of its own
/// (`display_window_buffer`): it waits there for the frame callback of the frame before, polling the socket a second at a time,
/// then attaches and commits on the `wl_surface` it kept; the app's swap returns up to two frames ahead of that thread. LG's SDL
/// destroys the `wl_surface` before `eglTerminate`, which waits for the thread. A present still waiting at the destroy runs on the
/// freed surface when its callback comes (round 16), or, with the callback destroyed with the surface, waits out the second and
/// SDL's force-quit crashes (rounds 17 and 18).
///
/// The client here: a first frame, drawn and landed; the second committed the moment the first's callback came, as libmali's
/// thread does (on the set 1-2 ms after the draw, a whole tick before the next); the app's third swap is then waiting on the
/// second's callback when SDL's teardown destroys the surface, unless that callback came with the commit. Returns whether it had;
/// whatever comes after the destroy is too late either way (a `done` runs the present on the freed surface, none stalls it).
fn mali_close_finds_the_last_callback_answered(policy: ClientCallbacks) -> bool {
    let (mut server, landed, drawn_at) = flipping(QUICK_DRAW);
    server.eval(move |s| s.client_callbacks = policy);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "plxnative");
    commit(&mut client, &card, 0xff19_80e6);
    wait_drawn(&drawn_at, 1);
    landed.store(1, Ordering::SeqCst);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1), "the first frame's callback");
    // The tick that would draw the second frame has not come.
    server.ticking.store(false, Ordering::SeqCst);
    commit(&mut client, &card, 0xffe6_8019);
    client.roundtrip();
    let answered = client.state.frames_done >= 2;
    // SDL's Wayland_DestroyWindow: the window's surface goes before eglTerminate (its shell surface only on the client's side).
    card.surface.destroy();
    client.roundtrip();
    server.ticking.store(true, Ordering::SeqCst);
    answered
}

#[test]
fn a_mali_clients_close_finds_its_last_callback_answered() {
    assert!(mali_close_finds_the_last_callback_answered(ClientCallbacks::Commit), "the callback came with the commit, before the teardown");
}

#[test]
fn with_client_callbacks_at_the_draw_a_mali_clients_close_can_leave_a_present_waiting() {
    assert!(!mali_close_finds_the_last_callback_answered(ClientCallbacks::Draw), "the callback waits for a draw the teardown comes first to");
}

#[test]
fn with_client_callbacks_at_the_flip_a_mali_clients_close_can_leave_a_present_waiting() {
    assert!(!mali_close_finds_the_last_callback_answered(ClientCallbacks::Landing), "the callback waits for a flip the teardown comes first to");
}

/// With the callbacks at the commit, the buffers a client's commits replaced are still given back at the draw, as under `early`.
#[test]
fn with_client_callbacks_at_the_commit_a_cards_replaced_buffer_is_released_at_the_draw() {
    let (mut server, _landed, drawn_at) = flipping(QUICK_DRAW);
    server.eval(|s| s.client_callbacks = ClientCallbacks::Commit);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "X1");
    let first = commit(&mut client, &card, 0xff19_80e6).id().protocol_id();
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1), "the first frame's callback comes with its commit");
    wait_drawn(&drawn_at, 1);
    commit(&mut client, &card, 0xffe6_8019);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 2), "and the second's");
    wait_drawn(&drawn_at, 2);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.releases.contains(&first)), "the buffer the second frame replaced is back at its draw");
    let (at_commit, ahead) = server.eval(|s| (s.stats.callbacks_at_commit, s.stats.callbacks_ahead));
    assert_eq!((at_commit, ahead), (2, 0));
}
