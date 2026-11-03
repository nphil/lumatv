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
    stood_in: Option<Arc<std::sync::atomic::AtomicU32>>,
    redraw_same: Option<Arc<AtomicBool>>,
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

    fn ui_set_exclusion(&mut self, area: Option<lumatv_compositor::Rect>) {
        self.gl.ui_set_exclusion(area);
    }

    fn ui_set_standin_root(&mut self, root: Option<lumatv_compositor::surface::SurfaceId>) {
        self.gl.ui_set_standin_root(root);
    }

    fn ui_needs_frame(&mut self) -> io::Result<bool> {
        Ok(self.gl.ui_needs_frame()? || self.gl.standin_due())
    }

    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats> {
        if self.redraw_same.as_ref().is_some_and(|requested| requested.swap(false, Ordering::SeqCst)) {
            self.frames.redraw(lumatv_compositor::kms::assign::Source::Page(self.frames.last_page()));
        }
        let (_, stats) = self.frames.draw(&mut self.gl, plan, surfaces, None)?;
        if let Some(count) = &self.stood_in { count.store(stats.stood_in, Ordering::SeqCst); }
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
        Box::new(ScanOut { gl, frames, pages, hold_next: h, release: r, held: None, stood_in: None, redraw_same: None })
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

    // Warm now: a frame of its own shows the new content, and nothing is owed after it. The warm draw's own fence is
    // polled at the ticks too (Mesa's threads take a moment on a loaded host), so the ticks go on until it is seen done.
    release.store(true, Ordering::SeqCst);
    let mut drawn = 2;
    let mut image = None;
    for _ in 0..200 {
        let (d, i) = server.eval(|s| {
            s.tick();
            (s.stats.frames_drawn, s.output.capture().unwrap())
        });
        (drawn, image) = (d, Some(i));
        if drawn == 3 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(drawn, 3, "the owed frame was drawn once the buffer was warm");
    assert_eq!(image.unwrap().pixel(100, 100), [230, 128, 25, 255], "the new buffer's content is on the page");
    let (drawn, errors) = server.eval(|s| {
        s.tick();
        (s.stats.frames_drawn, s.stats.gl_errors)
    });
    assert_eq!((drawn, errors), (3, 0), "nothing more is owed, and no GL error on the way");
}

/// A client's first buffer, with no page holding a last frame to stand in from (the session's first composed frame): the
/// frame is held until the driver is done, and no pass of the loop waits for that. On the tenth gate's trial a pass of
/// 55.2 ms came with the warm of a buffer whose fence was seen done 48.4 ms after; the loop itself never waits on it.
#[test]
fn a_first_buffers_warm_holds_the_frame_and_no_pass_of_the_loop() {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return;
    }
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let hold_next = Arc::new(AtomicBool::new(true));
    let release = Arc::new(AtomicBool::new(false));
    let (h, r) = (hold_next.clone(), release.clone());
    let mut server = Server::start_with(Box::new(move || {
        let mut gl = Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap();
        let pages: Vec<TargetId> = (0..3).map(|_| gl.add_target().unwrap()).collect();
        let frames = Frames::new(pages.clone(), 1920, 1080);
        Box::new(ScanOut { gl, frames, pages, hold_next: h, release: r, held: None, stood_in: None, redraw_same: None })
    }));
    server.ticking.store(false, Ordering::SeqCst);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "First");
    let blue = client.shm_buffer(300, 200, 0xff19_80e6);
    client.present(&card.surface, &blue);
    client.roundtrip();
    // Ticks while the driver is busy: each returns at once with the frame held, nothing drawn, the callback kept.
    let mut worst = Duration::ZERO;
    for _ in 0..5 {
        let (took, drawn, held) = server.eval(|s| {
            let t0 = std::time::Instant::now();
            s.tick();
            (t0.elapsed(), s.stats.frames_drawn, s.stats.held_ticks)
        });
        worst = worst.max(took);
        assert_eq!(drawn, 0, "nothing drawn while the first buffer warms");
        assert!(held >= 1);
    }
    assert!(worst < Duration::from_millis(50), "a held tick took {worst:?}");
    client.roundtrip();
    assert_eq!(client.state.frames_done, 0, "the client's callback waits for the frame that shows its buffer");
    // The driver is done: the tick that sees it (the warm draw's own fence is polled at the ticks as well) draws the
    // buffer and pays the callback.
    release.store(true, Ordering::SeqCst);
    let mut drawn = 0;
    let mut image = None;
    for _ in 0..200 {
        let (d, i) = server.eval(|s| {
            s.tick();
            (s.stats.frames_drawn, s.output.capture().unwrap())
        });
        (drawn, image) = (d, Some(i));
        if drawn == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(drawn, 1);
    assert_eq!(image.unwrap().pixel(100, 100), [25, 128, 230, 255]);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
}

#[cfg(feature = "ui")]
#[test]
fn popup_is_not_redrawn_over_a_warming_owned_caption_frame() {
    use compositor_protocol::{Event, Fill, Props, Rgba, Shape};
    use lumashow_ui::{Scene, SceneOptions, Ui};
    use lumatv_wl_protocols::lumatv_subtitles_v1::client::lumatv_subtitle_manager_v1::LumatvSubtitleManagerV1;

    if !Path::new(MESA).exists() { panic!("Mesa required at {MESA}"); }
    let shm = std::env::temp_dir().join(format!("lumatv-standin-popup-{}", std::process::id()));
    std::fs::create_dir_all(&shm).unwrap();
    let dir = shm.clone();
    let hold_next = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let (h, r) = (hold_next.clone(), release.clone());
    let stood_in = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let stood_in_output = stood_in.clone();
    let redraw_same = Arc::new(AtomicBool::new(false));
    let redraw_same_output = redraw_same.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut server = Server::start_with(Box::new(move || {
        let mut gl = Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap();
        tx.send(gl.attach_ui(MESA, 1000, dir).unwrap()).unwrap();
        gl.ui().unwrap().finish_handshake().unwrap();
        let pages: Vec<_> = (0..3).map(|_| gl.add_target().unwrap()).collect();
        let frames = Frames::new(pages.clone(), 1920, 1080);
        Box::new(ScanOut { gl, frames, pages, hold_next: h, release: r, held: None, stood_in: Some(stood_in_output), redraw_same: Some(redraw_same_output) })
    }));
    let (scene, _, events) = Scene::over_with(rx.recv().unwrap(), SceneOptions { shm_dir: Some(shm.clone()), ..Default::default() }).unwrap();
    let ui = Ui::new(scene);
    let shape = Shape { corner_radius: 0.0, fill: Fill::Solid(Rgba(220, 40, 60, 255)), border_width: 0.0, border: Rgba(0, 0, 0, 0) };
    let popup = ui.scene.shape(100, 80, 1800, shape, Props::at(20.0, 20.0));
    let tag = ui.try_commit().unwrap().unwrap();
    loop {
        let event = events.recv_timeout(Duration::from_secs(5)).expect("UI frame");
        ui.scene.observe(&event);
        if matches!(event, Event::Frame { tag: got, .. } if got >= tag) { break; }
    }
    server.ticking.store(false, Ordering::SeqCst);
    let mut client = Client::connect(&mut server);
    let manager: LumatvSubtitleManagerV1 = client.globals.bind(&client.qh, 1..=1, ()).unwrap();
    let card = client.card_as("_WEBOS_WINDOW_TYPE_CARD", "Marquee", "com.beb.plxnative");
    let region = manager.get_region(&card.surface, &client.qh, ());
    region.set_bounds(1920, 1080, 0, 0, 0, 0);
    let blue = client.shm_buffer(1920, 1080, 0);
    client.present(&card.surface, &blue);
    client.roundtrip();
    server.eval(|s| {
        let root = s.scene.windows().find(|w| w.app_id() == "com.beb.plxnative").unwrap().surface;
        s.scene.set_video_expected(root, true);
        s.scene.holes.push(lumatv_compositor::scene::Hole { surface: root, rect: lumatv_compositor::Rect::new(0, 0, 1920, 1080) });
    });
    let first = server.eval(harness::drawn);
    assert_eq!(first.pixel(30, 30), [220, 40, 60, 255], "popup is present on the empty-caption page");
    assert_eq!(first.pixel(500, 400), [0, 0, 0, 0], "the video hole remains transparent");
    assert!(server.eval(|s| s.scene.last_plan().video && !s.scene.last_plan().holes.is_empty()));

    redraw_same.store(true, Ordering::SeqCst);
    hold_next.store(true, Ordering::SeqCst);
    region.set_bounds(1920, 1080, 20, 20, 100, 80);
    let green = client.shm_pattern(1920, 1080, |x, y| if (20..120).contains(&x) && (20..100).contains(&y) { 0xff19_e680 } else { 0 });
    client.present(&card.surface, &green);
    client.roundtrip();
    let (old, held) = server.eval(|s| { let held = s.stats.held_ticks; s.tick(); (s.output.capture().unwrap(), s.stats.held_ticks - held) });
    assert_eq!(held, 0);
    assert_eq!(stood_in.load(Ordering::SeqCst), 1, "a real stand-in was used");
    assert_eq!(old.pixel(30, 30), [220, 40, 60, 255], "the old empty-caption page remains intact");
    assert_eq!(old.pixel(500, 400), [0, 0, 0, 0], "the stand-in preserves the video hole");
    release.store(true, Ordering::SeqCst);
    let mut image = None;
    for _ in 0..200 {
        let next = server.eval(|s| { s.tick(); s.output.capture().unwrap() });
        let ready = next.pixel(30, 30) == [25, 230, 128, 255];
        image = Some(next);
        if ready { break; }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(image.unwrap().pixel(30, 30), [25, 230, 128, 255], "new caption excludes the popup");

    release.store(false, Ordering::SeqCst);
    redraw_same.store(true, Ordering::SeqCst);
    hold_next.store(true, Ordering::SeqCst);
    region.set_bounds(1920, 1080, 25, 25, 80, 60);
    let purple = client.shm_pattern(1920, 1080, |x, y| if (25..105).contains(&x) && (25..85).contains(&y) { 0xffc0_40c0 } else { 0 });
    client.present(&card.surface, &purple);
    client.roundtrip();
    let (old, held) = server.eval(|s| { let held = s.stats.held_ticks; s.tick(); (s.output.capture().unwrap(), s.stats.held_ticks - held) });
    assert_eq!(held, 0);
    assert_eq!(stood_in.load(Ordering::SeqCst), 1);
    assert_eq!(old.pixel(30, 30), [25, 230, 128, 255], "the old caption remains while the changed caption warms");
    release.store(true, Ordering::SeqCst);
    let mut changed = None;
    for _ in 0..200 {
        let next = server.eval(|s| { s.tick(); s.output.capture().unwrap() });
        let ready = next.pixel(30, 30) == [192, 64, 192, 255];
        changed = Some(next);
        if ready { break; }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(changed.unwrap().pixel(30, 30), [192, 64, 192, 255], "changed caption appears without the popup");

    release.store(false, Ordering::SeqCst);
    hold_next.store(true, Ordering::SeqCst);
    region.set_bounds(1920, 1080, 0, 0, 0, 0);
    popup.set(Props::at(22.0, 22.0));
    ui.try_commit().unwrap().unwrap();
    let orange = client.shm_buffer(1920, 1080, 0);
    client.present(&card.surface, &orange);
    client.roundtrip();
    for _ in 0..2 {
        redraw_same.store(true, Ordering::SeqCst);
        card.surface.damage_buffer(0, 0, 1920, 1080);
        card.surface.commit();
        client.roundtrip();
        let old = server.eval(|s| { s.tick(); s.output.capture().unwrap() });
        assert_eq!(stood_in.load(Ordering::SeqCst), 1);
        assert_eq!(old.pixel(30, 30), [192, 64, 192, 255], "fresh popup stays off the old caption across stand-ins");
    }
    release.store(true, Ordering::SeqCst);
    let mut image = None;
    for _ in 0..200 {
        let next = server.eval(|s| { s.tick(); s.output.capture().unwrap() });
        let ready = next.pixel(30, 30) == [220, 40, 60, 255];
        image = Some(next);
        if ready { break; }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(image.unwrap().pixel(30, 30), [220, 40, 60, 255], "popup returns once the empty-caption frame is warm");
    drop((client, ui, server));
    let _ = std::fs::remove_dir_all(shm);
}
