//! Logical geometry: sizes and insets in UI logical units, and how a logical unit relates to display pixels.
//!
//! The UI is laid out in logical units. One unit is [`Scale`] display pixels, and the display's own pixel size
//! stays upright (rotation is the device layer's business and happens once). Anything that arrives in pixels is
//! divided by the scale once, at the host; code in the UI never converts a second time.

use std::fmt;

/// Why a size, inset or scale cannot be one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryError {
    /// A size must be finite and above zero in both directions.
    BadSize,
    /// An inset must be finite and not negative.
    BadInset,
    /// A scale must be finite and above zero.
    BadScale,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BadSize => "a size must be finite and above zero",
            Self::BadInset => "an inset must be finite and not negative",
            Self::BadScale => "a scale must be finite and above zero",
        })
    }
}

impl std::error::Error for GeometryError {}

/// A width and a height in logical units, both finite and above zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogicalSize {
    width: f32,
    height: f32,
}

impl LogicalSize {
    pub fn new(width: f32, height: f32) -> Result<Self, GeometryError> {
        let ok = |v: f32| v.is_finite() && v > 0.0;
        if ok(width) && ok(height) {
            Ok(Self { width, height })
        } else {
            Err(GeometryError::BadSize)
        }
    }

    pub fn width(self) -> f32 {
        self.width
    }

    pub fn height(self) -> f32 {
        self.height
    }
}

/// The distances to keep clear of each edge, in logical units, each finite and not negative.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

impl Insets {
    pub const ZERO: Self = Self {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };

    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Result<Self, GeometryError> {
        let ok = |v: f32| v.is_finite() && v >= 0.0;
        if ok(top) && ok(right) && ok(bottom) && ok(left) {
            Ok(Self {
                top,
                right,
                bottom,
                left,
            })
        } else {
            Err(GeometryError::BadInset)
        }
    }

    pub fn top(self) -> f32 {
        self.top
    }

    pub fn right(self) -> f32 {
        self.right
    }

    pub fn bottom(self) -> f32 {
        self.bottom
    }

    pub fn left(self) -> f32 {
        self.left
    }

    /// What is left of `size` inside these insets, or `None` when they leave nothing.
    pub fn shrink(self, size: LogicalSize) -> Option<LogicalSize> {
        LogicalSize::new(
            size.width - self.left - self.right,
            size.height - self.top - self.bottom,
        )
        .ok()
    }
}

/// Display pixels per logical unit: 1 on a screen drawn pixel for pixel, 2 on one drawn at twice the size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale(f32);

impl Scale {
    pub fn new(pixels_per_unit: f32) -> Result<Self, GeometryError> {
        if pixels_per_unit.is_finite() && pixels_per_unit > 0.0 {
            Ok(Self(pixels_per_unit))
        } else {
            Err(GeometryError::BadScale)
        }
    }

    pub fn pixels_per_unit(self) -> f32 {
        self.0
    }

    /// Display pixels to logical units. A host calls this once on each input coordinate.
    pub fn to_logical(self, pixels: f32) -> f32 {
        pixels / self.0
    }

    pub fn to_pixels(self, units: f32) -> f32 {
        units * self.0
    }
}

/// A rectangle in logical units: where a control, a card or a layer rests, top left and size. Unlike [`LogicalSize`] it may
/// be empty (a collapsed row has no height).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Frame {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Whether the point is inside: the left and top edges count, the right and bottom do not, so two frames that meet share
    /// no point and a finger on the seam hits exactly one.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// This frame shrunk by `by` on every side.
    pub fn inset(&self, by: f32) -> Self {
        Self::new(
            self.x + by,
            self.y + by,
            self.w - 2.0 * by,
            self.h - 2.0 * by,
        )
    }

    /// This frame grown by `by` on every side (a hit area larger than what is drawn).
    pub fn outset(&self, by: f32) -> Self {
        self.inset(-by)
    }

    /// The same size moved by (`dx`, `dy`).
    pub fn moved(&self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// Where this frame and `other` overlap, if they do.
    pub fn intersect(&self, other: &Frame) -> Option<Frame> {
        let (x, y) = (self.x.max(other.x), self.y.max(other.y));
        let (r, b) = (
            self.right().min(other.right()),
            self.bottom().min(other.bottom()),
        );
        (r > x && b > y).then(|| Frame::new(x, y, r - x, b - y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_finite_and_above_zero_in_both_directions() {
        assert!(LogicalSize::new(960.0, 480.0).is_ok());
        for (w, h) in [
            (0.0, 480.0),
            (960.0, 0.0),
            (-1.0, 480.0),
            (f32::NAN, 480.0),
            (960.0, f32::INFINITY),
        ] {
            assert_eq!(
                LogicalSize::new(w, h),
                Err(GeometryError::BadSize),
                "{w}x{h}"
            );
        }
    }

    #[test]
    fn insets_are_not_negative_and_shrink_a_size_or_leave_nothing() {
        let insets = Insets::new(24.0, 10.0, 20.0, 10.0).unwrap();
        let size = LogicalSize::new(960.0, 480.0).unwrap();
        assert_eq!(
            insets.shrink(size),
            Some(LogicalSize::new(940.0, 436.0).unwrap())
        );
        assert_eq!(Insets::ZERO.shrink(size), Some(size));
        assert_eq!(
            Insets::new(240.0, 0.0, 240.0, 0.0).unwrap().shrink(size),
            None
        );
        assert_eq!(
            Insets::new(0.0, 480.0, 0.0, 480.0).unwrap().shrink(size),
            None
        );
        for bad in [-0.5, f32::NAN, f32::INFINITY] {
            assert_eq!(
                Insets::new(bad, 0.0, 0.0, 0.0),
                Err(GeometryError::BadInset)
            );
            assert_eq!(
                Insets::new(0.0, 0.0, 0.0, bad),
                Err(GeometryError::BadInset)
            );
        }
    }

    #[test]
    fn a_scale_converts_pixels_to_units_once_and_back_exactly_for_powers_of_two() {
        let one = Scale::new(1.0).unwrap();
        assert_eq!(one.to_logical(960.0), 960.0);
        let two = Scale::new(2.0).unwrap();
        assert_eq!(two.to_logical(960.0), 480.0);
        assert_eq!(two.to_pixels(two.to_logical(959.0)), 959.0);
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(Scale::new(bad), Err(GeometryError::BadScale));
        }
    }
}
