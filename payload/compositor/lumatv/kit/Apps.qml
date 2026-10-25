pragma Singleton
import QtQuick 2.4
import WebOSServices 1.0

// What the TV can launch, for the all-apps library behind the Apps tile, the search and the voice bar: the app
// manager's launch points (`applicationManager.launchPointsList`, the list Home's dock starts from) reduced to the
// apps a person opens, with the owner's choices laid over them (`theme.apps`: `hidden`, a list of ids, and `names`,
// id -> the name he gave it; both are his, in his theme.json).
//
// A launch point is not always an app: the HDMI inputs are launch points of type "bookmark" and belong to the input
// picker; LG keeps its own system apps (Gallery+, Camera, Media Player, Multi View...) off Home with `hidden`, and the
// library follows it, except for Live TV, which stays (it is rarely used, and when it is, it is wanted here). LG's own
// store is the Apps tile's app; the library lists it as "LG Content Store" so installs stay reachable.
//
// Order: what is on the dock, in the dock's order, then everything else by name.
QtObject {
    id: apps

    readonly property string storeId: "com.webos.app.discovery"
    readonly property var keepSystem: ["com.webos.app.livetv"]
    // The ids LumaTV never offers to uninstall: LG's, LumaTV's own, the Homebrew Channel.
    readonly property var protectedId: /^(com\.(webos|palm|lge)\.|org\.nphil\.luma|org\.webosbrew\.hbchannel$)/

    // What Home hands over (a Binding on its own theme, as for Lu), so this singleton imports nothing of the kit: two
    // singletons that import the directory make a cycle, and PluginHub already does.
    property string dockKey: ""                          // the dock's tile ids, "|"-joined, in order
    readonly property var dockIds: dockKey === "" ? [] : dockKey.split("|")
    property var prefs: ({})                             // theme.apps
    readonly property var hiddenIds: prefs.hidden || []
    readonly property var names: prefs.names || ({})

    readonly property string raw: LS.applicationManager.launchPointsList
    readonly property var launchPoints: {
        try { return JSON.parse(raw).launchPoints || [] } catch (e) { return [] }
    }

    function iconOf(p) {
        var u = p.extraLargeIcon || p.largeIcon || p.icon || p.mediumLargeIcon || ""
        return u === "" ? "" : /^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(u) ? u : "file://" + u
    }
    function titleOf(p) {
        var n = names[p.id]
        if (typeof n === "string" && n !== "") return n
        return p.id === storeId ? "LG Content Store" : (p.title || p.id)
    }

    // [{ id, launchPointId, title, iconUrl, params, hidden, onDock, dockIndex, removable, system }]
    readonly property var list: {
        var out = [], seen = {}
        for (var i = 0; i < launchPoints.length; i++) {
            var p = launchPoints[i]
            if ((p.lptype || "default") !== "default" || !p.id || seen[p.id]) continue
            if (p.systemApp && p.hidden && keepSystem.indexOf(p.id) < 0) continue
            seen[p.id] = true
            var dock = dockIds.indexOf(p.id)
            out.push({ id: p.id, launchPointId: p.launchPointId || (p.id + "_default"), title: titleOf(p), iconUrl: iconOf(p), params: p.params || ({}),
                       hidden: hiddenIds.indexOf(p.id) >= 0, onDock: dock >= 0, dockIndex: dock, system: !!p.systemApp,
                       removable: p.removable === true && !p.systemApp && !protectedId.test(p.id) })
        }
        out.sort(function(a, b) {
            if (a.onDock !== b.onDock) return a.onDock ? -1 : 1
            if (a.onDock) return a.dockIndex - b.dockIndex
            var x = a.title.toLowerCase(), y = b.title.toLowerCase()
            return x < y ? -1 : x > y ? 1 : 0
        })
        return out
    }
    readonly property var shown: list.filter(function(a) { return !a.hidden })

    function byId(id) {
        for (var i = 0; i < list.length; i++) if (list[i].id === id) return list[i]
        return null
    }
}
