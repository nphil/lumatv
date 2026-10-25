import QtQuick 2.4
import "kit"

// LumaTV's Settings: a rail of sections on the left, the section's page on the right, both glass
// panes over the dimmed wallpaper (docs/mocks/30-settings-home, 04-settings-ai-access). A modal layer of
// Home like the kit gallery: LumaHome builds it when it is asked for, hands it the D-pad as
// `key(dir)` (up, down, left, right, ok, back) and closes it on `closeRequested`. Nothing here
// writes a file or calls a service itself: a page asks the host through the signals below, so the
// same surface runs in the offline host, in Home, and later in a window of its own.
//
// Sections are the pages that exist. What LG's Settings has beyond them stays LG's, behind the
// last row of the rail ("More in LG settings"), which launches LG's app.
//
// Focus has two zones, the rail and the page. The rail moves with Up and Down and enters the page
// with Right or OK; the page keeps its own stop and handles its keys in `key(dir)`, which returns
// false for the ones it does not use, and Left or Back then returns to the rail.
Item {
    id: settings
    anchors.fill: parent

    // what the host connects
    signal themePatch(var patch)         // applied live (Home reads it back on its next poll)
    signal themePersist(var patch)       // written to the user's theme.json
    signal launchApp(string appId)
    signal closeRequested()

    property string zone: "rail"
    property int railIndex: 0
    property bool rested: true
    property Item backdropItem: Lu.backdrop

    readonly property var sections: [
        { id: "look", title: "Home & look", page: "settings/PageLook.qml" }
    ]
    readonly property int railCount: sections.length + 1        // the last row is LG's Settings
    readonly property bool lgRow: railIndex >= sections.length

    // The words under each section's name; the page shown fills in its own when it is up.
    function summaryFor(id) {
        if (id === "look") {
            var t = Lu.theme || ({})
            return (t.accent === "wallpaper" ? "Dynamic tint" : "Fixed accent") + " \u00b7 " + (t.material === "auto" || !t.material ? "auto light" : t.material)
        }
        return ""
    }
    function patch(p) { themePatch(p); themePersist(p) }

    onRailIndexChanged: { rested = false; restTimer.restart() }
    onZoneChanged: { rested = false; restTimer.restart() }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: settings.rested = true }

    function key(dir) {
        if (zone === "rail") {
            if (dir === "up") { railIndex = Math.max(0, railIndex - 1); return true }
            if (dir === "down") { railIndex = Math.min(railCount - 1, railIndex + 1); return true }
            if (dir === "back") { closeRequested(); return true }
            if (dir === "ok" && lgRow) { launchApp("com.palm.app.settings"); closeRequested(); return true }
            if (dir === "right" || dir === "ok") { if (!lgRow) zone = "page"; return true }
            return true
        }
        var p = pageLoader.item
        if (p && p.key(dir)) return true
        if (dir === "left" || dir === "back") zone = "rail"
        return true
    }

    // The wallpaper dims a step behind the panes.
    Rectangle { anchors.fill: parent; color: Lu.scrim }

    // ---- header
    Item {
        id: head
        x: 64; y: 56; width: 1792; height: 60
        Glass {
            mat: Lu.material; isLight: Lu.isLight; cardRadius: Lu.radius("sheet") / 2; focusMix: 0
            screenX: head.x; screenY: head.y; screenW: head.width; screenH: head.height
            backdrop: settings.backdropItem
        }
        LuText { x: Lu.space(6); anchors.verticalCenter: parent.verticalCenter; role: "label"; text: "\u2039"; color: Lu.ink2 }
        LuText { x: Lu.space(6) + 30; anchors.verticalCenter: parent.verticalCenter; role: "title"; text: "Settings" }
        LuText {
            anchors.verticalCenter: parent.verticalCenter
            x: parent.width - Lu.space(6) - width
            role: "label"; color: Lu.ink2
            text: Qt.formatTime(clock.now, "h:mm AP")
        }
        QtObject { id: clock; property var now: new Date() }
        Timer { interval: 15000; running: settings.visible; repeat: true; onTriggered: clock.now = new Date() }
    }

    // ---- rail
    Item {
        id: rail
        x: 64; y: 137; width: 446; height: 883
        Glass {
            mat: Lu.material; isLight: Lu.isLight; cardRadius: Lu.radius("sheet"); focusMix: 0
            screenX: rail.x; screenY: rail.y; screenW: rail.width; screenH: rail.height
            backdrop: settings.backdropItem
        }
        Column {
            x: Lu.space(3); y: Lu.space(3)
            width: parent.width - 2 * Lu.space(3)
            Repeater {
                model: settings.sections
                delegate: LuRow {
                    width: parent.width
                    label: modelData.title
                    detail: index === settings.railIndex && pageLoader.item && pageLoader.item.summary ? pageLoader.item.summary : settings.summaryFor(modelData.id)
                    focused: settings.zone === "rail" && settings.railIndex === index
                    selected: settings.railIndex === index
                    rested: settings.rested
                    onActivated: { settings.railIndex = index; settings.zone = "page" }
                    onHoverFocus: { settings.railIndex = index; settings.zone = "rail" }
                }
            }
            LuRow {
                width: parent.width
                label: "More in LG settings"
                detail: "Picture, sound, network and the rest"
                focused: settings.zone === "rail" && settings.lgRow
                rested: settings.rested
                onActivated: { settings.launchApp("com.palm.app.settings"); settings.closeRequested() }
                onHoverFocus: { settings.railIndex = settings.sections.length; settings.zone = "rail" }
            }
        }
    }

    // ---- page
    Item {
        id: pane
        x: 536; y: 137; width: 1320; height: 883
        Glass {
            mat: Lu.material; isLight: Lu.isLight; cardRadius: Lu.radius("sheet"); focusMix: 0
            screenX: pane.x; screenY: pane.y; screenW: pane.width; screenH: pane.height
            backdrop: settings.backdropItem
        }
        Loader {
            id: pageLoader
            x: Lu.space(6); y: Lu.space(5)
            width: parent.width - 2 * Lu.space(6)
            height: parent.height - 2 * Lu.space(5)
            clip: true
            source: settings.lgRow ? "" : settings.sections[settings.railIndex].page
            onLoaded: item.host = settings
        }
    }
}
