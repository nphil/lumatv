//! Where everything of the guide is: the head (title and chips), the ruler of times, the rail of channels and the grid beside it, the strip
//! that tells what the focus is on and the hints. A function of the kit's tokens and the person's text size, so another theme or text size is
//! another layout; everything sits on whole pixels (the text stays crisp at the panel's x2).

use super::model::{MIN, Page};
use crate::kit::Kit;
use crate::kit::button::Button;
use crate::kit::hint::glyph_side;
use crate::kit::text::{role_style, width_of};
use lumatv_tokens::TypeRole;
use ui::components::Align;
use ui::geometry::Frame;

/// The safe area of the TV profile.
pub const LEFT: f32 = 96.0;
pub const TOP: f32 = 54.0;
pub const RIGHT: f32 = 1824.0;
pub const BOTTOM: f32 = 1026.0;

/// The most a scroller's track can be (`ui::limits`): the time and the rows are cut to it.
pub const EXTENT_MAX: f32 = 131_072.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The title and the chips share this row.
    pub head: Frame,
    /// The title's box and where the chips start.
    pub title: Frame,
    pub chips_x: f32,
    /// The times, over the grid.
    pub ruler: Frame,
    pub rail: Frame,
    pub grid: Frame,
    /// What the programme in focus is, under the grid.
    pub strip: Frame,
    pub hints: Frame,
    pub row_h: f32,
    /// Rows that are whole on the screen.
    pub rows: usize,
    /// Pixels a minute is.
    pub ppm: f32,
    /// The time the grid shows, in ms.
    pub span: i64,
    /// The logo's tile.
    pub logo: (u32, u32),
    /// The rail's words: where they start and how wide they may be.
    pub text_x: f32,
    pub text_w: f32,
    /// The rail says the channel's name under its number when a row has the height for it.
    pub two_lines: bool,
    /// Lines of the programme's description in the strip.
    pub desc_lines: usize,
    /// The air between a cell's edge and its words.
    pub pad: f32,
}

impl Layout {
    pub fn page(&self) -> Page {
        Page { rows: self.rows, span: (self.grid.w / self.ppm).floor() as i64 * MIN }
    }

    /// The x of the instant `t` in the grid's content (0 at `origin`), on a whole pixel.
    pub fn x_of(&self, t: i64, origin: i64) -> f32 {
        ((t - origin) as f64 / MIN as f64 * f64::from(self.ppm)).round() as f32
    }

    /// The instant at content x, to the second.
    pub fn t_at(&self, x: f32, origin: i64) -> i64 {
        origin + (f64::from(x) / f64::from(self.ppm) * MIN as f64).round() as i64
    }

    /// How far the grid is scrolled when its left edge is at `view`.
    pub fn offset_of(&self, view: i64, origin: i64) -> f32 {
        self.x_of(view, origin).clamp(0.0, EXTENT_MAX)
    }

    /// The rows that can be reached: a scroller's track is at most [`EXTENT_MAX`] long.
    pub fn most_rows(&self) -> usize {
        (EXTENT_MAX / self.row_h) as usize
    }
}

pub fn plan(kit: &mut Kit) -> Layout {
    let (s1, s2, s3, s7) = (kit.space(1), kit.space(2), kit.space(3), kit.space(7));
    let scale = kit.text_scale.clamp(1.0, 2.0);
    let (label, caption, body, title) = (kit.line(TypeRole::Label), kit.line(TypeRole::Caption), kit.line(TypeRole::Body), kit.line(TypeRole::Title));
    let two_lines = label + caption + 2.0 * s2 <= 96.0;
    let row_h = (if two_lines { label + caption + 2.0 * s2 } else { label + 2.0 * s3 }).max(kit.tokens.row).ceil();
    let head_h = title.max(Button::height(kit)).ceil();
    let hint_h = (glyph_side(kit, TypeRole::Caption) as f32).max(caption).ceil();
    let desc_lines = if scale <= 1.3 { 2 } else { 0 };
    let strip_h = (2.0 * s3 + body + caption + desc_lines as f32 * caption).ceil();
    let ruler_h = (caption + s2).ceil();

    let hints = Frame::new(LEFT, BOTTOM - hint_h, RIGHT - LEFT, hint_h);
    let strip = Frame::new(LEFT, hints.y - s3 - strip_h, RIGHT - LEFT, strip_h);
    let ruler_y = TOP + head_h + s3;
    let body_y = ruler_y + ruler_h + s1;
    let room = strip.y - s3 - body_y;
    let rows = ((room / row_h).floor() as usize).max(1);
    let body_h = rows as f32 * row_h;
    // what is left of the height is air above the ruler and under the grid
    let spare = (room - body_h).max(0.0).floor();
    let up = (spare / 2.0).floor();
    let (ruler_y, body_y) = (ruler_y + up, body_y + up);

    let ink = kit.ui.look.ink();
    let (label_style, caption_style) = (role_style(kit, TypeRole::Label, None, ink, Align::Left), role_style(kit, TypeRole::Caption, None, ink, Align::Left));
    let num_w = width_of(kit, label_style, "199-99");
    let name_w = if two_lines { width_of(kit, caption_style, "WWWWWWW") } else { 0.0 };
    let text_w = num_w.max(name_w).ceil();
    let logo_h = (row_h - 2.0 * s3).max(32.0).floor();
    let logo_w = (logo_h * 16.0 / 9.0).round();
    let rail_w = (s3 + logo_w + s3 + text_w + s3).ceil();
    let ppm = (8.0 * scale * 2.0).round() / 2.0;
    let grid_x = LEFT + rail_w + s2;
    let minutes = ((RIGHT - grid_x) / ppm).floor().max(30.0);
    let grid_w = minutes * ppm;

    let title_style = role_style(kit, TypeRole::Title, None, ink, Align::Left);
    let title_w = width_of(kit, title_style, "Guide");
    Layout {
        head: Frame::new(LEFT, TOP, RIGHT - LEFT, head_h),
        title: Frame::new(LEFT, TOP + ((head_h - title) / 2.0).floor(), title_w, title),
        chips_x: LEFT + title_w + s7,
        ruler: Frame::new(grid_x, ruler_y, grid_w, ruler_h),
        rail: Frame::new(LEFT, body_y, rail_w, body_h),
        grid: Frame::new(grid_x, body_y, grid_w, body_h),
        strip,
        hints,
        row_h,
        rows,
        ppm,
        span: minutes as i64 * MIN,
        logo: (logo_w as u32, logo_h as u32),
        text_x: LEFT + s3 + logo_w + s3,
        text_w,
        two_lines,
        desc_lines,
        pad: s3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use ui::lucent::palette::Mode;

    fn inside(f: Frame) -> bool {
        f.x >= LEFT && f.y >= TOP && f.x + f.w <= RIGHT && f.y + f.h <= BOTTOM
    }

    #[test]
    fn every_part_is_inside_the_safe_area_and_the_parts_are_stacked_without_overlap_at_100_and_200_percent() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let (_h, mut kit) = testkit::kit_in(Mode::Dark, scale);
            let l = plan(&mut kit);
            for (name, f) in [("head", l.head), ("ruler", l.ruler), ("rail", l.rail), ("grid", l.grid), ("strip", l.strip), ("hints", l.hints)] {
                assert!(inside(f), "{name} at {scale}: {f:?}");
            }
            assert!(l.head.y + l.head.h <= l.ruler.y && l.ruler.y + l.ruler.h <= l.grid.y, "the head, the ruler and the grid are stacked at {scale}");
            assert!(l.grid.y + l.grid.h <= l.strip.y && l.strip.y + l.strip.h <= l.hints.y, "the grid, the strip and the hints are stacked at {scale}");
            assert!(l.rail.x + l.rail.w <= l.grid.x, "the rail is beside the grid at {scale}");
            assert_eq!(l.grid.h, l.rows as f32 * l.row_h, "whole rows only");
            assert!(l.rows >= 4, "{} rows at {scale}", l.rows);
        }
    }

    #[test]
    fn a_row_a_minute_and_the_grid_are_whole_pixels_and_the_span_is_whole_minutes() {
        for scale in [1.0, 1.5, 2.0] {
            let (_h, mut kit) = testkit::kit_in(Mode::Dark, scale);
            let l = plan(&mut kit);
            for v in [l.row_h, l.grid.x, l.grid.y, l.grid.w, l.rail.w, l.text_x, l.pad, l.strip.y, l.hints.y, l.ruler.y] {
                assert_eq!(v, v.round(), "{v} at {scale}");
            }
            assert_eq!(l.page().span % MIN, 0);
            assert_eq!(l.page().span as f32 / MIN as f32 * l.ppm, l.grid.w);
            assert!(l.page().span >= 80 * MIN, "at least an hour and a half in the grid at {scale}: {}", l.page().span / MIN);
            assert_eq!(l.x_of(30 * MIN, 0), 30.0 * l.ppm);
            assert_eq!(l.t_at(l.x_of(95 * MIN, 0), 0), 95 * MIN);
        }
    }

    #[test]
    fn the_guide_shows_a_day_and_more_of_listings_for_hundreds_of_channels_within_what_a_scroller_can_be() {
        for scale in [1.0, 2.0] {
            let (_h, mut kit) = testkit::kit_in(Mode::Dark, scale);
            let l = plan(&mut kit);
            let five_days_and_six_hours = 126 * 60 * MIN;
            assert!(l.x_of(five_days_and_six_hours, 0) <= EXTENT_MAX, "the time fits the track at {scale}");
            assert!(l.most_rows() >= 800, "{} rows reachable at {scale}", l.most_rows());
        }
    }

    #[test]
    fn a_bigger_text_gives_a_row_with_the_number_alone_and_fewer_minutes() {
        let (_h, mut kit) = testkit::kit_in(Mode::Dark, 1.0);
        let one = plan(&mut kit);
        let (_h2, mut kit2) = testkit::kit_in(Mode::Dark, 2.0);
        let two = plan(&mut kit2);
        assert!(one.two_lines && !two.two_lines);
        assert!(two.ppm > one.ppm && two.page().span < one.page().span && two.rows <= one.rows);
    }
}
