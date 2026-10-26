.pragma library

// The receiver's audio format as the short words its own front display uses, from what the Denon says (read-only queries, S760H, 2026-10-25):
//   `SYSDA <text>`   the input signal, in words ("PCM ZERO" while nothing plays)     -> the format
//   `MS<text>`       the sound mode ("DOLBY AUDIO-DSUR", "STEREO", "MCH STEREO", ...) -> the mode word after it
// The helper publishes both, untouched, as `signal` and `soundMode` of the `receiver` topic. This turns them into something fit to read at the
// TV's distance, and into nothing when they say nothing: no raw code is ever shown. Seen on the set so far: "PCM ZERO" with "DOLBY AUDIO-DSUR"
// (idle). The Atmos, Digital, Digital+, TrueHD, DTS and Multi Ch In patterns below follow the receiver's published naming and have not been seen
// on the set yet; a text that matches none of them is not shown.

// The format of the input signal, or "" when it carries none.
function stream(signal) {
    var t = String(signal || "").toUpperCase().replace(/\s+/g, " ").replace(/^ | $/g, "")
    if (t === "" || /ZERO|NO SIGNAL|NONE|NON$/.test(t)) return ""
    if (/ATMOS/.test(t)) return "Dolby Atmos"
    if (/TRUE ?HD/.test(t)) return "Dolby TrueHD"
    if (/DD\+|D\+|DIGITAL PLUS|E-?AC-?3/.test(t)) return "Dolby Digital+"
    if (/DOLBY|DD|AC-?3/.test(t)) return "Dolby Digital"
    if (/DTS[ :-]?X/.test(t)) return "DTS:X"
    if (/DTS[ -]?HD/.test(t)) return "DTS-HD"
    if (/DTS/.test(t)) return "DTS"
    if (/MULTI ?CH|MCH ?IN/.test(t)) return "Multi Ch In"
    if (/PCM/.test(t)) {
        var layout = t.match(/(\d)\.(\d)/)
        if (layout) return "PCM " + layout[1] + "." + layout[2]
        var channels = t.match(/(\d)\s?CH/)
        return channels && channels[1] === "2" ? "PCM 2.0" : "PCM"
    }
    return ""
}

// The word the sound mode adds after the format, or "".
function mode(soundMode) {
    var t = String(soundMode || "").toUpperCase()
    if (/PURE DIRECT/.test(t)) return "Pure Direct"
    if (/DIRECT/.test(t)) return "Direct"
    if (/DSUR|SURROUND|PL2|PLII|NEO|NEURAL/.test(t) && !/DTS SURROUND/.test(t)) return "Surround"
    if (/^STEREO/.test(t)) return "Stereo"
    return ""
}

// "Dolby Digital+ · Surround", "DTS:X", "PCM 2.0": or "" while the format is not known.
function describe(soundMode, signal) {
    var format = stream(signal)
    if (format === "") return ""
    var word = mode(soundMode)
    if (word === "Surround" || word === "Direct" || word === "Pure Direct") return format + " \u00b7 " + word
    if (word === "Stereo" && format.indexOf("PCM") !== 0) return format + " \u00b7 Stereo"
    return format
}
