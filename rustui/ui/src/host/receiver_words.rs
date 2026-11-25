//! The Denon receiver's own words for what it plays, as a person reads them. The helper's `receiver` topic carries the sound mode
//! (`MS<text>`: `DOLBY AUDIO-DD+DSUR`) and the input signal (`SYSDA <text>`: `Dolby Atmos - DD+`, `PCM ZERO`) raw. The mode names the
//! decoder the receiver ended on; where it does not name the carrier (`DOLBY ATMOS`, `DOLBY AUDIO-DSUR` over whatever arrives) the signal
//! does. A word this table does not know is shown as the receiver sent it.

/// ASCII case-insensitive substring test, without copying either side.
fn has(text: &str, part: &str) -> bool {
    let (t, p) = (text.as_bytes(), part.as_bytes());
    t.windows(p.len()).any(|w| w.eq_ignore_ascii_case(p))
}

fn has_any(text: &str, parts: &[&str]) -> bool {
    parts.iter().any(|p| has(text, p))
}

/// The carrier a mode leaves unsaid, from the signal's words: `Dolby Atmos` rides on TrueHD or Digital Plus, a Dolby upmix works on what
/// arrives. `PCM`, `PCM ZERO` (silence), `Unknown` and anything else name no carrier.
fn carrier(signal: &str) -> &'static str {
    // (words in the signal, name); the first row that matches wins, so `Dolby Digital Plus` is not read as `Dolby Digital`.
    const CARRIERS: &[(&[&str], &str)] = &[
        (&["TRUEHD"], "Dolby TrueHD"),
        (&["DD+", "DIGITAL PLUS", "DIGITAL+", "EAC3"], "Dolby Digital Plus"),
        (&["DIGITAL", "AC3", "DD"], "Dolby Digital"),
        (&["DTS-HD"], "DTS-HD"),
        (&["DTS:X"], "DTS:X"),
        (&["DTS"], "DTS"),
    ];
    CARRIERS.iter().find(|(words, _)| has_any(signal, words)).map_or("", |(_, name)| name)
}

/// Whole names for modes that carry their own format.
const MODES: &[(&[&str], &str)] = &[
    (&["TRUEHD"], "Dolby TrueHD"),
    (&["DD+", "D+", "DIGITAL PLUS"], "Dolby Digital Plus"),
    (&["AUDIO-DD", "DOLBY DIGITAL"], "Dolby Digital"),
    (&["DTS-HD MSTR", "DTS-HD MA"], "DTS-HD Master Audio"),
    (&["DTS-HD HRA"], "DTS-HD High Resolution"),
    (&["DTS-HD"], "DTS-HD"),
    (&["DTS:X"], "DTS:X"),
    (&["DTS SURROUND", "DTS ES", "DTS 96"], "DTS"),
    (&["AURO"], "Auro-3D"),
    (&["PURE DIRECT"], "Pure Direct"),
    (&["DIRECT"], "Direct"),
    (&["MCH STEREO", "MULTI CH STEREO", "M CH STEREO"], "Multi Channel Stereo"),
    (&["STEREO"], "Stereo"),
    (&["MULTI CH IN", "M CH IN"], "Multi Channel In"),
];

/// What the receiver applies on top of the carrier, after the carrier's name.
const UPMIXERS: &[(&[&str], &str)] = &[(&["DSUR", "DOLBY SURROUND"], "Dolby Surround"), (&["NEURAL"], "DTS Neural:X")];

fn push(out: &mut String, word: &str) {
    if !out.is_empty() {
        out.push_str(" \u{b7} ");
    }
    out.push_str(word);
}

/// The words for what the receiver plays; "" when the mode says nothing (not read yet). `signal` may be "".
pub fn words(sound_mode: &str, signal: &str) -> String {
    let mode = sound_mode.trim();
    if mode.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(48);
    // Atmos is the decoder's work over a Dolby carrier: the carrier follows as what it arrived on.
    if has(mode, "ATMOS") {
        push(&mut out, "Dolby Atmos");
        match carrier(signal) {
            "Dolby TrueHD" => push(&mut out, "TrueHD"),
            "Dolby Digital Plus" => push(&mut out, "Digital Plus"),
            _ => {}
        }
        return out;
    }
    // The bare `DTS` is the mode's own name; as a part of `DTS NEURAL:X` it is the upmixer's.
    let named = MODES
        .iter()
        .find(|(words, _)| has_any(mode, words))
        .map(|(_, name)| *name)
        .or_else(|| mode.eq_ignore_ascii_case("DTS").then_some("DTS"));
    let upmixer = UPMIXERS.iter().find(|(words, _)| has_any(mode, words)).map(|(_, name)| *name);
    // Over a carrier the upmix follows its name; over PCM or silence it stands alone. A word that is neither is shown as sent.
    let first = match (named, upmixer) {
        (Some(name), _) => name,
        (None, Some(mix)) => Some(carrier(signal)).filter(|c| !c.is_empty()).unwrap_or(mix),
        (None, None) => return mode.to_owned(),
    };
    push(&mut out, first);
    if let Some(name) = upmixer.filter(|name| *name != first) {
        push(&mut out, name);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::words;

    #[test]
    fn the_receivers_words_become_the_format_it_plays() {
        for (mode, signal, shown) in [
            ("DOLBY ATMOS", "Dolby TrueHD", "Dolby Atmos \u{b7} TrueHD"),
            ("DOLBY ATMOS", "Dolby Atmos - DD+", "Dolby Atmos \u{b7} Digital Plus"),
            ("DOLBY ATMOS", "", "Dolby Atmos"),
            ("DOLBY AUDIO-TRUEHD", "Dolby TrueHD", "Dolby TrueHD"),
            ("DOLBY AUDIO-DD+", "Dolby Digital+", "Dolby Digital Plus"),
            ("DOLBY AUDIO-DD+ +DSUR", "", "Dolby Digital Plus \u{b7} Dolby Surround"),
            ("DOLBY AUDIO-DD+DSUR", "Dolby Digital+", "Dolby Digital Plus \u{b7} Dolby Surround"),
            ("DOLBY AUDIO-DD", "DOLBY DIGITAL", "Dolby Digital"),
            ("DOLBY AUDIO-DSUR", "PCM", "Dolby Surround"),
            ("DOLBY AUDIO-DSUR", "PCM ZERO", "Dolby Surround"),
            ("DOLBY AUDIO-DSUR", "Unknown", "Dolby Surround"),
            ("DOLBY AUDIO-DSUR", "Dolby TrueHD", "Dolby TrueHD \u{b7} Dolby Surround"),
            ("DOLBY AUDIO-DSUR", "DOLBY DIGITAL", "Dolby Digital \u{b7} Dolby Surround"),
            ("DTS-HD MSTR", "DTS-HD", "DTS-HD Master Audio"),
            ("DTS:X", "DTS:X", "DTS:X"),
            ("DTS SURROUND", "DTS", "DTS"),
            ("STEREO", "PCM", "Stereo"),
            ("DIRECT", "PCM", "Direct"),
            ("PURE DIRECT", "PCM ZERO", "Pure Direct"),
            ("MCH STEREO", "PCM", "Multi Channel Stereo"),
            ("MULTI CH IN", "PCM", "Multi Channel In"),
            ("AURO3D", "PCM", "Auro-3D"),
            ("NEURAL:X", "PCM", "DTS Neural:X"),
            ("NEURAL:X", "DTS", "DTS \u{b7} DTS Neural:X"),
            ("ROCK ARENA", "PCM", "ROCK ARENA"),
            ("  JAZZ CLUB ", "", "JAZZ CLUB"),
            ("", "Dolby TrueHD", ""),
        ] {
            assert_eq!(words(mode, signal), shown, "{mode} over {signal}");
        }
    }
}
