import QtQuick 2.4
import "."

// Two to five exclusive options in a quiet tray. The selected one carries a small accent mark and
// a light wash; the focused one the focus wash and lit edge, so the two read apart. Segments do
// not lift (the highlight and the halo are the cue), and the host owns focusedIndex (-1: none).
// Every segment is as wide as the widest label, measured as drawn, plus the pill's own air on each
// side, and never narrower than a target: a label is never cut or crammed against its pill. A tab bar
// (Lucent: destinations, labels and the current mark) has no tray: its labels read on the pane.
// Given a width to stay within (`fit`), segments that do not fit scroll into view by whole
// segments, the focused (else the chosen) one always in it and a chevron where more are.
Item {
    id: seg
    property var options: []
    property int current: 0
    property int focusedIndex: -1
    signal picked(int index)
    signal hoverFocus(int index)

    property string textRole: "label"        // "caption" for a dense row
    property bool tray: true                 // the quiet tray under the options; off for a tab bar, whose labels sit on the pane itself
    property int fit: 0                      // the width to stay within (0: as wide as the segments need)
    readonly property int pad: Lu.space(4)   // air between a label and its pill's edge
    property int widest: 0                   // the widest label as drawn; the delegates keep it
    function measure() {
        var w = 0
        for (var i = 0; i < rep.count; i++) { var it = rep.itemAt(i); if (it) w = Math.max(w, it.labelWidth) }
        widest = w
    }
    property int segmentWidth: Math.max(Lu.target, Math.ceil(widest) + 2 * pad + 2 * Lu.space(1))
    readonly property int need: options.length * segmentWidth
    readonly property bool scrolls: fit > 0 && need > fit
    readonly property int inView: scrolls ? Math.max(1, Math.floor(fit / segmentWidth)) : options.length
    readonly property int at: focusedIndex >= 0 ? focusedIndex : Math.max(0, current)
    property int first: 0
    function follow() { if (at < first) first = at; else if (at >= first + inView) first = at - inView + 1; if (first > options.length - inView) first = Math.max(0, options.length - inView) }
    onAtChanged: follow()
    onInViewChanged: follow()
    Component.onCompleted: follow()
    readonly property bool moreLeft: scrolls && first > 0
    readonly property bool moreRight: scrolls && first + inView < options.length

    // The plate under the chosen or focused label. A light wash on the dark glass lifts the background toward the white
    // label (3.5:1 over a bright wallpaper, 4.5 needed), so on dark it is a shade instead, and on light a tint, away from the label either way.
    function plate(kind) { return Lu.isLight ? Qt.rgba(1, 1, 1, kind === "focus" ? 0.34 : 0.2) : Qt.rgba(0, 0, 0, kind === "focus" ? 0.28 : 0.16) }

    implicitWidth: scrolls ? fit : need
    implicitHeight: Lu.target
    width: implicitWidth
    height: implicitHeight
    opacity: enabled || (parent && !parent.enabled) ? 1 : Lu.disabledOpacity   // a disabled row already dims what it holds

    Rectangle {
        visible: seg.tray
        anchors.fill: parent
        radius: Lu.radius("control")
        color: Lu.isLight ? Lu.trackOff : Qt.rgba(0, 0, 0, 0.22)
        antialiasing: true
    }
    LuHalo {
        x: (seg.focusedIndex - seg.first) * seg.segmentWidth + seg.segmentWidth / 2 - width / 2
        y: seg.height / 2 - height / 2
        width: seg.segmentWidth * 1.8
        height: seg.height * 3
        z: -1
        on: seg.focusedIndex >= 0
        Behavior on x { enabled: !Lu.reduced; NumberAnimation { duration: Lu.focusMs; easing.type: Lu.focusEasing } }
    }
    Item {
        anchors.fill: parent
        clip: seg.scrolls
        Repeater {
            id: rep
            model: seg.options
            delegate: Item {
                readonly property real labelWidth: label.implicitWidth
                onLabelWidthChanged: seg.measure()
                Component.onCompleted: seg.measure()
                visible: !seg.scrolls || (index >= seg.first && index < seg.first + seg.inView)
                x: (index - seg.first) * seg.segmentWidth
                width: seg.segmentWidth
                height: seg.height
                Rectangle {
                    anchors.fill: parent
                    anchors.margins: Lu.space(1)
                    radius: Lu.radius("control") - Lu.space(1)
                    color: index === seg.focusedIndex ? seg.plate("focus") : index === seg.current ? seg.plate("selected") : "transparent"
                    border.width: index === seg.focusedIndex ? 1 : 0
                    border.color: Lu.edgeRaised
                    antialiasing: true
                }
                LuText {
                    id: label
                    anchors.centerIn: parent
                    role: seg.textRole
                    text: modelData
                    color: Lu.ink            // every label in the reading ink: ink2 read under 3:1 on the tray; the wash and the mark say which is chosen
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
    // more segments that way: a chevron at the edge, over the last segment's air
    LuIcon { visible: seg.moreLeft; x: Lu.space(1); anchors.verticalCenter: parent.verticalCenter; name: "chevron-left"; size: Lu.space(5); color: Lu.ink3 }
    LuIcon { visible: seg.moreRight; x: parent.width - width - Lu.space(1); anchors.verticalCenter: parent.verticalCenter; name: "chevron-right"; size: Lu.space(5); color: Lu.ink3 }
}
