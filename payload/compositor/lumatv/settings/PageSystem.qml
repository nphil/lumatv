import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "Names.js" as N
import "."

// System (docs/mocks/30-settings-system): the handful of TV-wide settings a person reaches for, and where the
// rest of LG's Settings is. Keys and categories are the ones LG's own Settings app uses. TV information, the software update's
// state, privacy, self-diagnosis and reset are the Support page, the locks' state the Safety page; what is left in LG's app (the
// account, the apps, the channel lists) is behind the one row that opens it, each row that leaves landing on its own screen (R.lg `target`).
PageList {
    id: page
    title: "System"
    tabs: ["General", "Time & power", "OLED care", "Accessibility", "About"]
    summary: Power.sleepLeft                           // the one live fact: a sleep timer that runs

    readonly property var locales: (Sio.get("localeInfo") || ({})).locales || ({})     // the set keeps localeInfo outside every category
    readonly property var zone: Sio.get("time.timeZone") || ({})
    readonly property string zoneName: typeof zone === "string" ? zone : (zone.City || zone.ZoneID || "")
    readonly property var theme: Lu.theme || ({})
    function patch(p) { if (host) host.patch(p) }
    // One of LumaTV's panel-care switches (theme `oledcare`, read by the compositor within 2 s; each feature is off in the shipped theme until its TV check has passed).
    // A switch whose feature is not built yet still writes its key.
    function careRow(key, icon, label, help) {
        return { kind: "toggle", tab: 2, icon: icon, label: label, help: help,
                 on: function() { return !!page.theme.oledcare && page.theme.oledcare[key] === true },
                 act: function(v) { var o = {}; o[key] = v; page.patch({ oledcare: o }) } }
    }

    function refresh() {
        Sio.read("", ["localeInfo"])
        Sio.read("time", ["timeZone", "noOperationAutoOff", "autoOff15Min", "autoOff2HourOnTimer", "automaticStandBy", "onTimerEnable", "onTimerHour", "onTimerMinute",
                          "offTimerEnable", "offTimerHour", "offTimerMinute"])
        Sio.read("option", ["quickStartMode", "audioGuidance", "highContrast", "focusEmphasis", "screenMagnification", "adjustments", "emergencyInformationAtsc30",
                            "emergencyInformationLanguageAtsc30"])
        Sio.read("picture", ["screenShift", "logoLuminanceAdjust", "scheduledPixelCleaning"])
        Sio.read("other", ["oledCareMode", "oledCareRecommendation"])
        Sio.read("caption", ["captionEnable"])
        Sio.read("sound", ["audioDescription"])
        Sio.read("network", ["deviceName"])
        Power.readSleep()
        Power.readPanel()
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    // LG shows Emergency Information only on a TV with an ATSC 3.0 tuner
    Atsc3 { id: tuner }
    property alias atsc3: tuner.present

    function launch(target) { if (host) { host.launchApp("com.palm.app.settings", { target: target }); host.closeRequested() } }

    // the clock as the Time & Date row shows it (it moves on every half minute while the page is up) and a timer as its row shows it
    property var now: new Date()
    // triggeredOnStart: the page is built long before it is shown (it read the build time on the set), so it reads the clock when it is shown
    Timer { interval: 30000; running: page.visible; repeat: true; triggeredOnStart: true; onTriggered: page.now = new Date() }
    function clockText() { var h = now.getHours() % 12; return (h === 0 ? 12 : h) + ":" + (now.getMinutes() < 10 ? "0" : "") + now.getMinutes() + (now.getHours() < 12 ? " AM" : " PM") }
    function timerText(w) {
        if (String(Sio.get("time." + w + "TimerEnable")) !== "on") return "Off"
        var h = Sio.num("time." + w + "TimerHour", 0), m = Sio.num("time." + w + "TimerMinute", 0)
        return (h % 12 === 0 ? 12 : h % 12) + ":" + (m < 10 ? "0" : "") + m + (h < 12 ? " AM" : " PM")
    }

    rows: [
        R.link({ tab: 0, id: "language", icon: "languages", label: "Language & location", help: "Menu, audio and keyboard languages, the zip code and the LG services country.",
                 enabled: function() { return Sio.has("localeInfo") },
                 value: function() { return R.name({ "en-US": "English (US)", "en-GB": "English (UK)", "es-US": "Spanish (US)", "fr-FR": "French" }, page.locales.UI, "") },
                 go: function() { page.host.openPage("language") } }),
        C.lgShown(Sio, "time.noOperationAutoOff", R.seg(Sio, { ds: "time.noOperationAutoOff", tab: 0, icon: "power", label: "No Operation Auto Off", help: "No key for this long turns the TV off.",
                     options: ["Off", "3 h", "6 h", "12 h", "24 h"], values: ["off", "3", "6", "12", "24"] })),
        R.link({ tab: 0, id: "safety", icon: "shield", label: "Safety", help: "Which locks are on. A lock changes only with the PIN, in LG's screen.", go: function() { page.host.openPage("safety") } }),
        R.link({ tab: 0, id: "capture", icon: "camera", label: "Screenshots & recording", go: function() { page.host.openPage("capture") } }),
        R.lg(page, { tab: 0, label: "Open LG settings", help: "LG's full Settings app, themed. For what LumaTV does not cover: the account, apps and channel lists." }),

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
        C.lgShown(Sio, "other.oledCareMode", C.choice(Sio, { ds: "other.oledCareMode", tab: 2, icon: "shield-check", label: "Care Picture Settings", names: "oledCare", page: page,
                                                            help: "A picture set for the eyes or for the room: comfort, theater or kids. It replaces the energy saving step." })),
        C.lgShown(Sio, "picture.scheduledPixelCleaning", {
            kind: "toggle", tab: 2, id: "pixelcleaning", icon: "sparkles", label: "Pixel Cleaning", help: "The panel runs a cleaning the next time the TV is turned off, and the TV stays off for about an hour.",
            detail: R.unavailable(Sio, "picture.scheduledPixelCleaning"), enabled: function() { return Sio.usable("picture.scheduledPixelCleaning") },
            on: function() { return String(Sio.get("picture.scheduledPixelCleaning")) === "on" },
            act: function(v) {
                if (!v) { Sio.write("picture.scheduledPixelCleaning", "off"); return }
                C.ask(page, { icon: "sparkles", title: "Clean the pixels?", body: "It runs when the TV is turned off next, and takes about an hour.", yes: "Turn on", no: "Cancel" },
                      function() { Sio.write("picture.scheduledPixelCleaning", "on") })()
            } }),
        C.lgShown(Sio, "other.oledCareRecommendation", R.toggle(Sio, { ds: "other.oledCareRecommendation", tab: 2, icon: "lightbulb", label: "Care Recommendations",
                                                                      help: "Suggests a picture setting for eye strain or the room, from how the TV is watched." })),

        R.head("LumaTV panel care", 2),
        page.careRow("enabled", "shield-check", "LumaTV screens", "Moves LumaTV's own screens a pixel now and then and dims Home when it sits idle."),
        page.careRow("appDim", "sun-medium", "Dim still apps", "Dims an app a little when its picture has not changed for a few minutes. Video and any key bring it back."),
        page.careRow("appShift", "waves", "Shift apps", "Slowly moves an app's picture a few pixels while it sits still. A key or video puts it back."),
        page.careRow("boxDim", "sun-dim", "Dim bright still areas", "Dims only the bright parts of a picture that stays still, such as menu boxes. Video is never touched."),
        page.careRow("refreshGuard", "rotate-ccw", "Protect panel refresh", "Keeps LG's panel refresh from being cut short when the TV is turned on just after it turns off."),

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
        R.over(R.link({ tab: 3, id: "emergency", icon: "octagon-alert", label: "Emergency Information", help: "The emergency alert of NEXTGEN TV, and the language it is spoken in.",
                       go: function() { page.host.openPage("emergency") } }),
               { visible: function() { return page.atsc3 && (C.shown(Sio, "option.emergencyInformationAtsc30") || C.shown(Sio, "option.emergencyInformationLanguageAtsc30")) } }),

        { kind: "value", tab: 4, icon: "tv", label: function() { return Sio.has("network.deviceName") ? Sio.get("network.deviceName") : "This TV" },
          help: "The name on the network and in Home Assistant.", value: "", chevron: false, info: true },
        { kind: "value", tab: 4, icon: "sparkles", label: "LumaTV",
          value: function() { return Helper.up ? "Helper running" : "Helper stopped" }, chevron: false, info: true },
        R.link({ tab: 4, id: "support", icon: "circle-help", label: "Support", help: "TV information, software update, privacy, self-diagnosis and reset.", go: function() { page.host.openPage("support") } })
    ]
}
