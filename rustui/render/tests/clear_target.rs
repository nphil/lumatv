//! A render target is made clear by the GPU: no pixel goes through the CPU and no block of the heap is held for it. The compositor's
//! allocator keeps every block it takes, so the zeros of a panel-sized target (8 MB, or a band of rows of them) would stay as resident
//! heap for good, and uploading them a band at a time costs the driver a call for each.

use lumatv_render::gl::{Gles, take_target_cost};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

const GL_SCISSOR_TEST: u32 = 0x0C11;

static BIGGEST: AtomicUsize = AtomicUsize::new(0);

struct Biggest;

// SAFETY: every call is the system allocator's; the size is only noted.
unsafe impl GlobalAlloc for Biggest {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        BIGGEST.fetch_max(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        BIGGEST.fetch_max(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        BIGGEST.fetch_max(new_size, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Biggest = Biggest;

/// One test at a time: the allocator's account is the process's, and the software GL is made once at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn soft_gl() -> Gles {
    Gles::software(c"libEGL.so.1").expect("Mesa software GL is required")
}

/// A texture that held pixels, deleted: what the next one may be given back.
fn leave_dirty_memory(gl: &Gles, w: u32, h: u32) {
    let dirty = gl.new_texture(w, h);
    let ones = vec![0xFFu8; w as usize * h as usize * 4];
    // SAFETY: `ones` holds the `w * h` RGBA8 pixels of the bound texture.
    unsafe { gl.upload(0, 0, w, h, w, ones.as_ptr()) };
    gl.delete_texture(dirty);
}

/// The bytes of `pixels` that are not zero.
fn lit(pixels: &[u8]) -> usize {
    pixels.iter().filter(|&&b| b != 0).count()
}

#[test]
fn a_target_is_clear_in_every_row_and_no_pixel_goes_through_the_cpu() {
    let _serial = serial();
    let gl = soft_gl();
    let mut first = None;
    // a panel, sizes that end a band of rows part-way, a tall one, and a single row
    for (w, h) in [(1920u32, 1080u32), (1001, 777), (64, 4096), (3, 1)] {
        leave_dirty_memory(&gl, w, h);

        gl.take_calls();
        BIGGEST.store(0, Ordering::Relaxed);
        let target = gl.new_render_target(w, h).unwrap();
        let biggest = BIGGEST.load(Ordering::Relaxed);
        let calls = gl.take_calls();
        assert!(biggest <= 64 * 1024, "{w}x{h}: made from a block of {biggest} bytes");
        assert_eq!(calls.upload_bytes, 0, "{w}x{h}: pixels were handed to the driver");
        let expected = *first.get_or_insert(calls);
        assert_eq!(calls, expected, "{w}x{h}: what the driver is asked depends on the size");

        gl.bind_framebuffer(target.framebuffer);
        let unclear = lit(&gl.read_pixels(0, 0, w, h));
        assert_eq!(unclear, 0, "{w}x{h}: {unclear} bytes of the target are not clear");
        gl.delete_render_target(target);
    }
}

#[test]
fn making_a_target_clears_all_of_it_and_leaves_the_scissor_test_as_it_found_it() {
    let _serial = serial();
    let gl = soft_gl();
    let (w, h) = (64u32, 48u32);
    // a box of 4 x 3 pixels; with the test on a clear reaches only that, with it off all of the target
    for (on, reached) in [(true, 4 * 3), (false, (w * h) as usize)] {
        if on {
            gl.enable(GL_SCISSOR_TEST);
        } else {
            gl.disable(GL_SCISSOR_TEST);
        }
        gl.scissor(10, 20, 4, 3);
        leave_dirty_memory(&gl, w, h);
        let target = gl.new_render_target(w, h).unwrap();
        gl.bind_framebuffer(target.framebuffer);
        assert_eq!(lit(&gl.read_pixels(0, 0, w, h)), 0, "scissor test {on}: the box limited the clear of the new target");
        gl.clear(1.0, 1.0, 1.0, 1.0);
        assert_eq!(lit(&gl.read_pixels(0, 0, w, h)), reached * 4, "scissor test {on}: not as it was before the target was made");
        gl.delete_render_target(target);
    }
}

#[test]
fn an_offscreen_mesh_target_is_made_clear_the_same_way() {
    let _serial = serial();
    let gl = soft_gl();
    let (w, h) = (64u32, 48u32);
    leave_dirty_memory(&gl, w, h);
    gl.take_calls();
    let target = gl.offscreen(w, h, 0).expect("a colour and depth target");
    assert_eq!(gl.take_calls().upload_bytes, 0, "pixels were handed to the driver");
    assert_eq!(lit(&gl.read_pixels(0, 0, w, h)), 0, "the target is not clear");
    gl.delete_offscreen(target);
}

#[test]
fn what_making_targets_cost_is_counted_for_the_thread_that_made_them() {
    let _serial = serial();
    let gl = soft_gl();
    take_target_cost();
    let a = gl.new_render_target(64, 48).unwrap();
    let b = gl.new_render_target(8, 8).unwrap();
    let (made, pixels, ns) = take_target_cost();
    assert_eq!((made, pixels), (2, 64 * 48 + 8 * 8));
    assert!(ns > 0, "the time they took is counted");
    assert_eq!(take_target_cost(), (0, 0, 0), "what was read is not counted again");

    let c = gl.new_render_target(8, 8).unwrap();
    assert_eq!(std::thread::spawn(take_target_cost).join().unwrap(), (0, 0, 0), "another thread's frames are not charged with these");
    assert_eq!(take_target_cost().0, 1);
    for target in [a, b, c] {
        gl.delete_render_target(target);
    }
}
