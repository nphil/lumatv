//! Renders of the GeForce NOW app's screens: the real layers (`ui::GfnUi`) on the renderer's host in Mesa's software GL, in the dark and
//! the light material at 100 % and 200 % text, written as PNG to `rustui/build/gfn-shots/<material>-<scale>/<screen>.png`, with the
//! facts a picture can be checked for: that the box art is on the tiles and the placeholder where it is not, that the focused row is
//! lit, that the explanation card is opaque, that the stream's window is clear for the video plane except for the status chip, that the
//! material is the one asked for.
//!
//! `GFN_SHOTS=all` renders every screen of `fixtures::every_screen`; the default renders the screens the checks need.
use crate::gfn::app::{App, Art, Screen};
use crate::gfn::fixtures::{every_screen, games};
use crate::gfn::layout::{self, H, W};
use crate::gfn::ui::GfnUi;
use crate::look::Mode;
use compositor_protocol::Event;
use lumatv_render::host::{Engine, HostConfig};
use lumatv_render::{Gles, now_ns};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use ui::{SceneOptions, Ui};

const REFRESH_NS: u64 = 16_666_667;

/// What a frame is, read back: premultiplied RGBA, top row first.
pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Picture {
    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * self.w + x) * 4) as usize;
        [self.rgba[o], self.rgba[o + 1], self.rgba[o + 2], self.rgba[o + 3]]
    }

    /// The mean of (r + g + b) / 3 over a rectangle, 0-255.
    pub fn mean(&self, x: u32, y: u32, w: u32, h: u32) -> f32 {
        let mut sum = 0u64;
        for yy in y..y + h {
            for xx in x..x + w {
                let p = self.px(xx, yy);
                sum += u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]);
            }
        }
        sum as f32 / (w * h) as f32 / 3.0
    }

    /// Written over black, which is what the panel shows where nothing is drawn.
    pub fn write_png(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut out = Vec::with_capacity(self.rgba.len());
        for p in self.rgba.chunks_exact(4) {
            out.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
        let mut e = png::Encoder::new(io::BufWriter::new(std::fs::File::create(path)?), self.w, self.h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().and_then(|mut w| w.write_image_data(&out)).map_err(io::Error::other)
    }
}

pub struct Shots {
    engine: Engine,
    pub page: GfnUi,
    events: Receiver<Event>,
    now: u64,
    shm: PathBuf,
}

/// A soft light wallpaper for the light material (the shipped one is the dark silk): a lavender-to-peach wash with two broad blooms.
fn light_wall() -> Vec<u8> {
    let mut px = Vec::with_capacity((W * H * 4.0) as usize);
    let blob = |x: f32, y: f32, cx: f32, cy: f32, r: f32| (1.0 - ((x - cx).hypot(y - cy) / r)).clamp(0.0, 1.0).powi(2);
    for y in 0..H as u32 {
        for x in 0..W as u32 {
            let (fx, fy) = (x as f32 / W, y as f32 / H);
            let t = (fx + fy) / 2.0;
            let (a, b, c) = (blob(fx, fy, 0.2, 0.25, 0.55), blob(fx, fy, 0.85, 0.8, 0.5), 0.0);
            let _ = c;
            let r = 236.0 + 14.0 * t - 14.0 * a + 6.0 * b;
            let g = 226.0 + 17.0 * t - 18.0 * a - 4.0 * b;
            let bl = 244.0 - 8.0 * t - 6.0 * a - 14.0 * b;
            px.extend_from_slice(&[r.round() as u8, g.round() as u8, bl.round() as u8, 255]);
        }
    }
    px
}

impl Shots {
    pub fn new(scale: f32, mode: Mode) -> io::Result<Self> {
        crate::rig::software_gl();
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let shm = std::env::temp_dir().join(format!("gfn-shots-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&shm)?;
        let gles = Gles::software_pbuffer(c"libEGL.so.1", W as u32, H as u32)?;
        let (mut engine, app_end) = Engine::new(
            gles,
            HostConfig { width: W as u32, height: H as u32, refresh_hz: 60.0, shm_dir: shm.clone(), direct: false, gpu_sync: false, idle_trim: false, pages: 0, reference: false, blit: false, external: false, system_z: i32::MAX },
        )?;
        // as the app runs it: a window that is clear where nothing is drawn
        engine.set_transparent(true);
        let (scene, _welcome, events) = ui::Scene::over_with(app_end, SceneOptions { shm_dir: Some(shm.clone()), ..SceneOptions::default() })?;
        engine.finish_handshake()?;
        let mut ui = Ui::new(scene);
        ui.enable_glyph_text().map_err(|e| io::Error::other(format!("{e}")))?;
        let wall = (mode == Mode::Light).then(light_wall);
        let mut land = |ui: &mut Ui| -> io::Result<()> {
            let began = Instant::now();
            loop {
                while let Ok(event) = events.try_recv() {
                    ui.scene.observe(&event);
                }
                if let Some(atlas) = ui.text_atlas() {
                    atlas.pump();
                }
                if ui.scene.has_pending() {
                    let _ = ui.try_commit()?;
                }
                engine.pump()?;
                engine.apply_waiting(now_ns());
                engine.tidy();
                if ui.glyph_text_settled() && !ui.scene.has_pending() && !engine.tidy_more() {
                    return Ok(());
                }
                if began.elapsed() > Duration::from_secs(20) {
                    return Err(io::Error::other("the glyph atlas did not land in 20 s"));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let mut page = GfnUi::build(ui, &crate::rig::assets(), scale, Some(mode), wall, &mut land)?;
        land(&mut page.ui)?;
        Ok(Self { engine, page, events, now: now_ns(), shm })
    }

    /// One refresh on: what the page staged is committed and taken in, and the frame is drawn.
    fn frame(&mut self) -> io::Result<()> {
        self.now += REFRESH_NS;
        while let Ok(event) = self.events.try_recv() {
            self.page.ui.scene.observe(&event);
        }
        if let Some(atlas) = self.page.ui.text_atlas() {
            atlas.pump();
        }
        if self.page.ui.scene.has_pending() {
            let _ = self.page.ui.try_commit()?;
        }
        if self.engine.pump()?.closed {
            return Err(io::Error::other("the engine's side of the toolkit closed"));
        }
        let cost = self.engine.draw(self.now);
        self.engine.frame_done(self.now, &cost);
        self.engine.tidy();
        Ok(())
    }

    /// The screen as the rules' state `app` puts it up, once everything staged has been drawn. Pictures the rules say are in the core's
    /// cache are waited for (the decoder is on its own threads), so the frame has them.
    pub fn shot(&mut self, app: &App) -> io::Result<Picture> {
        let want = if app.screen == Screen::Library {
            app.games.iter().skip(app.first_row * crate::gfn::app::LIBRARY_COLUMNS).take(crate::gfn::app::LIBRARY_COLUMNS * crate::gfn::app::GRID_ROWS).filter(|g| matches!(app.art.get(&g.image_url), Some(Art::File(_)))).count()
        } else {
            0
        };
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            self.page.show(app);
            if self.page.thumbs_loaded() >= want && !self.page.art_busy() {
                break;
            }
            if Instant::now() > end {
                return Err(io::Error::other(format!("{} of {want} pictures decoded in 20 s", self.page.thumbs_loaded())));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // the commit, the engine's apply and the draw are three steps of the frame loop; the picture is the one after they have all happened
        for _ in 0..4 {
            self.frame()?;
            self.page.show(app);
        }
        for _ in 0..2 {
            self.frame()?;
        }
        // the first page of the engine's ring, top row first (the window's own framebuffer is only written when the engine blits)
        Ok(Picture { w: W as u32, h: H as u32, rgba: self.engine.read_page(0) })
    }
}

impl Drop for Shots {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.shm);
    }
}

fn out_dir(mode: Mode, scale: f32) -> PathBuf {
    let name = format!("{}-{}", if mode == Mode::Dark { "dark" } else { "light" }, (scale * 100.0) as u32);
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../build/gfn-shots").join(name)
}

/// A game's box art: a 600 x 800 picture whose colour is the game's own, light at the top and dark at the foot, with a pale band
/// across it that the tile's cut must keep (the middle of the picture shows).
fn art_file(dir: &Path, i: usize) -> PathBuf {
    const HUES: [[f32; 3]; 6] = [[0.85, 0.25, 0.2], [0.2, 0.55, 0.85], [0.95, 0.7, 0.15], [0.3, 0.75, 0.4], [0.7, 0.35, 0.85], [0.15, 0.8, 0.8]];
    let c = HUES[i % 6];
    let (w, h) = (600u32, 800u32);
    let mut px = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        let t = 1.0 - 0.55 * y as f32 / h as f32;
        for x in 0..w {
            let band = y > h * 2 / 5 && y < h * 3 / 5 && x > w / 6 && x < w * 5 / 6;
            for ck in c {
                let v = if band { 0.5 + 0.5 * ck } else { ck * t * (0.9 + 0.1 * (x as f32 / w as f32)) };
                px.push((v.clamp(0.0, 1.0) * 255.0) as u8);
            }
        }
    }
    let path = dir.join(format!("art{i}.jpg"));
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 88).encode(&px, w as u16, h as u16, jpeg_encoder::ColorType::Rgb).expect("a JPEG");
    std::fs::write(&path, out).expect("the picture file");
    path
}

/// The library with the pictures of its first twelve tiles in the core's cache.
fn library_with_art(dir: &Path) -> App {
    let mut app = every_screen().into_iter().find(|(n, _)| *n == "library").map(|(_, a)| a).expect("the library state");
    for (i, g) in games(40).iter().enumerate().take(12) {
        app.art.insert(g.image_url.clone(), Art::File(art_file(dir, i).to_string_lossy().into_owned()));
    }
    app
}

/// The tile `i` of the first row at its centre, above the strip: where the picture shows.
fn tile_centre(m: &layout::Metrics, i: usize) -> (u32, u32) {
    let f = layout::tile_frame(m, i, 0).expect("a tile of the first row");
    ((f.x + f.w / 2.0) as u32, (f.y + f.h * 0.3) as u32)
}

fn near(p: [u8; 4], want: [f32; 3], tol: f32) -> bool {
    (0..3).all(|k| (f32::from(p[k]) - want[k] * 255.0).abs() <= tol)
}

#[test]
fn the_screens_render_in_both_materials_at_100_and_200_percent_with_the_art_on_the_tiles() {
    let all = std::env::var("GFN_SHOTS").is_ok_and(|v| v == "all");
    let art_dir = std::env::temp_dir().join(format!("gfn-shots-art-{}", std::process::id()));
    std::fs::create_dir_all(&art_dir).unwrap();
    let needed = ["library", "library-header", "library-empty", "settings-stats", "settings-help-stream", "detail-ready", "stream", "overlay", "signin"];
    for mode in [Mode::Dark, Mode::Light] {
        for scale in [1.0f32, 2.0] {
            let mut shots = Shots::new(scale, mode).expect("the renderer on software GL");
            let m = layout::Metrics::new(scale);
            let dir = out_dir(mode, scale);
            let tag = format!("{mode:?} at {}%", (scale * 100.0) as u32);

            // the library with its pictures: each tile shows its own, cut from the middle, with the strip and the title on it
            let with_art = shots.shot(&library_with_art(&art_dir)).expect("the library with art");
            with_art.write_png(&dir.join("library-art.png")).unwrap();
            let hues: [[f32; 3]; 6] = [[0.85, 0.25, 0.2], [0.2, 0.55, 0.85], [0.95, 0.7, 0.15], [0.3, 0.75, 0.4], [0.7, 0.35, 0.85], [0.15, 0.8, 0.8]];
            for (i, c) in hues.iter().enumerate() {
                let (x, y) = tile_centre(&m, i);
                // above the band (the band is the middle fifth of the picture), where the picture is its colour at about 0.8 of its brightness
                let p = with_art.px(x, y);
                let hit = (0..3).all(|k| {
                    let want = c[k] * 0.84 * 255.0;
                    (f32::from(p[k]) - want).abs() < 48.0
                });
                assert!(hit, "{tag}: tile {i} shows its own picture at {x},{y}: {p:?}, wanted about {:?}", c.map(|v| (v * 0.84 * 255.0) as u32));
                assert_eq!(p[3], 255, "{tag}: the tile is opaque");
            }
            // the focused tile has its ring: the accent at the tile's edge, which the others do not have
            let first = layout::tile_frame(&m, 0, 0).unwrap();
            let ring = with_art.px(first.x as u32 + 1, (first.y + first.h * 0.5) as u32);
            let next = layout::tile_frame(&m, 1, 0).unwrap();
            let plain = with_art.px(next.x as u32 + 1, (next.y + next.h * 0.5) as u32);
            assert_ne!(ring, plain, "{tag}: the focused tile is outlined and its neighbour is not");

            for (name, app) in every_screen() {
                if name.is_empty() || !(all || needed.contains(&name)) {
                    continue;
                }
                let pic = shots.shot(&app).unwrap_or_else(|e| panic!("{tag}: {name}: {e}"));
                pic.write_png(&dir.join(format!("{name}.png"))).unwrap();
                match name {
                    "library" => {
                        // no pictures yet: every tile shows its placeholder, which is not any of the pictures' colours
                        let (x, y) = tile_centre(&m, 0);
                        let p = pic.px(x, y);
                        assert!(!near(p, hues[0], 60.0) && p[3] == 255, "{tag}: the placeholder is not a picture: {p:?}");
                        assert_eq!(pic.px(5, 5)[3], 255, "{tag}: the wallpaper covers the window on every screen but the stream's");
                    }
                    "settings-stats" => {
                        let panel = layout::panel_frame(&m);
                        let mean = pic.mean(panel.x as u32 + 12, (panel.y + panel.h - 60.0) as u32, 100, 30);
                        match mode {
                            Mode::Dark => assert!(mean < 110.0, "{tag}: the dark panel is dark: {mean}"),
                            Mode::Light => assert!(mean > 150.0, "{tag}: the light panel is light: {mean}"),
                        }
                    }
                    "settings-help-stream" => {
                        // the explanation is opaque: its card's middle is the backdrop colour whatever is under it
                        let (card, _) = layout::help_card(&mut shots.page.ui.fonts, &m, 0);
                        let p = pic.px((card.x + card.w - 20.0) as u32, (card.y + card.h - 20.0) as u32);
                        let want = if mode == Mode::Dark { [0x12, 0x0E, 0x1A] } else { [0xFA, 0xF4, 0xED] };
                        assert!((0..3).all(|k| (i32::from(p[k]) - want[k]).abs() <= 24), "{tag}: the explanation's backdrop is opaque: {p:?}");
                    }
                    "stream" => {
                        // over the video plane the window is clear: nothing at the middle, the status chip at the lower left
                        assert_eq!(pic.px(960, 540)[3], 0, "{tag}: the middle of the stream window is transparent");
                        assert_eq!(pic.px(5, 5)[3], 0);
                        let chip = layout::status_frame(&mut shots.page.ui.fonts, &m);
                        assert!(pic.px((chip.x + 10.0) as u32, (chip.y + chip.h / 2.0) as u32)[3] > 200, "{tag}: the status chip is drawn");
                    }
                    "overlay" => {
                        let o = layout::overlay_frame(&mut shots.page.ui.fonts, &m);
                        let p = pic.px((o.x + 8.0) as u32, (o.y + o.h / 2.0) as u32);
                        assert_eq!(p[3], 255, "{tag}: the overlay's backdrop hides the video");
                    }
                    _ => {}
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&art_dir);
}
