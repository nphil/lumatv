import QtQuick 2.4

// Card material for widgets, the dock, the status bar and popovers: one quad, one shader.
// The fragment samples the shared blurred-wallpaper texture at the card's own screen rect
// (the mock's backdrop-filter), applies the tint, the 1px inset edge, the focus sheen and
// rim, and draws the drop shadow from the same signed distance, so a card costs one draw
// call and no framebuffers, and its corners stay antialiased at any scale. Sized to its
// parent plus `margin` on every side so the shadow has room. Outside the card only the
// shadow term runs (an early branch on a coherent region, cheap on Mali); colour math is
// mediump, coordinates highp.
ShaderEffect {
    id: glass

    property var mat: ({})
    property real cardRadius: 26
    property real focusMix: 0              // 0..1, animated by the owner
    property bool isLight: false
    // Card rect in screen space, unscaled: what the backdrop is sampled from.
    property real screenX: 0
    property real screenY: 0
    property real screenW: 100
    property real screenH: 100
    property Item backdrop: null           // ShaderEffectSource of the blurred wallpaper
    // Shadow reach is shadowY + shadowB (74px focused); the last few pixels of tail are
    // below 1/255 and can be clipped.
    property real margin: 60

    anchors.fill: parent
    anchors.margins: -margin

    property color glassC: mat.glass || "#66120E1A"
    property color glassFocusC: mat.glassFocus || "#8F2C263A"
    property color edgeC: mat.glassEdge || "#21FFFFFF"
    property color edgeFocusC: mat.glassFocusEdge || "#33FFFFFF"
    function c4(c) { return Qt.vector4d(c.r, c.g, c.b, c.a) }

    // straight-alpha colours; the shader premultiplies its own output
    property vector4d tint: mix4(c4(glassC), c4(glassFocusC), focusMix)
    property real edgeFocus: 0             // edge-only focus (the dock's "active" state)
    property vector4d edge: mix4(c4(edgeC), c4(edgeFocusC), Math.max(focusMix, edgeFocus))
    property real rim: (isLight ? 1.0 : 0.42) * focusMix
    property real sheen: focusMix
    // CSS: normal 0 24px 60px .28 | focus 0 34px 80px .55 + 0 10px 24px .30 (light: .18/.28/.14)
    property vector4d shadow: Qt.vector4d(isLight ? 0.18 + 0.10 * focusMix : 0.28 + 0.27 * focusMix, 24 + 10 * focusMix, 30 + 10 * focusMix, 0)
    property vector4d shadow2: Qt.vector4d((isLight ? 0.14 : 0.30) * focusMix, 10, 12, 0)
    property vector4d region: Qt.vector4d(screenX / 1920, screenY / 1080, screenW / 1920, screenH / 1080)
    property vector2d dims: Qt.vector2d(width, height)
    property real radius: cardRadius
    property variant src: backdrop

    function mix4(a, b, t) {
        return Qt.vector4d(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, a.z + (b.z - a.z) * t, a.w + (b.w - a.w) * t)
    }

    fragmentShader: "
        uniform sampler2D src;
        uniform highp vec4 region;
        uniform highp vec2 dims;
        uniform highp float margin;
        uniform highp float radius;
        uniform mediump vec4 tint;
        uniform mediump vec4 edge;
        uniform mediump float rim;
        uniform mediump float sheen;
        uniform mediump vec4 shadow;    // alpha, y offset, blur, -
        uniform mediump vec4 shadow2;
        uniform lowp float qt_Opacity;
        varying highp vec2 qt_TexCoord0;
        highp float sd(highp vec2 p, highp vec2 hs, highp float r) {
            highp vec2 q = abs(p - hs) - (hs - vec2(r));
            return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
        }
        void main() {
            highp vec2 cs = dims - 2.0 * margin;
            highp vec2 p = qt_TexCoord0 * dims - vec2(margin);
            highp vec2 hs = cs * 0.5;
            highp float d = sd(p, hs, radius);
            if (d > 0.75) {
                mediump float s1 = shadow.x * (1.0 - smoothstep(-shadow.z, shadow.z, sd(p - vec2(0.0, shadow.y), hs, radius)));
                mediump float s2 = shadow2.x * (1.0 - smoothstep(-shadow2.z, shadow2.z, sd(p - vec2(0.0, shadow2.y), hs, radius)));
                mediump float sh = s1 + s2 - s1 * s2;
                gl_FragColor = vec4(0.0, 0.0, 0.0, sh) * qt_Opacity;
                return;
            }
            mediump float inside = 1.0 - smoothstep(-0.75, 0.75, d);
            highp vec2 uv = region.xy + clamp(p / cs, 0.0, 1.0) * region.zw;
            mediump vec3 col = mix(texture2D(src, uv).rgb, tint.rgb, tint.a);
            if (sheen > 0.001) {
                mediump float t = (p.x / cs.x + p.y / cs.y) * 0.5;
                mediump float g = mix(0.16, 0.05, smoothstep(0.0, 0.38, t)) * (1.0 - smoothstep(0.38, 0.60, t));
                col = mix(col, vec3(1.0), g * sheen);
                mediump float top = inside * (1.0 - smoothstep(0.5, 1.5, p.y)) * smoothstep(radius * 0.6, radius, min(p.x, cs.x - p.x));
                col = mix(col, vec3(1.0), top * rim);
            }
            mediump float band = smoothstep(-1.75, -1.0, d);
            col = mix(col, edge.rgb, band * edge.a);
            // the edge pixel band also carries the start of the shadow so the outline never gaps
            mediump float s1 = shadow.x * (1.0 - smoothstep(-shadow.z, shadow.z, sd(p - vec2(0.0, shadow.y), hs, radius)));
            mediump float a = inside + s1 * (1.0 - inside);
            gl_FragColor = vec4(col * inside, a) * qt_Opacity;
        }"
}
