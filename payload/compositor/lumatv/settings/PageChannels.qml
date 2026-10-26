import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "Names.js" as N
import "."

// Channels (LG: General › Channels): the settings of the tuner that are keys of the settings service. Channel Scrolling Options
// (`channel.autoscanChannelGroup`, primary or all channels), System Color (`channel.analogSystemColor`, analog TV only: LG shows it while
// Live TV is in front) and the two Interactive Service switches (`option.interactive-service` for Live TV and `option.interactive-service-hdmi`
// for HDMI; LG's own switch writes `onByUser` and `offByUser`, and reads `on` and `onByUser` as on). Every row is left out when the TV's
// description says LG does not show it (this set: no analog tuner). Auto Tuning, Manual Tuning, the Channel Manager and Channel Locks are
// scans and lists the tuner's service answers; they stay in LG's channel screen, the last row.
PageList {
    id: page
    title: "Channels"
    back: "Inputs & devices"

    readonly property bool liveTv: Inputs.current ? Inputs.current.appId === "com.webos.app.livetv" : false

    function refresh() {
        Sio.read("channel", ["autoscanChannelGroup", "analogSystemColor"])
        Sio.read("option", ["interactive-service", "interactive-service-hdmi"])
        Inputs.refresh()
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    function interactive(ds, label, help) {
        var row = R.toggle(Sio, { ds: ds, tab: 0, icon: "globe", label: label, help: help, on: "onByUser", off: "offByUser" })
        row.on = function() { var v = String(Sio.get(ds)); return v === "on" || v === "onByUser" }
        return C.lgShown(Sio, ds, row)
    }

    rows: [
        C.choice(Sio, { ds: "channel.autoscanChannelGroup", tab: 0, icon: "list-checks", label: "Channel Scrolling Options", names: { primary_channel: "Primary Channels", all_channel: "All Channels" },
                        page: page, help: "Which channels the channel keys step through." }),
        R.over(C.choice(Sio, { ds: "channel.analogSystemColor", tab: 0, icon: "palette", label: "System Color", names: { multi: "Multi", "pal-m": "PAL-M", "pal-n": "PAL-N", ntsc: "NTSC" },
                              page: page, help: "The colour system of an analog channel." }),
               { enabled: function() { return page.liveTv && Sio.usable("channel.analogSystemColor") }, detail: function() { return page.liveTv ? "" : "Live TV only" } }),
        interactive("option.interactive-service", "Interactive Service", "Content suggestions and ads that follow what Live TV is showing."),
        interactive("option.interactive-service-hdmi", "Interactive Service (HDMI)", "The same for a broadcast signal on an HDMI input."),
        R.lg(page, { id: "channeltuning", target: "channel", label: "Channel Tuning & Settings", help: "Auto and manual tuning, the channel list and locks, in LG's channel screen." })
    ]
}
