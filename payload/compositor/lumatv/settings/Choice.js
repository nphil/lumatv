.pragma library
.import "Rows.js" as R
.import "Names.js" as N

// Row builders for the pages that cover the rest of LG's Settings (docs/research/settings-coverage.md), beside Rows.js. Every row
// names a setting as "category.key", reads and writes through the settings service (`sio`, passed in), and shows LG's own name
// and the value in force; a second line is for state only. What is not obvious is `help`.

// LG leaves a row out when the TV's description says the key is not visible (the rows that depend on the picture, the input or the
// country); so does LumaTV. `shown` is that rule, `lgShown` puts it on a row built by Rows.js.
function shown(sio, ds) {
    var d = sio.descs[ds]
    return !sio.missing(ds) && !(d && d.ui && d.ui.visible === false)
}
function lgShown(sio, ds, row) { return R.over(row, { visible: function() { return shown(sio, ds) } }) }

// The values a choice row offers: the TV's own list for the key (its description, `values.arrayExt`, in the TV's order, the ones LG
// hides left out), each with LG's name for it. `o.values` narrows or orders the list; the TV's list still decides what is on it, so
// a value the TV does not offer is never shown or written. A key whose description has no list offers `o.values` as given.
function options(sio, o) {
    var d = sio.descs[o.ds]
    var arr = d && d.values && d.values.arrayExt ? d.values.arrayExt : null
    var out = []
    var order = o.values
    if (arr && arr.length) {
        var by = {}
        for (var i = 0; i < arr.length; i++) if (arr[i].visible !== false) by[String(arr[i].value)] = arr[i]
        if (!order) { order = []; for (var j = 0; j < arr.length; j++) if (arr[j].visible !== false) order.push(String(arr[j].value)) }
        for (var k = 0; k < order.length; k++) {
            var e = by[String(order[k])]
            if (e) out.push({ value: String(order[k]), label: N.name(o.names, order[k]), enabled: e.active !== false })
        }
        return out
    }
    order = order || []
    for (var m = 0; m < order.length; m++) out.push({ value: String(order[m]), label: N.name(o.names, order[m]), enabled: true })
    return out
}

// A row that shows the value in force and opens the chooser (PageChoice) on the TV's list. o: ds, label, icon, tab, help, names
// (a table name of Names.js), values, appId, page (the PageList that holds the row: its host opens the chooser), id.
function choice(sio, o) {
    var row = {
        kind: "value", tab: o.tab, id: o.id || o.ds, icon: o.icon, label: o.label, help: o.help, chevron: true,
        detail: R.unavailable(sio, o.ds, o.detail), visible: function() { return shown(sio, o.ds) },
        enabled: function() { return sio.usable(o.ds) },
        value: function() { return sio.has(o.ds) ? N.name(o.names, sio.get(o.ds)) : "" },
        options: function() { return options(sio, o) },
        current: function() { return String(sio.get(o.ds)) },
        pick: function(i) { var c = options(sio, o)[i]; if (c && c.enabled) sio.write(o.ds, c.value, R.app(o)) },
        act: function() { o.page.host.openPage("choice", { row: row, back: o.page.title, backId: o.page.host.current }) }
    }
    return row
}

// A short list on one row (a segmented control) built from the TV's own list for the key, in the TV's order, each value named as LG names
// it. `o.values` orders or narrows the list; the TV's list still decides what is on it, so a value the TV does not offer is never shown or
// written. o: ds, label, icon, tab, help, detail, names (a table of Names.js), values, appId, id.
function seg(sio, o) {
    function list() { return options(sio, o) }
    var row = R.seg(sio, { ds: o.ds, tab: o.tab, id: o.id, icon: o.icon, label: o.label, help: o.help, detail: o.detail, appId: o.appId, options: [], values: [] })
    row.options = function() { return list().map(function(x) { return x.label }) }
    row.index = function() {
        var l = list(), v = String(sio.get(o.ds))
        for (var i = 0; i < l.length; i++) if (l[i].value === v) return i
        return undefined
    }
    row.act = function(i) { var c = list()[i]; if (c && c.enabled) sio.write(o.ds, c.value, R.app(o)) }
    return lgShown(sio, o.ds, row)
}

// The five display switches of LG's Accessibility that exclude each other: Text with High Contrast (`highContrast`), Focus with High
// Contrast (`focusEmphasis`), Screen Zoom (`screenMagnification`), Grayscale (`blackAndWhite`) and Invert Colors (`colorInversion`).
// A switch writes the keys LG's own row writes with it: the first three switch the pair `option.adjustments` off, the last two first
// switch the zoom off and then set their half of the pair and put high contrast and focus off.
function display(sio, o) {
    var pair = o.key === "blackAndWhite" || o.key === "colorInversion"
    var ds = pair ? "option.adjustments" : "option." + o.key
    return {
        kind: "toggle", tab: o.tab, id: o.id || ds + "." + o.key, icon: o.icon, label: o.label, help: o.help,
        detail: R.unavailable(sio, ds, o.detail), visible: function() { return shown(sio, ds) },
        enabled: function() { return sio.usable(ds) },
        on: function() {
            var v = sio.get(ds)
            return pair ? !!v && v[o.key] === "on" : String(v) === "on"
        },
        act: function(v) {
            var s = v ? "on" : "off"
            if (!pair) {
                var w = { adjustments: { blackAndWhite: "off", colorInversion: "off" } }
                w[o.key] = s
                sio.writeMany("option", w)
                return
            }
            sio.writeMany("option", { screenMagnification: "off" })
            var a = { blackAndWhite: "off", colorInversion: "off" }
            a[o.key] = s
            sio.writeMany("option", { adjustments: a, highContrast: "off", focusEmphasis: "off" })
        }
    }
}

// A row that only shows: LG's name and a value (TV Information). o: icon, label, value (text or function), tab, help, id, visible.
function info(o) {
    return { kind: "value", info: true, chevron: false, tab: o.tab, id: o.id, icon: o.icon, label: o.label, help: o.help,
             value: o.value, detail: o.detail, visible: o.visible, enabled: o.enabled }
}

// A row with a button that does something once (Check for updates, Run): o: icon, label, button, tab, help, id, danger, enabled, visible,
// detail, act. Anything that cannot be undone goes through `ask`.
function action(o) {
    return { kind: "action", tab: o.tab, id: o.id, icon: o.icon, label: o.label, help: o.help, button: o.button || "Run", danger: o.danger === true,
             detail: o.detail, enabled: o.enabled, visible: o.visible, act: o.act }
}

// The function a row's `act` calls to ask before it does `go`: the existing confirm question (panels/Confirm.qml, opened by the
// host) with the safe answer first, which is also what Back means. o: icon, title, body, yes (the label of the doing answer),
// no, danger (the doing answer in the danger colour).
function ask(page, o, go) {
    return function() {
        page.host.confirm({ icon: o.icon || "circle-help", title: o.title, body: o.body || "",
                            answers: [{ id: "no", label: o.no || "Cancel" }, { id: "yes", label: o.yes, danger: o.danger === true }], focus: 0 },
                          function(id) { if (id === "yes") go() })
    }
}
