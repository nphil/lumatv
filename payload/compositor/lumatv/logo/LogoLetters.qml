import QtQuick 2.12
import "LogoMath.js" as M

// The name as separate letters (regular head, bold tail, see LogoMath.letters), each with its own
// progress `k(index)`: each letter takes `spanMs` from `startMs`, starting `staggerMs` after the one
// before (`reverse` starts from the last), and fades in or, `leaving`, out.
// One Text per letter on whole pixels; the row is centred by its owner.
Row {
    id: row

    property string name: ""
    property string family: "Manrope"
    property int pixelSize: 112
    property color color: "white"
    property real t: 0                  // ms into the animation
    property int startMs: 0
    property int staggerMs: 60
    property int spanMs: 260
    property bool reverse: false
    property bool leaving: false        // fade out and sink instead of fade in and rise
    property real lift: 14              // px a letter travels while it appears, or sinks while it leaves
    property string easing: "OutCubic"

    readonly property var chars: M.letters(name)

    function k(i) {
        var order = reverse ? chars.length - 1 - i : i
        return M.ease(easing, M.seg(t, startMs + order * staggerMs, spanMs))
    }
    // The time at which the last letter has finished.
    readonly property int endMs: startMs + Math.max(0, chars.length - 1) * staggerMs + spanMs

    Repeater {
        model: row.chars
        delegate: Text {
            readonly property real e: row.k(index)
            text: modelData.ch
            color: row.color
            font.family: row.family
            font.weight: modelData.bold ? Font.Bold : Font.Normal
            font.pixelSize: row.pixelSize
            font.letterSpacing: -0.03 * row.pixelSize
            renderType: Text.NativeRendering
            opacity: row.leaving ? 1 - e : e
            y: Math.round(row.lift * (row.leaving ? e : 1 - e))
        }
    }
}
