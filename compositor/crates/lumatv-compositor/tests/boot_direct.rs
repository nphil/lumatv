//! Boot direct (`boot_direct.rs`) under the real `State`, with the shell client played by messages, the apps by windows and `tools/lumatv`
//! by a script that writes down what it was asked: a Home hold opens LG's Home through the tool, the native Home steps aside while LG's
//! is in front, Home, Back or an app bring LumaTV's back, and nothing of it exists while the flag file is not there.
//!
//! Time is an argument everywhere it matters: the server is not left to tick on its own, and each step names the instant it is at.

mod harness;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::boot_direct::{GUARD_AFTER, HOME_HOLD, LG_WAIT, Phase};
use lumatv_compositor::input::code::*;
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::shell::Msg;
use serde_json::Value;
use wayland_client::protocol::wl_buffer::WlBuffer;

const WAIT: Duration = Duration::from_secs(3);
const HOME: &str = "com.webos.app.home";
const LAUNCH_HOME: &str = "luna://com.webos.applicationManager/launchDefaultApp";
const CLOSE_APP: &str = "luna://com.webos.applicationManager/closeByAppId";
const MS: fn(u64) -> Duration = Duration::from_millis;

struct Held {
    _client: Client,
    _win: Window,
    _buf: WlBuffer,
}

/// A card of `app_id` mapped with a buffer, brought to the front by its mapping. The server ticks while it maps. `cmd` is what the
/// process behind the connection is called (`/proc/PID/cmdline` in a real boot; here the connection is the test's own).
fn card(server: &mut Server, title: &str, app_id: &str, argb: u32, cmd: Option<&str>) -> Held {
    card_with(server, title, app_id, argb, cmd, |_| {})
}

/// [`card`] with `setup` run on the window before its first buffer (its properties and key mask).
fn card_with(server: &mut Server, title: &str, app_id: &str, argb: u32, cmd: Option<&str>, setup: impl FnOnce(&Window)) -> Held {
    server.ticking.store(true, Ordering::SeqCst);
    let before = server.eval(|s| s.clients.len());
    let mut client = Client::connect_extra(server);
    if let Some(cmd) = cmd {
        let cmd = cmd.to_string();
        // the connection is known to the compositor once its event is drained, a moment after the handshake
        for _ in 0..300 {
            if server.eval(|s| s.clients.len()) > before {
                break;
            }
            std::thread::sleep(MS(10));
        }
        server.eval(move |s| s.clients.values_mut().max_by_key(|c| c.since).expect("the client is connected").cmd = cmd);
    }
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", title);
    win.webos.set_property("appId".into(), app_id.into());
    setup(&win);
    let buf = client.shm_pattern(400, 225, move |_, _| argb);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "{title} presented");
    server.ticking.store(false, Ordering::SeqCst);
    Held { _client: client, _win: win, _buf: buf }
}

/// The Home stub: SAM's Home as a web page, a client with no name of LG's.
fn stub(server: &mut Server) -> Held {
    card(server, "Stub", HOME, 0xff10_1010, Some("/usr/sbin/webapp-mgr --webos-wayland"))
}

/// LG's own Home: Flutter, started by SAM as `flutter-client -i com.webos.app.home`.
fn lg_home(server: &mut Server) -> Held {
    card(server, "LG Home", HOME, 0xff20_4080, Some("flutter-client -i com.webos.app.home"))
}

/// A connected native UI with Home on the engine.
fn shell_up(server: &mut Server) {
    server.eval(|s| {
        s.shell_rust_open(0, "test".into()).unwrap();
        s.shell_rust.kinds = vec!["home".into(), "switcher".into()];
        s.shell_rust_handle(Msg::Connected);
        s.shell_rust_handle(Msg::Ui { can: Some(vec!["switcher".into()]), wants: None, live: None, state: None });
        s.shell_rust.engine_connected = true;
        s.shell_rust_sync_home();
    });
}

/// The flag file with a tool that writes its arguments into `calls` beside it. Returns the directory.
fn flag_on(server: &Server) -> PathBuf {
    server.eval(|s| {
        let dir = s.config.data_dir.clone();
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("lumatv");
        std::fs::write(&tool, "#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/calls\"\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("boot-direct"), format!("on\ntool {}\n", tool.display())).unwrap();
        dir
    })
}

fn calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls")).unwrap_or_default().lines().map(str::to_string).collect()
}

/// What the tool has been asked, once it has been asked `n` times.
fn wait_calls(dir: &Path, n: usize) -> Vec<String> {
    for _ in 0..300 {
        if calls(dir).len() >= n {
            break;
        }
        std::thread::sleep(MS(10));
    }
    calls(dir)
}

/// [`wait_calls`] while the compositor's tick runs, from the instant `at` on: a second ask waits for the run before it to end, and the
/// tick is what starts it (the server of these tests does not tick on its own).
fn settled_calls(server: &Server, dir: &Path, n: usize, at: Instant) -> Vec<String> {
    for i in 0..300 {
        if calls(dir).len() >= n {
            break;
        }
        server.eval(move |s| s.poll_holds_at(at + MS(i)));
        std::thread::sleep(MS(10));
    }
    calls(dir)
}

fn launches_home(s: &State) -> usize {
    sent(s, LAUNCH_HOME)
}

fn sent(s: &State, uri: &str) -> usize {
    s.luna_sent().into_iter().filter(|(u, _)| u == uri).count()
}

fn phase(s: &State) -> &'static str {
    match s.boot_direct.phase() {
        Phase::Off => "off",
        Phase::Waiting(_) => "waiting",
        Phase::Shown => "shown",
    }
}

fn key(s: &mut State, code: u32, pressed: bool, at: Instant) {
    s.key_event_at(SeatId(0), code, pressed, false, at);
}

/// An instant well ahead of the real clock, so that the server's own ticks (while a window maps) never look at the flag file again
/// before the test's next step, and the rate limit on that look has run out for the first of them.
fn start() -> Instant {
    Instant::now() + Duration::from_secs(20)
}

#[test]
fn with_the_flag_off_nothing_changes_and_a_long_press_of_home_is_a_home_tap() {
    let mut server = Server::start();
    shell_up(&mut server);
    let _stub = stub(&mut server);
    let (homes, report, home_drawn) = server.eval(|s| {
        let t0 = start();
        s.poll_holds_at(t0);
        key(s, KEY_HOMEPAGE, true, t0 + MS(100));
        s.poll_holds_at(t0 + HOME_HOLD + MS(500));
        assert_eq!(launches_home(s), 0, "nothing acts before the release");
        key(s, KEY_HOMEPAGE, false, t0 + HOME_HOLD + MS(600));
        (launches_home(s), s.shell_report()["bootDirect"].clone(), s.shell_home_drawn())
    });
    assert_eq!(homes, 1, "the release is the Home action, as ever");
    assert_eq!(report["on"], false);
    assert_eq!(report["phase"], "off");
    assert!(home_drawn);
    assert!(server.eval(|s| s.journal.iter().all(|l| !l.contains("boot-direct"))), "nothing said");
}

#[test]
fn a_home_hold_opens_lgs_home_the_native_one_steps_aside_and_back_returns() {
    let mut server = Server::start();
    shell_up(&mut server);
    let _apps = card(&mut server, "Netflix", "netflix", 0xff00_00ff, None);
    let _stub = stub(&mut server);
    let dir = flag_on(&server);
    let t0 = start();

    server.eval(move |s| {
        s.poll_holds_at(t0);
        assert!(s.boot_direct.on(), "the flag file was read");
        assert!(s.shell_home_drawn(), "LumaTV's Home is what is on the screen");
        key(s, KEY_HOMEPAGE, true, t0 + MS(100));
        key(s, KEY_HOMEPAGE, true, t0 + MS(600));
        s.poll_holds_at(t0 + MS(900));
        assert_eq!(phase(s), "off", "a Home tap has not become a hold");
        s.poll_holds_at(t0 + HOME_HOLD + MS(100));
        assert_eq!(phase(s), "waiting");
    });
    assert_eq!(wait_calls(&dir, 1), ["home lg"], "the tool is asked for LG's Home");

    server.eval(move |s| {
        key(s, KEY_HOMEPAGE, false, t0 + MS(1300));
        assert_eq!(launches_home(s), 0, "the release of a hold is no Home tap");
        assert!(s.shell_home_drawn(), "LumaTV's Home stays until LG's is in front");
        assert_eq!(s.shell_report()["bootDirect"]["lgHomeFront"], false);
    });

    // LG's Flutter Home maps
    let _lg = lg_home(&mut server);
    server.eval(move |s| {
        s.poll_holds_at(t0 + MS(2000));
        assert_eq!(phase(s), "shown");
        assert!(!s.shell_home_drawn(), "LG's Home is what is on the screen");
        assert!(!s.scene.home_rust());
        let report = s.shell_report();
        assert_eq!((&report["bootDirect"]["lgHomeFront"], &report["homeDrawn"]), (&Value::Bool(true), &Value::Bool(false)));
        assert!(s.journal.iter().any(|l| l.contains("stepped aside for LG's Home")), "{:?}", s.journal);

        // Back: the owner is done with it
        key(s, KEY_BACK, true, t0 + MS(3000));
        assert_eq!(phase(s), "off");
        assert!(s.shell_home_drawn(), "LumaTV's Home is back over LG's, which the tool is about to replace");
        key(s, KEY_BACK, false, t0 + MS(3060));
        assert_eq!(sent(s, CLOSE_APP), 0, "Back is not an app's close and its release is not a key for LG's Home");
        assert_eq!(launches_home(s), 0);
    });
    assert_eq!(settled_calls(&server, &dir, 2, t0 + MS(10_000)), ["home lg", "home luma"]);
}

#[test]
fn a_tap_of_home_on_lgs_home_returns_to_lumatvs_and_asks_for_the_stub() {
    let mut server = Server::start();
    shell_up(&mut server);
    let _stub = stub(&mut server);
    let _lg = lg_home(&mut server);
    let dir = flag_on(&server);
    let t0 = start();
    server.eval(move |s| {
        s.poll_holds_at(t0);
        // LG's Home is in front from the start (the tool's `home lg` run from the shell, say): a hold asks, the card is already there
        key(s, KEY_HOMEPAGE, true, t0 + MS(10));
        s.poll_holds_at(t0 + HOME_HOLD + MS(10));
        key(s, KEY_HOMEPAGE, false, t0 + HOME_HOLD + MS(50));
        s.poll_holds_at(t0 + HOME_HOLD + MS(100));
        assert_eq!(phase(s), "shown");
        assert!(!s.shell_home_drawn());

        key(s, KEY_HOMEPAGE, true, t0 + MS(5000));
        key(s, KEY_HOMEPAGE, false, t0 + MS(5070));
        assert_eq!(phase(s), "off");
        assert!(s.shell_home_drawn());
        assert_eq!(launches_home(s), 0, "the tap is not a launch of Home: the card in front is LG's already");
    });
    assert_eq!(settled_calls(&server, &dir, 2, t0 + MS(10_000)), ["home lg", "home luma"]);
}

#[test]
fn an_app_taking_the_front_from_lgs_home_ends_it_and_asks_for_the_stub() {
    let mut server = Server::start();
    shell_up(&mut server);
    let _stub = stub(&mut server);
    let _lg = lg_home(&mut server);
    let dir = flag_on(&server);
    let t0 = start();
    server.eval(move |s| {
        s.poll_holds_at(t0);
        key(s, KEY_HOMEPAGE, true, t0 + MS(10));
        s.poll_holds_at(t0 + HOME_HOLD + MS(10));
        key(s, KEY_HOMEPAGE, false, t0 + HOME_HOLD + MS(50));
        s.poll_holds_at(t0 + HOME_HOLD + MS(100));
        assert_eq!(phase(s), "shown");
    });
    // an app launched from LG's Home comes to the front
    let _plex = card(&mut server, "Plex", "plex", 0xff00_ff00, None);
    server.eval(move |s| {
        s.poll_holds_at(t0 + MS(4000));
        assert_eq!(phase(s), "off");
        assert_eq!(s.foreground_app(), "plex");
    });
    assert_eq!(settled_calls(&server, &dir, 2, t0 + MS(10_000)), ["home lg", "home luma"]);
}

#[test]
fn lgs_home_that_never_comes_is_given_up_and_lumatvs_stays() {
    let mut server = Server::start();
    shell_up(&mut server);
    let _stub = stub(&mut server);
    let dir = flag_on(&server);
    let t0 = start();
    server.eval(move |s| {
        s.poll_holds_at(t0);
        key(s, KEY_HOMEPAGE, true, t0 + MS(10));
        s.poll_holds_at(t0 + HOME_HOLD + MS(10));
        key(s, KEY_HOMEPAGE, false, t0 + HOME_HOLD + MS(50));
        assert_eq!(phase(s), "waiting");
        s.poll_holds_at(t0 + HOME_HOLD + LG_WAIT - MS(100));
        assert_eq!(phase(s), "waiting", "the wait is not over");
        s.poll_holds_at(t0 + HOME_HOLD + LG_WAIT + MS(100));
        assert_eq!(phase(s), "off");
        assert!(s.shell_home_drawn(), "the native Home was never taken away");
    });
    assert_eq!(wait_calls(&dir, 1), ["home lg"]);
    std::thread::sleep(MS(200));
    assert_eq!(calls(&dir), ["home lg"], "nothing was asked to put back");
}

#[test]
fn a_stub_nothing_draws_over_gets_lgs_home_after_the_guard() {
    let mut server = Server::start();
    shell_up(&mut server);
    let _stub = stub(&mut server);
    let dir = flag_on(&server);
    let t0 = start();
    server.eval(move |s| {
        s.poll_holds_at(t0);
        s.poll_holds_at(t0 + GUARD_AFTER * 2);
        assert_eq!(phase(s), "off", "the UI is up: the stub is under its Home");
        s.shell_rust.gave_up = true;
        s.poll_holds_at(t0 + GUARD_AFTER * 2 + MS(100));
        s.poll_holds_at(t0 + GUARD_AFTER * 3);
        assert_eq!(phase(s), "off", "not yet");
        s.poll_holds_at(t0 + GUARD_AFTER * 3 + MS(100));
        assert_eq!(phase(s), "waiting");
    });
    assert_eq!(wait_calls(&dir, 1), ["home lg"]);
}

#[test]
fn the_flag_file_going_away_gives_lumatvs_home_back_at_once() {
    let mut server = Server::start();
    shell_up(&mut server);
    let _stub = stub(&mut server);
    let _lg = lg_home(&mut server);
    let dir = flag_on(&server);
    let t0 = start();
    server.eval(move |s| {
        s.poll_holds_at(t0);
        key(s, KEY_HOMEPAGE, true, t0 + MS(10));
        s.poll_holds_at(t0 + HOME_HOLD + MS(10));
        key(s, KEY_HOMEPAGE, false, t0 + HOME_HOLD + MS(50));
        s.poll_holds_at(t0 + HOME_HOLD + MS(100));
        assert_eq!(phase(s), "shown");
        std::fs::remove_file(s.config.data_dir.join("boot-direct")).unwrap();
        s.poll_holds_at(t0 + MS(5000));
        assert!(!s.boot_direct.on());
        assert_eq!(phase(s), "off");
        assert!(s.shell_home_drawn(), "the native Home is what is drawn again");
        // the keys are what they were
        key(s, KEY_HOMEPAGE, true, t0 + MS(6000));
        s.poll_holds_at(t0 + MS(7500));
        key(s, KEY_HOMEPAGE, false, t0 + MS(7600));
        assert_eq!(launches_home(s), 1);
    });
    assert_eq!(wait_calls(&dir, 1), ["home lg"]);
}

#[test]
fn home_on_lgs_home_leaves_even_when_lgs_home_asked_for_the_key_itself() {
    let mut server = Server::start();
    shell_up(&mut server);
    let _stub = stub(&mut server);
    let dir = flag_on(&server);
    let t0 = start();
    server.eval(move |s| {
        s.poll_holds_at(t0);
        key(s, KEY_HOMEPAGE, true, t0 + MS(10));
        s.poll_holds_at(t0 + HOME_HOLD + MS(10));
        key(s, KEY_HOMEPAGE, false, t0 + HOME_HOLD + MS(50));
    });
    // LG's Home maps with the policy that gives it the Home key and the mask that lets the key through
    let _lg = card_with(&mut server, "LG Home", HOME, 0xff20_4080, Some("flutter-client -i com.webos.app.home"), |win| {
        win.webos.set_property("_WEBOS_ACCESS_POLICY_KEYS_HOME".into(), "true".into());
        win.webos.set_key_mask(1 | 8 | 16 | 32 | 64 | 128);
    });
    server.eval(move |s| {
        s.poll_holds_at(t0 + MS(2000));
        assert_eq!(phase(s), "shown");
        // the route is the window's own now: without boot direct this Home would go to LG's Home
        key(s, KEY_HOMEPAGE, true, t0 + MS(5000));
        key(s, KEY_HOMEPAGE, false, t0 + MS(5060));
        assert_eq!(phase(s), "off");
        assert!(s.shell_home_drawn());
    });
    assert_eq!(settled_calls(&server, &dir, 2, t0 + MS(10_000)), ["home lg", "home luma"]);
}
