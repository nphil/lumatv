import QtQuick 2.4
import "."

// A list row: label, optional detail under it, optional value at the right, and an optional
// control (a LuToggle, a LuSlider) in `trailing`. Transparent at rest. Focus is the wash and the lit
// edge at once, the lift after the focus has rested, and the accent halo under it; selection is a
// lighter wash that stays without focus. The lift grows the plate only: the tile, the words and the
// control stay where they are, so focus never moves them sideways. The host owns which row is focused.
Item {
    id: row
    property string label: ""
    property bool danger: false             // the row that destroys something (Uninstall): label and icon in the danger colour
    property string detail: ""
    property string value: ""
    property string help: ""                // an explanation behind the info mark (LuInfo); "" draws no mark. A row is its name and its value, so what is not obvious goes here
    property string icon: ""                // a Lucide icon name: draws a tile at the left
    property bool tileAccent: false         // the tile in the accent (a switch that is on, the current section)
    readonly property int tileSize: Math.round(Lu.target * 0.75)
    readonly property int textX: Lu.space(6) + (icon !== "" ? tileSize + Lu.space(5) : 0)
    property bool focused: false
    property bool selected: false
    property bool rested: true              // the host's focus-rest flag (LumaHome.focusRested)
    property alias trailing: trailingSlot.data
    signal activated()
    signal hoverFocus()
    signal helpAsked()                      // the pointer pressed the info mark (the D-pad and Menu key reach the host's own help)

    // two lines (label and detail) need more than the row height to breathe; a detail too long for its line takes a
    // second one (never a third) and the row grows with it, so a narrow row or a long translation is never cut. The
    // leading under a wrapped detail's last line is not part of the words: the block is centred without it.
    implicitHeight: Math.max(detail !== "" ? Lu.row + Lu.space(2) : Lu.row, words.height - detailText.trailing + Lu.space(4))
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
        scale: 1 + (Lu.rowScale - 1) * lift.value
        transformOrigin: Item.Center
    }
    LuTile {
        visible: row.icon !== ""
        x: Lu.space(6)
        anchors.verticalCenter: parent.verticalCenter
        size: row.tileSize
        icon: row.icon
        accent: row.tileAccent
    }
    Column {
        id: words
        x: row.textX
        anchors.verticalCenter: parent.verticalCenter
        anchors.verticalCenterOffset: detailText.trailing / 2       // the glyphs are the top of the column: they, not the leading, sit in the middle
        width: parent.width - row.textX - Lu.space(6) - trailingSlot.width - valueText.width - (valueText.visible ? Lu.space(4) : 0)
        Item {
            width: parent.width
            height: labelText.height
            LuText {
                id: labelText
                width: Math.min(implicitWidth, parent.width - (infoMark.visible ? infoMark.width + Lu.space(3) : 0))
                role: "label"; text: row.label; color: row.danger ? Lu.dangerInk : Lu.ink; elide: Text.ElideRight
            }
            LuInfo {
                id: infoMark
                visible: row.help !== ""
                x: labelText.width + Lu.space(3)
                anchors.verticalCenter: labelText.verticalCenter
                focused: row.focused
                onAsked: row.helpAsked()
            }
        }
        LuText { id: detailText; width: parent.width; role: "caption"; text: row.detail; color: (row.enabled ? Lu.ink2 : Lu.ink); visible: row.detail !== ""; wrapMode: Text.WordWrap; maximumLineCount: 2; elide: Text.ElideRight }
    }
    LuText {
        id: valueText
        visible: row.value !== ""
        anchors.verticalCenter: parent.verticalCenter
        x: parent.width - Lu.space(6) - width - trailingSlot.width - (trailingSlot.children.length ? Lu.space(4) : 0)
        role: "label"
        color: (row.enabled ? Lu.ink2 : Lu.ink)
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
        z: -1                               // under the controls: a click on a segment or on the info mark is theirs, the rest of the row is the row's
        anchors.fill: parent
        hoverEnabled: true
        onEntered: row.hoverFocus()
        onClicked: row.activated()
    }
}
