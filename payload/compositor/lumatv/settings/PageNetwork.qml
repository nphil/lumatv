import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// Network (docs/mocks/30-settings-network): what the connection manager says about the links, and the one
// network setting that matters to a house with Home Assistant, waking the TV over the network. Joining
// another network, a static address and the router details stay in LG's Network screen.
PageList {
    id: page
    title: "Network"
    tabs: ["Wi-Fi", "Wired", "Details"]
    subtitle: Net.wired ? "Wired" + (Net.ip ? " \u00b7 " + Net.ip : "") : Net.wifi ? (Net.ssid || "Wi-Fi") + (Net.band ? " \u00b7 " + Net.band : "") + (Net.strength ? " \u00b7 " + Net.strength : "") : "Not connected"
    summary: Net.summary

    Component.onCompleted: Sio.read("general", ["wolwowlOnOff"])

    rows: [
        { kind: "value", tab: 0, icon: "wifi", label: function() { return Net.wifi ? Net.ssid : "Wi-Fi" },
          detail: function() { return Net.wifi ? "Connected" + (Net.band ? " \u00b7 " + Net.band : "") + (Net.dbm ? " \u00b7 " + Net.dbm + " dBm" : "") + (Net.strength ? " \u00b7 " + Net.strength : "")
                                               : "Not connected to a wireless network." },
          value: function() { return Net.wifi ? "Connected" : "Off" }, chevron: false, info: true },
        { kind: "value", tab: 0, icon: "router", label: "IP address",
          detail: function() { return (Net.ip || "No address") + (Net.method ? " \u00b7 " + (Net.method === "dhcp" ? "from the router (DHCP)" : "set by hand") : "") },
          value: function() { return Net.method === "dhcp" ? "Automatic" : Net.method === "manual" ? "Manual" : "" }, chevron: false, info: true },
        R.head("Network features", 0),
        R.toggle(Sio, { ds: "general.wolwowlOnOff", tab: 0, on: "true", off: "false", icon: "power", label: "Wake from the network",
                        detail: "Lets Home Assistant turn the TV on (Wake-on-LAN)." }),
        R.lg(page, { tab: 0, detail: "Other networks, Ethernet details, Screen Share, AirPlay and HomeKit, IP control, LG Connect Apps." }),

        { kind: "value", tab: 1, icon: "ethernet-port", label: "Wired connection", chevron: false, info: true,
          detail: function() { return Net.wired ? "Connected" + (Net.ip ? " \u00b7 " + Net.ip : "") : "No cable connected." },
          value: function() { return Net.wired ? "Connected" : "Unplugged" } },
        R.lg(page, { tab: 1, detail: "Ethernet details and the router's settings." }),

        { kind: "value", tab: 2, icon: "signal", label: "Signal", chevron: false, info: true,
          detail: function() { return Net.wifi ? (Net.dbm ? Net.dbm + " dBm" : "Strength not reported") : "No wireless link." },
          value: function() { return Net.strength } },
        { kind: "value", tab: 2, icon: "globe", label: "Address", chevron: false, info: true,
          detail: function() { return Net.ip || "No address" }, value: function() { return Net.method === "dhcp" ? "DHCP" : Net.method === "manual" ? "Manual" : "" } }
    ]
}
