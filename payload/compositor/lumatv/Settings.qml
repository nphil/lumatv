import QtQuick 2.4
import "kit"

// LumaTV's Settings (docs/mocks/30-settings-home and its sisters): a bar with the way back, the clock
// and the link state, a rail of nine sections each with an icon tile and a live one-line summary, and
// the section's page in a glass pane beside it. A modal layer of Home: Layers builds it in the
// background after Home has presented, every page with it (each page is a settings/Page*.qml drawn
// from row descriptors by PageList), so opening it flips `visible` and moving down the rail switches
// pages without building anything. LumaHome hands it the D-pad as `key(dir)` (up, down, left, right, ok,
// back) and closes it on `closeRequested`. Nothing here writes a file or calls a service itself: a page
// asks the host through the signals below, so the same surface runs in the offline host, in Home and
// later in a window of its own.
//
// What LG's Settings has beyond these pages stays LG's, behind "More in LG settings" rows and the last
// row of the rail, which launch LG's app.
//
// Focus has two zones, the rail and the page. The rail moves with Up and Down and enters the page
// with Right or OK; the page keeps its own stop and handles its keys in `key(dir)`, which returns
// false for the ones it does not use, and Left or Back then returns to the rail (a child page, like
// the receiver's, goes back to its parent first).
Item {
    id: settings
    anchors.fill: parent

    // what the host connects
    signal themePatch(var patch)         // applied live (Home reads it back on its next poll)
    signal themePersist(var patch)       // written to the user's theme.json
    signal launchApp(string appId, var params)     // params: LG's launch parameters ({target: "Brightness"} lands on that screen)
    signal closeRequested()

    property string zone: "rail"
    property int railIndex: 0
    property string current: "picture"   // the page shown: a rail section, or a child page (the receiver)
    property bool rested: true
    property Item backdropItem: Lu.backdrop
    property int loaded: 0
    property var summaries: ({})         // page id -> the rail's second line, kept by each page's own summary
    readonly property bool built: loaded >= pages.length
    property var clock: new Date()

    // The rail's sections in the mock's order, then the pages that are children of one. `search: false` keeps a page out of Home's
    // search: the app chooser is only meaningful when opened for a key (`openPage("appkey", { key })`).
    readonly property var pages: [
        { id: "picture", title: "Picture", icon: "image", page: "settings/PagePicture.qml", rail: true },
        { id: "sound", title: "Sound", icon: "volume-2", page: "settings/PageSound.qml", rail: true },
        { id: "inputs", title: "Inputs & devices", icon: "cable", page: "settings/PageInputs.qml", rail: true },
        { id: "network", title: "Network", icon: "wifi", page: "settings/PageNetwork.qml", rail: true },
        { id: "ambient", title: "Ambient light", icon: "lamp", page: "settings/PageAmbient.qml", rail: true },
        { id: "look", title: "Home & look", icon: "house", page: "settings/PageLook.qml", rail: true },
        { id: "remote", title: "Remote & pointer", icon: "mouse-pointer-2", page: "settings/PageRemote.qml", rail: true },
        { id: "ai", title: "AI access", icon: "bot", page: "settings/PageAi.qml", rail: true },
        { id: "system", title: "System", icon: "settings", page: "settings/PageSystem.qml", rail: true },
        { id: "receiver", title: "Receiver", icon: "boom-box", page: "settings/PageReceiver.qml", rail: false },
        { id: "appkey", title: "App key", icon: "layout-grid", page: "settings/PageAppKey.qml", rail: false, search: false }
    ]
    readonly property var sections: pages.filter(function(p) { return p.rail })
    readonly property int railCount: sections.length + 1        // the last row is LG's Settings
    readonly property bool lgRow: railIndex >= sections.length

    function pageItem(id) {
        for (var i = 0; i < pageRepeater.count; i++) {
            var l = pageRepeater.itemAt(i)
            if (l && settings.pages[i].id === id) return l.item
        }
        return null
    }
    function summaryFor(id) { return summaries[id] || "" }
    function setSummary(id, text) {
        if (summaries[id] === text) return
        var s = {}
        for (var k in summaries) s[k] = summaries[k]
        s[id] = text
        summaries = s
    }
    // `arg` is what a child page is opened for (the app chooser: which key); the page reads it as `arg`.
    function openPage(id, arg) {
        current = id
        var p = pageItem(id)
        if (p && arg !== undefined) p.arg = arg
        if (p && p.reset) p.reset()
    }
    // Come to a page from outside (a tile of the quick panel): its section is the one on the rail and the page has the focus.
    function goTo(id) {
        var parentId = id === "receiver" ? "sound" : id === "appkey" ? "remote" : id
        for (var i = 0; i < sections.length; i++) if (sections[i].id === parentId) { railIndex = i; break }
        current = id
        zone = "page"
        var p = pageItem(id)
        if (p && p.reset) p.reset()
    }
    function patch(p) { themePatch(p); themePersist(p) }

    onRailIndexChanged: { rested = false; restTimer.restart(); if (railIndex < sections.length) current = sections[railIndex].id }
    onZoneChanged: { rested = false; restTimer.restart() }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: settings.rested = true }
    onVisibleChanged: {
        if (!visible) return
        zone = "rail"
        clock = new Date()
        if (railIndex < sections.length) current = sections[railIndex].id
    }

    function key(dir) {
        if (zone === "rail") {
            if (dir === "up") { railIndex = Math.max(0, railIndex - 1); return true }
            if (dir === "down") { railIndex = Math.min(railCount - 1, railIndex + 1); return true }
            if (dir === "back") { closeRequested(); return true }
            if (dir === "ok" && lgRow) { launchApp("com.palm.app.settings", {}); closeRequested(); return true }
            if (dir === "right" || dir === "ok") {
                if (!lgRow) {
                    zone = "page"
                    var p = pageItem(current)
                    if (p && p.reset) p.reset()
                }
                return true
            }
            return true
        }
        var page = pageItem(current)
        if (page && page.key(dir)) return true
        if (dir === "back" && current !== sections[railIndex].id) { openPage(sections[railIndex].id); return true }   // a child page returns to its parent
        if (dir === "left" || dir === "back") zone = "rail"
        return true
    }

    // The wallpaper dims a step behind the panes.
    Rectangle { anchors.fill: parent; color: Lu.scrim }

    // ---- bar
    LuPane {
        id: bar
        x: 64; y: 56; width: 1792; height: 60
        cardRadius: Lu.radius("sheet") / 2
        LuIcon { x: Lu.space(5); anchors.verticalCenter: parent.verticalCenter; name: "chevron-left"; size: 28; color: Lu.ink2 }
        LuText { x: Lu.space(5) + 40; anchors.verticalCenter: parent.verticalCenter; role: "title"; text: "Settings" }
        Row {
            anchors.verticalCenter: parent.verticalCenter
            x: parent.width - width - Lu.space(6)
            spacing: Lu.space(5)
            LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "wifi"; size: 28; color: Lu.ink2 }
            LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "mouse-pointer-2"; size: 28; color: Lu.ink2 }
            LuText { anchors.verticalCenter: parent.verticalCenter; role: "label"; color: Lu.ink2; text: Qt.formatTime(settings.clock, "h:mm AP") }
        }
        Timer { interval: 15000; running: settings.visible; repeat: true; onTriggered: settings.clock = new Date() }
    }

    // ---- rail
    LuPane {
        id: rail
        x: 64; y: 137; width: 446; height: 883
        Column {
            x: Lu.space(3); y: Lu.space(3)
            width: parent.width - 2 * Lu.space(3)
            spacing: 0
            Repeater {
                model: settings.sections
                delegate: LuRow {
                    width: parent.width
                    height: Math.round(Lu.row * 0.95)
                    icon: modelData.icon
                    label: modelData.title
                    detail: settings.summaryFor(modelData.id)
                    tileAccent: index === settings.railIndex
                    focused: settings.zone === "rail" && settings.railIndex === index
                    selected: settings.railIndex === index
                    rested: settings.rested
                    onActivated: { settings.railIndex = index; settings.zone = "page"; var p = settings.pageItem(settings.current); if (p && p.reset) p.reset() }
                    onHoverFocus: { settings.railIndex = index; settings.zone = "rail" }
                }
            }
            LuRow {
                width: parent.width
                height: Math.round(Lu.row * 0.95)
                icon: "square-arrow-out-up-right"
                label: "More in LG settings"
                detail: "Everything else, themed"
                focused: settings.zone === "rail" && settings.lgRow
                rested: settings.rested
                onActivated: { settings.launchApp("com.palm.app.settings", {}); settings.closeRequested() }
                onHoverFocus: { settings.railIndex = settings.sections.length; settings.zone = "rail" }
            }
        }
    }

    // ---- page
    LuPane {
        id: pane
        x: 536; y: 137; width: 1320; height: 883
        Item {
            id: content
            x: Lu.space(6); y: Lu.space(5)
            width: parent.width - 2 * Lu.space(6)
            height: parent.height - 2 * Lu.space(5)
            clip: true
            Repeater {
                id: pageRepeater
                model: settings.pages
                delegate: Loader {
                    id: slot
                    width: content.width
                    height: content.height
                    asynchronous: !(Lu.theme && Lu.theme.perf && Lu.theme.perf.syncResidents)
                    source: modelData.page
                    visible: status === Loader.Ready && settings.current === modelData.id && !settings.lgRow
                    onLoaded: { item.host = settings; settings.setSummary(modelData.id, item.summary || ""); settings.loaded++ }
                    Connections {
                        target: slot.item
                        ignoreUnknownSignals: true
                        onSummaryChanged: settings.setSummary(modelData.id, slot.item.summary || "")
                    }
                    onStatusChanged: if (status === Loader.Error) { console.warn("[Settings] page " + modelData.id + " did not load"); settings.loaded++ }
                }
            }
        }
    }
}
