pragma Singleton
import QtQuick 2.4
import WebOSServices 1.0

// What the TV knows about its inputs, for the input picker and the Settings pages: each HDMI port's name
// (the device names itself over HDMI-CEC) and whether anything is connected, from the external input
// manager (`com.webos.service.eim/getAllInputStatus`, PLAN "Reference: ecklf/lg-luma"), and which app is
// in front (`getForegroundAppInfo`), which is the current input when its id is an input's. The picker
// asks for a refresh when it opens; the service is also asked every 30 s while a surface that shows
// inputs is up (`watch`), so a cable plugged in reads as connected without a restart.
//
// The device icon follows the name the device gave itself, then the port's own label; the owner's choice
// per input (Auto, Always, Hidden) lives in the helper's settings and is applied by the surfaces.
QtObject {
    id: inputs

    property var list: []                  // [{id, appId, port, label, name, connected, icon}]
    property string foreground: ""         // app id in front
    property int watchers: 0               // surfaces that want the list kept fresh
    readonly property var connected: list.filter(function(i) { return i.connected })
    readonly property var current: {
        for (var i = 0; i < list.length; i++) if (list[i].appId === foreground) return list[i]
        return null
    }

    // The icon for what is on the port: the device's own name first.
    function iconFor(name, port) {
        var n = String(name || "").toLowerCase()
        if (/play ?station|ps[345]|xbox|switch|nintendo|console|steam deck/.test(n)) return "gamepad-2"
        if (/apple tv|chromecast|fire ?tv|roku|shield|streamer|nvidia|google tv/.test(n)) return "tv-minimal-play"
        if (/denon|marantz|yamaha|onkyo|receiver|avr|soundbar|sonos|bose|speaker|arc/.test(n)) return "speaker"
        if (/blu-?ray|dvd|player|disc|magnavox/.test(n)) return "disc-3"
        if (/pc|computer|laptop|mac|desktop|monitor/.test(n)) return "monitor"
        if (/antenna|tuner|cable box|set.?top/.test(n)) return "antenna"
        return n === "" ? "unplug" : "cable"
    }
    function portName(appId) {
        var m = /hdmi(\d)/.exec(String(appId))
        return m ? "HDMI " + m[1] : appId === "com.webos.app.livetv" ? "Live TV" : String(appId)
    }

    function parse(payload) {
        var doc = null
        try { doc = JSON.parse(payload) } catch (e) { return }
        if (!doc || doc.returnValue === false) return
        var src = doc.devices || doc.inputs || doc.deviceList || []
        var out = []
        for (var i = 0; i < src.length; i++) {
            var d = src[i]
            var appId = d.appId || d.id || ""
            var label = d.label || d.deviceLabel || d.name || ""
            var port = portName(appId)
            var up = d.connected === true || d.activate === true || d.connected === "true"
            out.push({ id: d.id || appId, appId: appId, port: port, label: label, name: up ? (label || "") : "", connected: up,
                       icon: iconFor(up ? label : "", port) })
        }
        list = out
    }
    function parseForeground(payload) {
        try {
            var doc = JSON.parse(payload)
            var f = doc.foregroundAppInfo
            foreground = f && f.length ? (f[0].appId || "") : (doc.appId || "")
        } catch (e) { }
    }

    function refresh() {
        eim.callService({ subscribe: true })
        fg.callService({ subscribe: true })
    }
    function watch(on) { watchers = Math.max(0, watchers + (on ? 1 : -1)) }

    property Service eim: Service {
        appId: "com.webos.surfacemanager"
        service: "luna://com.webos.service.eim"
        method: "getAllInputStatus"
        onResponse: inputs.parse(payload)
    }
    property Service fg: Service {
        appId: "com.webos.surfacemanager"
        service: "luna://com.webos.service.applicationmanager"
        method: "getForegroundAppInfo"
        onResponse: inputs.parseForeground(payload)
    }
    property Timer keep: Timer {
        interval: 30000
        repeat: true
        running: inputs.watchers > 0
        onTriggered: inputs.refresh()
    }
    Component.onCompleted: refresh()
}
