//! LG's guide while LumaTV's UI answers for it (`lg_guide.rs`): its window is in no frame from its first commit, never has the keyboard focus, and SAM
//! is asked to close the app; the UI is asked for its guide when SAM reports the launch, or when the window maps if the launch was not heard. With
//! the UI not there nothing is held and nothing is asked: LG's guide is drawn as it is on a stock set.

mod harness;

use std::sync::atomic::Ordering;
use std::time::Duration;

use harness::{Client, Server, Window, drawn};
use lumatv_compositor::State;
use lumatv_compositor::luna::{Event, ToBridge};
use lumatv_compositor::shell::Msg;
use serde_json::json;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_keyboard::WlKeyboard;

const GUIDE: &str = "com.webos.app.livemenu";
const LIVE_TV: &str = "com.webos.app.livetv";
/// What each draws, premultiplied ARGB, and the pixels it makes (RGBA).
const LIVE: u32 = 0xff20_a040;
const LG: u32 = 0xff30_30c0;
const OTHER: u32 = 0xffc0_6020;
const LIVE_PX: [u8; 4] = [0x20, 0xa0, 0x40, 0xff];
const LG_PX: [u8; 4] = [0x30, 0x30, 0xc0, 0xff];
const OTHER_PX: [u8; 4] = [0xc0, 0x60, 0x20, 0xff];

/// Live TV's card with the keyboard focus, and what LG launches over it.
struct Rig {
    client: Client,
    _kb: WlKeyboard,
    _live: Window,
    _bufs: Vec<WlBuffer>,
    windows: Vec<Window>,
}

fn rig(server: &mut Server) -> Rig {
    server.ticking.store(false, Ordering::SeqCst);
    let mut client = Client::connect(server);
    let kb = client.seat.get_keyboard(&client.qh, ());
    let live = client.card_as("_WEBOS_WINDOW_TYPE_CARD", "Live TV", LIVE_TV);
    let picture = client.shm_buffer(1920, 1080, LIVE);
    client.present(&live.surface, &picture);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.keyboard_enters >= 1), "Live TV has the keyboard focus");
    Rig { client, _kb: kb, _live: live, _bufs: vec![picture], windows: Vec::new() }
}

impl Rig {
    /// An overlay window of `app` with a picture of `argb`, as big as `w` x `h`, committed.
    fn overlay(&mut self, app: &str, w: i32, h: i32, argb: u32) {
        let window = self.client.card_as("_WEBOS_WINDOW_TYPE_OVERLAY", app, app);
        let picture = self.client.shm_buffer(w, h, argb);
        self.client.present(&window.surface, &picture);
        self.client.roundtrip();
        self.windows.push(window);
        self._bufs.push(picture);
    }

    /// LG's guide launches: its window maps with a full-screen picture.
    fn lg_guide(&mut self) {
        self.overlay(GUIDE, 1920, 1080, LG);
    }
}

/// The UI on the engine, answering for `kinds` (which it can show too).
fn ui(server: &Server, kinds: &[&str]) {
    let kinds: Vec<String> = kinds.iter().map(|k| k.to_string()).collect();
    server.eval(move |s| {
        if !s.shell_rust.enabled() {
            s.shell_rust_open(0, "test".into()).unwrap();
        }
        s.shell_rust.kinds.clone_from(&kinds);
        s.shell_rust_handle(Msg::Connected);
        // (the engine's handshake needs the GL engine, which the host harness does not run)
        s.shell_rust.engine_connected = true;
        s.shell_rust_handle(Msg::Ui { can: Some(kinds), wants: None, live: None, state: None });
    });
}

/// The next frame: whether a window of LG's guide is in its plan, and the pixel in the middle of the screen.
fn frame(server: &Server) -> (bool, [u8; 4]) {
    server.eval(|s| {
        let image = drawn(s);
        let guide = s.scene.last_plan().items.iter().any(|i| s.scene.window(i.window).is_some_and(|w| w.app_id() == GUIDE));
        (guide, image.pixel(960, 540))
    })
}

/// How many times the UI was asked for its guide, and SAM to close LG's.
fn asked(server: &Server) -> (usize, usize) {
    server.eval(|s| (shows(s), closes(s)))
}

fn shows(s: &State) -> usize {
    s.shell_rust.events().iter().filter(|e| e["topic"] == "show" && e["data"]["kind"] == "guide").count()
}

fn closes(s: &State) -> usize {
    s.luna.history().iter().filter(|m| matches!(m, ToBridge::Outbound(o) if o.uri.ends_with("/closeByAppId") && o.payload["id"] == GUIDE)).count()
}

/// The token of the compositor's subscription to SAM's life events.
fn life_token(s: &State) -> u64 {
    s.luna.history().iter().find_map(|m| match m {
        ToBridge::Outbound(o) if o.uri.ends_with("/getAppLifeEvents") && o.subscribe => Some(o.token),
        _ => None,
    }).expect("the compositor subscribed to SAM's life events")
}

/// An event of SAM's life subscription.
fn life(server: &Server, token: u64, event: &str, app: &str) {
    let (event, app) = (event.to_string(), app.to_string());
    server.eval(move |s| s.luna_event(Event { token, payload: json!({"event": event, "appId": app, "returnValue": true, "reason": "undefined"}) }));
}

#[test]
fn lg_guide_is_in_no_frame_while_the_ui_answers_for_it_and_sam_is_asked_to_close_it() {
    let mut server = Server::start();
    let mut rig = rig(&mut server);
    ui(&server, &["guide"]);
    assert_eq!(frame(&server), (false, LIVE_PX), "Live TV's picture is on the screen");
    rig.client.roundtrip();
    let before = rig.client.state.frames_done;
    rig.overlay("com.webos.app.scheduler", 400, 200, OTHER);
    rig.lg_guide();
    // the first frame it could be in
    assert_eq!(frame(&server), (false, LIVE_PX), "LG's guide is not drawn, from its first commit");
    rig.client.roundtrip();
    assert_eq!(rig.client.state.frames_done - before, 2, "the frame callbacks of both windows are paid, LG's too: its app never waits for a frame that is not coming");
    server.eval(|s| {
        assert_eq!(s.scene.windows().filter(|w| w.app_id() == GUIDE && w.map_serial > 0).count(), 1, "it is mapped: SAM and the apps see it");
        let fg = s.luna_method("com.webos.surfacemanager", "getForegroundAppInfo", &json!({}), "").unwrap();
        let apps: Vec<&str> = fg["foregroundAppInfo"].as_array().unwrap().iter().filter_map(|a| a["appId"].as_str()).collect();
        assert_eq!(apps, [LIVE_TV, "com.webos.app.scheduler", GUIDE], "as any overlay, so that SAM runs the flow it knows");
        let scheduler = s.scene.windows().find(|w| w.app_id() == "com.webos.app.scheduler").unwrap().surface;
        assert_eq!(s.seat.focus, Some(scheduler), "the keys are the top window's that is drawn: LG's guide, which is over it, never has the focus");
        assert_eq!(s.shell_report()["lgGuideHeld"], true);
    });
    assert_eq!(asked(&server), (1, 1), "the UI is asked for its guide and SAM to close LG's, once");
    let picture = server.eval(|s| {
        let image = drawn(s);
        (image.pixel(100, 100), s.scene.windows().filter(|w| w.app_id() == "com.webos.app.scheduler").count())
    });
    assert_eq!(picture, (OTHER_PX, 1), "another app's overlay is drawn as ever");
    assert!(server.eval(|s| s.journal.iter().any(|l| l.contains("lg-guide: LG's guide launched (its window mapped)"))));
}

#[test]
fn a_launch_sam_reports_asks_the_ui_and_closes_the_app_at_once_and_the_window_that_follows_asks_for_the_close_only() {
    let mut server = Server::start();
    let mut rig = rig(&mut server);
    ui(&server, &["guide"]);
    let token = server.eval(|s| {
        s.luna_register();
        life_token(s)
    });
    server.eval(move |s| s.luna_event(Event { token, payload: json!({"subscribed": true, "returnValue": true}) }));
    for (event, app) in [("launch", LIVE_TV), ("splash", GUIDE), ("foreground", GUIDE), ("pause", GUIDE)] {
        life(&server, token, event, app);
    }
    assert_eq!(asked(&server), (0, 0), "the subscription's own reply, another app's launch and the guide's other events are no launch");

    life(&server, token, "launch", GUIDE);
    assert_eq!(asked(&server), (1, 1), "the UI and SAM are asked at once, with no window yet (a cold start takes 2.6 s to map one)");
    rig.lg_guide();
    assert_eq!(frame(&server), (false, LIVE_PX));
    assert_eq!(asked(&server), (1, 2), "the window of that launch asks the UI for nothing more, and the app is closed again");

    // SAM's answer is in the journal
    let answered = server.eval(move |s| {
        let close = s.luna.history().iter().find_map(|m| match m {
            ToBridge::Outbound(o) if o.uri.ends_with("/closeByAppId") => Some(o.token),
            _ => None,
        });
        s.luna_event(Event { token: close.unwrap(), payload: json!({"returnValue": true}) });
        s.journal.iter().any(|l| l.contains("lg-guide: SAM answered closeByAppId"))
    });
    assert!(answered);
}

#[test]
fn with_no_ui_to_answer_for_it_lg_guide_is_drawn_as_stock_and_a_ui_that_goes_gives_it_back() {
    let mut server = Server::start();
    let mut rig = rig(&mut server);
    let token = server.eval(|s| {
        s.luna_register();
        life_token(s)
    });
    // never started: Live TV launches it, SAM says so, the window maps
    life(&server, token, "launch", GUIDE);
    rig.lg_guide();
    assert_eq!(frame(&server), (true, LG_PX), "LG's guide is on the screen");
    assert_eq!(asked(&server), (0, 0), "nothing is asked of the UI or of SAM");
    server.eval(|s| {
        let live = s.scene.windows().find(|w| w.app_id() == GUIDE).unwrap().surface;
        assert_eq!(s.seat.focus, Some(live), "and it has the keys");
        assert_eq!(s.shell_report()["lgGuideHeld"], false);
    });

    // a UI that has no guide to show does not hold it
    ui(&server, &["osd"]);
    assert_eq!(frame(&server), (true, LG_PX));

    // the UI that has: LG's guide that is up goes, and its app is closed
    ui(&server, &["guide"]);
    assert_eq!(frame(&server), (false, LIVE_PX), "LG's guide is not drawn once the UI answers for it");
    assert_eq!(asked(&server), (1, 1), "the guide that was up is answered like one that mapped");

    // the UI is gone (a crash, a restart): the next frame is LG's, and a launch is not answered
    server.eval(|s| s.shell_rust_restart());
    assert_eq!(frame(&server), (true, LG_PX), "LG's guide is back as stock");
    life(&server, token, "launch", GUIDE);
    assert_eq!(asked(&server), (1, 1), "nothing is asked with the UI gone");
    server.eval(|s| {
        let live = s.scene.windows().find(|w| w.app_id() == GUIDE).unwrap().surface;
        assert_eq!(s.seat.focus, Some(live), "it has the keys again");
        assert_eq!(s.shell_report()["lgGuideHeld"], false);
    });
}
