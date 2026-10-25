//! The UI engine in the compositor's frame (feature `ui`), on Mesa's software GL: the compositor's own EGL context, its own tile
//! ring over three pages, a window drawn between the engine's two stacks, and the demo surface as the app. Every page must be the
//! frame drawn whole, the context must be as the compositor left it after every call of the engine, and the first frames of the
//! idle demo must draw only the tiles that changed.
#![cfg(feature = "ui")]

use lumatv_compositor::damage::{Ring, Tiles};
use lumatv_compositor::render::egl::{self, Egl, Platform};
use lumatv_compositor::ui::Ui;
use lumatv_compositor::ui::demo::{Demo, SYSTEM_Z};
use lumatv_compositor::{OUTPUT_HEIGHT as H, OUTPUT_WIDTH as W, Rect};
use std::path::Path;

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";

/// A render target of the compositor's context, as a scan-out page is made: a texture and a framebuffer over it, opaque black.
fn page(egl: &Egl) -> u32 {
    let a = &egl.api;
    // SAFETY: GL object creation in the current context; every object is checked.
    unsafe {
        let (mut texture, mut fbo) = (0, 0);
        (a.glGenTextures)(1, &mut texture);
        (a.glBindTexture)(egl::GL_TEXTURE_2D, texture);
        (a.glTexImage2D)(egl::GL_TEXTURE_2D, 0, egl::GL_RGBA as i32, W as i32, H as i32, 0, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, std::ptr::null());
        (a.glTexParameteri)(egl::GL_TEXTURE_2D, egl::GL_TEXTURE_MIN_FILTER, egl::GL_NEAREST);
        (a.glTexParameteri)(egl::GL_TEXTURE_2D, egl::GL_TEXTURE_MAG_FILTER, egl::GL_NEAREST);
        (a.glGenFramebuffers)(1, &mut fbo);
        (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
        (a.glFramebufferTexture2D)(egl::GL_FRAMEBUFFER, egl::GL_COLOR_ATTACHMENT0, egl::GL_TEXTURE_2D, texture, 0);
        assert_eq!((a.glCheckFramebufferStatus)(egl::GL_FRAMEBUFFER), egl::GL_FRAMEBUFFER_COMPLETE);
        (a.glDisable)(egl::GL_SCISSOR_TEST);
        (a.glClearColor)(0.0, 0.0, 0.0, 1.0);
        (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
        fbo
    }
}

fn read(egl: &Egl, fbo: u32) -> Vec<u8> {
    let mut pixels = vec![0u8; (W * H * 4) as usize];
    // SAFETY: reads W x H RGBA pixels into a buffer of that size.
    unsafe {
        (egl.api.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
        (egl.api.glFinish)();
        (egl.api.glReadPixels)(0, 0, W as i32, H as i32, egl::GL_RGBA, egl::GL_UNSIGNED_BYTE, pixels.as_mut_ptr().cast());
    }
    pixels
}

/// A window of the compositor, drawn the way the compositor draws a quad it has to clip: a scissored colour inside each owed tile
/// run (a clear of the scissor, which is the simplest opaque rectangle there is).
fn window(egl: &Egl, fbo: u32, rect: Rect, rects: &[Rect]) {
    let a = &egl.api;
    // SAFETY: state calls in the current context.
    unsafe {
        (a.glBindFramebuffer)(egl::GL_FRAMEBUFFER, fbo);
        (a.glViewport)(0, 0, W as i32, H as i32);
        (a.glEnable)(egl::GL_SCISSOR_TEST);
        (a.glClearColor)(0.9, 0.3, 0.2, 1.0);
        for r in rects {
            let clip = r.intersect(&rect);
            if !clip.is_empty() {
                // a scan-out page's row 0 is the top of the screen, which is where GL's scissor counts from
                (a.glScissor)(clip.x, clip.y, clip.w, clip.h);
                (a.glClear)(egl::GL_COLOR_BUFFER_BIT);
            }
        }
        (a.glDisable)(egl::GL_SCISSOR_TEST);
    }
}

#[test]
fn the_demo_surface_is_drawn_into_the_compositors_pages_as_the_ring_owes_them() {
    if !Path::new(MESA).exists() {
        eprintln!("no Mesa libEGL on this host; skipping");
        return;
    }
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    // The compositor's own context: ES 2 is asked for (`render/egl.rs`), and Mesa answers with the newest it has.
    let egl = Egl::open(MESA, Platform::Surfaceless).expect("the compositor's EGL");
    let pages: Vec<u32> = (0..3).map(|_| page(&egl)).collect();
    let reference = page(&egl);
    let shm = std::env::temp_dir().join(format!("lumatv-compositor-ui-{}", std::process::id()));
    std::fs::create_dir_all(&shm).expect("a shm dir");
    // SAFETY: the context is current on this thread and outlives the engine; every call is made here.
    let (mut ui, app_end) = unsafe { Ui::new(MESA, W, H, SYSTEM_Z, shm.clone()) }.expect("the engine in the compositor's context");
    let mut demo = Demo::new(app_end, &shm).expect("the demo surface");
    ui.finish_handshake().expect("the handshake");

    let mut ring = Ring::new(3, W, H);
    let mut windows_before = Tiles::new(W, H);
    let (mut frames, mut partial, mut drawn_px, mut changed_px) = (0usize, 0usize, 0u64, 0u64);
    let mut now = lumatv_render_now();
    let state = |ui: &mut Ui| {
        let gl = ui.engine().gles();
        let saved = gl.enter();
        gl.leave(&saved);
        format!("{saved:?}")
    };
    for k in 0..240usize {
        now += 16_666_667;
        demo.tick(now).expect("the app's tick");
        match k {
            60 => demo.show_volume(0.4),
            75 => demo.show_volume(0.55),
            100 => demo.show_volume(0.8),
            140 => demo.hide_volume(),
            _ => {}
        }
        demo.tick(now).expect("the app's commit");
        ui.pump().expect("pump");

        // the window crosses the page; where it was and where it is are damage
        let at = Rect::new(240 + (k as i32 * 9) % 900, 430, 480, 200);
        let mut tiles = Tiles::new(W, H);
        tiles.add(&at);
        tiles.union(&windows_before);
        windows_before = Tiles::new(W, H);
        windows_before.add(&at);

        // the compositor's context has state of its own when the engine is called
        // SAFETY: state calls in the current context.
        unsafe {
            (egl.api.glBindFramebuffer)(egl::GL_FRAMEBUFFER, pages[k % 3]);
            (egl.api.glViewport)(0, 0, 640, 480);
            (egl.api.glEnable)(egl::GL_SCISSOR_TEST);
            (egl.api.glScissor)(5, 6, 7, 8);
            (egl.api.glEnable)(egl::GL_BLEND);
        }
        let before = state(&mut ui);
        ui.plan(now, &mut tiles).expect("plan");
        assert_eq!(state(&mut ui), before, "frame {k}: plan left the context's state changed");
        let owed = ring.frame(k % 3, &tiles).clone();
        partial += usize::from(owed.pixels() < u64::from(W) * u64::from(H));
        drawn_px += owed.pixels();

        let mut all = Tiles::new(W, H);
        all.fill();
        let compose = |ui: &mut Ui, fbo: u32, tiles: &Tiles, check: bool| {
            let mut r: Vec<Rect> = Vec::new();
            tiles.rects(&mut r);
            let state_before = state(ui);
            ui.draw_under(fbo, tiles).expect("stack 0");
            if check {
                assert_eq!(state(ui), state_before, "frame {k}: draw_under left the context's state changed");
            }
            window(&egl, fbo, at, &r);
            let state_before = state(ui);
            ui.draw_over(fbo, tiles).expect("stack 1");
            if check {
                assert_eq!(state(ui), state_before, "frame {k}: draw_over left the context's state changed");
            }
        };
        compose(&mut ui, pages[k % 3], &owed, true);
        compose(&mut ui, reference, &all, false);
        let cost = ui.finish(now).expect("finish");
        changed_px += cost.damage_px;
        assert_eq!(cost.gl_error, None, "frame {k}: a GL error");
        ui.tidy();

        let (drawn, whole) = (read(&egl, pages[k % 3]), read(&egl, reference));
        if drawn != whole {
            let at = drawn.iter().zip(&whole).position(|(a, b)| a != b).expect("a difference") / 4;
            panic!("frame {k}: the page differs from the frame drawn whole, first at ({}, {}); {} px owed", at as u32 % W, at as u32 / W, owed.pixels());
        }
        frames += 1;
    }
    assert_eq!(frames, 240);
    assert!(partial > 200, "only {partial} of {frames} pages were drawn in part");
    eprintln!(
        "{frames} frames, {partial} drawn in part, {:.1} % of the output drawn a frame, {:.1} % changed",
        100.0 * drawn_px as f64 / (frames as f64 * f64::from(W) * f64::from(H)),
        100.0 * changed_px as f64 / (frames as f64 * f64::from(W) * f64::from(H))
    );
    let _ = std::fs::remove_dir_all(&shm);
}

fn lumatv_render_now() -> u64 {
    lumatv_render::now_ns()
}
