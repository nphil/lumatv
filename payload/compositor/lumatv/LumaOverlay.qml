import QtQuick 2.4
import QtQuick.Window 2.2
import WebOSCompositorBase 1.0
import WebOSServices 1.0
import WebOS.Global 1.0
import TextFile 1.0
import "LumaBus.js" as Bus
import "kit"

// What Home draws over an app: the banners that say what is on the TV (an input picked, a channel, no signal), the notification
// drawer a held gear key opens, and the volume and mute display. The fullscreen container (StarfishFullscreenContainer.qml) loads it beside
// LumaHome, above the app's surface and below LG's own views (toasts, alerts, Quick Settings), and tells it `appFront`: an app, not Home, is
// on screen. Nothing here takes focus from the app and nothing draws until something is shown; the key filter hands it keys only while it has
// a surface that wants them (a banner with buttons, the drawer) and the app has the keys, so a remote that reaches an app reaches it untouched.
// The volume display wants no key at all: it answers the audio service and the receiver, so it is the one surface here that is up over Home
// too (the container keeps this layer shown in every state) and over an app that never lets a volume key through.
//
// Why a layer of ours and not a system-UI window of LG's (docs/research/shell-host-result.md): LG's compositor ties every system window to
// its own state machine. An overlay window is closed by the app launches and switches it follows, found by the key filters through
// `overlayView.activeFocus` and `.visible`, published in `getForegroundAppInfo` (which LG's Qt apps read) and registered with the number-key
// quick access; a layer of ours inside the container is none of that and LG never learns of it.
//
// Over video the glass has no wallpaper to sample (PLAN: tinted glass, no blur): the panes sample one flat colour of the material's own,
// `Lu.backdrop` for as long as an app is in front. A pane that sets `fillAlpha` below 1 (the volume display, at Lu.readingAlpha) lets the
// picture through its fill; the others are opaque.
//
//   banners   `showBanner(spec)` (panels/Banner.qml's specs); the input pill on its own when the owner has turned `banners.own` on
//   drawer    `gearHold("notifications")`, the gear key held where the owner chose the drawer for it (theme `remote.gearHold`)
//   volume    panels/VolumeOsd.qml: up whenever the audio service or the receiver reports a volume or a mute, and while the sound is muted
//   quick     panels/QuickPanel.qml: the gear's tap (the key filter's second entry), a tall panel docked to the right edge; Back slides it back
//   picker    panels/InputPicker.qml: the Input key, a sheet along the bottom; the key again goes to the next input
//   cards     panels/Overlays.qml (theme `overlays.enabled`): the doorbell, an alarm or timer that finished, a Home Assistant alert, a score, in a column
//             in the top right corner with the camera pop-up on top; OK and Back are the top card's (`takes`), every other key the app's
Item {
    id: ov
    anchors.fill: parent
    // `live` is what the compositor is told (the layer is drawn); `overlaysWarming` keeps the layer's tree built and drawn once while the shell warms it
    visible: live || overlaysWarming

    // set by the container: an app, not Home, is this container's surface and is on screen
    property bool appFront: false
    readonly property var theme: Lu.theme || ({})
    // LG's own banners draw the same moments (restyled through the overlay mount, tools/inputcommon.sh), so a second set is the owner's choice
    readonly property bool bannersOn: !!(theme.banners && theme.banners.own === true)
    readonly property string gearMode: !theme.remote || (theme.remote.gearHold !== "notifications" && theme.remote.gearHold !== "nothing") ? "settings" : theme.remote.gearHold

    property string surface: ""                           // the one that has the keys: "drawer", "quick", "picker", "banner" (one with buttons) or "" for none
    readonly property bool bannerUp: !!bannerRes.item && bannerRes.item.kind !== ""
    readonly property bool up: appFront && (surface !== "" || bannerUp)
    // the camera pop-up and viewer (panels/Cameras.qml) are up over Home as over an app: the layer is drawn for them whatever else is
    readonly property bool cameraUp: !!cameraRes.item && cameraRes.item.up
    readonly property var cameraItem: cameraRes.item          // the surface itself, for the gate and the host's tests
    readonly property var overlaysItem: overlaysRes.item     // the overlay system's surface, likewise
    readonly property bool cameraWants: !!cameraRes.item && cameraRes.item.wants
    readonly property bool viewerUp: !!cameraRes.item && cameraRes.item.viewerUp
    readonly property bool popupUp: !!cameraRes.item && cameraRes.item.popupUp
    readonly property bool live: up || osdLive || panelLeaving || panelHeld || cameraUp || overlaysLive || switcherUp || switcherLeaving
    // The cards (the camera's pop-up, the overlay system's) take OK and Back only, and only once the top one has been up a moment (theme
    // `overlays.armMs`): a press made as it arrived was meant for the app. Everything else is the app's. They are not a surface that takes the
    // keyboard focus: under LumaTV's compositor the layer asks for the two keys by name (`takes`) and the app keeps the focus and its arrows.
    readonly property bool cardsUp: popupUp || overlaysUp
    property bool cameraArmed: false
    readonly property bool cardsArmed: popupUp ? (!overlaysOn || cameraArmed) : (overlaysUp && !!overlaysRes.item && overlaysRes.item.armed)
    Timer { id: cameraArm; interval: !!overlaysRes.item ? overlaysRes.item.cfg.armMs : 600; onTriggered: ov.cameraArmed = true }
    onPopupUpChanged: { cameraArmed = false; if (popupUp) cameraArm.restart(); else cameraArm.stop() }
    // what takes every key (a panel, a banner with buttons, the camera viewer, the app switcher over Home or an app): the layer wants the keyboard focus
    readonly property bool wantsFocus: switcherUp || appFront && (surface !== "" || cameraWants)
    // and the question LG's key filter asks, which is answered key by key (`key` returns false for what it passes on)
    readonly property bool wants: switcherUp || appFront && (surface !== "" || cameraWants || cardsUp)
    readonly property var takes: !switcherUp && appFront && surface === "" && !cameraWants && cardsUp && cardsArmed ? ["ok", "back"] : []
    // the overlay system
    // the kinds the Rust shell answers (Shell.qml, from the compositor): its cards and its camera pop-up are drawn by it, and not again here
    property var rustKinds: []
    readonly property bool rustCards: rustKinds.indexOf("cards") >= 0
    readonly property bool rustPopup: rustKinds.indexOf("popup") >= 0
    readonly property bool rustOsd: rustKinds.indexOf("osd") >= 0                  // the volume bar and the mute mark are the Rust kind's: this one draws neither
    readonly property bool rustBanner: rustKinds.indexOf("banner") >= 0          // its input pill, the one with buttons and the no-signal screen are the Rust kind's
    readonly property bool overlaysOn: !!theme.overlays && theme.overlays.enabled === true && !rustCards
    readonly property bool overlaysLive: !!overlaysRes.item && overlaysRes.item.live
    readonly property bool overlaysUp: !!overlaysRes.item && overlaysRes.item.up
    property bool warming: false                              // set by the shell: its layer is being warmed (a card is drawn once with letters, nothing is shown)
    readonly property bool overlaysWarming: !!overlaysRes.item && warming && overlaysOn
    // Do-not-disturb: nothing shows (and what is up goes) while a panel, a PIN, an alert, the keyboard, the saver, the camera's viewer or the app switcher has the screen
    property bool systemBusy: false                           // set by the shell: the notifications' alert, input popup or PIN prompt, the keyboard, the saver
    readonly property bool dnd: surface !== "" || homeBusy || systemBusy || viewerUp || panelLeaving || switcherUp
    // the volume display has something on screen or is fading: this layer is drawn for it whatever else is up
    readonly property bool osdLive: !rustOsd && !!osdRes.item && osdRes.item.live
    // a panel that has been told to go is still sliding away: this layer stays drawn until it has gone
    readonly property bool panelLeaving: (!!quickRes.item && quickRes.item.leaving === true) || (!!pickerRes.item && pickerRes.item.leaving === true)
    // and one that has gone is parked in the scene for a few seconds, off the screen (panels/PanelMotion.qml): this layer stays in the scene too
    readonly property bool panelHeld: (!!quickRes.item && quickRes.item.held === true) || (!!pickerRes.item && pickerRes.item.held === true)
    // theme `remote.quickPanel` and `remote.inputPicker`: on unless the owner turned them off (the gear's tap and the Input key are then LG's)
    readonly property bool quickOn: !theme.remote || theme.remote.quickPanel !== false
    readonly property bool pickerOn: !theme.remote || theme.remote.inputPicker !== false
    // The app switcher (panels/Switcher.qml): over Home as over an app, so none of its flags wait for `appFront`. Under LumaTV's compositor the compositor
    // holds whether it is open (`switcherFeed`, which Shell.qml binds from its topics, and `switcherPost` takes the requests back); under LG's there
    // is none, and the surface is open when Back on Home or the door below says so.
    property var switcherFeed: null
    readonly property bool switcherOwn: switcherFeed !== null
    property bool switcherLocal: false
    readonly property bool switcherUp: switcherOwn ? switcherFeed.open === true : switcherLocal
    readonly property bool switcherLeaving: !!switcherRes.item && switcherRes.item.leaving === true
    // The second press of Home is the switch `remote.appSwitcher` alone; Back on Home (`remote.recents`) opens the same surface, so either switch builds and lists it.
    readonly property bool switcherPress: !!theme.remote && theme.remote.appSwitcher === true
    readonly property bool switcherOn: switcherPress || (!!theme.remote && theme.remote.recents === true)
    readonly property var switcherItem: switcherRes.item      // the surface itself, for the host's tests
    signal switcherPost(var op)
    // Built a while after the compositor started, never in a frame someone waits for (Layers.qml builds Home's in the same spirit).
    property bool warm: false
    Timer { interval: 8000; running: true; onTriggered: ov.warm = true }
    // the two larger panels are built a little after the others, one at a time, so no frame carries more than one build
    property bool warmQuick: false
    property bool warmPicker: false
    property bool warmSwitcher: false
    Timer { interval: 9500; running: true; onTriggered: ov.warmQuick = true }
    Timer { interval: 11000; running: true; onTriggered: ov.warmPicker = true }
    Timer { interval: 12500; running: true; onTriggered: ov.warmSwitcher = true }

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
    // Under LumaTV's compositor this layer is a window of its own (compositor/shell/Shell.qml): its panes sample the flat colour through
    // Lu.flatBackdrop (kit/LuPane.qml) and Home's blur stays Home's, since a texture cannot be sampled across windows.
    readonly property bool ownWindow: !!Lu.homeWindow && Window.window !== Lu.homeWindow
    readonly property bool flatGlass: ownWindow || Lu.backdrop === flatTex       // the glass samples the flat colour, not Home's wallpaper blur

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
    Resident {
        id: quickRes
        name: "overlay quick panel"
        source: "panels/QuickPanel.qml"
        anchors.fill: parent
        z: 1
        build: ov.warmQuick
        shown: ov.surface === "quick"
    }
    Resident {
        id: pickerRes
        name: "overlay input picker"
        source: "panels/InputPicker.qml"
        anchors.fill: parent
        z: 1
        build: ov.warmPicker
        shown: ov.surface === "picker"
    }
    // The app switcher: a surface of its own, above the panels and below the volume display and the cameras' pop-up.
    Resident {
        id: switcherRes
        name: "overlay switcher"
        source: "panels/Switcher.qml"
        anchors.fill: parent
        z: 2.2
        build: ov.warmSwitcher
        shown: ov.switcherUp
    }
    Binding { target: switcherRes.item; property: "feed"; value: ov.switcherFeed; when: !!switcherRes.item }
    Connections {
        target: switcherRes.item
        ignoreUnknownSignals: true
        onPost: ov.switcherPost(op)
        onCloseRequested: ov.switcherLocal = false
    }
    // The volume and mute display, drawn whenever it has something to say (its `live`): over an app's picture, where its glass is see-through,
    // and over Home, where it is not. Built with the others, so a first volume key never builds or compiles anything.
    Resident {
        id: osdRes
        name: "overlay volume"
        source: "panels/VolumeOsd.qml"
        anchors.fill: parent
        z: 3
        build: ov.warm && !ov.rustOsd
        shown: ov.warm && !ov.rustOsd
    }
    Binding { target: osdRes.item; property: "overVideo"; value: ov.appFront; when: !!osdRes.item }
    // The cameras: the pop-up over whatever plays, the viewer it expands to (panels/Cameras.qml). Above the panels and below the volume display,
    // so a volume key over a camera still shows the bar. A pop-up waits while a panel or a banner with buttons is up here, or Home has a prompt.
    Resident {
        id: cameraRes
        name: "overlay cameras"
        source: "panels/Cameras.qml"
        anchors.fill: parent
        z: 2.5
        build: ov.warm
        shown: ov.warm
    }
    Binding { target: cameraRes.item; property: "appFront"; value: ov.appFront; when: !!cameraRes.item }
    Binding { target: cameraRes.item; property: "popupOff"; value: ov.rustPopup; when: !!cameraRes.item }
    // The cards of the overlay system. Below the camera pop-up, whose card they go under, above the panels and below the volume display.
    Resident {
        id: overlaysRes
        name: "overlay cards"
        source: "panels/Overlays.qml"
        anchors.fill: parent
        z: 2.4
        build: ov.warm && ov.overlaysOn
        shown: ov.warm && ov.overlaysOn
    }
    Binding { target: overlaysRes.item; property: "appFront"; value: ov.appFront; when: !!overlaysRes.item }
    Binding { target: overlaysRes.item; property: "dnd"; value: ov.dnd; when: !!overlaysRes.item }
    Binding { target: overlaysRes.item; property: "cameraUp"; value: ov.popupUp; when: !!overlaysRes.item }
    Binding { target: overlaysRes.item; property: "cameraBottom"; value: !!cameraRes.item ? cameraRes.item.popupBottom : 0; when: !!overlaysRes.item }
    Binding { target: overlaysRes.item; property: "warming"; value: ov.warming; when: !!overlaysRes.item }
    // OK on a card without a button of its own opens the notification drawer, where it can be opened (Home's over Home, this layer's over an app)
    Binding { target: overlaysRes.item; property: "drawerReady"; value: !!overlaysRes.item && overlaysRes.item.pending && Bus.gearReady("notifications"); when: !!overlaysRes.item }
    Connections { target: overlaysRes.item; ignoreUnknownSignals: true; onOpenDrawer: Bus.gearHold("notifications") }
    // the camera pop-up showed a card: a doorbell record of the same ring is not a second one
    Connections { target: cameraRes.item; ignoreUnknownSignals: true; onPopupShown: { if (overlaysRes.item) overlaysRes.item.cameraShown(kind) } }
    Binding { target: cameraRes.item; property: "hostBusy"; value: ov.dnd; when: !!cameraRes.item }
    Binding { target: osdRes.item; property: "avoidTopRight"; value: ov.cardsUp; when: !!osdRes.item }
    // what Home says of itself (a PIN, an alert, a prompt, a confirmation up): read once a second while a pop-up may be waiting, not bound
    property bool homeBusy: false
    Timer { interval: 500; repeat: true; running: (!!cameraRes.item && (cameraRes.item.up || Helper.topics["cameras.popup"] !== null && Helper.topics["cameras.popup"] !== undefined)) || (!!overlaysRes.item && overlaysRes.item.pending); onTriggered: ov.homeBusy = Bus.homeBusy() }
    Connections {
        target: cameraRes.item
        ignoreUnknownSignals: true
        onCloseRequested: idle.stop()
    }
    Binding { target: osdRes.item; property: "still"; value: ov.surface === "quick" || ov.surface === "picker" || ov.panelLeaving; when: !!osdRes.item }
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
    Connections {
        target: quickRes.item
        ignoreUnknownSignals: true
        onCloseRequested: ov.closePanel()
        onOpenSettingsPage: ov.settingsFromPanel(id)
    }
    Connections {
        target: pickerRes.item
        ignoreUnknownSignals: true
        onCloseRequested: ov.closePanel()
        onActivated: { ov.closePanel(); LS.adhoc.call("luna://com.webos.applicationManager", "/launch", JSON.stringify({ id: appId })) }
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
    // The panels stay in the scene for a while after they are closed (panels/PanelMotion.qml). Another surface joins nodes to it and rebuilds the
    // render list anyway, and over Home nothing is left to open them for: they go then, not in a frame of their own later.
    function releaseParked(except) {
        if (except !== "quick" && quickRes.item && quickRes.item.release) quickRes.item.release()
        if (except !== "picker" && pickerRes.item && pickerRes.item.release) pickerRes.item.release()
    }
    onSurfaceChanged: if (surface !== "") releaseParked(surface)
    // The backdrop is an assignment on the way in, as Home's is on the way back (LumaHome.onHostActiveChanged): a Binding that switches off
    // behaves differently in Qt 5.12 (the set) and 5.15 (the offline host).
    onAppFrontChanged: { if (appFront) { if (!ownWindow) Lu.backdrop = flatTex; tryPending() } else { pending = null; closeAll(); releaseParked("") } }

    // ---- banners
    function showBanner(spec) {
        if (rustBanner || !appFront || !bannerRes.item || !spec || !spec.kind) return false
        if (surface === "drawer") return false           // the drawer is what the person asked for
        bannerRes.item.show(spec)
        if (bannerRes.item.interactive) { surface = "banner"; idle.restart() }
        else surface = ""
        return true
    }
    function hideBanner() { if (bannerRes.item) bannerRes.item.hide() }
    // What the buttons of a banner do over an app: the picker and Quick Settings are LG's system windows, which open over anything. They are
    // opened with the key filter's own luna client (Bus.systemLaunch), the client LG's key filters open them with; `LS.adhoc` is the way only
    // where the filter has none to give (the offline host), since the compositor hosts systemUIManager itself and the first TV run (round 7)
    // showed an overlay button that called it through LS.adhoc opening nothing.
    function systemWindow(appId) {
        var through = Bus.systemLaunch(appId) ? "the key filter's client" : "LS.adhoc"
        if (through === "LS.adhoc") LS.adhoc.call("luna://com.webos.service.systemUIManager", "/launch", JSON.stringify({ appId: appId }))
        console.info("[LumaOverlay] " + appId + " asked for through " + through)
    }
    function bannerPicked(id) {
        closeAll()
        if (id === "inputs") systemWindow("com.webos.app.quickinputpicker")
        else if (id === "picture") systemWindow("com.webos.app.quicksettings")
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
    // the gear key held (the key filter or the compositor times the hold; LumaBus.js asks): what the owner chose for it. Settings is a layer of
    // Home, so over an app the hold takes the TV to Home, which opens it; the drawer opens, or closes if it was up; nothing is taken all the same.
    function gearReady(mode) { return appFront && ((mode || gearMode) !== "notifications" || !!drawerRes.item) }
    function gearHold(mode) {
        if (!gearReady(mode)) return false
        var m = mode || gearMode
        if (m === "nothing") return true
        if (m === "settings") { closeDrawer(); settingsFromPanel(""); return true }
        if (surface === "drawer") { closeDrawer(); return true }
        return openDrawer()
    }

    // ---- the quick panel and the input picker over an app (the gear's tap and the Input key: the key filter's second entry, LumaBus.js).
    // The same panels as Home's, built here: a press shows one, the gear again hides the quick panel, the Input key again moves the picker on to
    // the next input. Whatever else was up (a banner, the drawer, the other panel) gives way.
    function panelReady(kind) {
        // the switcher is the one panel that is not over an app only: its door is a second press of Home, over Home or over an app
        if (kind === "switcher") return switcherOwn && switcherPress && !!switcherRes.item
        if (!appFront) return false
        return kind === "quick" ? quickOn && !!quickRes.item : kind === "picker" ? pickerOn && !!pickerRes.item : false
    }
    function panel(kind) {
        if (!panelReady(kind)) return false
        if (surface === kind) {
            if (kind === "picker") { pickerRes.item.next(); idle.restart() }
            else closePanel()
            return true
        }
        if (bannerRes.item) bannerRes.item.hide()
        if (kind === "picker") pickerRes.item.open({})
        surface = kind
        idle.restart()
        return true
    }
    function closePanel() { if (surface === "quick" || surface === "picker") { surface = ""; idle.stop() } }
    // The app switcher's doors for Home: how many apps it would list, and opening it (Back on the top of Home; a second press of Home is the
    // compositor's). Under LumaTV's compositor opening is a request it answers by opening; under LG's the surface opens itself.
    function switcherCount() { return !switcherRes.item || !switcherOn ? 0 : switcherRes.item.count }
    function switcherRead() { if (switcherRes.item && switcherRes.item.readLg) switcherRes.item.readLg() }
    function openSwitcher() {
        if (!switcherRes.item || !switcherOn || switcherRes.item.count === 0) return false
        if (switcherOwn) switcherPost({ op: "open" })
        else switcherLocal = true
        return true
    }
    // The gear is down over an app: its tap opens the quick panel when the key is let go, so the panel joins the scene now, off the screen, and the
    // slide of the opening starts in a scene that has it (panels/PanelMotion.qml). Nothing is shown and nothing is taken.
    function warmPanel(kind) {
        if (kind !== "quick" || !panelReady(kind) || surface !== "" || !quickRes.item.warm) return false
        quickRes.item.warm()
        return true
    }
    // "All settings" and the rows that open a page: Settings is a layer of Home, so over an app they take the TV to Home, which opens it
    // (LumaBus.js holds the request until Home is in front)
    function settingsFromPanel(page) {
        closePanel()
        Bus.wantSettings(page)
        LS.adhoc.call("luna://com.webos.applicationManager", "/launchDefaultApp", JSON.stringify({ category: "home" }))
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
    // The top card's key, over an app and over Home alike (Home asks through the bus): the camera's viewer takes every key; a card takes OK and
    // Back once armed, the camera's pop-up first (it is on top), then the overlay system's. True when taken.
    function cardKey(dir) {
        var cam = cameraRes.item
        if (cam && cam.viewerUp) return cam.key(dir)
        if (dir !== "ok" && dir !== "back") return false
        if (!cardsArmed) return false
        if (popupUp) return cam.key(dir)
        return !!overlaysRes.item && overlaysRes.item.key(dir)
    }
    property bool okHeldDown: false
    Timer { id: okHold; interval: Lu.motion.holdMs || 700; onTriggered: { if (ov.surface === "quick" && quickRes.item && quickRes.item.hold) quickRes.item.hold() } }
    function key(k, pressed, autoRepeat, deviceId) {
        if (pressed) Lu.pointerHide()
        if (!wants) return false
        var dir = directionFor(k)
        if (dir === "") return false
        // the app switcher has the keys while it is up, over Home as over an app
        if (switcherUp && switcherRes.item) {
            if (!pressed) return true
            if (autoRepeat && (dir === "back" || dir === "ok")) return true
            switcherRes.item.key(dir)
            return true
        }
        // Over an app the cards take OK and Back (once armed) and the camera's viewer every key; a press that is not theirs goes on to the app
        // (and its release with it)
        if (surface === "" && cardsUp || surface === "" && viewerUp) {
            if (!viewerUp && (dir !== "ok" && dir !== "back" || !cardsArmed)) return false
            if (!pressed) return true
            if (autoRepeat && (dir === "back" || dir === "ok")) return true
            return cardKey(dir)
        }
        // OK on the quick panel is decided on its release, as on Home, so that a held OK can edit the tiles (panels/QuickPanel.qml): the press
        // starts the timer, the release before it is up is an ordinary OK, the timer is the hold. Where the panel has nothing to pick up, OK acts on the press.
        if (dir === "ok" && surface === "quick" && quickRes.item && quickRes.item.holdReady && (okHold.running || okHeldDown || (pressed && quickRes.item.holdReady()))) {
            idle.restart()
            if (pressed && !autoRepeat) { okHeldDown = true; okHold.restart() }
            else if (!pressed && okHeldDown) { okHeldDown = false; if (okHold.running) { okHold.stop(); quickRes.item.key("ok") } }
            return true
        }
        if (!pressed) return true
        if (autoRepeat && (dir === "back" || dir === "ok")) return true
        idle.restart()
        if (surface === "drawer" && drawerRes.item) drawerRes.item.key(dir)
        else if (surface === "banner" && bannerRes.item) bannerRes.item.key(dir)
        else if (surface === "quick" && quickRes.item) quickRes.item.key(dir)
        else if (surface === "picker" && pickerRes.item) pickerRes.item.key(dir)
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
        else if (d && (d.open === "quick" || d.open === "picker")) panel(d.open)
        // {"card": {"class": "timer", "title": "Pasta", "body": "Time's up", "source": "Kitchen Show", "actions": [...], "score": {...}, "ttl": 20}}: an overlay card
        // made by hand, as the plugin broker's record would make it, for the gate and the TV steps
        else if (d && d.card && overlaysRes.item) overlaysRes.item.door(d.card)
        else if (d && d.kind) showBanner(d)
    }
    Timer { interval: 1000; running: ov.appFront; repeat: true; onTriggered: ov.readDoor() }

    Component.onCompleted: {
        Lu.flatBackdrop = flatTex
        Bus.setOverlay({ wants: function() { return ov.wants }, key: ov.key, ready: ov.gearReady, hold: ov.gearHold, panelReady: ov.panelReady, panel: ov.panel, warm: ov.warmPanel,
                         switcherCount: ov.switcherCount, openSwitcher: ov.openSwitcher, switcherRead: ov.switcherRead })
        // Home routes the D-pad here while a camera surface is up over it (LumaHome.keyAction)
        Bus.setCamera({ up: function() { return (ov.cameraUp || ov.overlaysUp) && ov.surface === "" }, key: function(dir) { return ov.cardKey(dir) } })
        console.info("[LumaOverlay] loaded")
    }
    Component.onDestruction: { Bus.setOverlay(null); Bus.setCamera(null) }
}
