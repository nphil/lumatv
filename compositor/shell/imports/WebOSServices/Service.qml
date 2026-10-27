import QtQuick 2.4
import LumaShell 1.0

// LG's `Service` (WebOSServices, the luna client of the compositor's QML) for the shell client: the same properties,
// `call(uri, method, payload)`, `callService(params)` and `cancel(token)`, and the `response(method, payload, token)`
// signal, with the calls made by the compositor's bridge through the UI channel (Link.qml) rather than by this
// process, whose bus role reaches the app manager only. A payload with `subscribe: true` keeps the call open until
// `cancel`; `response` fires for every answer the bridge relays.
QtObject {
    id: svc
    property string appId: ""
    property string service: ""
    property string method: ""
    signal response(string method, string payload, int token)

    function call(uri, method, payload) {
        var p = {}
        try { p = JSON.parse(payload || "{}") } catch (e) { p = {} }
        var m = String(method || "")
        var tag = Link.luna(String(uri || "") + m, p, p.subscribe === true, function(doc) {
            svc.response(m, JSON.stringify(doc === undefined ? {} : doc), tag)
        })
        return tag
    }
    function callService(params) {
        var p = params || {}
        var m = svc.method
        var tag = Link.luna(svc.service + "/" + m, p, p.subscribe === true, function(doc) {
            svc.response(m, JSON.stringify(doc === undefined ? {} : doc), tag)
        })
        return tag
    }
    function cancel(token) { Link.cancel(token) }
}
