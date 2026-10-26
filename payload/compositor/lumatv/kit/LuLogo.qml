import QtQuick 2.4
import "."

// An audio-format mark (icons/audio, generators/gen-audio-logos.py) in any colour: white on transparent, so the shader multiplies its alpha by `color`
// and one file serves both materials. The marks are wide, so `height` is given and the width follows the picture's proportions. `px` is the height
// the file is decoded at, so a mark drawn small is reduced once with Qt's own smoothing and not sampled four times down by the GPU: the caller gives
// twice the height it draws at (a 1080p surface shown on a 4K panel), and not the item's own height, which would then depend on `aspect`, which
// would depend on it. The shader is LuIcon's, so the program is shared.
Item {
    id: logo
    property string name: ""
    property string label: ""                     // what the mark says, for a reader that cannot see it
    property color color: Lu.ink
    property int px: 0                            // decode height; 0 keeps the file's own
    readonly property real aspect: img.implicitHeight > 0 ? img.implicitWidth / img.implicitHeight : 0
    readonly property bool ready: img.status === Image.Ready
    height: 28
    width: Math.round(height * aspect)
    Accessible.role: Accessible.Graphic
    Accessible.name: label

    Image {
        id: img
        visible: false
        anchors.fill: parent
        sourceSize.height: logo.px
        source: logo.name !== "" && Lu.baseUrl !== "" ? Lu.logoUrl(logo.name) : ""
        asynchronous: false
        cache: true
        smooth: true
    }
    ShaderEffect {
        anchors.fill: parent
        visible: logo.ready
        property variant src: img
        property color tint: logo.color
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
