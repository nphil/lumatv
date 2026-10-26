import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// IP Settings (LG: General › Network › the connection's Advanced Settings): automatic (DHCP) or a fixed address with its subnet mask, gateway
// and DNS server, for the link that is up (Wi-Fi or the cable). A network entrance, so it is done at the remote only: the addresses are typed
// on the on-screen keyboard into a draft, nothing changes until Apply, and Apply asks first because wrong addresses cut the TV off from the
// network. The read is the connection manager's `getStatus` (the link's method, address, netmask, gateway and dns1); the write is its
// `setipv4` (method, address, netmask, gateway, and the `ssid` for Wi-Fi) and `setdns`. LG's Network app is not in the Settings bundle, so the
// parameter names are webOS's own connection manager's and have not been run on this TV.
PageList {
    id: page
    title: "IP Settings"
    back: "Network"
    property string link: ""               // "wifi", "wired" or "" when neither is up
    property string ssid: ""
    property bool manual: false
    property string ip: ""
    property string mask: ""
    property string gateway: ""
    property string dns: ""
    property string status: ""

    LunaCall {
        id: luna
        onAnswered: {
            if (tag === "status") page.read(doc)
            else if (tag === "apply") page.status = doc && doc.returnValue !== false ? "Applied" : "Not applied"
        }
    }
    function read(doc) {
        if (!doc || doc.returnValue === false) { link = ""; return }
        var w = doc.wired || {}, f = doc.wifi || {}
        link = w.state === "connected" ? "wired" : f.state === "connected" ? "wifi" : ""
        var l = link === "wired" ? w : link === "wifi" ? f : {}
        ssid = f.ssid || ""
        manual = l.method === "manual"
        ip = l.ipAddress || ""
        mask = l.netmask || l.subnetMask || ""
        gateway = l.gateway || ""
        dns = l.dns1 || ""
    }
    function refresh() { status = ""; luna.call("status", "luna://com.webos.service.connectionmanager", "getStatus", {}) }
    onOpened: refresh()

    function valid(a) {
        var m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(a)
        return !!m && m[1] <= 255 && m[2] <= 255 && m[3] <= 255 && m[4] <= 255
    }
    readonly property bool ready: link !== "" && (!manual || (valid(ip) && valid(mask) && valid(gateway) && valid(dns)))
    function apply() {
        var p = { method: manual ? "manual" : "dhcp" }
        if (link === "wifi") p.ssid = ssid
        if (manual) { p.address = ip; p.netmask = mask; p.gateway = gateway }
        luna.call("apply", "luna://com.webos.service.connectionmanager", "setipv4", p)
        if (manual) { var d = { dns: [dns] }; if (link === "wifi") d.ssid = ssid; luna.call("dns", "luna://com.webos.service.connectionmanager", "setdns", d) }
        status = "Applying"
    }
    function field(id, icon, label, get, set) {
        return { kind: "value", id: id, icon: icon, label: label, chevron: true, value: get,
                 enabled: function() { return page.manual && page.link !== "" },
                 detail: function() { return page.manual ? (page.valid(get()) || get() === "" ? "" : "Not an address") : "Set to Manual" },
                 act: function() { if (page.host) page.host.entry({ prompt: label, text: get() }, set) } }
    }

    rows: [
        R.note(function() { return page.link === "" ? "No network link" : page.status }, 0, function() { return page.link === "" || page.status !== "" }),
        { kind: "seg", id: "method", icon: "router", label: "IP Address Setting", options: ["Automatic", "Manual"],
          enabled: function() { return page.link !== "" },
          index: function() { return page.manual ? 1 : 0 }, act: function(i) { page.manual = i === 1 } },
        field("ip", "globe", "IP Address", function() { return page.ip }, function(t) { page.ip = t.trim() }),
        field("mask", "network", "Subnet Mask", function() { return page.mask }, function(t) { page.mask = t.trim() }),
        field("gateway", "router", "Gateway", function() { return page.gateway }, function(t) { page.gateway = t.trim() }),
        field("dns", "server", "DNS Server", function() { return page.dns }, function(t) { page.dns = t.trim() }),
        C.action({ id: "apply", icon: "check", label: "Apply", button: "Apply", enabled: function() { return page.ready },
                   detail: function() { return page.link !== "" && !page.ready ? "Check the addresses" : "" },
                   act: C.ask(page, { icon: "router", title: "Change network settings?", body: "The TV may lose its connection if they are wrong.", yes: "Apply", no: "Cancel" }, function() { page.apply() }) })
    ]
}
