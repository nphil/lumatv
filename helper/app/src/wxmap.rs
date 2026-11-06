//! The weather view's base map, by cells: the shell asks for a mosaic of the cells of the map's raster grid that its screen needs, at one
//! grid zoom, and gets one picture.
//!
//! The drawing is LumaShow's `map` crate's: OpenFreeMap's vector tiles are got (kept gzipped on the disk by their address), read to what the
//! style draws and drawn by `tiny-skia` into 256 by 256 cells, by one worker. The crate's worker asks for a tile it lacks itself and one at a
//! time, which on the set is a child process and a TLS handshake for each of the 40 to 54 a screen of cold map takes: the tiles of the cells
//! that are wanted are fetched ahead of the worker, four at a time ([`crate::wxvector`]). This module is what holds the cells:
//!
//! - **Memory is bounded and ends with the view.** The finished cells are a [`ByteLru`] of [`CELL_BUDGET`] bytes (the engine's own raster cache has
//!   the same size and the same cells, so they are held once), and [`BaseMap::close`] (or the drop) lets all of it go, the engine's worker
//!   with it. Nothing is composed that was not asked for: a picture is built from the cells of one request, handed over, and forgotten.
//! - **A cell that was drawn once is kept on the disk** as a PNG (`<cache>/wx/map-raster`, [`DISK_BUDGET`], least recently used out first),
//!   under a name that holds the planet's version, the palette and a hash of the style's rules, so that a view that is opened again, or
//!   after the view was closed and everything in memory let go, is read back (about 1 ms a cell on the host) and not drawn again (12 to 18 ms
//!   a cell on the host, and the set's CPU is several times slower). A change of version, palette or style is a change of name.
//! - **Cancelling:** what a request still lacks is the engine's *wanted set*, the union of the requests asked within the last
//!   [`WANT_FOR`]; the engine drops work it has not begun for a cell that is no longer wanted. A request of an older view (`epoch`)
//!   lapses at once, and a view that closed has no wants. The one vector tile on the wire finishes.
//! - **Nothing is waited for.** A request answers with what there is: the picture when its cells are in, else how many are, and the same
//!   request again after a moment. After [`PARTIAL_AFTER`] a picture with the cells that are in (the ground colour where they are not) is
//!   given, marked as not complete, so that an offline map shows what the disk has.

use crate::wxcache::{ByteLru, fnv32};
use crate::wxpic::{CELL_PX, PicAnswer, PicQuery, TRAILER};
use crate::wxvector::{Prefetch, PrefetchStats};
use map::engine::{MapConfig, MapEngine, MapStats, MapStatus};
use map::raster::{self, RasterTile};
use map::source::TileSource;
use map::store::DiskCache;
use map::style::{MapPalette, MapStyle};
use map::webmerc::{GridZoom, RasterKey, TileKey};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// One finished cell: 256 by 256 premultiplied RGBA.
pub const CELL_BYTES: usize = CELL_PX * CELL_PX * 4;
/// How many bytes of finished cells stay in memory: 64 cells, a screen's worth (up to 54) and a coarse picture under it.
pub const CELL_BUDGET: usize = 64 * CELL_BYTES;
/// How many bytes of vector tiles stay in memory, decoded.
const VECTOR_BUDGET: usize = 8 << 20;
/// What the disk may hold of the vector tiles and of the drawn cells.
const VECTOR_DISK_BUDGET: u64 = 24 << 20;
pub const DISK_BUDGET: u64 = 24 << 20;
/// A request not asked again within this is no longer wanted.
const WANT_FOR: Duration = Duration::from_secs(3);
/// How long a picture waits for its cells before it is given as it is.
const PARTIAL_AFTER: Duration = Duration::from_millis(2_500);
/// The cells waiting to be written to the disk, at most.
const PERSIST_QUEUE: usize = 64;

type Cell = Arc<RasterTile>;

/// What a request is for: a picture, or only the drawing of its cells (the background warming: a cell that is on the disk is not read).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Compose,
    Prime,
}

/// A request, by the cells it is of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct PicId(u8, i32, i32, i32, i32);

/// One cell of a request.
#[derive(Clone)]
enum Slot {
    /// Outside the world: the ground.
    Ground,
    Have(Cell),
    Lacking(RasterKey),
}

/// A request that is not complete. It holds the cells it has (shared with the cache, not copies), so that a request for a screen of cells
/// is not kept from completing by the cache's budget: the cache is for the requests to come.
struct Want {
    id: PicId,
    /// The cells in reading order.
    slots: Vec<Slot>,
    /// The middle of its cells: the engine is asked for the ones near it first.
    centre: (f64, f64),
    epoch: u64,
    asked: Instant,
    since: Instant,
}

impl Want {
    fn lacking(&self) -> impl Iterator<Item = RasterKey> + '_ {
        self.slots.iter().filter_map(|s| if let Slot::Lacking(k) = s { Some(*k) } else { None })
    }
}

struct State {
    dark: Option<bool>,
    lru: ByteLru<RasterKey, Cell>,
    wants: Vec<Want>,
    /// What the engine was last told to want.
    union: Vec<RasterKey>,
    persist: VecDeque<(RasterKey, bool, u32, Cell)>,
    persister: bool,
    closed: bool,
    disk_hits: u64,
}

struct Shared {
    engine: MapEngine,
    /// The vector tiles' source as the engine has it: the real one, fetched several at a time ahead of the engine's worker.
    prefetch: Arc<Prefetch>,
    source: Arc<dyn TileSource>,
    epoch: Arc<AtomicU64>,
    disk: Option<DiskCache>,
    st: Mutex<State>,
    #[cfg(test)]
    probe: Arc<testing::Probe>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[cfg(not(test))]
    fn now(&self) -> Instant {
        Instant::now()
    }

    /// The tests' clock moves only when they move it.
    #[cfg(test)]
    fn now(&self) -> Instant {
        self.probe.now()
    }
}

/// What the base map holds, for the tests and the log.
#[derive(Clone, Copy, Debug, Default)]
pub struct BaseStats {
    pub cells: usize,
    pub bytes: usize,
    pub wants: usize,
    pub queued_for_disk: usize,
    pub disk_hits: u64,
    pub engine: MapStats,
    pub vector: PrefetchStats,
}

pub struct BaseMap {
    shared: Arc<Shared>,
}

fn palette(dark: bool) -> MapPalette {
    if dark { MapPalette::calm_dark() } else { MapPalette::calm_light() }
}

/// A hash of what draws a cell besides the planet: the palette and the rules at every grid zoom the view uses.
fn style_tag(p: MapPalette) -> u32 {
    let style = MapStyle::new(p);
    let rules: Vec<_> = (8u8..=22).map(|g| style.passes(f64::from(g) / 2.0)).collect();
    fnv32(&format!("{p:?}{rules:?}"))
}

fn encode(rgba: &[u8]) -> Option<Vec<u8>> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for p in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&p[..3]);
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, CELL_PX as u32, CELL_PX as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut w = enc.write_header().ok()?;
    w.write_image_data(&rgb).ok()?;
    drop(w);
    Some(out)
}

/// A cell read back: the cells are always opaque, so the PNG holds RGB.
fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if (info.width, info.height) != (CELL_PX as u32, CELL_PX as u32) || info.color_type != png::ColorType::Rgb || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let mut rgba = Vec::with_capacity(CELL_BYTES);
    for p in buf[..info.buffer_size()].chunks_exact(3) {
        rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
    }
    (rgba.len() == CELL_BYTES).then_some(rgba)
}

fn lower_priority() {
    // SAFETY: `gettid` and `setpriority` take and return plain integers and touch no memory of ours; a refusal is ignored.
    unsafe {
        let tid = libc::gettid() as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
    }
}

impl BaseMap {
    /// The base map over `source`, keeping its files under `root` (`<root>/map` for the vector tiles, `<root>/map-raster` for the cells).
    pub fn new(source: Arc<dyn TileSource>, root: &Path, epoch: Arc<AtomicU64>) -> Self {
        // opened before the engine's own handle writes anything: opening removes what a cut write left
        let prefetch = Prefetch::new(Arc::clone(&source), DiskCache::open(root.join("map"), VECTOR_DISK_BUDGET).ok());
        let mut cfg = MapConfig::new(Arc::clone(&prefetch) as Arc<dyn TileSource>);
        cfg.cache_dir = Some(root.join("map"));
        cfg.disk_budget = VECTOR_DISK_BUDGET;
        cfg.vector_budget = VECTOR_BUDGET;
        cfg.raster_budget = CELL_BUDGET;
        cfg.min_zoom = 5.0;
        cfg.max_zoom = 11.0;
        let st = State { dark: None, lru: ByteLru::new(CELL_BUDGET), wants: Vec::new(), union: Vec::new(), persist: VecDeque::new(), persister: false, closed: false, disk_hits: 0 };
        #[cfg(test)]
        let probe = Arc::new(testing::Probe::new());
        #[cfg(test)]
        let waker: map::Waker = {
            let probe = Arc::clone(&probe);
            Arc::new(move || probe.woke())
        };
        #[cfg(not(test))]
        let waker: map::Waker = Arc::new(|| {});
        let shared = Shared {
            engine: MapEngine::new(cfg, waker),
            prefetch,
            source,
            epoch,
            disk: DiskCache::open(root.join("map-raster"), DISK_BUDGET).ok(),
            st: Mutex::new(st),
            #[cfg(test)]
            probe,
        };
        Self { shared: Arc::new(shared) }
    }

    fn file(tag: u32, dark: bool, k: RasterKey) -> String {
        format!("r-{tag:08x}-{}-{}-{}-{}.png", u8::from(dark), k.grid.0, k.x, k.y)
    }

    fn tag(&self, dark: bool) -> u32 {
        fnv32(&self.shared.source.id()) ^ style_tag(palette(dark)).rotate_left(16)
    }

    /// Takes what the engine has finished into memory and into the requests that lack it, and sets the drawn cells to be written to the disk.
    fn take_ready(&self, st: &mut State, tag: u32) {
        let Some(dark) = st.dark else { return };
        for cell in self.shared.engine.take_ready() {
            let bytes = cell.rgba.len();
            st.lru.insert(cell.key, Arc::clone(&cell), bytes);
            for w in &mut st.wants {
                for slot in &mut w.slots {
                    if matches!(slot, Slot::Lacking(k) if *k == cell.key) {
                        *slot = Slot::Have(Arc::clone(&cell));
                    }
                }
            }
            if self.shared.disk.is_some() {
                if st.persist.len() >= PERSIST_QUEUE {
                    st.persist.pop_front();
                }
                st.persist.push_back((cell.key, dark, tag, cell));
            }
        }
        if !st.persist.is_empty() && !st.persister {
            st.persister = true;
            let shared = Arc::clone(&self.shared);
            if std::thread::Builder::new().name("wx-persist".into()).stack_size(512 * 1024).spawn(move || persist(&shared)).is_err() {
                st.persister = false;
            }
        }
    }

    /// Tells the engine what is wanted now: the cells every request still asked for lacks, the coarser pictures first. `want` twice, the first
    /// to forget what the engine counts as handed over, so that a cell it still has is handed over again.
    fn rewant(&self, st: &mut State) {
        let current = self.shared.epoch.load(Ordering::Relaxed);
        let now = self.shared.now();
        st.wants.retain(|w| w.epoch >= current && now.duration_since(w.asked) < WANT_FOR);
        let mut order: Vec<&Want> = st.wants.iter().collect();
        order.sort_by_key(|w| w.id.0);
        let mut union: Vec<RasterKey> = Vec::new();
        for w in order {
            let mut lacking: Vec<RasterKey> = w.lacking().collect();
            lacking.sort_by(|a, b| {
                let d = |k: &RasterKey| (f64::from(k.x) - w.centre.0).hypot(f64::from(k.y) - w.centre.1);
                d(a).total_cmp(&d(b))
            });
            for k in lacking {
                if !union.contains(&k) {
                    union.push(k);
                }
            }
        }
        if union != st.union {
            // the vector tiles those cells are drawn from, in the order the engine will want them, are fetched ahead of it: told before the engine
            // starts, so that the tile it asks for first is the one the prefetch has begun and not one it fetches itself beside it
            let mut seen = std::collections::HashSet::new();
            let tiles: Vec<TileKey> = union.iter().flat_map(|k| raster::source_tiles_for(*k, raster::source_zoom_for(k.grid))).filter(|t| seen.insert(*t)).collect();
            self.shared.prefetch.want(&tiles);
            self.shared.engine.want(&[]);
            self.shared.engine.want(&union);
            st.union = union;
        }
    }

    /// A cell from the cache, or from the disk (and then into the cache).
    fn find(&self, st: &mut State, tag: u32, dark: bool, key: RasterKey) -> Option<Cell> {
        if let Some(c) = st.lru.get(&key) {
            return Some(Arc::clone(c));
        }
        let disk = self.shared.disk.as_ref()?;
        let name = Self::file(tag, dark, key);
        let rgba = match decode(&disk.get(&name)?) {
            Some(rgba) => rgba,
            None => {
                disk.remove(&name);
                return None;
            }
        };
        let cell = Arc::new(RasterTile { key, rgba });
        st.disk_hits += 1;
        st.lru.insert(key, Arc::clone(&cell), CELL_BYTES);
        Some(cell)
    }

    /// The picture of the cells of `q` (see the module's account of what it answers).
    pub fn pic(&self, q: &PicQuery) -> PicAnswer {
        self.pic_with(q, Mode::Compose)
    }

    /// Whether the cells of `q` are all drawn and on the disk (or outside the world); those that are not are asked of the engine, which
    /// draws them and sets them to be written. Nothing is read back or composed: the background warming's.
    pub fn prime(&self, q: &PicQuery) -> bool {
        matches!(self.pic_with(q, Mode::Prime), PicAnswer::Ready(_)) && self.settled()
    }

    /// Whether the cells that were drawn are all on the disk.
    pub fn settled(&self) -> bool {
        let st = self.shared.lock();
        st.persist.is_empty() && !st.persister
    }

    fn pic_with(&self, q: &PicQuery, mode: Mode) -> PicAnswer {
        let shared = &self.shared;
        let mut st = shared.lock();
        if st.closed {
            return PicAnswer::Unknown;
        }
        if st.dark != Some(q.dark) {
            // another look: the cells are drawn again (those on the disk are of the other palette's name)
            shared.engine.set_palette(palette(q.dark));
            st.lru.clear();
            st.wants.clear();
            st.union.clear();
            st.dark = Some(q.dark);
        }
        let tag = self.tag(q.dark);
        self.take_ready(&mut st, tag);
        let id = PicId(q.zoom, q.x0, q.y0, q.x1, q.y1);
        let now = shared.now();
        let at = match st.wants.iter().position(|w| w.id == id) {
            Some(i) => {
                // asked again: what it lacks may be in the cache by now
                for j in 0..st.wants[i].slots.len() {
                    if let Slot::Lacking(k) = st.wants[i].slots[j]
                        && let Some(c) = st.lru.get(&k).map(Arc::clone)
                    {
                        st.wants[i].slots[j] = Slot::Have(c);
                    }
                }
                st.wants[i].asked = now;
                st.wants[i].epoch = q.epoch;
                i
            }
            None => {
                let grid = GridZoom(q.zoom);
                let mut slots = Vec::with_capacity(q.cells());
                for y in q.y0..=q.y1 {
                    for x in q.x0..=q.x1 {
                        let key = RasterKey { grid, x, y };
                        slots.push(if !key.in_world() {
                            Slot::Ground
                        } else if mode == Mode::Prime {
                            let on_disk = shared.disk.as_ref().is_some_and(|d| d.file_len(&Self::file(tag, q.dark, key)).is_some());
                            if on_disk || st.lru.contains(&key) { Slot::Ground } else { Slot::Lacking(key) }
                        } else {
                            match self.find(&mut st, tag, q.dark, key) {
                                Some(c) => Slot::Have(c),
                                None => Slot::Lacking(key),
                            }
                        });
                    }
                }
                let centre = (f64::from(q.x0 + q.x1) / 2.0, f64::from(q.y0 + q.y1) / 2.0);
                st.wants.push(Want { id, slots, centre, epoch: q.epoch, asked: now, since: now });
                st.wants.len() - 1
            }
        };
        let (have, total) = {
            let w = &st.wants[at];
            (w.slots.iter().filter(|s| !matches!(s, Slot::Lacking(_))).count(), w.slots.len())
        };
        if have == total {
            let w = st.wants.remove(at);
            self.rewant(&mut st);
            drop(st);
            if mode == Mode::Prime {
                return PicAnswer::Ready(Vec::new());
            }
            return PicAnswer::Ready(compose(q, &w.slots, q.dark, true));
        }
        self.rewant(&mut st);
        let Some(w) = st.wants.iter().find(|w| w.id == id) else {
            // asked for an older view than the newest: its want lapsed at once
            return PicAnswer::Coming { have, want: total, failing: false };
        };
        if mode == Mode::Compose && w.slots.iter().any(|s| matches!(s, Slot::Have(_))) && now.duration_since(w.since) >= PARTIAL_AFTER {
            // what there is, the ground where there is not
            let slots: Vec<Slot> = w.slots.iter().map(|s| if let Slot::Have(c) = s { Slot::Have(Arc::clone(c)) } else { Slot::Ground }).collect();
            drop(st);
            return PicAnswer::Ready(compose(q, &slots, q.dark, false));
        }
        let failing = matches!(shared.engine.status(), MapStatus::Offline { .. } | MapStatus::Failed(_));
        PicAnswer::Coming { have, want: total, failing }
    }

    /// What is held, for the tests and the log.
    pub fn stats(&self) -> BaseStats {
        let st = self.shared.lock();
        BaseStats { cells: st.lru.len(), bytes: st.lru.bytes(), wants: st.wants.len(), queued_for_disk: st.persist.len(), disk_hits: st.disk_hits, engine: self.shared.engine.stats(), vector: self.shared.prefetch.stats() }
    }

    /// Lets everything go: the cells, the wants (the engine drops what it had not begun), the queue for the disk.
    pub fn close(&self) {
        let mut st = self.shared.lock();
        st.closed = true;
        st.lru.clear();
        st.wants = Vec::new();
        st.union = Vec::new();
        st.persist = VecDeque::new();
        self.shared.engine.want(&[]);
        self.shared.engine.take_ready();
        self.shared.prefetch.close();
    }

    #[cfg(test)]
    fn probe(&self) -> &testing::Probe {
        &self.shared.probe
    }

    /// Returns when the drawn cells are all on the disk.
    #[cfg(test)]
    fn wait_settled(&self) {
        let mut st = self.shared.lock();
        while !st.persist.is_empty() || st.persister {
            st = self.shared.probe.settled.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl Drop for BaseMap {
    fn drop(&mut self) {
        self.close();
    }
}

/// Writes the drawn cells to the disk, one at a time at low priority, until there are none.
fn persist(shared: &Shared) {
    lower_priority();
    loop {
        let next = {
            let mut st = shared.lock();
            match st.persist.pop_front() {
                Some(n) if !st.closed => n,
                _ => {
                    st.persister = false;
                    #[cfg(test)]
                    shared.probe.settled.notify_all();
                    return;
                }
            }
        };
        let (key, dark, tag, cell) = next;
        let Some(disk) = &shared.disk else { continue };
        let name = BaseMap::file(tag, dark, key);
        if disk.file_len(&name).is_some() {
            continue;
        }
        if let Some(png) = encode(&cell.rgba) {
            let _ = disk.put(&name, &png);
        }
    }
}

/// The cells laid side by side into one picture on the land's colour (the ground where a cell is outside the world or not yet there).
fn compose(q: &PicQuery, slots: &[Slot], dark: bool, complete: bool) -> Vec<u8> {
    let (w, h) = (q.width(), q.height());
    let across = (q.x1 - q.x0 + 1) as usize;
    let ground = palette(dark).land;
    let mut out = Vec::with_capacity(w * h * 4 + TRAILER);
    out.resize(w * h * 4, 0);
    for px in out.chunks_exact_mut(4) {
        px.copy_from_slice(&ground);
    }
    let mut have = 0;
    for (i, slot) in slots.iter().enumerate() {
        let Slot::Have(c) = slot else { continue };
        have += 1;
        let (cx, cy) = ((i % across) * CELL_PX, (i / across) * CELL_PX);
        for row in 0..CELL_PX {
            let at = ((cy + row) * w + cx) * 4;
            out[at..at + CELL_PX * 4].copy_from_slice(&c.rgba[row * CELL_PX * 4..(row + 1) * CELL_PX * 4]);
        }
    }
    q.seal(out, complete, have, slots.len())
}

/// A map source for the tests: every vector tile of zoom 8 of the area the tests ask for (columns 70 to 90, rows 90 to 110), all water.
#[cfg(test)]
pub(crate) mod testing {
    use map::mvt::{GeomType, Layer, Point, Value, VectorTile};
    use map::source::{FakeSource, SourceError, TileSource};
    use map::webmerc::TileKey;
    use parking_lot::{Condvar, Mutex};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// How long a test waits for a thread before it says which one never came.
    const PATIENCE: Duration = Duration::from_secs(20);

    /// The base map's test clock (it moves only when a test moves it), and what its threads did.
    pub struct Probe {
        start: Instant,
        ahead: Mutex<Duration>,
        /// How often the engine's waker was called.
        woken: Mutex<u64>,
        wake: Condvar,
        /// Told when the cell writer ends (under the base map's own lock).
        pub(super) settled: std::sync::Condvar,
    }

    impl Probe {
        pub(super) fn new() -> Self {
            Self { start: Instant::now(), ahead: Mutex::new(Duration::ZERO), woken: Mutex::new(0), wake: Condvar::new(), settled: std::sync::Condvar::new() }
        }

        pub(super) fn now(&self) -> Instant {
            self.start + *self.ahead.lock()
        }

        pub fn advance(&self, by: Duration) {
            *self.ahead.lock() += by;
        }

        pub(super) fn woke(&self) {
            *self.woken.lock() += 1;
            self.wake.notify_all();
        }

        pub fn woken(&self) -> u64 {
            *self.woken.lock()
        }

        /// Returns once the engine has had news for the page since it had been woken `seen` times.
        pub fn wait_woken(&self, seen: u64) {
            let mut woken = self.woken.lock();
            let waited = self.wake.wait_while_for(&mut woken, |w| *w <= seen, PATIENCE);
            assert!(!waited.timed_out(), "the engine had nothing more for the page");
        }
    }

    /// A vector tile that is all water.
    pub fn water() -> Vec<u8> {
        let mut layer = Layer::new("water", 4096);
        let ring: &[Point] = &[[0, 0], [4096, 0], [4096, 4096], [0, 4096]];
        layer.push(GeomType::Polygon, &[ring], &[("class", Value::from("lake"))]).expect("a polygon");
        VectorTile { layers: vec![layer] }.encode()
    }

    /// A source whose every tile, at any zoom, is water: for the tests that go about the map.
    pub struct Water {
        pub calls: AtomicUsize,
        /// The most requests that were on the wire at once.
        pub peak: AtomicUsize,
        active: AtomicUsize,
        pub slow: Duration,
    }

    impl Water {
        pub fn new(slow_ms: u64) -> Arc<Self> {
            Arc::new(Self { calls: AtomicUsize::new(0), peak: AtomicUsize::new(0), active: AtomicUsize::new(0), slow: Duration::from_millis(slow_ms) })
        }
    }

    impl TileSource for Water {
        fn fetch(&self, _: TileKey) -> Result<Vec<u8>, SourceError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(self.slow);
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(water())
        }

        fn id(&self) -> String {
            "water".to_owned()
        }

        fn attribution(&self) -> &str {
            "test"
        }
    }

    /// A source whose tiles are held back until [`Gated::release`]: a fetch that is on the wire stays there, whatever the time.
    pub struct Gated {
        inner: Arc<FakeSource>,
        /// Open, and how many fetches began.
        state: Mutex<(bool, usize)>,
        changed: Condvar,
    }

    impl Gated {
        pub fn new(inner: Arc<FakeSource>) -> Arc<Self> {
            Arc::new(Self { inner, state: Mutex::new((false, 0)), changed: Condvar::new() })
        }

        pub fn release(&self) {
            self.state.lock().0 = true;
            self.changed.notify_all();
        }

        /// Returns once a fetch is on the wire.
        pub fn wait_began(&self) {
            let mut state = self.state.lock();
            let waited = self.changed.wait_while_for(&mut state, |s| s.1 == 0, PATIENCE);
            assert!(!waited.timed_out(), "no fetch began");
        }
    }

    impl TileSource for Gated {
        fn fetch(&self, key: TileKey) -> Result<Vec<u8>, SourceError> {
            let mut state = self.state.lock();
            state.1 += 1;
            self.changed.notify_all();
            self.changed.wait_while(&mut state, |s| !s.0);
            drop(state);
            self.inner.fetch(key)
        }

        fn id(&self) -> String {
            self.inner.id()
        }

        fn attribution(&self) -> &str {
            self.inner.attribution()
        }
    }

    pub fn source() -> Arc<FakeSource> {
        let s = Arc::new(FakeSource::with_id("ofm-test"));
        for x in 70..=90 {
            for y in 90..=110 {
                s.put(TileKey::new(8, x, y).unwrap(), water());
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{Gated, source};
    use super::*;
    use crate::wxpic::Kind;
    use map::source::FakeSource;
    use map::webmerc::TileKey;

    fn query(cells: (i32, i32, i32, i32), epoch: u64, dark: bool) -> PicQuery {
        PicQuery { kind: Kind::Map, zoom: 16, x0: cells.0, y0: cells.1, x1: cells.2, y1: cells.3, t: 0, dark, epoch }
    }

    /// Asks until the picture is there, waiting between asks for the engine's news.
    fn until_ready(m: &BaseMap, q: &PicQuery) -> PicAnswer {
        loop {
            let seen = m.probe().woken();
            match m.pic(q) {
                PicAnswer::Coming { .. } => m.probe().wait_woken(seen),
                other => return other,
            }
        }
    }

    fn map_in(dir: &Path, source: &Arc<FakeSource>) -> (BaseMap, Arc<AtomicU64>) {
        let epoch = Arc::new(AtomicU64::new(1));
        (BaseMap::new(source.clone(), dir, Arc::clone(&epoch)), epoch)
    }

    #[test]
    fn a_picture_is_the_cells_asked_for_drawn_and_side_by_side() {
        let dir = tempfile::tempdir().unwrap();
        let (m, _) = map_in(dir.path(), &source());
        let q = query((75, 100, 77, 101), 1, true);
        let PicAnswer::Ready(p) = until_ready(&m, &q) else { panic!("a picture") };
        assert_eq!(p.len(), 3 * 256 * 2 * 256 * 4 + TRAILER);
        let water = MapPalette::calm_dark().water;
        let mid = (300 * 3 * 256 + 400) * 4;
        assert_eq!(p[mid..mid + 3], water[..3], "a water tile is the water colour");
        assert_eq!(p[p.len() - TRAILER + 12], 1, "complete");
        let s = m.stats();
        assert_eq!((s.cells, s.bytes), (6, 6 * CELL_BYTES));
        assert_eq!(s.engine.rasters, 6, "six cells drawn and no other");
    }

    #[test]
    fn a_cell_outside_the_world_is_the_ground_and_costs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (m, _) = map_in(dir.path(), &source());
        // grid 16 is zoom 8: 256 columns; column -1 is outside
        let q = query((-1, 100, -1, 100), 1, true);
        let PicAnswer::Ready(p) = until_ready(&m, &q) else { panic!("a picture") };
        let land = MapPalette::calm_dark().land;
        assert_eq!(p[..4], land);
        assert_eq!(m.stats().bytes, 0);
    }

    #[test]
    fn the_cells_are_held_to_their_budget_and_a_cell_drawn_once_is_read_back_from_the_disk() {
        let dir = tempfile::tempdir().unwrap();
        let src = source();
        let (m, _) = map_in(dir.path(), &src);
        // 8 by 9 = 72 cells: more than the 64 that fit
        assert!(matches!(until_ready(&m, &query((72, 98, 79, 106), 1, true)), PicAnswer::Ready(_)));
        let s = m.stats();
        assert!(s.bytes <= CELL_BUDGET && s.cells <= 64, "{s:?}");
        // the drawing is written to the disk at low priority (a queue of 64, so the 72 above may lose some): four cells, written, are
        // read back by a map that has nothing in memory
        let q = query((85, 105, 86, 106), 1, true);
        assert!(matches!(until_ready(&m, &q), PicAnswer::Ready(_)));
        m.wait_settled();
        m.close();
        drop(m);
        let (again, _) = map_in(dir.path(), &src);
        let PicAnswer::Ready(_) = until_ready(&again, &q) else { panic!("a picture") };
        let s = again.stats();
        assert_eq!((s.engine.rasters, s.disk_hits), (0, 4), "{s:?}");
    }

    #[test]
    fn the_other_palette_is_another_set_of_cells() {
        let dir = tempfile::tempdir().unwrap();
        let (m, _) = map_in(dir.path(), &source());
        let q = query((75, 100, 75, 100), 1, true);
        let PicAnswer::Ready(dark) = until_ready(&m, &q) else { panic!("a picture") };
        let PicAnswer::Ready(light) = until_ready(&m, &PicQuery { dark: false, ..q }) else { panic!("a picture") };
        assert_ne!(dark[..4], light[..4]);
        assert_eq!(m.stats().cells, 1, "the dark cell was let go for the light one");
    }

    #[test]
    fn a_view_that_moves_stops_the_cells_it_no_longer_wants_and_closing_ends_the_work() {
        let dir = tempfile::tempdir().unwrap();
        let src = source();
        let gated = Gated::new(src.clone());
        let epoch = Arc::new(AtomicU64::new(1));
        let m = BaseMap::new(gated.clone(), dir.path(), Arc::clone(&epoch));
        // 12 cells: the first tile is on the wire (the others wait for it, as a source that has to learn where its tiles are would have them) when the
        // view moves on to a cell of its own
        assert!(matches!(m.pic(&query((72, 98, 75, 100), 1, true)), PicAnswer::Coming { .. }));
        gated.wait_began();
        epoch.fetch_max(2, Ordering::SeqCst);
        let q2 = query((85, 105, 85, 105), 2, true);
        assert!(matches!(m.pic(&q2), PicAnswer::Coming { .. }));
        gated.release();
        assert!(matches!(until_ready(&m, &q2), PicAnswer::Ready(_)));
        // the tile that was on the wire and the second view's: none of the other eleven of the first
        assert_eq!(src.call_count(), 2, "{:?}", src.calls());
        m.close();
        let s = m.stats();
        assert_eq!((s.cells, s.bytes, s.wants), (0, 0, 0));
        assert_eq!((s.vector.queued, s.vector.ready), (0, 0));
        assert_eq!(m.pic(&q2), PicAnswer::Unknown);
    }

    #[test]
    fn a_map_that_is_partly_there_is_given_as_it_is_after_a_while() {
        let dir = tempfile::tempdir().unwrap();
        let src = source();
        // one tile of the three never comes
        src.fail_tile(TileKey::new(8, 76, 100).unwrap(), Some(map::source::SourceError::Offline));
        let (m, _) = map_in(dir.path(), &src);
        let q = query((75, 100, 77, 100), 1, true);
        // the two cells there are drawn; the picture waits for the third
        loop {
            let seen = m.probe().woken();
            match m.pic(&q) {
                PicAnswer::Coming { have: 2, .. } => break,
                PicAnswer::Coming { .. } => m.probe().wait_woken(seen),
                other => panic!("{other:?} before the time"),
            }
        }
        m.probe().advance(PARTIAL_AFTER - Duration::from_millis(1));
        assert!(matches!(m.pic(&q), PicAnswer::Coming { have: 2, .. }), "not before the time");
        m.probe().advance(Duration::from_millis(1));
        let PicAnswer::Ready(p) = m.pic(&q) else { panic!("no picture after the time") };
        assert_eq!(p[p.len() - TRAILER + 12], 0, "marked as not complete");
        assert_eq!(u16::from_le_bytes(p[p.len() - TRAILER + 16..p.len() - TRAILER + 18].try_into().unwrap()), 2, "two of three");
    }
}
