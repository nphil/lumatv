import QtQuick 2.4
import "."

// A camera's picture, kept fresh from the helper (`GET /v1/cameras/{id}/frame?w=&h=`): the helper fetches the camera's current picture
// from Home Assistant's signed address when asked and decodes it off the CPU where the TV can, so what arrives here is a picture of about
// the size wanted, and this item asks for the next one `intervalMs` after the last arrived. Two hidden Images take turns loading (never the
// one on screen), and one quad shows the newest through rounded corners, the two mixed by one uniform: a picture never flashes or tears
// in while it loads, and a change of picture costs no layer and no batch beyond this quad (the pattern of NewsWidget's hero).
//
// Nothing is asked for while `active` is false: a hidden pop-up, a tile that scrolled away or a closed viewer costs nothing, here or in the helper.
// `cover` fills the box (a tile, the pop-up's preview); off, the whole picture fits inside it (the full screen) with black beside it.
Item {
    id: frame
    property string camera: ""
    property bool active: false
    property int intervalMs: 700
    // the size asked of the helper: the box's own, rounded up to the step the decoder scales by
    property int wantW: Math.max(16, Math.ceil(width))
    property int wantH: Math.max(16, Math.ceil(height))
    property real radius: 0
    property bool cover: true
    property string base: Helper.base
    // a picture of this camera is on screen; `frames` counts the ones that arrived; `failed` the fetches that did not
    readonly property bool ready: shown.status === Image.Ready
    property int frames: 0
    property int failed: 0
    property int n: 0
    property bool frontIsA: true
    property real mixv: 0
    readonly property Image shown: frontIsA ? imgA : imgB
    readonly property Image back: frontIsA ? imgB : imgA
    readonly property bool loading: back.status === Image.Loading

    function url() {
        n += 1
        return base + "/v1/cameras/" + camera + "/frame?w=" + wantW + "&h=" + wantH + "&n=" + n
    }
    function fetchNext() {
        if (!active || camera === "" || loading) return
        back.source = url()
    }
    function reset() {
        imgA.source = ""
        imgB.source = ""
        frontIsA = true
        mixv = 0
        frames = 0
        failed = 0
    }
    onCameraChanged: { reset(); if (active) fetchNext() }
    onActiveChanged: { if (active) fetchNext(); else { imgA.source = ""; imgB.source = ""; frames = 0 } }
    function arrived(img) {
        if (img.status === Image.Error) { failed += 1; again.restart(); return }
        if (img.status !== Image.Ready) return
        if (img === back) {
            frontIsA = !frontIsA
            // the swap: a short mix when motion is on, at once when it is reduced; the next ask follows the arrival, never a fixed clock
            if (Lu.reduced) mixv = frontIsA ? 0 : 1
            else swap.restart()
        }
        frames += 1
        again.restart()
    }
    Timer { id: again; interval: frame.intervalMs; onTriggered: frame.fetchNext() }
    NumberAnimation { id: swap; target: frame; property: "mixv"; to: frame.frontIsA ? 0 : 1; duration: 90 }

    Image {
        id: imgA
        visible: false
        asynchronous: true; cache: false
        sourceSize.width: frame.wantW
        sourceSize.height: frame.wantH
        onStatusChanged: frame.arrived(imgA)
    }
    Image {
        id: imgB
        visible: false
        asynchronous: true; cache: false
        sourceSize.width: frame.wantW
        sourceSize.height: frame.wantH
        onStatusChanged: frame.arrived(imgB)
    }
    // black until the first picture, and beside a fitted one
    Rectangle { anchors.fill: parent; radius: frame.radius; color: "#000000"; antialiasing: frame.radius > 0 }
    ShaderEffect {
        anchors.fill: parent
        visible: imgA.status === Image.Ready || imgB.status === Image.Ready
        property variant srcA: imgA
        property variant srcB: imgB
        property real mixv: frame.mixv
        property vector2d dims: Qt.vector2d(width, height)
        property real radius: frame.radius
        function scaleFor(img) {
            var ia = img.implicitHeight > 0 ? img.implicitWidth / img.implicitHeight : 16 / 9
            var ba = height > 0 ? width / height : 16 / 9
            if (frame.cover) return ia > ba ? Qt.vector2d(ba / ia, 1) : Qt.vector2d(1, ia / ba)
            return ia > ba ? Qt.vector2d(1, ia / ba) : Qt.vector2d(ba / ia, 1)
        }
        property vector2d scaleA: scaleFor(imgA)
        property vector2d scaleB: scaleFor(imgB)
        fragmentShader: "
            uniform sampler2D srcA;
            uniform sampler2D srcB;
            uniform mediump float mixv;
            uniform highp vec2 dims;
            uniform highp vec2 scaleA;
            uniform highp vec2 scaleB;
            uniform highp float radius;
            uniform lowp float qt_Opacity;
            varying highp vec2 qt_TexCoord0;
            lowp vec4 pick(sampler2D src, highp vec2 scale) {
                highp vec2 uv = (qt_TexCoord0 - 0.5) * scale + 0.5;
                lowp float inside = step(0.0, uv.x) * step(uv.x, 1.0) * step(0.0, uv.y) * step(uv.y, 1.0);
                return texture2D(src, clamp(uv, 0.0, 1.0)) * inside;
            }
            void main() {
                highp vec2 p = qt_TexCoord0 * dims;
                highp vec2 hs = dims * 0.5;
                highp vec2 q = abs(p - hs) - (hs - vec2(radius));
                highp float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
                mediump float a = 1.0 - smoothstep(-0.75, 0.75, d);
                gl_FragColor = mix(pick(srcA, scaleA), pick(srcB, scaleB), mixv) * a * qt_Opacity;
            }"
    }
}
