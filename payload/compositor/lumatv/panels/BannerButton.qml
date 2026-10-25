import QtQuick 2.4
import "../kit"

// The small pill button of the banners (docs/mocks/25-input-banner, 18-no-signal): an icon and a label on the quiet glass
// wash; focus is the raised glass and the lit edge at once and the lift after the focus has rested (the kit's rule).
Item {
    id: btn
    property string icon: ""
    property string text: ""
    property bool focused: false
    property bool rested: true
    signal activated()
    signal hoverFocus()

    implicitWidth: row.width + 2 * Lu.space(6)
    implicitHeight: 52
    width: implicitWidth
    height: implicitHeight
    z: focused ? 1 : 0
    FocusLift { id: lift; focused: btn.focused; rested: btn.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
    scale: 1 + (Lu.buttonScale - 1) * lift.value
    transformOrigin: Item.Center
    Rectangle {
        anchors.fill: parent; radius: Lu.radius("pill"); antialiasing: true
        color: btn.focused ? Lu.glassRaised : Lu.wash("selected")
        border.width: btn.focused ? 1 : 0; border.color: Lu.edgeRaised
    }
    Row {
        id: row
        anchors.centerIn: parent
        spacing: Lu.space(3)
        LuIcon { visible: btn.icon !== ""; anchors.verticalCenter: parent.verticalCenter; name: btn.icon; size: 28; color: Lu.ink }
        LuText { anchors.verticalCenter: parent.verticalCenter; role: "label"; font.weight: Font.DemiBold; text: btn.text }
    }
    MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: btn.hoverFocus(); onClicked: btn.activated() }
}
