//! The box dim's GPU side (`render/gl/boxdim.rs`) on Mesa's software GL: the sample pass reads each cell's brightness, holes black; the
//! gained program draws the map feathered and leaves what is not an app's; and what a frame with the map costs against one without
//! (printed, a host number for `docs/research/boxdim-result.md`).

mod harness;

use std::path::Path;
use std::time::Instant;

use harness::{Client, Server};
use lumatv_compositor::State;
use lumatv_compositor::boxdim::{CELL, CELLS, Settings};
use lumatv_compositor::output::Image;
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl};
use lumatv_compositor::scene::Hole;
use lumatv_compositor::{OUTPUT_HEIGHT, OUTPUT_WIDTH, Rect};

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";
const SEC: u64 = 1_000_000_000;
const MS: u64 = 1_000_000;
/// The white box: cells 10 to 20 on both axes, plus a half cell on the right (x 630 to 645).
const BOX: (i32, i32) = (10 * CELL, 21 * CELL);
const GREY: u32 = 0xff40_4040;
const WHITE: u32 = 0xffff_ffff;
const HOLE: Rect = Rect { x: 40 * CELL, y: 20 * CELL, w: 8 * CELL, h: 6 * CELL };

fn picture(x: i32, y: i32) -> u32 {
    if HOLE.contains(x, y) {
        0
    } else if (BOX.0..BOX.1 + CELL / 2).contains(&x) && (BOX.0..BOX.1).contains(&y) {
        WHITE
    } else {
        GREY
    }
}

/// A server on Mesa with a full-screen opaque app showing the picture and a hole; `None` without Mesa.
fn screen() -> Option<(Server, Client)> {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return None;
    }
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let mut server = Server::start_with(Box::new(|| Box::new(Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap())));
    server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Boxes");
    let opaque = client.region(0, 0, 1920, 1080);
    card.surface.set_opaque_region(Some(&opaque));
    let buf = client.shm_pattern(1920, 1080, picture);
    client.present(&card.surface, &buf);
    client.roundtrip();
    server.eval(|s| {
        let surface = s.scene.windows().find(|w| w.title == "Boxes").unwrap().surface;
        s.scene.holes.push(Hole { surface, rect: HOLE });
        s.scene.care.settings.boxdim = Settings { on: true, after_ns: 60 * SEC, ramp_ns: 20 * SEC, floor: 0.8, min_luma: 0.35 };
    });
    std::mem::forget(buf);
    Some((server, client))
}

fn run(s: &mut State, t: u64, for_ns: u64, step: u64) -> u64 {
    let end = t + for_ns;
    let mut t = t;
    while t < end {
        t += step;
        s.tick_at(t);
    }
    t
}

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

#[test]
fn the_sample_pass_reads_each_cells_brightness_with_the_hole_black() {
    let Some((server, _client)) = screen() else { return };
    let luma = server.eval(|s| {
        s.tick_at(SEC);
        let State { scene, surfaces, output, .. } = s;
        let mut luma = [0u8; CELLS];
        let began = Instant::now();
        assert!(output.sample_cells(scene.last_plan(), surfaces, &mut luma).unwrap());
        eprintln!("boxdim: sample pass {} us on Mesa", began.elapsed().as_micros());
        luma
    });
    let at = |c: usize, r: usize| luma[r * 64 + c];
    assert!(at(15, 15) >= 254, "a white cell: {}", at(15, 15));
    assert!(at(10, 10) >= 254 && at(20, 20) >= 254, "the box's corner cells are whole cells");
    assert!(at(40, 30).abs_diff(64) <= 1, "a grey cell: {}", at(40, 30));
    assert!(at(0, 0).abs_diff(64) <= 1 && at(63, 35).abs_diff(64) <= 1, "the screen's corners: {} {}", at(0, 0), at(63, 35));
    let half = at(21, 15);
    assert!(half.abs_diff(((255u32 + 64) / 2) as u8) <= 2, "a cell half white, half grey averages both: {half}");
    assert!(at(9, 15).abs_diff(64) <= 1, "the cell left of the box is grey: {}", at(9, 15));
    for r in 20..26 {
        for c in 40..48 {
            assert_eq!(at(c, r), 0, "the hole's cell ({c}, {r}) is black");
        }
    }
    assert!(at(39, 22).abs_diff(64) <= 1 && at(48, 22).abs_diff(64) <= 1, "beside the hole: grey");
}

#[test]
fn the_gained_program_draws_the_map_feathered_and_a_frame_with_it_costs_about_the_same() {
    let Some((server, _client)) = screen() else { return };
    let t = server.eval(|s| {
        s.tick_at(SEC);
        SEC
    });
    let t = run_on(&server, t, 100 * SEC, 2 * SEC);
    server.eval(move |s| {
        assert!(s.scene.boxdim.engaged(), "engaged after the wait and the ramp");
        let img = s.output.capture().unwrap();
        let c = (BOX.0 + BOX.1) / 2;
        assert!(px(&img, c, c)[0].abs_diff(204) <= 1, "the box at 0.8: {:?}", px(&img, c, c));
        let inside: Vec<u8> = (BOX.0 + CELL..BOX.1 - CELL).map(|x| px(&img, x, c)[0]).collect();
        assert!(inside.iter().all(|v| v.abs_diff(204) <= 1), "uniform inside the box: {inside:?}");
        let row: Vec<u8> = (0..BOX.0).map(|x| px(&img, x, c)[1]).collect();
        assert!(row.windows(2).all(|w| w[1] <= w[0] + 1 && w[0].saturating_sub(w[1]) <= 2), "no step wider than two codes: {row:?}");
        let ramp = row.iter().filter(|v| **v < 63 && **v > 53).count();
        assert!(ramp >= 30, "the feather is at least a cell wide: {ramp} px");
        assert!(px(&img, 100, 100)[0].abs_diff(64) <= 1, "far away untouched: {:?}", px(&img, 100, 100));
        assert!(px(&img, OUTPUT_WIDTH as i32 - 1, OUTPUT_HEIGHT as i32 - 1)[0].abs_diff(64) <= 1);
        assert_eq!(px(&img, HOLE.x + 10, HOLE.y + 10), [0, 0, 0, 0], "the hole stays transparent");
        // Cost: one full redraw with the map against one without, on this GL.
        let cost = |s: &mut State, t: u64| {
            s.scene.redraw_all();
            let State { scene, surfaces, output, .. } = s;
            let plan = scene.plan_at(surfaces, t);
            let began = Instant::now();
            let stats = output.render(plan, surfaces).unwrap();
            let us = began.elapsed().as_micros();
            surfaces.clear_damage();
            (us, stats.render_us)
        };
        let with = (0..3).map(|i| cost(s, t + 10 * MS * (i + 1))).min().unwrap();
        s.scene.care.settings.boxdim.on = false;
        let _ = cost(s, t + SEC);
        assert!(!s.scene.boxdim.engaged());
        let without = (0..3).map(|i| cost(s, t + SEC + 10 * MS * (i + 1))).min().unwrap();
        eprintln!("boxdim: full 1080p frame with the map {} us, without {} us (Mesa llvmpipe; render_us {} / {})", with.0, without.0, with.1, without.1);
    });
}
