import QtQuick 2.4
import "../kit"

// Home & look: the theme's few knobs a person changes from the sofa (theme.json, the same file
// `lumatv theme set` and the Home popovers write). Rows are LuRow with the control in `trailing`;
// Left and Right move inside a segmented row, OK flips a switch or picks the focused segment.
Item {
    id: page
    property var host                                   // the Settings surface: patch(), launchApp(), closeRequested()
    readonly property var theme: Lu.theme || ({})
    readonly property string summary: (tintOn ? "Dynamic tint" : "Fixed accent") + " \u00b7 " + materialNames[materialIndex].toLowerCase()

    // stops: 0 dynamic tint, 1 appearance, 2 clock, 3 wallpaper motion, 4 LG's Home settings
    property int stop: 0
    property int segFocus: 0
    readonly property int stops: 5
    readonly property bool rested: host ? host.rested : true
    readonly property bool active: host ? host.zone === "page" : false

    readonly property bool tintOn: theme.accent === "wallpaper"
    readonly property var materials: ["dark", "light", "auto"]
    readonly property var materialNames: ["Dark", "Light", "Auto"]
    readonly property int materialIndex: Math.max(0, materials.indexOf(theme.material || "auto"))
    readonly property var clockStyles: ["analog", "digital"]
    readonly property int clockIndex: Math.max(0, clockStyles.indexOf((theme.clock && theme.clock.style) || "digital"))
    readonly property bool motionOn: !theme.wallpaperMotion || theme.wallpaperMotion.enabled !== false

    function enterStop(n) {
        stop = n
        segFocus = n === 1 ? materialIndex : n === 2 ? clockIndex : 0
    }
    // The pointer arrived on a row: the page takes the focus, as the D-pad would.
    function pointAt(n) { if (host) host.zone = "page"; enterStop(n) }
    function key(dir) {
        if (dir === "up") { if (stop === 0) return false; enterStop(stop - 1); return true }
        if (dir === "down") { if (stop < stops - 1) enterStop(stop + 1); return true }
        if (stop === 1 || stop === 2) {
            var n = stop === 1 ? materials.length : clockStyles.length
            if (dir === "left") { if (segFocus === 0) return false; segFocus--; return true }
            if (dir === "right") { segFocus = Math.min(n - 1, segFocus + 1); return true }
        }
        if (dir === "ok") {
            if (stop === 0) host.patch({ accent: tintOn ? "tile" : "wallpaper" })
            else if (stop === 1) host.patch({ material: materials[segFocus] })
            else if (stop === 2) host.patch({ clock: { style: clockStyles[segFocus] } })
            else if (stop === 3) host.patch({ wallpaperMotion: { enabled: !motionOn } })
            else { host.launchApp("com.palm.app.settings"); host.closeRequested() }
            return true
        }
        return false
    }

    LuText { id: heading; role: "title"; text: "Home & look" }
    LuText {
        anchors.baseline: heading.baseline
        x: heading.width + Lu.space(4)
        role: "label"; color: Lu.ink2
        text: page.summary
    }

    Column {
        id: rows
        y: heading.height + Lu.space(5)
        width: parent.width
        LuRow {
            width: rows.width; label: "Dynamic tint"; detail: "Panes take a hint of colour from the wallpaper."
            focused: page.active && page.stop === 0; rested: page.rested
            onActivated: { page.pointAt(0); page.key("ok") }
            onHoverFocus: page.pointAt(0)
            trailing: LuToggle { checked: page.tintOn }
        }
        LuRow {
            width: rows.width; label: "Appearance"; detail: "Auto follows the wallpaper's brightness."
            focused: page.active && page.stop === 1; rested: page.rested
            onHoverFocus: page.pointAt(1)
            trailing: LuSegmented {
                options: page.materialNames; current: page.materialIndex
                focusedIndex: page.active && page.stop === 1 ? page.segFocus : -1
                onPicked: { page.pointAt(1); page.segFocus = index; page.key("ok") }
            }
        }
        LuRow {
            width: rows.width; label: "Clock on Home"; detail: "Analogue with the date, or digital with the greeting."
            focused: page.active && page.stop === 2; rested: page.rested
            onHoverFocus: page.pointAt(2)
            trailing: LuSegmented {
                options: ["Analogue", "Digital"]; current: page.clockIndex
                focusedIndex: page.active && page.stop === 2 ? page.segFocus : -1
                onPicked: { page.pointAt(2); page.segFocus = index; page.key("ok") }
            }
        }
        LuRow {
            width: rows.width; label: "Wallpaper motion"; detail: "The wallpaper drifts about ten times a second. Off keeps it still."
            focused: page.active && page.stop === 3; rested: page.rested
            onActivated: { page.pointAt(3); page.key("ok") }
            onHoverFocus: page.pointAt(3)
            trailing: LuToggle { checked: page.motionOn }
        }
        LuRow {
            width: rows.width; label: "More in LG settings"; detail: "Home promotion, content recommendation, Always Ready, standby light."
            value: "\u203a"
            focused: page.active && page.stop === 4; rested: page.rested
            onActivated: { page.pointAt(4); page.key("ok") }
            onHoverFocus: page.pointAt(4)
        }
    }
}
