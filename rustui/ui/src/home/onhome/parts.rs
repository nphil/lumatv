//! Where the parts of one item are inside its cell: the glyph, the switch, the lines of words. Pure, so that no two of them overlap at any
//! text size is a thing a test holds, not a thing a screenshot is looked at for.

use super::layout::{Metrics, Size, Style};
use ui::geometry::Frame;

/// How the contents of a cell are arranged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// The glyph above the words.
    Stacked,
    /// The glyph beside the words (a wide cell, or a cell too low for the glyph to be above).
    Compact,
    /// One line, the glyph beside it.
    Mini,
    /// A large cell: a big glyph, the words large below.
    Big,
}

/// The look of an item that is `size` in a grid whose cells are of `style`.
pub fn look_of(size: Size, style: Style) -> Look {
    match (size, style) {
        (_, Style::Mini) => Look::Mini,
        (Size::Small, Style::Stacked) => Look::Stacked,
        (Size::Small, Style::Compact) => Look::Compact,
        (Size::Wide, _) => Look::Compact,
        (Size::Large, _) => Look::Big,
    }
}

/// The role of type of a line of words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Display,
    Title,
    Body,
    Label,
    Caption,
}

impl Line {
    /// The height of a line of this role.
    pub fn height(self, m: &Metrics) -> f32 {
        match self {
            Line::Display => m.display,
            Line::Title => m.title,
            Line::Body => m.body,
            Line::Label => m.label,
            Line::Caption => m.caption,
        }
    }
}

/// The roles of the two lines of an item in a cell `h` high: the reading of a sensor is as large as the cell holds (never larger than a
/// glyph and a line under it need), the name of a lamp is a label, and a large cell has larger type.
pub fn lines_of(look: Look, headline: bool, h: f32, m: &Metrics) -> (Line, Line) {
    let g = glyph_side(look, headline, m);
    let fits = |main: Line, second: Line| match look {
        Look::Stacked | Look::Big => h >= 2.0 * m.pad_y + g + 4.0 + main.height(m) + second.height(m),
        Look::Compact => h >= 2.0 * m.pad_y + main.height(m) + second.height(m),
        Look::Mini => false,
    };
    match look {
        Look::Big if headline && fits(Line::Display, Line::Body) => (Line::Display, Line::Body),
        Look::Big if headline && fits(Line::Title, Line::Body) => (Line::Title, Line::Body),
        Look::Big if fits(Line::Title, Line::Body) => (Line::Title, Line::Body),
        Look::Big => (Line::Label, Line::Caption),
        Look::Stacked | Look::Compact if headline && fits(Line::Title, Line::Caption) => (Line::Title, Line::Caption),
        Look::Mini => (Line::Label, Line::Caption),
        _ => (Line::Label, Line::Caption),
    }
}

/// What the parts are made of: the sizes the type and the switch have.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizes {
    /// The line height of the first line (the name, or the reading of a sensor) and of the second.
    pub main: f32,
    pub second: f32,
    /// The switch at its full size, if the item has one.
    pub switch: Option<(f32, f32)>,
    /// Whether the first line is the reading and not the name.
    pub headline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parts {
    pub glyph: Frame,
    pub switch: Option<Frame>,
    pub main: Frame,
    pub second: Frame,
}

/// The side of the glyph in a look: smaller where the reading is the headline, larger in a large cell.
pub fn glyph_side(look: Look, headline: bool, m: &Metrics) -> f32 {
    let g = m.glyph;
    match look {
        Look::Big => (g * 1.5).round(),
        Look::Mini => (g * 0.8).round().max(24.0),
        _ if headline => (g * 0.75).round(),
        _ => g,
    }
}

/// How far a glyph's drawing stands in from the edge of its box: a line icon on a 24 grid keeps 3 units clear all round.
pub fn optical(g: f32) -> f32 {
    (g / 8.0).round()
}

/// The parts of a cell of `w` by `h`, on one grid: what is seen of the glyph, the left edge of the words and the right edge of a switch stand
/// the cell's inset (`pad_x`) from its sides, and the glyph and the switch share one centre line. The glyph's box is moved out by what its
/// drawing keeps clear ([`optical`]) so that it is the drawing that stands on the grid.
pub fn parts(look: Look, w: f32, h: f32, s: &Sizes, m: &Metrics) -> Parts {
    let (px, py) = (m.pad_x, m.pad_y);
    let g = glyph_side(look, s.headline, m);
    let o = optical(g);
    let gap = 14.0;
    // a switch smaller where the full one would meet the glyph: two thirds above the words (it is a mark in a corner, not a control to aim
    // at), three quarters where a cell is large, the full size only beside the words with room
    let switch_size = |room: f32, scale: f32| s.switch.map(|(sw, sh)| if scale >= 1.0 && sw <= room { (sw, sh) } else { ((sw * scale.min(0.75)).round(), (sh * scale.min(0.75)).round()) });
    match look {
        Look::Stacked | Look::Big => {
            let glyph = Frame::new(px - o, px - o, g, g);
            let sw = switch_size(w, if look == Look::Big { 0.75 } else { 2.0 / 3.0 });
            let centre = glyph.y + g / 2.0;
            let switch = sw.map(|(sw, sh)| Frame::new(w - px - sw, (centre - sh / 2.0).floor(), sw, sh));
            let second = Frame::new(px, h - py - s.second, w - 2.0 * px, s.second);
            let main = Frame::new(px, second.y - s.main, w - 2.0 * px, s.main);
            Parts { glyph, switch, main, second }
        }
        Look::Compact => {
            let glyph = Frame::new(px - o, ((h - g) / 2.0).floor(), g, g);
            let x = px + g - 2.0 * o + gap;
            let room = w - x - px;
            let sw = switch_size(room / 2.0, 1.0);
            let tw = room - sw.map_or(0.0, |(sw, _)| sw + gap);
            let block = s.main + s.second;
            let y = ((h - block) / 2.0).floor();
            let switch = sw.map(|(sw, sh)| Frame::new(w - px - sw, ((h - sh) / 2.0).floor(), sw, sh));
            Parts { glyph, switch, main: Frame::new(x, y, tw, s.main), second: Frame::new(x, y + s.main, tw, s.second) }
        }
        Look::Mini => {
            let glyph = Frame::new(px - o, ((h - g) / 2.0).floor(), g, g);
            let x = px + g - 2.0 * o + gap;
            let room = w - x - px;
            let sw = switch_size(room / 2.0, 1.0);
            let tw = room - sw.map_or(0.0, |(sw, _)| sw + gap);
            let switch = sw.map(|(sw, sh)| Frame::new(w - px - sw, ((h - sh) / 2.0).floor(), sw, sh));
            let main = Frame::new(x, ((h - s.main) / 2.0).floor(), tw, s.main);
            Parts { glyph, switch, main, second: Frame::new(x, main.bottom(), tw, 0.0) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(a: Frame, b: Frame) -> bool {
        a.w > 0.0 && a.h > 0.0 && b.w > 0.0 && b.h > 0.0 && a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }

    /// Every look of every kind of cell at both text sizes: the parts are inside the cell with the cell's own padding, none overlaps another
    /// and the words have a width to be in.
    #[test]
    fn no_part_of_an_item_overlaps_another_or_leaves_its_cell_at_any_text_size() {
        for scale in [1.0f32, 1.5, 2.0] {
            let m = Metrics::nominal(scale);
            for (look, w, h) in [(Look::Stacked, 154.0, 160.0), (Look::Stacked, 196.0, 188.0), (Look::Stacked, 150.0, m.stacked_h()), (Look::Compact, 254.0, m.compact_h()), (Look::Compact, 404.0, 160.0), (Look::Mini, 300.0, m.mini_h()), (Look::Big, 404.0, 328.0), (Look::Big, 320.0, 300.0)] {
                for switch in [None, Some((84.0, 48.0))] {
                    for headline in [false, true] {
                        let (w, h) = (w * scale.min(1.5), h.max(match look { Look::Stacked => m.stacked_h(), Look::Compact => m.compact_h(), Look::Mini => m.mini_h(), Look::Big => 0.0 }));
                        let (first, second) = lines_of(look, headline, h, &m);
                        let s = Sizes { main: first.height(&m), second: second.height(&m), switch, headline };
                        let p = parts(look, w, h, &s, &m);
                        let cell = Frame::new(0.0, 0.0, w, h);
                        let tag = format!("{look:?} {w}x{h} scale {scale} switch {switch:?} headline {headline}: {p:?}");
                        let inside = |f: Frame| f.x >= -0.01 && f.y >= -0.01 && f.right() <= cell.right() + 0.01 && f.bottom() <= cell.bottom() + 0.01;
                        assert!(inside(p.glyph) && inside(p.main) && inside(p.second) && p.switch.is_none_or(inside), "{tag}");
                        assert!(p.main.w >= 20.0, "{tag}: the words need a width");
                        assert!(!overlaps(p.glyph, p.main) && !overlaps(p.glyph, p.second) && !overlaps(p.main, p.second), "{tag}");
                        if let Some(sw) = p.switch {
                            assert!(!overlaps(sw, p.glyph) && !overlaps(sw, p.main) && !overlaps(sw, p.second), "{tag}");
                        }
                        for f in [p.glyph, p.main, p.second] {
                            if f.w > 0.0 {
                                assert!(f.x == f.x.floor() && f.y == f.y.floor(), "{tag}: whole pixels {f:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_cell_has_the_look_its_size_and_the_grid_give_it() {
        assert_eq!(look_of(Size::Small, Style::Stacked), Look::Stacked);
        assert_eq!(look_of(Size::Small, Style::Compact), Look::Compact);
        assert_eq!(look_of(Size::Wide, Style::Stacked), Look::Compact);
        assert_eq!(look_of(Size::Large, Style::Stacked), Look::Big);
        assert_eq!(look_of(Size::Large, Style::Mini), Look::Mini);
    }

    /// What is seen of the glyph, the left edge of the words and the right edge of a switch stand the same inset from the sides of a cell, the
    /// switch is centred on the glyph, and the switch is a mark smaller than the glyph's row is high.
    #[test]
    fn the_glyph_the_words_and_the_switch_stand_on_one_grid() {
        for scale in [1.0f32, 1.5, 2.0] {
            let m = Metrics::nominal(scale);
            for (look, w, h) in [(Look::Stacked, 162.0 * scale.min(1.5), m.stacked_h() + 20.0), (Look::Big, 340.0, 330.0)] {
                let (first, second) = lines_of(look, false, h, &m);
                let s = Sizes { main: first.height(&m), second: second.height(&m), switch: Some((95.0, 54.0)), headline: false };
                let p = parts(look, w, h, &s, &m);
                let o = optical(p.glyph.w);
                let tag = format!("{look:?} scale {scale}: {p:?}");
                assert_eq!(p.glyph.x + o, m.pad_x, "{tag}: the drawing of the glyph stands where the words start");
                assert_eq!(p.glyph.y + o, m.pad_x, "{tag}: and as far from the top as from the side");
                assert_eq!(p.main.x, m.pad_x, "{tag}");
                let sw = p.switch.expect("a switch");
                assert_eq!(sw.right(), w - m.pad_x, "{tag}: the switch ends the same inset from the other side");
                assert!((sw.y + sw.h / 2.0 - (p.glyph.y + p.glyph.h / 2.0)).abs() <= 0.5, "{tag}: one centre line");
                assert!(sw.h < p.glyph.h, "{tag}: a mark, not a control");
            }
        }
    }
}
