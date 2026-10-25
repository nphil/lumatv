import QtQuick 2.12
import "../kit"
import "LogoMath.js" as M

// Power-off, option 1 (default): the name fades and sinks, then the panes slide apart and fade,
// front pane first, until the screen is black. Total = 640 ms from the motion tokens (cardMs 180,
// layerMs 220, restMs 150, pressMs 90). Exits have no token for easing; InOutCubic is symmetric.
LogoBase {
    id: root

    readonly property int nameMs: cardMs
    readonly property int paneAt: restMs
    readonly property int paneMs: layerMs + cardMs
    duration: paneAt + pressMs + paneMs

    readonly property real xName: M.inOutCubic(M.seg(t, 0, nameMs))
    // 1 assembled and visible, 0 apart and gone; the front pane leaves first.
    readonly property real sIris: 1 - M.inOutCubic(M.seg(t, paneAt, paneMs))
    readonly property real sFoam: 1 - M.inOutCubic(M.seg(t, paneAt + pressMs / 2, paneMs))
    readonly property real sLove: 1 - M.inOutCubic(M.seg(t, paneAt + pressMs, paneMs))

    readonly property int markSize: 178
    readonly property int gap: 44

    Item {
        x: root.driftX
        y: root.driftY
        width: root.width
        height: root.height

        LogoMark {
            id: mark
            x: Math.round((root.width - width) / 2)
            y: Math.round((root.height - (height + root.gap + word.height)) / 2)
            unit: root.markSize / 142
            love: root.brand.love; foam: root.brand.foam; iris: root.brand.iris; sheen: root.brand.sheen
            brightness: root.shine
            travel: root.travel
            pLove: root.sLove; pFoam: root.sFoam; pIris: root.sIris
            kLove: root.sLove; kFoam: root.sFoam; kIris: root.sIris
        }
        LogoWord {
            id: word
            x: Math.round((root.width - width) / 2)
            y: mark.y + mark.height + root.gap + Math.round(10 * root.xName * root.travel)
            name: root.name
            family: root.family
            pixelSize: 88
            color: root.brand.ink
            opacity: (1 - root.xName) * root.shine
        }
    }
}
