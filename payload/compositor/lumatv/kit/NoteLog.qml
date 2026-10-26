pragma Singleton
import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0

// What the TV itself has told the person since it started: an update that is ready, a backup that finished, what LG's
// notification service raised as a toast, the answer to an AI access question. The Notification Centre (panels/Drawer.qml)
// shows this beside what the AI access audit and the plugins say. It lives in memory (a restart is a clean slate: the helper
// keeps the audit and the broker its own notifications). `post()` is the one way in, so a surface that has something to say does
// not know the drawer.
//
//   post({ source: "TV", icon: "download", title, body, time (ms, default now), actions: [{ id, label }] })
//
// LG's toasts reach the log through the subscription below. Toasts are subscribe-only on this firmware (there is no backlog), so
// the log only has what arrived while it existed: Home touches it when it is built (the bell's badge reads `unread`), and the
// first access is what starts the subscription. A toast that PluginHub itself raised for a plugin's notification is not logged
// again here: the drawer already lists it under the plugin.
//
// `unread` is what the bell's badge counts: the log's entries and the plugins' notifications that arrived after the drawer was
// last open (`markSeen()`). The AI access audit is not counted; the helper keeps it and the drawer reads it when it opens.
QtObject {
    id: log
    property var items: []
    property int serial: 0
    property double seenAt: 0
    signal actionPicked(string id, string action)

    readonly property int unread: {
        var n = 0, i
        for (i = 0; i < items.length; i++) if (items[i].time > seenAt) n++
        var notes = PluginHub.notes
        for (i = 0; i < notes.length; i++) if ((notes[i].receivedAt || 0) * 1000 > seenAt) n++
        return n
    }
    function markSeen() { seenAt = Date.now() }

    function post(n) {
        var list = items.slice()
        list.unshift({ id: "tv-" + (++serial), source: n.source || "TV", icon: n.icon || "info", title: n.title || "", body: n.body || "",
                       time: n.time || Date.now(), actions: n.actions || [] })
        items = list.slice(0, 50)
    }
    function dismiss(id) { items = items.filter(function(i) { return i.id !== id }) }
    function clear() { items = [] }

    // LG's own toasts. The first line of the message is the title, the rest the subtitle (LG writes a line break as `<br>` or "\n").
    function fromToast(m) {
        var text = String(m.message || "").replace(/<br\s*\/?>/gi, "\n").replace(/^\s+|\s+$/g, "")
        if (text === "" || PluginHub.wasToasted(text)) return
        var lines = text.split("\n")
        var title = lines[0], body = lines.slice(1).join(" ")
        var last = items.length ? items[0] : null
        // LG sometimes delivers one toast twice in a row.
        if (last && last.title === title && last.body === body && Date.now() - last.time < 5000) return
        post({ source: "TV", icon: "info", title: title, body: body })
    }
    property Service toastSvc: Service {
        appId: LS.appId
        onResponse: { try { var m = JSON.parse(payload); if (m && m.message) log.fromToast(m) } catch (e) { } }
        Component.onCompleted: call("luna://com.webos.notification", "/getToastNotification", JSON.stringify({ subscribe: true }))
    }
}
