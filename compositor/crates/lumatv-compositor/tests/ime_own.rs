//! LumaTV's own keyboard path (`server/ime.rs`, `own_*`): an app's text model on one connection and the shell's keyboard on the UI channel
//! (`Msg::Ime`, the `ime` topic, the `imekey` events), no input method at all. What the shell types reaches the field as `commit_string`,
//! `keysym` and `delete_surrounding_text`, the remote's navigation keys go to the shell while its keyboard is up and everything else stays
//! the app's, and LG's keyboard serves the field whenever the flag, the shell or its keyboard is missing.

mod harness;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use harness::{Client, Server, Window};
use lumatv_compositor::Rect;
use lumatv_compositor::State;
use lumatv_compositor::input::code::*;
use lumatv_compositor::server::ime::{OWN_FLAG, own_tick};
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::shell::Msg;
use lumatv_wl_protocols::input_method::client::input_method::InputMethod;
use lumatv_wl_protocols::text::client::text_model::TextModel;
use serde_json::{Value, json};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_keyboard::WlKeyboard;

const WAIT: Duration = Duration::from_secs(3);
const KEY_A: u32 = 30;
const KEY_3: u32 = 4;
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A compositor whose data directory holds the flag (or not) and whose shell client says it can show the keyboard (or not).
fn setup(flag: bool, can: bool) -> (Server, PathBuf) {
    let server = Server::start();
    let dir = std::env::temp_dir().join(format!("lumatv-ime-own-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&dir).unwrap();
    if flag {
        std::fs::write(dir.join(OWN_FLAG), "").unwrap();
    }
    let d = dir.clone();
    server.eval(move |s| {
        s.config.data_dir = d;
        s.shell_handle(Msg::Connected);
        let can = if can { vec!["keyboard".to_string()] } else { Vec::new() };
        s.shell_handle(Msg::Ui { can: Some(can), wants: None, live: None, state: None });
    });
    (server, dir)
}

/// An app with a card that has the keyboard focus and a text model.
struct App {
    client: Client,
    win: Window,
    model: TextModel,
    _kb: WlKeyboard,
    _buf: WlBuffer,
}

fn app(server: &mut Server, title: &str) -> App {
    let mut client = if server.client_stream.is_some() { Client::connect(server) } else { Client::connect_extra(server) };
    let kb = client.seat.get_keyboard(&client.qh, ());
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", title);
    let buf = client.shm_buffer(64, 64, 0xff00_0000);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.keyboard_enters >= 1), "the card has the focus");
    let model = client.text_model_factory().create_text_model(&client.qh, ());
    client.roundtrip();
    App { client, win, model, _kb: kb, _buf: buf }
}

impl App {
    fn activate(&mut self) {
        self.model.activate(1, &self.client.seat, &self.win.surface);
        self.client.conn.flush().unwrap();
    }
}

/// The shell's keyboard window of the system layer: 64x64, mapped above the app.
struct Layer {
    _client: Client,
    _kb: WlKeyboard,
    _buf: WlBuffer,
}

fn layer(server: &mut Server) -> Layer {
    let mut client = Client::connect_extra(server);
    let kb = client.seat.get_keyboard(&client.qh, ());
    let win = client.card("_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM", "Layer");
    win.webos.set_property("appId".into(), "org.nphil.lumatv.shell".into());
    let buf = client.shm_buffer(64, 64, 0x8000_0080);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "the layer presented");
    Layer { _client: client, _kb: kb, _buf: buf }
}

/// Polls the compositor's state until `pred` holds.
fn until(server: &Server, what: &str, pred: impl Fn(&mut State) -> bool + Send + Clone + 'static) {
    let deadline = Instant::now() + WAIT;
    loop {
        let p = pred.clone();
        if server.eval(move |s| p(s)) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// What the shell's keyboard says over `POST /v1/ime`.
fn say(server: &Server, op: Value) {
    server.eval(move |s| s.shell_handle(Msg::Ime(op)));
}

fn topic(server: &Server) -> Value {
    server.eval(|s| s.shell.topic("ime").cloned().unwrap_or(Value::Null))
}

fn imekeys(server: &Server) -> Vec<Value> {
    server.eval(|s| s.shell.recorded.iter().filter(|e| e["topic"] == "imekey").map(|e| e["data"].clone()).collect())
}

/// An app's field described and activated, asking for its keyboard; the compositor has taken it all in.
fn open_field(server: &mut Server, a: &mut App, hint: u32, purpose: u32, enter: u32) -> u64 {
    a.model.set_content_type(hint, purpose);
    a.model.set_enter_key_type(enter);
    a.model.set_surrounding_text("abc".into(), 3, 3);
    a.activate();
    a.model.show_input_panel();
    a.client.conn.flush().unwrap();
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_enters == 1), "the field is told it entered");
    until(server, "the shell is told the field wants its keyboard", |s| s.shell.topic("ime").is_some_and(|t| t["show"] == true));
    topic(server)["model"].as_u64().unwrap()
}

fn shown(server: &Server, model: u64, x: i32, y: i32, w: i32, h: i32) {
    say(server, json!({"op": "shown", "model": model, "x": x, "y": y, "w": w, "h": h}));
}

#[test]
fn what_the_shells_keyboard_types_reaches_the_field_and_the_field_is_described_to_it() {
    let (mut server, dir) = setup(true, true);
    let mut a = app(&mut server, "Search");
    let model = open_field(&mut server, &mut a, 0x1, 6, 5);

    // The shell is told what the field is: an e-mail field with a Search key, text in it. No text of the field is in the topic.
    let t = topic(&server);
    assert_eq!(
        (t["active"].clone(), t["hint"].clone(), t["purpose"].clone(), t["enter"].clone(), t["text"].clone()),
        (json!(true), json!(1), json!(6), json!(5), json!(true))
    );
    assert!(!t.to_string().contains("abc"), "the field's words never leave the compositor: {t}");
    assert_eq!(server.eval(|s| s.ime.asked), 0, "LG's input method was never asked for");
    assert!(!server.eval(|s| s.keyboard_up()), "asked for, not on the screen yet: the keys are the app's");

    // On the screen: the field is told where, the keyboard is up.
    shown(&server, model, 0, 626, 1920, 454);
    assert!(a.client.wait_until(WAIT, |s| s.ime.panel_states == vec![1]), "{:?}", a.client.state.ime.panel_states);
    assert_eq!(a.client.state.ime.panel_rects.last(), Some(&(0, 626, 1920, 454)));
    assert!(server.eval(|s| s.keyboard_up() && s.ime_grabbing()));

    // What it types: text, Backspace, Enter, Clear all.
    say(&server, json!({"op": "text", "model": model, "text": "h"}));
    say(&server, json!({"op": "key", "key": "backspace"}));
    say(&server, json!({"op": "key", "key": "enter"}));
    say(&server, json!({"op": "clear"}));
    assert!(a.client.wait_until(WAIT, |s| s.ime.keysyms.len() >= 4 && !s.ime.deletes.is_empty() && !s.ime.commits.is_empty()));
    let log = &a.client.state.ime;
    assert_eq!(log.commits, ["h"]);
    assert_eq!(log.keysyms, [(0xff08, 1), (0xff08, 0), (0xff0d, 1), (0xff0d, 0)], "Backspace and Enter are keys pressed and let go");
    assert_eq!(log.deletes, [(-3, 3)], "Clear all deletes what the field said it holds, from its start");
    let counts = server.eval(|s| s.ime_report()["own"]["counts"].clone());
    assert_eq!((counts["texts"].clone(), counts["keys"].clone(), counts["clears"].clone()), (json!(1), json!(2), json!(1)));

    // A request about another field than the active one is stale.
    say(&server, json!({"op": "text", "model": model + 100, "text": "x"}));
    a.client.roundtrip();
    assert_eq!(a.client.state.ime.commits, ["h"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_remotes_navigation_keys_are_the_keyboards_while_it_is_up_and_the_apps_again_when_it_hides() {
    let (mut server, dir) = setup(true, true);
    let mut a = app(&mut server, "Search");
    let model = open_field(&mut server, &mut a, 0x7, 0, 5);
    // Before it is on the screen the arrow is the app's.
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_LEFT, true, false);
        s.key_event(SeatId(0), KEY_LEFT, false, false);
    });
    assert_eq!(imekeys(&server).len(), 0, "nothing for the shell yet");

    shown(&server, model, 0, 626, 1920, 454);
    until(&server, "the keyboard is up", |s| s.ime_grabbing());
    server.eval(|s| {
        // an arrow held (a press, the remote's repeat, the release), a digit, and a letter of a hardware keyboard
        for (code, pressed) in [(KEY_RIGHT, true), (KEY_RIGHT, true), (KEY_RIGHT, false), (KEY_3, true), (KEY_3, false), (KEY_A, true), (KEY_A, false)] {
            s.key_event(SeatId(0), code, pressed, false);
        }
        // Back and OK as well
        for (code, pressed) in [(KEY_PREVIOUS, true), (KEY_PREVIOUS, false), (KEY_ENTER, true), (KEY_ENTER, false)] {
            s.key_event(SeatId(0), code, pressed, false);
        }
    });
    let keys: Vec<(String, bool, bool)> =
        imekeys(&server).iter().map(|e| (e["k"].as_str().unwrap().to_string(), e["d"].as_bool().unwrap(), e["r"].as_bool().unwrap())).collect();
    let k = |name: &str, d: bool, r: bool| (name.to_string(), d, r);
    assert_eq!(
        keys,
        [
            k("right", true, false),
            k("right", true, true),
            k("right", false, false),
            k("3", true, false),
            k("3", false, false),
            k("back", true, false),
            k("back", false, false),
            k("ok", true, false),
            k("ok", false, false)
        ],
        "the arrows, a digit, Back and OK reach the shell, the second press of a held key is marked as the remote's repeat"
    );
    assert!(
        a.client.wait_until(WAIT, |s| s.keys.contains(&(KEY_A, 1)) && s.keys.contains(&(KEY_A, 0))),
        "a hardware keyboard's letter is the app's: {:?}",
        a.client.state.keys
    );
    assert!(!a.client.state.keys.iter().any(|(c, _)| matches!(*c, KEY_RIGHT | KEY_3 | KEY_PREVIOUS | KEY_ENTER)), "{:?}", a.client.state.keys);

    // The shell hides its keyboard (Back): the field stays active, is told, and the keys are the app's again.
    say(&server, json!({"op": "hide", "model": model}));
    assert!(a.client.wait_until(WAIT, |s| s.ime.panel_states == vec![1, 0]), "{:?}", a.client.state.ime.panel_states);
    assert_eq!(a.client.state.ime.model_leaves, 0, "the field is still active");
    let t = topic(&server);
    assert_eq!((t["active"].clone(), t["show"].clone()), (json!(true), json!(false)));
    assert!(!server.eval(|s| s.keyboard_up()));
    let before = imekeys(&server).len();
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_RIGHT, true, false);
        s.key_event(SeatId(0), KEY_RIGHT, false, false);
    });
    assert!(a.client.wait_until(WAIT, |s| s.keys.contains(&(KEY_RIGHT, 1))), "the arrow is the app's");
    assert_eq!(imekeys(&server).len(), before);

    // The app asks again: the keyboard comes back as the same field's, and Exit closes the field.
    a.model.show_input_panel();
    a.client.conn.flush().unwrap();
    until(&server, "the field wants its keyboard again", |s| s.shell.topic("ime").is_some_and(|t| t["show"] == true));
    shown(&server, model, 0, 626, 1920, 454);
    until(&server, "up again", |s| s.keyboard_up());
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_EXIT, true, false);
        s.key_event(SeatId(0), KEY_EXIT, false, false);
    });
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_leaves == 1), "Exit closes the keyboard and the field");
    assert_eq!(topic(&server), json!({"active": false, "show": false}));
    assert!(!server.eval(|s| s.ime_grabbing() || s.scene.shell_pointer.is_some()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_field_hiding_its_keyboard_or_leaving_ends_it_and_a_new_field_is_described_to_the_shell_at_once() {
    let (mut server, dir) = setup(true, true);
    let mut a = app(&mut server, "Search");
    let model = open_field(&mut server, &mut a, 0x7, 2, 0);
    shown(&server, model, 580, 626, 760, 454);
    until(&server, "up", |s| s.keyboard_up());
    assert_eq!(server.eval(|s| s.scene.shell_pointer), Some(Rect::new(580, 626, 760, 454)), "the pointer is the keyboard's inside its rectangle");

    a.model.hide_input_panel();
    a.client.conn.flush().unwrap();
    until(&server, "hidden by the field", |s| !s.keyboard_up());
    assert_eq!(server.eval(|s| s.scene.shell_pointer), None);
    assert!(a.client.wait_until(WAIT, |s| s.ime.panel_states == vec![1, 0]));

    // A second field of the same app takes the activation while the first held it: one topic change, to the second.
    a.model.show_input_panel();
    a.client.conn.flush().unwrap();
    until(&server, "wants it again", |s| s.shell.topic("ime").is_some_and(|t| t["show"] == true));
    let second = a.client.text_model_factory().create_text_model(&a.client.qh, ());
    a.client.roundtrip();
    second.set_content_type(0xc0, 8);
    second.activate(2, &a.client.seat, &a.win.surface);
    second.show_input_panel();
    a.client.conn.flush().unwrap();
    until(&server, "the second field", |s| s.shell.topic("ime").is_some_and(|t| t["purpose"] == 8 && t["show"] == true));
    let t = topic(&server);
    assert_ne!(t["model"].as_u64().unwrap(), model, "the shell is told of the other field");
    let changes: Vec<Value> = server.eval(|s| s.shell.recorded.iter().filter(|e| e["topic"] == "ime").map(|e| e["data"].clone()).collect());
    let last_of_first = changes.iter().rposition(|c| c["model"] == json!(model) && c["show"] == true).expect("the first field asked for its keyboard");
    assert!(changes[last_of_first + 1..].iter().all(|c| c["active"] == true), "no inactive state between the two fields: {changes:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_release_of_the_key_that_opened_the_keyboard_goes_back_to_the_app() {
    let (mut server, dir) = setup(true, true);
    let mut a = app(&mut server, "Search");
    // OK is pressed on the field, which opens the keyboard; the keyboard comes up, and then OK is let go.
    server.eval(|s| s.key_event(SeatId(0), KEY_ENTER, true, false));
    let model = open_field(&mut server, &mut a, 0x7, 0, 0);
    shown(&server, model, 0, 626, 1920, 454);
    until(&server, "up", |s| s.ime_grabbing());
    server.eval(|s| s.key_event(SeatId(0), KEY_ENTER, false, false));
    a.client.roundtrip();
    assert_eq!(a.client.state.keys, vec![(KEY_ENTER, 1), (KEY_ENTER, 0)], "the app gets the release of the press it got");
    assert_eq!(imekeys(&server).len(), 0, "and the keyboard none of it");
    // The next press is the keyboard's.
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_ENTER, true, false);
        s.key_event(SeatId(0), KEY_ENTER, false, false);
    });
    assert_eq!(imekeys(&server).len(), 2);
    a.client.roundtrip();
    assert_eq!(a.client.state.keys.len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn without_the_flag_or_a_shell_that_can_show_it_the_field_goes_to_lgs_input_method() {
    for (flag, can) in [(false, true), (true, false), (false, false)] {
        let (mut server, dir) = setup(flag, can);
        let asked = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = asked.clone();
        server.eval(move |s| {
            s.ime.ask_for_input_method = Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            })
        });
        let mut a = app(&mut server, "Search");
        a.activate();
        until(&server, "the activation waits for an input method", |s| s.ime.asked == 1);
        assert_eq!(topic(&server)["active"], json!(false), "the shell's keyboard is not asked (flag {flag}, can show {can})");
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn maliit(server: &mut Server) -> (Client, InputMethod) {
    let mut client = Client::connect_extra(server);
    let method = client.input_method();
    method.set_display_id(0);
    client.roundtrip();
    (client, method)
}

#[test]
fn a_keyboard_that_does_not_come_up_in_two_seconds_gives_the_field_to_lgs_for_a_minute() {
    let (mut server, dir) = setup(true, true);
    let mut a = app(&mut server, "Search");
    let _model = open_field(&mut server, &mut a, 0x7, 0, 0);
    let (mut k, _method) = maliit(&mut server);
    assert_eq!(k.state.ime.activations, 0, "the shell was asked first");
    // The shell says nothing: past two seconds the field goes to LG's input method, which is shown at once.
    server.eval(|s| own_tick(s, Instant::now() + Duration::from_secs(3)));
    assert!(k.wait_until(WAIT, |s| s.ime.activations == 1 && s.ime.shows == 1), "the field was handed over and its keyboard asked to show");
    assert_eq!(topic(&server)["active"], json!(false));
    let report = server.eval(|s| s.ime_report()["own"].clone());
    assert_eq!((report["broken"].clone(), report["counts"]["failed"].clone()), (json!(true), json!(1)));
    // The next field is LG's too, while the minute lasts.
    let mut b = app(&mut server, "Other");
    b.activate();
    assert!(k.wait_until(WAIT, |s| s.ime.activations == 2), "{:?}", k.state.ime.activations);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_shell_going_ends_the_field_it_serves_and_a_late_ok_from_it_is_dropped() {
    let (mut server, dir) = setup(true, true);
    let mut a = app(&mut server, "Search");
    let model = open_field(&mut server, &mut a, 0x7, 0, 0);
    shown(&server, model, 0, 626, 1920, 454);
    until(&server, "up", |s| s.keyboard_up());
    server.eval(|s| s.shell_restart());
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_leaves == 1), "the field left with the shell's keyboard");
    assert!(!server.eval(|s| s.ime_grabbing() || s.keyboard_up() || s.scene.shell_pointer.is_some()));
    say(&server, json!({"op": "text", "text": "late"}));
    a.client.roundtrip();
    assert!(a.client.state.ime.commits.is_empty(), "nothing is typed into a field that left");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_system_layer_takes_the_pointer_inside_the_keyboard_and_nowhere_else() {
    let (mut server, dir) = setup(true, true);
    let mut a = app(&mut server, "Search");
    let _layer = layer(&mut server);
    let model = open_field(&mut server, &mut a, 0x7, 0, 0);
    let title_at = |x: i32, y: i32| {
        move |s: &mut State| {
            s.tick();
            s.scene.surface_at(x, y).and_then(|(sid, _)| s.scene.window_of(sid)).and_then(|w| s.scene.window(w)).map(|w| w.title.clone())
        }
    };
    assert_eq!(server.eval(title_at(10, 10)).as_deref(), Some("Search"), "the transparent layer lets the pointer through");
    // The keyboard is where the layer's window is (64x64 in this test): the pointer is the layer's there.
    shown(&server, model, 0, 0, 64, 64);
    until(&server, "up", |s| s.keyboard_up());
    assert_eq!(server.eval(title_at(10, 10)).as_deref(), Some("Layer"));
    say(&server, json!({"op": "hide"}));
    until(&server, "down", |s| !s.keyboard_up());
    assert_eq!(server.eval(title_at(10, 10)).as_deref(), Some("Search"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_field_that_never_asks_for_its_keyboard_still_gets_the_shells_at_activation() {
    // LG's Search app activates its text model and never sends `show_input_panel`; maliit shows at the activation, so does the shell's.
    let (mut server, _dir) = setup(true, true);
    let mut a = app(&mut server, "Search");
    a.model.set_content_type(5, 0);
    a.model.set_enter_key_type(5);
    a.activate();
    a.client.conn.flush().unwrap();
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_enters == 1), "the field is told it entered");
    until(&server, "the shell is told the field wants its keyboard", |s| s.shell.topic("ime").is_some_and(|t| t["show"] == true));
    // and the app can still take it away
    a.model.hide_input_panel();
    a.client.conn.flush().unwrap();
    until(&server, "the keyboard is hidden by the app", |s| s.shell.topic("ime").is_some_and(|t| t["show"] == false));
}

/// [`setup`] with the Rust client answering the kind `keyboard` (its channel is open and nothing runs behind it) instead of the QML client.
fn setup_rust() -> (Server, PathBuf) {
    let server = Server::start();
    let dir = std::env::temp_dir().join(format!("lumatv-ime-own-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(OWN_FLAG), "").unwrap();
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let d = dir.clone();
    server.eval(move |s| {
        s.config.data_dir = d;
        s.shell_rust.kinds = vec!["keyboard".to_string()];
        s.shell_rust_open(port, "token".into()).unwrap();
        s.shell_rust_handle(Msg::Connected);
        s.shell_rust_handle(Msg::Ui { can: Some(vec!["keyboard".to_string()]), wants: None, live: None, state: None });
    });
    (server, dir)
}

fn rust_topic(server: &Server, name: &'static str) -> Value {
    server.eval(move |s| s.shell_rust.topic(name).cloned().unwrap_or(Value::Null))
}

#[test]
fn the_rust_client_that_answers_the_keyboard_is_told_the_field_and_the_keys_and_what_it_types_reaches_the_field() {
    let (mut server, dir) = setup_rust();
    assert!(server.eval(|s| s.shell_can("keyboard")), "the Rust client's word is the one that counts");
    let mut a = app(&mut server, "Search");
    a.model.set_content_type(0x7, 6);
    a.model.set_enter_key_type(5);
    a.model.set_surrounding_text("abc".into(), 3, 3);
    a.activate();
    a.model.show_input_panel();
    a.client.conn.flush().unwrap();
    assert!(a.client.wait_until(WAIT, |s| s.ime.model_enters == 1), "the field is told it entered");
    until(&server, "the Rust client is told the field wants its keyboard", |s| s.shell_rust.topic("ime").is_some_and(|t| t["show"] == true));
    let t = rust_topic(&server, "ime");
    assert_eq!((t["purpose"].clone(), t["enter"].clone(), t["text"].clone()), (json!(6), json!(5), json!(true)));
    assert!(!t.to_string().contains("abc"), "the field's words never leave the compositor: {t}");
    assert_eq!(server.eval(|s| s.ime.asked), 0, "LG's input method was never asked for");
    let model = t["model"].as_u64().unwrap();

    // what the QML client says is not the keyboard's while the Rust client answers it
    say(&server, json!({"op": "shown", "model": model, "x": 0, "y": 626, "w": 1920, "h": 454}));
    a.client.roundtrip();
    assert!(!server.eval(|s| s.keyboard_up()), "a request through the QML client's route is dropped");

    // the Rust client's sheet on the screen: the field is told where, the remote's keys are the keyboard's
    server.eval(move |s| s.shell_rust_handle(Msg::Ime(json!({"op": "shown", "model": model, "x": 0, "y": 626, "w": 1920, "h": 454}))));
    assert!(a.client.wait_until(WAIT, |s| s.ime.panel_states == vec![1]), "{:?}", a.client.state.ime.panel_states);
    assert!(server.eval(|s| s.keyboard_up() && s.ime_grabbing()));
    server.eval(|s| s.key_event(SeatId(0), KEY_RIGHT, true, false));
    assert_eq!(rust_topic(&server, "imekey"), json!({"k": "right", "d": true, "r": false}));
    server.eval(|s| s.key_event(SeatId(0), KEY_RIGHT, false, false));
    assert_eq!(rust_topic(&server, "imekey"), json!({"k": "right", "d": false, "r": false}));
    assert!(server.eval(|s| s.shell.topic("imekey").is_none()), "the QML client hears nothing");

    // what it types: text and Enter, then Back hides it and the field stays
    server.eval(move |s| {
        s.shell_rust_handle(Msg::Ime(json!({"op": "text", "model": model, "text": "h"})));
        s.shell_rust_handle(Msg::Ime(json!({"op": "key", "model": model, "key": "enter"})));
    });
    assert!(a.client.wait_until(WAIT, |s| s.ime.commits == ["h"] && s.ime.keysyms.len() >= 2), "the text and Enter reach the field");
    server.eval(move |s| s.shell_rust_handle(Msg::Ime(json!({"op": "hide", "model": model}))));
    assert!(a.client.wait_until(WAIT, |s| s.ime.panel_states == vec![1, 0]));
    assert_eq!(rust_topic(&server, "ime")["show"], false);
    assert!(!server.eval(|s| s.keyboard_up()));

    let _ = std::fs::remove_dir_all(dir);
}
