//! Typed settings commands from Denon's telnet protocol (2015 and FY21 command charts). Support must be established by the caller.

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
// The wire text is the MS command. `MultiChIn` and `AllZoneStereo` are only ever reported, never commands.
values!(SoundMode { Stereo => "STEREO", Direct => "DIRECT", PureDirect => "PURE DIRECT", Auto => "AUTO", Dolby => "DOLBY DIGITAL", Dts => "DTS SURROUND", MultiChStereo => "MCH STEREO", RockArena => "ROCK ARENA", JazzClub => "JAZZ CLUB", MonoMovie => "MONO MOVIE", VideoGame => "VIDEO GAME", Matrix => "MATRIX", Virtual => "VIRTUAL", MultiChIn => "MULTI CH IN", AllZoneStereo => "ALL ZONE STEREO" });
values!(SoundCategory { Movie => "MOVIE", Music => "MUSIC", Game => "GAME", Pure => "PURE" });
values!(Eco { On => "ON", Auto => "AUTO", Off => "OFF" });
values!(MultEq { Audyssey => "AUDYSSEY", BypassLr => "BYP.LR", Flat => "FLAT", Off => "OFF" });
values!(ReferenceLevel { Zero => "0", Five => "5", Ten => "10", Fifteen => "15" });
values!(DynamicVolume { Off => "OFF", Light => "LIT", Medium => "MED", Heavy => "HEV" });
values!(DialogEnhancer { Off => "OFF", Low => "LOW", Medium => "MED", High => "HIGH" });
values!(Drc { Off => "OFF", Auto => "AUTO", Low => "LOW", Medium => "MID", High => "HI" });
// `Unset` is the receiver's MSQUICK0: no memory recalled since the last change. It is never sent.
values!(QuickSelect { Unset => "0", One => "1", Two => "2", Three => "3", Four => "4" });
values!(Channel { FrontLeft => "FL", FrontRight => "FR", Center => "C", Subwoofer => "SW", Subwoofer2 => "SW2", SurroundLeft => "SL", SurroundRight => "SR", SurroundBackLeft => "SBL", SurroundBackRight => "SBR", SurroundBack => "SB", FrontHeightLeft => "FHL", FrontHeightRight => "FHR", TopFrontLeft => "TFL", TopFrontRight => "TFR", TopMiddleLeft => "TML", TopMiddleRight => "TMR", FrontDolbyLeft => "FDL", FrontDolbyRight => "FDR", SurroundDolbyLeft => "SDL", SurroundDolbyRight => "SDR" });

impl SoundMode {
    /// The styles a command can select. The reported-only names are excluded.
    pub fn writable(self) -> bool {
        !matches!(self, Self::MultiChIn | Self::AllZoneStereo)
    }

    /// Groups a reported mode name with the style command that selects it. The receiver names the decoder it ended on
    /// ("DOLBY AUDIO-DSUR", "M CH IN+NEURAL:X"), which depends on the input signal; the FY21 chart lists those names under
    /// the Dolby Digital and DTS Surround commands.
    pub fn reported(name: &str) -> Option<Self> {
        let name = name.trim();
        if let Some(mode) = Self::parse(name).or(match name {
            "DSD DIRECT" => Some(Self::Direct),
            "DSD PURE DIRECT" => Some(Self::PureDirect),
            "MULTI CH STEREO" => Some(Self::MultiChStereo),
            _ => None,
        }) {
            return Some(mode);
        }
        // An upmixer on another format's decoder belongs to the upmixer's command: "DTS+DSUR" is Dolby Surround on DTS.
        if name.contains("NEURAL:X") || name.contains("NERUAL:X") || name.contains("VIRTUAL:X") {
            Some(Self::Dts)
        } else if name.contains("DSUR") || name.starts_with("DOLBY") {
            Some(Self::Dolby)
        } else if name.starts_with("DTS") || name.starts_with("IMAX DTS") {
            Some(Self::Dts)
        } else if name.starts_with("MULTI CH IN") {
            Some(Self::MultiChIn)
        } else {
            None
        }
    }

    /// The one category this mode belongs to, when only one MOVIE/MUSIC/GAME/PURE button can reach it.
    pub fn category(self) -> Option<SoundCategory> {
        match self {
            Self::MonoMovie => Some(SoundCategory::Movie),
            Self::RockArena | Self::JazzClub | Self::Matrix => Some(SoundCategory::Music),
            Self::VideoGame => Some(SoundCategory::Game),
            Self::Direct | Self::PureDirect | Self::Auto => Some(SoundCategory::Pure),
            _ => None,
        }
    }
}

impl SoundCategory {
    /// Whether the button can leave the receiver in `mode`. The receiver recalls the category's last mode or steps to its
    /// next one, so the result is a member, not a predictable mode. PURE has no command of its own; its first press is Direct.
    pub fn admits(self, mode: SoundMode) -> bool {
        use SoundMode as M;
        let shared = matches!(
            mode,
            M::Stereo | M::Dolby | M::Dts | M::MultiChStereo | M::MultiChIn | M::Virtual
        );
        match self {
            Self::Movie => shared || mode == M::MonoMovie,
            Self::Music => shared || matches!(mode, M::RockArena | M::JazzClub | M::Matrix),
            Self::Game => shared || mode == M::VideoGame,
            Self::Pure => matches!(mode, M::Direct | M::PureDirect),
        }
    }
}

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

/// Reads the protocol's "**" or "**5" level: two digits, or three ending in 5 for the half step.
fn halves(value: &str) -> Option<u16> {
    if !value.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n = value.parse::<u16>().ok()?;
    match value.len() {
        2 => Some(n * 2),
        3 if n % 10 == 5 => Some(n / 10 * 2 + 1),
        _ => None,
    }
}
fn write_halves(halves: u16) -> String {
    if halves % 2 == 0 {
        format!("{:02}", halves / 2)
    } else {
        format!("{:02}5", halves / 2)
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
        Self::new(f64::from(halves(value)?) / 2.0)
    }
}

/// A level trim from -12 dB through +12 dB in half-dB steps, sent as 38 to 62 with 50 = 0 dB (subwoofer and channel levels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trim(i8);
impl Trim {
    pub fn new(db: f64) -> Option<Self> {
        if !db.is_finite() || !(-12.0..=12.0).contains(&db) || (db * 2.0).fract() != 0.0 {
            return None;
        }
        Some(Self((db * 2.0) as i8))
    }
    pub fn db(self) -> f64 {
        f64::from(self.0) / 2.0
    }
    fn wire(self) -> String {
        write_halves((100 + i16::from(self.0)) as u16)
    }
    fn parse(value: &str) -> Option<Self> {
        Self::new(f64::from(halves(value)?) / 2.0 - 50.0)
    }
}

/// LFE level from 0 dB down to -10 dB in whole dB; the wire counts the cut, 00 to 10.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lfe(u8);
impl Lfe {
    pub fn new(db: i8) -> Option<Self> {
        (-10..=0).contains(&db).then(|| Self(db.unsigned_abs()))
    }
    pub fn db(self) -> i8 {
        -(self.0 as i8)
    }
    fn parse(value: &str) -> Option<Self> {
        if value.len() != 2 || !value.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Self::new(-value.parse::<i8>().ok()?)
    }
}

/// DTS:X Dialog Control, 0 through 6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogControl(u8);
impl DialogControl {
    pub fn new(level: u8) -> Option<Self> {
        (level <= 6).then_some(Self(level))
    }
    pub fn level(self) -> u8 {
        self.0
    }
    fn parse(value: &str) -> Option<Self> {
        if value.len() != 2 || !value.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Self::new(value.parse().ok()?)
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
    Zone2Mute,
    DialogEnhancer,
    DialogControl,
    SubwooferLevel,
    SubwooferDirect,
    ChannelLevels,
    Lfe,
    Drc,
    Loudness,
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
            Self::Zone2Mute => "Z2MU?",
            Self::DialogEnhancer => "PSDEH ?",
            Self::DialogControl => "PSDIC ?",
            Self::SubwooferLevel => "PSSWL ?",
            Self::SubwooferDirect => "PSSWR ?",
            Self::ChannelLevels => "CV?",
            Self::Lfe => "PSLFE ?",
            Self::Drc => "PSDRC ?",
            Self::Loudness => "PSLOM ?",
        }
    }

    pub const ALL: [Self; 23] = [
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
        Self::Zone2Mute,
        Self::DialogEnhancer,
        Self::DialogControl,
        Self::SubwooferLevel,
        Self::SubwooferDirect,
        Self::ChannelLevels,
        Self::Lfe,
        Self::Drc,
        Self::Loudness,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Power(bool),
    Source(Source),
    SoundMode(SoundMode),
    SoundCategory(SoundCategory),
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
    Zone2Mute(bool),
    DialogEnhancer(DialogEnhancer),
    DialogControl(DialogControl),
    SubwooferLevel(Trim),
    SubwooferDirect(bool),
    ChannelLevel(Channel, Trim),
    Lfe(Lfe),
    Drc(Drc),
    Loudness(bool),
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
            Self::SoundCategory(SoundCategory::Pure) => "MSDIRECT".into(),
            Self::SoundCategory(v) => format!("MS{}", v.as_str()),
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
            Self::Zone2Volume(v) => format!("Z2{}", write_halves(u16::from(v.0))),
            Self::Zone2Mute(v) => format!("Z2MU{}", on(v)),
            Self::DialogEnhancer(v) => format!("PSDEH {}", v.as_str()),
            Self::DialogControl(v) => format!("PSDIC {:02}", v.0),
            Self::SubwooferLevel(v) => format!("PSSWL {}", v.wire()),
            Self::SubwooferDirect(v) => format!("PSSWR {}", on(v)),
            Self::ChannelLevel(c, v) => format!("CV{} {}", c.as_str(), v.wire()),
            Self::Lfe(v) => format!("PSLFE {:02}", v.0),
            Self::Drc(v) => format!("PSDRC {}", v.as_str()),
            Self::Loudness(v) => format!("PSLOM {}", on(v)),
        }
    }

    pub fn query(self) -> Query {
        match self {
            Self::Power(_) => Query::Power,
            Self::Source(_) => Query::Source,
            Self::SoundMode(_) | Self::SoundCategory(_) => Query::SoundMode,
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
            Self::Zone2Mute(_) => Query::Zone2Mute,
            Self::DialogEnhancer(_) => Query::DialogEnhancer,
            Self::DialogControl(_) => Query::DialogControl,
            Self::SubwooferLevel(_) => Query::SubwooferLevel,
            Self::SubwooferDirect(_) => Query::SubwooferDirect,
            Self::ChannelLevel(..) => Query::ChannelLevels,
            Self::Lfe(_) => Query::Lfe,
            Self::Drc(_) => Query::Drc,
            Self::Loudness(_) => Query::Loudness,
        }
    }

    /// The read-back proves the write: the same value, or for a category button a mode that button can reach.
    pub fn matches_reply(self, reply: &Reply) -> bool {
        match (self, reply) {
            (Self::SoundCategory(category), Reply::SoundMode(mode)) => category.admits(*mode),
            _ => self == reply.setting(),
        }
    }
}

/// Only recognized, complete replies are returned; unknown values are not capability evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Power(bool),
    Source(Source),
    /// The style the reported mode belongs to; the exact name arrives separately as the receiver's mode.
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
    Zone2Mute(bool),
    DialogEnhancer(DialogEnhancer),
    DialogControl(DialogControl),
    SubwooferLevel(Trim),
    SubwooferDirect(bool),
    ChannelLevel(Channel, Trim),
    Lfe(Lfe),
    Drc(Drc),
    Loudness(bool),
}
impl Reply {
    /// The setting that would leave the receiver reporting this.
    pub fn setting(self) -> Setting {
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
            Self::Zone2Mute(v) => Setting::Zone2Mute(v),
            Self::DialogEnhancer(v) => Setting::DialogEnhancer(v),
            Self::DialogControl(v) => Setting::DialogControl(v),
            Self::SubwooferLevel(v) => Setting::SubwooferLevel(v),
            Self::SubwooferDirect(v) => Setting::SubwooferDirect(v),
            Self::ChannelLevel(c, v) => Setting::ChannelLevel(c, v),
            Self::Lfe(v) => Setting::Lfe(v),
            Self::Drc(v) => Setting::Drc(v),
            Self::Loudness(v) => Setting::Loudness(v),
        }
    }
    pub fn query(self) -> Query {
        self.setting().query()
    }
    /// Identifies the same field even when the value differs. Each channel level is its own field.
    pub fn same_field(self, setting: Setting) -> bool {
        match (self, setting) {
            (Self::SoundMode(_), Setting::SoundCategory(_)) => true,
            (Self::ChannelLevel(a, _), Setting::ChannelLevel(b, _)) => a == b,
            _ => std::mem::discriminant(&self.setting()) == std::mem::discriminant(&setting),
        }
    }
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
    if let Some(s) = line.strip_prefix("PSDIC ") {
        return DialogControl::parse(s).map(Reply::DialogControl);
    }
    // `PSSWL ?` also answers PSSWL ON and, with a second subwoofer, PSSWL2: neither is the level.
    if let Some(s) = line.strip_prefix("PSSWL ") {
        return Trim::parse(s).map(Reply::SubwooferLevel);
    }
    if let Some(s) = line.strip_prefix("PSSWR ") {
        return boolean(s).map(Reply::SubwooferDirect);
    }
    if let Some(s) = line.strip_prefix("PSLFE ") {
        return Lfe::parse(s).map(Reply::Lfe);
    }
    if let Some(s) = line.strip_prefix("PSDRC ") {
        return Drc::parse(s).map(Reply::Drc);
    }
    if let Some(s) = line.strip_prefix("PSLOM ") {
        return boolean(s).map(Reply::Loudness);
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
        return SoundMode::reported(s).map(Reply::SoundMode);
    }
    if let Some(s) = line.strip_prefix("ECO") {
        return Eco::parse(s.trim()).map(Reply::Eco);
    }
    if let Some(s) = line.strip_prefix("CV") {
        let (channel, level) = s.split_once(' ')?;
        return Some(Reply::ChannelLevel(Channel::parse(channel)?, Trim::parse(level)?));
    }
    if let Some(s) = line.strip_prefix("Z2MU") {
        return boolean(s).map(Reply::Zone2Mute);
    }
    if let Some(s) = line.strip_prefix("Z2") {
        return boolean(s)
            .map(Reply::Zone2Power)
            .or_else(|| ZoneVolume::parse(s).map(Reply::Zone2Volume))
            .or_else(|| Source::parse(s).map(Reply::Zone2Source));
    }
    None
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
        for db in [f64::NAN, -12.5, 12.5, 0.25] {
            assert_eq!(Trim::new(db), None);
        }
        let trim = |db| Trim::new(db).unwrap();
        assert_eq!(Setting::SubwooferLevel(trim(-12.0)).command(), "PSSWL 38");
        assert_eq!(
            Setting::ChannelLevel(Channel::Center, trim(-11.5)).command(),
            "CVC 385"
        );
        assert_eq!(
            Setting::ChannelLevel(Channel::TopFrontLeft, trim(11.5)).command(),
            "CVTFL 615"
        );
        assert_eq!(
            parse_reply("CVFHL 505"),
            Some(Reply::ChannelLevel(Channel::FrontHeightLeft, trim(0.5)))
        );
        assert_eq!(
            parse_reply("PSSWL 62"),
            Some(Reply::SubwooferLevel(trim(12.0)))
        );
        assert_eq!(parse_reply("PSSWL 37"), None);
        assert_eq!(parse_reply("CVSW 00"), None);
        assert_eq!(Lfe::new(1), None);
        assert_eq!(Lfe::new(-11), None);
        assert_eq!(Setting::Lfe(Lfe::new(-10).unwrap()).command(), "PSLFE 10");
        assert_eq!(parse_reply("PSLFE 00"), Some(Reply::Lfe(Lfe::new(0).unwrap())));
        assert_eq!(parse_reply("PSLFE 11"), None);
        assert_eq!(DialogControl::new(7), None);
        assert_eq!(
            Setting::DialogControl(DialogControl::new(3).unwrap()).command(),
            "PSDIC 03"
        );
        assert_eq!(parse_reply("PSDIC 07"), None);
    }

    #[test]
    fn candidates_round_trip_and_reject_unknowns() {
        let cases = [
            Setting::Power(true),
            Setting::Power(false),
            Setting::Source(Source::Tv),
            Setting::SoundMode(SoundMode::Stereo),
            Setting::SoundMode(SoundMode::RockArena),
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
            Setting::Zone2Mute(true),
            Setting::DialogEnhancer(DialogEnhancer::High),
            Setting::SubwooferDirect(false),
            Setting::Drc(Drc::High),
            Setting::Loudness(true),
            Setting::ChannelLevel(Channel::SurroundDolbyRight, Trim::new(-3.5).unwrap()),
        ];
        for setting in cases {
            let reply = parse_reply(&setting.command());
            assert!(
                reply.is_some_and(|r| setting.matches_reply(&r)),
                "{setting:?}"
            );
        }
        assert_eq!(parse_reply("MSQUICK0"), Some(Reply::QuickSelect(QuickSelect::Unset)));
        assert_eq!(parse_reply("Z2MUON"), Some(Reply::Zone2Mute(true)));
        assert_eq!(parse_reply("PSDEH ON"), None);
        assert_eq!(parse_reply("CVSW ON"), None);
        assert_eq!(parse_reply("CVEND"), None);
        assert_eq!(parse_reply("PSSWL ON"), None);
        assert_eq!(parse_reply("PSSWL2 50"), None);
        assert_eq!(parse_reply("PSTONE CTRL ?"), None);
        assert_eq!(parse_reply("Z2UP"), None);
        assert_eq!(parse_reply("MSAURO3D"), None);
        assert_eq!(Query::MultEq.as_str(), "PSMULTEQ: ?");
        assert_eq!(Query::CinemaEq.as_str(), "PSCINEMA EQ. ?");
    }

    #[test]
    fn reported_modes_group_under_the_command_that_selects_them() {
        for (name, style) in [
            ("DOLBY ATMOS", SoundMode::Dolby),
            ("DOLBY AUDIO-DD+DSUR", SoundMode::Dolby),
            ("DTS+DSUR", SoundMode::Dolby),
            ("M CH IN+DSUR", SoundMode::Dolby),
            ("DOLBY AUDIO-TRUEHD+NEURAL:X", SoundMode::Dts),
            ("NEURAL:X", SoundMode::Dts),
            ("DTS:X MSTR", SoundMode::Dts),
            ("MULTI CH IN 7.1", SoundMode::MultiChIn),
            ("DSD DIRECT", SoundMode::Direct),
            ("MCH STEREO", SoundMode::MultiChStereo),
        ] {
            assert_eq!(SoundMode::reported(name), Some(style), "{name}");
        }
        let category = Setting::SoundCategory(SoundCategory::Music);
        assert_eq!(category.command(), "MSMUSIC");
        assert_eq!(Setting::SoundCategory(SoundCategory::Pure).command(), "MSDIRECT");
        assert!(category.matches_reply(&Reply::SoundMode(SoundMode::JazzClub)));
        assert!(!category.matches_reply(&Reply::SoundMode(SoundMode::MonoMovie)));
        assert!(Reply::SoundMode(SoundMode::Stereo).same_field(category));
        assert_eq!(SoundMode::MonoMovie.category(), Some(SoundCategory::Movie));
        assert_eq!(SoundMode::Stereo.category(), None);
        let center = Setting::ChannelLevel(Channel::Center, Trim::new(0.0).unwrap());
        assert!(!Reply::ChannelLevel(Channel::FrontLeft, Trim::new(0.0).unwrap()).same_field(center));
    }
}
