//! The flowing focus, measured on the mirrors of the compositor's tracks (the same equations the compositor evaluates), sampled at
//! 120 Hz: the outgoing and the incoming lift move together and no frame between moves shows both at rest; a retarget mid-flight
//! keeps the velocity; a held key keeps one continuous motion within the pool; reduced motion is a crossfade.

use compositor_protocol::GroupPose;
use lumatv_ui::kit::flow::{Flow, Lifted, POOL, Spec};
use lumatv_ui::kit::focus::Id;
use lumatv_ui::kit::viewport::Reach;
use ui::geometry::Frame;
use std::cell::Cell;
use std::rc::Rc;
use ui::testing::harness;

const FRAME_NS: u64 = 8_333_333;
const MS: u64 = 1_000_000;

struct Rig {
    flow: Flow,
    clock: Rc<Cell<u64>>,
    lead: u64,
}

impl Rig {
    fn new(spec: Spec, tiles: u32) -> Self {
        let h = harness();
        let clock = Rc::new(Cell::new(1_000_000_000u64));
        let lead = 8 * MS;
        let c = Rc::clone(&clock);
        h.ui.scene.set_clock(move || c.get(), lead);
        let mut flow = Flow::new(&h.ui.scene, spec, POOL).expect("a pool of tracks");
        for i in 0..tiles {
            let rest = GroupPose { x: 100.0 * i as f32, y: 700.0, pivot_x: 80.0, pivot_y: 80.0, ..GroupPose::IDENTITY };
            let group = h.ui.scene.group(None, rest).expect("a group");
            let lit = h.ui.scene.group(Some(group.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).expect("a lit group");
            flow.lift(Id(i), Lifted { group, rest, scale: 1.12, rise: 4.0, lit: Some(lit) });
        }
        // the harness is kept alive by the scene handles the flow holds
        std::mem::forget(h);
        Self { flow, clock, lead }
    }

    fn now(&self) -> u64 {
        self.clock.get()
    }

    /// The time the screen shows what is asked for now.
    fn shown(&self) -> u64 {
        self.now() + self.lead
    }

    fn advance(&mut self, by_ns: u64) {
        self.clock.set(self.now() + by_ns);
    }

    fn go(&mut self, from: Option<u32>, to: Option<u32>) {
        let now = self.now();
        self.flow.go(from.map(Id), to.map(Id), now).expect("a settle");
    }

    fn lift(&self, id: u32, at: u64) -> f32 {
        self.flow.lift_at(Id(id), at)
    }
}

#[test]
fn the_outgoing_and_the_incoming_move_together_and_no_frame_shows_both_at_rest() {
    let mut rig = Rig::new(Spec::default(), 3);
    rig.go(None, Some(0));
    rig.advance(400 * MS);
    assert!(rig.lift(0, rig.shown()) > 0.99, "the first focus has lifted: {}", rig.lift(0, rig.shown()));
    rig.go(Some(0), Some(1));
    let start = rig.shown();
    let mut both_rest = 0;
    let mut frames = 0;
    let mut landed_at = None;
    for f in 0..60u64 {
        let at = start + f * FRAME_NS;
        let (a, b) = (rig.lift(0, at), rig.lift(1, at));
        frames += 1;
        if a < 0.02 && b < 0.02 {
            both_rest += 1;
        }
        if landed_at.is_none() && b > 0.99 && a < 0.01 {
            landed_at = Some(f * FRAME_NS / MS);
        }
        if f > 0 {
            let (pa, pb) = (rig.lift(0, at - FRAME_NS), rig.lift(1, at - FRAME_NS));
            assert!(a <= pa + 1e-4, "the outgoing only falls: frame {f} {pa} -> {a}");
            assert!(b >= pb - 1e-4, "the incoming only rises: frame {f} {pb} -> {b}");
        }
    }
    assert_eq!(both_rest, 0, "of {frames} frames, none showed both tiles at rest");
    let landed = landed_at.expect("the move lands within 500 ms");
    assert!((200..=330).contains(&landed), "a critically damped 260 ms spring lands in about 270 ms, landed at {landed} ms");
    assert!(rig.lift(1, start + 100 * MS) > 0.5 && rig.lift(0, start + 100 * MS) < 0.5, "half way after 100 ms, both moving");
}

#[test]
fn a_retarget_mid_flight_keeps_the_velocity_and_the_position() {
    let mut rig = Rig::new(Spec::default(), 3);
    rig.go(None, Some(0));
    rig.advance(400 * MS);
    rig.go(Some(0), Some(1));
    rig.advance(100 * MS);
    // the key comes back the other way while 1 is still rising: 1 turns from where it is, as fast as it was going
    let at = rig.shown();
    let (x_before, v_before) = (rig.lift(1, at), rig.flow.velocity_at(Id(1), at));
    assert!(x_before > 0.3 && x_before < 0.95, "mid-flight: {x_before}");
    assert!(v_before > 0.0, "rising: {v_before}");
    rig.go(Some(1), Some(0));
    let (x_after, v_after) = (rig.lift(1, at + MS / 10), rig.flow.velocity_at(Id(1), at + MS / 10));
    assert!((x_after - x_before).abs() < 0.01, "the position is continuous: {x_before} -> {x_after}");
    assert!((v_after - v_before).abs() < 0.1 * v_before.abs().max(1.0), "the velocity is continuous: {v_before} -> {v_after}");
    // and it comes down from there while 0 goes back up, together
    let later = at + 50 * MS;
    assert!(rig.lift(1, later) < x_before, "1 turns back");
    assert!(rig.lift(0, later) > rig.lift(0, at), "0 rises again");
    rig.advance(400 * MS);
    assert!(rig.lift(0, rig.shown()) > 0.99 && rig.lift(1, rig.shown()) < 0.01);
}

#[test]
fn a_held_key_is_one_continuous_motion_within_the_pool() {
    let mut rig = Rig::new(Spec::default(), 12);
    rig.go(None, Some(0));
    rig.advance(400 * MS);
    let mut current = 0u32;
    for next in 1..12u32 {
        rig.go(Some(current), Some(next));
        current = next;
        // every frame of the 40 ms between repeats: something is always lifting
        for f in 0..5u64 {
            let at = rig.shown() + f * FRAME_NS;
            let total: f32 = (0..12).map(|i| rig.lift(i, at)).sum();
            assert!(total > 0.3, "at key {next} frame {f} the lifts add up to {total}");
        }
        rig.advance(40 * MS);
        let now = rig.now();
        rig.flow.tidy(now);
        assert!(rig.flow.in_flight() <= POOL, "{} in flight", rig.flow.in_flight());
    }
    rig.advance(500 * MS);
    let at = rig.shown();
    assert!(rig.lift(11, at) > 0.99, "the last lands lifted: {}", rig.lift(11, at));
    for i in 0..11 {
        assert!(rig.lift(i, at) < 0.01, "tile {i} is at rest: {}", rig.lift(i, at));
    }
}

#[test]
fn reduced_motion_is_a_crossfade_within_the_ceiling() {
    let mut rig = Rig::new(Spec { reduced: true, ..Spec::default() }, 2);
    rig.go(None, Some(0));
    rig.advance(300 * MS);
    rig.go(Some(0), Some(1));
    let start = rig.shown();
    assert!(rig.lift(1, start + 60 * MS) > 0.4 && rig.lift(1, start + 60 * MS) < 0.6, "linear, half way at 60 of 120 ms");
    assert!(rig.lift(1, start + 125 * MS) > 0.99);
    assert!(rig.lift(0, start + 125 * MS) < 0.01);
    assert!(!rig.flow.settling_at(start + 130 * MS));
}

/// A row of 1200 by 70 under a viewport over `window` clipped to `clip`, lifted to the row scale: where the lint finds its plate and the clip.
fn row_in_viewport(clip: impl Fn(Frame) -> compositor_protocol::Rect) -> (Frame, Option<Frame>) {
    let h = harness();
    let clock = Rc::new(Cell::new(1_000_000_000u64));
    let c = Rc::clone(&clock);
    h.ui.scene.set_clock(move || c.get(), 8 * MS);
    let window = Frame::new(100.0, 200.0, 1200.0, 300.0);
    let viewport = h.ui.scene.group(None, GroupPose { clip: Some(clip(window)), ..GroupPose::IDENTITY }).expect("a viewport");
    let rest = GroupPose { x: window.x, y: window.y, pivot_x: 600.0, pivot_y: 35.0, ..GroupPose::IDENTITY };
    let group = h.ui.scene.group(Some(viewport.id()), rest).expect("a group");
    let lit = h.ui.scene.group(Some(group.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).expect("a lit group");
    let mut flow = Flow::new(&h.ui.scene, Spec::default(), POOL).expect("a pool of tracks");
    flow.lift(Id(1), Lifted { group, rest, scale: 1.02, rise: 0.0, lit: Some(lit) });
    flow.go(None, Some(Id(1)), clock.get()).expect("a settle");
    clock.set(clock.get() + 100 * MS);
    // mid-flight: the plate is reported at full lift, wherever the spring is
    let at = clock.get() + 8 * MS;
    let lit_plate = flow.lifted_plate(Id(1), at).expect("a plate");
    std::mem::forget((h, viewport));
    lit_plate
}

#[test]
fn a_focused_row_is_cut_by_a_clip_that_is_its_window_and_whole_in_the_kits_viewport_clip() {
    let (plate, clip) = row_in_viewport(|w| compositor_protocol::Rect { x: w.x, y: w.y, w: w.w, h: w.h });
    let clip = clip.expect("a clip");
    assert!((plate.x - 88.0).abs() < 0.01 && (plate.w - 1224.0).abs() < 0.01, "the row at 1.02 leaves its window by 12 px each side: {plate:?}");
    assert!(plate.x < clip.x - 11.0 && plate.right() > clip.right() + 11.0, "a clip that is the window cuts it: {plate:?} in {clip:?}");
    let (plate, clip) = row_in_viewport(|w| Reach::of(1.02, 0.0, (1200.0, 70.0)).clip(w));
    let clip = clip.expect("a clip");
    assert!(plate.x >= clip.x && plate.y >= clip.y && plate.right() <= clip.right() && plate.bottom() <= clip.bottom(), "the kit's clip leaves it whole: {plate:?} in {clip:?}");
}
