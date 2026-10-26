import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Delete My Personal Information (LG: General › Privacy & Terms): Delete Viewing Information and Delete Voice Information, each behind a question and
// each only while LG's terms are accepted (`eulaStatus`: generalTermsAllowed, and voiceAllowed for the voice) and the TV is online (`connectionmanager
// getStatus`: isInternetConnectionAvailable), as LG's. Viewing: the TV's device id (`sm/deviceid getIDs` LGUDID) goes to `usercontextmanager reset
// {toast, bodyData: {isDeleteAccountAllData, lgudid}}` and `userintentmanager reset {target: "viewingInfo", toast: false}` runs; voice: `lg.thinqai.adapter
// deletePrivacyData` and `userintentmanager reset {target: "voiceInfo", toast: false}`. Delete Voice ID is the signed-in user's own (LG's `activeUser`
// number), which this page does not have, so it stays in LG's screen. Done at the remote, never offered to an assistant.
PageList {
    id: page
    title: "Delete My Personal Information"
    back: "Support"
    backId: "support"

    property bool online: false
    property bool viewingDone: false            // asked and sent, until the page is opened again
    property bool voiceDone: false
    readonly property bool terms: { var e = Sio.get("eulaStatus"); return !!(e && e.generalTermsAllowed) }
    readonly property bool voiceTerms: { var e = Sio.get("eulaStatus"); return !!(e && e.voiceAllowed) }

    LunaCall {
        id: luna
        onAnswered: {
            if (!doc) return
            if (tag === "net") { page.online = doc.isInternetConnectionAvailable === true; return }
            if (tag !== "lgudid" || doc.returnValue === false) return
            var l = doc.idList || []
            for (var i = 0; i < l.length; i++)
                if (l[i].idType === "LGUDID") luna.act("luna://com.webos.service.usercontextmanager", "reset", { toast: true, bodyData: { isDeleteAccountAllData: true, lgudid: l[i].idValue } })
        }
    }
    function refresh() {
        viewingDone = false
        voiceDone = false
        Sio.read("", ["eulaStatus"])
        luna.call("net", "luna://com.webos.service.connectionmanager", "getStatus", {})
    }
    onOpened: refresh()

    function why(voice) { return !terms || (voice && !voiceTerms) ? "Needs LG's terms" : !online ? "Needs the network" : "" }
    function deleteViewing() {
        luna.call("lgudid", "luna://com.webos.service.sm/deviceid", "getIDs", { idType: ["LGUDID"] })
        luna.act("luna://com.webos.service.userintentmanager", "reset", { target: "viewingInfo", toast: false })
        viewingDone = true
    }
    function deleteVoice() {
        luna.act("luna://lg.thinqai.adapter", "deletePrivacyData", {})
        luna.act("luna://com.webos.service.userintentmanager", "reset", { target: "voiceInfo", toast: false })
        voiceDone = true
    }

    rows: [
        C.action({ id: "viewing", icon: "eye-off", label: "Delete Viewing Information", danger: true, button: function() { return page.viewingDone ? "Done" : "Delete" },
                   help: "Deletes the viewing information LG collected from this TV. It may change what LG recommends.",
                   enabled: function() { return page.why(false) === "" }, detail: function() { return page.why(false) },
                   act: C.ask(page, { icon: "eye-off", title: "Delete viewing information?", body: "It may change what LG recommends.", yes: "Delete", no: "Cancel", danger: true },
                              function() { page.deleteViewing() }) }),
        C.action({ id: "voice", icon: "mic", label: "Delete Voice Information", danger: true, button: function() { return page.voiceDone ? "Done" : "Delete" },
                   help: "Deletes the voice information LG collected from this TV.",
                   enabled: function() { return page.why(true) === "" }, detail: function() { return page.why(true) },
                   act: C.ask(page, { icon: "mic", title: "Delete voice information?", body: "LG's voice service forgets what it kept.", yes: "Delete", no: "Cancel", danger: true },
                              function() { page.deleteVoice() }) })
    ]
}
