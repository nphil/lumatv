import QtQuick 2.4
import "../kit"

// What the screen shows while something is on its way (docs/mocks/15-launch-splash, 15-spinner, 15-long-press-ring):
//   launch    the app's icon and name on a dim wallpaper with "Opening", from the moment OK is released on a tile
//             until the app has a window (the host calls `launch(name, iconUrl)` and `done()`)
//   spinner   a ring and a word (Loading) over whatever is behind, for a wait that has no app to name
//   hold      the ring that fills around a tile while OK is held, "Keep holding for options" under it (the
//             700 ms `motion.holdMs` of the dock's pick-up), so a long press is something you can see coming
// All three are one item that is built once and only changes what it shows; the ring is one rotating arc drawn from
// two quads (a track and a lit piece cut by a clip), not a Canvas, so it costs a transform per frame.
//
// `fraction` (0..1) drives the hold ring from the host's own hold timer; the spinner and the launch ring turn on
// their own clock and stop when hidden, so a hidden splash costs nothing.
Item {
    id: splash
    anchors.fill: parent
    property string mode: ""                   // "", "launch", "spinner", "hold"
    property string name: ""
    property string iconUrl: ""
    property string label: ""
    property real fraction: 0                  // hold: how much of the hold has passed
    property rect anchorRect: Qt.rect(0, 0, 0, 0)   // hold: the tile the ring surrounds
    readonly property bool built: true
    readonly property bool active: mode !== ""

    function launch(appName, icon) { name = appName; iconUrl = icon || ""; label = "Opening"; mode = "launch" }
    function spin(text) { label = text || "Loading"; mode = "spinner" }
    function hold(rectOfTile, appName) { anchorRect = Qt.rect(rectOfTile.x, rectOfTile.y, rectOfTile.width, rectOfTile.height); name = appName || ""; label = "Keep holding for options"; mode = "hold" }
    function done() { mode = "" }
    visible: active

    // The launch screen is the blurred wallpaper with the tint over it and nothing else, so Home is not seen to be
    // still there while the app starts (a full-size glass pane, no rounded corners).
    LuPane { anchors.fill: parent; visible: splash.mode === "launch"; cardRadius: 0 }

    // ---- launch: the icon, the name, "Opening" with a small ring
    Column {
        visible: splash.mode === "launch"
        anchors.centerIn: parent
        spacing: Lu.space(4)
        Rectangle {
            anchors.horizontalCenter: parent.horizontalCenter
            width: 160; height: 160; radius: 36; color: Lu.wash("hover"); border.width: 1; border.color: Lu.edge; antialiasing: true
            Image {
                anchors.centerIn: parent; width: 128; height: 128
                source: splash.iconUrl; fillMode: Image.PreserveAspectFit; asynchronous: false; cache: true; smooth: true
                visible: source != ""
            }
            LuIcon { anchors.centerIn: parent; visible: splash.iconUrl === ""; name: "layout-grid"; size: 64; color: Lu.ink2 }
        }
        LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "title"; text: splash.name }
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: Lu.space(3)
            LuRing { anchors.verticalCenter: parent.verticalCenter; size: 26; thick: 3; running: splash.mode === "launch" }
            LuText { anchors.verticalCenter: parent.verticalCenter; role: "label"; color: Lu.ink2; text: splash.label }
        }
    }

    // ---- spinner
    Column {
        visible: splash.mode === "spinner"
        x: Math.round((parent.width - width) / 2); y: 340
        spacing: Lu.space(3)
        LuRing { anchors.horizontalCenter: parent.horizontalCenter; size: 64; thick: 4; running: splash.mode === "spinner" }
        LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "caption"; color: Lu.ink2; text: splash.label }
    }

    // ---- hold ring around a tile
    Item {
        visible: splash.mode === "hold"
        x: splash.anchorRect.x - 10; y: splash.anchorRect.y - 10
        width: splash.anchorRect.width + 20; height: splash.anchorRect.height + 20
        LuRing { width: parent.width; height: parent.height; running: false; progress: splash.fraction; thick: 5; cornerRadius: 34 }
        Rectangle {
            x: Math.round((parent.width - width) / 2); y: parent.height + Lu.space(3)
            width: holdText.implicitWidth + 2 * Lu.space(5); height: Math.round(Lu.target * 0.7); radius: height / 2
            color: Lu.glass; border.width: 1; border.color: Lu.edge
            LuText { id: holdText; anchors.centerIn: parent; role: "caption"; text: splash.label }
        }
    }
}
