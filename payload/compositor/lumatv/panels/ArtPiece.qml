import QtQuick 2.4
import "../kit"

// One print of the art mode (panels/Screensaver.qml) in its matte and its frame, centred on the screen: the print at the size it has
// when it fits 1040 by 700, a matte of 64 px around it and a frame of 18 px around that (docs/mocks/21-screensaver-art). The colours
// are the artwork's presentation, a cream card in a dark frame as a gallery hangs a print, not the interface's palette, so they are
// not tokens of the theme.
//
// `put(meta)` takes one picture of art/index.json and starts loading it in the background (`ready` when it is decoded, `broken`
// when it could not be); `clear()` lets go of it. The print is decoded at the size it is shown at, never larger.
Item {
    id: piece
    anchors.fill: parent
    property var meta: null
    readonly property bool ready: meta !== null && img.status === Image.Ready
    readonly property bool broken: meta !== null && img.status === Image.Error
    readonly property int maxW: 1040
    readonly property int maxH: 700
    readonly property int matte: 64
    readonly property int frame: 18
    readonly property real fit: meta ? Math.min(maxW / meta.width, maxH / meta.height) : 1
    readonly property int pw: meta ? Math.round(meta.width * fit) : 0
    readonly property int ph: meta ? Math.round(meta.height * fit) : 0

    function put(m) { meta = m }
    function clear() { meta = null }

    visible: meta !== null && opacity > 0
    Rectangle {
        anchors.centerIn: parent
        width: piece.pw + 2 * (piece.matte + piece.frame)
        height: piece.ph + 2 * (piece.matte + piece.frame)
        color: "#1F1A1B"
        Rectangle {
            anchors.centerIn: parent
            width: piece.pw + 2 * piece.matte
            height: piece.ph + 2 * piece.matte
            color: "#E9E2D3"
            Image {
                id: img
                anchors.centerIn: parent
                width: piece.pw
                height: piece.ph
                source: piece.meta ? Lu.baseUrl + "art/" + piece.meta.file : ""
                sourceSize.width: piece.pw
                sourceSize.height: piece.ph
                asynchronous: true
                cache: false
                smooth: true
            }
        }
    }
}
