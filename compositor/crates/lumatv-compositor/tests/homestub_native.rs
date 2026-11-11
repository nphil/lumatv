//! The native Home stub (`homestub/homestub.c`, built by `homestub/build.sh`) as a client of the real server: the compositor takes the one
//! transparent pixel for LG's Home card, as it takes the web stub's page. The stub's own side (the wire, the idle, how it ends) is
//! `tests/test_homestub_native.py`; what is shown here is what the compositor makes of it. Skipped, with a line saying so, where the host
//! build of the stub is not there.

mod harness;

use std::os::fd::IntoRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::luna::{Event, ToBridge};
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::server::webos_shell::front_app;
use lumatv_compositor::window::{ShellState, WindowType};
use serde_json::json;
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

/// The stub on a connection of the harness's, as libwayland clients are handed one (`WAYLAND_SOCKET`): its first one, then extra ones.
fn start_stub(server: &mut Server, binary: &Path, launch: Option<&str>) -> Child {
    let stream = server.client_stream.take().unwrap_or_else(|| server.extra_stream());
    let fd = stream.into_raw_fd();
    // Rust opens it close-on-exec; the child is to have it.
    unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };
    // The name its one-card lock goes by is machine-wide: one of its own, or the tests that run at once would end each other's stubs.
    static STUBS: AtomicU32 = AtomicU32::new(0);
    let lock = format!("lumatv-homestub-test-{}-{}", std::process::id(), STUBS.fetch_add(1, Ordering::SeqCst));
    let mut command = Command::new(binary);
    command.env_clear().env("WAYLAND_SOCKET", fd.to_string()).env("LUMATV_HOMESTUB_SINGLE", lock);
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
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

// ---- the Home key and SAM's launches of Home: the compositor raises the stub's card itself (`home_stub.rs`) ----
//
// On the set (2026-11-05) the Home key made SAM end the stub and start another, 0.77 s from its launch call to the foreground change at the
// audio service, and a `luna-send launch` of Home over an app did nothing at all.

const KEY_HOME: u32 = lumatv_compositor::input::code::KEY_RF_HOME;

/// A tap of the Home key, as the remote's own or the network's.
fn tap(s: &mut State, network: bool) {
    s.key_event(SeatId(0), KEY_HOME, true, network);
    s.key_event(SeatId(0), KEY_HOME, false, network);
}

/// How many calls to SAM that launch an app the compositor has made.
fn sam_launches(s: &State) -> usize {
    s.luna.sent().iter().filter(|(uri, _)| uri.ends_with("/launchDefaultApp") || uri.ends_with("/applicationManager/launch")).count()
}

/// What the compositor has of the card of `app`: its state and whether the stub holds it.
fn card(s: &State, app: &str) -> Option<(ShellState, bool)> {
    s.scene.windows().find(|w| w.app_id() == app).map(|w| (w.state, w.held_by_home))
}

/// One event of SAM's life subscription, as `getAppLifeEvents` sends it.
fn life(server: &Server, payload: serde_json::Value) {
    fn token(s: &State) -> Option<u64> {
        s.luna.history().iter().find_map(|m| match m {
            ToBridge::Outbound(o) if o.uri.ends_with("/getAppLifeEvents") && o.subscribe => Some(o.token),
            _ => None,
        })
    }
    server.eval(move |s| {
        if token(s).is_none() {
            s.luna_register(); // what the bridge's coming up does, on the set
        }
        let token = token(s).expect("the compositor subscribed to SAM's life events");
        s.luna_event(Event { token, payload });
    });
}

fn event(name: &str, app: &str) -> serde_json::Value {
    json!({"event": name, "appId": app, "returnValue": true, "reason": "undefined"})
}

/// The stub's card mapped and an app's over it: what a launch from Home leaves.
fn stub_under_app(server: &mut Server, binary: &Path) -> (Child, App) {
    let child = start_stub(server, binary, None);
    wait_for(server, "Home's card mapped", home_mapped);
    let app = app_in_front(server);
    assert_eq!(front(server).as_deref(), Some(APP));
    (child, app)
}

/// The stub's card as old as SAM's relaunch guard has let it get: SAM sends no launch of Home for a stub it started under about 3 s before.
fn age_the_stub(server: &Server) {
    server.eval(|s| {
        let home = s.scene.windows().find(|w| w.app_id() == HOME).map(|w| w.id).expect("Home's window");
        s.scene.window_mut(home).unwrap().created -= Duration::from_secs(10);
    });
}

#[test]
fn the_home_key_raises_the_stub_in_its_own_pass_and_sam_is_not_asked() {
    let Some(binary) = stub_binary() else { return };
    let mut server = Server::start();
    let (mut child, _app) = stub_under_app(&mut server, &binary);

    let (took, now_front, home, app, asked, journal) = server.eval(|s| {
        let started = Instant::now();
        tap(s, false);
        let took = started.elapsed();
        let front = s.scene.foreground(&s.surfaces).map(|w| w.app_id().to_string());
        (took, front, card(s, HOME), card(s, APP), sam_launches(s), s.journal.clone())
    });
    assert_eq!(now_front.as_deref(), Some(HOME), "Home is in front when the key's pass is over: {journal:?}");
    assert_eq!(home, Some((ShellState::Fullscreen, false)));
    assert_eq!(app, Some((ShellState::Minimized, true)), "the app is covered, and stays so when the stub's card goes");
    assert_eq!(asked, 0, "SAM is not asked: it would end the stub and start another");
    assert!(journal.iter().any(|l| l == "input: Home: the stub's card raised, SAM not asked"), "{journal:?}");
    assert!(took < Duration::from_millis(50), "{took:?}");
    assert!(child.try_wait().unwrap().is_none(), "the same process, still there");

    // pressed again on Home: nothing to raise, and still no word to SAM
    let (now_front, asked, journal) = server.eval(|s| {
        tap(s, true);
        (front_of(s), sam_launches(s), s.journal.clone())
    });
    assert_eq!((now_front.as_deref(), asked), (Some(HOME), 0));
    assert!(journal.iter().any(|l| l == "input: Home (network key): Home's card is in front, SAM not asked"), "{journal:?}");
    child.kill().unwrap();
    child.wait().unwrap();
}

fn front_of(s: &State) -> Option<String> {
    s.scene.foreground(&s.surfaces).map(|w| w.app_id().to_string())
}

/// The key to Home in front, in the compositor, 200 times over an app that is brought back each time. A TV run reads the same on the set from
/// the probe (`tools/homestub-native-tv.sh`), and SAM's relaunch took 0.77 s there.
#[test]
fn the_home_key_to_home_in_front_is_measured() {
    let Some(binary) = stub_binary() else { return };
    let mut server = Server::start();
    let (mut child, _app) = stub_under_app(&mut server, &binary);
    let mut times = server.eval(|s| {
        let mut times = Vec::new();
        for _ in 0..200 {
            front_app(s, APP);
            assert_eq!(front_of(s).as_deref(), Some(APP));
            let started = Instant::now();
            tap(s, false);
            let took = started.elapsed();
            assert_eq!(front_of(s).as_deref(), Some(HOME));
            times.push(took);
        }
        times
    });
    times.sort();
    let (median, worst) = (times[times.len() / 2], times[times.len() - 1]);
    eprintln!("Home key to Home in front, {} presses over an app: median {median:?}, worst {worst:?}", times.len());
    assert!(worst < Duration::from_millis(20), "{worst:?}");
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn sams_launch_of_home_raises_the_stub_and_a_preload_or_another_app_does_not() {
    let Some(binary) = stub_binary() else { return };
    let mut server = Server::start();
    let (mut child, _app) = stub_under_app(&mut server, &binary);

    let relaunch_preload = json!({"event": "relaunch", "appId": HOME, "parameters": {"preload": "full"}, "returnValue": true});
    for not_a_launch in [event("preload", HOME), event("splash", HOME), event("foreground", HOME), relaunch_preload, event("launch", APP)] {
        life(&server, not_a_launch.clone());
        assert_eq!(front(&server).as_deref(), Some(APP), "{not_a_launch}");
    }
    life(&server, json!({"returnValue": true, "subscribed": true}));
    assert_eq!(front(&server).as_deref(), Some(APP), "the subscription's own answer");

    // SAM's launch of Home for a stub past its relaunch guard, which it ends and starts again: the card there now is raised at once
    age_the_stub(&server);
    life(&server, event("launch", HOME));
    let (home, app, asked, journal) = server.eval(|s| (card(s, HOME), card(s, APP), sam_launches(s), s.journal.clone()));
    assert_eq!(front(&server).as_deref(), Some(HOME));
    assert_eq!((home, app), (Some((ShellState::Fullscreen, false)), Some((ShellState::Minimized, true))));
    assert_eq!(asked, 0, "SAM is not asked again for what it launched");
    assert!(journal.iter().any(|l| l == "home: SAM said launch for Home: the stub's card raised"), "{journal:?}");

    // the Home key's relaunch, with its parameters, over the app again
    server.eval(|s| front_app(s, APP));
    assert_eq!(front(&server).as_deref(), Some(APP));
    life(&server, json!({"event": "relaunch", "appId": HOME, "reason": "undefined", "parameters": {"launchType": "homeKey", "activateType": "home"}, "returnValue": true}));
    assert_eq!(front(&server).as_deref(), Some(HOME));

    // Home in front already: said and left as it is
    let journal = server.eval(|s| s.journal.clone());
    life(&server, event("launch", HOME));
    let after = server.eval(|s| s.journal.clone());
    assert!(after.iter().any(|l| l == "home: SAM said launch for Home: its card is in front"), "{after:?}");
    assert_eq!(front(&server).as_deref(), Some(HOME));
    assert!(after.len() > journal.len());
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn the_app_under_the_stub_stays_covered_while_sam_ends_the_stub_and_starts_another() {
    let Some(binary) = stub_binary() else { return };
    let mut server = Server::start();
    let (mut child, _app) = stub_under_app(&mut server, &binary);
    server.eval(|s| tap(s, false));
    assert_eq!(front(&server).as_deref(), Some(HOME));

    // SAM ends it (the signal it ends a native app with) and the card goes: the app is not brought back for the moments before the next one
    child.kill().unwrap();
    child.wait().unwrap();
    wait_for(&server, "Home's card gone", |s| s.scene.windows().all(|w| w.app_id() != HOME));
    let (front_now, app) = server.eval(|s| (front_of(s), card(s, APP)));
    assert_eq!(front_now, None, "no card is in front: the shell's Home is what is on the screen");
    assert_eq!(app, Some((ShellState::Minimized, true)));

    // the new one maps and is in front; the app is where it was
    let mut next = start_stub(&mut server, &binary, None);
    wait_for(&server, "Home's card mapped again", home_mapped);
    assert_eq!(front(&server).as_deref(), Some(HOME));
    assert_eq!(server.eval(|s| card(s, APP)), Some((ShellState::Minimized, true)));

    // and an app that is asked for comes back as ever: the held card is raised by a launch, not by the stub's going
    server.eval(|s| front_app(s, APP));
    assert_eq!(front(&server).as_deref(), Some(APP));
    assert_eq!(server.eval(|s| card(s, APP)), Some((ShellState::Fullscreen, false)));
    next.kill().unwrap();
    next.wait().unwrap();
}

/// The race seen on the set (2026-11-11): SAM's `launch` event for the stub it had restarted was handled after the new card had mapped and a
/// newer launch of HDMI 2 had raised that app over it, and Home was raised over the app opened last.
#[test]
fn a_launch_event_for_the_stub_just_restarted_does_not_raise_it_over_the_app_opened_since() {
    let Some(binary) = stub_binary() else { return };
    let mut server = Server::start();
    let (mut child, _app) = stub_under_app(&mut server, &binary);

    // SAM ends the stub and starts another, which maps and takes the front; then the app's launch raises the app
    child.kill().unwrap();
    child.wait().unwrap();
    wait_for(&server, "Home's card gone", |s| s.scene.windows().all(|w| w.app_id() != HOME));
    let mut next = start_stub(&mut server, &binary, None);
    wait_for(&server, "Home's card mapped again", home_mapped);
    server.eval(|s| front_app(s, APP));
    assert_eq!(front(&server).as_deref(), Some(APP));

    // the restart's own `launch` event, only now
    life(&server, event("launch", HOME));
    let (app, journal) = server.eval(|s| (card(s, APP), s.journal.clone()));
    assert_eq!(front(&server).as_deref(), Some(APP), "{journal:?}");
    assert_eq!(app, Some((ShellState::Fullscreen, false)));
    let ignored = "home: SAM said launch for Home: the stub's card is new and under an app, so the event is old: nothing raised";
    assert!(journal.iter().any(|l| l == ignored), "{journal:?}");
    next.kill().unwrap();
    next.wait().unwrap();
}

/// LG's Home and the web stub do not say they are the native stub: the key goes to SAM as it always did, and their going brings the app back.
#[test]
fn a_home_card_that_is_not_the_stub_is_left_to_sam() {
    let mut server = Server::start();
    let mut home_client = Client::connect_extra(&mut server);
    let home_window = home_client.card("_WEBOS_WINDOW_TYPE_CARD", "Home");
    home_window.webos.set_property("appId".into(), HOME.into());
    let home_buffer = home_client.shm_buffer(1920, 1080, 0xff00_00ff);
    home_client.present(&home_window.surface, &home_buffer);
    assert!(home_client.wait_until(WAIT, |s| s.frames_done >= 1));
    let app = app_in_front(&mut server);
    assert_eq!(front(&server).as_deref(), Some(APP));

    let (now_front, asked, journal) = server.eval(|s| {
        tap(s, false);
        (front_of(s), sam_launches(s), s.journal.clone())
    });
    assert_eq!(now_front.as_deref(), Some(APP), "nothing is raised here: SAM launches Home, and Home asks for the front itself");
    assert_eq!(asked, 1);
    assert!(journal.iter().any(|l| l == "input: Home: launchDefaultApp"), "{journal:?}");
    life(&server, event("launch", HOME));
    assert_eq!(front(&server).as_deref(), Some(APP), "a launch event is no business of the compositor's either");

    // Home in front over the app, and gone: the app comes back, as the card below always did
    server.eval(|s| front_app(s, HOME));
    assert_eq!(front(&server).as_deref(), Some(HOME));
    drop((home_window, home_buffer, home_client));
    wait_for(&server, "Home's card gone", |s| s.scene.windows().all(|w| w.app_id() != HOME));
    assert_eq!(front(&server).as_deref(), Some(APP));
    assert_eq!(server.eval(|s| card(s, APP)), Some((ShellState::Fullscreen, false)));
    drop(app);
}
