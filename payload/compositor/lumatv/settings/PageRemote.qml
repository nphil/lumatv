import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// Remote & pointer (docs/mocks/03-settings-remote). The Magic Remote's pointer is the TV's own switch
// (`general.remotePointer`, PLAN "Not owned"): off gives a button-only remote and removes accidental hover
// moves. Speed and size are the service's own settings. Holding Home for LG's Home stays on: it is the escape
// if LumaTV's Home ever misbehaves, so it is not a switch.
PageList {
    id: page
    title: "Remote & pointer"
    readonly property bool pointerOn: String(Sio.get("general.remotePointer")) === "on"
    subtitle: "Magic Remote \u00b7 pointer " + (Sio.has("general.remotePointer") ? (pointerOn ? "on" : "off") : "unknown")
    summary: "Pointer " + (Sio.has("general.remotePointer") ? (pointerOn ? "on" : "off") : "unknown")

    Component.onCompleted: Sio.read("general", ["remotePointer", "pointerSpeed", "pointerSize"])

    rows: [
        R.head("Pointer"),
        R.toggle(Sio, { ds: "general.remotePointer", icon: "mouse-pointer-2", label: "Magic Remote pointer",
                        detail: "Use the pointer on screen. Turn off if you only use a button remote." }),
        R.seg(Sio, { ds: "general.pointerSpeed", icon: "gauge", label: "Pointer speed", detail: "How far the pointer moves for a flick of the wrist.",
                     options: ["Slow", "Normal", "Fast"], values: ["slow", "normal", "fast"] }),
        R.seg(Sio, { ds: "general.pointerSize", icon: "scan-eye", label: "Pointer size", detail: "Bigger is easier to find from the sofa.",
                     options: ["Small", "Medium", "Large"], values: ["small", "medium", "large"] }),
        R.head("Buttons"),
        R.note("Hold Home briefly (650 ms) to open LG's own Home if LumaTV misbehaves. It is always on."),
        R.lg(page, { detail: "Universal remote control, the remote's microphone, Settings help." })
    ]
}
