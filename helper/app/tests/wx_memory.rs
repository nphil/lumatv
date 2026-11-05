//! The weather view's engines end with the view: a view is opened, zoomed and panned through every level of the map and a loop of the radar,
//! then closed, and what the helper holds in memory (the caches' bytes, the threads, the process's resident size) is what it was before. Its own
//! process, with one test, so that nothing else allocates while the resident size is read.

use fetch::{Fetch, FetchError, FetchLimits};
use lumatv_helper::http::Request;
use lumatv_helper::wxview::WxView;
use map::mvt::{GeomType, Layer, Point, Value, VectorTile};
use map::radar::{Field, FrameRef, RadarError, RadarIndex, RadarLevel, RadarProvider};
use map::source::{SourceError, TileSource};
use map::webmerc::{LonLat, TileKey};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

const T0: i64 = 1_792_548_000;

struct NoFetch;

impl Fetch for NoFetch {
    fn get(&self, _: &str, _: FetchLimits) -> Result<Vec<u8>, FetchError> {
        Err(FetchError::Offline)
    }
}

/// Every vector tile is a lake with sixty vertices of coast: a tile that is more than a flat colour to read and to draw.
struct Coast;

impl TileSource for Coast {
    fn fetch(&self, key: TileKey) -> Result<Vec<u8>, SourceError> {
        let mut layer = Layer::new("water", 4096);
        let seed = i64::from(key.x) * 31 + i64::from(key.y) * 17;
        let ring: Vec<Point> = (0..60)
            .map(|i| {
                let a = f64::from(i) / 60.0 * std::f64::consts::TAU;
                let r = 1_700.0 + 300.0 * (a * 9.0 + seed as f64).sin();
                [(2_048.0 + r * a.cos()) as i32, (2_048.0 + r * a.sin()) as i32]
            })
            .collect();
        layer.push(GeomType::Polygon, &[&ring], &[("class", Value::from("lake"))]).expect("a polygon");
        Ok(VectorTile { layers: vec![layer] }.encode())
    }

    fn id(&self) -> String {
        "coast".to_owned()
    }

    fn attribution(&self) -> &str {
        "test"
    }
}

/// Twelve frames ten minutes apart; every tile is a field of rain.
struct Rain;

impl RadarProvider for Rain {
    fn index(&self) -> Result<RadarIndex, RadarError> {
        Ok(RadarIndex { generated_s: T0, host: "rain".into(), frames: (0..12).map(|i| FrameRef { time_s: T0 - 600 * (11 - i), path: format!("frame-{i}") }).collect() })
    }

    fn frame(&self, _: &RadarIndex, _: &FrameRef, _: RadarLevel, _: LonLat) -> Result<Vec<u8>, RadarError> {
        Err(RadarError::Other("fields only".into()))
    }

    fn field(&self, _: &RadarIndex, f: &FrameRef, _: RadarLevel, c: LonLat, px: u16) -> Result<Field, RadarError> {
        let v = 40 + (f.time_s % 100) as u8 + (c.lon.abs() as u8 % 50);
        Ok(Field { size: px, data: vec![v; usize::from(px) * usize::from(px)] })
    }

    fn attribution(&self) -> &str {
        "rain"
    }

    fn id(&self) -> &str {
        "rain"
    }
}

fn request(method: &str, path: &str, query: &[(&str, String)], body: &str) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        query: query.iter().map(|(k, v)| ((*k).into(), v.clone())).collect(),
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
        peer: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
    }
}

/// The process's resident set and, of it, what is anonymous (the heap: the rest is the program's own pages, which fault in as the code first
/// runs and are not the view's).
fn resident_kb() -> (u64, u64) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let get = |k: &str| status.lines().find_map(|l| l.strip_prefix(k)).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap();
    (get("VmRSS:"), get("RssAnon:"))
}

/// The threads of the weather view that are still alive: its workers, the engines' and the writers'. The thread that trims the heap a moment after a
/// close holds nothing, and is looked for last.
fn threads_like(starts: &[&str]) -> Vec<String> {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|t| std::fs::read_to_string(t.ok()?.path().join("comm")).ok())
        .map(|n| n.trim().to_owned())
        .filter(|n| starts.iter().any(|s| n.starts_with(s)))
        .collect()
}

fn view_threads() -> Vec<String> {
    threads_like(&["wx-index", "wx-radar", "wx-persist", "wx-vector", "map-worker"])
}

fn pic(wx: &WxView, kind: &str, cx: f64, cy: f64, zoom: f64, t: i64, epoch: u64) -> usize {
    let began = Instant::now();
    loop {
        // a screen of 960 by 540: a quarter of the cells of the set's, which fills the cell cache all the same over the visit
        let q = vec![("k", kind.to_owned()), ("cx", cx.to_string()), ("cy", cy.to_string()), ("zoom", zoom.to_string()), ("w", "960".to_owned()), ("h", "540".to_owned()), ("t", t.to_string()), ("epoch", epoch.to_string())];
        let r = wx.handle(&request("GET", "/v1/wx/pic", &q, ""), &["pic"]);
        match r.status {
            200 => return r.body.len(),
            204 => return 0,
            202 if began.elapsed() < Duration::from_secs(60) => std::thread::sleep(Duration::from_millis(10)),
            other => panic!("{kind} at zoom {zoom}: status {other}"),
        }
    }
}

/// The page's visit: open, then the levels of the map `zooms` names with the coarse picture under it, a pan at each, and the radar's frames
/// in turn with the next one asked for; the returned number is the bytes of pictures handed out.
fn visit(wx: &WxView, zooms: &[f64]) -> usize {
    let r = wx.handle(&request("POST", "/v1/wx/open", &[], &json!({"lat": 33.75, "lon": -84.39}).to_string()), &["open"]);
    assert_eq!(r.status, 200);
    let atlanta = LonLat::new(-84.39, 33.75).world();
    let (cx, cy) = (atlanta.x, atlanta.y);
    let mut handed = 0;
    let mut epoch = 1;
    for &zoom in zooms {
        epoch += 1;
        for pan in 0..2 {
            let x = cx + f64::from(pan) * 0.01 / 2f64.powf(zoom - 5.0);
            epoch += 1;
            handed += pic(wx, "backdrop", x, cy, zoom, 0, epoch);
            handed += pic(wx, "map", x, cy, zoom, 0, epoch);
            for i in 0..3 {
                let t = T0 - 600 * (3 - i);
                handed += pic(wx, "radar", x, cy, zoom, t, epoch);
                handed += pic(wx, "radar", x, cy, zoom, t + 600, epoch);
            }
        }
    }
    handed
}

#[test]
fn the_view_gives_back_what_it_held_when_it_closes() {
    // glibc raises its thresholds as blocks are freed and then keeps tens of MB of free arena; musl, the set's allocator, hands a large block back
    // when it is freed. The thresholds are fixed, so that what is read is what the view holds.
    // SAFETY: mallopt only sets the allocator's parameters.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 128 * 1024);
        libc::mallopt(libc::M_TRIM_THRESHOLD, 128 * 1024);
    }
    let dir = tempfile::tempdir().unwrap();
    let wx = WxView::with_sources(Arc::new(NoFetch), dir.path(), Box::new(|| (Arc::new(Coast) as Arc<dyn TileSource>, Arc::new(Rain) as Arc<dyn RadarProvider>)), Arc::new(|| T0 + 120));
    // the threads of the view end a moment after the close, and the heap is trimmed again once they have
    let settle = |what: &str| {
        let began = Instant::now();
        while !view_threads().is_empty() && began.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(view_threads().is_empty(), "{what}: threads of the view are alive: {:?}", view_threads());
    };
    let before = resident_kb();
    let handed = visit(&wx, &[9.0, 8.0, 7.0]);
    let during = resident_kb();
    let held = wx.stats();
    wx.close();
    settle("after the first visit");
    let after = resident_kb();
    let closed = wx.stats();
    // and again: what is given back stays given back, nothing builds up from visit to visit
    visit(&wx, &[9.0]);
    wx.close();
    settle("after the second");
    let again = resident_kb();
    let said = format!("{} MB of pictures handed out; resident (anonymous) {before:?} KiB before, {during:?} open with {} MB of tiles ({} cells, {} fields), {after:?} closed, {again:?} after another visit", handed >> 20, (held.map.bytes + held.radar.bytes) >> 20, held.map.cells, held.radar.tiles);
    assert!(held.open && held.map.bytes > 8 << 20, "the map's cells are held while the view is open: {held:?}");
    assert!(held.map.bytes <= lumatv_helper::wxmap::CELL_BUDGET && held.radar.bytes <= lumatv_helper::wxradar::FIELD_BUDGET, "and held to their budgets");
    assert!(during.1 > before.1 + 8 * 1024, "the view costs memory while it is open, or this test measures nothing: {said}");
    assert!(!closed.open && closed.map.bytes == 0 && closed.radar.bytes == 0 && closed.map.cells == 0 && closed.radar.tiles == 0, "{closed:?}");
    // the program's own pages that the visit faulted in stay (a few MB of png, tiny-skia and the map's code): the allowance
    assert!(after.0 <= before.0 + 6 * 1024 && after.1 <= before.1 + 3 * 1024, "the resident size is back to where it was: {said}");
    assert!(again.0 <= before.0 + 6 * 1024 && again.1 <= before.1 + 3 * 1024, "and stays there: {said}");
    let began = Instant::now();
    while !threads_like(&["wx-"]).is_empty() && began.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(threads_like(&["wx-", "map-worker"]).is_empty(), "no thread of the view is left: {:?}", threads_like(&["wx-", "map-worker"]));
}
