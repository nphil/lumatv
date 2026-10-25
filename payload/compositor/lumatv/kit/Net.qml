pragma Singleton
import QtQuick 2.4
import WebOSServices 1.0

// The network as Settings shows it: which link is up, the Wi-Fi name and strength, the address. From the
// connection manager (`com.webos.service.connectionmanager/getStatus`, subscribed). What it does not say
// stays empty and the page shows nothing for it.
QtObject {
    id: net
    property bool wifi: false
    property bool wired: false
    property string ssid: ""
    property int bars: 0                    // 0..4, from the signal level
    property string ip: ""
    property string band: ""                // "5 GHz"
    property string dbm: ""
    property string method: ""              // "dhcp" or "manual"
    readonly property string strength: !wifi ? "" : bars >= 3 ? "Strong" : bars === 2 ? "Good" : bars === 1 ? "Weak" : "Very weak"
    readonly property string summary: wired ? "Wired" + (ip ? " · " + ip : "") : wifi ? (ssid || "Wi-Fi") + (strength ? " · " + strength : "") : "Not connected"

    function parse(payload) {
        var doc = null
        try { doc = JSON.parse(payload) } catch (e) { return }
        if (!doc || doc.returnValue === false) return
        var w = doc.wifi || {}, e = doc.wired || {}
        wifi = w.state === "connected"
        wired = e.state === "connected"
        ssid = w.ssid || ""
        var level = parseInt(w.signalLevel !== undefined ? w.signalLevel : w.signalStrength, 10)
        dbm = isNaN(level) ? "" : String(level)
        var b = parseInt(w.signalBars, 10)
        bars = !isNaN(b) ? Math.max(0, Math.min(4, b)) : isNaN(level) ? 0 : level >= -55 ? 4 : level >= -65 ? 3 : level >= -75 ? 2 : 1
        band = w.channelFrequency ? (w.channelFrequency > 4000 ? "5 GHz" : "2.4 GHz") : (w.band || "")
        ip = (wired ? e.ipAddress : w.ipAddress) || ""
        method = (wired ? e.method : w.method) || ""
    }
    property Service status: Service {
        appId: "com.webos.surfacemanager"
        service: "luna://com.webos.service.connectionmanager"
        method: "getStatus"
        onResponse: net.parse(payload)
    }
    Component.onCompleted: status.callService({ subscribe: true })
}
