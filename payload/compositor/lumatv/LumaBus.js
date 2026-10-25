.pragma library

// One engine-wide instance (pragma library, same absolute URL from both sides): LumaHome
// registers its key handler here, the configd-registered key filter (payload/keyfilter/
// lumatv.js) dispatches into it through LumaKeyBridge.qml. Keys never travel through
// QtQuick focus: the compositor's KeyFilter sends them straight to the focused Wayland
// window, so a QML item can only see them from inside the filter chain.
var handler = null
var escapeHandler = null
var lastFaultMs = 0
var beatMs = 0

function setHandler(h) { handler = h }
function setEscapeHandler(h) { escapeHandler = h }

// LumaHome's once-a-second timer calls this. The filter takes a key only while it has been
// called within the last 3 s: a layer whose timers have stopped is not answering, and the
// remote must not be trapped behind it (the tool's guard reads the same beat from a file
// and gives LG's Home back after about 6 s).
function beat() { beatMs = Date.now() }
function answering() { return handler !== null && Date.now() - beatMs < 3000 }

// A handler that throws answers false, so LG's chain gets the key; one log line per 5 s.
function fault(what, e) {
    var now = Date.now()
    if (now - lastFaultMs < 5000) return
    lastFaultMs = now
    console.warn("[LumaBus] " + what + " threw, the key goes to LG: " + e)
}

function dispatch(key, pressed, autoRepeat, deviceId) {
    if (!handler) return false
    try { return handler(key, pressed, autoRepeat, deviceId) === true }
    catch (e) { fault("handler", e); return false }
}

// Home held 650 ms (LumaKeyBridge.qml has written its request): LumaHome starts following
// the tool's answer.
function escape() {
    if (!escapeHandler) return
    try { escapeHandler() }
    catch (e) { fault("escape handler", e) }
}
