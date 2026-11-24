//! Transitions on the KMS output's frame logic (`kms/frames.rs`: three scan-out pages whose row 0 is the top, the ring's damage), on Mesa's llvmpipe: the
//! pose program draws the mask the right way up on a page, and a first buffer the driver is still busy with is not drawn while the move waits for it
//! (it is seen from its start, not from its middle).

mod harness;

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

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

/// Mesa has one EGL display per process: every `Gl` here opens it, and dropping an `Egl` terminates it. A test still running then finds its pending
/// warm fences answering `EGL_NOT_INITIALIZED` for good (Mesa 25.0.7), so its buffers count as warming until the 100 ms of wall clock of
/// `WARM_LIMIT` have passed, which ticks on a fake clock do not wait for, and its frames show the old picture. The tests of this file take
/// turns, a turn lasting until the `Gl` is dropped.
struct Turn;

static TURNS: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

impl Turn {
    fn take() -> Turn {
        let mut taken = TURNS.0.lock().unwrap_or_else(|e| e.into_inner());
        while *taken {
            taken = TURNS.1.wait(taken).unwrap_or_else(|e| e.into_inner());
        }
        *taken = true;
        Turn
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        *TURNS.0.lock().unwrap_or_else(|e| e.into_inner()) = false;
        TURNS.1.notify_one();
    }
}

/// The KMS output without the device: the GL renderer, three pages of its own and the ring's frame logic. `hold_next` makes the next buffer a commit
/// prepares count as one the driver is busy with, until `release`.
struct ScanOut {
    gl: Gl,
    frames: Frames,
    pages: Vec<TargetId>,
    hold_next: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    held: Option<WlBuffer>,
    acknowledge: Arc<AtomicBool>,
    fail_present: Arc<AtomicBool>,
    pending_receipt: Option<lumatv_compositor::snapshot::Receipt>,
    /// Last, so that the turn ends only once `gl` is dropped.
    _turn: Turn,
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

    fn freeze_app(&mut self, id: lumatv_compositor::snapshot::SnapshotId, window: lumatv_compositor::window::WindowId, plan: &Plan, live: &lumatv_compositor::snapshot::Snapshots) -> bool {
        self.gl.freeze_app(id, window, plan, live)
    }

    fn retain_app_snapshots(&mut self, snapshots: &lumatv_compositor::snapshot::Snapshots) {
        self.gl.retain_app_snapshots(snapshots);
    }

    fn app_snapshot_count(&self) -> usize {
        self.gl.app_snapshot_count()
    }

    fn take_snapshot_presented(&mut self) -> Option<lumatv_compositor::snapshot::Receipt> {
        if self.acknowledge.load(Ordering::SeqCst) { self.pending_receipt.take() } else { None }
    }

    fn ui_needs_frame(&mut self) -> io::Result<bool> {
        Ok(self.gl.ui_needs_frame()? || self.gl.standin_due())
    }

    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> io::Result<RenderStats> {
        let (_, stats) = self.frames.draw(&mut self.gl, plan, surfaces, None)?;
        Ok(stats)
    }

    fn present(&mut self) -> io::Result<Presented> {
        let presented = self.gl.present()?;
        let receipt = self.gl.take_snapshot_presented();
        if self.fail_present.swap(false, Ordering::SeqCst) {
            return Err(io::Error::other("controlled submission failure"));
        }
        if receipt.is_some() {
            self.pending_receipt = receipt;
        }
        Ok(presented)
    }

    fn capture(&mut self) -> io::Result<Image> {
        self.gl.set_target(self.pages[self.frames.last_page()]);
        self.gl.capture()
    }
}

fn scan_out(hold_next: Arc<AtomicBool>, release: Arc<AtomicBool>) -> Server {
    scan_out_with_receipts(hold_next, release, Arc::new(AtomicBool::new(true)), Arc::new(AtomicBool::new(false)))
}

fn scan_out_with_receipts(hold_next: Arc<AtomicBool>, release: Arc<AtomicBool>, acknowledge: Arc<AtomicBool>, fail_present: Arc<AtomicBool>) -> Server {
    let turn = Turn::take();
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    Server::start_with(Box::new(move || {
        let mut gl = Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap();
        let pages: Vec<TargetId> = (0..3).map(|_| gl.add_target().unwrap()).collect();
        let frames = Frames::new(pages.clone(), 1920, 1080);
        Box::new(ScanOut { gl, frames, pages, hold_next, release, held: None, acknowledge, fail_present, pending_receipt: None, _turn: turn })
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

/// The first buffer of an app, which the driver is still busy with, is not drawn (the frame is held, what is on the screen stays): the move waits and is
/// seen from its start.
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
    // while the driver is busy: the frame is held, so the app's picture is not drawn and the move has not begun
    let mut t = 2_000 * MS;
    for _ in 0..4 {
        let img = tick(&server, t);
        assert_eq!(img.pixel(tile_centre.0, tile_centre.1), home_at_tile, "Home at the tile: the picture is not drawn yet");
        t += 16 * MS;
    }
    assert_eq!(server.eval(|s| (s.stats.held_ticks, s.scene.transitions.active())), (4, false), "four held ticks planned nothing");
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

#[test]
fn a_resume_requires_fresh_drawable_content_and_a_real_receipt_without_rolling_back_when_the_next_buffer_warms() {
    assert!(Path::new(MESA).exists(), "this pixel regression requires Mesa");
    let hold = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let acknowledge = Arc::new(AtomicBool::new(false));
    let fail = Arc::new(AtomicBool::new(false));
    let mut server = scan_out_with_receipts(hold.clone(), release.clone(), acknowledge.clone(), fail.clone());
    let mut rig = rig(&mut server);
    tick(&server, 1_000 * MS);
    let (win, old) = app(&mut rig, false);
    tick(&server, 2_000 * MS);
    assert_eq!(tick(&server, 2_300 * MS).pixel(960, 540), [0xd0, 0x40, 0x40, 255]);
    rig.stub.webos.set_state(3);
    rig.client.roundtrip();
    assert_eq!(server.eval(|s| s.scene.snapshots.bytes()), lumatv_compositor::snapshot::PICTURE_BYTES);
    assert_eq!(server.eval(|s| s.output.app_snapshot_count()), 1);
    let background = rig.client.shm_buffer(1920, 1080, 0xff20_40e0);
    rig.client.present(&win.surface, &background);
    old.destroy();
    rig.client.roundtrip();
    assert_eq!(tick(&server, 3_000 * MS).pixel(960, 540), [0xd0, 0x40, 0x40, 255]);
    tick(&server, 3_200 * MS);
    win.webos.set_state(3);
    win.surface.frame(&rig.client.qh, ());
    win.surface.commit();
    rig.client.roundtrip();
    tick(&server, 4_000 * MS);
    assert_eq!(tick(&server, 4_250 * MS).pixel(960, 540), [0xd0, 0x40, 0x40, 255],
        "neither a background commit nor a callback-only resume replaces the saved picture");

    hold.store(true, Ordering::SeqCst);
    let c = rig.client.shm_buffer(1920, 1080, 0xff20_d060);
    rig.client.present(&win.surface, &c);
    rig.client.roundtrip();
    assert_eq!(tick(&server, 4_300 * MS).pixel(960, 540), [0xd0, 0x40, 0x40, 255]);
    release.store(true, Ordering::SeqCst);
    // the receipt's frame draws the live window under the saved picture: the picture is what shows, never a gap
    assert_eq!(tick(&server, 4_320 * MS).pixel(960, 540), [0xd0, 0x40, 0x40, 255], "the saved picture stays over the first live frame");
    assert_eq!(server.eval(|s| s.scene.snapshots.len()), 1, "storage waits for the output, not the draw");

    release.store(false, Ordering::SeqCst);
    hold.store(true, Ordering::SeqCst);
    let d = rig.client.shm_buffer(1920, 1080, 0xffd0_a020);
    rig.client.present(&win.surface, &d);
    rig.client.roundtrip();
    assert_eq!(tick(&server, 4_340 * MS).pixel(960, 540), [0xd0, 0x40, 0x40, 255],
        "C was never shown: the picture stays while the next buffer warms");
    acknowledge.store(true, Ordering::SeqCst);
    assert_eq!(tick(&server, 4_360 * MS).pixel(960, 540), [0xd0, 0x40, 0x40, 255], "the dissolve starts at the picture");
    assert_eq!(server.eval(|s| s.scene.snapshots.len()), 1, "the picture lives through its dissolve");
    release.store(true, Ordering::SeqCst);
    tick(&server, 4_380 * MS);
    let mid = tick(&server, 4_430 * MS).pixel(960, 540);
    assert!(mid != [0xd0, 0x40, 0x40, 255] && mid != [0xd0, 0xa0, 0x20, 255], "half-way the window shows through the picture: {mid:?}");
    assert_eq!(tick(&server, (4_360 + 140 + 20) * MS).pixel(960, 540), [0xd0, 0xa0, 0x20, 255]);
    assert_eq!(server.eval(|s| s.scene.snapshots.len()), 0);
    assert_eq!(server.eval(|s| s.output.app_snapshot_count()), 0);
    for ms in [4_540, 4_560, 4_580] {
        server.eval(|s| s.scene.redraw_all());
        let image = tick(&server, ms * MS);
        assert_eq!(image.pixel(2, 2), [0xd0, 0xa0, 0x20, 255], "no old rounded cover on any ring page");
    }

    rig.stub.webos.set_state(3);
    rig.client.roundtrip();
    tick(&server, 5_000 * MS);
    tick(&server, 5_200 * MS);
    win.webos.set_state(3);
    rig.client.roundtrip();
    tick(&server, 6_000 * MS);
    let fresh = rig.client.shm_buffer(1920, 1080, 0xffc0_30d0);
    rig.client.present(&win.surface, &fresh);
    rig.client.roundtrip();
    fail.store(true, Ordering::SeqCst);
    tick(&server, 6_250 * MS);
    assert_eq!(server.eval(|s| s.scene.snapshots.len()), 1, "a failed submission is not a presentation");
    assert!(!server.eval(|s| s.scene.snapshots.fading()));
    tick(&server, 6_270 * MS);
    tick(&server, 6_290 * MS);
    assert!(server.eval(|s| s.scene.snapshots.fading()), "the presented frame starts the dissolve");
    assert_eq!(tick(&server, (6_290 + 140 + 20) * MS).pixel(960, 540), [0xc0, 0x30, 0xd0, 255]);
    assert_eq!(server.eval(|s| s.scene.snapshots.len()), 0);
    assert_eq!(server.eval(|s| s.stats.gl_errors), 0);
}

#[test]
fn switcher_activation_discards_a_bypassed_picture_before_the_next_trip_home() {
    use std::time::{Duration, Instant};

    assert!(Path::new(MESA).exists(), "this pixel regression requires Mesa");
    let mut server = scan_out(Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let mut rig = rig(&mut server);
    tick(&server, 1_000 * MS);
    let (win, _) = app(&mut rig, false);
    tick(&server, 2_000 * MS);
    tick(&server, 2_300 * MS);
    rig.stub.webos.set_state(3);
    rig.client.roundtrip();
    tick(&server, 3_000 * MS);
    tick(&server, 3_200 * MS);
    assert_eq!(server.eval(|s| s.output.app_snapshot_count()), 1);

    // An explicit future clock keeps activation unfinished during State's ordinary Instant::now() frame, without a sleep.
    let start = Instant::now() + Duration::from_secs(60);
    server.eval(move |s| {
        assert!(s.switcher_open(None, start));
        s.switcher_op(&json!({"op":"layout", "ms":0, "cards":[{"id":"com.example.a","x":400,"y":300,"w":640,"h":360,"r":22}]}), start);
        s.switcher_op(&json!({"op":"activate", "id":"com.example.a"}), start);
    });
    tick(&server, 3_400 * MS);
    win.webos.set_state(3);
    let current = rig.client.shm_buffer(1920, 1080, 0xff20_d060);
    rig.client.present(&win.surface, &current);
    rig.client.roundtrip();
    tick(&server, 4_000 * MS);
    assert!(server.eval(|s| s.switcher_active()), "the app becomes foreground before expansion finishes");
    assert_eq!(server.eval(|s| s.output.app_snapshot_count()), 0, "the bypassed old picture cannot outlive a live switcher activation");
    server.eval(move |s| {
        s.switcher_frame(start + Duration::from_secs(4));
        assert!(!s.switcher_active());
    });
    assert_eq!(tick(&server, 4_300 * MS).pixel(960, 540), [0x20, 0xd0, 0x60, 255]);
    rig.stub.webos.set_state(3);
    rig.client.roundtrip();
    assert_eq!(tick(&server, 5_000 * MS).pixel(960, 540), [0x20, 0xd0, 0x60, 255],
        "the next minimize freezes current C, never pre-switcher A");
    tick(&server, 5_200 * MS);
    win.webos.set_state(3);
    rig.client.roundtrip();
    tick(&server, 6_000 * MS);
    assert_eq!(tick(&server, 6_250 * MS).pixel(960, 540), [0x20, 0xd0, 0x60, 255]);
    assert_eq!(server.eval(|s| s.stats.gl_errors), 0);
}

#[test]
fn a_shared_memory_update_hidden_by_a_warming_group_member_is_uploaded_before_its_receipt() {
    use std::os::fd::{AsFd, FromRawFd};
    use std::os::unix::fs::FileExt;
    use wayland_client::protocol::wl_shm::Format;

    assert!(Path::new(MESA).exists(), "this pixel regression requires Mesa");
    let hold = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let acknowledge = Arc::new(AtomicBool::new(false));
    let mut server = scan_out_with_receipts(hold.clone(), release.clone(), acknowledge.clone(), Arc::new(AtomicBool::new(false)));
    let mut rig = rig(&mut server);
    tick(&server, 1_000 * MS);
    let win = rig.client.card_as("_WEBOS_WINDOW_TYPE_CARD", "App", "com.example.a");
    // The same anonymous shared-memory buffer is edited in place; no new attachment or layout can force a full upload later.
    // SAFETY: a fixed NUL-terminated name and valid flags; the successful descriptor is owned below.
    let fd = unsafe { libc::memfd_create(c"snapshot-shm-update".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(fd >= 0);
    // SAFETY: this descriptor was just created and is owned only by this file.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut bytes = 0xffd0_4040u32.to_le_bytes().repeat(1920 * 1080);
    file.set_len(bytes.len() as u64).unwrap();
    file.write_all_at(&bytes, 0).unwrap();
    let pool = rig.client.shm.create_pool(file.as_fd(), bytes.len() as i32, &rig.client.qh, ());
    let buffer = pool.create_buffer(0, 1920, 1080, 1920 * 4, Format::Argb8888, &rig.client.qh, ());
    pool.destroy();
    rig.client.present(&win.surface, &buffer);
    let group = rig.client.groups.create_surface_group(&win.surface, "warming-snapshot-group".into(), &rig.client.qh, ());
    let _layer = group.create_layer("tools".into(), 1, &rig.client.qh, ());
    let child = rig.client.card_as("_WEBOS_WINDOW_TYPE_CARD", "Tools", "com.example.a");
    group.attach(&child.surface, "tools".into());
    let old_child = rig.client.shm_buffer(200, 120, 0xffd0_a020);
    rig.client.present(&child.surface, &old_child);
    rig.client.roundtrip();
    tick(&server, 2_000 * MS);
    assert_eq!(tick(&server, 2_300 * MS).pixel(960, 540), [0xd0, 0x40, 0x40, 255]);
    rig.stub.webos.set_state(3);
    rig.client.roundtrip();
    assert_eq!(server.eval(|s| s.output.app_snapshot_count()), 1, "the complete toplevel group must have a saved picture before resume");
    tick(&server, 3_000 * MS);
    tick(&server, 3_200 * MS);
    win.webos.set_state(3);
    rig.client.roundtrip();
    tick(&server, 4_000 * MS);
    tick(&server, 4_250 * MS);

    hold.store(true, Ordering::SeqCst);
    let next_child = rig.client.shm_buffer(200, 120, 0xff20_40e0);
    rig.client.present(&child.surface, &next_child);
    let green = 0xff20_d060u32.to_le_bytes();
    for pixel in bytes.chunks_exact_mut(4) { pixel.copy_from_slice(&green); }
    file.write_all_at(&bytes, 0).unwrap();
    win.surface.damage(0, 0, 1920, 1080);
    win.surface.commit();
    rig.client.roundtrip();
    assert_eq!(tick(&server, 4_300 * MS).pixel(960, 540), [0xd0, 0x40, 0x40, 255], "the incomplete group stays covered");
    assert_eq!(server.eval(|s| s.output.app_snapshot_count()), 1);
    release.store(true, Ordering::SeqCst);
    let ready = tick(&server, 4_320 * MS);
    assert_eq!(ready.pixel(960, 540), [0xd0, 0x40, 0x40, 255], "the picture covers the first live frame");
    assert_eq!(server.eval(|s| s.output.app_snapshot_count()), 1, "ready content still awaits its presentation receipt");
    acknowledge.store(true, Ordering::SeqCst);
    tick(&server, 4_340 * MS);
    tick(&server, 4_360 * MS);
    let done = tick(&server, (4_360 + 140 + 20) * MS);
    assert_eq!(done.pixel(960, 540), [0x20, 0xd0, 0x60, 255], "consumed damage must not turn old root pixels into a fresh receipt");
    assert_eq!(done.pixel(40, 40), [0x20, 0x40, 0xe0, 255], "the complete group includes its now-ready child");
    assert_eq!(server.eval(|s| s.output.app_snapshot_count()), 0);
    assert_eq!(server.eval(|s| s.stats.gl_errors), 0);
}

#[test]
fn a_dormant_video_export_allows_only_confirmed_rgb_snapshots_and_an_active_hole_stops_the_zoom() {
    use lumatv_compositor::transition::FRect;
    use lumatv_compositor::Rect;

    assert!(Path::new(MESA).exists(), "this pixel regression requires Mesa");
    let hold = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let mut server = scan_out(hold.clone(), release.clone());
    let mut rig = rig(&mut server);
    tick(&server, 1_000 * MS);
    let (win, _) = app(&mut rig, false);
    let exported = rig.client.foreign.export_element(&win.surface, 0, &rig.client.qh, ());
    rig.client.roundtrip();
    let window = server.eval(|s| s.scene.windows().find(|w| w.app_id() == "com.example.a").unwrap().id);
    let full = FRect::from_rect(Rect::new(0, 0, 1920, 1080));
    tick(&server, 2_000 * MS);
    server.eval(move |s| {
        let p = s.scene.transitions.pose(window, 2_000 * MS).unwrap();
        assert_eq!((p.rect, p.clip, p.radius), (full, full, 0.0), "an export still takes the conservative cold fade");
        assert_eq!(s.output.app_snapshot_count(), 0);
        assert!(s.scene.holes.is_empty(), "exporting without a context creates no video hole");
    });
    tick(&server, 2_300 * MS);

    // A changed, unfinished source cannot be confirmed. Becoming drawable later must not switch this same leave from a fade to a zoom.
    hold.store(true, Ordering::SeqCst);
    let current = rig.client.shm_buffer(1920, 1080, 0xff20_d060);
    rig.client.present(&win.surface, &current);
    rig.stub.webos.set_state(3);
    rig.client.roundtrip();
    tick(&server, 3_000 * MS);
    assert_eq!(server.eval(|s| s.output.app_snapshot_count()), 0);
    release.store(true, Ordering::SeqCst);
    tick(&server, 3_080 * MS);
    server.eval(move |s| {
        let p = s.scene.transitions.pose(window, 3_080 * MS).unwrap();
        assert_eq!((p.rect, p.clip, p.radius), (full, full, 0.0));
        assert_eq!(s.output.app_snapshot_count(), 0, "readiness alone cannot confirm a capture that was declined");
    });
    tick(&server, 3_200 * MS);
    win.webos.set_state(3);
    rig.client.roundtrip();
    tick(&server, 4_000 * MS);
    assert_eq!(tick(&server, 4_300 * MS).pixel(960, 540), [0x20, 0xd0, 0x60, 255]);

    rig.stub.webos.set_state(3);
    rig.client.roundtrip();
    assert_eq!(server.eval(|s| s.output.app_snapshot_count()), 1, "a dormant export cannot block a drawable RGB UI copy");
    assert_eq!(tick(&server, 5_000 * MS).pixel(960, 540), [0x20, 0xd0, 0x60, 255]);
    tick(&server, 5_100 * MS);
    assert!(server.eval(|s| s.scene.last_plan().snapshots[0].pose.clip.w < 1920.0), "the confirmed menu picture returns towards its icon");
    tick(&server, 5_200 * MS);
    win.webos.set_state(3);
    rig.client.roundtrip();
    tick(&server, 6_000 * MS);
    assert!(server.eval(|s| s.scene.last_plan().snapshots[0].pose.clip.w < 1920.0), "the saved RGB menu resumes from its icon");
    tick(&server, 6_080 * MS);

    let imported = rig.client.foreign.import_element(rig.client.state.window_ids.last().unwrap().0.clone(), 0, &rig.client.qh, ());
    imported.attach_punchthrough("snapshot-video".into());
    let region = rig.client.region(0, 0, 1920, 1080);
    exported.set_exported_window(&region, &region);
    let clear = rig.client.shm_buffer(1920, 1080, 0);
    rig.client.present(&win.surface, &clear);
    rig.client.roundtrip();
    let covered = tick(&server, 6_120 * MS);
    assert_eq!(covered.pixel(960, 540)[3], 255, "the new video hole remains veiled");
    server.eval(move |s| {
        assert_eq!(s.output.app_snapshot_count(), 0, "an actual hole invalidates the saved UI regardless of playing status");
        let track = s.scene.transitions.track_of(window).unwrap();
        let p = track.pose_at(6_120 * MS);
        assert_eq!((p.rect, p.clip, p.radius), (full, full, 0.0), "video never follows the interrupted icon zoom");
        assert_eq!(track.end_ns(), 6_220 * MS, "the change does not restart the motion clock");
        let surface = s.scene.window(window).unwrap().surface;
        assert!(!s.scene.video_hole_visible(window, surface, Rect::new(0, 0, 1920, 1080)));
    });
    tick(&server, 6_300 * MS);
    assert_eq!(tick(&server, 6_500 * MS).pixel(960, 540), [0, 0, 0, 0], "the stationary veil releases the real plane's hole");
    rig.stub.webos.set_state(3);
    rig.client.roundtrip();
    assert_eq!(server.eval(|s| s.output.app_snapshot_count()), 0, "a retained video hole remains excluded when returning Home");
    assert_eq!(server.eval(|s| s.stats.gl_errors), 0);
}
