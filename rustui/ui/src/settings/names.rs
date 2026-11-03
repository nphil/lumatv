//! LG's own English names for the values its Settings app shows, one table per kind of value (`settings/Names.js` of the QML shell,
//! taken from the `qH` tables of LG's bundle). A row names its table; a value the table lacks shows as the service gave it, so a
//! value LG adds later still reads. The lists of values themselves are the TV's (`getSystemSettingDesc`), not these tables.

pub type Table = &'static [(&'static str, &'static str)];

/// The name for `value` in `table`; the value itself when the table has none.
pub fn name(table: Table, value: &str) -> String {
    table.iter().find(|(k, _)| *k == value).map_or_else(|| value.to_string(), |(_, n)| (*n).to_string())
}

/// The name for `value`, or `fallback` when the table has none (`N.name(table, value, "")`).
pub fn name_or(table: Table, value: &str, fallback: &str) -> String {
    table.iter().find(|(k, _)| *k == value).map_or_else(|| fallback.to_string(), |(_, n)| (*n).to_string())
}

/// LG's picture-mode captions; unknown native values remain readable as given.
pub fn picture_mode(value: &str) -> &str {
    match value {
        "filmMaker" => "Filmmaker",
        "cinema" => "Cinema",
        "normal" => "Standard",
        "vivid" => "Vivid",
        "personalized" => "Personalized",
        "game" => "Game",
        "sports" => "Sports",
        "eco" => "Eco",
        "expert1" => "Expert · day",
        "expert2" => "Expert · night",
        "dolbyHdrCinemaBright" => "Cinema Home",
        _ => value,
    }
}

pub const NONE: Table = &[];
pub const ONOFF: Table = &[("on", "On"), ("off", "Off")];
pub const SLOW_TO_FAST: Table = &[("verySlow", "Very slow"), ("slow", "Slow"), ("medium", "Medium"), ("fast", "Fast"), ("veryFast", "Very fast")];
pub const LOW_TO_HIGH: Table = &[("off", "Off"), ("veryLow", "Very low"), ("low", "Low"), ("medium", "Medium"), ("high", "High"), ("veryHigh", "Very high")];
pub const SUPPLEMENTARY_AUDIO: Table =
    &[("off", "Normal"), ("audioDescription", "Audio description"), ("spokenSubtitles", "Spoken subtitle"), ("allType", "Any supplementary audio")];
pub const ADDITIONAL_AUDIO_SELECTION: Table =
    &[("none", "None"), ("audioDescription", "Audio description"), ("cleanAudio", "Clean audio"), ("spokenSubtitles", "Spoken subtitle")];
pub const HEARING_AID: Table = &[("off", "Off"), ("bluetooth", "Bluetooth"), ("auracast", "Auracast"), ("external_arc", "HDMI (ARC)")];
pub const CAPTION_STYLE: Table = &[("manual", "Custom"), ("auto", "Set by program")];
pub const CAPTION_SIZE: Table = &[("small", "Small"), ("standard", "Standard"), ("large", "Large")];
pub const CAPTION_FONT: Table = &[
    ("font0", "Font 1"),
    ("font1", "Font 2"),
    ("font2", "Font 3"),
    ("font3", "Font 4"),
    ("font4", "Font 5"),
    ("font5", "Font 6"),
    ("font6", "Font 7"),
    ("font7", "Font 8"),
];
pub const CAPTION_COLOR: Table =
    &[("white", "White"), ("black", "Black"), ("red", "Red"), ("green", "Green"), ("blue", "Blue"), ("cyan", "Cyan"), ("magenta", "Magenta"), ("yellow", "Yellow")];
pub const CAPTION_OPACITY: Table = &[("flash", "Flash"), ("translucent", "Translucent"), ("transparent", "Transparent"), ("solid", "Solid")];
pub const CAPTION_EDGE_TYPE: Table = &[
    ("none", "None"),
    ("uniform", "Uniform"),
    ("raised", "Raised"),
    ("depressed", "Depressed"),
    ("left_drop_shadow", "Left drop shadow"),
    ("right_drop_shadow", "Right drop shadow"),
];
pub const CAPTION_ANALOG: Table = &[("cc1", "CC 1"), ("cc2", "CC 2"), ("cc3", "CC 3"), ("cc4", "CC 4"), ("t1", "Text 1"), ("t2", "Text 2"), ("t3", "Text 3"), ("t4", "Text 4")];
pub const CAPTION_DIGITAL: Table =
    &[("off", "Off"), ("svc1", "Service 1"), ("svc2", "Service 2"), ("svc3", "Service 3"), ("svc4", "Service 4"), ("svc5", "Service 5"), ("svc6", "Service 6")];
pub const DIGITAL_PLUS_CAPTION: Table = &[("eng", "English"), ("spa", "Spanish"), ("fre", "French")];
pub const DEVICE_CNT: Table = &[("singleConnection", "1 device"), ("dualConnection", "2 devices (headset only)")];
pub const SPEAKER_POSITION: Table =
    &[("left", "Left"), ("right", "Right"), ("front", "Front"), ("rear", "Rear"), ("center", "Center"), ("none", "None")];
pub const AUTOMATIC_STAND_BY: Table = &[("0", "Off"), ("2", "2 hours"), ("4", "4 hours"), ("6", "6 hours"), ("8", "8 hours")];
pub const TIMER_INPUT: Table = &[
    ("dtv", "Live TV"),
    ("atv", "Analog TV"),
    ("av1", "AV 1"),
    ("av2", "AV 2"),
    ("comp1", "Component 1"),
    ("comp2", "Component 2"),
    ("comp3", "Component 3"),
    ("rgb", "RGB"),
    ("hdmi1", "HDMI 1"),
    ("hdmi2", "HDMI 2"),
    ("hdmi3", "HDMI 3"),
    ("hdmi4", "HDMI 4"),
];
pub const CAPTION_POS_OPTION: Table = &[
    ("auto", "Set by program"),
    ("autoAi", "Automatic"),
    ("manual", "Manual"),
    ("splitDown16_9", "Separated caption (bottom)"),
    ("splitUp16_9", "Separated caption (top)"),
];
pub const LMHA: Table = &[("off", "Off"), ("on", "On"), ("low", "Low"), ("medium", "Medium"), ("high", "High"), ("auto", "Auto"), ("user", "User")];
pub const GAMMA: Table = &[("low", "1.9"), ("medium", "2.2"), ("high1", "2.4"), ("high2", "BT.1886"), ("mediumHavingLevel", "2.2")];
pub const COLOR_GAMUT: Table = &[
    ("wide", "Wide"),
    ("extended", "Extended"),
    ("normal", "Normal"),
    ("auto", "Auto Detect"),
    ("native", "Native"),
    ("dynamic", "Dynamic"),
    ("srgb", "sRGB"),
    ("adobeRGB", "Adobe RGB"),
    ("dci-p3", "Cinema"),
];
pub const TRU_MOTION: Table = &[
    ("off", "Off"),
    ("smooth", "Smooth Movement"),
    ("clear", "Clear"),
    ("user", "User Selection"),
    ("cinemaClear", "Cinematic Movement"),
    ("natural", "Natural"),
];
pub const TONE_MAPPING: Table = &[("on", "On"), ("off", "Off"), ("HGIG", "HGiG")];
pub const EXPRESSION: Table = &[("auto", "Auto"), ("on", "On"), ("off", "Off"), ("detail", "Detail"), ("highlight", "Brightness")];
pub const SOUND_OUTPUT: Table = &[
    ("tv_speaker", "TV Speaker"),
    ("external_arc", "HDMI(ARC) Device"),
    ("external_optical", "Optical Out Device"),
    ("bt_soundbar", "Bluetooth Device"),
    ("wow_cast", "LG WOWCAST Device"),
    ("mobile_phone", "Mobile Device"),
    ("lineout", "Audio Out Device"),
    ("headphone", "Wired Headphones"),
    ("usb_speaker", "USB Device"),
    ("tv_speaker_bluetooth", "Bluetooth Device + TV Speaker"),
    ("tv_speaker_wow_cast", "LG WOWCAST Device + TV Speaker"),
    ("tv_external_speaker", "Optical Out Device + TV Speaker"),
    ("tv_speaker_external_arc", "HDMI(ARC) Device + TV Speaker"),
    ("tv_speaker_headphone", "Wired Headphones + TV Speaker"),
    ("wisa_speaker", "WiSA Speakers"),
];
pub const TV_SETUP: Table = &[("stand", "Stand"), ("wall_mount", "Wall-mounted")];
pub const DTV_AUDIO: Table =
    &[("mpeg", "MPEG"), ("dra", "DRA"), ("dolbyDigital", "Dolby Digital"), ("dobyDigitalPlus", "Dolby Digital Plus"), ("heAaC", "HE-AAC"), ("auto", "Auto")];
pub const HDMI_AUDIO: Table = &[("bitstream", "Bitstream"), ("pcm", "PCM")];
pub const MAGIC_SOUND: Table = &[("off", "Off"), ("standard", "Standard"), ("bass", "Bass Boost"), ("treble", "Treble Boost"), ("flat", "Flat Mode")];
pub const MOVIE_RATING: Table = &[("off", "Off"), ("g", "G"), ("pg", "PG"), ("pg13", "PG-13"), ("r", "R"), ("nc-17", "NC-17"), ("x", "X")];
pub const BLACK_LEVEL: Table = &[("low", "Limited"), ("high", "Full"), ("auto", "Auto"), ("medium", "Medium"), ("limited", "Limited"), ("full", "Full")];
pub const WB_METHOD: Table = &[("2", "2 Points"), ("10", "10 Points Signal Level(%)"), ("22", "22 Points Signal Level(%)")];
pub const WB_POINT: Table = &[("low", "Low"), ("high", "High")];
pub const OLED_CARE: Table = &[("off", "Off"), ("comfortCare", "Comfort Mode"), ("cinemaCare", "Theater Mode"), ("kidsCare", "Kids Care Mode")];
pub const COLOR_SYSTEM: Table = &[("red", "Red"), ("green", "Green"), ("blue", "Blue"), ("cyan", "Cyan"), ("magenta", "Magenta"), ("yellow", "Yellow")];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_reads_in_lgs_name_and_one_the_table_lacks_reads_as_given() {
        assert_eq!(name(GAMMA, "high2"), "BT.1886");
        assert_eq!(name(LMHA, "veryNew"), "veryNew");
        assert_eq!(name_or(ONOFF, "maybe", ""), "");
        assert_eq!(name(SOUND_OUTPUT, "external_arc"), "HDMI(ARC) Device");
    }
}
