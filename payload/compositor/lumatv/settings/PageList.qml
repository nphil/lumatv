import QtQuick 2.4
import "../kit"

// A Settings page drawn from a list of row descriptors (docs/mocks/30-settings-*, 01-03, 24). A page file
// sets `title`, `subtitle`, `tabs` and `rows`; this draws the head, the tab bar and the rows, owns the page's
// focus and scrolling, and calls the row's own `act(value)` when the owner changes or activates it. The
// rows are built when the page is (Settings builds every page once, in the background), a tab switch only
// changes which rows are visible, and the page's state lives in the descriptors' functions, so nothing is
// created or destroyed by a key.
//
// A descriptor is an object: `kind` (toggle, seg, slider, value, action, cards, stepper, head, note), `tab`
// (default 0), `id`, `icon` (a Lucide name), `label`, `detail`, and per kind `on` (toggle), `options` and
// `index` (seg, stepper, cards), `min`, `max`, `step`, `num`, `unit` and `valueText` (slider; a text for the number at the right, which replaces the rounded value), `value` (value, a text), `zones` and `text` (figure: a small TV with the lit edges and a caption; shown, never focused),
// `button` (action), `cols` (cards), `text` (note, head), `chevron`, `selected` (value: the chosen one of a list of rows, drawn with a wash and a check at the right), `info` (a row that only shows: full strength,
// never focused). Any of label, detail, value, on, index,
// num, options, text, selected, enabled and visible may be a function: it runs inside the binding that draws the row,
// so what it reads (the settings service, the helper's topics) redraws the row when it changes. `act(v)` is
// called with the new value: a boolean for a toggle, the segment or card index for seg, stepper and cards, the
// number for a slider, nothing for value and action.
//
// Keys, from the host: Up and Down move between rows (the tab bar is above the first), Left and Right
// move inside a segmented row, a stepper, a card grid or a slider, or change the tab on the tab bar, OK
// flips or picks. `key(dir)` returns false for what the page cannot use (Left at the edge of a row, Back,
// Up from the tab bar), and the host then moves focus to the rail.
Item {
    id: page
    property var host                                   // the Settings surface
    property var arg: ({})                              // what this page was opened for (host.openPage(id, arg))
    property string title: ""
    property string subtitle: ""
    property string back: ""                            // a page this one is a child of: "Sound", shown as a pill
    property string backId: ""
    property string chip: ""
    property string chipTone: "positive"
    property var tabs: []
    property var rows: []
    property int tab: 0
    property int stop: -2                               // a row index, -1 the tab bar, -2 not yet placed
    property int seg: 0                                 // the inner focus of a seg, stepper or cards row
    property bool blocked: false                        // the owner swallows every key (a prompt is up)
    readonly property bool active: host ? host.zone === "page" : false
    readonly property bool rested: host ? host.rested : true
    property string summary: subtitle                   // the rail's second line; a page sets its own
    signal opened()                                     // the page was brought up: refresh what it reads

    function ev(x) { return typeof x === "function" ? x() : x }
    function rowTab(r) { return r.tab === undefined ? 0 : r.tab }
    function shown(i) { var r = rows[i]; return !!r && rowTab(r) === tab && ev(r.visible) !== false }
    function focusable(i) {
        var r = rows[i]
        return shown(i) && r.kind !== "head" && r.kind !== "note" && r.kind !== "figure" && r.info !== true && ev(r.enabled) !== false
    }
    function first() { for (var i = 0; i < rows.length; i++) if (focusable(i)) return i; return -2 }
    function step(from, dir) {
        for (var i = from + dir; i >= 0 && i < rows.length; i += dir) if (focusable(i)) return i
        return -2
    }
    function count(i) { var r = rows[i]; var o = ev(r.options); return o ? o.length : 0 }
    function enter(i) {
        stop = i
        var r = i >= 0 ? rows[i] : null
        seg = r && (r.kind === "seg" || r.kind === "stepper" || r.kind === "cards") ? Math.max(0, Math.min(count(i) - 1, ev(r.index) || 0)) : 0
    }
    // The host calls this when the page is brought up or the section is entered.
    function reset() {
        if (stop < 0 || !focusable(stop)) enter(first())
        opened()
    }
    // Right and Left on the tab bar change the tab and leave the focus on the bar, so the next press goes to the next tab; a tab picked
    // any other way (the pointer sets the bar's stop first) has its first row focused.
    onTabChanged: { if (stop !== -1) enter(first()); offset = 0 }
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
        else if (r.kind === "seg" || r.kind === "cards") change(i, seg)
        else if (r.kind === "value" || r.kind === "action") change(i)
        else if (r.kind === "nudge" && r.open) r.open()
    }

    function key(dir) {
        if (blocked) return true
        if (stop === -2) { var f0 = first(); if (f0 >= 0) enter(f0) }      // rows that were not there yet
        if (stop === -1 || (stop < 0 && tabs.length > 0)) {
            // the tab bar
            if (dir === "left") { if (tab === 0) return false; stop = -1; tab--; return true }
            if (dir === "right") { stop = -1; tab = Math.min(tabs.length - 1, tab + 1); return true }
            if (dir === "down") { var f = first(); if (f >= 0) enter(f); return true }
            if (dir === "up") return false
            return dir !== "back"
        }
        if (stop < 0) return false
        var r = rows[stop]
        var k = r.kind
        if (dir === "up") {
            if (k === "cards" && seg - (r.cols || 4) >= 0) { seg -= (r.cols || 4); return true }
            var p = step(stop, -1)
            if (p >= 0) enter(p)
            else if (tabs.length > 0) { stop = -1 }
            else return false
            return true
        }
        if (dir === "down") {
            if (k === "cards" && seg + (r.cols || 4) < count(stop)) { seg += (r.cols || 4); return true }
            var n = step(stop, 1)
            if (n >= 0) enter(n)
            return true
        }
        if (dir === "left" || dir === "right") {
            var d = dir === "left" ? -1 : 1
            if (k === "seg" || k === "cards") {
                if (seg + d < 0) return false
                seg = Math.min(count(stop) - 1, seg + d)
                return true
            }
            if (k === "nudge") { change(stop, d); return true }                 // the value is read back; Left and Right only say which way
            if (k === "stepper") {
                var n2 = count(stop)
                if (n2 > 0) { seg = (seg + d + n2) % n2; change(stop, seg) }
                return true
            }
            if (k === "slider") {
                var lo = ev(r.min), hi = ev(r.max), st = ev(r.step) || 1
                var cur = ev(r.num) || 0
                var v = Math.max(lo, Math.min(hi, Math.round((cur + d * st) / st) * st))
                if (v !== cur) change(stop, v)
                return true
            }
            return d < 0 ? false : true
        }
        if (dir === "ok") { activate(stop); return true }
        return false
    }

    // ---- head
    Item {
        id: head
        width: parent.width
        height: titleText.height
        Rectangle {
            id: backPill
            visible: page.back !== ""
            height: Math.round(Lu.target * 0.7)
            width: backText.implicitWidth + Lu.space(5) + Lu.space(6)
            radius: height / 2
            anchors.verticalCenter: titleText.verticalCenter
            color: Lu.wash("selected")
            border.width: 1
            border.color: Lu.edge
            LuIcon { x: Lu.space(3); anchors.verticalCenter: parent.verticalCenter; name: "chevron-left"; size: Math.round(Lu.target * 0.36); color: Lu.ink2 }
            LuText { id: backText; x: Lu.space(3) + Math.round(Lu.target * 0.36) + Lu.space(1); anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink2; text: page.back }
        }
        LuText { id: titleText; x: backPill.visible ? backPill.width + Lu.space(4) : 0; role: "title"; text: page.title }
        LuText {
            x: titleText.x + titleText.width + Lu.space(4)
            width: head.width - x - (chipItem.visible ? chipItem.width + Lu.space(4) : 0)
            anchors.baseline: titleText.baseline
            role: "label"; color: Lu.ink2
            text: page.subtitle
            elide: Text.ElideRight
        }
        LuChip { id: chipItem; visible: page.chip !== ""; x: head.width - width; anchors.verticalCenter: titleText.verticalCenter; text: page.chip; tone: page.chipTone }
    }

    // ---- tabs
    LuSegmented {
        id: tabBar
        visible: page.tabs.length > 0
        y: head.height + Lu.space(4)
        options: page.tabs
        current: page.tab
        focusedIndex: page.active && page.stop === -1 ? page.tab : -1
        segmentWidth: page.tabs.length > 0 ? Math.floor(page.width / page.tabs.length) : Lu.target * 3
        onPicked: { if (page.host) page.host.zone = "page"; page.stop = -1; page.tab = index }
        onHoverFocus: { if (page.host) page.host.zone = "page"; page.stop = -1; page.tab = index }
    }

    // ---- rows
    property real offset: 0
    function itemFor(i) { return rep.itemAt(i) }
    onStopChanged: scrollTimer.restart()
    Timer {
        id: scrollTimer
        interval: 1
        onTriggered: {
            var it = page.stop >= 0 ? page.itemFor(page.stop) : null
            if (!it) { if (page.stop === -1) page.offset = 0; return }
            var top = it.mapToItem(list, 0, 0).y
            if (top - page.offset < 0) page.offset = top
            else if (top + it.height - page.offset > viewport.height) page.offset = top + it.height - viewport.height
        }
    }
    Item {
        id: viewport
        y: head.height + Lu.space(4) + (page.tabs.length > 0 ? tabBar.height + Lu.space(4) : 0)
        width: parent.width
        height: parent.height - y
        clip: true
        Column {
            id: list
            width: parent.width
            y: -page.offset
            Behavior on y { enabled: !Lu.reduced; NumberAnimation { duration: Lu.cardMs; easing.type: Lu.cardEasing } }
            Repeater {
                id: rep
                model: page.rows.length                 // an integer: a list model would copy each descriptor and drop its functions
                delegate: Item {
                    id: cell
                    property var r: page.rows[index]
                    property int ri: index
                    readonly property bool isFocus: page.active && page.stop === ri
                    visible: page.shown(ri)
                    width: list.width
                    height: visible && body.item ? body.item.height : 0
                    Loader {
                        id: body
                        width: cell.width
                        sourceComponent: cell.r.kind === "toggle" ? cToggle : cell.r.kind === "seg" ? cSeg : cell.r.kind === "slider" ? cSlider
                                       : cell.r.kind === "value" ? cValue : cell.r.kind === "action" ? cAction : cell.r.kind === "cards" ? cCards
                                       : cell.r.kind === "stepper" ? cStepper : cell.r.kind === "nudge" ? cNudge : cell.r.kind === "head" ? cHead : cell.r.kind === "figure" ? cFigure : cNote
                    }

                    Component {
                        id: cToggle
                        LuRow {
                            icon: page.ev(cell.r.icon) || ""
                            label: page.ev(cell.r.label) || ""; detail: page.ev(cell.r.detail) || ""
                            enabled: page.ev(cell.r.enabled) !== false
                            focused: cell.isFocus; rested: page.rested
                            onActivated: { page.pointAt(cell.ri); page.activate(cell.ri) }
                            onHoverFocus: page.pointAt(cell.ri)
                            trailing: LuToggle { checked: !!page.ev(cell.r.on) }
                        }
                    }
                    Component {
                        id: cSeg
                        LuRow {
                            icon: page.ev(cell.r.icon) || ""
                            label: page.ev(cell.r.label) || ""; detail: page.ev(cell.r.detail) || ""
                            enabled: page.ev(cell.r.enabled) !== false
                            focused: cell.isFocus; rested: page.rested
                            onHoverFocus: page.pointAt(cell.ri)
                            trailing: LuSegmented {
                                options: page.ev(cell.r.options) || []
                                current: page.ev(cell.r.index) === undefined ? -1 : page.ev(cell.r.index)
                                textRole: "caption"
                                focusedIndex: cell.isFocus ? page.seg : -1
                                onPicked: { page.pointAt(cell.ri); page.seg = index; page.activate(cell.ri) }
                                onHoverFocus: { page.pointAt(cell.ri); page.seg = index }
                            }
                        }
                    }
                    Component {
                        id: cStepper
                        LuRow {
                            icon: page.ev(cell.r.icon) || ""
                            label: page.ev(cell.r.label) || ""; detail: page.ev(cell.r.detail) || ""
                            enabled: page.ev(cell.r.enabled) !== false
                            focused: cell.isFocus; rested: page.rested
                            onHoverFocus: page.pointAt(cell.ri)
                            trailing: Row {
                                spacing: Lu.space(3)
                                LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "chevron-left"; size: Math.round(Lu.target * 0.5); color: Lu.ink2 }
                                LuText {
                                    width: Math.round(Lu.target * 3); horizontalAlignment: Text.AlignHCenter
                                    role: "label"; anchors.verticalCenter: parent.verticalCenter
                                    text: { var o = page.ev(cell.r.options) || []; var i = page.ev(cell.r.index) || 0; return o[i] === undefined ? "" : o[i] }
                                }
                                LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "chevron-right"; size: Math.round(Lu.target * 0.5); color: Lu.ink2 }
                            }
                        }
                    }
                    Component {
                        id: cNudge
                        LuRow {
                            icon: page.ev(cell.r.icon) || ""
                            label: page.ev(cell.r.label) || ""; detail: page.ev(cell.r.detail) || ""
                            enabled: page.ev(cell.r.enabled) !== false
                            focused: cell.isFocus; rested: page.rested
                            onActivated: { page.pointAt(cell.ri); page.activate(cell.ri) }
                            onHoverFocus: page.pointAt(cell.ri)
                            trailing: Row {
                                spacing: Lu.space(3)
                                LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "chevron-left"; size: Math.round(Lu.target * 0.45); color: cell.isFocus ? Lu.ink : Lu.ink2 }
                                LuText { width: Math.round(Lu.target * 2.2); horizontalAlignment: Text.AlignHCenter; anchors.verticalCenter: parent.verticalCenter; role: "label"; text: page.ev(cell.r.value) || "" }
                                LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "chevron-right"; size: Math.round(Lu.target * 0.45); color: cell.isFocus ? Lu.ink : Lu.ink2 }
                            }
                        }
                    }
                    Component {
                        id: cSlider
                        LuRow {
                            icon: page.ev(cell.r.icon) || ""
                            label: page.ev(cell.r.label) || ""; detail: page.ev(cell.r.detail) || ""
                            enabled: page.ev(cell.r.enabled) !== false
                            focused: cell.isFocus; rested: page.rested
                            onHoverFocus: page.pointAt(cell.ri)
                            trailing: Row {
                                spacing: Lu.space(4)
                                LuSlider {
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: Lu.target * 5
                                    from: page.ev(cell.r.min) || 0; to: page.ev(cell.r.max) === undefined ? 100 : page.ev(cell.r.max)
                                    step: page.ev(cell.r.step) || 1
                                    value: page.ev(cell.r.num) || 0
                                    active: cell.isFocus
                                    onMoved: { page.pointAt(cell.ri); page.change(cell.ri, value) }
                                }
                                LuText {
                                    width: Lu.target * 1.5; horizontalAlignment: Text.AlignRight
                                    anchors.verticalCenter: parent.verticalCenter
                                    role: "label"
                                    text: cell.r.valueText !== undefined ? page.ev(cell.r.valueText) : String(Math.round(page.ev(cell.r.num) || 0)) + (cell.r.unit || "")
                                }
                            }
                        }
                    }
                    Component {
                        id: cValue
                        LuRow {
                            icon: page.ev(cell.r.icon) || ""
                            label: page.ev(cell.r.label) || ""; detail: page.ev(cell.r.detail) || ""
                            value: page.ev(cell.r.value) || ""
                            enabled: page.ev(cell.r.enabled) !== false
                            selected: page.ev(cell.r.selected) === true
                            focused: cell.isFocus; rested: page.rested
                            onActivated: { page.pointAt(cell.ri); page.activate(cell.ri) }
                            onHoverFocus: page.pointAt(cell.ri)
                            trailing: LuIcon {
                                visible: cell.r.chevron !== false || page.ev(cell.r.selected) === true
                                name: page.ev(cell.r.selected) === true ? "check" : "chevron-right"
                                size: cell.r.chevron === false && page.ev(cell.r.selected) !== true ? 0 : Math.round(Lu.target * 0.5)
                                color: page.ev(cell.r.selected) === true ? Lu.accent : Lu.ink2
                            }
                        }
                    }
                    Component {
                        id: cAction
                        LuRow {
                            icon: page.ev(cell.r.icon) || ""
                            label: page.ev(cell.r.label) || ""; detail: page.ev(cell.r.detail) || ""
                            enabled: page.ev(cell.r.enabled) !== false
                            focused: cell.isFocus; rested: page.rested
                            onActivated: { page.pointAt(cell.ri); page.activate(cell.ri) }
                            onHoverFocus: page.pointAt(cell.ri)
                            trailing: LuChip { text: page.ev(cell.r.button) || "Run" }
                        }
                    }
                    Component {
                        id: cCards
                        Item {
                            id: grid
                            readonly property var opts: page.ev(cell.r.options) || []
                            readonly property bool strip: cell.r.strip === true
                            readonly property int cols: strip ? 1000 : (cell.r.cols || 4)
                            readonly property real inView: (cell.r.inView || 4) + (strip ? 0.4 : 0)     // a strip shows part of the next card: it scrolls
                            readonly property int gap: Lu.space(3)
                            readonly property int cw: strip ? Math.floor((cell.width - gap * Math.ceil(inView - 1)) / inView) : Math.floor((cell.width - gap * (cols - 1)) / cols)
                            // the strip moves just far enough to keep the focused (or chosen) card whole
                            readonly property int at: cell.isFocus ? page.seg : Math.max(0, page.ev(cell.r.index) || 0)
                            readonly property real scroll: strip ? Math.max(0, Math.min(opts.length * (cw + gap) - gap - cell.width, (at + 1) * (cw + gap) - gap - cell.width)) : 0
                            width: cell.width
                            height: (strip ? 1 : Math.ceil(opts.length / cols)) * (Math.round(Lu.row * 0.95) + gap)
                            clip: strip
                            Repeater {
                                model: grid.opts
                                delegate: LuCard {
                                    x: (grid.strip ? index : index % grid.cols) * (grid.cw + grid.gap) - grid.scroll
                                    y: grid.strip ? 0 : Math.floor(index / grid.cols) * (height + grid.gap)
                                    Behavior on x { enabled: grid.strip && !Lu.reduced; NumberAnimation { duration: Lu.cardMs; easing.type: Lu.cardEasing } }
                                    width: grid.cw
                                    icon: modelData.icon || ""; title: modelData.title || ""; sub: modelData.sub || ""
                                    selected: page.ev(cell.r.index) === index
                                    focused: cell.isFocus && page.seg === index
                                    rested: page.rested
                                    onActivated: { page.pointAt(cell.ri); page.seg = index; page.activate(cell.ri) }
                                    onHoverFocus: { page.pointAt(cell.ri); page.seg = index }
                                }
                            }
                        }
                    }
                    Component {
                        id: cHead
                        Item {
                            width: cell.width
                            height: Math.round(Lu.row * 0.6)
                            LuText { x: Lu.space(2); anchors.bottom: parent.bottom; anchors.bottomMargin: Lu.space(2); role: "caption"; color: Lu.ink3; text: page.ev(cell.r.text) || page.ev(cell.r.label) || "" }
                        }
                    }
                    Component {
                        id: cFigure
                        Item {
                            id: fig
                            readonly property var zn: page.ev(cell.r.zones) || ({})
                            readonly property int tvW: Math.round(Lu.target * 2.4)
                            readonly property int tvH: Math.round(tvW * 0.56)
                            width: cell.width
                            height: tvH + Lu.space(8)
                            Item {
                                id: tv
                                x: Lu.space(2); y: Lu.space(4); width: fig.tvW; height: fig.tvH
                                Rectangle { anchors.fill: parent; anchors.margins: 6; radius: 4; color: Lu.wash("selected"); border.width: 1; border.color: Lu.edge }
                                Rectangle { visible: (fig.zn.top || 0) > 0; x: 6; y: 0; width: parent.width - 12; height: 4; radius: 2; color: Lu.accent }
                                Rectangle { visible: (fig.zn.bottom || 0) > 0; x: 6; y: parent.height - 4; width: parent.width - 12; height: 4; radius: 2; color: Lu.accent }
                                Rectangle { visible: (fig.zn.left || 0) > 0; x: 0; y: 6; width: 4; height: parent.height - 12; radius: 2; color: Lu.accent }
                                Rectangle { visible: (fig.zn.right || 0) > 0; x: parent.width - 4; y: 6; width: 4; height: parent.height - 12; radius: 2; color: Lu.accent }
                            }
                            LuText {
                                x: tv.x + tv.width + Lu.space(6); width: parent.width - x - Lu.space(2); anchors.verticalCenter: tv.verticalCenter
                                role: "caption"; color: Lu.ink2; wrapMode: Text.WordWrap
                                text: page.ev(cell.r.text) || ""
                            }
                        }
                    }
                    Component {
                        id: cNote
                        Item {
                            width: cell.width
                            height: noteText.height + Lu.space(5)
                            LuIcon { x: Lu.space(2); y: Lu.space(1); name: "info"; size: Math.round(Lu.target * 0.36); color: Lu.ink3 }
                            LuText {
                                id: noteText
                                x: Lu.space(2) + Math.round(Lu.target * 0.36) + Lu.space(3)
                                width: parent.width - x - Lu.space(2)
                                role: "caption"; color: Lu.ink2; wrapMode: Text.WordWrap
                                text: page.ev(cell.r.text) || ""
                            }
                        }
                    }
                }
            }
        }
    }

    function longest(opts) {
        var n = 0
        for (var i = 0; i < (opts ? opts.length : 0); i++) n = Math.max(n, String(opts[i]).length)
        return n
    }
}
