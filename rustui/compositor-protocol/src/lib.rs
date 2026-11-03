//! The contract between the LumaShow app and its compositor.
//!
//! The app owns *what* is on screen: a flat set of layers, each either an image the app rasterised (text,
//! icons) or a shape the compositor draws itself with a shader (rounded rectangles, gradients, borders —
//! the CPU is too slow for those on the Echo Show, see experiments/s2-raster). The compositor owns *when*:
//! it paces frames to the display, runs animations on its own clock and tells the app what reached the
//! screen. Coordinates are logical pixels of the upright screen; the compositor applies the panel rotation.
//!
//! Messages travel over a Unix stream socket as frames: a little-endian `u32` length, then the postcard
//! encoding of one message. Image pixels never travel over the socket: each image layer's pixels live in a
//! shared-memory file the app writes and the compositor reads (`image_path`).

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

pub mod display;
pub mod fds;
pub mod frames;
pub mod group;
pub mod motion;
pub mod resources;
pub mod scheduling;

pub use group::{GroupId, GroupPose, PoseBinding, PoseProperties};
pub use motion::{
    CancelPolicy, DrivePhase, DriveSample, MotionBounds, MotionId, MotionToken, ScrollToken,
    Settle, SpringToken, Units,
};
pub use resources::{AtlasId, AtlasRect, BackdropId, CaptureSpec, Glyph, GlyphStyle, PlateSource};

/// Bumped whenever a message changes shape. The compositor refuses a `Hello` with another version. Version 7 added native
/// groups, shared motion tracks, prepared backdrops, glyph atlases and runs, and the events that report them
/// (`docs/g2-shell-design.md` section 3.1). Version 8 gives every glyph quad an ink and every glyph run a palette of four
/// colours instead of one (`resources::Glyph::ink`, `GlyphStyle::palette`), so that a run of text in several colours is one
/// layer. Version 9 adds `Settle::TimedAfter`, a timed settle that waits out a delay on the compositor's clock, so that the parts of
/// an entrance that come one after another are asked for in one commit. Version 10 gives a clip a feather (`Props::feather`,
/// `GroupPose::feather`, [`Edges`]): the part of what is clipped that lies within so many pixels of an edge of the clip fades out
/// towards it. Version 11 adds "Smooth first" as a level the app asks the compositor for (`Request::SmoothFirst`, [`SmoothFirst`]),
/// the compositor's word for whether it can draw the Full level (`Event::Welcome::smooth_first_full`) and the step of the Full
/// ladder a frame was drawn at (`Event::Frame::quality`, [`ladder`]). Version 12 changes the curves an animation can ask for to
/// Lucent's (`Ease::{Linear, Out, In, Spring}`: the in-out cubic is gone, the exit curve and the spring's release are new, so the
/// third variant means another curve), and `Out` is the CSS curve itself, not the cubic it resembles. Version 13 adds `Content::Live`,
/// a layer that shows the frames a producer sends to the frame endpoint ([`frames`]) without the app or the compositor copying them.
/// The app and the compositor ship together in the signed base, so there is no version-7, -8, -9, -10, -11 or -12 path.
pub const PROTOCOL_VERSION: u32 = 14;

/// Where the compositor listens. Visible at the same path inside the Echo Show's Android chroot.
pub const SOCKET_PATH: &str = "/run/lumashow/compositor.sock";

/// Largest frame either side accepts; far above any real message, small enough to stop a corrupt length
/// from allocating the device's memory away.
pub const MAX_FRAME: u32 = 64 * 1024;

/// The shared-memory file holding an image layer's pixels: premultiplied RGBA8, `width * 4` bytes per row.
pub fn image_path(layer: LayerId) -> String {
    format!("/run/lumashow/shm/layer-{}.rgba", layer.0)
}

/// The shared-memory file holding a mesh layer's geometry (see [`MeshData`]). The app writes it completely before
/// it sends the `Create`; the compositor reads it once, when the layer is created.
pub fn mesh_path(layer: LayerId) -> String {
    format!("/run/lumashow/shm/layer-{}.mesh", layer.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LayerId(pub u32);

/// Names a source of frames that a producer other than the app fills (`frames`): the `Content::Live` layers that name it show its
/// newest picture. Chosen by the app (it makes the layers) and told to the producer, like a layer's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceId(pub u32);

/// A premultiplied-alpha colour, each channel 0..=255.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// How far in from each edge of a clip rectangle what is clipped fades out, in logical pixels: the layer's alpha is 0 at the
/// edge and rises linearly to 1 that far inside it, so a scrolling list or a pager that meets the edge of its window dissolves
/// instead of being cut. The widths of the two edges of an axis may overlap (a window narrower than both): the fade then never
/// reaches 1. A width of 0 leaves that edge hard, and a clip whose edges are all 0 ([`Edges::NONE`], the default) takes exactly
/// the path a plain clip takes, in the compositor and on the wire.
///
/// The widths are lengths in the same space as the clip they belong to: a group's feather is carried by the group's pose and
/// scaled with it, like the rectangle is. The fade is worked out at every pixel the clip leaves, in the fragment stage, from
/// the pixel's distance to each edge; the corner is the product of the two axes'.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edges {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Edges {
    /// No feather: every edge is hard.
    pub const NONE: Self = Self {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 0.0,
    };

    pub const fn new(left: f32, right: f32, top: f32, bottom: f32) -> Self {
        Self {
            left,
            right,
            top,
            bottom,
        }
    }

    /// The same width on all four edges.
    pub const fn all(width: f32) -> Self {
        Self::new(width, width, width, width)
    }

    /// The same width on the left and right edges (a strip or a pager that scrolls sideways).
    pub const fn horizontal(width: f32) -> Self {
        Self::new(width, width, 0.0, 0.0)
    }

    /// The same width on the top and bottom edges (a list that scrolls up and down).
    pub const fn vertical(width: f32) -> Self {
        Self::new(0.0, 0.0, width, width)
    }

    /// Whether every edge is hard.
    pub fn is_none(&self) -> bool {
        self.left == 0.0 && self.right == 0.0 && self.top == 0.0 && self.bottom == 0.0
    }

    /// Every width times `k` (a group's scale carries its feather to the screen with its clip).
    pub fn scaled(&self, k: f32) -> Self {
        Self::new(self.left * k, self.right * k, self.top * k, self.bottom * k)
    }

    /// Whether every width is a number and not negative: what a pose or a layer's props must have to be taken.
    pub fn is_valid(&self) -> bool {
        [self.left, self.right, self.top, self.bottom]
            .iter()
            .all(|w| w.is_finite() && *w >= 0.0)
    }

    fn widths(&self) -> [f32; 4] {
        [self.left, self.right, self.top, self.bottom]
    }
}

/// On the wire an `Edges` that is none is one byte (an empty sequence) and the rest are the four widths after a length of
/// four: a layer that is not feathered pays one byte for it in every `Set`, not sixteen.
impl Serialize for Edges {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        if self.is_none() {
            return serializer.serialize_seq(Some(0))?.end();
        }
        let mut seq = serializer.serialize_seq(Some(4))?;
        for width in self.widths() {
            seq.serialize_element(&width)?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Edges {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Widths;
        impl<'de> serde::de::Visitor<'de> for Widths {
            type Value = Edges;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("no widths, or the four widths of a feather")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Edges, A::Error> {
                use serde::de::Error;
                let Some(left) = seq.next_element::<f32>()? else {
                    return Ok(Edges::NONE);
                };
                let mut rest = [0.0f32; 3];
                for (width, what) in rest.iter_mut().zip(["right", "top", "bottom"]) {
                    *width = seq.next_element::<f32>()?.ok_or_else(|| {
                        A::Error::custom(format!("a feather without its {what} width"))
                    })?;
                }
                if seq.next_element::<f32>()?.is_some() {
                    return Err(A::Error::custom("a feather of more than four widths"));
                }
                Ok(Edges::new(left, rest[0], rest[1], rest[2]))
            }
        }
        deserializer.deserialize_seq(Widths)
    }
}

/// Where and how a layer is shown. Everything here can be animated by the compositor.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Props {
    /// Top-left corner in logical pixels.
    pub x: f32,
    pub y: f32,
    /// Uniform scale about the layer's centre.
    pub scale: f32,
    pub opacity: f32,
    /// Only the part of the layer inside this logical rectangle is drawn; `None` draws all of it.
    pub clip: Option<Rect>,
    /// How far in from each edge of `clip` the layer fades out (see [`Edges`]). `Edges::NONE` is a hard clip. Without a clip it
    /// means nothing.
    pub feather: Edges,
    pub visible: bool,
}

impl Props {
    pub const fn at(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            scale: 1.0,
            opacity: 1.0,
            clip: None,
            feather: Edges::NONE,
            visible: true,
        }
    }

    /// The same props drawn only inside `rect`, fading out over `edges` towards the rectangle's edges.
    pub const fn clip_feathered(self, rect: Rect, edges: Edges) -> Self {
        Self {
            clip: Some(rect),
            feather: edges,
            ..self
        }
    }
}

/// A shape the compositor draws with a shader, `width` × `height` logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub corner_radius: f32,
    pub fill: Fill,
    /// Border drawn inside the edge; width 0 draws none.
    pub border_width: f32,
    pub border: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Fill {
    Solid(Rgba),
    /// A linear gradient across the shape from `from` to `to`, at `angle_deg` (0 = left to right,
    /// 90 = top to bottom).
    Linear {
        from: Rgba,
        to: Rgba,
        angle_deg: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Content {
    /// Pixels the app writes into `image_path(id)`.
    Image,
    Shape(Shape),
    /// A Lucent glass surface. The layer's size is the body; a shadow is drawn outside it.
    Glass(Glass),
    /// A full-screen background the compositor animates on its own clock. The kinds whose
    /// [`WallpaperKind::uses_image`] is true warp the pixels the app writes into `image_path(id)`, exactly like an
    /// image layer's.
    Wallpaper(Wallpaper),
    /// A 3D scene: the geometry in `mesh_path(id)` seen through a camera, drawn into the layer's rectangle.
    Mesh(Mesh),
    /// Text as a run of glyph quads of an atlas (`Request::SetGlyphs` gives the quads; `resources` explains the whole
    /// route). The layer's size bounds what it draws.
    Glyphs(GlyphStyle),
    /// The newest picture of a producer's source (the frame endpoint, [`frames`]), drawn into the layer's rectangle like an image's.
    /// Nothing is written by the app; the layer shows nothing until the source has a picture, and again nothing from a `Clear`.
    Live(Live),
}

/// A layer that shows the frames of a producer's source (`Content::Live`). The layer's size is the rectangle the picture is drawn
/// into (a picture of another size is scaled to it, filtered linearly); everything else about the layer is the app's, as for an
/// image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Live {
    pub source: SourceId,
}

/// A Lucent glass surface (LANGUAGE.md section 2): one rounded quad that draws, bottom to top, its soft shadows,
/// the blurred backdrop, the tint, the neumorphic depth, the sheen, the inner top highlight and the perimeter edge.
/// Light comes from the upper left (`light.{x,y}`), fixed in screen space. Colours are premultiplied.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Glass {
    pub corner_radius: f32,
    /// The glass colour over the backdrop (palette role `glass`).
    pub tint: Rgba,
    /// The perimeter edge, `edge_width` logical pixels wide, inside the shape; width 0 draws none.
    pub edge: Rgba,
    pub edge_width: f32,
    /// The inner top lip (`highlight.rest`/`raised`): a one-pixel line just inside the top edge, fading out down
    /// the corners; transparent draws none.
    pub highlight: Rgba,
    pub sheen: Option<Sheen>,
    /// Up to two soft drop shadows (`shadow.rest` has one, `shadow.raised` two), drawn outside the body.
    pub shadows: [Option<Shadow>; 2],
    /// Neumorphic light/shade pair for wells and raised controls.
    pub depth: Option<Depth>,
    pub backdrop: Backdrop,
    /// Multiplies the backdrop's saturation (1 leaves it unchanged); ignored without a backdrop.
    pub saturation: f32,
    /// How far, in logical pixels, the backdrop is bent towards the inside at the edge, as through a lens; 0 is
    /// flat glass. Ignored without a backdrop.
    pub refraction: f32,
    /// Outward edge optics, independent of legacy inward refraction.
    pub edge_lens: Option<EdgeLens>,
    /// A vertically varying premultiplied tint.
    pub tint_gradient: Option<TintGradient>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeLens {
    pub bevel: f32,
    pub lens: f32,
    pub rim_clear: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TintGradient {
    pub top: Rgba,
    pub bottom: Rgba,
}

/// White light along the 135° light axis (`sheen.raised`): alpha `floor` outside `start..end` (fractions of the
/// body's diagonal from the upper-left corner) rising to `peak` in the middle of that band.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sheen {
    pub peak: f32,
    pub floor: f32,
    pub start: f32,
    pub end: f32,
}

/// A soft shadow with CSS `box-shadow` meaning: the body's shape moved down `offset_y` and blurred by `blur`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub offset_y: f32,
    pub blur: f32,
    pub colour: Rgba,
}

/// Neumorphic depth (`neumorphic.*`): a `light` copy of the shape towards the upper left and a `shade` copy towards
/// the lower right, each `offset` away and blurred by `blur`; outside the body for a raised control, inside it for
/// an inset well.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Depth {
    pub inset: bool,
    pub offset: f32,
    pub blur: f32,
    pub light: Rgba,
    pub shade: Rgba,
}

/// What a glass surface shows of the layers below it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Backdrop {
    /// Nothing: the tint alone (Lucent T0).
    None,
    /// A blurred snapshot of the layers below, `radius` logical pixels of blur (`blur.surface`). It is taken again
    /// only when a layer below changes what it is or where it rests (create, delete, restyle, damage, set), never
    /// because of an animation or a wallpaper's motion (Lucent T1).
    Cached { radius: f32 },
    /// The layers below, blurred afresh on every frame the surface is drawn, at 1/`downsample` of the screen's
    /// resolution (2, 4 or 8) (Lucent T2 and above).
    Live { radius: f32, downsample: u8 },
    /// The prepared set `id` (`Request::BackdropCreate`): its newest finished generation, or the one a lease holds. It is
    /// never made because of this surface drawing, moving or being touched; what the set holds is made when the app asks
    /// (see `resources`). A set with no finished generation yet draws the tint alone.
    Prepared { id: BackdropId },
}

/// A background the compositor animates. `speed` scales the designed pace (1 = as designed, 0 = still) and
/// `intensity` the amount of it (0..=1: wave amplitude, number of particles, weight of the weather).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wallpaper {
    pub kind: WallpaperKind,
    pub speed: f32,
    pub intensity: f32,
    /// How often the motion advances, in frames per second (at most the panel's refresh). While nothing else on
    /// screen changes the compositor draws the wallpaper at this rate; 0 holds it still.
    pub fps: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum WallpaperKind {
    /// The LumaGlass live wallpaper: the image through a slow domain warp, with light playing over the folds.
    Silk,
    /// Soft colour clouds drifting slowly, in the four given colours.
    Aurora { colours: [Rgba; 4] },
    /// A water surface over the image: tap ripples (`Request::Ripple`) and a slow swell refract it.
    Liquid,
    /// Twinkling stars on a slowly turning night sky.
    Starfield,
    /// Small glowing dots drifting upwards and fading; transparent between them, so it overlays a wallpaper below.
    Fireflies { colour: Rgba },
    /// Weather over a wallpaper below; transparent where there is none.
    Weather(Weather),
}

impl WallpaperKind {
    /// Whether the app supplies pixels for this kind in `image_path(id)`.
    pub fn uses_image(&self) -> bool {
        matches!(self, Self::Silk | Self::Liquid)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Weather {
    Rain,
    Storm,
    Snow,
    Sun,
    Clouds,
    Fog,
}

/// A 3D scene layer. The camera looks from `eye` at `target` with `fov_deg` vertical field of view (y is up);
/// the scene is lit by one directional light (`light` points from the scene towards it) plus `ambient`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub fov_deg: f32,
    pub light: [f32; 3],
    pub ambient: f32,
    /// Turns the eye about the vertical axis through `target`, on the compositor's clock; 0 holds it still.
    pub orbit_deg_per_s: f32,
    /// Smooths the edges of triangles (4× multisampling).
    pub antialias: bool,
    /// What the layer shows where no triangle is; transparent lets the layers below through.
    pub background: Rgba,
}

/// A mesh layer's geometry: triangles given as indices into the vertices. In the file (little-endian):
/// the magic `LSMESH1\0`, the vertex count and the index count as `u32`, the vertices ([`MESH_VERTEX_BYTES`] each:
/// position `[f32; 3]`, normal `[i8; 3]` scaled to -127..=127 plus one padding byte, premultiplied colour
/// `[u8; 4]`), then the indices as `u16`, three per triangle.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshVertex {
    pub position: [f32; 3],
    /// Unit normal.
    pub normal: [f32; 3],
    pub colour: Rgba,
}

pub const MESH_MAGIC: &[u8; 8] = b"LSMESH1\0";
pub const MESH_VERTEX_BYTES: usize = 20;
/// Largest mesh either side accepts, in vertices (u16 indices reach no further).
pub const MAX_MESH_VERTICES: usize = 65_536;
pub const MAX_MESH_INDICES: usize = 3 * 131_072;

impl MeshData {
    /// Checks the mesh is drawable: whole triangles, every index in range, sizes within the limits.
    pub fn validate(&self) -> Result<(), String> {
        if self.vertices.len() > MAX_MESH_VERTICES {
            return Err(format!(
                "{} vertices; at most {MAX_MESH_VERTICES}",
                self.vertices.len()
            ));
        }
        if self.indices.len() > MAX_MESH_INDICES || self.indices.len() % 3 != 0 {
            return Err(format!(
                "{} indices; a whole number of triangles, at most {MAX_MESH_INDICES}",
                self.indices.len()
            ));
        }
        if let Some(&i) = self
            .indices
            .iter()
            .find(|&&i| usize::from(i) >= self.vertices.len())
        {
            return Err(format!(
                "index {i} past the {} vertices",
                self.vertices.len()
            ));
        }
        if let Some(v) = self
            .vertices
            .iter()
            .find(|v| !v.position.iter().all(|c| c.is_finite()))
        {
            return Err(format!("vertex position {:?} is not a number", v.position));
        }
        Ok(())
    }

    /// The file form of a valid mesh.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut out = Vec::with_capacity(
            16 + self.vertices.len() * MESH_VERTEX_BYTES + self.indices.len() * 2,
        );
        out.extend_from_slice(MESH_MAGIC);
        out.extend_from_slice(&(self.vertices.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.indices.len() as u32).to_le_bytes());
        let snorm = |c: f32| (c.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8;
        for v in &self.vertices {
            for c in v.position {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&[
                snorm(v.normal[0]),
                snorm(v.normal[1]),
                snorm(v.normal[2]),
                0,
            ]);
            out.extend_from_slice(&[v.colour.0, v.colour.1, v.colour.2, v.colour.3]);
        }
        for i in &self.indices {
            out.extend_from_slice(&i.to_le_bytes());
        }
        Ok(out)
    }

    /// Reads the file form back; refuses anything `encode` would not have written.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let header = bytes.get(..16).ok_or("shorter than the mesh header")?;
        if &header[..8] != MESH_MAGIC {
            return Err("not a LumaShow mesh".into());
        }
        let count = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap()) as usize;
        let (vertex_count, index_count) = (count(8), count(12));
        if vertex_count > MAX_MESH_VERTICES || index_count > MAX_MESH_INDICES {
            return Err(format!(
                "{vertex_count} vertices, {index_count} indices: past the limits"
            ));
        }
        let indices_at = 16 + vertex_count * MESH_VERTEX_BYTES;
        if bytes.len() != indices_at + index_count * 2 {
            return Err(format!(
                "{} bytes where {vertex_count} vertices and {index_count} indices take {}",
                bytes.len(),
                indices_at + index_count * 2
            ));
        }
        let f = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let unsnorm = |b: u8| f32::from(b as i8) / 127.0;
        let vertices = (0..vertex_count)
            .map(|n| {
                let at = 16 + n * MESH_VERTEX_BYTES;
                MeshVertex {
                    position: [f(at), f(at + 4), f(at + 8)],
                    normal: [
                        unsnorm(bytes[at + 12]),
                        unsnorm(bytes[at + 13]),
                        unsnorm(bytes[at + 14]),
                    ],
                    colour: Rgba(
                        bytes[at + 16],
                        bytes[at + 17],
                        bytes[at + 18],
                        bytes[at + 19],
                    ),
                }
            })
            .collect();
        let indices = bytes[indices_at..]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let mesh = Self { vertices, indices };
        mesh.validate()?;
        Ok(mesh)
    }
}

/// Timing curve of an animation. These are the curves of Lucent's motion language and no others can be asked for: what each is for is
/// `ui::lucent::tokens::motion`'s to say, and [`motion::ease`] is the one place that evaluates them, for the compositor's frames and for
/// the app's estimates of them alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ease {
    /// Elapsed progress as it is (Lucent's reduced-motion fade, a jump that has to take a frame, a departure that goes on at the speed a
    /// finger let go at).
    Linear,
    /// Lucent's `--lu-ease`, `cubic-bezier(.33,1,.68,1)`, evaluated exactly: fast start, gentle landing. The default for anything
    /// that arrives, settles or fades in.
    Out,
    /// Lucent's `--lu-ease-exit`, `cubic-bezier(.4,0,1,1)`, evaluated exactly: starts gently and accelerates away. Everything that
    /// leaves from rest.
    In,
    /// A critically damped spring that starts at rest ([`motion::SPRING_K`]), landing on time: the release of something held out of
    /// place (a rubber band, a card let go of short of leaving), as the compositor's own tracks release theirs.
    Spring,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Request {
    /// First message on a connection.
    Hello {
        version: u32,
    },
    /// Creates a layer, or replaces it if the id exists (a layer's size is fixed for its life, so a resize
    /// is a replace). `z` orders layers: higher is drawn on top.
    Create {
        id: LayerId,
        width: u32,
        height: u32,
        z: i32,
        content: Content,
        props: Props,
    },
    /// The app finished writing `rect` (layer-local pixels) of an image layer, or of a wallpaper that uses an image.
    Damage {
        id: LayerId,
        rect: Rect,
    },
    /// Changes how a shape, glass, wallpaper or mesh layer looks. `content` must be of the layer's own kind (a
    /// wallpaper must keep whether it uses an image); a different kind is a new layer (`Create` again).
    Restyle {
        id: LayerId,
        content: Content,
    },
    /// Starts a ripple on a `Liquid` wallpaper at (`x`, `y`), layer-local logical pixels.
    Ripple {
        id: LayerId,
        x: f32,
        y: f32,
    },
    /// Moves a layer at once, cancelling any animation on it.
    Set {
        id: LayerId,
        props: Props,
    },
    /// Animates a layer from where it is now to `to`.
    Animate {
        id: LayerId,
        to: Props,
        duration_ms: u32,
        ease: Ease,
    },
    Delete {
        id: LayerId,
    },
    /// Everything since the previous commit becomes visible on the same frame. `tag` comes back in the
    /// `Frame` event of the frame that first shows it.
    Commit {
        tag: u64,
    },
    /// Asks for `Stats` once.
    Stats,
    /// Holds the GPU at one of its clocks (`mhz`, which must be one the device's profile lists; a device without clock
    /// control refuses it), or gives the clock back to the compositor's own policy with `None`. The compositor is the
    /// only writer of the GPU's clock: while frames flow it holds the fastest one by itself and hands the GPU back when
    /// they stop, and a hold replaces that for a measurement at a known clock. It takes effect at once, not with a
    /// `Commit` (the clock is no part of a frame), and ends when the connection does.
    HoldGpuClock {
        mhz: Option<u32>,
    },
    /// Switches the experimental drawing changes of the glass-ideas lab on and off: `mask` (bits of [`ideas`]) is all that
    /// is on, replacing what was. It is no part of a frame and takes effect with the next one drawn; it ends when the
    /// connection does.
    Ideas {
        mask: u32,
    },
    /// Sets "Smooth first": what the compositor may leave out or draw plainer to keep every frame on its refresh (see
    /// [`SmoothFirst`]). `Off` and `Invisible` are the glass ideas none and all (`ideas::DEFAULT`); `Full` is `Invisible` plus the
    /// ladder of visible steps ([`ladder`]), which a compositor that said it cannot (`Event::Welcome::smooth_first_full`)
    /// refuses. It replaces what [`Request::Ideas`] set. Like it, it is no part of a frame, takes effect with the next frame
    /// drawn, and ends when the connection does (the compositor goes back to `Invisible`).
    SmoothFirst {
        level: SmoothFirst,
    },
    /// Says whether anyone can see the screen. While it is dark (`on: false`, sent once the app has turned the light off)
    /// the compositor draws nothing: commits are taken in and animations keep their clock, but no frame is made, so a
    /// dark screen costs no GPU and next to no CPU. `on: true` draws the scene as it is now straight away, whether or not
    /// a commit comes with it; the app turns the light on once that frame is shown (`Event::Frame`). It takes effect at
    /// once, not with a `Commit`, and ends (the screen counts as seen) when the connection does.
    Display {
        on: bool,
    },

    // --- Native groups (`group`). Staged with the batch like a layer change and applied with its `Commit`. ---
    /// Creates a group under `parent` (or the screen) with `pose`. At most [`group::MAX_GROUPS`], [`group::MAX_GROUP_DEPTH`]
    /// deep; a cycle or a non-finite pose is refused.
    GroupCreate {
        id: GroupId,
        parent: Option<GroupId>,
        pose: GroupPose,
    },
    /// Sets a group's own pose at once. A property a track is bound to keeps following its track.
    GroupSet {
        id: GroupId,
        pose: GroupPose,
    },
    /// Moves a group under another parent. With `preserve_world` it takes the pose that leaves its evaluated world as it is
    /// now (bindings re-anchored by the same difference); without, it keeps its pose and so moves with its new parent.
    GroupParent {
        id: GroupId,
        parent: Option<GroupId>,
        preserve_world: bool,
    },
    /// Binds the properties `binding` names of the group to the track: the group follows the track from the next frame.
    /// A group has at most one binding per property; a new one replaces the old for the properties it names.
    GroupBind {
        id: GroupId,
        motion: MotionId,
        binding: PoseBinding,
    },
    /// Frees the properties from their bindings; they keep the values they have.
    GroupUnbind {
        id: GroupId,
        properties: PoseProperties,
    },
    /// Deletes a group. What hangs under it (groups and layers) goes to its parent with the world pose it had: nothing
    /// moves, and no hit tree is left behind.
    GroupDelete {
        id: GroupId,
    },
    /// Puts a layer under `group` (or, with `None`, back on the screen) with its own props `local`, in the group's space,
    /// replacing the props it had and cancelling its animation. The layer's own `Set` and `Animate` act on `local` from now on.
    Attach {
        id: LayerId,
        group: Option<GroupId>,
        local: Props,
    },

    // --- Shared motion tracks (`motion`). ---
    /// Creates a track at `value`. At most [`motion::MAX_MOTIONS`].
    MotionCreate {
        id: MotionId,
        value: f32,
        bounds: MotionBounds,
    },
    /// Deletes a track. The properties bound to it keep the values they have.
    MotionDelete {
        id: MotionId,
    },
    /// A pointer sample. **At once**, not with a `Commit`, so a finger that lands on a settling track catches it in the
    /// frame it is taken into (a sample for a track created in a batch not yet committed waits in that batch, in order).
    MotionDrive {
        id: MotionId,
        sample: DriveSample,
    },
    /// Starts a settle from the track's actual value, with `velocity` (the measured one when `None`), toward `target` (the
    /// nearest anchor, or where it is, when `None`; scroll inertia takes none). At once, like a drive sample.
    MotionSettle {
        id: MotionId,
        target: Option<f32>,
        velocity: Option<f32>,
        settle: Settle,
    },
    /// Stops the track. At once.
    MotionCancel {
        id: MotionId,
        policy: CancelPolicy,
    },

    // --- Prepared backdrops (`resources`). ---
    /// Creates a prepared set. Nothing is made until the first `BackdropRefresh`.
    BackdropCreate {
        id: BackdropId,
        spec: CaptureSpec,
    },
    /// The content the set shows has changed to `content` (an app-chosen number that comes back in the event): make the
    /// next generation in a moment the compositor has to spare, unless a lease holds the set, and answer
    /// `Event::BackdropReady`. Coalesced: only the newest content is made.
    BackdropRefresh {
        id: BackdropId,
        content: u64,
    },
    /// Holds the generation the set shows now (answered by `Event::BackdropFrozen` with its number); no capture of the set is
    /// made, and the set is not replaced, until the matching `BackdropRelease`. At once.
    BackdropFreeze {
        id: BackdropId,
    },
    /// Ends one lease. When the last ends, a refresh that came meanwhile is made. At once.
    BackdropRelease {
        id: BackdropId,
    },
    BackdropDelete {
        id: BackdropId,
    },

    // --- Atlases and glyph runs (`resources`). ---
    /// Creates an atlas of `width` x `height` coverage bytes (each side at most [`resources::MAX_ATLAS_SIDE`]).
    AtlasCreate {
        id: AtlasId,
        width: u32,
        height: u32,
    },
    /// The app has written `rect.bytes()` bytes of coverage, rows packed, into staging slot `slot` of the atlas: copy them
    /// to `rect` of the atlas texture and answer `Event::AtlasUploaded { generation }`. At most
    /// [`resources::MAX_UPLOAD_BYTES`]; staged with the batch and applied with its `Commit`. A slot is written again only
    /// after its `AtlasUploaded`: a chunk that arrives while [`resources::STAGING_SLOTS`] others wait is not copied (the
    /// compositor answers it at once, `AtlasUploaded`, so that nothing waits for it).
    AtlasUpload {
        id: AtlasId,
        slot: u8,
        generation: u64,
        rect: AtlasRect,
    },
    AtlasDelete {
        id: AtlasId,
    },
    /// Replaces the quads of a glyph-run layer. They take effect once every upload of the layer's atlas up to
    /// `after` has reached the texture (`0`: at once), so a label appears whole. At most [`resources::MAX_RUN_GLYPHS`].
    SetGlyphs {
        id: LayerId,
        after: u64,
        glyphs: Vec<Glyph>,
    },
}

/// The glass ideas under test (experiments/s9-glass/RESULTS.md, "Glass ideas"), one bit each in [`Request::Ideas`]'s
/// mask, and the names the lab's tool calls them by. Every one is off until asked for, and off draws exactly what the
/// compositor drew before it existed.
pub mod ideas {
    /// The blur behind glass is made at half the resolution (soft, so it is not seen).
    pub const HALF_RES: u32 = 1;
    /// Glass that has not changed, and has nothing changing behind it but wallpapers, is drawn once into a plate (a
    /// texture of the panel's size, `still.rs` in the compositor) and shown from it.
    pub const STILL_GLASS: u32 = 2;
    /// Glass in motion freezes its blur and drops its refraction, and both come back when it stops.
    pub const ADAPTIVE: u32 = 4;

    /// Every idea by name, in the order the lab shows them.
    pub const NAMES: [(&str, u32); 3] = [
        ("half_res", HALF_RES),
        ("still_glass", STILL_GLASS),
        ("adaptive", ADAPTIVE),
    ];

    /// All the bits there are.
    pub const ALL: u32 = HALF_RES | STILL_GLASS | ADAPTIVE;

    /// What the compositor draws with until a client asks otherwise, and again once that client has gone: the ideas nobody can
    /// see, which are all of them (Nitin compared each at the Show: half_res and still_glass show no difference, adaptive is
    /// barely noticed). It is the "Smooth first: Invisible only" default of the quality design (experiments/s9-glass/GOVERNOR.md).
    /// Measured 10-01 on all 20 lab screens: no screen draws slower with them, the T3 mean render p50 falls 8.90 -> 6.94 ms.
    pub const DEFAULT: u32 = ALL;
}

/// "Smooth first" (Settings › Performance): what the compositor may leave out, or draw plainer, so that no frame is late. The
/// compositor says what each level is, in one place, so that no app encodes its meaning (`ideas()`, `ladder()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmoothFirst {
    /// Nothing is skipped: every effect is drawn the full way, every frame. A very busy screen can drop a frame.
    Off,
    /// Skips only what cannot be seen (the glass ideas, `ideas::DEFAULT`), whatever the load. The default.
    Invisible,
    /// `Invisible`, and then, one step at a time and only while a frame is predicted to be late, plainer looks (the ladder,
    /// [`ladder`]); the steps are taken back, one at a time, when the load eases.
    Full,
}

impl SmoothFirst {
    /// What the compositor draws with until a client asks otherwise, and again once that client has gone.
    pub const DEFAULT: Self = Self::Invisible;

    /// Every level, in the order Settings lists them.
    pub const ALL: [Self; 3] = [Self::Off, Self::Invisible, Self::Full];

    /// The level's name in a stored setting and in the lab's tool.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Invisible => "invisible",
            Self::Full => "full",
        }
    }

    /// The level named `id` (`SmoothFirst::id`).
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.id() == id)
    }

    /// The glass ideas this level draws with: none for `Off`, every one for the others.
    pub const fn ideas(self) -> u32 {
        match self {
            Self::Off => 0,
            Self::Invisible | Self::Full => ideas::DEFAULT,
        }
    }

    /// Whether this level runs the ladder of visible steps.
    pub const fn ladder(self) -> bool {
        matches!(self, Self::Full)
    }
}

/// The ladder of the `Full` level: the visible steps, in the order they are taken (Lucent section 10: live blur, then ambient,
/// then extra sheen and shadow, then all optional effects), one at a time and only while the compositor's own pacer predicts a
/// frame will be late (`crates/compositor/src/ladder.rs`). A step is in force with all the ones before it.
pub mod ladder {
    /// What each step does, in plain words, first step first.
    pub const STEPS: [&str; 5] = [
        "live blur becomes a picture",
        "wallpaper at most 30 frames a second",
        "wallpaper and 3D orbit stand still",
        "shadows and sheen softened",
        "glass becomes a plain tint",
    ];

    /// How many steps there are: the most a frame's `quality` can be.
    pub const RUNGS: u8 = STEPS.len() as u8;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    Welcome {
        version: u32,
        panel_width: u32,
        panel_height: u32,
        refresh_hz: f32,
        /// Whether this compositor can draw `SmoothFirst::Full` (its ladder is built and was not switched off for a
        /// measurement, `LUMASHOW_LADDER=off`). Settings offers the level only while it is true: a request for it that
        /// the compositor cannot carry out is refused, never a level that does nothing.
        smooth_first_full: bool,
    },
    /// A frame reached the screen. Times are `CLOCK_MONOTONIC` nanoseconds.
    Frame {
        /// Highest commit tag included in this frame.
        tag: u64,
        /// When the frame's vsync/pan returned (the moment it started scanning out).
        presented_ns: u64,
        /// Time the compositor spent building this frame, until the GPU finished it.
        render_us: u32,
        /// Refreshes the compositor could have made and did not: a frame was due (an animation was running, or commits
        /// were following one another) and a compositor that had started the frame as soon as its content was there and
        /// the frame before was on the screen would have shown it that many refreshes sooner, with the pipeline the
        /// frame took (`pacer::Pacer::presented` in the compositor). Not refreshes that merely passed without a
        /// frame: a commit that reaches the compositor too late for a refresh has nothing to show at it, and a frame
        /// that costs more than its neighbours lands where its cost puts it (`render_us` is for that). An app that is
        /// late with its own updates is the app's to measure.
        missed: u32,
        /// Backdrops blurred for this frame: each live glass capture, and each cached one taken again.
        backdrops: u16,
        /// The clock the GPU was held at while the frame was drawn, in MHz (the boost, or what `HoldGpuClock` asked for);
        /// 0 when the compositor had left it to the driver's own scaling, which makes it whatever that chose.
        gpu_mhz: u16,
        /// Plate captures of prepared backdrops that ran since the previous frame event (zero during any motion).
        captures: u16,
        /// Bytes of atlas and picture uploads that reached textures since the previous frame event.
        uploaded_bytes: u32,
        /// How many steps of the Full ladder were in force when the frame was drawn (`ladder`): 0 when none, always so below
        /// `SmoothFirst::Full`. A frame that shows a step in force is not a frame of the full-material look, whatever its
        /// refresh, and is reported as such (`docs/g2-shell-design.md` section 12.3).
        quality: u8,
    },
    /// A motion track started a settle, was caught by a finger, or came to rest. `value` and `velocity` are what the
    /// track had at `time_ns` (`CLOCK_MONOTONIC`); `generation` changes with every settle and catch, so a late event
    /// cannot be mistaken for the current one.
    Motion {
        id: MotionId,
        generation: u32,
        kind: motion::NoticeKind,
        value: f32,
        velocity: f32,
        time_ns: u64,
    },
    /// A new generation of a prepared set is finished and is what its glass shows from the next frame (unless leased).
    BackdropReady {
        id: BackdropId,
        generation: u64,
        content: u64,
    },
    /// A lease holds `generation` of the set.
    BackdropFrozen {
        id: BackdropId,
        generation: u64,
    },
    /// Everything up to `generation` that was uploaded from `slot` of the atlas has reached its texture: the slot may be
    /// written again.
    AtlasUploaded {
        id: AtlasId,
        slot: u8,
        generation: u64,
    },
    Stats(Stats),
    /// A request was refused; the connection stays usable.
    Error {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub frames: u64,
    /// The sum of `Event::Frame::missed`.
    pub missed: u64,
    pub layers: u32,
    pub last_render_us: u32,
    /// Actual scan-out planes, recorded once after the startup self-test, not the requested route.
    pub scanout_planes: u32,
    pub worst_render_us: u32,
    /// Groups and tracks the client holds now.
    pub groups: u32,
    pub motions: u32,
    /// Plate captures made since the compositor started, and prepared generations published.
    pub captures: u64,
}

/// Writes one message as a frame.
pub fn write_frame<T: Serialize>(out: &mut impl Write, message: &T) -> io::Result<()> {
    let body =
        postcard::to_stdvec(message).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = u32::try_from(body.len())
        .ok()
        .filter(|&n| n <= MAX_FRAME)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("message of {} bytes is too large", body.len()),
            )
        })?;
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(&body);
    out.write_all(&frame)
}

/// Reads one frame; `Ok(None)` at a clean end of stream.
pub fn read_frame<T: for<'de> Deserialize<'de>>(
    input: &mut impl Read,
    buf: &mut Vec<u8>,
) -> io::Result<Option<T>> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len);
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame of {len} bytes exceeds the limit"),
        ));
    }
    buf.resize(len as usize, 0);
    input.read_exact(buf)?;
    postcard::from_bytes(buf)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_survive_a_round_trip_through_frames() {
        let requests = vec![
            Request::Hello {
                version: PROTOCOL_VERSION,
            },
            Request::Create {
                id: LayerId(7),
                width: 400,
                height: 120,
                z: 3,
                content: Content::Shape(Shape {
                    corner_radius: 24.0,
                    fill: Fill::Linear {
                        from: Rgba(1, 2, 3, 255),
                        to: Rgba(4, 5, 6, 255),
                        angle_deg: 90.0,
                    },
                    border_width: 1.0,
                    border: Rgba(255, 255, 255, 40),
                }),
                props: Props {
                    clip: Some(Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 10.0,
                        h: 10.0,
                    }),
                    ..Props::at(12.5, 30.0)
                },
            },
            Request::Animate {
                id: LayerId(7),
                to: Props::at(0.0, 0.0),
                duration_ms: 240,
                ease: Ease::Out,
            },
            Request::Restyle {
                id: LayerId(8),
                content: Content::Glass(Glass {
                    corner_radius: 24.0,
                    tint: Rgba(18, 14, 26, 102),
                    edge: Rgba(33, 33, 33, 33),
                    edge_width: 1.0,
                    highlight: Rgba(18, 18, 18, 18),
                    sheen: Some(Sheen {
                        peak: 0.16,
                        floor: 0.05,
                        start: 0.38,
                        end: 0.6,
                    }),
                    shadows: [
                        Some(Shadow {
                            offset_y: 24.0,
                            blur: 60.0,
                            colour: Rgba(0, 0, 0, 71),
                        }),
                        None,
                    ],
                    depth: Some(Depth {
                        inset: true,
                        offset: 2.0,
                        blur: 4.0,
                        light: Rgba(26, 26, 26, 26),
                        shade: Rgba(0, 0, 0, 82),
                    }),
                    backdrop: Backdrop::Live {
                        radius: 30.0,
                        downsample: 4,
                    },
                    saturation: 1.25,
                    refraction: 6.0,
                    edge_lens: Some(EdgeLens { bevel: 12.0, lens: 24.0, rim_clear: 0.6 }),
                    tint_gradient: Some(TintGradient { top: Rgba(3, 4, 5, 30), bottom: Rgba(4, 5, 6, 40) }),
                }),
            },
            Request::Create {
                id: LayerId(9),
                width: 960,
                height: 480,
                z: -10,
                content: Content::Wallpaper(Wallpaper {
                    kind: WallpaperKind::Weather(Weather::Snow),
                    speed: 1.0,
                    intensity: 0.5,
                    fps: 30,
                }),
                props: Props::at(0.0, 0.0),
            },
            Request::Ripple {
                id: LayerId(9),
                x: 480.0,
                y: 240.0,
            },
            Request::HoldGpuClock { mhz: Some(416) },
            Request::HoldGpuClock { mhz: None },
            Request::Ideas { mask: ideas::ALL },
            Request::SmoothFirst {
                level: SmoothFirst::Full,
            },
            Request::Commit { tag: u64::MAX },
        ];
        let mut wire = Vec::new();
        for r in &requests {
            write_frame(&mut wire, r).unwrap();
        }
        let mut input = wire.as_slice();
        let mut buf = Vec::new();
        for r in &requests {
            assert_eq!(
                read_frame::<Request>(&mut input, &mut buf)
                    .unwrap()
                    .as_ref(),
                Some(r)
            );
        }
        assert_eq!(read_frame::<Request>(&mut input, &mut buf).unwrap(), None);
    }

    #[test]
    fn a_feather_that_is_none_is_one_byte_on_the_wire_and_the_rest_come_back_whole() {
        let clip = Rect {
            x: 1.0,
            y: 2.0,
            w: 30.0,
            h: 40.0,
        };
        // Four numbers, no clip, no feather, shown: 16 + 1 + 1 + 1.
        let plain = Props::at(1.0, 2.0);
        assert_eq!(postcard::to_stdvec(&plain).unwrap().len(), 19);
        // A clip is 17 bytes and a feather is 17 (a length of four, and the four widths).
        let feathered = plain.clip_feathered(clip, Edges::new(1.0, 2.0, 3.0, 4.0));
        assert_eq!(
            postcard::to_stdvec(&feathered).unwrap().len(),
            16 + 17 + 17 + 1
        );
        let hard = Props {
            clip: Some(clip),
            ..plain
        };
        assert_eq!(postcard::to_stdvec(&hard).unwrap().len(), 16 + 17 + 1 + 1);
        for props in [plain, feathered, hard] {
            let bytes = postcard::to_stdvec(&props).unwrap();
            assert_eq!(postcard::from_bytes::<Props>(&bytes).unwrap(), props);
        }
        for edges in [
            Edges::NONE,
            Edges::all(12.0),
            Edges::horizontal(34.0),
            Edges::vertical(0.5),
            Edges::new(0.0, 0.0, 0.0, 7.0),
        ] {
            let bytes = postcard::to_stdvec(&edges).unwrap();
            assert_eq!(bytes.len(), if edges.is_none() { 1 } else { 17 });
            assert_eq!(postcard::from_bytes::<Edges>(&bytes).unwrap(), edges);
        }
        // A feather of three widths or of five is not one.
        for count in [1u8, 2, 3, 5] {
            let mut bytes = vec![count];
            bytes.extend((0..count).flat_map(|_| 1.0f32.to_le_bytes()));
            assert!(postcard::from_bytes::<Edges>(&bytes).is_err(), "{count}");
        }
    }

    #[test]
    fn each_smooth_first_level_names_what_it_draws_with_and_the_names_come_back() {
        // Off draws every effect the full way, Invisible and Full draw with the glass ideas nobody can see, and only Full has the
        // ladder: the compositor is the one place that says so.
        let levels = SmoothFirst::ALL;
        assert_eq!(
            levels.map(SmoothFirst::ideas),
            [0, ideas::DEFAULT, ideas::DEFAULT]
        );
        assert_eq!(levels.map(SmoothFirst::ladder), [false, false, true]);
        assert_eq!(levels.map(SmoothFirst::id), ["off", "invisible", "full"]);
        for level in levels {
            assert_eq!(SmoothFirst::from_id(level.id()), Some(level));
        }
        assert_eq!(SmoothFirst::from_id("Full"), None);
        assert_eq!(SmoothFirst::from_id(""), None);
        // Today's behaviour is the default, and the glass ideas' default is what Invisible draws with.
        assert_eq!(SmoothFirst::DEFAULT.ideas(), ideas::DEFAULT);
        assert!(!SmoothFirst::DEFAULT.ladder());
        // Every step of the ladder has its words.
        assert_eq!(usize::from(ladder::RUNGS), ladder::STEPS.len());
        assert!(ladder::STEPS.iter().all(|words| !words.is_empty()));
    }

    #[test]
    fn the_frame_says_how_many_steps_of_the_ladder_it_was_drawn_at_and_the_welcome_says_whether_the_ladder_is_there()
     {
        let events = [
            Event::Welcome {
                version: PROTOCOL_VERSION,
                panel_width: 960,
                panel_height: 480,
                refresh_hz: 59.64,
                smooth_first_full: true,
            },
            Event::Frame {
                tag: 3,
                presented_ns: 4,
                render_us: 5,
                missed: 0,
                backdrops: 1,
                gpu_mhz: 520,
                captures: 0,
                uploaded_bytes: 7,
                quality: ladder::RUNGS,
            },
        ];
        let mut wire = Vec::new();
        for event in &events {
            write_frame(&mut wire, event).unwrap();
        }
        let mut input = wire.as_slice();
        let mut buf = Vec::new();
        for event in &events {
            assert_eq!(
                read_frame::<Event>(&mut input, &mut buf).unwrap().as_ref(),
                Some(event)
            );
        }
    }

    #[test]
    fn edges_say_what_they_are() {
        assert!(Edges::NONE.is_none() && Edges::default().is_none());
        assert!(!Edges::vertical(1.0).is_none());
        assert_eq!(Edges::horizontal(5.0), Edges::new(5.0, 5.0, 0.0, 0.0));
        assert_eq!(Edges::vertical(5.0), Edges::new(0.0, 0.0, 5.0, 5.0));
        assert_eq!(Edges::all(2.0).scaled(1.5), Edges::all(3.0));
        assert!(Edges::all(2.0).is_valid());
        for bad in [-1.0, f32::NAN, f32::INFINITY] {
            assert!(!Edges::new(0.0, 0.0, bad, 0.0).is_valid(), "{bad}");
        }
    }

    #[test]
    fn a_corrupt_length_is_refused_before_allocating() {
        let mut wire = (MAX_FRAME + 1).to_le_bytes().to_vec();
        wire.extend_from_slice(&[0; 16]);
        let err = read_frame::<Request>(&mut wire.as_slice(), &mut Vec::new()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_truncated_frame_is_an_error_not_a_clean_end() {
        let mut wire = Vec::new();
        write_frame(&mut wire, &Request::Commit { tag: 1 }).unwrap();
        wire.pop();
        assert!(read_frame::<Request>(&mut wire.as_slice(), &mut Vec::new()).is_err());
    }

    fn triangle() -> MeshData {
        let v = |x: f32, y: f32| MeshVertex {
            position: [x, y, -1.5],
            normal: [0.0, 0.6, -0.8],
            colour: Rgba(10, 20, 30, 255),
        };
        MeshData {
            vertices: vec![v(0.0, 0.0), v(1.0, 0.0), v(0.0, 1.0)],
            indices: vec![0, 1, 2],
        }
    }

    #[test]
    fn a_mesh_survives_its_file_form() {
        let mesh = triangle();
        let bytes = mesh.encode().unwrap();
        assert_eq!(bytes.len(), 16 + 3 * MESH_VERTEX_BYTES + 3 * 2);
        let back = MeshData::decode(&bytes).unwrap();
        assert_eq!(back.indices, mesh.indices);
        for (a, b) in back.vertices.iter().zip(&mesh.vertices) {
            assert_eq!((a.position, a.colour), (b.position, b.colour));
            // Normals travel as signed bytes: within half a step of 1/127.
            for (x, y) in a.normal.iter().zip(b.normal) {
                assert!((x - y).abs() <= 0.5 / 127.0 + 1e-6, "{x} vs {y}");
            }
        }
    }

    #[test]
    fn meshes_that_cannot_be_drawn_are_refused_on_both_sides() {
        let mut past_the_end = triangle();
        past_the_end.indices[2] = 3;
        assert!(past_the_end.encode().is_err());
        let mut half_a_triangle = triangle();
        half_a_triangle.indices.pop();
        assert!(half_a_triangle.encode().is_err());
        let mut not_a_number = triangle();
        not_a_number.vertices[1].position[0] = f32::NAN;
        assert!(not_a_number.encode().is_err());

        let good = triangle().encode().unwrap();
        assert!(
            MeshData::decode(&good[..good.len() - 1]).is_err(),
            "truncated"
        );
        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert!(MeshData::decode(&bad_magic).is_err());
        let mut bad_index = good.clone();
        let last = bad_index.len() - 2;
        bad_index[last..].copy_from_slice(&7u16.to_le_bytes());
        assert!(
            MeshData::decode(&bad_index).is_err(),
            "an index past the vertices"
        );
    }

    #[test]
    fn only_the_image_wallpapers_ask_the_app_for_pixels() {
        assert!(WallpaperKind::Silk.uses_image());
        assert!(WallpaperKind::Liquid.uses_image());
        assert!(!WallpaperKind::Starfield.uses_image());
        assert!(!WallpaperKind::Weather(Weather::Rain).uses_image());
        assert!(
            !WallpaperKind::Fireflies {
                colour: Rgba(1, 1, 1, 1)
            }
            .uses_image()
        );
    }

    #[test]
    fn the_v7_messages_survive_a_round_trip_and_stay_small() {
        let requests = vec![
            Request::GroupCreate {
                id: GroupId(1),
                parent: None,
                pose: GroupPose {
                    clip: Some(Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 10.0,
                        h: 10.0,
                    }),
                    ..GroupPose::at(3.0, 4.0)
                },
            },
            Request::GroupBind {
                id: GroupId(1),
                motion: MotionId(2),
                binding: PoseBinding {
                    values: [0.0, 1.0],
                    from: GroupPose::IDENTITY,
                    to: GroupPose::at(0.0, -1.0),
                    properties: PoseProperties::TRANSLATION.union(PoseProperties::OPACITY),
                },
            },
            Request::GroupParent {
                id: GroupId(1),
                parent: Some(GroupId(3)),
                preserve_world: true,
            },
            Request::Attach {
                id: LayerId(5),
                group: Some(GroupId(1)),
                local: Props::at(1.0, 2.0),
            },
            Request::MotionCreate {
                id: MotionId(2),
                value: 0.0,
                bounds: MotionBounds {
                    ratio: -1.0,
                    ..MotionBounds::range(0.0, 480.0, Units::Pixels)
                },
            },
            Request::MotionDrive {
                id: MotionId(2),
                sample: DriveSample {
                    contact: 1,
                    phase: DrivePhase::Move,
                    position: 12.5,
                    time_ns: 99,
                },
            },
            Request::MotionSettle {
                id: MotionId(2),
                target: None,
                velocity: Some(-900.0),
                settle: Settle::Scroll(ScrollToken {
                    friction: 4.0,
                    spring: SpringToken { response_ms: 300.0 },
                }),
            },
            Request::MotionSettle {
                id: MotionId(2),
                target: Some(1.0),
                velocity: None,
                settle: Settle::TimedAfter {
                    delay_ms: 130,
                    token: MotionToken {
                        duration_ms: 280,
                        ease: Ease::Out,
                    },
                },
            },
            Request::MotionCancel {
                id: MotionId(2),
                policy: CancelPolicy::Hold,
            },
            Request::BackdropCreate {
                id: BackdropId(1),
                spec: CaptureSpec {
                    wallpaper: Some(LayerId(1)),
                    plates: vec![PlateSource { group: GroupId(1) }],
                    radius: 30.0,
                    downsample: 4,
                },
            },
            Request::BackdropRefresh {
                id: BackdropId(1),
                content: 77,
            },
            Request::BackdropFreeze { id: BackdropId(1) },
            Request::AtlasCreate {
                id: AtlasId(1),
                width: 1024,
                height: 1024,
            },
            Request::AtlasUpload {
                id: AtlasId(1),
                slot: 1,
                generation: 4,
                rect: AtlasRect {
                    x: 0,
                    y: 8,
                    w: 512,
                    h: 512,
                },
            },
            Request::SetGlyphs {
                id: LayerId(6),
                after: 4,
                glyphs: vec![
                    Glyph {
                        x: 2.0,
                        y: 3.0,
                        sx: 10,
                        sy: 11,
                        w: 20,
                        h: 30,
                        ink: 2,
                    };
                    5
                ],
            },
            Request::Create {
                id: LayerId(6),
                width: 100,
                height: 40,
                z: 1,
                content: Content::Glyphs(GlyphStyle {
                    atlas: AtlasId(1),
                    palette: [
                        Rgba(255, 255, 255, 255),
                        Rgba(160, 160, 170, 255),
                        Rgba(0, 0, 0, 0),
                        Rgba(12, 34, 56, 78),
                    ],
                }),
                props: Props::at(0.0, 0.0),
            },
        ];
        for request in &requests {
            let mut wire = Vec::new();
            write_frame(&mut wire, request).unwrap();
            let back: Request = read_frame(&mut wire.as_slice(), &mut Vec::new())
                .unwrap()
                .unwrap();
            assert_eq!(&back, request);
        }
        // A drive sample is what crosses the socket sixty times a second: it must stay tiny.
        let mut wire = Vec::new();
        write_frame(&mut wire, &requests[5]).unwrap();
        assert!(wire.len() <= 32, "{} bytes", wire.len());
        // A full glyph run fits one frame with room to spare.
        let full = Request::SetGlyphs {
            id: LayerId(1),
            after: u64::MAX,
            glyphs: vec![
                Glyph {
                    x: 1.0,
                    y: 1.0,
                    sx: u16::MAX,
                    sy: u16::MAX,
                    w: u16::MAX,
                    h: u16::MAX,
                    ink: resources::INKS as u8 - 1,
                };
                resources::MAX_RUN_GLYPHS
            ],
        };
        let mut wire = Vec::new();
        write_frame(&mut wire, &full).unwrap();
        assert!(wire.len() < MAX_FRAME as usize / 8, "{} bytes", wire.len());
        let events = vec![
            Event::Motion {
                id: MotionId(2),
                generation: 3,
                kind: motion::NoticeKind::Caught,
                value: 10.0,
                velocity: -5.0,
                time_ns: 1,
            },
            Event::BackdropReady {
                id: BackdropId(1),
                generation: 2,
                content: 9,
            },
            Event::BackdropFrozen {
                id: BackdropId(1),
                generation: 2,
            },
            Event::AtlasUploaded {
                id: AtlasId(1),
                slot: 0,
                generation: 4,
            },
        ];
        for event in &events {
            let mut wire = Vec::new();
            write_frame(&mut wire, event).unwrap();
            let back: Event = read_frame(&mut wire.as_slice(), &mut Vec::new())
                .unwrap()
                .unwrap();
            assert_eq!(&back, event);
        }
    }
}
