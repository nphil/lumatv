//! Black-bar handling. HyperHDR crops bars in software before averaging; here the video hardware offers presets that
//! skip them, so the job is to notice bars and ask for the right preset.
//!
//! What is visible: area 14 is a strip along the top of the picture, area 15 one down the left edge, area 1 covers
//! the lower fifth. They are in the same place in presets 0 to 2, so the strips stay readable after a switch and show
//! the bars going away. A strip counts as black when almost all its pixels are at or under the black code.
//!
//! - Letterbox: top strip black, and while in the plain preset the bottom area at least half black (a 2.39:1 film on
//!   a 16:9 set leaves bars of 12.8 % top and bottom, so area 1, rows 0.8..1.0, is 64 % bar). Inside preset 1 the
//!   bottom is no longer visible, so the top strip alone keeps the state.
//! - Pillarbox: left strip black.
//! - The picture itself must exist: a black frame (fade, scene change) decides nothing and holds the last state.
//! - A state is entered after `enter_ms` of consistent evidence and left after `leave_ms` of its absence, so a dark
//!   night sky under a bright foreground does not flip the preset.
//!
//! Games mode is not detected here; it is a setting (the helper knows the picture mode or the input).

use crate::zones::{BOTTOM_STRIP, CENTER, LEFT_STRIP, Preset, TOP_STRIP, ZoneFrame};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BarConfig {
    pub enabled: bool,
    /// Black fraction a strip needs to count as bar.
    pub strip_black: f32,
    /// Black fraction of the bottom area (rows 0.8..1.0) that confirms a letterbox in the plain preset.
    pub bottom_black: f32,
    /// Largest channel of the centre area under which the frame is treated as having no picture.
    pub picture_min: f32,
    pub enter_ms: u32,
    pub leave_ms: u32,
}

impl Default for BarConfig {
    fn default() -> Self {
        BarConfig {
            enabled: true,
            strip_black: 0.92,
            bottom_black: 0.5,
            picture_min: 0.06,
            enter_ms: 1500,
            leave_ms: 800,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bars {
    #[default]
    None,
    Letterbox,
    Pillarbox,
}

#[derive(Clone, Debug)]
pub struct BarDetector {
    cfg: BarConfig,
    state: Bars,
    candidate: Bars,
    since_ms: u64,
}

impl BarDetector {
    pub fn new(cfg: BarConfig) -> BarDetector {
        BarDetector {
            cfg,
            state: Bars::None,
            candidate: Bars::None,
            since_ms: 0,
        }
    }

    pub fn state(&self) -> Bars {
        self.state
    }

    /// The preset to capture with for the bars seen so far (games mode is the caller's choice).
    pub fn preset(&self) -> Preset {
        match self.state {
            Bars::None => Preset::Plain,
            Bars::Letterbox => Preset::TopBottom,
            Bars::Pillarbox => Preset::SideBars,
        }
    }

    pub fn reset(&mut self) {
        self.state = Bars::None;
        self.candidate = Bars::None;
    }

    /// Feed every captured frame; frames from the games preset carry no strips and are ignored.
    pub fn update(&mut self, now_ms: u64, f: &ZoneFrame) {
        if !self.cfg.enabled || f.preset == Preset::Games {
            return;
        }
        let has_picture =
            f.color[CENTER].max_channel() >= self.cfg.picture_min && f.count[CENTER] > 0;
        if !has_picture {
            self.candidate = self.state;
            return;
        }
        let strip = |i: usize| f.count[i] > 0 && f.black[i] >= self.cfg.strip_black;
        let top = strip(TOP_STRIP);
        let left = strip(LEFT_STRIP);
        let bottom_ok = match f.preset {
            Preset::Plain => {
                f.count[BOTTOM_STRIP] > 0 && f.black[BOTTOM_STRIP] >= self.cfg.bottom_black
            }
            _ => true,
        };
        let seen = if top && bottom_ok {
            Bars::Letterbox
        } else if left {
            Bars::Pillarbox
        } else {
            Bars::None
        };
        // Evidence for the state we are in is evidence; the hardware preset only changes what we can see of it.
        if seen == self.state {
            self.candidate = seen;
            return;
        }
        if seen != self.candidate {
            self.candidate = seen;
            self.since_ms = now_ms;
        }
        let need = if seen == Bars::None {
            self.cfg.leave_ms
        } else {
            self.cfg.enter_ms
        };
        if now_ms.saturating_sub(self.since_ms) >= u64::from(need) {
            self.state = seen;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rgb::Rgb;

    /// A frame with a bright picture and the given strips black.
    fn frame(preset: Preset, top_black: bool, bottom_black: bool, left_black: bool) -> ZoneFrame {
        let mut f = ZoneFrame::blank(preset);
        for i in 0..preset.zone_count() {
            f.color[i] = Rgb::new(0.5, 0.5, 0.5);
            f.count[i] = 1000;
        }
        if top_black {
            f.black[TOP_STRIP] = 0.99;
            f.color[TOP_STRIP] = Rgb::BLACK;
        }
        if bottom_black {
            f.black[BOTTOM_STRIP] = 0.64;
        }
        if left_black {
            f.black[LEFT_STRIP] = 0.99;
        }
        f
    }

    fn run(d: &mut BarDetector, from_ms: u64, to_ms: u64, f: &ZoneFrame) {
        let mut t = from_ms;
        while t <= to_ms {
            d.update(t, f);
            t += 33;
        }
    }

    #[test]
    fn letterbox_is_entered_after_the_hold_and_asks_for_preset_one() {
        let mut d = BarDetector::new(BarConfig::default());
        let f = frame(Preset::Plain, true, true, false);
        run(&mut d, 0, 1400, &f);
        assert_eq!(d.state(), Bars::None, "not yet");
        run(&mut d, 1433, 1600, &f);
        assert_eq!(d.state(), Bars::Letterbox);
        assert_eq!(d.preset(), Preset::TopBottom);
    }

    #[test]
    fn top_strip_alone_in_the_plain_preset_is_not_a_letterbox() {
        // A night sky: black top, picture below.
        let mut d = BarDetector::new(BarConfig::default());
        run(&mut d, 0, 10_000, &frame(Preset::Plain, true, false, false));
        assert_eq!(d.state(), Bars::None);
    }

    #[test]
    fn inside_preset_one_the_top_strip_keeps_the_state_and_its_end_releases_it() {
        let mut d = BarDetector::new(BarConfig::default());
        run(&mut d, 0, 2000, &frame(Preset::Plain, true, true, false));
        assert_eq!(d.state(), Bars::Letterbox);
        // Capturing with preset 1 now: the bottom area no longer shows bars, the top strip still does.
        run(
            &mut d,
            2033,
            8000,
            &frame(Preset::TopBottom, true, false, false),
        );
        assert_eq!(d.state(), Bars::Letterbox);
        // The film ends: top strip carries picture again; leaves after leave_ms.
        run(
            &mut d,
            8033,
            8700,
            &frame(Preset::TopBottom, false, false, false),
        );
        assert_eq!(d.state(), Bars::Letterbox, "held for leave_ms");
        run(
            &mut d,
            8733,
            9000,
            &frame(Preset::TopBottom, false, false, false),
        );
        assert_eq!(d.state(), Bars::None);
        assert_eq!(d.preset(), Preset::Plain);
    }

    #[test]
    fn pillarbox_uses_the_left_strip_and_preset_two() {
        let mut d = BarDetector::new(BarConfig::default());
        run(&mut d, 0, 2000, &frame(Preset::Plain, false, false, true));
        assert_eq!(d.state(), Bars::Pillarbox);
        assert_eq!(d.preset(), Preset::SideBars);
        run(
            &mut d,
            2033,
            4000,
            &frame(Preset::SideBars, false, false, true),
        );
        assert_eq!(d.state(), Bars::Pillarbox);
    }

    #[test]
    fn a_black_frame_decides_nothing() {
        let mut d = BarDetector::new(BarConfig::default());
        run(&mut d, 0, 2000, &frame(Preset::Plain, true, true, false));
        assert_eq!(d.state(), Bars::Letterbox);
        let mut black = ZoneFrame::blank(Preset::TopBottom);
        for i in 0..16 {
            black.black[i] = 1.0;
            black.count[i] = 1000;
        }
        run(&mut d, 2033, 20_000, &black);
        assert_eq!(
            d.state(),
            Bars::Letterbox,
            "a long fade to black keeps the state"
        );
    }

    #[test]
    fn a_short_blip_does_not_switch() {
        let mut d = BarDetector::new(BarConfig::default());
        run(&mut d, 0, 800, &frame(Preset::Plain, true, true, false));
        run(
            &mut d,
            833,
            3000,
            &frame(Preset::Plain, false, false, false),
        );
        assert_eq!(d.state(), Bars::None);
    }

    #[test]
    fn disabled_and_games_frames_never_change_state() {
        let mut d = BarDetector::new(BarConfig {
            enabled: false,
            ..BarConfig::default()
        });
        run(&mut d, 0, 5000, &frame(Preset::Plain, true, true, false));
        assert_eq!(d.state(), Bars::None);
        let mut d = BarDetector::new(BarConfig::default());
        run(&mut d, 0, 5000, &frame(Preset::Games, true, true, false));
        assert_eq!(d.state(), Bars::None);
    }
}
