//! Where things go: the sizes of the TV profile at the person's text size, the height and the parts of every kind of row
//! (`LuRow`'s rules: a name and its second line, a value, a control at the right), the rows laid in a column, and the scroll rule of
//! `PageList` (whole rows, the earliest first row that still leaves the focused row whole). Pure but for the font measure, which the
//! kit's fonts give, so it is tested with the test kit and no GL.

use crate::kit::Kit;
use crate::settings::row::{Kind, Row};
use lumatv_tokens::TypeRole;
use ui::components::text::{line_height, wrap_to};

/// The profile's sizes (`Lu.target`, `Lu.row`, `Lu.space(n)`) and the ones derived from them.
#[derive(Clone, Copy, Debug)]
pub struct Sizes {
    pub row: f32,
    pub target: f32,
    pub tile: f32,
    pub glyph: u32,
    pub s1: f32,
    pub s2: f32,
    pub s3: f32,
    pub s4: f32,
    pub s5: f32,
    pub s6: f32,
    pub s7: f32,
}

impl Sizes {
    pub fn of(kit: &Kit) -> Self {
        let t = kit.tokens.target;
        let tile = (t * 0.75).round();
        Sizes {
            row: kit.tokens.row,
            target: t,
            tile,
            glyph: (tile * 0.55).round() as u32,
            s1: kit.space(1),
            s2: kit.space(2),
            s3: kit.space(3),
            s4: kit.space(4),
            s5: kit.space(5),
            s6: kit.space(6),
            s7: kit.space(7),
        }
    }

    /// The control a toggle is: a track and its thumb.
    pub fn toggle(&self) -> (f32, f32) {
        let h = (self.target * 0.75).round();
        ((h * 1.75).round(), h)
    }

    /// The side of a chevron or a check in a row.
    pub fn chevron(&self) -> f32 {
        (self.target * 0.5).round()
    }
}

/// A segmented control: the width of a segment, how many, how many show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegGeom {
    pub seg_w: f32,
    pub n: usize,
    pub h: f32,
    pub in_view: usize,
}

impl SegGeom {
    pub fn total(&self) -> f32 {
        if self.in_view < self.n { self.in_view as f32 * self.seg_w } else { self.n as f32 * self.seg_w }
    }

    /// Which segment is at `x` from the control's left edge, given the first one in view.
    pub fn at(&self, x: f32, first: usize) -> Option<usize> {
        if x < 0.0 {
            return None;
        }
        let i = first + (x / self.seg_w) as usize;
        (i < self.n && i < first + self.in_view).then_some(i)
    }
}

/// A row of cards: the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardsGeom {
    pub edge: f32,
    pub gap: f32,
    pub card_w: f32,
    pub card_h: f32,
    pub across: usize,
    pub rows: usize,
}

/// What one row is made of, in the row's own coordinates (the row's top left is the origin).
#[derive(Clone, Debug)]
pub struct RowGeom {
    pub h: f32,
    pub label: Vec<String>,
    pub detail: Vec<String>,
    /// Where the words begin, and how much room they have before the value and the control.
    pub text_x: f32,
    pub text_w: f32,
    /// The control at the right: its width, and where the value text ends (right edge).
    pub trail_w: f32,
    pub value: String,
    pub value_w: f32,
    pub seg: Option<SegGeom>,
    pub cards: Option<CardsGeom>,
    pub chip_w: f32,
}

/// The words of a row broken to its room, a label in at most two lines and a second line in at most two.
fn lines(kit: &mut Kit, text: &str, role: TypeRole, w: f32, max: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let ty = kit.ty(role);
    wrap_to(&mut kit.ui.fonts, text, ty, w.max(40.0), max)
}

fn text_w(kit: &mut Kit, text: &str, role: TypeRole) -> f32 {
    let ty = kit.ty(role);
    kit.ui.fonts.measure(text, ty).width.ceil()
}

/// The control of a segmented row or a tab bar of `options`, each segment as wide as the widest label plus the pill's air at both
/// sides and never narrower than a target.
pub fn seg_geom(kit: &mut Kit, sz: &Sizes, options: &[String], role: TypeRole, fit: Option<f32>) -> SegGeom {
    let widest = options.iter().map(|o| text_w(kit, o, role)).fold(0.0, f32::max);
    let seg_w = sz.target.max(widest + 2.0 * sz.s4 + 2.0 * sz.s1);
    let n = options.len();
    let in_view = match fit {
        Some(f) if n as f32 * seg_w > f => ((f / seg_w).floor() as usize).max(1),
        _ => n,
    };
    SegGeom { seg_w, n, h: sz.target, in_view }
}

/// The tab label and its selected mark need separate vertical space at every text scale.
pub fn tab_height(kit: &Kit, sz: &Sizes) -> f32 {
    sz.target.max(line_height(kit.ty(TypeRole::Label)) + 2.0 * sz.s2 + 2.0 * sz.s1).ceil()
}

/// Measures `row` laid out `w` wide (a page's row, a rail's row).
pub fn measure_row(kit: &mut Kit, row: &Row, w: f32) -> RowGeom {
    let sz = Sizes::of(kit);
    let line_l = kit.line(TypeRole::Label);
    let line_c = kit.line(TypeRole::Caption);
    let mut g = RowGeom { h: 0.0, label: Vec::new(), detail: Vec::new(), text_x: 0.0, text_w: 0.0, trail_w: 0.0, value: String::new(), value_w: 0.0, seg: None, cards: None, chip_w: 0.0 };
    match row.kind {
        Kind::Head => {
            let l = kit.line(TypeRole::Caption);
            g.h = (sz.row * 0.72).round().max(l + sz.s4);
            g.label = lines(kit, &row.text, TypeRole::Caption, w - 2.0 * sz.s2, 1);
            return g;
        }
        Kind::Note => {
            let x = sz.s2 + (sz.target * 0.36).round() + sz.s3;
            g.text_x = x;
            g.text_w = w - x - sz.s2;
            g.label = lines(kit, &row.text, TypeRole::Caption, g.text_w, 6);
            g.h = (g.label.len().max(1) as f32 * line_c + sz.s5).ceil();
            return g;
        }
        Kind::Empty => {
            g.text_w = w;
            g.label = lines(kit, &row.label, TypeRole::Label, w, 2);
            g.detail = lines(kit, &row.text, TypeRole::Caption, w - 2.0 * sz.s6, 3);
            g.h = (sz.s5 + 64.0 + sz.s3 + g.label.len() as f32 * line_l + if g.detail.is_empty() { 0.0 } else { sz.s3 + g.detail.len() as f32 * line_c } + sz.s5).ceil();
            return g;
        }
        Kind::Cards => {
            let two = row.cards.iter().any(|c| !c.sub.is_empty());
            let across = if row.strip { row.in_view.max(1) } else { row.cols.max(1) };
            let edge = sz.s6;
            let gap = sz.s3;
            let card_w = ((w - 2.0 * edge - gap * (across as f32 - 1.0)) / across as f32).floor();
            let card_h = if two { sz.row + sz.s2 } else { sz.row };
            let rows = if row.strip { 1 } else { row.cards.len().div_ceil(row.cols.max(1)) };
            g.cards = Some(CardsGeom { edge, gap, card_w, card_h, across, rows });
            g.h = rows as f32 * (card_h + gap);
            return g;
        }
        Kind::Figure => {
            g.h = (sz.target * 2.4 * 0.56).round() + sz.s7 + sz.s1;
            return g;
        }
        // the QR code at the left with the quiet margin in it, the code and the words beside it
        Kind::Code => {
            let side = super::code::modules(&row.text).map_or(0.0, |qr| super::code::sides(&sz, &qr).1 as f32);
            g.text_x = sz.s6 + side + sz.s7;
            g.text_w = (w - g.text_x - sz.s6).max(60.0);
            g.label = lines(kit, &row.label, TypeRole::Display, g.text_w, 1);
            g.detail = lines(kit, &row.detail, TypeRole::Body, g.text_w, 5);
            let words = kit.line(TypeRole::Display) + sz.s3 + g.detail.len() as f32 * kit.line(TypeRole::Body);
            g.h = (side.max(words) + 2.0 * sz.s4).ceil();
            return g;
        }
        _ => {}
    }
    // a row with words: a tile at the left when it has an icon or a face, the control and the value at the right
    g.text_x = sz.s6 + if row.icon.is_empty() && row.face.is_none() { 0.0 } else { sz.tile + sz.s5 };
    let (trail_w, trail_gap) = match row.kind {
        Kind::Toggle => (sz.toggle().0, 0.0),
        Kind::Seg => {
            let s = seg_geom(kit, &sz, &row.options, TypeRole::Caption, Some((w - g.text_x - sz.s6 - 3.0 * sz.target).max(2.0 * sz.target)));
            g.seg = Some(s);
            (s.total(), 0.0)
        }
        Kind::Slider => {
            let value = row.value_text.clone().unwrap_or_else(|| format!("{}{}", row.num.round() as i64, row.unit));
            let vw = (sz.target * 1.5).round().max(text_w(kit, &value, TypeRole::Label));
            g.value = value;
            g.value_w = vw;
            (sz.target * 5.0 + sz.s4 + vw, 0.0)
        }
        Kind::Value => {
            let show = row.chevron || row.selected;
            if !row.value.is_empty() {
                g.value = row.value.clone();
                g.value_w = text_w(kit, &row.value, TypeRole::Label);
                // an answer held as a chip: a glyph, the word, a pill around them
                if !row.value_icon.is_empty() {
                    g.value_w = g.value_w.max(text_w(kit, &row.value_min, TypeRole::Label));
                    g.chip_w = (sz.s3 + sz.chevron() + sz.s2 + g.value_w + sz.s3).ceil();
                }
            }
            (if show { sz.chevron() } else { 0.0 }, if show && !row.value.is_empty() { sz.s4 } else { 0.0 })
        }
        Kind::Action => {
            g.chip_w = text_w(kit, &row.button, TypeRole::Caption) + 2.0 * sz.s4;
            (g.chip_w, 0.0)
        }
        Kind::Stepper => (sz.chevron() * 2.0 + sz.target * 3.0 + 2.0 * sz.s3, 0.0),
        Kind::Nudge => {
            let side = (sz.target * 0.45).round();
            (side * 2.0 + (sz.target * 2.2).round() + 2.0 * sz.s3, 0.0)
        }
        _ => (0.0, 0.0),
    };
    g.trail_w = trail_w;
    let value_room = if g.value.is_empty() || matches!(row.kind, Kind::Slider) { 0.0 } else if g.chip_w > 0.0 { g.chip_w + sz.s4 } else { g.value_w + sz.s4 };
    let info_mark = if row.help.is_empty() { 0.0 } else { (sz.target * 0.4).round() + sz.s3 };
    let room = (w - g.text_x - sz.s6 - trail_w - trail_gap - value_room).max(60.0);
    g.text_w = room;
    g.label = lines(kit, &row.label, TypeRole::Label, room - info_mark, 2);
    g.detail = lines(kit, &row.detail, TypeRole::Caption, room, 2);
    let words = g.label.len().max(1) as f32 * line_l + g.detail.len() as f32 * line_c;
    let base = if g.detail.is_empty() { sz.row } else { sz.row + sz.s2 };
    // the control is as tall as a target, so a row with a tall one (a tray of segments) is at least that and its air
    let control = match row.kind {
        Kind::Seg | Kind::Slider => sz.target + sz.s4,
        _ => 0.0,
    };
    g.h = base.max(words + sz.s4).max(control).ceil();
    g
}

/// Rows laid in a column: the top of each and the whole height.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Column {
    pub tops: Vec<f32>,
    pub heights: Vec<f32>,
    pub total: f32,
}

impl Column {
    pub fn of(heights: &[f32]) -> Self {
        let mut tops = Vec::with_capacity(heights.len());
        let mut y = 0.0;
        for h in heights {
            tops.push(y);
            y += h;
        }
        Column { tops, heights: heights.to_vec(), total: y }
    }
}

/// The room a list keeps for its chevrons once its rows do not all fit.
pub fn zone(total: f32, viewport_h: f32, s6: f32) -> f32 {
    if total > viewport_h { s6 } else { 0.0 }
}

/// `PageList.scrollTimer`: the list scrolls by whole rows. A row above the first in view brings its own top to the top; one below
/// brings in the earliest first row that still leaves it whole.
pub fn follow(col: &Column, stop: usize, offset: f32, usable: f32) -> f32 {
    let Some(&top) = col.tops.get(stop) else { return offset };
    let bottom = top + col.heights[stop];
    if top < offset {
        top
    } else if bottom - offset > usable {
        let mut k = stop;
        while k > 0 && bottom - col.tops[k - 1] <= usable {
            k -= 1;
        }
        col.tops[k]
    } else {
        offset
    }
}

/// The rows that are whole inside `[offset, offset + usable]`.
pub fn whole_rows(col: &Column, offset: f32, usable: f32) -> std::ops::Range<usize> {
    let eps = 0.5;
    let first = col.tops.iter().position(|t| *t >= offset - eps).unwrap_or(col.tops.len());
    let mut last = first;
    while last < col.tops.len() && col.tops[last] + col.heights[last] <= offset + usable + eps {
        last += 1;
    }
    first..last
}

/// `text` (a note, a body) in lines of the role's type and its height.
pub fn text_height(kit: &mut Kit, text: &str, role: TypeRole, w: f32, max: usize) -> (Vec<String>, f32) {
    let l = lines(kit, text, role, w, max);
    let h = l.len() as f32 * line_height(kit.ty(role));
    (l, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::settings::row::Row;

    fn col(h: &[f32]) -> Column {
        Column::of(h)
    }

    #[test]
    fn the_list_scrolls_by_whole_rows_the_least_that_shows_the_focused_one() {
        // ten rows of 80, 640 usable: eight fit
        let c = col(&[80.0; 10]);
        assert_eq!(follow(&c, 3, 0.0, 640.0), 0.0, "a row in view does not scroll");
        assert_eq!(follow(&c, 7, 0.0, 640.0), 0.0, "the eighth is whole at the bottom");
        assert_eq!(follow(&c, 8, 0.0, 640.0), 80.0, "the ninth brings in one row");
        assert_eq!(follow(&c, 9, 0.0, 640.0), 160.0);
        assert_eq!(follow(&c, 1, 160.0, 640.0), 80.0, "going back up brings the focused row's top to the top");
        assert_eq!(follow(&c, 0, 160.0, 640.0), 0.0);
        // rows of different heights: the earliest first row that leaves the focused row whole
        let c = col(&[100.0, 200.0, 300.0, 200.0]);
        assert_eq!(follow(&c, 3, 0.0, 500.0), 300.0, "200 + 300 = 500 fits from row 2");
        assert_eq!(whole_rows(&c, 300.0, 500.0), 2..4);
        assert_eq!(whole_rows(&c, 0.0, 500.0), 0..2, "the third would be cut at the bottom: it is not drawn");
    }

    #[test]
    fn the_chevrons_take_room_only_once_the_rows_do_not_fit() {
        assert_eq!(zone(500.0, 691.0, 24.0), 0.0);
        assert_eq!(zone(900.0, 691.0, 24.0), 24.0);
    }

    #[test]
    fn a_plain_row_is_a_row_high_and_one_with_a_second_line_two_spaces_more() {
        let (_h, mut kit) = testkit::kit(1.0);
        let plain = measure_row(&mut kit, &Row::toggle("a").icon("eye").label("Eye comfort"), 1272.0);
        assert_eq!(plain.h, 80.0);
        assert_eq!((plain.text_x, plain.trail_w), (92.0, 84.0), "a tile and a switch");
        let two = measure_row(&mut kit, &Row::toggle("a").icon("eye").label("Eye comfort").detail("Not available now"), 1272.0);
        assert_eq!(two.h, 88.0);
        let head = measure_row(&mut kit, &Row::head("Vision", 0), 1272.0);
        assert_eq!(head.h, 58.0, "a head is under a row, with air above it that parts one group from the one before");
    }

    #[test]
    fn a_row_with_a_face_keeps_its_words_clear_of_it_as_one_with_an_icon_does() {
        let (_h, mut kit) = testkit::kit(1.0);
        let face = crate::settings::row::Face { file: Some("/x/people-03.png".into()), letter: "I".into(), colour: "#EB6F92".into() };
        let person = measure_row(&mut kit, &Row::link("p:isabel").face(face).label("Isabel"), 1272.0);
        let icon = measure_row(&mut kit, &Row::link("p:isabel").icon("user").label("Isabel"), 1272.0);
        assert_eq!(person.text_x, icon.text_x, "the name starts after the picture");
        assert_eq!(person.text_x, 92.0);
    }

    #[test]
    fn large_text_wraps_the_words_and_grows_the_row_and_never_cuts_them() {
        let (_h, mut kit) = testkit::kit(2.0);
        let r = Row::toggle("a").icon("shield").label("Content recognition & advertising services").detail("Not available right now, the helper did not answer");
        let g = measure_row(&mut kit, &r, 900.0);
        assert!(g.label.len() == 2 && !g.detail.is_empty(), "{:?} {:?}", g.label, g.detail);
        assert!(g.h > 100.0, "the row grew: {}", g.h);
        assert!(!g.label.last().unwrap().ends_with('…') || g.label.len() == 2);
    }

    #[test]
    fn a_segmented_row_keeps_its_options_whole_and_scrolls_by_segments_when_they_do_not_fit() {
        let (_h, mut kit) = testkit::kit(1.0);
        let r = Row::seg("s", &["Auto", "Off", "Minimum", "Medium", "Maximum"]).icon("leaf").label("Energy saving");
        let g = measure_row(&mut kit, &r, 1272.0);
        let s = g.seg.unwrap();
        assert!(s.seg_w >= 64.0 && s.in_view == 5, "{s:?}");
        assert_eq!(s.at(s.seg_w * 1.5, 0), Some(1));
        assert_eq!(s.at(-1.0, 0), None);
        let narrow = measure_row(&mut kit, &r, 700.0);
        assert!(narrow.seg.unwrap().in_view < 5, "too many for the room: some scroll in");
    }

    #[test]
    fn a_strip_of_cards_is_one_row_high_and_a_grid_is_as_many_as_it_needs() {
        let (_h, mut kit) = testkit::kit(1.0);
        let cards: Vec<_> = (0..9).map(|i| crate::settings::row::Card { title: format!("Mode {i}"), icon: "tv".into(), sub: String::new(), face: None }).collect();
        let mut strip = Row::cards("modes", cards.clone());
        strip.strip = true;
        strip.in_view = 4;
        let g = measure_row(&mut kit, &strip, 1272.0);
        let c = g.cards.unwrap();
        assert_eq!((c.across, c.rows), (4, 1));
        assert_eq!(g.h, 80.0 + 12.0);
        let mut grid = Row::cards("g", cards);
        grid.cols = 3;
        assert_eq!(measure_row(&mut kit, &grid, 1272.0).cards.unwrap().rows, 3);
    }
}
