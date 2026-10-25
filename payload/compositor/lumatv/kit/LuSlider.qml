import QtQuick 2.4
import "."

// A slider: a quiet track, the accent fill to the value and an isolated thumb. The track is
// slim but the control is as tall as a target, so it is as easy to hit as any other. Left and
// right move it by `step` (adjust(-1), adjust(1)); like the toggle it sits in a LuRow.
Item {
    id: sl
    property real from: 0
    property real to: 1
    property real step: 0.1
    property real value: 0
    property bool active: false             // the row is in adjustment: the thumb grows
    signal moved(real value)

    readonly property real fraction: to > from ? Math.max(0, Math.min(1, (value - from) / (to - from))) : 0
    readonly property int thumbSize: Lu.space(7)

    width: Lu.target * 4
    height: Lu.target
    opacity: enabled ? 1 : Lu.disabledOpacity

    function adjust(dir) {
        var v = Math.round((value + dir * step) / step) * step
        v = Math.max(from, Math.min(to, Math.round(v * 1000) / 1000))
        if (v === value) return
        value = v
        moved(v)
    }

    Rectangle {
        id: track
        x: sl.thumbSize / 2
        width: sl.width - sl.thumbSize
        height: Lu.space(3)
        anchors.verticalCenter: parent.verticalCenter
        radius: height / 2
        color: Lu.trackOff
        antialiasing: true
        Rectangle {
            width: parent.width * sl.fraction
            height: parent.height
            radius: height / 2
            color: Lu.accent
            antialiasing: true
        }
    }
    Rectangle {
        width: sl.thumbSize
        height: width
        radius: width / 2
        scale: sl.active ? Lu.focus.buttonScale : 1
        x: track.x + track.width * sl.fraction - width / 2
        anchors.verticalCenter: parent.verticalCenter
        color: Lu.ink
        border.width: 1
        border.color: Lu.edgeRaised
        antialiasing: true
        Behavior on scale { enabled: !Lu.reduced; NumberAnimation { duration: Lu.pressMs } }
    }
}
