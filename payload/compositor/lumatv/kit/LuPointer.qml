import QtQuick 2.9
import "."

// The pointer's area over one control. Hover only shows (`hovered`, for a LuHover cue); the host's focus moves and the control acts
// on a click, never on entering. Every pointer area in the shell is this one, so no control can take the focus from the D-pad
// by being under the pointer. A move into the area reports without a position change, so entering reports too.
MouseArea {
    hoverEnabled: true
    readonly property bool hovered: containsMouse && Lu.pointerLive
    function seen() { var p = mapToItem(null, mouseX, mouseY); Lu.pointerAt(p.x, p.y) }
    onEntered: seen()
    onPositionChanged: seen()
}
