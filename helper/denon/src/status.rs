//! The status XML and what is read from it.

use std::fmt;

/// The receiver's main-zone power.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Power {
    On,
    /// Off. The set's S760H writes `OFF` (other models `STANDBY`) and keeps answering on its network port, with its last
    /// input and volume still reported.
    Standby,
    /// A value this code does not know, as the receiver wrote it.
    Other(String),
}

impl Power {
    fn from_raw(raw: &str) -> Self {
        match raw {
            "ON" => Self::On,
            "STANDBY" | "OFF" => Self::Standby,
            other => Self::Other(other.to_owned()),
        }
    }

    /// A short lowercase word for the API: `on`, `standby` or `unknown`.
    pub fn word(&self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Standby => "standby",
            Self::Other(_) => "unknown",
        }
    }
}

/// Which scale the receiver's own display shows the volume on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VolumeDisplay {
    /// 0 to 98 on the front panel: the dB value plus 80.
    Absolute,
    /// Decibels relative to the reference level, as `MasterVolume` is always written.
    Relative,
    /// Not in the reply, or a value this code does not know.
    Unknown,
}

impl VolumeDisplay {
    /// A short lowercase word for the API.
    pub fn word(&self) -> &'static str {
        match self {
            Self::Absolute => "absolute",
            Self::Relative => "relative",
            Self::Unknown => "unknown",
        }
    }
}

/// What the Lite status says. A field the reply leaves out or writes as `--` (some receivers do that for the volume while off;
/// the set's does not) is `None`; nothing is guessed.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub power: Power,
    /// `InputFuncSelect` as the receiver names it: `TV`, `SAT/CBL`, `MPLAY`, ...
    pub input: Option<String>,
    pub volume_display: VolumeDisplay,
    /// `MasterVolume` in dB relative to the reference level (the reply carries dB even when the panel shows 0 to 98).
    pub volume_db: Option<f64>,
    /// `Mute`: on is `Some(true)`.
    pub mute: Option<bool>,
}

impl Status {
    /// The volume as the receiver's panel shows it in `Absolute` mode (0 to 98): the dB value plus 80.
    pub fn volume_absolute(&self) -> Option<f64> {
        self.volume_db.map(|db| db + 80.0)
    }
}

/// Why a reply is not a status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// No `<item>` element: a web page or an error text, not the status XML.
    NotStatus,
    /// The element that every status carries is missing.
    Missing(&'static str),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStatus => f.write_str("the reply is not the receiver's status XML"),
            Self::Missing(tag) => write!(f, "the status has no {tag}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// The text of `<Tag><value>TEXT</value></Tag>`: tolerant of white space and line breaks between the elements, case-sensitive
/// as the receiver writes the names.
fn value_of<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let from = xml.find(&open)? + open.len();
    let inner = &xml[from..from + xml[from..].find(&close)?];
    let start = inner.find("<value>")? + "<value>".len();
    let end = start + inner[start..].find("</value>")?;
    Some(inner[start..end].trim())
}

/// `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` and numeric references: renamed inputs can carry them.
fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix('#')
                .and_then(|n| match n.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => n.parse::<u32>().ok(),
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Reads the Lite status: `Power` is required; the input, the volume and the mute are read when present.
pub fn parse_status_lite(body: &str) -> Result<Status, ParseError> {
    if !body.contains("<item>") {
        return Err(ParseError::NotStatus);
    }
    let power = value_of(body, "Power").ok_or(ParseError::Missing("Power"))?;
    let input = value_of(body, "InputFuncSelect")
        .map(unescape)
        .filter(|s| !s.is_empty());
    let volume_display = match value_of(body, "VolumeDisplay") {
        Some("Absolute") => VolumeDisplay::Absolute,
        Some("Relative") => VolumeDisplay::Relative,
        _ => VolumeDisplay::Unknown,
    };
    let volume_db = value_of(body, "MasterVolume")
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite());
    let mute = value_of(body, "Mute").and_then(|v| {
        if v.eq_ignore_ascii_case("on") {
            Some(true)
        } else if v.eq_ignore_ascii_case("off") {
            Some(false)
        } else {
            None
        }
    });
    Ok(Status {
        power: Power::from_raw(power),
        input,
        volume_display,
        volume_db,
        mute,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reply the set's receiver gave on 2026-10-25 (captured with one GET, body only).
    const ON_THE_SET: &str = include_str!("../tests/fixtures/status-lite-on-tv.xml");
    /// The reply on 2026-10-25 with the receiver off (it follows the TV into standby over HDMI-CEC): `OFF`, and the last volume kept.
    const OFF_ON_THE_SET: &str = include_str!("../tests/fixtures/status-lite-off-on-tv.xml");
    const VOLUME_DASHES: &str = include_str!("../tests/fixtures/status-lite-volume-dashes.xml");
    const RELATIVE_MUTED: &str = include_str!("../tests/fixtures/status-lite-relative-muted.xml");
    const ONE_LINE: &str = include_str!("../tests/fixtures/status-lite-one-line.xml");

    #[test]
    fn the_receivers_own_reply_is_read() {
        let s = parse_status_lite(ON_THE_SET).unwrap();
        assert_eq!(s.power, Power::On);
        assert_eq!(s.input.as_deref(), Some("TV"));
        assert_eq!(s.volume_display, VolumeDisplay::Absolute);
        assert_eq!(s.volume_db, Some(-44.5));
        assert_eq!(s.volume_absolute(), Some(35.5));
        assert_eq!(s.mute, Some(false));
    }

    #[test]
    fn off_is_standby_and_the_receiver_keeps_reporting_its_last_volume() {
        let s = parse_status_lite(OFF_ON_THE_SET).unwrap();
        assert_eq!(s.power, Power::Standby);
        assert_eq!(s.power.word(), "standby");
        assert_eq!(s.input.as_deref(), Some("TV"));
        assert_eq!(s.volume_db, Some(-44.5));
    }

    #[test]
    fn dashes_are_no_volume() {
        let s = parse_status_lite(VOLUME_DASHES).unwrap();
        assert_eq!(s.power, Power::Standby);
        assert_eq!(s.volume_db, None);
        assert_eq!(s.volume_absolute(), None);
        assert_eq!(s.input.as_deref(), Some("SAT/CBL"));
    }

    #[test]
    fn relative_display_and_mute_on() {
        let s = parse_status_lite(RELATIVE_MUTED).unwrap();
        assert_eq!(s.volume_display, VolumeDisplay::Relative);
        assert_eq!(s.volume_db, Some(-12.0));
        assert_eq!(s.mute, Some(true));
        // The input name is renamed by the owner and carries an escaped ampersand.
        assert_eq!(s.input.as_deref(), Some("Apple TV & Co"));
    }

    #[test]
    fn white_space_between_elements_does_not_matter() {
        let s = parse_status_lite(ONE_LINE).unwrap();
        assert_eq!(s.power, Power::On);
        assert_eq!(s.input.as_deref(), Some("GAME"));
        assert_eq!(s.volume_db, Some(0.5));
        assert_eq!(s.mute, Some(false));
    }

    #[test]
    fn an_error_page_is_not_a_status() {
        assert_eq!(
            parse_status_lite("Error 403: Forbidden\nAccess Forbidden\n"),
            Err(ParseError::NotStatus)
        );
        assert_eq!(parse_status_lite(""), Err(ParseError::NotStatus));
    }

    #[test]
    fn a_status_without_power_is_refused() {
        let xml = "<?xml version=\"1.0\"?><item><Mute><value>off</value></Mute></item>";
        assert_eq!(parse_status_lite(xml), Err(ParseError::Missing("Power")));
    }

    #[test]
    fn missing_or_odd_fields_are_none_not_guesses() {
        let xml = "<item><Power><value>ON</value></Power><MasterVolume><value>loud</value></MasterVolume>\
                   <Mute><value>maybe</value></Mute><VolumeDisplay><value>Sideways</value></VolumeDisplay></item>";
        let s = parse_status_lite(xml).unwrap();
        assert_eq!(s.volume_db, None);
        assert_eq!(s.mute, None);
        assert_eq!(s.volume_display, VolumeDisplay::Unknown);
        assert_eq!(s.input, None);
    }

    #[test]
    fn an_unknown_power_value_is_kept_as_written() {
        let xml = "<item><Power><value>WARMING</value></Power></item>";
        let s = parse_status_lite(xml).unwrap();
        assert_eq!(s.power, Power::Other("WARMING".into()));
        assert_eq!(s.power.word(), "unknown");
    }

    #[test]
    fn entities_are_decoded_once() {
        assert_eq!(unescape("a &amp; b"), "a & b");
        assert_eq!(unescape("&amp;lt;"), "&lt;");
        assert_eq!(unescape("&#65;&#x42;"), "AB");
        assert_eq!(unescape("fish & chips"), "fish & chips");
        assert_eq!(unescape("&bogus; x"), "&bogus; x");
        assert_eq!(unescape("trailing &"), "trailing &");
    }
}
