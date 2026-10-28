import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Home & look (docs/mocks/30-settings-home): the theme's few knobs a person changes from the sofa
// (theme.json, the same file `lumatv theme set` and the Home popovers write), in four tabs. A row reads
// the live theme through Lu and writes through the host's patch, which applies it at once and saves it. The few rows that are the TV's own
// settings (Home Settings and AI Recommendation on Home, LG's screen saver time and promotion on Screensaver, Always Ready on Standby) go through Sio, on LG's keys.
PageList {
    id: page
    title: "Home & look"
    tabs: ["Look", "Home", "Screensaver", "Standby"]

    readonly property var theme: Lu.theme || ({})
    readonly property bool tintOn: theme.accent === "wallpaper"
    readonly property var materials: ["dark", "light", "auto"]
    readonly property string material: theme.material || "auto"
    readonly property var clockStyles: ["analog", "digital", "off"]
    readonly property string clockStyle: theme.clock && theme.clock.enabled === false ? "off" : ((theme.clock && theme.clock.style) || "analog")
    readonly property bool motionOn: !theme.wallpaperMotion || theme.wallpaperMotion.enabled !== false
    readonly property bool transitionsOn: !!(theme.motion && theme.motion.transitions)
    readonly property var saver: theme.screensaver || ({})
    readonly property var art: saver.art || ({})
    readonly property string saverStyle: saver.style || "clock"
    readonly property var artEvery: [5, 15, 60]
    readonly property var logo: theme.logo || ({})
    readonly property var depthLevels: ["flat", "normal", "deep"]      // the steps of theme.depth.steps
    summary: ""                                         // nothing here is a state to glance at from the rail

    // the TV's own settings this page also holds: Home Settings and AI Recommendation (Home), the screen saver's two (Screensaver), Always Ready (Standby)
    function refreshTv() {
        Sio.read("general", ["aiNudge", "aiSettingsNudge", "lifeOnScreenMode", "homeAutoLaunch", "homePromotion", "screenSaverAd"])
        Sio.read("option", ["broadcastInfoNoti", "lifeOnScreenEnergySaving", "lifeOnScreenNotification", "lifeOnScreenUsingMotionSensor", "screenSaverTimer"])
        Sio.read("other", ["contentRecommendation"])
    }
    Component.onCompleted: refreshTv()
    onOpened: refreshTv()
    readonly property bool alwaysReady: Sio.has("general.lifeOnScreenMode") && String(Sio.get("general.lifeOnScreenMode")) !== "off"
    function whenReady(row) {
        return R.over(row, { enabled: function() { return page.alwaysReady && Sio.usable(row.id) }, detail: function() { return !Sio.has(row.id) ? "TV not answering" : page.alwaysReady ? "" : "Turn Always Ready on" } })
    }
    function patch(p) { if (host) host.patch(p) }
    // Theme patches merge one level deep, so a change to one art option carries the whole art block.
    function artPatch(field, v) {
        var a = { everyMin: art.everyMin || 15, order: art.order || "shuffle", caption: art.caption !== false }
        a[field] = v
        patch({ screensaver: { art: a } })
    }
    // The shipped wallpaper is "Aurora"; another file shows as its own name.
    function wallName() {
        var f = theme.wallpaper ? String(theme.wallpaper).replace(/^.*\//, "").replace(/\.[a-z]+$/i, "") : ""
        return f === "" || f === "wall_1080" ? "Aurora" : f
    }

    rows: [
        { kind: "toggle", tab: 0, icon: "palette", label: "Dynamic tint", help: "Panes take a hint of colour from the wallpaper.",
          on: function() { return page.tintOn }, act: function(v) { page.patch({ accent: v ? "wallpaper" : "tile" }) } },
        { kind: "seg", tab: 0, icon: "moon", label: "Appearance", help: "Auto follows the wallpaper's brightness.",
          detail: function() { return page.material === "auto" ? (Lu.isLight ? "Light now" : "Dark now") : "" },
          options: ["Dark", "Light", "Auto"], index: function() { return page.materials.indexOf(page.material) },
          act: function(i) { page.patch({ material: page.materials[i] }) } },
        { kind: "seg", tab: 0, icon: "clock", label: "Clock on Home",
          options: ["Analogue", "Digital", "Off"], index: function() { return page.clockStyles.indexOf(page.clockStyle) },
          act: function(i) { page.patch(i === 2 ? { clock: { enabled: false } } : { clock: { enabled: true, style: page.clockStyles[i] } }) } },
        { kind: "toggle", tab: 0, icon: "waves", label: "Wallpaper motion",
          on: function() { return page.motionOn }, act: function(v) { page.patch({ wallpaperMotion: { enabled: v } }) } },
        { kind: "seg", tab: 0, icon: "blend", label: "Glass depth",
          options: ["Flat", "Normal", "Deep"], index: function() { var i = page.depthLevels.indexOf(Lu.depthLevel); return i < 0 ? undefined : i },
          act: function(i) { page.patch({ depth: { level: page.depthLevels[i] } }) } },
        { kind: "toggle", tab: 0, icon: "sparkles", label: "Smooth transitions", help: "Apps grow out of their tile and shrink back.",
          on: function() { return page.transitionsOn }, act: function(v) { page.patch({ motion: { transitions: v } }) } },

        R.link({ tab: 1, id: "widgets", icon: "layout-grid", label: "Home widgets",
                 value: function() { return ((HomeLayout.layout.widgets || []).filter(function(e) { return e.type !== "dock" && !e.hidden }).length) + " shown" },
                 go: function() { page.host.openPage("widgets") } }),
        { kind: "toggle", tab: 1, icon: "clock", label: "12-hour clock",
          on: function() { return !theme.clock || theme.clock.twelveHour !== false }, act: function(v) { page.patch({ clock: { twelveHour: v } }) } },
        R.link({ tab: 1, id: "homeassistant", icon: "house-wifi", label: "Home Assistant",
                 value: function() { return PluginHub.haWords }, go: function() { page.host.openPage("homeassistant") } }),
        { kind: "toggle", tab: 1, icon: "sunrise", label: "Greeting", help: "Good morning, afternoon or evening under the digital clock.",
          on: function() { return !theme.clock || theme.clock.greeting !== false }, act: function(v) { page.patch({ clock: { greeting: v } }) } },
        { kind: "seg", tab: 1, icon: "sun-medium", label: "Temperature", help: function() { return "Weather for " + (page.theme.weather && page.theme.weather.zip ? page.theme.weather.zip : "your zip code") + "." },
          options: ["\u00b0F", "\u00b0C"], index: function() { return page.theme.weather && page.theme.weather.units === "celsius" ? 1 : 0 },
          act: function(i) { page.patch({ weather: { units: i === 1 ? "celsius" : "fahrenheit" } }) } },
        // LG's Home Settings. Power On Screen writes what LG's row writes with it: the input app stays off and the choice is marked as the owner's.
        C.lgShown(Sio, "general.homeAutoLaunch", {
            kind: "seg", tab: 1, id: "poweronscreen", icon: "power", label: "Power On Screen", help: "What the TV shows when it is turned on: Home, or the input it was last on.",
            detail: R.unavailable(Sio, "general.homeAutoLaunch"), enabled: function() { return Sio.usable("general.homeAutoLaunch") },
            options: ["Home", "Recent Input"],
            index: function() { return !Sio.has("general.homeAutoLaunch") ? undefined : String(Sio.get("general.homeAutoLaunch")) === "on" ? 0 : 1 },
            act: function(i) { Sio.writeMany("general", { homeAutoLaunch: i === 0 ? "on" : "off", selectedInputApp: "off", checkHomeAutoLaunchValue: "user" }) } }),
        C.lgShown(Sio, "general.homePromotion", R.toggle(Sio, { ds: "general.homePromotion", tab: 1, icon: "star", label: "Home Promotion",
                                                               help: "Shows or hides the sponsored ads in LG's own Home menu." })),
        C.lgShown(Sio, "other.contentRecommendation", R.toggle(Sio, { ds: "other.contentRecommendation", tab: 1, icon: "list-video", label: "Content Recommendation",
                                                                      help: "Shows or hides the content recommendations in LG's own Home menu. Not available in every country." })),
        C.lgShown(Sio, "option.broadcastInfoNoti", R.toggle(Sio, { ds: "option.broadcastInfoNoti", tab: 1, icon: "sparkles", label: "Who.Where.What?",
                                                                  help: "LG's suggestions about the people, places and things in what Live TV is showing." })),
        C.lgShown(Sio, "general.aiNudge", R.toggle(Sio, { ds: "general.aiNudge", tab: 1, icon: "sparkles", label: "Smart Tips",
                                                         help: "LG's tips about features, shown now and then on Home." })),
        C.lgShown(Sio, "general.aiSettingsNudge", R.toggle(Sio, { ds: "general.aiSettingsNudge", tab: 1, icon: "sparkles", label: "Smart Tips in Settings",
                                                                 help: "LG's tips about settings, shown inside LG's Settings screens." })),

        { kind: "seg", tab: 2, icon: "moon", label: "Screensaver", help: "Starts after a while with no key. It moves, so the panel never holds one picture.",
          options: ["Clock", "Art", "Off"], index: function() { var s = page.saverStyle; return s === "off" ? 2 : s === "art" ? 1 : 0 },
          act: function(i) { page.patch({ screensaver: { style: ["clock", "art", "off"][i] } }) } },
        { kind: "seg", tab: 2, icon: "gauge", label: "Dim over time", help: "Dims to 60 % over the first hour, gentle on the panel.",
          options: ["On", "Off"], index: function() { return page.saver.dim === false ? 1 : 0 },
          enabled: function() { return page.saverStyle !== "off" }, act: function(i) { page.patch({ screensaver: { dim: i === 0 } }) } },
        { kind: "toggle", tab: 2, icon: "house-wifi", label: "Weather and home fact", help: "Under the clock: the temperature and one thing about the house, such as a door locked.",
          visible: function() { return page.saverStyle === "clock" }, on: function() { return page.saver.glance !== false },
          act: function(v) { page.patch({ screensaver: { glance: v } }) } },
        { kind: "seg", tab: 2, icon: "timer", label: "Change the art every",
          visible: function() { return page.saverStyle === "art" }, options: ["5 min", "15 min", "1 hour"],
          index: function() { var m = page.art.everyMin || 15; return m <= 5 ? 0 : m <= 15 ? 1 : 2 }, act: function(i) { page.artPatch("everyMin", page.artEvery[i]) } },
        { kind: "seg", tab: 2, icon: "list-video", label: "Order",
          visible: function() { return page.saverStyle === "art" }, options: ["Shuffle", "In order"],
          index: function() { return page.art.order === "sequence" ? 1 : 0 }, act: function(i) { page.artPatch("order", i === 1 ? "sequence" : "shuffle") } },
        { kind: "toggle", tab: 2, icon: "type", label: "Artist and title",
          visible: function() { return page.saverStyle === "art" }, on: function() { return page.art.caption !== false },
          act: function(v) { page.artPatch("caption", v) } },
        { kind: "seg", tab: 2, icon: "power", label: "Then go to standby", help: "The panel turns off after this long in the screensaver.",
          options: ["Never", "2 h", "4 h", "8 h"], index: function() { var h = page.saver.standbyHours; return h === undefined ? 2 : h === 0 ? 0 : h <= 2 ? 1 : h <= 4 ? 2 : 3 },
          enabled: function() { return page.saverStyle !== "off" }, act: function(i) { page.patch({ screensaver: { standbyHours: [0, 2, 4, 8][i] } }) } },
        { kind: "toggle", tab: 2, icon: "moon-star", label: "LumaTV saver", help: "LumaTV's clock or art shows when the TV is idle on Home.",
          enabled: function() { return page.saverStyle !== "off" }, on: function() { return page.saver.own !== false },
          act: function(v) { page.patch({ screensaver: { own: v } }) } },
        // LG's own screen saver: its standby time (LG marks the choice as the owner's with the second key) and its ads
        C.choice(Sio, { ds: "option.screenSaverTimer", tab: 2, icon: "timer", label: "Screen Saver Standby Time", page: page,
                        names: { "3": "3 minutes", "10": "10 minutes", "20": "20 minutes", "30": "30 minutes" }, help: "How long without a key before LG's screen saver starts.",
                        write: function(v) { Sio.writeMany("option", { screenSaverTimer: v, checkChangedScreenSaverTimerByUser: "user" }) } }),
        C.lgShown(Sio, "general.screenSaverAd", R.toggle(Sio, { ds: "general.screenSaverAd", tab: 2, icon: "star", label: "Screen Saver Promotion",
                                                                help: "Shows or hides the ads some apps, such as LG Channels, put on LG's screen saver." })),

        { kind: "seg", tab: 3, icon: "power", label: "Power-on logo",
          options: ["Panes", "Arc", "Name only"], index: function() { return Math.max(0, Math.min(2, (page.logo.on || 1) - 1)) },
          act: function(i) { page.patch({ logo: { on: i + 1 } }) } },
        { kind: "seg", tab: 3, icon: "moon", label: "Power-off logo",
          options: ["Panes", "Arc", "Name only"], index: function() { return Math.max(0, Math.min(2, (page.logo.off || 1) - 1)) },
          act: function(i) { page.patch({ logo: { off: i + 1 } }) } },
        C.lgShown(Sio, "general.lifeOnScreenMode", {
            kind: "toggle", tab: 3, id: "alwaysready", icon: "moon-star", label: "Always Ready", help: "LG's standby: a wallpaper, the voice and music from a phone while the TV is off. It uses more power than a dark panel.",
            detail: R.unavailable(Sio, "general.lifeOnScreenMode"), enabled: function() { return Sio.usable("general.lifeOnScreenMode") },
            on: function() { return page.alwaysReady },
            act: function(v) {
                if (v) { Sio.write("general.lifeOnScreenMode", "allEnabled"); return }
                C.ask(page, { icon: "moon-star", title: "Turn Always Ready off?", body: "In standby the TV goes dark.", yes: "Turn off", no: "Cancel" }, function() { Sio.write("general.lifeOnScreenMode", "off") })()
            } }),
        whenReady(C.lgShown(Sio, "option.lifeOnScreenEnergySaving", C.seg(Sio, { ds: "option.lifeOnScreenEnergySaving", tab: 3, icon: "leaf", label: "Always Ready energy saving",
                                                                                names: { auto: "Auto", off: "Off" }, help: "Lets Always Ready dim and rest when nobody is near." }))),
        whenReady(C.lgShown(Sio, "option.lifeOnScreenNotification", R.toggle(Sio, { ds: "option.lifeOnScreenNotification", tab: 3, on: "true", off: "false", icon: "bell",
                                                                                   label: "Always Ready notifications", help: "Shows LG's notices on the Always Ready screen." }))),
        whenReady(C.lgShown(Sio, "option.lifeOnScreenUsingMotionSensor", R.toggle(Sio, { ds: "option.lifeOnScreenUsingMotionSensor", tab: 3, on: "true", off: "false", icon: "scan-eye",
                                                                                        label: "Always Ready motion sensor", help: "Wakes the screen when someone walks by." })))
    ]
}
