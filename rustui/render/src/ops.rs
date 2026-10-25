//! What the command thread hands the render thread: batches of scene changes, each visible atomically.

use crate::geometry::PixRect;
use crate::shm::ImageFile;
use compositor_protocol::frames::Spec;
use compositor_protocol::{
    AtlasId, AtlasRect, BackdropId, CancelPolicy, CaptureSpec, Content, DriveSample, Ease, Glyph,
    GroupId, GroupPose, LayerId, MeshData, MotionBounds, MotionId, PoseBinding, PoseProperties,
    Props, Settle, SmoothFirst, SourceId,
};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::Arc;

/// One validated change to the scene. Everything in an `Op` has been bounds-checked by the session that made it:
/// sizes are within limits, numbers are finite, damage lies inside its layer and the layer it names exists.
#[derive(Debug)]
pub enum Op {
    Create {
        id: LayerId,
        width: u32,
        height: u32,
        z: i32,
        content: Content,
        props: Props,
        /// The mapped pixels of an image layer, or of a wallpaper whose kind uses a picture.
        image: Option<Arc<ImageFile>>,
        /// The decoded geometry of a mesh layer: present exactly when `content` is `Content::Mesh`.
        mesh: Option<Arc<MeshData>>,
    },
    Damage {
        id: LayerId,
        rect: PixRect,
    },
    /// New look for a shape, glass, wallpaper or mesh layer; `content` is of the layer's own kind (the session
    /// checked), and for a wallpaper it still uses a picture exactly when the old one did.
    Restyle {
        id: LayerId,
        content: Content,
    },
    /// A tap on a `Liquid` wallpaper, in layer pixels.
    Ripple {
        id: LayerId,
        x: f32,
        y: f32,
    },
    Set {
        id: LayerId,
        props: Props,
    },
    Animate {
        id: LayerId,
        to: Props,
        duration_ms: u32,
        ease: Ease,
    },
    Delete {
        id: LayerId,
    },
    /// Deletes every layer, lets go of the GPU clock the client held and puts "Smooth first" (the glass ideas and the ladder)
    /// back to its default (`SmoothFirst::DEFAULT`): the client that owned them went away.
    Clear,
    /// The client holds the GPU at this clock (kHz, one the session checked against the profile), or gives it back to
    /// the compositor's own policy (`None`). Not a change to the scene; `gpu_clock::GpuClock` carries it out.
    HoldGpuClock(Option<u32>),
    /// The glass ideas that are on (`compositor_protocol::ideas`), all of them. Not a change to the scene.
    Ideas(u32),
    /// The "Smooth first" level (`compositor_protocol::SmoothFirst`): the glass ideas it draws with, and whether the ladder runs.
    /// Not a change to the scene.
    SmoothFirst(SmoothFirst),
    /// Whether anyone can see the screen (`Request::Display`). Not a change to the scene; the render thread draws no frames
    /// while it is dark.
    Display(bool),

    // --- Groups. The session checked that every id exists, the tree stays acyclic and at most `MAX_GROUP_DEPTH` deep,
    // and every number is finite and in range. ---
    GroupCreate {
        id: GroupId,
        parent: Option<GroupId>,
        pose: GroupPose,
    },
    GroupSet {
        id: GroupId,
        pose: GroupPose,
    },
    GroupParent {
        id: GroupId,
        parent: Option<GroupId>,
        preserve_world: bool,
    },
    GroupBind {
        id: GroupId,
        motion: MotionId,
        binding: PoseBinding,
    },
    GroupUnbind {
        id: GroupId,
        properties: PoseProperties,
    },
    GroupDelete {
        id: GroupId,
    },
    Attach {
        id: LayerId,
        group: Option<GroupId>,
        local: Props,
    },

    // --- Motion tracks. Drive, settle and cancel are not part of a frame's content: they may arrive in a transaction of
    // their own, without a tag (`Op::wants_frame`). ---
    MotionCreate {
        id: MotionId,
        value: f32,
        bounds: MotionBounds,
    },
    MotionDelete {
        id: MotionId,
    },
    MotionDrive {
        id: MotionId,
        sample: DriveSample,
    },
    MotionSettle {
        id: MotionId,
        target: Option<f32>,
        velocity: Option<f32>,
        settle: Settle,
    },
    MotionCancel {
        id: MotionId,
        policy: CancelPolicy,
    },

    // --- Prepared backdrops. ---
    BackdropCreate {
        id: BackdropId,
        spec: CaptureSpec,
    },
    BackdropRefresh {
        id: BackdropId,
        content: u64,
    },
    BackdropFreeze {
        id: BackdropId,
    },
    BackdropRelease {
        id: BackdropId,
    },
    BackdropDelete {
        id: BackdropId,
    },

    // --- Atlases and glyph runs. ---
    AtlasCreate {
        id: AtlasId,
        width: u32,
        height: u32,
    },
    /// `path` is the staging slot's file (`resources::staging_path`, in the session's shm directory), which the render
    /// thread reads when it makes the upload: `rect.bytes()` bytes, rows packed at the rectangle's width.
    AtlasUpload {
        id: AtlasId,
        slot: u8,
        generation: u64,
        rect: AtlasRect,
        path: PathBuf,
    },
    AtlasDelete {
        id: AtlasId,
    },
    SetGlyphs {
        id: LayerId,
        after: u64,
        glyphs: Vec<Glyph>,
    },

    // --- Live sources (the frame endpoint: `compositor_protocol::frames`). A producer's, not the app's: they arrive in
    // transactions of their own (`Transaction::PRODUCER`), take effect at once and are not part of any commit. ---
    /// Makes (or replaces) the source, importing `buffers` into the GPU. The descriptors are the compositor's from here: the
    /// importer keeps them as long as the GPU may read the memory, and closes them when the source goes.
    LiveAttach {
        source: SourceId,
        spec: Spec,
        buffers: Vec<OwnedFd>,
    },
    /// The source's newest picture is in `buffer`; the one shown until now is released.
    LiveFrame {
        source: SourceId,
        buffer: u8,
    },
    /// The source shows nothing until its next picture; the picture shown is released.
    LiveClear {
        source: SourceId,
    },
    /// The source is gone: the GPU lets go of its buffers.
    LiveDetach {
        source: SourceId,
    },
}

impl Op {
    /// Whether it changes what the scene is made of or how it moves (a layer made, restyled, set, animated or deleted),
    /// rather than refreshing a layer's pixels or a setting outside the scene. What frames cost can change with it on
    /// the very next frame: layers that an animation it starts brings into view (`pacer::Pacer::reshaped`).
    pub fn reshapes(&self) -> bool {
        match self {
            Op::Create { .. }
            | Op::Restyle { .. }
            | Op::Set { .. }
            | Op::Animate { .. }
            | Op::Delete { .. }
            | Op::Clear
            | Op::GroupCreate { .. }
            | Op::GroupSet { .. }
            | Op::GroupParent { .. }
            | Op::GroupBind { .. }
            | Op::GroupUnbind { .. }
            | Op::GroupDelete { .. }
            | Op::Attach { .. }
            | Op::MotionCreate { .. }
            | Op::MotionDelete { .. }
            | Op::MotionDrive { .. }
            | Op::MotionSettle { .. }
            | Op::MotionCancel { .. } => true,
            Op::Damage { .. }
            | Op::Ripple { .. }
            | Op::HoldGpuClock(_)
            | Op::Ideas(_)
            | Op::SmoothFirst(_)
            | Op::Display(_)
            | Op::BackdropCreate { .. }
            | Op::BackdropRefresh { .. }
            | Op::BackdropFreeze { .. }
            | Op::BackdropRelease { .. }
            | Op::BackdropDelete { .. }
            | Op::AtlasCreate { .. }
            | Op::AtlasUpload { .. }
            | Op::AtlasDelete { .. }
            | Op::SetGlyphs { .. }
            | Op::LiveAttach { .. }
            | Op::LiveFrame { .. }
            | Op::LiveClear { .. }
            | Op::LiveDetach { .. } => false,
        }
    }

    /// Whether the op asks for a frame by itself even in a transaction with no commit tag: a pointer sample, a settle or a
    /// cancel on a motion track changes what the next frame shows (`engine::needs_frame`), and so does a producer's picture
    /// (or the end of one).
    pub fn wants_frame(&self) -> bool {
        matches!(
            self,
            Op::MotionDrive { .. }
                | Op::MotionSettle { .. }
                | Op::MotionCancel { .. }
                | Op::LiveAttach { .. }
                | Op::LiveFrame { .. }
                | Op::LiveClear { .. }
                | Op::LiveDetach { .. }
        )
    }

    /// Whether it only changes what the scene is made of or what a glyph atlas holds: a layer, a group or an atlas made, set,
    /// restyled, attached, damaged or deleted. Whether such a commit shows anything new is for the scene it makes to say
    /// (`engine::shows_nothing`): its effect is in the layers that are drawn, which do not change when the layers it touches
    /// are hidden. What is left out can change what the screen shows without a layer changing (the glass ideas, a prepared
    /// backdrop, a motion track, a ripple, an animation that starts, the client going away, a glyph run: `SetGlyphs` is
    /// the engine's own case, because a swapped-in list does not change the layer the scene knows).
    pub fn is_scene_work(&self) -> bool {
        matches!(
            self,
            Op::Create { .. }
                | Op::Damage { .. }
                | Op::Restyle { .. }
                | Op::Set { .. }
                | Op::Delete { .. }
                | Op::GroupCreate { .. }
                | Op::GroupSet { .. }
                | Op::GroupParent { .. }
                | Op::GroupDelete { .. }
                | Op::Attach { .. }
                | Op::AtlasCreate { .. }
                | Op::AtlasUpload { .. }
                | Op::AtlasDelete { .. }
        )
    }
}

impl Transaction {
    /// The `client` of what a producer of frames sent (the app's connections are numbered from 1).
    pub const PRODUCER: u64 = 0;

    /// The client going away: what `Session::goodbye` makes. It cancels the commits before it, which nobody is left to
    /// show.
    pub fn is_goodbye(&self) -> bool {
        self.tag.is_none() && self.ops.iter().any(|op| matches!(op, Op::Clear))
    }

    /// Whether it asks for a frame by itself: a commit that can change what the screen shows, or a pointer sample, settle or
    /// cancel on a motion track. A commit of nothing but glyph-atlas work (a page made, chunks of it) shows nothing of its own: the
    /// chunks land in the housekeeping between frames, and the frame that first draws from them is a later commit's, or the one a
    /// reveal that waited for them asks for. Drawing a frame for each would make a page's landing cost a refresh a round trip (the
    /// start's warm is some thirty of them: 16 ms each, past the time the app gives it, and the labels it then mounts are pictures).
    pub fn asks_for_frame(&self) -> bool {
        (self.tag.is_some() && !self.only_atlas_work()) || self.ops.iter().any(Op::wants_frame)
    }

    /// Nothing but a glyph atlas made or chunks of one: what shows nothing by itself. An empty commit is not that (a client may
    /// commit nothing to be told a frame went by).
    fn only_atlas_work(&self) -> bool {
        !self.ops.is_empty()
            && self
                .ops
                .iter()
                .all(|op| matches!(op, Op::AtlasCreate { .. } | Op::AtlasUpload { .. }))
    }
}

/// The requests of one commit.
#[derive(Debug)]
pub struct Transaction {
    /// Which connection made it, so events about it never reach a later client. [`Transaction::PRODUCER`] for what a
    /// producer of frames sent: it is no client of the app's protocol, and the render thread's own events (which go to
    /// the client that spoke last) never go to it.
    pub client: u64,
    /// The commit's tag. `None` for a change that needs no frame of its own (the client going away).
    pub tag: Option<u64>,
    pub ops: Vec<Op>,
    /// When the command thread had it, stamped as it enters the inbox: how long a commit's content has been waiting
    /// is how a frame that gives up a refresh can say whether it could have made it (`pacer::Plan::avoidable`).
    pub received_ns: u64,
}
