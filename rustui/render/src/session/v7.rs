//! The session's side of the version 7 requests: native groups, shared motion tracks, prepared backdrops, atlases and glyph
//! runs. Like the layers, everything the client has made is mirrored here (including what is only staged in the batch being
//! built) so that a request naming something that does not exist, a tree that would loop or be too deep, a number that is
//! not a number, or a limit that would be passed is refused on the spot, and every `Op` that reaches the render thread is one
//! it can carry out.

use super::{
    Done, MAX_BLUR_RADIUS, Refusal, Session, check_feather, check_props, check_rect, coordinate,
    ranged,
};
use crate::ops::{Op, Transaction};
use compositor_protocol::group::{MAX_GROUP_DEPTH, MAX_GROUPS, MAX_SCALE, MIN_SCALE};
use compositor_protocol::motion::MAX_MOTIONS;
use compositor_protocol::resources::{
    INKS, MAX_ATLAS_BYTES, MAX_ATLAS_SIDE, MAX_ATLASES, MAX_BACKDROPS, MAX_PLATES, MAX_RUN_GLYPHS,
    MAX_UPLOAD_BYTES, STAGING_SLOTS, staging_path,
};
use compositor_protocol::{
    AtlasId, AtlasRect, BackdropId, CancelPolicy, CaptureSpec, Content, DriveSample, Glyph,
    GroupId, GroupPose, LayerId, MotionBounds, MotionId, PoseBinding, PoseProperties, Rect,
    Request, Settle,
};
use std::collections::{HashMap, HashSet};

struct AtlasInfo {
    width: u32,
    height: u32,
    /// The generation of the newest upload: the next must be higher, so that `SetGlyphs::after` means one thing.
    generation: u64,
}

/// What the client has made of the v7 kinds, including what is staged and not yet committed.
#[derive(Default)]
pub(super) struct Mirror {
    /// Each group and its parent.
    groups: HashMap<GroupId, Option<GroupId>>,
    motions: HashSet<MotionId>,
    /// The tracks and prepared sets made since the last commit: a request that acts at once on one of them must wait in the
    /// batch behind its creation, in order, instead of overtaking it.
    new_motions: HashSet<MotionId>,
    backdrops: HashSet<BackdropId>,
    new_backdrops: HashSet<BackdropId>,
    atlases: HashMap<AtlasId, AtlasInfo>,
    atlas_bytes: u64,
}

impl Mirror {
    /// The batch has been committed.
    pub(super) fn committed(&mut self) {
        self.new_motions.clear();
        self.new_backdrops.clear();
    }

    fn depth(&self, id: GroupId) -> usize {
        let mut depth = 0;
        let mut at = Some(id);
        while let Some(g) = at {
            depth += 1;
            if depth > MAX_GROUP_DEPTH + 1 {
                break;
            }
            at = self.groups.get(&g).copied().flatten();
        }
        depth
    }

    fn height(&self, id: GroupId) -> usize {
        1 + self
            .groups
            .iter()
            .filter(|(_, parent)| **parent == Some(id))
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
            at = self.groups.get(&g).copied().flatten();
        }
        false
    }
}

/// A pose with sane numbers; a scale or an opacity that drifts a hair out of range is pulled in, like a layer's props, and so
/// is a clip size that drifts below zero (an animation's rounding gives -1e-8). Whatever this returns is a pose the engine
/// takes (`GroupPose::is_valid`): the engine refuses a pose it does not take without a word, and a client's mirror of
/// its groups would then hold groups the engine does not have.
pub(super) fn check_pose(pose: GroupPose) -> Result<GroupPose, String> {
    Ok(GroupPose {
        x: coordinate("group x", pose.x)?,
        y: coordinate("group y", pose.y)?,
        scale: coordinate("group scale", pose.scale)?.clamp(MIN_SCALE, MAX_SCALE),
        pivot_x: coordinate("group pivot x", pose.pivot_x)?,
        pivot_y: coordinate("group pivot y", pose.pivot_y)?,
        opacity: coordinate("group opacity", pose.opacity)?.clamp(0.0, 1.0),
        visible: pose.visible,
        clip: pose
            .clip
            .map(|clip| {
                check_rect("group clip", clip).map(|clip| Rect {
                    w: clip.w.max(0.0),
                    h: clip.h.max(0.0),
                    ..clip
                })
            })
            .transpose()?,
        feather: check_feather("group clip feather", pose.feather, pose.clip.is_some())?,
    })
}

fn check_binding(binding: PoseBinding) -> Result<PoseBinding, String> {
    binding.check().map_err(str::to_owned)?;
    let values = [
        coordinate("binding value", binding.values[0])?,
        coordinate("binding value", binding.values[1])?,
    ];
    let mut from = binding.from;
    let mut to = binding.to;
    for pose in [&mut from, &mut to] {
        pose.x = coordinate("binding x", pose.x)?;
        pose.y = coordinate("binding y", pose.y)?;
        if binding.properties.contains(PoseProperties::SCALE) {
            pose.scale = ranged("binding scale", pose.scale, MIN_SCALE, MAX_SCALE)?;
        }
        if binding.properties.contains(PoseProperties::OPACITY) {
            pose.opacity = ranged("binding opacity", pose.opacity, 0.0, 1.0)?;
        }
    }
    Ok(PoseBinding {
        values,
        from,
        to,
        ..binding
    })
}

impl Session {
    fn group_known(&self, id: GroupId) -> Result<(), Refusal> {
        if self.v7.groups.contains_key(&id) {
            Ok(())
        } else {
            Err(Refusal::new(format!("group {} does not exist", id.0)))
        }
    }

    fn motion_known(&self, id: MotionId) -> Result<(), Refusal> {
        if self.v7.motions.contains(&id) {
            Ok(())
        } else {
            Err(Refusal::new(format!(
                "motion track {} does not exist",
                id.0
            )))
        }
    }

    fn backdrop_known(&self, id: BackdropId) -> Result<(), Refusal> {
        if self.v7.backdrops.contains(&id) {
            Ok(())
        } else {
            Err(Refusal::new(format!(
                "prepared backdrop {} does not exist",
                id.0
            )))
        }
    }

    fn atlas_known(&self, id: AtlasId) -> Result<&AtlasInfo, Refusal> {
        self.v7
            .atlases
            .get(&id)
            .ok_or_else(|| Refusal::new(format!("atlas {} does not exist", id.0)))
    }

    /// A request that acts at once rather than with the commit: on its own, unless what it names is still staged in the
    /// batch, where it waits behind that in order.
    fn at_once(&mut self, staged: bool, op: Op) -> Done {
        if staged {
            self.batch.push(op);
            Done::Nothing
        } else {
            Done::Batch(Transaction {
                client: self.client,
                tag: None,
                ops: vec![op],
                received_ns: 0,
            })
        }
    }

    /// What a layer's content refers to must exist: the atlas of a glyph run, the prepared set of a glass surface.
    pub(super) fn check_references(&self, content: &Content) -> Result<(), Refusal> {
        match content {
            Content::Glyphs(style) => self.atlas_known(style.atlas).map(|_| ()),
            Content::Glass(glass) => match glass.backdrop {
                compositor_protocol::Backdrop::Prepared { id } => self.backdrop_known(id),
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }

    pub(super) fn dispatch_v7(&mut self, request: Request) -> Result<Done, Refusal> {
        match request {
            Request::GroupCreate { id, parent, pose } => self.group_create(id, parent, pose),
            Request::GroupSet { id, pose } => {
                self.group_known(id)?;
                let pose = check_pose(pose)?;
                self.batch.push(Op::GroupSet { id, pose });
                Ok(Done::Nothing)
            }
            Request::GroupParent {
                id,
                parent,
                preserve_world,
            } => {
                self.group_known(id)?;
                if let Some(p) = parent {
                    self.group_known(p)?;
                    if self.v7.is_below(p, id) {
                        return Err(Refusal::new(format!(
                            "group {} cannot go under {}: that would make a loop",
                            id.0, p.0
                        )));
                    }
                    if self.v7.depth(p) + self.v7.height(id) > MAX_GROUP_DEPTH {
                        return Err(Refusal::new(format!(
                            "groups nest at most {MAX_GROUP_DEPTH} deep"
                        )));
                    }
                }
                self.v7.groups.insert(id, parent);
                self.batch.push(Op::GroupParent {
                    id,
                    parent,
                    preserve_world,
                });
                Ok(Done::Nothing)
            }
            Request::GroupBind {
                id,
                motion,
                binding,
            } => {
                self.group_known(id)?;
                self.motion_known(motion)?;
                let binding = check_binding(binding)?;
                self.batch.push(Op::GroupBind {
                    id,
                    motion,
                    binding,
                });
                Ok(Done::Nothing)
            }
            Request::GroupUnbind { id, properties } => {
                self.group_known(id)?;
                if !properties.is_valid() {
                    return Err(Refusal::new("a pose property that is not one"));
                }
                self.batch.push(Op::GroupUnbind { id, properties });
                Ok(Done::Nothing)
            }
            Request::GroupDelete { id } => {
                // Deleting what is not there is fine, as for layers.
                if let Some(parent) = self.v7.groups.remove(&id) {
                    for p in self.v7.groups.values_mut().filter(|p| **p == Some(id)) {
                        *p = parent;
                    }
                    for known in self.layers.values_mut().filter(|k| k.group == Some(id)) {
                        known.group = parent;
                    }
                    self.batch.push(Op::GroupDelete { id });
                }
                Ok(Done::Nothing)
            }
            Request::Attach { id, group, local } => {
                self.known(id)?;
                if let Some(g) = group {
                    self.group_known(g)?;
                }
                let local = check_props(local)?;
                if let Some(known) = self.layers.get_mut(&id) {
                    known.group = group;
                }
                self.batch.push(Op::Attach { id, group, local });
                Ok(Done::Nothing)
            }
            Request::MotionCreate { id, value, bounds } => self.motion_create(id, value, bounds),
            Request::MotionDelete { id } => {
                if self.v7.motions.remove(&id) {
                    self.v7.new_motions.remove(&id);
                    self.batch.push(Op::MotionDelete { id });
                }
                Ok(Done::Nothing)
            }
            Request::MotionDrive { id, sample } => self.motion_drive(id, sample),
            Request::MotionSettle {
                id,
                target,
                velocity,
                settle,
            } => self.motion_settle(id, target, velocity, settle),
            Request::MotionCancel { id, policy } => self.motion_cancel(id, policy),
            Request::BackdropCreate { id, spec } => self.backdrop_create(id, spec),
            Request::BackdropRefresh { id, content } => {
                self.backdrop_known(id)?;
                self.batch.push(Op::BackdropRefresh { id, content });
                Ok(Done::Nothing)
            }
            Request::BackdropFreeze { id } => {
                self.backdrop_known(id)?;
                let staged = self.v7.new_backdrops.contains(&id);
                Ok(self.at_once(staged, Op::BackdropFreeze { id }))
            }
            Request::BackdropRelease { id } => {
                self.backdrop_known(id)?;
                let staged = self.v7.new_backdrops.contains(&id);
                Ok(self.at_once(staged, Op::BackdropRelease { id }))
            }
            Request::BackdropDelete { id } => {
                if self.v7.backdrops.remove(&id) {
                    self.v7.new_backdrops.remove(&id);
                    self.batch.push(Op::BackdropDelete { id });
                }
                Ok(Done::Nothing)
            }
            Request::AtlasCreate { id, width, height } => self.atlas_create(id, width, height),
            Request::AtlasUpload {
                id,
                slot,
                generation,
                rect,
            } => self.atlas_upload(id, slot, generation, rect),
            Request::AtlasDelete { id } => {
                if self.v7.atlases.contains_key(&id) {
                    if self
                        .layers
                        .values()
                        .any(|k| matches!(k.content, Content::Glyphs(s) if s.atlas == id))
                    {
                        return Err(Refusal::new(format!(
                            "atlas {} is still the atlas of a glyph run",
                            id.0
                        )));
                    }
                    if let Some(info) = self.v7.atlases.remove(&id) {
                        self.v7.atlas_bytes -= u64::from(info.width) * u64::from(info.height);
                    }
                    self.batch.push(Op::AtlasDelete { id });
                }
                Ok(Done::Nothing)
            }
            Request::SetGlyphs { id, after, glyphs } => self.set_glyphs(id, after, glyphs),
            other => unreachable!("the dispatcher sends only version 7 requests here: {other:?}"),
        }
    }

    fn group_create(
        &mut self,
        id: GroupId,
        parent: Option<GroupId>,
        pose: GroupPose,
    ) -> Result<Done, Refusal> {
        if self.v7.groups.contains_key(&id) {
            return Err(Refusal::new(format!("group {} exists already", id.0)));
        }
        if self.v7.groups.len() >= MAX_GROUPS {
            return Err(Refusal::new(format!("more than {MAX_GROUPS} groups")));
        }
        if let Some(p) = parent {
            self.group_known(p)?;
            if self.v7.depth(p) >= MAX_GROUP_DEPTH {
                return Err(Refusal::new(format!(
                    "groups nest at most {MAX_GROUP_DEPTH} deep"
                )));
            }
        }
        let pose = check_pose(pose)?;
        self.v7.groups.insert(id, parent);
        self.batch.push(Op::GroupCreate { id, parent, pose });
        Ok(Done::Nothing)
    }

    fn motion_create(
        &mut self,
        id: MotionId,
        value: f32,
        bounds: MotionBounds,
    ) -> Result<Done, Refusal> {
        if self.v7.motions.contains(&id) {
            return Err(Refusal::new(format!(
                "motion track {} exists already",
                id.0
            )));
        }
        if self.v7.motions.len() >= MAX_MOTIONS {
            return Err(Refusal::new(format!(
                "more than {MAX_MOTIONS} motion tracks"
            )));
        }
        coordinate("motion value", value)?;
        bounds.check().map_err(str::to_owned)?;
        for (name, v) in [
            ("motion minimum", bounds.min),
            ("motion maximum", bounds.max),
            ("motion overscroll", bounds.overscroll),
            ("motion ratio", bounds.ratio),
        ] {
            coordinate(name, v)?;
        }
        self.v7.motions.insert(id);
        self.v7.new_motions.insert(id);
        self.batch.push(Op::MotionCreate { id, value, bounds });
        Ok(Done::Nothing)
    }

    fn motion_drive(&mut self, id: MotionId, sample: DriveSample) -> Result<Done, Refusal> {
        self.motion_known(id)?;
        coordinate("pointer position", sample.position)?;
        let staged = self.v7.new_motions.contains(&id);
        Ok(self.at_once(staged, Op::MotionDrive { id, sample }))
    }

    fn motion_settle(
        &mut self,
        id: MotionId,
        target: Option<f32>,
        velocity: Option<f32>,
        settle: Settle,
    ) -> Result<Done, Refusal> {
        self.motion_known(id)?;
        settle.check().map_err(str::to_owned)?;
        if let Some(t) = target {
            coordinate("settle target", t)?;
        }
        if let Some(v) = velocity {
            coordinate("settle velocity", v)?;
        }
        let staged = self.v7.new_motions.contains(&id);
        Ok(self.at_once(
            staged,
            Op::MotionSettle {
                id,
                target,
                velocity,
                settle,
            },
        ))
    }

    fn motion_cancel(&mut self, id: MotionId, policy: CancelPolicy) -> Result<Done, Refusal> {
        self.motion_known(id)?;
        let staged = self.v7.new_motions.contains(&id);
        Ok(self.at_once(staged, Op::MotionCancel { id, policy }))
    }

    fn backdrop_create(&mut self, id: BackdropId, spec: CaptureSpec) -> Result<Done, Refusal> {
        if self.v7.backdrops.contains(&id) {
            return Err(Refusal::new(format!(
                "prepared backdrop {} exists already",
                id.0
            )));
        }
        if self.v7.backdrops.len() >= MAX_BACKDROPS {
            return Err(Refusal::new(format!(
                "more than {MAX_BACKDROPS} prepared backdrops"
            )));
        }
        if spec.plates.len() > MAX_PLATES {
            return Err(Refusal::new(format!(
                "more than {MAX_PLATES} plates in a prepared backdrop"
            )));
        }
        if !matches!(spec.downsample, 2 | 4 | 8) {
            return Err(Refusal::new(format!(
                "plates are made at 1/2, 1/4 or 1/8 of the screen, not 1/{}",
                spec.downsample
            )));
        }
        let radius = ranged("blur radius", spec.radius, 0.0, MAX_BLUR_RADIUS)?;
        if let Some(wallpaper) = spec.wallpaper {
            self.known(wallpaper)?;
        }
        for plate in &spec.plates {
            self.group_known(plate.group)?;
        }
        self.v7.backdrops.insert(id);
        self.v7.new_backdrops.insert(id);
        self.batch.push(Op::BackdropCreate {
            id,
            spec: CaptureSpec { radius, ..spec },
        });
        Ok(Done::Nothing)
    }

    fn atlas_create(&mut self, id: AtlasId, width: u32, height: u32) -> Result<Done, Refusal> {
        if self.v7.atlases.contains_key(&id) {
            return Err(Refusal::new(format!("atlas {} exists already", id.0)));
        }
        if self.v7.atlases.len() >= MAX_ATLASES {
            return Err(Refusal::new(format!("more than {MAX_ATLASES} atlases")));
        }
        let side = 1..=MAX_ATLAS_SIDE;
        if !side.contains(&width) || !side.contains(&height) {
            return Err(Refusal::new(format!(
                "a {width}x{height} atlas is outside 1..={MAX_ATLAS_SIDE} pixels each way"
            )));
        }
        let bytes = u64::from(width) * u64::from(height);
        if self.v7.atlas_bytes + bytes > MAX_ATLAS_BYTES {
            return Err(Refusal::new(format!(
                "atlases would hold more than {} MiB",
                MAX_ATLAS_BYTES >> 20
            )));
        }
        self.v7.atlas_bytes += bytes;
        self.v7.atlases.insert(
            id,
            AtlasInfo {
                width,
                height,
                generation: 0,
            },
        );
        self.batch.push(Op::AtlasCreate { id, width, height });
        Ok(Done::Nothing)
    }

    fn atlas_upload(
        &mut self,
        id: AtlasId,
        slot: u8,
        generation: u64,
        rect: AtlasRect,
    ) -> Result<Done, Refusal> {
        let info = self.atlas_known(id)?;
        if slot >= STAGING_SLOTS {
            return Err(Refusal::new(format!(
                "staging slot {slot} is not one of the {STAGING_SLOTS} an atlas has"
            )));
        }
        let (w, h) = (u32::from(rect.w), u32::from(rect.h));
        let inside = w > 0
            && h > 0
            && u32::from(rect.x) + w <= info.width
            && u32::from(rect.y) + h <= info.height;
        if !inside {
            return Err(Refusal::new(format!(
                "an upload of {w}x{h} at ({}, {}) is not inside the {}x{} atlas",
                rect.x, rect.y, info.width, info.height
            )));
        }
        if rect.bytes() > MAX_UPLOAD_BYTES {
            return Err(Refusal::new(format!(
                "an upload of {} bytes is more than the {MAX_UPLOAD_BYTES} one request may carry",
                rect.bytes()
            )));
        }
        if generation <= info.generation {
            return Err(Refusal::new(format!(
                "upload generation {generation} does not follow {}",
                info.generation
            )));
        }
        let path = self.config.in_shm_dir(staging_path(id, slot));
        if let Some(info) = self.v7.atlases.get_mut(&id) {
            info.generation = generation;
        }
        self.batch.push(Op::AtlasUpload {
            id,
            slot,
            generation,
            rect,
            path,
        });
        Ok(Done::Nothing)
    }

    fn set_glyphs(&mut self, id: LayerId, after: u64, glyphs: Vec<Glyph>) -> Result<Done, Refusal> {
        let Content::Glyphs(style) = self.known(id)?.content else {
            return Err(Refusal::new(format!("layer {} is not a glyph run", id.0)));
        };
        if glyphs.len() > MAX_RUN_GLYPHS {
            return Err(Refusal::new(format!(
                "{} glyphs in one run; the most is {MAX_RUN_GLYPHS}",
                glyphs.len()
            )));
        }
        let info = self.atlas_known(style.atlas)?;
        for (index, glyph) in glyphs.iter().enumerate() {
            if usize::from(glyph.ink) >= INKS {
                return Err(Refusal::new(format!(
                    "glyph {index} has ink {}; a run has {INKS} inks, 0 to {}",
                    glyph.ink,
                    INKS - 1
                )));
            }
            if index > 0 && glyph.ink < glyphs[index - 1].ink {
                return Err(Refusal::new(format!(
                    "glyph {index} has ink {} after ink {}; a run's glyphs come in non-decreasing ink order",
                    glyph.ink,
                    glyphs[index - 1].ink
                )));
            }
            coordinate("glyph x", glyph.x)?;
            coordinate("glyph y", glyph.y)?;
            let inside = glyph.w > 0
                && glyph.h > 0
                && u32::from(glyph.sx) + u32::from(glyph.w) <= info.width
                && u32::from(glyph.sy) + u32::from(glyph.h) <= info.height;
            if !inside {
                return Err(Refusal::new(format!(
                    "a glyph of {}x{} at ({}, {}) is not inside the {}x{} atlas",
                    glyph.w, glyph.h, glyph.sx, glyph.sy, info.width, info.height
                )));
            }
        }
        self.batch.push(Op::SetGlyphs { id, after, glyphs });
        Ok(Done::Nothing)
    }
}
