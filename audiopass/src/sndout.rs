//! What the kernel says about the sound card: `/proc/lgtv-driver/audio/sndout` and one `adec.p<N>` per decoder input. Both are plain
//! procfs text, read without the sound card's control lock. A port of the validation in `lgroute-control.c` (dts_restore_plus,
//! LGPL-2.1-or-later).

use std::fmt;
use std::str::FromStr;

pub const SNDOUT: &str = "/proc/lgtv-driver/audio/sndout";
/// The kernel's view of one ADEC decoder input.
pub fn adec_path(index: u8) -> String {
    format!("/proc/lgtv-driver/audio/adec.p{index}")
}

/// The five words of the `Sndout EARC Outputtype` control. At rest on the C4: `2,1,0,0,1`; un-mixed: `2,1,0,0,0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Words {
    pub set_type: u32,
    pub codec: u32,
    pub channels: u32,
    pub rate: u32,
    /// 1 when LG mixes its system sounds into the eARC stream, which turns a bypassed stream into decoded PCM.
    pub mix: u32,
}

impl Words {
    /// The ranges upstream accepts; anything else is not a state to write back.
    pub fn valid(self) -> bool {
        self.set_type <= 2
            && (1..=9).contains(&self.codec)
            && self.channels <= 8
            && self.rate <= 192_000
            && self.mix <= 1
    }

    pub fn unmixed(self) -> Self {
        Self { mix: 0, ..self }
    }
}

impl fmt::Display for Words {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{},{},{},{},{}",
            self.set_type, self.codec, self.channels, self.rate, self.mix
        )
    }
}

impl FromStr for Words {
    type Err = &'static str;

    /// `a,b,c,d,e`, as the control is written.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut parts = text.split(',').map(|part| part.trim().parse::<u32>());
        let mut next = || parts.next().and_then(Result::ok).ok_or("not five numbers");
        let words = Self {
            set_type: next()?,
            codec: next()?,
            channels: next()?,
            rate: next()?,
            mix: next()?,
        };
        if parts.next().is_some() {
            return Err("not five numbers");
        }
        Ok(words)
    }
}

/// The parts of `sndout` the route reads. The `[ARC]` section carries words of its own; the eARC words come only from `[EARC]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sndout {
    pub earc_on: Option<bool>,
    pub earc: Option<Words>,
    /// `[ARC] OutputCodec`: `Pass-Through` when the kernel hands the stream on untouched.
    pub arc_codec: Option<String>,
    /// `[Misc] ConnectedAdec`: the ADEC inputs wired to the main output, empty when none.
    pub connected_adec: Option<String>,
}

impl Sndout {
    pub fn bypassing(&self) -> bool {
        self.arc_codec.as_deref() == Some("Pass-Through")
    }

    pub fn mixed(&self) -> bool {
        self.earc.is_some_and(|words| words.mix != 0)
    }

    /// The ADEC inputs wired to the main output, by the numbers the kernel lists.
    pub fn connected_adecs(&self) -> Vec<&str> {
        self.connected_adec
            .as_deref()
            .map(|list| {
                list.split(',')
                    .map(str::trim)
                    .filter(|adec| !adec.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn adec_connected(&self, index: u8) -> bool {
        self.connected_adecs().contains(&index.to_string().as_str())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Arc,
    Earc,
    Misc,
    Other,
}

pub fn parse(text: &str) -> Sndout {
    let mut out = Sndout::default();
    let mut section = Section::Other;
    let mut words = [None::<u32>; 5];
    for line in text.lines() {
        let line = line.trim_end();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = match name.to_ascii_lowercase().as_str() {
                "arc" => Section::Arc,
                "earc" => Section::Earc,
                "misc" => Section::Misc,
                _ => Section::Other,
            };
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match (section, key) {
            (Section::Earc, "EArcOnOff") => {
                out.earc_on = match value {
                    "On" => Some(true),
                    "Off" => Some(false),
                    _ => None,
                }
            }
            (Section::Earc, "Set_type") => words[0] = value.parse().ok(),
            (Section::Earc, "Codec_type") => words[1] = value.parse().ok(),
            (Section::Earc, "Channel_num") => words[2] = value.parse().ok(),
            (Section::Earc, "Sample_rate") => words[3] = value.parse().ok(),
            (Section::Earc, "Mix_option") => words[4] = value.parse().ok(),
            (Section::Arc, "OutputCodec") => out.arc_codec = Some(value.to_owned()),
            (Section::Misc, "ConnectedAdec") => out.connected_adec = Some(value.to_owned()),
            _ => {}
        }
    }
    if let [
        Some(set_type),
        Some(codec),
        Some(channels),
        Some(rate),
        Some(mix),
    ] = words
    {
        out.earc = Some(Words {
            set_type,
            codec,
            channels,
            rate,
            mix,
        });
    }
    out
}

/// The kernel's view of one ADEC input (`adec.p<N>`): one line of `Key=Value` pairs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adec {
    pub open: Option<bool>,
    /// The input it is fed from, such as `HDMI_PORT2`. While nothing is connected this is LG's record of the last use.
    pub connect: Option<String>,
    /// Whether the decoder is running.
    pub start: Option<bool>,
    pub gain: Option<String>,
    pub mute: Option<bool>,
}

impl Adec {
    /// The line in the kernel's own layout; what the journal keeps and what a restore is compared with.
    pub fn render(&self) -> String {
        let flag = |value: bool| if value { "1" } else { "0" };
        let mut parts = Vec::new();
        if let Some(open) = self.open {
            parts.push(format!("Open={}", flag(open)));
        }
        if let Some(connect) = &self.connect {
            parts.push(format!("Connect={connect}"));
        }
        if let Some(start) = self.start {
            parts.push(format!("Start={}", flag(start)));
        }
        if let Some(gain) = &self.gain {
            parts.push(format!("Gain={gain}"));
        }
        if let Some(mute) = self.mute {
            parts.push(format!("Mute={}", flag(mute)));
        }
        parts.join(" ")
    }
}

pub fn parse_adec(text: &str) -> Adec {
    let flag = |value: &str| match value {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    };
    let mut out = Adec::default();
    for pair in text.split_whitespace() {
        match pair.split_once('=') {
            Some(("Open", value)) => out.open = flag(value),
            Some(("Connect", value)) => out.connect = Some(value.to_owned()),
            Some(("Start", value)) => out.start = flag(value),
            Some(("Gain", value)) => out.gain = Some(value.to_owned()),
            Some(("Mute", value)) => out.mute = flag(value),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout `alsa_sndout.c` prints, with the values the C4 showed (EARC at rest, bypass not yet requested).
    pub(crate) const REST: &str = "[ARC]\nOutputCodec=PCM\nOutputChannel=2\nSet_type=1\nCodec_type=1\nMix_option=1\nUnderflow=9\n\
        [EARC]\nEArcOnOff=On\nSet_type=2\nCodec_type=1\nChannel_num=0\nSample_rate=0\nMix_option=1\nUnderflow=0\n\
        [Misc]\nConnectedAdec=\nConnectedAmixer=5,6,7\nMainAudioOutput=ADEC1\nConnectedMixerPort=0x0\n";

    #[test]
    fn rest_state_parses_and_arc_words_do_not_leak_into_earc() {
        let s = parse(REST);
        assert_eq!(s.earc_on, Some(true));
        assert_eq!(s.earc.unwrap().to_string(), "2,1,0,0,1");
        assert!(s.mixed());
        assert!(!s.bypassing());
        assert!(s.connected_adecs().is_empty());
        assert_eq!(s.connected_adec.as_deref(), Some(""));
    }

    #[test]
    fn unmixed_bypass_and_connected_adec_read_back() {
        let text = REST
            .replace("OutputCodec=PCM", "OutputCodec=Pass-Through")
            .replace("[EARC]\nEArcOnOff=On\nSet_type=2\nCodec_type=1\nChannel_num=0\nSample_rate=0\nMix_option=1", "[EARC]\nEArcOnOff=On\nSet_type=2\nCodec_type=1\nChannel_num=0\nSample_rate=0\nMix_option=0")
            .replace("ConnectedAdec=\n", "ConnectedAdec=1\n");
        let s = parse(&text);
        assert!(s.bypassing());
        assert!(!s.mixed());
        assert!(s.adec_connected(1) && !s.adec_connected(2));
        assert_eq!(s.earc.unwrap().unmixed(), s.earc.unwrap());
    }

    #[test]
    fn missing_or_garbled_words_read_as_unknown() {
        assert_eq!(parse("[EARC]\nEArcOnOff=On\nSet_type=2\n").earc, None);
        assert_eq!(
            parse(&REST.replace("Sample_rate=0", "Sample_rate=x")).earc,
            None
        );
        assert_eq!(parse("").earc_on, None);
        assert_eq!(
            parse(&REST.replace("EArcOnOff=On", "EArcOnOff=Off")).earc_on,
            Some(false)
        );
    }

    #[test]
    fn words_validate_upstream_ranges() {
        let ok = Words {
            set_type: 2,
            codec: 1,
            channels: 0,
            rate: 0,
            mix: 1,
        };
        assert!(ok.valid());
        assert!(!Words { set_type: 3, ..ok }.valid());
        assert!(!Words { codec: 0, ..ok }.valid());
        assert!(!Words { codec: 10, ..ok }.valid());
        assert!(!Words { channels: 9, ..ok }.valid());
        assert!(
            !Words {
                rate: 192_001,
                ..ok
            }
            .valid()
        );
        assert!(!Words { mix: 2, ..ok }.valid());
    }

    #[test]
    fn adec_line_reads_input_and_mute() {
        let a = parse_adec("Open=1 Connect=HDMI_PORT2 Start=0 Gain=0x800000 Mute=1\n");
        assert_eq!(a.connect.as_deref(), Some("HDMI_PORT2"));
        assert_eq!(a.mute, Some(true));
        assert_eq!(parse_adec("Mute=0").mute, Some(false));
        assert_eq!(parse_adec("").connect, None);
    }
}
