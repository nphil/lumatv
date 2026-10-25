import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// Home & look (docs/mocks/30-settings-home): the theme's few knobs a person changes from the sofa
// (theme.json, the same file `lumatv theme set` and the Home popovers write), in four tabs. A row reads
// the live theme through Lu and writes through the host's patch, which applies it at once and saves it.
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
    readonly property var saver: theme.screensaver || ({})
    readonly property var art: saver.art || ({})
    readonly property string saverStyle: saver.style || "clock"
    readonly property var artEvery: [5, 15, 60]
    readonly property var logo: theme.logo || ({})
    subtitle: (tintOn ? "Dynamic tint" : "Fixed accent") + " \u00b7 " + material
    summary: (tintOn ? "Dynamic tint" : "Fixed accent") + " \u00b7 " + wallName()

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
        { kind: "value", tab: 0, icon: "image", label: "Wallpaper", detail: "The wallpaper is a file in the theme folder; a slideshow from a USB drive is not built yet.",
          value: function() { return page.wallName() }, info: true },
        { kind: "toggle", tab: 0, icon: "palette", label: "Dynamic tint", detail: "Panes take a hint of colour from the wallpaper.",
          on: function() { return page.tintOn }, act: function(v) { page.patch({ accent: v ? "wallpaper" : "tile" }) } },
        { kind: "seg", tab: 0, icon: "moon", label: "Appearance",
          detail: function() { return (Lu.isLight ? "Light now. " : "Dark now. ") + "Auto follows the wallpaper's brightness." },
          options: ["Dark", "Light", "Auto"], index: function() { return page.materials.indexOf(page.material) },
          act: function(i) { page.patch({ material: page.materials[i] }) } },
        { kind: "seg", tab: 0, icon: "clock", label: "Clock on Home", detail: "Analogue with the date, or digital with the greeting.",
          options: ["Analogue", "Digital", "Off"], index: function() { return page.clockStyles.indexOf(page.clockStyle) },
          act: function(i) { page.patch(i === 2 ? { clock: { enabled: false } } : { clock: { enabled: true, style: page.clockStyles[i] } }) } },
        { kind: "toggle", tab: 0, icon: "waves", label: "Wallpaper motion", detail: "The wallpaper drifts about ten times a second. Off keeps it still.",
          on: function() { return page.motionOn }, act: function(v) { page.patch({ wallpaperMotion: { enabled: v } }) } },
        R.lg(page, { tab: 0, target: "HomeSettings", detail: "Home promotion, content recommendation, Home animation, power-on screen." }),

        { kind: "toggle", tab: 1, icon: "clock", label: "12-hour clock", detail: "Off shows 24-hour time on Home and in the bar.",
          on: function() { return !theme.clock || theme.clock.twelveHour !== false }, act: function(v) { page.patch({ clock: { twelveHour: v } }) } },
        { kind: "toggle", tab: 1, icon: "sunrise", label: "Greeting", detail: "Good morning, afternoon or evening under the digital clock.",
          on: function() { return !theme.clock || theme.clock.greeting !== false }, act: function(v) { page.patch({ clock: { greeting: v } }) } },
        { kind: "seg", tab: 1, icon: "sun-medium", label: "Temperature", detail: function() { return "Weather for " + (page.theme.weather && page.theme.weather.zip ? page.theme.weather.zip : "your zip code") + "." },
          options: ["\u00b0F", "\u00b0C"], index: function() { return page.theme.weather && page.theme.weather.units === "celsius" ? 1 : 0 },
          act: function(i) { page.patch({ weather: { units: i === 1 ? "celsius" : "fahrenheit" } }) } },

        { kind: "seg", tab: 2, icon: "moon", label: "Screensaver", detail: "Starts after a while with no key. It moves so the panel never holds one picture.",
          options: ["Clock", "Art", "Off"], index: function() { var s = page.saverStyle; return s === "off" ? 2 : s === "art" ? 1 : 0 },
          act: function(i) { page.patch({ screensaver: { style: ["clock", "art", "off"][i] } }) } },
        { kind: "seg", tab: 2, icon: "hourglass", label: "Start after", detail: "Minutes with no key or sound.",
          options: ["5 min", "10 min", "30 min"], index: function() { var m = page.saver.afterMin || 10; return m <= 5 ? 0 : m <= 10 ? 1 : 2 },
          enabled: function() { return page.saverStyle !== "off" }, act: function(i) { page.patch({ screensaver: { afterMin: [5, 10, 30][i] } }) } },
        { kind: "seg", tab: 2, icon: "gauge", label: "Dim over time", detail: "Dims to 60 % across the first hour so a long stay is gentle on the panel.",
          options: ["On", "Off"], index: function() { return page.saver.dim === false ? 1 : 0 },
          enabled: function() { return page.saverStyle !== "off" }, act: function(i) { page.patch({ screensaver: { dim: i === 0 } }) } },
        { kind: "toggle", tab: 2, icon: "house-wifi", label: "Weather and a home fact", detail: "Under the clock: the temperature and one thing about the house, such as a door locked.",
          visible: function() { return page.saverStyle === "clock" }, on: function() { return page.saver.glance !== false },
          act: function(v) { page.patch({ screensaver: { glance: v } }) } },
        { kind: "seg", tab: 2, icon: "timer", label: "Change the art every", detail: "A new painting from the public-domain set.",
          visible: function() { return page.saverStyle === "art" }, options: ["5 min", "15 min", "1 hour"],
          index: function() { var m = page.art.everyMin || 15; return m <= 5 ? 0 : m <= 15 ? 1 : 2 }, act: function(i) { page.artPatch("everyMin", page.artEvery[i]) } },
        { kind: "seg", tab: 2, icon: "list-video", label: "Order", detail: "Shuffle mixes the set; In order walks through it.",
          visible: function() { return page.saverStyle === "art" }, options: ["Shuffle", "In order"],
          index: function() { return page.art.order === "sequence" ? 1 : 0 }, act: function(i) { page.artPatch("order", i === 1 ? "sequence" : "shuffle") } },
        { kind: "toggle", tab: 2, icon: "type", label: "Artist and title", detail: "A small caption with the artist, the title and the year.",
          visible: function() { return page.saverStyle === "art" }, on: function() { return page.art.caption !== false },
          act: function(v) { page.artPatch("caption", v) } },
        { kind: "seg", tab: 2, icon: "power", label: "Then go to standby", detail: "The panel turns off after this long in the screensaver.",
          options: ["Never", "2 h", "4 h", "8 h"], index: function() { var h = page.saver.standbyHours; return h === undefined ? 2 : h === 0 ? 0 : h <= 2 ? 1 : h <= 4 ? 2 : 3 },
          enabled: function() { return page.saverStyle !== "off" }, act: function(i) { page.patch({ screensaver: { standbyHours: [0, 2, 4, 8][i] } }) } },

        { kind: "seg", tab: 3, icon: "power", label: "Power-on logo", detail: "Shown for a second when the TV starts.",
          options: ["Panes", "Arc", "Name only"], index: function() { return Math.max(0, Math.min(2, (page.logo.on || 1) - 1)) },
          act: function(i) { page.patch({ logo: { on: i + 1 } }) } },
        { kind: "seg", tab: 3, icon: "moon", label: "Power-off logo", detail: "Shown for half a second as the TV goes dark.",
          options: ["Panes", "Arc", "Name only"], index: function() { return Math.max(0, Math.min(2, (page.logo.off || 1) - 1)) },
          act: function(i) { page.patch({ logo: { off: i + 1 } }) } },
        R.note("In standby the panel goes fully dark: LG's Always Ready and Gallery+ screens are off, and the TV still wakes at once.", 3),
        R.lg(page, { tab: 3, target: "LifeOnScreenMode", label: "LG standby screens", detail: "Always Ready and the standby light, in LG's own screen." })
    ]
}
