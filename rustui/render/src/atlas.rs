//! Glyph atlases and the glyph runs drawn from them (design §3.1 item 4; `compositor_protocol::resources` is the contract).
//!
//! **An atlas** is a one-byte-per-pixel coverage texture. The app fills it in chunks: each is an `Op::AtlasUpload`, named by
//! a generation that only rises, whose bytes the app has written into a staging file. The ops only *queue* the chunks. They
//! are read and copied to the texture one at a time, in the order they came, in a moment the compositor has to spare
//! ([`Atlases::land_next`], called from `Renderer::tidy`), each answered with `Event::AtlasUploaded`; the app may write its
//! staging slot again only then. The texture itself is made when the first chunk lands, so that creating an atlas costs a frame
//! nothing. A chunk whose file is missing, short or in the way is logged and answered all the same, and uploads nothing: a
//! staging slot is not kept busy by a file that will never be read, and nothing waits for it for ever.
//!
//! **A run** is the list of quads a glyph layer draws (`Content::Glyphs`): a rectangle of the atlas, where it goes in the
//! layer, and which of the four colours of the layer's palette it is drawn in (its ink; the quads come sorted by ink, so
//! [`Atlases::place`] gives one span of vertices per ink). `SetGlyphs` swaps the whole list, and the swap waits until every chunk of the layer's atlas up to the generation it
//! names (`after`) has reached the texture, so that a label appears whole; until then the old list stays, and a newer
//! `SetGlyphs` replaces one that is still waiting. A reveal that is waiting for a chunk that is queued makes the frame that is
//! about to be drawn land it first ([`Atlases::land_gated`], at most one chunk's worth of bytes), so a label committed with
//! its glyphs is in the frame of its commit rather than the one after.
//!
//! **Revisions.** The scene does not know when a list is swapped (nor when a chunk lands under the quads of a list that
//! shows it), so each run carries a revision of its own that changes exactly then. The renderer folds it into the layer's
//! version wherever versions decide what is drawn again: the fingerprint of a cached backdrop, and what a damaged frame
//! repaints.

use crate::geometry::{Panel, PixRect, Placement, Vertex};
use crate::gl::Gles;
use crate::log::log;
use crate::shm;
use compositor_protocol::resources::{INKS, MAX_UPLOAD_BYTES, STAGING_SLOTS};
use compositor_protocol::{AtlasId, AtlasRect, Event, Glyph, LayerId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The most bytes of chunks that a frame lands for a reveal that waits for them: the design's bound on the pixels a presented
/// frame uploads (§11.1), so the critical path is never more than one chunk of the largest size.
pub const FRAME_UPLOAD_BYTES: usize = MAX_UPLOAD_BYTES as usize;

/// One chunk waiting to be copied to its atlas.
struct Upload {
    /// Which chunk came first, across atlases.
    arrival: u64,
    slot: u8,
    generation: u64,
    rect: AtlasRect,
    path: PathBuf,
}

struct Atlas {
    width: u32,
    height: u32,
    /// Made when the first chunk lands.
    texture: Option<u32>,
    queue: VecDeque<Upload>,
    /// The highest generation answered: its chunk is in the texture, or could not be read and was given up on.
    landed: u64,
    /// The highest generation of a chunk that was turned away because the queue was full (see [`Atlases::queue`]).
    refused: u64,
}

impl Atlas {
    /// The generation every chunk up to which has been dealt with, for a list that waits on it: what landed, and, once the
    /// queue has drained, what was turned away (a chunk that never lands must not keep a list waiting for ever).
    fn reached(&self) -> u64 {
        if self.queue.is_empty() {
            self.landed.max(self.refused)
        } else {
            self.landed
        }
    }
}

/// Lets a line through at most once in [`Quiet::EVERY`]: the compositor logs from the render thread, onto a stderr that is
/// not allowed to block, and a client that sends a chunk after chunk that cannot be used must not be able to turn that
/// into a flood of lines (a full pipe aborts the process).
#[derive(Default)]
struct Quiet {
    last: Option<Instant>,
    held_back: u32,
}

impl Quiet {
    const EVERY: Duration = Duration::from_secs(1);

    /// Whether a line may be written at `now`; when it may, how many were held back since the last one.
    fn allow(&mut self, now: Instant) -> Option<u32> {
        if self
            .last
            .is_some_and(|last| now.duration_since(last) < Self::EVERY)
        {
            self.held_back = self.held_back.saturating_add(1);
            return None;
        }
        self.last = Some(now);
        Some(std::mem::take(&mut self.held_back))
    }

    /// Writes `message` if a line may be written now, and how many were held back before it.
    fn say(&mut self, message: impl FnOnce() -> String) {
        if let Some(held_back) = self.allow(Instant::now()) {
            let message = message();
            if held_back == 0 {
                log(message);
            } else {
                log(format!(
                    "{message} ({held_back} more like it since the last line)"
                ));
            }
        }
    }
}

/// A list of quads waiting for the chunks it needs.
struct Reveal {
    after: u64,
    glyphs: Vec<Glyph>,
}

/// What a glyph layer draws.
struct Run {
    atlas: AtlasId,
    /// The quads drawn now.
    glyphs: Vec<Glyph>,
    /// Changes whenever what the run draws can: its list is swapped, the layer is restyled (another atlas, or another palette:
    /// the scene does not tell the run which), or a chunk lands under one of its quads.
    revision: u64,
    waiting: Option<Reveal>,
}

/// Where the vertices of a run are in the frame's buffer, one span per ink: the quads come sorted by ink, so those of ink `k`
/// are `counts[k]` vertices that follow the ones of ink `k - 1`, the first ink's from `first`. `ink` is the panel pixels the
/// quads drawn cover together (`[left, top, right, bottom]`, whole pixels outwards): all of the panel the run draws on, which
/// is far less than its layer's rectangle for the sparse art of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InkSpans {
    pub first: i32,
    pub counts: [i32; INKS],
    pub ink: [i32; 4],
}

/// What landing chunks did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Landed {
    /// Chunks answered, and the bytes of them that were copied to a texture.
    pub chunks: u32,
    pub bytes: u64,
    /// Runs that draw something else now and are of layers the last frame drew (`Atlases::set_shown`): a list that waited for
    /// the chunk was swapped in, or the chunk landed under a quad of the list drawn. A run of a layer nothing drew (a page made
    /// ahead under a group that is not shown) has its list all the same, and changes nothing on the panel.
    pub changed: u32,
}

impl Landed {
    pub fn add(&mut self, other: Landed) {
        self.chunks += other.chunks;
        self.bytes += other.bytes;
        self.changed += other.changed;
    }
}

#[derive(Default)]
pub struct Atlases {
    atlases: HashMap<AtlasId, Atlas>,
    runs: HashMap<LayerId, Run>,
    /// How many chunks have been queued, to tell which came first.
    arrivals: u64,
    /// Where a chunk's bytes are read to before they are copied to the texture: reused, so it grows to the largest chunk.
    staging: Vec<u8>,
    quiet: Quiet,
    /// The layers the last frame drew, once the renderer has been told: only a change in a run of one of them is a change of
    /// what the panel shows. `None` until then: every run counts.
    shown: Option<HashSet<LayerId>>,
}

/// Mixes a renderer-owned `revision` into a layer's scene `version`: the same version for revision 0, another for every other,
/// and so another whenever either changes.
pub fn mixed(version: u64, revision: u64) -> u64 {
    version ^ revision.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

fn overlaps(glyph: &Glyph, rect: &AtlasRect) -> bool {
    let (gx, gy) = (u32::from(glyph.sx), u32::from(glyph.sy));
    let (rx, ry) = (u32::from(rect.x), u32::from(rect.y));
    gx < rx + u32::from(rect.w)
        && rx < gx + u32::from(glyph.w)
        && gy < ry + u32::from(rect.h)
        && ry < gy + u32::from(glyph.h)
}

/// Whether a change of the run of `layer` is a change of what the panel shows: the layer was drawn by the last frame, or nobody has
/// said which were (`shown` is `None`).
fn draws(shown: &Option<HashSet<LayerId>>, layer: LayerId) -> bool {
    shown.as_ref().is_none_or(|shown| shown.contains(&layer))
}

impl Atlases {
    /// Adds the atlases' textures to `census`.
    pub fn census(&self, census: &mut crate::census::Census) {
        for atlas in self.atlases.values() {
            if atlas.texture.is_some() {
                census.atlases.texture(atlas.width, atlas.height, 1);
            }
        }
    }

    /// Makes an atlas of `width` x `height` coverage bytes, all zero. One of that name that exists is replaced: its texture
    /// goes to `graveyard` and the chunks queued for it are dropped.
    pub fn create(&mut self, id: AtlasId, width: u32, height: u32, graveyard: &mut Vec<u32>) {
        self.delete(id, graveyard);
        self.atlases.insert(
            id,
            Atlas {
                width,
                height,
                texture: None,
                queue: VecDeque::new(),
                landed: 0,
                refused: 0,
            },
        );
    }

    /// Lets the atlas go: its texture to `graveyard`, its queue dropped. What was drawn from it is drawn no more.
    pub fn delete(&mut self, id: AtlasId, graveyard: &mut Vec<u32>) {
        if let Some(atlas) = self.atlases.remove(&id) {
            graveyard.extend(atlas.texture);
            self.bump(id, None);
        }
    }

    /// Queues a chunk. A chunk for an atlas that is not there (it was deleted since) is nobody's.
    ///
    /// An atlas has [`STAGING_SLOTS`] staging slots and the app writes a slot again only after `AtlasUploaded` for it, so an
    /// app that keeps to the protocol has at most that many chunks queued. One that sends another while the queue is that
    /// long is not waiting for the answers: its chunk is turned away, and answered at once so that nothing waits for it. The
    /// queue is what bounds the work a frame can be made to do (a `SetGlyphs` that waits lands every chunk it needs in
    /// the frame that shows it), and the memory a client can make the compositor hold.
    pub fn queue(
        &mut self,
        id: AtlasId,
        slot: u8,
        generation: u64,
        rect: AtlasRect,
        path: PathBuf,
        events: &mut Vec<Event>,
    ) {
        let Some(atlas) = self.atlases.get_mut(&id) else {
            return;
        };
        if atlas.queue.len() >= usize::from(STAGING_SLOTS) {
            atlas.refused = atlas.refused.max(generation);
            events.push(Event::AtlasUploaded {
                id,
                slot,
                generation,
            });
            self.quiet.say(|| {
                format!(
                    "atlas {}: a chunk (slot {slot}, generation {generation}) came while {STAGING_SLOTS} were waiting to be copied; \
                     turned away, the app is not waiting for AtlasUploaded before it writes a staging slot again",
                    id.0
                )
            });
            return;
        }
        self.arrivals += 1;
        atlas.queue.push_back(Upload {
            arrival: self.arrivals,
            slot,
            generation,
            rect,
            path,
        });
    }

    /// Everything goes: the client that owned it has gone.
    pub fn clear(&mut self, graveyard: &mut Vec<u32>) {
        for atlas in self.atlases.drain().map(|(_, atlas)| atlas) {
            graveyard.extend(atlas.texture);
        }
        self.runs.clear();
    }

    /// A glyph layer is made (or made again): it draws nothing until it is given quads.
    pub fn run_made(&mut self, layer: LayerId, atlas: AtlasId) {
        self.runs.insert(
            layer,
            Run {
                atlas,
                glyphs: Vec::new(),
                revision: 0,
                waiting: None,
            },
        );
    }

    /// A glyph layer is restyled: it may draw from another atlas or in other colours now, so the revision changes whichever it
    /// was, and what it waits for is the chunks of that atlas.
    pub fn run_restyled(&mut self, layer: LayerId, atlas: AtlasId) {
        if let Some(run) = self.runs.get_mut(&layer) {
            run.atlas = atlas;
            run.revision += 1;
        }
        self.reveal_ready();
    }

    pub fn run_deleted(&mut self, layer: LayerId) {
        self.runs.remove(&layer);
    }

    /// `SetGlyphs`: the new list replaces the one drawn once every chunk of the layer's atlas up to `after` has landed (at
    /// once for 0 or for chunks that have), and until then waits, replacing a list that was still waiting.
    pub fn set_glyphs(&mut self, layer: LayerId, after: u64, glyphs: Vec<Glyph>) {
        let Some(run) = self.runs.get_mut(&layer) else {
            return;
        };
        let landed = self.atlases.get(&run.atlas).map_or(0, Atlas::reached);
        if after <= landed {
            run.glyphs = glyphs;
            run.revision += 1;
            run.waiting = None;
        } else {
            run.waiting = Some(Reveal { after, glyphs });
        }
    }

    /// Tells the atlases which layers the last frame drew (see [`Landed::changed`]): the renderer does, after every frame.
    pub fn set_shown(&mut self, layers: impl Iterator<Item = LayerId>) {
        let shown = self.shown.get_or_insert_with(HashSet::new);
        shown.clear();
        shown.extend(layers);
    }

    /// Swaps in the lists whose chunks have landed. Returns how many are of layers the last frame drew.
    fn reveal_ready(&mut self) -> u32 {
        let mut revealed = 0;
        for (layer, run) in &mut self.runs {
            let landed = self.atlases.get(&run.atlas).map_or(0, Atlas::reached);
            if run.waiting.as_ref().is_some_and(|w| w.after <= landed) {
                if let Some(reveal) = run.waiting.take() {
                    run.glyphs = reveal.glyphs;
                    run.revision += 1;
                    revealed += u32::from(draws(&self.shown, *layer));
                }
            }
        }
        revealed
    }

    /// The runs of atlas `id` that draw from `rect` of it (all of them for `None`) look different now. Returns how many are of
    /// layers the last frame drew.
    fn bump(&mut self, id: AtlasId, rect: Option<&AtlasRect>) -> u32 {
        let mut bumped = 0;
        for (layer, run) in self.runs.iter_mut().filter(|(_, r)| r.atlas == id) {
            if rect.is_none_or(|rect| run.glyphs.iter().any(|g| overlaps(g, rect))) {
                run.revision += 1;
                bumped += u32::from(draws(&self.shown, *layer));
            }
        }
        bumped
    }

    /// Whether a chunk is queued.
    pub fn pending(&self) -> bool {
        self.atlases.values().any(|a| !a.queue.is_empty())
    }

    /// Lands the chunk that came first, if any: from `Renderer::tidy`, one a call.
    pub fn land_next(&mut self, gl: &Gles, events: &mut Vec<Event>) -> Landed {
        let next = self
            .atlases
            .iter()
            .filter_map(|(id, a)| a.queue.front().map(|u| (u.arrival, *id)))
            .min();
        match next {
            Some((_, id)) => self.land(gl, id, events),
            None => Landed::default(),
        }
    }

    /// The generation the glyph lists waiting on atlas `id` need to have landed, or 0 when none waits.
    fn needed(&self, id: AtlasId) -> u64 {
        self.runs
            .values()
            .filter(|r| r.atlas == id)
            .filter_map(|r| r.waiting.as_ref().map(|w| w.after))
            .max()
            .unwrap_or(0)
    }

    /// Lands the chunks that glyph lists waiting to be shown are waiting for, in the order they came and while the bytes
    /// added up are within `budget` (the first chunk is landed whatever its size): the frame about to be drawn shows the lists
    /// whose chunks these were. Nothing is done when no list waits.
    pub fn land_gated(&mut self, gl: &Gles, events: &mut Vec<Event>, budget: usize) -> Landed {
        let mut total = Landed::default();
        let mut spent = 0usize;
        if self.runs.values().all(|r| r.waiting.is_none()) {
            return total;
        }
        loop {
            let next = self
                .atlases
                .iter()
                .filter_map(|(id, a)| {
                    let upload = a.queue.front()?;
                    (upload.generation <= self.needed(*id)).then_some((
                        upload.arrival,
                        *id,
                        upload.rect.bytes() as usize,
                    ))
                })
                .min();
            let Some((_, id, bytes)) = next else {
                break;
            };
            if total.chunks > 0 && spent + bytes > budget {
                break;
            }
            spent += bytes;
            total.add(self.land(gl, id, events));
        }
        total
    }

    /// Copies the first queued chunk of atlas `id` to its texture, answers it, and shows the lists that were waiting for it.
    fn land(&mut self, gl: &Gles, id: AtlasId, events: &mut Vec<Event>) -> Landed {
        let Some(atlas) = self.atlases.get_mut(&id) else {
            return Landed::default();
        };
        let Some(upload) = atlas.queue.pop_front() else {
            return Landed::default();
        };
        let rect = upload.rect;
        let bytes = rect.bytes() as usize;
        let inside = rect.w > 0
            && rect.h > 0
            && u32::from(rect.x) + u32::from(rect.w) <= atlas.width
            && u32::from(rect.y) + u32::from(rect.h) <= atlas.height;
        let copied = if !inside {
            self.quiet.say(|| {
                format!(
                    "atlas {}: a chunk of {}x{} at ({}, {}) is not inside the {}x{} atlas; nothing uploaded",
                    id.0, rect.w, rect.h, rect.x, rect.y, atlas.width, atlas.height
                )
            });
            0
        } else {
            match shm::read_staged(&upload.path, bytes, &mut self.staging) {
                Ok(()) => {
                    let (width, height) = (atlas.width, atlas.height);
                    let texture = *atlas
                        .texture
                        .get_or_insert_with(|| gl.new_alpha_texture(width, height));
                    gl.bind_texture(texture);
                    gl.upload_alpha(
                        u32::from(rect.x),
                        u32::from(rect.y),
                        u32::from(rect.w),
                        u32::from(rect.h),
                        &self.staging,
                    );
                    bytes as u64
                }
                Err(why) => {
                    self.quiet.say(|| {
                        format!(
                            "atlas {} slot {} generation {}: {why}; nothing uploaded",
                            id.0, upload.slot, upload.generation
                        )
                    });
                    0
                }
            }
        };
        atlas.landed = atlas.landed.max(upload.generation);
        events.push(Event::AtlasUploaded {
            id,
            slot: upload.slot,
            generation: upload.generation,
        });
        let bumped = if copied > 0 {
            self.bump(id, Some(&rect))
        } else {
            0
        };
        Landed {
            chunks: 1,
            bytes: copied,
            changed: bumped + self.reveal_ready(),
        }
    }

    /// The revision of the run of `layer`: 0 for a layer that is not a glyph run.
    pub fn revision(&self, layer: LayerId) -> u64 {
        self.runs.get(&layer).map_or(0, |run| run.revision)
    }

    /// The texture of atlas `id` and its size in texels, once a chunk has landed in it.
    pub fn texture(&self, id: AtlasId) -> Option<(u32, f32, f32)> {
        let atlas = self.atlases.get(&id)?;
        Some((atlas.texture?, atlas.width as f32, atlas.height as f32))
    }

    /// Adds the vertices of the quads of `layer`'s run to `vertices`, as the layer is placed by `at`, and says where each
    /// ink's are ([`InkSpans`]; a quad that is cut away or left out does not count): two triangles for every quad, corners at the panel position the quad's place in the layer comes to and the place in the atlas, in texels, as the
    /// other attribute. A quad is cut to the layer's rectangle (the size bounds what the layer draws, and everything the frame
    /// plans from a layer's extent relies on it), and left out when it is wholly outside `within`, the part of the panel
    /// that is drawn. The quads' places are whole pixels when the layer is at scale 1 on whole pixels, and then every texel is
    /// read at its centre. `None` when nothing is drawn: the run has no quad in sight, or its atlas no texture yet.
    pub fn place(
        &self,
        layer: LayerId,
        at: &Placement,
        panel: &Panel,
        within: PixRect,
        vertices: &mut Vec<Vertex>,
    ) -> Option<InkSpans> {
        let run = self.runs.get(&layer)?;
        self.atlases.get(&run.atlas)?.texture?;
        let first = vertices.len();
        let sight = [
            within.x as f32,
            within.y as f32,
            (within.x + within.w) as f32,
            (within.y + within.h) as f32,
        ];
        let mut counts = [0; INKS];
        let (mut ink_lo, mut ink_hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for glyph in &run.glyphs {
            let (x0, y0) = (glyph.x, glyph.y);
            let (x1, y1) = (x0 + f32::from(glyph.w), y0 + f32::from(glyph.h));
            let (left, top) = (x0.max(0.0), y0.max(0.0));
            let (right, bottom) = (x1.min(at.width), y1.min(at.height));
            if right <= left || bottom <= top {
                continue;
            }
            let mut quad = panel.quad_of(at, [left, top, right, bottom]);
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for v in &quad {
                lo = [lo[0].min(v[0]), lo[1].min(v[1])];
                hi = [hi[0].max(v[0]), hi[1].max(v[1])];
            }
            if hi[0] <= sight[0] || lo[0] >= sight[2] || hi[1] <= sight[1] || lo[1] >= sight[3] {
                continue;
            }
            ink_lo = [ink_lo[0].min(lo[0]), ink_lo[1].min(lo[1])];
            ink_hi = [ink_hi[0].max(hi[0]), ink_hi[1].max(hi[1])];
            // The place in the layer becomes the place in the atlas, texel for pixel.
            for v in &mut quad {
                v[2] = f32::from(glyph.sx) + (v[2] - x0);
                v[3] = f32::from(glyph.sy) + (v[3] - y0);
            }
            vertices.extend_from_slice(&[quad[0], quad[1], quad[2], quad[1], quad[3], quad[2]]);
            counts[usize::from(glyph.ink)] += 6;
        }
        (vertices.len() > first).then_some(InkSpans {
            first: first as i32,
            counts,
            ink: [
                ink_lo[0].floor() as i32,
                ink_lo[1].floor() as i32,
                ink_hi[0].ceil() as i32,
                ink_hi[1].ceil() as i32,
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use device_profile::Rotation;

    fn panel(rotation: Rotation) -> Panel {
        Panel {
            rotation,
            logical_width: 960.0,
            logical_height: 480.0,
            width: if rotation.swaps_axes() { 480 } else { 960 },
            height: if rotation.swaps_axes() { 960 } else { 480 },
        }
    }

    fn glyph(x: f32, y: f32, sx: u16, sy: u16, w: u16, h: u16) -> Glyph {
        Glyph {
            x,
            y,
            sx,
            sy,
            w,
            h,
            ink: 0,
        }
    }

    fn inked(ink: u8, glyph: Glyph) -> Glyph {
        Glyph { ink, ..glyph }
    }

    /// An atlas that has a texture, without a GL: only the quads' geometry is looked at.
    fn atlases_with(glyphs: Vec<Glyph>) -> Atlases {
        let mut atlases = Atlases::default();
        atlases.create(AtlasId(1), 64, 64, &mut Vec::new());
        atlases.atlases.get_mut(&AtlasId(1)).unwrap().texture = Some(7);
        atlases.run_made(LayerId(5), AtlasId(1));
        atlases.set_glyphs(LayerId(5), 0, glyphs);
        atlases
    }

    fn at(x: f32, y: f32, scale: f32) -> Placement {
        Placement {
            x,
            y,
            width: 100.0,
            height: 40.0,
            scale,
        }
    }

    fn whole(panel: &Panel) -> PixRect {
        PixRect::full(panel.width, panel.height)
    }

    #[test]
    fn a_quad_is_two_triangles_whose_corners_carry_the_places_in_the_atlas_texel_for_pixel() {
        let p = panel(Rotation::R0);
        let atlases = atlases_with(vec![glyph(10.0, 5.0, 3, 4, 8, 6)]);
        let mut vertices = Vec::new();
        let span = atlases
            .place(
                LayerId(5),
                &at(200.0, 100.0, 1.0),
                &p,
                whole(&p),
                &mut vertices,
            )
            .unwrap();
        assert_eq!(
            span,
            InkSpans {
                first: 0,
                counts: [6, 0, 0, 0],
                ink: [210, 105, 218, 111],
            }
        );
        // Top-left, top-right and bottom-left, then top-right, bottom-right and bottom-left: panel place, atlas place.
        assert_eq!(vertices[0], [210.0, 105.0, 3.0, 4.0]);
        assert_eq!(vertices[1], [218.0, 105.0, 11.0, 4.0]);
        assert_eq!(vertices[2], [210.0, 111.0, 3.0, 10.0]);
        assert_eq!(vertices[3], vertices[1]);
        assert_eq!(vertices[4], [218.0, 111.0, 11.0, 10.0]);
        assert_eq!(vertices[5], vertices[2]);
    }

    #[test]
    fn a_quad_is_cut_to_the_layer_and_what_is_cut_off_is_cut_from_the_atlas_rectangle_too() {
        let p = panel(Rotation::R0);
        // Hangs 4 pixels over the layer's left edge and 2 over the top; the layer is 100 x 40.
        let atlases = atlases_with(vec![
            glyph(-4.0, -2.0, 20, 30, 10, 8),
            glyph(95.0, 30.0, 0, 0, 10, 20),
            glyph(120.0, 5.0, 0, 0, 4, 4),
            glyph(-9.0, 5.0, 0, 0, 4, 4),
        ]);
        let mut vertices = Vec::new();
        atlases
            .place(LayerId(5), &at(0.0, 0.0, 1.0), &p, whole(&p), &mut vertices)
            .unwrap();
        // The first keeps 6 x 6 pixels of its 10 x 8, from atlas (24, 32) on; the second 5 x 10 of 10 x 20; the last two none.
        assert_eq!(vertices.len(), 12);
        assert_eq!(vertices[0], [0.0, 0.0, 24.0, 32.0]);
        assert_eq!(vertices[4], [6.0, 6.0, 30.0, 38.0]);
        assert_eq!(vertices[6], [95.0, 30.0, 0.0, 0.0]);
        assert_eq!(vertices[10], [100.0, 40.0, 5.0, 10.0]);
    }

    #[test]
    fn the_layers_scale_is_about_its_centre_and_the_atlas_places_do_not_scale() {
        let p = panel(Rotation::R0);
        let atlases = atlases_with(vec![glyph(0.0, 0.0, 8, 8, 10, 10)]);
        let mut vertices = Vec::new();
        atlases
            .place(
                LayerId(5),
                &at(100.0, 100.0, 2.0),
                &p,
                whole(&p),
                &mut vertices,
            )
            .unwrap();
        // The 100 x 40 layer doubled about (150, 120) has its corner at (50, 80), and its pixels twice as large.
        assert_eq!(vertices[0], [50.0, 80.0, 8.0, 8.0]);
        assert_eq!(vertices[4], [70.0, 100.0, 18.0, 18.0]);
    }

    #[test]
    fn the_places_are_turned_with_the_panel_and_the_atlas_is_never_mirrored() {
        let atlases = atlases_with(vec![glyph(10.0, 5.0, 3, 4, 8, 6)]);
        let p = panel(Rotation::R270);
        let mut vertices = Vec::new();
        atlases
            .place(
                LayerId(5),
                &at(200.0, 100.0, 1.0),
                &p,
                whole(&p),
                &mut vertices,
            )
            .unwrap();
        // The screen's top-left is the panel's top-right, and its x runs down the panel: logical (210, 105) is panel (375, 210).
        assert_eq!(vertices[0], [375.0, 210.0, 3.0, 4.0]);
        assert_eq!(vertices[4], [369.0, 218.0, 11.0, 10.0]);
    }

    #[test]
    fn quads_wholly_out_of_sight_are_not_drawn_and_a_run_with_none_in_sight_is_not_drawn_at_all() {
        let p = panel(Rotation::R0);
        let atlases = atlases_with(vec![
            glyph(0.0, 0.0, 0, 0, 10, 10),
            glyph(50.0, 0.0, 0, 0, 10, 10),
        ]);
        let mut vertices = Vec::new();
        // Only the part of the panel from x = 235 is drawn: the second quad (at x = 250) is in it, the first is not.
        let sight = PixRect {
            x: 235,
            y: 0,
            w: 725,
            h: 480,
        };
        let span = atlases
            .place(LayerId(5), &at(200.0, 100.0, 1.0), &p, sight, &mut vertices)
            .unwrap();
        assert_eq!(span.counts, [6, 0, 0, 0]);
        assert_eq!(vertices[0][..2], [250.0, 100.0]);
        let none = PixRect {
            x: 0,
            y: 300,
            w: 960,
            h: 100,
        };
        vertices.clear();
        assert!(
            atlases
                .place(LayerId(5), &at(200.0, 100.0, 1.0), &p, none, &mut vertices)
                .is_none()
        );
        assert!(vertices.is_empty());
    }

    #[test]
    fn the_vertices_of_each_ink_are_a_span_that_follows_the_one_before_even_when_quads_are_cut_or_left_out()
     {
        let p = panel(Rotation::R0);
        // Two quads of ink 0 (the second is wholly outside the 100 x 40 layer and so not drawn), one of ink 1 that is cut to
        // the layer's edge, one of ink 2 and none of ink 3. A first quad of ink 0 is drawn.
        let atlases = atlases_with(vec![
            glyph(0.0, 0.0, 1, 1, 4, 4),
            glyph(150.0, 0.0, 2, 2, 4, 4),
            inked(1, glyph(96.0, 10.0, 10, 10, 8, 4)),
            inked(2, glyph(20.0, 20.0, 30, 30, 4, 4)),
            inked(2, glyph(30.0, 20.0, 40, 40, 4, 4)),
        ]);
        let mut vertices = vec![[9.0; 4]; 3];
        let spans = atlases
            .place(LayerId(5), &at(0.0, 0.0, 1.0), &p, whole(&p), &mut vertices)
            .unwrap();
        // The three vertices that were there are not the run's: the spans start after them.
        assert_eq!(
            spans,
            InkSpans {
                first: 3,
                counts: [6, 6, 12, 0],
                // The quads that are drawn, the cut one to the layer's edge: the one that is wholly outside is not in it.
                ink: [0, 0, 100, 24],
            }
        );
        assert_eq!(vertices.len(), 3 + 24);
        let atlas_corner = |vertex: usize| (vertices[vertex][2], vertices[vertex][3]);
        // Ink 0: the first quad. Ink 1: the second, cut to 4 of its 8 pixels. Ink 2: the last two, in the order they came.
        assert_eq!(atlas_corner(3), (1.0, 1.0));
        assert_eq!(atlas_corner(9), (10.0, 10.0));
        assert_eq!(vertices[9 + 4], [100.0, 14.0, 14.0, 14.0]);
        assert_eq!(atlas_corner(15), (30.0, 30.0));
        assert_eq!(atlas_corner(21), (40.0, 40.0));
    }

    #[test]
    fn a_run_whose_every_quad_is_cut_away_has_no_spans_at_all() {
        let p = panel(Rotation::R0);
        let atlases = atlases_with(vec![
            inked(1, glyph(120.0, 0.0, 0, 0, 4, 4)),
            inked(3, glyph(-9.0, 0.0, 0, 0, 4, 4)),
        ]);
        let mut vertices = Vec::new();
        assert!(
            atlases
                .place(LayerId(5), &at(0.0, 0.0, 1.0), &p, whole(&p), &mut vertices)
                .is_none()
        );
        assert!(vertices.is_empty());
    }

    #[test]
    fn a_run_whose_atlas_has_no_texture_yet_or_that_is_not_a_run_draws_nothing() {
        let p = panel(Rotation::R0);
        let mut atlases = atlases_with(vec![glyph(0.0, 0.0, 0, 0, 10, 10)]);
        let mut vertices = Vec::new();
        assert!(
            atlases
                .place(LayerId(9), &at(0.0, 0.0, 1.0), &p, whole(&p), &mut vertices)
                .is_none()
        );
        atlases.atlases.get_mut(&AtlasId(1)).unwrap().texture = None;
        assert!(
            atlases
                .place(LayerId(5), &at(0.0, 0.0, 1.0), &p, whole(&p), &mut vertices)
                .is_none()
        );
    }

    #[test]
    fn a_list_is_swapped_at_once_when_its_chunks_have_landed_and_the_newest_waiting_list_wins() {
        let mut atlases = atlases_with(vec![glyph(0.0, 0.0, 0, 0, 10, 10)]);
        let one = vec![glyph(1.0, 1.0, 0, 0, 2, 2)];
        let two = vec![glyph(2.0, 2.0, 0, 0, 2, 2)];
        let three = vec![glyph(3.0, 3.0, 0, 0, 2, 2)];
        let revision = atlases.revision(LayerId(5));
        // Generation 4 has not landed: the list waits, and the one drawn stays.
        atlases.set_glyphs(LayerId(5), 4, one);
        assert_eq!(atlases.revision(LayerId(5)), revision);
        assert_eq!(atlases.runs[&LayerId(5)].glyphs[0].x, 0.0);
        // A newer list for the same generation replaces it; so does one for a later generation.
        atlases.set_glyphs(LayerId(5), 4, two);
        atlases.set_glyphs(LayerId(5), 5, three);
        assert_eq!(atlases.runs[&LayerId(5)].waiting.as_ref().unwrap().after, 5);
        // Nothing needs to land for 0: it is shown at once, and takes the waiting list's place.
        atlases.set_glyphs(LayerId(5), 0, vec![glyph(9.0, 9.0, 0, 0, 2, 2)]);
        assert_eq!(atlases.runs[&LayerId(5)].glyphs[0].x, 9.0);
        assert!(atlases.runs[&LayerId(5)].waiting.is_none());
        assert_eq!(atlases.revision(LayerId(5)), revision + 1);
    }

    #[test]
    fn a_stale_op_for_an_atlas_or_layer_that_is_not_there_is_nobodys() {
        let mut atlases = Atlases::default();
        let mut graveyard = Vec::new();
        atlases.queue(
            AtlasId(3),
            0,
            1,
            AtlasRect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            PathBuf::new(),
            &mut Vec::new(),
        );
        assert!(!atlases.pending());
        atlases.set_glyphs(LayerId(1), 0, vec![glyph(0.0, 0.0, 0, 0, 1, 1)]);
        atlases.run_restyled(LayerId(1), AtlasId(3));
        atlases.run_deleted(LayerId(1));
        atlases.delete(AtlasId(3), &mut graveyard);
        assert!(graveyard.is_empty());
        assert_eq!(atlases.revision(LayerId(1)), 0);
        assert!(atlases.texture(AtlasId(3)).is_none());
    }

    #[test]
    fn replacing_or_deleting_an_atlas_buries_its_texture_and_drops_its_queue() {
        let mut atlases = atlases_with(vec![glyph(0.0, 0.0, 0, 0, 10, 10)]);
        atlases.queue(
            AtlasId(1),
            0,
            1,
            AtlasRect {
                x: 0,
                y: 0,
                w: 2,
                h: 2,
            },
            PathBuf::from("/nowhere"),
            &mut Vec::new(),
        );
        assert!(atlases.pending());
        let revision = atlases.revision(LayerId(5));
        let mut graveyard = Vec::new();
        atlases.create(AtlasId(1), 32, 32, &mut graveyard);
        assert_eq!(graveyard, [7]);
        assert!(!atlases.pending());
        assert_eq!(atlases.texture(AtlasId(1)), None);
        assert!(
            atlases.revision(LayerId(5)) > revision,
            "what it drew is gone"
        );
    }

    fn chunk(atlases: &mut Atlases, generation: u64, events: &mut Vec<Event>) {
        atlases.queue(
            AtlasId(1),
            (generation % 2) as u8,
            generation,
            AtlasRect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            PathBuf::from("/nowhere"),
            events,
        );
    }

    #[test]
    fn an_app_that_does_not_wait_for_its_slots_cannot_make_the_queue_grow_and_is_answered_for_every_chunk()
     {
        let mut atlases = atlases_with(vec![glyph(0.0, 0.0, 0, 0, 10, 10)]);
        let mut events = Vec::new();
        for generation in 1..=1000 {
            chunk(&mut atlases, generation, &mut events);
        }
        // Two chunks wait (a slot each); the other 998 were turned away and answered on the spot.
        assert_eq!(
            atlases.atlases[&AtlasId(1)].queue.len(),
            usize::from(STAGING_SLOTS)
        );
        let answered: Vec<u64> = events
            .iter()
            .map(|e| match e {
                Event::AtlasUploaded { generation, .. } => *generation,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(answered, (3..=1000).collect::<Vec<_>>());
        // A list that names the last generation is not left waiting for a chunk that was turned away: once the two that
        // wait have been dealt with, everything up to it has been.
        atlases.set_glyphs(LayerId(5), 1000, vec![glyph(1.0, 1.0, 0, 0, 2, 2)]);
        assert!(atlases.runs[&LayerId(5)].waiting.is_some());
        atlases.atlases.get_mut(&AtlasId(1)).unwrap().queue.clear();
        atlases.atlases.get_mut(&AtlasId(1)).unwrap().landed = 2;
        assert_eq!(atlases.reveal_ready(), 1);
    }

    #[test]
    fn a_list_swapped_into_a_run_of_a_layer_the_last_frame_did_not_draw_changes_nothing_on_the_panel()
     {
        // Layer 5 is on the page that is shown, layer 6 on one made ahead under a group that is not.
        let mut atlases = atlases_with(vec![glyph(0.0, 0.0, 0, 0, 1, 1)]);
        atlases.run_made(LayerId(6), AtlasId(1));
        let swap = |atlases: &mut Atlases, generation: u64| {
            let atlas = atlases.atlases.get_mut(&AtlasId(1)).unwrap();
            atlas.landed = generation - 1;
            for layer in [5, 6] {
                atlases.set_glyphs(
                    LayerId(layer),
                    generation,
                    vec![glyph(0.0, 0.0, 0, 0, 1, 1)],
                );
                assert!(atlases.runs[&LayerId(layer)].waiting.is_some());
            }
            atlases.atlases.get_mut(&AtlasId(1)).unwrap().landed = generation;
            atlases.reveal_ready()
        };
        assert_eq!(
            swap(&mut atlases, 2),
            2,
            "nobody has said which layers were drawn: every run counts"
        );
        atlases.set_shown([LayerId(5)].into_iter());
        let before = atlases.revision(LayerId(6));
        assert_eq!(
            swap(&mut atlases, 3),
            1,
            "only the run of the layer that was drawn is a change of the panel"
        );
        assert!(
            atlases.revision(LayerId(6)) > before,
            "the run of the layer that was not has its list all the same, for the frame that shows it"
        );
        // A chunk that lands under the quads of the lists: the same.
        assert_eq!(
            atlases.bump(
                AtlasId(1),
                Some(&AtlasRect {
                    x: 0,
                    y: 0,
                    w: 1,
                    h: 1
                })
            ),
            1
        );
        atlases.set_shown([LayerId(5), LayerId(6)].into_iter());
        assert_eq!(
            swap(&mut atlases, 4),
            2,
            "the page is shown now: both count"
        );
        assert_eq!(atlases.bump(AtlasId(1), None), 2);
    }

    #[test]
    fn a_line_in_the_log_is_let_through_once_a_second_and_says_how_many_it_held_back() {
        let mut quiet = Quiet::default();
        let start = Instant::now();
        assert_eq!(quiet.allow(start), Some(0));
        let held: Vec<_> = (1..=999)
            .map(|n| quiet.allow(start + Duration::from_millis(n)))
            .collect();
        assert!(held.iter().all(Option::is_none), "{held:?}");
        assert_eq!(quiet.allow(start + Duration::from_millis(1000)), Some(999));
        assert_eq!(quiet.allow(start + Duration::from_millis(1500)), None);
        assert_eq!(quiet.allow(start + Duration::from_secs(5)), Some(1));
    }

    #[test]
    fn the_mix_of_a_revision_into_a_version_is_the_version_for_0_and_changes_with_either() {
        assert_eq!(mixed(42, 0), 42);
        assert_ne!(mixed(42, 1), 42);
        assert_ne!(mixed(42, 1), mixed(42, 2));
        assert_ne!(mixed(42, 1), mixed(43, 1));
    }
}
