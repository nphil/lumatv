import QtQuick 2.4
import "."

// The help mark (Nitin, 2026-10-25: a row shows its name and value only; what is not obvious sits behind one
// consistent affordance). A small "i" drawn after a row's name on every row that has an explanation: LuRow draws it
// when its `help` is set, and the host opens the explanation in a LuHelp. At rest it is a quiet glyph. While its row
// has the focus it grows into a pill that says how to open the help, so nobody has to remember the gesture: OK held
// (the dock's own "options" gesture), the Menu key (the gamepad's, the Magic Remote has none), or a click of the pointer on the
// mark. It never takes the focus: the row keeps it, and only the pointer presses the mark.
Item {
    id: mark
    property bool focused: false                // the row it belongs to has the focus
    property string hint: "Hold OK"
    signal asked()

    readonly property int glyph: Math.round(Lu.target * 0.4)
    implicitWidth: focused ? pill.width : glyph
    implicitHeight: focused ? Math.max(glyph, pill.height) : glyph
    width: implicitWidth
    height: implicitHeight

    Rectangle {
        id: pill
        visible: mark.focused
        anchors.verticalCenter: parent.verticalCenter
        height: Math.max(mark.glyph + Lu.space(2), pillRow.height + Lu.space(2))
        width: pillRow.width + Lu.space(4)
        x: 0
        radius: height / 2
        color: Lu.wash("selected")
        border.width: 1
        border.color: Lu.edgeRaised
        antialiasing: true
        Row {
            id: pillRow
            x: Lu.space(2)
            anchors.verticalCenter: parent.verticalCenter
            spacing: Lu.space(2)
            LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "info"; size: mark.glyph - Lu.space(1); color: Lu.ink }
            LuText { anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink; text: mark.hint }
        }
    }
    LuIcon {
        visible: !mark.focused
        anchors.centerIn: parent
        name: "info"
        size: mark.glyph
        color: Lu.ink2
    }
    // a pointer target of the profile's size, centred on the mark; the row's own click does not see it
    MouseArea {
        anchors.centerIn: parent
        width: Math.max(mark.width, Lu.target)
        height: Lu.target
        onClicked: mark.asked()
    }
}
