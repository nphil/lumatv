//! Prepared backdrops: the compositor's side of `compositor_protocol::resources` (design §3.1 item 3 and §3.2).
//!
//! This file holds the interface between the scene and the renderer: what the renderer asks of the scene to make a plate
//! and to place one. The set store, the plate capture and the composition are the renderer's.

use crate::scene::Drawn;
use compositor_protocol::group::World;
use compositor_protocol::{GroupId, LayerId};

/// What the renderer asks of the scene to make and to place a plate. Implemented by `scene::Scene`.
pub trait Source {
    /// The layers under `group` (and under the groups below it), in drawing order, as they are drawn with the pose of
    /// `group` itself and of its ancestors taken as the identity: group-local geometry. The poses and bindings of the
    /// groups *below* `group` apply as they are at rest (`Drawn::rest`'s view), so that a plate made while something moves
    /// is the one made when it stops. `props` and `rest` of each entry are the same, and the versions those of the layers.
    fn plate_layers(&self, group: GroupId, out: &mut Vec<Drawn>);

    /// One layer as it is drawn now (the wallpaper under a prepared set), whether or not it is shown; `None` when there
    /// is no such layer.
    fn layer(&self, id: LayerId) -> Option<Drawn>;

    /// The evaluated world of `group` now (its own pose, its bindings at the frame's time, and its ancestors'): the
    /// transform, opacity, visibility and screen clip a plate made from it is shown with. `None` when there is no such
    /// group.
    fn group_world(&self, group: GroupId) -> Option<World>;

    /// Whether a motion track is being driven by a finger or is settling. The renderer makes no plate capture then: the
    /// design's "zero captures attributable to motion".
    fn moving(&self) -> bool;
}

/// A scene with no groups and no layers to make plates of: what a frame is drawn with when no prepared set is in play, as
/// the warm-up's is.
pub struct NoGroups;

impl Source for NoGroups {
    fn plate_layers(&self, _: GroupId, out: &mut Vec<Drawn>) {
        out.clear();
    }

    fn layer(&self, _: LayerId) -> Option<Drawn> {
        None
    }

    fn group_world(&self, _: GroupId) -> Option<World> {
        None
    }

    fn moving(&self) -> bool {
        false
    }
}
