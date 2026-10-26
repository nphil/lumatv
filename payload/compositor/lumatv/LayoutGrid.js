.pragma library

// The Home grid, as one model: where each widget is drawn (LumaHome), what Settings' Home widgets page changes and what the
// layout editor moves. It is the layout simulator's model (the 1920x1080 stage, the safe area, the status bar, `cols` columns of
// `gutter` apart and rows that are a count or a list of pixel heights; widget entries {id, type, col, row, span, rows}) with the
// simulator's rules: a widget never overlaps another, a change that would overlap is refused, a new widget takes the first free
// place at the first size of a ladder that fits, and the TV's pixel rows become `UNIFORM_ROWS` equal rows when a layout is edited.
// Plain ES5 with no QML or DOM in it, so Home's QML engine and the tests run this one file.
//
// A layout is the merged object LumaHome holds (the shipped layout.json under the owner's). Nothing here changes one in place:
// every change returns { ok, why, layout, ... } with a new layout, so a caller can keep the old one to undo it.

var STAGE_W = 1920
var STAGE_H = 1080
var UNIFORM_ROWS = 16               // the simulator's grid: 37 px rows 20 px apart on the TV's 892 px of widget area

// What a person can put on Home: the name and the Lucide glyph the add list and Settings show (nothing else: a row is its name). `min` is
// the smallest card, in columns and rows, whose content is still worth drawing (a card can be made smaller by hand-editing the file, and
// still draws what fits); `want` is the ladder of sizes a new card tries, largest first; `id` is the id a first card of the type gets.
var TYPES = {
    clock:         { title: "Clock",    icon: "clock",       min: [2, 3], want: [[4, 4], [3, 4], [3, 3], [2, 3]], id: "clock" },
    homeassistant: { title: "At home",  icon: "house-wifi",  min: [2, 3], want: [[4, 5], [4, 4], [3, 4], [3, 3], [2, 3]], id: "home", defaults: { title: "At home" } },
    weather:       { title: "Weather",  icon: "sun",         min: [2, 3], want: [[4, 6], [3, 6], [3, 4], [2, 4], [2, 3]], id: "weather" },
    news:          { title: "News",     icon: "file-text",   min: [3, 4], want: [[5, 7], [4, 6], [4, 5], [3, 5], [3, 4]], id: "news" },
    dock:          { title: "Apps",     icon: "layout-grid", min: [3, 3], want: [[12, 4], [6, 4], [6, 3], [4, 3], [3, 3]], id: "dock", fixed: true }
}
// The types the add list offers, in its order. The dock is Home's way into every app: it is moved and resized, never added or removed.
var ADDABLE = ["clock", "homeassistant", "weather", "news"]

// The widths Settings offers, in columns of the 12.
var SIZES = [{ id: "small", label: "Small", span: 4 }, { id: "half", label: "Half", span: 6 }, { id: "wide", label: "Wide", span: 8 }]

function clamp(v, a, b) { return Math.max(a, Math.min(b, v)) }
function num(v, d) { return typeof v === "number" && isFinite(v) ? v : d }
function clone(v) { return JSON.parse(JSON.stringify(v)) }
// Deep equality that does not care in which order an object's keys were written.
function same(a, b) {
    if (a === b) return true
    if (a === null || b === null || typeof a !== "object" || typeof b !== "object" || Array.isArray(a) !== Array.isArray(b)) return false
    var ka = Object.keys(a), kb = Object.keys(b)
    if (ka.length !== kb.length) return false
    for (var i = 0; i < ka.length; i++) if (!(ka[i] in b) || !same(a[ka[i]], b[ka[i]])) return false
    return true
}
function extend(a, b) { var o = {}, k; for (k in a) o[k] = a[k]; for (k in b) o[k] = b[k]; return o }

// ---- geometry (the simulator's area, cell, rowCount, rectOf; LumaHome's cellRect is this)
function area(L) {
    var y = L.safe.y + L.statusBar.height + L.statusBar.gap
    return { x: L.safe.x, y: y, w: STAGE_W - 2 * L.safe.x, h: STAGE_H - L.safe.y - y }
}
function pixelRows(L) { return Array.isArray(L.grid.rows) }
function rowCount(L) { var r = L.grid.rows; return Array.isArray(r) ? r.length : Math.max(1, r || 8) }
function rowHeights(L) {
    if (pixelRows(L)) return L.grid.rows
    var a = area(L), n = rowCount(L), h = (a.h - (n - 1) * L.grid.gutter) / n, out = []
    for (var i = 0; i < n; i++) out.push(h)
    return out
}
function cellSize(L) {
    var a = area(L), g = L.grid, n = rowCount(L)
    return { w: (a.w - (g.cols - 1) * g.gutter) / g.cols, h: (a.h - (n - 1) * g.gutter) / n }
}
function rectOf(L, e) {
    var a = area(L), g = L.grid, rows = rowHeights(L), cw = (a.w - (g.cols - 1) * g.gutter) / g.cols
    var y = a.y, h = 0, r, k
    for (r = 0; r < e.row && r < rows.length; r++) y += rows[r] + g.gutter
    for (k = e.row; k < e.row + e.rows && k < rows.length; k++) h += rows[k] + (k > e.row ? g.gutter : 0)
    return { x: Math.round(a.x + e.col * (cw + g.gutter)), y: Math.round(y), w: Math.round(e.span * cw + (e.span - 1) * g.gutter), h: Math.round(h) }
}

// ---- the entries
function entries(L) { return L.widgets || [] }
function indexOf(L, id) {
    var w = entries(L)
    for (var i = 0; i < w.length; i++) if (w[i].id === id) return i
    return -1
}
function find(L, id) { var i = indexOf(L, id); return i < 0 ? null : entries(L)[i] }
function isDock(e) { return !!e && e.type === "dock" }
// The id of the dock ("" if the layout has none), and whether a card of this type is there but hidden (adding one brings it back).
function dockId(L) {
    var w = entries(L)
    for (var i = 0; i < w.length; i++) if (isDock(w[i])) return w[i].id
    return ""
}
function hiddenOf(L, type) {
    var w = entries(L)
    for (var i = 0; i < w.length; i++) if (w[i].type === type && w[i].hidden) return true
    return false
}
function typeOf(e) { return TYPES[e.type] || null }
// The name a card goes by: its own title (the At-home card's, the dock's), else its type's.
function titleOf(e) { var t = typeOf(e); return e.title || (t ? t.title : e.type) }
function visible(e) { return !e.hidden }
function minOf(e) { var t = typeOf(e); return t ? t.min : [1, 1] }
function withEntries(L, list) { var o = extend(L, {}); o.widgets = list; return o }
function replaceAt(L, i, e) { var w = entries(L).slice(); w[i] = e; return withEntries(L, w) }

// An entry the grid can hold: numbers where numbers belong, inside the grid. A layout edited by hand (or by the simulator with other
// numbers of columns) is cut to the grid, not refused, so the editor can always open on what Home is drawing.
function tidy(L, e) {
    var cols = L.grid.cols, n = rowCount(L), o = extend(e, {})
    o.span = clamp(Math.round(num(e.span, 1)), 1, cols)
    o.rows = clamp(Math.round(num(e.rows, 1)), 1, n)
    o.col = clamp(Math.round(num(e.col, 0)), 0, cols - o.span)
    o.row = clamp(Math.round(num(e.row, 0)), 0, n - o.rows)
    return o
}
function fits(L, e) {
    return e.span >= 1 && e.rows >= 1 && e.col >= 0 && e.row >= 0 && e.col + e.span <= L.grid.cols && e.row + e.rows <= rowCount(L)
}
// The simulator's rule: another card (a hidden one takes no room) whose cells meet `e`'s. Cards are told apart by id.
function blocker(L, e) {
    var w = entries(L)
    for (var i = 0; i < w.length; i++) {
        var o = w[i]
        if (o.id === e.id || o.hidden) continue
        if (e.col < o.col + o.span && o.col < e.col + e.span && e.row < o.row + o.rows && o.row < e.row + e.rows) return o
    }
    return null
}
function overlaps(L, e) { return blocker(L, e) !== null }

// A layout with every entry made to fit. Entries that are not objects, or have no type, are dropped (they could not be drawn anyway),
// ids are made unique, and each entry is cut to the grid.
function tidyLayout(L) {
    var seen = {}, out = [], w = entries(L)
    for (var i = 0; i < w.length; i++) {
        var e = w[i]
        if (!e || typeof e !== "object" || typeof e.type !== "string") continue
        var o = tidy(L, e), id = typeof o.id === "string" && o.id !== "" ? o.id : o.type
        var base = id, n = 2
        while (seen[id]) id = base + "-" + (n++)
        seen[id] = true
        o.id = id
        out.push(o)
    }
    return withEntries(L, out)
}

// The TV's pixel rows to `UNIFORM_ROWS` equal ones, each card keeping the rows nearest to the pixels it covered (the simulator's
// toUniform). A layout whose rows are already a count comes back as it was.
function toUniform(L) {
    if (!pixelRows(L)) return L
    var out = clone(L), pix = L.grid.rows, n = UNIFORM_ROWS, g = L.grid.gutter
    var c = (area(L).h - (n - 1) * g) / n
    out.widgets = entries(L).map(function (e) {
        var y0 = 0, h = 0, r, k, o = extend(e, {})
        for (r = 0; r < e.row; r++) y0 += (pix[r] === undefined ? 0 : pix[r]) + g
        for (k = e.row; k < e.row + e.rows; k++) h += (pix[k] === undefined ? 0 : pix[k]) + (k > e.row ? g : 0)
        o.row = Math.round(y0 / (c + g))
        o.rows = Math.max(1, Math.round((h + g) / (c + g)))
        return o
    })
    out.grid.rows = n
    return tidyLayout(out)
}

// Whether a card lies above (dy below zero) or below one with columns in common: Up from one with nothing over it is the way out of the grid.
function hasBeside(L, id, dy) {
    var e = find(L, id), w = entries(L)
    if (!e) return false
    for (var i = 0; i < w.length; i++) {
        var o = w[i]
        if (o.id === id || o.hidden || o.col >= e.col + e.span || e.col >= o.col + o.span) continue
        if (dy < 0 ? o.row + o.rows <= e.row : o.row >= e.row + e.rows) return true
    }
    return false
}

// ---- changing one card
// A move or a resize: `next` holds the new numbers. { ok: true, layout, entry } or { ok: false, why: "edge" | "overlap" | "small",
// blocker } (the card in the way for "overlap"). "edge" is a press that goes nowhere: the card is at the edge of the grid.
function change(L, id, next) {
    var i = indexOf(L, id)
    if (i < 0) return { ok: false, why: "missing" }
    var e = entries(L)[i], o = extend(e, next)
    if (o.col === e.col && o.row === e.row && o.span === e.span && o.rows === e.rows) return { ok: false, why: "edge" }
    var m = minOf(e)
    if ((o.span < e.span && o.span < m[0]) || (o.rows < e.rows && o.rows < m[1])) return { ok: false, why: "small" }
    if (!fits(L, o)) return { ok: false, why: "edge" }
    var b = blocker(L, o)
    if (b) return { ok: false, why: "overlap", blocker: b }
    return { ok: true, layout: replaceAt(L, i, o), entry: o }
}
// One cell in a direction, never off the grid.
function move(L, id, dc, dr) {
    var e = find(L, id)
    if (!e) return { ok: false, why: "missing" }
    return change(L, id, { col: clamp(e.col + dc, 0, L.grid.cols - e.span), row: clamp(e.row + dr, 0, rowCount(L) - e.rows) })
}
// One cell at the bottom right corner: the simulator's W and H steppers.
function resize(L, id, dspan, drows) {
    var e = find(L, id)
    if (!e) return { ok: false, why: "missing" }
    return change(L, id, { span: clamp(e.span + dspan, 1, L.grid.cols - e.col), rows: clamp(e.rows + drows, 1, rowCount(L) - e.row) })
}

// ---- finding room
// The first free place (top to bottom, then left to right, as the simulator scans) for a card of one of `sizes` ([[span, rows]], tried
// in order). { ok, entry } with the card placed, or { ok: false, why: "full" }.
function firstFree(L, e, sizes) {
    var n = rowCount(L), cols = L.grid.cols
    for (var s = 0; s < sizes.length; s++) {
        var span = Math.min(sizes[s][0], cols), rows = Math.min(sizes[s][1], n)
        for (var r = 0; r + rows <= n; r++)
            for (var k = 0; k + span <= cols; k++) {
                var o = extend(e, { col: k, row: r, span: span, rows: rows })
                if (!overlaps(L, o)) return { ok: true, entry: o }
            }
    }
    return { ok: false, why: "full" }
}
// The sizes a card of this size falls back to when its own does not fit (the simulator's placeCopy): the same, then halves.
function fallbacks(e) {
    var s = [[e.span, e.rows]]
    if (e.span > 2) s.push([Math.ceil(e.span / 2), e.rows])
    if (e.rows > 2) s.push([e.span, Math.ceil(e.rows / 2)], [Math.ceil(e.span / 2), Math.ceil(e.rows / 2)])
    return s
}
function atLeast(sizes, min) { return sizes.filter(function (s) { return s[0] >= min[0] && s[1] >= min[1] }) }

// An id for a new card of a type: the type's own (clock, home, weather, news) while it is free, then clock-2, clock-3.
function newId(L, type) {
    var base = (TYPES[type] && TYPES[type].id) || type, id = base, n = 2
    while (indexOf(L, id) >= 0) id = base + "-" + (n++)
    return id
}

// Adds a card of `type` in the first free place of its size ladder. A card of that type that is hidden comes back instead, as it was
// (so "hide" and "add" are one thing to the person). { ok, layout, id } or { ok: false, why: "full" | "unknown" | "fixed" }.
function add(L, type) {
    var t = TYPES[type]
    if (!t) return { ok: false, why: "unknown" }
    if (t.fixed) return { ok: false, why: "fixed" }
    var w = entries(L)
    for (var i = 0; i < w.length; i++) if (w[i].type === type && w[i].hidden) return show(L, w[i].id)
    var e = extend({ id: newId(L, type), type: type, col: 0, row: 0, span: 1, rows: 1 }, t.defaults || {})
    var at = firstFree(L, e, atLeast(t.want, t.min))
    if (!at.ok) return at
    return { ok: true, layout: withEntries(L, w.concat([at.entry])), id: at.entry.id }
}
function remove(L, id) {
    var e = find(L, id)
    if (!e) return { ok: false, why: "missing" }
    if (isDock(e)) return { ok: false, why: "fixed" }
    return { ok: true, layout: withEntries(L, entries(L).filter(function (o) { return o.id !== id })) }
}
// Hiding keeps the card where it was and the room it took is free; showing puts it back there, or, if something has taken the place,
// in the first free one at its size, then smaller.
function hide(L, id) {
    var i = indexOf(L, id)
    if (i < 0) return { ok: false, why: "missing" }
    var e = entries(L)[i]
    if (isDock(e)) return { ok: false, why: "fixed" }
    return { ok: true, layout: replaceAt(L, i, extend(e, { hidden: true })) }
}
function show(L, id) {
    var i = indexOf(L, id)
    if (i < 0) return { ok: false, why: "missing" }
    var e = entries(L)[i], o = extend(e, {})
    delete o.hidden
    o = tidy(L, o)
    if (!overlaps(L, o)) return { ok: true, layout: replaceAt(L, i, o), id: id }
    var at = firstFree(L, o, atLeast(fallbacks(o), minOf(o)))
    if (!at.ok) return at
    return { ok: true, layout: replaceAt(L, i, at.entry), id: id }
}

// ---- sizes and order (Settings' Home widgets page)
// Which of the three widths a card has: "small", "half", "wide", or "custom".
function sizeOf(e) {
    for (var i = 0; i < SIZES.length; i++) if (SIZES[i].span === e.span) return SIZES[i].id
    return "custom"
}
// The card made `span` columns wide. It grows to the right; one that is at the right edge of the grid grows to the left, so that a
// card on the right stays there. { ok, layout } or { ok: false, why: "overlap" | "edge" | "small", blocker }.
function setSpan(L, id, span) {
    var e = find(L, id)
    if (!e) return { ok: false, why: "missing" }
    var cols = L.grid.cols
    if (span > cols) return { ok: false, why: "edge" }
    var col = e.col + e.span >= cols ? cols - span : Math.min(e.col, cols - span)
    if (span === e.span && col === e.col) return { ok: false, why: "edge" }
    return change(L, id, { col: Math.max(0, col), span: span })
}
// The ids of the cards on Home in reading order: top to bottom, then left to right. The dock and hidden cards are not in it.
function order(L) {
    return entries(L).filter(function (e) { return !isDock(e) && !e.hidden }).sort(function (a, b) { return a.row - b.row || a.col - b.col }).map(function (e) { return e.id })
}
// Two cards trade places: each takes the other's column, row, width and height. Nothing can overlap that did not, but a card must not
// be made smaller than its type's smallest.
function swap(L, aId, bId) {
    var ia = indexOf(L, aId), ib = indexOf(L, bId)
    if (ia < 0 || ib < 0 || ia === ib) return { ok: false, why: "missing" }
    var a = entries(L)[ia], b = entries(L)[ib]
    var na = extend(a, { col: b.col, row: b.row, span: b.span, rows: b.rows }), nb = extend(b, { col: a.col, row: a.row, span: a.span, rows: a.rows })
    var ma = minOf(a), mb = minOf(b)
    if (na.span < ma[0] || na.rows < ma[1]) return { ok: false, why: "small", blocker: a }
    if (nb.span < mb[0] || nb.rows < mb[1]) return { ok: false, why: "small", blocker: b }
    return { ok: true, layout: replaceAt(replaceAt(L, ia, na), ib, nb) }
}
// A card one place earlier (-1) or later (+1) in reading order.
function nudge(L, id, d) {
    var o = order(L), i = o.indexOf(id)
    if (i < 0 || i + d < 0 || i + d >= o.length) return { ok: false, why: "edge" }
    return swap(L, id, o[i + d])
}

// ---- the owner's file
// What layout.json holds for a layout: only what differs from the shipped one (objects key by key, lists and numbers whole), which is
// what `theme.py minimize` does to the file at every apply, so a key never changed follows an update. `{}` for the shipped layout.
var RETIRED = ["version"]
function minimize(user, shipped, top) {
    var out = {}, any = false
    for (var k in user) {
        if (top && RETIRED.indexOf(k) >= 0) continue
        var v = user[k], s = shipped ? shipped[k] : undefined
        if (v !== null && typeof v === "object" && !Array.isArray(v) && s !== null && typeof s === "object" && !Array.isArray(s)) {
            var sub = minimize(v, s, false)
            if (sub !== undefined) { out[k] = sub; any = true }
        } else if (!(k in (shipped || {})) || !same(s, v)) { out[k] = v; any = true }
    }
    return any ? out : (top ? {} : undefined)
}
function forFile(L, shipped) {
    var o = extend(L, {})
    o.widgets = entries(L).map(function (e) { var c = extend(e, {}); delete c._dock; return c })
    return minimize(o, shipped, true)
}
// A card's place in two words for a list: "left, top", "right, middle", "full width, bottom".
function placeWords(L, e) {
    var third = function (v, n) { return v < n / 3 ? 0 : v < 2 * n / 3 ? 1 : 2 }
    var n = rowCount(L)
    var h = e.span >= L.grid.cols ? "full width" : ["left", "centre", "right"][third(e.col + e.span / 2, L.grid.cols)]
    var v = e.rows >= n ? "full height" : ["top", "middle", "bottom"][third(e.row + e.rows / 2, n)]
    return h + ", " + v
}

// ---- the pointer
// Where a card lands when it is dragged `dx`, `dy` pixels (the simulator's snap while dragging), and the size it takes when its corner
// is dragged by `dw`, `dh`: whole cells, kept on the grid. They only say where; `change` decides whether the card may go there.
function snapMove(L, id, dx, dy) {
    var e = find(L, id)
    if (!e) return null
    var c = cellSize(L), g = L.grid
    return { col: clamp(Math.round(e.col + dx / (c.w + g.gutter)), 0, g.cols - e.span), row: clamp(Math.round(e.row + dy / (c.h + g.gutter)), 0, rowCount(L) - e.rows) }
}
function snapResize(L, id, dw, dh) {
    var e = find(L, id)
    if (!e) return null
    var c = cellSize(L), g = L.grid, r = rectOf(L, e), m = minOf(e)
    return { span: clamp(Math.round((r.w + dw + g.gutter) / (c.w + g.gutter)), Math.min(m[0], e.span), g.cols - e.col),
             rows: clamp(Math.round((r.h + dh + g.gutter) / (c.h + g.gutter)), Math.min(m[1], e.rows), rowCount(L) - e.row) }
}
