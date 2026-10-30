//! The KMS output's frame logic (`kms::frames`) with the UI engine attached, over pages in the context's own memory and under
//! the real `State::tick` (feature `ui`, Mesa's software GL): direct scan-out of a full-screen opaque card while the UI shows
//! nothing over it, composition the frame a system layer appears when there is no UI ring, the split (the card direct on the
//! primary, the UI alone on an overlay page with alpha) when there is one, and the UI app's socket waking the loop.
#![cfg(feature = "ui")]

mod harness;

use compositor_protocol::{Event, Fill, Props, Rgba, Shape};
use harness::{Client, Server};
use lumashow_ui::{Scene, SceneOptions, ShapeLayer, Ui};
use lumatv_compositor::kms::assign::{Assignment, Source};
use lumatv_compositor::kms::frames::Frames;
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl, TargetId};
use lumatv_compositor::scene::Plan;
use lumatv_compositor::surface::Surfaces;
use parking_lot::Mutex;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use wayland_server::protocol::wl_buffer::WlBuffer;

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";
const FB: u32 = 7;

/// The KMS output without the device: the GL renderer, three pages of its own (and three with alpha for the overlay when the
/// rig asks) and the rings' frame logic. Every frame's assignment is kept for the test to read. Between frames the screen is
/// still, as KMS answers when no flip is in flight.
struct ScanOut {
    gl: Gl,
    frames: Frames,
    pages: Vec<TargetId>,
    ui_pages: Vec<TargetId>,
    sources: Arc<Mutex<Vec<Assignment>>>,
}

impl Output for ScanOut {
    fn name(&self) -> &'static str {
        "scanout-test"
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

    fn ui_needs_frame(&mut self) -> io::Result<bool> {
        self.gl.ui_needs_frame()
    }

    fn ui_fd(&self) -> Option<std::os::fd::RawFd> {
        self.gl.ui_fd()
    }

    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats> {
        // the device would take the window's buffer whenever the plan allows it
        let direct = plan.sole_fullscreen().map(|_| (FB, (0, 0)));
        let (assignment, stats) = self.frames.draw(&mut self.gl, plan, surfaces, direct)?;
        self.sources.lock().push(assignment);
        Ok(stats)
    }

    fn present(&mut self) -> io::Result<Presented> {
        self.gl.present()
    }

    /// The page last composed (what the panel would show after a composed frame), or the UI page of the last split frame when
    /// the frame before was one (what the overlay plane would show).
    fn capture(&mut self) -> io::Result<Image> {
        let last = self.sources.lock().last().copied();
        match last {
            Some(Assignment { overlay: Some((Source::UiPage(i), _)), .. }) => self.gl.set_target(self.ui_pages[i]),
            _ => self.gl.set_target(self.pages[self.frames.last_page()]),
        }
        self.gl.capture()
    }

    fn draws_on_damage(&self, _now: Instant) -> bool {
        true
    }
}

/// The compositor with the output above and the UI app over its socket: a Home ground, a small layer under the windows and a
/// system layer that starts hidden.
struct Rig {
    server: Server,
    ui: Ui,
    events: Receiver<Event>,
    sources: Arc<Mutex<Vec<Assignment>>>,
    shm: PathBuf,
    under: ShapeLayer,
    osd: ShapeLayer,
    _ground: ShapeLayer,
}

fn hidden() -> Props {
    Props { visible: false, ..Props::at(760.0, 900.0) }
}

impl Rig {
    /// `None` without Mesa. `split` gives the output a UI ring (an overlay plane that takes alpha).
    fn start(name: &str, split: bool) -> Option<Rig> {
        if !Path::new(MESA).exists() {
            eprintln!("skipped: no Mesa at {MESA}");
            return None;
        }
        // SAFETY: nothing else in this process reads the environment while it is set.
        unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
        let shm = std::env::temp_dir().join(format!("lumatv-ui-scanout-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&shm).unwrap();
        let dir = shm.clone();
        let sources = Arc::new(Mutex::new(Vec::new()));
        let log = sources.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let server = Server::start_with(Box::new(move || {
            let mut gl = Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap();
            tx.send(gl.attach_ui(MESA, 1000, dir).unwrap()).unwrap();
            gl.ui().unwrap().finish_handshake().unwrap();
            let pages: Vec<TargetId> = (0..3).map(|_| gl.add_target().unwrap()).collect();
            let ui_pages: Vec<TargetId> = if split { (0..3).map(|_| gl.add_target().unwrap()).collect() } else { Vec::new() };
            let mut frames = Frames::new(pages.clone(), 1920, 1080);
            if split {
                frames = frames.with_ui_pages(ui_pages.clone());
            }
            Box::new(ScanOut { gl, frames, pages, ui_pages, sources: log })
        }));
        let (scene, _, events) = Scene::over_with(rx.recv().unwrap(), SceneOptions { shm_dir: Some(shm.clone()), ..Default::default() }).unwrap();
        let ui = Ui::new(scene);
        let solid = |colour| Shape { corner_radius: 0.0, fill: Fill::Solid(colour), border_width: 0.0, border: Rgba(0, 0, 0, 0) };
        let ground = ui.scene.shape(1920, 1080, 0, solid(Rgba(18, 20, 32, 255)), Props::at(0.0, 0.0));
        let under = ui.scene.shape(100, 100, 1, solid(Rgba(90, 200, 90, 255)), Props::at(500.0, 500.0));
        let osd = ui.scene.shape(400, 60, 1000, solid(Rgba(245, 245, 255, 255)), hidden());
        Some(Rig { server, ui, events, sources, shm, under, osd, _ground: ground })
    }

    /// Commits what is staged and waits for the frame that shows it.
    fn commit(&self) {
        let tag = self.ui.try_commit().unwrap().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let event = self.events.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("a presented UI frame");
            self.ui.scene.observe(&event);
            if matches!(event, Event::Frame { tag: got, .. } if got >= tag) {
                break;
            }
        }
    }

    fn taken(&self) -> Vec<Assignment> {
        self.sources.lock().clone()
    }

    fn capture(&self) -> Image {
        self.server.eval(|s| s.output.capture().unwrap())
    }

    fn gl_errors(&self) -> u64 {
        self.server.eval(|s| s.stats.gl_errors)
    }
}
impl Drop for Rig {
    fn drop(&mut self) {
        // The server (the first field) goes right after this; what the engine still maps of the directory stays valid.
        let _ = std::fs::remove_dir_all(&self.shm);
    }
}

fn composed(page: usize) -> Assignment {
    Assignment { primary: Source::Page(page), overlay: None, at: (0, 0) }
}

fn direct() -> Assignment {
    Assignment { primary: Source::Direct(FB), overlay: None, at: (0, 0) }
}

fn split(ui_page: usize) -> Assignment {
    Assignment { primary: Source::Direct(FB), overlay: Some((Source::UiPage(ui_page), lumatv_compositor::Rect::new(0, 0, 1920, 1080))), at: (0, 0) }
}

/// A full-screen opaque card over Home, presented and on the screen.
fn card_over_home(rig: &mut Rig) -> Client {
    let mut client = Client::connect(&mut rig.server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Player");
    let opaque = client.region(0, 0, 1920, 1080);
    card.surface.set_opaque_region(Some(&opaque));
    let buffer = client.shm_buffer(1920, 1080, 0xff19_80e6);
    client.present(&card.surface, &buffer);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    client
}

#[test]
fn a_full_screen_window_is_scanned_out_directly_unless_the_ui_shows_something_over_it() {
    let Some(mut rig) = Rig::start("direct", false) else { return };

    // Home alone: composed.
    rig.commit();
    assert_eq!(rig.taken(), [composed(0)]);
    assert_eq!(rig.capture().pixel(600, 400), [18, 20, 32, 255]);

    // A full-screen opaque card over it: the plane takes the card's buffer, the pages are left as they were.
    let _client = card_over_home(&mut rig);
    let before = rig.taken();
    assert_eq!(before.last(), Some(&direct()), "{before:?}");
    assert_eq!(rig.capture().pixel(600, 400), [18, 20, 32, 255], "no page was composed for the card");

    // Home changes under the card: the app's commit is taken in and answered, and the frame is still direct.
    rig.under.set(Props::at(700.0, 500.0));
    rig.commit();
    let now = rig.taken();
    assert!(now.len() > before.len() && now[before.len()..].iter().all(|s| *s == direct()), "{now:?}");

    // A system layer appears: that frame is composed, and drawn whole (the card's frames never reached the pages).
    let before = rig.taken();
    rig.osd.set(Props::at(760.0, 900.0));
    rig.commit();
    let now = rig.taken();
    assert!(matches!(now[before.len()..], [Assignment { primary: Source::Page(_), overlay: None, .. }]), "{now:?}");
    let image = rig.capture();
    assert_eq!(image.pixel(780, 920), [245, 245, 255, 255], "the system layer is on the page");
    assert_eq!(image.pixel(600, 400), [25, 128, 230, 255], "the card is on the page, not the stale Home");
    assert_eq!(image.pixel(30, 30), [25, 128, 230, 255]);
    assert_eq!(image.pixel(1900, 1070), [25, 128, 230, 255]);
    assert_eq!(rig.gl_errors(), 0);

    // It goes: direct again, in the same frame.
    let before = rig.taken();
    rig.osd.set(hidden());
    rig.commit();
    let now = rig.taken();
    assert_eq!(&now[before.len()..], [direct()], "{now:?}");
    assert_eq!(rig.gl_errors(), 0);
}

#[test]
fn with_a_ui_ring_the_system_layer_goes_to_the_overlay_over_the_direct_window() {
    let Some(mut rig) = Rig::start("split", true) else { return };
    rig.commit();
    assert_eq!(rig.taken(), [composed(0)]);
    let _client = card_over_home(&mut rig);
    assert_eq!(rig.taken().last(), Some(&direct()));

    // The system layer appears: the card stays on the primary plane, the UI alone is drawn into a page with alpha for the
    // overlay, cleared to transparent everywhere else.
    let before = rig.taken();
    rig.osd.set(Props::at(760.0, 900.0));
    rig.commit();
    let now = rig.taken();
    assert_eq!(&now[before.len()..], [split(0)], "{now:?}");
    let image = rig.capture();
    assert_eq!(image.pixel(780, 920), [245, 245, 255, 255], "the system layer is on the UI page");
    assert_eq!(image.pixel(600, 400), [0, 0, 0, 0], "nothing of the card or Home is on it: the window shows through");
    assert_eq!(image.pixel(30, 30), [0, 0, 0, 0]);
    assert_eq!(rig.gl_errors(), 0);

    // It moves: the next UI page is drawn, only the tiles it owes (the old place cleared, the new one drawn).
    let before = rig.taken();
    rig.osd.set(Props::at(200.0, 100.0));
    rig.commit();
    let now = rig.taken();
    assert_eq!(&now[before.len()..], [split(1)], "{now:?}");
    let image = rig.capture();
    assert_eq!(image.pixel(220, 120), [245, 245, 255, 255], "the layer at its new place");
    assert_eq!(image.pixel(780, 920), [0, 0, 0, 0], "its old place is transparent on this page too");

    // It goes: direct alone again, the overlay off.
    let before = rig.taken();
    rig.osd.set(hidden());
    rig.commit();
    let now = rig.taken();
    assert_eq!(&now[before.len()..], [direct()], "{now:?}");

    // Back: the UI ring forgot its pages while the overlay was off, so the page is drawn whole (the stale layer at
    // 200,100 from two frames ago must not reappear).
    let before = rig.taken();
    rig.osd.set(Props::at(760.0, 900.0));
    rig.commit();
    let now = rig.taken();
    assert_eq!(&now[before.len()..], [split(2)], "{now:?}");
    let image = rig.capture();
    assert_eq!(image.pixel(780, 920), [245, 245, 255, 255]);
    assert_eq!(image.pixel(220, 120), [0, 0, 0, 0], "a page the ring forgot holds nothing stale");

    // The card goes: Home composed again into the primary ring, drawn whole, with the system layer on it.
    drop(_client);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !matches!(rig.taken().last(), Some(Assignment { primary: Source::Page(_), overlay: None, .. })) {
        assert!(Instant::now() < deadline, "the frame after the card went: {:?}", rig.taken());
        std::thread::sleep(Duration::from_millis(5));
    }
    rig.under.set(Props::at(600.0, 500.0));
    rig.commit();
    let now = rig.taken();
    assert!(matches!(now.last(), Some(Assignment { primary: Source::Page(_), overlay: None, .. })), "{now:?}");
    let image = rig.capture();
    assert_eq!(image.pixel(780, 920), [245, 245, 255, 255], "the system layer is composed with Home");
    assert_eq!(image.pixel(100, 100), [18, 20, 32, 255], "Home is drawn whole");
    assert_eq!(rig.gl_errors(), 0);
}

#[test]
fn a_ui_commit_wakes_the_loop_and_is_drawn_without_a_tick() {
    let Some(rig) = Rig::start("wake", false) else { return };
    // No tick at all: a commit is drawn only if its socket wakes the loop and the still screen is drawn at once.
    rig.server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    let mut latencies: Vec<Duration> = (0..30)
        .map(|k| {
            rig.under.set(Props::at(500.0 + k as f32 * 10.0, 500.0));
            let began = Instant::now();
            rig.commit();
            began.elapsed()
        })
        .collect();
    latencies.sort();
    let (median, worst) = (latencies[latencies.len() / 2], latencies[latencies.len() - 1]);
    eprintln!("UI commit to its receipt, no tick: median {:.2} ms, worst {:.2} ms", median.as_secs_f64() * 1e3, worst.as_secs_f64() * 1e3);
    assert_eq!(rig.taken().len(), 30, "one frame a commit");
    assert!(worst < Duration::from_millis(50), "worst {worst:?}");
    assert_eq!(rig.capture().pixel(500 + 290 + 50, 550), [90, 200, 90, 255], "the last commit is on the page");
    assert_eq!(rig.gl_errors(), 0);
}

/// What the clock card's hands layer is made of, as the shell redraws it each second: a thick opaque stroke from the centre of a `side`
/// square at `degrees` from twelve o'clock, on nothing.
fn hand_picture(side: u32, degrees: f32) -> Vec<u8> {
    let (c, a) = (side as f32 / 2.0, degrees.to_radians());
    let (dx, dy) = (a.sin(), -a.cos());
    let mut px = vec![0u8; (side * side * 4) as usize];
    for y in 0..side {
        for x in 0..side {
            let (rx, ry) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let along = rx * dx + ry * dy;
            let across = (rx * dy - ry * dx).abs();
            if (0.0..c - 2.0).contains(&along) && across <= 1.5 {
                let i = ((y * side + x) * 4) as usize;
                px[i..i + 4].copy_from_slice(&[255, 59, 92, 255]);
            }
        }
    }
    px
}

#[test]
fn a_picture_redrawn_whole_in_place_every_frame_leaves_nothing_of_the_last_one_on_any_page() {
    let Some(rig) = Rig::start("hand", false) else { return };
    const SIDE: u32 = 166;
    let (x0, y0) = (800i32, 200i32);
    let mut hand = rig.ui.scene.image(SIDE, SIDE, 5, Props::at(x0 as f32, y0 as f32)).unwrap();
    rig.commit();
    // Two turns of the ring's three pages and more, a step of the second hand each; another layer of Home moves in the same frames, as the
    // wallpaper does, so that a frame owes two rectangles.
    for step in 0..14u32 {
        let want = hand_picture(SIDE, step as f32 * 6.0);
        hand.pixels_mut().copy_from_slice(&want);
        hand.damage(None);
        rig.under.set(Props::at(500.0 + step as f32 * 7.0, 500.0));
        rig.commit();
        let image = rig.capture();
        let mut wrong = Vec::new();
        for y in 0..SIDE {
            for x in 0..SIDE {
                let i = ((y * SIDE + x) * 4) as usize;
                let expect = if want[i + 3] > 0 { [want[i], want[i + 1], want[i + 2], 255] } else { [18, 20, 32, 255] };
                let got = image.pixel((x0 as u32) + x, (y0 as u32) + y);
                if got != expect {
                    wrong.push((x, y, got, expect));
                }
            }
        }
        assert!(wrong.is_empty(), "step {step}: {} of the hand's pixels differ from its picture, the first {:?}", wrong.len(), &wrong[..wrong.len().min(3)]);
    }
    assert_eq!(rig.gl_errors(), 0);
}

#[test]
fn a_page_that_copies_what_it_owes_from_the_last_page_holds_the_frame_whole() {
    let Some(rig) = Rig::start("copy", false) else { return };
    rig.commit();
    // Steps of 160 px: each frame's change is two squares apart from the last one's, so a page copies the places the layer was two
    // and three frames before from the page drawn last and draws only where it is and where it just was.
    let place = |step: u32| (100 + step * 160, 500u32);
    for step in 0..10u32 {
        let (x, y) = place(step);
        rig.under.set(Props::at(x as f32, y as f32));
        rig.commit();
        let image = rig.capture();
        assert_eq!(image.pixel(x + 50, y + 50), [90, 200, 90, 255], "step {step}: the layer where it is");
        for back in 1..=3u32.min(step) {
            let (bx, by) = place(step - back);
            assert_eq!(image.pixel(bx + 50, by + 50), [18, 20, 32, 255], "step {step}: nothing left where it was {back} frames before");
        }
    }
    assert_eq!(rig.gl_errors(), 0);
}
