.pragma library

// One engine-wide instance (pragma library, same absolute URL from both sides): LumaHome
// registers its key handler here, the configd-registered key filter (payload/keyfilter/
// lumatv.js) dispatches into it through LumaKeyBridge.qml. Keys never travel through
// QtQuick focus: the compositor's KeyFilter sends them straight to the focused Wayland
// window, so a QML item can only see them from inside the filter chain.
var handler = null
var escapeHandler = null

function setHandler(h) { handler = h }
function setEscapeHandler(h) { escapeHandler = h }

function dispatch(key, pressed, autoRepeat, deviceId) {
    if (!handler) return false
    try { return handler(key, pressed, autoRepeat, deviceId) === true }
    catch (e) { console.warn("[LumaBus] handler threw: " + e); return false }
}

// Home held 650 ms (LumaKeyBridge.qml has written its request): LumaHome starts following
// the tool's answer.
function escape() {
    if (!escapeHandler) return
    try { escapeHandler() }
    catch (e) { console.warn("[LumaBus] escape handler threw: " + e) }
}
