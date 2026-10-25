import QtQuick 2.4
import "../kit"

// The volume and mute OSD (docs/mocks/11-volume-osd and its muted and offline variants), fed by the receiver
// and not by webOS's counter: with the sound on the Denon over eARC webOS's own volume is a number that drifts
// (the receiver can be turned from its own remote and tells the TV nothing), so the level shown is the one the
// receiver reports, in the receiver's own units. It draws the helper's `receiver` topic and nothing else.
//
// What it shows, and when:
//   a change of volume or of mute that the receiver reports      the pill for `holdMs`
//   a volume key (`nudge()`, from the host's key handler)         the pill while the new reading is on its way
//   a receiver that does not answer                               "Receiver not answering" and that the keys still reach it
// A reading the helper says is not fresh is not shown: the helper hides every value 6 s after the last good read,
// and this item hides its own copy after the same 6 s without a new `updatedAt`, so a helper that has stopped
// talking can never leave a number up that is no longer true.
//
// `shown` is the host's cue that the pill may be on screen at all (Home in front, or the compositor's volume
// view when this is mounted there); the item decides by itself when it has something to say.
Item {
    id: osd
    anchors.fill: parent
    property bool enabledHere: true
    property int holdMs: 2500
    property int staleMs: 6000
    readonly property var rc: Helper.receiver
    readonly property bool fresh: !!(rc && rc.reachable && rc.fresh) && !stale
    readonly property bool answering: !!(rc && rc.reachable !== false && rc.fresh !== false && rc.enabled !== false)
    property double lastReading: 0
    property bool stale: true
    property bool pill: false
    property bool waiting: false                   // a volume key was pressed and the answer has not come yet
    readonly property bool muted: fresh && !!rc.mute
    readonly property string level: fresh && rc.volumeText ? String(rc.volumeText) : ""
    readonly property real fraction: fresh && rc.volumeAbs !== undefined ? Math.max(0, Math.min(1, rc.volumeAbs / 98)) : 0
    property string wasText: ""                    // the level before it was muted
    readonly property string sub: fresh ? (rc.input ? String(rc.input) : "") + (rc.soundMode ? (rc.input ? " \u00b7 " : "") + rc.soundMode : "") : ""
    readonly property bool offline: pill && !fresh && (waiting || stale || (rc && rc.reachable === false))

    function show() {
        if (!enabledHere) return
        pill = true
        hide.restart()
    }
    // A volume key was pressed: ask the helper to read the receiver now and fast, and show the pill.
    function nudge() {
        waiting = true
        Helper.post("/v1/receiver/nudge", {})
        show()
        waitEnd.restart()
    }
    Timer { id: hide; interval: osd.holdMs; onTriggered: { osd.pill = false; osd.waiting = false } }
    Timer { id: waitEnd; interval: 1200; onTriggered: osd.waiting = false }
    // Six seconds without a new reading and the number is not shown.
    Timer { id: staleTimer; interval: osd.staleMs; onTriggered: osd.stale = true }

    function readReceiver() {
        var r = rc
        var at = r ? Number(r.updatedAt) : 0
        var remaining = staleMs - (Date.now() - at)
        staleTimer.stop()
        stale = !(r && r.fresh && r.reachable && isFinite(at) && at > 0 && remaining > 0 && remaining <= staleMs)
        if (stale) return
        lastReading = at
        staleTimer.interval = Math.max(1, Math.ceil(remaining))
        staleTimer.start()
        var nextKey = r.volumeText + "|" + r.mute
        if (levelKey !== nextKey && levelKey !== "") show()
        keepWas(r)
        levelKey = nextKey
    }
    onRcChanged: readReceiver()
    Component.onCompleted: readReceiver()
    property string levelKey: ""
    property string lastLevel: ""
    function keepWas(r) { if (!r.mute && r.volumeText) lastLevel = String(r.volumeText) }
    onMutedChanged: if (muted) wasText = lastLevel

    LuPane {
        id: card
        visible: osd.pill
        x: 64
        y: 1080 - 64 - height
        width: 520
        height: 96
        cardRadius: Lu.radius("card")
        opacity: osd.pill ? 1 : 0
        Behavior on opacity { enabled: !Lu.reduced; NumberAnimation { duration: Lu.cardMs } }

        LuTile { id: glyph; x: Lu.space(5); anchors.verticalCenter: parent.verticalCenter; size: 56; icon: osd.offline ? "unplug" : osd.muted ? "volume-x" : "volume-2" }

        // the level, in the receiver's units
        Column {
            visible: !osd.offline && !osd.muted
            x: glyph.x + glyph.width + Lu.space(5)
            width: parent.width - x - Lu.space(5)
            anchors.verticalCenter: parent.verticalCenter
            spacing: Lu.space(2)
            Row {
                spacing: Lu.space(3)
                LuText { id: bigLevel; role: "display"; font.pixelSize: 44; text: osd.level !== "" ? osd.level : "\u2013"; color: Lu.ink }
                Item { width: 1; height: 1 }
                LuText { anchors.baseline: bigLevel.baseline; role: "caption"; color: Lu.ink2; text: osd.sub; elide: Text.ElideRight }
            }
            Rectangle {
                width: parent.width; height: Lu.space(1); radius: height / 2; color: Lu.trackOff
                Rectangle { width: parent.width * osd.fraction; height: parent.height; radius: height / 2; color: Lu.accent
                            Behavior on width { enabled: !Lu.reduced; NumberAnimation { duration: Lu.focusMs } } }
            }
        }
        // muted: the word, what it was, a full empty bar
        Column {
            visible: osd.muted && !osd.offline
            x: glyph.x + glyph.width + Lu.space(5)
            width: parent.width - x - Lu.space(5)
            anchors.verticalCenter: parent.verticalCenter
            spacing: Lu.space(2)
            Row {
                spacing: Lu.space(3)
                LuText { id: mutedWord; role: "title"; text: "Muted" }
                LuText { anchors.baseline: mutedWord.baseline; role: "caption"; color: Lu.ink2; text: (osd.rc && osd.rc.input ? osd.rc.input + " \u00b7 " : "") + (osd.wasText !== "" ? "was " + osd.wasText : "") }
            }
            Rectangle { width: parent.width; height: Lu.space(1); radius: height / 2; color: Lu.trackOff
                        Rectangle { width: parent.width * 0.35; height: parent.height; radius: height / 2; color: Lu.ink3 } }
        }
        // the receiver does not answer: say so, and that the keys still reach it
        Column {
            visible: osd.offline
            x: glyph.x + glyph.width + Lu.space(5)
            width: parent.width - x - Lu.space(5)
            anchors.verticalCenter: parent.verticalCenter
            LuText { width: parent.width; role: "label"; text: "Receiver not answering"; elide: Text.ElideRight }
            LuText { width: parent.width; role: "caption"; color: Lu.ink2; text: "Volume keys still reach it"; elide: Text.ElideRight }
        }
    }
}
