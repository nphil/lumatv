import QtQuick 2.4
import WebOSServices 1.0
import "kit"

// The surfaces Home keeps built: Settings, the kit gallery, the input picker, the quick panel, the volume OSD
// and, as they are added, the panels that open over Home. LumaHome owns when each is shown (`open`, its key
// dispatch and the focus it gives back); this file owns that each exists before it is asked for. After Home has
// presented, each is built in the background, one after the other, so that showing one is a `visible` flip on a
// tree that is already laid out, and the startup warm-up (Warmup.qml) starts only when they are all built, so it
// finds their text styles in Home's item tree.
//
// A surface here is an Item with `key(dir)` (up, down, left, right, ok, back; returns nothing, it owns the keys
// while it is shown), `built` when it builds parts of itself, and signals the host connects to below. It never
// writes a file or calls a service of its own: it asks through those signals (the quick panel and the input
// picker are the exceptions that the TV's own services are the point of: they call luna through LS).
Item {
    id: layers
    anchors.fill: parent
    property Item home                                  // LumaHome
    readonly property bool hostActive: home ? home.hostActive : false
    property string open: ""                            // the modal surface that owns the keys ("" none)
    property int buildStep: -1                          // -1: waiting for Home to present
    readonly property int steps: 8                      // surfaces in the build chain
    readonly property bool built: buildStep >= steps
    signal closeRequested(string id)
    signal openSettingsPage(string id)

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

    function find(id) {
        return id === "settings" ? settingsRes : id === "gallery" ? galleryRes : id === "picker" ? pickerRes : id === "quick" ? quickRes
             : id === "menu" ? menuRes : id === "confirm" ? confirmRes : id === "remote" ? remoteRes : null
    }
    function key(dir) {
        var r = find(open)
        if (r && r.item) r.item.key(dir)
        return open !== ""
    }
    // The input key pressed again while the picker is up goes on to the next input.
    // Open Settings at a page (the quick panel's tiles do): the page's rail section is selected and the page has the focus.
    function settingsGoTo(id) { if (settingsRes.item && id !== "") settingsRes.item.goTo(id) }
    function pickerNext() { if (pickerRes.item) pickerRes.item.next() }
    // A volume key was pressed: the OSD asks the helper for a fresh reading and shows the pill.
    function volumeKey() { if (osdRes.item) osdRes.item.nudge() }
    onOpenChanged: { if (open === "picker" && pickerRes.item) pickerRes.item.open({}) }

    // What the host asks of the small panels. They are built, so each is a call on an item that is already there.
    function openMenu(spec) { if (menuRes.item) menuRes.item.open(spec) }
    function askConfirm(spec) { if (confirmRes.item) confirmRes.item.ask(spec) }
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
        onLaunchApp: LS.adhoc.call("luna://com.webos.applicationManager", "/launch", JSON.stringify({ id: appId }))
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
        shown: layers.open === "picker"
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
        shown: layers.open === "quick"
        onDone: layers.advance(step)
    }
    Connections {
        target: quickRes.item
        ignoreUnknownSignals: true
        onOpenSettingsPage: { layers.closeRequested("quick"); layers.openSettingsPage(id) }
        onCloseRequested: layers.closeRequested("quick")
    }

    // ---- the volume and mute OSD (step 3): always up, draws only when the receiver says something
    Resident {
        id: osdRes
        name: "osd"
        step: 3
        source: "panels/VolumeOsd.qml"
        anchors.fill: parent
        z: 80
        build: layers.buildStep >= step
        shown: layers.hostActive
        onDone: layers.advance(step)
    }

    // ---- the context menu, the confirm dialog and the on-screen remote (steps 4 to 6)
    Resident {
        id: menuRes
        name: "menu"
        step: 4
        source: "panels/ContextMenu.qml"
        anchors.fill: parent
        z: 64
        build: layers.buildStep >= step
        shown: layers.open === "menu"
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
        step: 5
        source: "panels/Confirm.qml"
        anchors.fill: parent
        z: 65
        build: layers.buildStep >= step
        shown: layers.open === "confirm"
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
        step: 6
        source: "panels/ScreenRemote.qml"
        anchors.fill: parent
        z: 66
        build: layers.buildStep >= step
        shown: layers.open === "remote"
        onDone: layers.advance(step)
    }
    Connections {
        target: remoteRes.item
        ignoreUnknownSignals: true
        onPressed: layers.remoteKey(name)
        onCloseRequested: layers.closeRequested("remote")
    }
    signal remoteKey(string name)

    // ---- the launch splash, the spinner and the hold ring (step 7): above everything but the AI prompt
    Resident {
        id: splashRes
        name: "splash"
        step: 7
        source: "panels/Splash.qml"
        anchors.fill: parent
        z: 75
        build: layers.buildStep >= step
        shown: layers.hostActive
        onDone: layers.advance(step)
    }

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
