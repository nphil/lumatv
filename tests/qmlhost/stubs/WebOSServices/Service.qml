import QtQuick 2.4
import WebOSCompositorBase 1.0
import "FakeSettings.js" as Fake

// Offline stand-in for LG's `Service` type: `call(uri, method, payload)` records and returns a token (> 0, as the real one does; `cancel(token)`
// records too), and `callService(params)` answers getSystemSettings from the fake settings service (FakeSettings.js) one
// event-loop turn later, as the real one answers by signal, so a page sees the same asynchrony. A test makes a service answer by calling
// the signal itself: `svc.response(method, payload, token)` (a subscription's later answers are the same).
QtObject {
    id: svc
    property string appId: ""
    property string service: ""
    property string method: ""
    property int issued: 0
    signal response(string method, string payload, int token)
    function call(uri, method, payload) { LS.record(uri, method, payload); issued++; return issued }
    function cancel(token) { LS.record("", "cancel", JSON.stringify({ token: token })) }
    function callService(params) {
        var m = svc.method
        var p = params || {}
        var payload = JSON.stringify(Fake.call(svc.service, m, p))
        LS.record(svc.service, "/" + m, JSON.stringify(p))
        replyTimer.pending = { m: m, payload: payload }
        replyTimer.restart()
    }
    property Timer replyTimer: Timer {
        property var pending: null
        interval: 1
        onTriggered: { var x = pending; pending = null; if (x) svc.response(x.m, x.payload, 0) }
    }
}
