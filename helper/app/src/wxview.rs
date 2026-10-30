//! The full weather view's radar and base map, for the Rust shell: LumaShow's `map` crate (the IEM NEXRAD mosaic for the past, the HRRR
//! model for the hours ahead, RainViewer outside the United States, Open-Meteo's outlook, OpenFreeMap's vector tiles drawn here) run in
//! the helper, so that the shell fetches nothing from the internet and decodes nothing: it asks this module for pictures that are ready.
//!
//! | route | what |
//! |---|---|
//! | `POST /v1/wx/open {lat, lon, zoom?}` | starts the engines for a place (the radar's window is two hours back to six ahead) and answers the state |
//! | `GET /v1/wx/state` | `{status, have, want, frames: [[time, kind, ready]], extent, framePx, attribution, map}`: kind is 0 observed, 1 forecast, 2 outlook; time is unix seconds |
//! | `GET /v1/wx/frame?t=&dark=` | the frame at time `t` as a PNG of the rain's colours (`framePx` square, the extent of the state), 202 while it is coming |
//! | `GET /v1/wx/map?w=&h=&zoom=&dark=` | the base map as a PNG of the size asked, centred on the place (206 while tiles are still coming, with what there is) |
//! | `POST /v1/wx/close` | lets the engines go (their workers end when idle; the pictures kept on disk stay) |
//!
//! The engines keep what they fetched on disk (`<cache>/wx/`): the radar's tiles by address (a layer's address names its time or run, so a
//! tile is fetched once), the last radar set, the map's vector tiles. Frames are fetched four at a time (`RadarConfig::parallel`), nearest
//! the clock first. A frame's colours are painted here and cached as the PNG the shell gets, so a frame asked twice is one lookup.

use crate::http::{Request, Response, error_response};
use fetch::Fetch;
use map::engine::{MapConfig, MapEngine, MapStatus};
use map::radar::engine::unix_now;
use map::radar::iem::{IemHrrr, IemNexrad};
use map::radar::outlook::OpenMeteoOutlook;
use map::radar::{FrameKind, RadarConfig, RadarEngine, RadarLevel, RadarStatus, Ramp, RampTheme, RainViewer, TimelineProvider, TimelineSources, colorize};
use map::source::OpenFreeMap;
use map::store::DiskCache;
use map::style::MapPalette;
use map::webmerc::{GridZoom, LonLat, TILE_PX, View};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The radar's window: the frames this far back and ahead are fetched.
const BACK_S: i64 = 2 * 3_600;
const AHEAD_S: i64 = 6 * 3_600;
/// Frames of colours kept as PNGs, per theme.
const KEPT_FRAMES: usize = 48;
/// The map's tiles are 256 px; the base map is drawn at this zoom (a grid zoom counts half levels: zoom 8 is `GridZoom(16)`).
const MAP_ZOOM_MIN: u8 = 5;
const MAP_ZOOM_MAX: u8 = 9;
/// How long the engines stay up with nobody asking.
const IDLE_CLOSE: Duration = Duration::from_secs(600);
/// How long a request waits for what it asks for.
const WAIT_FRAME: Duration = Duration::from_millis(350);
const WAIT_MAP: Duration = Duration::from_millis(1_500);

struct Live {
    radar: RadarEngine,
    map: MapEngine,
    centre: LonLat,
    level: RadarLevel,
    last_used: Instant,
    /// The inflated fields taken from the radar, by time and revision.
    fields: HashMap<i64, (u32, Arc<[u8]>)>,
    /// The frames painted, by (time, revision, dark).
    pngs: HashMap<(i64, u32, bool), Arc<Vec<u8>>>,
    /// The map's raster tiles taken, by key.
    tiles: HashMap<map::webmerc::RasterKey, Arc<map::raster::RasterTile>>,
    palette_dark: Option<bool>,
}

pub struct WxView {
    fetch: Arc<dyn Fetch>,
    root: PathBuf,
    live: Mutex<Option<Live>>,
}

fn level_for(zoom: Option<u64>) -> RadarLevel {
    match zoom {
        Some(z) if z <= 4 => RadarLevel::WIDE,
        _ => RadarLevel::REGIONAL,
    }
}

impl WxView {
    pub fn new(fetch: Arc<dyn Fetch>, root: &Path) -> Arc<Self> {
        Arc::new(Self { fetch, root: root.join("wx"), live: Mutex::new(None) })
    }

    pub fn handle(&self, request: &Request, rest: &[&str]) -> Response {
        let get = request.method == "GET";
        let post = request.method == "POST";
        match rest {
            ["open"] if post => self.open(request),
            ["state"] if get => self.state(),
            ["frame"] if get => self.frame(request),
            ["map"] if get => self.map(request),
            ["close"] if post => {
                self.close();
                Response::json(200, &json!({"ok": true}))
            }
            _ => error_response(404, "not_found", "no such endpoint"),
        }
    }

    fn build(&self, centre: LonLat, level: RadarLevel) -> Live {
        let tls = Arc::clone(&self.fetch);
        let now: Arc<dyn Fn() -> i64 + Send + Sync> = Arc::new(unix_now);
        let tiles = DiskCache::open(self.root.join("radar-tiles"), 12 << 20).ok();
        let timeline = TimelineProvider::new(
            TimelineSources {
                observed: Arc::new(IemNexrad::new(Arc::clone(&tls), Arc::clone(&now)).with_cache(tiles.clone())),
                forecast: Arc::new(IemHrrr::new(Arc::clone(&tls)).with_cache(tiles)),
                outlook: Arc::new(OpenMeteoOutlook::new(Arc::clone(&tls), Arc::clone(&now))),
                elsewhere: Arc::new(RainViewer::new(Arc::clone(&tls))),
            },
            Arc::clone(&now),
        );
        let mut radar_cfg = RadarConfig::new(Arc::new(timeline));
        radar_cfg.cache = DiskCache::open(self.root.join("radar"), 4 << 20).ok();
        radar_cfg.max_held = 40;
        let radar = RadarEngine::new(radar_cfg, Arc::new(|| {}));
        let meta = DiskCache::open(self.root.join("map-meta"), 64 << 10).ok();
        let mut map_cfg = MapConfig::new(Arc::new(OpenFreeMap::new(Arc::clone(&tls), meta))).for_screen(1792.0, 720.0);
        map_cfg.cache_dir = Some(self.root.join("map"));
        map_cfg.disk_budget = 24 << 20;
        map_cfg.min_zoom = f64::from(MAP_ZOOM_MIN);
        map_cfg.max_zoom = f64::from(MAP_ZOOM_MAX);
        map_cfg.raster_budget = 24 << 20;
        let map = MapEngine::new(map_cfg, Arc::new(|| {}));
        let n = unix_now();
        radar.set_window(Some((n - BACK_S, n + AHEAD_S)));
        radar.request(centre, level);
        Live { radar, map, centre, level, last_used: Instant::now(), fields: HashMap::new(), pngs: HashMap::new(), tiles: HashMap::new(), palette_dark: None }
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
        let level = level_for(body.get("zoom").and_then(Value::as_u64));
        {
            let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            match live.as_mut() {
                Some(l) if l.centre == centre && l.level == level => {
                    l.last_used = Instant::now();
                    let n = unix_now();
                    l.radar.set_window(Some((n - BACK_S, n + AHEAD_S)));
                    l.radar.request(centre, level);
                }
                _ => *live = Some(self.build(centre, level)),
            }
        }
        self.state()
    }

    fn close(&self) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(l) = live.take() {
            l.radar.release();
        }
    }

    /// Runs `f` on the live engines (dropping them first when nobody has asked for ten minutes); `None` before `open`.
    fn with<R>(&self, f: impl FnOnce(&mut Live) -> R) -> Option<R> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if live.as_ref().is_some_and(|l| l.last_used.elapsed() > IDLE_CLOSE) {
            *live = None;
        }
        let l = live.as_mut()?;
        l.last_used = Instant::now();
        Some(f(l))
    }

    fn state(&self) -> Response {
        match self.with(state_json) {
            Some(v) => Response::json(200, &v),
            None => error_response(409, "not_open", "the radar has not been opened"),
        }
    }

    /// The frame at `t`, coloured, as a PNG of straight colours.
    fn frame(&self, request: &Request) -> Response {
        let dark = request.param("dark") != Some("0");
        let Some(t) = request.param("t").and_then(|t| t.parse::<i64>().ok()) else {
            return error_response(400, "bad_request", "t is needed");
        };
        let deadline = Instant::now() + WAIT_FRAME;
        loop {
            let got = self.with(|l| {
                take_fields(l);
                let rev = l.fields.get(&t).map(|f| f.0);
                match rev {
                    Some(rev) => {
                        if let Some(png) = l.pngs.get(&(t, rev, dark)) {
                            return Some(Arc::clone(png));
                        }
                        let field = Arc::clone(&l.fields[&t].1);
                        let side = usize::from(l.radar.frame_px());
                        let png = Arc::new(paint_frame(&field, side, dark));
                        if l.pngs.len() >= KEPT_FRAMES * 2 {
                            l.pngs.clear();
                        }
                        l.pngs.insert((t, rev, dark), Arc::clone(&png));
                        Some(png)
                    }
                    None => {
                        l.radar.want_fresh(&[t]);
                        None
                    }
                }
            });
            match got {
                None => return error_response(409, "not_open", "the radar has not been opened"),
                Some(Some(png)) => return Response { status: 200, headers: vec![("Content-Type", "image/png".into())], body: (*png).clone() },
                Some(None) if Instant::now() >= deadline => return Response::json(202, &json!({"coming": true})),
                Some(None) => std::thread::sleep(Duration::from_millis(15)),
            }
        }
    }

    /// The base map: tiles composed into a picture of `w` by `h` centred on the place.
    fn map(&self, request: &Request) -> Response {
        let num = |k: &str, d: f64| request.param(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
        let (w, h) = (num("w", 1792.0).clamp(64.0, 2048.0) as u32, num("h", 720.0).clamp(64.0, 1200.0) as u32);
        let zoom = num("zoom", 7.0).round().clamp(f64::from(MAP_ZOOM_MIN), f64::from(MAP_ZOOM_MAX)) as u8;
        let dark = request.param("dark") != Some("0");
        let deadline = Instant::now() + WAIT_MAP;
        loop {
            let got = self.with(|l| {
                if l.palette_dark != Some(dark) {
                    l.map.set_palette(if dark { MapPalette::calm_dark() } else { MapPalette::calm_light() });
                    l.palette_dark = Some(dark);
                    l.tiles.clear();
                }
                let view = View::new(l.centre.world(), f64::from(zoom), w as f32, h as f32);
                let keys = view.raster_keys(GridZoom(zoom * 2), 0.0);
                l.map.want(&keys);
                for t in l.map.take_ready() {
                    l.tiles.insert(t.key, t);
                }
                let have = keys.iter().filter(|k| l.tiles.contains_key(k)).count();
                (compose(&view, &keys, &l.tiles, if dark { MapPalette::calm_dark() } else { MapPalette::calm_light() }), have == keys.len(), matches!(l.map.status(), MapStatus::Failed(_) | MapStatus::Offline { .. }))
            });
            match got {
                None => return error_response(409, "not_open", "the radar has not been opened"),
                Some((rgba, complete, failing)) => {
                    if complete || failing || Instant::now() >= deadline {
                        let png = encode_png(w, h, &rgba);
                        return Response { status: if complete { 200 } else { 206 }, headers: vec![("Content-Type", "image/png".into())], body: png };
                    }
                    std::thread::sleep(Duration::from_millis(40));
                }
            }
        }
    }
}

/// Takes the inflated fields the worker has for the frames asked (`want_fresh`) into the kept set.
fn take_fields(l: &mut Live) {
    for (t, rev, field) in l.radar.take_fresh_revs() {
        l.fields.insert(t, (rev, field));
    }
}

fn state_json(l: &mut Live) -> Value {
    take_fields(l);
    let Some(s) = l.radar.state() else {
        return json!({"status": "loading", "have": 0, "want": 0, "frames": [], "indexed": false});
    };
    // a frame whose field was replaced (the model's next run) is fetched and inflated again when asked
    let frames: Vec<Value> = s
        .frames
        .iter()
        .map(|f| {
            let kind = match f.kind {
                FrameKind::Observed => 0,
                FrameKind::Forecast => 1,
                FrameKind::Outlook => 2,
            };
            json!([f.time_s, kind, f.ready, f.rev])
        })
        .collect();
    let (status, newest, why) = match &s.status {
        RadarStatus::Loading { .. } => ("loading", None, None),
        RadarStatus::Ready { newest_s } => ("ready", Some(*newest_s), None),
        RadarStatus::Stale { newest_s, why } => ("stale", Some(*newest_s), Some(format!("{why:?}"))),
        RadarStatus::Unavailable { why } => ("unavailable", None, Some(why.clone())),
    };
    json!({
        "status": status, "newest": newest, "why": why, "have": s.have, "want": s.want, "indexed": s.indexed,
        "frames": frames, "framePx": l.radar.frame_px(), "attribution": s.attribution, "now": unix_now(),
        "extent": {"x0": s.extent.min.x, "y0": s.extent.min.y, "x1": s.extent.max.x, "y1": s.extent.max.y},
        "centre": {"lon": l.centre.lon, "lat": l.centre.lat}, "zoom": l.level.zoom,
    })
}

/// A field of strengths as the PNG of its colours. The ramp's pixels are premultiplied; a PNG is straight, and the shell premultiplies what it
/// decodes, so they are made straight here.
fn paint_frame(field: &[u8], side: usize, dark: bool) -> Vec<u8> {
    let ramp = Ramp::new(if dark { RampTheme::Dark } else { RampTheme::Light });
    let mut rgba = colorize(field, &ramp);
    for px in rgba.chunks_exact_mut(4) {
        let a = u32::from(px[3]);
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    encode_png(side as u32, side as u32, &rgba)
}

/// The tiles drawn side by side into a picture of the view's size, on the palette's land colour.
fn compose(view: &View, keys: &[map::webmerc::RasterKey], tiles: &HashMap<map::webmerc::RasterKey, Arc<map::raster::RasterTile>>, palette: MapPalette) -> Vec<u8> {
    let (w, h) = (view.width as usize, view.height as usize);
    let mut out = vec![0u8; w * h * 4];
    let land = palette.water;
    for px in out.chunks_exact_mut(4) {
        px.copy_from_slice(&land);
    }
    for key in keys {
        let Some(tile) = tiles.get(key) else { continue };
        let (sx, sy) = view.to_screen(key.rect().min);
        let (x0, y0) = (sx.round() as i64, sy.round() as i64);
        for ty in 0..TILE_PX as i64 {
            let y = y0 + ty;
            if y < 0 || y >= h as i64 {
                continue;
            }
            for tx in 0..TILE_PX as i64 {
                let x = x0 + tx;
                if x < 0 || x >= w as i64 {
                    continue;
                }
                let s = ((ty * i64::from(TILE_PX) + tx) * 4) as usize;
                let d = (y as usize * w + x as usize) * 4;
                // premultiplied over the ground
                let a = 255 - u32::from(tile.rgba[s + 3]);
                for c in 0..3 {
                    out[d + c] = (u32::from(tile.rgba[s + c]) + u32::from(out[d + c]) * a / 255).min(255) as u8;
                }
                out[d + 3] = 255;
            }
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

    #[test]
    fn a_picture_of_rain_is_a_png_of_the_fields_size_and_nothing_where_it_is_dry() {
        let mut field = vec![0u8; 16 * 16];
        field[5 * 16 + 5] = 200;
        let bytes = paint_frame(&field, 16, true);
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().expect("a png");
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).expect("a frame");
        assert_eq!((info.width, info.height), (16, 16));
        assert_eq!(buf[0..4], [0, 0, 0, 0], "dry is transparent");
        assert!(buf[(5 * 16 + 5) * 4 + 3] > 0, "rain is not");
    }

    #[test]
    fn the_level_follows_the_zoom_asked() {
        assert_eq!(level_for(None), RadarLevel::REGIONAL);
        assert_eq!(level_for(Some(6)), RadarLevel::REGIONAL);
        assert_eq!(level_for(Some(4)), RadarLevel::WIDE);
    }

    #[test]
    fn tiles_are_composed_on_the_view_and_a_missing_tile_leaves_the_ground() {
        let view = View::new(LonLat::new(-84.39, 33.75).world(), 7.0, 300.0, 200.0);
        let keys = view.raster_keys(GridZoom(14), 0.0);
        assert!(!keys.is_empty());
        let ground = MapPalette::calm_dark().water;
        let out = compose(&view, &keys, &HashMap::new(), MapPalette::calm_dark());
        assert_eq!(out.len(), 300 * 200 * 4);
        assert_eq!(out[0..4], ground);
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

    /// `cargo test -p lumatv-helper --release --lib wxview::tests::timings -- --ignored --nocapture`: the real network, the host's openssl.
    #[test]
    #[ignore]
    fn timings() {
        let dir = tempfile::tempdir().unwrap();
        let tls: Arc<dyn Fetch> = fetch::KeepAlive::new(fetch::TlsFetch::new("/usr/bin/openssl", "/etc/ssl/certs/ca-certificates.crt"));
        for round in ["cold", "warm"] {
            // a new service on the same disk each time: what a restart finds
            let wx = WxView::new(Arc::clone(&tls), dir.path());
            let t0 = Instant::now();
            let r = wx.handle(&request("POST", "/v1/wx/open", &[], r#"{"lat": 33.75, "lon": -84.39}"#), &["open"]);
            assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
            let (mut first, mut all, mut frames) = (None, None, Vec::new());
            while t0.elapsed() < Duration::from_secs(60) {
                let v: Value = serde_json::from_slice(&wx.handle(&request("GET", "/v1/wx/state", &[], ""), &["state"]).body).unwrap();
                frames = v["frames"].as_array().cloned().unwrap_or_default();
                let ready = frames.iter().filter(|f| f[2] == true).count();
                if first.is_none() && ready > 0 {
                    first = Some(t0.elapsed());
                }
                if v["indexed"] == true && v["want"].as_u64().unwrap_or(0) > 0 && v["have"] == v["want"] {
                    all = Some(t0.elapsed());
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            // the first frame as a picture the shell can draw, and the newest observed
            let t = frames.iter().find(|f| f[1] == 0).map(|f| f[0].as_i64().unwrap()).unwrap_or(0);
            let t1 = Instant::now();
            let frame = wx.handle(&request("GET", "/v1/wx/frame", &[("t", &t.to_string())], ""), &["frame"]);
            let frame_ms = t1.elapsed();
            let t2 = Instant::now();
            let map = wx.handle(&request("GET", "/v1/wx/map", &[("w", "1792"), ("h", "876"), ("zoom", "8")], ""), &["map"]);
            println!("{round}: first frame {first:?}, all {all:?} ({} frames); a frame as a picture {frame_ms:?} ({} B, {}); the base map {:?} ({} B, {})", frames.len(), frame.body.len(), frame.status, t2.elapsed(), map.body.len(), map.status);
            if let (Ok(dump), "cold") = (std::env::var("WX_DUMP"), round) {
                // the pictures the shell would be given, for the host renders of the page (`WX_DUMP=<dir>`)
                let dir = Path::new(&dump);
                std::fs::create_dir_all(dir).unwrap();
                let state: Value = serde_json::from_slice(&wx.handle(&request("GET", "/v1/wx/state", &[], ""), &["state"]).body).unwrap();
                std::fs::write(dir.join("state.json"), serde_json::to_vec(&state).unwrap()).unwrap();
                std::fs::write(dir.join("map.png"), &map.body).unwrap();
                let now = state["now"].as_i64().unwrap();
                for f in state["frames"].as_array().unwrap().iter().filter(|f| f[2] == true) {
                    let t = f[0].as_i64().unwrap();
                    if t < now - BACK_S || t > now + AHEAD_S || f[1] == 2 {
                        continue;
                    }
                    for _ in 0..40 {
                        let r = wx.handle(&request("GET", "/v1/wx/frame", &[("t", &t.to_string())], ""), &["frame"]);
                        if r.status == 200 {
                            std::fs::write(dir.join(format!("frame-{t}.png")), &r.body).unwrap();
                            break;
                        }
                    }
                }
            }
            wx.close();
        }
    }
}
