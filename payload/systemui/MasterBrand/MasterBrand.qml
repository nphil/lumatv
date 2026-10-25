import QtQuick 2.12
import SystemUIComponent 1.0
import TextFile 1.0

// The power-off logo, in place of LG's MasterBrand (the red LG logo, about 3 s). Same contract as
// LG's: the window is launched by the compositor's system UI manager at power-off,
// `responseOffLogoRequest(true)` tells tvpowerd the logo is done and the TV may go dark, and a
// launch with reason "preload" only hides the window.
//
// The one thing that must hold: tvpowerd waits for that answer, so it is sent exactly once, whatever
// happens. When the logo finishes it is sent; when the logo cannot be loaded it is sent at once;
// when the animation stalls it is sent after `giveUpMs`. The logo itself is {{dataDir}}/qml/
// WebOSCompositor/{{qmlModule}}/logo/ (theme.json `logo.off`: 1, 2 or 3).
StarfishSystemUIWindow {
    id: root
    objectName: "appRoot"

    property bool isDevice: true
    property real screenWidth: parent.width
    property real screenHeight: parent.height
    property int giveUpMs: 2500
    property bool answered: false

    width: screenWidth
    height: screenHeight

    readonly property string logoDir: "file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/logo/"
    readonly property var files: ["LogoOff.qml", "LogoOff2.qml", "LogoOff3.qml"]

    TextFile { id: themeFile; name: "file://{{dataDir}}/theme/theme.json" }
    // The window's own record, as in BootLogo.qml: one stamped line per step in {{dataDir}}/logo-events (monotonic clock from
    // /proc/uptime and wall clock), written after tvpowerd has its answer and inside a try, so it can never hold the power-off up.
    TextFile { id: eventsFile; name: "file://{{dataDir}}/logo-events" }
    TextFile { id: uptimeFile; name: "file:///proc/uptime" }
    function note(text) {
        try {
            var up = (uptimeFile.read() || "").split(" ")[0] || "-"
            var lines = (eventsFile.exists ? eventsFile.read() : "").split("\n").filter(function (l) { return l !== "" })
            lines.push("up=" + up + " wall=" + new Date().toISOString() + " " + text)
            eventsFile.write(lines.slice(-60).join("\n") + "\n")
        } catch (e) {}
    }
    function choice() {
        var n = 1
        try { n = Math.round(Number(JSON.parse(themeFile.exists ? themeFile.read() : "{}").logo.off)) } catch (e) { n = 1 }
        return n >= 1 && n <= files.length ? n : 1
    }

    function answer() {
        if (answered) return
        answered = true
        giveUp.stop()
        if (isDevice) root.responseOffLogoRequest(true)
        root.note("power-off answered tvpowerd")
    }

    Connections {
        target: root.lifeCycleStatus
        onLaunch: root.launch(payload)
        onClose: root.close(payload)
    }

    function launch(payload) {
        if (payload && payload.reason === "preload") { root.hide(); root.note("power-off preload (hidden)"); return }
        answered = false
        root.show()
        giveUp.restart()
        loader.source = ""
        var n = choice()
        console.info("[Logo] power-off logo " + n)
        root.note("power-off logo " + n + " launch reason=" + ((payload && payload.reason) || "-"))
        loader.source = logoDir + files[n - 1]
    }
    function close(payload) {
        giveUp.stop()
        loader.source = ""
        root.note("power-off close")
        root.exit()
    }

    Rectangle { anchors.fill: parent; color: "#000000" }

    Loader {
        id: loader
        width: 1920
        height: 1080
        transformOrigin: Item.TopLeft
        scale: root.width / 1920
        asynchronous: false
        onStatusChanged: {
            if (status === Loader.Error) { console.warn("[Logo] power-off logo did not load: " + source); root.answer(); root.note("power-off logo did not load") }
            if (status === Loader.Ready) item.start()
        }
        Connections {
            target: loader.item
            ignoreUnknownSignals: true
            onFinished: root.answer()
        }
    }

    Timer {
        id: giveUp
        interval: root.giveUpMs
        onTriggered: { console.warn("[Logo] power-off logo did not finish in " + interval + " ms"); root.answer() }
    }
}
