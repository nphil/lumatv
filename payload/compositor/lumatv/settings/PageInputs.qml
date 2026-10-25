import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// Inputs & devices (docs/mocks/30-settings-inputs). Each input shows the name its device gave itself over
// HDMI-CEC; the owner decides whether it is on Home's Inputs row and in the picker: Auto (only while something
// is connected), Always, or Hidden. The choice is kept in theme.json (`inputs.<port>`), which the picker
// and Home read. The HDMI tab holds the TV's own HDMI switches.
PageList {
    id: page
    title: "Inputs & devices"
    tabs: ["Inputs", "HDMI", "Devices"]

    readonly property var ports: ["hdmi1", "hdmi2", "hdmi3", "hdmi4"]
    readonly property string choices: "auto"
    function entry(n) {
        var l = Inputs.list
        for (var i = 0; i < l.length; i++) if (l[i].appId === "com.webos.app." + ports[n]) return l[i]
        return null
    }
    function choice(key) { var t = Lu.theme.inputs || ({}); return t[key] || "auto" }
    function choiceIndex(key) { return ["auto", "always", "hidden"].indexOf(choice(key)) }
    function setChoice(key, i) { var p = { inputs: {} }; p.inputs[key] = ["auto", "always", "hidden"][i]; host.patch(p) }
    function portRow(n) {
        return { kind: "seg", tab: 0, icon: function() { var e = page.entry(n); return e ? e.icon : "unplug" },
                 label: function() { var e = page.entry(n); return "HDMI " + (n + 1) + " \u00b7 " + (e && e.connected ? e.label : "Nothing connected") },
                 detail: function() { var e = page.entry(n); return e && e.connected ? "Named by the device over HDMI-CEC." : "Plug in a device to use it." },
                 options: ["Auto", "Always", "Hidden"], index: function() { return page.choiceIndex(page.ports[n]) },
                 act: function(i) { page.setChoice(page.ports[n], i) } }
    }
    readonly property int nConnected: Inputs.connected.length
    subtitle: (Inputs.current ? Inputs.current.port + " now \u00b7 " : "") + nConnected + (nConnected === 1 ? " device connected" : " devices connected")
    summary: (Inputs.current ? Inputs.current.port : "No input") + " \u00b7 " + nConnected + " connected"

    Component.onCompleted: { Inputs.refresh(); Sio.read("general", ["simplinkEnable", "autoInput"]) }

    rows: [
        { kind: "seg", tab: 0, icon: "antenna", label: "Live TV", detail: "Antenna. Shown on Home's Inputs row.", options: ["Auto", "Always", "Hidden"],
          index: function() { return page.choiceIndex("livetv") }, act: function(i) { page.setChoice("livetv", i) } },
        portRow(0), portRow(1), portRow(2), portRow(3),

        R.toggle(Sio, { ds: "general.simplinkEnable", tab: 1, icon: "cable", on: "true", off: "false", label: "HDMI-CEC (SIMPLINK)",
                        detail: "Devices turn on, switch and name themselves over the cable." }),
        R.toggle(Sio, { ds: "general.autoInput", tab: 1, icon: "refresh-cw", on: "true", off: "false", label: "Auto input switch",
                        detail: "Switches to a device that starts playing." }),
        R.lg(page, { tab: 1, detail: "Deep Colour per port, 4:4:4, Dolby Vision PC, Quick Media Switching." }),

        R.lg(page, { tab: 2, id: "bt", label: "Bluetooth devices", detail: "Controllers, headphones and remotes." }),
        R.lg(page, { tab: 2, id: "game", label: "Game Optimizer", detail: "VRR, ALLM and latency per input, in LG's own screen." })
    ]
}
