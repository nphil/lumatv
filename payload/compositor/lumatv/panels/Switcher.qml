import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0
import "../kit"
import "SwitcherLayout.js" as Geo

// The app switcher (docs/mocks/switcher), the one list of what is open with two doors: a second press of Home (the compositor opens it) and
// Back from the top of Home (LumaHome asks). Over LumaTV's compositor each running app is a live picture of its own window, which the
// compositor draws under this surface where `placed()` says (src/switcher.rs); this surface draws the rest: the names, the focus, Quit,
// Quit all, and for an app whose picture is on the video plane (no picture is taken of it) its icon on a blurred tile. Nothing here covers a
// picture but the focus ring, so the pictures show through a window that is otherwise clear. Under LG's compositor (LumaTV's module as LG's
// shadow module) there is no compositor to ask: the list is LG's recents, the pictures are LG's snapshots and the buttons call the app manager.
//
// Keys: Left and Right move between the cards, Down goes to a card's own Quit, Up to the header's Quit all; OK opens the card, quits, or asks;
// Back (and the compositor's Home) closes. A pointer moves the focus by hovering and acts by clicking. Quit all asks first.
//
// `feed` is the compositor's {apps, open, from, n} (compositor/shell/Shell.qml binds it; null under LG's compositor) and `post(op)` goes back
// to it. The cards' places are sent whenever the list or the window of cards changes, as one message the compositor moves its pictures to.
Item {
    id: sw
    anchors.fill: parent
    readonly property bool built: true

    // ---- who feeds it
    property var feed: null
    readonly property bool own: feed !== null
    property bool presented: false                  // Resident: wanted on screen (the compositor says so, or the overlay's door)
    signal post(var op)
    signal closeRequested()
    signal activated(string appId)

    // ---- the list: [{ id, title, iconUrl, picture: live | video | none | snap, thumb, state }]
    property var lgRecents: []
    readonly property var entries: own ? fromCompositor(feed.apps || []) : lgRecents
    readonly property int count: entries.length
    function fromCompositor(list) {
        var out = []
        for (var i = 0; i < list.length; i++) {
            var a = list[i], app = Apps.byId(a.id)
            out.push({ id: a.id, title: (app && app.title) || a.title || a.id, iconUrl: app ? app.iconUrl : "", picture: a.picture || "none", thumb: "", state: a.state || "" })
        }
        return out
    }

    // ---- the focus: a card, its Quit, or the header's Quit all
    property string level: "card"
    property int index: 0
    property int first: 0
    property bool asking: false                     // Quit all is being asked
    property int askIndex: 0                        // 0 Cancel, 1 Quit all
    readonly property bool moving: slide.running
    onCountChanged: {
        if (index >= count) index = Math.max(0, count - 1)
        first = Geo.windowStart(count, index, first)
        if (count === 0) { level = "card"; asking = false }
        if (presented) slide.restart()
        relayout.restart()
    }
    onEntriesChanged: relayout.restart()
    onIndexChanged: { first = Geo.windowStart(count, index, first); relayout.restart() }
    onFirstChanged: relayout.restart()
    onPresentedChanged: {
        if (presented) { index = 0; first = 0; level = "card"; asking = false; askIndex = 0; idle.restart(); slide.restart(); if (!own) readLg(); sendLayout(); relayout.restart() }
        else idle.stop()
        sweep.stop()
        sweep.to = presented ? 1 : 0
        sweep.duration = Lu.reduced ? Lu.ms("labelMs") : (presented ? 160 : 120)
        sweep.easing.type = presented ? Easing.OutCubic : Easing.InCubic
        sweep.start()
    }

    // ---- showing and leaving: the chrome (names, buttons) fades; the pictures are the compositor's
    property real shown: 0
    readonly property bool leaving: !presented && shown > 0
    readonly property bool held: false
    NumberAnimation { id: sweep; target: sw; property: "shown" }
    Timer { id: slide; interval: Lu.reduced ? 1 : 200 }          // how long the row is on the move: the ring waits for it

    // ---- what the shell tells the compositor
    property string lastLayout: ""
    function layoutDoc() {
        var s = Lu.scrimHide
        return { op: "layout", cards: Geo.placed(entries, first), scrim: { r: s.r, g: s.g, b: s.b, a: s.a }, ms: Lu.reduced ? 120 : 200, reduced: Lu.reduced }
    }
    function sendLayout() {
        if (!own || !presented) return
        var doc = layoutDoc(), text = JSON.stringify(doc)
        if (text === lastLayout) return
        lastLayout = text
        post(doc)
    }
    // one message for the changes of one turn of the event loop (the list and the focus move together)
    Timer { id: relayout; interval: 0; onTriggered: sw.sendLayout() }
    onOwnChanged: lastLayout = ""
    onFeedChanged: if (feed && feed.n !== undefined && feed.n !== seenOpening) { seenOpening = feed.n; lastLayout = "" }
    property int seenOpening: -1

    // ---- LG's recents, where there is no compositor (the old Recents panel's list)
    readonly property bool lgWanted: !own && !!Lu.theme && !!Lu.theme.remote && (Lu.theme.remote.recents === true || Lu.theme.remote.appSwitcher === true)
    onLgWantedChanged: { if (lgWanted) readLg() }
    function readLg() { if (lgWanted) svc.callService({}) }
    function parseLg(payload) {
        var doc = null
        try { doc = JSON.parse(payload) } catch (e) { return }
        if (!doc || doc.returnValue === false) return
        var list = doc.recentsAppList || doc.recents || []
        var out = []
        for (var i = 0; i < list.length; i++) {
            var r = list[i]
            if (!r || !r.appId || r.appId === "com.webos.app.home" || /^org\.nphil\.luma/.test(r.appId)) continue
            var a = Apps.byId(r.appId), snap = String(r.cardSnapShotFilePath || "")
            out.push({ id: r.appId, title: (a && a.title) || r.title || r.appId, iconUrl: a ? a.iconUrl : "", picture: snap !== "" ? "snap" : "none", thumb: snap !== "" ? "file://" + snap : "", state: "" })
        }
        lgRecents = out
    }
    Service { id: svc; appId: "com.webos.surfacemanager"; service: "luna://com.webos.service.recent"; method: "listRecents"; onResponse: sw.parseLg(payload) }
    Component.onCompleted: readLg()
    Timer { id: again; interval: 600; onTriggered: sw.readLg() }

    // ---- what a choice does
    function activate(i) {
        var e = entries[i]
        if (!e || e.state === "quitting") return
        if (own) post({ op: "activate", id: e.id })
        else { LS.adhoc.call("luna://com.webos.applicationManager", "/launch", JSON.stringify({ id: e.id })); activated(e.id); closeRequested() }
    }
    function quit(i) {
        var e = entries[i]
        if (!e || e.state === "quitting") return
        if (own) post({ op: "quit", id: e.id })
        else {
            // LG's own order (the app first, then its entry): removing the entry alone leaves the app running on webOS 10
            LS.adhoc.call("luna://com.webos.applicationManager", "/closeByAppId", JSON.stringify({ id: e.id, reason: "windowPolicy" }))
            LS.adhoc.call("luna://com.webos.service.recent", "/removeRecent", JSON.stringify({ appId: e.id }))
            again.restart()
        }
        level = "card"
    }
    function quitAll() {
        asking = false
        if (own) post({ op: "quitall" })
        else for (var i = entries.length - 1; i >= 0; i--) quit(i)
        level = "card"
    }
    function close() {
        if (own) post({ op: "close" })
        else closeRequested()
    }

    // ---- keys (the overlay hands them over while this is up)
    function key(dir) {
        idle.restart()
        if (asking) {
            if (dir === "left" || dir === "right") askIndex = askIndex === 0 ? 1 : 0
            else if (dir === "back") asking = false
            else if (dir === "ok") { if (askIndex === 1) quitAll(); else asking = false }
            return
        }
        if (dir === "back") { close(); return }
        if (count === 0) { if (dir === "ok") close(); return }
        if (dir === "ok") {
            if (level === "all") { asking = true; askIndex = 0 }
            else if (level === "quit") quit(index)
            else activate(index)
            return
        }
        var before = first
        var at = Geo.move({ level: level, index: index }, dir, count)
        level = at.level
        index = at.index
        if (first !== before) slide.restart()
    }
    // a pointer: hovering moves the focus and a click acts, outside the cards a click is Back
    function hover(i, l) { if (asking) return; idle.restart(); level = l; index = i }

    // A surface that takes the keys and gets none for this long closes: a remote cannot stay behind it for ever (and a still picture of an app
    // is not left on a panel that is burning in).
    Timer { id: idle; interval: 45000; onTriggered: sw.close() }

    // ---- the chrome
    // A button's edge when it has no focus: the ink at .42 (dark) or .55 (light) over the dim reads at 3:1 or more, which the palette's own `edge` (white
    // on white in the light material) does not
    readonly property color boundary: Qt.rgba(Lu.ink.r, Lu.ink.g, Lu.ink.b, Lu.isLight ? 0.55 : 0.42)
    Item {
        id: chrome
        anchors.fill: parent
        opacity: sw.shown
        visible: sw.shown > 0

        MouseArea { anchors.fill: parent; onClicked: sw.key("back") }

        // header
        Item {
            id: head
            x: 96; y: 54; width: 1728; height: Math.max(Lu.target, titleText.implicitHeight)
            LuText { id: titleText; anchors.verticalCenter: parent.verticalCenter; role: "title"; font.weight: Font.DemiBold; text: "Open apps" }
            LuText {
                id: countText
                x: titleText.width + Lu.space(4); y: titleText.y + titleText.baselineOffset - baselineOffset
                role: "caption"; color: Lu.ink2; visible: sw.count > 0; text: sw.count + " open"
            }
            Rectangle {
                id: quitAllButton
                visible: sw.count > 0
                anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
                width: allRow.width + 2 * Lu.space(6); height: Math.max(Lu.target, allRow.height + Lu.space(3))
                radius: Lu.radius("control")
                color: sw.level === "all" ? Lu.glassRaised : Lu.sheet
                border.width: 2; border.color: sw.level === "all" ? Lu.ink : sw.boundary
                Row {
                    id: allRow
                    anchors.centerIn: parent
                    spacing: Lu.space(3)
                    LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "x"; size: 28; color: Lu.ink }
                    LuText { anchors.verticalCenter: parent.verticalCenter; role: "label"; font.weight: Font.DemiBold; text: "Quit all" }
                }
                MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: sw.hover(sw.index, "all"); onClicked: { sw.level = "all"; sw.key("ok") } }
            }
        }

        // nothing is open
        Column {
            visible: sw.count === 0
            anchors.horizontalCenter: parent.horizontalCenter
            y: 380
            spacing: Lu.space(3)
            LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "title"; font.weight: Font.DemiBold; text: "No apps open" }
            LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "label"; color: Lu.ink2; text: "Apps you open stay here until you quit them." }
        }

        // the cards: every app, each at its place in the row; a card past the window waits out of sight where the row would carry it on
        Repeater {
            model: sw.count
            delegate: Item {
                id: card
                readonly property var e: sw.entries[index]
                readonly property var g: Geo.card(index, sw.count, sw.first)
                readonly property bool here: Geo.inWindow(index, sw.first)
                readonly property bool focused: sw.index === index && sw.level === "card" && !sw.asking
                readonly property bool quitFocus: sw.index === index && sw.level === "quit" && !sw.asking
                readonly property bool current: sw.index === index && sw.level !== "all"
                readonly property bool closing: !!e && e.state === "quitting"
                x: g.x; y: g.y; width: g.w
                opacity: here ? 1 : 0
                visible: opacity > 0
                Behavior on x { enabled: !Lu.reduced; NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }
                Behavior on opacity { NumberAnimation { duration: Lu.ms("labelMs") } }

                // the picture's place. The compositor draws a live picture below this window; here is what stands on it and what stands in for it.
                Item {
                    id: pic
                    width: card.g.w; height: card.g.h
                    // an app with no picture of its own to show: its icon on a blurred tile (decoded small and drawn large: no blur shader runs)
                    Rectangle {
                        anchors.fill: parent
                        visible: !!card.e && card.e.picture !== "live"
                        radius: card.g.r
                        color: Lu.glassRaised
                        border.width: 1; border.color: Lu.edge
                        LuAppIcon { anchors.fill: parent; radius: card.g.r; decode: 16; placeholder: false; opacity: 0.85; source: card.e && card.e.picture !== "snap" ? card.e.iconUrl : "" }
                        LuAppIcon { anchors.fill: parent; radius: card.g.r; decode: 512; placeholder: false; source: card.e && card.e.picture === "snap" ? card.e.thumb : "" }
                        Rectangle {
                            anchors.fill: parent; radius: card.g.r
                            gradient: Gradient {
                                GradientStop { position: 0; color: Qt.rgba(Lu.scrimHide.r, Lu.scrimHide.g, Lu.scrimHide.b, 0.10) }
                                GradientStop { position: 1; color: Qt.rgba(Lu.scrimHide.r, Lu.scrimHide.g, Lu.scrimHide.b, 0.55) }
                            }
                        }
                        LuAppIcon { anchors.centerIn: parent; width: 104; height: 104; radius: 24; visible: !!card.e && card.e.picture !== "snap"; source: card.e ? card.e.iconUrl : "" }
                    }
                    // the others are dimmed, the focused one is whole (the dim is a veil, so a picture of any kind takes it)
                    Rectangle {
                        anchors.fill: parent; radius: card.g.r
                        color: Qt.rgba(Lu.scrimHide.r, Lu.scrimHide.g, Lu.scrimHide.b, Lu.isLight ? 0.46 : 0.40)
                        opacity: card.current || sw.asking ? 0 : 1
                        visible: opacity > 0
                        Behavior on opacity { enabled: !Lu.reduced; NumberAnimation { duration: Lu.ms("focusMs") } }
                    }
                    // the focus: a ring just outside the picture, in the full ink (3:1 on any picture and on the dim); it never moves, it fades
                    Rectangle {
                        x: -7; y: -7; width: parent.width + 14; height: parent.height + 14
                        radius: card.g.r + 7
                        color: "transparent"
                        border.width: 3; border.color: Lu.ink
                        opacity: card.focused && !sw.moving ? 1 : 0
                        visible: opacity > 0
                        Behavior on opacity { enabled: !Lu.reduced; NumberAnimation { duration: Lu.ms("focusMs") } }
                    }
                    MouseArea {
                        anchors.fill: parent; hoverEnabled: true
                        onEntered: sw.hover(index, "card")
                        onClicked: { sw.hover(index, "card"); sw.key("ok") }
                    }
                }

                // the app's icon and name, then what is happening to it, then (on the focused card) Quit
                Item {
                    id: foot
                    y: pic.height + Lu.space(4) + 2
                    width: card.g.w
                    height: Math.max(nameCol.height, 48) + (card.current ? Lu.space(3) + quitBtn.height : 0)
                    LuAppIcon { id: icon; width: 48; height: 48; radius: 12; source: card.e ? card.e.iconUrl : "" }
                    Column {
                        id: nameCol
                        x: 48 + Lu.space(4); width: foot.width - x
                        LuText { width: parent.width; role: "body"; font.weight: Font.DemiBold; color: card.current ? Lu.ink : Lu.ink2; wrapMode: Text.WordWrap; maximumLineCount: 3; elide: Text.ElideRight; text: card.e ? card.e.title : "" }
                        LuText {
                            width: parent.width; role: "caption"; color: card.closing ? Lu.ink : Lu.ink2; wrapMode: Text.WordWrap
                            visible: text !== ""
                            text: !card.e ? "" : card.e.state === "quitting" ? "Quitting\u2026" : card.e.state === "stuck" ? "Couldn\u2019t quit" : card.e.picture === "video" || card.e.picture === "none" ? "No preview" : ""
                        }
                    }
                    Rectangle {
                        id: quitBtn
                        visible: card.current
                        y: Math.max(nameCol.height, 48) + Lu.space(3)
                        width: quitRow.width + 2 * Lu.space(6); height: Math.max(Lu.target, quitRow.height + Lu.space(3))
                        radius: Lu.radius("control")
                        color: card.quitFocus ? Lu.glassRaised : Lu.sheet
                        border.width: 2; border.color: card.quitFocus ? Lu.ink : sw.boundary
                        opacity: card.closing ? Lu.disabledOpacity : 1
                        Row {
                            id: quitRow
                            anchors.centerIn: parent
                            spacing: Lu.space(3)
                            LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "x"; size: 28; color: Lu.ink }
                            LuText { anchors.verticalCenter: parent.verticalCenter; role: "label"; font.weight: Font.DemiBold; text: card.e && card.e.state === "stuck" ? "Try again" : "Quit" }
                        }
                        MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: sw.hover(index, "quit"); onClicked: { sw.hover(index, "quit"); sw.key("ok") } }
                    }
                }
            }
        }

        // more than four: where the window of four is among them (inside the safe area: nothing is drawn in the margin)
        Item {
            visible: sw.count > Geo.VISIBLE
            x: 96; y: 830; width: 1728; height: 8
            Rectangle { anchors.fill: parent; radius: 4; color: Lu.trackOff }
            Rectangle {
                y: 0; height: parent.height; radius: 4; color: Lu.ink2
                width: Math.round(parent.width * Geo.VISIBLE / Math.max(1, sw.count))
                x: Math.round(parent.width * sw.first / Math.max(1, sw.count))
                Behavior on x { enabled: !Lu.reduced; NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }
            }
        }

        // what the keys do
        Rectangle {
            visible: sw.count > 0
            anchors.right: parent.right; anchors.rightMargin: 96; y: 1080 - 54 - height
            width: hints.width + 2 * Lu.space(6); height: Math.max(Lu.target, hints.height + Lu.space(3))
            radius: height / 2
            color: Qt.rgba(Lu.scrimHide.r, Lu.scrimHide.g, Lu.scrimHide.b, 0.86)
            Row {
                id: hints
                anchors.centerIn: parent
                spacing: Lu.space(7)
                Repeater {
                    model: [{ k: "OK", t: "Open" }, { k: "Back", t: "Close" }]
                    delegate: Row {
                        spacing: Lu.space(3)
                        Rectangle {
                            anchors.verticalCenter: parent.verticalCenter
                            width: keyText.implicitWidth + 2 * Lu.space(3); height: keyText.implicitHeight + Lu.space(2)
                            radius: 10; color: "transparent"; border.width: 2; border.color: Lu.edgeRaised
                            LuText { id: keyText; anchors.centerIn: parent; role: "label"; font.weight: Font.DemiBold; text: modelData.k }
                        }
                        LuText { anchors.verticalCenter: parent.verticalCenter; role: "label"; color: Lu.ink2; text: modelData.t }
                    }
                }
            }
        }

        // Quit all asks first: a backdrop of the palette's scrimHide over the pictures too, the question, the safe choice first, the danger style on
        // the one destructive choice only
        Rectangle { anchors.fill: parent; color: Lu.scrimHide; visible: sw.asking; MouseArea { anchors.fill: parent } }
        Rectangle {
            id: ask
            visible: sw.asking
            anchors.centerIn: parent
            width: 880; height: askCol.height + 2 * Lu.space(8)
            radius: Lu.radius("sheet")
            color: Lu.isLight ? "#FFFFFF" : Lu.glassRaised
            border.width: 1; border.color: Lu.edge
            Column {
                id: askCol
                x: 48; y: Lu.space(8)
                width: parent.width - 2 * x
                spacing: Lu.space(3)
                LuText { width: parent.width; role: "title"; font.weight: Font.DemiBold; wrapMode: Text.WordWrap; text: "Quit all " + sw.count + " apps?" }
                LuText { width: parent.width; role: "body"; color: Lu.ink2; wrapMode: Text.WordWrap; text: "Everything that is open closes. Anything unsaved in them is lost." }
                Item { width: 1; height: Lu.space(4) }
                Flow {
                    width: parent.width
                    layoutDirection: Qt.RightToLeft
                    spacing: Lu.space(5)
                    Rectangle {
                        id: yes
                        width: yesText.width + 2 * Lu.space(7); height: Math.max(Lu.target, yesText.height + Lu.space(4))
                        radius: Lu.radius("control")
                        color: Lu.isLight ? "#9A4B63" : Lu.danger
                        border.width: 2; border.color: sw.askIndex === 1 ? Lu.ink : "transparent"
                        LuText { id: yesText; anchors.centerIn: parent; role: "label"; font.weight: Font.DemiBold; color: Lu.isLight ? "#FFFFFF" : Lu.accentInk; text: "Quit all" }
                        MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: sw.askIndex = 1; onClicked: { sw.askIndex = 1; sw.key("ok") } }
                    }
                    Rectangle {
                        id: no
                        width: noText.width + 2 * Lu.space(7); height: yes.height
                        radius: Lu.radius("control")
                        color: sw.askIndex === 0 ? (Lu.isLight ? "#F1E9F6" : "#3A3350") : (Lu.isLight ? "#FFFFFF" : "#2C263A")
                        border.width: 2; border.color: sw.askIndex === 0 ? Lu.ink : sw.boundary
                        LuText { id: noText; anchors.centerIn: parent; role: "label"; font.weight: Font.DemiBold; text: "Cancel" }
                        MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: sw.askIndex = 0; onClicked: { sw.askIndex = 0; sw.key("ok") } }
                    }
                }
            }
        }
    }
}
