//! The radar page's zoom: one number that the wheel and the D-pad move and the compositor eases, and the pure arithmetic of it.
//!
//! The zoom is a Web Mercator zoom level for a screen 1920 px wide: at 9 a pixel is a quarter of a kilometre and the screen shows about 475 km (how the
//! page opens), each level fewer shows twice the width, and the least is the whole earth. The compositor's track carries the scale
//! `s = 2^(zoom - Z_HOME)`, not the level, because a group's pose is linear in its track: the group's scale is the track, so a turn of the wheel is one
//! request and every frame of the glide costs the app nothing.

/// The most zoomed out: the whole earth, where the helper's globe takes over.
pub const Z_GLOBE: f32 = 4.0;
/// The view the page opens at, and the one the map's pictures are posed for (scale 1).
pub const Z_HOME: f32 = 9.0;
/// Where the wide view (the radar's 1,900 km level) starts: below it the helper's globe is the picture.
pub const Z_WIDE: f32 = 7.0;
/// The most zoomed in: a street map's detail, two levels past the opening view.
pub const Z_MAX: f32 = 11.0;
/// How far a notch of the wheel zooms, and a press of Up or Down (a held key repeats it).
pub const WHEEL_STEP: f32 = 0.25;
pub const KEY_STEP: f32 = 0.5;
/// The scale at the ends of the track.
pub const S_MIN: f32 = 1.0 / 32.0;
pub const S_MAX: f32 = 4.0;

/// The scale of the view at `zoom`.
pub fn scale_of(zoom: f32) -> f32 {
    2f32.powf(zoom - Z_HOME)
}

/// Where the zoom is going, and what it is about.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    pub target: f32,
    /// The place on the screen the zoom keeps still (the pointer, when it is shown); the middle of the screen when none.
    pub anchor: Option<(f32, f32)>,
}

impl Default for Zoom {
    fn default() -> Self {
        Self { target: Z_HOME, anchor: None }
    }
}

impl Zoom {
    /// `d` levels in (negative: out), about `anchor`. Whether the zoom moved: a turn at an end of the range does nothing.
    pub fn by(&mut self, d: f32, anchor: Option<(f32, f32)>) -> bool {
        let to = (self.target + d).clamp(Z_GLOBE, Z_MAX);
        let moved = to != self.target;
        if moved {
            self.target = to;
            self.anchor = anchor;
        }
        moved
    }

    pub fn scale(&self) -> f32 {
        scale_of(self.target)
    }
}

/// How far the picture is moved from the middle of the screen, as a straight line of the scale: `at(s) = a + b * s` on each axis. A zoom keeps what is
/// under its anchor still, so from the scale `s0` the picture is moved (`off0`) and the anchor's place `p` on a screen whose middle is `c`:
/// `off(s) = (p - c) - s * (p - c - off0) / s0`. With no anchor the picture is brought back to the middle as the scale goes to 1 (a picture
/// that is not larger than the screen has nowhere to be but the middle), and moves in proportion as it is zoomed further in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pan {
    pub a: (f32, f32),
    pub b: (f32, f32),
}

impl Pan {
    pub const NONE: Pan = Pan { a: (0.0, 0.0), b: (0.0, 0.0) };

    pub fn at(&self, s: f32) -> (f32, f32) {
        (self.a.0 + self.b.0 * s, self.a.1 + self.b.1 * s)
    }

    pub fn new(anchor: Option<(f32, f32)>, centre: (f32, f32), s0: f32, off0: (f32, f32)) -> Pan {
        match anchor {
            Some(p) => {
                let (dx, dy) = (p.0 - centre.0, p.1 - centre.1);
                Pan { a: (dx, dy), b: (-(dx - off0.0) / s0, -(dy - off0.1) / s0) }
            }
            None if s0 > 1.001 => {
                let b = (off0.0 / (s0 - 1.0), off0.1 / (s0 - 1.0));
                Pan { a: (-b.0, -b.1), b }
            }
            None => Pan::NONE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zoom_runs_from_the_whole_earth_to_the_last_street_and_a_turn_at_either_end_does_nothing() {
        let mut z = Zoom::default();
        assert_eq!((z.target, z.scale()), (Z_HOME, 1.0));
        assert!(z.by(WHEEL_STEP, None));
        assert_eq!(z.target, Z_HOME + WHEEL_STEP);
        for _ in 0..40 {
            z.by(WHEEL_STEP, None);
        }
        assert_eq!(z.target, Z_MAX);
        assert!(!z.by(WHEEL_STEP, Some((1.0, 2.0))), "no further in");
        assert_eq!(z.anchor, None, "and a turn that did nothing does not move the anchor");
        for _ in 0..100 {
            z.by(-WHEEL_STEP, None);
        }
        assert_eq!(z.target, Z_GLOBE);
        assert!(!z.by(-WHEEL_STEP, None), "no further out than the earth");
        assert!(scale_of(Z_GLOBE) >= S_MIN && scale_of(Z_MAX) <= S_MAX, "the track holds the whole range");
    }

    #[test]
    fn a_zoom_about_a_point_keeps_the_place_under_it_where_it_was_at_every_scale() {
        let c = (960.0, 540.0);
        // the picture is at scale 2 and moved by (-30, 12); the pointer is at (1500, 300)
        let (s0, off0) = (2.0_f32, (-30.0_f32, 12.0_f32));
        let p = (1500.0, 300.0);
        let pan = Pan::new(Some(p), c, s0, off0);
        let (ox, oy) = pan.at(s0);
        assert!((ox - off0.0).abs() < 1e-3 && (oy - off0.1).abs() < 1e-3, "no jump when it is taken up at the scale it is at");
        // the place of the picture that is under the pointer at s0 (a point w of the picture at scale 1, before the scale and the move)
        let w = (c.0 + (p.0 - c.0 - off0.0) / s0, c.1 + (p.1 - c.1 - off0.1) / s0);
        for s in [0.5_f32, 1.0, 1.7, 3.0] {
            let (ox, oy) = pan.at(s);
            let screen = (c.0 + s * (w.0 - c.0) + ox, c.1 + s * (w.1 - c.1) + oy);
            assert!((screen.0 - p.0).abs() < 1e-2 && (screen.1 - p.1).abs() < 1e-2, "s {s}: {screen:?}");
        }
    }

    #[test]
    fn with_no_anchor_the_picture_comes_back_to_the_middle_as_the_scale_goes_to_one() {
        let pan = Pan::new(None, (960.0, 540.0), 3.0, (100.0, -40.0));
        assert_eq!(pan.at(3.0), (100.0, -40.0));
        let (x, y) = pan.at(1.0);
        assert!(x.abs() < 1e-4 && y.abs() < 1e-4);
        assert_eq!(Pan::new(None, (960.0, 540.0), 0.5, (7.0, 7.0)), Pan::NONE, "smaller than the screen: in the middle");
    }
}
