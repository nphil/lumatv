import QtQuick 2.4
import "../kit"

// The Notification Centre (docs/mocks/13-notifications): a panel down the right side that opens from the bell on Home's top
// bar (and a long press of the gear key), with what has happened grouped by who said it: AI access (what an assistant
// asked for and what was decided: the helper's audit), each plugin (the broker's notifications, with the plugin's own
// buttons) and the TV (NoteLog: an update ready, what LG's notification service said). One column the D-pad moves down; Right and
// Left move between an item's buttons; OK runs the focused button (an item without buttons: OK clears it); Back closes.
// "Clear all" empties the TV's own log and hides what the others said, until they say something new.
//
// Nothing in it is stored here: the audit is read each time the drawer opens, the broker's notifications come from
// PluginHub and the log from NoteLog, so a restart shows what the helper and the broker still have. Opening it is what marks
// everything seen (NoteLog.unread, the bell's badge).
//
// The host (Layers.qml) builds it once and shows it with `visible`; it calls `open()` first and sends the keys to `key(dir)`.
Item {
    id: drawer
    anchors.fill: parent
    property int index: 0                       // 0 = Clear all, then the items in order
    property int button: 0                      // the focused button of the focused item
    property bool rested: true
    property var audit: []
    property var assistants: []                 // the paired assistants as they were when the drawer opened (a paused one can be resumed from here)
    property var hidden: ({})                   // ids the person cleared (plugin notes and audit lines)
    property double hiddenBefore: 0             // "Clear all" hides every audit line up to this time (audit lines have no id)
    signal closeRequested()
    signal action(string pluginId, string id, string action)
    readonly property bool built: true

    readonly property int sheetTop: 56               // the status bar's top: the sheet starts where the bar does
    readonly property int sheetWidth: 640

    function open() {
        index = 1; button = 0; rested = true
        NoteLog.markSeen()
        assistants = (Helper.ai && Helper.ai.assistants) || []
        Helper.request("GET", "/v1/ai/audit?limit=12", undefined, function(status, doc) {
            try { if (doc && doc.entries) drawer.audit = doc.entries.slice().reverse() } catch (e) { }
        })
    }
    function when(ms) {
        if (!ms) return ""
        var d = new Date(ms), now = new Date()
        return d.toDateString() === now.toDateString() ? Qt.formatTime(d, "h:mm AP") : (now - d < 172800000 ? "Yesterday" : Qt.formatDate(d, "MMM d"))
    }
    // What an audit line says, and the button it offers: a denial pauses the assistant, and a paused one can be resumed from here
    // (the helper then asks on the TV, and only the physical remote answers).
    function auditText(e) {
        var who = e.assistant ? e.assistant.name : "An assistant"
        var o = e.outcome === "ok" ? "allowed" : e.outcome === "denied" ? "was denied" : e.outcome === "expired" ? "was not answered" : e.outcome === "never" ? "was blocked" : e.outcome === "paused" ? "was paused" : String(e.outcome || "")
        var what = e.message || e.tool
        var paused = null
        for (var i = 0; e.outcome === "denied" && e.assistant && i < assistants.length; i++) if (assistants[i].id === e.assistant.id && assistants[i].paused) paused = assistants[i]
        return { title: e.outcome === "ok" ? who + ": " + what : who + " " + o,
                 body: e.outcome === "ok" ? "" : paused ? "It asked to " + what + "; its requests are paused until you resume them." : what,
                 actions: paused ? [{ label: "Resume " + paused.name, ai: "resume", assistant: paused.id }] : [] }
    }

    // The entries the column draws: a header per group, then its items. An item carries its number among the items (`idx`, from 1),
    // which is what `index` is.
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
            add("AI access", "bot", { id: k, icon: "sparkles", title: t.title, body: t.body, time: e.at, actions: t.actions })
        }
        var notes = PluginHub.notes
        for (var j = 0; j < notes.length; j++) {
            var n = notes[j], pk = "plugin-" + n.pluginId + "-" + n.id + "-" + n.revision
            if (hidden[pk]) continue
            add(n.plugin, "plug", { id: pk, icon: n.channelIcon ? PluginHub.icon(n.channelIcon) : "bell", pluginId: n.pluginId, noteId: n.id, title: n.title, body: n.body, time: n.receivedAt ? n.receivedAt * 1000 : 0, actions: n.actions })
        }
        var log = NoteLog.items
        for (var m = 0; m < log.length; m++) add(log[m].source, "monitor", { id: log[m].id, icon: log[m].icon, title: log[m].title, body: log[m].body, time: log[m].time, actions: log[m].actions, own: true })
        var out = [], n = 0
        for (var g = 0; g < groups.length; g++) {
            out.push({ head: true, source: groups[g].source, icon: groups[g].icon, count: groups[g].items.length })
            for (var x = 0; x < groups[g].items.length; x++) out.push({ head: false, idx: ++n, item: groups[g].items[x] })
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
        button = 0
    }
    function key(dir) {
        var it = index > 0 ? items[index - 1] : null
        if (dir === "back") { closeRequested(); return }
        if (dir === "up") { index = Math.max(0, index - 1); button = 0 }
        else if (dir === "down") { index = Math.min(count, index + 1); button = 0 }
        else if (dir === "left" && it) button = Math.max(0, button - 1)
        else if (dir === "right" && it) button = Math.max(0, Math.min(it.actions.length - 1, button + 1))
        else if (dir === "ok") {
            if (index === 0) clearAll()
            else if (it.actions.length) {
                var a = it.actions[button] || it.actions[0]
                if (a.ai === "resume") Helper.post("/v1/ai/assistants/" + a.assistant + "/resume", {})
                else if (it.pluginId) PluginHub.act(it.pluginId, a.action, a.target)
                else NoteLog.actionPicked(it.id, a.id || a.action)
                action(it.pluginId || "", it.id, a.action || a.id || a.ai)
            }
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

    Rectangle { anchors.fill: parent; color: Lu.scrim }
    MouseArea { anchors.fill: parent; onClicked: drawer.closeRequested() }
    LuPane {
        id: sheet
        objectName: "drawerSheet"
        x: 1920 - 64 - width; y: drawer.sheetTop
        width: drawer.sheetWidth
        // The sheet is as tall as what it holds: the list, or, with nothing to list, the empty state (its words wrap to two lines, so its height is
        // the column's and not a number). Never under 240, and never over the screen less the bar's margin at the top and bottom.
        readonly property real bodyHeight: drawer.count === 0 ? emptyCol.y - (head.y + head.height) + emptyCol.height : Lu.space(2) + flick.contentHeight
        height: Math.max(240, Math.min(1080 - 2 * drawer.sheetTop, head.y + head.height + bodyHeight + Lu.space(4)))
        // a click on the sheet is not a click on the scrim behind it
        MouseArea { anchors.fill: parent }
        Item {
            id: head
            x: Lu.space(5); y: Lu.space(4); width: parent.width - 2 * Lu.space(5)
            // the button sits beside the title while both fit on a line, else under it (a large text size)
            readonly property bool stacked: clearBtn.visible && titleText.implicitWidth + clearBtn.width + Lu.space(4) > width
            readonly property int lineH: Math.round(Lu.target * 1.1)
            height: stacked ? titleText.height + Lu.space(2) + clearBtn.height : lineH
            LuText { id: titleText; width: Math.min(implicitWidth, parent.width); y: head.stacked ? 0 : Math.round((head.lineH - height) / 2); role: "title"; text: "Notifications" }
            LuButton {
                id: clearBtn
                x: head.stacked ? 0 : head.width - width
                y: head.stacked ? titleText.height + Lu.space(2) : Math.round((head.lineH - height) / 2)
                text: "Clear all"; icon: "check"; focused: drawer.index === 0; rested: drawer.rested
                visible: drawer.count > 0
                onHoverFocus: drawer.index = 0
                onActivated: drawer.clearAll()
            }
        }
        Flickable {
            id: flick
            x: Lu.space(3); y: head.y + head.height + Lu.space(2); width: parent.width - 2 * Lu.space(3); height: parent.height - y - Lu.space(4)
            contentWidth: width; contentHeight: col.height; clip: true
            interactive: false
            Behavior on contentY { enabled: !Lu.reduced; NumberAnimation { duration: Lu.ms("scrollMs"); easing.type: Easing.OutCubic } }
            Column {
                id: col
                width: flick.width
                Repeater {
                    id: rep
                    model: drawer.entries
                    delegate: Item {
                        readonly property var e: modelData
                        readonly property int itemIndex: e.head ? -1 : e.idx
                        width: col.width
                        height: e.head ? Math.round(Lu.row * 0.6) : itemBox.height + Lu.space(1)
                        Item {
                            visible: e.head
                            anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom; anchors.bottomMargin: Lu.space(2)
                            height: 28
                            LuIcon { x: Lu.space(5); y: 2; size: 24; name: e.head ? e.icon : ""; color: Lu.ink3 }
                            LuText { x: Lu.space(5) + 24 + Lu.space(3); y: Math.round((28 - height) / 2); role: "caption"; color: Lu.ink2; text: e.head ? e.source : "" }
                            LuText { x: col.width - width - Lu.space(5); y: Math.round((28 - height) / 2); role: "caption"; color: Lu.ink2; text: e.head ? String(e.count) : "" }
                        }
                        Item {
                            id: itemBox
                            visible: !e.head
                            readonly property var it: e.head ? null : e.item
                            readonly property bool focused: !e.head && drawer.index === itemIndex
                            width: parent.width
                            height: Lu.space(3) + Math.max(48, titleCol.height) + (it && it.actions.length ? Lu.space(3) + Lu.target : 0) + Lu.space(3)
                            FocusLift { id: lift; focused: itemBox.focused; rested: drawer.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                            Rectangle {
                                anchors.fill: parent; radius: Lu.radius("row")
                                color: itemBox.focused ? Lu.wash("focus") : "transparent"; border.width: itemBox.focused ? 1 : 0; border.color: Lu.edgeRaised; antialiasing: true
                                scale: 1 + (Lu.rowScale - 1) * lift.value; transformOrigin: Item.Center       // the plate lifts; the words stay put
                            }
                            LuTile { x: Lu.space(4); y: Lu.space(3); size: 48; icon: itemBox.it ? itemBox.it.icon : "" }
                            Column {
                                id: titleCol
                                x: Lu.space(4) + 48 + Lu.space(4); y: Lu.space(3); width: parent.width - x - (when.text !== "" ? when.width + Lu.space(3) : 0) - Lu.space(4)
                                LuText { width: parent.width; role: "label"; elide: Text.ElideRight; text: itemBox.it ? itemBox.it.title : "" }
                                LuText { width: parent.width; role: "caption"; line: 1.0; color: Lu.ink2; wrapMode: Text.WordWrap; maximumLineCount: 3; elide: Text.ElideRight; visible: itemBox.it && itemBox.it.body !== ""; text: itemBox.it ? itemBox.it.body : "" }
                            }
                            // the time: in the second ink, which reads over the glass at 3:1 where the third did not (2.3 to 2.9:1 in the audit)
                            LuText { id: when; x: parent.width - width - Lu.space(4); y: Lu.space(3); role: "caption"; color: Lu.ink2; text: itemBox.it ? drawer.when(itemBox.it.time) : "" }
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
        Column {
            id: emptyCol
            objectName: "drawerEmpty"
            visible: drawer.count === 0
            x: Lu.space(7); y: head.y + head.height + Lu.space(7); width: sheet.width - 2 * Lu.space(7)
            spacing: Lu.space(3)
            LuTile { x: Math.round((emptyCol.width - width) / 2); size: 64; icon: "bell-off" }
            LuText { width: emptyCol.width; horizontalAlignment: Text.AlignHCenter; role: "label"; text: "Nothing new" }
            LuText { width: emptyCol.width; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.WordWrap; role: "caption"; color: Lu.ink2; text: "What your assistants, plugins and this TV tell you shows here." }
        }
    }
}
