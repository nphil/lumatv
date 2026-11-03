//! Transitions on the KMS output's frame logic (`kms/frames.rs`: three scan-out pages whose row 0 is the top, the ring's damage), on Mesa's llvmpipe: the
//! pose program draws the mask the right way up on a page, and a first buffer the driver is still busy with is not drawn while the move waits for it
//! (it is seen from its start, not from its middle).

mod harness;

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use harness::{Client, Server, Window};
use lumatv_compositor::kms::frames::Frames;
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::render::egl::Platform;
use lumatv_compositor::render::gl::{BufferPath, Gl, TargetId};
use lumatv_compositor::scene::Plan;
use lumatv_compositor::surface::Surfaces;
use serde_json::json;
use wayland_client::protocol::wl_buffer::WlBuffer as ClientBuffer;
use wayland_server::protocol::wl_buffer::WlBuffer;

const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";
const MS: u64 = 1_000_000;
const TILE: (i32, i32, i32, i32) = (287, 699, 120, 120);

/// The KMS output without the device: the GL renderer, three pages of its own and the ring's frame logic. `hold_next` makes the next buffer a commit
/// prepares count as one the driver is busy with, until `release`.
struct ScanOut {
    gl: Gl,
    frames: Frames,
    pages: Vec<TargetId>,
    hold_next: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    held: Option<WlBuffer>,
}

impl Output for ScanOut {
    fn name(&self) -> &'static str {
        "scanout-test"
    }

    fn size(&self) -> (u32, u32) {
        self.gl.size()
    }

    fn query_buffer(&mut self, buffer: &WlBuffer) -> Option<((i32, i32), bool)> {
        self.gl.query_buffer(buffer)
    }

    fn prepare_buffer(&mut self, buffer: &WlBuffer) -> bool {
        let new = self.gl.prepare(buffer);
        if new && self.hold_next.swap(false, Ordering::SeqCst) {
            self.gl.hold_warm(buffer, true);
            self.held = Some(buffer.clone());
        }
        new
    }

    fn warming(&mut self) -> bool {
        if let Some(b) = self.held.take() {
            if self.release.load(Ordering::SeqCst) {
                self.gl.hold_warm(&b, false);
            } else {
                self.held = Some(b);
            }
        }
        self.gl.warming()
    }

    fn forget_buffer(&mut self, buffer: &WlBuffer) {
        self.gl.forget_buffer(buffer)
    }

    fn ui_needs_frame(&mut self) -> io::Result<bool> {
        Ok(self.gl.ui_needs_frame()? || self.gl.standin_due())
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
}

fn scan_out(hold_next: Arc<AtomicBool>, release: Arc<AtomicBool>) -> Server {
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    Server::start_with(Box::new(move || {
        let mut gl = Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap();
        let pages: Vec<TargetId> = (0..3).map(|_| gl.add_target().unwrap()).collect();
        let frames = Frames::new(pages.clone(), 1920, 1080);
        Box::new(ScanOut { gl, frames, pages, hold_next, release, held: None })
    }))
}

struct Rig {
    client: Client,
    stub: Window,
    _bufs: Vec<ClientBuffer>,
}

fn rig(server: &mut Server) -> Rig {
    server.ticking.store(false, Ordering::SeqCst);
    let mut client = Client::connect(server);
    let stub = client.card("_WEBOS_WINDOW_TYPE_CARD", "Stub");
    stub.webos.set_property("appId".into(), "com.webos.app.home".into());
    let clear = client.shm_buffer(64, 64, 0);
    client.present(&stub.surface, &clear);
    client.roundtrip();
    server.eval(|s| { s.scene.set_home_rust(true); });
    server.eval(|s| {
        s.motion.forced = Some(true);
        s.apply_motion();
        s.set_launch_origin(&json!({"appId": "com.example.a", "tile": {"x": TILE.0, "y": TILE.1, "width": TILE.2, "height": TILE.3, "radius": 22}}));
    });
    Rig { client, stub, _bufs: vec![clear] }
}

fn tick(server: &Server, ns: u64) -> Image {
    server.eval(move |s| {
        s.tick_at(ns);
        s.output.capture().unwrap()
    })
}

fn app(rig: &mut Rig, pattern: bool) -> (Window, ClientBuffer) {
    let win = rig.client.card("_WEBOS_WINDOW_TYPE_CARD", "A");
    win.webos.set_property("appId".into(), "com.example.a".into());
    let buf = if pattern {
        rig.client.shm_pattern(1920, 1080, |x, y| 0xff00_0040 | (((x * 255 / 1919) as u32) << 16) | (((y * 255 / 1079) as u32) << 8))
    } else {
        rig.client.shm_buffer(1920, 1080, 0xffd0_4040)
    };
    let whole = rig.client.region(0, 0, 1920, 1080);
    win.surface.set_opaque_region(Some(&whole));
    rig.client.present(&win.surface, &buf);
    rig.client.roundtrip();
    (win, buf)
}

/// A launch and a Home press on three scan-out pages (row 0 at the top) draws what the software renderer does: where the picture is, which way up, and the
/// card's rounded mask.
#[test]
fn a_move_on_scan_out_pages_is_the_software_renderers_picture() {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return;
    }
    let instants = [0u64, 50, 110, 170, 219, 400, 450, 520, 590, 650];
    let run = |gl: bool| -> Vec<Image> {
        let mut server = if gl { scan_out(Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false))) } else { Server::start() };
        let mut r = rig(&mut server);
        server.eval(|s| {
            s.tick_at(1_000 * MS);
        });
        let (_a, _buf) = app(&mut r, true);
        let mut out: Vec<Image> = instants[..5].iter().map(|ms| tick(&server, 2_000 * MS + ms * MS)).collect();
        out.push(tick(&server, 2_400 * MS));
        r.stub.webos.set_state(3);
        r.client.roundtrip();
        out.extend([0u64, 60, 120, 181].iter().map(|ms| tick(&server, 3_000 * MS + ms * MS)));
        out
    };
    let (soft, gl) = (run(false), run(true));
    let (mut off, mut total) = (0, 0);
    for (n, (a, b)) in soft.iter().zip(gl.iter()).enumerate() {
        for y in (3..1080).step_by(23) {
            for x in (3..1920).step_by(29) {
                let (p, q) = (a.pixel(x, y), b.pixel(x, y));
                let d = p.iter().zip(q.iter()).map(|(m, n)| m.abs_diff(*n)).max().unwrap();
                total += 1;
                if d > 6 {
                    off += 1;
                    if off < 6 {
                        eprintln!("frame {n} ({x},{y}): software {p:?} scan-out {q:?}");
                    }
                }
            }
        }
    }
    assert!(off * 200 <= total, "{off} of {total} samples differ by more than 6");
}

/// The first buffer of an app, which the driver is still busy with, is not drawn (what is under it shows): the move waits and is seen from its start.
#[test]
fn a_first_buffer_the_driver_is_busy_with_is_not_drawn_and_the_move_waits_for_it() {
    if !Path::new(MESA).exists() {
        eprintln!("skipped: no Mesa at {MESA}");
        return;
    }
    let (hold_next, release) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let mut server = scan_out(hold_next.clone(), release.clone());
    let mut r = rig(&mut server);
    let tile_centre = ((TILE.0 + TILE.2 / 2) as u32, (TILE.1 + TILE.3 / 2) as u32);
    let home_at_tile = tick(&server, 1_000 * MS).pixel(tile_centre.0, tile_centre.1);
    hold_next.store(true, Ordering::SeqCst);
    let (_a, _buf) = app(&mut r, false);
    // while the driver is busy: the plan runs (the page to stand in from is there), the app's picture is not drawn, the clock keeps starting again
    let mut t = 2_000 * MS;
    for _ in 0..4 {
        let img = tick(&server, t);
        assert_eq!(img.pixel(tile_centre.0, tile_centre.1), home_at_tile, "Home at the tile: the picture is not drawn yet");
        assert_eq!(img.pixel(960, 540), tick(&server, t).pixel(960, 540));
        t += 16 * MS;
    }
    assert!(server.eval(|s| s.scene.transitions.active()));
    release.store(true, Ordering::SeqCst);
    // the first frame that shows the app: at its start (a quarter seen over the tile), not part-way through its move
    let mut first = None;
    for _ in 0..300 {
        let img = tick(&server, t);
        if img.pixel(tile_centre.0, tile_centre.1) != home_at_tile {
            first = Some((t, img));
            break;
        }
        // (the host's GPU is a few threads of llvmpipe: the clock goes at the pace its warm does, as the set's does)
        t += MS;
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let (t_first, img) = first.expect("the app was drawn once the buffer was warm");
    let p = img.pixel(tile_centre.0, tile_centre.1);
    let (h, a) = (home_at_tile, [0xd0u8, 0x40, 0x40]);
    for c in 0..3 {
        let want = (a[c] as f32 * 0.25 + h[c] as f32 * 0.75) as i32;
        assert!((p[c] as i32 - want).abs() <= 8, "a quarter of the app over the tile, as at the start of the move: {p:?} want about {want}");
    }
    // 100 ms later the move is 100 ms in, not 100 ms plus the wait
    let mid = tick(&server, t_first + 100 * MS);
    assert!(mid.pixel(960, 540)[0] > 0xa0, "the card is across the middle of the screen by then: {:?}", mid.pixel(960, 540));
    assert_eq!(server.eval(|s| s.stats.gl_errors), 0);
    drop(r);
}
