import QtQuick 2.4
import "."

// A selectable card with an icon, a title and a short line, for a grid of choices (a picture mode, a
// receiver's sound mode, an input): quiet glass at rest, the focus wash and lit edge when focused, the
// accent check when it is the current one. The host owns which card is focused; the lift follows the
// focus rule (after the focus has rested) and grows the plate only: the icon and the words stay where
// they are, as in a row. Cards in a row or grid share one shape, so a longer title is cut with an
// ellipsis rather than growing the card. With a second line the card is as tall as a two-line row.
Item {
    id: card
    property string icon: ""
    property string title: ""
    property string sub: ""
    property bool focused: false
    property bool selected: false
    property bool rested: true
    signal activated()
    signal pointerFocus()
    implicitWidth: Lu.target * 4
    implicitHeight: sub !== "" ? Lu.row + Lu.space(2) : Lu.row
    width: implicitWidth
    height: implicitHeight
    opacity: enabled ? 1 : Lu.disabledOpacity
    z: focused ? 1 : 0

    FocusLift {
        id: lift
        focused: card.focused
        rested: card.rested
        reduced: Lu.reduced
        duration: Lu.cardMs
        easing: Lu.cardEasing
    }

    Rectangle {
        anchors.fill: parent
        radius: Lu.radius("control")
        color: card.focused ? Lu.wash("focus") : card.selected ? Lu.wash("selected") : Lu.wash("hover")
        border.width: 1
        border.color: card.focused ? Lu.edgeRaised : Lu.edge
        antialiasing: true
        scale: 1 + (Lu.buttonScale - 1) * lift.value
        transformOrigin: Item.Center
    }
    LuHover { on: ptr.hovered && !card.focused; radius: Lu.radius("control") }
    LuIcon {
        id: glyph
        visible: card.icon !== ""
        x: Lu.space(5)
        anchors.verticalCenter: parent.verticalCenter
        name: card.icon
        size: Math.round(Lu.target * 0.5)
        color: card.selected ? Lu.accent : Lu.ink2
    }
    Column {
        x: glyph.visible ? glyph.x + glyph.width + Lu.space(4) : Lu.space(5)
        width: parent.width - x - Lu.space(5) - (check.visible ? check.width + Lu.space(2) : 0)
        anchors.verticalCenter: parent.verticalCenter
        LuText { width: parent.width; role: "label"; text: card.title; elide: Text.ElideRight }
        LuText { width: parent.width; role: "caption"; text: card.sub; color: Lu.ink2; visible: card.sub !== ""; elide: Text.ElideRight }
    }
    LuIcon {
        id: check
        visible: card.selected
        x: parent.width - width - Lu.space(4)
        y: Lu.space(3)
        name: "check"
        size: Math.round(Lu.target * 0.34)
        color: Lu.accent
    }
    LuPointer {
        id: ptr
        anchors.fill: parent
        onClicked: { card.pointerFocus(); card.activated() }
    }
}
