//! The app's side of drawing and the public Lucent toolkit: layers it owns on the compositor, native groups and shared motion
//! tracks, prepared backdrops, glyph atlases and runs, text rastered into them, and the nonblocking, bounded writer that gets
//! it all to the compositor; the design language's tokens and material, the input model and the components every screen is
//! built from.
//!
//! The entry point is [`Scene`]. Everything the app creates is an owned handle (dropping it deletes the thing on the
//! compositor, in an order the compositor accepts), identities in this process are small copyable index-plus-generation
//! handles that are never reused, and nothing waits for the compositor: [`Scene::try_commit`] answers `WouldBlock` at once
//! when the compositor has not caught up, and the host sleeps on [`Scene::writer_fd`] until it can write again.

pub mod atlas;
pub mod backdrop;
pub mod components;
pub mod context;
pub mod error;
pub mod geometry;
pub mod group;
mod handle;
pub mod input;
pub mod kept;
pub mod layout;
pub mod limits;
pub mod lucent;
pub mod motion;
#[cfg(any(test, feature = "testing"))]
pub mod reference;
pub mod scene;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod text;
mod writer;

pub use atlas::{
    Atlas, AtlasId, AtlasView, GlyphId, GlyphRun, Palette, PlacedGlyph, Sprite, one_ink,
};
pub use backdrop::{
    BackdropGeneration, BackdropId, BackdropLease, BackdropSpec, ContentGeneration,
    PreparedBackdrop,
};
pub use context::{Look, Ui};
pub use error::{CommitTag, Kind, Limit, SubmitError, UiError};
pub use group::{Group, GroupId, GroupPose, PoseBinding, PoseProperties};
pub use kept::KeptImage;
pub use motion::{
    CancelPolicy, DEFAULT_PRESENT_LEAD_NS, DrivePhase, DriveSample, Motion, MotionBounds, MotionId,
    MotionToken, ScrollToken, Settle, SpringToken, Units,
};
pub use scene::{
    GlassLayer, ImageLayer, Layer, LiveLayer, MeshLayer, Scene, SceneOptions, ShapeLayer, Wake,
    WallpaperLayer, Watch, Welcome, monotonic_ns,
};
pub use text::{Canvas, Extent, Fonts, GlyphMetrics, Type, Weight};
