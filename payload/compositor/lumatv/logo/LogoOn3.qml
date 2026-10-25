import QtQuick 2.12
import "../kit"
import "LogoMath.js" as M

// Power-on, option 3: the name alone. Its letters rise one after another from the left, a hair
// apart, and rest. No mark, so the fewest pixels lit of the three. Total = 700 ms from the motion
// tokens (layerMs 220, pressMs 90, restMs 150).
LogoBase {
    id: root

    readonly property int stagger: pressMs * 2 / 3          // 60 ms
    readonly property int span: layerMs + pressMs            // 310 ms
    duration: letters.endMs

    Item {
        x: root.driftX
        y: root.driftY
        width: root.width
        height: root.height

        LogoLetters {
            id: letters
            x: Math.round((root.width - width) / 2)
            y: Math.round((root.height - height) / 2) - 20
            name: root.name
            family: root.family
            pixelSize: 128
            color: Qt.rgba(root.brand.ink.r * root.shine, root.brand.ink.g * root.shine, root.brand.ink.b * root.shine, 1)
            t: root.t
            staggerMs: root.stagger
            spanMs: root.span
            lift: 16 * root.travel
            easing: root.enterEasing
        }
    }
}
