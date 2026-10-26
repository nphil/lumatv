pragma Singleton
import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0

// The TV's own volume and mute, from the compositor's audio service: the one LG's volume popup reads (`LS.audioService`, whose
// `com.webos.service.audio` subscription LG's own views already hold, so this costs no luna call of its own). `cue()` is LG's
// `volumeUpdated`: the service reports a volume key, a mute, a change made by an app or by Home Assistant, at the moment LG's popup would
// have opened, whatever is on screen.
//
// What the service says on this TV, read on the set (2026-10-25): with the receiver over eARC `soundOutput` is "external_arc",
// `volumeSyncable` false and `volume` a step counter of 0 to 100 that moves by one a key (31 to 32 on Volume+, back on Volume-), the
// report arriving 44 to 63 ms after the key with `cause` "volumeUp" or "volumeDown". The counter says that a key was pressed and nothing
// about the receiver's level, which is why the receiver's own reading is shown when there is one (panels/VolumeOsd.qml); with the TV's
// speakers, headphones or Bluetooth as the output it is the real volume.
//
// Two imports because the module that holds `LS` differs: on the set it is WebOSCompositorBase (WebOSServices has no LS, so a file that
// imports only that fails with "LS is not defined"), in the offline host it is a stand-in in WebOSServices.
QtObject {
    id: tv

    readonly property var svc: LS.audioService
    readonly property string output: svc && svc.soundOutput ? String(svc.soundOutput) : ""
    readonly property bool known: output !== ""                    // the service has said where the sound goes
    readonly property int volume: svc && svc.volume > 0 ? svc.volume : 0
    readonly property int volumeMax: svc && svc.volumeMax > 0 ? svc.volumeMax : 100
    readonly property bool muted: !!svc && !!svc.muted
    readonly property string cause: svc && svc.cause ? String(svc.cause) : ""
    // The sound goes over HDMI ARC or optical to a device with a volume of its own (a receiver): the keys change that device, and the
    // counter above does not say where it stands.
    readonly property bool arc: output.indexOf("external_arc") >= 0 || output === "external_optical"
    readonly property string outputLabel: {
        if (arc) return "ARC"
        if (output === "external_optical") return "Optical"
        if (output === "headphone") return "Headphones"
        if (output === "bt_soundbar" || output === "bt_atmos" || output === "tv_speaker_bluetooth" || output === "tv_speaker_bt_surround") return "Bluetooth"
        if (output === "wisa_speaker") return "WiSA"
        if (output === "lineout") return "Line out"
        if (output === "usb_speaker") return "USB"
        return "TV"
    }

    signal cue()
    property QtObject link: Connections {
        target: tv.svc
        ignoreUnknownSignals: true
        onVolumeUpdated: tv.cue()
    }
}
