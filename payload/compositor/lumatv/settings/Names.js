.pragma library

// LG's own English names for the values its Settings app shows, one table per kind of value, taken from the `qH` tables of
// research/lgsettings/main.js (docs/research/settings-coverage.md). A row names its table; a value the table lacks shows as the
// service gave it, so a value LG adds later still reads. The lists of values themselves are the TV's (`getSystemSettingDesc`), not
// these tables.

var tables = {
    onoff: { on: "On", off: "Off" },
    slowToFast: { verySlow: "Very slow", slow: "Slow", medium: "Medium", fast: "Fast", veryFast: "Very fast" },
    lowToHigh: { off: "Off", veryLow: "Very low", low: "Low", medium: "Medium", high: "High", veryHigh: "Very high" },
    supplementaryAudio: { off: "Normal", audioDescription: "Audio description", spokenSubtitles: "Spoken subtitle", allType: "Any supplementary audio" },
    additionalAudioSelection: { none: "None", audioDescription: "Audio description", cleanAudio: "Clean audio", spokenSubtitles: "Spoken subtitle" },
    hearingAid: { off: "Off", bluetooth: "Bluetooth", auracast: "Auracast", external_arc: "HDMI (ARC)" },
    captionStyle: { manual: "Custom", auto: "Set by program" },
    captionSize: { small: "Small", standard: "Standard", large: "Large" },
    captionFont: { font0: "Font 1", font1: "Font 2", font2: "Font 3", font3: "Font 4", font4: "Font 5", font5: "Font 6", font6: "Font 7", font7: "Font 8" },
    captionColor: { white: "White", black: "Black", red: "Red", green: "Green", blue: "Blue", cyan: "Cyan", magenta: "Magenta", yellow: "Yellow" },
    captionOpacity: { flash: "Flash", translucent: "Translucent", transparent: "Transparent", solid: "Solid" },
    captionEdgeType: { none: "None", uniform: "Uniform", raised: "Raised", depressed: "Depressed", left_drop_shadow: "Left drop shadow", right_drop_shadow: "Right drop shadow" },
    captionAnalog: { cc1: "CC 1", cc2: "CC 2", cc3: "CC 3", cc4: "CC 4", t1: "Text 1", t2: "Text 2", t3: "Text 3", t4: "Text 4" },
    captionDigital: { off: "Off", svc1: "Service 1", svc2: "Service 2", svc3: "Service 3", svc4: "Service 4", svc5: "Service 5", svc6: "Service 6" },
    digitalPlusCaption: { eng: "English", spa: "Spanish", fre: "French" },
    deviceCnt: { singleConnection: "1 device", dualConnection: "2 devices (headset only)" },
    automaticStandBy: { 0: "Off", 2: "2 hours", 4: "4 hours", 6: "6 hours", 8: "8 hours" },
    timerInput: { dtv: "Live TV", atv: "Analog TV", av1: "AV 1", av2: "AV 2", comp1: "Component 1", comp2: "Component 2", comp3: "Component 3", rgb: "RGB",
                  hdmi1: "HDMI 1", hdmi2: "HDMI 2", hdmi3: "HDMI 3", hdmi4: "HDMI 4" },
    captionPosOption: { auto: "Set by program", autoAi: "Automatic", manual: "Manual", splitDown16_9: "Separated caption (bottom)", splitUp16_9: "Separated caption (top)" },
    lmha: { off: "Off", on: "On", low: "Low", medium: "Medium", high: "High", auto: "Auto", user: "User" },
    gamma: { low: "1.9", medium: "2.2", high1: "2.4", high2: "BT.1886", mediumHavingLevel: "2.2" },
    colorGamut: { wide: "Wide", extended: "Extended", normal: "Normal", auto: "Auto Detect", native: "Native", dynamic: "Dynamic", srgb: "sRGB", adobeRGB: "Adobe RGB", "dci-p3": "Cinema" },
    truMotion: { off: "Off", smooth: "Smooth Movement", clear: "Clear", user: "User Selection", cinemaClear: "Cinematic Movement", natural: "Natural" },
    toneMapping: { on: "On", off: "Off", HGIG: "HGiG" },
    expression: { auto: "Auto", on: "On", off: "Off", detail: "Detail", highlight: "Brightness" },
    soundOutput: { tv_speaker: "TV Speaker", external_arc: "HDMI(ARC) Device", external_optical: "Optical Out Device", bt_soundbar: "Bluetooth Device",
                   wow_cast: "LG WOWCAST Device", mobile_phone: "Mobile Device", lineout: "Audio Out Device", headphone: "Wired Headphones", usb_speaker: "USB Device",
                   tv_speaker_bluetooth: "Bluetooth Device + TV Speaker", tv_speaker_wow_cast: "LG WOWCAST Device + TV Speaker",
                   tv_external_speaker: "Optical Out Device + TV Speaker", tv_speaker_external_arc: "HDMI(ARC) Device + TV Speaker",
                   tv_speaker_headphone: "Wired Headphones + TV Speaker", wisa_speaker: "WiSA Speakers" },
    tvSetup: { stand: "Stand", wall_mount: "Wall-mounted" },
    dtvAudio: { mpeg: "MPEG", dra: "DRA", dolbyDigital: "Dolby Digital", dobyDigitalPlus: "Dolby Digital Plus", heAaC: "HE-AAC", auto: "Auto" },
    hdmiAudio: { bitstream: "Bitstream", pcm: "PCM" },
    magicSound: { off: "Off", standard: "Standard", bass: "Bass Boost", treble: "Treble Boost", flat: "Flat Mode" },
    movieRating: { off: "Off", g: "G", pg: "PG", pg13: "PG-13", r: "R", "nc-17": "NC-17", x: "X" }
};

// The name for `value` in `table` (a table name or an object); `fallback` (default: the value itself) when it has none.
function name(table, value, fallback) {
    var t = typeof table === "string" ? tables[table] : table;
    var v = String(value);
    if (t && t[v] !== undefined) return t[v];
    return fallback === undefined ? v : fallback;
}
