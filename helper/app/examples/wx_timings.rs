//! What the weather view costs on the host, as the page asks for it: the real network and the production transport (an `openssl s_client` child
//! for every request), the radar and the map of a place, timed from the open to the first picture and to a whole screen, the loop stepped over
//! the radar's window with the next frame asked for ahead, zoomed and panned, closed; and the helper's resident size at each step. The disk's
//! cache is the one under a directory of its own, so a first round is cold and a second a service that was started again on what the first kept.
//!
//! ```text
//! cargo run --release -p lumatv-helper --example wx_timings [lat lon]      (Atlanta by default)
//! ```

use fetch::{Fetch, TlsFetch};
use lumatv_helper::http::Request;
use lumatv_helper::wxview::{OPEN_ZOOM, WxView};
use map::webmerc::LonLat;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn rss(label: &str) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let get = |k: &str| status.lines().find_map(|l| l.strip_prefix(k)).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
    println!("  resident {label:<34} {:>7} KiB (anonymous {:>7})", get("VmRSS:"), get("RssAnon:"));
}

fn request(method: &str, path: &str, query: &[(&str, String)], body: &str) -> Request {
    Request { method: method.into(), path: path.into(), query: query.iter().map(|(k, v)| ((*k).into(), v.clone())).collect(), headers: Vec::new(), body: body.as_bytes().to_vec(), peer: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) }
}

struct Page<'a> {
    wx: &'a WxView,
    cx: f64,
    cy: f64,
    epoch: u64,
}

impl Page<'_> {
    /// Asks as the shell does (again after a `202`) until the picture is there: how long it took and how many bytes it was.
    fn pic(&self, kind: &str, zoom: f64, t: i64) -> (Duration, usize, u16) {
        let began = Instant::now();
        loop {
            let q = vec![("k", kind.to_owned()), ("cx", self.cx.to_string()), ("cy", self.cy.to_string()), ("zoom", zoom.to_string()), ("t", t.to_string()), ("epoch", self.epoch.to_string())];
            let r = self.wx.handle(&request("GET", "/v1/wx/pic", &q, ""), &["pic"]);
            if r.status != 202 || began.elapsed() > Duration::from_secs(120) {
                return (began.elapsed(), r.body.len(), r.status);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn state(&self) -> Value {
        serde_json::from_slice(&self.wx.handle(&request("GET", "/v1/wx/state", &[], ""), &["state"]).body).unwrap_or(Value::Null)
    }
}

fn ms(d: Duration) -> String {
    format!("{:.0} ms", d.as_secs_f64() * 1e3)
}

fn main() {
    let args: Vec<f64> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (lat, lon) = (args.first().copied().unwrap_or(33.75), args.get(1).copied().unwrap_or(-84.39));
    let dir = tempfile::tempdir().expect("a directory");
    let tls: Arc<dyn Fetch> = Arc::new(TlsFetch::new("/usr/bin/openssl", "/etc/ssl/certs/ca-certificates.crt"));
    let centre = LonLat::new(lon, lat).world();
    println!("the weather view at {lat}, {lon}, zoom {OPEN_ZOOM}");
    rss("process start");
    for round in ["cold disk", "warm disk (a service started again)"] {
        println!("\n[{round}]");
        let wx = WxView::new(Arc::clone(&tls), dir.path());
        let mut page = Page { wx: &wx, cx: centre.x, cy: centre.y, epoch: 1 };
        let began = Instant::now();
        wx.handle(&request("POST", "/v1/wx/open", &[], &json!({"lat": lat, "lon": lon}).to_string()), &["open"]);
        let state = loop {
            let s = page.state();
            if s["indexed"] == true || began.elapsed() > Duration::from_secs(60) {
                break s;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        println!("  the frame list: {} ({} frames in the window)", ms(began.elapsed()), state["frames"].as_array().map_or(0, Vec::len));
        let frames: Vec<(i64, i64)> = state["frames"].as_array().map(|a| a.iter().filter_map(|f| Some((f[0].as_i64()?, f[1].as_i64()?))).collect()).unwrap_or_default();
        let now = state["now"].as_i64().unwrap_or(0);
        // the page asks for the backdrop, the map and the frame nearest the clock together; the serial send thread answers in that order
        let (_, b, s) = page.pic("backdrop", OPEN_ZOOM, 0);
        println!("  first picture (the backdrop): {} after the open ({:.1} MB, status {s})", ms(began.elapsed()), b as f64 / 1048576.0);
        let (d, b, s) = page.pic("map", OPEN_ZOOM, 0);
        println!("  the map for the screen: {} after the open, {} to ask for it ({:.1} MB, status {s})", ms(began.elapsed()), ms(d), b as f64 / 1048576.0);
        let cur = frames.iter().enumerate().min_by_key(|(_, f)| (f.0 - now).abs()).map(|(i, _)| i).unwrap_or(0);
        let (d, b, s) = page.pic("radar", OPEN_ZOOM, frames[cur].0);
        println!("  the radar frame nearest now: {} to ask for it ({:.2} MB, status {s}), {} after the open", ms(d), b as f64 / 1048576.0, ms(began.elapsed()));
        rss("view open: backdrop, map, a frame");
        // the loop: every frame in turn, the next asked for ahead, as the page plays it
        let (mut worst, mut total, mut n, mut bytes) = (Duration::ZERO, Duration::ZERO, 0u32, 0usize);
        let loop_began = Instant::now();
        for i in 0..frames.len() {
            let (_, b, _) = page.pic("radar", OPEN_ZOOM, frames[i].0);
            let next = frames[(i + 1) % frames.len()].0;
            let (d2, b2, _) = page.pic("radar", OPEN_ZOOM, next);
            worst = worst.max(d2);
            total += d2;
            n += 1;
            bytes += b + b2;
        }
        println!("  the loop over {} frames: {}, the next frame ready in {} on average and {} at worst ({:.1} MB handed out)", frames.len(), ms(loop_began.elapsed()), ms(total / n.max(1)), ms(worst), bytes as f64 / 1048576.0);
        rss("after a loop of the radar");
        for zoom in [8.0, 7.0, 10.0, OPEN_ZOOM] {
            page.epoch += 1;
            let t = Instant::now();
            let (_, bm, _) = page.pic("map", zoom, 0);
            let (_, bb, _) = page.pic("backdrop", zoom, 0);
            let (_, br, _) = page.pic("radar", zoom, frames[cur].0);
            println!("  zoom {zoom}: map, backdrop and a frame in {} ({:.1} + {:.1} + {:.2} MB)", ms(t.elapsed()), bm as f64 / 1048576.0, bb as f64 / 1048576.0, br as f64 / 1048576.0);
        }
        page.cx += 0.5 * 1920.0 / (256.0 * 2f64.powf(OPEN_ZOOM));
        page.epoch += 1;
        let t = Instant::now();
        let (_, bm, _) = page.pic("map", OPEN_ZOOM, 0);
        let (_, br, _) = page.pic("radar", OPEN_ZOOM, frames[cur].0);
        println!("  half a screen east: map and a frame in {} ({:.1} + {:.2} MB)", ms(t.elapsed()), bm as f64 / 1048576.0, br as f64 / 1048576.0);
        let held = wx.stats();
        println!("  held: {} cells ({:.1} MB), {} radar tiles ({:.1} MB), vector tiles {:.1} MB, {} rasters drawn, {} cells read from the disk", held.map.cells, held.map.bytes as f64 / 1048576.0, held.radar.tiles, held.radar.bytes as f64 / 1048576.0, held.map.engine.vector_bytes as f64 / 1048576.0, held.map.engine.rasters, held.map.disk_hits);
        rss("after zoom and pan");
        wx.handle(&request("POST", "/v1/wx/close", &[], "{}"), &["close"]);
        rss("closed, at once");
        std::thread::sleep(Duration::from_millis(2_500));
        rss("closed, 2.5 s later");
        // what the warming does every five minutes, once: the engines for a moment, nothing left
        let t = Instant::now();
        wx.warm(LonLat::new(lon, lat), &|| false);
        println!("  one round of the warming: {}", ms(t.elapsed()));
        std::thread::sleep(Duration::from_millis(2_500));
        rss("after the warming, 2.5 s later");
        drop(wx);
    }
}
