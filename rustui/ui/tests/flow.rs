//! The flowing focus, measured on the mirrors of the compositor's tracks (the same equations the compositor evaluates), sampled at
//! 120 Hz: the outgoing and the incoming lift move together and no frame between moves shows both at rest; a retarget mid-flight
//! keeps the velocity; a held key keeps one continuous motion within the pool; reduced motion is a crossfade; the liquid lean rides on the
//! lift without changing it.

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
    scene: ui::scene::Scene,
    clock: Rc<Cell<u64>>,
    lead: u64,
}

impl Rig {
    /// `tiles` tiles in a row, 100 px apart.
    fn new(spec: Spec, tiles: u32) -> Self {
        Self::at(spec, &(0..tiles).map(|i| (100.0 * i as f32, 700.0)).collect::<Vec<_>>())
    }

    /// Tiles of 160 px with their top lefts at `places`.
    fn at(spec: Spec, places: &[(f32, f32)]) -> Self {
        let h = harness();
        let clock = Rc::new(Cell::new(1_000_000_000u64));
        let lead = 8 * MS;
        let c = Rc::clone(&clock);
        h.ui.scene.set_clock(move || c.get(), lead);
        let mut flow = Flow::new(&h.ui.scene, spec, POOL).expect("a pool of tracks");
        for (i, &(x, y)) in places.iter().enumerate() {
            let rest = GroupPose { x, y, pivot_x: 80.0, pivot_y: 80.0, ..GroupPose::IDENTITY };
            let group = h.ui.scene.group(None, rest).expect("a group");
            let lit = h.ui.scene.group(Some(group.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).expect("a lit group");
            flow.lift(Id(i as u32), Lifted { group, rest, scale: 1.12, rise: 4.0, lit: Some(lit) });
        }
        let scene = h.ui.scene.clone();
        // the harness is kept alive by the scene handles the flow holds
        std::mem::forget(h);
        Self { flow, scene, clock, lead }
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

/// Where tile `id` is centred on the screen at `at`: the lift's scale and rise keep it on its centre sideways, so what moves it there is the lean.
fn centre(rig: &Rig, id: u32, at: u64) -> (f32, f32) {
    rig.flow.group(Id(id)).expect("a group").world_at(at).transform.apply(80.0, 80.0)
}

/// The same tiles twice, one leaning and one not, taken through the same moves on the same clock: the difference of their centres is the lean.
struct Twin {
    lean: Rig,
    plain: Rig,
}

impl Twin {
    fn new(spec: Spec, places: &[(f32, f32)]) -> Self {
        Self { lean: Rig::at(spec, places), plain: Rig::at(Spec { lean_px: 0.0, ..spec }, places) }
    }

    fn row(spec: Spec, tiles: u32) -> Self {
        Self::new(spec, &(0..tiles).map(|i| (100.0 * i as f32, 700.0)).collect::<Vec<_>>())
    }

    fn go(&mut self, from: Option<u32>, to: Option<u32>) {
        self.lean.go(from, to);
        self.plain.go(from, to);
    }

    fn advance(&mut self, by_ns: u64) {
        self.lean.advance(by_ns);
        self.plain.advance(by_ns);
    }

    fn tidy(&mut self) {
        let now = self.lean.now();
        self.lean.flow.tidy(now);
        self.plain.flow.tidy(now);
    }

    /// The time the screen shows what is asked for now.
    fn shown(&self) -> u64 {
        self.lean.shown()
    }

    /// How far the leaning tiles have pushed tile `id` off where the plain ones have it at `at`.
    fn lean_of(&self, id: u32, at: u64) -> (f32, f32) {
        let (a, b) = (centre(&self.lean, id, at), centre(&self.plain, id, at));
        (a.0 - b.0, a.1 - b.1)
    }

    /// The push of tile `id` along `axis` (0 across, 1 down) that is furthest from none in the `window_ms` after `start`, and the millisecond
    /// it was at.
    fn peak(&self, id: u32, axis: usize, start: u64, window_ms: u64) -> (f32, u64) {
        let at = |t: u64| {
            let p = self.lean_of(id, start + t * MS);
            (if axis == 0 { p.0 } else { p.1 }, t)
        };
        (0..=window_ms).map(at).fold((0.0, 0), |best, now| if now.0.abs() > best.0.abs() { now } else { best })
    }
}

#[test]
fn a_move_leans_the_incoming_along_the_way_it_went_and_springs_it_back_while_the_outgoing_stays_put() {
    let mut twin = Twin::row(Spec { lean_px: 3.0, ..Spec::default() }, 3);
    twin.go(None, Some(0));
    twin.advance(400 * MS);
    twin.go(Some(0), Some(1));
    let start = twin.shown();
    let (incoming, at) = twin.peak(1, 0, start, 400);
    let (outgoing, _) = twin.peak(0, 0, start, 400);
    assert!((2.9..=3.05).contains(&incoming), "the incoming leans 3 px the way the focus went: {incoming}");
    assert!((30..=55).contains(&at), "it peaks at the spring's own quarter period: {at} ms");
    assert!(outgoing.abs() < 0.01, "the outgoing does not lean: {outgoing}");
    // the way is the line between the two centres, which are 4 px off level while the left one is lifted
    assert!(twin.peak(1, 1, start, 400).0.abs() < 0.2, "the lean is along the row");
    for id in [0, 1] {
        for t in [400u64, 500] {
            let (x, y) = twin.lean_of(id, start + t * MS);
            assert!(x.abs() < 0.01 && y.abs() < 0.01, "tile {id} is back where it stood at {t} ms: ({x}, {y})");
        }
    }
    // the lift is what it was: the same scale and the same lift in every millisecond
    for t in 0..400u64 {
        let at = start + t * MS;
        for id in [0, 1] {
            let (with, without) = (&twin.lean, &twin.plain);
            let scale = |rig: &Rig| rig.flow.group(Id(id)).expect("a group").world_at(at).transform.scale;
            assert_eq!(scale(with), scale(without), "tile {id} at {t} ms");
            assert_eq!(with.lift(id, at), without.lift(id, at), "tile {id} at {t} ms");
        }
    }
}

#[test]
fn a_lean_is_held_to_a_fortieth_of_the_tile_it_leans() {
    // the tiles are 160 px, so 4 px, whatever the theme asks for
    let mut twin = Twin::row(Spec { lean_px: 6.0, ..Spec::default() }, 2);
    twin.go(None, Some(0));
    twin.advance(400 * MS);
    twin.go(Some(0), Some(1));
    let start = twin.shown();
    let incoming = twin.peak(1, 0, start, 400).0;
    assert!((3.9..=4.05).contains(&incoming), "{incoming}");
}

#[test]
fn reduced_motion_does_not_lean_and_adds_nothing_to_the_scene() {
    let mut twin = Twin::row(Spec { lean_px: 6.0, reduced: true, ..Spec::default() }, 3);
    twin.go(None, Some(0));
    twin.advance(300 * MS);
    twin.go(Some(0), Some(1));
    let start = twin.shown();
    for t in 0..300u64 {
        for id in 0..3 {
            assert_eq!(twin.lean_of(id, start + t * MS), (0.0, 0.0), "tile {id} at {t} ms");
        }
    }
    let (with, without) = (&twin.lean.scene, &twin.plain.scene);
    assert_eq!((with.groups_alive(), with.motions_alive()), (without.groups_alive(), without.motions_alive()));
}

#[test]
fn a_held_key_leans_without_a_jump_and_adds_no_more_than_the_pool() {
    let mut twin = Twin::row(Spec { lean_px: 3.0, ..Spec::default() }, 12);
    twin.go(None, Some(0));
    twin.advance(400 * MS);
    let counts = |rig: &Rig| (rig.scene.groups_alive(), rig.scene.motions_alive());
    let (groups, tracks) = counts(&twin.lean);
    assert_eq!(counts(&twin.plain), (groups, tracks));
    let mut current = 0u32;
    let (mut widest, mut furthest) = (0.0f32, 0.0f32);
    for next in 1..12u32 {
        twin.go(Some(current), Some(next));
        current = next;
        let start = twin.shown();
        for id in 0..12u32 {
            let mut before = twin.lean_of(id, start);
            for t in 1..40u64 {
                let now = twin.lean_of(id, start + t * MS);
                widest = widest.max((now.0 - before.0).abs()).max((now.1 - before.1).abs());
                furthest = furthest.max(now.0.abs()).max(now.1.abs());
                before = now;
            }
        }
        twin.advance(40 * MS);
        twin.tidy();
    }
    // each tile leans once, as the focus lands on it, and no further than its lean
    assert!((2.9..=3.05).contains(&furthest), "the furthest a tile leaned was {furthest} px");
    assert!(widest < 0.8, "the most a lean moved in a millisecond was {widest} px");
    let (more_groups, more_tracks) = counts(&twin.lean);
    assert!(more_groups <= groups + POOL && more_tracks <= tracks + POOL, "{groups} groups and {tracks} tracks grew to {more_groups} and {more_tracks}");
    assert_eq!(counts(&twin.plain), (groups, tracks), "a theme with no lean adds nothing");
}

#[test]
fn a_sideways_kick_on_a_lean_still_moving_keeps_its_axis_and_does_not_jump() {
    // right along the row to the second, down to the tile under it, and back up to the second 60 ms after it was left, its lean not yet back
    let mut twin = Twin::new(Spec { lean_px: 3.0, ..Spec::default() }, &[(0.0, 700.0), (100.0, 700.0), (100.0, 900.0)]);
    twin.go(None, Some(0));
    twin.advance(400 * MS);
    twin.go(Some(0), Some(1));
    twin.advance(30 * MS);
    twin.go(Some(1), Some(2));
    twin.advance(30 * MS);
    let before = twin.lean_of(1, twin.shown());
    assert!(before.0 > 2.0, "the second is still leaning: {before:?}");
    twin.go(Some(2), Some(1));
    let start = twin.shown();
    let after = twin.lean_of(1, start);
    assert!((after.0 - before.0).abs() < 0.01 && (after.1 - before.1).abs() < 0.01, "the lean is continuous through the turn: {before:?} -> {after:?}");
    let mut widest = 0.0f32;
    for id in [1, 2] {
        let mut last = twin.lean_of(id, start);
        for t in 1..=400u64 {
            let now = twin.lean_of(id, start + t * MS);
            widest = widest.max((now.0 - last.0).abs()).max((now.1 - last.1).abs());
            last = now;
        }
        assert!(last.0.abs() < 0.01 && last.1.abs() < 0.01, "tile {id} is back where it stood: {last:?}");
    }
    assert!(widest < 0.8, "the most a lean moved in a millisecond was {widest} px");
    assert!(twin.peak(1, 1, start, 400).0.abs() < 0.2, "the second stays on its axis, along the row");
}

#[test]
fn a_lean_that_has_come_to_rest_is_aimed_anew_without_a_jump() {
    let mut twin = Twin::new(Spec { lean_px: 3.0, ..Spec::default() }, &[(0.0, 700.0), (100.0, 700.0), (100.0, 900.0)]);
    twin.go(None, Some(0));
    twin.advance(400 * MS);
    twin.go(Some(0), Some(1));
    twin.advance(600 * MS);
    twin.go(Some(1), Some(2));
    twin.advance(600 * MS);
    // the second has stood a while with its lean aimed along the row; the focus comes back to it from below
    let before = twin.lean_of(1, twin.shown());
    twin.go(Some(2), Some(1));
    let start = twin.shown();
    let after = twin.lean_of(1, start);
    assert!(before.0.abs() < 0.01 && after.0.abs() < 0.01 && after.1.abs() < 0.01, "{before:?} -> {after:?}");
    let (up, _) = twin.peak(1, 1, start, 400);
    assert!((-3.05..=-2.9).contains(&up), "the second leans up, the way the focus went: {up}");
    assert!(twin.peak(1, 0, start, 400).0.abs() < 0.1, "and not along the way it came the first time");
}
