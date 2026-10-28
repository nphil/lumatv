.pragma library

// Builders for the row descriptors PageList draws. A page names a setting as "category.key" and gets a row
// that reads the settings service (`sio`, passed in: a QML singleton cannot be imported here) and writes
// it back. A row whose key the service has not answered is dimmed ("TV not answering") rather than
// shown with a made-up value, one the service says it has no such key for is left out (as LG's own app
// leaves out a row its service lacks), and one the service describes as not changeable now is dimmed too
// ("Not available now"): `enabled` is `sio.usable(ds)`, `visible` is `!sio.missing(ds)`. `o.appId` (a
// function or a string), for the few per-app settings, is what the read and the write name. The functions
// run inside the binding that draws the row, so a new answer redraws it.
//
// Text (Nitin, 2026-10-25: minimal text on every screen): a row is its `label` and its value. `o.detail` is a second line
// for state the person needs and nothing else (a dimmed row's reason, "HDMI 2"), never a sentence about what the setting does;
// what is not obvious is `o.help`, one or two plain sentences behind the row's info mark (kit/LuInfo, kit/LuHelp).

function unavailable(sio, ds, detail) {
    return function() {
        if (!sio.has(ds)) return "TV not answering"
        return sio.usable(ds) ? (typeof detail === "function" ? detail() : detail) : "Not available now"
    }
}
function present(sio, ds) { return function() { return !sio.missing(ds) } }
function app(o) { return typeof o.appId === "function" ? o.appId() : o.appId }

// o: ds, icon, label, help, detail, tab, on ("on"), off ("off")
function toggle(sio, o) {
    var onv = o.on || "on", offv = o.off || "off"
    return {
        kind: "toggle", tab: o.tab, id: o.id || o.ds, icon: o.icon, label: o.label, help: o.help,
        detail: unavailable(sio, o.ds, o.detail), visible: present(sio, o.ds),
        enabled: function() { return sio.usable(o.ds) },
        on: function() { return String(sio.get(o.ds)) === onv },
        act: function(v) { sio.write(o.ds, v ? onv : offv, app(o)) }
    }
}

// o: ds, options (labels), values (what the service uses, same order), icon, label, help, detail, tab
function seg(sio, o) {
    return {
        kind: "seg", tab: o.tab, id: o.id || o.ds, icon: o.icon, label: o.label, help: o.help,
        detail: unavailable(sio, o.ds, o.detail), options: o.options, visible: present(sio, o.ds),
        enabled: function() { return sio.usable(o.ds) },
        index: function() { var i = o.values.indexOf(String(sio.get(o.ds))); return i < 0 ? undefined : i },
        act: function(i) { sio.write(o.ds, o.values[i], app(o)) }
    }
}

// o: ds, min, max, step, unit, icon, label, help, detail, tab. A key the service has not answered shows an en dash, not a zero.
function slider(sio, o) {
    return {
        kind: "slider", tab: o.tab, id: o.id || o.ds, icon: o.icon, label: o.label, help: o.help, unit: o.unit,
        detail: unavailable(sio, o.ds, o.detail), min: o.min, max: o.max, step: o.step || 1, visible: present(sio, o.ds),
        enabled: function() { return sio.usable(o.ds) },
        num: function() { return sio.num(o.ds, o.min) },
        valueText: function() { return sio.has(o.ds) ? String(Math.round(sio.num(o.ds, o.min))) + (o.unit || "") : "\u2013" },
        act: function(v) { sio.write(o.ds, v, app(o)) }
    }
}

// o: ds, options [{title, sub, icon}], values, cols, tab, help. One card is the current setting. `o.strip`: one row of cards, `cols` of them in
// view and the rest scrolled in, so a long list takes the height of one row (the mocks' mode choosers).
function cards(sio, o) {
    return {
        kind: "cards", tab: o.tab, id: o.id || o.ds, help: o.help, options: o.options, cols: o.strip ? 1000 : (o.cols || 4), strip: !!o.strip, inView: o.cols || 4,
        visible: present(sio, o.ds),
        enabled: function() { return sio.usable(o.ds) },
        index: function() { var i = o.values.indexOf(String(sio.get(o.ds))); return i < 0 ? undefined : i },
        act: function(i) { sio.write(o.ds, o.values[i], app(o)) }
    }
}

// A row that shows a text and a chevron and does `go()` on OK. o: icon, label, help, detail, value, tab, enabled
function link(o) {
    return { kind: "value", tab: o.tab, id: o.id, icon: o.icon, label: o.label, help: o.help, detail: o.detail, value: o.value, enabled: o.enabled,
             chevron: o.chevron, act: o.go }
}

// "More in LG settings": opens LG's Settings app and closes ours. `page` is the PageList (its host launches). `o.target` is the
// LG screen to land on, one of the panel names LG's own app takes in its launch parameters (`target`, for example Brightness,
// Color, Clarity, SoundOut, HomeSettings, SoftwareUpdate; the whole list is in tests/qmlhost/stubs/WebOSServices/LgSettings.js),
// so a row names the screen its label promises instead of leaving the owner at LG's Picture page. `o.app` is for the few things
// that are an app of their own (Game Optimizer). The row is its label (LG's own name for the screen) and, when needed, `o.help`;
// it has no second line: a list of what is behind it is what the target screen is for.
function lg(page, o) {
    return { kind: "value", tab: o.tab, id: o.id || "lg", icon: "square-arrow-out-up-right", label: o.label || "More in LG settings",
             help: o.help, value: "", target: o.target || "", app: o.app || "com.palm.app.settings",
             act: function() { page.host.launchApp(o.app || "com.palm.app.settings", o.target ? { target: o.target } : {}); page.host.closeRequested() } }
}

// A row descriptor with some of its fields replaced (a page that needs one more condition on `enabled`).
function over(row, fields) {
    var r = {}
    for (var k in row) r[k] = row[k]
    for (var f in fields) r[f] = fields[f]
    return r
}

function head(text, tab) { return { kind: "head", tab: tab, text: text } }
// What a tab has nothing to list: a glyph tile, `title` and a `text` line under it
function empty(title, text, icon, tab) { return { kind: "empty", tab: tab, icon: icon, title: title, text: text } }
function note(text, tab, visible) { return { kind: "note", tab: tab, text: text, visible: visible } }

// What a service value reads as, from a table; the raw value when the table lacks it.
function name(table, value, fallback) {
    var v = String(value)
    return table[v] !== undefined ? table[v] : (fallback === undefined ? v : fallback)
}
