import QtQuick 2.4
import "../kit"

// Edit mode of the quick panel (docs/mocks/29-qp-edit): the sheet's own place shows "Edit tiles" and one row per tile with what moves it
// and what removes it, then "Add a tile". It is entered by a held OK on a tile (that tile is picked up at once) and left with Back.
//   Up and Down choose a tile, or "Add a tile" at the end; Right reaches the remove button of the row and Left comes back
//   OK on a tile picks it up (it is raised and the up and down marks light); Up and Down then move the tile; OK drops it; Back
//   puts it back where it was
//   OK on the remove button takes the tile off the panel (it stays in "Add a tile"); OK on "Add a tile" opens that list
// The mock's two small buttons per row are the same moves for a pointer: a click on one moves the tile a place, a click on the cross
// removes it. Nothing here writes the theme: every change is `reorder(ids)`, the host keeps it (QuickArrange.js), and `rows` comes back
// with the new order.
Item {
    id: ed
    property var rows: []                      // the tiles' descriptors as the panel shows them, in order (id, icon, label, detail)
    property int index: 0                      // 0 .. rows.length - 1 a tile, rows.length "Add a tile"
    property bool grab: false                  // the tile at `index` is picked up
    property bool onRemove: false              // the focus is on the row's remove button
    property bool rested: true
    property bool instant: false
    readonly property int addIndex: rows.length
    property var undo: null                    // the ids as they were when the tile was picked up
    signal reorder(var ids, bool keep)         // `keep`: the order is final (a tile dropped, removed, added), write it
    signal addRequested()
    signal done()

    function ev(x) { return typeof x === "function" ? x() : x }
    function ids() { return rows.map(function(r) { return r.id }) }
    function begin(i) { index = Math.max(0, Math.min(rows.length - 1, i)); onRemove = false; grab = rows.length > 0; undo = ids() }
    function enter() { index = 0; grab = false; onRemove = false; undo = null }
    function swap(dir) {
        var j = index + dir
        if (j < 0 || j >= rows.length) return
        var order = ids()
        var t = order[index]; order[index] = order[j]; order[j] = t
        index = j
        reorder(order, false)
    }
    function remove(i) {
        var order = ids().filter(function(x, k) { return k !== i })
        grab = false
        onRemove = false
        reorder(order, true)
        index = Math.min(i, order.length)              // the next tile, or "Add a tile" when it was the last
    }
    function key(dir) {
        if (grab) {
            if (dir === "up") swap(-1)
            else if (dir === "down") swap(1)
            else if (dir === "ok") { grab = false; reorder(ids(), true) }
            else if (dir === "back") {
                // the tile goes back where it was and keeps the focus
                var was = undo, id = rows[index] ? rows[index].id : ""
                grab = false
                if (was) { reorder(was, true); index = Math.max(0, was.indexOf(id)) }
            }
        } else if (dir === "up") { index = Math.max(0, index - 1); onRemove = false }
        else if (dir === "down") { index = Math.min(addIndex, index + 1); if (index === addIndex) onRemove = false }
        else if (dir === "right") { if (index < addIndex) onRemove = true }
        else if (dir === "left") onRemove = false
        else if (dir === "ok") {
            if (index === addIndex) addRequested()
            else if (onRemove) remove(index)
            else begin(index)
        } else if (dir === "back") done()
        rested = false
        restTimer.restart()
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: ed.rested = true }

    // a row is as tall as its two lines need, so a large text size grows every row alike and none is cut
    readonly property int rowH: Math.max(Math.round(Lu.row * 0.95), probeLabel.height + probeCaption.height + Lu.space(4))
    LuText { id: probeLabel; visible: false; role: "label"; text: "Mg" }
    LuText { id: probeCaption; visible: false; role: "caption"; text: "Mg" }
    Flickable {
        id: view
        anchors.fill: parent
        contentHeight: column.height
        interactive: false
        clip: true
        // the focused row is always inside: the list moves by rows when it does not fit
        readonly property real focusTop: ed.index * (ed.rowH + Lu.space(1))
        onFocusTopChanged: {
            if (focusTop < contentY) contentY = focusTop
            else if (focusTop + ed.rowH > contentY + height) contentY = focusTop + ed.rowH - height
        }
        Column {
            id: column
            width: view.width
            spacing: Lu.space(1)
            Repeater {
                model: ed.rows.length
                delegate: Item {
                    id: tile
                    readonly property int ti: index
                    readonly property var r: ed.rows[index]
                    readonly property bool isFocus: index === ed.index
                    readonly property bool moving: isFocus && ed.grab
                    width: column.width
                    height: ed.rowH
                    z: moving ? 2 : isFocus ? 1 : 0
                    FocusLift { id: lift; focused: tile.isFocus; rested: ed.rested; reduced: Lu.reduced || ed.instant; duration: Lu.cardMs; easing: Lu.cardEasing }
                    LuHalo { anchors.centerIn: parent; width: parent.width * 1.2; height: parent.height * 2.2; z: -1; on: tile.moving; instant: ed.instant }
                    Rectangle {
                        anchors.fill: parent
                        radius: Lu.radius("row")
                        color: tile.moving ? Lu.glassRaised : tile.isFocus ? Lu.wash("focus") : "transparent"
                        border.width: tile.isFocus ? 1 : 0
                        border.color: Lu.edgeRaised
                        antialiasing: true
                        scale: 1 + (Lu.rowScale - 1) * (tile.moving ? 1 : lift.value)
                    }
                    LuTile { x: Lu.space(3); anchors.verticalCenter: parent.verticalCenter; icon: tile.r ? String(ed.ev(tile.r.icon) || "") : "" }
                    Column {
                        x: Lu.space(3) + Math.round(Lu.target * 0.75) + Lu.space(4)
                        anchors.verticalCenter: parent.verticalCenter
                        width: grip.x - x - Lu.space(3)
                        LuText { width: parent.width; role: "label"; elide: Text.ElideRight; text: tile.r ? String(ed.ev(tile.r.label) || "") : "" }
                        LuText { width: parent.width; visible: text !== ""; role: "caption"; color: Lu.ink2; elide: Text.ElideRight; text: tile.r ? String(ed.ev(tile.r.detail) || "") : "" }
                    }
                    Row {
                        id: grip
                        x: parent.width - width - Lu.space(3)
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: Lu.space(1)
                        Repeater {
                            model: [{ g: "chevron-left", turn: 90 }, { g: "chevron-right", turn: 90 }, { g: "x", turn: 0 }]
                            delegate: Rectangle {
                                readonly property bool isRemove: index === 2
                                readonly property bool lit: isRemove ? tile.isFocus && ed.onRemove : tile.moving
                                width: Math.round(Lu.target * 0.75); height: width
                                radius: Lu.radius("control")
                                color: lit ? Lu.accent : Lu.wash("selected")
                                border.width: 1
                                border.color: lit ? "transparent" : Lu.edge
                                antialiasing: true
                                LuIcon { anchors.centerIn: parent; rotation: modelData.turn; name: modelData.g; size: Math.round(parent.width * 0.5); color: parent.lit ? Lu.accentInk : Lu.ink2 }
                                MouseArea {
                                    anchors.fill: parent
                                    onClicked: { ed.index = tile.ti; ed.grab = false; if (parent.isRemove) ed.remove(tile.ti); else ed.moveBy(tile.ti, index === 0 ? -1 : 1) }
                                }
                            }
                        }
                    }
                    MouseArea { anchors.fill: parent; z: -1; hoverEnabled: true; onEntered: if (!ed.grab) ed.index = tile.ti }
                }
            }
            // the way to put one back
            Item {
                id: addRow
                width: column.width
                height: ed.rowH
                readonly property bool isFocus: ed.index === ed.addIndex
                Rectangle {
                    anchors.fill: parent
                    radius: Lu.radius("row")
                    color: addRow.isFocus ? Lu.wash("focus") : "transparent"
                    border.width: 1
                    border.color: addRow.isFocus ? Lu.edgeRaised : Lu.edge
                    antialiasing: true
                }
                LuIcon { x: Lu.space(4); anchors.verticalCenter: parent.verticalCenter; name: "plus"; size: 28; color: Lu.ink2 }
                LuText { x: Lu.space(4) + 28 + Lu.space(3); anchors.verticalCenter: parent.verticalCenter; role: "label"; text: "Add a tile" }
                MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: if (!ed.grab) ed.index = ed.addIndex; onClicked: { ed.index = ed.addIndex; ed.addRequested() } }
            }
        }
    }
    // a pointer's click on a move mark: the tile goes one place and keeps the focus
    function moveBy(i, dir) {
        var j = i + dir
        if (j < 0 || j >= rows.length) return
        var order = ids()
        var t = order[i]; order[i] = order[j]; order[j] = t
        index = j
        reorder(order, true)
    }
}
