//! The GPU side of a capture: `Gl::grab_target` reads a scan-out page back through a scratch target of the size asked for. Mesa's software GL
//! over pages in the context's own memory (the stand-in for KMS's dma-buf pages, `standin.rs`), under the real `State::tick`: the picture
//! comes out the right way up, at every size, opaque where the page had a hole, and the scratch target goes back when the capture is over.

mod harness;

use std::io;
use std::path::Path;

use harness::{Client, Server};
use lumatv_compositor::Rect;
use lumatv_compositor::kms::frames::Frames;
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl, TargetId};
use lumatv_compositor::scene::{Hole, Plan};
use lumatv_compositor::surface::Surfaces;
use wayland_server::protocol::wl_buffer::WlBuffer;

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";

/// KMS without the device: the GL renderer, three pages of its own and the ring's frame logic; `grab` is what `Kms::grab` does.
struct ScanOut {
    gl: Gl,
    frames: Frames,
    pages: Vec<TargetId>,
}

impl Output for ScanOut {
    fn name(&self) -> &'static str {
        "screencap-gl-test"
    }

    fn size(&self) -> (u32, u32) {
        self.gl.size()
    }

    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        self.gl.query_buffer(buffer)
    }

    fn forget_buffer(&mut self, buffer: &WlBuffer) {
        self.gl.forget_buffer(buffer)
    }

    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats> {
        let (_, stats) = self.frames.draw(&mut self.gl, plan, surfaces, None)?;
        Ok(stats)
    }

    fn present(&mut self) -> io::Result<Presented> {
        self.gl.present()
    }

    fn capture(&mut self) -> io::Result<Image> {
        self.gl.set_target(self.pages[self.frames.last_page()]);
        self.gl.capture()
    }

    fn grab(&mut self, size: (u32, u32), spare: Vec<u8>) -> io::Result<Image> {
        self.gl.grab_target(self.pages[self.frames.last_page()], size, spare)
    }

    fn grab_deferred(&mut self, size: (u32, u32), spare: Vec<u8>, tag: u64) -> io::Result<Option<(u64, Image)>> {
        self.gl.grab_target_deferred(self.pages[self.frames.last_page()], size, spare, tag)
    }

    fn grab_finish(&mut self, size: (u32, u32), spare: Vec<u8>) -> io::Result<Option<(u64, Image)>> {
        self.gl.grab_take_deferred(size, spare)
    }

    fn grab_end(&mut self) {
        self.gl.release_grab();
    }
}

/// A server whose output is the stand-in, with a card drawn: green, a red block at the top left, a blue block at the bottom right, and a
/// transparent rectangle in the middle that is also a hole for video. `None` when there is no Mesa on this machine.
fn screen() -> Option<(Server, Client)> {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return None;
    }
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let mut server = Server::start_with(Box::new(|| {
        let mut gl = Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap();
        let pages: Vec<TargetId> = (0..3).map(|_| gl.add_target().unwrap()).collect();
        let frames = Frames::new(pages.clone(), 1920, 1080);
        Box::new(ScanOut { gl, frames, pages })
    }));
    server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Grab");
    let buf = client.shm_pattern(1920, 1080, |x, y| match (x, y) {
        (0..200, 0..100) => 0xffff_0000,
        (1620..1920, 930..1080) => 0xff00_00ff,
        (800..1100, 400..600) => 0x0000_0000,
        _ => 0xff00_ff00,
    });
    client.present(&card.surface, &buf);
    client.roundtrip();
    server.eval(|s| {
        let surface = s.scene.windows().find(|w| w.title == "Grab").unwrap().surface;
        s.scene.holes.push(Hole { surface, rect: Rect::new(800, 400, 300, 200) });
    });
    Some((server, client))
}

fn near(a: [u8; 4], b: [u8; 4], tolerance: u8) -> bool {
    a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= tolerance)
}

#[test]
fn a_page_is_read_back_the_right_way_up_at_its_own_size_and_equals_the_cpu_readback() {
    let Some((server, _client)) = screen() else { return };
    let (grab, page) = server.eval(|s| {
        s.tick();
        (s.output.grab((1920, 1080), Vec::new()).unwrap(), s.output.capture().unwrap())
    });
    assert_eq!((grab.width, grab.height, grab.rgba.len()), (1920, 1080, 1920 * 1080 * 4));
    assert_eq!(grab.pixel(5, 5), [255, 0, 0, 255], "the red block is at the top left");
    assert_eq!(grab.pixel(1915, 1075), [0, 0, 255, 255], "the blue block is at the bottom right");
    assert_eq!(grab.pixel(1915, 5), [0, 255, 0, 255]);
    assert_eq!(grab.pixel(5, 1075), [0, 255, 0, 255]);
    // Everywhere the page was opaque the two readbacks are the same bytes; the hole is where they differ, by its alpha alone.
    for y in (0..1080).step_by(7) {
        for x in (0..1920).step_by(5) {
            let hole = (800..1100).contains(&x) && (400..600).contains(&y);
            let (g, p) = (grab.pixel(x, y), page.pixel(x, y));
            if hole {
                assert_eq!(g, [0, 0, 0, 255], "({x},{y}): video's hole is black and opaque");
                assert_eq!(p, [0, 0, 0, 0], "({x},{y}): the page itself holds it transparent");
            } else {
                assert_eq!(g, p, "({x},{y})");
            }
        }
    }
}

#[test]
fn the_gpu_scales_to_any_size_the_recording_asks_for_and_the_picture_keeps_its_shape() {
    let Some((server, _client)) = screen() else { return };
    let (half, hd, cpu) = server.eval(|s| {
        s.tick();
        let half = s.output.grab((960, 540), Vec::new()).unwrap();
        let hd = s.output.grab((1280, 720), half.rgba.clone()).unwrap();
        let cpu = s.output.capture().unwrap().scaled(960, 540, Vec::new());
        (half, hd, cpu)
    });
    assert_eq!((half.width, half.height, half.rgba.len()), (960, 540, 960 * 540 * 4));
    assert_eq!((hd.width, hd.height, hd.rgba.len()), (1280, 720, 1280 * 720 * 4), "a spare of another size is reused");
    // Exactly half: the GPU's bilinear filter is the box average, so it is the CPU's scaling within rounding.
    for y in (0..540).step_by(9) {
        for x in (0..960).step_by(7) {
            assert!(near(half.pixel(x, y), cpu.pixel(x, y), 2), "({x},{y}): {:?} against {:?}", half.pixel(x, y), cpu.pixel(x, y));
        }
    }
    assert_eq!(half.pixel(2, 2), [255, 0, 0, 255]);
    assert_eq!(half.pixel(99, 49), [255, 0, 0, 255], "the red block ends at 100, 50");
    assert_eq!(half.pixel(101, 51), [0, 255, 0, 255]);
    assert_eq!(half.pixel(957, 537), [0, 0, 255, 255]);
    assert_eq!(half.pixel(475, 250), [0, 0, 0, 255], "the hole");
    // Three to two: the blocks sit where a third off their places says.
    assert_eq!(hd.pixel(3, 3), [255, 0, 0, 255]);
    assert_eq!(hd.pixel(132, 65), [255, 0, 0, 255], "inside the red block, whose edge is at 133 and 66");
    assert_eq!(hd.pixel(1276, 716), [0, 0, 255, 255]);
    assert_eq!(hd.pixel(633, 333), [0, 0, 0, 255], "the hole's centre, (950, 500) in the output's pixels");
    assert_eq!(hd.pixel(640, 60), [0, 255, 0, 255]);
}

#[test]
fn the_scratch_target_is_given_back_and_made_again_and_the_pages_are_untouched() {
    let Some((server, _client)) = screen() else { return };
    let ((first, again), page_before, page_after, errors) = server.eval(|s| {
        s.tick();
        let page_before = s.output.capture().unwrap();
        let first = s.output.grab((640, 360), Vec::new()).unwrap();
        s.output.grab_end();
        // A different size after the end: a new target, the same picture.
        let again = s.output.grab((640, 360), Vec::new()).unwrap();
        s.output.grab_end();
        s.output.grab_end();
        (((first), again), page_before, s.output.capture().unwrap(), s.stats.gl_errors)
    });
    assert_eq!(first, again, "the same page at the same size reads the same, before and after the scratch target went");
    assert_eq!(page_before, page_after, "a grab draws into no page");
    assert_eq!(errors, 0);
}

#[test]
fn a_deferred_grab_returns_each_picture_a_call_later_with_its_tag_and_the_last_one_on_finish() {
    let Some((server, _client)) = screen() else { return };
    let (sync, first, second, third, last, none, errors) = server.eval(|s| {
        s.tick();
        let size = (640, 360);
        let sync = s.output.grab(size, Vec::new()).unwrap();
        let first = s.output.grab_deferred(size, Vec::new(), 10).unwrap();
        let second = s.output.grab_deferred(size, Vec::new(), 20).unwrap();
        let third = s.output.grab_deferred(size, Vec::new(), 30).unwrap();
        let last = s.output.grab_finish(size, Vec::new()).unwrap();
        let none = s.output.grab_finish(size, Vec::new()).unwrap();
        s.output.grab_end();
        (sync, first, second, third, last, none, s.stats.gl_errors)
    });
    assert_eq!(first, None, "nothing is back for the first call");
    let (tag2, img2) = second.expect("the first picture, a call later");
    let (tag3, img3) = third.expect("the second");
    let (tag4, img4) = last.expect("the third, taken by finish");
    assert_eq!((tag2, tag3, tag4), (10, 20, 30));
    assert!(img2 == sync && img3 == sync && img4 == sync, "the same picture as the synchronous read, alpha opaque");
    assert_eq!(none, None, "taken once");
    assert_eq!(errors, 0);
}
