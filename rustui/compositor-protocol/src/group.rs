//! Native retained groups: the shared pose and transform math, evaluated identically by the compositor (to draw) and
//! by the app (to hit-test and to estimate), so a pixel the compositor draws is a pixel the app can hit.
//!
//! A group is a node of a tree (depth at most [`MAX_GROUP_DEPTH`]) that carries a *pose*: a translation, a uniform scale
//! about a pivot, an opacity, a visibility and an optional clip. Layers and other groups hang under it with their own
//! geometry in the group's local space, so one pose change moves, scales and fades everything below it, with no
//! per-child request and no offscreen texture. A group's pose properties can be bound to shared motion tracks
//! ([`PoseBinding`]); several groups bound to one track move exactly together.
//!
//! The transform of every node is a *similarity*, a uniform scale `s > 0` followed by a translation: `p -> s p + t`.
//! Composition of two of them is again one, which is what lets the compositor flatten a group tree into the ordinary
//! `Props` of each layer (scale about the layer's centre, top-left position) without a matrix per vertex.

use crate::{Edges, Props, Rect};
use serde::{Deserialize, Serialize};

/// How deep groups nest (the design's §3.1: reject deeper trees at creation). Nine: the day line is the deepest thing the home makes
/// (the home's root and its fade, the focus's window and its pose, the dock's pose, the first screen's part for the line, the line's
/// window, the scroll track's group and the hand-set offset under it). A level adds no storage and no work to a frame (a frame
/// evaluates the whole tree once; only the rare operation that preserves a world walks a chain), so the number only bounds what a
/// client may build.
pub const MAX_GROUP_DEPTH: usize = 9;
/// Groups a client may have at once (the resource ledger of §11.2). A group at rest costs the compositor nothing per frame (no frame, no
/// wake, no evaluation) and one that does not exist costs no work: a frame evaluates the groups there are, and the compositor's
/// maps are made for the most a shell holds, not for this number. A group is about 620 bytes in the compositor and 540 in the app. The
/// TV's shell holds 44 at rest on Home and 68 at most, Home with Quick Settings (2026-11-09), about a quarter of the number: it is the
/// room a shell and the one that replaces it have together (a shell is built before the one it replaces goes), with plenty to spare.
pub const MAX_GROUPS: usize = 256;
/// The smallest scale a pose or a binding may produce: a group scaled to nothing is hidden instead.
pub const MIN_SCALE: f32 = 1.0e-3;
/// The largest, the same limit the compositor puts on a layer's own scale.
pub const MAX_SCALE: f32 = 64.0;

/// Names a group. The app allocates ids from one counter that only rises, so an id is never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GroupId(pub u32);

/// Where a group is, relative to its parent (or to the screen, with no parent).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroupPose {
    /// Translation of the group's space, in its parent's space.
    pub x: f32,
    pub y: f32,
    /// Uniform scale about (`pivot_x`, `pivot_y`), which is a point of the group's own space (before the scale).
    pub scale: f32,
    pub pivot_x: f32,
    pub pivot_y: f32,
    /// Multiplies the opacity of everything below, in 0..=1.
    pub opacity: f32,
    pub visible: bool,
    /// Only the part of what is below that lies inside this rectangle, given in the group's own space and carried by
    /// the group's own pose, is drawn. `None` clips nothing.
    pub clip: Option<Rect>,
    /// How far in from each edge of `clip` what is below fades out (see [`Edges`]), in the group's own space: the widths are
    /// scaled by the group's transform, like the rectangle is. `Edges::NONE` is a hard clip. Without a clip it means nothing.
    pub feather: Edges,
}

impl GroupPose {
    /// No translation, no scale, opaque, visible, unclipped.
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        scale: 1.0,
        pivot_x: 0.0,
        pivot_y: 0.0,
        opacity: 1.0,
        visible: true,
        clip: None,
        feather: Edges::NONE,
    };

    pub const fn at(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            ..Self::IDENTITY
        }
    }

    /// The same pose with `rect` (in the group's own space) as the clip, fading out over `edges` towards its edges.
    pub const fn clip_feathered(self, rect: Rect, edges: Edges) -> Self {
        Self {
            clip: Some(rect),
            feather: edges,
            ..self
        }
    }

    /// Whether every number is finite and in its range (the check made when a pose comes over the wire).
    pub fn is_valid(&self) -> bool {
        let finite = [
            self.x,
            self.y,
            self.scale,
            self.pivot_x,
            self.pivot_y,
            self.opacity,
        ]
        .iter()
        .all(|v| v.is_finite());
        let clip_ok = self.clip.is_none_or(|c| {
            [c.x, c.y, c.w, c.h].iter().all(|v| v.is_finite()) && c.w >= 0.0 && c.h >= 0.0
        });
        finite
            && clip_ok
            && self.feather.is_valid()
            && (MIN_SCALE..=MAX_SCALE).contains(&self.scale)
            && (0.0..=1.0).contains(&self.opacity)
    }
}

impl Default for GroupPose {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// The pose properties a binding drives: a mask of [`PoseProperties::TRANSLATION`], `SCALE` and `OPACITY`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PoseProperties(pub u8);

impl PoseProperties {
    pub const TRANSLATION: Self = Self(1);
    pub const SCALE: Self = Self(2);
    pub const OPACITY: Self = Self(4);
    pub const ALL: Self = Self(7);
    pub const NONE: Self = Self(0);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn overlaps(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether only known bits are set.
    pub const fn is_valid(self) -> bool {
        self.0 & !Self::ALL.0 == 0
    }
}

/// Maps a track's value to a pose. The two `values` (finite and distinct) correspond to the poses `from` and `to`; the
/// properties in the mask are interpolated between them, and *extrapolated* beyond them for translation (so that an
/// overscrolled list keeps moving with the finger) while opacity stays in 0..=1 and scale in [`MIN_SCALE`]..=[`MAX_SCALE`].
/// A scroll binding maps the pixels `[0, 1]` to the translations `[0, -1]`; a dock binding maps progress `[0, 1]` to
/// the idle and awake poses. Only the masked properties of `from` and `to` are read.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoseBinding {
    pub values: [f32; 2],
    pub from: GroupPose,
    pub to: GroupPose,
    pub properties: PoseProperties,
}

impl PoseBinding {
    /// Why the binding cannot be used, if it cannot.
    pub fn check(&self) -> Result<(), &'static str> {
        if !self.properties.is_valid() || self.properties.is_empty() {
            return Err("a binding must drive at least one known property");
        }
        if !self.values.iter().all(|v| v.is_finite()) || self.values[0] == self.values[1] {
            return Err("a binding's two track values must be finite and distinct");
        }
        let finite = |p: &GroupPose| [p.x, p.y, p.scale, p.opacity].iter().all(|v| v.is_finite());
        if !finite(&self.from) || !finite(&self.to) {
            return Err("a binding's poses must be finite");
        }
        if self.properties.contains(PoseProperties::SCALE)
            && !(self.from.scale > 0.0 && self.to.scale > 0.0)
        {
            return Err("a binding's scales must be positive");
        }
        Ok(())
    }

    /// The fraction of the way from the first value to the second that `value` is (unclamped).
    pub fn fraction(&self, value: f32) -> f32 {
        ((f64::from(value) - f64::from(self.values[0]))
            / (f64::from(self.values[1]) - f64::from(self.values[0]))) as f32
    }

    /// Writes the properties this binding drives, at track value `value`, into `pose`.
    pub fn apply(&self, value: f32, pose: &mut GroupPose) {
        let u = self.fraction(value);
        let lerp = |a: f32, b: f32| a + (b - a) * u;
        if self.properties.contains(PoseProperties::TRANSLATION) {
            pose.x = lerp(self.from.x, self.to.x);
            pose.y = lerp(self.from.y, self.to.y);
        }
        if self.properties.contains(PoseProperties::SCALE) {
            pose.scale = lerp(self.from.scale, self.to.scale).clamp(MIN_SCALE, MAX_SCALE);
        }
        if self.properties.contains(PoseProperties::OPACITY) {
            pose.opacity = lerp(self.from.opacity, self.to.opacity).clamp(0.0, 1.0);
        }
    }
}

/// A similarity transform: `p -> scale * p + (tx, ty)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub scale: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        scale: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// The transform of a pose by itself: scale about the pivot, then translate.
    pub fn of_pose(pose: &GroupPose) -> Self {
        Self {
            scale: pose.scale,
            tx: pose.pivot_x * (1.0 - pose.scale) + pose.x,
            ty: pose.pivot_y * (1.0 - pose.scale) + pose.y,
        }
    }

    /// `self` after `inner`: the transform that applies `inner` first.
    pub fn after(&self, inner: &Transform) -> Transform {
        Transform {
            scale: self.scale * inner.scale,
            tx: self.scale * inner.tx + self.tx,
            ty: self.scale * inner.ty + self.ty,
        }
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.scale * x + self.tx, self.scale * y + self.ty)
    }

    pub fn apply_rect(&self, r: &Rect) -> Rect {
        Rect {
            x: self.scale * r.x + self.tx,
            y: self.scale * r.y + self.ty,
            w: self.scale * r.w,
            h: self.scale * r.h,
        }
    }

    /// The transform that undoes this one.
    pub fn inverse(&self) -> Transform {
        let s = 1.0 / self.scale;
        Transform {
            scale: s,
            tx: -self.tx * s,
            ty: -self.ty * s,
        }
    }
}

/// The intersection of two rectangles; an empty one (zero size) when they do not overlap.
pub fn intersect(a: &Rect, b: &Rect) -> Rect {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.w).min(b.x + b.w);
    let bottom = (a.y + a.h).min(b.y + b.h);
    if right > left && bottom > top {
        Rect {
            x: left,
            y: top,
            w: right - left,
            h: bottom - top,
        }
    } else {
        Rect {
            x: left,
            y: top,
            w: 0.0,
            h: 0.0,
        }
    }
}

/// The clip of something that has `own` as its own (in the same space as `parent`'s, the screen's), under an ancestry that
/// already clips to `parent`: the two rectangles' intersection, each edge of it the tighter of the two and keeping the feather
/// of the clip it came from (the wider, where both have it on one line). A fade of the clip whose edge is the looser one is not
/// multiplied in, so two fades that overlap on one edge are exact only while one of them is hard: what a feathered viewport
/// inside a plainly clipped page, or the other way round, is. An empty intersection has no feather.
fn compose_clips(
    (parent, parent_feather): (Option<Rect>, Edges),
    (own, own_feather): (Option<Rect>, Edges),
) -> (Option<Rect>, Edges) {
    let (p, o) = match (parent, own) {
        (parent, None) => return (parent, parent_feather),
        (None, Some(own)) => return (Some(own), own_feather),
        (Some(p), Some(o)) => (p, o),
    };
    let rect = intersect(&p, &o);
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return (Some(rect), Edges::NONE);
    }
    // `inward` is 1 where a larger coordinate is the tighter edge (left, top) and -1 where a smaller one is (right, bottom).
    let width =
        |parent_edge: f32, own_edge: f32, parent_width: f32, own_width: f32, inward: f32| {
            let own_is_tighter = (own_edge - parent_edge) * inward;
            if own_is_tighter > 0.0 {
                own_width
            } else if own_is_tighter < 0.0 {
                parent_width
            } else {
                parent_width.max(own_width)
            }
        };
    let feather = Edges {
        left: width(p.x, o.x, parent_feather.left, own_feather.left, 1.0),
        right: width(
            p.x + p.w,
            o.x + o.w,
            parent_feather.right,
            own_feather.right,
            -1.0,
        ),
        top: width(p.y, o.y, parent_feather.top, own_feather.top, 1.0),
        bottom: width(
            p.y + p.h,
            o.y + o.h,
            parent_feather.bottom,
            own_feather.bottom,
            -1.0,
        ),
    };
    (Some(rect), feather)
}

/// What a node's ancestors make of everything below it: the transform to screen space, the opacity, whether it is
/// shown, and the screen-space clip (the intersection of every ancestor's) with the feather of its edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct World {
    pub transform: Transform,
    pub opacity: f32,
    pub visible: bool,
    pub clip: Option<Rect>,
    /// How far in from each edge of `clip` what is below fades out, in screen pixels (`Edges::NONE` for a hard edge, and
    /// whenever there is no clip).
    pub feather: Edges,
}

impl World {
    /// The screen itself: what a node with no parent hangs under.
    pub const ROOT: Self = Self {
        transform: Transform::IDENTITY,
        opacity: 1.0,
        visible: true,
        clip: None,
        feather: Edges::NONE,
    };

    /// The world of a child group whose pose is `pose`.
    pub fn child(&self, pose: &GroupPose) -> World {
        let transform = self.transform.after(&Transform::of_pose(pose));
        let own = (
            pose.clip.map(|clip| transform.apply_rect(&clip)),
            pose.feather.scaled(transform.scale),
        );
        let (clip, feather) = compose_clips((self.clip, self.feather), own);
        World {
            transform,
            opacity: self.opacity * pose.opacity,
            visible: self.visible && pose.visible,
            clip,
            feather,
        }
    }

    /// The screen-space `Props` of a layer of `size` (width, height) whose own props, in this world's space, are
    /// `local`. `Props::scale` is about the layer's centre, so the layer's centre goes through the transform and the
    /// scale multiplies.
    pub fn props(&self, local: &Props, size: (f32, f32)) -> Props {
        let (w, h) = size;
        let t = &self.transform;
        let (cx, cy) = t.apply(local.x + w * 0.5, local.y + h * 0.5);
        let own = (
            local.clip.map(|clip| t.apply_rect(&clip)),
            local.feather.scaled(t.scale),
        );
        let (clip, feather) = compose_clips((self.clip, self.feather), own);
        Props {
            x: cx - w * 0.5,
            y: cy - h * 0.5,
            scale: local.scale * t.scale,
            opacity: local.opacity * self.opacity,
            clip,
            feather,
            visible: local.visible && self.visible,
        }
    }

    /// Whether the screen point (`px`, `py`) lies in a layer of `size` placed by `local` in this world: the same
    /// half-open rectangle `[x, x + w) x [y, y + h)` the compositor draws, after the same transform and clip, so that
    /// a point the compositor draws is a point the app hits. A hidden or transparent layer is not hit.
    pub fn hit(&self, local: &Props, size: (f32, f32), px: f32, py: f32) -> bool {
        let at = self.props(local, size);
        if !at.visible || at.opacity <= 0.0 || at.scale <= 0.0 {
            return false;
        }
        if let Some(clip) = at.clip {
            if !(px >= clip.x && px < clip.x + clip.w && py >= clip.y && py < clip.y + clip.h) {
                return false;
            }
        }
        let (w, h) = (size.0 * at.scale, size.1 * at.scale);
        let (cx, cy) = (at.x + size.0 * 0.5, at.y + size.1 * 0.5);
        let (left, top) = (cx - w * 0.5, cy - h * 0.5);
        px >= left && px < left + w && py >= top && py < top + h
    }
}

/// The pose a group must have under a new parent for its world to stay as it is (`Group::set_parent` with
/// `preserve_world`). `world` is the group's own evaluated world now (so its scale, translation and opacity are
/// those of the whole chain) and `new_parent` the world it is moving under; `pose` supplies the pivot, visibility and
/// clip, which stay as they are. Opacity is the ratio of the two, capped to 1 (a parent that is fully transparent
/// leaves the opacity as it was: the group is not visible either way).
pub fn pose_preserving_world(world: &World, new_parent: &World, pose: &GroupPose) -> GroupPose {
    let local = new_parent.transform.inverse().after(&world.transform);
    let opacity = if new_parent.opacity > 1.0e-6 {
        (world.opacity / new_parent.opacity).clamp(0.0, 1.0)
    } else {
        pose.opacity
    };
    GroupPose {
        scale: local.scale.clamp(MIN_SCALE, MAX_SCALE),
        x: local.tx - pose.pivot_x * (1.0 - local.scale),
        y: local.ty - pose.pivot_y * (1.0 - local.scale),
        opacity,
        ..*pose
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1.0e-3
    }

    #[test]
    fn a_pose_scales_about_its_pivot_and_then_translates() {
        let pose = GroupPose {
            scale: 2.0,
            pivot_x: 10.0,
            pivot_y: 20.0,
            x: 5.0,
            y: -5.0,
            ..GroupPose::IDENTITY
        };
        let t = Transform::of_pose(&pose);
        // The pivot does not move under the scale, only under the translation.
        assert_eq!(t.apply(10.0, 20.0), (15.0, 15.0));
        // A point one unit right of the pivot ends two units right of the moved pivot.
        assert_eq!(t.apply(11.0, 20.0), (17.0, 15.0));
    }

    #[test]
    fn composition_applies_the_inner_transform_first_and_inverts() {
        let outer = Transform {
            scale: 2.0,
            tx: 10.0,
            ty: 0.0,
        };
        let inner = Transform {
            scale: 0.5,
            tx: 4.0,
            ty: 6.0,
        };
        let both = outer.after(&inner);
        let (x, y) = inner.apply(3.0, 3.0);
        assert_eq!(both.apply(3.0, 3.0), outer.apply(x, y));
        let back = both.inverse().after(&both);
        assert!(near(back.scale, 1.0) && near(back.tx, 0.0) && near(back.ty, 0.0));
    }

    #[test]
    fn a_layer_in_a_scaled_group_scales_about_the_groups_pivot_not_its_own_centre() {
        // A 100x50 layer at (0, 0) in a group scaled 2x about (0, 0): its centre (50, 25) goes to (100, 50), and
        // its top-left is where a 2x layer centred there starts.
        let world = ROOT_WITH_SCALE_TWO();
        let props = world.props(&Props::at(0.0, 0.0), (100.0, 50.0));
        assert_eq!(props.scale, 2.0);
        assert!(near(props.x + 50.0, 100.0) && near(props.y + 25.0, 50.0));
    }

    #[allow(non_snake_case)]
    fn ROOT_WITH_SCALE_TWO() -> World {
        World::ROOT.child(&GroupPose {
            scale: 2.0,
            ..GroupPose::IDENTITY
        })
    }

    #[test]
    fn nested_groups_compose_scale_translation_opacity_and_visibility() {
        let a = World::ROOT.child(&GroupPose {
            x: 10.0,
            opacity: 0.5,
            ..GroupPose::IDENTITY
        });
        let b = a.child(&GroupPose {
            scale: 2.0,
            opacity: 0.5,
            ..GroupPose::IDENTITY
        });
        assert!(near(b.opacity, 0.25));
        let props = b.props(&Props::at(0.0, 0.0), (10.0, 10.0));
        // Centre (5, 5) -> scale 2 -> (10, 10) -> + 10 -> (20, 10).
        assert!(near(props.x + 5.0, 20.0) && near(props.y + 5.0, 10.0));
        let hidden = a.child(&GroupPose {
            visible: false,
            ..GroupPose::IDENTITY
        });
        assert!(!hidden.props(&Props::at(0.0, 0.0), (1.0, 1.0)).visible);
    }

    #[test]
    fn clips_are_local_to_their_owner_and_intersect_in_screen_space() {
        let outer = World::ROOT.child(&GroupPose {
            x: 100.0,
            clip: Some(Rect {
                x: 0.0,
                y: 0.0,
                w: 50.0,
                h: 50.0,
            }),
            ..GroupPose::IDENTITY
        });
        // The outer clip is carried by the outer pose: (100, 0) .. (150, 50) on the screen.
        assert_eq!(
            outer.clip,
            Some(Rect {
                x: 100.0,
                y: 0.0,
                w: 50.0,
                h: 50.0
            })
        );
        let inner = outer.child(&GroupPose {
            scale: 2.0,
            clip: Some(Rect {
                x: 10.0,
                y: 0.0,
                w: 100.0,
                h: 10.0,
            }),
            ..GroupPose::IDENTITY
        });
        // The inner clip is (10, 0, 100, 10) in a space scaled 2x and moved to x = 100: (120, 0, 200, 20) on the
        // screen, cut to the outer one's right edge at 150.
        assert_eq!(
            inner.clip,
            Some(Rect {
                x: 120.0,
                y: 0.0,
                w: 30.0,
                h: 20.0
            })
        );
        // A layer's own clip joins them; one that lies outside leaves nothing.
        let layer = Props {
            clip: Some(Rect {
                x: 500.0,
                y: 500.0,
                w: 10.0,
                h: 10.0,
            }),
            ..Props::at(0.0, 0.0)
        };
        let props = inner.props(&layer, (1.0, 1.0));
        assert_eq!(props.clip.map(|c| (c.w, c.h)), Some((0.0, 0.0)));
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn a_feather_goes_with_its_clip_and_is_scaled_by_the_transform_as_the_clip_is() {
        // A viewport of 424 x 160 whose edges fade over 12, in a group the home scales by 1.3 about the screen's origin.
        let hero = World::ROOT.child(&GroupPose {
            scale: 1.3,
            ..GroupPose::IDENTITY
        });
        let pager = hero.child(
            &GroupPose::at(310.0, 30.0)
                .clip_feathered(rect(0.0, 0.0, 424.0, 160.0), Edges::horizontal(12.0)),
        );
        let clip = pager.clip.expect("the pager clips");
        assert!(near(clip.w, 424.0 * 1.3) && near(clip.x, 310.0 * 1.3));
        assert!(near(pager.feather.left, 15.6) && near(pager.feather.right, 15.6));
        assert_eq!((pager.feather.top, pager.feather.bottom), (0.0, 0.0));
        // What hangs under it is drawn within the same clip and the same fade.
        let card = pager.props(&Props::at(20.0, 10.0), (384.0, 116.0));
        assert_eq!(card.clip, pager.clip);
        assert_eq!(card.feather, pager.feather);
        // A layer's own clip is carried by the group it is in, feather and all: a 10 pixel fade at the layer's own scale.
        let own = pager.props(
            &Props::at(0.0, 0.0).clip_feathered(rect(0.0, 0.0, 100.0, 100.0), Edges::all(10.0)),
            (100.0, 100.0),
        );
        assert!(near(own.feather.top, 13.0), "{:?}", own.feather);
        // A feather with no clip is nothing, in a pose and in a layer.
        let stray = World::ROOT.child(&GroupPose {
            feather: Edges::all(5.0),
            ..GroupPose::IDENTITY
        });
        assert_eq!((stray.clip, stray.feather), (None, Edges::NONE));
        let layer = Props {
            feather: Edges::all(5.0),
            ..Props::at(0.0, 0.0)
        };
        let props = World::ROOT.props(&layer, (10.0, 10.0));
        assert_eq!((props.clip, props.feather), (None, Edges::NONE));
    }

    #[test]
    fn clips_that_are_one_inside_the_other_keep_the_feather_of_the_edge_that_is_the_tighter() {
        let at = |clip, feather| GroupPose::IDENTITY.clip_feathered(clip, feather);
        let page = World::ROOT.child(&GroupPose {
            clip: Some(rect(0.0, 0.0, 960.0, 480.0)),
            ..GroupPose::IDENTITY
        });
        // A feathered viewport inside a plainly clipped page is the viewport's, whole.
        let viewport = page.child(&at(
            rect(100.0, 50.0, 400.0, 300.0),
            Edges::new(20.0, 30.0, 40.0, 50.0),
        ));
        assert_eq!(viewport.clip, Some(rect(100.0, 50.0, 400.0, 300.0)));
        assert_eq!(viewport.feather, Edges::new(20.0, 30.0, 40.0, 50.0));
        // A plain clip round it that is tighter on one edge takes that edge hard: the fade the viewport had there is cut off.
        let cut = viewport.child(&GroupPose {
            clip: Some(rect(120.0, 0.0, 1000.0, 1000.0)),
            ..GroupPose::IDENTITY
        });
        assert_eq!(cut.clip, Some(rect(120.0, 50.0, 380.0, 300.0)));
        assert_eq!(cut.feather, Edges::new(0.0, 30.0, 40.0, 50.0));
        // The other way round, a looser plain clip leaves the feathered one as it is, and an edge on the same line keeps the
        // wider of the two fades.
        let same = viewport.child(&at(
            rect(100.0, 50.0, 400.0, 300.0),
            Edges::new(5.0, 50.0, 0.0, 10.0),
        ));
        assert_eq!(same.clip, viewport.clip);
        assert_eq!(same.feather, Edges::new(20.0, 50.0, 40.0, 50.0));
        // Clips that do not meet leave nothing, and nothing to fade.
        let apart = viewport.child(&at(rect(600.0, 0.0, 100.0, 100.0), Edges::all(10.0)));
        let c = apart.clip.expect("an empty clip");
        assert_eq!((c.w, c.h), (0.0, 0.0));
        assert_eq!(apart.feather, Edges::NONE);
    }

    #[test]
    fn a_pose_is_valid_only_with_a_feather_of_lengths() {
        let pose = |feather| GroupPose {
            clip: Some(rect(0.0, 0.0, 10.0, 10.0)),
            feather,
            ..GroupPose::IDENTITY
        };
        assert!(pose(Edges::NONE).is_valid() && pose(Edges::all(12.0)).is_valid());
        assert!(!pose(Edges::new(-1.0, 0.0, 0.0, 0.0)).is_valid());
        assert!(!pose(Edges::new(0.0, f32::NAN, 0.0, 0.0)).is_valid());
        assert!(!pose(Edges::new(0.0, 0.0, f32::INFINITY, 0.0)).is_valid());
    }

    #[test]
    fn hits_use_the_half_open_transformed_rectangle() {
        let world = World::ROOT.child(&GroupPose {
            x: 10.0,
            ..GroupPose::IDENTITY
        });
        let local = Props::at(0.0, 0.0);
        assert!(world.hit(&local, (20.0, 20.0), 10.0, 10.0));
        assert!(world.hit(&local, (20.0, 20.0), 29.9, 19.9));
        assert!(
            !world.hit(&local, (20.0, 20.0), 30.0, 15.0),
            "the right edge is outside"
        );
        assert!(
            !world.hit(&local, (20.0, 20.0), 15.0, 20.0),
            "so is the bottom edge"
        );
        assert!(!world.hit(&local, (20.0, 20.0), 9.9, 15.0));
        let faded = World::ROOT.child(&GroupPose {
            opacity: 0.0,
            ..GroupPose::IDENTITY
        });
        assert!(
            !faded.hit(&local, (20.0, 20.0), 5.0, 5.0),
            "transparent is not hit"
        );
        let clipped = World::ROOT.child(&GroupPose {
            clip: Some(Rect {
                x: 0.0,
                y: 0.0,
                w: 5.0,
                h: 5.0,
            }),
            ..GroupPose::IDENTITY
        });
        assert!(clipped.hit(&local, (20.0, 20.0), 4.0, 4.0));
        assert!(!clipped.hit(&local, (20.0, 20.0), 6.0, 6.0));
    }

    #[test]
    fn a_binding_interpolates_translation_with_extrapolation_but_clamps_opacity_and_scale() {
        let binding = PoseBinding {
            values: [0.0, 1.0],
            from: GroupPose {
                opacity: 0.0,
                scale: 1.0,
                ..GroupPose::at(0.0, 0.0)
            },
            to: GroupPose {
                opacity: 1.0,
                scale: 0.5,
                ..GroupPose::at(0.0, -1.0)
            },
            properties: PoseProperties::ALL,
        };
        binding.check().unwrap();
        let mut pose = GroupPose::IDENTITY;
        binding.apply(0.5, &mut pose);
        assert!(near(pose.y, -0.5) && near(pose.opacity, 0.5) && near(pose.scale, 0.75));
        // Past the end the translation keeps going (overscroll) and the rest saturate.
        binding.apply(3.0, &mut pose);
        assert!(near(pose.y, -3.0) && pose.opacity == 1.0 && pose.scale >= MIN_SCALE);
        binding.apply(-3.0, &mut pose);
        assert!(near(pose.y, 3.0) && pose.opacity == 0.0 && near(pose.scale, 2.5));
        // Only the masked properties are written.
        let only_opacity = PoseBinding {
            properties: PoseProperties::OPACITY,
            ..binding
        };
        let mut pose = GroupPose::at(7.0, 8.0);
        only_opacity.apply(1.0, &mut pose);
        assert_eq!((pose.x, pose.y), (7.0, 8.0));
    }

    #[test]
    fn a_binding_that_cannot_work_is_refused() {
        let ok = PoseBinding {
            values: [0.0, 1.0],
            from: GroupPose::IDENTITY,
            to: GroupPose::IDENTITY,
            properties: PoseProperties::ALL,
        };
        assert!(ok.check().is_ok());
        assert!(
            PoseBinding {
                values: [1.0, 1.0],
                ..ok
            }
            .check()
            .is_err()
        );
        assert!(
            PoseBinding {
                values: [0.0, f32::NAN],
                ..ok
            }
            .check()
            .is_err()
        );
        assert!(
            PoseBinding {
                properties: PoseProperties::NONE,
                ..ok
            }
            .check()
            .is_err()
        );
        assert!(
            PoseBinding {
                properties: PoseProperties(8),
                ..ok
            }
            .check()
            .is_err()
        );
        let bad_scale = PoseBinding {
            to: GroupPose {
                scale: 0.0,
                ..GroupPose::IDENTITY
            },
            ..ok
        };
        assert!(bad_scale.check().is_err());
    }

    #[test]
    fn reparenting_with_preserve_world_keeps_the_evaluated_pose() {
        let old_parent = World::ROOT.child(&GroupPose {
            x: 30.0,
            scale: 2.0,
            opacity: 0.8,
            ..GroupPose::IDENTITY
        });
        let pose = GroupPose {
            x: 4.0,
            y: 6.0,
            scale: 0.5,
            pivot_x: 8.0,
            pivot_y: 2.0,
            opacity: 0.5,
            ..GroupPose::IDENTITY
        };
        let world = old_parent.child(&pose);
        let new_parent = World::ROOT.child(&GroupPose {
            x: -10.0,
            y: 10.0,
            scale: 4.0,
            opacity: 0.5,
            ..GroupPose::IDENTITY
        });
        let moved = pose_preserving_world(&world, &new_parent, &pose);
        let again = new_parent.child(&moved);
        assert!(near(again.transform.scale, world.transform.scale));
        assert!(near(again.transform.tx, world.transform.tx));
        assert!(near(again.transform.ty, world.transform.ty));
        assert!(near(again.opacity, world.opacity));
        // A child layer lands on the same pixels either way.
        let layer = Props::at(3.0, 5.0);
        let (a, b) = (
            world.props(&layer, (10.0, 10.0)),
            again.props(&layer, (10.0, 10.0)),
        );
        assert!(near(a.x, b.x) && near(a.y, b.y) && near(a.scale, b.scale));
    }

    #[test]
    fn poses_are_validated() {
        assert!(GroupPose::IDENTITY.is_valid());
        assert!(
            !GroupPose {
                x: f32::INFINITY,
                ..GroupPose::IDENTITY
            }
            .is_valid()
        );
        assert!(
            !GroupPose {
                scale: 0.0,
                ..GroupPose::IDENTITY
            }
            .is_valid()
        );
        assert!(
            !GroupPose {
                opacity: 1.5,
                ..GroupPose::IDENTITY
            }
            .is_valid()
        );
        assert!(
            !GroupPose {
                scale: 100.0,
                ..GroupPose::IDENTITY
            }
            .is_valid()
        );
    }
}
