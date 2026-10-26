import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0
import "../kit"
import "../settings"

// The quick panel (docs/mocks/10-quick-panel, laid out as LG's Quick Settings is): a tall narrow sheet docked to the left edge, 492 by 1008,
// 24 from the edges (measured on LG's own panel, caps/luma/qs_res_warm.png), with the few things reached for most, one row each (QuickList):
// Left and Right change a value, OK opens the page that has more. It slides out from the left edge and back (PanelMotion: a transform and
// a fade, no layout while it moves; reduced motion is a fade). Built once and kept, like Quick Settings after the patch that keeps LG's
// panel built: opening it is a visibility flip and the start of the slide.
//
// The receiver tile reads the helper's `receiver` topic and moves the volume the way the volume keys do, through the TV (`audio/master/
// volumeUp` and `volumeDown`, which webOS sends to the receiver over HDMI-CEC), so the number it shows is always the receiver's own and the
// TV's counter is never involved; the helper is told to read the receiver now so the new level appears at once. OK on the tile opens
// Settings at the receiver's page. The sleep timer tile steps through LG's choices (kit/Power) and shows what is left while one runs.
//
// Tiles of plugins (PLAN, Plugins: `plugin.<id>.tiles`, settings/PluginTiles) follow the built-in ones; OK on one is a POST to the
// plugin's action route. "All settings" and the button for LG's own Quick Settings are under the list, one Down past the last tile.
Item {
    id: panel
    anchors.fill: parent
    property bool rested: true
    property string zone: "page"                        // the list reads host.zone: the rows or the foot buttons have the keys
    signal openSettingsPage(string id)
    signal openLgQuick()                                // LG's own Quick Settings, for what this panel does not have
    signal closeRequested()
    signal launchApp(string appId)
    readonly property bool built: true

    // Resident says when the panel is wanted (`presented`) and keeps it drawn while `leaving`
    property bool presented: false
    readonly property bool leaving: motion.leaving
    PanelMotion { id: motion; presented: panel.presented }
    readonly property real progress: motion.progress     // 0 away, 1 shown
    readonly property real slideX: slide.x               // where the sheet is (24 at rest), for the tests and the gate's step list
    readonly property real slideY: slide.y

    // LG's panel, measured: 492 wide, 1008 tall, its left and top edges 24 from the screen's
    readonly property int sheetW: 492
    readonly property int sheetH: 1008
    readonly property int edge: 24
    readonly property int pad: Lu.space(6)
    readonly property int inner: sheetW - 2 * pad

    // the receiver's reading, only while it is in date (kit/ReceiverGate)
    ReceiverGate { id: gate; active: panel.visible }
    readonly property var rc: gate.doc || Helper.receiver || ({})
    readonly property bool rcAnswers: gate.fresh
    property bool footFocus: false                      // a button under the list has the focus, not a tile
    property int footIndex: 0                           // 0 All settings, 1 LG's Quick Settings
    property var clock: new Date()
    Timer { interval: 15000; running: panel.visible; repeat: true; onTriggered: panel.clock = new Date() }
    onPresentedChanged: { if (presented) refresh(); else readTimer.stop() }
    // What the open path does is only what the first frame needs. The readings (each an answer that rebinds the rows and can change a
    // line's wrapping) are asked for once the slide is over and the frame burst the key started has ended: the rows show what the
    // last reading said for the first 170 ms, and a reading that differs redraws them after.
    function refresh() {
        clock = new Date(); footFocus = false; footIndex = 0; touched = false
        list.reset()
        readTimer.restart()
    }
    Timer {
        id: readTimer
        interval: motion.enterMs + 60
        onTriggered: { Sio.read("picture", ["pictureMode"]); Inputs.refresh(); Power.readSleep() }
    }
    onFootFocusChanged: zone = footFocus ? "foot" : "page"
    onZoneChanged: footFocus = (zone === "foot")
    // The first tile (Picture mode) is only focusable once the settings service has answered; until a key is pressed the
    // focus goes to it when it can.
    property bool touched: false
    Connections {
        target: Sio
        onAnswersChanged: if (panel.visible && !panel.touched) list.enter(list.first())
    }

    function key(dir) {
        touched = true
        if (footFocus) {
            if (dir === "up") footFocus = false
            else if (dir === "left") footIndex = 0
            else if (dir === "right") footIndex = 1
            else if (dir === "ok") { if (footIndex === 0) openSettingsPage(""); else openLgQuick() }
            else if (dir === "back") closeRequested()
            return
        }
        // Down past the last tile lands on the buttons; the list itself stops at its last row
        if (dir === "down" && list.stop >= 0 && list.step(list.stop, 1) < 0) { footIndex = 0; footFocus = true; return }
        if (list.key(dir)) return
        if (dir === "back") closeRequested()
    }
    function patch(p) { }                              // the list's host contract; the panel changes no theme

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

    // behind the sheet: the page dims as the panel arrives
    Rectangle { anchors.fill: parent; color: Lu.scrim; opacity: motion.progress }

    // The one thing that moves: the sheet slides out of the left edge and fades in. Only x and opacity change per frame, so nothing is
    // laid out while it moves; the glass finds its place in the wallpaper's blur again as it goes (LuPane.locate).
    Item {
        id: slide
        readonly property real travel: panel.edge + panel.sheetW
        x: panel.edge - (Lu.reduced ? 0 : travel * (1 - motion.progress))
        y: panel.edge
        width: panel.sheetW
        height: panel.sheetH
        opacity: Lu.reduced ? motion.progress : Math.min(1, motion.progress * 2.5)
        onXChanged: if (motion.moving) sheet.locate()

        LuPane {
            id: sheet
            anchors.fill: parent
            Item {
                id: head
                x: panel.pad; y: panel.pad
                width: panel.inner
                // the clock sits beside the title while both fit on a line, else under it (a large text size)
                readonly property bool clockBelow: titleProbe.implicitWidth + clockText.implicitWidth + Lu.space(4) > width
                height: clockBelow ? titleText.height + clockText.height : Math.max(titleText.height, clockText.height)
                LuText { id: titleText; width: parent.width - (head.clockBelow ? 0 : clockText.width + Lu.space(4)); role: "title"; text: "Quick settings"; wrapMode: Text.WordWrap }
                LuText { id: titleProbe; visible: false; role: "title"; text: titleText.text }
                LuText {
                    id: clockText
                    x: head.clockBelow ? 0 : parent.width - width
                    y: head.clockBelow ? titleText.height : Math.round((titleText.height - height) / 2)
                    role: "label"; color: Lu.ink2
                    text: Qt.formatTime(panel.clock, "h:mm AP")
                }
            }
            QuickList {
                id: list
                x: panel.pad; y: head.y + head.height + Lu.space(4)
                width: panel.inner
                height: foot.y - Lu.space(4) - y
                host: panel
                rested: panel.rested
                rows: panel.tiles()
            }
            Item {
                id: foot
                x: panel.pad
                width: panel.inner
                height: stacked ? 2 * allSettings.height + Lu.space(2) : allSettings.height
                y: sheet.height - panel.pad - height
                // side by side while both labels fit, else one above the other: at a large text size nothing overlaps
                readonly property bool stacked: lgButton.implicitWidth + Lu.space(3) + allSettings.implicitWidth > width
                LuButton {
                    id: allSettings
                    width: foot.stacked ? foot.width : foot.width - lgButton.width - Lu.space(3)
                    text: "All settings"; icon: "settings"
                    focused: panel.footFocus && panel.footIndex === 0; rested: panel.rested
                    onActivated: { panel.footIndex = 0; panel.footFocus = true; panel.openSettingsPage("") }
                    onHoverFocus: { panel.footIndex = 0; panel.footFocus = true }
                }
                LuButton {
                    id: lgButton
                    x: foot.stacked ? 0 : foot.width - width
                    y: foot.stacked ? allSettings.height + Lu.space(2) : 0
                    width: foot.stacked ? foot.width : implicitWidth
                    text: "LG"; icon: "tv"
                    focused: panel.footFocus && panel.footIndex === 1; rested: panel.rested
                    onActivated: { panel.footIndex = 1; panel.footFocus = true; panel.openLgQuick() }
                    onHoverFocus: { panel.footIndex = 1; panel.footFocus = true }
                }
            }
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
              detail: function() { var a = Helper.ai; return !a ? "" : (a.listener && a.listener.enabled ? "LAN only" : "") + (a.assistants && a.assistants.length ? (a.listener && a.listener.enabled ? " \u00b7 " : "") + a.assistants.length + " paired" : "") },
              value: function() { var a = Helper.ai; return a && a.listener && a.listener.enabled ? "On" : "Off" },
              tone: function() { var a = Helper.ai; return a && a.listener && a.listener.enabled ? "positive" : "" },
              act: function() { panel.openSettingsPage("ai") } }
        ]
        var ai = out.pop()                              // plugin tiles sit between the built-in ones and AI access (mock 10)
        return out.concat(pluginTiles.rows, [ai])
    }
}
