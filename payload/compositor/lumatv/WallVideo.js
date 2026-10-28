.pragma library

// Which of the moving wallpaper's clips (tools/vplane_wall.py writes them and their manifest, `wall.json`) is the one for what Home shows now,
// and why there is none. The clip is the wallpaper picture through the GPU wallpaper's warp with the dark material's scrim baked in, so it
// stands in for the GPU wallpaper only while everything it baked is still what Home would draw: the picture, the look
// (theme.wallpaperMotion: amplitude, speed, depth, scale, detail, vignette, specular, bloom, saturation, contrast) and the scrim. Anything
// else (another wallpaper, a look the owner is adjusting, the light material) draws the GPU wallpaper, as it did before there was a clip.

var KEYS = ["amplitude", "speed", "depth", "scale", "detail", "vignette", "specular", "bloom", "saturation", "contrast"]
var DEFAULTS = { amplitude: 4, speed: 1, depth: 0.25, scale: 1, detail: 0.35, vignette: 0, specular: 0, bloom: 0, saturation: 1, contrast: 1 }

// The look as LumaHome reads it: a key the theme lacks is the QML's own default.
function look(motion) {
    var out = {}
    for (var i = 0; i < KEYS.length; i++) {
        var k = KEYS[i]
        out[k] = motion && motion[k] !== undefined ? motion[k] : DEFAULTS[k]
    }
    return out
}

function near(a, b) { return Math.abs(Number(a) - Number(b)) < 1e-6 }

// The names of the settings in `motion` that are not the clip's, in the order of KEYS.
function differs(clipLook, motion) {
    var want = look(motion), bad = []
    for (var i = 0; i < KEYS.length; i++) {
        var k = KEYS[i]
        if (!near(clipLook.motion[k], want[k])) bad.push(k)
    }
    return bad
}

function sameScrim(a, b) {
    if (!a || !b || a.length !== b.length) return false
    for (var i = 0; i < a.length; i++) if (!near(a[i], b[i])) return false
    return true
}

// {entry, why}: the manifest's clip for the wallpaper `wallpaper` (theme.wallpaper: a file of the theme directory, by its bare name) under the
// look `motion` and the dark material's scrim `scrim`, or null and the reason.
function pick(manifest, wallpaper, motion, scrim) {
    if (!manifest || !manifest.walls || !manifest.walls.length) return { entry: null, why: "no clips" }
    var name = String(wallpaper || "")
    var same = []
    for (var i = 0; i < manifest.walls.length; i++) if (manifest.walls[i].source && manifest.walls[i].source.file === name) same.push(manifest.walls[i])
    if (!same.length) return { entry: null, why: "no clip for this wallpaper" }
    var why = ""
    for (var j = 0; j < same.length; j++) {
        var w = same[j], bad = differs(w.look, motion)
        if (bad.length) { why = "the look is not the clip's (" + bad.join(", ") + ")"; continue }
        if (!sameScrim(w.look.scrim, scrim)) { why = "the scrim is not the clip's"; continue }
        return { entry: w, why: "" }
    }
    return { entry: null, why: why }
}

// The clip's file name without the extension: the name a start request gives the player.
function clipName(entry) { return entry ? String(entry.file).replace(/\.vpc$/, "") : "" }
