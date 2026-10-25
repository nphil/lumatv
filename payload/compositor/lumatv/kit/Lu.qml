pragma Singleton
import QtQuick 2.4
import "../Tint.js" as Tint

// The Lucent token layer for the compositor's QML. Every colour, size, radius and duration a kit
// component draws with comes from here, and everything here comes from theme.json: LumaHome
// hands over the merged theme (shipped defaults under the user's edits), whether the glass is
// light, the wallpaper's accent and the shared blurred backdrop, and the rest follows by binding,
// so a theme edit, the auto light/dark switch and a new wallpaper reach every component at once.
// Nothing here is a design decision: a number that is not in theme.json does not exist.
QtObject {
    id: lu

    // ---- handed over by the host ----
    property var theme: ({})
    property bool isLight: false
    property color wallAccent: "transparent"     // transparent until the wallpaper has been sampled
    property Item backdrop: null                  // ShaderEffectSource of the blurred wallpaper, for Glass
    property string baseUrl: ""                   // the module directory as a file URL: where icons/ lives
    function iconUrl(name) { return baseUrl + "icons/lucide/" + name + ".png" }

    // ---- colour roles of the material in force ----
    readonly property var material: (isLight ? theme.light : theme.dark) || ({})
    // Qt 5.12 has no Qt.color(): a colour string becomes a colour value (r, g, b, a in 0..1) by
    // passing it through an operation that returns one and changes nothing.
    function asColor(value) { return Qt.lighter(value, 1.0) }
    function role(name) { var v = material[name]; return typeof v === "string" ? v : "transparent" }
    readonly property color canvas: role("canvas")
    readonly property color sheet: role("sheet")
    readonly property color scrim: role("scrim")
    readonly property color glass: role("glass")
    readonly property color glassRaised: role("glassFocus")
    readonly property color edge: role("glassEdge")
    readonly property color edgeRaised: role("glassFocusEdge")
    readonly property color ink: role("ink")
    readonly property color ink2: role("ink2")
    readonly property color ink3: role("ink3")
    readonly property color trackOff: role("trackOff")
    readonly property color positive: role("positive")
    readonly property color warning: role("warning")
    readonly property color danger: role("danger")
    readonly property color info: role("info")
    readonly property color live: role("live")

    // The accent is one colour with one job, action or state. theme.accent says where it comes
    // from: the palette's (the shipped "tile" mode keeps it for controls), the wallpaper's tint, or
    // a fixed #rrggbb.
    readonly property string accentMode: theme.accent || "tile"
    readonly property color accent: accentMode === "wallpaper" && wallAccent.a > 0 ? wallAccent
                                    : accentMode.charAt(0) === "#" ? accentMode : role("accent")
    // Text on the accent: whichever of the palette's accentInk and the two inks reads best on it.
    readonly property color accentInk: Tint.bestInk(accent, [asColor(role("accentInk")),
                                       asColor(theme.dark ? theme.dark.ink : role("ink")),
                                       asColor(theme.light ? theme.light.ink : role("ink"))])

    // ---- states: washes are the ink at a small alpha ----
    readonly property var states: theme.states || ({})
    function wash(kind) {
        var a = kind === "focus" ? (states.focusWash ? states.focusWash[isLight ? 1 : 0] : 0) : (states[kind + "Wash"] || 0)
        return Qt.rgba(ink.r, ink.g, ink.b, a)
    }
    readonly property real disabledOpacity: states.disabledOpacity || 1

    // ---- depth: how far glass lifts off the wallpaper (the shadow's reach and strength, the edge's weight) ----
    // theme.depth.level names a step of theme.depth.steps; Glass reads the number. 1 is the material as designed.
    readonly property var depthTokens: theme.depth || ({})
    readonly property string depthLevel: depthTokens.level || "normal"
    readonly property real depth: depthTokens.steps && depthTokens.steps[depthLevel] !== undefined ? depthTokens.steps[depthLevel] : 1

    // ---- type ----
    readonly property string family: theme.font && theme.font.family ? theme.font.family : ""
    function weightOf(name) {
        return name === "bold" ? Font.Bold : name === "semibold" ? Font.DemiBold : name === "medium" ? Font.Medium : Font.Normal
    }
    // {size, weight, line, tracking}: tracking in pixels, line a multiple of the size.
    function type(roleName) {
        var t = (theme.type && theme.type[roleName]) || ({})
        var size = t.size || 0
        return { size: size, weight: weightOf(t.weight), line: t.line || 1, tracking: (t.tracking || 0) * size }
    }

    // ---- shape and space ----
    function radius(name) { var r = theme.radii; return r && r[name] !== undefined ? r[name] : 0 }
    function space(n) { var s = theme.space; return s && s[n - 1] !== undefined ? s[n - 1] : 0 }
    readonly property int target: theme.target || 0      // smallest hit size, TV profile
    readonly property int row: theme.row || 0            // a list row's height, TV profile

    // ---- focus ----
    readonly property var focus: theme.focus || ({})
    readonly property real rowScale: focus.rowScale || 1
    readonly property real buttonScale: focus.buttonScale || 1
    readonly property real pressScale: focus.pressScale || 1
    readonly property int landing: focus.landing || 0     // the landing bar's thickness, px

    // ---- motion ----
    readonly property var motion: theme.motion || ({})
    readonly property bool reduced: !!motion.reduced
    function ms(name) { return motion[name] || 0 }
    function easing(name) { var v = name ? Easing[name] : undefined; return v !== undefined ? v : Easing.OutCubic }
    readonly property int focusMs: ms("focusMs")
    readonly property int cardMs: ms("cardMs")
    readonly property int layerMs: ms("layerMs")
    readonly property int pressMs: ms("pressMs")
    readonly property int restMs: ms("restMs")
    readonly property int focusEasing: easing(motion.focusEasing)
    readonly property int cardEasing: easing(motion.cardEasing)
}
