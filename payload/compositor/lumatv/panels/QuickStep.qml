import QtQuick 2.4
import "../kit"

// The value of a stepper or a nudge, on a line of its own: a chevron at each end and the value between them, in a recessed track. Left
// and Right change it, which is why the chevrons are there; they brighten with the row's focus. The value wraps rather than loses a letter.
Rectangle {
    id: step
    property string text: ""
    property bool focused: false
    readonly property int arrow: Math.round(Lu.target * 0.5)

    width: parent ? parent.width : 0
    height: Math.max(Math.round(Lu.target * 0.75), value.height + 2 * Lu.space(2))
    radius: Lu.radius("control")
    color: Lu.trackOff
    border.width: 1
    border.color: focused ? Lu.edgeRaised : Lu.edge
    antialiasing: true

    LuIcon {
        x: Lu.space(3); y: Math.round((step.height - height) / 2)
        name: "chevron-left"; size: step.arrow
        color: step.focused ? Lu.ink : Lu.ink2
    }
    LuText {
        id: value
        x: Lu.space(3) * 2 + step.arrow
        y: Math.round((step.height - height) / 2)
        width: step.width - 2 * x
        horizontalAlignment: Text.AlignHCenter
        role: "label"
        text: step.text
        wrapMode: Text.WordWrap
    }
    LuIcon {
        x: step.width - Lu.space(3) - width; y: Math.round((step.height - height) / 2)
        name: "chevron-right"; size: step.arrow
        color: step.focused ? Lu.ink : Lu.ink2
    }
}
