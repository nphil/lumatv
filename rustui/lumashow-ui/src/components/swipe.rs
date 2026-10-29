//! What a finger's release means: whether a page swiped from the left edge goes back, a pulled sheet stays open, a card is
//! dismissed, a pager turns. All pure: the components feed raw samples and apply the answer with layers.
//!
//! The gesture recogniser ([`crate::input::gesture`]) only classifies after the finger lifts; a page that follows the finger needs
//! these decisions while it is still down, and the same numbers at the lift. Every distance is in logical units and every speed in
//! units per second. Whether a swipe goes on or comes back is [`motion::Commit`]'s rule, the same for all four (the presets
//! [`commit`] holds are the approved mock-ups' numbers), and the band a swipe is pulled past its end by is the one rubber band.

use crate::input::TouchEvent;
use crate::lucent::tokens::motion::{self, commit};
use compositor_protocol::Ease;
use compositor_protocol::motion::{VELOCITY_WINDOW_NS, slope_per_s, start_slope};
use std::collections::VecDeque;

/// A swipe from the left edge starts closer to it than this (the recogniser's own band).
pub const EDGE_BAND: f32 = 40.0;
/// Farther than this from the down point and a touch is a drag, not a press (the recogniser's tap slop).
pub const SLOP: f32 = 24.0;

/// One raw touch sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub x: f32,
    pub y: f32,
    pub at_ns: u64,
}

impl Sample {
    /// The position and time of a touch event.
    pub fn of(touch: &TouchEvent) -> Self {
        Self {
            x: touch.x,
            y: touch.y,
            at_ns: touch.at_ns,
        }
    }
}

const KEPT: usize = 8;

/// One finger's path: where it went down, where it is, how fast it is going.
#[derive(Clone, Debug, Default)]
pub struct Contact {
    start: Option<Sample>,
    recent: VecDeque<Sample>,
}

impl Contact {
    pub fn down(&mut self, s: Sample) {
        self.start = Some(s);
        self.recent.clear();
        self.recent.push_back(s);
    }

    pub fn moved(&mut self, s: Sample) {
        if self.recent.len() == KEPT {
            self.recent.pop_front();
        }
        self.recent.push_back(s);
    }

    pub fn is_down(&self) -> bool {
        self.start.is_some()
    }

    pub fn end(&mut self) {
        self.start = None;
        self.recent.clear();
    }

    pub fn start(&self) -> Option<Sample> {
        self.start
    }

    pub fn last(&self) -> Option<Sample> {
        self.recent.back().copied()
    }

    /// Distance from the down point.
    pub fn travel(&self) -> (f32, f32) {
        match (self.start, self.last()) {
            (Some(a), Some(b)) => (b.x - a.x, b.y - a.y),
            _ => (0.0, 0.0),
        }
    }

    /// Whether the finger has left the tap slop.
    pub fn dragged(&self) -> bool {
        let (dx, dy) = self.travel();
        dx.hypot(dy) > SLOP
    }

    /// Units per second: the slope of the samples of the last [`VELOCITY_WINDOW_NS`] ([`slope_per_s`], the estimator every release uses),
    /// `(0, 0)` when there is nothing to measure. Averaged over a longer time it would be well below the speed the finger lifts at.
    pub fn velocity(&self) -> (f32, f32) {
        (
            slope_per_s(
                self.recent.iter().map(|s| (s.at_ns, s.x)),
                VELOCITY_WINDOW_NS,
            ),
            slope_per_s(
                self.recent.iter().map(|s| (s.at_ns, s.y)),
                VELOCITY_WINDOW_NS,
            ),
        )
    }
}

/// `value` moved by a finger, resisted past `0..=span` by the one rubber band ([`motion::rubber`], capped at
/// [`motion::OVERSCROLL_PX`]): full speed inside.
pub fn rubber(value: f32, span: f32) -> f32 {
    if value < 0.0 {
        -motion::rubber(-value, motion::OVERSCROLL_PX)
    } else if value > span {
        span + motion::rubber(value - span, motion::OVERSCROLL_PX)
    } else {
        value
    }
}

/// How long a settle over `remaining` of `span` takes: proportional to the distance left, never quicker than a press and never
/// longer than a layer's whole travel.
pub fn settle_ms(remaining: f32, span: f32) -> u32 {
    let full = motion::LAYER_MS as f32;
    let ms = full * (remaining.abs() / span.max(1.0));
    ms.clamp(motion::PRESS_MS as f32 + 30.0, full).round() as u32
}

/// How a page carries on after the finger lets go of it, `remaining` units short of where it is going, the finger having moved
/// that way at `speed` units per second (zero or less: it was not). The glide starts at the finger's own speed, so the page
/// neither lurches ahead nor stalls at the hand-off: a page leaving the screen keeps that speed to the end ([`motion::LINEAR`]; the
/// end is off screen), one landing in place slows onto its rest ([`motion::SETTLE`], whose start is [`start_slope`] times its mean
/// speed). A finger that was not moving that way gets [`settle_ms`]. Never quicker than a press, never longer than a scroll.
pub fn release(remaining: f32, speed: f32, span: f32, landing: bool) -> (u32, Ease) {
    let ease = if landing {
        motion::SETTLE
    } else {
        motion::LINEAR
    };
    let remaining = remaining.abs();
    if speed <= 0.0 || remaining == 0.0 {
        return (settle_ms(remaining, span), ease);
    }
    let ms = start_slope(ease) * remaining / speed * 1000.0;
    let ms = ms.clamp(motion::PRESS_MS as f32 + 30.0, motion::SCROLL_MS as f32);
    (ms.round() as u32, ease)
}

/// What lifting the finger did to a page dragged from the left edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settle {
    /// The page carries on to where it is dismissed.
    Complete,
    /// The page springs back.
    Cancel,
}

/// A page that follows a finger swiped in from the left edge.
#[derive(Clone, Copy, Debug)]
pub struct EdgeBack {
    /// The width of the page being pulled away.
    pub width: f32,
}

impl EdgeBack {
    /// A touch at `x` may start an edge swipe.
    pub fn starts(x: f32) -> bool {
        x < EDGE_BAND
    }

    /// Where the page is while the finger is at `x` having gone down at `start_x`: never left of its rest, never past its own
    /// width.
    pub fn offset(&self, start_x: f32, x: f32) -> f32 {
        (x - start_x).clamp(0.0, self.width)
    }

    /// The page goes back by the one commit rule ([`commit::DISMISS`], a page that leaves being a card that leaves): once it is
    /// that much of its width across, or when flung right; a flick back to the left cancels it however far it had come.
    pub fn settle(&self, offset: f32, vx: f32) -> Settle {
        if commit::DISMISS.decides(offset, self.width, vx) {
            Settle::Complete
        } else {
            Settle::Cancel
        }
    }
}

/// Whether a sheet pulled from an edge ends open or closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetEnd {
    Open,
    Closed,
}

/// A sheet whose revealed height follows a finger pulling it in from its edge, or pushing it back. The arithmetic is for a finger
/// that opens the sheet by moving towards positive `y` (a shade pulled down from the top); a sheet that opens the other way (a
/// keyboard raised from the bottom) feeds it mirrored coordinates.
#[derive(Clone, Copy, Debug)]
pub struct SheetDrag {
    /// How much of the sheet shows when it is open.
    pub open_h: f32,
}

impl SheetDrag {
    /// How much of the sheet shows with the finger at `y`, having grabbed it at `grab_y` when `shown` of it showed.
    pub fn reveal(&self, shown: f32, grab_y: f32, y: f32) -> f32 {
        (shown + (y - grab_y)).clamp(0.0, self.open_h)
    }

    /// The sheet goes to the other end than it started at by the one commit rule ([`commit::SHEET`]): when it has travelled half
    /// of its height that way, or a flick that way lets go of it; a flick back to where it started keeps it there.
    pub fn settle(&self, revealed: f32, vy: f32, was_open: bool) -> SheetEnd {
        let goes_on = if was_open {
            commit::SHEET.decides(self.open_h - revealed, self.open_h, -vy)
        } else {
            commit::SHEET.decides(revealed, self.open_h, vy)
        };
        if goes_on != was_open {
            SheetEnd::Open
        } else {
            SheetEnd::Closed
        }
    }
}

/// What lifting the finger did to a card swiped sideways.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fate {
    /// The card leaves in the direction of `dir` (-1 left, 1 right).
    Dismissed {
        dir: f32,
    },
    Restored,
}

/// A card that follows a finger swiped along it and leaves when let go far enough or fast enough.
#[derive(Clone, Copy, Debug)]
pub struct Dismiss {
    pub width: f32,
}

impl Dismiss {
    /// The card fades as it leaves, but stays readable while it is still being decided.
    pub fn alpha(&self, dx: f32) -> f32 {
        (1.0 - dx.abs() / self.width).clamp(0.35, 1.0)
    }

    /// Dismissed by the one commit rule ([`commit::DISMISS`]): once it is that much of its width across, or flung on in the
    /// direction it was moving; otherwise back, and a fling back against its drag brings it back however far it had come.
    pub fn settle(&self, dx: f32, vx: f32) -> Fate {
        let dir = dx.signum();
        if commit::DISMISS.decides(dx.abs(), self.width, vx * dir) {
            Fate::Dismissed { dir }
        } else {
            Fate::Restored
        }
    }
}

/// Pages side by side, one `width` apart, that a swipe turns.
#[derive(Clone, Copy, Debug)]
pub struct PagerDrag {
    pub pages: usize,
    pub width: f32,
}

impl PagerDrag {
    /// The strip's offset with `page` showing and the finger `dx` from where it went down: one for one, resisted past the first
    /// and last page by the one rubber band.
    pub fn offset(&self, page: usize, dx: f32) -> f32 {
        let at = page as f32 * self.width;
        let span = (self.pages.saturating_sub(1)) as f32 * self.width;
        -(rubber(at - dx, span))
    }

    /// The page showing after the finger lifts `dx` from where it went down at `vx`, by the one commit rule ([`commit::PAGER`]):
    /// that much of a page or a flick turns it, one page at the most and never past the ends.
    pub fn settle(&self, page: usize, dx: f32, vx: f32) -> usize {
        let last = self.pages.saturating_sub(1);
        match commit::PAGER.turn(-dx, self.width, -vx) {
            1 => (page + 1).min(last),
            -1 => page.saturating_sub(1),
            _ => page,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32, ms: u64) -> Sample {
        Sample {
            x,
            y,
            at_ns: ms * 1_000_000,
        }
    }

    #[test]
    fn velocity_is_units_per_second_over_the_last_fifty_milliseconds() {
        let mut c = Contact::default();
        c.down(at(0.0, 0.0, 0));
        // 2 units per millisecond for 200 ms; only the last 50 ms count.
        for ms in (10..=200).step_by(10) {
            c.moved(at(2.0 * ms as f32, 0.0, ms));
        }
        let (vx, vy) = c.velocity();
        assert!((vx - 2000.0).abs() < 1.0, "vx {vx}");
        assert_eq!(vy, 0.0);
        c.end();
        assert_eq!(c.velocity(), (0.0, 0.0));
        assert!(!c.is_down());
    }

    #[test]
    fn a_swipe_that_has_just_got_up_to_speed_leaves_at_that_speed_and_not_at_its_average_since_the_landing()
     {
        // The shape of the swipes made on the Show: the finger rests 20 ms, speeds up to 3 units a millisecond over 30 ms and goes on at
        // that; samples a refresh apart. Its speed averaged over the last 100 ms is 1.95 a millisecond (65 % of what it leaves at).
        let travelled = |t_ms: f32| {
            let ramp = (t_ms - 20.0).clamp(0.0, 30.0);
            0.5 * (3.0 / 30.0) * ramp * ramp + 3.0 * (t_ms - 50.0).max(0.0)
        };
        let mut c = Contact::default();
        c.down(at(0.0, 0.0, 0));
        for ms in [17u64, 34, 50, 67, 84, 100] {
            c.moved(at(0.0, travelled(ms as f32), ms));
        }
        let (vx, vy) = c.velocity();
        assert_eq!(vx, 0.0);
        assert!(
            (vy / 3000.0 - 1.0).abs() < 0.05,
            "leaves at {vy} units a second; the finger was going 3000"
        );
    }

    #[test]
    fn a_finger_that_has_not_left_the_slop_is_still_a_press() {
        let mut c = Contact::default();
        c.down(at(100.0, 100.0, 0));
        c.moved(at(110.0, 108.0, 16));
        assert!(!c.dragged());
        c.moved(at(140.0, 100.0, 32));
        assert!(c.dragged());
        assert_eq!(c.travel(), (40.0, 0.0));
    }

    #[test]
    fn the_edge_page_follows_the_finger_one_for_one_and_stays_inside_its_own_width() {
        let e = EdgeBack { width: 912.0 };
        assert!(EdgeBack::starts(8.0));
        assert!(!EdgeBack::starts(EDGE_BAND));
        assert_eq!(e.offset(8.0, 8.0), 0.0);
        assert_eq!(e.offset(8.0, 208.0), 200.0);
        assert_eq!(e.offset(8.0, -50.0), 0.0, "never left of rest");
        assert_eq!(e.offset(8.0, 5000.0), 912.0, "never past the page");
    }

    #[test]
    fn the_edge_page_goes_back_at_its_share_of_its_width_or_a_fling_and_not_before() {
        let e = EdgeBack { width: 900.0 };
        let (share, flick) = (commit::DISMISS.share * 900.0, commit::DISMISS.flick);
        assert_eq!(e.settle(share - 1.0, 0.0), Settle::Cancel);
        assert_eq!(e.settle(share + 1.0, 0.0), Settle::Complete);
        assert_eq!(
            e.settle(60.0, flick),
            Settle::Complete,
            "a short fling still goes back"
        );
        assert_eq!(
            e.settle(10.0, flick),
            Settle::Cancel,
            "a twitch is not a fling"
        );
        assert_eq!(
            e.settle(600.0, -flick),
            Settle::Cancel,
            "flicked back to the left"
        );
    }

    #[test]
    fn a_pulled_sheet_needs_half_its_height_to_open_and_half_of_the_way_back_to_close() {
        let s = SheetDrag { open_h: 400.0 };
        let flick = commit::SHEET.flick;
        assert_eq!(s.reveal(0.0, 6.0, 106.0), 100.0);
        assert_eq!(s.reveal(0.0, 6.0, 900.0), 400.0, "never past open");
        assert_eq!(
            s.reveal(400.0, 300.0, 100.0),
            200.0,
            "pushed back up by 200"
        );
        assert_eq!(s.settle(199.0, 0.0, false), SheetEnd::Closed);
        assert_eq!(s.settle(201.0, 0.0, false), SheetEnd::Open);
        // The same half the other way: an open sheet stays open until it is pushed half of its height back.
        assert_eq!(s.settle(201.0, 0.0, true), SheetEnd::Open);
        assert_eq!(s.settle(199.0, 0.0, true), SheetEnd::Closed);
        assert_eq!(s.settle(50.0, flick, false), SheetEnd::Open);
        assert_eq!(s.settle(380.0, -flick, true), SheetEnd::Closed);
        assert_eq!(
            s.settle(150.0, -flick, false),
            SheetEnd::Closed,
            "a flick back to where it started keeps it there"
        );
    }

    #[test]
    fn a_card_is_dismissed_by_distance_or_by_a_fling_the_way_it_was_going_and_fades_on_the_way() {
        let d = Dismiss { width: 900.0 };
        let (share, flick) = (commit::DISMISS.share * 900.0, commit::DISMISS.flick);
        assert_eq!(d.settle(share - 1.0, 0.0), Fate::Restored);
        assert_eq!(d.settle(share + 1.0, 0.0), Fate::Dismissed { dir: 1.0 });
        assert_eq!(
            d.settle(-(share + 50.0), 0.0),
            Fate::Dismissed { dir: -1.0 }
        );
        assert_eq!(d.settle(80.0, flick), Fate::Dismissed { dir: 1.0 });
        assert_eq!(
            d.settle(80.0, -flick),
            Fate::Restored,
            "flung back against its drag"
        );
        assert_eq!(
            d.settle(share + 50.0, -flick),
            Fate::Restored,
            "flung back however far it had come"
        );
        assert_eq!(
            d.settle(5.0, flick),
            Fate::Restored,
            "a twitch is not a fling"
        );
        assert_eq!(d.alpha(0.0), 1.0);
        assert!(d.alpha(450.0) < d.alpha(100.0));
        assert_eq!(
            d.alpha(900.0),
            0.35,
            "still visible while it is being decided"
        );
    }

    #[test]
    fn a_pager_turns_at_its_share_of_a_page_or_a_flick_and_resists_past_its_ends() {
        let p = PagerDrag {
            pages: 3,
            width: 960.0,
        };
        let (share, flick) = (commit::PAGER.share, commit::PAGER.flick);
        let edge = share * 960.0;
        assert_eq!(p.settle(0, -(edge - 1.0), 0.0), 0);
        assert_eq!(p.settle(0, -(edge + 1.0), 0.0), 1);
        assert_eq!(p.settle(1, 300.0, 0.0), 0);
        assert_eq!(p.settle(1, -30.0, -flick), 2);
        assert_eq!(p.settle(1, 30.0, flick), 0);
        assert_eq!(
            p.settle(1, -30.0, flick),
            1,
            "a flick against the drag is no turn"
        );
        assert_eq!(p.settle(1, -5.0, -flick), 1, "a twitch is not a flick");
        assert_eq!(p.settle(2, -400.0, 0.0), 2, "no page past the last");
        assert_eq!(p.settle(0, 400.0, 0.0), 0, "no page before the first");
        assert_eq!(p.offset(1, 0.0), -960.0);
        assert_eq!(p.offset(1, -100.0), -1060.0, "one for one inside");
        let past_first = p.offset(0, 100.0);
        assert!(
            past_first > 0.0 && past_first < 100.0 * motion::RUBBER_FOLLOW,
            "resisted by the one rubber band: {past_first}"
        );
        assert!(
            p.offset(0, 100_000.0) <= motion::OVERSCROLL_PX,
            "and capped"
        );
    }

    #[test]
    fn a_settle_takes_time_in_proportion_to_the_distance_left() {
        assert_eq!(settle_ms(900.0, 900.0), motion::LAYER_MS);
        assert!(settle_ms(450.0, 900.0) < motion::LAYER_MS);
        assert!(
            settle_ms(0.0, 900.0) >= motion::PRESS_MS,
            "never quicker than a press"
        );
    }

    #[test]
    fn a_released_page_starts_at_the_fingers_speed_leaving_at_it_and_landing_slowing_down() {
        // Units per millisecond at the start of a glide over `px` in `ms` with `ease`.
        let start = |px: f32, (ms, ease): (u32, Ease)| start_slope(ease) * px / ms as f32;
        // A finger at 1500 units/s is 1.5 per ms: a page leaving 300 short keeps that speed.
        let leaving = release(300.0, 1500.0, 960.0, false);
        assert_eq!(leaving.1, motion::LINEAR);
        assert!((start(300.0, leaving) - 1.5).abs() < 0.01, "{leaving:?}");
        // Landing 100 short it starts at that speed and slows onto its rest.
        let landing = release(100.0, 1500.0, 960.0, true);
        assert_eq!(landing.1, motion::SETTLE);
        assert!((start(100.0, landing) - 1.5).abs() < 0.01, "{landing:?}");
        // Far to go at a slow finger: no longer than a scroll. Little to go at a fast one: no quicker than a press.
        assert_eq!(release(900.0, 200.0, 960.0, false).0, motion::SCROLL_MS);
        assert_eq!(release(10.0, 5_000.0, 960.0, true).0, motion::PRESS_MS + 30);
        // A finger that stopped, or was going the other way: the distance decides.
        assert_eq!(release(480.0, 0.0, 960.0, false).0, settle_ms(480.0, 960.0));
        assert_eq!(
            release(480.0, -800.0, 960.0, true).0,
            settle_ms(480.0, 960.0)
        );
    }

    #[test]
    fn a_sample_takes_its_position_and_time_from_the_touch() {
        use crate::input::TouchPhase;
        let s = Sample::of(&TouchEvent {
            id: 3,
            phase: TouchPhase::Move,
            x: 12.0,
            y: 34.0,
            at_ns: 56,
        });
        assert_eq!(
            s,
            Sample {
                x: 12.0,
                y: 34.0,
                at_ns: 56
            }
        );
    }
}
