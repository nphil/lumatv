import QtQuick 2.4

QtObject {
    property string appId: ""
    signal response(string payload)
    function call(uri, method, payload) { LS.record(uri, method, payload) }
}
