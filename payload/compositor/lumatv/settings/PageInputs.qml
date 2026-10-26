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
    readonly property string choiceHelp: "Auto shows it on Home and in the picker while something is connected. Always keeps it there. Hidden removes it."
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
                 options: ["Auto", "Always", "Hidden"], index: function() { return page.choiceIndex(page.ports[n]) },
                 act: function(i) { page.setChoice(page.ports[n], i) } }
    }
    summary: Inputs.current ? Inputs.current.port : ""

    function refresh() { Inputs.refresh(); Sio.read("other", ["simplinkEnable", "qmsVrr", "uhdDeepColor", "444BypassHDMI1", "444BypassHDMI2", "444BypassHDMI3", "444BypassHDMI4", "enableDolbyVisionPC"]) }
    Component.onCompleted: refresh()
    onOpened: refresh()

    rows: [
        { kind: "seg", tab: 0, icon: "antenna", label: "Live TV", help: page.choiceHelp, options: ["Auto", "Always", "Hidden"],
          index: function() { return page.choiceIndex("livetv") }, act: function(i) { page.setChoice("livetv", i) } },
        portRow(0), portRow(1), portRow(2), portRow(3),
        { kind: "toggle", tab: 0, icon: "tv-minimal-play", label: "Input banner",
          help: "LumaTV draws its own banner over an input or Live TV, besides LG's. For when LG's is hidden.",
          on: function() { return !!(Lu.theme.banners && Lu.theme.banners.own === true) },
          act: function(v) { if (page.host) page.host.patch({ banners: { own: v } }) } },

        R.toggle(Sio, { ds: "other.simplinkEnable", id: "cec2", tab: 1, icon: "cable", label: "HDMI-CEC (SIMPLINK)",
                        help: "Devices turn on, switch and name themselves over the cable." }),
        R.toggle(Sio, { ds: "other.qmsVrr", tab: 1, icon: "fast-forward", label: "Quick Media Switching",
                        help: "Switches between a game and a video without a black screen. Only for the input in front." }),
        R.seg(Sio, { ds: "other.uhdDeepColor", tab: 1, icon: "palette", label: "HDMI Deep Color", options: ["Off", "4K"], values: ["off", "4k"],
                     help: "Richer picture from a source that sends it, such as a PC or console. Only for the input in front." }),
        R.toggle(Sio, { ds: "other.444BypassHDMI1", tab: 1, icon: "monitor", label: "4:4:4 Pass Through (HDMI 1)",
                        help: "Shows a PC or console picture with no processing, so text stays sharp. Only for the input in front." }),
        R.toggle(Sio, { ds: "other.444BypassHDMI2", tab: 1, icon: "monitor", label: "4:4:4 Pass Through (HDMI 2)" }),
        R.toggle(Sio, { ds: "other.444BypassHDMI3", tab: 1, icon: "monitor", label: "4:4:4 Pass Through (HDMI 3)" }),
        R.toggle(Sio, { ds: "other.444BypassHDMI4", tab: 1, icon: "monitor", label: "4:4:4 Pass Through (HDMI 4)" }),
        R.toggle(Sio, { ds: "other.enableDolbyVisionPC", tab: 1, icon: "monitor-cog", label: "Dolby Vision PC",
                        help: "Lets a PC send Dolby Vision. Turn HDR on in the PC's display settings." }),

        R.link({ tab: 2, id: "channels", icon: "antenna", label: "Channels", help: "Channel scrolling, system colour and the interactive service.", go: function() { page.host.openPage("channels") } }),
        R.link({ tab: 2, id: "bt", icon: "gamepad-2", label: "Bluetooth Devices", go: function() { page.host.openPage("bthid") } }),
        R.lg(page, { tab: 2, id: "game", app: "com.webos.app.gameoptimizer", label: "Game Optimizer", help: "VRR, ALLM and latency per input, in LG's own screen." })
    ]
}
