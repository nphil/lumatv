import QtQuick 2.4

// A picture with rounded corners and a reading scrim under its foot, drawn in one pass (tools/voice_patch.py lays this file into LG's Search
// tree; LG's own genre cards are square pictures with their words over them). The corners are a rounded-rectangle distance field in the
// fragment shader and the scrim is a ramp mixed into the picture before it is written, so the card is one quad: no mask, no second item
// and no offscreen target (a rounded Image cannot be cut with `clip`, and OpacityMask is a layer per card).
//
// `picture` is an Image that is not drawn itself (`visible: false`): the effect samples its texture directly, which is how kit/LuIcon.qml
// draws the icons of the compositor. The picture is cropped to fill the card from its centre, as `Image.PreserveAspectCrop` does.
// Until the picture is ready the item is invisible and whatever plate is under it shows.
ShaderEffect {
    id: photo

    property Item picture: null
    property real cornerRadius: 22
    property color scrimColor: "#120E1A"     // the sheet: the words are ink over this
    property real scrimStrength: 0.9         // how much of the sheet the foot of the picture is
    property real scrimFrom: 0.15            // where, from the top as a fraction of the height, the scrim begins
    property real scrimTo: 0.7               // and where it is at full strength

    visible: picture !== null && picture.status === Image.Ready && width > 0 && height > 0

    property variant src: picture
    property vector4d geom: Qt.vector4d(width, height, cornerRadius, 0)
    // The part of the picture that is seen, as a scale about its centre: the shorter axis shows all, the longer is cut.
    readonly property real ratioPlate: width / Math.max(1, height)
    readonly property real ratioPicture: picture !== null ? picture.implicitWidth / Math.max(1, picture.implicitHeight) : 1
    property vector4d crop: Qt.vector4d(ratioPicture > ratioPlate ? ratioPlate / ratioPicture : 1,
                                        ratioPicture > ratioPlate ? 1 : ratioPicture / ratioPlate, 0, 0)
    property vector4d scrim: Qt.vector4d(scrimColor.r, scrimColor.g, scrimColor.b, scrimStrength)
    property vector4d ramp: Qt.vector4d(scrimFrom, scrimTo, 0, 0)

    fragmentShader: "
        uniform sampler2D src;
        uniform highp vec4 geom;
        uniform highp vec4 crop;
        uniform highp vec4 scrim;
        uniform highp vec4 ramp;
        uniform lowp float qt_Opacity;
        varying highp vec2 qt_TexCoord0;
        highp float sd(highp vec2 p, highp vec2 hs, highp float r) {
            highp vec2 q = abs(p) - (hs - vec2(r));
            return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
        }
        void main() {
            highp vec2 px = (qt_TexCoord0 - 0.5) * geom.xy;
            highp float inside = 1.0 - smoothstep(-0.75, 0.75, sd(px, geom.xy * 0.5, geom.z));
            highp vec2 uv = (qt_TexCoord0 - 0.5) * crop.xy + 0.5;
            lowp vec3 pic = texture2D(src, uv).rgb;
            highp float s = smoothstep(ramp.x, ramp.y, qt_TexCoord0.y) * scrim.a;
            lowp vec3 rgb = mix(pic, scrim.rgb, s);
            gl_FragColor = vec4(rgb, 1.0) * (inside * qt_Opacity);
        }"
}
