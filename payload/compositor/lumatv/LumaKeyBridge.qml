import QtQuick 2.4
import "LumaBus.js" as Bus

// Instantiated by the key filter JS (Qt.createComponent on this file's absolute URL) so the
// filter can reach the LumaBus.js library instance LumaHome registered with.
QtObject {
    id: bridge

    // The Home key is only watched: a short press goes down LG's chain untouched. Held for
    // 650 ms, it asks for LG's own Home, and the rest of that hold (repeats and the release)
    // ends here. The compositor may not run commands, so the request is a file the tool's
    // guard reads (tools/lumatv homestub).
    readonly property string requestUrl: "file:///tmp/{{shortId}}-escape"
    // Set by the filter: stops the stock handling of the same hold, which would open Screen
    // Zoom at 2.5 s.
    property var escapeHook: null
    property bool homeHeld: false
    property bool escaped: false

    property Timer holdTimer: Timer {
        interval: 650
        onTriggered: {
            bridge.escaped = true
            bridge.resetTimer.restart()
            bridge.request()
        }
    }
    // A release that never arrives must not swallow Home for good.
    property Timer resetTimer: Timer {
        interval: 4000
        onTriggered: { bridge.escaped = false; bridge.homeHeld = false }
    }

    function dispatch(key, pressed, autoRepeat, deviceId) { return Bus.dispatch(key, pressed, autoRepeat, deviceId) }

    // true: the event ends here. LG's chain replays a short press as a press and a release
    // straight away, which this takes for a hold that ends at once.
    function homeKey(pressed, autoRepeat) {
        if (pressed) {
            if (!autoRepeat && !homeHeld) { homeHeld = true; escaped = false; holdTimer.restart() }
            return escaped
        }
        var swallow = escaped
        homeHeld = false
        escaped = false
        holdTimer.stop()
        resetTimer.stop()
        return swallow
    }

    function request() {
        console.info("[LumaKeys] Home held, asking for LG's Home")
        var x = new XMLHttpRequest()
        try { x.open("PUT", requestUrl); x.send(String(Date.now())) }
        catch (e) { console.warn("[LumaKeys] escape request failed: " + e) }
        Bus.escape()
        if (escapeHook) {
            try { escapeHook() }
            catch (e2) { console.warn("[LumaKeys] escape hook failed: " + e2) }
        }
    }
}
