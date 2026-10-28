.pragma library

// Where the app switcher's cards go (docs/mocks/switcher): the shell places them, the compositor draws each app's picture there
// (compositor/crates/lumatv-compositor/src/switcher.rs), so the places are the one thing both must agree on and they are made here, once.
// 1920x1080 TV pixels. A picture is 400x225 (16:9) with corners of 22, four fit side by side with 32 between and start at x 112 (fewer are
// centred); pictures sit at y 340. More than four are a window of four that moves by a card when the focus leaves it.
var W = 400
var H = 225
var GAP = 32
var TOP = 340
var RADIUS = 22
var VISIBLE = 4
var SCREEN = 1920

function shown(n) { return Math.min(n, VISIBLE) }

function x0(n) {
    var v = shown(n)
    return Math.round((SCREEN - (v * W + (v - 1) * GAP)) / 2)
}

// The first card of the window, moved only as far as keeps `index` inside it.
function windowStart(n, index, first) {
    var last = Math.max(0, n - VISIBLE)
    if (index < first) first = index
    else if (index > first + VISIBLE - 1) first = index - VISIBLE + 1
    return Math.max(0, Math.min(last, first))
}

function inWindow(i, first) { return i >= first && i < first + VISIBLE }

// Card i's rectangle when the window starts at `first`: cards past either end carry on along the same row, out of sight.
function card(i, n, first) { return { x: x0(n) + (i - first) * (W + GAP), y: TOP, w: W, h: H, r: RADIUS } }

// The compositor's half of a layout: the cards in the window whose picture it draws (an app whose picture is on the video plane has none).
function placed(entries, first) {
    var out = []
    for (var i = 0; i < entries.length; i++) {
        if (!inWindow(i, first) || entries[i].picture !== "live") continue
        var c = card(i, entries.length, first)
        out.push({ id: entries[i].id, x: c.x, y: c.y, w: c.w, h: c.h, r: c.r })
    }
    return out
}

// Whole 1920x1080 of the focus's neighbours, as the key moves it. `at` = { level: "card" | "quit" | "all", index }; the answer is a new `at`
// (the same one when the key leads nowhere). Up goes toward the header (Quit all), down toward the card's own Quit.
function move(at, dir, n) {
    if (n <= 0) return at
    var l = at.level, i = Math.max(0, Math.min(n - 1, at.index))
    if (dir === "left" && l !== "all") return { level: l, index: Math.max(0, i - 1) }
    if (dir === "right" && l !== "all") return { level: l, index: Math.min(n - 1, i + 1) }
    if (dir === "down") return l === "card" ? { level: "quit", index: i } : l === "all" ? { level: "card", index: i } : { level: l, index: i }
    if (dir === "up") return l === "quit" ? { level: "card", index: i } : l === "card" ? { level: "all", index: i } : { level: l, index: i }
    return { level: l, index: i }
}
