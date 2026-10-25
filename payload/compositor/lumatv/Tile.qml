import QtQuick 2.9

// One dock tile: plate, icon, inset edge, shadow, accent glow, top rim and sheen are a single
// shader quad (icon sampled from an offscreen mipmapped Image), so a tile is one draw call
// with exact antialiased corners at both rest and focus scale. Focus is depth, not a border:
// the quad lifts and scales, the shadow deepens and drops further, the rim and sheen come up. The name pill
// is an unscaled sibling, above the tile on the dock's top row and below it otherwise.
Item {
    id: tile

    property var launchPoint: ({})
    property bool focused: false
    property bool floating: false
    property color plate: "#2a2833"
    property color accent: "#9a8cf0"
    property int size: 120
    property int tileRadius: 22
    property int inset: 19
    property color ink: "#F2EFF6"
    property bool isLight: false
    property real focusScale: 1.12
    property int focusMs: 150
    property int focusEasing: Easing.OutCubic
    property int labelMs: 120
    property bool motionReduced: false
    property bool labelAbove: false
    property real glowStrength: 0      // accent halo around the focused tile; 0 = depth only
    property bool moving: false        // picked up for rearranging: higher lift and scale

    signal activated()
    signal holdActivated()
    signal hoverFocus()

    width: size
    height: size
    z: moving ? 4 : focused ? 3 : 1

    property real f: moving ? 1.6 : focused ? 1 : 0
    Behavior on f { enabled: !tile.motionReduced; NumberAnimation { duration: tile.focusMs; easing.type: tile.focusEasing } }

    Image {
        id: iconImage
        visible: false
        source: tile.launchPoint.iconUrl || ""
        asynchronous: true
        mipmap: true
        sourceSize.width: 240
        sourceSize.height: 240
    }

    ShaderEffect {
        id: face
        // shadow reach when held is 22 + 28 px plus the lift; the glow (off by default) fades by 64
        property real margin: 56
        x: -margin
        y: -margin - 6 * tile.f
        width: tile.size + 2 * margin
        height: tile.size + 2 * margin
        scale: 1 + (tile.focusScale - 1) * tile.f
        transformOrigin: Item.Center

        property variant icon: iconImage
        property vector2d dims: Qt.vector2d(width, height)
        property real mrg: margin
        property real radius: tile.tileRadius
        property real inset: tile.floating ? tile.inset : 0
        property vector2d iconAspect: {
            var a = iconImage.implicitHeight > 0 ? iconImage.implicitWidth / iconImage.implicitHeight : 1
            if (tile.floating) return a >= 1 ? Qt.vector2d(1, a) : Qt.vector2d(1 / a, 1)
            return a >= 1 ? Qt.vector2d(1 / a, 1) : Qt.vector2d(1, a)
        }
        property vector4d plateC: Qt.vector4d(tile.plate.r, tile.plate.g, tile.plate.b, 1)
        property vector4d accentC: Qt.vector4d(tile.accent.r, tile.accent.g, tile.accent.b, 1)
        property real f: Math.min(1, tile.f)   // shading saturates at focus; extra lift is transform only
        property real glowA: tile.glowStrength

        fragmentShader: "
            uniform sampler2D icon;
            uniform highp vec2 dims;
            uniform highp float mrg;
            uniform highp float radius;
            uniform highp float inset;
            uniform highp vec2 iconAspect;
            uniform mediump vec4 plateC;
            uniform mediump vec4 accentC;
            uniform mediump float f;
            uniform mediump float glowA;
            uniform lowp float qt_Opacity;
            varying highp vec2 qt_TexCoord0;
            highp float sd(highp vec2 p, highp vec2 hs, highp float r) {
                highp vec2 q = abs(p - hs) - (hs - vec2(r));
                return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
            }
            void main() {
                highp float T = dims.x - 2.0 * mrg;
                highp vec2 p = qt_TexCoord0 * dims - vec2(mrg);
                highp vec2 hs = vec2(T * 0.5);
                highp float d = sd(p, hs, radius);
                mediump float sA = mix(0.28, 0.6, f), sY = mix(6.0, 26.0, f), sB = mix(9.0, 34.0, f);
                mediump float sh = sA * (1.0 - smoothstep(-sB, sB, sd(p - vec2(0.0, sY), hs, radius)));
                if (d > 0.75) {
                    mediump float g = glowA * f * (1.0 - smoothstep(-36.0, 64.0, d));
                    mediump float ao = g + sh - g * sh;
                    gl_FragColor = vec4(accentC.rgb * g, ao) * qt_Opacity;
                    return;
                }
                mediump float inside = 1.0 - smoothstep(-0.75, 0.75, d);
                highp vec2 uv = (p - vec2(inset)) / (T - 2.0 * inset);
                highp vec2 uvf = (uv - 0.5) * iconAspect + 0.5;
                highp float inBox = step(0.0, uvf.x) * step(uvf.x, 1.0) * step(0.0, uvf.y) * step(uvf.y, 1.0);
                highp vec4 ic = texture2D(icon, clamp(uvf, 0.0, 1.0)) * inBox;
                highp vec3 face = ic.rgb + plateC.rgb * (1.0 - ic.a);
                highp float t = (0.342 * p.x + 0.940 * p.y) / (1.282 * T);
                face = mix(face, vec3(1.0), 0.18 * f * (1.0 - smoothstep(0.0, 0.55, t)));
                highp float band = smoothstep(-1.75, -1.0, d);
                face = mix(face, vec3(1.0), band * mix(0.06, 0.22, f));
                highp float top = inside * (1.0 - smoothstep(0.5, 1.5, p.y)) * smoothstep(radius * 0.6, radius, min(p.x, T - p.x));
                face = mix(face, vec3(1.0), top * 0.42 * f);
                mediump float a = inside + sh * (1.0 - inside);
                gl_FragColor = vec4(face * inside, a) * qt_Opacity;
            }"
    }

    // Name pill 10px under the focused plate. The mock scales it with the tile (22px * 1.12);
    // here it is drawn unscaled at the resulting size so the text stays on the pixel grid.
    property bool labelSettled: false
    Timer { id: labelTimer; interval: tile.labelMs; running: tile.focused; onTriggered: tile.labelSettled = true }
    onFocusedChanged: if (!focused) labelSettled = false

    Item {
        id: labelPill
        x: Math.round((tile.size - width) / 2)
        y: tile.labelAbove ? Math.round(-(tile.size * (tile.focusScale - 1)) / 2 - 4 - 10 - height)
                           : Math.round(tile.size + (tile.size * (tile.focusScale - 1)) / 2 - 4 + 10)
        width: pillText.implicitWidth + 26
        height: 43
        opacity: tile.labelSettled ? 1 : 0
        visible: opacity > 0.01
        Behavior on opacity { NumberAnimation { duration: tile.labelMs; easing.type: tile.focusEasing } }
        Rectangle {
            anchors.fill: parent
            anchors.topMargin: 6
            radius: 10
            color: "#000000"
            opacity: 0.35
            antialiasing: true
        }
        Rectangle {
            anchors.fill: parent
            radius: 10
            color: tile.isLight ? "#e6ffffff" : "#db120e1a"
            border.width: 1
            border.color: tile.isLight ? "#ffffff" : "#21ffffff"
            antialiasing: true
        }
        Text {
            id: pillText
            x: 13
            y: Math.round((parent.height - implicitHeight) / 2)
            text: tile.launchPoint.title || ""
            color: tile.ink
            font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 25
            renderType: Text.NativeRendering
        }
    }

    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        onEntered: tile.hoverFocus()
        onClicked: if (!holdFired) tile.activated()
        property bool holdFired: false
        onPressed: holdFired = false
        onPressAndHold: { holdFired = true; tile.holdActivated() }
        pressAndHoldInterval: 600
    }
}
