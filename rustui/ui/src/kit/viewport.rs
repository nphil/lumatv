//! The clip of a viewport that holds lifted focusables (a list of rows, a grid of tiles, the dock): the window grown by what a focus
//! lift adds to the biggest of them, so a focused plate at full lift is whole, rounded corners and rim included, wherever it stands in
//! the window.
//!
//! A lifted focusable grows about its centre by its scale and rises by its lift, so its edge leaves its frame by `(scale - 1) / 2` of
//! its size (a row 1270 px wide at the row scale of 1.02 leaves it by 12.7 px on each side) and, for a card, by the rise above. A clip
//! that is exactly the window squares those ends off. The viewport still cuts what has scrolled out: it is wider than the window only
//! by the reach, which is a few pixels along the way the content scrolls (a row's growth in height is under 2 px) and a plate's own
//! growth across it; the rows that are not shown are not drawn, or are cut at a whole row, so the margin shows none of them.

use super::widgets::Lift;
use super::Kit;
use compositor_protocol::Rect;
use ui::geometry::Frame;

/// The anti-aliased rim of a plate: a pixel beyond its box.
const RIM: f32 = 1.0;

/// How far a lifted focusable stands out of its frame, per side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reach {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Reach {
    pub const NONE: Reach = Reach { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 };

    /// The reach of a focusable of `size` that lifts to `scale` and rises `rise` px.
    pub fn of(scale: f32, rise: f32, size: (f32, f32)) -> Self {
        let grow = (scale - 1.0).max(0.0) / 2.0;
        let (gx, gy) = (grow * size.0 + RIM, grow * size.1 + RIM);
        Self { left: gx, top: gy + rise.max(0.0), right: gx, bottom: gy }
    }

    /// The reach of a focusable of `size` lifted as `lift` is in the kit's tokens.
    pub fn lift(kit: &Kit, lift: Lift, size: (f32, f32)) -> Self {
        Self::of(lift.scale(kit), lift.rise(kit), size)
    }

    /// The most of each side of `self` and `other`: a viewport that holds focusables of several kinds.
    pub fn and(self, other: Reach) -> Self {
        Self { left: self.left.max(other.left), top: self.top.max(other.top), right: self.right.max(other.right), bottom: self.bottom.max(other.bottom) }
    }

    /// The clip of a viewport over `window`: the window and the reach around it.
    pub fn clip(self, window: Frame) -> Rect {
        Rect { x: window.x - self.left, y: window.y - self.top, w: window.w + self.left + self.right, h: window.h + self.top + self.bottom }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_leaves_its_frame_by_half_the_growth_of_its_size_and_a_card_rises_above() {
        let row = Reach::of(1.02, 0.0, (1270.0, 70.0));
        assert!((row.left - 13.7).abs() < 0.01 && row.right == row.left, "{row:?}");
        assert!((row.top - 1.7).abs() < 0.01 && row.bottom == row.top, "{row:?}");
        let card = Reach::of(1.03, 4.0, (300.0, 200.0));
        assert!((card.top - card.bottom - 4.0).abs() < 0.001, "{card:?}");
        assert_eq!(Reach::of(1.0, 0.0, (500.0, 500.0)), Reach { left: RIM, top: RIM, right: RIM, bottom: RIM });
    }

    #[test]
    fn the_clip_is_the_window_and_the_reach_and_the_widest_of_two_kinds_wins_on_each_side() {
        let r = Reach::of(1.04, 0.0, (100.0, 50.0)).and(Reach::of(1.03, 4.0, (300.0, 200.0)));
        let c = r.clip(Frame::new(10.0, 20.0, 400.0, 300.0));
        assert!((c.x - (10.0 - r.left)).abs() < 0.001 && (c.w - (400.0 + r.left + r.right)).abs() < 0.001);
        assert!((c.y - (20.0 - r.top)).abs() < 0.001 && (c.h - (300.0 + r.top + r.bottom)).abs() < 0.001);
        assert!(r.left > 4.0 && r.top > r.bottom);
    }
}
