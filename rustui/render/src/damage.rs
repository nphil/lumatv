//! Drawing less of a frame: which panel pixels a frame changes, and which of what is drawn can be seen.
//!
//! Two questions, both answered in panel pixels ([`Area`], `[left, top, right, bottom]`) and both pure, so that they are
//! tested without a GPU:
//!
//! **What can be seen** ([`sights`]). A layer that an opaque layer above it covers completely is not drawn, and one that
//! it covers in part is drawn only within the rectangle of what is left. Which layers cover what is the renderer's
//! knowledge (`Cover`): here a cover is a set of rectangles that are certainly opaque.
//!
//! **What changed** ([`Region`], [`Ring`]). A frame that differs from the one before in a part of the screen only needs that
//! part drawn again, and the part is a few rectangles rather than one: a line that glides moves a hairline, a band of words
//! above it and a band of chips below, and the one rectangle holding all three is most of the panel while the three together
//! are a fraction of it. But the picture a frame is drawn *into* is not the one before: the scan-out pages are a ring (three
//! of them for the direct route, two render buffers for the copy route), and a page holds the frame drawn into it a whole turn
//! of the ring ago. So each page owes the union of what every frame since has changed; the ring keeps that account and
//! hands a frame the region its page needs. A page nothing is known of (the first turns, and after a frame that may not
//! have reached it whole) owes everything.

use crate::backdrop::Area;

/// The part of the panel one layer differs in; `None` when nothing does.
pub type Damage = Option<Area>;

/// The most rectangles a [`Region`] keeps apart. Each costs the GPU a scissored clear and a pass over the layers that touch
/// it, so past this many the two that waste the least pixels merged are merged. A 1080p home whose wallpaper steps shows it in
/// the margins and gutters between its cards, which are more rectangles than a page of a 960 x 480 panel has.
pub const MAX_RECTS: usize = 16;

/// The pixels a pass is worth besides the layers it draws: two rectangles whose one cover costs no more than both and this are
/// drawn as the cover (its scissored clear and the state it sets: about a tile row of pixels). The layers are not in this: a
/// region that is added to has only the geometry to go by, and [`Region::merged`] has the layers.
pub const RECT_WORTH: u64 = 4096;

/// The pixels one more layer drawn in a pass is worth: a layer that meets a rectangle is drawn again in the pass of every rectangle
/// it meets, and a draw costs the CPU its calls and the GPU its jobs whatever its pixels are. Measured on the Show: the settings
/// workload of the release gate (2,752 scrolling frames on the black Show, the region drawn as up to five rectangles and as their
/// one cover) took 10 ns for a pixel and 63 µs for a layer drawn (6,100 pixels); the white Show's dock workload 9 ns and 83 µs.
/// A pass costs a rectangle `RECT_WORTH`, its pixels and this for each layer it draws ([`Region::merged`]).
pub const DRAW_WORTH: u64 = 6_144;

/// The part of the panel that differs: up to [`MAX_RECTS`] rectangles, empty when nothing does. Rectangles that overlap, or
/// that are cheaper drawn as their one cover, are merged as they are added, so that drawing each rectangle on its own draws
/// as little as it can; a region is a value of fixed size, and nothing is allocated for one. Two regions are equal when they
/// hold the same rectangles, in whatever order.
#[derive(Clone, Copy, Debug, Default)]
pub struct Region {
    rects: [Area; MAX_RECTS],
    len: u8,
}

impl PartialEq for Region {
    fn eq(&self, other: &Self) -> bool {
        let (mut a, mut b) = (self.rects, other.rects);
        a[..self.len()].sort_unstable();
        b[..other.len()].sort_unstable();
        a[..self.len()] == b[..other.len()]
    }
}

impl Eq for Region {}

impl Region {
    /// Nothing differs.
    pub const NONE: Self = Self {
        rects: [[0; 4]; MAX_RECTS],
        len: 0,
    };

    /// Only `area` differs (nothing, when it is empty).
    pub fn of(area: Area) -> Self {
        let mut region = Self::NONE;
        region.add(area);
        region
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The rectangles, in no particular order.
    pub fn rects(&self) -> &[Area] {
        &self.rects[..usize::from(self.len)]
    }

    /// The smallest one rectangle holding all of it, if it holds anything.
    pub fn bounds(&self) -> Damage {
        self.rects().iter().copied().reduce(cover)
    }

    /// How many panel pixels drawing it draws: each rectangle's, and what two overlap in for each of them (the GPU draws each
    /// rectangle on its own).
    pub fn pixels(&self) -> u64 {
        self.rects().iter().map(pixels).sum()
    }

    /// Whether any of it meets `area`.
    pub fn overlaps(&self, area: &Area) -> bool {
        self.rects().iter().any(|r| intersect(r, area).is_some())
    }

    /// `area` differs too.
    pub fn add(&mut self, mut area: Area) {
        if pixels(&area) == 0 {
            return;
        }
        // Merged with each rectangle it is cheaper as one with, one at a time: the cover grows and may now be worth merging
        // with another.
        while let Some(k) = (0..self.len()).find(|&k| merges(&self.rects[k], &area)) {
            area = cover(self.remove(k), area);
        }
        if self.len() < MAX_RECTS {
            self.rects[self.len()] = area;
            self.len += 1;
            return;
        }
        // Full: of all the pairs among the rectangles and the newcomer, the one that wastes the least pixels as a cover goes.
        let mut best: (u64, usize, Option<usize>) = (u64::MAX, 0, None);
        for i in 0..MAX_RECTS {
            let with_newcomer = waste(&self.rects[i], &area);
            if with_newcomer < best.0 {
                best = (with_newcomer, i, None);
            }
            for j in i + 1..MAX_RECTS {
                let with_other = waste(&self.rects[i], &self.rects[j]);
                if with_other < best.0 {
                    best = (with_other, i, Some(j));
                }
            }
        }
        match best {
            (_, i, None) => {
                let merged = cover(self.remove(i), area);
                self.add(merged);
            }
            (_, i, Some(j)) => {
                // The later index first: removing swaps the last rectangle in.
                let (b, a) = (self.remove(j), self.remove(i));
                self.add(cover(a, b));
                self.add(area);
            }
        }
    }

    /// What one layer's damage says differs too.
    pub fn add_damage(&mut self, damage: Damage) {
        if let Some(area) = damage {
            self.add(area);
        }
    }

    /// Everything in `other` differs too.
    pub fn union(&mut self, other: &Self) {
        for &area in other.rects() {
            self.add(area);
        }
    }

    /// Each rectangle grown outwards to whole tiles of `tile` pixels, within `panel` ([`tiled`]); two that then meet are
    /// merged as ever.
    pub fn tiled(&self, tile: i32, panel: Area) -> Self {
        let mut out = Self::NONE;
        for &area in self.rects() {
            out.add(tiled(area, tile, panel));
        }
        out
    }

    /// The same region as at most `most` rectangles: the one cover of all of it for `most` of 1 (what a frame drawn as one
    /// rectangle draws).
    pub fn limited(&self, most: usize) -> Self {
        if self.len() <= most.max(1) {
            return *self;
        }
        let mut out = Self::NONE;
        if most <= 1 {
            out.add_damage(self.bounds());
            return out;
        }
        let mut left = *self;
        while left.len() > most {
            let mut best = (u64::MAX, 0, 0);
            for i in 0..left.len() {
                for j in i + 1..left.len() {
                    let waste = waste(&left.rects[i], &left.rects[j]);
                    if waste < best.0 {
                        best = (waste, i, j);
                    }
                }
            }
            let (b, a) = (left.remove(best.2), left.remove(best.1));
            left.add(cover(a, b));
        }
        left
    }

    /// The same region with the rectangles merged that are cheaper drawn in one pass over their cover than in a pass each, the
    /// pair that saves the most first, until no pair is. `cost` says what a pass over a rectangle costs (see [`RECT_WORTH`] and
    /// [`DRAW_WORTH`]: its pixels and every layer it draws, which a region knows nothing of), and `allowed` whether the cover of
    /// two (`allowed(a, b, cover)`) may take their place at all, for what a cost does not say. What is drawn only grows: every
    /// pixel the region held is in a rectangle of what comes back.
    ///
    /// [`Region::add`] merges by the geometry alone, and what that leaves apart is cheap to draw apart only if few layers meet
    /// the rectangles: the rows of a page that scrolls are one band each with a few percent of the page between them, and every
    /// layer under the page (its card, its shadows, the wallpaper) is drawn in the pass of each, where its cover is one pass.
    pub fn merged(
        &self,
        cost: impl Fn(&Area) -> u64,
        allowed: impl Fn(&Area, &Area, &Area) -> bool,
    ) -> Self {
        let mut out = *self;
        let mut costs = [0; MAX_RECTS];
        for (k, area) in out.rects().iter().enumerate() {
            costs[k] = cost(area);
        }
        loop {
            // Of the pairs whose cover costs less than the two apart, and that may be one, the one that saves the most.
            let mut best: Option<(u64, usize, usize, Area, u64)> = None;
            for i in 0..out.len() {
                for j in i + 1..out.len() {
                    let one = cover(out.rects[i], out.rects[j]);
                    let one_cost = cost(&one);
                    let saved = (costs[i] + costs[j]).saturating_sub(one_cost);
                    if saved > best.map_or(0, |b| b.0)
                        && allowed(&out.rects[i], &out.rects[j], &one)
                    {
                        best = Some((saved, i, j, one, one_cost));
                    }
                }
            }
            let Some((_, i, j, one, one_cost)) = best else {
                return out;
            };
            // `j` goes (the last rectangle takes its place, and its cost with it) and `i` is the cover.
            let last = out.len() - 1;
            out.remove(j);
            costs[j] = costs[last];
            out.rects[i] = one;
            costs[i] = one_cost;
        }
    }

    fn len(&self) -> usize {
        usize::from(self.len)
    }

    /// Takes rectangle `k` out, moving the last one into its place.
    fn remove(&mut self, k: usize) -> Area {
        let last = self.len() - 1;
        let out = self.rects[k];
        self.rects[k] = self.rects[last];
        self.len -= 1;
        out
    }
}

/// The pixels drawing `a` and `b` as their one cover draws that drawing each on its own does not: what the cover holds past
/// both (what they share is drawn twice apart and once as the cover, which counts for the cover).
fn waste(a: &Area, b: &Area) -> u64 {
    pixels(&cover(*a, *b)).saturating_sub(pixels(a) + pixels(b))
}

/// Whether `a` and `b` are cheaper drawn as their one cover than each on its own.
fn merges(a: &Area, b: &Area) -> bool {
    waste(a, b) <= RECT_WORTH
}

/// The smallest area holding both.
pub fn cover(a: Area, b: Area) -> Area {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// The smallest area holding both damages.
pub fn union(a: Damage, b: Damage) -> Damage {
    match (a, b) {
        (Some(a), Some(b)) => Some(cover(a, b)),
        (a, None) => a,
        (None, b) => b,
    }
}

/// `area` grown outwards to whole tiles of `tile` pixels, within `panel`. The GPU draws in tiles, and a tile drawn in
/// part has to be loaded from memory first and written back whole: a region that ends where tiles do costs it no more
/// than its pixels.
pub fn tiled(area: Area, tile: i32, panel: Area) -> Area {
    [
        (area[0].div_euclid(tile) * tile).max(panel[0]),
        (area[1].div_euclid(tile) * tile).max(panel[1]),
        ((area[2] + tile - 1).div_euclid(tile) * tile).min(panel[2]),
        ((area[3] + tile - 1).div_euclid(tile) * tile).min(panel[3]),
    ]
}

/// What both areas hold, if anything.
pub fn intersect(a: &Area, b: &Area) -> Option<Area> {
    let area = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    (area[2] > area[0] && area[3] > area[1]).then_some(area)
}

/// Whether `outer` holds every pixel of `inner`.
pub fn contains(outer: &Area, inner: &Area) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
}

/// How many pixels an area has.
pub fn pixels(area: &Area) -> u64 {
    (area[2] - area[0]).max(0) as u64 * (area[3] - area[1]).max(0) as u64
}

/// What is left of `rect` when `hole` is taken out of it: up to four rectangles that tile it exactly. `rect` itself when
/// they do not meet, nothing when the hole holds it.
fn without(rect: Area, hole: &Area, out: &mut Vec<Area>) {
    let Some(cut) = intersect(&rect, hole) else {
        out.push(rect);
        return;
    };
    // Above and below the hole across the whole width, then left and right of it beside it.
    for piece in [
        [rect[0], rect[1], rect[2], cut[1]],
        [rect[0], cut[3], rect[2], rect[3]],
        [rect[0], cut[1], cut[0], cut[3]],
        [cut[2], cut[1], rect[2], cut[3]],
    ] {
        if piece[2] > piece[0] && piece[3] > piece[1] {
            out.push(piece);
        }
    }
}

/// One layer of a frame, in drawing order, as far as what covers what is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece<'a> {
    /// Everything of the layer that is drawn, on the panel: its body and the fringe, shadows or light around it, less
    /// what its clip leaves out.
    pub extent: Area,
    /// Rectangles inside `extent` where the layer certainly covers what is below it completely.
    pub opaque: &'a [Area],
}

/// What can be seen of a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sight {
    /// Covered completely by what is above it: not drawn.
    Hidden,
    /// Drawn within this area, which is all of it that is not covered as far as a rectangle says.
    Seen(Area),
}

impl Sight {
    /// The area the layer is drawn in, if it is.
    pub fn area(self) -> Option<Area> {
        match self {
            Self::Hidden => None,
            Self::Seen(area) => Some(area),
        }
    }
}

/// The covers that are kept while sweeping down through a frame: past this many the smallest are forgotten, which can
/// only make a layer look more visible than it is.
const MAX_COVERS: usize = 24;
/// The pieces one layer's visible part may be cut into: past this it is taken whole, which is the same.
const MAX_PIECES: usize = 32;

/// What can be seen of each of `pieces`, which are in drawing order (the last is on top): the layers above cover what
/// they cover, and a layer's own covers count for the ones below it whether or not it is itself seen.
pub fn sights(pieces: &[Piece]) -> Vec<Sight> {
    let mut out = vec![Sight::Hidden; pieces.len()];
    let mut covers: Vec<Area> = Vec::new();
    let mut left: Vec<Area> = Vec::new();
    let mut next: Vec<Area> = Vec::new();
    for (index, piece) in pieces.iter().enumerate().rev() {
        left.clear();
        left.push(piece.extent);
        for hole in covers
            .iter()
            .filter(|c| intersect(&piece.extent, c).is_some())
        {
            if left.len() > MAX_PIECES {
                break;
            }
            next.clear();
            for &rect in &left {
                without(rect, hole, &mut next);
            }
            std::mem::swap(&mut left, &mut next);
            if left.is_empty() {
                break;
            }
        }
        out[index] = match left.iter().copied().reduce(cover) {
            Some(area) => Sight::Seen(area),
            None => Sight::Hidden,
        };
        covers.extend(piece.opaque.iter().filter(|a| pixels(a) > 0));
        if covers.len() > MAX_COVERS {
            covers.sort_by_key(|a| std::cmp::Reverse(pixels(a)));
            covers.truncate(MAX_COVERS);
        }
    }
    out
}

/// The account of what each picture a frame can be drawn into owes: what every frame drawn since it was last drawn into
/// has changed.
#[derive(Clone, Debug)]
pub struct Ring {
    owed: Vec<Region>,
    everything: Area,
}

impl Ring {
    /// A ring of `targets` pictures on a panel of `everything`, none of which is known to hold anything: each owes all
    /// of it.
    pub fn new(targets: usize, everything: Area) -> Self {
        Self {
            owed: vec![Region::of(everything); targets],
            everything,
        }
    }

    /// The number of pictures.
    pub fn len(&self) -> usize {
        self.owed.len()
    }

    /// The pixels picture `slot` owes: what the frames drawn into the other pictures since it was last drawn into changed, which the
    /// next frame into it draws before anything of its own. A picture that does not exist owes everything.
    pub fn owed_pixels(&self, slot: usize) -> u64 {
        self.owed
            .get(slot)
            .map_or_else(|| pixels(&self.everything), Region::pixels)
    }

    /// Frame after frame: `damage` is what the frame about to be drawn into picture `slot` changes from the frame before
    /// it (the one drawn into another picture), and the answer is what has to be drawn into `slot`: that and what the
    /// frames since it was last drawn into changed. Every other picture now owes `damage` too. A picture that does not
    /// exist is owed everything.
    pub fn frame(&mut self, slot: usize, damage: &Region) -> Region {
        let Some(owed) = self.owed.get_mut(slot) else {
            return Region::of(self.everything);
        };
        let mut region = std::mem::take(owed);
        region.union(damage);
        for (other, owed) in self.owed.iter_mut().enumerate() {
            if other != slot {
                owed.union(damage);
            }
        }
        region
    }

    /// Nothing is known of what the pictures hold: the next frame into each draws everything. For a frame that may not
    /// have been drawn whole.
    pub fn forget(&mut self) {
        self.owed.fill(Region::of(self.everything));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANEL: Area = [0, 0, 480, 960];

    fn piece(extent: Area, opaque: &[Area]) -> Piece<'_> {
        Piece { extent, opaque }
    }

    #[test]
    fn a_region_is_grown_to_whole_tiles_and_never_past_the_panel() {
        assert_eq!(tiled([17, 33, 40, 48], 16, PANEL), [16, 32, 48, 48]);
        assert_eq!(tiled([16, 32, 48, 48], 16, PANEL), [16, 32, 48, 48]);
        assert_eq!(tiled([0, 0, 1, 1], 16, PANEL), [0, 0, 16, 16]);
        // The panel's own edge is not a tile edge: nothing is drawn past it.
        assert_eq!(tiled([470, 950, 480, 960], 16, PANEL), [464, 944, 480, 960]);
        assert_eq!(
            tiled([475, 955, 478, 958], 16, [0, 0, 480, 950]),
            [464, 944, 480, 950]
        );
    }

    #[test]
    fn what_is_left_of_a_rectangle_after_a_hole_is_tiled_by_pieces_that_miss_the_hole() {
        let rect = [0, 0, 100, 100];
        let mut out = Vec::new();
        without(rect, &[40, 30, 60, 70], &mut out);
        assert_eq!(out.len(), 4);
        // Together the pieces and the hole are the rectangle, and they never overlap.
        let total: u64 = out.iter().map(pixels).sum();
        assert_eq!(total + 20 * 40, 100 * 100);
        for (i, a) in out.iter().enumerate() {
            assert!(intersect(a, &[40, 30, 60, 70]).is_none());
            for b in &out[i + 1..] {
                assert!(intersect(a, b).is_none());
            }
        }
        // A hole that misses leaves it whole, one that holds it leaves nothing, and one across a corner leaves an L.
        out.clear();
        without(rect, &[200, 0, 300, 100], &mut out);
        assert_eq!(out, [rect]);
        out.clear();
        without(rect, &[-5, -5, 105, 105], &mut out);
        assert!(out.is_empty());
        without(rect, &[50, 50, 200, 200], &mut out);
        assert_eq!(out.iter().map(pixels).sum::<u64>(), 100 * 100 - 50 * 50);
    }

    #[test]
    fn a_layer_an_opaque_layer_above_it_covers_is_hidden_and_the_layer_above_is_seen() {
        let page = [0, 0, 480, 960];
        let out = sights(&[
            piece([10, 10, 200, 200], &[]),
            piece(page, &[page]),
            piece([20, 20, 100, 100], &[]),
        ]);
        assert_eq!(out[0], Sight::Hidden);
        assert_eq!(out[1], Sight::Seen(page));
        assert_eq!(out[2], Sight::Seen([20, 20, 100, 100]));
    }

    #[test]
    fn a_layer_is_only_hidden_by_what_is_above_it_never_by_what_is_below() {
        let out = sights(&[
            piece([0, 0, 480, 960], &[[0, 0, 480, 960]]),
            piece([10, 10, 100, 100], &[]),
        ]);
        assert_eq!(
            out,
            [
                Sight::Seen([0, 0, 480, 960]),
                Sight::Seen([10, 10, 100, 100])
            ]
        );
    }

    #[test]
    fn a_layer_that_is_not_opaque_hides_nothing() {
        let out = sights(&[piece([10, 10, 100, 100], &[]), piece([0, 0, 480, 960], &[])]);
        assert_eq!(out[0], Sight::Seen([10, 10, 100, 100]));
    }

    #[test]
    fn two_covers_side_by_side_hide_what_neither_hides_alone() {
        let out = sights(&[
            piece([100, 100, 300, 200], &[]),
            piece([0, 0, 200, 960], &[[0, 0, 200, 960]]),
            piece([200, 0, 480, 960], &[[200, 0, 480, 960]]),
        ]);
        assert_eq!(out[0], Sight::Hidden);
    }

    #[test]
    fn a_layer_covered_in_part_is_seen_in_what_is_left_of_it() {
        // A sheet over the lower half of a card: the card is drawn only above it.
        let out = sights(&[
            piece([0, 0, 480, 400], &[]),
            piece([0, 300, 480, 960], &[[0, 300, 480, 960]]),
        ]);
        assert_eq!(out[0], Sight::Seen([0, 0, 480, 300]));
        // A cover in the middle leaves a frame around it; the area is the rectangle round what is left.
        let out = sights(&[
            piece([0, 0, 480, 400], &[]),
            piece([100, 100, 380, 300], &[[100, 100, 380, 300]]),
        ]);
        assert_eq!(out[0], Sight::Seen([0, 0, 480, 400]));
        // One that leaves a sliver on one side only is seen in the sliver.
        let out = sights(&[
            piece([0, 0, 480, 400], &[]),
            piece([0, 0, 470, 400], &[[0, 0, 470, 400]]),
        ]);
        assert_eq!(out[0], Sight::Seen([470, 0, 480, 400]));
    }

    #[test]
    fn a_hidden_layers_covers_still_hide_what_is_below_it() {
        // The middle layer is hidden by the top one, and covers the bottom one.
        let out = sights(&[
            piece([100, 100, 200, 200], &[]),
            piece([50, 50, 300, 300], &[[50, 50, 300, 300]]),
            piece([0, 0, 480, 960], &[[0, 0, 480, 960]]),
        ]);
        assert_eq!(out[0], Sight::Hidden);
        assert_eq!(out[1], Sight::Hidden);
    }

    #[test]
    fn a_frame_of_many_covers_forgets_the_small_ones_and_is_never_wrong_about_a_layer_being_hidden()
    {
        // Fifty small covers and a layer under all of them: the layer is seen (some covers are forgotten) or hidden, and
        // if it says hidden it is fully inside a cover that was kept.
        let covers: Vec<Area> = (0..50).map(|k| [k * 8, 0, k * 8 + 8, 8]).collect();
        let mut pieces = vec![piece([0, 0, 400, 8], &[])];
        for c in &covers {
            pieces.push(piece(*c, std::slice::from_ref(c)));
        }
        let out = sights(&pieces);
        assert!(matches!(out[0], Sight::Seen(_)), "{:?}", out[0]);
        // Whatever it says, what is seen contains what is really not covered.
        let Sight::Seen(area) = out[0] else { return };
        assert!(contains(&[0, 0, 400, 8], &area));
    }

    /// The rectangles of a region, sorted, so that a test can say what it holds whatever their order.
    fn sorted(region: &Region) -> Vec<Area> {
        let mut out = region.rects().to_vec();
        out.sort();
        out
    }

    #[test]
    fn a_frame_owes_a_picture_what_every_frame_since_it_was_drawn_into_has_changed() {
        let mut ring = Ring::new(3, PANEL);
        // Nothing is known of the pictures: the first turn draws everything, whatever it changed.
        for slot in 0..3 {
            assert_eq!(ring.frame(slot, &Region::NONE), Region::of(PANEL));
        }
        // Now every picture is known. A frame that changed a clock draws the clock into its picture, and the other two
        // pictures owe it.
        let clock = [100, 100, 140, 120];
        assert_eq!(ring.frame(0, &Region::of(clock)), Region::of(clock));
        // The next frame changes a toggle: picture 1 owes both (it missed the clock's frame), and is given both, apart:
        // the one cover of the two would be most of the panel.
        let toggle = [300, 400, 340, 440];
        let both = ring.frame(1, &Region::of(toggle));
        assert_eq!(sorted(&both), [clock, toggle]);
        // Picture 2 missed both as well; a frame that changes nothing still brings it up to date.
        assert_eq!(sorted(&ring.frame(2, &Region::NONE)), [clock, toggle]);
        // Picture 0 last drew the clock, and missed the toggle.
        assert_eq!(ring.frame(0, &Region::NONE), Region::of(toggle));
        // Everything is up to date: a frame that changes nothing draws nothing, into any picture.
        assert_eq!(ring.frame(1, &Region::NONE), Region::NONE);
        assert_eq!(ring.frame(2, &Region::NONE), Region::NONE);
        assert_eq!(ring.frame(0, &Region::NONE), Region::NONE);
    }

    #[test]
    fn a_ring_of_two_and_a_ring_of_one_keep_the_same_account() {
        let mut two = Ring::new(2, PANEL);
        two.frame(0, &Region::NONE);
        two.frame(1, &Region::NONE);
        let a = [0, 0, 10, 10];
        let b = [200, 200, 210, 210];
        assert_eq!(two.frame(0, &Region::of(a)), Region::of(a));
        assert_eq!(sorted(&two.frame(1, &Region::of(b))), [a, b]);
        assert_eq!(two.frame(0, &Region::NONE), Region::of(b));
        let mut one = Ring::new(1, PANEL);
        one.frame(0, &Region::NONE);
        assert_eq!(one.frame(0, &Region::of(a)), Region::of(a));
        assert_eq!(one.frame(0, &Region::NONE), Region::NONE);
    }

    #[test]
    fn forgetting_owes_every_picture_everything_and_a_picture_that_is_not_there_is_owed_everything()
    {
        let mut ring = Ring::new(3, PANEL);
        for slot in 0..3 {
            ring.frame(slot, &Region::NONE);
        }
        assert_eq!(ring.frame(1, &Region::NONE), Region::NONE);
        ring.forget();
        for slot in 0..3 {
            assert_eq!(ring.frame(slot, &Region::NONE), Region::of(PANEL));
        }
        assert_eq!(ring.frame(7, &Region::NONE), Region::of(PANEL));
        assert_eq!(ring.len(), 3);
    }

    #[test]
    fn a_region_keeps_rectangles_apart_that_are_cheaper_apart_and_merges_the_ones_that_are_not() {
        // A hairline, a band of words above the axis and a band of chips below it, as a line that glides changes them: the
        // one cover is 896 x 416 pixels, the three together a fraction of it.
        let hairline = [300, 64, 316, 480];
        let words = [32, 96, 928, 128];
        let chips = [32, 352, 928, 432];
        let mut region = Region::NONE;
        for area in [hairline, words, chips] {
            region.add(area);
        }
        assert_eq!(sorted(&region), [words, chips, hairline]);
        // Each rectangle's pixels, as the GPU draws them: the hairline crosses both bands, and what they share is drawn twice.
        assert_eq!(
            region.pixels(),
            pixels(&hairline) + pixels(&words) + pixels(&chips)
        );
        assert_eq!(region.bounds(), Some([32, 64, 928, 480]));
        assert!(region.overlaps(&[0, 0, 40, 100]));
        assert!(!region.overlaps(&[0, 0, 40, 60]));
        // A rectangle inside one of them changes nothing; one that holds one takes its place.
        region.add([400, 100, 500, 120]);
        assert_eq!(sorted(&region), [words, chips, hairline]);
        region.add([32, 90, 928, 130]);
        assert_eq!(sorted(&region), [[32, 90, 928, 130], chips, hairline]);
        // Two that overlap by most of themselves, and two that nearly touch, are one: their cover wastes nothing worth a
        // rectangle of its own.
        let mut close = Region::of([0, 0, 100, 100]);
        close.add([10, 10, 110, 110]);
        assert_eq!(close.rects(), [[0, 0, 110, 110]]);
        close.add([0, 112, 110, 140]);
        assert_eq!(close.rects(), [[0, 0, 110, 140]]);
        // Two of 200 x 200 offset by 50: apart they draw 80,000 pixels and their cover 62,500; the cover wastes nothing, and
        // what they share is no credit to drawing them apart (it is drawn twice).
        let mut offset = Region::of([0, 0, 200, 200]);
        offset.add([50, 50, 250, 250]);
        assert_eq!(offset.rects(), [[0, 0, 250, 250]]);
        // Three of 208 x 208 down a diagonal, each overlapping the next and all three sharing a corner: too far apart to be
        // one, and their pixels are the sum of each (the corner all three share is drawn three times).
        let mut diagonal = Region::NONE;
        for k in 0..3 {
            diagonal.add([k * 100, k * 100, k * 100 + 208, k * 100 + 208]);
        }
        assert_eq!(diagonal.rects().len(), 3);
        assert_eq!(diagonal.pixels(), 3 * 208 * 208);
        // The same rectangles in another order are the same region.
        let mut backwards = Region::NONE;
        for k in (0..3).rev() {
            backwards.add([k * 100, k * 100, k * 100 + 208, k * 100 + 208]);
        }
        assert_ne!(diagonal.rects(), backwards.rects());
        assert_eq!(diagonal, backwards);
        // An empty rectangle is nothing.
        close.add([50, 50, 50, 90]);
        assert_eq!(close.rects(), [[0, 0, 110, 140]]);
        assert_eq!(Region::of([5, 5, 5, 5]), Region::NONE);
        assert!(Region::NONE.is_empty() && Region::NONE.bounds().is_none());
    }

    #[test]
    fn a_full_region_merges_the_pair_that_wastes_the_least_and_a_limited_one_does_the_same() {
        // Seven small squares far apart: the first six stay as they are, and the seventh makes the two closest ones merge.
        let square = |x: i32, y: i32| [x, y, x + 10, y + 10];
        let mut region = Region::NONE;
        for k in 0..MAX_RECTS as i32 {
            region.add(square(k * 100, k * 100));
        }
        assert_eq!(region.rects().len(), MAX_RECTS);
        region.add(square(1000, 1000));
        assert_eq!(region.rects().len(), MAX_RECTS);
        assert!(sorted(&region).contains(&[0, 0, 110, 110]), "{region:?}");
        assert!(sorted(&region).contains(&square(1000, 1000)));
        // Limited to one rectangle, a region is its cover; to fewer than it has, the closest pairs go together first.
        assert_eq!(region.limited(1).rects(), [[0, 0, 1010, 1010]]);
        let two = region.limited(2);
        assert_eq!(two.rects().len(), 2);
        assert!(sorted(&two).contains(&square(1000, 1000)), "{two:?}");
        assert_eq!(region.limited(MAX_RECTS), region);
    }

    /// What a pass over `area` costs when `layers` are what a frame draws (each in the part of it that is seen): the pixels, and
    /// every layer that meets the pass, drawn again for it.
    fn pass_cost(layers: &[Area], area: &Area) -> u64 {
        let drawn = layers
            .iter()
            .filter(|l| intersect(l, area).is_some())
            .count() as u64;
        RECT_WORTH + pixels(area) + DRAW_WORTH * drawn
    }

    /// The rows of a page that scrolls, as the region keeps them: five bands of 60 x 600 pixels, ten pixels apart (the ten
    /// between each pair is 6,000 pixels, more than a rectangle of its own is worth to `add`, which sees no layers).
    fn rows() -> Region {
        let mut region = Region::NONE;
        for k in 0..5 {
            region.add([100 + 70 * k, 300, 160 + 70 * k, 900]);
        }
        region
    }

    #[test]
    fn the_rows_of_a_page_are_one_pass_when_a_pass_draws_the_layers_under_them_and_apart_when_it_draws_none()
     {
        let rows = rows();
        assert_eq!(
            rows.rects().len(),
            5,
            "apart for the geometry alone: {rows:?}"
        );
        let cover = rows.bounds().unwrap();
        // The page's card, its shadow and the wallpaper under every row, and a layer of each row's own: a pass over a band draws
        // four, the pass over all of them eight, where the five passes draw twenty.
        let mut layers = vec![[0, 0, 480, 960], [80, 280, 480, 920], [90, 290, 470, 910]];
        layers.extend(rows.rects().iter().copied());
        let one = rows.merged(|a| pass_cost(&layers, a), |_, _, _| true);
        assert_eq!(one.rects(), [cover]);
        // Nothing but the rows' own layers: a pass draws one layer, and the five passes (231,000 pixels' worth) are cheaper than
        // the cover's (238,000: the gaps and a layer drawn five times over).
        let own: Vec<Area> = rows.rects().to_vec();
        let apart = rows.merged(|a| pass_cost(&own, a), |_, _, _| true);
        assert_eq!(apart, rows);
    }

    #[test]
    fn rectangles_far_apart_stay_apart_however_many_layers_are_under_both() {
        // A clock in one corner and a toggle in the other, and forty layers across the panel: the cover is most of the panel
        // (405,000 pixels), and the forty draws it would save (246,000) are not worth what it costs to draw.
        let mut region = Region::of([20, 20, 120, 60]);
        region.add([400, 880, 460, 940]);
        let layers: Vec<Area> = (0..40).map(|_| PANEL).collect();
        let merged = region.merged(|a| pass_cost(&layers, a), |_, _, _| true);
        assert_eq!(merged, region);
        // One that is near enough is worth the forty draws it saves: the cover is 21,000 pixels over, forty layers are 246,000.
        let mut near = Region::of([20, 20, 120, 60]);
        near.add([160, 100, 260, 140]);
        assert_eq!(
            near.merged(|a| pass_cost(&layers, a), |_, _, _| true)
                .rects(),
            [[20, 20, 260, 140]]
        );
    }

    #[test]
    fn a_cover_that_may_not_take_the_place_of_two_rectangles_does_not_however_much_it_saves() {
        let rows = rows();
        let layers: Vec<Area> = (0..8).map(|_| PANEL).collect();
        assert_eq!(
            rows.merged(|a| pass_cost(&layers, a), |_, _, _| true)
                .rects()
                .len(),
            1
        );
        let refused = rows.merged(|a| pass_cost(&layers, a), |_, _, _| false);
        assert_eq!(refused, rows);
        // Some may and some may not: the two rows at the page's left are never one with the others.
        let left = rows.rects().iter().map(|r| r[0]).min().unwrap();
        let some = rows.merged(
            |a| pass_cost(&layers, a),
            |a, b, _| a[0] > left + 70 && b[0] > left + 70,
        );
        assert!(some.rects().len() > 1 && some.rects().len() < 5, "{some:?}");
    }

    #[test]
    fn merging_passes_only_ever_draws_more_and_never_leaves_out_a_pixel_of_the_region() {
        let layers: Vec<Area> = (0..6).map(|_| [0, 0, 480, 960]).collect();
        let mut region = rows();
        region.add([20, 20, 60, 60]);
        region.add([400, 880, 460, 940]);
        let merged = region.merged(|a| pass_cost(&layers, a), |_, _, _| true);
        assert!(merged.rects().len() < region.rects().len(), "{merged:?}");
        for rect in region.rects() {
            assert!(
                merged.rects().iter().any(|m| contains(m, rect)),
                "{rect:?} is left out of {merged:?}"
            );
        }
        // A region of one rectangle, and none, is what it was.
        assert_eq!(
            Region::of(PANEL).merged(|a| pass_cost(&layers, a), |_, _, _| true),
            Region::of(PANEL)
        );
        assert_eq!(
            Region::NONE.merged(|a| pass_cost(&layers, a), |_, _, _| true),
            Region::NONE
        );
    }

    #[test]
    fn tiling_a_region_grows_each_rectangle_to_whole_tiles_and_merges_what_then_meets() {
        let mut region = Region::of([17, 33, 40, 48]);
        region.add([41, 33, 60, 48]);
        // Apart by a pixel they are one anyway (the cover wastes a column); on tiles they are one rectangle of tiles.
        assert_eq!(region.tiled(16, PANEL).rects(), [[16, 32, 64, 48]]);
        let mut far = Region::of([0, 0, 1, 1]);
        far.add([470, 950, 480, 960]);
        assert_eq!(
            sorted(&far.tiled(16, PANEL)),
            [[0, 0, 16, 16], [464, 944, 480, 960]]
        );
    }
}
