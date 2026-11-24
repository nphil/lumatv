//! A hidden preload (`src/preload.rs`): the window of an app SAM started in the background maps but is never raised, shown or focused until the app is
//! launched. Marquee and Apple TV ask for fullscreen as they start, whatever the preload mode says (2026-11-23).

mod harness;

use std::time::Duration;

use harness::{Client, Server, Window};
use lumatv_compositor::luna::{Event, ToBridge};
use lumatv_compositor::server::webos_shell::front_app;
use lumatv_compositor::window::ShellState;
use serde_json::{Value, json};
use wayland_client::protocol::wl_buffer::WlBuffer;

const WAIT: Duration = Duration::from_secs(3);
const BASE: &str = "org.test.base";
const MARQUEE: &str = "com.beb.plxnative";
const HOME: &str = "com.webos.app.home";
const CARD: &str = "_WEBOS_WINDOW_TYPE_CARD";
const FULLSCREEN: u32 = 3;

/// An app in front with the keyboard, and the compositor subscribed to SAM's life events as it is once the bridge is up.
struct Rig {
    server: Server,
    _base: Client,
    _win: Window,
    _buf: WlBuffer,
}

fn rig() -> Rig {
    let mut server = Server::start();
    let mut base = Client::connect(&mut server);
    let _kb = base.seat.get_keyboard(&base.qh, ());
    let win = base.card_as(CARD, "Base", BASE);
    let buf = base.shm_buffer(1920, 1080, 0xff20_3060);
    base.present(&win.surface, &buf);
    assert!(base.wait_until(WAIT, |s| s.keyboard_enters >= 1), "the base app has the keyboard");
    server.eval(|s| s.luna_register());
    Rig { server, _base: base, _win: win, _buf: buf }
}

/// A new client, a preload's or not: the compositor reads that from the process's command line, which is the test's, so it is set here.
fn client(rig: &mut Rig, preload: bool) -> Client {
    let client = Client::connect_extra(&mut rig.server);
    rig.server.eval(move |s| s.clients.values_mut().max_by_key(|c| c.since).unwrap().preload = preload);
    client
}

/// A card of `app`, with `fullscreen` asked or not, and a first frame or not.
struct App {
    client: Client,
    win: Window,
    _buf: Option<WlBuffer>,
}

fn card(client: Client, app: &str, fullscreen: bool, frame: bool) -> App {
    let mut client = client;
    let win = client.card_as(CARD, app, app);
    if fullscreen {
        win.webos.set_state(FULLSCREEN);
    }
    let buf = frame.then(|| client.shm_buffer(1920, 1080, 0xffff_0000));
    if let Some(buf) = &buf {
        client.present(&win.surface, buf);
    }
    client.roundtrip();
    App { client, win, _buf: buf }
}

fn front(rig: &Rig) -> Option<String> {
    rig.server.eval(|s| s.scene.foreground(&s.surfaces).map(|w| w.app_id().to_string()))
}

fn state_of(rig: &Rig, app: &'static str) -> (bool, bool, ShellState) {
    rig.server.eval(move |s| {
        let w = s.scene.windows().find(|w| w.app_id() == app).expect("the app's window");
        (w.preloaded, w.front > 0, w.state)
    })
}

fn life(rig: &Rig, payload: Value) {
    rig.server.eval(move |s| {
        let token = s
            .luna
            .history()
            .iter()
            .find_map(|m| match m {
                ToBridge::Outbound(o) if o.uri.ends_with("/getAppLifeEvents") && o.subscribe => Some(o.token),
                _ => None,
            })
            .expect("the compositor subscribed to SAM's life events");
        s.luna_event(Event { token, payload });
    });
}

fn event(name: &str, app: &str) -> Value {
    json!({"event": name, "appId": app, "returnValue": true, "reason": "undefined"})
}

fn focus_is(rig: &Rig, app: &'static str) -> bool {
    rig.server.eval(move |s| {
        let surface = s.scene.windows().find(|w| w.app_id() == app).map(|w| w.surface);
        surface.is_some() && s.seat.focus == surface
    })
}

#[test]
fn a_preload_that_asks_for_fullscreen_maps_and_is_never_shown() {
    let mut rig = rig();
    let app = card(client(&mut rig, true), MARQUEE, true, true);
    assert_eq!(front(&rig).as_deref(), Some(BASE), "Home's place is not taken");
    let (held, fronted, state) = state_of(&rig, MARQUEE);
    assert!(held && !fronted, "held, and never at the front");
    assert_eq!(state, ShellState::Fullscreen, "answered as it asked");
    assert!(!focus_is(&rig, MARQUEE), "no keyboard focus");
    let mapped = rig.server.eval(|s| s.scene.windows().find(|w| w.app_id() == MARQUEE).unwrap().map_serial > 0);
    assert!(mapped, "its frame is kept");
    let stack = rig.server.eval(|s| s.scene.windows().filter(|w| w.app_id() == MARQUEE).count());
    assert_eq!(stack, 1);
    // exposed until its first frame (a Qt window renders only then), covered after it
    let exposed: Vec<&Vec<i32>> = app.client.state.exposed_of.iter().map(|(_, rects)| rects).collect();
    assert_eq!(exposed.len(), 2, "{exposed:?}");
    assert!(exposed[0].len() == 5 && exposed[0][2] > 0 && exposed[0][3] > 0, "{exposed:?}");
    assert_eq!(exposed[1], &vec![-1]);
    let _ = &app.win;
}

#[test]
fn the_apps_launch_shows_the_kept_frame_at_once() {
    let mut rig = rig();
    let mut app = card(client(&mut rig, true), MARQUEE, true, true);
    life(&rig, event("launch", MARQUEE));
    app.client.roundtrip();
    assert_eq!(front(&rig).as_deref(), Some(MARQUEE));
    let (held, fronted, state) = state_of(&rig, MARQUEE);
    assert!(!held && fronted && state == ShellState::Fullscreen);
    assert_eq!(state_of(&rig, BASE).2, ShellState::Minimized, "the card it covers is minimized");
    assert!(focus_is(&rig, MARQUEE), "the keyboard follows");
    assert_eq!(app.client.state.exposed_of.last().map(|(_, r)| r.clone()), Some(vec![0, 0, 1920, 1080, -1]), "it draws again");
}

#[test]
fn a_launch_that_is_a_preloads_does_not_show_it() {
    let mut rig = rig();
    let _app = card(client(&mut rig, true), MARQUEE, true, true);
    life(&rig, event("preload", MARQUEE));
    life(&rig, json!({"event": "relaunch", "appId": MARQUEE, "parameters": {"preload": "partial"}}));
    life(&rig, json!({"event": "launch", "appId": MARQUEE, "reason": "preload"}));
    life(&rig, event("background", MARQUEE));
    life(&rig, event("launch", "some.other.app"));
    assert_eq!(front(&rig).as_deref(), Some(BASE));
    assert!(state_of(&rig, MARQUEE).0, "still held");
}

#[test]
fn an_app_that_honors_the_preload_is_raised_by_its_own_request_after_the_launch() {
    // Netflix: no fullscreen request and no frame until SAM relaunches it
    let mut rig = rig();
    let mut app = card(client(&mut rig, true), MARQUEE, false, false);
    assert!(state_of(&rig, MARQUEE).0);
    assert_eq!(app.client.state.exposed_of.len(), 0, "it is not exposed before it asked for the screen");
    life(&rig, event("launch", MARQUEE));
    assert_eq!(front(&rig).as_deref(), Some(BASE), "nothing to show yet: it has no frame and asked for nothing");
    app.win.webos.set_state(FULLSCREEN);
    app.client.roundtrip();
    assert_eq!(front(&rig).as_deref(), Some(MARQUEE), "its own request is honored now");
}

#[test]
fn a_window_made_after_the_launch_is_shown_like_any() {
    let mut rig = rig();
    let mut client = client(&mut rig, true);
    let first = client.card_as(CARD, "First", MARQUEE);
    client.roundtrip();
    life(&rig, event("launch", MARQUEE));
    let second = client.card_as(CARD, "Second", MARQUEE);
    second.webos.set_state(FULLSCREEN);
    let buf = client.shm_buffer(1920, 1080, 0xff00_ff00);
    client.present(&second.surface, &buf);
    client.roundtrip();
    let held = rig.server.eval(|s| s.scene.windows().filter(|w| w.app_id() == MARQUEE).map(|w| w.preloaded).collect::<Vec<_>>());
    assert_eq!(held, [false, false], "the process is no preload after its launch");
    assert_eq!(front(&rig).as_deref(), Some(MARQUEE));
    let _ = first;
}

#[test]
fn raising_the_app_shows_a_held_window_without_a_launch() {
    let mut rig = rig();
    let _app = card(client(&mut rig, true), MARQUEE, true, true);
    let shown = rig.server.eval(|s| front_app(s, MARQUEE));
    assert!(shown);
    assert_eq!(front(&rig).as_deref(), Some(MARQUEE));
    assert!(!state_of(&rig, MARQUEE).0);
}

#[test]
fn a_process_that_was_not_started_as_a_preload_is_raised_as_before() {
    let mut rig = rig();
    let _app = card(client(&mut rig, false), MARQUEE, true, true);
    assert_eq!(front(&rig).as_deref(), Some(MARQUEE));
    assert!(!state_of(&rig, MARQUEE).0);
}

#[test]
fn nothing_is_held_while_sams_life_events_are_not_subscribed() {
    let mut server = Server::start();
    let mut base = Client::connect(&mut server);
    let win = base.card_as(CARD, "Base", BASE);
    let buf = base.shm_buffer(1920, 1080, 0xff20_3060);
    base.present(&win.surface, &buf);
    base.roundtrip();
    let mut rig = Rig { server, _base: base, _win: win, _buf: buf };
    let _app = card(client(&mut rig, true), MARQUEE, true, true);
    assert_eq!(front(&rig).as_deref(), Some(MARQUEE), "a hold nothing could end would leave the app hidden for good");
}

#[test]
fn a_launch_heard_while_the_process_was_starting_lets_its_window_go() {
    let mut rig = rig();
    life(&rig, event("launch", MARQUEE));
    let _app = card(client(&mut rig, true), MARQUEE, true, true);
    assert_eq!(front(&rig).as_deref(), Some(MARQUEE));
}

#[test]
fn home_is_never_held() {
    let mut rig = rig();
    let _app = card(client(&mut rig, true), HOME, true, true);
    assert_eq!(front(&rig).as_deref(), Some(HOME));
}
