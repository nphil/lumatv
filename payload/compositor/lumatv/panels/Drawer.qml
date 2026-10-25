import QtQuick 2.4
import "../kit"

// The Notification Centre (docs/mocks/13-notifications): a panel down the right side that opens from the bell on Home's top
// bar (and a long press of the gear key), with what has happened grouped by who said it: AI access (what an assistant
// asked for and what was decided: the helper's audit), each plugin (the broker's notifications, with the plugin's own
// buttons) and the TV (NoteLog: an update ready, a finished job). One column the D-pad moves down; Right and Left move
// between an item's buttons; OK runs the focused button (an item without buttons: OK clears it); Back closes.
// "Clear all" empties the TV's own log and hides what the others said, until they say something new.
//
// Nothing in it is stored here: the audit is read each time the drawer opens, the broker's notifications come from
// PluginHub and the log from NoteLog, so a restart shows what the helper and the broker still have.
Item {
    id: drawer
    anchors.fill: parent
    property int index: 0                       // 0 = Clear all, then the items in order
    property int button: 0                      // the focused button of the focused item
    property bool rested: true
    property var audit: []
    property var hidden: ({})                   // ids the person cleared (plugin notes and audit lines)
    property var hiddenBefore: 0                // "Clear all" hides everything older than this (audit lines have no id)
    signal closeRequested()
    signal action(string pluginId, string id, string action)
    readonly property bool built: true
    visible: true

    function open() {
        index = 1; button = 0
        Helper.request("GET", "/v1/ai/audit?limit=12", undefined, function(status, doc) {
            try { if (doc && doc.entries) drawer.audit = doc.entries.slice().reverse() } catch (e) { }
        })
    }
    function when(ms) {
        if (!ms) return ""
        var d = new Date(ms), now = new Date()
        return d.toDateString() === now.toDateString() ? Qt.formatTime(d, "h:mm AP") : (now - d < 172800000 ? "Yesterday" : Qt.formatDate(d, "MMM d"))
    }
    function auditText(e) {
        var who = e.assistant ? e.assistant.name : "An assistant"
        var o = e.outcome === "ok" ? "allowed" : e.outcome === "denied" ? "was denied" : e.outcome === "expired" ? "was not answered" : e.outcome === "never" ? "was blocked" : e.outcome === "paused" ? "was paused" : String(e.outcome || "")
        return { title: e.outcome === "ok" ? who + ": " + (e.message || e.tool) : who + " " + o, body: e.outcome === "ok" ? "" : (e.message || e.tool) }
    }

    // The entries the column draws: a header per group, then its items.
    readonly property var entries: {
        var groups = [], by = {}
        function add(source, icon, item) {
            if (!by[source]) { by[source] = { source: source, icon: icon, items: [] }; groups.push(by[source]) }
            by[source].items.push(item)
        }
        for (var i = 0; i < audit.length; i++) {
            var e = audit[i], k = "audit-" + e.at
            if (hidden[k] || e.at <= hiddenBefore) continue
            var t = auditText(e)
            add("AI access", "bot", { id: k, title: t.title, body: t.body, time: e.at, actions: [] })
        }
        var notes = PluginHub.notes
        for (var j = 0; j < notes.length; j++) {
            var n = notes[j], pk = "plugin-" + n.pluginId + "-" + n.id + "-" + n.revision
            if (hidden[pk]) continue
            add(n.plugin, "plug", { id: pk, pluginId: n.pluginId, noteId: n.id, title: n.title, body: n.body, time: n.receivedAt ? n.receivedAt * 1000 : 0, actions: n.actions })
        }
        var log = NoteLog.items
        for (var m = 0; m < log.length; m++) add(log[m].source, log[m].icon, { id: log[m].id, title: log[m].title, body: log[m].body, time: log[m].time, actions: log[m].actions, own: true })
        var out = []
        for (var g = 0; g < groups.length; g++) {
            out.push({ head: true, source: groups[g].source, icon: groups[g].icon, count: groups[g].items.length })
            for (var x = 0; x < groups[g].items.length; x++) out.push({ head: false, item: groups[g].items[x] })
        }
        return out
    }
    readonly property var items: entries.filter(function(e) { return !e.head }).map(function(e) { return e.item })
    readonly property int count: items.length

    function clearAll() {
        NoteLog.clear()
        var h = {}
        for (var k in hidden) h[k] = true
        for (var i = 0; i < items.length; i++) h[items[i].id] = true
        hidden = h
        var latest = 0
        for (var a = 0; a < audit.length; a++) latest = Math.max(latest, audit[a].at || 0)
        hiddenBefore = latest
        index = 0
    }
    function dismiss(it) {
        if (it.own) NoteLog.dismiss(it.id)
        else { var h = {}; for (var k in hidden) h[k] = true; h[it.id] = true; hidden = h }
        index = Math.min(index, count)
    }
    function key(dir) {
        var it = index > 0 ? items[index - 1] : null
        if (dir === "back") { closeRequested(); return }
        if (dir === "up") { index = Math.max(0, index - 1); button = 0 }
        else if (dir === "down") { index = Math.min(count, index + 1); button = 0 }
        else if (dir === "left" && it) button = Math.max(0, button - 1)
        else if (dir === "right" && it) button = Math.min(it.actions.length - 1, button + 1)
        else if (dir === "ok") {
            if (index === 0) clearAll()
            else if (it.actions.length) { var a = it.actions[button] || it.actions[0]; if (it.pluginId) PluginHub.act(it.pluginId, a.action, a.target); else NoteLog.actionPicked(it.id, a.id || a.action); action(it.pluginId || "", it.id, a.action || a.id) }
            else dismiss(it)
        }
        rested = false; restTimer.restart()
        ensureVisible()
    }
    function ensureVisible() {
        if (index === 0) { flick.contentY = 0; return }
        for (var i = 0; i < col.children.length; i++) {
            var c = col.children[i]
            if (c.itemIndex === index) {
                if (c.y < flick.contentY) flick.contentY = c.y - Lu.space(3)
                else if (c.y + c.height > flick.contentY + flick.height) flick.contentY = c.y + c.height - flick.height + Lu.space(3)
                return
            }
        }
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: drawer.rested = true }

    Rectangle { anchors.fill: parent; color: Lu.scrim; opacity: 0.5 }
    LuPane {
        id: sheet
        x: 1920 - 64 - width; y: 96
        width: 640; height: Math.max(240, Math.min(1080 - 96 - 96, head.height + body.contentHeight + 2 * Lu.space(4) + Lu.space(4)))
        Item {
            id: head
            x: Lu.space(5); y: Lu.space(4); width: parent.width - 2 * Lu.space(5); height: Math.round(Lu.target * 1.1)
            LuText { anchors.verticalCenter: parent.verticalCenter; role: "title"; text: "Notifications" }
            LuButton {
                x: parent.width - width; anchors.verticalCenter: parent.verticalCenter
                text: "Clear all"; focused: drawer.index === 0; rested: drawer.rested
                visible: drawer.count > 0
                onHoverFocus: drawer.index = 0
                onActivated: drawer.clearAll()
            }
        }
        Flickable {
            id: flick
            x: Lu.space(3); y: head.y + head.height + Lu.space(2); width: parent.width - 2 * Lu.space(3); height: parent.height - y - Lu.space(4)
            contentWidth: width; contentHeight: col.height; clip: true
            Behavior on contentY { enabled: !Lu.reduced; NumberAnimation { duration: Lu.scrollMs; easing.type: Easing.OutCubic } }
            Column {
                id: col
                width: flick.width
                Repeater {
                    id: rep
                    model: drawer.entries
                    delegate: Item {
                        readonly property var e: modelData
                        readonly property int itemIndex: { var n = 0; for (var i = 0; i <= index; i++) if (!drawer.entries[i].head) n++; return e.head ? -1 : n }
                        width: col.width
                        height: e.head ? Math.round(Lu.row * 0.55) : itemBox.height + Lu.space(1)
                        LuText { visible: e.head; x: Lu.space(5); anchors.bottom: parent.bottom; anchors.bottomMargin: Lu.space(2); role: "caption"; color: Lu.ink3; text: e.head ? e.source + "   " + e.count : "" }
                        Item {
                            id: itemBox
                            visible: !e.head
                            readonly property var it: e.head ? null : e.item
                            readonly property bool focused: !e.head && drawer.index === itemIndex
                            width: parent.width
                            height: Lu.space(3) + Math.max(48, titleCol.height) + (it && it.actions.length ? Lu.space(3) + Lu.target : 0) + Lu.space(3)
                            FocusLift { id: lift; focused: itemBox.focused; rested: drawer.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                            scale: 1 + (Lu.rowScale - 1) * lift.value
                            Rectangle {
                                anchors.fill: parent; radius: Lu.radius("row")
                                color: itemBox.focused ? Lu.wash("focus") : "transparent"; border.width: itemBox.focused ? 1 : 0; border.color: Lu.edgeRaised; antialiasing: true
                            }
                            LuTile { x: Lu.space(4); y: Lu.space(3); size: 48; icon: itemBox.it ? (itemBox.it.pluginId ? "plug" : "info") : "info" }
                            Column {
                                id: titleCol
                                x: Lu.space(4) + 48 + Lu.space(4); y: Lu.space(3); width: parent.width - x - 96 - Lu.space(4)
                                LuText { width: parent.width; role: "label"; lineHeight: 1.0; elide: Text.ElideRight; text: itemBox.it ? itemBox.it.title : "" }
                                LuText { width: parent.width; role: "caption"; lineHeight: 1.0; color: Lu.ink2; wrapMode: Text.WordWrap; maximumLineCount: 2; elide: Text.ElideRight; visible: itemBox.it && itemBox.it.body !== ""; text: itemBox.it ? itemBox.it.body : "" }
                            }
                            LuText { x: parent.width - width - Lu.space(4); y: Lu.space(3); role: "caption"; color: Lu.ink3; text: itemBox.it ? drawer.when(itemBox.it.time) : "" }
                            Row {
                                visible: itemBox.it && itemBox.it.actions.length > 0
                                x: Lu.space(4) + 48 + Lu.space(4); y: parent.height - Lu.space(3) - Lu.target; spacing: Lu.space(3)
                                Repeater {
                                    model: itemBox.it ? itemBox.it.actions : []
                                    delegate: LuButton {
                                        text: modelData.label
                                        focused: itemBox.focused && drawer.button === index
                                        rested: drawer.rested
                                        onHoverFocus: { drawer.index = itemIndex; drawer.button = index }
                                        onActivated: { drawer.index = itemIndex; drawer.button = index; drawer.key("ok") }
                                    }
                                }
                            }
                            MouseArea { anchors.fill: parent; z: -1; hoverEnabled: true; onEntered: drawer.index = itemIndex; onClicked: { drawer.index = itemIndex; if (!itemBox.it.actions.length) drawer.dismiss(itemBox.it) } }
                        }
                    }
                }
            }
        }
        LuText {
            visible: drawer.count === 0
            anchors.horizontalCenter: parent.horizontalCenter; y: head.y + head.height + Lu.space(8)
            role: "label"; color: Lu.ink2; text: "Nothing new"
        }
    }
    // the sheet sizes itself to the column
    property alias body: flick
}
