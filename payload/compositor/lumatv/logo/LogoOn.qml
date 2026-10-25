import QtQuick 2.12
import "../kit"
import "LogoMath.js" as M

// Power-on, option 1 (default): the three glass panes slide in from either side and settle into
// the mark, then the name rises underneath and fades in. One mark, one line of text, nothing else
// lit. Total = 740 ms from the motion tokens (layerMs 220, restMs 150, pressMs 90).
LogoBase {
    id: root

    readonly property int paneMs: 2 * layerMs               // each pane's slide
    readonly property int nameAt: 2 * restMs                // the name starts as the panes settle
    readonly property int nameMs: 2 * layerMs
    duration: Math.max(pressMs + paneMs, nameAt + nameMs)

    // Panes arrive staggered by half a press and a press: love, foam, then the front pane.
    readonly property real pLove: M.ease(enterEasing, M.seg(t, 0, paneMs))
    readonly property real pFoam: M.ease(enterEasing, M.seg(t, pressMs / 2, paneMs))
    readonly property real pIris: M.ease(enterEasing, M.seg(t, pressMs, paneMs))
    // A pane is fully visible after the first quarter of its slide.
    readonly property real kLove: M.outQuad(M.seg(t, 0, paneMs / 4))
    readonly property real kFoam: M.outQuad(M.seg(t, pressMs / 2, paneMs / 4))
    readonly property real kIris: M.outQuad(M.seg(t, pressMs, paneMs / 4))
    readonly property real eName: M.ease(enterEasing, M.seg(t, nameAt, nameMs))

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
            pLove: root.pLove; pFoam: root.pFoam; pIris: root.pIris
            kLove: root.kLove; kFoam: root.kFoam; kIris: root.kIris
        }
        LogoWord {
            id: word
            x: Math.round((root.width - width) / 2)
            y: mark.y + mark.height + root.gap + Math.round(14 * (1 - root.eName) * root.travel)
            name: root.name
            family: root.family
            pixelSize: 88
            color: root.brand.ink
            opacity: root.eName * root.shine
        }
    }
}
