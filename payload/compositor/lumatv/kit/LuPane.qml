import QtQuick 2.4
import ".."
import "."

// A pane of the glass material: the card shader sampling the shared blurred wallpaper at the pane's own
// screen rectangle, with content in front. The rectangle is found from where the pane sits in the window, once
// the pane has a size and again when it is resized or `locate()` is called (a pane that slides calls it
// while it moves). A pane costs one quad, like the cards of Home.
Item {
    id: pane
    property real cardRadius: Lu.radius("sheet")
    property real focusMix: 0
    property real liftMix: 0
    property real sx: 0
    property real sy: 0
    default property alias content: slot.data

    function locate() {
        var p = pane.mapToItem(null, 0, 0)
        sx = p.x
        sy = p.y
    }
    onWidthChanged: locate()
    onHeightChanged: locate()
    onVisibleChanged: if (visible) locate()
    Component.onCompleted: locate()

    Glass {
        mat: Lu.material
        isLight: Lu.isLight
        cardRadius: pane.cardRadius
        focusMix: pane.focusMix
        liftMix: pane.liftMix
        backdrop: Lu.backdrop
        screenX: pane.sx
        screenY: pane.sy
        screenW: pane.width
        screenH: pane.height
    }
    Item { id: slot; anchors.fill: parent }
}
