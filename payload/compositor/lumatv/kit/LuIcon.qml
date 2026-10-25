import QtQuick 2.4
import "."

// A Lucide icon in any colour. The PNGs (icons/lucide, generators/gen-lucide.py) are white strokes on
// transparent: the shader multiplies their alpha by `color`, so one 64 px file serves both materials,
// every state and every accent, and every icon on screen shares one program (warmed once).
Item {
    id: icon
    property string name: ""
    property color color: Lu.ink
    property int size: 32
    width: size
    height: size

    Image {
        id: img
        visible: false
        width: icon.size
        height: icon.size
        sourceSize.width: 64
        sourceSize.height: 64
        source: icon.name !== "" && Lu.baseUrl !== "" ? Lu.iconUrl(icon.name) : ""
        asynchronous: false
        cache: true
        smooth: true
    }
    ShaderEffect {
        anchors.fill: parent
        visible: icon.name !== "" && img.status === Image.Ready
        property variant src: img
        property color tint: icon.color
        fragmentShader: "
            uniform sampler2D src;
            uniform lowp vec4 tint;
            uniform lowp float qt_Opacity;
            varying highp vec2 qt_TexCoord0;
            void main() {
                lowp float a = texture2D(src, qt_TexCoord0).a * tint.a;
                gl_FragColor = vec4(tint.rgb * a, a) * qt_Opacity;
            }"
    }
}
