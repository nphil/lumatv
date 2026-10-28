//! The shell client's two windows in the scene (`window.rs`, `scene.rs`) and the UI channel's state on the loop's side
//! (`shell.rs`): LumaTV's Home in the card slot while the Home stub is in front, the system layer's keys and pointer, the
//! gear's and Input's requests answered by the shell, and luna calls made for it through the bridge.

mod harness;

use std::time::Duration;

use harness::{Client, Server, Window, drawn};
use lumatv_compositor::State;
use lumatv_compositor::input::code::*;
use lumatv_compositor::keys::SystemRequest;
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::render::software::Software;
use lumatv_compositor::scene::Plan;
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::shell::Msg;
use lumatv_compositor::surface::Surfaces;
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
    assert_eq!(foreground_topic, Some(json!({"app": HOME_STUB, "home": true, "covered": false})));
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
    assert_eq!(topic, Some(json!({"app": "netflix", "home": false, "covered": true})));
    // Back on the app closes it, as before.
    server.eval(|s| s.key_event(SeatId(0), KEY_PREVIOUS, true, false));
    assert!(app.client.wait_until(WAIT, |s| s.closes >= 1), "the app got close");

    // The app's card goes: the stub is the card again and Home is drawn in its place, with the focus.
    app.win.surface.destroy();
    app.client.roundtrip();
    let (drawn_home, focus, image, commits) = server.eval(|s| (s.shell_home_drawn(), focused_title(s), drawn(s), s.shell_report()["homeCommits"].as_u64()));
    assert!(drawn_home);
    assert_eq!(focus, "LumaHome");
    assert_eq!(image.pixel(5, 5), [0, 0, 0xff, 0xff]);
    assert_eq!(commits, Some(1), "the Home window's commits are counted for the idle-cost check");

    // A full-screen overlay over Home (LG's Settings, the gate's X1 probe): Home stays drawn under it but is covered, so the client
    // stops drawing it; a small one leaves it uncovered.
    let mut overlay = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_OVERLAY", "Probe", "org.nphil.lumatv.x1", 0xffff_ffff);
    assert!(overlay.client.wait_until(WAIT, |s| s.keyboard_enters >= 1));
    let topic = server.eval(|s| {
        s.luna_notify();
        s.shell.topic("foreground").cloned()
    });
    assert_eq!(topic, Some(json!({"app": HOME_STUB, "home": true, "covered": false})), "a 64x64 overlay covers nothing");
    let big = overlay.client.shm_buffer(1920, 1080, 0xffff_ffff);
    overlay.client.present(&overlay.win.surface, &big);
    assert!(overlay.client.wait_until(WAIT, |s| s.frames_done >= 2));
    let topic = server.eval(|s| {
        s.tick();
        s.luna_notify();
        s.shell.topic("foreground").cloned()
    });
    assert_eq!(topic, Some(json!({"app": HOME_STUB, "home": true, "covered": true})), "a full-screen overlay covers Home");
    drop(big);
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
    server.eval(|s| s.shell_handle(Msg::Ui { can: None, wants: Some(true), live: Some(true), state: None }));
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
    server.eval(|s| s.shell_handle(Msg::Ui { can: None, wants: Some(false), live: None, state: None }));
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
        s.shell_handle(Msg::Ui { can: Some(vec!["quick".into(), "picker".into()]), wants: None, live: None, state: Some(json!({"open": ""})) });
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
        s.shell_handle(Msg::Ui { can: Some(vec![]), wants: None, live: None, state: None });
        s.system_request(SystemRequest::InputPicker);
    });
    let launched = server.eval(launches);
    assert_eq!(launched.last().map(String::as_str), Some("com.webos.app.homeconnect"));
}

#[test]
fn a_panel_asked_for_and_a_notification_handed_over_leave_markers_in_the_trace_buffer() {
    let server = Server::start();
    let path = std::env::temp_dir().join(format!("lumatv-trace-marker-{}", std::process::id()));
    std::fs::write(&path, "").unwrap();
    let marker = path.clone();
    let tokens = server.eval(move |s| {
        s.shell.marker = marker;
        s.shell_handle(Msg::Connected);
        s.shell_handle(Msg::Ui { can: Some(vec!["picker".into(), "quick".into()]), wants: None, live: None, state: None });
        // The gear's press warms the panel and marks nothing; the instant the shell is asked to show one is marked; one it cannot show is not.
        s.shell_show("quick", true);
        s.shell_show("picker", false);
        s.shell_show("drawer", false);
        for (tag, feed) in [(1, "getToastNotification"), (2, "getAlertNotification"), (3, "getInputAlertNotification")] {
            s.shell_handle(Msg::Luna { tag, uri: format!("luna://com.webos.notification/{feed}"), payload: json!({"subscribe": true}), subscribe: true });
        }
        s.shell.luna_tags.iter().map(|t| t.token).collect::<Vec<_>>()
    });
    server.eval(move |s| {
        // A subscription's first answer carries no notification; a toast with a message does, an alert that closes does not, one that opens does.
        let answer = |token: u64, payload: Value| lumatv_compositor::luna::Event { token, payload };
        s.luna_event(answer(tokens[0], json!({"returnValue": true, "subscribed": true})));
        s.luna_event(answer(tokens[0], json!({"returnValue": true, "message": "A toast"})));
        s.luna_event(answer(tokens[1], json!({"alertAction": "close"})));
        s.luna_event(answer(tokens[1], json!({"alertAction": "open", "alertInfo": {}})));
        s.luna_event(answer(tokens[2], json!({"alertAction": "open", "alertInfo": {}})));
    });
    let text = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        vec!["Q:picker-show", "Q:toast-delivered", "Q:alert-delivered"],
        "the input popup is not one of the gate's surfaces"
    );
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

#[test]
fn the_system_layer_is_drawn_only_while_live_and_joins_without_a_frame_of_its_own() {
    let mut server = Server::start();
    let app = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_CARD", "App", "org.test.app", 0xffff_0000);
    let mut layer = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM", "Layer", "org.nphil.lumatv.shell", 0xff00_00ff);
    // From here the frames are the test's own ticks, so a frame drawn (or not) is accounted for.
    server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    let red = [255, 0, 0, 255];
    let blue = [0, 0, 255, 255];
    assert_eq!(server.eval(drawn).pixel(10, 10), blue, "live until the shell says otherwise");

    // Nothing to show: the layer leaves the stack, its window still mapped; a commit meanwhile is taken but not drawn.
    let ui = |live: bool| Msg::Ui { can: None, wants: None, live: Some(live), state: None };
    server.eval(move |s| s.shell_handle(ui(false)));
    assert_eq!(server.eval(drawn).pixel(10, 10), red);
    let mapped = server.eval(|s| s.scene.shell_window(WindowType::ShellSystem).is_some_and(|w| w.map_serial > 0));
    assert!(mapped);
    let green = layer.client.shm_buffer(64, 64, 0xff00_ff00);
    layer.client.present(&layer.win.surface, &green);
    layer.client.roundtrip();
    let (image, drawn_before) = server.eval(|s| (drawn(s), s.stats.frames_drawn));
    assert_eq!(image.pixel(10, 10), red, "a commit of an idle layer is not drawn");
    assert!(layer.client.wait_until(WAIT, |s| s.frames_done >= 2), "its callback goes out all the same");

    // Live again within 100 ms of that commit: the commit was the frame being announced, so the join draws it.
    let drew_now = server.eval(move |s| s.shell_handle(ui(true)));
    assert!(!drew_now, "the software output draws at the tick, not on damage");
    let (image, drawn_after) = server.eval(|s| (drawn(s), s.stats.frames_drawn));
    assert_eq!(image.pixel(10, 10), [0, 255, 0, 255]);
    assert_eq!(drawn_after, drawn_before + 1);

    // Idle again, then live long after its last commit: the layer joins quietly (no frame for the join), and its next
    // commit is the frame that shows it.
    server.eval(move |s| s.shell_handle(ui(false)));
    assert_eq!(server.eval(drawn).pixel(10, 10), red);
    std::thread::sleep(Duration::from_millis(450));
    server.eval(move |s| s.shell_handle(ui(true)));
    let (image, drawn_quiet, before) = server.eval(|s| {
        let before = s.stats.frames_drawn;
        (drawn(s), s.stats.frames_drawn, before)
    });
    assert_eq!(drawn_quiet, before, "no frame for the join alone");
    assert_eq!(image.pixel(10, 10), red);
    let blue_again = layer.client.shm_buffer(64, 64, 0xff00_00ff);
    layer.client.present(&layer.win.surface, &blue_again);
    layer.client.roundtrip();
    assert_eq!(server.eval(drawn).pixel(10, 10), blue, "the next commit shows the layer");
    drop(app);
}

/// The software output saying what KMS says between frames: the screen is still, so a commit is drawn at once.
struct OnDamage(Software);

impl Output for OnDamage {
    fn name(&self) -> &'static str {
        "on-damage"
    }
    fn size(&self) -> (u32, u32) {
        self.0.size()
    }
    fn query_buffer(&mut self, buffer: &wayland_server::protocol::wl_buffer::WlBuffer) -> Option<((i32, i32), bool)> {
        self.0.query_buffer(buffer)
    }
    fn forget_buffer(&mut self, buffer: &wayland_server::protocol::wl_buffer::WlBuffer) {
        self.0.forget_buffer(buffer)
    }
    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> std::io::Result<RenderStats> {
        self.0.render(plan, surfaces)
    }
    fn present(&mut self) -> std::io::Result<Presented> {
        self.0.present()
    }
    fn capture(&mut self) -> std::io::Result<Image> {
        self.0.capture()
    }
    fn draws_on_damage(&self, _now: std::time::Instant) -> bool {
        true
    }
}

#[test]
fn the_state_in_the_report_that_brings_the_layer_live_is_kept_when_the_join_draws_a_frame() {
    // Full gate of 2026-10-28: Volume+ left `osd false` for good. The layer's frame had come in ahead of the word on the
    // channel, so the join drew it and returned, and the state in the same report (`osd true`) was dropped; the client
    // never sends an identical report again, so the compositor kept the old state.
    let mut server = Server::start_with(Box::new(|| Box::new(OnDamage(Software::new(1920, 1080)))));
    let app = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_CARD", "App", "org.test.app", 0xffff_0000);
    let mut layer = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM", "Layer", "org.nphil.lumatv.shell", 0xff00_00ff);
    server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    server.eval(|s| s.shell_handle(Msg::Ui { can: None, wants: None, live: Some(false), state: Some(json!({"osd": false})) }));
    // The layer draws its volume display and commits; the word that it is live follows with the state.
    let green = layer.client.shm_buffer(64, 64, 0xff00_ff00);
    layer.client.present(&layer.win.surface, &green);
    layer.client.roundtrip();
    let drew = server.eval(|s| s.shell_handle(Msg::Ui { can: None, wants: None, live: Some(true), state: Some(json!({"osd": true})) }));
    assert!(drew, "the join drew the frame that was already in");
    let osd = server.eval(|s| s.shell.ui_state.get("osd").cloned());
    assert_eq!(osd, Some(json!(true)), "the state of the report that returned early is kept");
    drop(app);
}

#[test]
fn a_frame_of_the_layer_that_came_250_ms_before_the_word_that_it_is_live_is_drawn_by_the_join() {
    // The toast's frame is made by qml-runner (nice 10) and the word that the layer is live follows it by the time its own
    // request takes: under load that is well over the 100 ms the join once allowed, and the frame, the only one a toast
    // makes, waited for a commit that never came (61-118 ms from the delivery on the 2026-10-28 gate).
    let mut server = Server::start_with(Box::new(|| Box::new(OnDamage(Software::new(1920, 1080)))));
    let app = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_CARD", "App", "org.test.app", 0xffff_0000);
    let mut layer = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM", "Layer", "org.nphil.lumatv.shell", 0xff00_00ff);
    server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    server.eval(|s| s.shell_handle(Msg::Ui { can: None, wants: None, live: Some(false), state: None }));
    let green = layer.client.shm_buffer(64, 64, 0xff00_ff00);
    layer.client.present(&layer.win.surface, &green);
    layer.client.roundtrip();
    std::thread::sleep(Duration::from_millis(250));
    let drew = server.eval(|s| s.shell_handle(Msg::Ui { can: None, wants: None, live: Some(true), state: None }));
    assert!(drew, "the join draws the commit that is waiting");
    assert_eq!(server.eval(drawn).pixel(10, 10), [0, 255, 0, 255]);
    assert!(server.eval(|s| s.journal.iter().any(|l| l.contains("shell: system layer live; its last commit") && l.contains("join draws it"))));
    drop(app);
}

#[test]
fn a_layer_that_wanted_the_keys_while_idle_gets_them_when_it_goes_live() {
    // Full gate of 2026-10-28, the quick panel over an app: the layer had flickered idle, its `wants` word came before its
    // `live` word, nothing moved the focus when it joined, and Back went to the app instead of the panel.
    let mut server = Server::start();
    let mut app = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_CARD", "App", "org.test.app", 0xffff_0000);
    assert!(app.client.wait_until(WAIT, |s| s.keyboard_enters >= 1));
    let layer = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM", "Layer", "org.nphil.lumatv.shell", 0x8000_0080);
    server.eval(|s| s.shell_handle(Msg::Ui { can: None, wants: None, live: Some(false), state: None }));
    server.eval(|s| s.shell_handle(Msg::Ui { can: None, wants: Some(true), live: None, state: None }));
    assert_eq!(
        server.eval(|s| {
            s.tick();
            focused_title(s)
        }),
        "App",
        "an idle layer is not in the stack"
    );
    server.eval(|s| s.shell_handle(Msg::Ui { can: None, wants: None, live: Some(true), state: None }));
    assert_eq!(
        server.eval(|s| {
            s.tick();
            focused_title(s)
        }),
        "Layer",
        "it has the keys it asked for once it is live"
    );
    drop((app, layer));
}
