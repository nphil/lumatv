import QtQuick 2.12
import "../kit"
import "LogoMath.js" as M

// Power-off, option 3: the name alone, leaving the way it came: its letters fade from the right
// and sink a little, until the screen is black. Total = 510 ms from the motion tokens (cardMs 180,
// pressMs 90, layerMs 220).
LogoBase {
    id: root

    readonly property int stagger: pressMs / 2              // 45 ms
    readonly property int span: cardMs + pressMs / 2         // 225 ms
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
            reverse: true
            staggerMs: root.stagger
            spanMs: root.span
            leaving: true
            lift: 10 * root.travel
            easing: "InOutCubic"
        }
    }
}
