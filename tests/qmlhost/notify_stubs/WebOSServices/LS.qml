pragma Singleton
import QtQuick 2.4

// Offline stand-in for the compositor's luna bridge. Every call is recorded in `calls`; `deliver`
// answers as a service would: the live Service on that URI gets `method` set and a `response`.
QtObject {
    id: ls
    property var calls: []
    property var live: []
    function record(uri, method, payload) {
        var c = calls.slice()
        c.push({ uri: uri, method: method, payload: payload })
        calls = c
    }
    function register(s) { live.push(s) }
    function unregister(s) {
        var i = live.indexOf(s)
        if (i >= 0) live.splice(i, 1)
    }
    function deliver(uri, method, payload) {
        var n = 0
        for (var i = 0; i < live.length; i++) {
            if (live[i].service === uri) {
                live[i].method = method
                live[i].response(payload)
                n++
            }
        }
        return n
    }
}
