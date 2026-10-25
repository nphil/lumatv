import QtQuick 2.12
import SystemUIComponent 1.0
import TextFile 1.0

// The power-on logo, in place of LG's BootLogo (the "webOS" sprite). Same contract as LG's: the
// compositor's system UI manager launches the window with params.type ("gif", "sprite", "image",
// "sequence" or "fadeOut", all of which show the same logo here; "fadeOut" also starts the exit), a
// close fades the screen out over params.fadeOutDuration (default 1000 ms) and exits, a close with
// reason "powerSuspend" exits at once, and the window closes itself after 10 s at the latest.
// The logo is {{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/logo/ (theme.json `logo.on`: 1, 2 or 3);
// if it cannot be loaded the screen is plain black, as LG's is before its first frame.
StarfishSystemUIWindow {
    id: root
    objectName: "appRoot"

    property bool isDevice: true
    property real screenWidth: parent.width
    property real screenHeight: parent.height
    property int fadeOutDuration: 1000
    property int timeOutDuration: 10000
    property bool timedOut: false
    property bool leaving: false

    width: screenWidth
    height: screenHeight

    readonly property string logoDir: "file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/logo/"
    readonly property var files: ["LogoOn.qml", "LogoOn2.qml", "LogoOn3.qml"]

    TextFile { id: themeFile; name: "file://{{dataDir}}/theme/theme.json" }
    function choice() {
        var n = 1
        try { n = Math.round(Number(JSON.parse(themeFile.exists ? themeFile.read() : "{}").logo.on)) } catch (e) { n = 1 }
        return n >= 1 && n <= files.length ? n : 1
    }

    Connections {
        target: root.lifeCycleStatus
        onLaunch: root.launch(payload)
        onClose: {
            if (payload && payload.reason === "powerSuspend") { root.finish(); return }
            if (!root.timedOut) root.fadeOut()
        }
    }

    function launch(payload) {
        timedOut = false
        leaving = false
        scrim.opacity = 1
        var p = (payload && payload.params) || {}
        if (p.fadeOutDuration) fadeOutDuration = p.fadeOutDuration
        if (typeof webOSShell !== "undefined") webOSShell.setWindowProperty("delayCloseWindowTimeout", root.fadeOutDuration)
        root.show()
        loader.source = ""
        loader.source = logoDir + files[choice() - 1]
        limit.restart()
        if (p.type === "fadeOut") fadeOut()
    }
    function fadeOut() {
        if (leaving) return
        leaving = true
        limit.stop()
        fade.restart()
    }
    function finish() {
        limit.stop()
        fade.stop()
        loader.source = ""
        root.exit()
    }

    Rectangle { anchors.fill: parent; color: "#000000" }

    Item {
        id: scrim
        anchors.fill: parent
        Loader {
            id: loader
            width: 1920
            height: 1080
            transformOrigin: Item.TopLeft
            scale: root.width / 1920
            asynchronous: false
            onStatusChanged: {
                if (status === Loader.Error) console.warn("[Logo] power-on logo did not load: " + source)
                if (status === Loader.Ready) item.start()
            }
        }
    }

    NumberAnimation {
        id: fade
        target: scrim
        property: "opacity"
        to: 0
        duration: root.fadeOutDuration
        easing.type: Easing.InOutQuad
        onRunningChanged: if (!running && root.leaving) root.finish()
    }

    Timer {
        id: limit
        interval: root.timeOutDuration
        onTriggered: { root.timedOut = true; root.finish() }
    }
}
