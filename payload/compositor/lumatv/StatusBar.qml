import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0

// Status bar: a 60px glass pill (padding 0 10 0 6, 6px between buttons) with items built from
// layout.statusBar.left/right through a small type registry. Unknown types render a generic
// glyph so a bad layout entry never breaks the bar. "profile" and "notifications" open a
// glass popover under their button (rows navigable with Up/Down, OK activates, Back closes);
// "settings" and "search" launch their apps. Profile data comes from the LG account
// (accountmanager/getLoginUserData with serviceName LGE): nickname, initial, avatar colour.
Item {
    id: bar

    property var theme: ({})
    property var mat: ({ ink: "#F2EFF6", ink2: "#A8F2EFF6", glass: "#66120E1A", glassEdge: "#21FFFFFF" })
    property bool isLight: false
    property var items: []          // merged left + right, in visual order
    property int leftCount: 1
    property bool focusedLayer: false
    property int focusedIndex: 0
    property Item backdrop: null
    property int cardMs: 180
    property int cardEasing: Easing.OutCubic
    property int layerMs: 220
    property string profileName: account.nick
    property string profileInitial: account.initial
    property color profileColor: account.bg

    signal requestOpenPopover(int index)
    signal requestClosePopover()
    signal hoverFocus(int index)
    signal themePatch(var patch)
    signal themePersist(var patch)

    function activate(index) {
        var type = items[index] ? items[index].type : ""
        if (type === "notifications" || type === "profile" || type === "settings") { bar.requestOpenPopover(index); return }
        if (type === "search") LS.adhoc.call("luna://com.webos.applicationManager", "/launch", JSON.stringify({ id: "com.webos.app.voice" }))
    }

    function itemAt(index) {
        return index < leftCount ? leftRepeater.itemAt(index) : rightRepeater.itemAt(index - leftCount)
    }
    function focusedGlobalX() {
        var it = itemAt(focusedIndex)
        return it ? bar.x + it.parent.x + it.x + 24 : bar.x + bar.width / 2
    }

    Glass {
        mat: bar.mat
        isLight: bar.isLight
        cardRadius: bar.height / 2
        focusMix: 0
        screenX: bar.x; screenY: bar.y; screenW: bar.width; screenH: bar.height
        backdrop: bar.backdrop
    }

    Row {
        id: leftRow
        x: 6; y: 6
        spacing: 6
        Repeater {
            id: leftRepeater
            model: bar.items.slice(0, bar.leftCount)
            delegate: StatusItem {
                kind: modelData.type
                label: modelData.type === "profile" ? bar.profileName : ""
                initial: bar.profileInitial
                avatarColor: bar.profileColor
                badge: modelData.type === "notifications" ? notifSvc.badgeCount : 0
                mat: bar.mat
                isLight: bar.isLight
                iconSource: modelData.icon ? modelData.icon : ""
                focused: bar.focusedLayer && bar.focusedIndex === index
                backdrop: bar.backdrop
                screenX: bar.x + leftRow.x + x; screenY: bar.y + leftRow.y
                onActivated: bar.activate(index)
                onHoverFocus: bar.hoverFocus(index)
            }
        }
    }

    Row {
        id: rightRow
        x: bar.width - 10 - width; y: 6
        spacing: 6
        Repeater {
            id: rightRepeater
            model: bar.items.slice(bar.leftCount)
            delegate: StatusItem {
                kind: modelData.type
                label: modelData.type === "profile" ? bar.profileName : ""
                initial: bar.profileInitial
                avatarColor: bar.profileColor
                badge: modelData.type === "notifications" ? notifSvc.badgeCount : 0
                mat: bar.mat
                isLight: bar.isLight
                iconSource: modelData.icon ? modelData.icon : ""
                focused: bar.focusedLayer && bar.focusedIndex === (index + bar.leftCount)
                backdrop: bar.backdrop
                screenX: bar.x + rightRow.x + x; screenY: bar.y + rightRow.y
                onActivated: bar.activate(index + bar.leftCount)
                onHoverFocus: bar.hoverFocus(index + bar.leftCount)
            }
        }
    }

    // LG account: nickname, initial and avatar colour as Home shows them. Subscribed, so a
    // sign-in or nickname change lands without a restart.
    Service {
        id: account
        appId: LS.appId
        property string nick: ""
        property string initial: ""
        property color bg: "#7360E7"
        property string email: ""
        property bool signedIn: false
        onResponse: {
            try {
                var msg = JSON.parse(payload)
                var u = msg.userData
                if (!u) return
                signedIn = !!u.isLogin
                nick = u.profileNick || ""
                initial = u.iconNick || (nick ? nick.charAt(0).toUpperCase() : "")
                email = u.id || ""
                if (u.profileBg) bg = u.profileBg
            } catch (e) {}
        }
        Component.onCompleted: call("luna://com.webos.service.accountmanager", "/getLoginUserData", JSON.stringify({ serviceName: "LGE", subscribe: true }))
    }

    // Toasts are subscribe-only on this firmware (no backlog), so the badge counts what has
    // arrived since Home was drawn.
    Service {
        id: notifSvc
        appId: LS.appId
        property var recent: []
        property int badgeCount: recent.length
        onResponse: {
            try {
                var msg = JSON.parse(payload)
                if (msg && msg.message) recent = [{ text: msg.message, sourceId: msg.sourceId || "" }].concat(recent).slice(0, 5)
            } catch (e) {}
        }
        Component.onCompleted: call("luna://com.webos.notification", "/getToastNotification", JSON.stringify({ subscribe: true }))
    }

    // ---- popover: one glass panel under the focused button; content per item type ----
    property bool popoverOpen: false
    property string popoverType: ""
    property real popoverScrim: isLight ? 0.22 : 0.30
    property int popoverIndex: 0
    property var wm: theme.wallpaperMotion || {}
    property var popoverRows: popoverType === "profile"
        ? [ { label: account.signedIn ? "Manage LG account" : "Sign in to LG account", action: "membership" } ]
        : popoverType === "settings"
        ? [ { label: "Wallpaper motion", kind: "toggle", key: "enabled", value: wm.enabled !== false },
            { label: "Amount", kind: "slider", key: "amplitude", value: wm.amplitude === undefined ? 4 : wm.amplitude, min: 0, max: 16, step: 1, unit: " px" },
            { label: "Speed", kind: "slider", key: "speed", value: wm.speed === undefined ? 1 : wm.speed, min: 0.2, max: 3, step: 0.1, unit: "x" },
            { label: "Depth", kind: "slider", key: "depth", value: wm.depth === undefined ? 0.25 : wm.depth, min: 0, max: 1, step: 0.05, unit: "" },
            { label: "Wave size", kind: "slider", key: "scale", value: wm.scale === undefined ? 1 : wm.scale, min: 0.5, max: 2.5, step: 0.1, unit: "x" },
            { label: "Detail", kind: "slider", key: "detail", value: wm.detail === undefined ? 0.35 : wm.detail, min: 0, max: 1, step: 0.05, unit: "" },
            { label: "Frame rate", kind: "slider", key: "fps", value: wm.fps === undefined ? 30 : wm.fps, min: 15, max: 60, step: 5, unit: " fps" },
            { label: "Vignette", kind: "slider", key: "vignette", value: wm.vignette || 0, min: 0, max: 1, step: 0.05, unit: "" },
            { label: "Specular", kind: "slider", key: "specular", value: wm.specular || 0, min: 0, max: 1, step: 0.05, unit: "" },
            { label: "Bloom", kind: "slider", key: "bloom", value: wm.bloom || 0, min: 0, max: 1, step: 0.05, unit: "" },
            { label: "Saturation", kind: "slider", key: "saturation", value: wm.saturation === undefined ? 1 : wm.saturation, min: 0.5, max: 1.5, step: 0.05, unit: "" },
            { label: "Contrast", kind: "slider", key: "contrast", value: wm.contrast === undefined ? 1 : wm.contrast, min: 0.8, max: 1.3, step: 0.05, unit: "" },
            { label: "TV settings", action: "tvsettings" } ]
        : []
    property var dirty: ({})
    function popoverAdjust(dir) {
        var r = popoverRows[popoverIndex]
        if (!r || !r.kind) return
        var v
        if (r.kind === "toggle") v = !r.value
        else { v = Math.round((r.value + dir * r.step) / r.step) * r.step; v = Math.max(r.min, Math.min(r.max, Math.round(v * 100) / 100)) }
        var patch = {}; patch[r.key] = v
        dirty[r.key] = v
        bar.themePatch({ wallpaperMotion: patch })
    }
    function fmt(r) {
        if (r.kind === "toggle") return r.value ? "On" : "Off"
        var n = Number(r.value); if (isNaN(n)) n = 0
        var v = r.step >= 1 ? Math.round(n) : n.toFixed(r.step >= 0.1 ? 1 : 2)
        return v + r.unit
    }
    onRequestOpenPopover: { popoverType = items[index] ? items[index].type : ""; popoverIndex = 0; dirty = {}; popoverOpen = true }
    onRequestClosePopover: {
        popoverOpen = false
        var keys = Object.keys(dirty)
        if (keys.length) { bar.themePersist({ wallpaperMotion: dirty }); dirty = {} }
    }
    function popoverMove(d) {
        if (!popoverRows.length) return
        popoverIndex = Math.max(0, Math.min(popoverRows.length - 1, popoverIndex + d))
    }
    function popoverActivate() {
        var r = popoverRows[popoverIndex]
        if (!r) return
        if (r.kind === "toggle") { popoverAdjust(1); return }
        if (r.kind === "slider") return
        if (r.action === "membership") LS.adhoc.call("luna://com.webos.applicationManager", "/launch", JSON.stringify({ id: "com.webos.app.membership" }))
        if (r.action === "tvsettings") LS.adhoc.call("luna://com.webos.applicationManager", "/launch", JSON.stringify({ id: "com.palm.app.settings" }))
        bar.requestClosePopover()
    }

    Item {
        id: popover
        width: bar.popoverType === "settings" ? 440 : 380
        height: content.height + 40
        y: bar.height + 8 + (bar.popoverOpen ? 0 : -8)
        x: {
            // re-evaluated on every open: itemAt() is a function, not a dependency, and the
            // Repeater's items do not exist when this binding first runs
            var open = bar.popoverOpen, idx = bar.focusedIndex
            var it = bar.itemAt(idx)
            if (!it) return Math.round((bar.width - width) / 2)
            var left = it.parent.x + it.x
            // left-side items hang from their button's left edge, right-side from the right edge
            var x = idx < bar.leftCount ? left : left + 48 - width
            return Math.round(Math.min(bar.width - width, Math.max(0, x)))
        }
        opacity: bar.popoverOpen ? 1 : 0
        visible: opacity > 0.01
        Behavior on opacity { NumberAnimation { duration: bar.layerMs; easing.type: Easing.OutCubic } }
        Behavior on y { NumberAnimation { duration: bar.layerMs; easing.type: Easing.OutCubic } }
        Glass {
            mat: bar.mat
            isLight: bar.isLight
            cardRadius: 18
            focusMix: 0
            screenX: bar.x + popover.x; screenY: bar.y + popover.y; screenW: popover.width; screenH: popover.height
            backdrop: bar.backdrop
        }
        Item {
            id: content
            x: 20; y: 20
            width: parent.width - 40
            height: bar.popoverType === "profile" ? profileBody.height : bar.popoverType === "settings" ? settingsBody.height : notifBody.height

            // -- settings: value rows; Left/Right adjust, the change shows on Home behind at once
            Item {
                id: settingsBody
                visible: bar.popoverType === "settings"
                width: parent.width
                height: 34 + bar.popoverRows.length * 46
                Text {
                    x: 0; y: 0
                    text: "Wallpaper"
                    color: bar.mat.ink
                    font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 22
                    renderType: Text.NativeRendering
                }
                Repeater {
                    model: bar.popoverType === "settings" ? bar.popoverRows : []
                    delegate: Item {
                        x: 0; y: 34 + index * 46
                        width: settingsBody.width; height: 40
                        Rectangle {
                            anchors.fill: parent
                            radius: 12
                            antialiasing: true
                            color: bar.isLight ? "#8cffffff" : "#12ffffff"
                            border.width: 1
                            border.color: bar.isLight ? "#e6ffffff" : "#14ffffff"
                            opacity: bar.popoverIndex === index ? 1 : 0
                            Behavior on opacity { NumberAnimation { duration: 120 } }
                        }
                        Text {
                            x: 14; y: Math.round((44 - implicitHeight) / 2)
                            text: modelData.label
                            color: bar.popoverIndex === index ? bar.mat.ink : bar.mat.ink2
                            font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 20
                            renderType: Text.NativeRendering
                        }
                        Text {
                            visible: !!modelData.kind
                            x: parent.width - 14 - implicitWidth; y: Math.round((44 - implicitHeight) / 2)
                            text: (bar.popoverIndex === index && modelData.kind === "slider" ? "\u2039 " : "") + bar.fmt(modelData) + (bar.popoverIndex === index && modelData.kind === "slider" ? " \u203a" : "")
                            color: bar.mat.ink
                            font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 20
                            renderType: Text.NativeRendering
                        }
                        Text {
                            visible: !modelData.kind
                            x: parent.width - 14 - implicitWidth; y: Math.round((44 - implicitHeight) / 2)
                            text: "\u203a"
                            color: bar.mat.ink2
                            font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 22
                            renderType: Text.NativeRendering
                        }
                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            onEntered: bar.popoverIndex = index
                            onClicked: { bar.popoverIndex = index; if (modelData.kind === "slider") bar.popoverAdjust(1); else bar.popoverActivate() }
                        }
                    }
                }
            }

            // -- profile: avatar, nickname, email, then action rows
            Item {
                id: profileBody
                visible: bar.popoverType === "profile"
                width: parent.width
                height: 64 + 12 + bar.popoverRows.length * 52
                Rectangle {
                    x: 0; y: 0; width: 64; height: 64; radius: 32
                    antialiasing: true
                    gradient: Gradient {
                        GradientStop { position: 0.0; color: Qt.lighter(account.bg, 1.25) }
                        GradientStop { position: 1.0; color: Qt.darker(account.bg, 1.15) }
                    }
                    Text {
                        x: Math.round((64 - implicitWidth) / 2); y: Math.round((64 - implicitHeight) / 2)
                        text: account.initial || "?"
                        color: "#ffffff"
                        font.family: "Manrope"; font.weight: Font.Bold; font.pixelSize: 28
                        renderType: Text.NativeRendering
                    }
                }
                Text {
                    x: 80; y: 8
                    width: parent.width - 80
                    elide: Text.ElideRight
                    text: account.signedIn ? (account.nick || "LG account") : "Not signed in"
                    color: bar.mat.ink
                    font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 24
                    renderType: Text.NativeRendering
                }
                Text {
                    x: 80; y: 38
                    width: parent.width - 80
                    elide: Text.ElideRight
                    text: account.email
                    color: bar.mat.ink2
                    font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 18
                    renderType: Text.NativeRendering
                }
                Repeater {
                    model: bar.popoverRows
                    delegate: Item {
                        x: 0; y: 76 + index * 52
                        width: profileBody.width; height: 44
                        Rectangle {
                            anchors.fill: parent
                            radius: 12
                            antialiasing: true
                            color: bar.isLight ? "#8cffffff" : "#12ffffff"
                            border.width: 1
                            border.color: bar.isLight ? "#e6ffffff" : "#14ffffff"
                            opacity: bar.popoverIndex === index ? 1 : 0
                            Behavior on opacity { NumberAnimation { duration: 120 } }
                        }
                        Text {
                            x: 14; y: Math.round((44 - implicitHeight) / 2)
                            text: modelData.label
                            color: bar.popoverIndex === index ? bar.mat.ink : bar.mat.ink2
                            font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 20
                            renderType: Text.NativeRendering
                        }
                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            onEntered: bar.popoverIndex = index
                            onClicked: { bar.popoverIndex = index; bar.popoverActivate() }
                        }
                    }
                }
            }

            // -- notifications
            Column {
                id: notifBody
                visible: bar.popoverType === "notifications"
                width: parent.width
                spacing: 12
                Text {
                    text: "Notifications"
                    color: bar.mat.ink
                    font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 22
                    renderType: Text.NativeRendering
                }
                Text {
                    visible: notifSvc.recent.length === 0
                    text: "No notifications"
                    color: bar.mat.ink2
                    font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 20
                    renderType: Text.NativeRendering
                }
                Repeater {
                    model: notifSvc.recent
                    delegate: Text {
                        width: notifBody.width
                        text: modelData.text
                        color: bar.mat.ink
                        wrapMode: Text.WordWrap
                        font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 20
                        renderType: Text.NativeRendering
                    }
                }
            }
        }
    }
}
