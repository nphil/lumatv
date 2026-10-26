import QtQuick 2.4
import "."

// Text in a type role of theme.json (display, title, body, label, caption): family, weight, size,
// tracking and line height come from the token, the colour is the ink. Any of them can still be
// set on the instance. Drawn natively on whole pixels, never inside a scaled item. The role's line
// height is the pitch between lines; Qt adds that leading under every line, the last one too, so a
// text of one line keeps none of it (else it sits high in whatever centres it), and a text of more
// lines reports what hangs under its last line as `trailing` for a host that centres it. The line
// height is set when the line count changes, not bound to it: an elided text re-lays out for
// either and a binding would loop.
Text {
    id: text
    property string role: "body"
    readonly property var spec: Lu.type(role)
    property real line: spec.line            // the pitch between lines; an instance may set its own
    readonly property real trailing: lineCount > 1 ? (contentHeight / (lineCount * lineHeight)) * (lineHeight - 1) : 0

    color: Lu.ink
    font.family: Lu.family
    font.weight: spec.weight
    font.pixelSize: spec.size
    font.letterSpacing: spec.tracking
    lineHeightMode: Text.ProportionalHeight
    renderType: Text.NativeRendering

    function pitch() { lineHeight = lineCount > 1 ? line : Math.min(1, line) }
    onLineCountChanged: pitch()
    onLineChanged: pitch()
    Component.onCompleted: pitch()
}
