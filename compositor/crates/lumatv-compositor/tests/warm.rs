//! A buffer's first-use costs are paid at its commit, not in the frame that first draws it (Mesa's software GL under the
//! real `State`): the clients' pass imports a new buffer (`prepared` counts it before any frame exists), the frame that
//! then draws it imports nothing (`frame imports` stays 0) and shows it right. A buffer the client destroys is retired
//! by the frame that drops it and freed by a later one, with no GL error either way.

mod harness;

use harness::{Client, Server};
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";

#[test]
fn commit_imports_before_the_frame_and_a_destroyed_buffer_is_freed_after_one() {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return;
    }
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    let mut server = Server::start_with(Box::new(|| Box::new(Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap())));
    server.ticking.store(false, Ordering::SeqCst);
    let mut client = Client::connect(&mut server);
    let card = client.card("_WEBOS_WINDOW_TYPE_CARD", "Warm");
    let first = client.shm_buffer(300, 200, 0xff19_80e6);
    client.present(&card.surface, &first);
    client.roundtrip();
    let (prepared, drawn, errors) = server.eval(|s| (s.stats.prepared, s.stats.frames_drawn, s.stats.gl_errors));
    assert_eq!((prepared, drawn), (1, 0), "the commit imported the buffer before any frame was drawn");
    assert_eq!(errors, 0, "the warm-up draw is clean");

    server.ticking.store(true, Ordering::SeqCst);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 1));
    let (imports, image) = server.eval(|s| (s.stats.frame_imports, s.output.capture().unwrap()));
    assert_eq!(imports, 0, "the frame found its texture ready");
    assert_eq!(image.pixel(100, 100), [25, 128, 230, 255], "the shm content is uploaded by the frame, as before");

    let second = client.shm_buffer(300, 200, 0xffe6_8019);
    client.present(&card.surface, &second);
    assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= 2));
    first.destroy();
    client.roundtrip();
    // Two more frames: the first retires the dead texture, the next frees it.
    for n in 3..5 {
        client.present(&card.surface, &second);
        assert!(client.wait_until(Duration::from_secs(3), |s| s.frames_done >= n));
    }
    let (prepared, imports, errors, image) = server.eval(|s| (s.stats.prepared, s.stats.frame_imports, s.stats.gl_errors, s.output.capture().unwrap()));
    assert_eq!((prepared, imports, errors), (2, 0, 0));
    assert_eq!(image.pixel(100, 100), [230, 128, 25, 255], "the second buffer shows after the first was freed");
}
