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
    readonly property var logo: theme.logo || ({})
    subtitle: (tintOn ? "Dynamic tint" : "Fixed accent") + " \u00b7 " + material
    summary: (tintOn ? "Dynamic tint" : "Fixed accent") + " \u00b7 " + wallName()

    function patch(p) { if (host) host.patch(p) }
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
        R.lg(page, { tab: 0, detail: "Home promotion, content recommendation, Always Ready, standby light." }),

        { kind: "toggle", tab: 1, icon: "clock", label: "12-hour clock", detail: "Off shows 24-hour time on Home and in the bar.",
          on: function() { return !theme.clock || theme.clock.twelveHour !== false }, act: function(v) { page.patch({ clock: { twelveHour: v } }) } },
        { kind: "toggle", tab: 1, icon: "sunrise", label: "Greeting", detail: "Good morning, afternoon or evening under the digital clock.",
          on: function() { return !theme.clock || theme.clock.greeting !== false }, act: function(v) { page.patch({ clock: { greeting: v } }) } },
        { kind: "seg", tab: 1, icon: "sun-medium", label: "Temperature", detail: function() { return "Weather for " + (page.theme.weather && page.theme.weather.zip ? page.theme.weather.zip : "your zip code") + "." },
          options: ["\u00b0F", "\u00b0C"], index: function() { return page.theme.weather && page.theme.weather.units === "celsius" ? 1 : 0 },
          act: function(i) { page.patch({ weather: { units: i === 1 ? "celsius" : "fahrenheit" } }) } },

        { kind: "seg", tab: 2, icon: "moon", label: "Screensaver", detail: "Starts after a while with no key. It moves so the panel never holds one picture.",
          options: ["Clock", "Art", "Off"], index: function() { var s = page.saver.style || "clock"; return s === "off" ? 2 : s === "art" ? 1 : 0 },
          act: function(i) { page.patch({ screensaver: { style: ["clock", "art", "off"][i] } }) } },
        { kind: "seg", tab: 2, icon: "hourglass", label: "Start after", detail: "Minutes with no key or sound.",
          options: ["5 min", "10 min", "30 min"], index: function() { var m = page.saver.afterMin || 10; return m <= 5 ? 0 : m <= 10 ? 1 : 2 },
          enabled: function() { return (page.saver.style || "clock") !== "off" }, act: function(i) { page.patch({ screensaver: { afterMin: [5, 10, 30][i] } }) } },
        { kind: "seg", tab: 2, icon: "gauge", label: "Dim over time", detail: "Dims to 60 % across the first hour so a long stay is gentle on the panel.",
          options: ["On", "Off"], index: function() { return page.saver.dim === false ? 1 : 0 },
          enabled: function() { return (page.saver.style || "clock") !== "off" }, act: function(i) { page.patch({ screensaver: { dim: i === 0 } }) } },
        R.note("The screensaver is mostly black, everything on it drifts to a new spot every minute, and the panel goes to standby after a few hours (docs/PLAN.md, Phase 8).", 2),

        { kind: "seg", tab: 3, icon: "power", label: "Power-on logo", detail: "Shown for a second when the TV starts. Three styles, one chosen.",
          options: ["Style 1", "Style 2", "Style 3"], index: function() { return Math.max(0, Math.min(2, (page.logo.on || 1) - 1)) },
          act: function(i) { page.patch({ logo: { on: i + 1 } }) } },
        { kind: "seg", tab: 3, icon: "moon", label: "Power-off logo", detail: "Shown for half a second as the TV goes dark.",
          options: ["Style 1", "Style 2", "Style 3"], index: function() { return Math.max(0, Math.min(2, (page.logo.off || 1) - 1)) },
          act: function(i) { page.patch({ logo: { off: i + 1 } }) } },
        R.note("In standby the panel goes fully dark: LG's Always Ready and Gallery+ screens are off, and the TV still wakes at once.", 3)
    ]
}
