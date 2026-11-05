//! The weather's sky as the production engine takes it, drawn by Mesa and spoken to in the protocol's own requests (the app's `Layer::animate_stepped` is
//! `Request::Animate` with a step): an ambient motion asks for a frame a step apart and none at every refresh, a commit that only restates it asks for none,
//! it is drawn where the motion is at the frame's own time, and the app is told of a frame only when the frame has news for it.
use compositor_protocol::{
    Content, Ease, Event, Fill, LayerId, PROTOCOL_VERSION, Props, Request, Rgba, Shape, read_frame, write_frame,
};
use lumatv_render::host::{Engine, HostConfig};
use lumatv_render::{Demand, Gles};
use std::io;
use std::os::unix::net::UnixStream;

const W: u32 = 256;
const H: u32 = 128;
const MS: u64 = 1_000_000;
/// The time of the first frame; every other time is a number of milliseconds after it.
const T: u64 = 1_000_000_000;

struct Rig {
    engine: Engine,
    app: UnixStream,
    tag: u64,
    shm: std::path::PathBuf,
}

impl Rig {
    fn new() -> Self {
        static RIGS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = RIGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let shm = std::env::temp_dir().join(format!("lumatv-ambient-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&shm).unwrap();
        let gl = Gles::software_pbuffer(c"libEGL.so.1", W, H).expect("Mesa EGL");
        let (mut engine, mut app) = Engine::new(gl, HostConfig {
            width: W, height: H, refresh_hz: 120.0, shm_dir: shm.clone(), direct: false,
            gpu_sync: true, idle_trim: false, pages: 3, reference: true, blit: true,
            external: false, system_z: 1000,
        }).unwrap();
        write_frame(&mut app, &Request::Hello { version: PROTOCOL_VERSION }).unwrap();
        let welcome = read_frame::<Event>(&mut app, &mut Vec::new()).unwrap();
        assert!(matches!(welcome, Some(Event::Welcome { .. })), "{welcome:?}");
        engine.finish_handshake().unwrap();
        app.set_nonblocking(true).unwrap();
        Self { engine, app, tag: 0, shm }
    }

    fn send(&mut self, request: Request) {
        write_frame(&mut self.app, &request).unwrap();
    }

    /// A white square of 64 by 64 at x = 0 in the middle rows.
    fn square(&mut self, id: u32) {
        let shape = Shape { corner_radius: 0.0, fill: Fill::Solid(Rgba(255, 255, 255, 255)), border_width: 0.0, border: Rgba(0, 0, 0, 0) };
        self.send(Request::Create { id: LayerId(id), width: 64, height: 64, z: 1, content: Content::Shape(shape), props: at(0.0) });
    }

    fn animate(&mut self, id: u32, x: f32, duration_ms: u32, step_ms: u32) {
        self.send(Request::Animate { id: LayerId(id), to: at(x), duration_ms, ease: Ease::Linear, step_ms });
    }

    /// What was sent goes to the engine as one commit, which is not yet taken in.
    fn commit(&mut self) {
        self.tag += 1;
        self.send(Request::Commit { tag: self.tag });
        self.engine.pump().unwrap();
    }

    /// The commit taken in at `at_ms`, as the compositor does between frames.
    fn take_in(&mut self, at_ms: u64) {
        self.engine.apply_waiting(T + at_ms * MS);
    }

    /// A frame at `at_ms`; the `Event::Frame`s the app was sent for it.
    fn frame(&mut self, at_ms: u64) -> usize {
        let now = T + at_ms * MS;
        let cost = self.engine.draw(now);
        assert_eq!(cost.gl_error, None);
        self.engine.frame_done(now, &cost);
        let mut told = 0;
        let mut buf = Vec::new();
        loop {
            match read_frame::<Event>(&mut self.app, &mut buf) {
                Ok(Some(Event::Error { message })) => panic!("native scene: {message}"),
                Ok(Some(event)) => told += usize::from(matches!(event, Event::Frame { .. })),
                Ok(None) => break,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("{e}"),
            }
        }
        told
    }

    /// Where the square's left edge is in the last frame: the first lit pixel of a row it crosses.
    fn left(&self) -> usize {
        let pixels = self.engine.read_window();
        let row = &pixels[(H as usize / 2) * W as usize * 4..][..W as usize * 4];
        row.chunks_exact(4).position(|p| p[0] > 128).expect("the square is in the frame")
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.shm);
    }
}

fn at(x: f32) -> Props {
    Props::at(x, 32.0)
}

#[test]
fn an_ambient_motion_wants_a_frame_a_step_after_the_last_none_at_every_refresh_and_none_for_a_commit_that_restates_it() {
    let mut rig = Rig::new();
    rig.square(1);
    rig.commit();
    rig.take_in(0);
    rig.frame(0);
    assert_eq!((rig.engine.demand(), rig.left()), (Demand::Idle, 0));

    // A commit of nothing but the ambient motion asks for no frame; the motion wants one a step after the last, not at every refresh.
    rig.animate(1, 100.0, 500, 33);
    rig.commit();
    assert!(rig.engine.has_waiting() && !rig.engine.has_commits(), "an ambient commit asks for no frame");
    rig.take_in(20);
    assert_eq!(rig.engine.demand(), Demand::At(T + 33 * MS));

    // The frame shows the motion where it is at the frame's time, 20 ms after it began (100 px in 500 ms), and the next is a step after it.
    rig.frame(40);
    assert_eq!(rig.engine.demand(), Demand::At(T + 73 * MS));
    assert!(rig.left().abs_diff(4) <= 1, "left edge at {}", rig.left());
    rig.frame(270);
    assert!(rig.left().abs_diff(50) <= 1, "left edge at {}", rig.left());

    // The app restates the path (it does, 2.5 times a second): the motion goes on from where it is, 52 px, to the new far end.
    rig.animate(1, 200.0, 500, 33);
    rig.commit();
    assert!(!rig.engine.has_commits(), "a restated ambient motion asks for no frame");
    rig.take_in(280);
    rig.frame(530);
    assert!(rig.left().abs_diff(126) <= 1, "left edge at {}", rig.left());

    // It lands where it was told to, and nothing is wanted after that.
    rig.frame(780);
    assert_eq!((rig.left(), rig.engine.demand()), (200, Demand::Idle));
}

#[test]
fn a_motion_at_every_refresh_is_still_drawn_at_every_refresh_beside_an_ambient_one_and_in_its_place() {
    let mut rig = Rig::new();
    rig.square(1);
    rig.square(2);
    rig.commit();
    rig.take_in(0);
    rig.frame(0);
    rig.animate(1, 100.0, 500, 33);
    rig.commit();
    rig.take_in(10);
    assert_eq!(rig.engine.demand(), Demand::At(T + 33 * MS));
    rig.animate(2, 50.0, 200, 0);
    rig.commit();
    assert!(rig.engine.has_commits(), "a motion at every refresh asks for its frame");
    rig.take_in(20);
    assert_eq!(rig.engine.demand(), Demand::EveryRefresh);

    // The sky's own layer taken over by an ordinary animation is an ordinary animation, and a set that ends it asks for nothing more.
    rig.animate(1, 90.0, 500, 0);
    rig.commit();
    rig.take_in(30);
    rig.frame(40);
    rig.frame(300);
    assert_eq!(rig.engine.demand(), Demand::EveryRefresh);
    rig.send(Request::Set { id: LayerId(1), props: at(7.0) });
    rig.commit();
    rig.take_in(310);
    rig.frame(320);
    assert_eq!(rig.engine.demand(), Demand::Idle);
}

#[test]
fn a_step_longer_than_a_second_is_cut_to_a_second() {
    let mut rig = Rig::new();
    rig.square(1);
    rig.commit();
    rig.take_in(0);
    rig.frame(0);
    rig.animate(1, 100.0, 20_000, 60_000);
    rig.commit();
    rig.take_in(0);
    assert_eq!(rig.engine.demand(), Demand::At(T + 1_000 * MS));
}

#[test]
fn the_app_is_told_of_a_frame_only_when_it_has_news_of_a_commit() {
    let mut rig = Rig::new();
    rig.square(1);
    rig.commit();
    rig.take_in(0);
    assert_eq!(rig.frame(0), 1, "the first frame shows the first commit");
    rig.animate(1, 100.0, 500, 33);
    rig.commit();
    rig.take_in(5);
    assert_eq!(rig.frame(40), 1, "the frame that first shows a commit says so");
    // The frames of the motion after that show no commit the app has not been told of: the UI process is not woken for them.
    let told: Vec<usize> = [73, 106, 139, 172].into_iter().map(|at| rig.frame(at)).collect();
    assert_eq!(told, [0, 0, 0, 0]);
    // The next commit is told as the first was.
    rig.animate(1, 200.0, 500, 33);
    rig.commit();
    rig.take_in(180);
    assert_eq!(rig.frame(205), 1);
    assert_eq!(rig.frame(238), 0);
}
