import QtQuick 2.4
import "."

// A ring: a quiet track and a lit arc, one quad and one small shader (the arc is cut by angle, so it stays crisp at
// any size and costs no Canvas). Two uses: a progress ring that fills clockwise from twelve o'clock to `progress`
// (the long-press ring around a tile, a countdown), and a spinner (`running`): a quarter-length arc that turns once
// a second. It follows the shape of a rounded rectangle when `cornerRadius` is set, so the same ring surrounds
// a round button or a square tile. A ring that is not running or not visible has no animation to pay for.
ShaderEffect {
    id: ring
    property real size: 64
    property real thick: 3
    property real cornerRadius: -1             // -1: a circle (half the shorter side)
    property real progress: 1                  // 0..1 of the way round
    property bool running: false
    property color tint: Lu.accent
    property color trackColor: Lu.trackOff
    property real turn: 0
    width: size
    height: size
    NumberAnimation on turn { running: ring.running && ring.visible; from: 0; to: 1; duration: 900; loops: Animation.Infinite }

    property vector4d dims: Qt.vector4d(width, height, thick, cornerRadius < 0 ? Math.min(width, height) / 2 : cornerRadius)
    property vector4d lit: Qt.vector4d(running ? 0.28 : progress, running ? turn : 0, 0, 0)
    fragmentShader: "
        uniform highp vec4 dims;
        uniform highp vec4 lit;
        uniform lowp vec4 tint;
        uniform lowp vec4 trackColor;
        uniform lowp float qt_Opacity;
        varying highp vec2 qt_TexCoord0;
        highp float sd(highp vec2 p, highp vec2 hs, highp float r) {
            highp vec2 q = abs(p) - (hs - vec2(r));
            return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
        }
        void main() {
            highp vec2 px = (qt_TexCoord0 - 0.5) * dims.xy;
            highp float half_t = dims.z * 0.5;
            highp float d = abs(sd(px, dims.xy * 0.5 - vec2(half_t), max(dims.w - half_t, 0.0)));
            highp float band = 1.0 - smoothstep(half_t - 0.75, half_t + 0.75, d);
            highp float a = fract(atan(px.x, -px.y) / 6.2831853 - lit.y);
            lowp float on = step(a, lit.x);
            lowp vec4 c = mix(trackColor, tint, on);
            gl_FragColor = vec4(c.rgb * c.a, c.a) * band * qt_Opacity;
        }"
}
