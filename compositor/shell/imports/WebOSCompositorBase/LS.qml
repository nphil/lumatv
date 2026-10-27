pragma Singleton
import QtQuick 2.4
import LumaShell 1.0

// `LS` as LumaTV's QML knows it from LG's compositor (WebOSCompositorBase/global/LS.qml), in the shell client: the ad-hoc
// luna client, the app manager's launch points and the audio service's state, each fed over the UI channel (Link.qml)
// instead of the compositor's own luna plugins. `calls` stays undefined, as on the set: the offline host's stand-in
// records there.
QtObject {
    id: ls
    readonly property string appId: "org.nphil.lumatv.shell"
    property var calls: undefined

    readonly property QtObject adhoc: QtObject {
        // One-shot, the answer dropped: nothing reads an ad-hoc call's reply.
        function call(uri, method, payload) {
            var p = {}
            try { p = JSON.parse(payload || "{}") } catch (e) { p = {} }
            return Link.luna(uri + (method || ""), p, false, null)
        }
    }

    // LG's ApplicationManagerService: `launchPointsList` is the app manager's whole answer as a JSON string, changed with
    // every push (Dock.qml reads `launchPoints` off it). A push that carries a change rather than the list (`change`:
    // added, updated, removed) has the list read again, so the string always holds the whole list.
    readonly property QtObject applicationManager: QtObject {
        id: am
        property string launchPointsList: "{}"
        property var appInfoList: ({})
        function take(doc) { if (doc && doc.launchPoints) launchPointsList = JSON.stringify(doc) }
        Component.onCompleted: {
            Link.luna("luna://com.webos.service.applicationmanager/listLaunchPoints", { subscribe: true }, true, function(doc) {
                if (doc && doc.launchPoints) am.take(doc)
                else if (doc && doc.change) Link.luna("luna://com.webos.service.applicationmanager/listLaunchPoints", {}, false, am.take)
            })
        }
    }

    // LG's AudioService plugin with its property names, fed by the compositor's own getVolume subscription (the `audio` topic:
    // {"volumeStatus": {...}}): `volumeUpdated` is what the volume display listens for.
    readonly property QtObject audioService: QtObject {
        id: audio
        property int volume: 0
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
        function apply(doc) {
            var v = doc && doc.volumeStatus
            if (!v) return
            if (typeof v.volume === "number") volume = v.volume
            if (typeof v.maxVolume === "number" && v.maxVolume > 0) volumeMax = v.maxVolume
            if (typeof v.muteStatus === "boolean") muted = v.muteStatus
            if (typeof v.volumeSyncable === "boolean") volumeSyncable = v.volumeSyncable
            if (typeof v.ossActivate === "boolean") ossActivate = v.ossActivate
            if (typeof v.soundOutput === "string") soundOutput = v.soundOutput
            if (typeof v.mode === "string") mode = v.mode
            cause = typeof v.cause === "string" ? v.cause : ""
            disabled = v.activeStatus === false
            reports++
            volumeUpdated()
        }
    }
    property Connections link: Connections {
        target: Link
        onUpdated: if (topic === "audio") audio.apply(Link.topics.audio)
    }
    Component.onCompleted: { Link.start(); if (Link.topics.audio) audio.apply(Link.topics.audio) }
}
