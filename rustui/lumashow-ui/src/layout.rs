//! Size classes: which of a few layouts a screen's usable size calls for.
//!
//! The thresholds are Lucent's (`layout.panel.compact`, `layout.panel.wide`, `layout.short`, `layout.hysteresis`).
//! A size class is a property of the usable bounds, never of a device name, so a fixed panel keeps one class for
//! life and a window or a rotating screen changes class only at a settled size.
//!
//! Hysteresis keeps a size that hovers at a boundary (a scrollbar appearing, a rotation animating) from flipping
//! the layout back and forth: with a previous class, that class holds until the size is [`HYSTERESIS`] units past
//! the boundary, in whichever direction it leaves.

use crate::geometry::LogicalSize;

/// Usable widths below this are [`WidthClass::Compact`].
pub const COMPACT_BELOW: f32 = 680.0;
/// Usable widths from this are [`WidthClass::Expanded`]; those between are [`WidthClass::Medium`].
pub const EXPANDED_FROM: f32 = 900.0;
/// Usable heights up to and including this are [`HeightClass::Short`].
pub const SHORT_AT_MOST: f32 = 500.0;
/// How far past a boundary a size has to go before the class it left is given up.
pub const HYSTERESIS: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WidthClass {
    Compact,
    Medium,
    Expanded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeightClass {
    Short,
    Tall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Orientation {
    Landscape,
    Portrait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SizeClass {
    pub width: WidthClass,
    pub height: HeightClass,
    pub orientation: Orientation,
}

/// The class of a usable size, given the class it had before, if any.
///
/// With no previous class the nominal thresholds decide: width below [`COMPACT_BELOW`] is compact, from
/// [`EXPANDED_FROM`] expanded and between them medium; height up to [`SHORT_AT_MOST`] is short; a square or wider size
/// is landscape. With a previous class, each part of it holds while the size is within [`HYSTERESIS`] of the
/// side it came from, and is replaced by the nominal class once it is not. Leaving a class takes the same distance
/// in either direction.
pub fn classify(size: LogicalSize, previous: Option<SizeClass>) -> SizeClass {
    let (w, h) = (size.width(), size.height());
    SizeClass {
        width: previous
            .map(|p| p.width)
            .filter(|&p| holds_width(p, w))
            .unwrap_or_else(|| nominal_width(w)),
        height: previous
            .map(|p| p.height)
            .filter(|&p| holds_height(p, h))
            .unwrap_or_else(|| nominal_height(h)),
        orientation: previous
            .map(|p| p.orientation)
            .filter(|&p| holds_orientation(p, w, h))
            .unwrap_or_else(|| nominal_orientation(w, h)),
    }
}

fn nominal_width(w: f32) -> WidthClass {
    if w < COMPACT_BELOW {
        WidthClass::Compact
    } else if w < EXPANDED_FROM {
        WidthClass::Medium
    } else {
        WidthClass::Expanded
    }
}

fn nominal_height(h: f32) -> HeightClass {
    if h <= SHORT_AT_MOST {
        HeightClass::Short
    } else {
        HeightClass::Tall
    }
}

fn nominal_orientation(w: f32, h: f32) -> Orientation {
    if w >= h {
        Orientation::Landscape
    } else {
        Orientation::Portrait
    }
}

fn holds_width(previous: WidthClass, w: f32) -> bool {
    match previous {
        WidthClass::Compact => w < COMPACT_BELOW + HYSTERESIS,
        WidthClass::Medium => w > COMPACT_BELOW - HYSTERESIS && w < EXPANDED_FROM + HYSTERESIS,
        WidthClass::Expanded => w > EXPANDED_FROM - HYSTERESIS,
    }
}

fn holds_height(previous: HeightClass, h: f32) -> bool {
    match previous {
        HeightClass::Short => h < SHORT_AT_MOST + HYSTERESIS,
        HeightClass::Tall => h > SHORT_AT_MOST - HYSTERESIS,
    }
}

fn holds_orientation(previous: Orientation, w: f32, h: f32) -> bool {
    match previous {
        Orientation::Landscape => h - w < HYSTERESIS,
        Orientation::Portrait => w - h < HYSTERESIS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(w: f32, h: f32) -> LogicalSize {
        LogicalSize::new(w, h).unwrap()
    }

    fn width_of(w: f32, previous: Option<WidthClass>) -> WidthClass {
        let previous = previous.map(|width| SizeClass {
            width,
            height: HeightClass::Short,
            orientation: Orientation::Landscape,
        });
        classify(size(w, 400.0), previous).width
    }

    fn height_of(h: f32, previous: Option<HeightClass>) -> HeightClass {
        let previous = previous.map(|height| SizeClass {
            width: WidthClass::Expanded,
            height,
            orientation: Orientation::Landscape,
        });
        classify(size(1000.0, h), previous).height
    }

    #[test]
    fn the_nominal_thresholds_are_lucents() {
        for (w, class) in [
            (1.0, WidthClass::Compact),
            (679.0, WidthClass::Compact),
            (679.9, WidthClass::Compact),
            (680.0, WidthClass::Medium),
            (899.0, WidthClass::Medium),
            (899.9, WidthClass::Medium),
            (900.0, WidthClass::Expanded),
            (2000.0, WidthClass::Expanded),
        ] {
            assert_eq!(width_of(w, None), class, "width {w}");
        }
        for (h, class) in [
            (100.0, HeightClass::Short),
            (500.0, HeightClass::Short),
            (500.1, HeightClass::Tall),
            (501.0, HeightClass::Tall),
            (1280.0, HeightClass::Tall),
        ] {
            assert_eq!(height_of(h, None), class, "height {h}");
        }
    }

    #[test]
    fn the_echo_show_is_expanded_short_landscape_and_a_tall_portrait_screen_is_not() {
        assert_eq!(
            classify(size(960.0, 480.0), None),
            SizeClass {
                width: WidthClass::Expanded,
                height: HeightClass::Short,
                orientation: Orientation::Landscape
            }
        );
        assert_eq!(
            classify(size(800.0, 1280.0), None),
            SizeClass {
                width: WidthClass::Medium,
                height: HeightClass::Tall,
                orientation: Orientation::Portrait
            }
        );
        // A square is landscape, and the usable bounds decide, not a device name.
        assert_eq!(
            classify(size(600.0, 600.0), None).orientation,
            Orientation::Landscape
        );
    }

    #[test]
    fn a_class_is_held_until_the_size_is_twenty_units_past_the_boundary() {
        use WidthClass::*;
        // (previous class, width, class that results): the edges of each held region, both sides.
        for (previous, w, wanted) in [
            (Compact, 699.9, Compact),
            (Compact, 700.0, Medium),
            (Medium, 660.1, Medium),
            (Medium, 660.0, Compact),
            (Medium, 919.9, Medium),
            (Medium, 920.0, Expanded),
            (Expanded, 880.1, Expanded),
            (Expanded, 880.0, Medium),
            // within the band, whichever class it was in stays
            (Medium, 690.0, Medium),
            (Compact, 690.0, Compact),
            (Medium, 899.0, Medium),
            (Expanded, 899.0, Expanded),
            // a jump past the next class lands on the class the size is in
            (Compact, 950.0, Expanded),
            (Expanded, 100.0, Compact),
        ] {
            assert_eq!(width_of(w, Some(previous)), wanted, "{previous:?} at {w}");
        }
        use HeightClass::*;
        for (previous, h, wanted) in [
            (Short, 519.9, Short),
            (Short, 520.0, Tall),
            (Tall, 480.1, Tall),
            (Tall, 480.0, Short),
            (Short, 510.0, Short),
            (Tall, 490.0, Tall),
        ] {
            assert_eq!(height_of(h, Some(previous)), wanted, "{previous:?} at {h}");
        }
    }

    #[test]
    fn a_size_that_wobbles_around_a_boundary_never_changes_class() {
        // A scrollbar's worth of width either side of 680 and 900, and of height either side of 500.
        for centre in [COMPACT_BELOW, EXPANDED_FROM] {
            for start in [centre - 5.0, centre + 5.0] {
                let mut class = classify(size(start, 400.0), None);
                let first = class;
                for step in 0..200 {
                    let wobble = if step % 2 == 0 { -15.0 } else { 15.0 };
                    class = classify(size(centre + wobble, 400.0), Some(class));
                    assert_eq!(
                        class.width, first.width,
                        "centre {centre}, start {start}, step {step}"
                    );
                }
            }
        }
        let mut class = classify(size(1000.0, 495.0), None);
        let first = class;
        for step in 0..200 {
            let h = if step % 2 == 0 { 485.0 } else { 515.0 };
            class = classify(size(1000.0, h), Some(class));
            assert_eq!(class.height, first.height);
        }
    }

    #[test]
    fn a_slow_sweep_changes_class_once_at_each_boundary_each_way_and_at_different_sizes() {
        let sweep = |range: Vec<f32>| {
            let mut class = classify(size(range[0], 400.0), None);
            let mut changes = vec![];
            for &w in &range[1..] {
                let next = classify(size(w, 400.0), Some(class));
                if next.width != class.width {
                    changes.push((w, next.width));
                }
                class = next;
            }
            changes
        };
        let up: Vec<f32> = (1..=1000).map(|w| w as f32).collect();
        assert_eq!(
            sweep(up.clone()),
            [(700.0, WidthClass::Medium), (920.0, WidthClass::Expanded)]
        );
        let down: Vec<f32> = up.into_iter().rev().collect();
        assert_eq!(
            sweep(down),
            [(880.0, WidthClass::Medium), (660.0, WidthClass::Compact)]
        );
    }

    #[test]
    fn orientation_follows_the_usable_bounds_with_the_same_hysteresis() {
        let at = |w, h, previous| {
            classify(
                size(w, h),
                Some(SizeClass {
                    width: WidthClass::Expanded,
                    height: HeightClass::Tall,
                    orientation: previous,
                }),
            )
            .orientation
        };
        use Orientation::*;
        assert_eq!(at(800.0, 819.9, Landscape), Landscape);
        assert_eq!(at(800.0, 820.0, Landscape), Portrait);
        assert_eq!(at(820.0, 800.1, Portrait), Portrait);
        assert_eq!(at(820.0, 800.0, Portrait), Landscape);
        assert_eq!(at(1000.0, 400.0, Portrait), Landscape);
    }
}
