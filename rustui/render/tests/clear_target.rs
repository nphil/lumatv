//! A render target is made clear without holding a texture's worth of zeros: the compositor's allocator keeps every block it
//! takes, so an 8 MB buffer for a panel-sized target stays as 8 MB of resident heap.

use lumatv_render::gl::Gles;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

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

#[test]
fn a_target_is_clear_in_every_row_and_made_from_a_small_buffer() {
    let gl = Gles::software(c"libEGL.so.1").expect("Mesa software GL is required");
    // sizes that end a band part-way, one that is a band's rows, and a single row
    for (w, h) in [(1920u32, 1080u32), (1001, 777), (64, 4096), (3, 1)] {
        // a texture that held pixels, deleted: what the next one may be given back
        let dirty = gl.new_texture(w, h);
        let ones = vec![0xFFu8; w as usize * h as usize * 4];
        // SAFETY: `ones` holds the `w * h` RGBA8 pixels of the bound texture.
        unsafe { gl.upload(0, 0, w, h, w, ones.as_ptr()) };
        gl.delete_texture(dirty);
        drop(ones);

        BIGGEST.store(0, Ordering::Relaxed);
        let target = gl.new_render_target(w, h).unwrap();
        let biggest = BIGGEST.load(Ordering::Relaxed);
        assert!(biggest <= 512 * 1024, "{w}x{h}: made from a block of {biggest} bytes");

        gl.bind_framebuffer(target.framebuffer);
        let pixels = gl.read_pixels(0, 0, w, h);
        let lit = pixels.iter().filter(|&&b| b != 0).count();
        assert_eq!(lit, 0, "{w}x{h}: {lit} bytes of the target are not clear");
        gl.delete_render_target(target);
    }
}
