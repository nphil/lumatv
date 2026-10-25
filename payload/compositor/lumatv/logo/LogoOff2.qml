import QtQuick 2.12
import "../kit"
import "LogoMath.js" as M

// Power-off, option 2: the name fades, one soft arc sweeps once around the mark against the
// direction it took on the way in, and the mark fades out under it until the screen is black.
// Total = 530 ms from the motion tokens (cardMs 180, layerMs 220, pressMs 90).
LogoBase {
    id: root

    readonly property int nameMs: cardMs
    readonly property int arcMs: 2 * layerMs
    readonly property int markAt: pressMs
    readonly property int markMs: 2 * layerMs
    duration: Math.max(nameMs, arcMs, markAt + markMs)

    readonly property real xName: M.inOutCubic(M.seg(t, 0, nameMs))
    readonly property real sMark: 1 - M.inOutCubic(M.seg(t, markAt, markMs))
    readonly property real arcP: M.seg(t, 0, arcMs)
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
            pLove: root.sMark; pFoam: root.sMark; pIris: root.sMark
            kLove: root.sMark; kFoam: root.sMark; kIris: root.sMark
        }
        LogoArc {
            x: mark.x + Math.round(mark.width / 2) - 200
            y: mark.y + Math.round(mark.height / 2) - 200
            reverse: true
            sweep: -360 * M.inOutCubic(root.arcP)
            opacity: Math.max(0, root.arcEnv) * root.shine
        }
        LogoWord {
            x: Math.round((root.width - width) / 2)
            y: root.centerY + 200 + 40 + Math.round(8 * root.xName * root.travel)
            name: root.name
            family: root.family
            pixelSize: 64
            color: root.brand.ink
            opacity: (1 - root.xName) * root.shine
        }
    }
}
