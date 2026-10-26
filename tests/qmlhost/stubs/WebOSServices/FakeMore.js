.pragma library
.import "LgDesc.js" as Desc

// What the offline TV answers for the pages that finish the coverage of LG's Settings (docs/research/settings-coverage.md): language and
// location, support, safety, channels and the one-off rows. Same shape as FakeCoverage.js: `seed` (one plausible value per key, from the lists
// in LgDesc.js, the TV's own), `hidden` (keys the TV's description marks not visible), `act` (a call that changes the TV: null, or the reason it was
// refused), `effect` (the setting such a call changes, for LS to write) and `services` ("service/method" to the reply, or a function of the
// parameters). The identities here (serial, ESN, Widevine, AD ID) are made up for the offline host; the real TV's are read, never kept.
// the lists of keys the TV gives for LG's Reset of the white balance (keyListForCopyWb) and of Fine Tune (keyListForCopyCms)
var WB_KEYS = ["whiteBalanceRed", "whiteBalanceGreen", "whiteBalanceBlue", "adjustingLuminance", "whiteBalanceRed10pt", "whiteBalanceGreen10pt", "whiteBalanceBlue10pt",
               "adjustingLuminance10pt", "whiteBalanceRedGain", "whiteBalanceGreenGain", "whiteBalanceBlueGain", "whiteBalanceRedOffset", "whiteBalanceGreenOffset",
               "whiteBalanceBlueOffset", "whiteBalanceLuminance"];
var CMS_KEYS = [];
['Red', 'Green', 'Blue', 'Cyan', 'Magenta', 'Yellow'].forEach(function(c) { CMS_KEYS.push("colorManagementSaturation" + c, "colorManagementHue" + c, "colorManagementLuminance" + c); });
var seed = {
    "": { eulaStatus: { generalTermsAllowed: true, voiceAllowed: true } },
    option: { autoSmartServiceCountry: "on", zipcode: "94105", vsn: "FAKE-VSN-01", "interactive-service": "offByUser", "interactive-service-hdmi": "onByUser",
              broadcastInfoNoti: "on", lifeOnScreenEnergySaving: "auto", lifeOnScreenNotification: "true", lifeOnScreenUsingMotionSensor: "false",
              remoteSlowInput: "off", longPressInterval: "basic", repeatInterval: "basic", livePlus: "off", livePromotion: "off", emergencyInformationAtsc30: "off",
              emergencyInformationLanguageAtsc30: "eng", checkChangedScreenSaverTimerByUser: "default",
              cameraResourcePermission: [{ appId: "com.webos.app.camera", permissions: [{ resource: "camera", permission: true }] },
                                         { appId: "netflix", permissions: [{ resource: "camera", permission: false }] }],
              nearbyDevicePermission: [{ appId: "com.lighting.huesync", permissions: [{ resource: "ble", permission: true }] }], fileapiResourcePermission: [] },
    other: { oledCareMode: "off", oledCareRecommendation: "on", contentRecommendation: "on", colorimetry: "auto", eotf: "auto", masteringPeak: "auto", masteringColor: "auto",
             maxCLL: "auto", maxFALL: "auto", dolbyVSVDBVer: "v1" },
    support: { automaticUpdate: "off" },
    sound: { bluetoothMode: "speakerMode" },
    general: { doNotSellMyPersonalInformation: "off", lmt: "off", aiNudge: "on", aiSettingsNudge: "on", homePromotion: "on", screenSaverAd: "on", selectedInputApp: "off",
               checkHomeAutoLaunchValue: "default" },
    lock: { parentalControl: "false", movieRating: "off", applockPerApp: "false", inputBlock: "0", tvRatingGeneralAge: "off", tvRatingGeneralDialog: "off",
            tvRatingGeneralLanguage: "off", tvRatingGeneralSex: "off", tvRatingGeneralViolence: "off", tvRatingChildrenAge: "off", tvRatingChildrenFantasy: "off" },
    channel: { autoscanChannelGroup: "primary_channel", analogSystemColor: "multi" },
    picture: { scheduledPixelCleaning: "off", energySavingAutoMin: "0", pictureSettingModified: {}, whiteBalanceMethod: "22", whiteBalancePoint: "high", whiteBalanceIre: "100", whiteBalanceIre10pt: "100",
               whiteBalanceLuminance: "100", whiteBalanceRedGain: "0", whiteBalanceGreenGain: "0", whiteBalanceBlueGain: "0", whiteBalanceRedOffset: "0",
               whiteBalanceGreenOffset: "0", whiteBalanceBlueOffset: "0", colorManagementColorSystem: "red", keyListForCopyWb: WB_KEYS, keyListForCopyCms: CMS_KEYS }
};
// 22 and 10 numbers, one per signal level
function zeros(n) { var a = []; for (var i = 0; i < n; i++) a.push(0); return a; }
seed.picture.whiteBalanceRed = zeros(22); seed.picture.whiteBalanceGreen = zeros(22); seed.picture.whiteBalanceBlue = zeros(22); seed.picture.adjustingLuminance = zeros(22);
seed.picture.whiteBalanceRed10pt = zeros(10); seed.picture.whiteBalanceGreen10pt = zeros(10); seed.picture.whiteBalanceBlue10pt = zeros(10); seed.picture.adjustingLuminance10pt = zeros(10);
var colours = ["Red", "Green", "Blue", "Cyan", "Magenta", "Yellow"];
for (var ci = 0; ci < colours.length; ci++) {
    seed.picture["colorManagementSaturation" + colours[ci]] = "0"; seed.picture["colorManagementHue" + colours[ci]] = "0"; seed.picture["colorManagementLuminance" + colours[ci]] = "0";
}
var hidden = { "option.vsn": true, "option.broadcastInfoNoti": true };

function apply(data) {
    for (var c in seed) {
        if (!data[c]) data[c] = {};
        for (var k in seed[c]) if (data[c][k] === undefined) data[c][k] = seed[c][k];
    }
}

var ifa = 1;
function ifaId() { return "00000000-0000-4000-8000-00000000000" + ifa; }

// what the permission service (`ppm`) keeps per app: the camera's and the nearby devices' also live in the settings, so only the microphone and the files are changed here
var privacy = [
    { appId: "com.webos.app.camera", permissionTable: { "camera.privacy": "allow", "microphone.privacy": "deny" } },
    { appId: "netflix", permissionTable: { "camera.privacy": "deny", "microphone.privacy": "allow", "usbFileAccess.privacy": "allow" } },
    { appId: "com.lighting.huesync", permissionTable: { "nearbydevice.privacy": "allow", "microphone.privacy": "deny" } }
];
function setPermission(p) {
    if (!p || typeof p.appId !== "string" || (p.status !== "allow" && p.status !== "deny")) return "bad parameters";
    for (var i = 0; i < privacy.length; i++) {
        if (privacy[i].appId !== p.appId) continue;
        if (privacy[i].permissionTable[p.privacyGroup] === undefined) return "no " + p.privacyGroup + " for " + p.appId;
        privacy[i].permissionTable[p.privacyGroup] = p.status;
        return null;
    }
    return "unknown app";
}

// the sound cards on the TV's USB ports (`audio/tv getSoundCardsState`: number, name, playbackState, selected); one plays
var cards = [{ number: 1, name: "USB Audio Device", playbackState: "disconnected", selected: false }, { number: 2, name: "Gaming Headset", playbackState: "connected", selected: true }];
function setSoundOutput(p) {
    if (!p || p.soundOutput !== "usb_speaker") return "bad soundOutput";
    var found = false;
    for (var i = 0; i < cards.length; i++) {
        var on = cards[i].number === p.cardNumber;
        if (on) found = true;
        cards[i].playbackState = on ? "connected" : "disconnected";
        cards[i].selected = on;
    }
    return found ? null : "no such card";
}

function act(path, params) {
    if (path === "luna://com.webos.service.sdx/setCountrySettingByAuto") return params && params.uniqueIndex ? null : "no uniqueIndex";
    if (path === "luna://com.webos.service.devicereset/doReset") return params && (params.resetType === "user" || params.resetType === "userKeepNetwork") ? null : "bad resetType";
    if (path === "luna://com.webos.service.admanager/updateDNS") return params && (params.status === "on" || params.status === "off") ? null : "bad status";
    if (path === "luna://com.webos.service.admanager/resetIFA") { ifa++; return null; }
    if (path === "luna://com.webos.service.ppm/setPermission") return setPermission(params);
    if (path === "luna://com.webos.service.audio/setSoundOutput") return setSoundOutput(params);
    return null;
}
// the setting a call changes by itself (LG's ad service keeps its own switch in the settings)
function effect(path, params) {
    if (path === "luna://com.webos.service.admanager/updateDNS" && params) return { category: "general", settings: { doNotSellMyPersonalInformation: params.status } };
    if (path === "luna://com.webos.service.audio/setSoundOutput" && params && params.soundOutput === "usb_speaker") return { category: "sound", settings: { soundOutput: "usb_speaker" } };
    return null;
}

// ---- language: the lists the settings service gives for `getSystemSettingValues {key: "locale"}`
var languages = {
    returnValue: true, method: "getSystemSettingValues",
    locale: [
        { name: "English", languageCode: "en", countries: [{ name: "United States", spec: "en-US" }, { name: "United Kingdom", spec: "en-GB" }] },
        { name: "Espa\u00f1ol", languageCode: "es", countries: [{ name: "Estados Unidos", spec: "es-US" }] },
        { name: "Fran\u00e7ais", languageCode: "fr", countries: [{ name: "France", spec: "fr-FR" }] }
    ],
    audio: [
        { name: "English", languageCode: "en", countries: [{ spec: "en-US" }] },
        { name: "Spanish", languageCode: "es", countries: [{ spec: "es-US" }] },
        { name: "French", languageCode: "fr", countries: [{ spec: "fr-FR" }] }
    ],
    vkb: [{ name: "English", languageCode: "en" }, { name: "Espa\u00f1ol", languageCode: "es" }, { name: "Fran\u00e7ais", languageCode: "fr" }]
};

var services = {
    "luna://com.webos.service.settings/getSystemSettingValues": function(p) { return p && p.key === "locale" ? languages : { returnValue: false, errorText: "unknown key" }; },
    "luna://com.webos.service.ppm/queryPermissions": function() { return { returnValue: true, subscribed: true, privacyPermissions: privacy }; },
    "luna://com.webos.service.config/getConfigs": function(p) {
        var known = { "tv.conti.supportUsedTime": true, "system.supportATSC3tuner": true, "profile.list": ["ledLighting", "usbSoundOutput"],
                      "system.supportBluetoothFeatures": ["wirelesskeyboard", "btsound", "remoteapp", "btsoundsink", "bluetoothPlusTvSpeaker"] }, out = {};
        var names = p && p.configNames ? p.configNames : [];
        for (var i = 0; i < names.length; i++) if (known[names[i]] !== undefined) out[names[i]] = known[names[i]];
        return { returnValue: true, configs: out };
    },
    "luna://com.webos.service.btaudiosrc/surround/getVolumeBalancing": { returnValue: true, subscribed: true, tvspeaker_bluetooth: 5, bluetooth_left_right: 5 },
    "luna://com.webos.service.audio/tv/getSoundCardsState": function() { return { returnValue: true, subscribed: true, soundCards: cards }; },
    "luna://com.webos.service.sdx/setCountrySettingByAuto": { returnValue: true, serverResponse: { code: "200" } },
    "luna://com.palm.systemservice/deviceInfo/query": { returnValue: true, product_id: "OLED65C4PUA.AUSQLJR", serial_number: "FAKE-SERIAL-0001" },
    "luna://com.palm.systemservice/osInfo/query": { returnValue: true, core_os_release: "9.1.2", core_os_release_codename: "fake" },
    "luna://com.webos.service.attachedstoragemanager/getProperties": { returnValue: true, totalSpace: 2900, freeSpace: 1250 },
    "luna://com.webos.service.connectionmanager/getinfo": { returnValue: true, wiredInfo: { macAddress: "AA:BB:CC:11:22:33" }, wifiInfo: { macAddress: "AA:BB:CC:44:55:66" } },
    "luna://com.webos.service.pqcontroller/getInfo": { returnValue: true, dolbySWVersion: "4.0.1" },
    "luna://com.webos.service.sm/crypto/checkSeStore": { returnValue: true, keyStatusList: [{ id: "esn", isOk: true, identifier: "FAKE-ESN-0001" }, { id: "widevine", isOk: true, identifier: "FAKE-WV-0001" }] },
    "luna://com.webos.service.update/getStatus": { returnValue: true, status: "idle" },
    "luna://com.webos.service.sm/deviceid/getIDs": function(p) {
        var want = p && p.idType ? p.idType : ["IFAID"], out = [];
        for (var i = 0; i < want.length; i++) {
            if (want[i] === "IFAID") out.push({ idType: "IFAID", idValue: ifaId() });
            else if (want[i] === "LGUDID") out.push({ idType: "LGUDID", idValue: "FAKE-LGUDID-0001" });
        }
        return { returnValue: true, idList: out };
    },
    "luna://com.webos.notification/createPincodePrompt": { returnValue: true, matched: true }
};
