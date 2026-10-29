import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Network (docs/mocks/30-settings-network): what the connection manager says about the links, and the one
// network setting that matters to a house with Home Assistant, waking the TV over the network. Joining
// another network, a static address and the router details stay in LG's Network screen.
PageList {
    id: page
    title: "Network"
    tabs: ["Wi-Fi", "Wired", "Details"]
    summary: Net.wired ? "Wired" : Net.wifi ? (Net.ssid || "Wi-Fi") + (Net.strength ? " \u00b7 " + Net.strength : "") : "Not connected"

    function refresh() { Sio.read("network", ["wolwowlOnOff", "deviceName", "ipv6Enable", "bleAdvertisingOnOff"]) }
    Component.onCompleted: refresh()
    onOpened: refresh()

    rows: [
        { kind: "value", tab: 0, icon: "wifi", label: function() { return Net.wifi ? Net.ssid : "Wi-Fi" },
          value: function() { return Net.wifi ? (Net.strength || "Connected") : "Not connected" }, chevron: false, info: true },
        { kind: "value", tab: 0, icon: "router", label: "IP address", value: function() { return Net.ip || "None" }, chevron: false, info: true },
        R.link({ tab: 0, id: "join", icon: "wifi", label: "Join Network", go: function() { page.host.openPage("wifi") } }),
        R.link({ tab: 0, id: "ip", icon: "router", label: "IP Settings", go: function() { page.host.openPage("ipsettings") } }),
        R.toggle(Sio, { ds: "network.wolwowlOnOff", tab: 0, on: "true", off: "false", icon: "power", label: "Wake from the network",
                        help: "Lets Home Assistant turn the TV on (Wake-on-LAN)." }),

        { kind: "value", tab: 1, icon: "ethernet-port", label: "Wired connection", chevron: false, info: true,
          value: function() { return Net.wired ? "Connected" : "Unplugged" } },
        { kind: "value", tab: 1, icon: "router", label: "IP address", value: function() { return Net.ip || "None" }, chevron: false, info: true,
          visible: function() { return Net.wired } },
        R.link({ tab: 1, id: "ip2", icon: "router", label: "IP Settings", go: function() { page.host.openPage("ipsettings") } }),

        { kind: "value", tab: 2, icon: "signal", label: "Signal", chevron: false, info: true,
          value: function() { return Net.wifi ? (Net.dbm ? Net.dbm + " dBm" : Net.strength || "Not reported") : "No link" } },
        { kind: "value", tab: 2, icon: "radio", label: "Band", chevron: false, info: true, value: function() { return Net.band },
          visible: function() { return Net.wifi && Net.band !== "" } },
        { kind: "value", tab: 2, icon: "globe", label: "Address", chevron: false, info: true,
          value: function() { return Net.method === "dhcp" ? "Automatic" : Net.method === "manual" ? "Manual" : "" },
          visible: function() { return Net.method !== "" },
          help: "Automatic means the router hands it out (DHCP)." },
        { kind: "value", tab: 2, icon: "tv", label: "TV Name", chevron: true, enabled: function() { return Sio.usable("network.deviceName") },
          visible: function() { return C.shown(Sio, "network.deviceName") },
          help: "The name on the network and in Home Assistant.",
          value: function() { return Sio.has("network.deviceName") ? String(Sio.get("network.deviceName")) : "" },
          act: function() { if (page.host) page.host.entry({ prompt: "TV name", text: String(Sio.get("network.deviceName") || "") }, function(t) { if (t.trim() !== "") Sio.write("network.deviceName", t.trim()) }) } },
        C.lgShown(Sio, "network.ipv6Enable", R.toggle(Sio, { ds: "network.ipv6Enable", tab: 2, on: "true", off: "false", icon: "globe", label: "IPv6" })),
        C.lgShown(Sio, "network.bleAdvertisingOnOff", R.toggle(Sio, { ds: "network.bleAdvertisingOnOff", tab: 2, icon: "radio", label: "Wake from a phone (Bluetooth)",
                                                                   help: "Lets a phone turn the TV on over Bluetooth. Uses a little power while off." }))
    ]
}
