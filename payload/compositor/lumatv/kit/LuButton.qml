import QtQuick 2.4
import "."

// A pill button. The primary one is the accent with the ink that reads on it; the others are the
// raised glass wash. Focus is the wash (or a lighter accent), the lit edge and the halo at once,
// and the lift after the focus has rested; pressed shrinks it for the press duration. One primary
// button per task.
Item {
    id: button
    property string text: ""
    property string icon: ""                // a glyph (icons/lucide) before the label
    property bool primary: false
    property bool danger: false             // the answer that destroys something: the label in the danger colour
    property bool focused: false
    property bool rested: true
    signal activated()
    signal hoverFocus()

    readonly property int glyph: icon !== "" ? 22 + Lu.space(2) : 0
    implicitWidth: label.implicitWidth + glyph + 2 * Lu.space(7)
    implicitHeight: Lu.target
    width: implicitWidth
    height: implicitHeight
    opacity: enabled ? 1 : Lu.disabledOpacity
    z: focused ? 1 : 0

    FocusLift {
        id: lift
        focused: button.focused
        rested: button.rested
        reduced: Lu.reduced
        duration: Lu.cardMs
        easing: Lu.cardEasing
    }
    scale: (1 + (Lu.buttonScale - 1) * lift.value) * (mouse.pressed ? Lu.pressScale : 1)
    transformOrigin: Item.Center
    Behavior on scale { enabled: !Lu.reduced && mouse.pressed; NumberAnimation { duration: Lu.pressMs } }

    LuHalo {
        anchors.centerIn: parent
        width: parent.width * 1.6
        height: parent.height * 3
        z: -1
        on: button.focused
    }
    Rectangle {
        anchors.fill: parent
        radius: Lu.radius("pill")
        color: button.primary ? (button.focused ? Qt.lighter(Lu.accent, 1.12) : Lu.accent)
                              : (button.focused ? Lu.glassRaised : Lu.glass)
        border.width: 1
        border.color: button.focused ? Lu.edgeRaised : Lu.edge
        antialiasing: true
    }
    LuIcon {
        visible: button.icon !== ""
        x: Math.round((button.width - label.width - button.glyph) / 2); y: Math.round((button.height - height) / 2)
        size: 22; name: button.icon; color: label.color
    }
    LuText {
        id: label
        x: Math.round((button.width - width + button.glyph) / 2); y: Math.round((button.height - height) / 2)
        role: "label"
        text: button.text
        color: button.primary ? Lu.accentInk : button.danger ? Lu.danger : Lu.ink
    }
    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        onEntered: button.hoverFocus()
        onClicked: button.activated()
    }
}
