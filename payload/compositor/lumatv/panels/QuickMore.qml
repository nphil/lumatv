import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0
import "../kit"

// What the quick panel can show beyond its own list, and what its popups say (docs/mocks/29-qp-add, 29-qp-picker, 29-qp-slider).
//   rows()        the tiles "Add a tile" offers: each is the settings page's row for the same TV setting, as a quick-panel descriptor (QuickList).
//                 None is on the panel until the owner adds it (Edit tiles), and each is read when the panel opens (QuickPanel.readTimer).
//   pickExtras(id) the words and marks a value picker adds to a tile that has choices: what it is for, the glyph of a row, and the grey words at
//                 a row's right (when the sleep timer would turn the TV off).
// A tile with a `popup` field opens that card on OK (a slider for OLED pixel brightness); a stepper opens a value picker made from its own
// options. Nothing here writes a setting except through the same Sio the Settings pages use.
QtObject {
    id: more
    property var panel

    readonly property var energyNames: ["Auto", "Off", "Minimum", "Medium", "Maximum"]
    readonly property var energyValues: ["auto", "off", "min", "med", "max"]

    function rows() {
        return [
            { id: "screenoff", kind: "action", icon: "monitor", label: "Screen off, sound on", button: "Turn off",
              enabled: function() { return Sio.usable("option.screenOff") },
              act: function() { Sio.write("option.screenOff", "on"); if (panel) panel.closeRequested() } },
            { id: "energy", kind: "stepper", icon: "leaf", label: "Energy saving",
              options: more.energyNames,
              enabled: function() { return Sio.has("picture.energySaving") },
              index: function() { var i = more.energyValues.indexOf(String(Sio.get("picture.energySaving"))); return i < 0 ? undefined : i },
              act: function(i) { Sio.write("picture.energySaving", more.energyValues[i]) } },
            { id: "oled", kind: "nudge", icon: "sun", label: "OLED pixel brightness",
              enabled: function() { return Sio.usable("picture.backlight") },
              value: function() { return Sio.has("picture.backlight") ? String(Math.round(Sio.num("picture.backlight", 0))) : "\u2013" },
              act: function(d) { more.setLevel(Sio.num("picture.backlight", 0) + d * 5) },
              popup: { kind: "slider", title: "OLED pixel brightness", text: "How bright the screen is. Left and right change it.",
                       from: 0, to: 100, step: 5, marks: ["0", "50", "100"], icons: ["sun-dim", "sun"],
                       value: function() { return Sio.num("picture.backlight", 0) }, set: function(v) { more.setLevel(v) } } },
            { id: "pointer", kind: "toggle", icon: "mouse-pointer-2", label: "Magic Remote pointer",
              enabled: function() { return Sio.usable("general.remotePointer") },
              on: function() { return String(Sio.get("general.remotePointer")) === "on" },
              act: function(v) { Sio.write("general.remotePointer", v ? "on" : "off") } },
            { id: "eye", kind: "toggle", icon: "eye", label: "Eye comfort",
              enabled: function() { return Sio.usable("picture.eyeComfortMode") },
              on: function() { return String(Sio.get("picture.eyeComfortMode")) === "on" },
              act: function(v) { Sio.write("picture.eyeComfortMode", v ? "on" : "off") } },
            { id: "bluetooth", kind: "value", icon: "headphones", label: "Bluetooth audio", detail: "Speakers and headphones", value: "",
              act: function() { if (panel) panel.openSettingsPage("sound") } }
        ]
    }
    // The keys the candidates read: the panel asks for them when it opens, with the picture mode's
    readonly property var pictureKeys: ["backlight", "energySaving", "eyeComfortMode"]

    function setLevel(v) { Sio.write("picture.backlight", Math.max(0, Math.min(100, Math.round(v)))) }

    function pickExtras(id) {
        if (id === "sleep") return { text: "The TV turns off after this long. A minute before, a toast lets you cancel.", icon: "clock",
                                     detail: function(i) { return i > 0 ? Qt.formatTime(new Date(Date.now() + Power.sleepMinutes[i] * 60000), "h:mm AP") : "" } }
        if (id === "picture") return { text: "How the picture looks. Each input keeps its own.", icon: "image" }
        if (id === "energy") return { text: "How far the TV lowers the picture's brightness to save power.", icon: "leaf" }
        return {}
    }
}
