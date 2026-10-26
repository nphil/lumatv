import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// App Permissions (LG: General › Privacy & Terms › App Permissions): which apps may use the camera, the microphone, nearby devices (Bluetooth) and
// files, a tab for each and in it an app with a switch, as LG's four lists are. The camera's and the nearby devices' tables are TV settings
// (`option.cameraResourcePermission` and `option.nearbyDevicePermission`: [{appId, permissions: [{resource, permission}]}]), written whole with the one
// flag changed; the microphone's and the files' are the permission service's (`ppm`: `queryPermissions` lists them, `setPermission {appId, privacyGroup,
// status}` changes one). An app is named as its launch point is, or by its id. Turning one on asks first (Allow, Block), as LG's does; turning one off
// does not. A tab the TV's description calls not visible is left empty.
PageList {
    id: page
    title: "App Permissions"
    back: "Support"
    backId: "support"
    tabs: ["Camera", "Microphone", "Nearby Devices", "Access File"]

    readonly property string ppm: "luna://com.webos.service.ppm"
    property var micTable: []              // [{appId, on}] from queryPermissions
    property var fileTable: []
    // per tab: the resource LG's tables name, what the question calls it, and where the switch is written (a setting, or a `ppm` privacy group)
    readonly property var kinds: [
        { resource: "camera", noun: "camera", ds: "option.cameraResourcePermission" },
        { resource: "mic", noun: "microphone", group: "microphone.privacy" },
        { resource: "ble", noun: "Nearby Devices", ds: "option.nearbyDevicePermission" },
        { resource: "usbFileAccess", noun: "file", group: "usbFileAccess.privacy", visibleDs: "option.fileapiResourcePermission" }
    ]

    LunaCall {
        id: luna
        onAnswered: {
            if (tag !== "perm" || !doc || doc.returnValue === false) return
            var mic = [], files = []
            var l = doc.privacyPermissions || []
            for (var i = 0; i < l.length; i++) {
                var t = l[i].permissionTable || {}
                if (t["microphone.privacy"]) mic.push({ appId: l[i].appId, on: t["microphone.privacy"] === "allow" })
                if (t["usbFileAccess.privacy"]) files.push({ appId: l[i].appId, on: t["usbFileAccess.privacy"] === "allow" })
            }
            page.micTable = mic
            page.fileTable = files
        }
    }
    Timer { id: reread; interval: 600; onTriggered: luna.call("perm", page.ppm, "queryPermissions", {}) }

    function refresh() {
        Sio.read("option", ["cameraResourcePermission", "nearbyDevicePermission", "fileapiResourcePermission"])
        luna.call("perm", ppm, "queryPermissions", {})
    }
    onOpened: refresh()

    function appTitle(appId) {
        var l = Apps.launchPoints
        for (var i = 0; i < l.length; i++) if (l[i].id === appId) return l[i].title || appId
        return appId
    }
    // LG's tables: [{appId, permissions: [{resource, permission}]}], the entry that names the resource kept
    function fromSettings(ds, resource) {
        var t = Sio.get(ds), out = []
        if (!t || !t.length) return out
        for (var i = 0; i < t.length; i++) {
            var p = t[i].permissions || []
            for (var j = 0; j < p.length; j++) if (p[j].resource === resource) { out.push({ appId: t[i].appId, on: p[j].permission === true }); break }
        }
        return out
    }
    function entries(i) {
        var k = kinds[i]
        if (i === 1) return micTable
        if (i === 3) return C.shown(Sio, k.visibleDs) ? fileTable : []
        return C.shown(Sio, k.ds) ? fromSettings(k.ds, k.resource) : []
    }

    function setPermission(i, appId, on) {
        var k = kinds[i]
        if (k.ds) {
            var t = JSON.parse(JSON.stringify(Sio.get(k.ds) || []))
            for (var a = 0; a < t.length; a++) {
                var p = t[a].permissions || []
                if (t[a].appId === appId) for (var b = 0; b < p.length; b++) if (p[b].resource === k.resource) p[b].permission = on
            }
            var w = {}
            w[k.ds.substring(k.ds.indexOf(".") + 1)] = t
            Sio.writeMany("option", w)
            return
        }
        luna.act(ppm, "setPermission", { appId: appId, privacyGroup: k.group, status: on ? "allow" : "deny" })
        var shown = (i === 1 ? micTable : fileTable).map(function(e) { return e.appId === appId ? { appId: e.appId, on: on } : e })
        if (i === 1) micTable = shown
        else fileTable = shown
        reread.restart()
    }
    function appRow(i, e) {
        return { kind: "toggle", tab: i, id: "app:" + i + ":" + e.appId, icon: "layout-grid", label: page.appTitle(e.appId),
                 on: function() { return e.on },
                 act: function(v) {
                     if (!v) { page.setPermission(i, e.appId, false); return }
                     C.ask(page, { icon: "shield-check", title: "Allow " + page.kinds[i].noun + " access?", body: page.appTitle(e.appId), yes: "Allow", no: "Block" },
                           function() { page.setPermission(i, e.appId, true) })()
                 } }
    }
    function build() {
        var out = []
        for (var i = 0; i < kinds.length; i++) {
            var list = entries(i)
            if (list.length === 0) out.push(R.note("No app has asked.", i))
            for (var j = 0; j < list.length; j++) out.push(appRow(i, list[j]))
        }
        return out
    }
    rows: build()
}
