//! Direct scan-out with the UI engine attached (feature `ui`, Mesa's software GL): the KMS output's frame logic (`kms::frames`)
//! over pages in the context's own memory, under the real `State::tick`. A full-screen opaque card goes to the plane directly
//! while the UI shows nothing over it (its Home under the card is covered), the frame a system layer appears is composed and
//! drawn whole (the pages missed the card's frames), and the frame it goes is direct again. The app's commits are taken in and
//! answered either way.
#![cfg(feature = "ui")]

mod harness;

use compositor_protocol::{Event, Fill, Props, Rgba, Shape};
use harness::{Client, Server};
use lumashow_ui::{Scene, SceneOptions, Ui};
use lumatv_compositor::kms::assign::Source;
use lumatv_compositor::kms::frames::Frames;
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl, TargetId};
use lumatv_compositor::scene::Plan;
use lumatv_compositor::surface::Surfaces;
use parking_lot::Mutex;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wayland_server::protocol::wl_buffer::WlBuffer;

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";
const FB: u32 = 7;

/// The KMS output without the device: the GL renderer, three pages of its own and the ring's frame logic. Every frame's source
/// is kept for the test to read.
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
}

#[test]
fn a_full_screen_window_is_scanned_out_directly_unless_the_ui_shows_something_over_it() {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return;
    }
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let shm = std::env::temp_dir().join(format!("lumatv-ui-scanout-{}", std::process::id()));
    std::fs::create_dir_all(&shm).unwrap();
    let dir = shm.clone();
    let sources = Arc::new(Mutex::new(Vec::new()));
    let log = sources.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut server = Server::start_with(Box::new(move || {
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
    let _ground = ui.scene.shape(1920, 1080, 0, solid(Rgba(18, 20, 32, 255)), Props::at(0.0, 0.0));
    let under = ui.scene.shape(100, 100, 1, solid(Rgba(90, 200, 90, 255)), Props::at(500.0, 500.0));
    let hidden = Props { visible: false, ..Props::at(760.0, 900.0) };
    let osd = ui.scene.shape(400, 60, 1000, solid(Rgba(245, 245, 255, 255)), hidden);
    let wait = |tag| {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let event = events.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("a presented UI frame");
            ui.scene.observe(&event);
            if matches!(event, Event::Frame { tag: got, .. } if got >= tag) {
                break;
            }
        }
    };
    let taken = || sources.lock().clone();

    // Home alone: composed.
    wait(ui.try_commit().unwrap().unwrap());
    assert_eq!(taken(), [Source::Page(0)]);
    let image = server.eval(|s| s.output.capture().unwrap());
    assert_eq!(image.pixel(600, 400), [18, 20, 32, 255]);

    // A full-screen opaque card over it: the plane takes the card's buffer, the pages are left as they were.
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Player");
    let opaque = client.region(0, 0, 1920, 1080);
    card.surface.set_opaque_region(Some(&opaque));
    let buffer = client.shm_buffer(1920, 1080, 0xff19_80e6);
    client.present(&card.surface, &buffer);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    let before = taken();
    assert_eq!(before.last(), Some(&Source::Direct(FB)), "{before:?}");
    assert_eq!(server.eval(|s| s.output.capture().unwrap()).pixel(600, 400), [18, 20, 32, 255], "no page was composed for the card");

    // Home changes under the card: the app's commit is taken in and answered, and the frame is still direct.
    under.set(Props::at(700.0, 500.0));
    wait(ui.try_commit().unwrap().unwrap());
    let now = taken();
    assert!(now.len() > before.len() && now[before.len()..].iter().all(|s| *s == Source::Direct(FB)), "{now:?}");

    // A system layer appears: that frame is composed, and drawn whole (the card's frames never reached the pages).
    let before = taken();
    osd.set(Props::at(760.0, 900.0));
    wait(ui.try_commit().unwrap().unwrap());
    let now = taken();
    assert!(matches!(now[before.len()..], [Source::Page(_)]), "{now:?}");
    let (image, errors) = server.eval(|s| (s.output.capture().unwrap(), s.stats.gl_errors));
    assert_eq!(image.pixel(780, 920), [245, 245, 255, 255], "the system layer is on the page");
    assert_eq!(image.pixel(600, 400), [25, 128, 230, 255], "the card is on the page, not the stale Home");
    assert_eq!(image.pixel(30, 30), [25, 128, 230, 255]);
    assert_eq!(image.pixel(1900, 1070), [25, 128, 230, 255]);
    assert_eq!(errors, 0);

    // It goes: direct again, in the same frame.
    let before = taken();
    osd.set(hidden);
    wait(ui.try_commit().unwrap().unwrap());
    let now = taken();
    assert_eq!(&now[before.len()..], [Source::Direct(FB)], "{now:?}");
    assert_eq!(server.eval(|s| s.stats.gl_errors), 0);
    drop(server);
    let _ = std::fs::remove_dir_all(&shm);
}
