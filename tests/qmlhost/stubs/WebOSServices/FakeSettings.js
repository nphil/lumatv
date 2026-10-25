.pragma library

// A stand-in settings service for the offline host: the values the owner's TV answered in the X4 capture
// (picture mode Filmmaker, OLED pixel brightness 80, sound out the receiver over eARC) and plausible
// others, held in one place for every Service and for LS.adhoc's setSystemSettings. The host reads and
// writes it exactly as the pages do; whether the real service has the same keys is the set's to say
// (tools/settings_check.py, in the slot script).
var data = {
    picture: { pictureMode: "filmMaker", backlight: "80", contrast: "85", brightness: "50", color: "50", tint: "0", sharpness: "10",
               energySaving: "off", screenShift: "on", logoLuminanceAdjust: "strong", truMotionMode: "off", aspectRatio: "original",
               filmMakerMode: "on", aiPictureEnable: "off", eyeComfortMode: "off" },
    aspectRatio: { arcPerApp: "original" },
    sound: { soundMode: "standard", soundOutput: "external_arc", balance: "0", soundSyncDelay: "0", digitalSoundOut: "bitstream",
             avSyncAdjustment: "0", eArcSupport: "on", dolbyAtmos: "on", autoVolume: "off" },
    general: { deviceName: "Living Room TV", remotePointer: "on", pointerSpeed: "normal", pointerSize: "medium", quickStartMode: "on",
               simplinkEnable: "true", autoInput: "true", wolwowlOnOff: "true", timeZone: "America/New_York", timeZoneAuto: "on",
               autoOff4Hours: "on", autoOff2Hours: "off", language: "en-US" },
    option: { homeAutoLaunch: "on", lgLogoDisplay: "on", numberKeysDock: "on", holdHomeForLgHome: "on", audioGuidance: "off", highContrast: "off",
              country: "USA", localeCountryGroup: "US", ssuServer: "on", ambientLight: "off" },
    network: { },
    other: { lgLogoDisplay: "on", lightingEnable: "on" }
};

function read(category, keys) {
    var out = {};
    var cat = data[category] || {};
    var list = keys && keys.length ? keys : Object.keys(cat);
    for (var i = 0; i < list.length; i++) if (cat[list[i]] !== undefined) out[list[i]] = cat[list[i]];
    var doc = { returnValue: true, category: category, settings: out };
    if (category === "picture") doc.dimension = { pictureMode: cat.pictureMode || "filmMaker", dynamicRange: "sdr", input: "default" };
    return doc;
}

function write(category, settings) {
    if (!data[category]) data[category] = {};
    for (var k in settings) data[category][k] = settings[k];
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
    "luna://com.webos.service.connectionmanager/getStatus": {
        returnValue: true,
        wifi: { state: "connected", ssid: "Nate's Home", signalBars: 4, signalLevel: -52, channelFrequency: 5180, ipAddress: "192.168.1.146", method: "dhcp" },
        wired: { state: "disconnected" }
    }
};

function call(service, method, params) {
    if (method === "getSystemSettings") return read(params.category, params.keys || (params.key ? [params.key] : null));
    var doc = services[service + "/" + method];
    return doc || { returnValue: true };
}
