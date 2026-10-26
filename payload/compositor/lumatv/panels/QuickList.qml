import QtQuick 2.4
import "../kit"

// The quick panel's rows. The descriptors are those of settings/PageList (`kind`, `id`, `icon`, `label`, `detail`, and per kind `options`
// and `index` (stepper), `value` (nudge, value), `on` (toggle), `button` (action), `enabled`, `visible`, `act(v)`, `open()`; any of them may be
// a function, which runs inside the binding that draws the row, so what it reads redraws it) and so are the keys: Up and Down move between the
// rows that can take the focus, Left and Right change a stepper (it wraps) or nudge a nudge, OK flips a toggle and presses a value or an
// action. The tiles of the plugins come through the same descriptors. What differs is the drawing, for a column about 430 px wide: a row
// that needs width for its control puts it on a line of its own (QuickTile), so words and values wrap and never lose a letter.
// The rows are built when the panel is; a plugin's tile coming or going changes `rows`, and nothing is created by a key.
Item {
    id: list
    property var host                                   // the panel: `zone` says whether the rows have the focus
    property var rows: []
    property var tabs: []                               // PageList's name for a tab bar; the quick panel has none, and the panel's tests find the list by it
    property int tab: 0
    property int stop: -2                               // a row index, -2 not yet placed
    property int seg: 0                                 // the focused stepper's own position
    property bool rested: true
    readonly property bool active: host ? host.zone === "page" : false
    property real offset: 0

    readonly property var kinds: ({ stepper: true, nudge: true, toggle: true, value: true, action: true })

    function ev(x) { return typeof x === "function" ? x() : x }
    function shown(i) { var r = rows[i]; return !!r && ev(r.visible) !== false }
    function focusable(i) {
        var r = rows[i]
        return shown(i) && kinds[r.kind] === true && ev(r.enabled) !== false
    }
    function first() { for (var i = 0; i < rows.length; i++) if (focusable(i)) return i; return -2 }
    function step(from, dir) {
        for (var i = from + dir; i >= 0 && i < rows.length; i += dir) if (focusable(i)) return i
        return -2
    }
    function count(i) { var o = ev(rows[i].options); return o ? o.length : 0 }
    function enter(i) {
        stop = i
        var r = i >= 0 ? rows[i] : null
        seg = r && r.kind === "stepper" ? Math.max(0, Math.min(count(i) - 1, ev(r.index) || 0)) : 0
    }
    // The panel calls this when it is brought up.
    function reset() { if (stop < 0 || !focusable(stop)) enter(first()) }
    // A pointer that lands on a row takes the focus as the D-pad would.
    function pointAt(i) { if (host) host.zone = "page"; if (focusable(i)) enter(i) }

    function change(i, value) {
        var r = rows[i]
        if (r && r.act) r.act(value)
    }
    function activate(i) {
        var r = rows[i]
        if (!r || !focusable(i)) return
        if (r.kind === "toggle") change(i, !ev(r.on))
        else if (r.kind === "value" || r.kind === "action") change(i)
        else if (r.kind === "nudge" && r.open) r.open()
    }

    // true: the list used the key. Up from the first row and Left from any row are not the list's.
    function key(dir) {
        if (stop === -2) { var f0 = first(); if (f0 >= 0) enter(f0) }
        if (stop < 0) return false
        var r = rows[stop]
        if (dir === "up") {
            var p = step(stop, -1)
            if (p < 0) return false
            enter(p)
            return true
        }
        if (dir === "down") {
            var n = step(stop, 1)
            if (n >= 0) enter(n)
            return true
        }
        if (dir === "left" || dir === "right") {
            var d = dir === "left" ? -1 : 1
            if (r.kind === "nudge") { change(stop, d); return true }          // the value is read back; Left and Right only say which way
            if (r.kind === "stepper") {
                var n2 = count(stop)
                if (n2 > 0) { seg = (seg + d + n2) % n2; change(stop, seg) }
                return true
            }
            return d > 0
        }
        if (dir === "ok") { activate(stop); return true }
        return false
    }

    // ---- scrolling: the list shows whole rows only. `offset` is always the top of a row; a row that does not fit whole in the window below it
    // is not drawn (a half row is a row cut off, which nobody can read), and the focused row is always inside. At the normal text size every
    // row fits and nothing scrolls; a plugin's tiles, or a large text size, make it a window that moves by rows. The rows move, never resize.
    function itemFor(i) { return rep.itemAt(i) }
    function inWindow(it) { return !!it && it.y >= offset - 0.5 && it.y + it.height <= offset + height + 0.5 }
    function reveal(i) {
        var it = itemFor(i)
        if (!it || it.height <= 0) return
        if (inWindow(it)) return
        if (it.y < offset) { offset = it.y; return }
        // below the window: start at the earliest row from which the focused one ends inside it
        var o = it.y
        for (var k = i - 1; k >= 0; k--) {
            var c = itemFor(k)
            if (!c || c.height <= 0) continue
            if (it.y + it.height - c.y > height + 0.5) break
            o = c.y
        }
        offset = o
    }
    onStopChanged: scrollTimer.restart()
    onRowsChanged: scrollTimer.restart()
    onHeightChanged: scrollTimer.restart()
    Timer {
        id: scrollTimer
        interval: 1
        onTriggered: {
            if (list.stop >= 0) list.reveal(list.stop)
            else list.offset = 0
        }
    }

    Item {
        anchors.fill: parent
        clip: true
        Column {
            id: column
            width: parent.width
            y: -list.offset
            spacing: Lu.space(1)
            Repeater {
                id: rep
                model: list.rows.length                 // an integer: a list model would copy each descriptor and drop its functions
                delegate: Item {
                    id: cell
                    property var r: list.rows[index]
                    property int ri: index
                    readonly property bool isFocus: list.active && list.stop === ri
                    visible: list.shown(ri)
                    width: column.width
                    height: visible && body.item ? body.item.height : 0
                    Loader {
                        id: body
                        width: cell.width
                        visible: list.inWindow(cell) || list.stop === cell.ri
                        sourceComponent: cell.r.kind === "stepper" ? cStepper : cell.r.kind === "nudge" ? cNudge : cell.r.kind === "toggle" ? cToggle
                                       : cell.r.kind === "value" ? cValue : cell.r.kind === "action" ? cAction : null
                    }

                    Component {
                        id: cStepper
                        QuickTile {
                            icon: list.ev(cell.r.icon) || ""
                            label: list.ev(cell.r.label) || ""; detail: list.ev(cell.r.detail) || ""
                            enabled: list.ev(cell.r.enabled) !== false
                            focused: cell.isFocus; rested: list.rested
                            onHoverFocus: list.pointAt(cell.ri)
                            below: QuickStep {
                                focused: cell.isFocus
                                text: { var o = list.ev(cell.r.options) || []; var i = list.ev(cell.r.index) || 0; return o[i] === undefined ? "" : o[i] }
                            }
                        }
                    }
                    Component {
                        id: cNudge
                        QuickTile {
                            icon: list.ev(cell.r.icon) || ""
                            label: list.ev(cell.r.label) || ""; detail: list.ev(cell.r.detail) || ""
                            enabled: list.ev(cell.r.enabled) !== false
                            focused: cell.isFocus; rested: list.rested
                            onActivated: { list.pointAt(cell.ri); list.activate(cell.ri) }
                            onHoverFocus: list.pointAt(cell.ri)
                            below: QuickStep { focused: cell.isFocus; text: list.ev(cell.r.value) || "" }
                        }
                    }
                    Component {
                        id: cToggle
                        QuickTile {
                            icon: list.ev(cell.r.icon) || ""
                            label: list.ev(cell.r.label) || ""; detail: list.ev(cell.r.detail) || ""
                            enabled: list.ev(cell.r.enabled) !== false
                            focused: cell.isFocus; rested: list.rested
                            onActivated: { list.pointAt(cell.ri); list.activate(cell.ri) }
                            onHoverFocus: list.pointAt(cell.ri)
                            trailing: LuToggle { checked: !!list.ev(cell.r.on) }
                        }
                    }
                    Component {
                        id: cValue
                        QuickTile {
                            icon: list.ev(cell.r.icon) || ""
                            label: list.ev(cell.r.label) || ""; detail: list.ev(cell.r.detail) || ""
                            enabled: list.ev(cell.r.enabled) !== false
                            focused: cell.isFocus; rested: list.rested
                            onActivated: { list.pointAt(cell.ri); list.activate(cell.ri) }
                            onHoverFocus: list.pointAt(cell.ri)
                            trailing: Row {
                                spacing: Lu.space(3)
                                LuChip {
                                    visible: cell.r.chip === true
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: list.ev(cell.r.value) || ""
                                    tone: list.ev(cell.r.tone) || ""
                                }
                                LuText {
                                    visible: cell.r.chip !== true && text !== ""
                                    anchors.verticalCenter: parent.verticalCenter
                                    role: "label"; color: Lu.ink2
                                    text: list.ev(cell.r.value) || ""
                                }
                                LuIcon {
                                    visible: cell.r.chevron !== false && cell.r.chip !== true
                                    anchors.verticalCenter: parent.verticalCenter
                                    name: "chevron-right"; size: Math.round(Lu.target * 0.5); color: Lu.ink2
                                }
                            }
                        }
                    }
                    Component {
                        id: cAction
                        QuickTile {
                            icon: list.ev(cell.r.icon) || ""
                            label: list.ev(cell.r.label) || ""; detail: list.ev(cell.r.detail) || ""
                            enabled: list.ev(cell.r.enabled) !== false
                            focused: cell.isFocus; rested: list.rested
                            onActivated: { list.pointAt(cell.ri); list.activate(cell.ri) }
                            onHoverFocus: list.pointAt(cell.ri)
                            trailing: LuChip { text: list.ev(cell.r.button) || "Run" }
                        }
                    }
                }
            }
        }
    }
}
