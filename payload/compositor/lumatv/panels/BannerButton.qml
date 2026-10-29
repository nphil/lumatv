import QtQuick 2.4
import "../kit"

// The small pill button of the banners (docs/mocks/25-input-banner, 18-no-signal): an icon and a label on the quiet glass
// wash; focus is the raised glass and the lit edge at once and the lift after the focus has rested (the kit's rule).
// `dark` draws it in the dark material whatever the TV's material is: the no-signal screen is black in both.
Item {
    id: btn
    property string icon: ""
    property string text: ""
    property bool focused: false
    property bool rested: true
    property bool dark: false
    signal activated()
    signal pointerFocus()

    readonly property var mat: dark && Lu.theme && Lu.theme.dark ? Lu.theme.dark : Lu.material
    function tone(name, fallback) { var v = mat[name]; return typeof v === "string" ? Lu.asColor(v) : fallback }
    readonly property color ink: tone("ink", Lu.ink)
    readonly property color wash: Qt.rgba(ink.r, ink.g, ink.b, (Lu.states.selectedWash || 0))

    implicitWidth: row.width + 2 * Lu.space(6)
    implicitHeight: 52
    width: implicitWidth
    height: implicitHeight
    z: focused ? 1 : 0
    FocusLift { id: lift; focused: btn.focused; rested: btn.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
    Rectangle {
        anchors.fill: parent; radius: Lu.radius("pill"); antialiasing: true
        color: btn.focused ? btn.tone("glassFocus", Lu.glassRaised) : btn.wash
        border.width: btn.focused ? 1 : 0; border.color: btn.tone("glassFocusEdge", Lu.edgeRaised)
        scale: 1 + (Lu.buttonScale - 1) * lift.value; transformOrigin: Item.Center      // the pill lifts; the label stays put
    }
    LuHover { on: ptr.hovered && !btn.focused; radius: Lu.radius("pill") }
    Row {
        id: row
        anchors.centerIn: parent
        spacing: Lu.space(3)
        LuIcon { visible: btn.icon !== ""; anchors.verticalCenter: parent.verticalCenter; name: btn.icon; size: 28; color: btn.ink }
        LuText { anchors.verticalCenter: parent.verticalCenter; role: "label"; font.weight: Font.DemiBold; color: btn.ink; text: btn.text }
    }
    LuPointer { id: ptr; anchors.fill: parent; onClicked: { btn.pointerFocus(); btn.activated() } }
}
