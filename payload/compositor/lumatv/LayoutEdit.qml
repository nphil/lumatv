import QtQuick 2.4
import "kit"
import "LayoutGrid.js" as Grid

// The layout editor: Home's cards moved, sized, added and removed on the grid, with the D-pad or the pointer. It is a surface that stays
// built (LumaHome keeps it as a Resident) and is shown while `HomeLayout.editing`. The grid it edits is the layout simulator's (LayoutGrid.js):
// twelve columns by sixteen rows, a card is whole cells, and a change that would overlap another card is refused. Home draws the layout
// being edited as it changes (`preview`), so what is seen is what would be saved; this surface draws only what the editor adds: the cells
// where nothing is, the selection, a bar with what can be done, and what a refusal says.
//
// Keys, from the host's `key(dir)`: on the grid the arrows choose a card and OK picks it up; picked up, the arrows move it a cell and OK
// turns to its size (the arrows then move its bottom right corner) and OK again puts it down; Back puts it back where it was. Up from the
// top card, or the Menu key, goes to the bar; Down from the bar returns. Back with nothing picked up leaves, asking first if anything
// changed. Nothing here writes a file or draws Home: it asks (`preview`, `save`, `leave`, `ask`) and Home does it.
Item {
    id: edit
    anchors.fill: parent

    signal preview(var layout)              // Home draws this layout now
    signal save(var layout)                 // write it; Home leaves the editor
    signal leave(var layout)                // leave without saving; `layout` is what Home had when the editor opened
    signal ask(var spec)                    // a question for the confirm dialog: { title, answers, focus }

    property var orig: ({})                 // what Home was drawing when the editor opened
    property var base: ({})                 // the same on the editor's grid (sixteen rows): what a change is measured against
    property var work: ({})                 // the layout being edited
    property var history: []                // earlier `work`s, the newest last
    property string sel: ""                 // the chosen card
    property string mode: "select"          // "select", "move" (picked up), "resize"
    property var held: null                 // `work` when the card was picked up: Back puts it back
    property string zone: "grid"            // "grid", "bar", "add"
    property int barIndex: 0
    property int addIndex: 0
    property string toastText: ""
    property bool refused: false            // the last press was turned down: the selection flashes
    readonly property bool dirty: !Grid.same(work, base)
    readonly property var card: Grid.find(work, sel)
    readonly property bool canRemove: !!card && !Grid.isDock(card)

    // ---- open and close
    function start(id) {
        orig = HomeLayout.layout
        base = Grid.tidyLayout(Grid.toUniform(orig))
        work = base
        history = []
        held = null
        mode = "select"; zone = "grid"; barIndex = 0; addIndex = 0; toastText = ""
        var first = ""
        var order = Grid.order(work)
        if (Grid.find(work, id) && !Grid.find(work, id).hidden) first = id
        else if (order.length) first = order[0]
        else if (Grid.dockId(work)) first = Grid.dockId(work)
        sel = first
        preview(work)
    }
    function leaveNow() { leave(orig) }
    function saveNow() { if (dirty) save(work); else leave(orig) }
    // Back with nothing picked up
    function back() {
        if (zone === "add") { zone = "bar"; return }
        // the dialog's Back is its first answer, so that one is the one that loses nothing
        if (dirty) ask({ title: "Save changes?", answers: [{ id: "edit-keep", label: "Keep editing" }, { id: "edit-save", label: "Save" },
                                                        { id: "edit-discard", label: "Discard", danger: true }], focus: 0 })
        else leaveNow()
    }
    function answer(id) {
        if (id === "edit-save") saveNow()
        else if (id === "edit-discard") leaveNow()
    }

    // ---- changes
    function toast(text) { toastText = text; toastTimer.restart() }
    Timer { id: toastTimer; interval: 1800; onTriggered: edit.toastText = "" }
    function flash() { refused = true; flashTimer.restart() }
    Timer { id: flashTimer; interval: 450; onTriggered: edit.refused = false }
    function turnedDown(r) {
        if (r.why === "edge" || r.why === "missing") return
        flash()
        toast(r.why === "overlap" && r.blocker ? Grid.titleOf(r.blocker) + " is in the way" : r.why === "small" ? "Smallest size" : r.why === "full" ? "No room" : "Not here")
    }
    // a change that is one step of its own (add, remove, undo, reset): the layout before it is kept to go back to
    function step(next, id) {
        var h = history.slice()
        h.push(work)
        history = h
        work = next
        if (id !== undefined) sel = id
        preview(work)
    }
    // a change inside a pick-up: the pick-up is the step (kept when the card is put down)
    function change(r) {
        if (!r.ok) { turnedDown(r); return false }
        work = r.layout
        preview(work)
        return true
    }
    function pickUp() { if (card) { held = work; mode = "move" } }
    function putDown() {
        if (held && !Grid.same(held, work)) { var h = history.slice(); h.push(held); history = h }
        held = null; mode = "select"
    }
    function putBack() {
        if (held) { work = held; preview(work) }
        held = null; mode = "select"
    }
    function undo() {
        if (!history.length) return
        var h = history.slice()
        work = h.pop()
        history = h
        if (!Grid.find(work, sel)) sel = Grid.order(work).length ? Grid.order(work)[0] : Grid.dockId(work)
        preview(work)
    }
    function removeCard() {
        if (!canRemove) { toast("The dock stays"); return }
        var order = Grid.order(work), i = order.indexOf(sel)
        var r = Grid.remove(work, sel)
        if (!r.ok) { turnedDown(r); return }
        var rest = order.filter(function(x) { return x !== sel })
        step(r.layout, rest.length ? rest[Math.min(i, rest.length - 1)] : Grid.dockId(r.layout))
    }
    function addCard(type) {
        var r = Grid.add(work, type)
        zone = "grid"
        if (!r.ok) { turnedDown(r); return }
        held = work                                   // placed in the first free place and picked up, so it can be put where it is wanted; Back takes it away again
        work = r.layout
        sel = r.id
        mode = "move"
        preview(work)
    }

    // ---- choosing a card by the arrows: the nearest one in that direction (the dock counts)
    function neighbour(dir) {
        var dx = dir === "left" ? -1 : dir === "right" ? 1 : 0, dy = dir === "up" ? -1 : dir === "down" ? 1 : 0
        var from = Grid.find(work, sel)
        if (!from) return ""
        var fr = Grid.rectOf(work, from), fcx = fr.x + fr.w / 2, fcy = fr.y + fr.h / 2
        var best = "", bestScore = Infinity, w = work.widgets || []
        for (var i = 0; i < w.length; i++) {
            var e = w[i]
            if (e.id === sel || e.hidden) continue
            var r = Grid.rectOf(work, e), ddx = r.x + r.w / 2 - fcx, ddy = r.y + r.h / 2 - fcy
            if (dx !== 0 && ddx * dx <= 0) continue
            if (dy !== 0 && ddy * dy <= 0) continue
            var primary = dx !== 0 ? Math.abs(ddx) : Math.abs(ddy), secondary = dx !== 0 ? Math.abs(ddy) : Math.abs(ddx)
            var score = primary + secondary * 2
            if (score < bestScore) { bestScore = score; best = e.id }
        }
        return best
    }

    // ---- the bar
    readonly property var buttons: [
        { id: "add", label: "Add", icon: "plus", enabled: true },
        { id: "remove", label: "Remove", icon: "trash-2", enabled: canRemove },
        { id: "undo", label: "Undo", icon: "undo-2", enabled: history.length > 0 },
        { id: "cancel", label: "Cancel", icon: "x", enabled: true },
        { id: "save", label: "Save", icon: "check", enabled: true, primary: true }
    ]
    function nextEnabled(from, d) {
        for (var i = from + d; i >= 0 && i < buttons.length; i += d) if (buttons[i].enabled) return i
        return -1
    }
    function press(id) {
        if (id === "add") { zone = "add"; addIndex = 0 }
        else if (id === "remove") removeCard()
        else if (id === "undo") undo()
        else if (id === "cancel") back()
        else if (id === "save") saveNow()
    }
    readonly property var addable: Grid.ADDABLE

    // ---- the keys
    function key(dir) {
        if (zone === "add") return addKey(dir)
        if (zone === "bar") return barKey(dir)
        return gridKey(dir)
    }
    function menu() { if (mode === "select") { zone = "add"; addIndex = 0 } }
    function barKey(dir) {
        if (dir === "left") { var l = nextEnabled(barIndex, -1); if (l >= 0) barIndex = l }
        else if (dir === "right") { var r = nextEnabled(barIndex, 1); if (r >= 0) barIndex = r }
        else if (dir === "down") zone = "grid"
        else if (dir === "ok") press(buttons[barIndex].id)
        else if (dir === "back") back()
    }
    function addKey(dir) {
        if (dir === "up") addIndex = Math.max(0, addIndex - 1)
        else if (dir === "down") addIndex = Math.min(addable.length - 1, addIndex + 1)
        else if (dir === "ok") addCard(addable[addIndex])
        else if (dir === "back" || dir === "left") zone = "bar"
    }
    function gridKey(dir) {
        if (mode === "select") {
            if (dir === "ok") pickUp()
            else if (dir === "back") back()
            else if (dir === "up" && !Grid.hasBeside(work, sel, -1)) { zone = "bar"; barIndex = nearestButton(); if (!buttons[barIndex].enabled) barIndex = nextEnabled(barIndex, 1) }
            else { var n = neighbour(dir); if (n !== "") sel = n }
            return
        }
        if (dir === "back") { putBack(); return }
        if (mode === "move") {
            if (dir === "ok") mode = "resize"
            else change(Grid.move(work, sel, dir === "left" ? -1 : dir === "right" ? 1 : 0, dir === "up" ? -1 : dir === "down" ? 1 : 0))
            return
        }
        if (dir === "ok") putDown()
        else change(Grid.resize(work, sel, dir === "left" ? -1 : dir === "right" ? 1 : 0, dir === "up" ? -1 : dir === "down" ? 1 : 0))
    }
    // the button over the selected card
    function nearestButton() {
        if (!card) return buttons.length - 1
        var r = Grid.rectOf(work, card), cx = r.x + r.w / 2, best = 0, bestD = Infinity
        for (var i = 0; i < buttons.length; i++) {
            var b = buttonRep.itemAt(i)
            if (!b) continue
            var d = Math.abs(bar.x + barRow.x + b.x + b.width / 2 - cx)
            if (d < bestD) { bestD = d; best = i }
        }
        return best
    }

    // ---- what is drawn
    // the cells where no card is, one quad: the cells' rounded squares, here only where the cards (up to eight) are not
    function rectsOf() {
        var out = [], w = work.widgets || []
        for (var i = 0; i < w.length && out.length < 8; i++) {
            if (w[i].hidden) continue
            var r = Grid.rectOf(work, w[i])
            out.push(Qt.vector4d(r.x, r.y, r.w, r.h))
        }
        while (out.length < 8) out.push(Qt.vector4d(-10, -10, 0, 0))
        return out
    }
    readonly property var occupied: work.widgets ? rectsOf() : []
    ShaderEffect {
        id: cells
        x: a.x; y: a.y; width: a.w; height: a.h              // the grid's own rectangle: the quad draws nothing outside it
        visible: !!edit.work.grid
        readonly property var a: edit.work.grid ? Grid.area(edit.work) : ({ x: 0, y: 0, w: 1, h: 1 })
        readonly property var c: edit.work.grid ? Grid.cellSize(edit.work) : ({ w: 1, h: 1 })
        property vector4d area: Qt.vector4d(a.x, a.y, a.w, a.h)
        property vector4d cellv: Qt.vector4d(c.w, c.h, edit.work.grid ? edit.work.grid.gutter : 0, 8)
        property vector2d counts: Qt.vector2d(edit.work.grid ? edit.work.grid.cols : 1, edit.work.grid ? Grid.rowCount(edit.work) : 1)
        property vector4d r0: edit.occupied.length ? edit.occupied[0] : Qt.vector4d(-10, -10, 0, 0)
        property vector4d r1: edit.occupied.length ? edit.occupied[1] : Qt.vector4d(-10, -10, 0, 0)
        property vector4d r2: edit.occupied.length ? edit.occupied[2] : Qt.vector4d(-10, -10, 0, 0)
        property vector4d r3: edit.occupied.length ? edit.occupied[3] : Qt.vector4d(-10, -10, 0, 0)
        property vector4d r4: edit.occupied.length ? edit.occupied[4] : Qt.vector4d(-10, -10, 0, 0)
        property vector4d r5: edit.occupied.length ? edit.occupied[5] : Qt.vector4d(-10, -10, 0, 0)
        property vector4d r6: edit.occupied.length ? edit.occupied[6] : Qt.vector4d(-10, -10, 0, 0)
        property vector4d r7: edit.occupied.length ? edit.occupied[7] : Qt.vector4d(-10, -10, 0, 0)
        property color tint: Lu.ink
        property real fillA: Lu.isLight ? 0.06 : 0.05
        property real lineA: Lu.isLight ? 0.26 : 0.20
        fragmentShader: "
            uniform highp vec4 area;
            uniform highp vec4 cellv;
            uniform highp vec2 counts;
            uniform highp vec4 r0; uniform highp vec4 r1; uniform highp vec4 r2; uniform highp vec4 r3;
            uniform highp vec4 r4; uniform highp vec4 r5; uniform highp vec4 r6; uniform highp vec4 r7;
            uniform lowp vec4 tint;
            uniform lowp float fillA;
            uniform lowp float lineA;
            uniform lowp float qt_Opacity;
            varying highp vec2 qt_TexCoord0;
            highp float inside(highp vec4 r, highp vec2 p) {
                return step(r.x, p.x) * step(p.x, r.x + r.z) * step(r.y, p.y) * step(p.y, r.y + r.w);
            }
            void main() {
                highp vec2 q = qt_TexCoord0 * area.zw;
                highp vec2 p = q + area.xy;
                highp vec2 pitch = cellv.xy + vec2(cellv.z);
                highp vec2 idx = floor(q / pitch);
                highp vec2 loc = q - idx * pitch;
                highp float on = step(0.0, q.x) * step(0.0, q.y) * step(idx.x, counts.x - 1.0) * step(idx.y, counts.y - 1.0);
                highp vec2 hc = cellv.xy * 0.5;
                highp vec2 d2 = abs(loc - hc) - (hc - vec2(cellv.w));
                highp float d = length(max(d2, 0.0)) + min(max(d2.x, d2.y), 0.0) - cellv.w;
                highp float body = 1.0 - smoothstep(-0.75, 0.75, d);
                highp float line = 1.0 - smoothstep(0.0, 1.25, abs(d + 0.5));
                highp float taken = max(max(max(inside(r0, p), inside(r1, p)), max(inside(r2, p), inside(r3, p))),
                                        max(max(inside(r4, p), inside(r5, p)), max(inside(r6, p), inside(r7, p))));
                lowp float a = on * (1.0 - taken) * (fillA * body + lineA * line);
                gl_FragColor = vec4(tint.rgb * a, a) * qt_Opacity;
            }"
    }

    // the selected card
    readonly property var selRect: card ? Grid.rectOf(work, card) : ({ x: 0, y: 0, w: 0, h: 0 })
    readonly property color ringColor: refused ? Lu.danger : Lu.accent
    Rectangle {
        id: ring
        visible: !!edit.card
        x: edit.selRect.x - 6; y: edit.selRect.y - 6
        width: edit.selRect.w + 12; height: edit.selRect.h + 12
        radius: Lu.radius("card") + 6
        color: edit.mode === "select" ? "transparent" : Qt.rgba(edit.ringColor.r, edit.ringColor.g, edit.ringColor.b, 0.10)
        border.width: edit.mode === "select" ? 3 : 4
        border.color: edit.ringColor
        antialiasing: true
    }
    // where the arrows act in a resize: the corner
    Rectangle {
        visible: !!edit.card && edit.mode === "resize"
        x: edit.selRect.x + edit.selRect.w - width - 12; y: edit.selRect.y + edit.selRect.h - height - 12
        width: 36; height: 36; radius: 18
        color: edit.ringColor
        antialiasing: true
        LuIcon { anchors.centerIn: parent; size: 22; name: "chevron-right"; color: Lu.accentInk }
    }

    // ---- the bar, where the status bar is
    LuPane {
        id: bar
        x: 64; y: 50; width: 1792; height: 72
        cardRadius: Lu.radius("sheet") / 2
        LuText { id: heading; x: Lu.space(6); anchors.verticalCenter: parent.verticalCenter; role: "label"; text: "Edit Home" }
        LuText {
            x: heading.x + heading.width + Lu.space(6); anchors.verticalCenter: parent.verticalCenter
            width: Math.max(0, barRow.x - x - Lu.space(4))               // up to the buttons, whatever size the type has
            visible: width > 90
            elide: Text.ElideRight
            role: "caption"; color: Lu.ink2
            text: edit.mode === "select" ? "OK moves" : (edit.card ? edit.card.span + " \u00d7 " + edit.card.rows + " \u00b7 " : "") + (edit.mode === "move" ? "OK sizes \u00b7 Back undoes" : "OK done \u00b7 Back undoes")
        }
        Row {
            id: barRow
            x: parent.width - width - Lu.space(4)
            anchors.verticalCenter: parent.verticalCenter
            spacing: Lu.space(3)
            Repeater {
                id: buttonRep
                model: edit.buttons
                delegate: LuButton {
                    text: modelData.label
                    icon: modelData.icon
                    enabled: modelData.enabled
                    primary: modelData.primary === true
                    danger: modelData.danger === true
                    focused: edit.zone === "bar" && edit.barIndex === index
                    onActivated: { edit.barIndex = index; edit.press(modelData.id) }
                    onHoverFocus: { if (edit.mode === "select" && modelData.enabled) { edit.zone = "bar"; edit.barIndex = index } }
                }
            }
        }
    }

    // ---- the add list under the Add button
    LuPane {
        id: addPane
        visible: edit.zone === "add"
        x: bar.x + barRow.x + (buttonRep.itemAt(0) ? buttonRep.itemAt(0).x : 0)
        y: bar.y + bar.height + Lu.space(2)
        width: 380
        height: edit.addable.length * Lu.row + 2 * Lu.space(3)
        cardRadius: Lu.radius("sheet") / 2
        Column {
            x: Lu.space(2); y: Lu.space(3)
            width: parent.width - 2 * Lu.space(2)
            Repeater {
                model: edit.addable
                delegate: LuRow {
                    width: parent.width
                    icon: Grid.TYPES[modelData].icon
                    label: Grid.TYPES[modelData].title
                    value: Grid.hiddenOf(edit.work, modelData) ? "Hidden" : ""
                    focused: edit.zone === "add" && edit.addIndex === index
                    onActivated: { edit.addIndex = index; edit.addCard(modelData) }
                    onHoverFocus: edit.addIndex = index
                }
            }
        }
    }

    // ---- what a refusal says
    LuChip {
        id: toastChip
        x: Math.round((parent.width - width) / 2); y: 1080 - 54 - height - 8
        opacity: edit.toastText !== "" ? 1 : 0
        visible: opacity > 0.01
        text: edit.toastText
        tone: "warning"
        Behavior on opacity { enabled: !Lu.reduced; NumberAnimation { duration: 120 } }
    }

    // ---- the pointer: over a card it chooses it, dragging moves it, and the corner of the chosen card sizes it
    function cardAt(x, y) {
        var w = work.widgets || []
        for (var i = w.length - 1; i >= 0; i--) {
            if (w[i].hidden) continue
            var r = Grid.rectOf(work, w[i])
            if (x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h) return w[i].id
        }
        return ""
    }
    property var drag: null                 // { x, y, kind: "move" | "size", to: {col,row} | {span,rows} | null }
    MouseArea {
        id: pointer
        anchors.fill: parent
        hoverEnabled: true
        z: -1
        onPositionChanged: {
            if (edit.drag) {
                var d = edit.drag, dx = mouse.x - d.x, dy = mouse.y - d.y
                if (!d.on && Math.abs(dx) + Math.abs(dy) < 10) return
                d.on = true
                d.to = d.kind === "move" ? Grid.snapMove(edit.work, edit.sel, dx, dy) : Grid.snapResize(edit.work, edit.sel, dx, dy)
                edit.drag = d
                dragGhost.update()
            } else if (edit.mode === "select") {
                var id = edit.cardAt(mouse.x, mouse.y)
                if (id !== "" && id !== edit.sel) { edit.sel = id; edit.zone = "grid" }
            }
        }
        onPressed: {
            var id = edit.cardAt(mouse.x, mouse.y)
            if (edit.zone === "add") { edit.zone = "bar"; return }
            if (id === "") return
            if (edit.mode !== "select") edit.putDown()
            edit.sel = id; edit.zone = "grid"
            edit.drag = { x: mouse.x, y: mouse.y, kind: "move", on: false, to: null }
        }
        onReleased: edit.finishDrag()
        onCanceled: edit.drag = null
    }
    function finishDrag() {
        var d = drag
        drag = null
        if (!d || !d.on || !d.to) return
        var r = Grid.change(work, sel, d.to)
        if (r.ok) step(r.layout); else turnedDown(r)
    }
    // the corner handle of the chosen card
    MouseArea {
        visible: !!edit.card
        x: edit.selRect.x + edit.selRect.w - 30; y: edit.selRect.y + edit.selRect.h - 30
        width: 44; height: 44
        hoverEnabled: true
        onPressed: { edit.drag = { x: mouse.x + x, y: mouse.y + y, kind: "size", on: false, to: null } }
        onPositionChanged: {
            if (!edit.drag) return
            var d = edit.drag, dx = mouse.x + x - d.x, dy = mouse.y + y - d.y
            if (!d.on && Math.abs(dx) + Math.abs(dy) < 10) return
            d.on = true
            d.to = Grid.snapResize(edit.work, edit.sel, dx, dy)
            edit.drag = d
            dragGhost.update()
        }
        onReleased: edit.finishDrag()
        onCanceled: edit.drag = null
        Rectangle {
            anchors.centerIn: parent
            visible: edit.mode === "select"
            width: 24; height: 24; radius: 12
            color: Lu.accent; opacity: 0.9
            antialiasing: true
        }
    }
    // where a drag would put the card, red when it cannot go there
    Rectangle {
        id: dragGhost
        property var rect: null
        property bool ok: true
        function update() {
            var d = edit.drag, c = edit.card
            if (!d || !d.on || !d.to || !c) { rect = null; return }
            var next = Grid.change(edit.work, edit.sel, d.to)
            ok = next.ok || next.why === "edge"
            rect = Grid.rectOf(edit.work, { col: d.to.col !== undefined ? d.to.col : c.col, row: d.to.row !== undefined ? d.to.row : c.row,
                                              span: d.to.span !== undefined ? d.to.span : c.span, rows: d.to.rows !== undefined ? d.to.rows : c.rows })
        }
        visible: !!rect && !!edit.drag && edit.drag.on
        x: rect ? rect.x : 0; y: rect ? rect.y : 0; width: rect ? rect.w : 0; height: rect ? rect.h : 0
        radius: Lu.radius("card")
        color: "transparent"
        border.width: 3
        border.color: ok ? Lu.accent : Lu.danger
        antialiasing: true
    }
}
