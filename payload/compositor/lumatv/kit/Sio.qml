pragma Singleton
import QtQuick 2.4
import WebOSServices 1.0

// The TV's settings service as the Settings pages see it. A page names what it shows as
// "category.key" (the same names LG's Settings app and the stock services use, for example
// picture.backlight or general.remotePointer); this reads them with getSystemSettings and keeps the
// answers in `values`, so a row binds to `Sio.values["picture.backlight"]`, and writes one back with
// setSystemSettings. A write shows at once (the page is never waiting for the service to say what
// the owner just chose) and a later answer replaces it. A key without a category (localeInfo) is
// read with no category and named by the key alone.
//
// The set answers a read with the keys it has in `settings` and the rest in `errorKey`, with
// `returnValue: false` as soon as one key is missing (OLED65C4, webOS 24: asked for eleven picture
// keys it answered nine and refused filmmakerAmbientLight and truMotionMode); LG's own app takes the
// settings whatever the flag says, and so does this. What the service does not have stays absent:
// `has(ds)` is false for it, `missing(ds)` true once it said so, and the pages leave such a row out
// rather than show a made-up value. Picture settings belong to a "dimension" (picture mode, dynamic
// range, input): the service names the dimension it answered for, and a write carries the one the
// last answer had.
//
// Each read asks twice, for the values and for the service's description of them (`getSystemSettingDesc`,
// answered as `results`, one entry per key with `ui` and `values`, the `ui` LG's own Settings reads to dim a
// row): a setting whose description says it cannot be changed now stays on screen and is dimmed, `usable(ds)`
// is false for it, and a row that writes it does not take focus. A description that never comes leaves the
// row usable. A few settings are per app (the aspect ratio, TruMotion): `read` and `write` take that app id,
// and the service answers such a key only when it is asked with it.
//
// Reads go one at a time (the service answers by method name, not by call) and each gets 2 s.
QtObject {
    id: sio

    readonly property string service: "luna://com.webos.service.settings"
    property var values: ({})                  // "category.key" -> the value the service gave
    property var seen: ({})                    // "category.key" -> true once answered
    property var absent: ({})                  // "category.key" -> true once the service said it has no such key
    property var descs: ({})                   // "category.key" -> {ui: {visible, active}, values} from the service's description
    property var dimensions: ({})              // category -> {pictureMode, dynamicRange, input} of the last answer
    property var queue: []                     // [{category, keys}] waiting
    property var current: null                 // the read in flight
    property int answers: 0                    // counts answers, for a page that wants to know it has heard

    function get(ds) { return values[ds] }
    function has(ds) { return seen[ds] === true }
    function missing(ds) { return absent[ds] === true }
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
    function dsName(category, key) { return category ? category + "." + key : key }

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
        var params = { keys: current.keys }
        if (current.category) params.category = current.category
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
            var list = doc && doc.results ? doc.results : doc && doc.key ? [doc] : []
            if (list.length) {
                var ds2 = {}
                for (var kd in descs) ds2[kd] = descs[kd]
                for (var r = 0; r < list.length; r++) {
                    var e = list[r]
                    if (e && e.key) ds2[dsName(e.category || cat, e.key)] = { ui: e.ui || {}, values: e.values || {} }
                }
                descs = ds2
            }
            current = null
            answers++
            pump()
            return
        }
        if (doc && doc.settings) {
            var v = {}, s = {}
            for (var k in values) v[k] = values[k]
            for (var k2 in seen) s[k2] = seen[k2]
            for (var key in doc.settings) { v[dsName(cat, key)] = doc.settings[key]; s[dsName(cat, key)] = true }
            values = v
            seen = s
            if (doc.dimension) { var d = {}; for (var c in dimensions) d[c] = dimensions[c]; d[cat] = doc.dimension; dimensions = d }
        }
        if (doc && doc.errorKey && doc.errorKey.length) {
            var a = {}
            for (var ka in absent) a[ka] = absent[ka]
            for (var m = 0; m < doc.errorKey.length; m++) a[dsName(cat, doc.errorKey[m])] = true
            absent = a
        }
        current = null
        answers++
        pump()
    }

    // Change one setting. `ds` is "category.key" (or a bare key); the value goes out as the service wants it (strings).
    function write(ds, value, appId) {
        var at = ds.indexOf(".")
        var cat = at < 0 ? "" : ds.substring(0, at), key = at < 0 ? ds : ds.substring(at + 1)
        var v = {}
        for (var k in values) v[k] = values[k]
        v[ds] = value
        values = v
        var body = { settings: {} }
        if (cat) body.category = cat
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
