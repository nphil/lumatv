//! What the scene refuses, and why a commit was not taken.
//!
//! [`UiError`] is what a call on a handle says when it cannot be honoured. The scene checks every protocol ceiling and every
//! number itself, at creation, so the compositor is never sent a request it would refuse; a refusal there is a bug in the app,
//! and a [`UiError`] is how the app hears of it. [`SubmitError`] is the answer of `Scene::try_commit`: the one that is not a
//! failure is `WouldBlock`.

use compositor_protocol::group::{MAX_GROUP_DEPTH, MAX_GROUPS};
use compositor_protocol::motion::MAX_MOTIONS;
use compositor_protocol::resources::{
    MAX_ATLAS_BYTES, MAX_ATLAS_SIDE, MAX_ATLASES, MAX_BACKDROPS, MAX_PLATES, MAX_RUN_GLYPHS,
};
use std::fmt;
use std::io;

/// A ceiling of the protocol (the design's resource ledger, `docs/g2-shell-design.md` section 11.2) that the scene enforces on
/// the app's side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// Groups alive at once, counting those that wait for what hangs under them to go.
    Groups,
    /// How deep groups nest.
    GroupDepth,
    /// Motion tracks alive at once.
    Motions,
    /// Prepared backdrop sets alive at once.
    Backdrops,
    /// Plates in one prepared set.
    Plates,
    /// Glyph atlases alive at once.
    Atlases,
    /// Bytes all atlases hold together.
    AtlasBytes,
    /// The longest side of an atlas.
    AtlasSide,
    /// Glyph quads in one run.
    RunGlyphs,
}

impl Limit {
    /// The most there may be (bytes for [`Limit::AtlasBytes`], pixels for [`Limit::AtlasSide`]).
    pub const fn max(self) -> u64 {
        match self {
            Limit::Groups => MAX_GROUPS as u64,
            Limit::GroupDepth => MAX_GROUP_DEPTH as u64,
            Limit::Motions => MAX_MOTIONS as u64,
            Limit::Backdrops => MAX_BACKDROPS as u64,
            Limit::Plates => MAX_PLATES as u64,
            Limit::Atlases => MAX_ATLASES as u64,
            Limit::AtlasBytes => MAX_ATLAS_BYTES,
            Limit::AtlasSide => MAX_ATLAS_SIDE as u64,
            Limit::RunGlyphs => MAX_RUN_GLYPHS as u64,
        }
    }

    pub const fn what(self) -> &'static str {
        match self {
            Limit::Groups => "groups",
            Limit::GroupDepth => "levels of groups",
            Limit::Motions => "motion tracks",
            Limit::Backdrops => "prepared backdrops",
            Limit::Plates => "plates in a prepared backdrop",
            Limit::Atlases => "glyph atlases",
            Limit::AtlasBytes => "bytes of glyph atlas",
            Limit::AtlasSide => "pixels on an atlas side",
            Limit::RunGlyphs => "glyphs in a run",
        }
    }
}

/// What a handle named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Group,
    Motion,
    Backdrop,
    Atlas,
    Layer,
}

impl Kind {
    fn what(self) -> &'static str {
        match self {
            Kind::Group => "group",
            Kind::Motion => "motion track",
            Kind::Backdrop => "prepared backdrop",
            Kind::Atlas => "glyph atlas",
            Kind::Layer => "layer",
        }
    }
}

/// A call on the scene that cannot be honoured. Nothing was staged for it.
#[derive(Debug)]
pub enum UiError {
    /// A ceiling would be passed.
    Limit(Limit),
    /// The handle names something that has been dropped (or belongs to another scene).
    Stale(Kind),
    /// A value the protocol would refuse (not a number, out of range, empty): what, in words.
    Invalid(&'static str),
    /// The move would put a group under itself or under one of its own descendants.
    Cycle,
    /// A glyph of the run has been rastered and packed but its pixels have not been handed to the compositor yet (see
    /// `Atlas::pump`); a run is only ever shown whole.
    GlyphNotUploaded,
    /// The atlas has no room left for the glyph; make a new atlas (and drop this one) at a settled boundary.
    AtlasFull,
    /// A file the scene needs (a staging slot) could not be made.
    Io(io::Error),
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiError::Limit(l) => write!(f, "at most {} {}", l.max(), l.what()),
            UiError::Stale(k) => write!(f, "the {} is gone", k.what()),
            UiError::Invalid(why) => f.write_str(why),
            UiError::Cycle => f.write_str("a group cannot go under itself or what hangs under it"),
            UiError::GlyphNotUploaded => {
                f.write_str("a glyph of the run has not reached the compositor yet")
            }
            UiError::AtlasFull => f.write_str("the glyph atlas is full"),
            UiError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for UiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            UiError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for UiError {
    fn from(e: io::Error) -> Self {
        UiError::Io(e)
    }
}

impl From<UiError> for io::Error {
    fn from(e: UiError) -> Self {
        match e {
            UiError::Io(e) => e,
            other => io::Error::new(io::ErrorKind::InvalidInput, other.to_string()),
        }
    }
}

/// The number of a commit, which `Event::Frame::tag` brings back when the frame that first shows it is on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommitTag(pub u64);

impl CommitTag {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Why `Scene::try_commit` did not take the staged transaction.
#[derive(Debug)]
pub enum SubmitError {
    /// The writer's queue is full (the compositor has not read what was sent earlier). Not a failure: nothing was consumed,
    /// not even the commit's number, the staged transaction is exactly as it was, and it can be committed again once
    /// `Scene::writer_ready` has said there is room (the socket became writable): never in a loop.
    WouldBlock,
    /// The staged transaction is bigger than the writer can ever hold (three segments of 64 KiB, framed). It is still
    /// staged, and cannot be committed whole: large hidden preparations are split into several commits and revealed at
    /// once by a generation (`SetGlyphs::after`, a visibility flip of a group), which is what `Atlas::pump` does.
    TooLarge,
    /// The connection is gone or failed. Sticky: every later call says it again.
    Io(io::Error),
}

impl SubmitError {
    /// Whether this is only a full queue.
    pub fn would_block(&self) -> bool {
        matches!(self, SubmitError::WouldBlock)
    }

    /// The error as an `io::Error`, for a caller that has already dealt with `WouldBlock` and means every other answer to
    /// end what it is doing. There is deliberately no `From`: a `?` on `try_commit` would turn a full queue into a failure.
    pub fn into_io(self) -> io::Error {
        match self {
            SubmitError::Io(e) => e,
            SubmitError::WouldBlock => io::Error::new(io::ErrorKind::WouldBlock, self.to_string()),
            SubmitError::TooLarge => io::Error::new(io::ErrorKind::InvalidInput, self.to_string()),
        }
    }
}

impl fmt::Display for SubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SubmitError::WouldBlock => {
                f.write_str("the compositor has not caught up: wait for the socket")
            }
            SubmitError::TooLarge => {
                f.write_str("the staged transaction is larger than the writer can hold")
            }
            SubmitError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SubmitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SubmitError::Io(e) => Some(e),
            _ => None,
        }
    }
}
