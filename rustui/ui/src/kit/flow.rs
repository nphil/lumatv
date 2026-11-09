//! The flowing focus (Nitin, 2026-10-29; a Lucent motion rule of the TV profile, `docs/research/rust-shell-kit.md`): a focus
//! move is one continuous motion, as on tvOS and LG's own Home, not "the old one shrinks, then the new one jumps".
//!
//! - The outgoing and the incoming focusables move **at the same time**, each on a critically damped spring from wherever it is
//!   and however fast it is going (`Settle::CriticalSpring` with the measured velocity): a held or a rapid D-pad retargets the
//!   springs mid-flight and the motion never stops and starts.
//! - The lift (the scale about the focusable's centre, its rise, and the lit glass that carries the rim and the deeper shadow) is
//!   one value, 0 at rest and 1 lifted, on a **shared pool of tracks** that hand over between focusables of any shape or size:
//!   a tile, a card, a row, a button, a dock icon are all lifted the same way, scaled by their own kind's numbers.
//! - A row that scrolls to show the focus scrolls on the **same spring**, so the scroll and the lift are one gesture.
//! - Reduced motion: no scale and no rise; the lit state crossfades in the reduced-motion ceiling.
//!
//! The tracks are the compositor's (`compositor_protocol::motion`): it evaluates the springs at its predicted present time, at
//! every refresh, so a frame costs this process nothing; the mirrors ([`Motion::value_at`], [`Motion::velocity_at`]) answer
//! what the screen shows, which is what the measurement in `tests/flow.rs` reads. The spring's natural period is Lucent's
//! `motion.spring.responseMs` (260): critically damped it lands within about 1.05 periods, 270 ms, at 120 Hz about 33 frames.

use super::focus::{Id, Region};
use compositor_protocol::{Ease, GroupPose, PoseBinding, PoseProperties, Rect};
use compositor_protocol::group::Transform;
use ui::error::UiError;
use ui::geometry::Frame;
use ui::group::Group;
use ui::motion::{Motion, MotionBounds, MotionToken, Settle, SpringToken, Units};
use ui::scene::Scene;

/// The spring every focus move settles on, and the reduced-motion crossfade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    /// The critically damped spring's natural period (Lucent `motion.spring.responseMs`).
    pub response_ms: f32,
    /// The reduced-motion crossfade's length (Lucent `motion.reduce`, at most the reduced ceiling).
    pub reduced_ms: u32,
    pub reduced: bool,
}

impl Default for Spec {
    fn default() -> Self {
        Self { response_ms: 260.0, reduced_ms: 120, reduced: false }
    }
}

impl Spec {
    /// The settle a move uses: the spring, or the crossfade under reduced motion.
    pub fn settle(&self) -> Settle {
        if self.reduced {
            Settle::Timed(MotionToken { duration_ms: self.reduced_ms, ease: Ease::Linear })
        } else {
            Settle::CriticalSpring(SpringToken { response_ms: self.response_ms })
        }
    }
}

/// How a focusable lifts: the group that holds its layers at its rest pose (the pivot at the focusable's centre, in the group's
/// own space), the scale at full lift, the rise in pixels, and the group whose opacity carries the lit glass (rim, sheen, the
/// deeper shadow), when it has one.
pub struct Lifted {
    pub group: Group,
    pub rest: GroupPose,
    pub scale: f32,
    pub rise: f32,
    pub lit: Option<Group>,
}

impl Lifted {
    fn lifted_pose(&self) -> GroupPose {
        GroupPose { scale: self.scale, y: self.rest.y - self.rise, ..self.rest }
    }

    /// Binds the group (and the lit group) to `track`: 0 is rest, 1 is lifted. Under reduced motion only the lit glass moves.
    fn bind(&self, track: &Motion, reduced: bool) -> Result<(), UiError> {
        if reduced {
            self.group.unbind(PoseProperties::ALL);
            self.group.set_pose(self.rest);
        } else {
            self.group.bind(
                track,
                PoseBinding {
                    values: [0.0, 1.0],
                    from: self.rest,
                    to: self.lifted_pose(),
                    properties: PoseProperties::SCALE.union(PoseProperties::TRANSLATION),
                },
            )?;
        }
        if let Some(lit) = &self.lit {
            lit.bind(
                track,
                PoseBinding {
                    values: [0.0, 1.0],
                    from: GroupPose { opacity: 0.0, ..GroupPose::IDENTITY },
                    to: GroupPose::IDENTITY,
                    properties: PoseProperties::OPACITY,
                },
            )?;
        }
        Ok(())
    }

    /// Lets go of the track: the group stays where it is drawn, which is rest for a track at 0.
    fn unbind(&self, at_rest: bool) {
        self.group.unbind(PoseProperties::ALL);
        if at_rest {
            self.group.set_pose(self.rest);
        }
        if let Some(lit) = &self.lit {
            lit.unbind(PoseProperties::ALL);
            if at_rest {
                lit.set_pose(GroupPose { opacity: 0.0, ..GroupPose::IDENTITY });
            }
        }
    }
}

struct Slot {
    track: Motion,
    owner: Option<Id>,
}

/// A row or a grid that scrolls with the focus: its track in pixels, bound to the translation of the group that holds it.
struct Scroller {
    region: Region,
    track: Motion,
}

/// A group inside a lifted group that does not scale or rise with it: what a card holds that is text, which is drawn on whole pixels and
/// is soft in a layer that is scaled. It is bound to the lift's track with the inverse pose, so the two cancel at rest and at full lift.
struct Keep {
    group: Group,
    rest: GroupPose,
}

pub struct Flow {
    pub spec: Spec,
    slots: Vec<Slot>,
    lifts: Vec<(Id, Lifted)>,
    scrollers: Vec<Scroller>,
    keeps: Vec<(Id, Keep)>,
}

/// How many focusables may be in flight at once before the oldest is snapped to rest: a held key at the remote's repeat rate (every
/// 40 ms) against a 270 ms settle keeps about seven moving.
pub const POOL: usize = 8;

impl Flow {
    /// A pool of `pool` lift tracks on `scene` (the compositor allows 64 motion tracks in all; a screen's scrollers take theirs
    /// from the rest).
    pub fn new(scene: &Scene, spec: Spec, pool: usize) -> Result<Self, UiError> {
        let mut slots = Vec::with_capacity(pool);
        for _ in 0..pool.max(2) {
            slots.push(Slot { track: scene.motion(0.0, MotionBounds::range(0.0, 1.0, Units::Progress))?, owner: None });
        }
        Ok(Self { spec, slots, lifts: Vec::new(), scrollers: Vec::new(), keeps: Vec::new() })
    }

    /// Gives `id` a lift; a focusable that has none is focused without motion (a label, a row that only highlights).
    pub fn lift(&mut self, id: Id, lifted: Lifted) {
        if let Some(i) = self.lifts.iter().position(|(l, _)| *l == id) {
            if let Some(slot) = self.slots.iter().find(|s| s.owner == Some(id)) {
                let _ = lifted.bind(&slot.track, self.spec.reduced);
            }
            self.lifts[i].1 = lifted;
            if let Some(slot) = self.slots.iter().find(|s| s.owner == Some(id)) {
                let _ = self.bind_keep(id, &slot.track);
            }
        } else {
            self.lifts.push((id, lifted));
        }
    }

    /// Gives `id`'s lift a group for its lit glass, or takes it away (`None`), and hands back the one it had: a lift that makes its lit
    /// group only while it is lifted holds no group for it at rest. Bound to the track at once when the lift is in flight.
    pub fn set_lit(&mut self, id: Id, lit: Option<Group>) -> Option<Group> {
        let slot = self.slots.iter().position(|s| s.owner == Some(id));
        let (_, l) = self.lifts.iter_mut().find(|(i, _)| *i == id)?;
        let old = std::mem::replace(&mut l.lit, lit);
        if let Some(old) = &old {
            old.unbind(PoseProperties::ALL);
        }
        if let Some(si) = slot {
            let _ = l.bind(&self.slots[si].track, self.spec.reduced);
        }
        old
    }

    /// Gives the lift of `id` a group whose contents it does not move: `group` hangs under the lifted group, posed at `rest` when the lift is
    /// at rest.
    pub fn keep(&mut self, id: Id, group: Group, rest: GroupPose) {
        self.keeps.retain(|(i, _)| *i != id);
        self.keeps.push((id, Keep { group, rest }));
        if let Some(slot) = self.slots.iter().find(|s| s.owner == Some(id)) {
            let _ = self.bind_keep(id, &slot.track);
        }
    }

    /// The group of `id`'s lift that holds what does not lift, when it has one.
    pub fn kept(&self, id: Id) -> Option<&Group> {
        self.keeps.iter().find(|(i, _)| *i == id).map(|(_, k)| &k.group)
    }

    fn bind_keep(&self, id: Id, track: &Motion) -> Result<(), UiError> {
        let (Some((_, k)), Some(l)) = (self.keeps.iter().find(|(i, _)| *i == id), self.lifted(id)) else { return Ok(()) };
        if self.spec.reduced {
            k.group.unbind(PoseProperties::ALL);
            k.group.set_pose(k.rest);
            return Ok(());
        }
        // the lifted group's world is (T + pivot + s (p - pivot)); the kept group's, with 1/s and a drop of rise/s, brings p back to where it was
        let inverse = 1.0 / l.scale;
        k.group.bind(
            track,
            PoseBinding {
                values: [0.0, 1.0],
                from: k.rest,
                to: GroupPose { scale: inverse, y: k.rest.y + l.rise * inverse, ..k.rest },
                properties: PoseProperties::SCALE.union(PoseProperties::TRANSLATION),
            },
        )
    }

    fn unbind_keep(&self, id: Id, at_rest: bool) {
        if let Some((_, k)) = self.keeps.iter().find(|(i, _)| *i == id) {
            k.group.unbind(PoseProperties::ALL);
            if at_rest {
                k.group.set_pose(k.rest);
            }
        }
    }

    /// Takes the lift of `id` out, with its group (a tile renamed when the dock's order changed).
    pub fn take_lift(&mut self, id: Id) -> Option<Lifted> {
        let i = self.lifts.iter().position(|(l, _)| *l == id)?;
        if let Some(slot) = self.slots.iter_mut().find(|s| s.owner == Some(id)) {
            slot.owner = None;
            self.lifts[i].1.unbind(false);
            self.unbind_keep(id, false);
        }
        self.keeps.retain(|(k, _)| *k != id);
        Some(self.lifts.remove(i).1)
    }

    /// Takes the lift of `id` away (the focusable left the screen). The track it held goes back to rest at once, so the next focusable
    /// that takes the slot does not start lifted.
    pub fn drop_lift(&mut self, id: Id) {
        if let Some(i) = self.lifts.iter().position(|(l, _)| *l == id) {
            if let Some(slot) = self.slots.iter_mut().find(|s| s.owner == Some(id)) {
                slot.owner = None;
                let _ = slot.track.settle(Some(0.0), Some(0.0), Settle::Timed(MotionToken { duration_ms: 0, ease: Ease::Linear }));
            }
            self.lifts.remove(i);
            self.keeps.retain(|(k, _)| *k != id);
        }
    }

    fn lifted(&self, id: Id) -> Option<&Lifted> {
        self.lifts.iter().find(|(l, _)| *l == id).map(|(_, l)| l)
    }

    /// The group that holds `id`'s layers, for what the lift does not drive (its opacity, whether it shows).
    pub fn group(&self, id: Id) -> Option<&Group> {
        self.lifted(id).map(|l| &l.group)
    }

    /// Poses what the lift does not drive: `id`'s opacity and whether it shows. The rest pose takes
    /// them too, because a lift that comes to rest is posed at it (`tidy`, a slot taken for another focusable): without them the group would
    /// pop back to full and shown, and stay so until the next pose.
    pub fn pose(&mut self, id: Id, opacity: f32, visible: bool) {
        let Some((_, l)) = self.lifts.iter_mut().find(|(i, _)| *i == id) else { return };
        l.rest = GroupPose { opacity, visible, ..l.rest };
        l.group.set_pose(l.rest);
    }

    /// Whether `id` has a lift.
    pub fn has(&self, id: Id) -> bool {
        self.lifted(id).is_some()
    }

    /// Moves `id`'s rest pose (a tile that took another slot, a row that moved): bound again when it is in flight, posed at once otherwise.
    pub fn place(&mut self, id: Id, rest: GroupPose) -> Result<(), UiError> {
        let reduced = self.spec.reduced;
        let slot = self.slots.iter().position(|s| s.owner == Some(id));
        if let Some(i) = self.lifts.iter().position(|(l, _)| *l == id) {
            self.lifts[i].1.rest = rest;
            match slot {
                Some(si) => self.lifts[i].1.bind(&self.slots[si].track, reduced)?,
                None => self.lifts[i].1.group.set_pose(rest),
            }
        }
        Ok(())
    }

    /// The slot that drives `id`, taking a free one for it when it has none: a slot nobody owns, else one whose owner is back at rest,
    /// else the one nearest rest, which is snapped there.
    fn acquire(&mut self, id: Id, at_ns: u64) -> Result<Option<usize>, UiError> {
        if self.lifted(id).is_none() {
            return Ok(None);
        }
        if let Some(i) = self.slots.iter().position(|s| s.owner == Some(id)) {
            return Ok(Some(i));
        }
        let i = match self.slots.iter().position(|s| s.owner.is_none()) {
            Some(i) => i,
            None => {
                let rested = self.slots.iter().position(|s| !s.track.is_active() && s.track.value_at(at_ns) <= 0.001);
                match rested {
                    Some(i) => i,
                    None => {
                        let (i, _) = self
                            .slots
                            .iter()
                            .enumerate()
                            .map(|(i, s)| (i, s.track.value_at(at_ns)))
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .expect("a pool of at least two");
                        self.slots[i].track.cancel(ui::motion::CancelPolicy::Hold)?;
                        i
                    }
                }
            }
        };
        if let Some(old) = self.slots[i].owner.take() {
            if let Some(l) = self.lifted(old) {
                l.unbind(true);
            }
            self.unbind_keep(old, true);
        }
        let reduced = self.spec.reduced;
        let track = &self.slots[i].track;
        self.lifted(id).expect("checked above").bind(track, reduced)?;
        self.bind_keep(id, track)?;
        self.slots[i].owner = Some(id);
        Ok(Some(i))
    }

    /// The focus went from `from` to `to`: the outgoing settles to rest and the incoming lifts, together, each from where it is and
    /// as fast as it is going. Either may be absent (the first focus, a focus cleared).
    pub fn go(&mut self, from: Option<Id>, to: Option<Id>, now_ns: u64) -> Result<(), UiError> {
        if from == to {
            return Ok(());
        }
        let settle = self.spec.settle();
        if let Some(from) = from
            && let Some(i) = self.slots.iter().position(|s| s.owner == Some(from))
        {
            self.slots[i].track.settle(Some(0.0), None, settle)?;
        }
        if let Some(to) = to
            && let Some(i) = self.acquire(to, now_ns)?
        {
            self.slots[i].track.settle(Some(1.0), None, settle)?;
        }
        Ok(())
    }

    /// Puts `id` lifted or at rest without motion (a screen that appears with its focus already set).
    pub fn snap(&mut self, id: Id, lifted: bool, now_ns: u64) -> Result<(), UiError> {
        let Some(i) = self.acquire(id, now_ns)? else { return Ok(()) };
        let target = if lifted { 1.0 } else { 0.0 };
        self.slots[i].track.settle(Some(target), Some(0.0), Settle::Timed(MotionToken { duration_ms: 0, ease: Ease::Linear }))?;
        Ok(())
    }

    /// Frees the slots of focusables that have come to rest, so the next move finds one without stealing. Call after a frame.
    pub fn tidy(&mut self, now_ns: u64) {
        for i in 0..self.slots.len() {
            let Some(owner) = self.slots[i].owner else { continue };
            let track = &self.slots[i].track;
            if !track.is_active() && track.value_at(now_ns) <= 0.001 {
                if let Some(l) = self.lifted(owner) {
                    l.unbind(true);
                }
                self.unbind_keep(owner, true);
                self.slots[i].owner = None;
            }
        }
    }

    /// A region whose content scrolls with the focus: `group` holds the content at `rest`, and moves by `-offset` along `horizontal`'s
    /// axis, up to `extent_px`.
    pub fn scroller(&mut self, scene: &Scene, region: Region, group: &Group, rest: GroupPose, horizontal: bool, extent_px: f32) -> Result<(), UiError> {
        let extent = extent_px.max(1.0);
        let track = scene.motion(0.0, MotionBounds::range(0.0, extent, Units::Pixels))?;
        let moved = if horizontal { GroupPose { x: rest.x - extent, ..rest } } else { GroupPose { y: rest.y - extent, ..rest } };
        group.bind(&track, PoseBinding { values: [0.0, extent], from: rest, to: moved, properties: PoseProperties::TRANSLATION })?;
        self.scrollers.retain(|s| s.region != region);
        self.scrollers.push(Scroller { region, track });
        Ok(())
    }

    /// Scrolls `region` to `offset` on the focus spring, as part of the same move.
    pub fn scroll_to(&mut self, region: Region, offset: f32) -> Result<(), UiError> {
        let settle = self.spec.settle();
        if let Some(s) = self.scrollers.iter().find(|s| s.region == region) {
            s.track.settle(Some(offset), None, settle)?;
        }
        Ok(())
    }

    /// Puts `region` at `offset` without motion (a screen that opens already scrolled).
    pub fn scroll_snap(&mut self, region: Region, offset: f32) -> Result<(), UiError> {
        if let Some(s) = self.scrollers.iter().find(|s| s.region == region) {
            s.track.settle(Some(offset), Some(0.0), Settle::Timed(MotionToken { duration_ms: 0, ease: Ease::Linear }))?;
        }
        Ok(())
    }

    /// Gives the scroller of `region` back (its track is freed): the content left the screen.
    pub fn drop_scroller(&mut self, region: Region) {
        self.scrollers.retain(|s| s.region != region);
    }

    pub fn scroll_at(&self, region: Region, at_ns: u64) -> Option<f32> {
        self.scrollers.iter().find(|s| s.region == region).map(|s| s.track.value_at(at_ns))
    }

    /// Whether any lift or scroll is still moving by the estimate at `at_ns`.
    pub fn settling_at(&self, at_ns: u64) -> bool {
        self.slots.iter().any(|s| s.track.is_settling_at(at_ns)) || self.scrollers.iter().any(|s| s.track.is_settling_at(at_ns))
    }

    /// The lift of `id` (0 rest to 1 lifted) by the estimate at `at_ns`; 0 for one that has no slot.
    pub fn lift_at(&self, id: Id, at_ns: u64) -> f32 {
        self.slots.iter().find(|s| s.owner == Some(id)).map_or(0.0, |s| s.track.value_at(at_ns))
    }

    /// Whether `id` is at rest at `at_ns`: no track drives it, or its track has settled back to 0. A focusable that is lifted, or still
    /// on its way down, is not.
    pub fn resting(&self, id: Id, at_ns: u64) -> bool {
        self.slots.iter().find(|s| s.owner == Some(id)).is_none_or(|s| !s.track.is_settling_at(at_ns) && s.track.value_at(at_ns) <= 0.001)
    }

    /// The lift's speed (per second) by the estimate at `at_ns`.
    pub fn velocity_at(&self, id: Id, at_ns: u64) -> f32 {
        self.slots.iter().find(|s| s.owner == Some(id)).map_or(0.0, |s| s.track.velocity_at(at_ns))
    }

    /// The focusables that are lifted by more than half at `at_ns`: the ones the focus is on (the focus of each surface is its own: the
    /// engine's registry, Home's model).
    pub fn lifted_ids(&self, at_ns: u64) -> Vec<Id> {
        self.slots.iter().filter_map(|s| s.owner.filter(|_| s.track.value_at(at_ns) > 0.5)).collect()
    }

    /// Where the plate of `id` is on the screen at full lift (its box at the lifted scale and rise, wherever the spring is now) and the
    /// clip every ancestor of it puts on what it draws; `None` for a focusable with no lift, or one that is not shown.
    pub fn lifted_plate(&self, id: Id, at_ns: u64) -> Option<(Frame, Option<Frame>)> {
        let l = self.lifted(id)?;
        let world = l.group.world_at(at_ns);
        if !world.visible || world.opacity <= 0.0 {
            return None;
        }
        // the parents' transform is the group's world with its own pose taken off; the lifted pose goes on in its place
        let now = l.group.pose_at(at_ns);
        let parents = world.transform.after(&Transform::of_pose(&now).inverse());
        let pose = if self.spec.reduced { l.rest } else { l.lifted_pose() };
        let r = parents.after(&Transform::of_pose(&pose)).apply_rect(&Rect { x: 0.0, y: 0.0, w: 2.0 * l.rest.pivot_x, h: 2.0 * l.rest.pivot_y });
        Some((Frame::new(r.x, r.y, r.w, r.h), world.clip.map(|c| Frame::new(c.x, c.y, c.w, c.h))))
    }

    /// How many focusables are in flight or lifted.
    pub fn in_flight(&self) -> usize {
        self.slots.iter().filter(|s| s.owner.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compositor_protocol::Event;
    use compositor_protocol::motion::NoticeKind;
    use std::cell::Cell;
    use std::rc::Rc;

    const MS: u64 = 1_000_000;
    const FRAME_NS: u64 = 8_333_333;
    const POSED: f32 = 0.86;

    /// A card the focus has left is posed at a lower opacity; its lift is handed back when its spring has come to rest (the
    /// compositor reports the settle, `tidy` frees the slot). Between those two the drawn card must not leave its pose: the hand-back
    /// poses it at rest, which was full opacity until the next pose.
    #[test]
    fn a_card_posed_at_a_lower_opacity_keeps_it_in_every_frame_through_the_hand_back_of_its_lift() {
        let h = ui::testing::harness();
        let clock = Rc::new(Cell::new(1_000_000_000u64));
        let c = Rc::clone(&clock);
        h.ui.scene.set_clock(move || c.get(), 8 * MS);
        let mut flow = Flow::new(&h.ui.scene, Spec::default(), 2).expect("a pool of tracks");
        for i in 0..2u32 {
            let rest = GroupPose { x: 100.0 * i as f32, y: 700.0, pivot_x: 80.0, pivot_y: 80.0, ..GroupPose::IDENTITY };
            let group = h.ui.scene.group(None, rest).expect("a group");
            let lit = h.ui.scene.group(Some(group.id()), GroupPose { opacity: 0.0, ..GroupPose::IDENTITY }).expect("a lit group");
            flow.lift(Id(i), Lifted { group, rest, scale: 1.03, rise: 4.0, lit: Some(lit) });
        }
        flow.go(None, Some(Id(0)), clock.get()).expect("the first focus");
        clock.set(clock.get() + 400 * MS);
        flow.go(Some(Id(0)), Some(Id(1)), clock.get()).expect("the focus moves on");
        flow.pose(Id(0), POSED, true);
        flow.pose(Id(1), 1.0, true);
        let slot = flow.slots.iter().position(|s| s.owner == Some(Id(0))).expect("the left card's slot");

        let start = clock.get() + 8 * MS;
        let mut handed_back = None;
        for f in 0..120u64 {
            // what the compositor does: at each refresh the shown time moves on, and once the spring has landed it reports the settle
            let now = clock.get();
            clock.set(now + FRAME_NS);
            let at = clock.get() + 8 * MS;
            if handed_back.is_none() && flow.slots[slot].track.value_at(at) <= 0.001 {
                let t = &flow.slots[slot].track;
                h.ui.scene.observe(&Event::Motion { id: t.wire_id(), generation: t.generation(), kind: NoticeKind::Settled, value: 0.0, velocity: 0.0, time_ns: at });
                handed_back = Some(f);
            }
            flow.tidy(clock.get());
            let world = flow.group(Id(0)).expect("the left card").world_at(at);
            assert_eq!((world.opacity, world.visible), (POSED, true), "frame {f} ({} ms): the left card is drawn at {} and shown {}", (at - start) / MS, world.opacity, world.visible);
        }
        let at = handed_back.expect("the spring lands within the second");
        assert!(at > 20, "the spring takes its time: landed at frame {at}");
        assert_eq!(flow.in_flight(), 1, "the left card's slot was handed back");
    }
}
