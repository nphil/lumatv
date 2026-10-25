import QtQuick 2.12
import "../kit"
import "LogoMath.js" as M

// What every logo animation shares: a clock that runs once from 0 to 1 over `duration`
// milliseconds, the finished signal, the burn-in drift, and the motion tokens. A logo is a
// plain Item with no luna dependency; the system-UI window around it belongs to the host.
//
// The frame is a pure function of `progress`: the animation only moves `progress`, so setting it
// by hand (a still, a scrubber) shows exactly what the running logo shows at that moment.
// For a power-on logo progress 0 is black and 1 the resting mark; for a power-off logo 0 is the
// resting mark and 1 is black.
Item {
    id: root

    width: 1920
    height: 1080

    property int duration: 800                 // ms, first frame to end state; set by each logo
    property real progress: 0
    property alias running: anim.running
    signal finished()                          // once, when progress has reached 1

    readonly property real t: progress * duration

    // The wordmark text; the build fills in the brand name.
    property string name: "{{displayName}}"
    readonly property string family: Lu.family !== "" ? Lu.family : "Manrope"

    // Brand colours. The logo is always drawn on black, so the ink is the dark palette's whatever the
    // light/dark setting says; the three pane colours are icon.svg's.
    readonly property QtObject brand: QtObject {
        readonly property color black: "#000000"
        readonly property color ink: "#F2EFF6"
        readonly property color iris: "#C4A7E7"
        readonly property color foam: "#9CCFD8"
        readonly property color love: "#EB6F92"
        readonly property color sheen: "#E0DEF4"
    }

    // Motion tokens (theme.json "motion"). Until LumaHome has handed the theme over, Lu has none,
    // so the shipped values stand in: the logo runs at boot, before that handover.
    function tok(name, shipped) { var v = Lu.ms(name); return v > 0 ? v : shipped }
    readonly property int pressMs: tok("pressMs", 90)
    readonly property int cardMs: tok("cardMs", 180)
    readonly property int layerMs: tok("layerMs", 220)
    readonly property int restMs: tok("restMs", 150)
    readonly property string enterEasing: Lu.motion.cardEasing || "OutCubic"
    // Under reduced motion panes and letters stop travelling and only fade.
    readonly property real travel: Lu.reduced ? 0 : 1

    // Burn-in: the logo is never still for long. The drift and the breath step twice a second, in
    // whole pixels and at most 6 % brightness, so the compositor does not redraw at 60 Hz for a
    // resting logo. Peak brightness of anything lit is `peak`.
    property real peak: 0.85
    property real clock: 0                      // seconds since start()
    readonly property int driftX: M.driftX(clock)
    readonly property int driftY: M.driftY(clock)
    readonly property real shine: peak * (1 - 0.06 * M.breath(clock))
    property bool _drifting: false

    function start() {
        anim.stop()
        clock = 0
        progress = 0
        _drifting = true
        anim.start()
    }

    function reset() {
        anim.stop()
        _drifting = false
        clock = 0
        progress = 0
    }

    NumberAnimation {
        id: anim
        target: root
        property: "progress"
        from: 0
        to: 1
        duration: root.duration
        easing.type: Easing.Linear
        onRunningChanged: if (!running && root.progress >= 1) root.finished()
    }

    Rectangle { anchors.fill: parent; color: root.brand.black; z: -1 }

    // The logo can be up before LumaHome has loaded the fonts, so it loads the two weights it uses
    // itself; a local file is ready on the first frame.
    FontLoader { source: "file://{{dataDir}}/fonts/Manrope-Regular.ttf" }
    FontLoader { source: "file://{{dataDir}}/fonts/Manrope-Bold.ttf" }

    Timer {
        interval: 500
        repeat: true
        running: root._drifting && root.visible
        onTriggered: root.clock += 0.5
    }
}
