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
        console.info("[LumaKeys] bridge ready");
    } else {
        console.warn("[LumaKeys] bridge failed: " + c.errorString());
    }
}

function handleLumaHome(key, pressed, autoRepeat, deviceId) {
    if (!bridge) return KeyPolicy.NextPolicy;
    if (global.activeSurfaceAppId !== "com.webos.app.home" || !fullscreenView.activeFocus) return KeyPolicy.NextPolicy;
    return bridge.dispatch(key, pressed, autoRepeat, deviceId) ? KeyPolicy.Accepted : KeyPolicy.NextPolicy;
}
