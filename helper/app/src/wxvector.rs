//! The base map's vector tiles, fetched several at a time ahead of the engine that draws them.
//!
//! LumaShow's map engine has one worker, and the worker gets each vector tile it lacks itself, one request at a time: on the set every
//! request is a child process and a TLS handshake (`fetcher.rs`), so a screen of 40 to 54 tiles that are not on the disk is that many round
//! trips in a row (7 to 8 s on the host, with a fast network; a view that has been zoomed to or panned to for the first time is a wait of that
//! length, and the picture under it is the coarse one). This is a [`TileSource`] over the real one that the map engine is given instead: the
//! page's wanted tiles are told to it ([`Prefetch::want`]), up to [`PARALLEL`] of them are fetched at once by threads of its own, and when
//! the engine asks for a tile ([`TileSource::fetch`]) it is handed the one that is already there, or waits for the one on the wire, or, if
//! the tile has not begun, gets it itself at once.
//!
//! **Bounded.** At most [`BUFFERED`] tiles are fetched and not yet taken (a tile of the zooms the page uses is 40 to 150 KB, so a megabyte or
//! two), and none that the engine's own disk already has: the engine looks there first and would never ask for it. **Cancelling:** a new
//! wanted set replaces the old; the tiles not begun are dropped from the queue, those fetched and no longer wanted are freed, and one on the
//! wire finishes and is kept only if it is still wanted. [`Prefetch::close`] ends the threads and frees everything.

use crate::wxcache::fnv32;
use map::source::{SourceError, TileSource};
use map::store::DiskCache;
use map::webmerc::TileKey;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// Tiles fetched at once.
pub const PARALLEL: usize = 4;
/// Tiles fetched and waiting for the engine, with those on the wire, at most.
pub const BUFFERED: usize = 12;
/// Tiles remembered as handed to the engine (it has them in memory or has just written them to its disk), so that a tile it took a
/// moment ago, whose file is not yet there, is not fetched again.
const TAKEN: usize = 256;
/// How long a thread waits for the next tile before it goes.
const IDLE: Duration = Duration::from_millis(1_000);

struct State {
    /// What the page wants now.
    wanted: HashSet<TileKey>,
    /// Not begun, in the order they are wanted.
    queue: VecDeque<TileKey>,
    /// On the wire.
    active: HashSet<TileKey>,
    /// Fetched and not yet taken.
    ready: HashMap<TileKey, Result<Vec<u8>, SourceError>>,
    /// Handed to the engine lately, oldest first.
    taken: VecDeque<TileKey>,
    /// The tile the engine is waiting for (it asked for one that is on the wire).
    waiting: Option<TileKey>,
    threads: usize,
    closed: bool,
    fetched: u64,
    /// How often the engine asked for a tile.
    asked: u64,
}

pub struct Prefetch {
    inner: Arc<dyn TileSource>,
    /// The engine's own cache of vector tiles, to leave alone the tiles it has.
    disk: Option<DiskCache>,
    st: Mutex<State>,
    /// A tile came, or a place in the buffer was freed, or the wanted set changed.
    wake: Condvar,
}

/// What the prefetch holds, for the tests and the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrefetchStats {
    pub queued: usize,
    pub active: usize,
    pub ready: usize,
    pub threads: usize,
    pub fetched: u64,
    pub asked: u64,
}

impl Prefetch {
    /// A prefetch over `inner`; `disk` is the engine's cache directory (`<root>/map`), whose files the engine reads before it asks for a tile.
    pub fn new(inner: Arc<dyn TileSource>, disk: Option<DiskCache>) -> Arc<Self> {
        Arc::new(Self { inner, disk, st: Mutex::new(State { wanted: HashSet::new(), queue: VecDeque::new(), active: HashSet::new(), ready: HashMap::new(), taken: VecDeque::new(), waiting: None, threads: 0, closed: false, fetched: 0, asked: 0 }), wake: Condvar::new() })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The name the engine keeps a tile under (`map::engine`): under the source's current id, so another planet is another set of files.
    fn on_disk(&self, key: TileKey) -> bool {
        self.disk.as_ref().is_some_and(|d| d.file_len(&format!("v-{}-{}-{}-{:08x}.pbf", key.z, key.x, key.y, fnv32(&self.inner.id()))).is_some())
    }

    /// The tiles the page wants now, the most wanted first: replaces the wanted set.
    pub fn want(self: &Arc<Self>, keys: &[TileKey]) {
        let missing: Vec<TileKey> = {
            let st = self.lock();
            if st.closed {
                return;
            }
            keys.iter().copied().filter(|k| !st.ready.contains_key(k) && !st.active.contains(k) && !st.taken.contains(k)).collect()
        };
        // the disk is asked outside the lock
        let missing: Vec<TileKey> = missing.into_iter().filter(|k| !self.on_disk(*k)).collect();
        let mut st = self.lock();
        if st.closed {
            return;
        }
        st.wanted = keys.iter().copied().collect();
        let kept: HashMap<TileKey, Result<Vec<u8>, SourceError>> = std::mem::take(&mut st.ready).into_iter().filter(|(k, _)| st.wanted.contains(k)).collect();
        st.ready = kept;
        st.queue = missing.into_iter().collect();
        let threads = PARALLEL.min(st.queue.len());
        while st.threads < threads {
            st.threads += 1;
            let me = Arc::clone(self);
            if std::thread::Builder::new().name("wx-vector".into()).stack_size(512 * 1024).spawn(move || me.work()).is_err() {
                st.threads -= 1;
                break;
            }
        }
        drop(st);
        self.wake.notify_all();
    }

    fn work(self: Arc<Self>) {
        lower_priority();
        loop {
            let key = {
                let mut st = self.lock();
                loop {
                    if st.closed {
                        st.threads -= 1;
                        return;
                    }
                    // the first tile goes alone: a source that has to learn where its tiles are (OpenFreeMap's TileJSON) learns it once
                    let room = if st.fetched == 0 { 1 } else { PARALLEL };
                    if st.ready.len() + st.active.len() < BUFFERED
                        && st.active.len() < room
                        && let Some(k) = st.queue.pop_front()
                    {
                        st.active.insert(k);
                        break k;
                    }
                    let (guard, waited) = self.wake.wait_timeout(st, IDLE).unwrap_or_else(|e| e.into_inner());
                    st = guard;
                    if waited.timed_out() && st.queue.is_empty() {
                        st.threads -= 1;
                        return;
                    }
                }
            };
            let got = self.inner.fetch(key);
            let mut st = self.lock();
            st.active.remove(&key);
            st.fetched += 1;
            if !st.closed && (st.wanted.contains(&key) || st.waiting == Some(key)) {
                st.ready.insert(key, got);
            }
            drop(st);
            // the engine may be waiting for this one; a place in the buffer is free
            self.wake.notify_all();
        }
    }

    /// What is held, for the tests and the log.
    pub fn stats(&self) -> PrefetchStats {
        let st = self.lock();
        PrefetchStats { queued: st.queue.len(), active: st.active.len(), ready: st.ready.len(), threads: st.threads, fetched: st.fetched, asked: st.asked }
    }

    /// Returns once the engine has asked for a tile: whether it waits for the one on the wire or gets it itself, it holds that tile from then on.
    #[cfg(test)]
    pub fn wait_asked(&self) {
        let st = self.lock();
        let (_held, waited) = self.wake.wait_timeout_while(st, crate::wxmap::testing::PATIENCE, |s| s.asked == 0).unwrap_or_else(|e| e.into_inner());
        assert!(!waited.timed_out(), "the engine never asked for a tile");
    }

    /// Ends the threads and frees what was fetched.
    pub fn close(&self) {
        {
            let mut st = self.lock();
            st.closed = true;
            st.wanted = HashSet::new();
            st.queue = VecDeque::new();
            st.ready = HashMap::new();
            st.taken = VecDeque::new();
        }
        self.wake.notify_all();
    }
}

impl TileSource for Prefetch {
    fn fetch(&self, key: TileKey) -> Result<Vec<u8>, SourceError> {
        let mut st = self.lock();
        st.asked += 1;
        #[cfg(test)]
        self.wake.notify_all();
        loop {
            if st.closed {
                st.waiting = None;
                return Err(SourceError::Other("the view was closed".to_owned()));
            }
            if let Some(got) = st.ready.remove(&key) {
                st.waiting = None;
                if st.taken.len() >= TAKEN {
                    st.taken.pop_front();
                }
                st.taken.push_back(key);
                drop(st);
                self.wake.notify_all();
                return got;
            }
            if st.active.contains(&key) {
                // on the wire already: the engine waits for it, and it is kept for the engine even if the page has moved on
                st.waiting = Some(key);
                st = self.wake.wait(st).unwrap_or_else(|e| e.into_inner());
                continue;
            }
            // not begun (or not wanted at all): the engine gets it itself, now, and it is on the wire for the threads and for the next wanted set
            st.queue.retain(|k| *k != key);
            st.active.insert(key);
            drop(st);
            let got = self.inner.fetch(key);
            let mut st = self.lock();
            st.active.remove(&key);
            st.fetched += 1;
            if st.taken.len() >= TAKEN {
                st.taken.pop_front();
            }
            st.taken.push_back(key);
            drop(st);
            self.wake.notify_all();
            return got;
        }
    }

    fn id(&self) -> String {
        self.inner.id()
    }

    fn attribution(&self) -> &str {
        self.inner.attribution()
    }
}

impl Drop for Prefetch {
    fn drop(&mut self) {
        self.close();
    }
}

fn lower_priority() {
    // SAFETY: `gettid` and `setpriority` take and return plain integers and touch no memory of ours; a refusal is ignored.
    unsafe {
        let tid = libc::gettid() as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wxmap::testing::Water;
    use std::time::Instant;

    fn keys(n: u32) -> Vec<TileKey> {
        (0..n).map(|i| TileKey::new(9, 100 + i, 200).unwrap()).collect()
    }

    fn settle(p: &Prefetch, until: impl Fn(&PrefetchStats) -> bool) -> PrefetchStats {
        let began = Instant::now();
        loop {
            let s = p.stats();
            if until(&s) || began.elapsed() > Duration::from_secs(20) {
                return s;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn tiles_come_four_at_a_time_and_the_engine_is_handed_each_as_it_asks() {
        let water = Water::held();
        let p = Prefetch::new(water.clone(), None);
        p.want(&keys(8));
        // the first tile goes alone; with it fetched, every thread is on the wire and the rest of the tiles wait for one to come back
        water.wait_active(1);
        water.release(1);
        water.wait_active(PARALLEL);
        let s = p.stats();
        assert_eq!((s.active, s.queued), (PARALLEL, 3), "four on the wire at once, no more: {s:?}");
        water.release_all();
        // the engine takes them in its order, as its worker does
        for k in keys(8) {
            assert!(p.fetch(k).is_ok());
        }
        assert_eq!(water.calls.load(std::sync::atomic::Ordering::SeqCst), 8, "each tile once");
        assert_eq!(water.peak.load(std::sync::atomic::Ordering::SeqCst), PARALLEL, "never more than four on the wire");
    }

    #[test]
    fn a_tile_that_is_not_wanted_is_fetched_by_the_engine_itself() {
        let water = Water::new();
        let p = Prefetch::new(water.clone(), None);
        assert!(p.fetch(keys(1)[0]).is_ok());
        assert_eq!(water.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(p.stats(), PrefetchStats { fetched: 1, asked: 1, ..PrefetchStats::default() }, "nothing is held for it");
    }

    #[test]
    fn a_tile_the_engine_asked_for_before_it_was_wanted_is_one_request_and_not_two() {
        let water = Water::held();
        let p = Prefetch::new(water.clone(), None);
        let k = keys(1)[0];
        let engine = {
            let p = Arc::clone(&p);
            std::thread::spawn(move || p.fetch(k))
        };
        // the tile is on the wire (the engine's own request) when the page says it wants it
        water.wait_active(1);
        p.want(&[k]);
        water.release_all();
        assert!(engine.join().unwrap().is_ok());
        assert_eq!(water.calls.load(std::sync::atomic::Ordering::SeqCst), 1, "one request");
        assert_eq!((p.stats().queued, p.stats().ready), (0, 0));
    }

    #[test]
    fn a_new_wanted_set_drops_what_has_not_begun_and_frees_what_is_not_wanted() {
        let water = Water::held();
        let p = Prefetch::new(water.clone(), None);
        p.want(&keys(12));
        // the first tile goes alone; with it fetched, four are on the wire
        water.wait_active(1);
        water.release(1);
        water.wait_active(PARALLEL);
        // the view moves to two tiles of its own
        let others: Vec<TileKey> = (0..2).map(|i| TileKey::new(9, 300 + i, 300).unwrap()).collect();
        p.want(&others);
        assert_eq!(p.stats().ready, 0, "the first tile is no longer wanted and is freed");
        water.release_all();
        let s = settle(&p, |s| s.ready == 2 && s.active == 0);
        assert_eq!((s.queued, s.ready), (0, 2), "{s:?}");
        let calls = water.calls.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(calls, 1 + PARALLEL + 2, "the first, the four on the wire and the two now wanted, none of the seven queued");
        // a place the page has left is freed
        p.want(&[]);
        assert_eq!(p.stats().ready, 0);
    }

    #[test]
    fn what_waits_for_the_engine_is_bounded() {
        let water = Water::new();
        let p = Prefetch::new(water.clone(), None);
        p.want(&keys(40));
        // a tile is begun only while the fetched and the on the wire are fewer than the bound, so the twelfth to arrive is the last
        let s = settle(&p, |s| s.ready >= BUFFERED);
        assert_eq!((s.ready, s.active, s.queued), (BUFFERED, 0, 40 - BUFFERED), "the rest waits in the queue until the engine takes some: {s:?}");
        // the engine takes one: the next is fetched
        assert!(p.fetch(keys(40)[0]).is_ok());
        let s3 = settle(&p, |s| s.fetched as usize > BUFFERED);
        assert!(s3.fetched as usize > BUFFERED);
    }

    #[test]
    fn a_tile_the_engines_own_disk_has_is_not_fetched() {
        let dir = tempfile::tempdir().unwrap();
        let disk = DiskCache::open(dir.path(), 1 << 20).unwrap();
        let water = Water::new();
        let p = Prefetch::new(water.clone(), Some(disk.clone()));
        let k = keys(3);
        // the name the engine gives a tile under the source's id
        disk.put(&format!("v-{}-{}-{}-{:08x}.pbf", k[1].z, k[1].x, k[1].y, fnv32(&water.id())), b"x").unwrap();
        p.want(&k);
        let s = settle(&p, |s| s.ready == 2);
        assert_eq!(s.ready, 2, "{s:?}");
        assert!(p.fetch(k[1]).is_ok(), "the engine, which reads its disk first, would not ask: if it does it is served");
        assert_eq!(water.calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn closing_frees_the_tiles_and_ends_the_threads() {
        let water = Water::new();
        let p = Prefetch::new(water, None);
        p.want(&keys(6));
        settle(&p, |s| s.ready == 6);
        p.close();
        let s = settle(&p, |s| s.threads == 0);
        assert_eq!((s.ready, s.queued, s.threads), (0, 0, 0), "{s:?}");
        p.want(&keys(3));
        assert_eq!(p.stats().queued, 0, "a closed prefetch wants nothing");
    }
}
