//! The weather view's memory, through the production model and surface, in a process of its own (the resident size is the process's): the radar page is
//! opened, zoomed and panned, and closed, with the helper played by this test at the sizes the real one cuts its pictures; the layers the scene holds
//! (a picture is a layer and its texture), the bytes of pictures the page holds and the process's memory go back to what they were before.
use compositor_protocol::{Event, Request};
use lumatv_tokens::Tokens;
use lumatv_ui::home::{Home, HomeSurface, grid};
use lumatv_ui::kit::screen::{Effect, Input};
use lumatv_ui::kit::{Engine, Kit, icon::Icons};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use ui::lucent::{material::MaterialTier, palette::Mode};

/// What the helper's pictures weigh at the opening view of Atlanta: the cells that cover the screen at zoom 9 (`wxpic`).
const MAP_BYTES: usize = 2304 * 1280 * 4;
const TRAILER: usize = 64;

/// The UI's assets in the repository.
fn module_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../compositor/assets/ui"))
}

fn status(field: &str) -> u64 {
    let text = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
    text.lines().find_map(|l| l.strip_prefix(field)?.trim_start_matches(':').split_whitespace().next()?.parse().ok()).unwrap_or(0)
}

/// The pixels of every layer's picture as the compositor reads them: the files the scene made in its directory (the harness's), their count and bytes.
fn layer_files() -> (usize, u64) {
    let prefix = format!("lumashow-ui-test-{}-", std::process::id());
    let mut found = (0, 0);
    for dir in std::fs::read_dir(std::env::temp_dir()).expect("the temporary directory").flatten() {
        if !dir.file_name().to_string_lossy().starts_with(&prefix) {
            continue;
        }
        for f in std::fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            if let Ok(meta) = f.metadata() {
                if meta.is_file() {
                    found = (found.0 + 1, found.1 + meta.len());
                }
            }
        }
    }
    found
}

#[derive(Clone, Copy, Debug)]
struct Memory {
    rss_kib: u64,
    anon_kib: u64,
}

fn memory() -> Memory {
    Memory { rss_kib: status("VmRSS"), anon_kib: status("RssAnon") }
}

fn half_up(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// The cells of the helper's grid that cover a view (`wxpic::ViewQuery::plan`): the level's code, how many cells across and down, and the world they cover.
fn cut(kind: &str, cx: f64, cy: f64, zoom: f64) -> (u8, (u32, u32), [f64; 4]) {
    let grid = half_up(zoom * 2.0).clamp(10.0, 22.0);
    let (code, cell) = match kind {
        "radar" => {
            let level = half_up(zoom - 3.0).clamp(2.0, 7.0);
            (level as u8, 2f64.powf(-level))
        }
        "backdrop" => ((grid - 4.0) as u8, 2f64.powf(-(grid - 4.0) / 2.0)),
        _ => (grid as u8, 2f64.powf(-grid / 2.0)),
    };
    let half = (1920.0 / 2.0 / (256.0 * 2f64.powf(zoom)), 1080.0 / 2.0 / (256.0 * 2f64.powf(zoom)));
    let span = |c: f64, h: f64| (((c - h) / cell).floor().max(0.0), ((c + h) / cell - 1e-9).floor());
    let ((x0, x1), (y0, y1)) = (span(cx, half.0), span(cy, half.1));
    (code, ((x1 - x0 + 1.0) as u32, (y1 - y0 + 1.0) as u32), [x0 * cell, y0 * cell, (x1 + 1.0) * cell, (y1 + 1.0) * cell])
}

/// The helper's answer to a picture's ask: raw premultiplied RGBA and the trailer (`wxpic::PicQuery::seal`).
fn picture(path: &str) -> Vec<u8> {
    let param = |k: &str| path.split(['?', '&']).find_map(|p| p.strip_prefix(&format!("{k}="))).unwrap_or_else(|| panic!("{k} in {path}")).to_owned();
    let (kind, cx, cy, zoom) = (param("k"), param("cx").parse().unwrap(), param("cy").parse().unwrap(), param("zoom").parse().unwrap());
    let (code, (nx, ny), world) = cut(&kind, cx, cy, zoom);
    let (w, h) = (nx * 256, ny * 256);
    let mut b = vec![0x5a_u8; (w * h * 4) as usize];
    b.reserve(TRAILER);
    b.extend_from_slice(b"WXP2");
    for v in [w, h, 1] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&((nx * ny) as u16).to_le_bytes());
    b.extend_from_slice(&((nx * ny) as u16).to_le_bytes());
    b.push(code);
    b.extend_from_slice(&[0; 3]);
    for v in world {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&[0; 8]);
    b
}

fn state() -> Value {
    let now = 1_800_000_000_i64;
    let frames: Vec<Value> = (0..17).map(|i| json!([now - 7_200 + i * 900, if i < 8 { 0 } else { 1 }, true, 0])).collect();
    json!({"status": "ready", "have": 17, "want": 17, "frames": frames, "attribution": "test", "now": now, "newest": now - 300})
}

struct Rig {
    home: Home,
    surface: HomeSurface,
    kit: Kit,
    engine: Engine,
    harness: ui::testing::Harness,
    now: u64,
    applied: u32,
    asked: usize,
    posted_close: usize,
}

impl Rig {
    fn new() -> Self {
        let harness = ui::testing::harness();
        let mut kit = Kit::new(harness.ui.scene.clone(), Tokens::shipped(), Mode::Dark, MaterialTier::T1, 1.0).expect("a kit on the recording compositor");
        kit.icons = Icons::new(&module_dir());
        let engine = Engine::new(&kit).expect("an engine");
        let mut home = Home::new(Tokens::shipped().json().clone(), grid::shipped(), json!({}));
        home.start(1000);
        home.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 1000);
        let days: Vec<Value> = [("Today", "rain", 71, 51), ("Sat", "cloud", 73, 51), ("Sun", "rain", 62, 48)]
            .iter()
            .map(|(d, k, hi, lo)| json!({"day": d, "kind": k, "hi": format!("{hi}\u{b0}"), "lo": format!("{lo}\u{b0}")}))
            .collect();
        let doc = json!({"doc": {"fetchedAt": 7, "lat": 33.75, "lon": -84.39, "place": "Atlanta", "current": {"time": "2026-10-30T15:00", "temp": 61.0, "feelsLike": 57.0, "code": 63, "isDay": true, "humidity": 53.0, "windMph": 12.0},
            "display": {"place": "Atlanta", "temp": "61\u{b0}", "cond": "Rain", "stats": "Feels like 57\u{b0}", "nowKind": "rain", "days": days}}, "state": "fresh"});
        home.update(&Input::Helper { topic: "weather".into(), data: doc }, 1006);
        let mut surface = HomeSurface::new(1000);
        surface.dirs(&module_dir(), Path::new("/nonexistent"));
        let mut rig = Self { home, surface, kit, engine, harness, now: 2_000, applied: 0, asked: 0, posted_close: 0 };
        rig.apply();
        rig
    }

    fn apply(&mut self) {
        self.surface.apply(&mut self.kit, &mut self.engine, &mut self.home, self.now).expect("Home drawn");
        self.applied += 1;
        if self.applied % 25 == 0 {
            self.present();
        }
    }

    /// What the surface asked of the compositor goes out, as the host's loop sends it every turn (a recording is kept of it, which is small), and the
    /// compositor shows the frame: the files of the pixels of layers that were deleted go then.
    fn present(&mut self) {
        let sent = self.harness.commit();
        if let Some(Request::Commit { tag }) = sent.last() {
            self.kit.ui.scene.observe(&Event::Frame { tag: *tag, presented_ns: 1, render_us: 1, missed: 0, backdrops: 0, gpu_mhz: 0, captures: 0, uploaded_bytes: 0, quality: 0 });
        }
    }

    /// Plays the helper: answers what the model asks of it, and what that brings, until nothing is asked.
    fn serve(&mut self, effects: Vec<Effect>) {
        let mut queue = effects;
        while let Some(e) = queue.pop() {
            let answer = match e {
                Effect::Request { tag, path, .. } if path == "/v1/wx/open" => Some(Input::Fetched { tag, status: 200, data: state() }),
                Effect::Fetch { tag, path } if path == "/v1/wx/state" => Some(Input::Fetched { tag, status: 200, data: state() }),
                Effect::Fetch { tag, path } if path.starts_with("/v1/wx/pic") => {
                    self.asked += 1;
                    Some(Input::Bytes { tag, status: 200, bytes: picture(&path) })
                }
                Effect::Post { path, .. } if path == "/v1/wx/close" => {
                    self.posted_close += 1;
                    None
                }
                _ => None,
            };
            if let Some(input) = answer {
                queue.extend(self.home.update(&input, self.now));
            }
        }
    }

    fn input(&mut self, input: &Input) {
        let fx = self.home.update(input, self.now);
        self.serve(fx);
        self.apply();
    }

    /// The clock goes on `ms`, as the host's loop does: a tick whenever one is due.
    fn run_for(&mut self, ms: u64) {
        let until = self.now + ms;
        while self.now < until {
            self.now += 20;
            if self.home.due_at().is_some_and(|due| due <= self.now) {
                self.input(&Input::Tick);
            } else {
                self.apply();
            }
        }
    }

    fn press(&mut self, name: &str) {
        for down in [true, false] {
            self.input(&Input::Key { name: name.into(), down, repeat: false });
            self.now += 30;
        }
        self.run_for(60);
    }

    fn wheel(&mut self, name: &str) {
        self.input(&Input::Key { name: name.into(), down: true, repeat: false });
        self.input(&Input::Key { name: name.into(), down: false, repeat: false });
        self.run_for(40);
    }

    fn layers(&self) -> usize {
        self.kit.ui.scene.live_layers()
    }

    /// The weather card, OK, and Right to the radar page: the keys of the host render (`tests/render_rust_weather.py`).
    fn open_radar(&mut self) {
        self.press("down");
        for _ in 0..6 {
            if self.home.focus.current().and_then(|id| self.home.widget_of(id)) == Some("weather") {
                break;
            }
            self.press("right");
        }
        self.press("ok");
        self.press("right");
        // the helper opens, the frames and the first pictures come, the loop begins
        self.run_for(1_500);
        assert!(self.home.radar.open, "the radar page is up");
    }

    /// Zooms in to the closest view and out, with the pointer about two places of the screen and with the keys, and lets the pictures of the last view come.
    fn zoom_and_pan(&mut self) {
        for _ in 0..4 {
            self.press("up");
        }
        self.input(&Input::Pointer { x: 400.0, y: 300.0, buttons: 0, visible: true });
        for _ in 0..6 {
            self.wheel("wheeldown");
        }
        self.input(&Input::Pointer { x: 1500.0, y: 800.0, buttons: 0, visible: true });
        for _ in 0..6 {
            self.wheel("wheelup");
        }
        for _ in 0..2 {
            self.press("down");
        }
        self.run_for(2_000);
    }
}

#[test]
fn the_radar_page_gives_back_its_layers_pictures_and_memory_when_it_closes() {
    // as `main` does, before the first picture
    lumatv_ui::host::memory::tune();
    let mut rig = Rig::new();
    // a first round, so that what a process keeps for good (the fonts, the glyphs, the allocator's pools) is not mistaken for what the page keeps
    rig.open_radar();
    rig.zoom_and_pan();
    rig.press("back");
    rig.run_for(1_000);

    let before = (rig.layers(), memory(), layer_files());
    assert_eq!(rig.home.radar.picture_bytes(), 0);
    rig.open_radar();
    let opened = (rig.layers(), memory(), layer_files(), rig.home.radar.picture_bytes());
    assert!(opened.3 >= MAP_BYTES, "the map, the backdrop and two frames are held: {} bytes", opened.3);
    assert!(opened.0 > before.0 && opened.0 <= before.0 + 60, "the page's layers: {} over {}", opened.0, before.0);
    assert!(opened.2.1 >= before.2.1 + MAP_BYTES as u64, "and their pixels are in files the compositor reads: {:?} over {:?}", opened.2, before.2);
    rig.zoom_and_pan();
    let zoomed = (rig.layers(), memory(), layer_files(), rig.home.radar.picture_bytes());
    assert!(zoomed.3 <= 72 << 20, "the pictures held are bounded: {} bytes", zoomed.3);
    assert!(zoomed.0 <= before.0 + 60 && zoomed.2.1 <= before.2.1 + (72 << 20), "a zoom and a pan make no layers that are never given back: {} layers, {:?}", zoomed.0, zoomed.2);
    let asked_open = rig.asked;
    assert!(asked_open >= 8, "the pictures of every view were asked for: {asked_open}");

    // closed: Back leaves the view; the page's pictures, asks and layers are let go with it and the helper is told
    let closes = rig.posted_close;
    rig.press("back");
    rig.run_for(1_000);
    assert!(!rig.home.radar.open);
    assert_eq!(rig.posted_close, closes + 1, "the helper was told to let go");
    assert_eq!(rig.home.radar.picture_bytes(), 0, "no picture is held");
    assert!(rig.home.radar.take_pics().is_empty() && rig.home.radar.take_released().is_empty());
    assert_eq!(rig.layers(), before.0, "every layer of the page's pictures is gone");
    rig.present();
    rig.present();
    assert_eq!(layer_files(), before.2, "and, once the compositor has shown the frame that deletes them, every file of their pixels");
    let asked = rig.asked;
    rig.run_for(3_000);
    assert_eq!(rig.asked, asked, "and nothing is asked for after it");

    let after = memory();
    let growth = |m: Memory, base: Memory| (m.rss_kib as i64 - base.rss_kib as i64, m.anon_kib as i64 - base.anon_kib as i64);
    let (rss, anon) = growth(after, before.1);
    if std::env::var_os("LUMATV_TEST_REPORT").is_some() {
        eprintln!(
            "weather view over the closed view: layers {} -> {} -> {} -> {}; pictures {} KiB open, {} KiB zoomed; layer files {} KiB -> {} KiB -> {} KiB; resident KiB open {:+}, zoomed {:+}, closed {rss:+} (anon {anon:+})",
            before.0, opened.0, zoomed.0, rig.layers(), opened.3 >> 10, zoomed.3 >> 10, before.2.1 >> 10, opened.2.1 >> 10, layer_files().1 >> 10, growth(opened.1, before.1).0, growth(zoomed.1, before.1).0
        );
    }
    assert!(rss <= 4_096, "the process's resident memory is back to within 4 MiB of what it was ({rss} KiB over)");
    assert!(anon <= 4_096, "and its heap to within 4 MiB ({anon} KiB over)");
}
