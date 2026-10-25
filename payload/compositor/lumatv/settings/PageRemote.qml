import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// Remote & pointer (docs/mocks/03-settings-remote). The Magic Remote's pointer is the TV's own switch
// (`general.remotePointer`, PLAN "Not owned"): off gives a button-only remote and removes accidental hover
// moves. Speed and size are the service's own settings (`option.pointerSpeed`, `option.pointerSize`). Holding Home
// for LG's Home stays on: it is the escape if LumaTV's Home ever misbehaves, so it is shown, not switchable.
// Whether the number keys open the dock's tiles is LumaTV's own (`remote.numberKeys` in theme.json, default on).
PageList {
    id: page
    title: "Remote & pointer"
    readonly property var theme: Lu.theme || ({})
    readonly property bool pointerOn: String(Sio.get("general.remotePointer")) === "on"
    readonly property string speed: R.name({ slow: "Slow speed", normal: "Normal speed", fast: "Fast speed" }, Sio.get("option.pointerSpeed"), "")
    subtitle: "Magic Remote \u00b7 pointer " + (Sio.has("general.remotePointer") ? (pointerOn ? "on" : "off") : "unknown")
    summary: "Pointer " + (Sio.has("general.remotePointer") ? (pointerOn ? "on" : "off") : "unknown") + (pointerOn && speed ? " \u00b7 " + speed : "")

    function refresh() {
        Sio.read("general", ["remotePointer"])
        Sio.read("option", ["pointerSpeed", "pointerSize"])
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    rows: [
        R.head("Pointer"),
        R.toggle(Sio, { ds: "general.remotePointer", icon: "mouse-pointer-2", label: "Magic Remote pointer",
                        detail: "Use the pointer on screen. Turn off if you only use a button remote." }),
        R.seg(Sio, { ds: "option.pointerSpeed", icon: "gauge", label: "Pointer speed", detail: "How far the pointer moves for a flick of the wrist.",
                     options: ["Slow", "Normal", "Fast"], values: ["slow", "normal", "fast"] }),
        R.seg(Sio, { ds: "option.pointerSize", icon: "scan-eye", label: "Pointer size", detail: "Bigger is easier to find from the sofa.",
                     options: ["Small", "Medium", "Large"], values: ["small", "medium", "large"] }),
        R.head("Buttons"),
        { kind: "value", icon: "house", label: "Hold Home for LG's Home", info: true, chevron: false, value: "Always on",
          detail: "Press and hold Home for 650 ms to open LG's own Home if LumaTV misbehaves." },
        { kind: "toggle", icon: "hash", label: "Number keys open dock tiles", detail: "1 opens the first tile on the dock, 2 the second, and so on.",
          on: function() { return !page.theme.remote || page.theme.remote.numberKeys !== false },
          act: function(v) { if (page.host) page.host.patch({ remote: { numberKeys: v } }) } },
        R.lg(page, { target: "RemoteSlowInput", detail: "Universal remote control, extended press times, Settings help." })
    ]
}
