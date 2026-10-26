.pragma library
.import "LgSettings.js" as Lg
.import "LgDesc.js" as Desc
.import "FakeCoverage.js" as Cov
.import "FakeMore.js" as More

// A stand-in settings service for the offline host: the values the owner's TV answered in the X4 capture
// (picture mode Filmmaker, OLED pixel brightness 80, sound out the receiver over eARC) and plausible
// others, held in one place for every Service and for LS.adhoc's setSystemSettings. Keys sit in the categories
// LG's own Settings app uses (LgSettings.js); a write to any other key or to a value that key does not take is
// refused and kept in `rejected`, so a test can see that no page wrote one. Whether the real service answers
// the same keys is the set's to say (tools/settings_check.py, in the slot script).
//
// Answers take the set's own shape (OLED65C4, webOS 24, read 2026-10-25): a read answers the keys it has in
// `settings` and names the rest in `errorKey` with `returnValue: false`; a per-app key (truMotionMode, arcPerApp, justScan)
// answers only when asked with `app_id`, and with `app_id` the other keys of the category are the missing ones;
// a description comes as `results`, one entry per key the service has. The set has no filmmakerAmbientLight,
// and `localeInfo` is read with no category.
var data = {
    picture: { pictureMode: "filmMaker", backlight: "80", contrast: "85", brightness: "50", color: "50", tint: "0", sharpness: "10",
               energySaving: "off", screenShift: "on", logoLuminanceAdjust: "strong", truMotionMode: "off", blackLevel: "auto",
               filmMakerMode: "on", eyeComfortMode: "off" },
    aiPicture: { ai_Picture: "off", ai_Genre: "off", ai_Brightness: "on" },
    aspectRatio: { arcPerApp: "original", justScan: "on" },
    sound: { soundMode: "standard", soundOutput: "external_arc", audioBalance: "0", soundOutputDigital: "passThrough",
             avSync: "on", avSyncSpdif: "0", avSyncSpeaker: "0", avSyncWisa: "0", avSyncBtSurround: "0", eArcSupport: "on", autoVolume: "off" },
    general: { remotePointer: "on", lifeOnScreenMode: "off", homeAutoLaunch: "on" },
    network: { deviceName: "Living Room TV", wolwowlOnOff: "true" },
    option: { pointerSpeed: "normal", pointerSize: "medium", quickStartMode: "on", audioGuidance: "off", highContrast: "off", country: "USA",
              localeCountryGroup: "US", screenSaverTimer: "10" },
    other: { simplinkEnable: "on", lgLogoDisplay: "on", lightingEnable: "on", oledCareMode: "off" },
    time: { noOperationAutoOff: "3", autoOff15Min: "on", noSignalAutoOff: "on", timeZone: { ZoneID: "America/New_York", City: "New York" } },
    caption: { captionEnable: "off" },
    "": { localeInfo: { keyboards: ["en"], locales: { UI: "en-US", FMT: "en-US", STT: "en-US" }, clock: "locale", timezone: "" } }
};
Cov.apply(data);
More.apply(data);
// What the TV starts with, for a reset to put back.
var defaults = JSON.parse(JSON.stringify(data));
// resetSystemSettings {category, keys}: each key goes back to the value the TV started with (LG's reset of a sound mode, of Fine Tune or of
// White Balance sends the keys it wants back; a picture mode's reset names the mode instead and changes no key here).
function restore(params) {
    if (!params || !params.category || !Array.isArray(params.keys)) return;
    var cat = data[params.category] || {};
    var def = defaults[params.category] || {};
    for (var i = 0; i < params.keys.length; i++) if (def[params.keys[i]] !== undefined) cat[params.keys[i]] = JSON.parse(JSON.stringify(def[params.keys[i]]));
}
// the keys the service keeps per app: answered only with `app_id`
var perApp = { picture: ["truMotionMode"], aspectRatio: ["arcPerApp", "justScan"] };

// What the service refused, as {category, key, value, why}; a test reads it back through LS.
var rejected = [];
// "category.key" -> false: the service says the setting exists but cannot be changed now (its description's ui.active).
var inactive = {};

function perAppKey(category, key) { return (perApp[category] || []).indexOf(key) >= 0; }

function read(category, keys, appId) {
    var out = {};
    var missing = [];
    var cat = data[category || ""] || {};
    var list = keys && keys.length ? keys : Object.keys(cat);
    for (var i = 0; i < list.length; i++) {
        if (cat[list[i]] !== undefined && perAppKey(category, list[i]) === !!appId) out[list[i]] = cat[list[i]];
        else missing.push(list[i]);
    }
    var doc = { returnValue: missing.length === 0, method: "getSystemSettings", settings: out };
    if (category) doc.category = category;
    if (appId) doc.app_id = appId;
    if (missing.length) doc.errorKey = missing;
    if (category === "picture") doc.dimension = { pictureMode: cat.pictureMode || "filmMaker", dynamicRange: "sdr", input: "default" };
    return doc;
}

// Applies what the real service would accept and returns {returnValue, errorText?}. One bad key refuses the whole call.
function write(category, settings) {
    for (var k in settings) {
        var why = Lg.refuse(category, k, settings[k]);
        if (why) {
            rejected.push({ category: category, key: k, value: settings[k], why: why });
            return { returnValue: false, errorText: why };
        }
    }
    if (!data[category]) data[category] = {};
    for (var j in settings) data[category][j] = settings[j];
    return { returnValue: true };
}

// getSystemSettingDesc: per key the service has, whether the screen may show it and let it change (what LG's app reads as `ui`).
function desc(category, keys) {
    var cat = data[category || ""] || {};
    var results = [];
    var missing = [];
    var list = keys && keys.length ? keys : Object.keys(cat);
    for (var i = 0; i < list.length; i++) {
        if (cat[list[i]] === undefined) { missing.push(list[i]); continue; }
        var entry = { key: list[i], category: category || "", ui: { displayName: list[i], visible: Cov.hidden[category + "." + list[i]] !== true && More.hidden[category + "." + list[i]] !== true, active: inactive[category + "." + list[i]] !== false, widget: "None" }, values: {} };
        // the value list or range the TV's own description gives the key (LgDesc.js)
        var t = Desc.d[category + "." + list[i]];
        if (t && t[0] === "E") entry.values = { arrayExt: t[1].map(function(v) { return { visible: true, active: true, value: v }; })
                                                     .concat(t[2].map(function(v) { return { visible: false, active: true, value: v }; })) };
        else if (t && t[0] === "R") entry.values = { range: { min: t[1], max: t[2], interval: t[3] } };
        // a delay's limits come in the description, as LG's own app reads them (values.range); these are the fixture's, the TV's own are read
        if (category === "sound" && /^avSync(Spdif|Speaker|Wisa|BtSurround)$/.test(list[i])) entry.values = { range: { min: 0, max: 400, interval: 10 } };
        results.push(entry);
    }
    var doc = { returnValue: missing.length === 0, method: "getSystemSettingDesc", results: results };
    if (missing.length) doc.errorKey = missing;
    return doc;
}

// The other services the Settings pages and the input picker read, answered as the host's make-believe TV
// would: four HDMI ports (Apple TV 4K, the Denon receiver in front, a PlayStation, one empty) and a strong
// Wi-Fi link. `call` returns the JSON the service would send.
var services = {
    "luna://com.webos.service.eim/getAllInputStatus": {
        returnValue: true,
        devices: [
            { appId: "com.webos.app.hdmi1", id: "HDMI_1", label: "Apple TV 4K", connected: true },
            { appId: "com.webos.app.hdmi2", id: "HDMI_2", label: "Denon AVR-S760H", connected: true },
            { appId: "com.webos.app.hdmi3", id: "HDMI_3", label: "PlayStation 5", connected: true },
            { appId: "com.webos.app.hdmi4", id: "HDMI_4", label: "HDMI 4", connected: false }
        ]
    },
    "luna://com.webos.service.applicationmanager/getForegroundAppInfo": { returnValue: true, foregroundAppInfo: [{ appId: "com.webos.app.hdmi2" }] },
    // the panel's usage counter in LG's ten-minute ticks (10230 ticks = 1,705 h) and the config that says LG shows it
    "luna://com.webos.service.panelcontroller/getPanelUsageTime": { returnValue: true, panelUsageTime: 10230 },
    "luna://com.webos.service.config/getConfigs": { returnValue: true, configs: { "tv.conti.supportUsedTime": true } },
    "luna://com.webos.service.connectionmanager/getStatus": {
        returnValue: true,
        wifi: { state: "connected", ssid: "Nate's Home", signalBars: 4, signalLevel: -52, channelFrequency: 5180, ipAddress: "192.168.1.146", method: "dhcp" },
        wired: { state: "disconnected" }
    }
};

// The sleep timer (tvReservationAgent): what is left, in seconds, set by `timer/sleep/add` (minutes as a string, "0" clears it).
var sleepSeconds = 0;
var sleepAdds = [];
function sleepAdd(params) {
    var m = Number(params && params.sleepTimeMinute);
    sleepAdds.push(params);
    if (isNaN(m) || m < 0) return { returnValue: false, errorText: "bad sleepTimeMinute" };
    sleepSeconds = m * 60;
    return { returnValue: true };
}

function call(service, method, params) {
    // a call that changes the TV in a way the coverage pages read back (FakeCoverage.js), or is refused
    var refused = Cov.act(service + "/" + method, params || {}) || More.act(service + "/" + method, params || {});
    if (refused) return { returnValue: false, errorText: refused };
    if (method === "getSystemSettings") return read(params.category, params.keys || (params.key ? [params.key] : null), params.app_id);
    if (method === "getSystemSettingDesc") return desc(params.category, params.keys || (params.key ? [params.key] : null));
    if (service.indexOf("tvReservationAgent") >= 0 && method === "getRemainTime") {
        return sleepSeconds > 0 ? { returnValue: true, sleepTimeMinute: Math.floor(sleepSeconds / 60), sleepTimeSecond: sleepSeconds % 60 } : { returnValue: true, sleepTimeSecond: 0 };
    }
    var doc = Cov.services[service + "/" + method] || More.services[service + "/" + method] || services[service + "/" + method];
    if (typeof doc === "function") doc = doc(params);
    return doc || { returnValue: true };
}
