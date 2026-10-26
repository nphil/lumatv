import QtQuick 2.4
import "."

// The help sheet: what a row's info mark (LuInfo) opens. One reading-grade card (the overlay fill over a backdrop that
// hides the rows, never glass, so no word of the page reads through it) with the row's name and one or two plain
// sentences. It sits just under the row it is about, or just above it when there is no room below, inside `bounds`, and
// it does not move the focus: the row keeps it. Any key closes it (Back and OK included; the host's `key()` sends them
// here first) and so does a click of the pointer anywhere. Nothing times it out: it is read at the reader's pace.
//
// A host builds ONE of these, above its panes, and shows it with `show(title, text, anchorRect, boundsRect)`, both
// rectangles in this item's own coordinates. Every surface that has rows with a `help` uses it the same way.
Item {
    id: sheet
    property bool shown: false
    property string title: ""
    property string text: ""
    property rect anchor: Qt.rect(0, 0, 0, 0)       // the row it is about
    property rect bounds: Qt.rect(0, 0, 0, 0)       // where the card may sit
    readonly property int gap: Lu.space(2)
    readonly property int edge: Lu.space(4)
    readonly property int pad: Lu.space(6)
    readonly property int cardWidth: Math.min(Lu.target * 12, Math.max(0, bounds.width - 2 * edge))
    readonly property real below: anchor.y + anchor.height + gap
    readonly property bool fitsBelow: below + card.height <= bounds.y + bounds.height - edge
    signal closed()

    function show(t, body, a, b) {
        title = t
        text = body
        anchor = a
        bounds = b
        shown = true
    }
    function hide() { if (shown) { shown = false; closed() } }

    anchors.fill: parent
    visible: opacity > 0.001
    opacity: shown ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: Lu.reduced ? 120 : Lu.layerMs; easing.type: Lu.cardEasing } }

    // the page steps back (the same dim as a menu over Home): the row stays readable, the sheet is the one thing to read
    Rectangle { anchors.fill: parent; color: Lu.scrim }
    MouseArea { anchors.fill: parent; enabled: sheet.shown; hoverEnabled: true; onClicked: sheet.hide() }

    Item {
        id: card
        width: sheet.cardWidth
        height: body.height + 2 * Lu.space(5)
        x: Math.max(sheet.bounds.x + sheet.edge, Math.min(sheet.anchor.x + Lu.space(6), sheet.bounds.x + sheet.bounds.width - width - sheet.edge))
        y: Math.max(sheet.bounds.y + sheet.edge, sheet.fitsBelow ? sheet.below : sheet.anchor.y - height - sheet.gap)
        transform: Translate { y: Lu.reduced ? 0 : (1 - sheet.opacity) * (sheet.fitsBelow ? -Lu.space(3) : Lu.space(3)) }
        // the backdrop that hides the rows under it (Lucent scrim.hidesMin .97), then the overlay fill
        Rectangle { anchors.fill: parent; radius: Lu.radius("card"); color: Lu.scrimHide; antialiasing: true }
        Rectangle { anchors.fill: parent; radius: Lu.radius("card"); color: Lu.sheet; border.width: 1; border.color: Lu.edgeRaised; antialiasing: true }
        Column {
            id: body
            x: sheet.pad
            y: Lu.space(5)
            width: parent.width - 2 * sheet.pad
            spacing: Lu.space(2)
            Row {
                spacing: Lu.space(3)
                LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "info"; size: Math.round(Lu.target * 0.4); color: Lu.ink2 }
                LuText { anchors.verticalCenter: parent.verticalCenter; role: "label"; text: sheet.title }
            }
            LuText { width: parent.width; role: "body"; wrapMode: Text.WordWrap; text: sheet.text }
        }
    }
}
