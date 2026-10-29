import QtQuick 2.4
import "."

// The hover cue: the hover wash and a rim lifted from the glass edge, nothing else (no lift, no halo, no scale: those are focus).
// Put it under a control's content, sized to the control's plate; `on` is the area's `hovered` and false while the control has focus.
Rectangle {
    property bool on: false
    anchors.fill: parent
    radius: Lu.radius("row")
    color: Lu.wash("hover")
    border.width: 1
    border.color: Qt.rgba(Lu.edgeRaised.r, Lu.edgeRaised.g, Lu.edgeRaised.b, Lu.edgeRaised.a * 0.6)
    antialiasing: true
    opacity: on ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { enabled: !Lu.reduced; NumberAnimation { duration: Lu.pressMs } }
}
