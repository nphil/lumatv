//! Native overlay key routing over the key socket while apps keep keyboard focus.

mod harness;

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::input::code::*;
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::shell::Msg;
use serde_json::{Value, json};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_keyboard::WlKeyboard;

const WAIT: Duration = Duration::from_secs(3);

struct App {
    client: Client,
    _win: Window,
    _kb: WlKeyboard,
    _buf: WlBuffer,
    keys: UnixStream,
    key_bytes: Vec<u8>,
}

/// An app with a card, the keyboard focus and a mapped buffer.
fn app(server: &mut Server) -> App {
    let mut client = Client::connect(server);
    let kb = client.seat.get_keyboard(&client.qh, ());
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Player");
    win.webos.set_property("appId".into(), "org.test.player".into());
    let buf = client.shm_buffer(64, 64, 0xff00_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.keyboard_enters >= 1), "the card has the focus");
    let (ours, keys) = UnixStream::pair().unwrap();
    ours.set_nonblocking(true).unwrap();
    keys.set_nonblocking(true).unwrap();
    server.eval(move |s| s.shell_rust.adopt_key_socket(ours));
    App { client, _win: win, _kb: kb, _buf: buf, keys, key_bytes: Vec::new() }
}

/// The shell client connected and asking for `names`.
fn takes(server: &Server, names: &[&str]) {
    let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    server.eval(move |s| {
        s.shell_rust_handle(Msg::Connected);
        s.shell_rust_handle(Msg::Takes(names));
    });
}

/// Key messages received from the native UI socket.
fn syskeys(app: &mut App) -> Vec<(String, bool, bool)> {
    let mut buf = [0; 4096];
    loop {
        match app.keys.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => app.key_bytes.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) => panic!("key socket: {e}"),
        }
    }
    std::str::from_utf8(&app.key_bytes).unwrap().lines().map(|line| {
        let event: Value = serde_json::from_str(line).unwrap();
        let key = &event["key"];
        (key["k"].as_str().unwrap().to_string(), key["d"] == true, key["r"] == true)
    }).collect()
}

fn press(s: &mut State, code: u32, t: Instant) {
    s.key_event_at(SeatId(0), code, true, false, t);
}
fn release(s: &mut State, code: u32, t: Instant) {
    s.key_event_at(SeatId(0), code, false, false, t);
}

fn app_keys(a: &mut App) -> Vec<(u32, u32)> {
    a.client.roundtrip();
    a.client.state.keys.clone()
}

#[test]
fn ok_and_back_go_to_the_layer_while_it_takes_them_and_the_app_never_sees_them() {
    let mut server = Server::start();
    let mut a = app(&mut server);
    takes(&server, &["ok", "back"]);
    let t = Instant::now();
    server.eval(move |s| {
        press(s, KEY_ENTER, t);
        release(s, KEY_ENTER, t + Duration::from_millis(80));
        press(s, KEY_PREVIOUS, t + Duration::from_millis(200));
        release(s, KEY_PREVIOUS, t + Duration::from_millis(280));
    });
    assert_eq!(syskeys(&mut a), vec![("ok".into(), true, false), ("ok".into(), false, false), ("back".into(), true, false), ("back".into(), false, false)]);
    assert!(app_keys(&mut a).is_empty(), "the app saw nothing of OK and Back: {:?}", a.client.state.keys);
}

#[test]
fn the_arrows_and_the_digits_stay_the_apps_whatever_the_layer_takes() {
    let mut server = Server::start();
    let mut a = app(&mut server);
    takes(&server, &["ok", "back"]);
    let t = Instant::now();
    server.eval(move |s| {
        for (i, code) in [KEY_LEFT, KEY_RIGHT, KEY_UP, KEY_DOWN, KEY_1].into_iter().enumerate() {
            press(s, code, t + Duration::from_millis(i as u64 * 100));
            release(s, code, t + Duration::from_millis(i as u64 * 100 + 50));
        }
    });
    assert!(syskeys(&mut a).is_empty());
    let keys = app_keys(&mut a);
    assert_eq!(keys.len(), 10, "five presses and five releases reached the app: {keys:?}");
    assert_eq!(keys[0], (KEY_LEFT, 1));
    assert_eq!(keys[1], (KEY_LEFT, 0));
}

#[test]
fn nothing_is_taken_until_the_layer_asks_and_once_it_lets_go_the_keys_are_the_apps_again() {
    let mut server = Server::start();
    let mut a = app(&mut server);
    let t = Instant::now();
    server.eval(move |s| {
        press(s, KEY_ENTER, t);
        release(s, KEY_ENTER, t + Duration::from_millis(60));
    });
    assert_eq!(app_keys(&mut a), vec![(KEY_ENTER, 1), (KEY_ENTER, 0)], "no `takes`: the app has OK");
    takes(&server, &["ok"]);
    server.eval(move |s| {
        press(s, KEY_ENTER, t + Duration::from_secs(1));
        release(s, KEY_ENTER, t + Duration::from_millis(1060));
        // Back is not asked for (the compositor routes it as it always did); the arrow is the app's
        press(s, KEY_RIGHT, t + Duration::from_secs(2));
        release(s, KEY_RIGHT, t + Duration::from_millis(2060));
    });
    assert_eq!(syskeys(&mut a), vec![("ok".into(), true, false), ("ok".into(), false, false)]);
    let after = app_keys(&mut a);
    assert_eq!(after.len(), 4, "OK was taken, the arrow was not: {after:?}");
    assert_eq!(&after[2..], &[(KEY_RIGHT, 1), (KEY_RIGHT, 0)]);
    takes(&server, &[]);
    server.eval(move |s| {
        press(s, KEY_ENTER, t + Duration::from_secs(3));
        release(s, KEY_ENTER, t + Duration::from_millis(3060));
    });
    assert_eq!(syskeys(&mut a).len(), 2, "nothing more once the layer let go");
    assert_eq!(app_keys(&mut a).len(), 6);
}

#[test]
fn a_held_key_repeats_to_the_layer_and_its_release_follows_it_even_after_the_cards_are_gone() {
    let mut server = Server::start();
    let mut a = app(&mut server);
    takes(&server, &["ok", "back"]);
    let t = Instant::now();
    server.eval(move |s| {
        press(s, KEY_ENTER, t);
        press(s, KEY_ENTER, t + Duration::from_millis(500));
        press(s, KEY_ENTER, t + Duration::from_millis(600));
        // the last card went while the key was down
        s.shell_rust_handle(Msg::Takes(Vec::new()));
        press(s, KEY_ENTER, t + Duration::from_millis(700));
        release(s, KEY_ENTER, t + Duration::from_millis(800));
    });
    assert_eq!(
        syskeys(&mut a),
        vec![("ok".into(), true, false), ("ok".into(), true, true), ("ok".into(), true, true), ("ok".into(), true, true), ("ok".into(), false, false)]
    );
    assert!(app_keys(&mut a).is_empty(), "the app was given no release of a key it had no press of");
    // and the key is free again for the app
    let t2 = t + Duration::from_secs(5);
    server.eval(move |s| {
        press(s, KEY_ENTER, t2);
        release(s, KEY_ENTER, t2 + Duration::from_millis(50));
    });
    assert_eq!(app_keys(&mut a), vec![(KEY_ENTER, 1), (KEY_ENTER, 0)]);
}

#[test]
fn a_press_the_app_already_has_keeps_its_repeats_and_its_release() {
    let mut server = Server::start();
    let mut a = app(&mut server);
    let t = Instant::now();
    // OK went down before the cards arrived: it was meant for the app
    server.eval(move |s| press(s, KEY_ENTER, t));
    takes(&server, &["ok", "back"]);
    server.eval(move |s| {
        press(s, KEY_ENTER, t + Duration::from_millis(400));
        release(s, KEY_ENTER, t + Duration::from_millis(900));
    });
    assert!(syskeys(&mut a).is_empty(), "a repeat of the app's key is not a press of the layer's");
    let keys = app_keys(&mut a);
    assert_eq!(keys.last(), Some(&(KEY_ENTER, 0)), "the release reached the app: {keys:?}");
}

#[test]
fn a_press_that_never_got_its_release_is_not_taken_for_a_repeat_for_ever() {
    let mut server = Server::start();
    let mut a = app(&mut server);
    takes(&server, &["ok"]);
    let t = Instant::now();
    server.eval(move |s| {
        press(s, KEY_ENTER, t);
        // ten seconds on, a press again: a new press
        press(s, KEY_ENTER, t + Duration::from_secs(10));
        release(s, KEY_ENTER, t + Duration::from_millis(10_050));
    });
    assert_eq!(syskeys(&mut a), vec![("ok".into(), true, false), ("ok".into(), true, false), ("ok".into(), false, false)]);
}

#[test]
fn native_panel_takes_keys_without_moving_the_app_focus() {
    let mut server = Server::start();
    let mut a = app(&mut server);
    takes(&server, &[]);
    server.eval(|s| s.shell_rust_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: None }));
    let t = Instant::now();
    server.eval(move |s| {
        press(s, KEY_RIGHT, t);
        release(s, KEY_RIGHT, t + Duration::from_millis(60));
    });
    assert_eq!(syskeys(&mut a), vec![("right".into(), true, false), ("right".into(), false, false)]);
    assert!(app_keys(&mut a).is_empty());
}

#[test]
fn unsupported_and_duplicate_key_names_are_filtered_in_the_report() {
    let server = Server::start();
    takes(&server, &["left", "ok", "ok", "volume", "back"]);
    let report: Value = server.eval(|s| s.shell_rust_report());
    assert_eq!(report["takes"], json!(["ok", "back"]));
    server.eval(|s| s.shell_rust_handle(Msg::Takes(Vec::new())));
    let report: Value = server.eval(|s| s.shell_rust_report());
    assert_eq!(report["takes"], json!([]));
}

#[test]
fn the_keys_the_router_takes_are_never_the_layers() {
    // the gear is the compositor's (a tap asks for the quick panel); a layer that asked for OK and Back does not get it
    let mut server = Server::start();
    let mut a = app(&mut server);
    takes(&server, &["ok", "back"]);
    let t = Instant::now();
    server.eval(move |s| {
        press(s, KEY_MENU, t);
        release(s, KEY_MENU, t + Duration::from_millis(60));
    });
    assert!(syskeys(&mut a).is_empty());
    assert!(app_keys(&mut a).is_empty());
}
