import QtQuick 2.4
import "../kit"

// The volume and mute display (docs/mocks/11-volume-osd): the only one on the TV. LG's own, the speaker with a + and a -, the level bar and
// the mute bubble, is switched off in the shadow module (tools/volume_patch.py). This is a slim pane at the right edge, where LG's was, over
// Home, over every app and every input. It takes no key and no focus. It lives in LumaOverlay, the layer that is always up above whatever
// the compositor shows, so a volume key answers the same while Plex, HDMI 2 or Live TV is in front as it does on Home.
//
// Whose volume it shows. The keys change one device, and which one depends on where the sound goes:
//   the receiver  the sound leaves over HDMI ARC (or optical) and the Denon is on and answering: its own level in its own units (the helper's
//                 `receiver` topic), not webOS's counter, which is a step count that drifts (PLAN, "Receiver: volume and controls"). The topic
//                 is the Denon's own push, so a change made with the Denon's remote shows here too.
//   the TV        the TV's speakers, headphones or Bluetooth are the output, or the receiver is off, unreachable or switched off in Settings:
//                 the TV's own volume, the one LG's popup read (kit/TvVolume, the audio service).
// A reading that is not in date is never shown (kit/ReceiverGate). When the receiver may answer and has not yet (a key after minutes of
// quiet) the pane waits `decideMs` for it instead of showing the TV's counter and then changing to the receiver's level.
//
// When it opens: the audio service reports a change (a volume key, a mute, Home Assistant, an app: the moments LG's popup opened for), or
// the receiver reports a new level or mute (its own remote too). It goes `holdMs` after the last of either. While the sound is muted a small
// mark stays where the pane was and moves a little every ten seconds, as LG's mute bubble did: a still mark is what an OLED panel must not hold.
//
// `enabledHere` is the host's switch. `overVideo` says the pane is over an app's picture (the overlay): its glass then lets the picture through
// down to Lu.readingAlpha, the least that keeps the ink at the TV's distance contrast against the worst frame.
Item {
    id: osd
    anchors.fill: parent
    property bool enabledHere: true
    property bool overVideo: false
    property int holdMs: 2500
    property int staleMs: 6000
    property int decideMs: 250                    // how long a key waits for a receiver that may answer before the TV's own volume is shown
    property int hopMs: 10000                     // LG's mute bubble moves this often
    property int graceMs: 1500                    // a mute that was in date stays shown this long after its reading ages out and before the next one

    // The age gate every receiver surface shares (kit/ReceiverGate). It asks the helper to confirm an aged reading when a key is pressed
    // (`cue`) and, while a mute is on screen, each time the reading ages out: the helper moves a reading's stamp quietly, and a mute that is
    // shown for hours must go on being true.
    ReceiverGate { id: gate; staleMs: osd.staleMs; confirmAged: osd.mutedRx; onReading: osd.read(doc) }
    readonly property var rc: gate.rc
    readonly property bool fresh: gate.fresh
    readonly property bool stale: gate.stale

    // ---- whose volume the keys change
    readonly property bool tvOutput: TvVolume.known && !TvVolume.arc          // the TV's own speakers, headphones, Bluetooth: the receiver is not in the way
    readonly property bool receiverOn: fresh && rc.power !== "standby"
    readonly property bool receiverMay: Helper.up && !!rc && rc.enabled !== false && rc.reachable !== false && rc.power !== "standby"
    property bool deciding: false
    // "receiver", "tv", "wait" (the receiver may answer and has not yet) or "none" (the TV has not said where the sound goes and the receiver does not answer)
    readonly property string source: tvOutput ? "tv" : receiverOn ? "receiver" : receiverMay && deciding ? "wait" : TvVolume.known ? "tv" : "none"
    readonly property bool viaReceiver: source === "receiver"
    readonly property bool viaTv: source === "tv"

    // ---- what is read
    readonly property string rawLevel: viaReceiver ? (rc.volumeText ? String(rc.volumeText) : "") : viaTv ? String(TvVolume.volume) : ""
    readonly property bool hasUnit: /\s*dB$/.test(rawLevel)
    readonly property string level: hasUnit ? rawLevel.replace(/\s*dB$/, "") : rawLevel
    readonly property real fraction: viaReceiver ? (rc.volumeAbs !== undefined && rc.volumeAbs !== null ? Math.max(0, Math.min(1, rc.volumeAbs / 98)) : 0)
                                     : viaTv ? Math.max(0, Math.min(1, TvVolume.volume / TvVolume.volumeMax)) : 0
    readonly property bool muted: viaReceiver ? !!rc.mute : viaTv ? TvVolume.muted : false
    // the line under the number: the unit when the receiver counts in decibels, else where the sound goes (the receiver's input, or the TV's output)
    readonly property string caption: viaReceiver ? (hasUnit ? "dB" : (rc.input ? String(rc.input) : "")) : viaTv ? TvVolume.outputLabel : ""
    property string lastLevel: ""                 // the level before it was muted, and whose it was
    property string lastSource: ""
    readonly property string was: lastSource === source && lastLevel !== "" ? lastLevel : (rawLevel !== "" ? rawLevel : "")

    property bool pill: false
    readonly property real paneAlpha: overVideo ? Lu.readingAlpha : 1     // how much of the glass is painted
    readonly property int markPlace: mark.place
    readonly property bool offline: pill && source === "none"
    readonly property bool paneUp: pill && source !== "wait"

    // The mute that stays shown: the TV's, when the TV's volume is what is read; else the receiver's last in-date reading, kept a moment
    // across the gap between one reading and the next.
    property bool mutedRx: false
    readonly property bool mutedShown: viaTv ? TvVolume.muted : mutedRx
    readonly property bool markUp: enabledHere && mutedShown && !pill
    // something of this item is on screen or fading: the layer that holds it (LumaOverlay) has to be drawn for it
    readonly property bool live: paneUp || markUp || card.opacity > 0 || markHost.opacity > 0

    function show() {
        if (!enabledHere) return
        pill = true
        hide.restart()
    }
    // The audio service reported a change: show the pane now. If the receiver may be the one that changed, ask the helper to read it now.
    function cue() {
        if (!enabledHere) return
        if (!tvOutput && receiverMay) {
            deciding = !receiverOn
            if (deciding) decide.restart()
            var now = Date.now()
            if (now - nudgedAt > 300) { nudgedAt = now; Helper.post("/v1/receiver/nudge", {}) }
            if (gate.stale) gate.confirm()
        }
        if (viaTv && !muted) { lastLevel = String(TvVolume.volume); lastSource = "tv" }
        show()
    }
    property double nudgedAt: 0
    Timer { id: hide; interval: osd.holdMs; onTriggered: osd.pill = false }
    Timer { id: decide; interval: osd.decideMs; onTriggered: osd.deciding = false }
    Connections { target: TvVolume; onCue: osd.cue() }

    // A reading that is in date: a change of level or of mute, against the last one, shows the pane (while it is the receiver that is read).
    function read(r) {
        deciding = false
        var nextKey = r.volumeText + "|" + r.mute
        if (levelKey !== nextKey && levelKey !== "" && !tvOutput) show()
        if (!r.mute && r.volumeText) { lastLevel = String(r.volumeText); lastSource = "receiver" }
        levelKey = nextKey
        mutedRx = !!r.mute
        graceT.stop()
    }
    property string levelKey: ""
    onFreshChanged: if (!fresh && mutedRx) graceT.restart()
    Timer { id: graceT; interval: osd.graceMs; onTriggered: if (!osd.fresh) osd.mutedRx = false }

    // ---- the pane: as wide and as tall as its words need, so it holds at 200 % text as well, and never slimmer or lower than the design
    readonly property int edgeX: 64               // from the right edge, as Home's panes and the banners sit (layout.json safe.x)
    readonly property int pad: Lu.space(7)        // 32: text inside starts 96 from the screen's edge, the TV's 5 % safe area
    readonly property int gap: Lu.space(6)        // between the words, the level and the speaker
    readonly property int trackLen: 216
    // a long input name is cut at this width (secondary words), not allowed to widen the pane
    readonly property real captionLimit: Lu.type("caption").size * 5.5
    readonly property bool showNum: !muted && source !== "none"
    readonly property bool showWord: muted && source !== "none"
    readonly property bool showCap: source !== "none"
    readonly property real wordsW: Math.max(showNum ? numText.implicitWidth : 0, showWord ? wordText.implicitWidth : 0,
                                            showCap ? Math.min(capText.implicitWidth, captionLimit) : 0)
    readonly property real topH: Math.max(numText.implicitHeight, wordText.implicitHeight)
    LuPane {
        id: card
        width: Math.max(176, Math.ceil(osd.wordsW) + 2 * osd.pad)
        height: osd.pad + osd.topH + capText.implicitHeight + osd.gap + osd.trackLen + osd.gap + Lu.target + osd.pad
        x: osd.width - osd.edgeX - width
        y: Math.round((osd.height - height) / 2)
        cardRadius: Lu.radius("card")
        fillAlpha: osd.paneAlpha
        opacity: osd.paneUp ? 1 : 0
        visible: opacity > 0
        Behavior on opacity { enabled: !Lu.reduced; NumberAnimation { duration: osd.paneUp ? Lu.cardMs : Lu.restMs } }

        // the level, or the word that replaces it; the line under it says what it counts
        LuText {
            id: numText
            visible: osd.showNum
            x: osd.pad; y: osd.pad
            width: card.width - 2 * osd.pad
            horizontalAlignment: Text.AlignHCenter
            role: "title"
            text: osd.level !== "" ? osd.level : "\u2013"
        }
        LuText {
            id: wordText
            visible: osd.showWord
            x: osd.pad; y: osd.pad + Math.round((osd.topH - implicitHeight) / 2)
            width: card.width - 2 * osd.pad
            horizontalAlignment: Text.AlignHCenter
            role: "label"
            font.weight: Font.DemiBold
            text: "Muted"
        }
        LuText {
            id: capText
            visible: osd.showCap
            x: osd.pad; y: osd.pad + osd.topH
            width: card.width - 2 * osd.pad
            horizontalAlignment: Text.AlignHCenter
            role: "caption"
            text: osd.muted ? (osd.was !== "" ? "was " + osd.was : "") : osd.caption
            elide: Text.ElideRight
        }
        // nothing to read: say so
        LuText {
            visible: osd.source === "none"
            x: osd.pad; y: osd.pad
            width: card.width - 2 * osd.pad
            horizontalAlignment: Text.AlignHCenter
            role: "caption"
            wrapMode: Text.WordWrap
            text: "Receiver not answering"
        }

        // the level, filling from the bottom; muted, the same height dimmed
        Rectangle {
            visible: osd.source !== "none"
            x: Math.round((card.width - width) / 2)
            y: osd.pad + osd.topH + capText.implicitHeight + osd.gap
            width: Lu.space(3)
            height: osd.trackLen
            radius: width / 2
            color: Lu.trackOff
            antialiasing: true
            Rectangle {
                width: parent.width
                height: Math.round(parent.height * osd.fraction)
                anchors.bottom: parent.bottom
                radius: width / 2
                color: osd.muted ? Lu.ink3 : Lu.ink
                antialiasing: true
                Behavior on height { enabled: !Lu.reduced; NumberAnimation { duration: Lu.focusMs } }
            }
        }
        LuTile {
            x: Math.round((card.width - width) / 2)
            y: card.height - osd.pad - height
            size: Lu.target
            accent: !osd.muted && osd.source !== "none"
            icon: osd.source === "none" ? "unplug" : osd.muted ? "volume-x" : osd.fraction < 0.5 ? "volume-1" : "volume-2"
        }
    }

    // ---- the mark that stays while the sound is muted
    Item {
        id: markHost
        anchors.fill: parent
        opacity: osd.markUp ? 1 : 0
        visible: opacity > 0
        Behavior on opacity { enabled: !Lu.reduced; NumberAnimation { duration: Lu.restMs } }
        LuPane {
            id: mark
            readonly property int step: 94        // LG's bubble moves this far, between three places
            property int place: 0                 // -1, 0 or 1 steps from the middle
            property int way: 1
            width: Math.max(128, Math.ceil(markText.implicitWidth) + 2 * Lu.space(3))
            height: Lu.space(5) + 48 + Lu.space(1) + markText.implicitHeight + Lu.space(5)
            x: osd.width - osd.edgeX - width
            y: Math.round((osd.height - height) / 2) + place * step
            cardRadius: Lu.radius("card")
            fillAlpha: osd.paneAlpha
            LuIcon { x: Math.round((parent.width - width) / 2); y: Lu.space(5); size: 48; name: "volume-x" }
            LuText {
                id: markText
                x: Lu.space(3); y: Lu.space(5) + 48 + Lu.space(1)
                width: parent.width - 2 * Lu.space(3)
                horizontalAlignment: Text.AlignHCenter
                role: "caption"
                text: "Muted"
            }
            // out, to the next place (the glass samples the backdrop where the pane is, so it is told where that is now), and back in
            function hop() {
                if (place + way > 1 || place + way < -1) way = -way
                place += way
                locate()
            }
            SequentialAnimation {
                id: hopping
                NumberAnimation { target: mark; property: "opacity"; to: 0; duration: Lu.reduced ? 0 : Lu.restMs }
                ScriptAction { script: mark.hop() }
                PauseAnimation { duration: Lu.reduced ? 0 : 400 }
                NumberAnimation { target: mark; property: "opacity"; to: 1; duration: Lu.reduced ? 0 : Lu.restMs }
            }
        }
        Timer { interval: osd.hopMs; repeat: true; running: osd.markUp; onTriggered: hopping.restart() }
    }
}
