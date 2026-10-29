import QtQuick 2.4
import "."

// A pill button. The primary one is the accent with the ink that reads on it; the others are the
// raised glass wash. Focus is the wash (or a lighter accent), the lit edge and the halo at once,
// and the lift after the focus has rested; pressed shrinks it for the press duration. Both move
// the pill only, never the label. One primary button per task.
Item {
    id: button
    property string text: ""
    property string icon: ""                // a glyph (icons/lucide) before the label
    property bool primary: false
    property bool danger: false             // the answer that destroys something: the label in the danger colour
    property bool focused: false
    property bool rested: true
    property bool instant: false            // the surface is moving or out of sight: focus takes its new state at once (LuHalo)
    signal activated()
    signal pointerFocus()

    readonly property int glyph: icon !== "" ? 22 + Lu.space(2) : 0
    implicitWidth: label.implicitWidth + glyph + 2 * Lu.space(7)
    implicitHeight: Lu.target
    width: implicitWidth
    height: implicitHeight
    opacity: enabled || (parent && !parent.enabled) ? 1 : Lu.disabledOpacity   // a disabled row already dims what it holds
    z: focused ? 1 : 0

    FocusLift {
        id: lift
        focused: button.focused
        rested: button.rested
        reduced: Lu.reduced || button.instant
        duration: Lu.cardMs
        easing: Lu.cardEasing
    }

    LuHalo {
        anchors.centerIn: parent
        width: parent.width * 1.6
        height: parent.height * 3
        z: -1
        on: button.focused
        instant: button.instant
    }
    Rectangle {
        anchors.fill: parent
        radius: Lu.radius("pill")
        color: button.primary ? (button.focused ? Qt.lighter(Lu.accent, 1.12) : Lu.accent)
                              : button.danger ? Qt.rgba(Lu.danger.r, Lu.danger.g, Lu.danger.b, button.focused ? 0.24 : 0.16)      // the mock's .btn.danger.strong
                              : (button.focused ? Lu.glassRaised : Lu.glass)
        border.width: 1
        border.color: button.focused ? Lu.edgeRaised : (button.danger ? Qt.rgba(Lu.danger.r, Lu.danger.g, Lu.danger.b, 0.5) : Lu.edge)
        antialiasing: true
        scale: (1 + (Lu.buttonScale - 1) * lift.value) * (mouse.pressed ? Lu.pressScale : 1)
        transformOrigin: Item.Center
        Behavior on scale { enabled: !Lu.reduced && mouse.pressed; NumberAnimation { duration: Lu.pressMs } }
    }
    LuHover { on: mouse.hovered && !button.focused; radius: Lu.radius("pill") }
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
        color: button.primary ? Lu.accentInk : button.danger ? Lu.dangerOnWash : Lu.ink
    }
    LuPointer {
        id: mouse
        anchors.fill: parent
        onClicked: { button.pointerFocus(); button.activated() }
    }
}
