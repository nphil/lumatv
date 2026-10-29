//! Direct scan-out (`scanout`) under the real `State::tick` and real clients: an output that makes the KMS output's decisions without a device
//! (the plane of the set, imports that succeed or are refused as a test says, flips that land when it says), over the software renderer for
//! the frames that are composed. What it checks is what the TV check list asks of the switch: the eligibility rules on real buffers, the
//! refusal that composes the same frame, the change of state when a system layer comes and goes, and that a buffer the display reads is
//! neither released to its client nor has its framebuffer destroyed until a later flip has replaced it.

mod harness;

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

use harness::{Client, Server};
use lumatv_compositor::buffer::{fourcc, modifier};
use lumatv_compositor::control;
use lumatv_compositor::output::{Image, Output, Presented, RenderStats};
use lumatv_compositor::render::software::Software;
use lumatv_compositor::scanout::{Candidate, Device, ImportError, Imported, Mode, Policy, Scanout, Stage, meta};
use lumatv_compositor::scene::Plan;
use lumatv_compositor::surface::Surfaces;
use lumatv_wl_protocols::mali_buffer_sharing::client::mali_buffer_sharing::MaliBufferSharing;
use parking_lot::Mutex;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_server::protocol::wl_buffer::WlBuffer as ServerBuffer;

const WAIT: Duration = Duration::from_secs(5);

/// What each rendered frame was.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Frame {
    Direct(u32),
    Composed(String),
}

/// What a test sets and reads of the output it cannot reach through `State`.
#[derive(Default)]
struct Knobs {
    /// Flips land while true; while false the frame stays on its way and the screen keeps the one before it.
    stuck: AtomicBool,
    /// The errno the next imports are refused with (0: they succeed).
    refuse_import: AtomicI32,
    refuse_acquire: AtomicBool,
    /// The display's mode is not set yet while true (the session's first commit has not gone out).
    not_live: AtomicBool,
    /// The errno the driver commits are refused with (0: they go out).
    refuse_commit: AtomicI32,
    log: Mutex<Log>,
}

#[derive(Default)]
struct Log {
    frames: Vec<Frame>,
    /// Where the plane put each direct frame (`Choice::at`).
    places: Vec<(i32, i32)>,
    imports: Vec<(u32, u64)>,
    destroyed: Vec<u32>,
    acquires: u32,
    next_fb: u32,
}

/// The primary plane of the set: AR24/XR24 linear, AB24/XB24 linear and AFBC `...62`.
struct Plane {
    knobs: Arc<Knobs>,
}

impl Device for Plane {
    fn mode(&self) -> (i32, i32) {
        (1920, 1080)
    }

    fn live(&self) -> bool {
        !self.knobs.not_live.load(Ordering::SeqCst)
    }

    fn lists(&self, format: u32, m: u64) -> bool {
        let formats = [fourcc::ARGB8888, fourcc::XRGB8888, fourcc::ABGR8888, fourcc::XBGR8888];
        formats.contains(&format) && (m == modifier::LINEAR || (m == modifier::AFBC_32X8_SPLIT_SPARSE && matches!(format, fourcc::ABGR8888 | fourcc::XBGR8888)))
    }

    fn lists_format(&self, format: u32) -> bool {
        [fourcc::ARGB8888, fourcc::XRGB8888, fourcc::ABGR8888, fourcc::XBGR8888].contains(&format)
    }

    fn import(&mut self, c: &Candidate, _fd: BorrowedFd<'_>) -> Result<Imported, ImportError> {
        let errno = self.knobs.refuse_import.load(Ordering::SeqCst);
        let mut log = self.knobs.log.lock();
        log.imports.push((c.format, c.modifier));
        if errno != 0 {
            return Err(ImportError { stage: Stage::Prime, errno });
        }
        log.next_fb += 1;
        Ok(Imported { fb: log.next_fb, gem: 100 + log.next_fb })
    }

    fn destroy(&mut self, imported: Imported) {
        self.knobs.log.lock().destroyed.push(imported.fb);
    }

    fn acquire(&mut self, _buffer: &ServerBuffer) -> bool {
        self.knobs.log.lock().acquires += 1;
        !self.knobs.refuse_acquire.load(Ordering::SeqCst)
    }
}

/// The KMS output's frame logic without the device: the decision, the frame's life (drawn, committed, landed) and the composed frames drawn by the
/// software renderer, in the same frame that refused the plane.
struct DirectOut {
    inner: Software,
    scanout: Scanout,
    plane: Plane,
    knobs: Arc<Knobs>,
    /// A frame was rendered and not yet presented (the KMS output commits nothing on a tick that drew nothing).
    rendered: bool,
    pending: bool,
    compose_only: bool,
}

impl Output for DirectOut {
    fn name(&self) -> &'static str {
        "direct-test"
    }

    fn size(&self) -> (u32, u32) {
        self.inner.size()
    }

    fn query_buffer(&mut self, buffer: &ServerBuffer) -> Option<((i32, i32), bool)> {
        self.inner.query_buffer(buffer)
    }

    fn forget_buffer(&mut self, buffer: &ServerBuffer) {
        self.scanout.forget(&mut self.plane, buffer);
        self.inner.forget_buffer(buffer);
    }

    fn render(&mut self, plan: &Plan, surfaces: &Surfaces) -> std::io::Result<RenderStats> {
        let verdict = self.scanout.choose(&mut self.plane, plan, self.compose_only);
        let took = verdict.is_ok();
        self.scanout.settle(&verdict, took);
        self.scanout.drawn(verdict.as_ref().ok());
        let frame = match &verdict {
            Ok(c) => Frame::Direct(c.fb),
            Err(r) => Frame::Composed(r.key().to_string()),
        };
        let mut log = self.knobs.log.lock();
        if let Ok(c) = &verdict {
            log.places.push(c.at);
        }
        log.frames.push(frame);
        drop(log);
        self.rendered = true;
        if took { Ok(RenderStats { items: plan.items.len(), ..Default::default() }) } else { self.inner.render(plan, surfaces) }
    }

    fn present(&mut self) -> std::io::Result<Presented> {
        if !std::mem::take(&mut self.rendered) {
            return Ok(Presented::default());
        }
        let errno = self.knobs.refuse_commit.load(Ordering::SeqCst);
        if errno != 0 {
            // What the KMS output does with a commit the driver refused: the frame never went out, and a direct one is told to the decision.
            self.scanout.dropped();
            if let Some(Frame::Direct(fb)) = self.knobs.log.lock().frames.last().cloned() {
                self.scanout.refused_at_commit(fb, errno);
            }
            return Err(std::io::Error::from_raw_os_error(errno));
        }
        self.scanout.committed();
        self.pending = true;
        Ok(Presented { seq: 1, at_ms: lumatv_compositor::output::monotonic_ms(), pending: true, replaced: false })
    }

    fn dispatch_events(&mut self) -> std::io::Result<u32> {
        if self.pending && !self.knobs.stuck.load(Ordering::SeqCst) {
            self.pending = false;
            self.scanout.landed();
            self.scanout.reap(&mut self.plane);
            return Ok(1);
        }
        Ok(0)
    }

    fn capture(&mut self) -> std::io::Result<Image> {
        self.inner.capture()
    }

    fn set_compose_only(&mut self, on: bool) {
        self.compose_only = on;
    }

    fn holds(&self, buffer: &ServerBuffer) -> bool {
        self.scanout.holds(buffer)
    }

    fn scanout_report(&self) -> Option<serde_json::Value> {
        Some(self.scanout.report())
    }

    fn set_scanout(&mut self, mode: Mode) -> std::io::Result<()> {
        self.scanout.set_mode(mode);
        Ok(())
    }

    fn set_scanout_offset(&mut self, on: bool) -> std::io::Result<()> {
        self.scanout.set_offset(on);
        Ok(())
    }

    fn take_journal(&mut self) -> Vec<String> {
        self.scanout.take_journal()
    }
}

fn rig() -> (Server, Arc<Knobs>) {
    rig_with(Policy { mode: Mode::Direct, ..Policy::default() })
}

fn rig_with(policy: Policy) -> (Server, Arc<Knobs>) {
    let knobs = Arc::new(Knobs::default());
    let k = knobs.clone();
    let server = Server::start_with(Box::new(move || {
        let scanout = Scanout::new(policy.clone(), meta::Registry::default());
        Box::new(DirectOut {
            inner: Software::new(1920, 1080),
            scanout,
            plane: Plane { knobs: k.clone() },
            knobs: k,
            rendered: false,
            pending: false,
            compose_only: false,
        })
    }));
    (server, knobs)
}

/// A stand-in for a dma-buf: any fd carries the protocol, and the decision reads its size.
fn memfd(len: i64) -> OwnedFd {
    // SAFETY: a memfd for the test, sized below.
    let fd = unsafe { libc::memfd_create(c"scanout-test".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(fd >= 0);
    // SAFETY: the descriptor is ours.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: sizing the memfd.
    assert_eq!(unsafe { libc::ftruncate(fd.as_raw_fd(), len) }, 0);
    fd
}

/// A buffer as LG's libmali asks for one: `create_buffer(w, h, stride, format, modifier, fd)` on the driver's global (ours here).
fn dmabuf(client: &mut Client, mali: &MaliBufferSharing, size: (i32, i32), stride: u32, format: u32, m: u64) -> WlBuffer {
    let fd = memfd(i64::from(stride) * i64::from(size.1));
    mali.create_buffer(size.0, size.1, stride, format, (m >> 32) as u32, m as u32, fd.as_fd(), &client.qh, ())
}

/// A full-screen XB24 linear buffer: what the plane takes.
fn takeable(client: &mut Client, mali: &MaliBufferSharing) -> WlBuffer {
    dmabuf(client, mali, (1920, 1080), 7680, fourcc::XBGR8888, modifier::LINEAR)
}

fn frames(knobs: &Arc<Knobs>) -> Vec<Frame> {
    knobs.log.lock().frames.clone()
}

fn journal(server: &Server) -> Vec<String> {
    server.eval(|s| s.journal.clone())
}

/// Presents `buffer` and waits for the frame to land: the frame callback is the flip's.
fn show(client: &mut Client, surface: &wayland_client::protocol::wl_surface::WlSurface, buffer: &WlBuffer) {
    let before = client.state.frames_done;
    client.present(surface, buffer);
    assert!(client.wait_until(WAIT, |s| s.frames_done > before), "the frame did not land");
}

#[test]
fn a_full_screen_buffer_the_plane_takes_is_scanned_out_and_one_it_does_not_is_composed_in_the_same_frame() {
    let (mut server, knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");

    let a = takeable(&mut client, &mali);
    show(&mut client, &win.surface, &a);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Direct(1)), "XB24 linear, 1080p, pitch 7680: the plane's own list");

    // The client's own layout: 16x16 AFBC, which every recorded LG client buffer is.
    let b = dmabuf(&mut client, &mali, (1920, 1080), 7680, fourcc::ABGR8888, modifier::AFBC_16X16_YTR_SPARSE_TILED);
    let opaque = client.region(0, 0, 1920, 1080);
    win.surface.set_opaque_region(Some(&opaque));
    show(&mut client, &win.surface, &b);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("modifier".into())), "refused by the plane's list, composed at once");
    assert_eq!(knobs.log.lock().imports.len(), 1, "a layout the list refuses is never imported");

    // The first buffer again: its framebuffer is kept, no second import.
    show(&mut client, &win.surface, &a);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Direct(1)));
    assert_eq!(knobs.log.lock().imports.len(), 1);

    let report = server.eval(|s| s.scanout_report());
    assert!(report["frames"]["direct"].as_u64() >= Some(2) && report["frames"]["composed"].as_u64() >= Some(1), "{report}");
    assert!(report["refused"]["modifier"].as_u64() >= Some(1), "{report}");
    assert_eq!(report["last"]["state"], "direct");
    let lines = journal(&server);
    assert!(lines.iter().any(|l| l.starts_with("scanout: direct (1920x1080 XB24 linear as fb 1)")), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("scanout: composed (") && l.contains("AFBC 16x16")), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("scanout: refused: AB24 with AFBC 16x16")), "the refusal of the buffer is told once: {lines:?}");
    assert_eq!(lines.iter().filter(|l| l.starts_with("scanout: refused:")).count(), 1, "{lines:?}");
}

#[test]
fn every_rule_on_a_real_buffer_composes_the_frame_with_its_own_reason() {
    let (mut server, knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    let cases: Vec<(&str, WlBuffer)> = vec![
        ("geometry", dmabuf(&mut client, &mali, (1280, 720), 5120, fourcc::XBGR8888, modifier::LINEAR)),
        ("stride", dmabuf(&mut client, &mali, (1920, 1080), 8192, fourcc::XBGR8888, modifier::LINEAR)),
        ("format", dmabuf(&mut client, &mali, (1920, 1080), 7680, 0x3231_564e, modifier::LINEAR)),
        ("modifier", dmabuf(&mut client, &mali, (1920, 1080), 7680, fourcc::XRGB8888, modifier::AFBC_32X8_SPLIT_SPARSE)),
        ("too-small", {
            let fd = memfd(4096);
            mali.create_buffer(1920, 1080, 7680, fourcc::XBGR8888, 0, 0, fd.as_fd(), &client.qh, ())
        }),
    ];
    for (reason, buffer) in &cases {
        show(&mut client, &win.surface, buffer);
        assert_eq!(frames(&knobs).last(), Some(&Frame::Composed((*reason).into())), "{reason}");
    }
    // An shm buffer has no dma-buf at all, and a translucent picture (AB24, no opaque region) is not the screen's.
    let shm = client.shm_buffer(1920, 1080, 0xff10_2030);
    show(&mut client, &win.surface, &shm);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("shm".into())));
    let translucent = dmabuf(&mut client, &mali, (1920, 1080), 7680, fourcc::ABGR8888, modifier::LINEAR);
    win.surface.set_opaque_region(None);
    show(&mut client, &win.surface, &translucent);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("translucent".into())));
    // ... which the surface may declare opaque, and then the plane takes it.
    let region = client.region(0, 0, 1920, 1080);
    win.surface.set_opaque_region(Some(&region));
    show(&mut client, &win.surface, &translucent);
    assert!(matches!(frames(&knobs).last(), Some(Frame::Direct(_))));
    assert_eq!(
        server.eval(|s| s.scanout_report())["refused"]["geometry"].as_u64(),
        Some(1),
        "a window that is not the screen never gets as far as the buffer's rules"
    );
}

#[test]
fn a_system_layer_composes_the_frame_it_appears_in_and_the_plane_has_the_window_again_when_it_leaves() {
    let (mut server, knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let app = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    let sys = client.card("_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM", "LumaTV System");
    let a = takeable(&mut client, &mali);
    show(&mut client, &app.surface, &a);
    assert!(matches!(frames(&knobs).last(), Some(Frame::Direct(_))));

    // The system layer shows something (a volume bar): two pictures, the plane cannot blend them.
    let bar = client.shm_buffer(400, 80, 0xcc20_2020);
    show(&mut client, &sys.surface, &bar);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("layers".into())));
    // The app draws on under it: still composed.
    let a2 = takeable(&mut client, &mali);
    show(&mut client, &app.surface, &a2);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("layers".into())));

    // The layer has nothing to show any more: the shell says so, and the window's own buffer goes to the plane in the next frame.
    server.eval(|s| {
        let now = lumatv_compositor::output::monotonic_ns();
        s.scene.set_shell_system_live(false, now);
        s.scene.redraw_all();
    });
    let a3 = takeable(&mut client, &mali);
    show(&mut client, &app.surface, &a3);
    assert!(matches!(frames(&knobs).last(), Some(Frame::Direct(_))), "{:?}", frames(&knobs));

    let lines = journal(&server);
    let at = |pat: &str| lines.iter().position(|l| l.contains(pat));
    let (direct, composed) = (at("scanout: direct"), at("scanout: composed (2 pictures"));
    assert!(direct.is_some() && composed.is_some() && direct < composed, "{lines:?}");
    assert!(lines.iter().filter(|l| l.starts_with("scanout: direct (")).count() >= 2, "it went direct a second time: {lines:?}");
    let report = server.eval(|s| s.scanout_report());
    assert!(report["transitions"].as_u64() >= Some(3), "direct, composed, direct: {report}");
    assert!(report["refused"]["layers"].as_u64() >= Some(2), "{report}");
}

#[test]
fn the_shells_home_as_a_linear_buffer_is_taken_and_in_the_clients_own_layout_it_is_not() {
    let (mut server, knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let home = client.card("_WEBOS_WINDOW_TYPE_LUMATV_HOME", "LumaTV Home");
    // Qt's window is transparent (an alpha buffer), and the scene counts Home's picture opaque: it fills its window with a wallpaper.
    let linear = dmabuf(&mut client, &mali, (1920, 1080), 7680, fourcc::ABGR8888, modifier::LINEAR);
    show(&mut client, &home.surface, &linear);
    assert!(matches!(frames(&knobs).last(), Some(Frame::Direct(_))), "{:?}", frames(&knobs));
    let afbc = dmabuf(&mut client, &mali, (1920, 1080), 7680, fourcc::ABGR8888, modifier::AFBC_16X16_YTR_SPARSE_TILED);
    show(&mut client, &home.surface, &afbc);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("modifier".into())));
}

/// Home's buffers presented one after another while the OLED care's orbit walks one pixel a frame (a beat due at every frame rides on the
/// frame drawn anyway): the care's offset at each frame, with what the output did.
fn orbit_frames(server: &mut Server, client: &mut Client, mali: &MaliBufferSharing, home: &harness::Window, knobs: &Arc<Knobs>, n: usize) -> Vec<((i32, i32), Frame)> {
    let mut out = Vec::new();
    for _ in 0..n {
        let buf = takeable(client, mali);
        show(client, &home.surface, &buf);
        let offset = server.eval(|s| s.scene.care.offset());
        out.push((offset, frames(knobs).last().cloned().unwrap()));
    }
    out
}

#[test]
fn the_cares_orbit_is_the_planes_position_right_or_down_of_the_origin_when_allowed_and_composes_the_frame_otherwise() {
    let (mut server, knobs) = rig_with(Policy { mode: Mode::Direct, offset: true, ..Policy::default() });
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let home = client.card("_WEBOS_WINDOW_TYPE_LUMATV_HOME", "LumaTV Home");
    server.eval(|s| {
        s.scene.care.settings.enabled = true;
        s.scene.care.settings.radius = 1;
        s.scene.care.settings.step_ns = 1;
    });
    // Radius 1: the ring's eight points, one pixel a beat, beginning top left; one and a half turns show every point under both rules.
    let walked = orbit_frames(&mut server, &mut client, &mali, &home, &knobs, 14);
    let seen: std::collections::BTreeSet<(i32, i32)> = walked.iter().map(|(o, _)| *o).collect();
    assert!(seen.len() >= 8, "the orbit walked its ring: {walked:?}");
    for (offset, frame) in &walked {
        if offset.0 >= 0 && offset.1 >= 0 {
            assert!(matches!(frame, Frame::Direct(_)), "at {offset:?}: the plane carries a shift right or down: {walked:?}");
        } else {
            assert_eq!(frame, &Frame::Composed("shifted".into()), "at {offset:?}: a shift up or left is never the plane's (its position is unsigned)");
        }
    }
    // What the plane was told: the frames' places are the care's offsets, in order, for every direct frame.
    let direct_offsets: Vec<(i32, i32)> = walked.iter().filter(|(_, f)| matches!(f, Frame::Direct(_))).map(|(o, _)| *o).collect();
    assert_eq!(knobs.log.lock().places, direct_offsets);
    assert!(direct_offsets.iter().any(|&o| o != (0, 0)), "a placed frame happened: {walked:?}");
    let report = server.eval(|s| s.scanout_report());
    assert_eq!(report["offset"]["on"], true);
    assert!(report["offset"]["placedFrames"].as_u64() >= Some(1), "{report}");
    assert!(report["refused"]["shifted"].as_u64() >= Some(1), "{report}");
    let lines = journal(&server);
    assert!(lines.iter().any(|l| l.starts_with("scanout: direct at (1, ")), "the place is told when it changes: {lines:?}");

    // Switched off while running: the same orbit composes every shifted frame, and the plane is at the origin for the rest.
    assert!(server.eval(|s| s.set_scanout_offset(false).is_ok()));
    let places_before = knobs.log.lock().places.len();
    let walked = orbit_frames(&mut server, &mut client, &mali, &home, &knobs, 10);
    for (offset, frame) in &walked {
        if *offset == (0, 0) {
            assert!(matches!(frame, Frame::Direct(_)), "{walked:?}");
        } else {
            assert_eq!(frame, &Frame::Composed("shifted".into()), "at {offset:?}: {walked:?}");
        }
    }
    assert!(knobs.log.lock().places[places_before..].iter().all(|&p| p == (0, 0)));
    let report = server.eval(|s| s.scanout_report());
    assert_eq!(report["offset"]["on"], false);

    // The care off again: Home is back at the origin and direct, whatever the switch.
    server.eval(|s| s.scene.care.settings.enabled = false);
    let walked = orbit_frames(&mut server, &mut client, &mali, &home, &knobs, 2);
    assert!(matches!(walked.last(), Some(((0, 0), Frame::Direct(_)))), "{walked:?}");
}

#[test]
fn what_else_the_plan_has_is_named_and_not_scanned_out() {
    let (mut server, knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let app = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    let a = takeable(&mut client, &mali);
    show(&mut client, &app.surface, &a);
    let n = frames(&knobs).len();

    // The pointer shows: its sprite is drawn over the window.
    server.eval(|s| {
        s.seat.cursor_visible = true;
        s.seat.pointer_position = (500, 500);
    });
    let b = takeable(&mut client, &mali);
    show(&mut client, &app.surface, &b);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("cursor".into())));
    server.eval(|s| s.seat.cursor_visible = false);

    // The app has exported video: a hole is expected, and the video plane's picture is not the plane's to cover.
    let surface = server.eval(|s| s.scene.windows().next().unwrap().surface);
    server.eval(move |s| s.scene.set_video_expected(surface, true));
    let c = takeable(&mut client, &mali);
    show(&mut client, &app.surface, &c);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("video".into())));
    server.eval(move |s| s.scene.set_video_expected(surface, false));

    // The surface asks for a buffer transform: the plane cannot show it another way.
    app.surface.set_buffer_transform(wayland_client::protocol::wl_output::Transform::_90);
    let d = takeable(&mut client, &mali);
    show(&mut client, &app.surface, &d);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("transformed".into())));
    app.surface.set_buffer_transform(wayland_client::protocol::wl_output::Transform::Normal);
    let e = takeable(&mut client, &mali);
    show(&mut client, &app.surface, &e);
    assert!(matches!(frames(&knobs).last(), Some(Frame::Direct(_))), "it goes back when what was in the way is gone");
    assert!(frames(&knobs).len() > n);
}

#[test]
fn a_buffer_the_display_reads_is_not_released_to_its_client_until_a_later_flip_has_replaced_it() {
    let (mut server, knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    let a = takeable(&mut client, &mali);
    let b = takeable(&mut client, &mali);
    let (a_id, b_id) = (wayland_client::Proxy::id(&a).protocol_id(), wayland_client::Proxy::id(&b).protocol_id());
    show(&mut client, &win.surface, &a);

    // The flips stop landing (a late panel, an event the driver kept): B is committed, A is still what the panel scans.
    knobs.stuck.store(true, Ordering::SeqCst);
    client.present(&win.surface, &b);
    assert!(client.wait_until(WAIT, |_| knobs.log.lock().frames.len() >= 2));
    for _ in 0..10 {
        client.roundtrip();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(frames(&knobs).last(), Some(Frame::Direct(_))));
    assert!(!client.state.releases.contains(&a_id), "A is on the screen: the client may not draw into it ({:?})", client.state.releases);

    // The flip lands: B is on the screen, A is the client's again.
    knobs.stuck.store(false, Ordering::SeqCst);
    assert!(client.wait_until(WAIT, |s| s.releases.contains(&a_id)), "released once the display let go of it: {:?}", client.state.releases);
    assert!(!client.state.releases.contains(&b_id), "B is what the panel shows now");
}

#[test]
fn a_framebuffer_is_destroyed_when_the_display_has_let_go_of_it_and_not_before() {
    let (mut server, knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    let a = takeable(&mut client, &mali);
    let b = takeable(&mut client, &mali);
    show(&mut client, &win.surface, &a);
    show(&mut client, &win.surface, &b);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Direct(2)));

    // B goes to the plane, its flip does not land, and the client destroys A in the meantime (it is what the panel still scans).
    knobs.stuck.store(true, Ordering::SeqCst);
    let c = takeable(&mut client, &mali);
    client.present(&win.surface, &c);
    assert!(client.wait_until(WAIT, |_| knobs.log.lock().frames.len() >= 3));
    // (the frame before it is landed: B is on the screen and A is long gone from it, so A's framebuffer may go at once)
    b.destroy();
    client.roundtrip();
    std::thread::sleep(Duration::from_millis(30));
    assert!(!knobs.log.lock().destroyed.contains(&2), "B is on the screen: its framebuffer stays while it is read ({:?})", knobs.log.lock().destroyed);
    knobs.stuck.store(false, Ordering::SeqCst);
    assert!(client.wait_until(WAIT, |_| knobs.log.lock().destroyed.contains(&2)), "gone once a flip has replaced B: {:?}", knobs.log.lock().destroyed);
    assert!(!knobs.log.lock().destroyed.contains(&3), "C is on the screen");
}

#[test]
fn a_refusal_from_the_display_composes_the_same_frame_and_is_not_tried_again() {
    let (mut server, knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    knobs.refuse_import.store(libc::EINVAL, Ordering::SeqCst);
    let a = takeable(&mut client, &mali);
    show(&mut client, &win.surface, &a);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("import".into())), "the frame that was refused is composed, not skipped");
    for _ in 0..3 {
        show(&mut client, &win.surface, &a);
    }
    assert_eq!(knobs.log.lock().imports.len(), 1, "a buffer the driver refused is remembered");
    let lines = journal(&server);
    assert!(lines.iter().any(|l| l.contains("scattered pages") && l.starts_with("scanout: refused")), "{lines:?}");
    // A buffer made after the refusal is a new try, and the driver takes it now.
    knobs.refuse_import.store(0, Ordering::SeqCst);
    let b = takeable(&mut client, &mali);
    show(&mut client, &win.surface, &b);
    assert!(matches!(frames(&knobs).last(), Some(Frame::Direct(_))));
    assert_eq!(knobs.log.lock().imports.len(), 2);
}

#[test]
fn nothing_goes_to_the_plane_before_the_mode_is_set() {
    let (mut server, knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    knobs.not_live.store(true, Ordering::SeqCst);
    let a = takeable(&mut client, &mali);
    show(&mut client, &win.surface, &a);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("no-mode".into())), "the first commit carries the mode with a page");
    assert!(knobs.log.lock().imports.is_empty(), "no plane is asked before the display is up");
    knobs.not_live.store(false, Ordering::SeqCst);
    show(&mut client, &win.surface, &a);
    assert!(matches!(frames(&knobs).last(), Some(Frame::Direct(_))), "not remembered against the buffer: {:?}", frames(&knobs));
}

#[test]
fn a_commit_the_driver_refuses_is_composed_in_the_next_frame_and_not_sent_again() {
    let (mut server, knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    let a = takeable(&mut client, &mali);
    show(&mut client, &win.surface, &a);
    let b = takeable(&mut client, &mali);
    knobs.refuse_commit.store(libc::EINVAL, Ordering::SeqCst);
    client.present(&win.surface, &b);
    assert!(client.wait_until(WAIT, |_| knobs.log.lock().frames.len() >= 2));
    assert!(matches!(frames(&knobs).last(), Some(Frame::Direct(_))), "chosen before the driver said no: {:?}", frames(&knobs));
    knobs.refuse_commit.store(0, Ordering::SeqCst);
    // The client draws on: the buffer the driver refused is composed, and the commit is not tried with it again.
    show(&mut client, &win.surface, &b);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("test-commit".into())), "{:?}", frames(&knobs));
    let lines = journal(&server);
    assert!(lines.iter().any(|l| l.starts_with("scanout: refused: fb") && l.contains("at commit")), "{lines:?}");
}

#[test]
fn a_client_that_cannot_be_waited_for_composes_the_frame_but_is_not_remembered_against() {
    let (mut server, knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    let a = takeable(&mut client, &mali);
    knobs.refuse_acquire.store(true, Ordering::SeqCst);
    show(&mut client, &win.surface, &a);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("acquire".into())));
    knobs.refuse_acquire.store(false, Ordering::SeqCst);
    show(&mut client, &win.surface, &a);
    assert!(matches!(frames(&knobs).last(), Some(Frame::Direct(_))), "the barrier was the trouble, not the buffer");
    assert!(knobs.log.lock().acquires >= 2, "every direct frame waits for its client");
}

#[test]
fn a_capture_composes_every_frame_and_the_switch_works_at_run_time() {
    let (mut server, knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    let a = takeable(&mut client, &mali);
    show(&mut client, &win.surface, &a);
    assert!(matches!(frames(&knobs).last(), Some(Frame::Direct(_))));

    // A recording is armed: the screen must be in a page.
    server.eval(|s| s.output.set_compose_only(true));
    let b = takeable(&mut client, &mali);
    show(&mut client, &win.surface, &b);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("capture".into())));
    server.eval(|s| s.output.set_compose_only(false));

    // `ctl scanout off`: composed from the next frame on, `direct`: back.
    let reply = server.eval(|s| control::handle(s, "scanout off").text());
    let v: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!((v["ok"].as_bool(), v["scanout"]["mode"].as_str()), (Some(true), Some("off")), "{reply}");
    let n_off = frames(&knobs).len();
    client.wait_until(WAIT, |_| knobs.log.lock().frames.len() > n_off);
    let off = frames(&knobs);
    assert_eq!(off.last(), Some(&Frame::Composed("off".into())), "the screen is redrawn through the composition: {off:?}");
    let c = takeable(&mut client, &mali);
    show(&mut client, &win.surface, &c);
    assert_eq!(frames(&knobs).last(), Some(&Frame::Composed("off".into())));
    let reply = server.eval(|s| control::handle(s, "scanout direct").text());
    assert!(reply.contains("\"mode\":\"direct\""), "{reply}");
    let d = takeable(&mut client, &mali);
    show(&mut client, &win.surface, &d);
    assert!(matches!(frames(&knobs).last(), Some(Frame::Direct(_))));
    // `status` carries the same report, and a word the command does not know says what it takes.
    let status: serde_json::Value = serde_json::from_str(&server.eval(|s| control::handle(s, "status").text())).unwrap();
    assert_eq!(status["scanout"]["mode"], "direct");
    assert!(status["scanout"]["frames"]["direct"].as_u64() >= Some(2));
    assert!(server.eval(|s| control::handle(s, "scanout sideways").text()).contains("scanout [off|direct|offset on|off|shell-env"));
    // The plane offset's switch over the socket: on, read back, a word it does not take, off.
    let v: serde_json::Value = serde_json::from_str(&server.eval(|s| control::handle(s, "scanout offset on").text())).unwrap();
    assert_eq!((v["ok"].as_bool(), v["scanout"]["offset"]["on"].as_bool()), (Some(true), Some(true)), "{v}");
    assert!(server.eval(|s| s.config.scanout.offset));
    assert!(server.eval(|s| control::handle(s, "scanout offset sideways").text()).contains("scanout offset on|off"));
    let v: serde_json::Value = serde_json::from_str(&server.eval(|s| control::handle(s, "scanout offset off").text())).unwrap();
    assert_eq!(v["scanout"]["offset"]["on"].as_bool(), Some(false), "{v}");

    // The shell client's extra environment: set, read back in the report, refused when it is not NAME=VALUE, cleared.
    let reply = server.eval(|s| control::handle(s, "scanout shell-env WAYLAND_AFBC=0 A=b=c").text());
    let v: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(v["scanout"]["shellEnv"], serde_json::json!(["WAYLAND_AFBC=0", "A=b=c"]), "{reply}");
    assert_eq!(server.eval(|s| s.config.scanout_shell_env.clone()), vec![("WAYLAND_AFBC".to_string(), "0".to_string()), ("A".to_string(), "b=c".to_string())]);
    let reply = server.eval(|s| control::handle(s, "scanout shell-env nonsense").text());
    assert!(reply.contains("\"ok\":false") && reply.contains("NAME=VALUE"), "{reply}");
    assert_eq!(server.eval(|s| s.config.scanout_shell_env.len()), 2, "a refused line changes nothing");
    let reply = server.eval(|s| control::handle(s, "scanout shell-env none").text());
    let v: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(v["scanout"]["shellEnv"], serde_json::json!([]), "{reply}");
}

#[test]
fn the_tap_keys_a_buffer_the_way_the_output_looks_it_up() {
    let (mut server, _knobs) = rig();
    let mut client = Client::connect(&mut server);
    let mali: MaliBufferSharing = client.globals.bind(&client.qh, 5..=5, ()).unwrap();
    let win = client.card("_WEBOS_WINDOW_TYPE_CARD", "App");
    let a = takeable(&mut client, &mali);
    show(&mut client, &win.surface, &a);
    // The registry is the tap's; here a test files the descriptor, under the key of the resource the output will ask about.
    let found = server.eval(|s| {
        let buffer = s.surfaces.iter().find_map(|sf| sf.current.buffer.clone()).expect("the window has a buffer");
        let registry = meta::Registry::default();
        let key = meta::key_of(&buffer).expect("a resource has a key");
        let fd = memfd(4096);
        registry.insert(key, meta::Descriptor { width: 1920, height: 1080, stride: 7680, format: fourcc::XBGR8888, modifier: 0, fd, serial: 0 });
        (registry.lookup_buffer(&buffer).map(|d| d.stride), registry.lookup((key.0, key.1 + 1)).is_none())
    });
    assert_eq!(found, (Some(7680), true));
}
