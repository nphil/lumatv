import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Fine Tune (LG: Picture › Advanced Settings › Color › Fine Tune): one colour of the six at a time, its saturation, its tint (LG's key says Hue) and its
// luminance. LG's keys: Select Color `colorManagementColorSystem` (red, green, blue, cyan, magenta, yellow) and, for the colour chosen,
// `colorManagementSaturation<Colour>`, `colorManagementHue<Colour>` and `colorManagementLuminance<Colour>` (each -30 to 30 on this set, from the
// key's description). All answer only while Color Adjustment (`picture.dynamicColor`) is User. A change is sent with `pictureSettingModified`
// (LG marks the picture mode as changed). Reset is LG's: `resetSystemSettings` with the keys the TV itself lists in `keyListForCopyCms`.
PageList {
    id: page
    title: "Fine Tune"
    back: "Picture"

    readonly property bool user: String(Sio.get("picture.dynamicColor")) === "user"
    readonly property string colour: String(Sio.get("picture.colorManagementColorSystem"))
    readonly property string suffix: colour.length ? colour.charAt(0).toUpperCase() + colour.substring(1) : ""
    readonly property var resetKeys: { var k = Sio.get("picture.keyListForCopyCms"); return Array.isArray(k) ? k : [] }

    LunaCall { id: luna }

    function refresh() {
        Sio.read("picture", ["pictureMode", "pictureSettingModified", "keyListForCopyCms", "dynamicColor", "colorManagementColorSystem"])
        var keys = []
        var names = ["Red", "Green", "Blue", "Cyan", "Magenta", "Yellow"]
        for (var i = 0; i < names.length; i++) keys.push("colorManagementSaturation" + names[i], "colorManagementHue" + names[i], "colorManagementLuminance" + names[i])
        Sio.read("picture", keys)
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    function slider(part, label, icon, help) {
        function key() { return "picture.colorManagement" + part + page.suffix }
        var r = R.slider(Sio, { ds: "picture.colorManagement" + part + "Red", tab: 0, id: part, icon: icon, label: label, help: help, min: -30, max: 30, step: 1 })
        r.visible = function() { return page.suffix !== "" && !Sio.missing(key()) && C.shown(Sio, key()) }
        r.enabled = function() { return page.user && Sio.usable(key()) }
        r.detail = function() { return !Sio.has(key()) ? "TV not answering" : page.user ? "" : "Set Color Adjustment to User" }
        r.min = function() { var d = Sio.descs[key()]; return d && d.values && d.values.range && d.values.range.max !== undefined ? d.values.range.min : -30 }
        r.max = function() { var d = Sio.descs[key()]; return d && d.values && d.values.range && d.values.range.max !== undefined ? d.values.range.max : 30 }
        r.num = function() { return Sio.num(key(), 0) }
        r.valueText = function() { return Sio.has(key()) ? String(Math.round(Sio.num(key(), 0))) : "\u2013" }
        r.act = function(v) {
            var w = {}
            w["colorManagement" + part + page.suffix] = String(v)
            var m = C.modified(Sio)
            if (m) w.pictureSettingModified = m
            Sio.writeMany("picture", w)
        }
        return r
    }

    rows: [
        R.over(C.choice(Sio, { ds: "picture.colorManagementColorSystem", tab: 0, icon: "palette", label: "Select Color", names: "colorSystem", page: page,
                              help: "The colour the three rows below adjust. They work while Color Adjustment is User." }),
               { enabled: function() { return page.user && Sio.usable("picture.colorManagementColorSystem") },
                 detail: function() { return !Sio.has("picture.colorManagementColorSystem") ? "TV not answering" : page.user ? "" : "Set Color Adjustment to User" } }),
        slider("Saturation", "Adjust Saturation", "blend", "Makes the chosen colour clearer or duller."),
        slider("Hue", "Adjust Tint", "sliders-horizontal", "Turns the chosen colour towards its neighbour on the colour wheel."),
        slider("Luminance", "Adjust Luminance", "sun", "Makes the chosen colour lighter or darker."),
        C.action({ id: "reset", tab: 0, icon: "rotate-ccw", label: "Reset", button: "Reset", danger: true,
                   enabled: function() { return page.resetKeys.length > 0 },
                   detail: function() { return Sio.has("picture.keyListForCopyCms") ? "" : "TV not answering" },
                   help: "Puts the six colours' saturation, tint and luminance back to LG's.",
                   act: C.ask(page, { icon: "rotate-ccw", title: "Reset Fine Tune?", body: "The six colours go back to LG's.", yes: "Reset", no: "Cancel", danger: true },
                              function() { luna.act(Sio.service, "resetSystemSettings", { category: "picture", keys: page.resetKeys }); page.refresh() }) })
    ]
}
