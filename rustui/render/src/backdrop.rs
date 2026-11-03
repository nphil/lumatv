//! What a glass surface sees of the layers below it: a blurred picture of them, made small.
//!
//! **Making it.** The layers below the glass are drawn again into a texture `downsample` times smaller than the
//! screen in each direction (the *mirror*), and that is blurred by a chain of small passes (`shaders::blur`). The
//! mirror is drawn, rather than copied out of the frame being drawn, because a tile-based GPU cannot read the frame
//! it is drawing without first writing the whole of it to memory and loading it back: reading the frame's colour
//! mid-frame costs a full-screen resolve (see `experiments/s9-glass/RESULTS.md` for what it measured). Drawing the
//! layers again at a sixteenth of the pixels costs a sixteenth of the fill and needs nothing from the frame. The
//! glass surfaces among the layers below are drawn flat in the mirror (their tint, edge and shadow, without a
//! backdrop of their own): blurred by a wider blur on top, the difference cannot be seen.
//!
//! **Sharing it.** Glass surfaces that could not tell their backdrops apart share one picture: those with the same
//! blur, none of which has anything drawn over its body between the first one's backdrop and its own. What the surfaces
//! that share one have around their bodies, their shadows, does not keep them apart: in the frame they cast their
//! shadows together, before any of their bodies are drawn. A live picture has them in it, so that each body sees its
//! neighbours' shadows through the glass (CSS: a surface's own shadow is never under its body, and a neighbour's is); a
//! cached one does not, which is what keeps it from being taken again whenever one of the surfaces moves. Four cards
//! side by side with the shadows Lucent gives them make one picture and not four.
//!
//! **Keeping it.** A `Live` backdrop is made again every frame the glass is drawn, of the layers as they are
//! drawn now. A `Cached` one is made of the layers at rest (no animation running, wallpapers still) and only again
//! when a layer below changes what it is or where it rests: a fingerprint of the layers below decides.
//!
//! This file holds the pure parts: which backdrops a frame needs (`plan`), when a cached one is out of date
//! (`fingerprint`) and how the blur chain is set (`BlurPlan`).

use crate::census::Held;
use crate::gl::{GL_BLEND, GL_SCISSOR_TEST, Gles, RenderTarget};
use crate::log::log;
use crate::shaders::blur;
use compositor_protocol::LayerId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;

mod plan;
pub use self::plan::{Area, Ask, Backdrop, Layer, Refresh, overlaps, plan};

/// A number that stands for the layers below `upto`, at rest, and the blur: it is the same until one of them is
/// created, deleted, restyled, damaged or set somewhere else, and it does not change while one is animated or a
/// wallpaper moves. The surfaces that share a cached picture are not part of it: their shadows are drawn over it.
pub fn fingerprint(layers: &[Layer], upto: usize, radius: f32, downsample: u8) -> u64 {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |value: u64| {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    for layer in &layers[..upto] {
        if layer.rest.is_some() {
            mix(u64::from(layer.id.0));
            mix(layer.version);
        }
    }
    mix(u64::from(radius.to_bits()));
    mix(u64::from(downsample));
    hash
}

/// How the blur chain is set to blur as wide as asked: the number of halvings, the offset of the filters' taps in
/// texels of the texture they read, and how far back up the chain goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlurPlan {
    pub levels: u32,
    pub offset: f32,
    /// The level the picture is left at, 0 being `downsample` times smaller than the panel and each level half of the
    /// one before. The doubling passes below it are not made: glass reads the picture bilinearly, and a blur this wide
    /// is smooth at that size, so the biggest and dearest pass would only add texels nobody could tell.
    pub result: u32,
    /// The standard deviation of the blur this makes, in panel pixels.
    pub sigma: f32,
}

const MAX_LEVELS: u32 = 5;
/// Wider than this the five and eight taps land far enough apart to show as a pattern.
const MAX_OFFSET: f32 = 2.0;
/// The picture is left at the coarsest level whose texels are at most this fraction of the blur's standard deviation:
/// a Gaussian is smooth enough for bilinear reading at half a standard deviation to a texel.
const RESULT_TEXEL: f32 = 0.5;
/// The plan is found by running the chain on a row of this many level-0 texels (enough that nothing reaches its ends).
const SIMULATION: usize = 513;
/// Past this many standard deviations a Gaussian is under a quarter of a percent.
const REACH_SIGMAS: f32 = 3.2;

/// The taps of the halving filter along one axis: (offset in source texels, weight). The five taps of the shader
/// project onto the axis as the middle one and the four diagonal ones in pairs.
fn down_taps(offset: f64) -> [(f64, f64); 3] {
    [(0.0, 0.5), (-offset, 0.25), (offset, 0.25)]
}

/// The doubling filter's eight taps, projected onto one axis.
fn up_taps(offset: f64) -> [(f64, f64); 5] {
    let half = offset / 2.0;
    [
        (-offset, 1.0 / 12.0),
        (0.0, 2.0 / 12.0),
        (offset, 1.0 / 12.0),
        (-half, 4.0 / 12.0),
        (half, 4.0 / 12.0),
    ]
}

/// A row of texels read the way the GPU reads a texture: linearly between texel centres, clamped at the ends.
fn read(row: &[f64], u: f64) -> f64 {
    let n = row.len();
    let position = u * n as f64 - 0.5;
    let first = position.floor();
    let fraction = position - first;
    let at = |i: f64| row[i.clamp(0.0, n as f64 - 1.0) as usize];
    at(first) * (1.0 - fraction) + at(first + 1.0) * fraction
}

impl BlurPlan {
    /// A chain that goes all the way back up, for a blur of unknown width.
    #[cfg(test)]
    fn full(levels: u32, offset: f32) -> Self {
        Self {
            levels,
            offset,
            result: 0,
            sigma: 0.0,
        }
    }

    /// The standard deviation of the chain's blur, in panel pixels: one bright texel is put through the same taps and
    /// bilinear reads the GPU makes, and the spread of what comes out is measured. The filters' taps are diagonal, but
    /// what a row of the picture sees of them is the same as taps along the row: the chain's blur along one axis is
    /// worked out on a row.
    fn measure(levels: u32, offset: f32, result: u32, downsample: u8) -> f32 {
        let sizes: Vec<usize> = (0..=levels)
            .map(|level| SIMULATION.div_ceil(1 << level))
            .collect();
        let offset = f64::from(offset);
        let mut rows = vec![vec![0.0f64; SIMULATION]];
        rows[0][SIMULATION / 2] = 1.0;
        for level in 0..levels as usize {
            let (source, destination) = (sizes[level], sizes[level + 1]);
            let step = 1.0 / source as f64;
            let next: Vec<f64> = (0..destination)
                .map(|i| {
                    let u = (i as f64 + 0.5) / destination as f64;
                    down_taps(offset)
                        .iter()
                        .map(|(x, weight)| weight * read(&rows[level], u + x * step))
                        .sum()
                })
                .collect();
            rows.push(next);
        }
        for level in (result as usize..levels as usize).rev() {
            let source = sizes[level + 1];
            let step = 1.0 / source as f64;
            let coarse = rows[level + 1].clone();
            rows[level] = (0..sizes[level])
                .map(|i| {
                    let u = (i as f64 + 0.5) / sizes[level] as f64;
                    up_taps(offset)
                        .iter()
                        .map(|(x, weight)| weight * read(&coarse, u + x * step))
                        .sum()
                })
                .collect();
        }
        // What the glass sees: the picture at its level, read at the panel's resolution.
        let picture = &rows[result as usize];
        let centre = (SIMULATION / 2) as f64;
        let (mut mass, mut second) = (0.0, 0.0);
        for i in 0..SIMULATION {
            let value = read(picture, (i as f64 + 0.5) / SIMULATION as f64);
            mass += value;
            second += (i as f64 - centre).powi(2) * value;
        }
        ((second / mass).sqrt() * f64::from(downsample)) as f32
    }

    /// How far, in panel pixels, past a body the picture has to be right for a blur of this plan to be: the width of
    /// what matters in the blur, and the texels around it that the last filters read.
    pub fn reach(self, downsample: u8) -> i32 {
        (REACH_SIGMAS * self.sigma).ceil() as i32 + 2 * (i32::from(downsample) << self.result)
    }

    /// The fewest halvings that reach a blur of standard deviation `radius` with taps no wider than is good, and the
    /// offset that makes it exact. A blur too small for even one halving is none: the picture is only made small.
    pub fn for_radius(radius: f32, downsample: u8) -> Self {
        let result_for = |levels: u32| {
            (0..=levels)
                .rev()
                .find(|&k| f32::from(downsample) * (1u32 << k) as f32 <= RESULT_TEXEL * radius)
                .unwrap_or(0)
        };
        let plan = |levels: u32, offset: f32| {
            let result = result_for(levels);
            Self {
                levels,
                offset,
                result,
                sigma: Self::measure(levels, offset, result, downsample),
            }
        };
        let mut best = plan(0, 0.0);
        for levels in 1..=MAX_LEVELS {
            let narrowest = 0.25;
            if plan(levels, narrowest).sigma > radius {
                // Another halving would blur too much even with the taps close together.
                break;
            }
            if plan(levels, MAX_OFFSET).sigma < radius {
                best = plan(levels, MAX_OFFSET);
                continue;
            }
            // In between: the width grows with the offset, so bisect.
            let (mut low, mut high) = (narrowest, MAX_OFFSET);
            for _ in 0..14 {
                let middle = (low + high) / 2.0;
                if plan(levels, middle).sigma < radius {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            return plan(levels, (low + high) / 2.0);
        }
        best
    }
}

/// A texture-sized number of panel pixels: `panel` divided into `divisor`, rounded up.
fn texels(panel: u32, divisor: u32) -> u32 {
    panel.div_ceil(divisor).max(1)
}

/// The part of a texture that covers `area` of the panel, in texels, one texel wider all round (the filters read a
/// texel past it) and inside the texture.
pub fn texture_area(area: Area, panel: (u32, u32), texture: (u32, u32)) -> Area {
    let scale = |v: i32, panel: u32, texture: u32, up: bool| {
        let t = v as f32 * texture as f32 / panel as f32;
        (if up { t.ceil() } else { t.floor() }) as i32
    };
    [
        (scale(area[0], panel.0, texture.0, false) - 1).max(0),
        (scale(area[1], panel.1, texture.1, false) - 1).max(0),
        (scale(area[2], panel.0, texture.0, true) + 1).min(texture.0 as i32),
        (scale(area[3], panel.1, texture.1, true) + 1).min(texture.1 as i32),
    ]
}

struct BlurProgram {
    id: u32,
    texel: i32,
    offset: i32,
}

/// Render targets by size, so that nothing is allocated while frames are being drawn once the sizes in use have been
/// seen.
#[derive(Default)]
struct Pool {
    free: HashMap<(u32, u32), Vec<RenderTarget>>,
    /// In tests: how many more targets can be taken before the GPU has no room for another (`None`: it always has).
    #[cfg(test)]
    room: Option<usize>,
    /// In tests: how many takes the room refused.
    #[cfg(test)]
    refused: usize,
}

impl Pool {
    fn take(&mut self, gl: &Gles, width: u32, height: u32) -> Option<RenderTarget> {
        #[cfg(test)]
        if let Some(room) = self.room.as_mut() {
            let Some(left) = room.checked_sub(1) else {
                self.refused += 1;
                return None;
            };
            *room = left;
        }
        let target = match self.free.get_mut(&(width, height)).and_then(Vec::pop) {
            Some(target) => target,
            None => match gl.new_render_target(width, height) {
                Ok(target) => target,
                Err(e) => {
                    log(format!("cannot make a {width}x{height} render target: {e}"));
                    return None;
                }
            },
        };
        // What a target held before it was taken is another picture's, and a new one is clear: neither is a picture of what it
        // is taken for. In tests it is noise, so that a frame that shows what nothing drew into it is not the same as one
        // that does not (`gl::POISON`).
        #[cfg(test)]
        gl.poison_target(&target);
        Some(target)
    }

    fn give(&mut self, target: RenderTarget) {
        self.free
            .entry((target.width, target.height))
            .or_default()
            .push(target);
    }
}

/// A cached backdrop and what it was made of.
struct Kept {
    target: RenderTarget,
    fingerprint: u64,
    /// Asked for in the frame being drawn: those that are not are let go at its end.
    used: bool,
    /// The number of opened frames (`Backdrops::frames`) that last drew from it, or showed the glass that has it from a plate that
    /// stands: [`Backdrops::release_idle`] lets go of the ones that neither has done for a while.
    read: u64,
}

/// A backdrop picture ready to be drawn from.
#[derive(Clone, Copy, Debug)]
pub struct Made {
    pub texture: u32,
    /// Whether this frame had to make it: a live one always, a cached one only when it was out of date.
    pub remade: bool,
}

/// What a chain of blur passes is a picture of: `size` pixels of something (the panel, or what a plate stands for), the
/// first level `downsample` times smaller than that, and how the passes are set.
#[derive(Clone, Copy)]
struct Chain {
    size: (u32, u32),
    downsample: u8,
    plan: BlurPlan,
}

pub struct Backdrops {
    down: BlurProgram,
    up: BlurProgram,
    corners: u32,
    panel: (u32, u32),
    pool: Pool,
    cache: HashMap<LayerId, Kept>,
    /// The live pictures of the frame being drawn, given back when the next one starts.
    live: Vec<RenderTarget>,
    /// Frames that drew anything (`begin_frame` counts them).
    frames: u64,
    plans: RefCell<HashMap<(u32, u8), BlurPlan>>,
}

impl Backdrops {
    pub fn new(gl: &Gles, panel: (u32, u32)) -> io::Result<Self> {
        let vertex = blur::vertex();
        let program = |fragment| -> io::Result<BlurProgram> {
            let id = gl.program(&vertex, fragment)?;
            gl.use_program(id);
            gl.uniform1i(gl.uniform_location(id, c"u_source"), 0);
            Ok(BlurProgram {
                id,
                texel: gl.uniform_location(id, c"u_texel"),
                offset: gl.uniform_location(id, c"u_offset"),
            })
        };
        let corners = gl.new_vertex_buffer();
        gl.static_vertices(
            corners,
            &[
                [-1.0, -1.0, 0.0, 0.0],
                [1.0, -1.0, 0.0, 0.0],
                [-1.0, 1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0, 0.0],
            ],
        );
        Ok(Self {
            down: program(&blur::down())?,
            up: program(&blur::up())?,
            corners,
            panel,
            pool: Pool::default(),
            cache: HashMap::new(),
            live: Vec::new(),
            frames: 0,
            plans: RefCell::new(HashMap::new()),
        })
    }

    #[cfg(test)]
    /// From now on the GPU has room for `room` more of the targets the pool makes pictures of (`None`: as much as is asked).
    pub(crate) fn set_room(&mut self, room: Option<usize>) {
        self.pool.room = room;
    }

    #[cfg(test)]
    /// How many takes of a target the room set by [`Backdrops::set_room`] has refused.
    pub(crate) fn refused(&self) -> usize {
        self.pool.refused
    }

    #[cfg(test)]
    /// Whether a cached picture is kept for the surface `leader`.
    pub(crate) fn keeps(&self, leader: LayerId) -> bool {
        self.cache.contains_key(&leader)
    }
    /// Adds what the pool, the cache and the live pictures hold to `census`.
    pub fn census(&self, census: &mut crate::census::Census) {
        for (&(w, h), free) in &self.pool.free {
            for _ in free {
                census.pool_free.texture(w, h, 4);
            }
            if !free.is_empty() {
                *census.pool_sizes.entry((w, h)).or_default() += free.len() as u32;
            }
        }
        for kept in self.cache.values() {
            census
                .backdrops
                .texture(kept.target.width, kept.target.height, 4);
        }
        for target in &self.live {
            census.backdrops.texture(target.width, target.height, 4);
        }
    }

    /// The chain that blurs `radius` at 1/`downsample`, found once and remembered.
    pub fn plan_for(&self, radius: f32, downsample: u8) -> BlurPlan {
        *self
            .plans
            .borrow_mut()
            .entry((radius.to_bits(), downsample))
            .or_insert_with(|| BlurPlan::for_radius(radius, downsample))
    }

    /// How far, in panel pixels, past a body a blur of `radius` at 1/`downsample` has to be right.
    pub fn reach(&self, radius: f32, downsample: u8) -> i32 {
        self.plan_for(radius, downsample).reach(downsample)
    }

    /// The size, in texels, of the picture of a backdrop asked for as `backdrop`.
    fn picture_size(&self, backdrop: &Backdrop) -> (u32, u32) {
        Self::finished_size(
            self.plan_for(backdrop.radius, backdrop.downsample),
            self.panel,
            backdrop.downsample,
        )
    }

    /// The size, in texels, of the finished picture of a blur `plan` at 1/`downsample` of something `size` pixels big: the
    /// coarsest level of the chain it is left at.
    pub fn finished_size(plan: BlurPlan, size: (u32, u32), downsample: u8) -> (u32, u32) {
        let divisor = u32::from(downsample) << plan.result;
        (texels(size.0, divisor), texels(size.1, divisor))
    }

    /// Whether `picture` would have to make `backdrop` (whose first surface is `leader`) again: a live one always, a
    /// cached one when it has none, or one of another size, or one made of layers that have since changed.
    pub fn is_stale(&self, backdrop: &Backdrop, leader: LayerId, fingerprint: u64) -> bool {
        match backdrop.refresh {
            Refresh::Live => true,
            Refresh::Cached => {
                let size = self.picture_size(backdrop);
                self.cache.get(&leader).is_none_or(|kept| {
                    (kept.target.width, kept.target.height) != size
                        || kept.fingerprint != fingerprint
                })
            }
        }
    }

    /// Keeps the cached picture of the surface `leader` through a frame that draws none of its surfaces.
    pub fn keep(&mut self, leader: LayerId) {
        if let Some(kept) = self.cache.get_mut(&leader) {
            kept.used = true;
        }
    }

    /// Keeps the cached picture of the surface `leader` through a frame that shows its surfaces from plates that stand: nothing
    /// is drawn from the picture, but the glass is shown, and the picture is what its plates are made again from.
    pub fn standing(&mut self, leader: LayerId) {
        if let Some(kept) = self.cache.get_mut(&leader) {
            kept.used = true;
            kept.read = self.frames;
        }
    }

    pub fn begin_frame(&mut self) {
        self.frames += 1;
        for target in self.live.drain(..) {
            self.pool.give(target);
        }
        for kept in self.cache.values_mut() {
            kept.used = false;
        }
    }

    /// Lets go of what only makes the next frame cheaper, for a compositor that has nothing to draw: every render target of the
    /// pool (the free ones, and the live pictures of the last frame, which the next frame makes again whether they are there or
    /// not) and the cached pictures that no frame has drawn from, or shown glass from plates over, in the last `unread` frames.
    /// The picture of glass that is shown is kept: making it again is the blur chain, and the glass shown from a plate that
    /// stands is not drawn from it until it moves. What was held is counted in the first (the pool, in `Held`s) and the second
    /// (the cached pictures).
    pub fn release_idle(&mut self, gl: &Gles, unread: u64) -> (Held, Held) {
        let mut pool = Held::default();
        let mut cached = Held::default();
        for target in self.live.drain(..) {
            self.pool.give(target);
        }
        for (_, free) in std::mem::take(&mut self.pool.free) {
            for target in free {
                pool.texture(target.width, target.height, 4);
                gl.delete_render_target(target);
            }
        }
        let frames = self.frames;
        let old: Vec<LayerId> = self
            .cache
            .iter()
            .filter(|(_, kept)| frames.saturating_sub(kept.read) >= unread)
            .map(|(id, _)| *id)
            .collect();
        for id in old {
            if let Some(kept) = self.cache.remove(&id) {
                cached.texture(kept.target.width, kept.target.height, 4);
                gl.delete_render_target(kept.target);
            }
        }
        (pool, cached)
    }

    /// Lets go of the cached backdrops nothing asked for in the frame: their surface is gone or has changed.
    pub fn end_frame(&mut self) {
        let unused: Vec<LayerId> = self
            .cache
            .iter()
            .filter(|(_, kept)| !kept.used)
            .map(|(id, _)| *id)
            .collect();
        for id in unused {
            if let Some(kept) = self.cache.remove(&id) {
                self.pool.give(kept.target);
            }
        }
    }

    /// The picture for `backdrop`, whose first surface is `leader`. `mirror` is called to draw the layers below into the
    /// render target it is given (bound, and 1/`downsample` of the panel), within the part of it that covers the
    /// area it is given; it is not called for a cached picture that is up to date. `None` if the GPU has no room for
    /// it: the surface is then drawn without a backdrop.
    pub fn picture(
        &mut self,
        gl: &Gles,
        backdrop: &Backdrop,
        leader: LayerId,
        fingerprint: u64,
        mirror: impl FnOnce(&RenderTarget, Area),
    ) -> Option<Made> {
        let plan = self.plan_for(backdrop.radius, backdrop.downsample);
        let size = self.picture_size(backdrop);
        // The texture the picture ends up in.
        let picture = match backdrop.refresh {
            Refresh::Live => {
                let target = self.pool.take(gl, size.0, size.1)?;
                self.live.push(target);
                target
            }
            Refresh::Cached => {
                if let Some(kept) = self.cache.get_mut(&leader) {
                    if (kept.target.width, kept.target.height) == size
                        && kept.fingerprint == fingerprint
                    {
                        kept.used = true;
                        kept.read = self.frames;
                        return Some(Made {
                            texture: kept.target.texture,
                            remade: false,
                        });
                    }
                }
                // Out of date, or new: make it in the texture it had if that is the right size.
                let target = match self.cache.remove(&leader) {
                    Some(kept) if (kept.target.width, kept.target.height) == size => kept.target,
                    Some(kept) => {
                        self.pool.give(kept.target);
                        self.pool.take(gl, size.0, size.1)?
                    }
                    None => self.pool.take(gl, size.0, size.1)?,
                };
                self.cache.insert(
                    leader,
                    Kept {
                        target,
                        fingerprint,
                        used: true,
                        read: self.frames,
                    },
                );
                target
            }
        };
        let chain = Chain {
            size: self.panel,
            downsample: backdrop.downsample,
            plan,
        };
        if !self.make(gl, chain, backdrop.area, picture, mirror) {
            // The chain could not be made, so nothing was drawn into the picture: it is not shown, and not kept either (a
            // cached one is taken for up to date by the frames after this one, where nothing would make it again).
            match backdrop.refresh {
                Refresh::Live => {
                    if let Some(target) = self.live.pop() {
                        self.pool.give(target);
                    }
                }
                Refresh::Cached => {
                    if let Some(kept) = self.cache.remove(&leader) {
                        self.pool.give(kept.target);
                    }
                }
            }
            return None;
        }
        Some(Made {
            texture: picture.texture,
            remade: true,
        })
    }

    /// A blurred picture of whatever `mirror` draws, for the plates of a prepared backdrop: `size` pixels of something,
    /// drawn by `mirror` into the render target it is given (bound by `mirror`, as pixels `downsample` times larger than
    /// the target's, the whole of it) and blurred by `radius` like a glass surface's own. The picture is left at the
    /// chain's coarsest level ([`Backdrops::finished_size`]) and is the caller's until [`Backdrops::give`] takes it back;
    /// `None` when the GPU has no room for it or for the chain.
    pub fn blurred(
        &mut self,
        gl: &Gles,
        size: (u32, u32),
        radius: f32,
        downsample: u8,
        mirror: impl FnOnce(&RenderTarget),
    ) -> Option<RenderTarget> {
        let plan = self.plan_for(radius, downsample);
        let (w, h) = Self::finished_size(plan, size, downsample);
        let picture = self.pool.take(gl, w, h)?;
        let chain = Chain {
            size,
            downsample,
            plan,
        };
        let whole = [0, 0, size.0 as i32, size.1 as i32];
        if self.make(gl, chain, whole, picture, |target, _| mirror(target)) {
            Some(picture)
        } else {
            self.pool.give(picture);
            None
        }
    }

    /// A render target of `width` x `height` texels with undefined contents, from the pool of the ones the pictures use;
    /// `None` when the GPU has no room for one.
    pub fn take(&mut self, gl: &Gles, width: u32, height: u32) -> Option<RenderTarget> {
        self.pool.take(gl, width, height)
    }

    /// Gives a render target made by [`Backdrops::blurred`] or [`Backdrops::take`] back to the pool.
    pub fn give(&mut self, target: RenderTarget) {
        self.pool.give(target);
    }

    /// Draws the mirror and blurs it down and back up to the level of `picture`. Whether the GPU had room for the chain (when
    /// it had not, the mirror is not drawn and the picture holds nothing).
    fn make(
        &mut self,
        gl: &Gles,
        chain: Chain,
        area: Area,
        picture: RenderTarget,
        mirror: impl FnOnce(&RenderTarget, Area),
    ) -> bool {
        let Chain {
            size,
            downsample,
            plan,
        } = chain;
        // One texture per level: the picture's own where it belongs, the rest borrowed from the pool for the time.
        let d = u32::from(downsample);
        let mut levels: Vec<RenderTarget> = Vec::new();
        let mut borrowed: Vec<RenderTarget> = Vec::new();
        for level in 0..=plan.levels {
            if level == plan.result {
                levels.push(picture);
                continue;
            }
            let (w, h) = (texels(size.0, d << level), texels(size.1, d << level));
            match self.pool.take(gl, w, h) {
                Some(target) => {
                    levels.push(target);
                    borrowed.push(target);
                }
                None => {
                    // No room for the chain: give back what was taken.
                    for target in borrowed {
                        self.pool.give(target);
                    }
                    return false;
                }
            }
        }
        mirror(&levels[0], area);
        self.blur(gl, size, &levels, plan, area);
        for target in borrowed {
            self.pool.give(target);
        }
        true
    }

    /// Blurs the picture in `levels[0]` down through the rest of `levels` and back up to `levels[plan.result]`, within
    /// `area` (pixels of something `size` big, which the levels are pictures of).
    fn blur(
        &self,
        gl: &Gles,
        size: (u32, u32),
        levels: &[RenderTarget],
        plan: BlurPlan,
        area: Area,
    ) {
        if plan.levels == 0 {
            return;
        }
        gl.disable(GL_BLEND);
        gl.enable(GL_SCISSOR_TEST);
        gl.use_vertex_buffer(self.corners);
        let pass = |program: &BlurProgram, from: RenderTarget, to: RenderTarget| {
            let region = texture_area(area, size, (to.width, to.height));
            gl.bind_framebuffer(to.framebuffer);
            gl.viewport(0, 0, to.width as i32, to.height as i32);
            gl.scissor(
                region[0],
                region[1],
                region[2] - region[0],
                region[3] - region[1],
            );
            gl.use_program(program.id);
            gl.active_texture(0);
            gl.bind_texture(from.texture);
            gl.uniform2f(
                program.texel,
                1.0 / from.width as f32,
                1.0 / from.height as f32,
            );
            gl.uniform1f(program.offset, plan.offset);
            gl.draw_triangle_strip(0, 4);
        };
        for k in 0..plan.levels as usize {
            pass(&self.down, levels[k], levels[k + 1]);
        }
        for k in (plan.result as usize..plan.levels as usize).rev() {
            pass(&self.up, levels[k + 1], levels[k]);
        }
        gl.enable(GL_BLEND);
        gl.disable(GL_SCISSOR_TEST);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANEL: Area = [0, 0, 480, 960];

    fn plain(id: u32, version: u64, area: Area) -> Layer {
        Layer {
            id: LayerId(id),
            version,
            now: Some(area),
            rest: Some(area),
            ask: None,
        }
    }

    fn glass(id: u32, body: Area, refresh: Refresh, radius: f32, downsample: u8) -> Layer {
        Layer {
            ask: Some(Ask {
                refresh,
                radius,
                downsample,
                body_now: body,
                body_rest: body,
                sample_now: body,
                sample_rest: body,
            }),
            ..plain(id, 1, body)
        }
    }

    fn reach(_: f32, _: u8) -> i32 {
        10
    }

    #[test]
    fn surfaces_side_by_side_share_a_backdrop_and_it_covers_all_their_bodies_and_the_blur_around() {
        let layers = [
            plain(1, 1, PANEL),
            glass(2, [20, 20, 120, 100], Refresh::Live, 30.0, 4),
            glass(3, [200, 20, 300, 100], Refresh::Live, 30.0, 4),
        ];
        let backdrops = plan(&layers, PANEL, reach);
        assert_eq!(backdrops.len(), 1);
        assert_eq!(backdrops[0].upto, 1);
        assert_eq!(backdrops[0].users, vec![1, 2]);
        // From the first body's left edge less the reach (held at the panel's), to the second's right edge plus it.
        assert_eq!(backdrops[0].area, [10, 10, 310, 110]);
    }

    #[test]
    fn what_is_drawn_over_a_surfaces_body_since_a_backdrop_was_made_needs_a_new_one() {
        // A plain layer in between does.
        let layers = [
            plain(1, 1, PANEL),
            glass(2, [20, 20, 120, 100], Refresh::Live, 30.0, 4),
            plain(4, 1, [250, 50, 260, 60]),
            glass(3, [200, 20, 300, 100], Refresh::Live, 30.0, 4),
        ];
        assert_eq!(plan(&layers, PANEL, reach).len(), 2);
        // One that is nowhere near does not.
        let layers = [
            plain(1, 1, PANEL),
            glass(2, [20, 20, 120, 100], Refresh::Live, 30.0, 4),
            plain(4, 1, [250, 400, 260, 410]),
            glass(3, [200, 20, 300, 100], Refresh::Live, 30.0, 4),
        ];
        assert_eq!(plan(&layers, PANEL, reach).len(), 1);
        // Another surface's body over the body does: the second surface sees the first in its backdrop, and so a second
        // picture is made, of what is below the second.
        let layers = [
            plain(1, 1, PANEL),
            glass(2, [20, 20, 120, 100], Refresh::Live, 30.0, 4),
            glass(3, [100, 50, 300, 100], Refresh::Live, 30.0, 4),
        ];
        let backdrops = plan(&layers, PANEL, reach);
        assert_eq!(backdrops.len(), 2);
        assert_eq!((backdrops[0].upto, backdrops[1].upto), (1, 2));
    }

    #[test]
    fn surfaces_whose_shadows_fall_on_each_others_bodies_still_share_a_backdrop() {
        // Lucent's cards: each one's shadow reaches past its body, over the next one's. The shadows are cast into the
        // picture and the bodies see them through the glass, so that one picture will do.
        for refresh in [Refresh::Live, Refresh::Cached] {
            let card = |id, x: i32| {
                let body = [x, 96, x + 288, 246];
                let mut layer = glass(id, body, refresh, 30.0, 4);
                // The shadow's reach: 60 to the sides, 36 above, 84 below.
                let around = Some([x - 60, 60, x + 348, 330]);
                layer.now = around;
                layer.rest = around;
                layer
            };
            let layers = [plain(1, 1, PANEL), card(2, 24), card(3, 336), card(4, 648)];
            let backdrops = plan(&layers, PANEL, reach);
            assert_eq!(backdrops.len(), 1, "{refresh:?}");
            assert_eq!(backdrops[0].users, vec![1, 2, 3]);
            assert_eq!(backdrops[0].upto, 1);
        }
    }

    #[test]
    fn what_is_not_a_surface_of_the_picture_still_blocks_it_by_its_shadow_or_whatever_it_draws() {
        // A surface with a blur of its own between two that would share: its shadow falls on the second's body.
        let mut other = glass(3, [130, 20, 190, 100], Refresh::Live, 10.0, 4);
        other.now = Some([100, 0, 260, 180]);
        let layers = [
            plain(1, 1, PANEL),
            glass(2, [20, 20, 120, 100], Refresh::Live, 30.0, 4),
            other,
            glass(4, [200, 20, 300, 100], Refresh::Live, 30.0, 4),
        ];
        let backdrops = plan(&layers, PANEL, reach);
        assert_eq!(backdrops.len(), 3, "{backdrops:?}");
    }

    #[test]
    fn different_blurs_and_different_ways_of_keeping_a_backdrop_do_not_share() {
        let layers = [
            plain(1, 1, PANEL),
            glass(2, [20, 20, 120, 100], Refresh::Live, 30.0, 4),
            glass(3, [200, 20, 300, 100], Refresh::Live, 20.0, 4),
            glass(4, [20, 200, 120, 300], Refresh::Live, 30.0, 8),
            glass(5, [200, 200, 300, 300], Refresh::Cached, 30.0, 4),
        ];
        assert_eq!(plan(&layers, PANEL, reach).len(), 4);
    }

    #[test]
    fn a_cached_backdrop_is_of_the_layers_at_rest_and_covers_the_whole_panel() {
        // The first surface is being animated: now it covers the second's body, at rest it does not.
        let mut first = glass(2, [20, 20, 120, 100], Refresh::Cached, 30.0, 4);
        first.now = Some([0, 0, 400, 400]);
        first.ask.as_mut().unwrap().body_now = [0, 0, 400, 400];
        let layers = [
            plain(1, 1, PANEL),
            first,
            glass(3, [200, 20, 300, 100], Refresh::Cached, 30.0, 4),
        ];
        let backdrops = plan(&layers, PANEL, reach);
        assert_eq!(
            backdrops.len(),
            1,
            "an animation does not split a cached backdrop"
        );
        assert_eq!(backdrops[0].area, PANEL);
        // The same layers asked for live are split by it.
        let mut live_first = glass(2, [20, 20, 120, 100], Refresh::Live, 30.0, 4);
        live_first.now = Some([0, 0, 400, 400]);
        live_first.ask.as_mut().unwrap().body_now = [0, 0, 400, 400];
        let layers = [
            plain(1, 1, PANEL),
            live_first,
            glass(3, [200, 20, 300, 100], Refresh::Live, 30.0, 4),
        ];
        assert_eq!(plan(&layers, PANEL, reach).len(), 2);
    }

    #[test]
    fn layers_not_shown_at_all_are_not_in_the_way() {
        let mut hidden = plain(4, 1, [250, 50, 260, 60]);
        hidden.now = None;
        hidden.rest = None;
        let layers = [
            plain(1, 1, PANEL),
            glass(2, [20, 20, 120, 100], Refresh::Live, 30.0, 4),
            hidden,
            glass(3, [200, 20, 300, 100], Refresh::Live, 30.0, 4),
        ];
        assert_eq!(plan(&layers, PANEL, reach).len(), 1);
    }

    #[test]
    fn a_frame_with_no_glass_needs_no_backdrops() {
        let layers = [plain(1, 1, PANEL), plain(2, 1, [0, 0, 10, 10])];
        assert!(plan(&layers, PANEL, reach).is_empty());
    }

    #[test]
    fn the_fingerprint_follows_what_is_below_and_ignores_animation_and_what_is_above() {
        let base = [
            plain(1, 1, PANEL),
            plain(2, 4, [10, 10, 50, 50]),
            plain(3, 9, [60, 60, 90, 90]),
        ];
        let same = fingerprint(&base, 2, 30.0, 4);
        assert_eq!(fingerprint(&base, 2, 30.0, 4), same);
        // The layer above the backdrop changes: not this backdrop's business.
        let mut above = base;
        above[2].version = 10;
        assert_eq!(fingerprint(&above, 2, 30.0, 4), same);
        // A layer below is animated: where it is drawn now changes, what it is and where it rests do not.
        let mut moving = base;
        moving[1].now = Some([200, 200, 240, 240]);
        assert_eq!(fingerprint(&moving, 2, 30.0, 4), same);
        // A layer below is restyled, damaged or set: its version moved.
        let mut changed = base;
        changed[1].version = 5;
        assert_ne!(fingerprint(&changed, 2, 30.0, 4), same);
        // One is created or deleted below.
        assert_ne!(fingerprint(&base, 3, 30.0, 4), same);
        let deleted = [base[0], base[2]];
        assert_ne!(
            fingerprint(&deleted, 1, 30.0, 4),
            fingerprint(&base, 2, 30.0, 4)
        );
        // The blur is part of it.
        assert_ne!(fingerprint(&base, 2, 20.0, 4), same);
        assert_ne!(fingerprint(&base, 2, 30.0, 8), same);
        // A layer not shown at rest is not part of the picture.
        let mut hidden = base;
        hidden[1].rest = None;
        hidden[1].version = 77;
        let mut hidden_other = base;
        hidden_other[1].rest = None;
        hidden_other[1].version = 78;
        assert_eq!(
            fingerprint(&hidden, 2, 30.0, 4),
            fingerprint(&hidden_other, 2, 30.0, 4)
        );
    }

    /// The blur of the chain measured in two dimensions with the shader's own diagonal taps: a single bright texel goes
    /// through them and the spread of what comes out along x is the standard deviation.
    fn measured_sigma_2d(plan: BlurPlan, downsample: u8) -> f64 {
        let n0 = 257usize;
        let sizes: Vec<usize> = (0..=plan.levels).map(|l| n0.div_ceil(1 << l)).collect();
        let sample = |data: &[f64], n: usize, u: f64, v: f64| -> f64 {
            let (px, py) = (u * n as f64 - 0.5, v * n as f64 - 0.5);
            let (x0, y0) = (px.floor(), py.floor());
            let (fx, fy) = (px - x0, py - y0);
            let at = |x: f64, y: f64| {
                data[(y.clamp(0.0, n as f64 - 1.0) as usize) * n
                    + (x.clamp(0.0, n as f64 - 1.0) as usize)]
            };
            let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
            let bottom = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
            top * (1.0 - fy) + bottom * fy
        };
        let o = f64::from(plan.offset);
        let mut ladder = vec![vec![0.0f64; n0 * n0]];
        ladder[0][(n0 / 2) * n0 + n0 / 2] = 1.0;
        for level in 0..plan.levels as usize {
            let (ns, nd) = (sizes[level], sizes[level + 1]);
            let source = ladder[level].clone();
            let mut dest = vec![0.0f64; nd * nd];
            for j in 0..nd {
                for i in 0..nd {
                    let (u, v) = ((i as f64 + 0.5) / nd as f64, (j as f64 + 0.5) / nd as f64);
                    let t = o / ns as f64;
                    let mut sum = 4.0 * sample(&source, ns, u, v);
                    for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                        sum += sample(&source, ns, u + dx * t, v + dy * t);
                    }
                    dest[j * nd + i] = sum / 8.0;
                }
            }
            ladder.push(dest);
        }
        for level in (0..plan.levels as usize).rev() {
            let (ns, nd) = (sizes[level + 1], sizes[level]);
            let source = ladder[level + 1].clone();
            let mut dest = vec![0.0f64; nd * nd];
            for j in 0..nd {
                for i in 0..nd {
                    let (u, v) = ((i as f64 + 0.5) / nd as f64, (j as f64 + 0.5) / nd as f64);
                    let a = o / ns as f64;
                    let h = a / 2.0;
                    let mut sum = 0.0;
                    for (dx, dy) in [(-a, 0.0), (0.0, a), (a, 0.0), (0.0, -a)] {
                        sum += sample(&source, ns, u + dx, v + dy);
                    }
                    for (dx, dy) in [(-h, h), (h, h), (h, -h), (-h, -h)] {
                        sum += 2.0 * sample(&source, ns, u + dx, v + dy);
                    }
                    dest[j * nd + i] = sum / 12.0;
                }
            }
            ladder[level] = dest;
        }
        let out = &ladder[0];
        let (mut mass, mut second) = (0.0, 0.0);
        for j in 0..n0 {
            for i in 0..n0 {
                let x = i as f64 - (n0 / 2) as f64;
                mass += out[j * n0 + i];
                second += x * x * out[j * n0 + i];
            }
        }
        (second / mass).sqrt() * f64::from(downsample)
    }

    #[test]
    fn the_row_the_plan_is_found_on_gives_the_width_of_the_real_two_dimensional_chain_within_twelve_percent()
     {
        for downsample in [2u8, 4, 8] {
            for (levels, offset) in [(1, 1.0), (2, 0.75), (2, 1.5), (3, 1.0), (3, 2.0), (4, 1.25)] {
                let plan = BlurPlan::full(levels, offset);
                let row = f64::from(BlurPlan::measure(levels, offset, 0, downsample));
                let plane = measured_sigma_2d(plan, downsample);
                // Not exact: at the coarse levels the two dimensions are not independent (the halving samples the
                // other dimension's profile at a phase), and the row overstates a four-halving chain by about a tenth.
                // A blur a tenth narrower than asked is nothing to the eye.
                assert!(
                    (row - plane).abs() <= 0.12 * plane + 0.05,
                    "d={downsample} {plan:?}: row {row}, plane {plane}"
                );
            }
        }
    }

    #[test]
    fn the_chain_is_set_to_blur_as_wide_as_asked() {
        for downsample in [2u8, 4, 8] {
            for radius in [6.0f32, 12.0, 20.0, 30.0, 45.0, 64.0] {
                let plan = BlurPlan::for_radius(radius, downsample);
                assert!(
                    plan.levels <= MAX_LEVELS && plan.offset <= MAX_OFFSET + 1e-4,
                    "{plan:?}"
                );
                let narrowest = BlurPlan::measure(1, 0.25, 0, downsample);
                if radius < narrowest {
                    // Narrower than the coarsest picture's blur can be: only the picture's own softness is left.
                    assert_eq!(plan.levels, 0, "d={downsample} r={radius}");
                    continue;
                }
                assert!(
                    (plan.sigma - radius).abs() <= 0.03 * radius + 0.3,
                    "d={downsample} r={radius}: {plan:?}"
                );
            }
        }
    }

    #[test]
    fn a_coarser_picture_needs_fewer_passes_and_nothing_to_blur_needs_none() {
        for radius in [15.0f32, 30.0, 60.0] {
            let levels = |d| BlurPlan::for_radius(radius, d).levels;
            assert!(levels(8) <= levels(4) && levels(4) <= levels(2), "{radius}");
        }
        assert_eq!(BlurPlan::for_radius(0.0, 4).levels, 0);
    }

    #[test]
    fn a_pixel_beyond_the_reach_is_untouched_by_the_chain() {
        for (radius, downsample) in [(30.0f32, 4u8), (64.0, 2), (12.0, 8)] {
            let plan = BlurPlan::for_radius(radius, downsample);
            let reach = plan.reach(downsample);
            // Three standard deviations is nearly all of a Gaussian; the reach must be more than that.
            assert!(reach as f32 > 3.0 * plan.sigma, "{plan:?}");
        }
    }

    // --- The whole thing, drawn by the renderer into a software GL. ---

    use crate::testgl::{Shot, drawn, rig};
    use compositor_protocol::{Backdrop as Ask_, Content, Fill, Glass, Props, Rgba, Shape};

    fn shape(colour: Rgba) -> Content {
        Content::Shape(Shape {
            corner_radius: 0.0,
            fill: Fill::Solid(colour),
            border_width: 0.0,
            border: Rgba(0, 0, 0, 0),
        })
    }

    /// White on the left half of the screen, black on the right: a sharp edge for a backdrop to blur.
    fn split() -> Vec<crate::scene::Drawn> {
        vec![
            drawn(
                1,
                (960, 480),
                shape(Rgba(255, 255, 255, 255)),
                Props::at(0.0, 0.0),
            ),
            drawn(
                2,
                (480, 480),
                shape(Rgba(0, 0, 0, 255)),
                Props::at(480.0, 0.0),
            ),
        ]
    }

    /// Glass that shows its backdrop and nothing else.
    fn clear_glass(backdrop: Ask_) -> Glass {
        Glass { corner_radius: 0.0,
        tint: Rgba(0, 0, 0, 0),
        edge: Rgba(0, 0, 0, 0),
        edge_width: 0.0,
        highlight: Rgba(0, 0, 0, 0),
        sheen: None,
        shadows: [None, None],
        depth: None,
        backdrop,
        saturation: 1.0, refraction: 0.0, edge_lens: None, tint_gradient: None, }
    }

    fn glass_layer(id: u32, glass: Glass, size: (u32, u32), at: (f32, f32)) -> crate::scene::Drawn {
        drawn(id, size, Content::Glass(glass), Props::at(at.0, at.1))
    }

    fn live(radius: f32, downsample: u8) -> Ask_ {
        Ask_::Live { radius, downsample }
    }

    fn red(shot: &Shot, x: u32, y: u32) -> i32 {
        i32::from(shot.pixel(x, y)[0])
    }

    #[test]
    fn a_live_backdrop_is_the_blurred_picture_of_what_is_below_and_only_under_the_glass() {
        let Some(mut rig) = rig() else { return };
        let mut layers = split();
        layers.push(glass_layer(
            3,
            clear_glass(live(20.0, 4)),
            (300, 150),
            (330.0, 150.0),
        ));
        let shot = rig.draw(&layers);
        shot.save("backdrop-live");
        assert_eq!(rig.report.backdrops, 1);
        // Far from the edge the picture is as it was; at the edge it is half; a blur radius either side of it, about
        // the Gaussian's 84 % and 16 %.
        assert!(red(&shot, 345, 225) >= 250, "{}", red(&shot, 345, 225));
        assert!(red(&shot, 615, 225) <= 5, "{}", red(&shot, 615, 225));
        assert!(
            (red(&shot, 480, 225) - 128).abs() <= 12,
            "{}",
            red(&shot, 480, 225)
        );
        assert!(
            (red(&shot, 460, 225) - 214).abs() <= 28,
            "{}",
            red(&shot, 460, 225)
        );
        assert!(
            (red(&shot, 500, 225) - 41).abs() <= 28,
            "{}",
            red(&shot, 500, 225)
        );
        let ramp: Vec<i32> = (400..=560).step_by(8).map(|x| red(&shot, x, 225)).collect();
        assert!(
            ramp.windows(2).all(|w| w[1] <= w[0]),
            "falls steadily: {ramp:?}"
        );
        // Outside the glass the picture is sharp.
        assert_eq!(red(&shot, 479, 100), 255);
        assert_eq!(red(&shot, 480, 100), 0);
        assert_eq!(red(&shot, 479, 320), 255);
        assert_eq!(red(&shot, 480, 320), 0);
    }

    #[test]
    fn a_cached_backdrop_is_made_once_and_again_only_when_a_layer_below_changes() {
        let Some(mut rig) = rig() else { return };
        let glass = clear_glass(Ask_::Cached { radius: 20.0 });
        let mut layers = split();
        layers.push(glass_layer(3, glass, (300, 150), (330.0, 150.0)));
        let first = rig.draw(&layers);
        assert_eq!(rig.report.backdrops, 1);
        let blurred = red(&first, 480, 225);
        assert!((blurred - 128).abs() <= 12, "{blurred}");
        // The same frame again: nothing is made, and it looks the same.
        let again = rig.draw(&layers);
        assert_eq!(rig.report.backdrops, 0);
        assert_eq!(red(&again, 480, 225), blurred);
        // The glass itself is moved (set: a new version for it, not for what is below): still nothing to make, and
        // the picture follows it.
        layers[2] = glass_layer(3, glass, (300, 150), (380.0, 200.0));
        layers[2].version = 2;
        let moved = rig.draw(&layers);
        assert_eq!(rig.report.backdrops, 0);
        assert!((red(&moved, 480, 275) - 128).abs() <= 12);
        assert!(red(&moved, 395, 275) >= 250, "{}", red(&moved, 395, 275));
        // What is below is animated: it is drawn elsewhere now, but rests where it did, so the picture is not made
        // again and shows it as it rests.
        layers[1].props.x = 560.0;
        let sliding = rig.draw(&layers);
        assert_eq!(rig.report.backdrops, 0);
        assert_eq!(
            red(&sliding, 500, 100),
            255,
            "the animated layer is drawn where it is now"
        );
        assert!(
            (red(&sliding, 480, 275) - 128).abs() <= 12,
            "the picture is of it at rest"
        );
        // What is below is changed for good: the backdrop is made again, once.
        layers[1].props.x = 560.0;
        layers[1].rest.x = 560.0;
        layers[1].version = 9;
        let changed = rig.draw(&layers);
        assert_eq!(rig.report.backdrops, 1);
        assert!(
            red(&changed, 480, 275) >= 250,
            "the edge has moved away: {}",
            red(&changed, 480, 275)
        );
        rig.draw(&layers);
        assert_eq!(rig.report.backdrops, 0);
    }

    #[test]
    fn a_live_backdrop_is_made_afresh_every_frame() {
        let Some(mut rig) = rig() else { return };
        let mut layers = split();
        layers.push(glass_layer(
            3,
            clear_glass(live(20.0, 4)),
            (300, 150),
            (330.0, 150.0),
        ));
        for _ in 0..3 {
            rig.draw(&layers);
            assert_eq!(rig.report.backdrops, 1);
        }
    }

    #[test]
    fn surfaces_side_by_side_share_one_picture_and_overlapping_ones_need_two() {
        let Some(mut rig) = rig() else { return };
        // Pictures as drawn with no glass idea on: adaptive would hold the moved surface's picture instead of making it.
        rig.renderer.apply(&crate::ops::Op::Ideas(0));
        let mut layers = split();
        layers.push(glass_layer(
            3,
            clear_glass(live(20.0, 4)),
            (100, 100),
            (100.0, 100.0),
        ));
        layers.push(glass_layer(
            4,
            clear_glass(live(20.0, 4)),
            (100, 100),
            (300.0, 100.0),
        ));
        rig.draw(&layers);
        assert_eq!(rig.report.backdrops, 1);
        // The second sits over the first.
        layers[3] = glass_layer(4, clear_glass(live(20.0, 4)), (100, 100), (150.0, 100.0));
        rig.draw(&layers);
        assert_eq!(rig.report.backdrops, 2);
        // Another blur is another picture.
        layers[3] = glass_layer(4, clear_glass(live(10.0, 4)), (100, 100), (300.0, 100.0));
        rig.draw(&layers);
        assert_eq!(rig.report.backdrops, 2);
    }

    #[test]
    fn a_surface_over_another_sees_it_in_its_backdrop() {
        let Some(mut rig) = rig() else { return };
        // A black opaque glass surface on the white ground, and a clear one over its edge.
        let black = Glass {
            tint: Rgba(0, 0, 0, 255),
            ..clear_glass(Ask_::None)
        };
        let layers = vec![
            drawn(
                1,
                (960, 480),
                shape(Rgba(255, 255, 255, 255)),
                Props::at(0.0, 0.0),
            ),
            glass_layer(2, black, (200, 300), (100.0, 100.0)),
            glass_layer(3, clear_glass(live(12.0, 4)), (240, 100), (200.0, 150.0)),
        ];
        let shot = rig.draw(&layers);
        shot.save("backdrop-glass-over-glass");
        // The upper surface covers x 200..440 and the lower one's right edge is at 300: dark on its left, light on its
        // right, and in between the blurred edge.
        assert!(red(&shot, 210, 200) <= 10, "{}", red(&shot, 210, 200));
        assert!(red(&shot, 430, 200) >= 245, "{}", red(&shot, 430, 200));
        assert!(
            (red(&shot, 300, 200) - 128).abs() <= 20,
            "{}",
            red(&shot, 300, 200)
        );
    }

    /// The Lucent smart display's glass row: a strip and three cards, all blurring the picture below, with the drop
    /// shadows the material has.
    fn glass_row(backdrop: Ask_) -> Vec<crate::scene::Drawn> {
        let material = |radius: f32| Glass {
            corner_radius: radius,
            tint: Rgba(7, 6, 10, 102),
            shadows: [
                Some(compositor_protocol::Shadow {
                    offset_y: 24.0,
                    blur: 60.0,
                    colour: Rgba(0, 0, 0, 71),
                }),
                None,
            ],
            saturation: 1.25,
            ..clear_glass(backdrop)
        };
        let mut layers = vec![drawn(
            1,
            (960, 480),
            shape(Rgba(200, 150, 160, 255)),
            Props::at(0.0, 0.0),
        )];
        layers.push(glass_layer(2, material(27.0), (912, 54), (24.0, 20.0)));
        for card in 0..3u32 {
            layers.push(glass_layer(
                3 + card,
                material(24.0),
                (288, 150),
                (24.0 + 312.0 * card as f32, 96.0),
            ));
        }
        layers
    }

    #[test]
    fn every_surface_of_a_row_of_glass_shows_the_blurred_picture_below_it() {
        for (name, backdrop) in [
            ("live", live(30.0, 4)),
            ("cached", Ask_::Cached { radius: 30.0 }),
        ] {
            let Some(mut rig) = rig() else { return };
            let shot = rig.draw(&glass_row(backdrop));
            shot.save(&format!("backdrop-row-{name}"));
            // The ground is a flat (200, 150, 160), which the material's saturation of 1.25 makes a red of 208: under 40 %
            // of a near-black tint a surface is 60 % of that, less what the shadow of the surface above darkens it by.
            for (surface, x, y) in [
                ("strip", 480, 47),
                ("card 1", 168, 171),
                ("card 2", 480, 171),
                ("card 3", 792, 171),
            ] {
                let r = red(&shot, x, y);
                assert!(
                    (85..=140).contains(&r),
                    "{name} {surface}: {:?}",
                    shot.pixel(x, y)
                );
            }
        }
    }

    /// Cards of Lucent's glass side by side, each with the soft shadow the material has (24 px down, 60 px of blur),
    /// 24 px apart, so that every one's shadow falls on the next one's body. `blurs` gives each card's blur radius:
    /// cards of the same blur share a picture, and a hair's difference (which the chain cannot tell) does not.
    fn shadowed_cards(
        kept: fn(f32) -> Ask_,
        blurs: &[f32],
        shadows: bool,
    ) -> Vec<crate::scene::Drawn> {
        let material = |blur: f32| Glass {
            corner_radius: 24.0,
            tint: Rgba(7, 6, 10, 102),
            shadows: [
                shadows.then_some(compositor_protocol::Shadow {
                    offset_y: 24.0,
                    blur: 60.0,
                    colour: Rgba(0, 0, 0, 71),
                }),
                None,
            ],
            ..clear_glass(kept(blur))
        };
        let mut layers = vec![drawn(
            1,
            (960, 480),
            shape(Rgba(200, 150, 160, 255)),
            Props::at(0.0, 0.0),
        )];
        for (n, &blur) in blurs.iter().enumerate() {
            layers.push(glass_layer(
                2 + n as u32,
                material(blur),
                (200, 150),
                (24.0 + 224.0 * n as f32, 120.0),
            ));
        }
        layers
    }

    fn worst_difference(a: &Shot, b: &Shot) -> (u8, u32, u32) {
        let mut worst = (0, 0, 0);
        for y in 0..480 {
            for x in 0..960 {
                let (p, q) = (a.pixel(x, y), b.pixel(x, y));
                let d = (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap();
                if d > worst.0 {
                    worst = (d, x, y);
                }
            }
        }
        worst
    }

    fn kept_live(radius: f32) -> Ask_ {
        live(radius, 4)
    }

    fn kept_cached(radius: f32) -> Ask_ {
        Ask_::Cached { radius }
    }

    #[test]
    fn a_row_of_shadowed_glass_shares_one_picture_and_looks_as_if_each_had_its_own() {
        for (name, kept) in [
            ("live", kept_live as fn(f32) -> Ask_),
            ("cached", kept_cached),
        ] {
            let Some(mut rig) = rig() else { return };
            // The same four cards, the same blur; and again with a hair's difference between their blurs, which no eye
            // or chain can tell but which keeps them from sharing: each makes its own picture of what is below it.
            let shared = rig.draw(&shadowed_cards(kept, &[30.0; 4], true));
            assert_eq!(rig.report.backdrops, 1, "{name}: one picture for the row");
            shared.save(&format!("backdrop-row-shadows-{name}-shared"));
            let apart = rig.draw(&shadowed_cards(kept, &[30.0, 30.01, 30.02, 30.03], true));
            // A picture each; the first card's cached one is the one it had, of the same layers with the same blur.
            let made = if name == "cached" { 3 } else { 4 };
            assert_eq!(rig.report.backdrops, made, "{name}: a picture each");
            apart.save(&format!("backdrop-row-shadows-{name}-apart"));
            // They differ where a neighbour's shadow (or, for a cached picture, nothing else) reaches a body, and never
            // by much. A card's own picture has the shadows of the cards below it in it and not of the ones above; the
            // shared one has every neighbour's, so a card sees a later neighbour's shadow through its glass, softened
            // by the tint, where it was drawn over the glass at full strength. The shadow there is a quarter of a
            // level in a hundred at most and the tint 40 %: a few levels, and one more where the glass's noise rounds the
            // two the other way.
            let (worst, x, y) = worst_difference(&shared, &apart);
            assert!(worst <= 15, "{name}: {worst} at ({x}, {y})");
        }
    }

    #[test]
    fn what_a_row_of_glass_casts_together_is_seen_through_the_neighbours_of_a_live_picture_and_never_under_its_own_body()
     {
        for (name, kept) in [
            ("live", kept_live as fn(f32) -> Ask_),
            ("cached", kept_cached),
        ] {
            let Some(mut rig) = rig() else { return };
            let with = rig.draw(&shadowed_cards(kept, &[30.0; 4], true));
            // The cards are restyled without their shadows: their versions move, which is what makes a cached picture
            // that depends on them be taken again (it does not: see below).
            let mut bare = shadowed_cards(kept, &[30.0; 4], false);
            for card in &mut bare {
                card.version = 2;
            }
            let without = rig.draw(&bare);
            let red = |shot: &Shot, x, y| i32::from(shot.pixel(x, y)[0]);
            // The middle of a card's body is 100 px from the shadows of its neighbours, which reach 60: it is as it would
            // be without any shadow, so a card's own shadow is not under it (the mesh leaves the body out).
            for card in 0..4 {
                let x = 24 + 224 * card + 100;
                assert_eq!(
                    red(&with, x, 195),
                    red(&without, x, 195),
                    "{name} card {card}"
                );
            }
            // Near the edges of the second card, where its neighbours' shadows fall: a live picture has them in it, and
            // the glass shows them, darker, through its blur (the first card's, which the second's own picture always
            // had, and the third's, which it did not); a cached one has none, so the body is as it would be without.
            for (x, side) in [(248 + 12, "left"), (248 + 200 - 12, "right")] {
                let (with, without) = (red(&with, x, 195), red(&without, x, 195));
                if name == "live" {
                    assert!(
                        with + 2 <= without,
                        "{name}: the second card's {side} edge: {with} with the neighbours' shadows, {without} without"
                    );
                } else {
                    assert_eq!(with, without, "{name}: the second card's {side} edge");
                }
            }
            // Between and below the cards the shadows are what they were: drawn once, where the bodies are not.
            assert!(red(&with, 236, 195) < red(&without, 236, 195), "{name}");
            assert!(red(&with, 120, 290) < red(&without, 120, 290), "{name}");
        }
    }

    #[test]
    fn a_cached_picture_shared_by_a_row_is_not_taken_again_when_one_of_the_cards_moves_or_is_restyled()
     {
        let Some(mut rig) = rig() else { return };
        let mut row = shadowed_cards(kept_cached, &[30.0; 4], true);
        rig.draw(&row);
        assert_eq!(rig.report.backdrops, 1);
        // The third card is set somewhere else, as a finger dragging it would, and the second one loses its shadow.
        row[3].props.x += 10.0;
        row[3].rest.x += 10.0;
        row[3].version = 5;
        if let Content::Glass(glass) = &mut row[2].content {
            glass.shadows = [None, None];
        }
        row[2].version = 6;
        let moved = rig.draw(&row);
        assert_eq!(rig.report.backdrops, 0, "nothing below them changed");
        moved.save("backdrop-row-cached-moved");
        // They are drawn as they are now, all the same: the third card is where it was put.
        assert!(red(&moved, 24 + 224 * 2 + 10 + 100, 195) < 150);
        assert!(red(&moved, 24 + 224 * 2 + 100, 195) < 150);
        // A change of what is below the row takes it again, once.
        row[0].version = 2;
        rig.draw(&row);
        assert_eq!(rig.report.backdrops, 1);
        rig.draw(&row);
        assert_eq!(rig.report.backdrops, 0);
    }

    #[test]
    fn a_picture_made_between_two_surfaces_that_share_one_has_the_first_ones_shadow_and_not_the_second_ones()
     {
        // Two cards share a picture. A surface with another blur sits between them, touching the second card's shadow
        // but not its body. What is below that surface is the ground and the first card: the second card is drawn
        // above the surface, and its shadow is not in the surface's picture.
        let Some(mut rig) = rig() else { return };
        let card = |shadowed: bool| Glass {
            shadows: [
                shadowed.then_some(compositor_protocol::Shadow {
                    offset_y: 24.0,
                    blur: 60.0,
                    colour: Rgba(0, 0, 0, 71),
                }),
                None,
            ],
            ..clear_glass(live(30.0, 4))
        };
        let scene = |shadowed: bool| {
            vec![
                drawn(
                    1,
                    (960, 480),
                    shape(Rgba(200, 150, 160, 255)),
                    Props::at(0.0, 0.0),
                ),
                glass_layer(2, card(shadowed), (200, 150), (24.0, 120.0)),
                glass_layer(3, clear_glass(live(10.0, 4)), (100, 40), (300.0, 300.0)),
                glass_layer(4, card(shadowed), (200, 150), (272.0, 120.0)),
            ]
        };
        let with = rig.draw(&scene(true));
        assert_eq!(
            rig.report.backdrops, 2,
            "the cards share one picture and the surface between them has its own"
        );
        let without = rig.draw(&scene(false));
        // The middle of the surface between them is 50 px below the second card's body, well inside its shadow.
        let (a, b) = (red(&with, 350, 320), red(&without, 350, 320));
        assert!((a - b).abs() <= 2, "{a} with the shadows, {b} without");
    }

    #[test]
    fn saturation_is_applied_to_what_the_backdrop_shows() {
        let Some(mut rig) = rig() else { return };
        let grey = Glass {
            saturation: 0.0,
            ..clear_glass(live(8.0, 4))
        };
        let layers = vec![
            drawn(
                1,
                (960, 480),
                shape(Rgba(255, 0, 0, 255)),
                Props::at(0.0, 0.0),
            ),
            glass_layer(2, grey, (200, 100), (100.0, 100.0)),
        ];
        let shot = rig.draw(&layers);
        // Red drained of colour is its luma, 0.299 of full.
        let p = shot.pixel(200, 150);
        assert!(
            p[0].abs_diff(76) <= 3 && p[1].abs_diff(76) <= 3 && p[2].abs_diff(76) <= 3,
            "{p:?}"
        );
        assert_eq!(shot.pixel(50, 150)[..3], [255, 0, 0]);
    }

    #[test]
    fn a_bend_at_the_edge_shows_what_is_further_inside() {
        let Some(mut rig) = rig() else { return };
        // White until x = 340, black after; the glass's left edge is at 330.
        let ground = vec![
            drawn(
                1,
                (340, 480),
                shape(Rgba(255, 255, 255, 255)),
                Props::at(0.0, 0.0),
            ),
            drawn(
                2,
                (620, 480),
                shape(Rgba(0, 0, 0, 255)),
                Props::at(340.0, 0.0),
            ),
        ];
        let draw = |rig: &mut crate::testgl::Rig, refraction: f32| {
            let glass = Glass {
                refraction,
                ..clear_glass(live(0.0, 2))
            };
            let mut layers = ground.clone();
            layers.push(glass_layer(3, glass, (300, 150), (330.0, 150.0)));
            rig.draw(&layers)
        };
        let flat = draw(&mut rig, 0.0);
        let bent = draw(&mut rig, 10.0);
        shot_pair(&flat, &bent);
        // Three pixels inside the edge, still left of the picture's own edge: flat glass shows white, and the bent
        // glass has moved the picture towards the inside, so the black edge has moved out to it.
        assert!(red(&flat, 333, 225) >= 240, "{}", red(&flat, 333, 225));
        assert!(red(&bent, 333, 225) <= 60, "{}", red(&bent, 333, 225));
        // Deep inside there is no bend.
        assert_eq!(red(&flat, 500, 225), red(&bent, 500, 225));
    }

    fn shot_pair(a: &Shot, b: &Shot) {
        a.save("backdrop-flat");
        b.save("backdrop-bent");
    }

    #[test]
    fn a_wide_picture_is_made_from_fewer_texels() {
        // Cached backdrops choose their own coarseness by how wide the blur is.
        use crate::renderer::cached_downsample;
        assert_eq!(cached_downsample(4.0), 2);
        assert_eq!(cached_downsample(30.0), 4);
        assert_eq!(cached_downsample(64.0), 8);
    }

    #[test]
    fn the_picture_is_left_at_the_coarsest_level_that_is_still_smooth_and_the_blur_is_as_wide_as_asked()
     {
        for downsample in [2u8, 4, 8] {
            for radius in [12.0f32, 30.0, 64.0] {
                let plan = BlurPlan::for_radius(radius, downsample);
                if plan.levels == 0 {
                    continue;
                }
                let texel = |level: u32| f32::from(downsample) * (1u32 << level) as f32;
                let fine = RESULT_TEXEL * radius;
                assert!(plan.result <= plan.levels);
                // At the level it is left at the texels are fine enough (a downsample that is too coarse to begin with
                // leaves the finest level: there is nothing finer)...
                assert!(plan.result == 0 || texel(plan.result) <= fine, "{plan:?}");
                // ...and one level coarser would be too coarse to read bilinearly, or is past the chain.
                assert!(
                    plan.result == plan.levels || texel(plan.result + 1) > fine,
                    "{plan:?}"
                );
                // Leaving the passes out is not why the blur would be the wrong width.
                assert!(
                    (plan.sigma - radius).abs() <= 0.03 * radius + 0.3,
                    "d={downsample} r={radius}: {plan:?}"
                );
                assert!(plan.reach(downsample) as f32 >= 3.2 * radius);
            }
        }
        // A Lucent blur (30) at a quarter of the screen is left at 8 pixel texels, one halving down.
        assert_eq!(BlurPlan::for_radius(30.0, 4).result, 1);
    }
}
