import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0
import "../kit"

// LG's notification feeds (com.webos.notification) drawn by LumaTV under its own compositor, where LG's notification
// view (SystemUI/Notification, restyled through payload/notification under LG's compositor) does not run: the toast,
// the alert, the input popup (a device connected to an input) and the PIN prompt, in the looks of the restyle
// (docs/mocks 07, 06, 18, 19). Built in the shell's system layer (compositor/shell/Shell.qml); under LG's compositor it
// is never instantiated, LG's view draws these there.
//
// Nothing is filtered or delayed: an EAS or thermal alert, a remote-battery or pairing toast is shown like any other,
// as soon as the service pushes it. The feeds are the ones LG's models read (`getToastNotification`,
// `getAlertNotification`, `getInputAlertNotification`, `getPincodePromptNotification`, subscribed); a button's
// `action` is a luna call the service attached (`serviceURI`, `serviceMethod`, `launchParams`), an alert's
// `onCloseAction` runs when it closes unanswered, and the PIN goes back through `closePincodePrompt` as LG's view
// relays it (`closeType` "relay" with the digits, "close" for a cancel; the third wrong try closes it).
//
// Keys: `wants` says a surface here takes them (an alert, the input popup, the PIN prompt; a toast never does), `key(dir)`
// moves the focus and answers, `digit(n)` types into the PIN prompt. The host ends a surface that was left alone
// through `closeRequested` by its own idle rule; the popups have timeouts of their own.
Item {
    id: notify
    anchors.fill: parent

    property var toasts: []                    // newest first: { id, title, body, iconUrl, expires }
    property var alerts: []                    // oldest first, [0] is shown: the alertInfo with `index` (the focused button)
    property var inputAlert: null              // the input popup's alertInfo, with `index`
    property var pin: null                     // the PIN prompt's pincodePromptInfo, with `typed` and `index`
    property int toastSerial: 0
    property bool rested: true
    // Set by the shell client while its system layer's window is warmed (compositor/shell/Shell.qml): the kept toast slot is drawn
    // once, with letters and digits, in a layer the compositor is not drawing. It never makes this surface `live`.
    property bool warming: false
    readonly property var warmToast: ({ title: "The quick brown fox jumps over the lazy dog 0123456789",
                                        body: "THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG - abcdefghijklmnopqrstuvwxyz : , . ! ? ' ( ) /", iconUrl: "" })

    readonly property bool alertUp: alerts.length > 0
    readonly property bool inputUp: inputAlert !== null
    readonly property bool pinUp: pin !== null
    readonly property bool wants: alertUp || inputUp || pinUp
    readonly property bool live: wants || toasts.length > 0
    signal closeRequested()
    readonly property var lunaCalls: LS.calls || []        // what the offline host's luna stand-in recorded; the set's bridge has none

    // ---- the feeds
    Service {
        id: toastSvc
        appId: LS.appId
        onResponse: { var m = parse(payload); if (m && m.message) notify.toast(m) }
        Component.onCompleted: call("luna://com.webos.notification", "/getToastNotification", JSON.stringify({ subscribe: true }))
    }
    Service {
        id: alertSvc
        appId: LS.appId
        onResponse: { var m = parse(payload); if (m && m.alertAction) notify.alert(m) }
        Component.onCompleted: call("luna://com.webos.notification", "/getAlertNotification", JSON.stringify({ subscribe: true }))
    }
    Service {
        id: inputSvc
        appId: LS.appId
        onResponse: { var m = parse(payload); if (m && m.alertAction) notify.input(m) }
        Component.onCompleted: call("luna://com.webos.notification", "/getInputAlertNotification", JSON.stringify({ subscribe: true }))
    }
    Service {
        id: pinSvc
        appId: LS.appId
        onResponse: { var m = parse(payload); if (m && m.pincodePromptAction) notify.prompt(m) }
        Component.onCompleted: call("luna://com.webos.notification", "/getPincodePromptNotification", JSON.stringify({ subscribe: true }))
    }
    function parse(payload) { try { return JSON.parse(payload) } catch (e) { return null } }
    function act(action) {
        if (!action || !action.serviceURI) return
        LS.adhoc.call(action.serviceURI, action.serviceMethod || "", JSON.stringify(action.launchParams || {}))
    }

    // ---- toasts: a pill at the bottom centre, the first line the title, the rest the subtitle; gone after its timeout (LG's 5 s)
    function toast(m) {
        var text = String(m.message || "").replace(/<br\s*\/?>/gi, "\n").replace(/^\s+|\s+$/g, "")
        if (text === "") return
        var lines = text.split("\n")
        var icon = String(m.iconPath || "")
        var own = icon === "" || /(^|\/)(info|notice|default)\.[a-z]+$/i.test(icon)
        var t = { id: ++toastSerial, title: lines[0], body: lines.slice(1).join(" "), iconUrl: own ? "" : (icon.charAt(0) === "/" ? "file://" + icon : icon),
                  expires: Date.now() + (m.timeout > 0 ? m.timeout * 1000 : 5000) }
        var list = toasts.slice()
        // LG replaces a toast of the same source and text (ToastModel.js) rather than stacking it.
        list = list.filter(function(o) { return !(o.title === t.title && o.body === t.body) })
        list.unshift(t)
        toasts = list.slice(0, 3)
        sweep.restart()
    }
    Timer {
        id: sweep
        interval: 250; repeat: true; running: notify.toasts.length > 0
        onTriggered: {
            var now = Date.now()
            var list = notify.toasts.filter(function(t) { return t.expires > now })
            if (list.length !== notify.toasts.length) notify.toasts = list
        }
    }

    // ---- alerts: a queue; a modal one goes to the front (LG's AlertModel); `close` and `closeAll` from the service remove
    function alert(m) {
        var list = alerts.slice(), i
        if (m.alertAction === "closeAll") { alerts = []; return }
        if (m.alertAction === "close") {
            alerts = list.filter(function(a) { return !(m.alertInfo && a.timestamp === m.alertInfo.timestamp) })
            return
        }
        if (m.alertAction !== "open" || !m.alertInfo) return
        var a = m.alertInfo
        a.index = 0
        for (i = 0; i < (a.buttons || []).length; i++) if (a.buttons[i].focus) a.index = i
        for (i = 0; i < list.length; i++) {
            var d = list[i]
            if (d.sourceId === a.sourceId && d.title === a.title && d.message === a.message) { list.splice(i, 1); break }
        }
        if (a.modal) {
            for (i = 0; i < list.length && list[i].modal; i++) {}
            list.splice(i, 0, a)
        } else list.push(a)
        alerts = list
        if (a.timeout > 0) alertTimeout.start(a)
    }
    Timer {
        id: alertTimeout
        property var target: null
        function start(a) { target = a; interval = a.timeout * 1000; restart() }
        onTriggered: if (target) notify.dropAlert(target, true)
    }
    function dropAlert(a, unanswered) {
        if (unanswered && a.onCloseAction) act(a.onCloseAction)
        alerts = alerts.filter(function(x) { return x !== a })
    }
    function answerAlert(i) {
        var a = alerts[0]
        if (!a) return
        var b = (a.buttons || [])[i]
        if (b && b.action) act(b.action)
        dropAlert(a, false)
    }

    // ---- the input popup: one at a time (LG keeps one); a `close` with its timestamp or its own timeout ends it
    function input(m) {
        if (m.alertAction === "close") { if (inputAlert && (!m.alertInfo || m.alertInfo.timestamp === inputAlert.timestamp)) inputAlert = null; return }
        if (m.alertAction !== "open" || !m.alertInfo) return
        var a = m.alertInfo
        if (!a.inputs && a.appId) a.inputs = [{ appId: a.appId, params: a.params, portName: a.portName, action: a.action }]
        for (var i = 0; i < (a.inputs || []).length; i++) {
            if (!a.inputs[i].action) a.inputs[i].action = { serviceURI: "luna://com.webos.applicationManager", serviceMethod: "/launch",
                                                             launchParams: { id: a.inputs[i].appId, params: a.inputs[i].params || {} } }
        }
        a.index = 0
        inputAlert = a
        inputTimeout.interval = (a.timeout > 0 ? a.timeout : 15) * 1000
        inputTimeout.restart()
    }
    Timer { id: inputTimeout; onTriggered: notify.inputAlert = null }

    // ---- the PIN prompt
    function prompt(m) {
        if (m.pincodePromptAction === "close") { pin = null; return }
        if (m.pincodePromptAction !== "open" || !m.pincodePromptInfo) return
        var p = m.pincodePromptInfo
        p.typed = ""
        p.index = 0
        p.tries = pin && pin.retry && p.retry ? pin.tries + 1 : (p.retry ? 1 : 0)
        pin = p
        // LG closes the prompt on the third wrong try.
        if (p.tries >= 3) { cancelPin(); return }
    }
    function relayPin() {
        if (!pin || pin.typed.length < 4) return
        var mode = pin.promptType === "parental" || !pin.promptType ? "" : pin.promptType
        LS.adhoc.call("luna://com.webos.notification", "/closePincodePrompt", JSON.stringify({ pincode: pin.typed, closeType: "relay", mode: mode, appId: pin.appId || "" }))
        pin = Object.assign({}, pin, { typed: "" })
    }
    function cancelPin() {
        LS.adhoc.call("luna://com.webos.notification", "/closePincodePrompt", JSON.stringify({ closeType: "close" }))
        pin = null
    }
    readonly property var padKeys: ["1", "2", "3", "4", "5", "6", "7", "8", "9", "clear", "0", "back"]
    function padPress(k) {
        if (!pin) return
        var typed = pin.typed
        if (k === "clear") typed = ""
        else if (k === "back") typed = typed.substring(0, typed.length - 1)
        else if (typed.length < 4) typed += k
        pin = Object.assign({}, pin, { typed: typed })
    }
    function digit(n) { padPress(String(n)) }

    // ---- keys: the surface on top has them: the PIN prompt over the input popup over the alert (LG's order)
    function key(dir) {
        rested = false
        restTimer.restart()
        if (pinUp) {
            var i = pin.index
            if (dir === "back") { if (pin.typed.length) padPress("back"); else cancelPin() }
            else if (dir === "ok") { if (i >= 12) relayPin(); else padPress(padKeys[i]) }
            else if (dir === "left") i = i % 3 === 0 && i < 12 ? i : i - 1
            else if (dir === "right") i = i % 3 === 2 && i < 12 ? i : (i < 12 ? i + 1 : i)
            else if (dir === "up") i = i >= 12 ? 10 : (i >= 3 ? i - 3 : i)
            else if (dir === "down") i = i < 9 ? i + 3 : 12
            if (pin && i !== pin.index) pin = Object.assign({}, pin, { index: Math.max(0, Math.min(12, i)) })
            return
        }
        if (inputUp) {
            var n = (inputAlert.inputs || []).length + 1
            if (dir === "left") inputAlert = Object.assign({}, inputAlert, { index: Math.max(0, inputAlert.index - 1) })
            else if (dir === "right") inputAlert = Object.assign({}, inputAlert, { index: Math.min(n - 1, inputAlert.index + 1) })
            else if (dir === "ok") { if (inputAlert.index < n - 1) act(inputAlert.inputs[inputAlert.index].action); inputAlert = null }
            else if (dir === "back") inputAlert = null
            return
        }
        if (alertUp) {
            var a = alerts[0], count = (a.buttons || []).length
            if (dir === "left") a.index = Math.max(0, a.index - 1)
            else if (dir === "right") a.index = Math.min(Math.max(0, count - 1), a.index + 1)
            else if (dir === "ok") { answerAlert(a.index); return }
            else if (dir === "back") { dropAlert(a, true); return }
            alerts = alerts.slice()
        }
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: notify.rested = true }

    // ============================================================= drawing
    // toasts: centred at the foot of the screen, 54 px (5 % of the height, the TV-safe margin) above the bottom edge, the newest at the foot
    // with the older ones above it. While the volume bar's card is up (`barTop`, `barBottom`, from the overlay through Shell.qml) and the
    // column would meet it (16 px of air kept), the column sits above the bar; a bar lifted over an app is clear of it and moves nothing.
    property int barTop: -1
    property int barBottom: -1
    readonly property int toastFoot: 1080 - 54
    Column {
        id: toastStack
        readonly property bool barMeets: notify.barTop >= 0 && notify.barTop < notify.toastFoot + 16 && notify.barBottom + 16 > notify.toastFoot - height
        x: Math.round((1920 - width) / 2)
        y: (barMeets ? notify.barTop - 16 : notify.toastFoot) - height
        width: 560
        spacing: Lu.space(2)
        // One slot is kept built even with no toast (hidden), and shown once at the shell's warm-up with a pangram: the first real
        // toast then sets its text on a built item and finds its glyphs in the cache, instead of building a tile and laying out
        // text at nice 10 (61-118 ms from the delivery on the 2026-10-28 gate, 28-52 ms before it).
        Repeater {
            model: Math.max(1, notify.toasts.length)
            delegate: Rectangle {
                readonly property var t: notify.toasts[notify.toasts.length - 1 - index] || notify.warmToast
                visible: index < notify.toasts.length || notify.warming
                width: toastStack.width
                height: toastBody.height + 2 * Lu.space(3)
                radius: Lu.radius("sheet"); color: Lu.sheet; border.width: 1; border.color: Lu.edge
                Row {
                    id: toastBody
                    x: Lu.space(3); y: Lu.space(3); width: parent.width - 2 * Lu.space(3); spacing: Lu.space(3)
                    Image { visible: !!t.iconUrl; width: 48; height: 48; source: t.iconUrl || ""; fillMode: Image.PreserveAspectFit; asynchronous: true; anchors.verticalCenter: parent.verticalCenter }
                    LuTile { visible: !t.iconUrl; size: 48; icon: "info"; accent: true; anchors.verticalCenter: parent.verticalCenter }
                    Column {
                        width: parent.width - 48 - Lu.space(3)
                        spacing: Lu.space(1)
                        anchors.verticalCenter: parent.verticalCenter
                        LuText { width: parent.width; role: "label"; text: t.title; elide: Text.ElideRight; maximumLineCount: 2; wrapMode: Text.WordWrap }
                        LuText { visible: t.body !== ""; width: parent.width; role: "caption"; color: Lu.ink2; text: t.body; elide: Text.ElideRight; maximumLineCount: 2; wrapMode: Text.WordWrap }
                    }
                }
            }
        }
    }

    // the alert: a card in the middle over a scrim (full screen for a warning)
    Item {
        id: alertLayer
        anchors.fill: parent
        visible: notify.alertUp && !notify.inputUp && !notify.pinUp
        readonly property var a: notify.alerts[0] || ({})
        readonly property bool full: a.type === "warning"
        Rectangle { anchors.fill: parent; color: alertLayer.full ? Lu.canvas : Lu.scrim }
        Rectangle {
            color: Lu.sheet; radius: Lu.radius("sheet"); border.width: 1; border.color: Lu.edge
            width: 840
            height: alertBody.height + 2 * Lu.space(6)
            x: Math.round((1920 - width) / 2); y: Math.round((1080 - height) / 2)
            Column {
                id: alertBody
                x: Lu.space(6); y: Lu.space(6); width: parent.width - 2 * Lu.space(6); spacing: Lu.space(4)
                Row {
                    spacing: Lu.space(4)
                    LuTile { size: 56; icon: alertLayer.full ? "triangle-alert" : "bell"; accent: !alertLayer.full; anchors.verticalCenter: parent.verticalCenter }
                    LuText { anchors.verticalCenter: parent.verticalCenter; role: "title"; width: alertBody.width - 56 - Lu.space(4); wrapMode: Text.WordWrap; text: alertLayer.a.title || alertLayer.a.sourceId || "" }
                }
                LuText { width: parent.width; role: "body"; line: 0.99; color: Lu.ink2; wrapMode: Text.WordWrap; text: String(alertLayer.a.message || "").replace(/<br\s*\/?>/gi, "\n") }
                Row {
                    spacing: Lu.space(4)
                    Repeater {
                        model: (alertLayer.a.buttons || []).length
                        delegate: LuButton {
                            readonly property var b: alertLayer.a.buttons[index]
                            text: b.label || "OK"
                            primary: index === alertLayer.a.index && !b.danger
                            danger: !!b.danger
                            focused: alertLayer.a.index === index
                            rested: notify.rested
                            onPointerFocus: { alertLayer.a.index = index; notify.alerts = notify.alerts.slice() }
                            onActivated: notify.answerAlert(index)
                        }
                    }
                    LuButton { visible: !(alertLayer.a.buttons || []).length; text: "OK"; primary: true; focused: true; rested: notify.rested; onActivated: notify.answerAlert(0) }
                }
            }
        }
    }

    // the input popup: a sheet at the toast corner
    Rectangle {
        id: inputSheet
        visible: notify.inputUp && !notify.pinUp
        readonly property var a: notify.inputAlert || ({})
        readonly property var inputs: a.inputs || []
        color: Lu.sheet; radius: Lu.radius("sheet"); border.width: 1; border.color: Lu.edge
        width: 640
        height: inputBody.height + 2 * Lu.space(5)
        x: 1920 - width - Lu.space(6); y: Lu.space(6)
        Column {
            id: inputBody
            x: Lu.space(5); y: Lu.space(5); width: parent.width - 2 * Lu.space(5); spacing: Lu.space(3)
            Row {
                spacing: Lu.space(3)
                Image { visible: !!inputSheet.a.portIcon; width: 56; height: 56; source: inputSheet.a.portIcon ? (String(inputSheet.a.portIcon).charAt(0) === "/" ? "file://" + inputSheet.a.portIcon : inputSheet.a.portIcon) : ""; fillMode: Image.PreserveAspectFit; anchors.verticalCenter: parent.verticalCenter }
                LuTile { visible: !inputSheet.a.portIcon; size: 56; icon: "cable"; accent: true; anchors.verticalCenter: parent.verticalCenter }
                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: Lu.space(1)
                    LuText { role: "title"; text: (inputSheet.a.portName || (inputSheet.inputs[0] && inputSheet.inputs[0].portName) || "A device") + " connected" }
                    LuText { role: "body"; color: Lu.ink2; text: "Switch to it?" }
                }
            }
            Row {
                spacing: Lu.space(3)
                Repeater {
                    model: inputSheet.inputs.length
                    delegate: LuButton {
                        text: inputSheet.inputs.length > 1 ? "Switch to " + (inputSheet.inputs[index].portName || inputSheet.inputs[index].appId) : "Switch"
                        primary: inputSheet.a.index === index
                        focused: inputSheet.a.index === index
                        rested: notify.rested
                        onPointerFocus: notify.inputAlert = Object.assign({}, notify.inputAlert, { index: index })
                        onActivated: { notify.act(inputSheet.inputs[index].action); notify.inputAlert = null }
                    }
                }
                LuButton {
                    text: "Stay here"
                    focused: inputSheet.a.index === inputSheet.inputs.length
                    rested: notify.rested
                    onPointerFocus: notify.inputAlert = Object.assign({}, notify.inputAlert, { index: inputSheet.inputs.length })
                    onActivated: notify.inputAlert = null
                }
            }
        }
    }

    // the PIN prompt: a card over the dimmed screen, the pad and an OK that is live with four digits
    Item {
        id: pinLayer
        anchors.fill: parent
        visible: notify.pinUp
        readonly property var p: notify.pin || ({})
        Rectangle { anchors.fill: parent; color: Lu.scrimHide }
        Rectangle {
            color: Lu.sheet; radius: Lu.radius("sheet"); border.width: 1; border.color: Lu.edge
            width: 560
            height: pinBody.height + 2 * Lu.space(6)
            x: Math.round((1920 - width) / 2); y: Math.round((1080 - height) / 2)
            Column {
                id: pinBody
                x: Lu.space(6); y: Lu.space(6); width: parent.width - 2 * Lu.space(6); spacing: Lu.space(4)
                LuText { width: parent.width; role: "title"; wrapMode: Text.WordWrap; text: pinLayer.p.title || "Enter the PIN" }
                LuText { width: parent.width; role: "body"; color: pinLayer.p.retry ? Lu.dangerInk : Lu.ink2; wrapMode: Text.WordWrap
                         text: pinLayer.p.retry ? "Wrong PIN. Try again." : (pinLayer.p.message || "") }
                Row {
                    spacing: Lu.space(3)
                    anchors.horizontalCenter: parent.horizontalCenter
                    Repeater {
                        model: 4
                        delegate: Rectangle {
                            width: 56; height: 56; radius: Lu.radius("tile")
                            color: index < (pinLayer.p.typed || "").length ? Lu.accent : Lu.glass
                            border.width: 1; border.color: Lu.edge
                        }
                    }
                }
                Grid {
                    columns: 3
                    spacing: Lu.space(2)
                    anchors.horizontalCenter: parent.horizontalCenter
                    Repeater {
                        model: 12
                        delegate: LuButton {
                            readonly property string k: notify.padKeys[index]
                            text: k === "clear" ? "Clear" : k === "back" ? "\u232b" : k
                            focused: pinLayer.p.index === index
                            rested: notify.rested
                            onPointerFocus: notify.pin = Object.assign({}, notify.pin, { index: index })
                            onActivated: notify.padPress(k)
                        }
                    }
                }
                LuButton {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "OK"
                    primary: (pinLayer.p.typed || "").length === 4
                    focused: pinLayer.p.index === 12
                    rested: notify.rested
                    opacity: (pinLayer.p.typed || "").length === 4 ? 1 : Lu.disabledOpacity
                    onPointerFocus: notify.pin = Object.assign({}, notify.pin, { index: 12 })
                    onActivated: notify.relayPin()
                }
            }
        }
    }
}
