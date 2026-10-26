import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Support (LG: Support: TV information, Software Update, Screen and Sound Self-diagnosis, Memory Optimizer, Privacy & Terms, Reset to Initial
// Settings). Everything LG's TV Information shows is read from the TV's own services, one call each, and shown as LG shows it (no row has a
// value it was not given: an unanswered one is an en dash). The software update is shown, not run: its state is read (`update getStatus`) and the
// version; checking for an update sends the TV's identity to LG and installing it restarts the TV, which a person does in LG's own screen.
// Auto Update writes `support.automaticUpdate`; turning it on needs LG's terms (`eulaStatus.generalTermsAllowed`), which LG's own flow asks for in
// its sign-off app, so here it only turns off, or on once the terms were accepted. Privacy: Do Not Sell goes through the ad service as LG's row does,
// the AD ID is shown and reset, Limit AD Tracking is a switch. The self-diagnosis is LG's Customer Support app at its test (the app, not Settings).
// Reset is last and asks twice, with LG's own PIN prompt between (`createPincodePrompt`; this page acts on its yes or no and never sees the digits);
// the call is LG's `devicereset doReset {resetType}`. Nothing on this page is offered to an assistant: it is a Settings page, and Settings has no
// AI access tool.
PageList {
    id: page
    title: "Support"
    back: "System"
    tabs: ["TV info", "Update", "Privacy", "Tools", "Reset"]

    property string productId: ""
    property string serial: ""
    property string osVersion: ""
    property real storeTotal: -1
    property real storeFree: -1
    property string wiredMac: ""
    property string wifiMac: ""
    property string dolby: ""
    property string esn: ""
    property string widevine: ""
    property string updateState: ""
    property string adId: ""
    property bool recentsCleared: false
    property string pending: ""             // "user" or "userKeepNetwork" while a reset waits for its PIN and second answer

    readonly property string dash: "\u2013"
    readonly property string model: productId === "" ? "" : productId.split(".")[0]
    readonly property string webos: osVersion === "" ? "" : "webOS" + (Number(osVersion.split(".")[0]) + 15) + " / " + osVersion
    readonly property string storage: storeTotal < 0 ? "" : (storeFree / 1024).toFixed(2) + "/" + (storeTotal / 1024).toFixed(2) + "GB"
    readonly property bool termsOk: { var e = Sio.get("eulaStatus"); return !!(e && e.generalTermsAllowed) }
    readonly property var updateNames: ({ "in progress": "Update in progress", "ExternalChipInstall": "Installing", "check update": "Checking for updates", "pending": "Update paused",
                                          "paused": "Update paused", "warm reserved": "Update paused", "schedule detected": "Update reserved", "failed": "Update failed",
                                          "canceled": "Update canceled", "completed": "Installed; applies when the TV restarts" })

    LunaCall {
        id: luna
        onAnswered: {
            if (!doc) return
            if (tag === "pin") { page.pinAnswered(doc); return }
            if (doc.returnValue === false) return
            if (tag === "device") { page.productId = String(doc.product_id || ""); page.serial = String(doc.serial_number || "") }
            else if (tag === "os") page.osVersion = String(doc.core_os_release || "")
            else if (tag === "storage") { if (typeof doc.totalSpace === "number") { page.storeTotal = doc.totalSpace; page.storeFree = doc.freeSpace } }
            else if (tag === "net") { page.wiredMac = String((doc.wiredInfo && doc.wiredInfo.macAddress) || ""); page.wifiMac = String((doc.wifiInfo && doc.wifiInfo.macAddress) || "") }
            else if (tag === "dolby") page.dolby = String(doc.dolbySWVersion || "")
            else if (tag === "keys") {
                var l = doc.keyStatusList || []
                for (var i = 0; i < l.length; i++) {
                    if (!l[i].isOk || l[i].identifier === undefined) continue
                    if (l[i].id === "esn") page.esn = String(l[i].identifier)
                    else if (l[i].id === "widevine") page.widevine = String(l[i].identifier)
                }
            }
            else if (tag === "update") page.updateState = String(doc.status || "")
            else if (tag === "ifa") {
                var ids = doc.idList || []
                for (var k = 0; k < ids.length; k++) if (ids[k].idType === "IFAID") page.adId = String(ids[k].idValue)
            }
        }
    }
    function readAdId() { luna.call("ifa", "luna://com.webos.service.sm/deviceid", "getIDs", { idType: ["IFAID"] }) }
    function refresh() {
        Sio.read("", ["eulaStatus"])
        Sio.read("support", ["automaticUpdate"])
        Sio.read("general", ["doNotSellMyPersonalInformation", "lmt"])
        Sio.read("network", ["deviceName"])
        Sio.read("option", ["vsn"])
        Power.readPanel()
        luna.call("device", "luna://com.palm.systemservice/deviceInfo", "query", { parameters: ["product_id", "serial_number"] })
        luna.call("os", "luna://com.palm.systemservice/osInfo", "query", { parameters: ["core_os_release", "core_os_release_codename"] })
        luna.call("storage", "luna://com.webos.service.attachedstoragemanager", "getProperties", { deviceId: "INTERNAL_STORAGE" })
        luna.call("net", "luna://com.webos.service.connectionmanager", "getinfo", {})
        luna.call("dolby", "luna://com.webos.service.pqcontroller", "getInfo", { key: "dolbySWVersion" })
        luna.call("keys", "luna://com.webos.service.sm/crypto", "checkSeStore", { id: ["esn", "widevine"] })
        luna.call("update", "luna://com.webos.service.update", "getStatus", {})
        readAdId()
        pending = ""
    }
    Component.onCompleted: refresh()
    onOpened: refresh()
    Timer { id: reread; interval: 700; onTriggered: page.refresh() }

    // the TV's information: a row that shows, and can still be reached by the D-pad so the list scrolls
    function info(icon, label, value, extra) {
        var row = { kind: "value", tab: 0, id: "i:" + label, icon: icon, label: label, chevron: false,
                    value: function() { var v = typeof value === "function" ? value() : value; return v === "" || v === undefined ? page.dash : String(v) } }
        for (var k in (extra || {})) row[k] = extra[k]
        return row
    }

    // ---- Reset: LG's PIN prompt between the two answers; nothing is sent before the second yes
    // The PIN prompt is asked a moment after the first question has closed: a prompt that answered at once would otherwise open the second question
    // in the same instant the host closes the first, and lose it.
    function startReset(kind) { pending = kind; pinSoon.restart() }
    Timer { id: pinSoon; interval: 150; onTriggered: luna.call("pin", "luna://com.webos.notification", "createPincodePrompt", { promptType: "parental" }) }
    function pinAnswered(doc) {
        var kind = pending
        pending = ""
        if (kind === "" || doc.returnValue === false || doc.matched !== true) return
        C.ask(page, { icon: "triangle-alert", title: "Reset the TV now?", body: kind === "user" ? "Everything is erased. The TV restarts." : "Everything but the network settings is erased. The TV restarts.",
                      yes: "Reset now", no: "Cancel", danger: true }, function() { luna.act("luna://com.webos.service.devicereset", "doReset", { resetType: kind }) })()
    }
    function resetRow(id, label, kind, help) {
        return C.action({ id: id, tab: 4, icon: "rotate-ccw", label: label, button: "Reset", danger: true, help: help,
                          act: C.ask(page, { icon: "rotate-ccw", title: label + "?", body: "The TV asks for the PIN next, then once more.", yes: "Continue", no: "Cancel", danger: true },
                                     function() { page.startReset(kind) }) })
    }

    rows: [
        info("tv", "Device Name", function() { return Sio.has("network.deviceName") ? Sio.get("network.deviceName") : "" }),
        info("tv", "Model", function() { return page.model }),
        info("hash", "Model Code", function() { return page.productId }),
        info("hash", "Serial Number", function() { return page.serial }),
        info("settings", "webOS TV Version", function() { return page.webos }),
        info("clock", "Total Power On Time", function() { return Power.panelHours }, { visible: function() { return Power.panelShown } }),
        info("hard-drive", "Available Internal Storage", function() { return page.storage }),
        info("ethernet-port", "MAC Address (Wired)", function() { return page.wiredMac }, { visible: function() { return page.wiredMac !== "" } }),
        info("wifi", "MAC Address (Wireless)", function() { return page.wifiMac }),
        info("key-round", "ESN", function() { return page.esn }),
        info("hash", "VSN", function() { return Sio.has("option.vsn") ? Sio.get("option.vsn") : "" }, { visible: function() { return C.shown(Sio, "option.vsn") } }),
        info("shield-check", "Widevine", function() { return page.widevine }),
        info("film", "Dolby Vision", function() { return page.dolby }),

        info("settings", "Software Version", function() { return page.webos }, { tab: 1 }),
        { kind: "value", tab: 1, id: "updatestate", icon: "download", label: "Update status", chevron: false,
          value: function() { return page.updateState === "" ? page.dash : R.name(page.updateNames, page.updateState, "No update running") } },
        R.over(R.toggle(Sio, { ds: "support.automaticUpdate", tab: 1, icon: "refresh-cw", label: "Auto Update",
                               help: "The TV installs software updates by itself, with no notice. Turning it on needs LG's terms, which are accepted in LG's settings." }),
               { enabled: function() { return Sio.usable("support.automaticUpdate") && (String(Sio.get("support.automaticUpdate")) === "on" || page.termsOk) },
                 detail: function() { return !Sio.has("support.automaticUpdate") ? "TV not answering" : String(Sio.get("support.automaticUpdate")) !== "on" && !page.termsOk ? "Needs LG's terms" : "" } }),
        R.lg(page, { tab: 1, id: "checkupdate", target: "SoftwareUpdate", label: "Check for Updates", help: "Checking sends the TV's model and version to LG, and an install restarts the TV. Both are done in LG's screen." }),

        R.over(R.toggle(Sio, { ds: "general.doNotSellMyPersonalInformation", tab: 2, icon: "shield", label: "Do Not Sell My Personal Information",
                               help: "Tells LG to ask its partners to stop selling data about this TV's use. Live Plus and LG Channels keep working." }),
               { act: function(v) {
                     C.ask(page, { icon: "shield", title: v ? "Stop the sale of this TV's data?" : "Allow the sale of this TV's data?", body: "LG's ad service is told now.", yes: v ? "Turn on" : "Turn off", no: "Cancel" },
                           function() { luna.act("luna://com.webos.service.admanager", "updateDNS", { status: v ? "on" : "off" }); reread.restart() })()
                 } }),
        R.toggle(Sio, { ds: "general.lmt", tab: 2, icon: "eye-off", label: "Limit AD Tracking", help: "Stops the tracking of ad-related data from this TV." }),
        info("hash", "AD ID", function() { return page.adId }, { tab: 2 }),
        C.action({ id: "resetad", tab: 2, icon: "rotate-ccw", label: "Reset AD ID", button: "Reset", help: "Gives the TV a new advertising ID, and ads are tracked from the new one.",
                   act: C.ask(page, { icon: "rotate-ccw", title: "Reset the AD ID?", body: "The TV gets a new advertising ID.", yes: "Reset", no: "Cancel" },
                              function() { luna.act("luna://com.webos.service.admanager", "resetIFA", {}); page.adId = ""; reread.restart() }) }),

        R.lg(page, { tab: 3, id: "screentest", app: "com.webos.app.customersupport", target: "pictureTest", label: "Screen Self-diagnosis", help: "LG's picture test, in LG's Customer Support app." }),
        R.lg(page, { tab: 3, id: "soundtest", app: "com.webos.app.customersupport", target: "soundTest", label: "Sound Self-diagnosis", help: "LG's test tone, in LG's Customer Support app. The TV speaker must be the sound out." }),
        C.action({ id: "memory", tab: 3, icon: "cpu", label: "Memory Optimizer", button: function() { return page.recentsCleared ? "Done" : "Run" },
                   help: "Clears LG's list of recent apps to free memory.",
                   act: C.ask(page, { icon: "cpu", title: "Clear LG's recent apps?", body: "The list of recent apps is emptied.", yes: "Clear", no: "Cancel" },
                              function() { luna.act("luna://com.webos.service.recent", "removeAllRecents", {}); page.recentsCleared = true }) }),

        resetRow("resetall", "Reset All", "user", "Erases the TV's settings, accounts and apps, as new. It asks for the PIN, twice over."),
        resetRow("resetkeep", "Reset All Except Network Settings", "userKeepNetwork", "As Reset All, but the Wi-Fi and the wired settings stay.")
    ]
}
