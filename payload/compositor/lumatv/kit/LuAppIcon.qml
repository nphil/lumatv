import QtQuick 2.4
import "."

// An app's picture as a rounded square, for the places that draw one app large (the launch screen, a recent-apps frame, the
// Quick Access question): one quad with exact antialiased corners, the way Home's tiles draw it (Tile.qml), so an icon whose
// file has square corners never shows them. `source` empty, or a file that is not there, draws the generic mark in the pane's wash (`placeholder`). No layer and no shadow.
Item {
    id: icon
    property url source: ""
    property real radius: Math.round(Math.min(width, height) * 0.21)
    property int decode: 256                        // the picture is decoded at most this wide, mipmapped
    property bool placeholder: true                 // draw the generic mark while there is no picture (false: nothing, for a picture laid over another)
    readonly property bool ready: img.status === Image.Ready
    width: 96
    height: 96

    Image {
        id: img
        visible: false
        source: icon.source
        asynchronous: false
        cache: true
        mipmap: true
        sourceSize.width: icon.decode
        sourceSize.height: icon.decode
    }
    ShaderEffect {
        anchors.fill: parent
        visible: icon.ready
        property variant tex: img
        property vector3d dims: Qt.vector3d(width, height, icon.radius)
        fragmentShader: "
            uniform sampler2D tex;
            uniform highp vec3 dims;
            uniform lowp float qt_Opacity;
            varying highp vec2 qt_TexCoord0;
            void main() {
                highp vec2 p = (qt_TexCoord0 - 0.5) * dims.xy;
                highp vec2 q = abs(p) - (dims.xy * 0.5 - vec2(dims.z));
                highp float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - dims.z;
                lowp float a = 1.0 - smoothstep(-0.75, 0.75, d);
                lowp vec4 c = texture2D(tex, qt_TexCoord0);
                gl_FragColor = c * a * qt_Opacity;
            }"
    }
    Rectangle {
        anchors.fill: parent
        visible: icon.placeholder && !icon.ready
        radius: icon.radius
        color: Lu.wash("selected")
        border.width: 1
        border.color: Lu.edge
        antialiasing: true
        LuIcon { anchors.centerIn: parent; name: "layout-grid"; size: Math.round(parent.width * 0.4); color: Lu.ink2 }
    }
}
