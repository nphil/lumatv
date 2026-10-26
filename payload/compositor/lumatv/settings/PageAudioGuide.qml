import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Audio Guidance (LG: General › Accessibility › Audio Guidance): the menus read aloud, and the voice's speed, volume and pitch.
// `option.audioGuidance` and its three keys, the TV's own lists. The voice rows answer only while the guidance is on, as in LG's
// screen. LG's high-quality voice pack is a download from LG's server and stays out (docs/research/settings-coverage.md).
PageList {
    id: page
    title: "Audio Guidance"
    back: "System"
    readonly property bool on: String(Sio.get("option.audioGuidance")) === "on"

    function refresh() { Sio.read("option", ["audioGuidance", "audioGuidanceSpeed", "audioGuidanceVolume", "audioGuidancePitch"]) }
    Component.onCompleted: refresh()
    onOpened: refresh()

    function voice(ds, icon, label) {
        var row = C.choice(Sio, { ds: ds, icon: icon, label: label, names: ds === "option.audioGuidanceSpeed" ? "slowToFast" : "lowToHigh", page: page })
        return R.over(row, { enabled: function() { return page.on && Sio.usable(ds) }, detail: function() { return page.on ? "" : "Turn it on first" } })
    }

    rows: [
        R.toggle(Sio, { ds: "option.audioGuidance", icon: "audio-lines", label: "Audio Guidance" }),
        voice("option.audioGuidanceSpeed", "gauge", "Speed"),
        voice("option.audioGuidanceVolume", "volume-2", "Volume"),
        voice("option.audioGuidancePitch", "audio-waveform", "Pitch")
    ]
}
