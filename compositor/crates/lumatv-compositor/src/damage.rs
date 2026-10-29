//! Drawing less of a frame: which 16 px tiles of the output changed, and which of them each scan-out buffer still
//! owes (LumaShow lesson 10: tile damage, rings of 3 pages for direct scan-out).
//!
//! A frame is drawn into one page of a ring, and that page last showed the frame one whole turn of the ring ago,
//! so it owes the union of every frame's damage since. [`Ring::frame`] keeps that account per page. A page nothing
//! is known of (the first turns, after a resize or a lost frame) owes everything. Tiles make the account a fixed-size
//! bitmap: no allocation on the frame path, and the rectangles handed to the renderer (`Tiles::rects`) are the tile
//! rows run-length merged, at most one per tile row, capped at [`MAX_RECTS`] by falling back to the bounds.

use crate::Rect;

/// Tile edge in output pixels.
pub const TILE: i32 = 16;
/// Most rectangles a frame is drawn as; beyond it, one bounding rectangle.
pub const MAX_RECTS: usize = 12;
/// Tile grid of a 1920x1080 output: 120 x 68 (1080 / 16 = 67.5, rounded up).
const MAX_COLS: usize = 240;
const MAX_ROWS: usize = 136;
const WORDS_PER_ROW: usize = MAX_COLS / 64;

/// A set of tiles of the output.
#[derive(Clone)]
pub struct Tiles {
    cols: usize,
    rows: usize,
    width: i32,
    height: i32,
    bits: [[u64; WORDS_PER_ROW]; MAX_ROWS],
}

impl Tiles {
    pub fn new(width: u32, height: u32) -> Tiles {
        let cols = ((width as i32 + TILE - 1) / TILE) as usize;
        let rows = ((height as i32 + TILE - 1) / TILE) as usize;
        assert!(cols <= MAX_COLS && rows <= MAX_ROWS, "output larger than the tile grid ({width}x{height})");
        Tiles { cols, rows, width: width as i32, height: height as i32, bits: [[0; WORDS_PER_ROW]; MAX_ROWS] }
    }

    pub fn clear(&mut self) {
        for row in self.bits.iter_mut().take(self.rows) {
            *row = [0; WORDS_PER_ROW];
        }
    }

    pub fn is_empty(&self) -> bool {
        self.bits.iter().take(self.rows).all(|r| r.iter().all(|w| *w == 0))
    }

    pub fn fill(&mut self) {
        for row in self.bits.iter_mut().take(self.rows) {
            for c in 0..self.cols {
                row[c / 64] |= 1u64 << (c % 64);
            }
        }
    }

    /// Adds every tile `r` touches (clipped to the output).
    pub fn add(&mut self, r: &Rect) {
        let r = r.intersect(&Rect::new(0, 0, self.width, self.height));
        if r.is_empty() {
            return;
        }
        let c0 = (r.x / TILE) as usize;
        let c1 = ((r.right() - 1) / TILE) as usize;
        let r0 = (r.y / TILE) as usize;
        let r1 = ((r.bottom() - 1) / TILE) as usize;
        for row in r0..=r1.min(self.rows - 1) {
            for c in c0..=c1.min(self.cols - 1) {
                self.bits[row][c / 64] |= 1u64 << (c % 64);
            }
        }
    }

    pub fn union(&mut self, other: &Tiles) {
        for (a, b) in self.bits.iter_mut().zip(other.bits.iter()).take(self.rows) {
            for (x, y) in a.iter_mut().zip(b.iter()) {
                *x |= *y;
            }
        }
    }

    pub fn count(&self) -> usize {
        self.bits.iter().take(self.rows).map(|r| r.iter().map(|w| w.count_ones() as usize).sum::<usize>()).sum()
    }

    pub fn pixels(&self) -> u64 {
        self.count() as u64 * (TILE * TILE) as u64
    }

    fn has(&self, row: usize, col: usize) -> bool {
        self.bits[row][col / 64] & (1u64 << (col % 64)) != 0
    }

    /// The bounding rectangle in output pixels, `None` when empty.
    pub fn bounds(&self) -> Option<Rect> {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for row in 0..self.rows {
            for col in 0..self.cols {
                if self.has(row, col) {
                    x0 = x0.min(col);
                    x1 = x1.max(col);
                    y0 = y0.min(row);
                    y1 = y1.max(row);
                }
            }
        }
        if x0 == usize::MAX {
            return None;
        }
        Some(self.clip(Rect::new(x0 as i32 * TILE, y0 as i32 * TILE, (x1 - x0 + 1) as i32 * TILE, (y1 - y0 + 1) as i32 * TILE)))
    }

    fn clip(&self, r: Rect) -> Rect {
        r.intersect(&Rect::new(0, 0, self.width, self.height))
    }

    /// The tiles as rectangles: runs of set tiles in a row become one rectangle, equal runs on consecutive rows
    /// are merged vertically. Writes into `out` (cleared first) so the frame path allocates nothing; more than
    /// [`MAX_RECTS`] collapses to the bounds.
    pub fn rects(&self, out: &mut Vec<Rect>) {
        out.clear();
        for row in 0..self.rows {
            let mut col = 0;
            while col < self.cols {
                if !self.has(row, col) {
                    col += 1;
                    continue;
                }
                let start = col;
                while col < self.cols && self.has(row, col) {
                    col += 1;
                }
                let run = Rect::new(start as i32 * TILE, row as i32 * TILE, (col - start) as i32 * TILE, TILE);
                match out.last_mut() {
                    Some(prev) if prev.x == run.x && prev.w == run.w && prev.bottom() == run.y => prev.h += TILE,
                    _ => {
                        if out.len() == MAX_RECTS {
                            let b = self.bounds().unwrap_or_default();
                            out.clear();
                            out.push(b);
                            return;
                        }
                        out.push(run);
                    }
                }
            }
        }
        for r in out.iter_mut() {
            *r = self.clip(*r);
        }
    }
}

/// The damage each page of a scan-out ring still owes.
pub struct Ring {
    owed: Vec<Tiles>,
    known: Vec<bool>,
    scratch: Tiles,
}

impl Ring {
    pub fn new(pages: usize, width: u32, height: u32) -> Ring {
        let mut t = Tiles::new(width, height);
        t.fill();
        Ring { owed: vec![t.clone(); pages], known: vec![false; pages], scratch: Tiles::new(width, height) }
    }

    pub fn len(&self) -> usize {
        self.owed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.owed.is_empty()
    }

    /// Frame `damage` (what this frame changes against the last one shown) is about to be drawn into page `slot`:
    /// adds it to every page's account and returns what `slot` owes in total, then marks `slot` current.
    pub fn frame(&mut self, slot: usize, damage: &Tiles) -> &Tiles {
        for (i, page) in self.owed.iter_mut().enumerate() {
            if i != slot {
                page.union(damage);
            }
        }
        self.scratch.clear();
        if self.known[slot] {
            self.scratch.union(&self.owed[slot]);
            self.scratch.union(damage);
        } else {
            self.scratch.fill();
        }
        self.owed[slot].clear();
        self.known[slot] = true;
        &self.scratch
    }

    /// Page `slot` was just drawn, but `tiles` of it hold older content (copied from another page in place of a buffer the
    /// driver was still busy with): it owes them again.
    pub fn owe(&mut self, slot: usize, tiles: &Tiles) {
        if !tiles.is_empty() {
            self.owed[slot].union(tiles);
        }
    }

    /// Pixels page `slot` would have to draw now.
    pub fn owed_pixels(&self, slot: usize) -> u64 {
        if self.known[slot] { self.owed[slot].pixels() } else { self.scratch.width as u64 * self.scratch.height as u64 }
    }

    /// Forgets one page whose memory was replaced: its next frame is drawn whole.
    pub fn forget_page(&mut self, slot: usize) {
        self.known[slot] = false;
    }

    /// Forgets every page (a resize, a frame that may not have reached the panel whole).
    pub fn forget(&mut self) {
        for k in self.known.iter_mut() {
            *k = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_cover_touched_rectangles_and_merge_into_few_rects() {
        let mut t = Tiles::new(1920, 1080);
        assert!(t.is_empty());
        t.add(&Rect::new(10, 10, 4, 4));
        assert_eq!(t.count(), 1);
        assert_eq!(t.bounds(), Some(Rect::new(0, 0, 16, 16)));
        t.add(&Rect::new(15, 15, 2, 2)); // straddles four tiles
        assert_eq!(t.count(), 4);
        let mut out = Vec::new();
        t.rects(&mut out);
        assert_eq!(out, vec![Rect::new(0, 0, 32, 32)], "two equal runs merge vertically");
        t.clear();
        t.add(&Rect::new(1900, 1070, 100, 100));
        assert_eq!(t.bounds(), Some(Rect::new(1888, 1056, 32, 24)), "clipped to the output");
    }

    #[test]
    fn many_scattered_tiles_collapse_to_the_bounds() {
        let mut t = Tiles::new(1920, 1080);
        for k in 0..20 {
            t.add(&Rect::new(k * 80, k * 40, 8, 8));
        }
        let mut out = Vec::new();
        t.rects(&mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], t.bounds().unwrap());
    }

    #[test]
    fn a_ring_page_owes_what_every_frame_since_its_turn_changed() {
        let mut ring = Ring::new(3, 1920, 1080);
        let mut d = Tiles::new(1920, 1080);
        // First turn: every page owes everything.
        d.add(&Rect::new(0, 0, 16, 16));
        assert_eq!(ring.frame(0, &d).count(), 120 * 68);
        d.clear();
        d.add(&Rect::new(96, 96, 16, 16));
        assert_eq!(ring.frame(1, &d).count(), 120 * 68);
        d.clear();
        d.add(&Rect::new(192, 192, 16, 16));
        assert_eq!(ring.frame(2, &d).count(), 120 * 68);
        // Second turn: page 0 owes frames 1 and 2 plus its own.
        d.clear();
        d.add(&Rect::new(288, 288, 16, 16));
        let owed = ring.frame(0, &d);
        assert_eq!(owed.count(), 3);
        let mut out = Vec::new();
        owed.rects(&mut out);
        assert!(out.contains(&Rect::new(96, 96, 16, 16)) && out.contains(&Rect::new(192, 192, 16, 16)) && out.contains(&Rect::new(288, 288, 16, 16)));
        // A frame that changes nothing still owes what the other pages did since.
        d.clear();
        assert_eq!(ring.frame(1, &d).count(), 2, "page 1 owes frames 2 and 3");
        ring.forget();
        assert_eq!(ring.frame(2, &d).count(), 120 * 68);
    }

    #[test]
    fn tiles_a_page_copied_in_place_of_a_warming_buffer_stay_owed_by_that_page() {
        let mut ring = Ring::new(3, 1920, 1080);
        let mut d = Tiles::new(1920, 1080);
        for page in 0..3 {
            ring.frame(page, &d);
        }
        // Frame into page 0: one tile's content came from another page, not from the buffer.
        d.add(&Rect::new(320, 320, 16, 16));
        assert_eq!(ring.frame(0, &d).count(), 1);
        ring.owe(0, &d);
        d.clear();
        assert_eq!(ring.frame(1, &d).count(), 1, "page 1 owes frame 0's damage as ever");
        assert_eq!(ring.frame(2, &d).count(), 1);
        assert_eq!(ring.frame(0, &d).count(), 1, "page 0 owes the tile it only copied");
        assert_eq!(ring.frame(1, &d).count(), 0, "the other pages owe nothing for it");
    }

    #[test]
    fn a_page_made_over_in_another_layout_is_drawn_whole_and_the_others_keep_their_accounts() {
        let mut ring = Ring::new(3, 1920, 1080);
        let mut d = Tiles::new(1920, 1080);
        for page in 0..3 {
            ring.frame(page, &d);
        }
        ring.forget_page(1);
        d.add(&Rect::new(64, 64, 16, 16));
        assert_eq!(ring.frame(0, &d).count(), 1, "page 0 draws only its damage");
        d.clear();
        assert_eq!(ring.frame(1, &d).count(), 120 * 68, "the new page owes every tile");
        assert_eq!(ring.frame(2, &d).count(), 1, "page 2 still owes the one change it missed");
    }
}
