import QtQuick 2.4
import "../kit"
import "../kit/AudioFormat.js" as AudioFormat

// The volume and mute display (docs/mocks/11-volume-osd): the only one on the TV. LG's own, the speaker with a + and a -, the level bar and
// the mute bubble, is switched off in the shadow module (tools/volume_patch.py). This is a slim bar at the bottom centre, over Home, over
// every app and every input: the level as a number and a thin bar and, while the receiver says what it is playing, the format in small words
// at the end of it. It takes no key and no focus. It lives in LumaOverlay, the layer that is always up above whatever the compositor shows,
// so a volume key answers the same while Plex, HDMI 2 or Live TV is in front as it does on Home.
// Little copy: the number and the bar, and the format when it is known. Muted, the number gives way to a slashed speaker; no word is shown.
//
// Whose volume it shows. The keys change one device, and which one depends on where the sound goes:
//   the receiver  the sound leaves over HDMI ARC (or optical) and the Denon is on and answering: its own level in its own units (the helper's
//                 `receiver` topic), not webOS's counter, which is a step count that drifts (PLAN, "Receiver: volume and controls"). The topic
//                 is the Denon's own push, so a change made with the Denon's remote shows here too.
//   the TV        the TV's speakers, headphones or Bluetooth are the output, or the receiver is off, unreachable or switched off in Settings:
//                 the TV's own volume, the one LG's popup read (kit/TvVolume, the audio service).
// A reading that is not in date is never shown (kit/ReceiverGate). When the receiver may answer and has not yet (a key after minutes of
// quiet) the bar waits `decideMs` for it instead of showing the TV's counter and then changing to the receiver's level.
//
// The key answers before the receiver does. The TV reports a volume key 44 to 63 ms after the press, with its direction (`cause`), and the
// Denon's own reading follows some 0.4 s later. The bar appears and moves its fill on the key (one receiver step, 0.5, per key and per repeat of
// a held key), keeps what the keys have made it say for `settleMs` after the last one, then takes the reading: no stale reading in between
// moves it back.
//
// When it opens: the audio service reports a change (a volume key, a mute, Home Assistant, an app: the moments LG's popup opened for), or
// the receiver reports a new level or mute (its own remote too). It goes `holdMs` after the last of either. While the sound is muted a small
// mark stays in the top corners and moves every ten seconds, as LG's mute bubble did: a still mark is what an OLED panel must not hold.
// Never in the lower third (subtitles) or the middle of the picture.
//
// `enabledHere` is the host's switch. `overVideo` says the bar is over an app's picture (the overlay): its glass then lets the picture through
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
    property int settleMs: 1500                   // what the keys made the receiver say stays this long after the last key, then the reading is taken (its push comes 0.4 to 1 s after a key)

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

    // ---- the key answers before the receiver does
    property bool keyed: false                    // the keys have moved the receiver's level or mute since its last reading was trusted
    property real optAbs: -1                      // the level the keys made it (0 to 98, the receiver's own steps), or -1
    property int optMute: -1                      // the mute the keys made it, 0 or 1, or -1
    readonly property real receiverStep: 0.5      // one volume key moves the Denon this far on the set (42.5 to 43 to 42.5, read 2026-10-25)
    Timer { id: settleT; interval: osd.settleMs; onTriggered: osd.keyed = false }
    function readAbs() { return rc && rc.volumeAbs !== undefined && rc.volumeAbs !== null ? rc.volumeAbs : -1 }
    function key() {
        var cause = String(TvVolume.cause)
        var up = /up/i.test(cause), down = /down/i.test(cause)
        var base = keyed && optAbs >= 0 ? optAbs : readAbs()
        if (base >= 0) optAbs = down ? Math.max(0, base - receiverStep) : up ? Math.min(98, base + receiverStep) : base
        // the Denon cancels its mute on a volume key (read on the set: muted, Volume+, unmuted); a report with no direction keeps what is believed
        var was = keyed && optMute >= 0 ? optMute === 1 : !!(rc && rc.mute)
        optMute = up || down ? 0 : (was ? 1 : 0)
        keyed = true
        settleT.restart()
    }
    // The mute key's report has no direction (the TV's muteStatus alone, read on the set) but the TV's own mute flag moves with it: when that flag
    // differs from what the receiver last said, the key was the mute key and the receiver follows in 0.5 to 1 s, so the bar shows it now.
    function muteKey() {
        if (!viaReceiver || !rc || TvVolume.muted === !!rc.mute) return
        optMute = TvVolume.muted ? 1 : 0
        keyed = true
        settleT.restart()
    }
    function num(x) { return String(Math.round(x * 2) / 2) }

    // ---- what is read
    readonly property bool useKey: keyed && viaReceiver && optAbs >= 0
    readonly property real abs: viaReceiver ? (useKey ? optAbs : readAbs()) : -1
    readonly property bool relative: viaReceiver && rc.volumeDisplay === "relative"
    readonly property string rawLevel: viaReceiver ? (useKey ? (relative ? num(abs - 80) : num(abs)) : (rc.volumeText ? String(rc.volumeText) : ""))
                                       : viaTv ? String(TvVolume.volume) : ""
    readonly property string level: rawLevel.replace(/\s*dB$/, "")       // the decibel scale's unit is left out: the minus sign says it
    readonly property real fraction: viaReceiver ? (abs >= 0 ? Math.max(0, Math.min(1, abs / 98)) : 0)
                                     : viaTv ? Math.max(0, Math.min(1, TvVolume.volume / TvVolume.volumeMax)) : 0
    readonly property bool muted: viaReceiver ? (keyed && optMute >= 0 ? optMute === 1 : !!rc.mute) : viaTv ? TvVolume.muted : false
    // what the receiver is playing, in short words (kit/AudioFormat.js); nothing while it does not say or the TV's own sound is what is read
    readonly property string format: viaReceiver && !muted ? AudioFormat.describe(rc.soundMode, rc.signal) : ""

    property bool pill: false
    readonly property real paneAlpha: overVideo ? Lu.readingAlpha : 1     // how much of the glass is painted
    readonly property int markPlace: mark.place
    readonly property int markX: mark.x
    readonly property int markY: mark.y
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
    // Where the time goes on the set: the log line below, at the key's report, against `onVolumeUpdated` (LG's own line for the same report) and the
    // key injector's timestamp, gives the key to the bar; the bar itself is drawn on the next frame (it appears at once, without a fade).
    // The audio service reported a change: show the bar now. If the receiver may be the one that changed, ask the helper to read it now.
    function cue() {
        if (!enabledHere) return
        if (!tvOutput && receiverMay) {
            deciding = !receiverOn
            if (deciding) decide.restart()
            var now = Date.now()
            if (now - nudgedAt > 300) { nudgedAt = now; Helper.post("/v1/receiver/nudge", {}) }
            if (gate.stale) gate.confirm()
        }
        if (viaReceiver) key()
        console.info("[VolumeOsd] cue: the bar " + (pill ? "is up" : "appears now"))
        show()
    }
    property double nudgedAt: 0
    Timer { id: hide; interval: osd.holdMs; onTriggered: osd.pill = false }
    Timer { id: decide; interval: osd.decideMs; onTriggered: osd.deciding = false }
    Connections { target: TvVolume; onCue: osd.cue() }
    Connections { target: TvVolume; onMutedChanged: osd.muteKey() }

    // A reading that is in date: a change of level or of mute, against the last one, shows the bar (while it is the receiver that is read).
    function read(r) {
        deciding = false
        var nextKey = r.volumeText + "|" + r.mute
        if (levelKey !== nextKey && levelKey !== "" && !tvOutput) show()
        levelKey = nextKey
        mutedRx = !!r.mute
        graceT.stop()
    }
    property string levelKey: ""
    onFreshChanged: if (!fresh && mutedRx) graceT.restart()
    Timer { id: graceT; interval: osd.graceMs; onTriggered: if (!osd.fresh) osd.mutedRx = false }

    // ---- the bar: a pill at the bottom centre, as long as its number and its format need, so it holds at 200 % text as well
    readonly property int edgeY: 64               // from the bottom edge: the TV's safe area and clear of subtitles
    readonly property int padX: Lu.space(6)       // 24
    readonly property int padY: Lu.space(3)       // 12
    readonly property int gap: Lu.space(5)        // 20, between the number, the bar and the format
    readonly property int glyph: 32
    readonly property int barMin: 140
    readonly property real fmtLimit: 360          // a long format is cut here, not allowed to widen the bar
    // the widest level it shows, so the bar's start does not move as the digits change. Measured by text items of the same role, as
    // drawn: TextMetrics' bounding width is a few px under the advance a Text needs (140 against 144 for "PCM · Surround" at the
    // caption size), which elided the format to "PCM · Surrou…" on the set
    readonly property real numW: Math.max(Math.ceil(numProbe.implicitWidth), glyph)
    readonly property real fmtW: format !== "" ? Math.min(Math.ceil(fmtText.implicitWidth), fmtLimit) : 0
    readonly property real rowH: Math.max(numProbe.implicitHeight, fmtText.implicitHeight, glyph)
    LuText { id: numProbe; visible: false; role: "label"; font.weight: Font.DemiBold; text: "-88.8" }
    LuPane {
        id: card
        width: Math.max(440, 2 * osd.padX + osd.numW + osd.gap + osd.barMin + (osd.fmtW > 0 ? osd.gap + osd.fmtW : 0))
        height: Math.max(56, Math.ceil(osd.rowH) + 2 * osd.padY)
        x: Math.round((osd.width - width) / 2)
        y: osd.height - osd.edgeY - height
        cardRadius: height / 2
        fillAlpha: osd.paneAlpha
        opacity: osd.paneUp ? 1 : 0
        visible: opacity > 0
        // in on the key's own frame, out quickly
        Behavior on opacity { enabled: !Lu.reduced; NumberAnimation { duration: osd.paneUp ? 0 : Lu.restMs } }

        LuText {
            id: numText
            visible: osd.source !== "none" && !osd.muted
            x: osd.padX; y: Math.round((card.height - implicitHeight) / 2)
            width: osd.numW
            horizontalAlignment: Text.AlignRight
            role: "label"
            font.weight: Font.DemiBold
            text: osd.level !== "" ? osd.level : "\u2013"
        }
        // muted: the slashed speaker where the number was
        LuIcon {
            visible: osd.source !== "none" && osd.muted
            x: osd.padX + Math.round((osd.numW - osd.glyph) / 2); y: Math.round((card.height - osd.glyph) / 2)
            size: osd.glyph
            name: "volume-x"
        }
        // the level, filling from the left; muted, the same length dimmed
        Rectangle {
            visible: osd.source !== "none"
            x: osd.padX + osd.numW + osd.gap
            y: Math.round((card.height - height) / 2)
            width: card.width - x - osd.padX - (osd.fmtW > 0 ? osd.gap + osd.fmtW : 0)
            height: Lu.space(2)
            radius: height / 2
            color: Lu.trackOff
            antialiasing: true
            Rectangle {
                width: Math.round(parent.width * osd.fraction)
                height: parent.height
                radius: height / 2
                color: osd.muted ? Lu.ink3 : Lu.ink
                antialiasing: true
                Behavior on width { enabled: !Lu.reduced; NumberAnimation { duration: Lu.focusMs } }
            }
        }
        LuText {
            id: fmtText
            visible: osd.format !== ""
            x: card.width - osd.padX - osd.fmtW; y: Math.round((card.height - implicitHeight) / 2)
            width: osd.fmtW
            horizontalAlignment: Text.AlignRight
            role: "caption"
            elide: Text.ElideRight
            text: osd.format
        }
        // nothing to read: say so
        LuText {
            visible: osd.source === "none"
            x: osd.padX; y: Math.round((card.height - implicitHeight) / 2)
            width: card.width - 2 * osd.padX
            horizontalAlignment: Text.AlignHCenter
            role: "caption"
            elide: Text.ElideRight
            text: "Receiver not answering"
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
            // The top corners, inside the 5 % safe area (96 x 54 of 1920 x 1080) and below Home's status bar (layout.json: 54 + 60, then its 20 gap),
            // so neither the bar's items nor Netflix's subtitles (the lower third) sit under it. It visits the four in turn.
            readonly property int edgeX: 96
            readonly property int topY: 134
            readonly property int inner: 94       // LG's bubble moves this far
            readonly property var spots: [
                { x: edgeX, y: topY }, { x: osd.width - edgeX - 64, y: topY },
                { x: osd.width - edgeX - 64 - inner, y: topY + inner }, { x: edgeX + inner, y: topY + inner }]
            property int place: 0                 // the index into spots
            width: 64; height: 64                 // the TV's smallest target: a speaker with a slash says it all
            x: spots[place].x
            y: spots[place].y
            cardRadius: width / 2
            fillAlpha: osd.paneAlpha
            LuIcon { anchors.centerIn: parent; size: osd.glyph; name: "volume-x" }
            // out, to the next place (the glass samples the backdrop where the pane is, so it is told where that is now), and back in
            function hop() {
                place = (place + 1) % spots.length
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
