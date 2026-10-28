    // Luma: LumaTV's spinner (docs/mocks/15-spinner, tools/splash_patch.py) in place of LG's grey disc with a GIF: the picture dimmed, one ring
    // in the middle and, once the wait has lasted a second, "Loading" in a dark pill. LG's timers, the input-blocking mouse area and the
    // open and close of the view are as they were.
    Item {
        id: spinnerBackground
        anchors.fill: parent
        readonly property real k: root.width / 1920
        property bool wordShown: false
        onVisibleChanged: if (!visible) wordShown = false
        Timer { interval: 1000; running: root.isOpen && !spinnerBackground.wordShown; onTriggered: spinnerBackground.wordShown = true }

        Rectangle { anchors.fill: parent; color: "@@scrim@@" }

        Column {
            anchors.centerIn: parent
            spacing: 18 * spinnerBackground.k

            ShaderEffect {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 56 * spinnerBackground.k
                height: width
                property real turn: 0
                property vector4d dims: Qt.vector4d(width, height, Math.max(2, 5 * spinnerBackground.k), width / 2)
                property vector4d lit: Qt.vector4d(0.28, turn, 0, 0)
                property color tint: "@@ink@@"
                property color trackColor: "@@focusEdge@@"
                NumberAnimation on turn { running: root.isOpen; from: 0; to: 1; duration: 900; loops: Animation.Infinite }
                fragmentShader: "
                    uniform highp vec4 dims;
                    uniform highp vec4 lit;
                    uniform lowp vec4 tint;
                    uniform lowp vec4 trackColor;
                    uniform lowp float qt_Opacity;
                    varying highp vec2 qt_TexCoord0;
                    void main() {
                        highp vec2 px = (qt_TexCoord0 - 0.5) * dims.xy;
                        highp float half_t = dims.z * 0.5;
                        highp float d = abs(length(px) - (dims.x * 0.5 - half_t));
                        highp float band = 1.0 - smoothstep(half_t - 0.75, half_t + 0.75, d);
                        highp float a = fract(atan(px.x, -px.y) / 6.2831853 - lit.y);
                        lowp float on = step(a, lit.x);
                        lowp vec4 c = mix(trackColor, tint, on);
                        gl_FragColor = vec4(c.rgb * c.a, c.a) * band * qt_Opacity;
                    }"
            }
            Rectangle {
                anchors.horizontalCenter: parent.horizontalCenter
                opacity: spinnerBackground.wordShown ? 1 : 0
                width: word.implicitWidth + 36 * spinnerBackground.k
                height: 44 * spinnerBackground.k
                radius: height / 2
                color: "@@sheetBottom@@"
                Text {
                    id: word
                    anchors.centerIn: parent
                    text: "Loading"
                    font.pixelSize: 24 * spinnerBackground.k
                    font.family: "Manrope"
                    font.weight: Font.DemiBold
                    color: "@@ink@@"
                }
            }
        }
    }

