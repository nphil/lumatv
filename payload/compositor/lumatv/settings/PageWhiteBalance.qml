import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// White balance (LG: Picture › Advanced Settings › Color › White Balance). LG's keys, in the picture category of the picture mode in force:
// Method `whiteBalanceMethod` (2, 10 or 22 points), and then, by the method:
//   2 points: Point `whiteBalancePoint` (low or high) and Red, Green, Blue as `whiteBalance<Colour>Offset` (low) or `...Gain` (high), a number each;
//   10 or 22 points: the signal level `whiteBalanceIre10pt` or `whiteBalanceIre` (a list of percentages the TV gives), Luminance
//   `adjustingLuminance10pt` or `adjustingLuminance`, and Red, Green, Blue as `whiteBalance<Colour>10pt` or `whiteBalance<Colour>`, each a list of
//   10 or 22 numbers, one per level: a change sends the whole list with that level's number changed, as LG's own row does.
// Luminance at the maximum signal is `whiteBalanceLuminance`, answered only at 100 %. A change is sent with `pictureSettingModified` (LG marks the
// mode as changed). Reset is LG's: `resetSystemSettings` with the keys the TV itself lists in `keyListForCopyWb`. The code-value methods and
// rows are test-pattern drives LG hides on this set and are not offered.
PageList {
    id: page
    title: "White balance"
    back: "Picture"

    readonly property string method: String(Sio.get("picture.whiteBalanceMethod"))
    readonly property bool two: method === "2"
    readonly property bool high: String(Sio.get("picture.whiteBalancePoint")) === "high"
    readonly property string ireKey: method === "22" ? "whiteBalanceIre" : "whiteBalanceIre10pt"
    readonly property var levels: {
        var d = Sio.descs["picture." + ireKey]
        var out = []
        var arr = d && d.values && d.values.arrayExt ? d.values.arrayExt : []
        for (var i = 0; i < arr.length; i++) if (arr[i].visible !== false) out.push(String(arr[i].value))
        return out
    }
    readonly property int level: levels.indexOf(String(Sio.get("picture." + ireKey)))
    readonly property string levelText: level < 0 ? "" : levels[level] + "%"
    readonly property var resetKeys: { var k = Sio.get("picture.keyListForCopyWb"); return Array.isArray(k) ? k : [] }

    LunaCall { id: luna }

    function refresh() {
        Sio.read("picture", ["pictureMode", "pictureSettingModified", "keyListForCopyWb", "whiteBalanceMethod", "whiteBalancePoint", "whiteBalanceIre", "whiteBalanceIre10pt",
                             "whiteBalanceLuminance"])
        Sio.read("picture", ["whiteBalanceRed", "whiteBalanceGreen", "whiteBalanceBlue", "adjustingLuminance", "whiteBalanceRed10pt", "whiteBalanceGreen10pt",
                             "whiteBalanceBlue10pt", "adjustingLuminance10pt"])
        Sio.read("picture", ["whiteBalanceRedGain", "whiteBalanceGreenGain", "whiteBalanceBlueGain", "whiteBalanceRedOffset", "whiteBalanceGreenOffset", "whiteBalanceBlueOffset"])
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    // the key that holds a colour's number now (and whether it is a list)
    function colourKey(name) { return two ? "whiteBalance" + name + (high ? "Gain" : "Offset") : method === "10" ? "whiteBalance" + name + "10pt" : "whiteBalance" + name }
    function lumKey() { return method === "10" ? "adjustingLuminance10pt" : "adjustingLuminance" }
    function number(key) {
        var v = Sio.get("picture." + key)
        var n = Array.isArray(v) ? Number(v[level < 0 ? 0 : level]) : Number(v)
        return isNaN(n) ? 0 : n
    }
    function setNumber(key, n) {
        var v = Sio.get("picture." + key)
        var w = {}
        if (Array.isArray(v)) { var a = v.slice(); a[level < 0 ? 0 : level] = n; w[key] = a } else w[key] = String(n)
        var m = C.modified(Sio)
        if (m) w.pictureSettingModified = m
        Sio.writeMany("picture", w)
    }
    function setLevel(d) {
        if (levels.length === 0) return
        var i = Math.max(0, Math.min(levels.length - 1, (level < 0 ? levels.length - 1 : level) + d))
        var w = {}
        w[ireKey] = levels[i]
        Sio.writeMany("picture", w)
    }
    function slider(id, label, icon, key, help) {
        var r = R.slider(Sio, { ds: "picture." + id, tab: 0, id: id, icon: icon, label: label, help: help, min: -50, max: 50, step: 1 })
        var lim = function() { var d = Sio.descs["picture." + (typeof key === "function" ? key() : key)]; return d && d.values && d.values.range && d.values.range.max !== undefined ? d.values.range : null }
        var k = function() { return typeof key === "function" ? key() : key }
        r.visible = function() { return !Sio.missing("picture." + k()) && C.shown(Sio, "picture." + k()) }
        r.enabled = function() { return Sio.usable("picture." + k()) }
        r.detail = function() { return Sio.has("picture." + k()) ? "" : "TV not answering" }
        r.min = function() { var l = lim(); return l ? l.min : -50 }
        r.max = function() { var l = lim(); return l ? l.max : 50 }
        r.num = function() { return page.number(k()) }
        r.valueText = function() { return Sio.has("picture." + k()) ? String(page.number(k())) : "\u2013" }
        r.act = function(v) { page.setNumber(k(), v) }
        return r
    }

    rows: [
        C.choice(Sio, { ds: "picture.whiteBalanceMethod", tab: 0, icon: "sliders-horizontal", label: "Method", names: "wbMethod", page: page,
                        help: "How many points of the grey scale are adjusted. More points, finer control." }),
        R.over(C.seg(Sio, { ds: "picture.whiteBalancePoint", tab: 0, icon: "contrast", label: "Point", names: "wbPoint",
                            help: "Low adjusts the dark end of the grey scale, High the bright end." }), { visible: function() { return page.two && C.shown(Sio, "picture.whiteBalancePoint") } }),
        { kind: "nudge", tab: 0, id: "level", icon: "gauge", label: "Signal Level(%)", help: "The brightness of the grey the rows below adjust.",
          visible: function() { return !page.two && page.levels.length > 0 },
          enabled: function() { return Sio.usable("picture." + page.ireKey) },
          value: function() { return page.levelText },
          act: function(d) { page.setLevel(d) } },
        R.over(slider("lum", "Luminance", "sun", function() { return page.lumKey() }, "Makes the grey at this signal level lighter or darker."),
               { visible: function() { return !page.two && !Sio.missing("picture." + page.lumKey()) } }),
        slider("red", "Red", "palette", function() { return page.colourKey("Red") }, "Less or more red in the grey at this level."),
        slider("green", "Green", "palette", function() { return page.colourKey("Green") }, "Less or more green in the grey at this level."),
        slider("blue", "Blue", "palette", function() { return page.colourKey("Blue") }, "Less or more blue in the grey at this level."),
        R.over(R.slider(Sio, { ds: "picture.whiteBalanceLuminance", tab: 0, icon: "sun", label: "Brightness at 100%", min: 50, max: 500, step: 1,
                               help: "How bright the white at the top of the scale is. It answers only at the 100% signal level." }),
               { visible: function() { return !page.two && C.shown(Sio, "picture.whiteBalanceLuminance") },
                 enabled: function() { return Sio.usable("picture.whiteBalanceLuminance") && page.levels.length > 0 && page.levels[page.level] === "100" },
                 detail: function() { return !Sio.has("picture.whiteBalanceLuminance") ? "TV not answering" : page.levels[page.level] === "100" ? "" : "Only at 100%" },
                 act: function(v) { page.setNumber("whiteBalanceLuminance", v) } }),
        C.action({ id: "reset", tab: 0, icon: "rotate-ccw", label: "Reset", button: "Reset", danger: true,
                   enabled: function() { return page.resetKeys.length > 0 },
                   detail: function() { return Sio.has("picture.keyListForCopyWb") ? "" : "TV not answering" },
                   help: "Puts the white balance back to LG's.",
                   act: C.ask(page, { icon: "rotate-ccw", title: "Reset the white balance?", body: "Its settings go back to LG's.", yes: "Reset", no: "Cancel", danger: true },
                              function() { luna.act(Sio.service, "resetSystemSettings", { category: "picture", keys: page.resetKeys }); page.refresh() }) })
    ]
}
