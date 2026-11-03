//! The engine inside a compositor's frame: it wraps the context the compositor draws in, draws its layers into the compositor's
//! pages only where the compositor's account says the page owes, with the compositor's own drawing between its two stacks, and
//! leaves the context's state as it found it.
//!
//! The test is a small compositor: a context of its own (the host's), three pages in it, a ring that accounts for what each owes
//! (windows' damage and the engine's), and a window that is a solid rectangle drawn between the stacks. Every frame is also drawn
//! whole into another target, stack 0 straight from its layers where the page has it from the engine's floor, and the page must
//! be the same picture.

use compositor_protocol::{Backdrop, Fill, Glass, Props, Rgba, Shadow, Shape, Sheen};
use lumatv_render::host::{Engine, HostConfig};
use lumatv_render::tiles::{Ring, TileSet};
use lumatv_render::{Gles, RenderTarget};
use lumatv_rustui::rig::{REFRESH_NS, software_gl};
use std::ffi::CStr;
use ui::{SceneOptions, Ui};

const W: u32 = 1920;
const H: u32 = 1080;
const LIB: &CStr = c"libEGL.so.1";

fn solid(radius: f32, r: u8, g: u8, b: u8, a: u8) -> Shape {
    Shape { corner_radius: radius, fill: Fill::Solid(Rgba(r, g, b, a)), border_width: 0.0, border: Rgba(0, 0, 0, 0) }
}

fn glass(backdrop: Backdrop) -> Glass {
    Glass {
        corner_radius: 24.0,
        tint: Rgba(10, 8, 16, 90),
        edge: Rgba(30, 30, 30, 30),
        edge_width: 1.0,
        highlight: Rgba(12, 12, 12, 12),
        sheen: Some(Sheen { peak: 0.16, floor: 0.05, start: 0.38, end: 0.6 }),
        shadows: [Some(Shadow { offset_y: 12.0, blur: 30.0, colour: Rgba(0, 0, 0, 70) }), None],
        depth: None,
        backdrop,
        saturation: 1.2,
        refraction: 0.0,
        edge_lens: None,
        tint_gradient: None,
    }
}

/// A window of the compositor: a solid rectangle, drawn by the host's own GL code (a program, a vertex buffer, a scissor and a blend
/// state that the engine must not leave changed).
struct Windows {
    program: u32,
    buffer: u32,
    colour: i32,
}

impl Windows {
    fn new(gl: &Gles) -> Self {
        let program = gl
            .program(
                c"#version 300 es\nlayout(location = 0) in vec2 a_pos;\nvoid main() { gl_Position = vec4(a_pos, 0.0, 1.0); }",
                c"#version 300 es\nprecision mediump float;\nuniform vec4 u_colour;\nout vec4 o_colour;\nvoid main() { o_colour = u_colour; }",
            )
            .expect("the window program");
        let colour = gl.uniform_location(program, c"u_colour");
        Self { program, buffer: gl.new_vertex_buffer(), colour }
    }

    /// A rectangle of the page (panel pixels, row 0 at the top as a scan-out page is), drawn inside each of `areas`.
    fn draw(&self, gl: &Gles, target: u32, rect: [i32; 4], colour: [f32; 4], areas: &[[i32; 4]]) {
        let ndc = |x: i32, y: i32| [x as f32 / W as f32 * 2.0 - 1.0, y as f32 / H as f32 * 2.0 - 1.0, 0.0, 0.0];
        gl.static_vertices(self.buffer, &[ndc(rect[0], rect[1]), ndc(rect[2], rect[1]), ndc(rect[0], rect[3]), ndc(rect[2], rect[3])]);
        gl.bind_framebuffer(target);
        gl.viewport(0, 0, W as i32, H as i32);
        gl.use_program(self.program);
        gl.use_vertex_buffer(self.buffer);
        gl.uniform4f(self.colour, colour[0], colour[1], colour[2], colour[3]);
        gl.enable(lumatv_render::gl::GL_SCISSOR_TEST);
        gl.enable(lumatv_render::gl::GL_BLEND);
        gl.blend_func(lumatv_render::gl::GL_ONE, lumatv_render::gl::GL_ONE_MINUS_SRC_ALPHA);
        for area in areas {
            let clip = [area[0].max(rect[0]), area[1].max(rect[1]), area[2].min(rect[2]), area[3].min(rect[3])];
            if clip[2] > clip[0] && clip[3] > clip[1] {
                gl.scissor(clip[0], clip[1], clip[2] - clip[0], clip[3] - clip[1]);
                gl.draw_triangle_strip(0, 4);
            }
        }
        gl.disable(lumatv_render::gl::GL_SCISSOR_TEST);
    }
}

#[test]
fn the_engine_draws_into_a_compositors_pages_between_its_windows_and_leaves_its_state_alone() {
    software_gl();
    // the compositor's context, which is current
    let host = Gles::software_pbuffer(LIB, W, H).expect("a software GL");
    let windows = Windows::new(&host);
    let pages: Vec<RenderTarget> = (0..3).map(|_| host.new_render_target(W, H).expect("a page")).collect();
    let reference = host.new_render_target(W, H).expect("a target");
    // the engine is given the context as it is
    // SAFETY: the host's context is current on this thread and outlives the engine.
    let wrapped = unsafe { Gles::current(LIB) }.expect("the current context");
    assert!(wrapped.version().is_some_and(|(major, _)| major >= 3));
    let shm = std::env::temp_dir().join(format!("lumatv-embedded-{}", std::process::id()));
    std::fs::create_dir_all(&shm).expect("a shm dir");
    let (mut engine, app_end) = Engine::new(
        wrapped,
        HostConfig {
            width: W,
            height: H,
            refresh_hz: 60.0,
            shm_dir: shm.clone(),
            direct: false,
            gpu_sync: false,
            idle_trim: false,
            pages: 0,
            reference: false,
            blit: false,
            external: true,
            system_z: 100,
        },
    )
    .expect("an engine in the host's context");
    let (scene, _welcome, events) = ui::Scene::over_with(app_end, SceneOptions { shm_dir: Some(shm.clone()), ..SceneOptions::default() }).expect("a scene");
    engine.finish_handshake().expect("the handshake");
    let ui = Ui::new(scene);
    // stack 0: a ground, bars that move, glass that shows the blur of what is below it; stack 1: a toast and a pill over the windows
    let _ground = ui.scene.shape(W, H, 0, solid(0.0, 24, 28, 44, 255), Props::at(0.0, 0.0));
    let bar_a = ui.scene.shape(90, 90, 2, solid(20.0, 240, 150, 40, 255), Props::at(0.0, 150.0));
    let bar_b = ui.scene.shape(90, 90, 2, solid(20.0, 80, 220, 120, 255), Props::at(300.0, 700.0));
    let _card = ui.scene.glass(560, 280, 5, glass(Backdrop::Cached { radius: 24.0 }), Props::at(100.0, 80.0));
    let live = ui.scene.glass(500, 240, 6, glass(Backdrop::Live { radius: 12.0, downsample: 4 }), Props::at(900.0, 700.0));
    let toast = ui.scene.shape(420, 90, 100, solid(30.0, 240, 240, 250, 255), Props::at(750.0, 40.0));
    let pill = ui.scene.glass(300, 80, 101, glass(Backdrop::Live { radius: 16.0, downsample: 2 }), Props::at(300.0, 900.0));

    let mut ring = Ring::new(3, W, H);
    let (mut owed, mut total) = (TileSet::new(W, H), TileSet::new(W, H));
    let mut window_before = TileSet::new(W, H);
    let mut now = lumatv_render::now_ns();
    let (mut partial, mut frames, mut checked_state) = (0usize, 0usize, 0usize);
    let snapshot = |gl: &Gles| {
        let saved = gl.enter();
        gl.leave(&saved);
        format!("{saved:?}")
    };
    for k in 0..240usize {
        now += REFRESH_NS;
        let f = k as f32;
        bar_a.set(Props::at((f * 13.0) % 1800.0, 150.0));
        if k % 3 == 0 {
            bar_b.set(Props::at(300.0 + (f * 5.0) % 900.0, 700.0));
        }
        live.set(Props::at(900.0 + (f * 3.0) % 300.0, 700.0));
        if k % 50 < 20 {
            toast.set(Props::at(750.0, 40.0 + (k % 50) as f32 * 2.0));
        }
        pill.set(Props::at(300.0 + (f * 4.0) % 700.0, 900.0));
        if ui.scene.has_pending() {
            let _ = ui.try_commit().expect("a commit");
        }
        while events.try_recv().is_ok() {}
        engine.pump().expect("pump");

        // the window moves across the page: its old place and its new one are the page's damage
        let window = [200 + (k as i32 * 7) % 900, 300, 200 + (k as i32 * 7) % 900 + 480, 620];
        let mut window_now = TileSet::new(W, H);
        window_now.add(window);

        // The compositor's context has state of its own when the engine is called: a program, buffers, a scissor that is on.
        host.use_program(windows.program);
        host.use_vertex_buffer(windows.buffer);
        host.enable(lumatv_render::gl::GL_SCISSOR_TEST);
        host.scissor(5, 6, 7, 8);
        host.enable(lumatv_render::gl::GL_BLEND);
        host.viewport(0, 0, 640, 480);
        host.bind_framebuffer(pages[0].framebuffer);
        let before = snapshot(&host);

        engine.plan(now).expect("plan");
        assert!(engine.has_layers(0) && engine.has_layers(1), "frame {k}: both stacks have layers");
        assert_eq!(snapshot(&host), before, "frame {k}: plan left the context's state changed");
        // what the page owes: what the windows changed, what the engine changed, and what the frames before it left
        total.clear();
        total.union(&window_now);
        total.union(&window_before);
        for stack in 0..2 {
            total.union(engine.changed(stack).expect("planned"));
        }
        window_before = window_now;
        let page = k % 3;
        ring.frame(page, &total, &mut owed);
        let mut areas = Vec::new();
        owed.rects(&mut areas);
        assert!(areas.len() <= 16, "{} areas", areas.len());
        partial += usize::from(owed.pixels() < u64::from(W) * u64::from(H));

        let draw_all = |engine: &mut Engine, target: &RenderTarget, areas: &[[i32; 4]], check: bool| {
            let state = snapshot(&host);
            if check {
                engine.draw_stack(0, target.framebuffer, areas).expect("stack 0");
                assert_eq!(snapshot(&host), state, "frame {k}: draw_stack(0) left the context's state changed");
            } else {
                engine.draw_stack_direct(0, target.framebuffer, areas).expect("stack 0");
            }
            windows.draw(&host, target.framebuffer, window, [0.9, 0.3, 0.2, 1.0], areas);
            let state = snapshot(&host);
            engine.draw_stack(1, target.framebuffer, areas).expect("stack 1");
            if check {
                assert_eq!(snapshot(&host), state, "frame {k}: draw_stack(1) left the context's state changed");
            }
        };
        draw_all(&mut engine, &pages[page], &areas, true);
        checked_state += 1;
        // the same frame drawn whole into the reference, stack 0 straight from its layers (the page has it from the floor)
        draw_all(&mut engine, &reference, &[[0, 0, W as i32, H as i32]], false);
        let state = snapshot(&host);
        let cost = engine.finish().expect("finish");
        assert_eq!(snapshot(&host), state, "frame {k}: finish left the context's state changed");
        assert_eq!(cost.gl_error, None, "frame {k}: GL error");
        engine.frame_done(now, &cost);
        engine.tidy();

        host.bind_framebuffer(pages[page].framebuffer);
        let drawn = host.read_pixels(0, 0, W, H);
        host.bind_framebuffer(reference.framebuffer);
        let whole = host.read_pixels(0, 0, W, H);
        if drawn != whole {
            let differing: Vec<usize> = drawn.chunks_exact(4).zip(whole.chunks_exact(4)).enumerate().filter(|(_, (a, b))| a != b).map(|(at, _)| at).collect();
            let (x0, x1) = (differing.iter().map(|at| *at as u32 % W).min().unwrap(), differing.iter().map(|at| *at as u32 % W).max().unwrap());
            let (y0, y1) = (differing.iter().map(|at| *at as u32 / W).min().unwrap(), differing.iter().map(|at| *at as u32 / W).max().unwrap());
            let first = differing[0];
            for (name, rows) in [("page", &drawn), ("whole", &whole)] {
                let _ = lumatv_rustui_png(&std::env::temp_dir().join(format!("lumatv-embedded-{name}.png")), rows);
            }
            panic!(
                "frame {k}: {} px of the page differ from the frame drawn whole, in x {x0}..={x1}, y {y0}..={y1}; first ({}, {}): page {:?} whole {:?}; areas {areas:?}",
                differing.len(),
                first as u32 % W,
                first as u32 / W,
                &drawn[first * 4..first * 4 + 4],
                &whole[first * 4..first * 4 + 4]
            );
        }
        frames += 1;
    }
    assert_eq!(frames, 240);
    assert!(partial > 150, "only {partial} of {frames} pages were drawn in part");
    assert_eq!(checked_state, 240);
    let _ = std::fs::remove_dir_all(&shm);
}

fn lumatv_rustui_png(path: &std::path::Path, rows: &[u8]) -> std::io::Result<()> {
    let mut rgba = Vec::with_capacity(rows.len());
    // the page is laid out as a scan-out page is: row 0, the first in memory, is the top of the screen
    for y in 0..H as usize {
        for px in rows[y * W as usize * 4..(y + 1) * W as usize * 4].chunks_exact(4) {
            rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
    }
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path)?), W, H);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(&rgba)).map_err(std::io::Error::other)
}
