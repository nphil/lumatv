import QtQuick 2.4
import "../kit"

// Cameras through Home Assistant's LumaShow integration (docs/research/ha-cameras-result.md, docs/mocks/cameras/): what the helper's
// `cameras` and `cameras.popup` topics say, drawn in the layer that is up over everything (LumaOverlay), so the same surface serves over
// Home, over Netflix and over an input.
//
//   popup   a detector of a shared camera saw something (`cameras.popup`: the camera, the kind, when): a glass card in the top right
//           corner, inside the safe area and below the status bar's row, with the camera's name, what was seen and a live preview. It
//           goes by itself when the helper's `untilMs` passes; Back dismisses it (the helper is told, so every reader sees it go); OK
//           expands it to the full screen. Only OK and Back are taken: the D-pad stays the app's. While the host is busy (a PIN prompt,
//           an alert, a confirmation, another panel) the pop-up waits, and arrives when the host is free again if it is still in date.
//   full    one camera on the whole screen, fitted, black beside it; its name and the hints fade after a moment (Lucent: chrome over
//           full-screen media leaves, the way out never does); Left and Right go to the next camera, OK opens the grid, Back closes.
//   grid    every shared camera (at most six) as live tiles with their names; the D-pad moves the focus, OK opens a tile on the full
//           screen, Back closes.
//
// Nothing here fetches while nothing is shown (kit/CameraFrame.qml asks for pictures only while active), and the surface is a Resident
// that is built once: a pop-up never builds or compiles anything in the frame it arrives.
Item {
    id: cams
    anchors.fill: parent
    property bool appFront: false            // an app's picture is under this layer: the glass lets it through to the reading alpha
    property bool hostBusy: false            // the host has something up that must not be covered (a PIN, an alert, a panel): a pop-up waits
    signal closeRequested()
    signal popupShown(string kind)           // a pop-up came up: the overlay system takes in a doorbell record of the same ring

    readonly property var topic: Helper.topics.cameras || null
    readonly property var popupTopic: Helper.topics["cameras.popup"] || null
    readonly property var cameras: topic && topic.cameras ? topic.cameras : []
    readonly property int count: cameras.length
    // the live path (the helper's `cameras.live`): the full view's camera on the video plane through the feeder, this layer transparent
    // over it (the hole) with its chrome above; until it plays, and wherever it is off or fails, the pictures stay. Live is asked for while the link
    // is `ready` and, over an app, only with theme `cameras.liveOverApps` (the feeder's window over a playing app is unmeasured, check C12)
    readonly property bool liveAvailable: !!topic && topic.state === "ready" && (!appFront || (!!Lu.theme.cameras && Lu.theme.cameras.liveOverApps === true))
    readonly property var liveTopic: Helper.topics["cameras.live"] || null
    readonly property string currentId: current >= 0 && current < count ? cameras[current].id : ""
    readonly property bool livePlaying: mode === "full" && !!liveTopic && liveTopic.state === "playing" && liveTopic.camera === currentId
    readonly property bool hole: livePlaying
    property string liveAsked: ""            // the camera the helper was asked to play, "" when none
    function askLive(id) {
        if (!liveAvailable || id === "" || id === liveAsked) return
        liveAsked = id
        Helper.post("/v1/cameras/live", { camera: id })
    }
    function stopLive() {
        if (liveAsked === "") return
        liveAsked = ""
        Helper.post("/v1/cameras/live/stop", {})
    }
    // read from `current` itself: a binding on it (currentId) may not have been brought up to date before this handler runs
    function idAt(i) { return i >= 0 && i < count ? cameras[i].id : "" }
    onCurrentChanged: if (mode === "full") askLive(idAt(current))
    onModeChanged: { if (mode === "full") askLive(idAt(current)); else stopLive() }

    property string mode: ""                 // "", "popup", "full", "grid"
    property var popup: null                 // the pop-up shown now: the topic's value when it was taken
    property double shownId: 0               // the id of the last pop-up shown (a topic re-read is not a new pop-up)
    property double shownAt: 0               // when it was shown (the host's clock), for the gate's event-to-pop-up measure
    property int current: -1                 // the camera the full view shows
    property int focusIndex: 0               // the grid's focus
    readonly property bool up: mode !== ""
    readonly property bool popupUp: mode === "popup"
    readonly property bool viewerUp: mode === "full" || mode === "grid"
    // the keys this takes: every key while the viewer is up, OK and Back while a pop-up is
    readonly property bool wants: viewerUp
    readonly property bool built: true
    readonly property real paneAlpha: appFront ? Lu.readingAlpha : 1

    // ---- the pop-up: taken from the topic when it is new, in date, and the host is free
    function dueNow() { return popupTopic && popupTopic.id !== shownId && popupTopic.untilMs > Date.now() }
    function consider() {
        if (!dueNow()) { waiting.stop(); return }
        if (viewerUp) {
            // already watching: the full view turns to the camera something was seen on, with no card over it
            shownId = popupTopic.id
            if (mode === "full") { var i = indexOf(popupTopic.camera); if (i >= 0) current = i }
            return
        }
        if (hostBusy) { waiting.restart(); return }
        waiting.stop()
        shownId = popupTopic.id
        shownAt = Date.now()
        popup = popupTopic
        mode = "popup"
        popupShown(popupTopic.kind)
        ends.interval = Math.max(500, popupTopic.untilMs - Date.now())
        ends.restart()
        tick()
        console.info("[Cameras] pop-up " + popupTopic.kind + " at " + popupTopic.camera + " for " + ends.interval + " ms")
    }
    onPopupTopicChanged: {
        if (popupUp && (!popupTopic || popupTopic.id !== shownId)) hidePopup(false)   // cleared elsewhere, or replaced
        consider()
    }
    // Do not disturb: a PIN, an alert, the keyboard or a panel arrives while the card is up and it steps aside, without telling the helper (it is
    // not dismissed): it is due again, and comes back when the host is free if its time (`untilMs`) is not over.
    onHostBusyChanged: { if (hostBusy && popupUp) holdPopup(); else if (!hostBusy) consider() }
    function holdPopup() {
        ends.stop()
        mode = ""
        popup = null
        shownId = 0
    }
    Timer { id: waiting; interval: 500; repeat: true; onTriggered: cams.consider() }
    Timer { id: ends; onTriggered: cams.hidePopup(true) }
    function hidePopup(tell) {
        if (!popupUp) return
        ends.stop()
        mode = ""
        popup = null
        if (tell) Helper.post("/v1/cameras/popup/clear", {})
    }
    function dismiss() { hidePopup(true) }
    function expand() {
        var id = popup ? popup.camera : ""
        hidePopup(true)
        openFull(id)
    }

    // ---- the viewer
    function indexOf(id) { for (var i = 0; i < cameras.length; i++) if (cameras[i].id === id) return i; return -1 }
    function openFull(id) {
        var i = indexOf(id)
        current = i >= 0 ? i : (count > 0 ? 0 : -1)
        if (current < 0) { close(); return }
        mode = "full"
        showChrome()
    }
    function openGrid(focusOn) {
        if (count === 0) { close(); return }
        focusIndex = Math.max(0, Math.min(Math.min(count, 6) - 1, focusOn))
        mode = "grid"
        showChrome()
    }
    function close() {
        var was = up
        ends.stop()
        waiting.stop()
        chromeT.stop()
        mode = ""
        popup = null
        current = -1
        if (was) closeRequested()
    }
    onCountChanged: if (viewerUp && count === 0) close()
    // the full view's chrome: the name and the hints come with every key and go 2.5 s later
    property bool chrome: true
    function showChrome() { chrome = true; chromeT.restart() }
    Timer { id: chromeT; interval: 2500; onTriggered: cams.chrome = false }

    // ---- keys. True when taken.
    function key(dir) {
        if (popupUp) {
            if (dir === "ok") { expand(); return true }
            if (dir === "back") { dismiss(); return true }
            return false
        }
        if (mode === "full") {
            showChrome()
            if (dir === "left") current = (current + count - 1) % count
            else if (dir === "right") current = (current + 1) % count
            else if (dir === "ok") { if (count > 1) openGrid(current) }
            else if (dir === "back") close()
            return true
        }
        if (mode === "grid") {
            showChrome()
            var n = Math.min(count, 6), cols = gridCols
            if (dir === "left") focusIndex = focusIndex % cols > 0 ? focusIndex - 1 : focusIndex
            else if (dir === "right") focusIndex = focusIndex % cols < cols - 1 && focusIndex + 1 < n ? focusIndex + 1 : focusIndex
            else if (dir === "up") focusIndex = focusIndex - cols >= 0 ? focusIndex - cols : focusIndex
            // down lands on the nearest tile of the row below when that row is short (three cameras in two columns)
            else if (dir === "down") focusIndex = focusIndex + cols < n ? focusIndex + cols : Math.floor((n - 1) / cols) > Math.floor(focusIndex / cols) ? n - 1 : focusIndex
            else if (dir === "ok") openFull(cameras[focusIndex].id)
            else if (dir === "back") close()
            return true
        }
        return false
    }

    // ---- words
    readonly property var kindWords: ({ person: "Person", vehicle: "Vehicle", animal: "Animal", package: "Package", doorbell: "Doorbell", motion: "Motion" })
    readonly property var kindIcons: ({ person: "user", vehicle: "car", animal: "paw-print", package: "package", doorbell: "bell-ring", motion: "activity" })
    function kindWord(p) {
        if (!p) return ""
        var w = kindWords[p.kind] || p.kind
        if (p.kind === "doorbell") return "Doorbell rang"
        return p.label && String(p.label).toLowerCase() !== p.kind ? w + " \u00b7 " + p.label : w
    }
    property string ago: ""
    function tick() {
        if (!popup) { ago = ""; return }
        var s = Math.max(0, Math.round((Date.now() - popup.at) / 1000))
        ago = s < 60 ? "Just now" : Math.round(s / 60) + " min ago"
    }
    Timer { interval: 30000; repeat: true; running: cams.popupUp; onTriggered: cams.tick() }
    // the helper's topics are what this draws; the client is started once (idempotent: Home starts it too)
    Component.onCompleted: { Helper.start(); consider() }
    function nameOf(i) { return i >= 0 && i < cameras.length ? (cameras[i].name || cameras[i].id) : "" }

    // ================================================================= the pop-up card (docs/mocks/cameras/popup)
    // Top right, ending where Home's status bar does (theme overlays.right, 64: layout.json safe.x) and below its row (54 + 60 + 20 = 134): the mute mark's
    // row; the mark keeps to the left corner while this is up (LumaOverlay tells the volume display).
    readonly property int rightEdge: Lu.theme && Lu.theme.overlays && Lu.theme.overlays.right !== undefined ? Lu.theme.overlays.right : 64     // the cards under it use the same
    readonly property int padX: Lu.space(5)       // 20
    readonly property int cardW: 560
    readonly property int previewW: cardW - 2 * padX
    // 292: an even height, as the decoders like it. Where the head and the foot grow (large text) the picture gives up height, never below 120, until the
    // card ends above 75 % of the height (810): the film's subtitles and the volume bar keep the bottom quarter (docs/research/overlays-result.md).
    readonly property int previewNat: Math.floor(previewW * 9 / 16)
    readonly property int cardLimit: 810
    readonly property int previewH: Math.max(120, Math.min(previewNat, ((cardLimit - 134 - (2 * padX + head.height + 2 * Lu.space(3) + foot.height)) >> 1) << 1))
    // where the card ends, for the overlay system's cards that go under it
    readonly property real popupBottom: card.y + card.height
    readonly property real popupRight: card.x + card.width     // and where it ends across: where Home's right edge is
    LuPane {
        id: card
        width: cams.cardW
        height: head.y + head.height + Lu.space(3) + cams.previewH + Lu.space(3) + foot.height + cams.padX
        x: cams.width - cams.rightEdge - width
        y: 134
        cardRadius: Lu.radius("card")
        fillAlpha: cams.paneAlpha
        opacity: cams.popupUp ? 1 : 0
        visible: opacity > 0
        // in from a touch to the right and a little smaller, the card's own motion; out quickly. Nothing moves after it has landed.
        scale: cams.popupUp ? 1 : 0.96
        transform: Translate {
            x: cams.popupUp ? 0 : 24
            Behavior on x { enabled: !Lu.reduced; NumberAnimation { duration: cams.popupUp ? Lu.cardMs : Lu.restMs; easing.type: Lu.cardEasing } }
        }
        Behavior on opacity { enabled: !Lu.reduced; NumberAnimation { duration: cams.popupUp ? Lu.cardMs : Lu.restMs; easing.type: Lu.cardEasing } }
        Behavior on scale { enabled: !Lu.reduced; NumberAnimation { duration: Lu.cardMs; easing.type: Lu.cardEasing } }
        onOpacityChanged: if (opacity === 1) locate()

        Item {
            id: head
            x: cams.padX; y: cams.padX
            width: cams.previewW
            height: Math.max(glyph.height, nameText.implicitHeight + Lu.space(1) + whatText.implicitHeight)
            LuIcon {
                id: glyph
                name: cams.popup ? (cams.kindIcons[cams.popup.kind] || "camera") : "camera"
                size: 36
                y: 0
            }
            // the name has the whole line; what was seen and when share the second (little copy, nothing competing with the name at 200 %)
            LuText {
                id: nameText
                x: glyph.width + Lu.space(4)
                width: head.width - x
                role: "body"
                font.weight: Font.DemiBold
                textFormat: Text.PlainText               // a camera's name is Home Assistant's words, never markup
                elide: Text.ElideRight
                text: cams.popup ? (cams.popup.name || cams.popup.camera) : ""
            }
            LuText {
                id: whatText
                x: nameText.x
                y: nameText.implicitHeight + Lu.space(1)
                width: nameText.width
                role: "label"
                color: Lu.ink2
                wrapMode: Text.WordWrap
                maximumLineCount: 2
                elide: Text.ElideRight
                text: cams.kindWord(cams.popup) + (cams.ago !== "" ? "  \u00b7  " + cams.ago : "")
            }
        }
        CameraFrame {
            id: preview
            x: cams.padX
            y: head.y + head.height + Lu.space(3)
            width: cams.previewW
            height: cams.previewH
            radius: Lu.radius("tile")
            camera: cams.popup ? cams.popup.camera : ""
            active: cams.popupUp
            intervalMs: 700
            cover: true
        }
        Item {
            id: foot
            x: cams.padX
            y: preview.y + preview.height + Lu.space(3)
            width: cams.previewW
            // the keys sit at the end of the state's line, or on a line of their own when the text is large, and then each on a line when both do not
            // fit (a key's words are never cut and never at the card's edge)
            readonly property real keysW: hintOk.implicitWidth + Lu.space(5) + hintBack.implicitWidth
            readonly property bool twoLines: stateText.x + stateText.implicitWidth + Lu.space(4) + keysW > width
            readonly property bool stacked: keysW > width
            height: stateText.implicitHeight + (twoLines ? Lu.space(1) + hintOk.implicitHeight : 0) + (stacked ? Lu.space(1) + hintBack.implicitHeight : 0)
            Rectangle {
                id: liveDot
                width: 12; height: 12; radius: 6
                y: Math.round((stateText.implicitHeight - height) / 2)
                color: Lu.live
                visible: preview.ready
            }
            LuText {
                id: stateText
                x: liveDot.visible ? liveDot.width + Lu.space(2) : 0
                role: "caption"
                color: Lu.ink2
                text: preview.ready ? "Live" : preview.failed > 1 ? "No picture" : "Connecting"
            }
            LuText {
                id: hintOk
                x: foot.twoLines ? 0 : foot.width - foot.keysW
                y: foot.twoLines ? stateText.implicitHeight + Lu.space(1) : 0
                role: "caption"
                color: Lu.ink2
                text: "OK expands"
            }
            LuText {
                id: hintBack
                x: foot.stacked ? 0 : hintOk.x + hintOk.implicitWidth + Lu.space(5)
                y: foot.stacked ? hintOk.y + hintOk.implicitHeight + Lu.space(1) : hintOk.y
                role: "caption"
                color: Lu.ink2
                text: "Back dismisses"
            }
        }
    }

    // ================================================================= the full view (docs/mocks/cameras/full)
    Item {
        id: full
        anchors.fill: parent
        visible: cams.mode === "full"
        // black behind the pictures; nothing at all while the stream plays on the video plane under this layer
        Rectangle { anchors.fill: parent; color: "#000000"; visible: !cams.hole }
        CameraFrame {
            id: fullFrame
            anchors.fill: parent
            visible: !cams.hole
            camera: cams.currentId
            active: cams.mode === "full" && !cams.hole
            intervalMs: 700
            cover: false
        }
        // the name, top left, a pill that fades with the hints
        Item {
            id: fullChrome
            anchors.fill: parent
            opacity: cams.chrome ? 1 : 0
            visible: opacity > 0
            Behavior on opacity { enabled: !Lu.reduced; NumberAnimation { duration: Lu.layerMs } }
            LuPane {
                x: 96; y: 54
                width: nameRow.width + 2 * Lu.space(6)
                height: Math.max(Lu.target, nameRow.height + 2 * Lu.space(3))
                cardRadius: height / 2
                fillAlpha: Lu.readingAlpha
                Row {
                    id: nameRow
                    x: Lu.space(6); y: Math.round((parent.height - height) / 2)
                    spacing: Lu.space(3)
                    LuIcon { name: "video"; size: 32; anchors.verticalCenter: parent.verticalCenter }
                    LuText { role: "body"; font.weight: Font.DemiBold; text: cams.nameOf(cams.current); anchors.verticalCenter: parent.verticalCenter }
                    Rectangle { width: 12; height: 12; radius: 6; color: Lu.live; visible: fullFrame.ready || cams.livePlaying; anchors.verticalCenter: parent.verticalCenter }
                    // which of the cameras this is, on the same reading surface as the name (a count alone over a bright picture would not read)
                    LuText { visible: cams.count > 1; role: "label"; color: Lu.ink2; text: (cams.current + 1) + " of " + cams.count; anchors.verticalCenter: parent.verticalCenter }
                }
            }
            // the hints, bottom right: what the keys do here
            LuPane {
                anchors.right: parent.right; anchors.rightMargin: 96
                anchors.bottom: parent.bottom; anchors.bottomMargin: 54
                width: hints.width + 2 * Lu.space(6)
                height: Math.max(Lu.target, hints.height + 2 * Lu.space(3))
                cardRadius: height / 2
                fillAlpha: Lu.readingAlpha
                LuText {
                    id: hints
                    x: Lu.space(6); y: Math.round((parent.height - height) / 2)
                    role: "label"
                    color: Lu.ink2
                    text: (cams.count > 1 ? "\u2190 \u2192 Next camera  \u00b7  OK Grid  \u00b7  " : "") + "Back Close"
                }
            }
        }
    }

    // ================================================================= the grid (docs/mocks/cameras/grid)
    readonly property int gridN: Math.min(count, 6)
    readonly property int gridCols: gridN <= 1 ? 1 : gridN <= 4 ? 2 : 3
    readonly property int gridRows: Math.max(1, Math.ceil(gridN / gridCols))
    readonly property int gridGap: Lu.space(6)
    readonly property int labelH: 44
    // tiles as wide as the columns allow, and no taller than the rows allow inside the safe area (two rows of two: 764 by 430)
    readonly property int tileByW: Math.floor((width - 2 * 96 - (gridCols - 1) * gridGap) / gridCols)
    readonly property int tileByH: Math.floor((Math.floor((height - 2 * 54 - (gridRows - 1) * gridGap) / gridRows) - labelH) * 16 / 9)
    readonly property int tileW: Math.min(tileByW, tileByH)
    readonly property int tileH: Math.round(tileW * 9 / 16)
    readonly property int gridW: gridCols * tileW + (gridCols - 1) * gridGap
    readonly property int gridH: gridRows * (tileH + labelH) + (gridRows - 1) * gridGap
    Item {
        id: grid
        anchors.fill: parent
        visible: cams.mode === "grid"
        Rectangle { anchors.fill: parent; color: Lu.scrimHide }
        Repeater {
            model: cams.mode === "grid" ? cams.gridN : 0
            Item {
                id: tile
                readonly property bool focused: cams.focusIndex === index
                readonly property int col: index % cams.gridCols
                readonly property int row: Math.floor(index / cams.gridCols)
                x: Math.round((cams.width - cams.gridW) / 2) + col * (cams.tileW + cams.gridGap)
                y: Math.round((cams.height - cams.gridH) / 2) + row * (cams.tileH + cams.labelH + cams.gridGap)
                width: cams.tileW
                height: cams.tileH + cams.labelH
                scale: focused ? Lu.focus.cardScale || 1.03 : 1
                z: focused ? 1 : 0
                Behavior on scale { enabled: !Lu.reduced; NumberAnimation { duration: Lu.focusMs; easing.type: Lu.focusEasing } }
                CameraFrame {
                    id: tileFrame
                    width: cams.tileW
                    height: cams.tileH
                    radius: Lu.radius("tile")
                    camera: cams.cameras[index].id
                    active: cams.mode === "grid"
                    intervalMs: 1200
                    cover: true
                }
                // the focus: a lit edge on the picture (the landing light), never a white pill
                Rectangle {
                    anchors.fill: tileFrame
                    radius: Lu.radius("tile")
                    color: "transparent"
                    border.width: 3
                    border.color: tile.focused ? Lu.edgeRaised : Lu.edge
                    antialiasing: true
                    opacity: tile.focused ? 1 : 0.6
                }
                LuText {
                    y: cams.tileH + Lu.space(2)
                    width: cams.tileW
                    role: "label"
                    color: tile.focused ? Lu.ink : Lu.ink2
                    elide: Text.ElideRight
                    horizontalAlignment: Text.AlignHCenter
                    text: cams.cameras[index].name || cams.cameras[index].id
                }
                LuText {
                    visible: !tileFrame.ready
                    anchors.horizontalCenter: tileFrame.horizontalCenter
                    anchors.verticalCenter: tileFrame.verticalCenter
                    role: "label"
                    color: Lu.ink2
                    text: tileFrame.failed > 1 ? "No picture" : "Connecting"
                }
            }
        }
        LuText {
            opacity: cams.chrome ? 1 : 0
            visible: opacity > 0
            Behavior on opacity { enabled: !Lu.reduced; NumberAnimation { duration: Lu.layerMs } }
            anchors.right: parent.right; anchors.rightMargin: 96
            anchors.bottom: parent.bottom; anchors.bottomMargin: 54
            role: "label"
            color: Lu.ink2
            text: "OK Open  \u00b7  Back Close"
        }
    }
}
