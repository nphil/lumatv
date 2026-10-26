import QtQuick 2.4
import WebOSServices 1.0
import "kit"

// The surfaces Home keeps built: Settings, the kit gallery, the input picker, the quick panel, the
// small panels (menu, confirm, remote, splash), the library, search, voice bar and banner, the notification drawer and the
// screensaver. LumaHome owns when each is shown (`open`, its key dispatch and the focus it gives back); this file owns that
// each exists before it is asked for. After Home has presented, each is built in the background, one after the other, so
// that showing one is a `visible` flip on a tree that is already laid out, and the startup warm-up (Warmup.qml) starts only
// when they are all built, so it finds their text styles in Home's item tree.
//
// A surface here is an Item with `key(dir)` (up, down, left, right, ok, back; returns nothing, it owns the keys
// while it is shown), `built` when it builds parts of itself, and signals the host connects to below. It never
// writes a file or calls a service of its own: it asks through those signals (the quick panel and the input
// picker are the exceptions that the TV's own services are the point of: they call luna through LS).
//
// One surface has the keys, `open`; the ones under it (a menu over the library, a keyboard over the library) are `below`:
// they stay drawn and take no key, and when the top one closes the one under it has the keys again.
Item {
    id: layers
    anchors.fill: parent
    property Item home                                  // LumaHome
    readonly property bool hostActive: home ? home.hostActive : false
    property string open: ""                            // the modal surface that owns the keys ("" none)
    property var below: []                              // the modal surfaces under it, drawn and without keys
    property int buildStep: -1                          // -1: waiting for Home to present
    readonly property int steps: 13                     // surfaces in the build chain
    readonly property bool built: buildStep >= steps
    signal closeRequested(string id)
    signal openSettingsPage(string id)
    function up(id) { return open === id || below.indexOf(id) >= 0 }

    // Home has presented a second ago: start the chain. A built chain is never restarted.
    Timer {
        interval: 1000
        running: layers.hostActive && layers.buildStep < 0
        onTriggered: layers.buildStep = 0
    }
    // A surface that cannot be built must not hold the warm-up for ever.
    Timer {
        interval: 20000
        running: layers.buildStep >= 0 && !layers.built
        onTriggered: layers.buildStep = layers.steps
    }
    function advance(step) { if (buildStep <= step) buildStep = step + 1 }

    // The modal surfaces, by the name Home opens them with (the splash and the mic indicator are always up and never open).
    function find(id) {
        return id === "settings" ? settingsRes : id === "gallery" ? galleryRes : id === "picker" ? pickerRes : id === "quick" ? quickRes
             : id === "menu" ? menuRes : id === "confirm" ? confirmRes : id === "remote" ? remoteRes
             : id === "apps" ? appsRes : id === "search" ? searchRes : id === "voice" ? voiceRes : id === "banner" ? bannerRes
             : id === "drawer" ? drawerRes : null
    }
    function key(dir) {
        var r = find(open)
        if (r && r.item) r.item.key(dir)
        return open !== ""
    }
    // The long press of OK or the Menu key on the surface that has the keys: the library opens the options of its icon.
    function hold() {
        var r = find(open)
        if (r && r.item && r.item.hold) r.item.hold()
    }
    // Whether a held OK means something on the surface that has the keys now: Settings says yes on a row that has help.
    // The input key pressed again while the picker is up goes on to the next input.
    // Open Settings at a page (the quick panel's tiles do): the page's rail section is selected and the page has the focus.
    function settingsGoTo(id) { if (settingsRes.item && id !== "") settingsRes.item.goTo(id) }
    // Settings' pages as the search lists them: [{ id, title, sub, icon }], each with its own one-line summary.
    function settingsPages() {
        var s = settingsRes.item
        if (!s) return []
        return s.pages.filter(function(p) { return p.search !== false }).map(function(p) { var sum = String(s.summaryFor(p.id)).split(" \u00b7 ")[0]; return { id: p.id, title: p.title, icon: p.icon, sub: "Settings" + (sum ? " \u00b7 " + sum : "") } })
    }
    function pickerNext() { if (pickerRes.item) pickerRes.item.next() }
    onOpenChanged: { if (open === "picker" && pickerRes.item) pickerRes.item.open({}); else if (open === "drawer" && drawerRes.item) drawerRes.item.open() }

    // What the host asks of the small panels. They are built, so each is a call on an item that is already there.
    function openMenu(spec) { if (menuRes.item) menuRes.item.open(spec) }
    function askConfirm(spec) { if (confirmRes.item) confirmRes.item.ask(spec) }
    function appsReset() { if (appsRes.item) appsRes.item.open() }
    function searchOpen(spec) { if (searchRes.item) searchRes.item.open(spec) }
    function voiceListen() { if (voiceRes.item) voiceRes.item.listen() }
    function voiceIdle() { if (voiceRes.item) voiceRes.item.idle() }
    function voiceWorking() { if (voiceRes.item) voiceRes.item.working() }
    function voiceFailed(words) { if (voiceRes.item) voiceRes.item.failed(words) }
    function voiceMic(on) { if (voiceRes.item) voiceRes.item.micOpen = on }
    function bannerShow(spec) { if (bannerRes.item) bannerRes.item.show(spec) }
    function bannerHide() { if (bannerRes.item) bannerRes.item.hide() }
    function launchSplash(name, icon) {
        if (!splashRes.item) return
        splashRes.item.launch(name, icon)
        splashEnd.restart()
    }
    function spinner(text) { if (splashRes.item) splashRes.item.spin(text) }
    function holdRing(rect, name, fraction) { if (splashRes.item) { if (fraction < 0) splashRes.item.done(); else { splashRes.item.hold(rect, name); splashRes.item.fraction = fraction } } }
    function endSplash() { if (splashRes.item) splashRes.item.done(); splashEnd.stop() }
    // Home going away is the app arriving; if it never does, the splash gives up after a while rather than cover Home for ever.
    Timer { id: splashEnd; interval: 8000; onTriggered: layers.endSplash() }
    onHostActiveChanged: if (!hostActive) endSplash()

    // ---- Settings (step 0)
    Resident {
        id: settingsRes
        name: "settings"
        step: 0
        source: "Settings.qml"
        anchors.fill: parent
        z: 61
        build: layers.buildStep >= step
        shown: layers.open === "settings"
        onDone: layers.advance(step)
    }
    Connections {
        target: settingsRes.item
        ignoreUnknownSignals: true
        onThemePatch: layers.home.patchTheme(patch)
        onThemePersist: layers.home.persistTheme(patch)
        onLaunchApp: LS.adhoc.call("luna://com.webos.applicationManager", "/launch", JSON.stringify({ id: appId, params: params || {} }))
        onCloseRequested: layers.closeRequested("settings")
    }

    // ---- the input picker (step 1)
    Resident {
        id: pickerRes
        name: "picker"
        step: 1
        source: "panels/InputPicker.qml"
        anchors.fill: parent
        z: 62
        build: layers.buildStep >= step
        shown: layers.up("picker")
        onDone: layers.advance(step)
    }
    Connections {
        target: pickerRes.item
        ignoreUnknownSignals: true
        onActivated: { LS.adhoc.call("luna://com.webos.applicationManager", "/launch", JSON.stringify({ id: appId })); layers.closeRequested("picker") }
        onCloseRequested: layers.closeRequested("picker")
    }

    // ---- the quick panel with the receiver's tile (step 2)
    Resident {
        id: quickRes
        name: "quick"
        step: 2
        source: "panels/QuickPanel.qml"
        anchors.fill: parent
        z: 63
        build: layers.buildStep >= step
        shown: layers.up("quick")
        onDone: layers.advance(step)
    }
    Connections {
        target: quickRes.item
        ignoreUnknownSignals: true
        onOpenSettingsPage: { layers.closeRequested("quick"); layers.openSettingsPage(id) }
        onCloseRequested: layers.closeRequested("quick")
    }

    // ---- the context menu, the confirm dialog and the on-screen remote (steps 3 to 5)
    Resident {
        id: menuRes
        name: "menu"
        step: 3
        source: "panels/ContextMenu.qml"
        anchors.fill: parent
        z: 64
        build: layers.buildStep >= step
        shown: layers.up("menu")
        onDone: layers.advance(step)
    }
    Connections {
        target: menuRes.item
        ignoreUnknownSignals: true
        onPicked: layers.menuPicked(id, index)
        onCloseRequested: layers.closeRequested("menu")
    }
    signal menuPicked(string id, int index)
    Resident {
        id: confirmRes
        name: "confirm"
        step: 4
        source: "panels/Confirm.qml"
        anchors.fill: parent
        z: 65
        build: layers.buildStep >= step
        shown: layers.up("confirm")
        onDone: layers.advance(step)
    }
    Connections {
        target: confirmRes.item
        ignoreUnknownSignals: true
        onAnswered: layers.confirmAnswered(id)
        onCloseRequested: layers.closeRequested("confirm")
    }
    signal confirmAnswered(string id)
    Resident {
        id: remoteRes
        name: "remote"
        step: 5
        source: "panels/ScreenRemote.qml"
        anchors.fill: parent
        z: 66
        build: layers.buildStep >= step
        shown: layers.up("remote")
        onDone: layers.advance(step)
    }
    Connections {
        target: remoteRes.item
        ignoreUnknownSignals: true
        onPressed: layers.remoteKey(name)
        onCloseRequested: layers.closeRequested("remote")
    }
    signal remoteKey(string name)

    // ---- the launch splash, the spinner and the hold ring (step 6): above everything but the AI prompt
    Resident {
        id: splashRes
        name: "splash"
        step: 6
        source: "panels/Splash.qml"
        anchors.fill: parent
        z: 75
        build: layers.buildStep >= step
        shown: layers.hostActive
        onDone: layers.advance(step)
    }

    // ---- the all-apps library (step 7): the Apps tile's own grid, and Search over it (step 8)
    Resident {
        id: appsRes
        name: "apps"
        step: 7
        source: "panels/AppsGrid.qml"
        anchors.fill: parent
        z: 61
        build: layers.buildStep >= step
        shown: layers.up("apps")
        onDone: layers.advance(step)
    }
    signal appLaunched(var app)
    signal appMenu(var app, var anchor)
    signal searchRequested()
    Connections {
        target: appsRes.item
        ignoreUnknownSignals: true
        onLaunched: layers.appLaunched(app)
        onMenuRequested: layers.appMenu(app, anchor)
        onSearchRequested: layers.searchRequested()
        onCloseRequested: layers.closeRequested("apps")
    }
    Resident {
        id: searchRes
        name: "search"
        step: 8
        source: "panels/Search.qml"
        anchors.fill: parent
        z: 62
        build: layers.buildStep >= step
        shown: layers.up("search")
        onDone: layers.advance(step)
    }
    signal searchPicked(string kind, string id, string text)
    signal entered(string text)
    Connections {
        target: searchRes.item
        ignoreUnknownSignals: true
        onPicked: layers.searchPicked(kind, id, text)
        onEntered: layers.entered(text)
        onCloseRequested: layers.closeRequested("search")
    }

    // ---- the voice bar (step 9): above every surface but the AI prompt, and its mic indicator stays up while the mic is open
    Resident {
        id: voiceRes
        name: "voice"
        step: 9
        source: "panels/VoiceBar.qml"
        anchors.fill: parent
        z: 91
        build: layers.buildStep >= step
        shown: layers.up("voice") || (!!item && item.micOpen)
        onDone: layers.advance(step)
    }
    Connections {
        target: voiceRes.item
        ignoreUnknownSignals: true
        onCloseRequested: layers.closeRequested("voice")
    }

    // ---- the banner (step 10): the channel banner, the input pill and the no-signal screen. A banner without buttons takes no key
    // and hides itself, so the resident is always up and draws only what it has been given.
    Resident {
        id: bannerRes
        name: "banner"
        step: 10
        source: "panels/Banner.qml"
        anchors.fill: parent
        z: 72
        build: layers.buildStep >= step
        shown: layers.hostActive
        onDone: layers.advance(step)
    }
    signal bannerPicked(string id)
    Connections {
        target: bannerRes.item
        ignoreUnknownSignals: true
        onPicked: layers.bannerPicked(id)
        onCloseRequested: layers.closeRequested("banner")
    }

    // ---- the notification drawer (step 11): down the right side, from the bell on the status bar. It marks what it shows as seen
    // when it opens, and it owns its keys (Back asks to close).
    Resident {
        id: drawerRes
        name: "drawer"
        step: 11
        source: "panels/Drawer.qml"
        anchors.fill: parent
        z: 67
        build: layers.buildStep >= step
        shown: layers.up("drawer")
        onDone: layers.advance(step)
    }
    Connections {
        target: drawerRes.item
        ignoreUnknownSignals: true
        onCloseRequested: layers.closeRequested("drawer")
    }

    // ---- the screensaver and art mode (step 12): over everything here, under the AI prompt (Home's own, a level above this item),
    // while the TV's power state is Screen Saver. It is not a surface Home opens, so it has no name in find(); Home hands it the
    // key that wakes the TV (saverKey) and the key does nothing else.
    Resident {
        id: saverRes
        name: "saver"
        step: 12
        source: "panels/Screensaver.qml"
        anchors.fill: parent
        z: 95
        build: layers.buildStep >= step
        shown: layers.hostActive && !!item && item.active
        onDone: layers.advance(step)
    }
    readonly property bool saverUp: saverRes.visible
    function saverKey() { if (saverRes.item) saverRes.item.key("") }

    // ---- the kit gallery (development): built the first time the flag asks for it, then kept
    Resident {
        id: galleryRes
        name: "gallery"
        source: "KitGallery.qml"
        anchors.fill: parent
        z: 60
        shown: layers.open === "gallery"
    }
}
