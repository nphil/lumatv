import QtQuick 2.12
import "LogoMath.js" as M

// The wordmark: the name in Manrope, its first part regular and its capital-led tail bold (see
// LogoMath.splitName), tracked -0.03 em as the boot mock is. Native text on whole pixels: the
// width and height are integers and the owner must place the item on integers too.
Item {
    id: word

    property string name: ""
    property string family: "Manrope"
    property int pixelSize: 88
    property color color: "white"

    readonly property var parts: M.splitName(name)

    width: Math.ceil(head.implicitWidth) + Math.ceil(tail.implicitWidth)
    height: Math.ceil(head.implicitHeight)

    Text {
        id: head
        text: word.parts.head
        color: word.color
        font.family: word.family
        font.weight: Font.Normal
        font.pixelSize: word.pixelSize
        font.letterSpacing: -0.03 * word.pixelSize
        renderType: Text.NativeRendering
    }
    Text {
        id: tail
        x: Math.ceil(head.implicitWidth)
        text: word.parts.tail
        color: word.color
        font.family: word.family
        font.weight: Font.Bold
        font.pixelSize: word.pixelSize
        font.letterSpacing: -0.03 * word.pixelSize
        renderType: Text.NativeRendering
    }
}
