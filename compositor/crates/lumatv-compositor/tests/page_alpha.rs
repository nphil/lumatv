//! The scan-out pages carry alpha (AR24, the default) so that a video hole is transparent for the display engine instead of opaque black, on
//! Mesa's software GL over pages in the context's own memory under the real `State::tick`: the hole is alpha 0 and nothing else is, a translucent
//! layer over opaque pixels stays opaque (no video shows through the UI) and over the hole stays translucent (the video shows through it), a full
//! screen video app with nothing over it draws one frame and then nothing, and a layer that appears over it draws only its own rectangle and
//! hands the screen back to nothing when it goes.
//! A video hole clears every layer under its window inside its rectangle (another app's card and overlay included) and nothing outside it.

mod harness;

use harness::{Client, Server};
use lumatv_compositor::Rect;
use lumatv_compositor::afbc::PageKind;
use lumatv_compositor::control;
use lumatv_compositor::kms::frames::Frames;
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl, TargetId};
use lumatv_compositor::scene::{Hole, Plan};
use lumatv_compositor::surface::Surfaces;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use wayland_server::protocol::wl_buffer::WlBuffer;

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";

/// The KMS output without the device: the GL renderer, three RGBA pages of its own and the ring's frame logic. Counts the frames it composes (the
/// page flips the panel would see) and the pixels each composed frame owed.
struct Pages {
    gl: Gl,
    frames: Frames,
    pages: Vec<TargetId>,
    flips: Arc<AtomicU64>,
    owed_px: Arc<AtomicU64>,
    kind: PageKind,
}

impl Output for Pages {
    fn name(&self) -> &'static str {
        "page-alpha-test"
    }

    fn size(&self) -> (u32, u32) {
        self.gl.size()
    }

    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        self.gl.query_buffer(buffer)
    }

    fn prepare_buffer(&mut self, buffer: &WlBuffer) -> bool {
        self.gl.prepare(buffer)
    }

    fn warming(&mut self) -> bool {
        self.gl.warming()
    }

    fn forget_buffer(&mut self, buffer: &WlBuffer) {
        self.gl.forget_buffer(buffer)
    }

    fn ui_needs_frame(&mut self) -> io::Result<bool> {
        Ok(self.gl.ui_needs_frame()? || self.gl.standin_due())
    }

    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats> {
        let (_, stats) = self.frames.draw(&mut self.gl, plan, surfaces, None)?;
        // the real output commits a flip for every frame it composes and none for a tick that drew nothing
        self.flips.fetch_add(1, Ordering::SeqCst);
        self.owed_px.fetch_add(plan.tiles.as_ref().map_or(0, |t| t.pixels()), Ordering::SeqCst);
        Ok(stats)
    }

    fn page_kind(&self) -> Option<PageKind> {
        Some(self.kind)
    }

    fn present(&mut self) -> io::Result<Presented> {
        self.gl.present()
    }

    fn capture(&mut self) -> io::Result<Image> {
        self.gl.set_target(self.pages[self.frames.last_page()]);
        self.gl.capture()
    }
}

fn at(img: &Image, x: i32, y: i32) -> [u8; 4] {
    img.pixel(x as u32, y as u32)
}

fn rig() -> Option<(Server, Arc<AtomicU64>, Arc<AtomicU64>)> {
    rig_of(PageKind::Argb)
}

fn rig_of(kind: PageKind) -> Option<(Server, Arc<AtomicU64>, Arc<AtomicU64>)> {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return None;
    }
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let (flips, owed) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let (f, o) = (flips.clone(), owed.clone());
    let server = Server::start_with(Box::new(move || {
        let mut gl = Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap();
        let pages: Vec<TargetId> = (0..3).map(|_| gl.add_target().unwrap()).collect();
        let frames = Frames::new(pages.clone(), 1920, 1080);
        Box::new(Pages { gl, frames, pages, flips: f, owed_px: o, kind })
    }));
    server.ticking.store(false, Ordering::SeqCst);
    Some((server, flips, owed))
}

/// `ctl status` names the page format and says whether video holes are opaque: AR24 pages are not, the XR24 fallback's are.
#[test]
fn ctl_status_says_whether_video_holes_are_opaque() {
    for (kind, name, opaque) in [(PageKind::Argb, "AR24", false), (PageKind::Xrgb, "XR24", true)] {
        let Some((server, _, _)) = rig_of(kind) else { return };
        let status: serde_json::Value = serde_json::from_str(&server.eval(|s| control::handle(s, "status").text())).unwrap();
        assert_eq!((status["pageFormat"].as_str(), status["videoHolesOpaque"].as_bool()), (Some(name), Some(opaque)), "{status}");
    }
}

const FULL: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };
const HOLE: Rect = Rect { x: 600, y: 300, w: 720, h: 400 };
/// A strip of half-transparent white (premultiplied) across the card's lower part, 1000 px wide: it crosses the hole's left edge.
const STRIP: Rect = Rect { x: 200, y: 640, w: 800, h: 100 };
const RED: u32 = 0xffff_0000;
const HALF_WHITE: u32 = 0x8080_8080;

fn card_with_hole(server: &mut Server) -> (Client, harness::Window) {
    let mut client = Client::connect(server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Player");
    let buf = client.shm_pattern(1920, 1080, |x, y| {
        if HOLE.contains(x, y) && !STRIP.contains(x, y) {
            0
        } else if STRIP.contains(x, y) {
            HALF_WHITE
        } else {
            RED
        }
    });
    client.present(&win.surface, &buf);
    client.roundtrip();
    let sid = server.eval(|s| s.scene.windows().find(|w| w.title == "Player").unwrap().surface);
    server.eval(move |s| s.scene.holes.push(Hole { surface: sid, rect: HOLE }));
    std::mem::forget(buf);
    (client, win)
}

#[test]
fn the_hole_is_alpha_0_and_everything_else_is_opaque_whatever_is_drawn_over_it() {
    let Some((mut server, _, _)) = rig() else { return };
    let (_client, _win) = card_with_hole(&mut server);
    server.eval(|s| s.scene.redraw_all());
    let img = server.eval(harness::drawn);
    let mut translucent_over_hole = 0;
    for y in (0..1080).step_by(3) {
        for x in (0..1920).step_by(3) {
            let p = at(&img, x, y);
            let in_hole = HOLE.contains(x, y);
            if in_hole && !STRIP.contains(x, y) {
                assert_eq!(p, [0, 0, 0, 0], "({x},{y}): the hole is transparent black, for the display engine's video");
            } else if in_hole {
                // half-transparent white over the cleared hole: the video shows through half of it
                assert!(p[3].abs_diff(0x80) <= 1 && p[0].abs_diff(0x80) <= 1, "({x},{y}): the strip over the hole is translucent: {p:?}");
                translucent_over_hole += 1;
            } else if STRIP.contains(x, y) {
                // the same strip over the page's opaque black: 50 % white, and nothing shows through it
                assert_eq!(p[3], 255, "({x},{y}): a translucent layer over opaque pixels leaves them opaque: {p:?}");
                assert!(p[0].abs_diff(0x80) <= 1, "({x},{y}): {p:?}");
            } else {
                assert_eq!(p, [255, 0, 0, 255], "({x},{y}): opaque card pixels");
            }
        }
    }
    assert!(translucent_over_hole > 100, "the strip crosses the hole");
}

/// An app that draws its own video into an XRGB8888 buffer (LG's browser, GeForce NOW's stream) has an alpha byte that means nothing: the page
/// takes the picture opaque, never as the hole its zero would be.
#[test]
fn an_xrgb_buffers_meaningless_alpha_byte_never_makes_a_hole() {
    let Some((mut server, _, _)) = rig() else { return };
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Stream");
    let buf = client.shm_pattern_in(1920, 1080, wayland_client::protocol::wl_shm::Format::Xrgb8888, |x, _| if x < 960 { 0x0030_60c0 } else { 0x00c0_6030 });
    client.present(&win.surface, &buf);
    client.roundtrip();
    let img = server.eval(harness::drawn);
    assert_eq!((at(&img, 100, 100), at(&img, 1800, 900)), ([0x30, 0x60, 0xc0, 255], [0xc0, 0x60, 0x30, 255]), "opaque, in its own colours");
    std::mem::forget(buf);
}

#[test]
fn a_full_screen_video_app_draws_one_frame_and_then_nothing_and_a_layer_over_it_draws_only_its_own_rectangle() {
    let Some((mut server, flips, owed)) = rig() else { return };
    let n = |a: &Arc<AtomicU64>| a.load(Ordering::SeqCst);
    let drawn_of = |server: &Server| server.eval(|s| s.stats.frames_drawn);
    // A video app: a transparent card over the whole screen, its hole the whole screen.
    let mut client = Client::connect(&mut server);
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "Player");
    let clear = client.shm_buffer(1920, 1080, 0);
    client.present(&win.surface, &clear);
    client.roundtrip();
    let sid = server.eval(|s| s.scene.windows().find(|w| w.title == "Player").unwrap().surface);
    server.eval(move |s| s.scene.holes.push(Hole { surface: sid, rect: FULL }));
    let img = server.eval(harness::drawn);
    assert_eq!((at(&img, 960, 540), at(&img, 3, 3)), ([0, 0, 0, 0], [0, 0, 0, 0]), "the whole page is the hole");
    let (drawn0, flips0) = (drawn_of(&server), n(&flips));
    assert!(drawn0 >= 1);
    let first_us = server.eval(|s| s.stats.render_us_last);
    eprintln!("measured: the video app's first frame (the whole page a hole): {first_us} us of CPU in the GL driver, {} px owed", n(&owed));

    // Nothing over the video, for two seconds of refreshes: no frame composed and no page flipped.
    let px0 = n(&owed);
    server.eval(|s| {
        for _ in 0..120 {
            s.tick();
        }
    });
    assert_eq!((drawn_of(&server), n(&flips), n(&owed) - px0), (drawn0, flips0, 0), "a video with nothing over it costs the compositor nothing");

    // A toast appears over the video: a window of its own, 320x80, half-transparent dark glass.
    let toast = client.card("_WEBOS_WINDOW_TYPE_TOAST", "Toast");
    let first = glass(&mut client, 0x80);
    client.present(&toast.surface, &first);
    client.roundtrip();
    let px0 = n(&owed);
    let img = server.eval(harness::drawn);
    let (bounds, rect) = server.eval(|s| (s.scene.last_plan().damage, s.scene.last_plan().items.last().unwrap().rect));
    let owed_px = n(&owed) - px0;
    assert_eq!(drawn_of(&server), drawn0 + 1, "one frame for the toast");
    eprintln!(
        "measured: the toast's first frame: {} px owed ({:.1} % of the page), {} us",
        owed_px,
        owed_px as f64 * 100.0 / (1920.0 * 1080.0),
        server.eval(|s| s.stats.render_us_last)
    );
    assert_eq!((rect.w, rect.h), (320, 80), "the toast's window");
    let pad = 32;
    assert!(bounds.w <= rect.w + 2 * pad && bounds.h <= rect.h + 2 * pad, "damage is the toast's: {bounds:?} for {rect:?}");
    assert!(owed_px <= ((rect.w + 2 * pad) * (rect.h + 2 * pad)) as u64, "the page owes the toast's tiles, not the screen: {owed_px} px");
    let inside = at(&img, rect.x + 10, rect.y + 10);
    assert!(inside[3].abs_diff(0x80) <= 1, "the glass over the video is translucent, the video shows through it: {inside:?}");
    let far = |x: i32, y: i32| !Rect::new(rect.x - 64, rect.y - 64, rect.w + 128, rect.h + 128).contains(x, y);
    for (x, y) in [(500, 500), (1900, 1070), (960, 20), (1000, 300)] {
        assert!(!far(x, y) || at(&img, x, y) == [0, 0, 0, 0], "the video is visible around it: ({x},{y})");
    }

    // A fade: 30 commits, one frame each, each owing only the toast's tiles.
    let (d0, px0) = (drawn_of(&server), n(&owed));
    for step in 0..30u32 {
        let buf = glass(&mut client, 0x80 - step * 4);
        client.present(&toast.surface, &buf);
        client.roundtrip();
        server.eval(|s| s.tick());
        std::mem::forget(buf);
    }
    assert_eq!(drawn_of(&server) - d0, 30, "a frame for each step of the fade");
    let per_frame = (n(&owed) - px0) / 30;
    eprintln!(
        "measured: a 30 frame fade over the video: {per_frame} px owed a frame ({:.1} % of the page, {:.2} MB written at 4 bytes a pixel), {} us in the last",
        per_frame as f64 * 100.0 / (1920.0 * 1080.0),
        per_frame as f64 * 4.0 / 1e6,
        server.eval(|s| s.stats.render_us_last)
    );
    assert!(per_frame <= ((rect.w + 2 * pad) * (rect.h + 2 * pad)) as u64, "each owes the toast's tiles only: {per_frame} px a frame");

    // It goes: one frame gives the rectangle back to the video, then nothing again.
    toast.surface.attach(None, 0, 0);
    toast.surface.commit();
    client.roundtrip();
    let img = server.eval(harness::drawn);
    assert_eq!(at(&img, rect.x + 10, rect.y + 10), [0, 0, 0, 0], "the hole is whole again where the toast was");
    let (drawn2, flips2) = (drawn_of(&server), n(&flips));
    server.eval(|s| {
        for _ in 0..120 {
            s.tick();
        }
    });
    assert_eq!((drawn_of(&server), n(&flips)), (drawn2, flips2), "idle again at once");
    std::mem::forget((first, clear));
}

/// The toast's buffer: 320x80 of dark glass, `a` its premultiplied alpha.
fn glass(ui: &mut Client, a: u32) -> wayland_client::protocol::wl_buffer::WlBuffer {
    ui.shm_pattern(320, 80, move |_, _| (a << 24) | ((a / 2) << 16) | ((a / 2) << 8) | (a / 2))
}

const GAME_RED: u32 = 0xffc0_2040;
const OTHER: Rect = Rect { x: 500, y: 250, w: 900, h: 500 };
const ORANGE: u32 = 0xffe0_8020;
const CONTROLS: Rect = Rect { x: 200, y: 960, w: 1520, h: 80 };
const DARK: u32 = 0xff30_3030;

/// LumaGame's page: a chequer of two greens, and the same in red once it has drawn on.
fn game_green(x: i32, y: i32) -> u32 {
    if (x / 60 + y / 60) % 2 == 0 { 0xff20_8040 } else { 0xff40_2080 }
}

fn game_drawn_on(x: i32, y: i32) -> u32 {
    if (x / 60 + y / 60) % 2 == 0 { GAME_RED } else { game_green(x, y) }
}

fn opaque(argb: u32) -> [u8; 4] {
    [(argb >> 16) as u8, (argb >> 8) as u8, argb as u8, 255]
}

/// What the page holds at (x, y): the frame's own controls over everything, the video's hole clear where it is, then what the layers under the
/// frame draw, the other app's overlay over the game's card.
fn page_has(game: fn(i32, i32) -> u32, hole: Option<Rect>, x: i32, y: i32) -> [u8; 4] {
    if CONTROLS.contains(x, y) {
        opaque(DARK)
    } else if hole.is_some_and(|h| h.contains(x, y)) {
        [0, 0, 0, 0]
    } else if OTHER.contains(x, y) {
        opaque(ORANGE)
    } else {
        opaque(game(x, y))
    }
}

/// LumaGame under the frame, which is an overlay window with a video hole: the hole clears what the layers under the frame's window drew
/// (the game's card, and another app's overlay between them), inside its rectangle for the card and for the full screen, as it grows and as it
/// shrinks, and when the game draws on under it; what is outside it is untouched, and a page nobody draws on costs nothing.
#[test]
fn a_video_hole_clears_every_layer_under_its_window_inside_its_rectangle_and_nothing_outside_it() {
    let Some((mut server, flips, owed)) = rig() else { return };
    let n = |a: &Arc<AtomicU64>| a.load(Ordering::SeqCst);
    let mut game = Client::connect(&mut server);
    let game_win = game.card("_WEBOS_WINDOW_TYPE_CARD", "Game");
    let game_buf = game.shm_pattern(1920, 1080, game_green);
    game.present(&game_win.surface, &game_buf);
    game.roundtrip();
    let mut other = Client::connect_extra(&mut server);
    let other_win = other.card_as("_WEBOS_WINDOW_TYPE_OVERLAY", "Other", "com.example.other");
    let other_buf = other.shm_pattern(1920, 1080, |x, y| if OTHER.contains(x, y) { ORANGE } else { 0 });
    other.present(&other_win.surface, &other_buf);
    other.roundtrip();
    let mut frame = Client::connect_extra(&mut server);
    let frame_win = frame.card_as("_WEBOS_WINDOW_TYPE_OVERLAY", "Frame", "org.nphil.lumatv.frame");
    let frame_buf = frame.shm_pattern(1920, 1080, |x, y| if CONTROLS.contains(x, y) { DARK } else { 0 });
    frame.present(&frame_win.surface, &frame_buf);
    frame.roundtrip();
    let sid = server.eval(|s| s.scene.windows().find(|w| w.title == "Frame").unwrap().surface);
    let set_hole = |rect: Option<Rect>| {
        server.eval(move |s| {
            s.scene.holes.retain(|h| h.surface != sid);
            if let Some(rect) = rect {
                s.scene.holes.push(Hole { surface: sid, rect });
            }
        });
    };
    let check = |game: fn(i32, i32) -> u32, hole: Option<Rect>, what: &str| {
        let img = server.eval(harness::drawn);
        for y in (0..1080).step_by(7) {
            for x in (0..1920).step_by(7) {
                assert_eq!(at(&img, x, y), page_has(game, hole, x, y), "{what}: ({x},{y})");
            }
        }
        // the hole's own edge: its last pixels are cleared and the next ones outside it are not
        if let Some(h) = hole {
            for (x, y) in [(h.x, h.y + 3), (h.right() - 1, h.bottom() - 4), (h.x + 3, h.y), (h.right() - 4, h.bottom() - 1), (h.x - 1, h.y + 3), (h.right(), h.y + 3), (h.x + 3, h.y - 1), (h.x + 3, h.bottom())] {
                if (0..1920).contains(&x) && (0..1080).contains(&y) {
                    assert_eq!(at(&img, x, y), page_has(game, hole, x, y), "{what}: the hole's edge at ({x},{y})");
                }
            }
        }
    };
    check(game_green, None, "no video yet");
    set_hole(Some(HOLE));
    check(game_green, Some(HOLE), "a card's hole");
    set_hole(Some(FULL));
    check(game_green, Some(FULL), "the hole grown to the whole screen");
    // the game draws on under the video, a full picture each time: cleared again, and one pass over the screen at most
    let drawn_on = game.shm_pattern(1920, 1080, game_drawn_on);
    game.present(&game_win.surface, &drawn_on);
    game.roundtrip();
    let (px0, us0) = (n(&owed), server.eval(|s| s.stats.render_us_last));
    check(game_drawn_on, Some(FULL), "the game drawing on under a full-screen video");
    let (owed_px, us) = (n(&owed) - px0, server.eval(|s| s.stats.render_us_last));
    eprintln!("measured: a full picture drawn under a full-screen video hole: {owed_px} px owed, {us} us in the GL driver (the frame before: {us0} us)");
    assert!(owed_px <= 1920 * 1088, "one pass over the page's 64-row tiles at most (17 of them, 1088 rows): {owed_px} px");
    set_hole(Some(HOLE));
    check(game_drawn_on, Some(HOLE), "the hole back to the card: the game's picture returns round it");
    set_hole(None);
    check(game_drawn_on, None, "the video gone");
    let (drawn0, flips0, px0) = (server.eval(|s| s.stats.frames_drawn), n(&flips), n(&owed));
    server.eval(|s| {
        for _ in 0..120 {
            s.tick();
        }
    });
    assert_eq!((server.eval(|s| s.stats.frames_drawn), n(&flips), n(&owed) - px0), (drawn0, flips0, 0), "nothing changes, nothing is drawn");
    std::mem::forget((game_buf, other_buf, frame_buf, drawn_on));
}
