.pragma library

// Rows for a plugin's own settings page, from the schema the helper serves for it (`GET /v1/settings/<plugin>`: LumaShow's settings
// schema, format 1; the page the plugin declared, less what the helper keeps for itself). A plugin never draws: it names fields, and
// each device draws them in its own sizes and for its own input. This is the TV's drawing: a switch is a toggle, a number a slider, a
// field whose choices the plugin lists (`{"provider": "rooms"}`) a list of rows with a check, and what the plugin only says
// (`read-only`, a line it published) a row that shows it. A kind this TV has no row for (a time zone, a sheet, a timeline) is left
// out, not drawn wrong, and an address or a secret is not typed here: the owner links Home Assistant from a shell on the TV.
//
// `ctx` is what the page lends, so that this file holds no state (a library cannot import the singletons):
//   get(key)         the effective value of a field: text, number, boolean, or an array of ids for an ordered set
//   write(key, v)    asks the helper to change it (the helper checks it, the plugin applies it)
//   fact(capability) the line the plugin published for a read-only field (`plugin.<id>.<key>`), or undefined
//   choices(name)    what the plugin listed for a provider: {state, rows: [{id, label, detail?}], more}, or undefined while asked
// The functions of a descriptor run inside the bindings that draw the row, so what they read redraws it.

var NOT_READY = {
    loading: "Loading\u2026",
    unconfigured: "Not linked",
    offline: "Offline",
    failed: "Not loaded"
}

// A row says its name and its value; what a field's author wrote to explain it is its `help`, behind the row's info mark. A title that
// has some is a row that only shows, which the D-pad reaches for its mark (a plain title cannot take the focus).
function head(text, tab, help) {
    return help ? { kind: "value", tab: tab, id: "help:" + text, info: true, chevron: false, label: text, help: help }
                : { kind: "head", tab: tab, text: text }
}
function note(text, tab, visible) { return { kind: "note", tab: tab, text: text, visible: visible } }

function listOf(ctx, key) {
    var v = ctx.get(key)
    return v instanceof Array ? v : []
}

// One row of a picker. `many`: an ordered set (the order picked is shown as a number); else the one chosen.
function pick(ctx, field, tab, row, many, max) {
    var key = field.key
    return {
        kind: "value", tab: tab, id: key + ":" + row.id, label: row.label, chevron: false,
        selected: function() { return many ? listOf(ctx, key).indexOf(row.id) >= 0 : ctx.get(key) === row.id },
        value: function() { var i = many ? listOf(ctx, key).indexOf(row.id) : -1; return i >= 0 ? String(i + 1) : "" },
        act: function() {
            if (!many) { ctx.write(key, row.id); return }
            var now = listOf(ctx, key), at = now.indexOf(row.id)
            if (at >= 0) now = now.slice(0, at).concat(now.slice(at + 1))
            else if (now.length < max) now = now.concat([row.id])
            ctx.write(key, now)
        }
    }
}

// The rows of a field whose choices come from a provider, or are listed in the schema.
function picker(ctx, field, tab) {
    var kind = field.kind, many = kind.type === "ordered-set", max = kind.max_items || 64
    var out = [head(field.label, tab, field.help)]
    var options = kind.options || {}
    var got = options.inline ? { state: "ready", rows: options.inline.map(function(o) { return { id: o.id, label: o.label, detail: o.detail } }), more: false }
                             : ctx.choices(options.provider)
    if (!got) got = { state: "loading", rows: [] }
    if (got.state !== "ready") { out.push(note(NOT_READY[got.state] || NOT_READY.failed, tab)); return out }
    if (!got.rows.length) out.push(note("Nothing to pick", tab))
    for (var i = 0; i < got.rows.length; i++) out.push(pick(ctx, field, tab, got.rows[i], many, max))
    if (got.more) out.push(note("First " + got.rows.length + " shown", tab))
    return out
}

// The rows of one field, on tab `tab`.
function fieldRows(ctx, field, tab) {
    var kind = field.kind
    switch (kind.type) {
    case "read-only":
        var cap = kind.value && kind.value.capability
        var when = field.visible_when
        return [{ kind: "value", tab: tab, id: field.key, info: true, chevron: false, label: field.label,
                  value: function() { return cap ? (ctx.fact(cap) || "") : "" },
                  visible: when && when.type === "capability-present" ? function() { return ctx.fact(when.capability) !== undefined } : undefined }]
    case "bool":
        return [{ kind: "toggle", tab: tab, id: field.key, label: field.label, help: field.help || "",
                  on: function() { return ctx.get(field.key) === true }, act: function(v) { ctx.write(field.key, v) } }]
    case "integer":
        return [{ kind: "slider", tab: tab, id: field.key, label: field.label, help: field.help || "", unit: kind.unit ? " " + kind.unit : "",
                  min: kind.min, max: kind.max, step: kind.step || 1,
                  num: function() { var n = Number(ctx.get(field.key)); return isNaN(n) ? kind.min : n },
                  act: function(v) { ctx.write(field.key, v) } }]
    case "choice":
    case "ordered-set":
        return picker(ctx, field, tab)
    case "text":
        // Shown, not typed: an address is the owner's to give from a shell.
        return kind.secret ? [] : [{ kind: "value", tab: tab, id: field.key, info: true, chevron: false, label: field.label,
                                     value: function() { var t = ctx.get(field.key); return t === undefined || t === "" ? "Not set" : String(t) } }]
    default:
        return []
    }
}

// The tab names of a schema's sections: the short word for the sections the page names, the section's own title otherwise.
function tabs(schema, names) {
    var s = (schema && schema.sections) || []
    return s.map(function(sec) { return (names && names[sec.id]) || sec.title })
}

// Every section of the schema as the rows of its tab.
function rows(ctx, schema) {
    var out = [], s = (schema && schema.sections) || []
    for (var i = 0; i < s.length; i++)
        for (var j = 0; j < s[i].fields.length; j++) out = out.concat(fieldRows(ctx, s[i].fields[j], i))
    return out
}

// The providers a schema names (what to ask the helper for), in the order they are met.
function providers(schema) {
    var out = [], s = (schema && schema.sections) || []
    for (var i = 0; i < s.length; i++) for (var j = 0; j < s[i].fields.length; j++) {
        var k = s[i].fields[j].kind, o = k.options
        if ((k.type === "choice" || k.type === "ordered-set") && o && o.provider && out.indexOf(o.provider) < 0) out.push(o.provider)
    }
    return out
}
