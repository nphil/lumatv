import QtQuick 2.4
import QtQuick.Window 2.2
import WebOSCompositorBase 1.0
import WebOSServices 1.0
import "../kit"
import "../settings"
import "../LumaBus.js" as Bus
import "QuickArrange.js" as Arr

// The quick panel (docs/mocks/10-quick-panel): a tall sheet docked to the right edge, 560 wide, 40 from the right edge and 56 from the top and bottom, where
// the Notification Centre's sheet sits (the mock's place; LG's own sat at the left), with the few things reached for most, one row each (QuickList):
// Left and Right change a value, OK opens the page that has more. It slides out from the right edge and back (PanelMotion: a transform, no
// layout while it moves; reduced motion is a fade). Built once and kept, like Quick Settings after the patch that keeps LG's
// panel built: opening it is a visibility flip and the start of the slide.
//
// The receiver tile reads the helper's `receiver` topic and moves the volume the way the volume keys do, through the TV (`audio/master/
// volumeUp` and `volumeDown`, which webOS sends to the receiver over HDMI-CEC), so the number it shows is always the receiver's own and the
// TV's counter is never involved; the helper is told to read the receiver now so the new level appears at once. OK on the tile opens
// Settings at the receiver's page. The sleep timer tile steps through LG's choices (kit/Power) and shows what is left while one runs.
//
// Tiles of plugins (PLAN, Plugins: `plugin.<id>.tiles`, settings/PluginTiles) follow the built-in ones; OK on one is a POST to the
// plugin's action route. "All settings" is under the list, one Down past the last tile; what LG's Settings has beyond it is its row there.
//
// OK on a stepper tile, or on one that has a `popup`, opens a card beside the panel (QuickPopup: a value picker, a slider; docs/mocks/29-qp-*), and
// a held OK on a tile edits the tiles (QuickEdit): move one, take one off, add one back from the tiles the panel does not show (QuickMore). The
// owner's order and choice is the theme's `quickPanel.tiles` (QuickArrange.js); without it the panel shows its own list.
Item {
    id: panel
    anchors.fill: parent
    property bool rested: true
    property string zone: "page"                        // the list reads host.zone: the rows or the foot buttons have the keys
    signal openSettingsPage(string id)
    signal closeRequested()
    signal launchApp(string appId)
    readonly property bool built: true

    // Resident says when the panel is wanted (`presented`), keeps it drawn while `leaving` and keeps it in the scene while `held`: after the slide
    // the panel is parked, what it draws moved off the screen, so the next opening joins no nodes to the scene (PanelMotion). `up` is what is on
    // the screen: the clock, the receiver's reading and the answers to the readings only matter then.
    property bool presented: false
    readonly property bool leaving: motion.leaving
    readonly property bool moving: motion.moving          // the slide is in progress (LumaHome holds its tick meanwhile at 120 Hz)
    readonly property bool held: motion.held
    readonly property bool parked: motion.parked
    readonly property bool up: presented || leaving
    property alias lingerMs: motion.lingerMs
    function release() { motion.release() }
    // the clock is brought up to date with the press, so the opening does not draw the cached sheet again for a new minute
    function warm() { clock = new Date(); motion.warm() }
    PanelMotion { id: motion; presented: panel.presented; window: panel.Window.window }
    readonly property real progress: motion.progress     // 0 away, 1 shown
    readonly property real slideX: slide.x               // where the sheet is (1320 at rest), for the tests and the gate's step list
    readonly property real slideY: slide.y
    // the sheet's place: docked right, as the Notification Centre is (mock 10)
    readonly property int sheetW: 560
    readonly property int sheetH: 1080 - 2 * edgeY
    readonly property int edgeX: 40
    readonly property int edgeY: 56
    readonly property int pad: Lu.space(6)
    readonly property int inner: sheetW - 2 * pad

    // the receiver's reading, only while it is in date (kit/ReceiverGate). Kept while the panel is parked as well: the gate letting go
    // hides the receiver's numbers, which drew the cached sheet again as the closing ended and again at the next opening
    ReceiverGate { id: gate; active: panel.up || panel.held }
    readonly property var rc: gate.doc || Helper.receiver || ({})
    readonly property bool rcAnswers: gate.fresh
    property bool footFocus: false                      // the "All settings" button under the list has the focus, not a tile
    property var clock: new Date()
    Timer { interval: 15000; running: panel.up; repeat: true; onTriggered: panel.clock = new Date() }
    onPresentedChanged: { if (presented) refresh(); else { readTimer.stop(); endEdit(); popup.open = false } }
    // What the open path does is only what the first frame needs. The readings (each an answer that rebinds the rows and can change a
    // line's wrapping) are asked for once the slide is over and the frame burst the key started has ended: the rows show what the
    // last reading said for the first 310 ms, and a reading that differs redraws them after. Asked for at 170 ms, the answers' frames
    // chained onto the slide's (r15 over the film: a burst to 370 ms with two intervals over 20 ms, compositor frames only), as the
    // picker's did (InputPicker.qml).
    function refresh() {
        clock = new Date(); footFocus = false; touched = false; editing = false; popup.open = false
        list.reset()
        readTimer.restart()
    }
    Timer {
        id: readTimer
        interval: motion.enterMs + 200
        onTriggered: { Sio.read("picture", ["pictureMode"].concat(more.pictureKeys)); Sio.read("general", ["remotePointer"]); Inputs.refresh(); Power.readSleep(); more.readCapture() }
    }
    onFootFocusChanged: zone = footFocus ? "foot" : "page"
    onZoneChanged: footFocus = (zone === "foot")
    // The first tile (Picture mode) is only focusable once the settings service has answered; until a key is pressed the
    // focus goes to it when it can.
    property bool touched: false
    Connections {
        target: Sio
        onAnswersChanged: if (panel.up && !panel.touched) list.enter(list.first())
    }

    function key(dir) {
        touched = true
        if (popup.open) { popup.key(dir); return }
        if (editing) { edit.key(dir); return }
        if (footFocus) {
            if (dir === "up") footFocus = false
            else if (dir === "ok") openSettingsPage("")
            else if (dir === "back") closeRequested()
            return
        }
        // Down past the last tile lands on the button; the list itself stops at its last row
        if (dir === "down" && list.stop >= 0 && list.step(list.stop, 1) < 0) { footFocus = true; return }
        if (list.key(dir)) return
        if (dir === "back") closeRequested()
    }
    function patch(p) { }                              // the list's host contract; the panel changes no theme

    // ---- popups and edit mode. The arrangement is `arranged()`: the owner's list of ids (or the panel's own) as the tiles they stand for.
    property bool editing: false
    readonly property bool popupOpen: popup.open
    // theme `remote.quickPanelEdit` (Settings > Remote & pointer), off until the TV check: with it off OK on a stepper does nothing, a held OK is an ordinary OK,
    // and the hint line is not drawn
    readonly property bool popupsOn: !!Lu.theme && !!Lu.theme.remote && Lu.theme.remote.quickPanelEdit === true
    QuickMore { id: more; panel: panel }
    function ev(x) { return typeof x === "function" ? x() : x }
    function everything() {
        var all = tiles(), extra = more.rows()
        for (var i = 0; i < all.length; i++) all[i].dflt = true
        for (var j = 0; j < extra.length; j++) extra[j].dflt = false
        return all.concat(extra)
    }
    function ids(rows) { return rows.map(function(r) { return r.id }) }
    function arranged() {
        var all = everything()
        var saved = Lu.theme && Lu.theme.quickPanel ? Lu.theme.quickPanel.tiles : undefined
        var byId = {}
        for (var i = 0; i < all.length; i++) byId[all[i].id] = all[i]
        return Arr.shown(saved, all.map(function(r) { return { id: r.id, dflt: r.dflt } })).map(function(id) { return byId[id] })
    }
    // The owner's order, shown at once and (`keep`) written; with no Home loaded to write it nothing changes.
    function keepOrder(order, keep) { Bus.writeTheme(Arr.patch(order), keep) }
    function popupFor(row) {
        if (row.popup) return row.popup
        var x = more.pickExtras(row.id)
        return { kind: "pick", title: String(ev(row.label)), text: x.text || "", options: ev(row.options), icon: x.icon || String(ev(row.icon) || ""), detail: x.detail,
                 index: function() { var i = ev(row.index); return i === undefined ? -1 : i }, set: function(i) { row.act(i) } }
    }
    function openPopup(row) { popup.show(popupFor(row)) }
    function openAdd() {
        var shown = ids(arranged())
        var left = everything().filter(function(r) { return shown.indexOf(r.id) < 0 })
        popup.show({ kind: "add", title: "Add a tile", text: left.length ? "Tiles not on the panel. OK adds one at the bottom." : "Every tile is on the panel.",
                     items: left.map(function(r) { return { id: r.id, icon: String(ev(r.icon) || ""), label: String(ev(r.label)) } }),
                     pick: function(id) { keepOrder(Arr.added(shown, id), true); edit.index = shown.length } })
    }
    // A held OK on a tile (LumaHome and the overlay time the hold): that tile is picked up in edit mode.
    function holdReady() { return popupsOn && !editing && !popup.open && !footFocus && list.stop >= 0 && !!list.rows[list.stop] }
    function hold() {
        if (!holdReady()) return
        var at = ids(list.rows).indexOf(list.rows[list.stop].id)
        editStart = ids(list.rows)
        editing = true
        edit.begin(at)
    }
    // Leaving edit mode (Back, or the panel closing): a change that is only on the screen so far is written.
    property var editStart: []
    function endEdit() {
        if (!editing) return
        editing = false
        var now = ids(arranged())
        if (!Arr.same(now, editStart)) keepOrder(now, true)
    }

    PluginTiles { id: pluginTiles }

    // The tiles of a plugin come and go while the panel is up: the focus stays on the same tile by id.
    property string focusId: ""
    Connections {
        target: list
        onStopChanged: panel.focusId = (list.stop >= 0 && list.rows[list.stop]) ? (list.rows[list.stop].id || "") : panel.focusId
        onRowsChanged: {
            for (var i = 0; i < list.rows.length; i++) if (list.rows[i].id === panel.focusId && list.focusable(i)) { list.enter(i); return }
            list.reset()
        }
    }

    readonly property var modeValues: ["filmMaker", "cinema", "normal", "vivid", "game", "sports", "eco"]
    readonly property var modeNames: ({ filmMaker: "Filmmaker", cinema: "Cinema", normal: "Standard", vivid: "Vivid", game: "Game", sports: "Sports", eco: "Eco" })

    // Behind the sheet the page dims for as long as the panel is drawn. The dim is not faded: the scrim covers the whole screen, and every
    // change of its opacity made Qt's renderer batch the scene under it again, each frame of the slide (round 11: p90 3.70 ms over Home,
    // 2.31 ms over an input, where little is under it). Parked, the scrim is moved off the screen: it stays a node of the scene.
    Rectangle { x: panel.parked ? -width : 0; width: parent.width; height: parent.height; color: Lu.scrim }

    // The one thing that moves: the sheet slides out of the right edge. Only x changes per frame, so nothing is laid out while it moves; the
    // glass finds its place in the wallpaper's blur again as it goes (LuPane.locate). It does not fade as it travels: the frame where its
    // opacity passed 1 rebuilt the whole render list and every frame below 1 re-batched the sheet (round 12 over Home: the second frame of
    // an opening and the fourth and fifth of a closing, 3.1-4.5 ms against about 2 ms for the others). Reduced motion is the fade alone.
    Item {
        id: slide
        readonly property real travel: panel.edgeX + panel.sheetW
        readonly property real restX: 1920 - panel.edgeX - panel.sheetW
        x: panel.parked ? 1920 + 400 + (motion.nudge ? 1 : 0) : restX + (Lu.reduced ? 0 : travel * (1 - motion.progress))
        y: panel.edgeY
        width: panel.sheetW
        height: panel.sheetH
        opacity: Lu.reduced ? motion.progress : 1
        onXChanged: if (!panel.parked) sheet.locate()

        LuPane {
            id: sheet
            anchors.fill: parent
            cacheContent: true
            Item {
                id: head
                x: panel.pad; y: panel.pad
                width: panel.inner
                // the clock sits beside the title while both fit on a line, else under it (a large text size)
                readonly property bool clockBelow: titleProbe.implicitWidth + clockText.implicitWidth + Lu.space(4) > width
                height: clockBelow ? titleText.height + clockText.height : Math.max(titleText.height, clockText.height)
                LuText { id: titleText; width: parent.width - (head.clockBelow ? 0 : clockText.width + Lu.space(4)); role: "title"; text: panel.editing ? "Edit tiles" : "Quick settings"; wrapMode: Text.WordWrap }
                LuText { id: titleProbe; visible: false; role: "title"; text: titleText.text }
                LuText {
                    id: clockText
                    x: head.clockBelow ? 0 : parent.width - width
                    y: head.clockBelow ? titleText.height : Math.round((titleText.height - height) / 2)
                    role: "label"; color: Lu.ink2
                    text: panel.editing ? (edit.grab ? "Up and down moves \u00b7 OK drops" : "OK picks up \u00b7 Back is done") : Qt.formatTime(panel.clock, "h:mm AP")
                }
            }
            QuickList {
                id: list
                visible: !panel.editing
                x: panel.pad; y: head.y + head.height + Lu.space(4)
                width: panel.inner
                height: foot.y - Lu.space(4) - y
                host: panel
                rested: panel.rested
                instant: !motion.settled
                rows: panel.arranged()
            }
            QuickEdit {
                id: edit
                visible: panel.editing
                x: panel.pad; y: head.y + head.height + Lu.space(4)
                width: panel.inner
                height: sheet.height - panel.pad - y
                rows: panel.arranged()
                rested: panel.rested
                instant: !motion.settled
                onReorder: panel.keepOrder(ids, keep)
                onAddRequested: panel.openAdd()
                onDone: panel.endEdit()
            }
            Item {
                id: foot
                visible: !panel.editing
                x: panel.pad
                width: panel.inner
                height: allSettings.height + (hint.visible ? hint.height + Lu.space(2) : 0)
                y: sheet.height - panel.pad - height
                LuButton {
                    id: allSettings
                    width: foot.width
                    text: "All settings"; icon: "settings"
                    focused: panel.footFocus; rested: panel.rested; instant: !motion.settled
                    onActivated: { panel.footFocus = true; panel.openSettingsPage("") }
                    onPointerFocus: { panel.footFocus = true }
                }
                // what the keys do that the tiles do not say (docs/mocks/29-qp-slider)
                LuText {
                    id: hint
                    visible: panel.popupsOn
                    y: allSettings.height + Lu.space(2)
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    role: "caption"; color: Lu.ink2; wrapMode: Text.WordWrap
                    text: "Hold OK on a tile to edit \u00b7 Back closes"
                }
            }
        }
        // beside the sheet, 48 px from it (the mock): a value, a choice or a tile to add; it moves with the sheet and shows only while it is asked for
        QuickPopup {
            id: popup
            x: -(width + Lu.space(8) + Lu.space(2))
            y: 81
            rested: panel.rested
        }
    }

    function brand() {
        var m = /^(\w+)/.exec(String(panel.rc.model || ""))
        return m ? m[1] + " receiver" : "Receiver"
    }

    function tiles() {
        var out = [
            { id: "picture", kind: "stepper", icon: "image", label: "Picture mode",
              detail: function() { return Inputs.current ? Inputs.current.port : "" },
              options: ["Filmmaker", "Cinema", "Standard", "Vivid", "Game", "Sports", "Eco"],
              enabled: function() { return Sio.has("picture.pictureMode") },
              index: function() { var i = panel.modeValues.indexOf(String(Sio.get("picture.pictureMode"))); return i < 0 ? undefined : i },
              act: function(i) { Sio.write("picture.pictureMode", panel.modeValues[i]) } },
            { id: "receiver", kind: "nudge", icon: "boom-box", detailLines: 2, label: function() { return panel.brand() },
              detail: function() { return !panel.rcAnswers ? "Not answering" : (panel.rc.input || "Receiver") + (panel.rc.soundMode ? " \u00b7 " + panel.rc.soundMode : "") },
              value: function() { return panel.rcAnswers && panel.rc.volumeText ? String(panel.rc.volumeText) : "\u2013" },
              enabled: function() { return !gate.disabled },
              act: function(d) {
                  LS.adhoc.call("luna://com.webos.service.audio", d > 0 ? "/master/volumeUp" : "/master/volumeDown", "{}")
                  Helper.post("/v1/receiver/nudge", {})
              },
              open: function() { panel.openSettingsPage("receiver") } },
            { id: "sleep", kind: "stepper", icon: "moon", label: "Sleep timer",
              detail: function() { return Power.sleepLeft },
              options: Power.sleepLabels, index: function() { return Power.sleepIndex }, act: function(i) { Power.setSleep(i) } },
            { id: "wifi", kind: "value", icon: "wifi", label: "Wi-Fi", detail: function() { return Net.summary }, value: "", act: function() { panel.openSettingsPage("network") } },
            { id: "ai", kind: "value", icon: "bot", label: "AI access", chip: true,
              detail: function() { var a = Helper.ai; return !a ? "" : (a.listener && a.listener.enabled ? "Home network only" : "") + (a.assistants && a.assistants.length ? (a.listener && a.listener.enabled ? " \u00b7 " : "") + a.assistants.length + " paired" : "") },
              value: function() { var a = Helper.ai; return a && a.listener && a.listener.enabled ? "Allowed" : "Blocked" },
              tone: function() { var a = Helper.ai; return a && a.listener && a.listener.enabled ? "positive" : "" },
              act: function() { panel.openSettingsPage("ai") } }
        ]
        var ai = out.pop()                              // plugin tiles sit between the built-in ones and AI access (mock 10)
        return out.concat(pluginTiles.rows, [ai])
    }
}
