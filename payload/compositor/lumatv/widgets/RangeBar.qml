import QtQuick 2.4
import "../kit"

// How far one day's low and high reach, on the scale of the whole week: a capsule track with the day's span filled from the colour of
// its low to the colour of its high (cool to warm by temperature, so the same degrees look the same in every row). `vertical` stands it
// up, the high at the top. A day with no spread still shows a dot. The fill is one quad: a rounded shape and the two colours in a
// fragment shader (QtQuick 2.4 has no horizontal gradient).
Item {
    id: bar
    property real lo: 0                      // the day's low and high
    property real hi: 0
    property real min: 0                     // the week's coolest and warmest
    property real max: 1
    property bool vertical: false
    property int thickness: 6

    readonly property real span: Math.max(1, max - min)
    readonly property real a: Math.max(0, Math.min(1, (lo - min) / span))
    readonly property real b: Math.max(0, Math.min(1, (hi - min) / span))
    readonly property int length: Math.max(thickness, vertical ? height : width)
    readonly property int from: Math.min(length - thickness, Math.round(a * length))
    readonly property int to: Math.min(length, Math.max(from + thickness, Math.round(b * length)))
    function warmth(t) {
        var c = Lu.info, w = Lu.warning
        return Qt.rgba(c.r + (w.r - c.r) * t, c.g + (w.g - c.g) * t, c.b + (w.b - c.b) * t, 1)
    }

    Rectangle {
        // the track
        x: bar.vertical ? Math.round((bar.width - bar.thickness) / 2) : 0
        y: bar.vertical ? 0 : Math.round((bar.height - bar.thickness) / 2)
        width: bar.vertical ? bar.thickness : bar.width
        height: bar.vertical ? bar.height : bar.thickness
        radius: bar.thickness / 2
        color: Lu.trackOff
        antialiasing: true
    }
    ShaderEffect {
        // the day: its low's colour at the cool end of the span, its high's at the warm end
        x: bar.vertical ? Math.round((bar.width - bar.thickness) / 2) : bar.from
        y: bar.vertical ? bar.length - bar.to : Math.round((bar.height - bar.thickness) / 2)
        width: bar.vertical ? bar.thickness : bar.to - bar.from
        height: bar.vertical ? bar.to - bar.from : bar.thickness
        property color c0: bar.warmth(bar.a)
        property color c1: bar.warmth(bar.b)
        property vector2d dims: Qt.vector2d(width, height)
        property real vert: bar.vertical ? 1 : 0
        fragmentShader: "
            uniform lowp vec4 c0;
            uniform lowp vec4 c1;
            uniform highp vec2 dims;
            uniform lowp float vert;
            uniform lowp float qt_Opacity;
            varying highp vec2 qt_TexCoord0;
            void main() {
                highp vec2 p = qt_TexCoord0 * dims;
                highp float r = min(dims.x, dims.y) * 0.5;
                highp vec2 q = abs(p - dims * 0.5) - (dims * 0.5 - vec2(r));
                highp float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
                lowp float cover = 1.0 - smoothstep(-0.75, 0.75, d);
                lowp float t = mix(qt_TexCoord0.x, 1.0 - qt_TexCoord0.y, vert);
                lowp vec4 c = mix(c0, c1, t);
                gl_FragColor = vec4(c.rgb, 1.0) * cover * qt_Opacity;
            }"
    }
}
