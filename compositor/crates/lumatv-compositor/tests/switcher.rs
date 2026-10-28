//! The app switcher (`switcher.rs`) under the real `State`, with the shell client played by messages and the apps by windows:
//! the second press of Home (and that a single press is Home at once, with nothing held back to see whether another comes), the
//! Home hold's escape untouched, the list of running apps, the pictures drawn while it is open and never otherwise, a card's
//! motion, Quit through the app manager, and the shell going away.
//!
//! Time is an argument everywhere it matters: the server is not left to tick on its own, and each step names the instant it is at.

mod harness;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use harness::{Client, Server, Window, drawn};
use lumatv_compositor::State;
use lumatv_compositor::input::code::*;
use lumatv_compositor::luna::ToBridge;
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl};
use lumatv_compositor::scene::Hole;
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::shell::Msg;
use lumatv_compositor::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect};
use serde_json::{Value, json};
use wayland_client::protocol::wl_buffer::WlBuffer;

const WAIT: Duration = Duration::from_secs(3);
const HOME_STUB: &str = "com.webos.app.home";
const LAUNCH_HOME: &str = "luna://com.webos.applicationManager/launchDefaultApp";
const CLOSE_APP: &str = "luna://com.webos.applicationManager/closeByAppId";
const LAUNCH: &str = "luna://com.webos.applicationManager/launch";
const MS: fn(u64) -> Duration = Duration::from_millis;

const HOME_COLOUR: u32 = 0xffff_00ff;
const PLEX: u32 = 0xff00_ff00;
const YOUTUBE: u32 = 0xffff_0000;
const NETFLIX: u32 = 0xff00_00ff;

struct Held {
    _client: Client,
    win: Window,
    _buf: WlBuffer,
}

/// A card of `app_id` mapped with a buffer of the pattern, brought to the front by its mapping.
fn app_with(server: &mut Server, title: &str, app_id: &str, w: i32, h: i32, pixel: impl Fn(i32, i32) -> u32) -> Held {
    let mut client = Client::connect_extra(server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", title);
    win.webos.set_property("appId".into(), app_id.into());
    let buf = client.shm_pattern(w, h, pixel);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1), "{title} presented");
    Held { _client: client, win, _buf: buf }
}

fn app(server: &mut Server, title: &str, app_id: &str, argb: u32) -> Held {
    app_with(server, title, app_id, 400, 225, move |_, _| argb)
}

/// LumaTV's Home window (64x64) and what the shell says it can show.
fn shell_up(server: &mut Server, can: &[&str]) -> Held {
    server.eval(|s| s.shell_handle(Msg::Connected));
    let can: Vec<String> = can.iter().map(|c| c.to_string()).collect();
    server.eval(move |s| s.shell_handle(Msg::Ui { can: Some(can), wants: None, live: None, state: None }));
    let mut client = Client::connect_extra(server);
    let win = client.card("_WEBOS_WINDOW_TYPE_LUMATV_HOME", "LumaHome");
    win.webos.set_property("appId".into(), "org.nphil.lumatv.shell".into());
    let buf = client.shm_buffer(64, 64, HOME_COLOUR);
    client.present(&win.surface, &buf);
    assert!(client.wait_until(WAIT, |s| s.frames_done >= 1));
    Held { _client: client, win, _buf: buf }
}

struct Rig {
    server: Server,
    apps: Vec<Held>,
}

/// Netflix, YouTube and Plex (the newest of them) mapped, then Home's stub: Home is in front, with the three behind it.
fn rig_home(can: &[&str]) -> Rig {
    let mut server = Server::start();
    let mut apps = vec![shell_up(&mut server, can)];
    apps.push(app(&mut server, "Netflix", "netflix", NETFLIX));
    apps.push(app(&mut server, "YouTube", "youtube", YOUTUBE));
    apps.push(app(&mut server, "Plex", "plex", PLEX));
    apps.push(app(&mut server, "Stub", HOME_STUB, 0xff10_1010));
    server.eval(|s| s.luna_notify());
    // from here on time is the test's: the server does not tick on its own
    server.ticking.store(false, Ordering::SeqCst);
    Rig { server, apps }
}

/// The same three after Home's stub: Plex is in front, Home is behind everything.
fn rig_app(can: &[&str]) -> Rig {
    let mut server = Server::start();
    let mut apps = vec![shell_up(&mut server, can)];
    apps.push(app(&mut server, "Stub", HOME_STUB, 0xff10_1010));
    apps.push(app(&mut server, "Netflix", "netflix", NETFLIX));
    apps.push(app(&mut server, "YouTube", "youtube", YOUTUBE));
    apps.push(app(&mut server, "Plex", "plex", PLEX));
    server.eval(|s| s.luna_notify());
    // from here on time is the test's: the server does not tick on its own
    server.ticking.store(false, Ordering::SeqCst);
    Rig { server, apps }
}

/// A Home tap: the key goes down at `at` and comes up `held` later.
fn tap(s: &mut State, at: Instant, held: Duration) {
    s.key_event_at(SeatId(0), KEY_HOMEPAGE, true, false, at);
    s.key_event_at(SeatId(0), KEY_HOMEPAGE, false, false, at + held);
}

fn launches_home(s: &State) -> usize {
    s.luna_sent().into_iter().filter(|(u, _)| u == LAUNCH_HOME).count()
}

fn calls(s: &State, uri: &str) -> Vec<Value> {
    s.luna_sent().into_iter().filter(|(u, _)| u == uri).map(|(_, p)| p).collect()
}

fn topic(s: &State, name: &str) -> Value {
    s.shell.topic(name).cloned().unwrap_or(Value::Null)
}

fn recorded(s: &State, name: &str) -> Vec<Value> {
    s.shell.recorded.iter().filter(|e| e["topic"] == name).map(|e| e["data"].clone()).collect()
}

fn layout(cards: &[(&str, i32, i32, i32, i32, i32)], ms: u64) -> Value {
    json!({"op": "layout", "ms": ms, "scrim": {"r": 0.2, "g": 0.0, "b": 0.4, "a": 1.0},
           "cards": cards.iter().map(|(id, x, y, w, h, r)| json!({"id": id, "x": x, "y": y, "w": w, "h": h, "r": r})).collect::<Vec<_>>()})
}

fn op(s: &mut State, body: Value) -> bool {
    s.shell_handle(Msg::Switcher(body))
}

/// The pictures the switcher has in the frame at `t`: (rectangle, opacity, corner radius).
fn pictures_at(s: &mut State, t: Instant) -> Vec<(Rect, f32, f32)> {
    s.switcher_frame(t);
    let plan = s.scene.plan(&s.surfaces);
    plan.backdrop.as_ref().map(|b| b.previews.iter().map(|p| (p.rect, p.alpha, p.radius)).collect()).unwrap_or_default()
}

fn backdrop_in_plan(s: &mut State) -> bool {
    s.scene.plan(&s.surfaces).backdrop.is_some()
}

fn full() -> Rect {
    Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32)
}

// ---- the trigger ----

#[test]
fn a_single_press_of_home_goes_home_when_it_is_let_go_and_nothing_waits_for_a_second() {
    let rig = rig_app(&["switcher"]);
    let seen = rig.server.eval(|s| {
        let t0 = Instant::now();
        s.key_event_at(SeatId(0), KEY_HOMEPAGE, true, false, t0);
        let at_press = launches_home(s);
        s.key_event_at(SeatId(0), KEY_HOMEPAGE, false, false, t0 + MS(80));
        // no tick, no timer: the answer is in the call that saw the key let go
        (at_press, launches_home(s), s.switcher_active())
    });
    assert_eq!(seen, (0, 1, false), "Home is asked of the app manager as the key comes up, as it was before the switcher; the switcher is not opened");
}

#[test]
fn a_second_press_from_an_app_opens_the_switcher_and_does_not_ask_for_home_again() {
    let rig = rig_app(&["switcher"]);
    let (homes, open, apps) = rig.server.eval(|s| {
        let t0 = Instant::now();
        tap(s, t0, MS(80));
        assert_eq!(launches_home(s), 1, "the first press went Home at once");
        assert!(!s.switcher_active(), "and opened nothing");
        tap(s, t0 + MS(300), MS(80));
        (launches_home(s), topic(s, "switcher"), topic(s, "apps"))
    });
    assert_eq!(homes, 1, "the second press is the switcher's: no second Home");
    assert_eq!(open["open"], true);
    assert_eq!(open["from"], "plex", "Plex was in front at the first press");
    let ids: Vec<&str> = apps.as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["plex", "youtube", "netflix"], "newest first, Home and LumaTV's own windows left out");
    assert!(apps.as_array().unwrap().iter().all(|a| a["picture"] == "live"));
}

#[test]
fn the_window_is_four_hundred_milliseconds_and_a_press_after_it_is_a_first_one() {
    let rig = rig_app(&["switcher"]);
    let (homes, active) = rig.server.eval(|s| {
        let t0 = Instant::now();
        tap(s, t0, MS(80));
        // 401 ms after the first was let go
        tap(s, t0 + MS(80 + 401 - 80), MS(80));
        (launches_home(s), s.switcher_active())
    });
    assert_eq!((homes, active), (2, false), "two Home presses, no switcher");
    let rig = rig_app(&["switcher"]);
    let active = rig.server.eval(|s| {
        let t0 = Instant::now();
        tap(s, t0, MS(80));
        // 400 ms after the first was let go: this one is let go at t0 + 80 + 400
        tap(s, t0 + MS(400), MS(80));
        s.switcher_active()
    });
    assert!(active, "400 ms exactly still counts");
}

#[test]
fn with_the_switch_off_every_press_is_home_alone() {
    // the shell offers the switcher only while the owner's switch is on
    let rig = rig_app(&[]);
    let (homes, active) = rig.server.eval(|s| {
        let t0 = Instant::now();
        tap(s, t0, MS(80));
        tap(s, t0 + MS(200), MS(80));
        (launches_home(s), s.switcher_active())
    });
    assert_eq!((homes, active), (2, false));
    // or while the shell is not there to draw it
    let mut server = Server::start();
    let _home = shell_up(&mut server, &["switcher"]);
    let _stub = app(&mut server, "Stub", HOME_STUB, 0xff10_1010);
    let _plex = app(&mut server, "Plex", "plex", PLEX);
    server.ticking.store(false, Ordering::SeqCst);
    let (homes, active) = server.eval(|s| {
        // the shell client is gone
        s.shell_restart();
        let t0 = Instant::now();
        tap(s, t0, MS(80));
        tap(s, t0 + MS(200), MS(80));
        (launches_home(s), s.switcher_active())
    });
    assert_eq!((homes, active), (2, false));
}

#[test]
fn the_hold_of_home_is_the_same_escape_as_ever_also_right_after_a_tap_and_with_the_switcher_open() {
    let rig = rig_app(&["switcher"]);
    let (falls, homes, active) = rig.server.eval(|s| {
        let t0 = Instant::now();
        tap(s, t0, MS(80));
        // a second press, held: within the double-press window, but past 650 ms it is the escape and nothing else
        s.key_event_at(SeatId(0), KEY_HOMEPAGE, true, false, t0 + MS(200));
        s.poll_holds_at(t0 + MS(200 + 649));
        let before = s.luna.history().iter().filter(|m| matches!(m, ToBridge::Fallback { .. })).count();
        s.poll_holds_at(t0 + MS(200 + 650));
        let after = s.luna.history().iter().filter(|m| matches!(m, ToBridge::Fallback { .. })).count();
        assert_eq!((before, after), (0, 1), "650 ms, to the millisecond");
        s.key_event_at(SeatId(0), KEY_HOMEPAGE, false, false, t0 + MS(200 + 900));
        (after, launches_home(s), s.switcher_active())
    });
    assert_eq!((falls, homes, active), (1, 1, false), "the release after the escape launches nothing and opens nothing");
    // with the switcher open the hold is still the escape
    let rig = rig_app(&["switcher"]);
    let falls = rig.server.eval(|s| {
        let t0 = Instant::now();
        tap(s, t0, MS(80));
        tap(s, t0 + MS(250), MS(80));
        assert!(s.switcher_active());
        s.key_event_at(SeatId(0), KEY_HOMEPAGE, true, false, t0 + MS(1000));
        s.poll_holds_at(t0 + MS(1000 + 650));
        s.luna.history().iter().filter(|m| matches!(m, ToBridge::Fallback { .. })).count()
    });
    assert_eq!(falls, 1);
}

#[test]
fn a_press_on_home_with_apps_open_is_offered_to_the_shell_which_opens_it_and_a_third_press_closes_it() {
    let rig = rig_home(&["switcher"]);
    let (homes, offered, open, apps) = rig.server.eval(|s| {
        let t0 = Instant::now();
        tap(s, t0, MS(80));
        let offered = recorded(s, "homekey").last().cloned();
        // the shell, with nothing open on Home, asks for it
        op(s, json!({"op": "open"}));
        (launches_home(s), offered, topic(s, "switcher"), topic(s, "apps"))
    });
    assert_eq!(homes, 1, "Home as ever");
    assert_eq!(offered.unwrap()["switcher"], true, "the shell is told Home was pressed at rest with apps open");
    assert_eq!((open["open"].clone(), open["from"].clone()), (json!(true), json!("")), "opened from Home: no app shrinks into its card");
    assert_eq!(apps.as_array().unwrap().len(), 3);
    // Home again: it closes, and Home is not asked for again
    let (homes, open) = rig.server.eval(|s| {
        let t0 = Instant::now() + MS(1000);
        tap(s, t0, MS(80));
        (launches_home(s), topic(s, "switcher"))
    });
    assert_eq!((homes, open["open"].clone()), (1, json!(false)));
}

#[test]
fn with_no_app_open_home_offers_nothing() {
    let mut server = Server::start();
    let _home = shell_up(&mut server, &["switcher"]);
    let _stub = app(&mut server, "Stub", HOME_STUB, 0xff10_1010);
    server.eval(|s| s.luna_notify());
    server.ticking.store(false, Ordering::SeqCst);
    let last = server.eval(|s| {
        tap(s, Instant::now(), MS(80));
        (recorded(s, "homekey").last().cloned(), s.switcher_apps().len())
    });
    assert_eq!(last.1, 0);
    assert_eq!(last.0.unwrap()["switcher"], false);
}

// ---- the apps ----

#[test]
fn an_app_with_its_picture_on_the_video_plane_is_listed_as_video_and_never_given_a_picture() {
    let rig = rig_home(&["switcher"]);
    rig.server.eval(|s| {
        let surface = s.switcher_apps().into_iter().find(|a| a.id == "youtube").unwrap().surface;
        s.scene.holes.push(Hole { surface, rect: Rect::new(0, 0, 400, 225) });
        s.luna_notify();
        let t0 = Instant::now();
        let apps = topic(s, "apps");
        let youtube = apps.as_array().unwrap().iter().find(|a| a["id"] == "youtube").unwrap().clone();
        assert_eq!(youtube["picture"], "video");
        assert!(s.switcher_open(None, t0));
        op(s, layout(&[("plex", 100, 300, 400, 225, 22), ("youtube", 560, 300, 400, 225, 22), ("netflix", 1020, 300, 400, 225, 22)], 0));
        let pics = pictures_at(s, t0);
        assert_eq!(pics.len(), 2, "Plex and Netflix have a picture, YouTube has its icon");
    });
}

#[test]
fn inputs_are_video_by_their_ids() {
    let mut server = Server::start();
    let _home = shell_up(&mut server, &["switcher"]);
    let _hdmi = app(&mut server, "HDMI 2", "com.webos.app.hdmi2", 0xff00_0000);
    let _stub = app(&mut server, "Stub", HOME_STUB, 0xff10_1010);
    server.ticking.store(false, Ordering::SeqCst);
    let apps = server.eval(|s| s.switcher_apps());
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].picture, lumatv_compositor::switcher::Picture::Video);
}

// ---- the pictures ----

#[test]
fn the_pictures_and_the_dim_are_in_the_frame_only_while_it_is_open() {
    let rig = rig_home(&["switcher"]);
    rig.server.eval(|s| {
        let t0 = Instant::now();
        assert!(!backdrop_in_plan(s), "closed: the plan carries nothing of it");
        assert_eq!(drawn(s).pixel(1000, 900), [0, 0, 0, 255]);
        assert!(s.switcher_open(None, t0));
        // no cards placed yet: the dim comes with the layout, the pictures with their places
        op(s, layout(&[("plex", 100, 300, 400, 225, 0), ("youtube", 560, 300, 400, 225, 0), ("netflix", 1020, 300, 400, 225, 0)], 0));
        s.switcher_frame(t0 + MS(1));
        let img = s.output.capture().map(|_| ()).ok();
        let _ = img;
        let img = drawn(s);
        assert_eq!(img.pixel(300, 400), [0, 255, 0, 255], "Plex's own picture in its card");
        assert_eq!(img.pixel(700, 400), [255, 0, 0, 255], "YouTube's");
        assert_eq!(img.pixel(1200, 400), [0, 0, 255, 255], "Netflix's");
        assert_eq!(img.pixel(1000, 900), [51, 0, 102, 255], "the dim over everything else");
        assert_eq!(img.pixel(10, 10), [51, 0, 102, 255], "also over Home");
        op(s, json!({"op": "close"}));
        // the exit lasts 160 ms; after it nothing is left
        let t1 = Instant::now() + MS(400);
        s.switcher_frame(t1);
        assert!(!s.switcher_active());
    });
    rig.server.eval(|s| {
        let img = drawn(s);
        assert_eq!(img.pixel(300, 400), [0, 0, 0, 255], "closed: the pictures are gone");
        assert_eq!(img.pixel(10, 10), [255, 0, 255, 255], "and Home is itself again");
        assert!(s.scene.last_plan().backdrop.is_none(), "the plan carries nothing of it");
    });
}

#[test]
fn a_picture_is_the_whole_window_scaled_into_the_card_with_its_corners_rounded() {
    let mut server = Server::start();
    let _home = shell_up(&mut server, &["switcher"]);
    // left half red, right half blue, 400 by 225
    let _split = app_with(&mut server, "Split", "split", 400, 225, |x, _| if x < 200 { 0xffff_0000 } else { 0xff00_00ff });
    let _stub = app(&mut server, "Stub", HOME_STUB, 0xff10_1010);
    server.ticking.store(false, Ordering::SeqCst);
    server.eval(|s| {
        s.luna_notify();
        let t0 = Instant::now();
        assert!(s.switcher_open(None, t0));
        // 200 by 112: half the size
        op(s, layout(&[("split", 600, 400, 200, 112, 22)], 0));
        s.switcher_frame(t0);
        let img = drawn(s);
        assert_eq!(img.pixel(650, 456), [255, 0, 0, 255], "the left half is red");
        assert_eq!(img.pixel(750, 456), [0, 0, 255, 255], "the right half is blue");
        assert_eq!(img.pixel(699, 456), [255, 0, 0, 255], "and the seam is at the middle of the card");
        assert_eq!(img.pixel(700, 456), [0, 0, 255, 255]);
        assert_eq!(img.pixel(600, 400), [51, 0, 102, 255], "the corner pixel is not the picture's: the corners are rounded");
        assert_eq!(img.pixel(630, 430), [255, 0, 0, 255], "while a pixel well inside the corner is");
    });
}

#[test]
fn a_second_press_from_an_app_shrinks_its_picture_from_the_whole_screen_into_its_card_in_200_ms() {
    let rig = rig_app(&["switcher"]);
    rig.server.eval(|s| {
        let t0 = Instant::now();
        tap(s, t0, MS(80));
        tap(s, t0 + MS(300), MS(80));
        assert!(s.switcher_active());
        // the shell places the cards; Plex's card is the first
        let at = t0 + MS(400);
        op(s, layout(&[("plex", 112, 340, 400, 225, 22), ("youtube", 544, 340, 400, 225, 22), ("netflix", 976, 340, 400, 225, 22)], 200));
        // the shell's message arrived "now" on the compositor's clock: the tracks started then; read them against that
        let started = Instant::now();
        let _ = at;
        let first = pictures_at(s, started);
        assert_eq!(first.len(), 1, "at the start only Plex's picture shows: the whole screen, and the others have not arrived");
        assert!(first[0].0.w > 1700 && first[0].0.h > 950, "{:?}", first[0]);
        assert_eq!(first[0].1, 1.0);
        let mid = pictures_at(s, started + MS(60));
        let plex = mid.iter().max_by_key(|p| p.0.w).unwrap();
        assert!(plex.0.w < 1900 && plex.0.w > 400, "on its way: {:?}", plex);
        assert!(mid.len() >= 2, "the others arrive meanwhile");
        let end = pictures_at(s, started + MS(205));
        let rects: Vec<Rect> = end.iter().map(|p| p.0).collect();
        assert!(rects.contains(&Rect::new(112, 340, 400, 225)), "Plex rests in its card: {rects:?}");
        assert!(rects.contains(&Rect::new(544, 340, 400, 225)) && rects.contains(&Rect::new(976, 340, 400, 225)));
        assert!(end.iter().all(|p| p.1 == 1.0 && p.2 == 22.0));
    });
}

#[test]
fn from_home_the_pictures_arrive_a_little_smaller_and_fade_in() {
    let rig = rig_home(&["switcher"]);
    rig.server.eval(|s| {
        let t0 = Instant::now();
        assert!(s.switcher_open(None, t0));
        op(s, layout(&[("plex", 112, 340, 400, 225, 22)], 200));
        let started = Instant::now();
        let mid = pictures_at(s, started + MS(20));
        assert_eq!(mid.len(), 1);
        let (r, alpha, _) = mid[0];
        assert!(r.w < 400 && r.w > 370 && alpha > 0.0 && alpha < 1.0, "{mid:?}");
        let end = pictures_at(s, started + MS(210));
        assert_eq!(end[0].0, Rect::new(112, 340, 400, 225));
        assert_eq!(end[0].1, 1.0);
    });
}

#[test]
fn reduced_motion_fades_and_does_not_travel() {
    let rig = rig_app(&["switcher"]);
    rig.server.eval(|s| {
        let t0 = Instant::now();
        tap(s, t0, MS(80));
        tap(s, t0 + MS(300), MS(80));
        let mut l = layout(&[("plex", 112, 340, 400, 225, 22)], 120);
        l["reduced"] = json!(true);
        op(s, l);
        let started = Instant::now();
        let mid = pictures_at(s, started + MS(30));
        assert_eq!(mid[0].0, Rect::new(112, 340, 400, 225), "its card is there from the first frame: nothing shrinks, nothing travels");
        assert!(mid[0].1 > 0.0 && mid[0].1 < 1.0, "it fades in: {mid:?}");
        let end = pictures_at(s, started + MS(130));
        assert_eq!((end[0].0, end[0].1), (Rect::new(112, 340, 400, 225), 1.0));
    });
}

#[test]
fn the_shell_that_does_not_place_the_cards_in_time_is_given_up() {
    let rig = rig_home(&["switcher"]);
    rig.server.eval(|s| {
        let t0 = Instant::now();
        assert!(s.switcher_open(None, t0));
        s.switcher_frame(t0 + MS(700));
        assert_eq!(topic(s, "switcher")["open"], false);
        s.switcher_frame(t0 + MS(1000));
        assert!(!s.switcher_active());
    });
}

// ---- leaving ----

#[test]
fn ok_on_a_card_launches_the_app_and_grows_its_picture_to_the_whole_screen() {
    let rig = rig_home(&["switcher"]);
    rig.server.eval(|s| {
        let t0 = Instant::now();
        assert!(s.switcher_open(None, t0));
        op(s, layout(&[("plex", 112, 340, 400, 225, 22), ("youtube", 544, 340, 400, 225, 22)], 0));
        let t1 = Instant::now();
        op(s, json!({"op": "activate", "id": "youtube"}));
        assert_eq!(calls(s, LAUNCH).last().unwrap()["id"], "youtube", "the app manager is asked for the app");
        assert_eq!(topic(s, "switcher")["open"], false, "the shell is told, and fades its own");
        let start = pictures_at(s, t1);
        assert!(start.iter().any(|p| p.0 == Rect::new(544, 340, 400, 225)));
        let end = pictures_at(s, t1 + MS(210));
        assert_eq!(end.len(), 1, "the other pictures are gone");
        assert_eq!(end[0], (full(), 1.0, 0.0), "YouTube's picture fills the screen, square-cornered");
        // until it is the front app the picture stays; when it is, or after a second, the switcher is over
        assert!(s.switcher_active());
        assert!(!pictures_at(s, t1 + MS(1100)).is_empty() || !s.switcher_active());
        assert!(!s.switcher_active());
    });
}

#[test]
fn quit_closes_the_app_through_the_app_manager_and_the_card_goes_when_the_window_does() {
    let mut rig = rig_home(&["switcher"]);
    rig.server.eval(|s| {
        let t0 = Instant::now();
        assert!(s.switcher_open(None, t0));
        op(s, layout(&[("plex", 112, 340, 400, 225, 22), ("youtube", 544, 340, 400, 225, 22), ("netflix", 976, 340, 400, 225, 22)], 0));
        op(s, json!({"op": "quit", "id": "youtube"}));
        let sam = calls(s, CLOSE_APP);
        assert_eq!(sam, vec![json!({"id": "youtube", "reason": "windowPolicy"})], "closeByAppId, the reason LG's own recents gives");
        let apps = topic(s, "apps");
        let yt = apps.as_array().unwrap().iter().find(|a| a["id"] == "youtube").unwrap().clone();
        assert_eq!(yt["state"], "quitting", "the card says it is closing until the window is gone");
        // asked twice: asked once
        op(s, json!({"op": "quit", "id": "youtube"}));
        assert_eq!(calls(s, CLOSE_APP).len(), 1);
        // the picture dims while it closes
        let dim = pictures_at(s, Instant::now() + MS(200));
        let yt_pic = dim.iter().find(|p| p.0.x == 544).unwrap();
        assert!((yt_pic.1 - 0.35).abs() < 0.01, "{yt_pic:?}");
    });
    // the app's window closes: its card fades out and the list says so
    let youtube = rig.apps.remove(2);
    youtube.win.surface.destroy();
    drop(youtube);
    rig.apps[0]._client.roundtrip();
    rig.server.eval(|s| {
        s.luna_notify();
        let apps = topic(s, "apps");
        let ids: Vec<&str> = apps.as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["plex", "netflix"]);
        let later = Instant::now() + MS(500);
        let pics = pictures_at(s, later);
        assert_eq!(pics.len(), 2, "YouTube's picture has faded away and its card is dropped");
        assert!(s.switcher_report()["quitting"].as_array().unwrap().is_empty());
    });
}

#[test]
fn an_app_that_does_not_quit_in_four_seconds_says_so() {
    let rig = rig_home(&["switcher"]);
    rig.server.eval(|s| {
        let t0 = Instant::now();
        assert!(s.switcher_open(None, t0));
        op(s, layout(&[("plex", 112, 340, 400, 225, 22), ("youtube", 544, 340, 400, 225, 22)], 0));
        op(s, json!({"op": "quit", "id": "youtube"}));
        s.switcher_frame(t0 + MS(3900));
        assert_eq!(s.switcher_report()["stuck"], json!([]));
        s.switcher_frame(Instant::now() + Duration::from_secs(5));
        let apps = topic(s, "apps");
        let yt = apps.as_array().unwrap().iter().find(|a| a["id"] == "youtube").unwrap().clone();
        assert_eq!(yt["state"], "stuck");
        // it can be asked again
        op(s, json!({"op": "quit", "id": "youtube"}));
        assert_eq!(calls(s, CLOSE_APP).len(), 2);
    });
}

#[test]
fn quit_all_asks_for_every_app_and_lumatvs_own_are_never_closed() {
    let rig = rig_home(&["switcher"]);
    rig.server.eval(|s| {
        let t0 = Instant::now();
        assert!(s.switcher_open(None, t0));
        op(s, layout(&[("plex", 112, 340, 400, 225, 22)], 0));
        op(s, json!({"op": "quitall"}));
        let mut ids: Vec<String> = calls(s, CLOSE_APP).iter().map(|p| p["id"].as_str().unwrap().to_string()).collect();
        ids.sort();
        assert_eq!(ids, ["netflix", "plex", "youtube"]);
        // Home and the shell are not apps to quit
        op(s, json!({"op": "quit", "id": HOME_STUB}));
        op(s, json!({"op": "quit", "id": "org.nphil.lumatv.shell"}));
        assert_eq!(calls(s, CLOSE_APP).len(), 3);
    });
}

#[test]
fn home_and_the_shell_going_away_close_it() {
    let rig = rig_home(&["switcher"]);
    rig.server.eval(|s| {
        let t0 = Instant::now();
        assert!(s.switcher_open(None, t0));
        op(s, layout(&[("plex", 112, 340, 400, 225, 22)], 0));
        assert!(s.switcher_active());
        s.shell_restart();
        assert!(!s.switcher_active(), "no shell, no switcher: nothing could place the cards or take a key");
        assert!(!backdrop_in_plan(s));
    });
}

#[test]
fn another_app_coming_to_the_front_ends_it() {
    let mut rig = rig_home(&["switcher"]);
    rig.server.eval(|s| {
        let t0 = Instant::now();
        assert!(s.switcher_open(None, t0));
        op(s, layout(&[("plex", 112, 340, 400, 225, 22)], 0));
    });
    // an app the owner did not choose here maps (a launch by voice, say)
    rig.server.ticking.store(true, Ordering::SeqCst);
    rig.apps.push(app(&mut rig.server, "Disney", "disney", 0xff12_3456));
    rig.server.ticking.store(false, Ordering::SeqCst);
    rig.server.eval(|s| {
        s.switcher_frame(Instant::now());
        assert_eq!(topic(s, "switcher")["open"], false);
        assert!(s.journal.iter().any(|l| l.contains("another app came to the front")));
    });
}

#[test]
fn the_switcher_asks_for_nothing_it_has_no_cards_for() {
    let rig = rig_home(&["switcher"]);
    rig.server.eval(|s| {
        let t0 = Instant::now();
        assert!(!s.switcher_open(Some("nothing".into()), t0) || s.switcher_active());
        s.switcher_close("test", t0);
        op(s, json!({"op": "activate", "id": "ghost"}));
        op(s, json!({"op": "quit", "id": "ghost"}));
        op(s, json!({"op": "bogus"}));
        assert!(calls(s, LAUNCH).is_empty() && calls(s, CLOSE_APP).is_empty());
    });
}

// ---- the GL renderer (Mesa's software GL on the host) ----

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";

/// What the GL renderer draws for a switcher with one card, through the two smaller copies it makes of the window's picture and the
/// program that rounds the card's corners: the same picture as the software renderer's, within what filtering allows.
#[test]
fn the_gl_renderer_draws_the_scrim_and_the_picture_scaled_with_rounded_corners_and_frees_its_copies_when_closed() {
    if !std::path::Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return;
    }
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let mut server = Server::start_with(Box::new(|| Box::new(Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap())));
    let _home = shell_up(&mut server, &["switcher"]);
    // 800 by 450, left half red and right half blue: a quarter-size card is a 2x shrink twice over
    let _split = app_with(&mut server, "Split", "split", 800, 450, |x, _| if x < 400 { 0xffff_0000 } else { 0xff00_00ff });
    let _stub = app(&mut server, "Stub", HOME_STUB, 0xff10_1010);
    server.ticking.store(false, Ordering::SeqCst);
    server.eval(|s| {
        s.luna_notify();
        let t0 = Instant::now();
        assert!(s.switcher_open(None, t0));
        op(s, layout(&[("split", 600, 400, 200, 112, 22)], 0));
        s.switcher_frame(t0);
        let img = drawn(s);
        let near = |p: [u8; 4], q: [u8; 4]| p.iter().zip(q).all(|(a, b)| (*a as i32 - b as i32).abs() <= 6);
        assert!(near(img.pixel(650, 456), [255, 0, 0, 255]), "left half: {:?}", img.pixel(650, 456));
        assert!(near(img.pixel(750, 456), [0, 0, 255, 255]), "right half: {:?}", img.pixel(750, 456));
        assert!(near(img.pixel(1000, 900), [51, 0, 102, 255]), "the dim: {:?}", img.pixel(1000, 900));
        assert!(near(img.pixel(600, 400), [51, 0, 102, 255]), "the card's corner is rounded away: {:?}", img.pixel(600, 400));
        assert!(near(img.pixel(630, 430), [255, 0, 0, 255]), "and well inside it is the picture: {:?}", img.pixel(630, 430));
        assert_eq!(s.stats.gl_errors, 0, "no GL error");
        // closed: nothing of it is drawn, and the renderer lets its copies go
        op(s, json!({"op": "close"}));
        s.switcher_frame(Instant::now() + MS(400));
        assert!(!s.switcher_active());
        let img = drawn(s);
        assert!(near(img.pixel(650, 456), [0, 0, 0, 255]), "gone: {:?}", img.pixel(650, 456));
        assert_eq!(s.stats.gl_errors, 0);
    });
}
