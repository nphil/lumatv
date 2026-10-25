import QtQuick 2.4
import "."

// Text in a type role of theme.json (display, title, body, label, caption): family, weight, size,
// tracking and line height come from the token, the colour is the ink. Any of them can still be
// set on the instance. Drawn natively on whole pixels, never inside a scaled item.
Text {
    id: text
    property string role: "body"
    readonly property var spec: Lu.type(role)

    color: Lu.ink
    font.family: Lu.family
    font.weight: spec.weight
    font.pixelSize: spec.size
    font.letterSpacing: spec.tracking
    lineHeight: spec.line
    lineHeightMode: Text.ProportionalHeight
    renderType: Text.NativeRendering
}
