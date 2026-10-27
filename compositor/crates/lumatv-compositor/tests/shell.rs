//! The shell client's two windows in the scene (`window.rs`, `scene.rs`) and the UI channel's state on the loop's side
//! (`shell.rs`): LumaTV's Home in the card slot while the Home stub is in front, the system layer's keys and pointer, the
//! gear's and Input's requests answered by the shell, and luna calls made for it through the bridge.

mod harness;

use std::time::Duration;

use harness::{Client, Server, Window, drawn};
use lumatv_compositor::State;
use lumatv_compositor::input::code::*;
use lumatv_compositor::keys::SystemRequest;
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::shell::Msg;
use lumatv_compositor::window::{SHELL_HOME_TITLE, SHELL_SYSTEM_TITLE, WindowType};
use serde_json::{Value, json};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_keyboard::WlKeyboard;

const WAIT: Duration = Duration::from_secs(3);
const HOME_STUB: &str = "com.webos.app.home";

struct Shown {
    client: Client,
    win: Window,
    _kb: WlKeyboard,
    _buf: WlBuffer,
}

/// A client with one window of `kind`, mapped with a 64x64 buffer of one colour, and a keyboard.
fn shown(server: &mut Server, first: bool, kind: &str, title: &str, app_id: &str, argb: u32) -> Shown {
    let mut client = if first { Client::connect(server) } else { Client::connect_extra(server) };
    let kb = client.seat.get_keyboard(&client.qh, ());
    let win = client.card(kind, title);
    win.webos.set_property("appId".into(), app_id.into());
    let buf = client.shm_buffer(64, 64, argb);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "{title} presented");
    Shown { client, win, _kb: kb, _buf: buf }
}

fn focused_title(s: &mut State) -> String {
    s.seat.focus.and_then(|f| s.scene.window_of(f)).and_then(|w| s.scene.window(w)).map(|w| w.title.clone()).unwrap_or_default()
}

fn recorded(s: &State, topic: &str) -> Vec<Value> {
    s.shell.recorded.iter().filter(|e| e["topic"] == topic).map(|e| e["data"].clone()).collect()
}

fn journal_has(s: &State, needle: &str) -> bool {
    s.journal.iter().any(|l| l.contains(needle))
}

fn launches(s: &mut State) -> Vec<String> {
    s.luna_sent().into_iter().filter(|(u, _)| u.ends_with("/launch")).map(|(_, p)| p["id"].as_str().unwrap_or("").to_string()).collect()
}

fn cancels(s: &mut State) -> Vec<u64> {
    s.luna
        .history()
        .into_iter()
        .filter_map(|m| match m {
            lumatv_compositor::luna::ToBridge::Cancel { token } => Some(token),
            _ => None,
        })
        .collect()
}

#[test]
fn lumatv_home_takes_the_card_slot_and_the_keys_while_the_stub_is_in_front() {
    let mut server = Server::start();
    server.eval(|s| s.shell_handle(Msg::Connected));
    // The shell's Home alone: drawn (there is no card at all) and focused.
    let mut home = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_LUMATV_HOME", "LumaHome", "org.nphil.lumatv.shell", 0xff00_00ff);
    assert!(home.client.wait_until(WAIT, |s| s.keyboard_enters >= 1), "Home has the focus with no card in front");
    let (kind, drawn_home, image) = server.eval(|s| {
        let w = s.scene.shell_window(WindowType::ShellHome).map(|w| w.kind);
        let d = s.shell_home_drawn();
        (w, d, drawn(s))
    });
    assert_eq!(kind, Some(WindowType::ShellHome));
    assert!(drawn_home);
    assert_eq!(image.pixel(5, 5), [0, 0, 0xff, 0xff], "LumaTV's Home is on the screen");
    assert!(home.client.wait_until(WAIT, |s| s.exposed_of.iter().any(|(_, ints)| ints == &[0, 0, 64, 64, -1])), "a shell window is exposed once mapped");

    // The Home stub's card maps: SAM's foreground, not drawn (LumaTV's Home takes its slot), and never focused.
    let mut stub = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_CARD", "Stub", HOME_STUB, 0xffff_0000);
    let (fg, drawn_home, focus, image, foreground_topic) = server.eval(|s| {
        s.luna_notify();
        (s.foreground_app(), s.shell_home_drawn(), focused_title(s), drawn(s), s.shell.topic("foreground").cloned())
    });
    assert_eq!(fg, HOME_STUB, "SAM sees the stub in front");
    assert!(drawn_home);
    assert_eq!(focus, "LumaHome", "the keys stay LumaTV's Home's");
    assert_eq!(image.pixel(5, 5), [0, 0, 0xff, 0xff], "the stub's buffer is not drawn over Home");
    assert_eq!(foreground_topic, Some(json!({"app": HOME_STUB, "home": true})));
    assert_eq!(stub.client.state.keyboard_enters, 0);

    // Back and the arrows reach Home as keys; Back closes nothing (not Home, not the stub).
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_PREVIOUS, true, false);
        s.key_event(SeatId(0), KEY_PREVIOUS, false, false);
        s.key_event(SeatId(0), KEY_RIGHT, true, false);
        s.key_event(SeatId(0), KEY_RIGHT, false, false);
    });
    assert!(home.client.wait_until(WAIT, |s| s.keys.contains(&(KEY_PREVIOUS, 1)) && s.keys.contains(&(KEY_RIGHT, 1))), "Back and Right reached Home");
    stub.client.roundtrip();
    assert_eq!(stub.client.state.closes, 0, "the stub was not closed");
    assert_eq!(home.client.state.closes, 0, "Home was not closed");
    let closes = server.eval(|s| s.luna_sent().into_iter().filter(|(u, _)| u.ends_with("/closeByAppId")).count());
    assert_eq!(closes, 0);

    // An app's card maps: it is the foreground, drawn, focused; LumaTV's Home is left out of the scene.
    let mut app = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_CARD", "Netflix", "netflix", 0xff00_ff00);
    assert!(app.client.wait_until(WAIT, |s| s.keyboard_enters >= 1), "the app has the focus");
    let (fg, drawn_home, image, topic) = server.eval(|s| {
        s.luna_notify();
        (s.foreground_app(), s.shell_home_drawn(), drawn(s), s.shell.topic("foreground").cloned())
    });
    assert_eq!(fg, "netflix");
    assert!(!drawn_home);
    assert_eq!(image.pixel(5, 5), [0, 0xff, 0, 0xff]);
    assert_eq!(topic, Some(json!({"app": "netflix", "home": false})));
    // Back on the app closes it, as before.
    server.eval(|s| s.key_event(SeatId(0), KEY_PREVIOUS, true, false));
    assert!(app.client.wait_until(WAIT, |s| s.closes >= 1), "the app got close");

    // The app's card goes: the stub is the card again and Home is drawn in its place, with the focus.
    app.win.surface.destroy();
    app.client.roundtrip();
    let (drawn_home, focus, image) = server.eval(|s| (s.shell_home_drawn(), focused_title(s), drawn(s)));
    assert!(drawn_home);
    assert_eq!(focus, "LumaHome");
    assert_eq!(image.pixel(5, 5), [0, 0, 0xff, 0xff]);
    drop(home.win);
}

#[test]
fn a_plain_window_is_a_shell_window_by_its_title() {
    let mut server = Server::start();
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "x");
    win.webos.set_property("title".into(), SHELL_SYSTEM_TITLE.into());
    let other = client.card("_WEBOS_WINDOW_TYPE_CARD", "y");
    other.webos.set_property("title".into(), SHELL_HOME_TITLE.into());
    client.roundtrip();
    let kinds: Vec<WindowType> = server.eval(|s| s.scene.windows().map(|w| w.kind).collect());
    assert!(kinds.contains(&WindowType::ShellSystem));
    assert!(kinds.contains(&WindowType::ShellHome));
}

#[test]
fn the_system_layer_takes_keys_and_pointer_only_while_it_wants_them() {
    let mut server = Server::start();
    let mut app = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_CARD", "App", "org.test.app", 0xffff_0000);
    assert!(app.client.wait_until(WAIT, |s| s.keyboard_enters >= 1));
    let mut layer = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM", "Layer", "org.nphil.lumatv.shell", 0x8000_0080);
    // Mapped above the app, but neither the focus nor the pointer are its.
    let (focus, under) = server.eval(|s| {
        s.tick();
        (focused_title(s), s.scene.surface_at(10, 10).and_then(|(sid, _)| s.scene.window_of(sid)).and_then(|w| s.scene.window(w)).map(|w| w.title.clone()))
    });
    assert_eq!(focus, "App");
    assert_eq!(under.as_deref(), Some("App"), "the pointer goes through the transparent layer");
    assert_eq!(layer.client.state.keyboard_enters, 0);

    // The UI says its layer wants the keys: the focus moves to it, the pointer finds it.
    server.eval(|s| s.shell_handle(Msg::Ui { can: None, wants: Some(true), state: None }));
    assert!(layer.client.wait_until(WAIT, |s| s.keyboard_enters >= 1), "the layer got the focus");
    let (focus, under, wants) = server.eval(|s| {
        s.tick();
        (
            focused_title(s),
            s.scene.surface_at(10, 10).and_then(|(sid, _)| s.scene.window_of(sid)).and_then(|w| s.scene.window(w)).map(|w| w.title.clone()),
            s.shell.wants,
        )
    });
    assert_eq!(focus, "Layer");
    assert_eq!(under.as_deref(), Some("Layer"));
    assert!(wants);
    // Back goes to the layer as a key and closes nothing; Exit as well.
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_PREVIOUS, true, false);
        s.key_event(SeatId(0), KEY_PREVIOUS, false, false);
        s.key_event(SeatId(0), KEY_EXIT, true, false);
        s.key_event(SeatId(0), KEY_EXIT, false, false);
    });
    assert!(layer.client.wait_until(WAIT, |s| s.keys.contains(&(KEY_PREVIOUS, 1)) && s.keys.contains(&(KEY_EXIT, 1))));
    app.client.roundtrip();
    assert_eq!(app.client.state.closes, 0);
    // Volume stays the compositor's.
    server.eval(|s| s.key_event(SeatId(0), KEY_VOLUMEUP, true, false));
    layer.client.roundtrip();
    assert!(!layer.client.state.keys.contains(&(KEY_VOLUMEUP, 1)));
    let volume_calls = server.eval(|s| s.luna_sent().into_iter().filter(|(u, _)| u.ends_with("/master/volumeUp")).count());
    assert_eq!(volume_calls, 1);

    // It lets go: the app has the keys again.
    server.eval(|s| s.shell_handle(Msg::Ui { can: None, wants: Some(false), state: None }));
    assert!(app.client.wait_until(WAIT, |s| s.keyboard_enters >= 2), "the app got the focus back");
    drop(layer.win);
}

#[test]
fn the_gear_and_input_go_to_the_shell_when_it_can_show_the_panel() {
    let mut server = Server::start();
    let mut app = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_CARD", "App", "org.test.app", 0xffff_0000);
    assert!(app.client.wait_until(WAIT, |s| s.keyboard_enters >= 1));
    // No client connected: the interim route (LG's Settings app for the gear's tap).
    server.eval(|s| s.system_request(SystemRequest::QuickSettings));
    assert_eq!(server.eval(launches), vec!["com.palm.app.settings".to_string()]);

    // The client is connected and can show the quick panel and the picker: the requests go to it as `show` events.
    server.eval(|s| {
        s.shell_handle(Msg::Connected);
        s.shell_handle(Msg::Ui { can: Some(vec!["quick".into(), "picker".into()]), wants: None, state: Some(json!({"open": ""})) });
        s.system_request(SystemRequest::QuickSettings);
        s.system_request(SystemRequest::InputPicker);
        s.system_request(SystemRequest::Settings);
    });
    let (shows, launched, ui_state, j1, j2) = server.eval(|s| {
        (
            recorded(s, "show"),
            launches(s),
            s.shell.ui_state.clone(),
            journal_has(s, "gear tap: Quick Settings -> LumaTV's quick panel"),
            journal_has(s, "Input tap: the input picker -> LumaTV's input picker"),
        )
    });
    assert_eq!(shows.iter().map(|d| d["kind"].as_str().unwrap_or("")).collect::<Vec<_>>(), vec!["quick", "picker"]);
    assert_eq!(
        launched,
        vec!["com.palm.app.settings".to_string(), "com.palm.app.settings".to_string()],
        "the drawer is not offered: the gear's hold went to LG's Settings"
    );
    assert_eq!(ui_state, json!({"open": ""}));
    assert!(j1 && j2);

    // The gear's press warms the panel; the tap at its release shows it; a repeat of the press does not warm it again.
    let before = server.eval(|s| recorded(s, "show").len());
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_MENU, true, false);
        s.key_event(SeatId(0), KEY_MENU, true, false);
        s.key_event(SeatId(0), KEY_MENU, false, false);
    });
    let shows = server.eval(|s| recorded(s, "show"));
    assert_eq!(shows.len(), before + 2);
    assert_eq!(shows[before]["warm"], json!(true));
    assert_eq!(shows[before + 1]["warm"], json!(false));
    app.client.roundtrip();
    assert!(!app.client.state.keys.iter().any(|(c, _)| *c == KEY_MENU), "the gear never reaches the app");

    // The luna methods LG's own QML uses for the same windows reach the shell too.
    server.eval(|s| {
        s.luna_method("com.webos.service.systemUIManager", "launch", &json!({"appId": "com.webos.app.quickinputpicker"}), "x");
        s.luna_method("com.webos.surfacemanager", "showQuickSettings", &json!({}), "x");
    });
    let shows = server.eval(|s| recorded(s, "show"));
    assert_eq!(shows[shows.len() - 2]["kind"], "picker");
    assert_eq!(shows[shows.len() - 1]["kind"], "quick");

    // The client goes: nothing is offered to it any more.
    server.eval(|s| {
        s.shell_handle(Msg::Ui { can: Some(vec![]), wants: None, state: None });
        s.system_request(SystemRequest::InputPicker);
    });
    let launched = server.eval(launches);
    assert_eq!(launched.last().map(String::as_str), Some("com.webos.app.homeconnect"));
}

#[test]
fn luna_calls_for_the_shell_go_through_the_bridge_from_the_allowlist() {
    let server = Server::start();
    server.eval(|s| {
        s.shell_handle(Msg::Connected);
        s.shell_handle(Msg::Luna {
            tag: 7,
            uri: "luna://com.webos.service.audio/master/getVolume".into(),
            payload: json!({"subscribe": true}),
            subscribe: true,
        });
        s.shell_handle(Msg::Luna { tag: 8, uri: "luna://com.webos.applicationManager/launch".into(), payload: json!({"id": "netflix"}), subscribe: false });
        s.shell_handle(Msg::Luna { tag: 9, uri: "luna://com.webos.service.secret/read".into(), payload: json!({}), subscribe: false });
    });
    let (sent, refused, open) = server.eval(|s| (s.luna_sent(), recorded(s, "luna"), s.shell.luna_tags.clone()));
    assert!(sent.iter().any(|(u, p)| u == "luna://com.webos.service.audio/master/getVolume" && p["subscribe"] == json!(true)));
    assert!(sent.iter().any(|(u, p)| u == "luna://com.webos.applicationManager/launch" && p["id"] == "netflix"));
    assert!(!sent.iter().any(|(u, _)| u.contains("secret")), "a service off the allowlist is not called");
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0]["tag"], 9);
    assert_eq!(refused[0]["payload"]["returnValue"], json!(false));
    assert_eq!(open.len(), 2);

    // The bridge's answers come back under the tag; a one-shot call's tag is then forgotten, a subscription's kept.
    let tokens: Vec<u64> = open.iter().map(|t| t.token).collect();
    let first_token = tokens[0];
    server.eval(move |s| {
        s.luna_event(lumatv_compositor::luna::Event { token: tokens[0], payload: json!({"volumeStatus": {"volume": 3}}) });
        s.luna_event(lumatv_compositor::luna::Event { token: tokens[1], payload: json!({"returnValue": true}) });
        s.luna_event(lumatv_compositor::luna::Event { token: tokens[0], payload: json!({"volumeStatus": {"volume": 4}}) });
    });
    let (answers, open) = server.eval(|s| (recorded(s, "luna"), s.shell.luna_tags.clone()));
    let tags: Vec<u64> = answers.iter().map(|a| a["tag"].as_u64().unwrap()).collect();
    assert_eq!(tags, vec![9, 7, 8, 7]);
    assert_eq!(answers[3]["payload"]["volumeStatus"]["volume"], 4);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].tag, 7);

    // A cancel ends the subscription at the bridge; a client that is gone has every subscription cancelled.
    server.eval(|s| {
        s.shell_handle(Msg::Luna { tag: 10, uri: "luna://com.webos.service.config/getConfigs".into(), payload: json!({"subscribe": true}), subscribe: true });
        s.shell_handle(Msg::Cancel { tag: 7 });
    });
    assert_eq!(server.eval(cancels), vec![first_token]);
    server.eval(|s| s.shell_restart());
    let (cancelled, open, connected) = server.eval(|s| (cancels(s), s.shell.luna_tags.clone(), s.shell.connected));
    assert_eq!(cancelled.len(), 2, "the second subscription was cancelled with the client");
    assert!(open.is_empty());
    assert!(!connected);
}

#[test]
fn the_audio_state_and_the_screen_reach_the_shell_as_topics() {
    let server = Server::start();
    server.eval(|s| {
        s.luna_register();
        s.shell_handle(Msg::Connected);
    });
    let audio_token = server.eval(|s| s.luna_sent().into_iter().position(|(u, _)| u.ends_with("/master/getVolume")).map(|i| i as u64 + 1));
    let token = audio_token.expect("the volume subscription");
    server.eval(move |s| {
        s.luna_event(lumatv_compositor::luna::Event {
            token,
            payload: json!({"volumeStatus": {"volume": 12, "muteStatus": true, "soundOutput": "external_arc", "cause": "volumeUp"}}),
        });
    });
    let (audio, muted, screen) = server.eval(|s| (s.shell.topic("audio").cloned(), s.luna_state.muted, s.shell.topic("screen").cloned()));
    assert_eq!(audio.and_then(|a| a["volumeStatus"]["volume"].as_u64()), Some(12));
    assert!(muted);
    assert_eq!(screen, Some(json!({"on": true})));
}
