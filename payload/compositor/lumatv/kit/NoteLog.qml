pragma Singleton
import QtQuick 2.4

// What the TV itself has told the person since it started: an update that is ready, a backup that finished, the answer
// to an AI access question. The Notification Centre (panels/Drawer.qml) shows this beside what the AI access audit and
// the plugins say. It lives in memory (a restart is a clean slate: the helper keeps the audit and the broker its
// own notifications). `post()` is the one way in, so a surface that has something to say does not know the drawer.
//
//   post({ source: "TV", icon: "download", title, body, time (ms, default now), actions: [{ id, label }] })
QtObject {
    id: log
    property var items: []
    property int serial: 0
    signal actionPicked(string id, string action)

    function post(n) {
        var list = items.slice()
        list.unshift({ id: "tv-" + (++serial), source: n.source || "TV", icon: n.icon || "info", title: n.title || "", body: n.body || "",
                       time: n.time || Date.now(), actions: n.actions || [] })
        items = list.slice(0, 50)
    }
    function dismiss(id) { items = items.filter(function(i) { return i.id !== id }) }
    function clear() { items = [] }
}
