//! Glyph atlases and glyph runs, as the app holds them (design section 3.1, item 4): text that is the compositor's quads of a
//! texture it keeps, not a picture the app draws again whenever a digit changes.
//!
//! **Preparing** ([`AtlasView::prepare`], off the motion path: it rasterises, which costs milliseconds for a big glyph) takes the
//! raster of each character from `Fonts` (the same coverage bytes `Fonts::draw` blends from, so a run is pixel for pixel what the
//! CPU path made), packs it on a shelf of the atlas, and queues the pixels of the shelf's new stretch as one *band*: a rectangle
//! of the atlas of at most 256 KiB, all of it new.
//!
//! **Uploading** ([`AtlasView::pump`], and again by itself whenever the compositor reports a slot free) copies one band into a
//! *staging slot* (the shared file `resources::staging_path(atlas, slot)`; an atlas has two) and stages an `AtlasUpload` that
//! carries the next generation of the atlas. A slot is not written again until `Event::AtlasUploaded` says its upload has
//! reached the texture, so at most two bands are in flight and a third waits in the app's memory. The staged request rides in the
//! host's next commit; the pixels never cross the socket.
//!
//! **Showing** ([`GlyphRun::set_glyphs`]) swaps the whole list of quads of a run layer. It names (`after`) the newest generation
//! among the glyphs it uses, and the compositor holds the swap until every upload up to it has reached the texture, so a label is
//! revealed whole or not at all. A glyph whose band has not even been handed over yet is an error, not a hole. A clock whose
//! digits were prepared once changes a digit by a swap of at most 256 quads: no raster, no upload, no allocation.
//!
//! The atlas outlives the runs that draw from it: its deletion waits for them (their layers are deleted first).
//!
//! **Pages.** A run draws from the atlas it was made on for as long as it is a run, whatever the app has done since: [`GlyphRun::atlas`]
//! is a non-owning [`AtlasView`] of it, and what the run shows next is prepared and laid out through that view, never through the
//! atlas the app uses for *new* text by now (a UI that finds its text atlas full makes a fresh one for what it builds next; the
//! runs on the old page go on drawing from it, and it stays until the last of them is gone). A [`GlyphId`] names the atlas it
//! belongs to and a run refuses the glyphs of another, even when their index would exist in its own. A run that cannot show a
//! change at all (the page is full, the words need more quads or colours than a run has) is replaced, in the same compositor
//! layer, by a picture (`GlyphRun::become_picture`), and so lets go of its page.

use crate::components::icon::Icon;
use crate::error::{Kind, Limit, UiError};
use crate::handle::handle;
use crate::scene::{ImageLayer, Inner, Layer, Scene};
use crate::text::{Fonts, GlyphMetrics, Type};
use compositor_protocol as wire;
use compositor_protocol::resources::{
    INKS, MAX_ATLAS_BYTES, MAX_ATLAS_SIDE, MAX_RUN_GLYPHS, MAX_UPLOAD_BYTES, STAGING_SLOTS,
    staging_path,
};
use compositor_protocol::{AtlasRect, Content, Glyph, GlyphStyle, Props, Request, Rgba};
use memmap2::MmapMut;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;

handle!(
    /// Names an atlas in this process: an index and a generation, never reused. The compositor's id for it is
    /// [`AtlasView::wire_id`].
    AtlasId
);

/// The colours of a glyph run, by ink: what [`PlacedGlyph::ink`] indexes (premultiplied, as every colour of the wire is).
pub type Palette = [Rgba; INKS];

/// A palette whose ink 0 is `colour` and whose other inks draw nothing: the run of a single colour.
pub const fn one_ink(colour: Rgba) -> Palette {
    GlyphStyle::single(wire::AtlasId(0), colour).palette
}

/// A glyph prepared in an atlas: which atlas, and where in its table of glyphs (where its pixels are, how it is laid out). It
/// names the atlas that made it, so that a run can refuse the glyphs of another atlas whatever their index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphId {
    atlas: wire::AtlasId,
    index: u32,
}

/// A prepared glyph placed in a run: the top-left corner of its bitmap in the run layer's space, in whole pixels, and the ink
/// (an index into the run's palette) it is drawn in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedGlyph {
    pub glyph: GlyphId,
    pub x: f32,
    pub y: f32,
    pub ink: u8,
    /// How much of the bitmap's width is drawn (0: all of it): a line is kept as one strip, and a hairline as wide as a row is a
    /// quad of that width cut from it.
    pub crop: u16,
}

/// A mark the atlas keeps as it keeps a glyph: one shape of coverage (an icon, a hairline, a disc), drawn in an ink like text, so
/// that what is painted into a row's picture beside its words can be quads of its run instead. What each is, pixel for pixel, is
/// the painters' (`components::mark`); the atlas only keeps the coverage it is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sprite {
    /// An icon of the toolkit, `side` pixels square.
    Icon(Icon, u32),
    /// A line one pixel high and as wide as the atlas: a hairline of any width up to that is a crop of it.
    Rule,
    /// A filled disc of this diameter.
    Disc(u32),
    /// A rounded rectangle: width, height and corner radius.
    Round(u32, u32, u32),
    /// A bar of a signal mark: width and height.
    Bar(u32, u32),
    /// A rounded rectangle of one level of the ink: width, height, corner radius and the level (of 255).
    Veil(u32, u32, u32, u8),
    /// A rounded bar hatched with two levels of the ink: width, height, corner radius, the darker and the lighter level.
    Hatch(u32, u32, u32, u8, u8),
}

/// A staging slot: the shared file the compositor reads an upload from, and the upload in flight from it.
struct Staging {
    map: MmapMut,
    path: PathBuf,
    /// The generation of the upload the slot holds until `Event::AtlasUploaded` frees it.
    in_flight: Option<u64>,
}

/// A strip of the atlas of one height, filled left to right.
struct Shelf {
    y: u32,
    height: u32,
    /// The next free column.
    x: u32,
}

struct GlyphInfo {
    /// Where the bitmap's pixels are: its quad is this grown by [`MARGIN`] on every side.
    rect: AtlasRect,
    metrics: GlyphMetrics,
    /// Nothing to draw (a space).
    empty: bool,
    /// The generation of the upload that carries its pixels; 0 until the band has been handed over.
    generation: u64,
}

/// New pixels of one shelf, a rectangle of the atlas whose every pixel is new: one upload.
struct Band {
    rect: AtlasRect,
    bytes: Vec<u8>,
    glyphs: Vec<u32>,
}

/// A list of quads that waits for the bands of some of its glyphs to be handed over to the compositor (no generation to name
/// yet): it is sent, replacing what the run shows, when the last of them is.
struct Deferred {
    layer: wire::LayerId,
    glyphs: Vec<PlacedGlyph>,
}

pub(crate) struct AtlasEntry {
    wire: wire::AtlasId,
    width: u32,
    height: u32,
    slots: [Staging; STAGING_SLOTS as usize],
    /// The generation the next upload carries. Rises by one; 0 means "at once" in `SetGlyphs::after`.
    next_generation: u64,
    /// The newest generation the compositor has reported on any slot.
    applied: u64,
    glyphs: Vec<GlyphInfo>,
    index: HashMap<(Type, char), GlyphId>,
    /// The marks kept beside the glyphs.
    sprites: HashMap<Sprite, GlyphId>,
    shelves: Vec<Shelf>,
    pending: VecDeque<Band>,
    /// Band buffers kept for the next one.
    spare: Vec<Vec<u8>>,
    /// The lists of runs that wait for their glyphs to be handed over.
    deferred: Vec<Deferred>,
    /// Glyph runs drawing from the atlas.
    runs: u32,
    /// The `Atlas` handle is alive.
    handle: bool,
}

/// Pixels rastered (or made) and given a place on a shelf, waiting to be put in a band.
struct Placed {
    id: u32,
    shelf: usize,
    /// The column of the cell's left margin: the bitmap starts [`MARGIN`] further right.
    x: u32,
    /// The bitmap's size (the cell is [`MARGIN`] larger on every side).
    w: u32,
    h: u32,
    coverage: Vec<u8>,
}

impl Placed {
    fn cell_w(&self) -> u32 {
        self.w + 2 * MARGIN
    }
}

/// The pixels the packer puts between cells and shelves, so that sampling never reaches a neighbour.
const PAD: u32 = 1;

/// The zero texels round every bitmap, inside its cell, that its quad covers (quads are the bitmap grown by this on every side,
/// `run_quads`). The compositor filters a layer that sits between whole pixels, bilinearly, and a pixel that lies up to half a
/// texel outside the bitmap still gets a share of the bitmap's edge, exactly as it does from a picture of the same words: the
/// quad must reach it, and what the filter reads past the quad (the margin's far side) must be zero: the pad between cells is,
/// and past the atlas's own edge the texture clamps to the margin.
pub(crate) const MARGIN: u32 = 1;

impl AtlasEntry {
    /// The name of glyph number `index` of this atlas.
    fn glyph_id(&self, index: u32) -> GlyphId {
        GlyphId {
            atlas: self.wire,
            index,
        }
    }

    /// The glyph `glyph` names, if it is one of this atlas's: a glyph of another atlas has no entry here, whatever its index.
    fn info(&self, glyph: GlyphId) -> Option<&GlyphInfo> {
        self.glyphs
            .get(glyph.index as usize)
            .filter(|_| glyph.atlas == self.wire)
    }

    /// Nothing waits to be uploaded and nothing is in flight: every glyph prepared is on the texture.
    fn is_settled(&self) -> bool {
        self.pending.is_empty() && self.slots.iter().all(|s| s.in_flight.is_none())
    }

    /// A new shelf for bitmaps `h` rows high, of at least `room` rows (room for the next bitmaps of the same size, not only this
    /// one) while the page has them: its index, empty. `None` when the page has no room for a shelf of `h` rows.
    fn open_shelf(&mut self, h: u32, room: u32) -> Option<usize> {
        let top = self.shelves.last().map_or(0, |s| s.y + s.height + PAD);
        let mut height = h.max(room);
        if top + height > self.height {
            height = h;
        }
        if top + height > self.height {
            return None;
        }
        self.shelves.push(Shelf {
            y: top,
            height,
            x: 0,
        });
        Some(self.shelves.len() - 1)
    }

    /// The shelf with the least height, and of at least `h` rows, that has `w` columns free: the best fit (the first of equals).
    fn best_shelf(&self, w: u32, h: u32) -> Option<usize> {
        let mut best: Option<(usize, u32)> = None;
        for (i, shelf) in self.shelves.iter().enumerate() {
            if shelf.height >= h
                && shelf.x + w <= self.width
                && best.is_none_or(|(_, height)| shelf.height < height)
            {
                best = Some((i, shelf.height));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Where a `w` × `h` bitmap goes: the shelf `prefer` names if it has room for it, else the best-fitting shelf with room, or a
    /// new shelf of at least `room` rows. (Shelf index, column.)
    fn place(&mut self, w: u32, h: u32, room: u32, prefer: Option<usize>) -> Option<(usize, u32)> {
        let preferred = prefer.filter(|&i| {
            self.shelves
                .get(i)
                .is_some_and(|s| s.height >= h && s.x + w <= self.width)
        });
        if let Some(i) = preferred.or_else(|| self.best_shelf(w, h)) {
            let x = self.shelves[i].x;
            self.shelves[i].x += w + PAD;
            return Some((i, x));
        }
        if w > self.width {
            return None;
        }
        let i = self.open_shelf(h, room)?;
        self.shelves[i].x = w + PAD;
        Some((i, 0))
    }

    /// Gives a bitmap its place and its entry in the table of glyphs; nothing is queued yet. The place is a cell: the bitmap with
    /// [`MARGIN`] zero texels round it, which the glyph's quad covers.
    fn add(
        &mut self,
        metrics: GlyphMetrics,
        coverage: Vec<u8>,
        room: u32,
        prefer: Option<usize>,
    ) -> Result<Placed, UiError> {
        let (w, h) = (metrics.width as u32, metrics.height as u32);
        let (cell_w, cell_h) = (w + 2 * MARGIN, h + 2 * MARGIN);
        if cell_w * cell_h > MAX_UPLOAD_BYTES {
            return Err(UiError::Invalid("a glyph larger than one upload chunk"));
        }
        let Some((shelf, x)) = self.place(cell_w, cell_h, room + 2 * MARGIN, prefer) else {
            return Err(UiError::AtlasFull);
        };
        let id = self.glyphs.len() as u32;
        self.glyphs.push(GlyphInfo {
            rect: AtlasRect {
                x: (x + MARGIN) as u16,
                y: (self.shelves[shelf].y + MARGIN) as u16,
                w: w as u16,
                h: h as u16,
            },
            metrics,
            empty: false,
            generation: 0,
        });
        Ok(Placed {
            id,
            shelf,
            x,
            w,
            h,
            coverage,
        })
    }

    /// The shelf the characters of `text` that the atlas does not have yet should all go on, when there are two or more of them:
    /// the best-fitting shelf with room for every one, else a new shelf while the page has the rows. One shelf is one band (and
    /// the texts prepared one after another that continue it join into one upload). Glyph by glyph, each finds the shortest shelf
    /// with a gap that holds it, and a text of a dozen letters is a band on each of two or three shelves: a screen is made with
    /// two bands to hand over with, and a layer whose glyphs wait for a third is a picture.
    fn shelf_together(&mut self, fonts: &mut Fonts, ty: Type, text: &str) -> Option<usize> {
        let mut fresh: Vec<char> = Vec::new();
        let (mut columns, mut tallest) = (0, 0);
        for c in text.chars() {
            if self.index.contains_key(&(ty, c)) || fresh.contains(&c) {
                continue;
            }
            let (metrics, _) = fonts.raster(ty, c);
            if metrics.width == 0 || metrics.height == 0 {
                continue;
            }
            fresh.push(c);
            columns += metrics.width as u32 + 2 * MARGIN;
            tallest = tallest.max(metrics.height as u32 + 2 * MARGIN);
        }
        if fresh.len() < 2 {
            return None;
        }
        columns += PAD * (fresh.len() as u32 - 1);
        if columns > self.width {
            return None;
        }
        self.best_shelf(columns, tallest)
            .or_else(|| self.open_shelf(tallest, ty.px + 2 * MARGIN))
    }

    /// Rasters, packs and queues the characters of `text` that the atlas does not have yet: the glyphs of a text go on one shelf
    /// when one has room for them all ([`AtlasEntry::shelf_together`]), for the labels made as a screen is.
    fn prepare(&mut self, fonts: &mut Fonts, ty: Type, text: &str) -> Result<(), UiError> {
        let shelf = self.shelf_together(fonts, ty, text);
        self.prepare_on(fonts, ty, text, shelf)
    }

    /// [`AtlasEntry::prepare`] glyph by glyph, each on the shortest shelf that has a gap for it: the densest packing, for a warm
    /// that fills the page up to its bound.
    fn prepare_densely(&mut self, fonts: &mut Fonts, ty: Type, text: &str) -> Result<(), UiError> {
        self.prepare_on(fonts, ty, text, None)
    }

    /// Rasters, packs and queues the characters of `text` that the atlas does not have yet, those of them that fit on the shelf
    /// `prefer` names (if any) on it.
    fn prepare_on(
        &mut self,
        fonts: &mut Fonts,
        ty: Type,
        text: &str,
        prefer: Option<usize>,
    ) -> Result<(), UiError> {
        let mut placed: Vec<Placed> = Vec::new();
        let mut failure = None;
        for c in text.chars() {
            if self.index.contains_key(&(ty, c)) {
                continue;
            }
            let (metrics, coverage) = fonts.raster(ty, c);
            if metrics.width == 0 || metrics.height == 0 {
                let id = self.glyphs.len() as u32;
                self.glyphs.push(GlyphInfo {
                    rect: AtlasRect {
                        x: 0,
                        y: 0,
                        w: 0,
                        h: 0,
                    },
                    metrics,
                    empty: true,
                    generation: 0,
                });
                self.index.insert((ty, c), self.glyph_id(id));
                continue;
            }
            match self.add(metrics, coverage.to_vec(), ty.px, prefer) {
                Ok(p) => {
                    self.index.insert((ty, c), self.glyph_id(p.id));
                    placed.push(p);
                }
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        let queued = self.queue(placed);
        match failure {
            Some(e) => Err(e),
            None => queued,
        }
    }

    /// [`AtlasEntry::prepare`] that opens no shelf below the first `rows` rows of the page: what does not fit above that line is
    /// left alone and is no error (a warm that stops at its bound has done what it was asked to do). Glyphs that fit on shelves
    /// already there are still placed, each on the shortest shelf with a gap that holds it. A line at or below the page's end is no
    /// bound: the page being full is an error as ever.
    fn prepare_within(
        &mut self,
        fonts: &mut Fonts,
        ty: Type,
        text: &str,
        rows: u32,
    ) -> Result<(), UiError> {
        if rows >= self.height {
            return self.prepare_densely(fonts, ty, text);
        }
        let page = std::mem::replace(&mut self.height, rows);
        let prepared = self.prepare_densely(fonts, ty, text);
        self.height = page;
        match prepared {
            Err(UiError::AtlasFull) => Ok(()),
            other => other,
        }
    }

    /// Packs and queues the mark `sprite`, `w` x `h` bytes of coverage (rows packed), unless the atlas has it.
    fn prepare_sprite(
        &mut self,
        sprite: Sprite,
        make: impl FnOnce() -> (u32, u32, Vec<u8>),
    ) -> Result<GlyphId, UiError> {
        if let Some(&id) = self.sprites.get(&sprite) {
            return Ok(id);
        }
        let (w, h, coverage) = make();
        if w == 0 || h == 0 || coverage.len() != w as usize * h as usize {
            return Err(UiError::Invalid("a mark has pixels"));
        }
        let metrics = GlyphMetrics {
            xmin: 0,
            ymin: 0,
            width: w as usize,
            height: h as usize,
            advance: 0.0,
        };
        let placed = self.add(metrics, coverage, h, None)?;
        let id = self.glyph_id(placed.id);
        self.sprites.insert(sprite, id);
        self.queue(vec![placed])?;
        Ok(id)
    }

    /// [`AtlasEntry::prepare_sprite`] that opens no shelf below the first `rows` rows of the page, as [`AtlasEntry::prepare_within`]
    /// does for glyphs: a mark that does not fit above that line is left alone and is no error (a warm that stops at its bound has
    /// done what it was asked to do). A line at or below the page's end is no bound.
    fn prepare_sprite_within(
        &mut self,
        sprite: Sprite,
        make: impl FnOnce() -> (u32, u32, Vec<u8>),
        rows: u32,
    ) -> Result<(), UiError> {
        if rows >= self.height {
            return self.prepare_sprite(sprite, make).map(|_| ());
        }
        let page = std::mem::replace(&mut self.height, rows);
        let prepared = self.prepare_sprite(sprite, make);
        self.height = page;
        match prepared {
            Err(UiError::AtlasFull) => Ok(()),
            other => other.map(|_| ()),
        }
    }

    /// Queues the pixels of `placed` as bands: the neighbours on one shelf together, at most one upload chunk each.
    fn queue(&mut self, mut placed: Vec<Placed>) -> Result<(), UiError> {
        placed.sort_by_key(|p| (p.shelf, p.x));
        let mut i = 0;
        while i < placed.len() {
            let shelf = placed[i].shelf;
            let (shelf_y, shelf_h) = (self.shelves[shelf].y, self.shelves[shelf].height);
            let x0 = placed[i].x;
            let mut j = i;
            // A band is a rectangle of at most one upload chunk: as many neighbours as fit.
            while j < placed.len()
                && placed[j].shelf == shelf
                && (placed[j].x + placed[j].cell_w() - x0) * shelf_h <= MAX_UPLOAD_BYTES
            {
                j += 1;
            }
            if j == i {
                return Err(UiError::Invalid("a glyph larger than one upload chunk"));
            }
            let x1 = placed[j - 1].x + placed[j - 1].cell_w();
            let band_w = (x1 - x0) as usize;
            let mut bytes = self.spare.pop().unwrap_or_default();
            bytes.clear();
            bytes.resize(band_w * shelf_h as usize, 0);
            for p in &placed[i..j] {
                let left = (p.x - x0 + MARGIN) as usize;
                for row in 0..p.h as usize {
                    let w = p.w as usize;
                    bytes[(row + MARGIN as usize) * band_w + left..][..w]
                        .copy_from_slice(&p.coverage[row * w..][..w]);
                }
            }
            self.push_band(Band {
                rect: AtlasRect {
                    x: x0 as u16,
                    y: shelf_y as u16,
                    w: band_w as u16,
                    h: shelf_h as u16,
                },
                bytes,
                glyphs: placed[i..j].iter().map(|p| p.id).collect(),
            });
            i = j;
        }
        Ok(())
    }

    /// Puts `band` behind the ones waiting. A band that continues the one before it on the same shelf joins it, so that the
    /// glyphs and marks a screen prepares one text at a time travel as one upload, not one each.
    fn push_band(&mut self, band: Band) {
        if let Some(last) = self.pending.back_mut() {
            let (a, b) = (last.rect, band.rect);
            // Only meaningful for a band to the right of `last` on its shelf (checked below): a band on a shelf that starts further
            // left must not be taken for one that continues it.
            let width = (u32::from(b.x) + u32::from(b.w)).saturating_sub(u32::from(a.x));
            if a.y == b.y
                && a.h == b.h
                && u32::from(a.x) + u32::from(a.w) <= u32::from(b.x)
                && width * u32::from(a.h) <= MAX_UPLOAD_BYTES
            {
                let (aw, bw, h) = (usize::from(a.w), usize::from(b.w), usize::from(a.h));
                let (width, left) = (width as usize, usize::from(b.x - a.x));
                let mut bytes = self.spare.pop().unwrap_or_default();
                bytes.clear();
                bytes.resize(width * h, 0);
                for row in 0..h {
                    bytes[row * width..][..aw].copy_from_slice(&last.bytes[row * aw..][..aw]);
                    bytes[row * width + left..][..bw]
                        .copy_from_slice(&band.bytes[row * bw..][..bw]);
                }
                self.spare.push(std::mem::replace(&mut last.bytes, bytes));
                self.spare.push(band.bytes);
                last.rect.w = width as u16;
                last.glyphs.extend(band.glyphs);
                return;
            }
        }
        self.pending.push_back(band);
    }
}

impl Inner {
    /// Checks a new atlas against the ceilings and names it; the staging files are made before it is inserted.
    fn atlas_reserve(&mut self, width: u32, height: u32) -> Result<wire::AtlasId, UiError> {
        if width == 0 || height == 0 {
            return Err(UiError::Invalid("an atlas has pixels"));
        }
        if width > MAX_ATLAS_SIDE || height > MAX_ATLAS_SIDE {
            return Err(UiError::Limit(Limit::AtlasSide));
        }
        if self.atlases.is_full() {
            return Err(UiError::Limit(Limit::Atlases));
        }
        let held: u64 = self
            .atlases
            .iter()
            .map(|(_, _, a)| u64::from(a.width) * u64::from(a.height))
            .sum();
        if held + u64::from(width) * u64::from(height) > MAX_ATLAS_BYTES {
            return Err(UiError::Limit(Limit::AtlasBytes));
        }
        let id = wire::AtlasId(self.next_atlas);
        self.next_atlas += 1;
        Ok(id)
    }

    fn atlas_insert(
        &mut self,
        wire: wire::AtlasId,
        width: u32,
        height: u32,
        slots: [Staging; STAGING_SLOTS as usize],
    ) -> AtlasId {
        let entry = AtlasEntry {
            wire,
            width,
            height,
            slots,
            next_generation: 1,
            applied: 0,
            glyphs: Vec::new(),
            index: HashMap::new(),
            sprites: HashMap::new(),
            shelves: Vec::new(),
            pending: VecDeque::new(),
            spare: Vec::new(),
            deferred: Vec::new(),
            runs: 0,
            handle: true,
        };
        let key = self.atlases.insert(entry).expect("room was checked");
        self.batch.push(Request::AtlasCreate {
            id: wire,
            width,
            height,
        });
        AtlasId::from_key(key)
    }

    /// Copies pending bands into the free staging slots and stages their uploads. Returns how many. The lists of runs that were
    /// waiting for a band to be handed over are sent when the last band they wait for has been.
    pub(crate) fn atlas_pump(&mut self, id: AtlasId) -> usize {
        let Some(entry) = self.atlases.get_mut(id.key()) else {
            return 0;
        };
        let mut staged = 0;
        for slot in 0..entry.slots.len() {
            if entry.slots[slot].in_flight.is_some() {
                continue;
            }
            let Some(band) = entry.pending.pop_front() else {
                break;
            };
            entry.slots[slot].map[..band.bytes.len()].copy_from_slice(&band.bytes);
            let generation = entry.next_generation;
            entry.next_generation += 1;
            entry.slots[slot].in_flight = Some(generation);
            for glyph in &band.glyphs {
                entry.glyphs[*glyph as usize].generation = generation;
            }
            self.batch.push(Request::AtlasUpload {
                id: entry.wire,
                slot: slot as u8,
                generation,
                rect: band.rect,
            });
            entry.spare.push(band.bytes);
            staged += 1;
        }
        if staged > 0 {
            self.atlas_reveal(id);
        }
        staged
    }

    /// Sends the lists that were waiting for bands to be handed over, for those whose glyphs all have been.
    fn atlas_reveal(&mut self, id: AtlasId) {
        let Some(entry) = self.atlases.get_mut(id.key()) else {
            return;
        };
        let waiting = std::mem::take(&mut entry.deferred);
        let mut still = Vec::new();
        for list in waiting {
            match self.run_quads(id, &list.glyphs) {
                Ok(Some((quads, after))) => self.run_send(list.layer, after, quads),
                Ok(None) => still.push(list),
                // A list that cannot be shown at all (its atlas went) is no one's now.
                Err(_) => {}
            }
        }
        if let Some(entry) = self.atlases.get_mut(id.key()) {
            entry.deferred.extend(still);
        }
    }

    /// The quads of `glyphs` for a run of atlas `atlas`, in ink order, and the newest generation among the glyphs they use; `None`
    /// when a glyph's band has not been handed over yet (nothing to name). Errors are for what no later moment will make right.
    fn run_quads(
        &mut self,
        atlas: AtlasId,
        glyphs: &[PlacedGlyph],
    ) -> Result<Option<(Vec<Glyph>, u64)>, UiError> {
        if glyphs.len() > MAX_RUN_GLYPHS {
            return Err(UiError::Limit(Limit::RunGlyphs));
        }
        let entry = self
            .atlases
            .get(atlas.key())
            .ok_or(UiError::Stale(Kind::Atlas))?;
        let (mut after, mut missing, mut top) = (0, false, 0u8);
        for placed in glyphs {
            // A glyph names the atlas that made it: one of another atlas is refused even when its index is in this one's table.
            let info = entry
                .info(placed.glyph)
                .ok_or(UiError::Invalid("a glyph of another atlas"))?;
            if !(placed.x.is_finite() && placed.y.is_finite()) {
                return Err(UiError::Invalid("a glyph's place must be numbers"));
            }
            if usize::from(placed.ink) >= INKS {
                return Err(UiError::Invalid("a glyph's ink is not in the palette"));
            }
            top = top.max(placed.ink);
            if info.empty {
                continue;
            }
            if info.generation == 0 {
                missing = true;
                continue;
            }
            after = after.max(info.generation);
        }
        if missing {
            return Ok(None);
        }
        let mut quads = self.glyph_pool.pop().unwrap_or_default();
        quads.clear();
        for ink in 0..=top {
            for placed in glyphs.iter().filter(|p| p.ink == ink) {
                let info = &entry.glyphs[placed.glyph.index as usize];
                if info.empty {
                    continue;
                }
                // The quad is the bitmap's rectangle grown by MARGIN zero texels on every side. A layer that sits between whole
                // pixels is filtered by the compositor, and a pixel beside the bitmap gets the share of its edge that the filter
                // of a picture gives it: a quad cut at the bitmap would drop that share. A mark cut to `crop` columns ends at
                // the cut, where the strip it is cut from goes on.
                let margin = MARGIN as u16;
                let drawn = match placed.crop {
                    0 => info.rect.w,
                    crop => crop.min(info.rect.w),
                };
                let right = if drawn == info.rect.w { margin } else { 0 };
                quads.push(Glyph {
                    x: placed.x - MARGIN as f32,
                    y: placed.y - MARGIN as f32,
                    sx: info.rect.x - margin,
                    sy: info.rect.y - margin,
                    w: margin + drawn + right,
                    h: info.rect.h + 2 * margin,
                    ink,
                });
            }
        }
        Ok(Some((quads, after)))
    }

    /// Stages `quads` as the list of layer `id`: a swap staged before the commit is replaced, not queued behind.
    fn run_send(&mut self, id: wire::LayerId, after: u64, mut quads: Vec<Glyph>) {
        for request in self.batch.iter_mut().rev() {
            if let Request::SetGlyphs {
                id: staged,
                after: staged_after,
                glyphs: staged_quads,
            } = request
            {
                if *staged == id {
                    *staged_after = after;
                    std::mem::swap(staged_quads, &mut quads);
                    quads.clear();
                    self.glyph_pool.push(quads);
                    return;
                }
            }
        }
        self.push(Request::SetGlyphs {
            id,
            after,
            glyphs: quads,
        });
    }

    pub(crate) fn observe_atlas_uploaded(&mut self, id: wire::AtlasId, slot: u8, generation: u64) {
        let Some(key) = self
            .atlases
            .iter()
            .find(|(_, _, a)| a.wire == id)
            .map(|(i, g, _)| AtlasId::new(i, g))
        else {
            return;
        };
        let entry = self.atlases.get_mut(key.key()).expect("found above");
        if let Some(s) = entry.slots.get_mut(usize::from(slot)) {
            if s.in_flight.is_some_and(|g| generation >= g) {
                s.in_flight = None;
            }
        }
        entry.applied = entry.applied.max(generation);
        self.atlas_pump(key);
    }

    /// A glyph run or the handle is gone: the atlas goes when neither is left.
    fn atlas_free(&mut self, id: AtlasId) {
        let Some(entry) = self.atlases.get(id.key()) else {
            return;
        };
        if entry.handle || entry.runs > 0 {
            return;
        }
        let entry = self.atlases.remove(id.key()).expect("looked up above");
        self.batch.push(Request::AtlasDelete { id: entry.wire });
        for slot in &entry.slots {
            self.retire_file(&slot.path);
        }
    }

    fn atlas_handle_dropped(&mut self, id: AtlasId) {
        if let Some(entry) = self.atlases.get_mut(id.key()) {
            entry.handle = false;
        }
        self.atlas_free(id);
    }

    fn atlas_run_gone(&mut self, id: AtlasId, layer: wire::LayerId) {
        if let Some(entry) = self.atlases.get_mut(id.key()) {
            entry.runs = entry.runs.saturating_sub(1);
            entry.deferred.retain(|list| list.layer != layer);
        }
        self.atlas_free(id);
    }

    /// Lets go of the list that waits for layer `layer`'s glyphs: a newer list has come, which is the one to show.
    fn atlas_forget(&mut self, id: AtlasId, layer: wire::LayerId) {
        if let Some(entry) = self.atlases.get_mut(id.key()) {
            entry.deferred.retain(|list| list.layer != layer);
        }
    }

    /// Keeps `glyphs` for layer `layer` until the bands of all of them have been handed over, replacing a list that waited.
    fn atlas_defer(&mut self, id: AtlasId, layer: wire::LayerId, glyphs: &[PlacedGlyph]) {
        if let Some(entry) = self.atlases.get_mut(id.key()) {
            entry.deferred.retain(|list| list.layer != layer);
            entry.deferred.push(Deferred {
                layer,
                glyphs: glyphs.to_vec(),
            });
        }
    }
}

/// A glyph atlas: a coverage texture in the compositor that the app fills glyph by glyph. Dropping the handle deletes the atlas
/// once the runs that draw from it are gone. Preparing, laying out and handing over are on the [`AtlasView`] it derefs to.
pub struct Atlas {
    view: AtlasView,
}

/// A non-owning look at an atlas: everything an [`Atlas`] handle does with it (prepare, lay out, hand over), and what a run draws
/// from ([`GlyphRun::atlas`]). Holding one keeps nothing alive: the atlas goes when its handle and its runs do, and what a view
/// of one that has gone is asked is an error, or nothing, never a panic.
#[derive(Clone)]
pub struct AtlasView {
    id: AtlasId,
    wire: wire::AtlasId,
    size: (u32, u32),
    scene: Scene,
}

impl std::ops::Deref for Atlas {
    type Target = AtlasView;
    fn deref(&self) -> &AtlasView {
        &self.view
    }
}

impl Atlas {
    /// A non-owning view of the atlas (what a run keeps of it), to prepare and lay out through while the handle is used for
    /// something else.
    pub fn view(&self) -> AtlasView {
        self.view.clone()
    }
}

impl Scene {
    /// An atlas of `width` × `height` coverage bytes. At most 4 at once, a side of at most 2048 and 4 MiB together; its two
    /// staging files are made now.
    pub fn atlas(&self, width: u32, height: u32) -> Result<Atlas, UiError> {
        let wire = self.inner.borrow_mut().atlas_reserve(width, height)?;
        let bytes = (u64::from(width) * u64::from(height)).min(u64::from(MAX_UPLOAD_BYTES));
        let slots = [0u8, 1].map(|slot| self.staging(wire, slot, bytes));
        let [a, b] = slots;
        let slots = [a?, b?];
        let id = self
            .inner
            .borrow_mut()
            .atlas_insert(wire, width, height, slots);
        Ok(Atlas {
            view: AtlasView {
                id,
                wire,
                size: (width, height),
                scene: self.clone(),
            },
        })
    }

    /// Whether every atlas of the scene is settled (the text page and the pages older runs still draw from alike).
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn atlases_settled(&self) -> bool {
        self.inner
            .borrow()
            .atlases
            .iter()
            .all(|(_, _, entry)| entry.is_settled())
    }

    fn staging(&self, atlas: wire::AtlasId, slot: u8, bytes: u64) -> Result<Staging, UiError> {
        let path = self.file_of(staging_path(atlas, slot));
        let file = self.shm_file(&path, bytes)?;
        // SAFETY: the file is private to this atlas; the compositor only reads it, after the app has said it wrote it and
        // until the compositor says it has read it; the mapping is dropped with the atlas before the file is removed.
        let map = unsafe { MmapMut::map_mut(&file) }?;
        Ok(Staging {
            map,
            path,
            in_flight: None,
        })
    }

    /// A run layer of `width` × `height` that draws quads of `atlas`, each in the colour of `palette` (premultiplied) its ink
    /// names, times the glyph's coverage. Its glyphs come from [`GlyphRun::set_glyphs`]; the atlas stays until the run is gone.
    pub fn glyph_run(
        &self,
        atlas: &Atlas,
        width: u32,
        height: u32,
        z: i32,
        palette: Palette,
        props: Props,
    ) -> Result<GlyphRun, UiError> {
        if !std::rc::Rc::ptr_eq(&atlas.scene.inner, &self.inner) {
            return Err(UiError::Stale(Kind::Atlas));
        }
        if width == 0 || height == 0 {
            return Err(UiError::Invalid("a glyph run has an area"));
        }
        let wire = {
            let mut inner = self.inner.borrow_mut();
            let entry = inner
                .atlases
                .get_mut(atlas.id.key())
                .ok_or(UiError::Stale(Kind::Atlas))?;
            entry.runs += 1;
            entry.wire
        };
        let id = self.allocate_layer();
        let layer = self.create(
            id,
            width,
            height,
            z,
            Content::Glyphs(GlyphStyle {
                atlas: wire,
                palette,
            }),
            props,
        );
        Ok(GlyphRun {
            layer,
            _pin: AtlasPin {
                scene: self.clone(),
                id: atlas.id,
                layer: id,
            },
            atlas: atlas.view.clone(),
            width,
            height,
            z,
        })
    }
}

impl AtlasView {
    pub fn id(&self) -> AtlasId {
        self.id
    }

    /// Runs `f` on the atlas's table; `R::default()` when the atlas has gone (a view that was kept past its atlas).
    fn with<R: Default>(&self, f: impl FnOnce(&AtlasEntry) -> R) -> R {
        self.with_or(R::default(), f)
    }

    fn with_or<R>(&self, gone: R, f: impl FnOnce(&AtlasEntry) -> R) -> R {
        self.scene
            .inner
            .borrow()
            .atlases
            .get(self.id.key())
            .map_or(gone, f)
    }

    /// The compositor's name for the atlas (what `GlyphStyle::atlas` of a run names).
    pub fn wire_id(&self) -> wire::AtlasId {
        self.wire
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The widest bitmap a shelf takes: the atlas's width less the margins round a bitmap. A hairline's strip is this wide.
    pub fn widest(&self) -> u32 {
        self.size.0.saturating_sub(2 * MARGIN)
    }

    /// Rasters (through `fonts`' cache) and packs the characters of `text` at `ty` that the atlas does not hold yet, and queues
    /// their pixels for upload. The new glyphs of one text go together on one shelf when one has room for them all, so that they
    /// travel as one band (the screen being made has two to hand over with). **Off the motion path**: a glyph costs a raster
    /// (milliseconds when big and not cached) and the call allocates. Call [`AtlasView::pump`] (or let the compositor's
    /// acknowledgements do) to hand the pixels over. `Err(AtlasFull)` when the atlas has no room left: make a new one at a settled
    /// boundary and drop this.
    pub fn prepare(&self, fonts: &mut Fonts, ty: Type, text: &str) -> Result<(), UiError> {
        let mut inner = self.scene.inner.borrow_mut();
        inner
            .atlases
            .get_mut(self.id.key())
            .ok_or(UiError::Stale(Kind::Atlas))?
            .prepare(fonts, ty, text)
    }

    /// [`AtlasView::prepare`] that opens no shelf below the first `rows` rows of the page (`used_rows` stays within them): what
    /// does not fit above that line is left alone and is no error, so the glyphs of the text in front are the ones that are there.
    /// A warm uses it to leave the rest of the page to the labels that ask for glyphs as they are made, and it packs glyph by glyph,
    /// each in the shortest gap that holds it, to take the fewest rows (its bands go over in the time a warm has). A line at or
    /// below the page's end bounds nothing: `Err(AtlasFull)` when the page is full, as `prepare` says.
    pub fn prepare_within(
        &self,
        fonts: &mut Fonts,
        ty: Type,
        text: &str,
        rows: u32,
    ) -> Result<(), UiError> {
        let mut inner = self.scene.inner.borrow_mut();
        inner
            .atlases
            .get_mut(self.id.key())
            .ok_or(UiError::Stale(Kind::Atlas))?
            .prepare_within(fonts, ty, text, rows)
    }

    /// Makes the mark `sprite` ready unless the atlas has it: `make` is asked (with the widest bitmap a shelf takes, the atlas's
    /// width less the margins, for a hairline's strip) for its `(width, height, coverage)`, rows packed, only when it does not.
    /// **Off the motion path**, like [`AtlasView::prepare`].
    pub fn prepare_sprite(
        &self,
        sprite: Sprite,
        make: impl FnOnce(u32) -> (u32, u32, Vec<u8>),
    ) -> Result<(), UiError> {
        let mut inner = self.scene.inner.borrow_mut();
        let entry = inner
            .atlases
            .get_mut(self.id.key())
            .ok_or(UiError::Stale(Kind::Atlas))?;
        let width = entry.width.saturating_sub(2 * MARGIN);
        entry.prepare_sprite(sprite, || make(width)).map(|_| ())
    }

    /// [`AtlasView::prepare_sprite`] that opens no shelf below the first `rows` rows of the page, as
    /// [`AtlasView::prepare_within`] does for glyphs: what does not fit above that line is left alone and is no error.
    pub fn prepare_sprite_within(
        &self,
        sprite: Sprite,
        make: impl FnOnce(u32) -> (u32, u32, Vec<u8>),
        rows: u32,
    ) -> Result<(), UiError> {
        let mut inner = self.scene.inner.borrow_mut();
        let entry = inner
            .atlases
            .get_mut(self.id.key())
            .ok_or(UiError::Stale(Kind::Atlas))?;
        let width = entry.width.saturating_sub(2 * MARGIN);
        entry.prepare_sprite_within(sprite, || make(width), rows)
    }

    /// The prepared mark `sprite`, if there is one.
    pub fn sprite(&self, sprite: Sprite) -> Option<GlyphId> {
        self.with(|a| a.sprites.get(&sprite).copied())
    }

    /// Appends the mark `sprite` with its top-left corner at `at` of the run, cut to `crop` pixels of its width (0: all of it), in
    /// ink `ink`. It must have been prepared.
    pub fn layout_sprite(
        &self,
        sprite: Sprite,
        at: (i32, i32),
        crop: u16,
        ink: u8,
        out: &mut Vec<PlacedGlyph>,
    ) -> Result<(), UiError> {
        let glyph = self
            .sprite(sprite)
            .ok_or(UiError::Invalid("a mark is not prepared in the atlas"))?;
        out.push(PlacedGlyph {
            glyph,
            x: at.0 as f32,
            y: at.1 as f32,
            ink,
            crop,
        });
        Ok(())
    }

    /// Hands pending bands to the free staging slots: stages their `AtlasUpload`s in the next commit. Returns how many. A slot
    /// is only written when the compositor has said its last upload reached the texture.
    pub fn pump(&self) -> usize {
        self.scene.inner.borrow_mut().atlas_pump(self.id)
    }

    /// The prepared glyph for `c` at `ty`, if there is one.
    pub fn glyph(&self, ty: Type, c: char) -> Option<GlyphId> {
        self.with(|a| a.index.get(&(ty, c)).copied())
    }

    /// How far the pen advances after the glyph; `None` for a glyph of another atlas.
    pub fn advance(&self, glyph: GlyphId) -> Option<f32> {
        self.with(|a| a.info(glyph).map(|info| info.metrics.advance))
    }

    /// Appends the glyphs of `text` at `ty`, the pen starting at `origin` = (x, baseline), to `out`, in ink `ink`: the same
    /// equations as `Fonts::draw` (the bitmap's corner at the pen plus `xmin`, the baseline less the bitmap's height and `ymin`,
    /// each rounded to a whole pixel). Returns the pen after the text. Every character must have been prepared. Allocates only if
    /// `out` grows.
    pub fn layout(
        &self,
        text: &str,
        ty: Type,
        origin: (f32, f32),
        ink: u8,
        out: &mut Vec<PlacedGlyph>,
    ) -> Result<f32, UiError> {
        let inner = self.scene.inner.borrow();
        let entry = inner
            .atlases
            .get(self.id.key())
            .ok_or(UiError::Stale(Kind::Atlas))?;
        let (mut pen, baseline) = origin;
        for c in text.chars() {
            let id = *entry.index.get(&(ty, c)).ok_or(UiError::Invalid(
                "a character of the text is not prepared in the atlas",
            ))?;
            let info = &entry.glyphs[id.index as usize];
            if !info.empty {
                let m = &info.metrics;
                out.push(PlacedGlyph {
                    glyph: id,
                    x: (pen + m.xmin as f32).round(),
                    y: (baseline - m.height as f32 - m.ymin as f32).round(),
                    ink,
                    crop: 0,
                });
            }
            pen += info.metrics.advance;
        }
        Ok(pen)
    }

    /// Whether nothing waits to be uploaded and nothing is in flight: every prepared glyph is on the texture (true of an atlas
    /// that has gone).
    pub fn is_settled(&self) -> bool {
        self.with_or(true, AtlasEntry::is_settled)
    }

    /// Bands waiting for a free staging slot.
    pub fn pending_bands(&self) -> usize {
        self.with(|a| a.pending.len())
    }

    /// The newest upload generation the compositor has reported done.
    pub fn applied_generation(&self) -> u64 {
        self.with(|a| a.applied)
    }

    /// Rows of the atlas the shelves have used.
    pub fn used_rows(&self) -> u32 {
        self.with(|a| a.shelves.last().map_or(0, |s| s.y + s.height))
    }
}

impl Drop for Atlas {
    fn drop(&mut self) {
        self.view
            .scene
            .inner
            .borrow_mut()
            .atlas_handle_dropped(self.view.id);
    }
}

/// A run's hold on its atlas, released after the run's layer has been deleted.
struct AtlasPin {
    scene: Scene,
    id: AtlasId,
    layer: wire::LayerId,
}

impl Drop for AtlasPin {
    fn drop(&mut self) {
        self.scene
            .inner
            .borrow_mut()
            .atlas_run_gone(self.id, self.layer);
    }
}

/// A layer of glyph quads of an atlas, in up to [`INKS`] colours.
pub struct GlyphRun {
    // Dropped first: the layer's delete is staged before the atlas's.
    layer: Layer,
    _pin: AtlasPin,
    /// The atlas the run draws from while it is a run. Everything it shows next is laid out through this, whatever atlas the app
    /// makes new runs on by then.
    atlas: AtlasView,
    /// The layer's box and depth: what a picture that takes its place is made with ([`GlyphRun::become_picture`]).
    width: u32,
    height: u32,
    z: i32,
}

impl GlyphRun {
    /// The atlas the run draws from: what its next quads must be prepared and laid out in, even when the app makes the words of new
    /// runs on another atlas by then (a run is never moved to another: the quads it has are rectangles of this one).
    pub fn atlas(&self) -> AtlasView {
        self.atlas.clone()
    }

    /// The layer's size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Swaps the run's whole list of quads, sorted by ink (stably: the order within an ink is the order given). At most 256; every
    /// glyph must be prepared *and* handed over (`AtlasView::pump`) in the run's own atlas, or the call is an
    /// `Err(GlyphNotUploaded)` and nothing changes (see [`GlyphRun::set_glyphs_when_ready`] for a swap that waits); a glyph of
    /// another atlas is an `Err(Invalid)`. The swap takes effect when the compositor has every upload up to the newest of the
    /// glyphs used, so the label appears whole. A second call before a commit replaces the first. Allocates nothing once the run
    /// has been used (its vectors come from a pool).
    pub fn set_glyphs(&mut self, glyphs: &[PlacedGlyph]) -> Result<(), UiError> {
        let mut guard = self.layer.scene.inner.borrow_mut();
        let inner = &mut *guard;
        let id = self.layer.id();
        match inner.run_quads(self.atlas.id, glyphs)? {
            Some((quads, after)) => {
                inner.atlas_forget(self.atlas.id, id);
                inner.run_send(id, after, quads);
                Ok(())
            }
            None => Err(UiError::GlyphNotUploaded),
        }
    }

    /// Like [`GlyphRun::set_glyphs`], but a list with a glyph whose band has not been handed over yet (the atlas is busy with
    /// the bands before it) is kept and swapped in as soon as the last of its bands is, instead of being refused: what the run
    /// shows stays until then, and a newer list replaces one that waits. For text that changes after the run was made.
    pub fn set_glyphs_when_ready(&mut self, glyphs: &[PlacedGlyph]) -> Result<(), UiError> {
        let mut guard = self.layer.scene.inner.borrow_mut();
        let inner = &mut *guard;
        let id = self.layer.id();
        match inner.run_quads(self.atlas.id, glyphs)? {
            Some((quads, after)) => {
                inner.atlas_forget(self.atlas.id, id);
                inner.run_send(id, after, quads);
            }
            None => inner.atlas_defer(self.atlas.id, id, glyphs),
        }
        Ok(())
    }

    /// Whether a list waits for the bands of its glyphs to be handed over.
    pub fn is_waiting(&self) -> bool {
        let inner = self.layer.scene.inner.borrow();
        let id = self.layer.id();
        inner
            .atlases
            .get(self.atlas.id.key())
            .is_some_and(|a| a.deferred.iter().any(|list| list.layer == id))
    }

    /// Makes the run's layer a picture, in place, for words the run cannot show (see [`Layer::become_picture`]): the same compositor
    /// layer with the same id, group and place, now pixels the caller paints and damages. The run is spent when this returns `Ok`:
    /// drop it at once (that lets go of its atlas, and with it a list that waited for glyphs); with `Err` nothing changed.
    pub(crate) fn become_picture(&mut self) -> io::Result<ImageLayer> {
        self.layer.become_picture(self.width, self.height, self.z)
    }
}

impl GlyphRun {
    /// The colours (premultiplied) of the run's inks; the quads stay.
    pub fn set_palette(&self, palette: Palette) -> Result<(), UiError> {
        let wire = {
            let inner = self.layer.scene.inner.borrow();
            inner
                .atlases
                .get(self.atlas.id.key())
                .ok_or(UiError::Stale(Kind::Atlas))?
                .wire
        };
        self.layer.restyle(Content::Glyphs(GlyphStyle {
            atlas: wire,
            palette,
        }));
        Ok(())
    }

    /// The colour of a run of one ink: ink 0 is `colour` and the others draw nothing.
    pub fn set_colour(&self, colour: Rgba) -> Result<(), UiError> {
        self.set_palette(one_ink(colour))
    }
}

impl std::ops::Deref for GlyphRun {
    type Target = Layer;
    fn deref(&self) -> &Layer {
        &self.layer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneOptions;
    use crate::scene::testing::tempdir;
    use crate::scene::testing::{connect, drain};
    use crate::text::{Canvas, Weight};
    use compositor_protocol::Event;

    const DIGITS: &str = "0123456789:";
    const TYPE: Type = Type {
        px: 48,
        weight: Weight::Medium,
    };

    fn with_files() -> (Scene, std::os::unix::net::UnixStream, PathBuf) {
        let dir = tempdir();
        let (scene, comp, _events) = connect(SceneOptions {
            shm_dir: Some(dir.clone()),
            ..SceneOptions::default()
        });
        (scene, comp, dir)
    }

    /// The uploads among `requests`: (slot, generation, rect).
    fn uploads(requests: &[Request]) -> Vec<(u8, u64, AtlasRect)> {
        requests
            .iter()
            .filter_map(|r| match r {
                Request::AtlasUpload {
                    slot,
                    generation,
                    rect,
                    ..
                } => Some((*slot, *generation, *rect)),
                _ => None,
            })
            .collect()
    }

    fn uploaded(scene: &Scene, atlas: &Atlas, slot: u8, generation: u64) {
        scene.observe(&Event::AtlasUploaded {
            id: atlas.wire_id(),
            slot,
            generation,
        });
    }

    #[test]
    fn prepared_glyphs_are_handed_over_in_bands_through_two_slots_and_a_slot_is_not_rewritten_before_its_acknowledgement()
     {
        let (scene, mut comp, dir) = with_files();
        let atlas = scene.atlas(256, 256).unwrap();
        let mut fonts = Fonts::new();
        // Three sizes of glyphs: three shelves, three bands. Only two slots.
        for px in [20, 34, 48] {
            atlas
                .prepare(
                    &mut fonts,
                    Type {
                        px,
                        weight: Weight::Medium,
                    },
                    DIGITS,
                )
                .unwrap();
        }
        let bands = atlas.pending_bands();
        assert!(bands >= 3, "a band for each shelf touched: {bands}");
        assert!(!atlas.is_settled());
        assert_eq!(atlas.pump(), 2, "two free slots, two uploads");
        assert_eq!(atlas.pending_bands(), bands - 2);
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        let first = uploads(&sent);
        assert_eq!(
            first.iter().map(|(s, g, _)| (*s, *g)).collect::<Vec<_>>(),
            vec![(0, 1), (1, 2)],
            "slots 0 and 1, generations rising from 1"
        );
        // Slot 0's file holds the first band's pixels until the acknowledgement; pumping again writes nothing and stages nothing.
        let slot0 = dir.join(format!("atlas-{}-0.a8", atlas.wire_id().0));
        let before = std::fs::read(&slot0).unwrap();
        let band = first[0].2;
        let n = band.bytes() as usize;
        assert!(before[..n].iter().any(|b| *b != 0), "pixels are there");
        assert_eq!(atlas.pump(), 0);
        assert_eq!(std::fs::read(&slot0).unwrap(), before);
        // Slot 1 is acknowledged: the third band takes slot 1, not slot 0, and its generation is the next.
        uploaded(&scene, &atlas, 1, 2);
        scene.try_commit().unwrap();
        let after = uploads(&drain(&mut comp));
        assert_eq!(
            after.iter().map(|(s, g, _)| (*s, *g)).collect::<Vec<_>>(),
            vec![(1, 3)]
        );
        assert_eq!(
            std::fs::read(&slot0).unwrap(),
            before,
            "slot 0 still holds the first band"
        );
        // The compositor goes on acknowledging, always the oldest upload first; no slot is ever given a second upload while it
        // holds one, and the generations count 1, 2, 3... without a gap.
        let mut all: Vec<(u8, u64, AtlasRect)> = first.iter().chain(&after).copied().collect();
        let mut waiting: Vec<(u8, u64)> = vec![(0, 1), (1, 3)];
        while !atlas.is_settled() {
            let (slot, generation) = waiting.remove(0);
            uploaded(&scene, &atlas, slot, generation);
            scene.try_commit().unwrap();
            for (s, g, r) in uploads(&drain(&mut comp)) {
                assert!(
                    waiting.iter().all(|(held, _)| *held != s),
                    "slot {s} rewritten while it holds an upload"
                );
                waiting.push((s, g));
                all.push((s, g, r));
            }
        }
        assert!(waiting.is_empty());
        assert_eq!(all.len(), bands);
        assert_eq!(
            all.iter().map(|u| u.1).collect::<Vec<_>>(),
            (1..=bands as u64).collect::<Vec<_>>()
        );
        assert_eq!(atlas.applied_generation(), bands as u64);
        // The bands are rectangles inside the atlas, none overlapping.
        for (i, (_, _, a)) in all.iter().enumerate() {
            assert!(
                u32::from(a.x) + u32::from(a.w) <= 256 && u32::from(a.y) + u32::from(a.h) <= 256
            );
            assert!(u32::from(a.w) * u32::from(a.h) <= MAX_UPLOAD_BYTES);
            for (_, _, b) in &all[i + 1..] {
                let apart =
                    a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_band_that_starts_a_shelf_left_of_the_band_before_it_is_an_upload_of_its_own() {
        let (scene, mut comp, dir) = with_files();
        let atlas = scene.atlas(512, 256).unwrap();
        let mut fonts = Fonts::new();
        atlas.prepare(&mut fonts, TYPE, DIGITS).unwrap();
        assert_eq!(atlas.pump(), 1);
        // More of the same size continues its shelf to the right of where the first band began; a glyph too tall for that shelf
        // then starts one at the left edge, to the left of the band waiting before it.
        atlas.prepare(&mut fonts, TYPE, "abc").unwrap();
        let tall = Type {
            px: 90,
            weight: Weight::Medium,
        };
        atlas.prepare(&mut fonts, tall, "d").unwrap();
        assert_eq!(atlas.pending_bands(), 2, "two shelves, two bands");
        scene.try_commit().unwrap();
        let mut all = uploads(&drain(&mut comp));
        let mut waiting = vec![(0u8, 1u64)];
        while !atlas.is_settled() {
            let (slot, generation) = waiting.remove(0);
            uploaded(&scene, &atlas, slot, generation);
            scene.try_commit().unwrap();
            for (s, g, r) in uploads(&drain(&mut comp)) {
                waiting.push((s, g));
                all.push((s, g, r));
            }
        }
        assert_eq!(all.len(), 3);
        assert!(all[1].2.x > 0, "the continuation of the digits' shelf");
        assert_eq!(all[2].2.x, 0);
        assert!(all[2].2.y > 0, "a shelf of its own");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A short shelf with most of its width free (the sliver a warm that stopped at its bound leaves), and words in a larger type
    /// whose capitals are too tall for it and whose other letters are not.
    fn with_a_short_shelf() -> (Scene, std::os::unix::net::UnixStream, PathBuf, Atlas, Fonts) {
        let (scene, comp, dir) = with_files();
        let atlas = scene.atlas(512, 256).unwrap();
        let mut fonts = Fonts::new();
        let small = Type {
            px: 13,
            weight: Weight::Regular,
        };
        atlas.prepare(&mut fonts, small, "ab").unwrap();
        assert_eq!(atlas.pump(), 1);
        (scene, comp, dir, atlas, fonts)
    }

    const WORDS: Type = Type {
        px: 22,
        weight: Weight::SemiBold,
    };

    #[test]
    fn the_new_glyphs_of_texts_made_one_after_another_go_on_one_shelf_and_travel_as_one_band_whatever_gap_a_shorter_shelf_has()
     {
        let (scene, mut comp, dir, atlas, mut fonts) = with_a_short_shelf();
        // A screen is made with two bands in flight: a label whose glyphs wait for a third is a picture. Glyph by glyph, the
        // small letters of these words would each take the shortest shelf with a gap (the short one) and the others a shelf of
        // their own, a band on each, again for the second text: four bands, for a dozen glyphs.
        atlas.prepare(&mut fonts, WORDS, "Pasta timer").unwrap();
        atlas.prepare(&mut fonts, WORDS, "Laundry timer").unwrap();
        assert_eq!(
            atlas.pending_bands(),
            1,
            "both texts on the shelf the first one opened, one band"
        );
        assert_eq!(atlas.pump(), 1);
        scene.try_commit().unwrap();
        let sent = uploads(&drain(&mut comp));
        assert_eq!(sent.len(), 2, "the short shelf's band and the one band");
        assert!(
            sent[1].2.w > 100,
            "the glyphs of both texts: {:?}",
            sent[1].2
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_warm_goes_on_filling_the_gaps_of_the_shelves_it_has_glyph_by_glyph() {
        let (_scene, _comp, dir, atlas, mut fonts) = with_a_short_shelf();
        // The same words as the page's warm prepares them (`prepare_within`): the densest packing, so that the types of a screen
        // take as few of the page's rows as they can: the small letters go in the short shelf's gap, the others on a shelf of
        // their own, a band on each. A warm is made with the page to itself, and the bands go over in the time it takes.
        atlas
            .prepare_within(&mut fonts, WORDS, "Pasta timer", 256)
            .unwrap();
        assert_eq!(atlas.pending_bands(), 2, "the gap and the new shelf");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_run_is_shown_whole_after_the_uploads_it_needs_and_is_refused_before_they_are_handed_over()
    {
        let (scene, mut comp, dir) = with_files();
        let atlas = scene.atlas(512, 128).unwrap();
        let mut fonts = Fonts::new();
        atlas.prepare(&mut fonts, TYPE, DIGITS).unwrap();
        let mut run = scene
            .glyph_run(
                &atlas,
                200,
                60,
                1,
                one_ink(Rgba(255, 255, 255, 255)),
                Props::at(10.0, 10.0),
            )
            .unwrap();
        let mut placed = Vec::new();
        atlas
            .layout("12:34", TYPE, (0.0, 48.0), 0, &mut placed)
            .unwrap();
        assert_eq!(placed.len(), 5);
        assert!(matches!(
            run.set_glyphs(&placed),
            Err(UiError::GlyphNotUploaded)
        ));
        atlas.pump();
        run.set_glyphs(&placed).unwrap();
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        let up = uploads(&sent);
        let swap = sent
            .iter()
            .find_map(|r| match r {
                Request::SetGlyphs { after, glyphs, .. } => Some((*after, glyphs.clone())),
                _ => None,
            })
            .expect("the swap went out");
        assert_eq!(swap.1.len(), 5);
        assert_eq!(
            swap.0,
            up.iter().map(|u| u.1).max().unwrap(),
            "waits for the newest upload it uses"
        );
        // Each quad is the glyph's own rectangle of the atlas.
        let mut by_rect = swap
            .1
            .iter()
            .map(|g| (g.sx, g.sy, g.w, g.h))
            .collect::<Vec<_>>();
        by_rect.sort_unstable();
        by_rect.dedup();
        assert_eq!(
            by_rect.len(),
            5,
            "1, 2, :, 3 and 4 are five different glyphs of the atlas"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_digit_swap_replaces_the_staged_swap_and_needs_no_upload() {
        let (scene, mut comp, dir) = with_files();
        let atlas = scene.atlas(256, 128).unwrap();
        let mut fonts = Fonts::new();
        atlas.prepare(&mut fonts, TYPE, DIGITS).unwrap();
        atlas.pump();
        let mut run = scene
            .glyph_run(
                &atlas,
                200,
                60,
                1,
                one_ink(Rgba(255, 255, 255, 255)),
                Props::at(0.0, 0.0),
            )
            .unwrap();
        let mut placed = Vec::new();
        atlas
            .layout("00:00", TYPE, (0.0, 48.0), 0, &mut placed)
            .unwrap();
        run.set_glyphs(&placed).unwrap();
        scene.try_commit().unwrap();
        drain(&mut comp);
        for (i, now) in ["00:01", "00:02", "00:03"].iter().enumerate() {
            placed.clear();
            atlas
                .layout(now, TYPE, (0.0, 48.0), 0, &mut placed)
                .unwrap();
            run.set_glyphs(&placed).unwrap();
            if i < 2 {
                // Swapped again before a commit: the staged swap is replaced, not queued behind.
                placed.clear();
                atlas
                    .layout(now, TYPE, (0.0, 48.0), 0, &mut placed)
                    .unwrap();
                run.set_glyphs(&placed).unwrap();
            }
        }
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        assert_eq!(
            sent.iter()
                .filter(|r| matches!(r, Request::SetGlyphs { .. }))
                .count(),
            1,
            "{sent:?}"
        );
        assert!(uploads(&sent).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_quads_of_a_run_composite_to_the_pixels_fonts_draw() {
        // The atlas route and the CPU route must agree to the pixel: draw "12:34" both ways.
        let (scene, mut comp, dir) = with_files();
        let atlas = scene.atlas(256, 128).unwrap();
        let mut fonts = Fonts::new();
        atlas.prepare(&mut fonts, TYPE, DIGITS).unwrap();
        atlas.pump();
        let mut run = scene
            .glyph_run(
                &atlas,
                160,
                64,
                1,
                one_ink(Rgba(255, 255, 255, 255)),
                Props::at(0.0, 0.0),
            )
            .unwrap();
        let mut placed = Vec::new();
        let end = atlas
            .layout("12:34", TYPE, (3.5, 50.0), 0, &mut placed)
            .unwrap();
        run.set_glyphs(&placed).unwrap();
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        // The texture as the uploads left it.
        let mut texture = vec![0u8; 256 * 128];
        for (slot, _, rect) in uploads(&sent) {
            let file =
                std::fs::read(dir.join(format!("atlas-{}-{slot}.a8", atlas.wire_id().0))).unwrap();
            for row in 0..rect.h as usize {
                let src = &file[row * rect.w as usize..][..rect.w as usize];
                texture[(rect.y as usize + row) * 256 + rect.x as usize..][..rect.w as usize]
                    .copy_from_slice(src);
            }
        }
        let quads = sent
            .iter()
            .find_map(|r| match r {
                Request::SetGlyphs { glyphs, .. } => Some(glyphs.clone()),
                _ => None,
            })
            .unwrap();
        let mut by_quads = vec![0u8; 160 * 64];
        for q in &quads {
            for row in 0..q.h as usize {
                for col in 0..q.w as usize {
                    let (x, y) = (q.x as usize + col, q.y as usize + row);
                    by_quads[y * 160 + x] =
                        texture[(q.sy as usize + row) * 256 + q.sx as usize + col];
                }
            }
        }
        let mut cpu = vec![0u8; 160 * 64 * 4];
        let pen = fonts.draw(
            &mut Canvas {
                pixels: &mut cpu,
                width: 160,
            },
            "12:34",
            TYPE,
            (3.5, 50.0),
            Rgba(255, 255, 255, 255),
        );
        let by_cpu: Vec<u8> = cpu.chunks_exact(4).map(|p| p[3]).collect();
        assert_eq!(by_quads, by_cpu);
        assert!(
            by_cpu.iter().any(|a| *a > 0),
            "the comparison is not of two empty pictures"
        );
        assert_eq!(end, pen, "and the pen ends where the CPU pen does");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn runs_go_before_their_atlas_and_the_atlas_waits_for_them_and_its_files_go_after_the_commit_is_shown()
     {
        let (scene, mut comp, dir) = with_files();
        let atlas = scene.atlas(128, 64).unwrap();
        let mut fonts = Fonts::new();
        atlas.prepare(&mut fonts, TYPE, "8").unwrap();
        atlas.pump();
        let run = scene
            .glyph_run(
                &atlas,
                40,
                40,
                1,
                one_ink(Rgba(255, 255, 255, 255)),
                Props::at(0.0, 0.0),
            )
            .unwrap();
        let (wire, layer) = (atlas.wire_id(), run.id());
        let slot_file = dir.join(format!("atlas-{}-0.a8", wire.0));
        scene.try_commit().unwrap();
        drain(&mut comp);
        // The atlas handle goes first: nothing is deleted, the run still draws from it.
        drop(atlas);
        assert!(!scene.has_pending(), "the atlas waits for its run");
        drop(run);
        let tag = scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        let at = |f: fn(&Request) -> bool| sent.iter().position(f).expect("sent");
        assert_eq!(
            sent[at(|r| matches!(r, Request::Delete { .. }))],
            Request::Delete { id: layer }
        );
        assert!(
            at(|r| matches!(r, Request::Delete { .. }))
                < at(|r| matches!(r, Request::AtlasDelete { .. }))
        );
        assert!(sent.contains(&Request::AtlasDelete { id: wire }));
        assert!(slot_file.exists(), "the compositor may still read it");
        scene.shown(tag.get());
        assert!(!slot_file.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ceilings_are_errors_at_creation() {
        let (scene, _comp, dir) = with_files();
        assert!(matches!(scene.atlas(0, 10), Err(UiError::Invalid(_))));
        assert!(matches!(
            scene.atlas(2049, 10),
            Err(UiError::Limit(Limit::AtlasSide))
        ));
        let a = scene.atlas(1024, 1024).unwrap();
        let b = scene.atlas(1024, 1024).unwrap();
        let c = scene.atlas(1024, 1024).unwrap();
        let d = scene.atlas(1024, 1024).unwrap();
        assert!(matches!(
            scene.atlas(8, 8),
            Err(UiError::Limit(Limit::Atlases))
        ));
        drop(d);
        assert!(matches!(
            scene.atlas(1024, 1025),
            Err(UiError::Limit(Limit::AtlasBytes))
        ));
        let _e = scene.atlas(1024, 1024).unwrap();
        let _held = (a, b, c);
        assert!(
            matches!(scene.atlas(8, 8), Err(UiError::Limit(Limit::Atlases))),
            "four atlases again"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_atlas_that_is_full_says_so_and_what_fitted_is_still_handed_over() {
        let (scene, _comp, dir) = with_files();
        let atlas = scene.atlas(64, 64).unwrap();
        let mut fonts = Fonts::new();
        let big = Type {
            px: 40,
            weight: Weight::Regular,
        };
        let err = atlas
            .prepare(&mut fonts, big, "ABCDEFGHIJKLMNOP")
            .unwrap_err();
        assert!(matches!(err, UiError::AtlasFull), "{err}");
        let fitted = atlas.pending_bands();
        assert!(
            fitted >= 1,
            "the glyphs that fitted before it filled up are queued"
        );
        assert!(atlas.glyph(big, 'A').is_some());
        assert!(
            atlas.glyph(big, 'P').is_none(),
            "and the ones after are not there"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn too_many_quads_and_glyphs_of_another_atlas_are_refused() {
        let (scene, _comp, dir) = with_files();
        let atlas = scene.atlas(256, 128).unwrap();
        let other = scene.atlas(256, 128).unwrap();
        let mut fonts = Fonts::new();
        atlas.prepare(&mut fonts, TYPE, "0").unwrap();
        atlas.pump();
        let mut run = scene
            .glyph_run(
                &atlas,
                100,
                60,
                1,
                one_ink(Rgba(255, 255, 255, 255)),
                Props::at(0.0, 0.0),
            )
            .unwrap();
        let g = atlas.glyph(TYPE, '0').unwrap();
        let at = |x: f32| PlacedGlyph {
            glyph: g,
            x,
            y: 0.0,
            ink: 0,
            crop: 0,
        };
        let many = vec![at(0.0); MAX_RUN_GLYPHS + 1];
        assert!(matches!(
            run.set_glyphs(&many),
            Err(UiError::Limit(Limit::RunGlyphs))
        ));
        let ok = vec![at(0.0); MAX_RUN_GLYPHS];
        run.set_glyphs(&ok).unwrap();
        let bad = [PlacedGlyph {
            glyph: GlyphId {
                atlas: atlas.wire_id(),
                index: 99,
            },
            ..at(0.0)
        }];
        assert!(matches!(run.set_glyphs(&bad), Err(UiError::Invalid(_))));
        let nan = [at(f32::NAN)];
        assert!(matches!(run.set_glyphs(&nan), Err(UiError::Invalid(_))));
        let dark = [PlacedGlyph {
            ink: INKS as u8,
            ..at(0.0)
        }];
        assert!(matches!(run.set_glyphs(&dark), Err(UiError::Invalid(_))));
        assert!(matches!(
            scene.glyph_run(
                &other,
                0,
                10,
                0,
                one_ink(Rgba(0, 0, 0, 0)),
                Props::at(0.0, 0.0)
            ),
            Err(UiError::Invalid(_))
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_glyph_of_another_atlas_is_refused_even_when_its_index_is_in_this_one() {
        let (scene, mut comp, dir) = with_files();
        let (a, b) = (
            scene.atlas(256, 128).unwrap(),
            scene.atlas(256, 128).unwrap(),
        );
        let mut fonts = Fonts::new();
        // The same digits in both: the '0' of each has the same slot of its table, and its own atlas's name.
        for atlas in [&a, &b] {
            atlas.prepare(&mut fonts, TYPE, DIGITS).unwrap();
            atlas.pump();
        }
        let (of_a, of_b) = (a.glyph(TYPE, '0').unwrap(), b.glyph(TYPE, '0').unwrap());
        assert_eq!(of_a.index, of_b.index, "a slot that exists in both tables");
        assert_ne!(of_a, of_b);
        let mut run = run_of(&scene, &b, one_ink(Rgba(255, 255, 255, 255)));
        let at = |glyph| {
            [PlacedGlyph {
                glyph,
                x: 0.0,
                y: 0.0,
                ink: 0,
                crop: 0,
            }]
        };
        run.set_glyphs(&at(of_b)).unwrap();
        scene.try_commit().unwrap();
        drain(&mut comp);
        assert!(matches!(
            run.set_glyphs(&at(of_a)),
            Err(UiError::Invalid(_))
        ));
        assert!(matches!(
            run.set_glyphs_when_ready(&at(of_a)),
            Err(UiError::Invalid(_))
        ));
        // Nothing was staged for what was refused, and only an atlas answers for its own glyphs.
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        assert!(
            !sent.iter().any(|r| matches!(r, Request::SetGlyphs { .. })),
            "{sent:?}"
        );
        assert!(!run.is_waiting());
        assert!(a.advance(of_a).is_some() && a.advance(of_b).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_view_keeps_nothing_alive_and_asks_nothing_of_an_atlas_that_has_gone() {
        let (scene, mut comp, dir) = with_files();
        let atlas = scene.atlas(128, 64).unwrap();
        let mut fonts = Fonts::new();
        atlas.prepare(&mut fonts, TYPE, "8").unwrap();
        let view = atlas.view();
        assert_eq!(view.id(), atlas.id());
        assert_eq!(view.wire_id(), atlas.wire_id());
        assert_eq!((view.size(), view.widest()), ((128, 64), 126));
        assert_eq!(view.glyph(TYPE, '8'), atlas.glyph(TYPE, '8'));
        let wire = atlas.wire_id();
        // The handle goes, no run draws from it: the atlas is deleted, a view of it or not.
        drop(atlas);
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        assert!(
            sent.contains(&Request::AtlasDelete { id: wire }),
            "{sent:?}"
        );
        // What the view is asked now is answered with nothing, not with a panic.
        assert!(view.glyph(TYPE, '8').is_none());
        assert!(view.sprite(Sprite::Rule).is_none());
        assert!(view.is_settled());
        assert_eq!(
            (view.used_rows(), view.pending_bands(), view.pump()),
            (0, 0, 0)
        );
        assert!(matches!(
            view.prepare(&mut fonts, TYPE, "9"),
            Err(UiError::Stale(_))
        ));
        assert!(matches!(
            view.layout("8", TYPE, (0.0, 0.0), 0, &mut Vec::new()),
            Err(UiError::Stale(_))
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_run_holds_the_atlas_it_was_made_on_and_names_it_while_the_handle_is_gone() {
        let (scene, mut comp, dir) = with_files();
        let atlas = scene.atlas(128, 64).unwrap();
        let other = scene.atlas(128, 64).unwrap();
        let mut fonts = Fonts::new();
        let run = run_of(&scene, &atlas, one_ink(Rgba(255, 255, 255, 255)));
        assert_eq!(run.size(), (300, 80));
        let (page, wire) = (run.atlas(), atlas.wire_id());
        assert_eq!(page.id(), atlas.id());
        assert_ne!(page.id(), other.id());
        scene.try_commit().unwrap();
        drain(&mut comp);
        // The handle goes: the run's page stays, and what the run is given next is made ready on it.
        drop(atlas);
        page.prepare(&mut fonts, TYPE, "7").unwrap();
        assert!(page.glyph(TYPE, '7').is_some());
        scene.try_commit().unwrap();
        assert!(
            !drain(&mut comp).contains(&Request::AtlasDelete { id: wire }),
            "the run still draws from it"
        );
        drop(run);
        scene.try_commit().unwrap();
        assert!(drain(&mut comp).contains(&Request::AtlasDelete { id: wire }));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The texture as the uploads of `sent` left it (`width` x `height`), read from the staging files.
    fn texture_of(
        sent: &[Request],
        dir: &std::path::Path,
        atlas: &Atlas,
        (width, height): (usize, usize),
    ) -> Vec<u8> {
        let mut texture = vec![0u8; width * height];
        for (slot, _, rect) in uploads(sent) {
            let file =
                std::fs::read(dir.join(format!("atlas-{}-{slot}.a8", atlas.wire_id().0))).unwrap();
            for row in 0..rect.h as usize {
                let src = &file[row * rect.w as usize..][..rect.w as usize];
                texture[(rect.y as usize + row) * width + rect.x as usize..][..rect.w as usize]
                    .copy_from_slice(src);
            }
        }
        texture
    }

    fn run_of(scene: &Scene, atlas: &Atlas, palette: Palette) -> GlyphRun {
        scene
            .glyph_run(atlas, 300, 80, 1, palette, Props::at(0.0, 0.0))
            .unwrap()
    }

    fn the_list(sent: &[Request]) -> Vec<Glyph> {
        sent.iter()
            .filter_map(|r| match r {
                Request::SetGlyphs { glyphs, .. } => Some(glyphs.clone()),
                _ => None,
            })
            .next_back()
            .expect("a list was sent")
    }

    #[test]
    fn a_run_is_sent_in_ink_order_whatever_order_its_glyphs_come_in_and_keeps_the_order_within_an_ink()
     {
        let (scene, mut comp, dir) = with_files();
        let atlas = scene.atlas(256, 128).unwrap();
        let mut fonts = Fonts::new();
        atlas.prepare(&mut fonts, TYPE, DIGITS).unwrap();
        atlas.pump();
        let palette = [
            Rgba(255, 255, 255, 255),
            Rgba(160, 160, 170, 255),
            Rgba(0, 0, 0, 0),
            Rgba(200, 40, 40, 255),
        ];
        let mut run = run_of(&scene, &atlas, palette);
        // Five glyphs at x = 1..5 in the inks 3, 0, 1, 0, 3: the quads must go out as 0, 0, 1, 3, 3 with 2 and 4 first, then 3,
        // then 1 and 5.
        let one = |c: char, x: f32, ink: u8| PlacedGlyph {
            glyph: atlas.glyph(TYPE, c).unwrap(),
            x,
            y: 0.0,
            ink,
            crop: 0,
        };
        run.set_glyphs(&[
            one('1', 1.0, 3),
            one('2', 2.0, 0),
            one('3', 3.0, 1),
            one('4', 4.0, 0),
            one('5', 5.0, 3),
        ])
        .unwrap();
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        let quads = the_list(&sent);
        assert_eq!(
            quads.iter().map(|q| (q.ink, q.x)).collect::<Vec<_>>(),
            // A quad starts a texel left of its glyph's bitmap.
            vec![(0, 1.0), (0, 3.0), (1, 2.0), (3, 0.0), (3, 4.0)]
        );
        // The creation of the layer carried the palette.
        let made = sent
            .iter()
            .find_map(|r| match r {
                Request::Create {
                    content: Content::Glyphs(style),
                    ..
                } => Some(*style),
                _ => None,
            })
            .expect("the run was created");
        assert_eq!(made.palette, palette);
        // A palette of its own is one restyle; the quads stay.
        let other = [Rgba(1, 2, 3, 255); INKS];
        run.set_palette(other).unwrap();
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        assert!(
            sent.iter().any(|r| matches!(
                r,
                Request::Restyle { content: Content::Glyphs(style), .. } if style.palette == other
            )),
            "{sent:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// An atlas of three shelves (three sizes) whose first two bands are handed over, a third waiting for a slot; and the run's
    /// text, which is in the third.
    fn busy_atlas() -> (Scene, std::os::unix::net::UnixStream, PathBuf, Atlas, Type) {
        let (scene, comp, dir) = with_files();
        let atlas = scene.atlas(1024, 256).unwrap();
        let mut fonts = Fonts::new();
        let last = Type {
            px: 48,
            weight: Weight::Medium,
        };
        for px in [20, 34, 48] {
            let ty = Type {
                px,
                weight: Weight::Medium,
            };
            // No colon: it is shorter than the digits and would go on a shelf of a smaller size.
            atlas.prepare(&mut fonts, ty, "0123456789").unwrap();
        }
        assert_eq!(atlas.pump(), 2);
        assert_eq!(atlas.pending_bands(), 1, "the glyphs of the last type wait");
        (scene, comp, dir, atlas, last)
    }

    #[test]
    fn a_list_that_names_a_glyph_not_handed_over_yet_waits_and_is_sent_when_the_last_of_its_bands_is()
     {
        let (scene, mut comp, dir, atlas, ty) = busy_atlas();
        let mut run = run_of(&scene, &atlas, one_ink(Rgba(255, 255, 255, 255)));
        scene.try_commit().unwrap();
        drain(&mut comp);
        let mut placed = Vec::new();
        atlas
            .layout("1234", ty, (0.0, 48.0), 0, &mut placed)
            .unwrap();
        // The strict swap refuses it and changes nothing...
        assert!(matches!(
            run.set_glyphs(&placed),
            Err(UiError::GlyphNotUploaded)
        ));
        assert!(!run.is_waiting());
        // ...the swap that waits takes it, and sends nothing yet.
        run.set_glyphs_when_ready(&placed).unwrap();
        assert!(run.is_waiting());
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        assert!(
            !sent.iter().any(|r| matches!(r, Request::SetGlyphs { .. })),
            "{sent:?}"
        );
        // A slot is acknowledged: the last band is handed over, and the list goes out with the generation it needs.
        uploaded(&scene, &atlas, 0, 1);
        assert!(!run.is_waiting());
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        let up = uploads(&sent);
        assert_eq!(up.len(), 1, "the band that was waiting");
        let after = sent
            .iter()
            .find_map(|r| match r {
                Request::SetGlyphs { after, .. } => Some(*after),
                _ => None,
            })
            .expect("the list was sent");
        assert_eq!(after, up[0].1);
        assert_eq!(the_list(&sent).len(), 4);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_newer_list_replaces_one_that_waits_and_a_run_that_goes_takes_its_waiting_list_with_it() {
        let (scene, mut comp, dir, atlas, ty) = busy_atlas();
        let mut run = run_of(&scene, &atlas, one_ink(Rgba(255, 255, 255, 255)));
        let mut placed = Vec::new();
        atlas
            .layout("1234", ty, (0.0, 48.0), 0, &mut placed)
            .unwrap();
        run.set_glyphs_when_ready(&placed).unwrap();
        placed.clear();
        atlas.layout("43", ty, (0.0, 48.0), 0, &mut placed).unwrap();
        run.set_glyphs_when_ready(&placed).unwrap();
        scene.try_commit().unwrap();
        drain(&mut comp);
        uploaded(&scene, &atlas, 0, 1);
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        assert_eq!(
            sent.iter()
                .filter(|r| matches!(r, Request::SetGlyphs { .. }))
                .count(),
            1
        );
        assert_eq!(the_list(&sent).len(), 2, "the newer list, not the first");
        // A list that waits for a run that is deleted is never sent: the compositor would refuse a list for a layer it has not.
        let mut other = run_of(&scene, &atlas, one_ink(Rgba(255, 255, 255, 255)));
        let bigger = Type {
            px: 60,
            weight: Weight::Medium,
        };
        let mut fonts = Fonts::new();
        atlas.prepare(&mut fonts, bigger, "7").unwrap();
        atlas.pump();
        uploaded(&scene, &atlas, 1, 2);
        uploaded(&scene, &atlas, 0, 3);
        scene.try_commit().unwrap();
        drain(&mut comp);
        atlas.prepare(&mut fonts, bigger, "8").unwrap();
        placed.clear();
        atlas
            .layout("8", bigger, (0.0, 48.0), 0, &mut placed)
            .unwrap();
        // Both slots are in flight again, so the band of "8" waits behind them.
        atlas.pump();
        atlas.prepare(&mut fonts, bigger, "9").unwrap();
        atlas.prepare(&mut fonts, bigger, "0").unwrap();
        assert_eq!(atlas.pump(), 0);
        placed.clear();
        atlas
            .layout("90", bigger, (0.0, 48.0), 0, &mut placed)
            .unwrap();
        other.set_glyphs_when_ready(&placed).unwrap();
        assert!(other.is_waiting());
        drop(other);
        scene.try_commit().unwrap();
        drain(&mut comp);
        uploaded(&scene, &atlas, 1, 4);
        uploaded(&scene, &atlas, 0, 5);
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        assert!(
            !sent.iter().any(|r| matches!(r, Request::SetGlyphs { .. })),
            "{sent:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn glyphs_prepared_one_text_after_another_travel_as_one_band_and_every_one_has_its_pixels() {
        let (scene, mut comp, dir) = with_files();
        let atlas = scene.atlas(512, 128).unwrap();
        let mut fonts = Fonts::new();
        // Three texts of one type: their glyphs are neighbours on one shelf.
        for text in ["012", "345", "6789"] {
            atlas.prepare(&mut fonts, TYPE, text).unwrap();
        }
        assert_eq!(atlas.pending_bands(), 1);
        atlas.pump();
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        assert_eq!(uploads(&sent).len(), 1);
        let texture = texture_of(&sent, &dir, &atlas, (512, 128));
        let inner = scene.inner.borrow();
        let entry = inner.atlases.get(atlas.id.key()).unwrap();
        for c in "0123456789".chars() {
            let id = atlas.glyph(TYPE, c).unwrap();
            let info = &entry.glyphs[id.index as usize];
            let (metrics, coverage) = fonts.raster(TYPE, c);
            assert_eq!(info.rect.w as usize, metrics.width);
            for row in 0..metrics.height {
                let at = (info.rect.y as usize + row) * 512 + info.rect.x as usize;
                assert_eq!(
                    &texture[at..][..metrics.width],
                    &coverage[row * metrics.width..][..metrics.width],
                    "{c}"
                );
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_mark_is_made_once_kept_like_a_glyph_and_drawn_where_it_is_laid_out_cut_to_the_width_asked()
    {
        let (scene, mut comp, dir) = with_files();
        let atlas = scene.atlas(256, 128).unwrap();
        let mut made = 0;
        let mut make = |width: u32| {
            made += 1;
            (width, 1, vec![255u8; width as usize])
        };
        atlas.prepare_sprite(Sprite::Rule, &mut make).unwrap();
        atlas.prepare_sprite(Sprite::Rule, &mut make).unwrap();
        assert_eq!(made, 1, "a mark the atlas has is not made again");
        let disc = |d: u32| {
            move |_: u32| {
                let mut px = vec![0u8; (d * d) as usize];
                for (i, p) in px.iter_mut().enumerate() {
                    let (x, y) = ((i as u32 % d) as f32 + 0.5, (i as u32 / d) as f32 + 0.5);
                    let r = d as f32 / 2.0;
                    if (x - r).powi(2) + (y - r).powi(2) <= r * r {
                        *p = 255;
                    }
                }
                (d, d, px)
            }
        };
        atlas.prepare_sprite(Sprite::Disc(10), disc(10)).unwrap();
        atlas.pump();
        let mut run = run_of(&scene, &atlas, one_ink(Rgba(255, 255, 255, 255)));
        let mut placed = Vec::new();
        atlas
            .layout_sprite(Sprite::Rule, (4, 70), 120, 0, &mut placed)
            .unwrap();
        atlas
            .layout_sprite(Sprite::Disc(10), (30, 5), 0, 1, &mut placed)
            .unwrap();
        run.set_glyphs(&placed).unwrap();
        scene.try_commit().unwrap();
        let sent = drain(&mut comp);
        let quads = the_list(&sent);
        assert_eq!(quads.len(), 2);
        // Each quad is its bitmap grown by a texel on every side; the hairline, cut at 120 columns, ends where it was cut.
        assert_eq!((quads[0].ink, quads[0].w, quads[0].h), (0, 121, 3));
        assert_eq!((quads[0].x, quads[0].y), (3.0, 69.0));
        assert_eq!((quads[1].ink, quads[1].w, quads[1].h), (1, 12, 12));
        assert_eq!((quads[1].x, quads[1].y), (29.0, 4.0));
        // The pixels are where the quads say: the bitmaps solid or round, the texel round them zero.
        let texture = texture_of(&sent, &dir, &atlas, (256, 128));
        let at = |q: &Glyph, col: usize, row: usize| {
            texture[(q.sy as usize + row) * 256 + q.sx as usize + col]
        };
        let hairline = &quads[0];
        for col in 0..hairline.w as usize {
            assert_eq!(at(hairline, col, 0), 0, "above the hairline ({col})");
            assert_eq!(at(hairline, col, 2), 0, "below the hairline ({col})");
            assert_eq!(
                at(hairline, col, 1),
                if col == 0 { 0 } else { 255 },
                "the hairline is solid after its margin ({col})"
            );
        }
        let d = &quads[1];
        for edge in 0..12 {
            for (col, row) in [(edge, 0), (edge, 11), (0, edge), (11, edge)] {
                assert_eq!(at(d, col, row), 0, "the disc's margin ({col}, {row})");
            }
        }
        let (centre, corner) = (at(d, 6, 6), at(d, 1, 1));
        assert_eq!((centre, corner), (255, 0));
        assert!(matches!(
            atlas.layout_sprite(Sprite::Disc(11), (0, 0), 0, 0, &mut placed),
            Err(UiError::Invalid(_))
        ));
        std::fs::remove_dir_all(&dir).ok();
    }
}
