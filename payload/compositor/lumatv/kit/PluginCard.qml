import QtQuick 2.4
import "."

// One card a plugin put (PluginHub.cards): a facts card (a few labelled lines), an event card (something that is or will
// be) or a media card (what is playing, with its position). `size` decides the width, `role` the colour of the icon tile
// (neutral, accent, info, positive, warning, danger, live: the same words as the kit's colours). The inline actions
// are buttons below; the host owns focus (`focused` for the card, `actionIndex` for one of its buttons) and hears
// `activated()` for the card's `open` route and `actionPicked(action)` for a button. A card that has expired is
// removed by the helper, not hidden here.
Item {
    id: card
    property var spec: ({ content: {}, actions: [], size: "half" })
    property bool focused: false
    property int actionIndex: -1
    property bool rested: true
    signal activated()
    signal actionPicked(var action)
    signal hoverFocus()

    readonly property var c: spec.content || ({})
    readonly property string type: c.type || "facts"
    readonly property var facts: c.facts || []
    readonly property var actions: spec.actions || []
    readonly property color tone: {
        var r = c.role || "neutral"
        return r === "accent" ? Lu.accent : r === "info" ? Lu.info : r === "positive" ? Lu.positive : r === "warning" ? Lu.warning
             : r === "danger" ? Lu.danger : r === "live" ? Lu.live : Lu.ink2
    }
    readonly property int pad: Lu.space(5)
    implicitWidth: spec.size === "full" ? 720 : 352
    implicitHeight: pad + 56 + Lu.space(3) + body.height + (actions.length ? Lu.space(3) + Lu.target : 0) + pad
    width: implicitWidth
    height: implicitHeight
    z: focused ? 1 : 0

    FocusLift { id: lift; focused: card.focused && card.actionIndex < 0; rested: card.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
    scale: 1 + (Lu.buttonScale - 1) * lift.value
    transformOrigin: Item.Center

    Rectangle {
        anchors.fill: parent
        radius: Lu.radius("card")
        color: card.focused && card.actionIndex < 0 ? Lu.wash("focus") : Lu.wash("hover")
        border.width: 1
        border.color: card.focused && card.actionIndex < 0 ? Lu.edgeRaised : Lu.edge
        antialiasing: true
    }
    MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: card.hoverFocus(); onClicked: card.activated() }

    Row {
        x: card.pad; y: card.pad; spacing: Lu.space(4); height: 56
        Rectangle {
            width: 56; height: 56; radius: Lu.radius("tile"); color: Qt.rgba(card.tone.r, card.tone.g, card.tone.b, 0.22)
            border.width: 1; border.color: Qt.rgba(card.tone.r, card.tone.g, card.tone.b, 0.5); antialiasing: true
            LuIcon { anchors.centerIn: parent; name: PluginHub.icon(card.c.icon); size: 28; color: card.tone }
        }
        Column {
            anchors.verticalCenter: parent.verticalCenter; width: card.width - 2 * card.pad - 56 - Lu.space(4)
            LuText { width: parent.width; role: "label"; lineHeight: 1.0; text: card.c.title || ""; elide: Text.ElideRight }
            LuText { width: parent.width; role: "caption"; lineHeight: 1.0; color: Lu.ink2; elide: Text.ElideRight
                     text: card.type === "media" ? (card.c.subtitle || "") : card.type === "event" ? (card.c.detail || "") : (spec.plugin || "") }
        }
    }
    Column {
        id: body
        x: card.pad; y: card.pad + 56 + Lu.space(3); width: card.width - 2 * card.pad; spacing: Lu.space(1)
        Repeater {
            model: card.type === "facts" ? card.facts : []
            delegate: Row {
                width: parent.width
                LuText { width: parent.width * 0.45; role: "caption"; color: Lu.ink2; elide: Text.ElideRight; text: modelData.label }
                LuText { width: parent.width * 0.55; role: "caption"; elide: Text.ElideRight; horizontalAlignment: Text.AlignRight; text: modelData.value }
            }
        }
        LuText { visible: card.type === "event" && !!card.c.at; role: "caption"; color: Lu.ink2; text: card.c.at ? Qt.formatDateTime(new Date(card.c.at * 1000), "ddd h:mm AP") : "" }
        Rectangle {
            visible: card.type === "media" && !!card.c.duration_secs
            width: parent.width; height: 4; radius: 2; color: Lu.trackOff
            Rectangle { width: parent.width * Math.min(1, (card.c.position_secs || 0) / Math.max(1, card.c.duration_secs || 1)); height: parent.height; radius: 2; color: card.tone }
        }
        LuText { visible: card.type === "media"; role: "caption"; color: Lu.ink2; text: card.c.state === "playing" ? "Playing" : card.c.state === "paused" ? "Paused" : "Stopped" }
    }
    Row {
        visible: card.actions.length > 0
        x: card.pad; y: card.height - card.pad - Lu.target; spacing: Lu.space(3)
        Repeater {
            model: card.actions
            delegate: LuButton {
                text: modelData.label
                focused: card.focused && card.actionIndex === index
                rested: card.rested
                onHoverFocus: card.hoverFocus()
                onActivated: card.actionPicked(modelData)
            }
        }
    }
}
