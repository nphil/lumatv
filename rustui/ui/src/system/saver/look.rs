//! The numbers of the saver: how bright anything may be, how it dims over time, where the block sits and how it drifts.
//!
//! The saver is a burn-in rule before it is a picture (`docs/research/oledcare-result.md`, `docs/mocks/BATCH3.md`): the screen is black but for
//! what is shown, everything shown moves, and nothing is bright. The rule is a number here and a test over the rendered frame: **no pixel of
//! the saver is brighter than [`PEAK`] of full white** (97 of 255), whatever the theme says, and the average level of the picture stays a
//! fraction of a percent. A theme can only go lower.
//!
//! Contrast is traded on purpose. Lucent asks 4.5:1 for text and 7:1 for the TV's distance reading; on true black the saver's primary ink is
//! 3.4:1 at its first minute and about 1.4:1 once it has dimmed (against a black that emits nothing: the ratio's 0.05 flare floor does not
//! exist on an OLED, and the marks are read from the dark). Lucent's own rule for the screensaver is "dim clock and urgent state only" (section 9,
//! night rules); the burn-in cap is the limit, not a floor, and waking the TV brings the normal UI back.

use serde_json::Value;

/// The hard ceiling of every pixel of the saver, as a fraction of full white (0.38 of 255 is 97).
pub const PEAK: f32 = 0.38;
/// The inks, as fractions of the cap: the hands, the date, the temperature and the titles of notices; the condition, the values and the ages;
/// the ring, the ticks and the minor marks.
pub const INK1: f32 = 1.0;
pub const INK2: f32 = 0.78;
pub const INK3: f32 = 0.52;
/// The brightest a glow gets, as a fraction of the cap (alpha): a halo is a hint of light, not a second stroke.
pub const GLOW: f32 = 0.10;
/// The saver never dims below this fraction of its first brightness, whatever `dimTo` says: a clock that cannot be seen is a black screen.
pub const FLOOR: f32 = 0.25;

/// The block's reach: whole pixels in small steps (a pixel or two every few seconds), in two slow periods that do not meet (the art mode's numbers).
pub const AMP_X: f32 = 30.0;
pub const AMP_Y: f32 = 20.0;
pub const PERIOD_X_S: f64 = 540.0;
pub const PERIOD_Y_S: f64 = 380.0;

/// The TV's safe area at 1920 by 1080 (`Screensaver.qml`).
pub const SAFE_X: f32 = 96.0;
pub const SAFE_Y: f32 = 54.0;
/// A new spot is at least this far from the old one.
pub const MIN_MOVE: f32 = 420.0;

/// What the owner chose (`theme.screensaver`), read the way the QML does, with the shipped theme's numbers for what is missing.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// `style: off`.
    pub off: bool,
    pub dim: bool,
    /// The level the saver dims to, of its first.
    pub dim_to: f32,
    pub dim_over_ms: u64,
    /// The brightest any pixel gets: `min(PEAK, clockLevel)`.
    pub cap: f32,
    pub move_every_ms: u64,
    pub fade_ms: u32,
    pub shift_every_ms: u64,
    /// After this long up the TV is put in standby; 0 never.
    pub standby_ms: u64,
    /// The weather, the saver's items and the notices around the clock.
    pub glance: bool,
    pub reduced: bool,
}

fn num(v: Option<&Value>, default: f64) -> f64 {
    v.and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(default)
}

impl Config {
    /// `theme` is the merged theme; `reduced` the person's setting; `press_ms` the short fade a reduced setting gets instead of the long one.
    pub fn from_theme(theme: &Value, reduced: bool, press_ms: u32) -> Config {
        let s = theme.get("screensaver").unwrap_or(&Value::Null);
        let flag = |k: &str| s.get(k).and_then(Value::as_bool) != Some(false);
        let level = num(s.get("clockLevel"), 0.4).clamp(0.05, 1.0) as f32;
        Config {
            off: s.get("style").and_then(Value::as_str) == Some("off"),
            dim: flag("dim"),
            dim_to: (num(s.get("dimTo"), 0.4) as f32).clamp(FLOOR, 1.0),
            dim_over_ms: (num(s.get("dimOverMin"), 20.0).clamp(1.0, 600.0) * 60_000.0) as u64,
            cap: level.min(PEAK),
            move_every_ms: (num(s.get("moveEverySec"), 60.0).clamp(10.0, 3600.0) * 1000.0) as u64,
            fade_ms: if reduced { press_ms } else { num(s.get("fadeMs"), 2000.0).clamp(0.0, 10_000.0) as u32 },
            shift_every_ms: (num(s.get("shiftEverySec"), 4.0).clamp(1.0, 60.0) * 1000.0) as u64,
            standby_ms: (num(s.get("standbyHours"), 4.0).clamp(0.0, 48.0) * 3_600_000.0) as u64,
            glance: flag("glance"),
            reduced,
        }
    }
}

/// How bright the saver is `up_ms` after it came up, of its first: down to `dim_to` over `dim_over_ms`, in steps of a half percent (the
/// compositor is told when this changes, not every frame).
pub fn level(cfg: &Config, up_ms: u64) -> f32 {
    if !cfg.dim {
        return 1.0;
    }
    let done = (up_ms as f64 / cfg.dim_over_ms as f64).min(1.0) as f32;
    let raw = 1.0 - (1.0 - cfg.dim_to) * done;
    (raw * 200.0).round() / 200.0
}

/// The block's shift `up_ms` after the saver came up: whole pixels that change every `shiftEverySec`, so nothing is redrawn between steps.
pub fn drift(cfg: &Config, up_ms: u64) -> (i32, i32) {
    if cfg.reduced {
        return (0, 0);
    }
    let step = cfg.shift_every_ms.max(1);
    let t = (up_ms / step * step) as f64 / 1000.0;
    let w = std::f64::consts::TAU;
    ((f64::from(AMP_X) * (w * t / PERIOD_X_S).sin()).round() as i32, (f64::from(AMP_Y) * (w * t / PERIOD_Y_S).sin()).round() as i32)
}

/// A small generator of the block's next spot (xorshift64*): the spot is not a secret, only never the same twice in a row.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    /// 0 up to 1, not including 1.
    pub fn unit(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((v >> 40) as f32) / (1u64 << 24) as f32
    }
}

/// The room the block may move in at `screen` for a block of `block`: inside the safe area.
pub fn room(screen: (f32, f32), block: (f32, f32)) -> (f32, f32) {
    let k = (screen.0 / 1920.0).min(screen.1 / 1080.0).max(0.1);
    ((screen.0 - 2.0 * SAFE_X * k - block.0).max(0.0), (screen.1 - 2.0 * SAFE_Y * k - block.1).max(0.0))
}

/// Another spot, as fractions of the room, far enough from `from` to be a different place; twelve tries, then the other side.
pub fn pick(rng: &mut Rng, from: (f32, f32), room: (f32, f32)) -> (f32, f32) {
    for _ in 0..12 {
        let (nx, ny) = (rng.unit(), rng.unit());
        let (dx, dy) = ((nx - from.0) * room.0, (ny - from.1) * room.1);
        if (dx * dx + dy * dy).sqrt() > MIN_MOVE {
            return (nx, ny);
        }
    }
    (if from.0 < 0.5 { 0.85 } else { 0.15 }, from.1)
}

/// The block's top-left corner for the spot `at` (fractions of the room), in whole pixels.
pub fn place(screen: (f32, f32), block: (f32, f32), at: (f32, f32)) -> (i32, i32) {
    let k = (screen.0 / 1920.0).min(screen.1 / 1080.0).max(0.1);
    let r = room(screen, block);
    ((SAFE_X * k + at.0 * r.0).round() as i32, (SAFE_Y * k + at.1 * r.1).round() as i32)
}

/// `value` of an ink at `strength` of the cap and `level` of the dim, as the 0 to 255 a channel carries: never more than the cap.
pub fn channel(cap: f32, strength: f32, ink: u8) -> u8 {
    let top = (cap * 255.0).round();
    ((f32::from(ink) / 255.0 * top * strength.clamp(0.0, 1.0)).round()).min(top) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(v: Value) -> Config {
        Config::from_theme(&json!({"screensaver": v}), false, 90)
    }

    #[test]
    fn a_theme_that_says_nothing_gets_the_shipped_numbers_and_the_cap_can_only_go_down() {
        let c = Config::from_theme(&json!({}), false, 90);
        assert_eq!((c.dim_to, c.dim_over_ms, c.cap), (0.4, 20 * 60_000, 0.38));
        assert_eq!((c.move_every_ms, c.fade_ms, c.shift_every_ms, c.standby_ms), (60_000, 2000, 4000, 4 * 3_600_000));
        assert!(c.dim && c.glance && !c.off);
        assert_eq!(cfg(json!({"clockLevel": 0.9})).cap, PEAK, "a bright theme is held to the cap");
        assert_eq!(cfg(json!({"clockLevel": 0.2})).cap, 0.2, "a dim one is believed");
        assert_eq!(cfg(json!({"clockLevel": 0.0})).cap, 0.05, "and never to nothing");
        assert_eq!(cfg(json!({"dimTo": 0.01})).dim_to, FLOOR, "a clock that cannot be seen is a black screen");
        assert!(cfg(json!({"style": "off"})).off);
        assert_eq!(cfg(json!({"clockLevel": "bright", "dimOverMin": null})).cap, 0.38, "a value of the wrong kind is not taken");
        let r = Config::from_theme(&json!({"screensaver": {"fadeMs": 2000}}), true, 90);
        assert_eq!(r.fade_ms, 90, "reduced motion fades in the short time");
    }

    #[test]
    fn the_level_runs_down_to_dim_to_over_the_time_and_stays() {
        let c = cfg(json!({"dimTo": 0.4, "dimOverMin": 20}));
        assert_eq!(level(&c, 0), 1.0);
        assert!((level(&c, 10 * 60_000) - 0.7).abs() < 0.006);
        assert!((level(&c, 20 * 60_000) - 0.4).abs() < 0.006);
        assert!((level(&c, 5 * 3_600_000) - 0.4).abs() < 0.006, "and no lower");
        assert_eq!(level(&cfg(json!({"dim": false})), 3_600_000), 1.0);
        let mut last = 1.0;
        for m in 0..=25 {
            let l = level(&c, m * 60_000);
            assert!(l <= last, "it only goes down");
            last = l;
        }
    }

    #[test]
    fn the_drift_is_whole_pixels_one_at_a_time_within_its_reach_and_changes_only_at_a_step() {
        let c = cfg(json!({"shiftEverySec": 4}));
        assert_eq!(drift(&c, 0), (0, 0));
        let mut prev = drift(&c, 0);
        let mut changes = 0;
        for s in 0..2000u64 {
            let d = drift(&c, s * 1000);
            assert!(d.0.abs() <= AMP_X as i32 && d.1.abs() <= AMP_Y as i32);
            assert!((d.0 - prev.0).abs() <= 2 && (d.1 - prev.1).abs() <= 2, "a step is a pixel or two at most: {prev:?} -> {d:?}");
            if d != prev {
                changes += 1;
            }
            prev = d;
        }
        assert!(changes > 100, "it moves");
        assert_eq!(drift(&c, 3999), drift(&c, 0), "nothing is redrawn between steps");
        let seen: std::collections::HashSet<(i32, i32)> = (0..3000u64).map(|s| drift(&c, s * 1000)).collect();
        assert!(seen.len() > 300, "the two slow periods wander, they do not retrace one line");
        assert_eq!(drift(&Config::from_theme(&json!({}), true, 90), 123_456), (0, 0), "reduced motion holds still");
    }

    #[test]
    fn a_new_spot_is_far_from_the_old_one_and_inside_the_safe_area() {
        let screen = (1920.0, 1080.0);
        let block = (900.0, 540.0);
        let r = room(screen, block);
        assert_eq!(r, (1920.0 - 192.0 - 900.0, 1080.0 - 108.0 - 540.0));
        let mut rng = Rng::new(7);
        let mut at = (0.5, 0.5);
        for _ in 0..200 {
            let next = pick(&mut rng, at, r);
            let (dx, dy) = ((next.0 - at.0) * r.0, (next.1 - at.1) * r.1);
            assert!((dx * dx + dy * dy).sqrt() > MIN_MOVE || next.1 == at.1, "far enough, or the other side: {at:?} -> {next:?}");
            assert!((0.0..=1.0).contains(&next.0) && (0.0..=1.0).contains(&next.1));
            let (x, y) = place(screen, block, next);
            assert!(x >= 96 && y >= 54 && x + 900 <= 1920 - 96 && y + 540 <= 1080 - 54, "{x},{y} is inside the safe area");
            at = next;
        }
        assert_eq!(pick(&mut Rng::new(1), (0.1, 0.5), (10.0, 10.0)), (0.85, 0.5), "a room too small for a move goes to the other side");
        assert_eq!(place(screen, block, (0.0, 0.0)), (96, 54));
        assert_eq!(place((3840.0, 2160.0), (1800.0, 1080.0), (0.0, 0.0)), (192, 108), "the safe area scales with the screen");
    }

    #[test]
    fn a_channel_never_exceeds_the_cap() {
        for ink in [0u8, 90, 200, 255] {
            for s in [0.0, 0.5, 1.0, 2.0] {
                assert!(channel(0.38, s, ink) <= 97, "{ink} at {s}");
            }
        }
        assert_eq!(channel(0.38, 1.0, 255), 97);
        assert_eq!(channel(0.38, INK3, 255), 50);
        assert_eq!(channel(0.2, 1.0, 255), 51);
    }

    #[test]
    fn the_random_numbers_are_in_range_and_not_one_value() {
        let mut r = Rng::new(0);
        let v: Vec<f32> = (0..1000).map(|_| r.unit()).collect();
        assert!(v.iter().all(|x| (0.0..1.0).contains(x)));
        let mean = v.iter().sum::<f32>() / 1000.0;
        assert!((0.4..0.6).contains(&mean), "{mean}");
    }
}
