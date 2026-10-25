.pragma library

// The wallpaper's tint, from a small RGBA bitmap (LumaHome draws the wallpaper into 32x18).
// Each channel is cut into 8 levels (512 bins). Transparent pixels and near-black ones (every
// channel under 32, the lowest level) are skipped, so a dark wallpaper tints by its colour and
// not by its black. The most common bin wins; its centre is the tint. A tie goes to the lower
// bin, so the answer for one picture never changes between samples.

var STEP = 32          // 256 / 8 levels
var CLEAR_BELOW = 128  // alpha

// {r, g, b, share} (0..255, share of all pixels), or null when every pixel was skipped.
function dominant(data, pixels) {
    var count = {}, best = -1, bestN = 0
    for (var i = 0; i < pixels; i++) {
        var r = data[4 * i], g = data[4 * i + 1], b = data[4 * i + 2]
        if (data[4 * i + 3] < CLEAR_BELOW) continue
        if (r < STEP && g < STEP && b < STEP) continue
        var bin = ((r >> 5) << 6) | ((g >> 5) << 3) | (b >> 5)
        var n = (count[bin] || 0) + 1
        count[bin] = n
        if (n > bestN || (n === bestN && bin < best)) { bestN = n; best = bin }
    }
    if (best < 0) return null
    return { r: ((best >> 6) & 7) * STEP + STEP / 2, g: ((best >> 3) & 7) * STEP + STEP / 2, b: (best & 7) * STEP + STEP / 2,
             share: bestN / pixels }
}

// Mean luma 0..1 of the opaque pixels: what decides light or dark glass for an "auto" material.
function luma(data, pixels) {
    var sum = 0, n = 0
    for (var i = 0; i < pixels; i++) {
        if (data[4 * i + 3] < CLEAR_BELOW) continue
        sum += 0.299 * data[4 * i] + 0.587 * data[4 * i + 1] + 0.114 * data[4 * i + 2]
        n++
    }
    return n ? sum / n / 255 : -1
}

function rgbToHsl(r, g, b) {
    r /= 255; g /= 255; b /= 255
    var mx = Math.max(r, g, b), mn = Math.min(r, g, b), l = (mx + mn) / 2, h = 0, s = 0
    if (mx !== mn) {
        var d = mx - mn
        s = l > 0.5 ? d / (2 - mx - mn) : d / (mx + mn)
        if (mx === r) h = ((g - b) / d + (g < b ? 6 : 0)) / 6
        else if (mx === g) h = ((b - r) / d + 2) / 6
        else h = ((r - g) / d + 4) / 6
    }
    return { h: h, s: s, l: l }
}

function hslToRgb(h, s, l) {
    function channel(p, q, t) {
        if (t < 0) t += 1
        if (t > 1) t -= 1
        if (t < 1 / 6) return p + (q - p) * 6 * t
        if (t < 1 / 2) return q
        if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6
        return p
    }
    if (s === 0) return { r: Math.round(l * 255), g: Math.round(l * 255), b: Math.round(l * 255) }
    var q = l < 0.5 ? l * (1 + s) : l + s - l * s, p = 2 * l - q
    return { r: Math.round(channel(p, q, h + 1 / 3) * 255), g: Math.round(channel(p, q, h) * 255), b: Math.round(channel(p, q, h - 1 / 3) * 255) }
}

// The accent a control is drawn in: the tint's hue, with saturation and lightness set so it
// reads on glass of either material (the Rose Pine accents are about 80 % saturated and 78 %
// light on dark glass, muted and mid-light on light glass).
function accentFrom(tint, isLight) {
    var c = rgbToHsl(tint.r, tint.g, tint.b)
    return hslToRgb(c.h, Math.max(c.s, isLight ? 0.35 : 0.55), isLight ? 0.42 : 0.74)
}

function hex(c) {
    function two(v) { var s = Math.max(0, Math.min(255, Math.round(v))).toString(16); return s.length < 2 ? "0" + s : s }
    return "#" + two(c.r) + two(c.g) + two(c.b)
}

// "#rrggbb" -> {r, g, b}, or null for anything else.
function parseHex(text) {
    var m = /^#([0-9a-fA-F]{2})([0-9a-fA-F]{2})([0-9a-fA-F]{2})$/.exec(String(text))
    return m ? { r: parseInt(m[1], 16), g: parseInt(m[2], 16), b: parseInt(m[3], 16) } : null
}

// Qt's XPM writer (QImage::save to .xpm) lists every distinct colour once, then one string per
// row, so a grabbed 32x18 image is a few KiB of text. Returns {data, count, width, height} with
// data as r, g, b, a bytes like canvas image data, or null when the text is not such a file.
function parseXpm(text) {
    var strings = String(text).match(/"[^"]*"/g)
    if (!strings) return null
    var head = strings[0].slice(1, -1).split(/\s+/)
    var w = parseInt(head[0], 10), h = parseInt(head[1], 10), n = parseInt(head[2], 10), cpp = parseInt(head[3], 10)
    if (!(w > 0 && h > 0 && n > 0 && cpp > 0) || strings.length < 1 + n + h) return null
    var colors = {}, i, x, y
    for (i = 0; i < n; i++) {
        var line = strings[1 + i].slice(1, -1)
        var named = /^\s*c\s+(\S+)/.exec(line.substr(cpp))
        if (!named) return null
        var rgb = /^#([0-9a-fA-F]{2})([0-9a-fA-F]{2})([0-9a-fA-F]{2})$/.exec(named[1])
        colors[line.substr(0, cpp)] = rgb ? [parseInt(rgb[1], 16), parseInt(rgb[2], 16), parseInt(rgb[3], 16), 255] : [0, 0, 0, 0]
    }
    var data = []
    for (y = 0; y < h; y++) {
        var row = strings[1 + n + y].slice(1, -1)
        for (x = 0; x < w; x++) {
            var c = colors[row.substr(x * cpp, cpp)]
            if (!c) return null
            data.push(c[0], c[1], c[2], c[3])
        }
    }
    return { data: data, count: w * h, width: w, height: h }
}

// `base` (a colour with alpha, as QML hands it: r, g, b, a in 0..1) moved toward the tint by
// `mix`, alpha kept: the glass picks up the wallpaper's hue without changing its weight.
function mixToward(base, tint, mix) {
    return { r: base.r + (tint.r / 255 - base.r) * mix, g: base.g + (tint.g / 255 - base.g) * mix,
             b: base.b + (tint.b / 255 - base.b) * mix, a: base.a }
}

// sRGB relative luminance of a colour given as r, g, b in 0..1 (the form QML colours have).
function luminance(c) {
    function lin(v) { return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4) }
    return 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

// WCAG contrast ratio of two opaque colours, 1..21.
function contrast(a, b) {
    var la = luminance(a), lb = luminance(b)
    return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05)
}

// The candidate with the highest contrast against `bg`; the first wins a tie.
function bestInk(bg, candidates) {
    var best = candidates[0], bestC = contrast(bg, candidates[0])
    for (var i = 1; i < candidates.length; i++) {
        var k = contrast(bg, candidates[i])
        if (k > bestC) { bestC = k; best = candidates[i] }
    }
    return best
}
