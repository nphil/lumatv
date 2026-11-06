//! The full weather view's radar and base map, for the Rust shell: LumaShow's `map` crate (the IEM NEXRAD mosaic for the past, the HRRR
//! model for the hours ahead, RainViewer outside the United States, Open-Meteo's outlook, OpenFreeMap's vector tiles drawn here) run in
//! the helper, so that the shell fetches nothing from the internet and decodes nothing: it asks this module for pictures that are ready.
//!
//! | route | what |
//! |---|---|
//! | `POST /v1/wx/open {lat, lon}` | starts the view's engines for a place and answers the state |
//! | `GET /v1/wx/state` | `{status, why, have, want, frames: [[time, kind, ready, rev]], indexed, newest, attribution, now, centre}`: kind is 0 observed, 1 forecast; time is unix seconds; the frames are those of the window, two hours back to six ahead |
//! | `GET /v1/wx/pic?k=map\|backdrop\|radar&cx=&cy=&zoom=&w=&h=&dark=&epoch=[&t=]` | a mosaic of the cells that cover the view (`wxpic`): raw premultiplied RGBA and a trailer; 202 while its tiles are coming, 204 when the view has no such picture, 410 when the view has moved on |
//! | `GET /v1/wx/globe?lon=&lat=&size=&zoom=&dark=` | the globe seen from (lon, lat) as a PNG of `size` px square: NASA's rain (IMERG, about 6 h late) and satellite cloud over the earth, the night side shaded (206 while the textures are still coming) |
//! | `POST /v1/wx/close` | lets go of everything the view held: the engines, their caches in memory, the queued fetches |
//!
//! **Held only while the view is.** The engines of a view ([`wxmap::BaseMap`] for the map, [`wxradar::RadarTiles`] for the radar) are made by
//! `open` and gone at `close` (or after ten minutes without a request), with their caches (`wxmap::CELL_BUDGET`, `wxradar::FIELD_BUDGET`),
//! their queues and their threads, and the heap is trimmed. What stays is on the disk (`<cache>/wx/`): the radar's tiles by their address
//! (a layer's address names its time or run, so a tile is fetched once), the map's vector tiles and the cells drawn from them. The
//! background warming ([`WxView::prefetch`]) makes the engines for a moment every five minutes while the screen is on, fetches the
//! radar tiles of the opening view and draws and keeps its map cells when they are not on the disk, and lets them go: nothing of it stays in
//! memory, and opening the view finds its pictures on the disk.
//!
//! **Cancelling.** Every picture request carries the shell's view number; one older than the newest seen is refused (410), and the engines
//! drop the work they had queued for an older one. A fetch already on the wire finishes (the transport cannot call back a child process);
//! at most `wxvector::PARALLEL` vector tiles (and the engine's own, which the prefetch hands it when it is on the wire) and
//! `wxradar::WORKERS` radar tiles.

use crate::http::{Request, Response, error_response};
use crate::wxcache::trim_heap_after_close;
use crate::wxmap::BaseMap;
use crate::wxpic::{Kind, PicAnswer, PicQuery, ViewQuery};
use crate::wxradar::{RadarStats, RadarTiles};
use fetch::Fetch;
use map::globe::{CLOUD_H, CLOUD_W, GlobeConfig, GlobeEngine, RAIN_H, RAIN_W, SURFACE_H, SURFACE_W, Surface, class, sun_at};
use map::radar::engine::unix_now;
use map::radar::iem::{IemHrrr, IemNexrad};
use map::radar::outlook::OpenMeteoOutlook;
use map::radar::{RadarProvider, RainViewer, Ramp, RampTheme, TimelineProvider, TimelineSources, colorize};
use map::source::{OpenFreeMap, TileSource};
use map::store::DiskCache;
use map::style::MapPalette;
use map::webmerc::LonLat;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the engines stay up with nobody asking.
const IDLE_CLOSE: Duration = Duration::from_secs(600);
const PREFETCH_EVERY: Duration = Duration::from_secs(300);
/// The zoom the shell opens the radar page at (`radar_pics::OPEN_ZOOM`): what the background warming draws.
pub const OPEN_ZOOM: f64 = 9.0;
/// The screen the warming draws for.
const SCREEN: (f64, f64) = (1920.0, 1080.0);
/// How long a picture request waits for its tiles before it answers that they are coming: a tile that is on the disk is there in a few
/// milliseconds, and the shell's next ask (a tenth of a second later) would be a delay that is not the tiles'.
const WAIT_PIC: Duration = Duration::from_millis(120);
/// How long the warming may take, and how long it waits between looks.
const WARM_FOR: Duration = Duration::from_secs(180);
const WARM_LOOK: Duration = Duration::from_millis(150);

/// What a view's engines draw from: the base map's vector tiles and the radar's frames.
type Sources = Box<dyn Fn() -> (Arc<dyn TileSource>, Arc<dyn RadarProvider>) + Send + Sync>;

/// The engines of one view.
struct Live {
    radar: Arc<RadarTiles>,
    map: Arc<BaseMap>,
    /// The newest view number any request has carried.
    epoch: Arc<AtomicU64>,
    centre: LonLat,
    last_used: Instant,
    globe: Option<GlobeLive>,
}

impl Live {
    /// Lets everything the engines hold go.
    fn close(self) {
        self.epoch.store(u64::MAX, Ordering::SeqCst);
        self.radar.close();
        self.map.close();
    }
}

struct GlobeLive {
    engine: GlobeEngine,
    surface: Surface,
    rain: Option<Arc<[u8]>>,
    cloud: Option<Arc<[u8]>>,
    asked_at: i64,
}

/// What the service holds, for the tests and the log.
#[derive(Clone, Copy, Debug, Default)]
pub struct WxStats {
    pub open: bool,
    pub radar: RadarStats,
    pub map: crate::wxmap::BaseStats,
}

pub struct WxView {
    fetch: Arc<dyn Fetch>,
    sources: Sources,
    /// Unix seconds now: the radar's window is counted from it.
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
    root: PathBuf,
    live: Mutex<Option<Live>>,
}

impl WxView {
    /// The service over `fetch` (the production transport), keeping its files under `<root>/wx`.
    pub fn new(fetch: Arc<dyn Fetch>, root: &Path) -> Arc<Self> {
        let root = root.join("wx");
        // the radar engine's last-set cache of an earlier version
        let _ = std::fs::remove_dir_all(root.join("radar"));
        let (tls, dir) = (Arc::clone(&fetch), root.clone());
        let sources: Sources = Box::new(move || {
            let now: Arc<dyn Fn() -> i64 + Send + Sync> = Arc::new(unix_now);
            let tiles = DiskCache::open(dir.join("radar-tiles"), 12 << 20).ok();
            let timeline = TimelineProvider::new(
                TimelineSources {
                    observed: Arc::new(IemNexrad::new(Arc::clone(&tls), Arc::clone(&now)).with_cache(tiles.clone())),
                    forecast: Arc::new(IemHrrr::new(Arc::clone(&tls)).with_cache(tiles)),
                    outlook: Arc::new(OpenMeteoOutlook::new(Arc::clone(&tls), Arc::clone(&now))),
                    elsewhere: Arc::new(RainViewer::new(Arc::clone(&tls))),
                },
                now,
            );
            let meta = DiskCache::open(dir.join("map-meta"), 64 << 10).ok();
            (Arc::new(OpenFreeMap::new(Arc::clone(&tls), meta)), Arc::new(timeline))
        });
        Self::with_sources(fetch, &root, sources, Arc::new(unix_now))
    }

    /// The service over other sources and clock (a test's), with its files directly under `root`.
    pub fn with_sources(fetch: Arc<dyn Fetch>, root: &Path, sources: Sources, now: Arc<dyn Fn() -> i64 + Send + Sync>) -> Arc<Self> {
        Arc::new(Self { fetch, sources, now, root: root.to_path_buf(), live: Mutex::new(None) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Live>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Keeps the view's pictures on the disk while the screen is on: every five minutes the engines are made for a moment, the radar's tiles
    /// of the opening view are fetched and its map cells drawn if they are not on the disk, and the engines are let go. `place` gives the
    /// weather's place (none until there is one) and `screen_on` the screen.
    pub fn prefetch(self: &Arc<Self>, place: impl Fn() -> Option<(f64, f64)> + Send + 'static, screen_on: impl Fn() -> bool + Send + 'static, stop: crate::control::Stop) -> std::io::Result<std::thread::JoinHandle<()>> {
        let me = Arc::clone(self);
        std::thread::Builder::new().name("wx-prefetch".into()).stack_size(512 * 1024).spawn(move || {
            let mut last = Instant::now().checked_sub(PREFETCH_EVERY).unwrap_or_else(Instant::now);
            // five seconds a wait, which the stop ends at once (polled in 50 ms slices, it woke the helper 20 times a second)
            while !stop.sleep(Duration::from_secs(5)) {
                if last.elapsed() < PREFETCH_EVERY || !screen_on() {
                    continue;
                }
                let Some((lat, lon)) = place() else { continue };
                last = Instant::now();
                me.warm(LonLat::new(lon, lat), &|| stop.is_set());
            }
        })
    }

    /// One round of the warming (see [`WxView::prefetch`]); nothing when the view is up, whose engines have what it needs.
    pub fn warm(&self, centre: LonLat, stop: &dyn Fn() -> bool) {
        if self.lock().is_some() {
            return;
        }
        let live = self.build(centre);
        let w = centre.world();
        let view = |kind: Kind, t: i64| ViewQuery { kind, cx: w.x, cy: w.y, zoom: OPEN_ZOOM, w: SCREEN.0, h: SCREEN.1, t, dark: true, epoch: 1 };
        // the map: the cells that are not on the disk, the coarse picture first
        let maps: Vec<PicQuery> = [Kind::Backdrop, Kind::Map].into_iter().filter_map(|k| view(k, 0).plan()).collect();
        let mut radar: Vec<PicQuery> = Vec::new();
        let until = Instant::now() + WARM_FOR;
        while !stop() && Instant::now() < until {
            if radar.is_empty() {
                // the newest observed frame and the one after it: what the page shows first and plays next
                let state = live.radar.state();
                let frames: Vec<(i64, i64)> = state["frames"].as_array().map(|a| a.iter().filter_map(|f| Some((f[0].as_i64()?, f[1].as_i64()?))).collect()).unwrap_or_default();
                if let Some(i) = frames.iter().rposition(|f| f.1 == 0) {
                    radar = frames[i..frames.len().min(i + 2)].iter().filter_map(|f| view(Kind::Radar, f.0).plan()).collect();
                }
            }
            let radar_done = !radar.is_empty() && radar.iter().all(|q| live.radar.prime(q));
            let maps_done = maps.iter().all(|q| live.map.prime(q));
            if radar_done && maps_done {
                break;
            }
            std::thread::sleep(WARM_LOOK);
        }
        live.close();
        trim_heap_after_close();
    }

    pub fn handle(&self, request: &Request, rest: &[&str]) -> Response {
        let get = request.method == "GET";
        let post = request.method == "POST";
        match rest {
            ["open"] if post => self.open(request),
            ["state"] if get => self.state(),
            ["pic"] if get => self.pic(request),
            ["globe"] if get => self.globe(request),
            ["close"] if post => {
                self.close();
                Response::json(200, &json!({"ok": true}))
            }
            _ => error_response(404, "not_found", "no such endpoint"),
        }
    }

    fn build(&self, centre: LonLat) -> Live {
        let (source, provider) = (self.sources)();
        let epoch = Arc::new(AtomicU64::new(0));
        let radar = Arc::new(RadarTiles::new(provider, centre, Arc::clone(&epoch), Arc::clone(&self.now)));
        let map = Arc::new(BaseMap::new(source, &self.root, Arc::clone(&epoch)));
        Live { radar, map, epoch, centre, last_used: Instant::now(), globe: None }
    }

    fn open(&self, request: &Request) -> Response {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let (Some(lat), Some(lon)) = (body.get("lat").and_then(Value::as_f64), body.get("lon").and_then(Value::as_f64)) else {
            return error_response(400, "bad_request", "lat and lon are needed");
        };
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return error_response(400, "bad_request", "that place is not on the earth");
        }
        let centre = LonLat::new(lon, lat);
        let replaced = {
            let mut live = self.lock();
            match live.as_mut() {
                Some(l) if l.centre == centre => {
                    l.last_used = Instant::now();
                    l.radar.refresh();
                    None
                }
                _ => live.replace(self.build(centre)),
            }
        };
        if let Some(old) = replaced {
            old.close();
            trim_heap_after_close();
        }
        self.state()
    }

    /// Lets go of everything the view held.
    pub fn close(&self) {
        let taken = self.lock().take();
        if let Some(l) = taken {
            l.close();
            trim_heap_after_close();
        }
    }

    /// Runs `f` on the live engines (dropping them first when nobody has asked for ten minutes); `None` before `open`.
    fn with<R>(&self, f: impl FnOnce(&mut Live) -> R) -> Option<R> {
        let mut live = self.lock();
        if live.as_ref().is_some_and(|l| l.last_used.elapsed() > IDLE_CLOSE) {
            if let Some(old) = live.take() {
                old.close();
                trim_heap_after_close();
            }
        }
        let l = live.as_mut()?;
        l.last_used = Instant::now();
        Some(f(l))
    }

    fn state(&self) -> Response {
        match self.with(|l| {
            let mut v = l.radar.state();
            v["centre"] = json!({"lon": l.centre.lon, "lat": l.centre.lat});
            v
        }) {
            Some(v) => Response::json(200, &v),
            None => error_response(409, "not_open", "the radar has not been opened"),
        }
    }

    /// What the view's engines hold, for the tests and the log.
    pub fn stats(&self) -> WxStats {
        match self.lock().as_ref() {
            Some(l) => WxStats { open: true, radar: l.radar.stats(), map: l.map.stats() },
            None => WxStats::default(),
        }
    }

    /// A mosaic of the cells that cover the view (see `wxpic`).
    fn pic(&self, request: &Request) -> Response {
        let view = match ViewQuery::parse(request) {
            Ok(v) => v,
            Err(why) => return error_response(400, "bad_request", why),
        };
        // the engines, taken out of the lock: a picture is composed without holding it
        let Some((map, radar, epoch)) = self.with(|l| (Arc::clone(&l.map), Arc::clone(&l.radar), Arc::clone(&l.epoch))) else {
            return error_response(409, "not_open", "the radar has not been opened");
        };
        // a request of an older view than the newest is refused; a newer one moves the view on, which is what cancels the older work
        if view.epoch < epoch.fetch_max(view.epoch, Ordering::SeqCst) {
            return error_response(410, "stale_view", "the view has moved on");
        }
        let Some(q) = view.plan() else {
            return Response { status: 204, headers: Vec::new(), body: Vec::new() };
        };
        let began = Instant::now();
        let answer = loop {
            let got = match q.kind {
                Kind::Radar => radar.pic(&q),
                _ => map.pic(&q),
            };
            match got {
                PicAnswer::Coming { .. } if began.elapsed() < WAIT_PIC => std::thread::sleep(Duration::from_millis(4)),
                done => break done,
            }
        };
        match answer {
            PicAnswer::Ready(bytes) => Response { status: 200, headers: vec![("Content-Type", "application/octet-stream".into())], body: bytes },
            PicAnswer::Coming { have, want, failing } => Response::json(202, &json!({"coming": true, "have": have, "want": want, "failing": failing})),
            PicAnswer::Unknown => error_response(404, "no_frame", "the radar has no frame at that time"),
        }
    }

    /// The globe seen from a point: the textures the engine has (it fetches them on the first ask), drawn here.
    fn globe(&self, request: &Request) -> Response {
        let num = |k: &str, d: f64| request.param(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
        let (lon, lat) = (num("lon", 0.0), num("lat", 20.0).clamp(-85.0, 85.0));
        let size = num("size", 744.0).clamp(64.0, 1024.0) as usize;
        let zoom = num("zoom", 1.0).clamp(1.0, 4.0);
        let dark = request.param("dark") != Some("0");
        let tls = Arc::clone(&self.fetch);
        let root = self.root.clone();
        let got = self.with(|l| {
            let now = unix_now();
            let g = l.globe.get_or_insert_with(|| {
                let mut cfg = GlobeConfig::new(tls);
                cfg.cache = DiskCache::open(root.join("globe"), 2 << 20).ok();
                let engine = GlobeEngine::new(cfg, Arc::new(|| {}));
                engine.request(now);
                GlobeLive { engine, surface: Surface::bundled(), rain: None, cloud: None, asked_at: now }
            });
            // a globe kept open for a while is looked at again: the satellites have moved on
            if now - g.asked_at > 900 {
                g.engine.request(now);
                g.asked_at = now;
            }
            let fresh = g.engine.take_fresh();
            if let Some(r) = fresh.rain {
                g.rain = Some(r.data);
            }
            if let Some(c) = fresh.cloud {
                g.cloud = Some(c.data);
            }
            let complete = g.rain.is_some() && g.cloud.is_some();
            (render_globe(&g.surface, g.rain.as_deref(), g.cloud.as_deref(), sun_at(now), (lon, lat), (size, zoom), dark), complete)
        });
        match got {
            None => error_response(409, "not_open", "the radar has not been opened"),
            Some((rgba, complete)) => Response { status: if complete { 200 } else { 206 }, headers: vec![("Content-Type", "image/png".into())], body: encode_png(size as u32, size as u32, &rgba) },
        }
    }
}

/// The globe as seen from (`lon`, `lat`) degrees, orthographic, `size` px square, straight RGBA: the surface's classes in the map's colours, the
/// rain through the radar's ramp, the cloud as a lilac veil, the night side shaded from the sun's place and a thin glow at the limb.
fn render_globe(surface: &Surface, rain: Option<&[u8]>, cloud: Option<&[u8]>, sun: LonLat, (lon0, lat0): (f64, f64), (size, zoom): (usize, f64), dark: bool) -> Vec<u8> {
    let palette = if dark { MapPalette::calm_dark() } else { MapPalette::calm_light() };
    let ramp = Ramp::new(if dark { RampTheme::Dark } else { RampTheme::Light });
    let mut lut = [[0u8; 4]; 256];
    for (b, e) in lut.iter_mut().enumerate() {
        let px = colorize(&[b as u8], &ramp);
        *e = [px[0], px[1], px[2], px[3]];
    }
    let (l0, p0) = (lon0.to_radians(), lat0.to_radians());
    let (sl, sp) = (sun.lon.to_radians(), sun.lat.to_radians());
    let sunv = (sp.cos() * sl.cos(), sp.cos() * sl.sin(), sp.sin());
    // a zoom over 1 is a larger earth cut by the picture's square: the middle of it
    let r = (size as f64 / 2.0 - 3.0) * zoom;
    let c = size as f64 / 2.0;
    let mut out = vec![0u8; size * size * 4];
    let night = if dark { 0.55 } else { 0.7 };
    let glow = if dark { [0xC4u8, 0xA7, 0xE7] } else { [0x90, 0x7A, 0xA9] };
    let bilinear = |tex: &[u8], w: usize, h: usize, lon: f64, lat: f64| -> f32 {
        let fx = (lon + std::f64::consts::PI) / std::f64::consts::TAU * w as f64 - 0.5;
        let fy = ((std::f64::consts::FRAC_PI_2 - lat) / std::f64::consts::PI * h as f64 - 0.5).clamp(0.0, h as f64 - 1.0);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);
        let at = |x: f64, y: f64| f32::from(tex[(y as usize).min(h - 1) * w + (x.rem_euclid(w as f64)) as usize]);
        let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1.0, y0) * tx;
        let bottom = at(x0, y0 + 1.0) * (1.0 - tx) + at(x0 + 1.0, y0 + 1.0) * tx;
        top * (1.0 - ty) + bottom * ty
    };
    for py in 0..size {
        for px in 0..size {
            let (x, y) = ((px as f64 + 0.5 - c) / r, (c - (py as f64 + 0.5)) / r);
            let d2 = x * x + y * y;
            let cover = ((1.0 - d2.sqrt()) * r + 0.5).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            let z = (1.0 - d2.min(1.0)).sqrt();
            let sin_lat = (z * p0.sin() + y * p0.cos()).clamp(-1.0, 1.0);
            let lat = sin_lat.asin();
            let lon = l0 + (x).atan2(z * p0.cos() - y * p0.sin());
            let (col, row) = ((((lon.to_degrees() + 180.0).rem_euclid(360.0)) / 360.0 * SURFACE_W as f64) as usize % SURFACE_W, (((90.0 - lat.to_degrees()) / 180.0 * SURFACE_H as f64) as usize).min(SURFACE_H - 1));
            let base = match surface.class_of(col, row) {
                class::DEEP_WATER => palette.water,
                class::SHALLOW_WATER => [palette.water[0].saturating_add(7), palette.water[1].saturating_add(7), palette.water[2].saturating_add(10), 255],
                class::LAKE => palette.water,
                class::ICE => palette.ice,
                _ => palette.land,
            };
            let mut rgb = [f32::from(base[0]), f32::from(base[1]), f32::from(base[2])];
            if let Some(cl) = cloud {
                let v = bilinear(cl, CLOUD_W, CLOUD_H, lon, lat) / 255.0 * 0.45;
                for k in 0..3 {
                    rgb[k] = rgb[k] * (1.0 - v) + f32::from(glow[k]) * v;
                }
            }
            if let Some(rn) = rain {
                let b = bilinear(rn, RAIN_W, RAIN_H, lon, lat).round().clamp(0.0, 255.0) as usize;
                let p = lut[b];
                let a = f32::from(p[3]) / 255.0;
                for k in 0..3 {
                    // the ramp's pixels are premultiplied
                    rgb[k] = f32::from(p[k]) + rgb[k] * (1.0 - a);
                }
            }
            // the night side: a soft terminator about the sun's place
            let normal = (lat.cos() * lon.cos(), lat.cos() * lon.sin(), sin_lat);
            let lit = normal.0 * sunv.0 + normal.1 * sunv.1 + normal.2 * sunv.2;
            let day = ((lit + 0.10) / 0.26).clamp(0.0, 1.0);
            let shade = (night + (1.0 - night) * day * day * (3.0 - 2.0 * day)) as f32;
            // the limb: a thin glow of the atmosphere
            let rim = ((1.0 - z).powi(3) * 0.8) as f32;
            let i = (py * size + px) * 4;
            for k in 0..3 {
                out[i + k] = (rgb[k] * shade * (1.0 - rim) + f32::from(glow[k]) * rim * (0.4 + 0.6 * shade)).clamp(0.0, 255.0) as u8;
            }
            out[i + 3] = (cover * 255.0) as u8;
        }
    }
    out
}

fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    if let Ok(mut writer) = enc.write_header() {
        let _ = writer.write_image_data(rgba);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wxmap::testing::Water;
    use crate::wxpic::TRAILER;
    use crate::wxradar::testing::{Fake, T0};

    struct NoFetch;

    impl Fetch for NoFetch {
        fn get(&self, _: &str, _: fetch::FetchLimits) -> Result<Vec<u8>, fetch::FetchError> {
            Err(fetch::FetchError::Offline)
        }
    }

    fn service(dir: &Path) -> Arc<WxView> {
        WxView::with_sources(Arc::new(NoFetch), dir, Box::new(|| (Water::new(0) as Arc<dyn TileSource>, Fake::new(0) as Arc<dyn RadarProvider>)), Arc::new(|| T0 + 120))
    }

    fn request(method: &str, path: &str, query: &[(&str, &str)], body: &str) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            query: query.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
            peer: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        }
    }

    fn open(wx: &WxView) -> Response {
        wx.handle(&request("POST", "/v1/wx/open", &[], r#"{"lat": 33.75, "lon": -84.39}"#), &["open"])
    }

    /// Atlanta in world units.
    const CX: &str = "0.26703";
    const CY: &str = "0.41163";

    fn pic(wx: &WxView, kind: &str, zoom: &str, epoch: &str, t: i64) -> Response {
        let t = t.to_string();
        let mut q = vec![("k", kind), ("cx", CX), ("cy", CY), ("zoom", zoom), ("epoch", epoch)];
        if kind == "radar" {
            q.push(("t", &t));
        }
        wx.handle(&request("GET", "/v1/wx/pic", &q, ""), &["pic"])
    }

    /// Asks until the picture is there (a 202 is "ask again").
    fn pic_ready(wx: &WxView, kind: &str, zoom: &str, epoch: &str, t: i64) -> Response {
        let began = Instant::now();
        loop {
            let r = pic(wx, kind, zoom, epoch, t);
            if r.status != 202 || began.elapsed() > Duration::from_secs(20) {
                return r;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The trailer's size and level of a picture.
    fn trailer(r: &Response) -> (usize, usize, u8) {
        let t = &r.body[r.body.len() - TRAILER..];
        assert_eq!(&t[..4], b"WXP2");
        (u32::from_le_bytes(t[4..8].try_into().unwrap()) as usize, u32::from_le_bytes(t[8..12].try_into().unwrap()) as usize, t[20])
    }

    #[test]
    fn a_view_is_opened_asked_for_its_pictures_and_closed() {
        let dir = tempfile::tempdir().unwrap();
        let wx = service(dir.path());
        assert_eq!(wx.handle(&request("GET", "/v1/wx/state", &[], ""), &["state"]).status, 409, "not open yet");
        assert_eq!(open(&wx).status, 200);
        let began = Instant::now();
        let state = loop {
            let v: Value = serde_json::from_slice(&wx.handle(&request("GET", "/v1/wx/state", &[], ""), &["state"]).body).unwrap();
            if v["indexed"] == true || began.elapsed() > Duration::from_secs(5) {
                break v;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(state["frames"].as_array().unwrap().len(), 10);
        assert_eq!(state["centre"]["lat"], 33.75);
        let map = pic_ready(&wx, "map", "9", "1", 0);
        assert_eq!(map.status, 200);
        let (w, h, grid) = trailer(&map);
        assert_eq!((grid, map.body.len()), (18, w * h * 4 + TRAILER));
        assert!(w * h >= 1920 * 1080, "a screen of cells: {w} by {h}");
        let back = pic_ready(&wx, "backdrop", "9", "1", 0);
        assert_eq!((back.status, trailer(&back).2), (200, 14), "two zoom levels coarser");
        let rain = pic_ready(&wx, "radar", "9", "1", T0);
        assert_eq!(rain.status, 200);
        let (rw, rh, rz) = trailer(&rain);
        assert_eq!(rz, 6);
        assert!(rw * rh <= 3 * 2 * 256 * 256, "a frame is a few tiles");
        let s = wx.stats();
        assert!(s.open && s.map.cells > 0 && s.radar.tiles > 0, "{s:?}");
        // closed: everything is let go
        assert_eq!(wx.handle(&request("POST", "/v1/wx/close", &[], "{}"), &["close"]).status, 200);
        let s = wx.stats();
        assert!(!s.open && s.map.bytes == 0 && s.radar.bytes == 0, "{s:?}");
        assert_eq!(pic(&wx, "map", "9", "2", 0).status, 409);
    }

    #[test]
    fn an_older_view_is_refused_and_a_view_without_a_picture_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let wx = service(dir.path());
        open(&wx);
        let now = pic(&wx, "map", "9", "5", 0);
        assert!(now.status == 200 || now.status == 202, "{}", now.status);
        assert_eq!(pic(&wx, "map", "9", "4", 0).status, 410, "an older view");
        assert_eq!(pic(&wx, "backdrop", "6", "5", 0).status, 204, "no backdrop under zoom 7");
        assert_eq!(wx.handle(&request("GET", "/v1/wx/pic", &[("k", "map")], ""), &["pic"]).status, 400);
        // a radar frame the radar has not got
        let began = Instant::now();
        while !wx.stats().radar.indexed && began.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(pic(&wx, "radar", "9", "5", T0 + 7).status, 404);
    }

    #[test]
    fn opening_for_another_place_makes_the_engines_again_and_the_same_place_keeps_them() {
        let dir = tempfile::tempdir().unwrap();
        let wx = service(dir.path());
        open(&wx);
        let first = wx.lock().as_ref().map(|l| Arc::as_ptr(&l.map)).unwrap();
        open(&wx);
        assert_eq!(wx.lock().as_ref().map(|l| Arc::as_ptr(&l.map)), Some(first));
        wx.handle(&request("POST", "/v1/wx/open", &[], r#"{"lat": 51.5, "lon": -0.12}"#), &["open"]);
        assert_ne!(wx.lock().as_ref().map(|l| Arc::as_ptr(&l.map)), Some(first));
        assert_eq!(wx.handle(&request("POST", "/v1/wx/open", &[], r#"{"lat": 91, "lon": 0}"#), &["open"]).status, 400);
    }

    #[test]
    fn the_warming_leaves_nothing_in_memory_and_the_view_that_follows_finds_its_cells_on_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let wx = service(dir.path());
        let centre = LonLat::new(-84.39, 33.75);
        wx.warm(centre, &|| false);
        let s = wx.stats();
        assert!(!s.open, "the engines of the warming are not the view's: {s:?}");
        let cells = std::fs::read_dir(dir.path().join("map-raster")).unwrap().count();
        assert!(cells >= 30, "{cells} cells on the disk");
        // the view at the opening zoom: its cells are read, none drawn
        open(&wx);
        let w = centre.world();
        let (cx, cy) = (w.x.to_string(), w.y.to_string());
        let q = [("k", "map"), ("cx", cx.as_str()), ("cy", cy.as_str()), ("zoom", "9"), ("epoch", "1")];
        let began = Instant::now();
        let r = loop {
            let r = wx.handle(&request("GET", "/v1/wx/pic", &q, ""), &["pic"]);
            if r.status != 202 || began.elapsed() > Duration::from_secs(10) {
                break r;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(r.status, 200);
        let s = wx.stats();
        assert_eq!(s.map.engine.rasters, 0, "nothing was drawn for the view");
        assert!(s.map.disk_hits as usize >= 30, "{s:?}");
        // a second warming has nothing to draw: the view is up, so it does nothing at all
        wx.warm(centre, &|| false);
        wx.close();
        assert!(!wx.stats().open);
        // and with the view closed, one more finds every cell on the disk
        let files = std::fs::read_dir(dir.path().join("map-raster")).unwrap().count();
        wx.warm(centre, &|| false);
        assert_eq!(std::fs::read_dir(dir.path().join("map-raster")).unwrap().count(), files, "no cell was drawn again");
    }
}
