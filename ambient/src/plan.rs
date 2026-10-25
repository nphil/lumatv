//! What the person chose in Settings (the page's `base.ambient` values) turned into what the engine runs.
//!
//! The helper reads the settings store and fills a [`Settings`]; [`Settings::plan`] makes the engine's [`Config`], the list of
//! lights and the still colour from it, with no I/O, so the whole mapping is a function that can be tested on the workstation.
//!
//! | Setting | Becomes |
//! |---|---|
//! | brightness 0..100 % | `tuning.brightness` 0..1 (and the scale of the still colour) |
//! | saturation 50..150 % | `tuning.saturation` 0.5..1.5 |
//! | keep a glow in dark scenes | the dark gate's `floor` (its default, or 0 for none) |
//! | skip black bars | `bars.enabled` (the hardware preset follows letterbox and pillarbox) |
//! | delay 0..2000 ms | `delay_ms` |
//! | top, right, bottom, left sections | one light per section, clockwise from the top-left corner, WLED LED 0, 1, 2, ... |
//! | weight the edges for games | kept as `game_edges`: the runtime switches the engine to the games preset while a game is on |
//! | mode | picture, a still colour, or the picture but dark whenever Home or a menu covers it |

use crate::engine::{Config, ConfigError, MAX_LIGHTS};
use crate::layout::{Light, Target};
use crate::rgb::Rgb;
use crate::tuning::DarkConfig;
use std::fmt;

/// How far outside the picture's edge the strip sits, in picture widths and heights: behind the frame, a hand from the glass.
const OUTSIDE: f32 = 0.04;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The lights follow the picture.
    Picture,
    /// One colour, the still colour.
    Colour,
    /// The lights follow the picture and are dark while Home or a menu is in front of it.
    PictureOffHome,
}

impl Mode {
    pub fn parse(word: &str) -> Option<Mode> {
        match word {
            "picture" => Some(Mode::Picture),
            "colour" => Some(Mode::Colour),
            "picture-off-home" => Some(Mode::PictureOffHome),
            _ => None,
        }
    }

    /// The choice id Settings stores.
    pub fn word(self) -> &'static str {
        match self {
            Mode::Picture => "picture",
            Mode::Colour => "colour",
            Mode::PictureOffHome => "picture-off-home",
        }
    }

    pub fn follows_picture(self) -> bool {
        !matches!(self, Mode::Colour)
    }
}

/// The still colours Settings offers. Rosé Pine's rose, iris and foam are the palette's own; the whites are warm (about 3000 K) and
/// cool (about 7500 K) as an RGB strip shows them: first guesses, to be matched by eye on the strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Still {
    Warm,
    Cool,
    Rose,
    Iris,
    Foam,
}

impl Still {
    pub fn parse(word: &str) -> Option<Still> {
        match word {
            "warm" => Some(Still::Warm),
            "cool" => Some(Still::Cool),
            "rose" => Some(Still::Rose),
            "iris" => Some(Still::Iris),
            "foam" => Some(Still::Foam),
            _ => None,
        }
    }

    pub fn rgb(self) -> Rgb {
        let c = |r: u8, g: u8, b: u8| {
            Rgb::new(
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
            )
        };
        match self {
            Still::Warm => c(255, 180, 107),
            Still::Cool => c(214, 235, 255),
            Still::Rose => c(0xeb, 0xbc, 0xba),
            Still::Iris => c(0xc4, 0xa7, 0xe7),
            Still::Foam => c(0x9c, 0xcf, 0xd8),
        }
    }
}

/// The sections of the strip on each edge, `top` along the top from the left corner, then `right` down, `bottom` back along the
/// bottom and `left` up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edges {
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    pub left: u16,
}

impl Edges {
    pub fn sum(self) -> usize {
        usize::from(self.top)
            + usize::from(self.right)
            + usize::from(self.bottom)
            + usize::from(self.left)
    }
}

/// The ambient light settings, in their own units, with the schema's defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub mode: Mode,
    /// Percent, 0..=100.
    pub brightness: u8,
    pub still: Still,
    /// Percent, 50..=150.
    pub saturation: u8,
    pub dark_hold: bool,
    pub skip_bars: bool,
    pub game_edges: bool,
    pub delay_ms: u32,
    pub edges: Edges,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: false,
            mode: Mode::Picture,
            brightness: 70,
            still: Still::Warm,
            saturation: 100,
            dark_hold: true,
            skip_bars: true,
            game_edges: true,
            delay_ms: 40,
            edges: Edges {
                top: 18,
                right: 11,
                bottom: 18,
                left: 11,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// Every edge has no sections and no other light was given.
    NoLights,
    /// More lights than the pipeline carries.
    TooMany(usize),
    /// The engine refused the configuration the settings make.
    Engine(ConfigError),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::NoLights => write!(f, "no sections on any edge"),
            PlanError::TooMany(n) => write!(
                f,
                "{n} lights is more than the pipeline carries ({MAX_LIGHTS})"
            ),
            PlanError::Engine(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PlanError {}

/// What runs: the engine's configuration, the lights and the still colour.
#[derive(Clone, Debug)]
pub struct Plan {
    pub enabled: bool,
    pub mode: Mode,
    pub engine: Config,
    /// The strip's lights in LED order, then any others the caller gave.
    pub lights: Vec<Light>,
    /// Lights of the strip (the part of `lights` that are WLED LEDs).
    pub sections: usize,
    /// The still colour at the chosen brightness, for [`Mode::Colour`].
    pub still: Rgb,
    /// Whether the engine may switch to the games preset while a game is on.
    pub game_edges: bool,
}

impl Settings {
    /// The plan these settings make for a strip with the sections of `edges`, and `others` (Hue channels) after them.
    pub fn plan(&self, others: &[Light]) -> Result<Plan, PlanError> {
        let mut lights = strip_lights(self.edges);
        let sections = lights.len();
        lights.extend_from_slice(others);
        if lights.is_empty() {
            return Err(PlanError::NoLights);
        }
        if lights.len() > MAX_LIGHTS {
            return Err(PlanError::TooMany(lights.len()));
        }
        let mut engine = Config::default();
        engine.tuning.brightness = f32::from(self.brightness.min(100)) / 100.0;
        engine.tuning.saturation = f32::from(self.saturation.clamp(50, 150)) / 100.0;
        engine.dark.floor = if self.dark_hold {
            DarkConfig::default().floor
        } else {
            0.0
        };
        engine.bars.enabled = self.skip_bars;
        engine.delay_ms = self.delay_ms.min(crate::engine::MAX_DELAY_MS);
        engine.validate(lights.len()).map_err(PlanError::Engine)?;
        Ok(Plan {
            enabled: self.enabled,
            mode: self.mode,
            engine,
            lights,
            sections,
            still: self.still.rgb() * (f32::from(self.brightness.min(100)) / 100.0),
            game_edges: self.game_edges,
        })
    }
}

/// The strip's lights, clockwise from the top-left corner: each edge's sections are centred in equal parts of the edge, just
/// outside the picture, and the LED numbers run on from one edge to the next.
pub fn strip_lights(e: Edges) -> Vec<Light> {
    let mut lights = Vec::with_capacity(e.sum());
    let mut led: u16 = 0;
    let mut edge = |count: u16, from: (f32, f32), to: (f32, f32)| {
        for i in 0..count {
            let t = (f32::from(i) + 0.5) / f32::from(count);
            lights.push(Light {
                target: Target::Wled { led },
                x: from.0 + (to.0 - from.0) * t,
                y: from.1 + (to.1 - from.1) * t,
            });
            led = led.saturating_add(1);
        }
    };
    edge(e.top, (0.0, -OUTSIDE), (1.0, -OUTSIDE));
    edge(e.right, (1.0 + OUTSIDE, 0.0), (1.0 + OUTSIDE, 1.0));
    edge(e.bottom, (1.0, 1.0 + OUTSIDE), (0.0, 1.0 + OUTSIDE));
    edge(e.left, (-OUTSIDE, 1.0), (-OUTSIDE, 0.0));
    lights
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;

    fn wled(l: &Light) -> u16 {
        match l.target {
            Target::Wled { led } => led,
            Target::Hue { .. } => panic!("not a strip light"),
        }
    }

    #[test]
    fn the_defaults_are_the_mocks_strip_of_fifty_eight_sections_clockwise_from_the_top_left() {
        let plan = Settings::default().plan(&[]).unwrap();
        assert_eq!(plan.sections, 58);
        assert_eq!(plan.lights.len(), 58);
        let leds: Vec<u16> = plan.lights.iter().map(wled).collect();
        assert_eq!(leds, (0..58).collect::<Vec<u16>>());
        // Top, left to right, just above the picture.
        assert!(plan.lights[0].x < plan.lights[1].x && plan.lights[0].x < 0.05);
        assert!(plan.lights[..18].iter().all(|l| l.y < 0.0));
        assert!(plan.lights[17].x > 0.95);
        // Right, top to bottom, just beside it.
        assert!(plan.lights[18..29].iter().all(|l| l.x > 1.0));
        assert!(plan.lights[18].y < plan.lights[28].y);
        // Bottom, right to left.
        assert!(plan.lights[29..47].iter().all(|l| l.y > 1.0));
        assert!(plan.lights[29].x > plan.lights[46].x);
        // Left, bottom to top, ending next to where it began.
        assert!(plan.lights[47..].iter().all(|l| l.x < 0.0));
        assert!(plan.lights[47].y > plan.lights[57].y && plan.lights[57].y < 0.1);
        assert!(Engine::new(plan.engine, plan.lights).is_ok());
    }

    #[test]
    fn the_settings_become_the_engines_numbers() {
        let s = Settings {
            enabled: true,
            mode: Mode::PictureOffHome,
            brightness: 35,
            saturation: 130,
            dark_hold: false,
            skip_bars: false,
            game_edges: false,
            delay_ms: 250,
            ..Settings::default()
        };
        let plan = s.plan(&[]).unwrap();
        assert!(plan.enabled && plan.mode == Mode::PictureOffHome && !plan.game_edges);
        assert!((plan.engine.tuning.brightness - 0.35).abs() < 1e-6);
        assert!((plan.engine.tuning.saturation - 1.3).abs() < 1e-6);
        assert_eq!(plan.engine.dark.floor, 0.0);
        assert!(!plan.engine.bars.enabled);
        assert_eq!(plan.engine.delay_ms, 250);
        assert!(!plan.engine.games, "the games preset is the runtime's call");
        let kept = Settings::default().plan(&[]).unwrap();
        assert_eq!(kept.engine.dark.floor, DarkConfig::default().floor);
        assert!(kept.engine.bars.enabled);
        assert_eq!(kept.engine.delay_ms, 40);
    }

    #[test]
    fn the_still_colour_follows_the_brightness_and_every_choice_is_a_colour() {
        for word in ["warm", "cool", "rose", "iris", "foam"] {
            let c = Still::parse(word).unwrap().rgb();
            assert!(
                (0.0..=1.0).contains(&c.r)
                    && (0.0..=1.0).contains(&c.g)
                    && (0.0..=1.0).contains(&c.b)
            );
            assert!(c.max_channel() > 0.8, "{word}");
        }
        assert_eq!(Still::parse("magenta"), None);
        let full = Settings {
            brightness: 100,
            still: Still::Rose,
            ..Settings::default()
        }
        .plan(&[])
        .unwrap();
        let half = Settings {
            brightness: 50,
            still: Still::Rose,
            ..Settings::default()
        }
        .plan(&[])
        .unwrap();
        assert!((half.still.r - full.still.r * 0.5).abs() < 1e-6);
        let off = Settings {
            brightness: 0,
            ..Settings::default()
        }
        .plan(&[])
        .unwrap();
        assert_eq!(off.still, Rgb::BLACK);
    }

    #[test]
    fn modes_round_trip_through_the_words_settings_stores() {
        for mode in [Mode::Picture, Mode::Colour, Mode::PictureOffHome] {
            assert_eq!(Mode::parse(mode.word()), Some(mode));
        }
        assert_eq!(Mode::parse("rainbow"), None);
        assert!(
            Mode::Picture.follows_picture()
                && Mode::PictureOffHome.follows_picture()
                && !Mode::Colour.follows_picture()
        );
    }

    #[test]
    fn no_sections_is_no_plan_unless_other_lights_were_given() {
        let none = Settings {
            edges: Edges {
                top: 0,
                right: 0,
                bottom: 0,
                left: 0,
            },
            ..Settings::default()
        };
        assert_eq!(none.plan(&[]).unwrap_err(), PlanError::NoLights);
        let hue = Light {
            target: Target::Hue { channel: 0 },
            x: -0.1,
            y: 0.5,
        };
        let plan = none.plan(&[hue]).unwrap();
        assert_eq!((plan.sections, plan.lights.len()), (0, 1));
        // The Hue lights come after the strip's.
        let both = Settings::default().plan(&[hue]).unwrap();
        assert_eq!(both.lights.len(), 59);
        assert_eq!(both.lights[58].target, Target::Hue { channel: 0 });
        let many = Settings {
            edges: Edges {
                top: 120,
                right: 120,
                bottom: 120,
                left: 120,
            },
            ..Settings::default()
        };
        assert_eq!(many.plan(&[]).unwrap().lights.len(), 480);
        let too_many = vec![hue; MAX_LIGHTS];
        assert_eq!(
            many.plan(&too_many).unwrap_err(),
            PlanError::TooMany(480 + MAX_LIGHTS)
        );
    }

    #[test]
    fn an_engine_built_from_a_plan_switches_to_the_games_preset_and_back() {
        use crate::zones::Preset;
        let plan = Settings::default().plan(&[]).unwrap();
        assert!(plan.game_edges);
        let mut engine = Engine::new(plan.engine, plan.lights).unwrap();
        assert_eq!(engine.wanted_preset(), Preset::Plain);
        engine.set_games(true);
        assert_eq!(engine.wanted_preset(), Preset::Games);
        engine.set_games(false);
        assert_eq!(engine.wanted_preset(), Preset::Plain);
    }

    #[test]
    fn one_section_on_an_edge_sits_in_its_middle() {
        let lights = strip_lights(Edges {
            top: 1,
            right: 0,
            bottom: 0,
            left: 0,
        });
        assert_eq!(lights.len(), 1);
        assert!((lights[0].x - 0.5).abs() < 1e-6 && lights[0].y < 0.0);
    }
}
