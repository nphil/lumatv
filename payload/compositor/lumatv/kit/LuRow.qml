import QtQuick 2.4
import "."

// A list row: label, optional detail under it, optional value at the right, and an optional
// control (a LuToggle, a LuSlider) in `trailing`. Transparent at rest. Focus is the wash and the lit
// edge at once, the lift after the focus has rested, and the accent halo under it; selection is a
// lighter wash that stays without focus. The host owns which row is focused.
Item {
    id: row
    property string label: ""
    property string detail: ""
    property string value: ""
    property bool focused: false
    property bool selected: false
    property bool rested: true              // the host's focus-rest flag (LumaHome.focusRested)
    property alias trailing: trailingSlot.data
    signal activated()
    signal hoverFocus()

    // two lines (label and detail) need more than the row height to breathe
    implicitHeight: detail !== "" ? Lu.row + Lu.space(4) : Lu.row
    height: implicitHeight
    opacity: enabled ? 1 : Lu.disabledOpacity
    z: focused ? 1 : 0

    FocusLift {
        id: lift
        focused: row.focused
        rested: row.rested
        reduced: Lu.reduced
        duration: Lu.cardMs
        easing: Lu.cardEasing
    }
    scale: 1 + (Lu.rowScale - 1) * lift.value
    transformOrigin: Item.Center

    LuHalo {
        anchors.centerIn: parent
        width: parent.width * 1.2
        height: parent.height * 2.4
        z: -1
        on: row.focused
    }
    Rectangle {
        anchors.fill: parent
        radius: Lu.radius("row")
        color: row.focused ? Lu.wash("focus") : row.selected ? Lu.wash("selected") : "transparent"
        border.width: row.focused ? 1 : 0
        border.color: Lu.edgeRaised
        antialiasing: true
    }
    Column {
        x: Lu.space(6)
        anchors.verticalCenter: parent.verticalCenter
        width: parent.width - 2 * Lu.space(6) - trailingSlot.width - valueText.width - (valueText.visible ? Lu.space(4) : 0)
        LuText { width: parent.width; role: "label"; text: row.label; elide: Text.ElideRight }
        LuText { width: parent.width; role: "caption"; text: row.detail; color: Lu.ink2; visible: row.detail !== ""; elide: Text.ElideRight }
    }
    LuText {
        id: valueText
        visible: row.value !== ""
        anchors.verticalCenter: parent.verticalCenter
        x: parent.width - Lu.space(6) - width - trailingSlot.width - (trailingSlot.children.length ? Lu.space(4) : 0)
        role: "label"
        color: Lu.ink2
        text: row.value
    }
    Item {
        id: trailingSlot
        anchors.verticalCenter: parent.verticalCenter
        x: parent.width - Lu.space(6) - width
        width: childrenRect.width
        height: childrenRect.height
    }
    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        onEntered: row.hoverFocus()
        onClicked: row.activated()
    }
}
