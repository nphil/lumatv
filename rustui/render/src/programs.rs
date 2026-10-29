//! The engine's shader programs, linked once per process and handed from one renderer to the next.
//!
//! The engine has 48 programs and the set's Mali takes 1.3 s to compile and link them (round 17: `shader programs compiled and
//! linked in 1293 ms`). Linked by the renderer on the compositor's loop thread, as they were when a client's engine attached while
//! the compositor ran, the loop answers nothing for that long and the supervisor's heartbeat takes the compositor for dead (round
//! 18: two beats missed, LG's compositor back). A program is an object of the context's share group, not of the renderer that
//! asked for it, so a thread with a context that shares the loop's links them ([`crate::host::precompile`]) and the renderer that
//! attaches takes them from here. A renderer that is dropped leaves its programs here as well: a client that restarts attaches a
//! new engine every time, and none of them links anything.

use parking_lot::Mutex;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::CStr;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

/// What a program is made of: its two shaders' sources, hashed in two orders so that a collision needs both to collide.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Key(u64, u64);

impl Key {
    pub(crate) fn of(vertex: &CStr, fragment: &CStr) -> Self {
        let (vertex, fragment) = (vertex.to_bytes(), fragment.to_bytes());
        let mut first = DefaultHasher::new();
        vertex.hash(&mut first);
        fragment.hash(&mut first);
        let mut second = DefaultHasher::new();
        0x6c75_6d61_u32.hash(&mut second);
        fragment.hash(&mut second);
        vertex.hash(&mut second);
        Self(first.finish(), second.finish())
    }
}

/// The programs that are linked and in no renderer, for the renderers of one GL context's share group to take.
#[derive(Default)]
pub struct Cache {
    ready: Mutex<HashMap<Key, Vec<u32>>>,
    complete: AtomicBool,
    cancelled: AtomicBool,
}

impl Cache {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    /// Whether a renderer was built whole through this cache: every program the engine uses is linked, and waits here or is in a
    /// renderer that has the cache. A renderer built from a cache that is not complete links what it does not find.
    pub fn complete(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }

    /// Asks whoever is linking programs through this cache to stop at the next program (the compositor is going away).
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn mark_complete(&self) {
        self.complete.store(true, Ordering::Release);
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn take(&self, key: Key) -> Option<u32> {
        self.ready.lock().get_mut(&key)?.pop()
    }

    fn give(&self, key: Key, program: u32) {
        self.ready.lock().entry(key).or_default().push(program);
    }
}

/// What a renderer's GL wrapper holds of a [`Cache`]: the programs it was given or linked, which go back when it is dropped. A lease
/// made while the cache is complete is strict: whatever it does not find in the cache it does not link either (an engine that attaches
/// while the compositor's loop runs would stop the loop for it), and the renderer it is for is not built.
pub(crate) struct Lease {
    cache: Arc<Cache>,
    held: RefCell<Vec<(Key, u32)>>,
    taken: Cell<u32>,
    strict: bool,
}

impl Lease {
    pub(crate) fn new(cache: Arc<Cache>) -> Self {
        let strict = cache.complete();
        Self { cache, held: RefCell::new(Vec::new()), taken: Cell::new(0), strict }
    }

    pub(crate) fn strict(&self) -> bool {
        self.strict
    }

    /// A program of this make that waits in the cache, now this lease's.
    pub(crate) fn take(&self, key: Key) -> Option<u32> {
        let program = self.cache.take(key)?;
        self.taken.set(self.taken.get() + 1);
        self.held.borrow_mut().push((key, program));
        Some(program)
    }

    /// A program this lease linked itself, to go to the cache with the others.
    pub(crate) fn hold(&self, key: Key, program: u32) {
        self.held.borrow_mut().push((key, program));
    }

    /// A program that was taken and does not read as linked in this context: not this lease's any more, and not the cache's.
    pub(crate) fn forget(&self, program: u32) {
        self.held.borrow_mut().retain(|(_, held)| *held != program);
        self.taken.set(self.taken.get().saturating_sub(1));
    }

    pub(crate) fn taken(&self) -> u32 {
        self.taken.get()
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.cache.cancelled()
    }

    pub(crate) fn mark_complete(&self) {
        self.cache.mark_complete();
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        for (key, program) in self.held.take() {
            self.cache.give(key, program);
        }
    }
}

/// `LUMATV_COMPILE_DELAY_MS`: the host's stand-in for the set's driver, which takes 30 ms to link a program where Mesa takes one. Each
/// program a [`crate::Gles`] links itself takes this much longer, so that a host test can tell what links on the loop thread from
/// what does not. Unset: no delay.
pub(crate) fn stand_in_delay() {
    static DELAY: LazyLock<Duration> = LazyLock::new(|| {
        std::env::var("LUMATV_COMPILE_DELAY_MS").ok().and_then(|ms| ms.parse().ok()).map(Duration::from_millis).unwrap_or_default()
    });
    if !DELAY.is_zero() {
        std::thread::sleep(*DELAY);
    }
}
