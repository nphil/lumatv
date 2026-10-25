import QtQuick 2.12
import "../kit"
import "LogoMath.js" as M

// Power-on, option 2: the mark fades up almost in place, one soft arc sweeps once around it, and
// the name fades in underneath as the arc fades out. Total = 740 ms from the motion tokens
// (layerMs 220, restMs 150, pressMs 90).
LogoBase {
    id: root

    readonly property int paneMs: 2 * layerMs
    readonly property int arcAt: pressMs
    readonly property int arcMs: 2 * layerMs + cardMs
    readonly property int nameAt: 2 * restMs
    readonly property int nameMs: 2 * layerMs
    duration: Math.max(paneMs, arcAt + arcMs, nameAt + nameMs)

    readonly property real ePane: M.ease(enterEasing, M.seg(t, 0, paneMs))
    readonly property real kPane: M.outQuad(M.seg(t, 0, layerMs))
    readonly property real eName: M.ease(enterEasing, M.seg(t, nameAt, nameMs))
    // The arc travels one full turn, then fades over its last quarter.
    readonly property real arcP: M.seg(t, arcAt, arcMs)
    readonly property real arcEnv: Math.min(arcP / 0.12, 1) * Math.min((1 - arcP) / 0.25, 1)

    readonly property int markSize: 178
    readonly property int centerY: 450

    Item {
        x: root.driftX
        y: root.driftY
        width: root.width
        height: root.height

        LogoMark {
            id: mark
            x: Math.round((root.width - width) / 2)
            y: root.centerY - Math.round(height / 2)
            unit: root.markSize / 142
            love: root.brand.love; foam: root.brand.foam; iris: root.brand.iris; sheen: root.brand.sheen
            brightness: root.shine
            travel: root.travel * 0.35
            pLove: root.ePane; pFoam: root.ePane; pIris: root.ePane
            kLove: root.kPane; kFoam: root.kPane; kIris: root.kPane
        }
        LogoArc {
            x: mark.x + Math.round(mark.width / 2) - 200
            y: mark.y + Math.round(mark.height / 2) - 200
            sweep: 360 * M.inOutCubic(root.arcP)
            opacity: Math.max(0, root.arcEnv) * root.shine
        }
        LogoWord {
            x: Math.round((root.width - width) / 2)
            y: root.centerY + 200 + 40 + Math.round(12 * (1 - root.eName) * root.travel)
            name: root.name
            family: root.family
            pixelSize: 64
            color: root.brand.ink
            opacity: root.eName * root.shine
        }
    }
}
