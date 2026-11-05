//! A cache held to a number of bytes, least recently used out first: what the weather view keeps of its tiles in memory.
//!
//! The newest entry always stays, however large (the budget is a ceiling for the rest), and a lookup counts as a use. Lookups and
//! inserts are a hash probe; an insert over the budget scans the entries for the oldest, which is a few dozen to a couple of hundred
//! tiles here and runs once per tile that arrives.

use std::collections::HashMap;
use std::hash::Hash;

pub struct ByteLru<K, V> {
    map: HashMap<K, Slot<V>>,
    tick: u64,
    bytes: usize,
    cap: usize,
}

struct Slot<V> {
    value: V,
    bytes: usize,
    used: u64,
}

impl<K: Hash + Eq + Copy, V> ByteLru<K, V> {
    pub fn new(cap: usize) -> Self {
        Self { map: HashMap::new(), tick: 0, bytes: 0, cap }
    }

    /// The value, counted as used.
    pub fn get(&mut self, k: &K) -> Option<&V> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(k).map(|s| {
            s.used = tick;
            &s.value
        })
    }

    /// Whether the key is held, without counting a use.
    pub fn contains(&self, k: &K) -> bool {
        self.map.contains_key(k)
    }

    pub fn insert(&mut self, k: K, value: V, bytes: usize) {
        self.tick += 1;
        if let Some(old) = self.map.insert(k, Slot { value, bytes, used: self.tick }) {
            self.bytes -= old.bytes;
        }
        self.bytes += bytes;
        while self.bytes > self.cap && self.map.len() > 1 {
            let oldest = self.map.iter().filter(|(key, _)| **key != k).min_by_key(|(_, s)| s.used).map(|(key, _)| *key);
            match oldest.and_then(|key| self.map.remove(&key)) {
                Some(gone) => self.bytes -= gone.bytes,
                None => break,
            }
        }
    }

    /// Lets everything go and gives the memory back (a cleared `HashMap` keeps its table).
    pub fn clear(&mut self) {
        self.map = HashMap::new();
        self.bytes = 0;
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn cap(&self) -> usize {
        self.cap
    }
}

/// FNV-1a over `s`: the name part that tells one version of what a cached file was made from from another.
pub fn fnv32(s: &str) -> u32 {
    let mut h = 0x811c_9dc5u32;
    for b in s.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Gives the C library's freed heap pages back to the system, where the library has the call (glibc: the host's and the shell's; musl, the
/// helper's on the set, returns a large block when it is freed and has nothing to trim).
pub fn trim_heap() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: `malloc_trim` takes a pad size and touches only the allocator's own free pages.
    unsafe {
        libc::malloc_trim(0);
    }
}

/// When the heap is trimmed after a view closes, besides at once: its threads end a moment after the close (a worker finishes the fetch or the
/// drawing it was at), and what they held is freed to the C library by then and not before.
pub const TRIM_AFTER_CLOSE: [std::time::Duration; 2] = [std::time::Duration::from_millis(300), std::time::Duration::from_millis(1_200)];

/// [`trim_heap`] now and again at each of [`TRIM_AFTER_CLOSE`], on a thread that ends after the last. Where the library has nothing to trim
/// (musl) this does nothing.
pub fn trim_heap_after_close() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        trim_heap();
        let _ = std::thread::Builder::new().name("wx-trim".into()).stack_size(128 * 1024).spawn(|| {
            let mut waited = std::time::Duration::ZERO;
            for at in TRIM_AFTER_CLOSE {
                std::thread::sleep(at - waited);
                waited = at;
                trim_heap();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_least_recently_used_goes_first_and_the_newest_stays_whatever_its_size() {
        let mut c: ByteLru<u32, &str> = ByteLru::new(10);
        c.insert(1, "a", 4);
        c.insert(2, "b", 4);
        assert!(c.get(&1).is_some(), "1 is used again, so 2 is the oldest");
        c.insert(3, "c", 4);
        assert!(c.contains(&1) && !c.contains(&2) && c.contains(&3));
        assert_eq!(c.bytes(), 8);
        c.insert(4, "big", 50);
        assert_eq!((c.len(), c.bytes()), (1, 50), "everything else went for the one that does not fit");
        c.clear();
        assert_eq!((c.len(), c.bytes()), (0, 0));
    }

    #[test]
    fn an_entry_put_again_counts_its_new_size_only() {
        let mut c: ByteLru<u32, u8> = ByteLru::new(100);
        c.insert(1, 0, 40);
        c.insert(1, 1, 10);
        assert_eq!(c.bytes(), 10);
    }
}
