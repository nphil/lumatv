//! The Rust shell's Home under the compositor's transitions (feature `ui`, Mesa's software GL). Its Home is not a window: it is the engine's
//! layers under the windows (stack 0), so an app's card grows over it and shrinks back onto it, Home is told it is still on the screen while the
//! card moves (its launch screen and its layers stay up), and an app on the video plane (a hole) arrives on its veil and leaves with Home covering
//! the hole at once, so the plane's black is never what shows. Every frame of every move is read back, and none has a black pixel.
#![cfg(feature = "ui")]

mod harness;

use compositor_protocol::{Fill, Props, Rgba, Shape};
use harness::home_engine::{self, HomeEngine};
use harness::{Client, Server, Window};
use lumatv_compositor::output::Image;
use lumatv_compositor::scene::Hole;
use serde_json::json;
use wayland_client::protocol::wl_buffer::WlBuffer;

const MS: u64 = 1_000_000;
const HOME_STUB: &str = "com.webos.app.home";
const HOME: [u8; 3] = [32, 48, 96];
const APP_A: u32 = 0xffd0_4040;
const TILE: (i32, i32, i32, i32) = (287, 699, 120, 120);

struct Rig {
    server: Server,
    client: Client,
    _keep: Vec<(Window, WlBuffer)>,
    engine: HomeEngine,
}

/// The Rust shell's Home as the engine's ground, the Home stub card over nothing, the compositor's `home_rust` on and transitions forced on.
fn rig() -> Rig {
    let (mut server, engine) = home_engine::start(HOME);
    let mut client = Client::connect(&mut server);
    let stub = client.card("_WEBOS_WINDOW_TYPE_CARD", "Stub");
    stub.webos.set_property("appId".into(), HOME_STUB.into());
    let clear = client.shm_buffer(64, 64, 0);
    client.present(&stub.surface, &clear);
    client.roundtrip();
    server.eval(|s| {
        s.scene.set_home_rust(true);
        s.motion.forced = Some(true);
        s.apply_motion();
        s.set_launch_origin(&json!({"appId": "com.example.a", "tile": {"x": TILE.0, "y": TILE.1, "width": TILE.2, "height": TILE.3, "radius": 22}}));
        s.set_launch_origin(&json!({"appId": "com.webos.app.hdmi2", "tile": {"x": TILE.0, "y": TILE.1, "width": TILE.2, "height": TILE.3, "radius": 22}}));
        s.tick_at(1_000 * MS);
    });
    Rig { server, client, _keep: vec![(stub, clear)], engine }
}

fn tick(server: &Server, ns: u64) -> Image {
    server.eval(move |s| {
        s.tick_at(ns);
        s.output.capture().unwrap()
    })
}

/// Pixels of the frame that are black: opaque black is the plane's black under a UI that did not cover it, and a cleared hole ([0, 0, 0, 0])
/// is the video plane showing through.
fn black(image: &Image, clear_too: bool) -> usize {
    let mut n = 0;
    for y in (3..1080).step_by(17) {
        for x in (3..1920).step_by(23) {
            let p = image.pixel(x, y);
            if p[3] == 255 && p[..3] == [0, 0, 0] || clear_too && p[3] == 0 {
                n += 1;
            }
        }
    }
    n
}

fn app(rig: &mut Rig, id: &str) -> (Window, WlBuffer) {
    let win = rig.client.card("_WEBOS_WINDOW_TYPE_CARD", id);
    win.webos.set_property("appId".into(), id.into());
    let buf = rig.client.shm_buffer(1920, 1080, APP_A);
    let whole = rig.client.region(0, 0, 1920, 1080);
    win.surface.set_opaque_region(Some(&whole));
    rig.client.present(&win.surface, &buf);
    rig.client.roundtrip();
    (win, buf)
}

#[test]
fn a_launch_and_a_return_are_a_card_growing_out_of_its_tile_over_the_rust_home_and_back_with_home_told_it_stays_and_no_black_frame() {
    if !home_engine::available() {
        return;
    }
    let mut rig = rig();
    let home = tick(&rig.server, 1_100 * MS);
    assert_eq!(home.pixel(700, 300)[..3], HOME, "Home is the engine's ground");
    let (win, _buf) = app(&mut rig, "com.example.a");
    // the transition is requested for the launch, and Home is told it is still the screen's while the card moves (its layers and its launch screen stay)
    let t0 = 2_000 * MS;
    let mut worst = 0;
    for step in 0..16u64 {
        let img = tick(&rig.server, t0 + step * 16 * MS);
        worst = worst.max(black(&img, true));
        if step == 0 {
            assert!(rig.server.eval(|s| s.scene.transitions.stats.launches == 1 && s.scene.transitions.holds_home()), "a launch is moving and Home is held");
            let plan = rig.server.eval(|s| (s.scene.last_plan().animating, s.scene.last_plan().home_hidden));
            assert_eq!(plan, (true, false), "the card is moving: the next frame is due at the next refresh, and Home is drawn under it");
            let topic = rig.server.eval(|s| s.shell_topic_values().into_iter().find(|(n, _)| n == "foreground").map(|(_, v)| v));
            let topic = topic.expect("the foreground topic");
            assert_eq!((topic["home"].as_bool(), topic["covered"].as_bool()), (Some(true), Some(false)), "{topic}");
        }
    }
    assert_eq!(worst, 0, "no frame of the launch has a black pixel");
    let settled = tick(&rig.server, t0 + 400 * MS);
    assert_eq!(settled.pixel(960, 540)[..3], [0xd0, 0x40, 0x40]);
    assert!(rig.server.eval(|s| !s.scene.transitions.holds_home()), "the card is whole: Home is covered now");
    assert_eq!(rig.server.eval(|s| (s.scene.last_plan().animating, s.scene.last_plan().home_hidden)), (false, true), "nothing moves and Home is not drawn");

    // the app goes: its last picture shrinks into the tile over Home, which is under it from the first frame
    win.surface.destroy();
    rig.client.roundtrip();
    let t1 = 3_000 * MS;
    let mut worst = 0;
    let mut last = None;
    for step in 0..14u64 {
        let img = tick(&rig.server, t1 + step * 16 * MS);
        worst = worst.max(black(&img, true));
        last = Some(img);
    }
    assert_eq!(worst, 0, "no frame of the return has a black pixel");
    assert_eq!(last.expect("frames").pixel(5, 5)[..3], HOME, "Home, whole, at the end");
    assert_eq!(rig.server.eval(|s| (s.scene.transitions.stats.launches, s.scene.transitions.stats.leaves)), (1, 1));
}

#[test]
fn an_app_on_the_video_plane_arrives_on_a_veil_and_leaves_with_home_covering_its_hole_at_once() {
    if !home_engine::available() {
        return;
    }
    let mut rig = rig();
    let id = "com.webos.app.hdmi2";
    let win = rig.client.card("_WEBOS_WINDOW_TYPE_CARD", "Hdmi");
    win.webos.set_property("appId".into(), id.into());
    rig.client.roundtrip();
    let sid = rig.server.eval(move |s| s.scene.windows().find(|w| w.app_id() == id).map(|w| w.surface).unwrap());
    rig.server.eval(move |s| {
        s.scene.set_video_expected(sid, true);
        s.scene.holes.push(Hole { surface: sid, rect: lumatv_compositor::Rect::new(0, 0, 1920, 1080) });
    });
    let buf = rig.client.shm_pattern(1920, 1080, |x, y| if x < 500 && y < 120 { 0xff30_5080 } else { 0 });
    rig.client.present(&win.surface, &buf);
    rig.client.roundtrip();
    let t0 = 2_000 * MS;
    // the card arrives over Home on its veil: nowhere is the plane's black (a cleared hole) or an opaque black seen while it moves and holds
    for ms in [0u64, 60, 130, 300, 390] {
        let img = tick(&rig.server, t0 + ms * MS);
        assert_eq!(black(&img, true), 0, "the veil at {ms} ms");
    }
    // the app goes: no frame of the way back shows the plane through the hole, and Home is whole at the end
    win.surface.destroy();
    rig.client.roundtrip();
    let t1 = 3_000 * MS;
    for ms in [0u64, 16, 48, 90, 140, 179, 200] {
        let img = tick(&rig.server, t1 + ms * MS);
        assert_eq!(black(&img, true), 0, "the way back at {ms} ms");
    }
    let end = tick(&rig.server, t1 + 400 * MS);
    assert_eq!(end.pixel(960, 540)[..3], HOME);
    assert_eq!(rig.server.eval(|s| (s.scene.transitions.stats.launches, s.scene.transitions.stats.leaves)), (1, 1));
}

#[test]
fn a_rust_home_under_an_app_that_covers_it_is_not_drawn_and_is_drawn_whole_the_frame_the_app_goes() {
    if !home_engine::available() {
        return;
    }
    let mut rig = rig();
    // Home as more than a ground: twelve pieces over it, each a draw
    let solid = |c: [u8; 3]| Shape { corner_radius: 0.0, fill: Fill::Solid(Rgba(c[0], c[1], c[2], 255)), border_width: 0.0, border: Rgba(0, 0, 0, 0) };
    let pieces: Vec<_> = (0..12).map(|i| rig.engine.ui.scene.shape(100, 60, 10 + i, solid([200, 200, 60]), Props::at(40.0 + 120.0 * i as f32, 300.0))).collect();
    rig.engine.ui.try_commit().unwrap();
    // no motion: the card is there at once, with no transition to draw Home through
    rig.server.eval(|s| {
        s.motion.forced = Some(false);
        s.apply_motion();
    });
    let home = tick(&rig.server, 1_100 * MS);
    assert_eq!(home.pixel(60, 320)[..3], [200, 200, 60], "a piece of Home");
    let home_draws = rig.server.eval(|s| s.stats.ui_draws_last);
    assert!(home_draws >= 2, "Home's layers are drawn: {home_draws}");
    assert!(rig.server.eval(|s| !s.scene.last_plan().home_hidden), "nothing covers it");

    // an app's card over it: Home is not drawn under it, and what is drawn is the card
    let (win, _buf) = app(&mut rig, "com.example.a");
    let covered = tick(&rig.server, 1_200 * MS);
    assert!(rig.server.eval(|s| s.scene.last_plan().home_hidden), "the card covers Home and nothing moves");
    assert_eq!(covered.pixel(60, 320)[..3], [0xd0, 0x40, 0x40], "the card, not the piece of Home");
    assert_eq!(covered.pixel(5, 5)[..3], [0xd0, 0x40, 0x40]);
    assert_eq!(rig.server.eval(|s| s.stats.ui_draws_last), 0, "the engine drew nothing: Home is under the card");

    // a layer of the system layer over the card is drawn alone
    let toast = rig.engine.ui.scene.shape(300, 100, 1000, solid([10, 200, 10]), Props::at(500.0, 500.0));
    rig.engine.ui.try_commit().unwrap();
    let with_toast = tick(&rig.server, 1_300 * MS);
    assert_eq!(with_toast.pixel(600, 540)[..3], [10, 200, 10], "the system layer over the card");
    assert_eq!(with_toast.pixel(60, 320)[..3], [0xd0, 0x40, 0x40]);
    let toast_draws = rig.server.eval(|s| s.stats.ui_draws_last);
    assert!(toast_draws >= 1 && toast_draws < home_draws, "the toast's frame draws the toast and not Home: {toast_draws} against {home_draws}");
    // and a frame of it that moves nothing else draws nothing of Home either
    drop(toast);
    rig.engine.ui.try_commit().unwrap();
    tick(&rig.server, 1_350 * MS);
    assert_eq!(rig.server.eval(|s| s.stats.ui_draws_last), 0);

    // the app goes: the frame that finds Home uncovered draws it whole, the ground and the pieces, at once
    win.surface.destroy();
    rig.client.roundtrip();
    let back = tick(&rig.server, 1_400 * MS);
    assert!(rig.server.eval(|s| !s.scene.last_plan().home_hidden), "Home is uncovered");
    assert_eq!(back.pixel(5, 5)[..3], HOME, "the ground");
    assert_eq!(back.pixel(60, 320)[..3], [200, 200, 60], "a piece of Home, drawn again");
    assert_eq!(back.pixel(1400, 320)[..3], [200, 200, 60], "and the last one");
    assert_eq!(black(&back, true), 0, "no black anywhere in it");
    drop(pieces);
}
