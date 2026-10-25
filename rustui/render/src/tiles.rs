//! Damage as a bitmap of the panel's 16 px tiles: what a frame changes, what each page of a ring still owes, and what can be
//! seen of each layer.
//!
//! LumaShow's renderer keeps these accounts as up to six rectangles that merge by waste (`damage::Region`), which is right for a
//! page of cards on a 960 x 480 panel and wrong for a 1080p home: a wallpaper that steps shows in the margins and gutters between
//! the cards, its one bounding box is the whole panel, and a frame that changes only the margins is drawn in full. A bitmap keeps
//! exactly the tiles that differ whatever their shape, a union is an OR, and the rectangles a frame is drawn as are cut from it
//! only at the end ([`TileSet::rects`]), where the draw cost model chooses how many passes to make of them.
//!
//! The bitmap is in tiles of [`TILE`] px aligned to the panel's origin, the grid the GPU draws in (a tile drawn in part is loaded
//! from memory first and written back whole) and the grid the compositor's own damage ring uses (`lumatv-compositor`'s `damage.rs`),
//! so a set converts to the compositor's by its rectangles and back.

use crate::backdrop::Area;
use crate::damage::Region;

/// Tile edge in panel pixels.
pub const TILE: i32 = 16;

/// A set of tiles of one panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileSet {
    width: i32,
    height: i32,
    cols: usize,
    rows: usize,
    /// `u64`s a row of tiles takes.
    words: usize,
    bits: Vec<u64>,
}

impl TileSet {
    /// No tile of a `width` x `height` panel.
    pub fn new(width: u32, height: u32) -> Self {
        let cols = (width as usize).div_ceil(TILE as usize);
        let rows = (height as usize).div_ceil(TILE as usize);
        let words = cols.div_ceil(64).max(1);
        Self { width: width as i32, height: height as i32, cols, rows, words, bits: vec![0; rows * words] }
    }

    /// Every tile.
    pub fn full(width: u32, height: u32) -> Self {
        let mut set = Self::new(width, height);
        set.fill();
        set
    }

    /// The panel's own size in pixels.
    pub fn panel(&self) -> (u32, u32) {
        (self.width as u32, self.height as u32)
    }

    pub fn panel_area(&self) -> Area {
        [0, 0, self.width, self.height]
    }

    pub fn clear(&mut self) {
        self.bits.fill(0);
    }

    pub fn fill(&mut self) {
        for row in 0..self.rows {
            let words = &mut self.bits[row * self.words..(row + 1) * self.words];
            words.fill(0);
            set_range(words, 0, self.cols);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|&w| w == 0)
    }

    /// The tiles in the set.
    pub fn count(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// The panel pixels the tiles hold: the edge tiles of a panel that is not a multiple of 16 are clipped to it.
    pub fn pixels(&self) -> u64 {
        let mut total = 0u64;
        let last_col_px = (self.width - (self.cols as i32 - 1) * TILE) as u64;
        for row in 0..self.rows {
            let height = (self.height - row as i32 * TILE).min(TILE) as u64;
            let words = &self.bits[row * self.words..(row + 1) * self.words];
            let tiles: u64 = words.iter().map(|w| u64::from(w.count_ones())).sum();
            let mut px = tiles * TILE as u64;
            if bit(words, self.cols - 1) {
                px -= TILE as u64 - last_col_px;
            }
            total += px * height;
        }
        total
    }

    /// Every tile `area` touches.
    pub fn add(&mut self, area: Area) {
        let [x0, y0, x1, y1] = self.clipped(area);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let (c0, c1) = ((x0 / TILE) as usize, ((x1 + TILE - 1) / TILE) as usize);
        let (r0, r1) = ((y0 / TILE) as usize, ((y1 + TILE - 1) / TILE) as usize);
        for row in r0..r1 {
            set_range(&mut self.bits[row * self.words..(row + 1) * self.words], c0, c1);
        }
    }

    /// Every tile that lies wholly inside `area` (the part of an edge tile past the panel does not count against it): what an
    /// opaque layer covers completely, whatever is under it.
    pub fn add_inside(&mut self, area: Area) {
        let [x0, y0, x1, y1] = self.clipped(area);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let c0 = ((x0 + TILE - 1) / TILE) as usize;
        let c1 = if x1 >= self.width { self.cols } else { (x1 / TILE) as usize };
        let r0 = ((y0 + TILE - 1) / TILE) as usize;
        let r1 = if y1 >= self.height { self.rows } else { (y1 / TILE) as usize };
        for row in r0..r1 {
            set_range(&mut self.bits[row * self.words..(row + 1) * self.words], c0, c1);
        }
    }

    fn clipped(&self, area: Area) -> Area {
        [area[0].max(0), area[1].max(0), area[2].min(self.width), area[3].min(self.height)]
    }

    /// Everything in `other` too.
    pub fn union(&mut self, other: &Self) {
        debug_assert_eq!(self.bits.len(), other.bits.len());
        for (a, b) in self.bits.iter_mut().zip(&other.bits) {
            *a |= *b;
        }
    }

    /// The tiles of `other` that `area` touches, too.
    pub fn union_within(&mut self, other: &Self, area: Area) {
        let [x0, y0, x1, y1] = self.clipped(area);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let (c0, c1) = ((x0 / TILE) as usize, ((x1 + TILE - 1) / TILE) as usize);
        let (r0, r1) = ((y0 / TILE) as usize, ((y1 + TILE - 1) / TILE) as usize);
        let mut mask = vec![0u64; self.words];
        set_range(&mut mask, c0, c1);
        for row in r0..r1 {
            let at = row * self.words;
            for w in 0..self.words {
                self.bits[at + w] |= other.bits[at + w] & mask[w];
            }
        }
    }

    /// Without the tiles of `other`.
    pub fn subtract(&mut self, other: &Self) {
        debug_assert_eq!(self.bits.len(), other.bits.len());
        for (a, b) in self.bits.iter_mut().zip(&other.bits) {
            *a &= !*b;
        }
    }

    /// Whether any tile `area` touches is in the set.
    pub fn touches(&self, area: Area) -> bool {
        let [x0, y0, x1, y1] = self.clipped(area);
        if x1 <= x0 || y1 <= y0 {
            return false;
        }
        let (c0, c1) = ((x0 / TILE) as usize, ((x1 + TILE - 1) / TILE) as usize);
        let (r0, r1) = ((y0 / TILE) as usize, ((y1 + TILE - 1) / TILE) as usize);
        (r0..r1).any(|row| any_in_range(&self.bits[row * self.words..(row + 1) * self.words], c0, c1))
    }

    /// The smallest rectangle holding every tile of the set, in panel pixels.
    pub fn bounds(&self) -> Option<Area> {
        let mut area = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
        for row in 0..self.rows {
            let words = &self.bits[row * self.words..(row + 1) * self.words];
            let Some(first) = words.iter().position(|&w| w != 0) else { continue };
            let last = words.iter().rposition(|&w| w != 0).unwrap_or(first);
            let c0 = first * 64 + words[first].trailing_zeros() as usize;
            let c1 = last * 64 + 64 - words[last].leading_zeros() as usize;
            area[0] = area[0].min(c0 as i32 * TILE);
            area[2] = area[2].max((c1 as i32 * TILE).min(self.width));
            area[1] = area[1].min(row as i32 * TILE);
            area[3] = area[3].max(((row + 1) as i32 * TILE).min(self.height));
        }
        (area[2] > area[0]).then_some(area)
    }

    /// The tiles as rectangles of panel pixels, exactly: each row's runs of tiles, where the same run on the row below goes on
    /// the rectangle above it. Cleared first.
    pub fn rects(&self, out: &mut Vec<Area>) {
        out.clear();
        // (first tile, past the last, index in `out`) of the runs the row above ended with.
        let mut open: Vec<(usize, usize, usize)> = Vec::new();
        let mut next: Vec<(usize, usize, usize)> = Vec::new();
        for row in 0..self.rows {
            next.clear();
            let words = &self.bits[row * self.words..(row + 1) * self.words];
            let mut col = 0;
            while col < self.cols {
                let Some(start) = next_set(words, col, self.cols) else { break };
                let end = next_clear(words, start, self.cols);
                let bottom = ((row + 1) as i32 * TILE).min(self.height);
                let index = match open.iter().find(|&&(a, b, _)| a == start && b == end) {
                    Some(&(_, _, index)) => {
                        out[index][3] = bottom;
                        index
                    }
                    None => {
                        out.push([start as i32 * TILE, row as i32 * TILE, (end as i32 * TILE).min(self.width), bottom]);
                        out.len() - 1
                    }
                };
                next.push((start, end, index));
                col = end;
            }
            std::mem::swap(&mut open, &mut next);
        }
    }

    /// The set as a [`Region`] of the rectangles [`TileSet::rects`] cuts it into: the region merges two that are cheaper drawn as
    /// one cover and keeps at most [`crate::damage::MAX_RECTS`], so it can hold more than the set.
    pub fn region(&self) -> Region {
        let mut rects = Vec::new();
        self.rects(&mut rects);
        let mut region = Region::NONE;
        for rect in rects {
            region.add(rect);
        }
        region
    }
}

fn bit(words: &[u64], col: usize) -> bool {
    words[col / 64] & (1 << (col % 64)) != 0
}

/// Sets bits `c0..c1`.
fn set_range(words: &mut [u64], c0: usize, c1: usize) {
    if c0 >= c1 {
        return;
    }
    let (first, last) = (c0 / 64, (c1 - 1) / 64);
    for (w, word) in words.iter_mut().enumerate().take(last + 1).skip(first) {
        *word |= range_mask(if w == first { c0 % 64 } else { 0 }, if w == last { (c1 - 1) % 64 + 1 } else { 64 });
    }
}

fn any_in_range(words: &[u64], c0: usize, c1: usize) -> bool {
    if c0 >= c1 {
        return false;
    }
    let (first, last) = (c0 / 64, (c1 - 1) / 64);
    (first..=last).any(|w| words[w] & range_mask(if w == first { c0 % 64 } else { 0 }, if w == last { (c1 - 1) % 64 + 1 } else { 64 }) != 0)
}

/// Bits `lo..hi` of a word.
fn range_mask(lo: usize, hi: usize) -> u64 {
    let width = hi - lo;
    if width >= 64 { !0 } else { ((1u64 << width) - 1) << lo }
}

/// The first set bit at or after `from` and before `to`.
fn next_set(words: &[u64], from: usize, to: usize) -> Option<usize> {
    let mut word = from / 64;
    let mut mask = !0u64 << (from % 64);
    while word * 64 < to {
        let bits = words[word] & mask;
        if bits != 0 {
            let found = word * 64 + bits.trailing_zeros() as usize;
            return (found < to).then_some(found);
        }
        word += 1;
        mask = !0;
    }
    None
}

/// The first clear bit at or after `from`, or `to`.
fn next_clear(words: &[u64], from: usize, to: usize) -> usize {
    let mut word = from / 64;
    let mut mask = !0u64 << (from % 64);
    while word * 64 < to {
        let bits = !words[word] & mask;
        if bits != 0 {
            return (word * 64 + bits.trailing_zeros() as usize).min(to);
        }
        word += 1;
        mask = !0;
    }
    to
}

/// What can be seen of each of `layers`, in whole tiles: every tile its `extent` touches that no layer above it covers
/// completely (a tile is covered when it lies wholly inside one of the `opaque` rectangles of a layer above). The layers are in
/// drawing order, the last on top. `out` is reused: its sets are made empty first, and there is one for each layer.
///
/// Conservative where it can only draw more: a tile that an opaque layer covers in part is seen, and so is a layer's own rounded
/// corner next to its cover.
pub fn visible(width: u32, height: u32, layers: &[(Area, &[Area])], out: &mut Vec<TileSet>) {
    out.truncate(layers.len());
    for set in out.iter_mut() {
        set.clear();
    }
    while out.len() < layers.len() {
        out.push(TileSet::new(width, height));
    }
    let mut covered = TileSet::new(width, height);
    for (k, (extent, opaque)) in layers.iter().enumerate().rev() {
        out[k].add(*extent);
        out[k].subtract(&covered);
        for &rect in *opaque {
            covered.add_inside(rect);
        }
    }
}

/// The account of what each picture of a ring owes: what every frame drawn since it was last drawn into has changed (the
/// bitmap form of `damage::Ring`). A picture nothing is known of owes everything.
#[derive(Clone, Debug)]
pub struct Ring {
    owed: Vec<TileSet>,
}

impl Ring {
    /// A ring of `pictures` none of which is known to hold anything.
    pub fn new(pictures: usize, width: u32, height: u32) -> Self {
        Self { owed: vec![TileSet::full(width, height); pictures] }
    }

    pub fn len(&self) -> usize {
        self.owed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.owed.is_empty()
    }

    /// Frame after frame: `damage` is what the frame about to be drawn into picture `slot` changes from the frame before it
    /// (the one drawn into another picture). Every other picture now owes it too, and `into` is what `slot` has to draw: that and
    /// what the frames since it was last drawn into changed. A picture that does not exist is owed everything.
    pub fn frame(&mut self, slot: usize, damage: &TileSet, into: &mut TileSet) {
        let Some(owed) = self.owed.get_mut(slot) else {
            into.fill();
            return;
        };
        std::mem::swap(into, owed);
        owed.clear();
        into.union(damage);
        for (other, owed) in self.owed.iter_mut().enumerate() {
            if other != slot {
                owed.union(damage);
            }
        }
    }

    /// The pixels picture `slot` owes now (everything for one that does not exist).
    pub fn owed_pixels(&self, slot: usize) -> u64 {
        self.owed.get(slot).map_or_else(|| self.owed.first().map_or(0, |set| u64::from(set.panel().0) * u64::from(set.panel().1)), TileSet::pixels)
    }

    /// Nothing is known of what the pictures hold: each owes everything.
    pub fn forget(&mut self) {
        for set in &mut self.owed {
            set.fill();
        }
    }
}
