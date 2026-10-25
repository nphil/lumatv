//! A colour as three floats in 0..1, gamma-coded the way the picture is (the histogram counts code values).

use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb::new(0.0, 0.0, 0.0);

    pub const fn new(r: f32, g: f32, b: f32) -> Rgb {
        Rgb { r, g, b }
    }

    /// Rec. 709 weights on the code values; the same figure the probe prints as `luma`.
    pub fn luma(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }

    pub fn max_channel(self) -> f32 {
        self.r.max(self.g).max(self.b)
    }

    pub fn clamp01(self) -> Rgb {
        Rgb::new(
            self.r.clamp(0.0, 1.0),
            self.g.clamp(0.0, 1.0),
            self.b.clamp(0.0, 1.0),
        )
    }

    pub fn lerp(self, to: Rgb, t: f32) -> Rgb {
        self + (to - self) * t
    }

    /// Largest per-channel difference, the distance the smoother's anti-flicker compares.
    pub fn max_delta(self, other: Rgb) -> f32 {
        (self.r - other.r)
            .abs()
            .max((self.g - other.g).abs())
            .max((self.b - other.b).abs())
    }

    pub fn to_u8(self) -> [u8; 3] {
        let c = self.clamp01();
        [
            q(c.r, 255.0) as u8,
            q(c.g, 255.0) as u8,
            q(c.b, 255.0) as u8,
        ]
    }

    pub fn to_u16(self) -> [u16; 3] {
        let c = self.clamp01();
        [
            q(c.r, 65535.0) as u16,
            q(c.g, 65535.0) as u16,
            q(c.b, 65535.0) as u16,
        ]
    }
}

fn q(v: f32, max: f32) -> f32 {
    (v * max + 0.5).floor().clamp(0.0, max)
}

impl Add for Rgb {
    type Output = Rgb;
    fn add(self, o: Rgb) -> Rgb {
        Rgb::new(self.r + o.r, self.g + o.g, self.b + o.b)
    }
}

impl Sub for Rgb {
    type Output = Rgb;
    fn sub(self, o: Rgb) -> Rgb {
        Rgb::new(self.r - o.r, self.g - o.g, self.b - o.b)
    }
}

impl Mul<f32> for Rgb {
    type Output = Rgb;
    fn mul(self, k: f32) -> Rgb {
        Rgb::new(self.r * k, self.g * k, self.b * k)
    }
}
