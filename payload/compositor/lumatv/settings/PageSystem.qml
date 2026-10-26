import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "Names.js" as N
import "."

// System (docs/mocks/30-settings-system): the handful of TV-wide settings a person reaches for, and where the
// rest of LG's Settings is. Keys and categories are the ones LG's own Settings app uses. Software update,
// channels, privacy, the LG account and reset stay in LG's app (they are one-time screens with their own
// confirmations); each row that leaves lands on that screen (R.lg `target`), not at LG's Picture page.
PageList {
    id: page
    title: "System"
    tabs: ["General", "Time & power", "OLED care", "Accessibility", "About"]
    summary: Power.sleepLeft                           // the one live fact: a sleep timer that runs

    readonly property var locales: (Sio.get("localeInfo") || ({})).locales || ({})     // the set keeps localeInfo outside every category
    readonly property var zone: Sio.get("time.timeZone") || ({})
    readonly property string zoneName: typeof zone === "string" ? zone : (zone.City || zone.ZoneID || "")

    function refresh() {
        Sio.read("", ["localeInfo"])
        Sio.read("time", ["timeZone", "noOperationAutoOff", "autoOff15Min", "autoOff2HourOnTimer", "automaticStandBy", "onTimerEnable", "onTimerHour", "onTimerMinute",
                          "offTimerEnable", "offTimerHour", "offTimerMinute"])
        Sio.read("option", ["quickStartMode", "audioGuidance", "highContrast", "focusEmphasis", "screenMagnification", "adjustments"])
        Sio.read("picture", ["screenShift", "logoLuminanceAdjust"])
        Sio.read("caption", ["captionEnable"])
        Sio.read("sound", ["audioDescription"])
        Sio.read("network", ["deviceName"])
        Power.readSleep()
        Power.readPanel()
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    function launch(target) { if (host) { host.launchApp("com.palm.app.settings", { target: target }); host.closeRequested() } }

    // the clock as the Time & Date row shows it (it moves on every half minute while the page is up) and a timer as its row shows it
    property var now: new Date()
    Timer { interval: 30000; running: page.visible; repeat: true; onTriggered: page.now = new Date() }
    function clockText() { var h = now.getHours() % 12; return (h === 0 ? 12 : h) + ":" + (now.getMinutes() < 10 ? "0" : "") + now.getMinutes() + (now.getHours() < 12 ? " AM" : " PM") }
    function timerText(w) {
        if (String(Sio.get("time." + w + "TimerEnable")) !== "on") return "Off"
        var h = Sio.num("time." + w + "TimerHour", 0), m = Sio.num("time." + w + "TimerMinute", 0)
        return (h % 12 === 0 ? 12 : h % 12) + ":" + (m < 10 ? "0" : "") + m + (h < 12 ? " AM" : " PM")
    }

    rows: [
        { kind: "value", tab: 0, icon: "languages", label: "Language", help: "Menus, keyboard and audio language. OK opens LG's Language screen.", chevron: true,
          enabled: function() { return Sio.has("localeInfo") },
          value: function() { return R.name({ "en-US": "English (US)", "en-GB": "English (UK)", "es-US": "Spanish (US)", "fr-FR": "French" }, page.locales.UI, "") },
          act: function() { page.launch("Language") } },
        C.lgShown(Sio, "time.noOperationAutoOff", R.seg(Sio, { ds: "time.noOperationAutoOff", tab: 0, icon: "power", label: "No Operation Auto Off", help: "No key for this long turns the TV off.",
                     options: ["Off", "3 h", "6 h", "12 h", "24 h"], values: ["off", "3", "6", "12", "24"] })),
        R.lg(page, { tab: 0, label: "Open LG settings", help: "LG's full Settings app, themed. Channels, privacy, account, reset." }),

        { kind: "stepper", tab: 1, id: "sleep", icon: "hourglass", label: "Sleep Timer", detail: function() { return Power.sleepLeft },
          help: "The TV turns itself off after this long.",
          options: Power.sleepLabels, index: function() { return Power.sleepIndex }, act: function(i) { Power.setSleep(i) } },
        R.link({ tab: 1, id: "clock", icon: "clock", label: "Time & Date", value: function() { return page.clockText() }, go: function() { page.host.openPage("clock") } }),
        R.link({ tab: 1, id: "poweron", icon: "power", label: "Power On Timer", enabled: function() { return Sio.has("time.onTimerEnable") },
                 value: function() { return page.timerText("on") }, go: function() { page.host.openPage("poweron") } }),
        R.link({ tab: 1, id: "poweroff", icon: "power", label: "Power Off Timer", enabled: function() { return Sio.has("time.offTimerEnable") },
                 value: function() { return page.timerText("off") }, go: function() { page.host.openPage("poweroff") } }),
        C.lgShown(Sio, "time.autoOff15Min", R.toggle(Sio, { ds: "time.autoOff15Min", tab: 1, icon: "power", label: "No Signal Auto Off", help: "Switches off after 15 minutes without a picture." })),
        C.lgShown(Sio, "time.autoOff2HourOnTimer", R.toggle(Sio, { ds: "time.autoOff2HourOnTimer", tab: 1, icon: "hourglass", label: "Auto Power Off after 4 Hours",
                                                                help: "Turns the TV off after 4 hours without a key press when it was turned on by a timer." })),
        C.lgShown(Sio, "time.automaticStandBy", C.choice(Sio, { ds: "time.automaticStandBy", tab: 1, icon: "hourglass", label: "Auto Power Off", names: "automaticStandBy", page: page })),
        C.lgShown(Sio, "option.screenOff", C.action({ tab: 1, id: "screenoff", icon: "monitor", label: "Screen Off", button: "Turn off", enabled: function() { return Sio.usable("option.screenOff") },
                                                         help: "Turns the picture off and keeps the sound. Any key brings it back.", act: function() { Sio.write("option.screenOff", "on") } })),
        R.toggle(Sio, { ds: "option.quickStartMode", tab: 1, icon: "zap", label: "Quick Start+", help: "Wakes in about 2 s from standby. Uses a little power while off." }),

        { kind: "value", tab: 2, icon: "clock", label: "Panel hours", help: "How long the panel has been on, as LG counts it. It does not reset.",
          value: function() { return Power.panelHours !== "" ? Power.panelHours : "Not read" }, info: true, chevron: false,
          visible: function() { return Power.panelShown } },
        R.toggle(Sio, { ds: "picture.screenShift", tab: 2, icon: "scan-eye", label: "Screen shift", help: "Moves the picture a pixel now and then so the panel wears evenly." }),
        R.seg(Sio, { ds: "picture.logoLuminanceAdjust", tab: 2, icon: "contrast", label: "Dim logos and subtitles", help: "Lowers the brightness of static logos and subtitles so they do not burn in.",
                     options: ["Off", "Light", "Strong"], values: ["off", "light", "strong"] }),
        R.lg(page, { tab: 2, target: "OledCare", label: "Pixel refresher and panel care", help: "Runs while the TV is off and takes about an hour. LG's screen asks first." }),

        R.head("Vision", 3),
        R.link({ tab: 3, id: "audioguide", icon: "audio-lines", label: "Audio Guidance", enabled: function() { return Sio.has("option.audioGuidance") },
                 value: function() { return N.name("onoff", Sio.get("option.audioGuidance"), "") }, go: function() { page.host.openPage("audioguide") } }),
        C.display(Sio, { key: "highContrast", tab: 3, icon: "contrast", label: "Text with High Contrast", help: "Shows the menus' text in a stronger colour." }),
        C.display(Sio, { key: "focusEmphasis", tab: 3, icon: "scan-eye", label: "Focus with High Contrast", help: "Marks the selected item more strongly." }),
        C.display(Sio, { key: "blackAndWhite", tab: 3, icon: "moon", label: "Grayscale", help: "Turns every screen black and white." }),
        C.display(Sio, { key: "colorInversion", tab: 3, icon: "blend", label: "Invert Colors", help: "Inverts the colour of every screen." }),
        C.display(Sio, { key: "screenMagnification", tab: 3, icon: "search", label: "Screen Zoom", help: "Zooms the whole screen." }),
        R.head("Hearing", 3),
        R.link({ tab: 3, id: "captions", icon: "type", label: "Closed Caption", enabled: function() { return Sio.has("caption.captionEnable") },
                 value: function() { return N.name("onoff", Sio.get("caption.captionEnable"), "") }, go: function() { page.host.openPage("captions") } }),
        C.lgShown(Sio, "sound.audioDescription", R.toggle(Sio, { ds: "sound.audioDescription", tab: 3, icon: "audio-lines", label: "Audio Description" })),
        R.link({ tab: 3, id: "audiooptions", icon: "headphones", label: "Audio options", go: function() { page.host.openPage("audiooptions") } }),

        { kind: "value", tab: 4, icon: "tv", label: function() { return Sio.has("network.deviceName") ? Sio.get("network.deviceName") : "This TV" },
          help: "The name on the network and in Home Assistant.", value: "", chevron: false, info: true },
        { kind: "value", tab: 4, icon: "sparkles", label: "LumaTV",
          value: function() { return Helper.up ? "Helper running" : "Helper stopped" }, chevron: false, info: true },
        R.lg(page, { tab: 4, id: "update", target: "SoftwareUpdate", label: "Software update" }),
        R.lg(page, { tab: 4, id: "info", target: "TVInformation", label: "TV information" })
    ]
}
