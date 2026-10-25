import QtQuick 2.4
import WebOSServices 1.0

// Luna call stand-in: records what it is asked, answers only through LS.deliver.
QtObject {
    id: svc
    property string appId: ""
    property string service: ""
    property string method: ""
    signal response(string payload)
    signal error(string errorText)
    Component.onCompleted: LS.register(svc)
    Component.onDestruction: LS.unregister(svc)
    function call(uri, meth, payload) { LS.record(uri, meth, payload) }
    function callForApplication(source, uri, meth, payload) { LS.record(uri, meth, payload) }
    function callService(payload) { LS.record(service, method, JSON.stringify(payload)) }
    function cancel() {}
}
