//! The KMS output's frame logic (`kms::frames`) with the UI engine attached, over pages in the context's own memory and under
//! the real `State::tick` (feature `ui`, Mesa's software GL): direct scan-out of a full-screen opaque card while the UI shows
//! nothing over it, composition the frame a system layer appears, and the UI app's socket waking the loop.
#![cfg(feature = "ui")]

mod harness;

use compositor_protocol::{Event, Fill, Props, Rgba, Shape};
use harness::{Client, Server};
use lumashow_ui::{Scene, SceneOptions, ShapeLayer, Ui};
use lumatv_compositor::kms::assign::Source;
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

/// The KMS output without the device: the GL renderer, three pages of its own and the ring's frame logic. Every frame's source
/// is kept for the test to read. Between frames the screen is still, as KMS answers when no flip is in flight.
struct ScanOut {
    gl: Gl,
    frames: Frames,
    pages: Vec<TargetId>,
    sources: Arc<Mutex<Vec<Source>>>,
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
        let direct = plan.sole_fullscreen().map(|_| FB);
        let (source, stats) = self.frames.draw(&mut self.gl, plan, surfaces, direct)?;
        self.sources.lock().push(source);
        Ok(stats)
    }

    fn present(&mut self) -> io::Result<Presented> {
        self.gl.present()
    }

    /// The page last composed (what the panel would show after a composed frame).
    fn capture(&mut self) -> io::Result<Image> {
        self.gl.set_target(self.pages[self.frames.last_page()]);
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
    sources: Arc<Mutex<Vec<Source>>>,
    shm: PathBuf,
    under: ShapeLayer,
    osd: ShapeLayer,
    _ground: ShapeLayer,
}

fn hidden() -> Props {
    Props { visible: false, ..Props::at(760.0, 900.0) }
}

impl Rig {
    /// `None` without Mesa.
    fn start(name: &str) -> Option<Rig> {
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
            let frames = Frames::new(pages.clone(), 1920, 1080);
            Box::new(ScanOut { gl, frames, pages, sources: log })
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

    fn taken(&self) -> Vec<Source> {
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

#[test]
fn a_full_screen_window_is_scanned_out_directly_unless_the_ui_shows_something_over_it() {
    let Some(mut rig) = Rig::start("direct") else { return };

    // Home alone: composed.
    rig.commit();
    assert_eq!(rig.taken(), [Source::Page(0)]);
    assert_eq!(rig.capture().pixel(600, 400), [18, 20, 32, 255]);

    // A full-screen opaque card over it: the plane takes the card's buffer, the pages are left as they were.
    let mut client = Client::connect(&mut rig.server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Player");
    let opaque = client.region(0, 0, 1920, 1080);
    card.surface.set_opaque_region(Some(&opaque));
    let buffer = client.shm_buffer(1920, 1080, 0xff19_80e6);
    client.present(&card.surface, &buffer);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    let before = rig.taken();
    assert_eq!(before.last(), Some(&Source::Direct(FB)), "{before:?}");
    assert_eq!(rig.capture().pixel(600, 400), [18, 20, 32, 255], "no page was composed for the card");

    // Home changes under the card: the app's commit is taken in and answered, and the frame is still direct.
    rig.under.set(Props::at(700.0, 500.0));
    rig.commit();
    let now = rig.taken();
    assert!(now.len() > before.len() && now[before.len()..].iter().all(|s| *s == Source::Direct(FB)), "{now:?}");

    // A system layer appears: that frame is composed, and drawn whole (the card's frames never reached the pages).
    let before = rig.taken();
    rig.osd.set(Props::at(760.0, 900.0));
    rig.commit();
    let now = rig.taken();
    assert!(matches!(now[before.len()..], [Source::Page(_)]), "{now:?}");
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
    assert_eq!(&now[before.len()..], [Source::Direct(FB)], "{now:?}");
    assert_eq!(rig.gl_errors(), 0);
}

#[test]
fn a_ui_commit_wakes_the_loop_and_is_drawn_without_a_tick() {
    let Some(rig) = Rig::start("wake") else { return };
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
