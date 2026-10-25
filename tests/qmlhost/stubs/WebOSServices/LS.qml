pragma Singleton
import QtQuick 2.4
import "FakeSettings.js" as Fake
import "FakeApps.js" as FakeApps

// Offline stand-in for the compositor's luna bridge. Every call is appended to `calls`, so a
// surface's luna use can be read back from the host; a setSystemSettings also lands in the fake
// settings service, so a page reads back what it wrote. One the service refuses (an unknown key, a value
// the key does not take) keeps `error` on its entry and changes nothing.
QtObject {
    id: ls
    property string appId: "org.nphil.lumatv.test"
    property var calls: []
    function record(uri, method, payload, error) {
        var c = calls.slice()
        c.push({ uri: uri, method: method, payload: payload, error: error || "" })
        calls = c
    }
    property QtObject applicationManager: QtObject {
        // the owner's set as the app manager lists it (FakeApps.js): Home's dock and the all-apps library read this
        property string launchPointsList: JSON.stringify({ launchPoints: FakeApps.launchPoints(Qt.resolvedUrl("../../../../docs/mocks/img/apps/"), Qt.resolvedUrl("../../../../assets/")) })
        property var appInfoList: ({})
    }
    property QtObject adhoc: QtObject {
        function call(uri, method, payload) {
            var error = ""
            if (method === "/setSystemSettings") {
                try { var p = JSON.parse(payload); error = Fake.write(p.category, p.settings).errorText || "" } catch (e) { error = "bad payload" }
            }
            if (method === "/timer/sleep/add") {
                try { error = Fake.sleepAdd(JSON.parse(payload)).errorText || "" } catch (e2) { error = "bad payload" }
            }
            ls.record(uri, method, payload, error)
        }
    }
}
