//! Native UI topics, system requests and Luna bridge behavior.

mod harness;

use std::time::Duration;
use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::input::code::*;
use lumatv_compositor::keys::SystemRequest;
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::shell::Msg;
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

fn recorded(s: &State, topic: &str) -> Vec<Value> {
    s.shell_rust.events().iter().filter(|e| e["topic"] == topic).map(|e| e["data"].clone()).collect()
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

/// Native Home is covered only by a full-screen window above it.
#[test]
fn the_rust_home_is_uncovered_until_a_full_screen_window_is_over_it() {
    let mut server = Server::start();
    server.eval(|s| {
        s.shell_rust_open(0, "test".into()).unwrap();
        s.shell_rust_handle(Msg::Connected);
        s.scene.set_home_rust(true)
    });
    let foreground = |server: &Server| {
        server.eval(|s| {
            s.tick();
            s.luna_notify();
            s.shell_rust.topic("foreground").cloned()
        })
    };
    let _stub = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_CARD", "Stub", HOME_STUB, 0xffff_0000);
    assert_eq!(foreground(&server), Some(json!({"app": HOME_STUB, "home": true, "covered": false, "video": null})), "nothing full-screen is mapped");
    let mut overlay = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_OVERLAY", "Probe", "org.nphil.lumatv.x1", 0xffff_ffff);
    assert_eq!(foreground(&server), Some(json!({"app": HOME_STUB, "home": true, "covered": false, "video": null})), "a 64x64 overlay covers nothing");
    let big = overlay.client.shm_buffer(1920, 1080, 0xffff_ffff);
    overlay.client.present(&overlay.win.surface, &big);
    assert!(overlay.client.wait_until(WAIT, |s| s.frames_done >= 2));
    assert_eq!(foreground(&server), Some(json!({"app": HOME_STUB, "home": true, "covered": true, "video": null})), "a full-screen overlay covers it");
    let mut app = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_CARD", "Netflix", "netflix", 0xff00_ff00);
    app.client.roundtrip();
    assert_eq!(foreground(&server), Some(json!({"app": "netflix", "home": false, "covered": true, "video": null})));
}

#[test]
fn screensaver_covers_the_app_without_replacing_its_foreground_or_lifecycle() {
    fn foreground(s: &mut State) -> Vec<String> {
        let info = s.luna_method("com.webos.surfacemanager", "getForegroundAppInfo", &json!({}), "com.webos.applicationManager").unwrap();
        info["foregroundAppInfo"].as_array().unwrap().iter().map(|app| app["appId"].as_str().unwrap().to_owned()).collect()
    }
    let mut server = Server::start();
    let mut app = shown(&mut server, true, "_WEBOS_WINDOW_TYPE_CARD", "Player", "org.test.player", 0xff00_ff00);
    assert_eq!(server.eval(foreground), ["org.test.player"]);
    let app_state = server.eval(|s| s.scene.foreground(&s.surfaces).unwrap().state);
    let mut saver = shown(&mut server, false, "_WEBOS_WINDOW_TYPE_SCREENSAVER", "Screen Saver", "com.webos.app.screensaver", 0xffff_0000);
    assert_eq!(saver.client.state.exposed_of.last().map(|(_, rect)| rect.as_slice()), Some(&[0, 0, 64, 64, -1][..]), "the real client is told it may render");
    saver.win.shell_surface.set_fullscreen(wayland_client::protocol::wl_shell_surface::FullscreenMethod::Default, 0, None);
    saver.client.roundtrip();
    assert_eq!(server.eval(foreground), ["org.test.player", "com.webos.app.screensaver"], "SAM must see the saver lifecycle without replacing the fullscreen app");
    assert_eq!(server.eval(|s| s.scene.foreground(&s.surfaces).unwrap().app_id().to_owned()), "org.test.player");
    assert_eq!(server.eval(|s| s.scene.foreground(&s.surfaces).unwrap().state), app_state);
    assert_eq!(server.eval(|s| { let focus = s.scene.focus(&s.surfaces).unwrap(); s.scene.window(focus).unwrap().app_id().to_owned() }), "com.webos.app.screensaver");
    assert_eq!(server.eval(harness::drawn).pixel(5, 5), [0xff, 0, 0, 0xff], "saver covers the retained app");
    server.eval(|s| { s.key_event(SeatId(0), KEY_UP, true, false); });
    saver.client.roundtrip();
    assert!(saver.client.state.keys.is_empty(), "the wake press is handled by the system");
    saver.win.surface.attach(None, 0, 0);
    saver.win.surface.commit();
    saver.client.roundtrip();
    assert_eq!(saver.client.state.exposed_of.last().map(|(_, rect)| rect.as_slice()), Some(&[-1][..]), "unmapping clears exposure");
    assert_eq!(server.eval(foreground), ["org.test.player"]);
    assert_eq!(server.eval(|s| { let focus = s.scene.focus(&s.surfaces).unwrap(); s.scene.window(focus).unwrap().app_id().to_owned() }), "org.test.player");
    assert_eq!(server.eval(harness::drawn).pixel(5, 5), [0, 0xff, 0, 0xff], "dismissal reveals the same app");
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_UP, true, false);
        s.key_event(SeatId(0), KEY_UP, false, false);
    });
    app.client.roundtrip();
    assert!(app.client.state.keys.is_empty(), "wake repeats and release cannot move the revealed app");
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_UP, true, false);
        s.key_event(SeatId(0), KEY_UP, false, false);
    });
    app.client.roundtrip();
    assert_eq!(app.client.state.keys, [(KEY_UP, 1), (KEY_UP, 0)], "the next fresh press controls the retained app");
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
        s.shell_rust_open(0, "test".into()).unwrap();
        s.shell_rust.kinds = vec!["quick".into(), "picker".into()];
        s.shell_rust_handle(Msg::Connected);
        s.shell_rust_handle(Msg::Ui { can: Some(vec!["quick".into(), "picker".into()]), wants: None, live: None, state: Some(json!({"open": ""})) });
        s.system_request(SystemRequest::QuickSettings);
        s.system_request(SystemRequest::InputPicker);
        s.system_request(SystemRequest::GearHold);
    });
    let (shows, launched, ui_state, j1, j2) = server.eval(|s| {
        (
            vec![s.shell_rust.topic("show").cloned().unwrap()],
            launches(s),
            s.shell_rust.ui_state.clone(),
            journal_has(s, "gear tap: Quick Settings -> LumaTV's quick panel"),
            journal_has(s, "Input tap: the input picker -> LumaTV's input picker"),
        )
    });
    assert_eq!(shows[0]["kind"], "picker");
    assert_eq!(
        launched,
        vec!["com.palm.app.settings".to_string()],
        "only the first tap, before a shell was there, went to LG's Settings: the hold opens no LG app"
    );
    assert_eq!(ui_state, json!({"open": ""}));
    assert!(j1 && j2);

    // The gear's press warms the panel; the tap at its release shows it; a repeat of the press does not warm it again.
    let before = server.eval(|s| s.shell_rust.stats.shows);
    server.eval(|s| {
        s.key_event(SeatId(0), KEY_MENU, true, false);
        s.key_event(SeatId(0), KEY_MENU, true, false);
        s.key_event(SeatId(0), KEY_MENU, false, false);
    });
    let show = server.eval(|s| (s.shell_rust.stats.shows, s.shell_rust.topic("show").cloned().unwrap()));
    assert_eq!(show.0, before + 2);
    assert_eq!(show.1["warm"], json!(false));
    app.client.roundtrip();
    assert!(!app.client.state.keys.iter().any(|(c, _)| *c == KEY_MENU), "the gear never reaches the app");

    // The luna methods LG's own QML uses for the same windows reach the shell too.
    server.eval(|s| {
        s.luna_method("com.webos.service.systemUIManager", "launch", &json!({"appId": "com.webos.app.quickinputpicker"}), "x");
        s.luna_method("com.webos.surfacemanager", "showQuickSettings", &json!({}), "x");
    });
    let show = server.eval(|s| s.shell_rust.topic("show").cloned().unwrap());
    assert_eq!(show["kind"], "quick");

    // The client goes: nothing is offered to it any more.
    server.eval(|s| {
        s.shell_rust_handle(Msg::Ui { can: Some(vec![]), wants: None, live: None, state: None });
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
        s.shell_rust.marker = marker;
        s.shell_rust_open(0, "test".into()).unwrap();
        s.shell_rust.kinds = vec!["picker".into(), "quick".into()];
        s.shell_rust_handle(Msg::Connected);
        s.shell_rust_handle(Msg::Ui { can: Some(vec!["picker".into(), "quick".into()]), wants: None, live: None, state: None });
        // The gear's press warms the panel and marks nothing; the instant the shell is asked to show one is marked; one it cannot show is not.
        s.shell_show("quick", true);
        s.shell_show("picker", false);
        s.shell_show("drawer", false);
        for (tag, feed) in [(1, "getToastNotification"), (2, "getAlertNotification"), (3, "getInputAlertNotification")] {
            s.shell_rust_handle(Msg::Luna { tag, uri: format!("luna://com.webos.notification/{feed}"), payload: json!({"subscribe": true}), subscribe: true });
        }
        s.shell_rust.luna_tags.iter().map(|t| t.token).collect::<Vec<_>>()
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
fn lgs_remote_service_opens_only_while_the_owner_allows_it() {
    let server = Server::start();
    let dir = std::env::temp_dir().join(format!("lumatv-remoteservice-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("privacy")).unwrap();
    let data = dir.clone();
    let (sent, refused) = server.eval(move |s| {
        s.config.data_dir = data;
        s.shell_rust_handle(Msg::Connected);
        let remote = "com.webos.app.remoteservice";
        s.shell_rust_handle(Msg::Luna { tag: 1, uri: "luna://com.webos.applicationManager/launch".into(), payload: json!({"id": remote}), subscribe: false });
        s.luna_call("luna://com.webos.applicationManager/launch".into(), json!({"id": remote, "params": {}}));
        s.luna_call("luna://com.webos.applicationManager/launch".into(), json!({"id": "netflix"}));
        (launches(s), (recorded(s, "luna"), journal_has(s, "LG Remote Service is blocked")))
    });
    assert_eq!(sent, vec!["netflix".to_string()], "blocked by default: neither the shell nor the compositor's own calls open it");
    assert_eq!(refused.0.len(), 1, "the shell is told no");
    assert_eq!(refused.0[0]["payload"]["errorText"], "LG Remote Service is blocked");
    assert!(refused.1);
    // The owner allows it (the helper's confirmed switch writes the file): the same launch goes through.
    std::fs::write(dir.join("privacy/remoteservice.allowed"), "on\n").unwrap();
    let sent = server.eval(|s| {
        s.luna_call("luna://com.webos.applicationManager/launch".into(), json!({"id": "com.webos.app.remoteservice"}));
        launches(s)
    });
    assert_eq!(sent.last().map(String::as_str), Some("com.webos.app.remoteservice"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn luna_calls_for_the_shell_go_through_the_bridge_from_the_allowlist() {
    let server = Server::start();
    server.eval(|s| {
        s.shell_rust_open(0, "test".into()).unwrap();
        s.shell_rust_handle(Msg::Connected);
        s.shell_rust_handle(Msg::Luna {
            tag: 7,
            uri: "luna://com.webos.service.audio/master/getVolume".into(),
            payload: json!({"subscribe": true}),
            subscribe: true,
        });
        s.shell_rust_handle(Msg::Luna { tag: 8, uri: "luna://com.webos.applicationManager/launch".into(), payload: json!({"id": "netflix"}), subscribe: false });
        s.shell_rust_handle(Msg::Luna { tag: 9, uri: "luna://com.webos.service.secret/read".into(), payload: json!({}), subscribe: false });
    });
    let (sent, refused, open) = server.eval(|s| (s.luna_sent(), recorded(s, "luna"), s.shell_rust.luna_tags.clone()));
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
    let (answers, open) = server.eval(|s| (recorded(s, "luna"), s.shell_rust.luna_tags.clone()));
    let tags: Vec<u64> = answers.iter().map(|a| a["tag"].as_u64().unwrap()).collect();
    assert_eq!(tags, vec![9, 7, 8, 7]);
    assert_eq!(answers[3]["payload"]["volumeStatus"]["volume"], 4);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].tag, 7);

    // A cancel ends the subscription at the bridge; a client that is gone has every subscription cancelled.
    server.eval(|s| {
        s.shell_rust_handle(Msg::Luna { tag: 10, uri: "luna://com.webos.service.config/getConfigs".into(), payload: json!({"subscribe": true}), subscribe: true });
        s.shell_rust_handle(Msg::Cancel { tag: 7 });
    });
    assert_eq!(server.eval(cancels), vec![first_token]);
    server.eval(|s| s.shell_rust_restart());
    let (cancelled, open, connected) = server.eval(|s| (cancels(s), s.shell_rust.luna_tags.clone(), s.shell_rust.connected));
    assert_eq!(cancelled.len(), 2, "the second subscription was cancelled with the client");
    assert!(open.is_empty());
    assert!(!connected);
}

#[test]
fn a_cancel_that_arrives_before_its_call_leaves_no_subscription() {
    // `/v1/cancel` and `/v1/luna` are separate requests, so the client's cancel can be handled first (SaverVote's close then open: the
    // vote's subscriptions leaked this way, each one a tvpower registration whose questions nobody answered).
    let server = Server::start();
    let uri = "luna://com.webos.service.tvpower/power2/registerScreenSaverRequest";
    server.eval(move |s| {
        s.shell_rust_handle(Msg::Connected);
        s.shell_rust_handle(Msg::Cancel { tag: 40 });
        s.shell_rust_handle(Msg::Luna { tag: 40, uri: uri.into(), payload: json!({"clientName": "x", "subscribe": true}), subscribe: true });
        s.shell_rust_handle(Msg::Luna { tag: 41, uri: uri.into(), payload: json!({"clientName": "x", "subscribe": true}), subscribe: true });
    });
    let (sent, open) = server.eval(move |s| (s.luna_sent().into_iter().filter(|(u, _)| u == uri).count(), s.shell_rust.luna_tags.clone()));
    assert_eq!(sent, 1, "the cancelled call never reaches tvpower");
    assert_eq!(open.iter().map(|t| t.tag).collect::<Vec<_>>(), vec![41]);
    assert_eq!(server.eval(cancels), Vec::<u64>::new(), "nothing was open to cancel at the bridge");
    // the tag is forgotten once used, and the list is bounded
    server.eval(move |s| {
        for t in 100..200 {
            s.shell_rust_handle(Msg::Cancel { tag: t });
        }
        s.shell_rust_handle(Msg::Luna { tag: 100, uri: uri.into(), payload: json!({"subscribe": true}), subscribe: true });
        s.shell_rust_handle(Msg::Luna { tag: 199, uri: uri.into(), payload: json!({"subscribe": true}), subscribe: true });
        s.shell_rust_handle(Msg::Luna { tag: 199, uri: uri.into(), payload: json!({"subscribe": true}), subscribe: true });
    });
    let open = server.eval(|s| s.shell_rust.luna_tags.iter().map(|t| t.tag).collect::<Vec<_>>());
    assert_eq!(open, vec![41, 100, 199], "the oldest early cancels fell off (64 kept); 199 was dropped once");
}

#[test]
fn the_audio_state_and_the_screen_reach_the_shell_as_topics() {
    let server = Server::start();
    server.eval(|s| {
        s.shell_rust_open(0, "test".into()).unwrap();
        s.luna_register();
        s.shell_rust_handle(Msg::Connected);
    });
    let audio_token = server.eval(|s| s.luna_sent().into_iter().position(|(u, _)| u.ends_with("/master/getVolume")).map(|i| i as u64 + 1));
    let token = audio_token.expect("the volume subscription");
    server.eval(move |s| {
        s.luna_event(lumatv_compositor::luna::Event {
            token,
            payload: json!({"volumeStatus": {"volume": 12, "muteStatus": true, "soundOutput": "external_arc", "cause": "volumeUp"}}),
        });
    });
    let (audio, muted, screen) = server.eval(|s| (s.shell_rust.topic("audio").cloned(), s.luna_state.muted, s.shell_rust.topic("screen").cloned()));
    assert_eq!(audio.and_then(|a| a["volumeStatus"]["volume"].as_u64()), Some(12));
    assert!(muted);
    assert_eq!(screen, Some(json!({"on": true})));
}

