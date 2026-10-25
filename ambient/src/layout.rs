//! Where the lights are and which picture areas feed each one.
//!
//! A light sits at a point in picture coordinates ((0, 0) top left of the picture, (1, 1) bottom right; behind or
//! beside the set is outside that range, e.g. a backlight above the top edge is at y = -0.1). The settings screen
//! places lights by dragging them around a drawing of the TV; a strip is a row of lights from [`strip`].
//!
//! The areas are rectangles that overlap, not tiles, so a light's weight on an area is the area's reach to the light:
//! `exp(-(d / spread)^2)` with `d` the distance from the light to the nearest point of the rectangle, times
//! `area^-locality` so that a small area near the light counts for more than the big half-screen areas that also cover
//! it. Weights are normalised per light. Each of the four hardware presets has its own matrix, built once.

use crate::rgb::Rgb;
use crate::zones::{MAX_ZONES, Preset, ZoneFrame};
use serde::{Deserialize, Serialize};

/// Where a light's colour is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// A channel of the Hue entertainment configuration (0..=19).
    Hue { channel: u8 },
    /// An LED of the WLED strip (index along the strip).
    Wled { led: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Light {
    pub target: Target,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Falloff {
    /// Reach of an area beyond its edge, in picture widths. Small values make a light see only the areas it is in or
    /// next to; large values blend the whole picture.
    pub spread: f32,
    /// 0 treats big and small areas alike; 1 weights by 1/area.
    pub locality: f32,
}

impl Default for Falloff {
    fn default() -> Self {
        Falloff { spread: 0.18, locality: 0.6 }
    }
}

/// `count` lights evenly spaced from `start` to `end` (both included), WLED LEDs `first_led`, `first_led + 1`, ...
pub fn strip(start: (f32, f32), end: (f32, f32), count: usize, first_led: u16) -> Vec<Light> {
    (0..count)
        .map(|i| {
            let t = if count > 1 { i as f32 / (count - 1) as f32 } else { 0.5 };
            Light {
                target: Target::Wled { led: first_led + i as u16 },
                x: start.0 + (end.0 - start.0) * t,
                y: start.1 + (end.1 - start.1) * t,
            }
        })
        .collect()
}

/// Weights of every area on every light, for one preset.
#[derive(Clone, Debug)]
pub struct Mapping {
    preset: Preset,
    rows: Vec<[f32; MAX_ZONES]>,
}

impl Mapping {
    pub fn compile(lights: &[Light], preset: Preset, falloff: Falloff) -> Mapping {
        let rects = preset.rects();
        let spread = falloff.spread.max(1e-3);
        let rows = lights
            .iter()
            .map(|l| {
                let mut w = [0.0f32; MAX_ZONES];
                let mut nearest = (0usize, f32::MAX);
                for (i, r) in rects.iter().enumerate() {
                    let d = r.distance(l.x, l.y);
                    if d < nearest.1 {
                        nearest = (i, d);
                    }
                    let k = (d / spread).powi(2);
                    w[i] = (-k).exp() * r.area().max(1e-3).powf(-falloff.locality);
                }
                let max = w.iter().cloned().fold(0.0f32, f32::max);
                let sum: f32 = w.iter().filter(|&&v| v > max * 1e-3).sum();
                if sum <= 0.0 || !sum.is_finite() || max < 1e-12 {
                    // Far outside every area: follow the nearest one.
                    w = [0.0; MAX_ZONES];
                    w[nearest.0] = 1.0;
                } else {
                    for v in w.iter_mut() {
                        *v = if *v > max * 1e-3 { *v / sum } else { 0.0 };
                    }
                }
                w
            })
            .collect();
        Mapping { preset, rows }
    }

    pub fn preset(&self) -> Preset {
        self.preset
    }

    pub fn lights(&self) -> usize {
        self.rows.len()
    }

    pub fn weights(&self, light: usize) -> &[f32; MAX_ZONES] {
        &self.rows[light]
    }

    /// One colour per light from a frame captured with this mapping's preset.
    pub fn apply(&self, frame: &ZoneFrame, out: &mut Vec<Rgb>) {
        debug_assert_eq!(frame.preset, self.preset);
        out.clear();
        out.extend(self.rows.iter().map(|row| {
            let mut c = Rgb::BLACK;
            for (z, &w) in row.iter().enumerate() {
                if w > 0.0 {
                    c = c + frame.color[z] * w;
                }
            }
            c
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hue(x: f32, y: f32) -> Light {
        Light { target: Target::Hue { channel: 0 }, x, y }
    }

    #[test]
    fn weights_sum_to_one_for_every_preset_and_place() {
        for p in Preset::ALL {
            for &(x, y) in &[(0.5, 0.5), (0.0, 0.0), (1.0, 1.0), (-0.3, 0.5), (0.5, -0.2), (3.0, 3.0), (0.13, 0.9)] {
                let m = Mapping::compile(&[hue(x, y)], p, Falloff::default());
                let s: f32 = m.weights(0).iter().sum();
                assert!((s - 1.0).abs() < 1e-5, "{p:?} ({x},{y}) sums to {s}");
                assert!(m.weights(0).iter().all(|&w| w >= 0.0));
                assert!(m.weights(0)[p.zone_count()..].iter().all(|&w| w == 0.0));
            }
        }
    }

    #[test]
    fn a_light_follows_the_areas_next_to_it() {
        let m = Mapping::compile(&[hue(-0.1, 0.5), hue(1.1, 0.5), hue(0.5, -0.1)], Preset::Plain, Falloff::default());
        let heaviest = |l: usize| m.weights(l).iter().enumerate().fold((0, 0.0), |a, (i, &w)| if w > a.1 { (i, w) } else { a }).0;
        // Left of the set: the left strip (15) or the left-middle area (6), never a right-hand one.
        assert!([15, 6].contains(&heaviest(0)), "{}", heaviest(0));
        assert!([10, 9, 13].contains(&heaviest(1)), "{}", heaviest(1));
        assert!([14, 11, 3, 12].contains(&heaviest(2)), "{}", heaviest(2));
        // A left light has no weight on the right-hand half area (8).
        assert!(m.weights(0)[8] < 0.02);
    }

    #[test]
    fn far_outside_falls_back_to_the_nearest_area() {
        let m = Mapping::compile(&[hue(40.0, 0.5)], Preset::Plain, Falloff { spread: 0.05, locality: 0.0 });
        let w = m.weights(0);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert_eq!(w.iter().filter(|&&v| v > 0.0).count(), 1);
    }

    #[test]
    fn locality_prefers_small_areas() {
        let l = hue(0.05, 0.5); // inside the left strip (15), the left-middle (6) and the left half (4)
        let flat = Mapping::compile(&[l], Preset::Plain, Falloff { spread: 0.1, locality: 0.0 });
        let local = Mapping::compile(&[l], Preset::Plain, Falloff { spread: 0.1, locality: 1.0 });
        assert!(local.weights(0)[15] > flat.weights(0)[15]);
        assert!(local.weights(0)[4] < flat.weights(0)[4]);
    }

    #[test]
    fn apply_blends_zone_colours() {
        let mut f = ZoneFrame::blank(Preset::Plain);
        for c in f.color.iter_mut() {
            *c = Rgb::new(0.2, 0.4, 0.6);
        }
        let m = Mapping::compile(&[hue(0.5, 0.5), hue(0.0, 0.0)], Preset::Plain, Falloff::default());
        let mut out = Vec::new();
        m.apply(&f, &mut out);
        assert_eq!(out.len(), 2);
        for c in out {
            assert!((c.r - 0.2).abs() < 1e-5 && (c.g - 0.4).abs() < 1e-5 && (c.b - 0.6).abs() < 1e-5);
        }
    }

    #[test]
    fn strip_places_lights_along_the_line() {
        let s = strip((0.0, -0.1), (1.0, -0.1), 5, 10);
        assert_eq!(s.len(), 5);
        assert_eq!(s[0].target, Target::Wled { led: 10 });
        assert_eq!(s[4].target, Target::Wled { led: 14 });
        assert!((s[2].x - 0.5).abs() < 1e-6 && (s[2].y + 0.1).abs() < 1e-6);
        assert_eq!(strip((0.0, 0.0), (1.0, 0.0), 1, 0)[0].x, 0.5);
    }
}
