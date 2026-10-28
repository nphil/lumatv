import QtQuick 2.12

// Luma: a Lucide glyph in one ink, for the input app's no-signal screen (the compositor's LuIcon, which this process does not share).
// The picture is white strokes on transparent: the shader keeps its alpha and paints `color` through it.
Item {

    id: root

    property string name: ""
    property color color: "white"
    property int size: 28

    width: size
    height: size

    Image {

        id: picture

        visible: false
        width: root.size
        height: root.size
        sourceSize.width: 64
        sourceSize.height: 64
        source: root.name !== "" ? Qt.resolvedUrl("../../Images/luma/" + root.name + ".png") : ""
        cache: true
        smooth: true
    }

    ShaderEffect {

        anchors.fill: parent
        visible: picture.status === Image.Ready
        property variant src: picture
        property color tint: root.color
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
