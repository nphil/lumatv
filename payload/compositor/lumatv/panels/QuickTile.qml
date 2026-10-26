import QtQuick 2.4
import "../kit"

// One row of the quick panel, drawn for a narrow column (about 430 px): an icon tile, the name and, only when it says something the person
// needs (an input, a state), a second line. A control that fits on the name's line (a switch, a chevron, a chip) sits at its right in
// `trailing`; a control that needs width (a stepper's value) goes on a line of its own in `below`, under the words and as wide as they.
// Words wrap and the row grows: nothing is cut off, at any text size. Transparent at rest; focus is the wash, the lit edge and the halo,
// and the lift grows the plate only, so words and controls never move.
Item {
    id: tile
    property string label: ""
    property string detail: ""
    property int detailLines: 1             // lines the second line keeps, empty or not, for a row whose answer comes late: the rows below do not move
    property string icon: ""
    property bool tileAccent: false
    property bool focused: false
    property bool rested: true
    property alias trailing: trailingSlot.data
    property alias below: belowSlot.data
    signal activated()
    signal hoverFocus()

    readonly property int pad: Lu.space(3)
    readonly property int gap: Lu.space(4)
    readonly property int tileSize: Math.round(Lu.target * 0.75)
    readonly property int textX: pad + (icon !== "" ? tileSize + gap : 0)
    readonly property real trailW: trailingSlot.childrenRect.width
    readonly property real belowH: belowSlot.childrenRect.height
    readonly property real textW: width - textX - pad - (trailW > 0 ? trailW + gap : 0)
    readonly property real textH: Math.max(words.height, detailLines > 1 && detail !== "" ? labelText.height + detailLines * lineProbe.height : 0)
    // a second line that does not fit on one breaks at its dots ("Follows the picture" over "70%"), not in the middle of a phrase. Measured
    // with font metrics: a hidden Text would lay itself out again at every answer that changes the line.
    readonly property string shownDetail: detailWidth.advanceWidth > textW ? detail.replace(/ \u00b7 /g, "\n") : detail

    implicitHeight: belowH > 0 ? pad + textH + Lu.space(2) + belowH + pad
                               : Math.max(Lu.row, pad + Math.max(textH, tileSize, trailingSlot.childrenRect.height) + pad)
    height: implicitHeight
    opacity: enabled ? 1 : Lu.disabledOpacity
    z: focused ? 1 : 0

    FocusLift {
        id: lift
        focused: tile.focused
        rested: tile.rested
        reduced: Lu.reduced
        duration: Lu.cardMs
        easing: Lu.cardEasing
    }
    LuHalo {
        anchors.centerIn: parent
        width: parent.width * 1.2
        height: parent.height * 2.2
        z: -1
        on: tile.focused
    }
    Rectangle {
        anchors.fill: parent
        radius: Lu.radius("row")
        color: tile.focused ? Lu.wash("focus") : "transparent"
        border.width: tile.focused ? 1 : 0
        border.color: Lu.edgeRaised
        antialiasing: true
        scale: 1 + (Lu.rowScale - 1) * lift.value
        transformOrigin: Item.Center
    }
    LuTile {
        visible: tile.icon !== ""
        x: tile.pad
        y: tile.belowH > 0 ? tile.pad : Math.round((tile.height - tile.tileSize) / 2)
        size: tile.tileSize
        icon: tile.icon
        accent: tile.tileAccent
    }
    Column {
        id: words
        x: tile.textX
        y: tile.belowH > 0 ? tile.pad : Math.round((tile.height - height) / 2)
        width: tile.textW
        LuText { id: labelText; width: parent.width; role: "label"; text: tile.label; wrapMode: Text.WordWrap }
        LuText { id: detailText; width: parent.width; role: "caption"; text: tile.shownDetail; color: Lu.ink2; wrapMode: Text.WordWrap; visible: tile.detail !== "" }
    }
    TextMetrics { id: detailWidth; font: detailText.font; text: tile.detail }
    LuText { id: lineProbe; visible: false; role: "caption"; text: "M" }
    Item {
        id: trailingSlot
        x: tile.width - tile.pad - width
        y: Math.round((tile.height - height) / 2)
        width: childrenRect.width
        height: childrenRect.height
    }
    Item {
        id: belowSlot
        x: tile.textX
        y: tile.pad + tile.textH + Lu.space(2)
        width: tile.width - tile.textX - tile.pad
        height: childrenRect.height
    }
    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        onEntered: tile.hoverFocus()
        onClicked: tile.activated()
    }
}
