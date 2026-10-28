import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0
import "../kit"

// Recent apps (docs/mocks/14-recents): a row of frames over the dimmed Home, one per app LG's recents list holds, the newest first and
// focused. OK switches to the app, a held OK closes it (the app first, then its card: removing the card alone leaves the app running on
// webOS 10, PLAN, Glasshouse), Back returns. A frame is the app's last snapshot when LG has one (`cardSnapShotFilePath`, taken every
// 120 s while an app is in front) or a plate with the app's own picture; under the name is when it was last used, only for an app whose
// snapshot says (its file name carries the time), so no card shows a time nobody read. No "Playing" mark: nothing here knows what plays.
//
// The list is LG's `com.webos.service.recent/listRecents`, read whenever Home comes to the front and again as the frames open, so Back on
// Home knows before it is pressed whether there is anything to show (`count`). LumaHome opens it with Back when the owner turned it on
// (theme `remote.recents`, off until the TV check) and there is at least one app; with none, Back is LG's. Home itself and LumaTV's own
// helper app are never listed. Over four apps the row moves by frames, so the focused one is always inside.
Item {
    id: rc
    anchors.fill: parent
    readonly property bool built: true
    property bool presented: false
    property bool rested: true
    property var items: []                     // [{ appId, title, when, thumb, iconUrl }], newest first
    property int index: 0
    readonly property int count: items.length
    readonly property int cardW: 384
    readonly property int gap: 22
    readonly property int windowCards: 4
    property int closing: -1                   // the frame whose app was just closed: it fades while the list is read again
    signal activated(string appId)
    signal closeRequested()

    // LG's list is read only while the switch is on: with it off this surface asks the system for nothing
    readonly property bool wanted: !!Lu.theme && !!Lu.theme.remote && Lu.theme.remote.recents === true
    onWantedChanged: { if (wanted) read() }
    function read() { if (wanted) svc.callService({}) }
    function parse(payload) {
        var doc = null
        try { doc = JSON.parse(payload) } catch (e) { return }
        if (!doc || doc.returnValue === false) return
        var list = doc.recentsAppList || doc.recents || []
        var out = []
        for (var i = 0; i < list.length; i++) {
            var r = list[i]
            if (!r || !r.appId || r.appId === "com.webos.app.home" || /^org\.nphil\.luma/.test(r.appId)) continue
            var a = Apps.byId(r.appId)
            var snap = String(r.cardSnapShotFilePath || "")
            out.push({ appId: r.appId, title: (a && a.title) || r.title || r.appId, when: whenOf(snap, r.subtitle),
                       thumb: snap !== "" ? "file://" + snap : "", iconUrl: a ? a.iconUrl : "" })
        }
        items = out
        closing = -1
        if (index >= out.length) index = Math.max(0, out.length - 1)
    }
    // "_Sun_Oct_25_19_40_17_2026.JPEG" is when LG took the snapshot (the TV's own clock); the app's subtitle is LG's if it gave one
    function whenOf(path, subtitle) {
        var m = /_([A-Z][a-z]{2})_([A-Z][a-z]{2})_(\d{1,2})_(\d{2})_(\d{2})_(\d{2})_(\d{4})\.\w+$/.exec(path)
        if (!m) return String(subtitle || "")
        var mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].indexOf(m[2])
        if (mon < 0) return String(subtitle || "")
        var then = new Date(Number(m[7]), mon, Number(m[3]), Number(m[4]), Number(m[5]), Number(m[6]))
        return ago((Date.now() - then.getTime()) / 60000)
    }
    function ago(min) {
        if (!(min >= 0)) return ""
        if (min < 1) return "Just now"
        if (min < 60) return Math.round(min) + " min ago"
        if (min < 1440) return Math.round(min / 60) + " hr ago"
        return min < 2880 ? "Yesterday" : Math.floor(min / 1440) + " days ago"
    }

    onPresentedChanged: { if (presented) { index = 0; rested = true; read() } }
    Component.onCompleted: read()
    Service { id: svc; appId: "com.webos.surfacemanager"; service: "luna://com.webos.service.recent"; method: "listRecents"; onResponse: rc.parse(payload) }

    function key(dir) {
        if (dir === "left") index = Math.max(0, index - 1)
        else if (dir === "right") index = Math.min(count - 1, index + 1)
        else if (dir === "ok") { if (items[index]) activated(items[index].appId) }
        else if (dir === "back") closeRequested()
        rested = false
        restTimer.restart()
    }
    // A held OK closes the app of the focused frame, then its entry; the list is read again for what is left.
    function hold() { if (items[index]) closeApp(index) }
    function holdReady() { return !!items[index] }
    function closeApp(i) {
        var id = items[i].appId
        closing = i
        LS.adhoc.call("luna://com.webos.applicationManager", "/closeByAppId", JSON.stringify({ id: id }))
        LS.adhoc.call("luna://com.webos.service.recent", "/removeRecent", JSON.stringify({ appId: id }))
        again.restart()
    }
    Timer { id: again; interval: 600; onTriggered: rc.read() }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: rc.rested = true }

    readonly property color pill: Qt.rgba(Lu.scrimHide.r, Lu.scrimHide.g, Lu.scrimHide.b, 0.72)

    // Home stays behind, dimmed to where only its shape is left (the mock's scrim of .74). Dark ink on a pale veil reads through it at the same
    // strength much more than light on a dark one, so the light veil is denser: the page's own text no longer runs into the header.
    Rectangle { anchors.fill: parent; color: Qt.rgba(Lu.scrimHide.r, Lu.scrimHide.g, Lu.scrimHide.b, Lu.isLight ? 0.9 : 0.74) }

    Column {
        anchors.centerIn: parent
        spacing: Lu.space(7)
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: Lu.space(4)
            LuText { anchors.verticalCenter: parent.verticalCenter; role: "title"; text: "Recent apps" }
            LuText { anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink2; text: rc.count === 0 ? "Nothing open" : rc.count + " open" }
        }
        // the frames: a window of four, moved by frames; the padding above and below is room for the focused one to lift
        Item {
            id: windowItem
            anchors.horizontalCenter: parent.horizontalCenter
            readonly property int shown: Math.min(rc.count, rc.windowCards)
            width: Math.max(rc.cardW, shown * rc.cardW + (shown - 1) * rc.gap)
            height: 340
            clip: true
            readonly property int first: Math.max(0, Math.min(rc.count - rc.windowCards, rc.index - (rc.windowCards - 1)))
            Row {
                id: strip
                y: 28
                spacing: rc.gap
                x: -windowItem.first * (rc.cardW + rc.gap)
                Behavior on x { enabled: !Lu.reduced; NumberAnimation { duration: Lu.ms("scrollMs"); easing.type: Lu.easing(Lu.motion.scrollEasing) } }
                Repeater {
                    model: rc.count
                    delegate: Item {
                        id: card
                        readonly property var r: rc.items[index]
                        readonly property bool isFocus: rc.presented && rc.index === index
                        width: rc.cardW
                        height: body.height + 20
                        opacity: rc.closing === index ? 0.3 : 1
                        FocusLift { id: lift; focused: card.isFocus; rested: rc.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                        LuHalo { anchors.centerIn: parent; width: parent.width * 1.15; height: parent.height * 1.25; z: -1; on: card.isFocus }
                        Item {
                            anchors.fill: parent
                            transform: [ Translate { y: -10 * lift.value }, Scale { origin.x: card.width / 2; origin.y: card.height / 2; xScale: 1 + 0.06 * lift.value; yScale: 1 + 0.06 * lift.value } ]
                            Rectangle {
                                anchors.fill: parent
                                radius: Lu.radius("card")
                                color: card.isFocus ? Lu.glassRaised : Lu.sheet
                                border.width: 1
                                border.color: card.isFocus ? Lu.edgeRaised : Lu.edge
                                antialiasing: true
                            }
                            Column {
                                id: body
                                x: 10; y: 10
                                spacing: Lu.space(3)
                                Item {
                                    width: 364; height: 205
                                    // the plate is always there; a snapshot LG still has on file covers it
                                    Rectangle {
                                        anchors.fill: parent
                                        radius: 20
                                        gradient: Gradient {
                                            GradientStop { position: 0; color: Lu.glassRaised }
                                            GradientStop { position: 1; color: Lu.scrimHide }
                                        }
                                        LuAppIcon { anchors.centerIn: parent; width: 88; height: 88; radius: 20; source: card.r ? card.r.iconUrl : "" }
                                    }
                                    LuAppIcon { anchors.fill: parent; visible: !!card.r && card.r.thumb !== ""; placeholder: false; radius: 20; decode: 512; source: card.r ? card.r.thumb : "" }
                                }
                                Row {
                                    x: 6
                                    width: 364 - 12
                                    height: 52
                                    spacing: Lu.space(3)
                                    LuAppIcon { anchors.verticalCenter: parent.verticalCenter; width: 40; height: 40; radius: 10; source: card.r ? card.r.iconUrl : "" }
                                    Column {
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: parent.width - 40 - 44 - 2 * Lu.space(3)
                                        LuText { width: parent.width; role: "label"; font.weight: Font.DemiBold; elide: Text.ElideRight; text: card.r ? card.r.title : "" }
                                        LuText { width: parent.width; visible: !!card.r && card.r.when !== ""; role: "caption"; color: Lu.ink2; elide: Text.ElideRight; text: card.r ? card.r.when : "" }
                                    }
                                    Rectangle {
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 44; height: 44; radius: 12
                                        color: Lu.wash("selected")
                                        LuIcon { anchors.centerIn: parent; name: "x"; size: 24; color: Lu.ink2 }
                                        MouseArea { anchors.fill: parent; onClicked: rc.closeApp(index) }
                                    }
                                }
                            }
                            MouseArea {
                                anchors.fill: parent
                                z: -1
                                hoverEnabled: true
                                onEntered: rc.index = index
                                onClicked: { rc.index = index; rc.key("ok") }
                            }
                        }
                    }
                }
            }
        }
        // what the keys do
        Rectangle {
            anchors.horizontalCenter: parent.horizontalCenter
            width: hints.width + 2 * Lu.space(5)
            height: Math.round(Lu.target * 0.8)
            radius: height / 2
            color: rc.pill
            Row {
                id: hints
                anchors.centerIn: parent
                spacing: Lu.space(7)
                Repeater {
                    model: [{ k: "OK", t: "Switch to it" }, { k: "Hold OK", t: "Close the app" }, { k: "Back", t: "Return" }]
                    delegate: Row {
                        spacing: Lu.space(2)
                        Rectangle {
                            anchors.verticalCenter: parent.verticalCenter
                            width: keyText.implicitWidth + 2 * Lu.space(2); height: keyText.implicitHeight + Lu.space(1)
                            radius: 8; color: "transparent"; border.width: 1; border.color: Lu.edge
                            LuText { id: keyText; anchors.centerIn: parent; role: "caption"; font.weight: Font.DemiBold; color: Lu.ink2; text: modelData.k }
                        }
                        LuText { anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink2; text: modelData.t }
                    }
                }
            }
        }
    }
}
