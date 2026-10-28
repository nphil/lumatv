import QtQuick 2.4
import "../kit"

// The value of a stepper or a nudge, on a line of its own: a chevron at each end and the value between them, in a recessed track. Left
// and Right change it, which is why the chevrons are there; they brighten with the row's focus. The value wraps rather than loses a letter.
Rectangle {
    id: step
    property string text: ""
    property bool focused: false
    readonly property int arrow: Math.round(Lu.target * 0.5)
    // The width it needs on one line: a chevron at each end and the longer of the value and `widest` (the longest value its row takes), so a
    // row that puts the control beside its words (QuickTile) does not change its mind as the value changes.
    property string widest: "Filmmaker"
    readonly property real need: 2 * (Lu.space(3) * 2 + arrow) + Math.max(Math.ceil(value.implicitWidth), Math.ceil(typical.advanceWidth))
    TextMetrics { id: typical; font: value.font; text: step.widest }

    width: parent ? parent.width : 0
    height: Math.max(Math.round(Lu.target * 0.75), value.height + 2 * Lu.space(2))
    radius: Lu.radius("control")
    color: Lu.isLight ? Lu.trackOff : Qt.rgba(0, 0, 0, 0.22)      // a shade on dark: the track's own ink wash read the value 3.7:1 over a bright wallpaper
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
