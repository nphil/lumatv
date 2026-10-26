.pragma library

// One engine-wide instance (pragma library, same absolute URL from both sides): LumaHome
// registers its key handler here, the configd-registered key filter (payload/keyfilter/
// lumatv.js) dispatches into it through LumaKeyBridge.qml. Keys never travel through
// QtQuick focus: the compositor's KeyFilter sends them straight to the focused Wayland
// window, so a QML item can only see them from inside the filter chain.
//
// Two more surfaces register here. The gear key's hold (the filter times it) is answered by
// LumaHome while Home is in front and by the overlay (LumaOverlay.qml, drawn over an app by the
// fullscreen container) while an app is: each says whether it can show the notification drawer now
// (`ready`) and opens it (`hold`). The overlay also takes keys while it has a surface that wants them
// (`wants`), as Home does.
var handler = null
var escapeHandler = null
var gearHome = null                     // { ready(): bool, hold(): bool }
var overlay = null                      // { wants(): bool, key(k, pressed, autoRepeat, device): bool, ready(): bool, hold(): bool }
var appKeyHandler = null                // (id, frontAppId) -> true when LumaHome took the remote's app key (launched an app, or does nothing)
var systemLauncher = null               // (appId, params) -> true when LG's system window was asked for: the key filter's own luna client (see systemLaunch)
var lastFaultMs = 0
var beatMs = 0

function setHandler(h) { handler = h }
function setEscapeHandler(h) { escapeHandler = h }
function setGear(g) { gearHome = g }
function setOverlay(o) { overlay = o }
function setAppKey(h) { appKeyHandler = h }
function setSystemLauncher(l) { systemLauncher = l }

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

// The overlay over an app has something that takes keys, and the layer is answering.
function overlayWants() {
    if (!overlay || !answering()) return false
    try { return overlay.wants() === true }
    catch (e) { fault("overlay.wants", e); return false }
}

function dispatchOverlay(key, pressed, autoRepeat, deviceId) {
    if (!overlay) return false
    try { return overlay.key(key, pressed, autoRepeat, deviceId) === true }
    catch (e) { fault("overlay", e); return false }
}

// Something of ours could show the drawer now: the hold is ours, and LG's Settings long press is not wanted.
function gearReady() {
    if (!answering()) return false
    try {
        if (gearHome && gearHome.ready() === true) return true
        return !!overlay && overlay.ready() === true
    } catch (e) { fault("gear ready", e); return false }
}

// The gear was held: the drawer opens where it can (Home first, then the overlay). True when one opened.
function gearHold() {
    try {
        if (gearHome && gearHome.ready() === true) return gearHome.hold() === true
        if (overlay && overlay.ready() === true) return overlay.hold() === true
    } catch (e) { fault("gear hold", e) }
    return false
}

// One of the remote's app keys (Netflix, Prime Video, Disney+, LG Channels: theme `remote.appKeys`) was pressed over whatever is in
// front. True when LumaHome took it; false hands it to LG, whose own target (and its auto-install) is the default only where that
// app is installed.
function appKey(id, frontAppId) {
    if (!appKeyHandler || !answering()) return false
    try { return appKeyHandler(id, frontAppId) === true }
    catch (e) { fault("app key", e); return false }
}

// Opens one of LG's system windows (Quick Settings, the input picker: systemUIManager/launch) through the key filter's own luna client,
// the one LG's key filters launch them with. The compositor hosts that service itself, and the plain `LS.adhoc` client is not known to be
// allowed to call it: over HDMI 2 an overlay button that used it opened nothing (docs/research/round7-2026-10-25/slot7). False when the
// filter has no client to give (the offline host, a filter that failed to start): the caller then falls back to LS.adhoc.
function systemLaunch(appId, params) {
    if (!systemLauncher) return false
    try { return systemLauncher(appId, params) === true }
    catch (e) { fault("system launch", e); return false }
}

// Home held 650 ms (LumaKeyBridge.qml has written its request): LumaHome starts following
// the tool's answer.
function escape() {
    if (!escapeHandler) return
    try { escapeHandler() }
    catch (e) { fault("escape handler", e) }
}
