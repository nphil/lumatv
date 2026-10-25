//! The helper-side pipeline: captured areas in, one colour per light out.
//!
//! ```text
//!   capture thread                          output thread (25..60 Hz)
//!   ZoneFrame --push_frame--> bars ----+
//!                             delay ---+--tick--> map -> dark gate -> shape -> smooth --> lights
//! ```
//!
//! `push_frame` is called at the capture rate with the capture time; `tick` at the output rate and returns the
//! colours to send. Both take a millisecond clock from the caller (any monotonic base), so nothing here reads the
//! time and every behaviour is testable with synthetic timestamps. [`Engine::wanted_preset`] is what the capture
//! process should be running; the helper relays a change to it (the 32-bit capture helper calls
//! `vtCapture_setPresetHistogram`).
//!
//! Capture stops while the screen is off or Home is showing (the lights then fade out after `stale_ms`), and a
//! disabled engine fades them out at once; both use the same smoother, so they never cut to black.

use crate::bars::{BarConfig, BarDetector, Bars};
use crate::delay::DelayLine;
use crate::layout::{Falloff, Light, Mapping, Target};
use crate::rgb::Rgb;
use crate::smooth::{SmoothConfig, Smoother};
use crate::tuning::{DarkConfig, DarkGate, LightTuning, Tuning};
use crate::zones::{FrameError, FrameParams, Preset, RawZone, ZoneFrame};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_DELAY_MS: u32 = 2000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub frame: FrameParams,
    pub falloff: Falloff,
    pub tuning: Tuning,
    /// Per light, in the order of the light list; lights past the end of this list use the defaults.
    pub light_tuning: Vec<LightTuning>,
    pub dark: DarkConfig,
    pub smooth: SmoothConfig,
    pub bars: BarConfig,
    /// Lights follow the picture this late, 0..=2000 ms.
    pub delay_ms: u32,
    /// Capture with the games preset (edge-weighted, no bar handling).
    pub games: bool,
    /// No frame for this long and the lights fade out.
    pub stale_ms: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            frame: FrameParams::default(),
            falloff: Falloff::default(),
            tuning: Tuning::default(),
            light_tuning: Vec::new(),
            dark: DarkConfig::default(),
            smooth: SmoothConfig::default(),
            bars: BarConfig::default(),
            delay_ms: 0,
            games: false,
            stale_ms: 1500,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    NoLights,
    TooManyLights(usize),
    LightTuning { given: usize, lights: usize },
    Tuning,
    Delay(u32),
    Falloff,
    Smooth,
    Dark,
    Bars,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::NoLights => write!(f, "no lights"),
            ConfigError::TooManyLights(n) => {
                write!(f, "{n} lights is more than the pipeline carries")
            }
            ConfigError::LightTuning { given, lights } => {
                write!(f, "{given} per-light tunings for {lights} lights")
            }
            ConfigError::Tuning => write!(
                f,
                "tuning values must be finite, gamma above 0, the rest not negative"
            ),
            ConfigError::Delay(ms) => write!(f, "delay {ms} ms is over {MAX_DELAY_MS}"),
            ConfigError::Falloff => write!(f, "falloff spread must be above 0 and locality finite"),
            ConfigError::Smooth => write!(
                f,
                "smoothing times and anti-flicker values must be finite and not negative"
            ),
            ConfigError::Dark => {
                write!(f, "dark gate threshold, floor and ratios are out of range")
            }
            ConfigError::Bars => write!(f, "bar fractions must be within 0..1"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// The most lights the pipeline carries: a WLED strip of 2 x 300 LEDs plus the Hue channels.
pub const MAX_LIGHTS: usize = 1024;

impl Config {
    pub fn validate(&self, lights: usize) -> Result<(), ConfigError> {
        if lights == 0 {
            return Err(ConfigError::NoLights);
        }
        if lights > MAX_LIGHTS {
            return Err(ConfigError::TooManyLights(lights));
        }
        if self.light_tuning.len() > lights {
            return Err(ConfigError::LightTuning {
                given: self.light_tuning.len(),
                lights,
            });
        }
        let lt_ok = self.light_tuning.iter().all(|t| {
            t.gain.is_finite()
                && t.gain >= 0.0
                && t.white_balance.iter().all(|w| w.is_finite() && *w >= 0.0)
        });
        if !self.tuning.is_valid() || !lt_ok {
            return Err(ConfigError::Tuning);
        }
        if self.delay_ms > MAX_DELAY_MS {
            return Err(ConfigError::Delay(self.delay_ms));
        }
        if !(self.falloff.spread.is_finite()
            && self.falloff.spread > 0.0
            && self.falloff.locality.is_finite())
        {
            return Err(ConfigError::Falloff);
        }
        let af = self.smooth.anti_flicker;
        if ![af.threshold, af.step]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0)
        {
            return Err(ConfigError::Smooth);
        }
        let d = self.dark;
        if !(d.threshold.is_finite()
            && (0.0..=1.0).contains(&d.threshold)
            && (0.0..=1.0).contains(&d.floor))
            || !(d.enter_ratio > 0.0
                && d.enter_ratio <= 1.0
                && d.exit_ratio >= 1.0
                && d.exit_ratio.is_finite())
        {
            return Err(ConfigError::Dark);
        }
        let b = self.bars;
        if ![b.strip_black, b.bottom_black, b.picture_min]
            .iter()
            .all(|v| (0.0..=1.0).contains(v))
        {
            return Err(ConfigError::Bars);
        }
        Ok(())
    }
}

pub struct Engine {
    cfg: Config,
    lights: Vec<Light>,
    mappings: Vec<Mapping>,
    bars: BarDetector,
    dark: DarkGate,
    smoother: Smoother,
    delay: DelayLine<ZoneFrame>,
    enabled: bool,
    last_frame_ms: Option<u64>,
    forced_off: bool,
    scratch: Vec<Rgb>,
}

impl Engine {
    pub fn new(cfg: Config, lights: Vec<Light>) -> Result<Engine, ConfigError> {
        cfg.validate(lights.len())?;
        let mappings = Preset::ALL
            .iter()
            .map(|&p| Mapping::compile(&lights, p, cfg.falloff))
            .collect();
        let n = lights.len();
        Ok(Engine {
            bars: BarDetector::new(cfg.bars),
            dark: DarkGate::new(cfg.dark, n),
            smoother: Smoother::new(cfg.smooth, n),
            delay: DelayLine::new(256),
            enabled: true,
            last_frame_ms: None,
            forced_off: false,
            scratch: Vec::with_capacity(n),
            mappings,
            lights,
            cfg,
        })
    }

    pub fn lights(&self) -> &[Light] {
        &self.lights
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// The hardware preset the capture should be using now.
    pub fn wanted_preset(&self) -> Preset {
        if self.cfg.games {
            Preset::Games
        } else {
            self.bars.preset()
        }
    }

    pub fn bars(&self) -> Bars {
        self.bars.state()
    }

    pub fn is_dark(&self) -> bool {
        self.dark.is_dark()
    }

    /// Off fades the lights to black; on lets the next frames bring them back.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.delay.clear();
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn push_histograms(
        &mut self,
        now_ms: u64,
        preset: Preset,
        zones: &[RawZone],
    ) -> Result<(), FrameError> {
        let f = ZoneFrame::from_histograms(preset, zones, self.cfg.frame)?;
        self.push_frame(now_ms, f);
        Ok(())
    }

    pub fn push_frame(&mut self, now_ms: u64, frame: ZoneFrame) {
        if !self.enabled {
            return;
        }
        self.last_frame_ms = Some(now_ms);
        self.bars.update(now_ms, &frame);
        self.delay.push(now_ms, frame);
    }

    /// The colours to show at `now_ms`, one per light in list order.
    pub fn tick(&mut self, now_ms: u64) -> &[Rgb] {
        let stale = self
            .last_frame_ms
            .is_none_or(|t| now_ms.saturating_sub(t) > u64::from(self.cfg.stale_ms));
        if self.enabled && !stale {
            self.forced_off = false;
            if let Some(f) = self.delay.pop_due(now_ms, u64::from(self.cfg.delay_ms)) {
                self.process(now_ms, &f);
            }
        } else if !self.forced_off {
            self.forced_off = true;
            self.scratch.clear();
            self.scratch.resize(self.lights.len(), Rgb::BLACK);
            self.smoother.set_target(now_ms, &self.scratch);
        }
        self.smoother.step(now_ms)
    }

    fn process(&mut self, now_ms: u64, f: &ZoneFrame) {
        self.mappings[f.preset.index() as usize].apply(f, &mut self.scratch);
        self.dark.process(now_ms, &mut self.scratch);
        for (i, c) in self.scratch.iter_mut().enumerate() {
            let lt = self.cfg.light_tuning.get(i).copied().unwrap_or_default();
            *c = self.cfg.tuning.shape(*c, &lt);
        }
        self.smoother.set_target(now_ms, &self.scratch);
    }
}

/// Hue entertainment channels from the lights' colours: one entry per distinct channel (the last light on a channel
/// wins), sorted by channel, as 16-bit RGB.
pub fn hue_channels(lights: &[Light], colors: &[Rgb]) -> Vec<(u8, [u16; 3])> {
    let mut by_channel: Vec<(u8, [u16; 3])> = Vec::new();
    for (l, c) in lights.iter().zip(colors) {
        if let Target::Hue { channel } = l.target {
            let v = c.to_u16();
            match by_channel.iter_mut().find(|(ch, _)| *ch == channel) {
                Some(e) => e.1 = v,
                None => by_channel.push((channel, v)),
            }
        }
    }
    by_channel.sort_by_key(|(ch, _)| *ch);
    by_channel
}

/// RGB bytes for a WLED strip of `led_count` LEDs; LEDs without a light stay black, lights past the strip are dropped.
pub fn wled_leds(lights: &[Light], colors: &[Rgb], led_count: usize) -> Vec<u8> {
    let mut out = vec![0u8; led_count * 3];
    for (l, c) in lights.iter().zip(colors) {
        if let Target::Wled { led } = l.target {
            let i = usize::from(led);
            if i < led_count {
                out[i * 3..i * 3 + 3].copy_from_slice(&c.to_u8());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smooth::AntiFlicker;
    use crate::zones::{BOTTOM_STRIP, CENTER, TOP_STRIP};

    fn hue(ch: u8, x: f32, y: f32) -> Light {
        Light {
            target: Target::Hue { channel: ch },
            x,
            y,
        }
    }

    fn fast() -> Config {
        Config {
            smooth: SmoothConfig {
                settle_ms: 100,
                anti_flicker: AntiFlicker {
                    threshold: 0.0,
                    step: 0.0,
                    timeout_ms: 0,
                },
            },
            dark: DarkConfig {
                enabled: false,
                ..DarkConfig::default()
            },
            bars: BarConfig {
                enabled: false,
                ..BarConfig::default()
            },
            ..Config::default()
        }
    }

    /// Left half of the picture red, right half blue.
    fn split_frame(preset: Preset) -> ZoneFrame {
        let mut f = ZoneFrame::blank(preset);
        for (i, r) in preset.rects().iter().enumerate() {
            f.color[i] = if (r.x0 + r.x1) / 2.0 < 0.5 {
                Rgb::new(0.9, 0.0, 0.0)
            } else {
                Rgb::new(0.0, 0.0, 0.9)
            };
            f.count[i] = 1000;
        }
        f
    }

    fn lights() -> Vec<Light> {
        vec![hue(0, -0.1, 0.5), hue(1, 1.1, 0.5)]
    }

    #[test]
    fn left_light_follows_left_of_picture_and_right_follows_right() {
        let mut e = Engine::new(fast(), lights()).unwrap();
        e.push_frame(0, split_frame(Preset::Plain));
        let out = e.tick(0).to_vec();
        assert_eq!(out, vec![Rgb::BLACK; 2], "starts from black and runs");
        let out = e.tick(100).to_vec();
        assert!(out[0].r > 0.5 && out[0].b < 0.3, "{:?}", out[0]);
        assert!(out[1].b > 0.5 && out[1].r < 0.3, "{:?}", out[1]);
    }

    #[test]
    fn the_delay_holds_lights_back_by_exactly_delay_ms() {
        let mut e = Engine::new(
            Config {
                delay_ms: 300,
                ..fast()
            },
            lights(),
        )
        .unwrap();
        e.push_frame(1000, split_frame(Preset::Plain));
        for t in [1000, 1100, 1299] {
            assert_eq!(e.tick(t), &[Rgb::BLACK; 2], "t={t}");
        }
        e.tick(1300); // the frame is due: the run to it starts here
        let mid = e.tick(1350)[0];
        assert!(mid.r > 0.1 && mid.r < 0.9, "{mid:?}");
        assert!(e.tick(1400)[0].r > 0.5);
    }

    #[test]
    fn stale_capture_fades_the_lights_out() {
        let mut e = Engine::new(
            Config {
                stale_ms: 500,
                ..fast()
            },
            lights(),
        )
        .unwrap();
        e.push_frame(0, split_frame(Preset::Plain));
        e.tick(0);
        e.tick(100);
        assert!(e.tick(200)[0].r > 0.5);
        // Frames stop; after stale_ms the target becomes black and the smoother runs there.
        assert!(e.tick(600)[0].r > 0.5, "inside stale_ms still showing");
        e.tick(701);
        assert_eq!(e.tick(900)[0], Rgb::BLACK);
    }

    #[test]
    fn disabling_fades_out_and_ignores_frames_until_re_enabled() {
        let mut e = Engine::new(fast(), lights()).unwrap();
        e.push_frame(0, split_frame(Preset::Plain));
        e.tick(0);
        assert!(e.tick(150)[0].r > 0.5);
        e.set_enabled(false);
        e.push_frame(200, split_frame(Preset::Plain));
        e.tick(200);
        assert_eq!(e.tick(400)[0], Rgb::BLACK);
        e.set_enabled(true);
        e.push_frame(500, split_frame(Preset::Plain));
        e.tick(500);
        assert!(e.tick(650)[0].r > 0.5);
    }

    #[test]
    fn letterbox_frames_ask_for_preset_one_and_that_geometry_is_used() {
        let cfg = Config {
            bars: BarConfig::default(),
            ..fast()
        };
        let mut e = Engine::new(cfg, lights()).unwrap();
        assert_eq!(e.wanted_preset(), Preset::Plain);
        let mut f = split_frame(Preset::Plain);
        f.color[TOP_STRIP] = Rgb::BLACK;
        f.black[TOP_STRIP] = 0.99;
        f.black[BOTTOM_STRIP] = 0.64;
        f.color[CENTER] = Rgb::new(0.4, 0.4, 0.4);
        let mut t = 0;
        while t <= 2000 {
            e.push_frame(t, f.clone());
            e.tick(t);
            t += 33;
        }
        assert_eq!(e.bars(), Bars::Letterbox);
        assert_eq!(e.wanted_preset(), Preset::TopBottom);
        // A frame in the new geometry maps through the preset-1 matrix without complaint.
        e.push_frame(t, split_frame(Preset::TopBottom));
        let _ = e.tick(t + 200);
    }

    #[test]
    fn games_setting_forces_preset_three() {
        let e = Engine::new(
            Config {
                games: true,
                ..fast()
            },
            lights(),
        )
        .unwrap();
        assert_eq!(e.wanted_preset(), Preset::Games);
    }

    #[test]
    fn black_scenes_settle_to_the_dim_floor_not_to_noise() {
        let cfg = Config {
            dark: DarkConfig::default(),
            stale_ms: 60_000,
            ..fast()
        };
        let mut e = Engine::new(cfg, lights()).unwrap();
        let mut t = 0;
        while t < 6000 {
            e.push_frame(t, split_frame(Preset::Plain));
            e.tick(t);
            t += 33;
        }
        let bright = e.tick(t)[0];
        assert!(bright.r > 0.5, "{bright:?}");
        let mut last = Vec::new();
        while t < 12_000 {
            let mut f = ZoneFrame::blank(Preset::Plain);
            let noise = 0.004 + 0.008 * ((t / 33) % 4) as f32 / 3.0;
            for i in 0..16 {
                f.color[i] = Rgb::new(noise, noise, noise);
                f.count[i] = 1000;
            }
            e.push_frame(t, f);
            last.push(e.tick(t)[0]);
            t += 33;
        }
        assert!(e.is_dark());
        let tail = &last[last.len() - 20..];
        assert!(tail.iter().all(|c| *c == tail[0]), "dark output is steady");
        let floor = DarkConfig::default().floor;
        assert!(
            (tail[0].max_channel() - floor).abs() < 0.004,
            "{:?}",
            tail[0]
        );
        assert!(
            tail[0].r > tail[0].b,
            "keeps the red tint of the last bright scene"
        );
    }

    #[test]
    fn per_light_tuning_applies_in_list_order() {
        let cfg = Config {
            light_tuning: vec![LightTuning {
                gain: 0.5,
                white_balance: [1.0; 3],
            }],
            ..fast()
        };
        let mut e = Engine::new(cfg, lights()).unwrap();
        let mut f = ZoneFrame::blank(Preset::Plain);
        for i in 0..16 {
            f.color[i] = Rgb::new(0.8, 0.8, 0.8);
            f.count[i] = 1;
        }
        e.push_frame(0, f);
        e.tick(0);
        let out = e.tick(500).to_vec();
        assert!((out[0].r - 0.4).abs() < 1e-5);
        assert!(
            (out[1].r - 0.8).abs() < 1e-5,
            "second light has no entry, so the default"
        );
    }

    #[test]
    fn config_validation_names_the_problem() {
        let l = lights();
        assert_eq!(Config::default().validate(0), Err(ConfigError::NoLights));
        assert_eq!(
            Config {
                delay_ms: 5000,
                ..Config::default()
            }
            .validate(2),
            Err(ConfigError::Delay(5000))
        );
        assert_eq!(
            Config {
                light_tuning: vec![LightTuning::default(); 3],
                ..Config::default()
            }
            .validate(2),
            Err(ConfigError::LightTuning {
                given: 3,
                lights: 2
            })
        );
        assert_eq!(
            Config {
                tuning: Tuning {
                    gamma: 0.0,
                    ..Tuning::default()
                },
                ..Config::default()
            }
            .validate(2),
            Err(ConfigError::Tuning)
        );
        assert_eq!(
            Config {
                falloff: Falloff {
                    spread: 0.0,
                    locality: 0.5
                },
                ..Config::default()
            }
            .validate(2),
            Err(ConfigError::Falloff)
        );
        assert_eq!(
            Config {
                dark: DarkConfig {
                    floor: 2.0,
                    ..DarkConfig::default()
                },
                ..Config::default()
            }
            .validate(2),
            Err(ConfigError::Dark)
        );
        assert_eq!(
            Config {
                bars: BarConfig {
                    strip_black: 1.5,
                    ..BarConfig::default()
                },
                ..Config::default()
            }
            .validate(2),
            Err(ConfigError::Bars)
        );
        assert!(Config::default().validate(l.len()).is_ok());
        assert!(
            Engine::new(
                Config {
                    delay_ms: 5000,
                    ..Config::default()
                },
                l
            )
            .is_err()
        );
    }

    #[test]
    fn config_survives_json_and_accepts_partial_settings() {
        let c = Config {
            delay_ms: 120,
            games: true,
            light_tuning: vec![LightTuning {
                gain: 0.7,
                white_balance: [1.0, 0.9, 0.8],
            }],
            ..Config::default()
        };
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&s).unwrap(), c);
        let partial: Config =
            serde_json::from_str(r#"{"delay_ms": 250, "tuning": {"brightness": 0.5}}"#).unwrap();
        assert_eq!(partial.delay_ms, 250);
        assert_eq!(partial.tuning.brightness, 0.5);
        assert_eq!(partial.tuning.saturation, 1.0);
        assert_eq!(partial.smooth, SmoothConfig::default());
    }

    #[test]
    fn hue_and_wled_outputs_pick_their_own_targets() {
        let lights = vec![
            hue(3, 0.0, 0.0),
            Light {
                target: Target::Wled { led: 1 },
                x: 0.5,
                y: -0.1,
            },
            hue(1, 1.0, 0.0),
            Light {
                target: Target::Wled { led: 9 },
                x: 0.9,
                y: -0.1,
            },
            hue(1, 1.0, 0.5),
        ];
        let colors = vec![
            Rgb::new(1.0, 0.0, 0.0),
            Rgb::new(0.0, 1.0, 0.0),
            Rgb::new(0.0, 0.0, 1.0),
            Rgb::new(1.0, 1.0, 1.0),
            Rgb::new(0.5, 0.5, 0.5),
        ];
        let h = hue_channels(&lights, &colors);
        assert_eq!(
            h,
            vec![(1, [32768, 32768, 32768]), (3, [65535, 0, 0])],
            "sorted, duplicate channel: last wins"
        );
        let w = wled_leds(&lights, &colors, 4);
        assert_eq!(w.len(), 12);
        assert_eq!(&w[3..6], &[0, 255, 0]);
        assert!(
            w[..3].iter().chain(&w[6..]).all(|&b| b == 0),
            "other LEDs stay black; LED 9 is off the strip"
        );
    }

    #[test]
    fn push_histograms_validates_and_feeds_the_pipeline() {
        let mut e = Engine::new(fast(), lights()).unwrap();
        let zones: Vec<RawZone> = (0..16).map(RawZone::empty).collect();
        assert!(e.push_histograms(0, Preset::Plain, &zones).is_ok());
        assert!(e.push_histograms(0, Preset::Games, &zones).is_err());
        let _ = e.tick(10);
    }
}
