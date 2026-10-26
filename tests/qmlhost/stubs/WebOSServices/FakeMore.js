.pragma library
.import "LgDesc.js" as Desc

// What the offline TV answers for the pages that finish the coverage of LG's Settings (docs/research/settings-coverage.md): language and
// location, support, safety, channels and the one-off rows. Same shape as FakeCoverage.js: `seed` (one plausible value per key, from the lists
// in LgDesc.js, the TV's own), `hidden` (keys the TV's description marks not visible), `act` (a call that changes the TV: null, or the reason it
// was refused) and `services` ("service/method" to the reply, or a function of the parameters).
var seed = {
    option: { autoSmartServiceCountry: "on", zipcode: "94105" }
};
var hidden = {};

function apply(data) {
    for (var c in seed) {
        if (!data[c]) data[c] = {};
        for (var k in seed[c]) if (data[c][k] === undefined) data[c][k] = seed[c][k];
    }
}

// the calls the pages make that change the TV, recorded for a test to read
var calls = [];

function act(path, params) {
    if (path === "luna://com.webos.service.sdx/setCountrySettingByAuto") {
        calls.push({ path: path, params: params });
        return params && params.uniqueIndex ? null : "no uniqueIndex";
    }
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
    "luna://com.webos.service.sdx/setCountrySettingByAuto": { returnValue: true, serverResponse: { code: "200" } }
};
