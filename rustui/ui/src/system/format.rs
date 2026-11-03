//! The receiver's audio format as the short words its own front display uses (`kit/AudioFormat.js` of the QML shell, read-only
//! queries of the Denon S760H, 2026-10-25): the input signal (`SYSDA <text>`) and the sound mode (`MS<text>`) become something
//! fit to read at the TV's distance and the name of the mark the bar draws, and nothing when they say nothing. No raw code is
//! ever shown. "DOLBY ATMOS" as the sound mode over a Dolby carrier is the receiver decoding Atmos, which is what is heard.

fn norm(text: &str) -> String {
    text.to_uppercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has(t: &str, any: &[&str]) -> bool {
    any.iter().any(|p| t.contains(p))
}

/// The format of the input signal, or "" when it carries none.
pub fn stream(signal: &str) -> String {
    let t = norm(signal);
    if t.is_empty() || has(&t, &["ZERO", "NO SIGNAL", "NONE"]) || t.ends_with("NON") {
        return String::new();
    }
    let compact = t.replace([' ', '-'], "");
    if t.contains("ATMOS") {
        "Dolby Atmos"
    } else if compact.contains("TRUEHD") {
        "Dolby TrueHD"
    } else if compact.contains("DD+") || compact.contains("D+") || compact.contains("DIGITALPLUS") || compact.contains("DIGITAL+") || compact.contains("EAC3") {
        "Dolby Digital+"
    } else if t.contains("DOLBY") || t.contains("DD") || compact.contains("AC3") {
        "Dolby Digital"
    } else if compact.contains("DTSX") || compact.contains("DTS:X") {
        "DTS:X"
    } else if compact.contains("DTSHD") {
        "DTS-HD"
    } else if t.contains("DTS") {
        "DTS"
    } else if compact.contains("MULTICH") || compact.contains("MCHIN") {
        "Multi Ch In"
    } else if t.contains("PCM") {
        let digits: Vec<char> = t.chars().collect();
        for i in 0..digits.len().saturating_sub(2) {
            if digits[i].is_ascii_digit() && digits[i + 1] == '.' && digits[i + 2].is_ascii_digit() {
                return format!("PCM {}.{}", digits[i], digits[i + 2]);
            }
        }
        if compact.contains("2CH") {
            return "PCM 2.0".to_string();
        }
        "PCM"
    } else {
        ""
    }
    .to_string()
}

/// The word the sound mode adds after the format, or "".
pub fn mode(sound_mode: &str) -> &'static str {
    let t = norm(sound_mode);
    if t.contains("PURE DIRECT") {
        "Pure Direct"
    } else if t.contains("DIRECT") {
        "Direct"
    } else if has(&t, &["DSUR", "SURROUND", "PL2", "PLII", "NEO", "NEURAL"]) && !t.contains("DTS SURROUND") {
        "Surround"
    } else if t.starts_with("STEREO") {
        "Stereo"
    } else {
        ""
    }
}

/// The format heard: the receiver's Atmos decoding over a Dolby carrier is Atmos.
pub fn played(sound_mode: &str, signal: &str) -> String {
    let format = stream(signal);
    if norm(sound_mode).contains("ATMOS") && (format == "Dolby Digital+" || format == "Dolby TrueHD") {
        return "Dolby Atmos".to_string();
    }
    format
}

// PCM remains the carrier even when the receiver applies a branded upmixer.
fn upmixer(sound_mode: &str, carrier: &str) -> Option<(&'static str, &'static str)> {
    if !carrier.starts_with("PCM") && carrier != "Multi Ch In" { return None; }
    let mode = sound_mode.trim();
    if ["DOLBY AUDIO-DSUR", "DOLBY SURROUND", "DSUR"].iter().any(|m| mode.eq_ignore_ascii_case(m)) {
        Some(("Dolby Surround", "dolby"))
    } else if ["DTS NEURAL:X", "DTS NEURAL X", "DTS NEURALX", "NEURAL:X"].iter().any(|m| mode.eq_ignore_ascii_case(m)) {
        Some(("DTS Neural:X", "dts"))
    } else { None }
}

pub fn describe(sound_mode: &str, signal: &str) -> String {
    let format = played(sound_mode, signal);
    if format.is_empty() {
        return String::new();
    }
    if let Some((name, _)) = upmixer(sound_mode, &format) {
        return format!("{format} · {name}");
    }
    let word = mode(sound_mode);
    if format == "Dolby Atmos" || format == "DTS:X" {
        return format;
    }
    if matches!(word, "Surround" | "Direct" | "Pure Direct") || (word == "Stereo" && !format.starts_with("PCM")) {
        return format!("{format} \u{b7} {word}");
    }
    format
}

/// The mark the bar draws for the format (`icons/audio/<name>.png`), or "" for a format with no mark.
pub fn logo(sound_mode: &str, signal: &str) -> &'static str {
    match played(sound_mode, signal).as_str() {
        "Dolby Atmos" => "atmos",
        "Dolby Digital" | "Dolby Digital+" | "Dolby TrueHD" => "dolby",
        "DTS:X" => "dtsx",
        "DTS" | "DTS-HD" => "dts",
        carrier => upmixer(sound_mode, carrier).map_or("", |(_, logo)| logo),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sets_readings_become_the_fronts_words_and_marks() {
        assert_eq!(stream("PCM ZERO"), "");
        assert_eq!(describe("DOLBY AUDIO-DSUR", "PCM ZERO"), "", "idle shows nothing");
        assert_eq!(played("DOLBY ATMOS", "Dolby Atmos - DD+"), "Dolby Atmos");
        assert_eq!(logo("DOLBY ATMOS", "Dolby Atmos - DD+"), "atmos");
        assert_eq!(describe("DOLBY ATMOS", "Dolby Atmos - DD+"), "Dolby Atmos");
        assert_eq!(describe("DOLBY AUDIO-DSUR", "Dolby Digital+"), "Dolby Digital+ \u{b7} Surround");
        assert_eq!(logo("DOLBY AUDIO-DSUR", "Dolby Digital+"), "dolby");
        assert_eq!(describe("STEREO", "PCM 2CH"), "PCM 2.0");
        assert_eq!(logo("STEREO", "PCM 2.0"), "");
        assert_eq!(describe("DTS NEURAL:X", "DTS:X"), "DTS:X");
        assert_eq!(logo("MCH STEREO", "DTS-HD MA"), "dts");
        assert_eq!(describe("STEREO", "DTS"), "DTS \u{b7} Stereo");
        assert_eq!(describe("DOLBY ATMOS", "Dolby TrueHD"), "Dolby Atmos", "Atmos decoding over TrueHD");
        assert_eq!(stream("SOMETHING ELSE"), "");
    }
    #[test]
    fn pcm_upmixing_reports_processing_without_relabelling_the_input() {
        assert_eq!(played("DOLBY AUDIO-DSUR", "PCM"), "PCM");
        assert_eq!(logo("DOLBY AUDIO-DSUR", "PCM"), "dolby");
        assert_eq!(describe("DOLBY AUDIO-DSUR", "PCM"), "PCM · Dolby Surround");
        assert_eq!(played("DTS NEURAL:X", "PCM 2CH"), "PCM 2.0");
        assert_eq!(logo("DTS NEURAL:X", "PCM 2CH"), "dts");
        assert_eq!(describe("DTS NEURAL:X", "PCM 2CH"), "PCM 2.0 · DTS Neural:X");
        for signal in ["", "PCM ZERO", "NO SIGNAL", "SOMETHING ELSE"] {
            assert_eq!(logo("DOLBY AUDIO-DSUR", signal), "");
            assert_eq!(describe("DOLBY AUDIO-DSUR", signal), "");
        }
        for mode in ["STEREO", "DIRECT", "MULTI CH STEREO", "DOLBY ATMOS"] {
            assert_eq!(logo(mode, "PCM"), "");
        }
    }
}
