pragma Singleton
import QtQuick 2.4
import "FakeSettings.js" as Fake
import "FakeApps.js" as FakeApps
import "FakeCoverage.js" as Cov

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
    // The compositor's audio service (AudioService plugin) with LG's property names. `soundOutput` is empty until a test or a scene says where
    // the sound goes, which is how the real service looks before its first answer: the volume display then knows nothing of the TV's own
    // volume. `report()` is what a volume key does on the set: the service changes its state and announces it with `volumeUpdated`.
    property QtObject audioService: QtObject {
        id: audio
        property int volume: 12
        property int volumeMax: 100
        property bool muted: false
        property bool disabled: false
        property bool volumeSyncable: true
        property bool ossActivate: false
        property string soundOutput: ""
        property string mode: "normal"
        property string cause: ""
        property int reports: 0
        signal volumeUpdated()
        function report(why, level, mute) {
            if (level !== undefined && level !== null) volume = level
            if (mute !== undefined && mute !== null) muted = mute
            cause = why || ""
            reports++
            volumeUpdated()
        }
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
            // the coverage pages' services (FakeCoverage.js): a change of the clock, the zone or a pairing is kept for the next read
            try { error = error || Cov.act(uri + method, JSON.parse(payload)) || "" } catch (e3) { error = error || "bad payload" }
            ls.record(uri, method, payload, error)
        }
    }
}
