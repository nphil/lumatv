//! The liquid lean through the production engine and Mesa's pixels: the flow speaks to the compositor in the protocol's own requests and the compositor
//! draws its own groups, tracks and retained pages. The incoming tile is drawn pushed the way the focus went and back where the flow's mirror says it is, the
//! outgoing one where a flow without a lean draws it, the pages agree with a full render in every frame, and a tile whose lean is taken off is drawn as one that never leaned.
//!
//! A run is one engine; runs go one after another. The engine draws into the GL context that was made current last, so two stages side by side draw into each
//! other (329 pixels of a lift disagreed with their full render in a lift-only run beside a leaning one).
use compositor_protocol::{Event, Fill, GroupPose, Props, Rgba, Shape};
use lumatv_render::Gles;
use lumatv_render::host::{Engine, HostConfig};
use lumatv_ui::kit::flow::{Flow, Lifted, POOL, Spec};
use lumatv_ui::kit::focus::Id;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use ui::group::Group;
use ui::scene::{Scene, SceneOptions, ShapeLayer};

const W: u32 = 320;
const H: u32 = 160;
const MS: u64 = 1_000_000;
const FRAME: u64 = 8_333_333;
const LEAD: u64 = 8 * MS;
const SIDE: u32 = 60;
/// Frames of the move that a run records: 533 ms, past the lift's 450 ms.
const FRAMES: usize = 64;

/// Two white tiles of 60 px in a row, the second 140 px right of the first, each lifting by 1.1 and 6 px, drawn at 120 Hz.
struct Stage {
    engine: Engine,
    scene: Scene,
    events: Receiver<Event>,
    flow: Flow,
    clock: Rc<Cell<u64>>,
    shm: std::path::PathBuf,
    _row: Group,
    _tiles: Vec<ShapeLayer>,
}

impl Stage {
    fn new(spec: Spec) -> Self {
        static STAGES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = STAGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let shm = std::env::temp_dir().join(format!("lumatv-lean-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&shm).unwrap();
        let gl = Gles::software_pbuffer(c"libEGL.so.1", W, H).expect("Mesa EGL");
        let (mut engine, app) = Engine::new(gl, HostConfig {
            width: W, height: H, refresh_hz: 120.0, shm_dir: shm.clone(), direct: false,
            gpu_sync: true, idle_trim: false, pages: 3, reference: true, blit: true,
            external: false, system_z: 1000,
        }).unwrap();
        let (scene, _, events) = Scene::over_with(app, SceneOptions { shm_dir: Some(shm.clone()), ..SceneOptions::default() }).unwrap();
        engine.finish_handshake().unwrap();
        let clock = Rc::new(Cell::new(1_000_000_000u64));
        let c = Rc::clone(&clock);
        scene.set_clock(move || c.get(), LEAD);
        let mut flow = Flow::new(&scene, spec, POOL).unwrap();
        let row = scene.group(None, GroupPose { x: 10.0, y: 20.0, ..GroupPose::IDENTITY }).unwrap();
        let mut tiles = Vec::new();
        for i in 0..2u32 {
            let rest = GroupPose { x: 20.0 + 140.0 * i as f32, y: 40.0, pivot_x: 30.0, pivot_y: 30.0, ..GroupPose::IDENTITY };
            let group = scene.group(Some(row.id()), rest).unwrap();
            let shape = Shape { corner_radius: 0.0, fill: Fill::Solid(Rgba(255, 255, 255, 255)), border_width: 0.0, border: Rgba(0, 0, 0, 0) };
            let tile = scene.shape(SIDE, SIDE, 1, shape, Props::at(0.0, 0.0));
            group.attach(&tile, Props::at(0.0, 0.0)).unwrap();
            flow.lift(Id(i), Lifted { group, rest, scale: 1.1, rise: 6.0, lit: None });
            tiles.push(tile);
        }
        Self { engine, scene, events, flow, clock, shm, _row: row, _tiles: tiles }
    }

    fn go(&mut self, from: Option<u32>, to: Option<u32>) {
        let now = self.clock.get();
        self.flow.go(from.map(Id), to.map(Id), now).unwrap();
    }

    /// One refresh: what the scene holds is committed and drawn at the clock plus the lead, and the clock goes on. The time drawn, the window's pixels and
    /// how many of them the pages got wrong against the frame drawn whole.
    fn frame(&mut self) -> (u64, Vec<u8>, u64) {
        let at = self.clock.get() + LEAD;
        let wanted = self.scene.has_pending().then(|| self.scene.try_commit().unwrap().get());
        let until = Instant::now() + Duration::from_secs(5);
        let mismatch = loop {
            self.scene.writer_ready().unwrap();
            self.engine.pump().unwrap();
            let cost = self.engine.draw(at);
            assert_eq!(cost.gl_error, None);
            self.engine.frame_done(at, &cost);
            self.engine.pump().unwrap();
            while let Ok(event) = self.events.try_recv() {
                if let Event::Error { message } = &event {
                    panic!("native scene: {message}");
                }
                self.scene.observe(&event);
            }
            if wanted.is_none_or(|tag| cost.tag >= tag) {
                break cost.mismatch_px;
            }
            assert!(Instant::now() < until, "the commit did not arrive: wanted {wanted:?}, presented {}", cost.tag);
            std::thread::yield_now();
        };
        self.clock.set(self.clock.get() + FRAME);
        (at, self.engine.read_window(), mismatch)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.shm);
    }
}

/// Where the lit run of the middle row inside `from..to` starts, in px, with the coverage of its first pixel counted.
fn left_edge(pixels: &[u8], from: usize, to: usize) -> f32 {
    let row = (H / 2) as usize;
    let lit = |x: usize| f32::from(pixels[(row * W as usize + x) * 4]) / 255.0;
    let first = (from..to).find(|&x| lit(x) > 0.0).expect("the tile is in the row");
    first as f32 + 1.0 - lit(first)
}

/// One frame of the move: where each tile's left edge is drawn, where the flow's mirror puts it (`None` once a tile has been handed back), and how many pixels
/// of the frame the pages got wrong.
struct Shot {
    edges: [f32; 2],
    mirror: Option<[f32; 2]>,
    mismatch: u64,
}

struct Run {
    shots: Vec<Shot>,
    /// The window once the flow has tidied up after the move.
    last: Vec<u8>,
}

/// Focus on tile 0 at rest, then the move to tile 1, [`FRAMES`] of it; with `hand_back` the lift of tile 1 is taken from the flow after that frame of it.
fn run(spec: Spec, hand_back: Option<usize>) -> Run {
    let mut stage = Stage::new(spec);
    stage.go(None, Some(0));
    for _ in 0..60 {
        stage.frame();
    }
    stage.go(Some(0), Some(1));
    let (mut shots, mut taken) = (Vec::new(), None);
    for k in 0..FRAMES {
        let (at, pixels, mismatch) = stage.frame();
        let mirror = taken.is_none().then(|| [0, 1].map(|i| stage.flow.group(Id(i)).expect("a lift").world_at(at).transform.apply(0.0, 0.0).0));
        shots.push(Shot { edges: [left_edge(&pixels, 0, 130), left_edge(&pixels, 130, W as usize)], mirror, mismatch });
        if hand_back == Some(k) {
            taken = stage.flow.take_lift(Id(1));
        }
    }
    let now = stage.clock.get();
    stage.flow.tidy(now);
    Run { shots, last: stage.frame().1 }
}

#[test]
fn the_compositor_draws_the_lean_where_the_mirror_says_and_the_pages_match_a_full_render() {
    let lean = run(Spec { lean_px: 3.0, ..Spec::default() }, None);
    let plain = run(Spec::default(), None);
    // the tiles are 60 px, so a lean of 3 px is held to 1.5 px; the outgoing tile does not lean: how far each is drawn from where a flow without a lean draws it
    let pushed: Vec<[f32; 2]> = lean.shots.iter().zip(&plain.shots).map(|(a, b)| [a.edges[1] - b.edges[1], a.edges[0] - b.edges[0]]).collect();
    let furthest = |of: usize| pushed.iter().enumerate().map(|(k, p)| (p[of], k)).fold((0.0f32, 0), |best, now| if now.0 > best.0 { now } else { best });
    let (incoming, at_incoming) = furthest(0);
    assert!((1.4..1.6).contains(&incoming), "the incoming tile was pushed {incoming} px at most");
    assert!((4..=6).contains(&at_incoming), "the push peaked at frame {at_incoming}, 41 ms in");
    for (k, p) in pushed.iter().enumerate() {
        assert!(p[0] > -0.01, "frame {k}: the incoming tile was pushed away from the way the focus went: {p:?}");
        assert!(p[1].abs() < 0.01, "frame {k}: the outgoing tile moved off where a flow without a lean draws it: {p:?}");
        if k >= 30 {
            assert!(p[0].abs() < 0.01 && p[1].abs() < 0.01, "frame {k}: 250 ms on, a tile is still off its place by {p:?}");
        }
    }
    for (k, shot) in lean.shots.iter().enumerate() {
        let mirror = shot.mirror.expect("both lifts are in the flow");
        for i in 0..2 {
            assert!((mirror[i] - shot.edges[i]).abs() < 0.05, "frame {k}, tile {i}: drawn at {} and mirrored at {}", shot.edges[i], mirror[i]);
        }
    }
    assert!(lean.shots.iter().chain(&plain.shots).all(|s| s.mismatch == 0), "the pages and the frame drawn whole disagree");
    assert!(lean.last == plain.last, "the lean left something in the picture once its slots were handed back");
}

#[test]
fn a_lift_handed_back_in_the_middle_of_its_lean_is_drawn_where_it_rests_and_the_pages_match_a_full_render() {
    // the lift of tile 1 is taken from the flow 41 ms into the move, as the lean peaks
    let lean = run(Spec { lean_px: 3.0, ..Spec::default() }, Some(5));
    let plain = run(Spec::default(), Some(5));
    for (k, (a, b)) in lean.shots.iter().zip(&plain.shots).enumerate() {
        assert_eq!((a.mismatch, b.mismatch), (0, 0), "frame {k}: the pages and the frame drawn whole disagree");
        if k > 5 {
            assert!((a.edges[1] - b.edges[1]).abs() < 0.01, "frame {k}: tile 1 is drawn at {} where a flow with no lean draws it at {}", a.edges[1], b.edges[1]);
        }
    }
    assert!(lean.last == plain.last, "the lean left something in the picture once its slots were handed back");
}
