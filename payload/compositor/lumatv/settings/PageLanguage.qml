import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "Names.js" as N
import "."

// Language & location (LG: General › System › Language and Location). LG keeps every language in one setting, `localeInfo`, read and
// written with no category: `locales` holds the menu language (UI, with FMT and TV) and the audio language (AUD), `keyboards` the on-screen
// keyboard languages. The lists to choose from come from the settings service (`getSystemSettingValues {key: "locale"}`: `locale`, `audio`
// and `vkb`, each a list of languages with their countries). LG's menu language row writes UI, FMT and TV together and sets the keyboards
// to English and the language's own; its keyboards panel adds and removes a keyboard, English and the one of the menu language staying.
// Location: the broadcast country only shows (changing it resets the TV and its channels); the LG services country is a switch
// (`option.autoSmartServiceCountry`) that asks the TV's sdx service to find the country by itself; the zip code is `option.zipcode`.
// Changing the menu language reloads LG's menus, so it asks first.
PageList {
    id: page
    title: "Language & location"
    back: "System"
    tabs: ["Language", "Keyboards", "Location"]

    property var lists: ({ locale: [], audio: [], vkb: [] })
    property bool zipBad: false
    readonly property var info: Sio.get("localeInfo") || ({})
    readonly property var locales: info.locales || ({})
    readonly property var keyboards: info.keyboards || []
    readonly property string ui: String(locales.UI || "")
    readonly property var countries: ({ USA: "United States", CAN: "Canada", MEX: "Mexico", GBR: "United Kingdom", AUS: "Australia", NZL: "New Zealand", DEU: "Germany",
                                        FRA: "France", ESP: "Spain", ITA: "Italy", JPN: "Japan", KOR: "South Korea", BRA: "Brazil", IND: "India" })

    LunaCall {
        id: luna
        onAnswered: {
            if (tag === "locale" && doc && doc.returnValue !== false) page.lists = { locale: doc.locale || [], audio: doc.audio || [], vkb: doc.vkb || [] }
            else if (tag === "country") {
                // LG sets the setting first and turns it back off when the TV's service does not find a country
                var ok = !!(doc && doc.returnValue !== false && (!doc.serverResponse || String(doc.serverResponse.code) === "200"))
                if (!ok) Sio.write("option.autoSmartServiceCountry", "off")
            }
        }
    }
    function refresh() {
        Sio.read("", ["localeInfo"])
        Sio.read("option", ["country", "autoSmartServiceCountry", "zipcode"])
        luna.call("locale", Sio.service, "getSystemSettingValues", { key: "locale" })
        zipBad = false
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    // [{value, label}] from a list of languages with their countries (LG's `ZM`): "English - United States", or the language alone
    function flatten(list) {
        var out = []
        for (var i = 0; i < (list || []).length; i++) {
            var l = list[i]
            if (l.countries && l.countries.length) {
                for (var j = 0; j < l.countries.length; j++) out.push({ value: String(l.countries[j].spec), label: String(l.name) + (l.countries[j].name ? " - " + l.countries[j].name : "") })
            } else out.push({ value: String(l.languageCode || l.spec), label: String(l.name) })
        }
        return out
    }
    readonly property var menuOptions: flatten(lists.locale)
    readonly property var audioOptions: flatten(lists.audio)
    function labelFor(opts, value) {
        for (var i = 0; i < opts.length; i++) if (opts[i].value === String(value)) return opts[i].label
        return String(value || "")
    }

    // what is written: a copy of localeInfo with the locales and keyboards LG's own rows set
    function setLocale(patch, keyboardList) {
        var copy = JSON.parse(JSON.stringify(info))
        if (!copy.locales) copy.locales = {}
        for (var k in patch) copy.locales[k] = patch[k]
        if (keyboardList) copy.keyboards = keyboardList
        Sio.writeMany("", { localeInfo: copy })
    }
    // the menu language: the keyboard of its own language goes with it (the Indic languages use the Hindi one), English always
    function setMenu(spec) {
        var key = ["as", "bn", "gu", "kn", "ml", "mr", "pa", "ta", "te", "ur"].indexOf(spec) >= 0 ? "hi" : spec
        var own = "en"
        for (var i = 0; i < lists.vkb.length; i++) {
            var s = String(lists.vkb[i].languageCode || lists.vkb[i].spec)
            if (key.indexOf(s) > -1) { own = s; break }
        }
        setLocale({ UI: spec, FMT: spec, TV: spec }, own === "en" ? ["en"] : ["en", own])
    }
    // an on-screen keyboard that stays: English and the menu language's own (LG marks them Deactivated)
    function locked(spec) { return spec === "en" || (ui !== "" && ui.indexOf(spec) > -1) }
    function toggleKeyboard(spec, on) {
        var list = keyboards.slice()
        var at = list.indexOf(spec)
        if (on && at < 0) list.push(spec)
        else if (!on && at >= 0) list.splice(at, 1)
        setLocale({}, list)
    }

    // The two language rows have the shape of a choice row (a value, its options, the one in force, and a pick), so the one chooser page lists them.
    function langRow(id, label, options, current, pick) {
        var row = {
            kind: "value", tab: 0, id: id, icon: "languages", label: label, chevron: true,
            enabled: function() { return options().length > 0 && Sio.has("localeInfo") },
            detail: function() { return !Sio.has("localeInfo") ? "TV not answering" : options().length === 0 ? "No list from the TV" : "" },
            value: function() { return page.labelFor(options(), current()) },
            options: function() { return options().map(function(o) { return { value: o.value, label: o.label, enabled: true } }) },
            current: current,
            pick: function(i) { var o = options()[i]; if (o && o.value !== current()) pick(o.value) },
            act: function() { page.host.openPage("choice", { row: row, back: page.title, backId: "language" }) }
        }
        return row
    }
    function build() {
        var out = []
        var menu = langRow("menu", "Menu Language", function() { return page.menuOptions }, function() { return page.ui }, function(v) {
            C.ask(page, { icon: "languages", title: "Change to " + page.labelFor(page.menuOptions, v) + "?", body: "LG's menus reload in the new language.", yes: "Change", no: "Cancel" },
                  function() { page.setMenu(v) })()
        })
        menu.help = "The language of LG's menus and apps. LumaTV's own text stays English."
        out.push(menu)
        out.push(langRow("audio", "Audio Language", function() { return page.audioOptions }, function() { return String(page.locales.AUD || "") },
                         function(v) { page.setLocale({ AUD: v }, null) }))
        var kb = lists.vkb
        for (var i = 0; i < kb.length; i++) out.push(keyboardRow(String(kb[i].languageCode || kb[i].spec), String(kb[i].name || kb[i].displayValue)))

        out.push({ kind: "value", tab: 2, id: "country", icon: "globe", label: "Broadcast Country", chevron: false, info: true,
                   help: "Changing it resets the TV and erases its channels. That is done in LG's settings.",
                   visible: function() { return Sio.has("option.country") },
                   value: function() { return page.countries[String(Sio.get("option.country"))] || String(Sio.get("option.country")) } })
        out.push(C.lgShown(Sio, "option.autoSmartServiceCountry", {
            kind: "toggle", tab: 2, id: "svcCountry", icon: "globe-2", label: "LG Services Country", help: "On: the TV finds the country of its LG services itself. Changing it restarts those services.",
            detail: R.unavailable(Sio, "option.autoSmartServiceCountry"), enabled: function() { return Sio.usable("option.autoSmartServiceCountry") },
            on: function() { return String(Sio.get("option.autoSmartServiceCountry")) === "on" },
            act: function(v) {
                C.ask(page, { icon: "globe", title: v ? "Find the country automatically?" : "Stop finding the country automatically?", body: "LG's services restart.", yes: v ? "Turn on" : "Turn off", no: "Cancel" },
                      function() {
                          Sio.write("option.autoSmartServiceCountry", v ? "on" : "off")
                          if (v) luna.call("country", "luna://com.webos.service.sdx", "setCountrySettingByAuto", { uniqueIndex: "2013" })
                      })()
            } }))
        out.push(C.lgShown(Sio, "option.zipcode", {
            kind: "value", tab: 2, id: "zip", icon: "hash", label: "Zip Code", chevron: true, help: "Five digits, typed on the keyboard. LG's services use it for the weather and the local news.",
            detail: function() { return !Sio.has("option.zipcode") ? "TV not answering" : page.zipBad ? "Needs 5 digits" : "" },
            enabled: function() { return Sio.usable("option.zipcode") },
            value: function() { return Sio.has("option.zipcode") ? String(Sio.get("option.zipcode")) : "" },
            act: function() {
                page.zipBad = false
                page.host.entry({ prompt: "Zip code", text: String(Sio.get("option.zipcode") || "") }, function(t) {
                    var z = String(t).trim()
                    if (/^[0-9]{5}$/.test(z)) Sio.write("option.zipcode", z)
                    else page.zipBad = true
                })
            } }))
        return out
    }
    function keyboardRow(spec, name) {
        return { kind: "toggle", tab: 1, id: "kb:" + spec, icon: "keyboard", label: name,
                 detail: function() { return page.locked(spec) ? "In use" : "" },
                 enabled: function() { return !page.locked(spec) && Sio.has("localeInfo") },
                 on: function() { return page.keyboards.indexOf(spec) >= 0 },
                 act: function(v) { page.toggleKeyboard(spec, v) } }
    }
    rows: build()
}
