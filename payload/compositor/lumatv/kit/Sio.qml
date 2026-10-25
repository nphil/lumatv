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
// Reads go one at a time (the service answers by method name, not by call) and each gets 2 s.
QtObject {
    id: sio

    readonly property string service: "luna://com.webos.service.settings"
    property var values: ({})                  // "category.key" -> the value the service gave
    property var seen: ({})                    // "category.key" -> true once answered
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

    // Ask for keys of one category. Nothing is asked twice while it is still queued.
    function read(category, keys) {
        for (var i = 0; i < queue.length; i++) if (queue[i].category === category && queue[i].keys.join() === keys.join()) return
        var q = queue.slice()
        q.push({ category: category, keys: keys })
        queue = q
        pump()
    }
    function pump() {
        if (current || queue.length === 0) return
        current = queue[0]
        queue = queue.slice(1)
        svc.callService({ category: current.category, keys: current.keys })
        guard.restart()
    }
    function answer(method, payload) {
        if (method !== "getSystemSettings") return
        var doc = null
        try { doc = JSON.parse(payload) } catch (e) { doc = null }
        guard.stop()
        var cat = current ? current.category : (doc && doc.category) || ""
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
    function write(ds, value) {
        var at = ds.indexOf(".")
        var cat = ds.substring(0, at), key = ds.substring(at + 1)
        var v = {}
        for (var k in values) v[k] = values[k]
        v[ds] = value
        values = v
        var body = { category: cat, settings: {} }
        body.settings[key] = String(value)
        if (dimensions[cat]) body.dimension = dimensions[cat]
        LS.adhoc.call(service, "/setSystemSettings", JSON.stringify(body))
    }

    property Service svc: Service {
        appId: "com.webos.surfacemanager"
        service: sio.service
        method: "getSystemSettings"
        onResponse: sio.answer(method, payload)
    }
    property Timer guard: Timer {
        interval: 2000
        onTriggered: { sio.current = null; sio.pump() }
    }
}
