//! The box dim on the scene and the software renderer (`boxdim`, `scene.rs`, `render/software.rs`): a bright box that stays still dims
//! after the wait, feathered, while the rest of the picture and the shell do not; the app keeps the plane until then and gets it back on
//! a key; small damage leaves the dim, big damage and a capture take it away; a hole samples black.

mod harness;

use std::sync::atomic::Ordering;

use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::boxdim::{CELL, COLS, Settings};
use lumatv_compositor::output::Image;
use lumatv_compositor::scene::Hole;
use lumatv_compositor::window::WindowType;
use lumatv_compositor::{OUTPUT_WIDTH, Rect};
use wayland_client::protocol::wl_buffer::WlBuffer;

const SEC: u64 = 1_000_000_000;
const MS: u64 = 1_000_000;
/// The white box: cells 10 to 20 on both axes.
const BOX: (i32, i32) = (10 * CELL, 21 * CELL);
const GREY: u32 = 0xff40_4040;
const WHITE: u32 = 0xffff_ffff;

/// A white box on a mid grey (luma 64, below the dim's `min_luma`: never dimmed on its own).
fn picture(x: i32, y: i32) -> u32 {
    if (BOX.0..BOX.1).contains(&x) && (BOX.0..BOX.1).contains(&y) { WHITE } else { GREY }
}

fn quiet_server() -> Server {
    let server = Server::start();
    server.ticking.store(false, Ordering::SeqCst);
    server
}

/// A full-screen opaque app card showing `pixel`.
fn app_card(server: &mut Server, pixel: impl Fn(i32, i32) -> u32) -> (Client, Window, WlBuffer) {
    let mut app = Client::connect(server);
    let card = app.card("_WEBOS_WINDOW_TYPE_CARD", "Netflix");
    card.webos.set_property("appId".into(), "com.example.netflix".into());
    let buf = app.shm_pattern(1920, 1080, pixel);
    let opaque = app.region(0, 0, 1920, 1080);
    card.surface.set_opaque_region(Some(&opaque));
    app.present(&card.surface, &buf);
    app.roundtrip();
    (app, card, buf)
}

/// Quick timings: a minute still, the ramp over 20 s.
fn on(s: &mut State) {
    s.scene.care.settings.boxdim = Settings { on: true, after_ns: 60 * SEC, ramp_ns: 20 * SEC, floor: 0.8, min_luma: 0.35 };
}

/// Ticks every `step` for `for_ns` from `t`; the time reached.
fn run(s: &mut State, t: u64, for_ns: u64, step: u64) -> u64 {
    let end = t + for_ns;
    let mut t = t;
    while t < end {
        t += step;
        s.tick_at(t);
    }
    t
}

/// [`run`] in evals of 10 s of ticks each: a run of minutes never nears the harness's wait on one eval.
fn run_on(server: &Server, t: u64, for_ns: u64, step: u64) -> u64 {
    let mut t = t;
    let end = t + for_ns;
    while t < end {
        let part = (end - t).min(10 * SEC);
        t = server.eval(move |s| run(s, t, part, step));
    }
    t
}

fn px(img: &Image, x: i32, y: i32) -> [u8; 4] {
    let i = (y as usize * img.width as usize + x as usize) * 4;
    [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2], img.rgba[i + 3]]
}

fn direct(s: &State) -> bool {
    s.scene.last_plan().sole_fullscreen().is_some()
}

/// The dim as drawn: the box's centre at the floor, its inside uniform, the grey around it feathered with no step wider than a code
/// value, and nothing far away touched.
fn check_dimmed(img: &Image) {
    let c = (BOX.0 + BOX.1) / 2;
    assert_eq!(px(img, c, c), [204, 204, 204, 255], "the box at 0.8");
    let inside: Vec<u8> = (BOX.0 + CELL..BOX.1 - CELL).map(|x| px(img, x, c)[0]).collect();
    assert!(inside.iter().all(|v| *v == 204), "uniform inside the box: {inside:?}");
    let row: Vec<u8> = (0..BOX.0).map(|x| px(img, x, c)[1]).collect();
    assert!(row.windows(2).all(|w| w[1] <= w[0] && w[0] - w[1] <= 1), "the grey falls toward the box one code at a time: {row:?}");
    let ramp = row.iter().filter(|v| **v < 64 && **v > 52).count();
    assert!(ramp >= 30, "the feather is at least a cell wide: {ramp} px");
    assert_eq!(px(img, 100, 100), [64, 64, 64, 255], "far away untouched");
    assert_eq!(px(img, OUTPUT_WIDTH as i32 - 1, 1079), [64, 64, 64, 255]);
}

#[test]
fn a_still_bright_box_dims_after_the_wait_feathered_and_the_app_keeps_the_plane_until_then() {
    let mut server = quiet_server();
    let (_app, _card, _buf) = app_card(&mut server, picture);
    let t = server.eval(|s| {
        s.tick_at(SEC);
        assert!(direct(s), "an opaque full-screen app: direct scan-out");
        SEC
    });
    let t = run_on(&server, t, 30 * SEC, 2 * SEC);
    let t = server.eval(move |s| {
        assert!(!s.scene.boxdim.engaged() && s.scene.boxdim.samples == 0, "off: nothing sampled, nothing dimmed");
        on(s);
        t
    });
    let t = run_on(&server, t, 50 * SEC, 2 * SEC);
    server.eval(|s| {
        assert!(!s.scene.boxdim.engaged() && direct(s), "still for less than the wait: nothing, and the plane is kept");
        assert!(s.scene.boxdim.samples >= 10, "sampled every 4 s: {}", s.scene.boxdim.samples);
        assert_eq!(s.scene.boxdim.luma(15, 15), 255, "a white cell");
        assert_eq!(s.scene.boxdim.luma(40, 30), 64, "a grey cell");
    });
    let t = run_on(&server, t, 40 * SEC, 2 * SEC);
    server.eval(move |s| {
        assert!(s.scene.boxdim.engaged(), "engaged after the wait and the ramp");
        assert!(!direct(s), "engaged: composed");
        assert!(s.journal.iter().any(|l| l.contains("oledcare: boxDim on:")), "journalled: {:?}", s.journal);
        let img = s.output.capture().unwrap();
        check_dimmed(&img);
        let status = s.scene.boxdim.report(t, &s.scene.care.settings.boxdim);
        assert_eq!(status["engaged"], true);
        assert_eq!(status["lowest"], 0.8);
        assert!(status["cells"].as_u64().unwrap() >= 121, "{status}");
    });
}

#[test]
fn a_key_brings_the_picture_back_within_250_ms_and_the_plane_with_it() {
    let mut server = quiet_server();
    let (_app, _card, _buf) = app_card(&mut server, picture);
    let t = server.eval(|s| {
        on(s);
        s.tick_at(SEC);
        SEC
    });
    let t = run_on(&server, t, 100 * SEC, 2 * SEC);
    let t = server.eval(move |s| {
        assert!(s.scene.boxdim.engaged());
        let c = (BOX.0 + BOX.1) / 2;
        let mut t = t;
        s.scene.input(t);
        let mut seen = vec![204u8];
        let mut frames = 0;
        while s.scene.boxdim.engaged() {
            t += 16 * MS;
            s.tick_at(t);
            frames += 1;
            seen.push(px(&s.output.capture().unwrap(), c, c)[0]);
            assert!(frames <= 15, "back within 250 ms: {seen:?}");
        }
        assert!(seen.windows(2).all(|w| w[1] >= w[0]), "monotone: {seen:?}");
        assert!(frames >= 4, "a ramp, not a jump: {seen:?}");
        assert_eq!(*seen.last().unwrap(), 255);
        assert!(direct(s), "the app has the plane again");
        assert!(s.journal.iter().any(|l| l == "oledcare: boxDim off (key)"), "{:?}", s.journal);
        t
    });
    let t = run_on(&server, t, 50 * SEC, 2 * SEC);
    server.eval(|s| assert!(!s.scene.boxdim.engaged(), "and the wait starts over"));
    run_on(&server, t, 40 * SEC, 2 * SEC);
    server.eval(|s| assert!(s.scene.boxdim.engaged()));
}

#[test]
fn small_damage_keeps_the_dim_big_damage_and_a_capture_take_it_away_and_a_hole_samples_black() {
    let mut server = quiet_server();
    let (mut app, card, buf) = app_card(&mut server, picture);
    let t = server.eval(|s| {
        on(s);
        s.tick_at(SEC);
        SEC
    });
    let t = run_on(&server, t, 100 * SEC, 2 * SEC);
    server.eval(|s| assert!(s.scene.boxdim.engaged()));
    // A clock of 60 x 30 px ticks: its own two cells restart, the box stays dimmed.
    card.surface.attach(Some(&buf), 0, 0);
    card.surface.damage(900, 500, 60, 30);
    card.surface.commit();
    app.roundtrip();
    let t = server.eval(move |s| {
        let t = run(s, t, 2 * SEC, 500 * MS);
        assert!(s.scene.boxdim.engaged(), "a small change does not restore the screen");
        let c = (BOX.0 + BOX.1) / 2;
        assert_eq!(px(&s.output.capture().unwrap(), c, c)[0], 204);
        t
    });
    // A popup of 400 x 200 px: everything restarts.
    card.surface.attach(Some(&buf), 0, 0);
    card.surface.damage(700, 400, 400, 200);
    card.surface.commit();
    app.roundtrip();
    let t = server.eval(move |s| {
        let t = run(s, t, 300 * MS, 16 * MS);
        assert!(!s.scene.boxdim.engaged(), "a real change restores everything");
        assert!(s.journal.iter().any(|l| l == "oledcare: boxDim off (change)"), "{:?}", s.journal);
        t
    });
    let t = run_on(&server, t, 100 * SEC, 2 * SEC);
    let t = server.eval(move |s| {
        assert!(s.scene.boxdim.engaged(), "and it dims again after the wait");
        // A capture armed: the next plan carries no map, in one step, and damages the dimmed region.
        s.scene.boxdim.capturing = true;
        let State { scene, surfaces, .. } = s;
        let plan = scene.plan_at(surfaces, t + SEC);
        assert!(plan.gain().is_none(), "no map in the frame a capture reads");
        assert!(plan.damage.contains(BOX.0 + 10, BOX.0 + 10) && plan.damage.contains(BOX.1 - 10, BOX.1 - 10), "{:?}", plan.damage);
        surfaces.clear_damage();
        s.scene.boxdim.capturing = false;
        assert!(s.scene.boxdim.take_journal().iter().any(|l| l == "oledcare: boxDim off (capture)"));
        t + SEC
    });
    // A video in a hole over the box: the app draws transparent there (the plane shows through), and what is under the video samples
    // black, so nothing is bright enough to dim.
    let hole = Rect::new(BOX.0 - CELL, BOX.0 - CELL, BOX.1 - BOX.0 + 2 * CELL, BOX.1 - BOX.0 + 2 * CELL);
    let punched = app.shm_pattern(1920, 1080, move |x, y| if hole.contains(x, y) { 0 } else { picture(x, y) });
    app.present(&card.surface, &punched);
    app.roundtrip();
    let t = server.eval(move |s| {
        let surface = s.scene.windows().find(|w| w.kind == WindowType::Card).unwrap().surface;
        s.scene.holes.push(Hole { surface, rect: hole });
        t
    });
    run_on(&server, t, 120 * SEC, 2 * SEC);
    server.eval(|s| {
        assert!(!s.scene.boxdim.engaged(), "nothing bright outside the hole");
        assert_eq!(s.scene.boxdim.luma(15, 15), 0, "the hole's cells read black");
        assert_eq!(s.scene.boxdim.luma(40, 30), 64);
        assert!(s.scene.boxdim.samples > 0);
        let _ = COLS;
    });
    drop(punched);
}
