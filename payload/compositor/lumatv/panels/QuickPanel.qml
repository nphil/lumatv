import QtQuick 2.4
import WebOSServices 1.0
import "../kit"
import "../settings"

// The quick panel (docs/mocks/10-quick-panel): a sheet down the right edge with the few things reached for most,
// each one a row of the Settings engine (settings/PageList): Left and Right change a value, OK opens the page that
// has more, "All settings" opens Settings. Built once and kept, like Quick Settings after the patch that keeps
// LG's panel built: opening it is a visibility flip.
//
// The receiver tile reads the helper's `receiver` topic and moves the volume the way the volume keys do, through
// the TV (`audio/master/volumeUp` and `volumeDown`, which webOS sends to the receiver over HDMI-CEC), so the number
// it shows is always the receiver's own and the TV's counter is never involved; the helper is told to read the
// receiver now so the new level appears at once. OK on the tile opens Settings at the receiver's page.
// The sleep timer tile steps through LG's choices (kit/Power) and shows what is left while one runs.
//
// Tiles of plugins (PLAN, Plugins: `plugin.<id>.tiles`, settings/PluginTiles) follow the built-in ones; OK on one is
// a POST to the plugin's action route. "All settings" is the button under the list, one Down past the last tile.
Item {
    id: panel
    anchors.fill: parent
    property bool rested: true
    property string zone: "page"                        // PageList reads host.zone: the panel has only the one zone
    signal openSettingsPage(string id)
    signal closeRequested()
    signal launchApp(string appId)
    readonly property bool built: true
    // the receiver's reading, only while it is in date (kit/ReceiverGate)
    ReceiverGate { id: gate; active: panel.visible }
    readonly property var rc: gate.doc || Helper.receiver || ({})
    readonly property bool rcAnswers: gate.fresh
    property bool footFocus: false                      // the All settings button has the focus, not a tile
    property var clock: new Date()
    Timer { interval: 15000; running: panel.visible; repeat: true; onTriggered: panel.clock = new Date() }
    onVisibleChanged: if (visible) { clock = new Date(); footFocus = false; touched = false; Sio.read("picture", ["pictureMode"]); Inputs.refresh(); Power.readSleep(); list.reset() }
    onFootFocusChanged: zone = footFocus ? "foot" : "page"
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
            else if (dir === "ok") openSettingsPage("")
            else if (dir === "back") closeRequested()
            return
        }
        // Down past the last tile lands on the button; PageList itself stops at its last row
        if (dir === "down" && list.stop >= 0 && list.step(list.stop, 1) < 0) { footFocus = true; return }
        if (list.key(dir)) return
        if (dir === "back") closeRequested()
    }
    function patch(p) { }                              // PageList's host contract; the panel changes no theme

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

    Rectangle { anchors.fill: parent; color: Lu.scrim }
    LuPane {
        id: sheet
        x: 1920 - 72 - width; y: 56
        width: 644
        // as tall as its tiles (mock 10), up to the screen; PageList scrolls what does not fit
        readonly property int tileH: Lu.row + Lu.space(2)
        height: Math.min(1080 - 56 - 80, list.y + listHead + list.rows.length * tileH + Lu.space(3) + footer.height + footHint.height + Lu.space(7))
        readonly property int listHead: Math.round(Lu.target * 1.3)
        PageList {
            id: list
            x: Lu.space(6); y: Lu.space(5)
            width: parent.width - 2 * Lu.space(6)
            height: sheet.height - 2 * Lu.space(5) - footer.height - footHint.height - Lu.space(3)
            host: panel
            title: "Quick settings"
            rows: panel.tiles()
        }
        LuText {
            anchors.right: list.right
            y: list.y + Math.round(Lu.target * 0.12)
            role: "label"; color: Lu.ink2
            text: Qt.formatTime(panel.clock, "h:mm AP")
        }
        LuButton {
            id: footer
            x: Lu.space(6); width: parent.width - 2 * Lu.space(6)
            y: list.y + list.height + Lu.space(3)
            text: "All settings"
            focused: panel.footFocus; rested: panel.rested
            onActivated: { panel.footFocus = true; panel.openSettingsPage("") }
            onHoverFocus: panel.footFocus = true
        }
        LuText { id: footLabel; visible: false; role: "label"; text: footer.text }
        LuIcon { x: footer.x + (footer.width - footLabel.implicitWidth) / 2 - width - Lu.space(3); y: footer.y + (footer.height - height) / 2
                 z: 2; scale: footer.scale; name: "settings"; size: 28; color: Lu.ink }
        LuText {
            id: footHint
            x: Lu.space(6); width: parent.width - 2 * Lu.space(6); y: footer.y + footer.height + Lu.space(3)
            role: "caption"; color: Lu.ink2; horizontalAlignment: Text.AlignHCenter
            text: "Left and right change a value \u00b7 Back closes"
        }
    }

    function brand() {
        var m = /^(\w+)/.exec(String(panel.rc.model || ""))
        return m ? m[1] + " receiver" : "Receiver"
    }

    function tiles() {
        var out = [
            { id: "picture", kind: "stepper", icon: "image", label: "Picture mode",
              detail: function() { return Inputs.current ? Inputs.current.port : "Picture" },
              options: ["Filmmaker", "Cinema", "Standard", "Vivid", "Game", "Sports", "Eco"],
              enabled: function() { return Sio.has("picture.pictureMode") },
              index: function() { var i = panel.modeValues.indexOf(String(Sio.get("picture.pictureMode"))); return i < 0 ? undefined : i },
              act: function(i) { Sio.write("picture.pictureMode", panel.modeValues[i]) } },
            { id: "receiver", kind: "nudge", icon: "boom-box", label: function() { return panel.brand() },
              detail: function() { return !panel.rcAnswers ? "Not answering" : (panel.rc.input || "Receiver") + (panel.rc.soundMode ? " \u00b7 " + panel.rc.soundMode : "") },
              value: function() { return panel.rcAnswers && panel.rc.volumeText ? String(panel.rc.volumeText) : "\u2013" },
              enabled: function() { return !gate.disabled },
              act: function(d) {
                  LS.adhoc.call("luna://com.webos.service.audio", d > 0 ? "/master/volumeUp" : "/master/volumeDown", "{}")
                  Helper.post("/v1/receiver/nudge", {})
              },
              open: function() { panel.openSettingsPage("receiver") } },
            { id: "sleep", kind: "stepper", icon: "moon", label: "Sleep timer",
              detail: function() { return Power.sleepLeft !== "" ? Power.sleepLeft : "TV stays on" },
              options: Power.sleepLabels, index: function() { return Power.sleepIndex }, act: function(i) { Power.setSleep(i) } },
            { id: "wifi", kind: "value", icon: "wifi", label: "Wi-Fi", detail: function() { return Net.summary }, value: "", act: function() { panel.openSettingsPage("network") } },
            { id: "ai", kind: "value", icon: "bot", label: "AI access",
              detail: function() { var a = Helper.ai; return !a ? "Off" : (a.listener && a.listener.enabled ? "LAN only" : "Off") + " \u00b7 " + (a.assistants || []).length + " paired" },
              value: function() { var a = Helper.ai; return a && a.listener && a.listener.enabled ? "On" : "Off" },
              act: function() { panel.openSettingsPage("ai") } }
        ]
        var ai = out.pop()                              // plugin tiles sit between the built-in ones and AI access (mock 10)
        return out.concat(pluginTiles.rows, [ai])
    }
}
