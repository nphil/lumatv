//! The retained group tree and the shared motion tracks: the compositor's side of `compositor_protocol::{group, motion}`.
//!
//! A group carries a pose; its properties may follow motion tracks (`PoseBinding`). Every frame the tree is evaluated once,
//! parents first, into two worlds for each group: *now* (tracks at the value they have at the frame's time) and *at rest*
//! (tracks at the value they had when they began to move, or their value when they are still). Layers under a group are
//! flattened into plain screen-space `Props` of both kinds by `scene`, so the renderer needs to know nothing of groups:
//! what it blurs into a cached backdrop is the layers at rest, and a drag or a settle moves the layers without a cached
//! backdrop being invalidated.
//!
//! The math (similarity transforms, worlds, bindings, the tracks' closed forms) is `compositor_protocol`'s, shared with the
//! app; this file keeps the state and the order in which things are evaluated. Nothing here allocates once the maps have
//! reached the ledger's sizes (`MAX_GROUPS` groups, `MAX_MOTIONS` tracks).

use compositor_protocol::group::{MAX_GROUP_DEPTH, MAX_GROUPS, World, pose_preserving_world};
use compositor_protocol::motion::{MAX_MOTIONS, Notice, Track};
use compositor_protocol::{
    CancelPolicy, DriveSample, GroupId, GroupPose, MotionBounds, MotionId, PoseBinding,
    PoseProperties, Settle,
};
use std::collections::HashMap;

/// The three bindable properties, one binding slot each.
const PROPERTIES: [PoseProperties; 3] = [
    PoseProperties::TRANSLATION,
    PoseProperties::SCALE,
    PoseProperties::OPACITY,
];

/// A binding of one property: `binding.properties` is exactly that property.
#[derive(Clone, Copy, Debug)]
struct Bound {
    motion: MotionId,
    binding: PoseBinding,
}

struct Node {
    parent: Option<GroupId>,
    pose: GroupPose,
    bound: [Option<Bound>; 3],
}

struct Motion {
    track: Track,
    /// The value at rest: the track's value while it is still, and the one it had when it began to move while it moves.
    rest: f32,
}

/// A group's world now and at rest.
#[derive(Clone, Copy, Debug)]
pub struct Worlds {
    pub now: World,
    pub rest: World,
}

#[derive(Default)]
pub struct Groups {
    nodes: HashMap<GroupId, Node>,
    /// Every group, parents before children: the order the tree is evaluated in.
    order: Vec<GroupId>,
    motions: HashMap<MotionId, Motion>,
    worlds: HashMap<GroupId, Worlds>,
    notices: Vec<(MotionId, Notice)>,
}

impl Groups {
    pub fn new() -> Self {
        let mut groups = Self::default();
        groups.nodes.reserve(MAX_GROUPS);
        groups.order.reserve(MAX_GROUPS);
        groups.worlds.reserve(MAX_GROUPS);
        groups.motions.reserve(MAX_MOTIONS);
        groups.notices.reserve(2 * MAX_MOTIONS);
        groups
    }

    pub fn group_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn motion_count(&self) -> usize {
        self.motions.len()
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.order.clear();
        self.motions.clear();
        self.worlds.clear();
        self.notices.clear();
    }

    pub fn contains(&self, id: GroupId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// Whether a finger is on a track or one settles: frames are wanted at once, and no plate is captured.
    pub fn any_active(&self) -> bool {
        self.motions.values().any(|m| m.track.is_active())
    }

    /// Whether a settle is running, which needs a frame at every refresh. A finger that is down and still needs none.
    pub fn any_settling(&self) -> bool {
        self.motions.values().any(|m| m.track.is_settling())
    }

    /// Whether a settle that runs now will have arrived by `at_ns`: the frame drawn for that time brings the track's
    /// layers to rest, and the cached pictures behind glass are made again in it.
    pub fn any_settles_by(&self, at_ns: u64) -> bool {
        self.motions.values().any(|m| {
            let mut track = m.track;
            track.is_settling() && track.advance(at_ns).is_some()
        })
    }

    /// Whether a track is anywhere but at rest after the frame drawn for `at_ns`: a finger is on it (down, or just off it with no
    /// settle asked for yet), or a settle that runs now has not arrived by then.
    pub fn any_moving_after(&self, at_ns: u64) -> bool {
        self.motions.values().any(|m| {
            let mut track = m.track;
            track.is_moving() && (!track.is_settling() || track.advance(at_ns).is_none())
        })
    }

    /// The notices (a settle began, a finger caught one, a track came to rest) since the last call, appended to `out`.
    pub fn take_notices(&mut self, out: &mut Vec<(MotionId, Notice)>) {
        out.append(&mut self.notices);
    }

    pub fn worlds(&self, id: GroupId) -> Option<&Worlds> {
        self.worlds.get(&id)
    }

    /// The depth of `id`, counting itself (a group under the screen is 1).
    fn depth(&self, id: GroupId) -> usize {
        let mut depth = 0;
        let mut at = Some(id);
        while let Some(g) = at {
            depth += 1;
            if depth > MAX_GROUP_DEPTH + 1 {
                break;
            }
            at = self.nodes.get(&g).and_then(|n| n.parent);
        }
        depth
    }

    /// The deepest group among `id` and everything below it, relative to `id` (1 for a group with no children).
    fn height(&self, id: GroupId) -> usize {
        1 + self
            .nodes
            .iter()
            .filter(|(_, n)| n.parent == Some(id))
            .map(|(child, _)| self.height(*child))
            .max()
            .unwrap_or(0)
    }

    fn is_below(&self, candidate: GroupId, ancestor: GroupId) -> bool {
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
            at = self.nodes.get(&g).and_then(|n| n.parent);
        }
        false
    }

    /// The group's pose with every bound property at its track's value (now, or at rest).
    fn pose_of(&self, node: &Node, rest: bool) -> GroupPose {
        let mut pose = node.pose;
        for bound in node.bound.iter().flatten() {
            if let Some(m) = self.motions.get(&bound.motion) {
                bound
                    .binding
                    .apply(if rest { m.rest } else { m.track.value() }, &mut pose);
            }
        }
        pose
    }

    /// The evaluated world of `id`, walking up its ancestors: for the rare uses (an op that preserves a world, a plate's
    /// placement); the frame itself evaluates the whole tree once (`evaluate`).
    fn chain_world(&self, id: GroupId, rest: bool) -> World {
        let mut chain = [GroupId(0); MAX_GROUP_DEPTH + 1];
        let mut len = 0;
        let mut at = Some(id);
        while let Some(g) = at {
            if len == chain.len() || !self.nodes.contains_key(&g) {
                break;
            }
            chain[len] = g;
            len += 1;
            at = self.nodes[&g].parent;
        }
        let mut world = World::ROOT;
        for g in chain[..len].iter().rev() {
            world = world.child(&self.pose_of(&self.nodes[g], rest));
        }
        world
    }

    /// The world of group `from` as `ancestor`'s own pose and everything above it are the identity: what a plate made from
    /// `ancestor` draws a layer of `from` with. `None` when `from` is not `ancestor` or below it.
    pub fn world_below(&self, from: GroupId, ancestor: GroupId, rest: bool) -> Option<World> {
        let mut chain = [GroupId(0); MAX_GROUP_DEPTH + 1];
        let mut len = 0;
        let mut at = Some(from);
        while let Some(g) = at {
            if g == ancestor {
                let mut world = World::ROOT;
                for g in chain[..len].iter().rev() {
                    world = world.child(&self.pose_of(&self.nodes[g], rest));
                }
                return Some(world);
            }
            if len == chain.len() || !self.nodes.contains_key(&g) {
                return None;
            }
            chain[len] = g;
            len += 1;
            at = self.nodes[&g].parent;
        }
        None
    }

    /// Evaluates the whole tree for the frame: tracks are at the value they have at the frame's time (`advance` has run).
    pub fn evaluate(&mut self) {
        self.worlds.clear();
        for id in &self.order {
            let node = &self.nodes[id];
            let (parent_now, parent_rest) = match node.parent.and_then(|p| self.worlds.get(&p)) {
                Some(w) => (w.now, w.rest),
                None => (World::ROOT, World::ROOT),
            };
            let now = parent_now.child(&self.pose_of(node, false));
            let rest = parent_rest.child(&self.pose_of(node, true));
            self.worlds.insert(*id, Worlds { now, rest });
        }
    }

    // --- Groups ---

    /// Makes a group. Refused (`false`) when it exists, its parent does not, the tree would be too deep or too big, or the
    /// pose is not valid: the session checks all of that, so a refusal here is a stale request.
    pub fn create(&mut self, id: GroupId, parent: Option<GroupId>, pose: GroupPose) -> bool {
        if self.nodes.contains_key(&id)
            || self.nodes.len() >= MAX_GROUPS
            || !pose.is_valid()
            || parent
                .is_some_and(|p| !self.nodes.contains_key(&p) || self.depth(p) >= MAX_GROUP_DEPTH)
        {
            return false;
        }
        self.nodes.insert(
            id,
            Node {
                parent,
                pose,
                bound: [None; 3],
            },
        );
        // Parents come first, and this one is new: it goes last.
        self.order.push(id);
        true
    }

    /// Sets the group's own pose. A bound property keeps following its track.
    pub fn set_pose(&mut self, id: GroupId, pose: GroupPose) {
        if pose.is_valid() {
            if let Some(node) = self.nodes.get_mut(&id) {
                node.pose = pose;
            }
        }
    }

    /// Binds the properties of `binding` to `motion`, replacing earlier bindings of those properties.
    pub fn bind(&mut self, id: GroupId, motion: MotionId, binding: PoseBinding) {
        if binding.check().is_err() || !self.motions.contains_key(&motion) {
            return;
        }
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        for (slot, property) in node.bound.iter_mut().zip(PROPERTIES) {
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
    }

    /// Frees the properties from their bindings: they keep the values they have now.
    pub fn unbind(&mut self, id: GroupId, properties: PoseProperties) {
        let Some(node) = self.nodes.get(&id) else {
            return;
        };
        let mut pose = node.pose;
        let mut freed = [false; 3];
        for (k, property) in PROPERTIES.iter().enumerate() {
            if let (true, Some(bound)) = (properties.contains(*property), node.bound[k]) {
                if let Some(m) = self.motions.get(&bound.motion) {
                    bound.binding.apply(m.track.value(), &mut pose);
                }
                freed[k] = true;
            }
        }
        let node = self.nodes.get_mut(&id).expect("looked up above");
        node.pose = pose;
        for (slot, freed) in node.bound.iter_mut().zip(freed) {
            if freed {
                *slot = None;
            }
        }
    }

    /// Moves `id` under `parent`. With `preserve_world` its evaluated world now stays as it is: the pose that leaves it so
    /// replaces the free properties, and the bindings of the bound ones are shifted (translation, added to both ends) or
    /// scaled (scale and opacity, by the ratio) by the same difference, so they keep following their tracks from there.
    /// Refused (`false`) when the move would make a cycle or a tree too deep: a stale request, the session checks both.
    pub fn reparent(&mut self, id: GroupId, parent: Option<GroupId>, preserve_world: bool) -> bool {
        if !self.nodes.contains_key(&id) {
            return false;
        }
        if let Some(p) = parent {
            if !self.nodes.contains_key(&p)
                || self.is_below(p, id)
                || self.depth(p) + self.height(id) > MAX_GROUP_DEPTH
            {
                return false;
            }
        }
        if preserve_world {
            let world = self.chain_world(id, false);
            let new_parent = parent.map_or(World::ROOT, |p| self.chain_world(p, false));
            let node = &self.nodes[&id];
            let evaluated = self.pose_of(node, false);
            let wanted = pose_preserving_world(&world, &new_parent, &evaluated);
            let ratio = |new: f32, old: f32| if old.abs() > 1.0e-6 { new / old } else { 1.0 };
            let (dx, dy) = (wanted.x - evaluated.x, wanted.y - evaluated.y);
            let scale = ratio(wanted.scale, evaluated.scale);
            let opacity = ratio(wanted.opacity, evaluated.opacity);
            let node = self.nodes.get_mut(&id).expect("looked up above");
            for (slot, property) in node.bound.iter_mut().zip(PROPERTIES) {
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
                            node.pose.x = wanted.x;
                            node.pose.y = wanted.y;
                        } else if property == PoseProperties::SCALE {
                            node.pose.scale = wanted.scale;
                        } else {
                            node.pose.opacity = wanted.opacity;
                        }
                    }
                }
            }
        }
        self.nodes.get_mut(&id).expect("looked up above").parent = parent;
        self.sort();
        true
    }

    /// Deletes a group. The groups under it move to its parent with the world they had; what the caller must do for the
    /// layers under it is returned: the parent they go to and the world of the deleted group relative to that parent (its
    /// own pose, evaluated now), which turns a layer's props in the group's space into props in the parent's. `None` when
    /// there is no such group.
    pub fn delete(&mut self, id: GroupId) -> Option<(Option<GroupId>, World)> {
        let node = self.nodes.get(&id)?;
        let parent = node.parent;
        let relative = World::ROOT.child(&self.pose_of(node, false));
        let children: Vec<GroupId> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.parent == Some(id))
            .map(|(child, _)| *child)
            .collect();
        for child in children {
            // Cannot fail: the child's new parent is the deleted group's own, one level up.
            self.reparent(child, parent, true);
        }
        self.nodes.remove(&id);
        self.worlds.remove(&id);
        self.order.retain(|g| *g != id);
        Some((parent, relative))
    }

    /// Puts `order` right after a change to the tree's shape: parents before children, in a stable order.
    fn sort(&mut self) {
        let mut placed: Vec<GroupId> = Vec::with_capacity(self.nodes.len());
        let mut pending: Vec<GroupId> = self.order.clone();
        while !pending.is_empty() {
            let before = pending.len();
            pending.retain(|g| {
                let ready = self.nodes[g].parent.is_none_or(|p| placed.contains(&p));
                if ready {
                    placed.push(*g);
                }
                !ready
            });
            if pending.len() == before {
                // Unreachable for a tree without cycles; whatever is left cannot be placed and is dropped from the order
                // rather than looping.
                break;
            }
        }
        self.order = placed;
    }

    // --- Motion tracks ---

    pub fn create_motion(&mut self, id: MotionId, value: f32, bounds: MotionBounds) -> bool {
        if self.motions.contains_key(&id)
            || self.motions.len() >= MAX_MOTIONS
            || !value.is_finite()
            || bounds.check().is_err()
        {
            return false;
        }
        let track = Track::new(value, bounds);
        self.motions.insert(
            id,
            Motion {
                rest: track.value(),
                track,
            },
        );
        true
    }

    /// Deletes a track. What was bound to it keeps the value it has.
    pub fn delete_motion(&mut self, id: MotionId) {
        let Some(m) = self.motions.get(&id) else {
            return;
        };
        let value = m.track.value();
        for node in self.nodes.values_mut() {
            for slot in &mut node.bound {
                if let Some(bound) = slot.filter(|b| b.motion == id) {
                    bound.binding.apply(value, &mut node.pose);
                    *slot = None;
                }
            }
        }
        self.motions.remove(&id);
    }

    fn note(&mut self, id: MotionId, notice: Option<Notice>) {
        if let Some(notice) = notice {
            self.notices.push((id, notice));
        }
        if let Some(m) = self.motions.get_mut(&id) {
            if !m.track.is_moving() {
                m.rest = m.track.value();
            }
        }
    }

    pub fn drive(&mut self, id: MotionId, sample: DriveSample, now_ns: u64) {
        let notice = self
            .motions
            .get_mut(&id)
            .and_then(|m| m.track.drive(sample, now_ns));
        self.note(id, notice);
    }

    pub fn settle(
        &mut self,
        id: MotionId,
        target: Option<f32>,
        velocity: Option<f32>,
        settle: Settle,
        now_ns: u64,
    ) {
        let notice = self
            .motions
            .get_mut(&id)
            .map(|m| m.track.settle(now_ns, target, velocity, settle));
        self.note(id, notice);
    }

    pub fn cancel(&mut self, id: MotionId, policy: CancelPolicy, now_ns: u64) {
        let notice = self
            .motions
            .get_mut(&id)
            .and_then(|m| m.track.cancel(now_ns, policy));
        self.note(id, notice);
    }

    /// Brings every settling track to the frame's time.
    pub fn advance(&mut self, now_ns: u64) {
        for (id, m) in &mut self.motions {
            if m.track.is_settling() {
                if let Some(notice) = m.track.advance(now_ns) {
                    self.notices.push((*id, notice));
                }
            }
            if !m.track.is_moving() {
                m.rest = m.track.value();
            }
        }
    }
}
