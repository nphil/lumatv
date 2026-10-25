import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// System (docs/mocks/30-settings-system): the handful of TV-wide settings a person reaches for, and where the
// rest of LG's Settings is. Software update, channels, privacy, the LG account and reset stay in LG's app (they
// are one-time screens with their own confirmations), linked from the last row of each tab.
PageList {
    id: page
    title: "System"
    tabs: ["General", "Power", "OLED care", "About"]
    subtitle: "webOS"
    summary: "webOS \u00b7 " + (Sio.has("general.deviceName") ? Sio.get("general.deviceName") : "LumaTV")

    Component.onCompleted: Sio.read("general", ["deviceName", "language", "timeZone", "timeZoneAuto", "quickStartMode", "autoOff4Hours", "autoOff2Hours"])
    readonly property string autoOff: String(Sio.get("general.autoOff2Hours")) === "on" ? "2h" : String(Sio.get("general.autoOff4Hours")) === "on" ? "4h" : "off"

    rows: [
        { kind: "value", tab: 0, icon: "languages", label: "Language", detail: "Menus, keyboard and audio language.", chevron: false, info: true,
          value: function() { return R.name({ "en-US": "English (US)", "en-GB": "English (UK)" }, Sio.get("general.language"), "") } },
        { kind: "value", tab: 0, icon: "clock", label: "Time zone", detail: function() { return String(Sio.get("general.timeZoneAuto")) === "on" ? "Set from the network. " + (Sio.get("general.timeZone") || "") : (Sio.get("general.timeZone") || "") },
          value: function() { return String(Sio.get("general.timeZoneAuto")) === "on" ? "Automatic" : "Manual" }, chevron: false, info: true },
        R.lg(page, { tab: 0, label: "Open LG settings", detail: "Software update, channels, privacy, account, reset." }),

        { kind: "seg", tab: 1, icon: "power", label: "Turn off when idle", detail: "No key for this long turns the TV off.",
          options: ["2 h", "4 h", "Off"], enabled: function() { return Sio.has("general.autoOff4Hours") },
          index: function() { return page.autoOff === "2h" ? 0 : page.autoOff === "4h" ? 1 : 2 },
          act: function(i) { Sio.write("general.autoOff2Hours", i === 0 ? "on" : "off"); Sio.write("general.autoOff4Hours", i === 1 ? "on" : "off") } },
        R.toggle(Sio, { ds: "general.quickStartMode", tab: 1, icon: "zap", label: "Quick Start+", detail: "Wakes in about 2 s from standby. Uses a little power while off." }),
        R.note("In standby the panel goes fully dark; the TV still wakes at once.", 1),

        R.toggle(Sio, { ds: "picture.screenShift", tab: 2, icon: "scan-eye", label: "Screen shift", detail: "Moves the picture a pixel now and then so the panel wears evenly." }),
        R.lg(page, { tab: 2, label: "Pixel refresher and panel care", detail: "Runs while the TV is off and takes about an hour; LG's screen asks first." }),

        { kind: "value", tab: 3, icon: "tv", label: function() { return Sio.has("general.deviceName") ? Sio.get("general.deviceName") : "This TV" }, detail: "The name on the network and in Home Assistant.",
          value: "", chevron: false, info: true },
        { kind: "value", tab: 3, icon: "sparkles", label: "LumaTV", detail: "The Home, panels and Settings you are looking at.", value: function() { return Lu.theme.name || "LumaTV" }, chevron: false, info: true }
    ]
}
