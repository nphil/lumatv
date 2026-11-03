//! Caption protection exercised through the real embedded engine and software GL.
use compositor_protocol::{Backdrop, Fill, Glass, Props, Rgba, Shadow, Shape};
use lumatv_render::host::{Engine, Exclusion, HostConfig};
use lumatv_render::tiles::TileSet;
use lumatv_render::{Gles, RenderTarget};
use lumatv_rustui::rig::{software_gl, REFRESH_NS};
use ui::{SceneOptions, Ui};

const W: u32 = 1920;
const H: u32 = 1080;

fn solid(colour: Rgba) -> Shape {
    Shape { corner_radius: 0.0, fill: Fill::Solid(colour), border_width: 0.0, border: Rgba(0, 0, 0, 0) }
}

fn protected(area: [i32; 4]) -> Exclusion {
    Exclusion { area, popup_z: (100, 110), marker_z: (120, 130) }
}

fn pixel(gl: &Gles, target: &RenderTarget, x: i32, y: i32) -> Vec<u8> {
    gl.bind_framebuffer(target.framebuffer);
    gl.read_pixels(x, y, 1, 1)
}

fn frame(engine: &mut Engine, target: &RenderTarget, now: &mut u64, full: bool) -> Vec<[i32; 4]> {
    *now += REFRESH_NS;
    engine.pump().unwrap();
    engine.plan(*now).unwrap();
    let mut changed = TileSet::new(W, H);
    for stack in 0..2 {
        changed.union(engine.changed(stack).unwrap());
    }
    if full { changed.fill(); }
    let mut areas = Vec::new();
    changed.rects(&mut areas);
    engine.draw_stack_direct(0, target.framebuffer, &areas).unwrap();
    engine.draw_stack(1, target.framebuffer, &areas).unwrap();
    let cost = engine.finish().unwrap();
    assert_eq!(cost.gl_error, None);
    engine.frame_done(*now, &cost);
    engine.tidy();
    areas
}

fn covers(areas: &[[i32; 4]], x: i32, y: i32) -> bool {
    areas.iter().any(|a| a[0] <= x && x < a[2] && a[1] <= y && y < a[3])
}

#[test]
fn exclusion_suppresses_whole_groups_and_damages_their_reappearance() {
    software_gl();
    let gl = Gles::software_pbuffer(c"libEGL.so.1", W, H).unwrap();
    let target = gl.new_render_target(W, H).unwrap();
    // SAFETY: the current host context outlives the embedded engine.
    let wrapped = unsafe { Gles::current(c"libEGL.so.1") }.unwrap();
    let shm = std::env::temp_dir().join(format!("lumatv-caption-exclusion-{}", std::process::id()));
    std::fs::create_dir_all(&shm).unwrap();
    let (mut engine, socket) = Engine::new(wrapped, HostConfig {
        width: W, height: H, refresh_hz: 60.0, shm_dir: shm.clone(), direct: false,
        gpu_sync: false, idle_trim: false, pages: 0, reference: false, blit: false,
        external: true, system_z: 100,
    }).unwrap();
    let (scene, _, _events) = ui::Scene::over_with(socket, SceneOptions { shm_dir: Some(shm.clone()), ..Default::default() }).unwrap();
    engine.finish_handshake().unwrap();
    let ui = Ui::new(scene);
    let _ground = ui.scene.shape(W, H, 0, solid(Rgba(20, 30, 40, 255)), Props::at(0.0, 0.0));
    let _popup = ui.scene.shape(100, 80, 100, solid(Rgba(200, 20, 30, 255)), Props::at(100.0, 100.0));
    let _popup_detail = ui.scene.shape(30, 30, 101, solid(Rgba(240, 220, 20, 255)), Props::at(260.0, 100.0));
    let _marker = ui.scene.shape(30, 30, 120, solid(Rgba(20, 200, 30, 255)), Props::at(400.0, 100.0));
    let _overlay = ui.scene.shape(30, 30, 140, solid(Rgba(20, 30, 220, 255)), Props::at(110.0, 110.0));
    let _glass = ui.scene.glass(100, 80, 102, Glass {
        corner_radius: 10.0, tint: Rgba(180, 180, 180, 255), edge: Rgba(0, 0, 0, 0), edge_width: 0.0,
        highlight: Rgba(0, 0, 0, 0), sheen: None,
        shadows: [Some(Shadow { offset_y: 0.0, blur: 30.0, colour: Rgba(0, 0, 0, 200) }), None],
        depth: None, backdrop: Backdrop::Cached { radius: 12.0 }, saturation: 1.0, refraction: 0.0,
        edge_lens: None, tint_gradient: None,
    }, Props::at(600.0, 100.0));
    ui.try_commit().unwrap();
    let mut now = lumatv_render::now_ns();
    // The scene arrives over a socket; wait for its first real frame.
    for _ in 0..100 {
        frame(&mut engine, &target, &mut now, true);
        if pixel(&gl, &target, 150, 150) == [200, 20, 30, 255] { break; }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(pixel(&gl, &target, 150, 150), [200, 20, 30, 255]);
    assert_eq!(pixel(&gl, &target, 270, 110), [240, 220, 20, 255]);
    assert_eq!(pixel(&gl, &target, 410, 110), [20, 200, 30, 255]);
    assert_eq!(pixel(&gl, &target, 115, 115), [20, 30, 220, 255]);

    engine.set_exclusion(Some(protected([200, 100, 201, 110])));
    frame(&mut engine, &target, &mut now, false);
    assert_eq!(pixel(&gl, &target, 150, 150), [200, 20, 30, 255], "touching an edge is not overlap");
    engine.set_exclusion(Some(protected([190, 140, 210, 160])));
    assert!(engine.redraw_wanted(), "caption metadata alone requests a frame");
    let damage = frame(&mut engine, &target, &mut now, false);
    assert!(covers(&damage, 150, 150) && covers(&damage, 270, 110), "the removed popup and its distant detail must both be damaged: {damage:?}");
    assert_eq!(pixel(&gl, &target, 150, 150), [20, 30, 40, 255]);
    assert_eq!(pixel(&gl, &target, 270, 110), [20, 30, 40, 255]);
    assert_eq!(pixel(&gl, &target, 410, 110), [20, 200, 30, 255]);
    assert_eq!(pixel(&gl, &target, 115, 115), [20, 30, 220, 255], "unrelated overlay remains");

    engine.set_exclusion(Some(protected([400, 100, 410, 110])));
    let damage = frame(&mut engine, &target, &mut now, false);
    assert!(covers(&damage, 150, 150) && covers(&damage, 410, 110));
    assert_eq!(pixel(&gl, &target, 150, 150), [200, 20, 30, 255]);
    assert_eq!(pixel(&gl, &target, 410, 110), [20, 30, 40, 255]);

    // This rectangle is outside the glass body but inside its shadow extent.
    engine.set_exclusion(Some(protected([595, 125, 599, 135])));
    let damage = frame(&mut engine, &target, &mut now, false);
    assert!(covers(&damage, 150, 150) && covers(&damage, 270, 110));
    assert_eq!(pixel(&gl, &target, 150, 150), [20, 30, 40, 255], "shadow overlap suppresses the whole popup");
    assert_eq!(pixel(&gl, &target, 410, 110), [20, 200, 30, 255]);

    engine.set_exclusion(None);
    let damage = frame(&mut engine, &target, &mut now, false);
    assert!(covers(&damage, 150, 150) && covers(&damage, 270, 110));
    assert_eq!(pixel(&gl, &target, 150, 150), [200, 20, 30, 255]);
    assert_eq!(pixel(&gl, &target, 270, 110), [240, 220, 20, 255]);
    assert_eq!(pixel(&gl, &target, 115, 115), [20, 30, 220, 255]);

    engine.set_standin_groups(Some(((100, 110), (120, 130))));
    let damage = frame(&mut engine, &target, &mut now, false);
    assert!(covers(&damage, 150, 150) && covers(&damage, 270, 110) && covers(&damage, 410, 110));
    assert_eq!(pixel(&gl, &target, 150, 150), [20, 30, 40, 255]);
    assert_eq!(pixel(&gl, &target, 270, 110), [20, 30, 40, 255]);
    assert_eq!(pixel(&gl, &target, 410, 110), [20, 30, 40, 255]);
    assert_eq!(pixel(&gl, &target, 115, 115), [20, 30, 220, 255], "stand-in retains unrelated overlays");

    engine.set_standin_groups(None);
    let damage = frame(&mut engine, &target, &mut now, false);
    assert!(covers(&damage, 150, 150) && covers(&damage, 270, 110) && covers(&damage, 410, 110));
    assert_eq!(pixel(&gl, &target, 150, 150), [200, 20, 30, 255]);
    assert_eq!(pixel(&gl, &target, 270, 110), [240, 220, 20, 255]);
    assert_eq!(pixel(&gl, &target, 410, 110), [20, 200, 30, 255]);
    assert_eq!(pixel(&gl, &target, 115, 115), [20, 30, 220, 255]);
    let _ = std::fs::remove_dir_all(shm);
}
