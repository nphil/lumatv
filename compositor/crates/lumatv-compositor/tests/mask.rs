//! The shell's claim on a region of Live TV's window (`mask.rs`): LG draws its channel banner into the card's one picture over the video, and while
//! LumaTV's own banner stands in for it the region is not drawn, so the video's hole shows there. A channel key to Live TV starts the blanking, an
//! engage does without a key, and a release (or the shell going away) gives the region back; nothing outside the region is touched.

mod harness;

use std::time::Duration;

use harness::{Client, Server, Window};
use lumatv_compositor::Rect;
use lumatv_compositor::mask::Op;
use lumatv_compositor::scene::Hole;
use lumatv_compositor::server::seat::SeatId;
use lumatv_compositor::shell::Msg;
use serde_json::json;
use wayland_client::protocol::wl_buffer::WlBuffer;

const MS: u64 = 1_000_000;
const FULL: Rect = Rect::new(0, 0, 1920, 1080);
/// Where LG's banner is (flush left, 156 px at 556) and what the claim names of it.
const FOOTPRINT: Rect = Rect::new(0, 556, 1512, 168);
const LG_BANNER: u32 = 0xff30_3030;
const CONTROLS: u32 = 0xff00_ff00;
const KEY_CHANNELUP: u32 = 402;
const KEY_CHANNELDOWN: u32 = 403;

struct Rig {
    client: Client,
    _kb: wayland_client::protocol::wl_keyboard::WlKeyboard,
    _stub: Window,
    _live: Window,
    _bufs: Vec<WlBuffer>,
}

/// Native Home and Live TV: a card whose picture is LG's banner bar and a control at the top right, over a video hole that covers the screen.
fn rig(server: &mut Server) -> Rig {
    server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    let mut client = Client::connect(server);
    let stub = client.card("_WEBOS_WINDOW_TYPE_CARD", "Stub");
    stub.webos.set_property("appId".into(), "com.webos.app.home".into());
    let clear = client.shm_buffer(64, 64, 0);
    client.present(&stub.surface, &clear);
    client.roundtrip();
    server.eval(|s| {
        s.scene.set_home_rust(true);
        s.tick_at(1_000 * MS);
    });
    let live = client.card("_WEBOS_WINDOW_TYPE_CARD", "Live TV");
    live.webos.set_property("appId".into(), "com.webos.app.livetv".into());
    client.roundtrip();
    let sid = server.eval(|s| s.scene.windows().find(|w| w.app_id() == "com.webos.app.livetv").map(|w| w.surface).unwrap());
    server.eval(move |s| s.scene.set_video_expected(sid, true));
    let ui = client.shm_pattern(1920, 1080, |x, y| {
        if (556..724).contains(&y) && x < 1512 {
            LG_BANNER
        } else if y < 100 && x > 1700 {
            CONTROLS
        } else {
            0
        }
    });
    client.present(&live.surface, &ui);
    client.roundtrip();
    server.eval(move |s| {
        s.scene.holes.push(Hole { surface: sid, rect: FULL });
        s.tick_at(1_500 * MS);
    });
    let kb = client.seat.get_keyboard(&client.qh, ());
    assert!(client.wait_until(Duration::from_secs(3), |s| s.keyboard_enters >= 1), "Live TV's keyboard has the focus");
    Rig { client, _kb: kb, _stub: stub, _live: live, _bufs: vec![clear, ui] }
}

fn claim(server: &Server, keys: &[u32], hold_ms: u64) {
    let body = json!({"op": "claim", "app": "com.webos.app.livetv", "rects": [{"x": FOOTPRINT.x, "y": FOOTPRINT.y, "w": FOOTPRINT.w, "h": FOOTPRINT.h}], "keys": keys, "holdMs": hold_ms});
    server.eval(move |s| {
        s.scene.mask_op(Op::parse(&body).expect("a claim"));
    });
}

fn press(rig: &mut Rig, server: &Server, code: u32) {
    server.eval(move |s| {
        s.key_event(SeatId(0), code, true, false);
        s.key_event(SeatId(0), code, false, false);
    });
    rig.client.roundtrip();
}

/// One tick at `ns`: (the plan's damage, whether it blanks, the picture).
fn tick(server: &Server, ns: u64) -> (Rect, bool, lumatv_compositor::output::Image) {
    server.eval(move |s| {
        s.tick_at(ns);
        let plan = s.scene.last_plan();
        (plan.damage, !plan.masks.is_empty(), s.output.capture().unwrap())
    })
}

const DRAWN: [u8; 4] = [0x30, 0x30, 0x30, 0xff];
const VIDEO: [u8; 4] = [0, 0, 0, 0];
const GREEN: [u8; 4] = [0, 0xff, 0, 0xff];

#[test]
fn a_channel_key_to_live_tv_blanks_the_claimed_region_for_the_hold_and_the_video_shows_there() {
    let mut server = Server::start();
    let mut rig = rig(&mut server);
    claim(&server, &[KEY_CHANNELUP, KEY_CHANNELDOWN], 6_500);
    let (_, blank, image) = tick(&server, 2_000 * MS);
    assert!(!blank && image.pixel(300, 600) == DRAWN && image.pixel(1800, 50) == GREEN, "no key yet: LG's banner is drawn");

    press(&mut rig, &server, KEY_CHANNELUP);
    let (damage, blank, image) = tick(&server, 2_100 * MS);
    assert!(blank, "the key's blanking is in the plan");
    assert_eq!((image.pixel(300, 600), image.pixel(1500, 700)), (VIDEO, VIDEO), "the claimed region is the video's hole: LG's banner is not drawn");
    assert_eq!((image.pixel(1600, 600), image.pixel(1800, 50)), (VIDEO, GREEN), "outside the region the card's picture is as it was");
    assert_eq!(damage.intersect(&FOOTPRINT), FOOTPRINT, "the pixels it takes over are drawn again");
    assert_eq!(server.eval(|s| s.scene.mask_report()["active"].clone()), json!(true));

    let (damage, blank, _) = tick(&server, 2_100 * MS + 6_000 * MS);
    assert!(blank && damage.is_empty(), "it holds with nothing more to draw");
    let (damage, blank, image) = tick(&server, 2_100 * MS + 6_501 * MS);
    assert!(!blank && image.pixel(300, 600) == DRAWN, "the hold ended: LG's banner is drawn again");
    assert_eq!(damage.intersect(&FOOTPRINT), FOOTPRINT, "and the region is drawn again");
    assert_eq!(server.eval(|s| s.scene.mask_report()["active"].clone()), json!(false));

    // a second press carries the blanking on from its own plan, and each key does it
    press(&mut rig, &server, KEY_CHANNELDOWN);
    tick(&server, 20_000 * MS);
    let (_, blank, _) = tick(&server, 20_000 * MS + 6_000 * MS);
    assert!(blank);
}

#[test]
fn a_key_that_is_not_the_claims_and_a_claim_on_another_app_blank_nothing() {
    let mut server = Server::start();
    let mut rig = rig(&mut server);
    claim(&server, &[KEY_CHANNELDOWN], 6_500);
    press(&mut rig, &server, KEY_CHANNELUP);
    let (_, blank, image) = tick(&server, 2_000 * MS);
    assert!(!blank && image.pixel(300, 600) == DRAWN, "CH+ is not one of the claim's keys");

    let other = json!({"op": "claim", "app": "com.example.other", "rects": [{"x": 0, "y": 556, "w": 1512, "h": 168}], "keys": [KEY_CHANNELUP], "holdMs": 6500});
    server.eval(move |s| {
        s.scene.mask_op(Op::parse(&other).unwrap());
    });
    press(&mut rig, &server, KEY_CHANNELUP);
    let (_, blank, image) = tick(&server, 3_000 * MS);
    assert!(!blank && image.pixel(300, 600) == DRAWN, "the key went to Live TV, whose window is not the claimed app's");
}

#[test]
fn an_engage_blanks_without_a_key_and_a_release_gives_the_region_back_at_once() {
    let mut server = Server::start();
    let _rig = rig(&mut server);
    claim(&server, &[KEY_CHANNELUP], 6_500);
    server.eval(|s| s.scene.mask_op(Op::Engage(5_000)));
    let (_, blank, image) = tick(&server, 2_000 * MS);
    assert!(blank && image.pixel(300, 600) == VIDEO, "Live TV came to the front: its launch banner is not drawn");
    // the banner is found to be elsewhere (the position setting answered): the new claim keeps what runs, and blanks its own place
    let moved = Rect::new(414, 0, 1506, 168);
    let claim_moved = json!({"op": "claim", "app": "com.webos.app.livetv", "rects": [{"x": moved.x, "y": moved.y, "w": moved.w, "h": moved.h}], "keys": [KEY_CHANNELUP], "holdMs": 6_500});
    server.eval(move |s| {
        s.scene.mask_op(Op::parse(&claim_moved).unwrap());
    });
    let (_, blank, image) = tick(&server, 6_900 * MS);
    assert!(
        blank && image.pixel(1800, 50) == VIDEO && image.pixel(300, 600) == DRAWN,
        "a replaced claim does not end it, and what it no longer names is drawn"
    );
    let (_, blank, image) = tick(&server, 7_100 * MS);
    assert!(!blank && image.pixel(1800, 50) == GREEN, "five seconds from the plan that began it");

    server.eval(|s| s.scene.mask_op(Op::Engage(5_000)));
    tick(&server, 8_000 * MS);
    assert!(server.eval(|s| s.scene.mask_op(Op::Release)), "a release is a change the next frame shows");
    let (damage, blank, image) = tick(&server, 8_100 * MS);
    assert!(!blank && image.pixel(1800, 50) == GREEN, "released: the card's own picture is back, with the claim gone");
    assert_eq!(damage.intersect(&moved), moved);
    assert_eq!(server.eval(|s| s.scene.mask_report()), serde_json::Value::Null);
    assert!(!server.eval(|s| s.scene.mask_op(Op::Release)), "nothing left to release");
}

#[test]
fn the_shells_posts_make_the_claim_and_the_shell_going_away_gives_the_apps_picture_back() {
    let mut server = Server::start();
    let mut rig = rig(&mut server);
    server.eval(|s| {
        s.shell_rust_open(0, "test".into()).unwrap();
        s.shell_rust_handle(Msg::Connected);
    });
    let body = json!({"op": "claim", "app": "com.webos.app.livetv", "rects": [{"x": FOOTPRINT.x, "y": FOOTPRINT.y, "w": FOOTPRINT.w, "h": FOOTPRINT.h}], "keys": [KEY_CHANNELUP], "holdMs": 6_500});
    server.eval(move |s| {
        s.shell_rust_handle(Msg::Mask(body));
        s.shell_rust_handle(Msg::Mask(json!({"op": "dance"})));
    });
    press(&mut rig, &server, KEY_CHANNELUP);
    let (_, blank, image) = tick(&server, 2_000 * MS);
    assert!(blank && image.pixel(300, 600) == VIDEO, "the shell's claim blanks on the key, and a post that is no op of it changes nothing");
    server.eval(|s| s.shell_rust_restart());
    let (damage, blank, image) = tick(&server, 2_100 * MS);
    assert!(!blank && image.pixel(300, 600) == DRAWN, "the shell is gone: LG's banner is the app's own again");
    assert_eq!(damage.intersect(&FOOTPRINT), FOOTPRINT);
    assert_eq!(server.eval(|s| s.scene.mask_report()), serde_json::Value::Null);
}
