pragma Singleton
import QtQuick 2.4
import WebOSServices 1.0
import "."

// What the plugin broker (the helper's `plugins` module, LumaShow plugin API level 1) puts on screen, as lists a surface
// can bind to. The helper publishes the roster (`plugins`) and, per plugin that is connected, its cards, tiles and
// notifications (`plugin.<id>.cards|tiles|notifications`); every contribution has already been validated there, so
// nothing here checks a plugin's words, it only draws them and routes the person's taps back (`act`): a tap is
// `POST /v1/plugins/<id>/actions/<action>`, which the broker turns into the plugin's `action.invoke`.
//
//   cards  [{ pluginId, plugin, id, revision, expiresAt, size: "full"|"half", priority, content, open, actions }]
//          content: { type: "facts", title, icon, role, facts: [{label, value}], switch? }
//                 | { type: "event", title, detail?, icon, role, at? }
//                 | { type: "media", title, subtitle?, state, position_secs?, duration_secs? }
//   tiles  [{ pluginId, plugin, id, label, icon, state: {type: on|off|observed|unknown|unavailable}, action, target?, hint? }]
//   notes  [{ pluginId, plugin, id, channel, revision, receivedAt, expiresAt, title, body, actions }]
//
// A new notification is also posted as an LG toast (what the person is told about right now); the first answer
// after the helper comes up only marks what is already there, so a restart never replays old ones. Nothing here
// installs, enables or removes a plugin: switching one on is the owner's act (roster file, Principle 0).
QtObject {
    id: hub

    property bool toasts: true                          // theme.plugins.toasts
    readonly property var list: (Helper.topics.plugins && Helper.topics.plugins.plugins) || []
    readonly property string problem: (Helper.topics.plugins && Helper.topics.plugins.problem) || ""
    readonly property var connected: list.filter(function(p) { return p.state === "connected" })
    readonly property var cards: gather("cards", function(it, p) {
        var c = it.card || {}
        return { pluginId: p.id, plugin: p.name || p.id, id: it.id, revision: it.revision, expiresAt: it.expiresAt, size: c.size || "half",
                 priority: c.priority || "new", content: c.content || {}, open: c.open, actions: c.actions || [] }
    })
    readonly property var tiles: gather("tiles", function(it, p) {
        return { pluginId: p.id, plugin: p.name || p.id, id: it.id, label: it.label, icon: it.icon, state: it.state || { type: "unknown" },
                 action: it.action, target: it.target, hint: it.hint }
    })
    readonly property var notes: gather("notifications", function(it, p) {
        var n = it.post || {}
        return { pluginId: p.id, plugin: p.name || p.id, id: it.id, channel: it.channel, revision: it.revision, receivedAt: it.receivedAt,
                 expiresAt: it.expiresAt, title: n.title || "", body: n.body || "", actions: n.actions || [] }
    })

    function gather(kind, shape) {
        var out = []
        for (var i = 0; i < connected.length; i++) {
            var topic = Helper.topics["plugin." + connected[i].id + "." + kind]
            var items = (topic && topic.items) || []
            for (var j = 0; j < items.length; j++) out.push(shape(items[j], connected[i]))
        }
        return out
    }

    // The plugin's icon names are its own ("base.icons/message"); the closest of the icons this TV carries, or a plug.
    readonly property var iconMap: ({
        message: "bell", notification: "bell", home: "house", light: "lightbulb", lamp: "lamp", lock: "lock", camera: "camera",
        music: "audio-lines", speaker: "boom-box", tv: "monitor", sensor: "activity", clock: "clock", timer: "hourglass", info: "info",
        warning: "circle-alert", check: "circle-check", wifi: "wifi", mic: "mic", heart: "heart-pulse", leaf: "leaf", moon: "moon", sun: "sun"
    })
    function icon(name) {
        var n = String(name || "").split("/").pop().toLowerCase()
        return iconMap[n] || "plug"
    }

    function act(pluginId, action, target) { return Helper.pluginAction(pluginId, action, target) }
    function tileState(pluginId, id) {
        for (var i = 0; i < tiles.length; i++) if (tiles[i].pluginId === pluginId && tiles[i].id === id) return tiles[i].state
        return { type: "unknown" }
    }
    function tileOn(s) { return s.type === "on" || (s.type === "observed" && s.on === true) }
    function unavailableText(s) { return s.reason === "offline" ? "Offline" : s.reason === "not_permitted" ? "Not allowed" : s.reason === "no_capability" ? "Not on this TV" : "Not available" }

    // Quick-panel rows (PageList descriptors): a tile with an on/off state is a toggle, any other a button that runs its
    // action. An observed tile shows its state and cannot be flipped from here (the plugin owns it).
    function tileRows() {
        var out = []
        for (var i = 0; i < tiles.length; i++) out.push(tileRow(tiles[i].pluginId, tiles[i].id))
        return out
    }
    function tileRow(pid, id) {
        function me() { for (var i = 0; i < hub.tiles.length; i++) if (hub.tiles[i].pluginId === pid && hub.tiles[i].id === id) return hub.tiles[i]; return null }
        var t = me() || {}
        var stateful = t.state && (t.state.type === "on" || t.state.type === "off" || t.state.type === "observed")
        return {
            kind: stateful ? "toggle" : "value", id: "plugin." + pid + "." + id, icon: hub.icon(t.icon), label: t.label || id,
            detail: function() { var x = me(); if (!x) return ""; return x.state.type === "unavailable" ? hub.unavailableText(x.state) : (x.hint || x.plugin) },
            value: "",
            enabled: function() { var x = me(); return !!x && x.state.type !== "unavailable" && x.state.type !== "observed" },
            on: function() { var x = me(); return !!x && hub.tileOn(x.state) },
            act: function() { var x = me(); if (x) hub.act(x.pluginId, x.action, x.target) }
        }
    }

    // ---- toasts for notifications that arrive
    property var announced: ({})
    property bool primed: false
    readonly property bool ready: Helper.up
    onReadyChanged: if (ready) primer.restart()
    property Timer primer: Timer { interval: 1500; onTriggered: { hub.mark(); hub.primed = true } }
    property Connections link: Connections {
        target: Helper
        onUpdated: if (topic.indexOf("plugin.") === 0 && topic.slice(-14) === ".notifications" && hub.primed) hub.announce()
    }
    function mark() { var a = {}; for (var i = 0; i < notes.length; i++) a[notes[i].pluginId + "/" + notes[i].id + "/" + notes[i].revision] = true; announced = a }
    function announce() {
        var a = {}, now = Date.now() / 1000
        for (var k in announced) a[k] = true
        for (var i = 0; i < notes.length; i++) {
            var n = notes[i], key = n.pluginId + "/" + n.id + "/" + n.revision
            if (a[key]) continue
            a[key] = true
            if (toasts && (!n.expiresAt || n.expiresAt > now)) toast(n)
        }
        announced = a
    }
    // `com.webos.app.home` is the source LG's own toasts from Home carry; the first line of `message` is the title, the rest the subtitle.
    function toast(n) {
        LS.adhoc.call("luna://com.webos.notification", "/createToast",
                      JSON.stringify({ sourceId: "com.webos.app.home", message: n.body ? n.title + "\n" + n.body : n.title, noaction: true, persistent: false }))
    }
}
