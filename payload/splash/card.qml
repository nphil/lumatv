    // Luma: LumaTV's launch screen for an app that ships no splash picture of its own (docs/mocks/15-launch-splash, tools/splash_patch.py):
    // the app's picture, its name and "Opening" on the dark canvas. An app with its own picture keeps it; `lumaCard` is set by open() below.
    Item {
        id: lumaLaunch
        anchors.fill: parent
        visible: root.lumaCard && wiper.state === "open"
        readonly property real k: root.width / 1920

        Rectangle { anchors.fill: parent; color: "#000000" }
        Rectangle {
            anchors.fill: parent
            gradient: Gradient {
                GradientStop { position: 0.0; color: "@@sheetTop@@" }
                GradientStop { position: 1.0; color: "@@sheetBottom@@" }
            }
        }

        Column {
            anchors.centerIn: parent
            spacing: 24 * lumaLaunch.k

            Item {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 180 * lumaLaunch.k
                height: width
                Image {
                    id: lumaPicture
                    visible: false
                    source: root.lumaIcon
                    mipmap: true
                    sourceSize.width: 256
                    sourceSize.height: 256
                }
                // the picture as a rounded square, corners cut by the shader so a square-cornered file shows none
                ShaderEffect {
                    anchors.fill: parent
                    visible: lumaPicture.status === Image.Ready
                    property variant tex: lumaPicture
                    property vector3d dims: Qt.vector3d(width, height, 38 * lumaLaunch.k)
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
                            gl_FragColor = texture2D(tex, qt_TexCoord0) * a * qt_Opacity;
                        }"
                }
                Rectangle {
                    anchors.fill: parent
                    visible: lumaPicture.status !== Image.Ready
                    radius: 38 * lumaLaunch.k
                    color: "@@row@@"
                    border.width: 1
                    border.color: "@@hairline@@"
                }
            }

            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: root.lumaName
                font.pixelSize: 40 * lumaLaunch.k
                font.family: "Manrope"
                font.weight: Font.DemiBold
                color: "@@ink@@"
            }

            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: 14 * lumaLaunch.k

                ShaderEffect {
                    id: lumaRing
                    anchors.verticalCenter: parent.verticalCenter
                    width: 28 * lumaLaunch.k
                    height: width
                    property real turn: 0
                    property vector4d dims: Qt.vector4d(width, height, Math.max(2, 3 * lumaLaunch.k), width / 2)
                    property vector4d lit: Qt.vector4d(0.28, turn, 0, 0)
                    property color tint: "@@ink@@"
                    property color trackColor: "@@focusEdge@@"
                    NumberAnimation on turn { running: lumaLaunch.visible; from: 0; to: 1; duration: 900; loops: Animation.Infinite }
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
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Opening"
                    font.pixelSize: 24 * lumaLaunch.k
                    font.family: "Manrope"
                    font.weight: Font.Medium
                    color: "@@inkDim@@"
                }
            }
        }
    }

