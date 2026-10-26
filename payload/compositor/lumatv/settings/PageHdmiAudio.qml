import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Select HDMI Input Audio Format (LG: Sound › Advanced Settings): for each input, the format the TV asks the device to send: Bitstream (the
// original soundtrack, for a receiver) or PCM. LG's keys are `sound.inputAudioFormatHDMI1` to `4`, and DP and USB-C on models that have those
// inputs (the TV hides them here); the TV's description says which exist and which may be changed now. A change makes the device
// renegotiate, so the picture or sound can drop for a moment.
PageList {
    id: page
    title: "HDMI input audio"
    back: "Sound"

    readonly property var ports: ["HDMI1", "HDMI2", "HDMI3", "HDMI4", "DP1", "DP2", "USBC1", "USBC2"]
    readonly property var labels: ["HDMI 1", "HDMI 2", "HDMI 3", "HDMI 4", "DisplayPort 1", "DisplayPort 2", "USB-C 1", "USB-C 2"]

    function refresh() { Sio.read("sound", ports.map(function(p) { return "inputAudioFormat" + p })) }
    Component.onCompleted: refresh()
    onOpened: refresh()

    function portRow(i) {
        return C.seg(Sio, { ds: "sound.inputAudioFormat" + ports[i], icon: i < 4 ? "cable" : i < 6 ? "monitor" : "usb", label: labels[i], names: "hdmiAudio",
                            help: i === 0 ? "Bitstream sends the original soundtrack to a receiver. Changing it may blank the picture or sound for a moment." : undefined })
    }
    rows: [portRow(0), portRow(1), portRow(2), portRow(3), portRow(4), portRow(5), portRow(6), portRow(7)]
}
