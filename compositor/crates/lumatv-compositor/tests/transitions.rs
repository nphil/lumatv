//! Smooth transitions in the compositor's frame (`transition.rs`, `scene.rs`, the renderers): a launch grows the card out of its tile, Home takes it
//! back into it, one app replaces another, a card on the video plane arrives on a veil and leaves under Home dissolving in, and the logical state (the front
//! card, the keyboard focus, what Home is told) never waits for any of it. The scene is planned at a clock the test moves by hand; the pictures are the
//! software renderer's.

mod harness;

use std::time::Duration;

use harness::{Client, Server, Window};
use lumatv_compositor::State;
use lumatv_compositor::output::Image;
use lumatv_compositor::scene::Hole;
use serde_json::json;
use wayland_client::protocol::wl_buffer::WlBuffer;

const MS: u64 = 1_000_000;
const HOME_STUB: &str = "com.webos.app.home";
// The software renderer does not draw the native engine layer.
const HOME: u32 = 0xff00_0000;
const APP_A: u32 = 0xffd0_4040;
const APP_B: u32 = 0xff40_c060;
/// Netflix's tile on Home, as the shell would say it.
const TILE: (i32, i32, i32, i32) = (287, 699, 120, 120);

struct Rig {
    client: Client,
    stub: Window,
    _bufs: Vec<WlBuffer>,
}

fn rgb(argb: u32) -> [u8; 3] {
    [(argb >> 16) as u8, (argb >> 8) as u8, argb as u8]
}

fn near(a: [u8; 4], want: [u8; 3], tol: u8) -> bool {
    a[3] == 255 && a.iter().zip(want.iter()).all(|(g, w)| g.abs_diff(*w) <= tol)
}

/// Native Home occupies the engine layer above SAM's transparent Home card.
fn rig(server: &mut Server, on: bool) -> Rig {
    server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    let mut client = Client::connect(server);
    let stub = client.card("_WEBOS_WINDOW_TYPE_CARD", "Stub");
    stub.webos.set_property("appId".into(), HOME_STUB.into());
    let clear = client.shm_buffer(64, 64, 0);
    client.present(&stub.surface, &clear);
    client.roundtrip();
    server.eval(|s| { s.scene.set_home_rust(true); });
    if on {
        server.eval(|s| {
            s.motion.forced = Some(true);
            s.apply_motion();
            s.set_launch_origin(&json!({"appId": "com.example.a", "tile": {"x": TILE.0, "y": TILE.1, "width": TILE.2, "height": TILE.3, "radius": 22}}));
            s.tick_at(1_000 * MS);
        });
    }
    Rig { client, stub, _bufs: vec![clear] }
}

/// An app's card: mapped over Home with a full-screen picture of one colour.
fn app(rig: &mut Rig, name: &str, id: &str, argb: u32) -> (Window, WlBuffer) {
    let win = rig.client.card("_WEBOS_WINDOW_TYPE_CARD", name);
    win.webos.set_property("appId".into(), id.into());
    let buf = rig.client.shm_buffer(1920, 1080, argb);
    // an app that fills its window says so, as the apps do: what lets the plane scan a lone window out directly
    let whole = rig.client.region(0, 0, 1920, 1080);
    win.surface.set_opaque_region(Some(&whole));
    rig.client.present(&win.surface, &buf);
    rig.client.roundtrip();
    (win, buf)
}

fn tick(server: &Server, ns: u64) -> Image {
    server.eval(move |s| {
        s.tick_at(ns);
        s.output.capture().unwrap()
    })
}

fn state<R: Send + 'static>(server: &Server, f: impl FnOnce(&mut State) -> R + Send + 'static) -> R {
    server.eval(f)
}

#[test]
fn a_launch_is_drawn_from_the_first_frame_as_a_card_growing_out_of_its_tile() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    let (_a, _buf) = app(&mut rig, "A", "com.example.a", APP_A);
    // The front card is the app's, and the keys are its, before any frame is drawn: nothing waits for the move.
    let (front, focus) = state(&server, |s| {
        (
            s.scene.foreground(&s.surfaces).map(|w| w.app_id().to_string()),
            s.seat.focus.and_then(|f| s.scene.window_of(f)).map(|w| s.scene.window(w).unwrap().app_id().to_string()),
        )
    });
    assert_eq!(front.as_deref(), Some("com.example.a"));
    assert_eq!(focus.as_deref(), Some("com.example.a"), "the app takes the keys at once");

    // The software renderer shows a quarter of the app over its black clear at the tile.
    let t0 = 2_000 * MS;
    let first = tick(&server, t0);
    let centre = first.pixel((TILE.0 + TILE.2 / 2) as u32, (TILE.1 + TILE.3 / 2) as u32);
    let (h, a) = (rgb(HOME), rgb(APP_A));
    let want: Vec<u8> = (0..3).map(|i| (a[i] as f32 * 0.25 + h[i] as f32 * 0.75) as u8).collect();
    assert!(near(centre, [want[0], want[1], want[2]], 4), "a quarter of the app at the tile: {centre:?} want {want:?}");
    assert!(near(first.pixel(1700, 200), h, 2), "the clear remains outside the card");
    assert!(state(&server, |s| s.scene.transitions.active() && s.scene.transitions.holds_home()));

    // Halfway the card is opaque at its centre; Home remains planned beneath it.
    let mid = tick(&server, t0 + 110 * MS);
    assert!(near(mid.pixel(960, 540), a, 2), "the card at the middle of the screen: {:?}", mid.pixel(960, 540));
    assert!(state(&server, |s| s.scene.transitions.holds_home()), "native Home remains under the card");

    // At 220 ms it is the screen, Home gone: the plan is the app alone, drawn as any window (the plane could scan it out directly).
    let end = tick(&server, t0 + 221 * MS);
    assert!(near(end.pixel(3, 3), a, 1) && near(end.pixel(1916, 1076), a, 1));
    let (active, holds, sole) =
        state(&server, |s| (s.scene.transitions.active(), s.scene.transitions.holds_home(), s.scene.last_plan().sole_fullscreen().is_some()));
    assert!(!active && !holds, "the transition is over");
    assert!(sole, "and the app is drawn plainly again: sole full-screen window, no pose, no ghost");
}

#[test]
fn the_switch_off_is_the_old_cut() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, false);
    let (_a, _buf) = app(&mut rig, "A", "com.example.a", APP_A);
    let first = tick(&server, 2_000 * MS);
    assert!(near(first.pixel(960, 540), rgb(APP_A), 1) && near(first.pixel(5, 5), rgb(APP_A), 1), "the app whole in its first frame");
    assert!(state(&server, |s| !s.scene.transitions.active() && s.scene.transitions.stats.launches == 0));
}

#[test]
fn home_takes_the_card_back_into_its_tile_without_giving_the_ghost_pointer_input() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    let (_a, _buf) = app(&mut rig, "A", "com.example.a", APP_A);
    tick(&server, 2_000 * MS);
    let settled = tick(&server, 2_300 * MS);
    assert!(near(settled.pixel(960, 540), rgb(APP_A), 1));

    // Home is pressed: SAM's Home card comes to the front again.
    rig.stub.webos.set_state(3);
    rig.client.roundtrip();
    let t0 = 3_000 * MS;
    let first = tick(&server, t0);
    assert!(near(first.pixel(960, 540), rgb(APP_A), 1), "the first frame is as it was: the card whole");
    let (front_is_home, focus, hit) = state(&server, |s| {
        (
            s.scene.last_plan().items.iter().any(|i| i.ghost),
            s.scene.foreground(&s.surfaces).map(|w| w.app_id().to_string()),
            s.scene.surface_at(960, 540).and_then(|(sid, _)| s.scene.window_of(sid)).map(|w| s.scene.window(w).unwrap().app_id().to_string()),
        )
    });
    assert!(front_is_home, "the card is a ghost while it goes");
    assert_eq!(focus.as_deref(), Some(HOME_STUB));
    assert_ne!(hit.as_deref(), Some("com.example.a"), "the outgoing card does not receive pointer input");
    // Home is back in the shell's word at once (it resumes drawing while the card goes)
    assert!(state(&server, |s| s.shell_home_drawn()));

    // The card shrinks toward its tile while the native Home pose brightens.
    let early = tick(&server, t0 + 100 * MS);
    assert!(near(early.pixel(960, 540), rgb(APP_A), 20), "still the card at the middle: {:?}", early.pixel(960, 540));
    let shade = state(&server, move |s| s.scene.transitions.pose(lumatv_compositor::scene::HOME_RUST, t0 + 100 * MS).unwrap().shade);
    assert!(shade > 0.0 && shade < 1.0, "Home is brightening: {shade}");
    let mid = tick(&server, t0 + 150 * MS);
    assert!(!near(mid.pixel(960, 540), rgb(APP_A), 40), "the card has left the middle: {:?}", mid.pixel(960, 540));
    // The software renderer shows its clear once the card has gone.
    let end = tick(&server, t0 + 181 * MS);
    assert!(near(end.pixel(960, 540), rgb(HOME), 1) && near(end.pixel(5, 5), rgb(HOME), 1));
    assert!(state(&server, |s| !s.scene.transitions.active()));
}

#[test]
fn a_second_press_takes_the_picture_where_it_is() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    let (a, _buf) = app(&mut rig, "A", "com.example.a", APP_A);
    tick(&server, 2_000 * MS);
    tick(&server, 2_300 * MS);
    rig.stub.webos.set_state(3);
    rig.client.roundtrip();
    let t0 = 3_000 * MS;
    tick(&server, t0);
    tick(&server, t0 + 60 * MS);
    let window = state(&server, |s| s.scene.windows().find(|w| w.app_id() == "com.example.a").map(|w| w.id).unwrap());
    let visual = state(&server, move |s| s.scene.take_visual(window)).expect("a leave is on its way");
    assert!(visual.rect.w > 120 && visual.rect.w < 1930 && visual.alpha > 0.0, "{visual:?}");
    // the card is the switcher's now: the next frames draw no ghost of it
    tick(&server, t0 + 77 * MS);
    assert!(state(&server, move |s| !s.scene.last_plan().items.iter().any(|i| i.ghost && i.window == window)));
    drop(a);
}

#[test]
fn an_app_replaces_another_the_new_one_fading_over_the_old_which_darkens() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    let (_a, _ba) = app(&mut rig, "A", "com.example.a", APP_A);
    tick(&server, 2_000 * MS);
    tick(&server, 2_300 * MS);
    let (_b, _bb) = app(&mut rig, "B", "com.example.b", APP_B);
    let t0 = 3_000 * MS;
    let first = tick(&server, t0);
    // B at a quarter over A: the corner outside B's scaled card is A, undimmed
    let (a, b) = (rgb(APP_A), rgb(APP_B));
    let centre = first.pixel(960, 540);
    let want: Vec<u8> = (0..3).map(|i| (b[i] as f32 * 0.25 + a[i] as f32 * 0.75) as u8).collect();
    assert!(near(centre, [want[0], want[1], want[2]], 4), "B at a quarter over A: {centre:?} want {want:?}");
    let mid = tick(&server, t0 + 150 * MS);
    assert!(near(mid.pixel(960, 540), b, 2));
    let corner = mid.pixel(2, 2);
    assert!(corner[0] < a[0] && near(corner, [corner[0], corner[1], corner[2]], 0) && corner[3] == 255, "the old card darkens at the edge: {corner:?}");
    let end = tick(&server, t0 + 221 * MS);
    assert!(near(end.pixel(2, 2), b, 1) && near(end.pixel(1917, 1077), b, 1));
    assert_eq!(state(&server, |s| (s.scene.transitions.stats.launches, s.scene.transitions.stats.switches)), (1, 1));
}

#[test]
fn a_card_on_the_video_plane_arrives_on_a_veil_and_the_plane_is_never_shown_black() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    // an input: a transparent card with a hole over the whole screen, its video element exported before the first frame
    let win = rig.client.card("_WEBOS_WINDOW_TYPE_CARD", "Hdmi");
    win.webos.set_property("appId".into(), "com.webos.app.hdmi2".into());
    rig.client.roundtrip();
    let sid = state(&server, |s| s.scene.windows().find(|w| w.app_id() == "com.webos.app.hdmi2").map(|w| w.surface).unwrap());
    state(&server, move |s| s.scene.set_video_expected(sid, true));
    let clear = rig.client.shm_buffer(1920, 1080, 0);
    rig.client.present(&win.surface, &clear);
    rig.client.roundtrip();
    let t0 = 2_000 * MS;
    let veil = [0x19, 0x17, 0x24];
    // the card is on screen in its first frame, and what is in it is the veil over Home, not the plane
    let first = tick(&server, t0);
    let c = first.pixel(960, 540);
    assert!(c[3] == 255, "no hole in the first frame: {c:?}");
    // arrived (220 ms): the hole comes (the compositor's placement) at 100 ms; the veil is held opaque
    state(&server, move |s| s.scene.holes.push(Hole { surface: sid, rect: lumatv_compositor::Rect::new(0, 0, 1920, 1080) }));
    tick(&server, t0 + 100 * MS);
    assert!(!state(&server, move |s| {
        let w = s.scene.window_of(sid).unwrap();
        s.scene.video_hole_visible(w, sid, lumatv_compositor::Rect::new(0, 0, 1920, 1080))
    }), "the posed veil does not clear video to the screen");
    let held = tick(&server, t0 + 300 * MS);
    assert!(near(held.pixel(960, 540), veil, 1) && near(held.pixel(10, 10), veil, 1), "veil held: {:?}", held.pixel(960, 540));
    // the video has had its 150 ms since the hole came (100 + 150 = 250, and the card has arrived): the frame at 300 ms is where the fade starts (at
    // full opacity), over 180 ms
    let half = tick(&server, t0 + 300 * MS + 90 * MS);
    let h = half.pixel(960, 540);
    assert!((h[3] as i32 - 128).abs() <= 3, "half through the dissolve: {h:?}");
    assert!(state(&server, move |s| {
        let w = s.scene.window_of(sid).unwrap();
        s.scene.video_hole_visible(w, sid, lumatv_compositor::Rect::new(0, 0, 1920, 1080))
    }), "the partially transparent veil reveals the planned hole");
    assert!(h[0] <= 0x19 / 2 + 2, "premultiplied veil at half");
    let gone = tick(&server, t0 + 300 * MS + 181 * MS);
    assert_eq!(gone.pixel(960, 540), [0, 0, 0, 0], "the plane shows through: a hole, transparent for the display engine");
    assert!(state(&server, move |s| {
        let w = s.scene.window_of(sid).unwrap();
        s.scene.video_hole_visible(w, sid, lumatv_compositor::Rect::new(0, 0, 1920, 1080))
    }), "the uncovered hole remains visible");
    assert!(state(&server, |s| !s.scene.transitions.active()));
}

#[test]
fn foreground_video_hole_rejects_an_offscreen_transition_pose() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    state(&server, |s| {
        s.set_launch_origin(&json!({"appId": "com.example.video", "tile": {"x": 3000, "y": 0, "width": 120, "height": 120}}));
    });
    let win = rig.client.card("_WEBOS_WINDOW_TYPE_CARD", "Video");
    win.webos.set_property("appId".into(), "com.example.video".into());
    rig.client.roundtrip();
    let sid = state(&server, |s| s.scene.windows().find(|w| w.app_id() == "com.example.video").unwrap().surface);
    let hole = lumatv_compositor::Rect::new(0, 0, 1920, 1080);
    state(&server, move |s| {
        s.scene.set_video_expected(sid, true);
        s.scene.holes.push(Hole { surface: sid, rect: hole });
    });
    let clear = rig.client.shm_buffer(1920, 1080, 0);
    rig.client.present(&win.surface, &clear);
    rig.client.roundtrip();
    tick(&server, 2_000 * MS);
    state(&server, move |s| {
        let w = s.scene.window_of(sid).unwrap();
        let pose = s.scene.transitions.pose(w, 2_000 * MS).expect("the card is posed");
        assert!(pose.bounds().intersect(&lumatv_compositor::Rect::new(0, 0, 1920, 1080)).is_empty());
        assert!(!s.scene.last_plan().items.iter().any(|item| item.surface == sid));
        assert!(!s.scene.video_hole_visible(w, sid, hole));
    });
    tick(&server, 2_300 * MS);
    tick(&server, 2_500 * MS);
    tick(&server, 2_800 * MS);
    tick(&server, 3_000 * MS);
    assert!(state(&server, move |s| {
        let w = s.scene.window_of(sid).unwrap();
        s.scene.video_hole_visible(w, sid, hole)
    }), "the same hole is visible after the card arrives and its veil clears");
}

#[test]
fn reduced_motion_is_one_fade_with_no_travel() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    state(&server, |s| {
        s.motion.reduced = true;
        s.scene.transitions.settings.reduced = true;
    });
    let (_a, _buf) = app(&mut rig, "A", "com.example.a", APP_A);
    let t0 = 2_000 * MS;
    let first = tick(&server, t0);
    assert!(near(first.pixel(960, 540), rgb(HOME), 1), "alpha 0 at the first frame");
    let mid = tick(&server, t0 + 60 * MS);
    let (h, a) = (rgb(HOME), rgb(APP_A));
    let want: Vec<u8> = (0..3).map(|i| ((a[i] as u32 + h[i] as u32) / 2) as u8).collect();
    assert!(near(mid.pixel(5, 5), [want[0], want[1], want[2]], 3), "the whole screen half through, corners too: {:?}", mid.pixel(5, 5));
    assert!(near(mid.pixel(960, 540), [want[0], want[1], want[2]], 3));
    let end = tick(&server, t0 + 121 * MS);
    assert!(near(end.pixel(5, 5), a, 1));
}

#[test]
fn turning_the_switch_off_in_the_middle_ends_it_at_once() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    let (_a, _buf) = app(&mut rig, "A", "com.example.a", APP_A);
    tick(&server, 2_000 * MS);
    assert!(state(&server, |s| s.scene.transitions.active()));
    state(&server, |s| {
        s.motion.forced = Some(false);
        s.apply_motion();
    });
    let img = tick(&server, 2_050 * MS);
    assert!(near(img.pixel(5, 5), rgb(APP_A), 1), "the app whole");
    assert!(state(&server, |s| !s.scene.transitions.active() && !s.shell_home_drawn()));
}

#[test]
fn what_home_is_told_follows_the_move_not_the_cut() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    let (_a, _buf) = app(&mut rig, "A", "com.example.a", APP_A);
    state(&server, |s| {
        s.tick_at(2_000 * MS);
        s.shell_sync_topics();
    });
    let during = state(&server, |s| s.shell_home_drawn());
    assert!(during, "Home is still drawn under the card that grows");
    state(&server, |s| {
        s.tick_at(2_230 * MS);
        s.shell_sync_topics();
    });
    assert!(state(&server, |s| !s.shell_home_drawn()), "and not once it is whole");
    std::thread::sleep(Duration::from_millis(1));
}

#[test]
fn the_switch_and_the_reports_go_over_the_control_socket_and_the_luna_method() {
    let mut server = Server::start();
    let _rig = rig(&mut server, false);
    let r = state(&server, |s| s.transitions_report());
    assert_eq!(r["enabled"], false);
    let ok = state(&server, |s| s.set_launch_origin(&json!({"appId": "com.x", "tile": {"x": 1, "y": 2, "width": 3, "height": 4, "radius": 5}})));
    assert_eq!(ok["returnValue"], true);
    let bad = state(&server, |s| s.set_launch_origin(&json!({"tile": {"x": 1}})));
    assert_eq!(bad["returnValue"], false);
    let r = state(&server, |s| {
        s.motion.forced = Some(true);
        s.apply_motion();
        s.transitions_report()
    });
    assert_eq!(r["enabled"], true);
    assert_eq!(r["forced"], true);
    assert!(r["stats"]["launches"].is_number());
}

/// The GL renderer on Mesa's llvmpipe draws a transition as the software renderer does: the same scene, the same clock, a picture that has a
/// gradient in it so that a flipped or shifted quad shows. Home dimmed behind a card that grows out of its tile, the card's rounded mask included.
#[test]
fn the_gl_renderer_draws_a_launch_as_the_software_renderer_does() {
    const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";
    if !std::path::Path::new(MESA).exists() {
        eprintln!("no Mesa libEGL on this host; skipping");
        return;
    }
    // SAFETY: set before the server thread (the only EGL user of this test) starts.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let instants = [0u64, 40, 90, 150, 219];
    let run = |gl: bool| -> (Vec<Image>, &'static str, u64) {
        let mut server = if gl {
            Server::start_with(Box::new(|| {
                use lumatv_compositor::render::egl::Platform;
                use lumatv_compositor::render::gl::{BufferPath, Gl};
                Box::new(Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).expect("Mesa GL renderer"))
            }))
        } else {
            Server::start()
        };
        let mut rig = rig(&mut server, true);
        let win = rig.client.card("_WEBOS_WINDOW_TYPE_CARD", "A");
        win.webos.set_property("appId".into(), "com.example.a".into());
        // red grows to the right, green downwards: where the picture is, and which way up
        let buf = rig.client.shm_pattern(1920, 1080, |x, y| 0xff00_0040 | (((x * 255 / 1919) as u32) << 16) | (((y * 255 / 1079) as u32) << 8));
        let whole = rig.client.region(0, 0, 1920, 1080);
        win.surface.set_opaque_region(Some(&whole));
        rig.client.present(&win.surface, &buf);
        rig.client.roundtrip();
        let out = instants.iter().map(|ms| tick(&server, 2_000 * MS + ms * MS)).collect();
        let (name, errors) = state(&server, |s| (s.output.name(), s.stats.gl_errors));
        drop(buf);
        (out, name, errors)
    };
    let ((soft, soft_name, _), (gl, gl_name, gl_errors)) = (run(false), run(true));
    assert_eq!((soft_name, gl_name, gl_errors), ("software", "gl", 0), "the two renderers, and no GL error from the pose program");
    let mut worst = 0u8;
    let mut off = 0;
    let mut total = 0;
    for (n, (a, b)) in soft.iter().zip(gl.iter()).enumerate() {
        for y in (3..1080).step_by(23) {
            for x in (3..1920).step_by(29) {
                let (p, q) = (a.pixel(x, y), b.pixel(x, y));
                let d = p.iter().zip(q.iter()).map(|(m, n)| m.abs_diff(*n)).max().unwrap();
                worst = worst.max(d);
                total += 1;
                if d > 6 {
                    off += 1;
                    if off < 6 {
                        eprintln!("instant {n} ({x},{y}): software {p:?} gl {q:?}");
                    }
                }
            }
        }
    }
    assert!(off * 200 <= total, "{off} of {total} samples differ by more than 6 (worst {worst})");
    // and the frames are not the same picture: the transition is on the screen
    assert_ne!(soft[0].pixel(960, 540), soft[4].pixel(960, 540));
}

/// A free of a driver import stalls the loop for milliseconds (10 ms for a Netflix buffer): while a card moves, the textures of the buffers clients
/// destroyed wait, and they are freed in the first frame after the move.
#[test]
fn the_frees_of_destroyed_buffers_wait_for_the_move_to_end() {
    const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";
    if !std::path::Path::new(MESA).exists() {
        eprintln!("no Mesa libEGL on this host; skipping");
        return;
    }
    // SAFETY: set before the server thread (the only EGL user of this test) starts.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let mut server = Server::start_with(Box::new(|| {
        use lumatv_compositor::render::egl::Platform;
        use lumatv_compositor::render::gl::{BufferPath, Gl};
        Box::new(Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).expect("Mesa GL renderer"))
    }));
    let mut rig = rig(&mut server, true);
    let (a, b1) = app(&mut rig, "A", "com.example.a", APP_A);
    let t0 = 2_000 * MS;
    tick(&server, t0);
    tick(&server, t0 + 20 * MS);
    // the app swaps its buffer in the middle of its own launch and the client destroys the old one, which nothing shows any more
    let b2 = rig.client.shm_buffer(1920, 1080, APP_B);
    rig.client.present(&a.surface, &b2);
    b1.destroy();
    rig.client.roundtrip();
    assert_eq!(state(&server, |s| s.output.retired()), 1, "the old buffer's texture is retired");
    tick(&server, t0 + 40 * MS);
    tick(&server, t0 + 80 * MS);
    assert!(state(&server, move |s| s.scene.transitions.animating(t0 + 80 * MS)), "still moving");
    assert_eq!(state(&server, |s| s.output.retired()), 1, "it is not freed while the card moves");
    // the move ends at 220 ms; the first frame after it, drawn, frees what waited (the frame before it had the flag of the move)
    tick(&server, t0 + 230 * MS);
    state(&server, |s| s.scene.redraw_all());
    tick(&server, t0 + 250 * MS);
    state(&server, |s| s.scene.redraw_all());
    tick(&server, t0 + 270 * MS);
    assert_eq!(state(&server, |s| s.output.retired()), 0, "freed once the move is over");
    drop(b2);
}

/// The same for a card on the video plane: the veil (a flat fill through the card's mask, held, then fading over a cleared hole) and the hole
/// the display engine blends the plane under, in both renderers.
#[test]
fn the_gl_renderer_draws_a_veil_as_the_software_renderer_does() {
    const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";
    if !std::path::Path::new(MESA).exists() {
        eprintln!("no Mesa libEGL on this host; skipping");
        return;
    }
    // SAFETY: set before the server thread (the only EGL user of this test) starts.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let instants = [0u64, 60, 130, 300, 390, 500];
    let run = |gl: bool| -> Vec<Image> {
        let mut server = if gl {
            Server::start_with(Box::new(|| {
                use lumatv_compositor::render::egl::Platform;
                use lumatv_compositor::render::gl::{BufferPath, Gl};
                Box::new(Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).expect("Mesa GL renderer"))
            }))
        } else {
            Server::start()
        };
        let mut rig = rig(&mut server, true);
        state(&server, |s| {
            s.set_launch_origin(&json!({"appId": "com.webos.app.hdmi2", "tile": {"x": TILE.0, "y": TILE.1, "width": TILE.2, "height": TILE.3, "radius": 22}}));
        });
        let win = rig.client.card("_WEBOS_WINDOW_TYPE_CARD", "Hdmi");
        win.webos.set_property("appId".into(), "com.webos.app.hdmi2".into());
        rig.client.roundtrip();
        let sid = state(&server, |s| s.scene.windows().find(|w| w.app_id() == "com.webos.app.hdmi2").map(|w| w.surface).unwrap());
        state(&server, move |s| {
            s.scene.set_video_expected(sid, true);
            s.scene.holes.push(Hole { surface: sid, rect: lumatv_compositor::Rect::new(0, 0, 1920, 1080) });
        });
        // a banner in the card's corner, so that something of the card itself is drawn over the veil
        let buf = rig.client.shm_pattern(1920, 1080, |x, y| if x < 500 && y < 120 { 0xff30_5080 } else { 0 });
        rig.client.present(&win.surface, &buf);
        rig.client.roundtrip();
        let out = instants.iter().map(|ms| tick(&server, 2_000 * MS + ms * MS)).collect();
        drop(buf);
        out
    };
    let (soft, gl) = (run(false), run(true));
    let mut off = 0;
    let mut total = 0;
    for (n, (a, b)) in soft.iter().zip(gl.iter()).enumerate() {
        for y in (3..1080).step_by(23) {
            for x in (3..1920).step_by(29) {
                let (p, q) = (a.pixel(x, y), b.pixel(x, y));
                let d = p.iter().zip(q.iter()).map(|(m, n)| m.abs_diff(*n)).max().unwrap();
                total += 1;
                if d > 6 {
                    off += 1;
                    if off < 6 {
                        eprintln!("instant {n} ({x},{y}): software {p:?} gl {q:?}");
                    }
                }
            }
        }
    }
    assert!(off * 200 <= total, "{off} of {total} samples differ by more than 6");
    // held at 300 ms: the veil; at 500 ms: a hole the plane shows through, the banner still drawn
    assert_eq!(soft[3].pixel(960, 540)[3], 255);
    assert_eq!(gl[5].pixel(960, 540), [0, 0, 0, 0]);
    assert_eq!(soft[5].pixel(960, 540), [0, 0, 0, 0]);
}

#[test]
fn an_app_that_exits_is_still_taken_away_by_its_transition() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    let (a, _buf) = app(&mut rig, "A", "com.example.a", APP_A);
    tick(&server, 2_000 * MS);
    let settled = tick(&server, 2_300 * MS);
    assert!(near(settled.pixel(960, 540), rgb(APP_A), 1));
    // Back at the app's root: the app destroys its window. Home is the front card again at once; the last picture of the app is what the move takes away.
    a.surface.destroy();
    rig.client.roundtrip();
    assert!(state(&server, |s| s.scene.windows().all(|w| w.app_id() != "com.example.a")), "the window is gone from the scene");
    let t0 = 3_000 * MS;
    let first = tick(&server, t0);
    assert!(near(first.pixel(960, 540), rgb(APP_A), 1), "its last picture is on the first frame: {:?}", first.pixel(960, 540));
    let mid = tick(&server, t0 + 150 * MS);
    assert!(!near(mid.pixel(960, 540), rgb(APP_A), 40), "and it is on its way to the tile");
    let end = tick(&server, t0 + 181 * MS);
    assert!(near(end.pixel(960, 540), rgb(HOME), 1) && near(end.pixel(5, 5), rgb(HOME), 1), "Home, whole");
    assert!(state(&server, |s| !s.scene.transitions.active()));
}

#[test]
fn a_full_screen_overlay_that_is_an_app_opens_and_closes_like_a_card() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    // LG's Settings: an overlay window the size of the screen, which SAM never names as the foreground app
    let win = rig.client.card("_WEBOS_WINDOW_TYPE_OVERLAY", "Settings");
    win.webos.set_property("appId".into(), "com.palm.app.settings".into());
    let buf = rig.client.shm_buffer(1920, 1080, APP_B);
    rig.client.present(&win.surface, &buf);
    rig.client.roundtrip();
    let t0 = 2_000 * MS;
    let first = tick(&server, t0);
    let (h, b) = (rgb(HOME), rgb(APP_B));
    let want: Vec<u8> = (0..3).map(|i| (b[i] as f32 * 0.25 + h[i] as f32 * 0.75) as u8).collect();
    // no tile for it: it grows from the middle of the screen (the whole picture, a little smaller), a quarter seen over Home
    assert!(near(first.pixel(960, 540), [want[0], want[1], want[2]], 4), "{:?} want {want:?}", first.pixel(960, 540));
    assert!(near(first.pixel(3, 3), h, 2), "the edge of the screen is Home's: the card starts at .94");
    let end = tick(&server, t0 + 221 * MS);
    assert!(near(end.pixel(3, 3), b, 1), "whole");
    assert_eq!(state(&server, |s| s.scene.transitions.stats.launches), 1);
    // Back: the app closes its window
    win.surface.destroy();
    rig.client.roundtrip();
    let t1 = 3_000 * MS;
    let gone = tick(&server, t1);
    assert!(near(gone.pixel(960, 540), b, 1), "its last picture first");
    let home = tick(&server, t1 + 181 * MS);
    assert!(near(home.pixel(960, 540), h, 1), "then Home");
    drop(buf);
}

#[test]
fn an_overlay_that_goes_reveals_the_card_under_it_which_does_not_arrive_again() {
    let mut server = Server::start();
    let mut rig = rig(&mut server, true);
    let (_a, _ba) = app(&mut rig, "A", "com.example.a", APP_A);
    tick(&server, 2_000 * MS);
    tick(&server, 2_300 * MS);
    let win = rig.client.card("_WEBOS_WINDOW_TYPE_OVERLAY", "Settings");
    win.webos.set_property("appId".into(), "com.palm.app.settings".into());
    let buf = rig.client.shm_buffer(1920, 1080, APP_B);
    rig.client.present(&win.surface, &buf);
    rig.client.roundtrip();
    tick(&server, 3_000 * MS);
    tick(&server, 3_300 * MS);
    win.surface.destroy();
    rig.client.roundtrip();
    let t0 = 4_000 * MS;
    tick(&server, t0);
    // the card under it was there all along: at no frame is it faded in again from nothing; at the end it is the screen, whole
    let mid = tick(&server, t0 + 150 * MS);
    let a = rgb(APP_A);
    assert!(mid.pixel(5, 5)[0] >= a[0] / 2, "the card is under the overlay's fade, not arriving: {:?}", mid.pixel(5, 5));
    let end = tick(&server, t0 + 181 * MS);
    assert!(near(end.pixel(960, 540), a, 1), "{:?}", end.pixel(960, 540));
    drop(buf);
}

/// A window destroyed at the front (an app that exits): the GL renderer keeps drawing its last picture while the move takes it away, and
/// lets the texture go with the move. (An shm buffer the client destroys goes with it: the driver's own buffers, the apps', stay imported.)
#[test]
fn the_gl_renderer_draws_an_exiting_apps_last_picture_and_lets_it_go() {
    const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";
    if !std::path::Path::new(MESA).exists() {
        eprintln!("no Mesa libEGL on this host; skipping");
        return;
    }
    // SAFETY: set before the server thread (the only EGL user of this test) starts.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let mut server = Server::start_with(Box::new(|| {
        use lumatv_compositor::render::egl::Platform;
        use lumatv_compositor::render::gl::{BufferPath, Gl};
        Box::new(Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).expect("Mesa GL renderer"))
    }));
    let mut rig = rig(&mut server, true);
    let (a, _buf) = app(&mut rig, "A", "com.example.a", APP_A);
    tick(&server, 2_000 * MS);
    tick(&server, 2_300 * MS);
    a.surface.destroy();
    rig.client.roundtrip();
    let t0 = 3_000 * MS;
    let first = tick(&server, t0);
    assert!(near(first.pixel(960, 540), rgb(APP_A), 2), "the last picture: {:?}", first.pixel(960, 540));
    let mid = tick(&server, t0 + 150 * MS);
    assert!(!near(mid.pixel(960, 540), rgb(APP_A), 40));
    let end = tick(&server, t0 + 181 * MS);
    assert!(near(end.pixel(960, 540), rgb(HOME), 2));
    let (skipped, errors) = state(&server, |s| (s.stats.skipped_items, s.stats.gl_errors));
    assert_eq!((skipped, errors), (0, 0));
}

// Software-rendered transition contact sheets; native engine layers are not present in these images.

/// An app's picture: a dark canvas, a hero banner and a row of posters, so that scale, mask and corner show.
fn app_picture(x: i32, y: i32, hue: u32) -> u32 {
    let banner = (96..1824).contains(&x) && (170..690).contains(&y);
    let poster = (0..5).any(|i| (96 + i * 340..96 + i * 340 + 316).contains(&x) && (740..960).contains(&y));
    let bar = (96..600).contains(&x) && (54..126).contains(&y);
    let (r, g, b) = match hue {
        0 => (0xe5u32, 0x4f, 0x4f),
        _ => (0x4f, 0xb0, 0x6a),
    };
    if bar {
        0xffff_ffff
    } else if banner {
        let t = (x - 96) as f32 / 1728.0;
        0xff00_0000 | (((r as f32 * (0.4 + 0.5 * t)) as u32) << 16) | (((g as f32 * (0.4 + 0.5 * t)) as u32) << 8) | (b as f32 * (0.4 + 0.5 * t)) as u32
    } else if poster {
        0xff00_0000 | ((r / 3) << 16) | ((g / 3) << 8) | (b / 3)
    } else {
        0xff0c_0b10
    }
}

/// The plane's stand-in picture under a hole: a landscape gradient.
fn plane_picture(x: u32, y: u32) -> [u8; 3] {
    let t = y as f32 / 1080.0;
    [(90.0 + 120.0 * (1.0 - t) + 20.0 * (x as f32 / 1920.0)) as u8, (110.0 + 80.0 * (1.0 - t)) as u8, (60.0 + 140.0 * t) as u8]
}

/// A contact sheet of `frames` (3 columns, each third size), the plane's picture blended under the pixels the picture leaves transparent.
fn sheet(frames: &[Image], plane: bool) -> Image {
    let (cols, w, h) = (3usize, 640usize, 360usize);
    let rows = frames.len().div_ceil(cols);
    let (sw, sh) = (cols * w + (cols + 1) * 16, rows * h + (rows + 1) * 16);
    let mut rgba = vec![24u8; sw * sh * 4];
    for px in rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&[0x19, 0x17, 0x24, 255]);
    }
    for (n, f) in frames.iter().enumerate() {
        let (ox, oy) = (16 + (n % cols) * (w + 16), 16 + (n / cols) * (h + 16));
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0u32; 4];
                for dy in 0..3 {
                    for dx in 0..3 {
                        let p = f.pixel((x * 3 + dx) as u32, (y * 3 + dy) as u32);
                        let under = if plane { plane_picture((x * 3 + dx) as u32, (y * 3 + dy) as u32) } else { [0, 0, 0] };
                        let inv = 255 - p[3] as u32;
                        for c in 0..3 {
                            acc[c] += (p[c] as u32 + under[c] as u32 * inv / 255).min(255);
                        }
                    }
                }
                let o = ((oy + y) * sw + ox + x) * 4;
                rgba[o..o + 4].copy_from_slice(&[(acc[0] / 9) as u8, (acc[1] / 9) as u8, (acc[2] / 9) as u8, 255]);
            }
        }
    }
    Image { width: sw as u32, height: sh as u32, rgba }
}

/// `LUMATV_TRANSITIONS_RENDER_DIR=docs/research/transitions cargo test --test transitions renders_the_transitions_for_the_docs`
#[test]
fn renders_the_transitions_for_the_docs() {
    let Ok(dir) = std::env::var("LUMATV_TRANSITIONS_RENDER_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, img: &Image| lumatv_compositor::capture::write_png(&dir.join(name), img).unwrap();

    // launch, then Home pressed, then another app over it: one run, the frames picked out of it
    let mut server = Server::start();
    let mut r = rig(&mut server, true);
    state(&server, |s| {
        s.tick_at(1_500 * MS);
    });
    let a = r.client.card("_WEBOS_WINDOW_TYPE_CARD", "A");
    a.webos.set_property("appId".into(), "com.example.a".into());
    let pic_a = r.client.shm_pattern(1920, 1080, |x, y| app_picture(x, y, 0));
    let whole = r.client.region(0, 0, 1920, 1080);
    a.surface.set_opaque_region(Some(&whole));
    r.client.present(&a.surface, &pic_a);
    r.client.roundtrip();
    let launch: Vec<Image> = [0u64, 44, 88, 132, 176, 220].iter().map(|ms| tick(&server, 2_000 * MS + ms * MS)).collect();
    write("launch.png", &sheet(&launch, false));
    tick(&server, 2_400 * MS);

    r.stub.webos.set_state(3);
    r.client.roundtrip();
    let leave: Vec<Image> = [0u64, 36, 72, 108, 144, 180].iter().map(|ms| tick(&server, 3_000 * MS + ms * MS)).collect();
    write("leave.png", &sheet(&leave, false));
    tick(&server, 3_400 * MS);

    // the second app over the first (Home brought the first back, so launch the first again, then the second)
    a.webos.set_state(3);
    r.client.roundtrip();
    tick(&server, 4_000 * MS);
    tick(&server, 4_300 * MS);
    let b = r.client.card("_WEBOS_WINDOW_TYPE_CARD", "B");
    b.webos.set_property("appId".into(), "com.example.b".into());
    let pic_b = r.client.shm_pattern(1920, 1080, |x, y| app_picture(x, y, 1));
    b.surface.set_opaque_region(Some(&whole));
    r.client.present(&b.surface, &pic_b);
    r.client.roundtrip();
    let switch: Vec<Image> = [0u64, 44, 88, 132, 176, 220].iter().map(|ms| tick(&server, 5_000 * MS + ms * MS)).collect();
    write("switch.png", &sheet(&switch, false));

    // a card on the video plane: HDMI arrives on a veil and the plane is dissolved in; then Home dissolves over it
    let mut server = Server::start();
    let mut r = rig(&mut server, true);
    state(&server, |s| {
        s.set_launch_origin(
            &json!({"appId": "com.webos.app.hdmi2", "tile": {"x": TILE.0 + 144 * 4, "y": TILE.1 + 144, "width": 120, "height": 120, "radius": 22}}),
        );
        s.tick_at(1_500 * MS);
    });
    let h = r.client.card("_WEBOS_WINDOW_TYPE_CARD", "Hdmi");
    h.webos.set_property("appId".into(), "com.webos.app.hdmi2".into());
    r.client.roundtrip();
    let sid = state(&server, |s| s.scene.windows().find(|w| w.app_id() == "com.webos.app.hdmi2").map(|w| w.surface).unwrap());
    state(&server, move |s| {
        s.scene.set_video_expected(sid, true);
        s.scene.holes.push(Hole { surface: sid, rect: lumatv_compositor::Rect::new(0, 0, 1920, 1080) });
    });
    let banner = r.client.shm_pattern(1920, 1080, |x, y| if (96..560).contains(&x) && (54..130).contains(&y) { 0xff28_3a58 } else { 0 });
    r.client.present(&h.surface, &banner);
    r.client.roundtrip();
    let at = [0u64, 90, 180, 270, 360, 450];
    let veil: Vec<Image> = at.iter().map(|ms| tick(&server, 2_000 * MS + ms * MS)).collect();
    write("video-launch.png", &sheet(&veil, true));
    tick(&server, 3_000 * MS);
    r.stub.webos.set_state(3);
    r.client.roundtrip();
    let dissolve: Vec<Image> = [0u64, 36, 72, 108, 144, 180].iter().map(|ms| tick(&server, 4_000 * MS + ms * MS)).collect();
    write("video-leave.png", &sheet(&dissolve, true));
    drop((pic_a, pic_b, banner));
}
