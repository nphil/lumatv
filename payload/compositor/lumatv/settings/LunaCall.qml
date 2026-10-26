import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0

// Calls to the TV's luna services other than the settings service, for the pages that cover LG's Settings: the clock, the time zone
// list, the network, Bluetooth, the software update. `call(tag, service, method, params)` asks once and `answered(tag, doc)` brings
// the reply (the parsed JSON, or null when it was not JSON); `subscribe(...)` keeps the call open and answers on every change until
// `close(tag)`. The calls are made as the compositor (`com.webos.surfacemanager`), the way every other service of the shell is called.
// `act(service, method, params)` is for what needs no answer. A page never touches the service types itself, so the offline host
// can answer for all of them from one table (tests/qmlhost/stubs/WebOSServices/FakeCoverage.js).
QtObject {
    id: luna
    signal answered(string tag, var doc)
    property var open: ({})
    property Component maker: Component { Service { appId: "com.webos.surfacemanager" } }

    function make(tag, service, method, keep) {
        var s = maker.createObject(luna, { service: service, method: method })
        s.response.connect(function(m, payload) {
            var doc = null
            try { doc = JSON.parse(payload) } catch (e) { doc = null }
            luna.answered(tag, doc)
            if (!keep) s.destroy(50)
        })
        return s
    }
    function call(tag, service, method, params) {
        make(tag, service, method, false).callService(params || {})
    }
    function subscribe(tag, service, method, params) {
        close(tag)
        var s = make(tag, service, method, true)
        var o = {}
        for (var k in open) o[k] = open[k]
        o[tag] = s
        open = o
        var p = {}
        for (var q in (params || {})) p[q] = params[q]
        p.subscribe = true
        s.callService(p)
    }
    function close(tag) {
        if (!open[tag]) return
        open[tag].destroy(50)
        var o = {}
        for (var k in open) if (k !== tag) o[k] = open[k]
        open = o
    }
    function act(service, method, params) { LS.adhoc.call(service, "/" + method, JSON.stringify(params || {})) }
}
