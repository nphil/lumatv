import QtQuick 2.4

// A motion stepped once per rendered frame, for the 120 Hz refresh mode (Lu.fast; docs/research/ui120-result.md).
//
// The shell client runs Qt's basic render loop (LG's Qt 5.12, WEBOS_GL_DISABLE_THREADED_RENDERING), which installs no animation driver:
// every QML animation steps on the unified timer, a 16 ms timer whatever the refresh (qsgrenderloop.cpp `animationDriver()` null,
// qabstractanimation.cpp DEFAULT_TIMER_INTERVAL 16). Round 14 at 120 Hz: every frame of the slides 16.7 ms apart, two refreshes, with the
// compositor's pipeline at 4.75 ms. A value set in the window's `afterAnimating` is in that frame's sync (the basic loop emits it between
// polish and sync), and `window.update()` asks for the next frame, which QtWayland delivers on the compositor's frame callback (it holds the
// request while one is pending): one step per refresh, as fast as the callbacks come. Time advances a refresh per frame, the even steps of a
// regular cadence, and is put back on the wall clock when it is more than a frame off either way (a frame skipped; the host, which has no
// refresh to pace it), as Qt's own vsync animation driver does.
//
// `start()` moves `value` from where it is to `to` over `duration` ms with the easing named; `value` holds at the end. Not used at 60 Hz:
// the owner keeps its NumberAnimation or Behavior there, so the look at 60 is untouched.
QtObject {
    id: fm

    property QtObject window: null             // the owner's Window.window, whose frames step the motion
    property real value: 0
    readonly property bool running: active
    property bool active: false
    property real to: 0
    property int duration: 0
    property string easing: "outCubic"          // outCubic | inCubic | linear
    property int steps: 0                       // frames the current motion was stepped in (the host tests read it)
    readonly property real stepMs: 1000 / Math.max(1, Lu.refreshHz)
    property real from: 0
    property double startedAt: 0
    property real t: 0

    function start(target, ms, ease) {
        from = value
        to = target
        duration = ms
        easing = ease || "outCubic"
        steps = 0
        t = 0
        startedAt = Date.now()
        if (duration <= 0 || from === to) { value = to; active = false; return }
        active = true
        kick()
    }
    function stop() { active = false }
    function kick() { if (window && window.update) window.update() }
    function curve(k) {
        return easing === "inCubic" ? k * k * k : easing === "linear" ? k : 1 - Math.pow(1 - k, 3)
    }
    // one frame: a refresh of motion time, resynced to the wall clock when more than a frame apart
    function step() {
        if (!active) return
        var wall = Date.now() - startedAt
        t += stepMs
        if (wall > t + stepMs || wall < t - stepMs) t = wall
        steps++
        var k = Math.min(1, t / duration)
        value = from + (to - from) * curve(k)
        if (k >= 1) {
            active = false
            // how many frames the motion got and how long it took: the shell's state (`Lu.slides`, what `ctl shell` and the gate read) and
            // a line for the runner's log (PmLog on the set)
            Lu.noteSlide(steps, Math.round(wall), duration)
            console.info("[FrameMotion] " + steps + " steps in " + Math.round(wall) + " ms for " + duration)
        } else kick()
    }

    property Connections frames: Connections {
        target: fm.window
        ignoreUnknownSignals: true
        onAfterAnimating: fm.step()
    }
}
