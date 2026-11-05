//! Boot direct (`boot_direct.rs`) under the real `State`, with the shell client played by messages and the apps by windows: the flag file
//! changes nothing about the keys. A Home held for any time, past five seconds and past ten, is one Home tap at its release, as with the
//! flag off, and nothing is run on the way (the one way to LG's Home is the supervisor's escape, which is not in this process). What the
//! compositor does say about it is the report of `ctl shell`, which the checks of a boot read.
//!
//! Time is an argument everywhere it matters: the server is not left to tick on its own, and each step names the instant it is at.

mod harness;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::input::code::*;
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::shell::Msg;
use wayland_client::protocol::wl_buffer::WlBuffer;

const WAIT: Duration = Duration::from_secs(3);
const HOME: &str = "com.webos.app.home";
const LAUNCH_HOME: &str = "luna://com.webos.applicationManager/launchDefaultApp";
const MS: fn(u64) -> Duration = Duration::from_millis;

struct Held {
    _client: Client,
    _win: Window,
    _buf: WlBuffer,
}

/// A card of `app_id` mapped with a buffer, brought to the front by its mapping. The server ticks while it maps. `cmd` is what the
/// process behind the connection is called (`/proc/PID/cmdline` in a real boot; here the connection is the test's own).
fn card(server: &mut Server, title: &str, app_id: &str, argb: u32, cmd: &str) -> Held {
    server.ticking.store(true, Ordering::SeqCst);
    let before = server.eval(|s| s.clients.len());
    let mut client = Client::connect_extra(server);
    let cmd = cmd.to_string();
    // the connection is known to the compositor once its event is drained, a moment after the handshake
    for _ in 0..300 {
        if server.eval(|s| s.clients.len()) > before {
            break;
        }
        std::thread::sleep(MS(10));
    }
    server.eval(move |s| s.clients.values_mut().max_by_key(|c| c.since).expect("the client is connected").cmd = cmd);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", title);
    win.webos.set_property("appId".into(), app_id.into());
    let buf = client.shm_pattern(400, 225, move |_, _| argb);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "{title} presented");
    server.ticking.store(false, Ordering::SeqCst);
    Held { _client: client, _win: win, _buf: buf }
}

/// The Home stub: SAM's Home as a web page, a client with no name of LG's.
fn stub(server: &mut Server) -> Held {
    card(server, "Stub", HOME, 0xff10_1010, "/usr/sbin/webapp-mgr --webos-wayland")
}

/// LG's own Home: Flutter, started by SAM as `flutter-client -i com.webos.app.home`.
fn lg_home(server: &mut Server) -> Held {
    card(server, "LG Home", HOME, 0xff20_4080, "flutter-client -i com.webos.app.home")
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

/// The flag file as the earlier build wrote it, with a tool beside it that writes its arguments into `calls`: nothing may run it now.
/// Returns the directory.
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

fn launches_home(s: &State) -> usize {
    s.luna_sent().into_iter().filter(|(u, _)| u == LAUNCH_HOME).count()
}

fn key(s: &mut State, code: u32, pressed: bool, at: Instant) {
    s.key_event_at(SeatId(0), code, pressed, false, at);
}

/// A Home key that goes down at `from` and is held until `until`, the remote repeating it every half second as it does, with the
/// compositor's tick looking in every 250 ms. The release is the caller's.
fn hold_home(s: &mut State, from: Instant, until: Instant) {
    key(s, KEY_HOMEPAGE, true, from);
    let mut at = from;
    while at < until {
        at = (at + MS(250)).min(until);
        if (at - from).as_millis() % 500 == 0 {
            key(s, KEY_HOMEPAGE, true, at);
        }
        s.poll_holds_at(at);
    }
}

#[test]
fn home_held_past_five_and_ten_seconds_is_one_home_tap_at_the_release_with_the_flag_on_or_off() {
    let mut server = Server::start();
    shell_up(&mut server);
    let _stub = stub(&mut server);
    let mut dir = None;
    for (round, flag) in [false, true].into_iter().enumerate() {
        if flag {
            dir = Some(flag_on(&server));
        }
        let (taps, report, home_drawn) = server.eval(move |s| {
            let t0 = Instant::now() + Duration::from_secs(30 * round as u64);
            let before = launches_home(s);
            s.poll_holds_at(t0);
            hold_home(s, t0 + MS(100), t0 + MS(12_100));
            assert_eq!(launches_home(s), before, "flag {flag}: nothing acts before the release, not at five seconds and not at ten");
            key(s, KEY_HOMEPAGE, false, t0 + MS(12_200));
            (launches_home(s) - before, s.shell_report()["bootDirect"].clone(), s.shell_home_drawn())
        });
        assert_eq!(taps, 1, "flag {flag}: the release is the Home action, as ever");
        assert!(home_drawn, "flag {flag}: LumaTV's Home is what is drawn");
        assert_eq!(report["on"], flag);
        assert_eq!(report["lgHomeFront"], false);
    }
    assert!(server.eval(|s| s.journal.iter().all(|l| !l.contains("boot-direct"))), "nothing said");
    std::thread::sleep(MS(150));
    assert!(!dir.unwrap().join("calls").exists(), "the tool of an earlier flag file is never run");
}

#[test]
fn the_report_says_whose_home_is_in_front_and_whether_the_ui_is_there() {
    let mut server = Server::start();
    shell_up(&mut server);
    flag_on(&server);
    let report = |server: &Server| server.eval(|s| s.shell_report()["bootDirect"].clone());
    let before = report(&server);
    assert_eq!(before["on"], true);
    assert_eq!((before["homeFront"].as_bool(), before["lgHomeFront"].as_bool(), before["uiGone"].as_bool()), (Some(false), Some(false), Some(false)));

    let _stub = stub(&mut server);
    let r = report(&server);
    assert_eq!((r["homeFront"].as_bool(), r["lgHomeFront"].as_bool()), (Some(true), Some(false)), "the stub is a Home card and not LG's");

    let _lg = lg_home(&mut server);
    let r = report(&server);
    assert_eq!((r["homeFront"].as_bool(), r["lgHomeFront"].as_bool()), (Some(true), Some(true)), "the Flutter client is LG's Home: what a boot check fails on");

    server.eval(|s| s.shell_rust.gave_up = true);
    assert_eq!(report(&server)["uiGone"], true);
}
