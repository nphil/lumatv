//! The weather view's radar, by tiles: the shell asks for one frame's mosaic of the few tiles that are on its screen, and only those are
//! fetched, read and kept.
//!
//! A frame is the radar at one time (an observed mosaic, or the model's forecast for it). A **tile** is one cell of the web map's grid at the
//! radar's own zoom (256 by 256 intensity bytes, `map::radar` says what the bytes mean); the shell picks the zoom so that its screen is one to
//! four of them, and asks for the frame on the cells it needs ([`RadarTiles::pic`]): the tiles are got from the provider
//! (`RadarProvider::field` on a square that is exactly the tile, which the sources answer from one request and keep on the disk by the
//! tile's address), coloured by the ramp of the theme and laid side by side into one premultiplied picture.
//!
//! **Memory is bounded and ends with the view.** The fields in memory are a [`ByteLru`] of [`FIELD_BUDGET`] bytes (48 tiles: the frame on
//! show, the next, and a few before them; an empty tile costs nothing), the queue holds only what a request of the current view asked,
//! and [`RadarTiles::close`] (or the drop) lets all of it go. **Cancelling:** every request carries the shell's view number (`epoch`); the
//! workers drop a queued tile whose number is older than the newest seen, so a view that moved or closed stops the fetching that has not
//! begun. A request already on the wire (at most [`WORKERS`], each limited by the provider's own deadline) finishes: the transport has no
//! way to call a child back, and the tile it brings is kept, since it is most likely wanted again.
//!
//! The frame list comes from the provider's index (`RadarProvider::index_for`, on a thread of its own, again every five minutes while the
//! view is open): what it names inside the window is what the shell may ask for, and a frame's `rev` is a hash of the name the source gave
//! it, so that a frame the model has replaced is asked for again.

use crate::wxcache::{ByteLru, fnv32};
use crate::wxpic::{CELL_PX, PicAnswer, PicQuery};
use map::radar::{FrameKind, FrameRef, RadarError, RadarIndex, RadarLevel, RadarProvider, Ramp, RampTheme, colorize};
use map::webmerc::{LonLat, World};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The window of frames the shell shows: this far back and ahead of the clock (seconds).
pub const BACK_S: i64 = 2 * 3_600;
pub const AHEAD_S: i64 = 6 * 3_600;
/// A tile's intensity bytes.
const FIELD_BYTES: usize = CELL_PX * CELL_PX;
/// How many bytes of tiles are kept in memory.
pub const FIELD_BUDGET: usize = 48 * FIELD_BYTES;
/// How long the frame list is kept before it is asked for again, and how long after a failure.
const REFRESH_EVERY: Duration = Duration::from_secs(300);
const RETRY_INDEX: Duration = Duration::from_secs(20);
/// How long a tile that could not be had is left alone.
const RETRY_TILE: Duration = Duration::from_secs(5);
/// An observed frame older than this is not the radar of now.
const STALE_AFTER_S: i64 = 3_600;
/// Tiles fetched at once, and how long an idle worker waits for the next before it goes.
pub const WORKERS: usize = 2;
const IDLE: Duration = Duration::from_millis(1_500);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    /// The frame, by `rev`.
    frame: u32,
    z: u8,
    x: u32,
    y: u32,
}

#[derive(Clone)]
enum Tile {
    /// Nothing falls there (the source has no tile, or it is dry): costs no memory.
    Empty,
    Field(Arc<[u8]>),
}

impl Tile {
    fn bytes(&self) -> usize {
        match self {
            Tile::Empty => 0,
            Tile::Field(f) => f.len(),
        }
    }
}

struct Task {
    key: Key,
    frame: Arc<FrameRef>,
    index: Arc<RadarIndex>,
    epoch: u64,
}

struct State {
    index: Option<Arc<RadarIndex>>,
    /// Why the last asking for the frame list failed (kept until one succeeds).
    error: Option<String>,
    next_refresh: Instant,
    refreshing: bool,
    attribution: String,
    lru: ByteLru<Key, Tile>,
    queue: VecDeque<Task>,
    inflight: HashSet<Key>,
    failed: HashMap<Key, Instant>,
    workers: usize,
    closed: bool,
    fetched: u64,
}

struct Shared {
    provider: Arc<dyn RadarProvider>,
    centre: LonLat,
    /// The newest view number any request has carried (the service's, shared with the map).
    epoch: Arc<AtomicU64>,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
    st: Mutex<State>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What the radar holds, for the tests and the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RadarStats {
    pub tiles: usize,
    pub bytes: usize,
    pub queued: usize,
    pub inflight: usize,
    pub workers: usize,
    pub fetched: u64,
    pub indexed: bool,
}

pub struct RadarTiles {
    shared: Arc<Shared>,
}

/// The place a tile's centre is, for the provider (a square of one tile about it is exactly the tile).
fn tile_centre(z: u8, x: u32, y: u32) -> LonLat {
    let n = f64::from(1u32 << z);
    World::new((f64::from(x) + 0.5) / n, (f64::from(y) + 0.5) / n).lonlat()
}

/// Lowers the calling thread to nice 10, so that the shell and the compositor run first. Best effort.
fn lower_priority() {
    // SAFETY: `gettid` and `setpriority` take and return plain integers and touch no memory of ours; a refusal is ignored.
    unsafe {
        let tid = libc::gettid() as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
    }
}

fn kind_code(kind: FrameKind) -> u8 {
    match kind {
        FrameKind::Observed => 0,
        FrameKind::Forecast => 1,
        FrameKind::Outlook => 2,
    }
}

impl RadarTiles {
    /// The radar of the place `centre`, from `provider`; `epoch` is the view number the service shares with the map.
    pub fn new(provider: Arc<dyn RadarProvider>, centre: LonLat, epoch: Arc<AtomicU64>, now: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        let st = State {
            index: None,
            error: None,
            next_refresh: Instant::now(),
            refreshing: false,
            attribution: String::new(),
            lru: ByteLru::new(FIELD_BUDGET),
            queue: VecDeque::new(),
            inflight: HashSet::new(),
            failed: HashMap::new(),
            workers: 0,
            closed: false,
            fetched: 0,
        };
        let me = Self { shared: Arc::new(Shared { provider, centre, epoch, now, st: Mutex::new(st), wake: Condvar::new() }) };
        me.refresh();
        me
    }

    /// Asks for the frame list when it is due: on a thread of its own, since the provider's answer is several requests.
    pub fn refresh(&self) {
        {
            let mut st = self.shared.lock();
            if st.closed || st.refreshing || Instant::now() < st.next_refresh {
                return;
            }
            st.refreshing = true;
        }
        let shared = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new().name("wx-index".into()).stack_size(512 * 1024).spawn(move || {
            lower_priority();
            let got = shared.provider.index_for(shared.centre, RadarLevel::REGIONAL);
            let mut st = shared.lock();
            st.refreshing = false;
            if st.closed {
                return;
            }
            match got {
                Ok(index) => {
                    st.attribution = shared.provider.attribution().to_owned();
                    st.index = Some(Arc::new(index));
                    st.error = None;
                    st.next_refresh = Instant::now() + REFRESH_EVERY;
                }
                Err(e) => {
                    st.error = Some(e.to_string());
                    st.next_refresh = Instant::now() + RETRY_INDEX;
                }
            }
        });
        if spawned.is_err() {
            self.shared.lock().refreshing = false;
        }
    }

    /// The frames of the window, oldest first, as `(time, kind, rev)`.
    fn window(shared: &Shared, index: &RadarIndex, now: i64) -> Vec<(i64, FrameKind, u32)> {
        index
            .frames
            .iter()
            .filter_map(|f| {
                let kind = shared.provider.frame_kind(f);
                (kind != FrameKind::Outlook && f.time_s >= now - BACK_S && f.time_s <= now + AHEAD_S).then(|| (f.time_s, kind, fnv32(&f.path)))
            })
            .collect()
    }

    /// The state the shell reads: what the radar can show, the frames inside the window, and how current they are.
    pub fn state(&self) -> Value {
        self.refresh();
        let now = (self.shared.now)();
        let st = self.shared.lock();
        let Some(index) = st.index.clone() else {
            let (status, why) = if let Some(e) = &st.error { ("unavailable", Some(e.clone())) } else { ("loading", None) };
            return json!({"status": status, "why": why, "have": 0, "want": 0, "frames": [], "indexed": false, "now": now});
        };
        let attribution = st.attribution.clone();
        let error = st.error.clone();
        drop(st);
        let frames = Self::window(&self.shared, &index, now);
        let newest = frames.iter().filter(|f| f.1 == FrameKind::Observed).map(|f| f.0).max();
        let (status, why) = match (newest, error) {
            (None, e) => ("unavailable", Some(e.unwrap_or_else(|| "the radar names no frame".to_owned()))),
            (Some(n), Some(e)) => ("stale", Some(format!("{e}; the newest frame is {} s old", now - n))),
            (Some(n), None) if now - n > STALE_AFTER_S => ("stale", Some(format!("the newest frame is {} s old", now - n))),
            _ => ("ready", None),
        };
        let list: Vec<Value> = frames.iter().map(|(t, kind, rev)| json!([t, kind_code(*kind), true, rev])).collect();
        json!({"status": status, "why": why, "newest": newest, "have": list.len(), "want": list.len(), "indexed": true, "frames": list, "attribution": attribution, "now": now})
    }

    /// The mosaic of one frame on the cells asked: the whole picture when every tile is in, else the tiles still to come are asked for and
    /// the answer says how many are.
    pub fn pic(&self, q: &PicQuery) -> PicAnswer {
        self.pic_with(q, true)
    }

    /// Whether the tiles of `q` are in (those that are not are asked for): nothing is composed. The background warming's, which wants the
    /// sources' own disk caches to have the tiles.
    pub fn prime(&self, q: &PicQuery) -> bool {
        matches!(self.pic_with(q, false), PicAnswer::Ready(_))
    }

    fn pic_with(&self, q: &PicQuery, compose_it: bool) -> PicAnswer {
        self.refresh();
        let shared = &self.shared;
        let n = 1i64 << q.zoom;
        let mut st = shared.lock();
        if st.closed {
            return PicAnswer::Unknown;
        }
        let Some(index) = st.index.clone() else {
            return PicAnswer::Coming { have: 0, want: q.cells(), failing: st.error.is_some() };
        };
        let Some(frame) = index.frames.iter().find(|f| f.time_s == q.t) else {
            return PicAnswer::Unknown;
        };
        let frame = Arc::new(frame.clone());
        let rev = fnv32(&frame.path);
        let now = Instant::now();
        // each cell in reading order: outside the world, a tile that is in, or one still to get
        let mut cells: Vec<Option<Tile>> = Vec::with_capacity(q.cells());
        let (mut have, mut failing) = (0usize, false);
        let mut asked = false;
        for y in q.y0..=q.y1 {
            for x in q.x0..=q.x1 {
                if i64::from(x) < 0 || i64::from(y) < 0 || i64::from(x) >= n || i64::from(y) >= n {
                    cells.push(Some(Tile::Empty));
                    have += 1;
                    continue;
                }
                let key = Key { frame: rev, z: q.zoom, x: x as u32, y: y as u32 };
                if let Some(tile) = st.lru.get(&key) {
                    cells.push(Some(tile.clone()));
                    have += 1;
                    continue;
                }
                cells.push(None);
                if st.inflight.contains(&key) {
                    continue;
                }
                match st.failed.get(&key) {
                    Some(until) if now < *until => failing = true,
                    _ => {
                        st.failed.remove(&key);
                        st.inflight.insert(key);
                        st.queue.push_back(Task { key, frame: Arc::clone(&frame), index: Arc::clone(&index), epoch: q.epoch });
                        asked = true;
                    }
                }
            }
        }
        if asked {
            Self::ensure_workers(shared, &mut st);
        }
        drop(st);
        if have < q.cells() {
            return PicAnswer::Coming { have, want: q.cells(), failing };
        }
        PicAnswer::Ready(if compose_it { compose(q, &cells) } else { Vec::new() })
    }

    fn ensure_workers(shared: &Arc<Shared>, st: &mut State) {
        // `inflight` holds what is queued and what is on the wire
        let wanted = WORKERS.min(st.inflight.len());
        while st.workers < wanted {
            st.workers += 1;
            let me = Arc::clone(shared);
            if std::thread::Builder::new().name("wx-radar".into()).stack_size(512 * 1024).spawn(move || work(&me)).is_err() {
                st.workers -= 1;
                break;
            }
        }
        shared.wake.notify_all();
    }

    /// What is held, for the tests and the log.
    pub fn stats(&self) -> RadarStats {
        let st = self.shared.lock();
        RadarStats { tiles: st.lru.len(), bytes: st.lru.bytes(), queued: st.queue.len(), inflight: st.inflight.len(), workers: st.workers, fetched: st.fetched, indexed: st.index.is_some() }
    }

    /// Lets everything go: the tiles, the queue, the frame list. The workers see it and end; one that is on the wire ends when it comes back.
    pub fn close(&self) {
        {
            let mut st = self.shared.lock();
            st.closed = true;
            st.lru.clear();
            st.queue = VecDeque::new();
            st.inflight = HashSet::new();
            st.failed = HashMap::new();
            st.index = None;
        }
        self.shared.wake.notify_all();
    }
}

impl Drop for RadarTiles {
    fn drop(&mut self) {
        self.close();
    }
}

fn work(shared: &Arc<Shared>) {
    lower_priority();
    loop {
        let task = {
            let mut st = shared.lock();
            loop {
                if st.closed {
                    st.workers -= 1;
                    return;
                }
                if let Some(t) = st.queue.pop_front() {
                    break t;
                }
                let (guard, waited) = shared.wake.wait_timeout(st, IDLE).unwrap_or_else(|e| e.into_inner());
                st = guard;
                if waited.timed_out() && st.queue.is_empty() {
                    st.workers -= 1;
                    return;
                }
            }
        };
        if task.epoch < shared.epoch.load(Ordering::Relaxed) {
            // the view has moved on: not begun, so dropped
            shared.lock().inflight.remove(&task.key);
            continue;
        }
        let got = fetch(shared, &task);
        let mut st = shared.lock();
        st.inflight.remove(&task.key);
        if st.closed {
            continue;
        }
        match got {
            Ok(tile) => {
                st.fetched += 1;
                st.failed.remove(&task.key);
                let bytes = tile.bytes();
                st.lru.insert(task.key, tile, bytes);
            }
            Err(_) => {
                st.failed.insert(task.key, Instant::now() + RETRY_TILE);
            }
        }
    }
}

fn fetch(shared: &Shared, t: &Task) -> Result<Tile, RadarError> {
    let field = shared.provider.field(&t.index, &t.frame, RadarLevel { zoom: t.key.z }, tile_centre(t.key.z, t.key.x, t.key.y), CELL_PX as u16)?;
    if usize::from(field.size) != CELL_PX || field.data.len() != FIELD_BYTES {
        return Err(RadarError::Malformed("a tile that is not 256 by 256".to_owned()));
    }
    if field.data.iter().all(|b| *b == 0) {
        return Ok(Tile::Empty);
    }
    Ok(Tile::Field(Arc::from(field.data)))
}

/// The cells laid side by side, coloured: premultiplied RGBA, sealed. A cell that is not there is nothing falling.
fn compose(q: &PicQuery, cells: &[Option<Tile>]) -> Vec<u8> {
    let (w, h) = (q.width(), q.height());
    let across = (q.x1 - q.x0 + 1) as usize;
    let mut strengths = vec![0u8; w * h];
    for (i, cell) in cells.iter().enumerate() {
        let Some(Tile::Field(f)) = cell else { continue };
        let (cx, cy) = ((i % across) * CELL_PX, (i / across) * CELL_PX);
        for row in 0..CELL_PX {
            let at = (cy + row) * w + cx;
            strengths[at..at + CELL_PX].copy_from_slice(&f[row * CELL_PX..(row + 1) * CELL_PX]);
        }
    }
    let ramp = Ramp::new(if q.dark { RampTheme::Dark } else { RampTheme::Light });
    let rgba = colorize(&strengths, &ramp);
    q.seal(rgba, true, cells.len(), cells.len())
}

/// A radar for the tests: ten frames ten minutes apart whose tile (x, y) is one strength that differs from its neighbours', except the tiles of
/// column 0, which are dry. The provider answers without a network.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use map::radar::Field;
    use std::sync::atomic::AtomicUsize;

    pub const T0: i64 = 1_792_548_000;

    pub struct Fake {
        pub calls: AtomicUsize,
        pub slow: Duration,
    }

    impl RadarProvider for Fake {
        fn index(&self) -> Result<RadarIndex, RadarError> {
            Ok(RadarIndex { generated_s: T0, host: "fake".into(), frames: (0..10).map(|i| FrameRef { time_s: T0 - 600 * (9 - i), path: format!("frame-{i}") }).collect() })
        }
        fn frame(&self, _: &RadarIndex, _: &FrameRef, _: RadarLevel, _: LonLat) -> Result<Vec<u8>, RadarError> {
            Err(RadarError::Other("fields only".into()))
        }
        fn field(&self, _: &RadarIndex, _: &FrameRef, level: RadarLevel, centre: LonLat, px: u16) -> Result<Field, RadarError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(self.slow);
            let w = centre.world();
            let (x, y) = ((w.x * f64::from(1u32 << level.zoom)).floor() as u32, (w.y * f64::from(1u32 << level.zoom)).floor() as u32);
            let v = if x == 0 { 0 } else { 1 + ((x * 61 + y * 29) % 250) as u8 };
            Ok(Field { size: px, data: vec![v; usize::from(px) * usize::from(px)] })
        }
        fn attribution(&self) -> &str {
            "fake"
        }
        fn id(&self) -> &str {
            "fake"
        }
    }

    impl Fake {
        pub fn new(slow_ms: u64) -> Arc<Self> {
            Arc::new(Self { calls: AtomicUsize::new(0), slow: Duration::from_millis(slow_ms) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{Fake, T0};
    use super::*;
    use crate::wxpic::{Kind, TRAILER};

    fn radar(slow_ms: u64) -> (RadarTiles, Arc<AtomicU64>, Arc<Fake>) {
        let fake = Fake::new(slow_ms);
        let epoch = Arc::new(AtomicU64::new(1));
        let r = RadarTiles::new(fake.clone(), LonLat::new(-84.39, 33.75), Arc::clone(&epoch), Arc::new(|| T0 + 120));
        (r, epoch, fake)
    }

    fn query(t: i64, z: u8, cells: (i32, i32, i32, i32), epoch: u64) -> PicQuery {
        PicQuery { kind: Kind::Radar, zoom: z, x0: cells.0, y0: cells.1, x1: cells.2, y1: cells.3, t, dark: true, epoch }
    }

    /// Asks until the answer is a picture (or the time is up).
    fn until_ready(r: &RadarTiles, q: &PicQuery) -> PicAnswer {
        let t = Instant::now();
        loop {
            match r.pic(q) {
                PicAnswer::Coming { .. } if t.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(5)),
                other => return other,
            }
        }
    }

    #[test]
    fn the_state_names_the_window_and_says_the_radar_is_current() {
        let (r, _, _) = radar(0);
        let t = Instant::now();
        let v = loop {
            let v = r.state();
            if v["indexed"] == true || t.elapsed() > Duration::from_secs(5) {
                break v;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(v["status"], "ready", "{v}");
        assert_eq!(v["frames"].as_array().unwrap().len(), 10);
        assert_eq!(v["frames"][9][0], T0);
        assert_eq!(v["frames"][9][1], 0, "an observed frame");
        assert_eq!(v["newest"], T0);
        assert_eq!(v["attribution"], "fake");
    }

    #[test]
    fn a_mosaic_is_the_tiles_asked_for_side_by_side_coloured_and_nothing_else_is_fetched() {
        let (r, _, fake) = radar(0);
        let q = query(T0, 6, (17, 25, 18, 26), 1);
        let PicAnswer::Ready(png) = until_ready(&r, &q) else { panic!("a picture") };
        assert_eq!(png.len(), 512 * 512 * 4 + TRAILER);
        assert_eq!(fake.calls.load(Ordering::SeqCst), 4, "the four tiles of the cells asked and no other");
        let px = |x: usize, y: usize| &png[(y * 512 + x) * 4..(y * 512 + x) * 4 + 4];
        // four different strengths in four blocks: each block is one colour and the blocks differ
        let colours = [px(10, 10), px(300, 10), px(10, 300), px(300, 300)];
        for (i, a) in colours.iter().enumerate() {
            assert!(a[3] > 0, "rain in block {i}");
            for b in &colours[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // asked again, it is one lookup
        let _ = r.pic(&q);
        assert_eq!(fake.calls.load(Ordering::SeqCst), 4);
        assert_eq!(r.stats().tiles, 4);
    }

    #[test]
    fn a_dry_tile_and_a_cell_outside_the_world_cost_no_memory() {
        let (r, _, _) = radar(0);
        // column 0 is dry; column -1 is outside the world
        let q = query(T0, 6, (-1, 20, 0, 20), 1);
        let PicAnswer::Ready(png) = until_ready(&r, &q) else { panic!("a picture") };
        assert!(png[..png.len() - TRAILER].iter().all(|b| *b == 0), "transparent");
        let s = r.stats();
        assert_eq!((s.tiles, s.bytes), (1, 0), "{s:?}");
    }

    #[test]
    fn the_cache_is_held_to_its_budget() {
        let (r, _, _) = radar(0);
        // 60 tiles of strength one frame each: more than 48 fit
        for x in 1..=10 {
            let q = query(T0, 6, (x, 1, x, 6), 1);
            assert!(matches!(until_ready(&r, &q), PicAnswer::Ready(_)));
        }
        let s = r.stats();
        assert_eq!(s.bytes, FIELD_BUDGET, "{s:?}");
        assert_eq!(s.tiles, 48);
    }

    #[test]
    fn a_view_that_moves_drops_the_tiles_not_yet_begun() {
        let (r, epoch, fake) = radar(150);
        // twelve tiles are asked for in view 1; two are fetched at a time, each takes 150 ms
        let q1 = query(T0, 6, (1, 1, 4, 3), 1);
        assert!(matches!(r.pic(&q1), PicAnswer::Coming { .. }));
        std::thread::sleep(Duration::from_millis(50));
        // the view moves: the next request carries view 2
        epoch.fetch_max(2, Ordering::SeqCst);
        let q2 = query(T0, 6, (9, 9, 9, 9), 2);
        let PicAnswer::Ready(_) = until_ready(&r, &q2) else { panic!("view 2's tile") };
        std::thread::sleep(Duration::from_millis(200));
        let s = r.stats();
        let calls = fake.calls.load(Ordering::SeqCst);
        assert!(calls <= WORKERS + 1, "{calls} tiles were fetched: the two on the wire and view 2's, none of the ten queued");
        assert_eq!((s.queued, s.inflight), (0, 0), "{s:?}");
    }

    #[test]
    fn closing_lets_everything_go_and_ends_the_workers() {
        let (r, _, _) = radar(0);
        let q = query(T0, 6, (1, 1, 3, 2), 1);
        assert!(matches!(until_ready(&r, &q), PicAnswer::Ready(_)));
        assert!(r.stats().bytes > 0);
        r.close();
        let s = r.stats();
        assert_eq!((s.tiles, s.bytes, s.queued, s.inflight, s.indexed), (0, 0, 0, 0, false));
        let t = Instant::now();
        while r.stats().workers > 0 && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(r.stats().workers, 0, "the workers ended");
        assert_eq!(r.pic(&q), PicAnswer::Unknown);
    }

    #[test]
    fn a_time_the_radar_has_no_frame_for_is_unknown() {
        let (r, _, _) = radar(0);
        let t = Instant::now();
        while !r.stats().indexed && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(r.pic(&query(T0 + 7, 6, (1, 1, 1, 1), 1)), PicAnswer::Unknown);
    }
}
