//! What the settings screen changes: the stream's resolution and rate as one of four tiers, HDR, the bitrate cap, the controller's
//! dead zone and whether the status line stays on. The app keeps them in its own small file (`prefs.json` in its data directory) and
//! sends them to the core with `settings.set` at every start, so a value the viewer chose survives a restart whatever state the core's
//! own settings file is in; the core's names and ranges (`native/opennow-core/src/settings.rs`) are the only ones used.
//!
//! The ladders are short on purpose: a remote steps through them with Left and Right, and every rung is one that the TV path was
//! measured at (`docs/research/gfn-444-vrr-result.md`: 4K120 HEVC Main10 4:2:0 at a fixed 120 Hz is the ceiling; the 80 Mbit/s 4K120 clip is
//! the gate; 100 Mbit/s is the feeder's top test clip).
use serde_json::{Value, json};
use std::io;
use std::path::Path;

/// A resolution and frame rate, in the order the settings row steps through them. The ceiling first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Uhd120,
    Qhd120,
    Uhd60,
    Fhd120,
}

impl Tier {
    pub const ALL: [Tier; 4] = [Tier::Uhd120, Tier::Qhd120, Tier::Uhd60, Tier::Fhd120];

    pub fn label(self) -> &'static str {
        match self {
            Tier::Uhd120 => "4K 120",
            Tier::Qhd120 => "1440p 120",
            Tier::Uhd60 => "4K 60",
            Tier::Fhd120 => "1080p 120",
        }
    }

    /// The core's `resolution` value.
    pub fn resolution(self) -> &'static str {
        match self {
            Tier::Uhd120 | Tier::Uhd60 => "3840x2160",
            Tier::Qhd120 => "2560x1440",
            Tier::Fhd120 => "1920x1080",
        }
    }

    pub fn fps(self) -> u32 {
        match self {
            Tier::Uhd60 => 60,
            _ => 120,
        }
    }

    /// The resolution's own name, without the rate.
    pub fn res_label(self) -> &'static str {
        match self {
            Tier::Uhd120 | Tier::Uhd60 => "4K",
            Tier::Qhd120 => "1440p",
            Tier::Fhd120 => "1080p",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Tier::Uhd120 => "uhd120",
            Tier::Qhd120 => "qhd120",
            Tier::Uhd60 => "uhd60",
            Tier::Fhd120 => "fhd120",
        }
    }

    fn from_id(id: &str) -> Option<Tier> {
        Tier::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// The bitrate cap's rungs in Mbit/s.
pub const BITRATES: [u32; 6] = [20, 35, 50, 65, 80, 100];
/// The controller dead zone's rungs in percent of stick travel.
pub const DEADZONES: [u8; 9] = [0, 2, 4, 6, 8, 10, 12, 15, 20];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefs {
    pub tier: Tier,
    pub hdr: bool,
    pub bitrate_mbps: u32,
    pub deadzone: u8,
    pub show_stats: bool,
}

impl Default for Prefs {
    /// The first start: the TV path's ceiling, at the bitrate its gate was measured with, and the pad's own dead zone.
    fn default() -> Self {
        Self { tier: Tier::Uhd120, hdr: true, bitrate_mbps: 80, deadzone: 8, show_stats: false }
    }
}

/// The rung of `ladder` nearest to `value`.
fn snap<T: Copy + Into<i64>>(ladder: &[T], value: i64) -> T {
    *ladder.iter().min_by_key(|r| (Into::<i64>::into(**r) - value).abs()).expect("a ladder has rungs")
}

impl Prefs {
    /// The core's `settings.set` parameters: the tier, HDR with the colour depth it needs, the bitrate cap, both sticks' dead zone,
    /// and what only this platform can say (the codec is left to the core's resolver, the video backend is the feeder's).
    pub fn settings(&self) -> Value {
        json!({
            "resolution": self.tier.resolution(),
            "fps": self.tier.fps(),
            "codec": "auto",
            "colorQuality": if self.hdr { "10bit_420" } else { "8bit_420" },
            "enableHdr": self.hdr,
            "maxBitrateMbps": self.bitrate_mbps,
            "controllerLeftStickDeadzone": self.deadzone,
            "controllerRightStickDeadzone": self.deadzone,
        })
    }

    /// What a game's card says the stream will be: "4K · 120 fps · HDR10".
    pub fn summary(&self) -> String {
        let mut s = format!("{} · {} fps", self.tier.res_label(), self.tier.fps());
        if self.hdr {
            s.push_str(" · HDR10");
        }
        s
    }

    pub fn to_json(&self) -> Value {
        json!({"tier": self.tier.id(), "hdr": self.hdr, "bitrateMbps": self.bitrate_mbps, "deadzone": self.deadzone, "showStats": self.show_stats})
    }

    /// Whatever the file holds, read leniently: a missing or foreign field is the default's, a number off the ladder is the nearest rung.
    pub fn from_json(v: &Value) -> Prefs {
        let d = Prefs::default();
        Prefs {
            tier: v["tier"].as_str().and_then(Tier::from_id).unwrap_or(d.tier),
            hdr: v["hdr"].as_bool().unwrap_or(d.hdr),
            bitrate_mbps: v["bitrateMbps"].as_i64().map(|n| snap(&BITRATES, n)).unwrap_or(d.bitrate_mbps),
            deadzone: v["deadzone"].as_i64().map(|n| snap(&DEADZONES, n)).unwrap_or(d.deadzone),
            show_stats: v["showStats"].as_bool().unwrap_or(d.show_stats),
        }
    }

    pub fn load(path: &Path) -> Prefs {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).map(|v| Prefs::from_json(&v)).unwrap_or_default()
    }

    /// Written whole beside the file and renamed over it, so a power cut leaves the old file or the new one.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.to_json()).map_err(io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }
}

/// The rows of the settings screen, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Stream,
    Hdr,
    Bitrate,
    Deadzone,
    Stats,
}

impl Row {
    pub const ALL: [Row; 5] = [Row::Stream, Row::Hdr, Row::Bitrate, Row::Deadzone, Row::Stats];

    pub fn name(self) -> &'static str {
        match self {
            Row::Stream => "Stream",
            Row::Hdr => "HDR",
            Row::Bitrate => "Bitrate cap",
            Row::Deadzone => "Stick dead zone",
            Row::Stats => "Show stats",
        }
    }

    /// The help mark's words: what the row does and what it costs, in two sentences at most.
    pub fn help(self) -> &'static str {
        match self {
            Row::Stream => "Resolution and frames a second. 4K 120 is the most this TV takes and needs a fast connection; 1080p 120 is the lightest at 120.",
            Row::Hdr => "Brighter highlights and wider colour on a game that has HDR. Off sends an ordinary picture and uses less data.",
            Row::Bitrate => "The most data a second the stream may use. A lower cap keeps a slow connection smooth and costs some sharpness.",
            Row::Deadzone => "How far a stick moves before the game feels it. Raise it if the picture drifts with the pad at rest.",
            Row::Stats => "Keeps the frame rate and bitrate on the screen while you play, not only after a key press.",
        }
    }

    /// The value as the row shows it.
    pub fn value(self, p: &Prefs) -> String {
        match self {
            Row::Stream => p.tier.label().to_owned(),
            Row::Hdr => on_off(p.hdr),
            Row::Bitrate => format!("{} Mbit/s", p.bitrate_mbps),
            Row::Deadzone => format!("{}%", p.deadzone),
            Row::Stats => on_off(p.show_stats),
        }
    }

    /// A row's value one rung up (`step` 1) or down (-1), or round to the other end when `wrap`. Booleans flip on either.
    pub fn step(self, p: &mut Prefs, step: i32, wrap: bool) -> bool {
        fn walk<T: Copy + PartialEq>(ladder: &[T], at: &mut T, step: i32, wrap: bool) -> bool {
            let i = ladder.iter().position(|r| r == at).unwrap_or(0) as i32;
            let n = ladder.len() as i32;
            let j = if wrap { (i + step).rem_euclid(n) } else { (i + step).clamp(0, n - 1) };
            *at = ladder[j as usize];
            j != i
        }
        match self {
            Row::Stream => walk(&Tier::ALL, &mut p.tier, step, wrap),
            Row::Hdr => {
                p.hdr = !p.hdr;
                true
            }
            Row::Bitrate => walk(&BITRATES, &mut p.bitrate_mbps, step, wrap),
            Row::Deadzone => walk(&DEADZONES, &mut p.deadzone, step, wrap),
            Row::Stats => {
                p.show_stats = !p.show_stats;
                true
            }
        }
    }

    /// The core's setting keys the row decides (the stats line is the UI's own: none). The core takes one key and value a request.
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            Row::Stream => &["resolution", "fps"],
            Row::Hdr => &["enableHdr", "colorQuality"],
            Row::Bitrate => &["maxBitrateMbps"],
            Row::Deadzone => &["controllerLeftStickDeadzone", "controllerRightStickDeadzone"],
            Row::Stats => &[],
        }
    }
}

fn on_off(v: bool) -> String {
    if v { "On" } else { "Off" }.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_tv_ceiling_and_the_settings_use_the_cores_own_names() {
        let s = Prefs::default().settings();
        assert_eq!((s["resolution"].as_str(), s["fps"].as_u64()), (Some("3840x2160"), Some(120)));
        assert_eq!((s["enableHdr"].as_bool(), s["colorQuality"].as_str()), (Some(true), Some("10bit_420")));
        assert_eq!((s["maxBitrateMbps"].as_u64(), s["controllerLeftStickDeadzone"].as_u64(), s["controllerRightStickDeadzone"].as_u64()), (Some(80), Some(8), Some(8)));
        assert_eq!(s["codec"].as_str(), Some("auto"));
    }

    #[test]
    fn each_tier_is_one_resolution_and_one_rate_and_hdr_off_drops_to_eight_bit() {
        let want = [("4K 120", "3840x2160", 120), ("1440p 120", "2560x1440", 120), ("4K 60", "3840x2160", 60), ("1080p 120", "1920x1080", 120)];
        for (t, (label, res, fps)) in Tier::ALL.into_iter().zip(want) {
            let p = Prefs { tier: t, ..Prefs::default() };
            assert_eq!((t.label(), p.settings()["resolution"].as_str().unwrap(), p.settings()["fps"].as_u64().unwrap()), (label, res, fps as u64));
        }
        let p = Prefs { hdr: false, ..Prefs::default() };
        assert_eq!((p.settings()["enableHdr"].as_bool(), p.settings()["colorQuality"].as_str()), (Some(false), Some("8bit_420")));
    }

    #[test]
    fn a_row_steps_along_its_ladder_clamps_at_the_ends_and_wraps_on_ok() {
        let mut p = Prefs::default();
        assert!(Row::Stream.step(&mut p, 1, false));
        assert_eq!(p.tier, Tier::Qhd120);
        assert!(Row::Stream.step(&mut p, -1, false));
        assert_eq!(p.tier, Tier::Uhd120);
        assert!(!Row::Stream.step(&mut p, -1, false), "no step past the first rung");
        assert!(Row::Stream.step(&mut p, -1, true), "OK wraps to the last rung");
        assert_eq!(p.tier, Tier::Fhd120);
        p.bitrate_mbps = 100;
        assert!(!Row::Bitrate.step(&mut p, 1, false));
        assert!(Row::Bitrate.step(&mut p, 1, true));
        assert_eq!(p.bitrate_mbps, 20);
        assert!(Row::Hdr.step(&mut p, 1, false) && !p.hdr);
        assert!(Row::Stats.step(&mut p, -1, false) && p.show_stats);
        assert!(Row::Deadzone.step(&mut p, 1, false));
        assert_eq!(p.deadzone, 10);
    }

    #[test]
    fn the_file_round_trips_and_a_damaged_one_is_read_leniently() {
        let p = Prefs { tier: Tier::Uhd60, hdr: false, bitrate_mbps: 50, deadzone: 12, show_stats: true };
        assert_eq!(Prefs::from_json(&p.to_json()), p);
        assert_eq!(p.to_json()["deadzone"], 12, "the engine reads this key of the file when a stream starts (gfn/engine, pads.rs)");
        let odd = Prefs::from_json(&json!({"tier": "8k", "hdr": "yes", "bitrateMbps": 77, "deadzone": 99, "showStats": 1}));
        assert_eq!(odd, Prefs { bitrate_mbps: 80, deadzone: 20, ..Prefs::default() });
        assert_eq!(Prefs::from_json(&json!(null)), Prefs::default());
        let dir = std::env::temp_dir().join(format!("gfn-prefs-{}", std::process::id()));
        let path = dir.join("prefs.json");
        assert_eq!(Prefs::load(&path), Prefs::default(), "no file is the defaults");
        p.save(&path).unwrap();
        assert_eq!(Prefs::load(&path), p);
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(Prefs::load(&path), Prefs::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_row_has_a_name_a_value_and_help_and_only_the_stats_line_stays_in_the_ui() {
        let p = Prefs::default();
        let values: Vec<String> = Row::ALL.iter().map(|r| r.value(&p)).collect();
        assert_eq!(values, ["4K 120", "On", "80 Mbit/s", "8%", "Off"]);
        for r in Row::ALL {
            assert!(!r.name().is_empty() && r.help().len() > 30 && r.help().len() < 160, "{:?}", r);
            assert_eq!(r.keys().is_empty(), r == Row::Stats);
        }
    }
}
