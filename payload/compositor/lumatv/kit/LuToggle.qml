import QtQuick 2.4
import "."

// A switch: a pill track and a thumb that slides. On is the accent track with the thumb in the ink
// that reads on it; off is the quiet track. The position of the thumb says the state, not the
// colour. It has no focus of its own: it sits in a LuRow, which is what the D-pad lands on.
Item {
    id: sw
    property bool checked: false
    readonly property int trackH: Math.round(Lu.target * 0.75)
    readonly property int pad: Lu.space(1)

    width: Math.round(trackH * 1.75)
    height: trackH
    opacity: enabled || (parent && !parent.enabled) ? 1 : Lu.disabledOpacity   // a disabled row already dims what it holds

    Rectangle {
        anchors.fill: parent
        radius: height / 2
        color: sw.checked ? Lu.accent : Lu.trackOff
        antialiasing: true
        Behavior on color { enabled: !Lu.reduced; ColorAnimation { duration: Lu.ms("labelMs") } }
    }
    Rectangle {
        id: thumb
        width: sw.trackH - 2 * sw.pad
        height: width
        radius: width / 2
        y: sw.pad
        x: sw.checked ? sw.width - width - sw.pad : sw.pad
        color: sw.checked ? Lu.accentInk : Lu.ink
        border.width: 1
        border.color: Lu.edgeRaised
        antialiasing: true
        Behavior on x { enabled: !Lu.reduced; NumberAnimation { duration: Lu.ms("labelMs"); easing.type: Lu.focusEasing } }
    }
}
