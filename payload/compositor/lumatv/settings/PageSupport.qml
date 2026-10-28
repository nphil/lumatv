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
// the AD ID is shown and reset, Limit AD Tracking is a switch; Live Plus (LG keeps it under Additional Settings, but it is consent) asks before it turns on and
// holds Live Promotion; App Permissions and Delete My Personal Information are pages of their own. The self-diagnosis is LG's Customer Support app at its test (the app, not Settings).
// Block LG system updates (the Homebrew Channel's own `blockUpdates`, set through its service) keeps LG's next major update from removing root by accident; while it is on, or cannot be read, Check for Updates does nothing.
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
    readonly property bool livePlusOn: String(Sio.get("option.livePlus")) === "on"
    readonly property var updateNames: ({ "in progress": "Update in progress", "ExternalChipInstall": "Installing", "check update": "Checking for updates", "pending": "Update paused",
                                          "paused": "Update paused", "warm reserved": "Update paused", "schedule detected": "Update reserved", "failed": "Update failed",
                                          "canceled": "Update canceled", "completed": "Installed; applies when the TV restarts" })

    // ---- LumaTV recovered from a problem. When LumaTV's compositor was put out of the way (it died, hung, or drew nothing the panel showed), the supervisor
    // handed the screen back to LG's compositor and saved a crash bundle on the TV; the helper's `/v1/crash` tells what the newest one is and whether anybody has
    // read it. Until the owner has, this page says so at its top, and Details tells why and when, and where the log and the trace of its last seconds are. The
    // helper is asked when the page opens (nothing polls); an escape the owner made (Home held) is saved too and is no notice.
    property var crash: null                // {pending, kind, at, reason, uptimeSecs, path, ...} of the newest bundle; null when there is none or the helper does not answer
    readonly property bool crashPending: crash !== null && crash.pending === true && crash.kind !== "escape"
    readonly property string crashWords: crash === null ? "" : String(crash.at) + ": " + String(crash.reason)
    function readCrash() {
        Helper.request("GET", "/v1/crash", undefined, function(status, doc) { page.crash = doc && doc.ok === true && doc.crash ? doc.crash : null })
    }
    function crashRan(secs) {
        if (secs === null || secs === undefined) return ""
        return " after " + (secs >= 120 ? Math.round(secs / 60) + " minutes" : secs + " s")
    }
    function crashDetails() {
        var c = page.crash
        if (c === null) return
        page.host.confirm({ icon: "triangle-alert", title: "LumaTV recovered from a problem",
                            body: c.at + ": " + c.reason + crashRan(c.uptimeSecs) + ". LG's compositor took over and Home came back. The log and a trace of the last seconds are saved on the TV, in " + c.path + ".",
                            answers: [{ id: "close", label: "Close" }, { id: "clear", label: "Clear this notice" }], focus: 0 },
                          function(id) { if (id === "clear") page.clearCrash() })
    }
    function clearCrash() { Helper.post("/v1/crash/ack", {}, function() { page.readCrash() }) }

    LunaCall {
        id: luna
        onAnswered: {
            if (tag === "hbc" || tag === "hbcset") { page.blockAnswered(tag, doc); return }
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
        readCrash()
        Sio.read("", ["eulaStatus"])
        Sio.read("support", ["automaticUpdate"])
        Sio.read("general", ["doNotSellMyPersonalInformation", "lmt"])
        Sio.read("network", ["deviceName"])
        Sio.read("option", ["vsn", "livePlus", "livePromotion"])
        Power.readPanel()
        luna.call("device", "luna://com.palm.systemservice/deviceInfo", "query", { parameters: ["product_id", "serial_number"] })
        luna.call("os", "luna://com.palm.systemservice/osInfo", "query", { parameters: ["core_os_release", "core_os_release_codename"] })
        luna.call("storage", "luna://com.webos.service.attachedstoragemanager", "getProperties", { deviceId: "INTERNAL_STORAGE" })
        luna.call("net", "luna://com.webos.service.connectionmanager", "getinfo", {})
        luna.call("dolby", "luna://com.webos.service.pqcontroller", "getInfo", { key: "dolbySWVersion" })
        luna.call("keys", "luna://com.webos.service.sm/crypto", "checkSeStore", { id: ["esn", "widevine"] })
        luna.call("update", "luna://com.webos.service.update", "getStatus", {})
        Helper.start()                          // idempotent: the live state of the hosts block is the helper's `updateblock` topic
        luna.call("hbc", page.hbc, "getConfiguration", {})
        readAdId()
        pending = ""
    }

    // ---- Block LG system updates. webOS 26, LG's next major update, is reported to patch the root access this TV runs on, so the TV must not be
    // updated by accident. The Homebrew Channel keeps the block as its own setting, `blockUpdates`, read and set through its service
    // (`getConfiguration`, `setConfiguration`); at each boot, while it is on, the channel maps LG's update servers to this TV in /etc/hosts. So a
    // change applies when the TV restarts, and what is mapped now is the helper's `updateblock` topic (another tool's hosts file once hid the block).
    // Turning it off asks twice and says what LG's update can do. While updates are blocked, or while it cannot be told, Check for Updates does nothing.
    readonly property string hbc: "luna://org.webosbrew.hbchannel.service"
    property var blockFlag: null                // the channel's blockUpdates; null until it has answered
    property string blockError: ""
    readonly property var blockLive: Helper.updateBlock
    readonly property bool blockOn: blockFlag !== null ? blockFlag === true : (blockLive !== null && blockLive.flag === true)
    readonly property bool updatesBlocked: blockFlag === true || (blockLive !== null && blockLive.blocking === true)
    readonly property bool blockUnknown: blockFlag === null && blockLive === null
    readonly property string blockWords: {
        if (blockError !== "") return blockError
        if (blockFlag === null && blockLive === null) return "Homebrew Channel not answering"
        if (blockLive === null) return blockOn ? "On" : "Off"
        if (blockLive.state === "blocked") return "Active: LG's update servers are blocked"
        if (blockLive.state === "armed") return "On; active once the TV restarts"
        if (blockLive.state === "hosts-only") return "Off; still blocked until the TV restarts"
        return "Off; LG's update can reach this TV"
    }
    function blockAnswered(tag, doc) {
        if (!doc || doc.returnValue === false) {
            blockError = doc && doc.errorText ? "Homebrew Channel: " + String(doc.errorText) : "Homebrew Channel not answering"
            if (tag === "hbc") blockFlag = null
            return
        }
        blockError = ""
        if (typeof doc.blockUpdates === "boolean") blockFlag = doc.blockUpdates
        if (tag === "hbcset") reread.restart()
    }
    function setBlock(on) {
        if (on) { luna.call("hbcset", hbc, "setConfiguration", { blockUpdates: true }); return }
        page.host.confirm({ icon: "shield-alert", title: "Allow LG system updates?", body: "LG's update can remove root and LumaTV.", focus: 0,
                            answers: [{ id: "no", label: "Keep blocked" }, { id: "yes", label: "Continue", danger: true }] },
                          function(id) {
                              if (id !== "yes") return
                              page.host.confirm({ icon: "shield-alert", title: "Turn the update block off?", body: "The TV can then fetch and install LG's update, and it cannot be undone from here.", focus: 0,
                                                  answers: [{ id: "no", label: "Keep blocked" }, { id: "yes", label: "Turn off", danger: true }] },
                                                function(id2) { if (id2 === "yes") luna.call("hbcset", hbc, "setConfiguration", { blockUpdates: false }) })
                          })
    }
    function checkUpdatesRow() {
        var row = R.lg(page, { tab: 1, id: "checkupdate", target: "SoftwareUpdate", label: "Check for Updates", help: "Checking sends the TV's model and version to LG, and an install restarts the TV. Both are done in LG's screen." })
        var go = row.act
        return R.over(row, { enabled: function() { return !page.updatesBlocked && !page.blockUnknown },
                             value: function() { return page.updatesBlocked ? "Updates blocked" : page.blockUnknown ? "Block not readable" : "" },
                             chevron: false,
                             act: function() { if (!page.updatesBlocked && !page.blockUnknown) go() } })
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
        info("triangle-alert", "LumaTV recovered from a problem", function() { return page.crashWords }, { visible: function() { return page.crashPending } }),
        C.action({ id: "crashdetails", tab: 0, icon: "file-text", label: "Details", button: "Open", visible: function() { return page.crashPending },
                   help: "Why LumaTV's compositor was put out of the way, and when. The log and a trace of its last seconds are saved on the TV.",
                   act: function() { page.crashDetails() } }),
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
        { kind: "toggle", tab: 1, id: "blockupdates", icon: "shield-check", label: "Block LG system updates",
          help: "Keeps LG's update servers out of reach, so the TV is not updated by accident: LG's update can remove root and LumaTV. This is the Homebrew Channel's own setting; a change applies when the TV restarts.",
          on: function() { return page.blockOn }, enabled: function() { return page.blockFlag !== null },
          detail: function() { return page.blockWords }, act: function(v) { page.setBlock(v) } },
        page.checkUpdatesRow(),

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

        // LG keeps these two under System › Additional Settings; they are consent, so they sit with the privacy rows. Live Promotion follows Live Plus.
        C.lgShown(Sio, "option.livePlus", R.over(R.toggle(Sio, { ds: "option.livePlus", tab: 2, icon: "scan-eye", label: "Live Plus",
                                                                 help: "LG recognises what the TV shows and uses it for recommendations and ads." }),
                { act: function(v) {
                      if (!v) { Sio.write("option.livePlus", "off"); return }
                      C.ask(page, { icon: "scan-eye", title: "Turn Live Plus on?", body: "LG recognises what the TV shows and may use it for recommendations and ads.", yes: "Turn on", no: "Cancel" },
                            function() { Sio.write("option.livePlus", "on") })()
                  } })),
        C.lgShown(Sio, "option.livePromotion", R.over(R.toggle(Sio, { ds: "option.livePromotion", tab: 2, icon: "star", label: "Live Promotion",
                                                                      help: "Turns custom ads and services on or off while watching. Needs Live Plus." }),
                { enabled: function() { return page.livePlusOn && Sio.usable("option.livePromotion") },
                  detail: function() { return !Sio.has("option.livePromotion") ? "TV not answering" : page.livePlusOn ? "" : "Turn Live Plus on" } })),
        R.link({ tab: 2, id: "permissions", icon: "shield-check", label: "App Permissions", help: "Which apps may use the camera, the microphone, nearby devices and files.",
                 go: function() { page.host.openPage("permissions") } }),
        R.link({ tab: 2, id: "deletedata", icon: "eye-off", label: "Delete My Personal Information", help: "Deletes the viewing and voice information LG collected from this TV.",
                 go: function() { page.host.openPage("deletedata") } }),

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
