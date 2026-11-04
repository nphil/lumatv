//! Where the card's items are.
//!
//! The room the card gives them makes a grid of whole-pixel cells: a gutter between cells, the pixels that do not divide split at the two
//! ends, so every item has the same size and the same space around it. An item takes one cell (small), two side by side (wide) or four
//! (large); one that cannot be that big in this room is the next size down. A grid of one row scrolls along it by whole columns, a grid of
//! several rows scrolls down by whole rows, so no item is ever cut and the next one shows only as a peek in the card's padding.
//!
//! Pure: no fonts, no layers. The surface gives it the line heights it measured ([`Metrics`]) and draws what comes out.

use crate::kit::focus::Dir;
use ui::geometry::Frame;

/// The most cells in view at once: the layers and groups a cell costs are the scene's, and a card is no place for a wall of tiles.
pub const MAX_CELLS: usize = 12;
/// The most rows of cells.
pub const MAX_ROWS: usize = 4;
/// Strides built beyond the view on each side while the grid scrolls: the next row or column is made before it is wanted.
pub const PEEK: usize = 1;
/// How much of the next stride a scrolling card shows past its last whole one, as a share of a cell along the scroll axis: enough to be
/// a tile that is recognised (its glass, its glyph), fading out behind the card's edge.
pub const PEEK_SHARE: f32 = 0.45;
/// How far the peek may reach into the card's padding, so that the card does not give up all that room for it.
pub const BLEED: f32 = 20.0;

/// What the owner may make an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Size {
    Small,
    Wide,
    Large,
}

impl Size {
    pub const ALL: [Size; 3] = [Size::Small, Size::Wide, Size::Large];

    pub fn id(self) -> &'static str {
        match self {
            Size::Small => "small",
            Size::Wide => "wide",
            Size::Large => "large",
        }
    }

    pub fn from_id(id: &str) -> Option<Size> {
        Size::ALL.into_iter().find(|s| s.id() == id)
    }

    /// The cells it takes: columns, rows.
    pub fn cells(self) -> (usize, usize) {
        match self {
            Size::Small => (1, 1),
            Size::Wide => (2, 1),
            Size::Large => (2, 2),
        }
    }

    /// The glyph of the size control.
    pub fn glyph(self) -> &'static str {
        match self {
            Size::Small => "square",
            Size::Wide => "rectangle-horizontal",
            Size::Large => "layout-grid",
        }
    }
}

/// How the contents of a cell are arranged: the glyph above the words, the glyph beside them, or one line (a card too low for more).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Stacked,
    Compact,
    Mini,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// One row of cells: the grid scrolls sideways.
    Horizontal,
    /// Several rows: it scrolls down.
    Vertical,
}

/// The numbers of the type and the spacing a cell is built from, at the person's text size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub gap: f32,
    pub pad_x: f32,
    pub pad_y: f32,
    /// The side of a glyph.
    pub glyph: f32,
    /// The height of one line of each role of type a cell uses.
    pub label: f32,
    pub caption: f32,
    pub title: f32,
    pub body: f32,
    pub display: f32,
    /// The narrowest a cell may be.
    pub min_w: f32,
    pub scale: f32,
}

impl Metrics {
    /// From what the kit measures: the lines of the roles at the person's text size, in the order label, caption, title, body, display.
    pub fn new(gap: f32, pad_x: f32, pad_y: f32, lines: [f32; 5], scale: f32) -> Self {
        let s = scale.max(1.0);
        // a cell needs room for a name like "Desk lamp" at the person's size
        Self { gap, pad_x, pad_y, glyph: (44.0 * s.min(1.5)).round(), label: lines[0], caption: lines[1], title: lines[2], body: lines[3], display: lines[4], min_w: (170.0 * s.min(2.0)).round(), scale: s }
    }

    /// The shipped type scale at `scale`: what the kit measures, for the model, which has no fonts, until the surface says.
    pub fn nominal(scale: f32) -> Self {
        let s = scale.clamp(1.0, 2.0);
        // the type is rounded up at the person's size and the line is 1.4 of it, rounded up (`Look::ty`, `line_height`)
        let line = |px: f32| ((px * s).ceil() * 1.4).ceil();
        Self::new(16.0, 16.0, 12.0, [line(26.0), line(24.0), line(40.0), line(28.0), line(92.0)], s)
    }

    /// The height of a cell with the glyph over the words, at the least.
    pub fn stacked_h(&self) -> f32 {
        2.0 * self.pad_y + self.glyph + 4.0 + self.label + self.caption
    }

    /// With the glyph beside the words.
    pub fn compact_h(&self) -> f32 {
        2.0 * self.pad_y + self.glyph.max(self.label + self.caption)
    }

    /// One line of words, the glyph beside it.
    pub fn mini_h(&self) -> f32 {
        (self.pad_y * 2.0 * 0.67).round() + self.glyph.max(self.label)
    }

    /// What the card's heading takes: the card's title at the first line, the gutter under it.
    pub fn heading_h(&self) -> f32 {
        self.label + self.gap
    }

    fn min_h(&self, style: Style) -> f32 {
        match style {
            Style::Stacked => self.stacked_h(),
            Style::Compact => self.compact_h(),
            Style::Mini => self.mini_h(),
        }
    }
}

/// The cells of a card: how many, how big, where they start, and how they scroll.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    /// The size of one cell, whole pixels.
    pub cell: (f32, f32),
    pub gap: f32,
    /// Where the first cell is in the card's content box.
    pub origin: (f32, f32),
    /// The height the heading takes, or none when the room is better spent on the cells.
    pub heading: Option<f32>,
    pub style: Style,
    pub axis: Axis,
    /// The content box.
    pub content: (f32, f32),
    /// How much of the next stride shows past the last whole one when the grid scrolls (see [`PEEK_SHARE`]); the room it takes is already
    /// out of the cells. 0 for a grid that holds all its items.
    pub peek: f32,
}

impl Grid {
    pub fn stride(&self) -> (f32, f32) {
        (self.cell.0 + self.gap, self.cell.1 + self.gap)
    }

    /// The box of the cells `col..col+w` by `row..row+h`, with the grid scrolled `off` strides along its axis.
    pub fn rect(&self, col: usize, row: usize, w: usize, h: usize, off: usize) -> Frame {
        let (sx, sy) = self.stride();
        let (dx, dy) = match self.axis {
            Axis::Horizontal => (off as f32 * sx, 0.0),
            Axis::Vertical => (0.0, off as f32 * sy),
        };
        Frame::new(self.origin.0 + col as f32 * sx - dx, self.origin.1 + row as f32 * sy - dy, w as f32 * self.cell.0 + (w as f32 - 1.0) * self.gap, h as f32 * self.cell.1 + (h as f32 - 1.0) * self.gap)
    }

    /// How many strides the view holds along the axis.
    pub fn visible(&self) -> usize {
        match self.axis {
            Axis::Horizontal => self.cols,
            Axis::Vertical => self.rows,
        }
    }

    /// The grid's own box, the cells at rest.
    pub fn bounds(&self) -> Frame {
        self.rect(0, 0, self.cols, self.rows, 0)
    }
}

/// The grid for a content box of `w` by `h`. The heading is dropped when it would leave less than a cell of the smallest style.
pub fn grid(w: f32, h: f32, heading: bool, m: &Metrics) -> Grid {
    let (w, h) = (w.floor().max(1.0), h.floor().max(1.0));
    grid_in(w, h, heading, m, (w, h))
}

/// The least a cell of `style` may be: wide enough and high enough for its words.
fn least(style: Style, m: &Metrics) -> (f32, f32) {
    let w = match style {
        Style::Stacked => m.min_w,
        Style::Compact | Style::Mini => (m.glyph + m.pad_x * 2.0 + m.min_w).round(),
    };
    (w, m.min_h(style))
}

/// The part of the next stride a grid shows at the end of the axis it scrolls along: a share of a cell, whole pixels.
fn peek_of(g: &Grid) -> f32 {
    let along = match g.axis {
        Axis::Horizontal => g.cell.0,
        Axis::Vertical => g.cell.1,
    };
    (along * PEEK_SHARE).round()
}

/// The grid that keeps room past its last whole stride for a peek of the next one, along the axis it scrolls: the room is taken from the cells,
/// which changes the cells and so what a share of one comes to; it settles in a few steps, and a grid that does not settle, or whose cells would
/// be too small for their words, or that scrolls along another axis once the room is taken, is the plain one with no peek.
pub fn grid_peeking(w: f32, h: f32, heading: bool, m: &Metrics) -> Grid {
    let (w, h) = (w.floor().max(1.0), h.floor().max(1.0));
    let plain = grid(w, h, heading, m);
    let axis = plain.axis;
    let with = |reserve: f32| match axis {
        Axis::Horizontal => grid_in((w - reserve).max(1.0), h, heading, m, (w, h)),
        Axis::Vertical => grid_in(w, (h - reserve).max(1.0), heading, m, (w, h)),
    };
    let needs = |g: &Grid| (m.gap + peek_of(g) - BLEED).max(0.0);
    let mut reserve = needs(&plain);
    for _ in 0..4 {
        let next = with(reserve);
        let (least_w, least_h) = least(next.style, m);
        if next.axis != axis || next.cell.0 < least_w || next.cell.1 < least_h {
            break;
        }
        // the room is enough for the peek the cells now show (a little more than it needs is fine)
        if needs(&next) <= reserve + 1.0 {
            return Grid { peek: peek_of(&next).min(reserve + BLEED - m.gap).max(0.0), ..next };
        }
        reserve = needs(&next);
    }
    plain
}

fn grid_in(w: f32, h: f32, heading: bool, m: &Metrics, content: (f32, f32)) -> Grid {
    let (w, h) = (w.floor().max(1.0), h.floor().max(1.0));
    let head = if heading && h - m.heading_h() >= m.mini_h() { Some(m.heading_h()) } else { None };
    let top = head.unwrap_or(0.0);
    let avail = h - top;
    let style = if avail >= m.stacked_h() {
        Style::Stacked
    } else if avail >= m.compact_h() {
        Style::Compact
    } else {
        Style::Mini
    };
    let (min_w, min_h) = least(style, m);
    let mut rows = (((avail + m.gap) / (min_h + m.gap)).floor() as usize).clamp(1, MAX_ROWS);
    let cell_h = |rows: usize| ((avail - (rows as f32 - 1.0) * m.gap) / rows as f32).floor();
    // squarish cells where the glyph is over the words, a third as wide again again beside them
    let target = |rows: usize| match style {
        Style::Stacked => cell_h(rows).max(m.min_w),
        Style::Compact | Style::Mini => (cell_h(rows) * 2.6).max(min_w),
    };
    // a little wider than high, so that a name has room: a cell narrower than it is high leaves the words short of their cell
    let cols_for = |rows: usize| (((w + m.gap) / (target(rows) * 1.08 + m.gap)).round() as usize).max(1);
    let mut cols = cols_for(rows);
    // a column no wider than the least a cell needs is not a column
    while cols > 1 && ((w - (cols as f32 - 1.0) * m.gap) / cols as f32).floor() < min_w {
        cols -= 1;
    }
    while cols * rows > MAX_CELLS {
        if rows > 1 {
            rows -= 1;
            cols = cols_for(rows).min(MAX_CELLS / rows).max(1);
        } else {
            cols -= 1;
        }
    }
    let cell = (((w - (cols as f32 - 1.0) * m.gap) / cols as f32).floor(), cell_h(rows));
    let used = (cols as f32 * cell.0 + (cols as f32 - 1.0) * m.gap, rows as f32 * cell.1 + (rows as f32 - 1.0) * m.gap);
    let origin = (((w - used.0) / 2.0).floor(), top + ((avail - used.1) / 2.0).floor());
    Grid { cols, rows, cell, gap: m.gap, origin, heading: head, style, axis: if rows == 1 { Axis::Horizontal } else { Axis::Vertical }, content, peek: 0.0 }
}

/// One item in the grid: its place in the list, the size it has here and its first cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub index: usize,
    pub size: Size,
    pub col: usize,
    pub row: usize,
}

impl Placed {
    fn span(&self) -> (usize, usize) {
        self.size.cells()
    }
}

/// Every item placed, in the order of the list, each in the first place it fits.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub grid: Grid,
    pub placed: Vec<Placed>,
    /// Columns and rows the items fill.
    pub used: (usize, usize),
}

/// The size an item of `want` has in `grid`: a large one needs two rows and two columns, a wide one two columns.
pub fn fit(want: Size, grid: &Grid) -> Size {
    match want {
        Size::Large if grid.rows >= 2 && grid.cols >= 2 => Size::Large,
        Size::Large | Size::Wide if grid.cols >= 2 => Size::Wide,
        _ => Size::Small,
    }
}

pub fn plan(grid: Grid, wanted: &[Size]) -> Plan {
    let mut placed = Vec::with_capacity(wanted.len());
    let mut used = (0usize, 0usize);
    match grid.axis {
        Axis::Horizontal => {
            let mut col = 0;
            for (index, want) in wanted.iter().enumerate() {
                let size = fit(*want, &grid);
                placed.push(Placed { index, size, col, row: 0 });
                col += size.cells().0;
            }
            used = (col, usize::from(!wanted.is_empty()));
        }
        Axis::Vertical => {
            // the first place that is free, scanning the rows across: a small one fills what a wide one left
            let mut taken: Vec<Vec<bool>> = Vec::new();
            for (index, want) in wanted.iter().enumerate() {
                let size = fit(*want, &grid);
                let (w, h) = size.cells();
                let mut row = 0;
                'find: loop {
                    while taken.len() < row + h {
                        taken.push(vec![false; grid.cols]);
                    }
                    for col in 0..=grid.cols - w {
                        if (row..row + h).all(|r| (col..col + w).all(|c| !taken[r][c])) {
                            for line in taken.iter_mut().skip(row).take(h) {
                                for cell in line.iter_mut().skip(col).take(w) {
                                    *cell = true;
                                }
                            }
                            placed.push(Placed { index, size, col, row });
                            used = (used.0.max(col + w), used.1.max(row + h));
                            break 'find;
                        }
                    }
                    row += 1;
                }
            }
        }
    }
    Plan { grid, placed, used }
}

/// The plan for a content box of `w` by `h` and the sizes the owner wants: on the grid that holds every item whole when it can, else on the
/// grid that scrolls and keeps room for a peek of the next stride (the cells are a little smaller for it).
pub fn plan_for(w: f32, h: f32, heading: bool, m: &Metrics, wanted: &[Size]) -> Plan {
    let plain = plan(grid(w, h, heading, m), wanted);
    if plain.max_offset() == 0 {
        return plain;
    }
    let peeking = plan(grid_peeking(w, h, heading, m), wanted);
    if peeking.grid.peek > 0.0 { peeking } else { plain }
}

impl Plan {
    /// What the view shows of the next stride past the last whole one: nothing for a plan that holds all its items.
    pub fn peek(&self) -> f32 {
        if self.max_offset() > 0 { self.grid.peek } else { 0.0 }
    }

    /// The first stride of the view that shows the item `at` (an index into `placed`) whole and moves the view least from `off`.
    pub fn offset_for(&self, at: usize, off: usize) -> usize {
        let Some(p) = self.placed.get(at) else { return off.min(self.max_offset()) };
        let (w, h) = p.span();
        let (first, len, view) = match self.grid.axis {
            Axis::Horizontal => (p.col, w, self.grid.cols),
            Axis::Vertical => (p.row, h, self.grid.rows),
        };
        let off = if first < off {
            first
        } else if first + len > off + view {
            first + len - view
        } else {
            off
        };
        off.min(self.max_offset())
    }

    /// The last first stride the view may have.
    pub fn max_offset(&self) -> usize {
        match self.grid.axis {
            Axis::Horizontal => self.used.0.saturating_sub(self.grid.cols),
            Axis::Vertical => self.used.1.saturating_sub(self.grid.rows),
        }
    }

    /// The box of the item `at` in the content box, the view at `off`.
    pub fn rect(&self, at: usize, off: usize) -> Option<Frame> {
        let p = self.placed.get(at)?;
        let (w, h) = p.span();
        Some(self.grid.rect(p.col, p.row, w, h, off))
    }

    /// The items that are in view at `off`, and the stride on each side of it that peeks in.
    pub fn window(&self, off: usize) -> Vec<usize> {
        let (lo, hi) = (off.saturating_sub(PEEK), off + self.grid.visible() + PEEK);
        (0..self.placed.len())
            .filter(|&i| {
                let p = &self.placed[i];
                let (w, h) = p.span();
                let (first, len) = match self.grid.axis {
                    Axis::Horizontal => (p.col, w),
                    Axis::Vertical => (p.row, h),
                };
                first < hi && first + len > lo
            })
            .collect()
    }

    /// The item in view at `off` that holds the point (`x`, `y`) of the content box.
    pub fn hit(&self, x: f32, y: f32, off: usize) -> Option<usize> {
        let view = self.grid.bounds();
        if !view.contains(x, y) {
            return None;
        }
        self.window(off).into_iter().find(|&i| self.rect(i, off).is_some_and(|r| r.contains(x, y)))
    }

    /// The item a D-pad key goes to from the item `from`: the nearest one that lies that way (strictly beyond it), by how far and then by how
    /// far off to the side. None when nothing does: the focus holds at the edge, it does not wrap.
    pub fn step(&self, from: usize, dir: Dir) -> Option<usize> {
        let a = self.placed.get(from)?;
        let (aw, ah) = a.span();
        // how far apart two spans of cells are across: none when they share a cell, else one more than the empty cells between them
        let apart = |lo1: usize, hi1: usize, lo2: usize, hi2: usize| -> usize {
            if lo2 >= hi1 {
                lo2 - hi1 + 1
            } else if lo1 >= hi2 {
                lo1 - hi2 + 1
            } else {
                0
            }
        };
        let mut best: Option<((usize, usize, usize), usize)> = None;
        for (i, b) in self.placed.iter().enumerate() {
            if i == from {
                continue;
            }
            let (bw, bh) = b.span();
            let beyond = match dir {
                Dir::Right => b.col >= a.col + aw,
                Dir::Left => b.col + bw <= a.col,
                Dir::Down => b.row >= a.row + ah,
                Dir::Up => b.row + bh <= a.row,
            };
            if !beyond {
                continue;
            }
            let (along, across) = match dir {
                Dir::Right => (b.col - (a.col + aw), apart(a.row, a.row + ah, b.row, b.row + bh)),
                Dir::Left => (a.col - (b.col + bw), apart(a.row, a.row + ah, b.row, b.row + bh)),
                Dir::Down => (b.row - (a.row + ah), apart(a.col, a.col + aw, b.col, b.col + bw)),
                Dir::Up => (a.row - (b.row + bh), apart(a.col, a.col + aw, b.col, b.col + bw)),
            };
            // a Left or Right never leaves the rows it is on, as the focus engine's end of a row holds
            if matches!(dir, Dir::Left | Dir::Right) && across > 0 {
                continue;
            }
            // the first in reading order among equals
            let key = (along, across, i);
            if best.is_none_or(|(k, _)| key < k) {
                best = Some((key, i));
            }
        }
        best.map(|(_, i)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixed(n: usize) -> Vec<Size> {
        [Size::Small, Size::Wide, Size::Small, Size::Large, Size::Small, Size::Small, Size::Wide, Size::Small, Size::Large, Size::Small, Size::Wide, Size::Small].iter().copied().cycle().take(n).collect()
    }

    /// The content box of a card of `cols` columns and `rows` of the editor's sixteen: what the surface lays out in.
    fn content_of(cols: i64, rows: i64) -> (f32, f32) {
        let col = (1792.0 - 11.0 * 20.0) / 12.0;
        let row = 37.0;
        let w = cols as f32 * col + (cols - 1) as f32 * 20.0;
        let h = rows as f32 * row + (rows - 1) as f32 * 20.0;
        ((w - 64.0).floor(), (h - 56.0).floor())
    }

    #[test]
    fn a_grid_is_whole_pixels_with_equal_gutters_and_the_leftover_split_at_the_ends() {
        let m = Metrics::nominal(1.0);
        let g = grid(822.0, 209.0, true, &m);
        assert_eq!(g.cell.0, g.cell.0.floor());
        assert_eq!(g.cell.1, g.cell.1.floor());
        let used = g.cols as f32 * g.cell.0 + (g.cols as f32 - 1.0) * g.gap;
        assert!((g.origin.0 * 2.0 + used - 822.0).abs() <= 1.0, "{g:?}");
        assert!(g.heading.is_some() && g.style == Style::Stacked, "{g:?}");
        assert_eq!(g.axis, Axis::Horizontal, "one row: it scrolls along");
        let tall = grid(822.0, 437.0, true, &m);
        assert_eq!((tall.axis, tall.rows >= 2), (Axis::Vertical, true), "{tall:?}");
    }

    #[test]
    fn a_wide_item_is_two_cells_and_a_large_four_and_each_is_the_next_size_down_where_it_cannot_be() {
        let m = Metrics::nominal(1.0);
        let one_row = grid(822.0, 209.0, true, &m);
        assert_eq!((fit(Size::Large, &one_row), fit(Size::Wide, &one_row), fit(Size::Small, &one_row)), (Size::Wide, Size::Wide, Size::Small));
        let one_col = Grid { cols: 1, ..grid(822.0, 437.0, true, &m) };
        assert_eq!((fit(Size::Large, &one_col), fit(Size::Wide, &one_col)), (Size::Small, Size::Small));
        let p = plan(grid(822.0, 437.0, true, &m), &[Size::Large, Size::Small, Size::Small]);
        let g = p.grid;
        let r = p.rect(0, 0).unwrap();
        assert_eq!((r.w, r.h), (2.0 * g.cell.0 + g.gap, 2.0 * g.cell.1 + g.gap));
    }

    #[test]
    fn the_smallest_card_is_a_row_of_compact_cells_and_a_taller_one_has_a_heading() {
        let m = Metrics::nominal(1.0);
        let (w, h) = content_of(3, 3);
        let g = grid(w, h, true, &m);
        assert_eq!((g.heading, g.style, g.rows), (None, Style::Compact, 1), "{g:?} in {w} by {h}");
        let (w, h) = content_of(6, 5);
        let g = grid(w, h, true, &m);
        assert!(g.heading.is_some(), "{g:?}");
    }

    /// Every card size the editor offers, at both text sizes, with a mixed list: every item is whole inside the box, none overlaps another, the
    /// gutters are equal, every item can be reached with the keys, and the scroll that shows the one in front shows all of it.
    #[test]
    fn every_card_size_places_a_mixed_list_whole_without_overlap_and_every_item_can_be_reached() {
        for scale in [1.0f32, 2.0] {
            let m = Metrics::nominal(scale);
            for cols in [3i64, 4, 6, 8] {
                for rows in [3i64, 4, 5, 7, 9, 12, 16] {
                    for heading in [true, false] {
                        let (w, h) = content_of(cols, rows);
                        let g = grid(w, h, heading, &m);
                        let tag = format!("scale {scale} card {cols}x{rows} heading {heading}: {g:?}");
                        assert!(g.cols * g.rows <= MAX_CELLS && g.cols >= 1 && g.rows >= 1, "{tag}");
                        let bounds = g.bounds();
                        assert!(bounds.x >= 0.0 && bounds.y >= g.heading.unwrap_or(0.0) && bounds.right() <= w && bounds.bottom() <= h, "{tag} bounds {bounds:?}");
                        assert!(g.cell.0 >= 1.0 && g.cell.1 >= 1.0, "{tag}");
                        for n in [0usize, 1, 3, 7, 12, 30] {
                            let wanted = mixed(n);
                            let p = plan(g, &wanted);
                            assert_eq!(p.placed.len(), n, "{tag}");
                            // whole, inside the box, no two overlapping: cells never cross
                            let mut taken = std::collections::HashSet::new();
                            for (i, pl) in p.placed.iter().enumerate() {
                                let (cw, ch) = pl.size.cells();
                                assert!(pl.size.cells().0 <= wanted[i].cells().0 && pl.size.cells().1 <= wanted[i].cells().1, "{tag}: never bigger than asked");
                                if g.axis == Axis::Vertical {
                                    assert!(pl.col + cw <= g.cols, "{tag}: {pl:?}");
                                }
                                for c in pl.col..pl.col + cw {
                                    for r in pl.row..pl.row + ch {
                                        assert!(taken.insert((c, r)), "{tag}: cell {c},{r} twice");
                                    }
                                }
                                // the stride of the view that holds it shows it whole
                                let off = p.offset_for(i, 0);
                                let r = p.rect(i, off).unwrap();
                                let view = g.bounds();
                                assert!(r.x >= view.x - 0.01 && r.y >= view.y - 0.01 && r.right() <= view.right() + 0.01 && r.bottom() <= view.bottom() + 0.01, "{tag}: item {i} {r:?} outside {view:?} at {off}");
                                assert_eq!(r.w, r.w.floor(), "{tag}");
                            }
                            // reachable: from the first, by the keys, every item is found
                            let mut seen = vec![false; n];
                            let mut stack = if n > 0 { vec![0usize] } else { vec![] };
                            while let Some(i) = stack.pop() {
                                if std::mem::replace(&mut seen[i], true) {
                                    continue;
                                }
                                for d in Dir::ALL {
                                    if let Some(j) = p.step(i, d) {
                                        stack.push(j);
                                    }
                                }
                            }
                            assert!(seen.iter().all(|s| *s), "{tag}: with {n} items, unreachable {:?}", seen.iter().enumerate().filter(|(_, s)| !**s).map(|(i, _)| i).collect::<Vec<_>>());
                            // the window never holds more than the layers allow
                            for off in 0..=p.max_offset() {
                                assert!(p.window(off).len() <= 32, "{tag}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn keys_move_to_the_nearest_item_that_way_and_hold_at_the_edge() {
        let m = Metrics::nominal(1.0);
        let g = Grid { cols: 3, rows: 2, axis: Axis::Vertical, ..grid(822.0, 437.0, true, &m) };
        // a wide one first, then three small ones, then a large one
        let p = plan(g, &[Size::Wide, Size::Small, Size::Small, Size::Small, Size::Large]);
        let at = |i: usize| (p.placed[i].col, p.placed[i].row, p.placed[i].size);
        assert_eq!((at(0), at(1), at(2), at(3)), ((0, 0, Size::Wide), (2, 0, Size::Small), (0, 1, Size::Small), (1, 1, Size::Small)));
        assert_eq!(p.step(0, Dir::Right), Some(1));
        assert_eq!(p.step(1, Dir::Left), Some(0));
        assert_eq!(p.step(1, Dir::Right), None, "the end of the row holds");
        assert_eq!(p.step(0, Dir::Down), Some(2), "the nearest below, of the two under a wide one: the left");
        assert_eq!(p.step(3, Dir::Up), Some(0));
        assert_eq!(p.step(0, Dir::Up), None);
        assert_eq!(p.step(0, Dir::Left), None);
    }

    #[test]
    fn the_view_scrolls_by_whole_strides_and_only_as_far_as_the_item_needs() {
        let m = Metrics::nominal(1.0);
        let g = grid(822.0, 209.0, true, &m);
        assert_eq!(g.axis, Axis::Horizontal);
        let p = plan(g, &[Size::Small; 12]);
        assert_eq!(p.max_offset(), 12 - g.cols);
        assert_eq!(p.offset_for(0, 0), 0);
        assert_eq!(p.offset_for(g.cols - 1, 0), 0, "the last one in view needs no scroll");
        assert_eq!(p.offset_for(g.cols, 0), 1, "one past it: one column");
        assert_eq!(p.offset_for(11, 0), 12 - g.cols);
        assert_eq!(p.offset_for(2, 5), 2, "back: only as far as it needs");
        // a window holds the view and a stride on each side
        let w = p.window(3);
        assert_eq!((w.first().copied(), w.last().copied()), (Some(2), Some(3 + g.cols)));
        // a point in an item finds it, at a scroll
        let r = p.rect(4, 2).unwrap();
        assert_eq!(p.hit(r.x + 3.0, r.y + 3.0, 2), Some(4));
        assert_eq!(p.hit(-5.0, 5.0, 2), None);
    }

    #[test]
    fn a_taller_text_size_makes_taller_cells_and_never_a_cell_too_low_for_its_words() {
        for (scale, cols, rows) in [(1.0f32, 6, 5), (2.0, 6, 5), (2.0, 3, 3), (1.0, 3, 3), (2.0, 6, 9)] {
            let m = Metrics::nominal(scale);
            let (w, h) = content_of(cols, rows);
            let g = grid(w, h, true, &m);
            let need = match g.style {
                Style::Stacked => m.stacked_h(),
                Style::Compact => m.compact_h(),
                Style::Mini => m.mini_h(),
            };
            // a card so low that even one line does not fit (a text size too large for it) is the editor's to refuse; every other has room
            if h >= m.mini_h() {
                assert!(g.cell.1 >= need, "{scale} {cols}x{rows}: {g:?} needs {need} in {h}");
            }
        }
    }

    /// A card whose items do not all fit keeps room past its last whole stride for a peek of the next one: a tile's worth that is recognised,
    /// never a sliver, inside the content box and the bleed; the cells stay whole pixels and big enough for their words. A card that holds its
    /// items has none.
    #[test]
    fn a_card_that_scrolls_shows_a_meaningful_peek_of_the_next_stride_and_one_that_does_not_shows_none() {
        for scale in [1.0f32, 2.0] {
            let m = Metrics::nominal(scale);
            for (cols, rows) in [(3i64, 3i64), (4, 4), (6, 5), (6, 9), (8, 7), (12, 16)] {
                let (w, h) = content_of(cols, rows);
                for n in [1usize, 3, 7, 12, 30] {
                    let wanted = mixed(n);
                    let tag = format!("scale {scale} card {cols}x{rows} with {n} items");
                    let p = plan_for(w, h, true, &m, &wanted);
                    let g = p.grid;
                    assert_eq!(p.placed.len(), n, "{tag}");
                    assert_eq!((g.cell.0, g.cell.1), (g.cell.0.floor(), g.cell.1.floor()), "{tag}");
                    if p.max_offset() == 0 {
                        assert_eq!(p.peek(), 0.0, "{tag}: nothing to scroll to, nothing to peek at");
                        continue;
                    }
                    let along = if g.axis == Axis::Horizontal { g.cell.0 } else { g.cell.1 };
                    let peek = p.peek();
                    if peek == 0.0 {
                        // no room to spare for it: the cells stay as plain as the plain grid makes them
                        assert_eq!(g, grid(w, h, true, &m), "{tag}");
                        continue;
                    }
                    assert!(peek >= (along * 0.3).floor() && peek <= along, "{tag}: a tile's worth, got {peek} of {along}");
                    let end = match g.axis {
                        Axis::Horizontal => g.bounds().right() + g.gap + peek,
                        Axis::Vertical => g.bounds().bottom() + g.gap + peek,
                    };
                    let room = if g.axis == Axis::Horizontal { w } else { h };
                    assert!(end <= room + BLEED + 0.01, "{tag}: the peek ends at {end}, the box at {room} and the bleed {BLEED}");
                    let (least_w, least_h) = least(g.style, &m);
                    assert!(g.cell.0 >= least_w && g.cell.1 >= least_h, "{tag}: {g:?}");
                }
            }
        }
    }

    /// A card of one row (a large text size in a card that is not tall) scrolls sideways and shows the next item at its right, made but not
    /// something that can be pressed.
    #[test]
    fn a_single_row_card_scrolls_sideways_with_a_peek_to_the_right() {
        let m = Metrics::nominal(2.0);
        let (w, h) = content_of(6, 5);
        let p = plan_for(w, h, true, &m, &mixed(12));
        assert_eq!(p.grid.axis, Axis::Horizontal, "{:?}", p.grid);
        assert!(p.peek() > 0.0, "{:?}", p.grid);
        // the item after the last whole one is in the window, so that it is drawn behind the fade
        let last = p.grid.cols;
        assert!(p.window(0).iter().any(|&i| p.placed[i].col == last), "the next column is made");
        // it is outside the cells that can be pressed
        let next = p.rect(p.window(0).into_iter().find(|&i| p.placed[i].col == last).unwrap(), 0).unwrap();
        assert!(next.x >= p.grid.bounds().right(), "{next:?} {:?}", p.grid.bounds());
        assert_eq!(p.hit(next.x + 2.0, next.y + 2.0, 0), None, "a peek is not pressed");
    }
}
