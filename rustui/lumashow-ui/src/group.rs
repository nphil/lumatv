//! Native retained groups, as the app holds them: owned handles with the compositor's pose math, so that what the app hit-tests
//! is the image the compositor draws.
//!
//! A [`Group`] carries a pose (a translation, a uniform scale about a pivot, an opacity, a visibility and an optional clip).
//! Layers and other groups hang under it with their own geometry in its local space, so one pose change moves everything below
//! with no request per child. Poses can be bound to [`Motion`] tracks (`bind`): several groups bound to one track move exactly
//! together, driven by the compositor at its predicted presentation time.
//!
//! The app keeps a mirror of each group (its pose, its parent and its bindings) and evaluates it with the pure functions of
//! `compositor_protocol::group` and the mirrors of the tracks (`motion.rs`): [`Group::world`] is the world the compositor
//! draws the group in, [`Group::hit`] the compositor's own half-open rectangle test through the same transform and clip, and
//! [`Group::set_parent`] with `preserve_world` re-anchors bindings by the rule of the compositor's `Groups::reparent`.
//!
//! **Ownership.** A group lives on the compositor until its handle is dropped *and* nothing hangs under it: the groups below
//! it, the layers attached to it, the prepared sets that capture it. That makes the order of deletion the one the compositor
//! expects without anyone having to think of it (children before parents, a layer's `Delete` before its group's), and means
//! the compositor never has to move orphans to a grandparent: no hit tree is left behind.

use crate::error::{Kind, Limit, UiError};
use crate::handle::handle;
use crate::motion::{Motion, MotionId};
use crate::scene::{Inner, Layer, Scene};
use compositor_protocol as wire;
pub use compositor_protocol::group::Transform;
use compositor_protocol::group::{
    MAX_GROUP_DEPTH, MAX_GROUPS, MAX_SCALE, MIN_SCALE, World, pose_preserving_world,
};
pub use compositor_protocol::{Edges, GroupPose, PoseBinding, PoseProperties};
use compositor_protocol::{Props, Rect, Request};
use std::rc::Rc;

handle!(
    /// Names a group in this process: an index and a generation, never reused. The compositor's id for it is
    /// [`Group::wire_id`].
    GroupId
);

/// The three properties a binding can drive, one binding slot each.
const PROPERTIES: [PoseProperties; 3] = [
    PoseProperties::TRANSLATION,
    PoseProperties::SCALE,
    PoseProperties::OPACITY,
];

/// One property bound to a track: `binding.properties` is exactly that property.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bound {
    pub motion: MotionId,
    pub binding: PoseBinding,
}

pub(crate) struct GroupEntry {
    pub(crate) wire: wire::GroupId,
    pub(crate) parent: Option<GroupId>,
    /// The pose as the app last set it. A bound property follows its track instead.
    pose: GroupPose,
    /// Hidden and unhittable, whatever `pose.visible` says.
    inert: bool,
    bound: [Option<Bound>; 3],
    /// What hangs under the group: groups, layers, prepared sets. The compositor's group goes when this is 0 and the handle is gone.
    pub(crate) members: u32,
    /// The `Group` handle is alive.
    handle: bool,
}

impl GroupEntry {
    /// The pose that goes over the wire: the app's, hidden when inert.
    fn wire_pose(&self) -> GroupPose {
        GroupPose {
            visible: self.pose.visible && !self.inert,
            ..self.pose
        }
    }

    /// The properties of this group bound to `motion` are freed, keeping the value `value` gives them (the compositor does the
    /// same when a track is deleted); the unbind request is staged.
    pub(crate) fn unbind_motion(&mut self, motion: MotionId, value: f32, batch: &mut Vec<Request>) {
        let mut freed = PoseProperties::NONE;
        for (slot, property) in self.bound.iter_mut().zip(PROPERTIES) {
            if let Some(bound) = slot.filter(|b| b.motion == motion) {
                bound.binding.apply(value, &mut self.pose);
                *slot = None;
                freed = freed.union(property);
            }
        }
        if !freed.is_empty() {
            batch.push(Request::GroupUnbind {
                id: self.wire,
                properties: freed,
            });
        }
    }
}

/// Makes a pose the compositor accepts of one the caller gave: a number that is not one keeps its old value, the scale and the
/// opacity are brought into range, a clip that cannot be one is dropped (and its feather with it), and a feather width that is
/// not a length is a hard edge.
fn sanitise(old: &GroupPose, new: GroupPose) -> GroupPose {
    let keep = |v: f32, old: f32| if v.is_finite() { v } else { old };
    let clip = new
        .clip
        .filter(|c| [c.x, c.y, c.w, c.h].iter().all(|v| v.is_finite()) && c.w >= 0.0 && c.h >= 0.0);
    let width = |w: f32| if w.is_finite() { w.max(0.0) } else { 0.0 };
    GroupPose {
        x: keep(new.x, old.x),
        y: keep(new.y, old.y),
        scale: keep(new.scale, old.scale).clamp(MIN_SCALE, MAX_SCALE),
        pivot_x: keep(new.pivot_x, old.pivot_x),
        pivot_y: keep(new.pivot_y, old.pivot_y),
        opacity: keep(new.opacity, old.opacity).clamp(0.0, 1.0),
        visible: new.visible,
        clip,
        feather: match clip {
            Some(_) => Edges::new(
                width(new.feather.left),
                width(new.feather.right),
                width(new.feather.top),
                width(new.feather.bottom),
            ),
            None => Edges::NONE,
        },
    }
}

impl Inner {
    fn group_entry(&self, id: GroupId) -> Result<&GroupEntry, UiError> {
        self.groups.get(id.key()).ok_or(UiError::Stale(Kind::Group))
    }

    /// How many groups there are from `id` up to the screen, itself included.
    pub(crate) fn group_depth(&self, id: GroupId) -> usize {
        let mut depth = 0;
        let mut at = Some(id);
        while let Some(g) = at {
            depth += 1;
            if depth > MAX_GROUP_DEPTH + 1 {
                break;
            }
            at = self.groups.get(g.key()).and_then(|e| e.parent);
        }
        depth
    }

    /// The deepest level among `id` and what hangs under it, counted from `id` (1 for a group with no groups below).
    fn group_height(&self, id: GroupId) -> usize {
        1 + self
            .groups
            .iter()
            .filter(|(_, _, e)| e.parent == Some(id))
            .map(|(i, g, _)| self.group_height(GroupId::new(i, g)))
            .max()
            .unwrap_or(0)
    }

    /// Whether `candidate` is `ancestor` or hangs below it.
    fn group_is_below(&self, candidate: GroupId, ancestor: GroupId) -> bool {
        let mut at = Some(candidate);
        let mut steps = 0;
        while let Some(g) = at {
            if g == ancestor {
                return true;
            }
            steps += 1;
            if steps > MAX_GROUP_DEPTH + 1 {
                return false;
            }
            at = self.groups.get(g.key()).and_then(|e| e.parent);
        }
        false
    }

    /// The group's own pose with every bound property at its track's estimated value at `at_ns`.
    fn group_pose_at(&self, entry: &GroupEntry, at_ns: u64) -> GroupPose {
        let mut pose = entry.pose;
        for bound in entry.bound.iter().flatten() {
            if let Some(v) = self.motion_value_at(bound.motion, at_ns) {
                bound.binding.apply(v, &mut pose);
            }
        }
        pose.visible = pose.visible && !entry.inert;
        pose
    }

    /// The world `id` is drawn in at `at_ns`: its ancestors' poses and its own, composed from the screen down.
    pub(crate) fn group_world(&self, id: GroupId, at_ns: u64) -> Option<World> {
        let mut chain = [id; MAX_GROUP_DEPTH + 1];
        let mut len = 0;
        let mut at = Some(id);
        while let Some(g) = at {
            if len == chain.len() {
                break;
            }
            let entry = self.groups.get(g.key())?;
            chain[len] = g;
            len += 1;
            at = entry.parent;
        }
        let mut world = World::ROOT;
        for g in chain[..len].iter().rev() {
            let entry = self.groups.get(g.key())?;
            world = world.child(&self.group_pose_at(entry, at_ns));
        }
        Some(world)
    }

    pub(crate) fn group_create(
        &mut self,
        parent: Option<GroupId>,
        pose: GroupPose,
    ) -> Result<GroupId, UiError> {
        if !pose.is_valid() {
            return Err(UiError::Invalid(
                "a group's pose must be finite, with a scale in range and an opacity in 0 to 1",
            ));
        }
        let parent_wire = match parent {
            Some(p) => {
                let wire = self.group_entry(p)?.wire;
                if self.group_depth(p) >= MAX_GROUP_DEPTH {
                    return Err(UiError::Limit(Limit::GroupDepth));
                }
                Some(wire)
            }
            None => None,
        };
        if self.groups.is_full() {
            return Err(UiError::Limit(Limit::Groups));
        }
        let wire = wire::GroupId(self.next_group);
        self.next_group += 1;
        let entry = GroupEntry {
            wire,
            parent,
            pose,
            inert: false,
            bound: [None; 3],
            members: 0,
            handle: true,
        };
        let key = self.groups.insert(entry).expect("room was checked above");
        self.batch.push(Request::GroupCreate {
            id: wire,
            parent: parent_wire,
            pose,
        });
        if let Some(p) = parent {
            self.groups.get_mut(p.key()).expect("checked above").members += 1;
        }
        Ok(GroupId::from_key(key))
    }

    pub(crate) fn group_set_pose(&mut self, id: GroupId, pose: GroupPose) {
        let Some(entry) = self.groups.get_mut(id.key()) else {
            return;
        };
        // The same pose again changes nothing on the compositor, and a commit of nothing else would still cost a frame.
        let pose = sanitise(&entry.pose, pose);
        if pose == entry.pose {
            return;
        }
        entry.pose = pose;
        let (wire, pose) = (entry.wire, entry.wire_pose());
        self.batch.push(Request::GroupSet { id: wire, pose });
    }

    pub(crate) fn group_set_clip_feathered(
        &mut self,
        id: GroupId,
        clip: Option<Rect>,
        feather: Edges,
    ) {
        let Some(entry) = self.groups.get(id.key()) else {
            return;
        };
        let pose = GroupPose {
            clip,
            feather,
            ..entry.pose
        };
        self.group_set_pose(id, pose);
    }

    pub(crate) fn group_set_inert(&mut self, id: GroupId, inert: bool) {
        let Some(entry) = self.groups.get_mut(id.key()) else {
            return;
        };
        if entry.inert == inert {
            return;
        }
        entry.inert = inert;
        let (wire, pose) = (entry.wire, entry.wire_pose());
        self.batch.push(Request::GroupSet { id: wire, pose });
    }

    pub(crate) fn group_bind(
        &mut self,
        id: GroupId,
        motion: MotionId,
        binding: PoseBinding,
    ) -> Result<(), UiError> {
        binding.check().map_err(UiError::Invalid)?;
        let motion_wire = self
            .motions
            .get(motion.key())
            .ok_or(UiError::Stale(Kind::Motion))?
            .wire;
        let entry = self
            .groups
            .get_mut(id.key())
            .ok_or(UiError::Stale(Kind::Group))?;
        for (slot, property) in entry.bound.iter_mut().zip(PROPERTIES) {
            if binding.properties.contains(property) {
                *slot = Some(Bound {
                    motion,
                    binding: PoseBinding {
                        properties: property,
                        ..binding
                    },
                });
            }
        }
        let wire = entry.wire;
        self.batch.push(Request::GroupBind {
            id: wire,
            motion: motion_wire,
            binding,
        });
        Ok(())
    }

    /// Frees the properties from their bindings, keeping the values they have now.
    pub(crate) fn group_unbind(&mut self, id: GroupId, properties: PoseProperties) {
        if properties.is_empty() || !properties.is_valid() {
            return;
        }
        let now = self.estimate_ns();
        let Some(entry) = self.groups.get(id.key()) else {
            return;
        };
        let mut pose = entry.pose;
        let mut freed = [false; 3];
        for (k, property) in PROPERTIES.iter().enumerate() {
            if let (true, Some(bound)) = (properties.contains(*property), entry.bound[k]) {
                if let Some(v) = self.motion_value_at(bound.motion, now) {
                    bound.binding.apply(v, &mut pose);
                }
                freed[k] = true;
            }
        }
        if !freed.iter().any(|f| *f) {
            return;
        }
        let entry = self.groups.get_mut(id.key()).expect("looked up above");
        entry.pose = pose;
        let mut mask = PoseProperties::NONE;
        for ((slot, freed), property) in entry.bound.iter_mut().zip(freed).zip(PROPERTIES) {
            if freed {
                *slot = None;
                mask = mask.union(property);
            }
        }
        let wire = entry.wire;
        self.batch.push(Request::GroupUnbind {
            id: wire,
            properties: mask,
        });
    }

    /// Moves `id` under `parent`. With `preserve_world` the group takes the pose that leaves its evaluated world as it is now,
    /// and its bindings are shifted (translation, added to both ends) or scaled (scale and opacity, by the ratio) by the same
    /// difference so that they keep following their tracks from there: the rule of the compositor's `Groups::reparent`, applied
    /// to the estimate of the tracks at the time the screen will show the move.
    pub(crate) fn group_set_parent(
        &mut self,
        id: GroupId,
        parent: Option<GroupId>,
        preserve_world: bool,
    ) -> Result<(), UiError> {
        let entry = self.group_entry(id)?;
        let (wire, old_parent) = (entry.wire, entry.parent);
        let parent_wire = match parent {
            Some(p) => {
                let parent_entry = self.group_entry(p)?;
                if self.group_is_below(p, id) {
                    return Err(UiError::Cycle);
                }
                if self.group_depth(p) + self.group_height(id) > MAX_GROUP_DEPTH {
                    return Err(UiError::Limit(Limit::GroupDepth));
                }
                Some(parent_entry.wire)
            }
            None => None,
        };
        if old_parent == parent {
            // Under the parent it already has, the world is the world: nothing changes.
            return Ok(());
        }
        if preserve_world {
            let now = self.estimate_ns();
            let world = self.group_world(id, now).expect("a live group");
            let new_parent = match parent {
                Some(p) => self.group_world(p, now).expect("a live group"),
                None => World::ROOT,
            };
            let entry = self.group_entry(id)?;
            let evaluated = self.group_pose_at(entry, now);
            let wanted = pose_preserving_world(&world, &new_parent, &evaluated);
            let ratio = |new: f32, old: f32| if old.abs() > 1.0e-6 { new / old } else { 1.0 };
            let (dx, dy) = (wanted.x - evaluated.x, wanted.y - evaluated.y);
            let scale = ratio(wanted.scale, evaluated.scale);
            let opacity = ratio(wanted.opacity, evaluated.opacity);
            let entry = self.groups.get_mut(id.key()).expect("looked up above");
            for (slot, property) in entry.bound.iter_mut().zip(PROPERTIES) {
                match slot {
                    Some(bound) => {
                        let b = &mut bound.binding;
                        if property == PoseProperties::TRANSLATION {
                            b.from.x += dx;
                            b.from.y += dy;
                            b.to.x += dx;
                            b.to.y += dy;
                        } else if property == PoseProperties::SCALE {
                            b.from.scale *= scale;
                            b.to.scale *= scale;
                        } else {
                            b.from.opacity *= opacity;
                            b.to.opacity *= opacity;
                        }
                    }
                    None => {
                        if property == PoseProperties::TRANSLATION {
                            entry.pose.x = wanted.x;
                            entry.pose.y = wanted.y;
                        } else if property == PoseProperties::SCALE {
                            entry.pose.scale = wanted.scale;
                        } else {
                            entry.pose.opacity = wanted.opacity;
                        }
                    }
                }
            }
        }
        self.groups
            .get_mut(id.key())
            .expect("looked up above")
            .parent = parent;
        self.batch.push(Request::GroupParent {
            id: wire,
            parent: parent_wire,
            preserve_world,
        });
        if let Some(p) = parent {
            self.groups.get_mut(p.key()).expect("checked above").members += 1;
        }
        if let Some(old) = old_parent {
            self.release_member(old);
        }
        Ok(())
    }

    /// Puts `layer` under `group` with `local` props in the group's space; the layer's membership moves from the group it hung
    /// under before.
    pub(crate) fn group_attach(
        &mut self,
        id: GroupId,
        layer: &Layer,
        local: Props,
    ) -> Result<(), UiError> {
        let wire = self.group_entry(id)?.wire;
        self.push(Request::Attach {
            id: layer.id(),
            group: Some(wire),
            local,
        });
        layer.props.set(local);
        let previous = layer.group.replace(Some(id));
        if previous != Some(id) {
            self.groups
                .get_mut(id.key())
                .expect("checked above")
                .members += 1;
            if let Some(old) = previous {
                self.release_member(old);
            }
        }
        Ok(())
    }

    /// Something that hung under the group is gone: the group goes too if its handle already has.
    pub(crate) fn release_member(&mut self, id: GroupId) {
        if let Some(entry) = self.groups.get_mut(id.key()) {
            entry.members = entry.members.saturating_sub(1);
        }
        self.free_group(id);
    }

    /// The `Group` handle is dropped.
    fn group_handle_dropped(&mut self, id: GroupId) {
        if let Some(entry) = self.groups.get_mut(id.key()) {
            entry.handle = false;
        }
        self.free_group(id);
    }

    /// Deletes the group on the compositor if nothing holds it any more, then the parent that waited for it.
    fn free_group(&mut self, id: GroupId) {
        let mut at = Some(id);
        while let Some(g) = at {
            let Some(entry) = self.groups.get(g.key()) else {
                break;
            };
            if entry.handle || entry.members > 0 {
                break;
            }
            let entry = self.groups.remove(g.key()).expect("looked up above");
            self.batch.push(Request::GroupDelete { id: entry.wire });
            at = entry.parent;
            if let Some(parent) = at {
                if let Some(p) = self.groups.get_mut(parent.key()) {
                    p.members = p.members.saturating_sub(1);
                }
            }
        }
    }

    /// A prepared set captures the group.
    pub(crate) fn group_pin(&mut self, id: GroupId) -> Result<wire::GroupId, UiError> {
        let entry = self
            .groups
            .get_mut(id.key())
            .ok_or(UiError::Stale(Kind::Group))?;
        entry.members += 1;
        Ok(entry.wire)
    }
}

/// A group: a pose that moves, scales and fades everything under it. Dropping the handle lets the group go as soon as nothing
/// hangs under it (see the module documentation).
pub struct Group {
    id: GroupId,
    scene: Scene,
}

impl Scene {
    /// A group under `parent` (the screen when `None`) with `pose`. At most 96 groups at once (the ones that wait for what
    /// hangs under them count), nested at most 8 deep; a pose the protocol would refuse is an `Err`. Made while a view is being
    /// built by [`Scene::adopting_view`], a group with no parent hangs under the view's group.
    pub fn group(&self, parent: Option<GroupId>, pose: GroupPose) -> Result<Group, UiError> {
        let nest = self.inner.borrow().nest;
        let id = self
            .inner
            .borrow_mut()
            .group_create(parent.or(nest), pose)?;
        Ok(Group {
            id,
            scene: self.clone(),
        })
    }
}

impl Group {
    pub fn id(&self) -> GroupId {
        self.id
    }

    /// The compositor's name for the group.
    pub fn wire_id(&self) -> wire::GroupId {
        self.scene
            .inner
            .borrow()
            .group_entry(self.id)
            .expect("a live handle")
            .wire
    }

    pub fn parent(&self) -> Option<GroupId> {
        self.scene
            .inner
            .borrow()
            .group_entry(self.id)
            .expect("a live handle")
            .parent
    }

    /// Puts `layer` under the group with `local` props, in the group's space: from now on the layer's own `set` and `animate`
    /// act on those. A layer hangs under one group at a time; this moves it from the one it was under. Its group stays on the
    /// compositor as long as the layer does.
    pub fn attach(&self, layer: &Layer, local: Props) -> Result<(), UiError> {
        if !Rc::ptr_eq(&layer.scene.inner, &self.scene.inner) {
            return Err(UiError::Stale(Kind::Layer));
        }
        self.scene
            .inner
            .borrow_mut()
            .group_attach(self.id, layer, local)
    }

    /// Binds the properties `mapping` names to `motion`: from the next frame the group follows the track. A group has at most
    /// one binding per property; a new one replaces the old for the properties it names. The mapping must name finite, distinct
    /// track values and poses the protocol accepts (`PoseBinding::check`).
    pub fn bind(&self, motion: &Motion, mapping: PoseBinding) -> Result<(), UiError> {
        if !Rc::ptr_eq(&motion.scene().inner, &self.scene.inner) {
            return Err(UiError::Stale(Kind::Motion));
        }
        self.scene
            .inner
            .borrow_mut()
            .group_bind(self.id, motion.id(), mapping)
    }

    /// Frees the properties from their bindings; they keep the values they have.
    pub fn unbind(&self, properties: PoseProperties) {
        self.scene
            .inner
            .borrow_mut()
            .group_unbind(self.id, properties);
    }

    /// Moves the group under `parent` (the screen when `None`). With `preserve_world` it takes the pose that leaves its world as
    /// it is now (bindings re-anchored by the same difference); without, it keeps its pose and moves with its new parent. A move
    /// under itself or what hangs under it is an `Err(UiError::Cycle)`, one that makes the tree too deep a
    /// `Limit(GroupDepth)`.
    ///
    /// While a track the group follows is moving, the re-anchoring uses the app's estimate of the track at the moment, which is
    /// within the estimate's error of what the compositor uses (a scale that differs between the old and new parent makes the
    /// shift depend on the value).
    pub fn set_parent(&self, parent: Option<GroupId>, preserve_world: bool) -> Result<(), UiError> {
        self.scene
            .inner
            .borrow_mut()
            .group_set_parent(self.id, parent, preserve_world)
    }

    /// Sets the group's own pose at once. A property a track is bound to keeps following its track. A number that is not one
    /// keeps its old value, the scale and the opacity are brought into range (the compositor would refuse the pose otherwise).
    pub fn set_pose(&self, pose: GroupPose) {
        self.scene.inner.borrow_mut().group_set_pose(self.id, pose);
    }

    /// Replaces only the clip and the feather of the group's own pose (everything else, bindings included, stays) and stages one
    /// `GroupSet`. A feather width that is not a length is 0; a feather without a clip is dropped.
    pub fn set_clip_feathered(&self, clip: Option<Rect>, feather: Edges) {
        self.scene
            .inner
            .borrow_mut()
            .group_set_clip_feathered(self.id, clip, feather);
    }

    /// An inert group is hidden and cannot be hit, whatever its pose says (a hidden view does no work and takes no touch).
    pub fn set_inert(&self, inert: bool) {
        self.scene
            .inner
            .borrow_mut()
            .group_set_inert(self.id, inert);
    }

    pub fn is_inert(&self) -> bool {
        self.scene
            .inner
            .borrow()
            .group_entry(self.id)
            .expect("a live handle")
            .inert
    }

    /// The group's own pose as it is drawn at `at_ns`: the pose the app set, with every bound property at its track's
    /// estimated value, and hidden if inert.
    pub fn pose_at(&self, at_ns: u64) -> GroupPose {
        let inner = self.scene.inner.borrow();
        let entry = inner.group_entry(self.id).expect("a live handle");
        inner.group_pose_at(entry, at_ns)
    }

    /// The world the group is drawn in at `at_ns` (`CLOCK_MONOTONIC`): the transform to the screen, the opacity, whether it is
    /// shown and the screen-space clip, of every ancestor and the group together.
    pub fn world_at(&self, at_ns: u64) -> World {
        self.scene
            .inner
            .borrow()
            .group_world(self.id, at_ns)
            .expect("a live handle")
    }

    /// The world the screen will show a request made now.
    pub fn world(&self) -> World {
        let at = self.scene.inner.borrow().estimate_ns();
        self.world_at(at)
    }

    /// Whether the screen point (`x`, `y`) lies in a layer of `size` placed by `local` (its props in this group's space) as it
    /// is drawn at `at_ns`: the same half-open rectangle, after the same transform and clip, that the compositor draws. A
    /// hidden, inert or transparent layer is not hit.
    pub fn hit_at(&self, local: &Props, size: (f32, f32), x: f32, y: f32, at_ns: u64) -> bool {
        self.world_at(at_ns).hit(local, size, x, y)
    }

    /// [`Group::hit_at`] for the time the screen will show a request made now.
    pub fn hit(&self, local: &Props, size: (f32, f32), x: f32, y: f32) -> bool {
        let at = self.scene.inner.borrow().estimate_ns();
        self.hit_at(local, size, x, y, at)
    }

    /// The point of the group's own space that lies under the screen point (`x`, `y`) at `at_ns`: where a finger is on a
    /// scrolling list, in the list's coordinates.
    pub fn to_local_at(&self, x: f32, y: f32, at_ns: u64) -> (f32, f32) {
        self.world_at(at_ns).transform.inverse().apply(x, y)
    }

    /// [`Group::to_local_at`] for the time the screen will show a request made now.
    pub fn to_local(&self, x: f32, y: f32) -> (f32, f32) {
        let at = self.scene.inner.borrow().estimate_ns();
        self.to_local_at(x, y, at)
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.scene.inner.borrow_mut().group_handle_dropped(self.id);
    }
}

/// How many groups the compositor holds for this scene, the ones that wait for what hangs under them included.
impl Scene {
    pub fn groups_alive(&self) -> usize {
        self.inner.borrow().groups.live()
    }
}

const _: () = assert!(MAX_GROUPS == crate::limits::MAX_GROUP_NODES);
const _: () = assert!(MAX_GROUP_DEPTH == crate::limits::MAX_GROUP_DEPTH);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{DrivePhase, DriveSample, Settle, sample};
    use crate::scene::SceneOptions;
    use crate::scene::testing::{connect, drain};
    use compositor_protocol::motion::Track;
    use compositor_protocol::{
        CancelPolicy, Event, Fill, MotionBounds, Rect, Rgba, ScrollToken, Shape, SpringToken, Units,
    };

    const MS: u64 = 1_000_000;

    fn shape() -> Shape {
        Shape {
            corner_radius: 0.0,
            fill: Fill::Solid(Rgba(1, 1, 1, 255)),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        }
    }

    fn pose(x: f32, y: f32) -> GroupPose {
        GroupPose::at(x, y)
    }

    /// A scene whose clock is the test's and whose lead is zero: the estimate is the track at that time.
    fn at_time(scene: &Scene) -> std::rc::Rc<std::cell::Cell<u64>> {
        let now = std::rc::Rc::new(std::cell::Cell::new(0u64));
        let read = std::rc::Rc::clone(&now);
        let mut inner = scene.inner.borrow_mut();
        inner.clock = std::rc::Rc::new(move || read.get());
        inner.lead = crate::motion::Lead::fixed(0);
        now
    }

    fn same(a: &World, b: &World) {
        let t = |w: &World| [w.transform.scale, w.transform.tx, w.transform.ty, w.opacity];
        for (x, y) in t(a).into_iter().zip(t(b)) {
            assert!((x - y).abs() < 1e-3, "{a:?} vs {b:?}");
        }
        assert_eq!(a.visible, b.visible);
    }

    #[test]
    fn a_group_is_deleted_after_what_hangs_under_it_whatever_order_the_handles_are_dropped_in() {
        for order in [[0usize, 1, 2], [2, 1, 0], [1, 0, 2], [1, 2, 0]] {
            let (scene, mut comp, _events) = connect(SceneOptions::default());
            let root = scene.group(None, pose(0.0, 0.0)).unwrap();
            let child = scene.group(Some(root.id()), pose(5.0, 5.0)).unwrap();
            let leaf = scene.shape(10, 10, 0, shape(), Props::at(0.0, 0.0));
            child.attach(&leaf, Props::at(1.0, 1.0)).unwrap();
            let (g_root, g_child, l) = (root.wire_id(), child.wire_id(), leaf.id());
            scene.try_commit().unwrap();
            drain(&mut comp);
            let mut handles: [Option<Box<dyn std::any::Any>>; 3] = [
                Some(Box::new(root)),
                Some(Box::new(child)),
                Some(Box::new(leaf)),
            ];
            for k in order {
                handles[k] = None;
            }
            scene.try_commit().unwrap();
            let sent = drain(&mut comp);
            let deletes: Vec<&Request> = sent
                .iter()
                .filter(|r| matches!(r, Request::Delete { .. } | Request::GroupDelete { .. }))
                .collect();
            assert_eq!(
                deletes,
                vec![
                    &Request::Delete { id: l },
                    &Request::GroupDelete { id: g_child },
                    &Request::GroupDelete { id: g_root },
                ],
                "dropped in order {order:?}"
            );
            assert_eq!(scene.groups_alive(), 0);
        }
    }

    #[test]
    fn a_dropped_group_with_a_layer_still_under_it_stays_and_counts_against_the_ceiling() {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let g = scene.group(None, pose(0.0, 0.0)).unwrap();
        let layer = scene.shape(4, 4, 0, shape(), Props::at(0.0, 0.0));
        g.attach(&layer, Props::at(0.0, 0.0)).unwrap();
        let wire = g.wire_id();
        drop(g);
        scene.try_commit().unwrap();
        assert!(
            !drain(&mut comp)
                .iter()
                .any(|r| matches!(r, Request::GroupDelete { .. }))
        );
        assert_eq!(scene.groups_alive(), 1);
        // Detaching the layer lets it go.
        layer.detach(Props::at(7.0, 7.0));
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        assert!(
            sent.contains(&Request::GroupDelete { id: wire }),
            "{sent:?}"
        );
        assert!(sent.contains(&Request::Attach {
            id: layer.id(),
            group: None,
            local: Props::at(7.0, 7.0)
        }));
        let attach_at = sent
            .iter()
            .position(|r| matches!(r, Request::Attach { .. }))
            .unwrap();
        let delete_at = sent
            .iter()
            .position(|r| matches!(r, Request::GroupDelete { .. }))
            .unwrap();
        assert!(
            attach_at < delete_at,
            "the layer leaves before the group does"
        );
    }

    #[test]
    fn at_most_sixty_four_groups_eight_deep_and_a_refusal_stages_nothing() {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let mut groups = Vec::new();
        for _ in 0..MAX_GROUPS {
            groups.push(scene.group(None, pose(0.0, 0.0)).unwrap());
        }
        scene.try_commit().unwrap();
        drain(&mut comp);
        assert!(matches!(
            scene.group(None, pose(0.0, 0.0)),
            Err(UiError::Limit(Limit::Groups))
        ));
        assert!(!scene.has_pending(), "nothing was staged for the refusal");
        groups.clear();
        // Eight levels fit; a ninth does not.
        let mut chain: Vec<Group> = Vec::new();
        for level in 0..MAX_GROUP_DEPTH {
            let parent = chain.last().map(Group::id);
            chain.push(
                scene
                    .group(parent, pose(0.0, 0.0))
                    .unwrap_or_else(|e| panic!("level {level}: {e}")),
            );
        }
        let deepest = chain.last().unwrap().id();
        assert!(matches!(
            scene.group(Some(deepest), pose(0.0, 0.0)),
            Err(UiError::Limit(Limit::GroupDepth))
        ));
        // A pose the protocol would refuse.
        for bad in [
            GroupPose {
                scale: 0.0,
                ..pose(0.0, 0.0)
            },
            GroupPose {
                opacity: 1.5,
                ..pose(0.0, 0.0)
            },
            GroupPose {
                x: f32::NAN,
                ..pose(0.0, 0.0)
            },
        ] {
            assert!(matches!(scene.group(None, bad), Err(UiError::Invalid(_))));
        }
    }

    #[test]
    fn a_stale_parent_and_a_group_of_another_scene_are_refused() {
        let (scene, _c, _e) = connect(SceneOptions::default());
        let (other, _c2, _e2) = connect(SceneOptions::default());
        let a = scene.group(None, pose(0.0, 0.0)).unwrap();
        let old = a.id();
        drop(a);
        assert!(matches!(
            scene.group(Some(old), pose(0.0, 0.0)),
            Err(UiError::Stale(Kind::Group))
        ));
        let g = scene.group(None, pose(0.0, 0.0)).unwrap();
        let foreign = other.shape(1, 1, 0, shape(), Props::at(0.0, 0.0));
        assert!(matches!(
            g.attach(&foreign, Props::at(0.0, 0.0)),
            Err(UiError::Stale(Kind::Layer))
        ));
    }

    #[test]
    fn a_move_under_itself_or_what_hangs_under_it_is_a_cycle_and_one_that_makes_the_tree_too_deep_is_refused()
     {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let a = scene.group(None, pose(0.0, 0.0)).unwrap();
        let b = scene.group(Some(a.id()), pose(0.0, 0.0)).unwrap();
        let c = scene.group(Some(b.id()), pose(0.0, 0.0)).unwrap();
        scene.try_commit().unwrap();
        drain(&mut comp);
        assert!(matches!(
            a.set_parent(Some(a.id()), false),
            Err(UiError::Cycle)
        ));
        assert!(matches!(
            a.set_parent(Some(c.id()), true),
            Err(UiError::Cycle)
        ));
        assert!(!scene.has_pending());
        // A chain of six cannot be put under a group so deep that the whole is a level past the ceiling, and can under the next
        // one up, which makes the whole exactly as deep as groups may go.
        let mut deep = vec![scene.group(None, pose(0.0, 0.0)).unwrap()];
        for _ in 1..MAX_GROUP_DEPTH - 5 {
            let p = deep.last().unwrap().id();
            deep.push(scene.group(Some(p), pose(0.0, 0.0)).unwrap());
        }
        let tall_root = scene.group(None, pose(0.0, 0.0)).unwrap();
        let mut tall = vec![];
        for _ in 0..5 {
            let p = tall.last().map_or(tall_root.id(), |g: &Group| g.id());
            tall.push(scene.group(Some(p), pose(0.0, 0.0)).unwrap());
        }
        // tall_root has height 6: under a group at depth `MAX_GROUP_DEPTH - 5` that is one level past the ceiling.
        assert!(matches!(
            tall_root.set_parent(Some(deep[MAX_GROUP_DEPTH - 6].id()), false),
            Err(UiError::Limit(Limit::GroupDepth))
        ));
        // Under one level up it is exactly the ceiling: fine.
        tall_root
            .set_parent(Some(deep[MAX_GROUP_DEPTH - 7].id()), false)
            .unwrap();
    }

    #[test]
    fn set_parent_with_preserve_world_leaves_the_world_and_without_it_the_group_moves_with_its_new_parent()
     {
        let (scene, _comp, _events) = connect(SceneOptions::default());
        let now = at_time(&scene);
        now.set(0);
        let old_parent = scene
            .group(
                None,
                GroupPose {
                    scale: 2.0,
                    pivot_x: 10.0,
                    pivot_y: 20.0,
                    opacity: 0.5,
                    ..pose(100.0, 50.0)
                },
            )
            .unwrap();
        let new_parent = scene
            .group(
                None,
                GroupPose {
                    scale: 0.5,
                    opacity: 0.8,
                    ..pose(-30.0, 40.0)
                },
            )
            .unwrap();
        let child = scene
            .group(
                Some(old_parent.id()),
                GroupPose {
                    opacity: 0.9,
                    ..pose(10.0, 20.0)
                },
            )
            .unwrap();
        let before = child.world_at(0);
        child.set_parent(Some(new_parent.id()), true).unwrap();
        same(&child.world_at(0), &before);
        assert_eq!(child.parent(), Some(new_parent.id()));
        // Without: its pose stays as the child of the new parent, so its world is the new parent's with that pose.
        let other = scene
            .group(Some(old_parent.id()), pose(10.0, 20.0))
            .unwrap();
        other.set_parent(Some(new_parent.id()), false).unwrap();
        let expect = new_parent.world_at(0).child(&pose(10.0, 20.0));
        same(&other.world_at(0), &expect);
        // And to the screen, preserving too.
        child.set_parent(None, true).unwrap();
        same(&child.world_at(0), &before);
    }

    #[test]
    fn a_preserved_reparent_re_anchors_a_bound_group_by_the_same_difference_and_it_keeps_following_its_track()
     {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let now = at_time(&scene);
        now.set(0);
        let strip = scene
            .motion(300.0, MotionBounds::range(0.0, 1000.0, Units::Pixels))
            .unwrap();
        let parent = scene.group(None, pose(40.0, 10.0)).unwrap();
        let list = scene.group(None, pose(0.0, 0.0)).unwrap();
        // The list scrolls: pixels [0, 1] map to translations [0, -1] (x).
        list.bind(
            &strip,
            PoseBinding {
                values: [0.0, 1.0],
                from: GroupPose::at(0.0, 0.0),
                to: GroupPose::at(-1.0, 0.0),
                properties: PoseProperties::TRANSLATION,
            },
        )
        .unwrap();
        scene.try_commit().unwrap();
        drain(&mut comp);
        let before = list.world_at(0);
        assert!((before.transform.tx + 300.0).abs() < 1e-3);
        list.set_parent(Some(parent.id()), true).unwrap();
        same(&list.world_at(0), &before);
        // The track moves 100 more: the group follows by exactly that, wherever it hangs now.
        let mut track = Track::new(300.0, MotionBounds::range(0.0, 1000.0, Units::Pixels));
        track.settle(
            0,
            Some(400.0),
            Some(0.0),
            Settle::Timed(crate::motion::MotionToken {
                duration_ms: 100,
                ease: compositor_protocol::Ease::Linear,
            }),
        );
        strip
            .settle(
                Some(400.0),
                Some(0.0),
                Settle::Timed(crate::motion::MotionToken {
                    duration_ms: 100,
                    ease: compositor_protocol::Ease::Linear,
                }),
            )
            .unwrap();
        let later = list.world_at(100 * MS);
        assert!(
            (later.transform.tx - (before.transform.tx - 100.0)).abs() < 1e-3,
            "{later:?}"
        );
        // The request carries the flag, and the mirror and the compositor use one rule.
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        assert!(sent.iter().any(|r| matches!(
            r,
            Request::GroupParent {
                preserve_world: true,
                ..
            }
        )));
    }

    #[test]
    fn hit_testing_is_the_shared_world_math_through_the_bound_tracks_estimate() {
        let (scene, _comp, _events) = connect(SceneOptions::default());
        let now = at_time(&scene);
        let bounds = MotionBounds {
            ratio: -1.0,
            ..MotionBounds::range(0.0, 2000.0, Units::Pixels)
        };
        let scroll = scene.motion(0.0, bounds).unwrap();
        let viewport = scene
            .group(
                None,
                GroupPose {
                    clip: Some(Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 400.0,
                        h: 200.0,
                    }),
                    ..pose(50.0, 80.0)
                },
            )
            .unwrap();
        let strip = scene.group(Some(viewport.id()), pose(0.0, 0.0)).unwrap();
        strip
            .bind(
                &scroll,
                PoseBinding {
                    values: [0.0, 1.0],
                    from: GroupPose::at(0.0, 0.0),
                    to: GroupPose::at(-1.0, 0.0),
                    properties: PoseProperties::TRANSLATION,
                },
            )
            .unwrap();
        // A card 120 wide at x = 600 in the strip: off the viewport at rest.
        let card = Props::at(600.0, 20.0);
        let size = (120.0, 100.0);
        assert!(!strip.hit_at(&card, size, 50.0 + 650.0, 100.0, 0));
        // A finger drags it left by 500 px: the card is now at 50 + 100 = 150.. in the screen.
        let mut own = Track::new(0.0, bounds);
        for (phase, position, t) in [
            (DrivePhase::Down, 700.0, 0u64),
            (DrivePhase::Move, 500.0, 16),
            (DrivePhase::Move, 300.0, 32),
            (DrivePhase::Up, 200.0, 48),
        ] {
            now.set(t * MS);
            let s = DriveSample {
                contact: 1,
                phase,
                position,
                time_ns: t * MS,
            };
            scroll.drive(s).unwrap();
            own.drive(s, t * MS);
        }
        assert_eq!(scroll.value_at(48 * MS), 500.0);
        let at = 48 * MS;
        let manual = World::ROOT
            .child(&viewport.pose_at(at))
            .child(&GroupPose::at(-own.value(), 0.0));
        for (x, y) in [
            (120.0, 100.0),
            (149.0, 100.0),
            (150.0, 100.0),
            (269.9, 100.0),
            (270.0, 100.0),
            (200.0, 101.0),
            (200.0, 79.0),
        ] {
            assert_eq!(
                strip.hit_at(&card, size, x, y, at),
                manual.hit(&card, size, x, y),
                "({x}, {y})"
            );
        }
        // The half-open edges: the card's left edge is in, its right edge is out.
        assert!(strip.hit_at(&card, size, 150.0, 100.0, at));
        assert!(!strip.hit_at(&card, size, 270.0, 100.0, at));
        // Outside the viewport's clip the card is not drawn, so it is not hit, even where its rectangle is.
        assert!(!strip.hit_at(&Props::at(600.0, 20.0), (120.0, 100.0), 460.0, 100.0, 0));
        // A point on the screen maps back into the strip's own coordinates.
        let (lx, ly) = strip.to_local_at(200.0, 100.0, at);
        assert!(
            (lx - (200.0 - 50.0 + 500.0)).abs() < 1e-3 && (ly - 20.0).abs() < 1e-3,
            "{lx} {ly}"
        );
        // An inert group is not hit, and is hidden on the wire.
        strip.set_inert(true);
        assert!(!strip.hit_at(&card, size, 200.0, 100.0, at));
        assert!(strip.is_inert());
    }

    #[test]
    fn a_bound_group_follows_a_scroll_settle_to_its_anchor_by_the_same_equations_as_the_track() {
        let (scene, _comp, _events) = connect(SceneOptions::default());
        let now = at_time(&scene);
        let bounds = MotionBounds {
            ratio: -1.0,
            overscroll: 60.0,
            anchors: [0.0, 480.0, 960.0, 1440.0, 0.0, 0.0, 0.0, 0.0],
            anchor_count: 4,
            ..MotionBounds::range(0.0, 1440.0, Units::Pixels)
        };
        let scroll = scene.motion(0.0, bounds).unwrap();
        let strip = scene.group(None, pose(0.0, 0.0)).unwrap();
        strip
            .bind(
                &scroll,
                PoseBinding {
                    values: [0.0, 1.0],
                    from: GroupPose::at(0.0, 0.0),
                    to: GroupPose::at(-1.0, 0.0),
                    properties: PoseProperties::TRANSLATION,
                },
            )
            .unwrap();
        now.set(0);
        scroll.drive(sample::down(1, 800.0, 0)).unwrap();
        for k in 1..=10u64 {
            now.set(k * 16 * MS);
            scroll
                .drive(sample::moved(1, 800.0 - 12.0 * k as f32, k * 16 * MS))
                .unwrap();
        }
        let released = 11 * 16 * MS;
        now.set(released);
        scroll
            .drive(sample::up(1, 800.0 - 132.0, released))
            .unwrap();
        scroll
            .settle(
                None,
                None,
                Settle::Scroll(ScrollToken {
                    friction: 4.0,
                    spring: SpringToken { response_ms: 300.0 },
                }),
            )
            .unwrap();
        // 132 px along at 750 px/s: the fling would end near 320, and the nearest anchor is 480.
        let rest = strip.world_at(released + 20_000 * MS);
        assert!(
            (rest.transform.tx + 480.0).abs() < 1e-2,
            "settled on the 480 anchor: {rest:?}"
        );
        for k in 0..40u64 {
            let t = released + k * 16 * MS;
            assert!(
                (strip.world_at(t).transform.tx + scroll.value_at(t)).abs() < 1e-3,
                "frame {k}"
            );
        }
        scroll.cancel(CancelPolicy::Complete).unwrap();
        let _ = Event::Stats(compositor_protocol::Stats::default());
    }

    #[test]
    fn a_track_that_goes_frees_what_is_bound_to_it_first_and_the_group_keeps_the_value_it_had() {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let now = at_time(&scene);
        now.set(0);
        let track = scene
            .motion(0.5, MotionBounds::range(0.0, 1.0, Units::Progress))
            .unwrap();
        let dock = scene.group(None, pose(0.0, 0.0)).unwrap();
        dock.bind(
            &track,
            PoseBinding {
                values: [0.0, 1.0],
                from: GroupPose {
                    opacity: 0.0,
                    ..pose(0.0, 100.0)
                },
                to: GroupPose {
                    opacity: 1.0,
                    ..pose(0.0, 0.0)
                },
                properties: PoseProperties::TRANSLATION.union(PoseProperties::OPACITY),
            },
        )
        .unwrap();
        scene.try_commit().unwrap();
        drain(&mut comp);
        let (m, g) = (track.wire_id(), dock.wire_id());
        drop(track);
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        let unbind = sent
            .iter()
            .position(|r| matches!(r, Request::GroupUnbind { .. }))
            .expect("unbound");
        let delete = sent
            .iter()
            .position(|r| matches!(r, Request::MotionDelete { .. }))
            .expect("deleted");
        assert!(unbind < delete, "bindings before tracks");
        assert_eq!(
            sent[unbind],
            Request::GroupUnbind {
                id: g,
                properties: PoseProperties::TRANSLATION.union(PoseProperties::OPACITY)
            }
        );
        assert_eq!(sent[delete], Request::MotionDelete { id: m });
        // The mirror kept what the binding had made: y = 50, opacity 0.5, now free.
        let pose = dock.pose_at(0);
        assert!(
            (pose.y - 50.0).abs() < 1e-4 && (pose.opacity - 0.5).abs() < 1e-4,
            "{pose:?}"
        );
    }

    #[test]
    fn set_pose_makes_a_pose_the_compositor_accepts_and_a_bound_property_still_follows_its_track() {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let g = scene.group(None, pose(5.0, 6.0)).unwrap();
        scene.try_commit().unwrap();
        drain(&mut comp);
        g.set_pose(GroupPose {
            x: f32::NAN,
            y: 9.0,
            scale: 1000.0,
            opacity: -2.0,
            ..pose(0.0, 0.0)
        });
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        let Request::GroupSet {
            pose: sent_pose, ..
        } = &sent[0]
        else {
            panic!("{sent:?}")
        };
        assert!(sent_pose.is_valid(), "{sent_pose:?}");
        assert_eq!(
            (sent_pose.x, sent_pose.y),
            (5.0, 9.0),
            "a number that is not one keeps the old value"
        );
        assert_eq!(sent_pose.scale, MAX_SCALE);
        assert_eq!(sent_pose.opacity, 0.0);
        // Inert hides it on the wire and `set_pose` while inert keeps it hidden.
        g.set_inert(true);
        g.set_pose(pose(1.0, 1.0));
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        let sets: Vec<&Request> = sent
            .iter()
            .filter(|r| matches!(r, Request::GroupSet { .. }))
            .collect();
        assert_eq!(sets.len(), 2, "the inert switch and the new pose: {sent:?}");
        assert!(
            sets.iter()
                .all(|r| matches!(r, Request::GroupSet { pose, .. } if !pose.visible)),
            "{sent:?}"
        );
        g.set_inert(false);
        scene.try_commit().unwrap();
        assert!(matches!(&drain(&mut comp)[0], Request::GroupSet { pose, .. } if pose.visible));
    }

    #[test]
    fn bindings_are_checked_and_a_binding_replaces_the_old_one_for_the_properties_it_names() {
        let (scene, _comp, _events) = connect(SceneOptions::default());
        let track = scene
            .motion(0.0, MotionBounds::range(0.0, 1.0, Units::Progress))
            .unwrap();
        let g = scene.group(None, pose(0.0, 0.0)).unwrap();
        let ok = PoseBinding {
            values: [0.0, 1.0],
            from: GroupPose::at(0.0, 0.0),
            to: GroupPose::at(10.0, 0.0),
            properties: PoseProperties::TRANSLATION,
        };
        for bad in [
            PoseBinding {
                values: [1.0, 1.0],
                ..ok
            },
            PoseBinding {
                values: [0.0, f32::NAN],
                ..ok
            },
            PoseBinding {
                properties: PoseProperties::NONE,
                ..ok
            },
            PoseBinding {
                properties: PoseProperties(8),
                ..ok
            },
            PoseBinding {
                properties: PoseProperties::SCALE,
                from: GroupPose {
                    scale: 0.0,
                    ..ok.from
                },
                ..ok
            },
        ] {
            assert!(
                matches!(g.bind(&track, bad), Err(UiError::Invalid(_))),
                "{bad:?}"
            );
        }
        g.bind(&track, ok).unwrap();
        let other = scene
            .motion(0.0, MotionBounds::range(0.0, 1.0, Units::Progress))
            .unwrap();
        // Rebound to a track with a steeper mapping: only the new binding drives translation at 0.5.
        g.bind(
            &other,
            PoseBinding {
                to: GroupPose::at(100.0, 0.0),
                ..ok
            },
        )
        .unwrap();
        other
            .settle(
                Some(0.5),
                Some(0.0),
                Settle::Timed(crate::motion::MotionToken {
                    duration_ms: 0,
                    ease: compositor_protocol::Ease::Linear,
                }),
            )
            .unwrap();
        assert!((g.pose_at(u64::MAX / 2).x - 50.0).abs() < 1e-3);
        // A motion that is gone is stale.
        let gone_id = other.id();
        drop(other);
        let _ = gone_id;
    }

    fn window() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        }
    }

    fn group_sets(sent: &[Request]) -> Vec<GroupPose> {
        sent.iter()
            .filter_map(|r| match r {
                Request::GroupSet { pose, .. } => Some(*pose),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_group_pose_with_a_feather_goes_over_the_wire_in_create_and_set() {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let fade = Edges::vertical(12.0);
        let g = scene
            .group(
                None,
                GroupPose {
                    clip: Some(window()),
                    feather: fade,
                    ..pose(5.0, 6.0)
                },
            )
            .unwrap();
        g.set_pose(GroupPose {
            clip: Some(window()),
            feather: Edges::horizontal(8.0),
            ..pose(5.0, 6.0)
        });
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        assert!(
            sent.iter().any(|r| matches!(
                r,
                Request::GroupCreate { pose, .. } if pose.feather == fade && pose.clip == Some(window())
            )),
            "{sent:?}"
        );
        let sets = group_sets(&sent);
        assert_eq!(sets.len(), 1, "{sent:?}");
        assert_eq!(sets[0].feather, Edges::horizontal(8.0));
    }

    #[test]
    fn set_clip_feathered_replaces_only_the_clip_and_feather_in_one_group_set() {
        let (scene, mut comp, _events) = connect(SceneOptions::default());
        let g = scene
            .group(
                None,
                GroupPose {
                    scale: 1.2,
                    opacity: 0.5,
                    ..pose(5.0, 9.0)
                },
            )
            .unwrap();
        scene.try_commit().unwrap();
        drain(&mut comp);
        g.set_clip_feathered(Some(window()), Edges::new(1.0, 2.0, 3.0, 4.0));
        scene.try_commit().unwrap();
        let sets = group_sets(&drain(&mut comp));
        assert_eq!(sets.len(), 1, "exactly one GroupSet");
        assert_eq!(
            sets[0],
            GroupPose {
                scale: 1.2,
                opacity: 0.5,
                clip: Some(window()),
                feather: Edges::new(1.0, 2.0, 3.0, 4.0),
                ..pose(5.0, 9.0)
            }
        );
        assert_eq!(g.pose_at(0).clip, Some(window()));
        // A width that is not a length is a hard edge on that side.
        g.set_clip_feathered(
            Some(window()),
            Edges::new(f32::NAN, -3.0, 4.0, f32::INFINITY),
        );
        scene.try_commit().unwrap();
        let sets = group_sets(&drain(&mut comp));
        assert_eq!(sets[0].feather, Edges::new(0.0, 0.0, 4.0, 0.0));
        // A feather without a clip means nothing and is dropped; clearing the clip clears it.
        g.set_clip_feathered(None, Edges::all(10.0));
        scene.try_commit().unwrap();
        let sets = group_sets(&drain(&mut comp));
        assert_eq!((sets[0].clip, sets[0].feather), (None, Edges::NONE));
        assert_eq!((sets[0].scale, sets[0].opacity), (1.2, 0.5));
    }

    #[test]
    fn the_world_of_a_feathered_group_carries_the_feather_in_screen_pixels_and_hits_ignore_it() {
        let (scene, _comp, _events) = connect(SceneOptions::default());
        let g = scene
            .group(
                None,
                GroupPose {
                    scale: 1.3,
                    clip: Some(window()),
                    feather: Edges::all(12.0),
                    ..pose(10.0, 20.0)
                },
            )
            .unwrap();
        let world = g.world_at(0);
        let f = world.feather;
        for w in [f.left, f.right, f.top, f.bottom] {
            assert!((w - 15.6).abs() < 1e-3, "{f:?}");
        }
        // The layer fills the window: screen x 10..140, y 20..150. A point inside the window but inside the fade is still hit.
        let layer = Props::at(0.0, 0.0);
        let size = (100.0, 100.0);
        assert!(g.hit_at(&layer, size, 11.0, 21.0, 0));
        assert!(g.hit_at(&layer, size, 75.0, 85.0, 0));
        assert!(!g.hit_at(&layer, size, 141.0, 85.0, 0));
        // The same group with a hard edge is hit in the same places.
        g.set_clip_feathered(Some(window()), Edges::NONE);
        assert_eq!(g.world_at(0).feather, Edges::NONE);
        assert!(g.hit_at(&layer, size, 11.0, 21.0, 0));
    }
}
