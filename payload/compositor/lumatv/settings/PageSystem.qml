import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// System (docs/mocks/30-settings-system): the handful of TV-wide settings a person reaches for, and where the
// rest of LG's Settings is. Keys and categories are the ones LG's own Settings app uses. Software update,
// channels, privacy, the LG account and reset stay in LG's app (they are one-time screens with their own
// confirmations); each row that leaves lands on that screen (R.lg `target`), not at LG's Picture page.
PageList {
    id: page
    title: "System"
    tabs: ["General", "Power", "OLED care", "Accessibility", "About"]
    subtitle: "webOS"
    summary: "webOS \u00b7 " + (Sio.has("network.deviceName") ? Sio.get("network.deviceName") : "LumaTV")

    readonly property var locales: Sio.get("localeInfo.locales") || ({})
    readonly property var zone: Sio.get("time.timeZone") || ({})
    readonly property string zoneName: typeof zone === "string" ? zone : (zone.City || zone.ZoneID || "")

    function refresh() {
        Sio.read("localeInfo", ["locales"])
        Sio.read("time", ["timeZone", "noOperationAutoOff", "autoOff15Min"])
        Sio.read("option", ["quickStartMode", "audioGuidance"])
        Sio.read("picture", ["screenShift", "logoLuminanceAdjust"])
        Sio.read("caption", ["captionEnable"])
        Sio.read("network", ["deviceName"])
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    function launch(target) { if (host) { host.launchApp("com.palm.app.settings", { target: target }); host.closeRequested() } }

    rows: [
        { kind: "value", tab: 0, icon: "languages", label: "Language", detail: "Menus, keyboard and audio language. LG's Language screen changes it.", chevron: true,
          enabled: function() { return Sio.has("localeInfo.locales") },
          value: function() { return R.name({ "en-US": "English (US)", "en-GB": "English (UK)", "es-US": "Spanish (US)", "fr-FR": "French" }, page.locales.UI, "") },
          act: function() { page.launch("Language") } },
        { kind: "value", tab: 0, icon: "clock", label: "Time zone", detail: "Set from the network by default. LG's Date and time screen changes it.", chevron: true,
          enabled: function() { return Sio.has("time.timeZone") },
          value: function() { return page.zoneName }, act: function() { page.launch("DateTime") } },
        R.seg(Sio, { ds: "time.noOperationAutoOff", tab: 0, icon: "power", label: "Turn off when idle", detail: "No key for this long turns the TV off.",
                     options: ["Off", "3 h", "6 h", "12 h", "24 h"], values: ["off", "3", "6", "12", "24"] }),
        R.toggle(Sio, { ds: "option.quickStartMode", tab: 0, icon: "zap", label: "Quick Start+", detail: "Wakes in about 2 s from standby. Uses a little power while off." }),
        R.lg(page, { tab: 0, label: "Open LG settings", detail: "LG's full Settings app, themed. Channels, privacy, account, reset." }),

        R.toggle(Sio, { ds: "time.autoOff15Min", tab: 1, icon: "power", label: "Turn off with no signal", detail: "Switches off after 15 minutes without a picture." }),
        R.note("In standby the panel goes fully dark; the TV still wakes at once.", 1),
        R.lg(page, { tab: 1, target: "Timer", label: "Timers", detail: "Sleep timer and the power-on and power-off timers." }),

        R.toggle(Sio, { ds: "picture.screenShift", tab: 2, icon: "scan-eye", label: "Screen shift", detail: "Moves the picture a pixel now and then so the panel wears evenly." }),
        R.seg(Sio, { ds: "picture.logoLuminanceAdjust", tab: 2, icon: "contrast", label: "Dim logos and subtitles", detail: "Lowers the brightness of static logos and subtitles so they do not burn in.",
                     options: ["Off", "Light", "Strong"], values: ["off", "light", "strong"] }),
        R.lg(page, { tab: 2, target: "OledCare", label: "Pixel refresher and panel care", detail: "Runs while the TV is off and takes about an hour; LG's screen asks first." }),

        R.toggle(Sio, { ds: "option.audioGuidance", tab: 3, icon: "audio-lines", label: "Audio guidance", detail: "Reads the menus aloud." }),
        R.toggle(Sio, { ds: "caption.captionEnable", tab: 3, icon: "type", label: "Closed captions", detail: "Shows captions when a programme has them." }),
        R.lg(page, { tab: 3, target: "accessibility", detail: "High contrast, grayscale, invert colours, screen zoom, sign language." }),

        { kind: "value", tab: 4, icon: "tv", label: function() { return Sio.has("network.deviceName") ? Sio.get("network.deviceName") : "This TV" }, detail: "The name on the network and in Home Assistant.",
          value: "", chevron: false, info: true },
        { kind: "value", tab: 4, icon: "sparkles", label: "LumaTV", detail: "The Home, panels and Settings you are looking at. Theme: " + (Lu.theme.name || "LumaTV") + ".",
          value: function() { return Helper.up ? "Helper running" : "Helper stopped" }, chevron: false, info: true },
        R.lg(page, { tab: 4, id: "update", target: "SoftwareUpdate", label: "Software update", detail: "webOS updates, checked and installed only when you choose." }),
        R.lg(page, { tab: 4, id: "info", target: "TVInformation", label: "TV information", detail: "Software version, serial number, model code, network addresses." })
    ]
}
