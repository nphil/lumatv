import QtQuick 2.4
import "../kit"
import "PluginRows.js" as P
import "."

// Home Assistant, a page of Home & look: the settings page of LumaShow's Home Assistant plugin (plugins/home-assistant), drawn by the
// TV from the schema the helper serves for it (`GET /v1/settings/home-assistant`, PluginRows.js) and written through the same
// route, so a choice made here is the plugin's setting, as it is on the Show. The pickers (the room, its lights and scenes, the
// doorbell, the presence sensors) are lists the plugin makes from Home Assistant: the page asks the helper for each provider
// (`GET /v1/plugins/home-assistant/choices/<provider>`), which asks the plugin.
//
// What is not here, on purpose: the address and the access token. They are the owner's alone, given from a shell on the TV
// (`lumatv-helper ha-link`, the token on standard input) so that no key press, no route of the helper and no plugin can set either;
// this page shows the address and whether a token is kept, and can forget them (a second OK: it only ever takes access away).
PageList {
    id: page
    title: "Home Assistant"
    back: "Home & look"
    backId: "look"
    tabs: hs.known ? P.tabs(hs.schema, { connection: "Connection", room: "Room", doorbell: "Doorbell", presence: "Presence", notifications: "Alerts" }) : ["Connection"]

    readonly property string pid: PluginHub.haId
    readonly property string haState: PluginHub.haState
    chip: PluginHub.haWords
    chipTone: haState === "connected" ? "positive" : haState === "refused" || haState === "inert" ? "danger" : haState === "off" || haState === "unlinked" || haState === "connecting" ? "info" : "warning"
    summary: PluginHub.haWords

    HelperSettings { id: hs; owner: PluginHub.haId }

    // What the plugin listed for each provider: provider -> {state, rows, more}. A provider is asked again when the page is opened,
    // when the link comes up and when the room changes (its lights and sensors are the room's); the last answer stays until the next.
    property var listed: ({})
    property string roomAsked: ""
    property bool armed: false                                  // "Forget" waits for a second OK

    function ask(provider) {
        Helper.request("GET", "/v1/plugins/" + pid + "/choices/" + provider + "?limit=64", undefined, function(status, doc) {
            var next = {}
            for (var k in listed) next[k] = listed[k]
            var got = doc && doc.ok && doc.page ? doc.page : null
            next[provider] = got ? { state: got.state, rows: got.rows || [], more: !!got.more } : { state: "failed", rows: [], more: false }
            listed = next
        })
    }
    function askAll() {
        if (!hs.known) return
        var names = P.providers(hs.schema)
        for (var i = 0; i < names.length; i++) ask(names[i])
        roomAsked = String(hs.get("room") || "")
    }
    function refresh() {
        hs.refresh()
        armed = false
    }
    onOpened: refresh()
    Component.onCompleted: refresh()
    Connections { target: hs; onKnownChanged: page.askAll() }
    // The room chosen moves what the lights and the sensors are chosen among.
    Connections {
        target: hs
        onVersionChanged: {
            var room = String(hs.get("room") || "")
            if (hs.known && room !== page.roomAsked) page.askAll()
        }
    }
    onHaStateChanged: if (haState === "connected" || haState === "unlinked") hs.refresh()
    Connections { target: PluginHub; onHaStateChanged: if (PluginHub.haState === "connected") page.askAll() }
    // The rows of every tab are one list, and a picker's list grows and shrinks as the plugin answers: the focus follows its row.
    property string stopId: ""
    onStopChanged: {
        armed = false
        stopId = stop >= 0 && rows[stop] && rows[stop].id ? String(rows[stop].id) : ""
    }
    onRowsChanged: {
        if (stop < 0 || stopId === "") return
        for (var i = 0; i < rows.length; i++) if (rows[i].id === stopId && focusable(i)) { if (i !== stop) enter(i); return }
        if (!focusable(stop)) enter(first())
    }
    Timer { interval: 6000; running: page.armed; onTriggered: page.armed = false }

    function forget() {
        armed = false
        Helper.post("/v1/plugins/" + pid + "/actions/lease.ha.forget", {})
    }

    // What the rows lend PluginRows.js.
    readonly property var lend: ({
        get: function(key) { return hs.get(key) },
        write: function(key, value) { hs.write(key, value) },
        fact: function(capability) { var k = String(capability).replace("plugin." + PluginHub.haId + ".", ""); return PluginHub.haFacts[k] },
        choices: function(provider) { return page.listed[provider] }
    })

    // The whole page again only when its shape changes (the schema arrives, a list arrives, the plugin comes or goes): a value that
    // changes is read by the row that shows it.
    function build() {
        var own = [
            { kind: "value", tab: 0, id: "address", info: true, chevron: false, icon: "globe", label: "Address",
              help: "Set with lumatv-helper ha-link, in a shell on the TV",
              value: function() { return PluginHub.haLink.address || "Not set" } },
            { kind: "value", tab: 0, id: "token", info: true, chevron: false, icon: "key-round", label: "Access token",
              help: "Kept on this TV and never shown. Set with lumatv-helper ha-link, in a shell on the TV",
              value: function() { return PluginHub.haLink.token ? "Saved" : "Not saved" } },
            { kind: "action", tab: 0, id: "forget", icon: "unplug", label: "Forget link", danger: true,
              help: "Deletes the saved address and token",
              button: function() { return page.armed ? "Sure?" : "Forget" },
              enabled: function() { return !!PluginHub.haLink.token && PluginHub.haWords !== "Not switched on" },
              act: function() { if (page.armed) page.forget(); else page.armed = true } }
        ]
        if (!hs.known) return own
        // The first tab's rows come first (its focus never moves), then the rest in the schema's order.
        var all = P.rows(lend, hs.schema)
        return all.filter(function(r) { return r.tab === 0 }).concat(own, all.filter(function(r) { return r.tab !== 0 }))
    }
    rows: build()
}
