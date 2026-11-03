//! One client connection's side of the protocol: it checks every request against the limits, mirrors what the
//! client has created (so a request naming a layer that does not exist is refused on the spot) and stages the
//! changes of a batch until the client commits it. It knows nothing of sockets or the GPU.

use crate::geometry::clamp_damage;
use crate::ops::{Op, Transaction};
use crate::shm::{ImageFile, read_limited};
use crate::stats::SharedStats;
use compositor_protocol::{
    Backdrop, Content, Depth, Edges, Event, Fill, Glass, LayerId, MAX_MESH_INDICES,
    MAX_MESH_VERTICES, MESH_VERTEX_BYTES, Mesh, MeshData, PROTOCOL_VERSION, Props, Rect, Request,
    Shadow, Shape, Sheen, Wallpaper, WallpaperKind, ideas, image_path, mesh_path,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

mod v7;

/// What a client may ask for. The device has little memory: these stop a buggy or hostile client from using it up.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub layers: usize,
    /// Largest width or height of a layer.
    pub dimension: u32,
    /// Pixel memory all image layers (and the pictures of image wallpapers) together may hold.
    pub image_bytes: u64,
    /// Geometry all mesh layers together may hold, as the bytes of their files (the GPU keeps a copy of the same size).
    pub mesh_bytes: u64,
    pub ops_per_batch: usize,
}

impl Limits {
    /// The limits for a GPU whose largest texture is `max_texture` pixels across. OpenGL ES 3 guarantees 2048, so
    /// a driver that says less is not believed.
    pub fn for_texture_size(max_texture: u32) -> Self {
        Self {
            layers: 256,
            dimension: max_texture.clamp(2048, 4096),
            image_bytes: 64 << 20,
            mesh_bytes: 16 << 20,
            ops_per_batch: 8192,
        }
    }
}

pub struct Config {
    pub limits: Limits,
    pub panel_width: u32,
    pub panel_height: u32,
    pub refresh_hz: f32,
    /// The directory the layers' pixel files live in.
    pub shm_dir: PathBuf,
    /// The clocks the GPU can be held at, kHz (the profile's `gpu.opp_khz`); empty where the device has no clock control.
    pub gpu_opp_khz: Vec<u32>,
}

impl Config {
    /// The shared-memory file of a layer: the protocol names it, only the directory is ours to move (tests do).
    fn in_shm_dir(&self, name: String) -> PathBuf {
        let file = std::path::Path::new(&name)
            .file_name()
            .expect("the protocol names a file");
        self.shm_dir.join(file)
    }

    fn image_path(&self, id: LayerId) -> PathBuf {
        self.in_shm_dir(image_path(id))
    }

    fn mesh_path(&self, id: LayerId) -> PathBuf {
        self.in_shm_dir(mesh_path(id))
    }
}

/// Scale beyond which a layer is not worth drawing.
const MAX_SCALE: f32 = 64.0;
/// Positions and sizes beyond this many pixels are mistakes, and would lose precision in the vertex math.
const MAX_COORD: f32 = 1.0e6;
/// The widest a backdrop blur may be (the standard deviation of the Gaussian, in logical pixels). Lucent's
/// `blur.surface` is 30.
const MAX_BLUR_RADIUS: f32 = 64.0;
/// Soft shadows: Lucent's biggest (`shadow.raised`) is 80 wide and 34 down; a quad grown by more than this is a
/// mistake that would cost a screenful of fill.
const MAX_SHADOW_BLUR: f32 = 128.0;
const MAX_SHADOW_OFFSET: f32 = 128.0;
/// Neumorphic depth is 2 offset and 4 blur in Lucent.
const MAX_DEPTH_OFFSET: f32 = 16.0;
const MAX_DEPTH_BLUR: f32 = 32.0;
const MAX_EDGE_WIDTH: f32 = 64.0;
const MAX_SATURATION: f32 = 4.0;
const MAX_REFRACTION: f32 = 64.0;
/// How much faster than designed a wallpaper may be asked to move.
const MAX_WALLPAPER_SPEED: f32 = 8.0;
/// The camera's vertical field of view, in degrees.
const FOV_RANGE: (f32, f32) = (1.0, 170.0);
/// A camera that turns faster than this is a mistake, not a look.
const MAX_ORBIT_DEG_PER_S: f32 = 720.0;

/// What handling one request produced.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Events for the client, in order.
    pub replies: Vec<Event>,
    /// A commit's batch, ready for the render thread.
    pub commit: Option<Transaction>,
    /// The connection cannot continue.
    pub close: bool,
}

struct Refusal {
    message: String,
    fatal: bool,
}

impl Refusal {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fatal: false,
        }
    }

    fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            fatal: true,
        }
    }
}

impl From<String> for Refusal {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

enum Done {
    Nothing,
    Reply(Event),
    /// A batch for the render thread: a commit's changes, or a request that acts on its own.
    Batch(Transaction),
}

struct Known {
    width: u32,
    height: u32,
    /// What the layer is now: its latest `Create` or `Restyle`. Restyle, Damage and Ripple are checked against it.
    content: Content,
    /// Whether the layer has a picture in a shared-memory file (an image, or a wallpaper of a kind that uses one).
    image: bool,
    /// The size of the mesh file the layer was created from, for the memory limit; 0 for the other kinds.
    mesh_bytes: u64,
    /// The group the layer hangs under.
    group: Option<compositor_protocol::GroupId>,
}

pub struct Session {
    config: Arc<Config>,
    stats: Arc<SharedStats>,
    client: u64,
    greeted: bool,
    /// The layers this client has created so far, including those in the batch being staged.
    layers: HashMap<LayerId, Known>,
    image_bytes: u64,
    mesh_bytes: u64,
    batch: Vec<Op>,
    /// Groups, tracks, prepared backdrops and atlases (`v7`).
    v7: v7::Mirror,
}

impl Session {
    pub fn new(config: Arc<Config>, stats: Arc<SharedStats>, client: u64) -> Self {
        Self {
            config,
            stats,
            client,
            greeted: false,
            layers: HashMap::new(),
            image_bytes: 0,
            mesh_bytes: 0,
            batch: Vec::new(),
            v7: v7::Mirror::default(),
        }
    }

    pub fn client(&self) -> u64 {
        self.client
    }

    /// The change that removes everything this client showed, for when it goes away.
    pub fn goodbye(&self) -> Transaction {
        Transaction {
            client: self.client,
            tag: None,
            ops: vec![Op::Clear],
            received_ns: 0,
        }
    }

    pub fn handle(&mut self, request: Request) -> Outcome {
        let mut outcome = Outcome::default();
        match self.dispatch(request) {
            Ok(Done::Nothing) => {}
            Ok(Done::Reply(event)) => outcome.replies.push(event),
            Ok(Done::Batch(transaction)) => outcome.commit = Some(transaction),
            Err(refusal) => {
                outcome.replies.push(Event::Error {
                    message: refusal.message,
                });
                outcome.close = refusal.fatal;
            }
        }
        outcome
    }

    fn dispatch(&mut self, request: Request) -> Result<Done, Refusal> {
        if !self.greeted {
            return match request {
                Request::Hello { version } if version == PROTOCOL_VERSION => {
                    self.greeted = true;
                    Ok(Done::Reply(Event::Welcome {
                        version: PROTOCOL_VERSION,
                        panel_width: self.config.panel_width,
                        panel_height: self.config.panel_height,
                        refresh_hz: self.config.refresh_hz,
                        smooth_first_full: crate::ladder::available(),
                    }))
                }
                Request::Hello { version } => Err(Refusal::fatal(format!(
                    "protocol version {version} is not supported; this compositor speaks {PROTOCOL_VERSION}"
                ))),
                _ => Err(Refusal::fatal("the first request must be Hello")),
            };
        }
        if !matches!(
            request,
            Request::Hello { .. }
                | Request::Commit { .. }
                | Request::HoldGpuClock { .. }
                | Request::Ideas { .. }
                | Request::SmoothFirst { .. }
                | Request::Display { .. }
                | Request::Stats
        ) {
            self.room()?;
        }
        match request {
            Request::Hello { .. } => Err(Refusal::new("Hello was already sent")),
            Request::Create {
                id,
                width,
                height,
                z,
                content,
                props,
            } => self.create(id, width, height, z, content, props),
            Request::Damage { id, rect } => {
                let known = self.known(id)?;
                if !known.image {
                    return Err(Refusal::new(format!(
                        "layer {} is {}, which has no picture to damage",
                        id.0,
                        describe(&known.content)
                    )));
                }
                let rect = check_rect("damage rectangle", rect)?;
                if let Some(rect) = clamp_damage(rect, known.width, known.height) {
                    self.batch.push(Op::Damage { id, rect });
                }
                Ok(Done::Nothing)
            }
            Request::Restyle { id, content } => {
                let old = self.known(id)?.content;
                let content = check_restyle(id, &old, content, self.config.refresh_hz)?;
                self.check_references(&content)?;
                self.batch.push(Op::Restyle { id, content });
                if let Some(known) = self.layers.get_mut(&id) {
                    known.content = content;
                }
                Ok(Done::Nothing)
            }
            Request::Ripple { id, x, y } => {
                let known = self.known(id)?;
                if !matches!(
                    known.content,
                    Content::Wallpaper(Wallpaper {
                        kind: WallpaperKind::Liquid,
                        ..
                    })
                ) {
                    return Err(Refusal::new(format!(
                        "layer {} is {}, and only a Liquid wallpaper ripples",
                        id.0,
                        describe(&known.content)
                    )));
                }
                let x = coordinate("ripple x", x)?;
                let y = coordinate("ripple y", y)?;
                self.batch.push(Op::Ripple { id, x, y });
                Ok(Done::Nothing)
            }
            Request::Set { id, props } => {
                self.known(id)?;
                let props = check_props(props)?;
                self.batch.push(Op::Set { id, props });
                Ok(Done::Nothing)
            }
            Request::Animate {
                id,
                to,
                duration_ms,
                ease,
            } => {
                self.known(id)?;
                let to = check_props(to)?;
                self.batch.push(Op::Animate {
                    id,
                    to,
                    duration_ms,
                    ease,
                });
                Ok(Done::Nothing)
            }
            Request::Delete { id } => {
                // Deleting what is not there is fine: an app deletes on drop, possibly after a refused Create.
                if let Some(known) = self.layers.remove(&id) {
                    if known.image {
                        self.image_bytes -= image_size(known.width, known.height);
                    }
                    self.mesh_bytes -= known.mesh_bytes;
                    self.batch.push(Op::Delete { id });
                }
                Ok(Done::Nothing)
            }
            Request::Commit { tag } => {
                self.v7.committed();
                Ok(Done::Batch(Transaction {
                    client: self.client,
                    tag: Some(tag),
                    ops: std::mem::take(&mut self.batch),
                    received_ns: 0,
                }))
            }
            Request::HoldGpuClock { mhz } => {
                let khz = mhz.map(|mhz| self.gpu_clock(mhz)).transpose()?;
                // At once and on its own: the clock is no part of a frame, and the request must not wait for (or
                // take with it) the changes staged for the next commit.
                Ok(Done::Batch(Transaction {
                    client: self.client,
                    tag: None,
                    ops: vec![Op::HoldGpuClock(khz)],
                    received_ns: 0,
                }))
            }
            Request::Ideas { mask } => {
                if mask & !ideas::ALL != 0 {
                    return Err(Refusal::new(format!(
                        "{mask:#x} names an idea there is not"
                    )));
                }
                Ok(Done::Batch(Transaction {
                    client: self.client,
                    tag: None,
                    ops: vec![Op::Ideas(mask)],
                    received_ns: 0,
                }))
            }
            // At once and on its own, like the glass ideas it replaces. A level the compositor cannot carry out is refused, so that
            // Settings never shows a level that does nothing (`Event::Welcome::smooth_first_full` says which there are).
            Request::SmoothFirst { level } => {
                if let Some(why) = crate::ladder::refusal(level, crate::ladder::available()) {
                    return Err(Refusal::new(why));
                }
                Ok(Done::Batch(Transaction {
                    client: self.client,
                    tag: None,
                    ops: vec![Op::SmoothFirst(level)],
                    received_ns: 0,
                }))
            }
            // At once and on its own, like a hold of the GPU's clock: whether anyone can see the screen is no part of a
            // frame.
            Request::Display { on } => Ok(Done::Batch(Transaction {
                client: self.client,
                tag: None,
                ops: vec![Op::Display(on)],
                received_ns: 0,
            })),
            Request::Stats => Ok(Done::Reply(Event::Stats(self.stats.snapshot()))),
            request @ (Request::GroupCreate { .. }
            | Request::GroupSet { .. }
            | Request::GroupParent { .. }
            | Request::GroupBind { .. }
            | Request::GroupUnbind { .. }
            | Request::GroupDelete { .. }
            | Request::Attach { .. }
            | Request::MotionCreate { .. }
            | Request::MotionDelete { .. }
            | Request::MotionDrive { .. }
            | Request::MotionSettle { .. }
            | Request::MotionCancel { .. }
            | Request::BackdropCreate { .. }
            | Request::BackdropRefresh { .. }
            | Request::BackdropFreeze { .. }
            | Request::BackdropRelease { .. }
            | Request::BackdropDelete { .. }
            | Request::AtlasCreate { .. }
            | Request::AtlasUpload { .. }
            | Request::AtlasDelete { .. }
            | Request::SetGlyphs { .. }) => self.dispatch_v7(request),
        }
    }

    /// `mhz` as one of the clocks the GPU offers, in kHz.
    fn gpu_clock(&self, mhz: u32) -> Result<u32, Refusal> {
        let offered = &self.config.gpu_opp_khz;
        if offered.is_empty() {
            return Err(Refusal::new("this device has no GPU clock control"));
        }
        mhz.checked_mul(1000)
            .filter(|khz| offered.contains(khz))
            .ok_or_else(|| {
                let list: Vec<String> =
                    offered.iter().map(|khz| (khz / 1000).to_string()).collect();
                Refusal::new(format!(
                    "{mhz} MHz is not one of this GPU's clocks ({} MHz)",
                    list.join(", ")
                ))
            })
    }

    fn known(&self, id: LayerId) -> Result<&Known, Refusal> {
        self.layers
            .get(&id)
            .ok_or_else(|| Refusal::new(format!("layer {} does not exist", id.0)))
    }

    /// Refuses a request that would stage a change once a batch has grown past its limit, before anything is
    /// recorded, so the mirror of the client's layers never runs ahead of the batch.
    fn room(&self) -> Result<(), Refusal> {
        if self.batch.len() >= self.config.limits.ops_per_batch {
            return Err(Refusal::new(format!(
                "more than {} requests without a Commit",
                self.config.limits.ops_per_batch
            )));
        }
        Ok(())
    }

    fn create(
        &mut self,
        id: LayerId,
        width: u32,
        height: u32,
        z: i32,
        content: Content,
        props: Props,
    ) -> Result<Done, Refusal> {
        let limits = self.config.limits;
        if width == 0 || height == 0 || width > limits.dimension || height > limits.dimension {
            return Err(Refusal::new(format!(
                "a {width}x{height} layer is outside 1..={} pixels each way",
                limits.dimension
            )));
        }
        let replaced = self.layers.get(&id);
        if replaced.is_none() && self.layers.len() >= limits.layers {
            return Err(Refusal::new(format!("more than {} layers", limits.layers)));
        }
        // What a layer this one replaces gives back before the new one is counted.
        let freed_image = replaced
            .filter(|k| k.image)
            .map_or(0, |k| image_size(k.width, k.height));
        let freed_mesh = replaced.map_or(0, |k| k.mesh_bytes);
        let props = check_props(props)?;
        let content = check_content(content, self.config.refresh_hz)?;
        self.check_references(&content)?;

        let mut image = None;
        if has_picture(&content) {
            if self.image_bytes - freed_image + image_size(width, height) > limits.image_bytes {
                return Err(Refusal::new(format!(
                    "image layers would use more than {} MiB",
                    limits.image_bytes >> 20
                )));
            }
            image = Some(Arc::new(ImageFile::open(
                &self.config.image_path(id),
                width,
                height,
            )?));
        }
        let mut mesh = None;
        let mut mesh_bytes = 0;
        if matches!(content, Content::Mesh(_)) {
            let room = limits
                .mesh_bytes
                .saturating_sub(self.mesh_bytes - freed_mesh);
            let path = self.config.mesh_path(id);
            let bytes = read_limited(&path, room.min(MAX_MESH_FILE))
                .map_err(|e| format!("mesh of layer {}: {e}", id.0))?;
            let data = MeshData::decode(&bytes)
                .map_err(|e| format!("mesh of layer {} ({}): {e}", id.0, path.display()))?;
            mesh_bytes = bytes.len() as u64;
            mesh = Some(Arc::new(data));
        }

        if let Some(old) = self.layers.remove(&id) {
            if old.image {
                self.image_bytes -= image_size(old.width, old.height);
            }
            self.mesh_bytes -= old.mesh_bytes;
        }
        if image.is_some() {
            self.image_bytes += image_size(width, height);
        }
        self.mesh_bytes += mesh_bytes;
        self.layers.insert(
            id,
            Known {
                width,
                height,
                content,
                image: image.is_some(),
                mesh_bytes,
                group: None,
            },
        );
        self.batch.push(Op::Create {
            id,
            width,
            height,
            z,
            content,
            props,
            image,
            mesh,
        });
        Ok(Done::Nothing)
    }
}

/// The largest a mesh file can be: the header, the most vertices and the most indices the format allows.
const MAX_MESH_FILE: u64 =
    (16 + MAX_MESH_VERTICES * MESH_VERTEX_BYTES + MAX_MESH_INDICES * 2) as u64;

fn image_size(width: u32, height: u32) -> u64 {
    u64::from(width) * u64::from(height) * 4
}

/// Whether the layer's pixels are in a shared-memory file the app writes: an image, or a wallpaper of a kind that
/// warps a picture.
fn has_picture(content: &Content) -> bool {
    match content {
        Content::Image => true,
        Content::Wallpaper(w) => w.kind.uses_image(),
        Content::Shape(_)
        | Content::Glass(_)
        | Content::Mesh(_)
        | Content::Glyphs(_)
        | Content::Live(_) => false,
    }
}

fn describe(content: &Content) -> &'static str {
    match content {
        Content::Image => "an image",
        Content::Shape(_) => "a shape",
        Content::Glass(_) => "a glass surface",
        Content::Wallpaper(_) => "a wallpaper",
        Content::Mesh(_) => "a mesh",
        Content::Glyphs(_) => "a glyph run",
        Content::Live(_) => "a live layer",
    }
}

fn coordinate(name: &str, value: f32) -> Result<f32, String> {
    if !value.is_finite() {
        Err(format!("{name} is not a number"))
    } else if value.abs() > MAX_COORD {
        Err(format!("{name} is beyond {MAX_COORD} pixels"))
    } else {
        Ok(value)
    }
}

/// A number that must lie in `lo..=hi`. One a hair outside (the app's arithmetic rounding: 0.7 + 0.3 can come out
/// over 1) is pulled in; anything further out is refused.
fn ranged(name: &str, value: f32, lo: f32, hi: f32) -> Result<f32, String> {
    if !value.is_finite() {
        return Err(format!("{name} is not a number"));
    }
    let slack = (hi - lo) * 1.0e-3;
    if value < lo - slack || value > hi + slack {
        return Err(format!("{name} {value} is outside {lo}..={hi}"));
    }
    Ok(value.clamp(lo, hi))
}

fn vector(name: &str, v: [f32; 3]) -> Result<[f32; 3], String> {
    Ok([
        coordinate(name, v[0])?,
        coordinate(name, v[1])?,
        coordinate(name, v[2])?,
    ])
}

fn check_rect(name: &str, rect: Rect) -> Result<Rect, String> {
    Ok(Rect {
        x: coordinate(name, rect.x)?,
        y: coordinate(name, rect.y)?,
        w: coordinate(name, rect.w)?,
        h: coordinate(name, rect.h)?,
    })
}

/// The widths of a clip's feather: each a number and not beyond the coordinates' bounds; one a hair under zero (an animation's
/// rounding) is a hard edge, as a clip size is. A feather that belongs to no clip is dropped, so that what the compositor holds
/// is what it draws.
fn check_feather(name: &str, feather: Edges, clipped: bool) -> Result<Edges, String> {
    if !clipped {
        return Ok(Edges::NONE);
    }
    let width = |side: &str, w: f32| coordinate(&format!("{name} {side}"), w).map(|w| w.max(0.0));
    Ok(Edges {
        left: width("left", feather.left)?,
        right: width("right", feather.right)?,
        top: width("top", feather.top)?,
        bottom: width("bottom", feather.bottom)?,
    })
}

/// Rejects numbers that are not numbers or are absurd; scale and opacity that drift a little out of range (a
/// fade computed as 0.7 + 0.3 can come out a hair over 1) are pulled back in.
fn check_props(props: Props) -> Result<Props, String> {
    Ok(Props {
        x: coordinate("x", props.x)?,
        y: coordinate("y", props.y)?,
        scale: coordinate("scale", props.scale)?.clamp(0.0, MAX_SCALE),
        opacity: coordinate("opacity", props.opacity)?.clamp(0.0, 1.0),
        clip: props
            .clip
            .map(|clip| check_rect("clip", clip))
            .transpose()?,
        feather: check_feather("clip feather", props.feather, props.clip.is_some())?,
        visible: props.visible,
    })
}

fn check_shape(shape: Shape) -> Result<Shape, String> {
    if let Fill::Linear { angle_deg, .. } = shape.fill {
        coordinate("gradient angle", angle_deg)?;
    }
    Ok(Shape {
        corner_radius: coordinate("corner radius", shape.corner_radius)?.max(0.0),
        border_width: coordinate("border width", shape.border_width)?.max(0.0),
        ..shape
    })
}

fn check_sheen(sheen: Sheen) -> Result<Sheen, String> {
    let sheen = Sheen {
        peak: ranged("sheen peak", sheen.peak, 0.0, 1.0)?,
        floor: ranged("sheen floor", sheen.floor, 0.0, 1.0)?,
        start: ranged("sheen start", sheen.start, 0.0, 1.0)?,
        end: ranged("sheen end", sheen.end, 0.0, 1.0)?,
    };
    if sheen.start > sheen.end {
        return Err(format!(
            "the sheen starts at {} but ends at {}",
            sheen.start, sheen.end
        ));
    }
    Ok(sheen)
}

fn check_shadow(shadow: Shadow) -> Result<Shadow, String> {
    Ok(Shadow {
        offset_y: ranged(
            "shadow offset",
            shadow.offset_y,
            -MAX_SHADOW_OFFSET,
            MAX_SHADOW_OFFSET,
        )?,
        blur: ranged("shadow blur", shadow.blur, 0.0, MAX_SHADOW_BLUR)?,
        ..shadow
    })
}

fn check_depth(depth: Depth) -> Result<Depth, String> {
    Ok(Depth {
        offset: ranged("depth offset", depth.offset, 0.0, MAX_DEPTH_OFFSET)?,
        blur: ranged("depth blur", depth.blur, 0.0, MAX_DEPTH_BLUR)?,
        ..depth
    })
}

fn check_backdrop(backdrop: Backdrop) -> Result<Backdrop, String> {
    Ok(match backdrop {
        Backdrop::None => Backdrop::None,
        Backdrop::Cached { radius } => Backdrop::Cached {
            radius: ranged("blur radius", radius, 0.0, MAX_BLUR_RADIUS)?,
        },
        Backdrop::Prepared { id } => Backdrop::Prepared { id },
        Backdrop::Live { radius, downsample } => {
            if !matches!(downsample, 2 | 4 | 8) {
                return Err(format!(
                    "a live backdrop is blurred at 1/2, 1/4 or 1/8 of the screen, not 1/{downsample}"
                ));
            }
            Backdrop::Live {
                radius: ranged("blur radius", radius, 0.0, MAX_BLUR_RADIUS)?,
                downsample,
            }
        }
    })
}

fn check_glass(glass: Glass) -> Result<Glass, String> {
    let mut shadows = [None; 2];
    for (checked, shadow) in shadows.iter_mut().zip(glass.shadows) {
        *checked = shadow.map(check_shadow).transpose()?;
    }
    Ok(Glass {
        corner_radius: coordinate("glass corner radius", glass.corner_radius)?.max(0.0),
        edge_width: coordinate("glass edge width", glass.edge_width)?.clamp(0.0, MAX_EDGE_WIDTH),
        sheen: glass.sheen.map(check_sheen).transpose()?,
        shadows,
        depth: glass.depth.map(check_depth).transpose()?,
        backdrop: check_backdrop(glass.backdrop)?,
        saturation: ranged("saturation", glass.saturation, 0.0, MAX_SATURATION)?,
        refraction: ranged("refraction", glass.refraction, 0.0, MAX_REFRACTION)?,
        edge_lens: glass.edge_lens.map(|lens| -> Result<_, String> {
            Ok(compositor_protocol::EdgeLens {
                bevel: ranged("edge bevel", lens.bevel, 0.0, MAX_REFRACTION)?,
                lens: ranged("edge lens", lens.lens, 0.0, MAX_REFRACTION)?,
                rim_clear: ranged("edge rim clear", lens.rim_clear, 0.0, 1.0)?,
            })
        }).transpose()?,
        ..glass
    })
}

fn check_wallpaper(wallpaper: Wallpaper, refresh_hz: f32) -> Result<Wallpaper, String> {
    // A whole number of frames per second, so 60 is fine on a 59.64 Hz panel.
    if f32::from(wallpaper.fps) > refresh_hz.ceil() {
        return Err(format!(
            "a wallpaper at {} fps is faster than the panel's {refresh_hz:.2} Hz",
            wallpaper.fps
        ));
    }
    Ok(Wallpaper {
        speed: ranged("wallpaper speed", wallpaper.speed, 0.0, MAX_WALLPAPER_SPEED)?,
        intensity: ranged("wallpaper intensity", wallpaper.intensity, 0.0, 1.0)?,
        ..wallpaper
    })
}

fn check_mesh(mesh: Mesh) -> Result<Mesh, String> {
    let eye = vector("camera eye", mesh.eye)?;
    let target = vector("camera target", mesh.target)?;
    let light = vector("light direction", mesh.light)?;
    let length = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let apart = [eye[0] - target[0], eye[1] - target[1], eye[2] - target[2]];
    if length(apart) < 1.0e-4 {
        return Err("the camera is at the point it looks at".into());
    }
    if length(light) < 1.0e-6 {
        return Err("the light has no direction".into());
    }
    Ok(Mesh {
        eye,
        target,
        light,
        fov_deg: ranged("field of view", mesh.fov_deg, FOV_RANGE.0, FOV_RANGE.1)?,
        ambient: ranged("ambient light", mesh.ambient, 0.0, 1.0)?,
        orbit_deg_per_s: ranged(
            "orbit speed",
            mesh.orbit_deg_per_s,
            -MAX_ORBIT_DEG_PER_S,
            MAX_ORBIT_DEG_PER_S,
        )?,
        ..mesh
    })
}

fn check_content(content: Content, refresh_hz: f32) -> Result<Content, String> {
    Ok(match content {
        Content::Image => Content::Image,
        Content::Shape(shape) => Content::Shape(check_shape(shape)?),
        Content::Glass(glass) => Content::Glass(check_glass(glass)?),
        Content::Wallpaper(wallpaper) => {
            Content::Wallpaper(check_wallpaper(wallpaper, refresh_hz)?)
        }
        Content::Mesh(mesh) => Content::Mesh(check_mesh(mesh)?),
        Content::Glyphs(style) => Content::Glyphs(style),
        Content::Live(live) => Content::Live(live),
    })
}

/// What a `Restyle` may turn layer `id`, which is `old` now, into: the same kind, with sane numbers. A wallpaper
/// keeps whether it has a picture, because the picture's file belongs to the layer.
fn check_restyle(
    id: LayerId,
    old: &Content,
    new: Content,
    refresh_hz: f32,
) -> Result<Content, Refusal> {
    if matches!(old, Content::Image | Content::Live(_)) {
        return Err(Refusal::new(format!(
            "layer {} is {}, which has no look to change",
            id.0,
            describe(old)
        )));
    }
    if std::mem::discriminant(old) != std::mem::discriminant(&new) {
        return Err(Refusal::new(format!(
            "layer {} is {}, so a Restyle must give {} (another kind is a new layer)",
            id.0,
            describe(old),
            describe(old)
        )));
    }
    if has_picture(old) != has_picture(&new) {
        return Err(Refusal::new(format!(
            "layer {} is a wallpaper that {} a picture; a Restyle cannot change that (create a new layer)",
            id.0,
            if has_picture(old) { "has" } else { "has no" }
        )));
    }
    Ok(check_content(new, refresh_hz)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use compositor_protocol::{Ease, MeshVertex, Rgba};

    struct Rig {
        dir: TempDir,
        session: Session,
    }

    fn rig() -> Rig {
        rig_with(Limits::for_texture_size(4096))
    }

    fn rig_with(limits: Limits) -> Rig {
        let dir = TempDir::new("session");
        let config = Arc::new(Config {
            limits,
            panel_width: 480,
            panel_height: 960,
            refresh_hz: 59.64,
            shm_dir: dir.path().to_owned(),
            gpu_opp_khz: vec![520_000, 416_000, 299_000],
        });
        let mut session = Session::new(config, Arc::new(SharedStats::default()), 7);
        let hello = session.handle(Request::Hello {
            version: PROTOCOL_VERSION,
        });
        assert!(matches!(hello.replies[..], [Event::Welcome { .. }]));
        Rig { dir, session }
    }

    impl Rig {
        fn image_file(&self, id: u32, width: u32, height: u32) {
            std::fs::write(
                self.dir.path().join(format!("layer-{id}.rgba")),
                vec![0u8; (width * height * 4) as usize],
            )
            .unwrap();
        }

        fn mesh_file(&self, id: u32, data: &MeshData) {
            self.mesh_bytes(id, &data.encode().unwrap());
        }

        fn mesh_bytes(&self, id: u32, bytes: &[u8]) {
            std::fs::write(self.dir.path().join(format!("layer-{id}.mesh")), bytes).unwrap();
        }

        fn send(&mut self, request: Request) -> Outcome {
            self.session.handle(request)
        }

        fn error(&mut self, request: Request) -> String {
            let out = self.send(request);
            assert!(out.commit.is_none());
            match &out.replies[..] {
                [Event::Error { message }] => message.clone(),
                other => panic!("expected one error, got {other:?}"),
            }
        }

        fn ok(&mut self, request: Request) {
            let out = self.send(request);
            assert!(out.replies.is_empty(), "{:?}", out.replies);
            assert!(!out.close);
        }

        fn commit(&mut self, tag: u64) -> Transaction {
            self.send(Request::Commit { tag }).commit.unwrap()
        }
    }

    fn shape() -> Shape {
        Shape {
            corner_radius: 8.0,
            fill: Fill::Solid(Rgba(1, 2, 3, 255)),
            border_width: 1.0,
            border: Rgba(9, 9, 9, 9),
        }
    }

    fn create_shape(id: u32, width: u32, height: u32) -> Request {
        Request::Create {
            id: LayerId(id),
            width,
            height,
            z: 0,
            content: Content::Shape(shape()),
            props: Props::at(0.0, 0.0),
        }
    }

    fn create_image(id: u32, width: u32, height: u32) -> Request {
        Request::Create {
            id: LayerId(id),
            width,
            height,
            z: 0,
            content: Content::Image,
            props: Props::at(0.0, 0.0),
        }
    }

    fn glass() -> Glass {
        Glass { corner_radius: 28.0,
        tint: Rgba(18, 14, 26, 102),
        edge: Rgba(31, 31, 31, 31),
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
            inset: false,
            offset: 2.0,
            blur: 4.0,
            light: Rgba(26, 26, 26, 26),
            shade: Rgba(0, 0, 0, 82),
        }),
        backdrop: Backdrop::Live {
            radius: 30.0,
            downsample: 4,
        },
        saturation: 1.2, refraction: 6.0, edge_lens: None, tint_gradient: None, }
    }

    fn wallpaper(kind: WallpaperKind) -> Wallpaper {
        Wallpaper {
            kind,
            speed: 1.0,
            intensity: 0.5,
            fps: 30,
        }
    }

    fn mesh() -> Mesh {
        Mesh {
            eye: [4.0, 5.0, 6.0],
            target: [0.0, 0.0, 0.0],
            fov_deg: 40.0,
            light: [0.3, 1.0, 0.2],
            ambient: 0.35,
            orbit_deg_per_s: 6.0,
            antialias: true,
            background: Rgba(0, 0, 0, 0),
        }
    }

    /// One triangle: 3 vertices and 3 indices, 82 bytes as a file.
    fn mesh_data() -> MeshData {
        let vertex = |x: f32, y: f32| MeshVertex {
            position: [x, y, 0.0],
            normal: [0.0, 0.0, 1.0],
            colour: Rgba(200, 100, 50, 255),
        };
        MeshData {
            vertices: vec![vertex(0.0, 0.0), vertex(1.0, 0.0), vertex(0.0, 1.0)],
            indices: vec![0, 1, 2],
        }
    }

    fn create_content(id: u32, width: u32, height: u32, content: Content) -> Request {
        Request::Create {
            id: LayerId(id),
            width,
            height,
            z: 0,
            content,
            props: Props::at(0.0, 0.0),
        }
    }

    fn restyle(id: u32, content: Content) -> Request {
        Request::Restyle {
            id: LayerId(id),
            content,
        }
    }

    fn ripple(id: u32, x: f32, y: f32) -> Request {
        Request::Ripple {
            id: LayerId(id),
            x,
            y,
        }
    }

    fn damage_all(id: u32) -> Request {
        Request::Damage {
            id: LayerId(id),
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
        }
    }

    #[test]
    fn hello_is_answered_with_the_panel_and_a_wrong_version_is_refused_and_ends_the_connection() {
        let dir = TempDir::new("hello");
        let config = Arc::new(Config {
            limits: Limits::for_texture_size(2048),
            panel_width: 480,
            panel_height: 960,
            refresh_hz: 59.64,
            shm_dir: dir.path().to_owned(),
            gpu_opp_khz: Vec::new(),
        });
        let mut s = Session::new(config.clone(), Arc::default(), 1);
        let out = s.handle(Request::Hello {
            version: PROTOCOL_VERSION,
        });
        assert_eq!(
            out.replies,
            vec![Event::Welcome {
                version: PROTOCOL_VERSION,
                panel_width: 480,
                panel_height: 960,
                refresh_hz: 59.64,
                smooth_first_full: true,
            }]
        );
        assert!(!out.close);

        let mut s = Session::new(config.clone(), Arc::default(), 2);
        let out = s.handle(Request::Hello {
            version: PROTOCOL_VERSION + 1,
        });
        assert!(
            matches!(&out.replies[..], [Event::Error { message }] if message.contains("version"))
        );
        assert!(out.close);

        let mut s = Session::new(config, Arc::default(), 3);
        let out = s.handle(Request::Commit { tag: 1 });
        assert!(
            matches!(&out.replies[..], [Event::Error { message }] if message.contains("Hello"))
        );
        assert!(out.close && out.commit.is_none());
    }

    #[test]
    fn a_second_hello_is_refused_but_the_connection_carries_on() {
        let mut r = rig();
        let out = r.send(Request::Hello {
            version: PROTOCOL_VERSION,
        });
        assert!(matches!(&out.replies[..], [Event::Error { .. }]));
        assert!(!out.close);
        r.ok(create_shape(1, 10, 10));
    }

    #[test]
    fn a_batch_reaches_the_render_thread_whole_and_in_order_only_at_commit() {
        let mut r = rig();
        r.ok(create_shape(1, 100, 50));
        r.ok(Request::Set {
            id: LayerId(1),
            props: Props::at(5.0, 6.0),
        });
        r.ok(Request::Animate {
            id: LayerId(1),
            to: Props::at(9.0, 6.0),
            duration_ms: 200,
            ease: Ease::Out,
        });
        let t = r.commit(4);
        assert_eq!((t.client, t.tag), (7, Some(4)));
        assert!(matches!(
            &t.ops[..],
            [
                Op::Create {
                    width: 100,
                    height: 50,
                    ..
                },
                Op::Set { .. },
                Op::Animate {
                    duration_ms: 200,
                    ..
                }
            ]
        ));
        // Nothing is left over for the next commit.
        assert!(r.commit(5).ops.is_empty());
    }

    #[test]
    fn requests_naming_layers_that_do_not_exist_are_refused() {
        let mut r = rig();
        for request in [
            Request::Set {
                id: LayerId(3),
                props: Props::at(0.0, 0.0),
            },
            Request::Animate {
                id: LayerId(3),
                to: Props::at(0.0, 0.0),
                duration_ms: 10,
                ease: Ease::Linear,
            },
            restyle(3, Content::Shape(shape())),
            ripple(3, 1.0, 1.0),
            Request::Damage {
                id: LayerId(3),
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
            },
        ] {
            assert!(r.error(request).contains("does not exist"));
        }
        assert!(r.commit(1).ops.is_empty());
    }

    #[test]
    fn deleting_forgets_the_layer_and_deleting_twice_is_harmless() {
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        r.ok(Request::Delete { id: LayerId(1) });
        r.ok(Request::Delete { id: LayerId(1) });
        r.ok(Request::Delete { id: LayerId(2) });
        assert!(
            r.error(Request::Set {
                id: LayerId(1),
                props: Props::at(0.0, 0.0)
            })
            .contains("does not exist")
        );
        let t = r.commit(1);
        assert!(matches!(&t.ops[..], [Op::Create { .. }, Op::Delete { .. }]));
    }

    #[test]
    fn the_size_limit_follows_the_gpu_within_what_es3_guarantees_and_the_device_can_afford() {
        assert_eq!(Limits::for_texture_size(0).dimension, 2048);
        assert_eq!(Limits::for_texture_size(3000).dimension, 3000);
        assert_eq!(Limits::for_texture_size(8192).dimension, 4096);
    }

    #[test]
    fn layer_sizes_outside_the_limits_are_refused() {
        let mut r = rig_with(Limits {
            dimension: 1024,
            ..Limits::for_texture_size(4096)
        });
        for (w, h) in [(0, 10), (10, 0), (1025, 10), (10, 1025), (u32::MAX, 1)] {
            assert!(
                r.error(create_shape(1, w, h)).contains("outside"),
                "{w}x{h}"
            );
        }
        r.ok(create_shape(1, 1024, 1024));
    }

    #[test]
    fn numbers_that_are_not_numbers_are_refused_and_drifting_ones_are_pulled_in() {
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        let bad = |f: fn(&mut Props)| {
            let mut p = Props::at(0.0, 0.0);
            f(&mut p);
            Request::Set {
                id: LayerId(1),
                props: p,
            }
        };
        for request in [
            bad(|p| p.x = f32::NAN),
            bad(|p| p.y = f32::INFINITY),
            bad(|p| p.scale = f32::NEG_INFINITY),
            bad(|p| p.opacity = f32::NAN),
            bad(|p| p.x = 1e9),
            bad(|p| {
                p.clip = Some(Rect {
                    x: 0.0,
                    y: 0.0,
                    w: f32::NAN,
                    h: 1.0,
                })
            }),
        ] {
            let message = r.error(request);
            assert!(
                message.contains("not a number") || message.contains("beyond"),
                "{message}"
            );
        }
        r.ok(Request::Set {
            id: LayerId(1),
            props: Props {
                opacity: 1.0000001,
                scale: 1000.0,
                ..Props::at(0.0, 0.0)
            },
        });
        let t = r.commit(1);
        let Op::Set { props, .. } = &t.ops[1] else {
            panic!("{:?}", t.ops)
        };
        assert_eq!((props.opacity, props.scale), (1.0, MAX_SCALE));
    }

    #[test]
    fn a_feather_must_be_numbers_and_one_that_belongs_to_no_clip_is_dropped() {
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        let clip = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
        };
        let set = |feather| Request::Set {
            id: LayerId(1),
            props: Props {
                clip: Some(clip),
                feather,
                ..Props::at(0.0, 0.0)
            },
        };
        for feather in [
            Edges::new(f32::NAN, 0.0, 0.0, 0.0),
            Edges::new(0.0, f32::INFINITY, 0.0, 0.0),
            Edges::horizontal(1e9),
        ] {
            let message = r.error(set(feather));
            assert!(
                message.contains("not a number") || message.contains("beyond"),
                "{message}"
            );
        }
        // A width a hair under nothing (an animation's rounding) is a hard edge, as a clip's size is.
        r.ok(set(Edges::new(-1.0e-8, 12.0, 0.0, 34.0)));
        // A feather the props have no clip for is not kept.
        r.ok(Request::Set {
            id: LayerId(1),
            props: Props {
                feather: Edges::all(9.0),
                ..Props::at(0.0, 0.0)
            },
        });
        let t = r.commit(1);
        let feathers: Vec<Edges> = t
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Set { props, .. } => Some(props.feather),
                _ => None,
            })
            .collect();
        assert_eq!(feathers, [Edges::new(0.0, 12.0, 0.0, 34.0), Edges::NONE]);
    }

    #[test]
    fn a_shape_restyle_must_have_sane_numbers() {
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        let broken = Shape {
            corner_radius: f32::NAN,
            ..shape()
        };
        assert!(
            r.error(restyle(1, Content::Shape(broken)))
                .contains("corner radius")
        );
        let angle = Shape {
            fill: Fill::Linear {
                from: Rgba(0, 0, 0, 255),
                to: Rgba(0, 0, 0, 255),
                angle_deg: f32::NAN,
            },
            ..shape()
        };
        assert!(r.error(restyle(1, Content::Shape(angle))).contains("angle"));
        let negative = Shape {
            corner_radius: -5.0,
            border_width: -1.0,
            ..shape()
        };
        r.ok(restyle(1, Content::Shape(negative)));
        let t = r.commit(1);
        let Op::Restyle {
            content: Content::Shape(shape),
            ..
        } = &t.ops[1]
        else {
            panic!("{:?}", t.ops)
        };
        assert_eq!((shape.corner_radius, shape.border_width), (0.0, 0.0));
    }

    #[test]
    fn a_restyle_stays_within_the_layers_own_kind() {
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        r.ok(create_content(2, 100, 50, Content::Glass(glass())));
        r.image_file(3, 4, 4);
        r.ok(create_image(3, 4, 4));
        assert!(
            r.error(restyle(1, Content::Glass(glass())))
                .contains("a shape")
        );
        assert!(
            r.error(restyle(2, Content::Shape(shape())))
                .contains("glass")
        );
        assert!(
            r.error(restyle(3, Content::Shape(shape())))
                .contains("image")
        );
        assert!(r.error(restyle(3, Content::Image)).contains("image"));
        r.ok(restyle(1, Content::Shape(shape())));
        let rounder = Glass {
            corner_radius: 3.0,
            ..glass()
        };
        r.ok(restyle(2, Content::Glass(rounder)));
        let t = r.commit(1);
        let restyled: Vec<_> = t
            .ops
            .iter()
            .filter(|op| matches!(op, Op::Restyle { .. }))
            .collect();
        assert_eq!(restyled.len(), 2, "only the two accepted ones are staged");
    }

    #[test]
    fn a_wallpaper_keeps_its_picture_or_its_lack_of_one_through_a_restyle() {
        let mut r = rig();
        r.image_file(1, 8, 4);
        r.ok(create_content(
            1,
            8,
            4,
            Content::Wallpaper(wallpaper(WallpaperKind::Silk)),
        ));
        r.ok(create_content(
            2,
            8,
            4,
            Content::Wallpaper(wallpaper(WallpaperKind::Starfield)),
        ));
        let aurora = WallpaperKind::Aurora {
            colours: [Rgba(1, 1, 1, 255); 4],
        };
        // Silk and Liquid both warp the picture; Starfield and Fireflies are drawn from nothing.
        r.ok(restyle(
            1,
            Content::Wallpaper(wallpaper(WallpaperKind::Liquid)),
        ));
        r.ok(restyle(2, Content::Wallpaper(wallpaper(aurora))));
        assert!(
            r.error(restyle(1, Content::Wallpaper(wallpaper(aurora))))
                .contains("picture")
        );
        assert!(
            r.error(restyle(
                2,
                Content::Wallpaper(wallpaper(WallpaperKind::Silk))
            ))
            .contains("picture")
        );
    }

    #[test]
    fn only_a_liquid_wallpaper_takes_ripples_at_a_place_that_is_a_number() {
        let mut r = rig();
        r.image_file(1, 8, 4);
        r.ok(create_content(
            1,
            8,
            4,
            Content::Wallpaper(wallpaper(WallpaperKind::Liquid)),
        ));
        r.ok(create_content(
            2,
            8,
            4,
            Content::Wallpaper(wallpaper(WallpaperKind::Starfield)),
        ));
        r.ok(create_shape(3, 10, 10));
        r.ok(ripple(1, 3.0, 2.0));
        assert!(r.error(ripple(2, 3.0, 2.0)).contains("Liquid"));
        assert!(r.error(ripple(3, 3.0, 2.0)).contains("Liquid"));
        assert!(r.error(ripple(1, f32::NAN, 2.0)).contains("not a number"));
        assert!(r.error(ripple(1, 3.0, 1e9)).contains("beyond"));
        assert!(r.error(ripple(9, 3.0, 2.0)).contains("does not exist"));
        // A wallpaper that stopped being Liquid stops rippling, and one that became Liquid starts.
        r.ok(restyle(
            1,
            Content::Wallpaper(wallpaper(WallpaperKind::Silk)),
        ));
        assert!(r.error(ripple(1, 3.0, 2.0)).contains("Liquid"));
        r.ok(restyle(
            1,
            Content::Wallpaper(wallpaper(WallpaperKind::Liquid)),
        ));
        r.ok(ripple(1, 3.0, 2.0));
        let t = r.commit(1);
        let ripples = t
            .ops
            .iter()
            .filter(|op| matches!(op, Op::Ripple { .. }))
            .count();
        assert_eq!(ripples, 2);
    }

    /// Sends `content` as a new layer and returns the refusal, or panics if it was accepted.
    fn refusal_of(r: &mut Rig, content: Content) -> String {
        r.error(create_content(90, 100, 50, content))
    }

    #[test]
    fn glass_numbers_must_be_numbers_in_their_ranges_and_only_drift_is_pulled_in() {
        let mut r = rig();
        let bad = |f: fn(&mut Glass)| {
            let mut g = glass();
            f(&mut g);
            Content::Glass(g)
        };
        let cases: [(Content, &str); 12] = [
            (bad(|g| g.corner_radius = f32::NAN), "corner radius"),
            (bad(|g| g.edge_width = f32::INFINITY), "edge width"),
            (bad(|g| g.saturation = 9.0), "saturation"),
            (bad(|g| g.refraction = -3.0), "refraction"),
            (bad(|g| g.sheen.as_mut().unwrap().peak = 2.0), "sheen peak"),
            (
                bad(|g| {
                    let s = g.sheen.as_mut().unwrap();
                    (s.start, s.end) = (0.7, 0.3);
                }),
                "sheen",
            ),
            (
                bad(|g| {
                    g.shadows[1] = Some(Shadow {
                        blur: 1000.0,
                        ..g.shadows[0].unwrap()
                    })
                }),
                "shadow blur",
            ),
            (
                bad(|g| g.shadows[0].as_mut().unwrap().offset_y = f32::NAN),
                "shadow offset",
            ),
            (bad(|g| g.depth.as_mut().unwrap().blur = -5.0), "depth blur"),
            (
                bad(|g| g.backdrop = Backdrop::Cached { radius: 100.0 }),
                "blur radius",
            ),
            (
                bad(|g| {
                    g.backdrop = Backdrop::Live {
                        radius: 30.0,
                        downsample: 3,
                    }
                }),
                "1/3",
            ),
            (
                bad(|g| {
                    g.backdrop = Backdrop::Live {
                        radius: f32::NAN,
                        downsample: 4,
                    }
                }),
                "blur radius",
            ),
        ];
        for (content, needle) in cases {
            let message = refusal_of(&mut r, content);
            assert!(message.contains(needle), "{needle}: {message}");
        }
        // Rounding drift is pulled in, a negative radius becomes square, and every legal downsample is accepted.
        let drifting = Glass {
            corner_radius: -4.0,
            sheen: Some(Sheen {
                peak: 1.0005,
                floor: -0.0004,
                start: 0.38,
                end: 0.6,
            }),
            ..glass()
        };
        r.ok(create_content(1, 100, 50, Content::Glass(drifting)));
        for (id, downsample) in [(2, 2), (3, 4), (4, 8)] {
            let live = Glass {
                backdrop: Backdrop::Live {
                    radius: 64.0,
                    downsample,
                },
                ..glass()
            };
            r.ok(create_content(id, 100, 50, Content::Glass(live)));
        }
        let t = r.commit(1);
        let Op::Create {
            content: Content::Glass(g),
            ..
        } = &t.ops[0]
        else {
            panic!("{:?}", t.ops)
        };
        assert_eq!(g.corner_radius, 0.0);
        let sheen = g.sheen.unwrap();
        assert_eq!((sheen.peak, sheen.floor), (1.0, 0.0));
    }

    #[test]
    fn a_wallpaper_cannot_move_faster_than_the_panel_or_by_nonsense_amounts() {
        let mut r = rig();
        let with = |f: fn(&mut Wallpaper)| {
            let mut w = wallpaper(WallpaperKind::Starfield);
            f(&mut w);
            Content::Wallpaper(w)
        };
        // 59.64 Hz: 60 fps is the panel's rate to the whole number, 61 is not reachable.
        let cases: [(Content, &str); 5] = [
            (with(|w| w.fps = 61), "faster than the panel"),
            (with(|w| w.intensity = 2.0), "intensity"),
            (with(|w| w.intensity = f32::NAN), "intensity"),
            (with(|w| w.speed = -1.0), "speed"),
            (with(|w| w.speed = f32::INFINITY), "speed"),
        ];
        for (content, needle) in cases {
            let message = refusal_of(&mut r, content);
            assert!(message.contains(needle), "{needle}: {message}");
        }
        r.ok(create_content(1, 100, 50, with(|w| w.fps = 60)));
        r.ok(create_content(2, 100, 50, with(|w| w.fps = 0)));
        r.ok(create_content(3, 100, 50, with(|w| w.speed = 0.0)));
    }

    #[test]
    fn what_is_faster_than_the_panel_is_the_panels_own_rate() {
        let at = |fps: u8, hz: f32| {
            let mut w = wallpaper(WallpaperKind::Starfield);
            w.fps = fps;
            check_wallpaper(w, hz).is_ok()
        };
        // The 59.64 Hz panel reaches 60, the 62.26 Hz panel 63 (the whole number above its rate).
        assert!(at(60, 59.64) && !at(61, 59.64));
        assert!(at(63, 62.26) && !at(64, 62.26));
    }

    #[test]
    fn a_mesh_needs_a_camera_that_can_see_and_a_light_that_points_somewhere() {
        let mut r = rig();
        r.mesh_file(1, &mesh_data());
        let with = |f: fn(&mut Mesh)| {
            let mut m = mesh();
            f(&mut m);
            Content::Mesh(m)
        };
        let cases: [(Content, &str); 7] = [
            (with(|m| m.eye[1] = f32::NAN), "camera eye"),
            (with(|m| m.eye = m.target), "looks at"),
            (with(|m| m.light = [0.0; 3]), "no direction"),
            (with(|m| m.fov_deg = 0.0), "field of view"),
            (with(|m| m.fov_deg = 200.0), "field of view"),
            (with(|m| m.ambient = 2.0), "ambient"),
            (with(|m| m.orbit_deg_per_s = 1.0e4), "orbit"),
        ];
        for (content, needle) in cases {
            let message = refusal_of(&mut r, content);
            assert!(message.contains(needle), "{needle}: {message}");
        }
        r.ok(create_content(1, 100, 50, Content::Mesh(mesh())));
    }

    #[test]
    fn a_mesh_layer_carries_its_decoded_geometry_and_refuses_files_that_are_not_meshes() {
        let mut r = rig();
        assert!(
            r.error(create_content(1, 100, 50, Content::Mesh(mesh())))
                .contains("cannot open")
        );
        r.mesh_bytes(1, b"short");
        assert!(
            r.error(create_content(1, 100, 50, Content::Mesh(mesh())))
                .contains("mesh")
        );
        let mut cut = mesh_data().encode().unwrap();
        cut.pop();
        r.mesh_bytes(1, &cut);
        assert!(
            r.error(create_content(1, 100, 50, Content::Mesh(mesh())))
                .contains("bytes where")
        );
        r.mesh_file(1, &mesh_data());
        r.ok(create_content(1, 100, 50, Content::Mesh(mesh())));
        let t = r.commit(1);
        let [
            Op::Create {
                content: Content::Mesh(_),
                mesh: Some(data),
                image: None,
                ..
            },
        ] = &t.ops[..]
        else {
            panic!("{:?}", t.ops)
        };
        assert_eq!(**data, mesh_data());
    }

    #[test]
    fn mesh_geometry_counts_against_its_own_memory_limit() {
        // Each mesh file is 82 bytes.
        let mut r = rig_with(Limits {
            mesh_bytes: 100,
            ..Limits::for_texture_size(4096)
        });
        for id in 1..=2 {
            r.mesh_file(id, &mesh_data());
        }
        r.ok(create_content(1, 10, 10, Content::Mesh(mesh())));
        assert!(
            r.error(create_content(2, 10, 10, Content::Mesh(mesh())))
                .contains("at most 18")
        );
        // Replacing the layer gives its bytes back; so does deleting it.
        r.ok(create_content(1, 10, 10, Content::Mesh(mesh())));
        r.ok(Request::Delete { id: LayerId(1) });
        r.ok(create_content(2, 10, 10, Content::Mesh(mesh())));
        // A mesh replaced by a shape frees its bytes too.
        r.ok(create_shape(2, 10, 10));
        r.ok(create_content(1, 10, 10, Content::Mesh(mesh())));
    }

    #[test]
    fn a_file_larger_than_any_mesh_can_be_is_refused_before_it_is_read() {
        let mut r = rig();
        r.mesh_bytes(1, &vec![0u8; MAX_MESH_FILE as usize + 1]);
        let message = r.error(create_content(1, 10, 10, Content::Mesh(mesh())));
        assert!(message.contains("at most"), "{message}");
    }

    #[test]
    fn only_layers_with_a_picture_can_be_damaged() {
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        r.ok(create_content(2, 10, 10, Content::Glass(glass())));
        r.mesh_file(3, &mesh_data());
        r.ok(create_content(3, 10, 10, Content::Mesh(mesh())));
        r.ok(create_content(
            4,
            10,
            10,
            Content::Wallpaper(wallpaper(WallpaperKind::Starfield)),
        ));
        for (id, what) in [(1, "shape"), (2, "glass"), (3, "mesh"), (4, "wallpaper")] {
            let message = r.error(damage_all(id));
            assert!(message.contains(what), "{what}: {message}");
        }
        // A wallpaper that warps a picture is damaged like an image layer.
        r.image_file(5, 10, 10);
        r.ok(create_content(
            5,
            10,
            10,
            Content::Wallpaper(wallpaper(WallpaperKind::Silk)),
        ));
        r.ok(damage_all(5));
        let t = r.commit(1);
        assert!(matches!(
            t.ops.last(),
            Some(Op::Damage { id: LayerId(5), .. })
        ));
    }

    #[test]
    fn a_live_layer_has_no_file_of_its_own_no_look_to_change_and_no_damage() {
        use compositor_protocol::{Live, SourceId};
        let mut r = rig();
        let live = |source| {
            Content::Live(Live {
                source: SourceId(source),
            })
        };
        // Its picture is a producer's buffer: no file is read, so there is none to be missing, and no image memory is used.
        r.ok(create_content(1, 100, 50, live(4)));
        let t = r.commit(1);
        assert!(matches!(
            &t.ops[..],
            [Op::Create {
                content: Content::Live(_),
                image: None,
                mesh: None,
                ..
            }]
        ));
        assert_eq!(r.session.image_bytes, 0);
        let message = r.error(restyle(1, live(5)));
        assert!(
            message.contains("live layer") && message.contains("no look"),
            "{message}"
        );
        let message = r.error(damage_all(1));
        assert!(message.contains("live layer"), "{message}");
        // It counts as a layer, like any other.
        let mut small = rig_with(Limits {
            layers: 1,
            ..Limits::for_texture_size(4096)
        });
        small.ok(create_content(1, 10, 10, live(1)));
        assert!(
            small
                .error(create_content(2, 10, 10, live(1)))
                .contains("more than 1 layers")
        );
    }

    #[test]
    fn the_picture_of_a_wallpaper_counts_against_the_image_memory() {
        let mut r = rig_with(Limits {
            image_bytes: 1024,
            ..Limits::for_texture_size(4096)
        });
        r.image_file(1, 16, 16);
        r.image_file(2, 16, 16);
        let silk = || Content::Wallpaper(wallpaper(WallpaperKind::Silk));
        r.ok(create_content(1, 16, 16, silk()));
        assert!(
            r.error(create_content(2, 16, 16, silk()))
                .contains("more than")
        );
        // A wallpaper of a kind with no picture costs nothing, and replacing the silk with one frees its pixels.
        r.ok(create_content(
            3,
            960,
            480,
            Content::Wallpaper(wallpaper(WallpaperKind::Starfield)),
        ));
        r.ok(create_content(
            1,
            16,
            16,
            Content::Wallpaper(wallpaper(WallpaperKind::Starfield)),
        ));
        r.ok(create_content(2, 16, 16, silk()));
        let t = r.commit(1);
        assert!(matches!(
            &t.ops[..],
            [
                Op::Create { image: Some(_), .. },
                Op::Create { image: None, .. },
                Op::Create { image: None, .. },
                Op::Create { image: Some(_), .. },
            ]
        ));
    }

    #[test]
    fn an_image_layer_needs_its_file_and_a_refused_create_leaves_nothing_behind() {
        let mut r = rig();
        assert!(r.error(create_image(1, 4, 4)).contains("cannot open"));
        assert!(
            r.error(Request::Damage {
                id: LayerId(1),
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0
                }
            })
            .contains("does not exist")
        );
        r.ok(Request::Delete { id: LayerId(1) });
        r.image_file(1, 2, 2);
        assert!(r.error(create_image(1, 4, 4)).contains("needs 64"));
        r.image_file(1, 4, 4);
        r.ok(create_image(1, 4, 4));
        let t = r.commit(1);
        assert!(matches!(&t.ops[..], [Op::Create { image: Some(_), .. }]));
    }

    #[test]
    fn damage_is_clamped_to_the_layer_and_dropped_when_nothing_is_inside() {
        let mut r = rig();
        r.image_file(1, 10, 10);
        r.ok(create_image(1, 10, 10));
        let damage = |x: f32, y: f32, w: f32, h: f32| Request::Damage {
            id: LayerId(1),
            rect: Rect { x, y, w, h },
        };
        r.ok(damage(-5.0, 2.5, 100.0, 3.0));
        r.ok(damage(50.0, 50.0, 5.0, 5.0));
        assert!(
            r.error(damage(f32::NAN, 0.0, 1.0, 1.0))
                .contains("not a number")
        );
        let t = r.commit(1);
        let rects: Vec<_> = t
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Damage { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(rects.len(), 1);
        assert_eq!(
            (rects[0].x, rects[0].y, rects[0].w, rects[0].h),
            (0, 2, 10, 4)
        );
    }

    #[test]
    fn shapes_cannot_be_damaged() {
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        let message = r.error(Request::Damage {
            id: LayerId(1),
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
        });
        assert!(message.contains("shape"), "{message}");
    }

    #[test]
    fn creating_over_an_existing_id_replaces_it_and_keeps_the_memory_count_honest() {
        let mut r = rig_with(Limits {
            image_bytes: 1024,
            ..Limits::for_texture_size(4096)
        });
        r.image_file(1, 16, 16);
        r.image_file(2, 16, 16);
        r.ok(create_image(1, 16, 16));
        // 1024 bytes are all there is: a second layer does not fit, but replacing the first one does.
        assert!(r.error(create_image(2, 16, 16)).contains("more than"));
        r.ok(create_image(1, 16, 16));
        r.ok(Request::Delete { id: LayerId(1) });
        r.ok(create_image(2, 16, 16));
    }

    #[test]
    fn there_is_a_cap_on_layers_and_on_requests_between_commits() {
        let mut r = rig_with(Limits {
            layers: 3,
            ops_per_batch: 5,
            ..Limits::for_texture_size(4096)
        });
        r.ok(create_shape(1, 1, 1));
        r.ok(create_shape(2, 1, 1));
        r.ok(create_shape(3, 1, 1));
        assert!(
            r.error(create_shape(4, 1, 1))
                .contains("more than 3 layers")
        );
        // Replacing an existing layer is not a new layer.
        r.ok(create_shape(3, 1, 1));
        r.ok(create_shape(3, 1, 1));
        assert!(r.error(create_shape(3, 1, 1)).contains("without a Commit"));
        assert_eq!(r.commit(1).ops.len(), 5);
        r.ok(create_shape(3, 1, 1));
    }

    #[test]
    fn stats_report_the_shared_counters() {
        let mut r = rig();
        r.session.stats.frame(1234, 1);
        let out = r.send(Request::Stats);
        assert!(
            matches!(&out.replies[..], [Event::Stats(s)] if s.frames == 1 && s.missed == 1 && s.worst_render_us == 1234)
        );
    }

    #[test]
    fn goodbye_clears_the_scene_without_asking_for_a_frame() {
        let r = rig();
        let t = r.session.goodbye();
        assert_eq!((t.client, t.tag), (7, None));
        assert!(matches!(&t.ops[..], [Op::Clear]));
    }

    #[test]
    fn a_gpu_clock_hold_takes_effect_on_its_own_without_waiting_for_or_taking_the_staged_changes() {
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        let out = r.send(Request::HoldGpuClock { mhz: Some(416) });
        let hold = out.commit.expect("a hold is handed over at once");
        assert_eq!((hold.client, hold.tag), (7, None));
        assert!(matches!(&hold.ops[..], [Op::HoldGpuClock(Some(416_000))]));
        assert!(!hold.is_goodbye());
        // The layer staged before it is still waiting for its commit.
        assert!(matches!(&r.commit(1).ops[..], [Op::Create { .. }]));
        // Giving the clock back is a hold of none.
        let out = r.send(Request::HoldGpuClock { mhz: None });
        assert!(matches!(
            &out.commit.unwrap().ops[..],
            [Op::HoldGpuClock(None)]
        ));
    }

    #[test]
    fn a_gpu_clock_the_device_does_not_offer_is_refused_and_says_which_it_does() {
        let mut r = rig();
        for mhz in [0, 300, 521, u32::MAX] {
            let message = r.error(Request::HoldGpuClock { mhz: Some(mhz) });
            assert!(message.contains("520, 416, 299"), "{message}");
        }
        // The refusals leave the connection usable.
        assert!(!r.session.handle(Request::Stats).close);
    }

    #[test]
    fn a_device_without_gpu_clock_control_refuses_a_hold_but_lets_it_go() {
        let dir = TempDir::new("no-gpu");
        let config = Arc::new(Config {
            limits: Limits::for_texture_size(4096),
            panel_width: 480,
            panel_height: 960,
            refresh_hz: 59.64,
            shm_dir: dir.path().to_owned(),
            gpu_opp_khz: Vec::new(),
        });
        let mut session = Session::new(config, Arc::default(), 1);
        session.handle(Request::Hello {
            version: PROTOCOL_VERSION,
        });
        let out = session.handle(Request::HoldGpuClock { mhz: Some(520) });
        assert!(
            matches!(&out.replies[..], [Event::Error { message }] if message.contains("no GPU clock control"))
        );
        assert!(out.commit.is_none() && !out.close);
        let out = session.handle(Request::HoldGpuClock { mhz: None });
        assert!(out.commit.is_some());
    }

    #[test]
    fn the_glass_ideas_are_handed_over_on_their_own_and_a_bit_that_is_no_idea_is_refused() {
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        let out = r.send(Request::Ideas {
            mask: ideas::STILL_GLASS | ideas::ADAPTIVE,
        });
        let switch = out.commit.expect("handed over at once");
        assert_eq!((switch.client, switch.tag), (7, None));
        assert!(
            matches!(&switch.ops[..], [Op::Ideas(mask)] if *mask == ideas::STILL_GLASS | ideas::ADAPTIVE)
        );
        assert!(!switch.is_goodbye());
        // The layer staged before it is still waiting for its commit.
        assert!(matches!(&r.commit(1).ops[..], [Op::Create { .. }]));
        let message = r.error(Request::Ideas {
            mask: ideas::ALL + 1,
        });
        assert!(message.contains("an idea there is not"), "{message}");
        assert!(!r.session.handle(Request::Stats).close);
    }

    #[test]
    fn smooth_first_is_handed_over_on_its_own_and_asks_for_no_frame_and_a_ladder_that_is_off_refuses_only_full()
     {
        use compositor_protocol::SmoothFirst;
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        for level in SmoothFirst::ALL {
            let switch = r
                .send(Request::SmoothFirst { level })
                .commit
                .expect("handed over at once");
            assert_eq!((switch.client, switch.tag), (7, None));
            assert!(matches!(&switch.ops[..], [Op::SmoothFirst(l)] if *l == level));
            assert!(!switch.is_goodbye() && !switch.asks_for_frame());
        }
        // The layer staged before them is still waiting for its commit.
        assert!(matches!(&r.commit(1).ops[..], [Op::Create { .. }]));
        assert!(!r.session.handle(Request::Stats).close);
        // What a compositor whose ladder is switched off (LUMASHOW_LADDER=off) says to each level: only Full is refused.
        for level in SmoothFirst::ALL {
            assert_eq!(
                crate::ladder::refusal(level, false).is_some(),
                level == SmoothFirst::Full
            );
            assert_eq!(crate::ladder::refusal(level, true), None);
        }
    }

    #[test]
    fn going_dark_is_handed_over_at_once_and_leaves_the_staged_changes_for_their_commit() {
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        let dark = r
            .send(Request::Display { on: false })
            .commit
            .expect("handed over at once");
        assert_eq!((dark.client, dark.tag), (7, None));
        assert!(matches!(&dark.ops[..], [Op::Display(false)]));
        assert!(matches!(&r.commit(1).ops[..], [Op::Create { .. }]));
    }

    #[test]
    fn a_hold_before_the_greeting_is_refused() {
        let dir = TempDir::new("hold-first");
        let config = Arc::new(Config {
            limits: Limits::for_texture_size(4096),
            panel_width: 480,
            panel_height: 960,
            refresh_hz: 59.64,
            shm_dir: dir.path().to_owned(),
            gpu_opp_khz: vec![520_000],
        });
        let mut session = Session::new(config, Arc::default(), 1);
        let out = session.handle(Request::HoldGpuClock { mhz: Some(520) });
        assert!(out.close && out.commit.is_none());
    }

    // --- Version 7: groups, motion tracks, prepared backdrops, atlases and glyph runs. ---

    fn group_req(id: u32, parent: Option<u32>) -> Request {
        Request::GroupCreate {
            id: compositor_protocol::GroupId(id),
            parent: parent.map(compositor_protocol::GroupId),
            pose: compositor_protocol::GroupPose::IDENTITY,
        }
    }

    fn move_req(id: u32, parent: Option<u32>) -> Request {
        Request::GroupParent {
            id: compositor_protocol::GroupId(id),
            parent: parent.map(compositor_protocol::GroupId),
            preserve_world: true,
        }
    }

    fn track_req(id: u32) -> Request {
        Request::MotionCreate {
            id: compositor_protocol::MotionId(id),
            value: 0.0,
            bounds: compositor_protocol::MotionBounds::range(
                0.0,
                100.0,
                compositor_protocol::Units::Pixels,
            ),
        }
    }

    fn drive_req(id: u32, position: f32) -> Request {
        Request::MotionDrive {
            id: compositor_protocol::MotionId(id),
            sample: compositor_protocol::DriveSample {
                contact: 1,
                phase: compositor_protocol::DrivePhase::Move,
                position,
                time_ns: 1,
            },
        }
    }

    #[test]
    fn a_pose_the_session_passes_on_is_always_one_the_engine_takes() {
        // A group the engine refuses (without a word) is one the client's mirror still counts and attaches to. A clip size a
        // hair under zero is what an animation's rounding makes; the rest is whatever else a client can send.
        use compositor_protocol::{GroupPose, Rect};
        let numbers = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            -1e-8,
            1e-8,
            -1e6,
            1e6,
            2e6,
            f32::NAN,
            f32::INFINITY,
        ];
        let mut passed = 0;
        for w in numbers {
            for h in numbers {
                for scale in [0.0, 1.0, 1e9, -1.0] {
                    let pose = GroupPose {
                        clip: Some(Rect {
                            x: 0.0,
                            y: 0.0,
                            w,
                            h,
                        }),
                        scale,
                        opacity: 1.0000001,
                        ..GroupPose::IDENTITY
                    };
                    if let Ok(checked) = super::v7::check_pose(pose) {
                        assert!(checked.is_valid(), "{pose:?} came out as {checked:?}");
                        passed += 1;
                    }
                }
            }
        }
        assert!(passed > 0, "some of them pass");
        // And through the session: a clip a hair under zero is a group the engine gets, with the clip at zero.
        let mut r = rig();
        let sliver = GroupPose {
            clip: Some(Rect {
                x: 1.0,
                y: 2.0,
                w: -1e-8,
                h: 30.0,
            }),
            ..GroupPose::IDENTITY
        };
        r.ok(Request::GroupCreate {
            id: compositor_protocol::GroupId(1),
            parent: None,
            pose: sliver,
        });
        r.ok(Request::GroupSet {
            id: compositor_protocol::GroupId(1),
            pose: sliver,
        });
        let ops = r.commit(1).ops;
        let poses: Vec<GroupPose> = ops
            .iter()
            .filter_map(|op| match op {
                Op::GroupCreate { pose, .. } | Op::GroupSet { pose, .. } => Some(*pose),
                _ => None,
            })
            .collect();
        assert_eq!(poses.len(), 2);
        for pose in poses {
            assert!(pose.is_valid(), "{pose:?}");
            assert_eq!(pose.clip.map(|c| (c.w, c.h)), Some((0.0, 30.0)));
        }
    }

    #[test]
    fn groups_nest_to_a_bounded_depth_without_loops_and_only_under_groups_that_exist() {
        let mut r = rig();
        assert!(
            r.error(group_req(2, Some(1)))
                .contains("group 1 does not exist")
        );
        r.ok(group_req(1, None));
        assert!(r.error(group_req(1, None)).contains("exists already"));
        let max = compositor_protocol::group::MAX_GROUP_DEPTH as u32;
        for id in 2..=max {
            r.ok(group_req(id, Some(id - 1)));
        }
        let deep = format!("at most {max} deep");
        assert!(r.error(group_req(max + 1, Some(max))).contains(&deep));
        // A loop, and a move that would make a subtree too deep.
        assert!(r.error(move_req(1, Some(5))).contains("loop"));
        assert!(r.error(move_req(1, Some(1))).contains("loop"));
        r.ok(group_req(20, None));
        assert!(r.error(move_req(1, Some(20))).contains(&deep));
        r.ok(move_req(max, Some(20)));
        // Deleting a group moves what is under it to its parent, and that frees depth.
        r.ok(Request::GroupDelete {
            id: compositor_protocol::GroupId(2),
        });
        r.ok(move_req(1, Some(20)));
        r.ok(Request::GroupDelete {
            id: compositor_protocol::GroupId(77),
        });
        // Everything accepted reached the batch in order, and nothing refused did.
        let ops = r.commit(1).ops;
        assert_eq!(
            ops.iter()
                .filter(|o| matches!(o, Op::GroupCreate { .. }))
                .count(),
            max as usize + 1
        );
        assert_eq!(
            ops.iter()
                .filter(|o| matches!(o, Op::GroupParent { .. }))
                .count(),
            2
        );
        assert_eq!(
            ops.iter()
                .filter(|o| matches!(o, Op::GroupDelete { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn there_is_a_cap_on_groups_and_tracks() {
        let mut r = rig();
        let groups = compositor_protocol::group::MAX_GROUPS as u32;
        assert_eq!(groups, 96);
        for id in 0..groups {
            r.ok(group_req(id, None));
        }
        assert!(
            r.error(group_req(groups, None))
                .contains(&format!("more than {groups} groups"))
        );
        let most = compositor_protocol::motion::MAX_MOTIONS as u32;
        for id in 0..most {
            r.ok(track_req(id));
        }
        assert!(
            r.error(track_req(most))
                .contains(&format!("more than {most} motion tracks"))
        );
    }

    #[test]
    fn group_poses_and_bindings_must_be_numbers_and_drift_is_pulled_in() {
        let mut r = rig();
        let pose = |x: f32, scale: f32, opacity: f32| compositor_protocol::GroupPose {
            x,
            scale,
            opacity,
            ..compositor_protocol::GroupPose::IDENTITY
        };
        for bad in [
            pose(f32::NAN, 1.0, 1.0),
            pose(1.0e9, 1.0, 1.0),
            pose(0.0, f32::INFINITY, 1.0),
        ] {
            let out = r.send(Request::GroupCreate {
                id: compositor_protocol::GroupId(1),
                parent: None,
                pose: bad,
            });
            assert!(matches!(&out.replies[..], [Event::Error { .. }]));
        }
        r.ok(Request::GroupCreate {
            id: compositor_protocol::GroupId(1),
            parent: None,
            pose: pose(0.0, 1.0, 1.0004),
        });
        r.ok(track_req(1));
        let binding = |values: [f32; 2], properties| compositor_protocol::PoseBinding {
            values,
            from: compositor_protocol::GroupPose::IDENTITY,
            to: compositor_protocol::GroupPose::at(1.0, 0.0),
            properties,
        };
        let bind = |binding| Request::GroupBind {
            id: compositor_protocol::GroupId(1),
            motion: compositor_protocol::MotionId(1),
            binding,
        };
        assert!(
            r.error(bind(binding(
                [0.0, 0.0],
                compositor_protocol::PoseProperties::TRANSLATION
            )))
            .contains("distinct")
        );
        assert!(
            r.error(bind(binding(
                [0.0, 1.0],
                compositor_protocol::PoseProperties::NONE
            )))
            .contains("at least one")
        );
        r.ok(bind(binding(
            [0.0, 1.0],
            compositor_protocol::PoseProperties::TRANSLATION,
        )));
        assert!(
            r.error(Request::GroupBind {
                id: compositor_protocol::GroupId(1),
                motion: compositor_protocol::MotionId(9),
                binding: binding([0.0, 1.0], compositor_protocol::PoseProperties::TRANSLATION),
            })
            .contains("motion track 9")
        );
        let ops = r.commit(1).ops;
        assert!(matches!(&ops[0], Op::GroupCreate { pose, .. } if pose.opacity == 1.0));
    }

    #[test]
    fn attaching_puts_a_layer_under_a_group_that_exists_with_numbers_that_are_numbers() {
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        r.ok(group_req(1, None));
        let attach = |layer: u32, group: Option<u32>, x: f32| Request::Attach {
            id: LayerId(layer),
            group: group.map(compositor_protocol::GroupId),
            local: Props::at(x, 0.0),
        };
        assert!(r.error(attach(1, Some(3), 0.0)).contains("group 3"));
        assert!(r.error(attach(9, Some(1), 0.0)).contains("layer 9"));
        assert!(
            r.error(attach(1, Some(1), f32::NAN))
                .contains("not a number")
        );
        r.ok(attach(1, Some(1), 5.0));
        r.ok(attach(1, None, 5.0));
        assert_eq!(
            r.commit(1)
                .ops
                .iter()
                .filter(|o| matches!(o, Op::Attach { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn a_pointer_sample_goes_over_at_once_on_its_own_and_a_track_made_in_the_batch_keeps_its_samples_behind_it()
     {
        let mut r = rig();
        r.ok(track_req(1));
        // Made in this batch: the sample waits behind the creation, in order.
        r.ok(drive_req(1, 3.0));
        let t = r.commit(1);
        assert!(matches!(
            &t.ops[..],
            [Op::MotionCreate { .. }, Op::MotionDrive { .. }]
        ));
        // Committed: a sample is handed over alone, with no tag, ahead of any commit, and asks for its frame by itself.
        r.ok(create_shape(1, 10, 10));
        let out = r.send(drive_req(1, 7.0));
        let t = out.commit.expect("a sample is handed over at once");
        assert_eq!((t.client, t.tag), (7, None));
        assert!(matches!(&t.ops[..], [Op::MotionDrive { .. }]) && t.ops[0].wants_frame());
        assert!(!t.is_goodbye());
        // The layer staged before it is still waiting for its commit.
        assert!(matches!(&r.commit(2).ops[..], [Op::Create { .. }]));
        // Unknown tracks, non-numbers, impossible settles and bad bounds are refused.
        assert!(r.error(drive_req(5, 1.0)).contains("motion track 5"));
        assert!(r.error(drive_req(1, f32::NAN)).contains("not a number"));
        let settle = |settle| Request::MotionSettle {
            id: compositor_protocol::MotionId(1),
            target: None,
            velocity: None,
            settle,
        };
        assert!(
            r.error(settle(compositor_protocol::Settle::CriticalSpring(
                compositor_protocol::SpringToken { response_ms: 1.0 }
            )))
            .contains("10 to 5000")
        );
        let out = r.send(settle(compositor_protocol::Settle::CriticalSpring(
            compositor_protocol::SpringToken { response_ms: 300.0 },
        )));
        assert!(matches!(
            &out.commit.unwrap().ops[..],
            [Op::MotionSettle { .. }]
        ));
        let mut bad =
            compositor_protocol::MotionBounds::range(5.0, 1.0, compositor_protocol::Units::Pixels);
        assert!(
            r.error(Request::MotionCreate {
                id: compositor_protocol::MotionId(2),
                value: 0.0,
                bounds: bad
            })
            .contains("min <= max")
        );
        bad = compositor_protocol::MotionBounds {
            ratio: 0.0,
            ..compositor_protocol::MotionBounds::range(0.0, 1.0, compositor_protocol::Units::Pixels)
        };
        assert!(
            r.error(Request::MotionCreate {
                id: compositor_protocol::MotionId(2),
                value: 0.0,
                bounds: bad
            })
            .contains("ratio")
        );
    }

    fn backdrop_req(id: u32, wallpaper: Option<u32>, plates: &[u32], downsample: u8) -> Request {
        Request::BackdropCreate {
            id: compositor_protocol::BackdropId(id),
            spec: compositor_protocol::CaptureSpec {
                wallpaper: wallpaper.map(LayerId),
                plates: plates
                    .iter()
                    .map(|g| compositor_protocol::PlateSource {
                        group: compositor_protocol::GroupId(*g),
                    })
                    .collect(),
                radius: 30.0,
                downsample,
            },
        }
    }

    #[test]
    fn a_prepared_backdrop_needs_a_wallpaper_and_groups_that_exist_and_a_lease_goes_over_at_once() {
        let mut r = rig();
        r.ok(create_shape(1, 10, 10));
        r.ok(group_req(1, None));
        assert!(
            r.error(backdrop_req(1, Some(9), &[], 4))
                .contains("layer 9")
        );
        assert!(
            r.error(backdrop_req(1, Some(1), &[7], 4))
                .contains("group 7")
        );
        assert!(r.error(backdrop_req(1, Some(1), &[1], 3)).contains("1/3"));
        assert!(
            r.error(backdrop_req(1, Some(1), &[1; 9], 4))
                .contains("more than 8 plates")
        );
        r.ok(backdrop_req(1, Some(1), &[1], 4));
        assert!(
            r.error(backdrop_req(1, Some(1), &[1], 4))
                .contains("exists already")
        );
        for id in 2..=4 {
            r.ok(backdrop_req(id, None, &[], 4));
        }
        assert!(
            r.error(backdrop_req(5, None, &[], 4))
                .contains("more than 4")
        );
        // Made in this batch: a freeze waits behind it. Committed: it goes over alone.
        r.ok(Request::BackdropFreeze {
            id: compositor_protocol::BackdropId(1),
        });
        let t = r.commit(1);
        assert!(matches!(t.ops.last(), Some(Op::BackdropFreeze { .. })));
        let out = r.send(Request::BackdropFreeze {
            id: compositor_protocol::BackdropId(1),
        });
        let alone = out.commit.expect("a lease is taken at once");
        assert_eq!(alone.tag, None);
        let out = r.send(Request::BackdropRelease {
            id: compositor_protocol::BackdropId(1),
        });
        assert!(matches!(
            &out.commit.unwrap().ops[..],
            [Op::BackdropRelease { .. }]
        ));
        assert!(
            r.error(Request::BackdropRefresh {
                id: compositor_protocol::BackdropId(9),
                content: 1
            })
            .contains("prepared backdrop 9")
        );
    }

    #[test]
    fn glass_over_a_prepared_backdrop_needs_the_backdrop_to_exist() {
        let mut r = rig();
        let prepared = |id| {
            Content::Glass(Glass {
                backdrop: Backdrop::Prepared {
                    id: compositor_protocol::BackdropId(id),
                },
                ..glass()
            })
        };
        assert!(
            r.error(create_content(1, 40, 40, prepared(1)))
                .contains("prepared backdrop 1")
        );
        r.ok(backdrop_req(1, None, &[], 4));
        r.ok(create_content(1, 40, 40, prepared(1)));
        assert!(
            r.error(restyle(1, prepared(2)))
                .contains("prepared backdrop 2")
        );
        r.ok(restyle(1, prepared(1)));
    }

    fn glyph_layer(id: u32, atlas: u32) -> Request {
        create_content(
            id,
            200,
            40,
            Content::Glyphs(compositor_protocol::GlyphStyle::single(
                compositor_protocol::AtlasId(atlas),
                Rgba(255, 255, 255, 255),
            )),
        )
    }

    fn glyph(sx: u16, w: u16) -> compositor_protocol::Glyph {
        compositor_protocol::Glyph {
            x: 0.0,
            y: 0.0,
            sx,
            sy: 0,
            w,
            h: 10,
            ink: 0,
        }
    }

    #[test]
    fn atlases_are_bounded_and_uploads_must_fit_them_and_come_in_generation_order() {
        let mut r = rig();
        let atlas = |id, w, h| Request::AtlasCreate {
            id: compositor_protocol::AtlasId(id),
            width: w,
            height: h,
        };
        assert!(r.error(atlas(1, 0, 10)).contains("outside 1..=2048"));
        assert!(r.error(atlas(1, 4096, 10)).contains("outside 1..=2048"));
        r.ok(atlas(1, 1024, 1024));
        assert!(r.error(atlas(1, 8, 8)).contains("exists already"));
        r.ok(atlas(2, 1024, 1024));
        r.ok(atlas(3, 1024, 1024));
        r.ok(atlas(4, 1024, 1024));
        assert!(r.error(atlas(5, 8, 8)).contains("more than 4 atlases"));
        // 4 MiB are all there is.
        let mut r = rig();
        r.ok(atlas(1, 2048, 2048));
        assert!(r.error(atlas(2, 2048, 2048)).contains("more than 4 MiB"));
        r.ok(Request::AtlasDelete {
            id: compositor_protocol::AtlasId(1),
        });
        r.ok(atlas(2, 2048, 2048));

        let mut r = rig();
        r.ok(atlas(1, 1024, 512));
        let upload = |slot, generation, rect| Request::AtlasUpload {
            id: compositor_protocol::AtlasId(1),
            slot,
            generation,
            rect,
        };
        let rect = |x, y, w, h| compositor_protocol::AtlasRect { x, y, w, h };
        assert!(
            r.error(upload(2, 1, rect(0, 0, 8, 8)))
                .contains("staging slot 2")
        );
        assert!(
            r.error(upload(0, 1, rect(1000, 0, 40, 8)))
                .contains("not inside")
        );
        assert!(
            r.error(upload(0, 1, rect(0, 0, 0, 8)))
                .contains("not inside")
        );
        assert!(
            r.error(upload(0, 1, rect(0, 0, 1024, 512)))
                .contains("one request may carry")
        );
        r.ok(upload(0, 5, rect(0, 0, 512, 128)));
        assert!(
            r.error(upload(1, 5, rect(0, 128, 8, 8)))
                .contains("does not follow 5")
        );
        assert!(
            r.error(upload(1, 4, rect(0, 128, 8, 8)))
                .contains("does not follow 5")
        );
        r.ok(upload(1, 6, rect(0, 128, 8, 8)));
        assert!(
            r.error(Request::AtlasUpload {
                id: compositor_protocol::AtlasId(9),
                slot: 0,
                generation: 1,
                rect: rect(0, 0, 1, 1)
            })
            .contains("atlas 9")
        );
        let ops = r.commit(1).ops;
        let Op::AtlasUpload {
            path,
            slot,
            generation,
            ..
        } = &ops[1]
        else {
            panic!("{ops:?}")
        };
        assert_eq!((*slot, *generation), (0, 5));
        assert_eq!(
            path,
            &r.dir.path().join("atlas-1-0.a8"),
            "in the session's shm directory"
        );
    }

    #[test]
    fn a_glyph_run_needs_its_atlas_and_its_quads_must_lie_inside_it_and_it_keeps_the_atlas_alive() {
        let mut r = rig();
        assert!(r.error(glyph_layer(1, 1)).contains("atlas 1"));
        r.ok(Request::AtlasCreate {
            id: compositor_protocol::AtlasId(1),
            width: 64,
            height: 32,
        });
        r.ok(glyph_layer(1, 1));
        r.ok(create_shape(2, 10, 10));
        let set = |id, glyphs| Request::SetGlyphs {
            id: LayerId(id),
            after: 0,
            glyphs,
        };
        assert!(r.error(set(2, vec![])).contains("not a glyph run"));
        assert!(r.error(set(9, vec![])).contains("layer 9"));
        assert!(r.error(set(1, vec![glyph(60, 8)])).contains("not inside"));
        assert!(r.error(set(1, vec![glyph(0, 0)])).contains("not inside"));
        assert!(
            r.error(set(
                1,
                vec![compositor_protocol::Glyph {
                    x: f32::NAN,
                    ..glyph(0, 8)
                }]
            ))
            .contains("not a number")
        );
        assert!(
            r.error(set(1, vec![glyph(0, 8); 257]))
                .contains("257 glyphs")
        );
        r.ok(set(1, vec![glyph(0, 8); 256]));
        assert!(
            r.error(Request::AtlasDelete {
                id: compositor_protocol::AtlasId(1)
            })
            .contains("still the atlas")
        );
        r.ok(Request::Delete { id: LayerId(1) });
        r.ok(Request::AtlasDelete {
            id: compositor_protocol::AtlasId(1),
        });
        assert!(matches!(
            r.commit(1).ops.last(),
            Some(Op::AtlasDelete { .. })
        ));
    }

    #[test]
    fn a_runs_quads_name_an_ink_the_palette_has_and_come_in_non_decreasing_ink_order() {
        let mut r = rig();
        r.ok(Request::AtlasCreate {
            id: compositor_protocol::AtlasId(1),
            width: 64,
            height: 32,
        });
        r.ok(glyph_layer(1, 1));
        let set = |glyphs| Request::SetGlyphs {
            id: LayerId(1),
            after: 0,
            glyphs,
        };
        let inked = |ink| compositor_protocol::Glyph { ink, ..glyph(0, 8) };
        assert!(r.error(set(vec![inked(4)])).contains("glyph 0 has ink 4"));
        assert!(
            r.error(set(vec![inked(0), inked(1), inked(255)]))
                .contains("glyph 2 has ink 255")
        );
        let decreasing = r.error(set(vec![inked(0), inked(2), inked(1)]));
        assert!(
            decreasing.contains("glyph 2 has ink 1 after ink 2"),
            "{decreasing}"
        );
        assert!(r.error(set(vec![inked(3), inked(0)])).contains("glyph 1"));
        // Sorted, with repeats and gaps, up to the last ink of the palette.
        r.ok(set(vec![inked(0), inked(0), inked(2), inked(3), inked(3)]));
        r.ok(set(vec![inked(3)]));
        r.ok(set(vec![]));
        let ops = r.commit(1).ops;
        let sets: Vec<&Op> = ops
            .iter()
            .filter(|op| matches!(op, Op::SetGlyphs { .. }))
            .collect();
        assert_eq!(
            sets.len(),
            3,
            "only the lists that were accepted are sent on"
        );
    }
}
