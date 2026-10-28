import QtQuick 2.4
import "."

// A short state tag: text, with a dot in a semantic colour when there is a tone. The meaning is
// in the text; the dot only helps. Passive: it never takes focus.
Item {
    id: chip
    property string text: ""
    property string tone: ""            // "", "positive", "warning", "danger", "info", "live"
    property bool danger: false         // a button-like tag that destroys something (Delete): the danger wash and ink, as the mock's .btn.danger.strong
    readonly property color toneColor: tone === "" ? "transparent" : Lu.role(tone)

    implicitWidth: row.width + 2 * Lu.space(4)
    implicitHeight: label.implicitHeight + 2 * Lu.space(2)
    width: implicitWidth
    height: implicitHeight

    Rectangle {
        anchors.fill: parent
        radius: Lu.radius("pill")
        color: chip.danger ? Qt.rgba(Lu.danger.r, Lu.danger.g, Lu.danger.b, 0.16) : Lu.wash("selected")
        border.width: 1
        border.color: chip.danger ? Qt.rgba(Lu.danger.r, Lu.danger.g, Lu.danger.b, 0.5) : Lu.edge
        antialiasing: true
    }
    Row {
        id: row
        x: Lu.space(4)
        anchors.verticalCenter: parent.verticalCenter
        spacing: Lu.space(2)
        Rectangle {
            visible: chip.tone !== ""
            width: Lu.space(2); height: width; radius: width / 2
            anchors.verticalCenter: parent.verticalCenter
            color: chip.toneColor
            antialiasing: true
        }
        LuText { id: label; role: "caption"; text: chip.text; color: chip.danger ? Lu.dangerOnWash : Lu.ink }
    }
}
