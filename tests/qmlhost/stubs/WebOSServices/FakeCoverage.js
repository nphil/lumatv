.pragma library
.import "LgDesc.js" as Desc

// What the offline TV answers for the keys and luna methods the coverage pages (docs/research/settings-coverage.md) read: one
// plausible value per key, from the lists in LgDesc.js (the TV's own), and replies for the services LG's Settings calls. FakeSettings
// merges the seeds under the values it already holds, so no earlier test sees a key it did not have before.
var seed = {
    caption: { captionEnable: "off", captionAnalog: "cc1", captionDigital: "svc1", digitalPlusCaptionLanguage: "eng", captionStyle: "auto",
               captionSize: "standard", captionFont: "font0", captionTextColor: "white", captionTextOpacity: "solid", captionBgColor: "black",
               captionBgOpacity: "solid", captionEdgeType: "none", captionEdgeColor: "black", captionWindowColor: "black",
               captionWindowOpacity: "solid", captionDigitalPosOption: "auto", caption608PosOption: "auto", hardOfHearing: "off",
               aiCaptionEnable: "off" },
    option: { audioGuidance: "off", audioGuidanceSpeed: "fast", audioGuidanceVolume: "medium", audioGuidancePitch: "medium",
              screenMagnification: "off", highContrast: "off", focusEmphasis: "off", adjustments: { colorInversion: "off", blackAndWhite: "off" },
              supplementaryAudio: "off", additionalAudioSelection: "none", subtitleLanguageFirst: "eng", subtitleLanguageSecond: "eng" },
    sound: { deviceCntOfBtGeneral: "singleConnection", audioDescription: "off", audioDescriptionAtsc30: "off", audioDescriptionVolume: "10", visualImpaired: "off", hearingAid: "off", dialogEnhancementAtsc30: "off",
             dialogEnhancementVolumeAtsc30: "3", tvPowerSound: "on",
             applyToAllInputSSM: "done", avSyncBypassInput: "off", supportATMOS: "on", userEqualizer: "on", userEqualizerValues: [0, 0, 0, 0, 0], soundModeModified: "false",
             tvSetupConfiguration: "stand", digitalAudioPriority: "auto", lgSoundSync: "off", soundAlive: "off", magicSpaceSound: "off",
             inputAudioFormatHDMI1: "bitstream", inputAudioFormatHDMI2: "bitstream", inputAudioFormatHDMI3: "pcm", inputAudioFormatHDMI4: "pcm" },
    general: { tvOnSound: "medium" },
    other: { autoSLZoomWithObjectDetection: "off", qmsVrr: "on", uhdDeepColor: "4k", "444BypassHDMI1": "off", "444BypassHDMI2": "off", "444BypassHDMI3": "off",
             "444BypassHDMI4": "off", enableDolbyVisionPC: "off" },
    network: { ipv6Enable: "true", bleAdvertisingOnOff: "on" },
    picture: { dynamicContrast: "off", peakBrightness: "off", gamma: "high2", nearBlackDetail: "0", motionEyeCare: "off", colorTemperature: "0", colorGamut: "auto",
               dynamicColor: "off", superResolution: "off", noiseReduction: "off", mpegNoiseReduction: "off", smoothGradation: "off", realCinema: "on",
               dolbyPrecisionDetail: "off", truMotionJudder: "0", truMotionBlur: "10", motionPro: "off", hdrDynamicToneMapping: "on",
               expressionEnhancer: "off", applyToAllInput: "done" },
    time: { autoClock: "on", autoOff2HourOnTimer: "on", automaticStandBy: "0", onTimerEnable: "off", onTimerHour: "7", onTimerMinute: "30", onTimerWeekday: "0",
            onTimerSource: "hdmi1", onTimerVolume: "10", offTimerEnable: "off", offTimerHour: "23", offTimerMinute: "0", offTimerWeekday: "0" }
};
seed.option.screenOff = "off";

// Keys the TV's description marks not visible, as it said on 2026-10-25 (LG leaves such a row out): the offline description says so too.
var hidden = { "caption.aiCaptionEnable": true, "option.signLanguageGuidance": true, "time.automaticStandBy": true,
               "picture.nearBlackDetail": true, "picture.expressionEnhancer": true, "picture.dolbyPrecisionDetail": true, "picture.hdrDynamicToneMapping": true,
               "sound.eArcSupport": true, "sound.inputAudioFormatDP1": true, "sound.inputAudioFormatDP2": true, "sound.inputAudioFormatUSBC1": true, "sound.inputAudioFormatUSBC2": true };

function apply(data) {
    for (var c in seed) {
        if (!data[c]) data[c] = {};
        for (var k in seed[c]) if (data[c][k] === undefined) data[c][k] = seed[c][k];
    }
}

// ---- the clock and the zone (com.palm.systemservice): what the TV answers, and what a write changes
var zones = [
    { ZoneID: "America/New_York", City: "New York", Country: "United States of America", CountryCode: "US", Description: "Eastern Time", offsetFromUTC: -300 },
    { ZoneID: "America/Chicago", City: "Chicago", Country: "United States of America", CountryCode: "US", Description: "Central Time", offsetFromUTC: -360 },
    { ZoneID: "America/Denver", City: "Denver", Country: "United States of America", CountryCode: "US", Description: "Mountain Time", offsetFromUTC: -420 },
    { ZoneID: "America/Los_Angeles", City: "Los Angeles", Country: "United States of America", CountryCode: "US", Description: "Pacific Time", offsetFromUTC: -480 },
    { ZoneID: "Europe/London", City: "London", Country: "United Kingdom", CountryCode: "GB", Description: "GMT", offsetFromUTC: 0 }
];
var clock = { utc: 1761501000, auto: true, zone: zones[0] };     // 2025-10-26 17:30:00 UTC

// The Homebrew Channel's service (org.webosbrew.hbchannel.service): its flags, of which Settings reads and sets `blockUpdates` (the block on LG's
// system updates). `answers` false makes it say what the bus says of a service that is not there.
var hbc = { answers: true, root: true, blockUpdates: true, telnetDisabled: false, sshdEnabled: false, failsafe: false };
var hbcSets = [];

// What a reset was asked for, as {category, dimension} (a picture mode) or {category, keys}: the service wants the input and the mode it is to
// reset, or the keys.
var resets = [];

// A call that changes the TV (what the stub's `adhoc.call` hands over, as "uri + method" and its parameters): null, or the reason it was refused.
function act(path, params) {
    if (path === "luna://com.webos.service.settings/resetSystemSettings") {
        if (params.dimension) {
            if (params.category !== "picture") return "unknown category";
            if (typeof params.dimension.input !== "string" || typeof params.dimension.pictureMode !== "string") return "no dimension";
        } else {
            if (!params.category || !Array.isArray(params.keys) || !params.keys.length) return "no keys";
            for (var i = 0; i < params.keys.length; i++) if (!Desc.d[params.category + "." + params.keys[i]]) return "unknown key " + params.category + "." + params.keys[i];
        }
        resets.push(params);
        return null;
    }
    if (path === "luna://com.palm.systemservice/time/setSystemTime") {
        if (typeof params.utc !== "number") return "utc is not a number";
        clock.utc = params.utc;
    } else if (path === "luna://com.palm.systemservice/setPreferences") {
        if (params.useNetworkTime !== undefined) clock.auto = !!params.useNetworkTime;
        if (params.timeZone) clock.zone = params.timeZone;
    } else if (path === "luna://org.webosbrew.hbchannel.service/__answers") {
        hbc.answers = params.on !== false;          // a test door, not a method of the service
    } else if (path === "luna://org.webosbrew.hbchannel.service/setConfiguration") {
        if (!hbc.answers) return "Service does not exist: org.webosbrew.hbchannel.service";
        for (var f in params) if (typeof params[f] !== "boolean") return "Luna Error: " + f + " is not a boolean";
        hbcSets.push(params);
        if (params.blockUpdates !== undefined) hbc.blockUpdates = params.blockUpdates;
    } else if (path.indexOf("luna://com.webos.service.wifi/") === 0) {
        return netAct(path.substring("luna://com.webos.service.wifi/".length), params);
    } else if (path.indexOf("luna://com.webos.service.connectionmanager/") === 0) {
        return netAct(path.substring("luna://com.webos.service.connectionmanager/".length), params);
    } else if (path.indexOf("luna://com.webos.service.btaudiosrc/") === 0) {
        return btAudio(path.substring("luna://com.webos.service.btaudiosrc/".length), params);
    } else if (path.indexOf("luna://com.webos.service.bthidmanager/") === 0) {
        return btHid(path.substring("luna://com.webos.service.bthidmanager/".length), params);
    }
    return null;
}

// ---- Bluetooth (btaudiosrc for speakers, bthidmanager for controllers): what is paired, what can be found, and what a call changes
var bt = {
    audio: { paired: [{ address: "AA:BB:CC:00:00:01", name: "Living Room Headphones", state: "disconnected", display: true, surround: "none" }],
             found: [{ address: "AA:BB:CC:00:00:02", name: "Kitchen Speaker" }] },
    hid: { paired: [{ deviceAddr: "11:22:33:00:00:01", isPaired: true, deviceName: "Wireless Controller", deviceType: "gamepad", isConnected: false }],
           found: [{ address: "11:22:33:00:00:02", name: "Keyboard K380" }] }
};
// The speakers' volume and balance (btaudiosrc/surround): 0 to 10, 5 the middle of the balance.
var balance = { tvspeaker_bluetooth: 5, bluetooth_left_right: 5 };
var positions = ["left", "right", "front", "rear", "center", "none"];
// Test variants of the paired speakers, put in place by `btaudiosrc/fake/speakers {set}` (not a call of the real service): "matching" is two connected
// speakers of one model that both answer over AVRCP, as the TV lists a stereo pair; "different" is two of two models; "avrcpOff" is a pair whose
// second speaker is not on AVRCP; "default" is the paired list the pages start with.
var heldPaired = null;
function speaker(addr, name, surround, avrcp) {
    return { address: addr, name: name, state: "connected", display: true, surround: surround, avrcpSupported: true, avrcpConnected: avrcp };
}
function fakeSpeakers(set) {
    var a = bt.audio;
    if (heldPaired === null) heldPaired = a.paired;
    if (set === "default") { a.paired = heldPaired; heldPaired = null; balance.bluetooth_left_right = 5; return null; }
    if (set === "matching") a.paired = [speaker("AA:BB:CC:00:00:11", "Studio Speaker (L)", "left", true), speaker("AA:BB:CC:00:00:12", "Studio Speaker (R)", "right", true)];
    else if (set === "different") a.paired = [speaker("AA:BB:CC:00:00:11", "Studio Speaker (L)", "left", true), speaker("AA:BB:CC:00:00:13", "Garage Boombox", "none", true)];
    else if (set === "avrcpOff") a.paired = [speaker("AA:BB:CC:00:00:11", "Studio Speaker (L)", "left", true), speaker("AA:BB:CC:00:00:12", "Studio Speaker (R)", "right", false)];
    else return "unknown variant";
    return null;
}
// `setChannel {address, from, to}`: `from` is the position the speaker has, `to` one of the six; a speaker that holds `to` takes `from`.
function setChannel(p) {
    var list = bt.audio.paired, me = null, k;
    for (k = 0; k < list.length; k++) if (list[k].address === p.address) me = list[k];
    if (!me) return "unknown device";
    if (me.state !== "connected") return "not connected";
    if (positions.indexOf(p.to) < 0) return "unknown position";
    if (p.from !== me.surround) return "from is not the speaker's position";
    for (k = 0; k < list.length; k++) if (list[k] !== me && list[k].state === "connected" && list[k].surround === p.to && p.to !== "none") list[k].surround = p.from;
    me.surround = p.to;
    return null;
}
function btAudio(path, p) {
    var a = bt.audio, i;
    var at = function(list, addr) { for (var k = 0; k < list.length; k++) if (list[k].address === addr) return k; return -1; };
    if (path === "fake/speakers") return fakeSpeakers(p.set);
    if (path === "surround/setVolumeBalancing") {
        var keys = Object.keys(p);
        if (keys.length !== 1 || balance[keys[0]] === undefined) return "unknown key";
        if (typeof p[keys[0]] !== "number" || p[keys[0]] < 0 || p[keys[0]] > 10 || p[keys[0]] % 1 !== 0) return "value out of range";
        balance[keys[0]] = p[keys[0]];
        return null;
    }
    if (path === "setChannel") return setChannel(p);
    if (path === "connect") {
        i = at(a.paired, p.address);
        if (i < 0) {
            var f = at(a.found, p.address);
            if (f < 0) return "unknown device";
            a.paired.push({ address: a.found[f].address, name: a.found[f].name, state: "connected", display: true, surround: "none" });
            a.found.splice(f, 1);
        } else a.paired[i].state = "connected";
    } else if (path === "disconnect" || path === "removePairedDevice") {
        i = at(a.paired, p.address);
        if (i < 0) return "unknown device";
        if (path === "disconnect") a.paired[i].state = "disconnected"; else a.paired.splice(i, 1);
    }
    return null;
}
function btHid(path, p) {
    var h = bt.hid, i;
    var at = function(addr) { for (var k = 0; k < h.paired.length; k++) if (h.paired[k].deviceAddr === addr) return k; return -1; };
    if (path === "requestConnect") {
        i = at(p.address);
        if (i < 0) {
            h.paired.push({ deviceAddr: p.address, isPaired: true, deviceName: p.name, deviceType: "keyboard", isConnected: true });
            h.found = h.found.filter(function(x) { return x.address !== p.address; });
        } else h.paired[i].isConnected = true;
    } else if (path === "requestDisconnect" || path === "requestDelete") {
        i = at(p.deviceAddr);
        if (i < 0) return "unknown device";
        if (path === "requestDisconnect") h.paired[i].isConnected = false; else h.paired.splice(i, 1);
    }
    return null;
}

// ---- the network: the Wi-Fi the TV can see, the link it is on, and what a join or an address change does to it
var link = { ssid: "Nate's Home", method: "dhcp", address: "192.168.1.146", netmask: "255.255.255.0", gateway: "192.168.1.1", dns1: "192.168.1.1" };
var networks = [
    { networkInfo: { ssid: "Nate's Home", availableSecurityTypes: ["psk"], signalBars: 4 } },
    { networkInfo: { ssid: "Guest", availableSecurityTypes: [], signalBars: 2 } },
    { networkInfo: { ssid: "Neighbour 5G", availableSecurityTypes: ["psk", "wps"], signalBars: 1 } }
];
function ipv4(a) {
    var m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(String(a));
    return !!m && m[1] <= 255 && m[2] <= 255 && m[3] <= 255 && m[4] <= 255;
}
function netAct(path, p) {
    if (path === "connect") {
        if (typeof p.ssid !== "string" || p.ssid === "") return "no ssid";
        var known = false;
        for (var i = 0; i < networks.length; i++) if (networks[i].networkInfo.ssid === p.ssid) known = networks[i].networkInfo;
        if (known && known.availableSecurityTypes.length && !p.security) return "password needed";
        if (p.security && (p.security.securityType !== "psk" || !p.security.simpleSecurity || String(p.security.simpleSecurity.passKey).length < 8)) return "invalid password";
        link.ssid = p.ssid;
    } else if (path === "setipv4") {
        if (p.method !== "dhcp" && p.method !== "manual") return "bad method";
        if (p.method === "manual") {
            if (!ipv4(p.address) || !ipv4(p.netmask) || !ipv4(p.gateway)) return "bad address";
            link.address = p.address; link.netmask = p.netmask; link.gateway = p.gateway;
        }
        link.method = p.method;
    } else if (path === "setdns") {
        if (!p.dns || !p.dns.length || !ipv4(p.dns[0])) return "bad dns";
        link.dns1 = p.dns[0];
    }
    return null;
}

// "service/method" -> the reply (or a function of the parameters), for the luna services other than the settings service
// (FakeSettings.services holds the older ones).
var services = {
    "luna://org.webosbrew.hbchannel.service/getConfiguration": function() {
        return hbc.answers ? { returnValue: true, root: hbc.root, homebrewBaseDir: "/media/developer/apps/usr/palm/applications/org.webosbrew.hbchannel", telnetDisabled: hbc.telnetDisabled,
                               sshdEnabled: hbc.sshdEnabled, blockUpdates: hbc.blockUpdates, failsafe: hbc.failsafe }
                           : { returnValue: false, errorText: "Service does not exist: org.webosbrew.hbchannel.service" };
    },
    "luna://org.webosbrew.hbchannel.service/setConfiguration": function() { return { returnValue: true, blockUpdates: hbc.blockUpdates }; },
    "luna://com.palm.systemservice/time/getEffectiveBroadcastTime": function() { return { returnValue: true, adjustedUtc: clock.utc, systemTimeSource: clock.auto ? "ntp" : "user" }; },
    "luna://com.palm.systemservice/getPreferences": function() { return { returnValue: true, useNetworkTime: clock.auto, timeZone: clock.zone }; },
    "luna://com.palm.systemservice/getPreferenceValues": function() { return { returnValue: true, timeZone: zones }; },
    "luna://com.webos.service.wifi/findnetworks": function() { return { returnValue: true, foundNetworks: networks }; },
    "luna://com.webos.service.connectionmanager/getStatus": function() {
        return { returnValue: true, isInternetConnectionAvailable: true, wifi: { state: "connected", ssid: link.ssid, signalBars: 4, signalLevel: -52, channelFrequency: 5180, ipAddress: link.address,
                                            netmask: link.netmask, gateway: link.gateway, dns1: link.dns1, method: link.method }, wired: { state: "disconnected" } };
    },
    "luna://com.webos.service.btaudiosrc/getPairedDevices": function() { return { returnValue: true, subscribed: true, devices: bt.audio.paired }; },
    "luna://com.webos.service.btaudiosrc/startDiscovery": function() {
        return bt.audio.found.length ? { returnValue: true, timeOut: 30, state: "started", device: bt.audio.found[0] } : { returnValue: true, timeOut: 30, state: "stopped" };
    },
    "luna://com.webos.service.btaudiosrc/surround/getVolumeBalancing": function() {
        return { returnValue: true, subscribed: true, tvspeaker_bluetooth: balance.tvspeaker_bluetooth, bluetooth_left_right: balance.bluetooth_left_right };
    },
    "luna://com.webos.service.btaudiosrc/getDeviceStatus": { returnValue: true, subscribed: true, state: "idle" },
    "luna://com.webos.service.bthidmanager/getPairedDeviceLists": function() {
        // the TV's own shape: an array with one entry for each of its four slots, the empty ones "Not Paired" with the address "N/A"
        var l = bt.hid.paired.slice();
        while (l.length < 4) l.push({ deviceAddr: "N/A", isPaired: false, deviceName: "Not Paired", isConnected: false, deviceType: "N/A", pairedTime: "0000/00/00, 00:00:00" });
        return { returnValue: true, deviceLists: l };
    },
    "luna://com.webos.service.bthidmanager/subscribeDevicesStatus": { returnValue: true, subscribed: true, status: "disconnected" },
    "luna://com.webos.service.bthidmanager/subscribeDiscovery": function() {
        return bt.hid.found.length ? { returnValue: true, subscribed: true, address: bt.hid.found[0].address, name: bt.hid.found[0].name } : { returnValue: true, subscribed: true, status: "endDiscovery" };
    }
};
