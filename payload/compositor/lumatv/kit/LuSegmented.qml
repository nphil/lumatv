import QtQuick 2.4
import "."

// Two to five exclusive options in a quiet tray. The selected one carries a small accent mark and
// a light wash; the focused one the focus wash and lit edge, so the two read apart. Segments do
// not lift (the highlight and the halo are the cue), and the host owns focusedIndex (-1: none).
Item {
    id: seg
    property var options: []
    property int current: 0
    property int focusedIndex: -1
    signal picked(int index)
    signal hoverFocus(int index)

    property string textRole: "label"        // "caption" for a dense row
    readonly property int longestChars: {
        var n = 0
        for (var i = 0; i < options.length; i++) n = Math.max(n, String(options[i]).length)
        return n
    }
    // Wide enough for the longest option at the role's size (Manrope averages 0.56 em a character), never narrower than a target.
    property int segmentWidth: Math.max(Math.round(Lu.target * 1.4), Math.round(longestChars * Lu.type(textRole).size * 0.56) + Lu.space(7))

    implicitWidth: options.length * segmentWidth
    implicitHeight: Lu.target
    width: implicitWidth
    height: implicitHeight
    opacity: enabled ? 1 : Lu.disabledOpacity

    Rectangle {
        anchors.fill: parent
        radius: Lu.radius("control")
        color: Lu.trackOff
        antialiasing: true
    }
    LuHalo {
        x: seg.focusedIndex * seg.segmentWidth + seg.segmentWidth / 2 - width / 2
        y: seg.height / 2 - height / 2
        width: seg.segmentWidth * 1.8
        height: seg.height * 3
        z: -1
        on: seg.focusedIndex >= 0
        Behavior on x { enabled: !Lu.reduced; NumberAnimation { duration: Lu.focusMs; easing.type: Lu.focusEasing } }
    }
    Repeater {
        model: seg.options
        delegate: Item {
            x: index * seg.segmentWidth
            width: seg.segmentWidth
            height: seg.height
            Rectangle {
                anchors.fill: parent
                anchors.margins: Lu.space(1)
                radius: Lu.radius("control") - Lu.space(1)
                color: index === seg.focusedIndex ? Lu.wash("focus") : index === seg.current ? Lu.wash("selected") : "transparent"
                border.width: index === seg.focusedIndex ? 1 : 0
                border.color: Lu.edgeRaised
                antialiasing: true
            }
            LuText {
                anchors.centerIn: parent
                role: seg.textRole
                text: modelData
                color: index === seg.current || index === seg.focusedIndex ? Lu.ink : Lu.ink2
            }
            Rectangle {
                visible: index === seg.current
                width: Lu.space(5); height: Lu.space(1)
                radius: height / 2
                anchors.horizontalCenter: parent.horizontalCenter
                y: parent.height - height - Lu.space(2)
                color: Lu.accent
                antialiasing: true
            }
            MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                onEntered: seg.hoverFocus(index)
                onClicked: seg.picked(index)
            }
        }
    }
}
