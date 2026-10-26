import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Bluetooth devices (LG: General › External Devices › Connect Bluetooth Controller, and Sound › Sound Out › Bluetooth Device): the
// devices the TV knows, the ones it can see, and a search. `kind` says which of LG's two services answers: "audio" is `btaudiosrc`
// (speakers and headphones: getPairedDevices, startDiscovery, connect) and "hid" is `bthidmanager` (keyboards, mice, game pads:
// getPairedDeviceLists, requestDiscovery, requestConnect). Both are read as LG's own screens read them and shown the same way: registered
// devices first (OK opens the device), then connectable ones (OK pairs and connects), then Search. Searching stops when the page is left, as
// LG's does. Connecting and deleting are done here at the remote only; nothing of this is offered to an assistant.
PageList {
    id: page
    property string kind: "audio"
    readonly property bool audio: kind === "audio"
    property var paired: []                // [{address, name, connected, state}]
    property var found: []                 // [{address, name}]
    property bool searching: false
    property string busy: ""               // the address being connected
    title: audio ? "Bluetooth Speakers" : "Bluetooth Devices"
    back: audio ? "Sound" : "Inputs & devices"
    readonly property string svc: audio ? "luna://com.webos.service.btaudiosrc" : "luna://com.webos.service.bthidmanager"

    LunaCall {
        id: luna
        onAnswered: {
            if (!doc || doc.returnValue === false) return
            if (tag === "paired") page.readPaired(doc)
            else if (tag === "found") page.readFound(doc)
            else if (tag === "status" || tag === "connect") {
                // a change of state (or the answer to a connect): read the list again; a connection that failed or ended is no longer pending
                if (tag === "status" && (doc.status === "fail" || doc.status === "disconnected")) page.busy = ""
                luna.call("list", page.svc, page.audio ? "getPairedDevices" : "getPairedDeviceLists", {})
            } else if (tag === "list") page.readPaired(doc)
        }
    }
    function readPaired(doc) {
        var out = []
        if (page.audio) {
            var d = doc.devices || []
            for (var i = 0; i < d.length; i++) if (d[i].display !== false) out.push({ address: d[i].address, name: d[i].name, state: d[i].state || "disconnected", connected: d[i].state === "connected" })
        } else {
            var l = doc.deviceLists || {}
            var keys = Object.keys(l).sort()
            // the service answers one entry for each of its slots: an empty one is "Not Paired" with the address "N/A"
            for (var k = 0; k < keys.length; k++) {
                var e = l[keys[k]]
                if (e.isPaired === false || !e.deviceAddr || e.deviceAddr === "N/A") continue
                out.push({ address: e.deviceAddr, name: e.deviceName, state: e.isConnected ? "connected" : "disconnected", connected: !!e.isConnected })
            }
        }
        paired = out
        if (busy !== "") for (var b = 0; b < out.length; b++) if (out[b].address === busy && out[b].connected) busy = ""
    }
    function readFound(doc) {
        var dev = page.audio ? (doc.state === "started" ? doc.device : null) : (doc.address ? doc : null)
        if (page.audio && doc.state === "stopped") { searching = false; return }
        if (!page.audio && doc.status === "endDiscovery") { searching = false; return }
        if (!page.audio && doc.status === "startDiscovery") { searching = true; return }
        if (!dev || !dev.address) return
        for (var i = 0; i < paired.length; i++) if (paired[i].address === dev.address) return
        for (var j = 0; j < found.length; j++) if (found[j].address === dev.address) return
        found = found.concat([{ address: dev.address, name: dev.name || dev.address }])
    }

    function refresh() {
        Sio.read("sound", ["deviceCntOfBtGeneral"])
        found = []
        searching = false
        if (audio) {
            luna.subscribe("paired", svc, "getPairedDevices", {})
            luna.subscribe("status", svc, "getDeviceStatus", {})
        } else {
            luna.call("list", svc, "getPairedDeviceLists", {})
            luna.subscribe("status", svc, "subscribeDevicesStatus", {})
        }
    }
    function search() {
        found = []
        searching = true
        if (audio) luna.subscribe("found", svc, "startDiscovery", {})
        else { luna.subscribe("found", svc, "subscribeDiscovery", {}); luna.call("go", svc, "requestDiscovery", {}) }
    }
    function stopSearch() {
        if (!searching) return
        searching = false
        luna.close("found")
        if (!audio) luna.call("stop", svc, "cancelDiscovery", {})
    }
    function connectTo(d) {
        busy = d.address
        if (audio) luna.call("connect", svc, "connect", { address: d.address })
        else luna.call("connect", svc, "requestConnect", { name: d.name, address: d.address })
        found = found.filter(function(x) { return x.address !== d.address })
    }
    onOpened: refresh()                     // a search and a subscription are for the page that is up, not for one built in the background
    onVisibleChanged: if (!visible) { stopSearch(); luna.close("paired"); luna.close("status") }

    function stateText(d) {
        return busy === d.address ? "Connecting" : d.state === "connected" ? "Connected" : d.state === "connecting" ? "Connecting" : d.state === "disconnecting" ? "Disconnecting" : ""
    }
    function deviceRow(d) {
        return { kind: "value", id: "dev:" + d.address, icon: audio ? "headphones" : "gamepad-2", label: d.name, value: page.stateText(d), chevron: true,
                 act: function() { page.host.openPage("btdevice", { kind: page.kind, address: d.address, name: d.name, connected: d.connected, back: page.title, backId: page.audio ? "btaudio" : "bthid" }) } }
    }
    function foundRow(d) {
        return { kind: "value", id: "found:" + d.address, icon: audio ? "headphones" : "gamepad-2", label: d.name, value: page.busy === d.address ? "Connecting" : "", chevron: false,
                 act: function() { page.connectTo(d) } }
    }
    function build() {
        var out = []
        if (audio) out.push(C.choice(Sio, { ds: "sound.deviceCntOfBtGeneral", icon: "headphones", label: "Speaker Connection", names: "deviceCnt", page: page }))
        out.push(R.head("Registered Devices (" + paired.length + ")"))
        if (!paired.length) out.push(R.note("There are no paired devices."))
        for (var i = 0; i < paired.length; i++) out.push(deviceRow(paired[i]))
        out.push(R.head("Connectable Devices (" + found.length + ")"))
        for (var j = 0; j < found.length; j++) out.push(foundRow(found[j]))
        out.push(C.action({ id: "search", icon: "search", label: searching ? "Searching" : "Search", button: searching ? "Stop" : "Search",
                            act: function() { if (page.searching) page.stopSearch(); else page.search() } }))
        return out
    }
    rows: build()
}
