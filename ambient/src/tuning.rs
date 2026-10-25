//! Colour shaping per light and the dark gate, after HyperHDR's brightness, saturation, white balance, gamma and
//! "black scenes" handling.
//!
//! Order per light: saturation (in code space, keeping luma), brightness gain, white balance (global then per light),
//! output gamma. The dark gate is scene wide and runs on the mapped colours: when every area is near black the
//! lights hold a faint, slowly tracked tint instead of following the noise in a black picture, which is what makes
//! lights flicker between dim colours in dark scenes.

use crate::rgb::Rgb;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    /// Overall brightness (the quick panel's intensity), 0 off .. 1 as captured; above 1 pushes colours up.
    pub brightness: f32,
    /// 1 as captured, 0 grey, above 1 more vivid.
    pub saturation: f32,
    /// Exponent applied last: 1 leaves codes alone; a bare LED strip that shows linear PWM wants 2.2 or so.
    pub gamma: f32,
    /// Per channel gain for the whole installation.
    pub white_balance: [f32; 3],
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            brightness: 1.0,
            saturation: 1.0,
            gamma: 1.0,
            white_balance: [1.0; 3],
        }
    }
}

/// What differs from light to light: lamps of different makes do not match without it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LightTuning {
    pub gain: f32,
    pub white_balance: [f32; 3],
}

impl Default for LightTuning {
    fn default() -> Self {
        LightTuning {
            gain: 1.0,
            white_balance: [1.0; 3],
        }
    }
}

impl Tuning {
    pub fn shape(&self, c: Rgb, per_light: &LightTuning) -> Rgb {
        let l = c.luma();
        let mut c = Rgb::new(
            l + (c.r - l) * self.saturation,
            l + (c.g - l) * self.saturation,
            l + (c.b - l) * self.saturation,
        )
        .clamp01();
        let k = self.brightness * per_light.gain;
        c = c * k;
        c = Rgb::new(
            c.r * self.white_balance[0] * per_light.white_balance[0],
            c.g * self.white_balance[1] * per_light.white_balance[1],
            c.b * self.white_balance[2] * per_light.white_balance[2],
        )
        .clamp01();
        if (self.gamma - 1.0).abs() > 1e-6 {
            c = Rgb::new(
                c.r.powf(self.gamma),
                c.g.powf(self.gamma),
                c.b.powf(self.gamma),
            );
        }
        c
    }

    pub fn is_valid(&self) -> bool {
        self.brightness.is_finite()
            && self.brightness >= 0.0
            && self.saturation.is_finite()
            && self.saturation >= 0.0
            && self.gamma.is_finite()
            && self.gamma > 0.0
            && self
                .white_balance
                .iter()
                .all(|w| w.is_finite() && *w >= 0.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DarkConfig {
    pub enabled: bool,
    /// Brightest area (largest channel, 0..1) under which the scene counts as dark.
    pub threshold: f32,
    /// Brightness the lights hold in a dark scene, 0 off .. 1; Hue lamps hold a steady dim glow around 0.02.
    pub floor: f32,
    /// The scene must stay under `threshold * enter_ratio` this long before the gate closes.
    pub enter_ms: u32,
    /// It reopens at once when the brightest area passes `threshold * exit_ratio`.
    pub exit_ratio: f32,
    pub enter_ratio: f32,
    /// Time constant of the tint kept from the last bright scenes.
    pub tint_ms: u32,
}

impl Default for DarkConfig {
    fn default() -> Self {
        DarkConfig {
            enabled: true,
            threshold: 0.06,
            floor: 0.02,
            enter_ms: 600,
            exit_ratio: 1.4,
            enter_ratio: 0.8,
            tint_ms: 4000,
        }
    }
}

/// The scene-wide dark state machine and the tint it holds.
#[derive(Clone, Debug)]
pub struct DarkGate {
    cfg: DarkConfig,
    dark: bool,
    below_since: Option<u64>,
    tint: Vec<Rgb>,
    last_ms: Option<u64>,
}

impl DarkGate {
    pub fn new(cfg: DarkConfig, lights: usize) -> DarkGate {
        DarkGate {
            cfg,
            dark: false,
            below_since: None,
            tint: vec![Rgb::new(1.0, 1.0, 1.0); lights],
            last_ms: None,
        }
    }

    pub fn is_dark(&self) -> bool {
        self.dark
    }

    /// Feeds one mapped frame (before shaping); rewrites it to the held tint while the gate is closed.
    pub fn process(&mut self, now_ms: u64, colors: &mut [Rgb]) {
        if !self.cfg.enabled {
            return;
        }
        let level = colors
            .iter()
            .map(|c| c.max_channel())
            .fold(0.0f32, f32::max);
        if self.dark {
            if level > self.cfg.threshold * self.cfg.exit_ratio {
                self.dark = false;
                self.below_since = None;
            }
        } else if level < self.cfg.threshold * self.cfg.enter_ratio {
            let since = *self.below_since.get_or_insert(now_ms);
            if now_ms.saturating_sub(since) >= u64::from(self.cfg.enter_ms) {
                self.dark = true;
            }
        } else {
            self.below_since = None;
        }

        if self.dark {
            for (c, t) in colors.iter_mut().zip(&self.tint) {
                *c = *t * self.cfg.floor;
            }
        } else {
            // Track the colour of bright scenes, each light normalised to its brightest channel, so that the glow a
            // dark scene leaves behind matches the scene that just ended. Only frames with something to see count.
            let dt = self.last_ms.map_or(0, |l| now_ms.saturating_sub(l)) as f32;
            let a = 1.0 - (-dt / self.cfg.tint_ms.max(1) as f32).exp();
            if level >= self.cfg.threshold {
                for (c, t) in colors.iter().zip(self.tint.iter_mut()) {
                    let m = c.max_channel();
                    if m > 1e-4 {
                        *t = t.lerp(*c * (1.0 / m), a);
                    }
                }
            }
        }
        self.last_ms = Some(now_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_tuning_leaves_colours_alone() {
        let t = Tuning::default();
        let c = Rgb::new(0.2, 0.5, 0.9);
        let o = t.shape(c, &LightTuning::default());
        assert!(c.max_delta(o) < 1e-6);
    }

    #[test]
    fn saturation_zero_is_luma_grey_and_two_is_more_vivid() {
        let c = Rgb::new(0.8, 0.4, 0.2);
        let grey = Tuning {
            saturation: 0.0,
            ..Tuning::default()
        }
        .shape(c, &LightTuning::default());
        assert!((grey.r - grey.g).abs() < 1e-6 && (grey.g - grey.b).abs() < 1e-6);
        assert!((grey.r - c.luma()).abs() < 1e-6);
        let vivid = Tuning {
            saturation: 1.5,
            ..Tuning::default()
        }
        .shape(c, &LightTuning::default());
        assert!(vivid.r - vivid.b > c.r - c.b);
    }

    #[test]
    fn brightness_gain_balance_and_gamma_compose() {
        let t = Tuning {
            brightness: 0.5,
            gamma: 2.0,
            white_balance: [1.0, 0.8, 1.0],
            ..Tuning::default()
        };
        let lt = LightTuning {
            gain: 0.5,
            white_balance: [1.0, 1.0, 0.5],
        };
        let o = t.shape(Rgb::new(1.0, 1.0, 1.0), &lt);
        // (1 * 0.5 * 0.5) = 0.25 gain; then channel factors; then squared.
        assert!((o.r - 0.25f32.powi(2)).abs() < 1e-6);
        assert!((o.g - (0.25f32 * 0.8).powi(2)).abs() < 1e-6);
        assert!((o.b - (0.25f32 * 0.5).powi(2)).abs() < 1e-6);
    }

    #[test]
    fn output_never_leaves_the_unit_range() {
        let t = Tuning {
            brightness: 3.0,
            saturation: 3.0,
            white_balance: [2.0; 3],
            ..Tuning::default()
        };
        let o = t.shape(Rgb::new(0.9, 0.1, 0.5), &LightTuning::default());
        for v in [o.r, o.g, o.b] {
            assert!((0.0..=1.0).contains(&v));
        }
    }

    #[test]
    fn validity_catches_nonsense() {
        assert!(Tuning::default().is_valid());
        assert!(
            !Tuning {
                gamma: 0.0,
                ..Tuning::default()
            }
            .is_valid()
        );
        assert!(
            !Tuning {
                brightness: f32::NAN,
                ..Tuning::default()
            }
            .is_valid()
        );
        assert!(
            !Tuning {
                white_balance: [1.0, -1.0, 1.0],
                ..Tuning::default()
            }
            .is_valid()
        );
    }

    fn frame(v: f32) -> Vec<Rgb> {
        vec![Rgb::new(v, v * 0.5, 0.0); 3]
    }

    #[test]
    fn dark_scene_holds_a_dim_tint_instead_of_noise() {
        let mut g = DarkGate::new(DarkConfig::default(), 3);
        // A bright orange scene teaches the tint.
        for i in 0..400 {
            let mut f = frame(0.8);
            g.process(i * 50, &mut f);
        }
        assert!(!g.is_dark());
        // Black with noise: nothing happens until it has lasted enter_ms ...
        let mut t = 20_050;
        let mut f = frame(0.02);
        g.process(t, &mut f);
        assert!(!g.is_dark());
        assert_eq!(f[0], Rgb::new(0.02, 0.01, 0.0));
        // ... then the gate closes and the lights hold floor * tint, whatever the noise is doing.
        let mut seen = Vec::new();
        for k in 0..30 {
            t += 50;
            let mut f = frame(0.005 + 0.01 * (k % 3) as f32);
            g.process(t, &mut f);
            seen.push(f[0]);
        }
        assert!(g.is_dark());
        let held = *seen.last().unwrap();
        assert!(
            seen[seen.len() - 5..].iter().all(|c| *c == held),
            "steady in the dark"
        );
        assert!((held.max_channel() - 0.02).abs() < 1e-3, "{held:?}");
        assert!(
            held.r > held.g && held.b < 0.01,
            "keeps the orange tint: {held:?}"
        );
    }

    #[test]
    fn dark_gate_has_hysteresis_and_reopens_at_once() {
        let cfg = DarkConfig {
            enter_ms: 0,
            ..DarkConfig::default()
        };
        let mut g = DarkGate::new(cfg, 3);
        let mut f = frame(0.01);
        g.process(0, &mut f);
        g.process(10, &mut f);
        assert!(g.is_dark());
        // Just above threshold but under threshold * exit_ratio: stays dark.
        let mut f = frame(cfg.threshold * 1.2);
        g.process(20, &mut f);
        assert!(g.is_dark());
        // Past it: open on the same frame, colours untouched.
        let mut f = frame(cfg.threshold * 1.5);
        let before = f.clone();
        g.process(30, &mut f);
        assert!(!g.is_dark());
        assert_eq!(f, before);
    }

    #[test]
    fn a_dip_shorter_than_enter_ms_does_not_close_the_gate() {
        let mut g = DarkGate::new(DarkConfig::default(), 3);
        for t in [0, 100, 200] {
            let mut f = frame(0.01);
            g.process(t, &mut f);
        }
        let mut f = frame(0.5);
        g.process(300, &mut f);
        for t in [400, 500, 600, 700] {
            let mut f = frame(0.01);
            g.process(t, &mut f);
        }
        assert!(!g.is_dark());
    }

    #[test]
    fn disabled_gate_never_touches_frames() {
        let mut g = DarkGate::new(
            DarkConfig {
                enabled: false,
                ..DarkConfig::default()
            },
            3,
        );
        let mut f = frame(0.0);
        for t in 0..100 {
            g.process(t * 100, &mut f);
        }
        assert!(!g.is_dark());
        assert_eq!(f, frame(0.0));
    }
}
