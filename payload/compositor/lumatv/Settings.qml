import QtQuick 2.4
import "kit"
import "settings/Children.js" as Children

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
        { id: "homeassistant", title: "Home Assistant", icon: "house-wifi", page: "settings/PageHomeAssistant.qml", rail: false },
        { id: "widgets", title: "Home widgets", icon: "layout-grid", page: "settings/PageWidgets.qml", rail: false },
        { id: "appkey", title: "App key", icon: "layout-grid", page: "settings/PageAppKey.qml", rail: false, search: false }
    ].concat(Children.pages)
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
    // The pages a child was opened from, nearest last: Back goes to the one it was opened from, whatever the depth, and `up()` is that
    // step. `arg` is what a child page is opened for (the app chooser: which key); the page reads it as `arg`.
    property var trail: []
    function openPage(id, arg) {
        var at = trail.indexOf(id)
        if (at >= 0) trail = trail.slice(0, at)
        else if (current !== id) trail = trail.concat([current])
        current = id
        var p = pageItem(id)
        if (p && arg !== undefined) p.arg = arg
        if (p && p.reset) p.reset()
    }
    function up() { openPage(trail.length ? trail[trail.length - 1] : sections[railIndex].id) }
    // Come to a page from outside (a tile of the quick panel, a Home card): its section is the one on the rail and the page has
    // the focus. An id this Settings has no page for leaves it where it was.
    function goTo(id) {
        var known = false
        for (var k = 0; k < pages.length; k++) if (pages[k].id === id) known = true
        if (!known) return
        var def = null
        for (var d = 0; d < pages.length; d++) if (pages[d].id === id) def = pages[d]
        var parentId = def && def.parent ? def.parent : id === "receiver" ? "sound" : id === "appkey" ? "remote" : id === "homeassistant" || id === "widgets" ? "look" : id
        for (var i = 0; i < sections.length; i++) if (sections[i].id === parentId) { railIndex = i; break }
        current = id
        zone = "page"
        var p = pageItem(id)
        if (p && p.reset) p.reset()
    }
    function patch(p) { themePatch(p); themePersist(p) }

    // ---- a question over the page (docs/mocks/06-confirm-dialog): panels/Confirm.qml, built when the first question is asked, on the
    // opaque overlay surface over the whole window. `confirm(spec, then)` shows it and `then(id)` gets the id of the answer (the first
    // answer is the safe one and Back means it); the keys go to it while it is up. Reset, a software update, the lock PIN and deleting
    // personal data are asked here and done at the remote only.
    property var confirmThen: null
    readonly property bool confirmOpen: confirmLoader.active && confirmLoader.item !== null
    property int confirmAsked: 0
    property bool confirmKept: false
    function confirm(spec, then) {
        confirmThen = then
        confirmAsked++
        confirmLoader.active = true
        if (confirmLoader.item) confirmLoader.item.ask(spec)
    }
    Loader {
        id: confirmLoader
        anchors.fill: parent
        z: 6
        active: false
        source: "panels/Confirm.qml"
        Connections {
            target: confirmLoader.item
            ignoreUnknownSignals: true
            onAnswered: { var t = settings.confirmThen; settings.confirmThen = null; var n = settings.confirmAsked; if (t) t(id); settings.confirmKept = settings.confirmAsked !== n }
            onCloseRequested: { if (settings.confirmKept) settings.confirmKept = false; else confirmClose.restart() }       // the dialog is still inside its own key() here: it goes when that has returned; one asked for by its own answer stays
        }
        Timer { id: confirmClose; interval: 1; onTriggered: confirmLoader.active = false }
    }

    // ---- text typed at the remote (a Wi-Fi password, the TV's name, an address): panels/Search.qml in its entry mode, the keyboard of Home's
    // rename, built when the first text is asked for and over everything. `entry({ prompt, text, secret }, then)` opens it and `then(text)`
    // gets what was typed when Done is pressed (Back leaves without it); `secret` shows bullets, the last character apart. The keys go to it
    // while it is up. Nothing typed here is kept: the page that asked sends it to the service and drops it.
    property var entryThen: null
    readonly property bool entryOpen: entryLoader.active && entryLoader.item !== null
    readonly property var entryItem: entryLoader.item                // the keyboard while it is up (a test types into it)
    property int entryAsked: 0
    property bool entryKept: false
    function entry(spec, then) {
        entryThen = then
        entryAsked++
        entryLoader.active = true
        var s = {}
        for (var k in spec) s[k] = spec[k]
        s.mode = "entry"
        if (entryLoader.item) entryLoader.item.open(s)
    }
    Loader {
        id: entryLoader
        anchors.fill: parent
        z: 7
        active: false
        source: "panels/Search.qml"
        Connections {
            target: entryLoader.item
            ignoreUnknownSignals: true
            onEntered: { var t = settings.entryThen; settings.entryThen = null; var n = settings.entryAsked; if (t) t(text); settings.entryKept = settings.entryAsked !== n }
            onCloseRequested: { if (settings.entryKept) settings.entryKept = false; else entryClose.restart() }       // a keyboard asked for by its own answer stays
        }
        Timer { id: entryClose; interval: 1; onTriggered: entryLoader.active = false }
    }

    // ---- help (kit/LuInfo, kit/LuHelp): the one explanation behind a row's info mark. The page that has the focus offers it for
    // its focused row (`helpReady()` and `openHelp()` on the page); the host calls `hold()` for OK held or the Menu key, and any
    // key closes the sheet before anything else sees it (Back closes the top layer first). Focus never leaves the row.
    readonly property bool helpOpen: helpSheet.shown
    readonly property string helpText: helpSheet.shown ? helpSheet.title + "\n" + helpSheet.text : ""
    function showHelp(title, text, item) {
        var r = item.mapToItem(settings, 0, 0)
        helpSheet.show(title, text, Qt.rect(r.x, r.y, item.width, item.height), Qt.rect(pane.x, pane.y, pane.width, pane.height))
    }
    function hideHelp() { helpSheet.hide() }
    function holdReady() {
        if (zone !== "page" || helpSheet.shown || confirmOpen || entryOpen) return false
        var p = pageItem(current)
        return !!(p && p.helpReady && p.helpReady())
    }
    function hold() {
        if (!holdReady()) return
        pageItem(current).openHelp()
    }

    onRailIndexChanged: { rested = false; restTimer.restart(); hideHelp(); trail = []; if (railIndex < sections.length) current = sections[railIndex].id }
    onZoneChanged: { rested = false; restTimer.restart(); hideHelp() }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: settings.rested = true }
    onVisibleChanged: {
        if (!visible) { confirmLoader.active = false; confirmThen = null; entryLoader.active = false; entryThen = null; return }
        zone = "rail"
        trail = []
        clock = new Date()
        if (railIndex < sections.length) current = sections[railIndex].id
    }

    function key(dir) {
        if (helpSheet.shown) { helpSheet.hide(); return true }
        if (entryOpen) { entryLoader.item.key(dir); return true }
        if (confirmOpen) { confirmLoader.item.key(dir); return true }
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
        if (dir === "back" && current !== sections[railIndex].id) { up(); return true }   // a child page returns to the page it was opened from
        if (dir === "left" || dir === "back") zone = "rail"
        return true
    }

    // The wallpaper dims a step behind the panes.
    Rectangle { anchors.fill: parent; color: Lu.scrim }

    // ---- bar
    LuPane {
        id: bar
        x: 64; y: 56; width: 1792; height: Lu.target        // the 40 px title with its descenders needs a target's height
        cardRadius: Lu.radius("sheet") / 2
        LuIcon { x: Lu.space(5); anchors.verticalCenter: parent.verticalCenter; name: "chevron-left"; size: 28; color: Lu.ink2 }
        LuText { x: Lu.space(5) + 40; anchors.verticalCenter: parent.verticalCenter; role: "title"; text: "Settings" }
        MouseArea { x: 0; width: 360; height: parent.height; onClicked: settings.key("back") }      // the pointer's way back, as the Back key
        Row {
            anchors.verticalCenter: parent.verticalCenter
            x: parent.width - width - Lu.space(6)
            spacing: Lu.space(5)
            // what is true now: the cable, the Wi-Fi (quiet when there is no link), the pointer only while it is on
            LuIcon { anchors.verticalCenter: parent.verticalCenter; name: Net.wired ? "ethernet-port" : "wifi"; size: 28; color: Net.wired || Net.wifi ? Lu.ink2 : Lu.ink3 }
            LuIcon { anchors.verticalCenter: parent.verticalCenter; visible: String(Sio.get("general.remotePointer")) === "on"; name: "mouse-pointer-2"; size: 28; color: Lu.ink2 }
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
                    height: Math.max(Lu.row, implicitHeight - Lu.space(2))      // a row with a summary keeps a little less air than a page row, so ten fit the rail; more text grows it
                    icon: modelData.icon
                    label: modelData.title
                    detail: settings.summaryFor(modelData.id)
                    tileAccent: index === settings.railIndex
                    focused: settings.zone === "rail" && settings.railIndex === index
                    selected: settings.railIndex === index
                    rested: settings.rested
                    onActivated: { settings.railIndex = index; settings.zone = "page"; var p = settings.pageItem(settings.current); if (p && p.reset) p.reset() }
                    onPointerFocus: { settings.railIndex = index; settings.zone = "rail" }
                }
            }
            LuRow {
                width: parent.width
                height: Math.max(Lu.row, implicitHeight - Lu.space(2))
                icon: "square-arrow-out-up-right"
                label: "More in LG settings"
                // label only: nothing here is a state
                focused: settings.zone === "rail" && settings.lgRow
                rested: settings.rested
                onActivated: { settings.launchApp("com.palm.app.settings", {}); settings.closeRequested() }
                onPointerFocus: { settings.railIndex = settings.sections.length; settings.zone = "rail" }
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

    // ---- the help sheet: above both panes, so no row's words read through it
    LuHelp { id: helpSheet; z: 5 }
}
