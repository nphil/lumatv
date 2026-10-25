import QtQuick 2.4

// The mini-app window: one Glass quad that grows out of the card that opened it (x, y,
// width, height ease from the card's rect to the frame rect), a title bar, a spinner while
// the content is on its way, and an interior rect that either hosts a QML component or is
// left clear for the overlay web app ({{appId}}.frame) to paint into. Closing
// runs the same path backwards. Everything animated is a transform, an opacity or the
// quad's geometry; text is native and only shown once the frame has landed.
Item {
    id: frame

    property var mat: ({})
    property bool isLight: false
    property real cardRadius: 26
    property Item backdrop: null
    property int motionMs: 260
    property int layerMs: 220

    property bool open: false
    property rect fromRect: Qt.rect(0, 0, 0, 0)
    property rect toRect: Qt.rect(200, 150, 1520, 860)
    property string title: ""
    property string subtitle: ""
    property bool loading: false
    property bool landed: false
    readonly property int barHeight: 64
    readonly property int pad: 32
    // interior the content (QML or the overlay app) fills, in screen coordinates
    readonly property rect interior: Qt.rect(toRect.x + pad, toRect.y + barHeight, toRect.width - 2 * pad, toRect.height - barHeight - pad)

    visible: open || opacity > 0.01
    opacity: open ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: frame.layerMs; easing.type: Easing.OutCubic } }

    x: open ? toRect.x : fromRect.x
    y: open ? toRect.y : fromRect.y
    width: open ? toRect.width : fromRect.width
    height: open ? toRect.height : fromRect.height
    Behavior on x { NumberAnimation { duration: frame.motionMs; easing.type: Easing.OutCubic } }
    Behavior on y { NumberAnimation { duration: frame.motionMs; easing.type: Easing.OutCubic } }
    Behavior on width { NumberAnimation { duration: frame.motionMs; easing.type: Easing.OutCubic } }
    Behavior on height { NumberAnimation { id: heightAnim; duration: frame.motionMs; easing.type: Easing.OutCubic; onRunningChanged: if (!running && frame.open) frame.landed = true } }
    onOpenChanged: if (!open) landed = false

    Glass {
        mat: frame.mat
        isLight: frame.isLight
        cardRadius: frame.cardRadius
        focusMix: 1
        screenX: frame.x; screenY: frame.y; screenW: frame.width; screenH: frame.height
        backdrop: frame.backdrop
    }

    // title bar and content only once the geometry has settled: native text is never scaled
    Item {
        anchors.fill: parent
        opacity: frame.landed ? 1 : 0
        visible: opacity > 0.01
        Behavior on opacity { NumberAnimation { duration: 160 } }
        Text {
            x: frame.pad; y: Math.round((frame.barHeight - implicitHeight) / 2)
            width: parent.width - 2 * frame.pad - subtitleText.implicitWidth - 24
            elide: Text.ElideRight
            text: frame.title
            color: frame.mat.ink || "#F2EFF6"
            font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 26
            renderType: Text.NativeRendering
        }
        Text {
            id: subtitleText
            x: parent.width - frame.pad - implicitWidth; y: Math.round((frame.barHeight - implicitHeight) / 2)
            text: frame.subtitle
            color: frame.mat.ink2 || "#A8F2EFF6"
            font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 22
            renderType: Text.NativeRendering
        }
        Rectangle {
            x: frame.pad; y: frame.barHeight - 1
            width: parent.width - 2 * frame.pad; height: 1
            color: frame.isLight ? "#1f17131f" : "#1affffff"
        }

        // spinner: a 270-degree arc painted once, spun by a rotation transform
        Item {
            id: spinner
            x: Math.round((parent.width - 64) / 2); y: Math.round(frame.barHeight + (parent.height - frame.barHeight - 64) / 2)
            width: 64; height: 64
            opacity: frame.loading ? 1 : 0
            visible: opacity > 0.01
            Behavior on opacity { NumberAnimation { duration: 180 } }
            Canvas {
                id: arc
                anchors.fill: parent
                property color ink: frame.mat.ink || "#F2EFF6"
                onInkChanged: requestPaint()
                onPaint: {
                    var ctx = getContext("2d"); ctx.reset()
                    ctx.lineWidth = 4; ctx.lineCap = "round"
                    ctx.strokeStyle = Qt.rgba(ink.r, ink.g, ink.b, 0.16)
                    ctx.beginPath(); ctx.arc(32, 32, 28, 0, Math.PI * 2); ctx.stroke()
                    ctx.strokeStyle = ink
                    ctx.beginPath(); ctx.arc(32, 32, 28, -Math.PI / 2, Math.PI * 0.9); ctx.stroke()
                }
                RotationAnimation on rotation { from: 0; to: 360; duration: 900; loops: Animation.Infinite; running: spinner.visible }
            }
        }

        // QML content slot (future mini apps); the web kind leaves this empty
        Item {
            id: contentSlot
            x: frame.pad; y: frame.barHeight
            width: parent.width - 2 * frame.pad; height: parent.height - frame.barHeight - frame.pad
            clip: true
        }
    }
    property alias contentSlot: contentSlot
}
