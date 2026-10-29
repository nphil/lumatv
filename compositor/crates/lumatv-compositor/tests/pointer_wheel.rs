//! What shows the Magic Remote's pointer (`remote.pointerShow`): a turn of the wheel by default, any motion as on LG's compositor. A hidden
//! pointer is nobody's: no enter, no motion, no cursor to draw; the wheel press is OK, a key; an arrow hides the pointer again.

mod harness;

use std::time::{Duration, Instant};

use harness::{Client, Server};
use lumatv_compositor::config::Config;
use lumatv_compositor::input::Keys;
use lumatv_compositor::server::seat::{CURSOR_TIMEOUT, PointerShow, remote_motion, wheel_turn};
use serde_json::json;

const WAIT: Duration = Duration::from_secs(3);

/// A client with a keyboard, a pointer, an input manager and a 64x64 card at the origin that has been drawn once.
fn pointed(server: &mut Server) -> (Client, Box<dyn std::any::Any>) {
    let mut client = Client::connect(server);
    let _kb = client.seat.get_keyboard(&client.qh, ());
    let _ptr = client.seat.get_pointer(&client.qh, ());
    let _input: lumatv_wl_protocols::webos_input_manager::client::wl_webos_input_manager::WlWebosInputManager =
        client.globals.bind(&client.qh, 1..=1, ()).unwrap();
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Pointed");
    let buf = client.shm_buffer(64, 64, 0xffff_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.keyboard_enters >= 1 && s.frames_done >= 1));
    (client, Box::new((win, buf, _kb, _ptr, _input)))
}

fn ctl(server: &Server, line: &'static str) -> String {
    server.eval(move |s| lumatv_compositor::control::handle(s, line).text())
}

#[test]
fn motion_alone_shows_nothing_and_a_turn_of_the_wheel_shows_the_pointer_where_the_remote_points() {
    let mut server = Server::start();
    let (mut client, _keep) = pointed(&mut server);
    assert_eq!(server.eval(|s| s.keys.pointer_show), PointerShow::Wheel, "the default");
    assert_eq!(client.state.cursor_visibility, vec![0]);

    // Picking the remote up and moving it: onto the card and off it, nothing is shown, entered or sent.
    let sent = server.eval(|s| [remote_motion(s, 10, 20), remote_motion(s, 30, 40), remote_motion(s, 500, 500), remote_motion(s, 20, 25)]);
    assert_eq!(sent, [false; 4]);
    client.roundtrip();
    let (visible, last, motions, at, focus) =
        server.eval(|s| (s.seat.cursor_visible, s.seat.last_input, s.seat.motions, s.seat.pointer_position, s.seat.pointer_focus));
    assert!(!visible && focus.is_none() && motions == 0, "no pointer: {visible} {motions} {focus:?}");
    assert_ne!(last, "pointer", "a hidden pointer is no pointer input");
    assert_eq!(at, (20, 25), "where the remote points is kept");
    assert!(client.state.pointer_enters.is_empty() && client.state.motions.is_empty(), "no pointer event reached the client");
    assert_eq!(client.state.cursor_visibility, vec![0], "nobody was told a cursor appeared");

    // A turn of the wheel shows it, where the remote points: the card is entered there and hears the motion.
    assert!(server.eval(wheel_turn));
    assert!(client.wait_until(WAIT, |s| !s.pointer_enters.is_empty() && !s.motions.is_empty() && s.cursor_visibility.last() == Some(&1)));
    assert_eq!(client.state.pointer_enters[0], (20.0, 25.0));
    assert_eq!(client.state.motions[0], (20.0, 25.0));
    assert!(server.eval(|s| s.seat.cursor_visible && s.seat.last_input == "pointer"));

    // Shown, it follows the remote.
    assert!(server.eval(|s| remote_motion(s, 30, 40)));
    assert!(client.wait_until(WAIT, |s| s.motions.last() == Some(&(30.0, 40.0))));
    assert_eq!(client.state.pointer_enters.len(), 1, "still the same card");

    // The wheel press while it shows is the click; hover is a cue and the click selects.
    ctl(&server, "click tap");
    assert!(client.wait_until(WAIT, |s| s.buttons.len() >= 2));
    assert_eq!(client.state.buttons, vec![(0x110, 1), (0x110, 0)]);
    assert!(client.state.keys.is_empty(), "a click, not KEY_ENTER");
}

#[test]
fn a_wheel_press_while_the_pointer_is_hidden_is_the_ok_key_and_an_arrow_hides_the_pointer_again() {
    let mut server = Server::start();
    let (mut client, _keep) = pointed(&mut server);
    server.eval(|s| remote_motion(s, 10, 20));
    // Pressing the wheel: OK, to the app as a key, and nothing else.
    ctl(&server, "click tap");
    assert!(client.wait_until(WAIT, |s| s.keys.len() >= 2));
    assert_eq!(client.state.keys, vec![(28, 1), (28, 0)]);
    client.roundtrip();
    assert!(client.state.buttons.is_empty(), "no button without a pointer");
    assert!(client.state.pointer_enters.is_empty(), "the press did not show the pointer");
    assert!(!server.eval(|s| s.seat.cursor_visible));

    // Shown by the wheel, an arrow key hides it (leave, told to every client) and goes on to the app.
    server.eval(wheel_turn);
    assert!(client.wait_until(WAIT, |s| !s.pointer_enters.is_empty()));
    ctl(&server, "remote 108 tap");
    assert!(client.wait_until(WAIT, |s| s.pointer_leaves >= 1 && s.keys.len() >= 4));
    assert!(!server.eval(|s| s.seat.cursor_visible));
    assert!(client.wait_until(WAIT, |s| s.cursor_visibility.last() == Some(&0)));

    // Moved again, it stays hidden; the next turn shows it where the remote points now.
    assert!(!server.eval(|s| remote_motion(s, 40, 50)));
    client.roundtrip();
    let entered = client.state.pointer_enters.len();
    assert!(!server.eval(|s| s.seat.cursor_visible));
    assert!(server.eval(wheel_turn));
    assert!(client.wait_until(WAIT, |s| s.pointer_enters.len() > entered));
    assert_eq!(client.state.pointer_enters.last(), Some(&(40.0, 50.0)));
}

#[test]
fn key_1199_is_the_remotes_cursor_off_it_hides_the_pointer_and_is_no_ok_and_the_keep_alive_pulse_is_no_key() {
    use lumatv_compositor::input::code::{KEY_CURSOR_OFF, KEY_ENTER, KEY_POINTER_ALIVE};
    use lumatv_compositor::server::seat::SeatId;
    assert_eq!((KEY_POINTER_ALIVE, KEY_CURSOR_OFF), (1198, 1199), "lginput2's CURSOR_ON and CURSOR_OFF as the set recorded them");
    let mut server = Server::start();
    let (mut client, _keep) = pointed(&mut server);
    let seat = SeatId(0);
    // The pulse the remote sends every 0.5 s while the pointer is active: down and up, and nothing of it is a key or the person's input.
    let before = server.eval(|s| (s.keys.delivered, s.keys.dropped, s.seat.last_input));
    server.eval(move |s| {
        s.key_event(seat, KEY_POINTER_ALIVE, true, false);
        s.key_event(seat, KEY_POINTER_ALIVE, false, false);
    });
    client.roundtrip();
    assert!(client.state.keys.is_empty(), "no key reached the app");
    assert_eq!(server.eval(|s| (s.keys.delivered, s.keys.dropped, s.seat.last_input)), before, "and it was counted as no input");

    // The remote laid down (1199, down and up in one report), with the pointer hidden: nothing, and above all no OK.
    server.eval(move |s| {
        s.key_event(seat, KEY_CURSOR_OFF, true, false);
        s.key_event(seat, KEY_CURSOR_OFF, false, false);
    });
    client.roundtrip();
    assert!(client.state.keys.is_empty() && client.state.buttons.is_empty(), "no key and no click: {:?} {:?}", client.state.keys, client.state.buttons);

    // OK, a real KEY_ENTER, while the pointer is hidden is OK to the app.
    server.eval(move |s| {
        s.key_event(seat, KEY_ENTER, true, false);
        s.key_event(seat, KEY_ENTER, false, false);
    });
    assert!(client.wait_until(WAIT, |s| s.keys.len() >= 2));
    assert_eq!(client.state.keys, vec![(28, 1), (28, 0)]);
    assert!(client.state.buttons.is_empty(), "no pointer, no click");

    // While the pointer shows, OK is the click on what it points at.
    server.eval(|s| {
        remote_motion(s, 20, 25);
        wheel_turn(s)
    });
    assert!(client.wait_until(WAIT, |s| !s.pointer_enters.is_empty()));
    server.eval(move |s| {
        s.key_event(seat, KEY_ENTER, true, false);
        s.key_event(seat, KEY_ENTER, false, false);
    });
    assert!(client.wait_until(WAIT, |s| s.buttons.len() >= 2));
    assert_eq!(client.state.buttons, vec![(0x110, 1), (0x110, 0)]);
    assert_eq!(client.state.keys.len(), 2, "a click, not a second Enter");
    // The keep-alive does not hide or show anything either.
    server.eval(move |s| s.key_event(seat, KEY_POINTER_ALIVE, true, false));
    client.roundtrip();
    assert!(server.eval(|s| s.seat.cursor_visible), "the pointer is still up");

    // The remote laid down while the pointer shows: the pointer goes, the app is told it left, and neither an OK nor a click reaches it.
    let leaves = client.state.pointer_leaves;
    server.eval(move |s| {
        s.key_event(seat, KEY_CURSOR_OFF, true, false);
        s.key_event(seat, KEY_CURSOR_OFF, false, false);
    });
    assert!(client.wait_until(WAIT, move |s| s.pointer_leaves > leaves));
    assert!(!server.eval(|s| s.seat.cursor_visible), "the remote's cursor-off hid the pointer");
    assert_eq!((client.state.keys.len(), client.state.buttons.len()), (2, 2), "nothing was pressed");
}

#[test]
fn the_wheel_keeps_the_pointer_up_and_stillness_hides_it() {
    let mut server = Server::start();
    let (mut client, _keep) = pointed(&mut server);
    server.eval(|s| {
        remote_motion(s, 10, 20);
        wheel_turn(s);
    });
    assert!(client.wait_until(WAIT, |s| !s.pointer_enters.is_empty()));
    // Nearly timed out, then a notch: the timer starts again.
    server.eval(|s| {
        s.seat.cursor_shown_at = Some(Instant::now() - (CURSOR_TIMEOUT - Duration::from_secs(1)));
        wheel_turn(s);
        s.poll_holds();
    });
    assert!(server.eval(|s| s.seat.cursor_visible), "a turn of the wheel is activity");
    server.eval(|s| {
        s.seat.cursor_shown_at = Some(Instant::now() - CURSOR_TIMEOUT);
        s.poll_holds();
    });
    assert!(client.wait_until(WAIT, |s| s.pointer_leaves >= 1));
    assert!(!server.eval(|s| s.seat.cursor_visible));
}

#[test]
fn movement_mode_is_lgs_motion_shows_it_and_the_wheel_alone_does_not() {
    let mut server = Server::start();
    let (mut client, _keep) = pointed(&mut server);
    server.eval(|s| s.keys.pointer_show = PointerShow::Movement);
    assert!(!server.eval(wheel_turn), "no pointer from a wheel in LG's mode");
    assert!(!server.eval(|s| s.seat.cursor_visible));
    assert!(server.eval(|s| remote_motion(s, 10, 20)));
    assert!(client.wait_until(WAIT, |s| s.pointer_enters.len() == 1 && s.motions.len() == 1 && s.cursor_visibility.last() == Some(&1)));
    assert_eq!(client.state.motions[0], (10.0, 20.0));
    ctl(&server, "remote 108 tap");
    assert!(client.wait_until(WAIT, |s| s.pointer_leaves >= 1));
    assert!(server.eval(|s| remote_motion(s, 12, 22)));
    assert!(client.wait_until(WAIT, |s| s.pointer_enters.len() == 2));
}

#[test]
fn the_control_socket_drives_the_same_two_events() {
    let mut server = Server::start();
    let (_client, _keep) = pointed(&mut server);
    let r = ctl(&server, "pointer aim 10 20");
    assert!(r.contains("\"ok\":true") && r.contains("\"delivered\":false"), "{r}");
    assert!(!server.eval(|s| s.seat.cursor_visible));
    let r = ctl(&server, "wheel");
    assert!(r.contains("\"delivered\":true") && r.contains("\"visible\":true") && r.contains("\"show\":\"wheel\""), "{r}");
}

#[test]
fn apps_keep_the_cursor_standby_timer_and_set_the_cursor_position_while_the_pointer_is_hidden() {
    let mut server = Server::start();
    let (mut client, _keep) = pointed(&mut server);
    let starfish = client.starfish_pointer();
    starfish.set_mrcu_standby_timer(300_000);
    starfish.set_cursor_position(32, 32);
    starfish.get_mrcu_standby_timer();
    client.roundtrip();
    assert_eq!(client.state.starfish_timer_values, vec![300_000]);
    let (at, visible) = server.eval(|s| (s.seat.pointer_position, s.seat.cursor_visible));
    assert_eq!((at, visible), ((32, 32), false), "a warp moves the pointer; it does not show it");
    server.eval(wheel_turn);
    assert!(client.wait_until(WAIT, |s| s.pointer_enters.first() == Some(&(32.0, 32.0))));
}

#[test]
fn the_themes_choice_is_read_again_when_the_file_changes_at_most_once_a_second() {
    let server = Server::start();
    let dir = std::env::temp_dir().join(format!("lumatv-pointer-show-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("theme")).unwrap();
    let theme = dir.join("theme/theme.json");
    std::fs::write(&theme, json!({"remote": {"pointerShow": "movement"}}).to_string()).unwrap();
    let t = Instant::now();
    let dir2 = dir.clone();
    let before = server.eval(move |s| {
        s.keys = Keys::new(&Config { data_dir: dir2, ..Config::default() });
        s.keys.poll_theme(t);
        s.keys.pointer_show
    });
    assert_eq!(before, PointerShow::Movement);
    std::fs::write(&theme, json!({"remote": {"pointerShow": "wheel", "numberKeys": true}}).to_string()).unwrap();
    let (soon, later) = server.eval(move |s| {
        s.keys.poll_theme(t + Duration::from_millis(200));
        let soon = s.keys.pointer_show;
        s.keys.poll_theme(t + Duration::from_secs(2));
        (soon, s.keys.pointer_show)
    });
    assert_eq!((soon, later), (PointerShow::Movement, PointerShow::Wheel));
    // A word the compositor does not know, and a theme without the key, are the default.
    for (i, text) in [json!({"remote": {"pointerShow": "tilt"}}).to_string(), json!({"remote": {}, "padding": "x"}).to_string()].into_iter().enumerate() {
        let at = t + Duration::from_secs(10 * (i as u64 + 1));
        std::fs::write(&theme, json!({"remote": {"pointerShow": "movement"}}).to_string()).unwrap();
        server.eval(move |s| s.keys.poll_theme(at));
        std::fs::write(&theme, text).unwrap();
        let got = server.eval(move |s| {
            s.keys.poll_theme(at + Duration::from_secs(2));
            s.keys.pointer_show
        });
        assert_eq!(got, PointerShow::Wheel);
    }
    let _ = std::fs::remove_dir_all(dir);
}
