import QtQuick 2.4
import "."

// The rounded square an icon sits in, in a row, a rail or a menu: a quiet wash at rest, the accent
// when `accent` is true (the current section, a switch that is on) and a dim ink glyph when the row
// it belongs to is not available. One rect and one icon, no shadow, no layer.
Item {
    id: tile
    property string icon: ""
    property bool accent: false
    property int size: Math.round(Lu.target * 0.75)
    property int glyph: Math.round(size * 0.55)
    property real radiusScale: 0.32
    width: size
    height: size

    Rectangle {
        anchors.fill: parent
        radius: Math.round(tile.size * tile.radiusScale)
        color: tile.accent ? Lu.accent : Lu.wash("selected")
        border.width: tile.accent ? 0 : 1
        border.color: Lu.edge
        antialiasing: true
    }
    LuIcon {
        anchors.centerIn: parent
        name: tile.icon
        size: tile.glyph
        color: tile.accent ? Lu.accentInk : Lu.ink
    }
}
