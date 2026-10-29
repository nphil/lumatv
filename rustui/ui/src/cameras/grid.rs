//! The grid of live stills (`docs/research/cameras-ui-spec.md`, "The grid"): where the tiles go, how the D-pad moves between them and
//! the header's buttons, which rows show, and what a tile says while its picture is not fresh. Pure and in whole pixels, so the
//! spec's test vectors are the host tests.
//!
//! Columns by count: 1 to 4 across (1, 2, 2, 2, 3, 3, then 4). At most two rows show, so at most 8 stills are asked for. A tile is as
//! wide as its columns allow and no taller than its rows allow with 44 px of name under it, 16:9, an even size, gap 24, the block
//! centred in the room. More than 8 cameras scroll by whole rows, only as far as the focus needs.

use super::data::LinkState;
use super::words;

/// The gap between tiles.
pub const GAP: i32 = 24;
/// The name under a tile.
pub const NAME_H: i32 = 44;
/// The most rows that show at once.
pub const ROWS_SHOWN: usize = 2;

/// The room the block of tiles is centred in: inside the safe area, under the header and over the foot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Room {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The header's height at a text size: the title's line, never under the target.
pub fn header_height(text_scale: f32) -> i32 {
    64.max((40.0 * text_scale.max(1.0) * 1.25).round() as i32)
}

/// The foot's height (the hints' row) at a text size.
pub fn foot_height(text_scale: f32) -> i32 {
    (40.0 * text_scale.max(1.0)).round() as i32
}

/// The banner under the header (one line of the state's words) and the gap after it.
pub fn banner_height(text_scale: f32) -> i32 {
    64.max((26.0 * text_scale.max(1.0) * 1.3).round() as i32) + 16
}

/// The room of the block of tiles at a text size: x 96 to 1824, under the header (at y 54) with a gap of 32 and over the foot with a
/// gap of 20, so y 150 to 966 at 100 %; a banner takes its height off the top.
pub fn room(text_scale: f32, banner: bool) -> Room {
    let top = 54 + header_height(text_scale) + 32 + if banner { banner_height(text_scale) } else { 0 };
    let bottom = 1026 - foot_height(text_scale) - 20;
    Room { x: 96, y: top, w: 1728, h: (bottom - top).max(NAME_H + 2) }
}

/// Columns by count: 1, 2, 2, 2, 3, 3, then 4.
pub fn columns(n: usize) -> usize {
    match n {
        0 | 1 => 1,
        2..=4 => 2,
        5 | 6 => 3,
        _ => 4,
    }
}

/// Where the tiles go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub count: usize,
    pub cols: usize,
    pub rows_total: usize,
    pub rows_shown: usize,
    pub tile_w: i32,
    pub tile_h: i32,
    /// The left of the first column and the top of the first shown row.
    pub x0: i32,
    pub y0: i32,
}

/// JavaScript's `Math.round`: halves go up.
fn round_half_up(v: f32) -> i32 {
    (v + 0.5).floor() as i32
}

fn even_down(v: i32) -> i32 {
    v - v.rem_euclid(2)
}

/// The grid of `n` tiles in `room`.
pub fn layout(n: usize, room: Room) -> Layout {
    let cols = columns(n);
    let rows_total = n.div_ceil(cols).max(1);
    let rows_shown = rows_total.min(ROWS_SHOWN);
    let by_width = (room.w - (cols as i32 - 1) * GAP) / cols as i32;
    let row_h = (room.h - (rows_shown as i32 - 1) * GAP) / rows_shown as i32 - NAME_H;
    let by_height = (row_h as f32 * 16.0 / 9.0).floor() as i32;
    let tile_w = even_down(by_width.min(by_height)).max(2);
    let tile_h = round_half_up(tile_w as f32 * 9.0 / 16.0 / 2.0) * 2;
    let block_w = cols as i32 * tile_w + (cols as i32 - 1) * GAP;
    let block_h = rows_shown as i32 * (tile_h + NAME_H) + (rows_shown as i32 - 1) * GAP;
    Layout {
        count: n,
        cols,
        rows_total,
        rows_shown,
        tile_w,
        tile_h,
        x0: room.x + round_half_up((room.w - block_w) as f32 / 2.0),
        y0: room.y + round_half_up((room.h - block_h) as f32 / 2.0),
    }
}

impl Layout {
    /// The vertical distance between the tops of two rows.
    pub fn pitch(&self) -> i32 {
        self.tile_h + NAME_H + GAP
    }

    pub fn row_of(&self, i: usize) -> usize {
        i / self.cols
    }

    pub fn col_of(&self, i: usize) -> usize {
        i % self.cols
    }

    /// The top left of tile `i` when the first shown row is row 0 of the content (its place in the scrolled content: add the row's
    /// pitch for a row further down).
    pub fn tile_at(&self, i: usize) -> (i32, i32) {
        (self.x0 + self.col_of(i) as i32 * (self.tile_w + GAP), self.y0 + self.row_of(i) as i32 * self.pitch())
    }

    /// The first shown row that keeps `focus` in view, from `top`: scrolls by whole rows and only as far as the focus needs.
    pub fn top_for(&self, top: usize, focus: usize) -> usize {
        let row = self.row_of(focus);
        let max_top = self.rows_total.saturating_sub(self.rows_shown);
        let top = top.min(max_top);
        if row < top {
            row
        } else if row >= top + self.rows_shown {
            (row + 1 - self.rows_shown).min(max_top)
        } else {
            top
        }
    }

    /// The tiles of the rows from `top` that show.
    pub fn shown(&self, top: usize) -> std::ops::Range<usize> {
        let first = top * self.cols;
        let last = ((top + self.rows_shown) * self.cols).min(self.count);
        first.min(self.count)..last
    }

    /// 1-based numbers of the first and last camera below the shown rows, when there are any: "9 to 12".
    pub fn below(&self, top: usize) -> Option<(usize, usize)> {
        let after = (top + self.rows_shown) * self.cols;
        (after < self.count).then_some((after + 1, self.count))
    }

    /// 1-based numbers of the cameras above the shown rows, when there are any.
    pub fn above(&self, top: usize) -> Option<(usize, usize)> {
        (top > 0).then_some((1, top * self.cols))
    }
}

/// Where the focus is in the grid: a tile or one of the header's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Tile(usize),
    /// The `Events` button of the header.
    Events,
    /// The `Settings` button of the header.
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// A D-pad key in the grid: Left and Right along a row without wrapping, Up and Down by rows; Down from the last tile of a longer row
/// above a short last row lands on the last tile; Up from the top row moves to the header's buttons (the last column to `Settings`, the
/// rest to `Events`), Left and Right between them, Down back to the tile that was left (`back_to`).
pub fn step(layout: &Layout, from: Focus, dir: Dir, back_to: usize) -> Focus {
    let n = layout.count;
    match from {
        Focus::Tile(i) => {
            let (row, col) = (layout.row_of(i), layout.col_of(i));
            match dir {
                Dir::Left if col > 0 => Focus::Tile(i - 1),
                Dir::Right if col + 1 < layout.cols && i + 1 < n => Focus::Tile(i + 1),
                Dir::Up if row > 0 => Focus::Tile(i - layout.cols),
                Dir::Up => {
                    if col + 1 == layout.cols && layout.cols > 1 || n == 1 {
                        Focus::Settings
                    } else {
                        Focus::Events
                    }
                }
                Dir::Down if i + layout.cols < n => Focus::Tile(i + layout.cols),
                Dir::Down if layout.row_of(n - 1) > row => Focus::Tile(n - 1),
                _ => from,
            }
        }
        Focus::Events => match dir {
            Dir::Right => Focus::Settings,
            Dir::Down if n > 0 => Focus::Tile(back_to.min(n - 1)),
            _ => from,
        },
        Focus::Settings => match dir {
            Dir::Left => Focus::Events,
            Dir::Down if n > 0 => Focus::Tile(back_to.min(n - 1)),
            _ => from,
        },
    }
}

/// What a picture is doing for one tile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pic {
    /// When the newest picture arrived (milliseconds on the model's clock), if any has.
    pub last_at: Option<u64>,
    /// Asks refused in a row since the last picture.
    pub refused: u32,
}

/// A picture is fresh for this long.
pub const FRESH_MS: u64 = 12_000;

/// What a tile says over its picture (centred, up to two lines) and whether its picture is dimmed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TileWords {
    pub lines: Vec<String>,
    pub dim: bool,
}

/// What tile words say (spec "Tile words"): nothing while the picture is fresh and the link is `ready`; `Connecting` before the
/// first; `No picture` after two refused asks; a camera that is not online "Offline" (with its age past a minute); any other state of
/// the link, or a picture older than 12 s, dims the tile and gives its age.
pub fn tile_words(online: bool, link: LinkState, pic: Pic, now: u64) -> TileWords {
    let age = pic.last_at.map(|t| now.saturating_sub(t));
    if !online {
        let mut lines = vec![words::OFFLINE.to_string()];
        if let Some(a) = age.filter(|a| *a >= 60_000) {
            lines.push(words::age(a as i64));
        }
        return TileWords { lines, dim: pic.last_at.is_some() };
    }
    match (link, age) {
        (LinkState::Ready, Some(a)) if a < FRESH_MS => TileWords::default(),
        (_, Some(a)) => TileWords { lines: vec![words::age(a as i64)], dim: true },
        (LinkState::Ready, None) => {
            let word = if pic.refused >= 2 { words::NO_PICTURE } else { words::CONNECTING };
            TileWords { lines: vec![word.to_string()], dim: false }
        }
        (_, None) => TileWords { lines: vec![words::NO_PICTURE.to_string()], dim: false },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The room of the spec's test vectors: x 96 to 1824, y 150 to 966 at 100 % text.
    const ROOM: Room = Room { x: 96, y: 150, w: 1728, h: 816 };

    fn dims(n: usize) -> (usize, usize, i32, i32, i32, i32) {
        let l = layout(n, ROOM);
        (l.cols, l.rows_shown, l.tile_w, l.tile_h, l.x0, l.y0)
    }

    #[test]
    fn the_room_is_the_specs_at_100_percent_and_shrinks_with_larger_type_and_a_banner() {
        assert_eq!(room(1.0, false), ROOM);
        let big = room(2.0, false);
        assert_eq!((big.y, big.y + big.h), (186, 926), "the header's height at larger type moves y");
        let banner = room(1.0, true);
        assert_eq!((banner.y, banner.y + banner.h), (150 + 80, 966));
        assert!(layout(4, banner).tile_w < layout(4, ROOM).tile_w, "four cameras are limited by the room's height");
        assert_eq!(layout(8, banner).tile_w, layout(8, ROOM).tile_w, "eight are limited by its width");
    }

    #[test]
    fn the_columns_follow_the_count() {
        let cols: Vec<usize> = (0..=9).map(columns).collect();
        assert_eq!(cols, [1, 1, 2, 2, 2, 3, 3, 4, 4, 4]);
        assert_eq!(columns(32), 4);
    }

    #[test]
    fn the_specs_test_vectors_hold() {
        assert_eq!(dims(1), (1, 1, 1372, 772, 274, 150), "1 camera 1x1 at 1372x772 (x 274)");
        assert_eq!(dims(2), (2, 1, 852, 480, 96, 296), "2: 2x1 at 852x480 (x 96, y 296)");
        assert_eq!(dims(3), (2, 2, 624, 352, 324, 150), "3 and 4: 2x2 at 624x352 (x 324)");
        assert_eq!(dims(4), (2, 2, 624, 352, 324, 150));
        assert_eq!(dims(5), (3, 2, 560, 316, 96, 186), "5 and 6: 3x2 at 560x316 (y 186)");
        assert_eq!(dims(6), (3, 2, 560, 316, 96, 186));
        assert_eq!(dims(7), (4, 2, 414, 232, 96, 270), "7 and 8: 4x2 at 414x232 (y 270)");
        assert_eq!(dims(8), (4, 2, 414, 232, 96, 270));
        let l = layout(12, ROOM);
        assert_eq!((l.cols, l.rows_total, l.rows_shown, l.tile_w, l.tile_h), (4, 3, 2, 414, 232), "12: 4 across, 3 rows, two shown");
    }

    #[test]
    fn the_block_stays_inside_the_room_and_the_tiles_are_sixteen_by_nine_and_even() {
        for n in 1..=32 {
            let l = layout(n, ROOM);
            assert_eq!((l.tile_w % 2, l.tile_h % 2), (0, 0), "{n}: even");
            let r = l.tile_w as f32 / l.tile_h as f32;
            assert!((r - 16.0 / 9.0).abs() < 0.03, "{n}: {r}");
            let block_w = l.cols as i32 * l.tile_w + (l.cols as i32 - 1) * GAP;
            let block_h = l.rows_shown as i32 * (l.tile_h + NAME_H) + (l.rows_shown as i32 - 1) * GAP;
            assert!(l.x0 >= ROOM.x && l.x0 + block_w <= ROOM.x + ROOM.w, "{n}: across");
            assert!(l.y0 >= ROOM.y && l.y0 + block_h <= ROOM.y + ROOM.h, "{n}: down");
        }
    }

    #[test]
    fn a_larger_header_and_foot_shrink_the_room_and_the_tiles_with_it() {
        let big = Room { x: 96, y: 186, w: 1728, h: 740 };
        let l = layout(4, big);
        assert!(l.tile_w < 624 && l.y0 >= big.y && l.y0 + 2 * (l.tile_h + NAME_H) + GAP <= big.y + big.h);
    }

    #[test]
    fn tiles_sit_at_their_column_and_row_and_a_row_down_is_one_pitch() {
        let l = layout(8, ROOM);
        assert_eq!(l.tile_at(0), (96, 270));
        assert_eq!(l.tile_at(1), (96 + 414 + 24, 270));
        assert_eq!(l.tile_at(4), (96, 270 + 232 + 44 + 24));
        assert_eq!(l.pitch(), 300);
    }

    #[test]
    fn the_grid_scrolls_by_whole_rows_only_as_far_as_the_focus_needs() {
        let l = layout(12, ROOM);
        assert_eq!(l.top_for(0, 3), 0, "the second row shows");
        assert_eq!(l.top_for(0, 8), 1, "the third row needs one row of scroll");
        assert_eq!(l.top_for(1, 5), 1, "inside the window: no scroll");
        assert_eq!(l.top_for(1, 2), 0, "back up to the first row");
        assert_eq!(l.top_for(5, 11), 1, "never past the last row");
        assert_eq!(l.shown(0), 0..8);
        assert_eq!(l.shown(1), 4..12);
        let big = layout(14, ROOM);
        assert_eq!(big.shown(2), 8..14, "a short last row");
        assert_eq!(l.below(0), Some((9, 12)), "the foot's left says the range left below");
        assert_eq!(l.below(1), None);
        assert_eq!(l.above(1), Some((1, 4)));
        assert_eq!(l.above(0), None);
    }

    fn walk(layout: &Layout, start: Focus, keys: &[Dir]) -> Focus {
        keys.iter().fold(start, |f, d| step(layout, f, *d, 0))
    }

    #[test]
    fn left_and_right_move_along_a_row_and_never_wrap() {
        let l = layout(8, ROOM);
        assert_eq!(walk(&l, Focus::Tile(0), &[Dir::Left]), Focus::Tile(0));
        assert_eq!(walk(&l, Focus::Tile(0), &[Dir::Right, Dir::Right, Dir::Right]), Focus::Tile(3));
        assert_eq!(walk(&l, Focus::Tile(3), &[Dir::Right]), Focus::Tile(3), "the end of the row holds");
        assert_eq!(walk(&l, Focus::Tile(4), &[Dir::Left]), Focus::Tile(4), "and so does the start of the next one");
    }

    #[test]
    fn up_and_down_move_by_rows_and_a_short_last_row_is_landed_on_its_last_tile() {
        let three = layout(3, ROOM);
        assert_eq!(walk(&three, Focus::Tile(0), &[Dir::Down]), Focus::Tile(2));
        assert_eq!(walk(&three, Focus::Tile(0), &[Dir::Right, Dir::Down]), Focus::Tile(2), "right, down lands on the lone tile of the row below");
        assert_eq!(walk(&three, Focus::Tile(0), &[Dir::Right, Dir::Down, Dir::Left]), Focus::Tile(2), "and left stays");
        assert_eq!(walk(&three, Focus::Tile(2), &[Dir::Up]), Focus::Tile(0));
        let five = layout(5, ROOM);
        assert_eq!(walk(&five, Focus::Tile(2), &[Dir::Down]), Focus::Tile(4), "from the last of a longer row");
        assert_eq!(walk(&five, Focus::Tile(4), &[Dir::Down]), Focus::Tile(4), "nothing below the last row");
        let eight = layout(8, ROOM);
        assert_eq!(walk(&eight, Focus::Tile(1), &[Dir::Down]), Focus::Tile(5));
    }

    #[test]
    fn up_from_the_top_row_goes_to_the_headers_buttons_and_down_comes_back() {
        let l = layout(8, ROOM);
        assert_eq!(walk(&l, Focus::Tile(1), &[Dir::Up]), Focus::Events);
        assert_eq!(walk(&l, Focus::Tile(3), &[Dir::Up]), Focus::Settings, "the last column is under the right button");
        assert_eq!(walk(&l, Focus::Events, &[Dir::Right]), Focus::Settings);
        assert_eq!(walk(&l, Focus::Settings, &[Dir::Left]), Focus::Events);
        assert_eq!(walk(&l, Focus::Settings, &[Dir::Right, Dir::Up]), Focus::Settings, "the header is the top");
        assert_eq!(step(&l, Focus::Events, Dir::Down, 2), Focus::Tile(2), "Down goes back to the tile that was left");
        assert_eq!(step(&l, Focus::Settings, Dir::Down, 99), Focus::Tile(7), "never past the last tile");
        let one = layout(1, ROOM);
        assert_eq!(walk(&one, Focus::Tile(0), &[Dir::Up]), Focus::Settings);
    }

    #[test]
    fn a_tile_says_nothing_while_its_picture_is_fresh_and_the_link_is_ready() {
        let fresh = Pic { last_at: Some(95_000), refused: 0 };
        assert_eq!(tile_words(true, LinkState::Ready, fresh, 100_000), TileWords::default());
        let stale = tile_words(true, LinkState::Ready, fresh, 95_000 + 13_000);
        assert_eq!((stale.lines, stale.dim), (vec!["13 s ago".to_string()], true), "older than 12 s: dimmed with its age");
    }

    #[test]
    fn a_tile_with_no_picture_yet_says_connecting_then_no_picture_after_two_refusals() {
        let none = Pic::default();
        assert_eq!(tile_words(true, LinkState::Ready, none, 0).lines, ["Connecting"]);
        assert_eq!(tile_words(true, LinkState::Ready, Pic { last_at: None, refused: 1 }, 0).lines, ["Connecting"]);
        assert_eq!(tile_words(true, LinkState::Ready, Pic { last_at: None, refused: 2 }, 0).lines, ["No picture"]);
        assert_eq!(tile_words(true, LinkState::Offline, none, 0).lines, ["No picture"], "no link and no picture: nothing is made up");
    }

    #[test]
    fn an_offline_camera_says_offline_with_its_age_past_a_minute() {
        let pic = Pic { last_at: Some(0), refused: 0 };
        let t = tile_words(false, LinkState::Ready, pic, 30_000);
        assert_eq!((t.lines, t.dim), (vec!["Offline".to_string()], true));
        let t = tile_words(false, LinkState::Ready, pic, 3 * 60_000);
        assert_eq!(t.lines, ["Offline", "3 min ago"]);
        assert!(!tile_words(false, LinkState::Ready, Pic::default(), 0).dim, "nothing to dim");
    }

    #[test]
    fn any_other_state_of_the_link_dims_the_last_picture_and_gives_its_age() {
        let pic = Pic { last_at: Some(0), refused: 0 };
        for state in [LinkState::Offline, LinkState::Connecting, LinkState::Refused, LinkState::Untrusted] {
            let t = tile_words(true, state, pic, 4 * 60_000);
            assert_eq!((t.lines, t.dim), (vec!["4 min ago".to_string()], true), "{state:?}");
        }
    }
}
