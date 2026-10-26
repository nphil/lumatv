import QtQuick 2.4
import "../kit"

// How a panel arrives and leaves: one number, `progress`, from 0 (away) to 1 (shown), moved by one animation. The panel draws itself
// from it with a transform only (a slide from its edge; an opacity that changes re-batches what it covers every frame), so nothing is
// laid out while it moves. The host says when the panel is wanted (`presented`, set by Resident); a panel that is no longer
// wanted stays drawn until it has gone (`leaving`, which Resident reads to keep it visible). Reduced motion keeps only a short fade.
//
// Once it has gone the panel is parked: it moves what it draws off the screen (`parked`) and its nodes stay in the scene for `lingerMs`
// (`held`, which Resident reads too). Nodes that join the scene or leave it make Qt's renderer build the render list and the batches of the
// whole scene again; on the set that was the slow frame of every opening and closing (round 14, over Home: 3.2-3.4 ms where the nodes joined
// and 4.0-4.4 ms where they left, against about 2 ms for the rest). Opening a parked panel joins nothing. Parked nodes cost a draw in every
// frame of the scene, so they go once nobody has come back for `lingerMs`; `release()` lets them go at once.
//
// Durations: Lucent's layer motion is 220 ms in and 180 ms out, but the PLAN's gate wants the burst of frames a key starts to be over 180 ms
// after the key. On the set the first frame comes 41-42 ms after the gear tap or Back (the quick panel; the picker's came at 7 and 24 ms) and
// the last frame of the slide one frame after it ends, so a slide of 150 ms ended at 192-208 ms and 241 ms once. The slide therefore takes the
// shorter of the layer token and 110 ms in both directions: 41 + 110 + a 17 ms frame is 168 ms, which leaves the gate its margin.
QtObject {
    id: motion
    property bool presented: false
    property real progress: 0
    readonly property bool moving: anim.running
    // true while the panel is on its way out: it is drawn until progress reaches 0
    readonly property bool leaving: !presented && progress > 0
    // not wanted and the slide is over: nothing of the panel is on the screen. `held` starts when the panel is told to go (Resident's `visible`
    // has no gap where the slide ends) and runs out `lingerMs` later; the gate opens a panel again 1.5 s after it closed it
    readonly property bool parked: !presented && !leaving
    property int lingerMs: 4000
    property bool held: false
    function release() { held = false; linger.stop() }
    readonly property int enterMs: Math.min(Lu.layerMs, 110)
    readonly property int exitMs: Math.min(Lu.layerMs, 110)
    readonly property int fadeMs: Lu.ms("labelMs")           // reduced motion: one short fade, no travel

    onPresentedChanged: {
        if (presented) release()
        else if (lingerMs > 0) { held = true; linger.restart() }
        anim.stop()
        anim.to = presented ? 1 : 0
        anim.duration = Lu.reduced ? fadeMs : (presented ? enterMs : exitMs)
        anim.easing.type = presented ? Easing.OutCubic : Easing.InCubic
        anim.start()
    }

    property NumberAnimation anim: NumberAnimation { target: motion; property: "progress" }
    property Timer linger: Timer { interval: motion.lingerMs; onTriggered: motion.held = false }
}
