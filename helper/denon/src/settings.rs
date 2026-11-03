//! Candidate commands from Denon's 2015 serial protocol. Support must be established by the caller.

macro_rules! values {
    ($name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub fn as_str(self) -> &'static str { match self { $(Self::$variant => $wire),+ } }
            pub fn parse(value: &str) -> Option<Self> { match value { $($wire => Some(Self::$variant),)+ _ => None } }
        }
    };
}

values!(Source { Phono => "PHONO", Cd => "CD", Tuner => "TUNER", Dvd => "DVD", Bluray => "BD", Tv => "TV", SatCbl => "SAT/CBL", MediaPlayer => "MPLAY", Game => "GAME", Aux1 => "AUX1", Aux2 => "AUX2", Network => "NET", Usb => "USB", Bluetooth => "BT", OnlineMusic => "ONLINE MUSIC", InternetRadio => "IRADIO", Server => "SERVER", Favorites => "FAVORITES" });
values!(SoundMode { Stereo => "STEREO", Direct => "DIRECT", PureDirect => "PURE DIRECT", Auto => "AUTO", DolbySurround => "DOLBY SURROUND", DtsNeuralX => "DTS NEURAL:X", MultiChStereo => "MULTI CH STEREO", RockArena => "ROCK ARENA", JazzClub => "JAZZ CLUB", MonoMovie => "MONO MOVIE", VideoGame => "VIDEO GAME", Matrix => "MATRIX", Virtual => "VIRTUAL" });
values!(Eco { On => "ON", Auto => "AUTO", Off => "OFF" });
values!(MultEq { Audyssey => "AUDYSSEY", BypassLr => "BYP.LR", Flat => "FLAT", Off => "OFF" });
values!(ReferenceLevel { Zero => "0", Five => "5", Ten => "10", Fifteen => "15" });
values!(DynamicVolume { Off => "OFF", Light => "LIT", Medium => "MED", Heavy => "HEV" });
values!(DialogEnhancer { Off => "OFF", Low => "LOW", Medium => "MED", High => "HIGH" });
values!(QuickSelect { One => "1", Two => "2", Three => "3", Four => "4" });

/// Tone adjustment in whole dB, from -6 through +6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tone(i8);
impl Tone {
    pub fn new(db: i8) -> Option<Self> {
        (-6..=6).contains(&db).then_some(Self(db))
    }
    pub fn db(self) -> i8 {
        self.0
    }
    fn parse(value: &str) -> Option<Self> {
        if value.len() != 2 || !value.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Self::new(value.parse::<i8>().ok()? - 50)
    }
}

/// Zone 2 absolute volume, stored as half steps from 0 to 98.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZoneVolume(u8);
impl ZoneVolume {
    pub fn new(value: f64) -> Option<Self> {
        if !value.is_finite() || !(0.0..=98.0).contains(&value) || (value * 2.0).fract() != 0.0 {
            return None;
        }
        Some(Self((value * 2.0) as u8))
    }
    pub fn value(self) -> f64 {
        f64::from(self.0) / 2.0
    }
    fn parse(value: &str) -> Option<Self> {
        if !value.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let n = value.parse::<u16>().ok()?;
        match value.len() {
            2 => Self::new(f64::from(n)),
            3 if n % 10 == 5 => Self::new(f64::from(n) / 10.0),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    Power,
    Source,
    SoundMode,
    QuickSelect,
    Eco,
    Bass,
    Treble,
    ToneEnabled,
    CinemaEq,
    MultEq,
    DynamicEq,
    ReferenceLevel,
    DynamicVolume,
    Zone2,
    DialogEnhancer,
}
impl Query {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Power => "PW?",
            Self::Source => "SI?",
            Self::SoundMode => "MS?",
            Self::QuickSelect => "MSQUICK ?",
            Self::Eco => "ECO?",
            Self::Bass => "PSBAS ?",
            Self::Treble => "PSTRE ?",
            Self::ToneEnabled => "PSTONE CTRL ?",
            Self::CinemaEq => "PSCINEMA EQ. ?",
            Self::MultEq => "PSMULTEQ: ?",
            Self::DynamicEq => "PSDYNEQ ?",
            Self::ReferenceLevel => "PSREFLEV ?",
            Self::DynamicVolume => "PSDYNVOL ?",
            Self::Zone2 => "Z2?",
            Self::DialogEnhancer => "PSDEH ?",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Power(bool),
    Source(Source),
    SoundMode(SoundMode),
    QuickSelect(QuickSelect),
    Eco(Eco),
    Bass(Tone),
    Treble(Tone),
    ToneEnabled(bool),
    CinemaEq(bool),
    MultEq(MultEq),
    DynamicEq(bool),
    ReferenceLevel(ReferenceLevel),
    DynamicVolume(DynamicVolume),
    Zone2Power(bool),
    Zone2Source(Source),
    Zone2Volume(ZoneVolume),
    DialogEnhancer(DialogEnhancer),
}
impl Setting {
    /// Encodes only enumerated commands; no caller-provided text reaches the wire.
    pub fn command(self) -> String {
        let on = |v| if v { "ON" } else { "OFF" };
        match self {
            Self::Power(v) => {
                if v {
                    "PWON".into()
                } else {
                    "PWSTANDBY".into()
                }
            }
            Self::Source(v) => format!("SI{}", v.as_str()),
            Self::SoundMode(v) => format!("MS{}", v.as_str()),
            Self::QuickSelect(v) => format!("MSQUICK{}", v.as_str()),
            Self::Eco(v) => format!("ECO{}", v.as_str()),
            Self::Bass(v) => format!("PSBAS{:02}", v.db() + 50),
            Self::Treble(v) => format!("PSTRE{:02}", v.db() + 50),
            Self::ToneEnabled(v) => format!("PSTONE CTRL {}", on(v)),
            Self::CinemaEq(v) => format!("PSCINEMA EQ.{}", on(v)),
            Self::MultEq(v) => format!("PSMULTEQ:{}", v.as_str()),
            Self::DynamicEq(v) => format!("PSDYNEQ {}", on(v)),
            Self::ReferenceLevel(v) => format!("PSREFLEV {}", v.as_str()),
            Self::DynamicVolume(v) => format!("PSDYNVOL {}", v.as_str()),
            Self::Zone2Power(v) => format!("Z2{}", on(v)),
            Self::Zone2Source(v) => format!("Z2{}", v.as_str()),
            Self::Zone2Volume(v) => {
                if v.0 % 2 == 0 {
                    format!("Z2{:02}", v.0 / 2)
                } else {
                    format!("Z2{:02}5", v.0 / 2)
                }
            }
            Self::DialogEnhancer(v) => format!("PSDEH {}", v.as_str()),
        }
    }
}

/// Only recognized, complete replies are returned; unknown values are not capability evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Power(bool),
    Source(Source),
    SoundMode(SoundMode),
    QuickSelect(QuickSelect),
    Eco(Eco),
    Bass(Tone),
    Treble(Tone),
    ToneEnabled(bool),
    CinemaEq(bool),
    MultEq(MultEq),
    DynamicEq(bool),
    ReferenceLevel(ReferenceLevel),
    DynamicVolume(DynamicVolume),
    Zone2Power(bool),
    Zone2Source(Source),
    Zone2Volume(ZoneVolume),
    DialogEnhancer(DialogEnhancer),
}
fn boolean(s: &str) -> Option<bool> {
    match s {
        "ON" => Some(true),
        "OFF" => Some(false),
        _ => None,
    }
}
pub fn parse_reply(line: &str) -> Option<Reply> {
    let line = line.trim();
    if let Some(s) = line.strip_prefix("MSQUICK") {
        return QuickSelect::parse(s.trim()).map(Reply::QuickSelect);
    }
    if let Some(s) = line.strip_prefix("PSTONE CTRL ") {
        return boolean(s).map(Reply::ToneEnabled);
    }
    if let Some(s) = line.strip_prefix("PSCINEMA EQ.") {
        return boolean(s.trim()).map(Reply::CinemaEq);
    }
    if let Some(s) = line.strip_prefix("PSMULTEQ:") {
        return MultEq::parse(s.trim()).map(Reply::MultEq);
    }
    if let Some(s) = line.strip_prefix("PSDYNEQ ") {
        return boolean(s).map(Reply::DynamicEq);
    }
    if let Some(s) = line.strip_prefix("PSREFLEV ") {
        return ReferenceLevel::parse(s).map(Reply::ReferenceLevel);
    }
    if let Some(s) = line.strip_prefix("PSDYNVOL ") {
        return DynamicVolume::parse(s).map(Reply::DynamicVolume);
    }
    if let Some(s) = line.strip_prefix("PSDEH ") {
        return DialogEnhancer::parse(s).map(Reply::DialogEnhancer);
    }
    if let Some(s) = line.strip_prefix("PSBAS") {
        return Tone::parse(s.trim()).map(Reply::Bass);
    }
    if let Some(s) = line.strip_prefix("PSTRE") {
        return Tone::parse(s.trim()).map(Reply::Treble);
    }
    if let Some(s) = line.strip_prefix("PW") {
        return match s {
            "ON" => Some(Reply::Power(true)),
            "STANDBY" => Some(Reply::Power(false)),
            _ => None,
        };
    }
    if let Some(s) = line.strip_prefix("SI") {
        return Source::parse(s).map(Reply::Source);
    }
    if let Some(s) = line.strip_prefix("MS") {
        return SoundMode::parse(s).map(Reply::SoundMode);
    }
    if let Some(s) = line.strip_prefix("ECO") {
        return Eco::parse(s.trim()).map(Reply::Eco);
    }
    if let Some(s) = line.strip_prefix("Z2") {
        return boolean(s)
            .map(Reply::Zone2Power)
            .or_else(|| ZoneVolume::parse(s).map(Reply::Zone2Volume))
            .or_else(|| Source::parse(s).map(Reply::Zone2Source));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numerical_bounds_and_wire_format() {
        assert_eq!(Tone::new(-7), None);
        assert_eq!(Tone::new(7), None);
        assert_eq!(Setting::Bass(Tone::new(-6).unwrap()).command(), "PSBAS44");
        assert_eq!(
            parse_reply("PSTRE 56"),
            Some(Reply::Treble(Tone::new(6).unwrap()))
        );
        assert_eq!(parse_reply("PSBAS43"), None);
        assert_eq!(parse_reply("PSBAS050"), None);
        for value in [f64::NAN, -0.5, 98.5, 1.25] {
            assert_eq!(ZoneVolume::new(value), None);
        }
        assert_eq!(
            Setting::Zone2Volume(ZoneVolume::new(5.5).unwrap()).command(),
            "Z2055"
        );
        assert_eq!(
            parse_reply("Z298"),
            Some(Reply::Zone2Volume(ZoneVolume::new(98.0).unwrap()))
        );
        assert_eq!(parse_reply("Z2985"), None);
        assert_eq!(parse_reply("Z2051"), None);
    }
    #[test]
    fn candidates_round_trip_and_reject_unknowns() {
        let cases = [
            Setting::Power(true),
            Setting::Power(false),
            Setting::Source(Source::Tv),
            Setting::SoundMode(SoundMode::Stereo),
            Setting::QuickSelect(QuickSelect::Four),
            Setting::Eco(Eco::Auto),
            Setting::Bass(Tone::new(0).unwrap()),
            Setting::ToneEnabled(true),
            Setting::CinemaEq(false),
            Setting::MultEq(MultEq::BypassLr),
            Setting::DynamicEq(true),
            Setting::ReferenceLevel(ReferenceLevel::Fifteen),
            Setting::DynamicVolume(DynamicVolume::Heavy),
            Setting::Zone2Power(false),
            Setting::Zone2Source(Source::Cd),
            Setting::DialogEnhancer(DialogEnhancer::High),
        ];
        for setting in cases {
            assert!(parse_reply(&setting.command()).is_some(), "{:?}", setting);
        }
        assert_eq!(parse_reply("PSDEH ON"), None);
        assert_eq!(parse_reply("CVSW ON"), None);
        assert_eq!(parse_reply("PSSWL ON"), None);
        assert_eq!(parse_reply("PSTONE CTRL ?"), None);
        assert_eq!(parse_reply("Z2UP"), None);
        assert_eq!(Query::MultEq.as_str(), "PSMULTEQ: ?");
        assert_eq!(Query::CinemaEq.as_str(), "PSCINEMA EQ. ?");
    }
}

impl Query {
    pub const ALL: [Self; 15] = [
        Self::Power,
        Self::Source,
        Self::SoundMode,
        Self::QuickSelect,
        Self::Eco,
        Self::Bass,
        Self::Treble,
        Self::ToneEnabled,
        Self::CinemaEq,
        Self::MultEq,
        Self::DynamicEq,
        Self::ReferenceLevel,
        Self::DynamicVolume,
        Self::Zone2,
        Self::DialogEnhancer,
    ];
}
impl Setting {
    pub fn query(self) -> Query {
        match self {
            Self::Power(_) => Query::Power,
            Self::Source(_) => Query::Source,
            Self::SoundMode(_) => Query::SoundMode,
            Self::QuickSelect(_) => Query::QuickSelect,
            Self::Eco(_) => Query::Eco,
            Self::Bass(_) => Query::Bass,
            Self::Treble(_) => Query::Treble,
            Self::ToneEnabled(_) => Query::ToneEnabled,
            Self::CinemaEq(_) => Query::CinemaEq,
            Self::MultEq(_) => Query::MultEq,
            Self::DynamicEq(_) => Query::DynamicEq,
            Self::ReferenceLevel(_) => Query::ReferenceLevel,
            Self::DynamicVolume(_) => Query::DynamicVolume,
            Self::Zone2Power(_) | Self::Zone2Source(_) | Self::Zone2Volume(_) => Query::Zone2,
            Self::DialogEnhancer(_) => Query::DialogEnhancer,
        }
    }
    pub fn matches_reply(self, reply: &Reply) -> bool {
        self == reply.setting()
    }
}
impl Reply {
    fn setting(self) -> Setting {
        match self {
            Self::Power(v) => Setting::Power(v),
            Self::Source(v) => Setting::Source(v),
            Self::SoundMode(v) => Setting::SoundMode(v),
            Self::QuickSelect(v) => Setting::QuickSelect(v),
            Self::Eco(v) => Setting::Eco(v),
            Self::Bass(v) => Setting::Bass(v),
            Self::Treble(v) => Setting::Treble(v),
            Self::ToneEnabled(v) => Setting::ToneEnabled(v),
            Self::CinemaEq(v) => Setting::CinemaEq(v),
            Self::MultEq(v) => Setting::MultEq(v),
            Self::DynamicEq(v) => Setting::DynamicEq(v),
            Self::ReferenceLevel(v) => Setting::ReferenceLevel(v),
            Self::DynamicVolume(v) => Setting::DynamicVolume(v),
            Self::Zone2Power(v) => Setting::Zone2Power(v),
            Self::Zone2Source(v) => Setting::Zone2Source(v),
            Self::Zone2Volume(v) => Setting::Zone2Volume(v),
            Self::DialogEnhancer(v) => Setting::DialogEnhancer(v),
        }
    }
    pub fn query(self) -> Query {
        self.setting().query()
    }
    /// Identifies the same field even when the value differs.
    pub fn same_field(self, setting: Setting) -> bool {
        std::mem::discriminant(&self.setting()) == std::mem::discriminant(&setting)
    }
}

/// Current, explicitly reported prerequisites. None means not reported.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prerequisites {
    pub tone_available: Option<bool>,
    pub tone_enabled: Option<bool>,
    pub dynamic_eq_available: Option<bool>,
    pub reference_level_available: Option<bool>,
    pub dynamic_volume_available: Option<bool>,
    pub mult_eq_available: Option<bool>,
    pub main_sources: Option<Vec<SourceChoice>>,
    pub zone2_sources: Option<Vec<SourceChoice>>,
    pub zone2_present: Option<bool>,
    pub zone2_volume_limit: Option<f64>,
    pub zone2_volume_variable: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceChoice {
    pub id: Source,
    pub name: String,
}
