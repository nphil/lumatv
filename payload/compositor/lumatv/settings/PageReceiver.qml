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
//   levels {dialog, subwoofer, bass, treble} (dB), cinemaEq, eco (off|on|auto),
//   zone2 {name, power, volumeDb, volumeText, input}.
// Today's helper publishes none of this (it has no command route and nothing to read these back with: the receiver's
// full status page answers 403 on port 8080 and the Telnet port is Home Assistant's), which is why the page names that
// in its first row; scenes `24-settings-receiver*` publish the whole of it as a fixture of the contract.
PageList {
    id: page
    title: "Receiver"
    back: "Sound"
    backId: "sound"
    tabs: ["Sound mode", "Levels", "Audyssey", "Zones & power"]

    // the receiver's reading, only while it is in date (kit/ReceiverGate): 6 s after the last answer every value goes
    ReceiverGate { id: gate; active: page.visible }
    readonly property var rc: gate.doc || ({})
    readonly property bool answers: gate.fresh
    readonly property bool ctl: Helper.receiverControls && answers
    readonly property var aud: rc.audyssey || ({})
    readonly property var lev: rc.levels || ({})
    readonly property var z2: rc.zone2 || ({})
    chip: answers ? "Answering" : "Not answering"
    chipTone: answers ? "positive" : "danger"
    subtitle: rc.model || "Denon receiver"
    summary: answers ? (rc.volumeText || "On") : "Not answering"

    function set(what, value) { Helper.receiverSet(what, value) }
    function why() { return !answers ? "Receiver not answering" : Helper.receiverControls ? "" : "Read only for now" }
    function en() { return ctl }
    // a level in dB the way the receiver's display writes it: a sign and one decimal where it moves by halves
    function db(v, half) { return v === undefined || v === null ? "\u2013" : (v > 0 ? "+" : v < 0 ? "\u2212" : "") + (half ? Math.abs(v).toFixed(1) : String(Math.abs(v))) + " dB" }
    function indexIn(list, v) { var i = (list || []).indexOf(v); return i < 0 ? undefined : i }

    rows: [
        R.note(function() { return page.why() }, 0, function() { return page.why() !== "" }),
        { kind: "toggle", tab: 0, icon: "power", label: "Receiver power", help: "Turns on with the TV over HDMI-CEC. Off here keeps the TV on.",
          enabled: function() { return page.ctl }, on: function() { return page.rc.power !== false }, act: function(v) { page.set("power", v) } },
        { kind: "stepper", tab: 0, icon: "cable", label: "Input", help: "What the receiver plays. TV Audio is this TV over eARC.",
          enabled: function() { return page.ctl && (page.rc.inputs || []).length > 0 }, options: function() { return page.rc.inputs || [] },
          index: function() { return page.indexIn(page.rc.inputs, page.rc.input) }, act: function(i) { page.set("input", page.rc.inputs[i]) } },
        R.head(function() { return "Sound mode" + (page.rc.soundMode ? " \u00b7 now " + page.rc.soundMode : "") }, 0),
        { kind: "cards", tab: 0, cols: 4, enabled: function() { return page.ctl && (page.rc.modes || []).length > 0 },
          options: function() { return (page.rc.modes || []).map(function(m) { return { title: m.title, icon: m.icon || "audio-waveform" } }) },
          index: function() { var m = page.rc.modes || []; for (var i = 0; i < m.length; i++) if (m[i].id === page.rc.soundModeId) return i; return undefined },
          act: function(i) { page.set("soundMode", page.rc.modes[i].id) } },
        { kind: "seg", tab: 0, icon: "zap", label: "Quick Select", help: "Four memories: input, volume and mode in one press.",
          enabled: function() { return page.ctl && (page.rc.quick || []).length > 0 }, options: function() { return (page.rc.quick || []).map(function(q) { return q.n + " \u00b7 " + q.label }) },
          index: function() { return page.rc.quickSelect === undefined ? undefined : page.rc.quickSelect - 1 }, act: function(i) { page.set("quickSelect", i + 1) } },

        { kind: "slider", tab: 1, icon: "volume-2", label: "Volume", info: true,
          min: function() { return page.rc.volumeDisplay === "absolute" ? 0 : -80 }, max: function() { return page.rc.volumeDisplay === "absolute" ? 98 : 18 }, step: 0.5,
          num: function() { return page.answers ? (page.rc.volumeDisplay === "absolute" ? page.rc.volumeAbs : page.rc.volumeDb) || 0 : 0 },
          valueText: function() { return page.answers && page.rc.volumeText ? String(page.rc.volumeText) : "\u2013" } },
        { kind: "slider", tab: 1, icon: "mic", label: "Dialog level", help: "Louder voices without louder everything.", min: -12, max: 12, step: 0.5,
          enabled: function() { return page.ctl && page.lev.dialog !== undefined }, num: function() { return page.lev.dialog || 0 },
          valueText: function() { return page.db(page.lev.dialog, true) }, act: function(v) { page.set("dialog", v) } },
        { kind: "slider", tab: 1, icon: "speaker", label: "Subwoofer level", help: "More or less bass from the sub only.", min: -12, max: 12, step: 0.5,
          enabled: function() { return page.ctl && page.lev.subwoofer !== undefined }, num: function() { return page.lev.subwoofer || 0 },
          valueText: function() { return page.db(page.lev.subwoofer, true) }, act: function(v) { page.set("subwoofer", v) } },
        { kind: "slider", tab: 1, icon: "sliders-vertical", label: "Bass", help: "Tone control. Off in Direct and Pure Direct.", min: -6, max: 6, step: 1,
          enabled: function() { return page.ctl && page.lev.bass !== undefined }, num: function() { return page.lev.bass || 0 },
          valueText: function() { return page.db(page.lev.bass, false) }, act: function(v) { page.set("bass", v) } },
        { kind: "slider", tab: 1, icon: "sliders-vertical", label: "Treble", help: "Tone control. Off in Direct and Pure Direct.", min: -6, max: 6, step: 1,
          enabled: function() { return page.ctl && page.lev.treble !== undefined }, num: function() { return page.lev.treble || 0 },
          valueText: function() { return page.db(page.lev.treble, false) }, act: function(v) { page.set("treble", v) } },
        { kind: "toggle", tab: 1, icon: "theater", label: "Cinema EQ", help: "Softens bright film soundtracks mixed for a big room.",
          enabled: function() { return page.ctl && page.rc.cinemaEq !== undefined }, on: function() { return !!page.rc.cinemaEq }, act: function(v) { page.set("cinemaEq", v) } },

        { kind: "seg", tab: 2, icon: "audio-lines", label: "MultEQ", help: "Audyssey is the room correction the receiver measured. These switch its parts on and off; re-measuring is done on the receiver.", options: ["Reference", "L/R bypass", "Flat", "Off"],
          enabled: function() { return page.ctl && page.aud.multEq !== undefined }, index: function() { return page.indexIn(["reference", "bypass", "flat", "off"], page.aud.multEq) },
          act: function(i) { page.set("multEq", ["reference", "bypass", "flat", "off"][i]) } },
        { kind: "toggle", tab: 2, icon: "activity", label: "Dynamic EQ", help: "Keeps bass and surround full at low volume.",
          enabled: function() { return page.ctl && page.aud.dynEq !== undefined }, on: function() { return !!page.aud.dynEq }, act: function(v) { page.set("dynEq", v) } },
        { kind: "seg", tab: 2, icon: "gauge", label: "Reference level offset", help: "For Dynamic EQ: 0 for films, 5\u201315 for TV and music mixed quieter.", options: ["0 dB", "5", "10", "15"],
          enabled: function() { return page.ctl && page.aud.refOffset !== undefined }, index: function() { return page.indexIn([0, 5, 10, 15], page.aud.refOffset) },
          act: function(i) { page.set("refOffset", [0, 5, 10, 15][i]) } },
        { kind: "seg", tab: 2, icon: "volume-1", label: "Dynamic Volume", help: "Evens out loud adverts and quiet dialogue.", options: ["Off", "Light", "Medium", "Heavy"],
          enabled: function() { return page.ctl && page.aud.dynVol !== undefined }, index: function() { return page.indexIn(["off", "light", "medium", "heavy"], page.aud.dynVol) },
          act: function(i) { page.set("dynVol", ["off", "light", "medium", "heavy"][i]) } },

        R.head(function() { return "Zone 2" + (page.z2.name ? " \u00b7 " + page.z2.name : "") }, 3),
        { kind: "toggle", tab: 3, icon: "power", label: "Zone 2 power", help: "The second pair of speakers, on their own volume and input.",
          enabled: function() { return page.ctl && page.z2.power !== undefined }, on: function() { return !!page.z2.power }, act: function(v) { page.set("zone2Power", v) } },
        { kind: "slider", tab: 3, icon: "volume-2", label: "Zone 2 volume", detail: function() { return page.z2.power ? "" : "Zone 2 is off" }, min: function() { return page.z2.volumeDisplay === "absolute" ? 0 : -80 },
          max: function() { return page.z2.volumeDisplay === "absolute" ? 98 : 18 }, step: 0.5,
          enabled: function() { return page.ctl && !!page.z2.power && page.z2.volumeDb !== undefined }, num: function() { return page.z2.volumeDb || 0 },
          valueText: function() { return page.z2.volumeText ? String(page.z2.volumeText) : "\u2013" }, act: function(v) { page.set("zone2Volume", v) } },
        { kind: "stepper", tab: 3, icon: "cable", label: "Zone 2 input", help: "Follows the main zone unless set here.",
          enabled: function() { return page.ctl && !!page.z2.power && (page.rc.inputs || []).length > 0 }, options: function() { return page.rc.inputs || [] },
          index: function() { return page.indexIn(page.rc.inputs, page.z2.input) }, act: function(i) { page.set("zone2Input", page.rc.inputs[i]) } },
        R.head("Power", 3),
        { kind: "seg", tab: 3, icon: "leaf", label: "ECO mode", help: "Less power at low volume. Auto decides by level; On always.", options: ["On", "Auto", "Off"],
          enabled: function() { return page.ctl && page.rc.eco !== undefined }, index: function() { return page.indexIn(["on", "auto", "off"], page.rc.eco) },
          act: function(i) { page.set("eco", ["on", "auto", "off"][i]) } }
    ]
}
