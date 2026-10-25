//! The frame path must not touch the heap once it is warm (LumaShow lesson 20): the allocator below counts the
//! allocations of *this* thread while a test arms it. The pattern is LumaShow's `alloc_tests.rs`; the tests here are
//! this compositor's (`pacer_tests.rs` uses `counted` too).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

struct Counting;

fn count() {
    if ARMED.with(Cell::get) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: every call is forwarded to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `f` with this thread's allocations counted.
pub(crate) fn counted(f: impl FnOnce()) -> usize {
    ALLOCATIONS.store(0, Ordering::Relaxed);
    ARMED.with(|a| a.set(true));
    f();
    ARMED.with(|a| a.set(false));
    ALLOCATIONS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::counted;
    use crate::Rect;
    use crate::damage::{Ring, TILE, Tiles};
    use crate::scene::Scene;
    use crate::surface::Surfaces;

    #[test]
    fn planning_a_still_scene_allocates_nothing_once_warm() {
        let mut scene = Scene::default();
        let surfaces = Surfaces::default();
        let _ = scene.plan(&surfaces);
        let n = counted(|| {
            for _ in 0..100 {
                let plan = scene.plan(&surfaces);
                assert!(plan.items.is_empty());
            }
        });
        assert_eq!(n, 0);
    }

    #[test]
    fn the_damage_ring_allocates_nothing_once_warm() {
        let mut ring = Ring::new(3, 1920, 1080);
        let mut tiles = Tiles::new(1920, 1080);
        tiles.add(&Rect::new(10, 10, 50, 50));
        let _ = ring.frame(0, &tiles);
        let _ = ring.frame(1, &tiles);
        let n = counted(|| {
            for k in 0..300usize {
                tiles.clear();
                tiles.add(&Rect::new((k as i32 * 7) % 1800, 100, 64, 64));
                let owed = ring.frame(k % 3, &tiles);
                assert!(owed.bounds().map(|r| r.w >= TILE).unwrap_or(false));
            }
        });
        assert_eq!(n, 0);
    }
}
