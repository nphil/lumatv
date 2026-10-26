import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Join Network (LG: General › Network › Wi-Fi Connection): the networks the TV can see, and joining one. A network entrance, so it is
// done at the remote only: the password is typed on the on-screen keyboard (shown as bullets), a question follows (the TV may leave
// the network it is on), and the password goes straight to `com.webos.service.wifi/connect` and is kept nowhere. The list is the service's
// `findnetworks` answer (`foundNetworks[].networkInfo`: ssid, availableSecurityTypes, signalBars); the join is the service's `connect`
// with `security.simpleSecurity.passKey` for a secured network and the bare SSID for an open one. The method names are webOS's own Wi-Fi
// service (LG's Network app is not in the Settings bundle): they have not been run on this TV.
PageList {
    id: page
    title: "Join Network"
    back: "Network"
    property var nets: []
    property bool scanning: false
    property string status: ""

    LunaCall {
        id: luna
        onAnswered: {
            if (tag === "scan") {
                page.scanning = false
                if (!doc || doc.returnValue === false) { page.nets = []; page.status = "No answer from the Wi-Fi"; return }
                var l = doc.foundNetworks || [], out = []
                for (var i = 0; i < l.length; i++) {
                    var n = l[i].networkInfo || l[i]
                    if (n && n.ssid) out.push({ ssid: n.ssid, secured: page.isSecured(n), bars: n.signalBars !== undefined ? n.signalBars : 0 })
                }
                page.nets = out
            } else if (tag === "join") {
                page.status = doc && doc.returnValue !== false ? "Joined" : "Could not join"
            }
        }
    }
    function isSecured(n) {
        var t = n.availableSecurityTypes || []
        for (var i = 0; i < t.length; i++) if (t[i] !== "none" && t[i] !== "open") return true
        return false
    }
    function scan() { scanning = true; status = ""; luna.call("scan", "luna://com.webos.service.wifi", "findnetworks", {}) }
    onOpened: scan()

    function barsText(b) { return b >= 3 ? "Strong" : b === 2 ? "Good" : b === 1 ? "Weak" : "" }
    // the question, then the join; the password lives in this closure until the join is sent
    function join(ssid, pass) {
        C.ask(page, { icon: "wifi", title: "Join " + ssid + "?", yes: "Join", no: "Cancel" }, function() {
            page.status = "Joining " + ssid
            var p = { ssid: ssid }
            if (pass !== "") p.security = { securityType: "psk", simpleSecurity: { passKey: pass } }
            luna.call("join", "luna://com.webos.service.wifi", "connect", p)
        })()
    }
    function pick(n) {
        if (!page.host) return
        if (n.secured) page.host.entry({ prompt: "Password for " + n.ssid, secret: true }, function(text) { if (text !== "") page.join(n.ssid, text) })
        else page.join(n.ssid, "")
    }
    function netRow(n) {
        return { kind: "value", id: "net:" + n.ssid, icon: "wifi", label: n.ssid, chevron: false,
                 value: (page.barsText(n.bars) + (n.secured ? (n.bars ? " \u00b7 " : "") + "Secured" : "")),
                 selected: function() { return Net.wifi && Net.ssid === n.ssid }, act: function() { page.pick(n) } }
    }
    function build() {
        var out = []
        if (status !== "" || scanning) out.push(R.note(scanning ? "Looking for networks" : status))
        for (var i = 0; i < nets.length; i++) out.push(netRow(nets[i]))
        out.push({ kind: "value", id: "hidden", icon: "eye-off", label: "Other Network", chevron: true,
                   act: function() {
                       if (!page.host) return
                       page.host.entry({ prompt: "Network name" }, function(ssid) {
                           if (ssid === "") return
                           page.host.entry({ prompt: "Password for " + ssid, secret: true }, function(text) { page.join(ssid, text) })
                       })
                   } })
        out.push(C.action({ id: "rescan", icon: "refresh-cw", label: "Search Again", button: "Search", enabled: function() { return !page.scanning }, act: function() { page.scan() } }))
        return out
    }
    rows: build()
}
