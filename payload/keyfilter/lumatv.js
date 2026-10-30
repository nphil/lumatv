// Luma key filter. Registered in com.webos.surfacemanager.keyFilters (configd) by
// tools/lumatv; the stock StarfishKeyFilter loads every entry of that list through its
// StarfishKeyFilterLoader shell, which is the only supported way to see remote keys in the
// compositor: shadowing controllers/StarfishKeyFilter.qml never loads on this firmware.
//
// Runs inside the compositor's QML engine, so it can instantiate LumaKeyBridge.qml and share
// the LumaBus.js library instance with LumaHome. Accepted keys stop here; everything else
// falls through to the stock chain and Home's own surface.
//
// The filter fails open. A key is taken only while a surface of ours has the screen (Home, or
// the overlay over an app while it has something that takes keys), LumaHome is answering (its
// heartbeat is under 3 s old) and that surface's handler takes it; anything else, and any
// error anywhere in this file or in the handler, answers NextPolicy so LG's chain sees the key
// untouched. Presses and releases are paired: a release goes where its press went (an error
// is the one exception, it always goes to LG), so LG never receives half of a key it did not
// see go down, and a held OK that LumaHome acts on at release (never on press) cannot leave
// a stray release for the app it opens.
//
// The gear (Settings) key. LG's chain swallows its press (preProcess.js, `handleLongPressKeys`
// is fourth, behind this filter) and decides on release: before 1 s it replays the key, which
// opens Quick Settings and counts toward the three-tap AV reset (LG's hidden support menu, `tvhotkey`);
// at 1 s it injects Key_Option, which launches LG's Settings app. This filter watches the press, so a
// tap is LG's as it always was (the press also tells the quick panel to join the scene: `warmQuick`),
// except the third tap within 4 s, which is taken here whole and opens the quick panel itself, so
// LG's AV reset never counts it (`gearTaps`). Held 650 ms it asks LumaHome (or the overlay, over an
// app) to do what the owner chose for the hold (theme `remote.gearHold`: LumaTV's Settings, the
// notification drawer or nothing); once it has, LG's own long-press timer is cancelled the way
// the Home hold cancels Screen Zoom's, and the rest of the hold, its repeats and its release,
// ends here, so LG neither opens its Settings nor replays a tap. When nothing of ours can answer
// the hold, it is LG's. Key_Option, the 1 s conversion, is the safety net: while the gear is
// down it is ours too, and LG's Settings never opens over what the hold opened.
//
// The remote's app keys (Netflix, Prime Video, Disney+, LG Channels) are watched in any foreground: the owner may set each to open
// any app or nothing (theme `remote.buttons`, read by LumaHome). While LumaHome answers, the TV is on and nothing of LG's is on top of the
// app, LumaHome says whether it took the key; if it did, the press, its repeats and its release end here, and if not (the key is
// LG's default and that app is installed) LG's chain handles it as it always did.
//
// LumaTV's quick panel and input picker take the gear's tap and the Input key over LG's Quick Settings and input picker, on Home and over
// an app (`handleLumaQuick`, the second entry of this file in configd's list, placed just before LG's own `handleHotkeys`, the handler
// that opens those windows). By then everything earlier in the chain has had its say: LG's long-press replay of a tap has come back round
// to it, the three-tap AV reset has counted it, a hold this file took never gets there, and the key blocks and the screensaver's wake have
// run. It takes a key only where LG would have opened its window and something of ours can show the panel now; the gear is acted on at
// the release and the Input key at the press, as LG's are, and a second press closes the panel or goes to the next input. Anything else
// (the layer not answering, LG's own window on top, the first-use setup, a panel still being built) is LG's, whose window opens as it did.

var bridge = null;
var taken = {};          // key code -> "home", "overlay" or "appkey" while that holds the key's press
var lastFaultMs = 0;
var lastWhyMs = 0;      // when the last "left to LG" reason was logged

var GEAR_HOLD_MS = 650;  // PLAN: a hold is one press held 650 ms
var GEAR_KEY = null;     // WebOS.Key_webOS_Settings, read at init: a scope without WebOS leaves the gear to LG
var GEAR_WATCH_MS = 30000; // a release that never arrives must not hold the gear's state for good (a hold without repeat events is not cut short either)
var gear = { down: false, fired: false, hold: -1, watch: -1, option: false };
// LG counts three releases of the gear within 3 s as the AV reset (`handleAvReset`, counted after the long-press replay). The remote's own
// presses are counted here (a press within GEAR_REPLAY_MS of a release is LG's replay of the tap before it, not a press of the remote), a
// little wider than LG's window; the one that makes three is taken whole, so the replay LG counts never happens.
var GEAR_TAPS = 3;
var GEAR_TAPS_MS = 4000;
var GEAR_REPLAY_MS = 80;
var gearTaps = { times: [], lastUp: 0, taken: false };
var APP_KEYS = {};       // key code -> "netflix" | "amazon" | "disneyplus" | "lgchannels", read at init
var INPUT_KEYS = {};     // key code -> true: Key_webOS_InputSource and Key_webOS_LocalInputSource, read at init
var CURSOR_KEYS = {};    // key code -> true: Key_webOS_CursorShow and Key_webOS_CursorHide, which LG's chain makes of the Magic Remote's pointer showing and hiding
var late = {};           // key code -> "quick" | "picker" while handleLumaQuick holds that key's press

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
            // LG's system windows (Quick Settings, the input picker) are opened with the same client LG's own key filters open them with
            bridge.launchHook = function(appId, params) {
                var body = { appId: appId };
                if (params !== undefined && params !== null) body.params = params;
                lunaCall.send("com.webos.service.systemUIManager", "/launch", JSON.stringify(body));
                console.info("[LumaKeys] system window asked for: " + appId);
                return true;
            };
            var webos = null;
            try { webos = WebOS; } catch (e0) { console.warn("[LumaKeys] no WebOS in scope: the gear and the app keys stay LG's"); }
            if (webos) {
                GEAR_KEY = typeof webos.Key_webOS_Settings === "number" ? webos.Key_webOS_Settings : null;
                var names = { netflix: "Key_webOS_Netflix", amazon: "Key_webOS_Amazon", disneyplus: "Key_webOS_Disneyplus", lgchannels: "Key_webOS_LGChannels" };
                for (var id in names) if (typeof webos[names[id]] === "number") APP_KEYS[webos[names[id]]] = id;
                if (typeof webos.Key_webOS_InputSource === "number") INPUT_KEYS[webos.Key_webOS_InputSource] = true;
                if (typeof webos.Key_webOS_LocalInputSource === "number") INPUT_KEYS[webos.Key_webOS_LocalInputSource] = true;
                if (typeof webos.Key_webOS_CursorShow === "number") CURSOR_KEYS[webos.Key_webOS_CursorShow] = true;
                if (typeof webos.Key_webOS_CursorHide === "number") CURSOR_KEYS[webos.Key_webOS_CursorHide] = true;
            }
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

// An app has the screen and the keys: no LG overlay, popup, keyboard or alert is on top of it (they
// take the focus from the fullscreen view, and their keys are not ours to read).
function appInFront() {
    var id = global.activeSurfaceAppId;
    return fullscreenView.activeFocus === true && id !== "" && id !== "com.webos.app.home";
}

// The surface of ours that takes a new press: Home when it is in front and answering, else the overlay
// over an app while it has a surface that takes keys (a banner with buttons, the drawer). "" for neither.
function frontTarget() {
    if (homeInFront()) return bridge.answering() ? "home" : "";
    if (appInFront() && bridge.overlayWants()) return "overlay";
    return "";
}

function dispatchTo(target, key, pressed, autoRepeat, deviceId) {
    if (target === "appkey") return true;          // nothing to tell: LumaHome took the press and the rest of the key ends here
    return target === "overlay" ? bridge.dispatchOverlay(key, pressed, autoRepeat, deviceId)
                                : bridge.dispatch(key, pressed, autoRepeat, deviceId);
}

// true: the event ends here. false: LG's chain gets it.
function takeKey(key, pressed, autoRepeat, deviceId) {
    if (!pressed) {
        // The record goes first, so nothing below can leave it behind.
        var to = taken[key];
        if (!to) return false;
        delete taken[key];
        dispatchTo(to, key, false, false, deviceId);
        return true;
    }
    if (autoRepeat) {
        // Repeats follow their press: held keys keep moving focus, and a key LG got stays LG's.
        var on = taken[key];
        if (!on) return false;
        dispatchTo(on, key, true, true, deviceId);
        return true;
    }
    // A new press supersedes whatever an earlier one left, so a release that never came
    // cannot turn the next release of an app's key into ours.
    delete taken[key];
    var target = frontTarget();
    if (target === "") return false;
    if (!dispatchTo(target, key, true, false, deviceId)) return false;
    taken[key] = target;
    return true;
}

// ---- the remote's app keys: only a new press is decided here (the repeats and the release of a press taken follow it in takeKey)
function appKey(key, pressed, autoRepeat) {
    if (!pressed || autoRepeat) return false;
    // LG's own handling stays for everything it does better: the TV off (the key powers it on and opens the app), a modal alert, LG's
    // overlay or keyboard on top (the focus is not the fullscreen view's), and a layer that is not answering
    if (global.powerState !== "active" || fullscreenView.activeFocus !== true || !bridge.answering()) return false;
    if (!bridge.appKey(APP_KEYS[key], global.activeSurfaceAppId)) return false;
    taken[key] = "appkey";
    return true;
}

// ---- the gear key

function gearStop(id) {
    if (id >= 0) { try { genericTimer.cancel(id); } catch (e) { } }
}

function gearReset() {
    gearStop(gear.hold);
    gearStop(gear.watch);
    gear.down = false; gear.fired = false; gear.hold = -1; gear.watch = -1; gear.option = false;
}

// Something of ours has the screen and can do what the hold is for now.
function gearWanted() {
    return (homeInFront() || appInFront()) && bridge.answering() && bridge.gearReady();
}

// LG's own timer for this press (1 s, with its ring on screen) would turn the hold into Key_Option.
function gearCancelLg() {
    try {
        longPressTimer.setBlockCancel(false);
        longPressTimer.cancel();
        longPressTimer.setDeviceId(-1);
    } catch (e) {
        console.warn("[LumaKeys] could not stop the stock gear hold: " + e);
    }
}

function gearWatch() {
    // restarted by every event of the hold: it only fires when the events stop (a lost release)
    gearStop(gear.watch);
    gear.watch = genericTimer.set(GEAR_WATCH_MS, function() { gearReset(); });
}

function gearFire() {
    gear.hold = -1;
    try {
        if (!gear.down || gear.fired || !gearWanted()) return;
        if (!bridge.gearHold()) return;          // nothing answered: the hold stays LG's
        gear.fired = true;
        gearCancelLg();
        gearWatch();
        console.info("[LumaKeys] gear held, the hold's own answer is up");
    } catch (e) {
        fault("the gear hold", e);
    }
}

// true: the event ends here.
function gearKey(pressed, autoRepeat) {
    if (pressed) {
        if (!autoRepeat) {
            // A new press supersedes whatever an earlier one left. LG's chain still sees the press: a tap stays LG's.
            gearReset();
            if (gearWanted()) {
                gear.down = true;
                gear.hold = genericTimer.set(GEAR_HOLD_MS, gearFire);
                gearWatch();
            }
            return false;
        }
        if (!gear.fired) return false;
        gearWatch();
        return true;
    }
    // When LG's own timer ran (Key_Option was injected) LG must see this release: it ends that hold in its own state
    // (it answers with the Key_Option release, which optionKey takes). Otherwise the hold ended here and LG saw none of it.
    var swallow = gear.fired && !gear.option;
    var option = gear.option;
    gearReset();
    gear.option = option;
    return swallow;
}

// The third tap of the gear within GEAR_TAPS_MS: LG's chain never gets it. Its press and repeats end here; at the release, unless a hold fired,
// the quick panel opens (or, when ours cannot, LG's own Quick Settings, which is what LG's chain did with the first two).
function gearTapTaken() {
    try {
        if (panelWanted("quick") && bridge.panel("quick") === true) return;
        if (global.powerState === "active" && lgWouldOpen("quick") && typeof bridge.launchHook === "function") bridge.launchHook("com.webos.app.quicksettings");
    } catch (e) {
        fault("the gear's third tap", e);
    }
}

// One event of the gear: the hold's state (`gearKey`) and the three-tap guard. true: the event ends here.
function gearEvent(pressed, autoRepeat) {
    var t = Date.now();
    if (pressed && !autoRepeat) {
        gearTaps.taken = false;
        if (t - gearTaps.lastUp >= GEAR_REPLAY_MS) {
            gearTaps.times = gearTaps.times.filter(function(at) { return t - at < GEAR_TAPS_MS; });
            gearTaps.times.push(t);
            gearTaps.taken = gearTaps.times.length >= GEAR_TAPS;
        }
        warmQuick();
    }
    var heldHere = gear.fired && !gear.option;
    var swallow = gearKey(pressed, autoRepeat);
    if (!gearTaps.taken) {
        if (!pressed) gearTaps.lastUp = t;
        return swallow;
    }
    if (!pressed) {
        gearTaps.taken = false;
        if (!heldHere && !gear.option) gearTapTaken();
    }
    return true;
}

// The gear is down. If it is let go within the hold, LG replays it as a tap and the second entry below opens the quick panel, so the panel
// joins the scene now, off the screen, and the tap's slide starts in a scene that has it (panels/PanelMotion.qml has the numbers). Nothing is
// shown, and LG's chain still gets the press.
function warmQuick() {
    try {
        if (global.powerState !== "active" || (!homeInFront() && !appInFront()) || !bridge.answering()) return;
        if (bridge.panelReady("quick") === true) bridge.warmPanel("quick");
    } catch (e) {
        fault("the quick panel's warm-up", e);
    }
}

// Key_Option: LG's 1 s conversion of a held gear (and the key an Option key of a keyboard would send, which is none of
// this). Ours only while the gear is down or its hold has been taken.
function optionKey(pressed) {
    if (pressed) {
        if (gear.fired) { gear.option = true; return true; }
        if (gear.down && gearWanted() && bridge.gearHold()) {
            gear.fired = true; gear.option = true;
            gearWatch();
            console.info("[LumaKeys] gear held, the hold's own answer is up (LG's own long press ran first)");
            return true;
        }
        return false;
    }
    if (gear.option) { gear.option = false; return true; }
    return false;
}

function handleLumaHome(key, pressed, autoRepeat, deviceId) {
    try {
        if (!bridge) return KeyPolicy.NextPolicy;
        // The pointer showing or hiding is not a key anybody pressed: it ends no hold, wakes nothing and is no activity of ours. LG's chain handles it
        // as it always did (its screensaver takes it from the compositor's own signal), so it is never given to Home or the overlay.
        if (CURSOR_KEYS[key] === true) return KeyPolicy.NextPolicy;
        // Home is only watched, so a short press still reaches LG's handler as it always did.
        if (key === Qt.Key_Super_L) return bridge.homeKey(pressed, autoRepeat) ? KeyPolicy.Accepted : KeyPolicy.NextPolicy;
        // A surface of ours may take any key first: the saver wakes on the gear like on any other key
        if (takeKey(key, pressed, autoRepeat, deviceId)) return KeyPolicy.Accepted;
        if (APP_KEYS[key] !== undefined) return appKey(key, pressed, autoRepeat) ? KeyPolicy.Accepted : KeyPolicy.NextPolicy;
        if (GEAR_KEY !== null && key === GEAR_KEY) {
            return gearEvent(pressed, autoRepeat) ? KeyPolicy.Accepted : KeyPolicy.NextPolicy;
        }
        if (key === Qt.Key_Option) return optionKey(pressed) ? KeyPolicy.Accepted : KeyPolicy.NextPolicy;
        return KeyPolicy.NextPolicy;
    } catch (e) {
        fault("the filter", e);
        return KeyPolicy.NextPolicy;
    }
}

// ---- LumaTV's quick panel and input picker (the second entry)

function panelKind(key) {
    if (GEAR_KEY !== null && key === GEAR_KEY) return "quick";
    if (INPUT_KEYS[key] === true) return "picker";
    return "";
}

// LG's own conditions for opening these windows (KeyFilters/appLaunch.js, handleHotkeys): launches allowed, not the first-use setup, and
// the input picker is not opened over the partial view. A condition this scope cannot read throws, which leaves the key to LG.
function lgWouldOpen(kind) {
    if (access.keyAction.launchApp !== true) return false;
    if (bootManager.bootFirstUse === true && systemPropertyService.powerOnlyMode !== true) return false;
    if (kind === "picker" && global.isForegroundApp("com.webos.app.partialview")) return false;
    return true;
}

// One line per 5 s of why a key was left to LG's window, for whoever finds the panel not opening: a plain "no" leaves no other trace.
function left(kind, why) {
    var now = Date.now();
    if (now - lastWhyMs >= 5000) {
        lastWhyMs = now;
        console.info("[LumaKeys] the " + (kind === "quick" ? "gear's tap" : "Input key") + " is LG's: " + why);
    }
    return false;
}

// The TV is on, Home or an app has the screen and the keys (no LG overlay, popup or alert on top), the layer answers and has the panel built.
function panelWanted(kind) {
    if (global.powerState !== "active") return left(kind, "the TV is not active (" + global.powerState + ")");
    if (!homeInFront() && !appInFront()) return left(kind, "neither Home nor an app has the screen and the focus");
    if (!lgWouldOpen(kind)) return left(kind, "LG would not open its window either");
    if (!bridge.answering()) return left(kind, "the layer is not answering");
    if (!bridge.panelReady(kind)) return left(kind, "no panel is ready (not built yet, switched off, or not in front)");
    return true;
}

// true: the event ends here.
function panelKey(key, kind, pressed, autoRepeat) {
    if (pressed) {
        // a repeat follows its press, and a key that was never ours has nothing to hold
        if (autoRepeat) return late[key] === kind;
        delete late[key];
        if (!panelWanted(kind)) return false;
        // the Input key acts on the press, as LG's does; the gear on its release
        if (kind === "picker" && !bridge.panel(kind)) return false;
        late[key] = kind;
        return true;
    }
    if (late[key] !== kind) return false;
    delete late[key];
    if (kind === "picker") return true;
    // the press was ours; if the panel cannot open after all, LG sees the release and opens its window
    return bridge.panel(kind) === true;
}

function handleLumaQuick(key, pressed, autoRepeat, deviceId) {
    try {
        if (!bridge) return KeyPolicy.NextPolicy;
        var kind = panelKind(key);
        if (kind === "") return KeyPolicy.NextPolicy;
        return panelKey(key, kind, pressed, autoRepeat) ? KeyPolicy.Accepted : KeyPolicy.NextPolicy;
    } catch (e) {
        delete late[key];
        fault("the quick panel key", e);
        return KeyPolicy.NextPolicy;
    }
}
