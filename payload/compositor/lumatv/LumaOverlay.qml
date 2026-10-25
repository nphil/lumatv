import QtQuick 2.4
import WebOSServices 1.0
import WebOS.Global 1.0
import TextFile 1.0
import "LumaBus.js" as Bus
import "kit"

// What Home draws over an app: the banners that say what is on the TV (an input picked, a channel, no signal) and the notification
// drawer a held gear key opens. The fullscreen container (StarfishFullscreenContainer.qml) loads it beside LumaHome, above the app's surface
// and below LG's own views (toasts, alerts, the volume, Quick Settings), and tells it `appFront`: an app, not Home, is on screen. Nothing
// here takes focus from the app and nothing draws until something is shown; the key filter hands it keys only while it has a surface that
// wants them (a banner with buttons, the drawer) and the app has the keys, so a remote that reaches an app reaches it untouched.
//
// Why a layer of ours and not a system-UI window of LG's (docs/research/shell-host-result.md): LG's compositor ties every system window to
// its own state machine. An overlay window is closed by the app launches and switches it follows, found by the key filters through
// `overlayView.activeFocus` and `.visible`, published in `getForegroundAppInfo` (which LG's Qt apps read) and registered with the number-key
// quick access; a layer of ours inside the container is none of that and LG never learns of it.
//
// Over video the glass has no wallpaper to sample (PLAN: tinted glass, no blur): the panes sample one flat colour of the material's own,
// `Lu.backdrop` for as long as an app is in front.
//
//   banners   `showBanner(spec)` (panels/Banner.qml's specs); the input pill on its own when the owner has turned `banners.own` on
//   drawer    `gearHold()`, the gear key held (theme `remote.gearHold`, on unless the owner turned it off)
Item {
    id: ov
    anchors.fill: parent
    visible: up

    // set by the container: an app, not Home, is this container's surface and is on screen
    property bool appFront: false
    readonly property var theme: Lu.theme || ({})
    // LG's own banners draw the same moments (restyled through the overlay mount, tools/inputcommon.sh), so a second set is the owner's choice
    readonly property bool bannersOn: !!(theme.banners && theme.banners.own === true)
    readonly property bool gearHoldOn: !theme.remote || theme.remote.gearHold !== false

    property string surface: ""                           // the one that has the keys: "drawer", "banner" (one with buttons) or "" for none
    readonly property bool bannerUp: !!bannerRes.item && bannerRes.item.kind !== ""
    readonly property bool up: appFront && (surface !== "" || bannerUp)
    readonly property bool wants: appFront && surface !== ""
    // Built a while after the compositor started, never in a frame someone waits for (Layers.qml builds Home's in the same spirit).
    property bool warm: false
    Timer { interval: 8000; running: true; onTriggered: ov.warm = true }

    // ---- the flat colour the glass samples
    readonly property color baseColor: {
        var g = Lu.material && Lu.material.glass ? Qt.lighter(Lu.material.glass, 1.0) : Qt.rgba(0.08, 0.07, 0.11, 1)
        return Qt.rgba(g.r, g.g, g.b, 1)
    }
    Rectangle { id: flatSrc; width: 8; height: 8; color: ov.baseColor; visible: false }
    ShaderEffectSource {
        id: flatTex
        sourceItem: flatSrc
        width: 8; height: 8
        live: false
        visible: false
    }
    onBaseColorChanged: flatTex.scheduleUpdate()
    // An assignment on the way in, as Home's is on the way back (LumaHome.onHostActiveChanged): a Binding that switches off behaves differently in Qt 5.12 and 5.15
    readonly property bool flatGlass: Lu.backdrop === flatTex       // the glass samples the flat colour, not Home's wallpaper blur

    // ---- the surfaces, built once and kept (Resident.qml)
    Resident {
        id: bannerRes
        name: "overlay banner"
        source: "panels/Banner.qml"
        anchors.fill: parent
        z: 2
        build: ov.warm
        shown: ov.warm
    }
    Resident {
        id: drawerRes
        name: "overlay drawer"
        source: "panels/Drawer.qml"
        anchors.fill: parent
        z: 1
        build: ov.warm
        shown: ov.surface === "drawer"
    }
    Connections {
        target: bannerRes.item
        ignoreUnknownSignals: true
        onPicked: ov.bannerPicked(id)
        onCloseRequested: { if (ov.surface === "banner") ov.surface = ""; idle.stop() }
    }
    Connections {
        target: drawerRes.item
        ignoreUnknownSignals: true
        onCloseRequested: ov.closeDrawer()
    }

    // A surface that wants the keys and gets none for this long closes: a remote cannot stay behind it for ever.
    Timer { id: idle; interval: 45000; onTriggered: ov.closeAll() }

    function closeAll() {
        var was = surface
        surface = ""
        idle.stop()
        if (bannerRes.item) bannerRes.item.hide()
        return was !== ""
    }
    onAppFrontChanged: { if (appFront) { Lu.backdrop = flatTex; tryPending() } else { pending = null; closeAll() } }

    // ---- banners
    function showBanner(spec) {
        if (!appFront || !bannerRes.item || !spec || !spec.kind) return false
        if (surface === "drawer") return false           // the drawer is what the person asked for
        bannerRes.item.show(spec)
        if (bannerRes.item.interactive) { surface = "banner"; idle.restart() }
        else surface = ""
        return true
    }
    function hideBanner() { if (bannerRes.item) bannerRes.item.hide() }
    // What the buttons of a banner do over an app: the picker and Quick Settings are LG's system windows, which open over anything.
    function bannerPicked(id) {
        closeAll()
        var sysui = "luna://com.webos.service.systemUIManager"
        if (id === "inputs") LS.adhoc.call(sysui, "/launch", JSON.stringify({ appId: "com.webos.app.quickinputpicker" }))
        else if (id === "picture") LS.adhoc.call(sysui, "/launch", JSON.stringify({ appId: "com.webos.app.quicksettings" }))
        else if (id === "home") LS.adhoc.call("luna://com.webos.applicationManager", "/launchDefaultApp", JSON.stringify({ category: "home" }))
    }

    // The input pill when an input comes to the front. SAM says which app is in front a little before its window maps, so a pill that
    // is asked for early waits (up to 6 s) for the app to be on screen.
    readonly property var inputApp: /^com\.webos\.app\.(hdmi\d|livetv|dp\d|usbc\d)$/
    property var pending: null
    property double pendingAt: 0
    function inputSpec(appId) {
        if (appId === "com.webos.app.livetv") return { kind: "input", icon: "antenna", name: "Live TV", facts: "Antenna", holdMs: 4000 }
        var cur = null
        for (var i = 0; i < Inputs.list.length; i++) if (Inputs.list[i].appId === appId) cur = Inputs.list[i]
        var port = Inputs.portName(appId)
        var label = cur && cur.connected ? cur.label : ""
        return { kind: "input", icon: cur ? cur.icon : "cable", name: label && label !== port ? port + " \u00b7 " + label : port,
                 facts: label && Inputs.isReceiver(label) ? "eARC" : "", holdMs: 4000 }
    }
    function inputSwitched(appId) {
        if (!bannersOn || !inputApp.test(String(appId))) return
        pending = inputSpec(String(appId))
        pendingAt = Date.now()
        tryPending()
    }
    function tryPending() {
        if (!pending || !appFront || !bannersOn) return
        var spec = pending
        if (Date.now() - pendingAt > 6000) { pending = null; return }
        if (showBanner(spec)) pending = null
    }
    Connections { target: Inputs; onForegroundChanged: ov.inputSwitched(Inputs.foreground) }

    // ---- the drawer
    function openDrawer() {
        if (!appFront || !drawerRes.item) return false
        if (bannerRes.item) bannerRes.item.hide()
        surface = "drawer"
        drawerRes.item.open()
        idle.restart()
        return true
    }
    function closeDrawer() { if (surface === "drawer") { surface = ""; idle.stop() } }
    // the gear key held (the key filter times the hold; LumaBus.js asks): the drawer opens, or closes if it was up
    function gearReady() { return appFront && gearHoldOn && !!drawerRes.item }
    function gearHold() {
        if (!gearReady()) return false
        if (surface === "drawer") { closeDrawer(); return true }
        return openDrawer()
    }

    // ---- keys: the D-pad goes to the surface that wants them. A press acts (there is no hold here); the release of a press taken is
    // the filter's to pair, whatever this answers to it.
    function directionFor(k) {
        if (k === Qt.Key_Left) return "left"
        if (k === Qt.Key_Right) return "right"
        if (k === Qt.Key_Up) return "up"
        if (k === Qt.Key_Down) return "down"
        if (k === Qt.Key_Return || k === Qt.Key_Enter || k === Qt.Key_Select) return "ok"
        if (k === Qt.Key_Back || k === WebOS.Key_webOS_Back) return "back"
        return ""
    }
    function key(k, pressed, autoRepeat, deviceId) {
        if (!wants) return false
        var dir = directionFor(k)
        if (dir === "") return false
        if (!pressed) return true
        if (autoRepeat && (dir === "back" || dir === "ok")) return true
        idle.restart()
        if (surface === "drawer" && drawerRes.item) drawerRes.item.key(dir)
        else if (surface === "banner" && bannerRes.item) bannerRes.item.key(dir)
        return true
    }

    // ---- a door for the gate and the TV steps: a JSON spec written to /tmp/{{shortId}}-overlay is shown (a banner's, or {"open":"drawer"});
    // the same text again is not shown twice. The file is in /tmp, so a reboot clears it.
    TextFile { id: doorFile; name: "file:///tmp/{{shortId}}-overlay" }
    property string lastDoor: ""
    function readDoor() {
        var text = (doorFile.exists ? doorFile.read() : "").replace(/^\s+|\s+$/g, "")
        if (text === lastDoor) return
        lastDoor = text
        if (text === "") return
        var d = null
        try { d = JSON.parse(text) } catch (e) { console.warn("[LumaOverlay] the door holds no JSON: " + text.substring(0, 80)); return }
        if (d && d.open === "drawer") openDrawer()
        else if (d && d.kind) showBanner(d)
    }
    Timer { interval: 1000; running: ov.appFront; repeat: true; onTriggered: ov.readDoor() }

    Component.onCompleted: {
        Bus.setOverlay({ wants: function() { return ov.wants }, key: ov.key, ready: ov.gearReady, hold: ov.gearHold })
        console.info("[LumaOverlay] loaded")
    }
    Component.onDestruction: Bus.setOverlay(null)
}
