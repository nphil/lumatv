// Luma key filter. Registered in com.webos.surfacemanager.keyFilters (configd) by
// tools/lumatv; the stock StarfishKeyFilter loads every entry of that list through its
// StarfishKeyFilterLoader shell, which is the only supported way to see remote keys in the
// compositor: shadowing controllers/StarfishKeyFilter.qml never loads on this firmware.
//
// Runs inside the compositor's QML engine, so it can instantiate LumaKeyBridge.qml and share
// the LumaBus.js library instance with LumaHome. Accepted keys stop here; everything else
// falls through to the stock chain and Home's own surface.

var bridge = null;

function init() {
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
}

function handleLumaHome(key, pressed, autoRepeat, deviceId) {
    if (!bridge) return KeyPolicy.NextPolicy;
    // Home is only watched, so a short press still reaches LG's handler as it always did.
    if (key === Qt.Key_Super_L) return bridge.homeKey(pressed, autoRepeat) ? KeyPolicy.Accepted : KeyPolicy.NextPolicy;
    if (global.activeSurfaceAppId !== "com.webos.app.home" || !fullscreenView.activeFocus) return KeyPolicy.NextPolicy;
    return bridge.dispatch(key, pressed, autoRepeat, deviceId) ? KeyPolicy.Accepted : KeyPolicy.NextPolicy;
}
