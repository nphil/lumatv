// Luma key filter. Registered in com.webos.surfacemanager.keyFilters (configd) by
// tools/lumatv; the stock StarfishKeyFilter loads every entry of that list through its
// StarfishKeyFilterLoader shell, which is the only supported way to see remote keys in the
// compositor: shadowing controllers/StarfishKeyFilter.qml never loads on this firmware.
//
// Runs inside the compositor's QML engine, so it can instantiate LumaKeyBridge.qml and share
// the LumaBus.js library instance with LumaHome. Accepted keys stop here; everything else
// falls through to the stock chain and Home's own surface.
//
// The filter fails open. A key is taken only while Home is in front, LumaHome is answering
// (its heartbeat is under 3 s old) and its handler takes it; anything else, and any error
// anywhere in this file or in the handler, answers NextPolicy so LG's chain sees the key
// untouched. Presses and releases are paired: a release goes where its press went (an error
// is the one exception, it always goes to LG), so LG never receives half of a key it did not
// see go down, and a held OK that LumaHome acts on at release (never on press) cannot leave
// a stray release for the app it opens.

var bridge = null;
var taken = {};          // key code -> true while LumaHome holds that key's press
var lastFaultMs = 0;

function fault(where, e) {
    // one line per 5 s: a handler that throws on every key repeat must not flood the log
    var now = Date.now();
    if (now - lastFaultMs < 5000) return;
    lastFaultMs = now;
    console.warn("[LumaKeys] " + where + " failed, the key goes to LG: " + e);
}

function init() {
    try {
        var c = Qt.createComponent("file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/LumaKeyBridge.qml");
        if (c.status === Component.Ready) {
            bridge = c.createObject(null);
            // Stock starts its own timer on a Home press: 2.5 s of hold opens Screen Zoom. The
            // 650 ms hold is LG's-Home escape only, so once it has fired the stock timer goes.
            bridge.escapeHook = function() {
                try { longPressTimer.setBlockCancel(false); longPressTimer.cancel(); }
                catch (e) { console.warn("[LumaKeys] could not stop the stock Home hold: " + e); }
            };
            console.info("[LumaKeys] bridge ready");
        } else {
            console.warn("[LumaKeys] bridge failed: " + c.errorString());
        }
    } catch (e) {
        bridge = null;
        fault("init", e);
    }
}

function homeInFront() {
    return global.activeSurfaceAppId === "com.webos.app.home" && fullscreenView.activeFocus === true;
}

// true: the event ends here. false: LG's chain gets it.
function takeKey(key, pressed, autoRepeat, deviceId) {
    if (!pressed) {
        // The record goes first, so nothing below can leave it behind.
        if (!taken[key]) return false;
        delete taken[key];
        bridge.dispatch(key, false, false, deviceId);
        return true;
    }
    if (autoRepeat) {
        // Repeats follow their press: held keys keep moving focus, and a key LG got stays LG's.
        if (!taken[key]) return false;
        bridge.dispatch(key, true, true, deviceId);
        return true;
    }
    // A new press supersedes whatever an earlier one left, so a release that never came
    // cannot turn the next release of an app's key into ours.
    delete taken[key];
    if (!homeInFront() || !bridge.answering()) return false;
    if (!bridge.dispatch(key, true, false, deviceId)) return false;
    taken[key] = true;
    return true;
}

function handleLumaHome(key, pressed, autoRepeat, deviceId) {
    try {
        if (!bridge) return KeyPolicy.NextPolicy;
        // Home is only watched, so a short press still reaches LG's handler as it always did.
        if (key === Qt.Key_Super_L) return bridge.homeKey(pressed, autoRepeat) ? KeyPolicy.Accepted : KeyPolicy.NextPolicy;
        return takeKey(key, pressed, autoRepeat, deviceId) ? KeyPolicy.Accepted : KeyPolicy.NextPolicy;
    } catch (e) {
        fault("the filter", e);
        return KeyPolicy.NextPolicy;
    }
}
