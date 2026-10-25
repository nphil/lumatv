pragma Singleton
import QtQuick 2.4
import "FakeSettings.js" as Fake

// Offline stand-in for the compositor's luna bridge. Every call is appended to `calls`, so a
// surface's luna use can be read back from the host; a setSystemSettings also lands in the fake
// settings service, so a page reads back what it wrote.
QtObject {
    id: ls
    property string appId: "org.nphil.lumatv.test"
    property var calls: []
    function record(uri, method, payload) {
        var c = calls.slice()
        c.push({ uri: uri, method: method, payload: payload })
        calls = c
    }
    property QtObject applicationManager: QtObject {
        property string launchPointsList: "{\"launchPoints\":[]}"
        property var appInfoList: ({})
    }
    property QtObject adhoc: QtObject {
        function call(uri, method, payload) {
            ls.record(uri, method, payload)
            if (method === "/setSystemSettings") {
                try { var p = JSON.parse(payload); Fake.write(p.category, p.settings) } catch (e) { }
            }
        }
    }
}
