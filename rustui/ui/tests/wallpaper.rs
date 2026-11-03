//! Owner wallpaper controls through the production surface, native engine, and Mesa pixels.
use compositor_protocol::{Props, Wallpaper, WallpaperKind};
use lumatv_render::host::{Engine, FrameCost, HostConfig};
use lumatv_render::{Demand, Gles, silk_report};
use lumatv_tokens::Tokens;
use lumatv_ui::home::wallpaper::WallpaperSurface;
use lumatv_ui::host::theme;
use lumatv_ui::kit::Kit;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use ui::lucent::{material::MaterialTier, palette::Mode};
use ui::scene::{Scene, SceneOptions};

const W: u32 = 1920;
const H: u32 = 1080;
const STEP: u64 = 1_000_000_000 / 60;

struct HomeWall {
    surface: WallpaperSurface,
    kit: Kit,
    engine: Engine,
    events: Receiver<compositor_protocol::Event>,
    data: PathBuf,
    keep_data: bool,
    now: u64,
}

impl HomeWall {
    fn new() -> Self {
        let data = std::env::temp_dir().join(format!("lumatv-wallpaper-gpu-{}", std::process::id()));
        std::fs::create_dir_all(data.join("theme")).unwrap();
        let mut pixels = Vec::with_capacity((W * H * 4) as usize);
        for y in 0..H {
            for x in 0..W {
                let wave = |n: u32| ((n % 96).min(96 - n % 96) * 4) as u8;
                pixels.extend_from_slice(&[50 + wave(x), 35 + wave(y), 180, 255]);
            }
        }
        let file = std::fs::File::create(data.join("theme/wall_1080.png")).unwrap();
        let mut encoder = png::Encoder::new(file, W, H);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
        static CONNECTION: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let connection = CONNECTION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let shm = data.join(format!("shm-{connection}"));
        std::fs::create_dir_all(&shm).unwrap();
        let gl = Gles::software_pbuffer(c"libEGL.so.1", W, H).expect("Mesa EGL");
        let (mut engine, app) = Engine::new(gl, HostConfig {
            width: W, height: H, refresh_hz: 60.0, shm_dir: shm.clone(), direct: false,
            gpu_sync: true, idle_trim: false, pages: 3, reference: true, blit: true,
            external: false, system_z: 1000,
        }).unwrap();
        let (scene, _, events) = Scene::over_with(app, SceneOptions { shm_dir: Some(shm), ..SceneOptions::default() }).unwrap();
        engine.finish_handshake().unwrap();
        let kit = Kit::new(scene, Tokens::shipped(), Mode::Dark, MaterialTier::T1, 1.0).unwrap();
        let mut wall = Self { surface: WallpaperSurface::new(0), kit, engine, events, data, keep_data: false, now: 1_000_000_000 };
        wall.surface.set_held(true);
        if theme::theme_path(&wall.data).is_file() { wall.reload(); } else { wall.configure(json!({})); }
        wall
    }

    fn configure(&mut self, motion: Value) {
        // Replacing only this section models reset as well as an atomic persisted owner edit.
        theme::update(&self.data, |doc| { doc["wallpaperMotion"] = motion; true }).unwrap();
        self.reload();
    }
    fn reload(&mut self) {
        let persisted: Value = serde_json::from_slice(&std::fs::read(theme::theme_path(&self.data)).unwrap()).unwrap();
        self.surface.load(&mut self.kit, &persisted, Some(&self.data), None).unwrap();
        // The report deliberately arrives before the scene's Create/Restyle transaction.
        silk_report::apply(&mut self.engine, &json!({"home": {"silk": self.surface.report()}}));
    }

    fn phase(&mut self, speed: f64, elapsed: u64) -> Vec<u8> {
        self.surface = WallpaperSurface::new(0);
        self.configure(json!({"fps": 60.0, "speed": speed}));
        let mut frame = self.shot();
        let until = self.now + elapsed;
        while self.now < until { frame = self.draw_at((self.now + 50_000_000).min(until)).0; }
        frame
    }

    fn prepare_glass(&mut self) -> ui::scene::GlassLayer {
        let theme: Value = serde_json::from_slice(&std::fs::read(theme::theme_path(&self.data)).unwrap()).unwrap();
        for _ in 0..100 {
            self.surface.sync_backdrop(&mut self.kit, &theme);
            self.shot();
            self.engine.tidy();
            while let Ok(event) = self.events.try_recv() { self.kit.ui.scene.observe(&event); }
            self.surface.sync_backdrop(&mut self.kit, &theme);
            if let Some(id) = self.kit.wall_backdrop {
                let clear = compositor_protocol::Rgba(0, 0, 0, 0);
                return self.kit.ui.scene.glass(400, 300, 2, compositor_protocol::Glass {
                    corner_radius: 0.0, tint: clear, edge: clear, edge_width: 0.0,
                    highlight: clear, sheen: None, shadows: [None, None], depth: None,
                    backdrop: compositor_protocol::Backdrop::Prepared { id }, saturation: 1.0, refraction: 0.0,
                    edge_lens: None, tint_gradient: None,
                }, Props::at(760.0, 390.0));
            }
        }
        panic!("graded plain source did not become a prepared glass backdrop");
    }


    fn draw_at(&mut self, now: u64) -> (Vec<u8>, FrameCost) {
        let wanted = self.kit.ui.scene.has_pending().then(|| self.kit.ui.scene.try_commit().unwrap().get());
        let until = Instant::now() + Duration::from_secs(5);
        self.now = now;
        loop {
            self.kit.ui.scene.writer_ready().unwrap();
            self.engine.pump().unwrap();
            let cost = self.engine.draw(now);
            assert_eq!(cost.gl_error, None);
            assert_eq!(cost.mismatch_px, 0, "retained pages must match a full render");
            self.engine.frame_done(now, &cost);
            self.engine.pump().unwrap();
            while let Ok(event) = self.events.try_recv() {
                if let compositor_protocol::Event::Error { message } = &event { panic!("native scene: {message}"); }
                self.kit.ui.scene.observe(&event);
            }
            if wanted.is_none_or(|tag| cost.tag >= tag) { return (self.engine.read_window(), cost); }
            assert!(Instant::now() < until, "native scene commit did not arrive: wanted {wanted:?}, presented {}", cost.tag);
            std::thread::yield_now();
        }
    }

    fn shot(&mut self) -> Vec<u8> { self.draw_at(self.now + STEP).0 }
}

impl Drop for HomeWall {
    fn drop(&mut self) { if !self.keep_data { let _ = std::fs::remove_dir_all(&self.data); } }
}

fn difference(a: &[u8], b: &[u8], left: u32, right: u32) -> usize {
    a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate().filter(|(i, (p, q))| {
        let x = *i as u32 % W;
        x >= left && x < right && (0..3).any(|c| p[c].abs_diff(q[c]) > 2)
    }).count()
}

fn difference_rect(a: &[u8], b: &[u8], left: u32, top: u32, right: u32, bottom: u32) -> usize {
    a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate().filter(|(i, (p, q))| {
        let x = *i as u32 % W;
        let y = *i as u32 / W;
        x >= left && x < right && y >= top && y < bottom && (0..3).any(|c| p[c].abs_diff(q[c]) > 2)
    }).count()
}

#[test]
fn persisted_live_controls_change_their_picture_without_leaking_or_damaging_a_paused_reset() {
    let mut h = HomeWall::new();
    let baseline = h.shot();
    let source_sample = h.surface.sample;
    for (key, value) in [
        ("amplitude", 16.0), ("depth", 1.0), ("scale", 2.5), ("detail", 1.0),
        ("vignette", 1.0), ("specular", 1.0), ("bloom", 1.0),
        ("saturation", 0.5), ("contrast", 1.3),
    ] {
        h.configure(json!({key: value}));
        assert!(h.engine.redraw_wanted() || h.kit.ui.scene.has_pending(), "{key} wakes a held wallpaper exactly through damage or a content commit");
        let (changed, cost) = h.draw_at(h.now + STEP);
        let different = difference(&baseline, &changed, 0, W);
        eprintln!("{key}: {different} changed pixels, {} damage pixels", cost.damage_px);
        assert!(different > 5000, "{key} must change visible pixels, got {different}");
        assert!(cost.damage_px > 0, "{key} must invalidate a held wallpaper");
        assert_eq!(h.surface.sample, source_sample, "grading must not alter source material/tint sampling");
        h.configure(json!({}));
        assert_eq!(h.shot(), baseline, "{key} reset must start from the original, not a cumulative grade");
    }
    h.configure(json!({"vignette": 1.0}));
    let scoped = h.shot();
    let old_id = h.surface.report()["id"].clone();
    h.surface = WallpaperSurface::new(0);
    h.surface.set_held(true);
    h.configure(json!({"vignette": 1.0}));
    assert_ne!(h.surface.report()["id"], old_id);
    assert_eq!(h.shot(), scoped, "replacement scene/report order keeps the persisted look");
    h.configure(json!({"vignette": 99.0}));
    assert_eq!(h.shot(), scoped, "out-of-range persisted controls use the slider's bound");
    h.configure(json!({}));
    assert_eq!(h.shot(), baseline);
    let glass = h.prepare_glass();
    let glass_baseline = h.shot();
    drop(glass);
    h.configure(json!({"saturation": 0.5, "contrast": 1.3}));
    let glass = h.prepare_glass();
    let graded_glass = h.shot();
    assert!(difference_rect(&glass_baseline, &graded_glass, 760, 390, 1160, 690) > 5000,
        "glass must sample the same graded original as the visible wallpaper");
    drop(glass);
    h.configure(json!({}));
    let glass = h.prepare_glass();
    assert_eq!(h.shot(), glass_baseline, "glass grading reset is not cumulative");
    drop(glass);
    let slow = h.phase(1.0, 50_000_000);
    let fast = h.phase(3.0, 50_000_000);
    let equivalent = h.phase(1.0, 150_000_000);
    assert!(difference(&slow, &fast, 0, W) > 1000, "speed changes distance travelled at the same elapsed time");
    assert_eq!(difference(&fast, &equivalent, 0, W), 0, "three times the speed reaches the same field in one third the time");


    // Fractional JSON frame rates are emitted by slider persistence, not integer JSON numbers.
    h.surface.set_held(false);
    h.configure(json!({"fps": 5.0}));
    let a = h.shot();
    let due = match h.engine.demand() { Demand::At(at) => at, other => panic!("a 5 fps cap must sleep between pictures: {other:?}") };
    assert!(due > h.now && due - h.now <= 210_000_000, "a lowered frame cap preserves phase without an overdue or distant wake");
    let b = h.draw_at(due).0;
    let next = match h.engine.demand() { Demand::At(at) => at, other => panic!("capped wallpaper must sleep: {other:?}") };
    assert!((190_000_000..=210_000_000).contains(&(next - due)), "persisted 5.0 must schedule near 200 ms, not fallback to 10 fps");
    eprintln!("frame cap: {} ns between 5 fps pictures; speed3 and speed1 at3x elapsed matched", next - due);
    assert!(difference(&a, &b, 0, W) > 1000, "speed advances the live field");
    h.configure(json!({"fps": 60.0, "speed": 3.0}));
    h.shot();
    assert!(match h.engine.demand() { Demand::EveryRefresh => true, Demand::At(at) => at <= h.now + STEP, Demand::Idle => false },
        "raising the frame cap permits the next output refresh");
    let fast = h.draw_at(h.now + 12 * STEP).0;
    assert!(difference(&b, &fast, 0, W) > 1000, "the faster phase reaches the shader");
    h.surface.set_held(true);
    h.configure(json!({"fps": 60.0, "speed": 3.0}));
    let held = h.shot();
    assert!(matches!(h.engine.demand(), Demand::Idle));
    assert_eq!(h.draw_at(h.now + 30_000_000_000).0, held, "held wallpaper takes no elapsed-time jump");
    h.configure(json!({"enabled": false, "specular": 1.0, "bloom": 1.0}));
    let off = h.shot();
    assert!(matches!(h.engine.demand(), Demand::Idle));
    assert_eq!(h.draw_at(h.now + 30_000_000_000).0, off);
    h.kit.ui.look.reduced_motion = true;
    h.configure(json!({"enabled": true, "specular": 1.0, "bloom": 1.0}));
    assert_eq!(h.shot(), off, "reduced motion uses the same graded still");
    h.kit.ui.look.reduced_motion = false;
    h.configure(json!({}));
    h.shot();

    let mut other = h.kit.ui.scene.wallpaper(W, H, 1, Wallpaper {
        kind: WallpaperKind::Silk, speed: 1.0, intensity: 1.0, fps: 0,
    }, Props::at(W as f32 / 2.0, 0.0)).unwrap();
    other.pixels_mut().unwrap().fill(150);
    for p in other.pixels_mut().unwrap().chunks_exact_mut(4) { p[3] = 255; }
    other.damage(None);
    let before = h.shot();
    h.configure(json!({"specular": 1.0}));
    let after = h.shot();
    assert!(difference(&before, &after, 0, W / 2) > 5000);
    assert_eq!(difference(&before, &after, W / 2, W), 0, "Home's look must not reach another Silk layer");
    assert!(silk_report::apply(&mut h.engine, &json!({"home": {"silk": null}})));
    assert!(h.engine.redraw_wanted(), "clearing a paused override schedules a frame");
    assert_eq!(h.shot(), before, "clearing the report resets only its scoped wallpaper");
    h.configure(json!({"vignette": 1.0}));
    h.keep_data = true;
    drop(other);
    drop(h);
    let mut reconnected = HomeWall::new();
    assert_eq!(reconnected.shot(), scoped, "a fresh native connection restores the persisted owner look before its first Create");

}
