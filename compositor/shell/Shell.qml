import QtQuick 2.4
import Eos.Window 0.1
import LumaShell 1.0
import "imports/LumaShell/Keymap.js" as Keymap

// LumaTV's shell under LumaTV's compositor: LumaTV's existing QML (the module staged at <dataDir>/qml/WebOSCompositor/<module>,
// the one LG's compositor loads as its shadow module) in a 32-bit Qt process on LG's Qt 5.12, started by the compositor
// through /usr/bin/qml-runner (compositor/crates/lumatv-compositor/src/shell.rs). Two windows, told apart by their
// types and titles:
//
//   this one   LumaHome with its layers (Settings, the panels, the library, the saver on Home), drawn in the card slot
//              while the Home stub's card is the foreground one; `hostActive` and `homeCurrent` as the fullscreen
//              container sets them under LG's compositor, from the compositor's `foreground` topic.
//   sysWin     the system layer above every window: LumaOverlay (the banners, the drawer, the quick panel and the input
//              picker over an app, the volume display over everything), the notifications (panels/Notify.qml) and the
//              screensaver over an app. Mapped only while something of it shows, so an app's picture goes straight to the
//              screen the rest of the time; it takes the keys only while a surface of it wants them (`wants`).
//
// Keys come on wl_keyboard after the compositor's own routing (Home, Volume, Mute, the gear and Input never arrive) and
// are handed to LumaBus.js through the module's own LumaKeyBridge.qml, as the key filter hands them under LG's
// compositor. The gear's tap, Input and the gear's hold come as `show` events once this client said it `can` show the
// panel; the compositor otherwise launches LG's app for the key, as it did before this client existed.
WebOSWindow {
    id: homeWin
    objectName: "homeWin"
    windowType: "_WEBOS_WINDOW_TYPE_LUMATV_HOME"
    appId: "org.nphil.lumatv.shell"
    title: "LumaTV shell: home"
    width: 1920
    height: 1080
    color: "black"
    // Mapped once Home has loaded (the stub's card, which draws nothing, shows meanwhile) and not while LG's own Home is
    // meant to be on the screen (the Home stub's `lg` mode: LumaHome steps aside).
    visible: lumaHome.status === Loader.Ready && !lgHome

    readonly property string moduleUrl: "file://" + (Link.params.moduleDir || "/var/lib/lumatv/qml/WebOSCompositor/lumatv") + "/"
    readonly property var fg: Link.topics.foreground || ({})
    readonly property bool screenOn: !Link.topics.screen || Link.topics.screen.on !== false
    // The compositor draws LumaTV's Home now: the Home stub's card is in front, or no card is.
    readonly property bool homeCurrent: fg.home === true
    readonly property bool covered: fg.covered !== false
    readonly property bool appFront: !homeCurrent && !!fg.app && fg.app !== "com.webos.app.home" && screenOn
    readonly property bool lgHome: !!lumaHome.item && lumaHome.item.lgHome === true
    readonly property bool homeReady: lumaHome.status === Loader.Ready && !!lumaHome.item

    Loader {
        id: lumaHome
        anchors.fill: parent
        source: homeWin.moduleUrl + "LumaHome.qml"
        onStatusChanged: {
            if (status === Loader.Error) console.warn("[Shell] LumaHome failed to load from " + source)
            else if (status === Loader.Ready) { console.info("[Shell] LumaHome loaded"); homeWin.makeBridge() }
        }
    }
    Binding { target: lumaHome.item; property: "hostActive"; value: homeWin.homeCurrent && homeWin.screenOn && !homeWin.lgHome; when: homeWin.homeReady }
    Binding { target: lumaHome.item; property: "homeCurrent"; value: homeWin.homeCurrent; when: homeWin.homeReady }
    // Covered whole (an app, a full-screen overlay such as LG's Settings): the idle tick stops, nothing is drawn until Home shows again.
    Binding { target: lumaHome.item; property: "idleHold"; value: homeWin.covered; when: homeWin.homeReady }

    // The module's key bridge: the way into LumaBus.js's engine-wide instance (the one LumaHome and LumaOverlay registered
    // with), made from the module's own file so the library URL is theirs.
    property var bridge: null
    function makeBridge() {
        if (bridge) return
        var c = Qt.createComponent(moduleUrl + "LumaKeyBridge.qml")
        if (c.status === Component.Ready) bridge = c.createObject(homeWin)
        else console.warn("[Shell] LumaKeyBridge: " + c.errorString())
    }

    // ---- keys: a press while the key is down is the remote's repeat
    property var down: ({})
    function keyEvent(event, pressed, toOverlay) {
        var code = Keymap.scancode(event)
        var k = Keymap.qtKey(event)
        var repeat = pressed && !!down[code]
        if (pressed) down[code] = true; else delete down[code]
        var taken = false
        if (bridge) {
            try { taken = toOverlay ? bridge.dispatchOverlay(k, pressed, repeat, 0) : bridge.dispatch(k, pressed, repeat, 0) }
            catch (e) { console.warn("[Shell] key " + code + " threw: " + e) }
        }
        event.accepted = true
        keysSeen++
        if (!taken) keysDropped++
    }
    property int keysSeen: 0
    property int keysDropped: 0
    Item {
        id: homeKeys
        anchors.fill: parent
        focus: true
        Keys.onPressed: homeWin.keyEvent(event, true, false)
        Keys.onReleased: homeWin.keyEvent(event, false, false)
    }

    // ---- the system layer
    WebOSWindow {
        id: sysWin
        objectName: "sysWin"
        windowType: "_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM"
        appId: "org.nphil.lumatv.shell"
        title: "LumaTV shell: system"
        width: 1920
        height: 1080
        color: "transparent"
        visible: homeWin.sysLive

        Loader {
            id: overlay
            anchors.fill: parent
            source: homeWin.moduleUrl + "LumaOverlay.qml"
            onStatusChanged: if (status === Loader.Error) console.warn("[Shell] LumaOverlay failed to load from " + source)
        }
        Binding { target: overlay.item; property: "appFront"; value: homeWin.appFront; when: overlay.status === Loader.Ready }
        Loader {
            id: notify
            anchors.fill: parent
            z: 2
            source: homeWin.moduleUrl + "panels/Notify.qml"
            onStatusChanged: if (status === Loader.Error) console.warn("[Shell] Notify failed to load from " + source)
        }
        // The screensaver over an app (on Home, LumaHome's own saver draws): active while LG's power state says Screen Saver, or
        // while tvpower asked and was told LumaTV's is the one to show.
        Loader {
            id: saver
            anchors.fill: parent
            z: 3
            source: homeWin.moduleUrl + "panels/Screensaver.qml"
            visible: !!item && item.active
            onStatusChanged: if (status === Loader.Error) console.warn("[Shell] Screensaver failed to load from " + source)
        }
        Binding { target: saver.item; property: "hostActive"; value: homeWin.appFront; when: saver.status === Loader.Ready }
        Item {
            id: sysKeys
            anchors.fill: parent
            focus: true
            Keys.onPressed: homeWin.sysKey(event, true)
            Keys.onReleased: homeWin.sysKey(event, false)
        }
    }
    readonly property bool notifyWants: !!notify.item && notify.item.wants
    readonly property bool saverUp: !!saver.item && saver.item.active
    readonly property bool overlayWants: !!overlay.item && overlay.item.wants
    readonly property bool sysWants: overlayWants || notifyWants || saverUp
    readonly property bool sysLive: (!!overlay.item && overlay.item.visible) || (!!notify.item && notify.item.live) || saverUp
    // The notifications and the saver have the keys before the overlay's surfaces (LG's order: the PIN prompt and alerts above
    // everything); digits go to the PIN prompt.
    function sysKey(event, pressed) {
        if (saverUp) {
            if (pressed && !down[Keymap.scancode(event)]) saver.item.key("")
            homeWin.noteKey(event, pressed)
            return
        }
        if (notifyWants) {
            var k = Keymap.qtKey(event)
            if (pressed && !down[Keymap.scancode(event)]) {
                var dir = k === Qt.Key_Left ? "left" : k === Qt.Key_Right ? "right" : k === Qt.Key_Up ? "up" : k === Qt.Key_Down ? "down"
                        : (k === Qt.Key_Return || k === Qt.Key_Enter) ? "ok" : k === Qt.Key_Back ? "back" : ""
                if (dir !== "") notify.item.key(dir)
                else if (k >= Qt.Key_0 && k <= Qt.Key_9) notify.item.digit(k - Qt.Key_0)
            }
            homeWin.noteKey(event, pressed)
            return
        }
        keyEvent(event, pressed, true)
    }
    function noteKey(event, pressed) {
        var code = Keymap.scancode(event)
        if (pressed) down[code] = true; else delete down[code]
        event.accepted = true
        keysSeen++
    }

    // ---- what the compositor is told: what this client can show, whether the layer wants the keys, and its state
    property string lastReport: ""
    function can() {
        var c = []
        if (!bridge) return c
        try {
            if (bridge.panelReady("quick")) c.push("quick")
            if (bridge.panelReady("picker")) c.push("picker")
            if (bridge.gearReady()) c.push("drawer")
        } catch (e) {}
        return c
    }
    function state() {
        var h = lumaHome.item, o = overlay.item, n = notify.item
        return {
            homeLoaded: !!h, hostActive: !!h && h.hostActive === true, layer: h ? String(h.layerOpen || "") : "", focus: h ? String(h.focusLayer || "") : "",
            settings: !!h && h.settingsOpen === true, overlay: o ? String(o.surface || "") : "", osd: !!o && o.osdLive === true,
            toasts: n ? n.toasts.length : 0, alert: !!n && n.alertUp, pin: !!n && n.pinUp, saver: saverUp || (!!h && h.saverUp === true), keys: keysSeen, dropped: keysDropped,
            beat: !!h && h.lastBeatMs > 0, idleHold: !!h && h.idleHold === true, link: { up: Link.up, calls: Link.nextTag, events: Link.events }
        }
    }
    function report() {
        var doc = { can: can(), wants: sysWants, state: state() }
        var text = JSON.stringify(doc)
        if (text === lastReport) return
        lastReport = text
        Link.report(doc)
    }
    Timer { interval: 1000; repeat: true; running: true; onTriggered: homeWin.report() }
    onSysWantsChanged: report()
    onHomeCurrentChanged: report()

    // ---- the compositor's requests
    property int shows: 0
    function show(kind, warm) {
        shows++
        if (!bridge) return
        try {
            if (warm) { bridge.warmPanel(kind); return }
            if (kind === "drawer") bridge.gearHold()
            else bridge.panel(kind)
        } catch (e) { console.warn("[Shell] show " + kind + " threw: " + e) }
        report()
    }
    Connections {
        target: Link
        onShow: homeWin.show(kind, warm)
        // The Home key while LumaTV's Home has the screen: back to the top of Home, as LG's Home does on its own key.
        onHomeKey: {
            var h = lumaHome.item
            if (!h) return
            if (h.settingsOpen) h.closeSettings()
            h.closeAllLayers()
            if (h.hostActive) h.focusLayer = "dock"
        }
        onUpChanged: if (Link.up) { homeWin.lastReport = ""; homeWin.report() }
    }

    Component.onCompleted: {
        Link.start()
        console.info("[Shell] started: module " + moduleUrl + ", channel " + Link.base)
    }
}
