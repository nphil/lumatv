import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "SoundOut.js" as S
import "."

// Equalizer (LG: Sound › Advanced Settings): five bands for the TV's own speakers, 100 Hz to 10 kHz, in the Standard sound mode. LG keeps
// the five levels in one key, `sound.userEqualizerValues` (a list of five numbers, its limits in the key's description), and marks the mode
// modified (`soundModeModified`) in the same write, which is what its own panel sends. The Reset of Sound › Advanced puts them back.
PageList {
    id: page
    title: "Equalizer"
    back: "Sound"

    readonly property var bands: ["100 Hz", "300 Hz", "1 kHz", "3 kHz", "10 kHz"]
    readonly property var limits: { var d = Sio.descs["sound.userEqualizerValues"]; return d && d.values && d.values.range && d.values.range.max !== undefined ? d.values.range : null }
    readonly property var levels: {
        var v = Sio.get("sound.userEqualizerValues"), out = []
        for (var i = 0; i < 5; i++) { var n = v && v.length > i ? Number(v[i]) : 0; out.push(isNaN(n) ? 0 : n) }
        return out
    }
    readonly property string output: String(Sio.get("sound.soundOutput"))
    readonly property bool allowed: String(Sio.get("sound.soundMode")) === "standard" && S.equalizer(output)
                                    && Sio.usable("sound.userEqualizer") && Sio.has("sound.userEqualizerValues")

    function refresh() { Sio.read("sound", ["userEqualizer", "userEqualizerValues", "soundOutput", "soundMode"]) }
    Component.onCompleted: refresh()
    onOpened: refresh()

    function setBand(i, v) {
        var a = levels.slice()
        a[i] = v
        Sio.writeMany("sound", { userEqualizerValues: a, soundModeModified: "true" })
    }
    function signed(n) { return n > 0 ? "+" + n : String(n) }
    function band(i) {
        return { kind: "slider", tab: 0, id: "band" + i, icon: "sliders-vertical", label: bands[i],
                 help: i === 0 ? "Lower bands are the bass, higher ones the treble. 0 leaves a band as it is." : undefined,
                 detail: function() { return !Sio.has("sound.userEqualizerValues") ? "TV not answering" : page.allowed ? "" : "Standard mode and TV speakers" },
                 enabled: function() { return page.allowed },
                 min: function() { return page.limits ? page.limits.min : -10 }, max: function() { return page.limits ? page.limits.max : 10 },
                 step: function() { return page.limits && page.limits.interval ? page.limits.interval : 1 },
                 num: function() { return page.levels[i] },
                 valueText: function() { return Sio.has("sound.userEqualizerValues") ? page.signed(page.levels[i]) : "\u2013" },
                 act: function(v) { page.setBand(i, v) } }
    }
    rows: [band(0), band(1), band(2), band(3), band(4)]
}
