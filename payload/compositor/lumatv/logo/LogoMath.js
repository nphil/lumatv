.pragma library

// Pure helpers for the power-on and power-off logos. Every frame of a logo is a function of the
// elapsed milliseconds, so a still and the running animation can never disagree.

function clamp01(x) { return x < 0 ? 0 : x > 1 ? 1 : x }

// 0..1 position of t inside the span [start, start + dur].
function seg(t, start, dur) { return dur <= 0 ? (t >= start ? 1 : 0) : clamp01((t - start) / dur) }

function outCubic(x) { var u = 1 - x; return 1 - u * u * u }
function inCubic(x) { return x * x * x }
function inOutCubic(x) { return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2 }
function outQuad(x) { return 1 - (1 - x) * (1 - x) }
function inQuad(x) { return x * x }

// An easing named as in theme.json's motion tokens ("OutCubic"); unknown names fall back to it.
function ease(name, x) {
    switch (name) {
    case "InCubic": return inCubic(x)
    case "InOutCubic": return inOutCubic(x)
    case "OutQuad": return outQuad(x)
    case "InQuad": return inQuad(x)
    case "Linear": return x
    default: return outCubic(x)
    }
}

function lerp(a, b, x) { return a + (b - a) * x }

// The wordmark is set in two weights, "Luma" light and "TV" bold: the split is the last place a
// lower-case letter is followed by a capital. A name without one is a single regular run.
function splitName(name) {
    for (var i = name.length - 1; i > 0; i--) {
        var a = name.charAt(i - 1), b = name.charAt(i)
        if (a !== a.toUpperCase() && b !== b.toLowerCase())
            return { head: name.substring(0, i), tail: name.substring(i) }
    }
    return { head: name, tail: "" }
}

// Letters of the name with the weight each is set in.
function letters(name) {
    var s = splitName(name), out = [], i
    for (i = 0; i < s.head.length; i++) out.push({ ch: s.head.charAt(i), bold: false })
    for (i = 0; i < s.tail.length; i++) out.push({ ch: s.tail.charAt(i), bold: true })
    return out
}

// The slow wander that keeps the lit pixels moving: whole pixels, a wide Lissajous figure.
// clock is in seconds.
function driftX(clock) { return Math.round(4 * Math.sin(2 * Math.PI * clock / 32)) }
function driftY(clock) { return Math.round(3 * Math.sin(2 * Math.PI * clock / 20 + 1)) }
// 0..1, a six-second breath.
function breath(clock) { return 0.5 + 0.5 * Math.sin(2 * Math.PI * clock / 6) }
