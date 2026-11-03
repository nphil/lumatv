//! Compositor-held resources an app prepares ahead of showing them: prepared (pre-blurred) backdrops and glyph atlases.
//!
//! **Prepared backdrops** (design §3.1 item 3, §3.2). A glass surface whose backdrop is [`crate::Backdrop::Prepared`]
//! shows the *prepared set* it names: a wallpaper plate and one small, already blurred plate per source group, made at
//! `1/downsample` of the screen in each direction, margins for the blur included. The set is made when the app *asks*
//! (`BackdropRefresh`), by a bounded step in a moment the compositor has to spare, never because of a touch, a drag or
//! a settle. A finished set is a *generation*; the glass shows the newest finished one, or the one a lease holds
//! ([`crate::Request::BackdropFreeze`]) and, while a lease is held, no capture of that set is made at all. Plates keep
//! the transform of the group they were made from, so the dock and the content move in lockstep over their own plates with
//! no new capture. Nothing is read back to the CPU.
//!
//! **Atlases and glyph runs** (§3.1 item 4). An atlas is a one-byte-per-pixel coverage texture in the compositor. The
//! app fills it from `Fonts`' raster output in chunks of at most [`MAX_UPLOAD_BYTES`], each staged in one of two shared
//! files ([`staging_path`]) and named by an `AtlasUpload` that carries a generation; the compositor copies the chunk to
//! the texture in a moment it has to spare and answers `Event::AtlasUploaded` with that generation, and only then may the
//! app rewrite that staging slot. A glyph-run layer ([`crate::Content::Glyphs`]) is a list of quads (a rectangle of the atlas to
//! a place in the layer), each drawn in one of the four colours of the layer's palette (its *ink*); `SetGlyphs` swaps the
//! whole list, and the swap waits for the atlas generation it names (`after`) so a label is revealed only when every glyph it
//! uses has arrived. A clock digit that changes is a new list of 256 bytes or less, not a new picture. A list comes sorted by
//! ink, so a run is drawn as one span of triangles per ink with one colour each: a row of a list, with its title, its detail
//! and its trailing value, is one layer and not three.

use crate::LayerId;
use crate::group::GroupId;
use serde::{Deserialize, Serialize};

/// Names a prepared set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BackdropId(pub u32);

/// Names an atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AtlasId(pub u32);

/// Prepared sets a client may have at once, plates in one set, and the largest `downsample`.
pub const MAX_BACKDROPS: usize = 4;
pub const MAX_PLATES: usize = 8;
/// Atlases a client may have, the largest side of one, and the bytes all of them may hold together (the design's font
/// ledger, §11.2, is 8 MiB for the CPU cache and the GPU atlases together).
pub const MAX_ATLASES: usize = 4;
pub const MAX_ATLAS_SIDE: u32 = 2048;
pub const MAX_ATLAS_BYTES: u64 = 4 << 20;
/// The most pixels (bytes) one `AtlasUpload` may carry: the design's chunk bound.
pub const MAX_UPLOAD_BYTES: u32 = 256 * 1024;
/// Staging slots per atlas.
pub const STAGING_SLOTS: u8 = 2;
/// Glyph quads in one run.
pub const MAX_RUN_GLYPHS: usize = 256;
/// Colours in a glyph run's palette: the inks its quads may name.
pub const INKS: usize = 4;

/// The shared file holding one staging slot of an atlas: coverage bytes, rows packed at the width of the upload's
/// rectangle. The app writes it, sends `AtlasUpload`, and leaves it alone until `Event::AtlasUploaded`.
pub fn staging_path(atlas: AtlasId, slot: u8) -> String {
    format!("/run/lumashow/shm/atlas-{}-{}.a8", atlas.0, slot)
}

/// One source of a prepared set: the layers of a group (and of the groups below it) at the group's own local geometry,
/// as the group's pose is the identity, so that the plate can be drawn again wherever the group goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlateSource {
    pub group: GroupId,
}

/// What a prepared set is made of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptureSpec {
    /// The wallpaper layer drawn under every plate, as a plate of its own; `None` leaves the set transparent there.
    pub wallpaper: Option<LayerId>,
    /// At most [`MAX_PLATES`] groups, drawn in this order (first is the bottom), each as its own plate.
    pub plates: Vec<PlateSource>,
    /// The blur's standard deviation in logical pixels (as `Backdrop::Cached`'s).
    pub radius: f32,
    /// How many times smaller than the screen the plates are: 2, 4 or 8 (the design prepares quarter size plates).
    pub downsample: u8,
}

/// One quad of a glyph run: the rectangle (`sx`, `sy`, `w`, `h`) of the atlas, shown with its top-left at (`x`, `y`) of
/// the layer, in the colour `ink` names. Whole pixels both ways, so the text is as sharp as the CPU raster was.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Glyph {
    pub x: f32,
    pub y: f32,
    pub sx: u16,
    pub sy: u16,
    pub w: u16,
    pub h: u16,
    /// Which colour of the run's [`GlyphStyle::palette`] the quad is drawn in: below [`INKS`]. A run's quads come in
    /// non-decreasing ink order (`SetGlyphs` refuses any other), so the compositor draws one span of triangles per ink.
    pub ink: u8,
}

/// What a glyph-run layer draws: quads of `atlas`, each in the colour of `palette` its ink names (premultiplied) times the
/// glyph's coverage. A row of a list is one run of two or three inks: its title, its detail and its trailing value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GlyphStyle {
    pub atlas: AtlasId,
    pub palette: [crate::Rgba; INKS],
}

impl GlyphStyle {
    /// A run of one colour: `colour` is ink 0, and the other inks are transparent.
    pub const fn single(atlas: AtlasId, colour: crate::Rgba) -> Self {
        Self {
            atlas,
            palette: [
                colour,
                crate::Rgba(0, 0, 0, 0),
                crate::Rgba(0, 0, 0, 0),
                crate::Rgba(0, 0, 0, 0),
            ],
        }
    }
}

/// The rectangle of an atlas an upload fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtlasRect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl AtlasRect {
    pub fn bytes(&self) -> u32 {
        u32::from(self.w) * u32::from(self.h)
    }
}
