    // Luma: the startup cover in the Lucent look (docs/mocks/26-startup-cover, tools/cover_patch.py): the first app's tile and name, a ring and
    // what is happening. Everything LG's sequencing does in this file (the first app, smart-service-ready, the keys, the fade) is as it was;
    // the ids `title` and `logoText` stay because the close animation fades `logoText`, and `blockUI` is LG's own screen for a stolen set.
    Column {
        anchors.centerIn: parent
        width: parent.width
        spacing: styleSheet.getRValue(40)
        visible: !blockUI.visible

        Image {
            id: lumaTile
            anchors.horizontalCenter: parent.horizontalCenter
            width: styleSheet.getRValue(160)
            height: width
            sourceSize.width: 320
            sourceSize.height: 320
            smooth: true
            source: String(systemProperties.appIdToLaunch || "").indexOf("com.webos.app.hdmi") === 0 ? "cover-input.png" : "cover-app.png"
            visible: logoText.visible
            opacity: logoText.opacity
        }

        Text {
            id: title

            objectName: "mainTitle"

            width: parent.width
            anchors.horizontalCenter: parent.horizontalCenter
            visible: systemProperties.appNameToLaunch !== undefined && logoText.visible
            text: systemProperties.appNameToLaunch !== undefined && systemProperties.appNameToLaunch !== "" ? systemProperties.appNameToLaunch : (systemProperties.cpAppNames[systemProperties.appToLaunch.id])
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            font.pixelSize: styleSheet.getRValue(40)
            font.family: "Manrope"
            font.weight: Font.DemiBold
            color: "@@ink@@"
            opacity: logoText.opacity
        }

        Item {
            id: logoText

            anchors.horizontalCenter: parent.horizontalCenter
            width: lumaStatus.width
            height: lumaStatus.height
            visible: !systemProperties.supportsBNO && systemProperties.appToLaunch && systemProperties.appNameToLaunch !== undefined

            Row {
                id: lumaStatus
                spacing: styleSheet.getRValue(14)

                // the ring: a quiet track and a quarter-length arc that turns once a second, cut by angle in one small shader
                ShaderEffect {
                    id: lumaRing
                    anchors.verticalCenter: parent.verticalCenter
                    width: styleSheet.getRValue(28)
                    height: width
                    property real turn: 0
                    property vector4d dims: Qt.vector4d(width, height, Math.max(2, styleSheet.getRValue(3)), width / 2)
                    property vector4d lit: Qt.vector4d(0.28, turn, 0, 0)
                    property color tint: "@@ink@@"
                    property color trackColor: "@@focusEdge@@"
                    NumberAnimation on turn { running: logoText.visible; from: 0; to: 1; duration: 900; loops: Animation.Infinite }
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
                    text: systemProperties.lastInput && String(systemProperties.appIdToLaunch) === String(systemProperties.lastInput.id) ? "Starting where you left off" : "Starting"
                    font.pixelSize: styleSheet.getRValue(24)
                    font.family: "Manrope"
                    font.weight: Font.Medium
                    color: "@@inkDim@@"
                }
            }
        }
    }

    Row {
        anchors.left: parent.left
        anchors.leftMargin: styleSheet.getRValue(64)
        anchors.bottom: parent.bottom
        anchors.bottomMargin: styleSheet.getRValue(56)
        spacing: styleSheet.getRValue(12)
        opacity: 0.65
        visible: !blockUI.visible

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: styleSheet.getRValue(32)
            height: width
            radius: styleSheet.getRValue(10)
            color: "@@accent@@"
        }
        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: "@@name@@"
            font.pixelSize: styleSheet.getRValue(24)
            font.family: "Manrope"
            font.weight: Font.Medium
            color: "@@inkDim@@"
        }
    }

