import QtQuick 2.4
import "../kit"

// How a panel arrives and leaves: one number, `progress`, from 0 (away) to 1 (shown), moved by one animation. The panel draws itself
// from it with transforms and opacity only (a slide from its edge and a fade), so nothing is laid out while it moves and a panel over
// video costs the same as one over Home. The host says when the panel is wanted (`presented`, set by Resident); a panel that is no longer
// wanted stays drawn until it has gone (`leaving`, which Resident reads to keep it visible). Reduced motion keeps only a short fade.
//
// Durations: Lucent's layer motion is 220 ms in and 180 ms out, but the PLAN's gate wants the surface fully open 180 ms after the key
// and the key reaches the panel a frame or two late, so the slide takes the shorter of the layer token and 150 ms (LG's own Quick Settings
// slides in 150 ms, after a 180 ms pause); the slide back takes the shorter of the exit token and 150 ms.
QtObject {
    id: motion
    property bool presented: false
    property real progress: 0
    readonly property bool moving: anim.running
    // true while the panel is on its way out: it is drawn until progress reaches 0
    readonly property bool leaving: !presented && progress > 0
    readonly property int enterMs: Math.min(Lu.layerMs, 150)
    readonly property int exitMs: Math.min(Lu.layerMs, 150)
    readonly property int fadeMs: Lu.ms("labelMs")           // reduced motion: one short fade, no travel

    onPresentedChanged: {
        anim.stop()
        anim.to = presented ? 1 : 0
        anim.duration = Lu.reduced ? fadeMs : (presented ? enterMs : exitMs)
        anim.easing.type = presented ? Easing.OutCubic : Easing.InCubic
        anim.start()
    }

    property NumberAnimation anim: NumberAnimation { target: motion; property: "progress" }
}
