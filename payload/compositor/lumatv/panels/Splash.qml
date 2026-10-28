import QtQuick 2.4
import "../kit"

// What the screen shows while something is on its way (docs/mocks/15-launch-splash, 15-spinner, 15-long-press-ring):
//   launch    the app's own picture, large, and its name on the dimmed wallpaper with "Opening", from the moment OK is released on a
//             tile until the app has a window (the host calls `launch(name, iconUrl)` and `done()`)
//   spinner   a ring over the dimmed frame for a wait that has no app to name; the word (Loading) comes after a second, in a dark pill
//             so it reads over any picture
//   hold      the ring that fills around a tile while OK is held, "Keep holding for options" under it (the
//             700 ms `motion.holdMs` of the dock's pick-up), so a long press is something you can see coming
// All three are one item that is built once and only changes what it shows; the ring is one rotating arc drawn from
// one small shader (kit/LuRing), not a Canvas, so it costs a transform per frame. The mock's soft shadow under the picture is left out:
// a blurred shadow is a live effect, which the frame gate forbids.
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
    property bool wordShown: false             // spinner: the word waits a second, so a short wait shows no flash of text
    readonly property bool built: true
    readonly property bool active: mode !== ""
    // How long the launch screen takes to fade in when it comes after a wait (the host sets it: a splash that is a cut is what the transitions
    // replace), and where its icon is, for the card that grows out of it.
    property int fadeMs: 0
    property real shown: 1
    function iconRect() { var p = launchIcon.mapToItem(null, 0, 0); return Qt.rect(p.x, p.y, launchIcon.width, launchIcon.height) }
    readonly property real iconRadius: launchIcon.radius
    opacity: mode === "launch" ? shown : 1
    onModeChanged: {
        if (mode === "launch" && fadeMs > 0) { shown = 0; fadeIn.restart() }
        else { fadeIn.stop(); shown = 1 }
    }
    NumberAnimation { id: fadeIn; target: splash; property: "shown"; from: 0; to: 1; duration: splash.fadeMs; easing.type: Easing.OutCubic }
    // the dark pill behind the words that sit over a picture: the sheet's dark at 72 %, so ink on it reads over a white frame too
    readonly property color pill: Qt.rgba(Lu.scrimHide.r, Lu.scrimHide.g, Lu.scrimHide.b, 0.72)

    function launch(appName, icon) { name = appName; iconUrl = icon || ""; label = "Opening"; mode = "launch" }
    function spin(text) { label = text || "Loading"; wordShown = false; mode = "spinner" }
    function hold(rectOfTile, appName) { anchorRect = Qt.rect(rectOfTile.x, rectOfTile.y, rectOfTile.width, rectOfTile.height); name = appName || ""; label = "Keep holding for options"; mode = "hold" }
    function done() { mode = "" }
    visible: active

    Timer {
        interval: 1000
        running: splash.mode === "spinner" && !splash.wordShown
        onTriggered: splash.wordShown = true
    }

    // The launch screen is the blurred wallpaper dimmed by the scrim and nothing else, so Home is not seen to be
    // still there while the app starts (a full-size glass pane, no rounded corners).
    LuPane { anchors.fill: parent; visible: splash.mode === "launch"; cardRadius: 0 }
    Rectangle { anchors.fill: parent; visible: splash.mode === "launch" || splash.mode === "spinner"; color: Lu.scrim }

    // ---- launch: the picture, the name, "Opening" with a small ring
    Column {
        visible: splash.mode === "launch"
        anchors.centerIn: parent
        spacing: Lu.space(6)
        LuAppIcon { id: launchIcon; anchors.horizontalCenter: parent.horizontalCenter; width: 180; height: 180; radius: 38; source: splash.iconUrl }
        LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "title"; text: splash.name }
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: Lu.space(3)
            LuRing { anchors.verticalCenter: parent.verticalCenter; size: 28; thick: 3; tint: Lu.ink; trackColor: Lu.ink3; running: splash.mode === "launch" }
            LuText { anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink2; text: splash.label }
        }
    }

    // ---- spinner: centred on the screen, the ring first and the word a second later
    Column {
        visible: splash.mode === "spinner"
        anchors.centerIn: parent
        spacing: Lu.space(4)
        LuRing { anchors.horizontalCenter: parent.horizontalCenter; size: 56; thick: 5; tint: Lu.ink; trackColor: Lu.ink3; running: splash.mode === "spinner" }
        Rectangle {
            anchors.horizontalCenter: parent.horizontalCenter
            opacity: splash.wordShown ? 1 : 0
            width: spinWord.implicitWidth + 2 * Lu.space(5)
            height: Math.round(Lu.target * 0.7)
            radius: height / 2
            color: splash.pill
            LuText { id: spinWord; anchors.centerIn: parent; role: "caption"; text: splash.label }
        }
    }

    // ---- hold ring around a tile
    Item {
        visible: splash.mode === "hold"
        x: splash.anchorRect.x - 10; y: splash.anchorRect.y - 10
        width: splash.anchorRect.width + 20; height: splash.anchorRect.height + 20
        LuRing { width: parent.width; height: parent.height; running: false; progress: splash.fraction; thick: 6; cornerRadius: 34; trackColor: Qt.rgba(Lu.ink.r, Lu.ink.g, Lu.ink.b, 0.18) }
        Rectangle {
            x: Math.round((parent.width - width) / 2); y: parent.height + Lu.space(4)
            width: holdText.implicitWidth + 2 * Lu.space(5); height: Math.round(Lu.target * 0.7); radius: height / 2
            color: splash.pill
            LuText { id: holdText; anchors.centerIn: parent; role: "caption"; text: splash.label }
        }
    }
}
