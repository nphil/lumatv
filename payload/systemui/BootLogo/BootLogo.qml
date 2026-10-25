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
    // The window's own record, kept apart from the log: /var/log/messages rotates within minutes and the log keeps
    // nothing before ~12.7 s of a boot. One line per step, stamped with the monotonic clock (/proc/uptime: the wall
    // clock is not set for the first seconds of a boot) and the wall clock; the last 60 lines stay across boots.
    // tools/bootcap.sh and docs/research/notify/reboot_logo.py read it. Never allowed to hold up or break the logo.
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
        try { n = Math.round(Number(JSON.parse(themeFile.exists ? themeFile.read() : "{}").logo.on)) } catch (e) { n = 1 }
        return n >= 1 && n <= files.length ? n : 1
    }

    Connections {
        target: root.lifeCycleStatus
        onLaunch: root.launch(payload)
        onClose: {
            if (payload && payload.reason === "powerSuspend") { root.note("power-on close reason=powerSuspend"); root.finish("suspend"); return }
            root.note("power-on close reason=" + ((payload && payload.reason) || "-") + (root.timedOut ? " (already timed out)" : ""))
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
        var n = choice()
        console.info("[Logo] power-on logo " + n)
        root.note("power-on logo " + n + " launch type=" + (p.type || "-"))
        loader.source = logoDir + files[n - 1]
        limit.restart()
        if (p.type === "fadeOut") fadeOut()
    }
    function fadeOut() {
        if (leaving) return
        leaving = true
        limit.stop()
        fade.restart()
    }
    function finish(why) {
        limit.stop()
        fade.stop()
        loader.source = ""
        root.note("power-on exit (" + why + ")")
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
                if (status === Loader.Error) { console.warn("[Logo] power-on logo did not load: " + source); root.note("power-on logo did not load") }
                if (status === Loader.Ready) { item.start(); root.note("power-on logo ready") }
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
        onRunningChanged: if (!running && root.leaving) root.finish("faded")
    }

    Timer {
        id: limit
        interval: root.timeOutDuration
        onTriggered: { root.timedOut = true; root.finish("timeout") }
    }
}
