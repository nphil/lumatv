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
//              screensaver over an app. Its window stays mapped once Home has settled, so a toast's first frame costs one
//              frame and not a window, its buffers and their warm-up as well (44-52 ms on the tenth gate); the compositor
//              draws it only while `live` says something of it shows (`report`), so an app's picture goes straight to the
//              screen the rest of the time. It takes the keys only while a surface of it wants them (`wants`).
//
// Keys come on wl_keyboard after the compositor's own routing (Home, Volume, Mute, the gear and Input never arrive) and
// are handed to LumaBus.js through the module's own LumaKeyBridge.qml, as the key filter hands them under LG's
// compositor. The gear's tap, Input and the gear's hold come as `show` events once this client said it `can` show the
// panel; the compositor otherwise launches LG's app for the key, as it did before this client existed. The overlay's cards (the doorbell,
// a finished timer, an alert, a score: panels/Overlays.qml) do not want the keyboard focus, since that would take the arrows from the app: the
// report's `takes` asks the compositor for OK and Back by name while a card is up and has been for a moment, and they come as `syskey` events.
WebOSWindow {
    id: homeWin
    objectName: "homeWin"
    windowType: "_WEBOS_WINDOW_TYPE_LUMATV_HOME"
    appId: "org.nphil.lumatv.shell"
    title: "LumaTV shell: home"
    width: 1920
    height: 1080
    // Clear to transparent: LumaHome draws an opaque wallpaper of its own, which covers the window whole, except while the moving wallpaper plays on
    // the video plane under this window (the player's window, role wall): then nothing is drawn there and the plane shows through.
    color: "transparent"
    // Mapped once Home has loaded (the stub's card, which draws nothing, shows meanwhile) and not while LG's own Home is
    // meant to be on the screen (the Home stub's `lg` mode: LumaHome steps aside).
    visible: lumaHome.status === Loader.Ready && !lgHome

    readonly property string moduleUrl: "file://" + (Link.params.moduleDir || "/var/lib/lumatv/qml/WebOSCompositor/lumatv") + "/"
    readonly property var fg: Link.topics.foreground || ({})
    readonly property var switcherTopic: Link.topics.switcher || ({})
    readonly property var switcherFeed: ({ apps: Link.topics.apps || [], open: switcherTopic.open === true, from: switcherTopic.from || "", n: switcherTopic.n || 0 })
    readonly property bool screenOn: !Link.topics.screen || Link.topics.screen.on !== false
    // The compositor draws LumaTV's Home now: the Home stub's card is in front, or no card is.
    readonly property bool homeCurrent: fg.home === true
    readonly property bool covered: fg.covered !== false
    readonly property bool appFront: !homeCurrent && !!fg.app && fg.app !== "com.webos.app.home" && screenOn
    readonly property bool lgHome: !!lumaHome.item && lumaHome.item.lgHome === true
    readonly property bool homeReady: lumaHome.status === Loader.Ready && !!lumaHome.item
    // An app that takes the front under a panel of the system layer ends the panel (LG's own windows end with the app switch): the
    // Input key held opens the picker at its press, and a second later the Home Hub in front of it. Banners are left alone.
    readonly property string fgApp: fg.app || ""
    onFgAppChanged: {
        if (overlay.item && overlay.item.closePanel) overlay.item.closePanel()
        if (fgApp !== "" && fgApp !== "com.webos.app.home" && lumaHome.item && lumaHome.item.appFronted) lumaHome.item.appFronted(fgApp)
    }

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
    // The compositor animates launches and returns (its `transitions` topic): Home tells it where a tile is and waits to show its launch screen.
    readonly property bool transitionsOn: !!Link.topics.transitions && Link.topics.transitions.on === true
    Binding { target: lumaHome.item; property: "smoothLaunch"; value: homeWin.transitionsOn; when: homeWin.homeReady }
    // The moving wallpaper (LumaHome's WallVideo): this compositor has the wallpaper window under Home's, so the video wallpaper is offered, and Home
    // takes its own wallpaper away only while the compositor says the hole is in its picture (the `wall` topic).
    readonly property bool wallHole: !!Link.topics.wall && Link.topics.wall.hole === true
    Binding { target: lumaHome.item; property: "videoWallBackend"; value: "own"; when: homeWin.homeReady }
    Binding { target: lumaHome.item; property: "videoWallHole"; value: homeWin.wallHole; when: homeWin.homeReady }
    // The Hz the compositor's output runs at (the `refresh` topic; 120 is the test mode, docs/research/ui120-result.md): Home trims what a
    // frame costs at 120 (kit/Lu.qml `fast`).
    readonly property int refreshHz: Link.topics.refresh && Link.topics.refresh.hz ? Link.topics.refresh.hz : 60
    Binding { target: lumaHome.item; property: "refreshHz"; value: homeWin.refreshHz; when: homeWin.homeReady }

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
        visible: homeWin.sysLive || homeWin.sysWarm

        Loader {
            id: overlay
            anchors.fill: parent
            source: homeWin.moduleUrl + "LumaOverlay.qml"
            onStatusChanged: if (status === Loader.Error) console.warn("[Shell] LumaOverlay failed to load from " + source)
        }
        Binding { target: overlay.item; property: "appFront"; value: homeWin.appFront; when: overlay.status === Loader.Ready }
        // Do not disturb: the overlay's cards wait while an alert, the input popup or the PIN prompt of the notifications, the keyboard or the saver
        // has the screen, and while the screen is off (a ring while the TV is in standby is still there when it is switched on, if it is in date);
        // its first card is drawn once while the layer is warmed
        Binding { target: overlay.item; property: "systemBusy"; value: homeWin.notifyWants || homeWin.keyboardUp || homeWin.saverUp || !homeWin.screenOn; when: overlay.status === Loader.Ready }
        Binding { target: overlay.item; property: "warming"; value: homeWin.warmShow; when: overlay.status === Loader.Ready }
        // The app switcher (panels/Switcher.qml in the overlay): what the compositor says is running and whether it has opened the switcher; the
        // overlay's requests go back over the channel (the cards' places, OK, Quit, Quit all, close).
        Binding { target: overlay.item; property: "switcherFeed"; value: homeWin.switcherFeed; when: overlay.status === Loader.Ready && Link.up }
        Connections { target: overlay.item; ignoreUnknownSignals: true; onSwitcherPost: Link.post("/v1/switcher", op) }
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
        // The capture indicator (panels/CaptureMark.qml): the recording dot while the compositor's `capture` topic says one runs, a note when a
        // capture ends. Above everything else of this layer. The compositor starts no recording, and ends one, without the shell saying in its
        // state (`capture.dot`) that the dot is up.
        Loader {
            id: capture
            anchors.fill: parent
            z: 5
            source: homeWin.moduleUrl + "panels/CaptureMark.qml"
            onStatusChanged: if (status === Loader.Error) console.warn("[Shell] CaptureMark failed to load from " + source)
        }
        Binding { target: capture.item; property: "topic"; value: Link.topics.capture || ({}); when: capture.status === Loader.Ready }
        // LumaTV's own on-screen keyboard (panels/Keyboard.qml): built once in the background, drawn only while a field the compositor serves
        // here asks for it. It takes no keys of this window: the remote's keys reach it as `imekey` events (Link), so that the field's app
        // keeps the keyboard focus. Below the overlay, whose volume display sits at the foot of the screen, where the keyboard's keys are, and
        // below the notifications: a change of volume while typing is seen.
        Loader {
            id: kbd
            anchors.fill: parent
            z: -1
            asynchronous: true
            source: homeWin.moduleUrl + "panels/Keyboard.qml"
            onStatusChanged: {
                if (status === Loader.Error) console.warn("[Shell] Keyboard failed to load from " + source)
                homeWin.report()
            }
        }
        Binding { target: kbd.item; property: "field"; value: Link.topics.ime || ({}); when: kbd.status === Loader.Ready }
        Binding { target: kbd.item; property: "voiceOk"; value: homeWin.voiceOk; when: kbd.status === Loader.Ready }
        Binding { target: kbd.item; property: "voiceNote"; value: homeWin.voiceNote; when: kbd.status === Loader.Ready }
        Binding { target: kbd.item; property: "listening"; value: homeWin.voiceListening; when: kbd.status === Loader.Ready }
        Connections {
            target: kbd.item
            ignoreUnknownSignals: true
            onTyped: homeWin.imePost({ op: "text", text: text })
            onFieldAction: homeWin.imePost(name === "clear" ? { op: "clear" } : { op: "key", key: name })
            onHideAsked: homeWin.imePost({ op: "hide" })
            onShownAt: homeWin.imePost({ op: "shown", x: x, y: y, w: w, h: h })
            onVoiceAsked: homeWin.voiceStart()
            onVoiceCancelled: homeWin.voiceStop()
            // the keyboard going away ends a listening that is still open
            onWantedChanged: if (kbd.item && !kbd.item.wanted && homeWin.voiceListening) homeWin.voiceStop()
        }
        Item {
            id: sysKeys
            anchors.fill: parent
            focus: true
            Keys.onPressed: homeWin.sysKey(event, true)
            Keys.onReleased: homeWin.sysKey(event, false)
        }
        // A few frames at the warm-up walk the window's three buffers, so the compositor has imported and warmed each of
        // them (the first toast's second and third frames would otherwise each wait for one) while nothing shows.
        Rectangle {
            width: 1
            height: 1
            color: "transparent"
            opacity: 0.0
            NumberAnimation on opacity { id: warmRun; running: false; from: 0.001; to: 0.002; duration: 80 }
        }
    }
    readonly property bool notifyWants: !!notify.item && notify.item.wants
    readonly property bool saverUp: !!saver.item && saver.item.active
    // the surfaces of the overlay that take every key (the keyboard focus is theirs); the cards take OK and Back by name (`takes`) and the app keeps the focus
    readonly property bool overlayWants: !!overlay.item && (overlay.item.wantsFocus !== undefined ? overlay.item.wantsFocus : overlay.item.wants)
    readonly property var overlayTakes: !!overlay.item && overlay.item.takes !== undefined ? overlay.item.takes : []
    readonly property bool sysWants: overlayWants || notifyWants || saverUp
    readonly property bool keyboardUp: (!!kbd.item && kbd.item.wanted === true) || (!!Link.topics.keyboard && Link.topics.keyboard.up === true)
    readonly property bool kbdLive: !!kbd.item && kbd.item.live
    // the keyboard's panel, for the host tests and the gate (nothing in the shell reads it)
    readonly property var keyboardItem: kbd.item
    // the capture mark, for the host tests and the gate (nothing in the shell reads it)
    readonly property var captureItem: capture.item
    readonly property bool captureLive: !!capture.item && capture.item.live
    readonly property bool sysLive: (!!overlay.item && (overlay.item.live !== undefined ? overlay.item.live : overlay.item.visible)) || (!!notify.item && notify.item.live) || saverUp || kbdLive || captureLive
    // The system layer's window is made (and kept) two seconds after Home loaded, out of the way of Home's own first frames;
    // the walk of its buffers starts a moment after it mapped.
    property bool sysWarm: false
    Timer { interval: 2000; running: homeWin.homeReady; onTriggered: homeWin.sysWarm = true }
    Timer { interval: 300; running: homeWin.sysWarm; onTriggered: { warmRun.start(); homeWin.warmShow = true; warmHide.start() } }
    // The notifications' kept toast slot is drawn for a moment while the layer is warmed, so a first toast finds its glyphs cached.
    property bool warmShow: false
    Timer { id: warmHide; interval: 400; onTriggered: homeWin.warmShow = false }
    Binding { target: notify.item; property: "warming"; value: homeWin.warmShow; when: notify.status === Loader.Ready }
    // ---- the keyboard's link with the compositor (server/ime.rs): what it types, the keys it presses and where it is go back over
    // `POST /v1/ime`, tagged with the field they are for, so that a request that outlives its field is dropped there. The remote's keys come as
    // `imekey` events while the keyboard is up. The requests go one at a time, each after the answer to the one before: two requests made in
    // the same moment (Enter, then the keyboard going away) travel on connections of their own and arrived the other way round.
    readonly property int imeModel: Link.topics.ime ? (Link.topics.ime.model | 0) : 0
    property var imeQueue: []
    property bool imeBusy: false
    function imePost(op) {
        op.model = imeModel
        imeQueue.push(op)
        if (!imeBusy) imeSend()
    }
    function imeSend() {
        if (!imeQueue.length) { imeBusy = false; imeStall.stop(); return }
        imeBusy = true
        imeStall.restart()
        Link.post("/v1/ime", imeQueue.shift(), function() { homeWin.imeSend() })
    }
    // an answer that never comes (the compositor went) must not hold the keyboard's requests for ever
    Timer { id: imeStall; interval: 2000; onTriggered: { homeWin.imeQueue = []; homeWin.imeBusy = false } }
    Connections {
        target: Link
        onImeKey: { if (kbd.item) kbd.item.imeKey(key, down, repeat) }
        onUpChanged: { if (Link.up && !homeWin.voiceProbed) { homeWin.voiceProbed = true; homeWin.voiceProbe() } }
    }

    // ---- the keyboard's microphone: LG's voice service does the listening (the call maliit's own dictation card makes, recognizeVoiceWithDetails
    // with `runVoiceUi: false`), the keyboard says what it is doing on its space bar and the first answer is typed into the field. The key is
    // there only when the TV says it has voice recognition; a TV whose voice services are stopped (the `lean` profile) answers with an error
    // and the space bar says so.
    property bool voiceOk: false
    property bool voiceProbed: false
    property bool voiceListening: false
    property string voiceNote: ""
    property string voiceTicket: ""
    property int voiceTag: 0
    function voiceProbe() {
        Link.luna("luna://com.webos.service.config/getConfigs", { configNames: ["system.supportVoiceRecognition"] }, false, function(p) {
            voiceOk = !!(p && p.configs && p.configs["system.supportVoiceRecognition"] === true)
        })
    }
    function voiceStart() {
        if (voiceListening) return
        voiceListening = true
        voiceTicket = ""
        voiceNote = "Listening"
        voiceTag = Link.luna("luna://com.webos.service.voiceconductor/system/recognizeVoiceWithDetails",
                             { subscribe: true, language: "en-US", runVoiceUi: false }, true, voiceEvent)
        voiceLimit.restart()
    }
    function voiceStop(note) {
        voiceLimit.stop()
        if (voiceTag) {
            Link.cancel(voiceTag)
            if (voiceTicket !== "") Link.luna("luna://com.webos.service.voiceconductor/cancelRecognition", { voiceTicket: voiceTicket }, false, function() {})
        }
        voiceTag = 0
        voiceTicket = ""
        voiceListening = false
        voiceNote = note || ""
        if (note) voiceNoteClear.restart()
    }
    function voiceEvent(p) {
        if (!voiceListening || !p) return
        if (p.returnValue === false || p.event === "sttError") { voiceStop("Voice isn't available"); return }
        if (p.event === "sttStart") voiceTicket = String(p.voiceTicket || "")
        else if (p.event === "sttConnecting") voiceNote = "Working"
        else if (p.event === "sttResult") {
            var words = p.text && p.text.length ? String(p.text[0]) : ""
            voiceStop(words === "" ? "Nothing heard" : "")
            if (words !== "") imePost({ op: "text", text: words })
        }
    }
    Timer { id: voiceLimit; interval: 20000; onTriggered: homeWin.voiceStop("Voice isn't available") }
    Timer { id: voiceNoteClear; interval: 3000; onTriggered: homeWin.voiceNote = "" }
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
    // A key the overlay's cards took (the compositor's `takes`, shell.rs: OK and Back while an app has the focus and a card is up, the app never
    // sees them): the same dispatch as a key of this layer's own window, from the event Link brings.
    function sysKeyEvent(name, pressed, repeat) {
        var k = name === "ok" ? Qt.Key_Return : name === "back" ? Qt.Key_Back : 0
        if (k === 0 || !bridge) return
        var taken = false
        try { taken = bridge.dispatchOverlay(k, pressed, repeat, 0) }
        catch (e) { console.warn("[Shell] syskey " + name + " threw: " + e) }
        keysSeen++
        if (!taken) keysDropped++
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
            if (bridge.panelReady("switcher")) c.push("switcher")
            if (kbd.status === Loader.Ready && !!kbd.item && kbd.item.ready === true) c.push("keyboard")
        } catch (e) {}
        return c
    }
    function state() {
        var h = lumaHome.item, o = overlay.item, n = notify.item
        return {
            homeLoaded: !!h, hostActive: !!h && h.hostActive === true, layer: h ? String(h.layerOpen || "") : "", focus: h ? String(h.focusLayer || "") : "",
            settings: !!h && h.settingsOpen === true, overlay: o ? String(o.surface || "") : "", osd: !!o && o.osdLive === true,
            cards: o ? { on: o.overlaysOn === true, popup: o.popupUp === true, stack: o.overlaysUp === true, armed: o.cardsArmed === true, dnd: o.dnd === true } : null,
            toasts: n ? n.toasts.length : 0, alert: !!n && n.alertUp, pin: !!n && n.pinUp, saver: saverUp || (!!h && h.saverUp === true), keys: keysSeen, dropped: keysDropped,
            beat: !!h && h.lastBeatMs > 0, idleHold: !!h && h.idleHold === true, link: { up: Link.up, calls: Link.nextTag, events: Link.events },
            wall: { backend: h ? String(h.videoWallBackend || "") : "", hole: wallHole, up: !!h && h.videoWallUp === true },
            // the refresh Home draws for (the `refresh` topic as Home took it): `ctl shell` says whether the 120 Hz mode reached the client
            refreshHz: h ? (h.refreshHz | 0) : 0,
            // the last per-frame slides at 120 Hz ([{steps, ms, duration}]) and whether Home's base is drawn from its cached texture now
            slides: h && h.slides ? h.slides : [], baseCached: !!h && h.baseCached === true,
            capture: { dot: !!capture.item && capture.item.dotUp === true },
            keyboard: kbd.item ? { up: kbd.item.wanted, live: kbd.item.live, layout: kbd.item.layoutId, focus: kbd.item.focusAt, key: kbd.item.focusKey ? kbd.item.focusKey.id : "",
                                   shift: kbd.item.shift, listening: voiceListening, voice: kbd.item.voiceOk, busy: kbd.item.busy } : null
        }
    }
    function report() {
        var doc = { can: can(), wants: sysWants, takes: overlayTakes, live: sysLive, state: state() }
        var text = JSON.stringify(doc)
        if (text === lastReport) return
        lastReport = text
        Link.report(doc)
    }
    Timer { interval: 1000; repeat: true; running: true; onTriggered: homeWin.report() }
    onSysWantsChanged: report()
    onOverlayTakesChanged: report()
    onSysLiveChanged: report()
    onHomeCurrentChanged: report()
    Connections { target: capture.item; ignoreUnknownSignals: true; onDotUpChanged: homeWin.report() }

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
        onSysKey: homeWin.sysKeyEvent(key, down, repeat)
        // The Home key while LumaTV's Home has the screen: back to the top of Home, as LG's Home does on its own key; with nothing open on Home and
        // the switcher on offer (the compositor says so), the app switcher instead.
        onHomeKey: {
            var h = lumaHome.item
            if (!h) return
            if (switcher && h.atRest && overlay.item && overlay.item.openSwitcher()) return
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
