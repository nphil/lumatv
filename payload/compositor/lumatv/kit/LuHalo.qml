import QtQuick 2.4
import "."

// The soft accent halo that spreads under a focused control, one radial-falloff quad: the accent
// at `strength` in the middle, nothing at 70 % of the way to the edge. It follows the highlight, not
// the lift: it fades in and out over the card duration when `on` changes, and the host moves or
// resizes it as it likes. Size it to about twice the control it sits under, and put it below it.
ShaderEffect {
    id: halo
    property bool on: false
    property color tint: Lu.accent
    property real strength: Lu.focus.ambient || 0

    opacity: on ? strength : 0
    visible: opacity > 0.01
    Behavior on opacity { enabled: !Lu.reduced; NumberAnimation { duration: Lu.cardMs } }

    property vector4d tintC: Qt.vector4d(tint.r, tint.g, tint.b, 0.5)
    fragmentShader: "
        uniform highp vec4 tintC;
        uniform lowp float qt_Opacity;
        varying highp vec2 qt_TexCoord0;
        void main() {
            highp float r = length((qt_TexCoord0 - 0.5) * 2.0);
            highp float a = tintC.a * (1.0 - smoothstep(0.0, 0.7, r));
            gl_FragColor = vec4(tintC.rgb * a, a) * qt_Opacity;
        }"
}
