import QtQuick 2.4
import "../kit"

// The capture indicator of the shell's system layer (docs/research/capture-result.md): a red dot in the top left corner for as long as the
// compositor says a recording runs, and a short note when a capture ends ("Screenshot saved", "Recording saved"). It is the compositor's rule
// that no recording runs without this: the recording starts only once the shell has said the dot is up (`dotUp`, in the state the shell
// reports), and ends if it stops saying so. Under LG's compositor it is never instantiated (there is no capture there).
//
// The dot is a filled circle in the palette's live red inside a dark ring, so it reads over any picture (the ring against a white frame, the
// red against the ring) and the circle's shape says "recording" without the colour. It stands still for people who asked for less motion and
// dims every other second (a step, not a fade: one frame a second) for everyone else, so a frozen picture is not mistaken for a stopped
// recording, and a bright dot is not left burning one place on the panel. Nothing here takes a key, and a hidden mark costs nothing.
Item {
    id: mark
    anchors.fill: parent

    // the compositor's `capture` topic: { recording, stage, since, n, last: { kind, name, ok, note, at } }
    property var topic: ({})
    readonly property bool recording: topic.recording === true
    // a capture ended: the words, for a moment
    property string note: ""
    property bool bright: true
    // what this shell had seen when it began: captures from before it started are not news
    property int seenN: -1
    property double seenAt: -1
    // the system layer has something of this to show: the compositor draws the layer only while one of its surfaces does
    readonly property bool live: recording || note !== ""
    // the dot is on the screen: what the shell reports to the compositor, which will not record without it
    readonly property bool dotUp: recording && dot.visible

    onTopicChanged: take(topic)
    function take(t) {
        if (!t || t.n === undefined) return
        var last = t.last || null
        var at = last && last.at !== undefined ? last.at : 0
        if (seenN < 0) { seenN = t.n; seenAt = at; return }
        var words = ""
        if (t.n > seenN) words = "Screenshot saved"
        else if (last && at !== seenAt) {
            if (last.kind === "recording") words = last.ok ? "Recording saved" : "Recording stopped"
            else if (last.kind === "shot" && last.ok === false) words = "Screenshot failed"
        }
        seenN = t.n
        seenAt = at
        if (words !== "") { note = words; noteTimer.restart() }
    }
    Timer { id: noteTimer; interval: 2600; onTriggered: mark.note = "" }
    Timer {
        interval: 1000; repeat: true
        running: mark.recording && !Lu.reduced
        onTriggered: mark.bright = !mark.bright
        onRunningChanged: if (!running) mark.bright = true
    }

    Rectangle {
        id: ring
        visible: mark.recording
        x: Lu.space(6); y: Lu.space(6)
        width: 36; height: 36; radius: 18
        color: "#E6000000"
        Rectangle {
            id: dot
            anchors.centerIn: parent
            width: 22; height: 22; radius: 11
            color: Lu.live
            opacity: mark.bright ? 1 : 0.45
        }
    }

    Rectangle {
        id: pill
        visible: mark.note !== ""
        x: mark.recording ? ring.x + ring.width + Lu.space(3) : Lu.space(6); y: Lu.space(6)
        width: words.width + 2 * Lu.space(5)
        height: words.height + 2 * Lu.space(2)
        radius: Lu.radius("pill"); color: Lu.sheet; border.width: 1; border.color: Lu.edge
        LuText { id: words; anchors.centerIn: parent; role: "label"; text: mark.note }
    }
}
