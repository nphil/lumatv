.pragma library

// Builders for the row descriptors PageList draws. A page names a setting as "category.key" and gets a row
// that reads the settings service (`sio`, passed in: a QML singleton cannot be imported here) and writes
// it back. A row whose key the service never answered is dimmed ("Not available on this TV") rather than
// shown with a made-up value: `enabled` is `sio.has(ds)`. The functions run inside the binding that draws
// the row, so a new answer redraws it.

function unavailable(sio, ds, detail) {
    return function() { return sio.has(ds) ? detail : "Not available on this TV" }
}

// o: ds, icon, label, detail, tab, on ("on"), off ("off")
function toggle(sio, o) {
    var onv = o.on || "on", offv = o.off || "off"
    return {
        kind: "toggle", tab: o.tab, id: o.id || o.ds, icon: o.icon, label: o.label,
        detail: unavailable(sio, o.ds, o.detail),
        enabled: function() { return sio.has(o.ds) },
        on: function() { return String(sio.get(o.ds)) === onv },
        act: function(v) { sio.write(o.ds, v ? onv : offv) }
    }
}

// o: ds, options (labels), values (what the service uses, same order), icon, label, detail, tab, segWidth
function seg(sio, o) {
    return {
        kind: "seg", tab: o.tab, id: o.id || o.ds, icon: o.icon, label: o.label, segWidth: o.segWidth,
        detail: unavailable(sio, o.ds, o.detail), options: o.options,
        enabled: function() { return sio.has(o.ds) },
        index: function() { var i = o.values.indexOf(String(sio.get(o.ds))); return i < 0 ? undefined : i },
        act: function(i) { sio.write(o.ds, o.values[i]) }
    }
}

// o: ds, min, max, step, unit, icon, label, detail, tab
function slider(sio, o) {
    return {
        kind: "slider", tab: o.tab, id: o.id || o.ds, icon: o.icon, label: o.label, unit: o.unit,
        detail: unavailable(sio, o.ds, o.detail), min: o.min, max: o.max, step: o.step || 1,
        enabled: function() { return sio.has(o.ds) },
        num: function() { return sio.num(o.ds, o.min) },
        act: function(v) { sio.write(o.ds, v) }
    }
}

// o: ds, options [{title, sub, icon}], values, cols, tab. One card is the current setting.
function cards(sio, o) {
    return {
        kind: "cards", tab: o.tab, id: o.id || o.ds, options: o.options, cols: o.cols || 4,
        enabled: function() { return sio.has(o.ds) },
        index: function() { var i = o.values.indexOf(String(sio.get(o.ds))); return i < 0 ? undefined : i },
        act: function(i) { sio.write(o.ds, o.values[i]) }
    }
}

// A row that shows a text and a chevron and does `go()` on OK. o: icon, label, detail, value, tab, enabled
function link(o) {
    return { kind: "value", tab: o.tab, id: o.id, icon: o.icon, label: o.label, detail: o.detail, value: o.value, enabled: o.enabled,
             chevron: o.chevron, act: o.go }
}

// "More in LG settings": opens LG's Settings app and closes ours. `page` is the PageList (its host launches).
function lg(page, o) {
    return { kind: "value", tab: o.tab, id: o.id || "lg", icon: "square-arrow-out-up-right", label: o.label || "More in LG settings",
             detail: o.detail, value: "", act: function() { page.host.launchApp("com.palm.app.settings"); page.host.closeRequested() } }
}

function head(text, tab) { return { kind: "head", tab: tab, text: text } }
function note(text, tab, visible) { return { kind: "note", tab: tab, text: text, visible: visible } }

// What a service value reads as, from a table; the raw value when the table lacks it.
function name(table, value, fallback) {
    var v = String(value)
    return table[v] !== undefined ? table[v] : (fallback === undefined ? v : fallback)
}
