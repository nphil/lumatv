pragma Singleton
import QtQuick 2.4
import WebOSServices 1.0

// The TV's settings service as the Settings pages see it. A page names what it shows as
// "category.key" (the same names LG's Settings app and the stock services use, for example
// picture.backlight or general.remotePointer); this reads them with getSystemSettings and keeps the
// answers in `values`, so a row binds to `Sio.values["picture.backlight"]`, and writes one back with
// setSystemSettings. A write shows at once (the page is never waiting for the service to say what
// the owner just chose) and a later answer replaces it.
//
// What the service does not answer stays absent: `has(ds)` is false for it, and the pages dim the
// row ("Not available on this TV") instead of showing a made-up value. Picture settings belong to a
// "dimension" (picture mode, dynamic range, input): the service names the dimension it answered for,
// and a write carries the one the last answer had.
//
// Each read asks twice, for the values and for the service's description of them (`getSystemSettingDesc`, the `ui`
// LG's own Settings reads to dim a row): a setting whose description says it cannot be changed now stays on screen
// and is dimmed, `usable(ds)` is false for it, and a row that writes it does not take focus. A description that never
// comes leaves the row usable. A few settings are per app (the aspect ratio): `read` and `write` take that app id.
//
// Reads go one at a time (the service answers by method name, not by call) and each gets 2 s.
QtObject {
    id: sio

    readonly property string service: "luna://com.webos.service.settings"
    property var values: ({})                  // "category.key" -> the value the service gave
    property var seen: ({})                    // "category.key" -> true once answered
    property var descs: ({})                   // "category.key" -> {ui: {visible, active}} from the service's description
    property var dimensions: ({})              // category -> {pictureMode, dynamicRange, input} of the last answer
    property var queue: []                     // [{category, keys}] waiting
    property var current: null                 // the read in flight
    property int answers: 0                    // counts answers, for a page that wants to know it has heard

    function get(ds) { return values[ds] }
    function has(ds) { return seen[ds] === true }
    function num(ds, fallback) {
        var v = parseFloat(values[ds])
        return isNaN(v) ? (fallback === undefined ? 0 : fallback) : v
    }
    function is(ds, on) { return String(values[ds]) === (on === undefined ? "on" : on) }
    // The service says the setting may be shown and changed (true until a description says otherwise).
    function usable(ds) {
        var d = descs[ds]
        return seen[ds] === true && !(d && d.ui && (d.ui.active === false || d.ui.visible === false))
    }

    // Ask for keys of one category (and, for a per-app one, that app's values). Nothing is asked twice while it is still queued.
    function read(category, keys, appId) {
        var q = queue.slice()
        for (var kind = 0; kind < 2; kind++) {
            var method = kind === 0 ? "getSystemSettings" : "getSystemSettingDesc"
            var again = false
            for (var i = 0; i < q.length; i++) if (q[i].category === category && q[i].method === method && q[i].keys.join() === keys.join() && q[i].appId === appId) again = true
            if (!again) q.push({ category: category, keys: keys, appId: appId, method: method })
        }
        queue = q
        pump()
    }
    function pump() {
        if (current || queue.length === 0) return
        current = queue[0]
        queue = queue.slice(1)
        var params = { category: current.category, keys: current.keys }
        if (current.appId) params.app_id = current.appId
        var target = current.method === "getSystemSettings" ? svc : descSvc
        target.callService(params)
        guard.restart()
    }
    function answer(method, payload) {
        if (method !== "getSystemSettings" && method !== "getSystemSettingDesc") return
        var doc = null
        try { doc = JSON.parse(payload) } catch (e) { doc = null }
        guard.stop()
        var cat = current ? current.category : (doc && doc.category) || ""
        if (method === "getSystemSettingDesc") {
            if (doc && doc.returnValue !== false && doc.settings) {
                var ds2 = {}
                for (var kd in descs) ds2[kd] = descs[kd]
                for (var key2 in doc.settings) ds2[cat + "." + key2] = doc.settings[key2]
                descs = ds2
            }
            current = null
            answers++
            pump()
            return
        }
        if (doc && doc.returnValue !== false && doc.settings) {
            var v = {}, s = {}
            for (var k in values) v[k] = values[k]
            for (var k2 in seen) s[k2] = seen[k2]
            for (var key in doc.settings) { v[cat + "." + key] = doc.settings[key]; s[cat + "." + key] = true }
            values = v
            seen = s
            if (doc.dimension) { var d = {}; for (var c in dimensions) d[c] = dimensions[c]; d[cat] = doc.dimension; dimensions = d }
        }
        current = null
        answers++
        pump()
    }

    // Change one setting. `ds` is "category.key"; the value goes out as the service wants it (strings).
    function write(ds, value, appId) {
        var at = ds.indexOf(".")
        var cat = ds.substring(0, at), key = ds.substring(at + 1)
        var v = {}
        for (var k in values) v[k] = values[k]
        v[ds] = value
        values = v
        var body = { category: cat, settings: {} }
        body.settings[key] = String(value)
        if (dimensions[cat]) body.dimension = dimensions[cat]
        if (appId) body.app_id = appId
        LS.adhoc.call(service, "/setSystemSettings", JSON.stringify(body))
    }

    property Service svc: Service {
        appId: "com.webos.surfacemanager"
        service: sio.service
        method: "getSystemSettings"
        onResponse: sio.answer(method, payload)
    }
    property Service descSvc: Service {
        appId: "com.webos.surfacemanager"
        service: sio.service
        method: "getSystemSettingDesc"
        onResponse: sio.answer(method, payload)
    }
    property Timer guard: Timer {
        interval: 2000
        onTriggered: { sio.current = null; sio.pump() }
    }
}
