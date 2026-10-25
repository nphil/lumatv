//! The UI engine on the compositor's live drawing path (feature `ui`, Mesa's software GL): `Gl` with a `Ui` attached, under the
//! real `State::tick`. A Home ground and a system panel from an app over the socket, a card window between them, and a move of
//! the panel alone, which must wake the output and redraw only the panel's tiles (the window under them comes back as it was).
//! When the app hangs up, its layers go and the output is drawn whole without them.
#![cfg(feature = "ui")]

mod harness;

use compositor_protocol::{Event, Fill, Props, Rgba, Shape};
use harness::{Client, Server};
use lumashow_ui::{Scene, SceneOptions, Ui};
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl};
use std::path::Path;
use std::time::{Duration, Instant};

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";

#[test]
fn system_layers_survive_window_composition_and_redraw_without_window_damage() {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return;
    }
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let shm = std::env::temp_dir().join(format!("lumatv-ui-output-{}", std::process::id()));
    std::fs::create_dir_all(&shm).unwrap();
    let dir = shm.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut server = Server::start_with(Box::new(move || {
        let mut gl = Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap();
        tx.send(gl.attach_ui(MESA, 1000, dir).unwrap()).unwrap();
        gl.ui().unwrap().finish_handshake().unwrap();
        Box::new(gl)
    }));
    let app_end = rx.recv().unwrap();
    // The app's socket, to hang up with: its reader thread keeps a copy of it, so dropping the scene alone would not.
    let hangup = app_end.try_clone().unwrap();
    let (scene, _, events) = Scene::over_with(app_end, SceneOptions { shm_dir: Some(shm.clone()), ..Default::default() }).unwrap();
    let ui = Ui::new(scene);
    let solid = |colour| Shape { corner_radius: 0.0, fill: Fill::Solid(colour), border_width: 0.0, border: Rgba(0, 0, 0, 0) };
    let _ground = ui.scene.shape(1920, 1080, 0, solid(Rgba(18, 20, 32, 255)), Props::at(0.0, 0.0));
    let panel = ui.scene.shape(100, 80, 1000, solid(Rgba(220, 40, 60, 255)), Props::at(20.0, 20.0));
    let first = ui.try_commit().unwrap().unwrap();
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
    wait(first);
    let image = server.eval(|s| s.output.capture().unwrap());
    assert_eq!(image.pixel(600, 400), [18, 20, 32, 255], "the first frame, with no window, draws Home");
    assert_eq!(image.pixel(30, 30), [220, 40, 60, 255], "and the system panel over it");

    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Test app");
    let buffer = client.shm_buffer(300, 200, 0xff19_80e6);
    client.present(&card.surface, &buffer);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    let image = server.eval(|s| s.output.capture().unwrap());
    assert_eq!(image.pixel(30, 30), [220, 40, 60, 255], "the system panel is above the app");
    assert_eq!(image.pixel(200, 100), [25, 128, 230, 255], "the app is above Home");
    assert_eq!(image.pixel(600, 400), [18, 20, 32, 255], "Home is preserved outside the app");

    panel.set(Props::at(400.0, 300.0));
    let moved = ui.try_commit().unwrap().unwrap();
    wait(moved);
    let (image, errors) = server.eval(|s| (s.output.capture().unwrap(), s.stats.gl_errors));
    assert_eq!(image.pixel(30, 30), [25, 128, 230, 255], "moving the panel uncovers the unchanged app");
    assert_eq!(image.pixel(410, 310), [220, 40, 60, 255], "UI-only damage wakes the output");
    assert_eq!(image.pixel(600, 400), [18, 20, 32, 255]);
    assert_eq!(errors, 0);

    hangup.shutdown(std::net::Shutdown::Both).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let (image, errors) = loop {
        let (image, errors) = server.eval(|s| (s.output.capture().unwrap(), s.stats.gl_errors));
        if image.pixel(410, 310) != [220, 40, 60, 255] || Instant::now() > deadline {
            break (image, errors);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(image.pixel(410, 310), [0, 0, 0, 255], "the app hung up: its panel is gone");
    assert_eq!(image.pixel(600, 400), [0, 0, 0, 255], "and its Home with it");
    assert_eq!(image.pixel(200, 100), [25, 128, 230, 255], "the window stays");
    assert_eq!(errors, 0);
    drop(server);
    let _ = std::fs::remove_dir_all(&shm);
}
