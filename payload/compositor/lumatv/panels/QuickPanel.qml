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
//
// Tiles of plugins (PLAN, Plugins: `plugin.<id>.tiles`) are appended below the built-in ones (settings/PluginTiles
// builds the descriptors; a tap is a POST to the plugin's action route).
Item {
    id: panel
    anchors.fill: parent
    property bool rested: true
    property string zone: "page"                        // PageList reads host.zone: the panel has only the one zone
    signal openSettingsPage(string id)
    signal closeRequested()
    signal launchApp(string appId)
    readonly property bool built: true
    readonly property var rc: Helper.receiver || ({})
    readonly property bool rcAnswers: !!(rc.reachable && rc.fresh)
    property var clock: new Date()
    Timer { interval: 15000; running: panel.visible; repeat: true; onTriggered: panel.clock = new Date() }
    onVisibleChanged: if (visible) { clock = new Date(); Sio.read("picture", ["pictureMode"]); Inputs.refresh(); list.reset() }

    function key(dir) {
        if (list.key(dir)) return
        if (dir === "back") closeRequested()
    }
    function patch(p) { }                              // PageList's host contract; the panel changes no theme
    property var plugins: []                            // tile descriptors from the broker (Phase 9)

    readonly property var modeValues: ["filmMaker", "cinema", "normal", "vivid", "game", "sports", "eco"]
    readonly property var modeNames: ({ filmMaker: "Filmmaker", cinema: "Cinema", normal: "Standard", vivid: "Vivid", game: "Game", sports: "Sports", eco: "Eco" })

    Rectangle { anchors.fill: parent; color: Lu.scrim }
    LuPane {
        id: sheet
        x: 1920 - 64 - width; y: 56
        width: 640; height: 1080 - 56 - 120
        PageList {
            id: list
            x: Lu.space(6); y: Lu.space(5)
            width: parent.width - 2 * Lu.space(6); height: parent.height - 2 * Lu.space(5) - 56
            host: panel
            title: "Quick settings"
            subtitle: Qt.formatTime(panel.clock, "h:mm AP")
            rows: panel.tiles()
        }
        LuText {
            x: Lu.space(6); width: parent.width - 2 * Lu.space(6); y: parent.height - 44
            role: "caption"; color: Lu.ink2; horizontalAlignment: Text.AlignHCenter
            text: "Left and right change a value \u00b7 Back closes"
        }
    }

    function tiles() {
        var out = [
            { kind: "stepper", icon: "image", label: "Picture mode",
              detail: function() { return Inputs.current ? Inputs.current.port : "Picture" },
              options: ["Filmmaker", "Cinema", "Standard", "Vivid", "Game", "Sports", "Eco"],
              enabled: function() { return Sio.has("picture.pictureMode") },
              index: function() { var i = panel.modeValues.indexOf(String(Sio.get("picture.pictureMode"))); return i < 0 ? undefined : i },
              act: function(i) { Sio.write("picture.pictureMode", panel.modeValues[i]) } },
            { kind: "nudge", icon: "boom-box", label: "Receiver",
              detail: function() { return !panel.rcAnswers ? "Not answering" : (panel.rc.input || "Receiver") + (panel.rc.soundMode ? " \u00b7 " + panel.rc.soundMode : "") },
              value: function() { return panel.rcAnswers && panel.rc.volumeText ? String(panel.rc.volumeText) : "\u2013" },
              enabled: function() { return panel.rc.enabled !== false },
              act: function(d) {
                  LS.adhoc.call("luna://com.webos.service.audio", d > 0 ? "/master/volumeUp" : "/master/volumeDown", "{}")
                  Helper.post("/v1/receiver/nudge", {})
              },
              open: function() { panel.openSettingsPage("receiver") } },
            { kind: "value", icon: "wifi", label: "Wi-Fi", detail: function() { return Net.summary }, value: "", act: function() { panel.openSettingsPage("network") } },
            { kind: "value", icon: "bot", label: "AI access",
              detail: function() { var a = Helper.ai; return !a ? "Off" : (a.listener && a.listener.enabled ? "LAN only" : "Off") + " \u00b7 " + (a.assistants || []).length + " paired" },
              value: function() { var a = Helper.ai; return a && a.listener && a.listener.enabled ? "On" : "Off" },
              act: function() { panel.openSettingsPage("ai") } }
        ]
        out = out.concat(panel.plugins)
        out.push({ kind: "value", icon: "settings", label: "All settings", detail: "Picture, sound, inputs, network and more", value: "", act: function() { panel.openSettingsPage("") } })
        return out
    }
}
