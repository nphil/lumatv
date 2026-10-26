pragma Singleton
import QtQuick 2.4
import WebOSServices 1.0

// What the plugin broker (the helper's `plugins` module, LumaShow plugin API level 1) puts on screen, as lists a surface
// can bind to. The helper publishes the roster (`plugins`: the plugins the owner switched on, and the helper's own, the
// weather, news, clock and ambient light plugins, as `firstParty`) and, per plugin that is connected, its cards, tiles and
// notifications (`plugin.<id>.cards|tiles|notifications`); every contribution has already been validated there, so nothing
// here checks a plugin's words, it only draws them and routes the person's taps back (`act`): a tap is
// `POST /v1/plugins/<id>/actions/<action>`, which the broker turns into the plugin's `action.invoke`.
//
//   cards  [{ pluginId, plugin, firstParty, id, revision, createdAt, expiresAt, size: "full"|"half", priority, content, open, actions }]
//          content: { type: "facts", title, icon, role, facts: [{label, value}], switch? }
//                 | { type: "event", title, detail?, icon, role, at? }
//                 | { type: "media", title, subtitle?, state, position_secs?, duration_secs? }
//   tiles  [{ pluginId, plugin, id, label, icon, state: {type: on|off|observed|unknown|unavailable}, action, target?, hint? }]
//   notes  [{ pluginId, plugin, firstParty, id, channel, channelTitle, channelIcon, revision, receivedAt, expiresAt, title, body, actions }]
//
// The lists are assigned only when their content changes. `Helper.topics` is replaced by every event of the helper (the
// receiver's poll alone is one every couple of seconds), so a binding on it would rebuild every surface that reads a list, hidden or
// not; here a list is recomputed when a `plugin` topic changes and kept as it was when it comes out the same.
//
// A new notification is also posted as an LG toast (what the person is told about right now); the first answer
// after the helper comes up only marks what is already there, so a restart never replays old ones. Nothing here
// installs, enables or removes a plugin: switching one on is the owner's act (roster file, Principle 0).
QtObject {
    id: hub

    property bool toasts: true                          // theme.plugins.toasts
    property var list: []                               // the plugins the owner switched on, then the helper's own
    property string problem: ""
    property var connected: []
    property var cards: []
    property var tiles: []
    property var notes: []
    property var haLink: ({ state: "off" })             // the `ha.link` topic: where the helper's Home Assistant lease is (see `haState`)
    property var haFacts: ({})                          // what the Home Assistant plugin says of itself (`plugin.home-assistant.facts`)

    // What Home's At-home slot draws as chips: every plugin's cards but the helper's own that Home draws in full elsewhere (the
    // weather, news and clock widgets), the most urgent first.
    readonly property var homeExcluded: ["weather", "news", "clock"]
    readonly property var homeCards: {
        var rank = { ringing: 0, running: 1, "new": 2 }
        var out = cards.filter(function(c) { return homeExcluded.indexOf(c.pluginId) < 0 })
        return out.map(function(c, i) { return { c: c, i: i } }).sort(function(a, b) {
            var d = (rank[a.c.priority] === undefined ? 3 : rank[a.c.priority]) - (rank[b.c.priority] === undefined ? 3 : rank[b.c.priority])
            return d !== 0 ? d : a.i - b.i
        }).map(function(x) { return x.c })
    }

    function same(a, b) { return JSON.stringify(a) === JSON.stringify(b) }
    function refresh() {
        var t = Helper.topics.plugins || {}
        var l = (t.plugins || []).concat(t.firstParty || [])
        if (!same(l, list)) list = l
        var p = t.problem || ""
        if (p !== problem) problem = p
        var c = l.filter(function(x) { return x.state === "connected" })
        if (!same(c, connected)) connected = c
        var cs = gather("cards", function(it, pl) {
            var k = it.card || {}
            return { pluginId: pl.id, plugin: pl.name || pl.id, firstParty: !!pl.firstParty, id: it.id, revision: it.revision, createdAt: it.createdAt,
                     expiresAt: it.expiresAt, size: k.size || "half", priority: k.priority || "new", content: k.content || {}, open: k.open, actions: k.actions || [] }
        })
        if (!same(cs, cards)) cards = cs
        var ts = gather("tiles", function(it, pl) {
            return { pluginId: pl.id, plugin: pl.name || pl.id, id: it.id, label: it.label, icon: it.icon, state: it.state || { type: "unknown" },
                     action: it.action, target: it.target, hint: it.hint }
        })
        if (!same(ts, tiles)) tiles = ts
        var ns = gather("notifications", function(it, pl) {
            var n = it.post || {}
            return { pluginId: pl.id, plugin: pl.name || pl.id, firstParty: !!pl.firstParty, id: it.id, channel: it.channel, channelTitle: it.channelTitle || "",
                     channelIcon: it.channelIcon || "", revision: it.revision, receivedAt: it.receivedAt, expiresAt: it.expiresAt,
                     title: n.title || "", body: n.body || "", actions: n.actions || [] }
        })
        if (!same(ns, notes)) notes = ns
        var link = Helper.topics["ha.link"] || { state: "off" }
        if (!same(link, haLink)) haLink = link
        var facts = (Helper.topics["plugin." + haId + ".facts"] || {}).facts || {}
        if (!same(facts, haFacts)) haFacts = facts
    }
    function gather(kind, shape) {
        var out = []
        for (var i = 0; i < connected.length; i++) {
            var topic = Helper.topics["plugin." + connected[i].id + "." + kind]
            var items = (topic && topic.items) || []
            for (var j = 0; j < items.length; j++) out.push(shape(items[j], connected[i]))
        }
        return out
    }
    Component.onCompleted: refresh()

    // ---- Home Assistant: the one plugin the At-home lane and a Settings page of its own are about
    readonly property string haId: "home-assistant"
    readonly property var haPlugin: {
        for (var i = 0; i < list.length; i++) if (list[i].id === haId) return list[i]
        return null
    }
    // Where it stands, in one word: `off` (not switched on), `inert` (switched on but this TV will not run it), `waiting` (its program
    // is not up), then what the helper's lease says: `unlinked` (no address or token saved), `refused` (Home Assistant did not take
    // the token), `offline`, `connecting` or `connected`.
    readonly property string haState: {
        var p = haPlugin
        if (!p) return "off"
        if (p.state === "inert") return "inert"
        if (p.state !== "connected") return "waiting"
        var s = haLink.state
        return s === "auth_failed" ? "refused" : (!s || s === "off" || s === "unconfigured") ? "unlinked" : s
    }
    // What the At-home lane says when no plugin has a card: a title, the line under it, and the tone of its glyph.
    readonly property var laneNone: {
        var where = "Settings \u203a Home Assistant"
        switch (haState) {
        case "inert": return { title: "Home Assistant can't run on this TV", sub: "See " + where, role: "warning" }
        case "waiting": return { title: haPlugin && haPlugin.reason ? "Home Assistant isn't running" : "Starting Home Assistant", sub: haPlugin && haPlugin.reason ? "See " + where : "", role: haPlugin && haPlugin.reason ? "warning" : "" }
        case "refused": return { title: "Home Assistant refused the token", sub: "Link it again in " + where, role: "danger" }
        case "offline": return { title: "Can't reach Home Assistant", sub: "It is tried again. See " + where, role: "warning" }
        case "connecting": return { title: "Connecting to Home Assistant", sub: "", role: "" }
        case "connected": return { title: "Nothing to show yet", sub: "Pick a room in " + where, role: "" }
        default: return { title: "Home Assistant isn't linked yet", sub: "Link it in " + where, role: "" }
        }
    }
    // A short phrase for a row that opens the page.
    readonly property string haWords: {
        switch (haState) {
        case "off": return "Not switched on"
        case "inert": return "Cannot run"
        case "waiting": return "Starting"
        case "unlinked": return "Not linked"
        case "refused": return "Token refused"
        case "offline": return "Offline"
        case "connecting": return "Connecting"
        default: return "Connected"
        }
    }

    // The plugin's card, whoever it is.
    function card(pluginId, id) {
        for (var i = 0; i < cards.length; i++) if (cards[i].pluginId === pluginId && cards[i].id === id) return cards[i]
        return null
    }
    // The glance line of a card: its title and what it says first (a facts card: its first fact; an event: the detail).
    function glance(c) {
        var k = c.content || {}
        if (k.type === "facts") return { title: k.title || "", sub: (k.facts && k.facts.length ? k.facts[0].value : "") }
        if (k.type === "event") return { title: k.title || "", sub: k.detail || "" }
        if (k.type === "media") return { title: k.title || "", sub: k.subtitle || "" }
        return { title: k.title || "", sub: "" }
    }

    // The plugin's icon names are its own ("base.icons/message"); the closest of the icons this TV carries, or a plug.
    readonly property var iconMap: ({
        message: "bell", notification: "bell", home: "house", light: "lightbulb", lamp: "lamp", lock: "lock", camera: "camera",
        music: "audio-lines", speaker: "boom-box", tv: "monitor", sensor: "activity", clock: "clock", timer: "hourglass", info: "info",
        "home-assistant": "house-wifi", doorbell: "bell",
        warning: "circle-alert", check: "circle-check", wifi: "wifi", mic: "mic", heart: "heart-pulse", leaf: "leaf", moon: "moon", sun: "sun",
        newspaper: "scroll-text", news: "scroll-text", cloud: "sun-dim", weather: "sun-dim", bot: "bot", download: "download"
    })
    // ...and the names it already has (icons/lucide/index.json) are drawn as they are.
    readonly property var carried: [
        "activity", "airplay", "antenna", "aperture", "arrow-big-up", "arrow-left", "audio-lines", "audio-waveform",
        "badge-check", "ban", "bell", "bell-off", "blend", "boom-box", "bot", "brain", "cable", "camera", "cast", "check",
        "chevron-down", "chevron-left", "chevron-right", "circle", "circle-alert", "circle-check", "circle-help", "circle-x",
        "clapperboard", "clipboard-list", "clock", "cog", "contrast", "corner-down-left", "cpu", "delete", "disc-3", "download",
        "drama", "ellipsis", "ethernet-port", "external-link", "eye", "eye-off", "fast-forward", "file-text", "film",
        "gamepad-2", "gauge", "gem", "globe", "globe-2", "hard-drive", "hash", "headphones", "heart-pulse", "history",
        "hourglass", "house", "house-wifi", "image", "info", "key-round", "keyboard", "lamp", "languages", "layout-grid", "leaf",
        "lightbulb", "list-checks", "list-video", "lock", "lock-keyhole", "log-out", "message-square-warning", "mic", "minus",
        "monitor", "monitor-cog", "monitor-speaker", "moon", "moon-star", "mountain", "mouse-pointer-2", "music", "network",
        "octagon-alert", "package", "paintbrush", "palette", "pause", "picture-in-picture", "play", "plug", "plus", "popcorn",
        "power", "projector", "radio", "rectangle-horizontal", "refresh-cw", "rotate-ccw", "router", "satellite-dish",
        "scan-eye", "scroll-text", "search", "server", "settings", "shield", "shield-alert", "shield-check", "shield-off",
        "signal", "skip-forward", "sliders-horizontal", "sliders-vertical", "smartphone", "space", "sparkles", "speaker",
        "square", "square-arrow-out-up-right", "square-check", "star", "sun", "sun-dim", "sun-medium", "sunrise",
        "text-cursor-input", "theater", "timer", "toggle-right", "trash-2", "triangle-alert", "tv", "tv-minimal",
        "tv-minimal-play", "type", "undo-2", "unplug", "usb", "user", "volume-1", "volume-2", "volume-x", "wallpaper",
        "wand-sparkles", "waves", "wifi", "wrench", "x", "zap"
    ]
    function icon(name) {
        var n = String(name || "").split("/").pop().toLowerCase()
        return iconMap[n] || (carried.indexOf(n) >= 0 ? n : "plug")
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

    // A toast this hub raised is remembered for a while, so the log of what LG's notification service said (NoteLog) does not list
    // it a second time beside the plugin's own entry in the drawer.
    property var toasted: ({})
    function wasToasted(message) {
        var t = toasted[message]
        return t !== undefined && Date.now() - t < 30000
    }

    // ---- toasts for notifications that arrive
    property var announced: ({})
    property bool primed: false
    readonly property bool ready: Helper.up
    onReadyChanged: if (ready) primer.restart()
    property Timer primer: Timer { interval: 1500; onTriggered: { hub.mark(); hub.primed = true } }
    property Connections link: Connections {
        target: Helper
        onUpdated: {
            if (topic === "plugins" || topic === "ha.link" || topic.indexOf("plugin.") === 0) hub.refresh()
            if (topic.indexOf("plugin.") === 0 && topic.slice(-14) === ".notifications" && hub.primed) hub.announce()
        }
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
        var message = n.body ? n.title + "\n" + n.body : n.title
        var t = {}
        for (var k in toasted) if (Date.now() - toasted[k] < 30000) t[k] = toasted[k]
        t[message] = Date.now()
        toasted = t
        LS.adhoc.call("luna://com.webos.notification", "/createToast",
                      JSON.stringify({ sourceId: "com.webos.app.home", message: message, noaction: true, persistent: false }))
    }
}
