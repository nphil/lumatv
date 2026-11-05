//! The native Home stub (`homestub/homestub.c`, built by `homestub/build.sh`) as a client of the real server: the compositor takes the one
//! transparent pixel for LG's Home card, as it takes the web stub's page. The stub's own side (the wire, the idle, how it ends) is
//! `tests/test_homestub_native.py`; what is shown here is what the compositor makes of it. Skipped, with a line saying so, where the host
//! build of the stub is not there.

mod harness;

use std::os::fd::IntoRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::window::{ShellState, WindowType};
use wayland_client::protocol::wl_buffer::WlBuffer;

const WAIT: Duration = Duration::from_secs(3);
const HOME: &str = "com.webos.app.home";
const APP: &str = "com.netflix.ninja";

fn stub_binary() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../homestub/build/host/lumatv-homestub");
    if path.exists() {
        Some(path)
    } else {
        eprintln!("homestub/build/host/lumatv-homestub is not built (sh homestub/build.sh): skipped");
        None
    }
}

/// The stub on the harness's own socket pair, as libwayland clients are handed one (`WAYLAND_SOCKET`).
fn start_stub(server: &mut Server, binary: &Path, launch: Option<&str>) -> Child {
    let fd = server.client_stream.take().expect("the harness's one connection").into_raw_fd();
    // Rust opens it close-on-exec; the child is to have it.
    unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };
    let mut command = Command::new(binary);
    command.env_clear().env("WAYLAND_SOCKET", fd.to_string()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    if let Some(argument) = launch {
        command.arg(argument);
    }
    let child = command.spawn().expect("the stub starts");
    unsafe { libc::close(fd) };
    child
}

fn wait_for(server: &Server, what: &str, ready: impl FnMut(&mut State) -> bool + Send + Clone + 'static) {
    let end = Instant::now() + WAIT;
    while Instant::now() < end {
        if server.eval(ready.clone()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("not seen in {WAIT:?}: {what}");
}

fn home_mapped(s: &mut State) -> bool {
    s.scene.windows().any(|w| w.app_id() == HOME && w.map_serial > 0)
}

#[test]
fn the_stub_is_lgs_home_card_in_front_and_nothing_to_look_at() {
    let Some(binary) = stub_binary() else { return };
    let mut server = Server::start();
    let mut child = start_stub(&mut server, &binary, None);
    wait_for(&server, "Home's card mapped", home_mapped);

    let (kind, state, mapped, size, front, picture) = server.eval(|s| {
        let w = s.scene.windows().find(|w| w.app_id() == HOME).expect("Home's window");
        let surface = s.surfaces.get(w.surface).unwrap();
        let front = s.scene.foreground(&s.surfaces).map(|f| f.app_id().to_string());
        (w.kind, w.state, surface.is_mapped(), surface.current.size, front, harness::drawn(s))
    });
    assert_eq!(kind, WindowType::Card);
    assert_eq!(state, ShellState::Fullscreen, "a launch asks for the front, and the compositor answers that it has it");
    assert!(mapped);
    assert_eq!(size, Some((1, 1)), "one pixel: nothing for the compositor to hold or draw");
    assert_eq!(front.as_deref(), Some(HOME), "SAM's foreground answer names Home");
    for (x, y) in [(0, 0), (960, 540), (1919, 1079)] {
        assert_eq!(picture.pixel(x, y), [0, 0, 0, 0xff], "transparent over the opaque black the output is without a window");
    }

    // closeByAppId: the stub is asked to close, ends cleanly, and Home's card goes with its connection
    assert_eq!(server.eval(|s| s.close_app(HOME)), 1);
    let end = Instant::now() + WAIT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < end, "the stub did not end on the compositor's close");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(status.success(), "{status:?}");
    wait_for(&server, "Home's card gone", |s| s.scene.windows().all(|w| w.app_id() != HOME));
}

/// An app's card, presented and in front, for as long as the value lives.
struct App {
    _client: Client,
    _window: Window,
    _buffer: WlBuffer,
}

fn app_in_front(server: &mut Server) -> App {
    let mut client = Client::connect_extra(server);
    let window = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    window.webos.set_property("appId".into(), APP.into());
    let buffer = client.shm_buffer(1920, 1080, 0xffff_0000);
    client.present(&window.surface, &buffer);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    App { _client: client, _window: window, _buffer: buffer }
}

fn front(server: &Server) -> Option<String> {
    server.eval(|s| s.scene.foreground(&s.surfaces).map(|w| w.app_id().to_string()))
}

#[test]
fn a_launch_puts_home_over_the_app_and_a_preload_does_not_even_map() {
    let Some(binary) = stub_binary() else { return };
    // A card that gets its first buffer is at the front: so a preload's window has none, and the app on screen keeps the screen.
    for (launch, mapped) in [(None, true), (Some(r#"{"reason":"preload","appId":"com.webos.app.home"}"#), false)] {
        let mut server = Server::start();
        let _app = app_in_front(&mut server);
        assert_eq!(front(&server).as_deref(), Some(APP));
        let mut child = start_stub(&mut server, &binary, launch);
        wait_for(&server, "Home's window made", |s| s.scene.windows().any(|w| w.app_id() == HOME));
        if mapped {
            wait_for(&server, "Home's card mapped", home_mapped);
        } else {
            std::thread::sleep(Duration::from_millis(100));
        }
        let (home_state, home_map_serial, app_state) = server.eval(|s| {
            let home = s.scene.windows().find(|w| w.app_id() == HOME).expect("Home's window");
            let app = s.scene.windows().find(|w| w.app_id() == APP).expect("the app's window");
            (home.state, home.map_serial, app.state)
        });
        let (want_front, want_app) = if mapped { (HOME, ShellState::Minimized) } else { (APP, ShellState::Fullscreen) };
        assert_eq!(front(&server).as_deref(), Some(want_front), "launch {launch:?}");
        assert_eq!(app_state, want_app, "launch {launch:?}");
        assert_eq!(home_map_serial > 0, mapped, "launch {launch:?}");
        if !mapped {
            assert_ne!(home_state, ShellState::Fullscreen, "it asked for nothing");
        }
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
