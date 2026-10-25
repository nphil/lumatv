import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// Receiver (docs/mocks/24-settings-receiver and its Audyssey, levels and zones tabs): the Denon's own
// controls, one page below Sound, drawn only from what the helper's `receiver` topic says (PLAN, Receiver:
// the receiver's own units and names, values read back after every change, and any value older than 6 s
// without an answer is hidden, not shown stale). The page itself holds no state of the receiver's: a row
// reads the topic and sends `POST /v1/receiver/set {what, value}`; the next reading is what the row then shows.
//
// The helper's receiver module reads power, input, volume and mute today and says `controls: false`: then
// the page shows those and dims every control, with the reason in the first row. When the module learns the
// Denon's commands it adds `controls: true` and these fields, which the rows below read:
//   model, soundMode, modes [{id, title, sub, icon}], inputs [name], quick [{n, label}], quickSelect,
//   audyssey {multEq: reference|bypass|flat|off, dynEq, refOffset: 0|5|10|15, dynVol: off|light|medium|heavy},
//   levels {dialog, subwoofer, bass, treble} (dB), cinemaEq, eco (off|on|auto), zone2 {power, volumeAbs, input}.
PageList {
    id: page
    title: "Receiver"
    back: "Sound"
    backId: "sound"
    tabs: ["Sound mode", "Levels", "Audyssey", "Zones & power"]

    readonly property var rc: Helper.receiver || ({})
    readonly property bool answers: !!(rc.reachable && rc.fresh)
    readonly property bool ctl: Helper.receiverControls && answers
    readonly property var aud: rc.audyssey || ({})
    readonly property var lev: rc.levels || ({})
    readonly property var z2: rc.zone2 || ({})
    chip: answers ? "Answering" : "Not answering"
    chipTone: answers ? "positive" : "danger"
    subtitle: (rc.model || "Denon receiver") + (answers ? " \u00b7 " + (rc.power === false ? "Standby" : "On") + (rc.input ? " \u00b7 " + rc.input : "") + (rc.soundMode ? " \u00b7 " + rc.soundMode : "") : "")
    summary: answers ? (rc.volumeText || "On") : "Not answering"

    function set(what, value) { Helper.receiverSet(what, value) }
    function why() { return !answers ? "The receiver is not answering." : Helper.receiverControls ? "" : "This helper reads the receiver but cannot send it commands yet." }
    function en() { return ctl }
    function indexIn(list, v) { var i = (list || []).indexOf(v); return i < 0 ? undefined : i }

    rows: [
        R.note(function() { return page.why() }, 0, function() { return page.why() !== "" }),
        { kind: "toggle", tab: 0, icon: "power", label: "Receiver power", detail: "Turns on with the TV over HDMI-CEC. Off here keeps the TV on.",
          enabled: function() { return page.ctl }, on: function() { return page.rc.power !== false }, act: function(v) { page.set("power", v) } },
        { kind: "stepper", tab: 0, icon: "cable", label: "Input", detail: function() { return "What the receiver plays. " + (page.rc.input || "") },
          enabled: function() { return page.ctl && (page.rc.inputs || []).length > 0 }, options: function() { return page.rc.inputs || [] },
          index: function() { return page.indexIn(page.rc.inputs, page.rc.input) }, act: function(i) { page.set("input", page.rc.inputs[i]) } },
        R.head(function() { return "Sound mode" + (page.rc.soundMode ? " \u00b7 now " + page.rc.soundMode : "") }, 0),
        { kind: "cards", tab: 0, cols: 4, enabled: function() { return page.ctl && (page.rc.modes || []).length > 0 },
          options: function() { return (page.rc.modes || []).map(function(m) { return { title: m.title, sub: m.sub, icon: m.icon || "audio-waveform" } }) },
          index: function() { var m = page.rc.modes || []; for (var i = 0; i < m.length; i++) if (m[i].id === page.rc.soundModeId) return i; return undefined },
          act: function(i) { page.set("soundMode", page.rc.modes[i].id) } },
        { kind: "seg", tab: 0, icon: "zap", label: "Quick Select", detail: "Four memories: input, volume and mode in one press.",
          enabled: function() { return page.ctl && (page.rc.quick || []).length > 0 }, options: function() { return (page.rc.quick || []).map(function(q) { return q.n + " \u00b7 " + q.label }) },
          index: function() { return page.rc.quickSelect === undefined ? undefined : page.rc.quickSelect - 1 }, act: function(i) { page.set("quickSelect", i + 1) } },

        { kind: "slider", tab: 1, icon: "volume-2", label: "Volume", detail: "Read live. The volume keys move it.", min: 0, max: 98, step: 1, info: true,
          num: function() { return page.answers && page.rc.volumeAbs !== undefined ? page.rc.volumeAbs : 0 } },
        { kind: "slider", tab: 1, icon: "mic", label: "Dialogue", detail: "Lifts speech above the effects.", min: -12, max: 12, step: 1, unit: " dB",
          enabled: function() { return page.ctl && page.lev.dialog !== undefined }, num: function() { return page.lev.dialog || 0 }, act: function(v) { page.set("dialog", v) } },
        { kind: "slider", tab: 1, icon: "speaker", label: "Subwoofer", detail: "The bass the receiver sends to the sub.", min: -12, max: 12, step: 1, unit: " dB",
          enabled: function() { return page.ctl && page.lev.subwoofer !== undefined }, num: function() { return page.lev.subwoofer || 0 }, act: function(v) { page.set("subwoofer", v) } },
        { kind: "slider", tab: 1, icon: "sliders-vertical", label: "Bass", detail: "", min: -6, max: 6, step: 1, unit: " dB",
          enabled: function() { return page.ctl && page.lev.bass !== undefined }, num: function() { return page.lev.bass || 0 }, act: function(v) { page.set("bass", v) } },
        { kind: "slider", tab: 1, icon: "sliders-vertical", label: "Treble", detail: "", min: -6, max: 6, step: 1, unit: " dB",
          enabled: function() { return page.ctl && page.lev.treble !== undefined }, num: function() { return page.lev.treble || 0 }, act: function(v) { page.set("treble", v) } },
        { kind: "toggle", tab: 1, icon: "theater", label: "Cinema EQ", detail: "Softens harsh highs in film soundtracks.",
          enabled: function() { return page.ctl && page.rc.cinemaEq !== undefined }, on: function() { return !!page.rc.cinemaEq }, act: function(v) { page.set("cinemaEq", v) } },
        { kind: "seg", tab: 1, icon: "leaf", label: "ECO mode", detail: "Lowers the receiver's power draw.", options: ["Off", "On", "Auto"],
          enabled: function() { return page.ctl && page.rc.eco !== undefined }, index: function() { return page.indexIn(["off", "on", "auto"], page.rc.eco) },
          act: function(i) { page.set("eco", ["off", "on", "auto"][i]) } },

        R.note("Audyssey is the room correction the receiver measured with its microphone. These switch its parts on and off; re-measuring is done on the receiver.", 2),
        { kind: "seg", tab: 2, icon: "audio-lines", label: "MultEQ", detail: "Reference is Audyssey's curve; Flat keeps the highs.", options: ["Reference", "L/R bypass", "Flat", "Off"],
          enabled: function() { return page.ctl && page.aud.multEq !== undefined }, index: function() { return page.indexIn(["reference", "bypass", "flat", "off"], page.aud.multEq) },
          act: function(i) { page.set("multEq", ["reference", "bypass", "flat", "off"][i]) } },
        { kind: "toggle", tab: 2, icon: "activity", label: "Dynamic EQ", detail: "Keeps bass and warmth full at low volume.",
          enabled: function() { return page.ctl && page.aud.dynEq !== undefined }, on: function() { return !!page.aud.dynEq }, act: function(v) { page.set("dynEq", v) } },
        { kind: "seg", tab: 2, icon: "gauge", label: "Reference level offset", detail: "For Dynamic EQ: 0 dB for films, 5 to 15 for TV and music mixed quieter.", options: ["0 dB", "5", "10", "15"],
          enabled: function() { return page.ctl && page.aud.refOffset !== undefined }, index: function() { return page.indexIn([0, 5, 10, 15], page.aud.refOffset) },
          act: function(i) { page.set("refOffset", [0, 5, 10, 15][i]) } },
        { kind: "seg", tab: 2, icon: "volume-1", label: "Dynamic Volume", detail: "Evens out loud adverts and quiet dialogue.", options: ["Off", "Light", "Medium", "Heavy"],
          enabled: function() { return page.ctl && page.aud.dynVol !== undefined }, index: function() { return page.indexIn(["off", "light", "medium", "heavy"], page.aud.dynVol) },
          act: function(i) { page.set("dynVol", ["off", "light", "medium", "heavy"][i]) } },

        { kind: "toggle", tab: 3, icon: "power", label: "Zone 2", detail: "A second room on the same receiver.",
          enabled: function() { return page.ctl && page.z2.power !== undefined }, on: function() { return !!page.z2.power }, act: function(v) { page.set("zone2Power", v) } },
        { kind: "slider", tab: 3, icon: "volume-2", label: "Zone 2 volume", detail: "", min: 0, max: 98, step: 1,
          enabled: function() { return page.ctl && page.z2.volumeAbs !== undefined }, num: function() { return page.z2.volumeAbs || 0 }, act: function(v) { page.set("zone2Volume", v) } }
    ]
}
