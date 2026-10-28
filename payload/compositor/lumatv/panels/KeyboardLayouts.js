.pragma library

// The on-screen keyboard's model (panels/Keyboard.qml draws it, the tests run this file): which layout a text field gets from its content
// type, the keys of each layout on a grid, how the D-pad moves over it, what a key does when it is pressed and when the remote's repeats are
// let through. Plain ES5 with no QML in it.
//
// A field says what it is with three numbers of the text protocol (compositor/protocol/webos/text.xml): a purpose (digits, email, url,
// password, ...), hint bits (hidden text, uppercase, ...) and an enter key type (search, done, ...). `layoutFor` turns the purpose into a
// layout; the hint bits and the enter type change the keys of that layout, not which one it is.
//
// A layout is a grid of `cols` unit columns and `rows` rows; a key is {id, kind, label, col, row, span, ...}. Kinds:
//   char   types `text` (`shifted` with Shift)        text   types `text` as it is (".com")        space
//   shift  cycles off, one capital, caps lock           mode   shows another layout (`to`)           backspace
//   enter  the field's action (its label is the enter type's)      clear  deletes the field's text   voice   LG's voice
//   cursor moves the field's caret (`dir`)                         hide   puts the keyboard away (where there is no voice key: the grid has no hole)
// `fn` marks the keys drawn recessed (everything but the characters), `accent` the one primary key.

var PURPOSE = { normal: 0, alpha: 1, digits: 2, number: 3, phone: 4, url: 5, email: 6, name: 7, password: 8, date: 9, time: 10, datetime: 11, terminal: 12 }
var HINT = { lowercase: 0x8, uppercase: 0x10, titlecase: 0x20, hiddenText: 0x40, sensitiveData: 0x80 }
var ENTER = { def: 0, ret: 1, done: 2, go: 3, send: 4, search: 5, next: 6, previous: 7 }

var FULL_COLS = 12
var FULL_ROWS = 5
var PAD_COLS = 4
var PAD_ROWS = 4

// The most keys, and the most keys with a picture, any layout has: the QML makes this many of each once and never again (a layout change
// rebinds them), so no key is created while the remote waits.
var MAX_KEYS = 51
var MAX_ICONS = 8

// ---- what a field gets

// The layout a field's purpose asks for: a number pad for digits, numbers, phone numbers and dates and times, the e-mail and URL
// letters for those, the plain letters for everything else (a password too: the keys are the same, only nothing is ever shown of what is typed).
function layoutFor(purpose, hint) {
    switch (purpose | 0) {
    case PURPOSE.digits: return "pad-digits"
    case PURPOSE.number: return "pad-number"
    case PURPOSE.phone: return "pad-phone"
    case PURPOSE.date:
    case PURPOSE.time:
    case PURPOSE.datetime: return "pad-date"
    case PURPOSE.email: return "email"
    case PURPOSE.url: return "url"
    }
    return "letters"
}

function isPad(id) { return id.indexOf("pad-") === 0 }

// A field whose characters are hidden (a password, a PIN). The keyboard never shows what was typed in any field; this says the field is one
// whose words must not rest anywhere else either.
function secret(purpose, hint) {
    return (purpose | 0) === PURPOSE.password || ((hint | 0) & (HINT.hiddenText | HINT.sensitiveData)) !== 0
}

// Shift when the field opens: caps lock for a field that asks for uppercase, one capital for a name or a title.
function initialShift(purpose, hint) {
    var h = hint | 0
    if (h & HINT.uppercase) return 2
    if ((h & HINT.lowercase) !== 0) return 0
    if ((h & HINT.titlecase) !== 0 || (purpose | 0) === PURPOSE.name) return 1
    return 0
}

// The enter key: its picture or words, and whether the keyboard goes away after it (a field that is finished: Search, Go, Send, Done).
function enterLook(type) {
    switch (type | 0) {
    case ENTER.done: return { icon: "", text: "Done", hides: true }
    case ENTER.go: return { icon: "", text: "Go", hides: true }
    case ENTER.send: return { icon: "", text: "Send", hides: true }
    case ENTER.search: return { icon: "", text: "Search", hides: true }
    case ENTER.next: return { icon: "", text: "Next", hides: false }
    case ENTER.previous: return { icon: "", text: "Prev", hides: false }
    }
    return { icon: "corner-down-left", text: "", hides: false }
}

// ---- the layouts

function ch(text, letter) {
    var up = text.toUpperCase()
    return { kind: "char", id: text, label: text, text: text, shifted: up, letter: letter === true && up !== text }
}
function chars(list, letter) {
    var out = []
    for (var i = 0; i < list.length; i++) out.push(ch(list[i], letter))
    return out
}
function copy(k) {
    var c = {}
    for (var p in k) c[p] = k[p]
    return c
}
function fn(kind, id, label, extra) {
    var k = { kind: kind, id: id, label: label, text: "", shifted: "", fn: true }
    for (var p in extra) k[p] = extra[p]
    return k
}

var DIGITS = "1234567890".split("")
var Q_ROW = "qwertyuiop".split("")
var A_ROW = "asdfghjkl".split("")
var Z_ROW = "zxcvbnm".split("")

function dotcom() { return { kind: "text", id: "dotcom", label: ".com", text: ".com", shifted: ".com", fn: true } }

// The keys of the three letter layouts differ in a few places; the grid is the same, so a hand that knows one knows the others.
var SETS = {
    letters: { letters: true, l1: "shift", l2: ch("'"), r1: Q_ROW, r2: A_ROW.concat(["?"]), l3: ch("@"), r3: Z_ROW.concat([",", ".", "-"]) },
    email: { letters: true, l1: "shift", l2: ch("@"), r1: Q_ROW, r2: A_ROW.concat(["_"]), l3: dotcom(), r3: Z_ROW.concat([".", "-", "+"]) },
    url: { letters: true, l1: "shift", l2: ch("/"), r1: Q_ROW, r2: A_ROW.concat([":"]), l3: dotcom(), r3: Z_ROW.concat([".", "-", "_"]) },
    symbols: { letters: false, l1: ch("`"), l2: ch("~"), r1: "!@#$%^&*()".split(""), r2: "-_=+[]{}|\\".split(""), l3: ch("\u20ac"), r3: ";:'\",.<>/?".split("") }
}

function full(id, o) {
    var set = SETS[id]
    var keys = []
    function add(row, col, span, k) { var c = copy(k); c.row = row; c.col = col; c.span = span; keys.push(c) }
    function run(row, list) { for (var i = 0; i < list.length; i++) add(row, 1 + i, 1, ch(list[i], set.letters)) }
    var enter = enterLook(o.enter)
    // the left column: the way to the other characters, Shift (or the first symbol), and two keys of the layout
    add(0, 0, 1, set.letters ? fn("mode", "mode", "?123", { to: "symbols" }) : fn("mode", "mode", "ABC", { to: "letters" }))
    run(0, DIGITS)
    add(0, 11, 1, fn("backspace", "backspace", "", { icon: "delete", repeat: true }))
    add(1, 0, 1, set.letters ? fn("shift", "shift", "", { icon: "arrow-big-up" }) : set.l1)
    run(1, set.r1)
    add(1, 11, 1, fn("enter", "enter", enter.text, { icon: enter.icon, accent: true, hides: enter.hides }))
    add(2, 0, 1, set.l2)
    run(2, set.r2)
    add(2, 11, 1, o.voice ? fn("voice", "voice", "", { icon: "mic" }) : fn("hide", "hide", "", { icon: "chevron-down" }))
    add(3, 0, 1, set.l3)
    run(3, set.r3)
    add(3, 11, 1, fn("clear", "clear", "Clear", { needsText: true }))
    add(4, 0, 1, fn("cursor", "left", "", { icon: "chevron-left", dir: "left", repeat: true }))
    add(4, 1, 10, fn("space", "space", "", { text: " " }))
    add(4, 11, 1, fn("cursor", "right", "", { icon: "chevron-right", dir: "right", repeat: true }))
    return { id: id, pad: false, cols: FULL_COLS, rows: FULL_ROWS, shiftable: set.letters, keys: keys, homeRow: 2, homeCol: 5 }
}

var CORNERS = {
    "pad-digits": [null, null],
    "pad-number": [".", "-"],
    "pad-phone": ["+", "-"],
    "pad-date": ["/", ":"]
}

function pad(id, o) {
    var keys = []
    function add(row, col, k) { var c = copy(k); c.row = row; c.col = col; c.span = 1; keys.push(c) }
    for (var i = 0; i < 9; i++) add(Math.floor(i / 3), i % 3, ch(String(i + 1)))
    var corners = CORNERS[id]
    if (corners[0] !== null) add(3, 0, ch(corners[0]))
    add(3, 1, ch("0"))
    if (corners[1] !== null) add(3, 2, ch(corners[1]))
    var enter = enterLook(o.enter)
    add(0, 3, fn("backspace", "backspace", "", { icon: "delete", repeat: true }))
    add(1, 3, fn("enter", "enter", enter.text, { icon: enter.icon, accent: true, hides: enter.hides }))
    add(2, 3, fn("clear", "clear", "Clear", { needsText: true }))
    return { id: id, pad: true, cols: PAD_COLS, rows: PAD_ROWS, shiftable: false, keys: keys, homeRow: 1, homeCol: 1 }
}

// `o`: {enter: the field's enter key type, voice: whether LG's voice can be asked}.
function build(id, o) {
    var opt = o || {}
    return isPad(id) ? pad(id, opt) : full(id, opt)
}

// The grid in pixels: unit columns `unit` wide and `keyH` high, `gap` apart. Returns a rectangle per key (the layout's order) and the size.
function rects(layout, unit, keyH, gap) {
    var out = []
    for (var i = 0; i < layout.keys.length; i++) {
        var k = layout.keys[i]
        out.push({ x: k.col * (unit + gap), y: k.row * (keyH + gap), w: k.span * unit + (k.span - 1) * gap, h: keyH })
    }
    return { rects: out, width: layout.cols * unit + (layout.cols - 1) * gap, height: layout.rows * keyH + (layout.rows - 1) * gap }
}

// ---- the D-pad

function at(layout, row, col) {
    for (var i = 0; i < layout.keys.length; i++) {
        var k = layout.keys[i]
        if (k.row === row && col >= k.col && col < k.col + k.span) return i
    }
    return -1
}

// The key the focus starts on when a field opens: the middle of the letters, the 5 of a pad.
function home(layout) {
    var i = at(layout, layout.homeRow, layout.homeCol)
    return i >= 0 ? i : 0
}

function centre(k) { return k.col + k.span / 2 }

function keysOfRow(layout, row) {
    var list = []
    for (var i = 0; i < layout.keys.length; i++) if (layout.keys[i].row === row) list.push(i)
    list.sort(function(a, b) { return layout.keys[a].col - layout.keys[b].col })
    return list
}

// One step of the D-pad from key `from`: {index, want}. Left and Right walk the row and wrap to its other end; Up and Down go to the key
// above or below at the column the walk started from (`want`, kept while only moving vertically, so a wide key such as the space bar hands
// the column back), wrapping top to bottom, skipping rows with no key and landing on the nearest key where the row has a hole at that column.
function move(layout, from, dir, want) {
    var k = layout.keys[from]
    if (!k) return { index: home(layout), want: centre(layout.keys[home(layout)]) }
    if (dir === "left" || dir === "right") {
        var row = keysOfRow(layout, k.row)
        var p = row.indexOf(from)
        var n = row[(p + (dir === "right" ? 1 : row.length - 1)) % row.length]
        return { index: n, want: centre(layout.keys[n]) }
    }
    var step = dir === "down" ? 1 : layout.rows - 1
    var target = want === undefined ? centre(k) : want
    for (var i = 1; i <= layout.rows; i++) {
        var r = (k.row + step * i) % layout.rows
        var list = keysOfRow(layout, r)
        if (!list.length) continue
        var best = -1, dist = 1e9
        for (var j = 0; j < list.length; j++) {
            var c = layout.keys[list[j]]
            var d = (target >= c.col && target < c.col + c.span) ? 0 : Math.min(Math.abs(target - c.col), Math.abs(target - (c.col + c.span)))
            if (d < dist) { dist = d; best = list[j] }
        }
        return { index: best, want: target }
    }
    return { index: from, want: target }
}

// The key under a point of the grid (the pointer), or -1.
function hit(grid, x, y) {
    for (var i = 0; i < grid.rects.length; i++) {
        var r = grid.rects[i]
        if (x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h) return i
    }
    return -1
}

// ---- keys

// What pressing key `index` does, from the keyboard's state `st` ({shift: 0 off, 1 one capital, 2 caps lock}): {text} typed, {action} for
// the field ("backspace", "enter", "left", "right", "clear"), {mode} to show, {shift} the new shift, {voice}, {hide} when the keyboard goes
// away after it. An empty object when nothing happens.
function press(layout, index, st) {
    var k = layout.keys[index]
    if (!k) return {}
    switch (k.kind) {
    case "char": {
        var up = layout.shiftable && st.shift > 0
        var out = { text: up ? k.shifted : k.text }
        if (up && k.letter && st.shift === 1) out.shift = 0                  // one capital, then small letters again
        return out
    }
    case "text": return { text: k.text }
    case "space": return { text: " " }
    case "shift": return { shift: (st.shift + 1) % 3 }
    case "mode": return { mode: k.to }
    case "backspace": return { action: "backspace" }
    case "clear": return { action: "clear" }
    case "cursor": return { action: k.dir }
    case "voice": return { voice: true }
    case "hide": return { hide: true }
    case "enter": return k.hides ? { action: "enter", hide: true } : { action: "enter" }
    }
    return {}
}

// The label a key shows: the capital with Shift on, the words of the enter key.
function labelOf(layout, index, shift) {
    var k = layout.keys[index]
    if (!k) return ""
    return k.kind === "char" && layout.shiftable && shift > 0 ? k.shifted : k.label
}

// ---- the remote's repeats

// One held key of the remote arrives as a press and then a press for every repeat. The first press moves; the repeats are let through
// after `delay` ms and then at most one in `interval` ms, one in `fast` ms after `fastAfter` steps: a held arrow crosses the keyboard
// quickly without the pace being the remote's. `g` is the gate's own record ({key, first, last, steps}); returns whether to act.
function gate(g, key, repeat, now, cfg) {
    if (!repeat || g.key !== key) {
        g.key = key
        g.first = now
        g.last = now
        g.steps = 0
        return true
    }
    if (now - g.first < cfg.delay) return false
    if (now - g.last < (g.steps >= cfg.fastAfter ? cfg.fast : cfg.interval)) return false
    g.last = now
    g.steps++
    return true
}

// Which keys of the remote repeat at all: the arrows, and OK on a key that is held to be repeated (Backspace, the cursor keys).
function repeats(name, layout, index) {
    if (name === "up" || name === "down" || name === "left" || name === "right") return true
    if (name !== "ok") return false
    var k = layout.keys[index]
    return !!k && k.repeat === true
}
